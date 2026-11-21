use super::*;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicU8, AtomicUsize, Ordering},
};

struct SlotTip {
    mode: Arc<AtomicU8>,
    calls: Arc<AtomicUsize>,
}
#[mod_api::sunrise_part]
impl mod_api::MenuPart for SlotTip {
    fn screen_tooltip(
        &self,
        _: &dyn mod_api::ClientHost,
        event: &mut mod_api::ScreenTooltipEvent<'_>,
    ) {
        // Player inventory slot 9 is menu slot 27 in a three-row chest.
        assert_eq!(event.slot, Some(27));
        assert_eq!(event.gui_size, [512, 512]);
        assert_eq!(event.origin, [168.0, 173.0]);
        assert_eq!(event.pointer, [184, 265]);
        assert_eq!(event.at, [16.0, 92.0]);
        assert!(
            event
                .lines
                .iter()
                .any(|(line, _)| *line == mod_api::Text::literal("item hook tail"))
        );
        self.calls.fetch_add(1, Ordering::Relaxed);
        match self.mode.load(Ordering::Relaxed) {
            1 => *event.lines = vec![(mod_api::Text::literal("replacement"), u32::MAX)],
            2 => event.lines.clear(),
            3 => event
                .lines
                .push((mod_api::Text::literal("slot hint"), u32::MAX)),
            _ => {}
        }
    }
}
struct Image(Arc<AtomicUsize>);
#[mod_api::sunrise_part]
impl mod_api::ItemPart for Image {
    fn tooltip_component(
        &self,
        _: &dyn mod_api::ClientHost,
        _: &mut mod_api::TooltipComponentEvent<'_>,
    ) -> Option<[f32; 2]> {
        Some([20.0, 10.0])
    }
    fn tooltip_component_draw(
        &self,
        _: &dyn mod_api::ClientHost,
        _: &mut mod_api::TooltipComponentDrawEvent<'_>,
    ) {
        self.0.fetch_add(1, Ordering::Relaxed);
    }
}
struct Listener {
    enabled: bool,
    painted: Arc<AtomicUsize>,
    mode: Arc<AtomicU8>,
    calls: Arc<AtomicUsize>,
    drawn: Arc<Mutex<Vec<Vec<mod_api::TooltipElement>>>>,
}
#[mod_api::__subscriptions]
impl mod_api::Hooks for Listener {
    fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
        let item = r.item_key("minecraft:stone");
        r.item_part(item, Image(self.painted.clone()));
        if self.enabled {
            r.menu_part(
                "minecraft:generic_9x3",
                SlotTip {
                    mode: self.mode.clone(),
                    calls: self.calls.clone(),
                },
            );
        }
    }
    fn item_tooltip(&self, _: &dyn mod_api::ClientHost, event: &mut mod_api::ItemTooltipEvent<'_>) {
        event
            .lines
            .push((mod_api::Text::literal("item hook tail"), 0xFF124578));
    }
    fn render_tooltip_pre(
        &self,
        _: &dyn mod_api::ClientHost,
        event: &mut mod_api::RenderTooltipPreEvent<'_>,
    ) -> mod_api::Flow {
        self.drawn.lock().unwrap().push(event.elements.to_vec());
        mod_api::Flow::Continue
    }
    fn render_tooltip_texture(
        &self,
        _: &dyn mod_api::ClientHost,
        event: &mut mod_api::RenderTooltipTextureEvent<'_>,
    ) {
        assert!(event.stack.is_some());
        event.background = 0xFF123456;
    }
}

#[test]
#[ignore = "daily"]
fn filled_slot_tooltip_replaces_lines_but_keeps_item_image_and_visibility() {
    for enabled in [false, true] {
        let painted = Arc::new(AtomicUsize::new(0));
        let mode = Arc::new(AtomicU8::new(1));
        let calls = Arc::new(AtomicUsize::new(0));
        let drawn = Arc::new(Mutex::new(Vec::new()));
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
            "tips",
            Box::new(Listener {
                enabled,
                painted: painted.clone(),
                mode: mode.clone(),
                calls: calls.clone(),
                drawn: drawn.clone(),
            }),
        )]
        .into();
        mod_platform::testing::with_mods(mods.clone(), || {
            mod_platform::__private::part_tests::install_parts_fixture(&mods).unwrap();
            crate::game::native_entity_refs::tests::with_client_game(|game| {
                let (pipe, _server) = sunrise_protocol::connection::Pipe::pair();
                game.connect_local(pipe, "tips").unwrap();
                game.remote
                    .as_mut()
                    .unwrap()
                    .menu
                    .set(Some((1, "minecraft:generic_9x3".into())));
                game.screen = Screen::Inventory;
                game.container = Some((1, 2, 3));
                game.mod_screen = Some(OpenNativeScreen {
                    layout_revision: 0,
                    slot_moves: Default::default(),
                    key: (1, 2, 3),
                    window: 1,
                    menu: "minecraft:generic_9x3".into(),
                    block: Some("tips:chest".into()),
                    mod_type: None,
                    data: Vec::new(),
                    extra: None,
                    screen: mod_api::Screen {
                        width: 176.0,
                        height: 166.0,
                        inventory_x: 8.0,
                        inventory_y: 84.0,
                        ..Default::default()
                    },
                });
                let mut stack = ItemStack::new("minecraft:stone", 1);
                stack.data_mut().custom_name = Some(sunrise_core::nbt::Tag::compound([
                    ("text", sunrise_core::nbt::Tag::String("Named stone".into())),
                    ("color", sunrise_core::nbt::Tag::String("gold".into())),
                    ("italic", sunrise_core::nbt::Tag::Byte(1)),
                ]));
                game.inventory[9] = Some(stack.clone());
                let mut batch = HudBatch::scratch([512, 512]);
                let (ox, oy) = game.inventory_origin(&batch);
                let (x, y) = game.slot_position(9);
                game.cursor = (ox + x + 8.0, oy + y + 8.0);
                assert_eq!(game.slot_at(&batch, game.cursor.0, game.cursor.1), Some(9));
                game.draw_stack_tooltip(&mut batch, &stack, game.cursor.0, game.cursor.1);
                let original = drawn.lock().unwrap().pop().unwrap();
                assert!(
                    original
                        .iter()
                        .any(|element| matches!(element, mod_api::TooltipElement::Image(_)))
                );
                let before_image_draw = painted.load(Ordering::Relaxed);
                batch.vertices.clear();
                game.draw_slot_tooltip(&mut batch);
                assert!(
                    painted.load(Ordering::Relaxed) > before_image_draw,
                    "the retained image is drawn"
                );
                assert!(
                    batch
                        .vertices
                        .iter()
                        .any(|v| v.color == rgba(0x12, 0x34, 0x56, 0xFF)),
                    "the stack-aware box style is drawn"
                );
                let actual = drawn.lock().unwrap().pop().unwrap();
                if enabled {
                    assert!(
                        matches!(&actual[0], mod_api::TooltipElement::Text(text, _) if *text == mod_api::Text::literal("replacement"))
                    );
                    let images = |elements: &[mod_api::TooltipElement]| {
                        elements
                            .iter()
                            .filter(|element| matches!(element, mod_api::TooltipElement::Image(_)))
                            .cloned()
                            .collect::<Vec<_>>()
                    };
                    assert!(images(&actual) == images(&original));
                    assert_eq!(actual.len(), 2);
                    mode.store(0, Ordering::Relaxed);
                    game.draw_slot_tooltip(&mut batch);
                    assert!(
                        drawn.lock().unwrap().pop().unwrap() == original,
                        "passing preserves styled item lines"
                    );
                    mode.store(3, Ordering::Relaxed);
                    game.draw_slot_tooltip(&mut batch);
                    let appended = drawn.lock().unwrap().pop().unwrap();
                    assert!(
                        appended[..original.len()] == original,
                        "adding a hint preserves every styled source entry"
                    );
                    assert!(
                        matches!(appended.last().unwrap(), mod_api::TooltipElement::Text(text, _) if *text == mod_api::Text::literal("slot hint"))
                    );
                    mode.store(2, Ordering::Relaxed);
                    game.draw_slot_tooltip(&mut batch);
                    assert!(drawn.lock().unwrap().pop().unwrap() == images(&original));
                } else {
                    assert!(
                        actual == original,
                        "without a slot listener, use the item's tooltip"
                    );
                }
                let before = calls.load(Ordering::Relaxed);
                game.carried = Some(stack);
                game.draw_slot_tooltip(&mut batch);
                assert!(drawn.lock().unwrap().is_empty());
                assert_eq!(calls.load(Ordering::Relaxed), before);
            });
        });
    }
}
