//! The `grindstone_take` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// The server player `player` (UUID) and `player_entity_id` took a
    /// grindstone's result at `pos` (`GrindstoneEvent.OnTakeItem`, in its
    /// result slot's `onTake`): `inputs` lends the pre-take stacks. Use `edits.xp` to
    /// change the experience (-1 for vanilla's, or a nonnegative award) and
    /// `edits.new_top` and `edits.new_bottom` to leave items in the input slots.
    /// Set `cancelled` to skip the rest of `onTake`: the result is still taken,
    /// but no experience or sound is given, and the inputs stay.
    /// The click, this hook and the take's own effects are one action. What a listener writes
    /// through the `Host` stays only when the take is applied; a cancel keeps the click and
    /// undoes the listener's writes. Invalid remainders refuse even a cancelled answer.
    /// An uncancelled XP below `-1`, a stale read, or a failing listener also refuses the
    /// whole take: the result, inputs, cursor, inventory and drops go back as they were.
    /// Writing to the player's inventory or cursor can make the take stale, as can changing
    /// the backing block even if it is restored. Accepted writes and experience remain
    /// tentative until the outermost transaction commits. Server-side.
    #[hook(
        dispatch = "flat",
        side = "server",
        contract = "cancel",
        out = "cancel",
        neoforge = "GrindstoneEvent.OnTakeItem",
        cancellable = true,
        version = 3
    )]
    fn grindstone_take(
        player: u128,
        player_entity_id: i32,
        pos: [i32; 3],
        inputs: &mod_api::item::menu::GrindstoneTakeInputs<'_>,
        edits: &mut mod_api::item::menu::GrindstoneTakeEdits,
        cancel: &mut bool,
    );
}
