//! `StackedContents` and `StackedItemContents`: the item counts a recipe book asks
//! (`canCraft`, `getBiggestCraftableStack`) and a server's placement fills.
use crate::crafting::Ingredient;
use crate::items::ItemStack;
use std::{collections::HashMap, sync::Arc};

use super::picker::Picker;

/// `StackedContents<Holder<Item>>`: item counts in an iteration order.
#[derive(Clone, Debug, Default)]
pub struct Stacked {
    keys: Vec<Arc<str>>,
    amounts: Vec<i32>,
    index: HashMap<Arc<str>, usize>,
}

/// `Inventory.isUsableForCrafting`: not damaged, enchanted or renamed.
pub fn usable_for_crafting(stack: &ItemStack) -> bool {
    stack.damage == 0 && stack.enchantments().is_empty() && stack.custom_name().is_none()
}

impl Stacked {
    /// `StackedContents.account`.
    pub fn account(&mut self, item: &Arc<str>, count: i32) {
        match self.index.get(item) {
            Some(&i) => self.amounts[i] += count,
            None => {
                self.index.insert(item.clone(), self.keys.len());
                self.keys.push(item.clone());
                self.amounts.push(count);
            }
        }
    }
    /// `StackedItemContents.accountStack(stack, maxCount)`.
    pub fn account_stack(&mut self, stack: &ItemStack, max: u32) {
        if stack.count > 0 {
            self.account(&stack.item, stack.count.min(max) as i32);
        }
    }
    /// `accountSimpleStack`.
    pub fn account_simple(&mut self, stack: &ItemStack, max: u32) {
        if usable_for_crafting(stack) {
            self.account_stack(stack, max);
        }
    }
    /// Reorders the keys: the given items first, in that order (vanilla's hash
    /// map order for the same items), then the rest as they were.
    pub fn set_order(&mut self, order: &[String]) {
        let mut keys: Vec<Arc<str>> = order
            .iter()
            .filter_map(|o| self.index.get_key_value(o.as_str()).map(|(k, _)| k.clone()))
            .collect();
        for k in &self.keys {
            if !keys.contains(k) {
                keys.push(k.clone());
            }
        }
        let amounts = keys.iter().map(|k| self.amounts[self.index[k]]).collect();
        self.index = keys
            .iter()
            .cloned()
            .enumerate()
            .map(|(i, k)| (k, i))
            .collect();
        self.keys = keys;
        self.amounts = amounts;
    }
    pub fn keys(&self) -> &[Arc<str>] {
        &self.keys
    }
    /// `canCraft(ingredients, null)` without touching the counts (the recipe
    /// book's craftable test; the answer does not depend on the order; no
    /// ingredients at all can always be found).
    pub fn can_craft(&self, ingredients: &[&Ingredient]) -> bool {
        self.clone().try_pick(ingredients, 1, None)
    }
    fn pick(
        &mut self,
        ingredients: &[&Ingredient],
        amount: i32,
        all: bool,
        output: Option<&mut Vec<Arc<str>>>,
    ) -> i32 {
        let keys = &self.keys;
        let mut picker = Picker::new(&mut self.amounts, ingredients.len(), |key, ingredient| {
            if ingredients[ingredient].items.contains(&*keys[key]) {
                i32::MAX
            } else {
                0
            }
        });
        let mut selected = Vec::new();
        let target = output.as_ref().map(|_| &mut selected);
        let result = if all {
            picker.try_pick_all(amount, target)
        } else {
            i32::from(picker.try_pick(amount, target))
        };
        if let Some(output) = output {
            output.extend(selected.into_iter().map(|index| keys[index].clone()));
        }
        result
    }
    /// `tryPick` (`canCraft(recipe, amount, output)`).
    /// When `output` is `None` this is a non-destructive check: `self` is not
    /// mutated. When `output` is `Some`, the matched items are deducted from
    /// `self` and recorded in `output`.
    pub fn try_pick(
        &mut self,
        ingredients: &[&Ingredient],
        amount: i32,
        output: Option<&mut Vec<Arc<str>>>,
    ) -> bool {
        self.pick(ingredients, amount, false, output) != 0
    }
    /// `tryPickAll` (`getBiggestCraftableStack`).
    pub fn try_pick_all(
        &mut self,
        ingredients: &[&Ingredient],
        max_size: i32,
        output: Option<&mut Vec<Arc<str>>>,
    ) -> i32 {
        self.pick(ingredients, max_size, true, output)
    }
}
