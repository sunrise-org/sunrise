//! The `render_frame_post` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// Once after a render call returns, in a world or not, and before `flip_frame`
    /// (NeoForge's `RenderFrameEvent.Post`). It pairs with `render_frame_pre`, even
    /// when the draw fails or does not present; no render call means neither fires.
    /// `elapsed`, `world_partial_tick` and `partial_tick` keep the snapshot Pre
    /// received, without recomputing it after listeners run. Client-side.
    #[hook(
        dispatch = "flat",
        side = "client",
        contract = "listen",
        version = 3,
        neoforge = "RenderFrameEvent.Post"
    )]
    fn render_frame_post(elapsed: Duration, world_partial_tick: f32, partial_tick: f32);
}
