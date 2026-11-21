use super::*;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

#[test]
#[ignore = "daily"]
fn empty_slot_draws_icon_but_a_filled_slot_does_not() {
    mod_platform::testing::with_mods(mod_api::Mods::default(), || {
        crate::game::native_entity_refs::tests::with_client_game(|game| {
            let icon = "minecraft:container/slot/shield";
            let Some(sunrise_client::hud::GuiSprite::Present { rect, .. }) =
                game.gui.atlas.gui_lookup(icon)
            else {
                panic!("the shield icon must resolve to a real GUI sprite");
            };
            game.screen = Screen::Inventory;
            game.container = Some((1, 2, 3));
            game.mod_screen = Some(OpenNativeScreen {
                layout_revision: 0,
                slot_moves: Default::default(),
                key: (1, 2, 3),
                window: 1,
                menu: "test:menu".into(),
                block: Some("test:block".into()),
                screen: mod_api::Screen {
                    slots: vec![Some(mod_api::ScreenSlot {
                        position: [8.0, 18.0],
                        no_item_icon: Some(icon.into()),
                    })],
                    ..Default::default()
                },
                mod_type: None,
                data: Vec::new(),
                extra: None,
            });
            let mut actual = HudBatch::scratch([128, 128]);
            game.draw_slot_item(&mut actual, None, 300, &None, 8.0, 18.0);
            let mut expected = HudBatch::scratch([128, 128]);
            expected.rect_uv(8.0, 18.0, 16.0, 16.0, rect, 0xFFFF_FFFF);
            let geometry = |batch: &HudBatch| {
                batch
                    .vertices
                    .iter()
                    .map(|v| (v.pos, v.uv))
                    .collect::<Vec<_>>()
            };
            assert_eq!(geometry(&actual), geometry(&expected));
            let filled = Some(ItemStack {
                item: "minecraft:stone".into(),
                count: 1,
                damage: 0,
                extra: None,
            });
            let mut with_icon = HudBatch::scratch([128, 128]);
            game.draw_slot_item(&mut with_icon, None, 300, &filled, 8.0, 18.0);
            game.mod_screen.as_mut().unwrap().screen.slots[0]
                .as_mut()
                .unwrap()
                .no_item_icon = None;
            let mut without_icon = HudBatch::scratch([128, 128]);
            game.draw_slot_item(&mut without_icon, None, 300, &filled, 8.0, 18.0);
            assert_eq!(geometry(&with_icon), geometry(&without_icon));
        });
    });
}

struct IconScreen(Arc<AtomicBool>);
#[mod_api::sunrise_part]
impl mod_api::MenuPart for IconScreen {
    fn screen(
        &self,
        _: &dyn mod_api::Host,
        _: &mut mod_api::ScreenEvent<'_>,
    ) -> Option<mod_api::Screen> {
        Some(mod_api::Screen {
            width: 176.0,
            height: 166.0,
            slots: vec![Some(mod_api::ScreenSlot {
                position: [8.0, 18.0],
                no_item_icon: Some(
                    if self.0.load(Ordering::Relaxed) {
                        "icon_open:missing"
                    } else {
                        "minecraft:container/slot/shield"
                    }
                    .into(),
                ),
            })],
            ..Default::default()
        })
    }
}
struct IconMod(Arc<AtomicBool>);
#[mod_api::__subscriptions]
impl mod_api::Hooks for IconMod {
    fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
        let block = r.block_key("icon_open:machine");
        r.block_container(
            &block,
            mod_api::ModContainer {
                slots: 9,
                menu: Some("minecraft:generic_9x1".into()),
                ..Default::default()
            },
        );
        r.menu_part("minecraft:generic_9x1", IconScreen(self.0.clone()));
    }
    fn blocks(&self, _: &mod_api::RegistrationContext, event: &mut mod_api::BlocksEvent<'_>) {
        event
            .register(mod_api::Block::new("icon_open:machine", "minecraft:barrel").with_item(false))
            .unwrap();
    }
}
struct Diagnostics(Mutex<Vec<String>>);
impl log::Log for Diagnostics {
    fn enabled(&self, _: &log::Metadata<'_>) -> bool {
        true
    }
    fn log(&self, record: &log::Record<'_>) {
        self.0.lock().unwrap().push(record.args().to_string());
    }
    fn flush(&self) {}
}
static DIAGNOSTICS: Diagnostics = Diagnostics(Mutex::new(Vec::new()));

#[test]
#[ignore = "daily"]
fn unknown_slot_icon_refuses_screen_at_open_with_diagnostic() {
    // Content registries and the logger are process-global.
    if crate::game::native_entity_refs::tests::run_fresh_cases(
        "game::native_hooks::slot_icon_tests::unknown_slot_icon_refuses_screen_at_open_with_diagnostic",
        "SUNRISE_SLOT_ICON_OPEN_CHILD",
        &["open"],
    ) {
        return;
    }
    log::set_logger(&DIAGNOSTICS).unwrap();
    log::set_max_level(log::LevelFilter::Error);
    let unknown = Arc::new(AtomicBool::new(false));
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "icon_open",
        Box::new(IconMod(unknown.clone())),
    )]
    .into();
    mod_platform::testing::with_mods(mods.clone(), || {
        crate::game::api_values::install_parts(&mods).unwrap();
        mod_api::__private::mark_game_thread();
        let base = std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
        let root = crate::game::mods::content_root(&base, &Default::default()).unwrap();
        crate::game::native_entity_refs::tests::with_game_role(
            root.prepared().to_owned(),
            crate::game::execution_role::ExecutionRole::Client,
            |game| {
                let pos = (1, 64, 2);
                game.world
                    .chunks
                    .insert((0, 0), Arc::new(sunrise_client::chunk::Chunk::empty(0, 0)));
                let block = game
                    .pack
                    .assets
                    .registry
                    .default_state("icon_open:machine")
                    .unwrap();
                game.world.set(pos.0, pos.1, pos.2, block).unwrap();
                game.open_native_screen(pos, Some(pos), 1, "minecraft:generic_9x1", None);
                assert!(game.mod_screen.is_some(), "valid icon must open the screen");
                DIAGNOSTICS.0.lock().unwrap().clear();
                unknown.store(true, Ordering::Relaxed);
                // A panic fails this test; no catch_unwind masks it.
                game.open_native_screen(pos, Some(pos), 2, "minecraft:generic_9x1", None);
                assert!(
                    game.mod_screen.is_none(),
                    "unknown icon must refuse, not retain the prior screen"
                );
                assert!(DIAGNOSTICS.0.lock().unwrap().iter().any(|message|
                    message == "screen for icon_open:machine: unknown empty-slot GUI sprite \"icon_open:missing\""
                ));
            },
        );
    });
}
