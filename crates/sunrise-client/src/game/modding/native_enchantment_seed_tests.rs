//! `PlayerRef::enchantment_seed` (`Player.getEnchantmentSeed`): once a player opens an
//! enchanting table, the read equals the seed the enchantment menu seeds its offers with.

use super::native_guest_turn_tests::with_guests;
use crate::game::stations::{Kind as StationKind, Station};
use mod_api::Host;

/// The seed is made when the table first opens, and the read gives the value the menu holds
/// in its `enchantmentSeed` data slot (index 3) for that player.
#[test]
#[ignore = "daily"]
fn the_read_equals_the_seed_the_enchantment_menu_uses_after_opening_a_table() {
    crate::game::in_own_process(
        "game::native_enchantment_seed_tests::the_read_equals_the_seed_the_enchantment_menu_uses_after_opening_a_table",
        || {
            with_guests(|game| {
                game.options.dedicated = Some(Vec::new());
                let player = game.player(12).unwrap();
                let host: &dyn Host = &*game;
                assert_eq!(
                    player.enchantment_seed(host),
                    Some(0),
                    "no table opened yet"
                );

                assert!(
                    game.lan_open_station(12, &Station::new(StationKind::Enchanting, [1, 64, 1]))
                );
                let menu_seed = game.lan.as_ref().unwrap().menus[&12]
                    .station
                    .as_ref()
                    .unwrap()
                    .menu
                    .data[3];
                let host: &dyn Host = &*game;
                assert_ne!(menu_seed, 0);
                assert_eq!(player.enchantment_seed(host), Some(menu_seed));
            });
        },
    );
}
