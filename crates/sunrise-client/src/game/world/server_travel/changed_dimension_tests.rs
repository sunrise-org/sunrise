//! `player_changed_dimension` is heard on every logical server: the integrated server that runs
//! single player and the LAN world opened from it, and a dedicated server (#3250).
//! These prepared tests also exercise menu retirement during travel.
use super::Dim;
use crate::game::{
    Game, execution_role::ExecutionRole, lan::LanSettings, lan_actions::GuestCommand,
    native_entity_refs::tests::with_game_role,
};
use mod_api::{Hooks, ModEntry, PlayerChangedDimensionEvent, ServerHost};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicI32, AtomicUsize},
};
use sunrise_client::{
    chunk::Chunk,
    lan::{Joined, NetConfig, PlayerSender, guest::Guest, hub::HubConfig},
};
use sunrise_core::geometry::Vec3;
use sunrise_protocol::packets::configuration::ClientInformation;

const CONN: u32 = 12;
const UUID: u128 = 120;

type Heard = Arc<Mutex<Vec<(String, String)>>>;

struct Recorder(Heard);

#[mod_api::__subscriptions]
impl Hooks for Recorder {
    fn player_changed_dimension(
        &self,
        _: &mut dyn ServerHost,
        event: &mut PlayerChangedDimensionEvent<'_>,
    ) {
        self.0
            .lock()
            .unwrap()
            .push((event.from.to_owned(), event.to.to_owned()));
    }
}

/// A server of `role` with one survival player at (3.5, 64, 5.5) in the Overworld, its options as
/// `integrated.rs`'s server Game and a dedicated server set them.
fn with_server(role: ExecutionRole, test: impl FnOnce(&mut Game<'_>)) {
    let root = std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
    with_game_role(root, role, |game| {
        game.options.dedicated = Some(Vec::new());
        let config = HubConfig {
            default_game_mode: 0,
            guest_commands: true,
            force_game_mode: false,
            hardcore: false,
            view_distance: 2,
            simulation_distance: 2,
            dimension: "minecraft:overworld".into(),
            allow_flight: false,
            seed: 0,
            max_players: 2,
            integrated: role == ExecutionRole::IntegratedServer,
            levels: game.level_shapes.clone(),
            level_seeds: game.levels.runtime.seeds(),
        };
        game.lan_host(
            LanSettings {
                port: 0,
                guest_commands: true,
                force_game_mode: false,
            },
            NetConfig {
                listen: false,
                announce: false,
                lod_distance: 0,
                ..NetConfig::lan(0, String::new(), 2)
            },
            config,
        )
        .unwrap();
        light(game, 0, 0);
        let (tx, _rx) = std::sync::mpsc::channel();
        let mut guest = Guest::new(
            Joined {
                conn: CONN,
                uuid: UUID,
                name: "Owner".into(),
                properties: Vec::new(),
                info: ClientInformation::new(2),
                brand: "test".into(),
                sender: PlayerSender::new(tx, Arc::new(AtomicUsize::new(0))),
                latency: Arc::new(AtomicI32::new(0)),
                address: "127.0.0.1:1".parse().unwrap(),
                expected: false,
            },
            CONN as i32,
            [3.5, 64.0, 5.5],
            2,
        );
        guest.dimension = "minecraft:overworld".into();
        guest.placed = true;
        guest.loaded = true;
        guest.health = 20.0;
        guest.motion = Some(Box::new(
            sunrise_client::motion::PlayerMotion::new(game.motion, Vec3::new(3.5, 64.0, 5.5))
                .unwrap(),
        ));
        game.lan.as_mut().unwrap().hub.guests.push(guest);
        test(game);
    });
}

/// Lights the 5x5 chunks around chunk (`cx`, `cz`) of the level in place, as its
/// streaming does once they have loaded.
fn light(game: &mut Game<'_>, cx: i32, cz: i32) {
    for x in cx - 2..=cx + 2 {
        for z in cz - 2..=cz + 2 {
            game.streaming
                .insert_remote(&mut game.world, Chunk::empty(x, z));
        }
    }
}

/// Runs the server's travel until the player's trip has ended in `dim`, loading
/// the destination around the trip's target.
fn finish_trip(game: &mut Game<'_>, dim: Dim) {
    let home = game.dim;
    game.enter_level(dim).unwrap();
    let at = game
        .server_trips
        .iter()
        .find(|trip| trip.conn == CONN)
        .expect("the player is on a trip")
        .target;
    light(game, (at.x.floor() as i32) >> 4, (at.z.floor() as i32) >> 4);
    for _ in 0..20 {
        if !game.on_server_trip(CONN) {
            break;
        }
        game.tick_server_travel();
    }
    assert!(
        !game.on_server_trip(CONN),
        "the trip into {dim:?} never ended"
    );
    game.enter_level(home).unwrap();
}

fn run_as_player(game: &mut Game<'_>, role: ExecutionRole, command: &str) {
    assert!(game.run_guest_command(GuestCommand {
        conn: CONN,
        command: command.into(),
        permission: 4,
        integrated: role == ExecutionRole::IntegratedServer,
        quiet: false,
    }));
    // The integrated server runs them at its tick's start (`integrated.rs`).
    game.run_level_teleports();
}

fn heard_on(role: ExecutionRole, test: impl FnOnce(&mut Game<'_>, &Heard)) {
    let heard = Heard::default();
    let mods: mod_api::Mods = vec![ModEntry::new(
        "dimension-recorder",
        Box::new(Recorder(heard.clone())),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_server(role, |game| test(game, &heard));
    });
}

const SERVERS: [ExecutionRole; 2] = [
    ExecutionRole::IntegratedServer,
    ExecutionRole::DedicatedServer,
];

fn overworld_to_nether() -> Vec<(String, String)> {
    vec![("minecraft:overworld".into(), "minecraft:the_nether".into())]
}

#[test]
#[ignore = "daily"]
fn a_cross_level_teleport_returns_both_cursors_before_arrival() {
    use sunrise_client::items::ItemStack;

    with_server(ExecutionRole::DedicatedServer, |game| {
        let inventory_cursor = ItemStack::new("minecraft:iron_ingot", 3);
        let open_cursor = ItemStack::new("minecraft:gold_ingot", 5);
        let hub = &mut game.lan.as_mut().unwrap().hub;
        hub.guests
            .update(0, |g| g.carried = Some(inventory_cursor.clone()));
        hub.open_native_menu(
            CONN,
            1,
            sunrise_protocol::text::literal("fixture"),
            Vec::new(),
        )
        .unwrap();
        hub.guests
            .update(0, |g| g.carried = Some(open_cursor.clone()));
        game.level_teleports.push(super::LevelTeleport {
            conn: CONN,
            dim: Dim::Nether,
            at: [8.5, 70.0, 8.5],
            rotation: None,
            vehicle: None,
        });
        game.run_level_teleports();

        let guest = &game.lan.as_ref().unwrap().hub.guests[0];
        assert_eq!(guest.dimension, Dim::Nether.id());
        assert!(guest.menu.is_none());
        assert!(guest.carried.is_none());
        assert_eq!(
            guest
                .inventory
                .iter()
                .flatten()
                .cloned()
                .collect::<Vec<_>>(),
            [inventory_cursor, open_cursor]
        );
    });
}

#[test]
#[ignore = "daily"]
fn a_cross_level_teleport_survives_cursor_overflow_removing_an_earlier_guest() {
    use mod_api::{EntityJoinLevelEvent, Flow, PlayerRef};
    use sunrise_client::items::ItemStack;

    struct RemoveFake(Arc<Mutex<Option<PlayerRef>>>);
    #[mod_api::__subscriptions]
    impl Hooks for RemoveFake {
        fn entity_join_level(
            &self,
            host: &mut dyn ServerHost,
            event: &mut EntityJoinLevelEvent<'_>,
        ) -> Flow {
            if event.kind == "minecraft:item"
                && let Some(fake) = self.0.lock().unwrap().take()
            {
                assert!(fake.remove_fake(host));
            }
            Flow::Continue
        }
    }

    let fake = Arc::new(Mutex::new(None));
    let mods = vec![ModEntry::new(
        "remove-fake-on-drop",
        Box::new(RemoveFake(fake.clone())),
    )];
    mod_platform::testing::with_mods(mods.into(), || {
        with_server(ExecutionRole::DedicatedServer, |game| {
            let traveller = game.lan.as_mut().unwrap().hub.guests.remove(0);
            let level = game.level(Dim::Overworld.id()).unwrap();
            *fake.lock().unwrap() = Some(game.fake_player(level, 121, "Automation").unwrap());
            let hub = &mut game.lan.as_mut().unwrap().hub;
            assert!(hub.guests[0].is_fake());
            hub.guests.push(traveller);
            assert_eq!(hub.guests[1].conn, CONN);
            let queued = ItemStack::new("minecraft:iron_ingot", 3);
            let carried = ItemStack::new("minecraft:gold_ingot", 5);
            hub.guests.update(1, |g| {
                g.inventory
                    .fill(Some(ItemStack::new("minecraft:stone", 64)));
            });
            let title = sunrise_protocol::text::literal("fixture");
            hub.open_native_menu(CONN, 1, title.clone(), Vec::new())
                .unwrap();
            hub.guests.update(1, |g| g.carried = Some(queued.clone()));
            hub.close_container(CONN); // A cursor waiting to return when travel starts.
            hub.open_native_menu(CONN, 1, title, Vec::new()).unwrap();
            hub.guests.update(1, |g| g.carried = Some(carried.clone()));

            run_as_player(
                game,
                ExecutionRole::DedicatedServer,
                "execute in minecraft:the_nether run tp @s 8.5 70 8.5",
            );

            let guests = &game.lan.as_ref().unwrap().hub.guests;
            assert_eq!(guests.len(), 1);
            assert_eq!(guests[0].conn, CONN);
            assert_eq!(guests[0].dimension, Dim::Nether.id());
            assert!(guests[0].menu.is_none());
            assert!(guests[0].carried.is_none());
            assert_eq!(game.dim, Dim::Overworld);
            assert_eq!(
                game.dropped
                    .iter()
                    .map(|item| item.stack.clone())
                    .collect::<Vec<_>>(),
                [queued, carried]
            );
        });
    });
}

#[test]
#[ignore = "daily"]
fn a_failed_cross_level_trip_closes_the_retired_client_window() {
    use mod_api::{BlockUpdateFlags, EntityJoinLevelEvent, Flow};
    use sunrise_client::{items::ItemStack, lan::Out};
    use sunrise_protocol::{buf::Reader, ids::play::clientbound};

    const CHEST: [i32; 3] = [4, 64, 5];
    struct OpenOnOverflow;
    #[mod_api::__subscriptions]
    impl Hooks for OpenOnOverflow {
        fn entity_join_level(
            &self,
            host: &mut dyn ServerHost,
            event: &mut EntityJoinLevelEvent<'_>,
        ) -> Flow {
            if event.kind == "minecraft:item" {
                let level = host.level(Dim::Overworld.id()).unwrap();
                let chest = host.block_entity(level, CHEST).unwrap();
                let player = host.player(UUID).unwrap();
                assert_eq!(player.open_menu(host, chest), Ok(true));
            }
            Flow::Continue
        }
    }

    let mods = vec![ModEntry::new("open-on-overflow", Box::new(OpenOnOverflow))];
    mod_platform::testing::with_mods(mods.into(), || {
        with_server(ExecutionRole::DedicatedServer, |game| {
            // Opening an unloaded destination refuses an unreadable saved biome layout.
            // Set a world path: the constructor-only fixture otherwise reads the process's cwd.
            let world = tempfile::tempdir().unwrap();
            game.world_root = world.path().to_path_buf();
            let record = world.path().join("data/sunrise/biome_regions.dat");
            std::fs::create_dir_all(record.parent().unwrap()).unwrap();
            std::fs::write(record, b"not a biome layout").unwrap();
            let level = game.level(Dim::Overworld.id()).unwrap();
            let chest = game
                .pack
                .assets
                .registry
                .default_state("minecraft:chest")
                .unwrap();
            assert_eq!(
                game.level_set_block(level, CHEST, chest, BlockUpdateFlags::NEIGHBORS),
                Ok(true)
            );
            let (tx, rx) = std::sync::mpsc::channel();
            game.lan.as_mut().unwrap().hub.update_guest(CONN, |g| {
                g.sender = PlayerSender::new(tx, Arc::default());
                g.inventory
                    .fill(Some(ItemStack::new("minecraft:stone", 64)));
            });
            for via_portal in [false, true] {
                game.lan.as_mut().unwrap().hub.update_guest(CONN, |g| {
                    assert!(g.menu.is_none());
                    g.carried = Some(ItemStack::new("minecraft:gold_ingot", 5));
                });
                if via_portal {
                    game.start_server_trip(CONN, Dim::Nether, None, false);
                } else {
                    run_as_player(
                        game,
                        ExecutionRole::DedicatedServer,
                        "execute in minecraft:the_nether run tp @s 8.5 70 8.5",
                    );
                }
                let mut windows = Vec::new();
                for packet in rx.try_iter() {
                    if let Out::Packet(packet) = packet {
                        let mut reader = Reader::new(&packet);
                        let id = reader.varint().unwrap();
                        assert_ne!(id, clientbound::RESPAWN);
                        if matches!(id, clientbound::OPEN_SCREEN | clientbound::CONTAINER_CLOSE) {
                            windows.push((id, reader.varint().unwrap()));
                        }
                    }
                }
                assert!(
                    matches!(windows.as_slice(),
                        [(clientbound::OPEN_SCREEN, open), (clientbound::CONTAINER_CLOSE, close)]
                            if open == close && *open != 0
                    ),
                    "via portal: {via_portal}, packets: {windows:?}"
                );
                let guest = game.lan.as_ref().unwrap().hub.guest(CONN).unwrap();
                assert_eq!(guest.dimension, Dim::Overworld.id());
                assert!(guest.menu.is_none());
            }
        });
    });
}

#[test]
#[ignore = "daily"]
fn a_nether_portal_trip_is_heard_once_on_every_server() {
    for role in SERVERS {
        heard_on(role, |game, heard| {
            let portal = game
                .pack
                .assets
                .registry
                .default_state("minecraft:nether_portal")
                .unwrap();
            game.world.set(3, 64, 5, portal).unwrap();
            game.world.set(3, 65, 5, portal).unwrap();
            // `playersNetherPortalDefaultDelay` is 80 ticks for a survival player.
            for _ in 0..100 {
                if game.on_server_trip(CONN) {
                    break;
                }
                game.tick_server_travel();
            }
            assert!(
                game.on_server_trip(CONN),
                "{role:?}: the portal never took the player"
            );
            assert!(
                heard.lock().unwrap().is_empty(),
                "{role:?}: heard before the trip ended"
            );
            finish_trip(game, Dim::Nether);
            assert_eq!(*heard.lock().unwrap(), overworld_to_nether(), "{role:?}");
        });
    }
}

#[test]
#[ignore = "daily"]
fn a_command_teleport_into_the_nether_is_heard_once_on_every_server() {
    for role in SERVERS {
        heard_on(role, |game, heard| {
            run_as_player(
                game,
                role,
                "execute in minecraft:the_nether run tp @s 0 64 0",
            );
            finish_trip(game, Dim::Nether);
            assert_eq!(*heard.lock().unwrap(), overworld_to_nether(), "{role:?}");
        });
    }
}

#[test]
#[ignore = "daily"]
fn an_end_gateway_jump_within_the_level_is_not_heard_on_any_server() {
    for role in SERVERS {
        heard_on(role, |game, heard| {
            game.build_registries();
            run_as_player(
                game,
                role,
                "setblock 3 64 5 minecraft:end_gateway{exit_portal:[I;40,64,40],ExactTeleport:0b}",
            );
            // Not exact: the player waits for the exit's chunks, then lands near it.
            game.start_gateway_trip(CONN, [3, 64, 5]);
            finish_trip(game, Dim::Overworld);
            let at = game.lan.as_ref().unwrap().hub.guest(CONN).unwrap().position;
            assert!(
                (at[0] - 40.5).hypot(at[2] - 40.5) < 16.0,
                "{role:?}: the jump landed at {at:?}"
            );
            assert!(heard.lock().unwrap().is_empty(), "{role:?}: {heard:?}");
        });
    }
}
