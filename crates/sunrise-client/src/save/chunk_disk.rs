//! The chunk writer's isolated mod attachment callback boundary.
//! Server code captures encoded envelopes; only workers execute mod code.
use super::*;
use mod_api::{ChunkDiskContextWire, ChunkDiskJob, ChunkDiskKind, ChunkDiskPhase, WorldEpoch};
use sunrise_mod_runtime::attachments::Attachments;

pub(super) struct DiskRuntime {
    /// Chunks whose load listener failed: their stored record is kept as it was read and no
    /// save listener runs for them for the rest of the session.
    save_suppressed: Mutex<HashSet<(i32, i32)>>,
    /// Chunks whose captured save did not reach disk; the game marks them dirty again.
    failed_saves: Mutex<Vec<(i32, i32)>>,
    world: WorldEpoch,
    dimension: mod_api::Identifier,
    resources: Arc<sunrise_world::native_load_context::WorldgenContext>,
}

impl DiskRuntime {
    fn capture(
        &self,
        chunk: &Chunk,
        kind: ChunkDiskKind,
        phase: ChunkDiskPhase,
    ) -> Option<ChunkDiskJob> {
        let context = ChunkDiskContextWire::new(
            self.world,
            self.dimension.clone(),
            [chunk.x, chunk.z],
            kind,
            phase,
        );
        let empty = Attachments::default();
        ChunkDiskJob::capture(
            &context,
            chunk
                .saved
                .as_ref()
                .map_or(&empty, |saved| &saved.attachments),
        )
    }

    fn run(&self, job: &ChunkDiskJob) -> std::io::Result<Compound> {
        let registry = |name: &str| self.resources.registry(name);
        let setting = |owner: &str, key: &str| self.resources.setting(owner, key);
        // A disk job has no block-state index (that is the game's, published for worldgen), so
        // `state_with_property` answers None, as worldgen does before the index is published.
        let state_with_property = |_: mod_api::BlockState, _: &str, _: &str| None;
        job.run(&mod_api::OwningWorldgen::new(
            &registry,
            &setting,
            &state_with_property,
        ))
        .clone()
        .map_err(|failure| {
            std::io::Error::other(format!("chunk disk callback refused: {failure:?}"))
        })
    }
}

pub(super) struct DiskRecord {
    pub(super) original: Record,
    job: ChunkDiskJob,
    runtime: Arc<DiskRuntime>,
    encoded: OnceLock<std::result::Result<Option<Arc<NamedTag>>, String>>,
}
impl DiskRecord {
    fn encode(&self) -> std::io::Result<Option<Arc<NamedTag>>> {
        self.encoded
            .get_or_init(|| {
                let entries = self.runtime.run(&self.job).map_err(|e| e.to_string())?;
                let mut tag = self
                    .original
                    .tag_for_write()
                    .ok_or("captured chunk has no disk record")?;
                replace_entries(Arc::make_mut(&mut tag), entries).map_err(|e| e.to_string())?;
                Ok(Some(tag))
            })
            .clone()
            .map_err(std::io::Error::other)
    }

    pub(super) fn without_block_entity_edits(&self) -> Option<Arc<NamedTag>> {
        let mut tag = self.original.without_block_entity_edits()?;
        // The original encoding has already run the chunk-data callback.
        // Replay only its result, not the callback or its other writes.
        if let Some(Ok(Some(encoded))) = self.encoded.get() {
            let entries = match encoded.tag.get(ATTACHMENTS_KEY) {
                Some(Tag::Compound(entries)) => entries.clone(),
                _ => Compound::new(),
            };
            replace_entries(Arc::make_mut(&mut tag), entries).ok()?;
        }
        Some(tag)
    }

    // Readers may wait for the owning writer, but never execute save callbacks.
    pub(super) fn read(&self) -> Option<Arc<NamedTag>> {
        self.encoded.wait().as_ref().ok().cloned().flatten()
    }
}

/// The shared save gate: an open action savepoint holds tentative state, so no
/// subscribed save may capture, and no callback runs, until it settles.
fn admit_disk_save() -> Result<()> {
    if crate::join_seq::visibility::passes_settled() {
        Ok(())
    } else {
        Err("chunk save refused: an action savepoint is open".into())
    }
}

pub(super) fn encode_record(record: &Record) -> std::io::Result<Option<Arc<NamedTag>>> {
    match record.disk_job() {
        Some(record) => record.encode(),
        None => Ok(record.tag_for_write()),
    }
}

pub(super) fn prepare_batch(batch: &[QueuedWrite]) -> std::io::Result<()> {
    // Coalescing may skip a disk write, never the callback of a captured record.
    let mut refusal = None;
    for job in batch {
        if let Some(record) = job.record.disk_job()
            && let Err(error) = record.encode()
        {
            refusal.get_or_insert(error);
        }
    }
    refusal.map_or(Ok(()), Err)
}

fn replace_entries(tag: &mut NamedTag, entries: Compound) -> std::io::Result<()> {
    let Tag::Compound(root) = &mut tag.tag else {
        return Err(std::io::Error::other("invalid chunk root"));
    };
    if entries.is_empty() {
        root.remove(&ATTACHMENTS_KEY.into());
    } else {
        root.insert(ATTACHMENTS_KEY.into(), Tag::Compound(entries));
    }
    Ok(())
}

impl SaveDir {
    /// Bind disk callbacks before this server level starts any chunk workers.
    /// The world epoch belongs to the existing lifecycle owner. Remote caches
    /// never call this method. With no subscribers no context is retained.
    pub fn set_chunk_disk_context(
        &self,
        world: WorldEpoch,
        dimension: mod_api::Identifier,
        resources: Arc<sunrise_world::native_load_context::WorldgenContext>,
    ) {
        if !mod_api::__private::listened(mod_api::HookId::ChunkDataLoad)
            && !mod_api::__private::listened(mod_api::HookId::ChunkDataSave)
        {
            return;
        }
        let _ = self.disk.set(Arc::new(DiskRuntime {
            save_suppressed: Mutex::default(),
            failed_saves: Mutex::default(),
            world,
            dimension,
            resources,
        }));
    }

    /// Whether `x`, `z` failed its load listener this session. Such a chunk loads with its stored
    /// attachments untouched and is saved without any save listener, so half-initialised state
    /// never replaces the stored record. A poisoned set suppresses, never lets a listener run.
    pub(super) fn chunk_save_suppressed(&self, x: i32, z: i32) -> bool {
        self.disk.get().is_some_and(|runtime| {
            runtime
                .save_suppressed
                .lock()
                .map_or(true, |suppressed| suppressed.contains(&(x, z)))
        })
    }

    pub(super) fn capture_disk_record(&self, record: Record) -> Result<Record> {
        let (chunk, kind) = match &record {
            Record::Chunk(chunk, ..) => (&**chunk, ChunkDiskKind::Full),
            Record::Proto(proto, ..) => (&proto.column, ChunkDiskKind::Proto),
            _ => return Ok(record),
        };
        if !mod_api::__private::listened(mod_api::HookId::ChunkDataSave)
            || self.chunk_save_suppressed(chunk.x, chunk.z)
        {
            return Ok(record);
        }
        admit_disk_save()?;
        let Some(runtime) = self.disk.get() else {
            if !self.trusted_fluid_load {
                return Ok(record);
            }
            return Err("chunk disk worker context is not bound".into());
        };
        let Some(job) = runtime.capture(chunk, kind, ChunkDiskPhase::Save) else {
            return Ok(record);
        };
        Ok(Record::Disk(Arc::new(DiskRecord {
            original: record,
            job,
            runtime: runtime.clone(),
            encoded: OnceLock::new(),
        })))
    }

    pub(super) fn load_disk_attachments(
        &self,
        chunk: &mut Chunk,
        kind: ChunkDiskKind,
    ) -> Result<()> {
        if !mod_api::__private::listened(mod_api::HookId::ChunkDataLoad) {
            return Ok(());
        }
        let Some(runtime) = self.disk.get() else {
            // This save directory can also be a non-authoritative remote cache.
            return Ok(());
        };
        let Some(job) = runtime.capture(chunk, kind, ChunkDiskPhase::Load) else {
            return Ok(());
        };
        match runtime.run(&job) {
            Ok(entries) => {
                Arc::make_mut(chunk.saved.get_or_insert_with(Default::default)).attachments =
                    Attachments::from_nbt(entries);
                Ok(())
            }
            Err(error) => {
                // The chunk loads with the attachments it was stored with.
                let first = runtime
                    .save_suppressed
                    .lock()
                    .map_err(|_| "chunk disk suppression poisoned")?
                    .insert((chunk.x, chunk.z));
                if first {
                    log::error!(
                        "chunk {},{}: a chunk_data_load listener failed ({error}); the stored record \
                         is kept unchanged and no chunk_data_save listener runs for this chunk this session",
                        chunk.x,
                        chunk.z
                    );
                }
                Ok(())
            }
        }
    }

    pub(super) fn save_disk_sync(
        &self,
        chunks: &[Arc<Chunk>],
        registry: &BlockRegistry,
        biomes: &Biomes,
        items: &crate::items::Items,
    ) -> Option<Result<usize>> {
        if !mod_api::__private::listened(mod_api::HookId::ChunkDataSave) {
            return None;
        }
        if let Err(error) = admit_disk_save() {
            return Some(Err(error));
        }
        Some((|| {
            let runtime = self
                .disk
                .get()
                .ok_or("chunk disk worker context is not bound")?;
            let jobs: Vec<_> = chunks
                .iter()
                .map(|chunk| {
                    (!self.chunk_save_suppressed(chunk.x, chunk.z))
                        .then(|| runtime.capture(chunk, ChunkDiskKind::Full, ChunkDiskPhase::Save))
                        .flatten()
                })
                .collect();
            let work = || -> std::io::Result<Vec<_>> {
                chunks
                    .iter()
                    .zip(&jobs)
                    .map(|(chunk, job)| {
                        let entries = job.as_ref().map(|job| runtime.run(job)).transpose()?;
                        let blending = self.calculated_blending(chunk.x, chunk.z);
                        let mut tag = encode_saved(
                            chunk,
                            registry,
                            biomes,
                            self.tables.get().map(|t| &**t),
                            self.shape,
                            blending.as_deref(),
                            items,
                        );
                        if let Some(entries) = entries {
                            replace_entries(&mut tag, entries)?;
                        }
                        Ok(((chunk.x, chunk.z), Some(tag)))
                    })
                    .collect()
            };
            let records = std::thread::scope(|scope| scope.spawn(work).join())
                .map_err(|_| "chunk disk worker unwound")??;
            self.write_records("region", records, true)?;
            self.save_poi(chunks, true)?;
            Ok(chunks.len())
        })())
    }
}

/// Remember the chunks of a failed write so the game can mark them dirty again.
pub(super) fn note_failed(batch: &[QueuedWrite]) {
    for job in batch {
        if let Some(record) = job.record.disk_job()
            && let Ok(mut failed) = record.runtime.failed_saves.lock()
        {
            failed.push(job.key);
        }
    }
}

impl SaveDir {
    /// Chunks whose subscribed save was refused or failed, once each. The
    /// caller marks them modified again, so dirty state is never lost to a
    /// save that did not complete. Empty without a subscriber.
    pub fn take_failed_chunk_saves(&self) -> Vec<(i32, i32)> {
        self.disk.get().map_or_else(Vec::new, |runtime| {
            runtime
                .failed_saves
                .lock()
                .map_or_else(|_| Vec::new(), |mut failed| std::mem::take(&mut *failed))
        })
    }

    pub(super) fn note_refused_saves(&self, folder: Folder, keys: &[(i32, i32)]) {
        if folder == Folder::Region
            && let Some(runtime) = self.disk.get()
            && let Ok(mut failed) = runtime.failed_saves.lock()
        {
            failed.extend_from_slice(keys);
        }
    }
}
