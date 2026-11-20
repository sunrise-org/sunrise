//! Common-checkpoint admission over the existing immutable player Snapshot.
//! This owns pending save records, never a live player/inventory or a second player-file write path.
use super::{Autosaves, SavedPlayer, Snapshot, path};
use std::{
    collections::BTreeMap,
    fs,
    path::{Component, Path, PathBuf},
    sync::{Arc, atomic::Ordering},
};
use sunrise_core::nbt::{self, Limits, NamedTag, Tag};

const MAX_PLAYERS: usize = 256;
const MAX_BYTES: u64 = 64 << 20;
const MAX_RECORD_BYTES: usize = 16 << 20;

#[derive(Clone, Debug, PartialEq, Eq)]
struct RootWitness {
    supplied: PathBuf,
    canonical: PathBuf,
    #[cfg(unix)]
    identity: (u64, u64),
}
impl RootWitness {
    fn capture(root: &Path) -> crate::Result<Self> {
        if !root.is_absolute()
            || root
                .components()
                .any(|c| matches!(c, Component::ParentDir | Component::CurDir))
        {
            return Err("common player checkpoint needs one absolute world root".into());
        }
        let metadata = fs::symlink_metadata(root)?;
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            return Err("common player checkpoint root is not a real directory".into());
        }
        #[cfg(unix)]
        let identity = {
            use std::os::unix::fs::MetadataExt;
            (metadata.dev(), metadata.ino())
        };
        Ok(Self {
            supplied: root.to_owned(),
            canonical: fs::canonicalize(root)?,
            #[cfg(unix)]
            identity,
        })
    }

    fn validate(&self) -> crate::Result<()> {
        if Self::capture(&self.supplied)? != *self {
            return Err("common player checkpoint world root changed".into());
        }
        Ok(())
    }
    fn validate_snapshot(&self, snapshot: &Snapshot) -> crate::Result<()> {
        if snapshot.file != path(&self.supplied, snapshot.uuid) {
            return Err("common player snapshot crosses world roots".into());
        }
        Ok(())
    }
}
struct Deferred {
    snapshot: Arc<Snapshot>,
    bytes: u64,
    sequence: u64,
}
#[derive(Default)]
pub(super) struct CommonPlayerSaves {
    required: bool,
    failed: Option<String>,
    generation: u64,
    root: Option<RootWitness>,
    observed_root: Option<PathBuf>,
    observed_multiple_roots: bool,
    snapshots: BTreeMap<u128, Deferred>,
    bytes: u64,
    identity: Arc<()>,
}
/// Read-only exact existing I/O snapshots. The Game may supersede an older same-UUID deferred tag in
/// its common WAL; acknowledgment releases its receipt, never writes those older bytes again.
pub struct FrozenPlayerSaves {
    identity: Arc<()>,
    generation: u64,
    root: Option<RootWitness>,
    snapshots: Vec<Arc<Snapshot>>,
}
impl FrozenPlayerSaves {
    pub fn generation(&self) -> u64 {
        self.generation
    }
    /// None only before this queue has observed any world root; snapshots must then be empty.
    pub fn world_root(&self) -> Option<&Path> {
        self.root.as_ref().map(|r| r.supplied.as_path())
    }
    pub fn snapshots(&self) -> impl ExactSizeIterator<Item = (u128, &str, &Tag)> {
        self.snapshots
            .iter()
            .map(|s| (s.uuid, s.name.as_str(), &s.tag))
    }
}
impl CommonPlayerSaves {
    pub(super) fn observe_root(&mut self, root: &Path) {
        match &self.observed_root {
            Some(previous) if previous != root => self.observed_multiple_roots = true,
            None => self.observed_root = Some(root.to_owned()),
            _ => {}
        }
    }
    pub(super) fn is_empty(&self) -> bool {
        self.snapshots.is_empty()
    }
    pub(super) fn required(&self) -> bool {
        self.required || self.failed.is_some()
    }
    pub(super) fn check(&self) -> crate::Result<()> {
        if let Some(error) = &self.failed {
            return Err(error.clone().into());
        }
        if !self.required {
            return Err("common player checkpoint is not enabled".into());
        }
        Ok(())
    }
    pub(super) fn fail(&mut self, message: impl Into<String>) {
        self.required = true;
        if self.failed.is_none() {
            self.failed = Some(message.into());
        }
    }
    pub(super) fn admit(&mut self, world: &Path, snapshot: Snapshot) -> crate::Result<()> {
        self.check()?;
        if snapshot.name.len() > 1024 || world.as_os_str().len() > 65535 {
            return Err("common player snapshot name/root budget".into());
        }
        let captured = RootWitness::capture(world)?;
        let root = self.root.as_ref().unwrap_or(&captured);
        if *root != captured {
            return Err("common player queue cannot cross world roots".into());
        }
        root.validate_snapshot(&snapshot)?;
        if !self.snapshots.contains_key(&snapshot.uuid) && self.snapshots.len() >= MAX_PLAYERS {
            return Err("common player checkpoint UUID budget".into());
        }
        let bytes = nbt::write_named(
            &NamedTag {
                name: "".into(),
                tag: snapshot.tag.clone(),
            },
            Limits {
                max_bytes: MAX_RECORD_BYTES,
                ..Limits::saved()
            },
        )?
        .len() as u64;
        let bytes = bytes
            .checked_add(snapshot.name.len() as u64)
            .and_then(|n| n.checked_add(snapshot.file.as_os_str().len() as u64))
            .ok_or("common player snapshot byte overflow")?;
        let previous = self.snapshots.get(&snapshot.uuid).map_or(0, |s| s.bytes);
        let total = self
            .bytes
            .checked_sub(previous)
            .and_then(|n| n.checked_add(bytes))
            .ok_or("common player checkpoint byte overflow")?;
        if total > MAX_BYTES {
            return Err("common player checkpoint byte budget".into());
        }
        let next = self
            .generation
            .checked_add(1)
            .ok_or("common player generation exhausted")?;
        self.snapshots.insert(
            snapshot.uuid,
            Deferred {
                snapshot: Arc::new(snapshot),
                bytes,
                sequence: next,
            },
        );
        self.root = Some(captured);
        self.bytes = total;
        self.generation = next;
        Ok(())
    }
}
impl Autosaves {
    /// Drain/join prior independent FIFO work before the first common Prepared can exist. Old receipts
    /// stay ready for the Game's normal completion delivery; this function fires no Game callback.
    /// Any failure is terminal, and the old writer is closed/joined before returning an error.
    pub fn enable_common_checkpoint(&mut self) -> crate::Result<()> {
        if self.common.required() {
            return self.common.check();
        }
        self.common.required = true;
        let completed = self.completed(true);
        self.ready.extend(completed);
        let mut failure = self.error.clone();
        if let Some(writer) = self.writer.take() {
            drop(writer.send);
            if writer.thread.join().is_err() {
                failure.get_or_insert_with(|| "the player writer unwound".into());
            }
        }
        if self.pending != 0 {
            failure.get_or_insert_with(|| "the player writer did not drain".into());
        }
        if self.common.observed_multiple_roots {
            failure.get_or_insert_with(|| {
                "common player FIFO crossed world-session roots; reset after recovery".into()
            });
        }
        if failure.is_none()
            && let Some(root) = &self.common.observed_root
        {
            match RootWitness::capture(root) {
                Ok(root) => self.common.root = Some(root),
                Err(error) => {
                    failure = Some(format!("common player root refused: {error}"));
                }
            }
        }
        if let Some(error) = failure {
            self.common.fail(error.clone());
            return Err(error.into());
        }
        Ok(())
    }
    pub fn common_required(&self) -> bool {
        self.common.required()
    }
    pub fn deferred_generation(&self) -> u64 {
        self.common.generation
    }
    pub fn freeze_common(&self) -> crate::Result<FrozenPlayerSaves> {
        self.common.check()?;
        if self.writer.is_some() || self.pending != 0 {
            return Err("independent player writer still exists".into());
        }
        if let Some(root) = &self.common.root {
            root.validate()?;
        }
        let mut records: Vec<_> = self.common.snapshots.values().collect();
        records.sort_by_key(|s| s.sequence);
        for record in &records {
            self.common
                .root
                .as_ref()
                .ok_or("player snapshot root missing")?
                .validate_snapshot(&record.snapshot)?;
        }
        Ok(FrozenPlayerSaves {
            identity: self.common.identity.clone(),
            generation: self.common.generation,
            root: self.common.root.clone(),
            snapshots: records.into_iter().map(|s| s.snapshot.clone()).collect(),
        })
    }
    pub fn validate_common(&self, frozen: &FrozenPlayerSaves) -> crate::Result<()> {
        self.common.check()?;
        if self.writer.is_some()
            || self.pending != 0
            || !Arc::ptr_eq(&self.common.identity, &frozen.identity)
            || self.common.generation != frozen.generation
            || self.common.root != frozen.root
            || self.common.snapshots.len() != frozen.snapshots.len()
        {
            return Err("common player frozen generation/owner/root changed".into());
        }
        if let Some(root) = &frozen.root {
            root.validate()?;
        }
        for snapshot in &frozen.snapshots {
            let current = self
                .common
                .snapshots
                .get(&snapshot.uuid)
                .ok_or("common player snapshot disappeared")?;
            if !Arc::ptr_eq(&current.snapshot, snapshot) {
                return Err("common player snapshot changed".into());
            }
            frozen
                .root
                .as_ref()
                .ok_or("player snapshot root missing")?
                .validate_snapshot(snapshot)?;
        }
        Ok(())
    }
    /// Only after the GLOBAL WAL is fully acknowledged. The global batch may contain a newer actual
    /// live capture at the same UUID. Never replay these deferred files or rerun save callbacks here.
    pub fn acknowledge_common(
        &mut self,
        frozen: &FrozenPlayerSaves,
    ) -> crate::Result<Vec<SavedPlayer>> {
        self.validate_common(frozen)?;
        let next = self
            .common
            .generation
            .checked_add(1)
            .ok_or("common player generation exhausted")?;
        let saved = frozen
            .snapshots
            .iter()
            .map(|snapshot| SavedPlayer {
                uuid: snapshot.uuid,
                name: snapshot.name.clone(),
            })
            .collect();
        self.common.snapshots.clear();
        self.common.bytes = 0;
        self.common.generation = next;
        Ok(saved)
    }
    /// Once a common decision fails, no independent player save can run again in this session.
    /// Successful enable already guarantees writer=None. Cancellation also closes an older FIFO if a
    /// caller stops before enable; it cannot interrupt an already-running write, so no Prepared may
    /// be published until enable succeeded.
    pub fn fail_common(&mut self) {
        self.common
            .fail("player saves are fenced until fresh-process common recovery");
        if let Some(writer) = self.writer.take() {
            writer.stop.store(true, Ordering::Release);
            drop(writer.send);
            let _ = writer.thread.join();
            while let Ok(result) = writer.done.try_recv() {
                if let Ok(player) = result {
                    self.ready.push(player);
                }
            }
        }
        self.pending = 0;
    }
}

#[cfg(test)]
#[path = "playerdata_common_checkpoint_tests.rs"]
mod tests;
