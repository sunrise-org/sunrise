//! The `grindstone_place` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A grindstone at `pos` made its result after its inputs changed, for
    /// the server player `player` (UUID) and `player_entity_id`
    /// (`GrindstoneEvent.OnPlaceItem`, in `GrindstoneMenu.createResult`):
    /// `place` holds the inputs, the vanilla result and an initially empty
    /// override. Set `output` to replace the result and `xp` (0 or more)
    /// for what taking it gives; set `cancel` for no result.
    /// The hook runs inside an action on the guest's open grindstone. A cancel, an answer the
    /// game refuses (`xp` below -1, an output with a count below 1), a write to a player's
    /// inventory or cursor or to the inputs, or a failing listener leaves no result and undoes
    /// every write the listener made. A backing block that changed and was restored still
    /// makes the place stale. Server-side.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "cancel",
        neoforge = "GrindstoneEvent.OnPlaceItem",
        cancellable = true,
        version = 2
    )]
    fn grindstone_place();
}
