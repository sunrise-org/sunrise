//! `menu_slot_take` through a real `BlockEntityPart` on the block a mod menu type is open on, as
//! a LAN guest's clicks reach the server: what each take hands over, once per take; what is not a
//! take is silent; and a failing listener undoes its writes with the take. The fixtures need
//! prepared resources.
use crate::game::native_guest_turn_tests::with_guests;
use mod_api::{
    BlockEntityPart, BlockPart, Hooks, HostBlockStates, ITEMS, ItemCraftedEvent, ItemPlace,
    ItemResource, MenuPart, MenuSlotRulesEvent, MenuSlotTakeEvent, ModEntry, Mods, Registrar,
};
use std::{
    cell::{Cell, RefCell},
    sync::{Arc, mpsc},
};
use sunrise_client::{
    chunk::Chunk,
    containers::{
        BlockEntity, ModBlockEntity, ModBlockEntityFields, ModContainer, ModCrafting, ModLinks,
        ModMenuType,
    },
    items::ItemStack,
    lan::{
        Out, PlayerSender,
        menu::{ModCraft, Slots},
    },
    menu::{Input, Kind, Menu},
};
use sunrise_protocol::packets::play::{Clientbound, ContainerClick, Serverbound};

/// The menu type the pot opens: a prepared registry slot (`minecraft:generic_9x3`) for a
/// synthetic mod menu type, so the guest's click names it.
const MENU: &str = "minecraft:generic_9x3";
const POS: (i32, i32, i32) = (1, 64, 1);
const LINK: (i32, i32, i32) = (2, 64, 1);
/// The pot's slot whose pickup rule refuses.
const KEPT: usize = 1;
/// The pot's result slot: it refuses placement.
const RESULT: usize = 2;
/// Slots of the pot's container: its hotbar 0 is menu slot `SLOTS + 27`.
const SLOTS: usize = 9;

/// A take as heard: player, menu, slot, what was taken and what was left (item and count).
type Heard = (u128, String, usize, (String, u32), Option<(String, u32)>);

#[derive(Clone, Copy)]
enum TakeScript {
    Hear,
    Reward,
    Panic,
    PanicWithoutReward,
    Invalid,
    RefillGrid,
    RefillThenPanic,
    RemoveBacking,
    RemoveBackingDuringSwap,
    SwapRoom,
    ReplacePickAllCursor,
    LinkedWriteThenPanic,
}

thread_local! {
    static HEARD: RefCell<Vec<Heard>> = const { RefCell::new(Vec::new()) };
    static SCRIPT: Cell<TakeScript> = const { Cell::new(TakeScript::Hear) };
    static CRAFTED: Cell<usize> = const { Cell::new(0) };
    static CHANGED: Cell<usize> = const { Cell::new(0) };
    static LINK_INSERTED: Cell<u32> = const { Cell::new(0) };
}

fn api_item(id: &str, count: u32) -> mod_api::ItemStack {
    mod_api::ItemStack::new(mod_api::__private::item_key(id).unwrap(), count).unwrap()
}

fn stack(stack: &mod_api::ItemStack) -> (String, u32) {
    let item = mod_api::__private::item_key_id(stack.item()).unwrap();
    (item.to_owned(), stack.count())
}

struct Pot;
#[mod_api::sunrise_part]
impl BlockPart for Pot {
    fn contents_changed(
        &self,
        _: &mut dyn mod_api::ServerHost,
        _: &mut mod_api::ContentsChangedEvent,
    ) {
        CHANGED.set(CHANGED.get() + 1);
    }
}
#[mod_api::sunrise_part]
impl BlockEntityPart for Pot {
    fn menu_slot_take(
        &self,
        host: &mut dyn mod_api::ServerHost,
        event: &mut MenuSlotTakeEvent<'_>,
    ) {
        let heard = (
            event.player,
            event.menu.to_owned(),
            event.slot,
            stack(event.taken),
            event.left.map(stack),
        );
        let heard_count = HEARD.with(|h| {
            let mut h = h.borrow_mut();
            h.push(heard);
            h.len()
        });
        let place = ItemPlace::PlayerSlot {
            player: event.player,
            slot: 8,
        };
        let tx = host.transaction_current().expect("a take is tentative");
        match SCRIPT.get() {
            TakeScript::Hear => {}
            TakeScript::PanicWithoutReward => panic!("a broken craft listener"),
            TakeScript::Reward | TakeScript::Panic | TakeScript::Invalid => {
                assert_eq!(
                    place.insert(host, &tx, &ItemResource::of(event.taken), 1),
                    1
                );
                if matches!(SCRIPT.get(), TakeScript::Invalid) {
                    event.entity.items[0] = Some(api_item("minecraft:stone", 65));
                }
                if matches!(SCRIPT.get(), TakeScript::Panic) {
                    panic!("a broken listener");
                }
            }
            TakeScript::RefillGrid if heard_count == 1 => {
                assert!(
                    event.entity.items[0].is_none(),
                    "the craft consumed its log"
                );
                assert_eq!(
                    place.amount(host),
                    4,
                    "the first craft already reached hotbar 8"
                );
                event.entity.items[0] = Some(api_item("minecraft:oak_log", 1));
                event.entity.items[10] = Some(api_item("minecraft:stone", 7));
            }
            TakeScript::RefillThenPanic => {
                if heard_count == 2 {
                    panic!("the second throw failed");
                }
                assert!(event.entity.items[0].is_none());
                event.entity.items[0] = Some(api_item("minecraft:stone", 2));
            }
            TakeScript::RemoveBacking | TakeScript::RemoveBackingDuringSwap => {
                let level = host.level("minecraft:overworld").unwrap();
                let air = mod_api::BlockState::of(
                    &host.block_states(),
                    mod_api::__private::block_key("minecraft:air").unwrap(),
                    &[],
                )
                .unwrap();
                assert_eq!(
                    level.set_block(
                        host,
                        [POS.0, POS.1, POS.2],
                        air,
                        mod_api::BlockUpdateFlags::KNOWN_SHAPE
                    ),
                    Ok(true)
                );
            }
            TakeScript::SwapRoom => {
                // The old slot stack has not been added to inventory or dropped yet.
                let dirt = place.resource(host).expect("hotbar 8 still holds dirt");
                assert_eq!(place.extract(host, &tx, &dirt, 64), 64);
            }
            TakeScript::ReplacePickAllCursor if heard_count == 1 => {
                let player = host.player(event.player).unwrap();
                let cursor = player.container_menu(host).unwrap().cursor();
                assert_eq!(
                    cursor.extract(host, &tx, &ItemResource::of(event.taken), 4),
                    4
                );
                let pearls = ItemResource::of(&api_item("minecraft:ender_pearl", 1));
                assert_eq!(cursor.insert(host, &tx, &pearls, 14), 14);
            }
            TakeScript::LinkedWriteThenPanic => {
                let level = host.level("minecraft:overworld").unwrap();
                let items = level
                    .capability(host, &ITEMS, [LINK.0, LINK.1, LINK.2], None)
                    .unwrap();
                LINK_INSERTED.set(items.insert_at(host, &tx, 0, &ItemResource::of(event.taken), 1));
                panic!("a linked write failed");
            }
            TakeScript::RefillGrid | TakeScript::ReplacePickAllCursor => {}
        }
    }
}
/// The pot's pickup rule: slot `KEPT` gives nothing up.
struct Rules;
#[mod_api::sunrise_part]
impl MenuPart for Rules {
    fn menu_slot_rules(&self, _: &dyn mod_api::Host, event: &mut MenuSlotRulesEvent<'_>) {
        event.may_pickup &= event.slot != KEPT;
        if matches!(
            SCRIPT.get(),
            TakeScript::SwapRoom | TakeScript::RemoveBackingDuringSwap
        ) {
            event.max_stack = 1;
        }
    }
}
struct TakeMod;
#[mod_api::__subscriptions]
impl Hooks for TakeMod {
    fn item_crafted(&self, _: &mut dyn mod_api::ServerHost, _: &mut ItemCraftedEvent<'_>) {
        CRAFTED.set(CRAFTED.get() + 1);
    }

    fn register(&self, r: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
        r.menu_part(MENU, Rules);
        let stone = r.block_key("minecraft:stone");
        r.block_part(stone, Pot);
        let kind = r.block_entity_type("pot").blocks(&[]).build();
        r.block_entity_part(&kind, Pot).unwrap();
    }
}

fn pot_type(slots: usize, crafting: Option<ModCrafting>) -> &'static ModMenuType {
    Box::leak(Box::new(ModMenuType {
        id: MENU,
        owner: "take",
        slots,
        player_inventory: true,
        data_slots: 0,
        owned_slots: None,
        results: vec![RESULT],
        inputs: Vec::new(),
        crafting,
        links: None,
    }))
}
fn item(name: &str, count: u32) -> Option<ItemStack> {
    Some(ItemStack::new(name, count))
}
fn stone(count: u32) -> Option<ItemStack> {
    item("minecraft:stone", count)
}
fn heard() -> Vec<Heard> {
    HEARD.with(|h| std::mem::take(&mut *h.borrow_mut()))
}
/// A take of `taken` stone from `slot`, `left` stone staying (0 for none), heard from guest 12.
fn took(slot: usize, taken: u32, left: u32) -> Heard {
    let stone = |count| ("minecraft:stone".to_owned(), count);
    let left = (left > 0).then(|| stone(left));
    (12, MENU.to_owned(), slot, stone(taken), left)
}

/// Guest 12 with the pot of menu type `menu_type` at `POS` open.
fn with_pot(menu_type: &'static ModMenuType, test: impl FnOnce(&mut crate::game::Game<'_>)) {
    let mods: Mods = vec![ModEntry::new("take", Box::new(TakeMod))].into();
    mod_platform::testing::with_mods(mods.clone(), || {
        crate::game::api_values::install_parts(&mods).unwrap();
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
            let slots = menu_type.slots;
            let container = Arc::new(ModContainer {
                slots,
                inputs: vec![0],
                ..Default::default()
            });
            let state = game.world.block(POS.0, POS.1, POS.2).0;
            let containers = [(state, container)].into_iter().collect();
            game.world.block_entities.set_mod_containers(containers);
            let fields = ModBlockEntityFields {
                items: vec![None; slots],
                ..Default::default()
            };
            let entity = ModBlockEntity::new("take:pot".into(), fields);
            game.world
                .block_entities
                .insert(POS, BlockEntity::Mod(Box::new(entity)));
            let host = game.lan.as_mut().unwrap();
            let title = sunrise_core::nbt::Tag::String("pot".into());
            let items = vec![None; slots];
            host.hub
                .open_container(12, MENU, title, POS, Slots::Block(POS), items, None);
            host.hub.guests.update(0, |guest| {
                guest.menu.as_mut().unwrap().mod_type = Some(menu_type);
            });
            SCRIPT.set(TakeScript::Hear);
            CRAFTED.set(0);
            CHANGED.set(0);
            LINK_INSERTED.set(0);
            heard();
            test(game);
        });
    });
}

/// Sets the pot's slots (`pot`), the guest's inventory (`inventory`, by index) and cursor.
fn set(
    game: &mut crate::game::Game<'_>,
    pot: &[(usize, Option<ItemStack>)],
    inventory: &[(usize, Option<ItemStack>)],
    carried: Option<ItemStack>,
) {
    let slots = game.world.block_entities.get_mut(&POS).unwrap().slots_mut();
    slots.fill(None);
    for (slot, stack) in pot {
        slots[*slot] = stack.clone();
    }
    game.lan.as_mut().unwrap().hub.guests.update(0, |guest| {
        guest.inventory.fill(None);
        for (slot, stack) in inventory {
            guest.inventory[*slot] = stack.clone();
        }
        guest.carried = carried;
        guest.offhand = None;
    });
}
/// The guest's click (`input` is `ClickType`'s id), claiming nothing.
fn click(game: &mut crate::game::Game<'_>, slot: i32, button: i8, input: i32) {
    let menu = game.lan.as_ref().unwrap().hub.guests[0].menu.as_ref();
    let (container, state) = menu.map(|m| (m.window, m.state)).unwrap();
    game.lan_guest_packet(
        12,
        Serverbound::ContainerClick(ContainerClick {
            container,
            state,
            slot: i16::try_from(slot).unwrap(),
            button,
            input,
            changed: Vec::new(),
            carried: None,
        }),
    );
}
/// The count in pot slot `slot`.
fn in_pot(game: &crate::game::Game<'_>, slot: usize) -> u32 {
    let slots = game.world.block_entities.get(&POS).unwrap().slots();
    slots[slot].as_ref().map_or(0, |s| s.count)
}
/// The count in the guest's inventory slot `slot`, and on their cursor.
fn held(game: &crate::game::Game<'_>, slot: usize) -> (u32, u32) {
    let guest = &game.lan.as_ref().unwrap().hub.guests[0];
    let count = |stack: &Option<ItemStack>| stack.as_ref().map_or(0, |s| s.count);
    (count(&guest.inventory[slot]), count(&guest.carried))
}

#[test]
#[ignore = "daily"]
fn a_pickup_hears_what_left_the_slot_and_what_stayed() {
    with_pot(pot_type(SLOTS, None), |game| {
        // The left button takes all five, the right button half rounded up.
        set(game, &[(0, stone(5))], &[], None);
        click(game, 0, 0, 0);
        assert_eq!(heard(), [took(0, 5, 0)]);
        set(game, &[(0, stone(5))], &[], None);
        click(game, 0, 1, 0);
        assert_eq!(heard(), [took(0, 3, 2)]);
        // The result slot refuses placement: a matching cursor takes all of it.
        set(game, &[(RESULT, stone(4))], &[], stone(3));
        click(game, RESULT as i32, 0, 0);
        assert_eq!(heard(), [took(RESULT, 4, 0)]);
        assert_eq!(held(game, 0).1, 7);
        // The hook is the server's: the client's prediction of the same take hears nothing.
        let mut menu = Menu::new(Kind::Mod(pot_type(SLOTS, None)));
        menu.set_registry_id(Some(MENU));
        menu.load_slot(0, stone(5));
        menu.click(&game.menu_env(), 0, 0, Input::Pickup).unwrap();
        assert_eq!(menu.carried.as_ref().map(|c| c.count), Some(5));
        assert!(heard().is_empty());
    });
}

#[test]
#[ignore = "daily"]
fn a_swap_hears_the_slots_stack() {
    with_pot(pot_type(SLOTS, None), |game| {
        // Into an empty hotbar slot: the whole stack. With dirt there: the slot's former stack,
        // the dirt now in the slot.
        set(game, &[(0, stone(5))], &[], None);
        click(game, 0, 0, 2);
        assert_eq!(heard(), [took(0, 5, 0)]);
        set(
            game,
            &[(0, stone(5))],
            &[(0, item("minecraft:dirt", 2))],
            None,
        );
        click(game, 0, 0, 2);
        let dirt = Some(("minecraft:dirt".to_owned(), 2));
        assert_eq!(
            heard(),
            [(12, MENU.into(), 0, ("minecraft:stone".into(), 5), dirt)]
        );
        // Into the empty off hand (button 40): the whole stack.
        set(game, &[(0, stone(5))], &[], None);
        click(game, 0, 40, 2);
        assert_eq!(heard(), [took(0, 5, 0)]);
    });
}

#[test]
#[ignore = "daily"]
fn a_throw_and_pick_all_hear_each_take() {
    with_pot(pot_type(SLOTS, None), |game| {
        // A throw of one, and of the stack (one take: the slot is empty after it).
        set(game, &[(0, stone(5))], &[], None);
        click(game, 0, 0, 4);
        assert_eq!(heard(), [took(0, 1, 4)]);
        set(game, &[(0, stone(5))], &[], None);
        click(game, 0, 1, 4);
        assert_eq!(heard(), [took(0, 5, 0)]);
        // Pick-all on an empty slot gathers from slots 0 and 3, each heard with its part, and
        // from the hotbar, which is the player's own and not heard.
        set(
            game,
            &[(0, stone(3)), (3, stone(2))],
            &[(4, stone(6))],
            stone(1),
        );
        click(game, 5, 0, 6);
        assert_eq!(heard(), [took(0, 3, 0), took(3, 2, 0)]);
        assert_eq!(held(game, 4), (0, 12));
    });
}

#[test]
#[ignore = "daily"]
fn pick_all_respects_the_cursor_items_new_capacity_after_a_take() {
    with_pot(pot_type(SLOTS, None), |game| {
        let pearls = "minecraft:ender_pearl";
        set(game, &[(0, stone(3)), (3, item(pearls, 10))], &[], stone(1));
        SCRIPT.set(TakeScript::ReplacePickAllCursor);
        click(game, 5, 0, 6);
        let took_pearls = (
            12,
            MENU.into(),
            3,
            (pearls.into(), 2),
            Some((pearls.into(), 8)),
        );
        assert_eq!(heard(), [took(0, 3, 0), took_pearls]);
        assert_eq!(in_pot(game, 3), 8);
        let guest = &game.lan.as_ref().unwrap().hub.guests[0];
        assert_eq!(guest.carried, item(pearls, 16));
    });
}

#[test]
#[ignore = "daily"]
fn a_shift_click_hears_the_amount_moved_once_per_craft() {
    with_pot(pot_type(SLOTS, None), |game| {
        // A plain slot: the stack moved to the inventory, once.
        set(game, &[(0, stone(5))], &[], None);
        click(game, 0, 0, 1);
        assert_eq!(heard(), [took(0, 5, 0)]);
        // The result slot holding many crafts' output (a furnace's shape): one move takes all
        // twenty, heard once.
        set(game, &[(RESULT, stone(20))], &[], None);
        click(game, RESULT as i32, 0, 1);
        assert_eq!(heard(), [took(RESULT, 20, 0)]);
        // Room for four only (the inventory full but for four stone of room): one notice with
        // the four moved and the sixteen left; with no room at all, nothing moves and nothing
        // is heard.
        let dirt = |slot| (slot, item("minecraft:dirt", 64));
        let mut full: Vec<_> = (1..36).map(dirt).collect();
        full.push((0, stone(60)));
        set(game, &[(RESULT, stone(20))], &full, None);
        click(game, RESULT as i32, 0, 1);
        assert_eq!(heard(), [took(RESULT, 4, 16)]);
        full.pop();
        full.push(dirt(0));
        set(game, &[(RESULT, stone(20))], &full, None);
        click(game, RESULT as i32, 0, 1);
        assert!(heard().is_empty());
        assert_eq!(in_pot(game, RESULT), 20);
    });
    // A crafting grid (slots 0 to 8) and its result (slot 9): three oak logs make planks three
    // times, each craft heard with its four planks.
    let grid = ModCrafting {
        grid: 0,
        size: 3,
        result: 9,
    };
    with_pot(pot_type(10, Some(grid)), |game| {
        let logs = item("minecraft:oak_log", 3);
        set(game, &[(0, logs.clone())], &[], None);
        game.lan.as_mut().unwrap().hub.guests.update(0, |guest| {
            let mut shown = vec![None; 9];
            shown[0] = logs;
            let result = item("minecraft:oak_planks", 4);
            guest.menu.as_mut().unwrap().craft = Some(ModCraft {
                grid: shown,
                result,
            });
        });
        click(game, 9, 0, 1);
        let crafts: Vec<_> = heard()
            .into_iter()
            .map(|(_, _, slot, taken, _)| (slot, taken))
            .collect();
        let planks = (9, ("minecraft:oak_planks".to_owned(), 4));
        assert_eq!(crafts, [planks.clone(), planks.clone(), planks]);
        assert_eq!(in_pot(game, 0), 0);
    });
}

#[test]
#[ignore = "daily"]
fn an_exchange_an_insertion_and_a_refused_take_are_silent() {
    with_pot(pot_type(SLOTS, None), |game| {
        // Dirt on the cursor over stone: the stacks change places, nothing is taken.
        set(game, &[(0, stone(5))], &[], item("minecraft:dirt", 2));
        click(game, 0, 0, 0);
        assert_eq!(in_pot(game, 0), 2);
        // Stone put into an empty slot.
        set(game, &[], &[], stone(5));
        click(game, 0, 0, 0);
        assert_eq!(in_pot(game, 0), 5);
        // The slot whose pickup rule refuses: nothing leaves it.
        set(game, &[(KEPT, stone(5))], &[], None);
        click(game, KEPT as i32, 0, 0);
        assert_eq!(in_pot(game, KEPT), 5);
        assert!(heard().is_empty());
    });
}

#[test]
#[ignore = "daily"]
fn a_failing_listener_undoes_its_writes_and_the_take() {
    with_pot(pot_type(SLOTS, None), |game| {
        // The listener puts one stone in hotbar slot 8, then fails: the stone it put is gone,
        // the five stone are back in the slot and the cursor is empty.
        SCRIPT.set(TakeScript::Panic);
        set(game, &[(0, stone(5))], &[], None);
        click(game, 0, 0, 0);
        assert_eq!(heard(), [took(0, 5, 0)]);
        assert_eq!(in_pot(game, 0), 5);
        assert_eq!(held(game, 8), (0, 0));
        // A listener that returns keeps its write, and the take stands.
        SCRIPT.set(TakeScript::Reward);
        click(game, 0, 0, 0);
        assert_eq!(heard(), [took(0, 5, 0)]);
        assert_eq!(in_pot(game, 0), 0);
        assert_eq!(held(game, 8), (1, 5));
    });
}

#[test]
#[ignore = "daily"]
fn invalid_snapshot_or_failed_offhand_swap_rolls_back_without_publishing() {
    with_pot(pot_type(SLOTS, None), |game| {
        for script in [TakeScript::Invalid, TakeScript::Panic] {
            SCRIPT.set(script);
            set(game, &[(0, stone(5))], &[], None);
            click(game, 0, 40, 2);
            assert_eq!(heard(), [took(0, 5, 0)]);
            assert_eq!(in_pot(game, 0), 5);
            assert_eq!(held(game, 8), (0, 0));
            assert!(game.lan.as_ref().unwrap().hub.guests[0].offhand.is_none());
            assert_eq!(
                CHANGED.get(),
                0,
                "a refused take publishes no storage notice"
            );
        }
        SCRIPT.set(TakeScript::Hear);
        click(game, 0, 40, 2);
        assert_eq!(CHANGED.get(), 1);
        assert_eq!(game.lan.as_ref().unwrap().hub.guests[0].offhand, stone(5));
    });
}

#[test]
#[ignore = "daily"]
fn linked_sources_are_silent_and_linked_writes_abort_with_the_click() {
    let menu = Box::leak(Box::new(ModMenuType {
        links: Some(ModLinks { from: 0, count: 1 }),
        ..pot_type(SLOTS, None).clone()
    }));
    with_pot(menu, |game| {
        let state = game.world.block(POS.0, POS.1, POS.2);
        game.world.set(LINK.0, LINK.1, LINK.2, state).unwrap();
        let put_link = |game: &mut crate::game::Game<'_>| {
            game.world.block_entities.insert(
                LINK,
                BlockEntity::Mod(Box::new(ModBlockEntity::new(
                    "take:pot".into(),
                    ModBlockEntityFields {
                        items: vec![stone(3); SLOTS],
                        ..Default::default()
                    },
                ))),
            );
        };
        put_link(game);
        game.lan
            .as_mut()
            .unwrap()
            .hub
            .guests
            .update(0, |guest| guest.menu.as_mut().unwrap().links.push(LINK));
        let linked = |game: &crate::game::Game<'_>| {
            game.world.block_entities.get(&LINK).unwrap().slots()[0].clone()
        };
        click(game, 0, 0, 0);
        assert!(heard().is_empty());
        assert_eq!(held(game, 8).1, 3);
        assert_eq!(linked(game), None);

        // Pick-all first takes from the linked source without a notice. The later own-slot
        // notice writes into that same linked container and fails: all three writes abort.
        put_link(game);
        set(game, &[(3, stone(5))], &[], stone(1));
        SCRIPT.set(TakeScript::LinkedWriteThenPanic);
        click(game, 5, 0, 6);
        assert_eq!(heard(), [took(3, 5, 0)]);
        assert_eq!(
            LINK_INSERTED.get(),
            1,
            "the listener actually wrote to the link"
        );
        assert_eq!(in_pot(game, 3), 5);
        assert_eq!(linked(game), stone(3));
        assert_eq!(held(game, 8), (0, 1));
    });
}

#[test]
#[ignore = "daily"]
fn a_throw_repeat_sees_the_previous_notice_and_oversized_swap_delivers_after_it() {
    with_pot(pot_type(SLOTS, None), |game| {
        set(game, &[(0, stone(2))], &[], None);
        SCRIPT.set(TakeScript::RefillThenPanic);
        click(game, 0, 1, 4);
        assert_eq!(heard(), [took(0, 2, 0), took(0, 2, 0)]);
        assert_eq!(in_pot(game, 0), 2);
        assert!(game.dropped.is_empty());

        let dirt: Vec<_> = (0..36).map(|i| (i, item("minecraft:dirt", 64))).collect();
        set(game, &[(0, stone(5))], &dirt, None);
        SCRIPT.set(TakeScript::SwapRoom);
        click(game, 0, 0, 2);
        assert_eq!(
            heard(),
            [(
                12,
                MENU.to_owned(),
                0,
                ("minecraft:stone".to_owned(), 5),
                Some(("minecraft:dirt".to_owned(), 1))
            )]
        );
        assert_eq!(in_pot(game, 0), 1);
        assert_eq!(held(game, 0).0, 63);
        assert_eq!(
            game.lan.as_ref().unwrap().hub.guests[0].inventory[8],
            stone(5)
        );
        assert!(game.dropped.is_empty());
    });
}

#[test]
#[ignore = "daily"]
fn an_oversized_swap_delivers_the_displaced_stack_even_when_the_holder_is_removed() {
    with_pot(pot_type(SLOTS, None), |game| {
        let dirt = "minecraft:dirt";
        let mut inventory: Vec<_> = (0..36).map(|i| (i, item(dirt, 64))).collect();
        inventory[8].1 = stone(62);
        set(game, &[(0, stone(5))], &inventory, None);
        SCRIPT.set(TakeScript::RemoveBackingDuringSwap);
        click(game, 0, 0, 2);
        assert_eq!(
            heard(),
            [(
                12,
                MENU.into(),
                0,
                ("minecraft:stone".into(), 5),
                Some((dirt.into(), 1))
            )]
        );
        assert!(game.world.block_entities.get(&POS).is_none());
        let guest = &game.lan.as_ref().unwrap().hub.guests[0];
        assert!(guest.menu.is_none());
        assert_eq!(guest.inventory[0], item(dirt, 63));
        assert_eq!(guest.inventory[8], stone(64));
        let spawned = game.dropped.iter().map(|e| &e.stack);
        let queued = game.spill.spawns.iter().map(|e| &e.stack);
        assert_eq!(
            spawned
                .chain(queued)
                .filter(|s| &*s.item == "minecraft:stone")
                .map(|s| s.count)
                .sum::<u32>(),
            3,
            "only the part of the displaced stack that did not fit is dropped"
        );
    });
}

#[test]
#[ignore = "daily"]
fn replacing_the_detached_holder_spills_only_the_contents_after_the_take() {
    with_pot(pot_type(SLOTS, None), |game| {
        set(game, &[(0, stone(5)), (3, stone(2))], &[], None);
        SCRIPT.set(TakeScript::RemoveBacking);
        click(game, 0, 0, 0);
        assert_eq!(heard(), [took(0, 5, 0)]);
        assert!(game.world.block_entities.get(&POS).is_none());
        let guest = &game.lan.as_ref().unwrap().hub.guests[0];
        assert!(guest.menu.is_none());
        // A live close returns the stack just taken from the cursor to the inventory.
        assert_eq!(guest.carried, None);
        assert_eq!(guest.inventory[0], stone(5));
        let spawned = game.dropped.iter().map(|e| &e.stack);
        let queued = game.spill.spawns.iter().map(|e| &e.stack);
        assert_eq!(
            spawned
                .chain(queued)
                .filter(|s| &*s.item == "minecraft:stone")
                .map(|s| s.count)
                .sum::<u32>(),
            2
        );
    });
}

const PLANKS: &str = "minecraft:oak_planks";
const LOG: &str = "minecraft:oak_log";
const GRID: ModCrafting = ModCrafting {
    grid: 0,
    size: 3,
    result: 9,
};

#[test]
#[ignore = "daily"]
fn a_refilled_grid_crafts_again_and_the_reply_includes_the_listener_only_slot() {
    with_pot(pot_type(11, Some(GRID)), |game| {
        set(game, &[(0, item(LOG, 1))], &[], None);
        let (tx, rx) = mpsc::channel();
        game.lan.as_mut().unwrap().hub.guests.update(0, |guest| {
            guest.sender = PlayerSender::new(tx, Arc::default())
        });
        SCRIPT.set(TakeScript::RefillGrid);
        click(game, 9, 0, 1);
        let takes = heard();
        assert_eq!(takes.len(), 2);
        assert!(
            takes
                .iter()
                .all(|take| take.2 == 9 && take.3 == (PLANKS.to_owned(), 4))
        );
        assert_eq!(in_pot(game, 0), 0);
        assert_eq!(in_pot(game, 10), 7);
        assert_eq!(held(game, 8), (8, 0));
        let mut reply = rx.try_iter().flat_map(|out| match out {
            Out::Packet(body) => vec![body],
            Out::Packets(bodies) => bodies,
            _ => Vec::new(),
        });
        assert!(reply.any(|body| {
            let mut r = sunrise_protocol::buf::Reader::new(&body);
            matches!(Clientbound::decode(r.varint().unwrap(), r.rest()),
                Ok(Clientbound::ContainerSetSlot(p)) if p.slot == 10 && p.item.as_ref().is_some_and(|s| s.count == 7))
        }), "the client did not predict the listener's write to slot 10");
    });
}

#[test]
#[ignore = "daily"]
fn a_partial_mod_craft_keeps_crafting_bookkeeping_but_not_item_crafted() {
    use sunrise_client::advancements::{Advancement, Criterion, Tree};
    const ID: &str = "take:crafted";
    const CRITERION: &str = "planks";
    const TRIGGER: &str = "minecraft:recipe_crafted";
    with_pot(pot_type(10, Some(GRID)), |game| {
        let mut tree = Tree::default();
        tree.list.push(Advancement {
            id: ID.into(), parent: None, display: None, rewards: Default::default(),
            requirements: vec![vec![CRITERION.into()]],
            criteria: vec![Criterion { name: CRITERION.into(), trigger: TRIGGER.into(), conditions: serde_json::json!({"recipes": [PLANKS], "ingredients": [{"items": LOG}]}).into() }],
        });
        tree.by_id.insert(ID.into(), 0);
        tree.by_trigger.insert(TRIGGER.into(), vec![(0, 0)]);
        game.adv.tree = Arc::new(tree);
        let mut full: Vec<_> = (1..36).map(|i| (i, item("minecraft:dirt", 64))).collect();
        full.push((0, item(PLANKS, 62)));
        for (script, success) in [
            (TakeScript::PanicWithoutReward, false),
            (TakeScript::Hear, true),
        ] {
            set(game, &[(0, item(LOG, 1))], &full, None);
            SCRIPT.set(script);
            click(game, 9, 0, 1);
            let takes = heard();
            assert_eq!(takes.len(), 1);
            assert_eq!(takes[0].3, (PLANKS.to_owned(), 2));
            assert_eq!(takes[0].4, Some((PLANKS.to_owned(), 2)));
            assert_eq!(in_pot(game, 0), u32::from(!success));
            assert_eq!(held(game, 0), (if success { 64 } else { 62 }, 0));
            assert_eq!(
                game.dropped.iter().map(|e| e.stack.count).sum::<u32>(),
                if success { 2 } else { 0 }
            );
            let host = game.lan.as_ref().unwrap();
            assert_eq!(
                host.play[&12].stats.get("crafted", PLANKS),
                if success { 2 } else { 0 }
            );
            assert_eq!(host.hub.guests[0].advancements.has(ID, CRITERION), success);
            assert_eq!(CRAFTED.get(), 0);
        }
        let host = game.lan.as_mut().unwrap();
        host.hub.close_container(12);
        host.hub.guests.update(0, |guest| {
            guest.craft_grid[0] = item(LOG, 1);
            guest.carried = None;
        });
        game.lan_guest_packet(
            12,
            Serverbound::ContainerClick(ContainerClick {
                container: 0,
                state: 0,
                slot: 0,
                button: 0,
                input: 0,
                changed: Vec::new(),
                carried: None,
            }),
        );
        assert_eq!(
            CRAFTED.get(),
            1,
            "the vanilla inventory crafting notice is unchanged"
        );
        assert!(heard().is_empty());
    });
}
