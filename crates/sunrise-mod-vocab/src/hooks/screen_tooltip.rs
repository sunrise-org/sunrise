//! The `screen_tooltip` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0030

pub trait Hooks {
    /// The tooltip at `at` (GUI pixels from the top left) of the screen of
    /// mod menu type `menu` (`block` as in `screen`, including menus and blocks
    /// in a content package served by this mod via `served_by`).
    /// `slot` is the hovered menu index, including player inventory slots,
    /// or `None` outside a mapped slot. On a filled slot, `lines` starts as
    /// the item's tooltip after item tooltip hooks; your final lines replace
    /// it, as `AbstractContainerScreen.getTooltipFromContainerItem` does.
    /// Empty slots and screen space start empty. Images, style and the carried
    /// stack visibility rule are unchanged. Lines are rich Text/color pairs,
    /// so editing one line does not flatten the styling of the others.
    ///
    /// `pointer` is the frame's absolute integer pointer in scaled GUI pixels
    /// (truncated toward zero); `gui_size` is that frame's scaled width and height.
    /// `origin` is the panel's top-left in the same coordinates. The pointer may
    /// be outside the panel; use these frame values rather than caching a size.
    #[hook(
        version = 4,
        side = "client",
        contract = "collect",
        out = "lines",
        scope = "menu",
        key = "menu",
        neoforge = "none (see screen)"
    )]
    fn screen_tooltip(
        menu: &str,
        block: Option<&str>,
        data: &[i32],
        at: [f32; 2],
        slot: Option<u32>,
        origin: [f32; 2],
        pointer: [i32; 2],
        gui_size: [i32; 2],
        lines: &mut Vec<(Text, u32)>,
    );
}
