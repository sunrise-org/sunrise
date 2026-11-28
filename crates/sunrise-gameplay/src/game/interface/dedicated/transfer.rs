//! A proxy's requests on the control endpoint, for a player it moves between two servers:
//! `player-expect <uuid> [seconds]` (the target is told who is coming; its login is held),
//! `player-drop <uuid>` (the target forgets it, and lets a held player go),
//! `player-export <uuid> [freeze]` (the source answers with the player's record) and
//! `player-release <uuid> <hand-over> moved [announce] | abort` (the source lets the player go),
//! `player-import <uuid> <json>` and `player-confirm <uuid> <hand-over>` (the target takes the
//! record and lets the held player in), and `transfer-status` (the counts a proxy reads before
//! it sends players here). They run
//! between two ticks, on the server thread, so the player is never caught half-way through a
//! tick. The rules are `lan/arrivals.rs`'s. All of them are refused while `server.toml`'s
//! `[transfer]` is off, and those that bring a player in once the server has begun to stop.
use serde_json::{Value, json};
use std::time::Instant;
use sunrise_gameplay_common::game::Dim;
use sunrise_protocol::packets::play::{self, Clientbound as C};
use sunrise_sim::lan::arrivals;

impl crate::game::ServerGame<'_> {
    /// A proxy's `player-*` request; the reply is one JSON line, `{"ok": true, ...}` or
    /// `{"error": "..."}`. Each request writes one line to the log with what became of it
    /// (`arrivals::request_log`): a hand-over that failed leaves a trace.
    pub(super) fn player_request(&mut self, verb: &str, args: &str) -> Value {
        // The name is read before the request runs: a release or a drop takes the player away.
        let name = args
            .split_whitespace()
            .next()
            .and_then(arrivals::parse_uuid)
            .and_then(|uuid| {
                self.server
                    .lan
                    .as_ref()?
                    .hub
                    .guests
                    .iter()
                    .find(|g| g.uuid == uuid)
                    .map(|g| g.name.clone())
            });
        let reply = self.run_player_request(verb, args);
        let (level, line) = arrivals::request_log(verb, name.as_deref(), args, &reply);
        super::log(level, &line);
        reply
    }

    /// Once a stop has begun (`/stop` from the console, a player or the control endpoint, or a stop
    /// signal), the transfers know it before the next request is answered: what would bring a player
    /// in is refused from then on, and the ways out stay open (`arrivals::Transfers::stopping`).
    pub(super) fn note_stopping(&mut self, stop: bool) {
        let stopping = stop || sunrise_threads::stop_requests() > 0;
        if let Some(transfers) = self
            .server
            .lan
            .as_mut()
            .and_then(|h| h.hub.transfers.as_mut())
        {
            transfers.stopping |= stopping;
        }
    }

    /// Whether the tick loop ends now. A stop (`/stop`, a stop signal) ends it at once, unless
    /// `[transfer]` is on and players are here: the stop then waits for a proxy to move them out
    /// (`arrivals::StopWait`), every login refused, the server ticking on. A second `stop` ends the
    /// wait; a second signal ends the process (`sunrise_threads::catch_stop_signals`).
    pub(super) fn stop_now(&mut self, state: &super::ServerState) -> bool {
        let asked = std::mem::take(&mut state.lists.lock().unwrap().stop);
        let stop = asked || sunrise_threads::stop_requests() > 0;
        let Some(host) = self.server.lan.as_mut() else {
            return stop;
        };
        let waiting = host
            .hub
            .transfers
            .as_ref()
            .is_some_and(|t| t.stop_wait.is_some());
        if !stop && !waiting {
            return false;
        }
        let (placed, frozen) = arrivals::placed_and_frozen(&host.hub);
        let Some(transfers) = host.hub.transfers.as_mut() else {
            return stop;
        };
        let now = Instant::now();
        // A freeze lasts ticks: how long one takes now, for `check_freeze`.
        let period = arrivals::tick_period(self.common.clock.tick_duration(), state.transfer_tps());
        match &mut transfers.stop_wait {
            None => {
                transfers.stopping = true;
                let wait = arrivals::StopWait::start(&transfers.settings, now, period);
                let Some(wait) = wait.filter(|_| placed > 0) else {
                    return true;
                };
                transfers.stop_wait = Some(wait);
                host.hub.lan.shared.refuse_logins();
                super::log(
                    "INFO",
                    &arrivals::stop_wait_line(transfers.settings.stop_wait),
                );
                false
            }
            Some(_) if asked => {
                super::log("INFO", "A second stop: stopping now");
                true
            }
            Some(wait) => {
                wait.period = period;
                let over = wait.over(now, placed, frozen);
                if over && frozen > 0 {
                    super::log(
                        "WARN",
                        &format!(
                            "Stopping with {frozen} hand-over(s) still frozen: their players are saved here"
                        ),
                    );
                }
                over
            }
        }
    }

    /// `transfer-status`: the counts a proxy reads before it sends players here
    /// (`arrivals::status_request`). A proxy asks it every few seconds, so it writes nothing to
    /// the log.
    pub(super) fn transfer_status(&self) -> Value {
        match self.server.lan.as_ref() {
            Some(host) => arrivals::status_request(&host.hub, Instant::now()),
            None => json!({ "error": "the server is not open to players" }),
        }
    }

    fn run_player_request(&mut self, verb: &str, args: &str) -> Value {
        let Some(host) = self.server.lan.as_ref() else {
            return json!({ "error": "the server is not open to players" });
        };
        if host.hub.transfers.is_none() {
            return json!({ "error": arrivals::OFF });
        }
        match verb {
            "player-expect" => match self.server.lan.as_mut() {
                Some(host) => arrivals::expect_request(&mut host.hub, args, Instant::now()),
                None => json!({ "error": "the server is not open to players" }),
            },
            "player-drop" => match self.server.lan.as_mut() {
                Some(host) => arrivals::drop_request(&mut host.hub, args),
                None => json!({ "error": "the server is not open to players" }),
            },
            "player-export" => self.player_export(args),
            "player-import" => self.player_import(args),
            "player-confirm" => self.player_confirm(args),
            "player-release" => match self.server.lan.as_mut() {
                Some(host) => arrivals::release_request(&mut host.hub, args),
                None => json!({ "error": "the server is not open to players" }),
            },
            other => json!({ "error": format!("unknown request {other:?}") }),
        }
    }

    /// `player-export <uuid> [freeze]`: closes the player's menus as vanilla does when a menu closes
    /// (the cursor stack and the crafting grids go back into the inventory), then answers with the
    /// player's record (`arrivals::export_record`). With `freeze`, a hand-over starts: the player's
    /// own inputs stop being acted on until it ends, and a later export answers with the same record.
    fn player_export(&mut self, args: &str) -> Value {
        let mut parts = args.split_whitespace();
        let Some(uuid) = parts.next().and_then(arrivals::parse_uuid) else {
            return json!({ "error": "player-export needs the player's unique id" });
        };
        let freeze = match parts.next() {
            None => false,
            Some("freeze") => true,
            Some(_) => return json!({ "error": "the only option of player-export is freeze" }),
        };
        if parts.next().is_some() {
            return json!({ "error": "player-export takes a unique id and at most freeze" });
        }
        self.export_from(uuid, freeze)
    }

    /// `player-import <uuid> <json>`: applies a record to the held player (`lan/import.rs`), with
    /// its advancements and statistics, which need the game's tree and counters, and the mods'
    /// attachments. Unless the request says `"place": false`, it then places the player as
    /// `player-confirm` does, its file written first.
    fn player_import(&mut self, args: &str) -> Value {
        let Some((id, request)) = args.trim().split_once(char::is_whitespace) else {
            return json!({ "error": "player-import needs a unique id and a request" });
        };
        let Some(uuid) = arrivals::parse_uuid(id) else {
            return json!({ "error": "player-import needs the player's unique id" });
        };
        let request: Value = match serde_json::from_str(request.trim()) {
            Ok(v) => v,
            Err(e) => return json!({ "error": format!("the request is not JSON: {e}") }),
        };
        let Some(host) = self.server.lan.as_mut() else {
            return json!({ "error": "the server is not open to players" });
        };
        // A retry after the commit, whose reply was lost: the player is placed from this
        // hand-over already, and the answer is the same.
        let retried = request["record"]["handover"]
            .as_str()
            .and_then(arrivals::parse_handover)
            .and_then(|id| host.hub.placed_arrival(uuid, id).map(|g| (id, g)));
        if let Some((id, i)) = retried {
            return json!({
                "ok": true,
                "handover": arrivals::handover_text(id),
                "placed": true,
                "announced": host.hub.guests[i].arrival_announce,
                "recentered": host.hub.guests[i].arrival_recentered,
                "retried": true,
            });
        }
        let result = sunrise_sim::lan::import::import_player(
            &mut host.hub,
            self.common.motion.attributes(),
            uuid,
            &request,
        );
        match result {
            Ok(done) => {
                self.take_arrival_records(uuid, &done);
                self.sync_replaced_book_settings(uuid);
                let handover = arrivals::handover_text(done.handover);
                let placed = if done.place {
                    self.place_arrival(uuid, done.handover)
                } else {
                    Ok(false)
                };
                match placed {
                    Ok(placed) => json!({
                        "ok": true,
                        "handover": handover,
                        "parts": done.parts,
                        "placed": placed,
                        "announced": done.announce,
                        "recentered": done.recentered,
                    }),
                    Err(e) => json!({ "error": e }),
                }
            }
            Err(e) => json!({ "error": e }),
        }
    }

    /// What an import brought that the game keeps itself: the advancements (`PlayerAdvancements
    /// .applyFrom` drops those this server lacks) and the statistics.
    fn take_arrival_records(&mut self, uuid: u128, done: &sunrise_sim::lan::import::Imported) {
        let tree = self.advancement_tree();
        let host = self.server.lan.as_mut().expect("server host is present");
        let Some(index) = host.hub.guests.iter().position(|g| g.uuid == uuid) else {
            return;
        };
        host.hub.guests.update(index, |g| {
            if let Some(saved) = &done.advancements {
                let mut progress = sunrise_sim::advancements::Progress::from_json(saved);
                progress
                    .criteria
                    .retain(|id, _| tree.by_id.contains_key(id));
                g.advancements = progress;
                g.advancements_loaded = true;
            }
            if let Some(stats) = &done.stats {
                let play = host.play.entry(g.conn).or_default();
                play.stats = crate::game::stats::Stats::from_json(stats);
                play.stats.mark_all_dirty();
            }
        });
    }

    /// `player-confirm <uuid> <hand-over>`: places a held player whose record of that hand-over was
    /// imported without placing it.
    fn player_confirm(&mut self, args: &str) -> Value {
        let Some(host) = self.server.lan.as_ref() else {
            return json!({ "error": "the server is not open to players" });
        };
        match arrivals::confirm_args(&host.hub, args) {
            // A retry after the commit, whose reply was lost: the same answer.
            Ok((uuid, id)) if host.hub.placed_arrival(uuid, id).is_some() => {
                let i = host.hub.placed_arrival(uuid, id).unwrap_or_default();
                json!({
                    "ok": true,
                    "placed": true,
                    "announced": host.hub.guests[i].arrival_announce,
                    "retried": true,
                })
            }
            Ok((uuid, id)) => match self.place_arrival(uuid, id) {
                Ok(_) => {
                    let announced = self
                        .server
                        .lan
                        .as_ref()
                        .expect("server host is present")
                        .hub
                        .guests
                        .iter()
                        .find(|g| g.uuid == uuid)
                        .is_some_and(|g| g.arrival_announce);
                    json!({ "ok": true, "placed": true, "announced": announced })
                }
                Err(e) => json!({ "error": e }),
            },
            Err(e) => json!({ "error": e }),
        }
    }

    /// The hand-over's commit point on the target: the arrival's player file, advancements and
    /// statistics are written, synchronously, then it is placed. Only then does the reply say
    /// `ok`, so a proxy that sends `player-release moved` to the source after it knows a copy of
    /// the player is on disk here. When the file cannot be written the player stays held and
    /// imported: the proxy can confirm again or drop it.
    fn place_arrival(&mut self, uuid: u128, id: u128) -> Result<bool, String> {
        self.server
            .lan
            .as_ref()
            .expect("server host is present")
            .hub
            .imported_arrival(uuid, id)?;
        // An autosave of this player still in flight must not land after this write.
        // Its completion fires save hooks, so the Host must still see the server.
        let _ = self.finish_player_autosaves(true);
        let host = self.server.lan.as_mut().expect("server host is present");
        let i = host.hub.imported_arrival(uuid, id)?;
        let g = host.hub.guests.take_for_save(i);
        let written = sunrise_sim::lan::playerdata::save_now(
            &g,
            &self.server.world_root,
            self.common.res.items(),
        )
        .and_then(|()| {
            self.write_arrival_records(
                self.server.lan.as_ref().expect("server host is present"),
                &g,
            )
        });
        let name = g.name.clone();
        let host = self.server.lan.as_mut().expect("server host is present");
        host.hub.guests.insert(i, g);
        written?;
        let announced = host.hub.confirm_arrival(uuid, id)?;
        let directory = sunrise_sim::lan::playerdata::directory(&self.server.world_root);
        self.native_player_save_to_file(uuid, &name, &directory);
        self.arrival_placed(uuid, &name, announced);
        Ok(true)
    }

    /// What the game does once a held player is placed: the log's lines, and the mods' "logged
    /// in" (`native_player_logged_in`), which a held login did not send, so a placed arrival
    /// is heard of as any login and its leaving later is not the mods' first word of it.
    fn arrival_placed(&mut self, uuid: u128, name: &str, announced: bool) {
        super::log("INFO", &arrivals::placed_line(name));
        if announced {
            super::log("INFO", &format!("System chat: {name} joined the game"));
        }
        self.announce_login(uuid, name);
    }

    fn export_from(&mut self, uuid: u128, freeze: bool) -> Value {
        let host = self.server.lan.as_ref().expect("server host is present");
        let i = match arrivals::check_exportable(&host.hub, uuid) {
            Ok(i) => i,
            Err(why) => return json!({ "error": why }),
        };
        if host.hub.guests[i].is_fake() {
            return json!({ "error": "fake players cannot be exported" });
        }
        if freeze && let Err(why) = arrivals::check_freeze(&host.hub, i, Instant::now()) {
            return json!({ "error": why });
        }
        // A repeat while the hand-over lasts: the same record, and nothing touched.
        if let Some(h) = &host.hub.guests[i].handover {
            return arrivals::export_reply(&host.hub, i, h.record.clone());
        }
        let conn = host.hub.guests[i].conn;
        let window = host.hub.guests[i].menu.as_ref().map(|m| m.window);
        // The menu closes in the player's own level, as when it leaves (`game/network/lan/mod.rs`'s close of
        // every guest): a chest's lid, a villager's trade and a dropped cursor stack belong there.
        let back = self.common.dim;
        let level = Dim::from_id(&host.hub.guests[i].dimension);
        if let Err(e) = self.enter_level(level) {
            return json!({ "error": format!("the player's level could not be opened: {e}") });
        }
        let hold = self.guest_close_menus_for_save(conn);
        if let Err(e) = self.enter_level(back) {
            log::error!("could not reopen {}: {e}", back.id());
        }
        let host = self.server.lan.as_mut().expect("server host is present");
        let Some(i) = host.hub.guests.iter().position(|g| g.conn == conn) else {
            self.resume_menu_opens(hold);
            return json!({ "error": "that player is not connected to this server" });
        };
        // The player may stay (a hand-over that does not finish): its client closes the menu too
        // and shows the inventory as it now is.
        if let Some(container) = window {
            host.hub
                .send_to(conn, &C::ContainerClose(play::ContainerClose { container }));
        }
        host.hub.send_inventory(i);
        let advancements = if self.common.adv.tree.list.is_empty() {
            Value::Null
        } else {
            host.hub.guests[i]
                .advancements
                .to_json(&self.common.adv.tree, sunrise_world::DATA_VERSION)
        };
        let stats = host
            .play
            .get(&conn)
            .map_or(Value::Null, |p| p.stats.to_json());
        let records = arrivals::Records {
            advancements,
            stats,
        };
        match arrivals::export_record(&mut host.hub, i, freeze, records) {
            Ok(record) => {
                let reply = arrivals::export_reply(&host.hub, i, record);
                // The player leaves with the record: a menu the close asked for never opens.
                self.discard_menu_opens(hold);
                reply
            }
            Err(e) => {
                self.resume_menu_opens(hold);
                json!({ "error": e })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::{
        ServerGame,
        native_entity_refs::tests::with_game_at,
        native_guest_turn_tests::{open_fixture_hub, with_guests},
    };
    use mod_api::{Flow, Hooks, HostSide, ModEntry, ServerHost};
    use std::sync::{Arc, Mutex};
    use sunrise_core::{
        geometry::Vec3,
        nbt::Tag,
        random::{LegacyRandom, RandomSource},
    };
    use sunrise_game_data::nbt_value::{float, string, text};
    use sunrise_protocol::{
        PROTOCOL_VERSION,
        connection::Connection,
        crypto::offline_uuid,
        packets::{Empty, configuration as cfg, handshake, login},
    };
    use sunrise_sim::lan::{
        configuration_tasks::{PreparedSpawn, SpawnLevel, SpawnTask},
        playerdata,
    };

    const CONN: u32 = 12;
    const UUID: u128 = 12;
    const PREPARED: [f64; 3] = [6.5, 70.0, 1.5];
    const SAVED: [f64; 3] = [4.5, 69.0, 2.5];

    const PREPARED_SPAWN: PreparedSpawn = PreparedSpawn {
        level: SpawnLevel {
            id: "minecraft:overworld",
            generation: 0,
        },
        position: PREPARED,
        rotation: [40.0, 10.0],
    };

    fn write_saved_player(game: &mut ServerGame<'_>, player: u128) {
        game.server.world_root = game.server.save.0.root.clone();
        let tag = Tag::compound([
            (
                "Pos",
                Tag::List(SAVED.map(|v| Tag::Double(v.to_bits())).to_vec()),
            ),
            ("Health", Tag::Float(25.0f32.to_bits())),
            (
                "attributes",
                Tag::List(
                    [("minecraft:max_health", 30.0f64), ("minecraft:scale", 0.5)]
                        .map(|(name, base)| {
                            Tag::compound([
                                ("id", string(name)),
                                ("base", Tag::Double(base.to_bits())),
                            ])
                        })
                        .to_vec(),
                ),
            ),
        ]);
        sunrise_sim::save::write_dat(&playerdata::path(&game.server.world_root, player), tag)
            .unwrap();
    }

    fn join_held_player(game: &mut ServerGame<'_>) {
        write_saved_player(game, UUID);
        game.server
            .lan
            .as_mut()
            .unwrap()
            .hub
            .update_guest(CONN, |guest| {
                assert!(guest.motion.is_none());
                guest.placed = false;
                guest.expected = true;
            })
            .unwrap();
        game.lan_guest_joined(CONN, &[], PREPARED, Some(PREPARED_SPAWN));
    }

    /// Drive a configured join through the packet handler and check that each observed hook
    /// fires once in order, including `datapack_sync` for the joining player after file load
    /// and before commands. Check player visibility and that commands see saved health at
    /// the prepared spawn position.
    #[test]
    #[ignore = "daily"]
    fn prepared_join_calls_login_then_announces_once() {
        use std::time::Duration;
        use sunrise_core::prepared_test_support::{SERVER_INPUTS, WORLDGEN_INPUTS};

        const NAME: &str = "PreparedJoin";
        const WAIT: Duration = Duration::from_secs(10);
        #[derive(Debug, PartialEq, Eq)]
        enum Stage {
            BeforePlacement,
            Entities,
            Construct,
            File,
            DatapackSync,
            Commands,
            ClientInformation,
            Announced,
        }
        struct Observe {
            player: u128,
            stages: Arc<Mutex<Vec<Stage>>>,
        }
        #[mod_api::__subscriptions]
        impl Hooks for Observe {
            fn player_login(
                &self,
                host: &mut dyn ServerHost,
                event: &mut mod_api::PlayerLoginEvent<'_>,
            ) -> Flow {
                assert_eq!(event.name, NAME);
                assert_eq!(event.player, self.player);
                assert!(host.player(event.player).is_none());
                self.stages.lock().unwrap().push(Stage::BeforePlacement);
                Flow::Continue
            }
            fn entities_load(
                &self,
                _: &mut dyn ServerHost,
                event: &mut mod_api::EntitiesLoadEvent<'_>,
            ) {
                assert_eq!((event.chunk, event.count), ([0, 0], 1));
                self.stages.lock().unwrap().push(Stage::Entities);
            }
            fn entity_constructing(
                &self,
                host: HostSide<'_>,
                event: &mut mod_api::EntityConstructingEvent,
            ) {
                if matches!(host, HostSide::Server(_))
                    && event.holder == mod_api::Holder::Player(self.player)
                {
                    self.stages.lock().unwrap().push(Stage::Construct);
                }
            }
            fn player_load_from_file(
                &self,
                host: &mut dyn ServerHost,
                event: &mut mod_api::PlayerLoadFromFileEvent<'_>,
            ) {
                assert_eq!(event.player, self.player);
                assert!(host.player(event.player).is_none());
                self.stages.lock().unwrap().push(Stage::File);
            }
            fn datapack_sync(
                &self,
                _: &mut dyn ServerHost,
                event: &mut mod_api::DatapackSyncEvent<'_>,
            ) {
                assert_eq!(event.player.uuid(), self.player);
                self.stages.lock().unwrap().push(Stage::DatapackSync);
            }
            fn player_commands_sent(
                &self,
                host: &dyn mod_api::Host,
                event: &mut mod_api::PlayerCommandsSentEvent<'_>,
            ) -> Option<mod_api::CommandTreeSelection> {
                assert_eq!(event.player.uuid(), self.player);
                let living = host
                    .living(event.player.entity())
                    .expect("joining player's lent body");
                assert_eq!(living.max_health(host), Some(30.0));
                assert_eq!(living.health(host), Some(25.0));
                assert_eq!(event.player.entity().position(host), Some(PREPARED));
                self.stages.lock().unwrap().push(Stage::Commands);
                None
            }
            fn client_information_updated(
                &self,
                host: &mut dyn ServerHost,
                event: &mut mod_api::ClientInformationUpdatedEvent<'_>,
            ) {
                assert_eq!(event.player, self.player);
                assert!(host.player(event.player).is_some());
                self.stages.lock().unwrap().push(Stage::ClientInformation);
            }
            fn player_logged_in(
                &self,
                host: &mut dyn ServerHost,
                event: &mut mod_api::PlayerLoggedInEvent<'_>,
            ) {
                assert_eq!(event.name, NAME);
                assert_eq!(event.player, self.player);
                assert!(host.player(event.player).is_some());
                self.stages.lock().unwrap().push(Stage::Announced);
            }
        }

        let player = offline_uuid(NAME);
        let stages = Arc::new(Mutex::new(Vec::new()));
        let mods = vec![ModEntry::new(
            "prepared-join-order",
            Box::new(Observe {
                player,
                stages: stages.clone(),
            }),
        )]
        .into();
        let root = crate::prepared_test_root::prepared_root(&[
            WORLDGEN_INPUTS,
            SERVER_INPUTS,
            &["movement-properties.nbt"],
        ]);
        mod_platform::testing::with_mods(mods, || {
            with_game_at(root, |game| {
                write_saved_player(game, player);
                game.world.loaded_entities.push((
                    (0, 0),
                    vec![
                        Tag::compound([
                            ("id", string("minecraft:marker")),
                            ("UUID", Tag::IntArray(vec![100, 2, 3, 4])),
                            (
                                "Pos",
                                Tag::List(SAVED.map(|v| Tag::Double(v.to_bits())).to_vec()),
                            ),
                        ])
                        .into(),
                    ],
                ));
                open_fixture_hub(game);
                let lan = &mut game.server.lan.as_mut().unwrap().hub.lan;
                let mut client = Connection::local(lan.connect_local().unwrap());
                client.reader.set_read_timeout(Some(WAIT)).unwrap();
                for packet in [
                    handshake::Serverbound::Intention(handshake::Intention {
                        protocol: PROTOCOL_VERSION,
                        host: "local".into(),
                        port: 0,
                        intent: handshake::LOGIN,
                    })
                    .encode(),
                    login::Serverbound::Hello(login::ServerHello {
                        name: NAME.into(),
                        uuid: player,
                    })
                    .encode(),
                ] {
                    client.send(&packet).unwrap();
                }
                let (id, body) = client.recv().unwrap();
                assert!(matches!(
                    login::Clientbound::decode(id, &body).unwrap(),
                    login::Clientbound::LoginFinished(_)
                ));
                client
                    .send(&login::Serverbound::LoginAcknowledged(Empty).encode())
                    .unwrap();

                loop {
                    let (id, body) = client.recv().unwrap();
                    match cfg::Clientbound::decode(id, &body).unwrap() {
                        cfg::Clientbound::SelectKnownPacks(packs) => client
                            .send(&cfg::Serverbound::SelectKnownPacks(packs).encode())
                            .unwrap(),
                        cfg::Clientbound::UpdateTags(_) => {
                            // Registry sync precedes the configuration request. Supply the
                            // already-ready spawn; this test does not drive chunk generation.
                            let deadline = Instant::now() + WAIT;
                            let request = loop {
                                if let Some(request) = lan.configuration_requests().next() {
                                    break request;
                                }
                                assert!(Instant::now() < deadline, "no configuration request");
                                std::thread::yield_now();
                            };
                            lan.configuration_tasks.insert(&request);
                            request
                                .session
                                .advance_spawn(SpawnTask::Ready(PREPARED_SPAWN));
                            lan.configuration_tasks
                                .register(request.connection, Vec::new());
                        }
                        cfg::Clientbound::FinishConfiguration(_) => break,
                        cfg::Clientbound::Disconnect(d) => panic!("configuration refused: {d:?}"),
                        _ => {}
                    }
                }
                client
                    .send(&cfg::Serverbound::FinishConfiguration(Empty).encode())
                    .unwrap();

                // The hub emits HostEvent::Joined with the configured lease and the
                // packet tick dispatches it via lan_process_events.
                let deadline = Instant::now() + WAIT;
                loop {
                    game.server.level_pass = Default::default();
                    game.tick_lan_packets();
                    if let Some(guest) = game
                        .server
                        .lan
                        .as_ref()
                        .unwrap()
                        .hub
                        .guests
                        .iter()
                        .find(|g| g.uuid == player && g.placed)
                    {
                        assert_eq!(guest.position, PREPARED);
                        assert!(guest.motion.is_some(), "body already installed on join");
                        break;
                    }
                    assert!(
                        Instant::now() < deadline,
                        "configured join did not place the player"
                    );
                    std::thread::yield_now();
                }
                assert_eq!(
                    *stages.lock().unwrap(),
                    [
                        Stage::BeforePlacement,
                        Stage::Entities,
                        Stage::Construct,
                        Stage::File,
                        Stage::DatapackSync,
                        Stage::Commands,
                        Stage::ClientInformation,
                        Stage::Announced,
                    ]
                );
            });
        });
    }

    #[test]
    #[ignore = "daily"]
    fn held_partial_import_confirms_the_merged_save_without_reseeding_the_body() {
        with_guests(|game| {
            join_held_player(game);
            let hub = &mut game.server.lan.as_mut().unwrap().hub;
            hub.transfers = Some(arrivals::Transfers::new(Default::default()));
            let mut expected = LegacyRandom::new(816);
            let identity = hub
                .update_guest(CONN, |guest| {
                    assert!(guest.held && !guest.placed);
                    let body = guest
                        .motion
                        .as_deref_mut()
                        .expect("held join, before any lend");
                    body.random.set_seed(816);
                    assert_eq!(body.random.next_long(), expected.next_long());
                    std::ptr::from_ref(body)
                })
                .unwrap();

            let handover = arrivals::handover_text(1);
            let uuid = sunrise_protocol::crypto::uuid_string(UUID);
            let bytes = sunrise_core::nbt::write_named(
                &sunrise_core::nbt::NamedTag {
                    name: "".into(),
                    tag: Tag::compound([("XpLevel", Tag::Int(28))]),
                },
                sunrise_core::nbt::Limits::saved(),
            )
            .unwrap();
            let request = json!({
                "record": {
                    "version": arrivals::RECORD_VERSION,
                    "data_version": sunrise_world::DATA_VERSION,
                    "uuid": uuid,
                    "handover": handover,
                    "player": sunrise_sim::lan::base64(&bytes),
                },
                "parts": ["experience"],
                "place": false,
            });
            let imported = game.player_request("player-import", &format!("{uuid} {request}"));
            assert_eq!(imported["ok"], true, "{imported}");
            assert_eq!(imported["placed"], false);
            let confirmed = game.player_request("player-confirm", &format!("{uuid} {handover}"));
            assert_eq!(confirmed["ok"], true, "{confirmed}");
            assert_eq!(confirmed["placed"], true);
            let saved =
                sunrise_sim::save::read_dat(&playerdata::path(&game.server.world_root, UUID))
                    .unwrap();
            let mut level = sunrise_sim::save::Level::default();
            level.read_player(&saved);
            assert_eq!(level.xp_level, 28);
            assert_eq!(level.position, Some(PREPARED));
            assert_eq!(level.health, 25.0);
            let Some(Tag::List(attributes)) = saved.get("attributes") else {
                panic!("saved attributes")
            };
            let scale = attributes
                .iter()
                .find(|a| text(a.get("id")).as_deref() == Some("minecraft:scale"))
                .unwrap();
            assert_eq!(float(scale.get("base")), Some(0.5));
            game.server
                .lan
                .as_mut()
                .unwrap()
                .hub
                .update_guest(CONN, |guest| {
                    assert!(guest.placed && !guest.held);
                    let body = guest.motion.as_deref_mut().unwrap();
                    assert_eq!(std::ptr::from_ref(body), identity);
                    assert_eq!(
                        body.position,
                        Vec3::new(PREPARED[0], PREPARED[1], PREPARED[2])
                    );
                    assert_eq!(body.random.next_long(), expected.next_long());
                })
                .unwrap();
        });
    }
}
