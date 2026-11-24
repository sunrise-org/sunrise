//! One bounded save decision over real current/parked World, Streaming and SaveDir owners.
use super::*;
use crate::game::Dim;
use std::time::{Duration, Instant};
use sunrise_client::save::{FrozenLevelSave, SaveDir, live_world_journal as wal};
use sunrise_core::{
    nbt::{Compound, NamedTag, Tag},
    position::ChunkPos,
    ticks::SavedTick,
};

struct Readset {
    level: LevelRef,
    revision: u64,
    content: u64,
    time: i64,
    attachments: Compound,
    chunks: Vec<((i32, i32), usize)>,
    schedules: Vec<(
        (i32, i32),
        Vec<SavedTick>,
        Vec<SavedTick>,
        Option<u64>,
        Option<u64>,
    )>,
    projection: Option<Vec<u8>>,
}
struct Captured {
    readset: Readset,
    save: Arc<SaveDir>,
    frozen: FrozenLevelSave,
    entities: Vec<((i32, i32), Vec<Tag>)>,
    terrain_columns: BTreeSet<(i32, i32)>,
    modified: BTreeSet<(i32, i32)>,
}
struct PlayerCapture {
    frozen: sunrise_client::lan::playerdata::FrozenPlayerSaves,
    images: Vec<wal::Image>,
    names: BTreeMap<u128, String>,
    live: BTreeMap<u128, Tag>,
    local: Option<Tag>,
    owner_attachments: Compound,
    profile: Option<[i32; 4]>,
}
impl Game<'_> {
    fn checkpoint_storage(context: &str, error: impl std::fmt::Display) -> Error {
        log::error!("live-world checkpoint {context}: {error}");
        #[cfg(test)]
        eprintln!("LIVE_WORLD_STORAGE {context}: {error}");
        Error::Storage
    }
    fn checkpoint_readset(&self, level: LevelRef, epoch: u64) -> Result<Readset, Error> {
        if !self.level_handle_current(level) {
            return Err(Error::Stale);
        }
        let (world, ticks) = if level.dimension() == self.dim.key() {
            (&self.world, &self.block_ticks)
        } else {
            self.levels
                .parked_transfer_parts(level.dimension_id())
                .ok_or(Error::NotResident)?
        };
        let mut chunks: Vec<_> = world
            .chunks
            .iter()
            .map(|(key, chunk)| (*key, Arc::as_ptr(chunk) as usize))
            .collect();
        chunks.sort_unstable();
        let schedules = chunks
            .iter()
            .map(|(key, _)| {
                let chunk = ChunkPos::new(key.0, key.1);
                (
                    *key,
                    ticks
                        .blocks
                        .chunk(chunk)
                        .map_or_else(Vec::new, |ticks| ticks.pack(world.game_time)),
                    ticks
                        .fluids
                        .chunk(chunk)
                        .map_or_else(Vec::new, |ticks| ticks.pack(world.game_time)),
                    ticks
                        .blocks
                        .chunk(chunk)
                        .map(|ticks| ticks.persistence_revision()),
                    ticks
                        .fluids
                        .chunk(chunk)
                        .map(|ticks| ticks.persistence_revision()),
                )
            })
            .collect();
        Ok(Readset {
            level,
            revision: world.revision,
            content: world.block_entities.content_revision(),
            time: world.game_time,
            attachments: world
                .attachments
                .try_to_nbt()
                .map_err(|error| Self::checkpoint_storage("level attachments", error))?,
            chunks,
            schedules,
            projection: self.projection_save_image(level, epoch)?,
        })
    }
    fn validate_checkpoint_readset(&self, before: &Readset, epoch: u64) -> Result<(), Error> {
        let now = self.checkpoint_readset(before.level, epoch)?;
        if now.revision != before.revision
            || now.content != before.content
            || now.time != before.time
            || now.attachments != before.attachments
            || now.chunks != before.chunks
            || now.schedules != before.schedules
            || now.projection != before.projection
        {
            return Err(Error::Revision);
        }
        Ok(())
    }
    fn quiesce_live_world_save(&mut self) -> Result<(), Error> {
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            match self.streaming.prepare_save(&mut self.world, true, deadline) {
                sunrise_client::streaming::SaveStep::Full(chunk) => {
                    self.save_late_full_chunk(chunk);
                }
                sunrise_client::streaming::SaveStep::Protos(protos) => {
                    self.streaming.finish_save(protos, true);
                    break;
                }
                sunrise_client::streaming::SaveStep::Pending => return Err(Error::Busy),
            }
        }
        self.spawn_loaded_entities();
        self.eject_player_riders(None);
        Ok(())
    }

    /// Ordinary terrain dirtying already includes generation, edits, light and raw extra
    /// changes. Be conservative for live/opaque BE data, attachments and scheduled work,
    /// and always include active membership and prepared-transfer participants. Entity/POI
    /// columns and deferred records are captured separately, never filtered by this set.
    fn checkpoint_terrain_columns(
        &self,
        level: LevelRef,
        chunks: &[Arc<sunrise_client::chunk::Chunk>],
        readset: &Readset,
        transfer: Option<&super::transfer_cells::PreparedCells>,
    ) -> BTreeSet<(i32, i32)> {
        let mut selected: BTreeSet<_> = self.world.modified.iter().copied().collect();
        selected.extend(
            self.world
                .block_entities
                .keys()
                .map(|p| (p.0 >> 4, p.2 >> 4)),
        );
        selected.extend(
            readset
                .schedules
                .iter()
                .filter(|(key, blocks, fluids, _, _)| {
                    let pos = ChunkPos::new(key.0, key.1);
                    !blocks.is_empty()
                        || !fluids.is_empty()
                        || self
                            .block_ticks
                            .blocks
                            .chunk(pos)
                            .is_some_and(|ticks| ticks.needs_persistent_save())
                        || self
                            .block_ticks
                            .fluids
                            .chunk(pos)
                            .is_some_and(|ticks| ticks.needs_persistent_save())
                })
                .map(|(key, ..)| *key),
        );
        selected.extend(
            self.block_ticks
                .comparator_outputs
                .keys()
                .map(|p| (p.x >> 4, p.z >> 4)),
        );
        selected.extend(
            self.block_ticks
                .moving_pistons
                .iter()
                .map(|(p, _)| (p.x >> 4, p.z >> 4)),
        );
        for chunk in chunks {
            let owned_data = !chunk.block_entities.is_empty()
                || chunk.saved.as_ref().is_some_and(|saved| {
                    !saved.block_entities.is_empty()
                        || !saved.block_entity_sources.is_empty()
                        || !saved.block_entity_attachments.is_empty()
                        || saved
                            .block_entity_save_edits
                            .as_ref()
                            .is_some_and(|edits| !edits.is_empty())
                        || !saved.pending_loot.is_empty()
                        || !saved.block_ticks.is_empty()
                        || !saved.fluid_ticks.is_empty()
                        || !saved.attachments.is_empty()
                });
            if owned_data {
                selected.insert((chunk.x, chunk.z));
            }
        }
        for entry in self.live_worlds.borrow().entries.values() {
            if entry.handle.level() == level {
                selected.extend(entry.cells.iter().map(|p| (p[0] >> 4, p[2] >> 4)));
            }
        }
        if let Some(columns) = transfer.and_then(|plan| plan.affected_columns().get(&level)) {
            selected.extend(columns.iter().copied());
        }
        selected.retain(|key| self.world.chunks.contains_key(key));
        selected
    }

    fn validate_captured_dirty(&self, captured: &Captured) -> Result<(), Error> {
        let world = self
            .native_level_world(captured.readset.level)
            .ok_or(Error::Stale)?;
        if world.modified.iter().copied().collect::<BTreeSet<_>>() != captured.modified {
            return Err(Error::Revision);
        }
        Ok(())
    }
    pub(in crate::game) fn live_world_can_retire_streaming(&self, idle_sweep: bool) -> bool {
        (self.live_world_durability.retiring_streams || idle_sweep)
            && self.live_world_commit_frontier()
            && !self.live_world_checkpoint_running()
            && !self.has_open_transaction()
    }

    /// Retire one held Streaming plan at the same synchronous frontier that saved it. A callback
    /// changing any live readset or the plan's residency inputs refuses retirement, not the save.
    pub(in crate::game) fn checkpoint_live_world_streaming(
        &mut self,
        plan: sunrise_client::streaming::StreamingUpdate,
        idle_sweep: bool,
    ) -> Result<
        (
            sunrise_client::streaming::StreamingCommit,
            Vec<sunrise_client::streaming::ProtoSave>,
        ),
        sunrise_client::streaming::StreamingUpdate,
    > {
        if !self.live_world_can_retire_streaming(idle_sweep) {
            return Err(plan);
        }
        let Some(level) = self.level(self.dim.id()) else {
            return Err(plan);
        };
        let Ok(epoch) = wal::next_epoch(&self.world_root) else {
            return Err(plan);
        };
        let Ok(before) = self.checkpoint_readset(level, epoch) else {
            return Err(plan);
        };
        let forced = self.streaming.forced.clone();
        let extra = self.streaming.extra.clone();
        let view = (
            self.streaming.center,
            self.streaming.radius,
            self.streaming.paused,
        );
        if self.checkpoint_live_world_levels(&[]).is_err()
            || self.validate_checkpoint_readset(&before, epoch).is_err()
            || self.streaming.forced != forced
            || self.streaming.extra != extra
            || (
                self.streaming.center,
                self.streaming.radius,
                self.streaming.paused,
            ) != view
        {
            return Err(plan);
        }
        // No callback, Done drain or tick occurs between this validation and owner removal.
        let revision = self.world.revision;
        let content = self.world.block_entities.content_revision();
        let attachments = match self.world.attachments.try_to_nbt() {
            Ok(attachments) => attachments,
            Err(error) => {
                Self::checkpoint_storage("streaming level attachments", error);
                return Err(plan);
            }
        };
        let columns: BTreeMap<_, _> = self
            .world
            .chunks
            .iter()
            .map(|(key, chunk)| (*key, Arc::as_ptr(chunk) as usize))
            .collect();
        self.streaming
            .prepare_checkpointed_commit(&mut self.world, plan, |world| {
                world.revision == revision
                    && world.block_entities.content_revision() == content
                    && world
                        .attachments
                        .try_to_nbt()
                        .is_ok_and(|current| current == attachments)
                    && world.chunks.len() == columns.len()
                    && world.chunks.iter().all(|(key, chunk)| {
                        columns.get(key) == Some(&(Arc::as_ptr(chunk) as usize))
                    })
            })
    }
    fn live_checkpoint_player_tags(&self) -> Result<BTreeMap<u128, Tag>, Error> {
        let Some(host) = &self.lan else {
            return Ok(BTreeMap::new());
        };
        host.hub
            .guests
            .iter()
            .filter(|g| {
                g.placed
                    && g.participates_in_player_simulation()
                    && !g.released
                    && !self.crash_save.excludes(g.conn)
            })
            .map(|g| {
                sunrise_client::lan::playerdata::try_player_tag(g, self.res.items())
                    .map(|tag| (g.uuid, tag))
                    .map_err(|error| Self::checkpoint_storage("guest attachments", error))
            })
            .collect()
    }
    fn checkpoint_players_settled(&self) -> bool {
        self.carried.is_none()
            && self.craft_grid.iter().all(Option::is_none)
            && self.lan.as_ref().is_none_or(|host| {
                host.hub.guests.iter().all(|g| {
                    g.carried.is_none()
                        && g.craft_grid.iter().all(Option::is_none)
                        && g.native_menu.iter().all(Option::is_none)
                })
            })
    }
    fn validate_checkpoint_profile(&self, profile: Option<[i32; 4]>) -> Result<(), Error> {
        if self.level.player_uuid != profile {
            return Err(Error::Revision);
        }
        if let Some(owner) = self.lan.as_ref().and_then(|host| host.owner.as_ref())
            && profile != Some(sunrise_client::lan::playerdata::uuid_ints(owner.uuid))
        {
            return Err(Error::Revision);
        }
        Ok(())
    }
    fn checkpoint_players(&mut self) -> Result<PlayerCapture, Error> {
        if !self.checkpoint_players_settled() {
            return Err(Error::Busy);
        }
        // Old independent completions were drained/joined before common admission. Their already
        // durable receipts may fire now; none can run inside the subsequent Prepared interval.
        let _ = self
            .finish_player_autosaves(true)
            .map_err(|error| Self::checkpoint_storage("capture", error))?;
        if self.player.is_some() && self.lan.is_none() {
            // Startup migration must already have selected the real profile written in level.dat.
            // A common decision cannot choose a new owner-file identity behind that metadata.
            if self.level.player_uuid.is_none() {
                return Err(Error::Unavailable);
            }
        }
        self.validate_checkpoint_profile(self.level.player_uuid)?;
        self.sync_level();
        let owner_expected = self.lan.as_ref().is_some_and(|host| {
            host.owner.as_ref().is_some_and(|owner| {
                host.hub.online_players().any(|guest| {
                    guest.uuid == owner.uuid
                        && !guest.released
                        && !self.crash_save.excludes(guest.conn)
                })
            })
        });
        let owner = self.store_owner();
        if owner_expected && owner.is_none() {
            return Err(Error::Storage);
        }
        let profile = self.level.player_uuid;
        self.validate_checkpoint_profile(profile)?;
        let frozen = self
            .player_autosaves
            .freeze_common()
            .map_err(|error| Self::checkpoint_storage("capture", error))?;
        if frozen
            .world_root()
            .is_some_and(|root| root != self.world_root)
        {
            return Err(Error::Stale);
        }
        let mut chosen: BTreeMap<_, _> = frozen
            .snapshots()
            .map(|(uuid, name, tag)| (uuid, (name.to_owned(), tag.clone())))
            .collect();
        let live = self.live_checkpoint_player_tags()?;
        if let Some(host) = self.lan.as_ref() {
            for g in host
                .hub
                .guests
                .iter()
                .filter(|g| live.contains_key(&g.uuid))
            {
                chosen.insert(g.uuid, (g.name.clone(), live[&g.uuid].clone()));
            }
        }
        if let Some((uuid, name)) = owner {
            chosen.insert(uuid, (name, self.level.player_tag(self.res.items())));
        }
        let local = if self.player.is_some() && self.lan.is_none() {
            let uuid =
                sunrise_client::lan::playerdata::ints_uuid(profile.ok_or(Error::Unavailable)?);
            let tag = self.level.player_tag(self.res.items());
            chosen.insert(uuid, (self.options.player_name(), tag.clone()));
            Some(tag)
        } else {
            None
        };
        let mut images = Vec::with_capacity(chosen.len());
        let mut names = BTreeMap::new();
        for (uuid, (name, tag)) in chosen {
            let raw = sunrise_core::nbt::write_named(
                &NamedTag {
                    name: "".into(),
                    tag,
                },
                sunrise_core::nbt::Limits::saved(),
            )
            .map_err(|error| Self::checkpoint_storage("capture", error))?;
            let after = sunrise_world::compression::Compression::Gzip
                .encode(&raw)
                .map_err(|error| Self::checkpoint_storage("capture", error))?;
            images.push(wal::Image {
                key: wal::Key::PlayerData { uuid },
                after: Some(after),
            });
            names.insert(uuid, name);
        }
        Ok(PlayerCapture {
            frozen,
            images,
            names,
            live,
            local,
            profile,
            owner_attachments: self
                .player_state
                .attachments
                .try_to_nbt()
                .map_err(|error| Self::checkpoint_storage("owner attachments", error))?,
        })
    }
    fn validate_checkpoint_players(&mut self, players: &PlayerCapture) -> Result<(), Error> {
        self.validate_checkpoint_profile(players.profile)?;
        if !self.checkpoint_players_settled()
            || self.live_checkpoint_player_tags()? != players.live
            || self
                .player_state
                .attachments
                .try_to_nbt()
                .map_err(|error| Self::checkpoint_storage("owner attachments", error))?
                != players.owner_attachments
        {
            return Err(Error::Revision);
        }
        if let Some(before) = &players.local {
            self.sync_level();
            if self.level.player_tag(self.res.items()) != *before {
                return Err(Error::Revision);
            }
        }
        self.player_autosaves
            .validate_common(&players.frozen)
            .map_err(|_| Error::Revision)
    }
    /// Synchronous common barrier. The server frontier prevents any level ticker or guest turn
    /// from running between capture and acknowledgment. All callbacks finish before Prepared;
    /// the authorized current writer receives only frozen Tag/Delete records afterwards.
    pub(in crate::game) fn checkpoint_live_world_levels(
        &mut self,
        extra: &[LevelRef],
    ) -> Result<mod_api::LiveWorldSaveReceipt, Error> {
        self.checkpoint_live_world_levels_at_frontier(extra, false)
    }
    /// Final teardown has no next tick to save writes made by post-acknowledgment notices.
    /// A changed closing frontier refuses and fences instead of reporting a clean shutdown.
    pub(in crate::game) fn checkpoint_live_world_shutdown(
        &mut self,
    ) -> Result<mod_api::LiveWorldSaveReceipt, Error> {
        self.checkpoint_live_world_levels_at_frontier(&[], true)
    }
    fn checkpoint_live_world_levels_at_frontier(
        &mut self,
        extra: &[LevelRef],
        final_save: bool,
    ) -> Result<mod_api::LiveWorldSaveReceipt, Error> {
        if self.live_world_session_failed() {
            return Err(Error::Storage);
        }
        if !self.live_world_commit_frontier()
            || !self.checkpoint_players_settled()
            || self.live_world_checkpoint_running()
            || self.transaction_lifecycle() != mod_api::TransactionLifecycle::None
            || !sunrise_client::join_seq::visibility::passes_settled()
        {
            return Err(Error::Busy);
        }
        if self.session_lock.is_none() {
            return Err(Error::Unavailable);
        }
        self.live_world_durability.saving = true;
        let result = self
            .capture_live_world_checkpoint(extra, None, final_save)
            .map(|(receipt, _)| receipt);
        self.live_world_durability.saving = false;
        result
    }
    pub(super) fn checkpoint_live_world_transfer(
        &mut self,
        plan: super::transfer_cells::PreparedCells,
        cells: &[mod_api::LiveWorldTransferCell],
    ) -> Result<mod_api::LiveWorldTransferReceipt, Error> {
        if !self.live_world_commit_frontier()
            || self.live_world_checkpoint_running()
            || self.has_open_transaction()
            || !self.checkpoint_players_settled()
            || !sunrise_client::join_seq::visibility::passes_settled()
        {
            return Err(Error::Busy);
        }
        if self.session_lock.is_none() {
            return Err(Error::Unavailable);
        }
        let levels: Vec<_> = plan.affected_columns().keys().copied().collect();
        self.live_world_durability.saving = true;
        let result = self.capture_live_world_checkpoint(&levels, Some((plan, cells)), false);
        self.live_world_durability.saving = false;
        result.map(|(saved, retired)| {
            mod_api::LiveWorldTransferReceipt::new(
                saved.barrier_epoch,
                cells.len() as u32,
                saved.levels,
                retired,
            )
        })
    }
    fn capture_live_world_checkpoint(
        &mut self,
        extra: &[LevelRef],
        mut transfer: Option<(
            super::transfer_cells::PreparedCells,
            &[mod_api::LiveWorldTransferCell],
        )>,
        final_save: bool,
    ) -> Result<(mod_api::LiveWorldSaveReceipt, Vec<LiveWorldRef>), Error> {
        self.live_world_durability.last_capture = None;
        for &level in extra {
            self.hold_live_world_level(level)?;
        }
        let levels: Vec<_> = self.live_world_durability.members.keys().copied().collect();
        let coverage = self.live_world_durability.coverage_epoch;
        let root = self.world_root.clone();
        let mut limits = wal::Limits::default();
        if let Some((plan, _)) = &transfer {
            limits.bytes = limits.bytes.min(plan.maximum_bytes());
            limits.record_bytes = u32::try_from(limits.bytes.min(u64::from(limits.record_bytes)))
                .map_err(|_| Error::Budget)?;
        }
        let writes = self.transactions.writes();
        let epoch =
            wal::next_epoch(&root).map_err(|error| Self::checkpoint_storage("capture", error))?;
        let back = self.dim;
        let mut captured = Vec::with_capacity(levels.len());
        let mut player_capture = None;
        let capture = (|| {
            for &level in &levels {
                if !self.level_handle_current(level) {
                    return Err(Error::Stale);
                }
                self.enter_level(Dim::from_id(level.dimension_id()))
                    .map_err(|_| Error::NotResident)?;
                self.quiesce_live_world_save()?;
            }
            if self.dim != back {
                self.enter_level(back)
                    .map_err(|error| Self::checkpoint_storage("capture", error))?;
            }
            let players = self.checkpoint_players()?;
            let mut images = players.images.clone();
            player_capture = Some(players);
            let mut snapshots = BTreeMap::new();
            let mut snapshot_readsets = BTreeMap::new();
            for &level in &levels {
                self.enter_level(Dim::from_id(level.dimension_id()))
                    .map_err(|_| Error::NotResident)?;
                let readset = self.checkpoint_readset(level, epoch)?;
                let chunks: Vec<_> = self.world.chunks.values().cloned().collect();
                let terrain_columns = self.checkpoint_terrain_columns(
                    level,
                    &chunks,
                    &readset,
                    transfer.as_ref().map(|(plan, _)| plan),
                );
                let modified: BTreeSet<_> = self.world.modified.iter().copied().collect();
                // Clean ordinary terrain keeps its existing immutable column. Only real save
                // participants clone live BE/schedule/opaque snapshots; POI/entities still see
                // every resident column below.
                let selected = self.with_entities(
                    chunks
                        .iter()
                        .filter(|chunk| terrain_columns.contains(&(chunk.x, chunk.z))),
                );
                let mut selected: BTreeMap<_, _> = selected
                    .into_iter()
                    .map(|chunk| ((chunk.x, chunk.z), chunk))
                    .collect();
                snapshots.insert(
                    level,
                    chunks
                        .into_iter()
                        .map(|chunk| selected.remove(&(chunk.x, chunk.z)).unwrap_or(chunk))
                        .collect(),
                );
                snapshot_readsets.insert(level, (readset, terrain_columns, modified));
            }
            if let Some((plan, _)) = &transfer {
                self.transfer_cell_afterimages(plan, &mut snapshots)?;
            }
            for &level in &levels {
                self.enter_level(Dim::from_id(level.dimension_id()))
                    .map_err(|_| Error::NotResident)?;
                let (readset, terrain_columns, modified) =
                    snapshot_readsets.remove(&level).ok_or(Error::Storage)?;
                if readset.chunks.len() > limits.records as usize {
                    return Err(Error::Budget);
                }
                let keys = readset.chunks.iter().map(|(key, _)| *key).collect();
                let chunks = snapshots.remove(&level).ok_or(Error::Storage)?;
                let mut entities = self.entity_chunks(&keys, false);
                entities.sort_by_key(|(key, _)| *key);
                self.validate_checkpoint_readset(&readset, epoch)?;
                let save = self.save.clone();
                if save.root != sunrise_client::save::dimension_dir(&root, level.dimension_id()) {
                    return Err(Error::Stale);
                }
                let frozen = save
                    .freeze_live_world_save(
                        level.dimension_id(),
                        &chunks,
                        &terrain_columns,
                        &entities,
                        self.pack.assets.registry,
                        world_biomes!(self),
                        &self.world.attachments,
                        if let Some((_, cells)) = &transfer {
                            self.projection_transfer_afterimage(level, epoch, cells)?
                        } else {
                            readset.projection.clone()
                        },
                        limits,
                    )
                    .map_err(|error| {
                        if let Some(budget) = error.downcast_ref::<wal::PayloadBudgetExceeded>() {
                            self.live_world_durability.last_capture =
                                Some(super::durability::CaptureUsage {
                                    epoch,
                                    usage: None,
                                    payload_bound: Some((
                                        budget.required_at_least,
                                        budget.records_encoded,
                                    )),
                                    limit: budget.limit,
                                    fits_byte_budget: false,
                                });
                            log::warn!("live-world checkpoint capture: {error}");
                            Error::Budget
                        } else {
                            Self::checkpoint_storage("capture", error)
                        }
                    })?;
                images.extend(frozen.images.iter().cloned());
                captured.push(Captured {
                    readset,
                    save,
                    frozen,
                    entities,
                    terrain_columns,
                    modified,
                });
            }
            // Re-read the actual ordinary EntityStorage projection after all save callbacks,
            // before Prepared. No snapshot callback is replayed and no live entity owner is removed.
            for member in &captured {
                self.enter_level(Dim::from_id(member.readset.level.dimension_id()))
                    .map_err(|_| Error::NotResident)?;
                let keys = member.readset.chunks.iter().map(|(key, _)| *key).collect();
                let mut current = self.entity_chunks(&keys, false);
                current.sort_by_key(|(key, _)| *key);
                if current != member.entities {
                    return Err(Error::Revision);
                }
            }
            if self.dim != back {
                self.enter_level(back)
                    .map_err(|error| Self::checkpoint_storage("capture", error))?;
            }
            self.validate_checkpoint_players(player_capture.as_ref().ok_or(Error::Storage)?)?;
            if self.live_world_durability.coverage_epoch != coverage
                || self.transactions.writes() != writes
            {
                return Err(Error::Revision);
            }
            if let Some((plan, _)) = &transfer {
                self.validate_transfer_cells(plan)?;
            }
            if let Some((source, original, _)) = &self.live_world_durability.migration {
                images.push(wal::Image {
                    key: wal::Key::ProjectionOriginal {
                        level: source.dimension_id().to_owned(),
                    },
                    after: Some(original.clone()),
                });
            }
            images.push(wal::checkpoint_image(epoch));
            let prepared = match wal::Prepared::capture(&root, epoch, images, limits) {
                Ok(prepared) => prepared,
                Err(error) => {
                    if let Some(budget) = wal::byte_budget_error(&error) {
                        self.live_world_durability.last_capture =
                            Some(super::durability::CaptureUsage {
                                epoch,
                                usage: Some(budget.usage),
                                payload_bound: None,
                                limit: budget.limit,
                                fits_byte_budget: false,
                            });
                        log::warn!("live-world checkpoint capture: {error}");
                        return Err(Error::Budget);
                    }
                    return Err(Self::checkpoint_storage("capture", error));
                }
            };
            self.live_world_durability.last_capture = Some(super::durability::CaptureUsage {
                epoch,
                usage: Some(
                    prepared
                        .encoded_usage()
                        .map_err(|error| Self::checkpoint_storage("capture size", error))?,
                ),
                payload_bound: None,
                limit: limits.bytes,
                fits_byte_budget: true,
            });
            for member in &captured {
                self.validate_checkpoint_readset(&member.readset, epoch)?;
                self.validate_captured_dirty(member)?;
                member
                    .save
                    .validate_frozen_live_world_save(&member.frozen)
                    .map_err(|_| Error::Revision)?;
            }
            Ok(prepared)
        })();
        if self.dim != back {
            self.enter_level(back)
                .map_err(|error| Self::checkpoint_storage("capture", error))?;
        }
        let prepared = capture?;
        let players = player_capture.ok_or(Error::Storage)?;
        // A publication failure may occur after rename/fsync: every later engine write is fenced.
        let mut retired_bindings = Vec::new();
        let write = (|| -> sunrise_client::Result<()> {
            prepared.publish(&root)?;
            for member in &captured {
                member
                    .save
                    .mark_live_world_prepared(&root, &prepared, &member.frozen)?;
            }
            for member in &captured {
                member.save.write_frozen_live_world_save(
                    member.readset.level.dimension_id(),
                    &member.frozen,
                )?;
            }
            prepared.write_projection_originals(&root)?;
            prepared.write_player_data(&root)?;
            prepared.write_checkpoint_epoch(&root)?;
            for member in &captured {
                self.validate_checkpoint_readset(&member.readset, epoch)
                    .map_err(|error| {
                        format!("live-world readset changed after Prepared: {error:?}")
                    })?;
                self.validate_captured_dirty(member).map_err(|error| {
                    format!("live-world dirty selection changed after Prepared: {error:?}")
                })?;
                member.save.ready_frozen_live_world_ack(&member.frozen)?;
            }
            self.validate_checkpoint_players(&players)
                .map_err(|error| format!("player readset changed after Prepared: {error:?}"))?;
            if self.live_world_durability.coverage_epoch != coverage {
                return Err("common coverage changed after Prepared".into());
            }
            if let Some((plan, cells)) = transfer.take() {
                self.apply_prepared_transfer_cells(plan)
                    .map_err(|error| format!("prepared live handoff refused: {error:?}"))?;
                retired_bindings = self
                    .apply_transfer_membership(cells)
                    .map_err(|error| format!("prepared membership handoff refused: {error:?}"))?;
            }
            prepared.acknowledge(&root)?;
            for member in &mut captured {
                member.frozen.acknowledge()?;
            }
            self.player_autosaves.acknowledge_common(&players.frozen)?;
            Ok(())
        })();
        if let Err(error) = write {
            Self::checkpoint_storage("publish/write/ack", error);
            self.fence_live_world_session();
            return Err(Error::Storage);
        }
        self.live_world_durability.last_barrier_epoch = epoch;
        self.live_world_durability.saved_owner_profile = players.profile;
        let mut receipts = Vec::with_capacity(captured.len());
        for member in captured {
            let level = member.readset.level;
            let binding = self
                .live_worlds
                .borrow()
                .entries
                .values()
                .find(|e| e.handle.level() == level)
                .map(|e| e.handle);
            let actual = self.native_level_world(level).ok_or(Error::Stale)?;
            let revision = actual.revision;
            let content = actual.block_entities.content_revision();
            let retired = self.live_world_durability.retired.get(&level);
            receipts.push(mod_api::LiveWorldLevelReceipt::new(
                level,
                revision,
                content,
                binding,
                retired.map(|r| r.generation),
                retired
                    .filter(|r| r.source_revision == revision)
                    .map(|r| r.members),
            ));
            self.live_world_durability.dirty.remove(&level);
            // Only the exact real queues represented by an acknowledged fresh terrain image
            // become clean. Transfer/live mutations after capture conservatively stay dirty.
            let ticks = if level.dimension() == self.dim.key() {
                &mut self.block_ticks
            } else {
                self.levels
                    .parked_transfer_parts_mut(level.dimension_id())
                    .ok_or(Error::Stale)?
                    .1
            };
            for (key, _, _, blocks, fluids) in &member.readset.schedules {
                if !member.terrain_columns.contains(key) {
                    continue;
                }
                let pos = ChunkPos::new(key.0, key.1);
                if let Some(revision) = blocks {
                    ticks.blocks.acknowledge_persistent_save(pos, *revision);
                }
                if let Some(revision) = fluids {
                    ticks.fluids.acknowledge_persistent_save(pos, *revision);
                }
            }
            if level.dimension() == self.dim.key() {
                for key in &member.terrain_columns {
                    self.world.modified.remove(key);
                }
            } else if let Some(world) = self.levels.parked_world_mut_of(level.dimension_id()) {
                for key in &member.terrain_columns {
                    world.modified.remove(key);
                }
            }
        }
        // Only final teardown needs a post-notice cut: ordinary saves can schedule another
        // checkpoint on the next tick. Keep this bounded witness temporary, never pinning chunks.
        let final_readsets = if final_save {
            let captured = receipts
                .iter()
                .map(|row| self.checkpoint_readset(row.level, epoch))
                .collect::<Result<Vec<_>, _>>();
            match captured {
                Ok(captured) => Some((
                    captured,
                    self.transactions.writes(),
                    self.player_autosaves.deferred_generation(),
                    self.live_world_durability.coverage_epoch,
                )),
                Err(error) => {
                    self.fence_live_world_session();
                    return Err(error);
                }
            }
        } else {
            None
        };
        // SaveToFile notices belong after the common decision, once per actual final UUID.
        let directory = sunrise_client::lan::playerdata::directory(&root);
        for (uuid, name) in players.names {
            self.native_player_save_to_file(uuid, &name, &directory);
        }
        if let Some((readsets, writes, players, coverage)) = final_readsets {
            let settled = (|| -> Result<(), Error> {
                if self.transactions.writes() != writes
                    || self.player_autosaves.deferred_generation() != players
                    || self.live_world_durability.coverage_epoch != coverage
                    || !self
                        .player_autosaves
                        .writes_finished()
                        .map_err(|error| Self::checkpoint_storage("final player cut", error))?
                {
                    return Err(Error::Revision);
                }
                for before in &readsets {
                    self.validate_checkpoint_readset(before, epoch)?;
                }
                if self
                    .live_world_durability
                    .members
                    .values()
                    .any(|member| member.save.live_world_barrier_pending())
                {
                    return Err(Error::Revision);
                }
                Ok(())
            })();
            if let Err(error) = settled {
                self.fence_live_world_session();
                return Err(error);
            }
        }
        Ok((
            mod_api::LiveWorldSaveReceipt::new(epoch, receipts),
            retired_bindings,
        ))
    }
}
