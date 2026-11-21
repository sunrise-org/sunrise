use super::*;

/// The hook points. Each does nothing in Sunrise.
impl Game<'_> {
    /// After the boss overlay and before the health and contextual HUD layers
    /// (skipped under F1; `RenderGuiEvent.Post`). Place a box with
    /// `corners.place` (boxes stack away from their edge) and
    /// draw it with `native_fill` and `native_text`. Skip a corner
    /// `native_corner_shown` says is taken.
    pub(in crate::game) fn native_render_gui_post(
        &self,
        draw: &mut HudBatch,
        corners: &mut Corners,
    ) {
        self.fire_render_gui_post(draw, corners);
    }
}
