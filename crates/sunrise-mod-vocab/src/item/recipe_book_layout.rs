//! Menu-owned destination roles, not recipe ingredient semantics.
/// Recipe destinations at physical container indices, never indices into the
/// appended player inventory. The menu owns these roles; the registered screen
/// factory supplies client geometry separately. Local slot moves and hides do
/// not change the destination of a recipe.
#[derive(Clone, Debug, PartialEq, Eq, sunrise_mod_api_macros::Wire)]
// exhaustive: the four destination maps cover the five vanilla displays, and `Mod` hands the rest to the mod
pub enum RecipeBookLayout {
    Crafting {
        width: u16,
        height: u16,
        slots: Vec<u16>,
        result: u16,
    },
    Furnace {
        input: u16,
        fuel: u16,
        result: u16,
    },
    Stonecutter {
        input: u16,
        result: u16,
    },
    Smithing {
        template: u16,
        base: u16,
        addition: u16,
        result: u16,
    },
    /// The menu's placement is answered by the mod: the `recipe_book_place` hook of its
    /// `MenuPart`, with `ServerHost::place_recipe_in_grid` as vanilla's placer. The roles are the
    /// mod's own, so none is declared or checked here, and the game maps no display to a slot.
    Mod,
}

impl RecipeBookLayout {
    /// Engine integration: diagnostics retain the actual declaring mod identity.
    // hidden: engine diagnostics need registration ownership, not a mod-facing lookup service.
    #[doc(hidden)]
    pub fn declaring_mod(menu: &str) -> Option<&'static str> {
        crate::parts::installed()?
            .menu_recipe_books
            .get(menu)
            .map(|binding| binding.owner)
    }
    /// Check that a permanent exclusion mask leaves every declared role
    /// available at its backing index. Loading checks the default and every
    /// opener-specific mask. Local screen hides are not exclusions.
    pub fn validate_exclusions(&self, excluded: &[u16]) -> Result<(), String> {
        self.check_roles(|slot, role| {
            if excluded.contains(&slot) {
                return Err(format!("{role} slot {slot} is permanently excluded"));
            }
            Ok(())
        })
    }
    /// Menus with declarations, for content-load validation, never per tick.
    pub fn registered_menus() -> Vec<String> {
        let mut menus = crate::parts::installed()
            .map(|r| r.menu_recipe_books.keys().cloned().collect::<Vec<_>>())
            .unwrap_or_default();
        menus.sort();
        menus
    }
    /// Frozen declaration for a menu, as registered at startup. No declaration
    /// means that the menu has no custom-book placement contract.
    pub fn registered(menu: &str) -> Option<(crate::RecipeBookTypeKey, String, Self)> {
        crate::parts::installed()?
            .menu_recipe_books
            .get(menu)
            .map(|binding| {
                (
                    binding.book.clone(),
                    binding.recipe_type.clone(),
                    binding.layout.clone(),
                )
            })
    }

    /// Check role indices against the content menu's container slot count.
    /// The count includes permanently excluded slots. Exclusion masks are
    /// checked separately with [`Self::validate_exclusions`].
    pub fn validate(&self, slot_count: u16) -> Result<(), String> {
        if slot_count == 0 || slot_count > 256 {
            return Err("container slot count must be in 1..=256".into());
        }
        if let Self::Crafting {
            width,
            height,
            slots,
            ..
        } = self
        {
            let cells = u32::from(*width) * u32::from(*height);
            if *width == 0
                || *height == 0
                || cells > u32::from(slot_count)
                || cells as usize != slots.len()
            {
                return Err("grid dimensions must match its slot list and fit the menu".into());
            }
        }
        let mut seen = [false; 256];
        self.check_roles(|slot, role| {
            if slot >= slot_count {
                return Err(format!("{role} slot {slot} is outside the menu"));
            }
            if seen[usize::from(slot)] {
                return Err(format!("{role} slot {slot} overlaps another role"));
            }
            seen[usize::from(slot)] = true;
            Ok(())
        })
    }

    fn check_roles(
        &self,
        mut check: impl FnMut(u16, &'static str) -> Result<(), String>,
    ) -> Result<(), String> {
        match self {
            Self::Crafting { slots, result, .. } => {
                for &slot in slots {
                    check(slot, "grid")?;
                }
                check(*result, "result")
            }
            Self::Furnace {
                input,
                fuel,
                result,
            } => {
                check(*input, "input")?;
                check(*fuel, "fuel")?;
                check(*result, "result")
            }
            Self::Stonecutter { input, result } => {
                check(*input, "input")?;
                check(*result, "result")
            }
            Self::Smithing {
                template,
                base,
                addition,
                result,
            } => {
                check(*template, "template")?;
                check(*base, "base")?;
                check(*addition, "addition")?;
                check(*result, "result")
            }
            Self::Mod => Ok(()),
        }
    }
}
