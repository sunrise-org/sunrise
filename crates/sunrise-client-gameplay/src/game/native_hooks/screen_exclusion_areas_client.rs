/// The hook points. Each does nothing in Sunrise.
impl crate::game::ClientGame<'_> {
    /// Where the open screen draws outside its `rect`, and where mods draw
    /// over it: the recipe book's panel, the creative tab rows, the active
    /// effects beside the inventory, then whatever `native_screen_exclusion_areas`
    /// adds. Each is `[x, y, width, height]` in GUI pixels; an empty or
    /// non-finite one is dropped. A mod that lays out its own panels
    /// (a recipe viewer) keeps them clear of these. Client-side; empty with
    /// no screen up.
    pub(in crate::game) fn screen_exclusion(&self) -> Vec<[f32; 4]> {
        let Some(view) = self.observed_screen_view() else {
            return Vec::new();
        };
        // The framebuffer `Renderer::render` leaves behind while it has the renderer out, so a
        // frame callback reads the areas too.
        let Some(framebuffer) = self.renderer_framebuffer() else {
            return Vec::new();
        };
        let frame = self.gui_frame_for(framebuffer);
        let mut areas = self.screen_own_exclusion(frame.size());
        self.native_screen_exclusion_areas(&view, &mut areas);
        areas.retain(|a| a.iter().all(|v| v.is_finite()) && a[2] > 0.0 && a[3] > 0.0);
        areas
    }
}
