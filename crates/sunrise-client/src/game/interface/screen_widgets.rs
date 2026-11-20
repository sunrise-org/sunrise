//! Stock screen listener edits are installed once per ScreenBuild. Stock
//! controls retain their normal handlers; added controls use the same widget
//! rendering, controller targets and narrator registration as those handlers.

use super::{Game, Screen, glfw_modifiers};
use mod_api::client::widgets::{
    Outcome, ScreenWidgetAction, ScreenWidgetBuild, Widget, WidgetId, Widgets,
};
use mod_api::content::crash::{ModCall, enter_native_mod};
use std::cell::RefCell;
use std::sync::atomic::AtomicU64;
use sunrise_client::{hud::HudBatch, ui, window::GuiSize};
use winit::keyboard::KeyCode;

pub(super) struct StockControl {
    pub id: u8,
    pub bounds: [f32; 4],
    pub label: mod_api::Text,
    pub active: bool,
}

use sunrise_client::widgets::{Kind, Target};

pub(super) struct BuiltScreenWidgets {
    stock: Vec<StockControl>,
    removed: [u64; 4],
    added: RefCell<Widgets>,
    actions: Vec<(WidgetId, ScreenWidgetAction)>,
    labels: Vec<std::sync::Arc<crate::text::Text>>,
    targets: Vec<Target>,
    added_targets: Vec<(Target, WidgetId)>,
}

impl BuiltScreenWidgets {
    fn action(&self, outcome: Outcome) -> Option<&ScreenWidgetAction> {
        let Outcome::Pressed(id) = outcome else {
            return None;
        };
        self.actions
            .iter()
            .find_map(|(widget, action)| (*widget == id).then_some(action))
    }
}

impl Game<'_> {
    pub(super) fn init_stock_screen_widgets(
        &mut self,
        identity: Option<&'static str>,
        gui: GuiSize,
        overlay: bool,
    ) {
        let size = (gui.size[0] as f32, gui.size[1] as f32);
        let title = !overlay && self.screen == Screen::Playing && identity == Some("title");
        let pause = !overlay && self.screen == Screen::Paused && self.pause_menu;
        let epoch = self.screen_state.build.widget_epoch;
        let mut view = self
            .observed_screen_view()
            .unwrap_or_else(|| self.screen_change_view(self.screen));
        if let Some(id) = identity {
            view.id = id;
        }
        if view.rect == [0.0; 4] {
            view.rect = [0.0, 0.0, size.0, size.1];
        }
        let mut skip_stock = false;
        if title || pause {
            self.native_screen_init_pre(&view, &mut skip_stock);
        }
        let post = self.has_screen_init_post();
        if !post && !skip_stock {
            return;
        }
        let stock: Vec<StockControl> = if title && !skip_stock {
            let title = self.title.as_ref().expect("title build has a title");
            title
                .main_widget_rows(&self.gui.atlas, size)
                .into_iter()
                .map(|(id, bounds, label, active)| StockControl {
                    id,
                    bounds,
                    label: mod_api::Text::literal(label),
                    active,
                })
                .collect()
        } else if pause && !skip_stock {
            self.pause_stock_widgets(size)
        } else {
            Vec::new()
        };
        // Never reuse a reference from another build or a rolled-back edit.
        static NEXT_LISTENER: AtomicU64 = AtomicU64::new(0);
        let mut staged = ScreenWidgetBuild::__new(
            (title || pause).then_some([size.0, size.1]),
            &NEXT_LISTENER,
            |text| {
                sunrise_client_lib::api_text::to_engine(text).map(|label| {
                    let label: std::sync::Arc<dyn std::any::Any + Send + Sync> =
                        std::sync::Arc::new(label);
                    label
                })
            },
        );
        for widget in &stock {
            staged.__stock(widget.id, widget.bounds, Some(widget.label.clone()));
        }
        if post {
            self.native_screen_init_post(&view, &mut staged);
        }
        if self.screen_state.build.widget_epoch != epoch || (!title && !pause) {
            return;
        }
        let mut removed = if skip_stock { [u64::MAX; 4] } else { [0u64; 4] };
        for widget in &stock {
            removed[usize::from(widget.id / 64)] |= 1 << (widget.id % 64);
        }
        let mut added = Widgets::new();
        let mut actions = Vec::new();
        let mut labels = Vec::new();
        let mut added_targets = Vec::new();
        for (ordinal, entry) in staged.__finish().into_iter().enumerate() {
            if let Some(id) = entry.stock {
                removed[usize::from(id / 64)] &= !(1 << (id % 64));
                continue;
            }
            let (id, active, rect, kind) = match entry.widget {
                Some(Widget::Button(button)) => {
                    let label = entry.render_label.expect("prepared button label");
                    labels.push(
                        label
                            .downcast::<crate::text::Text>()
                            .expect("game prepared the label"),
                    );
                    let (active, rect) = (button.active, button.rect);
                    (added.add_button(button), active, rect, Kind::Button)
                }
                Some(Widget::TextField(field)) => {
                    let (active, rect) = (field.active, field.rect);
                    (added.add_text_field(field), active, rect, Kind::Field)
                }
                _ => continue,
            };
            if active {
                added_targets.push((
                    Target {
                        rect,
                        kind,
                        listener: Some((epoch, 256 + ordinal)),
                    },
                    id,
                ));
            }
            if let Some(action) = entry.action {
                actions.push((id, action));
            }
        }
        let mut stock = stock;
        stock.retain(|widget| removed[usize::from(widget.id / 64)] & (1 << (widget.id % 64)) == 0);
        let targets = stock
            .iter()
            .filter(|control| control.active)
            .map(|control| Target {
                rect: control.bounds,
                kind: Kind::Button,
                listener: Some((epoch, usize::from(control.id))),
            })
            .chain(added_targets.iter().map(|&(target, _)| target))
            .collect::<Vec<_>>();
        if title && let Some(title) = &mut self.title {
            title.set_removed_main_widgets(removed);
        }
        let screen = self.key_screen();
        let stock_focus = if title {
            &mut self
                .title
                .as_mut()
                .expect("title build has a title")
                .focused_button
        } else {
            &mut self.pause_focus
        };
        let old_stock = *stock_focus;
        let old_target = self.pad.menu.focus.filter(|_| {
            self.focus_screen == screen
                || (self.pad.menu.keyboard && self.pad.menu.key_screen == screen)
        });
        let old_stock_id = old_target.and_then(|target| match target.listener {
            Some((_, id)) => u8::try_from(id).ok(),
            None => stock
                .iter()
                .find(|control| control.bounds == target.rect)
                .map(|control| control.id),
        });
        let surviving_stock = |id| {
            targets
                .iter()
                .copied()
                .find(|target| target.listener == Some((epoch, usize::from(id))))
        };
        let replacement = if old_target.is_some() {
            old_stock_id
                .and_then(surviving_stock)
                .or_else(|| targets.first().copied())
        } else if let Some(id) = old_stock {
            surviving_stock(id).or_else(|| targets.first().copied())
        } else {
            None
        };
        *stock_focus = replacement.and_then(|target| u8::try_from(target.listener.unwrap().1).ok());
        if old_target.is_some() {
            self.pad.menu.focus = replacement;
        }
        if let Some(target) = replacement {
            // The first draw must not clear the focus just repaired by init.
            self.focus_screen = self.key_screen();
            if self.pad.menu.keyboard {
                self.pad.menu.key_screen = self.focus_screen;
                self.pad.menu.key_restart = None;
            }
            if let Some(&(_, id)) = added_targets
                .iter()
                .find(|(candidate, _)| candidate.same_control(target))
            {
                added.set_focus(self, Some(id));
                if old_target.is_none() {
                    self.pad.menu.focus = Some(target);
                }
            }
        }
        self.screen_state.build.widgets = Some(BuiltScreenWidgets {
            stock,
            removed,
            added: RefCell::new(added),
            actions,
            labels,
            targets,
            added_targets,
        });
    }

    fn active_stock_widgets(&self) -> Option<&BuiltScreenWidgets> {
        self.screen_state
            .build
            .widgets
            .as_ref()
            .filter(|_| self.overlay_screen_id().is_none())
    }

    pub(super) fn pause_widget_removed(&self, id: u8) -> bool {
        self.screen_state
            .build
            .widgets
            .as_ref()
            .is_some_and(|widgets| widgets.removed[usize::from(id / 64)] & (1 << (id % 64)) != 0)
    }

    pub(super) fn draw_stock_screen_additions(&self, batch: &mut HudBatch) {
        let Some(build) = self.active_stock_widgets() else {
            return;
        };
        let mut draw = super::host::draw::GameDrawContext::new(self, batch);
        draw.widget_labels = Some(build.labels.iter());
        let mouse = if self.pad.menu.focus.is_some() && self.pad_focus_mode() {
            // The controller's synthetic pointer is not a hover on every
            // widget under it: only the selected listener has focus.
            [f32::INFINITY; 2]
        } else {
            [self.cursor.0, self.cursor.1]
        };
        let _ = build.added.borrow_mut().draw(&mut draw, self, mouse);
    }

    fn press_stock_screen_button(&mut self, action: ScreenWidgetAction) {
        let _owner = enter_native_mod(ModCall {
            id: action.owner,
            hook: "screen_widget_press",
            part: None,
        });
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            (action.press)(self, &action.id)
        }));
    }

    pub(super) fn stock_widget_mouse(&mut self, pressed: bool) -> bool {
        let Some(build) = self.active_stock_widgets() else {
            return false;
        };
        let outcome = if pressed {
            if build.stock.iter().any(|s| {
                let [x, y, w, h] = s.bounds;
                s.active && ui::hovered(self.cursor, x, y, w, h)
            }) {
                build.added.borrow_mut().set_focus(self, None);
                return false;
            }
            build.added.borrow_mut().mouse_pressed(
                self,
                [self.cursor.0, self.cursor.1],
                0,
                glfw_modifiers(self.modifiers),
            )
        } else {
            build.added.borrow_mut().mouse_released(0)
        };
        if pressed && outcome.handled() {
            let action = build.action(outcome).cloned();
            if self.screen == Screen::Paused {
                self.pause_focus = None;
            } else if let Some(title) = &mut self.title {
                title.focused_button = None;
            }
            if let Some(action) = action {
                self.press_stock_screen_button(action);
            }
        }
        outcome.handled()
    }

    pub(super) fn end_stock_widget_capture(&self) -> bool {
        self.screen_state
            .build
            .widgets
            .as_ref()
            .is_some_and(|build| build.added.borrow_mut().mouse_released(0).handled())
    }

    pub(super) fn stock_widget_drag(&self) -> bool {
        self.active_stock_widgets().is_some_and(|build| {
            build
                .added
                .borrow_mut()
                .mouse_dragged(self, [self.cursor.0, self.cursor.1], 0)
                .handled()
        })
    }

    pub(super) fn stock_widget_text_focused(&self) -> bool {
        self.active_stock_widgets().is_some_and(|build| {
            let added = build.added.borrow();
            added
                .focused()
                .is_some_and(|id| added.text_field(id).is_some())
        })
    }

    pub(super) fn stock_widget_edit(
        &self,
        edited: Option<&mod_api::client::widgets::TextEdit>,
    ) -> Option<mod_api::client::widgets::TextEdit> {
        let mut added = self.active_stock_widgets()?.added.borrow_mut();
        let id = added.focused()?;
        let field = added.text_field_mut(id)?;
        if let Some(edited) = edited {
            if field.value() != edited.value {
                field.edit.select_all();
                field.edit.insert_text(&edited.value);
            }
            field.edit.set_cursor(edited.highlight, false);
            field.edit.set_cursor(edited.cursor, true);
        }
        Some(field.edit.clone())
    }

    pub(super) fn stock_widget_type(&self, text: &str) -> bool {
        let Some(build) = self.active_stock_widgets() else {
            return false;
        };
        let mut added = build.added.borrow_mut();
        let Some(id) = added.focused() else {
            return false;
        };
        if added.text_field(id).is_none() {
            return false;
        }
        for c in text.chars() {
            added.char_typed(self, c);
        }
        true
    }

    pub(super) fn clear_stock_widget_focus(&self) {
        if let Some(build) = self.active_stock_widgets() {
            build.added.borrow_mut().set_focus(self, None);
        }
    }

    pub(super) fn stock_widget_targets(&self) -> Option<&[Target]> {
        Some(&self.active_stock_widgets()?.targets)
    }

    pub(super) fn focus_stock_widget(&self, target: Target) -> bool {
        let Some(build) = self.active_stock_widgets() else {
            return false;
        };
        let id = build
            .added_targets
            .iter()
            .find_map(|(candidate, id)| candidate.same_control(target).then_some(*id));
        build.added.borrow_mut().set_focus(self, id);
        id.is_some()
    }

    pub(super) fn press_stock_widget_target(&mut self, target: Target) -> bool {
        let Some(build) = self.active_stock_widgets() else {
            return false;
        };
        let Some(&(_, id)) = build
            .added_targets
            .iter()
            .find(|(candidate, _)| candidate.same_control(target))
        else {
            return false;
        };
        if target.kind != Kind::Button {
            return false;
        }
        // Arrow/Tab focus already identifies the target. Don't route the
        // activation through a pointer hit test that may find a different widget.
        let mut added = build.added.borrow_mut();
        added.set_focus(self, Some(id));
        let outcome = added.key_pressed(self, "key.keyboard.enter", 0);
        drop(added);
        if let Some(action) = build.action(outcome).cloned() {
            self.press_stock_screen_button(action);
        }
        outcome.handled()
    }

    pub(super) fn stock_widget_key(&mut self, key: KeyCode) -> bool {
        if key == KeyCode::Tab {
            return false;
        }
        let Some(name) = sunrise_client::keybinds::key_name(key) else {
            return false;
        };
        let Some(build) = self.active_stock_widgets() else {
            return false;
        };
        let outcome =
            build
                .added
                .borrow_mut()
                .key_pressed(self, name, glfw_modifiers(self.modifiers));
        if let Some(action) = build.action(outcome).cloned() {
            self.press_stock_screen_button(action);
        }
        outcome.handled()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::{
        event_loop::EventLoopCtl, native_entity_refs::tests::with_client_game,
        native_hooks::particle_count_tests::frame_scene,
    };
    use mod_api::client::widgets::{Button, ScreenWidgetError, ScreenWidgetRef};
    use mod_api::{
        ClientHost, Flow, Hooks, ModEntry, ScreenCharTypedPreEvent, ScreenInitPostEvent,
        ScreenInitPreEvent, ScreenMouseButtonReleasedPreEvent, Text,
    };
    use mod_platform::testing::with_mods;
    use std::cell::RefCell;
    use std::sync::{Arc, Mutex};
    use sunrise_client::{gamepad, osk, widgets};
    use winit::{
        dpi::PhysicalSize,
        event::{DeviceId, ElementState, MouseButton, WindowEvent},
    };

    fn resize(game: &mut Game<'_>, width: u32, height: u32) {
        game.handle_window_event_inner(
            &NoLoop,
            WindowEvent::Resized(PhysicalSize::new(width, height)),
            None,
        );
    }

    fn left(game: &mut Game<'_>, state: ElementState) {
        game.handle_window_event_inner(
            &NoLoop,
            WindowEvent::MouseInput {
                device_id: DeviceId::dummy(),
                state,
                button: MouseButton::Left,
            },
            None,
        );
    }

    fn frame(game: &mut Game<'_>, generation: u64) -> widgets::Narration {
        widgets::set_narrating(true);
        assert!(game.render_with(|_, _| Ok(true)).unwrap());
        widgets::end_frame(generation);
        widgets::set_narrating(false);
        widgets::take_last_narration()
    }

    const ADDED: [f32; 4] = [9.0, 17.0, 60.0, 20.0];
    thread_local! {
        static PRESSES: RefCell<Vec<(&'static str, String)>> = const { RefCell::new(Vec::new()) };
    }

    fn press(_: &mut dyn ClientHost, action: &str) {
        let owner = mod_api::content::crash::current_native_mod().unwrap();
        PRESSES.with_borrow_mut(|p| p.push((owner, action.to_owned())));
    }

    struct NoLoop;
    impl EventLoopCtl for NoLoop {
        fn exit(&self) {}
        fn set_control_flow(&self, _: winit::event_loop::ControlFlow) {}
    }

    struct Edits(Arc<Mutex<Vec<ScreenWidgetRef>>>);
    #[mod_api::__subscriptions]
    impl Hooks for Edits {
        fn screen_init_post(&self, _: &dyn ClientHost, event: &mut ScreenInitPostEvent<'_>) {
            if event.screen.id != "paused" {
                return;
            }
            let mut refs = self.0.lock().unwrap();
            if let Some(&old) = refs.last() {
                assert_eq!(
                    event.widgets.remove_listener(old),
                    Err(ScreenWidgetError::UnknownListener)
                );
            }
            let options = event
                .widgets
                .listeners()
                .find(|view| view.label == Some(&Text::translate("menu.options", vec![])))
                .unwrap();
            event.widgets.remove_listener(options.listener).unwrap();
            let widget = Widget::Button(Button::new(ADDED, Text::literal("Example action")));
            refs.push(
                event
                    .widgets
                    .add_listener(widget, Some(("settings", press)))
                    .unwrap(),
            );
        }
    }

    struct Refused;
    #[mod_api::__subscriptions]
    impl Hooks for Refused {
        fn screen_init_post(&self, _: &dyn ClientHost, event: &mut ScreenInitPostEvent<'_>) {
            if event.screen.id != "paused" {
                return;
            }
            let widget = Widget::Button(Button::new(
                [5.0, 60.0, 60.0, 20.0],
                Text::literal("Refused"),
            ));
            event
                .widgets
                .add_listener(widget, Some(("refused", press)))
                .unwrap();
            panic!("reject this listener's edits");
        }
    }

    struct Later;
    #[mod_api::__subscriptions]
    impl Hooks for Later {
        fn screen_init_post(&self, _: &dyn ClientHost, event: &mut ScreenInitPostEvent<'_>) {
            if event.screen.id != "paused" {
                return;
            }
            assert_eq!(event.widgets.listeners().last().unwrap().bounds, ADDED);
            assert!(
                !event
                    .widgets
                    .listeners()
                    .any(|view| view.label == Some(&Text::literal("Refused")))
            );
            let widget = Widget::Button(Button::new(ADDED, Text::literal("Second action")));
            event
                .widgets
                .add_listener(widget, Some(("settings", press)))
                .unwrap();
        }
    }

    #[test]
    #[ignore = "daily"]
    fn pause_listeners_are_ordered_and_pressed_only_by_the_owner() {
        let refs = Arc::new(Mutex::new(Vec::new()));
        let mods = vec![
            ModEntry::new("first", Box::new(Edits(refs.clone()))),
            ModEntry::new("refused", Box::new(Refused)),
            ModEntry::new("later", Box::new(Later)),
        ]
        .into();
        with_mods(mods, || {
            with_client_game(|game| {
                let _event_loop = frame_scene(game);
                game.settings.gui_scale = 3;
                resize(game, 1280, 720);
                game.pause_menu = true;
                game.pause_focus = Some(1); // Options, removed by the first listener.
                game.set_screen(Screen::Paused);
                let said = frame(game, 1);
                assert_eq!(game.pause_focus, Some(0)); // Repaired through the first frame.
                assert!(
                    !said
                        .items
                        .iter()
                        .any(|item| item.rect == [111.0, 140.0, 98.0, 20.0])
                );
                let buttons: Vec<_> = said
                    .items
                    .iter()
                    .filter_map(|item| match &item.what {
                        widgets::What::Button { message, .. } => Some(message.as_str()),
                        _ => None,
                    })
                    .collect();
                assert!(buttons.ends_with(&["Example action", "Second action"]));

                // A real mouse event over the former Options button does not open it.
                game.cursor = (125.0, 149.0);
                left(game, ElementState::Pressed);
                left(game, ElementState::Released);
                assert_eq!(game.screen, Screen::Paused);

                let targets = game.stock_widget_targets().unwrap();
                let [.., last_stock, first, second] = targets else {
                    panic!("expected stock and two additions")
                };
                let (last_stock, first, second) = (*last_stock, *first, *second);
                assert_eq!(first.rect, second.rect);
                assert!(!first.same_control(second));
                game.cursor = (20.0, 20.0);
                left(game, ElementState::Pressed);
                left(game, ElementState::Released);
                game.clear_stock_widget_focus();
                game.key_focus_adopt(last_stock, false);
                assert!(game.key_focus_key(&NoLoop, KeyCode::Tab));
                assert_eq!(game.pad.menu.focus, Some(first));
                assert!(game.key_focus_key(&NoLoop, KeyCode::Tab));
                assert_eq!(game.pad.menu.focus, Some(second));
                frame(game, 2);
                let targets = game.stock_widget_targets().unwrap().to_vec();
                game.pad_follow_focus(&NoLoop, &targets);
                assert!(game.key_focus_key(&NoLoop, KeyCode::Enter));
                game.pad.menu.focus = Some(first);
                game.pad_menu_button(&NoLoop, gamepad::Button::South, true)
                    .unwrap();
                game.pad_menu_button(&NoLoop, gamepad::Button::South, false)
                    .unwrap();
                assert_eq!(
                    PRESSES.with_borrow_mut(std::mem::take),
                    [
                        ("first", "settings".to_owned()),
                        ("later", "settings".to_owned()),
                        ("first", "settings".to_owned()),
                    ]
                );
                assert_eq!(refs.lock().unwrap().len(), 1);

                // The physical viewport changed, but both GUI extents round to the same pixels.
                let old_gui = game.gui_size();
                game.pad.menu.focus = Some(first);
                game.pad_menu_button(&NoLoop, gamepad::Button::South, true)
                    .unwrap();
                resize(game, 1279, 719);
                assert_eq!(game.gui_size(), old_gui);
                game.pad_menu_button(&NoLoop, gamepad::Button::South, false)
                    .unwrap();
                assert!(PRESSES.with_borrow(Vec::is_empty));
                assert_eq!(refs.lock().unwrap().len(), 2);
                assert_eq!(
                    game.stock_widget_targets()
                        .unwrap()
                        .iter()
                        .filter(|target| target.rect == ADDED)
                        .count(),
                    2
                );
                game.set_screen(Screen::Playing);
                assert!(game.stock_widget_targets().is_none());
                assert!(!game.stock_widget_mouse(true));
            })
        });
    }

    #[test]
    #[ignore = "daily"]
    fn no_post_listener_preserves_stock_widgets() {
        with_mods(Vec::new().into(), || {
            with_client_game(|game| {
                let _event_loop = frame_scene(game);
                game.settings.gui_scale = 3;
                resize(game, 1280, 720);
                game.pause_menu = true;
                game.set_screen(Screen::Paused);
                let said = frame(game, 1);
                assert!(
                    said.items
                        .iter()
                        .any(|item| item.rect == [111.0, 140.0, 98.0, 20.0])
                );
                assert!(game.stock_widget_targets().is_none());
                game.cursor = (125.0, 149.0);
                left(game, ElementState::Pressed);
                left(game, ElementState::Released);
                assert_eq!(game.screen, Screen::Options);
            })
        });
    }

    struct TitleField;
    #[mod_api::__subscriptions]
    impl Hooks for TitleField {
        fn screen_mouse_button_released_pre(
            &self,
            _: &dyn ClientHost,
            _: &mut ScreenMouseButtonReleasedPreEvent<'_>,
        ) -> Flow {
            Flow::Cancel
        }

        fn screen_char_typed_pre(
            &self,
            _: &dyn ClientHost,
            event: &mut ScreenCharTypedPreEvent<'_>,
        ) -> Flow {
            if event.c == '!'
                && let Some(field) = event.field.as_mut()
            {
                field.value.push_str("cd");
                field.cursor = 4;
                field.highlight = 4;
                return Flow::Cancel;
            }
            Flow::Continue
        }

        fn screen_init_post(&self, _: &dyn ClientHost, event: &mut ScreenInitPostEvent<'_>) {
            if event.screen.id != "title" {
                return;
            }
            let standard_options = event
                .widgets
                .listeners()
                .find(|row| row.bounds == [113.0, 204.0, 98.0, 20.0])
                .unwrap();
            event
                .widgets
                .remove_listener(standard_options.listener)
                .unwrap();
            let mut field = mod_api::client::widgets::TextField::new(ADDED, 32);
            field.message = "Example query".into();
            event
                .widgets
                .add_listener(Widget::TextField(field), None)
                .unwrap();
        }
    }

    #[test]
    #[ignore = "daily"]
    fn title_post_mounts_an_edit_box_and_a_modal_does_not_reinitialize_it() {
        let mods = vec![ModEntry::new("title_field", Box::new(TitleField))].into();
        with_mods(mods, || {
            with_client_game(|game| {
                let _event_loop = frame_scene(game);
                game.settings.gui_scale = 3;
                resize(game, 1280, 720);
                let saves = tempfile::tempdir().unwrap();
                game.show_title(game.new_title(saves.path().to_owned()));
                assert_eq!(
                    game.stock_widget_targets().unwrap().last().unwrap().kind,
                    Kind::Field
                );
                assert!(
                    !game
                        .stock_widget_targets()
                        .unwrap()
                        .iter()
                        .any(|row| row.rect == [113.0, 204.0, 98.0, 20.0])
                );
                let field = *game.stock_widget_targets().unwrap().last().unwrap();
                game.key_focus_adopt(field, false);
                game.pad_menu_button(&NoLoop, gamepad::Button::South, true)
                    .unwrap();
                assert!(game.pad.menu.osk.is_some());
                game.pad_osk_output(osk::Output::Text("ab!".into()));
                game.pad_osk_output(osk::Output::Backspace);
                assert_eq!(game.stock_widget_edit(None).unwrap().value, "abc");
                game.pad_osk_output(osk::Output::Close);
                frame(game, 1);
                for release in [true, false] {
                    game.cursor = (20.0, 20.0);
                    left(game, ElementState::Pressed);
                    if release {
                        // The release's Pre hook cancels delivery to the field.
                        left(game, ElementState::Released);
                    } else {
                        game.handle_window_event_inner(&NoLoop, WindowEvent::Focused(false), None);
                    }
                    let before = game.stock_widget_edit(None).unwrap();
                    let projection = game.renderer_pointer_projection().unwrap();
                    let position = projection.window_pointer((72.0, 20.0));
                    let sample = sunrise_client::window::PointerSample::Physical {
                        position,
                        backing_scale: 1.0,
                    };
                    game.handle_window_event_inner(
                        &NoLoop,
                        WindowEvent::CursorMoved {
                            position,
                            device_id: DeviceId::dummy(),
                        },
                        Some(sample),
                    );
                    assert_eq!(game.stock_widget_edit(None).unwrap(), before);
                }
                // A sign editor is an overlay, not a new title build.
                game.open_sign_editor((0, 64, 0), true);
                assert!(game.stock_widget_targets().is_none());
                game.sign_edit = None;
                game.build_screen();
                let added = game.active_stock_widgets().unwrap().added.borrow();
                assert_eq!(
                    added.text_field(added.focused().unwrap()).unwrap().value(),
                    "abc"
                );
            })
        });
    }

    struct SkipStock;
    #[mod_api::__subscriptions]
    impl Hooks for SkipStock {
        fn screen_init_pre(&self, _: &dyn ClientHost, _: &mut ScreenInitPreEvent<'_>) -> Flow {
            Flow::Cancel
        }

        fn screen_init_post(&self, _: &dyn ClientHost, event: &mut ScreenInitPostEvent<'_>) {
            if !matches!(event.screen.id, "title" | "paused") {
                return;
            }
            assert_eq!(event.widgets.listeners().len(), 0);
            let mut opaque = Text::literal("Invalid label");
            opaque.metadata = sunrise_core::nbt::Tag::compound([(
                "number",
                sunrise_core::nbt::Tag::Float(f32::NAN.to_bits()),
            )]);
            let binding = Some(("only", press as mod_api::client::widgets::ScreenWidgetPress));
            assert_eq!(
                event
                    .widgets
                    .add_listener(Widget::Button(Button::new(ADDED, opaque)), binding),
                Err(ScreenWidgetError::InvalidText)
            );
            event
                .widgets
                .add_listener(
                    Widget::Button(Button::new(ADDED, Text::literal("Only added control"))),
                    binding,
                )
                .unwrap();
        }
    }

    #[test]
    #[ignore = "daily"]
    fn pre_skips_stock_but_post_can_install_the_only_control() {
        let mods = vec![ModEntry::new("skip_stock", Box::new(SkipStock))].into();
        with_mods(mods, || {
            with_client_game(|game| {
                let _event_loop = frame_scene(game);
                game.settings.gui_scale = 3;
                resize(game, 1280, 720);
                let saves = tempfile::tempdir().unwrap();
                game.show_title(game.new_title(saves.path().to_owned()));
                let title = frame(game, 1);
                assert_eq!(title.items.len(), 1);
                assert!(
                    matches!(&title.items[0].what, widgets::What::Button { message, .. } if message == "Only added control")
                );
                game.hide_title();
                game.pause_menu = true;
                game.pause_focus = Some(1);
                game.set_screen(Screen::Paused);
                let paused = frame(game, 2);
                assert_eq!(paused.items.len(), 1);
                assert!(
                    matches!(&paused.items[0].what, widgets::What::Button { message, .. } if message == "Only added control")
                );
                // The removed stock focus repaired to the only remaining control,
                // not to the old Options rectangle or to an absent stock row.
                assert_eq!(
                    game.pad.menu.focus,
                    game.stock_widget_targets().unwrap().first().copied()
                );
                game.cursor = (20.0, 20.0);
                left(game, ElementState::Pressed);
                left(game, ElementState::Released);
                assert_eq!(
                    PRESSES.with_borrow_mut(std::mem::take),
                    [("skip_stock", "only".to_owned())]
                );
            })
        });
    }
}
