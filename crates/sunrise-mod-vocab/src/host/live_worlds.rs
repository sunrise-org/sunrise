//! Experimental runtime-world projections; generated native and wasm handle methods.
#![allow(unused_variables)]
use crate::*;

/// Read boundary reached by projection handles.
#[doc(hidden)]
#[sunrise_mod_api_macros::host_area]
pub trait LiveWorldsBackend {
    /// Side: server.
    /// The caller-owned binding of this runtime level, if one is active.
    #[access(WorldOnly)]
    fn level_live_world(&self, level: LevelRef) -> Result<Option<LiveWorldRef>, LiveWorldError>;
    /// Side: server.
    /// Coherent accepted pose pair. Old generations and foreign owners refuse.
    #[access(WorldOnly)]
    fn live_world_sample(
        &self,
        live_world: LiveWorldRef,
    ) -> Result<LiveWorldSample, LiveWorldError>;
    /// Side: server.
    /// Read each sparse member's actual block state, collision shape and live entity handle.
    /// Air members are retained in the membership list and have empty collision geometry.
    #[access(WorldOnly)]
    fn live_world_cells(
        &self,
        live_world: LiveWorldRef,
    ) -> Result<Vec<LiveWorldCell>, LiveWorldError>;
    /// Side: server.
    /// Resolve a ray against the real sparse block collision shapes, including gaps.
    /// The ray has finite world coordinates and length at most 1024 blocks.
    #[access(WorldOnly)]
    fn live_world_clip(
        &self,
        live_world: LiveWorldRef,
        from: [f64; 3],
        to: [f64; 3],
    ) -> Result<Option<LiveWorldHit>, LiveWorldError>;
    /// Side: server.
    /// Check owner, generation, pose and source revisions and that the cell still exists.
    /// This is a read, not permission to mutate: ordinary reach/permissions still apply.
    #[access(WorldOnly)]
    fn live_world_validate_hit(
        &self,
        live_world: LiveWorldRef,
        hit: LiveWorldHit,
    ) -> Result<(), LiveWorldError>;
}

/// Write boundary reached by level and projection handles.
#[doc(hidden)]
#[sunrise_mod_api_macros::host_area]
pub trait LiveWorldsBackendMut {
    /// Side: server.
    /// Bind this mod's open Void runtime world to `destination`, with an explicit sparse
    /// list of local cells. The blocks, inventories, tickers, attachments and schedules
    /// remain their one real level instances. No source or destination block is rewritten.
    /// Requires resident cells and a checked collision-free starting pose. One binding per
    /// local world, subject to server per-owner and aggregate reservations; never disguises a vanilla entity as the owner.
    #[access(WorldOnly)]
    fn level_project(
        &mut self,
        level: LevelRef,
        destination: LevelRef,
        cells: &[[i32; 3]],
        pose: LiveWorldPose,
        limits: LiveWorldLimits,
    ) -> Result<LiveWorldRef, LiveWorldError>;
    /// Side: server.
    /// Publish an externally simulated pose with optimistic revision validation.
    /// The caller supplies finite bounded translation/rotation/work budgets. The
    /// engine checks the swept real shapes and commits only a completely clear step.
    /// A combined translation and turn is refused until a simultaneous-sweep route lands.
    /// No rigid-body integrator, block reconstruction or player teleport runs here.
    #[access(WorldOnly)]
    fn live_world_publish_pose(
        &mut self,
        live_world: LiveWorldRef,
        expected_revision: u64,
        pose: LiveWorldPose,
    ) -> Result<LiveWorldSample, LiveWorldError>;
    /// Side: server.
    /// Save the actual source/destination levels through their existing disk barriers, then
    /// durably publish this projection's metadata. Refuses inside lifted save/transaction
    /// contexts and if a callback changes the recorded revisions during the barrier.
    #[access(WorldOnly)]
    fn live_world_checkpoint(
        &mut self,
        live_world: LiveWorldRef,
    ) -> Result<LiveWorldCheckpoint, LiveWorldError>;
    /// Side: server.
    /// Restore this caller's saved binding after opening its real source and destination
    /// levels and making the member chunks resident. Parsing is bounded and all normal
    /// project/residency/collision checks run again. Old handles stay stale.
    #[access(WorldOnly)]
    fn level_restore_projection(&mut self, level: LevelRef)
    -> Result<LiveWorldRef, LiveWorldError>;
    /// Side: server.
    /// Remove only this projection, preserving the local world's blocks and saved data.
    /// An old handle cannot become current again. This is not disassembly or deletion.
    #[access(WorldOnly)]
    fn live_world_retire(&mut self, live_world: LiveWorldRef) -> Result<(), LiveWorldError>;
}
