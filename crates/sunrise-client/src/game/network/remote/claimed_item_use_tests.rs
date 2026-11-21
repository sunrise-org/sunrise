use super::{C, Game, PlayerMotion, S, Vec3, play};
use crate::game::{
    Hit, Screen,
    native_entity_refs::tests::{with_client_game, with_game},
    native_hotbar_slot::hotbar_slot_tests::{logged_in, sent},
};
use mod_api::{ClientHost, ClientItemUseBehavior, Hooks, ItemKey, Registrar, RegistrationContext};
use std::sync::{Arc, Mutex};
use sunrise_client::{chunk::Chunk, interaction_input::Button, items::ItemStack};
use sunrise_core::nbt::{JavaString, Tag};

const OWNER: &str = "client_use_fixture";
const SHIELD: &str = "minecraft:shield";
const STICK: &str = "minecraft:stick";
const CUSTOM_DATA: &str = "minecraft:custom_data";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    Start,
    Release(u32),
    Stop(u32),
}

#[derive(Debug, PartialEq, Eq)]
struct Call {
    phase: Phase,
    registered_for: &'static str,
    item: ItemKey,
    slot: u32,
    count: u32,
    marker: Option<Tag>,
}

#[derive(Clone, Default)]
struct Calls(Arc<Mutex<Vec<Call>>>);

impl Calls {
    fn take(&self) -> Vec<Call> {
        std::mem::take(&mut *self.0.lock().unwrap())
    }
}

#[mod_api::__subscriptions]
impl Hooks for Calls {
    fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
        for registered_for in [SHIELD, STICK] {
            let item = r.item_key(registered_for);
            r.register_client_item_use(
                item,
                Arc::new(RecordUse {
                    registered_for,
                    calls: self.clone(),
                }),
            );
        }
    }
}

struct RecordUse {
    registered_for: &'static str,
    calls: Calls,
}

impl RecordUse {
    fn record(&self, phase: Phase, stack: &mod_api::ItemStack, slot: u32) {
        let call = mod_api::content::crash::current_native_call().unwrap();
        assert_eq!(call.id, OWNER);
        assert_eq!(
            call.hook,
            match phase {
                Phase::Start => "use_item",
                Phase::Release(_) => "release_using",
                Phase::Stop(_) => "on_stop_using",
            }
        );
        let marker = stack
            .components()
            .and_then(|components| components.as_nbt().get(&JavaString::from(CUSTOM_DATA)))
            .and_then(|data| data.get("marker"))
            .cloned();
        self.calls.0.lock().unwrap().push(Call {
            phase,
            registered_for: self.registered_for,
            item: stack.item(),
            slot,
            count: stack.count(),
            marker,
        });
    }
}

impl ClientItemUseBehavior for RecordUse {
    fn use_item(&self, _: &mut dyn ClientHost, stack: &mod_api::ItemStack, slot: u32) {
        self.record(Phase::Start, stack, slot);
    }

    fn release_using(
        &self,
        _: &mut dyn ClientHost,
        stack: &mod_api::ItemStack,
        slot: u32,
        ticks_held: u32,
    ) {
        self.record(Phase::Release(ticks_held), stack, slot);
    }

    fn on_stop_using(
        &self,
        _: &mut dyn ClientHost,
        stack: &mod_api::ItemStack,
        slot: u32,
        ticks_held: u32,
    ) {
        self.record(Phase::Stop(ticks_held), stack, slot);
    }
}

fn with_owner(test: impl FnOnce(&Calls)) {
    let calls = Calls::default();
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(OWNER, Box::new(calls.clone()))].into();
    crate::game::api_values::install_parts(&mods).unwrap();
    mod_platform::testing::with_mods(mods, || test(&calls));
}

#[test]
#[ignore = "daily"]
fn interception_does_not_send_item_use_or_release_packets() {
    with_owner(|calls| {
        with_client_game(|game| {
            let mut reader = logged_in(game);
            game.screen = Screen::Playing;
            game.remote.as_mut().unwrap().loaded_sent = true;

            // An owner for the off-hand kind does not suppress normal off-hand use or release.
            game.offhand = Some(ItemStack::new(SHIELD, 1));
            game.right = true;
            game.remote.as_mut().unwrap().input.press(Button::Use);
            game.remote_handle_input();
            assert!(game.using.is_some());
            assert!(
                sent(&mut reader)
                    .iter()
                    .any(|packet| matches!(packet, S::UseItem(play::UseItem { hand: 1, .. })))
            );
            game.right = false;
            game.remote_handle_input();
            assert!(game.using.is_none());
            assert!(sent(&mut reader).iter().any(|packet| matches!(
                packet,
                S::PlayerAction(play::PlayerAction { action: 6, .. })
            )));
            assert!(calls.take().is_empty());

            // Now target a block with the registered kind in the main hand. Two presses in one
            // input drain both reach start-use handling, but only the first can capture it.
            game.inventory[game.selected] = game.offhand.take();
            let stone = game
                .pack
                .assets
                .registry
                .default_state("minecraft:stone")
                .unwrap();
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            game.world.set(0, 64, 0, stone).unwrap();
            game.forced_hit = Some(Hit {
                pos: [0, 64, 0],
                face: [0, 1, 0],
                point: [0.5, 65.0, 0.5],
                inside: false,
            });
            game.right = true;
            let input = &mut game.remote.as_mut().unwrap().input;
            input.press(Button::Use);
            input.press(Button::Use);
            game.remote_handle_input();
            assert!(game.client_item_use.is_some());
            assert!(game.using.is_none());

            game.use_delay = 0;
            game.remote_handle_input();
            game.remote_tick_using();
            assert!(game.using.is_none());
            assert!(game.remote.as_ref().unwrap().use_item.is_none());

            game.right = false;
            game.remote_handle_input();
            assert!(game.client_item_use.is_none());
            assert!(game.using.is_none());
            assert_eq!(
                calls
                    .take()
                    .into_iter()
                    .map(|call| call.phase)
                    .collect::<Vec<_>>(),
                [Phase::Start, Phase::Release(1), Phase::Stop(1)]
            );
            let packets = sent(&mut reader);
            assert!(
                !packets.iter().any(|packet| matches!(
                    packet,
                    S::UseItem(_)
                        | S::UseItemOn(_)
                        | S::Interact(_)
                        | S::PlayerAction(play::PlayerAction { action: 6, .. })
                )),
                "interception must never send use or release packets: {packets:?}"
            );
        });
    });
}

#[test]
#[ignore = "daily"]
fn owner_gets_release_or_reset_with_the_original_typed_stack() {
    fn marked(item: &str, marker: Tag) -> ItemStack {
        let mut stack = ItemStack::new(item, 1);
        stack
            .data_mut()
            .set_other(CUSTOM_DATA, Some(Tag::compound([("marker", marker)])));
        stack
    }
    fn start(game: &mut Game<'_>) {
        game.select_hotbar_slot(3).unwrap();
        game.inventory[3] = Some(marked(SHIELD, Tag::Byte(23)));
        game.right = true;
        game.remote.as_mut().unwrap().input.press(Button::Use);
        game.remote_handle_input();
        assert!(game.client_item_use.is_some());
        assert!(game.using.is_none());
    }
    fn key_up(game: &mut Game<'_>) {
        game.right = false;
        game.remote_handle_input();
        assert!(game.client_item_use.is_none());
    }

    with_owner(|calls| {
        with_client_game(|game| {
            let _peer = logged_in(game);
            game.screen = Screen::Playing;
            game.remote.as_mut().unwrap().loaded_sent = true;
            let shield = mod_api::__private::item_key(SHIELD).unwrap();
            let original = |phase| Call {
                phase,
                registered_for: SHIELD,
                item: shield,
                slot: 3,
                count: 1,
                marker: Some(Tag::Byte(23)),
            };

            start(game);
            assert_eq!(calls.take(), [original(Phase::Start)]);
            game.remote_tick_using();
            game.inventory[3] = Some(marked(SHIELD, Tag::Int(99)));
            game.remote_tick_using();
            key_up(game);
            game.reset_client_item_use();
            assert_eq!(
                calls.take(),
                [original(Phase::Release(2)), original(Phase::Stop(2))]
            );

            // Another registered item (even for the same mod) cannot receive this use's end.
            start(game);
            game.remote_tick_using();
            game.inventory[3] = Some(marked(STICK, Tag::Int(99)));
            key_up(game);
            game.reset_client_item_use();
            assert_eq!(
                calls.take(),
                [original(Phase::Start), original(Phase::Stop(1))]
            );

            // The same kind in a different slot is a reset, even on the key-up tick.
            start(game);
            game.inventory[4] = Some(marked(SHIELD, Tag::Int(99)));
            game.select_hotbar_slot(4).unwrap();
            key_up(game);
            game.reset_client_item_use();
            assert_eq!(
                calls.take(),
                [original(Phase::Start), original(Phase::Stop(0))]
            );

            start(game);
            assert!(mod_api::__private::set_mod_enabled(OWNER, false));
            key_up(game);
            assert_eq!(calls.take(), [original(Phase::Start)]);
            assert!(mod_api::__private::set_mod_enabled(OWNER, true));

            // A keybind opening a screen clears the mapping without an input release.
            start(game);
            game.release_use_for_keybind_screen();
            key_up(game);
            assert_eq!(
                calls.take(),
                [original(Phase::Start), original(Phase::Stop(0))]
            );

            // The death packet retires the capture without waiting for another input tick.
            start(game);
            game.handle_packet(C::SetHealth(play::SetHealth {
                health: 0.0,
                food: 20,
                saturation: 5.0,
            }))
            .unwrap();
            assert!(game.client_item_use.is_none());
            game.reset_client_item_use();
            assert_eq!(
                calls.take(),
                [original(Phase::Start), original(Phase::Stop(0))]
            );
        });

        with_game(|game| {
            game.player = Some(PlayerMotion::new(game.motion, Vec3::new(0.5, 65.0, 0.5)).unwrap());
            game.inventory[game.selected] = Some(marked(SHIELD, Tag::Byte(23)));
            assert!(game.use_hand(None).unwrap());
            assert_eq!(game.using, Some((game.selected, 0)));
            assert!(
                calls.take().is_empty(),
                "normal server item use has no client owner"
            );
        });
    });
}
