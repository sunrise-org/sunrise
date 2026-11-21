use super::*;
use crate::client::menu_screen::OwnedMenuScreen;

// no wire: frozen registration storage; each boundary value has its own Wire derive.
#[derive(Clone)]
pub(crate) struct MenuBookDeclaration {
    pub(crate) owner: &'static str,
    pub(crate) book: crate::RecipeBookTypeKey,
    pub(crate) recipe_type: String,
    pub(crate) layout: crate::RecipeBookLayout,
}

// no wire: frozen registration and panic state, never a window instance.
/// Engine lookup record for a registered constructor.
#[derive(Clone)]
pub(crate) struct MenuScreenRegistration {
    pub(crate) owner: &'static str,
    pub(crate) index: usize,
    pub(crate) factory: Arc<dyn crate::MenuScreenFactory>,
    pub(crate) panicked: Arc<std::sync::atomic::AtomicBool>,
}

/// Look up the factory for the resolved custom-menu owner or the actual stock opener.
fn menu_screen_factory(
    menu: &str,
    menu_owner: Option<&str>,
    opener: Option<&str>,
) -> Option<&'static MenuScreenRegistration> {
    let rows = installed()?.menu_screen_factories.get(menu)?;
    let index = match menu_owner {
        Some(owner) => crate::__private::mod_index(owner)?,
        None => crate::runtime::content_owner_index(opener?)?,
    };
    rows.get(&index)
}

/// Whether the content menu's owner or the verified stock opener's owner supplies a constructor.
pub fn has_menu_screen_factory(menu: &str, menu_owner: Option<&str>, opener: Option<&str>) -> bool {
    menu_screen_factory(menu, menu_owner, opener).is_some()
}

/// Build a window's screen. The outer option distinguishes an unregistered factory
/// from one that declined or was refused. The returned instance is not part of registration.
pub fn create_menu_screen(
    host: &dyn crate::ClientHost,
    open: crate::MenuScreenOpen<'_>,
    menu_owner: Option<&str>,
) -> Option<(&'static str, Result<Option<OwnedMenuScreen>, String>)> {
    let registration = menu_screen_factory(open.menu, menu_owner, open.block)?;
    let screen = crate::callback_owner::call(
        registration.owner,
        registration.index,
        &registration.panicked,
        "menu screen factory",
        "menu_screen_factory",
        || {
            registration
                .factory
                .create(host, open)
                .map(|screen| OwnedMenuScreen::new(registration.owner, registration.index, screen))
        },
    );
    Some((registration.owner, screen))
}

impl HookParts {
    pub(crate) fn menu<'k>(
        &self,
        id: impl FnOnce() -> Option<&'k str>,
    ) -> &[Attached<dyn MenuPart>] {
        attached(&self.menus, id)
    }
    /// The menu's owner lookup: only the part whose package owns the menu type, or the block
    /// opening a vanilla menu, receives callbacks. A bound recipe book keeps ownership with
    /// its declaring mod.
    pub(crate) fn menu_for_opener(
        &self,
        menu: &str,
        block: Option<&str>,
    ) -> &[Attached<dyn MenuPart>] {
        let parts = attached(&self.menus, || Some(menu));
        let owner = crate::RecipeBookLayout::declaring_mod(menu);
        let namespace = match owner {
            Some(owner) => owner,
            None if menu.starts_with("minecraft:") => {
                let Some((namespace, _)) = block.and_then(|b| b.split_once(':')) else {
                    return &[];
                };
                namespace
            }
            None => {
                let Some((namespace, _)) = menu.split_once(':') else {
                    return &[];
                };
                namespace
            }
        };
        match parts
            .iter()
            .position(|p| owns_namespace(p.mod_id, namespace))
        {
            Some(i) => std::slice::from_ref(&parts[i]),
            None => &[],
        }
    }
}

/// Whether the Parts attached to the menu type `menu` subscribe to `hook`; the game
/// asks once when a menu opens, so a menu with none skips the hook while it is open.
pub fn menu_listens(hook: HookId, menu: &str) -> bool {
    if !has_part_hook(hook) {
        return false;
    }
    parts(hook, "menu routing")
        .for_hook(hook)
        .is_some_and(|table| !table.menu(|| Some(menu)).is_empty())
}

/// Authoritative exclusions for the validated opener, or the menu type's default.
/// The caller checks these backing indices against the count of its actual menu.
pub fn menu_slot_exclusions(menu: &str, block: Option<&str>) -> &'static [u16] {
    let Some(scopes) = installed().and_then(|installed| installed.menu_slot_exclusions.get(menu))
    else {
        return &[];
    };
    // An empty scope key denotes the default; explicit block IDs cannot be empty.
    block
        .and_then(|block| scopes.get(block))
        .or_else(|| scopes.get(""))
        .map_or(&[], |slots| slots.as_ref())
}

/// All permanent masks for a menu, including the default and each exact opener override.
/// Content-load validation checks recipe roles against each mask before the menu can open.
pub fn menu_slot_exclusion_masks(
    menu: &str,
) -> impl Iterator<Item = (Option<&'static str>, &'static [u16])> + use<> {
    installed()
        .and_then(|installed| installed.menu_slot_exclusions.get(menu))
        .into_iter()
        .flatten()
        .map(|(scope, slots)| {
            (
                (!scope.is_empty()).then_some(scope.as_str()),
                slots.as_ref(),
            )
        })
}

impl dyn Registrar + '_ {
    /// Attach gameplay behavior to `menu`: a mod's, or one of vanilla's that
    /// a mod block opens. Register the client screen separately with
    /// [`MenusRegistrar::register_menu_screen`].
    pub fn menu_part<P: MenuPart>(&mut self, menu: &str, part: P) {
        self.attach_menu_part(menu, type_name::<P>(), Arc::new(part));
    }
}

/// Registrar operations for menus.
pub trait MenusRegistrar {
    /// Register one client constructor for a namespaced menu type. A custom menu must
    /// be in your namespace. For a stock menu, the constructor is called only when
    /// a verified mod block owned by your package opens that menu.
    /// The factory builds a fresh screen on the client for each opening. The
    /// window owns its callbacks and local slot moves until close or replacement.
    /// If a custom menu or a stock opener with excluded slots lacks its factory,
    /// the client warns and leaves the current menu unchanged.
    /// A duplicate or out-of-scope registration refuses installation, even if its
    /// error is ignored. A package with a factory cannot declare `side = "server"`.
    fn register_menu_screen(
        &mut self,
        menu: &str,
        factory: Arc<dyn crate::MenuScreenFactory>,
    ) -> Result<(), String>;

    /// Exclude container backing indices from server pickup and quick-move and
    /// from the client layout. Indices stay in place; the player inventory is unaffected.
    /// The owning mod of a custom `menu` may use `None` for its default or `Some(block)`
    /// to replace it when that exact mod block's container opens this menu type.
    /// For a stock menu, `block` must name an explicit mod block owned by the registrar.
    /// Both IDs must be namespaced. Indices are sorted and deduplicated.
    /// An empty slice is an explicit empty mask.
    /// A duplicate or out-of-scope declaration refuses installation even if the error
    /// is ignored. An index outside the actual container backing refuses the open.
    /// Screen layouts leave these positions empty, without renumbering later slots.
    /// Local hides never change this mask or server-side slot validation.
    fn menu_slot_exclusions(
        &mut self,
        menu: &str,
        block: Option<&str>,
        excluded: &[u16],
    ) -> Result<(), String>;

    /// Declare this menu's book, authoritative recipe type and destination roles.
    /// The menu remains content-owned; loading checks these roles against its
    /// actual container slots, crafting declaration and every permanent mask
    /// declared with [`Self::menu_slot_exclusions`].
    /// To show this menu, register a factory with [`Self::register_menu_screen`]
    /// whose layout sets [`crate::MenuScreenLayout::recipe_book`] to this `book`.
    /// Content can load without the factory. The client checks for it when the
    /// menu opens and validates geometry when the factory constructs the screen. The
    /// declaring mod owns the screen's presentation callbacks and slot controls.
    /// This does not turn display candidates into ingredient predicates.
    /// Invalid ownership, unknown books, duplicate menus or malformed roles
    /// refuse installation, even when the caller ignores the error.
    fn menu_recipe_book(
        &mut self,
        menu: &str,
        book: &crate::RecipeBookTypeKey,
        recipe_type: &str,
        layout: crate::RecipeBookLayout,
    ) -> Result<(), String>;
    /// Attach `part`, of kind `kind`, to the menu type `menu`.
    fn attach_menu_part(&mut self, menu: &str, kind: &'static str, part: Arc<dyn MenuPart>);
}
