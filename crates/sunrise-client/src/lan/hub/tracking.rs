//! Where the tick's chunk, view and tick-tracking work lives: the tick's
//! own steps (`tick`, `poll`), the level pass and its chunk map, and the
//! world synchronization after every level (time, weather, chunk batches).
use super::*;

impl Hub {
    // ---- per tick ------------------------------------------------------

    /// A whole tick with one level: the network, the level, then the
    /// players' latencies.
    pub fn tick(&mut self, ctx: &mut TickCtx) -> Vec<HostEvent> {
        let mut events = self.poll(ctx);
        events.extend(self.drain_packets(ctx));
        // A caller that consumes raw events must resume the inbox before
        // running level/maintenance work. Game uses the split methods.
        if !self.packets_paused() {
            self.sync_initial_health_data();
            for i in 0..self.guests.len() {
                if self.here(&self.guests[i]) {
                    self.sync_progress(i);
                }
            }
            events.extend(self.tick_level(ctx, None));
            self.finish_tracking_pass();
            self.end_tick();
        }
        events
    }

    /// The network since the last tick, once per server tick before any
    /// level ticks: joins (into the level in place), leaves, and each guest's
    /// packets queued for its own level's pass.
    pub fn poll(&mut self, ctx: &mut TickCtx) -> Vec<HostEvent> {
        self.ticks += 1;
        let mut events = std::mem::take(&mut self.pending_kicks);
        self.finish_fake_kicks(&mut events);
        self.expire_arrivals(&mut events);
        let mut credits = Vec::new();
        for receipt in self.lan.poll() {
            let (event, credit) = receipt.into_parts();
            match event {
                NetEvent::RawPayload(delivery) => {
                    self.poll_raw_payload(delivery, credit, &mut events, &mut credits);
                }
                NetEvent::Joined(j, spawn) => {
                    credits.push(credit);
                    while let Some(i) = self
                        .guests
                        .iter()
                        .position(|g| g.uuid == j.uuid && !g.participates_online())
                    {
                        events.push(HostEvent::Left(Box::new(self.guests.remove(i))));
                    }
                    let conn = self.admit(*j, ctx);
                    events.push(HostEvent::Joined(conn, spawn));
                }
                NetEvent::Left(conn, reason) => {
                    credits.push(credit);
                    if let Some(i) = self.guests.iter().position(|g| g.conn == conn) {
                        let g = self.remove_guest(i, &reason);
                        events.push(HostEvent::Left(Box::new(g)));
                    }
                }
                NetEvent::Packet(conn, packet, bytes) => {
                    self.update_guest(conn, |g| {
                        g.sender.undrained.taken(bytes);
                        if g.left_reason.is_empty() {
                            g.inbox.push_back(InboxItem::Packet(*packet, Some(credit)));
                        }
                    });
                }
                NetEvent::Duplicate(uuid) => {
                    credits.push(credit);
                    let conns: Vec<u32> = self
                        .guests
                        .iter()
                        .filter(|g| g.uuid == uuid && g.participates_online())
                        .map(|g| g.conn)
                        .collect();
                    for conn in conns {
                        self.kick(
                            conn,
                            translatable("multiplayer.disconnect.duplicate_login", &[]),
                        );
                    }
                }
            }
        }
        // keepConnectionAlive's timeout: close a kicked client still unread a second later.
        if self.ticks.is_multiple_of(20) {
            for g in &self.guests {
                if g.participates_online() && !g.left_reason.is_empty() && g.sender.timed_out() {
                    g.sender.close();
                }
            }
            // `keepConnectionAlive` never times out the singleplayer owner (`!isSingleplayerOwner()`).
            let silent: Vec<u32> = self
                .guests
                .iter()
                .filter(|g| {
                    g.participates_online()
                        && g.left_reason.is_empty()
                        && g.sender.timed_out()
                        && !self.lan.shared.is_singleplayer_owner(&g.name)
                })
                .map(|g| g.conn)
                .collect();
            for conn in silent {
                self.kick(conn, translatable("disconnect.timeout", &[]));
            }
        }
        events.push(HostEvent::InputCredits(credits));
        events
    }

    pub(super) fn observes_play_pongs(guest: &Guest) -> bool {
        guest.joined()
            && guest.participates_online()
            && mod_api::__private::listened(mod_api::HookId::PlayerPong)
    }

    /// Keeps server-answer packets in a frozen player's inbox (teleport, chunk batch,
    /// loaded, keep-alive, ping request, tick end, client information), plus debug
    /// subscription updates. Do not filter beyond a raw payload or subscribed play pong.
    /// Chat messages and signed commands are dropped, but their last-seen messages
    /// are taken first (`unpackAndApplyLastSeen`). Chat acknowledgements are handled
    /// here in arrival order: the client counts them as acknowledged, and its next
    /// message after the freeze would not validate otherwise.
    pub(crate) fn keep_acknowledgements(&mut self, i: usize, events: &mut Vec<HostEvent>) {
        // Only the entries before the next callback boundary are filtered; the rest stay
        // in the inbox untouched until the boundary's turn has finished in Game.
        let mut kept = Vec::new();
        let mut credits = Vec::new();
        while let Some(entry) = self.guests.inbox_mut(i).and_then(|inbox| inbox.pop_front()) {
            let (packet, credit) = match entry {
                InboxItem::Packet(packet, credit)
                    if !matches!(&packet, S::Pong(_))
                        || !Self::observes_play_pongs(&self.guests[i]) =>
                {
                    (packet, credit)
                }
                boundary => {
                    if let Some(inbox) = self.guests.inbox_mut(i) {
                        inbox.push_front(boundary);
                    }
                    break;
                }
            };
            match &packet {
                S::Chat(c) => {
                    self.apply_last_seen(i, &c.last_seen, events);
                }
                S::ChatCommandSigned(c) => {
                    self.apply_last_seen(i, &c.last_seen, events);
                }
                S::ChatAck(c) => self.chat_ack(i, c.offset, events),
                S::AcceptTeleportation(_)
                | S::PlayerLoaded(_)
                | S::ChunkBatchReceived(_)
                | S::ClientInformation(_)
                | S::ClientTickEnd(_)
                | S::KeepAlive(_)
                | S::DebugSubscriptionRequest(_)
                | S::PingRequest(_) => {
                    kept.push(InboxItem::Packet(packet, credit));
                    continue;
                }
                // handleMovePlayer validates packets even while the End credits are showing.
                S::MovePlayerPos(_)
                | S::MovePlayerPosRot(_)
                | S::MovePlayerRot(_)
                | S::MovePlayerStatus(_)
                    if self.guests[i].won_game =>
                {
                    kept.push(InboxItem::Packet(packet, credit));
                    continue;
                }
                // A player whose body was removed still controls the respawn button.
                S::ClientCommand(c) if c.action == 0 && self.guests[i].respawning => {
                    kept.push(InboxItem::Packet(packet, credit));
                    continue;
                }
                _ => {}
            }
            credits.extend(credit);
        }
        if let Some(inbox) = self.guests.inbox_mut(i) {
            for entry in kept.into_iter().rev() {
                inbox.push_front(entry);
            }
        }
        events.push(HostEvent::InputCredits(credits));
    }

    /// The level in place's part of the tick (`ServerLevel.tick`'s chunk
    /// map and the connections of the players in it): the chunks, block
    /// changes, containers and entities they see. Player packets are drained
    /// earlier (`drain_packets`) so scheduled ticks see what they placed.
    pub fn tick_level(
        &mut self,
        ctx: &mut TickCtx,
        entity_attachments: Option<
            &crate::fast_hash::FxHashMap<u32, sunrise_mod_runtime::attachments::Attachments>,
        >,
    ) -> Vec<HostEvent> {
        self.tick_level_inner(ctx, entity_attachments, true)
    }

    /// The level snapshot when `ServerPlayer.doTick` will run after all levels.
    pub fn tick_level_before_connections(
        &mut self,
        ctx: &mut TickCtx,
        entity_attachments: Option<
            &crate::fast_hash::FxHashMap<u32, sunrise_mod_runtime::attachments::Attachments>,
        >,
    ) -> Vec<HostEvent> {
        self.tick_level_inner(ctx, entity_attachments, false)
    }

    fn tick_level_inner(
        &mut self,
        ctx: &mut TickCtx,
        entity_attachments: Option<
            &crate::fast_hash::FxHashMap<u32, sunrise_mod_runtime::attachments::Attachments>,
        >,
        publish_players: bool,
    ) -> Vec<HostEvent> {
        if !self.guests.simulation_iter().any(|g| self.watches(g)) {
            // ServerTickRateManager broadcasts a change to the players there
            // are; one who joins later is sent the state as it is then
            // (`updateJoiningPlayer`).
            if !self
                .guests
                .iter()
                .any(|g| g.placed && g.participates_online())
            {
                self.ticking = (ctx.tick_rate, ctx.tick_frozen);
                self.stepping = ctx.tick_steps.revision;
            }
            if self.viewed_pass(ctx) {
                return Vec::new();
            }
            if !self
                .guests
                .iter()
                .any(|g| g.participates_online() && g.dimension == self.config.dimension)
            {
                let _ = ctx.world.block_log.take();
                ctx.world.connection_group_log = None;
                let _ = ctx.world.light_log.take();
                ctx.world.chunks.track_send_ready(false);
                ctx.world.bulk_log.clear();
                ctx.world.chunk_resends.clear();
            }
            return Vec::new();
        }
        if ctx.world.block_log.is_none() {
            ctx.world.block_log = Some(Vec::new());
        }
        // What becomes ready to send from here on is an event for `poll_ready`.
        ctx.world.chunks.track_send_ready(true);
        if ctx.world.connection_group_log.is_none() {
            ctx.world.connection_group_log = Some(Default::default());
        }
        if ctx.world.light_log.is_none() {
            ctx.world.light_log = Some(Default::default());
        }
        crate::spans::time("lan_sync_world", || self.sync_world(ctx));
        crate::spans::time("lan_chunks", || self.update_chunks(ctx));
        crate::spans::time("lan_viewed_levels", || self.send_viewed(ctx));
        crate::spans::time("lan_block_changes", || {
            self.broadcast_light_changes(ctx.world);
            self.broadcast_block_changes(ctx.world);
        });
        // ServerGamePacketListenerImpl.tick: the actions handled since the last tick are
        // acknowledged after the block changes they made were sent, so a client's predicted
        // blocks settle on the server's states.
        for g in self.guests.iter().filter(|g| self.here(g)) {
            let sequence = g.ack_up_to.replace(-1);
            if sequence >= 0 {
                g.send(&C::BlockChangedAck(play::BlockChangedAck { sequence }));
            }
        }
        let events = crate::spans::time("lan_containers", || self.sync_containers(ctx.world));
        self.sync_hidden();
        crate::spans::time("lan_track_entities", || {
            self.track_entities(ctx, entity_attachments)
        });
        if publish_players {
            if self.native_tracking.iter().any(|b| !b.held.is_empty()) {
                // The mods hear the pass's pairings first (`finish_tracking_pass`).
                self.players_after_tracking = true;
            } else {
                self.sync_players();
            }
        }
        events
    }

    /// `ServerPlayer.doTick`'s health, food, experience and effects after a connection tick.
    pub fn sync_guest_after_tick(&mut self, conn: u32) {
        if let Some(i) = self.guests.connection_index(conn)
            && self.here(&self.guests[i])
        {
            self.sync_player_state(i);
            self.sync_effects(i);
        }
    }

    /// `ServerPlayer.doTick`'s health, food, experience and effects for the level's players.
    pub(super) fn sync_players(&mut self) {
        crate::spans::time("lan_player_state", || {
            for i in 0..self.guests.len() {
                if self.here(&self.guests[i]) {
                    self.sync_player_state(i);
                    self.sync_effects(i);
                }
            }
        });
    }

    /// After every level: `PlayerList.tick`.
    pub fn end_tick(&mut self) {
        let latencies: Vec<_> = self
            .guests
            .iter()
            .filter(|g| g.participates_online())
            .map(|g| (g.uuid, g.latency.load(Ordering::Relaxed)))
            .collect();
        // PlayerList.tick: UPDATE_LATENCY once `++sendAllPlayerInfoIn > 600`,
        // so on the 601st tick, then every 601.
        if self.ticks.is_multiple_of(601) && !latencies.is_empty() {
            let entries = latencies
                .iter()
                .map(|(uuid, latency)| play::PlayerInfo {
                    uuid: *uuid,
                    latency: Some(*latency),
                    ..Default::default()
                })
                .collect();
            self.broadcast(&C::PlayerInfoUpdate(play::PlayerInfoUpdate {
                actions: 16,
                entries,
            }));
        }
    }

    /// The starts and stops of tracking the guests had since the last call
    /// are kept in `native_tracking`.
    pub fn collect_tracking(&mut self) {
        self.guests.update_all(|g| {
            take_tracking(
                g,
                &mut self.native_tracking,
                &mut self.attachment_snapshots,
                self.sync_attachments,
                None,
            );
        });
    }

    pub(in crate::lan) fn remove_guest(&mut self, i: usize, reason: &str) -> Guest {
        let behind = self.guests.iter().take(i).map(|g| g.conn).collect();
        let mut g = self.guests.remove(i);
        g.listed_behind = behind;
        super::super::chunk_watch::unwatch_all(&mut self.chunk_watches, &mut g);
        if !g.placed {
            let uuid = g.uuid;
            let generation = g.menu_generation;
            if let Some(menu) = g.menu.as_mut() {
                let inputs = menu.take_processing_inputs(&mut g.native_menu);
                if !inputs.is_empty() {
                    self.native_menu_events.push(
                        super::super::menu::NativeMenuEvent::ReturnProcessingInputs {
                            conn: g.conn,
                            items: inputs,
                            drop_instead: true,
                        },
                    );
                }
                let closed = menu.native_removed(uuid, generation, &mut g.native_menu, false);
                self.native_menu_events.extend(closed);
            }
        }
        // A held player a proxy dropped keeps, in the log, how its connection really ended
        // (an unreadable packet, a reset): the drop says only that it was dropped.
        if g.held
            && !g.placed
            && !reason.is_empty()
            && !g.left_reason.is_empty()
            && g.left_reason != reason
        {
            g.left_reason = format!("{} (the connection ended: {reason})", g.left_reason);
        }
        if g.left_reason.is_empty() {
            g.left_reason = reason.to_owned();
        }
        if std::env::var_os("SUNRISE_NET_DEBUG").is_some() {
            log::info!("[lan] {} left: {reason}", g.name);
        }
        if !g.joined() {
            return g;
        }
        // `PlayerList.remove`: the entities it tracked stop (`removePairing`). The stops stay
        // with the guest: the game tells them after `player_logged_out` (`HostEvent::Left`).
        g.tracker.clear();
        // To the held players too: they were told of this one, and see its entity.
        self.broadcast_watching(&C::PlayerInfoRemove(play::PlayerInfoRemove {
            uuids: vec![g.uuid],
        }));
        self.broadcast_watching(&C::RemoveEntities(play::RemoveEntities {
            ids: vec![g.entity_id],
        }));
        let left = C::SystemChat(play::SystemChat {
            content: yellow_names(
                "multiplayer.player.left",
                vec![self.player_display_name(g.uuid, &g.name)],
            ),
            overlay: false,
        })
        .encode();
        self.guests.update_all(|o| {
            if o.placed && !o.tab_hidden.remove(&g.uuid) && !g.quiet_leave {
                o.sender.send(left.clone());
            }
        });
        g
    }

    /// Takes the frozen player `uuid` out of the world because a proxy moved it to another
    /// server, for the hand-over `id`: the others see it go (its entity and its player-list
    /// entry), but nothing is saved (the other server has the player now) and, unless
    /// `announce`, the "left the game" line is not sent. The game is told it left, marked
    /// released, so it saves nothing.
    ///
    /// Refused when the player is not in that hand-over: another one, or none because the
    /// freeze ran out (the player has played on since, and the record the other server has is
    /// older), or a player who died while frozen.
    pub fn release_moved(
        &mut self,
        uuid: u128,
        id: u128,
        announce: bool,
    ) -> Result<(), &'static str> {
        let i = self.handover_of(uuid, id)?;
        self.guests.update(i, |g| {
            g.quiet_leave = !announce;
            g.left_reason = "moved to another server".to_owned();
            g.sender
                .disconnect(translatable("multiplayer.disconnect.generic", &[]));
        });
        let mut gone = self.remove_guest(i, "moved to another server");
        gone.released = true;
        gone.handover = None;
        self.pending_kicks.push(HostEvent::Left(Box::new(gone)));
        Ok(())
    }

    /// The guest index of the player `uuid` while it is frozen for the hand-over `id`.
    pub fn handover_of(&self, uuid: u128, id: u128) -> Result<usize, &'static str> {
        let Some(i) = self.guests.iter().position(|g| g.uuid == uuid) else {
            return Err("that player is not connected to this server");
        };
        let g = &self.guests[i];
        match &g.handover {
            _ if g.dead => Err("that player died during the hand-over"),
            None => Err("that player is not in a hand-over (or its freeze ran out)"),
            Some(h) if h.id != id => Err("that is not the player's current hand-over"),
            Some(_) => Ok(i),
        }
    }

    /// `ServerTickRateManager.updateStateToClients`: send the current ticking state
    /// and remember it for the next tick's synchronization.
    pub fn send_ticking_state(&mut self, rate: f32, frozen: bool) {
        self.ticking = (rate, frozen);
        self.broadcast(&ticking_state(rate, frozen));
    }

    /// `ServerTickRateManager.updateStepTicks` / `updateStateToClients`:
    /// send clock changes before the block events caused by their ticks.
    pub fn sync_ticking(&mut self, rate: f32, frozen: bool, steps: crate::clock::StepState) {
        if (rate, frozen) != self.ticking {
            self.send_ticking_state(rate, frozen);
        }
        // ServerTickRateManager.updateStepTicks: a step started or stopped.
        if steps.revision != self.stepping {
            self.stepping = steps.revision;
            self.broadcast(&C::TickingStep(play::TickingStep {
                steps: steps.requested,
            }));
        }
    }

    /// Time and weather (ServerLevel.tickTime / advanceWeatherCycle broadcasts).
    fn sync_world(&mut self, ctx: &TickCtx) {
        let world = &*ctx.world;
        // MinecraftServer.tickServer: the game time every 20 ticks
        // (forceGameTimeSynchronization, no clocks).
        if self.ticks.is_multiple_of(20) {
            self.broadcast_level(&C::SetTime(play::SetTime {
                game_time: world.game_time,
                clocks: Vec::new(),
            }));
        }
        // ServerClockManager.modifyClock: a day that jumped (`time set`, `time
        // add`) or a new rate updates the overworld clock.
        let (day, rate) = (self.overworld_day.unwrap_or(world.day_time), ctx.clock_rate);
        let step = i64::from(rate.ceil() as i32).max(1);
        if self.last_clock != i64::MIN
            && ((day - self.last_clock).abs() > step || rate != self.last_rate)
        {
            let overworld = self
                .sync
                .id("minecraft:world_clock", "minecraft:overworld")
                .unwrap_or(0);
            self.broadcast_level(&C::SetTime(play::SetTime {
                game_time: world.game_time,
                clocks: vec![play::ClockState {
                    clock: overworld,
                    total_ticks: day,
                    partial_tick: 0.0,
                    rate,
                }],
            }));
        }
        self.last_clock = day;
        self.last_rate = rate;
        // The same for the clock the level follows when it is not the Overworld's: a jump
        // (`time set` in the level) or a new rate reaches its players at once.
        match self.level_clock.clone() {
            Some((name, level_rate)) => {
                let total = world.day_time;
                let (last_total, last_rate) = self.last_level_clock;
                let step = i64::from(level_rate.ceil() as i32).max(1);
                if last_total != i64::MIN
                    && ((total - last_total).abs() > step || level_rate != last_rate)
                    && let Some(clock) = self.sync.id("minecraft:world_clock", &name)
                {
                    self.broadcast_level(&C::SetTime(play::SetTime {
                        game_time: world.game_time,
                        clocks: vec![play::ClockState {
                            clock,
                            total_ticks: total,
                            partial_tick: 0.0,
                            rate: level_rate,
                        }],
                    }));
                }
                self.last_level_clock = (total, level_rate);
            }
            None => self.last_level_clock = (i64::MIN, 0.0),
        }
        self.sync_ticking(ctx.tick_rate, ctx.tick_frozen, ctx.tick_steps);
        let state = weather::state(world, &self.config.dimension);
        weather::send_updates(Some(self.weather), state, |packet| {
            self.broadcast_level(packet);
        });
        self.weather = state;
    }

    // ---- chunks --------------------------------------------------------

    /// ChunkMap.move's `updateChunkTracking` for guest `i` after its move packet: the readiness that
    /// landed since the last poll first (against the view it had), then its view follows it.
    pub(super) fn follow_view(
        &mut self,
        i: usize,
        world: &World,
        streaming: Option<&crate::streaming::Streaming>,
        ticking: Option<&crate::ticket_levels::Levels>,
    ) {
        if !self.watches(&self.guests[i]) {
            return;
        }
        self.poll_ready(world, streaming, ticking);
        self.update_view(i, Some(&|key| readiness(world, streaming, ticking, key)));
    }

    /// `ChunkMap.tick`'s `updateChunkTracking` for every player here, where `ServerChunkCache.tick`
    /// runs it, after the changed chunks' broadcast: the readiness since the last poll first, then each
    /// view follows its player (a teleport, a changed view distance).
    pub fn update_views(
        &mut self,
        world: &World,
        streaming: Option<&crate::streaming::Streaming>,
        ticking: Option<&crate::ticket_levels::Levels>,
    ) {
        self.poll_ready(world, streaming, ticking);
        for i in 0..self.guests.len() {
            if self.watches(&self.guests[i]) {
                self.update_view(i, Some(&|key| readiness(world, streaming, ticking, key)));
            }
        }
    }

    /// Every player's chunks here: the columns that became ready (`poll_ready`), then each
    /// player's view (`update_view`) and its next batch (PlayerChunkSender.sendNextChunks). The
    /// batches are chosen first, the columns in them encoded once each, their new sections and
    /// their light on the warm-up threads too (`ChunkEncoder::warm`, `bodies_with`), then sent:
    /// nothing changes the level in between, and each player's packets keep their order.
    pub(super) fn update_chunks(&mut self, ctx: &mut TickCtx) {
        let (streaming, ticking) = (ctx.streaming, ctx.ticking.as_ref());
        self.poll_ready(ctx.world, streaming, ticking);
        let world = &*ctx.world;
        let watching = self.chunk_watches.recording();
        let mut batches = Vec::new();
        for i in 0..self.guests.len() {
            if self.watches(&self.guests[i]) {
                let ready = |key| readiness(world, streaming, ticking, key);
                self.update_far(i, Some(&ready));
                self.update_view(i, Some(&ready));
                if let Some(batch) = self.next_batch(i, world, streaming, ticking) {
                    batches.push((i, batch));
                }
            }
        }
        if batches.is_empty() {
            return;
        }
        // Each column once, however many players it goes to, with the
        // number of batches that take it.
        let mut columns = Columns::default();
        let batches: Vec<_> = batches
            .into_iter()
            .map(|(i, batch)| {
                let (taken, bodies): (Vec<_>, Vec<_>) = batch
                    .into_iter()
                    .map(|(key, chunk)| (key, columns.add(chunk)))
                    .unzip();
                (i, taken, bodies)
            })
            .collect();
        self.encoder.warm(&columns.chunks);
        let encoded_results = self.encoder.bodies_with(&columns.chunks, world, &|x, z| {
            ctx.streaming.and_then(|s| s.proto_sections(x, z))
        });
        let mut encoded: Vec<Option<Vec<u8>>> = encoded_results
            .into_iter()
            .enumerate()
            .map(|(j, result)| match result {
                Ok(body) => Some(body),
                Err(error) => {
                    let chunk = columns.chunks[j];
                    log::warn!(
                        "[lan] chunk ({}, {}) could not be encoded for streaming: {error}",
                        chunk.x,
                        chunk.z
                    );
                    None
                }
            })
            .collect();
        let uses = &mut columns.uses;
        let chunks = &columns.chunks;
        for (i, taken, batch) in batches {
            let mut sent_taken = Vec::with_capacity(taken.len());
            let mut sent_batch = Vec::with_capacity(batch.len());
            let mut failed_taken = Vec::new();
            for (key, j) in taken.into_iter().zip(batch) {
                if encoded[j].is_some() {
                    sent_taken.push(key);
                    sent_batch.push(j);
                } else {
                    failed_taken.push(key);
                }
            }
            self.guests.update(i, |g| {
                let size = sent_batch.len();
                // The single-player window shares the sections it is sent (handoff.rs).
                let memory = g.address() == super::super::conn::MEMORY_ADDRESS;
                let sync_groups = g.negotiated_connection_groups();
                if size > 0 {
                    g.batch_quota -= size as f32;
                    g.send(&C::ChunkBatchStart(sunrise_protocol::packets::Empty));
                    let grouped = sync_groups
                        && sent_batch
                            .iter()
                            .any(|&j| !chunks[j].connection_groups.is_empty());
                    let bodies = sent_batch.into_iter().flat_map(|j| {
                        let chunk = chunks[j];
                        if memory {
                            super::super::handoff::offer(chunk);
                        }
                        let groups = if grouped && !chunk.connection_groups.is_empty() {
                            block_changes::encode_chunk_connection_groups(chunk)
                        } else {
                            Vec::new()
                        };
                        uses[j] -= 1;
                        let body = if uses[j] == 0 {
                            encoded[j].take().expect("valid chunk body")
                        } else {
                            encoded[j].as_ref().expect("valid chunk body").clone()
                        };
                        block_changes::group_bundle(body, groups)
                    });
                    if grouped {
                        g.sender.send_batch(bodies);
                    } else {
                        g.sender.send_many(bodies);
                    }
                    g.send(&C::ChunkBatchFinished(play::ChunkBatchFinished {
                        size: size as i32,
                    }));
                    g.unacknowledged += 1;
                }
                if watching {
                    // ChunkMap.markChunkPendingToSend, then PlayerChunkSender.sendChunk's end:
                    // Sunrise picks a chunk only as it sends it.
                    for &key in &sent_taken {
                        if g.watched.insert(key) {
                            self.chunk_watches.push(g, ChunkWatchKind::Watch, key);
                        }
                    }
                    for &key in &sent_taken {
                        self.chunk_watches.push(g, ChunkWatchKind::Sent, key);
                    }
                }
                // `collectChunksToSend`: the chosen chunks leave the pending ones; the ones the
                // encoder could not write are not sent.
                for key in sent_taken.iter().chain(&failed_taken) {
                    g.pending_chunks.remove(&pack(*key));
                }
                let seen = sent_taken;
                if self.sync_attachments && !seen.is_empty() {
                    queue_snapshots(
                        &mut self.attachment_snapshots,
                        g.uuid,
                        seen.iter().map(|&(x, z)| Holder::Chunk([x, z])),
                    );
                }
                for key in seen {
                    g.sent_chunks.insert(key);
                }
            });
        }
    }

    /// ChunkMap.move's view change (`applyChunkTrackingView`): the new centre, then the chunks that
    /// entered and left the view in `ChunkTrackingView.difference`'s walk. An entering chunk joins
    /// the pending ones when `ready` says it is ready to send (`markChunkPendingToSend`); one that
    /// isn't joins them when it becomes so (`poll_ready`), and with no level to ask (`None`) the
    /// next poll asks. A leaving one that was still pending is dropped silently, any other is
    /// forgotten (`PlayerChunkSender.dropChunk`).
    pub(super) fn update_view(
        &mut self,
        i: usize,
        ready: Option<&dyn Fn((i32, i32)) -> Readiness>,
    ) {
        let (cx, cz) = self.guests[i].chunk();
        let r = self.guests[i].view_distance;
        self.guests.update(i, |g| {
            if g.center == Some((cx, cz)) && !g.view_dirty && g.tracked_view.is_some() {
                return;
            }
            let moved = g.center != Some((cx, cz));
            g.center = Some((cx, cz));
            g.view_dirty = false;
            if moved && g.center_sent.take() != Some((cx, cz)) {
                g.send(&C::SetChunkCacheCenter(play::SetChunkCacheCenter {
                    x: cx,
                    z: cz,
                }));
            }
            let next = ((cx, cz), r);
            let last = g.tracked_view.replace(next);
            let fresh = last.is_none();
            for (key, sees) in view_difference(last, next) {
                if sees {
                    // A level's views swapped in keep the chunks they were sent.
                    if g.sent_chunks.contains(&key) {
                        continue;
                    }
                    match ready.map(|ready| ready(key)) {
                        Some(r) if goes_to(g, true, key, r) => {
                            g.pending_chunks.insert(&pack(key));
                        }
                        // Lit but not ready: asked again at each poll.
                        Some(Readiness::Held | Readiness::Loaded) => {
                            waiters(&mut self.held, &self.config.dimension)
                                .entry(key)
                                .or_insert(Readiness::Held);
                        }
                        Some(_) => {}
                        None => g.recheck_chunks.push(key),
                    }
                } else if !g.viewed.own.kept.contains(&key) {
                    // One an area keeps past the view stays (`viewed_levels::own`).
                    drop_chunk(&mut self.chunk_watches, g, key);
                }
            }
            // A view that starts afresh while its client still holds chunks: a viewed level's,
            // swapped in as the player's own (`viewed_levels::switch`). Those this view doesn't
            // track, nor an area keeps, leave as a vanilla view's do, by x then z.
            if fresh {
                let kept = &g.viewed.own.kept;
                let mut past: Vec<_> = g
                    .sent_chunks
                    .iter()
                    .copied()
                    .filter(|&(x, z)| !tracked(cx, cz, r, x, z) && !kept.contains(&(x, z)))
                    .collect();
                past.sort_unstable();
                for key in past {
                    drop_chunk(&mut self.chunk_watches, g, key);
                }
            }
        });
    }

    /// `ChunkMap.onChunkReadyToSend`, polled before every view change, as vanilla's readiness lands
    /// before the move that follows it: each column that came in lit since the last poll (in the
    /// order it did: `ChunkMap::take_send_ready`) and is ready to send joins the pending chunks of
    /// every player here whose view holds it now. One lit but not ready yet (a neighbour not at
    /// FULL, the ticket level not ticking) waits in `held` and is asked again at each poll: with
    /// the column of this poll that was its last FULL neighbour, in that column's place (vanilla's
    /// range future completes when its last neighbour does), else in key order before them; the
    /// chunks a view change couldn't ask about (`recheck_chunks`) are asked here, in the order
    /// they entered. Chunks in no view are never probed. Past the view, a column an area keeps
    /// (`viewed_levels::own`) joins once it is full and lit, its ticket level ticking or not.
    pub(super) fn poll_ready(
        &mut self,
        world: &World,
        streaming: Option<&crate::streaming::Streaming>,
        ticking: Option<&crate::ticket_levels::Levels>,
    ) {
        // What became ready to send (`Ready`), or full past the view (`Loaded`), this poll.
        let mut became = Vec::new();
        let mut waiting = self.held.remove(&self.config.dimension).unwrap_or_default();
        let events = world.chunks.take_send_ready();
        arrivals(&mut waiting, &mut became, &events, |key, later| {
            readiness_before(world, streaming, ticking, key, later)
        });
        for i in 0..self.guests.len() {
            if !self.watches(&self.guests[i]) {
                continue;
            }
            let held = &mut waiting;
            self.guests.update(i, |g| {
                let Some(((cx, cz), r)) = g.tracked_view else {
                    g.recheck_chunks.clear();
                    return;
                };
                for &(key, now) in &became {
                    if goes_to(g, tracked(cx, cz, r, key.0, key.1), key, now) {
                        g.pending_chunks.insert(&pack(key));
                    }
                }
                for key in std::mem::take(&mut g.recheck_chunks) {
                    let packed = pack(key);
                    let in_view = tracked(cx, cz, r, key.0, key.1);
                    if !(in_view || g.viewed.own.sends(key))
                        || g.pending_chunks.contains(&packed)
                        || g.sent_chunks.contains(&key)
                    {
                        continue;
                    }
                    match readiness(world, streaming, ticking, key) {
                        r if goes_to(g, in_view, key, r) => {
                            g.pending_chunks.insert(&packed);
                        }
                        Readiness::Loaded | Readiness::Held => {
                            held.entry(key).or_insert(Readiness::Held);
                        }
                        _ => {}
                    }
                }
            });
        }
        if !waiting.is_empty() {
            self.held.insert(self.config.dimension.clone(), waiting);
        }
    }

    /// PlayerChunkSender.sendNextChunks' choice (`collectChunksToSend`): as many as the batch
    /// quota the client's acknowledgements pace allows, from the pending chunks in their set's
    /// order, the nearest to the player's chunk first: `Comparators.least` when there are more
    /// than that, else all of them sorted, equally near ones in the set's order. A memory
    /// connection (single player's own window) always takes every pending chunk, and its quota
    /// goes below zero by the batch, as vanilla's does. The chunks chosen leave the pending ones
    /// as `update_chunks` sends them.
    pub(super) fn next_batch<'w>(
        &mut self,
        i: usize,
        world: &'w World,
        streaming: Option<&crate::streaming::Streaming>,
        ticking: Option<&crate::ticket_levels::Levels>,
    ) -> Option<Vec<((i32, i32), &'w Chunk)>> {
        let budget = self.guests.update(i, |g| {
            if self.ticks <= g.placed_at + 1 || g.unacknowledged >= g.max_unacknowledged {
                return None;
            }
            g.batch_quota = (g.batch_quota + g.chunks_per_tick).min(g.chunks_per_tick.max(1.0));
            if g.batch_quota < 1.0 || g.pending_chunks.is_empty() {
                return None;
            }
            Some(g.batch_quota.floor() as usize)
        })?;
        let g = &self.guests[i];
        let memory = g.address() == super::super::conn::MEMORY_ADDRESS;
        let (px, pz) = g.chunk();
        // `ChunkPos.distanceSquared`, in int arithmetic.
        let near = |p: i64| {
            let (x, z) = unpack(p);
            let (dx, dz) = (x.wrapping_sub(px), z.wrapping_sub(pz));
            dx.wrapping_mul(dx).wrapping_add(dz.wrapping_mul(dz))
        };
        // A column an area keeps past the view (`viewed_levels::own`, a Sunrise client's) goes
        // after the view's, nearest to an area's centre first; with none, the order is vanilla's.
        let own = &g.viewed.own;
        let past = |(x, z): (i32, i32)| {
            own.sends((x, z))
                && !g
                    .tracked_view
                    .is_some_and(|((cx, cz), r)| tracked(cx, cz, r, x, z))
        };
        let order = |p: i64| {
            let (x, z) = unpack(p);
            if past((x, z)) {
                (
                    1,
                    super::super::viewed_levels::view::distance(&own.sources, x, z),
                )
            } else {
                (0, near(p))
            }
        };
        let chosen: Vec<i64> = if !memory && g.pending_chunks.len() > budget {
            crate::top_k::least(g.pending_chunks.stream(), budget, order)
        } else {
            let mut all: Vec<i64> = g.pending_chunks.stream().collect();
            all.sort_by_key(|&p| order(p));
            all
        };
        // `getChunkToSend` on each pick: one not ready to send now (unloaded, or back unlit) stays
        // pending. Past the view, one an area keeps goes once it is full and lit, and so does one
        // in a held arrival's view (`goes_to`).
        let far = |key: (i32, i32)| {
            readiness(world, streaming, ticking, key) == Readiness::Loaded
                && (past(key) || (g.held && !g.placed))
        };
        let batch: Vec<_> = chosen
            .into_iter()
            .filter_map(|p| {
                let key = unpack(p);
                (send_ready(world, streaming, ticking, key) || far(key)).then_some(())?;
                Some((key, &**world.chunks.get(&key)?))
            })
            .collect();
        (!batch.is_empty()).then_some(batch)
    }

    /// `ChunkMap.resendBiomesForChunks`: each player in the level is sent,
    /// in one packet, the biomes of those of `columns` it has been sent
    /// (`getPlayers(pos, false)`), in the order given.
    pub fn resend_biomes(&mut self, world: &World, columns: &[(i32, i32)]) {
        let data: Vec<_> = columns
            .iter()
            .filter_map(|key| Some((*key, self.encoder.biome_data(world.chunks.get(key)?))))
            .collect();
        let watching: Vec<_> = self
            .guests
            .simulation_iter()
            .filter(|g| self.watches(g))
            .collect();
        let bodies =
            super::super::biome_resend::bodies(watching.iter().map(|g| &g.sent_chunks), &data);
        for (g, body) in watching.into_iter().zip(bodies) {
            if let Some(body) = body {
                g.send_chunks_biomes_body(body);
            }
        }
    }

    /// `ChunkHolder.broadcastChanges` where `ServerChunkCache.tick` runs it,
    /// before the block events: what changed since the last time (the last
    /// tick's block events, entities and block entities, this tick's
    /// scheduled and random ticks), as it is now.
    pub fn broadcast_changes(&mut self, world: &mut World) {
        if self.guests.iter().any(|g| self.here(g)) || self.viewed_here() {
            self.broadcast_light_changes(world);
            self.broadcast_block_changes(world);
        }
    }
}
