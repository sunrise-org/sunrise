use super::*;
use std::sync::{Arc, Mutex};
use winit::dpi::PhysicalPosition;

#[derive(Debug, PartialEq)]
struct Frame {
    pointer: [i32; 2],
    size: [i32; 2],
    origin: [f32; 2],
}

#[derive(Default)]
struct ScreenEvents {
    /// The block whose menu it is and the position heard by `screen_draw`.
    draw: Vec<(Option<String>, Option<[i32; 3]>)>,
    frames: Vec<Frame>,
    tooltip_at: Vec<[f32; 2]>,
    data: Vec<Vec<i32>>,
}
type Seen = Arc<Mutex<ScreenEvents>>;
/// A mod menu type no block opens here.
const PACK: &str = "test:pack";

struct Position(Seen);
#[mod_api::sunrise_part]
impl mod_api::MenuPart for Position {
    fn screen(
        &self,
        _: &dyn mod_api::Host,
        event: &mut mod_api::ScreenEvent<'_>,
    ) -> Option<mod_api::Screen> {
        // A block's screen leaves slot 4 out; without a block it shows none.
        let slots = match event.block {
            Some(_) => (0..27)
                .map(|i| (i != 4).then(mod_api::ScreenSlot::default))
                .collect(),
            None => Vec::new(),
        };
        Some(mod_api::Screen {
            height: if event.block.is_some() { 166.0 } else { 133.0 },
            slots,
            own_labels: true,
            own_background: true,
            ..Default::default()
        })
    }
    fn screen_draw(&self, _: &dyn mod_api::ClientHost, event: &mut mod_api::ScreenDrawEvent<'_>) {
        let mut seen = self.0.lock().unwrap();
        seen.draw.push((event.block.map(str::to_owned), event.pos));
        seen.data.push(event.data.to_vec());
        seen.frames.push(Frame {
            pointer: event.pointer,
            size: event.gui_size,
            origin: event.origin,
        });
    }

    fn screen_tooltip(
        &self,
        _: &dyn mod_api::ClientHost,
        event: &mut mod_api::ScreenTooltipEvent<'_>,
    ) {
        let mut seen = self.0.lock().unwrap();
        let draw = seen.frames.last().unwrap();
        assert_eq!(event.pointer, draw.pointer);
        assert_eq!(event.gui_size, draw.size);
        assert_eq!(event.origin, draw.origin);
        seen.tooltip_at.push(event.at);
    }
}
struct Listener(Seen);
#[mod_api::__subscriptions]
impl mod_api::Hooks for Listener {
    fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
        r.menu_part("minecraft:generic_9x3", Position(self.0.clone()));
        r.menu_part(PACK, Position(self.0.clone()));
    }
}
fn with_listener(test: impl FnOnce(&mut Game<'_>, &Seen)) {
    let seen = Seen::default();
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "test",
        Box::new(Listener(seen.clone())),
    )]
    .into();
    mod_platform::testing::with_mods(mods.clone(), || {
        mod_platform::__private::part_tests::install_parts_fixture(&mods).unwrap();
        crate::game::native_entity_refs::tests::with_client_game(|game| test(game, &seen));
    });
}
#[test]
#[ignore = "daily"]
fn slotless_screen_draw_sees_full_width_updates_only_for_its_window() {
    use sunrise_protocol::{
        connection::Pipe,
        menu_extra::{DATA_CHANNEL, DataSlot, LIVE_DATA_REVISION},
        packets::{
            configuration::CustomPayload,
            play::{Clientbound as C, ContainerClose, OpenScreen},
        },
    };
    sunrise_client::containers::with_mod_menu_types(
        [(
            PACK.into(),
            "test".into(),
            0,
            2,
            Vec::new(),
            Vec::new(),
            None,
            None,
            false,
            None,
        )],
        || {
            with_listener(|game, seen| {
                let (pipe, _server) = Pipe::pair();
                game.connect_local(pipe, "Slotless").unwrap();
                let remote = game.remote.as_mut().unwrap();
                Arc::make_mut(&mut remote.registries).menu_extra_revision = LIVE_DATA_REVISION;
                Arc::make_mut(&mut remote.builtin).registries.insert(
                    "minecraft:menu".into(),
                    sunrise_protocol::registry::Ids::from_names(
                        ["minecraft:anvil", PACK].map(str::to_owned).to_vec(),
                    ),
                );
                let open = |game: &mut Game<'_>, window, menu| {
                    game.handle_packet(C::OpenScreen(OpenScreen {
                        container: window,
                        menu,
                        title: sunrise_core::nbt::Tag::String("Test".into()),
                    }))
                    .unwrap();
                };
                let receive = |game: &mut Game<'_>, window, index, value| {
                    game.handle_packet(C::CustomPayload(CustomPayload {
                        channel: DATA_CHANNEL.into(),
                        data: DataSlot {
                            window,
                            index,
                            value,
                        }
                        .encode(),
                    }))
                    .unwrap();
                };
                let draw = |game: &Game<'_>| {
                    let mut batch = HudBatch::scratch([200, 200]);
                    assert!(game.draw_native_screen(&mut batch, 0.0, 0.0));
                };
                open(game, 1, 0); // Anvil
                assert!(game.station.is_some());
                assert!(!game.slots().is_empty());
                game.cursor = (25.0, 40.0);
                open(game, 2, 1); // No intervening close packet.
                assert_eq!(game.screen, Screen::Inventory);
                assert_eq!(game.cursor, (25.0, 40.0));
                assert!(game.station.is_none());
                assert!(game.slots().is_empty());
                assert_eq!(game.slot_count(), 0);
                assert_eq!(Game::inventory_screen_slot(PACK, 0), None);
                assert_eq!(
                    game.slot_at(&HudBatch::scratch([200, 200]), 25.0, 40.0),
                    None
                );
                receive(game, 2, 0, -80_000);
                receive(game, 2, 1, 3);
                draw(game);
                receive(game, 2, 2, 9);
                receive(game, 2, 0, i32::MAX);
                draw(game);
                open(game, 3, 1);
                receive(game, 2, 0, 8);
                draw(game);
                receive(game, 3, 0, -17);
                receive(game, 3, 1, i32::MIN);
                draw(game);
                game.handle_packet(C::ContainerClose(ContainerClose { container: 3 }))
                    .unwrap();
                receive(game, 3, 1, 18);
                assert!(game.shown_native_screen().is_none());
                assert_eq!(
                    seen.lock().unwrap().data,
                    [
                        vec![-80_000, 3],
                        vec![i32::MAX, 3],
                        vec![],
                        vec![-17, i32::MIN],
                    ]
                );
            })
        },
    );
}

/// A mod that adds `test:crate`, a block whose container opens vanilla's
/// `minecraft:generic_9x3`, with the `Position` screen on that type.
struct CrateMod(Seen);
#[mod_api::__subscriptions]
impl mod_api::Hooks for CrateMod {
    fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
        let block = r.block_key("test:crate");
        r.block_container(
            &block,
            mod_api::ModContainer {
                slots: 27,
                menu: Some("minecraft:generic_9x3".into()),
                ..Default::default()
            },
        );
        r.menu_part("minecraft:generic_9x3", Position(self.0.clone()));
    }
    fn blocks(&self, _: &mod_api::RegistrationContext, event: &mut mod_api::BlocksEvent<'_>) {
        event
            .register(mod_api::Block::new("test:crate", "minecraft:barrel").with_item(false))
            .unwrap();
    }
}
/// A mod block whose container opens a vanilla menu type: the client's screen is the type's
/// part's, told the block and its position, and the server asks the same part with the block
/// and leaves the slot it hides out of the menu.
#[test]
#[ignore = "daily"]
fn a_mod_blocks_menu_of_a_vanilla_type_gets_the_types_screen_on_both_sides() {
    // Content registries are process-global.
    if crate::game::native_entity_refs::tests::run_fresh_cases(
        "game::native_hooks::screen_position_tests::a_mod_blocks_menu_of_a_vanilla_type_gets_the_types_screen_on_both_sides",
        "SUNRISE_SCREEN_POSITION_CRATE_CHILD",
        &["crate"],
    ) {
        return;
    }
    let seen = Seen::default();
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "test",
        Box::new(CrateMod(seen.clone())),
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
                let pos = (17, 63, -9);
                game.world.chunks.insert(
                    (1, -1),
                    Arc::new(sunrise_client::chunk::Chunk::empty(1, -1)),
                );
                let state = game
                    .pack
                    .assets
                    .registry
                    .default_state("test:crate")
                    .unwrap();
                game.world.set(pos.0, pos.1, pos.2, state).unwrap();
                game.open_native_screen(pos, Some(pos), 1, "minecraft:generic_9x3", None);
                let open = game.mod_screen.as_ref().expect("the menu type's screen");
                assert_eq!(open.block.as_deref(), Some("test:crate"));
                assert_eq!(open.screen.height, 166.0);
                game.screen = Screen::Inventory;
                game.container = Some(pos);
                game.title = None;
                game.ui.hide_gui = true;
                game.settings.gui_scale = 2;
                game.level.recipe_book.open = [false; 4];
                game.world
                    .block_entities
                    .insert(pos, BlockEntity::Chest(vec![None; 27]));
                let screen = &mut game.mod_screen.as_mut().unwrap().screen;
                screen.width = 176.0;
                screen.inventory_x = 8.0;
                screen.inventory_y = 84.0;
                // No new motion: the odd-sized resize allows GUI scale 2.
                let small = [638, 478];
                let projection = game.pointer_projection(small);
                let sample = projection
                    .window
                    .physical_sample(PhysicalPosition::new(321.75, 231.75));
                game.set_window_cursor(sample, projection);
                game.hud(small);
                game.hud([641, 481]);

                // A cursor supplied in GUI pixels directly still truncates toward zero.
                game.window_cursor = None;
                game.cursor = (-3.75, 129.75);
                game.hud([641, 481]);
                {
                    let seen = seen.lock().unwrap();
                    let block = Some("test:crate".to_owned());
                    assert_eq!(seen.draw, vec![(block, Some([17, 63, -9])); 3]);
                    assert_eq!(
                        seen.frames,
                        [
                            Frame {
                                pointer: [321, 231],
                                size: [638, 478],
                                origin: [231.0, 156.0],
                            },
                            Frame {
                                pointer: [161, 116],
                                size: [321, 241],
                                origin: [72.0, 37.0],
                            },
                            Frame {
                                pointer: [-3, 129],
                                size: [321, 241],
                                origin: [72.0, 37.0],
                            },
                        ]
                    );
                    assert_eq!(
                        seen.tooltip_at,
                        [[90.0, 75.0], [89.0, 79.0], [-75.75, 92.75]]
                    );
                }
                // The server's menu for the block: asked with the block, so slot 4 is not in it.
                assert_eq!(
                    game.native_hidden_slots("minecraft:generic_9x3", Some(pos), 27),
                    [4]
                );
            },
        );
    });
}
/// #1172: a mod menu type opened without a block (an item's use, an entity, a command) gets
/// its type's screen, and a vanilla type gets a mod's screen only for the mod block that opens it.
#[test]
#[ignore = "daily"]
fn a_mod_menu_opened_without_a_block_gets_its_types_screen() {
    sunrise_client::containers::with_mod_menu_types(
        [(
            PACK.into(),
            "test".into(),
            9,
            0,
            Vec::new(),
            Vec::new(),
            None,
            None,
            true,
            None,
        )],
        || {
            with_listener(|game, seen| {
                let key = (i32::MIN, 0, 0);
                game.open_native_screen(key, None, 1, PACK, None);
                let open = game.mod_screen.as_ref().expect("the menu type's screen");
                assert_eq!((open.screen.height, &open.block), (133.0, &None));
                game.screen = Screen::Inventory;
                game.container = Some(key);
                let mut batch = HudBatch::scratch([128, 128]);
                assert!(game.draw_native_screen(&mut batch, 0.0, 0.0));
                assert_eq!(seen.lock().unwrap().draw, [(None, None)]);
                game.open_native_screen(key, None, 2, "minecraft:generic_9x3", None);
                assert!(
                    game.mod_screen.is_none(),
                    "a vanilla chest keeps its screen"
                );
            });
        },
    );
}

/// What the hopper screens' parts heard, as (the part's mod, the hook).
type Heard = Arc<Mutex<Vec<(&'static str, &'static str)>>>;
/// Mod `owner`'s screen on vanilla's `minecraft:hopper` for its machine: it hides slot
/// `HIDDEN`, its button is `HIDDEN` and its tooltip line names it. Each mod's is its own
/// kind, as two mods' parts are.
#[derive(Clone)]
struct HopperScreen<const HIDDEN: usize> {
    owner: &'static str,
    heard: Heard,
}
#[mod_api::sunrise_part]
impl<const HIDDEN: usize> mod_api::MenuPart for HopperScreen<HIDDEN> {
    fn screen(
        &self,
        _: &dyn mod_api::Host,
        _: &mut mod_api::ScreenEvent<'_>,
    ) -> Option<mod_api::Screen> {
        self.heard.lock().unwrap().push((self.owner, "screen"));
        Some(mod_api::Screen {
            slots: (0..5)
                .map(|i| (i != HIDDEN).then(mod_api::ScreenSlot::default))
                .collect(),
            own_labels: true,
            own_background: true,
            ..Default::default()
        })
    }
    fn screen_draw(&self, _: &dyn mod_api::ClientHost, _: &mut mod_api::ScreenDrawEvent<'_>) {
        self.heard.lock().unwrap().push((self.owner, "draw"));
    }
    fn screen_click(
        &self,
        _: &dyn mod_api::ClientHost,
        _: &mut mod_api::ScreenClickEvent<'_>,
    ) -> Option<i32> {
        self.heard.lock().unwrap().push((self.owner, "click"));
        Some(HIDDEN as i32)
    }
    fn screen_tooltip(
        &self,
        _: &dyn mod_api::ClientHost,
        event: &mut mod_api::ScreenTooltipEvent<'_>,
    ) {
        self.heard.lock().unwrap().push((self.owner, "tooltip"));
        event.lines.push((mod_api::Text::literal(self.owner), 0));
    }
}
/// Mod `owner`: its block `<owner>:machine`, whose container opens `minecraft:hopper`,
/// with its `HopperScreen` on that type.
struct HopperMod<const HIDDEN: usize>(HopperScreen<HIDDEN>);
#[mod_api::__subscriptions]
impl<const HIDDEN: usize> mod_api::Hooks for HopperMod<HIDDEN> {
    fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
        let block = r.block_key(&format!("{}:machine", self.0.owner));
        r.block_container(
            &block,
            mod_api::ModContainer {
                slots: 5,
                menu: Some("minecraft:hopper".into()),
                ..Default::default()
            },
        );
        r.menu_part("minecraft:hopper", self.0.clone());
    }
    fn blocks(&self, _: &mod_api::RegistrationContext, event: &mut mod_api::BlocksEvent<'_>) {
        let id = format!("{}:machine", self.0.owner);
        event
            .register(mod_api::Block::new(&id, "minecraft:barrel").with_item(false))
            .unwrap();
    }
}
/// Two mods' machines opening vanilla's hopper menu each get only their own mod's screen:
/// its layout on both sides, its drawing, its button, its tooltip and its slot controls. A
/// vanilla hopper keeps vanilla's screen.
#[test]
#[ignore = "daily"]
fn two_mods_sharing_a_vanilla_menu_type_keep_their_screens_apart() {
    // Content registries are process-global.
    if crate::game::native_entity_refs::tests::run_fresh_cases(
        "game::native_hooks::screen_position_tests::two_mods_sharing_a_vanilla_menu_type_keep_their_screens_apart",
        "SUNRISE_SCREEN_POSITION_HOPPER_CHILD",
        &["hoppers"],
    ) {
        return;
    }
    let heard = Heard::default();
    let alpha = HopperScreen::<0> {
        owner: "alpha",
        heard: heard.clone(),
    };
    let beta = HopperScreen::<4> {
        owner: "beta",
        heard: heard.clone(),
    };
    let mods: mod_api::Mods = vec![
        mod_api::ModEntry::new("alpha", Box::new(HopperMod(alpha))),
        mod_api::ModEntry::new("beta", Box::new(HopperMod(beta))),
    ]
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
                game.world
                    .chunks
                    .insert((0, 0), Arc::new(sunrise_client::chunk::Chunk::empty(0, 0)));
                for (x, block, hidden, owner) in [
                    (1, "alpha:machine", 0, "alpha"),
                    (2, "beta:machine", 4, "beta"),
                ] {
                    let pos = (x, 64, 0);
                    let state = game.pack.assets.registry.default_state(block).unwrap();
                    game.world.set(pos.0, pos.1, pos.2, state).unwrap();
                    heard.lock().unwrap().clear();
                    game.open_native_screen(pos, Some(pos), x, "minecraft:hopper", None);
                    game.screen = Screen::Inventory;
                    game.container = Some(pos);
                    let open = game.mod_screen.as_ref().expect("the machine's screen");
                    let (menu, block) = (open.menu.clone(), open.block.clone().unwrap());
                    let shown: Vec<_> = (0..5).filter(|&i| i != hidden).collect();
                    let ids: Vec<_> = shown.iter().map(|i| 300 + i).collect();
                    assert_eq!(game.native_screen_slots(), Some(ids));
                    let block = Some(block.as_str());
                    let mut batch = HudBatch::scratch([8, 8]);
                    let mut draw = crate::game::host::draw::GameDrawContext::new(game, &mut batch);
                    game.native_screen_draw(mod_api::__private::ScreenDrawCall {
                        menu: &menu,
                        block,
                        pos: None,
                        data: &[],
                        draw: &mut draw,
                        origin: [0.0; 2],
                        pointer: [0; 2],
                        gui_size: [8; 2],
                    });
                    let mut button = None;
                    game.native_screen_click(&menu, block, &[], [0.0; 2], false, &mut button);
                    assert_eq!(button, Some(hidden as i32));
                    let mut lines = Vec::new();
                    game.native_screen_tooltip(mod_api::__private::ScreenTooltipCall {
                        menu: &menu,
                        block,
                        data: &[],
                        at: [0.0; 2],
                        slot: None,
                        origin: [0.0; 2],
                        pointer: [0; 2],
                        gui_size: [8; 2],
                        lines: &mut lines,
                    });
                    assert_eq!(lines, [(mod_api::Text::literal(owner), 0)]);
                    let hooks = ["screen", "draw", "click", "tooltip"];
                    let only_owner: Vec<_> = hooks.iter().map(|&hook| (owner, hook)).collect();
                    assert_eq!(*heard.lock().unwrap(), only_owner);
                    // The server leaves out the owner's hidden slot, and only it.
                    assert_eq!(game.native_hidden_slots(&menu, Some(pos), 5), [hidden]);
                    // Only the owner reads and moves the screen's slots.
                    for calling in ["alpha", "beta"] {
                        let _call = mod_api::content::crash::enter_native_mod(
                            mod_api::content::crash::ModCall {
                                id: calling,
                                hook: "screen_draw",
                                part: None,
                            },
                        );
                        let owns = calling == owner;
                        assert_eq!(game.screen_slot(&menu, shown[0]).is_some(), owns);
                        let moved = game.set_screen_slot(&menu, shown[0], Some([1.0, 1.0]));
                        assert_eq!(moved.is_ok(), owns);
                    }
                }
                // A vanilla hopper asks neither mod.
                let pos = (3, 64, 0);
                let state = game
                    .pack
                    .assets
                    .registry
                    .default_state("minecraft:hopper")
                    .unwrap();
                game.world.set(pos.0, pos.1, pos.2, state).unwrap();
                heard.lock().unwrap().clear();
                game.open_native_screen(pos, Some(pos), 3, "minecraft:hopper", None);
                assert!(
                    game.mod_screen.is_none(),
                    "a vanilla hopper keeps its screen"
                );
                assert!(
                    game.native_hidden_slots("minecraft:hopper", Some(pos), 5)
                        .is_empty()
                );
                assert!(heard.lock().unwrap().is_empty());
            },
        );
    });
}
