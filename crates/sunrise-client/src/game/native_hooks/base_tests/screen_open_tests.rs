use super::*;
use mod_api::{
    ClientHost, Flow, Hooks, InteractionKeyMappingTriggeredEvent, KeyMappingPreEvent, ModEntry,
    ScreenClosingEvent, ScreenOpeningEvent,
};
use mod_platform::testing::with_mods;
use std::sync::{Arc, Mutex};

/// A mod that hears `screen_opening` and passes.
struct Passer;
#[mod_api::__subscriptions]
impl Hooks for Passer {
    fn screen_opening(
        &self,
        _host: &dyn ClientHost,
        _event: &mut ScreenOpeningEvent<'_>,
    ) -> Option<ScreenOpening> {
        None
    }
}
/// A mod with no screen hook.
struct Idle;
#[mod_api::__subscriptions]
impl Hooks for Idle {}

/// The creative screen keeps its two tab rows clear: one strip above the window
/// and one below it, each as wide as the window and a tab row deep (vanilla's
/// `CreativeModeInventoryScreen` tab buttons, 28 px outside the 195x136 window).
#[test]
#[ignore = "daily"]
fn the_creative_screen_excludes_its_two_tab_rows() {
    with_mods(Vec::new().into(), || {
        crate::game::native_entity_refs::tests::with_game(|game| {
            game.screen = Screen::Creative;
            let batch = HudBatch::scratch([854, 480]);
            let image = game.container_image(&batch);
            let (x, y) = (image.left as f32, image.top as f32);
            let areas = game.screen_own_exclusion(&batch);
            assert_eq!(areas[0], [x, y - 28.0, 195.0, 28.0]);
            assert_eq!(areas[1], [x, y + 136.0, 195.0, 28.0]);
        });
    });
}

struct KeyProbe(Arc<Mutex<Vec<String>>>);
#[mod_api::__subscriptions]
impl Hooks for KeyProbe {
    fn interaction_key_mapping_triggered(
        &self,
        _host: &mut dyn ClientHost,
        event: &mut InteractionKeyMappingTriggeredEvent,
    ) -> Flow {
        self.0
            .lock()
            .unwrap()
            .push(format!("{:?}:{:?}", event.key, event.attempt));
        match event.key {
            InteractionKey::Attack => {
                event.swing_hand = false;
                Flow::Cancel
            }
            InteractionKey::Use => Flow::Cancel,
            _ => Flow::Continue,
        }
    }
}

/// Each key reaches a listener as its own kind; a cancelled attack that clears
/// `swing_hand` reports no swing, a cancelled use keeps its swing, and an
/// uncancelled pick goes on.
#[test]
#[ignore = "daily"]
fn an_interaction_key_reaches_a_mod_as_its_kind_and_can_be_cancelled() {
    let log = Arc::new(Mutex::new(Vec::new()));
    let mods = vec![ModEntry::new("keys", Box::new(KeyProbe(log.clone())))].into();
    with_mods(mods, || {
        crate::game::native_entity_refs::tests::with_game(|game| {
            let hand = Hand::Main;
            assert_eq!(
                game.native_interaction(InteractionKey::Attack, hand, InteractionAttempt::Held)
                    .canceled_swing(),
                Some(false)
            );
            assert_eq!(
                game.native_interaction(InteractionKey::Use, hand, InteractionAttempt::Press)
                    .canceled_swing(),
                Some(true)
            );
            assert_eq!(
                game.native_interaction(InteractionKey::PickBlock, hand, InteractionAttempt::Press)
                    .canceled_swing(),
                None
            );
        });
    });
    assert_eq!(take(&log), ["Attack:Held", "Use:Press", "PickBlock:Press"]);
}

struct MappingProbe(Arc<Mutex<Vec<String>>>);
#[mod_api::__subscriptions]
impl Hooks for MappingProbe {
    fn key_mapping_pre(&self, _host: &mut dyn ClientHost, event: &mut KeyMappingPreEvent) -> Flow {
        self.0
            .lock()
            .unwrap()
            .push(format!("{}:{}", event.mapping, event.pressed));
        Flow::Cancel
    }
}

/// Cancels the second press it hears and no other: an allowed down, then a cancelled repeat.
struct CancelSecondPress(Arc<Mutex<usize>>);
#[mod_api::__subscriptions]
impl Hooks for CancelSecondPress {
    fn key_mapping_pre(&self, _host: &mut dyn ClientHost, event: &mut KeyMappingPreEvent) -> Flow {
        let mut heard = self.0.lock().unwrap();
        if event.pressed {
            *heard += 1;
            if *heard == 2 {
                return Flow::Cancel;
            }
        }
        Flow::Continue
    }
}

/// A cancelled repeat keeps the hold an accepted press made, while a cancelled first press
/// is never held, nor are its repeats.
#[test]
#[ignore = "daily"]
fn a_cancelled_repeat_keeps_the_hold_and_a_cancelled_first_press_is_never_held() {
    use winit::keyboard::KeyCode;
    let heard = Arc::new(Mutex::new(0));
    let mods = vec![ModEntry::new(
        "second",
        Box::new(CancelSecondPress(heard.clone())),
    )]
    .into();
    with_mods(mods, || {
        crate::game::native_entity_refs::tests::with_game(|game| {
            game.title = None;
            game.screen = Screen::Playing;
            let space = KeyCode::Space;
            // The first down is allowed and held; its repeat is cancelled but the hold stays.
            game.physical_keys.insert(space);
            assert!(!game.key_press_canceled(space, true, false));
            game.physical_keys.insert(space);
            assert!(game.key_press_canceled(space, true, true));
            assert!(game.physical_keys.contains(&space));
            assert!(!game.rejected_keys.contains(&space));
        });
    });
    let mods = vec![ModEntry::new(
        "mapping",
        Box::new(MappingProbe(Arc::new(Mutex::new(Vec::new())))),
    )]
    .into();
    with_mods(mods, || {
        crate::game::native_entity_refs::tests::with_game(|game| {
            game.title = None;
            game.screen = Screen::Playing;
            let space = KeyCode::Space;
            // A cancelled first press is not held, and neither is its repeat.
            game.physical_keys.insert(space);
            assert!(game.key_press_canceled(space, true, false));
            assert!(!game.physical_keys.contains(&space));
            game.physical_keys.insert(space);
            assert!(game.key_press_canceled(space, true, true));
            assert!(!game.physical_keys.contains(&space));
            // Its release clears the rejection.
            assert!(!game.key_press_canceled(space, false, false));
            assert!(!game.rejected_keys.contains(&space));
        });
    });
}

/// A cancelled press of a mapping is dropped while playing, and only then asked; the
/// release of the same key is asked too but never canceled, so nothing stays held.
#[test]
#[ignore = "daily"]
fn a_cancelled_mapping_press_is_dropped_and_its_release_is_not() {
    let log = Arc::new(Mutex::new(Vec::new()));
    let mods = vec![ModEntry::new(
        "mapping",
        Box::new(MappingProbe(log.clone())),
    )]
    .into();
    with_mods(mods, || {
        crate::game::native_entity_refs::tests::with_game(|game| {
            game.title = None;
            game.screen = Screen::Playing;
            assert!(game.key_mapping_canceled(Some("key.keyboard.space"), true));
            assert!(!game.key_mapping_canceled(Some("key.keyboard.space"), false));
            game.screen = Screen::Inventory;
            assert!(!game.key_mapping_canceled(Some("key.keyboard.space"), true));
        });
    });
    assert_eq!(take(&log), ["key.jump:true", "key.jump:false"]);
}

struct Probe {
    log: Arc<Mutex<Vec<String>>>,
    answer: Arc<Mutex<Option<ScreenOpening>>>,
}
#[mod_api::__subscriptions]
impl Hooks for Probe {
    fn screen_opening(
        &self,
        _host: &dyn ClientHost,
        event: &mut ScreenOpeningEvent<'_>,
    ) -> Option<ScreenOpening> {
        self.log
            .lock()
            .unwrap()
            .push(format!("opening {}->{}", event.current.id, event.new.id));
        self.answer.lock().unwrap().clone()
    }
    fn screen_closing(&self, _host: &dyn ClientHost, event: &mut ScreenClosingEvent<'_>) {
        self.log
            .lock()
            .unwrap()
            .push(format!("closing {}", event.screen.id));
    }
}

/// Runs `test` on a client game with one probe mod; the probe's log and answer.
fn run(
    test: impl FnOnce(&mut Game<'_>, &Arc<Mutex<Vec<String>>>, &Arc<Mutex<Option<ScreenOpening>>>),
) {
    let log = Arc::new(Mutex::new(Vec::new()));
    let answer = Arc::new(Mutex::new(None));
    let mods = vec![
        ModEntry::new("passer", Box::new(Passer)),
        ModEntry::new(
            "screens",
            Box::new(Probe {
                log: log.clone(),
                answer: answer.clone(),
            }),
        ),
    ]
    .into();
    with_game_of(mods, |game| test(game, &log, &answer));
}
/// A client game with a player, under `mods`.
fn with_game_of(mods: Arc<[ModEntry]>, test: impl FnOnce(&mut Game<'_>)) {
    with_mods(mods, || {
        crate::game::native_entity_refs::tests::with_game(|game| {
            game.options.execution_role = ExecutionRole::Client;
            game.player = Some(
                sunrise_client::motion::PlayerMotion::new(
                    game.motion,
                    sunrise_core::geometry::Vec3::ZERO,
                )
                .unwrap(),
            );
            test(game);
        });
    });
}

/// Only a mod that listens to `screen_opening` or `screen_closing` makes a screen
/// change a hook call; a mod with neither leaves it a plain assignment, and a mod
/// screen's entry still releases the keys.
#[test]
#[ignore = "daily"]
fn a_mod_with_no_screen_hook_leaves_set_screen_plain() {
    with_game_of(vec![ModEntry::new("idle", Box::new(Idle))].into(), |game| {
        game.keys.insert(KeyCode::KeyW);
        game.set_screen(Screen::Mod);
        assert_eq!(game.screen, Screen::Mod);
        assert!(game.keys.is_empty());
    });
}
#[test]
#[ignore = "daily"]
fn mods_actions_route_pointer_and_keyboard_to_the_owner_and_close_to_the_selection() {
    use mod_api::{ModListActionResult, Registrar, RegistrationContext};
    use sunrise_client::{
        gamepad::Button,
        mod_list::{Installed, NativePage, NativeSetting, NativeSettingValue},
        options_ui::Page,
        widgets::{self, Kind},
    };
    use winit::{
        event::{DeviceId, ElementState, MouseButton, WindowEvent},
        event_loop::ControlFlow,
    };

    thread_local! {
        static PRESSES: std::cell::RefCell<Vec<(&'static str, String)>> = const { std::cell::RefCell::new(Vec::new()) };
    }
    struct ActionOwner;
    #[mod_api::__subscriptions]
    impl Hooks for ActionOwner {
        fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
            r.register_mod_list_action("details", "gui.done", |_, key| {
                PRESSES.with_borrow_mut(|presses| presses.push(("action_owner", key.into())));
                ModListActionResult::OpenScreen
            })
            .unwrap();
        }
    }
    struct OtherOwner;
    #[mod_api::__subscriptions]
    impl Hooks for OtherOwner {
        fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
            for key in [
                "details", "extra_01", "extra_02", "extra_03", "extra_04", "extra_05", "extra_06",
                "extra_07", "extra_08", "extra_09", "extra_10", "extra_11", "extra_12", "extra_13",
                "extra_14", "extra_15", "extra_16",
            ] {
                r.register_mod_list_action(key, "gui.done", |_, key| {
                    PRESSES.with_borrow_mut(|presses| presses.push(("other_owner", key.into())));
                    ModListActionResult::Stay
                })
                .unwrap();
            }
        }
    }
    struct NoLoop;
    impl crate::game::event_loop::EventLoopCtl for NoLoop {
        fn exit(&self) {}
        fn set_control_flow(&self, _: ControlFlow) {}
    }
    let log = Arc::new(Mutex::new(Vec::new()));
    let answer = Arc::new(Mutex::new(None));
    let mods = vec![
        ModEntry::new("action_owner", Box::new(ActionOwner)),
        ModEntry::new("other_owner", Box::new(OtherOwner)),
        ModEntry::new(
            "screens",
            Box::new(Probe {
                log: log.clone(),
                answer: answer.clone(),
            }),
        ),
    ]
    .into();
    with_game_of(mods, |game| {
        mod_platform::__private::part_tests::install_parts_fixture(mod_api::__private::mods())
            .unwrap();
        game.settings.mods.installed =
            [("action_owner", "A fixture"), ("other_owner", "B fixture")]
                .map(|(id, title)| Installed {
                    id: id.into(),
                    title: title.into(),
                    description: String::new(),
                    problem: None,
                    dev: false,
                    native: Some(NativePage {
                        settings: vec![NativeSetting {
                            key: "flag".into(),
                            label: "gui.done".into(),
                            tooltip: None,
                            value: NativeSettingValue::Toggle(false),
                        }],
                        ..Default::default()
                    }),
                })
                .into();
        let draw = |game: &mut Game<'_>| {
            let mut batch = HudBatch::new([1280, 720], game.settings.gui_scale_inputs());
            widgets::cover_frame();
            game.draw_options(&mut batch);
            widgets::end_frame(widgets::generation() + 1);
        };
        let target = |kind| {
            widgets::last_targets()
                .into_iter()
                .find(|t| t.kind == kind)
                .unwrap()
        };
        let action = |owner, key| {
            target(Kind::ModListAction(
                mod_api::__private::ModListActionId::new(owner, key),
            ))
        };
        let tab_to = |game: &mut Game<'_>, rect| {
            for _ in 0..=widgets::last_targets().len() {
                if game.pad_focus_rect() == Some(rect) {
                    return;
                }
                assert!(game.key_focus_key(&NoLoop, KeyCode::Tab));
            }
            panic!("Tab did not reach {rect:?}");
        };
        let click = |game: &mut Game<'_>, at| {
            game.cursor = at;
            let event = WindowEvent::MouseInput {
                device_id: DeviceId::dummy(),
                state: ElementState::Pressed,
                button: MouseButton::Left,
            };
            game.key_focus_mouse(&NoLoop, &event, None);
            game.key_focus_mouse_press(&event);
            game.options_mouse(true, false).unwrap();
            game.options_mouse(false, false).unwrap();
        };
        let expect_press = |owner, key: &str| {
            assert_eq!(
                PRESSES.with(std::cell::RefCell::take),
                [(owner, key.to_owned())]
            );
        };
        let show = |game: &mut Game<'_>, title| {
            game.open_options_at(title, Page::Mods);
            game.pad_clear_focus();
            draw(game);
            assert!(!game.key_focus_key(&NoLoop, KeyCode::ArrowDown));
            game.options_key(KeyCode::ArrowDown, None).unwrap();
            draw(game);
        };
        let assert_parent = |game: &Game<'_>| {
            assert_eq!(game.screen, Screen::Options);
            assert_eq!(game.ui.options.page(), Page::Mods);
            assert_eq!(
                game.ui
                    .options
                    .mods_screen
                    .as_ref()
                    .unwrap()
                    .selected_package(),
                Some("action_owner")
            );
            assert_eq!(game.mod_screen(), None);
        };
        show(game, false);
        take(&log);
        tab_to(game, target(Kind::Field).rect);
        assert!(game.ui.options.typing());
        tab_to(game, target(Kind::Row).rect);
        assert!(!game.ui.options.typing());
        let config_label =
            sunrise_client::text::Text::translate("sunrise.mods.screen.config", &[]).plain();
        let config = widgets::last_labelled()
            .into_iter()
            .find(|w| w.label == config_label && w.enabled)
            .unwrap();
        tab_to(game, [config.x, config.y, config.w, config.h]);
        let focus = action("action_owner", "details").rect;
        tab_to(game, focus);
        *answer.lock().unwrap() = Some(ScreenOpening::Cancel);
        assert!(game.key_focus_key(&NoLoop, KeyCode::Enter));
        assert_parent(game);
        assert_eq!(game.pad_focus_rect(), Some(focus));
        assert_eq!(take(&log), ["opening options->mod"]);
        expect_press("action_owner", "details");
        *answer.lock().unwrap() = None;
        assert!(game.key_focus_key(&NoLoop, KeyCode::Enter));
        assert_eq!(game.mod_screen(), Some("action_owner"));
        expect_press("action_owner", "details");
        let owner = mod_api::__private::enter("action_owner", "fixture");
        assert_eq!(game.mod_list_action_key(), Ok(Some("details")));
        drop(owner);
        let other = mod_api::__private::enter("screens", "fixture");
        assert_eq!(game.mod_list_action_key(), Ok(None));
        drop(other);
        game.key_pressed(KeyCode::Escape, KeyCode::Escape).unwrap();
        assert_parent(game);
        draw(game);
        tab_to(game, target(Kind::Row).rect);
        assert!(game.key_focus_key(&NoLoop, KeyCode::ArrowDown));
        assert_eq!(
            game.ui
                .options
                .mods_screen
                .as_ref()
                .unwrap()
                .selected_package(),
            Some("other_owner")
        );
        game.key_focus_key(&NoLoop, KeyCode::Enter);
        assert!(PRESSES.with_borrow(Vec::is_empty));
        draw(game);

        // Click a Stay action, then scroll another button into the remembered rectangle.
        let details = action("other_owner", "details");
        click(game, details.center());
        expect_press("other_owner", "details");
        game.options_scroll(-1.0).unwrap();
        assert!(!game.key_focus_key(&NoLoop, KeyCode::Enter));
        assert!(PRESSES.with_borrow(Vec::is_empty));
        draw(game);
        let scrolled = action("other_owner", "extra_01");
        assert_eq!(scrolled.rect, details.rect);
        assert!(!game.key_focus_key(&NoLoop, KeyCode::Enter));
        assert!(PRESSES.with_borrow(Vec::is_empty));

        // From the selected row, Tab to the replacement, then activate it with the controller.
        // A resize between pressing and releasing must not dispatch even with the same selection.
        let second_row = widgets::last_targets()
            .into_iter()
            .filter(|t| t.kind == Kind::Row)
            .nth(1)
            .unwrap();
        click(game, second_row.center());
        tab_to(game, scrolled.rect);
        game.pad_menu_button(&NoLoop, Button::South, true).unwrap();
        game.pad_menu_button(&NoLoop, Button::South, false).unwrap();
        expect_press("other_owner", "extra_01");
        game.pad_menu_button(&NoLoop, Button::South, true).unwrap();
        let previous_scale = game.settings.gui_scale;
        game.settings.gui_scale = if game.gui_size() == (1280.0, 720.0) {
            2
        } else {
            1
        };
        game.pad_menu_button(&NoLoop, Button::South, false).unwrap();
        assert!(!game.key_focus_key(&NoLoop, KeyCode::Enter));
        assert!(PRESSES.with_borrow(Vec::is_empty));
        game.settings.gui_scale = previous_scale;

        let _setup = mod_api::__private::PhaseOwner::scope(mod_api::__private::ModPhase::Setup);
        game.player = None;
        game.title = Some(game.new_title(Game::game_dir().join("saves")));
        show(game, true);
        let details = action("action_owner", "details");
        game.settings.mods.disabled.insert("action_owner".into());
        click(game, details.center());
        assert_parent(game);
        assert!(PRESSES.with_borrow(Vec::is_empty));
        game.settings.mods.disabled.clear();
        click(game, details.center());
        assert_eq!(game.mod_screen(), Some("action_owner"));
        expect_press("action_owner", "details");
        mod_api::prelude::ScreensClientHost::mod_screen_close(game, "action_owner");
        assert_parent(game);
        assert!(game.title.is_some());
    });
}

fn take(log: &Arc<Mutex<Vec<String>>>) -> Vec<String> {
    std::mem::take(&mut *log.lock().unwrap())
}

/// Opening fires once, before the change, and the old screen's closing
/// follows it (`ScreenEvent.Opening`, then `.Closing`): none for the null
/// screen the game plays on, one for each screen it leaves.
#[test]
#[ignore = "daily"]
fn a_new_screen_fires_opening_then_closing_of_the_old_one() {
    run(|game, log, _| {
        game.set_screen(Screen::Inventory);
        assert_eq!(game.screen, Screen::Inventory);
        assert_eq!(take(log), ["opening playing->inventory"]);
        game.set_screen(Screen::Playing);
        assert_eq!(game.screen, Screen::Playing);
        assert_eq!(
            take(log),
            ["opening inventory->playing", "closing inventory"]
        );
    });
}

#[test]
#[ignore = "daily"]
fn cancelling_opening_keeps_the_current_screen() {
    run(|game, log, answer| {
        game.set_screen(Screen::Paused);
        take(log);
        *answer.lock().unwrap() = Some(ScreenOpening::Cancel);
        game.set_screen(Screen::Options);
        assert_eq!(game.screen, Screen::Paused);
        assert_eq!(take(log), ["opening paused->options"]);
    });
}

#[test]
#[ignore = "daily"]
fn replacing_opening_opens_the_mods_screen_instead() {
    run(|game, log, answer| {
        game.set_screen(Screen::Paused);
        take(log);
        *answer.lock().unwrap() = Some(ScreenOpening::ModScreen);
        game.set_screen(Screen::Options);
        assert_eq!(game.screen, Screen::Mod);
        // The second mod answered: the screen is its own, not the passer's.
        assert_eq!(game.mod_screen(), Some("screens"));
        // One Opening, for the screen asked for: the replacement does not fire it again.
        assert_eq!(take(log), ["opening paused->options", "closing paused"]);
        *answer.lock().unwrap() = None;
        game.close_mod_screen();
        assert_eq!(game.screen, Screen::Playing);
        assert_eq!(game.mod_screen(), None);
        assert_eq!(take(log), ["opening mod->playing", "closing mod"]);
    });
}

#[test]
#[ignore = "daily"]
fn the_same_screen_again_fires_nothing() {
    run(|game, log, _| {
        game.set_screen(Screen::Playing);
        assert!(take(log).is_empty());
        game.set_screen(Screen::Inventory);
        take(log);
        game.set_screen(Screen::Inventory);
        assert!(take(log).is_empty());
    });
}

fn observed(id: &'static str, overlay: bool) -> Option<ObservedScreen> {
    Some(ObservedScreen {
        id,
        overlay,
        menu: None,
        title: None,
        rect: [0.0; 4],
        slots: 0,
        mouse: [0.0; 2],
        partial_tick: 0.0,
    })
}

/// An overlay (here a sign editor) is announced as a screen is: Opening
/// when it shows, Opening back to `playing` and Closing when it goes. A
/// `Screen` the observer sees change was announced by `set_screen`, so it
/// is not announced twice.
#[test]
#[ignore = "daily"]
fn an_overlay_fires_opening_then_closing_through_the_observer() {
    run(|game, log, _| {
        game.observe_change(observed("sign_editor", true));
        assert_eq!(take(log), ["opening playing->sign_editor"]);
        game.observe_change(observed("sign_editor", true));
        assert!(take(log).is_empty());
        game.observe_change(None);
        assert_eq!(
            take(log),
            ["opening sign_editor->playing", "closing sign_editor"]
        );
        game.observe_change(observed("inventory", false));
        game.observe_change(None);
        assert!(take(log).is_empty());
        game.observe_change(observed("inventory", false));
        game.observe_change(observed("sign_editor", true));
        assert_eq!(take(log), ["opening playing->sign_editor"]);
    });
}

/// An overlay opens through `open_screen_overlay`: Opening fires before it is
/// set, once, and the frame's observer does not announce it again.
#[test]
#[ignore = "daily"]
fn an_overlay_fires_opening_before_it_opens() {
    run(|game, log, _| {
        game.open_sign_editor((0, 64, 0), true);
        assert!(game.sign_edit.is_some());
        assert_eq!(take(log), ["opening playing->sign_editor"]);
        game.observe_screen();
        assert!(take(log).is_empty());
    });
}

/// A title screen asked for through `Title::set_mode` opens once the game
/// fired `screen_opening` (then `screen_closing` for the one it leaves),
/// and `Cancel` keeps the title screen shown.
#[test]
#[ignore = "daily"]
fn a_title_screen_opens_through_screen_opening() {
    use sunrise_client::title::Mode;
    run(|game, log, answer| {
        // No saves: Singleplayer opens Create New World.
        let saves = std::env::temp_dir().join("sunrise-title-gate-test-no-saves");
        game.title = Some(game.new_title(saves));
        game.settle_title_mode();
        assert!(game.title.as_ref().unwrap().gated);
        *answer.lock().unwrap() = Some(ScreenOpening::Cancel);
        game.title.as_mut().unwrap().enter_singleplayer();
        assert!(matches!(game.title.as_ref().unwrap().mode, Mode::Main));
        game.settle_title_mode();
        assert!(matches!(game.title.as_ref().unwrap().mode, Mode::Main));
        assert_eq!(take(log), ["opening title->world_create"]);
        *answer.lock().unwrap() = None;
        game.title.as_mut().unwrap().enter_singleplayer();
        game.settle_title_mode();
        assert!(matches!(
            game.title.as_ref().unwrap().mode,
            Mode::Create { .. }
        ));
        assert_eq!(take(log), ["opening title->world_create", "closing title"]);
        game.settle_title_mode();
        assert!(take(log).is_empty());
    });
}

/// `Vanilla` opens another vanilla screen where it can: the death screen's
/// way to the title ends on the server list, while the same title target
/// asked for in the world keeps the screen asked for.
#[test]
#[ignore = "daily"]
fn replacing_opening_opens_another_vanilla_screen_where_it_can() {
    use mod_api::ScreenTarget;
    run(|game, log, answer| {
        let saves = std::env::temp_dir().join("sunrise-title-gate-test-no-saves");
        game.title = None;
        game.set_screen(Screen::Dead);
        take(log);
        *answer.lock().unwrap() = Some(ScreenOpening::Vanilla(ScreenTarget::Multiplayer));
        // Leaving the world: no title screen opens in it.
        game.set_screen(Screen::Playing);
        assert_eq!(game.screen, Screen::Playing);
        game.show_title(game.new_title(saves));
        assert_eq!(game.title.as_ref().unwrap().mode.id(), "multiplayer");
        // The replacement fires nothing again.
        assert_eq!(
            take(log),
            [
                "opening dead->playing",
                "closing dead",
                "opening playing->title"
            ]
        );
    });
}

/// `ModScreen` opens the mod's screen over the title's screens with no world,
/// and Escape or the controller's Back returns to the title screen.
#[test]
#[ignore = "daily"]
fn replacing_a_title_screen_opens_the_mods_screen_over_the_title() {
    use sunrise_client::title::Mode;
    run(|game, log, answer| {
        let saves = std::env::temp_dir().join("sunrise-title-gate-test-no-saves");
        game.player = None;
        game.title = Some(game.new_title(saves));
        game.settle_title_mode();
        let back: [fn(&mut Game<'_>); 2] = [
            |game| game.key_pressed(KeyCode::Escape, KeyCode::Escape).unwrap(),
            |game| game.pad_back().unwrap(),
        ];
        for back in back {
            *answer.lock().unwrap() = Some(ScreenOpening::ModScreen);
            game.title.as_mut().unwrap().set_mode(Mode::Select {
                worlds: Vec::new(),
                selected: None,
            });
            game.settle_title_mode();
            assert_eq!(game.screen, Screen::Mod);
            assert_eq!(game.mod_screen(), Some("screens"));
            assert!(matches!(game.title.as_ref().unwrap().mode, Mode::Main));
            assert_eq!(take(log), ["opening title->world_select", "closing title"]);
            *answer.lock().unwrap() = None;
            back(game);
            assert_eq!(game.screen, Screen::Playing);
            assert!(matches!(game.title.as_ref().unwrap().mode, Mode::Main));
            assert_eq!(take(log), ["opening mod->title", "closing mod"]);
        }
    });
}

/// The title appearing (a world closed) and going (a world opened) fires
/// `screen_opening`, then `screen_closing` for what it leaves, before the
/// change, so the frame's observer has nothing left to announce. With no
/// world, `Cancel` doesn't keep the title shut.
#[test]
#[ignore = "daily"]
fn the_title_appears_and_goes_through_screen_opening() {
    run(|game, log, answer| {
        let saves = std::env::temp_dir().join("sunrise-title-gate-test-no-saves");
        game.title = None;
        *answer.lock().unwrap() = Some(ScreenOpening::Cancel);
        game.show_title(game.new_title(saves));
        assert!(game.title.is_some());
        assert_eq!(take(log), ["opening playing->title"]);
        game.observe_screen();
        assert!(take(log).is_empty());
        *answer.lock().unwrap() = None;
        game.hide_title();
        assert!(game.title.is_none());
        assert_eq!(take(log), ["opening title->playing", "closing title"]);
        game.observe_screen();
        assert!(take(log).is_empty());
    });
}

/// `screen_init_post` follows an overlay that opened, after its Opening.
#[test]
#[ignore = "daily"]
fn an_overlay_fires_init_post_after_it_opens() {
    use mod_api::ScreenInitPostEvent;
    /// Hears Opening and Init.Post, in one log.
    struct Ordered(Arc<Mutex<Vec<String>>>);
    #[mod_api::__subscriptions]
    impl Hooks for Ordered {
        fn screen_opening(
            &self,
            _host: &dyn ClientHost,
            event: &mut ScreenOpeningEvent<'_>,
        ) -> Option<ScreenOpening> {
            self.0
                .lock()
                .unwrap()
                .push(format!("opening {}", event.new.id));
            None
        }
        fn screen_init_post(&self, _host: &dyn ClientHost, event: &mut ScreenInitPostEvent<'_>) {
            self.0
                .lock()
                .unwrap()
                .push(format!("init {}", event.screen.id));
        }
    }
    let log = Arc::new(Mutex::new(Vec::new()));
    let mods = vec![ModEntry::new("ordered", Box::new(Ordered(log.clone())))].into();
    with_game_of(mods, |game| {
        game.open_sign_editor((0, 64, 0), true);
        assert_eq!(take(&log), ["opening sign_editor", "init sign_editor"]);
    });
}

/// An overlay a mod's `Vanilla` answer replaced is followed by the screen
/// that opened's `screen_init_post`, once, with no second Opening.
#[test]
#[ignore = "daily"]
fn an_overlay_replaced_by_a_vanilla_screen_fires_its_init_post() {
    use mod_api::{ScreenInitPostEvent, ScreenTarget};
    /// Answers the pause screen for every opening; logs Opening and Init.Post.
    struct ToPause(Arc<Mutex<Vec<String>>>);
    #[mod_api::__subscriptions]
    impl Hooks for ToPause {
        fn screen_opening(
            &self,
            _host: &dyn ClientHost,
            event: &mut ScreenOpeningEvent<'_>,
        ) -> Option<ScreenOpening> {
            self.0
                .lock()
                .unwrap()
                .push(format!("opening {}", event.new.id));
            Some(ScreenOpening::Vanilla(ScreenTarget::Pause))
        }
        fn screen_init_post(&self, _host: &dyn ClientHost, event: &mut ScreenInitPostEvent<'_>) {
            self.0
                .lock()
                .unwrap()
                .push(format!("init {}", event.screen.id));
        }
    }
    let log = Arc::new(Mutex::new(Vec::new()));
    let mods = vec![ModEntry::new("pause", Box::new(ToPause(log.clone())))].into();
    with_game_of(mods, |game| {
        game.title = None;
        game.open_sign_editor((0, 64, 0), true);
        assert!(game.sign_edit.is_none());
        assert_eq!(game.screen, Screen::Paused);
        assert_eq!(take(&log), ["opening sign_editor", "init paused"]);
    });
}

/// The read and the hook see the same live screen, including overlays.
#[test]
#[ignore = "daily"]
fn client_gui_state_matches_screen_init_hook() {
    // Winit permits only one event loop per process, even after it is dropped.
    if crate::game::native_entity_refs::tests::run_fresh_cases(
        "game::native_hooks::base::screen_open_tests::client_gui_state_matches_screen_init_hook",
        "SUNRISE_SCREEN_STATE_CHILD",
        &["window"],
    ) {
        return;
    }
    use mod_api::ScreenInitPostEvent;
    struct Observe(Arc<Mutex<Vec<(String, Option<String>)>>>);
    #[mod_api::__subscriptions]
    impl Hooks for Observe {
        fn screen_opening(
            &self,
            _host: &dyn ClientHost,
            _event: &mut ScreenOpeningEvent<'_>,
        ) -> Option<ScreenOpening> {
            None
        }
        fn screen_init_post(&self, host: &dyn ClientHost, event: &mut ScreenInitPostEvent<'_>) {
            self.0.lock().unwrap().push((
                event.screen.id.to_owned(),
                host.observed_screen_id().map(|id| id.id),
            ));
        }
    }
    let seen = Arc::new(Mutex::new(Vec::new()));
    let mods = vec![ModEntry::new("observe", Box::new(Observe(seen.clone())))].into();
    with_mods(mods, || {
        crate::game::native_entity_refs::tests::with_client_game(|game| {
            game.focused = false;
            assert!(!mod_api::prelude::ScreensClientHost::window_focused(game));
            game.focused = true;
            assert!(mod_api::prelude::ScreensClientHost::window_focused(game));
            game.title = None;
            game.screen = Screen::Playing;
            assert!(mod_api::prelude::ScreensClientHost::observed_screen_id(game).is_none());
            game.screen = Screen::Inventory;
            assert!(mod_api::prelude::ScreensClientHost::observed_screen_id(game).is_none());
            let mut builder = winit::event_loop::EventLoop::builder();
            #[cfg(target_os = "linux")]
            {
                use winit::platform::x11::EventLoopBuilderExtX11;
                builder.with_x11().with_any_thread(true);
            }
            let event_loop = builder.build().unwrap();
            #[allow(deprecated)]
            let window = Arc::new(
                event_loop
                    .create_window(
                        winit::window::Window::default_attributes()
                            .with_visible(false)
                            .with_inner_size(winit::dpi::PhysicalSize::new(320, 240)),
                    )
                    .unwrap(),
            );
            game.options.offscreen = true;
            game.renderer = Some(game.new_renderer(window).unwrap());
            game.screen = Screen::Playing;
            assert!(mod_api::prelude::ScreensClientHost::observed_screen_id(game).is_none());
            game.set_screen(Screen::Inventory);
            game.observe_screen();
            assert_eq!(
                mod_api::prelude::ScreensClientHost::observed_screen_id(game)
                    .unwrap()
                    .id,
                "inventory"
            );
            game.open_screen_overlay("link_prompt", |game| {
                game.link_prompt = Some("https://example.com".into());
            });
            assert_eq!(
                mod_api::prelude::ScreensClientHost::observed_screen_id(game)
                    .unwrap()
                    .id,
                "link_prompt"
            );
            let seen = seen.lock().unwrap();
            assert!(
                seen.iter()
                    .any(|(event, read)| event == "inventory"
                        && read.as_deref() == Some("inventory"))
            );
            assert!(
                seen.iter().any(|(event, read)| event == "link_prompt"
                    && read.as_deref() == Some("link_prompt"))
            );
        });
    });
}

/// `Cancel` keeps an overlay shut, as it keeps a `Screen` shut.
#[test]
#[ignore = "daily"]
fn cancelling_opening_keeps_an_overlay_shut() {
    run(|game, log, answer| {
        *answer.lock().unwrap() = Some(ScreenOpening::Cancel);
        game.open_sign_editor((0, 64, 0), true);
        assert!(game.sign_edit.is_none());
        assert_eq!(take(log), ["opening playing->sign_editor"]);
        let opened = game.open_screen_overlay("link_prompt", |game| {
            game.link_prompt = Some("https://example.com".into())
        });
        assert!(!opened);
        assert!(game.link_prompt.is_none());
        assert_eq!(take(log), ["opening playing->link_prompt"]);
    });
}

/// `ModScreen` opens the answering mod's screen in place of the overlay.
#[test]
#[ignore = "daily"]
fn replacing_opening_of_an_overlay_opens_the_mods_screen() {
    run(|game, log, answer| {
        *answer.lock().unwrap() = Some(ScreenOpening::ModScreen);
        game.open_book(crate::game::books::BookScreen::View {
            pages: Vec::new(),
            page: 0,
            lectern: None,
        });
        assert!(game.book.is_none());
        assert_eq!(game.screen, Screen::Mod);
        assert_eq!(game.mod_screen(), Some("screens"));
        assert_eq!(take(log), ["opening playing->book_view"]);
    });
}

#[test]
#[ignore = "daily"]
fn no_mods_is_a_plain_assignment() {
    with_mods(Vec::new().into(), || {
        crate::game::native_entity_refs::tests::with_game(|game| {
            game.options.execution_role = ExecutionRole::Client;
            game.set_screen(Screen::Inventory);
            assert_eq!(game.screen, Screen::Inventory);
        });
    });
}
