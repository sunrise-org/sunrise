//! The `level_ticking` area: what the level ticks now, and running a tick by hand. Host method declarations; access defaults and game forwarders are generated.
#![allow(unused_variables)]
use crate::*;
/// The handles' reads of the `level_ticking` area, implemented by the game and reached through [`Backend`].
// hidden: a handle's backend, reached through `Backend`; mods call the handle, never this trait
#[doc(hidden)]
#[sunrise_mod_api_macros::host_area]
pub trait LevelTickingBackend {
    /// Side: server.
    /// Whether this is the level in place: the one the server ticks now, whose block entities
    /// the game's own capabilities read and write and whose capabilities
    /// `LevelRef::capability` looks up. False for a level parked while the server ticks
    /// another (its blocks can still be read), for a handle of an earlier open, and on a
    /// client.
    fn level_in_place(&self, level: LevelRef) -> bool;
    /// Side: server.
    /// `Some(true)` when this resident, in-bounds position has entity-ticking tickets.
    /// `Some(false)` for unloaded or out-of-bounds positions or tickets that forbid ticking.
    /// With no ticket system, resident in-bounds chunks tick; an active but empty ticket
    /// system permits none. This reads the current ticket decision, not a tick guarantee.
    /// `None` for stale handles, a client or a level not currently in place. Parked
    /// levels cannot answer because the live ticket state belongs to the level in place.
    /// Does not load chunks. Each call shares the ticket map through a reference count;
    /// it does not allocate a memo table or refresh a resident-chunk cache.
    fn level_entity_ticking(&self, level: LevelRef, pos: [i32; 3]) -> Option<bool>;
    /// Side: server.
    /// `Some(true)` when scheduled blocks, block events and block entities can tick here:
    /// a resident, in-bounds position with block-ticking tickets and no pending entities.
    /// `Some(false)` for unloaded or out-of-bounds positions, tickets that forbid ticking,
    /// or a chunk waiting for its entities. With no ticket system, resident in-bounds
    /// chunks with no pending entities tick; an active but empty ticket system permits none.
    /// `None` for stale handles, a client or a level not currently in place. Parked
    /// levels cannot answer because the live ticket state belongs to the level in place.
    /// Does not load chunks. Each call shares the ticket map through a reference count
    /// and borrows the pending-entity set without copying it or allocating a memo table.
    fn level_should_tick_blocks_at(&self, level: LevelRef, pos: [i32; 3]) -> Option<bool>;
}
/// The handles' writes of the `level_ticking` area, implemented by the game and reached through [`BackendMut`].
// hidden: a handle's backend, reached through `Backend`; mods call the handle, never this trait
#[doc(hidden)]
#[sunrise_mod_api_macros::host_area]
pub trait LevelTickingBackendMut {
    /// Side: server.
    /// Run the block at `pos`'s scheduled tick now (`ServerLevel.tickBlock`): the path a due
    /// tick from the queue takes, the block's part first, else `BlockState.tick`, with the same
    /// random draws and level events. The queue is not read or changed; `schedule_block_tick`
    /// queues one. False for a refused write, a stale or parked handle, a client and an
    /// unloaded or out-of-height position.
    fn level_block_tick(&mut self, level: LevelRef, pos: [i32; 3]) -> bool;
    /// Side: server.
    /// Random-tick `pos` now, as `ServerLevel.tickChunk` does for a position it picked: the
    /// block's `randomTick` when it ticks randomly (its part first), then lava's fluid random
    /// tick, with the same random draws and level events. False as for `block_tick`.
    fn level_block_random_tick(&mut self, level: LevelRef, pos: [i32; 3]) -> bool;
    /// Side: server.
    /// Queue a tick of `block` at `pos` in `delay` game ticks
    /// (`ScheduledTickAccess.scheduleTick(pos, block, delay, priority)`, which `LevelAccessor`
    /// inherits and `ServerLevel.getBlockTicks().schedule` backs): the queue `update_shape`'s
    /// `schedule_block_tick` fills, so the tick is saved with its chunk and is ignored when the
    /// block at `pos` is no longer `block` as it comes due. Run order (`LevelTicks.collectTicks`):
    /// a chunk's own queue runs by trigger tick, then priority, then the order scheduled. Across
    /// chunks there is no single order: each chunk with a due tick competes by its next tick's
    /// priority, then the order scheduled, not by trigger tick. Ticks that come due on the same
    /// game tick therefore run by priority, then the order scheduled, in every chunk; but when
    /// ticks of different trigger ticks are due together (a chunk that could not tick, a pass
    /// over its budget), an older `Low` tick in one chunk can run after a newer `High` tick in
    /// another. `Ok(true)` when queued or already queued:
    /// vanilla keeps one tick per block and position (`LevelChunkTicks.schedule`), so a repeat
    /// with another delay or priority is dropped and the first keeps its deadline and priority.
    /// Inside a transaction the tick is queued when it commits, and its delay counts from there.
    /// `Ok(false)` when the position's chunk has no tick container (unloaded, or loaded with no
    /// tick list yet), for a position a carrier transfer holds and for a block with no world
    /// form. The height is not checked, as `LevelTicks.schedule` checks only the container.
    /// `Err` for a stale handle, a client or a level not in place. The in-line hosts'
    /// `LevelTicksHost::schedule_block_tick_with_priority` makes the same engine call into the
    /// same queue with the same [`TickPriority`].
    fn level_schedule_block_tick(
        &mut self,
        level: LevelRef,
        pos: [i32; 3],
        block: BlockKey,
        delay: i32,
        priority: TickPriority,
    ) -> Result<bool, LevelWriteError>;
    /// Side: server.
    /// Queue a tick of `fluid` at `pos` in `delay` game ticks
    /// (`ScheduledTickAccess.scheduleTick(pos, fluid, delay, priority)`), the queue
    /// `level_fluid_tick`'s due tick comes from. Results as for `schedule_block_tick`, one tick
    /// per fluid and position; `Ok(false)` also for a fluid with no world form.
    fn level_schedule_fluid_tick(
        &mut self,
        level: LevelRef,
        pos: [i32; 3],
        fluid: FluidKey,
        delay: i32,
        priority: TickPriority,
    ) -> Result<bool, LevelWriteError>;
    /// Side: server.
    /// Lend the named level's live stream to `draw`. Run no callback when the level is
    /// unavailable. See [`LevelRef::with_random`].
    #[decline_unavailable]
    fn with_level_random(&mut self, level: LevelRef, draw: RandomDraw<'_>);
}
