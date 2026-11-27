//! `ItemStack.onCraftedBy`'s `Stats.ITEM_CRAFTED` count from the result slots that
//! award it on a take: the crafting grid (`ResultSlot.checkTakeAchievements`), the
//! stonecutter and the smithing table (`StonecutterMenu.onTake`, `SmithingMenu.onTake`).

use super::*;

/// Selects the stonecutter recipe whose result is `item`, as a click on its
/// button in the screen does.
fn cut(env: &Env, menu: &mut Menu, item: &str) -> i32 {
    let i = menu
        .stone
        .recipes
        .iter()
        .position(|&r| &*env.station_recipes.stonecutting[r].result.item == item)
        .unwrap_or_else(|| panic!("no stonecutter recipe for {item}"));
    assert!(menu.button(env, i as i32), "the recipe button");
    i as i32
}

/// `StonecutterMenu.onTake` counts the taken result (`carried.onCraftedBy`)
/// before `awardUsedRecipes` reports the recipe.
#[test]
fn a_stonecutter_take_counts_the_item_crafted() {
    with_menu(Kind::Stonecutter, |env, menu| {
        menu.creative = false;
        menu.set_slot(env, 0, stack("stone", 4));
        cut(env, menu, "minecraft:stone_bricks");
        menu.click(env, 1, 0, Input::Pickup).unwrap();
        assert_eq!(
            menu.crafted_counts,
            [("minecraft:stone_bricks".into(), 1)],
            "one take counts one crafted item"
        );
        assert_eq!(menu.crafted.len(), 1, "the recipe-crafted trigger fires");
    });
}

/// `SmithingMenu.onTake` counts the taken result before it awards the recipe
/// and uses one of each input.
#[test]
fn a_smithing_take_counts_the_item_crafted() {
    with_menu(Kind::Smithing, |env, menu| {
        menu.creative = false;
        menu.set_slot(env, 0, stack("netherite_upgrade_smithing_template", 1));
        menu.set_slot(env, 1, stack("diamond_sword", 1));
        menu.set_slot(env, 2, stack("netherite_ingot", 1));
        assert_eq!(
            menu.slot(3).map(|s| &*s.item),
            Some("minecraft:netherite_sword"),
            "the upgrade's result"
        );
        menu.click(env, 3, 0, Input::Pickup).unwrap();
        assert_eq!(
            menu.crafted_counts,
            [("minecraft:netherite_sword".into(), 1)]
        );
        assert_eq!(menu.crafted.len(), 1, "the recipe-crafted trigger fires");
        assert_eq!(menu.slot(1), None, "the base item is used up");
    });
}

/// `StonecutterMenu.quickMoveStack` hands `onTake` what its move left behind:
/// a shift-click that fits everything counts no crafted item at all, and
/// `ItemStack.getItem` of that emptied stack is AIR.
#[test]
fn a_stonecutter_shift_click_counts_no_item_crafted() {
    with_menu(Kind::Stonecutter, |env, menu| {
        menu.creative = false;
        menu.set_slot(env, 0, stack("stone", 1));
        cut(env, menu, "minecraft:stone_bricks");
        menu.click(env, 1, 0, Input::QuickMove).unwrap();
        assert_eq!(menu.slot(1), None, "the result moved to the inventory");
        assert_eq!(menu.crafted_counts, [("minecraft:air".into(), 0)]);
        assert_eq!(menu.crafted.len(), 1, "the recipe-crafted trigger fires");
    });
}

/// `ResultSlot.checkTakeAchievements` counts `removeCount`, what the take asked for, while
/// `ResultContainer.removeItem` hands over the whole stack: one oak log makes four planks,
/// and a right-click carries all four but counts two, a Q-press drops all four but counts
/// one. A shift-click counts what moved (`onQuickCraft`), a hotbar swap the whole stack
/// (`onSwapCraft`). Every take reports the recipe once (`awardUsedRecipes`).
#[test]
fn a_crafting_result_take_counts_what_it_asked_for() {
    for (input, button, counted) in [
        (Input::Pickup, 0, 4),
        (Input::Pickup, 1, 2),
        (Input::QuickMove, 0, 4),
        (Input::Swap, 0, 4),
        (Input::Throw, 0, 1),
        (Input::Throw, 1, 4),
    ] {
        with_menu(Kind::Inventory, |env, menu| {
            menu.creative = false;
            menu.set_slot(env, 1, stack("oak_log", 1));
            assert_eq!(
                menu.slot(0),
                stack("oak_planks", 4).as_ref(),
                "the grid's result"
            );
            menu.click(env, 0, button, input).unwrap();
            assert_eq!(
                menu.crafted_counts,
                [("minecraft:oak_planks".into(), counted)],
                "{input:?} {button}"
            );
            assert_eq!(
                menu.crafted.len(),
                1,
                "{input:?} {button}: the recipe is reported"
            );
            assert_eq!(
                menu.slot(0),
                None,
                "{input:?} {button}: the whole result left"
            );
            assert_eq!(menu.slot(1), None, "{input:?} {button}: the log was used");
        });
    }
}
