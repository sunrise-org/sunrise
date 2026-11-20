//! Same-source persistent marker and caller receipt metadata, through the ordinary attachment API.
use mod_api::{
    AttachmentCodec, AttachmentError, AttachmentType, Compound, JavaString, LiveWorldPose,
    LiveWorldSample, Tag,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Initializing,
    LocalOwned,
    Projected,
    WorldOwned,
}
#[derive(Clone, Debug)]
pub struct Accepted {
    pub previous: LiveWorldPose,
    pub current: LiveWorldPose,
    pub revision: u64,
}
impl From<LiveWorldSample> for Accepted {
    fn from(value: LiveWorldSample) -> Self {
        Self {
            previous: value.previous,
            current: value.current,
            revision: value.revision,
        }
    }
}
#[derive(Clone, Debug)]
pub struct Record {
    pub phase: Phase,
    pub accepted: Option<Accepted>,
    pub planned_epoch: Option<u64>,
    pub transfer_epoch: Option<u64>,
    pub steps: u32,
}
impl Record {
    pub(super) fn unstarted_initialization(&self) -> bool {
        self.phase == Phase::Initializing
            && self.accepted.is_none()
            && self.planned_epoch.is_none()
            && self.transfer_epoch.is_none()
            && self.steps == 0
    }
    pub fn new(phase: Phase) -> Self {
        Self {
            phase,
            accepted: None,
            planned_epoch: None,
            transfer_epoch: None,
            steps: 0,
        }
    }
}
pub static OWNER: AttachmentType<Record> = AttachmentType {
    namespace: "live_world_fixture",
    name: "owner",
    default: || Record::new(Phase::Initializing),
    codec: Some(AttachmentCodec { encode, decode }),
    copy_on_death: false,
    sync: false,
};
fn pose_bits(p: LiveWorldPose) -> Vec<i64> {
    p.translation
        .into_iter()
        .chain(p.pivot)
        .chain(p.rotation)
        .map(|x| x.to_bits() as i64)
        .collect()
}
fn pose(bits: &[i64]) -> LiveWorldPose {
    LiveWorldPose {
        translation: std::array::from_fn(|i| f64::from_bits(bits[i] as u64)),
        pivot: std::array::from_fn(|i| f64::from_bits(bits[i + 3] as u64)),
        rotation: std::array::from_fn(|i| f64::from_bits(bits[i + 6] as u64)),
    }
}
fn epoch(value: Option<u64>) -> Tag {
    Tag::LongArray(value.into_iter().map(|v| v as i64).collect())
}
pub(super) fn encode(value: &Record) -> std::result::Result<Compound, AttachmentError> {
    let mut sample = Vec::new();
    if let Some(s) = &value.accepted {
        sample.extend(pose_bits(s.previous));
        sample.extend(pose_bits(s.current));
        sample.push(s.revision as i64);
    }
    Ok([
        ("version".into(), Tag::Int(1)),
        ("source".into(), Tag::String(super::SOURCE.into())),
        (
            "phase".into(),
            Tag::Byte(match value.phase {
                Phase::Initializing => 0,
                Phase::LocalOwned => 1,
                Phase::Projected => 2,
                Phase::WorldOwned => 3,
            }),
        ),
        ("accepted_ieee_bits".into(), Tag::LongArray(sample)),
        ("planned_epoch".into(), epoch(value.planned_epoch)),
        ("transfer_epoch".into(), epoch(value.transfer_epoch)),
        ("steps".into(), Tag::Int(value.steps as i32)),
    ]
    .into())
}
fn decode(value: &Compound) -> std::result::Result<Record, AttachmentError> {
    let bad = || AttachmentError::Decode;
    if !matches!(value.get(&"version".into()), Some(Tag::Int(1))) {
        return Err(bad());
    }
    if !matches!(value.get(&"source".into()), Some(Tag::String(id)) if id == &JavaString::from(super::SOURCE))
    {
        return Err(bad());
    }
    let phase = match value.get(&"phase".into()) {
        Some(Tag::Byte(0)) => Phase::Initializing,
        Some(Tag::Byte(1)) => Phase::LocalOwned,
        Some(Tag::Byte(2)) => Phase::Projected,
        Some(Tag::Byte(3)) => Phase::WorldOwned,
        _ => return Err(bad()),
    };
    let accepted = match value.get(&"accepted_ieee_bits".into()) {
        Some(Tag::LongArray(bits)) if bits.is_empty() => None,
        Some(Tag::LongArray(bits)) if bits.len() == 21 => Some(Accepted {
            previous: pose(&bits[..10]),
            current: pose(&bits[10..20]),
            revision: bits[20] as u64,
        }),
        _ => return Err(bad()),
    };
    if phase == Phase::Projected && accepted.is_none() {
        return Err(bad());
    }
    let read_epoch = |key: &str| match value.get(&key.into()) {
        Some(Tag::LongArray(bits)) if bits.is_empty() => Ok(None),
        Some(Tag::LongArray(bits)) if bits.len() == 1 => Ok(Some(bits[0] as u64)),
        _ => Err(bad()),
    };
    let steps = match value.get(&"steps".into()) {
        Some(Tag::Int(v)) if (0..=super::MOTION_STEPS as i32).contains(v) => *v as u32,
        _ => return Err(bad()),
    };
    Ok(Record {
        phase,
        accepted,
        planned_epoch: read_epoch("planned_epoch")?,
        transfer_epoch: read_epoch("transfer_epoch")?,
        steps,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn persisted_resume_authorization_refuses_every_started_or_other_ownership_record() {
        let empty = Record::new(Phase::Initializing);
        assert!(
            decode(&encode(&empty).unwrap())
                .unwrap()
                .unstarted_initialization()
        );
        let accepted = Accepted {
            previous: LiveWorldPose::at([1.0, 2.0, 3.0]),
            current: LiveWorldPose::at([4.0, 5.0, 6.0]),
            revision: 9,
        };
        let mut forbidden = Vec::new();
        for phase in [Phase::LocalOwned, Phase::WorldOwned] {
            forbidden.push(Record::new(phase));
        }
        let mut projected = Record::new(Phase::Projected);
        projected.accepted = Some(accepted.clone());
        forbidden.push(projected);
        let mut sample = empty.clone();
        sample.accepted = Some(accepted);
        forbidden.push(sample);
        let mut planned = empty.clone();
        planned.planned_epoch = Some(0); // Presence is meaningful, even a zero-valued epoch.
        forbidden.push(planned);
        let mut transferred = empty.clone();
        transferred.transfer_epoch = Some(0);
        forbidden.push(transferred);
        let mut moved = empty;
        moved.steps = 1;
        forbidden.push(moved);
        for record in forbidden {
            let saved = encode(&record).unwrap();
            let loaded = decode(&saved).unwrap();
            assert!(
                !loaded.unstarted_initialization(),
                "must refuse persisted {loaded:?}"
            );
            assert_eq!(
                encode(&loaded).unwrap(),
                saved,
                "inspection must preserve the actual audit record"
            );
        }
    }
}
