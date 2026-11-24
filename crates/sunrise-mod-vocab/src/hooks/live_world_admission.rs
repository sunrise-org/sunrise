//! Prospective source ownership admission for an ordinary projected placement.
pub trait Hooks {
    /// Before an ordinary block item changes a projected source, the exact bounded canonical
    /// writes and shapes have passed host coordinate, ownership, residency, shape, stream and
    /// collision budgets. The projection owner may cancel to enforce its supported state
    /// contract before either the block or held stack changes. Other ordinary protection mods
    /// may cancel too. The host keeps this decision and revalidates source, pose, membership
    /// and player-storage epochs after callbacks; it refuses stale decisions without selecting
    /// another placement. Listener writes and refusal use the normal host transaction.
    #[hook(
        target = "sunrise_client::game::native_hooks::live_world_admission::Game<'_>::native_live_world_admission",
        dispatch = "flat",
        side = "server",
        contract = "cancel",
        out = "canceled",
        cancellable = true,
        neoforge = "none (Sunrise live world placement admission)"
    )]
    fn live_world_admission(
        live_world: LiveWorldRef,
        player: Option<PlayerRef>,
        changes: &[LiveWorldPlacementCell],
        canceled: &mut bool,
    );
}
