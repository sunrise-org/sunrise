//! The `menu_extra_data` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// Block `block` at `pos` opens its mod menu type `menu` for server
    /// player `player`: set `data` to hand the menu's screen bytes of your
    /// own (a machine's tier, mode or recipes), up to 1,048,547 bytes with opening
    /// tokens, or 1,048,555 with earlier peers (vanilla's 1 MiB clientbound limit
    /// minus the envelope). The client reads these bytes only from
    /// `MenuScreenOpen::extra` during construction (NeoForge's `openMenu`
    /// with an extra-data writer). Oversize data is logged and refuses the
    /// menu open. For live values use the menu's data slots
    /// (`MenuScreenFrame::data` on the client). The first mod to set `data`
    /// wins, so answer only for your blocks. The player's state is not
    /// swapped in.
    #[hook(
        dispatch = "flat",
        side = "server",
        version = 2,
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
