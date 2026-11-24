//! Local experimental runtime-world projections (Refs #1022).
//! Blocks, inventories, schedules and attachments stay in the real source level.
//! This table contains only membership, pose samples and derived collision geometry.
use super::{Game, api_values::TryIntoApi as _};
#[path = "native_live_worlds/durability.rs"]
mod durability;
pub(in crate::game) use durability::Durability;
#[path = "native_live_worlds/binding_journal.rs"]
mod binding_journal;
pub(in crate::game) use binding_journal::KeptProjectionBinding;
#[path = "native_live_worlds/actions.rs"]
mod actions;
#[path = "native_live_worlds/checkpoint.rs"]
mod checkpoint;
#[path = "native_live_worlds/inspection.rs"]
mod inspection;
#[path = "native_live_worlds/menus.rs"]
mod menus;
#[path = "native_live_worlds/obstruction.rs"]
pub(in crate::game) mod obstruction;
#[path = "native_live_worlds/persistence_codec.rs"]
mod persistence_codec;
#[path = "native_live_worlds/picking.rs"]
mod picking;
#[path = "native_live_worlds/placement.rs"]
mod placement;
#[cfg(test)]
#[path = "native_live_worlds/publication_tests.rs"]
mod publication_tests;
#[path = "native_live_worlds/retention.rs"]
mod retention;
#[path = "native_live_worlds/sync.rs"]
mod sync;
#[path = "native_live_worlds/trace.rs"]
pub(in crate::game) mod trace;
#[path = "native_live_worlds/transfer.rs"]
mod transfer;
#[path = "native_live_worlds/transfer_cells.rs"]
mod transfer_cells;
use mod_api::{
    ColliderBox, LevelRef, LiveWorldCell, LiveWorldError as Error, LiveWorldFootprint,
    LiveWorldHit, LiveWorldLimits, LiveWorldPose, LiveWorldRef, LiveWorldSample,
};
use sha2::{Digest as _, Sha256};
use std::{
    cell::RefCell,
    collections::{BTreeMap, BTreeSet},
    rc::Rc,
    sync::Arc,
};
use sunrise_client::{mesher::Vertex, world::World};
use sunrise_client_render::moving_colliders::{Placed, Prepared};
use sunrise_core::{
    geometry::{
        Axis, Vec3, VoxelShape,
        oriented::{ColliderBoxes, Frame, OrientedCollider},
    },
    position::BlockPos,
};
use sunrise_world::carrier::contact::{
    CarrierCell, CarrierGeometry, ContactBudget, Obstacle, RigidTransform, Rotation,
};

/// No binding exists until a mod explicitly asks for one.
#[derive(Default)]
pub(in crate::game) struct LiveWorlds {
    next: u64,
    sequence: u64,
    entries: BTreeMap<u64, Entry>,
    quotas: Quotas,
    external_pose_owners: BTreeSet<String>,
    sync: sync::Sync,
    action: Option<actions::ProjectedAction>,
    building: Option<Rc<obstruction::Building>>,
    menus: BTreeMap<u32, menus::ProjectedMenu>,
    pending_opens: BTreeMap<u32, menus::PendingOpen>,
    menu_scope: Option<menus::MenuScope>,
    pick: Option<picking::Pick>,
}

struct Quotas {
    owner_cells: u64,
    total_cells: u64,
    owner_boxes: u64,
    total_boxes: u64,
    owner_chunks: u64,
    total_chunks: u64,
}
impl Default for Quotas {
    fn default() -> Self {
        Self {
            owner_cells: 131072,
            total_cells: 262144,
            owner_boxes: 524288,
            total_boxes: 1048576,
            owner_chunks: 4096,
            total_chunks: 8192,
        }
    }
}

impl LiveWorlds {
    #[cfg(test)]
    pub(in crate::game) fn binding_count(&self) -> usize {
        self.entries.len()
    }
    pub(in crate::game) fn clear(&mut self) {
        self.entries.clear();
        self.sync.clear();
        self.action = None;
        self.building = None;
        self.menus.clear();
        self.pending_opens.clear();
        self.menu_scope = None;
        self.pick = None;
    }
}

#[derive(Clone)]
struct Entry {
    handle: LiveWorldRef,
    owner: &'static str,
    destination: LevelRef,
    cells: Vec<[i32; 3]>,
    membership: u64,
    retained: Vec<(i32, i32)>,
    stream_areas: Vec<sunrise_client::lan::viewed_levels::areas::ChunkArea>,
    limits: LiveWorldLimits,
    sample: LiveWorldSample,
    /// An engine-allocated collision identity, never a disguised vanilla entity.
    collider: u32,
    accepted_tick: Option<u64>,
    tick_start: LiveWorldPose,
    carrying: bool,
    /// A client keeps its last coherent caches while real view updates are incomplete.
    frozen: bool,
    geometry: RefCell<Option<Rc<Geometry>>>,
    mesh: RefCell<Option<Rc<Mesh>>>,
}

/// A motion before-image in the ordinary Host transaction journal, never a live body copy.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(in crate::game) struct KeptProjectionPose {
    handle: LiveWorldRef,
    sample: LiveWorldSample,
    accepted_tick: Option<u64>,
    tick_start: LiveWorldPose,
    carrying: bool,
}

struct Geometry {
    source_revision: u64,
    membership: u64,
    resources: u64,
    state_hash: [u8; 32],

    contact: CarrierGeometry,
    boxes: Arc<ColliderBoxes>,
}

struct Mesh {
    source_revision: u64,
    membership: u64,
    resources: u64,
    cells: Vec<MeshCell>,
}
struct MeshCell {
    pos: [i32; 3],
    state: sunrise_world::registry::BlockStateId,
    faces: Vec<super::minecarts::PreparedModelFace>,
}

/// Only membership and canonical block states affect this witness; inventory ticks do not.
fn state_hash(world: &World, cells: &[[i32; 3]]) -> [u8; 32] {
    let mut hash = Sha256::new();
    for &cell in cells {
        for axis in cell {
            hash.update(axis.to_le_bytes());
        }
        hash.update(world.block(cell[0], cell[1], cell[2]).0.to_le_bytes());
    }
    hash.finalize().into()
}

fn vector(v: [f64; 3]) -> Vec3 {
    Vec3::new(v[0], v[1], v[2])
}
fn position(p: [i32; 3]) -> BlockPos {
    BlockPos::new(p[0], p[1], p[2])
}

fn checked_pose(pose: LiveWorldPose) -> Result<RigidTransform, Error> {
    if pose
        .translation
        .iter()
        .any(|v| !v.is_finite() || v.abs() >= 30_000_000.0)
        || pose
            .pivot
            .iter()
            .any(|v| !v.is_finite() || v.abs() >= 30_000_000.0)
        || !sunrise_core::geometry::oriented::check_rotation(pose.rotation)
    {
        return Err(Error::Invalid);
    }
    let [x, y, z, w] = pose.rotation;
    RigidTransform::new(
        vector(pose.translation),
        vector(pose.pivot),
        Rotation::from_components(w, x, y, z).map_err(|_| Error::Invalid)?,
    )
    .map_err(|_| Error::Invalid)
}

fn frame(pose: LiveWorldPose) -> Frame {
    Frame::from_quaternion_about(vector(pose.translation), vector(pose.pivot), pose.rotation)
}

fn checked_cells(cells: &[[i32; 3]], limits: LiveWorldLimits) -> Result<Vec<[i32; 3]>, Error> {
    if limits.cells == 0
        || limits.cells > 65536
        || limits.chunks == 0
        || limits.chunks > 4096
        || limits.boxes == 0
        || limits.boxes > 262144
        || limits.contacts == 0
        || limits.contacts > 4194304
        || limits.candidates == 0
        || limits.candidates > 1048576
        || !limits.translation.is_finite()
        || !(0.0..=64.0).contains(&limits.translation)
        || !limits.rotation_degrees.is_finite()
        || !(0.0..=90.0).contains(&limits.rotation_degrees)
        || limits.motion_steps == 0
        || limits.motion_steps > 128
        || (0..3).any(|i| {
            limits.min[i] > limits.max[i]
                || limits.min[i] <= -30_000_000
                || limits.max[i] >= 30_000_000
        })
    {
        return Err(Error::Invalid);
    }
    if cells.len() > limits.cells as usize {
        return Err(Error::Budget);
    }
    let mut ordered = cells.to_vec();
    ordered.sort_unstable();
    if ordered.windows(2).any(|pair| pair[0] == pair[1])
        || ordered
            .iter()
            .any(|p| (0..3).any(|i| p[i] < limits.min[i] || p[i] > limits.max[i]))
    {
        return Err(Error::Invalid);
    }
    let chunks: BTreeSet<_> = ordered.iter().map(|p| (p[0] >> 4, p[2] >> 4)).collect();
    if chunks.len() > limits.chunks as usize {
        return Err(Error::Budget);
    }
    Ok(ordered)
}

/// Unlike convenience shape reads, a failed collision query never turns into empty space.
fn shape(game: &Game<'_>, world: &World, pos: [i32; 3]) -> Result<VoxelShape, Error> {
    if !world.is_loaded_at(pos[0], pos[1], pos[2]) {
        return Err(Error::NotResident);
    }
    let state = world.block(pos[0], pos[1], pos[2]);
    state_shape(game, state)
}

/// Ordinary parent terrain is queried in its actual World/position with no colliding entity.
/// Its contextual/published geometry is not restricted to the carried source palette. Errors
/// remain refusal, unlike convenience box helpers which default a failed query to empty.
fn destination_shape(game: &Game<'_>, world: &World, pos: [i32; 3]) -> Result<VoxelShape, Error> {
    if !world.shape.contains_y(pos[1]) || !world.is_loaded_at(pos[0], pos[1], pos[2]) {
        return Err(Error::NotResident);
    }
    let state = world.block(pos[0], pos[1], pos[2]);
    let mut scene = sunrise_client::collisions::SceneCollisions {
        world,
        binding: game.collision,
        subject: None,
    };
    game.collision
        .collision_shape_with_context(
            &mut scene,
            state,
            BlockPos::new(pos[0], pos[1], pos[2]),
            &sunrise_world::collision_context::CollisionContext::empty(false),
        )
        .map_err(|_| Error::Unsupported)
}

/// Context-free source shapes are admitted before an ordinary placement can consume a stack.
fn state_shape(
    game: &Game<'_>,
    state: sunrise_world::registry::BlockStateId,
) -> Result<VoxelShape, Error> {
    let name = &game.res.facts().get(state).name;
    if super::text_displays::has_special_model(name)
        && !matches!(
            &**name,
            "minecraft:chest" | "minecraft:trapped_chest" | "minecraft:ender_chest"
        )
    {
        return Err(Error::Unsupported);
    }
    if game.collision.has_context_rule(state) || game.collision.has_positioned_collision(state) {
        return Err(Error::Unsupported);
    }
    game.collision
        .static_shape(state)
        .map_err(|_| Error::Unsupported)?
        .cloned()
        .ok_or(Error::Unsupported)
}

impl Game<'_> {
    pub(super) fn level_projection_footprint(
        &self,
        level: LevelRef,
        cells: &[[i32; 3]],
    ) -> Result<LiveWorldFootprint, Error> {
        let owner = self.projection_owner()?;
        if !self.level_handle_current(level)
            || !self.levels.runtime.owned_void(level.dimension_id(), owner)
        {
            return Err(Error::NotLocalWorld);
        }
        self.native_level_world(level).ok_or(Error::Stale)?;
        retention::selection_footprint(level, cells)
    }

    pub(in crate::game) fn capture_projection_pose(
        &self,
        handle: LiveWorldRef,
    ) -> Option<KeptProjectionPose> {
        let table = self.live_worlds.borrow();
        let entry = table
            .entries
            .get(&handle.generation())
            .filter(|entry| entry.handle == handle)?;
        Some(KeptProjectionPose {
            handle,
            sample: entry.sample,
            accepted_tick: entry.accepted_tick,
            tick_start: entry.tick_start,
            carrying: entry.carrying,
        })
    }
    pub(in crate::game) fn restore_projection_pose(&mut self, kept: KeptProjectionPose) {
        if let Some(entry) = self
            .live_worlds
            .get_mut()
            .entries
            .get_mut(&kept.handle.generation())
            .filter(|entry| entry.handle == kept.handle)
        {
            entry.sample = kept.sample;
            entry.accepted_tick = kept.accepted_tick;
            entry.tick_start = kept.tick_start;
            entry.carrying = kept.carrying;
        }
    }
    /// Administrator-owned reservations; mods cannot raise the per-owner or aggregate budget.
    pub(in crate::game) fn read_live_world_quotas(
        &mut self,
        property: impl Fn(&str) -> Option<String>,
    ) {
        let authorized = property("sunrise-live-world-external-pose-owners").unwrap_or_default();
        let owners: Vec<_> = authorized
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .collect();
        let valid = owners.len() <= 64
            && owners.iter().all(|owner| {
                owner.len() <= 128
                    && owner.bytes().all(|c| {
                        c.is_ascii_lowercase()
                            || c.is_ascii_digit()
                            || matches!(c, b'_' | b'-' | b'.')
                    })
            });
        let table = self.live_worlds.get_mut();
        table.external_pose_owners.clear();
        if valid {
            table
                .external_pose_owners
                .extend(owners.into_iter().map(str::to_owned));
        }
        let quotas = &mut table.quotas;
        for (name, value, ceiling) in [
            (
                "sunrise-live-world-owner-cells",
                &mut quotas.owner_cells,
                1048576,
            ),
            (
                "sunrise-live-world-total-cells",
                &mut quotas.total_cells,
                4194304,
            ),
            (
                "sunrise-live-world-owner-boxes",
                &mut quotas.owner_boxes,
                4194304,
            ),
            (
                "sunrise-live-world-total-boxes",
                &mut quotas.total_boxes,
                16777216,
            ),
            (
                "sunrise-live-world-owner-chunks",
                &mut quotas.owner_chunks,
                32768,
            ),
            (
                "sunrise-live-world-total-chunks",
                &mut quotas.total_chunks,
                65536,
            ),
        ] {
            if let Some(parsed) = property(name)
                .and_then(|v| v.parse::<u64>().ok())
                .filter(|v| *v <= ceiling)
            {
                *value = parsed;
            }
        }
    }

    fn projection_owner(&self) -> Result<&'static str, Error> {
        self.runtime_caller().map_err(|_| Error::Unavailable)
    }

    fn projection_entry(&self, handle: LiveWorldRef) -> Result<std::cell::Ref<'_, Entry>, Error> {
        let owner = self.projection_owner()?;
        let entry = self.resolved_projection_entry(handle)?;
        if entry.owner != owner {
            return Err(Error::Stale);
        }
        Ok(entry)
    }

    /// Trusted engine identity lookup. The caller must already own either mod authority
    /// or an authenticated connection and a binding resolved from this server table.
    fn resolved_projection_entry(
        &self,
        handle: LiveWorldRef,
    ) -> Result<std::cell::Ref<'_, Entry>, Error> {
        if !self.live_world_lifecycle_current(handle.level()) {
            return Err(Error::Stale);
        }
        std::cell::Ref::filter_map(self.live_worlds.borrow(), |table| {
            table.entries.get(&handle.generation()).filter(|entry| {
                entry.handle == handle && self.live_world_lifecycle_current(entry.destination)
            })
        })
        .map_err(|_| Error::Stale)
    }

    fn projection_geometry(&self, entry: &Entry) -> Result<Rc<Geometry>, Error> {
        let world = self
            .native_level_view_world(entry.handle.level())
            .ok_or(Error::Stale)?;
        if !entry.frozen
            && entry
                .retained
                .iter()
                .any(|chunk| !world.chunks.contains_key(chunk))
        {
            return Err(Error::NotResident);
        }
        if let Some(geometry) = entry.geometry.borrow().as_ref()
            && (entry.frozen
                || (geometry.source_revision == world.revision
                    && geometry.membership == entry.membership
                    && geometry.resources == self.standalone_geometry.generation()))
        {
            return Ok(geometry.clone());
        }
        let mut cells = Vec::with_capacity(entry.cells.len());
        let mut boxes = Vec::new();
        for &pos in &entry.cells {
            let shape = shape(self, world, pos)?;
            if boxes
                .len()
                .checked_add(shape.boxes().len())
                .is_none_or(|n| n > entry.limits.boxes as usize)
            {
                return Err(Error::Budget);
            }
            let offset = vector(pos.map(f64::from));
            boxes.extend(shape.boxes().iter().map(|bounds| bounds.move_by(offset)));
            cells.push(CarrierCell::new(position(pos), shape));
        }
        let geometry = Rc::new(Geometry {
            source_revision: world.revision,
            membership: entry.membership,
            resources: self.standalone_geometry.generation(),
            state_hash: state_hash(world, &entry.cells),
            contact: CarrierGeometry::new(u128::from(entry.collider), cells)
                .map_err(|_| Error::Invalid)?,
            boxes: Arc::new(ColliderBoxes::new(boxes)),
        });
        *entry.geometry.borrow_mut() = Some(geometry.clone());
        Ok(geometry)
    }

    pub(super) fn level_live_world(&self, level: LevelRef) -> Result<Option<LiveWorldRef>, Error> {
        let owner = self.projection_owner()?;
        if !self.level_handle_current(level) {
            return Err(Error::Stale);
        }
        Ok(self
            .live_worlds
            .borrow()
            .entries
            .values()
            .find(|entry| entry.owner == owner && entry.handle.level() == level)
            .map(|entry| entry.handle))
    }

    pub(super) fn live_world_sample(&self, handle: LiveWorldRef) -> Result<LiveWorldSample, Error> {
        Ok(self.projection_entry(handle)?.sample)
    }

    pub(super) fn live_world_cells(
        &self,
        handle: LiveWorldRef,
    ) -> Result<Vec<LiveWorldCell>, Error> {
        let entry = self.projection_entry(handle)?;
        let world = self
            .native_level_world(handle.level())
            .ok_or(Error::Stale)?;
        let mut total = 0usize;
        entry
            .cells
            .iter()
            .map(|&pos| {
                let shape = shape(self, world, pos)?;
                total = total
                    .checked_add(shape.boxes().len())
                    .ok_or(Error::Budget)?;
                if total > entry.limits.boxes as usize {
                    return Err(Error::Budget);
                }
                let collision = shape
                    .boxes()
                    .iter()
                    .map(|b| ColliderBox {
                        min: b.min_position().components(),
                        max: b.max_position().components(),
                    })
                    .collect();
                Ok(LiveWorldCell::new(
                    pos,
                    world
                        .block(pos[0], pos[1], pos[2])
                        .try_into_api()
                        .map_err(|_| Error::Unsupported)?,
                    collision,
                    self.block_entity(handle.level(), pos),
                    world.revision,
                    world.block_entities.content_revision(),
                ))
            })
            .collect()
    }

    pub(super) fn live_world_clip(
        &self,
        handle: LiveWorldRef,
        from: [f64; 3],
        to: [f64; 3],
    ) -> Result<Option<LiveWorldHit>, Error> {
        let from = vector(from);
        let to = vector(to);
        if !from.is_finite() || !to.is_finite() || from.distance_squared(to) > 1024.0 * 1024.0 {
            return Err(Error::Invalid);
        }
        let entry = self.projection_entry(handle)?;
        let geometry = self.projection_geometry(&entry)?;
        Ok(geometry
            .contact
            .clip(&checked_pose(entry.sample.current)?, from, to)
            .map(|hit| {
                LiveWorldHit::new(
                    handle,
                    [hit.local_cell.x, hit.local_cell.y, hit.local_cell.z],
                    entry.sample.revision,
                    geometry.source_revision,
                    hit.location.components(),
                    hit.local_direction,
                    hit.local_location.components(),
                )
            }))
    }

    pub(super) fn live_world_validate_hit(
        &self,
        handle: LiveWorldRef,
        hit: LiveWorldHit,
    ) -> Result<(), Error> {
        self.projection_entry(handle)?;
        self.live_world_validate_hit_resolved(handle, hit)
    }

    fn live_world_validate_hit_resolved(
        &self,
        handle: LiveWorldRef,
        hit: LiveWorldHit,
    ) -> Result<(), Error> {
        let entry = self.resolved_projection_entry(handle)?;
        let world = self
            .native_level_world(handle.level())
            .ok_or(Error::Stale)?;
        if hit.world != handle
            || hit.pose_revision != entry.sample.revision
            || hit.cell_revision != world.revision
            || entry.cells.binary_search(&hit.cell).is_err()
        {
            return Err(Error::Revision);
        }
        if !hit
            .point
            .iter()
            .chain(&hit.local_point)
            .all(|v| v.is_finite())
        {
            return Err(Error::Invalid);
        }
        let local = checked_pose(entry.sample.current)?
            .to_local(vector(hit.point))
            .subtract(vector(hit.cell.map(f64::from)));
        if vector(hit.local_point)
            .distance_squared(local.add_vector(vector(hit.cell.map(f64::from))))
            > 1.0e-12
        {
            return Err(Error::Revision);
        }
        let shape = shape(self, world, hit.cell)?;
        let normal = vector(hit.face.normal().map(f64::from));
        let point = vector(hit.local_point);
        // VoxelShape::clip refuses rays whose squared length is below COLLISION_EPSILON.
        // Use a real surface probe above that existing threshold, without changing it.
        let probe = sunrise_core::geometry::COLLISION_EPSILON.sqrt();
        let clipped = shape.clip(
            point.add_vector(normal.scale(probe)),
            point.subtract(normal.scale(probe)),
            position(hit.cell),
        );
        if clipped.is_none_or(|actual| {
            actual.direction != hit.face || actual.location.distance_squared(point) > 1.0e-12
        }) {
            return Err(Error::Revision);
        }
        // A copied token cannot redirect an action to a different cell or empty gap.
        if !shape.boxes().iter().any(|bounds| {
            Axis::ALL.iter().all(|&axis| {
                local.component(axis) >= bounds.min(axis) - 1.0e-7
                    && local.component(axis) <= bounds.max(axis) + 1.0e-7
            })
        }) {
            return Err(Error::Revision);
        }
        Ok(())
    }

    pub(super) fn level_project(
        &mut self,
        level: LevelRef,
        destination: LevelRef,
        cells: &[[i32; 3]],
        pose: LiveWorldPose,
        limits: LiveWorldLimits,
    ) -> Result<LiveWorldRef, Error> {
        self.project_cells_with_cover(level, destination, cells, pose, limits, None)
    }
    fn project_cells_with_cover(
        &mut self,
        level: LevelRef,
        destination: LevelRef,
        cells: &[[i32; 3]],
        pose: LiveWorldPose,
        limits: LiveWorldLimits,
        cover: Option<Vec<sunrise_client::lan::viewed_levels::areas::ChunkArea>>,
    ) -> Result<LiveWorldRef, Error> {
        let owner = self.projection_owner()?;
        if destination.generation() != 0 {
            return Err(Error::Unsupported);
        }
        self.check_live_world_publication(owner, limits.publication)?;
        if level == destination
            || !self.level_handle_current(level)
            || !self.levels.runtime.owned_void(level.dimension_id(), owner)
        {
            return Err(Error::NotLocalWorld);
        }
        self.native_level_world(destination).ok_or(Error::Stale)?;
        let cells = checked_cells(cells, limits)?;
        let source_world = self.native_level_world(level).ok_or(Error::NotResident)?;
        if cells.is_empty()
            || cells
                .iter()
                .any(|p| source_world.block(p[0], p[1], p[2]) == source_world.air)
        {
            return Err(Error::Invalid);
        }
        let stream_areas = match cover {
            Some(areas) => areas,
            None => sync::stream_areas(level, &cells)?,
        };
        self.check_projection_views(destination, &stream_areas)?;
        checked_pose(pose)?;
        {
            let table = self.live_worlds.borrow();
            if table
                .entries
                .values()
                .any(|entry| entry.handle.level() == level)
            {
                return Err(Error::AlreadyBound);
            }
            if table.entries.len() >= 64 || table.next == u64::MAX {
                return Err(Error::Budget);
            }
            let reserved = |belongs: fn(&Entry, &str) -> bool, count: fn(&Entry) -> u32| {
                table
                    .entries
                    .values()
                    .filter(|entry| belongs(entry, owner))
                    .map(|entry| u64::from(count(entry)))
                    .sum::<u64>()
            };
            let own = |entry: &Entry, owner: &str| entry.owner == owner;
            let all = |_: &Entry, _: &str| true;
            if reserved(own, |e| e.limits.cells) + u64::from(limits.cells)
                > table.quotas.owner_cells
                || reserved(all, |e| e.limits.cells) + u64::from(limits.cells)
                    > table.quotas.total_cells
                || reserved(own, |e| e.limits.boxes) + u64::from(limits.boxes)
                    > table.quotas.owner_boxes
                || reserved(all, |e| e.limits.boxes) + u64::from(limits.boxes)
                    > table.quotas.total_boxes
                || reserved(own, |e| e.limits.chunks) + u64::from(limits.chunks)
                    > table.quotas.owner_chunks
                || reserved(all, |e| e.limits.chunks) + u64::from(limits.chunks)
                    > table.quotas.total_chunks
            {
                return Err(Error::Budget);
            }
        }
        if self.entity_ids_exhausted() {
            return Err(Error::Budget);
        }
        if !cells.iter().all(|&p| self.live_world_tick_ready(level, p)) {
            return Err(Error::NotResident);
        }
        let retained = retention::checked_retention(&cells, limits)?;
        let generation = self.live_worlds.borrow().next + 1;
        let handle = LiveWorldRef::new(level, generation);
        let entry = Entry {
            handle,
            owner,
            destination,
            cells,
            membership: 0,
            retained,
            stream_areas,
            limits,
            sample: LiveWorldSample::new(pose, pose, 0),
            collider: self.next_entity + 1,
            accepted_tick: None,
            tick_start: pose,
            carrying: false,
            frozen: false,
            geometry: RefCell::new(None),
            mesh: RefCell::new(None),
        };
        let geometry = self.projection_geometry(&entry)?;
        let placement = checked_pose(pose)?;
        let obstacles =
            self.projection_obstacles(&entry, &geometry, &placement, Vec3::ZERO, 0.0)?;
        // A zero translation only reports penetration, without interpreting touching as blocked.
        let checked = if limits.publication == mod_api::LiveWorldPublication::CollisionChecked {
            Some(
                geometry
                    .contact
                    .translate_step(
                        &placement,
                        Vec3::ZERO,
                        &obstacles,
                        &ContactBudget {
                            boxes: limits.boxes as usize,
                            iterations: limits.contacts as usize,
                        },
                    )
                    .map_err(|_| Error::Budget)?,
            )
        } else {
            None
        };
        if checked.is_some_and(|check| check.fraction < 1.0) {
            return Err(Error::Obstructed);
        }
        self.journal_projection_binding(level);
        self.dirty_live_world_level(level)?;
        self.live_world_durability.retired.remove(&level);
        self.next_entity += 1;
        let mut table = self.live_worlds.borrow_mut();
        table.next = generation;
        table.entries.insert(generation, entry);
        drop(table);
        self.refresh_live_world_retention(level);
        Ok(handle)
    }

    pub(super) fn live_world_publish_pose(
        &mut self,
        handle: LiveWorldRef,
        expected_revision: u64,
        pose: LiveWorldPose,
    ) -> Result<LiveWorldSample, Error> {
        if self.live_worlds.borrow().action.is_some() {
            return Err(Error::Busy);
        }
        let result = {
            let entry = self.projection_entry(handle)?;
            self.check_live_world_publication(entry.owner, entry.limits.publication)?;
            if entry.sample.revision != expected_revision
                || entry.accepted_tick == Some(self.live_worlds.borrow().sequence)
                || entry.carrying
            {
                return Err(Error::Revision);
            }
            if !entry
                .cells
                .iter()
                .all(|&p| self.live_world_tick_ready(handle.level(), p))
            {
                return Err(Error::NotResident);
            }
            let before = checked_pose(entry.sample.current)?;
            let requested = checked_pose(pose)?;
            // Preserve this binding's fixed pivot. Reparameterizing a caller pose changes
            // no physical point, and cannot change collision/render/history interpolation.
            let pivot = before.pivot();
            let after = RigidTransform::new(
                requested.to_world(pivot).subtract(pivot),
                pivot,
                requested.rotation(),
            )
            .map_err(|_| Error::Invalid)?;
            let q = after.rotation().components();
            let pose = LiveWorldPose {
                translation: after.translation().components(),
                pivot: pivot.components(),
                rotation: [q[1], q[2], q[3], q[0]],
            };
            let delta = after.translation().subtract(before.translation());
            let a = before.rotation().components();
            let b = after.rotation().components();
            let inverse =
                Rotation::from_components(a[0], -a[1], -a[2], -a[3]).map_err(|_| Error::Invalid)?;
            let relative = inverse.then(after.rotation()).components();
            let sine = vector([relative[1], relative[2], relative[3]]).length();
            let angle = 2.0 * sine.atan2(relative[0]);
            if delta
                .components()
                .iter()
                .any(|v| v.abs() > entry.limits.translation)
                || angle > entry.limits.rotation_degrees.to_radians()
            {
                return Err(Error::Invalid);
            }
            let turning = a != b;
            let geometry = self.projection_geometry(&entry)?;
            let obstacles = self.projection_obstacles(
                &entry,
                &geometry,
                &before,
                delta,
                if turning { angle } else { 0.0 },
            )?;
            let intervals =
                (angle / sunrise_world::carrier::contact::MAX_ROTATION_RADIANS).ceil() as u32;
            let intervals = intervals
                .max(
                    delta
                        .components()
                        .iter()
                        .map(|v| v.abs().ceil() as u32)
                        .max()
                        .unwrap_or(1),
                )
                .max(1);
            if intervals > entry.limits.motion_steps {
                return Err(Error::Budget);
            }
            // Reserving an equal share bounds total pair tests without adding another motion solver.
            let budget = ContactBudget {
                boxes: entry.limits.boxes as usize,
                iterations: (entry.limits.contacts / intervals) as usize,
            };
            let mut intermediate = before;
            let checked_intervals =
                if entry.limits.publication == mod_api::LiveWorldPublication::CollisionChecked {
                    intervals
                } else {
                    0
                };
            // ExternallyResolved consumes the authorized owner's contact-resolved sample. Candidate
            // residency/shape/step/work admission above remains; no second dynamics or pose clamp runs.
            for _ in 0..checked_intervals {
                let axis = if sine > 0.0 {
                    vector([relative[1], relative[2], relative[3]])
                } else {
                    Vec3::new(0.0, 1.0, 0.0)
                };
                let checked = geometry
                    .contact
                    .rigid_step(
                        &intermediate,
                        delta.scale(1.0 / f64::from(intervals)),
                        axis,
                        angle / f64::from(intervals),
                        &obstacles,
                        &budget,
                    )
                    .map_err(|_| Error::Budget)?;
                if checked.fraction < 1.0 {
                    return Err(Error::Obstructed);
                }
                intermediate = if angle == 0.0 {
                    intermediate
                } else {
                    intermediate
                        .rotated(axis, angle / f64::from(intervals))
                        .map_err(|_| Error::Invalid)?
                }
                .translated(delta.scale(1.0 / f64::from(intervals)));
            }
            LiveWorldSample::new(
                entry.tick_start,
                pose,
                entry.sample.revision.checked_add(1).ok_or(Error::Budget)?,
            )
        };
        self.journal_projection_pose(handle);
        let mut table = self.live_worlds.borrow_mut();
        let sequence = table.sequence;
        let entry = table
            .entries
            .get_mut(&handle.generation())
            .ok_or(Error::Stale)?;
        entry.sample = result;
        entry.accepted_tick = Some(sequence);
        Ok(result)
    }

    pub(super) fn level_checkpoint(
        &mut self,
        level: LevelRef,
    ) -> Result<mod_api::LiveWorldSaveReceipt, Error> {
        let owner = self.projection_owner()?;
        if !self.live_world_lifecycle_current(level) {
            return Err(Error::Stale);
        }
        if level.generation() != 0 && self.levels.runtime.owner_of(level.dimension_id()) != owner {
            return Err(Error::Denied);
        }
        let mut receipt = self.checkpoint_live_world_levels(&[level])?;
        receipt.levels.retain(|row| {
            row.level.generation() == 0
                || self.levels.runtime.owner_of(row.level.dimension_id()) == owner
        });
        Ok(receipt)
    }
    pub(super) fn level_saved_projection(
        &self,
        level: LevelRef,
    ) -> Result<mod_api::LiveWorldSavedProjection, Error> {
        self.saved_projection_descriptor(level)
    }
    pub(super) fn level_restore_projection(
        &mut self,
        level: LevelRef,
    ) -> Result<LiveWorldRef, Error> {
        self.restore_saved_projection(level)
    }

    pub(super) fn live_world_retire(&mut self, handle: LiveWorldRef) -> Result<(), Error> {
        self.projection_entry(handle)?;
        if self.transaction_current().is_some() && self.projection_has_open_menu(handle) {
            return Err(Error::Busy);
        }
        self.close_projection_menus(handle)?;
        let members = self.projection_entry(handle)?.cells.len() as u32;
        self.journal_projection_binding(handle.level());
        self.record_projection_retirement(handle, members)?;
        self.live_worlds
            .borrow_mut()
            .entries
            .remove(&handle.generation());
        self.refresh_live_world_retention(handle.level());
        Ok(())
    }

    fn projection_obstacles(
        &self,
        entry: &Entry,
        geometry: &Geometry,
        pose: &RigidTransform,
        delta: Vec3,
        angle: f64,
    ) -> Result<Vec<Obstacle>, Error> {
        let Some(mut area) = geometry.contact.world_bounds(pose) else {
            return Ok(Vec::new());
        };
        area = area.union_bounds(area.move_by(delta));
        if angle != 0.0 {
            // Every corner moves by at most radius * angle during this small angular sweep.
            let pivot = pose.translation().add_vector(pose.pivot());
            let radius = Axis::ALL
                .iter()
                .map(|&axis| {
                    (area.min(axis) - pivot.component(axis))
                        .abs()
                        .max((area.max(axis) - pivot.component(axis)).abs())
                })
                .map(|v| v * v)
                .sum::<f64>()
                .sqrt();
            let margin = radius * angle.abs();
            area = area.inflate(Vec3::new(margin, margin, margin));
        }
        if Axis::ALL
            .iter()
            .any(|&axis| area.min(axis) <= -30_000_000.0 || area.max(axis) >= 30_000_000.0)
        {
            return Err(Error::Invalid);
        }
        let lo = Axis::ALL.map(|axis| area.min(axis).floor() as i32 - 1);
        let hi = Axis::ALL.map(|axis| area.max(axis).ceil() as i32 + 1);
        let count = (0..3)
            .try_fold(1u64, |n, i| {
                n.checked_mul((i64::from(hi[i]) - i64::from(lo[i]) + 1) as u64)
            })
            .ok_or(Error::Budget)?;
        let chunks = (i64::from(hi[0] >> 4) - i64::from(lo[0] >> 4) + 1)
            .checked_mul(i64::from(hi[2] >> 4) - i64::from(lo[2] >> 4) + 1)
            .ok_or(Error::Budget)?;
        if count > u64::from(entry.limits.candidates) || chunks > i64::from(entry.limits.chunks) {
            return Err(Error::Budget);
        }
        let world = self
            .native_level_world(entry.destination)
            .ok_or(Error::Stale)?;
        let mut result = Vec::new();
        let mut boxes = 0usize;
        for x in lo[0]..=hi[0] {
            for z in lo[2]..=hi[2] {
                if !world.loaded(x, z) {
                    return Err(Error::NotResident);
                }
                for y in lo[1]..=hi[1] {
                    if !world.shape.contains_y(y) {
                        return Err(Error::NotResident);
                    }
                    let pos = [x, y, z];
                    let shape = destination_shape(self, world, pos)?;
                    boxes = boxes
                        .checked_add(shape.boxes().len())
                        .ok_or(Error::Budget)?;
                    if boxes > entry.limits.boxes as usize {
                        return Err(Error::Budget);
                    }
                    if !shape.is_empty() {
                        result.push(Obstacle::world_block(position(pos), shape));
                    }
                }
            }
        }
        Ok(result)
    }

    /// Capture the physical start before server callbacks can publish a new external pose.
    pub(in crate::game) fn live_world_begin_tick(&mut self) {
        self.live_worlds.get_mut().sequence = self
            .live_worlds
            .get_mut()
            .sequence
            .checked_add(1)
            .expect("server tick sequence exhausted");
        for entry in self.live_worlds.get_mut().entries.values_mut() {
            entry.tick_start = entry.sample.current;
            entry.sample.previous = entry.sample.current;
            entry.carrying = false;
        }
    }

    /// Endpoints become physical together before the existing collided carry pass.
    pub(in crate::game) fn live_world_end_level(&mut self) {
        for entry in self
            .live_worlds
            .get_mut()
            .entries
            .values_mut()
            .filter(|entry| entry.destination.dimension() == self.dim.key())
        {
            entry.carrying = true;
        }
    }

    /// One local world's block/entity/scheduled work pauses as a unit if membership loses residency.
    pub(in crate::game) fn live_world_local_ready(&self) -> bool {
        self.live_worlds
            .borrow()
            .entries
            .values()
            .filter(|entry| entry.handle.level().dimension() == self.dim.key())
            .all(|entry| {
                entry
                    .cells
                    .iter()
                    .all(|&pos| self.live_world_tick_ready(entry.handle.level(), pos))
            })
    }

    /// Engine lookup, without mod authority: the same frame the carry pass reads.
    pub(in crate::game) fn live_world_frame(&self, id: u32) -> Option<Frame> {
        if !self.is_logical_server() {
            return self.client_projection_frame(id);
        }
        self.live_worlds
            .borrow()
            .entries
            .values()
            .find(|entry| {
                entry.collider == id
                    && entry.destination.dimension() == self.dim.key()
                    && self.level_handle_current(entry.handle.level())
                    && self.level_handle_current(entry.destination)
            })
            .map(|entry| frame(entry.sample.current))
    }

    pub(in crate::game) fn live_world_colliders(&self, base: Prepared) -> Prepared {
        if !self.is_logical_server() {
            return self.client_projection_colliders(base);
        }
        let table = self.live_worlds.borrow();
        if table.entries.is_empty() {
            return base;
        }
        let projected: Vec<_> = table
            .entries
            .values()
            .filter(|entry| {
                entry.destination.dimension() == self.dim.key()
                    && self.level_handle_current(entry.handle.level())
                    && self.level_handle_current(entry.destination)
            })
            .filter_map(|entry| {
                // A temporarily unavailable source freezes the last coherent derived collider.
                // It never becomes observed empty terrain. New publications still require a
                // fresh successful query; explicit empty/closed bindings are retired separately.
                self.projection_geometry(entry)
                    .or_else(|error| entry.geometry.borrow().clone().ok_or(error))
                    .ok()
                    .map(|geometry| Placed {
                        id: entry.collider,
                        collider: OrientedCollider::new(
                            frame(if entry.carrying {
                                entry.sample.current
                            } else {
                                entry.tick_start
                            }),
                            geometry.boxes.clone(),
                        ),
                        exempt: Vec::new(),
                    })
            })
            .collect();
        if projected.is_empty() {
            return base;
        }
        let mut placed: Vec<_> = base
            .placed()
            .map(|p| Placed {
                id: p.id,
                collider: p.collider.clone(),
                exempt: p.exempt.clone(),
            })
            .collect();
        placed.extend(projected);
        Prepared::new(placed)
    }

    fn projection_mesh(&self, entry: &Entry, world: &World) -> Rc<Mesh> {
        let resources = self.standalone_geometry.generation();
        if let Some(mesh) = entry.mesh.borrow().as_ref()
            && (entry.frozen
                || entry
                    .retained
                    .iter()
                    .any(|chunk| !world.chunks.contains_key(chunk))
                || (mesh.source_revision == world.revision
                    && mesh.membership == entry.membership
                    && mesh.resources == resources))
        {
            return mesh.clone();
        }
        let cells = entry
            .cells
            .iter()
            .filter_map(|&pos| {
                if !world.is_loaded_at(pos[0], pos[1], pos[2]) {
                    return None;
                }
                let state = world.block(pos[0], pos[1], pos[2]);
                let seed = sunrise_core::random::position_seed(position(pos));
                let faces = self
                    .pack
                    .render
                    .model_catalog
                    .collect_seeded(state, seed)
                    .ok()
                    .into_iter()
                    .flatten()
                    .flat_map(|(_, model)| model.faces.iter())
                    .filter_map(|face| {
                        let sprite = self
                            .pack
                            .layer_names
                            .get(&face.layer)
                            .and_then(|name| self.gui.atlas.sprites.get(name))?;
                        Some(super::minecarts::PreparedModelFace::new(
                            &face.quad, *sprite,
                        ))
                    })
                    .collect();
                Some(MeshCell { pos, state, faces })
            })
            .collect();
        let mesh = Rc::new(Mesh {
            source_revision: world.revision,
            membership: entry.membership,
            resources,
            cells,
        });
        *entry.mesh.borrow_mut() = Some(mesh.clone());
        mesh
    }

    /// Uses the existing transformed block model drawing on the server fixture or real client replicas.
    pub(in crate::game) fn live_world_geometry(
        &self,
        camera: glam::DVec3,
        partial: f64,
        out: &mut Vec<Vertex>,
    ) {
        if !self.is_logical_server() {
            self.client_projection_geometry(camera, partial, out);
            return;
        }
        let table = self.live_worlds.borrow();
        for entry in table
            .entries
            .values()
            .filter(|entry| entry.destination.dimension() == self.dim.key())
        {
            self.draw_projection(
                entry,
                [entry.sample.previous, entry.sample.current],
                camera,
                partial,
                out,
            );
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn projection_block_entity_model(
        &self,
        entry: &Entry,
        world: &World,
        pos: [i32; 3],
        state: sunrise_world::registry::BlockStateId,
        matrix: glam::Mat4,
        light: u8,
        partial: f64,
        out: &mut Vec<Vertex>,
    ) {
        let level = entry.handle.level();
        let anims = if self.is_logical_server() {
            if level.dimension() == self.dim.key() {
                Some(&self.be_anims)
            } else {
                self.levels
                    .parked_anims(super::Dim::from_id(level.dimension_id()))
            }
        } else {
            self.remote
                .as_ref()
                .and_then(|r| r.viewed.anims_of(level.dimension()))
        };
        let Some(anims) = anims else {
            return;
        };
        let lid = |p| anims.lid(p, partial as f32);
        let anim = |p| anims.block_anim(p, world.game_time, partial as f32, Default::default());
        let scene = sunrise_client::block_entity_render::Scene {
            camera: glam::DVec3::ZERO,
            orientation: glam::Quat::IDENTITY,
            anim: &anim,
            texts: None,
            partial: partial as f32,
            game_time: world.game_time,
            max_y: world.shape.max_y() - 1,
            world,
            sprites: &self.gui.atlas.sprites,
            emissive: &self.gui.atlas.emissive,
            translucent_sprites: &self.gui.atlas.translucent,
            atlas_revision: self.gui.atlas.revision,
            nether: self.nether_lighting(),
            lid: &lid,
            sign: &|_| None,
            banner: &|_| Vec::new(),
            skull: &|_| None,
            glyph: &|c| self.gui.atlas.glyph_cell(c),
            glyphs: self.gui.atlas.font.epoch(),
            font: Some(&self.gui.atlas.font),
            dynamic_visible: None,
        };
        self.res
            .block_entity_render()
            .render_block(&scene, pos, state.0, matrix, light, out);
    }

    fn draw_projection(
        &self,
        entry: &Entry,
        poses: [LiveWorldPose; 2],
        camera: glam::DVec3,
        partial: f64,
        out: &mut Vec<Vertex>,
    ) {
        let Some(world) = self.native_level_view_world(entry.handle.level()) else {
            return;
        };
        let previous = frame(poses[0]);
        let current = frame(poses[1]);
        let pose = previous.interpolate(&current, partial);
        let origin = pose.to_world(Vec3::ZERO);
        let axes = [
            Vec3::new(1.0, 0.0, 0.0),
            Vec3::new(0.0, 1.0, 0.0),
            Vec3::new(0.0, 0.0, 1.0),
        ]
        .map(|v| pose.to_world(v).subtract(origin));
        let matrix = glam::DMat4::from_cols(
            glam::DVec4::new(axes[0].x, axes[0].y, axes[0].z, 0.0),
            glam::DVec4::new(axes[1].x, axes[1].y, axes[1].z, 0.0),
            glam::DVec4::new(axes[2].x, axes[2].y, axes[2].z, 0.0),
            (glam::DVec3::from_array(origin.components()) - camera).extend(1.0),
        );
        let vertex_start = out.len();
        let tracing = trace::enabled();
        let mut first_cell_matrix = None;
        let mesh = self.projection_mesh(entry, world);
        for cell in &mesh.cells {
            let pos = cell.pos;
            let at = pose.to_world(vector(pos.map(f64::from)));
            let packed = self.world.light(
                at.x.floor() as i32,
                at.y.floor() as i32,
                at.z.floor() as i32,
            );
            let light = sunrise_client::block_entity_render::vertex_light(packed);
            let cell_matrix = (matrix
                * glam::DMat4::from_translation(glam::DVec3::from_array(pos.map(f64::from))))
            .as_mat4();
            if tracing && first_cell_matrix.is_none() {
                first_cell_matrix = Some((pos, cell_matrix.to_cols_array()));
            }
            super::minecarts::emit_model_faces(
                cell.faces.iter().copied().map(|face| (face, light)),
                cell_matrix,
                true,
                |_| None,
                out,
            );
            if self
                .res
                .facts()
                .get(cell.state)
                .name
                .strip_prefix("minecraft:")
                .is_some_and(super::text_displays::has_special_model)
            {
                self.projection_block_entity_model(
                    entry,
                    world,
                    pos,
                    cell.state,
                    cell_matrix,
                    packed,
                    partial,
                    out,
                );
            }
        }
        trace::emit("model_draw", || {
            serde_json::json!({
                "side": if self.is_logical_server() { "server" } else { "client" },
                "generation": entry.handle.generation(), "collider": entry.collider,
                "client_tick": self.ticks, "coherent_sample": trace::sample(entry.sample),
                "membership": entry.membership, "frozen": entry.frozen,
                "poses": poses.map(trace::pose), "partial": partial, "partial_bits": partial.to_bits(),
                "camera": camera.to_array(), "camera_bits": camera.to_array().map(f64::to_bits),
                "transform": trace::transform(&pose),
                "camera_relative_matrix": matrix.to_cols_array(),
                "camera_relative_matrix_bits": matrix.to_cols_array().map(f64::to_bits),
                "first_cell_matrix": first_cell_matrix.map(|(pos, matrix)| serde_json::json!({
                    "cell": pos, "matrix_f32": matrix, "matrix_f32_bits": matrix.map(f32::to_bits)
                })),
                "first_vertex": out.get(vertex_start).map(|vertex| serde_json::json!({
                    "position_f32": vertex.pos, "position_f32_bits": vertex.pos.map(f32::to_bits)
                })),
                "mesh_cells": mesh.cells.len(), "vertices": out.len() - vertex_start
            })
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn configured_coordinates_and_sparse_chunk_budget_are_checked_before_copying() {
        let limits = LiveWorldLimits {
            min: [-1024, -64, -1024],
            max: [1024, 319, 1024],
            chunks: 2,
            ..Default::default()
        };
        assert_eq!(
            checked_cells(&[[512, 64, 0], [-512, 64, 0]], limits).unwrap(),
            [[-512, 64, 0], [512, 64, 0]]
        );
        assert_eq!(
            checked_cells(&[[512, 64, 0], [512, 64, 0]], limits),
            Err(Error::Invalid)
        );
        assert_eq!(
            checked_cells(&[[512, 64, 0], [-512, 64, 0], [0, 64, 0]], limits),
            Err(Error::Budget)
        );
        assert_eq!(checked_cells(&[[1025, 64, 0]], limits), Err(Error::Invalid));
    }
    #[test]
    fn pivot_frame_and_rigid_transform_agree_for_a_quarter_turn() {
        let h = std::f64::consts::FRAC_1_SQRT_2;
        let pose = LiveWorldPose {
            translation: [10.0, 64.0, 10.0],
            pivot: [1.0, 0.0, 0.0],
            rotation: [0.0, h, 0.0, h],
        };
        let point = Vec3::new(2.0, 0.0, 0.0);
        let actual = frame(pose).to_world(point);
        assert!(actual.distance_squared(Vec3::new(11.0, 64.0, 9.0)) < 1.0e-25);
        assert!(actual.distance_squared(checked_pose(pose).unwrap().to_world(point)) < 1.0e-25);
        let mut invalid = pose;
        invalid.rotation = [0.0; 4];
        assert!(matches!(checked_pose(invalid), Err(Error::Invalid)));
    }
}

impl Game<'_> {
    fn transfer_level_parts(
        &self,
        level: LevelRef,
    ) -> Option<(&World, &sunrise_world::level::Ticks)> {
        if !self.live_world_lifecycle_current(level) {
            return None;
        }
        if self.dim.key() == level.dimension() {
            Some((&self.world, &self.block_ticks))
        } else {
            self.levels.parked_transfer_parts(level.dimension_id())
        }
    }
    fn transfer_level_parts_mut(
        &mut self,
        level: LevelRef,
    ) -> Option<(&mut World, &mut sunrise_world::level::Ticks)> {
        if !self.live_world_lifecycle_current(level) {
            return None;
        }
        if self.dim.key() == level.dimension() {
            Some((&mut self.world, &mut self.block_ticks))
        } else {
            self.levels.parked_transfer_parts_mut(level.dimension_id())
        }
    }
}

impl Game<'_> {
    fn check_live_world_publication(
        &self,
        owner: &str,
        policy: mod_api::LiveWorldPublication,
    ) -> Result<(), Error> {
        match policy {
            mod_api::LiveWorldPublication::CollisionChecked => Ok(()),
            mod_api::LiveWorldPublication::ExternallyResolved
                if self
                    .live_worlds
                    .borrow()
                    .external_pose_owners
                    .contains(owner) =>
            {
                Ok(())
            }
            _ => Err(Error::Denied),
        }
    }
    /// Integrated servers read only the existing game-directory server properties, without
    /// normalizing, creating or saving that file. Dedicated startup already uses its ordinary parser.
    pub(in crate::game) fn read_integrated_live_world_properties(&mut self) {
        if self.options.execution_role.is_dedicated_server() {
            return;
        }
        let read = || -> Option<sunrise_java_properties::Properties> {
            use std::io::Read as _;
            let path = std::env::current_dir().ok()?.join("server.properties");
            let metadata = std::fs::symlink_metadata(&path).ok()?;
            if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() > 65536 {
                return None;
            }
            let mut bytes = Vec::with_capacity(metadata.len() as usize);
            std::fs::File::open(path)
                .ok()?
                .take(65537)
                .read_to_end(&mut bytes)
                .ok()?;
            if bytes.len() > 65536 {
                return None;
            }
            sunrise_java_properties::Properties::load_file(&bytes).ok()
        };
        let properties = read();
        self.read_live_world_quotas(|key| properties.as_ref().and_then(|p| p.get(key)));
    }
}

#[cfg(test)]
#[path = "native_live_worlds/perf_tests.rs"]
mod perf_tests;

#[cfg(test)]
#[path = "native_live_worlds/fence_tests.rs"]
mod fence_tests;
