//! Draft `native_live_worlds/persistence_codec.rs`, not integrated or compiled.
//! Schema 2 stores every accepted f64 component as u64 bits; no legacy decoder or normalization on restore.
use super::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PoseBits {
    translation: [u64; 3],
    pivot: [u64; 3],
    rotation: [u64; 4],
}
impl From<LiveWorldPose> for PoseBits {
    fn from(pose: LiveWorldPose) -> Self {
        Self {
            translation: pose.translation.map(f64::to_bits),
            pivot: pose.pivot.map(f64::to_bits),
            rotation: pose.rotation.map(f64::to_bits),
        }
    }
}
impl PoseBits {
    pub fn decode(self) -> Result<LiveWorldPose, Error> {
        let pose = LiveWorldPose {
            translation: self.translation.map(f64::from_bits),
            pivot: self.pivot.map(f64::from_bits),
            rotation: self.rotation.map(f64::from_bits),
        };
        checked_pose(pose)?;
        // checked_pose constructs a derived collision transform; its normalized components are not a new
        // accepted sample. The host's saved current receipt must retain its exact original finite bits.
        Ok(pose)
    }
}

#[derive(Clone, Copy, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct LimitsBits {
    min: [i32; 3],
    max: [i32; 3],
    cells: u32,
    chunks: u32,
    boxes: u32,
    contacts: u32,
    candidates: u32,
    motion_steps: u32,
    translation: u64,
    rotation_degrees: u64,
    publication: u8,
}
impl From<LiveWorldLimits> for LimitsBits {
    fn from(limits: LiveWorldLimits) -> Self {
        Self {
            min: limits.min,
            max: limits.max,
            cells: limits.cells,
            chunks: limits.chunks,
            boxes: limits.boxes,
            contacts: limits.contacts,
            candidates: limits.candidates,
            motion_steps: limits.motion_steps,
            translation: limits.translation.to_bits(),
            rotation_degrees: limits.rotation_degrees.to_bits(),
            publication: match limits.publication {
                mod_api::LiveWorldPublication::CollisionChecked => 0,
                mod_api::LiveWorldPublication::ExternallyResolved => 1,
                _ => u8::MAX,
            },
        }
    }
}
impl LimitsBits {
    pub fn decode(self) -> Result<LiveWorldLimits, Error> {
        let limits = LiveWorldLimits {
            min: self.min,
            max: self.max,
            cells: self.cells,
            chunks: self.chunks,
            boxes: self.boxes,
            contacts: self.contacts,
            candidates: self.candidates,
            motion_steps: self.motion_steps,
            translation: f64::from_bits(self.translation),
            rotation_degrees: f64::from_bits(self.rotation_degrees),
            publication: match self.publication {
                0 => mod_api::LiveWorldPublication::CollisionChecked,
                1 => mod_api::LiveWorldPublication::ExternallyResolved,
                _ => return Err(Error::Storage),
            },
        };
        checked_cells(&[], limits)?;
        Ok(limits)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn nonidentity_accepted_pose_roundtrips_all_component_bits() {
        // A finite canonical sample with a nonzero logical pivot, nontrivial translation and rotation. The
        // expected value is the original receipt bits, not a second call through quaternion normalization.
        let pose = LiveWorldPose {
            translation: [f64::from_bits(0x4026666666666666), -0.1, 10.0],
            pivot: [3.0, 64.0, -2.0],
            rotation: [0.0, 0.008726535498373935, 0.0, 0.9999619230641713],
        };
        let bits = PoseBits::from(pose);
        let encoded = serde_json::to_vec(&bits).unwrap();
        let decoded = serde_json::from_slice::<PoseBits>(&encoded)
            .unwrap()
            .decode()
            .unwrap();
        assert_eq!(PoseBits::from(decoded), bits);
    }
    #[test]
    fn malformed_saved_float_bits_do_not_reach_projection_publication() {
        let mut bits = PoseBits::from(LiveWorldPose::at([0.0, 0.0, 0.0]));
        bits.translation[0] = f64::NAN.to_bits();
        assert_eq!(bits.decode(), Err(Error::Invalid));
        bits = PoseBits::from(LiveWorldPose::at([0.0, 0.0, 0.0]));
        bits.rotation = [0; 4];
        assert_eq!(bits.decode(), Err(Error::Invalid));
    }
}

/// One current-format durable descriptor. An explicit tombstone is not a missing checkpoint.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct SavedLevel {
    pub schema: u32,
    pub owner: String,
    pub source: String,
    pub barrier_epoch: u64,
    pub retired_generation: Option<u64>,
    pub retired_members: Option<u32>,
    pub binding: Option<SavedBinding>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct SavedBinding {
    pub destination: String,
    pub generation: u64,
    pub pose_revision: u64,
    pub membership: u64,
    pub pose: PoseBits,
    pub cells: Vec<[i32; 3]>,
    pub limits: LimitsBits,
    /// The admitted cover is persisted: removing sparse members must not fragment the guest budget.
    pub stream_areas: Vec<[i32; 3]>,
}
impl SavedLevel {
    pub fn checked(&self, owner: &str, source: &str) -> Result<(), Error> {
        if self.schema != 2
            || self.owner != owner
            || self.source != source
            || self.barrier_epoch == 0
        {
            return Err(Error::Storage);
        }
        if self.retired_members.is_some() && self.retired_generation.is_none() {
            return Err(Error::Storage);
        }
        if let Some(binding) = &self.binding {
            if self.retired_generation.is_some()
                || binding.generation == 0
                || binding.generation == u64::MAX
                || !sunrise_core::level_id::valid_level_id(&binding.destination)
            {
                return Err(Error::Storage);
            }
            let limits = binding.limits.decode()?;
            let cells = checked_cells(&binding.cells, limits)?;
            if cells.is_empty() || cells != binding.cells {
                return Err(Error::Storage);
            }
            retention::checked_retention(&cells, limits)?;
            binding.pose.decode()?;
            if binding.stream_areas.is_empty() || binding.stream_areas.len() > 32 {
                return Err(Error::Budget);
            }
            let mut distinct = BTreeSet::new();
            for &area in &binding.stream_areas {
                if !(0..=32).contains(&area[2])
                    || area[0].unsigned_abs()
                        > sunrise_client::lan::viewed_levels::areas::MAX_CENTER as u32
                    || area[1].unsigned_abs()
                        > sunrise_client::lan::viewed_levels::areas::MAX_CENTER as u32
                    || !distinct.insert(area)
                {
                    return Err(Error::Storage);
                }
            }
            let areas: Vec<_> = binding
                .stream_areas
                .iter()
                .map(|p| sunrise_client::lan::viewed_levels::areas::ChunkArea {
                    level: source.to_owned(),
                    center: (p[0], p[1]),
                    radius: u32::try_from(p[2]).unwrap_or(u32::MAX),
                    mode: sunrise_client::lan::viewed_levels::areas::ChunkAreaMode::Loaded,
                })
                .collect();
            for p in &binding.cells {
                let x = p[0] >> 4;
                let z = p[2] >> 4;
                if !areas.iter().any(|a| {
                    a.radius <= 32
                        && (i64::from(x) - i64::from(a.center.0)).abs() <= i64::from(a.radius)
                        && (i64::from(z) - i64::from(a.center.1)).abs() <= i64::from(a.radius)
                }) {
                    return Err(Error::Storage);
                }
            }
            // Complete aggregate view/chunk and current guest capability admission is rechecked at restore.
        }
        Ok(())
    }
}

impl Game<'_> {
    pub(super) fn read_projection_bytes(&self, source: LevelRef) -> Result<Option<Vec<u8>>, Error> {
        sunrise_client::save::live_world_journal::read_projection(
            &self.world_root,
            source.dimension_id(),
        )
        .map_err(|_| Error::Storage)
    }
    pub(super) fn projection_transfer_afterimage(
        &self,
        source: LevelRef,
        epoch: u64,
        cells: &[mod_api::LiveWorldTransferCell],
    ) -> Result<Option<Vec<u8>>, Error> {
        let bytes = self.projection_save_image(source, epoch)?;
        let bound = self
            .live_worlds
            .borrow()
            .entries
            .values()
            .any(|e| e.handle.level() == source);
        if !bound {
            return Ok(bytes);
        }
        let mut metadata: SavedLevel =
            serde_json::from_slice(bytes.as_deref().ok_or(Error::Storage)?)
                .map_err(|_| Error::Storage)?;
        let binding = metadata.binding.as_mut().ok_or(Error::Storage)?;
        let before = binding.cells.len();
        binding
            .cells
            .retain(|p| !cells.iter().any(|c| c.source == source && c.from == *p));
        if binding.cells.len() != before {
            binding.membership = binding.membership.checked_add(1).ok_or(Error::Budget)?;
            if binding.cells.is_empty() {
                metadata.retired_generation = Some(binding.generation);
                metadata.retired_members = Some(0);
                metadata.binding = None;
            }
        }
        metadata.checked(&metadata.owner, source.dimension_id())?;
        serde_json::to_vec(&metadata)
            .map(Some)
            .map_err(|_| Error::Storage)
    }
    pub(super) fn projection_save_image(
        &self,
        source: LevelRef,
        epoch: u64,
    ) -> Result<Option<Vec<u8>>, Error> {
        if let Some((level, _, converted)) = &self.live_world_durability.migration
            && *level == source
        {
            let mut converted = converted.clone();
            converted.barrier_epoch = epoch;
            converted.checked(&converted.owner, source.dimension_id())?;
            return serde_json::to_vec(&converted)
                .map(Some)
                .map_err(|_| Error::Storage);
        }
        let table = self.live_worlds.borrow();
        let entry = table
            .entries
            .values()
            .find(|entry| entry.handle.level() == source);
        let retired = self.live_world_durability.retired.get(&source);
        if entry.is_none() && retired.is_none() {
            return self.read_projection_bytes(source);
        }
        let owner = entry
            .map(|entry| entry.owner)
            .unwrap_or_else(|| self.levels.runtime.owner_of(source.dimension_id()));
        let binding = entry.map(|entry| SavedBinding {
            destination: entry.destination.dimension_id().to_owned(),
            generation: entry.handle.generation(),
            pose_revision: entry.sample.revision,
            membership: entry.membership,
            pose: entry.sample.current.into(),
            cells: entry.cells.clone(),
            limits: entry.limits.into(),
            stream_areas: entry
                .stream_areas
                .iter()
                .map(|area| [area.center.0, area.center.1, area.radius as i32])
                .collect(),
        });
        let current_revision = self
            .native_level_world(source)
            .ok_or(Error::Stale)?
            .revision;
        let metadata = SavedLevel {
            schema: 2,
            owner: owner.to_owned(),
            source: source.dimension_id().to_owned(),
            barrier_epoch: epoch,
            retired_generation: retired.map(|retired| retired.generation),
            retired_members: retired
                .filter(|retired| retired.source_revision == current_revision)
                .map(|retired| retired.members),
            binding,
        };
        metadata.checked(owner, source.dimension_id())?;
        let bytes = serde_json::to_vec(&metadata).map_err(|_| Error::Storage)?;
        if bytes.len() > 16 * 1024 * 1024 {
            return Err(Error::Budget);
        }
        Ok(Some(bytes))
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LegacyPose {
    translation: [f64; 3],
    pivot: [f64; 3],
    rotation: [f64; 4],
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LegacyLimits {
    min: [i32; 3],
    max: [i32; 3],
    cells: u32,
    chunks: u32,
    boxes: u32,
    contacts: u32,
    candidates: u32,
    translation: f64,
    rotation_degrees: f64,
    motion_steps: u32,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LegacyProjection {
    schema: u32,
    owner: String,
    source: String,
    destination: String,
    generation: u64,
    pose_revision: u64,
    pose: LegacyPose,
    cells: Vec<[i32; 3]>,
    limits: LegacyLimits,
}
impl Game<'_> {
    pub(in crate::game) fn level_migrate_projection(
        &mut self,
        source: LevelRef,
    ) -> Result<mod_api::LiveWorldSavedProjection, Error> {
        if !self.live_world_commit_frontier()
            || self.has_open_transaction()
            || self.live_world_checkpoint_running()
            || self.live_world_durability.migration.is_some()
        {
            return Err(Error::Busy);
        }
        let (converted, legacy, original) = self.decode_saved_projection(source)?;
        if !legacy {
            return self.saved_projection_descriptor(source);
        }
        if self
            .live_worlds
            .borrow()
            .entries
            .values()
            .any(|entry| entry.handle.level() == source)
        {
            return Err(Error::AlreadyBound);
        }
        if let Some(previous) = sunrise_client::save::live_world_journal::read_projection_original(
            &self.world_root,
            source.dimension_id(),
        )
        .map_err(|_| Error::Storage)?
            && previous != original
        {
            return Err(Error::Storage);
        }
        self.live_world_durability.migration = Some((source, original, converted));
        let result = self.checkpoint_live_world_levels(&[source]);
        self.live_world_durability.migration = None;
        result?;
        self.saved_projection_descriptor(source)
    }
    /// Reopening a durable explicit retirement retains its complete membership witness until an
    /// actual later source block write changes this open generation's block revision.
    pub(in crate::game) fn remember_saved_projection_retirement(
        &mut self,
        source: LevelRef,
    ) -> Result<(), Error> {
        if source.generation() == 0 {
            return Ok(());
        }
        let Some(bytes) = self.read_projection_bytes(source)? else {
            return Ok(());
        };
        let value: serde_json::Value =
            serde_json::from_slice(&bytes).map_err(|_| Error::Storage)?;
        if value.get("schema").and_then(serde_json::Value::as_u64) == Some(1) {
            return Ok(());
        }
        let saved: SavedLevel = serde_json::from_value(value).map_err(|_| Error::Storage)?;
        saved.checked(
            self.levels.runtime.owner_of(source.dimension_id()),
            source.dimension_id(),
        )?;
        let epoch = sunrise_client::save::live_world_journal::read_epoch(&self.world_root)
            .map_err(|_| Error::Storage)?;
        if saved.barrier_epoch > epoch {
            return Err(Error::Storage);
        }
        if saved.binding.is_none()
            && let (Some(generation), Some(members)) =
                (saved.retired_generation, saved.retired_members)
        {
            let revision = self
                .native_level_world(source)
                .ok_or(Error::Stale)?
                .revision;
            self.live_world_durability.retired.insert(
                source,
                super::binding_journal::RetiredBinding {
                    generation,
                    members,
                    source_revision: revision,
                },
            );
        }
        Ok(())
    }
    pub(super) fn decode_saved_projection(
        &self,
        source: LevelRef,
    ) -> Result<(SavedLevel, bool, Vec<u8>), Error> {
        let owner = self.projection_owner()?;
        if !self.live_world_lifecycle_current(source) {
            return Err(Error::Stale);
        }
        if !self.levels.runtime.owned_void(source.dimension_id(), owner) {
            return Err(Error::NotLocalWorld);
        }
        let bytes = self.read_projection_bytes(source)?.ok_or(Error::NotSaved)?;
        let value: serde_json::Value =
            serde_json::from_slice(&bytes).map_err(|_| Error::Storage)?;
        let schema = value["schema"].as_u64().ok_or(Error::Storage)?;
        let (metadata, legacy) = if schema == 2 {
            let metadata: SavedLevel = serde_json::from_value(value).map_err(|_| Error::Storage)?;
            metadata.checked(owner, source.dimension_id())?;
            let epoch = sunrise_client::save::live_world_journal::read_epoch(&self.world_root)
                .map_err(|_| Error::Storage)?;
            if metadata.barrier_epoch > epoch {
                return Err(Error::Storage);
            }
            (metadata, false)
        } else if schema == 1 {
            let old: LegacyProjection =
                serde_json::from_value(value).map_err(|_| Error::Storage)?;
            if old.schema != 1 || old.owner != owner || old.source != source.dimension_id() {
                return Err(Error::Stale);
            }
            let l = old.limits;
            let mut limits = LiveWorldLimits {
                min: l.min,
                max: l.max,
                cells: l.cells,
                chunks: l.chunks,
                boxes: l.boxes,
                contacts: l.contacts,
                candidates: l.candidates,
                translation: l.translation,
                rotation_degrees: l.rotation_degrees,
                motion_steps: l.motion_steps,
                publication: mod_api::LiveWorldPublication::CollisionChecked,
            };
            let footprint = retention::selection_footprint(source, &old.cells)?;
            // Explicit development migration charges the actual terrain cover, which the earlier
            // experiment incorrectly counted as member columns. No cells or inventories are seeded.
            limits.chunks = limits.chunks.max(footprint.retained_columns);
            let cells = checked_cells(&old.cells, limits)?;
            let pose = LiveWorldPose {
                translation: old.pose.translation,
                pivot: old.pose.pivot,
                rotation: old.pose.rotation,
            };
            checked_pose(pose)?;
            let areas = sync::stream_areas(source, &cells)?;
            let metadata = SavedLevel {
                schema: 2,
                owner: old.owner,
                source: old.source,
                barrier_epoch: 1,
                retired_generation: None,
                retired_members: None,
                binding: Some(SavedBinding {
                    destination: old.destination,
                    generation: old.generation,
                    pose_revision: old.pose_revision,
                    membership: 0,
                    pose: pose.into(),
                    cells,
                    limits: limits.into(),
                    stream_areas: areas
                        .iter()
                        .map(|a| [a.center.0, a.center.1, a.radius as i32])
                        .collect(),
                }),
            };
            metadata.checked(owner, source.dimension_id())?;
            (metadata, true)
        } else {
            return Err(Error::Unsupported);
        };
        Ok((metadata, legacy, bytes))
    }
    pub(super) fn saved_projection_descriptor(
        &self,
        source: LevelRef,
    ) -> Result<mod_api::LiveWorldSavedProjection, Error> {
        let (metadata, legacy, _) = self.decode_saved_projection(source)?;
        let binding = metadata.binding.ok_or(Error::Stale)?;
        let limits = binding.limits.decode()?;
        let footprint = retention::selection_footprint(source, &binding.cells)?;
        let columns: BTreeSet<_> = binding
            .cells
            .iter()
            .map(|p| [p[0] >> 4, p[2] >> 4])
            .collect();
        Ok(mod_api::LiveWorldSavedProjection::new(
            binding.destination,
            columns.into_iter().collect(),
            binding.cells.len() as u32,
            footprint.retained_columns,
            limits,
            if legacy { 0 } else { metadata.barrier_epoch },
            if legacy {
                mod_api::LiveWorldRecovery::ExperimentalMigrationRequired
            } else {
                mod_api::LiveWorldRecovery::Ready
            },
        ))
    }
    pub(super) fn restore_saved_projection(
        &mut self,
        source: LevelRef,
    ) -> Result<LiveWorldRef, Error> {
        if self.level_live_world(source)?.is_some() {
            return Err(Error::AlreadyBound);
        }
        let (metadata, legacy, _) = self.decode_saved_projection(source)?;
        if legacy {
            return Err(Error::MigrationRequired);
        }
        let binding = metadata.binding.ok_or(Error::Stale)?;
        let destination = self.level(&binding.destination).ok_or(Error::NotResident)?;
        let limits = binding.limits.decode()?;
        let pose = binding.pose.decode()?;
        let areas = binding
            .stream_areas
            .iter()
            .map(|p| sunrise_client::lan::viewed_levels::areas::ChunkArea {
                level: source.dimension_id().to_owned(),
                center: (p[0], p[1]),
                radius: p[2] as u32,
                mode: sunrise_client::lan::viewed_levels::areas::ChunkAreaMode::Loaded,
            })
            .collect();
        {
            let table = self.live_worlds.get_mut();
            table.next = table.next.max(binding.generation);
        }
        let handle = self.project_cells_with_cover(
            source,
            destination,
            &binding.cells,
            pose,
            limits,
            Some(areas),
        )?;
        let mut table = self.live_worlds.borrow_mut();
        let entry = table
            .entries
            .get_mut(&handle.generation())
            .ok_or(Error::Stale)?;
        // The accepted public sample is bit-preserving. Derived collision normalization never
        // replaces it, including repeated nonidentity save/restore cycles.
        entry.sample = LiveWorldSample::new(pose, pose, binding.pose_revision);
        entry.membership = binding.membership;
        Ok(handle)
    }
}
