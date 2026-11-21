//! The `grindstone_place` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A grindstone at `pos` made its result after its inputs changed, for
    /// the server player `player` (UUID) and `player_entity_id`
    /// (`GrindstoneEvent.OnPlaceItem`, in `GrindstoneMenu.createResult`):
    /// `inputs` lends the input stacks and vanilla result. Leave `edits.output` at
    /// `None` to keep vanilla's result and experience, or set a replacement
    /// and its `edits.xp` (`-1` for vanilla's award, or a nonnegative amount).
    /// The hook runs inside an action on the guest's open grindstone. Cancellation, an
    /// invalid replacement stack, `edits.xp` below `-1` on an uncancelled answer,
    /// a write to a player's inventory or cursor or to the inputs, or a failing listener
    /// leaves no result and undoes every write the listener made. A backing block that
    /// changed and was restored still makes the place stale. Server-side.
    #[hook(
        dispatch = "flat",
        side = "server",
        contract = "cancel",
        out = "cancel",
        neoforge = "GrindstoneEvent.OnPlaceItem",
        cancellable = true,
        version = 3
    )]
    fn grindstone_place(
        player: u128,
        player_entity_id: i32,
        pos: [i32; 3],
        inputs: &mod_api::item::menu::GrindstonePlaceInputs<'_>,
        edits: &mut mod_api::item::menu::GrindstonePlaceEdits,
        cancel: &mut bool,
    );
}
