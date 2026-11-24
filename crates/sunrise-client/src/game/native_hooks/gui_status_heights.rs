use super::*;

impl Game<'_> {
    /// NeoForge's `Hud.leftHeight` and `rightHeight` while the HUD layers draw.
    pub(in crate::game) fn gui_status_heights(&self) -> Option<[i32; 2]> {
        self.hud_layer_next
            .map(|_| self.hud_status_heights.heights())
    }
}
