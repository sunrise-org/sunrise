//! The `screen` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0027

pub trait Hooks {
    /// A menu of mod menu type `menu` opens on this client: set `screen` to
    /// lay its screen out (`screen_draw` draws it), whatever opened the menu
    /// (a block, an item's use, an entity, a command). `block` is the mod
    /// block at the menu's position whose container opens this menu type,
    /// and `None` when there is none (`player_open_mod_menu` without such a
    /// position): blocks that share a menu type branch on it. One part owns a
    /// screen and alone hears its four hooks: a mod menu type takes one part
    /// answering them, and a vanilla one takes one per mod, asked only for
    /// that mod's own or served (`served_by`) blocks; a second is refused at
    /// load. A serving native host may also own screens for menu types in its
    /// served packages' namespaces. A mod menu type
    /// with no answer has no screen, and its window closes again. The server
    /// asks it too, whatever opened the menu: a slot it hides takes no
    /// shift-click there. A recipe-book menu's registered screen opens only
    /// for its block's menu.
    #[hook(
        version = 3,
        side = "both",
        contract = "replace",
        out = "screen",
        scope = "menu",
        key = "menu",
        host = "read",
        neoforge = "RegisterMenuScreensEvent.register (one screen per menu type, whatever opened the menu; Sunrise's is a layout value and three drawing and input hooks, not a Screen class)"
    )]
    fn screen();
}
