//! Actual Game common-checkpoint, callback and prepared ownership handoff regressions.
//! Every mutation/save goes through the real Game, public handles, ordinary Host transaction or
//! existing saved-entity load queue. No fake Host/World, success receipt or WAL decision is constructed.
use super::*;
use mod_api::{AttachmentError, Holder, HostTransaction as _, ServerHost};
use std::{
    path::{Path, PathBuf},
    sync::{
        Mutex,
        atomic::{AtomicBool, AtomicUsize},
    },
};
use sunrise_client::save::{SaveDir, SessionLock, dimension_dir, live_world_journal as wal};

const A: &str = "pose_fixture:a";
const B: &str = "pose_fixture:b";
const C: &str = "pose_fixture:metadata";
const MEMBERS: &[[i32; 3]] = &[[0, 64, 0], [1, 64, 0], [17, 64, 0]];

fn fresh(name: &str) -> bool {
    super::super::native_entity_refs::tests::run_fresh_cases(
        name,
        "SUNRISE_CHECKPOINT_ADVERSE_CHILD",
        &["actual-game"],
    )
}
fn actual_game(mods: Vec<ModEntry>, run: impl FnOnce(&mut Game<'_>, &Path)) {
    mod_platform::testing::with_mods(mods.into(), || {
        super::super::native_entity_refs::tests::with_game(|game| {
            let _phase = mod_api::__private::PhaseOwner::scope(mod_api::__private::ModPhase::World);
            let scratch = tempfile::tempdir().unwrap();
            game.world_root = scratch.path().to_path_buf();
            game.session_lock = Some(SessionLock::acquire(scratch.path()).unwrap());
            // with_game's initial SaveDir is another scratch folder. Align the actual current
            // owner too: the real common writer must never publish its destination elsewhere.
            game.save = Arc::new(SaveDir::new(
                dimension_dir(scratch.path(), game.dim.id()),
                game.world.shape,
                Some(Default::default()),
            ));
            game.options.dedicated = Some(Vec::new());
            game.native_lifecycle_world_ready();
            let _owner =
                mod_api::content::crash::enter_native_mod(mod_api::content::crash::ModCall {
                    id: "pose_fixture",
                    hook: "server_tick",
                    part: None,
                });
            run(game, scratch.path());
            game.session_lock.take();
        });
    });
}
fn empty_owned(game: &mut Game<'_>) -> mod_api::LevelRef {
    let level = game
        .runtime_level_open(
            C,
            RuntimeLevel::new("minecraft:overworld", RuntimeGenerator::Void, 1022),
        )
        .unwrap();
    game.apply_runtime_levels();
    assert!(game.native_level_world(level).unwrap().chunks.is_empty());
    assert_eq!(level.live_world(game).unwrap(), None);
    level
}
fn pose_bits(sample: mod_api::LiveWorldSample) -> Vec<u64> {
    [sample.previous, sample.current]
        .into_iter()
        .flat_map(|pose| {
            pose.translation
                .into_iter()
                .chain(pose.pivot)
                .chain(pose.rotation)
                .map(f64::to_bits)
        })
        .collect()
}
fn metadata(n: i64) -> mod_api::Compound {
    [("n".into(), mod_api::Tag::Long(n))].into()
}
fn core_images(root: &Path, ids: &[&str]) -> Vec<(PathBuf, Option<Vec<u8>>)> {
    let mut paths = vec![root.join("data/sunrise_live_world_epoch.bin")];
    for id in ids {
        let level = dimension_dir(root, id);
        for file in [
            "region/r.0.0.mca",
            "entities/r.0.0.mca",
            "poi/r.0.0.mca",
            "data/sunrise/attachments.dat",
            "data/sunrise_live_world_projection.json",
        ] {
            paths.push(level.join(file));
        }
    }
    paths
        .into_iter()
        .map(|path| {
            let bytes = match std::fs::read(&path) {
                Ok(bytes) => Some(bytes),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
                Err(e) => panic!("{}: {e}", path.display()),
            };
            (path, bytes)
        })
        .collect()
}
fn assert_saved_empty_metadata(game: &Game<'_>, root: &Path, level: mod_api::LevelRef) {
    let directory = dimension_dir(root, level.dimension_id());
    let file = directory.join("data/sunrise/attachments.dat");
    assert!(file.is_file(), "actual metadata afterimage must be on disk");
    assert!(
        !directory.join("region").exists(),
        "an empty owner needs no invented chunk"
    );
    let saved = sunrise_client::save::read_dat(&file).unwrap();
    assert_eq!(
        saved.get("data"),
        Some(&sunrise_core::nbt::Tag::Compound(
            game.native_level_world(level)
                .unwrap()
                .attachments
                .try_to_nbt()
                .unwrap()
        ))
    );
    assert!(game.native_level_world(level).unwrap().chunks.is_empty());
}

#[test]
#[ignore = "daily"]
fn two_source_root_abort_restores_exact_poses_and_third_empty_attachment_then_retries() {
    if fresh(
        "game::native_live_worlds_tests::checkpoint_adverse_tests::two_source_root_abort_restores_exact_poses_and_third_empty_attachment_then_retries",
    ) {
        return;
    }
    actual_game(
        vec![ModEntry::new("pose_fixture", Box::new(PublicationFixture))],
        |game, root| {
            let a = prepare_named(game, A);
            let b = prepare_named(game, B);
            let c = empty_owned(game);
            let destination = game.level("minecraft:overworld").unwrap();
            let pa = a
                .project(
                    game,
                    destination,
                    MEMBERS,
                    LiveWorldPose::at([10., 16., 10.]),
                    LiveWorldLimits::default(),
                )
                .unwrap();
            let pb = b
                .project(
                    game,
                    destination,
                    MEMBERS,
                    LiveWorldPose::at([10., 16., -10.]),
                    LiveWorldLimits::default(),
                )
                .unwrap();
            let sa = pa.sample(game).unwrap();
            let sb = pb.sample(game).unwrap();
            let ka = game.capture_projection_pose(pa).unwrap();
            let kb = game.capture_projection_pose(pb).unwrap();
            PUBLICATION
                .set(&mut a.attachments(game), Holder::Level, publication(sa))
                .unwrap();
            PUBLICATION
                .set(&mut b.attachments(game), Holder::Level, publication(sb))
                .unwrap();
            assert_eq!(
                PUBLICATION
                    .get_existing(&mut c.attachments(game), Holder::Level)
                    .unwrap(),
                None
            );
            let mut next_a = sa.current;
            next_a.translation[0] += 0.125;
            let mut next_b = sb.current;
            next_b.translation[2] -= 0.125;
            let tick = game.ticks;
            let files = core_images(root, &[A, B, C]);
            let epoch = wal::read_epoch(root).unwrap();
            let refused = (game as &mut dyn ServerHost).transaction(|host, _| {
                let aa = pa.publish_pose(host, sa.revision, next_a).unwrap();
                let bb = pb.publish_pose(host, sb.revision, next_b).unwrap();
                PUBLICATION.set(&mut a.attachments(host), Holder::Level, publication(aa))?;
                PUBLICATION.set(&mut b.attachments(host), Holder::Level, publication(bb))?;
                PUBLICATION.set(&mut c.attachments(host), Holder::Level, metadata(73))?;
                BAD_PUBLICATION.set(&mut c.attachments(host), Holder::Level, metadata(99))
            });
            assert_eq!(refused, Err(AttachmentError::Encode));
            assert_eq!(pose_bits(pa.sample(game).unwrap()), pose_bits(sa));
            assert_eq!(pose_bits(pb.sample(game).unwrap()), pose_bits(sb));
            assert_eq!(
                game.capture_projection_pose(pa).unwrap(),
                ka,
                "root restores actual once-tick admission"
            );
            assert_eq!(game.capture_projection_pose(pb).unwrap(), kb);
            assert_eq!(
                PUBLICATION
                    .get_existing(&mut a.attachments(game), Holder::Level)
                    .unwrap(),
                Some(publication(sa))
            );
            assert_eq!(
                PUBLICATION
                    .get_existing(&mut b.attachments(game), Holder::Level)
                    .unwrap(),
                Some(publication(sb))
            );
            assert_eq!(
                PUBLICATION
                    .get_existing(&mut c.attachments(game), Holder::Level)
                    .unwrap(),
                None
            );
            assert_eq!(game.ticks, tick);
            assert_eq!(wal::read_epoch(root).unwrap(), epoch);
            assert_eq!(core_images(root, &[A, B, C]), files);
            let (aa, bb) = (game as &mut dyn ServerHost)
                .transaction(|host, _| {
                    let aa = pa.publish_pose(host, sa.revision, next_a).unwrap();
                    let bb = pb.publish_pose(host, sb.revision, next_b).unwrap();
                    PUBLICATION.set(&mut a.attachments(host), Holder::Level, publication(aa))?;
                    PUBLICATION.set(&mut b.attachments(host), Holder::Level, publication(bb))?;
                    PUBLICATION.set(&mut c.attachments(host), Holder::Level, metadata(73))?;
                    Ok::<_, AttachmentError>((aa, bb))
                })
                .unwrap();
            assert_eq!(
                (aa.revision, bb.revision),
                (sa.revision + 1, sb.revision + 1)
            );
            assert_eq!(game.ticks, tick);
            assert_eq!(
                wal::read_epoch(root).unwrap(),
                epoch,
                "successful publication roots do not save whole worlds per tick"
            );
            assert_eq!(
                core_images(root, &[A, B, C]),
                files,
                "publication retains a gated in-memory save group until an actual barrier"
            );
            let saved = a.checkpoint(game).unwrap();
            for level in [a, b, c] {
                assert!(saved.levels.iter().any(|row| row.level == level));
            }
            let empty = saved.levels.iter().find(|row| row.level == c).unwrap();
            assert_eq!(empty.binding, None);
            assert_eq!(empty.retired_members, None);
            assert_eq!(
                empty.block_revision,
                c.revisions(game).unwrap().block_revision
            );
            assert_eq!(wal::read_epoch(root).unwrap(), saved.barrier_epoch);
            assert!(saved.barrier_epoch > epoch);
            assert_saved_empty_metadata(game, root, c);
        },
    );
}

struct JoinTarget {
    armed: Arc<AtomicBool>,
    joins: Arc<AtomicUsize>,
    target: Option<&'static str>,
    results: Arc<Mutex<Vec<Result<(), AttachmentError>>>>,
}
#[mod_api::__subscriptions]
impl Hooks for JoinTarget {
    fn register(&self, r: &mut dyn mod_api::Registrar, context: &mod_api::RegistrationContext) {
        PublicationFixture.register(r, context);
    }
    fn entity_join_level(
        &self,
        host: &mut dyn ServerHost,
        event: &mut mod_api::EntityJoinLevelEvent<'_>,
    ) -> mod_api::Flow {
        if event.kind == "minecraft:marker" && event.dimension == A && event.loaded_from_disk {
            self.joins.fetch_add(1, Ordering::SeqCst);
            if self.armed.swap(false, Ordering::SeqCst)
                && let Some(id) = self.target
            {
                let level = host.level(id).unwrap();
                let result =
                    PUBLICATION.set(&mut level.attachments(host), Holder::Level, metadata(91));
                self.results.lock().unwrap().push(result);
            }
        }
        mod_api::Flow::Continue
    }
}
// Create a genuine stock entity through the ordinary command, serialize/remove it through the
// existing EntityStorage path, then place that actual saved record on the ordinary load queue.
fn queue_actual_saved_marker(game: &mut Game<'_>, source: mod_api::LevelRef) {
    game.enter_level(Dim::from_id(source.dimension_id()))
        .unwrap();
    game.world
        .chunks
        .insert((2, 0), Arc::new(Chunk::empty(2, 0)));
    game.streaming.forced.insert((2, 0));
    game.build_registries();
    game.native_joins_skip();
    assert!(game.as_console(|g| g.run_command("summon marker 33.5 70 0.5 {UUID:[I;29,0,0,71]}")));
    let mut saved = game.entity_chunks(&std::collections::HashSet::from([(2, 0)]), true);
    assert_eq!(saved.len(), 1);
    let (column, tags) = saved.pop().unwrap();
    assert_eq!(column, (2, 0));
    assert_eq!(tags.len(), 1);
    assert_eq!(
        tags[0].get("id"),
        Some(&sunrise_core::nbt::Tag::String("minecraft:marker".into()))
    );
    game.native_leaves_flush();
    game.world.queue_entity_nbt(column, tags);
    game.enter_level(Dim::Overworld).unwrap();
}

#[test]
#[ignore = "daily"]
fn actual_join_after_selection_retains_third_target_and_refuses_before_any_prepared_write() {
    if fresh(
        "game::native_live_worlds_tests::checkpoint_adverse_tests::actual_join_after_selection_retains_third_target_and_refuses_before_any_prepared_write",
    ) {
        return;
    }
    let armed = Arc::new(AtomicBool::new(false));
    let joins = Arc::new(AtomicUsize::new(0));
    let results = Arc::new(Mutex::new(Vec::new()));
    let hook = JoinTarget {
        armed: armed.clone(),
        joins: joins.clone(),
        target: Some(C),
        results: results.clone(),
    };
    actual_game(
        vec![ModEntry::new("pose_fixture", Box::new(hook))],
        |game, root| {
            let source = prepare_named(game, A);
            let c = empty_owned(game);
            let destination = game.level("minecraft:overworld").unwrap();
            let projection = source
                .project(
                    game,
                    destination,
                    MEMBERS,
                    LiveWorldPose::at([10., 16., 10.]),
                    LiveWorldLimits::default(),
                )
                .unwrap();
            queue_actual_saved_marker(game, source);
            assert_eq!(
                projection.cells(game).unwrap().len(),
                3,
                "selected source columns remain genuinely readable"
            );
            let images = core_images(root, &[A, C]);
            let epoch = wal::read_epoch(root).unwrap();
            armed.store(true, Ordering::SeqCst);
            assert_eq!(source.checkpoint(game), Err(LiveWorldError::Revision));
            assert_eq!(joins.load(Ordering::SeqCst), 1);
            assert_eq!(*results.lock().unwrap(), vec![Ok(())]);
            assert_eq!(wal::read_epoch(root).unwrap(), epoch);
            assert_eq!(core_images(root, &[A, C]), images);
            assert!(!root.join("data/sunrise_live_world_journal.bin").exists());
            assert_eq!(
                PUBLICATION
                    .get_existing(&mut c.attachments(game), Holder::Level)
                    .unwrap(),
                Some(metadata(91)),
                "actual join write remains live for a new decision"
            );
            let receipt = source.checkpoint(game).unwrap();
            assert!(
                receipt
                    .levels
                    .iter()
                    .any(|row| row.level == c && row.binding.is_none())
            );
            assert_eq!(
                joins.load(Ordering::SeqCst),
                1,
                "fresh retry never reloads/replays the accepted entity join"
            );
            assert_eq!(*results.lock().unwrap(), vec![Ok(())]);
            assert_saved_empty_metadata(game, root, c);
        },
    );
}

#[test]
#[ignore = "daily"]
fn binding_create_and_retire_abort_restore_actual_handle_then_nonempty_receipt_counts_members() {
    if fresh(
        "game::native_live_worlds_tests::checkpoint_adverse_tests::binding_create_and_retire_abort_restore_actual_handle_then_nonempty_receipt_counts_members",
    ) {
        return;
    }
    actual_game(
        vec![ModEntry::new("pose_fixture", Box::new(PublicationFixture))],
        |game, _| {
            let source = prepare_named(game, A);
            let destination = game.level("minecraft:overworld").unwrap();
            let pose = LiveWorldPose::at([10., 16., 10.]);
            let tick = game.ticks;
            let mut canceled = None;
            let refused = (game as &mut dyn ServerHost).transaction(|host, _| {
                canceled = Some(
                    source
                        .project(host, destination, MEMBERS, pose, LiveWorldLimits::default())
                        .unwrap(),
                );
                BAD_PUBLICATION.set(&mut source.attachments(host), Holder::Level, metadata(1))
            });
            assert_eq!(refused, Err(AttachmentError::Encode));
            let stale = canceled.unwrap();
            assert_eq!(source.live_world(game).unwrap(), None);
            assert_eq!(stale.sample(game), Err(LiveWorldError::Stale));
            let current = source
                .project(game, destination, MEMBERS, pose, LiveWorldLimits::default())
                .unwrap();
            assert_ne!(
                current, stale,
                "aborted handles must never alias a later binding"
            );
            let sample = current.sample(game).unwrap();
            let members = current.cells(game).unwrap();
            let kept = game.capture_projection_pose(current).unwrap();
            let refused = (game as &mut dyn ServerHost).transaction(|host, _| {
                current.retire(host).unwrap();
                BAD_PUBLICATION.set(&mut source.attachments(host), Holder::Level, metadata(2))
            });
            assert_eq!(refused, Err(AttachmentError::Encode));
            assert_eq!(source.live_world(game).unwrap(), Some(current));
            assert_eq!(pose_bits(current.sample(game).unwrap()), pose_bits(sample));
            assert_eq!(game.capture_projection_pose(current).unwrap(), kept);
            assert_eq!(current.cells(game).unwrap(), members);
            assert_eq!(game.ticks, tick);
            current.retire(game).unwrap();
            let receipt = source.checkpoint(game).unwrap();
            let row = receipt
                .levels
                .iter()
                .find(|row| row.level == source)
                .unwrap();
            assert_eq!(row.binding, None);
            assert_eq!(row.retired_generation, Some(current.generation()));
            assert_eq!(row.retired_members, Some(3));
        },
    );
}

#[test]
#[ignore = "daily"]
fn ordinary_final_member_removal_has_zero_retirement_receipt_and_keeps_unrelated_real_container() {
    if fresh(
        "game::native_live_worlds_tests::checkpoint_adverse_tests::ordinary_final_member_removal_has_zero_retirement_receipt_and_keeps_unrelated_real_container",
    ) {
        return;
    }
    actual_game(
        vec![ModEntry::new("pose_fixture", Box::new(PublicationFixture))],
        |game, _| {
            let source = prepare_named(game, A);
            let destination = game.level("minecraft:overworld").unwrap();
            let projection = source
                .project(
                    game,
                    destination,
                    &[[17, 64, 0]],
                    LiveWorldPose::at([10., 0., 10.]),
                    LiveWorldLimits::default(),
                )
                .unwrap();
            let physical = sunrise_core::geometry::Vec3::new(27.5, 65.5, 13.);
            game.player =
                Some(sunrise_client::motion::PlayerMotion::new(game.motion, physical).unwrap());
            super::super::native_entity_refs::tests::bind_local_player(game, 0, 123);
            game.lan_acting_name = Some("Fixture".into());
            game.lan_acting_permission = Some(4);
            game.set_mode(sunrise_client::command::args::GameMode::Creative);
            let player = game.native_acting_player().unwrap();
            let eye = game.eye_position().unwrap();
            let hit = projection
                .clip(game, eye, [27.5, 64.9, 10.5])
                .unwrap()
                .unwrap();
            assert_eq!(hit.cell, [17, 64, 0]);
            assert_eq!(projection.destroy_block(game, player, hit, false), Ok(true));
            assert_eq!(source.live_world(game).unwrap(), None);
            assert_eq!(projection.sample(game), Err(LiveWorldError::Stale));
            let world = game.native_level_world(source).unwrap();
            assert!(game.res.behaviours().is_air(world.block(17, 64, 0)));
            let Some(BlockEntity::Chest(items)) = world.block_entities.get(&(0, 64, 0)) else {
                panic!("unrelated actual chest lost")
            };
            assert_eq!(items[0].as_ref().unwrap().count, 7);
            // End the fixture's explicit local acting context before the actual global checkpoint.
            game.lan_acting_entity = None;
            game.lan_acting_uuid = None;
            game.lan_acting_name = None;
            game.lan_acting_permission = None;
            let saved = source.checkpoint(game).unwrap();
            let row = saved.levels.iter().find(|row| row.level == source).unwrap();
            assert_eq!(row.binding, None);
            assert_eq!(row.retired_generation, Some(projection.generation()));
            assert_eq!(row.retired_members, Some(0));
            assert_eq!(game.player.as_ref().unwrap().position, physical);
        },
    );
}

struct SaveCounter(Arc<AtomicUsize>);
#[mod_api::__subscriptions]
impl Hooks for SaveCounter {
    fn chunk_data_save(
        &self,
        _: &dyn mod_api::ChunkDiskHost,
        _: &mut mod_api::ChunkDataSaveEvent<'_>,
    ) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}
fn actual_guest_and_owner(game: &mut Game<'_>) -> u32 {
    // Reuse the existing actual guest/Hub constructor, then establish a distinct real outer owner.
    let at = sunrise_core::geometry::Vec3::new(3., 65., 3.);
    game.player = Some(sunrise_client::motion::PlayerMotion::new(game.motion, at).unwrap());
    game.lan_acting = Some(7);
    game.lan_acting_name = Some("Guest".into());
    super::super::native_entity_refs::tests::bind_local_player(game, 70, 123);
    super::super::native_entity_refs::tests::bind_acting_guest(game);
    game.player = Some(sunrise_client::motion::PlayerMotion::new(game.motion, at).unwrap());
    game.player.as_mut().unwrap().entity_id = 1;
    game.level.player_uuid = Some(sunrise_client::lan::playerdata::uuid_ints(777));
    game.inventory[0] = Some(ItemStack::new("minecraft:stone", 13));
    game.carried = Some(ItemStack::new("minecraft:gold_ingot", 1));
    let guest_index = game.guest_index(7).unwrap();
    game.lan
        .as_mut()
        .unwrap()
        .hub
        .guests
        .update(guest_index, |guest| {
            guest.inventory[0] = Some(ItemStack::new("minecraft:diamond", 5));
            guest.carried = Some(ItemStack::new("minecraft:iron_ingot", 2));
        });
    game.lan_acting = None;
    game.lan_acting_entity = None;
    game.lan_acting_uuid = None;
    game.lan_acting_name = None;
    game.lan_acting_permission = None;
    game.reset_entity_residency();
    game.as_guest(7, |_| ())
        .expect("actual guest turn admitted for normal initialization");
    7
}

#[test]
#[ignore = "daily"]
fn checkpoint_inside_actual_guest_turn_is_busy_before_callbacks_and_preserves_both_players() {
    if fresh(
        "game::native_live_worlds_tests::checkpoint_adverse_tests::checkpoint_inside_actual_guest_turn_is_busy_before_callbacks_and_preserves_both_players",
    ) {
        return;
    }
    let armed = Arc::new(AtomicBool::new(false));
    let joins = Arc::new(AtomicUsize::new(0));
    let saves = Arc::new(AtomicUsize::new(0));
    let hook = JoinTarget {
        armed: armed.clone(),
        joins: joins.clone(),
        target: None,
        results: Arc::new(Mutex::new(Vec::new())),
    };
    actual_game(
        vec![
            ModEntry::new("pose_fixture", Box::new(hook)),
            ModEntry::new("save_observer", Box::new(SaveCounter(saves.clone()))),
        ],
        |game, root| {
            let source = prepare_named(game, A);
            let destination = game.level("minecraft:overworld").unwrap();
            settle_transfer_columns(game, source);
            settle_transfer_columns(game, destination);
            source
                .project(
                    game,
                    destination,
                    MEMBERS,
                    LiveWorldPose::at([10., 16., 10.]),
                    LiveWorldLimits::default(),
                )
                .unwrap();
            let conn = actual_guest_and_owner(game);
            let guest_index = game.guest_index(conn).unwrap();
            queue_actual_saved_marker(game, source);
            armed.store(true, Ordering::SeqCst);
            let host_before = (
                game.inventory.clone(),
                game.carried.clone(),
                game.offhand.clone(),
                game.player.as_ref().unwrap().position,
                game.level.player_uuid,
            );
            let guest_before = sunrise_client::lan::playerdata::player_tag(
                &game.lan.as_ref().unwrap().hub.guests[guest_index],
                game.res.items(),
            );
            let guest_cursor = game.lan.as_ref().unwrap().hub.guests[guest_index]
                .carried
                .clone();
            let files = core_images(root, &[A]);
            let player_files: Vec<_> = [123, 777]
                .into_iter()
                .map(|uuid| {
                    let file = sunrise_client::lan::playerdata::path(root, uuid);
                    let bytes = match std::fs::read(&file) {
                        Ok(bytes) => Some(bytes),
                        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
                        Err(e) => panic!("{}: {e}", file.display()),
                    };
                    (file, bytes)
                })
                .collect();
            let epoch = wal::read_epoch(root).unwrap();
            let callbacks = saves.load(Ordering::SeqCst);
            let cells = [mod_api::LiveWorldTransferCell::new(
                source,
                [0, 64, 0],
                destination,
                [2, 64, 0],
            )];
            let (result, transfer) = game
                .as_guest(conn, |g| {
                    (
                        source.checkpoint(g),
                        source.prepare_transfer(
                            g,
                            &cells,
                            mod_api::LiveWorldTransferLimits::default(),
                        ),
                    )
                })
                .unwrap();
            assert_eq!(result, Err(LiveWorldError::Busy));
            assert_eq!(transfer, Err(LiveWorldError::Busy));
            assert_eq!(joins.load(Ordering::SeqCst), 0);
            assert_eq!(
                saves.load(Ordering::SeqCst),
                callbacks,
                "acting-scope refusal precedes all disk-save callbacks"
            );
            assert_eq!(
                (
                    game.inventory.clone(),
                    game.carried.clone(),
                    game.offhand.clone(),
                    game.player.as_ref().unwrap().position,
                    game.level.player_uuid
                ),
                host_before
            );
            assert_eq!(
                sunrise_client::lan::playerdata::player_tag(
                    &game.lan.as_ref().unwrap().hub.guests[guest_index],
                    game.res.items()
                ),
                guest_before
            );
            assert_eq!(
                game.lan.as_ref().unwrap().hub.guests[guest_index].carried,
                guest_cursor
            );
            assert_eq!(core_images(root, &[A]), files);
            for (file, before) in &player_files {
                let after = match std::fs::read(file) {
                    Ok(bytes) => Some(bytes),
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
                    Err(e) => panic!("{}: {e}", file.display()),
                };
                assert_eq!(
                    &after, before,
                    "Busy must not replace either real player's file"
                );
            }
            assert_eq!(wal::read_epoch(root).unwrap(), epoch);
            assert!(!root.join("data/sunrise_live_world_journal.bin").exists());
            // Actual cursor return through the ordinary close route precedes a durable player snapshot.
            game.close_container();
            game.as_guest(conn, |g| g.close_container()).unwrap();
            let receipt = source.checkpoint(game).unwrap();
            assert!(receipt.levels.iter().any(|row| row.level == source));
            assert_eq!(joins.load(Ordering::SeqCst), 1);
            assert!(saves.load(Ordering::SeqCst) > callbacks);
            assert_eq!(wal::read_epoch(root).unwrap(), receipt.barrier_epoch);
            // This row deliberately registers a real custom chunk serializer to observe Busy's
            // pre-effect ordering. The initial stock-transfer slice refuses that codec policy;
            // the separate public transfer row proves admission with ordinary stock serializers.
            assert_eq!(
                source.prepare_transfer(game, &cells, mod_api::LiveWorldTransferLimits::default()),
                Err(LiveWorldError::Unsupported)
            );
        },
    );
}

/// Fixture admission only: build the same real lit neighborhood used by the ordinary first-tick
/// tests, then run the actual registration/ticket/unpack phases without advancing a furnace tick.
fn settle_transfer_columns(game: &mut Game<'_>, level: mod_api::LevelRef) {
    game.enter_level(Dim::from_id(level.dimension_id()))
        .unwrap();
    for x in -1..=2 {
        for z in -1..=1 {
            if !game.world.chunks.contains_key(&(x, z)) {
                game.world
                    .chunks
                    .insert((x, z), Arc::new(Chunk::empty(x, z)));
            }
        }
    }
    for x in [0, 1] {
        Arc::make_mut(game.world.chunks.get_mut(&(x, 0)).unwrap()).lit = true;
        level
            .chunk_ref(game, [x, 0])
            .unwrap()
            .set_forced(game, true);
    }
    // These are the real ordinary Game phases; no synthetic Ticks, handoff plan or permission.
    game.sync_tick_chunks();
    game.update_chunk_tickets();
    game.start_ticking_chunks();
    for p in MEMBERS {
        assert_eq!(level.should_tick_blocks_at(game, *p), Some(true));
        let pos = sunrise_core::position::ChunkPos::new(p[0] >> 4, p[2] >> 4);
        assert!(game.block_ticks.blocks.chunk(pos).is_some());
        assert!(game.block_ticks.fluids.chunk(pos).is_some());
    }
}
fn actual_cell_images(
    game: &Game<'_>,
    level: mod_api::LevelRef,
    cells: &[[i32; 3]],
) -> Vec<String> {
    let world = game.native_level_world(level).unwrap();
    cells
        .iter()
        .map(|p| {
            format!(
                "{:?}:{:?}",
                world.block(p[0], p[1], p[2]),
                world.block_entities.get(&(p[0], p[1], p[2]))
            )
        })
        .collect()
}
fn assert_actual_fixture_contents(game: &Game<'_>, level: mod_api::LevelRef, cells: &[[i32; 3]]) {
    let world = game.native_level_world(level).unwrap();
    let chest = cells[0];
    let furnace = cells[1];
    let stone = cells[2];
    let Some(BlockEntity::Chest(items)) = world.block_entities.get(&(chest[0], chest[1], chest[2]))
    else {
        panic!("actual chest owner missing")
    };
    assert_eq!(items[0].as_ref().unwrap().count, 7);
    assert_eq!(items[0], Some(ItemStack::new("minecraft:stone", 7)));
    assert!(items[1..].iter().all(Option::is_none));
    let Some(BlockEntity::Furnace(value)) = world
        .block_entities
        .get(&(furnace[0], furnace[1], furnace[2]))
    else {
        panic!("actual furnace owner missing")
    };
    assert_eq!(value.cooking_timer, 40);
    assert_eq!(value.lit_remaining, 5);
    assert_eq!(value.items[0].as_ref().unwrap().count, 1);
    assert_eq!(value.items[1].as_ref().unwrap().count, 1);
    assert!(value.items[2].is_none());
    assert_eq!(
        world.block(stone[0], stone[1], stone[2]),
        game.pack
            .assets
            .registry
            .default_state("minecraft:stone")
            .unwrap()
    );
}
fn assert_actual_empty(game: &Game<'_>, level: mod_api::LevelRef, cells: &[[i32; 3]]) {
    let world = game.native_level_world(level).unwrap();
    for p in cells {
        assert!(game.res.behaviours().is_air(world.block(p[0], p[1], p[2])));
        assert!(world.block_entities.get(&(p[0], p[1], p[2])).is_none());
        assert!(!world.block_entities.is_taken(&(p[0], p[1], p[2])));
    }
}
fn assert_transfer_receipt_revisions(
    game: &Game<'_>,
    receipt: &mod_api::LiveWorldTransferReceipt,
    levels: &[mod_api::LevelRef],
) {
    for level in levels {
        let live = level.revisions(game).unwrap();
        let saved = receipt
            .levels
            .iter()
            .find(|row| row.level == *level)
            .unwrap();
        assert_eq!(saved.block_revision, live.block_revision);
        assert_eq!(saved.content_revision, live.content_revision);
    }
}

#[test]
#[ignore = "daily"]
fn public_transfer_refuses_second_destination_aborts_root_then_moves_and_returns_real_owners() {
    if fresh(
        "game::native_live_worlds_tests::checkpoint_adverse_tests::public_transfer_refuses_second_destination_aborts_root_then_moves_and_returns_real_owners",
    ) {
        return;
    }
    actual_game(
        vec![ModEntry::new("pose_fixture", Box::new(PublicationFixture))],
        |game, root| {
            let source = prepare_named(game, A);
            let c = empty_owned(game);
            let parent = game.level("minecraft:overworld").unwrap();
            settle_transfer_columns(game, source);
            settle_transfer_columns(game, parent);
            let projection = source
                .project(
                    game,
                    parent,
                    MEMBERS,
                    LiveWorldPose::at([10., 16., 10.]),
                    LiveWorldLimits::default(),
                )
                .unwrap();
            let targets = [[2, 64, 0], [3, 64, 0], [17, 64, 0]];
            let cells: Vec<_> = MEMBERS
                .iter()
                .zip(targets)
                .map(|(from, to)| mod_api::LiveWorldTransferCell::new(source, *from, parent, to))
                .collect();
            assert_actual_fixture_contents(game, source, MEMBERS);
            assert_actual_empty(game, parent, &targets);
            let original = actual_cell_images(game, source, MEMBERS);
            let sample = projection.sample(game).unwrap();
            let pose_admission = game.capture_projection_pose(projection).unwrap();
            // An occupied SECOND target must refuse all mappings, including the empty first one.
            let stone = game
                .pack
                .assets
                .registry
                .default_state("minecraft:stone")
                .unwrap();
            game.world.set(3, 64, 0, stone).unwrap();
            let occupied = actual_cell_images(game, parent, &targets);
            let revisions = (
                source.revisions(game).unwrap(),
                parent.revisions(game).unwrap(),
            );
            let epoch = wal::read_epoch(root).unwrap();
            let files = core_images(root, &[A, C, "minecraft:overworld"]);
            assert_eq!(
                source.prepare_transfer(game, &cells, mod_api::LiveWorldTransferLimits::default()),
                Err(LiveWorldError::Obstructed)
            );
            assert_eq!(actual_cell_images(game, source, MEMBERS), original);
            assert_eq!(actual_cell_images(game, parent, &targets), occupied);
            assert_eq!(
                (
                    source.revisions(game).unwrap(),
                    parent.revisions(game).unwrap()
                ),
                revisions
            );
            assert_eq!(
                PUBLICATION
                    .get_existing(&mut c.attachments(game), Holder::Level)
                    .unwrap(),
                None
            );
            assert_eq!(
                pose_bits(projection.sample(game).unwrap()),
                pose_bits(sample)
            );
            assert_eq!(
                game.capture_projection_pose(projection).unwrap(),
                pose_admission
            );
            assert_eq!(wal::read_epoch(root).unwrap(), epoch);
            assert_eq!(core_images(root, &[A, C, "minecraft:overworld"]), files);
            game.world.set(3, 64, 0, game.world.air).unwrap();

            let epoch = wal::read_epoch(root).unwrap();
            let files = core_images(root, &[A, C, "minecraft:overworld"]);
            let revisions = (
                source.revisions(game).unwrap(),
                parent.revisions(game).unwrap(),
            );
            let token = source
                .prepare_transfer(game, &cells, mod_api::LiveWorldTransferLimits::default())
                .unwrap();
            assert_eq!(token.receipt(game), Err(LiveWorldError::NotCommitted));
            let aborted = (game as &mut dyn ServerHost).transaction(|host, tx| {
                let intention = token.stage(host, tx).unwrap();
                assert!(intention.planned_epoch > epoch);
                assert_eq!(token.receipt(host), Err(LiveWorldError::Busy));
                PUBLICATION.set(&mut c.attachments(host), Holder::Level, metadata(81))?;
                BAD_PUBLICATION.set(&mut c.attachments(host), Holder::Level, metadata(99))
            });
            assert_eq!(aborted, Err(AttachmentError::Encode));
            assert_eq!(token.receipt(game), Err(LiveWorldError::NotCommitted));
            assert_eq!(actual_cell_images(game, source, MEMBERS), original);
            assert_actual_empty(game, parent, &targets);
            assert_eq!(
                (
                    source.revisions(game).unwrap(),
                    parent.revisions(game).unwrap()
                ),
                revisions
            );
            assert_eq!(
                PUBLICATION
                    .get_existing(&mut c.attachments(game), Holder::Level)
                    .unwrap(),
                None
            );
            assert_eq!(source.live_world(game).unwrap(), Some(projection));
            assert_eq!(
                pose_bits(projection.sample(game).unwrap()),
                pose_bits(sample)
            );
            assert_eq!(
                game.capture_projection_pose(projection).unwrap(),
                pose_admission
            );
            assert_eq!(wal::read_epoch(root).unwrap(), epoch);
            assert_eq!(core_images(root, &[A, C, "minecraft:overworld"]), files);
            token.cancel(game).unwrap();

            let token = source
                .prepare_transfer(game, &cells, mod_api::LiveWorldTransferLimits::default())
                .unwrap();
            let intention = (game as &mut dyn ServerHost)
                .transaction(|host, tx| {
                    let pending = token.stage(host, tx).unwrap();
                    assert_eq!(token.receipt(host), Err(LiveWorldError::Busy));
                    PUBLICATION.set(&mut c.attachments(host), Holder::Level, metadata(82))?;
                    Ok::<_, AttachmentError>(pending)
                })
                .unwrap();
            let receipt = token.receipt(game).unwrap();
            assert_eq!(receipt.barrier_epoch, intention.planned_epoch);
            assert_eq!(wal::read_epoch(root).unwrap(), receipt.barrier_epoch);
            assert_eq!(receipt.moved_cells, 3);
            assert!(receipt.retired_bindings.contains(&projection));
            assert_transfer_receipt_revisions(game, &receipt, &[source, parent, c]);
            let row = receipt
                .levels
                .iter()
                .find(|row| row.level == source)
                .unwrap();
            assert_eq!(row.binding, None);
            assert_eq!(row.retired_generation, Some(projection.generation()));
            assert_eq!(row.retired_members, Some(0));
            assert_eq!(source.live_world(game).unwrap(), None);
            assert_eq!(projection.sample(game), Err(LiveWorldError::Stale));
            assert_actual_empty(game, source, MEMBERS);
            assert_actual_fixture_contents(game, parent, &targets);
            assert_eq!(actual_cell_images(game, parent, &targets), original);
            assert_saved_empty_metadata(game, root, c);
            let after = (
                source.revisions(game).unwrap(),
                parent.revisions(game).unwrap(),
            );
            assert!(after.0.block_revision > revisions.0.block_revision);
            assert!(after.0.content_revision > revisions.0.content_revision);
            assert!(after.1.block_revision > revisions.1.block_revision);
            assert!(after.1.content_revision > revisions.1.content_revision);

            let reverse: Vec<_> = targets
                .into_iter()
                .zip(MEMBERS)
                .map(|(from, to)| mod_api::LiveWorldTransferCell::new(parent, from, source, *to))
                .collect();
            let back = source
                .prepare_transfer(game, &reverse, mod_api::LiveWorldTransferLimits::default())
                .unwrap();
            assert_eq!(back.receipt(game), Err(LiveWorldError::NotCommitted));
            let returned = back.commit(game).unwrap();
            assert_eq!(returned.moved_cells, 3);
            assert!(returned.barrier_epoch > receipt.barrier_epoch);
            assert_eq!(wal::read_epoch(root).unwrap(), returned.barrier_epoch);
            assert_transfer_receipt_revisions(game, &returned, &[source, parent]);
            assert_actual_fixture_contents(game, source, MEMBERS);
            assert_eq!(actual_cell_images(game, source, MEMBERS), original);
            assert_actual_empty(game, parent, &targets);
            assert_eq!(source.live_world(game).unwrap(), None);
            token.cancel(game).unwrap();
            back.cancel(game).unwrap();

            // A dry, state-invariant half slab follows the same real transfer lifecycle.
            game.enter_level(Dim::from_id(source.dimension_id()))
                .unwrap();
            let slab = game
                .pack
                .assets
                .registry
                .default_state("minecraft:stone_slab")
                .unwrap();
            game.world.set(18, 64, 0, slab).unwrap();
            game.enter_level(Dim::Overworld).unwrap();
            let thin = source
                .project(
                    game,
                    parent,
                    &[[18, 64, 0]],
                    LiveWorldPose::at([10., 16., 10.]),
                    LiveWorldLimits::default(),
                )
                .unwrap();
            let actual = thin.cells(game).unwrap();
            assert_eq!(actual.len(), 1);
            assert!(
                actual[0]
                    .collision
                    .iter()
                    .all(|b| b.max[1] - b.min[1] < 1.0)
            );
            let moving = source
                .prepare_transfer(
                    game,
                    &[mod_api::LiveWorldTransferCell::new(
                        source,
                        [18, 64, 0],
                        parent,
                        [18, 64, 0],
                    )],
                    mod_api::LiveWorldTransferLimits::default(),
                )
                .unwrap();
            assert_eq!(moving.commit(game).unwrap().moved_cells, 1);
            assert_eq!(thin.sample(game), Err(LiveWorldError::Stale));
            assert_eq!(
                game.native_level_world(parent).unwrap().block(18, 64, 0),
                slab
            );
            let returning = source
                .prepare_transfer(
                    game,
                    &[mod_api::LiveWorldTransferCell::new(
                        parent,
                        [18, 64, 0],
                        source,
                        [18, 64, 0],
                    )],
                    mod_api::LiveWorldTransferLimits::default(),
                )
                .unwrap();
            returning.commit(game).unwrap();
            assert_eq!(
                game.native_level_world(source).unwrap().block(18, 64, 0),
                slab
            );
            moving.cancel(game).unwrap();
            returning.cancel(game).unwrap();

            // This fixture seeded an already-burning BE. Give its final hot-transfer scene the
            // corresponding real stock LIT state; serverTick changes LIT only on burning transitions.
            game.enter_level(Dim::from_id(source.dimension_id()))
                .unwrap();
            let state = game.world.block(1, 64, 0);
            let mut canonical = game.pack.assets.registry.state(state).unwrap().clone();
            canonical.properties.insert("lit".into(), "true".into());
            let burning = game.pack.assets.registry.resolve(&canonical).unwrap();
            game.world.set(1, 64, 0, burning).unwrap();
            game.enter_level(Dim::Overworld).unwrap();
            // The actual scheduler advances once; known stock emission remains supported.
            game.tick_server(false).unwrap();
            let Some(BlockEntity::Furnace(before)) = game
                .native_level_world(source)
                .unwrap()
                .block_entities
                .get(&(1, 64, 0))
            else {
                panic!("source furnace absent");
            };
            let progress = before.cooking_timer;
            assert_eq!(progress, 41);
            let lit_state = game.native_level_world(source).unwrap().block(1, 64, 0);
            assert_eq!(
                game.pack
                    .assets
                    .registry
                    .state(lit_state)
                    .unwrap()
                    .properties
                    .get("lit")
                    .map(String::as_str),
                Some("true")
            );
            let hot = source
                .prepare_transfer(
                    game,
                    &[mod_api::LiveWorldTransferCell::new(
                        source,
                        [1, 64, 0],
                        parent,
                        [3, 64, 0],
                    )],
                    mod_api::LiveWorldTransferLimits::default(),
                )
                .unwrap();
            hot.commit(game).unwrap();
            let Some(BlockEntity::Furnace(after)) = game
                .native_level_world(parent)
                .unwrap()
                .block_entities
                .get(&(3, 64, 0))
            else {
                panic!("live lit furnace absent at destination");
            };
            assert_eq!(after.cooking_timer, progress);
            game.tick_server(false).unwrap();
            let Some(BlockEntity::Furnace(after)) = game
                .native_level_world(parent)
                .unwrap()
                .block_entities
                .get(&(3, 64, 0))
            else {
                unreachable!();
            };
            assert_eq!(
                after.cooking_timer,
                progress + 1,
                "moved ticker has one actual owner on the following global tick"
            );
            assert!(
                game.native_level_world(source)
                    .unwrap()
                    .block_entities
                    .get(&(1, 64, 0))
                    .is_none()
            );
            hot.cancel(game).unwrap();
            // Two distinct payload destinations join one decision; C remains a third metadata owner.
            let dormant = (game as &mut dyn ServerHost)
                .runtime_levels()
                .open(
                    B,
                    RuntimeLevel::new("minecraft:overworld", RuntimeGenerator::Void, 1022),
                )
                .unwrap();
            game.apply_runtime_levels();
            settle_transfer_columns(game, dormant);
            game.enter_level(Dim::Overworld).unwrap();
            let split = vec![
                mod_api::LiveWorldTransferCell::new(source, [17, 64, 0], parent, [17, 64, 0]),
                mod_api::LiveWorldTransferCell::new(source, [0, 64, 0], dormant, [0, 64, 0]),
            ];
            assert_actual_empty(game, parent, &[[17, 64, 0]]);
            let before_source = actual_cell_images(game, source, &[[17, 64, 0], [0, 64, 0]]);
            game.enter_level(Dim::from_id(B)).unwrap();
            game.world.set(0, 64, 0, stone).unwrap();
            game.enter_level(Dim::Overworld).unwrap();
            let before_dest = actual_cell_images(game, dormant, &[[0, 64, 0]]);
            let before_epoch = wal::read_epoch(root).unwrap();
            let before_files = core_images(root, &[A, B, C, "minecraft:overworld"]);
            let before_revisions =
                [source, parent, dormant].map(|level| level.revisions(game).unwrap());
            assert_eq!(
                source.prepare_transfer(game, &split, mod_api::LiveWorldTransferLimits::default()),
                Err(LiveWorldError::Obstructed)
            );
            assert_eq!(
                actual_cell_images(game, source, &[[17, 64, 0], [0, 64, 0]]),
                before_source
            );
            assert_actual_empty(game, parent, &[[17, 64, 0]]);
            assert_eq!(
                actual_cell_images(game, dormant, &[[0, 64, 0]]),
                before_dest
            );
            assert_eq!(
                [source, parent, dormant].map(|level| level.revisions(game).unwrap()),
                before_revisions
            );
            assert_eq!(wal::read_epoch(root).unwrap(), before_epoch);
            assert_eq!(
                core_images(root, &[A, B, C, "minecraft:overworld"]),
                before_files
            );
            game.enter_level(Dim::from_id(B)).unwrap();
            game.world.set(0, 64, 0, game.world.air).unwrap();
            game.enter_level(Dim::Overworld).unwrap();
            let stale_destination = source
                .prepare_transfer(game, &split, mod_api::LiveWorldTransferLimits::default())
                .unwrap();
            let before_epoch = wal::read_epoch(root).unwrap();
            let before_files = core_images(root, &[A, B, C, "minecraft:overworld"]);
            let before_revisions =
                [source, parent, dormant].map(|level| level.revisions(game).unwrap());
            (game as &mut dyn ServerHost)
                .runtime_levels()
                .unload(B)
                .unwrap();
            let refused = (game as &mut dyn ServerHost)
                .transaction(|host, tx| stale_destination.stage(host, tx).map(|_| ()));
            assert_eq!(refused, Err(LiveWorldError::Stale));
            assert_eq!(
                stale_destination.receipt(game),
                Err(LiveWorldError::NotCommitted)
            );
            assert_eq!(
                actual_cell_images(game, source, &[[17, 64, 0], [0, 64, 0]]),
                before_source
            );
            assert_actual_empty(game, parent, &[[17, 64, 0]]);
            assert_actual_empty(game, dormant, &[[0, 64, 0]]);
            assert_eq!(
                [source, parent].map(|level| level.revisions(game).unwrap()),
                [before_revisions[0], before_revisions[1]]
            );
            assert_eq!(dormant.revisions(game), Err(mod_api::LevelReadError::Stale));
            let actual = game.native_level_world(dormant).unwrap();
            assert_eq!(
                (actual.revision, actual.block_entities.content_revision()),
                (
                    before_revisions[2].block_revision,
                    before_revisions[2].content_revision
                )
            );
            assert_eq!(wal::read_epoch(root).unwrap(), before_epoch);
            assert_eq!(
                core_images(root, &[A, B, C, "minecraft:overworld"]),
                before_files
            );
            stale_destination.cancel(game).unwrap();
            game.apply_runtime_levels();
            assert_eq!(dormant.revisions(game), Err(mod_api::LevelReadError::Stale));
            let destination_b = (game as &mut dyn ServerHost)
                .runtime_levels()
                .open(
                    B,
                    RuntimeLevel::new("minecraft:overworld", RuntimeGenerator::Void, 1022),
                )
                .unwrap();
            game.apply_runtime_levels();
            assert_ne!(destination_b, dormant);
            settle_transfer_columns(game, destination_b);
            game.enter_level(Dim::Overworld).unwrap();
            let split = [
                mod_api::LiveWorldTransferCell::new(source, [17, 64, 0], parent, [17, 64, 0]),
                mod_api::LiveWorldTransferCell::new(source, [0, 64, 0], destination_b, [0, 64, 0]),
            ];
            let ready = source
                .prepare_transfer(game, &split, mod_api::LiveWorldTransferLimits::default())
                .unwrap();
            let intention = (game as &mut dyn ServerHost)
                .transaction(|host, tx| {
                    let pending = ready.stage(host, tx).unwrap();
                    PUBLICATION.set(&mut c.attachments(host), Holder::Level, metadata(84))?;
                    Ok::<_, AttachmentError>(pending)
                })
                .unwrap();
            let completed = ready.receipt(game).unwrap();
            assert_eq!(completed.barrier_epoch, intention.planned_epoch);
            assert_eq!(completed.moved_cells, 2);
            assert_transfer_receipt_revisions(
                game,
                &completed,
                &[source, parent, destination_b, c],
            );
            assert_eq!(wal::read_epoch(root).unwrap(), completed.barrier_epoch);
            assert_actual_empty(game, source, &[[17, 64, 0], [0, 64, 0]]);
            assert_eq!(
                actual_cell_images(game, parent, &[[17, 64, 0]]),
                vec![original[2].clone()]
            );
            assert_eq!(
                actual_cell_images(game, destination_b, &[[0, 64, 0]]),
                vec![original[0].clone()]
            );
            assert_saved_empty_metadata(game, root, c);
            assert_eq!(
                PUBLICATION
                    .get_existing(&mut c.attachments(game), Holder::Level)
                    .unwrap(),
                Some(metadata(84))
            );
            let reverse = [
                mod_api::LiveWorldTransferCell::new(parent, [17, 64, 0], source, [17, 64, 0]),
                mod_api::LiveWorldTransferCell::new(destination_b, [0, 64, 0], source, [0, 64, 0]),
            ];
            let returning = source
                .prepare_transfer(game, &reverse, mod_api::LiveWorldTransferLimits::default())
                .unwrap();
            (game as &mut dyn ServerHost)
                .transaction(|host, tx| {
                    returning.stage(host, tx).unwrap();
                    PUBLICATION.set(&mut c.attachments(host), Holder::Level, metadata(85))
                })
                .unwrap();
            let returned = returning.receipt(game).unwrap();
            assert_eq!(returned.moved_cells, 2);
            assert!(returned.barrier_epoch > completed.barrier_epoch);
            assert_transfer_receipt_revisions(game, &returned, &[source, parent, destination_b, c]);
            assert_eq!(wal::read_epoch(root).unwrap(), returned.barrier_epoch);
            assert_eq!(
                actual_cell_images(game, source, &[[17, 64, 0], [0, 64, 0]]),
                before_source
            );
            assert_actual_empty(game, parent, &[[17, 64, 0]]);
            assert_actual_empty(game, destination_b, &[[0, 64, 0]]);
            assert_eq!(
                PUBLICATION
                    .get_existing(&mut c.attachments(game), Holder::Level)
                    .unwrap(),
                Some(metadata(85))
            );
            assert_saved_empty_metadata(game, root, c);
            let Some(BlockEntity::Furnace(final_counter)) = game
                .native_level_world(parent)
                .unwrap()
                .block_entities
                .get(&(3, 64, 0))
            else {
                panic!("one live furnace owner remains in Parent");
            };
            assert_eq!(
                final_counter.cooking_timer,
                progress + 1,
                "bulk metadata/lifecycle/save operations do not run another BE tick"
            );
            ready.cancel(game).unwrap();
            returning.cancel(game).unwrap();
        },
    );
}

#[path = "checkpoint_adverse_tests/shutdown_tests.rs"]
mod shutdown_tests;

#[path = "checkpoint_adverse_tests/retired_recapture_tests.rs"]
mod retired_recapture_tests;

#[path = "checkpoint_adverse_tests/budget_tests.rs"]
mod budget_tests;
