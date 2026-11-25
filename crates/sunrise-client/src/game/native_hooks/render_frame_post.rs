use super::render_frame_pre::FrameMoments;
use super::*;

/// The hook points. Each does nothing in Sunrise.
impl Game<'_> {
    /// Fire `render_frame_post` for a frame whose `render_frame_pre` fired, with the values
    /// that Pre reported.
    pub(in crate::game) fn finish_render_frame(
        &mut self,
        elapsed: Duration,
        moments: FrameMoments,
    ) {
        self.extras.frame_partial_ticks = None;
        self.native_render_frame_post(
            elapsed,
            moments.world_partial_tick,
            moments.partial_tick,
            moments.realtime_delta_ticks,
        );
    }

    /// Once a frame that reached the render call, right after it was rendered and before
    /// `flip_frame` (`RenderFrameEvent.Post`, around `GameRenderer.render`).
    pub(in crate::game) fn native_render_frame_post(
        &mut self,
        elapsed: Duration,
        world_partial_tick: f32,
        partial_tick: f32,
        realtime_delta_ticks: f32,
    ) {
        self.fire_render_frame_post(
            elapsed,
            world_partial_tick,
            partial_tick,
            realtime_delta_ticks,
        );
    }
}
