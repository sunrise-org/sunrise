//! A controller action does what its keyboard key does, on a real `Game` without a window.
//! What needs a renderer (inventory slots, screenshots) is covered by tools/pad_parity_e2e.py.

use super::super::{Audio, Game, Options, Progress, Screen, Settings, load_resources};
use sunrise_client::actions::Action;
use sunrise_client::command::args::GameMode;
use sunrise_client::gamepad::{Button, PadState};
use sunrise_client::held::Source;
use sunrise_client::motion::PlayerMotion;
use sunrise_core::geometry::Vec3;
use winit::event::MouseButton;
use winit::keyboard::KeyCode;

/// A game at play with no world: input reaches the mappings, as in `Screen::Playing`.
use crate::game::execution_role::ExecutionRole;

fn playing(test: impl FnOnce(&mut Game)) {
    let root = std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
    let settings = Settings::default();
    let loaded = load_resources(
        sunrise_assets::resource_source::ResourceRoot::directory(root.clone()),
        Vec::new(),
        None,
        &root,
        &settings,
        std::time::Instant::now(),
        false,
        ExecutionRole::IntegratedServer,
        &Progress::new(),
        &mut |_, _| unreachable!("no menu handoff"),
    )
    .unwrap();
    let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
    options.resources = root.clone();
    options.execution_role = crate::game::execution_role::ExecutionRole::IntegratedServer;
    let mut game = Game::new(
        &loaded.resources,
        loaded.pack,
        loaded.gui.unwrap(),
        options,
        settings,
        Audio::silent(&sunrise_assets::resource_source::ResourceSource::directory(
            root.to_path_buf(),
        )),
        None,
    )
    .unwrap();
    game.title = None;
    game.screen = Screen::Playing;
    test(&mut game);
}

/// No window loop runs in these gameplay input tests.
struct NoEventLoop;

impl crate::game::event_loop::EventLoopCtl for NoEventLoop {
    fn exit(&self) {
        unreachable!("a gameplay pad press does not exit the event loop");
    }

    fn set_control_flow(&self, _: winit::event_loop::ControlFlow) {
        unreachable!("a gameplay pad press does not change the event loop");
    }
}

/// Hold `button` through the real press dispatcher, including its action ownership.
fn pad_press(game: &mut Game, button: Button, action: &str) {
    assert!(
        game.settings
            .controller
            .actions_of(button)
            .contains(&action)
    );
    game.pad.virtual_input = true;
    let mut state = PadState::default();
    // A trigger is held by how far it's pulled, not by a button bit.
    match button {
        Button::LeftTrigger => state.triggers[0] = 1.0,
        Button::RightTrigger => state.triggers[1] = 1.0,
        _ => state.set(button, true),
    }
    game.pad.edges.update(&state, 0.4);
    game.pad_button(&NoEventLoop, button, true).unwrap();
}

/// Release the sole held test button through the same dispatcher as its press.
fn pad_release(game: &mut Game, button: Button) {
    game.pad.edges.update(&PadState::default(), 0.4);
    game.pad_button(&NoEventLoop, button, false).unwrap();
}

#[test]
#[ignore = "daily"]
fn a_keyboard_release_keeps_the_sneak_the_pad_went_on_to_hold() {
    playing(|game| {
        // Hold mode: toggling, the pad's press would flip it.
        game.settings.controller.toggle_sneak = false;
        let shift = KeyCode::ShiftLeft;
        game.mapped_key(shift, true, false, shift, Source::Keyboard)
            .unwrap();
        pad_press(game, Button::RightStick, "key.sneak");
        game.mapped_key(shift, false, false, shift, Source::Keyboard)
            .unwrap();
        assert!(game.keys.held(Action::Sneak), "R3 is still down");
    });
}

#[test]
#[ignore = "daily"]
fn pad_smooth_camera_binding_toggles_the_cinematic_camera() {
    playing(|game| {
        game.settings
            .controller
            .bind("key.smoothCamera", Some(Button::Select));
        assert!(!game.smooth_camera);
        game.pad_action("key.smoothCamera", true).unwrap();
        assert!(game.smooth_camera, "as its key does in play");
    });
}

/// A player at the origin: the wheel's gates ask for one.
fn with_player(game: &mut Game) {
    game.player = Some(PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
}

#[test]
#[ignore = "daily"]
fn bumpers_step_the_hotbar_where_the_wheel_does() {
    playing(|game| {
        game.selected = 8;
        game.pad_action("pad.hotbarNext", true).unwrap();
        assert_eq!(game.selected, 8, "no player, no hotbar");
        with_player(game);
        game.pad_action("pad.hotbarNext", true).unwrap();
        assert_eq!(game.selected, 0, "wraps past the last slot");
        game.pad_action("pad.hotbarPrevious", true).unwrap();
        assert_eq!(game.selected, 8, "and back");
        game.pad_action("pad.hotbarNext", false).unwrap();
        assert_eq!(game.selected, 8, "a release does nothing");
        game.screen = Screen::Paused;
        game.pad_action("pad.hotbarNext", true).unwrap();
        assert_eq!(game.selected, 8, "a screen keeps the wheel from the hotbar");
    });
}

#[test]
#[ignore = "daily"]
fn a_spectators_bumpers_set_the_flying_speed_as_the_wheel_does() {
    playing(|game| {
        with_player(game);
        game.mode = GameMode::Spectator;
        game.selected = 3;
        let speed = game.player.as_ref().unwrap().flying_speed;
        game.pad_action("pad.hotbarPrevious", true).unwrap();
        // One wheel notch up: `MouseHandler.onScroll` adds 0.005.
        let after = game.player.as_ref().unwrap().flying_speed;
        assert!((after - speed - 0.005).abs() < 1e-6, "{speed} -> {after}");
        assert_eq!(game.selected, 3, "a spectator's hotbar doesn't step");
    });
}

#[test]
#[ignore = "daily"]
fn readers_of_the_movement_keys_see_the_stick() {
    playing(|game| {
        game.focused = true;
        assert!(!game.mapping("key.forward").unwrap().unwrap().down);
        // Up and to the left, on a diagonal: two keys, as a keyboard would hold.
        game.pad.state.left = [-0.7, 0.7];
        assert!(game.mapping("key.forward").unwrap().unwrap().down);
        assert!(game.mapping("key.left").unwrap().unwrap().down);
        assert!(!game.mapping("key.back").unwrap().unwrap().down);
        assert!(!game.mapping("key.right").unwrap().unwrap().down);
        assert!(
            !game.mapping("key.jump").unwrap().unwrap().down,
            "the stick holds only the movement keys"
        );
        // Forward is inside the eight-direction sector, left outside it. A direct
        // write on forward must not reclassify what remains of the stick.
        with_player(game);
        game.pad.state.left = [-0.34, 0.93];
        let before = game.sample_movement_keys(game.pad_move());
        assert!(game.mapping_state("key.forward").unwrap().0);
        assert!(!game.mapping_state("key.left").unwrap().0);
        assert!(before.forward);
        let [left, forward] = before.direction();
        assert!(left > 0.3 && forward > 0.8);
        game.set_mapping_down("key.forward", false).unwrap();
        let after = game.sample_movement_keys(game.pad_move());
        assert!(!game.mapping_state("key.forward").unwrap().0);
        assert!(!game.mapping_state("key.left").unwrap().0);
        assert!(!after.forward);
        assert_eq!(after.left, before.left);
        assert_eq!(after.direction(), [left, 0.0]);

        game.pad.state.left = [-0.85, 0.0];
        let [left, _] = game.pad_move().unwrap();
        game.set_mapping_down("key.forward", true).unwrap();
        let after = game.sample_movement_keys(game.pad_move());
        assert!(game.mapping_state("key.forward").unwrap().0);
        assert!(after.forward);
        assert_eq!(after.direction(), [left, 1.0]);

        // A tilt below the digital threshold is new analog input and still
        // takes over. A changed left component does not take over forward.
        game.pad.state.left = [0.0, 0.0];
        let before = game.pad_move();
        game.set_mapping_down("key.forward", false).unwrap();
        game.pad.state.left = [-0.4, 0.0];
        game.resume_stick_mappings(before);
        assert_eq!(game.keys.override_for(&KeyCode::KeyW), Some(false));
        game.pad.state.left = [0.0, 0.0];
        let before = game.pad_move();
        game.pad.state.left = [0.0, game.settings.controller.move_deadzone + 0.01];
        game.resume_stick_mappings(before);
        let after = game.sample_movement_keys(game.pad_move());
        assert!(!after.forward);
        assert!(after.direction()[1] > 0.0);

        // A screen takes the stick away.
        game.screen = Screen::Paused;
        assert!(!game.mapping("key.forward").unwrap().unwrap().down);
    });
}

#[test]
#[ignore = "daily"]
fn mapping_state_reads_what_mapping_reads_and_follows_the_binding() {
    playing(|game| {
        game.focused = true;
        let (down, bound) = game.mapping_state("key.forward").unwrap();
        assert!(!down);
        game.pad.state.left = [-0.7, 0.7];
        let held = game.mapping_state("key.forward").unwrap();
        assert_eq!(held, (true, bound), "holding it leaves the binding's hash");
        assert_eq!(held.0, game.mapping("key.forward").unwrap().unwrap().down);
        game.settings.keys.set("key.forward", "key.keyboard.up");
        assert_ne!(game.mapping_state("key.forward").unwrap().1, bound);
        assert_eq!(game.mapping_state("key.not_a_mapping"), None);
    });
}

#[test]
#[ignore = "daily"]
fn pad_number_actions_select_the_hotbar_as_their_keys_do() {
    playing(|game| {
        with_player(game);
        game.settings
            .controller
            .bind("key.hotbar.3", Some(Button::DPadUp));
        // This tests the one-shot number action, not all actions sharing D-pad up
        // (the default chat binding would open a screen before the number action).
        game.pad_action("key.hotbar.3", true).unwrap();
        assert_eq!(
            game.hotbar_presses.map(|c| c.count()),
            [0, 0, 1, 0, 0, 0, 0, 0, 0]
        );
    });
}

#[test]
#[ignore = "daily"]
fn a_held_pad_toolbar_action_counts_as_the_toolbar_key() {
    playing(|game| {
        game.settings
            .controller
            .bind("key.saveToolbarActivator", Some(Button::DPadUp));
        assert!(!game.mapped_key_down("key.saveToolbarActivator"));
        pad_press(game, Button::DPadUp, "key.saveToolbarActivator");
        assert!(game.mapped_key_down("key.saveToolbarActivator"));
        assert!(!game.mapped_key_down("key.loadToolbarActivator"));
    });
}

#[test]
#[ignore = "daily"]
fn the_pads_attack_and_use_reach_what_the_mouse_tells_mods() {
    playing(|game| {
        assert_eq!(game.native_clicks(), (0, 0));
        pad_press(game, Button::RightTrigger, "key.attack");
        assert!(game.mapping("key.attack").unwrap().unwrap().down);
        assert_eq!(game.native_clicks(), (1, 0));
        pad_release(game, Button::RightTrigger);
        assert!(!game.mapping("key.attack").unwrap().unwrap().down);
        pad_press(game, Button::LeftTrigger, "key.use");
        assert!(game.mapping("key.use").unwrap().unwrap().down);
        assert_eq!(game.native_clicks(), (1, 1));
    });
}

#[test]
#[ignore = "daily"]
fn the_mouse_letting_go_leaves_what_the_pad_holds_and_back() {
    playing(|game| {
        game.mouse_mapping(MouseButton::Left, true, Source::Keyboard)
            .unwrap();
        pad_press(game, Button::RightTrigger, "key.attack");
        game.mouse_mapping(MouseButton::Left, false, Source::Keyboard)
            .unwrap();
        assert!(game.left, "the pad still holds the attack");
        // The pad let go, so its edges are no longer held either.
        pad_release(game, Button::RightTrigger);
        assert!(!game.left);

        // The other way round: the mouse button is physically down.
        game.physical_buttons.insert(MouseButton::Right);
        game.mouse_mapping(MouseButton::Right, true, Source::Keyboard)
            .unwrap();
        pad_press(game, Button::LeftTrigger, "key.use");
        pad_release(game, Button::LeftTrigger);
        assert!(game.right, "the mouse still holds the use");
        game.physical_buttons.remove(&MouseButton::Right);
        game.mouse_mapping(MouseButton::Right, false, Source::Keyboard)
            .unwrap();
        assert!(!game.right);

        // Rebinding a physical mouse button to an ordinary key goes through
        // the keyboard source path, not the default attack/use path above.
        with_player(game);
        let binding = "key.mouse.left";
        game.settings.keys.set("key.forward", binding);
        for mouse_releases_first in [true, false] {
            game.mapped_mouse_event(MouseButton::Left, binding, true)
                .unwrap();
            game.pad_action("key.forward", true).unwrap();
            game.set_mapping_down("key.forward", false).unwrap();
            assert!(!game.keys.held(Action::Forward));
            for mouse_releases in [mouse_releases_first, !mouse_releases_first] {
                if mouse_releases {
                    game.mapped_mouse_event(MouseButton::Left, binding, false)
                        .unwrap();
                } else {
                    game.pad_action("key.forward", false).unwrap();
                }
                assert_eq!(
                    game.keys.held(Action::Forward),
                    mouse_releases == mouse_releases_first
                );
            }
        }
    });
}

#[test]
#[ignore = "daily"]
fn a_release_from_a_screen_can_not_strand_the_pads_attack() {
    playing(|game| {
        // The mouse went down in play, and came up where the mappings didn't hear it.
        game.mouse_mapping(MouseButton::Left, true, Source::Keyboard)
            .unwrap();
        game.left = false;
        pad_press(game, Button::RightTrigger, "key.attack");
        assert!(game.left);
        pad_release(game, Button::RightTrigger);
        assert!(!game.left, "nothing is physically held");
    });
}
