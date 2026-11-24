//! Experimental projections of real runtime levels into another level.
//! The runtime level owns all blocks, block entities, inventories and scheduled work.

use crate::LiveWorldRef;

/// A rigid sample. Rotation is a unit quaternion `[x, y, z, w]`.
#[derive(Clone, Copy, Debug, PartialEq, sunrise_mod_api_macros::Wire)]
// exhaustive: callers provide all pose fields
pub struct LiveWorldPose {
    /// World translation in blocks.
    pub translation: [f64; 3],
    /// Local pivot in blocks.
    pub pivot: [f64; 3],
    /// Unit rotation; the engine rejects non-finite or non-unit input.
    pub rotation: [f64; 4],
}

impl LiveWorldPose {
    /// Identity rotation with a local origin translated to `translation`.
    pub fn at(translation: [f64; 3]) -> Self {
        Self {
            translation,
            pivot: [0.0; 3],
            rotation: [0.0, 0.0, 0.0, 1.0],
        }
    }
}

/// Checked limits of one projection. Limits may be reduced from the engine ceiling.
#[derive(Clone, Copy, Debug, PartialEq, sunrise_mod_api_macros::Wire)]
// exhaustive: every budget is supplied together
pub struct LiveWorldLimits {
    /// Inclusive local block coordinate minimum on each axis.
    pub min: [i32; 3],
    /// Inclusive local block coordinate maximum on each axis.
    pub max: [i32; 3],
    /// Reserved member-cell budget; default 4096, checked against server quotas.
    pub cells: u32,
    /// Reserved source terrain-retention columns and destination sweep chunks; default 128.
    pub chunks: u32,
    /// Reserved shape-box budget; default 32768.
    pub boxes: u32,
    /// Maximum contact pair tests across one published step; default 65536.
    pub contacts: u32,
    /// Maximum candidate world block positions in one published sweep; default 65536.
    pub candidates: u32,
    /// Maximum absolute translation per axis in one published pose; default one block.
    pub translation: f64,
    /// Maximum angular change in one published pose; default five degrees.
    pub rotation_degrees: f64,
    /// Admission policy; externally resolved poses require explicit server owner authorization.
    pub publication: LiveWorldPublication,
    /// Maximum conservative contact-kernel subintervals per pose; default 32.
    pub motion_steps: u32,
}

impl Default for LiveWorldLimits {
    fn default() -> Self {
        Self {
            min: [-128; 3],
            max: [127; 3],
            cells: 4096,
            chunks: 128,
            boxes: 32768,
            contacts: 65536,
            candidates: 65536,
            translation: 1.0,
            rotation_degrees: 5.0,
            motion_steps: 32,
            publication: LiveWorldPublication::CollisionChecked,
        }
    }
}

/// Derived requirements of a new sparse source selection; this read reserves nothing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, sunrise_mod_api_macros::Wire)]
#[non_exhaustive]
pub struct LiveWorldFootprint {
    /// Number of distinct selected cells.
    pub cells: u32,
    /// Number of distinct chunks containing selected cells.
    pub source_chunks: u32,
    /// Actual source terrain-cover columns held by the ordinary forced-streaming rings.
    pub retained_columns: u32,
    /// Initial sparse view areas, before other owners' aggregate guest admission.
    pub view_areas: u32,
}
impl LiveWorldFootprint {
    /// Engine constructor for an admitted bounded footprint calculation.
    // hidden: engine/Wire constructs admitted query values; host operations revalidate caller input.
    #[doc(hidden)]
    pub fn new(cells: u32, source_chunks: u32, retained_columns: u32, view_areas: u32) -> Self {
        Self {
            cells,
            source_chunks,
            retained_columns,
            view_areas,
        }
    }
}

/// Coherent previous/current accepted samples and their revision.
#[derive(Clone, Copy, Debug, PartialEq, sunrise_mod_api_macros::Wire)]
#[non_exhaustive]
pub struct LiveWorldSample {
    /// Previous accepted rigid pose.
    pub previous: LiveWorldPose,
    /// Current accepted rigid pose.
    pub current: LiveWorldPose,
    /// Changes on accepted motion; stale actions must re-resolve.
    pub revision: u64,
}

impl LiveWorldSample {
    /// Engine constructor for one coherent pair.
    // hidden: engine/Wire constructs admitted query values; host operations revalidate caller input.
    #[doc(hidden)]
    pub fn new(previous: LiveWorldPose, current: LiveWorldPose, revision: u64) -> Self {
        Self {
            previous,
            current,
            revision,
        }
    }
}

/// A server ray hit. Every field is revalidated before an action can consume it.
#[derive(Clone, Copy, Debug, PartialEq, sunrise_mod_api_macros::Wire)]
#[non_exhaustive]
pub struct LiveWorldHit {
    /// The owning projection and generation.
    pub world: LiveWorldRef,
    /// The real local cell.
    pub cell: [i32; 3],
    /// Pose revision used for the hit.
    pub pose_revision: u64,
    /// Source world revision used for the hit; changes invalidate the hit.
    pub cell_revision: u64,
    /// Authoritative world-space hit point.
    pub point: [f64; 3],
    /// Exact source-cell face from the ordinary voxel-shape clip, before rotation.
    pub face: crate::Direction,
    /// Exact hit point in source-level coordinates, before rotation and translation.
    pub local_point: [f64; 3],
}

impl LiveWorldHit {
    /// Engine constructor; public data is never trusted without backend validation.
    // hidden: engine/Wire constructs admitted query values; host operations revalidate caller input.
    #[doc(hidden)]
    pub fn new(
        world: LiveWorldRef,
        cell: [i32; 3],
        pose_revision: u64,
        cell_revision: u64,
        point: [f64; 3],
        face: crate::Direction,
        local_point: [f64; 3],
    ) -> Self {
        Self {
            world,
            cell,
            pose_revision,
            cell_revision,
            point,
            face,
            local_point,
        }
    }
}

/// A real member cell, read from the source level rather than a separate block store.
#[derive(Clone, Debug, PartialEq, sunrise_mod_api_macros::Wire)]
#[non_exhaustive]
pub struct LiveWorldCell {
    /// Source-level block coordinates.
    pub pos: [i32; 3],
    /// The actual current block state.
    pub state: crate::BlockState,
    /// Real block-local collision boxes; gaps remain empty.
    pub collision: Vec<crate::ColliderBox>,
    /// The existing live block entity, when this cell has one.
    pub entity: Option<crate::BlockEntityRef>,
    /// Conservative source block revision, shared by member cells.
    pub revision: u64,
    /// Conservative real source block-entity content/attachment mutation revision.
    /// Inventory/ticker writes advance this without rebuilding static collision geometry.
    pub content_revision: u64,
}

impl LiveWorldCell {
    /// Engine constructor over a real source cell.
    // hidden: engine/Wire constructs admitted query values; host operations revalidate caller input.
    #[doc(hidden)]
    pub fn new(
        pos: [i32; 3],
        state: crate::BlockState,
        collision: Vec<crate::ColliderBox>,
        entity: Option<crate::BlockEntityRef>,
        revision: u64,
        content_revision: u64,
    ) -> Self {
        Self {
            pos,
            state,
            collision,
            entity,
            revision,
            content_revision,
        }
    }
}

/// One exact prospective ordinary placement write, before mutation or item consumption.
#[derive(Clone, Debug, PartialEq, sunrise_mod_api_macros::Wire)]
#[non_exhaustive]
pub struct LiveWorldPlacementCell {
    /// Source-level coordinate in the prospective ownership set.
    pub pos: [i32; 3],
    /// Canonical state selected by the ordinary placement decision.
    pub state: crate::BlockState,
    /// Checked local collision boxes for this state.
    pub collision: Vec<crate::ColliderBox>,
}
impl LiveWorldPlacementCell {
    /// Engine constructor; hook inputs are a fixed decision, never another state selection.
    // hidden: engine/Wire constructs admitted query values; host operations revalidate caller input.
    #[doc(hidden)]
    pub fn new(
        pos: [i32; 3],
        state: crate::BlockState,
        collision: Vec<crate::ColliderBox>,
    ) -> Self {
        Self {
            pos,
            state,
            collision,
        }
    }
}

/// A projection refusal; failed calls preserve the last accepted state.
#[derive(Clone, Copy, Debug, PartialEq, Eq, sunrise_mod_api_macros::Wire)]
#[non_exhaustive]
pub enum LiveWorldError {
    /// Wrong side, no world or no enabled mod context.
    Unavailable,
    /// The ordinary host protection or permission check denied this action.
    Denied,
    /// The transformed hit is outside the real player's current interaction range.
    OutOfReach,
    /// A scoped action already holds this projection context.
    Busy,
    /// Old world/binding generation, retired binding or another owner.
    Stale,
    /// The local level is not an open Void runtime level owned by this mod.
    NotLocalWorld,
    /// The level already belongs to a projection.
    AlreadyBound,
    /// Invalid coordinate, quaternion, duplicate membership or configured budget.
    Invalid,
    /// A configured engine/work budget would be exceeded.
    Budget,
    /// Source/destination cells are not fully resident or ticking.
    NotResident,
    /// The requested rigid sweep meets an obstacle.
    Obstructed,
    /// The caller's expected pose/source revision no longer holds.
    Revision,
    /// A required route is not enabled by this experimental slice.
    Unsupported,
    /// Durable save or bounded metadata recovery failed.
    Storage,
    /// No projection checkpoint exists. Corrupt or unreadable data returns Storage.
    NotSaved,
    /// No durable commit receipt exists because this preparation was canceled or aborted.
    NotCommitted,
    /// Earlier experimental disk metadata requires explicit validated recovery migration.
    MigrationRequired,
    /// The supplied transaction loan does not identify this host's current frame.
    InvalidContext,
}

impl crate::NoBackend for LiveWorldError {
    fn no_backend() -> Self {
        Self::Unavailable
    }
}

/// How an accepted externally supplied pose is admitted; neither policy integrates dynamics.
#[derive(Clone, Copy, Debug, PartialEq, Eq, sunrise_mod_api_macros::Wire)]
#[non_exhaustive]
pub enum LiveWorldPublication {
    /// The host conservatively checks the complete bounded rigid sweep against terrain.
    CollisionChecked,
    /// An explicitly authorized server owner supplies an already resolved rigid pose.
    /// Identity, residency, finite values, work/step budgets and player contacts still apply.
    ExternallyResolved,
}

/// Explicit recovery state for earlier local experimental disk data.
#[derive(Clone, Copy, Debug, PartialEq, Eq, sunrise_mod_api_macros::Wire)]
#[non_exhaustive]
pub enum LiveWorldRecovery {
    /// The exact current-format saved descriptor is ready for ordinary residency preflight.
    Ready,
    /// Earlier experimental metadata must be migrated explicitly with originals retained.
    ExperimentalMigrationRequired,
}

/// Saved membership/residency metadata only; this never exposes a second inventory authority.
#[derive(Clone, Debug, PartialEq, sunrise_mod_api_macros::Wire)]
#[non_exhaustive]
pub struct LiveWorldSavedProjection {
    /// Persistent ordinary destination level id.
    pub destination: String,
    /// Distinct actual saved member columns to ticket before attempting restore.
    pub source_chunks: Vec<[i32; 2]>,
    /// Saved membership size.
    pub member_cells: u32,
    /// Checked ordinary terrain retention cover cost.
    pub retained_columns: u32,
    /// Saved operational budgets and publication policy.
    pub limits: LiveWorldLimits,
    /// Common durable epoch that includes this descriptor.
    pub barrier_epoch: u64,
    /// Whether explicit recovery migration is required before restore.
    pub recovery: LiveWorldRecovery,
}
impl LiveWorldSavedProjection {
    /// Engine constructor over fully checked metadata.
    // hidden: engine/Wire constructs admitted query values; host operations revalidate caller input.
    #[doc(hidden)]
    pub fn new(
        destination: String,
        source_chunks: Vec<[i32; 2]>,
        member_cells: u32,
        retained_columns: u32,
        limits: LiveWorldLimits,
        barrier_epoch: u64,
        recovery: LiveWorldRecovery,
    ) -> Self {
        Self {
            destination,
            source_chunks,
            member_cells,
            retained_columns,
            limits,
            barrier_epoch,
            recovery,
        }
    }
}

/// One actual covered level at a completed common save decision.
#[derive(Clone, Debug, PartialEq, Eq, sunrise_mod_api_macros::Wire)]
#[non_exhaustive]
pub struct LiveWorldLevelReceipt {
    /// Exact current world/open generation.
    pub level: crate::LevelRef,
    /// Saved actual World's block epoch.
    pub block_revision: u64,
    /// Saved actual block-entity content epoch.
    pub content_revision: u64,
    /// Current binding, if any.
    pub binding: Option<LiveWorldRef>,
    /// Saved retired binding identity; never a fabricated moved-cell receipt.
    pub retired_generation: Option<u64>,
    /// Actual remaining member count at retirement, valid only through its checked source readset.
    pub retired_members: Option<u32>,
}
impl LiveWorldLevelReceipt {
    /// Engine constructor after common acknowledgment.
    // hidden: engine/Wire constructs admitted query values; host operations revalidate caller input.
    #[doc(hidden)]
    pub fn new(
        level: crate::LevelRef,
        block_revision: u64,
        content_revision: u64,
        binding: Option<LiveWorldRef>,
        retired_generation: Option<u64>,
        retired_members: Option<u32>,
    ) -> Self {
        Self {
            level,
            block_revision,
            content_revision,
            binding,
            retired_generation,
            retired_members,
        }
    }
}
/// Durable common acknowledgment. Only listed levels are covered.
#[derive(Clone, Debug, PartialEq, Eq, sunrise_mod_api_macros::Wire)]
#[non_exhaustive]
pub struct LiveWorldSaveReceipt {
    /// World-root checked monotone durable epoch.
    pub barrier_epoch: u64,
    /// Actual payload, pose, binding and persistent attachment coverage union.
    pub levels: Vec<LiveWorldLevelReceipt>,
}
impl LiveWorldSaveReceipt {
    /// Engine constructor after common acknowledgment.
    // hidden: engine/Wire constructs admitted query values; host operations revalidate caller input.
    #[doc(hidden)]
    pub fn new(barrier_epoch: u64, levels: Vec<LiveWorldLevelReceipt>) -> Self {
        Self {
            barrier_epoch,
            levels,
        }
    }
}
/// One exact source/destination mapping; no payload is stored in caller-owned data.
#[derive(Clone, Copy, Debug, PartialEq, Eq, sunrise_mod_api_macros::Wire)]
#[non_exhaustive]
pub struct LiveWorldTransferCell {
    /// Actual source level.
    pub source: crate::LevelRef,
    /// Actual source cell.
    pub from: [i32; 3],
    /// Actual destination level.
    pub destination: crate::LevelRef,
    /// Actual empty destination cell.
    pub to: [i32; 3],
}
impl LiveWorldTransferCell {
    /// Describe one untrusted mapping for the host's complete preflight.
    pub fn new(
        source: crate::LevelRef,
        from: [i32; 3],
        destination: crate::LevelRef,
        to: [i32; 3],
    ) -> Self {
        Self {
            source,
            from,
            destination,
            to,
        }
    }
}
/// Caller-selected work ceilings, independently checked against host and common-WAL quotas.
#[derive(Clone, Copy, Debug, PartialEq, Eq, sunrise_mod_api_macros::Wire)]
// exhaustive: callers supply the whole quota set
pub struct LiveWorldTransferLimits {
    /// Mapped cells.
    pub cells: u32,
    /// Distinct actual levels including persistent attachment coverage.
    pub levels: u32,
    /// Distinct affected columns.
    pub chunks: u32,
    /// Source collision boxes.
    pub boxes: u32,
    /// Actual preserved schedule occurrences.
    pub scheduled_ticks: u32,
    /// Complete serialized common decision byte ceiling.
    pub bytes: u64,
}
impl Default for LiveWorldTransferLimits {
    fn default() -> Self {
        Self {
            cells: 4096,
            levels: 16,
            chunks: 128,
            boxes: 32768,
            scheduled_ticks: 65536,
            bytes: 16 << 20,
        }
    }
}
/// A staged root-close intention; this does not prove a durable commit.
#[derive(Clone, Copy, Debug, PartialEq, Eq, sunrise_mod_api_macros::Wire)]
#[non_exhaustive]
pub struct LiveWorldTransferPending {
    /// Planned common epoch; only receipt() can confirm completion.
    pub planned_epoch: u64,
}
impl LiveWorldTransferPending {
    /// Engine constructor for a staged intention.
    // hidden: engine/Wire constructs admitted query values; host operations revalidate caller input.
    #[doc(hidden)]
    pub fn new(planned_epoch: u64) -> Self {
        Self { planned_epoch }
    }
}
/// A completed transfer at one common durable decision.
#[derive(Clone, Debug, PartialEq, Eq, sunrise_mod_api_macros::Wire)]
#[non_exhaustive]
pub struct LiveWorldTransferReceipt {
    /// Acknowledged common epoch.
    pub barrier_epoch: u64,
    /// Actual mapped ownership handoffs.
    pub moved_cells: u32,
    /// Actual covered levels, including third-level persistent metadata writes.
    pub levels: Vec<LiveWorldLevelReceipt>,
    /// Binding identities retired by this decision.
    pub retired_bindings: Vec<LiveWorldRef>,
}
impl LiveWorldTransferReceipt {
    /// Engine constructor after common acknowledgment.
    // hidden: engine/Wire constructs admitted query values; host operations revalidate caller input.
    #[doc(hidden)]
    pub fn new(
        barrier_epoch: u64,
        moved_cells: u32,
        levels: Vec<LiveWorldLevelReceipt>,
        retired_bindings: Vec<LiveWorldRef>,
    ) -> Self {
        Self {
            barrier_epoch,
            moved_cells,
            levels,
            retired_bindings,
        }
    }
}
