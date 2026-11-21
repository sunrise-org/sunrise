//! Clicks and recipe placement in a content mod's crafting grid, against the
//! crafting table's.

use super::*;

/// Clicking the result quickly, over and over: every click after the
/// first is a double click (the same slot within 250 ms, a stack in
/// hand), and `AbstractContainerScreen.mouseReleased` sends PICKUP_ALL
/// only where `canTakeItemForPickAll` allows it, else the PICKUP that
/// crafts again. Ten clicks on planks are ten crafts of sticks, on the
/// crafting table and on a mod menu type's grid alike.
#[test]
fn rapid_clicks_on_a_crafting_result_each_craft() {
    const CLICKS: u32 = 10;
    with_menu(Kind::Crafting, |env, table| {
        let station = &mut Menu::new(mod_crafting_table());
        for menu in [&mut *table, &mut *station] {
            menu.set_slot(env, 1, stack("oak_planks", 32));
            menu.set_slot(env, 4, stack("oak_planks", 32));
            for k in 0..CLICKS {
                let double = k > 0 && menu.carried.is_some();
                let input = if double && menu.can_take_for_pick_all(0) {
                    Input::PickupAll
                } else {
                    Input::Pickup
                };
                menu.click(env, 0, 0, input).unwrap();
            }
            assert!(!menu.can_take_for_pick_all(0));
            let sticks = menu.carried.as_ref().map(|c| (&*c.item, c.count));
            assert_eq!(sticks, Some(("minecraft:stick", 4 * CLICKS)));
            assert_eq!(menu.slot(1).map(|s| s.count), Some(32 - CLICKS));
        }
        assert_eq!(seen(station), seen(table));
    });
}

/// The recipe book's placement (`ServerPlaceRecipe`) and the viewers'
/// transfer fill a mod crafting grid as the crafting table's, and a
/// placement without the items leaves the grid empty for a ghost.
#[test]
fn recipes_place_in_a_mod_crafting_grid_as_in_the_table() {
    with_menu(Kind::Crafting, |env, table| {
        let station = &mut Menu::new(mod_crafting_table());
        for menu in [&mut *table, &mut *station] {
            menu.set_player_item(0, stack("oak_planks", 10));
            menu.set_player_item(5, stack("cobblestone", 3));
            menu.set_slot(env, 5, stack("dirt", 1));
        }
        let mut places = Vec::new();
        for menu in [&mut *table, &mut *station] {
            let first = menu.place_recipe(env, "minecraft:crafting_table", false, false, None);
            let again = menu.place_recipe(env, "minecraft:crafting_table", true, false, None);
            let ghost = menu.place_recipe(env, "minecraft:furnace", false, false, None);
            let transfer = menu.transfer_recipe(env, "minecraft:stick", false);
            places.push((first, again, ghost, transfer, seen(menu)));
        }
        assert_eq!(places[1], places[0]);
        let (.., transfer, (slots, ..)) = &places[1];
        assert_eq!(*transfer, Ok(()));
        assert_eq!(slots[0], stack("stick", 4));
    });
}

/// `menu_slot_rules` on a mod crafting menu: an ask about a grid slot names `Grid` and indexes
/// inside the grid, the result's names `Result`, and either context reads a grid slot and the
/// result from the staged menu.
#[test]
fn slot_rules_read_a_mod_grid_slot_and_the_result() {
    use mod_api::MenuContainer::{Grid, Result};
    type Asked = (
        usize,
        mod_api::MenuContainer,
        Option<(String, u32)>,
        Option<(String, u32)>,
    );
    #[derive(Default)]
    struct GridRules(std::cell::RefCell<Vec<Asked>>);
    impl SlotRules for GridRules {
        fn listens(&self, menu: &str) -> bool {
            menu == "test:station"
        }
        fn rule(
            &self,
            _: &str,
            slot: usize,
            _: &ItemStack,
            context: SlotContext<'_>,
            _: &mut bool,
            _: &mut bool,
            _: &mut u32,
        ) {
            let read = |container, index| {
                let stack = context.slot(container, index)?;
                Some((stack.item.to_string(), stack.count))
            };
            let asked = (slot, context.container, read(Grid, 0), read(Result, 0));
            self.0.borrow_mut().push(asked);
        }
    }
    with_menu(Kind::Crafting, |env, _| {
        let rules = GridRules::default();
        let env = &Env {
            slot_rules: Some(&rules),
            ..*env
        };
        let station = &mut Menu::new(mod_crafting_table());
        station.set_slot(env, 1, stack("oak_planks", 1));
        station.set_slot(env, 4, stack("oak_planks", 1));
        let planks = stack("oak_planks", 1).unwrap();
        // Menu slot 2 is grid slot 1; menu slot 0 is the result.
        station.slot_rules(env, 2, &planks);
        station.slot_rules(env, 0, &planks);
        let planks = Some(("minecraft:oak_planks".to_owned(), 1));
        let sticks = Some(("minecraft:stick".to_owned(), 4));
        assert_eq!(
            *rules.0.borrow(),
            [
                (1, Grid, planks.clone(), sticks.clone()),
                (0, Result, planks.clone(), sticks.clone()),
            ]
        );
        station.set_slot(env, 1, None);
        station.slot_rules(env, 5, &stack("stick", 1).unwrap());
        assert_eq!(
            rules.0.borrow()[2],
            (4, Grid, None, Some(("minecraft:oak_button".to_owned(), 1)))
        );
    });
}

#[test]
fn slot_rule_answers_follow_the_grid_slots_that_were_read() {
    use mod_api::MenuContainer::Grid;
    use std::cell::Cell;

    struct NeedsIngredient(Cell<u32>);
    impl SlotRules for NeedsIngredient {
        fn listens(&self, menu: &str) -> bool {
            menu == "test:station"
        }
        fn rule(
            &self,
            _: &str,
            _: usize,
            _: &ItemStack,
            context: SlotContext<'_>,
            _: &mut bool,
            may_pickup: &mut bool,
            _: &mut u32,
        ) {
            self.0.set(self.0.get() + 1);
            *may_pickup = context.slot(Grid, 0).is_some();
        }
    }

    bare_env(|env| {
        let mut live = Menu::new(mod_crafting_table());
        let held = stack("stone", 1).unwrap();
        live.load_slot(2, Some(held.clone()));
        let mut preview = live.clone();
        let rules = NeedsIngredient(Cell::new(0));
        let answers = RuleAnswers::default();
        let planning = PlanningRules {
            rules: Some(&rules),
            answers: &answers,
        };
        let run = |menu: &mut Menu, rules: &dyn SlotRules| {
            let env = Env {
                slot_rules: Some(rules),
                ..*env
            };
            menu.planned(Asked::default(), |menu| {
                let before = menu.slot_rules(&env, 2, &held).1;
                // Changing an unread slot keeps the same answer for this question.
                menu.load_slot(4, stack("dirt", 1));
                let unrelated = menu.slot_rules(&env, 2, &held).1;
                // An empty slot the part read gains the ingredient it needed.
                menu.load_slot(1, stack("oak_planks", 1));
                let with_ingredient = menu.slot_rules(&env, 2, &held).1;
                [before, unrelated, with_ingredient]
            })
            .0
        };
        assert_eq!(run(&mut preview, &planning), [false, false, true]);
        assert_eq!(rules.0.get(), 2);
        let replay = StoredRules(&answers);
        assert_eq!(run(&mut live, &replay), [false, false, true]);
        assert_eq!(answers.missed(), 0);

        let env = Env {
            slot_rules: Some(&replay),
            ..*env
        };
        live.planned(Asked::default(), |menu| {
            // A grid not seen in planning keeps the slot's own vanilla answer.
            menu.load_slot(1, stack("dirt", 1));
            assert!(menu.slot_rules(&env, 2, &held).1);
            // The miss must not mask the stored refusal when the original grid returns.
            menu.load_slot(1, None);
            assert!(!menu.slot_rules(&env, 2, &held).1);
        });
        assert_eq!(answers.missed(), 1);
        assert_eq!(rules.0.get(), 2, "apply only replays the recorded answers");
    });
}
