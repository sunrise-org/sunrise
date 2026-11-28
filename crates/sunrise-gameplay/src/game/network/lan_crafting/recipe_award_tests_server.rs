//! A crafting take on the server path teaches the recipe: a single-player window's clicks
//! are a guest's, and `ResultSlot.checkTakeAchievements` ends in
//! `RecipeCraftingHolder.awardUsedRecipes`, which `ServerPlayer.awardRecipes` the recipe into
//! the guest's book (`ServerRecipeBook.addRecipes`: known, highlighted, and sent to the client).

use super::*;
use sunrise_protocol::packets::play::{ContainerClick, Serverbound};
use sunrise_sim::lan::menu::input;

/// A right-click on the inventory grid's result: the recipe the book did not know becomes
/// known, is highlighted and queued for the client, and `Stats.ITEM_CRAFTED` counts the two
/// planks the click asked for while the guest carries all four.
#[test]
#[ignore = "daily"]
fn a_guest_crafting_take_teaches_the_recipe_and_counts_what_it_asked_for() {
    const PLANKS: &str = "minecraft:oak_planks";
    let mut resources = None;
    let mut fixture = crate::game::game_tests::tests::server_game(&mut resources);
    let game = fixture.game_mut();
    let _rx = crate::game::server_side::lan::world_spawn_sync_tests::host_for(game);
    let host = game.server.lan.as_mut().expect("a LAN host");
    let conn = host.hub.guests[0].conn;
    assert!(
        !host.hub.guests[0].recipe_book.knows(PLANKS),
        "the book starts without planks"
    );
    host.hub.guests.update(0, |g| {
        g.craft_grid[0] = Some(ItemStack::new("minecraft:oak_log", 1));
    });
    game.lan_guest_packet(
        conn,
        Serverbound::ContainerClick(ContainerClick {
            container: 0,
            state: 0,
            slot: 0,
            button: 1,
            input: input::PICKUP,
            changed: Vec::new(),
            carried: None,
        }),
    );
    let host = game.server.lan.as_ref().expect("the host back");
    let guest = &host.hub.guests[0];
    assert_eq!(
        guest.carried,
        Some(ItemStack::new(PLANKS, 4)),
        "the whole result"
    );
    assert!(guest.recipe_book.knows(PLANKS), "the recipe is learned");
    assert!(
        guest.recipe_book.to_be_displayed.contains(PLANKS),
        "the new recipe is highlighted"
    );
    assert!(
        guest.recipes_unlocked.iter().any(|r| r == PLANKS),
        "the client hears of it (the toast)"
    );
    let stats = &host.play.get(&conn).expect("the guest").stats;
    assert_eq!(stats.get("crafted", PLANKS), 2, "removeCount: half of four");
}
