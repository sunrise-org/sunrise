//! Receiving-provider credit follows ordered events and decoder placeholders.
use super::*;

fn local_transfer_notice() -> Event {
    Event::Disconnected {
        reason: sunrise_client::lang::get("disconnect.transfer"),
        key: Some("disconnect.transfer".into()),
        from_server: true,
    }
}

impl Remote {
    pub(super) fn receive_credited_backlog(&mut self, events: &mut VecDeque<Received>) {
        // Chunk packets start decoding now; the frame that reaches one in
        // order inserts its column.
        for receipt in self.session.poll() {
            let (event, credit) = receipt.into_parts();
            events.push_back(match event {
                Event::Packet(p) if matches!(*p, C::LevelChunkWithLight(_)) => {
                    match self.decoder.submit(self.chunk_format.clone(), p) {
                        None => Received::Chunk(self.chunk_format.clone(), credit),
                        Some(p) => Received::Event(Event::Packet(p), credit),
                    }
                }
                event => Received::Event(event, credit),
            });
        }
    }
}

impl Game<'_> {
    /// Handle what the server sent, in order. `before_tick`: a tick is about to
    /// run, so what an earlier poll left for later is taken whole (no frame
    /// budget, each column waited for) and nothing newer is received.
    pub(super) fn poll_remote_events(&mut self, before_tick: bool) -> Result<()> {
        let Some(r) = self.remote.as_mut() else {
            return Ok(());
        };
        let mut events = std::mem::take(&mut r.backlog);
        if !before_tick {
            r.receive_credited_backlog(&mut events);
        }
        let overdue = r
            .backlog_since
            .is_some_and(|since| since.elapsed() >= BACKLOG_LIMIT);
        let mut chunks = Duration::ZERO;
        // A run of ForgetLevelChunk (thousands when the server's view moves far: a
        // teleport at 32 chunks) is forgotten at once, before whatever follows it.
        let mut forgotten = sunrise_client::fast_hash::FxHashSet::default();
        let mut forgotten_credits = Vec::new();
        while let Some(item) = events.pop_front() {
            if let Received::Event(Event::Packet(packet), _) = &item
                && let C::ForgetLevelChunk(p) = &**packet
            {
                self.observe_client_packet(packet);
                forgotten.insert((p.pos[0], p.pos[1]));
                forgotten_credits.push(item.into_event_credit());
                continue;
            }
            sunrise_client::spans::time("pr_forget", || {
                self.forget_remote_columns(&mut forgotten, true)
            });
            forgotten_credits.clear();
            let chunk = matches!(item, Received::Chunk(_, _));
            if chunk && !overdue && !before_tick && chunks >= CHUNK_PACKETS_PER_FRAME {
                events.push_front(item);
                break;
            }
            let started = chunk.then(Instant::now);
            let (event, _credit) = match item {
                Received::Event(event, credit) => (event, credit),
                Received::Chunk(format, credit) => {
                    // A column still decoding waits for the next frame, with
                    // what came after it, overdue or not.
                    let Some(next) = self.remote.as_ref().map(|r| {
                        if before_tick {
                            r.decoder.wait_next(TICK_DECODE_WAIT)
                        } else {
                            r.decoder.next()
                        }
                    }) else {
                        break;
                    };
                    let (packet, column) = match next {
                        Next::Ready(decoded) => decoded,
                        Next::Decoding => {
                            events.push_front(Received::Chunk(format, credit));
                            break;
                        }
                        Next::Lost => {
                            log::warn!("Sunrise: the chunk decoder stopped; a column is missing");
                            continue;
                        }
                    };
                    self.observe_client_packet(&packet);
                    if let C::LevelChunkWithLight(p) = *packet {
                        sunrise_client::spans::time("pr_chunk", || {
                            self.remote_chunk(&p, Some((&format, column)))
                        });
                    }
                    chunks += started.map_or(Duration::ZERO, |t| t.elapsed());
                    continue;
                }
            };
            if !self.handle_remote_event(event)? {
                return Ok(());
            }
            chunks += started.map_or(Duration::ZERO, |t| t.elapsed());
            if self.remote.is_none() {
                break;
            }
        }
        if self.remote.is_some() {
            sunrise_client::spans::time("pr_forget", || {
                self.forget_remote_columns(&mut forgotten, true)
            });
        }
        if let Some(r) = self.remote.as_mut() {
            r.backlog_since = if events.is_empty() {
                None
            } else {
                Some(r.backlog_since.unwrap_or_else(Instant::now))
            };
            r.backlog = events;
        }
        self.send_suggestion_request();
        Ok(())
    }

    /// `ClientPacketListener.handleBundlePacket`: use one main-thread turn, so
    /// the chunk decoder/frame quota cannot split a bundle at a column packet.
    fn handle_play_bundle(&mut self, bundle: sunrise_client::net::CompletedBundle) -> Result<bool> {
        use sunrise_client::net::BundleStep;
        use sunrise_client::raw_payloads::{Classified, Incoming, dispatch_bundle};
        use sunrise_protocol::{connection_groups, payload_negotiation::Phase};
        let sunrise_client::net::CompletedBundle { packets, io } = bundle;
        let Some(remote) = self.remote.as_ref() else {
            return Ok(false);
        };
        if !io.accepts_bundle() {
            return Ok(true);
        }
        let endpoint = remote.session.payload_endpoint();
        let negotiated = endpoint.transport().epoch().is_some_and(|epoch| {
            epoch.phase == Phase::Play
                && epoch
                    .peer
                    .engine_receives(Phase::Play, connection_groups::CHANNEL)
        });
        if self.route_connection_group_bundle(&packets, negotiated)? {
            return Ok(self.remote.is_some());
        }
        let mut end = None;
        let mut transfer = None;
        for packet in packets {
            match io.bundle_member(packet) {
                BundleStep::Event(event) => {
                    self.handle_remote_event(event)?;
                }
                BundleStep::Payload(payload) => {
                    match Incoming::classify(endpoint.transport(), Phase::Play, payload) {
                        Ok(Classified::Application(incoming)) => {
                            for task in dispatch_bundle(incoming, &endpoint) {
                                self.run_payload(endpoint.clone(), task);
                            }
                        }
                        Ok(Classified::Other(payload)) => {
                            self.handle_remote_event(Event::Packet(Box::new(C::CustomPayload(
                                payload,
                            ))))?;
                        }
                        Ok(Classified::Raw(_)) => {}
                        Err(error) => {
                            log::warn!("Sunrise: bundle payload refused: {error}");
                            return self.handle_remote_event(io.invalid_bundle_payload());
                        }
                    }
                }
                BundleStep::Transfer(packet) => {
                    transfer = self.start_remote_transfer(packet)?;
                    if transfer.is_none() {
                        end.get_or_insert_with(local_transfer_notice);
                    }
                }
                BundleStep::Close(notice) => {
                    // Discard the old listener only after its remaining handlers run.
                    end.get_or_insert(notice);
                }
                BundleStep::Continue => {}
            }
        }
        if let Some(opening) = transfer {
            self.install_remote_transfer(opening)?;
            return Ok(false);
        }
        if let Some(end) = end {
            return self.handle_remote_event(end);
        }
        Ok(true)
    }

    /// False means the old connection ended, even if it transferred to a new server.
    fn handle_remote_event(&mut self, event: Event) -> Result<bool> {
        match event {
            Event::Status(s) => {
                if let Some(r) = self.remote.as_mut() {
                    r.status = s;
                }
            }
            Event::Joined {
                registries,
                name,
                uuid,
            } => {
                self.set_registries(registries);
                self.connection_joined();
                self.close_dialogs();
                if let Some(r) = self.remote.as_mut() {
                    r.joined = true;
                    r.profile = uuid;
                    r.profile_name = name.clone();
                    r.status = sunrise_client::lang::get_or(
                        "multiplayer.downloadingTerrain",
                        "Loading terrain...",
                    );
                }
                self.begin_book_negotiation();
            }
            Event::Features(features) => {
                if let Some(remote) = &mut self.remote {
                    remote.features = features.into();
                }
            }
            Event::Brand(brand) => {
                if let Some(remote) = &mut self.remote {
                    remote.brand = brand;
                }
            }
            Event::QuietCommands => {
                if let Some(remote) = &mut self.remote {
                    remote.quiet_commands = true;
                }
            }
            Event::CustomBooks => {
                if let Some(remote) = &mut self.remote {
                    remote.custom_books = true;
                }
            }
            Event::FarTerrain(offer) => {
                if let Some(remote) = &mut self.remote {
                    remote.far_terrain = Some(offer);
                }
            }
            Event::PlayCookie(cookie) => {
                let (receiver, cookie) = *cookie;
                if receiver.accepts_bundle() {
                    receiver.handle_cookie(cookie);
                }
            }
            Event::Reconfiguring(resume) => {
                self.reset_book_view();
                // handleConfigurationStart: the held-back chat shows, then
                // clearClientLevel opens ServerReconfigScreen over no level.
                self.flush_chat_queue();
                self.send_chat_acknowledgement();
                self.close_dialogs();
                // The server configuring us now (a proxy's next one)
                // offers its own far terrain, or none, and announces its
                // own features again.
                if let Some(remote) = &mut self.remote {
                    remote.viewed.clear();
                    remote.far_terrain = None;
                    remote.quiet_commands = false;
                    remote.custom_books = false;
                }
                self.clear_lod();
                if let Some(remote) = &self.remote {
                    remote.session.reset_far_terrain();
                }
                self.remove_local_player_body();
                self.set_screen(Screen::Playing);
                self.capture(false);
                // clearClientLevel drops the client level now, so its
                // unload comes before the configuration screen, not
                // after the next login's load.
                self.forget_all_remote_columns();
                if let Some(old) = self.remote.as_mut().and_then(|r| r.heard_level.take()) {
                    self.native_client_level_unload(&old);
                }
                self.native_lifecycle.begin_close();
                self.native_lifecycle_world_closed();
                let ticks = self.ticks;
                if let Some(r) = self.remote.as_mut() {
                    self.native_net.holds.borrow_mut().clear();
                    r.reconfiguring = Some(ticks);
                    r.chat_tree_received = false;
                }
                resume.acknowledge();
            }
            Event::Reconfigured(registries) => {
                // handleConfigurationStart → clearClientLevel, and the next
                // ClientPacketListener starts with no players or scoreboard,
                // no waypoints (its own ClientWaypointManager: a proxy's next
                // server never untracks the last one's), and its new
                // ClientSuggestionProvider with no custom chat completions.
                // clearClientLevel clears the narrator.
                self.clear_narrator();
                self.hud_disconnected();
                self.score.board = Default::default();
                self.clear_waypoints();
                if let Some(r) = self.remote.as_mut() {
                    r.attachments.clear();
                    r.players.clear();
                    r.tab_players.clear();
                    r.listed.clear();
                    r.signatures = Default::default();
                    r.custom_completions.clear();
                }
                self.ui.registries.custom_completions.clear();
                self.set_registries(registries);
            }
            Event::Transfer(packet) => {
                if self
                    .remote
                    .as_ref()
                    .is_some_and(|r| r.session.accepts_packets())
                {
                    if let Some(opening) = self.start_remote_transfer(packet)? {
                        self.install_remote_transfer(opening)?;
                        return Ok(false);
                    }
                    return self.handle_remote_event(local_transfer_notice());
                }
            }
            Event::PackPush(p) => self.handle_pack_push(p),
            Event::PackPop(id) => self.handle_pack_pop(id),
            Event::ServerLinks(links) => {
                if let Some(r) = self.remote.as_mut() {
                    r.server_links = crate::game::server_links::trusted(links);
                }
            }
            Event::ShowDialog { dialog, registries } => {
                self.show_dialog_in_phase(&dialog, Some(registries));
            }
            Event::ClearDialog => self.clear_dialog(),
            Event::CodeOfConduct(text) => self.handle_code_of_conduct(text),
            Event::DecodeError(e) => {
                if let Some(r) = self.remote.as_mut() {
                    r.decode_errors += 1;
                    if r.decode_errors <= 20 {
                        log::warn!("Sunrise: skipped undecodable packet {e}");
                    }
                }
            }
            Event::Disconnected {
                reason,
                key,
                from_server,
            } => {
                self.reset_book_view();
                log::info!("Sunrise: disconnected: {reason}");
                let by = if from_server {
                    sunrise_hooks::connection::By::Server
                } else {
                    sunrise_hooks::connection::By::Connection
                };
                self.connection_ended(&reason, key.as_deref(), by);
                self.leave_server(Some(Text::literal(reason)))?;
                return Ok(false);
            }
            Event::Bundle(bundle) => return self.handle_play_bundle(*bundle),
            Event::RawPayload(incoming) => {
                if let Some(remote) = self.remote.as_ref()
                    && remote.session.connected()
                {
                    self.run_payload(remote.session.payload_endpoint(), *incoming);
                }
            }
            Event::Packet(packet) => match *packet {
                C::CustomPayload(p) if p.channel == sunrise_client::perf_net::CHANNEL => {
                    self.remote_perf_reply(&p.data)
                }
                packet => self.handle_packet(packet)?,
            },
        }
        Ok(true)
    }
}
