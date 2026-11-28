//! The `enchantment_level_set` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// An enchanting table at `pos` drew the level of one offer
    /// (`EnchantmentMenu.slotsChanged`, after `getEnchantmentCost`): `offer`
    /// holds the server player whose table it is, the row (0 to 2), the
    /// power (bookshelves counted), the item and the level the row keeps. A
    /// draw below `row + 1` is already cleared to 0 (no offer) when the hook is
    /// asked, as NeoForge clears it before `EnchantmentLevelSetEvent`. Change
    /// `level` to change the offer: the row's cost is `level` as left, and 0 or
    /// less offers nothing. Asked for each row in order, before
    /// `prepare_item_enchant`. Server-side.
    #[hook(
        dispatch = "flat",
        side = "server",
        contract = "listen",
        neoforge = "EnchantmentLevelSetEvent",
        cancellable = false,
        version = 2
    )]
    fn enchantment_level_set(
        pos: [i32; 3],
        offer: &mod_api::item::stations::EnchantmentRow<'_>,
        level: &mut i32,
    );
}
