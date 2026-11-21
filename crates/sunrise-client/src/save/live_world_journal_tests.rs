//! Real-filesystem failure cases. Drafts only: NOT EXECUTED by the storage worker.
use super::*;
use std::io::{Seek, SeekFrom, Write};
use std::sync::atomic::{AtomicU64, Ordering};
use sunrise_core::nbt::Tag;

struct Scratch(PathBuf);
impl Scratch {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "sunrise-live-journal-{}-{}",
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
fn cell(level: &str, x: i32, value: i32) -> Image {
    let tag = NamedTag {
        name: "".into(),
        tag: Tag::compound([
            ("xPos", Tag::Int(x)),
            ("zPos", Tag::Int(0)),
            ("value", Tag::Int(value)),
            (
                "block_entities",
                Tag::List(vec![Tag::compound([
                    ("id", Tag::String("fixture:container".into())),
                    (
                        "Items",
                        Tag::List(vec![Tag::compound([
                            ("Slot", Tag::Byte(0)),
                            ("count", Tag::Int(value)),
                        ])]),
                    ),
                ])]),
            ),
            (
                "block_ticks",
                Tag::List(vec![Tag::compound([
                    ("x", Tag::Int(x * 16)),
                    ("t", Tag::Int(37)),
                ])]),
            ),
            (
                "fluid_ticks",
                Tag::List(vec![Tag::compound([
                    ("x", Tag::Int(x * 16)),
                    ("t", Tag::Int(19)),
                ])]),
            ),
        ]),
    };
    Image {
        key: Key::Record {
            level: level.into(),
            folder: Folder::Region,
            chunk: [x, 0],
        },
        after: Some(nbt::write_named(&tag, Limits::default().nbt()).unwrap()),
    }
}
fn projection(level: &str, value: &str) -> Image {
    Image {
        key: Key::Projection {
            level: level.into(),
        },
        after: Some(value.as_bytes().to_vec()),
    }
}
fn player(uuid: u128, count: i32) -> Image {
    let tag = NamedTag {
        name: "".into(),
        tag: Tag::compound([(
            "Inventory",
            Tag::List(vec![Tag::compound([("count", Tag::Int(count))])]),
        )]),
    };
    Image {
        key: Key::PlayerData { uuid },
        after: Some(
            Compression::Gzip
                .encode(&nbt::write_named(&tag, Limits::default().nbt()).unwrap())
                .unwrap(),
        ),
    }
}
fn value(root: &Path, level: &str, x: i32) -> Option<Tag> {
    let bytes = read_image(root, &cell(level, x, 0).key, Limits::default()).unwrap()?;
    named(&bytes, Limits::default())
        .unwrap()
        .tag
        .get("value")
        .cloned()
}
fn capture(root: &Path, epoch: u64, mut images: Vec<Image>) -> Prepared {
    images.push(checkpoint_image(epoch));
    Prepared::capture(root, epoch, images, Limits::default()).unwrap()
}
fn raw_slot(path: &Path, slot: usize) -> (u32, u32, Vec<u8>) {
    let raw = fs::read(path).unwrap();
    let offset = u32::from_be_bytes(raw[slot * 4..slot * 4 + 4].try_into().unwrap());
    let timestamp = u32::from_be_bytes(raw[4096 + slot * 4..4100 + slot * 4].try_into().unwrap());
    let start = (offset >> 8) as usize * 4096;
    (
        offset,
        timestamp,
        raw[start..start + (offset & 255) as usize * 4096].to_vec(),
    )
}
fn overwrite_header(path: &Path) {
    let mut file = fs::OpenOptions::new().write(true).open(path).unwrap();
    file.seek(SeekFrom::Start(0)).unwrap();
    file.write_all(&[0; 8192]).unwrap();
    file.sync_all().unwrap();
}
#[test]
fn interrupted_transfer_repairs_torn_header_payload_and_repeated_recovery() {
    let scratch = Scratch::new();
    let root = &scratch.0;
    apply_image(root, &cell("fixture:source", 0, 7), Limits::default()).unwrap();
    apply_image(root, &cell("fixture:source", 1, 29), Limits::default()).unwrap();
    let original_raw = raw_slot(&cell("fixture:source", 1, 0).key.path(root).unwrap(), 1);
    let untouched = read_image(root, &cell("fixture:source", 1, 0).key, Limits::default()).unwrap();
    let prepared = capture(
        root,
        1,
        vec![
            cell("fixture:source", 0, 0),
            cell("fixture:destination", 0, 7),
            projection("fixture:destination", r#"{"epoch":1}"#),
            player(17, 3),
        ],
    );
    prepared.publish(root).unwrap();
    let source = prepared
        .images()
        .find(|i| i.key == cell("fixture:source", 0, 0).key)
        .unwrap();
    apply_image(root, source, Limits::default()).unwrap();
    let path = source.key.path(root).unwrap();
    // Tear the selected compressed payload, then the entire allocation header, preserving unselected sectors.
    let mut region = RegionFile::open(&path).unwrap();
    let selected = region.read(ChunkPos::new(0, 0)).unwrap().unwrap();
    assert!(!selected.compressed.is_empty());
    drop(region);
    let mut raw = fs::read(&path).unwrap();
    let entry = u32::from_be_bytes(raw[..4].try_into().unwrap());
    raw[(entry >> 8) as usize * 4096 + 5] ^= 255;
    fs::write(&path, &raw).unwrap();
    overwrite_header(&path);
    // Simulate another interrupted recovery: original maps are repaired, only one afterimage is applied.
    prepared.preflight_disk(root, true).unwrap();
    for region in &prepared.regions {
        region.restore_allocation(root).unwrap();
    }
    apply_image(root, source, Limits::default()).unwrap();
    overwrite_header(&path);
    assert!(recover(root, Limits::default()).unwrap());
    assert_eq!(value(root, "fixture:source", 0), Some(Tag::Int(0)));
    assert_eq!(value(root, "fixture:destination", 0), Some(Tag::Int(7)));
    assert_eq!(
        read_image(root, &cell("fixture:source", 1, 0).key, Limits::default()).unwrap(),
        untouched
    );
    assert_eq!(
        read_image(root, &player(17, 0).key, Limits::default()).unwrap(),
        player(17, 3).after
    );
    assert_eq!(raw_slot(&path, 1), original_raw);
    assert_eq!(read_epoch(root).unwrap(), 1);
    assert_eq!(next_epoch(root).unwrap(), 2);
    assert!(!recover(root, Limits::default()).unwrap());
    let second = capture(
        root,
        2,
        vec![projection("fixture:destination", r#"{"epoch":2}"#)],
    );
    second.publish(root).unwrap();
    assert!(recover(root, Limits::default()).unwrap());
    assert_eq!(read_epoch(root).unwrap(), 2);
}
#[test]
fn divergent_metadata_refuses_whole_batch_before_header_repair() {
    let scratch = Scratch::new();
    let root = &scratch.0;
    let initial = cell("fixture:source", 0, 7);
    apply_image(root, &initial, Limits::default()).unwrap();
    let prepared = capture(
        root,
        1,
        vec![
            cell("fixture:source", 0, 0),
            cell("fixture:destination", 0, 7),
            projection("fixture:destination", r#"{"epoch":1}"#),
        ],
    );
    prepared.publish(root).unwrap();
    let path = initial.key.path(root).unwrap();
    overwrite_header(&path);
    let before = fs::read(&path).unwrap();
    apply_image(
        root,
        &projection("fixture:destination", r#"{"epoch":99}"#),
        Limits::default(),
    )
    .unwrap();
    assert!(recover(root, Limits::default()).is_err());
    assert_eq!(fs::read(&path).unwrap(), before);
    assert_eq!(value(root, "fixture:destination", 0), None);
    assert!(journal_path(root).unwrap().exists());
}
#[test]
fn corrupted_decision_and_corrupted_unselected_sector_refuse_without_writes() {
    let scratch = Scratch::new();
    let root = &scratch.0;
    apply_image(root, &cell("fixture:source", 0, 7), Limits::default()).unwrap();
    apply_image(root, &cell("fixture:source", 1, 29), Limits::default()).unwrap();
    let prepared = capture(root, 1, vec![cell("fixture:source", 0, 0)]);
    prepared.publish(root).unwrap();
    let pending = journal_path(root).unwrap();
    let correct = fs::read(&pending).unwrap();
    let mut corrupt = correct.clone();
    corrupt[12] ^= 1;
    fs::write(&pending, &corrupt).unwrap();
    assert!(recover(root, Limits::default()).is_err());
    assert_eq!(fs::read(&pending).unwrap(), corrupt);
    assert!(
        matches!(Prepared::capture(root,1,vec![cell("fixture:source",0,9),checkpoint_image(1)],Limits::default()),Err(e) if e.kind()==io::ErrorKind::AlreadyExists)
    );
    fs::write(&pending, &correct).unwrap();
    let path = cell("fixture:source", 1, 0).key.path(root).unwrap();
    let mut raw = fs::read(&path).unwrap();
    let offset = u32::from_be_bytes(raw[4..8].try_into().unwrap());
    raw[(offset >> 8) as usize * 4096 + 6] ^= 1;
    fs::write(&path, &raw).unwrap();
    assert!(recover(root, Limits::default()).is_err());
    assert_eq!(fs::read(&path).unwrap(), raw);
    assert_eq!(fs::read(&pending).unwrap(), correct);
}
#[test]
fn before_readset_changes_and_invalid_input_never_publish_prepared() {
    let scratch = Scratch::new();
    let root = &scratch.0;
    apply_image(root, &cell("fixture:source", 0, 7), Limits::default()).unwrap();
    let prepared = capture(
        root,
        1,
        vec![
            cell("fixture:source", 0, 0),
            cell("fixture:destination", 0, 7),
        ],
    );
    apply_image(root, &cell("fixture:destination", 0, 99), Limits::default()).unwrap();
    assert!(prepared.publish(root).is_err());
    assert_eq!(value(root, "fixture:source", 0), Some(Tag::Int(7)));
    assert!(!journal_path(root).unwrap().exists());
    assert!(
        Prepared::capture(
            root,
            1,
            vec![cell("fixture:../escape", 0, 7), checkpoint_image(1)],
            Limits::default()
        )
        .is_err()
    );
    assert!(
        Prepared::capture(
            root,
            1,
            vec![
                cell("fixture:source", 0, 7),
                cell("fixture:source", 0, 9),
                checkpoint_image(1)
            ],
            Limits::default()
        )
        .is_err()
    );
    assert!(
        Prepared::capture(
            root,
            1,
            vec![cell("fixture:source", 0, 7), checkpoint_image(1)],
            Limits {
                bytes: 64,
                record_bytes: 32,
                ..Limits::default()
            }
        )
        .is_err()
    );
    let mut trailing = cell("fixture:source", 0, 7);
    trailing.after.as_mut().unwrap().push(0);
    assert!(
        Prepared::capture(
            root,
            1,
            vec![trailing, checkpoint_image(1)],
            Limits::default()
        )
        .is_err()
    );
    assert!(!journal_path(root).unwrap().exists());
}
#[test]
fn selected_external_stream_replays_while_unselected_stream_is_exact() {
    let scratch = Scratch::new();
    let root = &scratch.0;
    let mut payload = Vec::with_capacity(1_060_000);
    let mut state = 19u32;
    for _ in 0..1_060_000 {
        state ^= state << 13;
        state ^= state >> 17;
        state ^= state << 5;
        payload.push(state as u8);
    }
    let mut large = cell("fixture:source", 0, 7);
    let mut tag = named(large.after.as_ref().unwrap(), Limits::default()).unwrap();
    if let Tag::Compound(data) = &mut tag.tag {
        data.insert("payload".into(), Tag::ByteArray(payload));
    }
    large.after = Some(nbt::write_named(&tag, Limits::default().nbt()).unwrap());
    let path = large.key.path(root).unwrap();
    ensure_parents(root, &path).unwrap();
    let mut region = RegionFile::open_writable(&path, true).unwrap();
    let record =
        ChunkRecord::from_nbt(&tag, Compression::None, 17, Limits::default().nbt()).unwrap();
    region.write(ChunkPos::new(0, 0), &record).unwrap();
    let mut untouched_tag = tag.clone();
    if let Tag::Compound(data) = &mut untouched_tag.tag {
        data.insert("xPos".into(), Tag::Int(1));
    }
    let untouched_record = ChunkRecord::from_nbt(
        &untouched_tag,
        Compression::None,
        91,
        Limits::default().nbt(),
    )
    .unwrap();
    region
        .write(ChunkPos::new(1, 0), &untouched_record)
        .unwrap();
    region.sync().unwrap();
    drop(region);
    let untouched = path.parent().unwrap().join("c.1.0.mcc");
    let untouched_bytes = fs::read(&untouched).unwrap();
    let mut destination = large.clone();
    destination.key = cell("fixture:destination", 0, 0).key;
    let prepared = capture(
        root,
        1,
        vec![cell("fixture:source", 0, 0), destination.clone()],
    );
    prepared.publish(root).unwrap();
    apply_image(root, &cell("fixture:source", 0, 0), Limits::default()).unwrap();
    assert!(!path.parent().unwrap().join("c.0.0.mcc").exists());
    overwrite_header(&path);
    assert!(recover(root, Limits::default()).unwrap());
    assert_eq!(fs::read(&untouched).unwrap(), untouched_bytes);
    assert_eq!(
        read_image(root, &destination.key, Limits::default()).unwrap(),
        destination.after
    );
    assert!(
        destination
            .key
            .path(root)
            .unwrap()
            .parent()
            .unwrap()
            .join("c.0.0.mcc")
            .exists()
    );
}

#[test]
fn explicit_migration_retains_original_and_recovers_both_metadata_images() {
    let scratch = Scratch::new();
    let level = "fixture:migration";
    let original = br#"{"schema":1,"pose":{"translation":[0.1,0.0,0.0]}}"#.to_vec();
    let updated = br#"{"schema":2,"pose_bits":[4591870180066957722]}"#.to_vec();
    let old = Image {
        key: Key::Projection {
            level: level.into(),
        },
        after: Some(original.clone()),
    };
    apply_image(&scratch.0, &old, Limits::default()).unwrap();
    let keep = Image {
        key: Key::ProjectionOriginal {
            level: level.into(),
        },
        after: Some(original.clone()),
    };
    let after = Image {
        key: old.key.clone(),
        after: Some(updated.clone()),
    };
    let prepared = capture(&scratch.0, 1, vec![keep, after]);
    prepared.publish(&scratch.0).unwrap();
    prepared.write_projection_originals(&scratch.0).unwrap();
    assert_eq!(
        read_projection(&scratch.0, level).unwrap(),
        Some(original.clone())
    );
    assert_eq!(
        read_projection_original(&scratch.0, level).unwrap(),
        Some(original.clone())
    );
    assert!(recover(&scratch.0, Limits::default()).unwrap());
    assert_eq!(read_projection(&scratch.0, level).unwrap(), Some(updated));
    assert_eq!(
        read_projection_original(&scratch.0, level).unwrap(),
        Some(original)
    );
    assert!(!recover(&scratch.0, Limits::default()).unwrap());
}

#[test]
fn missing_expected_payload_must_still_refuse_acknowledgment() {
    let scratch = Scratch::new();
    let image = cell("fixture:payload", 0, 9);
    let prepared = capture(&scratch.0, 1, vec![image]);
    prepared.publish(&scratch.0).unwrap();
    prepared.write_checkpoint_epoch(&scratch.0).unwrap();
    assert!(
        prepared.acknowledge(&scratch.0).is_err(),
        "a missing nonempty afterimage must still refuse"
    );
}

#[test]
fn exact_envelope_usage_and_typed_budget_refuse_before_publish() {
    let scratch = Scratch::new();
    let root = &scratch.0;
    let old = projection("fixture:source", r#"{"state":"old"}"#);
    apply_image(root, &old, Limits::default()).unwrap();
    let images = vec![
        projection("fixture:source", r#"{"state":"new"}"#),
        cell("fixture:source", 0, 7),
        Image {
            key: Key::Record {
                level: "fixture:source".into(),
                folder: Folder::Entities,
                chunk: [0, 0],
            },
            after: None,
        },
        player(123, 2),
        checkpoint_image(1),
    ];
    let prepared = Prepared::capture(root, 1, images.clone(), Limits::default()).unwrap();
    let usage = prepared.encoded_usage().unwrap();
    assert_eq!(
        usage.serialized_bytes,
        prepared.encode().unwrap().len() as u64
    );
    assert_eq!(
        usage.payload_bytes,
        images
            .iter()
            .filter_map(|image| image.after.as_ref())
            .map(|bytes| bytes.len() as u64)
            .sum::<u64>()
    );
    assert_eq!(
        (
            usage.records,
            usage.levels,
            usage.terrain_records,
            usage.entity_records,
            usage.metadata_records
        ),
        (5, 1, 1, 1, 3)
    );
    assert!(usage.witness_bytes > 16_384);
    let limits = Limits {
        bytes: usage.serialized_bytes - 1,
        record_bytes: 4096,
        ..Limits::default()
    };
    let error = match Prepared::capture(root, 1, images, limits) {
        Ok(_) => panic!("full envelope exceeds exact limit"),
        Err(error) => error,
    };
    let budget = byte_budget_error(&error).expect("typed complete-envelope budget");
    assert_eq!(budget.usage, usage);
    assert_eq!(budget.limit, limits.bytes);
    assert!(!journal_path(root).unwrap().exists());
    assert_eq!(read_epoch(root).unwrap(), 0);
    assert_eq!(
        read_image(root, &old.key, Limits::default()).unwrap(),
        old.after
    );
    assert!(!root.join("dimensions/fixture/source/region").exists());
}
