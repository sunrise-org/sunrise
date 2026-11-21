//! Authoritative placement data crosses one checked load boundary, never a draw path.
#[cfg(test)]
#[path = "mod_placement_tests.rs"]
mod tests;
use super::{DisplayLoadError, DisplayResources, RecipeBookCategory};
use crate::{
    crafting::{Ingredient, Placement},
    menu::place::custom_plan::{MenuPlacement, PlacementRefusal},
};
use mod_api::{
    PlacementInfo, RecipeBookDisplayEntry,
    item::crafting::{IngredientRule, Shape},
};
use sunrise_core::nbt::{Tag, discard_tag};

// A rejected native declaration may contain a tree too deep even to drop recursively.
// Drain every new placement value before its category batch is dropped on refusal.
struct BatchGuard<'a> {
    values: &'a mut [RecipeBookCategory],
    accepted: bool,
}
impl Drop for BatchGuard<'_> {
    fn drop(&mut self) {
        if self.accepted {
            return;
        }
        for category in &mut *self.values {
            for entry in &mut category.displays {
                if let Some(info) = entry.placement.take() {
                    let mut pending: Vec<_> = info.cells.into_iter().flatten().collect();
                    while let Some(mut ingredient) = pending.pop() {
                        if let Some((_, children)) = ingredient.children.take() {
                            pending.extend(children);
                        }
                        if let Some(rule) = ingredient.rule.take() {
                            let IngredientRule::Components { patch, .. } = *rule;
                            discard_tag(Tag::Compound(patch));
                        }
                        if let Some(display) = ingredient.display.take() {
                            discard_tag(Tag::Compound(*display));
                        }
                    }
                }
            }
        }
    }
}
fn invalid(reason: impl Into<String>) -> DisplayLoadError {
    DisplayLoadError::InvalidPlacement(reason.into())
}

pub(super) fn validate_batch(
    values: &mut [RecipeBookCategory],
    resources: &dyn DisplayResources,
) -> Result<(), DisplayLoadError> {
    let mut guard = BatchGuard {
        values,
        accepted: false,
    };
    for category in &*guard.values {
        for entry in &category.displays {
            let Some(info) = &entry.placement else {
                continue;
            };
            validate(info, resources).map_err(|reason| {
                invalid(format!("recipe {} placement: {reason}", entry.recipe))
            })?;
        }
    }
    guard.accepted = true;
    Ok(())
}
fn validate(info: &PlacementInfo, resources: &dyn DisplayResources) -> Result<(), String> {
    if let Some([w, h]) = info.shape
        && (w == 0
            || h == 0
            || usize::from(w).checked_mul(usize::from(h)) != Some(info.cells.len()))
    {
        return Err("shape dimensions disagree with cells".into());
    }
    // Bound recursive use and keep the code-free projection within the recipe
    // catalogue's depth budget (category, entry, cells and first ingredient).
    let mut pending: Vec<_> = info
        .cells
        .iter()
        .flatten()
        .map(|value| (value, 4usize))
        .collect();
    while let Some((ingredient, depth)) = pending.pop() {
        if depth > usize::from(mod_api::wire::MAX_DEPTH) {
            return Err("placement ingredient exceeds depth".into());
        }
        if ingredient.items.iter().any(|id| !resources.item(id)) {
            return Err("unknown ingredient item".into());
        }
        let ordered: std::collections::HashSet<_> = ingredient.order.iter().cloned().collect();
        if !ordered.is_subset(&ingredient.items)
            || (simple_ingredient(ingredient) && ordered != ingredient.items)
            || ordered.len() != ingredient.order.len()
        {
            return Err("ingredient holder order disagrees with its items".into());
        }
        if ingredient.custom.is_some()
            && (ingredient.children.is_some() || ingredient.rule.is_some())
        {
            return Err("custom ingredient cannot also have a child shape or built-in rule".into());
        }
        if let Some((shape, children)) = &ingredient.children {
            let valid = match shape {
                Shape::Compound | Shape::Intersection => !children.is_empty(),
                Shape::Difference => children.len() == 2,
                Shape::CustomDisplay => children.len() == 1,
            };
            if !valid {
                return Err("ingredient shape has invalid children".into());
            }
            pending.extend(children.iter().map(|value| (value, depth + 1)));
        }
        // The native declaration itself has no codec. Its remaining NBT
        // fields still have the independent vanilla depth budget.
        let mut output = mod_api::wire::Writer::new();
        output.put(&ingredient.rule);
        output.put(&ingredient.display);
        if let Some(error) = output.error() {
            return Err(format!("invalid placement ingredient data: {error}"));
        }
    }
    Ok(())
}

pub(super) fn compile(
    owner: &str,
    menu: &str,
    entry: &RecipeBookDisplayEntry,
    layout: &super::super::slot_layout::CheckedSlotLayout,
) -> Result<Option<MenuPlacement>, DisplayLoadError> {
    let info = entry.placement.as_ref().ok_or_else(|| {
        invalid(format!(
            "mod {owner} recipe {}: menu-bound recipe type must supply PlacementInfo",
            entry.recipe
        ))
    })?;
    let cells = info
        .cells
        .iter()
        .map(|cell| cell.as_ref().map(convert))
        .collect();
    let placement = Placement::of(
        cells,
        info.shape.map(|[w, h]| (usize::from(w), usize::from(h))),
    );
    let compiled = match layout.declaration() {
        mod_api::RecipeBookLayout::Mod => MenuPlacement::for_mod(menu.into(), placement),
        _ => MenuPlacement::new(menu.into(), placement, layout),
    };
    match compiled {
        Ok(plan) => Ok(Some(plan)),
        Err(PlacementRefusal::NotPlaceable) => Ok(None),
        Err(error) => Err(invalid(format!(
            "mod {owner} recipe {} placement: {error:?}",
            entry.recipe
        ))),
    }
}
// Only called after validate_batch accepted the native declaration.
fn convert(value: &mod_api::Ingredient) -> Ingredient {
    Ingredient {
        items: value.items.clone(),
        order: value.order.clone(),
        custom: value.custom.clone(),
        rule: None,
        compiled: None,
        display: value.display.as_deref().cloned(),
        component_defaults: Default::default(),
        shown: None,
        children: value
            .children
            .as_ref()
            .map(|(shape, children)| (shape.clone(), children.iter().map(convert).collect())),
    }
}

// Bound the walk before consulting an untrusted native provider's recursive tree.
fn simple_ingredient(ingredient: &mod_api::Ingredient) -> bool {
    let mut pending = vec![(ingredient, 0usize)];
    while let Some((ingredient, depth)) = pending.pop() {
        if depth > usize::from(mod_api::wire::MAX_DEPTH)
            || ingredient.rule.is_some()
            || ingredient.custom.is_some()
        {
            return false;
        }
        if let Some((_, children)) = &ingredient.children {
            pending.extend(children.iter().map(|child| (child, depth + 1)));
        }
    }
    true
}
