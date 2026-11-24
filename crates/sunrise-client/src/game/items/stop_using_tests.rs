#![cfg(test)]
//! A golden apple's `ItemPart` hears `on_stop_using`
//! (`IItemExtension.onStopUsing`) once per stop, whatever stopped the use:
//! a finish, a release or another item in the hand.
use super::Game;
use crate::game::Screen;
use mod_api::{Hand, Hooks, HostSide, ItemPart, OnStopUsingEvent, Registrar, RegistrationContext};
use std::sync::Mutex;
use sunrise_client::items::ItemStack;

/// What the part heard: `(on the server, entity, main hand, time left)`.
static HEARD: Mutex<Vec<(bool, Option<i32>, bool, i32)>> = Mutex::new(Vec::new());

#[derive(Clone)]
struct Apple;
#[mod_api::sunrise_part]
impl ItemPart for Apple {
    fn on_stop_using(&self, host: HostSide, event: &mut OnStopUsingEvent<'_>) {
        HEARD.lock().unwrap().push((
            matches!(host, HostSide::Server(_)),
            event.entity,
            matches!(event.hand, Hand::Main),
            event.time_left,
        ));
    }
}
struct Parts;
#[mod_api::__subscriptions]
impl Hooks for Parts {
    fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
        let key = r.item_key("minecraft:golden_apple");
        r.item_part(key, Apple);
    }
}

fn heard() -> Vec<(bool, Option<i32>, bool, i32)> {
    HEARD.lock().unwrap().drain(..).collect()
}

/// Starts eating a golden apple with the part on, ticks the use five times,
/// then runs `body`.
fn eating(body: impl FnOnce(&mut Game<'_>)) {
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new("apple", Box::new(Parts))].into();
    mod_platform::testing::with_mods(mods.clone(), || {
        crate::game::api_values::install_parts(&mods).unwrap();
        super::item_use::interact_vibration_tests::with_game("minecraft:golden_apple", |game| {
            game.creative = false;
            game.build_registries();
            game.screen = Screen::Playing;
            game.set_mapping_held("key.use", true);
            assert!(game.start_using_item());
            for _ in 0..5 {
                game.tick_using();
            }
            assert_eq!(game.using, Some((game.selected, 5)));
            assert!(heard().is_empty(), "a use going on isn't a stop");
            body(game);
        })
        .expect("prepared resources");
    });
}

#[test]
#[ignore = "daily"]
fn a_part_hears_each_stop_of_its_items_use_once() {
    crate::game::in_own_process(
        "game::stop_using_tests::a_part_hears_each_stop_of_its_items_use_once",
        || {
            // A finished use: 0 ticks left, heard once the apple is eaten.
            eating(|game| {
                for _ in 0..64 {
                    if game.using.is_none() {
                        break;
                    }
                    game.tick_using();
                }
                assert!(game.using.is_none());
                assert_eq!(heard(), [(true, None, true, 0)]);
            });
            // A release: the ticks of the use that were left.
            eating(|game| {
                let left = game.use_duration - 5;
                game.set_mapping_held("key.use", false);
                game.tick_using();
                assert!(game.using.is_none());
                assert_eq!(heard(), [(true, None, true, left)]);
                assert_eq!(game.held().map(|s| s.count), Some(1), "not eaten");
            });
            // Another item in the hand: the apple's use stops, and the apple's part hears it.
            eating(|game| {
                let left = game.use_duration - 5;
                *game.held_mut() = Some(ItemStack::new("minecraft:bow", 1));
                game.tick_using();
                assert!(game.using.is_none());
                assert_eq!(heard(), [(true, None, true, left)]);
                // Nothing left to stop: no second stop.
                game.tick_using();
                assert!(heard().is_empty());
            });
        },
    );
}
