//! Negotiated live-world projection metadata. Blocks and block entities use the existing
//! real viewed-level chunk/update protocol; this channel carries only membership and poses.
use crate::{Error, Result, buf::Reader};
use std::collections::BTreeSet;
use sunrise_core::{geometry::oriented, level_id::valid_level_id, vanilla_bytes::Writer};

pub const CHANNEL: &str = "sunrise:live_worlds";
pub const VERSION: &str = "1";
pub const MAX_CELLS: usize = 65536;
pub const MAX_MESSAGE: usize = 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pose {
    pub translation: [f64; 3],
    pub pivot: [f64; 3],
    pub rotation: [f64; 4],
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sample {
    pub frame: u64,
    pub source_revision: u64,
    /// Witness of sorted real member positions and canonical states.
    pub state_hash: [u8; 32],
    pub previous: Pose,
    pub current: Pose,
    pub revision: u64,
}
#[derive(Clone, Debug, PartialEq)]
pub enum Message {
    Snapshot {
        id: u64,
        collider: u32,
        boxes: u32,
        source: String,
        destination: String,
        cells: Vec<[i32; 3]>,
        membership: u64,
        sample: Sample,
    },
    Pose {
        id: u64,
        sample: Sample,
    },
    Clear {
        id: u64,
    },
}

fn pose_ok(p: Pose) -> bool {
    p.translation
        .iter()
        .chain(&p.pivot)
        .all(|v| v.is_finite() && v.abs() < 30_000_000.0)
        && oriented::check_rotation(p.rotation)
}
fn write_pose(w: &mut Writer, p: Pose) {
    for v in p.translation.into_iter().chain(p.pivot).chain(p.rotation) {
        w.f64(v);
    }
}
fn read_pose(r: &mut Reader<'_>) -> Result<Pose> {
    let mut values = [0.0; 10];
    for v in &mut values {
        *v = r.f64()?;
    }
    let p = Pose {
        translation: values[..3].try_into().unwrap(),
        pivot: values[3..6].try_into().unwrap(),
        rotation: values[6..].try_into().unwrap(),
    };
    if !pose_ok(p) {
        return Err(Error::new("live world: invalid pose"));
    }
    Ok(p)
}
fn write_sample(w: &mut Writer, s: Sample) {
    w.i64(s.frame as i64);
    w.i64(s.source_revision as i64);
    w.bytes(&s.state_hash);
    w.i64(s.revision as i64);
    write_pose(w, s.previous);
    write_pose(w, s.current);
}
fn read_sample(r: &mut Reader<'_>) -> Result<Sample> {
    let frame = r.i64()? as u64;
    let source_revision = r.i64()? as u64;
    let state_hash = r.bytes(32)?.try_into().unwrap();
    let revision = r.i64()? as u64;
    let previous = read_pose(r)?;
    let current = read_pose(r)?;
    Ok(Sample {
        frame,
        source_revision,
        state_hash,
        previous,
        current,
        revision,
    })
}

pub fn encode(message: &Message) -> Result<Vec<u8>> {
    let mut w = Writer::new();
    match message {
        Message::Snapshot {
            id,
            collider,
            boxes,
            source,
            destination,
            cells,
            membership,
            sample,
        } => {
            if *id == 0
                || *collider == 0
                || *boxes == 0
                || *boxes > 262144
                || !valid_level_id(source)
                || !valid_level_id(destination)
                || source == destination
                || cells.len() > MAX_CELLS
                || cells.windows(2).any(|p| p[0] >= p[1])
                || cells
                    .iter()
                    .flatten()
                    .any(|v| !(-29_999_999..=29_999_999).contains(v))
                || !pose_ok(sample.previous)
                || !pose_ok(sample.current)
            {
                return Err(Error::new("live world: invalid snapshot"));
            }
            w.u8(0);
            w.i64(*id as i64);
            w.u32(*collider);
            w.u32(*boxes);
            w.string(source);
            w.string(destination);
            w.i64(*membership as i64);
            write_sample(&mut w, *sample);
            w.varint(cells.len() as i32);
            for cell in cells {
                for &axis in cell {
                    w.i32(axis);
                }
            }
        }
        Message::Pose { id, sample } => {
            if *id == 0 || !pose_ok(sample.previous) || !pose_ok(sample.current) {
                return Err(Error::new("live world: invalid sample"));
            }
            w.u8(1);
            w.i64(*id as i64);
            write_sample(&mut w, *sample);
        }
        Message::Clear { id } => {
            if *id == 0 {
                return Err(Error::new("live world: invalid identity"));
            }
            w.u8(2);
            w.i64(*id as i64);
        }
    }
    let bytes = w.data;
    if bytes.len() > MAX_MESSAGE {
        return Err(Error::new("live world: payload budget"));
    }
    Ok(bytes)
}

pub fn decode(bytes: &[u8]) -> Result<Message> {
    if bytes.len() > MAX_MESSAGE {
        return Err(Error::new("live world: payload budget"));
    }
    let mut r = Reader::new(bytes);
    let kind = r.u8()?;
    let id = r.i64()? as u64;
    if id == 0 {
        return Err(Error::new("live world: invalid identity"));
    }
    let message = match kind {
        0 => {
            let collider = r.u32()?;
            let boxes = r.u32()?;
            let source = r.string(256)?;
            let destination = r.string(256)?;
            if collider == 0
                || boxes == 0
                || boxes > 262144
                || !valid_level_id(&source)
                || !valid_level_id(&destination)
                || source == destination
            {
                return Err(Error::new("live world: invalid levels"));
            }
            let membership = r.i64()? as u64;
            let sample = read_sample(&mut r)?;
            let count = r.varint()?;
            if count < 0
                || count as usize > MAX_CELLS
                || (count as usize)
                    .checked_mul(12)
                    .is_none_or(|size| size > r.remaining())
            {
                return Err(Error::new("live world: cell budget"));
            }
            let mut cells = Vec::with_capacity(count as usize);
            let mut chunks = BTreeSet::new();
            for _ in 0..count {
                let cell = [r.i32()?, r.i32()?, r.i32()?];
                if cell.iter().any(|v| !(-29_999_999..=29_999_999).contains(v))
                    || cells.last().is_some_and(|previous| *previous >= cell)
                {
                    return Err(Error::new("live world: invalid membership"));
                }
                chunks.insert((cell[0] >> 4, cell[2] >> 4));
                if chunks.len() > 4096 {
                    return Err(Error::new("live world: chunk budget"));
                }
                cells.push(cell);
            }
            Message::Snapshot {
                id,
                collider,
                boxes,
                source,
                destination,
                cells,
                membership,
                sample,
            }
        }
        1 => Message::Pose {
            id,
            sample: read_sample(&mut r)?,
        },
        2 => Message::Clear { id },
        _ => return Err(Error::new("live world: unknown message")),
    };
    if r.remaining() != 0 {
        return Err(Error::new("live world: trailing bytes"));
    }
    Ok(message)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sparse_snapshot_keeps_absolute_pose_and_refuses_duplicate_or_truncated_cells() {
        let p = Pose {
            translation: [10.0, 64.0, 10.0],
            pivot: [0.0; 3],
            rotation: [0.0, 0.0, 0.0, 1.0],
        };
        let sample = Sample {
            frame: 3,
            source_revision: 12,
            state_hash: [9; 32],
            previous: p,
            current: p,
            revision: 7,
        };
        let message = Message::Snapshot {
            id: 42,
            collider: 8,
            boxes: 32,
            source: "test:local".into(),
            destination: "minecraft:overworld".into(),
            cells: vec![[0, 0, 0], [17, 0, 0]],
            sample,
            membership: 0,
        };
        let bytes = encode(&message).unwrap();
        assert_eq!(decode(&bytes).unwrap(), message);
        assert!(decode(&bytes[..bytes.len() - 1]).is_err());
        let mut bad = message;
        let Message::Snapshot { cells, .. } = &mut bad else {
            unreachable!()
        };
        cells[1] = cells[0];
        assert!(encode(&bad).is_err());
    }
}
