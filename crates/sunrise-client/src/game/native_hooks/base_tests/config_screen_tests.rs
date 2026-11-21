use super::*;
use crate::game::{
    EventLoopCtl,
    native_entity_refs::tests::{run_fresh_cases, with_client_game},
};
use mod_api::{
    ClientHost, ConfigScreenContext, ConfigScreenFactory, ConfigScreenParent, ConfigScreenRefusal,
    ConfigScreenRegistrationError, ConfigScreenSession, Flow, Hooks, InputKeyEvent, ModEntry,
    Registrar, RegistrationContext, ScreenCharTypedPreEvent, ScreenKeyPressedPreEvent,
    ScreenKeyReleasedPreEvent, ScreenMouseButtonPressedPostEvent, ScreenMouseButtonPressedPreEvent,
    ScreenMouseButtonReleasedPreEvent, ScreenMouseDraggedPreEvent, ScreenMouseScrolledPreEvent,
    ScreenOpeningEvent, ScreenRenderPostEvent, SettingSpec,
};
use std::{
    sync::{Arc, Mutex},
    time::Instant,
};
use sunrise_client::{mod_list::Installed, options_ui::Page, title::Action};
use winit::{
    application::ApplicationHandler,
    event::{
        DeviceId, ElementState, Ime, KeyEvent, MouseButton, MouseScrollDelta, TouchPhase,
        WindowEvent,
    },
    event_loop::{ActiveEventLoop, EventLoop},
    keyboard::PhysicalKey,
    platform::{pump_events::EventLoopExtPumpEvents, x11::EventLoopBuilderExtX11},
    raw_window_handle::{HasWindowHandle, RawWindowHandle},
    window::WindowId,
};
use x11rb::{
    connection::Connection,
    protocol::{
        xproto::{self, ConnectionExt as _},
        xtest::ConnectionExt as _,
    },
};

#[derive(Default)]
struct State {
    refuse: bool,
    veto: bool,
    calls: Vec<bool>,
    input: Vec<&'static str>,
    characters: Vec<char>,
    raw_w: Vec<mod_api::KeyAction>,
    press_post: usize,
    parent: Option<ConfigScreenParent>,
}

struct Factory(Arc<Mutex<State>>);
impl ConfigScreenFactory for Factory {
    fn create_screen(
        &self,
        owner: &str,
        context: ConfigScreenContext,
        parent: ConfigScreenParent,
    ) -> Result<ConfigScreenSession, ConfigScreenRefusal> {
        assert_eq!(owner, "fixture");
        let mut state = self.0.lock().unwrap();
        state.calls.push(context.in_world());
        if state.refuse {
            return Err(ConfigScreenRefusal::Unavailable);
        }
        state.parent = Some(parent.clone());
        Ok(parent.screen())
    }
}

struct Duplicate;
impl ConfigScreenFactory for Duplicate {
    fn create_screen(
        &self,
        _: &str,
        _: ConfigScreenContext,
        _: ConfigScreenParent,
    ) -> Result<ConfigScreenSession, ConfigScreenRefusal> {
        panic!("duplicate registration replaced the first binding")
    }
}

struct ConfigMod(Arc<Mutex<State>>);
#[mod_api::__subscriptions]
impl Hooks for ConfigMod {
    fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
        r.register_setting(SettingSpec::toggle("show", "test.show", false))
            .unwrap();
        r.register_config_screen_factory(Box::new(Factory(self.0.clone())))
            .unwrap();
        assert_eq!(
            r.register_config_screen_factory(Box::new(Duplicate)),
            Err(ConfigScreenRegistrationError::Duplicate),
        );
    }

    fn screen_opening(
        &self,
        _: &dyn ClientHost,
        event: &mut ScreenOpeningEvent<'_>,
    ) -> Option<ScreenOpening> {
        (event.new.id == "mod" && self.0.lock().unwrap().veto).then_some(ScreenOpening::Cancel)
    }

    fn screen_render_post(&self, host: &dyn ClientHost, event: &mut ScreenRenderPostEvent<'_>) {
        if event.screen.id == "mod" && host.mod_screen() == Some("fixture") {
            event.draw.fill([10.0, 20.0, 12.0, 8.0], 0xfffa20e0);
        }
    }

    fn screen_mouse_button_pressed_pre(
        &self,
        _: &dyn ClientHost,
        event: &mut ScreenMouseButtonPressedPreEvent<'_>,
    ) -> Flow {
        self.input(event.screen.id, "press")
    }

    fn screen_mouse_button_pressed_post(
        &self,
        _: &dyn ClientHost,
        event: &mut ScreenMouseButtonPressedPostEvent<'_>,
    ) {
        if event.screen.id == "mod" {
            self.0.lock().unwrap().press_post += 1;
        }
    }

    fn screen_mouse_button_released_pre(
        &self,
        _: &dyn ClientHost,
        event: &mut ScreenMouseButtonReleasedPreEvent<'_>,
    ) -> Flow {
        self.input(event.screen.id, "release")
    }

    fn screen_mouse_dragged_pre(
        &self,
        _: &dyn ClientHost,
        event: &mut ScreenMouseDraggedPreEvent<'_>,
    ) -> Flow {
        self.input(event.screen.id, "drag")
    }

    fn screen_mouse_scrolled_pre(
        &self,
        _: &dyn ClientHost,
        event: &mut ScreenMouseScrolledPreEvent<'_>,
    ) -> Flow {
        self.input(event.screen.id, "scroll")
    }

    fn screen_key_pressed_pre(
        &self,
        _: &dyn ClientHost,
        event: &mut ScreenKeyPressedPreEvent<'_>,
    ) -> Flow {
        if event.key == "key.keyboard.w" {
            self.input(event.screen.id, "key down")
        } else {
            Flow::Continue
        }
    }

    fn screen_key_released_pre(
        &self,
        _: &dyn ClientHost,
        event: &mut ScreenKeyReleasedPreEvent<'_>,
    ) -> Flow {
        if event.key == "key.keyboard.w" {
            self.input(event.screen.id, "key up")
        } else {
            Flow::Continue
        }
    }

    fn input_key(&self, _: &mut dyn ClientHost, event: &mut InputKeyEvent<'_>) {
        if event.key == "key.keyboard.w" {
            self.0.lock().unwrap().raw_w.push(event.action);
        }
    }

    fn screen_char_typed_pre(
        &self,
        host: &dyn ClientHost,
        event: &mut ScreenCharTypedPreEvent<'_>,
    ) -> Flow {
        if event.screen.id != "mod" {
            return Flow::Continue;
        }
        self.0.lock().unwrap().characters.push(event.c);
        if matches!(event.c, 'x' | 'y') {
            let value = event.c == 'x';
            assert_eq!(host.setting_bool("show"), Some(!value));
            host.set_setting("fixture", "show", if value { "true" } else { "false" });
            host.set_setting("fixture", "show", "invalid");
            assert_eq!(host.setting_bool("show"), Some(value));
        }
        Flow::Cancel
    }
}

impl ConfigMod {
    fn input(&self, screen: &str, event: &'static str) -> Flow {
        if screen != "mod" {
            return Flow::Continue;
        }
        self.0.lock().unwrap().input.push(event);
        if matches!(event, "press" | "key down") {
            Flow::Continue
        } else {
            Flow::Cancel
        }
    }
}

struct Generated;
#[mod_api::__subscriptions]
impl Hooks for Generated {
    fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
        r.register_setting(SettingSpec::toggle("show", "test.show", false))
            .unwrap();
    }
}

struct NoLoop;
impl EventLoopCtl for NoLoop {
    fn exit(&self) {
        panic!("fixture requested exit");
    }
    fn set_control_flow(&self, _: winit::event_loop::ControlFlow) {}
}

/// Pump the X11 key through the real window callback without running idle ticks.
struct KeyDelivery<'a, 'res> {
    game: &'a mut Game<'res>,
    expected: (KeyCode, ElementState),
    received: Option<KeyEvent>,
}

impl ApplicationHandler for KeyDelivery<'_, '_> {
    fn resumed(&mut self, _: &ActiveEventLoop) {}

    fn window_event(&mut self, event_loop: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        if let WindowEvent::KeyboardInput {
            event: key,
            is_synthetic: false,
            ..
        } = &event
            && id == self.game.window.as_ref().unwrap().id()
            && (key.physical_key, key.state)
                == (PhysicalKey::Code(self.expected.0), self.expected.1)
        {
            self.received = Some(key.clone());
            self.game.window_event(event_loop, id, event);
        }
    }
}

#[test]
#[ignore = "daily"]
fn typed_config_screen_keeps_its_mods_parent_and_owns_input() {
    if run_fresh_cases(
        "game::native_hooks::base::config_screen_tests::typed_config_screen_keeps_its_mods_parent_and_owns_input",
        "SUNRISE_CONFIG_SCREEN_CHILD",
        &["window"],
    ) {
        return;
    }
    let state = Arc::new(Mutex::new(State::default()));
    let mods = vec![
        ModEntry::new("fixture", Box::new(ConfigMod(state.clone()))),
        ModEntry::new("generated", Box::new(Generated)),
    ]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_client_game(|game| {
            let mut builder = EventLoop::builder();
            builder.with_x11().with_any_thread(true);
            let mut event_loop = builder.build().unwrap();
            #[allow(deprecated)]
            let window = Arc::new(
                event_loop
                    .create_window(
                        winit::window::Window::default_attributes()
                            .with_inner_size(winit::dpi::PhysicalSize::new(480, 240)),
                    )
                    .unwrap(),
            );
            game.options.offscreen = true;
            game.settings.gui_scale = 1;
            game.renderer = Some(game.new_renderer(window.clone()).unwrap());
            let window_id = window.id();
            let RawWindowHandle::Xlib(handle) = window.window_handle().unwrap().as_raw() else {
                panic!("expected an X11 window");
            };
            game.window = Some(window);

            let (x11, screen) = x11rb::connect(None).unwrap();
            let setup = x11.setup();
            let root = setup.roots[screen].root;
            x11.set_input_focus(xproto::InputFocus::PARENT, handle.window as u32, 0u32)
                .unwrap()
                .check()
                .unwrap();
            let keymap = x11
                .get_keyboard_mapping(setup.min_keycode, setup.max_keycode - setup.min_keycode + 1)
                .unwrap()
                .reply()
                .unwrap();
            let keycode = |keysym| {
                setup.min_keycode
                    + keymap
                        .keysyms
                        .chunks_exact(usize::from(keymap.keysyms_per_keycode))
                        .position(|names| names[0] == keysym)
                        .unwrap() as u8
            };
            let key = |game: &mut Game<'_>, event_loop: &mut EventLoop<()>, code, state| {
                let keysym = match code {
                    KeyCode::KeyW => u32::from(b'w'),
                    KeyCode::Escape => 0xff1b,
                    _ => unreachable!(),
                };
                let kind = if state == ElementState::Pressed {
                    xproto::KEY_PRESS_EVENT
                } else {
                    xproto::KEY_RELEASE_EVENT
                };
                x11.xtest_fake_input(kind, keycode(keysym), 0, root, 0, 0, 0)
                    .unwrap()
                    .check()
                    .unwrap();
                let mut delivery = KeyDelivery {
                    game,
                    expected: (code, state),
                    received: None,
                };
                let deadline = Instant::now() + Duration::from_secs(3);
                while delivery.received.is_none() {
                    let remaining = deadline.saturating_duration_since(Instant::now());
                    assert!(
                        !remaining.is_zero(),
                        "window did not deliver {code:?} {state:?}"
                    );
                    event_loop.pump_app_events(Some(remaining), &mut delivery);
                }
                delivery.received.unwrap()
            };
            let input = |game: &mut Game<'_>, event| {
                game.handle_window_event(
                    &NoLoop,
                    window_id,
                    event,
                    crate::game::input_trace::InputOrigin::Agent,
                );
            };
            let button = |state| WindowEvent::MouseInput {
                device_id: DeviceId::dummy(),
                state,
                button: MouseButton::Left,
            };
            let move_to = |x, y| WindowEvent::CursorMoved {
                device_id: DeviceId::dummy(),
                position: winit::dpi::PhysicalPosition::new(x, y),
            };
            let click = |game: &mut Game<'_>, x, y| {
                input(game, move_to(x, y));
                input(game, button(ElementState::Pressed));
                input(game, button(ElementState::Released));
            };
            let activate = |game: &mut Game<'_>| click(game, 397.0, 192.0);
            let assert_parent = |game: &Game<'_>, parent| {
                assert_eq!(game.screen, Screen::Options);
                assert_eq!(game.ui.options.page(), Page::Mods);
                let list = game.ui.options.mods_screen.as_ref().unwrap();
                assert_eq!(list.id(), parent);
                assert_eq!(list.selected_mod(), Some((0, "fixture")));
            };
            let row = |id: &str| Installed {
                id: id.into(),
                title: "Z Test Mod".into(),
                description: String::new(),
                problem: None,
                dev: false,
                native: None,
            };
            let exercise = |game: &mut Game<'_>, event_loop: &mut EventLoop<()>, in_world: bool| {
                assert_eq!(game.mod_screen(), Some("fixture"));
                state.lock().unwrap().input.clear(); // The activation click is already released.
                assert!(
                    game.render_with(|_, frame| {
                        assert_eq!(frame.title, !in_world);
                        let marker: Vec<_> = frame
                            .hud
                            .vertices
                            .iter()
                            .filter(|v| v.color == sunrise_client::hud::rgba(250, 32, 224, 255))
                            .map(|v| v.pos)
                            .collect();
                        assert_eq!(marker.len(), 6);
                        assert_eq!(marker[0], [10.0, 20.0]);
                        assert_eq!(marker[2], [22.0, 28.0]);
                        Ok(true)
                    })
                    .unwrap()
                );

                input(game, move_to(61.0, 52.0)); // Minecraft's row in the hidden list.
                input(game, button(ElementState::Pressed));
                input(game, move_to(80.0, 60.0));
                input(
                    game,
                    WindowEvent::MouseWheel {
                        device_id: DeviceId::dummy(),
                        delta: MouseScrollDelta::LineDelta(0.0, 1.0),
                        phase: TouchPhase::Moved,
                    },
                );
                input(game, button(ElementState::Released));
                assert!(!game.left);
                let mut press = key(game, event_loop, KeyCode::KeyW, ElementState::Pressed);
                assert!(game.physical_keys.contains(&KeyCode::KeyW));
                assert!(
                    !game.keys.raw(&KeyCode::KeyW),
                    "a held, unconsumed key reached gameplay"
                );
                key(game, event_loop, KeyCode::KeyW, ElementState::Released);
                let c = if in_world { 'y' } else { 'x' };
                if in_world {
                    input(game, WindowEvent::Ime(Ime::Commit(c.to_string())));
                } else {
                    // The shape winit delivers for Windows Alt-code text: a press
                    // with printable text and no physical key.
                    press.physical_key =
                        PhysicalKey::Unidentified(winit::keyboard::NativeKeyCode::Unidentified);
                    press.text = Some(c.to_string().into());
                    input(
                        game,
                        WindowEvent::KeyboardInput {
                            device_id: DeviceId::dummy(),
                            event: press,
                            is_synthetic: false,
                        },
                    );
                }
                assert_eq!(
                    game.stored_setting("fixture", "show").as_deref(),
                    Some(if in_world { "false" } else { "true" })
                );
                let mut heard = state.lock().unwrap();
                assert_eq!(
                    std::mem::take(&mut heard.input),
                    ["press", "drag", "scroll", "release", "key down", "key up"]
                );
                assert_eq!(std::mem::take(&mut heard.characters), ['w', c]);
                assert_eq!(
                    std::mem::take(&mut heard.raw_w),
                    [mod_api::KeyAction::Press, mod_api::KeyAction::Release]
                );
                // A Post for the Config activation click, and another for the screen's click.
                assert_eq!(std::mem::take(&mut heard.press_post), 2);
            };

            game.settings.mods.installed = vec![row("fixture")];
            game.list_native_mods();
            game.player = None;
            game.title = Some(game.new_title(game.save.root.clone()));
            {
                use mod_api::__private::{ModPhase, PhaseOwner, contain_refusal};
                let _setup = PhaseOwner::scope(ModPhase::Setup);
                game.title_action(&NoLoop, Action::Mods).unwrap();
                click(game, 61.0, 76.0);
                let denied = contain_refusal("fixture", || {
                    mod_api::prelude::ConfigHost::set_setting(game, "fixture", "show", "true");
                })
                .unwrap_err();
                assert_eq!(denied.operation, "set_setting");
                assert_eq!(game.stored_setting("fixture", "show"), None);

                let request = game.ui.options.click(
                    &mut game.settings,
                    &game.gui.atlas,
                    397.0,
                    192.0,
                    (480.0, 240.0),
                    [480, 240],
                    false,
                    false,
                );
                game.ui.options.open_at(false, Page::Mods);
                click(game, 61.0, 76.0);
                let title_parent = game.ui.options.mods_screen.as_ref().unwrap().id();
                game.handle_option_events(request).unwrap();
                assert_parent(game, title_parent);
                assert!(
                    state.lock().unwrap().calls.is_empty(),
                    "a stale request invoked the factory"
                );

                activate(game);
                assert_eq!(
                    game.stored_setting("fixture", "show"),
                    None,
                    "opening saves nothing"
                );
                exercise(game, &mut event_loop, false);
                game.replay_line("key w tap", &NoLoop);
                {
                    let mut heard = state.lock().unwrap();
                    assert_eq!(
                        std::mem::take(&mut heard.raw_w),
                        [mod_api::KeyAction::Press, mod_api::KeyAction::Release]
                    );
                    heard.input.clear();
                }
                key(
                    game,
                    &mut event_loop,
                    KeyCode::Escape,
                    ElementState::Pressed,
                );
                key(
                    game,
                    &mut event_loop,
                    KeyCode::Escape,
                    ElementState::Released,
                );
                assert_parent(game, title_parent);

                state.lock().unwrap().refuse = true;
                activate(game);
                assert_parent(game, title_parent);
                state.lock().unwrap().refuse = false;
                state.lock().unwrap().veto = true;
                activate(game);
                assert_parent(game, title_parent);
            }
            let stale = state.lock().unwrap().parent.clone().unwrap();
            state.lock().unwrap().veto = false;

            let _world = mod_api::__private::PhaseOwner::scope(mod_api::__private::ModPhase::World);
            game.title = None;
            game.player = Some(
                sunrise_client::motion::PlayerMotion::new(
                    game.motion,
                    sunrise_core::geometry::Vec3::ZERO,
                )
                .unwrap(),
            );
            game.open_options_at(false, Page::Mods);
            click(game, 61.0, 76.0);
            let world_parent = game.ui.options.mods_screen.as_ref().unwrap().id();
            activate(game);
            exercise(game, &mut event_loop, true);

            // A dialog remains interactive even though the Config session is retained beneath it.
            let inputs = (0..12)
                .map(|index| {
                    format!(
                        "{{key:s{index},type:number_range,label:'Slider {index}',start:0,end:1}}"
                    )
                })
                .collect::<Vec<_>>()
                .join(",");
            let (dialog, _) = sunrise_core::nbt::parse_snbt(&format!(
                "{{type:notice,title:'Server dialog',inputs:[{inputs}]}}"
            ))
            .unwrap();
            game.show_dialog(&dialog);
            assert!(game.dialog_open());
            let mut frame = 0;
            let mut first_slider = |game: &mut Game<'_>| {
                use sunrise_client::widgets::{self, Kind};
                assert!(game.render_with(|_, _| Ok(true)).unwrap());
                frame += 1;
                widgets::end_frame(frame);
                widgets::last_labelled().into_iter().find_map(|widget| {
                    let Kind::Slider { value, track } = widget.kind else {
                        return None;
                    };
                    widget
                        .label
                        .starts_with("Slider 0")
                        .then_some((widget.y, value, track))
                })
            };
            let (y, value, [x, width]) = first_slider(game).expect("visible dialog slider");
            assert_eq!(value, 0.5);
            input(
                game,
                move_to(f64::from(x + width * 0.5), f64::from(y + 10.0)),
            );
            input(game, button(ElementState::Pressed));
            input(game, move_to(f64::from(x + width), f64::from(y + 10.0)));
            input(game, button(ElementState::Released));
            assert_eq!(first_slider(game).unwrap().1, 1.0);
            input(
                game,
                WindowEvent::MouseWheel {
                    device_id: DeviceId::dummy(),
                    delta: MouseScrollDelta::LineDelta(0.0, -1.0),
                    phase: TouchPhase::Moved,
                },
            );
            assert!(first_slider(game).is_none_or(|(scrolled_y, _, _)| scrolled_y < y));
            key(
                game,
                &mut event_loop,
                KeyCode::Escape,
                ElementState::Pressed,
            );
            key(
                game,
                &mut event_loop,
                KeyCode::Escape,
                ElementState::Released,
            );
            assert!(!game.dialog_open());

            stale.go_back();
            game.settle_config_screen();
            assert_eq!(game.mod_screen(), Some("fixture"));
            state.lock().unwrap().parent.as_ref().unwrap().go_back();
            game.settle_config_screen();
            assert_parent(game, world_parent);
            activate(game);
            assert_eq!(game.mod_screen(), Some("fixture"));
            mod_api::prelude::ScreensClientHost::mod_screen_close(game, "fixture");
            assert_parent(game, world_parent);
            assert_eq!(
                state.lock().unwrap().calls,
                [false, false, false, true, true]
            );

            game.settings.mods.installed = vec![row("generated")];
            game.list_native_mods();
            game.open_options_at(false, Page::Mods);
            click(game, 61.0, 76.0);
            activate(game);
            assert_eq!(game.screen, Screen::Options);
            assert_eq!(game.ui.options.page(), Page::NativeMod(0));
        })
    });
}
