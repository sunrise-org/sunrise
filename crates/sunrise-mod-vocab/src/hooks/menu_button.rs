//! The `menu_button` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0031

pub trait Hooks {
    /// A player pressed a mod menu's button (`clickMenuButton`), on the
    /// server. `entity` is the block entity backing the menu when there is
    /// one, out of the world during the call; it is None for a blockless
    /// menu. Mods may change it and the clicking player's carried stack.
    #[notes = r#"### Menu buttons

`menu_button` handles a mod menu's button (`AbstractContainerMenu.clickMenuButton(Player, int)`) on the server, for
single player and LAN guests. It is routed to the active menu type's owner and exposes the player identity, button,
optional block position, optional backing block entity and mutable carried stack, so a widget outside the slots can
take or give what the player carries. A blockless menu supplies None for the position and entity."#]
    #[hook(
        version = 2,
        side = "server",
        contract = "listen",
        scope = "menu",
        key = "menu",
        neoforge = "none (NeoForge has no such event; vanilla AbstractContainerMenu.clickMenuButton)"
    )]
    fn menu_button();
}
