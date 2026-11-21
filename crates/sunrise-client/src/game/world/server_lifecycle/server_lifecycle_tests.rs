use crate::game::{
    execution_role::ExecutionRole, native_entity_refs::tests::with_game,
    native_guest_turn_tests::with_guests,
};
use mod_api::{Hooks, ModEntry, ServerHost};
use mod_platform::testing::with_mods;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

struct PanicAt {
    stopping: AtomicBool,
    save: AtomicBool,
    config_unloading: AtomicBool,
    logout: AtomicBool,
}

struct StoppedProbe {
    serving_stopped: Box<dyn Fn() -> bool + Send + Sync>,
    lan: std::sync::Weak<sunrise_client::lan::Shared>,
    root: std::path::PathBuf,
    save: Arc<sunrise_client::save::SaveDir>,
}

#[test]
#[ignore = "daily"]
fn integrated_stop_finishes_after_an_unencodable_record() {
    use sunrise_client::save::{LevelShape, SaveDir, SessionLock};
    use sunrise_core::nbt::{DISK_WRITE_NESTING_GUARD, Tag};

    struct SaveEvents(Arc<Mutex<Vec<&'static str>>>);
    #[mod_api::__subscriptions]
    impl Hooks for SaveEvents {
        fn level_save(&self, _: &mut dyn ServerHost, _: &mut mod_api::LevelSaveEvent<'_>) {
            self.0.lock().unwrap().push("level saved");
        }
        fn world_saved(&self, _: &mut dyn ServerHost, _: &mut mod_api::WorldSavedEvent) {
            self.0.lock().unwrap().push("world saved");
        }
        fn world_lifecycle(
            &self,
            _: &mut dyn ServerHost,
            event: &mut mod_api::WorldLifecycleEvent<'_>,
        ) {
            match event.phase {
                mod_api::WorldPhase::WorldCloseFinal => self.0.lock().unwrap().push("closed"),
                mod_api::WorldPhase::WorldCloseFailed => {
                    self.0.lock().unwrap().push("close failed")
                }
                _ => (),
            }
        }
    }

    let root = tempfile::tempdir().unwrap();
    let events = Arc::new(Mutex::new(Vec::new()));
    let mods = vec![ModEntry::new(
        "save_events",
        Box::new(SaveEvents(events.clone())),
    )]
    .into();
    with_mods(mods, || {
        with_game(|game| {
            game.world_root = root.path().to_path_buf();
            let open = || {
                SaveDir::new(
                    root.path().to_path_buf(),
                    LevelShape::OVERWORLD,
                    Some(Default::default()),
                )
            };
            game.save = Arc::new(open());
            game.session_lock = Some(SessionLock::acquire(root.path()).unwrap());
            game.world_adopted = true;
            game.native_lifecycle
                .begin_open(mod_api::Authority::IntegratedServer);
            game.native_lifecycle.world_open_ready().unwrap();
            game.save_game().unwrap();
            assert_eq!(*events.lock().unwrap(), ["level saved", "world saved"]);
            events.lock().unwrap().clear();
            game.level.name = "saved on exit after record failure".into();

            let slot = (2, -3);
            let neighbour = (3, -3);
            let deleted = (4, -3);
            let entity = |marker| Tag::compound([("marker", Tag::Int(marker))]);
            game.save
                .save_entities(vec![(slot, vec![entity(1)]), (deleted, vec![entity(3)])])
                .unwrap();
            let invalid = (0..=DISK_WRITE_NESTING_GUARD)
                .fold(Tag::Int(0), |nested, _| Tag::List(vec![nested]));
            game.save
                .save_entities_later(vec![
                    (slot, vec![invalid]),
                    (neighbour, vec![entity(2)]),
                    (deleted, vec![]),
                ])
                .unwrap();

            // An Err here reaches the client through Integrated::stop and makes quitting fatal.
            game.run_server_lifecycle(false, |_, began| {
                *began = true;
                Ok(())
            })
            .unwrap();
            assert!(game.session_lock.is_none());
            let disk = open();
            assert_eq!(disk.load_entities(slot.0, slot.1), [entity(1)]);
            assert_eq!(disk.load_entities(neighbour.0, neighbour.1), [entity(2)]);
            assert!(disk.load_entities(deleted.0, deleted.1).is_empty());
            assert_eq!(
                sunrise_client::save::Level::read(root.path()).unwrap().name,
                "saved on exit after record failure"
            );
            game.save.flush().unwrap();
            assert_eq!(game.save.record_save_failures(), 1);
            assert_eq!(
                *events.lock().unwrap(),
                ["level saved", "world saved", "closed"]
            );
        });
    });
}

#[test]
#[ignore = "daily"]
fn integrated_stop_propagates_metadata_errors_even_after_a_record_failure() {
    use sunrise_client::save::{LevelShape, SaveDir, SessionLock};
    use sunrise_core::nbt::{DISK_WRITE_NESTING_GUARD, Tag};

    with_mods(Vec::new().into(), || {
        with_game(|game| {
            for mixed in [false, true] {
                let root = tempfile::tempdir().unwrap();
                game.world_root = root.path().to_path_buf();
                game.save = Arc::new(SaveDir::new(
                    game.world_root.clone(),
                    LevelShape::OVERWORLD,
                    Some(Default::default()),
                ));
                game.session_lock = Some(SessionLock::acquire(root.path()).unwrap());
                game.world_adopted = true;
                if mixed {
                    let invalid = (0..=DISK_WRITE_NESTING_GUARD)
                        .fold(Tag::Int(0), |nested, _| Tag::List(vec![nested]));
                    game.save
                        .save_entities(vec![((2, 0), vec![invalid])])
                        .unwrap();
                    assert_eq!(game.save.record_save_failures(), 1);
                }
                let blocked = root.path().join("level.dat_new");
                std::fs::create_dir(&blocked).unwrap();
                let expected = std::fs::write(&blocked, b"test").unwrap_err();
                let error = game
                    .run_server_lifecycle(false, |_, began| {
                        *began = true;
                        Ok(())
                    })
                    .unwrap_err();
                assert_eq!(
                    error.downcast_ref::<std::io::Error>().unwrap().kind(),
                    expected.kind()
                );
                assert!(game.session_lock.is_none());
            }
        });
    });
}

/// Exercise the real stop/save paths, not just direct calls to hook dispatch.
#[test]
#[ignore = "daily"]
fn every_started_server_stops_and_saves_even_when_startup_or_ticks_fail() {
    struct Log(
        Arc<Mutex<Vec<&'static str>>>,
        Arc<PanicAt>,
        Arc<Mutex<Option<StoppedProbe>>>,
    );
    #[mod_api::__subscriptions]
    impl Hooks for Log {
        fn register_configs(
            &self,
            _: mod_api::HostSide,
            e: &mut mod_api::RegisterConfigsEvent<'_>,
        ) {
            if self.1.config_unloading.load(Ordering::Relaxed) {
                e.configs
                    .push(mod_api::Config::new("log", mod_api::ConfigKind::Server, ""));
            }
        }
        fn config_unloading(
            &self,
            _: &mut dyn ServerHost,
            _: &mut mod_api::ConfigUnloadingEvent<'_>,
        ) {
            self.0.lock().unwrap().push("config-unloading");
            panic!("config unloading listener");
        }
        fn server_starting(&self, _: &mut dyn ServerHost, _: &mut mod_api::ServerStartingEvent) {
            self.0.lock().unwrap().push("starting");
        }
        fn server_stopping(&self, _: &mut dyn ServerHost, _: &mut mod_api::ServerStoppingEvent) {
            self.0.lock().unwrap().push("stopping");
            if self.1.stopping.load(Ordering::Relaxed) {
                panic!("stopping listener");
            }
        }
        fn world_saved(&self, _: &mut dyn ServerHost, _: &mut mod_api::WorldSavedEvent) {
            self.0.lock().unwrap().push("saved");
            if self.1.save.load(Ordering::Relaxed) {
                panic!("save listener");
            }
        }
        fn player_logged_out(
            &self,
            _: &mut dyn ServerHost,
            _: &mut mod_api::PlayerLoggedOutEvent<'_>,
        ) {
            if self.1.logout.load(Ordering::Relaxed) {
                self.0.lock().unwrap().push("logout");
                panic!("logout listener");
            }
        }
        fn server_stopped(&self, _: &mut dyn ServerHost, _: &mut mod_api::ServerStoppedEvent) {
            if let Some(probe) = self.2.lock().unwrap().as_ref() {
                assert!((probe.serving_stopped)(), "serving retired before Stopped");
                assert!(probe.lan.upgrade().is_none(), "LAN dropped before Stopped");
                let saved = sunrise_client::save::Level::read(&probe.root).unwrap();
                assert_eq!(saved.name, "lifecycle final save");
                assert!(
                    probe.save.raw_chunk(0, 0).is_some(),
                    "dirty chunk saved before Stopped"
                );
                let last = if self.1.logout.load(Ordering::Relaxed) {
                    "logout"
                } else {
                    "saved"
                };
                assert_eq!(self.0.lock().unwrap().last(), Some(&last));
                // Storage is released before the notification, even if the
                // final-save listener itself panicked after writing metadata.
                let _lock = sunrise_client::save::SessionLock::acquire(&probe.root).unwrap();
            }
            self.0.lock().unwrap().push("stopped");
        }
    }
    let log = Arc::new(Mutex::new(Vec::new()));
    let panic_at = Arc::new(PanicAt {
        stopping: AtomicBool::new(false),
        save: AtomicBool::new(false),
        config_unloading: AtomicBool::new(false),
        logout: AtomicBool::new(false),
    });
    let stopped_probe = Arc::new(Mutex::new(None));
    with_mods(
        vec![ModEntry::new(
            "log",
            Box::new(Log(log.clone(), panic_at.clone(), stopped_probe.clone())),
        )]
        .into(),
        || {
            for dedicated in [false, true] {
                for case in [
                    "normal",
                    "startup-error",
                    "tick-error",
                    "panic",
                    "save-error",
                    "unopened",
                    "locked-unadopted",
                    "stopping-panic",
                    "save-panic",
                    "config-unloading-panic",
                    "config-unloading-save-panic",
                    "config-unloading-logout-panic",
                    "before-start",
                ] {
                    log.lock().unwrap().clear();
                    panic_at
                        .stopping
                        .store(case == "stopping-panic", Ordering::Relaxed);
                    panic_at.save.store(
                        matches!(case, "save-panic" | "config-unloading-save-panic"),
                        Ordering::Relaxed,
                    );
                    panic_at
                        .logout
                        .store(case == "config-unloading-logout-panic", Ordering::Relaxed);
                    let config_panic = case.starts_with("config-unloading");
                    panic_at
                        .config_unloading
                        .store(config_panic, Ordering::Relaxed);
                    *stopped_probe.lock().unwrap() = None;
                    let run_case = |game: &mut crate::game::Game<'_>| {
                        game.options.execution_role = if dedicated {
                            ExecutionRole::DedicatedServer
                        } else {
                            ExecutionRole::IntegratedServer
                        };
                        let root = std::env::temp_dir().join(format!(
                            "server-lifecycle-{}-{dedicated}-{case}",
                            std::process::id()
                        ));
                        std::fs::create_dir_all(&root).unwrap();
                        game.world_root = root.clone();
                        game.save = Arc::new(sunrise_client::save::SaveDir::new(
                            root.clone(),
                            sunrise_client::save::LevelShape::OVERWORLD,
                            Some(Default::default()),
                        ));
                        game.level.name = "lifecycle final save".into();
                        let adopted =
                            !matches!(case, "unopened" | "before-start" | "locked-unadopted");
                        if adopted || case == "locked-unadopted" {
                            game.session_lock =
                                Some(sunrise_client::save::SessionLock::acquire(&root).unwrap());
                        }
                        // The lock is held while tags are readable, before the prepared
                        // level replaces the placeholder. Only adoption may save.
                        game.world_adopted = adopted;
                        if matches!(case, "unopened" | "locked-unadopted") {
                            // A failed open must not overwrite an existing, unreadable save.
                            std::fs::write(root.join("level.dat"), b"unadopted save").unwrap();
                        }
                        if config_panic {
                            game.level.name = "previous save".into();
                            game.level.write(&root, &Default::default()).unwrap();
                            // Nothing queues these metadata/block changes. A flush
                            // without the final save must leave the old name and no chunk.
                            game.level.name = "lifecycle final save".into();
                            game.world.chunks.insert(
                                (0, 0),
                                Arc::new(sunrise_client::chunk::Chunk::empty(0, 0)),
                            );
                            let stone = game
                                .pack
                                .assets
                                .registry
                                .default_state("minecraft:stone")
                                .unwrap();
                            game.world.set(1, 64, 1, stone).unwrap();
                            assert!(game.save.raw_chunk(0, 0).is_none());
                            let serving_stopped = if dedicated {
                                Box::new(|| true) as Box<dyn Fn() -> bool + Send + Sync>
                            } else {
                                // Cover both a published and an unpublished integrated
                                // server. Neither may merely unpublish on shutdown.
                                Box::new(super::super::integrated::tests::install_serving(
                                    game,
                                    case == "config-unloading-save-panic",
                                ))
                            };
                            let lan = Arc::downgrade(&game.lan.as_ref().unwrap().hub.lan.shared);
                            *stopped_probe.lock().unwrap() = Some(StoppedProbe {
                                serving_stopped,
                                lan,
                                root: root.clone(),
                                save: game.save.clone(),
                            });
                            game.native_configs_server_start(&root, dedicated);
                        }
                        let result = game.run_server_lifecycle(dedicated, |game, began| {
                            if case == "before-start" {
                                return Err("before startup".into());
                            }
                            *began = true;
                            game.native_server_about_to_start(dedicated);
                            game.native_server_starting(dedicated);
                            game.native_lifecycle_open_started();
                            if matches!(case, "startup-error" | "unopened" | "locked-unadopted") {
                                return Err("failed init".into());
                            }
                            game.native_lifecycle_world_ready();
                            match case {
                                "tick-error" => Err("failed tick".into()),
                                "panic" => panic!("failed tick"),
                                "save-error" => {
                                    let file = root.join("not-a-directory");
                                    std::fs::write(&file, b"occupied").unwrap();
                                    game.world_root = file;
                                    Ok(())
                                }
                                _ => Ok(()),
                            }
                        });
                        assert_eq!(result.is_ok(), case == "normal", "{dedicated} {case}");
                        if config_panic || matches!(case, "stopping-panic" | "save-panic") {
                            let message = result.unwrap_err().to_string();
                            let needle = if config_panic {
                                "config unloading listener"
                            } else if case == "stopping-panic" {
                                "stopping listener"
                            } else {
                                "save listener"
                            };
                            assert!(message.contains(needle), "{dedicated} {case}: {message}");
                            if config_panic {
                                assert_eq!(
                                    message,
                                    "the server shutdown panicked: config unloading listener"
                                );
                            }
                        }
                        let expected: &[&str] = match case {
                            "before-start" => &[],
                            "unopened" | "save-error" | "locked-unadopted" => {
                                &["starting", "stopping", "stopped"]
                            }
                            "config-unloading-panic" | "config-unloading-save-panic" => &[
                                "starting",
                                "stopping",
                                "config-unloading",
                                "saved",
                                "stopped",
                            ],
                            "config-unloading-logout-panic" => &[
                                "starting",
                                "stopping",
                                "config-unloading",
                                "saved",
                                "logout",
                                "stopped",
                            ],
                            _ => &["starting", "stopping", "saved", "stopped"],
                        };
                        assert_eq!(*log.lock().unwrap(), expected, "{dedicated} {case}");
                        assert!(game.session_lock.is_none(), "storage released after {case}");
                        assert!(!game.world_adopted, "adoption cleared after {case}");
                        assert!(game.serving.is_none(), "serving retired after {case}");
                        assert!(game.lan.is_none(), "LAN dropped after {case}");
                        if config_panic {
                            let registry = &game.pack.assets.registry;
                            let chunk =
                                game.save.load(0, 0, registry, world_biomes!(game)).unwrap();
                            assert_eq!(
                                chunk.block(1, 64, 1),
                                registry.default_state("minecraft:stone").unwrap()
                            );
                        }
                        if matches!(case, "unopened" | "locked-unadopted") {
                            assert_eq!(
                                std::fs::read(root.join("level.dat")).unwrap(),
                                b"unadopted save",
                                "{dedicated} {case}"
                            );
                        } else if !matches!(case, "save-error" | "before-start") {
                            let saved = sunrise_client::save::Level::read(&root).unwrap();
                            assert_eq!(saved.name, "lifecycle final save");
                        }
                        std::fs::remove_dir_all(root).unwrap();
                    };
                    if config_panic {
                        with_guests(run_case);
                    } else {
                        with_game(run_case);
                    }
                }
            }
        },
    );
}
