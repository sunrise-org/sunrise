//! The `grindstone_take` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// The server player `player` (UUID) and `player_entity_id` took a
    /// grindstone's result at `pos` (`GrindstoneEvent.OnTakeItem`, in its
    /// result slot's `onTake`): `take` holds the inputs, the experience (-1
    /// for vanilla's, or what `grindstone_place` set) and what the inputs
    /// hold afterwards (empty). Set `xp` (0 or more) to give that much
    /// experience, `new_top` and `new_bottom` to leave items in the inputs.
    /// Set `cancel` to skip the rest of `onTake`: the result is still taken,
    /// but no experience, no sound, and the inputs stay.
    /// The click, this hook and the take's own effects are one action. What a listener writes
    /// through the `Host` stays only when the take is applied; a cancel keeps the click and
    /// undoes the listener's writes. An answer the game refuses (`xp` below -1, an empty
    /// `new_top` or `new_bottom` stack, a write to the player's inventory or cursor or to
    /// anything the take read, a failing listener) refuses the whole take: the result, the
    /// inputs, the cursor, the inventory and the drops go back as they were. A backing block
    /// that changed and was restored still makes the take stale. Accepted writes and experience
    /// remain tentative until the outermost transaction commits. Server-side.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "cancel",
        neoforge = "GrindstoneEvent.OnTakeItem",
        cancellable = true,
        version = 2
    )]
    fn grindstone_take();
}
