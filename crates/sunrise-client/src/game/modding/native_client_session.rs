//! Read-only projections of the connected client's existing state.

use super::{Game, api_values::TryIntoApi, remote::Remote};
use core::ops::ControlFlow;
#[cfg(test)]
use mod_api::prelude::*;
use mod_api::{
    BlockBreakingState, BlockPos, ClientFrameStats, ClientServer, ClientTabList,
    ClientTabListEntry, GameMode, PlayerRef, Text, TextFormError,
};
use std::sync::Arc;
use sunrise_client::{
    scoreboard::Scoreboard,
    tab_list::{TabPlayer, name_for_display, sorted_players},
};

impl Game<'_> {
    pub(super) fn native_session(&self) -> Option<&Remote> {
        self.native_window().then_some(())?;
        self.native_lifecycle.epoch()?;
        self.remote.as_ref().filter(|remote| remote.logged_in)
    }

    /// The connected server, not a stale reconnect target. None outside a
    /// logged-in client world. External display names are not retained; memory
    /// connections have no address. Friend-invite secrets are redacted.
    pub(super) fn native_client_server(&self) -> Option<ClientServer<'_>> {
        let remote = self.native_session()?;
        let integrated = self.integrated.as_ref();
        Some(ClientServer {
            name: integrated.map(|server| server.world_name.as_str()),
            address: remote.display_address.as_deref(),
            integrated_local: integrated.is_some(),
            save_folder: integrated.map(|server| server.save_folder.as_str()),
        })
    }

    /// A snapshot of the applied language code, not a pending options selection.
    /// None outside a logged-in client world.
    pub(super) fn native_client_language(&self) -> Option<Arc<str>> {
        self.native_session()?;
        Some(sunrise_client::lang::code_snapshot())
    }

    /// `PlayerInfo.getLatency` of the local player in milliseconds, 0 before the server's first
    /// measurement; None outside a logged-in client world or before the server lists the player.
    pub(super) fn native_client_latency(&self) -> Option<u32> {
        self.native_session()?;
        let latency = self.own_latency()?;
        Some(u32::try_from(latency).unwrap_or(0))
    }

    /// The local player's item use (`isUsingItem`, `getUsedItemHand`, `getTicksUsingItem`,
    /// `getUseItemRemainingTicks`); None when no item is in use or outside a logged-in client
    /// world.
    pub(in crate::game) fn local_item_use(&self) -> Option<mod_api::ItemUse> {
        self.native_session()?;
        let (_, ticks) = self.using?;
        let ticks_used = i32::try_from(ticks).unwrap_or(i32::MAX);
        let hand = match self.used_hand()? {
            sunrise_client::first_person::Hand::Main => mod_api::Hand::Main,
            sunrise_client::first_person::Hand::Off => mod_api::Hand::Off,
        };
        Some(mod_api::ItemUse::new(
            hand,
            ticks_used,
            self.use_duration.saturating_sub(ticks_used),
        ))
    }

    /// `setYRot` / `setXRot` of the local player by a client mod: yaw as given, pitch `% 360`
    /// clamped to -90..90, the previous angles moved by as much (`Entity.turn`). Sends nothing:
    /// the tick's player sync (`remote_send_state`) carries the look.
    /// `Minecraft.rightClickDelay`, which `startUseItem` sets to 4 and the client tick counts
    /// down; None outside a client world.
    pub(in crate::game) fn right_click_delay(&self) -> Option<u32> {
        (self.native_session().is_some() && self.player.is_some()).then_some(self.use_delay)
    }
    /// Sets `Minecraft.rightClickDelay`; false, changing nothing, outside a client world.
    pub(in crate::game) fn set_right_click_delay(&mut self, ticks: u32) -> bool {
        if self.native_session().is_none() || self.player.is_none() {
            return false;
        }
        self.use_delay = ticks;
        true
    }
    pub(in crate::game) fn set_local_rotation(
        &mut self,
        yaw: f32,
        pitch: f32,
    ) -> Result<(), mod_api::LocalRotationError> {
        if self.native_session().is_none() || self.player.is_none() {
            return Err(mod_api::LocalRotationError::NoWorld);
        }
        if !(yaw.is_finite() && pitch.is_finite()) {
            return Err(mod_api::LocalRotationError::NotFinite);
        }
        let pitch = (pitch % 360.0).clamp(-90.0, 90.0);
        let (turned_yaw, turned_pitch) = (yaw - self.yaw, pitch - self.pitch);
        self.rope_rot_old.0 += turned_yaw;
        self.rope_rot_old.1 += turned_pitch;
        self.yaw = yaw;
        self.pitch = pitch;
        if let Some(model) = self.player_model.as_mut() {
            model.previous_head_pitch += turned_pitch;
            model.head_pitch = pitch;
        }
        Ok(())
    }

    /// Existing local frame counters; None outside a logged-in client world.
    /// This read collects no history and sends nothing anywhere.
    pub(super) fn native_client_frame_stats(&self) -> Option<ClientFrameStats> {
        self.native_session()?;
        Some(ClientFrameStats {
            fps: self.fps,
            rendered_frames: self.frames,
            frame_time_ns: self
                .debug_charts
                .frame_times
                .latest()
                .and_then(|time| u64::try_from(time).ok()),
        })
    }

    pub(super) fn native_local_status(&self, player: PlayerRef) -> Option<()> {
        self.native_session()?;
        (self.local_player()? == player).then_some(())
    }

    /// The local player's predicted breaking state, including Idle. None for
    /// stale, nonlocal or fake handles and outside a logged-in client world.
    pub(super) fn player_block_breaking(&self, player: PlayerRef) -> Option<BlockBreakingState> {
        self.native_local_status(player)?;
        Some(
            self.mining
                .as_ref()
                .map_or(BlockBreakingState::Idle, |mining| {
                    BlockBreakingState::Breaking {
                        position: BlockPos::new(mining.pos[0], mining.pos[1], mining.pos[2]),
                        progress: mining.progress,
                        stage: mining.stage(),
                    }
                }),
        )
    }

    /// The local player's existing cave ambience mood, normally 0 through 1.
    /// None for stale, nonlocal or fake handles and outside a client world.
    pub(super) fn player_ambient_mood(&self, player: PlayerRef) -> Option<f32> {
        self.native_local_status(player)?;
        Some(self.ambience.moodiness)
    }
}

impl ClientTabList for Game<'_> {
    fn visit_entries(
        &self,
        visit: &mut dyn FnMut(&dyn ClientTabListEntry) -> ControlFlow<()>,
    ) -> ControlFlow<()> {
        visit_tab_entries(
            self.remote.iter().flat_map(Remote::listed_tab_player_refs),
            &self.score.board,
            visit,
        )
    }

    fn copy_header(&self) -> Result<Option<Text>, TextFormError> {
        self.tab_list
            .header
            .as_ref()
            .map(TryIntoApi::try_into_api)
            .transpose()
    }

    fn copy_footer(&self) -> Result<Option<Text>, TextFormError> {
        self.tab_list
            .footer
            .as_ref()
            .map(TryIntoApi::try_into_api)
            .transpose()
    }
}

// Keep the overlay's ordering and cap, lending one stack-scoped row at a time.
fn visit_tab_entries<'a>(
    players: impl IntoIterator<Item = &'a TabPlayer>,
    board: &'a Scoreboard,
    visit: &mut dyn FnMut(&dyn ClientTabListEntry) -> ControlFlow<()>,
) -> ControlFlow<()> {
    sorted_players(players, board).try_for_each(|player| visit(&TabEntryView { player, board }))
}

struct TabEntryView<'a> {
    player: &'a TabPlayer,
    board: &'a Scoreboard,
}

impl ClientTabListEntry for TabEntryView<'_> {
    fn name(&self) -> &str {
        &self.player.name
    }

    fn uuid(&self) -> u128 {
        self.player.uuid
    }

    fn game_mode(&self) -> GameMode {
        // `GameType.byId` returns survival for an unknown id.
        GameMode::from_id(self.player.game_mode).unwrap_or(GameMode::Survival)
    }

    fn latency(&self) -> i32 {
        self.player.latency
    }

    fn copy_display(&self) -> Result<Text, TextFormError> {
        name_for_display(self.player, self.board).try_into_api()
    }
}

#[cfg(test)]
pub(super) mod tests {
    use super::super::native_entity_refs::tests::with_client_game;
    use super::*;
    use sunrise_client::scoreboard::tag_to_json;
    use sunrise_core::nbt::Tag;

    /// The Tab overlay's borrowed rows, order, cap and rich names.
    #[test]
    fn the_tab_list_reads_as_the_overlay_draws_it() {
        let mut board = Scoreboard::default();
        let team = board.add_team("red");
        team.prefix = serde_json::json!("[R] ");
        team.suffix = serde_json::json!(" *");
        board.add_to_team("Alex", "red");
        let mut named = TabPlayer::new(1, "Steve");
        named.display_name = Some(serde_json::json!({
            "text": "Captain",
            "test:opaque": [null], // Unknown component fields aren't part of the display name.
        }));
        named.latency = 42;
        let teamed = TabPlayer::new(2, "Alex");
        let mut watcher = TabPlayer::new(3, "Bob");
        watcher.game_mode = 3;
        let mut first = TabPlayer::new(4, "Zed");
        first.list_order = 5;
        let mut wrapping = TabPlayer::new(5, "Wrap");
        wrapping.list_order = i32::MIN;
        let players = [watcher, named, teamed, first, wrapping];
        fn plain(text: &Text) -> String {
            let own = match &text.content {
                mod_api::text::TextContent::Literal(words) => words.as_str(),
                _ => "",
            };
            text.children
                .iter()
                .map(plain)
                .fold(own.to_owned(), |all, c| all + &c)
        }
        let mut names = Vec::new();
        assert_eq!(
            visit_tab_entries(&players, &board, &mut |row| {
                names.push(row.name().to_owned());
                match row.uuid() {
                    1 => {
                        assert_eq!(row.latency(), 42);
                        assert_eq!(row.copy_display(), Ok(Text::literal("Captain")));
                    }
                    2 => assert_eq!(plain(&row.copy_display().unwrap()), "[R] Alex *"),
                    3 => {
                        assert_eq!(row.game_mode(), GameMode::Spectator);
                        assert_eq!(row.copy_display().unwrap().style.italic, Some(true));
                    }
                    4 => assert_eq!(row.game_mode(), GameMode::Survival),
                    _ => {}
                }
                ControlFlow::Continue(())
            }),
            ControlFlow::Continue(())
        );
        assert_eq!(names, ["Wrap", "Zed", "Steve", "Alex", "Bob"]);

        let crowd: Vec<_> = (0..100)
            .rev()
            .map(|i| TabPlayer::new(i, format!("p{i:03}")))
            .collect();
        let mut ids = Vec::new();
        assert_eq!(
            visit_tab_entries(&crowd, &board, &mut |row| {
                ids.push(row.uuid());
                ControlFlow::Continue(())
            }),
            ControlFlow::Continue(())
        );
        assert!(
            ids.into_iter().eq(0_u128..80),
            "only the first 80 in drawn order"
        );

        let ties: Vec<_> = (0..81)
            .rev()
            .map(|i| TabPlayer::new(i, if i % 2 == 0 { "Alex" } else { "alex" }))
            .collect();
        let no_teams = Scoreboard::default();
        let mut ids = Vec::new();
        assert_eq!(
            visit_tab_entries(&ties, &no_teams, &mut |row| {
                ids.push(row.uuid());
                ControlFlow::Continue(())
            }),
            ControlFlow::Continue(())
        );
        assert!(
            ids.into_iter().eq((1_u128..81).rev()),
            "even the cutoff preserves ties"
        );

        let mut uncopyable = TabPlayer::new(42, "A");
        // The hover item retains its custom data, but the packet's NBT-to-JSON
        // projection turns the non-finite number into a nested null.
        uncopyable.display_name = Some(tag_to_json(&Tag::compound([
            ("text", Tag::String("Hover for details".into())),
            (
                "hover_event",
                Tag::compound([
                    ("action", Tag::String("show_item".into())),
                    ("id", Tag::String("minecraft:stone".into())),
                    (
                        "components",
                        Tag::compound([(
                            "minecraft:custom_data",
                            Tag::compound([("example", Tag::Double(f64::NAN.to_bits()))]),
                        )]),
                    ),
                ]),
            ),
        ])));
        let mut seen = Vec::new();
        assert_eq!(
            visit_tab_entries([&uncopyable, &players[2]], &board, &mut |row| {
                seen.push(row.uuid());
                if row.uuid() == 42 {
                    assert_eq!(row.name(), "A");
                    assert_eq!(row.copy_display(), Err(TextFormError::NestedNull));
                    return ControlFlow::Continue(());
                }
                assert_eq!(plain(&row.copy_display().unwrap()), "[R] Alex *");
                ControlFlow::Break(())
            }),
            ControlFlow::Break(())
        );
        assert_eq!(seen, [42, 2], "a text refusal does not stop the visit");
        assert_eq!(
            visit_tab_entries(&players, &board, &mut |row| {
                assert_eq!(row.name(), "Wrap", "no calls after Break");
                ControlFlow::Break(())
            }),
            ControlFlow::Break(())
        );
    }

    /// `LocalPlayer.permissionLevel` (#5529): on a client the local player's level is the one the
    /// server's entity events 24 to 28 last set, and another player's is unknown.
    #[test]
    #[ignore = "daily"]
    fn the_local_players_permission_level_follows_the_servers_events() {
        use mod_api::{EntityRef, WorldEpoch};
        use sunrise_protocol::packets::play::{self, Clientbound as C};
        super::super::native_entity_refs::tests::with_client_game(|game| {
            let _server = logged_client(game);
            let player = game.local_player().unwrap();
            assert_eq!(player.permission_level(game), Some(0));
            for (event, level) in [(26, 2), (28, 4), (24, 0)] {
                game.handle_packet(C::EntityEvent(play::EntityEvent { id: 7, event }))
                    .unwrap();
                assert_eq!(player.permission_level(game), Some(level), "event {event}");
            }
            // Another entity's event leaves the local player's level alone.
            game.handle_packet(C::EntityEvent(play::EntityEvent { id: 8, event: 28 }))
                .unwrap();
            assert_eq!(player.permission_level(game), Some(0));
            let epoch: WorldEpoch = player.entity().epoch();
            let other = mod_api::__private::game::player_ref(EntityRef::new(epoch, 99), 5);
            assert_eq!(other.permission_level(game), None);
        });
    }

    pub(in crate::game) fn logged_client(
        game: &mut Game<'_>,
    ) -> sunrise_protocol::connection::Pipe {
        let (client, server) = sunrise_protocol::connection::Pipe::pair();
        game.connect_local(client, "SessionReadTest").unwrap();
        let remote = game.remote.as_mut().unwrap();
        remote.logged_in = true;
        remote.entity_id = 7;
        remote.profile = 42;
        server
    }

    /// Drive the server's side of a memory client login through to play, so a test can
    /// read packets from the ordinary client sender.
    pub(in crate::game) fn finish_login(
        game: &mut Game<'_>,
        server: &mut sunrise_protocol::connection::Connection,
    ) {
        use sunrise_protocol::packets::{Empty, configuration as cfg, login};
        server.recv().unwrap(); // Intention.
        server.recv().unwrap(); // Hello.
        server
            .send(
                &login::Clientbound::LoginFinished(login::LoginFinished {
                    profile: login::GameProfile {
                        uuid: 42,
                        name: "SessionReadTest".into(),
                        properties: vec![],
                    },
                    session: 1,
                })
                .encode(),
            )
            .unwrap();
        server.recv().unwrap(); // Login acknowledged.
        server.recv().unwrap(); // Brand.
        server.recv().unwrap(); // Client information.
        server
            .send(&cfg::Clientbound::FinishConfiguration(Empty).encode())
            .unwrap();
        game.acknowledge_test_configuration();
        server.recv().unwrap(); // Finish configuration.
    }

    #[test]
    #[ignore = "daily"]
    fn client_session_reads_follow_connection_language_and_frame_state() {
        use core::ops::ControlFlow;
        use sunrise_client::lang::{self, Language};
        use sunrise_core::nbt::Tag;
        use sunrise_protocol::packets::play::{
            Clientbound, PlayerInfo, PlayerInfoUpdate, SetObjective, TabList,
        };

        with_client_game(|game| {
            assert!(matches!(game.client_session().client_tab_list(), Ok(None)));
            assert_eq!(game.client_session().client_server(), None);
            assert_eq!(game.client_session().client_language(), None);
            assert_eq!(game.client_session().client_frame_stats(), None);
            assert!(matches!(
                ScoreboardClientHost::client_scoreboard(game),
                Ok(None)
            ));
            let _server = logged_client(game);
            let empty_list = game.client_session().client_tab_list().unwrap().unwrap();
            assert_eq!(
                empty_list.visit_entries(&mut |_| panic!("new session's tab list is empty")),
                ControlFlow::Continue(())
            );
            assert_eq!(empty_list.copy_header(), Ok(None));
            assert_eq!(empty_list.copy_footer(), Ok(None));
            game.apply_packet(Clientbound::PlayerInfoUpdate(PlayerInfoUpdate {
                actions: 1 | 8,
                entries: vec![
                    PlayerInfo {
                        uuid: 1,
                        name: Some("Hidden".to_owned()),
                        listed: Some(false),
                        ..PlayerInfo::default()
                    },
                    PlayerInfo {
                        uuid: 2,
                        name: Some("Listed".to_owned()),
                        listed: Some(true),
                        ..PlayerInfo::default()
                    },
                ],
            }))
            .unwrap();
            let list = game.client_session().client_tab_list().unwrap().unwrap();
            let mut listed = Vec::new();
            assert_eq!(
                list.visit_entries(&mut |entry| {
                    listed.push(entry.uuid());
                    ControlFlow::Continue(())
                }),
                ControlFlow::Continue(())
            );
            assert_eq!(
                listed,
                [2],
                "only players the server marked listed are exposed"
            );
            game.apply_packet(Clientbound::TabList(TabList {
                header: Tag::String("Header".into()),
                footer: Tag::String("Footer".into()),
            }))
            .unwrap();
            let list = game.client_session().client_tab_list().unwrap().unwrap();
            assert_eq!(list.copy_header(), Ok(Some(Text::literal("Header"))));
            assert_eq!(list.copy_footer(), Ok(Some(Text::literal("Footer"))));
            game.apply_packet(Clientbound::TabList(TabList {
                header: Tag::String("".into()),
                footer: Tag::String("".into()),
            }))
            .unwrap();
            let list = game.client_session().client_tab_list().unwrap().unwrap();
            assert_eq!(list.copy_header(), Ok(None));
            assert_eq!(list.copy_footer(), Ok(None));

            let empty_board = ScoreboardClientHost::client_scoreboard(game)
                .unwrap()
                .unwrap();
            assert_eq!(
                empty_board.visit_objectives(&mut |_| panic!("new session's board is empty")),
                ControlFlow::Continue(())
            );
            game.remote_objective(SetObjective {
                name: "received".into(),
                method: 0,
                display: Some((Tag::String("Received".into()), 0, None)),
            });
            let live_board = ScoreboardClientHost::client_scoreboard(game)
                .unwrap()
                .unwrap();
            assert_eq!(
                live_board.visit_objectives(&mut |objective| {
                    assert_eq!(objective.name(), "received");
                    ControlFlow::Break(())
                }),
                ControlFlow::Break(())
            );
            let memory = game.client_session().client_server().unwrap();
            assert_eq!(memory.name, None);
            assert_eq!(memory.address, None);
            assert!(
                !memory.integrated_local,
                "a memory guest does not own the server"
            );

            // Exercise the same display-address calculation as Remote construction without
            // opening an extra network connection for either address.
            for (address, display) in [
                ("127.0.0.1:25565", "127.0.0.1:25565"),
                ("friend:private-invite", "Friend's world"),
            ] {
                let remote = game.remote.as_mut().unwrap();
                remote.session.address = address.into();
                remote.display_address = Remote::display_address_for(&remote.session.address);
                let external = game.client_session().client_server().unwrap();
                assert_eq!(external.address, Some(display));
                assert_eq!(
                    external.name, None,
                    "Session.name is the player, not the server"
                );
                assert!(
                    !external.integrated_local,
                    "an address does not determine ownership"
                );
            }

            game.settings.language = "fr_fr".into();
            let _publication = lang::isolate_publication(Language::of("es_es", []));
            let outer = game.client_session().client_language().unwrap();
            assert_eq!(outer.as_ref(), "es_es");
            let inner = {
                let _nested = lang::isolate_publication(Language::of("en_us", []));
                lang::set(Language::of("de_de", []));
                let inner = game.client_session().client_language().unwrap();
                assert_eq!(inner.as_ref(), "de_de");
                lang::scoped(Language::of("ja_jp", []), || {
                    assert_eq!(
                        game.client_session().client_language().as_deref(),
                        Some("ja_jp")
                    );
                });
                assert_eq!(lang::code(), "de_de");
                inner
            };
            assert_eq!(
                inner.as_ref(),
                "de_de",
                "the snapshot outlives its publication"
            );
            assert_eq!(
                game.client_session().client_language().as_deref(),
                Some("es_es")
            );
            assert_eq!(
                outer.as_ref(),
                "es_es",
                "publication does not mutate earlier snapshots"
            );
            game.debug_charts.frame_times.reset();
            assert_eq!(
                game.client_session()
                    .client_frame_stats()
                    .unwrap()
                    .frame_time_ns,
                None
            );
            game.fps = 63.5;
            game.frames = 99;
            game.debug_charts.frame_times.log(15_432_123);
            assert_eq!(
                game.client_session().client_frame_stats(),
                Some(ClientFrameStats {
                    fps: 63.5,
                    rendered_frames: 99,
                    frame_time_ns: Some(15_432_123),
                })
            );
            game.fps = 59.0;
            game.frames = 100;
            game.debug_charts.frame_times.log(17_123_456);
            assert_eq!(
                game.client_session().client_frame_stats(),
                Some(ClientFrameStats {
                    fps: 59.0,
                    rendered_frames: 100,
                    frame_time_ns: Some(17_123_456),
                })
            );
            game.pause = true;
            assert!(
                game.is_paused(),
                "the existing applied pause read remains canonical"
            );
            game.remote.as_mut().unwrap().logged_in = false;
            assert!(matches!(game.client_session().client_tab_list(), Ok(None)));
            assert_eq!(game.client_session().client_server(), None);
            assert_eq!(game.client_session().client_language(), None);
            assert_eq!(game.client_session().client_frame_stats(), None);
            assert!(matches!(
                ScoreboardClientHost::client_scoreboard(game),
                Ok(None)
            ));
            game.remote = None;
            // A reconnect target must not be mistaken for the current session.
            game.connection_starts("stale.example:25565", "OldPlayer", false);
            assert_eq!(game.client_session().client_server(), None);
            assert!(matches!(
                ScoreboardClientHost::client_scoreboard(game),
                Ok(None)
            ));
        });
    }

    #[test]
    #[ignore = "daily"]
    fn latency_and_item_use_read_the_local_player_as_the_client_sees_it() {
        with_client_game(|game| {
            assert_eq!(game.client_session().client_latency(), None);
            assert_eq!(game.local_item_use(), None);
            let _server = logged_client(game);
            assert_eq!(
                game.client_session().client_latency(),
                None,
                "not listed yet"
            );
            game.remote.as_mut().unwrap().list_self(0);
            assert_eq!(game.client_session().client_latency(), Some(0));
            game.remote.as_mut().unwrap().list_self(87);
            assert_eq!(game.client_session().client_latency(), Some(87));

            // A bow (use duration 72000) drawn in the main hand for 7 ticks.
            game.using = Some((game.selected, 7));
            game.use_duration = 72_000;
            assert_eq!(
                game.local_item_use(),
                Some(mod_api::ItemUse::new(mod_api::Hand::Main, 7, 71_993))
            );
            game.using = Some((super::super::shield::OFFHAND_SLOT, 3));
            assert_eq!(
                game.local_item_use().map(|u| u.hand),
                Some(mod_api::Hand::Off)
            );
            game.using = None;
            assert_eq!(game.local_item_use(), None, "released");

            game.remote.as_mut().unwrap().logged_in = false;
            assert_eq!(game.client_session().client_latency(), None);
            game.using = Some((game.selected, 7));
            assert_eq!(game.local_item_use(), None, "no world");
        });
    }

    #[test]
    #[ignore = "daily"]
    fn local_player_status_tracks_mining_mood_and_refuses_other_handles() {
        with_client_game(|game| {
            let _server = logged_client(game);
            let player = game.local_player().unwrap();
            assert_eq!(player.block_breaking(game), Some(BlockBreakingState::Idle));
            game.ambience.moodiness = 0.375;
            assert_eq!(player.ambient_mood(game), Some(0.375));
            for (progress, stage) in [(0.0, None), (0.45, Some(4)), (1.0, Some(9))] {
                game.mining = Some(super::super::Mining {
                    pos: [4, 65, -2],
                    progress,
                    per_tick: 0.0,
                    ticks: 3,
                    crack: None,
                });
                assert_eq!(
                    player.block_breaking(game),
                    Some(BlockBreakingState::Breaking {
                        position: BlockPos::new(4, 65, -2),
                        progress,
                        stage,
                    })
                );
            }
            game.mining = None;
            game.ambience.moodiness = 0.0;
            assert_eq!(player.block_breaking(game), Some(BlockBreakingState::Idle));
            assert_eq!(player.ambient_mood(game), Some(0.0));
            let fields = mod_api::__private::game::player_ref;
            let epoch = player.entity().epoch();
            for other in [
                fields(mod_api::EntityRef::new(epoch, 7), 43),
                fields(mod_api::EntityRef::new(epoch, 8), 42),
                fields(mod_api::EntityRef::new(epoch, 99), 999),
            ] {
                assert_eq!(other.block_breaking(game), None);
                assert_eq!(other.ambient_mood(game), None);
            }
            game.native_lifecycle.begin_open(None);
            assert_eq!(player.block_breaking(game), None, "the old epoch is stale");
            assert_eq!(player.ambient_mood(game), None);
            let current = game.local_player().unwrap();
            assert_eq!(current.ambient_mood(game), Some(0.0));
            game.options.execution_role =
                super::super::execution_role::ExecutionRole::IntegratedServer;
            assert_eq!(current.block_breaking(game), None);
            assert_eq!(current.ambient_mood(game), None);
            assert_eq!(game.client_session().client_server(), None);
            game.options.execution_role = super::super::execution_role::ExecutionRole::Client;
            game.remote.as_mut().unwrap().logged_in = false;
            assert_eq!(current.block_breaking(game), None);
            assert_eq!(current.ambient_mood(game), None);
            game.remote = None;
            assert_eq!(current.block_breaking(game), None);
        });
    }
}
