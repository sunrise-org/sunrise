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

/// Receipt returned only after the real level save barrier and durable projection metadata.
#[derive(Clone, Debug, PartialEq, Eq, sunrise_mod_api_macros::Wire)]
#[non_exhaustive]
pub struct LiveWorldCheckpoint {
    /// Persistent source runtime-level identity, independent of a world-open handle.
    pub source: String,
    /// The binding generation saved by this barrier.
    pub generation: u64,
    /// The accepted pose revision saved by this barrier.
    pub pose_revision: u64,
    /// The source's block revision at the barrier.
    pub source_revision: u64,
}

impl LiveWorldCheckpoint {
    /// Engine constructor for a completed barrier.
    #[doc(hidden)]
    pub fn new(source: String, generation: u64, pose_revision: u64, source_revision: u64) -> Self {
        Self {
            source,
            generation,
            pose_revision,
            source_revision,
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
}

impl crate::NoBackend for LiveWorldError {
    fn no_backend() -> Self {
        Self::Unavailable
    }
}
