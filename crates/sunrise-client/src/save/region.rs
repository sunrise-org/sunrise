use super::*;

/// `RegionFileStorage.MAX_CACHE_SIZE`: the region files of a folder kept
/// open. Each holds a file descriptor: kept for every region ever visited,
/// a server whose players travel runs out of them (`Too many open files`).
pub(super) const MAX_OPEN_REGIONS: usize = 256;
/// A folder's region files, and the ones known to be missing, by region,
/// with when each was last asked for.
#[derive(Default)]
pub(super) struct RegionFiles {
    pub(super) regions: HashMap<(i32, i32), (Option<Arc<Mutex<RegionFile>>>, u64)>,
    clock: u64,
    /// Each region's write stamp (`far_regions::writing`).
    pub(super) written: HashMap<(i32, i32), u64>,
}
impl RegionFiles {
    #[cfg(test)]
    pub(super) fn is_empty(&self) -> bool {
        self.regions.is_empty()
    }
    /// The open region files.
    #[cfg(test)]
    pub(super) fn open(&self) -> usize {
        self.regions.values().filter(|(r, _)| r.is_some()).count()
    }
    /// Close the least recently used regions until there is room for one
    /// more, as `RegionFileStorage.getRegionFile` does. A region a reader or
    /// the writer still holds stays: closing it under them would let a
    /// second handle read a header older than its sectors.
    fn make_room(&mut self) {
        while self.regions.len() >= MAX_OPEN_REGIONS {
            let oldest = self
                .regions
                .iter()
                .filter(|(_, (r, _))| r.as_ref().is_none_or(|r| Arc::strong_count(r) == 1))
                .min_by_key(|(_, (_, used))| *used)
                .map(|(key, _)| *key);
            let Some(key) = oldest else {
                return;
            };
            self.regions.remove(&key);
        }
    }
}
/// The region file at `path` through the cache, which readers and the writer
/// share, so a read never sees a header older than the sectors it points to.
/// `create` makes a missing file (for a write); otherwise a missing file is
/// `None`, until a write creates it.
pub(super) fn cached_region(
    cache: &RegionCache,
    key: (i32, i32),
    path: &Path,
    create: bool,
) -> Result<Option<Arc<Mutex<RegionFile>>>> {
    let mut cache = cache.lock().map_err(|_| "region cache poisoned")?;
    cache.clock += 1;
    let now = cache.clock;
    if let Some((cached, used)) = cache.regions.get_mut(&key)
        && (cached.is_some() || !create)
    {
        *used = now;
        return Ok(cached.clone());
    }
    cache.make_room();
    if create && let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    // A file this process can't write (a read-only world) is still read.
    let region = match RegionFile::open_writable(path, create) {
        Err(e) if !create && e.kind() == std::io::ErrorKind::PermissionDenied => {
            RegionFile::open(path)
        }
        other => other,
    };
    let region = match region {
        Ok(r) => Some(Arc::new(Mutex::new(r))),
        Err(_) if !create => None,
        Err(e) => return Err(e.into()),
    };
    cache.regions.insert(key, (region.clone(), now));
    Ok(region)
}
/// Write `records` into the region files of `folder` in place, as vanilla's
/// `RegionFile.write` does: each chunk takes free sectors and the header
/// then points to it; the region's other chunks are not touched. `None`
/// clears a chunk. The chunks are encoded and compressed before their region
/// is locked for the writes (`compress_records`), and each region is flushed
/// to the disk once.
pub(super) fn write_region_records(
    root: &Path,
    cache: &RegionCache,
    folder: &str,
    records: &[((i32, i32), &Record)],
) -> Result<()> {
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as u32);
    let compressed = compress_records(records, timestamp);
    type RegionWrite<'a> = (ChunkPos, &'a Record, Option<CompressedWrite>);
    let mut by_region: BTreeMap<(i32, i32), Vec<RegionWrite<'_>>> = BTreeMap::new();
    for (&((x, z), source), record) in records.iter().zip(compressed) {
        let record = record?;
        by_region
            .entry((x >> 5, z >> 5))
            .or_default()
            .push((ChunkPos::new(x, z), source, record));
    }
    for ((rx, rz), list) in by_region {
        let path = region_path(root, folder, rx, rz);
        // Clearing chunks of a region that doesn't exist changes nothing.
        let create = list.iter().any(|(_, _, r)| r.is_some());
        let Some(region) = cached_region(cache, (rx, rz), &path, create)? else {
            continue;
        };
        // Keep far readers on the exact locked read path while writes and their
        // flush are in progress, so they cannot use a stale kept header.
        let _writing = far_regions::writing(cache, (rx, rz));
        let mut region = region.lock().map_err(|_| "region file poisoned")?;
        let mut changed = false;
        for (pos, source, record) in list {
            match record {
                Some(record) => {
                    write_chunk_record(&mut region, pos, source, record)?;
                    changed = true;
                }
                None => changed |= region.clear(pos, timestamp)?,
            }
        }
        if changed {
            match region.sync_handle() {
                Ok(handle) => {
                    drop(region);
                    handle.sync_data()?;
                }
                // Out of descriptors: flush under the lock as before rather than
                // skip durability for this region and every region after it.
                Err(_) => region.sync()?,
            }
        }
    }
    Ok(())
}

pub(super) fn write_chunk_record(
    region: &mut RegionFile,
    pos: ChunkPos,
    source: &Record,
    CompressedWrite { record, primary }: CompressedWrite,
) -> std::io::Result<()> {
    let error = match region.write(pos, &record) {
        Ok(()) => {
            if primary {
                source.accept_block_entity_edits();
            }
            return Ok(());
        }
        Err(error) => error,
    };
    if !primary
        || !error
            .get_ref()
            .is_some_and(|cause| cause.is::<sunrise_world::region::CompressedChunkLimit>())
    {
        return Err(error);
    }
    let (compression, timestamp) = (record.compression, record.timestamp);
    drop(record);
    let Some(stored) = source.without_block_entity_edits() else {
        return Err(error);
    };
    log::warn!(
        "block_entity_save_additional: discarding listener edits from chunk {},{}: {error}",
        pos.x,
        pos.z
    );
    let stored = ChunkRecord::from_nbt(&stored, compression, timestamp, Limits::saved())?;
    region.write(pos, &stored)
}

pub(super) fn region_path(root: &Path, folder: &str, rx: i32, rz: i32) -> PathBuf {
    root.join(format!("{folder}/r.{rx}.{rz}.mca"))
}

/// A queued POI record as it is written: claims kept from the saved one, and
/// a chunk that lost its last POI gets an empty one, as vanilla trusts a
/// saved section. None when the chunk never had any.
pub(super) fn resolve_poi(
    root: &Path,
    cache: &RegionCache,
    (x, z): (i32, i32),
    record: &Record,
) -> Option<Record> {
    let Record::Poi(chunk, tables, professions) = record else {
        return Some(record.clone());
    };
    let old = read_region_record(root, cache, "poi", x, z)
        .and_then(|old| {
            old.map(|mut old| {
                upgrade_record(Folder::Poi, &mut old, || None)?;
                Ok(old)
            })
            .transpose()
        })
        .unwrap_or_else(|e| {
            log::warn!("Sunrise: could not read the poi record of chunk {x} {z}: {e}");
            None
        });
    let tag = match (encode_poi(chunk, tables, professions.as_deref()), old) {
        (Some(mut tag), Some(old)) => {
            keep_claims(&mut tag, &old);
            tag
        }
        (Some(tag), None) => tag,
        (None, Some(_)) => poi_root(Compound::new()),
        (None, None) => return None,
    };
    Some(Record::Tag(Arc::new(tag)))
}
pub(super) fn read_region_record(
    root: &Path,
    cache: &RegionCache,
    folder: &str,
    x: i32,
    z: i32,
) -> Result<Option<NamedTag>> {
    let key = (x >> 5, z >> 5);
    let path = region_path(root, folder, key.0, key.1);
    let Some(region) = cached_region(cache, key, &path, false)? else {
        return Ok(None);
    };
    let record = region
        .lock()
        .map_err(|_| "region file poisoned")?
        .read(ChunkPos::new(x, z))?;
    Ok(match record {
        Some(r) => Some(r.nbt(Limits::saved())?),
        None => None,
    })
}
