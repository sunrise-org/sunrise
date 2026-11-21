//! The `general_entity_tick` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// The authoritative tick of one general entity row of type `kind`
    /// (`mymod:robot`), with ID `entity`, in the server level's entity phase:
    /// `ServerLevel.tickNonPassenger`'s `entity.tick()` for a row the type
    /// registry owns (#792 stage 4). The row's common bookkeeping has run
    /// (its previous position is its current one); a rider is ticked after
    /// its vehicle, depth-first as `tickPassenger` recurses. A passenger
    /// dismounted before its join-order slot can tick as a root if it has
    /// not already ticked this pass. Rows that joined this tick first tick
    /// next pass; parked levels and rows outside entity-ticking chunks do
    /// not tick. Canceling `entity_tick_pre` skips the row's own tick;
    /// cancellation alone does not skip its passengers' ticks. A listener
    /// can separately change the riding graph. Read and change the row's
    /// data through its attachments (`event.entity` is their holder);
    /// there is no parallel per-entity state. Told only for general entity
    /// types: vanilla families tick their own paths (#6509 owns bringing
    /// them to one join-order walk), so a part attached to `minecraft:zombie`
    /// hears no tick from this hook. Server-side.
    #[hook(
        side = "server",
        contract = "listen",
        scope = "entity",
        key = "kind",
        neoforge = "none (vanilla Entity.tick has no part-scoped counterpart; NeoForge's EntityTickEvent is the global pre/post pair)"
    )]
    fn general_entity_tick();
}
