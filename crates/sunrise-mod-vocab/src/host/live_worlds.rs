//! Experimental runtime-world projections; generated native handle methods.
#![allow(unused_variables)]
use crate::*;

/// Read boundary reached by projection handles.
// hidden: generated handle backend; mods use the natural receiver methods.
#[doc(hidden)]
#[sunrise_mod_api_macros::host_area]
pub trait LiveWorldsBackend {
    /// Side: server.
    /// Checked sparse selection footprint before creating a binding or changing chunks.
    /// Reads this mod's open Void source, including a parked source; loads nothing.
    /// `retained_columns` includes the existing forced-streaming terrain ring, so callers
    /// can choose an adequate `LiveWorldLimits::chunks`. Destination sweep, real shapes,
    /// residency, other viewers and owner/global reservations are still checked by `project`.
    /// Refuses selections that cannot fit the engine's checked coordinate/cover ceilings.
    fn level_projection_footprint(
        &self,
        level: LevelRef,
        cells: &[[i32; 3]],
    ) -> Result<LiveWorldFootprint, LiveWorldError>;
    /// Side: server.
    /// The caller-owned binding of this runtime level, if one is active.
    fn level_live_world(&self, level: LevelRef) -> Result<Option<LiveWorldRef>, LiveWorldError>;
    /// Side: server.
    /// Coherent accepted pose pair. Old generations and foreign owners refuse.
    fn live_world_sample(
        &self,
        live_world: LiveWorldRef,
    ) -> Result<LiveWorldSample, LiveWorldError>;
    /// Side: server.
    /// Read each sparse member's actual block state, collision shape and live entity handle.
    /// Successful ordinary removal updates the sparse membership.
    fn live_world_cells(
        &self,
        live_world: LiveWorldRef,
    ) -> Result<Vec<LiveWorldCell>, LiveWorldError>;
    /// Side: server.
    /// Resolve a ray against the real sparse block collision shapes, including gaps.
    /// The ray has finite world coordinates and length at most 1024 blocks.
    fn live_world_clip(
        &self,
        live_world: LiveWorldRef,
        from: [f64; 3],
        to: [f64; 3],
    ) -> Result<Option<LiveWorldHit>, LiveWorldError>;
    /// Side: server.
    /// Check owner, generation, pose and source revisions and that the cell still exists.
    /// This is a read, not permission to mutate: ordinary reach/permissions still apply.
    fn live_world_validate_hit(
        &self,
        live_world: LiveWorldRef,
        hit: LiveWorldHit,
    ) -> Result<(), LiveWorldError>;
    /// Side: server.
    /// Read bounded saved membership/residency requirements without loading or reserving chunks.
    /// Earlier experimental data explicitly reports migration required; missing alone is NotSaved.
    fn level_saved_projection(
        &self,
        level: LevelRef,
    ) -> Result<LiveWorldSavedProjection, LiveWorldError>;
    /// Side: server.
    /// Actual completed common decision, Busy while staged, NotCommitted for an unstaged or canceled
    /// root. Old-world, foreign and evicted bounded token identities refuse. No cell payload is copied.
    fn live_world_transfer_receipt(
        &self,
        transfer: LiveWorldTransferRef,
    ) -> Result<LiveWorldTransferReceipt, LiveWorldError>;
}

/// Write boundary reached by level and projection handles.
// hidden: generated handle backend; mods use the natural receiver methods.
#[doc(hidden)]
#[sunrise_mod_api_macros::host_area]
pub trait LiveWorldsBackendMut {
    /// Side: server.
    /// Bind this mod's open Void runtime world to `destination`, with an explicit sparse
    /// list of local cells. The blocks, inventories, tickers, attachments and schedules
    /// remain their one real level instances. No source or destination block is rewritten.
    /// Requires resident cells and a starting pose admitted by the explicit publication policy.
    /// CollisionChecked is the default; ExternallyResolved requires trusted server-owner configuration.
    /// One binding per
    /// local world, subject to server per-owner and aggregate reservations; never disguises a vanilla entity as the owner.
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
    /// CollisionChecked checks swept real shapes and commits only a completely clear step.
    /// Explicit trusted ExternallyResolved consumes the caller's resolved pose without a second
    /// dynamics solver or correction; identity/residency and bounded motion/work still apply.
    /// Linear motion of the binding's fixed local pivot and shortest angular motion are checked
    /// continuously together. Equivalent caller pivot representations are reanchored without
    /// moving any physical point; returned samples use the binding's pivot.
    /// No rigid-body integrator, block reconstruction or player teleport runs here.
    fn live_world_publish_pose(
        &mut self,
        live_world: LiveWorldRef,
        expected_revision: u64,
        pose: LiveWorldPose,
    ) -> Result<LiveWorldSample, LiveWorldError>;
    /// Side: server.
    /// Break the transformed hit as this real player, using ordinary protection, game mode,
    /// tools, hooks, drops, experience and inventory routes. The host re-clips from the actual
    /// player eye, checks reach and static occlusion, then lends the real source context.
    /// This explicit mod action has the same immediate destroy semantics as PlayerRef::destroy_block.
    fn live_world_destroy_block(
        &mut self,
        live_world: LiveWorldRef,
        player: PlayerRef,
        hit: LiveWorldHit,
        into_inventory: bool,
    ) -> Result<bool, LiveWorldError>;
    /// Side: server.
    /// Use the transformed source hit through the ordinary block/item/menu pipeline. A source
    /// chest or furnace and block items are supported; other item-only effects refuse before
    /// mutation. The physical player body is never moved. Placement admission is exact,
    /// transactional and revalidated, and successful writes update sparse membership.
    fn live_world_use_block(
        &mut self,
        live_world: LiveWorldRef,
        player: PlayerRef,
        hit: LiveWorldHit,
        hand: Hand,
    ) -> Result<bool, LiveWorldError>;
    /// Side: server.
    /// Save one checked common decision over actual level data and the entire pending persistent
    /// root write group. Only listed receipt members are durable. Refuses tentative or mid-level
    /// contexts; ambiguous/post-Prepared storage failure fences the session until recovery.
    fn level_checkpoint(&mut self, level: LevelRef)
    -> Result<LiveWorldSaveReceipt, LiveWorldError>;
    /// Side: server.
    /// Restore this caller's saved binding after opening its real source and destination
    /// levels and making the member chunks resident. Parsing is bounded and all normal
    /// project/residency/collision checks run again. Old handles stay stale.
    fn level_restore_projection(&mut self, level: LevelRef)
    -> Result<LiveWorldRef, LiveWorldError>;
    /// Side: server.
    /// Explicit local-development migration of this service's earlier experimental disk-v1 data.
    /// Preserves the exact original descriptor and all live contents in the common durable decision;
    /// no automatic reseed or old mod API compatibility is provided.
    fn level_migrate_projection(
        &mut self,
        level: LevelRef,
    ) -> Result<LiveWorldSavedProjection, LiveWorldError>;
    /// Side: server.
    /// Prepare a bounded multi-level move over genuine resident blocks, live block entities and
    /// settled schedules. All destinations must be empty; an occupied/unloaded later mapping
    /// refuses the entire selection before any source destruction. Only this mod's runtime worlds
    /// and ordinary server levels are admitted. No ticker runs at the completed global frontier.
    /// This initial stock-payload slice refuses custom chunk load/save or block-entity additional
    /// serializers before selection, preserving strict live/durable inventory agreement.
    fn level_prepare_transfer(
        &mut self,
        level: LevelRef,
        cells: &[LiveWorldTransferCell],
        limits: LiveWorldTransferLimits,
    ) -> Result<LiveWorldTransferRef, LiveWorldError>;
    /// Side: server.
    /// Stage one prepared batch in the exact current root transaction. Writes remain deferred until
    /// root close. Pose/binding and persistent owned Level attachments may join that decision,
    /// including a third unbound level; other transaction effects refuse before Prepared.
    /// The planned epoch is an intention, not a completion receipt.
    fn live_world_transfer_stage(
        &mut self,
        transfer: LiveWorldTransferRef,
        tx: &TransactionContext,
    ) -> Result<LiveWorldTransferPending, LiveWorldError>;
    /// Side: server.
    /// Own a root transaction for this batch; an existing transaction refuses Busy. The returned
    /// receipt follows the common journal acknowledgment, actual live owner handoff and revalidation.
    fn live_world_transfer_commit(
        &mut self,
        transfer: LiveWorldTransferRef,
    ) -> Result<LiveWorldTransferReceipt, LiveWorldError>;
    /// Side: server.
    /// Dispose this caller's unstaged token or completed receipt. A staged root still owns its token.
    fn live_world_transfer_cancel(
        &mut self,
        transfer: LiveWorldTransferRef,
    ) -> Result<(), LiveWorldError>;
    /// Side: server.
    /// Remove only this projection, preserving the local world's blocks and saved data.
    /// Committed retirement cannot revive an old handle. A canceled Host root restores its
    /// exact binding beforeimage. This is not disassembly or deletion.
    fn live_world_retire(&mut self, live_world: LiveWorldRef) -> Result<(), LiveWorldError>;
}
