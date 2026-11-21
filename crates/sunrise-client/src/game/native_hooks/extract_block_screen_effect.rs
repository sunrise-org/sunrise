use super::*;

/// The hook points. Each does nothing in Sunrise.
impl Game<'_> {
    /// Before one of the first-person view's block overlays (`ScreenEffectRenderer`) is drawn,
    /// once a frame each is shown: `Block` with the view-blocking `state` at `pos` the camera is
    /// inside, `Water` with the `state` at the eye's block `pos` while the eye is under water, and
    /// `Fire` with fire's default `state` at the player's block `pos` while they burn. Never for a
    /// spectator; `Fire` only in first person. Cancellation leaves that vanilla overlay out,
    /// but not the selected renderer. `partial_tick` is the world's. Client-side.
    pub(in crate::game) fn native_extract_block_screen_effect(
        &self,
        effect: mod_api::BlockScreenEffect,
        state: mod_api::BlockState,
        pos: BlockPos,
        partial_tick: f32,
        canceled: &mut bool,
        renderer: &mut Option<mod_api::__private::SelectedBlockScreenEffectRenderer>,
    ) {
        self.fire_extract_block_screen_effect(effect, state, pos, partial_tick, canceled, renderer);
    }
}
