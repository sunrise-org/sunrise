//! Frozen current save payloads; ordinary SaveDir writer and actual BlockEntity owners remain authoritative.
use super::live_world_journal::{Folder as JournalFolder, Image, Key, Limits as JournalLimits};
use super::live_world_save_gate::{DeferredKey, LiveWorldGate};
use super::*;

#[doc(hidden)]
pub struct FrozenLevelSave {
    pub images: Vec<Image>,
    pub(super) generation: u64,
    pub(super) gate: Arc<Mutex<LiveWorldGate>>,
    pub(super) prepared_root: OnceLock<PathBuf>,
    receipts: Vec<Record>,
    selected_deferred: HashSet<DeferredKey>,
}
impl FrozenLevelSave {
    /// Only AFTER the common Prepared::acknowledge succeeds for every level and root epoch.
    pub fn acknowledge(&mut self) -> Result<()> {
        self.gate
            .lock()
            .map_err(|_| "live-world gate poisoned")?
            .acknowledge(self.generation, &self.selected_deferred)?;
        for record in &self.receipts {
            record.accept_block_entity_edits();
        }
        Ok(())
    }
}
impl Drop for FrozenLevelSave {
    fn drop(&mut self) {
        // Borrowed deferred receipts stay in gate on pre-Prepared failure. Only fresh receipts resolve here.
        for record in &self.receipts {
            record.resolve_block_entity_edits(false);
        }
    }
}
fn journal_folder(folder: Folder) -> JournalFolder {
    match folder {
        Folder::Region => JournalFolder::Region,
        Folder::Entities => JournalFolder::Entities,
        Folder::Poi => JournalFolder::Poi,
    }
}
impl SaveDir {
    /// Snapshot selected terrain from real Game::with_entities chunks/BE/schedules, all
    /// supplied entity/POI columns, pending ordinary records and actual attachments.
    /// Acquire a lease first; it drained only earlier independent writes. Freeze never flushes current deferred data.
    #[doc(hidden)]
    pub fn freeze_live_world_save(
        &self,
        level: &str,
        chunks: &[Arc<Chunk>],
        terrain_columns: &std::collections::BTreeSet<(i32, i32)>,
        entities: &[((i32, i32), Vec<Tag>)],
        registry: &'static BlockRegistry,
        biomes: &Arc<Biomes>,
        attachments: &sunrise_mod_runtime::attachments::Attachments,
        projection: Option<Vec<u8>>,
        limits: JournalLimits,
    ) -> Result<FrozenLevelSave> {
        if !crate::join_seq::visibility::passes_settled() {
            return Err("live-world save refused: an action savepoint is open".into());
        }
        if !sunrise_core::level_id::valid_level_id(level) || level.len() > 512 {
            return Err("live-world save level id".into());
        }
        let (generation, deferred) = self.snapshot_live_world_deferred()?;
        let cap = deferred
            .len()
            .checked_add(terrain_columns.len())
            .and_then(|n| n.checked_add(entities.len()))
            .and_then(|n| {
                n.checked_add(if self.tables.get().is_some() {
                    chunks.len()
                } else {
                    0
                })
            })
            .and_then(|n| n.checked_add(2))
            .ok_or("live-world record overflow")?;
        if cap > limits.records as usize {
            return Err("live-world save record budget".into());
        }
        let mut records: HashMap<DeferredKey, Record> = deferred.iter().cloned().collect();
        let mut selected_deferred: HashSet<_> = records.keys().copied().collect();
        let mut entity_rows: HashMap<_, _> =
            entities.iter().map(|(key, rows)| (*key, rows)).collect();
        if entity_rows.len() != entities.len() || entity_rows.len() != chunks.len() {
            return Err("live-world entity snapshot coverage".into());
        }
        {
            let failed = self
                .failed_entity_reads
                .lock()
                .map_err(|_| "failed entity reads poisoned")?;
            if entity_rows.keys().any(|key| failed.contains(key)) {
                return Err("live-world entity snapshot would replace an unreadable column".into());
            }
        }
        let mut fresh_keys = HashSet::new();
        let encoding = self.encoding_for_save_later(chunks, registry, biomes)?;
        let professions = self
            .professions
            .read()
            .map_err(|_| "profession snapshot poisoned")?
            .clone();
        let mut frozen = FrozenLevelSave {
            images: Vec::with_capacity(cap),
            generation,
            gate: self.live_world_gate.clone(),
            prepared_root: OnceLock::new(),
            receipts: Vec::with_capacity(terrain_columns.len()),
            selected_deferred: HashSet::new(),
        };
        for chunk in chunks {
            let key = (Folder::Region, chunk.x, chunk.z);
            if !fresh_keys.insert(key) {
                return Err("duplicate fresh chunk in live-world snapshot".into());
            }
            if terrain_columns.contains(&(chunk.x, chunk.z)) {
                let record = self
                    .capture_disk_record(Record::Chunk(
                        chunk.clone(),
                        encoding.clone(),
                        self.calculated_blending(chunk.x, chunk.z),
                    ))?
                    .track_block_entity_edits();
                frozen.receipts.push(record.clone());
                selected_deferred.remove(&key);
                records.insert(key, record);
            }
            // Explicit actual Game::entity_chunks snapshots, including ordinary deletions.
            // Chunk.entities belongs to loading and must not masquerade as the current live owners.
            let entities = entity_rows
                .remove(&(chunk.x, chunk.z))
                .ok_or("live-world entity snapshot missing column")?;
            let key = (Folder::Entities, chunk.x, chunk.z);
            let record = if entities.is_empty() {
                Record::Delete
            } else {
                Record::Tag(Arc::new(NamedTag {
                    name: "".into(),
                    tag: Tag::compound([
                        ("DataVersion", Tag::Int(DATA_VERSION)),
                        ("Position", Tag::IntArray(vec![chunk.x, chunk.z])),
                        ("Entities", Tag::List(entities.clone())),
                    ]),
                }))
            };
            selected_deferred.remove(&key);
            records.insert(key, record);
            if let Some(tables) = self.tables.get() {
                let key = (Folder::Poi, chunk.x, chunk.z);
                selected_deferred.remove(&key);
                records.insert(
                    key,
                    Record::Poi(chunk.clone(), tables.clone(), professions.clone()),
                );
            }
        }
        if terrain_columns
            .iter()
            .any(|&(x, z)| !fresh_keys.contains(&(Folder::Region, x, z)))
        {
            return Err("live-world terrain selection outside snapshot".into());
        }
        // The owner ran each capture callback once. Reuse its encoded record on the disk worker.
        let encoded = std::thread::scope(|scope| {
            scope
                .spawn(|| -> std::io::Result<Vec<_>> {
                    for (_, record) in &deferred {
                        if record.disk_job().is_some() {
                            chunk_disk::encode_record(record)?;
                        }
                    }
                    for record in &frozen.receipts {
                        if record.disk_job().is_some() {
                            chunk_disk::encode_record(record)?;
                        }
                    }
                    let mut result = Vec::with_capacity(records.len());
                    for (key, record) in &records {
                        let record = if key.0 == Folder::Poi {
                            resolve_poi(self.region_storage(Folder::Poi), (key.1, key.2), record)
                        } else {
                            Some(record.clone())
                        };
                        if let Some(record) = record {
                            result.push((*key, chunk_disk::encode_record(&record)?));
                        }
                    }
                    Ok(result)
                })
                .join()
        })
        .map_err(|_| "live-world disk snapshot worker unwound")??;
        frozen.selected_deferred = selected_deferred;
        let nbt_limits = Limits {
            max_bytes: limits.record_bytes as usize,
            ..Limits::saved()
        };
        let mut bytes = 0u64;
        let mut push = |key, after: Option<Vec<u8>>| -> Result<()> {
            if after
                .as_ref()
                .is_some_and(|data| data.len() > limits.record_bytes as usize)
            {
                return Err("live-world record byte budget".into());
            }
            bytes = bytes
                .checked_add(after.as_ref().map_or(0, |data| data.len() as u64))
                .ok_or("live-world byte overflow")?;
            if bytes > limits.bytes {
                return Err(Box::new(super::live_world_journal::PayloadBudgetExceeded {
                    required_at_least: bytes,
                    limit: limits.bytes,
                    records_encoded: (frozen.images.len() + 1) as u32,
                }));
            }
            frozen.images.push(Image { key, after });
            Ok(())
        };
        for ((folder, x, z), tag) in encoded {
            push(
                Key::Record {
                    level: level.into(),
                    folder: journal_folder(folder),
                    chunk: [x, z],
                },
                tag.map(|tag| nbt::write_named(&tag, nbt_limits))
                    .transpose()?,
            )?;
        }
        let root = NamedTag {
            name: "".into(),
            tag: Tag::compound([
                ("data", Tag::Compound(attachments.try_to_nbt()?)),
                ("DataVersion", Tag::Int(DATA_VERSION)),
            ]),
        };
        push(
            Key::Attachments {
                level: level.into(),
            },
            Some(Compression::Gzip.encode(&nbt::write_named(&root, nbt_limits)?)?),
        )?;
        push(
            Key::Projection {
                level: level.into(),
            },
            projection,
        )?;
        self.validate_frozen_live_world_save(&frozen)?;
        Ok(frozen)
    }
    /// Only after mark_live_world_prepared verified that these bytes occur in the durable common WAL.
    /// Any error permanently fences this SaveDir; the Game must fence all other batch members too.
    #[doc(hidden)]
    pub fn write_frozen_live_world_save(
        &self,
        level: &str,
        frozen: &FrozenLevelSave,
    ) -> Result<()> {
        let write = || -> Result<()> {
            // Decode and validate ALL destination records and metadata before this SaveDir's first write.
            // The common Prepared already preflighted the whole cross-level disk readset.
            let mut groups: [Vec<((i32, i32), Option<NamedTag>)>; 3] = Default::default();
            for image in &frozen.images {
                if image.key.level() != level {
                    return Err("live-world save owner mismatch".into());
                }
                if let Key::Record { folder, chunk, .. } = &image.key {
                    let tag = image
                        .after
                        .as_ref()
                        .map(|bytes| -> Result<NamedTag> {
                            let (tag, used) = nbt::read_named(bytes, Limits::saved())?;
                            if used != bytes.len() {
                                return Err("frozen record trailing bytes".into());
                            }
                            Ok(tag)
                        })
                        .transpose()?;
                    let index = match folder {
                        JournalFolder::Region => 0,
                        JournalFolder::Entities => 1,
                        JournalFolder::Poi => 2,
                    };
                    groups[index].push(((chunk[0], chunk[1]), tag));
                }
            }
            for (folder, records) in [Folder::Region, Folder::Entities, Folder::Poi]
                .into_iter()
                .zip(groups)
            {
                self.write_frozen_records(frozen, folder, records)?;
            }
            let root = frozen
                .prepared_root
                .get()
                .ok_or("prepared world root missing")?;
            for image in &frozen.images {
                if matches!(image.key, Key::Record { .. }) {
                    continue;
                }
                live_world_journal::write_level_metadata(root, image)?;
            }
            Ok(())
        };
        let result = write();
        if result.is_err() {
            self.fail_live_world_save();
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::super::live_world_journal::{Prepared, checkpoint_image};
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    struct Scratch(PathBuf);
    impl Scratch {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "sunrise-live-gate-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    fn tag(value: i32) -> NamedTag {
        NamedTag {
            name: "".into(),
            tag: Tag::compound([("value", Tag::Int(value))]),
        }
    }
    fn fixture() -> (Scratch, Arc<SaveDir>) {
        let scratch = Scratch::new();
        let directory = dimension_dir(&scratch.0, "fixture:level");
        fs::create_dir_all(&directory).unwrap();
        let save = Arc::new(SaveDir::new(directory, LevelShape::OVERWORLD, None));
        (scratch, save)
    }
    fn freeze_tags(save: &SaveDir) -> FrozenLevelSave {
        let (generation, records) = save.snapshot_live_world_deferred().unwrap();
        let images = records
            .iter()
            .map(|((folder, x, z), record)| Image {
                key: Key::Record {
                    level: "fixture:level".into(),
                    folder: journal_folder(*folder),
                    chunk: [*x, *z],
                },
                after: chunk_disk::encode_record(record)
                    .unwrap()
                    .map(|tag| nbt::write_named(&tag, Limits::saved()).unwrap()),
            })
            .collect();
        FrozenLevelSave {
            images,
            generation,
            gate: save.live_world_gate.clone(),
            prepared_root: OnceLock::new(),
            receipts: vec![],
            selected_deferred: records.iter().map(|(key, _)| *key).collect(),
        }
    }
    #[test]
    fn leased_ordinary_records_coalesce_without_writes_then_use_real_acknowledged_writer() {
        let (scratch, save) = fixture();
        save.write_records("region", vec![((0, 0), Some(tag(1)))], true)
            .unwrap();
        let lease = save.require_live_world_barrier().unwrap();
        save.write_records("region", vec![((0, 0), Some(tag(2)))], false)
            .unwrap();
        save.write_records("region", vec![((0, 0), Some(tag(3)))], false)
            .unwrap();
        assert!(save.flush().is_err());
        assert!(!save.writes_finished().unwrap());
        assert_eq!(
            save.read_record_raw("region", 0, 0)
                .unwrap()
                .tag
                .get("value"),
            Some(&Tag::Int(1))
        );
        let mut frozen = freeze_tags(&save);
        assert_eq!(frozen.images.len(), 1);
        let mut images = frozen.images.clone();
        images.push(checkpoint_image(1));
        let prepared = Prepared::capture(&scratch.0, 1, images, JournalLimits::default()).unwrap();
        save.validate_frozen_live_world_save(&frozen).unwrap();
        prepared.publish(&scratch.0).unwrap();
        save.mark_live_world_prepared(&scratch.0, &prepared, &frozen)
            .unwrap();
        save.write_frozen_live_world_save("fixture:level", &frozen)
            .unwrap();
        prepared.write_checkpoint_epoch(&scratch.0).unwrap();
        prepared.acknowledge(&scratch.0).unwrap();
        frozen.acknowledge().unwrap();
        drop(lease);
        assert!(!save.live_world_barrier_required());
        assert!(save.writes_finished().unwrap());
        assert_eq!(
            save.read_record_raw("region", 0, 0)
                .unwrap()
                .tag
                .get("value"),
            Some(&Tag::Int(3))
        );
    }
    #[test]
    fn failed_prepared_writer_fences_following_saves_and_recovery_keeps_decision() {
        let (scratch, save) = fixture();
        let lease = save.require_live_world_barrier().unwrap();
        save.write_records("region", vec![((0, 0), Some(tag(4)))], false)
            .unwrap();
        let frozen = freeze_tags(&save);
        let mut images = frozen.images.clone();
        images.push(checkpoint_image(1));
        let prepared = Prepared::capture(&scratch.0, 1, images, JournalLimits::default()).unwrap();
        prepared.publish(&scratch.0).unwrap();
        save.mark_live_world_prepared(&scratch.0, &prepared, &frozen)
            .unwrap();
        // A deterministic real OS failure: the future region directory is a regular file.
        fs::write(save.root.join("region"), b"blocked").unwrap();
        assert!(
            save.write_frozen_live_world_save("fixture:level", &frozen)
                .is_err()
        );
        assert!(
            save.write_records("region", vec![((0, 0), Some(tag(9)))], false)
                .is_err()
        );
        assert!(save.writes_finished().is_err());
        assert!(
            scratch
                .0
                .join("data/sunrise_live_world_journal.bin")
                .exists()
        );
        fs::remove_file(save.root.join("region")).unwrap();
        drop(lease);
        drop(frozen);
        drop(save);
        assert!(
            super::super::live_world_journal::recover(&scratch.0, JournalLimits::default())
                .unwrap()
        );
        let reopened = SaveDir::new(
            dimension_dir(&scratch.0, "fixture:level"),
            LevelShape::OVERWORLD,
            None,
        );
        assert_eq!(
            reopened
                .read_record_raw("region", 0, 0)
                .unwrap()
                .tag
                .get("value"),
            Some(&Tag::Int(4))
        );
    }
    #[test]
    fn changed_deferred_generation_refuses_stale_freeze_without_prepared_or_core_writes() {
        let (_scratch, save) = fixture();
        let _lease = save.require_live_world_barrier().unwrap();
        save.write_records("region", vec![((0, 0), Some(tag(2)))], false)
            .unwrap();
        let frozen = freeze_tags(&save);
        save.write_records("region", vec![((1, 0), Some(tag(5)))], false)
            .unwrap();
        assert!(save.validate_frozen_live_world_save(&frozen).is_err());
        assert!(!save.root.join("region").exists());
        assert_eq!(save.snapshot_live_world_deferred().unwrap().1.len(), 2);
    }
    #[test]
    fn empty_level_metadata_creates_its_dimension_only_after_prepared() {
        let (scratch, save) = fixture();
        fs::remove_dir_all(&save.root).unwrap();
        let lease = save.require_live_world_barrier().unwrap();
        let mut frozen = freeze_tags(&save);
        let root = NamedTag {
            name: "".into(),
            tag: Tag::compound([
                ("DataVersion", Tag::Int(DATA_VERSION)),
                ("data", Tag::compound([("opaque", Tag::Int(71))])),
            ]),
        };
        frozen.images.push(Image {
            key: Key::Attachments {
                level: "fixture:level".into(),
            },
            after: Some(
                Compression::Gzip
                    .encode(&nbt::write_named(&root, Limits::saved()).unwrap())
                    .unwrap(),
            ),
        });
        let mut images = frozen.images.clone();
        images.push(checkpoint_image(1));
        let prepared = Prepared::capture(&scratch.0, 1, images, JournalLimits::default()).unwrap();
        assert!(!save.root.exists());
        prepared.publish(&scratch.0).unwrap();
        save.mark_live_world_prepared(&scratch.0, &prepared, &frozen)
            .unwrap();
        save.write_frozen_live_world_save("fixture:level", &frozen)
            .unwrap();
        prepared.write_checkpoint_epoch(&scratch.0).unwrap();
        save.ready_frozen_live_world_ack(&frozen).unwrap();
        prepared.acknowledge(&scratch.0).unwrap();
        frozen.acknowledge().unwrap();
        drop(lease);
        assert_eq!(
            super::super::read_dat(&save.attachments_path())
                .unwrap()
                .get("data")
                .unwrap()
                .get("opaque"),
            Some(&Tag::Int(71))
        );
    }
    #[test]
    fn missing_entities_region_delete_ack_uses_ordinary_writer_without_inventing_region() {
        let (scratch, save) = fixture();
        let lease = save.require_live_world_barrier().unwrap();
        save.write_records("entities", vec![((0, 0), None)], false)
            .unwrap();
        let mut frozen = freeze_tags(&save);
        let mut images = frozen.images.clone();
        images.push(checkpoint_image(1));
        let prepared = Prepared::capture(&scratch.0, 1, images, JournalLimits::default()).unwrap();
        prepared.publish(&scratch.0).unwrap();
        save.mark_live_world_prepared(&scratch.0, &prepared, &frozen)
            .unwrap();
        save.write_frozen_live_world_save("fixture:level", &frozen)
            .unwrap();
        // apply_image/recovery would create a header and would hide this integration mismatch.
        assert!(!save.root.join("entities/r.0.0.mca").exists());
        prepared.write_checkpoint_epoch(&scratch.0).unwrap();
        prepared.acknowledge(&scratch.0).unwrap();
        frozen.acknowledge().unwrap();
        drop(lease);
        assert!(!save.root.join("entities/r.0.0.mca").exists());
        assert!(
            !scratch
                .0
                .join("data/sunrise_live_world_journal.bin")
                .exists()
        );
    }
}
