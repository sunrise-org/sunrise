//! The `render_frame_pre` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// Once per render call, in a world or not, after extraction and before drawing
    /// (NeoForge's `RenderFrameEvent.Pre`). Texture uploads can reach this draw, but
    /// they cannot change already-extracted geometry. Closing a screen or clearing
    /// particles affects the next frame. Neither hook fires when no render call is made.
    /// `elapsed` is the frame's time. `world_partial_tick` is the interpolation about
    /// to be drawn, held while paused or frozen; `partial_tick` is the advancing
    /// window clock. Outside a world both use the window clock. Client-side.
    #[hook(
        dispatch = "flat",
        side = "client",
        contract = "listen",
        version = 3,
        neoforge = "RenderFrameEvent.Pre"
    )]
    fn render_frame_pre(elapsed: Duration, world_partial_tick: f32, partial_tick: f32);
}
