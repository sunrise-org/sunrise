//! The `screen_draw` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0028

pub trait Hooks {
    /// Draw the screen of mod menu type `menu` at `origin` (its top left, GUI
    /// pixels), over a plain panel with its slots' frames and under the items
    /// and labels, from the open window's data slots `data`. A block-backed
    /// menu still carries the block entity's values as vanilla signed shorts;
    /// a blockless menu carries the full signed `i32` from its opener and
    /// `MenuRef::set_data_slot`. Data is empty until the server's first values
    /// arrive, so read it with `get`. Draw with `event.draw.sprite`,
    /// `event.draw.fill` and `event.draw.text`.
    /// `block` is the mod block whose menu it is, as in `screen` (including
    /// menus and blocks in a content package served by this mod via `served_by`).
    /// `pos` is the open menu's block position, as reported by the server
    /// (or the clicked block for a server without menu-extra support).
    /// It is `None` for a menu opened without a block position. Read the
    /// client level's current view at this position through the existing
    /// both-sided level reads; it is not an authoritative server snapshot.
    ///
    /// `pointer` is the frame's absolute integer pointer in scaled GUI pixels
    /// (truncated toward zero); `gui_size` is that frame's scaled width and height.
    /// `origin` is the panel's top-left in the same coordinates. The pointer may
    /// be outside the panel; use these frame values rather than caching a size.
    #[hook(
        version = 5,
        side = "client",
        contract = "listen",
        scope = "menu",
        key = "menu",
        neoforge = "none (see screen)"
    )]
    fn screen_draw(
        menu: &str,
        block: Option<&str>,
        pos: Option<[i32; 3]>,
        data: &[i32],
        draw: &mut dyn DrawContext,
        origin: [f32; 2],
        pointer: [i32; 2],
        gui_size: [i32; 2],
    );
}
