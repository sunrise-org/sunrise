//! `menu_clicked` and `menu_quick_move` through real listeners: both are server hooks on the menus
//! the game runs for a guest, and each now runs as one decided action (`decided_action`): the
//! hook is fired inside an action savepoint, its answer is validated once and applied once, and a
//! refusal (stale, invalid, a failing listener) changes nothing and undoes every write the
//! listener made. The fixtures need prepared resources.
use crate::game::native_guest_turn_tests::{register_test_menu, with_guests};
use mod_api::{
    BlockEntityPart, Flow, Hooks, ITEMS, ItemPlace, ItemResource, MenuButtonEvent,
    MenuClickedEvent, MenuQuickMoveEvent, MenuSlotTakeEvent, ModEntry, Mods, Registrar, ServerHost,
};
use std::{
    cell::{Cell, RefCell},
    sync::Arc,
};
use sunrise_client::{
    containers::{BlockEntity, ModBlockEntity, ModBlockEntityFields, ModContainer, ModMenuType},
    items::ItemStack,
    lan::menu::Slots,
};
use sunrise_protocol::packets::play::{ContainerButtonClick, ContainerClick, Serverbound};

#[derive(Clone, Default)]
struct Script {
    /// Put dirt in guest slot 20 through the `Host`, in the listener's own nested transaction.
    write_dirt: bool,
    panic: bool,
    /// `menu_quick_move`: take 9 of the stack into slots 1 and 2 of the block (4, then 5).
    take: bool,
    all: bool,
    /// ...and answer with a stack past the item's maximum in slot 1.
    invalid: bool,
    /// `menu_quick_move`: move 4 of the stack into the second block through its `ITEMS`
    /// capability, leave 6 in `stack`, and return unhandled unless `handle_cross` is set.
    cross: bool,
    handle_cross: bool,
    fail_take: bool,
    /// `menu_button`: replace the open menu through the host, returning its cursor to inventory.
    replace_menu: bool,
    /// `menu_button`: propose a cursor edit before replacing the menu, to ensure stale edits lose.
    change_cursor: bool,
    /// `menu_button`: consume the player's carried stack.
    clear_cursor: bool,
    clicked: Option<Flow>,
    write_other_block: bool,
    write_clicked_block: bool,
    continue_after_player_write: bool,
    replace_display: bool,
}

thread_local! {
    static SCRIPT: RefCell<Script> = RefCell::new(Script::default());
    static DISPATCHES: Cell<usize> = const { Cell::new(0) };
    static TAKE: Cell<Option<(u32, u32)>> = const { Cell::new(None) };
    /// `menu_button` calls on the depot.
    static BUTTONS: Cell<usize> = const { Cell::new(0) };
    /// `menu_button` calls that received the depot's actual backing entity.
    static BUTTON_ENTITIES: Cell<usize> = const { Cell::new(0) };
    /// Optional positions delivered to menu-button listeners.
    static BUTTON_POSITIONS: RefCell<Vec<Option<[i32; 3]>>> = const { RefCell::new(Vec::new()) };
    /// Whether a transaction was open at each dispatch: the action's savepoint.
    static IN_ACTION: RefCell<Vec<bool>> = const { RefCell::new(Vec::new()) };
}

fn script(script: Script) {
    DISPATCHES.set(0);
    TAKE.set(None);
    super::decided_action::DECIDED.set(0);
    IN_ACTION.with(|a| a.borrow_mut().clear());
    SCRIPT.with(|s| *s.borrow_mut() = script);
}

fn write_dirt(host: &mut dyn ServerHost) {
    let key = mod_api::__private::item_key("minecraft:dirt").unwrap();
    let dirt = mod_api::ItemStack::new(key, 1).unwrap();
    let place = ItemPlace::PlayerSlot {
        player: 12,
        slot: 20,
    };
    let tx = host.transaction_current().expect("inside the action");
    assert_eq!(place.insert(host, &tx, &ItemResource::of(&dirt), 2), 2);
}

struct Part;
#[mod_api::sunrise_part]
impl mod_api::BlockPart for Part {}
#[mod_api::sunrise_part]
impl BlockEntityPart for Part {
    fn menu_quick_move(
        &self,
        host: &mut dyn ServerHost,
        event: &mut MenuQuickMoveEvent<'_>,
    ) -> Flow {
        DISPATCHES.set(DISPATCHES.get() + 1);
        IN_ACTION.with(|a| a.borrow_mut().push(host.transaction_current().is_some()));
        let s = SCRIPT.with(|s| s.borrow().clone());
        let stone = |count| {
            mod_api::ItemStack::new(
                mod_api::__private::item_key("minecraft:stone").unwrap(),
                count,
            )
            .unwrap()
        };
        if s.write_dirt {
            write_dirt(host);
        }
        if s.take {
            event.entity.items.resize(9, None);
            event.entity.items[1] = Some(stone(if s.invalid { 65 } else { 4 }));
            event.entity.items[2] = Some(stone(if s.all { 6 } else { 5 }));
            if s.all {
                *event.stack = None;
            } else {
                event.stack.as_mut().unwrap().set_count(1).unwrap();
            }
        }
        if s.cross {
            let level = host.level("minecraft:overworld").expect("the level");
            let items = level
                .capability(host, &ITEMS, [2, 64, 1], None)
                .expect("the second block's items");
            let tx = host.transaction_current().expect("inside the action");
            assert_eq!(
                items.insert_at(host, &tx, 0, &ItemResource::of(&stone(1)), 4),
                4
            );
            event.stack.as_mut().unwrap().set_count(6).unwrap();
        }
        if s.panic {
            panic!("a listener failed");
        }
        if s.take || s.handle_cross {
            return Flow::Cancel;
        }
        Flow::Continue
    }

    fn menu_slot_take(&self, host: &mut dyn ServerHost, event: &mut MenuSlotTakeEvent<'_>) {
        if !SCRIPT.with(|s| s.borrow().handle_cross) {
            return;
        }
        TAKE.set(Some((
            event.taken.count(),
            event.left.map_or(0, |left| left.count()),
        )));
        if SCRIPT.with(|s| s.borrow().fail_take) {
            write_dirt(host);
            panic!("the take listener failed after the cross-block move");
        }
    }
}
#[mod_api::sunrise_part]
impl mod_api::MenuPart for Part {
    fn menu_clicked(&self, host: &mut dyn ServerHost, event: &mut MenuClickedEvent<'_>) -> Flow {
        let s = SCRIPT.with(|s| s.borrow().clone());
        let Some(flow) = s.clicked else {
            return Flow::Continue;
        };
        assert_eq!(event.menu, DEPOT_MENU);
        assert!(
            host.menu(event.uuid, event.window).is_none(),
            "not a native window"
        );
        let player = host.player(event.uuid).unwrap();
        assert!(player.container_menu(host).is_some());
        let tx = host.transaction_current().unwrap();
        if s.write_other_block || s.write_clicked_block {
            let level = host.level("minecraft:overworld").unwrap();
            let (pos, id) = if s.write_clicked_block {
                ([1, 64, 1], "minecraft:stone")
            } else {
                ([2, 64, 1], "minecraft:dirt")
            };
            let stack =
                mod_api::ItemStack::new(mod_api::__private::item_key(id).unwrap(), 1).unwrap();
            let items = level.capability(host, &ITEMS, pos, None).unwrap();
            assert_eq!(
                items.insert_at(host, &tx, 0, &ItemResource::of(&stack), 2),
                2
            );
        }
        flow
    }

    fn menu_button_valid(
        &self,
        _: &dyn mod_api::Host,
        event: &mut mod_api::MenuButtonValidEvent<'_>,
    ) -> Flow {
        if matches!(event.id, 3 | 4) {
            Flow::Cancel
        } else {
            Flow::Continue
        }
    }

    fn menu_button(&self, host: &mut dyn ServerHost, event: &mut MenuButtonEvent<'_>) {
        BUTTONS.set(BUTTONS.get() + 1);
        if event.entity.is_some() {
            BUTTON_ENTITIES.set(BUTTON_ENTITIES.get() + 1);
        }
        BUTTON_POSITIONS.with(|positions| positions.borrow_mut().push(event.context.pos));
        let script = SCRIPT.with(|script| script.borrow().clone());
        if script.clear_cursor {
            *event.carried = None;
        }
        if script.change_cursor {
            *event.carried = Some(
                mod_api::ItemStack::new(
                    mod_api::__private::item_key("minecraft:gold_ingot").unwrap(),
                    1,
                )
                .unwrap(),
            );
        }
        if script.replace_menu {
            host.open_menu(
                event.context.player,
                1,
                mod_api::Text::literal("opened by menu_button"),
                Vec::new(),
                mod_api::MenuReplace::InPlace,
            )
            .unwrap()
            .expect("the menu button's player is online");
        }
    }
}

struct Mod;
#[mod_api::__subscriptions]
impl Hooks for Mod {
    fn register(&self, r: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
        let kind = r.block_entity_type("depot").blocks(&[]).build();
        r.block_entity_part(&kind, Part).unwrap();
        r.menu_part(DEPOT_MENU, Part);
        r.menu_part(STATUS_MENU, Part);
    }
}

struct ClickMod;
#[mod_api::__subscriptions]
impl Hooks for ClickMod {
    fn register(&self, r: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
        r.menu_part("minecraft:generic_9x1", ClickPart);
    }
}

struct ClickPart;
#[mod_api::sunrise_part]
impl mod_api::MenuPart for ClickPart {
    fn menu_clicked(
        &self,
        host: &mut dyn ServerHost,
        event: &mut MenuClickedEvent<'_>,
    ) -> mod_api::Flow {
        DISPATCHES.set(DISPATCHES.get() + 1);
        IN_ACTION.with(|a| a.borrow_mut().push(host.transaction_current().is_some()));
        assert_eq!(event.menu, "minecraft:generic_9x1");
        assert_eq!(event.uuid, 12);
        let s = SCRIPT.with(|s| s.borrow().clone());
        if s.write_dirt {
            write_dirt(host);
        }
        if s.replace_display {
            let item =
                mod_api::ItemStack::new(mod_api::__private::item_key("minecraft:dirt").unwrap(), 1)
                    .unwrap();
            host.menu_set_items(event.uuid, event.window, vec![Some(item)])
                .unwrap();
        }
        if s.panic {
            panic!("a listener failed");
        }
        if s.replace_display || (s.write_dirt && !s.continue_after_player_write) {
            Flow::Cancel
        } else {
            Flow::Continue
        }
    }
}

fn with_mod(test: impl FnOnce()) {
    let mods: Mods = vec![ModEntry::new("movemod", Box::new(Mod))].into();
    mod_platform::testing::with_mods(mods.clone(), || {
        crate::game::api_values::install_parts(&mods).unwrap();
        test();
    });
}

/// How many decisions were entered since the last `script`.
fn decided() -> usize {
    super::decided_action::DECIDED.get()
}

fn count(stack: &Option<ItemStack>) -> u32 {
    stack.as_ref().map_or(0, |s| s.count)
}

fn inventory(game: &crate::game::Game<'_>, slot: usize) -> u32 {
    count(&game.lan.as_ref().unwrap().hub.guests[0].inventory[slot])
}

#[test]
#[ignore = "daily"]
fn a_click_on_a_mod_menu_tells_the_mods_inside_an_action_and_a_failure_undoes_it() {
    let mods: Mods = vec![ModEntry::new("clickmod", Box::new(ClickMod))].into();
    mod_platform::testing::with_mods(mods.clone(), || {
        crate::game::api_values::install_parts(&mods).unwrap();
        with_guests(|game| {
            let window = open_native_menu(game);
            let click = |game: &mut crate::game::Game<'_>| {
                game.lan_guest_packet(12, container_click(window, 3, 0));
                game.run_native_menu_events();
            };
            // Row 72: the mods hear the click once, inside the action, and what the listener
            // wrote through the `Host` stays when it finishes.
            script(Script {
                write_dirt: true,
                ..Script::default()
            });
            click(game);
            assert_eq!(DISPATCHES.get(), 1);
            assert_eq!(decided(), 1);
            assert_eq!(IN_ACTION.with(|a| a.borrow().clone()), [true]);
            assert_eq!(inventory(game, 20), 2);
            assert!(game.transactions.is_empty());
            // Rows 79 and 84: a write and then a panic: the write is undone, the click is not
            // heard again, and the next click works.
            game.lan.as_mut().unwrap().hub.guests.update(0, |g| {
                g.inventory[20] = None;
            });
            script(Script {
                write_dirt: true,
                panic: true,
                ..Script::default()
            });
            click(game);
            assert_eq!(DISPATCHES.get(), 1);
            assert_eq!(inventory(game, 20), 0);
            assert!(game.transactions.is_empty());
            script(Script::default());
            click(game);
            assert_eq!(DISPATCHES.get(), 1);
            script(Script {
                write_dirt: true,
                continue_after_player_write: true,
                ..Script::default()
            });
            click(game);
            assert_eq!(DISPATCHES.get(), 1);
            assert_eq!(
                inventory(game, 20),
                0,
                "Continue cannot change its read set"
            );
            assert!(game.transactions.is_empty());
            // Row 90: heard inside a transaction that then aborts: the write is undone.
            script(Script {
                write_dirt: true,
                ..Script::default()
            });
            let seal = game.transaction_open();
            click(game);
            assert_eq!(inventory(game, 20), 2);
            game.transaction_close(&seal, false);
            assert_eq!(inventory(game, 20), 0);
        });
    });
}

#[test]
#[ignore = "daily"]
fn without_a_listener_a_click_on_a_mod_menu_is_heard_plainly() {
    // Row 92: no mod at all: the plain call, no transaction and no journaled write.
    with_guests(|game| {
        let window = open_native_menu(game);
        script(Script::default());
        let writes = game.transactions.writes();
        game.lan_guest_packet(12, container_click(window, 3, 0));
        game.run_native_menu_events();
        assert_eq!(DISPATCHES.get(), 0);
        // The ordinary path: the decision is never entered.
        assert_eq!(decided(), 0);
        assert_eq!(game.transactions.writes(), writes);
        assert!(game.transactions.is_empty());
    });
}

fn container_click(window: i32, slot: i16, input: i32) -> Serverbound {
    Serverbound::ContainerClick(ContainerClick {
        container: window,
        state: 0,
        slot,
        button: 0,
        input,
        changed: Vec::new(),
        carried: None,
    })
}

/// A menu a mod opened (`Slots::Native`) for guest 12.
fn open_native_menu(game: &mut crate::game::Game<'_>) -> i32 {
    use sunrise_client::lan::menu::Menu;
    let menu = Menu {
        window: 5,
        kind: 2,
        pos: None,
        slots: Slots::Native,
        size: 9,
        state: 0,
        sent: Vec::new(),
        drag: None,
        sent_data: Vec::new(),
        blockless_data: Vec::new(),
        hidden: Vec::new(),
        mod_type: None,
        registry_id: "minecraft:generic_9x1".into(),
        craft: None,
        links: Vec::new(),
        asks_still_valid: false,
        opening_item: None,
    };
    game.lan
        .as_mut()
        .unwrap()
        .hub
        .guests
        .update(0, |guest| guest.menu = Some(menu));
    5
}

const POS: (i32, i32, i32) = (1, 64, 1);

/// A mod container (a block entity of the mod's registered type) with stone 10 in slot 0, open
/// as a 9-slot menu for guest 12.
fn open_depot(game: &mut crate::game::Game<'_>) -> i32 {
    game.world.chunks.insert(
        (0, 0),
        std::sync::Arc::new(sunrise_client::chunk::Chunk::empty(0, 0)),
    );
    let mut fields = ModBlockEntityFields {
        items: vec![None; 9],
        ..Default::default()
    };
    fields.items[0] = Some(ItemStack::new("minecraft:stone", 10));
    game.world.block_entities.insert(
        POS,
        BlockEntity::Mod(Box::new(ModBlockEntity::new(
            "movemod:depot".into(),
            fields,
        ))),
    );
    let host = game.lan.as_mut().unwrap();
    host.hub.open_container(
        12,
        "minecraft:generic_9x1",
        sunrise_core::nbt::Tag::String("test".into()),
        POS,
        Slots::Block(POS),
        vec![None; 9],
        None,
    );
    host.hub.guests[0].menu.as_ref().unwrap().window
}

const SECOND: (i32, i32, i32) = (2, 64, 1);

/// A second mod container beside the depot, empty, that a listener can move items into through
/// its `ITEMS` capability.
fn second_block(game: &mut crate::game::Game<'_>) {
    let state = game
        .pack
        .assets
        .registry
        .default_state("minecraft:stone")
        .unwrap();
    game.world.set(SECOND.0, SECOND.1, SECOND.2, state).unwrap();
    game.world.block_entities.insert(
        SECOND,
        BlockEntity::Mod(Box::new(ModBlockEntity::new(
            "movemod:depot".into(),
            ModBlockEntityFields {
                items: vec![None; 9],
                ..Default::default()
            },
        ))),
    );
    game.world.block_entities.set_mod_containers(
        [(
            state.0,
            Arc::new(ModContainer {
                slots: 9,
                inputs: vec![0],
                ..Default::default()
            }),
        )]
        .into_iter()
        .collect(),
    );
}

/// Every stone in the depot, the second block and guest 12's inventory: where a move left them.
fn stone_everywhere(game: &crate::game::Game<'_>) -> (u32, u32, u32) {
    let held = |items: &[Option<ItemStack>]| {
        items
            .iter()
            .flatten()
            .filter(|s| &*s.item == "minecraft:stone")
            .map(|s| s.count)
            .sum::<u32>()
    };
    let block = |pos| {
        let Some(BlockEntity::Mod(m)) = game.world.block_entities.get(&pos) else {
            panic!("the block entity is gone");
        };
        held(&m.items)
    };
    (
        block(POS),
        block(SECOND),
        held(&game.lan.as_ref().unwrap().hub.guests[0].inventory),
    )
}

/// The block's slots 0, 1 and 2, and guest 12's slot 20.
fn depot(game: &crate::game::Game<'_>) -> (u32, u32, u32, u32) {
    let Some(BlockEntity::Mod(m)) = game.world.block_entities.get(&POS) else {
        panic!("the block entity is gone");
    };
    let slot = |k: usize| count(m.items.get(k).unwrap_or(&None));
    (slot(0), slot(1), slot(2), inventory(game, 20))
}

#[test]
#[ignore = "daily"]
fn a_mod_quick_move_is_one_action_and_a_refusal_leaves_no_prefix() {
    with_mod(|| {
        with_guests(|game| {
            open_depot(game);
            let click = |game: &mut crate::game::Game<'_>| {
                let window = game.lan.as_ref().unwrap().hub.guests[0]
                    .menu
                    .as_ref()
                    .unwrap()
                    .window;
                game.lan_guest_packet(12, container_click(window, 0, 1));
            };
            // Row 72: the mod takes 9 of the 10 into two slots: the move applies once, inside the
            // action, and the plain routing is skipped.
            script(Script {
                take: true,
                ..Script::default()
            });
            click(game);
            assert_eq!(DISPATCHES.get(), 1);
            assert_eq!(IN_ACTION.with(|a| a.borrow().clone()), [true]);
            assert_eq!(depot(game), (1, 4, 5, 0));
            assert!(game.transactions.is_empty());
            // A complete move empties the source through the optional API remainder.
            open_depot(game);
            script(Script {
                take: true,
                all: true,
                ..Script::default()
            });
            click(game);
            assert_eq!(depot(game), (0, 4, 6, 0));
            // Row 77: the same move, and the listener also writes the guest's inventory: stale,
            // so the whole move is refused: the source keeps its 10 and neither target slot is
            // left holding a prefix of the move.
            open_depot(game);
            script(Script {
                take: true,
                write_dirt: true,
                ..Script::default()
            });
            click(game);
            assert_eq!(DISPATCHES.get(), 1);
            assert_eq!(depot(game), (10, 0, 0, 0));
            // Row 89: an invalid answer (a stack past the maximum) refuses the whole move too.
            open_depot(game);
            script(Script {
                take: true,
                invalid: true,
                ..Script::default()
            });
            click(game);
            assert_eq!(depot(game), (10, 0, 0, 0));
            // Rows 79 and 84: a panic after writing: the block entity is put back as it was
            // (it is out of the world while the mods are asked), the write is undone, and the next
            // click works.
            open_depot(game);
            script(Script {
                take: true,
                write_dirt: true,
                panic: true,
                ..Script::default()
            });
            click(game);
            assert_eq!(depot(game), (10, 0, 0, 0));
            assert!(game.transactions.is_empty());
            // The panic alone, with no `Host` write (which would make the move stale by itself):
            // the move the listener answered is still refused and the block entity is put back.
            open_depot(game);
            script(Script {
                take: true,
                panic: true,
                ..Script::default()
            });
            click(game);
            assert_eq!(DISPATCHES.get(), 1);
            assert_eq!(depot(game), (10, 0, 0, 0));
            assert!(game.transactions.is_empty());
            open_depot(game);
            script(Script {
                take: true,
                ..Script::default()
            });
            click(game);
            assert_eq!(depot(game), (1, 4, 5, 0));
            // Row 90: applied inside a transaction that then aborts: the move is undone.
            open_depot(game);
            let seal = game.transaction_open();
            click(game);
            assert_eq!(depot(game), (1, 4, 5, 0));
            game.transaction_close(&seal, false);
            assert_eq!(depot(game), (10, 0, 0, 0));
        });
    });
}

#[test]
#[ignore = "daily"]
fn a_mod_quick_move_no_mod_took_is_left_to_the_games_routing() {
    let run = || {
        let mut result = None;
        with_guests(|game| {
            let window = open_depot(game);
            script(Script::default());
            game.lan_guest_packet(12, container_click(window, 0, 1));
            assert!(game.transactions.is_empty());
            result = Some(depot(game));
        });
        result.unwrap()
    };
    // Row 92: with no part for the hook, the plain click: no dispatch, and the result is the
    // game's own routing.
    let plain = run();
    assert_eq!(DISPATCHES.get(), 0);
    // The ordinary path: the decision is never entered.
    assert_eq!(decided(), 0);
    // A listener that only looks is asked once, in the action, and leaves the click to the same
    // routing: the same result.
    with_mod(|| {
        assert_eq!(run(), plain);
        assert_eq!(DISPATCHES.get(), 1);
        assert_eq!(decided(), 1);
        assert_eq!(IN_ACTION.with(|a| a.borrow().clone()), [true]);
    });
}

#[test]
#[ignore = "daily"]
fn an_untaken_quick_move_that_wrote_to_another_block_is_undone_and_routed_once() {
    with_mod(|| {
        with_guests(|game| {
            let window = open_depot(game);
            second_block(game);
            // The listener moves 4 of the 10 into the second block, leaves 6 in `stack` and
            // returns unhandled. The action is cancelled: the second block is empty again, and
            // the game's own routing then moves the original 10 exactly once, so no stone exists
            // twice and none is lost.
            script(Script {
                cross: true,
                ..Script::default()
            });
            game.lan_guest_packet(12, container_click(window, 0, 1));
            assert_eq!(DISPATCHES.get(), 1);
            assert_eq!(IN_ACTION.with(|a| a.borrow().clone()), [true]);
            let (depot, second, inventory) = stone_everywhere(game);
            assert_eq!((second, depot + inventory), (0, 10));
            assert!(game.transactions.is_empty());
            // The same click inside a transaction that aborts afterwards: the depot's 10 are
            // back in it, the second block is empty, and the inventory holds what the first
            // click routed there.
            let window = open_depot(game);
            let seal = game.transaction_open();
            game.lan_guest_packet(12, container_click(window, 0, 1));
            game.transaction_close(&seal, false);
            assert_eq!(stone_everywhere(game), (10, 0, inventory));
        });
    });
}

#[test]
#[ignore = "daily"]
fn a_handled_cross_block_move_commits_only_if_its_take_returns() {
    with_mod(|| {
        with_guests(|game| {
            let kind = Box::leak(Box::new(ModMenuType {
                id: "minecraft:generic_9x1",
                owner: "minecraft",
                slots: 9,
                player_inventory: true,
                data_slots: 0,
                owned_slots: None,
                results: Vec::new(),
                inputs: Vec::new(),
                crafting: None,
                links: None,
            }));
            for fail_take in [true, false] {
                let window = open_depot(game);
                second_block(game);
                game.lan
                    .as_mut()
                    .unwrap()
                    .hub
                    .guests
                    .update(0, |g| g.menu.as_mut().unwrap().mod_type = Some(kind));
                script(Script {
                    cross: true,
                    handle_cross: true,
                    fail_take,
                    ..Script::default()
                });
                game.lan_guest_packet(12, container_click(window, 0, 1));
                assert_eq!(DISPATCHES.get(), 1);
                assert_eq!(TAKE.get(), Some((4, 6)));
                assert_eq!(
                    stone_everywhere(game),
                    if fail_take { (10, 0, 0) } else { (6, 4, 0) }
                );
                assert_eq!(inventory(game, 20), 0);
                assert!(game.transactions.is_empty());
            }
        })
    });
}

#[test]
#[ignore = "daily"]
fn a_replaced_block_entity_makes_an_admitted_quick_move_stale() {
    use super::native_quick_move::Admitted;
    with_guests(|game| {
        let window = open_depot(game);
        let admitted = Admitted::of(game, 12, POS, 9, window).expect("the menu is open");
        assert!(admitted.holds(game));
        // The same values written again keep the block entity's identity: still fresh.
        let same = game.world.block_entities.get(&POS).unwrap().clone();
        game.world.block_entities.insert(POS, same.clone());
        assert!(admitted.holds(game));
        // Another instance with equal values, which a listener's command could have placed: stale.
        game.world.block_entities.replace(POS, same);
        assert!(!admitted.holds(game));
        // A fresh admission on the replacement holds, and a closed window does not.
        let admitted = Admitted::of(game, 12, POS, 9, window).unwrap();
        assert!(admitted.holds(game));
        game.lan
            .as_mut()
            .unwrap()
            .hub
            .guests
            .update(0, |guest| guest.menu = None);
        assert!(!admitted.holds(game));
    });
}

/// A content mod's menu type, the one the depot's container opens (`depot_container`).
const DEPOT_MENU: &str = "movemod:depot_menu";
const STATUS_MENU: &str = "movemod:status_menu";

/// Makes the depot at `POS` a mod container whose menu is `DEPOT_MENU`, so `mod_menu_at(POS)`
/// names it.
fn depot_container(game: &mut crate::game::Game<'_>) {
    let state = game
        .pack
        .assets
        .registry
        .default_state("minecraft:stone")
        .unwrap();
    game.world.set(POS.0, POS.1, POS.2, state).unwrap();
    game.world.block_entities.set_mod_containers(
        [(
            state.0,
            Arc::new(ModContainer {
                slots: 9,
                menu: Some(DEPOT_MENU.into()),
                ..Default::default()
            }),
        )]
        .into_iter()
        .collect(),
    );
}

#[test]
#[ignore = "daily"]
fn a_quick_move_cannot_reach_a_player_slot_omitted_from_the_menu() {
    sunrise_client::containers::with_mod_menu_types(
        [(
            "minecraft:generic_9x1".into(),
            "movemod".into(),
            9,
            0,
            Vec::new(),
            Vec::new(),
            None,
            None,
            false,
            None,
        )],
        || {
            with_mod(|| {
                with_guests(|game| {
                    let window = open_depot(game);
                    game.lan.as_mut().unwrap().hub.guests.update(0, |guest| {
                        guest.inventory[9] = Some(ItemStack::new("minecraft:stone", 10));
                    });
                    script(Script {
                        take: true,
                        ..Script::default()
                    });
                    game.lan_guest_packet(12, container_click(window, 9, 1));
                    assert_eq!(DISPATCHES.get(), 0);
                    assert_eq!(inventory(game, 9), 10);
                })
            })
        },
    );
}

#[test]
#[ignore = "daily"]
fn a_code_opened_menus_button_reaches_a_blocks_menu_button_only_for_its_menu_type() {
    let menus = [
        (
            DEPOT_MENU.into(),
            "movemod".into(),
            9,
            0,
            Vec::new(),
            Vec::new(),
            None,
            None,
            true,
            None,
        ),
        (
            STATUS_MENU.into(),
            "movemod".into(),
            0,
            0,
            Vec::new(),
            Vec::new(),
            None,
            None,
            false,
            None,
        ),
    ];
    sunrise_client::containers::with_mod_menu_types(menus, || {
        with_mod(|| {
            with_guests(|game| {
                let button = |game: &mut crate::game::Game<'_>, window, id| {
                    game.lan_guest_packet(
                        12,
                        Serverbound::ContainerButtonClick(ContainerButtonClick {
                            container: window,
                            button: id,
                        }),
                    );
                };
                BUTTONS.set(0);
                BUTTON_ENTITIES.set(0);
                BUTTON_POSITIONS.with(|positions| positions.borrow_mut().clear());
                let window = open_depot(game);
                depot_container(game);
                // The prepared protocol registry has no fixture mod IDs. Use the generic
                // window's protocol slot but give the server menu its actual mod owner.
                game.lan.as_mut().unwrap().hub.guests.update(0, |guest| {
                    let menu = guest.menu.as_mut().unwrap();
                    menu.registry_id = DEPOT_MENU.into();
                    menu.mod_type = sunrise_client::containers::mod_menu_type(DEPOT_MENU);
                });
                button(game, window, 99);
                assert_eq!(BUTTONS.get(), 0, "a menu rejects unsupported button IDs");
                button(game, window + 1, 3);
                assert_eq!(BUTTONS.get(), 0, "a stale window has no button owner");
                button(game, window, 3);
                assert_eq!(
                    BUTTONS.get(),
                    1,
                    "the block's own menu reaches its menu_button"
                );
                assert_eq!(BUTTON_ENTITIES.get(), 1, "the block menu lends its entity");
                BUTTON_POSITIONS.with(|positions| {
                    assert_eq!(
                        positions.borrow().as_slice(),
                        &[Some([POS.0, POS.1, POS.2])]
                    );
                });
                game.lan.as_mut().unwrap().hub.guests.update(0, |guest| {
                    guest.carried = Some(ItemStack::new("minecraft:diamond", 2));
                });
                SCRIPT.with(|script| {
                    *script.borrow_mut() = Script {
                        clear_cursor: true,
                        ..Script::default()
                    }
                });
                button(game, window, 4);
                SCRIPT.with(|script| *script.borrow_mut() = Script::default());
                let guest = game.lan.as_ref().unwrap().hub.guest(12).unwrap();
                assert_eq!(
                    guest.carried, None,
                    "a button can consume the carried stack"
                );
                assert!(
                    !guest
                        .inventory
                        .iter()
                        .flatten()
                        .any(|stack| { &*stack.item == "minecraft:diamond" && stack.count == 2 })
                );
                assert_eq!(BUTTONS.get(), 2);
                assert_eq!(BUTTON_ENTITIES.get(), 2);
                // A slotless, code-opened menu routes by its live menu owner. A position hint
                // does not lend an entity; only Slots::Block does.
                game.lan.as_mut().unwrap().hub.guests.update(0, |guest| {
                    guest.brand = "sunrise".into();
                    guest.sender.menu_extra_rev = sunrise_protocol::menu_extra::LIVE_DATA_REVISION;
                });
                let player = game.player(12).unwrap();
                let key = register_test_menu(game, STATUS_MENU);
                let window = player
                    .open_mod_menu(game, {
                        let mut request =
                            mod_api::ModMenuOpen::new(key, mod_api::Text::literal("Status"));
                        request.pos = Some([POS.0, POS.1, POS.2]);
                        request.replace = mod_api::MenuReplace::InPlace;
                        request
                    })
                    .unwrap()
                    .unwrap();
                assert_eq!(
                    player.container_menu(game).unwrap().slot_count(game),
                    Some(0)
                );
                button(game, window, 3);
                assert_eq!(
                    BUTTONS.get(),
                    3,
                    "the menu type owns its button even without block backing"
                );
                assert_eq!(
                    BUTTON_ENTITIES.get(),
                    2,
                    "a code-opened menu's position alone does not lend the block entity"
                );
                BUTTON_POSITIONS.with(|positions| {
                    assert_eq!(
                        positions.borrow().as_slice(),
                        &[
                            Some([POS.0, POS.1, POS.2]),
                            Some([POS.0, POS.1, POS.2]),
                            None,
                        ],
                        "a blockless menu supplies no position even if it has a screen hint"
                    );
                });
                // Another menu type at that position (another mod's menu at a machine): not the
                // block's.
                game.lan.as_mut().unwrap().hub.guests.update(0, |guest| {
                    guest.menu.as_mut().unwrap().registry_id = "test:native".into();
                });
                button(game, window, 3);
                assert_eq!(
                    BUTTONS.get(),
                    3,
                    "another menu type reaches no block's menu_button"
                );
                game.lan.as_mut().unwrap().hub.guests.update(0, |guest| {
                    guest.menu.as_mut().unwrap().registry_id = STATUS_MENU.into();
                    guest.carried = Some(ItemStack::new("minecraft:diamond", 2));
                });
                SCRIPT.with(|script| {
                    *script.borrow_mut() = Script {
                        replace_menu: true,
                        change_cursor: true,
                        ..Script::default()
                    }
                });
                button(game, window, 4);
                SCRIPT.with(|script| *script.borrow_mut() = Script::default());
                assert_eq!(BUTTONS.get(), 4);
                assert_eq!(BUTTON_ENTITIES.get(), 2);
                let guest = game.lan.as_ref().unwrap().hub.guest(12).unwrap();
                assert_eq!(
                    guest.carried, None,
                    "a cursor edit from the old menu cannot overwrite the replacement"
                );
                assert!(
                    guest
                        .inventory
                        .iter()
                        .flatten()
                        .any(|stack| { &*stack.item == "minecraft:diamond" && stack.count == 2 }),
                    "the returned cursor stack stays in the player's inventory"
                );
                assert!(
                    !guest.inventory.iter().flatten().any(|stack| {
                        &*stack.item == "minecraft:gold_ingot" && stack.count == 1
                    }),
                    "the stale menu's explicit cursor edit is not applied"
                );
            });
        })
    });
}

#[test]
#[ignore = "daily"]
fn a_block_menu_click_commits_only_the_replacement_or_a_fresh_default() {
    use sunrise_client::lan::{Out, PlayerSender, menu::input::SWAP};
    use sunrise_protocol::{buf::Reader, packets::play::Clientbound};

    sunrise_client::containers::with_mod_menu_types(
        [(
            DEPOT_MENU.into(),
            "movemod".into(),
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
            with_mod(|| {
                with_guests(|game| {
                    let window = open_depot(game);
                    depot_container(game);
                    second_block(game);
                    game.lan.as_mut().unwrap().hub.guests.update(0, |g| {
                        let open = g.menu.as_mut().unwrap();
                        open.registry_id = DEPOT_MENU.into();
                        open.mod_type = sunrise_client::containers::mod_menu_type(DEPOT_MENU);
                    });
                    let content = |game: &crate::game::Game<'_>, pos, slot: usize| {
                        let Some(BlockEntity::Mod(entity)) = game.world.block_entities.get(&pos)
                        else {
                            panic!("missing {pos:?}")
                        };
                        count(&entity.items[slot])
                    };

                    script(Script {
                        clicked: Some(Flow::Cancel),
                        write_other_block: true,
                        ..Script::default()
                    });
                    game.lan_guest_packet(12, container_click(window, 0, 0));
                    assert_eq!(decided(), 1);
                    assert_eq!(content(game, POS, 0), 10);
                    assert_eq!(content(game, SECOND, 0), 2);
                    assert!(game.lan.as_ref().unwrap().hub.guests[0].carried.is_none());

                    // A visible player inventory still does not include the offhand.
                    let (tx, rx) = std::sync::mpsc::channel();
                    let offhand = Some(ItemStack::new("minecraft:iron_ingot", 1));
                    let hub = &mut game.lan.as_mut().unwrap().hub;
                    hub.guests.update(0, |g| {
                        g.sender = PlayerSender::new(tx, Arc::default());
                        g.offhand = offhand.clone();
                    });
                    let menu = hub.guests[0].menu.as_ref().unwrap();
                    assert_eq!(menu.player_slots(), 36);
                    let state = menu.state;
                    let expected = hub.stack(&offhand);
                    assert!(expected.is_some());
                    script(Script {
                        clicked: Some(Flow::Cancel),
                        ..Script::default()
                    });
                    game.lan_guest_packet(
                        12,
                        Serverbound::ContainerClick(ContainerClick {
                            container: window,
                            state,
                            slot: 0,
                            button: 40,
                            input: SWAP,
                            changed: Vec::new(),
                            carried: None,
                        }),
                    );
                    assert_eq!(decided(), 1, "the swap reached the click decision");
                    assert_eq!(content(game, POS, 0), 10);
                    assert_eq!(game.lan.as_ref().unwrap().hub.guests[0].offhand, offhand);
                    let corrections: Vec<_> = rx
                        .try_iter()
                        .flat_map(|out| match out {
                            Out::Packet(body) => vec![body],
                            Out::Packets(bodies) => bodies,
                            Out::Disconnect(_) => Vec::new(),
                        })
                        .filter_map(|body| {
                            let mut reader = Reader::new(&body);
                            let id = reader.varint().unwrap();
                            match Clientbound::decode(id, reader.rest()).unwrap() {
                                Clientbound::SetPlayerInventory(p) if p.slot == 40 => Some(p.item),
                                _ => None,
                            }
                        })
                        .collect();
                    assert_eq!(corrections, [expected]);

                    script(Script {
                        clicked: Some(Flow::Continue),
                        write_clicked_block: true,
                        ..Script::default()
                    });
                    game.lan_guest_packet(12, container_click(window, 0, 0));
                    assert_eq!(decided(), 1);
                    assert_eq!(
                        content(game, POS, 0),
                        10,
                        "the default's slot read set changed"
                    );

                    script(Script {
                        clicked: Some(Flow::Continue),
                        write_other_block: true,
                        take: true,
                        invalid: true,
                        ..Script::default()
                    });
                    game.lan_guest_packet(12, container_click(window, 0, 1));
                    assert_eq!(
                        decided(),
                        2,
                        "the rejected quick move ran under the click decision"
                    );
                    assert_eq!(content(game, POS, 0), 10);
                    assert_eq!(
                        content(game, SECOND, 0),
                        2,
                        "a refused default aborts the click listener's write"
                    );

                    script(Script {
                        clicked: Some(Flow::Continue),
                        write_other_block: true,
                        ..Script::default()
                    });
                    game.lan_guest_packet(12, container_click(window, 0, 0));
                    assert_eq!(content(game, POS, 0), 0);
                    assert_eq!(content(game, SECOND, 0), 4);
                    assert_eq!(count(&game.lan.as_ref().unwrap().hub.guests[0].carried), 10);
                })
            })
        },
    );
}

#[test]
#[ignore = "daily"]
fn a_listener_for_another_menu_type_does_not_admit_a_click_decision() {
    with_mod(|| {
        with_guests(|game| {
            let window = open_native_menu(game);
            script(Script::default());
            game.lan_guest_packet(12, container_click(window, 3, 0));
            assert_eq!(decided(), 0);
        })
    });
}

#[test]
#[ignore = "daily"]
fn a_display_replacement_is_journaled_along_with_the_click() {
    let mods: Mods = vec![ModEntry::new("clickmod", Box::new(ClickMod))].into();
    mod_platform::testing::with_mods(mods.clone(), || {
        crate::game::api_values::install_parts(&mods).unwrap();
        with_guests(|game| {
            let window = game
                .lan
                .as_mut()
                .unwrap()
                .hub
                .open_native_menu(
                    12,
                    1,
                    sunrise_core::nbt::Tag::String("display".into()),
                    vec![None; 9],
                )
                .unwrap();
            script(Script {
                replace_display: true,
                ..Script::default()
            });
            let seal = game.transaction_open();
            game.lan_guest_packet(12, container_click(window, 0, 0));
            assert_eq!(
                count(&game.lan.as_ref().unwrap().hub.guests[0].native_menu[0]),
                1
            );
            game.transaction_close(&seal, false);
            assert_eq!(
                count(&game.lan.as_ref().unwrap().hub.guests[0].native_menu[0]),
                0
            );
            game.lan_guest_packet(12, container_click(window, 0, 0));
            assert_eq!(
                count(&game.lan.as_ref().unwrap().hub.guests[0].native_menu[0]),
                1
            );
        });
    });
}
