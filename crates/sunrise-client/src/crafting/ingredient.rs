//! Engine recipe ingredients and stack matching.
#[path = "ingredient/components.rs"]
mod components;
#[path = "ingredient/display.rs"]
mod display;
#[path = "ingredient/matching.rs"]
mod matching;
#[path = "ingredient/stack.rs"]
mod stack;
use super::{ItemStack, Items};
use display::display_stacks;
pub use mod_api::item::crafting::{IngredientRule, Shape};
use mod_api::{
    __private::{WalkedIngredient, ingredient_parser::CustomIds},
    IngredientPredicate,
};
use serde_json::Value;
use std::collections::HashSet;

/// The kind an ingredient object names: `neoforge:ingredient_type` in
/// current NeoForge, `type` in older packs.
pub(super) fn ingredient_kind(o: &serde_json::Map<String, Value>) -> Option<&str> {
    o.get("neoforge:ingredient_type")
        .or_else(|| o.get("type"))?
        .as_str()
}

/// A set of accepted items (expanded from an item, list or tag), and the
/// items in its `HolderSet` order, with built-in component and display rules.
#[derive(Clone, Debug)]
// exhaustive: the game builds it with a literal; mods only read it
pub struct Ingredient {
    /// Full candidate set; a non-simple predicate tests each concrete stack.
    pub items: HashSet<String>,
    /// Default-stack display/simple-match order; may be empty for non-simple kinds.
    pub order: Vec<String>,
    /// The predicate bound to this occurrence by the mod that defined it.
    pub custom: Option<IngredientPredicate>,
    /// NeoForge's shapes, with their children parsed: matching a concrete
    /// stack asks each child its own `test` (a mod's kind among them),
    /// as on NeoForge — the flat list above only names items.
    pub children: Option<(Shape, Vec<Self>)>,
    /// Built-in predicate evaluated against the concrete stack.
    pub rule: Option<IngredientRule>,
    /// Validated decoded component predicate, shared by all candidate checks.
    pub compiled: Option<std::sync::Arc<components::ComponentPredicate>>,
    /// Slot display override; membership continues to use `items`.
    pub display: Option<sunrise_core::nbt::Compound>,
    /// Resolved slot display stacks; never used for placement or matching.
    pub shown: Option<Vec<ItemStack>>,
    /// Shared prototypes of the load that parsed this ingredient.
    pub component_defaults:
        std::sync::Arc<std::collections::HashMap<String, sunrise_core::nbt::Compound>>,
}
impl Ingredient {
    /// Reject malformed component patches before any part of a recipe is published.
    pub fn validate_recipe(value: &Value, items: &Items) -> Result<(), String> {
        components::validate_recipe(value, items)
    }
    pub(crate) fn validate_persistence(value: &Value, recipe: &str) -> bool {
        if let Err(error) = components::persistent_recipe(value) {
            let recipe = sunrise_items::recipe_sources::qualified(recipe);
            log::warn!("recipe {recipe} rejected: {error}");
            return false;
        }
        true
    }
    /// Decode built-in rules and materialize display stacks at a load boundary.
    pub fn prepare_components(&mut self, items: &Items) -> Option<()> {
        if let Some(rule) = &self.rule {
            let compiled = std::sync::Arc::new(
                components::ComponentPredicate::compile(rule, &self.items, items).ok()?,
            );
            if self.display.is_none() {
                self.shown = Some(compiled.shown(&self.order));
            }
            self.compiled = Some(compiled);
        }
        Some(())
    }
    fn default_matches(&self, stack: &ItemStack) -> bool {
        match &self.children {
            Some((Shape::Compound, kids)) => kids.iter().any(|kid| kid.default_matches(stack)),
            Some((Shape::Intersection, kids)) => kids.iter().all(|kid| kid.default_matches(stack)),
            Some((Shape::Difference, kids)) => {
                kids[0].default_matches(stack) && !kids[1].default_matches(stack)
            }
            Some((Shape::CustomDisplay, kids)) => kids[0].default_matches(stack),
            None => {
                self.items.contains(stack.item.as_ref())
                    && self
                        .compiled
                        .as_ref()
                        .is_none_or(|rule| rule.matches(stack))
            }
        }
    }

    /// Resolve a stored slot display at the owning resource boundary.
    pub fn resolve_display(
        display: &sunrise_core::nbt::Compound,
        items: &Items,
    ) -> Option<Vec<ItemStack>> {
        display_stacks(display, items)
    }

    /// Whether item membership fully determines shapeless matching.
    pub fn is_simple(&self) -> bool {
        self.rule.is_none()
            && self.custom.is_none()
            && self
                .children
                .as_ref()
                .is_none_or(|(_, kids)| kids.iter().all(Self::is_simple))
    }

    /// The items of a list, in order.
    pub fn of(items: Vec<String>) -> Self {
        Self {
            items: items.iter().cloned().collect(),
            order: items,
            custom: None,
            children: None,
            rule: None,
            compiled: None,
            display: None,
            shown: None,
            component_defaults: Default::default(),
        }
    }
    /// Parse a built-in ingredient without answers from a recipe load.
    pub fn parse(v: &Value, items: &Items) -> Option<Self> {
        Self::parse_with(v, items, &|_, _| None)
    }

    pub(super) fn parse_with(
        v: &Value,
        items: &Items,
        custom: CustomIds<'_, IngredientPredicate>,
    ) -> Option<Self> {
        let walked = WalkedIngredient::walk(
            v,
            &items.item_tag_order,
            &|tag| {
                Some(
                    items
                        .block_tag_order
                        .get(tag)?
                        .iter()
                        .filter_map(|block| {
                            let item = items
                                .block_items
                                .get(block)
                                .map(String::as_str)
                                .unwrap_or(block);
                            (item != "minecraft:air" && items.get(item).is_some())
                                .then(|| item.to_owned())
                        })
                        .collect(),
                )
            },
            custom,
            &components::json_patch,
        );
        let walked = walked?;
        // A mod kind's empty answer is warned about at load, with its recipe file.
        if walked.order.is_empty() && walked.custom.is_none() {
            log::warn!(
                "recipes: ingredient has no placement candidates; recipe cannot be placed: {v}"
            );
        }
        Self::of_walked(walked, items)
    }
    /// Preserve the standard shapes and the predicate bound to each custom leaf.
    fn of_walked(walked: WalkedIngredient<IngredientPredicate>, items: &Items) -> Option<Self> {
        let children = match walked.children {
            Some((shape, kids)) => Some((
                shape,
                kids.into_iter()
                    .map(|kid| Self::of_walked(kid, items))
                    .collect::<Option<_>>()?,
            )),
            None => None,
        };
        let shown = match &walked.display {
            Some(display) => Some(display_stacks(display, items)?),
            None => None,
        };
        let mut ingredient = Self {
            items: walked.set,
            order: walked.order,
            custom: walked.custom,
            rule: walked.rule,
            compiled: None,
            shown,
            display: walked.display,
            component_defaults: items.component_defaults.clone(),
            children,
        };
        ingredient.prepare_components(items)?;
        let order: Vec<String> = match &ingredient.children {
            Some((Shape::Difference, kids)) => ingredient
                .order
                .iter()
                .filter(|id| !kids[1].default_matches(&ItemStack::new(id, 1)))
                .cloned()
                .collect(),
            Some((Shape::Intersection, _)) => ingredient
                .order
                .iter()
                .filter(|id| ingredient.default_matches(&ItemStack::new(id, 1)))
                .cloned()
                .collect(),
            _ => return Some(ingredient),
        };
        if ingredient.is_simple() {
            ingredient.items = order.iter().cloned().collect();
        }
        ingredient.order = order;
        Some(ingredient)
    }
}

#[cfg(test)]
#[path = "custom_data_tests.rs"]
mod custom_data_tests;

#[cfg(test)]
#[path = "ingredient/builtin_kinds_tests/mod.rs"]
mod builtin_kinds_tests;

#[cfg(test)]
#[path = "ingredient/review_tests.rs"]
mod review_tests;

#[cfg(test)]
#[path = "ingredient/list_entry_tests.rs"]
mod list_entry_tests;

#[cfg(test)]
#[path = "ingredient/second_review_tests.rs"]
mod second_review_tests;
