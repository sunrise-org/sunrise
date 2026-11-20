//! The `block_drops` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// NeoForge's `BlockDropsEvent`, on the authoritative server after block loot and item
    /// preparation. This also covers drop-eligible explosion hits and empty loot rolls.
    /// `cause` describes the destruction; `breaker`, `breaker_entity` and `tool` describe
    /// the actual break, not the owner of a blast.
    ///
    /// `block_entity` is a read-only snapshot reference to the outgoing block entity.
    /// `level.block_entity_data` reads the exact payload used by loot, including admitted
    /// `block_entity_save_additional` output. The reference is stale after dispatch, even
    /// on unwind. A block without an entity has no reference.
    ///
    /// Edit `drops` and `experience` together, or set `cancel` to suppress those outputs
    /// and `spawnAfterBreak` effects. An invalid edit leaves the entire original decision
    /// intact. Cancellation does not undo destruction or suppress separate removal spills.
    /// The host is read-only; the enclosing write publishes accepted drops.
    ///
    /// In a blast, the event owns only this block's share of a merged item entity.
    /// Unchanged shares rejoin their original entity; a no-op listener preserves loot and
    /// random order. Primed TNT does not enter the drop path.
    #[notes = r#"## Block drops

`block_drops(level, event)` supplies the outgoing state, typed cause, breaker and tool, and
an optional outgoing block-entity snapshot. `level.block_entity_data` reads the same captured
payload as loot, even for an explosion; the reference is usable only during dispatch. Edit
`drops` and `experience`, or set `cancel` to suppress those and `spawnAfterBreak` effects.
Invalid edits are refused together. Destruction and separate removal spills are not canceled.
Blast events own only their block's contribution to the merged output. Empty loot still fires
when the block enters the drop path. See the block experience and explosions guide."#]
    #[hook(
        side = "server",
        contract = "cancel",
        out = "cancel",
        neoforge = "BlockDropsEvent",
        cancellable = true,
        version = 3,
        target = "sunrise_client::game::native_hooks::block_drops::GameLevel<'_>::native_block_drops"
    )]
    fn block_drops(
        dimension: LevelDimension,
        pos: BlockPos,
        state: BlockState,
        cause: BlockDestructionCause,
        breaker: Option<u128>,
        breaker_entity: Option<EntityRef>,
        block_entity: Option<BlockEntityRef>,
        tool: Option<&ItemStack>,
        drops: &mut Vec<ItemDrop>,
        experience: &mut u32,
        cancel: &mut bool,
    );
}
