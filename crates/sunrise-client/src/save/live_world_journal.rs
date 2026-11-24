//! Bounded common save decisions over the existing acknowledged storage writers.
//! A prepared save contains frozen, already-hooked core records and metadata.
//! Live inventories remain in World/BlockEntities; these bytes are only WAL afterimages.
//! Normal commits use SaveDir's acknowledged writer; recovery runs before any reader/writer opens.

use sha2::{Digest as _, Sha256};
use std::{
    collections::BTreeSet,
    fs,
    io::{self, Read},
    path::{Path, PathBuf},
};
use sunrise_core::{
    level_id::valid_level_id,
    nbt::{self, Limits as NbtLimits, NamedTag},
    position::ChunkPos,
};
use sunrise_mod_package::durable;
use sunrise_world::{
    carrier::region_strict::{self, Strict},
    compression::Compression,
    region::{ChunkRecord, RegionFile},
};

#[path = "live_world_region_witness.rs"]
mod region_witness;
use region_witness::RegionWitness;

const MAGIC: &[u8; 8] = b"SNLWJNL2";
const EPOCH_MAGIC: &[u8; 8] = b"SNLWEP01";
const EPOCH_NAME: &str = "sunrise_live_world_epoch.bin";
const NAME: &str = "sunrise_live_world_journal.bin";

#[derive(Clone, Copy)]
#[doc(hidden)]
pub struct Limits {
    pub records: u32,
    pub bytes: u64,
    pub record_bytes: u32,
    pub levels: u32,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            records: 8192,
            bytes: 64 << 20,
            record_bytes: 16 << 20,
            levels: 64,
        }
    }
}
impl Limits {
    fn check(self) -> io::Result<Self> {
        if self.records == 0
            || self.records > 32768
            || self.bytes == 0
            || self.bytes > 512 << 20
            || self.record_bytes == 0
            || self.record_bytes > 64 << 20
            || u64::from(self.record_bytes) > self.bytes
            || self.levels == 0
            || self.levels > 256
        {
            return Err(invalid("live-world journal budget"));
        }
        Ok(self)
    }
    fn nbt(self) -> NbtLimits {
        NbtLimits {
            max_bytes: self.record_bytes as usize,
            ..NbtLimits::saved()
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[doc(hidden)]
pub enum Folder {
    Region,
    Entities,
    Poi,
}
impl Folder {
    pub fn name(self) -> &'static str {
        match self {
            Self::Region => "region",
            Self::Entities => "entities",
            Self::Poi => "poi",
        }
    }
}

/// Typed destinations only. A journal never carries an arbitrary filesystem path.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[doc(hidden)]
pub enum Key {
    Record {
        level: String,
        folder: Folder,
        chunk: [i32; 2],
    },
    Attachments {
        level: String,
    },
    Projection {
        level: String,
    },
    /// Preserved exact earlier experimental descriptor during an explicit development migration.
    ProjectionOriginal {
        level: String,
    },
    /// Exact existing playerdata bytes after the Game's ordinary player-save callback.
    PlayerData {
        uuid: u128,
    },
    /// Fixed world-root identity for the common checkpoint; never a caller-supplied path.
    CheckpointEpoch,
}
impl Key {
    pub fn level(&self) -> &str {
        match self {
            Self::Record { level, .. }
            | Self::Attachments { level }
            | Self::Projection { level }
            | Self::ProjectionOriginal { level } => level,
            Self::CheckpointEpoch | Self::PlayerData { .. } => "",
        }
    }
    fn path(&self, root: &Path) -> io::Result<PathBuf> {
        if !matches!(self, Self::CheckpointEpoch | Self::PlayerData { .. })
            && (self.level().len() > 512 || !valid_level_id(self.level()))
        {
            return Err(invalid("journal level id"));
        }
        let directory = super::dimension_dir(root, self.level());
        let path = match self {
            Self::Record { folder, chunk, .. } => directory.join(folder.name()).join(format!(
                "r.{}.{}.mca",
                chunk[0] >> 5,
                chunk[1] >> 5
            )),
            Self::Attachments { .. } => directory.join("data/sunrise/attachments.dat"),
            Self::Projection { .. } => directory.join("data/sunrise_live_world_projection.json"),
            Self::ProjectionOriginal { .. } => {
                directory.join("data/sunrise_live_world_projection.v1.original.json")
            }
            Self::CheckpointEpoch => root.join("data").join(EPOCH_NAME),
            Self::PlayerData { uuid } => crate::lan::playerdata::path(root, *uuid),
        };
        checked_path(root, &path)?;
        if let Self::Record { chunk, .. } = self {
            checked_path(
                root,
                &path
                    .parent()
                    .unwrap()
                    .join(format!("c.{}.{}.mcc", chunk[0], chunk[1])),
            )?;
            checked_path(
                root,
                &path
                    .parent()
                    .unwrap()
                    .join(format!(".c.{}.{}.mcc.tmp", chunk[0], chunk[1])),
            )?;
        } else {
            checked_path(
                root,
                &path.with_file_name(format!(
                    ".{}.durable-new",
                    path.file_name().unwrap().to_str().unwrap()
                )),
            )?;
        }
        Ok(path)
    }
}

#[derive(Clone)]
#[doc(hidden)]
pub struct Image {
    pub key: Key,
    /// Uncompressed named NBT for records; exact durable file bytes for attachments/metadata.
    pub after: Option<Vec<u8>>,
}
struct Change {
    image: Image,
    before: Option<[u8; 32]>,
}
#[doc(hidden)]
pub struct Prepared {
    pub epoch: u64,
    changes: Vec<Change>,
    regions: Vec<RegionWitness>,
    limits: Limits,
}

/// Exact encoded decision size, measured from borrowed frozen bytes without cloning them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[doc(hidden)]
pub struct Usage {
    pub serialized_bytes: u64,
    pub payload_bytes: u64,
    /// Region count and allocation/header witnesses, excluding the common checksum.
    pub witness_bytes: u64,
    pub records: u32,
    pub levels: u32,
    pub terrain_records: u32,
    pub entity_records: u32,
    pub poi_records: u32,
    pub metadata_records: u32,
}

#[derive(Debug)]
#[doc(hidden)]
pub struct ByteBudgetExceeded {
    pub usage: Usage,
    pub limit: u64,
}
impl std::fmt::Display for ByteBudgetExceeded {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "journal byte budget: required={} limit={} payload={} witnesses={} records={} levels={} terrain={} entities={} poi={} metadata={}",
            self.usage.serialized_bytes,
            self.limit,
            self.usage.payload_bytes,
            self.usage.witness_bytes,
            self.usage.records,
            self.usage.levels,
            self.usage.terrain_records,
            self.usage.entity_records,
            self.usage.poi_records,
            self.usage.metadata_records,
        )
    }
}
impl std::error::Error for ByteBudgetExceeded {}

/// Distinguish this pre-decision operational refusal from malformed data or an I/O failure.
#[doc(hidden)]
pub fn byte_budget_error(error: &io::Error) -> Option<&ByteBudgetExceeded> {
    error.get_ref()?.downcast_ref()
}

/// Early frozen-payload cap. Remaining records were not encoded, so this is explicitly a
/// lower bound and must never be presented as the exact complete journal requirement.
#[derive(Debug)]
#[doc(hidden)]
pub struct PayloadBudgetExceeded {
    pub required_at_least: u64,
    pub limit: u64,
    pub records_encoded: u32,
}
impl std::fmt::Display for PayloadBudgetExceeded {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "live-world payload byte budget: required_at_least={} limit={} records_encoded={} (complete journal size not measured)",
            self.required_at_least, self.limit, self.records_encoded
        )
    }
}
impl std::error::Error for PayloadBudgetExceeded {}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}
fn digest(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}
fn signature(bytes: Option<&[u8]>) -> Option<[u8; 32]> {
    bytes.map(digest)
}

/// Do not follow a journal-supplied symlink out of the locked world. Reads do not create directories.
fn checked_path(root: &Path, path: &Path) -> io::Result<()> {
    if fs::symlink_metadata(root)?.file_type().is_symlink() {
        return Err(invalid("journal root is a symlink"));
    }
    let relative = path
        .strip_prefix(root)
        .map_err(|_| invalid("journal path outside world"))?;
    let mut next = root.to_owned();
    for component in relative.components() {
        if !matches!(component, std::path::Component::Normal(_)) {
            return Err(invalid("journal path component"));
        }
        next.push(component);
        match fs::symlink_metadata(&next) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(invalid("journal path is a symlink"));
            }
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
    }
    Ok(())
}
fn read_file(path: &Path, cap: u64) -> io::Result<Option<Vec<u8>>> {
    let file = match fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    if !file.metadata()?.is_file() {
        return Err(invalid("journal destination is not a regular file"));
    }
    let mut bytes = Vec::new();
    file.take(cap + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > cap {
        return Err(invalid("journal file exceeds budget"));
    }
    Ok(Some(bytes))
}
fn named(bytes: &[u8], limits: Limits) -> io::Result<NamedTag> {
    let (tag, used) = nbt::read_named(bytes, limits.nbt()).map_err(|_| invalid("journal NBT"))?;
    if used != bytes.len() {
        return Err(invalid("journal NBT trailing bytes"));
    }
    Ok(tag)
}
fn read_image(root: &Path, key: &Key, limits: Limits) -> io::Result<Option<Vec<u8>>> {
    let path = key.path(root)?;
    match key {
        Key::Record { chunk, .. } => {
            let tag = match region_strict::read_chunk(
                path.parent().unwrap(),
                ChunkPos::new(chunk[0], chunk[1]),
            ) {
                Ok(tag) => tag,
                Err(Strict::NoRegion) => None,
                Err(error) => return Err(invalid(&format!("journal strict region read: {error}"))),
            };
            tag.map(|tag| {
                nbt::write_named(&tag, limits.nbt())
                    .map_err(|_| invalid("journal existing NBT budget"))
            })
            .transpose()
        }
        _ => read_file(&path, u64::from(limits.record_bytes)),
    }
}
fn validate_image(image: &Image, limits: Limits) -> io::Result<()> {
    let Some(bytes) = image.after.as_deref() else {
        return Ok(());
    };
    if bytes.len() > limits.record_bytes as usize {
        return Err(invalid("journal record exceeds budget"));
    }
    match &image.key {
        Key::Record { .. } => {
            named(bytes, limits)?;
        }
        Key::Attachments { .. } | Key::PlayerData { .. } => {
            let raw = Compression::Gzip.decode(bytes, limits.record_bytes as usize)?;
            named(&raw, limits)?;
        }
        Key::Projection { .. } | Key::ProjectionOriginal { .. } => {
            serde_json::from_slice::<serde_json::Value>(bytes)
                .map_err(|_| invalid("journal projection JSON"))?;
        }
        Key::CheckpointEpoch => {
            parse_epoch(bytes)?;
        }
    }
    Ok(())
}

fn epoch_bytes(epoch: u64) -> Vec<u8> {
    let mut bytes = EPOCH_MAGIC.to_vec();
    bytes.extend_from_slice(&epoch.to_le_bytes());
    bytes
}
fn parse_epoch(bytes: &[u8]) -> io::Result<u64> {
    if bytes.len() != 16 || &bytes[..8] != EPOCH_MAGIC {
        return Err(invalid("checkpoint epoch schema"));
    }
    let epoch = u64::from_le_bytes(bytes[8..].try_into().unwrap());
    if epoch == 0 {
        return Err(invalid("zero persisted checkpoint epoch"));
    }
    Ok(epoch)
}
#[doc(hidden)]
pub fn read_epoch(root: &Path) -> io::Result<u64> {
    read_file(&Key::CheckpointEpoch.path(root)?, 16)?
        .as_deref()
        .map(parse_epoch)
        .transpose()
        .map(|n| n.unwrap_or(0))
}
#[doc(hidden)]
pub fn next_epoch(root: &Path) -> io::Result<u64> {
    read_epoch(root)?
        .checked_add(1)
        .ok_or_else(|| invalid("checkpoint epoch exhausted"))
}
#[doc(hidden)]
pub fn checkpoint_image(epoch: u64) -> Image {
    Image {
        key: Key::CheckpointEpoch,
        after: Some(epoch_bytes(epoch)),
    }
}
fn journal_path(root: &Path) -> io::Result<PathBuf> {
    let path = root.join("data").join(NAME);
    checked_path(root, &path)?;
    checked_path(root, &path.with_file_name(format!(".{NAME}.durable-new")))?;
    Ok(path)
}

impl Prepared {
    /// Capture after all ordinary save callbacks and previous writer barriers. No core write occurs here.
    /// Revalidation of the live World/BE/attachment/pose witnesses belongs immediately before `publish`.
    pub fn capture(
        root: &Path,
        epoch: u64,
        mut images: Vec<Image>,
        limits: Limits,
    ) -> io::Result<Self> {
        let limits = limits.check()?;
        if epoch == 0 || images.is_empty() || images.len() > limits.records as usize {
            return Err(invalid("journal record count/epoch"));
        }
        if fs::symlink_metadata(journal_path(root)?).is_ok() {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "live-world recovery pending",
            ));
        }
        if next_epoch(root)? != epoch {
            return Err(invalid("checkpoint epoch is not next"));
        }
        if images
            .iter()
            .filter(|i| matches!(i.key, Key::CheckpointEpoch))
            .count()
            != 1
            || !images.iter().any(|i| {
                matches!(i.key, Key::CheckpointEpoch)
                    && i.after.as_deref() == Some(epoch_bytes(epoch).as_slice())
            })
        {
            return Err(invalid("checkpoint epoch image missing/mismatched"));
        }
        images.sort_by(|a, b| a.key.cmp(&b.key));
        let regions = RegionWitness::capture_all(root, &images, limits)?;
        let mut changes = Vec::with_capacity(images.len());
        let mut levels = BTreeSet::new();
        for image in images {
            if changes
                .last()
                .is_some_and(|previous: &Change| previous.image.key == image.key)
            {
                return Err(invalid("duplicate journal destination"));
            }
            image.key.path(root)?;
            if !matches!(image.key, Key::CheckpointEpoch | Key::PlayerData { .. }) {
                levels.insert(image.key.level().to_owned());
            }
            if levels.len() > limits.levels as usize {
                return Err(invalid("journal level budget"));
            }
            validate_image(&image, limits)?;
            let before = read_image(root, &image.key, limits)?;
            changes.push(Change {
                before: signature(before.as_deref()),
                image,
            });
        }
        let prepared = Self {
            epoch,
            changes,
            regions,
            limits,
        };
        // Count before allocating an envelope. Capturing a decision does not need a second
        // full journal buffer, and a refusal can report the exact complete required size.
        prepared.check_usage(prepared.encoded_usage()?)?;
        Ok(prepared)
    }

    pub fn images(&self) -> impl Iterator<Item = &Image> {
        self.changes.iter().map(|change| &change.image)
    }

    /// The durable ownership/save decision. Existing or corrupt pending work is never replaced.
    pub fn publish(&self, root: &Path) -> io::Result<()> {
        let path = journal_path(root)?;
        match fs::symlink_metadata(&path) {
            Ok(_) => {
                return Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    "live-world recovery pending",
                ));
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        // No earlier/later core writer may interleave between this check and publication.
        self.preflight_disk(root, false)?;
        ensure_parents(root, &path)?;
        durable::write(&path, &self.encode()?)
    }

    /// Prove that a SaveDir's exact frozen images are part of this durable Prepared decision.
    pub fn covers(&self, root: &Path, images: &[Image]) -> io::Result<()> {
        let pending = read_file(&journal_path(root)?, self.limits.bytes)?
            .ok_or_else(|| invalid("prepared journal missing"))?;
        if pending != self.encode()? {
            return Err(invalid("prepared journal changed"));
        }
        for image in images {
            let index = self
                .changes
                .binary_search_by(|change| change.image.key.cmp(&image.key))
                .map_err(|_| invalid("frozen image is outside Prepared"))?;
            if self.changes[index].image.after != image.after {
                return Err(invalid("frozen image is outside Prepared"));
            }
        }
        Ok(())
    }

    /// Exact frozen player files; all callbacks and live owner selection already happened in Game.
    /// Call after every level's ordinary writer, before the root epoch and common acknowledgment.
    pub fn write_player_data(&self, root: &Path) -> io::Result<()> {
        self.covers(root, &[])?;
        for change in &self.changes {
            if matches!(change.image.key, Key::PlayerData { .. }) {
                apply_image(root, &change.image, self.limits)?;
            }
        }
        Ok(())
    }

    /// Call after every level's ordinary writer and metadata are durable, before `acknowledge`.
    pub fn write_checkpoint_epoch(&self, root: &Path) -> io::Result<()> {
        let pending = read_file(&journal_path(root)?, self.limits.bytes)?
            .ok_or_else(|| invalid("prepared journal missing"))?;
        if pending != self.encode()? {
            return Err(invalid("prepared journal changed"));
        }
        apply_image(root, &checkpoint_image(self.epoch), self.limits)
    }

    /// The normal SaveDir writer has acknowledged every frozen afterimage. Never call on a failed write.
    pub fn acknowledge(&self, root: &Path) -> io::Result<()> {
        let images: Vec<&Image> = self.images().collect();
        for region in &self.regions {
            region.verify_after_header(root, &images)?;
            region.preflight(root, self.limits, false)?;
        }
        for change in &self.changes {
            let actual = read_image(root, &change.image.key, self.limits)?;
            if signature(actual.as_deref()) != signature(change.image.after.as_deref()) {
                return Err(invalid("journal afterimage not durable"));
            }
        }
        let path = journal_path(root)?;
        let pending = read_file(&path, self.limits.bytes)?
            .ok_or_else(|| invalid("prepared journal missing"))?;
        if pending != self.encode()? {
            return Err(invalid("prepared journal changed"));
        }
        durable::remove(&path)
    }

    fn preflight_disk(&self, root: &Path, allow_after: bool) -> io::Result<()> {
        // One whole-batch read pass before the first write. A selected region payload may be torn
        // after Prepared; its authoritative bytes are in this WAL. Verify every unselected occupied
        // sector/external stream independently of the possibly torn current allocation header.
        for region in &self.regions {
            region.preflight(root, self.limits, !allow_after)?;
        }
        // Validate the complete batch before changing even one owner, including a partial prior application.
        for change in &self.changes {
            if allow_after && matches!(change.image.key, Key::Record { .. }) {
                continue;
            }
            let actual = read_image(root, &change.image.key, self.limits)?;
            let actual = signature(actual.as_deref());
            if actual != change.before
                && (!allow_after || actual != signature(change.image.after.as_deref()))
            {
                return Err(invalid("journal destination diverged"));
            }
        }
        Ok(())
    }

    pub fn encoded_usage(&self) -> io::Result<Usage> {
        let mut usage = Usage {
            serialized_bytes: (MAGIC.len() + 8 + 4 + 32) as u64,
            records: self.changes.len() as u32,
            witness_bytes: 4,
            ..Usage::default()
        };
        let mut levels = BTreeSet::new();
        for change in &self.changes {
            let level = change.image.key.level();
            if !matches!(
                change.image.key,
                Key::CheckpointEpoch | Key::PlayerData { .. }
            ) {
                levels.insert(level);
            }
            let key_bytes = match change.image.key {
                Key::Record { folder, .. } => {
                    match folder {
                        Folder::Region => usage.terrain_records += 1,
                        Folder::Entities => usage.entity_records += 1,
                        Folder::Poi => usage.poi_records += 1,
                    }
                    1 + 8
                }
                Key::PlayerData { .. } => {
                    usage.metadata_records += 1;
                    1 + 16
                }
                _ => {
                    usage.metadata_records += 1;
                    1
                }
            };
            let payload = change
                .image
                .after
                .as_ref()
                .map_or(0, |bytes| bytes.len() as u64);
            usage.payload_bytes = usage
                .payload_bytes
                .checked_add(payload)
                .ok_or_else(|| invalid("journal payload size overflow"))?;
            let bytes = 2u64
                .checked_add(level.len() as u64)
                .and_then(|n| {
                    n.checked_add(key_bytes + 1 + if change.before.is_some() { 32 } else { 0 })
                })
                .and_then(|n| n.checked_add(1 + if change.image.after.is_some() { 4 } else { 0 }))
                .and_then(|n| n.checked_add(payload))
                .ok_or_else(|| invalid("journal record size overflow"))?;
            usage.serialized_bytes = usage
                .serialized_bytes
                .checked_add(bytes)
                .ok_or_else(|| invalid("journal size overflow"))?;
        }
        usage.levels = levels.len() as u32;
        for region in &self.regions {
            usage.witness_bytes = usage
                .witness_bytes
                .checked_add(region.encoded_bytes()?)
                .ok_or_else(|| invalid("journal witness size overflow"))?;
        }
        usage.serialized_bytes = usage
            .serialized_bytes
            .checked_add(usage.witness_bytes)
            .ok_or_else(|| invalid("journal size overflow"))?;
        Ok(usage)
    }

    fn check_usage(&self, usage: Usage) -> io::Result<()> {
        if usage.serialized_bytes > self.limits.bytes {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                ByteBudgetExceeded {
                    usage,
                    limit: self.limits.bytes,
                },
            ));
        }
        Ok(())
    }

    fn encode(&self) -> io::Result<Vec<u8>> {
        let usage = self.encoded_usage()?;
        self.check_usage(usage)?;
        let mut out = Vec::new();
        out.try_reserve_exact(
            usize::try_from(usage.serialized_bytes)
                .map_err(|_| invalid("journal envelope address-space budget"))?,
        )
        .map_err(|_| io::Error::new(io::ErrorKind::OutOfMemory, "journal envelope allocation"))?;
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&self.epoch.to_le_bytes());
        out.extend_from_slice(&(self.changes.len() as u32).to_le_bytes());
        for change in &self.changes {
            let level = change.image.key.level();
            out.extend_from_slice(&(level.len() as u16).to_le_bytes());
            out.extend_from_slice(level.as_bytes());
            match &change.image.key {
                Key::Record { folder, chunk, .. } => {
                    out.push(match folder {
                        Folder::Region => 0,
                        Folder::Entities => 1,
                        Folder::Poi => 2,
                    });
                    out.extend_from_slice(&chunk[0].to_le_bytes());
                    out.extend_from_slice(&chunk[1].to_le_bytes());
                }
                Key::Attachments { .. } => out.push(3),
                Key::Projection { .. } => out.push(4),
                Key::ProjectionOriginal { .. } => out.push(7),
                Key::CheckpointEpoch => out.push(5),
                Key::PlayerData { uuid } => {
                    out.push(6);
                    out.extend_from_slice(&uuid.to_le_bytes());
                }
            }
            out.push(u8::from(change.before.is_some()));
            if let Some(before) = change.before {
                out.extend_from_slice(&before);
            }
            match &change.image.after {
                Some(bytes) => {
                    out.push(1);
                    out.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
                    out.extend_from_slice(bytes);
                }
                None => out.push(0),
            }
            if out.len() as u64 + 32 > self.limits.bytes {
                return Err(invalid("journal byte budget"));
            }
        }
        out.extend_from_slice(&(self.regions.len() as u32).to_le_bytes());
        for region in &self.regions {
            region.encode(&mut out, self.limits)?;
        }
        if out.len() as u64 + 32 > self.limits.bytes {
            return Err(invalid("journal witness byte budget"));
        }
        out.extend_from_slice(&digest(&out));
        if out.len() as u64 != usage.serialized_bytes {
            return Err(invalid("journal size accounting diverged"));
        }
        Ok(out)
    }

    fn decode(bytes: &[u8], limits: Limits) -> io::Result<Self> {
        let limits = limits.check()?;
        if bytes.len() < 52 || bytes.len() as u64 > limits.bytes {
            return Err(invalid("journal envelope budget"));
        }
        let (body, checksum) = bytes.split_at(bytes.len() - 32);
        if digest(body).as_slice() != checksum {
            return Err(invalid("journal checksum"));
        }
        let mut input = Input(body);
        if input.take(8)? != MAGIC {
            return Err(invalid("journal schema"));
        }
        let epoch = u64::from_le_bytes(input.array()?);
        let count = u32::from_le_bytes(input.array()?) as usize;
        if epoch == 0 || count == 0 || count > limits.records as usize {
            return Err(invalid("journal record count/epoch"));
        }
        let mut changes = Vec::with_capacity(count);
        let mut levels = BTreeSet::new();
        for _ in 0..count {
            let length = u16::from_le_bytes(input.array()?) as usize;
            if length > 512 {
                return Err(invalid("journal level size"));
            }
            let level = std::str::from_utf8(input.take(length)?)
                .map_err(|_| invalid("journal level UTF-8"))?
                .to_owned();
            if !level.is_empty() && !valid_level_id(&level) {
                return Err(invalid("journal level id"));
            }
            if !level.is_empty() {
                levels.insert(level.clone());
            }
            if levels.len() > limits.levels as usize {
                return Err(invalid("journal level budget"));
            }
            let key = match input.byte()? {
                value @ 0..=2 => Key::Record {
                    level,
                    folder: match value {
                        0 => Folder::Region,
                        1 => Folder::Entities,
                        _ => Folder::Poi,
                    },
                    chunk: [
                        i32::from_le_bytes(input.array()?),
                        i32::from_le_bytes(input.array()?),
                    ],
                },
                3 => Key::Attachments { level },
                4 => Key::Projection { level },
                7 => Key::ProjectionOriginal { level },
                5 if level.is_empty() => Key::CheckpointEpoch,
                6 if level.is_empty() => Key::PlayerData {
                    uuid: u128::from_le_bytes(input.array()?),
                },
                _ => return Err(invalid("journal destination kind")),
            };
            if !matches!(key, Key::CheckpointEpoch | Key::PlayerData { .. })
                && key.level().is_empty()
            {
                return Err(invalid("empty journal level"));
            }
            if changes
                .last()
                .is_some_and(|previous: &Change| previous.image.key >= key)
            {
                return Err(invalid("journal destinations not unique/sorted"));
            }
            let before = match input.byte()? {
                0 => None,
                1 => Some(input.array()?),
                _ => return Err(invalid("journal before presence")),
            };
            let after = match input.byte()? {
                0 => None,
                1 => {
                    let length = u32::from_le_bytes(input.array()?) as usize;
                    if length > limits.record_bytes as usize {
                        return Err(invalid("journal record budget"));
                    }
                    Some(input.take(length)?.to_vec())
                }
                _ => return Err(invalid("journal after presence")),
            };
            let image = Image { key, after };
            validate_image(&image, limits)?;
            changes.push(Change { image, before });
        }
        let images: Vec<&Image> = changes.iter().map(|c| &c.image).collect();
        let regions = RegionWitness::decode_all(&mut input, &images, limits)?;
        if changes
            .iter()
            .filter(|c| matches!(c.image.key, Key::CheckpointEpoch))
            .count()
            != 1
            || !changes.iter().any(|c| {
                matches!(c.image.key, Key::CheckpointEpoch)
                    && c.image.after.as_deref() == Some(epoch_bytes(epoch).as_slice())
            })
        {
            return Err(invalid("checkpoint epoch image missing/mismatched"));
        }
        if !input.0.is_empty() {
            return Err(invalid("journal trailing bytes"));
        }
        Ok(Self {
            epoch,
            changes,
            regions,
            limits,
        })
    }
}

struct Input<'a>(&'a [u8]);
impl<'a> Input<'a> {
    fn take(&mut self, length: usize) -> io::Result<&'a [u8]> {
        if length > self.0.len() {
            return Err(invalid("truncated journal"));
        }
        let (head, tail) = self.0.split_at(length);
        self.0 = tail;
        Ok(head)
    }
    fn array<const N: usize>(&mut self) -> io::Result<[u8; N]> {
        Ok(self.take(N)?.try_into().unwrap())
    }
    fn byte(&mut self) -> io::Result<u8> {
        Ok(self.take(1)?[0])
    }
}

fn sync_directory(path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    fs::File::open(path)?.sync_all()?;
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}
pub(super) fn ensure_parents(root: &Path, path: &Path) -> io::Result<()> {
    checked_path(root, path)?;
    let parent = path.parent().ok_or_else(|| invalid("journal parent"))?;
    let mut current = root.to_owned();
    for component in parent
        .strip_prefix(root)
        .map_err(|_| invalid("journal parent outside world"))?
        .components()
    {
        current.push(component);
        match fs::create_dir(&current) {
            Ok(()) => sync_directory(current.parent().unwrap())?,
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists && current.is_dir() => {}
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

/// Only recovery before level publication may use direct region writes. Normal commits keep SaveDir's caches
/// and pending readers coherent by submitting these exact tags through its existing writer, without callbacks.
fn apply_image(root: &Path, image: &Image, limits: Limits) -> io::Result<()> {
    let path = image.key.path(root)?;
    ensure_parents(root, &path)?;
    match &image.key {
        Key::Record { chunk, .. } => {
            let mut region = RegionFile::open_writable(&path, true)?;
            let chunk = ChunkPos::new(chunk[0], chunk[1]);
            match &image.after {
                Some(bytes) => region.write(
                    chunk,
                    &ChunkRecord::from_nbt(
                        &named(bytes, limits)?,
                        Compression::Zlib,
                        0,
                        limits.nbt(),
                    )?,
                )?,
                None => {
                    region.clear(chunk, 0)?;
                }
            }
            region.sync()?;
            sync_directory(path.parent().unwrap())
        }
        _ => match &image.after {
            Some(bytes) => durable::write(&path, bytes),
            None => durable::remove(&path),
        },
    }
}

/// Recover before any level/chunk/attachment reads, runtime ownership publication, or player joins.
/// Corrupt or divergent data refuses open, preserving the prepared decision for explicit investigation.
#[doc(hidden)]
pub fn recover(root: &Path, limits: Limits) -> io::Result<bool> {
    let path = journal_path(root)?;
    let Some(bytes) = read_file(&path, limits.check()?.bytes)? else {
        return Ok(false);
    };
    let prepared = Prepared::decode(&bytes, limits)?;
    prepared.preflight_disk(root, true)?;
    // Restore ALL allocation maps before opening any forgiving RegionFile reader/writer. Then replay ALL
    // selected records: original selected sectors may already have been reused by another selected record.
    for region in &prepared.regions {
        region.restore_allocation(root)?;
    }
    for change in &prepared.changes {
        apply_image(root, &change.image, limits)?;
    }
    prepared.acknowledge(root)?;
    Ok(true)
}

/// Fixed level metadata destinations, through the same checked world-root writer used by recovery.
/// A SaveDir for a genuinely empty new level need not have created its dimension directory yet.
pub(super) fn write_level_metadata(root: &Path, image: &Image) -> io::Result<()> {
    if !matches!(
        image.key,
        Key::Attachments { .. } | Key::Projection { .. } | Key::ProjectionOriginal { .. }
    ) {
        return Err(invalid("not a level metadata destination"));
    }
    apply_image(root, image, Limits::default())
}

/// Read this typed metadata destination without following symlink components or creating files.
#[doc(hidden)]
pub fn read_projection(root: &Path, level: &str) -> io::Result<Option<Vec<u8>>> {
    read_image(
        root,
        &Key::Projection {
            level: level.to_owned(),
        },
        Limits::default(),
    )
}

/// Read the retained exact descriptor for an explicit development migration.
#[doc(hidden)]
pub fn read_projection_original(root: &Path, level: &str) -> io::Result<Option<Vec<u8>>> {
    read_image(
        root,
        &Key::ProjectionOriginal {
            level: level.to_owned(),
        },
        Limits::default(),
    )
}
impl Prepared {
    /// Write only the migration-original images selected by this already durable decision.
    #[doc(hidden)]
    pub fn write_projection_originals(&self, root: &Path) -> io::Result<()> {
        for change in &self.changes {
            if matches!(change.image.key, Key::ProjectionOriginal { .. }) {
                apply_image(root, &change.image, Limits::default())?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "live_world_journal_tests.rs"]
mod tests;
