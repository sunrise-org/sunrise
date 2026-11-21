use super::*;

#[test]
fn every_inventory_menu_slot_is_a_game_slot_and_45_is_the_off_hand() {
    // InventoryMenu: result, 2x2 grid, armour, 27 + 9 inventory slots, then the off hand.
    assert_eq!(Game::inventory_screen_slot("inventory", 45), Some(504));
    let mut seen = std::collections::HashSet::new();
    for slot in 0..46 {
        let internal = Game::inventory_screen_slot("inventory", slot);
        assert!(internal.is_some_and(|i| seen.insert(i)), "slot {slot}");
    }
    assert_eq!(Game::inventory_screen_slot("inventory", 46), None);
}

#[test]
fn remote_clicks_find_slots_past_the_old_large_chest_cutoff() {
    for (menu, internal, index) in [
        ("minecraft:generic_9x6", 18, 63),
        ("minecraft:generic_9x6", 19, 64),
        ("minecraft:generic_9x6", 0, 81),
        ("minecraft:generic_9x6", 8, 89),
        ("minecraft:generic_9x6", 353, 53),
        ("minecraft:generic_9x5", 8, 80),
        ("minecraft:generic_9x4", 8, 71),
        ("minecraft:generic_9x3", 8, 62),
        ("inventory", 504, 45),
    ] {
        assert_eq!(
            Game::menu_slot_index(menu, internal),
            Some(index),
            "{menu}, game slot {internal}"
        );
    }
    assert_eq!(Game::menu_slot_index("minecraft:generic_9x6", 354), None);
    assert_eq!(Game::menu_slot_index("minecraft:generic_9x6", 504), None);
    assert_eq!(Game::menu_slot_index("unknown", 0), None);
}
