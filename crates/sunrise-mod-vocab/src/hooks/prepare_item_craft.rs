//! The `prepare_item_craft` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0324

pub trait Hooks {
    /// A crafting-grid result was prepared for a server player; `pos` is the crafting table, or None for the inventory grid. Change `result` before it is sent.
    #[hook(
        target = "sunrise_client::game::native_hooks::base::Game<'_>::native_prepare_item_craft",
        dispatch = "flat",
        side = "server",
        contract = "replace",
        out = "result",
        neoforge = "none (NeoForge has no such event)",
        paper = "PrepareItemCraftEvent"
    )]
    fn prepare_item_craft(
        player: u128,
        name: &str,
        pos: Option<[i32; 3]>,
        result: &mut Option<ItemStack>,
    );
}
