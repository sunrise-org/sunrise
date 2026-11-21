//! Code-free projection for the recipe book's remote display and craftable filter.
use mod_api::item::crafting::{Ingredient, IngredientRule, Shape};
use mod_api::{
    PlacementInfo, RecipeBookCategory, RecipeBookDisplay, RecipeBookDisplayEntry, RecipeBookTypeKey,
};
use std::collections::HashSet;
use sunrise_core::nbt::Compound;

#[derive(mod_api::Wire)]
pub(super) struct Category {
    id: String,
    recipe_type: String,
    icon: String,
    secondary_icon: Option<String>,
    target_menu: String,
    book: RecipeBookTypeKey,
    search_group: String,
    displays: Vec<Entry>,
}

#[derive(mod_api::Wire)]
struct Entry {
    recipe: String,
    displays: Vec<RecipeBookDisplay>,
    placement: Option<Cells>,
}

#[derive(mod_api::Wire)]
struct Cells {
    cells: Vec<Option<StaticIngredient>>,
    shape: Option<[u16; 2]>,
}

#[derive(mod_api::Wire)]
struct StaticIngredient {
    items: HashSet<String>,
    order: Vec<String>,
    children: Option<(Shape, Vec<Self>)>,
    rule: Option<Box<IngredientRule>>,
    display: Option<Box<Compound>>,
}

impl StaticIngredient {
    fn project(value: &Ingredient) -> Option<Self> {
        if value.custom.is_some() {
            return None;
        }
        let children = match &value.children {
            Some((shape, children)) => Some((
                shape.clone(),
                children.iter().map(Self::project).collect::<Option<_>>()?,
            )),
            None => None,
        };
        Some(Self {
            items: value.items.clone(),
            order: value.order.clone(),
            children,
            rule: value.rule.clone(),
            display: value.display.clone(),
        })
    }
}

impl From<StaticIngredient> for Ingredient {
    fn from(value: StaticIngredient) -> Self {
        Self {
            items: value.items,
            order: value.order,
            custom: None,
            children: value
                .children
                .map(|(shape, children)| (shape, children.into_iter().map(Into::into).collect())),
            rule: value.rule,
            display: value.display,
        }
    }
}

impl From<&RecipeBookCategory> for Category {
    fn from(value: &RecipeBookCategory) -> Self {
        Self {
            id: value.id.clone(),
            recipe_type: value.recipe_type.clone(),
            icon: value.icon.clone(),
            secondary_icon: value.secondary_icon.clone(),
            target_menu: value.target_menu.clone(),
            book: value.book.clone(),
            search_group: value.search_group.clone(),
            displays: value
                .displays
                .iter()
                .map(|entry| Entry {
                    recipe: entry.recipe.clone(),
                    displays: entry.displays.clone(),
                    placement: entry.placement.as_ref().map(|info| {
                        let cells = info
                            .cells
                            .iter()
                            .map(|cell| match cell {
                                None => Some(None),
                                Some(ingredient) => StaticIngredient::project(ingredient).map(Some),
                            })
                            .collect::<Option<_>>();
                        // The client cannot predict craftability for a native predicate. Keep
                        // the pictures and selection; only the server has a placement plan.
                        cells.map_or(
                            Cells {
                                cells: Vec::new(),
                                shape: None,
                            },
                            |cells| Cells {
                                cells,
                                shape: info.shape,
                            },
                        )
                    }),
                })
                .collect(),
        }
    }
}

impl From<Category> for RecipeBookCategory {
    fn from(value: Category) -> Self {
        Self {
            id: value.id,
            recipe_type: value.recipe_type,
            icon: value.icon,
            secondary_icon: value.secondary_icon,
            target_menu: value.target_menu,
            book: value.book,
            search_group: value.search_group,
            displays: value
                .displays
                .into_iter()
                .map(|entry| RecipeBookDisplayEntry {
                    recipe: entry.recipe,
                    displays: entry.displays,
                    placement: entry.placement.map(|info| PlacementInfo {
                        cells: info
                            .cells
                            .into_iter()
                            .map(|cell| cell.map(Into::into))
                            .collect(),
                        shape: info.shape,
                    }),
                })
                .collect(),
        }
    }
}
