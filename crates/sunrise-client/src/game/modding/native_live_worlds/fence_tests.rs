//! A real OS write refusal fences Game, followed by recovery/loading in another process.
use super::*;
use sunrise_client::save::{SaveDir, SessionLock, dimension_dir, live_world_journal as wal};
struct Fixture;
#[mod_api::__subscriptions]
impl mod_api::Hooks for Fixture {}
#[test]
#[ignore = "daily"]
#[cfg(unix)]
fn post_prepared_os_failure_fences_game_and_fresh_process_recovers_before_loading() {
    use std::{os::unix::fs::PermissionsExt, process::Command};
    const FLAG: &str = "SUNRISE_LIVE_FENCE_PHASE";
    const ROOT: &str = "SUNRISE_LIVE_FENCE_ROOT";
    let Ok(phase) = std::env::var(FLAG) else {
        let root = tempfile::tempdir().unwrap();
        for phase in ["failure", "recover"] {
            let scratch = root.path().join(format!("harness-{phase}"));
            std::fs::create_dir(&scratch).unwrap();
            let mut command = Command::new(std::env::current_exe().unwrap());
            command.args(["--exact", "game::native_live_worlds::fence_tests::post_prepared_os_failure_fences_game_and_fresh_process_recovers_before_loading", "--ignored", "--nocapture", "--test-threads=1"])
                .current_dir(&scratch).env(FLAG, phase).env(ROOT, root.path());
            for key in [
                "SUNRISE_HOME",
                "SUNRISE_LAUNCHER_HOME",
                "XDG_DATA_HOME",
                "XDG_CONFIG_HOME",
                "XDG_RUNTIME_DIR",
                "SUNRISE_LAUNCHER_TRASH",
                "SUNRISE_LAUNCHER_CLIPBOARD",
                "SUNRISE_LAUNCHER_DESKTOP",
            ] {
                let home = scratch.join(key);
                std::fs::create_dir(&home).unwrap();
                command.env(key, home);
            }
            let status = command.status().unwrap();
            assert!(
                status.success(),
                "actual fresh process {phase} failed: {status}"
            );
        }
        return;
    };
    let root = std::path::PathBuf::from(std::env::var_os(ROOT).unwrap());
    mod_platform::testing::with_mods(
        vec![mod_api::ModEntry::new("live_fixture", Box::new(Fixture))].into(),
        || {
            super::super::native_entity_refs::tests::with_game(|game| {
                let _phase =
                    mod_api::__private::PhaseOwner::scope(mod_api::__private::ModPhase::World);
                let _owner =
                    mod_api::content::crash::enter_native_mod(mod_api::content::crash::ModCall {
                        id: "live_fixture",
                        hook: "server_tick",
                        part: None,
                    });
                game.world_root = root.clone();
                game.session_lock = Some(SessionLock::acquire(&root).unwrap());
                if phase == "recover" {
                    assert!(wal::recover(&root, wal::Limits::default()).unwrap());
                    assert!(!wal::recover(&root, wal::Limits::default()).unwrap());
                }
                game.save = Arc::new(SaveDir::new(
                    dimension_dir(&root, game.dim.id()),
                    game.world.shape,
                    Some(Default::default()),
                ));
                game.options.dedicated = Some(Vec::new());
                game.native_lifecycle_world_ready();
                if phase == "failure" {
                    game.save_game_metadata().unwrap();
                    let source = super::super::native_live_worlds_tests::prepare_named(
                        game,
                        "live_fixture:fence",
                    );
                    let parent = game.level("minecraft:overworld").unwrap();
                    source
                        .project(
                            game,
                            parent,
                            &[[0, 64, 0], [1, 64, 0], [17, 64, 0]],
                            LiveWorldPose::at([10.0, 0.0, 10.0]),
                            LiveWorldLimits::default(),
                        )
                        .unwrap();
                    let directory = dimension_dir(&root, source.dimension_id()).join("region");
                    std::fs::create_dir_all(&directory).unwrap();
                    std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o555))
                        .unwrap();
                    assert!(
                        std::fs::write(directory.join("permission-probe"), b"refuse").is_err(),
                        "actual OS permission refusal is required"
                    );
                    let result = source.checkpoint(game);
                    // Restore only this fixture's directory permission so the next recovered process can write.
                    std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o755))
                        .unwrap();
                    assert_eq!(result, Err(Error::Storage));
                    assert!(game.live_world_session_failed());
                    let before = game
                        .native_level_world(source)
                        .unwrap()
                        .block_entities
                        .content_revision();
                    assert!(game.tick_server(false).is_err());
                    assert_eq!(
                        game.native_level_world(source)
                            .unwrap()
                            .block_entities
                            .content_revision(),
                        before
                    );
                    assert!(game.save_game_with(true).is_err());
                    assert!(game.save_level(true).is_err());
                    assert!(game.admit_level_save().is_err());
                    assert!(root.join("data/sunrise_live_world_journal.bin").is_file());
                    eprintln!(
                        "LIVE_WORLD_FENCE actual_os_write_failure=true further_tick_and_save_refused=true"
                    );
                } else {
                    let source = game
                        .runtime_level_open(
                            "live_fixture:fence",
                            mod_api::RuntimeLevel::new(
                                "minecraft:overworld",
                                mod_api::RuntimeGenerator::Void,
                                1022,
                            ),
                        )
                        .unwrap();
                    game.apply_runtime_levels();
                    // Discard the unrelated with_game Parent through its ordinary close/open path;
                    // the reopened Streaming and SaveDir must BOTH read this recovered root.
                    game.enter_level(super::super::Dim::from_id(source.dimension_id()))
                        .unwrap();
                    game.close_parked_level(super::super::Dim::Overworld)
                        .unwrap();
                    game.enter_level(super::super::Dim::Overworld).unwrap();
                    for dim in [
                        super::super::Dim::Overworld,
                        super::super::Dim::from_id(source.dimension_id()),
                    ] {
                        game.enter_level(dim).unwrap();
                        game.streaming.paused = false;
                        game.streaming.forced.extend([(0, 0), (1, 0)]);
                        for _ in 0..2000 {
                            game.update_world();
                            if game.world.chunks.contains_key(&(0, 0))
                                && game.world.chunks.contains_key(&(1, 0))
                            {
                                break;
                            }
                            game.streaming.wait(std::time::Duration::from_millis(5));
                        }
                        assert!(
                            game.world.chunks.contains_key(&(0, 0))
                                && game.world.chunks.contains_key(&(1, 0))
                        );
                        game.tick_world(false).unwrap();
                    }
                    let Some(sunrise_client::containers::BlockEntity::Chest(items)) =
                        game.world.block_entities.get(&(0, 64, 0))
                    else {
                        panic!("real chest did not recover");
                    };
                    assert_eq!(items[0].as_ref().unwrap().count, 7);
                    let Some(sunrise_client::containers::BlockEntity::Furnace(furnace)) =
                        game.world.block_entities.get(&(1, 64, 0))
                    else {
                        panic!("real furnace did not recover");
                    };
                    assert_eq!(furnace.cooking_timer, 40);
                    game.enter_level(super::super::Dim::Overworld).unwrap();
                    let binding = source.restore_projection(game).unwrap();
                    assert_eq!(binding.cells(game).unwrap().len(), 3);
                    assert!(!game.live_world_session_failed());
                    eprintln!(
                        "LIVE_WORLD_FENCE fresh_process_recovered_before_source_load=true inventory=7 counter=40"
                    );
                }
            });
        },
    );
}
