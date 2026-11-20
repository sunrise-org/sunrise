//! Actual filesystem and immutable existing Snapshot common-checkpoint tests.
use super::super::{Guest, player_tag, read_dat};
use super::*;
use crate::lan::{Joined, PlayerSender};
use std::sync::{
    atomic::{AtomicI32, AtomicU64},
    mpsc,
};
use sunrise_protocol::packets::configuration::ClientInformation;

struct Scratch(PathBuf);
impl Scratch {
    fn new(label: &str) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "sunrise-player-common-{label}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        Self(root)
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn guest(uuid: u128, count: u32) -> Guest {
    let (send, _receive) = mpsc::channel();
    let mut guest = Guest::new(
        Joined {
            conn: uuid as u32,
            uuid,
            name: format!("Player{uuid}"),
            properties: Vec::new(),
            info: ClientInformation::new(8),
            brand: "test".into(),
            sender: PlayerSender::new(send, Arc::default()),
            latency: Arc::new(AtomicI32::new(0)),
            address: "127.0.0.1:25565".parse().unwrap(),
            expected: false,
        },
        uuid as i32,
        [0., 64., 0.],
        8,
    );
    guest.inventory[0] = Some(crate::items::ItemStack {
        item: "minecraft:diamond".into(),
        count,
        damage: 0,
        extra: None,
    });
    guest
}
fn publish_exact(frozen: &FrozenPlayerSaves) {
    let root = frozen.world_root().unwrap();
    for (uuid, _, tag) in frozen.snapshots() {
        crate::save::write_dat_durable(&path(root, uuid), tag.clone()).unwrap();
    }
}
#[test]
fn enable_drains_prior_fifo_then_disconnected_snapshots_stay_immutable_without_writes() {
    let root = Scratch::new("drain");
    let items = Default::default();
    let mut saves = Autosaves::default();
    saves.enqueue(&guest(7, 1), &root.0, &items);
    saves.enable_common_checkpoint().unwrap();
    assert!(saves.writer.is_none());
    assert_eq!(
        saves
            .completed(false)
            .iter()
            .map(|s| s.uuid)
            .collect::<Vec<_>>(),
        vec![7]
    );
    let before = fs::read(path(&root.0, 7)).unwrap();
    let mut live = guest(7, 3);
    let expected = player_tag(&live, &items);
    saves.enqueue(&live, &root.0, &items);
    live.inventory[0] = None;
    drop(live);
    assert_eq!(fs::read(path(&root.0, 7)).unwrap(), before);
    assert!(!saves.writes_finished().unwrap());
    let frozen = saves.freeze_common().unwrap();
    assert_eq!(frozen.world_root(), Some(root.0.as_path()));
    assert_eq!(
        frozen
            .snapshots()
            .map(|(uuid, name, tag)| (uuid, name.to_owned(), tag.clone()))
            .collect::<Vec<_>>(),
        vec![(7, "Player7".into(), expected)]
    );
    publish_exact(&frozen);
    let saved = saves.acknowledge_common(&frozen).unwrap();
    assert_eq!(saved[0].uuid, 7);
    assert!(saves.writes_finished().unwrap());
    assert!(saves.acknowledge_common(&frozen).is_err());
}
#[test]
fn a_newer_live_payload_can_supersede_deferred_logout_without_old_file_replay() {
    let root = Scratch::new("supersede");
    let items = Default::default();
    let mut saves = Autosaves::default();
    saves.enable_common_checkpoint().unwrap();
    assert!(saves.save_after(&guest(9, 3), &root.0, &items).is_empty());
    let frozen = saves.freeze_common().unwrap();
    let newer = player_tag(&guest(9, 11), &items);
    // Stand in only for the external GLOBAL WAL's exact durable playerdata write, using the actual writer.
    crate::save::write_dat_durable(&path(&root.0, 9), newer.clone()).unwrap();
    saves.validate_common(&frozen).unwrap();
    assert_eq!(saves.acknowledge_common(&frozen).unwrap()[0].uuid, 9);
    drop(saves);
    assert_eq!(read_dat(&path(&root.0, 9)).unwrap(), newer);
}
#[test]
fn a_changed_queue_or_different_queue_refuses_frozen_ack_without_file_writes() {
    let root = Scratch::new("generation");
    let items = Default::default();
    let mut saves = Autosaves::default();
    saves.enable_common_checkpoint().unwrap();
    saves.enqueue(&guest(1, 2), &root.0, &items);
    let first = saves.freeze_common().unwrap();
    saves.enqueue(&guest(2, 3), &root.0, &items);
    assert!(saves.validate_common(&first).is_err());
    assert!(saves.acknowledge_common(&first).is_err());
    let second = saves.freeze_common().unwrap();
    let mut other = Autosaves::default();
    other.enable_common_checkpoint().unwrap();
    other.enqueue(&guest(1, 2), &root.0, &items);
    assert!(other.validate_common(&second).is_err());
    assert!(!root.0.join("players").exists());
    publish_exact(&second);
    assert_eq!(saves.acknowledge_common(&second).unwrap().len(), 2);
}
#[test]
fn common_coalescing_is_bounded_by_uuid_and_keeps_latest_actual_snapshot() {
    let root = Scratch::new("uuid-cap");
    let items = Default::default();
    let mut saves = Autosaves::default();
    saves.enable_common_checkpoint().unwrap();
    for uuid in 0..MAX_PLAYERS as u128 {
        saves.enqueue(&guest(uuid, 1), &root.0, &items);
    }
    saves.enqueue(&guest(0, 9), &root.0, &items);
    assert_eq!(
        saves.freeze_common().unwrap().snapshots().len(),
        MAX_PLAYERS
    );
    let last = saves.freeze_common().unwrap();
    assert_eq!(last.snapshots().last().unwrap().0, 0);
    saves.enqueue(&guest(MAX_PLAYERS as u128, 1), &root.0, &items);
    assert!(saves.freeze_common().is_err());
    assert!(saves.writes_finished().is_err());
    assert!(saves.writes_finished().is_err());
    assert!(!root.0.join("players").exists());
}
#[test]
fn multi_world_admission_and_failed_fifo_enable_never_restart_independent_writer() {
    let first = Scratch::new("root-a");
    let second = Scratch::new("root-b");
    let items = Default::default();
    let mut saves = Autosaves::default();
    saves.enable_common_checkpoint().unwrap();
    saves.enqueue(&guest(1, 2), &first.0, &items);
    saves.enqueue(&guest(2, 3), &second.0, &items);
    assert!(saves.freeze_common().is_err());
    assert!(saves.writer.is_none());
    assert!(!first.0.join("players").exists());
    assert!(!second.0.join("players").exists());
    let failed = Scratch::new("old-io-error");
    fs::write(failed.0.join("players"), b"blocked").unwrap();
    let mut blocked = Autosaves::default();
    blocked.enqueue(&guest(4, 2), &failed.0, &items);
    assert!(blocked.enable_common_checkpoint().is_err());
    assert!(blocked.common_required());
    assert!(blocked.writer.is_none());
    fs::remove_file(failed.0.join("players")).unwrap();
    blocked.enqueue(&guest(5, 3), &failed.0, &items);
    assert!(blocked.writes_finished().is_err());
    assert!(blocked.writer.is_none());
    drop(blocked);
    assert!(!path(&failed.0, 5).exists());
}
#[cfg(unix)]
#[test]
fn replacing_the_same_root_path_refuses_the_old_snapshot_identity() {
    let root = Scratch::new("stale-root");
    let items = Default::default();
    let mut saves = Autosaves::default();
    saves.enable_common_checkpoint().unwrap();
    saves.enqueue(&guest(1, 2), &root.0, &items);
    let frozen = saves.freeze_common().unwrap();
    let old = root.0.with_extension("old");
    fs::rename(&root.0, &old).unwrap();
    fs::create_dir(&root.0).unwrap();
    assert!(saves.validate_common(&frozen).is_err());
    assert!(saves.freeze_common().is_err());
    saves.fail_common();
    drop(saves);
    assert!(!root.0.join("players").exists());
    fs::remove_dir_all(old).unwrap();
}
#[test]
fn post_decision_failure_keeps_deferred_records_and_forbids_old_or_new_replay() {
    let root = Scratch::new("terminal");
    let items = Default::default();
    let mut saves = Autosaves::default();
    saves.enable_common_checkpoint().unwrap();
    saves.enqueue(&guest(1, 2), &root.0, &items);
    let frozen = saves.freeze_common().unwrap();
    saves.fail_common();
    saves.enqueue(&guest(1, 99), &root.0, &items);
    assert!(saves.validate_common(&frozen).is_err());
    assert!(saves.writes_finished().is_err());
    assert!(saves.writes_finished().is_err());
    drop(saves);
    assert!(!path(&root.0, 1).exists());
}
#[test]
#[ignore = "daily"]
fn aggregate_serialized_payload_budget_refuses_without_any_player_file() {
    let root = Scratch::new("byte-cap");
    let mut saves = Autosaves::default();
    saves.enable_common_checkpoint().unwrap();
    for uuid in 0..4 {
        let snapshot = Snapshot {
            uuid,
            name: format!("Player{uuid}"),
            file: path(&root.0, uuid),
            tag: Tag::compound([("payload", Tag::ByteArray(vec![7; 13 << 20]))]),
        };
        saves.common.admit(&root.0, snapshot).unwrap();
    }
    let snapshot = Snapshot {
        uuid: 4,
        name: "Player4".into(),
        file: path(&root.0, 4),
        tag: Tag::compound([("payload", Tag::ByteArray(vec![7; 13 << 20]))]),
    };
    assert!(saves.common.admit(&root.0, snapshot).is_err());
    assert!(!root.0.join("players").exists());
}
