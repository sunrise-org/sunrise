//! The `collision_shape` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// The collision shape of the block state `state` at `pos` is being read for the collision
    /// query `query` (vanilla's `BlockBehaviour.getCollisionShape(state, level, pos, context)`),
    /// by an entity's move, a placement check or a projectile's path. `query` is vanilla's
    /// `EntityCollisionContext`, captured when the query was made: the entity's id and class,
    /// descending, placement, always-fluid, the main-hand item, and `is_above`. `shape`
    /// starts as `None`, which keeps the shape the block would otherwise have (its declared one,
    /// a context-keyed one, or the one its block entity published); a part that answers
    /// `Some(shape)` replaces it for this query with a block-local [`VoxelShape`]. Build it
    /// with `VoxelShape::try_from_boxes`, which rejects invalid geometry rather than imposing
    /// a box count. A Part that panics or traps loses only its answer; the previously accepted
    /// shape stands, and later Parts can still answer. It is asked on the owning tick thread
    /// once for each distinct query per tick. Fluid passage and face support never ask. It
    /// is given the loop `context` of the side that ticks the entity and takes no `Host`: the
    /// part answers from `query` and the state it keeps itself, keyed by the entity's id and
    /// synced to the client, which asks the same question for its own prediction. A collision
    /// query made inside the callback gets the empty-context answer, never a nested call.
    /// Diverges from upstream: new hook.
    #[notes = r#"## Collision shape by the querying entity

- **Block Part method `collision_shape(context, event)`.** Set `*event.shape = Some(shape)` to override the shape for this query (`BlockBehaviour.BlockStateBase.getCollisionShape`; NeoForge adds no event here). Construct a block-local `VoxelShape` with `VoxelShape::try_from_boxes`; if construction fails, leave the answer unchanged. `None` selects the normal, contextual or block-entity-published shape. If a Part panics, only that invocation's answer is discarded; earlier answers stand and later Parts still run.
- **Who asks, and when.** Entity movement (`Entity.move` and its step-up retries), placement checks, projectile paths and the other reads that take a `CollisionContext`; never fluid passage or face support. A part is asked once for each distinct question in a tick (the state, the position, and every input of the query), and later reads of that tick (a step-up retry, a second sweep) get the same answer, so a shape cannot change in the middle of a move. A collision query made inside the callback gets the empty-context answer (the block's normal shape), never a nested call. The per-state shape cache stays the empty-context answer, so a block still counts as a large shape where its normal shape is one. It runs on the thread that owns the level, the server tick and the client's own prediction; any other thread (the mesher, world generation) reads the normal shape.
- **Why there is no `Host`.** A move holds the level's collision data and cannot lend a `Host` to a mod in the middle of it, as for `entity_move`, `friction` and the other loop-context hooks. The part answers from the query and from state it keeps itself: a rule that reads more than the query (Create's belt asks whether the entity is in the belt's passenger map; powder snow reads a fall distance) tracks that state in the mod, keyed by `query.entity()`, and syncs it so the client computes the same answer. If a real mod needs an entity read it cannot track itself, one captured field is added to `CollisionQuery`; the `Host` is not reopened.
- **The client's duty.** A client that disagrees with the server about the mod's own state predicts a different shape; the server then corrects the player's position (the usual "moved wrongly" correction, not a kick). Sync the state the rule reads.
- **Cost.** A block with no part that answers it costs nothing. An opted-in block costs one call per distinct question in a tick. Diverges from upstream: new hook."#]
    #[hook(
        side = "both",
        version = 2,
        contract = "listen",
        context = "loop",
        scope = "block",
        key = "state",
        cancellable = false,
        target = "sunrise_client_lib::collision_parts::native_collision_shape",
        neoforge = "none (vanilla BlockBehaviour.getCollisionShape with its CollisionContext; NeoForge adds no event or extension for it)"
    )]
    fn collision_shape(
        pos: BlockPos,
        state: BlockState,
        query: &CollisionQuery,
        shape: &mut Option<VoxelShape>,
    );
}
