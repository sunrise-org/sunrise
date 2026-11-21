//! Validate registered destination roles before publishing content menu types.
use mod_api::RecipeBookLayout;
use sunrise_client::recipe_book::{
    menu_layout::{MenuLayoutMetadata, compile},
    slot_layout::LayoutError,
};
use sunrise_mod_package::content::MenuType;

pub(super) fn validate_registered(menus: &[&MenuType]) -> Result<(), String> {
    for menu in RecipeBookLayout::registered_menus() {
        let Some((_, _, layout)) = RecipeBookLayout::registered(&menu) else {
            continue;
        };
        validate(
            menus,
            &menu,
            layout,
            mod_api::__private::menu_slot_exclusion_masks(&menu),
        )?;
    }
    Ok(())
}

fn validate<'mask>(
    menus: &[&MenuType],
    id: &str,
    layout: RecipeBookLayout,
    masks: impl IntoIterator<Item = (Option<&'mask str>, &'mask [u16])>,
) -> Result<(), String> {
    let menu = menus
        .iter()
        .find(|menu| menu.id == id)
        .ok_or_else(|| format!("recipe book menu {id:?}: unknown content menu"))?;
    let metadata = MenuLayoutMetadata {
        slots: menu.slots,
        crafting: menu
            .crafting
            .map(|grid| (grid.grid, grid.size, grid.result)),
    };
    let checked = compile(layout, metadata)
        .map_err(|error: LayoutError| format!("recipe book menu {id:?}: {error:?}"))?;
    for (opener, excluded) in masks {
        checked
            .declaration()
            .validate_exclusions(excluded)
            .map_err(|error| format!("recipe book menu {id:?}, opener {opener:?}: {error}"))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn menu() -> MenuType {
        MenuType {
            id: "demo:pot".into(),
            slots: 3,
            player_inventory: true,
            data_slots: 0,
            owned_slots: None,
            results: vec![2],
            inputs: Vec::new(),
            crafting: None,
            links: None,
        }
    }
    #[test]
    fn actual_content_count_is_required_before_publication() {
        let menu = menu();
        let roles = RecipeBookLayout::Furnace {
            input: 0,
            fuel: 1,
            result: 2,
        };
        assert!(validate(&[&menu], "demo:pot", roles.clone(), []).is_ok());
        assert!(
            validate(&[], "demo:pot", roles.clone(), [])
                .unwrap_err()
                .contains("unknown content menu")
        );
        let short = MenuType { slots: 2, ..menu };
        assert!(
            validate(&[&short], "demo:pot", roles.clone(), [])
                .unwrap_err()
                .contains("OutOfRange(2)")
        );
        let slotless = MenuType {
            slots: 0,
            player_inventory: false,
            results: vec![],
            ..short
        };
        let error = validate(&[&slotless], "demo:pot", roles, []).unwrap_err();
        assert!(
            error.contains("recipe book menu \"demo:pot\"") && error.contains("InvalidSlotCount"),
            "{error}"
        );
    }
    #[test]
    fn a_bound_menu_loads_without_a_client_factory() {
        struct Demo;
        #[mod_api::__subscriptions]
        impl mod_api::Hooks for Demo {
            fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
                let book = r.recipe_book_type("demo:book").unwrap();
                r.menu_recipe_book(
                    "demo:pot",
                    &book,
                    "demo:cooking",
                    RecipeBookLayout::Furnace {
                        input: 0,
                        fuel: 1,
                        result: 2,
                    },
                )
                .unwrap();
            }
        }
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new("demo", Box::new(Demo))].into();
        mod_platform::testing::with_mods(mods.clone(), || {
            mod_platform::__private::part_tests::install_parts_fixture(&mods).unwrap();
            validate_registered(&[&menu()]).unwrap();
        });
    }

    #[test]
    fn any_permanent_mask_excluding_a_role_refuses_the_content_menu() {
        let menu = menu();
        let roles = RecipeBookLayout::Furnace {
            input: 0,
            fuel: 1,
            result: 2,
        };
        assert!(validate(&[&menu], "demo:pot", roles.clone(), [(None, &[][..])]).is_ok());
        let masks = [(None, &[][..]), (Some("demo:machine"), &[2][..])];
        assert_eq!(
            validate(&[&menu], "demo:pot", roles, masks).unwrap_err(),
            "recipe book menu \"demo:pot\", opener Some(\"demo:machine\"): result slot 2 is permanently excluded"
        );
    }
    #[test]
    fn persistent_grid_roles_are_valid_but_computed_storage_must_agree() {
        let persistent = MenuType {
            slots: 8,
            results: vec![6],
            ..menu()
        };
        let roles = RecipeBookLayout::Crafting {
            width: 3,
            height: 2,
            slots: (0..6).collect(),
            result: 6,
        };
        assert!(validate(&[&persistent], "demo:pot", roles.clone(), []).is_ok());
        let computed = MenuType {
            crafting: Some(sunrise_mod_package::content::MenuCrafting {
                grid: 0,
                size: 2,
                result: 6,
            }),
            ..persistent
        };
        assert!(
            validate(&[&computed], "demo:pot", roles, [])
                .unwrap_err()
                .contains("InvalidGrid")
        );
        let matching = RecipeBookLayout::Crafting {
            width: 2,
            height: 2,
            slots: (0..4).collect(),
            result: 6,
        };
        assert!(validate(&[&computed], "demo:pot", matching, []).is_ok());
    }
}
