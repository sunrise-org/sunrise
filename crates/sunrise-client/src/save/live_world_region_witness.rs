//! Original allocation maps and bounded witnesses of sectors the checkpoint must never change.
//! Selected payloads belong to Prepared and are always replayed; they are not a second live owner.
use super::*;
use std::io::{Seek, SeekFrom, Write};

const HEADER_BYTES: usize = 8192;
const SECTOR: u64 = 4096;

struct StreamWitness {
    bytes: u32,
    hash: [u8; 32],
}
struct Untouched {
    slot: u16,
    hash: [u8; 32],
    external: Option<StreamWitness>,
}
pub(super) struct RegionWitness {
    key: Key,
    existed: bool,
    length: u64,
    before: [u8; HEADER_BYTES],
    untouched: Vec<Untouched>,
}
fn region_key(key: &Key) -> Option<Key> {
    match key {
        Key::Record {
            level,
            folder,
            chunk,
        } => Some(Key::Record {
            level: level.clone(),
            folder: *folder,
            chunk: [chunk[0] & !31, chunk[1] & !31],
        }),
        _ => None,
    }
}
fn slot(key: &Key) -> usize {
    let Key::Record { chunk, .. } = key else {
        unreachable!()
    };
    chunk[0].rem_euclid(32) as usize + chunk[1].rem_euclid(32) as usize * 32
}
fn entry(header: &[u8; HEADER_BYTES], slot: usize) -> (u64, u64) {
    let value = u32::from_be_bytes(header[slot * 4..slot * 4 + 4].try_into().unwrap());
    (
        u64::from(value >> 8) * SECTOR,
        u64::from(value & 255) * SECTOR,
    )
}
fn selected_slots(images: &[&Image], key: &Key) -> BTreeSet<usize> {
    images
        .iter()
        .filter_map(|image| match (&image.key, key) {
            (
                Key::Record {
                    level,
                    folder,
                    chunk,
                },
                Key::Record {
                    level: target,
                    folder: target_folder,
                    chunk: base,
                },
            ) if level == target
                && folder == target_folder
                && chunk[0] & !31 == base[0]
                && chunk[1] & !31 == base[1] =>
            {
                Some(slot(&image.key))
            }
            _ => None,
        })
        .collect()
}

fn charge(total: &mut u64, bytes: u64, limits: Limits) -> io::Result<()> {
    *total = total
        .checked_add(bytes)
        .ok_or_else(|| invalid("region witness byte overflow"))?;
    if *total > limits.bytes {
        return Err(invalid("region witness scan budget"));
    }
    Ok(())
}
fn hash_range(file: &mut fs::File, offset: u64, bytes: u64) -> io::Result<[u8; 32]> {
    file.seek(SeekFrom::Start(offset))?;
    let mut remaining = bytes;
    let mut buffer = [0u8; 65536];
    let mut hash = Sha256::new();
    while remaining != 0 {
        let count = remaining.min(buffer.len() as u64) as usize;
        file.read_exact(&mut buffer[..count])?;
        hash.update(&buffer[..count]);
        remaining -= count as u64;
    }
    Ok(hash.finalize().into())
}
impl RegionWitness {
    pub fn encoded_bytes(&self) -> io::Result<u64> {
        // level length, folder/coordinates, existence/length, original header and slot count.
        let mut bytes = 2u64
            .checked_add(self.key.level().len() as u64)
            .and_then(|n| n.checked_add(1 + 8 + 1 + 8 + HEADER_BYTES as u64 + 2))
            .ok_or_else(|| invalid("journal witness size overflow"))?;
        for untouched in &self.untouched {
            bytes = bytes
                .checked_add(
                    2 + 32
                        + 1
                        + if untouched.external.is_some() {
                            4 + 32
                        } else {
                            0
                        },
                )
                .ok_or_else(|| invalid("journal witness size overflow"))?;
        }
        Ok(bytes)
    }

    fn validate(&self, images: &[&Image], limits: Limits) -> io::Result<()> {
        if region_key(&self.key).as_ref() != Some(&self.key) || self.length > limits.bytes {
            return Err(invalid("region witness key/length"));
        }
        if !self.existed && (self.length != 0 || self.before.iter().any(|b| *b != 0)) {
            return Err(invalid("absent region witness"));
        }
        if self.existed && self.length < HEADER_BYTES as u64 {
            return Err(invalid("short original region"));
        }
        let selected = selected_slots(images, &self.key);
        let mut ranges = Vec::new();
        let mut expected = Vec::new();
        for slot in 0..1024 {
            let (offset, bytes) = entry(&self.before, slot);
            if offset == 0 && bytes == 0 {
                continue;
            }
            if offset < HEADER_BYTES as u64 || bytes == 0 || offset + bytes > self.length {
                return Err(invalid("invalid original region allocation"));
            }
            ranges.push((offset, offset + bytes));
            if !selected.contains(&slot) {
                expected.push(slot as u16);
            }
        }
        ranges.sort_unstable();
        if ranges.windows(2).any(|pair| pair[0].1 > pair[1].0) {
            return Err(invalid("overlapping original allocations"));
        }
        if expected != self.untouched.iter().map(|u| u.slot).collect::<Vec<_>>() {
            return Err(invalid("unselected region witness coverage"));
        }
        if self.untouched.iter().any(|u| {
            u.external
                .as_ref()
                .is_some_and(|e| e.bytes > limits.record_bytes)
        }) {
            return Err(invalid("unselected external stream budget"));
        }
        Ok(())
    }
    fn external_path(&self, root: &Path, slot: usize) -> io::Result<PathBuf> {
        let Key::Record {
            level,
            folder,
            chunk,
        } = &self.key
        else {
            unreachable!()
        };
        let key = Key::Record {
            level: level.clone(),
            folder: *folder,
            chunk: [chunk[0] + (slot % 32) as i32, chunk[1] + (slot / 32) as i32],
        };
        let path = key.path(root)?.parent().unwrap().join(format!(
            "c.{}.{}.mcc",
            chunk[0] + (slot % 32) as i32,
            chunk[1] + (slot / 32) as i32
        ));
        checked_path(root, &path)?;
        Ok(path)
    }
    pub fn capture_all(root: &Path, images: &[Image], limits: Limits) -> io::Result<Vec<Self>> {
        let refs: Vec<&Image> = images.iter().collect();
        let keys: BTreeSet<_> = images.iter().filter_map(|i| region_key(&i.key)).collect();
        let mut total = 0;
        let mut result = Vec::with_capacity(keys.len());
        for key in keys {
            charge(&mut total, HEADER_BYTES as u64, limits)?;
            let path = key.path(root)?;
            let mut witness = Self {
                key,
                existed: false,
                length: 0,
                before: [0; HEADER_BYTES],
                untouched: Vec::new(),
            };
            match fs::File::open(&path) {
                Ok(mut file) => {
                    witness.existed = true;
                    witness.length = file.metadata()?.len();
                    if witness.length > limits.bytes {
                        return Err(invalid("original region length budget"));
                    }
                    file.read_exact(&mut witness.before)?;
                    // Validate allocation bounds/overlap before any allocation or sector seek.
                    let selected = selected_slots(&refs, &witness.key);
                    for slot in 0..1024 {
                        let (_, bytes) = entry(&witness.before, slot);
                        if bytes != 0 && !selected.contains(&slot) {
                            witness.untouched.push(Untouched {
                                slot: slot as u16,
                                hash: [0; 32],
                                external: None,
                            });
                        }
                    }
                    witness.validate(&refs, limits)?;
                    for index in 0..witness.untouched.len() {
                        let slot = witness.untouched[index].slot as usize;
                        let (offset, bytes) = entry(&witness.before, slot);
                        charge(&mut total, bytes, limits)?;
                        let hash = hash_range(&mut file, offset, bytes)?;
                        file.seek(SeekFrom::Start(offset + 4))?;
                        let mut compression = [0];
                        file.read_exact(&mut compression)?;
                        let external = if compression[0] & 128 != 0 {
                            let mut stream = fs::File::open(witness.external_path(root, slot)?)?;
                            let length = stream.metadata()?.len();
                            if length > u64::from(limits.record_bytes) {
                                return Err(invalid("unselected external stream budget"));
                            }
                            charge(&mut total, length, limits)?;
                            Some(StreamWitness {
                                bytes: length as u32,
                                hash: hash_range(&mut stream, 0, length)?,
                            })
                        } else {
                            None
                        };
                        witness.untouched[index] = Untouched {
                            slot: slot as u16,
                            hash,
                            external,
                        };
                    }
                }
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(error),
            }
            witness.validate(&refs, limits)?;
            result.push(witness);
        }
        Ok(result)
    }
    pub fn preflight(&self, root: &Path, limits: Limits, original: bool) -> io::Result<()> {
        let path = self.key.path(root)?;
        let mut file = match fs::File::open(&path) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound && !self.existed => return Ok(()),
            Err(error) => return Err(error),
        };
        let length = file.metadata()?.len();
        let growth = limits
            .bytes
            .checked_add(u64::from(limits.records) * SECTOR)
            .ok_or_else(|| invalid("region growth budget"))?;
        if length > self.length + growth {
            return Err(invalid("recovery region length budget"));
        }
        if original {
            if !self.existed || length != self.length {
                return Err(invalid("original region changed"));
            }
            let mut header = [0; HEADER_BYTES];
            file.read_exact(&mut header)?;
            if header != self.before {
                return Err(invalid("original region header changed"));
            }
        }
        for untouched in &self.untouched {
            let (offset, bytes) = entry(&self.before, untouched.slot as usize);
            if hash_range(&mut file, offset, bytes)? != untouched.hash {
                return Err(invalid("unselected region sector diverged"));
            }
            file.seek(SeekFrom::Start(offset + 4))?;
            let mut compression = [0];
            file.read_exact(&mut compression)?;
            if (compression[0] & 128 != 0) != untouched.external.is_some() {
                return Err(invalid("unselected external witness mismatch"));
            }
            if let Some(expected) = &untouched.external {
                let mut stream =
                    fs::File::open(self.external_path(root, untouched.slot as usize)?)?;
                if stream.metadata()?.len() != u64::from(expected.bytes)
                    || hash_range(&mut stream, 0, u64::from(expected.bytes))? != expected.hash
                {
                    return Err(invalid("unselected external stream diverged"));
                }
            }
        }
        Ok(())
    }
    pub fn verify_after_header(&self, root: &Path, images: &[&Image]) -> io::Result<()> {
        let mut file = match fs::File::open(self.key.path(root)?) {
            Ok(file) => file,
            Err(error)
                if error.kind() == io::ErrorKind::NotFound
                    && !self.existed
                    && images
                        .iter()
                        .filter(|image| region_key(&image.key).as_ref() == Some(&self.key))
                        .all(|image| image.after.is_none()) =>
            {
                return Ok(());
            }
            Err(error) => return Err(error),
        };
        let mut header = [0; HEADER_BYTES];
        file.read_exact(&mut header)?;
        let selected = selected_slots(images, &self.key);
        for slot in 0..1024 {
            if selected.contains(&slot) {
                continue;
            }
            for base in [0, 4096] {
                if header[base + slot * 4..base + slot * 4 + 4]
                    != self.before[base + slot * 4..base + slot * 4 + 4]
                {
                    return Err(invalid("unselected region header diverged"));
                }
            }
        }
        Ok(())
    }
    pub fn restore_allocation(&self, root: &Path) -> io::Result<()> {
        let path = self.key.path(root)?;
        ensure_parents(root, &path)?;
        let mut file = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)?;
        file.seek(SeekFrom::Start(0))?;
        file.write_all(&self.before)?;
        file.sync_all()?;
        sync_directory(path.parent().unwrap())
    }
    pub fn encode(&self, out: &mut Vec<u8>, limits: Limits) -> io::Result<()> {
        let Key::Record {
            level,
            folder,
            chunk,
        } = &self.key
        else {
            unreachable!()
        };
        out.extend_from_slice(&(level.len() as u16).to_le_bytes());
        out.extend_from_slice(level.as_bytes());
        out.push(match folder {
            Folder::Region => 0,
            Folder::Entities => 1,
            Folder::Poi => 2,
        });
        for coordinate in chunk {
            out.extend_from_slice(&coordinate.to_le_bytes());
        }
        out.push(u8::from(self.existed));
        out.extend_from_slice(&self.length.to_le_bytes());
        out.extend_from_slice(&self.before);
        out.extend_from_slice(&(self.untouched.len() as u16).to_le_bytes());
        for u in &self.untouched {
            out.extend_from_slice(&u.slot.to_le_bytes());
            out.extend_from_slice(&u.hash);
            out.push(u8::from(u.external.is_some()));
            if let Some(e) = &u.external {
                out.extend_from_slice(&e.bytes.to_le_bytes());
                out.extend_from_slice(&e.hash);
            }
        }
        if out.len() as u64 + 32 > limits.bytes {
            return Err(invalid("journal region witness budget"));
        }
        Ok(())
    }
    pub fn decode_all(
        input: &mut Input<'_>,
        images: &[&Image],
        limits: Limits,
    ) -> io::Result<Vec<Self>> {
        let expected: Vec<Key> = images
            .iter()
            .filter_map(|i| region_key(&i.key))
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        let count = u32::from_le_bytes(input.array()?) as usize;
        if count != expected.len() {
            return Err(invalid("region witness coverage"));
        }
        let mut result = Vec::with_capacity(count);
        let mut scan = 0;
        for expected in expected {
            let size = u16::from_le_bytes(input.array()?) as usize;
            if size == 0 || size > 512 {
                return Err(invalid("region witness level size"));
            }
            let level = std::str::from_utf8(input.take(size)?)
                .map_err(|_| invalid("region level UTF-8"))?
                .to_owned();
            let folder = match input.byte()? {
                0 => Folder::Region,
                1 => Folder::Entities,
                2 => Folder::Poi,
                _ => return Err(invalid("region witness folder")),
            };
            let key = Key::Record {
                level,
                folder,
                chunk: [
                    i32::from_le_bytes(input.array()?),
                    i32::from_le_bytes(input.array()?),
                ],
            };
            if key != expected {
                return Err(invalid("region witness key mismatch"));
            }
            let existed = match input.byte()? {
                0 => false,
                1 => true,
                _ => return Err(invalid("region witness presence")),
            };
            let length = u64::from_le_bytes(input.array()?);
            let before = input.array()?;
            let count = u16::from_le_bytes(input.array()?) as usize;
            if count > 1024 {
                return Err(invalid("region untouched count"));
            }
            let mut untouched = Vec::with_capacity(count);
            for _ in 0..count {
                let slot = u16::from_le_bytes(input.array()?);
                if slot >= 1024 {
                    return Err(invalid("region untouched slot"));
                }
                let hash = input.array()?;
                let external = match input.byte()? {
                    0 => None,
                    1 => Some(StreamWitness {
                        bytes: u32::from_le_bytes(input.array()?),
                        hash: input.array()?,
                    }),
                    _ => return Err(invalid("region external presence")),
                };
                untouched.push(Untouched {
                    slot,
                    hash,
                    external,
                });
            }
            let witness = Self {
                key,
                existed,
                length,
                before,
                untouched,
            };
            witness.validate(images, limits)?;
            charge(&mut scan, HEADER_BYTES as u64, limits)?;
            for u in &witness.untouched {
                charge(&mut scan, entry(&witness.before, u.slot as usize).1, limits)?;
                if let Some(e) = &u.external {
                    charge(&mut scan, u64::from(e.bytes), limits)?;
                }
            }
            result.push(witness);
        }
        Ok(result)
    }
}
