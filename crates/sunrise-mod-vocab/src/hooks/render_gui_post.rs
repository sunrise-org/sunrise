//! The `render_gui_post` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// After the boss overlay and before the health and contextual HUD layers
    /// (skipped under F1; NeoForge's `RenderGuiEvent.Post`). Place a box with
    /// `corners.place` (boxes stack away from their edge) and draw it with
    /// `native_fill` and `native_text`.
    /// Use `ClientHost::debug_overlay_visible()` to avoid drawing over the debug overlay.
    /// Not heard when a mod canceled `render_gui_pre`. Client-side.
    #[hook(side = "client", contract = "listen", neoforge = "RenderGuiEvent.Post")]
    fn render_gui_post();
}
