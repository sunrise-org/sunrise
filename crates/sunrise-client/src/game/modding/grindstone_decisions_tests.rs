//! `grindstone_place` and `grindstone_take` through real listeners: both run as decided actions
//! on a guest's workstation menu, with the same savepoint and refusal as the other menu hooks.
//! The fixtures need prepared resources.
use crate::game::{
    native_guest_turn_tests::with_guests,
    stations::{Kind as StationKind, Station},
};
use mod_api::{
    Flow, GrindstonePlaceEvent, GrindstoneTakeEvent, Hooks, HostBlockStates, ItemPlace,
    ItemResource, ModEntry, Mods, Registrar, ServerHost,
};
use std::cell::{Cell, RefCell};
use sunrise_client::items::ItemStack;
use sunrise_protocol::packets::play::{ContainerClick, Serverbound};

#[derive(Clone, Default)]
struct Script {
    place_output: bool,
    place_cancel: bool,
    place_write: bool,
    place_bad_xp: bool,
    replace_backing: bool,
    place_panic: bool,
    take_panic: bool,
    take_write: bool,
    take_cancel: bool,
    take_bad_xp: bool,
}

thread_local! {
    static SCRIPT: RefCell<Script> = RefCell::new(Script::default());
    static DISPATCHES: Cell<usize> = const { Cell::new(0) };
    static TAKE_XP: Cell<Option<i32>> = const { Cell::new(None) };
    static REPLACED: Cell<bool> = const { Cell::new(false) };
}

fn script(script: Script) {
    DISPATCHES.set(0);
    TAKE_XP.set(None);
    REPLACED.set(false);
    SCRIPT.with(|s| *s.borrow_mut() = script);
}

fn dirt(count: u32) -> mod_api::ItemStack {
    mod_api::ItemStack::new(
        mod_api::__private::item_key("minecraft:dirt").unwrap(),
        count,
    )
    .unwrap()
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

fn replace_backing(host: &mut dyn ServerHost) {
    let pos = [1, 64, 1];
    let level = host.level("minecraft:overworld").unwrap();
    let original = level.block_state(host, pos).unwrap();
    let air = mod_api::BlockState::of(
        &host.block_states(),
        mod_api::__private::block_key("minecraft:air").unwrap(),
        &[],
    )
    .unwrap();
    let flags = mod_api::BlockUpdateFlags::KNOWN_SHAPE;
    assert_eq!(level.set_block(host, pos, air, flags), Ok(true));
    assert_eq!(level.set_block(host, pos, original, flags), Ok(true));
    assert_eq!(level.block_state(host, pos), Some(original));
    REPLACED.set(true);
}

struct Mod;

#[mod_api::__subscriptions]
impl Hooks for Mod {
    fn register(&self, _: &mut dyn Registrar, _: &mod_api::RegistrationContext) {}

    fn grindstone_place(
        &self,
        host: &mut dyn ServerHost,
        event: &mut GrindstonePlaceEvent,
    ) -> mod_api::Flow {
        DISPATCHES.set(DISPATCHES.get() + 1);
        assert!(host.transaction_current().is_some());
        let s = SCRIPT.with(|s| s.borrow().clone());
        if s.replace_backing {
            replace_backing(host);
        }
        if s.place_write {
            write_dirt(host);
        }
        if s.place_output {
            event.place.output = Some(dirt(1));
            event.place.xp = 5;
        }
        if s.place_bad_xp {
            event.place.xp = -2;
        }
        if s.place_cancel {
            event.cancelled = true;
        }
        if s.place_panic {
            panic!("a place listener failed");
        }
        Flow::Continue
    }

    fn grindstone_take(
        &self,
        host: &mut dyn ServerHost,
        event: &mut GrindstoneTakeEvent,
    ) -> mod_api::Flow {
        DISPATCHES.set(DISPATCHES.get() + 1);
        TAKE_XP.set(Some(event.take.xp));
        assert!(host.transaction_current().is_some());
        let s = SCRIPT.with(|s| s.borrow().clone());
        if s.replace_backing {
            replace_backing(host);
        }
        if s.take_write {
            write_dirt(host);
        }
        if s.take_bad_xp {
            event.take.xp = -2;
        }
        if s.take_cancel {
            event.cancelled = true;
        }
        if s.take_panic {
            panic!("a take listener failed");
        }
        Flow::Continue
    }
}

fn with_mod(test: impl FnOnce()) {
    let mods: Mods = vec![ModEntry::new("grindmod", Box::new(Mod))].into();
    mod_platform::testing::with_mods(mods, || {
        assert!(mod_api::__private::listened(
            mod_api::HookId::GrindstonePlace
        ));
        assert!(mod_api::__private::listened(
            mod_api::HookId::GrindstoneTake
        ));
        test();
    });
}

fn without_mod(test: impl FnOnce()) {
    let mods: Mods = Vec::new().into();
    mod_platform::testing::with_mods(mods, test);
}

fn engine_item(game: &mut crate::game::Game<'_>, id: &str, count: u32) -> ItemStack {
    let key = mod_api::__private::item_key(id).unwrap();
    {
        use crate::game::api_values::*;
        mod_api::ItemStack::new(key, count)
            .unwrap()
            .try_into_engine(game.items())
    }
    .ok()
    .unwrap()
}

fn open_grindstone(game: &mut crate::game::Game<'_>) -> i32 {
    // ContainerClick checks stillValid before routing to the workstation. Give the menu a
    // real block, as the other workstation fixtures do, so that check keeps it open.
    game.world.chunks.insert(
        (0, 0),
        std::sync::Arc::new(sunrise_client::chunk::Chunk::empty(0, 0)),
    );
    let state = game
        .pack
        .assets
        .registry
        .default_state("minecraft:grindstone")
        .unwrap();
    game.world.set(1, 64, 1, state).unwrap();
    assert!(game.lan_open_station(12, &Station::new(StationKind::Grindstone, [1, 64, 1])));
    let host = game.lan.as_ref().unwrap();
    assert!(game.lan_station_valid(
        &host.hub.guests[0],
        host.menus[&12].station.as_ref().unwrap(),
    ));
    game.lan.as_ref().unwrap().menus[&12]
        .station
        .as_ref()
        .unwrap()
        .window
}

fn enchanted_sword(game: &mut crate::game::Game<'_>, damage: u32) -> ItemStack {
    let mut sword = engine_item(game, "minecraft:iron_sword", 1);
    sword.damage = damage;
    sword.set_crafting_enchantments(vec![("minecraft:sharpness".into(), 3)]);
    sword
}

fn slot(game: &crate::game::Game<'_>, index: usize) -> Option<ItemStack> {
    game.lan.as_ref().unwrap().menus[&12]
        .station
        .as_ref()
        .unwrap()
        .menu
        .slot(index)
        .cloned()
}

fn count(stack: &Option<ItemStack>) -> u32 {
    stack.as_ref().map_or(0, |s| s.count)
}

fn inventory(game: &crate::game::Game<'_>, index: usize) -> u32 {
    count(&game.lan.as_ref().unwrap().hub.guests[0].inventory[index])
}

fn container_click(window: i32, slot: i16, button: i8, input: i32) -> Serverbound {
    Serverbound::ContainerClick(ContainerClick {
        state: 0,
        changed: Vec::new(),
        carried: None,
        container: window,
        slot,
        button,
        input,
    })
}

fn click(game: &mut crate::game::Game<'_>, window: i32, slot: i16, button: i8) {
    game.lan_guest_packet(12, container_click(window, slot, button, 0));
}

#[test]
#[ignore = "daily"]
fn a_place_listener_sets_the_grindstone_result() {
    with_mod(|| {
        with_guests(|game| {
            let window = open_grindstone(game);
            let sword = enchanted_sword(game, 200);
            game.lan.as_mut().unwrap().hub.guests.update(0, |g| {
                g.carried = Some(sword);
            });
            script(Script {
                place_output: true,
                ..Script::default()
            });
            click(game, window, 0, 0);
            assert_eq!(DISPATCHES.get(), 1);
            assert_eq!(slot(game, 2), Some(ItemStack::new("minecraft:dirt", 1)));
            click(game, window, 2, 0);
            assert_eq!(TAKE_XP.get(), Some(5));
            assert_eq!(
                game.lan.as_ref().unwrap().hub.guests[0].carried,
                Some(ItemStack::new("minecraft:dirt", 1))
            );
        });
    });
}

#[test]
#[ignore = "daily"]
fn a_cancelled_place_leaves_no_grindstone_result() {
    with_mod(|| {
        with_guests(|game| {
            let window = open_grindstone(game);
            let sword = enchanted_sword(game, 200);
            game.lan.as_mut().unwrap().hub.guests.update(0, |g| {
                g.carried = Some(sword);
            });
            script(Script {
                place_cancel: true,
                ..Script::default()
            });
            click(game, window, 0, 0);
            assert_eq!(DISPATCHES.get(), 1);
            assert_eq!(count(&slot(game, 2)), 0);
        });
    });
}

#[test]
#[ignore = "daily"]
fn a_stale_place_is_refused_and_the_listener_write_undone() {
    with_mod(|| {
        with_guests(|game| {
            let window = open_grindstone(game);
            let sword = enchanted_sword(game, 200);
            game.lan.as_mut().unwrap().hub.guests.update(0, |g| {
                g.carried = Some(sword);
            });
            script(Script {
                place_output: true,
                place_write: true,
                ..Script::default()
            });
            click(game, window, 0, 0);
            assert_eq!(DISPATCHES.get(), 1);
            assert_eq!(count(&slot(game, 2)), 0);
            assert_eq!(inventory(game, 20), 0);
            assert!(game.transactions.is_empty());
        });
    });
}

#[test]
#[ignore = "daily"]
fn equal_state_backing_replacement_refuses_place_and_take() {
    with_mod(|| {
        with_guests(|game| {
            let _serving = crate::game::integrated::tests::install_serving(game, false);
            let window = open_grindstone(game);
            let backing = game.world.block(1, 64, 1);
            let sword = enchanted_sword(game, 200);
            script(Script {
                place_output: true,
                replace_backing: true,
                ..Script::default()
            });
            game.lan.as_mut().unwrap().hub.guests.update(0, |g| {
                g.carried = Some(sword.clone());
            });
            click(game, window, 0, 0);
            assert_eq!(DISPATCHES.get(), 1);
            assert!(REPLACED.get());
            assert_eq!(game.world.block(1, 64, 1), backing);
            assert_eq!(slot(game, 0), Some(sword.clone()));
            assert_eq!(slot(game, 2), None, "a stale place cannot set the result");

            script(Script {
                place_output: true,
                ..Script::default()
            });
            click(game, window, 0, 0);
            click(game, window, 0, 0);
            assert_eq!(slot(game, 2), Some(ItemStack::new("minecraft:dirt", 1)));

            script(Script {
                replace_backing: true,
                ..Script::default()
            });
            click(game, window, 2, 0);
            assert_eq!(TAKE_XP.get(), Some(5));
            assert!(REPLACED.get());
            assert_eq!(game.world.block(1, 64, 1), backing);
            assert_eq!(slot(game, 0), Some(sword));
            assert_eq!(slot(game, 2), Some(ItemStack::new("minecraft:dirt", 1)));
            assert_eq!(game.lan.as_ref().unwrap().hub.guests[0].carried, None);
            assert!(game.orbs.is_empty());
        });
    });
}

#[test]
#[ignore = "daily"]
fn an_invalid_place_answer_is_refused() {
    with_mod(|| {
        with_guests(|game| {
            let window = open_grindstone(game);
            let sword = enchanted_sword(game, 200);
            game.lan.as_mut().unwrap().hub.guests.update(0, |g| {
                g.carried = Some(sword);
            });
            script(Script {
                place_bad_xp: true,
                ..Script::default()
            });
            click(game, window, 0, 0);
            assert_eq!(DISPATCHES.get(), 1);
            assert_eq!(count(&slot(game, 2)), 0);
            assert!(game.transactions.is_empty());
        });
    });
}

#[test]
#[ignore = "daily"]
fn a_panicking_take_listener_leaves_the_grindstone_as_it_was() {
    with_mod(|| {
        with_guests(|game| {
            let window = open_grindstone(game);
            let sword = enchanted_sword(game, 200);
            let original = sword.clone();
            script(Script {
                place_output: true,
                ..Script::default()
            });
            game.lan.as_mut().unwrap().hub.guests.update(0, |g| {
                g.carried = Some(sword);
            });
            click(game, window, 0, 0);
            script(Script {
                take_panic: true,
                ..Script::default()
            });
            click(game, window, 2, 0);
            assert_eq!(DISPATCHES.get(), 1);
            assert_eq!(slot(game, 2), Some(ItemStack::new("minecraft:dirt", 1)));
            assert_eq!(slot(game, 0), Some(original));
            assert_eq!(slot(game, 1), None);
            let carried = game.lan.as_ref().unwrap().hub.guests[0].carried.clone();
            assert_eq!(count(&carried), 0);
            assert!(game.transactions.is_empty());
        });
    });
}

#[test]
#[ignore = "daily"]
fn a_grindstone_without_a_listener_keeps_the_vanilla_result() {
    without_mod(|| {
        with_guests(|game| {
            assert!(!mod_api::__private::listened(
                mod_api::HookId::GrindstonePlace
            ));
            assert!(!mod_api::__private::listened(
                mod_api::HookId::GrindstoneTake
            ));
            let window = open_grindstone(game);
            let a = enchanted_sword(game, 200);
            let b = enchanted_sword(game, 150);
            game.lan.as_mut().unwrap().hub.guests.update(0, |g| {
                g.carried = Some(a);
            });
            click(game, window, 0, 0);
            game.lan.as_mut().unwrap().hub.guests.update(0, |g| {
                g.carried = Some(b);
            });
            click(game, window, 1, 0);
            assert!(
                game.lan
                    .as_ref()
                    .unwrap()
                    .menus
                    .get(&12)
                    .is_some_and(|p| p.station.is_some()),
                "the grindstone station is gone after the click"
            );
            // Iron swords have 250 durability: 50 + 100 remaining + floor(250 * .05)
            // leaves 162 durability, i.e. damage 88. Sharpness is removed.
            let mut expected = ItemStack::new("minecraft:iron_sword", 1);
            expected.damage = 88;
            assert_eq!(slot(game, 2), Some(expected));
        });
    });
}

/// A sword in the grindstone's top input with a place listener's result in the output, ready for
/// the take click.
fn placed(game: &mut crate::game::Game<'_>) -> (i32, ItemStack) {
    let window = open_grindstone(game);
    let sword = enchanted_sword(game, 200);
    let original = sword.clone();
    script(Script {
        place_output: true,
        ..Script::default()
    });
    game.lan.as_mut().unwrap().hub.guests.update(0, |g| {
        g.carried = Some(sword);
    });
    click(game, window, 0, 0);
    (window, original)
}

#[test]
#[ignore = "daily"]
fn a_cancelled_take_keeps_the_click_and_undoes_the_listener_write() {
    with_mod(|| {
        with_guests(|game| {
            let (window, original) = placed(game);
            script(Script {
                take_write: true,
                take_cancel: true,
                ..Script::default()
            });
            click(game, window, 2, 0);
            // The result is still taken and the inputs stay, but the listener's write is undone.
            assert_eq!(
                game.lan.as_ref().unwrap().hub.guests[0].carried,
                Some(ItemStack::new("minecraft:dirt", 1))
            );
            assert_eq!(slot(game, 0), Some(original));
            assert_eq!(inventory(game, 20), 0);
            assert!(game.transactions.is_empty());
        });
    });
}

#[test]
#[ignore = "daily"]
fn a_refused_quick_move_take_puts_the_moved_result_back() {
    with_mod(|| {
        with_guests(|game| {
            let (window, original) = placed(game);
            script(Script {
                take_bad_xp: true,
                ..Script::default()
            });
            game.lan_guest_packet(12, container_click(window, 2, 0, 1));
            let moved: u32 = (0..36).map(|i| inventory(game, i)).sum();
            assert_eq!(moved, 0, "the result stays in the grindstone's output");
            assert_eq!(slot(game, 2), Some(ItemStack::new("minecraft:dirt", 1)));
            assert_eq!(slot(game, 0), Some(original));
            assert!(game.transactions.is_empty());
        });
    });
}

#[test]
#[ignore = "daily"]
fn an_outer_abort_undoes_an_accepted_grindstone_take() {
    use std::sync::{Arc, mpsc};
    use sunrise_client::lan::PlayerSender;
    use sunrise_core::{
        geometry::Vec3,
        random::{LegacyRandom, RandomSource},
    };

    with_mod(|| {
        with_guests(|game| {
            let (window, original) = placed(game);
            // Five XP splits into 3, 1, 1. The 3-XP piece merges into this resident orb.
            *game.world.random = LegacyRandom::new(23);
            let mut expected_random = (*game.world.random).clone();
            let matching_id = 40 + expected_random.next_int_bound(40).unwrap() as u32;
            game.next_entity = matching_id - 1;
            game.add_orb([1.5, 64.5, 1.5], 3);
            let orb = &mut game.orbs[0];
            orb.position = Vec3::new(1.5, 64.5, 1.5);
            orb.age = 42;
            let join = orb.join;
            let (tx, rx) = mpsc::channel();
            game.lan.as_mut().unwrap().hub.guests.update(0, |g| {
                g.sender = PlayerSender::new(tx, Arc::default());
            });
            let events = game.lan_events.len();
            script(Script::default());

            for commit in [false, true] {
                let outer = game.transaction_open();
                click(game, window, 2, 0);
                assert_eq!(slot(game, 0), None);
                assert_eq!(
                    game.lan.as_ref().unwrap().hub.guests[0].carried,
                    Some(ItemStack::new("minecraft:dirt", 1))
                );
                assert_eq!(game.orbs[game.orbs.locate(join).unwrap()].count, 2);
                assert!(
                    rx.try_recv().is_err(),
                    "published station content before commit"
                );
                assert_eq!(
                    game.lan_events.len(),
                    events,
                    "published level event before commit"
                );
                game.transaction_close(&outer, commit);
                if !commit {
                    assert_eq!(slot(game, 0), Some(original.clone()));
                    assert_eq!(slot(game, 2), Some(ItemStack::new("minecraft:dirt", 1)));
                    assert_eq!(game.lan.as_ref().unwrap().hub.guests[0].carried, None);
                    assert_eq!(game.orbs.len(), 1);
                    let orb = &game.orbs[game.orbs.locate(join).unwrap()];
                    assert_eq!((orb.count, orb.age), (1, 42));
                    assert_eq!(
                        (*game.world.random).clone().next_int(),
                        LegacyRandom::new(23).next_int()
                    );
                    assert!(rx.try_recv().is_err(), "published station content on abort");
                    assert_eq!(
                        game.lan_events.len(),
                        events,
                        "published level event on abort"
                    );
                } else {
                    assert_eq!(
                        game.orbs
                            .iter()
                            .map(|orb| orb.value as u32 * orb.count)
                            .sum::<u32>(),
                        8
                    );
                    assert!(
                        rx.try_recv().is_ok(),
                        "did not publish station content on commit"
                    );
                    assert_eq!(game.lan_events.len(), events + 1);
                }
            }
        });
    });
}

#[test]
#[ignore = "daily"]
fn panicking_grindstone_listeners_roll_back_place_and_take() {
    with_mod(|| {
        with_guests(|game| {
            let window = open_grindstone(game);
            let sword = enchanted_sword(game, 200);
            game.lan.as_mut().unwrap().hub.guests.update(0, |g| {
                g.carried = Some(sword.clone());
            });
            script(Script {
                place_output: true,
                place_write: true,
                place_panic: true,
                ..Script::default()
            });
            click(game, window, 0, 0);
            assert_eq!(DISPATCHES.get(), 1);
            // A failed createResult does not undo the already placed input.
            assert_eq!(slot(game, 0), Some(sword.clone()));
            assert_eq!(slot(game, 2), None);
            assert_eq!(inventory(game, 20), 0);
            assert!(game.transactions.is_empty());

            script(Script::default());
            click(game, window, 0, 0);
            script(Script {
                place_output: true,
                ..Script::default()
            });
            click(game, window, 0, 0);
            let output = Some(ItemStack::new("minecraft:dirt", 1));
            assert_eq!(slot(game, 2), output);

            let events = game.lan_events.len();
            script(Script {
                take_write: true,
                take_panic: true,
                ..Script::default()
            });
            click(game, window, 2, 0);
            assert_eq!(TAKE_XP.get(), Some(5));
            assert_eq!(DISPATCHES.get(), 1);
            assert_eq!(slot(game, 0), Some(sword));
            assert_eq!(slot(game, 2), output);
            assert_eq!(game.lan.as_ref().unwrap().hub.guests[0].carried, None);
            assert_eq!(inventory(game, 20), 0);
            assert_eq!(game.lan_events.len(), events);
            assert!(game.orbs.is_empty());
            assert!(game.transactions.is_empty());

            script(Script::default());
            click(game, window, 2, 0);
            assert_eq!(game.lan.as_ref().unwrap().hub.guests[0].carried, output);
            assert!(!game.orbs.is_empty());
        });
    });
}

#[test]
#[ignore = "daily"]
fn a_refused_throw_take_does_not_drop_the_result() {
    with_mod(|| {
        with_guests(|game| {
            let (window, original) = placed(game);
            script(Script {
                take_bad_xp: true,
                ..Script::default()
            });
            game.lan_guest_packet(12, container_click(window, 2, 0, 4));
            assert_eq!(TAKE_XP.get(), Some(5), "the throw reached onTake");
            assert!(game.dropped.is_empty());
            assert_eq!(slot(game, 2), Some(ItemStack::new("minecraft:dirt", 1)));
            assert_eq!(slot(game, 0), Some(original));
        });
    });
}
