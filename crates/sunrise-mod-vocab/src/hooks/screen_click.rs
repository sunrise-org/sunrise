//! The `screen_click` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0029

pub trait Hooks {
    /// A left or right (`right`) press at `at` (GUI pixels from the screen's
    /// top left, so outside it too) on the screen of mod menu type `menu`
    /// (`block` as in `screen`, including menus and blocks in a content package
    /// served by this mod via `served_by`), over no slot: set `button` to a menu button
    /// id when it is on one of yours.
    /// Sunrise sends it to the server (`ContainerButtonClick`,
    /// `menu_button` there) and the press goes no further. The menu type's
    /// `MenuPart` hears a button for that type, including a menu opened
    /// without a block.
    #[hook(
        version = 3,
        side = "client",
        contract = "replace",
        out = "button",
        scope = "menu",
        key = "menu",
        neoforge = "none (see screen)"
    )]
    fn screen_click();
}
