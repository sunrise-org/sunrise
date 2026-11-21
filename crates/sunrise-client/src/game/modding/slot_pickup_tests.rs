//! `menu_slot_rules`' `may_pickup` through a real `MenuPart` on a mod menu type: each path that
//! takes from a slot asks it, and a refusal leaves the slot and the cursor as they were, on the
//! server (a LAN guest's click) and in the client's prediction alike, with the slots a client
//! predicted otherwise sent back; a drag and a creative clone never ask it. The fixtures need
//! prepared resources.
use crate::game::native_guest_turn_tests::with_guests;
use mod_api::{
    BlockEntityPart, BlockPart, Hooks, MenuPart, MenuQuickMoveEvent, MenuSlotRulesEvent, ModEntry,
    Mods, Registrar,
};
use std::{
    cell::RefCell,
    sync::{Arc, mpsc},
};
use sunrise_client::{
    chunk::Chunk,
    containers::{BlockEntity, ModBlockEntity, ModBlockEntityFields, ModContainer, ModMenuType},
    items::ItemStack,
    lan::{Out, PlayerSender, menu::Slots},
    menu::{Input, Kind, Menu},
};
use sunrise_protocol::{
    buf::Reader,
    item::Hashed,
    packets::play::{Clientbound as C, ContainerClick, Serverbound},
};

/// The menu type the pot opens: a prepared registry slot (`minecraft:generic_9x3`) for a
/// synthetic mod menu type, so the guest's click names it.
const MENU: &str = "minecraft:generic_9x3";
/// The pot's slot that only shows its stack: it refuses pickup and placement. (The rule's slot
/// is the backing index, so the tests leave the player's inventory index 1 alone.)
const SHOWN: usize = 1;
/// The pot's result slot: as a mod menu type's result, it refuses placement.
const RESULT: usize = 2;
/// The pot's slot that takes a stack but gives nothing up.
const KEEPS: usize = 4;
const POS: (i32, i32, i32) = (1, 64, 1);
/// The pot's container slots: the menu's hotbar 0 is slot `SLOTS + 27`.
const SLOTS: usize = 27;

thread_local! {
    /// Every question the part was asked: slot, item and count.
    static ASKED: RefCell<Vec<(usize, String, u32)>> = const { RefCell::new(Vec::new()) };
    /// The slots `menu_quick_move` was heard for.
    static MOVED: RefCell<Vec<usize>> = const { RefCell::new(Vec::new()) };
}

struct Pot;
#[mod_api::sunrise_part]
impl MenuPart for Pot {
    fn menu_slot_rules(&self, _: &dyn mod_api::Host, event: &mut MenuSlotRulesEvent<'_>) {
        let item = mod_api::__private::item_key_id(event.stack.item())
            .unwrap()
            .to_owned();
        ASKED.with(|asked| {
            asked
                .borrow_mut()
                .push((event.slot, item, event.stack.count()))
        });
        if event.slot == SHOWN {
            event.may_pickup = false;
            event.may_place = false;
        }
        event.may_pickup &= event.slot != KEEPS;
    }
}
/// The pot block's shift-click: heard, and left to the game.
struct PotBlock;
#[mod_api::sunrise_part]
impl BlockPart for PotBlock {}
#[mod_api::sunrise_part]
impl BlockEntityPart for PotBlock {
    fn menu_quick_move(
        &self,
        _: &mut dyn mod_api::ServerHost,
        event: &mut MenuQuickMoveEvent<'_>,
    ) -> mod_api::Flow {
        MOVED.with(|moved| moved.borrow_mut().push(event.slot));
        mod_api::Flow::Continue
    }
}
/// The pot's mod, with the block's `menu_quick_move` listener or without it.
struct PotMod {
    quick_move: bool,
}
#[mod_api::__subscriptions]
impl Hooks for PotMod {
    fn register(&self, r: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
        r.menu_part(MENU, Pot);
        let kind = r.block_entity_type("pot").blocks(&[]).build();
        if self.quick_move {
            r.block_entity_part(&kind, PotBlock).unwrap();
        }
    }
}

fn pot_type() -> &'static ModMenuType {
    Box::leak(Box::new(ModMenuType {
        id: MENU,
        owner: "test",
        slots: SLOTS,
        player_inventory: true,
        data_slots: 0,
        owned_slots: None,
        results: vec![RESULT],
        inputs: Vec::new(),
        crafting: None,
        links: None,
    }))
}
fn stone(count: u32) -> Option<ItemStack> {
    Some(ItemStack::new("minecraft:stone", count))
}
/// What the rule was asked, in order: slot, item and count.
type Asks = Vec<(usize, String, u32)>;
fn asked() -> Asks {
    ASKED.with(|asked| std::mem::take(&mut *asked.borrow_mut()))
}
/// How many times `asks` name `count` of `item` in `slot`.
fn times(asks: &Asks, (slot, item, count): (usize, &str, u32)) -> usize {
    let question = (slot, item.to_owned(), count);
    asks.iter().filter(|asked| **asked == question).count()
}

/// Guest 12 with the pot at `POS` open, and what their client is sent.
fn with_pot(test: impl FnOnce(&mut crate::game::Game<'_>, &mpsc::Receiver<Out>)) {
    with_pot_mod(true, test);
}

/// `with_pot`, with the block's `menu_quick_move` listener when `quick_move`.
fn with_pot_mod(
    quick_move: bool,
    test: impl FnOnce(&mut crate::game::Game<'_>, &mpsc::Receiver<Out>),
) {
    let mods: Mods = vec![ModEntry::new("pot", Box::new(PotMod { quick_move }))].into();
    crate::game::api_values::install_parts(&mods).unwrap();
    crate::entry::install_client_hooks();
    mod_platform::testing::with_mods(mods, || {
        with_guests(|game| {
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let stone = game
                .pack
                .assets
                .registry
                .default_state("minecraft:stone")
                .unwrap();
            game.world.set(POS.0, POS.1, POS.2, stone).unwrap();
            // The block's container shows its slots to the menu.
            let container = Arc::new(ModContainer {
                slots: SLOTS,
                ..Default::default()
            });
            let state = game.world.block(POS.0, POS.1, POS.2).0;
            let containers = [(state, container)].into_iter().collect();
            game.world.block_entities.set_mod_containers(containers);
            let fields = ModBlockEntityFields {
                items: vec![None; SLOTS],
                ..Default::default()
            };
            let entity = ModBlockEntity::new("pot:pot".into(), fields);
            game.world
                .block_entities
                .insert(POS, BlockEntity::Mod(Box::new(entity)));
            let (tx, rx) = mpsc::channel();
            let host = game.lan.as_mut().unwrap();
            let title = sunrise_core::nbt::Tag::String("pot".into());
            let items = vec![None; SLOTS];
            host.hub
                .open_container(12, MENU, title, POS, Slots::Block(POS), items, None);
            let menu_type = pot_type();
            host.hub.guests.update(0, |guest| {
                guest.menu.as_mut().unwrap().mod_type = Some(menu_type);
                guest.sender = PlayerSender::new(tx, Arc::default());
                // Within reach, so the menu stays valid for `menu_quick_move`.
                guest.position = [1.5, 64.0, 2.5];
            });
            test(game, &rx);
        });
    });
}

/// A client's copy of the pot's menu: `slots` by menu index, `carried` on the cursor.
fn prediction(slots: &[(usize, Option<ItemStack>)], carried: Option<ItemStack>) -> Menu {
    let mut menu = Menu::new(Kind::Mod(pot_type()));
    menu.set_registry_id(Some(MENU));
    for (slot, stack) in slots {
        menu.load_slot(*slot, stack.clone());
    }
    menu.carried = carried;
    menu
}

/// What a menu shows: the pot's slots, the inventory by index, then the cursor.
type Shown = Vec<Option<(String, u32)>>;
fn shown(stack: Option<&ItemStack>) -> Option<(String, u32)> {
    stack.map(|s| (s.item.to_string(), s.count))
}
fn predicted(menu: &Menu) -> Shown {
    (0..SLOTS)
        .map(|s| shown(menu.slot(s)))
        .chain((0..36).map(|i| shown(menu.player_item(i))))
        .chain([shown(menu.carried.as_ref())])
        .collect()
}
fn served(game: &crate::game::Game<'_>) -> Shown {
    let guest = &game.lan.as_ref().unwrap().hub.guests[0];
    let pot = game.world.block_entities.get(&POS).unwrap().slots();
    pot.iter()
        .map(Option::as_ref)
        .chain(guest.inventory[..36].iter().map(Option::as_ref))
        .chain([guest.carried.as_ref()])
        .map(shown)
        .collect()
}
/// Starts both sides with the same pot, inventory and cursor state.
fn serve(game: &mut crate::game::Game<'_>, menu: &Menu) {
    let pot = game.world.block_entities.get_mut(&POS).unwrap().slots_mut();
    for (s, slot) in pot.iter_mut().enumerate() {
        *slot = menu.slot(s).cloned();
    }
    let hub = &mut game.lan.as_mut().unwrap().hub;
    let window = hub.guests[0].menu.as_ref().unwrap().window;
    hub.guests.update(0, |guest| {
        for i in 0..36 {
            guest.inventory[i] = menu.player_item(i).cloned();
        }
        guest.carried = menu.carried.clone();
    });
    hub.resend_menu(12, window, &game.world);
}

/// A client's claim that a slot or the cursor holds `count` stone.
fn claim(game: &mut crate::game::Game<'_>, count: u32) -> Option<Hashed> {
    let item = game
        .lan
        .as_mut()
        .unwrap()
        .hub
        .item_id(&"minecraft:stone".into());
    let count = i32::try_from(count).unwrap();
    Some(Hashed {
        item,
        count,
        added: Vec::new(),
        removed: Vec::new(),
    })
}

/// The guest's click, sent as a client that predicted the take sends it (`changed`, `carried`),
/// and the slots and cursor the guest is sent back: (slot, count), with `-1` for the cursor.
fn click(
    game: &mut crate::game::Game<'_>,
    rx: &mpsc::Receiver<Out>,
    (slot, button, input): (i32, i32, Input),
    changed: Vec<(i16, Option<Hashed>)>,
    carried: Option<Hashed>,
) -> Vec<(i16, u32)> {
    let id = match input {
        Input::Pickup => 0,
        Input::QuickMove => 1,
        Input::Swap => 2,
        Input::Clone => 3,
        Input::Throw => 4,
        Input::QuickCraft => 5,
        Input::PickupAll => 6,
    };
    let menu = game.lan.as_ref().unwrap().hub.guests[0].menu.as_ref();
    let (container, state) = menu.map(|m| (m.window, m.state)).unwrap();
    rx.try_iter().for_each(drop);
    game.lan_guest_packet(
        12,
        Serverbound::ContainerClick(ContainerClick {
            container,
            state,
            slot: i16::try_from(slot).unwrap(),
            button: i8::try_from(button).unwrap(),
            input: id,
            changed,
            carried,
        }),
    );
    let count = |item: &Option<sunrise_protocol::item::Stack>| {
        item.as_ref().map_or(0, |s| u32::try_from(s.count).unwrap())
    };
    rx.try_iter()
        .flat_map(|out| match out {
            Out::Packet(body) => vec![body],
            Out::Packets(bodies) => bodies,
            Out::Disconnect(_) => Vec::new(),
        })
        .filter_map(|body| {
            let mut reader = Reader::new(&body);
            let id = reader.varint().unwrap();
            match C::decode(id, reader.rest()).unwrap() {
                C::ContainerSetSlot(set) => Some((set.slot, count(&set.item))),
                C::SetCursorItem(cursor) => Some((-1, count(&cursor.item))),
                _ => None,
            }
        })
        .collect()
}

/// `action` from `slots` and `carried` in the client's prediction, then on the server as that
/// client sends it: the prediction, and what each side asked the rule.
fn asked_by_both(
    game: &mut crate::game::Game<'_>,
    rx: &mpsc::Receiver<Out>,
    (slots, carried): (&[(usize, Option<ItemStack>)], Option<ItemStack>),
    action: (i32, i32, Input),
) -> (Menu, Asks, Asks) {
    let mut menu = prediction(slots, carried);
    serve(game, &menu);
    asked();
    menu.click(&game.menu_env(), action.0, action.1, action.2)
        .unwrap();
    let predicted = asked();
    click(game, rx, action, Vec::new(), None);
    (menu, predicted, asked())
}

/// `click` from `slots` and `carried`, on the server and in the client's prediction: what each
/// leaves, and what the guest is sent back when its client claims the shown slot emptied.
fn refused(
    game: &mut crate::game::Game<'_>,
    rx: &mpsc::Receiver<Out>,
    (slots, carried): (&[(usize, Option<ItemStack>)], Option<ItemStack>),
    action: (i32, i32, Input),
) -> (Shown, Shown, Vec<(i16, u32)>) {
    let mut menu = prediction(slots, carried);
    serve(game, &menu);
    let before = predicted(&menu);
    menu.click(&game.menu_env(), action.0, action.1, action.2)
        .unwrap();
    assert_eq!(predicted(&menu), before, "the prediction refuses the take");
    let carried = menu.carried.as_ref().map(|c| c.count);
    let carried = carried.and_then(|count| claim(game, count));
    let sent = click(game, rx, action, vec![(SHOWN as i16, None)], carried);
    (before, served(game), sent)
}

#[test]
#[ignore = "daily"]
fn a_refused_pickup_click_takes_and_merges_nothing_and_the_slot_is_sent_back() {
    with_pot(|game, rx| {
        let held = [(SHOWN, stone(5))];
        // Either button with an empty cursor, and the left button onto a matching cursor:
        // nothing leaves the slot. A client that predicted the take is sent the slot back.
        for (carried, button) in [(None, 0), (None, 1), (stone(3), 0)] {
            asked();
            let (before, after, sent) = refused(
                game,
                rx,
                (&held, carried.clone()),
                (SHOWN as i32, button, Input::Pickup),
            );
            assert_eq!(after, before, "{carried:?} {button}");
            assert!(sent.contains(&(SHOWN as i16, 5)), "{sent:?}");
            // The rule was asked for the stack the slot holds.
            assert!(asked().contains(&(SHOWN, "minecraft:stone".into(), 5)));
        }
        // A client that predicted the five stone on its cursor is sent the empty cursor.
        let mut menu = prediction(&held, None);
        serve(game, &menu);
        menu.click(&game.menu_env(), SHOWN as i32, 0, Input::Pickup)
            .unwrap();
        let cursor = claim(game, 5);
        let sent = click(
            game,
            rx,
            (SHOWN as i32, 0, Input::Pickup),
            vec![(SHOWN as i16, None)],
            cursor,
        );
        assert_eq!(sent, [(SHOWN as i16, 5), (-1, 0)]);
    });
}

#[test]
#[ignore = "daily"]
fn an_allowed_pickup_takes_the_stack_on_both_sides() {
    with_pot(|game, rx| {
        let mut menu = prediction(&[(0, stone(5))], None);
        serve(game, &menu);
        asked();
        menu.click(&game.menu_env(), 0, 0, Input::Pickup).unwrap();
        let predicted_asks = asked();
        assert_eq!(menu.slot(0), None);
        assert_eq!(menu.carried.as_ref().map(|c| c.count), Some(5));
        let cursor = claim(game, 5);
        let sent = click(game, rx, (0, 0, Input::Pickup), vec![(0, None)], cursor);
        // The server agrees with the prediction: nothing is sent back, and it asked the same.
        assert!(sent.is_empty(), "{sent:?}");
        assert_eq!(served(game), predicted(&menu));
        assert_eq!(asked(), predicted_asks);
        assert!(predicted_asks.contains(&(0, "minecraft:stone".into(), 5)));
    });
}

#[test]
#[ignore = "daily"]
fn a_refused_quick_move_moves_nothing_and_asks_before_menu_quick_move() {
    with_pot(|game, rx| {
        MOVED.with(|moved| moved.borrow_mut().clear());
        let held = [(SHOWN, stone(5))];
        let (before, after, sent) =
            refused(game, rx, (&held, None), (SHOWN as i32, 0, Input::QuickMove));
        assert_eq!(after, before);
        assert!(sent.contains(&(SHOWN as i16, 5)), "{sent:?}");
        // The pickup rule refused before the block's shift-click was asked.
        assert!(MOVED.with(|moved| moved.borrow().is_empty()));
        // An allowed slot is asked the block's shift-click, which leaves it to the game: the
        // stack moves to the inventory, as the prediction moved it.
        let mut menu = prediction(&[(0, stone(5))], None);
        serve(game, &menu);
        menu.click(&game.menu_env(), 0, 0, Input::QuickMove)
            .unwrap();
        click(game, rx, (0, 0, Input::QuickMove), Vec::new(), None);
        assert_eq!(MOVED.with(|moved| moved.borrow().clone()), [0]);
        assert_eq!(served(game), predicted(&menu));
        assert_eq!(menu.slot(0), None);
        // The rule is asked once for the clicked stack on each side: the server's ask before
        // the hook is the one the game's routing uses. Main inventory index 30 (menu slot
        // `SLOTS + 21`) moves into the pot, whose slots the move asks, so no other question
        // names its index.
        let (menu, client_asks, server_asks) =
            asked_by_both(game, rx, (&[(0, stone(5))], None), (0, 0, Input::QuickMove));
        assert_eq!(served(game), predicted(&menu));
        assert_eq!(server_asks, client_asks);
        // Reverse inventory traversal inserts into hotbar slot 8 (inventory index 8).
        assert_eq!(menu.player_item(8).map(|s| s.count), Some(5));
        assert_eq!(
            times(&server_asks, (8, "minecraft:stone", 5)),
            1,
            "{server_asks:?}"
        );
        let from = SLOTS + 21;
        MOVED.with(|moved| moved.borrow_mut().clear());
        let (menu, client_asks, server_asks) = asked_by_both(
            game,
            rx,
            (&[(from, stone(5))], None),
            (from as i32, 0, Input::QuickMove),
        );
        assert_eq!(MOVED.with(|moved| moved.borrow().clone()), [from]);
        assert_eq!(served(game), predicted(&menu));
        assert_eq!(server_asks, client_asks);
        assert_eq!(
            times(&server_asks, (30, "minecraft:stone", 5)),
            1,
            "{server_asks:?}"
        );
        // A refused shift-click asks once on each side too, and the hook is not heard.
        MOVED.with(|moved| moved.borrow_mut().clear());
        let action = (SHOWN as i32, 0, Input::QuickMove);
        let (_, client_asks, server_asks) =
            asked_by_both(game, rx, (&[(SHOWN, stone(5))], None), action);
        assert_eq!(server_asks, client_asks);
        assert_eq!(server_asks, [(SHOWN, "minecraft:stone".to_owned(), 5)]);
        assert!(MOVED.with(|moved| moved.borrow().is_empty()));
    });
}

#[test]
#[ignore = "daily"]
fn a_quick_move_without_menu_quick_move_asks_once() {
    with_pot_mod(false, |game, rx| {
        // No listener: the game's routing asks alone, once for the clicked stack, as the
        // prediction does, refused or not.
        let (menu, client_asks, server_asks) =
            asked_by_both(game, rx, (&[(0, stone(5))], None), (0, 0, Input::QuickMove));
        assert_eq!(served(game), predicted(&menu));
        assert_eq!(server_asks, client_asks);
        // Reverse inventory traversal inserts into hotbar slot 8 (inventory index 8).
        assert_eq!(menu.player_item(8).map(|s| s.count), Some(5));
        assert_eq!(
            times(&server_asks, (8, "minecraft:stone", 5)),
            1,
            "{server_asks:?}"
        );
        let from = SLOTS + 21;
        let (menu, client_asks, server_asks) = asked_by_both(
            game,
            rx,
            (&[(from, stone(5))], None),
            (from as i32, 0, Input::QuickMove),
        );
        assert_eq!(menu.player_item(30), None);
        assert_eq!(served(game), predicted(&menu));
        assert_eq!(server_asks, client_asks);
        assert_eq!(
            times(&server_asks, (30, "minecraft:stone", 5)),
            1,
            "{server_asks:?}"
        );
        let action = (SHOWN as i32, 0, Input::QuickMove);
        let (_, client_asks, server_asks) =
            asked_by_both(game, rx, (&[(SHOWN, stone(5))], None), action);
        assert_eq!(server_asks, client_asks);
        assert_eq!(server_asks, [(SHOWN, "minecraft:stone".to_owned(), 5)]);
    });
}

#[test]
#[ignore = "daily"]
fn each_path_asks_the_rule_once_per_slot_and_stack_on_both_sides() {
    with_pot(|game, rx| {
        let hotbar = SLOTS + 27;
        let dirt = || Some(ItemStack::new("minecraft:dirt", 2));
        let (stone5, dirt2) = (("minecraft:stone", 5), ("minecraft:dirt", 2));
        // Pot slot 3 answers true, the shown slot refuses. Each row: the slots and cursor, the
        // click, and how often the rule is asked each (slot, item, count) on each side. A take,
        // `allowModification` and a throw's repeat reuse the click's answer for the held
        // stack; the cursor's stack is a second question, for placing it.
        type Row = (
            Vec<(usize, Option<ItemStack>)>,
            Option<ItemStack>,
            (i32, i32, Input),
            Vec<(usize, (&'static str, u32), usize)>,
        );
        let rows: Vec<Row> = vec![
            // Pickup with either button.
            (
                vec![(3, stone(5))],
                None,
                (3, 0, Input::Pickup),
                vec![(3, stone5, 1)],
            ),
            (
                vec![(3, stone(5))],
                None,
                (3, 1, Input::Pickup),
                vec![(3, stone5, 1)],
            ),
            (
                vec![(SHOWN, stone(5))],
                None,
                (1, 0, Input::Pickup),
                vec![(SHOWN, stone5, 1)],
            ),
            (
                vec![(SHOWN, stone(5))],
                None,
                (1, 1, Input::Pickup),
                vec![(SHOWN, stone5, 1)],
            ),
            // A matching cursor: the held stack, then the cursor's for placing.
            (
                vec![(3, stone(5))],
                stone(3),
                (3, 0, Input::Pickup),
                vec![(3, stone5, 1), (3, ("minecraft:stone", 3), 1)],
            ),
            (
                vec![(SHOWN, stone(5))],
                stone(3),
                (1, 0, Input::Pickup),
                vec![(SHOWN, stone5, 1), (SHOWN, ("minecraft:stone", 3), 0)],
            ),
            // Hotbar and offhand swaps; with both stacks present the hotbar's is asked for
            // placing, unless the pickup is refused first.
            (
                vec![(3, stone(5))],
                None,
                (3, 0, Input::Swap),
                vec![(3, stone5, 1)],
            ),
            (
                vec![(3, stone(5)), (hotbar, dirt())],
                None,
                (3, 0, Input::Swap),
                vec![(3, stone5, 1), (3, dirt2, 1)],
            ),
            (
                vec![(3, stone(5))],
                None,
                (3, 40, Input::Swap),
                vec![(3, stone5, 1)],
            ),
            (
                vec![(SHOWN, stone(5))],
                None,
                (1, 0, Input::Swap),
                vec![(SHOWN, stone5, 1)],
            ),
            (
                vec![(SHOWN, stone(5)), (hotbar, dirt())],
                None,
                (1, 0, Input::Swap),
                vec![(SHOWN, stone5, 1), (SHOWN, dirt2, 0)],
            ),
            (
                vec![(SHOWN, stone(5))],
                None,
                (1, 40, Input::Swap),
                vec![(SHOWN, stone5, 1)],
            ),
            // Throws of one and of all.
            (
                vec![(3, stone(5))],
                None,
                (3, 0, Input::Throw),
                vec![(3, stone5, 1)],
            ),
            (
                vec![(3, stone(5))],
                None,
                (3, 1, Input::Throw),
                vec![(3, stone5, 1)],
            ),
            (
                vec![(SHOWN, stone(5))],
                None,
                (1, 0, Input::Throw),
                vec![(SHOWN, stone5, 1)],
            ),
            (
                vec![(SHOWN, stone(5))],
                None,
                (1, 1, Input::Throw),
                vec![(SHOWN, stone5, 1)],
            ),
            // Pick-all on the refusing slot and on an empty one: each source once, over both
            // passes and the take.
            (
                vec![(0, stone(3)), (SHOWN, stone(5))],
                stone(1),
                (1, 0, Input::PickupAll),
                vec![(0, ("minecraft:stone", 3), 1), (SHOWN, stone5, 1)],
            ),
            (
                vec![(0, stone(3)), (SHOWN, stone(5))],
                stone(1),
                (5, 0, Input::PickupAll),
                vec![(0, ("minecraft:stone", 3), 1), (SHOWN, stone5, 1)],
            ),
            // The result slot onto a matching cursor, refused whole and taken whole.
            (
                vec![(RESULT, stone(10))],
                stone(60),
                (2, 0, Input::Pickup),
                vec![
                    (RESULT, ("minecraft:stone", 10), 1),
                    (RESULT, ("minecraft:stone", 60), 1),
                ],
            ),
            (
                vec![(RESULT, stone(10))],
                stone(50),
                (2, 0, Input::Pickup),
                vec![
                    (RESULT, ("minecraft:stone", 10), 1),
                    (RESULT, ("minecraft:stone", 50), 1),
                ],
            ),
        ];
        for (slots, carried, action, expected) in rows {
            let (menu, client_asks, server_asks) =
                asked_by_both(game, rx, (&slots, carried.clone()), action);
            let row = format!("{action:?} {carried:?}");
            assert_eq!(served(game), predicted(&menu), "{row}");
            assert_eq!(server_asks, client_asks, "{row}");
            for (slot, (item, count), n) in expected {
                assert_eq!(
                    times(&server_asks, (slot, item, count)),
                    n,
                    "{row} {server_asks:?}"
                );
            }
        }
    });
}

#[test]
#[ignore = "daily"]
fn a_refused_swap_keeps_both_stacks() {
    with_pot(|game, rx| {
        let hotbar = SLOTS + 27;
        let dirt = Some(ItemStack::new("minecraft:dirt", 2));
        // An empty hotbar slot, one that holds dirt (both stacks present) and the empty off
        // hand (button 40): no exchange.
        for (other, button) in [(None, 0), (dirt, 0), (None, 40)] {
            let held = [(SHOWN, stone(5)), (hotbar, other.clone())];
            let (before, after, sent) =
                refused(game, rx, (&held, None), (SHOWN as i32, button, Input::Swap));
            assert_eq!(after, before, "{other:?} {button}");
            assert!(sent.contains(&(SHOWN as i16, 5)), "{sent:?}");
        }
    });
}

#[test]
#[ignore = "daily"]
fn a_refused_throw_drops_nothing() {
    with_pot(|game, rx| {
        let held = [(SHOWN, stone(5))];
        // One item, and the whole stack (the control variant repeats while it takes).
        for button in [0, 1] {
            let dropped = game.dropped.len();
            let (before, after, sent) = refused(
                game,
                rx,
                (&held, None),
                (SHOWN as i32, button, Input::Throw),
            );
            assert_eq!(after, before, "{button}");
            assert!(sent.contains(&(SHOWN as i16, 5)), "{sent:?}");
            assert_eq!(game.dropped.len(), dropped, "nothing was thrown");
        }
    });
}

#[test]
#[ignore = "daily"]
fn pick_all_on_a_refusing_slot_gathers_from_the_others_and_skips_it() {
    with_pot(|game, rx| {
        // The double click lands on the shown slot, which refuses, then on an empty slot: either
        // way the cursor gathers the stone of slot 0 and the shown slot keeps its own.
        for clicked in [SHOWN, 5] {
            let mut menu = prediction(&[(0, stone(3)), (SHOWN, stone(5))], stone(1));
            serve(game, &menu);
            menu.click(&game.menu_env(), clicked as i32, 0, Input::PickupAll)
                .unwrap();
            assert_eq!(menu.slot(0), None);
            assert_eq!(menu.slot(SHOWN).map(|s| s.count), Some(5));
            assert_eq!(menu.carried.as_ref().map(|c| c.count), Some(4));
            let cursor = claim(game, 4);
            let action = (clicked as i32, 0, Input::PickupAll);
            let sent = click(game, rx, action, vec![(0, None)], cursor);
            assert!(sent.is_empty(), "{sent:?}");
            assert_eq!(served(game), predicted(&menu));
        }
    });
}

#[test]
#[ignore = "daily"]
fn a_drag_that_ends_on_one_slot_is_a_pickup_click_and_asks() {
    with_pot(|game, rx| {
        // Eight stone dragged onto a slot that takes stone but gives nothing up, holding five:
        // the drag becomes a pickup click on it, which asks for its five and merges nothing.
        let mut menu = prediction(&[(KEEPS, stone(5))], stone(8));
        serve(game, &menu);
        let before = predicted(&menu);
        asked();
        for (slot, button) in [(-999, 0), (KEEPS as i32, 1), (-999, 2)] {
            menu.click(&game.menu_env(), slot, button, Input::QuickCraft)
                .unwrap();
            click(
                game,
                rx,
                (slot, button, Input::QuickCraft),
                Vec::new(),
                None,
            );
        }
        assert_eq!(predicted(&menu), before);
        assert_eq!(served(game), before);
        assert!(asked().contains(&(KEEPS, "minecraft:stone".into(), 5)));
    });
}

#[test]
#[ignore = "daily"]
fn a_result_slot_gives_all_or_nothing_to_a_matching_cursor() {
    with_pot(|game, rx| {
        // Ten stone in the result, sixty on the cursor: only four fit, so nothing is taken.
        let held = [(RESULT, stone(10))];
        let (before, after, _) = refused(
            game,
            rx,
            (&held, stone(60)),
            (RESULT as i32, 0, Input::Pickup),
        );
        assert_eq!(after, before);
        // With room for all ten, the cursor takes them.
        let mut menu = prediction(&held, stone(50));
        serve(game, &menu);
        menu.click(&game.menu_env(), RESULT as i32, 0, Input::Pickup)
            .unwrap();
        assert_eq!(menu.carried.as_ref().map(|c| c.count), Some(60));
        click(
            game,
            rx,
            (RESULT as i32, 0, Input::Pickup),
            Vec::new(),
            None,
        );
        assert_eq!(served(game), predicted(&menu));
    });
}

#[test]
#[ignore = "daily"]
fn an_ordinary_slot_gives_on_every_path_as_predicted() {
    with_pot(|game, rx| {
        // Quick move, a hotbar and an off-hand swap, a throw of one and of all, on slot 0 that
        // answers true: the server does what the prediction did.
        for (button, input) in [
            (0, Input::QuickMove),
            (0, Input::Swap),
            (40, Input::Swap),
            (0, Input::Throw),
            (1, Input::Throw),
        ] {
            let mut menu = prediction(&[(0, stone(5))], None);
            serve(game, &menu);
            menu.click(&game.menu_env(), 0, button, input).unwrap();
            assert_ne!(menu.slot(0).map(|s| s.count), Some(5), "{button}");
            click(game, rx, (0, button, input), Vec::new(), None);
            assert_eq!(served(game), predicted(&menu), "{button}");
        }
    });
}

#[test]
#[ignore = "daily"]
fn a_drag_and_a_creative_clone_never_ask_the_pickup_rule() {
    with_pot(|game, rx| {
        // A drag of eight stone over slot 0, the shown slot (holding five) and slot 2: only
        // placement and capacity are asked, each for the carried eight, never for the shown
        // slot's five; the shown slot refuses, the other two take four each.
        let mut menu = prediction(&[(SHOWN, stone(5))], stone(8));
        serve(game, &menu);
        asked();
        for (slot, button) in [(-999, 0), (0, 1), (SHOWN as i32, 1), (3, 1), (-999, 2)] {
            menu.click(&game.menu_env(), slot, button, Input::QuickCraft)
                .unwrap();
            click(
                game,
                rx,
                (slot, button, Input::QuickCraft),
                Vec::new(),
                None,
            );
        }
        let asks = asked();
        assert!(!asks.is_empty());
        assert!(asks.iter().all(|(_, _, count)| *count == 8), "{asks:?}");
        assert_eq!(served(game), predicted(&menu));
        assert_eq!(menu.slot(0).map(|s| s.count), Some(4));
        assert_eq!(menu.slot(SHOWN).map(|s| s.count), Some(5));
        assert_eq!(menu.slot(3).map(|s| s.count), Some(4));
        // A creative clone of the shown slot fills the cursor and asks nothing.
        let mut menu = prediction(&[(SHOWN, stone(5))], None);
        menu.creative = true;
        serve(game, &menu);
        game.lan.as_mut().unwrap().hub.guests.update(0, |guest| {
            guest.game_mode = sunrise_client::lan::guest::CREATIVE;
        });
        menu.click(&game.menu_env(), SHOWN as i32, 2, Input::Clone)
            .unwrap();
        click(game, rx, (SHOWN as i32, 2, Input::Clone), Vec::new(), None);
        assert_eq!(menu.carried.as_ref().map(|c| c.count), Some(64));
        assert_eq!(served(game), predicted(&menu));
        assert!(asked().is_empty());
    });
}
