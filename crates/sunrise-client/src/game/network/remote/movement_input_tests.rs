//! Exercise the real local-player movement caller with and without a native listener.
use super::*;
use crate::game::native_entity_refs::tests::with_client_game;
use mod_api::{ClientHost, Hooks, MovementInputEvent};
use sunrise_client::motion::PlayerMotion;
use winit::keyboard::KeyCode;

struct Native;
#[mod_api::__subscriptions]
impl Hooks for Native {
    fn movement_input(&self, _: &dyn ClientHost, event: &mut MovementInputEvent) {
        if event.right_key {
            event.forward_key = true;
            event.sprint = false;
        } else if event.left_key {
            if !(event.forward_key && event.backward_key) {
                event.forward = -0.5;
            }
            event.strafe = 0.25;
            event.jump = !event.jump;
            event.sneak = !event.sneak;
            event.sprint = true;
        } else if event.backward_key {
            event.forward_key = true;
            event.left_key = true;
            event.right_key = true;
            // Direction-key edits must win over these conflicting axis edits.
            event.forward = 0.25;
            event.strafe = 0.5;
            event.sprint = true;
        } else {
            event.forward_key = true;
            event.sprint = true;
        }
    }
}

fn reset_player(game: &mut Game<'_>, food: i32) {
    let mut player = PlayerMotion::new(game.motion, Vec3::new(8.0, 64.0, 8.0)).unwrap();
    player.client_side = true;
    player.health.food.food = food;
    player.ground.on_ground = true;
    game.player = Some(player);
    game.keys.clear();
    game.using = None;
    let remote = game.remote.as_mut().unwrap();
    remote.health = (20.0, food, 5.0);
    remote.was_sprinting = false;
    remote.last_input = 0;
}

fn connect(game: &mut Game<'_>) -> sunrise_protocol::connection::Pipe {
    let (client, server) = sunrise_protocol::connection::Pipe::pair();
    game.connect_local(client, "movement-client").unwrap();
    let remote = game.remote.as_mut().unwrap();
    remote.logged_in = true;
    remote.loaded_sent = true;
    remote.entity_id = 42;
    remote.level_load.finish();
    game.screen = Screen::Playing;
    game.streaming
        .insert_remote(&mut game.world, sunrise_client::chunk::Chunk::empty(0, 0));
    server
}

fn tick(game: &mut Game<'_>) {
    if let Some(input) = game.tick_local_player(Vec::new(), 7).unwrap() {
        game.remote_send_state(input);
    }
}

fn matrix(mods: mod_api::Mods) {
    mod_platform::testing::with_mods(mods, || {
        with_client_game(|game| {
            let server = connect(game);
            // A direction edit discards a controller's old angle and magnitude.
            let keys = game.mod_movement_keys(Keys {
                analog: Some([0.2, 0.8]),
                ..Keys::default()
            });
            assert!(keys.forward && keys.sprint);
            assert_eq!(keys.direction(), [0.0, 1.0]);
            assert!(keys.analog.is_none());

            // Without a key edit, an independent controller vector remains intact.
            let keys = game.mod_movement_keys(Keys {
                forward: true,
                analog: Some([0.2, 0.8]),
                ..Keys::default()
            });
            assert_eq!(keys.direction(), [0.2, 0.8]);

            // Raw opposites remain distinct; changed keys win over both axes.
            let keys = game.mod_movement_keys(Keys {
                backward: true,
                analog: Some([0.0, -0.7]),
                ..Keys::default()
            });
            assert!(keys.forward && keys.backward && keys.left && keys.right);
            assert_eq!(keys.direction(), [0.0, 0.0]);
            assert!(keys.analog.is_none());

            // Axis-only edits update just the corresponding packet directions.
            let keys = game.mod_movement_keys(Keys {
                left: true,
                ..Keys::default()
            });
            assert_eq!(keys.direction(), [0.25, -0.5]);
            assert!(!keys.forward && keys.backward && keys.left && !keys.right);
            assert!(keys.jump && keys.shift && keys.sprint);
            let keys = game.mod_movement_keys(Keys {
                forward: true,
                backward: true,
                left: true,
                ..Keys::default()
            });
            assert_eq!(keys.direction(), [0.25, 0.0]);
            assert!(
                keys.forward && keys.backward,
                "unchanged axis keeps both keys"
            );

            // Consumer-visible acceptance: no physical input still moves the
            // local player; the final keys and eligible sprint reach the server.
            for food in [0, 6, 7, 20] {
                reset_player(game, food);
                let mut replayed = game.player.as_ref().unwrap().clone();
                tick(game);
                let player = game.player.as_ref().unwrap();
                assert_eq!(player.move_vector, [0.0, 1.0]);
                assert!(
                    player.position.z > 8.0,
                    "injected forward must move, food={food}"
                );
                assert_eq!(player.sprinting, food > 6, "food={food}");
                assert_eq!(game.remote.as_ref().unwrap().last_input, 1 | 64);
                assert_eq!(game.remote.as_ref().unwrap().was_sprinting, food > 6);
                // What the client sent is enough to play its tick again: the input packet's keys
                // and the sprint state, through `MotionWorld::replayed_tick` (the tick a mod's
                // `native_simulate_player_move` plays), without asking the listener again, move
                // the player as the client's own tick did.
                let position = player.position;
                let sprinting = player.sprinting;
                let sent_keys =
                    crate::game::lan_vehicles::input_keys(game.remote.as_ref().unwrap().last_input);
                let side = game_side!(game);
                MotionWorld {
                    side: &side,
                    entities: &[],
                    player: &mut replayed,
                    scene: &mut game.world,
                    assets: &game.pack.assets,
                    collision_binding: game.collision,
                    resources: game.motion,
                    health: &game.health_resources,
                    loop_context: &game.loop_context,
                    damage_hooks: &game.damage_hooks,
                    tag_items: None,
                    block_registries: None,
                }
                .replayed_tick(sent_keys, sprinting)
                .unwrap();
                assert_eq!(replayed.move_vector, [0.0, 1.0]);
                assert_eq!(replayed.sprinting, sprinting);
                assert!((replayed.position.z - position.z).abs() < 1.0e-10);
            }

            reset_player(game, 20);
            game.keys.insert(KeyCode::KeyS);
            tick(game);
            assert_eq!(game.player.as_ref().unwrap().move_vector, [0.0, 0.0]);
            assert_eq!(game.player.as_ref().unwrap().position.z, 8.0);
            assert_eq!(game.remote.as_ref().unwrap().last_input, 1 | 2 | 4 | 8 | 64);
            reset_player(game, 20);
            game.keys.insert(KeyCode::KeyA);
            tick(game);
            assert_eq!(game.player.as_ref().unwrap().move_vector, [0.25, -0.5]);
            assert_eq!(
                game.remote.as_ref().unwrap().last_input,
                2 | 4 | 16 | 32 | 64
            );

            // A mod's sprint request still uses the actual player's vanilla
            // crouching and slow-item eligibility, not only the hunger check.
            reset_player(game, 20);
            game.player.as_mut().unwrap().shift_key_down = true;
            game.keys.insert(KeyCode::ShiftLeft);
            tick(game);
            assert!(!game.player.as_ref().unwrap().sprinting);
            reset_player(game, 20);
            game.inventory[0] = Some(sunrise_client::items::ItemStack::new("minecraft:apple", 1));
            game.using = Some((0, 0));
            tick(game);
            assert!(!game.player.as_ref().unwrap().sprinting);

            // Subscribed false clears an existing latch and cannot start from
            // a pending double tap. KeyD selects this fixture listener's veto.
            for already_sprinting in [false, true] {
                reset_player(game, 20);
                game.keys.insert(KeyCode::KeyD);
                let player = game.player.as_mut().unwrap();
                player.sprint_input.trigger_time = 5;
                player.set_sprinting(already_sprinting).unwrap();
                game.remote.as_mut().unwrap().was_sprinting = already_sprinting;
                tick(game);
                let player = game.player.as_ref().unwrap();
                assert!(!player.sprinting);
                assert_eq!(player.sprint_input.trigger_time, 0);
                assert!(!game.remote.as_ref().unwrap().was_sprinting);
                assert_eq!(game.remote.as_ref().unwrap().last_input, 1 | 8);
            }
            server.shutdown();
        });
    });
}

#[test]
#[ignore = "daily"]
fn native_movement_input_keys_axes_and_sprint() {
    matrix(vec![mod_api::ModEntry::new("movement", Box::new(Native))].into());
}

#[test]
#[ignore = "daily"]
fn movement_input_without_listeners_keeps_vanilla_latch_double_tap_and_controller() {
    mod_platform::testing::with_mods(Vec::new().into(), || {
        with_client_game(|game| {
            let server = connect(game);
            let keys = game.mod_movement_keys(Keys {
                forward: true,
                analog: Some([0.2, 0.8]),
                ..Keys::default()
            });
            assert_eq!(keys.direction(), [0.2, 0.8]);
            assert!(!keys.sprint_suppressed);
            reset_player(game, 20);
            game.keys.insert(KeyCode::KeyW);
            game.keys.insert(KeyCode::ControlLeft);
            tick(game);
            assert!(game.player.as_ref().unwrap().sprinting);
            game.keys.remove(&KeyCode::ControlLeft);
            tick(game);
            assert!(game.player.as_ref().unwrap().sprinting, "vanilla latch");
            game.keys.clear();
            tick(game);
            assert!(!game.player.as_ref().unwrap().sprinting);
            reset_player(game, 20);
            game.keys.insert(KeyCode::KeyW);
            tick(game);
            assert!(!game.player.as_ref().unwrap().sprinting);
            game.keys.clear();
            tick(game);
            game.keys.insert(KeyCode::KeyW);
            tick(game);
            assert!(
                game.player.as_ref().unwrap().sprinting,
                "vanilla double tap"
            );
            server.shutdown();
        });
    });
}

#[test]
#[ignore = "daily"]
fn movement_input_disabled_listener_keeps_vanilla_keys() {
    mod_platform::testing::with_mods(
        vec![mod_api::ModEntry::new("movement", Box::new(Native))].into(),
        || {
            assert!(mod_api::__private::set_mod_enabled("movement", false));
            with_client_game(|game| {
                let keys = game.mod_movement_keys(Keys {
                    forward: true,
                    analog: Some([0.2, 0.8]),
                    ..Keys::default()
                });
                assert_eq!(keys.direction(), [0.2, 0.8]);
                assert!(!keys.sprint_suppressed);
                assert!(!keys.sprint);
                let keys = game.mod_movement_keys(Keys::default());
                assert_eq!(keys.direction(), [0.0, 0.0]);
                assert!(!keys.forward && !keys.sprint_suppressed);
                assert!(mod_api::__private::set_mod_enabled("movement", true));
                let keys = game.mod_movement_keys(Keys::default());
                assert_eq!(keys.direction(), [0.0, 1.0]);
                assert!(keys.forward && keys.sprint && !keys.sprint_suppressed);
                let keys = game.mod_movement_keys(Keys {
                    right: true,
                    ..Keys::default()
                });
                assert!(keys.forward && keys.right && keys.sprint_suppressed);
                assert!(mod_api::__private::set_mod_enabled("movement", false));
                let keys = game.mod_movement_keys(Keys::default());
                assert_eq!(keys.direction(), [0.0, 0.0]);
                assert!(!keys.forward && !keys.sprint_suppressed);
            });
        },
    );
}

#[path = "local_rotation_tests.rs"]
mod local_rotation_tests;

/// LocalPlayer.isShiftKeyDown is the input's sneak while the player rides too, so the
/// sneak held to get off already counts on the first tick out of the vehicle.
#[test]
#[ignore = "daily"]
fn the_sneak_key_counts_while_riding() {
    mod_platform::testing::with_mods(Vec::new().into(), || {
        with_client_game(|game| {
            let server = connect(game);
            reset_player(game, 20);
            let boat =
                crate::game::boats::Boat::new(5, "oak".into(), Vec3::new(8.0, 64.0, 8.0), 0.0);
            game.boats.push(boat);
            game.riding = Some(5);
            game.remote.as_mut().unwrap().first_passenger = false;
            game.keys.insert(KeyCode::ShiftLeft);
            tick(game);
            assert_eq!(game.riding, Some(5));
            assert!(game.player.as_ref().unwrap().shift_key_down);
            server.shutdown();
        });
    });
}
