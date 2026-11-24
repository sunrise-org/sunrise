//! The `level_ticking` area: what the level ticks now, and running a tick by hand. Host method declarations; access defaults and game forwarders are generated.
#![allow(unused_variables)]
use crate::*;
/// The handles' reads of the `level_ticking` area, implemented by the game and reached through [`Backend`].
// hidden: a handle's backend, reached through `Backend`; mods call the handle, never this trait
#[doc(hidden)]
#[sunrise_mod_api_macros::host_area]
pub trait LevelTickingBackend {
    /// Side: server.
    /// Current actual block and block-entity content read epochs without a projection or chunk
    /// load. Reads an ordinary open level or this enabled mod's owned open parked runtime level,
    /// including an empty destination. Stale/closing targets refuse `Stale`; a client,
    /// unavailable phase or foreign parked runtime target refuses `Unavailable`.
    /// These epochs do not certify durable completion.
    fn level_revisions(&self, level: LevelRef) -> Result<LevelRevisions, LevelReadError>;
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
    /// Reads any open server level, including one parked while another level ticks, using
    /// that level's own current ticket map and pending-entity set. `None` for stale,
    /// closed or unavailable handles and on a client. A positive answer is a current
    /// readiness decision, not a promise that the scheduler will activate an otherwise
    /// idle level: request its ordinary forced chunks from a running/global callback.
    /// Does not load chunks or switch levels. Each call shares the ticket map through a
    /// reference count and borrows pending entities without copying or allocating a memo table.
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
    /// Lend the named level's live stream to `draw`. Run no callback when the level is
    /// unavailable. See [`LevelRef::with_random`].
    #[decline_unavailable]
    fn with_level_random(&mut self, level: LevelRef, draw: RandomDraw<'_>);
}
