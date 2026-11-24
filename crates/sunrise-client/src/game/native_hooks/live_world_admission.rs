use super::*;
use mod_api::{LiveWorldPlacementCell, LiveWorldRef};
impl Game<'_> {
    /// Exact prospective placement into a live-world owner, before mutation and consumption.
    pub(in crate::game) fn native_live_world_admission(
        &mut self,
        live_world: LiveWorldRef,
        player: Option<PlayerRef>,
        changes: &[LiveWorldPlacementCell],
        canceled: &mut bool,
    ) {
        self.fire_live_world_admission(live_world, player, changes, canceled);
    }
}
