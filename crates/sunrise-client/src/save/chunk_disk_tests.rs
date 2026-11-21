//! Real region-file checks for the chunk worker's attachment boundary.
use super::*;
use mod_api::{ChunkDataLoadEvent, ChunkDataSaveEvent, ChunkDiskHost, Hooks};
use std::sync::atomic::AtomicUsize;
mod fixture;
mod writer_stack;

#[derive(Default)]
struct Listener {
    saves: Arc<AtomicUsize>,
    loads: Arc<AtomicUsize>,
}
#[mod_api::sunrise_mod]
impl Hooks for Listener {
    fn chunk_data_save(&self, _: &dyn ChunkDiskHost, event: &mut ChunkDataSaveEvent<'_>) {
        self.saves.fetch_add(1, Ordering::Relaxed);
        fixture::save(event.data);
    }
    fn chunk_data_load(&self, host: &dyn ChunkDiskHost, event: &mut ChunkDataLoadEvent<'_>) {
        self.loads.fetch_add(1, Ordering::Relaxed);
        fixture::load(host, event.data);
    }
}
fn context(save: &SaveDir) {
    use sunrise_world::native_load_context::{DataLoadOrigin, LoadContext, LoadInputs};
    let load = LoadContext::new(
        DataLoadOrigin::ServerDataLoad,
        Arc::new(LoadInputs::new(BTreeMap::new(), save.root.join("settings"))),
        [],
    );
    save.set_chunk_disk_context(
        mod_api::WorldEpoch::new(17),
        mod_api::Identifier::parse("minecraft:overworld").unwrap(),
        Arc::new(load.worldgen_context([])),
    );
}
fn chunk(value: i32) -> Chunk {
    let mut chunk = Chunk::empty(2, -3);
    let mut extras = ChunkExtras::default();
    fixture::VALUE
        .set(&mut extras.attachments, mod_api::Holder::Level, value)
        .unwrap();
    chunk.saved = Some(Arc::new(extras));
    chunk
}
fn value(chunk: &mut Chunk) -> i32 {
    fixture::VALUE
        .get(
            &mut Arc::make_mut(chunk.saved.as_mut().unwrap()).attachments,
            mod_api::Holder::Level,
        )
        .unwrap()
}
fn root() -> PathBuf {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    std::env::temp_dir().join(format!(
        "sunrise-chunk-disk-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ))
}
fn with_listener(run: impl FnOnce(Arc<AtomicUsize>, Arc<AtomicUsize>)) {
    let saves = Arc::new(AtomicUsize::new(0));
    let loads = Arc::new(AtomicUsize::new(0));
    let mods = vec![mod_api::ModEntry::new(
        "chunk_disk_fixture",
        Box::new(Listener {
            saves: saves.clone(),
            loads: loads.clone(),
        }),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || run(saves, loads));
}

#[test]
fn capture_7_callback_8_live_9_round_trips_through_region_writer() {
    let (registry, biomes) = super::loot_tests::fixture();
    let registry = Box::leak(Box::new(registry));
    let biomes = Arc::new(biomes);
    let path = root();
    with_listener(|saves, _| {
        let save = SaveDir::new(
            path.clone(),
            LevelShape::OVERWORLD,
            Some(Default::default()),
        );
        context(&save);
        let mut live = chunk(7);
        save.save_later(&[Arc::new(live.clone())], registry, &biomes)
            .unwrap();
        fixture::VALUE
            .set(
                &mut Arc::make_mut(live.saved.as_mut().unwrap()).attachments,
                mod_api::Holder::Level,
                9,
            )
            .unwrap();
        save.flush().unwrap();
        assert_eq!(saves.load(Ordering::Relaxed), 1);
        assert_eq!(value(&mut live), 9);
        let disk = SaveDir::new(
            path.clone(),
            LevelShape::OVERWORLD,
            Some(Default::default()),
        );
        assert_eq!(value(&mut disk.load(2, -3, registry, &biomes).unwrap()), 8);
    });
    fs::remove_dir_all(path).unwrap();
}

#[test]
fn load_disk_8_callback_increment_publishes_9_once_through_decode() {
    let (registry, biomes) = super::loot_tests::fixture();
    let path = root();
    mod_platform::testing::with_mods(Vec::new().into(), || {
        SaveDir::new(
            path.clone(),
            LevelShape::OVERWORLD,
            Some(Default::default()),
        )
        .save(&[Arc::new(chunk(8))], &registry, &biomes)
        .unwrap();
    });
    with_listener(|_, loads| {
        let save = SaveDir::new(
            path.clone(),
            LevelShape::OVERWORLD,
            Some(Default::default()),
        );
        context(&save);
        assert_eq!(value(&mut save.load(2, -3, &registry, &biomes).unwrap()), 9);
        assert_eq!(loads.load(Ordering::Relaxed), 1);
    });
    fs::remove_dir_all(path).unwrap();
}

#[test]
fn full_and_proto_writer_records_each_invoke_once_before_encode() {
    let (registry, biomes) = super::loot_tests::fixture();
    let registry = Box::leak(Box::new(registry));
    let biomes = Arc::new(biomes);
    let path = root();
    with_listener(|saves, _| {
        let save = SaveDir::new(
            path.clone(),
            LevelShape::OVERWORLD,
            Some(Default::default()),
        );
        context(&save);
        save.save_later(&[Arc::new(chunk(7))], registry, &biomes)
            .unwrap();
        save.save_protos(
            vec![SavedProto {
                column: chunk(7),
                decorated: true,
                block_entities: Vec::new(),
                entities: Vec::new(),
            }],
            registry,
            &biomes,
        )
        .unwrap();
        save.flush().unwrap();
        assert_eq!(saves.load(Ordering::Relaxed), 2);
        assert_eq!(
            value(
                &mut SaveDir::new(path.clone(), LevelShape::OVERWORLD, None)
                    .load_proto(2, -3, registry, &biomes)
                    .unwrap()
                    .column
            ),
            8
        );
    });
    fs::remove_dir_all(path).unwrap();
}

/// Records what each save callback saw, in the order the writer ran them.
// Its own module: `sunrise_mod` defines one `hooks` function per module.
mod echo {
    use super::*;
    #[derive(Default)]
    pub struct Echo {
        pub seen: Arc<Mutex<Vec<(mod_api::ChunkDiskKind, i32)>>>,
    }
    #[mod_api::sunrise_mod]
    impl Hooks for Echo {
        fn chunk_data_save(&self, host: &dyn ChunkDiskHost, event: &mut ChunkDataSaveEvent<'_>) {
            // A worker callback has no live world: the old handle's path refuses.
            assert!(host.backend().is_none());
            let value = event.data.get(&fixture::VALUE).unwrap();
            self.seen.lock().unwrap().push((host.job().kind, value));
        }
    }
}
use echo::Echo;
fn with_echo(run: impl FnOnce(Arc<Mutex<Vec<(mod_api::ChunkDiskKind, i32)>>>)) {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let mods = vec![mod_api::ModEntry::new(
        "chunk_disk_fixture",
        Box::new(Echo { seen: seen.clone() }),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || run(seen));
}
fn disk_value(path: &Path, registry: &BlockRegistry, biomes: &Biomes) -> Option<i32> {
    SaveDir::new(
        path.to_path_buf(),
        LevelShape::OVERWORLD,
        Some(Default::default()),
    )
    .load(2, -3, registry, biomes)
    .map(|mut chunk| value(&mut chunk))
}

#[test]
fn revisions_10_then_11_queued_together_leave_disk_at_11() {
    let (registry, biomes) = super::loot_tests::fixture();
    let registry = Box::leak(Box::new(registry));
    let biomes = Arc::new(biomes);
    let path = root();
    with_echo(|seen| {
        let save = SaveDir::new(
            path.clone(),
            LevelShape::OVERWORLD,
            Some(Default::default()),
        );
        context(&save);
        save.save_later(&[Arc::new(chunk(10))], registry, &biomes)
            .unwrap();
        save.save_later(&[Arc::new(chunk(11))], registry, &biomes)
            .unwrap();
        save.flush().unwrap();
        // Each captured record had its callback, whether or not its write was
        // coalesced away; only the newest revision is on disk.
        let seen: Vec<i32> = seen.lock().unwrap().iter().map(|(_, v)| *v).collect();
        assert_eq!(seen, [10, 11]);
        assert_eq!(disk_value(&path, registry, &biomes), Some(11));
    });
    fs::remove_dir_all(path).unwrap();
}

#[test]
fn every_save_kind_runs_exactly_one_callback_per_captured_record() {
    let (registry, biomes) = super::loot_tests::fixture();
    let registry = Box::leak(Box::new(registry));
    let biomes = Arc::new(biomes);
    let path = root();
    with_echo(|seen| {
        let save = SaveDir::new(
            path.clone(),
            LevelShape::OVERWORLD,
            Some(Default::default()),
        );
        context(&save);
        // Asynchronous full saves: explicit, forced, late and unload all queue this way.
        save.save_later(&[Arc::new(chunk(1))], registry, &biomes)
            .unwrap();
        save.save_later(&[Arc::new(chunk(2))], registry, &biomes)
            .unwrap();
        save.flush().unwrap();
        // The synchronous route used by closing and forced saves.
        save.save(&[Arc::new(chunk(3))], registry, &biomes).unwrap();
        // Proto chunks.
        save.save_protos(
            vec![SavedProto {
                column: chunk(4),
                decorated: true,
                block_entities: Vec::new(),
                entities: Vec::new(),
            }],
            registry,
            &biomes,
        )
        .unwrap();
        save.flush().unwrap();
        use mod_api::ChunkDiskKind::{Full, Proto};
        assert_eq!(
            *seen.lock().unwrap(),
            [(Full, 1), (Full, 2), (Full, 3), (Proto, 4)]
        );
    });
    fs::remove_dir_all(path).unwrap();
}

#[test]
#[ignore = "daily"]
fn save_refused_during_open_action_savepoint_runs_no_callback_and_keeps_disk_7() {
    // The savepoint signal is process-wide: hold it only in an isolated child.
    const CHILD: &str = "SUNRISE_CHUNK_DISK_SAVEPOINT_CHILD";
    if std::env::var_os(CHILD).is_none() {
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--include-ignored",
                "--exact",
                "save::chunk_disk_tests::save_refused_during_open_action_savepoint_runs_no_callback_and_keeps_disk_7",
            ])
            .env(CHILD, "1")
            .status()
            .unwrap();
        assert!(status.success());
        return;
    }
    let (registry, biomes) = super::loot_tests::fixture();
    let registry = Box::leak(Box::new(registry));
    let biomes = Arc::new(biomes);
    let path = root();
    mod_platform::testing::with_mods(Vec::new().into(), || {
        SaveDir::new(
            path.clone(),
            LevelShape::OVERWORLD,
            Some(Default::default()),
        )
        .save(&[Arc::new(chunk(7))], registry, &biomes)
        .unwrap();
    });
    with_echo(|seen| {
        let save = SaveDir::new(
            path.clone(),
            LevelShape::OVERWORLD,
            Some(Default::default()),
        );
        context(&save);
        let savepoint = crate::join_seq::visibility::PassLease::enter();
        // Tentative 9 is open: asynchronous and synchronous saves both refuse.
        assert!(
            save.save_later(&[Arc::new(chunk(9))], registry, &biomes)
                .is_err()
        );
        assert_eq!(save.take_failed_chunk_saves(), [(2, -3)]);
        assert!(save.save(&[Arc::new(chunk(9))], registry, &biomes).is_err());
        drop(savepoint);
        assert!(seen.lock().unwrap().is_empty());
    });
    assert_eq!(disk_value(&path, registry, &biomes), Some(7));
    fs::remove_dir_all(path).unwrap();
}

#[test]
fn unload_then_reload_reads_saved_8_in_a_fresh_holder() {
    let (registry, biomes) = super::loot_tests::fixture();
    let registry = Box::leak(Box::new(registry));
    let biomes = Arc::new(biomes);
    let path = root();
    with_listener(|_, _| {
        let save = SaveDir::new(
            path.clone(),
            LevelShape::OVERWORLD,
            Some(Default::default()),
        );
        context(&save);
        save.save_later(&[Arc::new(chunk(7))], registry, &biomes)
            .unwrap();
        save.flush().unwrap();
    });
    // No listener on reload: the stored value is exactly what the save callback wrote.
    mod_platform::testing::with_mods(Vec::new().into(), || {
        assert_eq!(disk_value(&path, registry, &biomes), Some(8));
    });
    fs::remove_dir_all(path).unwrap();
}

#[cfg(unix)]
#[test]
fn disk_write_failure_keeps_old_disk_7_and_snapshot_8_for_recovery() {
    use std::os::unix::fs::PermissionsExt;
    let (registry, biomes) = super::loot_tests::fixture();
    let registry = Box::leak(Box::new(registry));
    let biomes = Arc::new(biomes);
    let path = root();
    mod_platform::testing::with_mods(Vec::new().into(), || {
        SaveDir::new(
            path.clone(),
            LevelShape::OVERWORLD,
            Some(Default::default()),
        )
        .save(&[Arc::new(chunk(7))], registry, &biomes)
        .unwrap();
    });
    let region = path.join("region/r.0.-1.mca");
    let writable = fs::metadata(&region).unwrap().permissions();
    fs::set_permissions(&region, fs::Permissions::from_mode(0o444)).unwrap();
    with_listener(|saves, _| {
        let save = SaveDir::new(
            path.clone(),
            LevelShape::OVERWORLD,
            Some(Default::default()),
        );
        context(&save);
        save.save_later(&[Arc::new(chunk(7))], registry, &biomes)
            .unwrap();
        // The close path reports the failure instead of treating the save as durable.
        assert!(save.flush().is_err());
        assert_eq!(saves.load(Ordering::Relaxed), 1);
        // The chunk is marked for another attempt instead of silently clean.
        assert_eq!(save.take_failed_chunk_saves(), [(2, -3)]);
        // The unwritten snapshot (8) stays readable for recovery. Read it with no
        // listener installed, so no load callback adds to it.
        mod_platform::testing::with_mods(Vec::new().into(), || {
            let mut recovered = save.load(2, -3, registry, &biomes).unwrap();
            assert_eq!(value(&mut recovered), 8);
        });
    });
    fs::set_permissions(&region, writable).unwrap();
    // The old disk record is untouched.
    mod_platform::testing::with_mods(Vec::new().into(), || {
        assert_eq!(disk_value(&path, registry, &biomes), Some(7));
    });
    fs::remove_dir_all(path).unwrap();
}

/// A chunk whose load listener failed loads with its stored attachments and is saved by every
/// path with no save listener, so the stored value 8 is written back unchanged.
#[test]
fn failed_load_keeps_stored_8_and_runs_no_save_listener_for_every_save_path() {
    let (registry, biomes) = super::loot_tests::fixture();
    let registry = Box::leak(Box::new(registry));
    let biomes = Arc::new(biomes);
    for (load_only, sync) in [(true, true), (true, false), (false, true), (false, false)] {
        let path = root();
        mod_platform::testing::with_mods(Vec::new().into(), || {
            SaveDir::new(
                path.clone(),
                LevelShape::OVERWORLD,
                Some(Default::default()),
            )
            .save(&[Arc::new(chunk(8))], registry, &biomes)
            .unwrap();
        });
        let saves = Arc::new(AtomicUsize::new(0));
        let listener: Box<dyn Hooks> = if load_only {
            Box::new(breaker::Breaker)
        } else {
            Box::new(breaker_both::BreakerBoth {
                saves: saves.clone(),
            })
        };
        let mods = vec![mod_api::ModEntry::new("chunk_disk_fixture", listener)].into();
        mod_platform::testing::with_mods(mods, || {
            let save = SaveDir::new(
                path.clone(),
                LevelShape::OVERWORLD,
                Some(Default::default()),
            );
            context(&save);
            let mut loaded = save
                .load(2, -3, registry, &biomes)
                .expect("the chunk loads");
            assert!(save.chunk_save_suppressed(2, -3));
            assert!(!save.chunk_save_suppressed(0, 0));
            // The failed job's edits were dropped: the stored value is still there.
            assert_eq!(value(&mut loaded), 8);
            let chunks = [Arc::new(loaded)];
            if sync {
                save.save(&chunks, registry, &biomes).unwrap();
            } else {
                save.save_later(&chunks, registry, &biomes).unwrap();
                save.flush().unwrap();
            }
            assert!(save.take_failed_chunk_saves().is_empty());
        });
        assert_eq!(saves.load(Ordering::Relaxed), 0);
        mod_platform::testing::with_mods(Vec::new().into(), || {
            assert_eq!(disk_value(&path, registry, &biomes), Some(8));
        });
        fs::remove_dir_all(path).unwrap();
    }
}

// Its own module: `sunrise_mod` defines one `hooks` function per module.
mod breaker {
    use super::*;
    #[derive(Default)]
    pub struct Breaker;
    #[mod_api::sunrise_mod]
    impl Hooks for Breaker {
        fn chunk_data_load(&self, _: &dyn ChunkDiskHost, _: &mut ChunkDataLoadEvent<'_>) {
            panic!("load listener fails");
        }
    }
}
mod breaker_both {
    use super::*;
    #[derive(Default)]
    pub struct BreakerBoth {
        pub saves: Arc<AtomicUsize>,
    }
    #[mod_api::sunrise_mod]
    impl Hooks for BreakerBoth {
        fn chunk_data_load(&self, _: &dyn ChunkDiskHost, _: &mut ChunkDataLoadEvent<'_>) {
            panic!("load listener fails");
        }
        fn chunk_data_save(&self, _: &dyn ChunkDiskHost, _: &mut ChunkDataSaveEvent<'_>) {
            self.saves.fetch_add(1, Ordering::Relaxed);
        }
    }
}
