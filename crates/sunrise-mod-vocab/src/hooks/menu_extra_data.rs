//! The `menu_extra_data` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// Block `block` at `pos` opens its mod menu type `menu` for server
    /// player `player`: set `data` to hand the menu's screen bytes of your
    /// own (a machine's tier, mode or recipes), up to 32 KiB, which the
    /// client reads only from `MenuScreenOpen::extra` during construction
    /// (NeoForge's `openMenu` with an extra-data writer). For live values use
    /// the menu's data slots (`MenuScreenFrame::data` on the client). The first
    /// mod to set `data` wins, so answer only for your blocks. The player's
    /// state is not swapped in.
    #[hook(
        dispatch = "flat",
        side = "server",
        contract = "replace",
        out = "data",
        neoforge = "IPlayerExtension.openMenu",
        cancellable = false
    )]
    fn menu_extra_data(
        player: u128,
        menu: &str,
        block: &str,
        pos: [i32; 3],
        data: &mut Option<Vec<u8>>,
    );
}
