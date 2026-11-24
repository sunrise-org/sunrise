//! Bounded engine action requests. Connection identity, level handles and permissions are server-owned.
use crate::{Error, Result, buf::Reader};
use sunrise_core::vanilla_bytes::Writer;

pub const CHANNEL: &str = "sunrise:live_world_actions";
pub const VERSION: &str = "1";
pub const MESSAGE_BYTES: usize = 99;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Use,
    StartDestroy,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Request {
    pub action: Action,
    pub generation: u64,
    pub membership: u64,
    pub pose_revision: u64,
    pub source_revision: u64,
    pub cell: [i32; 3],
    /// Direction.get3DDataValue, in the source frame.
    pub face: u8,
    pub point: [f64; 3],
    pub local_point: [f64; 3],
    pub hand: u8,
    pub sequence: i32,
}

fn valid(p: &Request) -> bool {
    p.generation != 0
        && p.face < 6
        && p.hand < 2
        && p.sequence >= 0
        && (p.action != Action::StartDestroy || p.hand == 0)
        && p.cell
            .iter()
            .all(|v| (-29_999_999..=29_999_999).contains(v))
        && p.point
            .iter()
            .chain(&p.local_point)
            .all(|v| v.is_finite() && v.abs() < 30_000_000.0)
}

pub fn encode(p: &Request) -> Result<Vec<u8>> {
    if !valid(p) {
        return Err(Error::new("live world action: invalid request"));
    }
    let mut w = Writer::new();
    w.u8(match p.action {
        Action::Use => 0,
        Action::StartDestroy => 1,
    });
    for v in [
        p.generation,
        p.membership,
        p.pose_revision,
        p.source_revision,
    ] {
        w.i64(v as i64);
    }
    for v in p.cell {
        w.i32(v);
    }
    w.u8(p.face);
    for v in p.point.into_iter().chain(p.local_point) {
        w.f64(v);
    }
    w.u8(p.hand);
    w.i32(p.sequence);
    Ok(w.data)
}

pub fn decode(bytes: &[u8]) -> Result<Request> {
    if bytes.len() != MESSAGE_BYTES {
        return Err(Error::new("live world action: wrong size"));
    }
    let mut r = Reader::new(bytes);
    let action = match r.u8()? {
        0 => Action::Use,
        1 => Action::StartDestroy,
        _ => return Err(Error::new("live world action: unknown action")),
    };
    let generation = r.i64()? as u64;
    let membership = r.i64()? as u64;
    let pose_revision = r.i64()? as u64;
    let source_revision = r.i64()? as u64;
    let cell = [r.i32()?, r.i32()?, r.i32()?];
    let face = r.u8()?;
    let point = [r.f64()?, r.f64()?, r.f64()?];
    let local_point = [r.f64()?, r.f64()?, r.f64()?];
    let hand = r.u8()?;
    let sequence = r.i32()?;
    let request = Request {
        action,
        generation,
        membership,
        pose_revision,
        source_revision,
        cell,
        face,
        point,
        local_point,
        hand,
        sequence,
    };
    if !valid(&request) {
        return Err(Error::new("live world action: invalid request"));
    }
    Ok(request)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn action_decoder_bounds_shape_and_rejects_non_finite_target() {
        // A literal request, independent of encode: Use, id 1, north face, main hand, sequence 7.
        let mut bytes = [0u8; MESSAGE_BYTES];
        bytes[8] = 1;
        bytes[45] = 2;
        bytes[98] = 7;
        let p = decode(&bytes).unwrap();
        assert_eq!(p.generation, 1);
        assert_eq!(p.action, Action::Use);
        assert_eq!(p.face, 2);
        assert_eq!(p.sequence, 7);
        assert_eq!(encode(&p).unwrap(), bytes);
        assert!(decode(&bytes[..98]).is_err());
        assert!(decode(&[bytes.as_slice(), &[0]].concat()).is_err());
        for (offset, bad) in [(0, 2), (45, 6), (94, 2)] {
            let mut invalid = bytes;
            invalid[offset] = bad;
            assert!(decode(&invalid).is_err());
        }
        bytes[46..54].copy_from_slice(&f64::NAN.to_be_bytes());
        assert!(decode(&bytes).is_err());
    }
}
