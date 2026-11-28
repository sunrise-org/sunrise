//! #1999: closing the options screens opened from the title grabs the pointer
//! only when no title is shown (`MouseHandler.grabMouse` runs on
//! `setScreen(null)`, and `OptionsScreen.onClose` from `TitleScreen` returns to
//! the title). The fixture has no window: the gameplay grab restores held
//! keys before the window check, except on macOS where vanilla skips that step.
use sunrise_gameplay_common::game::Screen;
use winit::keyboard::KeyCode;

/// The title's Options page, closed: back under the title, which is still
/// shown, with the pointer left free.
#[test]
#[ignore = "daily"]
fn closing_options_opened_from_the_title_leaves_the_pointer_free() {
    crate::game::native_entity_refs::tests::with_client_game(|game| {
        let saves = tempfile::tempdir().unwrap();
        game.client.title.0 = Some(game.new_title(saves.path().to_path_buf()));
        game.client.focused = true;
        game.open_options(true);
        game.client.physical_keys.insert(KeyCode::KeyW);
        game.leave_options();
        assert_eq!(game.client.screen, Screen::Playing);
        assert!(game.client.title.0.is_some());
        assert!(!game.movement_key_held(KeyCode::KeyW));
    });
}

/// The startup handoff entered a world behind its open options: closing them
/// returns to that world and grabs the pointer, as before.
#[test]
#[ignore = "daily"]
fn closing_title_options_into_a_world_entered_behind_them_grabs() {
    crate::game::native_entity_refs::tests::with_client_game(|game| {
        game.client.title.0 = None;
        game.client.focused = true;
        game.open_options(true);
        game.client.physical_keys.insert(KeyCode::KeyW);
        game.leave_options();
        assert_eq!(game.client.screen, Screen::Playing);
        assert_eq!(
            game.movement_key_held(KeyCode::KeyW),
            !cfg!(target_os = "macos")
        );
    });
}

/// The pause menu's options, closed: back to the pause menu, a screen over
/// the world that keeps the pointer free, as before.
#[test]
#[ignore = "daily"]
fn closing_pause_menu_options_leaves_the_pointer_free() {
    crate::game::native_entity_refs::tests::with_client_game(|game| {
        game.client.title.0 = None;
        game.client.focused = true;
        game.open_options(false);
        game.client.physical_keys.insert(KeyCode::KeyW);
        game.leave_options();
        assert_eq!(game.client.screen, Screen::Paused);
        assert!(!game.movement_key_held(KeyCode::KeyW));
    });
}
