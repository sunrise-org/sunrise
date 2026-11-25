//! The `render_frame_post` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// Once a frame that reached the render call, in a world or not, right after
    /// the frame was rendered and before `flip_frame` (NeoForge's
    /// `RenderFrameEvent.Post`, fired around `GameRenderer.render`). It pairs
    /// with `render_frame_pre`: it fires only on a frame whose Pre fired, and
    /// is not a callback that runs on skipped frames. A frame the surface then
    /// refused to present still fires it; `flip_frame` is the moment a frame was
    /// presented. `elapsed` is the frame's time. `world_partial_tick` is the
    /// interpolation used by that frame, held while paused or frozen;
    /// `partial_tick` is the advancing window clock. Outside a world both use
    /// the window clock. Neither is recomputed after listeners run.
    /// `realtime_delta_ticks` is the value Pre reported (0.5 when the real time
    /// was over 7 ticks). Client-side.
    #[hook(
        dispatch = "flat",
        side = "client",
        contract = "listen",
        version = 3,
        neoforge = "RenderFrameEvent.Post"
    )]
    fn render_frame_post(
        elapsed: Duration,
        world_partial_tick: f32,
        partial_tick: f32,
        realtime_delta_ticks: f32,
    );
}
