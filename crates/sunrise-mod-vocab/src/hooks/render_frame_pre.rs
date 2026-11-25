//! The `render_frame_pre` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// Once a frame that reaches the render call, in a world or not, after the
    /// frame's extraction has finished and just before it is rendered
    /// (NeoForge's `RenderFrameEvent.Pre`, fired around `GameRenderer.render`).
    /// `elapsed` is the frame's time. `world_partial_tick` is the interpolation
    /// about to be drawn, held while paused or frozen; `partial_tick` is the
    /// window clock, which keeps advancing. Outside a world both use the window
    /// clock. `realtime_delta_ticks` is the real time of the last frame in ticks
    /// of the default length, whatever the tick rate or pause (vanilla's
    /// `DeltaTracker.getRealtimeDeltaTicks`: 0.5 when it is over 7).
    /// The frame is already extracted, so a listener's writes do not change what
    /// this frame draws, nor its captured interpolation; they reach the next
    /// frame. A frame that is skipped before the render call fires nothing; each
    /// frame that fires this fires `render_frame_post` once. Client-side.
    #[hook(
        dispatch = "flat",
        side = "client",
        contract = "listen",
        version = 3,
        neoforge = "RenderFrameEvent.Pre"
    )]
    fn render_frame_pre(
        elapsed: Duration,
        world_partial_tick: f32,
        partial_tick: f32,
        realtime_delta_ticks: f32,
    );
}
