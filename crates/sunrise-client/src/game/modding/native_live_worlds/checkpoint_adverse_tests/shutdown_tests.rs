//! Real normal teardown, including the post-initial-save leaving-player queue.
use super::*;

struct StopProbe {
    late_write: bool,
    remove_guest_after_save: bool,
    guest_remove_outcome: Mutex<Option<bool>>,
    after_saved: AtomicBool,
    worlds_saved: AtomicUsize,
    final_notices: AtomicUsize,
    source_seen: AtomicBool,
    write_outcome: Mutex<Option<std::result::Result<(), AttachmentError>>>,
}
struct StopFixture(Arc<StopProbe>);
#[mod_api::__subscriptions]
impl Hooks for StopFixture {
    fn register(&self, registrar: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
        registrar
            .register_attachment_type("pose_fixture:publication", false)
            .unwrap();
    }
    fn world_saved(&self, host: &mut dyn ServerHost, _: &mut mod_api::WorldSavedEvent) {
        self.0.worlds_saved.fetch_add(1, Ordering::SeqCst);
        self.0.after_saved.store(true, Ordering::SeqCst);
        if self.0.remove_guest_after_save {
            let outcome = host
                .player(123)
                .map(|guest| guest.set_inventory_slot(host, 0, None).is_ok());
            *self.0.guest_remove_outcome.lock().unwrap() = outcome;
        }
    }
    fn player_save_to_file(
        &self,
        host: &mut dyn ServerHost,
        _: &mut mod_api::PlayerSaveToFileEvent<'_>,
    ) {
        if !self.0.after_saved.load(Ordering::SeqCst) {
            return;
        }
        self.0.final_notices.fetch_add(1, Ordering::SeqCst);
        if self.0.late_write && self.0.after_saved.swap(false, Ordering::SeqCst) {
            let source = host.level(A);
            self.0.source_seen.store(source.is_some(), Ordering::SeqCst);
            if let Some(source) = source {
                *self.0.write_outcome.lock().unwrap() = Some(PUBLICATION.set(
                    &mut source.attachments(host),
                    Holder::Level,
                    metadata(91),
                ));
            }
        }
    }
}

#[test]
#[ignore = "daily"]
fn normal_shutdown_checkpoints_leaving_players_and_rejects_late_notices() {
    const NAME: &str = "game::native_live_worlds_tests::checkpoint_adverse_tests::shutdown_tests::normal_shutdown_checkpoints_leaving_players_and_rejects_late_notices";
    const FLAG: &str = "SUNRISE_COMMON_SHUTDOWN_CHILD";
    if super::super::super::native_entity_refs::tests::run_fresh_cases(
        NAME,
        FLAG,
        &["projected", "late-notice", "ordinary"],
    ) {
        return;
    }
    let case = std::env::var(FLAG).unwrap();
    let probe = Arc::new(StopProbe {
        late_write: case == "late-notice",
        remove_guest_after_save: case == "projected",
        guest_remove_outcome: Mutex::new(None),
        after_saved: AtomicBool::new(false),
        worlds_saved: AtomicUsize::new(0),
        final_notices: AtomicUsize::new(0),
        source_seen: AtomicBool::new(false),
        write_outcome: Mutex::new(None),
    });
    actual_game(
        vec![ModEntry::new(
            "pose_fixture",
            Box::new(StopFixture(probe.clone())),
        )],
        |game, root| {
            game.world_adopted = true;
            let source = if case != "ordinary" {
                let source = prepare_named(game, A);
                let destination = game.level("minecraft:overworld").unwrap();
                source
                    .project(
                        game,
                        destination,
                        MEMBERS,
                        LiveWorldPose::at([10., 0., 10.]),
                        LiveWorldLimits::default(),
                    )
                    .unwrap();
                PUBLICATION
                    .set(&mut source.attachments(game), Holder::Level, metadata(77))
                    .unwrap();
                Some(source)
            } else {
                None
            };
            let guest = actual_guest_and_owner(game);
            // Return both real cursors through the ordinary close path, preserving items.
            game.close_container();
            game.as_guest(guest, |g| g.close_container()).unwrap();
            let source_save = source.map(|s| {
                game.levels
                    .parked_save_of(s.dimension_id())
                    .unwrap()
                    .clone()
            });
            let (before_epoch, before_chunk) = if let Some(source) = source {
                let receipt = source.checkpoint(game).unwrap();
                assert!(game.player_autosaves.writes_finished().unwrap());
                assert!(
                    source_save.as_ref().unwrap().flush().is_err(),
                    "an active common lease still refuses independent flush after checkpoint"
                );
                (
                    receipt.barrier_epoch,
                    Some(source_save.as_ref().unwrap().raw_chunk(0, 0).unwrap().tag),
                )
            } else {
                assert!(!game.player_autosaves.common_required());
                (0, None)
            };
            if let Some(source) = source {
                let world = game.native_level_world(source).unwrap();
                assert!(
                    matches!(world.block_entities.get(&(0,64,0)), Some(sunrise_client::containers::BlockEntity::Chest(items)) if items[0].as_ref().unwrap().count==7)
                );
                assert!(
                    matches!(world.block_entities.get(&(1,64,0)), Some(sunrise_client::containers::BlockEntity::Furnace(f)) if f.cooking_timer==40)
                );
            }
            let before_attachment = source.map(|_| {
                sunrise_client::save::read_dat(
                    &dimension_dir(root, A).join("data/sunrise/attachments.dat"),
                )
                .unwrap()
                .get("data")
                .unwrap()
                .clone()
            });
            if let Some(chunk) = &before_chunk {
                let Some(sunrise_core::nbt::Tag::List(entities)) = chunk.get("block_entities")
                else {
                    panic!("real encoded block entities missing");
                };
                for expected in ["minecraft:chest", "minecraft:furnace"] {
                    assert!(entities.iter().any(|entity| matches!(entity.get("id"), Some(sunrise_core::nbt::Tag::String(id)) if id == &sunrise_core::nbt::JavaString::from(expected))));
                }
            }
            // This is the ordinary integrated stop path, not a direct new-helper invocation.
            let stopped = game.stop_integrated(false);
            assert!(game.lan.is_none());
            assert!(game.session_lock.is_none());
            assert!(probe.worlds_saved.load(Ordering::SeqCst) > 0);
            if case == "late-notice" {
                assert!(stopped.is_err());
                assert!(game.live_world_session_failed());
                assert!(probe.source_seen.load(Ordering::SeqCst));
                assert_eq!(*probe.write_outcome.lock().unwrap(), Some(Ok(())));
                let epoch = wal::read_epoch(root).unwrap();
                assert!(
                    game.save_game().is_err(),
                    "a refused final cut stays fenced"
                );
                assert_eq!(wal::read_epoch(root).unwrap(), epoch);
            } else {
                stopped.unwrap();
                assert!(!game.live_world_session_failed());
                assert!(game.player_autosaves.writes_finished().unwrap());
                if source.is_some() {
                    assert!(probe.final_notices.load(Ordering::SeqCst) > 0);
                    assert!(wal::read_epoch(root).unwrap() >= before_epoch + 2);
                }
            }
            if let Some(source_save) = source_save {
                assert_eq!(
                    source_save.raw_chunk(0, 0).unwrap().tag,
                    before_chunk.unwrap()
                );
                let attachment = sunrise_client::save::read_dat(
                    &dimension_dir(root, A).join("data/sunrise/attachments.dat"),
                )
                .unwrap();
                assert_eq!(
                    attachment.get("data"),
                    before_attachment.as_ref(),
                    "a late-notice write is not silently persisted or called clean"
                );
                if case == "late-notice" {
                    assert!(
                        source_save.flush().is_err(),
                        "the failed final cut stays fenced"
                    );
                } else {
                    assert!(!source_save.live_world_barrier_pending());
                    assert!(
                        source_save.flush().is_ok(),
                        "a fully closed clean level has released its leases"
                    );
                }
            } else {
                assert!(game.save.flush().is_ok());
            }
            for uuid in [123, 777] {
                assert!(sunrise_client::lan::playerdata::path(root, uuid).is_file());
            }
            // Ordinary reader entry starts under a newly acquired session lock.
            let _lock = SessionLock::acquire(root).unwrap();
            wal::recover(root, wal::Limits::default()).unwrap();
            for uuid in [123, 777] {
                let tag = sunrise_client::save::read_dat(&sunrise_client::lan::playerdata::path(
                    root, uuid,
                ))
                .unwrap();
                let Some(sunrise_core::nbt::Tag::List(items)) = tag.get("Inventory") else {
                    panic!("actual saved inventory is missing");
                };
                if case == "projected" && uuid == 123 {
                    assert_eq!(*probe.guest_remove_outcome.lock().unwrap(), Some(true));
                    assert!(!items.iter().any(|item| matches!(item.get("id"), Some(sunrise_core::nbt::Tag::String(id)) if id == &sunrise_core::nbt::JavaString::from("minecraft:diamond"))), "the leaving snapshot after WorldSaved supersedes the earlier inventory image");
                }
            }
        },
    );
}
