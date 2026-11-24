//! SaveDir's bounded admission gate. Ordinary unbound stores keep the existing writer path.
//! Leased stores retain actual Record/QueuedWrite snapshots and their existing admission receipts.
use super::{writes::QueuedWrite, *};

pub(super) const MAX_DEFERRED_RECORDS: usize = 8192;
const MAX_DEFERRED_BYTES: u64 = 64 << 20;
const MAX_DEFERRED_RECORD_BYTES: usize = 16 << 20;
pub(super) type DeferredKey = (Folder, i32, i32);
#[derive(Default)]
pub(super) struct LiveWorldGate {
    leases: usize,
    pub(super) generation: u64,
    pub(super) prepared: Option<u64>,
    failed: bool,
    attachment: Option<Tag>,
    checkpoint_requested: bool,
    costs: HashMap<DeferredKey, u64>,
    attachment_bytes: u64,
    pub(super) deferred: HashMap<DeferredKey, QueuedWrite>,
}
struct LeaseOwner {
    save: Arc<SaveDir>,
}
/// Cloning a lease retains the same requirement. The final clone releases it.
#[derive(Clone)]
#[doc(hidden)]
pub struct LiveWorldSaveLease {
    _owner: Arc<LeaseOwner>,
}
impl Drop for LeaseOwner {
    fn drop(&mut self) {
        if let Ok(mut gate) = self.save.live_world_gate.lock() {
            gate.leases = gate.leases.saturating_sub(1);
            if gate.leases == 0
                && (!gate.deferred.is_empty()
                    || gate.attachment.is_some()
                    || gate.checkpoint_requested
                    || gate.prepared.is_some())
            {
                // A lease cannot turn pending ownership into an ordinary independent save.
                gate.failed = true;
            }
        }
    }
}
impl LiveWorldGate {
    pub(super) fn check(&self) -> Result<()> {
        if self.failed {
            Err("live-world saves are fenced until session recovery".into())
        } else {
            Ok(())
        }
    }
    pub(super) fn acknowledge(
        &mut self,
        generation: u64,
        selected: &HashSet<DeferredKey>,
    ) -> Result<()> {
        self.check()?;
        if self.prepared.is_none() || self.generation != generation {
            return Err("live-world acknowledgment readset changed".into());
        }
        for (key, job) in &self.deferred {
            if selected.contains(key) {
                job.record.accept_block_entity_edits();
            }
        }
        self.deferred.clear();
        self.costs.clear();
        self.attachment_bytes = 0;
        self.attachment = None;
        self.checkpoint_requested = false;
        self.prepared = None;
        Ok(())
    }
    pub(super) fn refuse(&mut self) {
        self.failed = true;
    }
    pub(super) fn required(&self) -> bool {
        self.leases != 0 || self.prepared.is_some() || self.failed
    }
}
impl SaveDir {
    /// Obtain before publication of a source/destination projection or retirement-dirty state.
    /// Lock order is gate -> writer -> pending; the writer thread never takes gate.
    #[doc(hidden)]
    pub fn require_live_world_barrier(self: &Arc<Self>) -> Result<LiveWorldSaveLease> {
        let mut gate = self
            .live_world_gate
            .lock()
            .map_err(|_| "live-world gate poisoned")?;
        gate.check()?;
        if gate.prepared.is_some() {
            return Err("live-world checkpoint already Prepared".into());
        }
        let leases = gate
            .leases
            .checked_add(1)
            .ok_or("live-world lease overflow")?;
        if gate.leases == 0 {
            self.flush_writer()?;
        }
        gate.leases = leases;
        Ok(LiveWorldSaveLease {
            _owner: Arc::new(LeaseOwner { save: self.clone() }),
        })
    }
    pub fn live_world_barrier_required(&self) -> bool {
        self.live_world_gate
            .lock()
            .map_or(true, |gate| gate.required())
    }
    pub fn live_world_barrier_pending(&self) -> bool {
        self.live_world_gate.lock().map_or(true, |gate| {
            gate.failed
                || gate.prepared.is_some()
                || gate.checkpoint_requested
                || !gate.deferred.is_empty()
                || gate.attachment.is_some()
        })
    }
    pub fn request_live_world_barrier(&self) {
        if let Ok(mut gate) = self.live_world_gate.lock() {
            if gate.prepared.is_some() {
                gate.failed = true;
            } else if gate.required() {
                gate.checkpoint_requested = true;
            }
        }
    }
    /// A post-Prepared failure is terminal for this session, including an uncertain WAL publication.
    #[doc(hidden)]
    pub fn fail_live_world_save(&self) {
        if let Ok(mut gate) = self.live_world_gate.lock() {
            gate.failed = true;
        }
    }
    /// The existing attachment envelope is retained only as an ordinary pending snapshot.
    /// freeze uses the latest actual World.attachments from the same Game barrier.
    pub(super) fn write_or_defer_attachments(&self, tag: Tag) -> Result<()> {
        let mut gate = self
            .live_world_gate
            .lock()
            .map_err(|_| "live-world gate poisoned")?;
        gate.check()?;
        if gate.required() {
            if gate.prepared.is_some() {
                gate.failed = true;
                return Err("ordinary attachment save after Prepared".into());
            }
            let length = nbt::write_named(
                &NamedTag {
                    name: "".into(),
                    tag: tag.clone(),
                },
                Limits {
                    max_bytes: MAX_DEFERRED_RECORD_BYTES,
                    ..Limits::saved()
                },
            )?
            .len() as u64;
            if gate
                .costs
                .values()
                .try_fold(length, |total, bytes| total.checked_add(*bytes))
                .is_none_or(|total| total > MAX_DEFERRED_BYTES)
            {
                gate.failed = true;
                return Err("live-world deferred attachment byte budget".into());
            }
            gate.attachment_bytes = length;
            gate.generation = gate
                .generation
                .checked_add(1)
                .ok_or("live-world queue generation exhausted")?;
            gate.attachment = Some(tag);
            Ok(())
        } else {
            write_dat(&self.attachments_path(), tag)
        }
    }
    /// Called by the queue wrapper while it holds gate. A failure accepts none of this batch.
    pub(super) fn defer_live_world_records(
        &self,
        gate: &mut LiveWorldGate,
        folder: Folder,
        records: Vec<((i32, i32), Record)>,
    ) -> Result<()> {
        gate.check()?;
        if gate.prepared.is_some() {
            return Err("ordinary save after Prepared".into());
        }
        let unique: HashSet<_> = records.iter().map(|(k, _)| (folder, k.0, k.1)).collect();
        let added = unique
            .iter()
            .filter(|key| !gate.deferred.contains_key(key))
            .count();
        if gate
            .deferred
            .len()
            .checked_add(added)
            .is_none_or(|count| count > MAX_DEFERRED_RECORDS)
        {
            return Err(
                "live-world deferred record budget; retain the source chunks/protos".into(),
            );
        }
        let generation = gate
            .generation
            .checked_add(1)
            .ok_or("live-world queue generation exhausted")?;
        // Capture callbacks run on the owner before the bounded encoding worker sees the records.
        // The completed snapshot is published only after the entire batch passes admission.
        let records = records
            .into_iter()
            .map(|(key, record)| self.capture_disk_record(record).map(|record| (key, record)))
            .collect::<std::result::Result<Vec<_>, _>>()?;
        // Measure the actual payload on a disk worker. Freeze reuses the encoded snapshot.
        let costs = std::thread::scope(|scope| {
            scope
                .spawn(|| -> std::io::Result<Vec<u64>> {
                    records
                        .iter()
                        .map(|(_, record)| -> std::io::Result<u64> {
                            chunk_disk::encode_record(record)?
                                .map(|tag| {
                                    nbt::write_named(
                                        &tag,
                                        Limits {
                                            max_bytes: MAX_DEFERRED_RECORD_BYTES,
                                            ..Limits::saved()
                                        },
                                    )
                                    .map(|bytes| bytes.len() as u64)
                                    .map_err(std::io::Error::other)
                                })
                                .transpose()
                                .map(|bytes| bytes.unwrap_or(0))
                        })
                        .collect()
                })
                .join()
        })
        .map_err(|_| "deferred disk snapshot worker unwound")??;
        let mut projected = gate.costs.clone();
        for ((key, _), cost) in records.iter().zip(costs) {
            projected.insert((folder, key.0, key.1), cost);
        }
        if projected
            .values()
            .try_fold(gate.attachment_bytes, |total, cost| {
                total.checked_add(*cost)
            })
            .is_none_or(|total| total > MAX_DEFERRED_BYTES)
        {
            return Err("live-world deferred payload byte budget".into());
        }
        let mut jobs = Vec::with_capacity(records.len());
        for (key, record) in records {
            let job = QueuedWrite::new(key, record, self.sequence.fetch_add(1, Ordering::Relaxed));
            jobs.push(job);
        }
        for job in jobs {
            gate.deferred.insert((folder, job.key.0, job.key.1), job);
        }
        // Deferred records are not published into PendingWrites: DiskRecord/EditedChunk readers would
        // wait for a writer that cannot run until Prepared. World/Streaming retain the live chunks;
        // pre-Prepared disk/cache reads keep the previous durable checkpoint. Frozen queue publication
        // uses the existing PendingWrites/cache path after Prepared, with no callback on a reader.
        gate.costs = projected;
        gate.generation = generation;
        Ok(())
    }
    pub(super) fn snapshot_live_world_deferred(&self) -> Result<(u64, Vec<(DeferredKey, Record)>)> {
        let gate = self
            .live_world_gate
            .lock()
            .map_err(|_| "live-world gate poisoned")?;
        gate.check()?;
        if gate.leases == 0 || gate.prepared.is_some() {
            return Err("live-world freeze requires an idle leased store".into());
        }
        let mut jobs: Vec<_> = gate.deferred.iter().collect();
        jobs.sort_by_key(|(_, job)| job.seq);
        Ok((
            gate.generation,
            jobs.into_iter()
                .map(|(key, job)| (*key, job.record.clone()))
                .collect(),
        ))
    }
    #[doc(hidden)]
    pub fn validate_frozen_live_world_save(
        &self,
        frozen: &live_world_save_snapshot::FrozenLevelSave,
    ) -> Result<()> {
        let gate = self
            .live_world_gate
            .lock()
            .map_err(|_| "live-world gate poisoned")?;
        gate.check()?;
        if gate.leases == 0
            || gate.prepared.is_some()
            || gate.generation != frozen.generation
            || !Arc::ptr_eq(&self.live_world_gate, &frozen.gate)
        {
            return Err("live-world frozen readset changed".into());
        }
        Ok(())
    }
    /// Must follow successful WAL publication and whole-Game final revalidation. No current core write here.
    #[doc(hidden)]
    pub fn mark_live_world_prepared(
        &self,
        root: &Path,
        prepared: &live_world_journal::Prepared,
        frozen: &live_world_save_snapshot::FrozenLevelSave,
    ) -> Result<()> {
        prepared.covers(root, &frozen.images)?;
        let mut gate = self
            .live_world_gate
            .lock()
            .map_err(|_| "live-world gate poisoned")?;
        gate.check()?;
        if gate.leases == 0
            || gate.prepared.is_some()
            || gate.generation != frozen.generation
            || !Arc::ptr_eq(&self.live_world_gate, &frozen.gate)
        {
            gate.failed = true;
            return Err("Prepared readset changed; session recovery required".into());
        }
        frozen
            .prepared_root
            .set(root.to_owned())
            .map_err(|_| "prepared world root already bound")?;
        gate.prepared = Some(prepared.epoch);
        Ok(())
    }
    #[doc(hidden)]
    pub fn ready_frozen_live_world_ack(
        &self,
        frozen: &live_world_save_snapshot::FrozenLevelSave,
    ) -> Result<()> {
        let gate = self
            .live_world_gate
            .lock()
            .map_err(|_| "live-world gate poisoned")?;
        gate.check()?;
        if gate.prepared.is_none()
            || gate.generation != frozen.generation
            || !Arc::ptr_eq(&self.live_world_gate, &frozen.gate)
        {
            return Err("Prepared live-world readset changed".into());
        }
        Ok(())
    }
    pub(super) fn write_frozen_records(
        &self,
        frozen: &live_world_save_snapshot::FrozenLevelSave,
        folder: Folder,
        records: Vec<((i32, i32), Option<NamedTag>)>,
    ) -> Result<()> {
        let gate = self
            .live_world_gate
            .lock()
            .map_err(|_| "live-world gate poisoned")?;
        gate.check()?;
        if gate.prepared.is_none()
            || gate.generation != frozen.generation
            || !Arc::ptr_eq(&self.live_world_gate, &frozen.gate)
        {
            return Err("frozen writer is not authorized by Prepared".into());
        }
        let records = records
            .into_iter()
            .map(|(k, tag)| {
                (
                    k,
                    tag.map_or(Record::Delete, |tag| Record::Tag(Arc::new(tag))),
                )
            })
            .collect();
        // The existing queue supplies its per-record outcomes, cache invalidation and fsync.
        // Tag/Delete cannot enter capture_disk_record's callback branch.
        self.write_prepared_records(folder, records)
    }
}
