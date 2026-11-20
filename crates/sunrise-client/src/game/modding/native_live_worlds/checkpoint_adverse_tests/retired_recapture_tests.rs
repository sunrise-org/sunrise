//! Completed Return, normal whole-Game stop, and Capture in another process over the same save.
//! Only the first child seeds this synthetic scene; the second uses ordinary world/Streaming readers.
use super::*;
use std::collections::BTreeSet;
use sunrise_core::{position::BlockPos, ticks::SavedTick};
use sunrise_world::level::LevelAccess as _;

const NAME: &str = "game::native_live_worlds_tests::checkpoint_adverse_tests::retired_recapture_tests::completed_return_reopens_retired_source_and_recaptures_into_new_columns";
const PHASE: &str = "SUNRISE_RETIRED_RECAPTURE_PHASE";
const ROOT: &str = "SUNRISE_RETIRED_RECAPTURE_ROOT";
const TARGETS: &[[i32; 3]] = &[[2, 64, 0], [3, 64, 0], [17, 64, 0]];
const INBOUND: &[[i32; 3]] = &[[32, 72, 0], [33, 72, 0], [49, 72, 0]];
// Source B's disjoint published [10,16,-10] transform also needs actual Parent columns.
const OTHER_PARENT: &[[i32; 3]] = &[[10, 80, -10], [11, 80, -10], [27, 80, -10]];

fn schedule_rows(game: &Game<'_>, level: mod_api::LevelRef, cells: &[[i32; 3]]) -> Vec<SavedTick> {
    let (world, ticks) = if level.dimension() == game.dim.key() {
        (&game.world, &game.block_ticks)
    } else {
        game.levels
            .parked_transfer_parts(level.dimension_id())
            .unwrap()
    };
    let mut rows = Vec::new();
    let columns: BTreeSet<_> = cells.iter().map(|p| (p[0] >> 4, p[2] >> 4)).collect();
    for (x, z) in columns {
        let chunk = ticks
            .blocks
            .chunk(sunrise_core::position::ChunkPos::new(x, z))
            .unwrap();
        rows.extend(
            chunk
                .pack(world.game_time)
                .into_iter()
                .filter(|t| cells.contains(&[t.pos.x, t.pos.y, t.pos.z])),
        );
    }
    rows
}

fn trace_cut(game: &Game<'_>, phase: &str, levels: &[mod_api::LevelRef]) {
    eprintln!(
        "LIVE_WORLD_RECAPTURE phase={phase} frontier={} transaction={:?} fenced={} epoch={:?} pending_journal={}",
        game.live_world_commit_frontier(),
        game.transaction_lifecycle(),
        game.live_world_session_failed(),
        wal::read_epoch(&game.world_root),
        game.world_root
            .join("data/sunrise_live_world_journal.bin")
            .exists()
    );
    for &level in levels {
        let (world, ticks) = if level.dimension() == game.dim.key() {
            (&game.world, &game.block_ticks)
        } else {
            game.levels
                .parked_transfer_parts(level.dimension_id())
                .unwrap()
        };
        let mut columns: Vec<_> = world
            .chunks
            .iter()
            .map(|(key, chunk)| {
                (
                    *key,
                    Arc::as_ptr(chunk) as usize,
                    chunk.saved.as_ref().map(|s| s.last_update),
                )
            })
            .collect();
        columns.sort_unstable();
        eprintln!(
            "LIVE_WORLD_RECAPTURE level={} revisions={:?} time={} order={} scheduler={:?} columns={columns:?}",
            level.dimension_id(),
            level.revisions(game),
            world.game_time,
            ticks.transfer_order_cursor(),
            ticks.transfer_busy_bits(&BTreeSet::new())
        );
    }
}

/// Refusal must conserve the entire selected live payload, scalar, schedules and durable files.
/// The trace is bounded to this synthetic fixture; it contains no other caller's opaque data.
fn move_with_scalar(
    game: &mut Game<'_>,
    root: &Path,
    source: mod_api::LevelRef,
    parent: mod_api::LevelRef,
    (from, from_cells): (mod_api::LevelRef, &[[i32; 3]]),
    (to, to_cells): (mod_api::LevelRef, &[[i32; 3]]),
    scalar: i64,
) -> mod_api::LiveWorldTransferReceipt {
    let cells: Vec<_> = from_cells
        .iter()
        .zip(to_cells)
        .map(|(from_pos, to_pos)| mod_api::LiveWorldTransferCell::new(from, *from_pos, to, *to_pos))
        .collect();
    let before = (
        actual_cell_images(game, from, from_cells),
        actual_cell_images(game, to, to_cells),
    );
    let schedules = (
        schedule_rows(game, from, from_cells),
        schedule_rows(game, to, to_cells),
    );
    let revisions = (from.revisions(game).unwrap(), to.revisions(game).unwrap());
    let marker = PUBLICATION
        .get_existing(&mut source.attachments(game), Holder::Level)
        .unwrap();
    let epoch = wal::read_epoch(root).unwrap();
    let files = core_images(root, &[A, B, "minecraft:overworld"]);
    trace_cut(game, "before public preparation", &[source, parent]);
    let token = source.prepare_transfer(
        game,
        &cells,
        mod_api::LiveWorldTransferLimits {
            bytes: 64 << 20,
            ..Default::default()
        },
    );
    eprintln!("LIVE_WORLD_RECAPTURE actual_prepare={token:?}");
    let result = token.and_then(|token| {
        let staged = (game as &mut dyn ServerHost).transaction(|host, tx| {
            let intention = token.stage(host, tx)?;
            PUBLICATION
                .set(
                    &mut source.attachments(host),
                    Holder::Level,
                    metadata(scalar),
                )
                .unwrap();
            Ok::<_, LiveWorldError>(intention)
        });
        eprintln!("LIVE_WORLD_RECAPTURE actual_stage_closure={staged:?}");
        staged.and_then(|_| token.receipt(game))
    });
    if let Err(error) = &result {
        trace_cut(
            game,
            &format!("actual receipt {error:?}"),
            &[source, parent],
        );
        assert!(
            !game.live_world_session_failed(),
            "pre-decision refusal must not fence"
        );
        assert_eq!(
            (
                actual_cell_images(game, from, from_cells),
                actual_cell_images(game, to, to_cells)
            ),
            before
        );
        assert_eq!(
            (
                schedule_rows(game, from, from_cells),
                schedule_rows(game, to, to_cells)
            ),
            schedules
        );
        assert_eq!(
            (from.revisions(game).unwrap(), to.revisions(game).unwrap()),
            revisions
        );
        assert_eq!(
            PUBLICATION
                .get_existing(&mut source.attachments(game), Holder::Level)
                .unwrap(),
            marker
        );
        assert_eq!(wal::read_epoch(root).unwrap(), epoch);
        assert_eq!(core_images(root, &[A, B, "minecraft:overworld"]), files);
        assert!(!root.join("data/sunrise_live_world_journal.bin").exists());
    }
    let receipt = result.expect("actual prepared root must transfer the stable selected payload");
    assert_eq!(receipt.moved_cells, 3);
    assert!(receipt.barrier_epoch > epoch);
    assert_transfer_receipt_revisions(game, &receipt, &[source, parent]);
    assert_eq!(
        PUBLICATION
            .get_existing(&mut source.attachments(game), Holder::Level)
            .unwrap(),
        Some(metadata(scalar))
    );
    assert_actual_empty(game, from, from_cells);
    assert_actual_fixture_contents(game, to, to_cells);
    assert_eq!(actual_cell_images(game, to, to_cells), before.0);
    assert!(schedule_rows(game, from, from_cells).is_empty());
    let mut expected = schedules.0;
    for tick in &mut expected {
        let i = from_cells
            .iter()
            .position(|p| *p == [tick.pos.x, tick.pos.y, tick.pos.z])
            .unwrap();
        let p = to_cells[i];
        tick.pos = BlockPos::new(p[0], p[1], p[2]);
    }
    assert_eq!(schedule_rows(game, to, to_cells), expected);
    receipt
}

/// Real disk/generation/ticket readiness after opening, without inventing replacement loaded chunks.
fn load_columns(game: &mut Game<'_>, level: mod_api::LevelRef, cells: &[[i32; 3]]) {
    game.enter_level(Dim::from_id(level.dimension_id()))
        .unwrap();
    game.streaming.paused = false;
    let columns: BTreeSet<_> = cells.iter().map(|p| (p[0] >> 4, p[2] >> 4)).collect();
    for &column in &columns {
        level
            .chunk_ref(game, [column.0, column.1])
            .unwrap()
            .set_forced(game, true);
    }
    for _ in 0..2000 {
        game.update_world();
        game.tick_world(false).unwrap();
        if cells
            .iter()
            .all(|p| level.should_tick_blocks_at(game, *p) == Some(true))
            && columns
                .iter()
                .all(|&(x, z)| crate::game::ticking_chunk(&game.world, x, z))
        {
            // The ordinary first-ticking phase owns saved-list unpack. A paused world tick
            // still registers chunks but does not run tick_blocks/start_ticking_chunks.
            game.start_ticking_chunks();
            return;
        }
        game.streaming.wait(std::time::Duration::from_millis(5));
    }
    trace_cut(game, "ordinary saved-column readiness timeout", &[level]);
    panic!("ordinary Streaming did not make the declared fixture columns ready");
}

fn assert_saved_tick(save: &SaveDir, at: [i32; 3]) {
    let tag = save.raw_chunk(at[0] >> 4, at[2] >> 4).unwrap().tag;
    let Some(sunrise_core::nbt::Tag::List(ticks)) = tag.get("block_ticks") else {
        panic!("actual saved block tick list missing");
    };
    assert_eq!(ticks.len(), 1);
    let tick = &ticks[0];
    assert_eq!(
        tick.get("i"),
        Some(&sunrise_core::nbt::Tag::String("minecraft:stone".into()))
    );
    for (name, value) in [
        ("x", at[0]),
        ("y", at[1]),
        ("z", at[2]),
        ("t", 2000),
        ("p", -1),
    ] {
        assert_eq!(tick.get(name), Some(&sunrise_core::nbt::Tag::Int(value)));
    }
}

/// First-child initialization on genuinely loaded Void chunks, using the same public writes as
/// the canonical example. No helper may replace Parent chunks while Streaming owns them.
fn seed_live_source(game: &mut Game<'_>, source: mod_api::LevelRef) {
    game.enter_level(Dim::from_id(source.dimension_id()))
        .unwrap();
    assert!(game.save.tables.get().is_some());
    for (p, name) in MEMBERS
        .iter()
        .zip(["minecraft:chest", "minecraft:furnace", "minecraft:stone"])
    {
        let state = (game as &dyn ServerHost).block_state_id(name).unwrap();
        assert!(
            source
                .set_block(
                    game,
                    *p,
                    state,
                    mod_api::BlockUpdateFlags::NEIGHBORS | mod_api::BlockUpdateFlags::CLIENTS
                )
                .unwrap()
        );
    }
    let item = |slot: i8, name: &str| {
        mod_api::Tag::compound([
            ("Slot", mod_api::Tag::Byte(slot)),
            ("id", mod_api::Tag::String(name.into())),
            ("count", mod_api::Tag::Int(1)),
        ])
    };
    let mut chest_item = item(0, "minecraft:stone");
    let mod_api::Tag::Compound(ref mut fields) = chest_item else {
        unreachable!()
    };
    fields.insert("count".into(), mod_api::Tag::Int(7));
    let chest = (game as &dyn ServerHost)
        .block_entity(source, MEMBERS[0])
        .unwrap();
    assert!(
        chest
            .merge_data(
                game,
                [("Items".into(), mod_api::Tag::List(vec![chest_item]))].into()
            )
            .unwrap()
    );
    let furnace = (game as &dyn ServerHost)
        .block_entity(source, MEMBERS[1])
        .unwrap();
    assert!(
        furnace
            .merge_data(
                game,
                [
                    (
                        "Items".into(),
                        mod_api::Tag::List(vec![
                            item(0, "minecraft:raw_iron"),
                            item(1, "minecraft:coal")
                        ])
                    ),
                    ("lit_time_remaining".into(), mod_api::Tag::Int(5)),
                    ("lit_total_time".into(), mod_api::Tag::Int(200)),
                    ("cooking_time_spent".into(), mod_api::Tag::Int(40)),
                    ("cooking_total_time".into(), mod_api::Tag::Int(200)),
                ]
                .into()
            )
            .unwrap()
    );
    assert_actual_fixture_contents(game, source, MEMBERS);
}

fn open_source(game: &mut Game<'_>, id: &str) -> mod_api::LevelRef {
    let source = (game as &mut dyn ServerHost)
        .runtime_levels()
        .open(
            id,
            RuntimeLevel::new("minecraft:overworld", RuntimeGenerator::Void, 1022),
        )
        .unwrap();
    game.apply_runtime_levels();
    source
}

#[test]
#[ignore = "daily"]
fn completed_return_reopens_retired_source_and_recaptures_into_new_columns() {
    let Ok(phase) = std::env::var(PHASE) else {
        let scratch = tempfile::tempdir().unwrap();
        let root = scratch.path().join("same-save");
        for phase in ["return-and-stop", "reopen-and-capture"] {
            let home = scratch.path().join(phase);
            std::fs::create_dir(&home).unwrap();
            let mut command = std::process::Command::new(std::env::current_exe().unwrap());
            command
                .args([
                    "--exact",
                    NAME,
                    "--ignored",
                    "--nocapture",
                    "--test-threads=1",
                ])
                .current_dir(&home)
                .env(PHASE, phase)
                .env(ROOT, &root)
                .env("SUNRISE_NO_AUDIO", "1")
                .env("SUNRISE_LAUNCHER_NO_SELF_UPDATE", "1");
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
                let directory = home.join(key);
                std::fs::create_dir(&directory).unwrap();
                command.env(key, directory);
            }
            let status = command.status().unwrap();
            assert!(status.success(), "actual child {phase} failed: {status}");
        }
        return;
    };
    assert!(matches!(
        phase.as_str(),
        "return-and-stop" | "reopen-and-capture"
    ));
    let root = PathBuf::from(std::env::var_os(ROOT).unwrap());
    mod_platform::testing::with_mods(
        vec![ModEntry::new("pose_fixture", Box::new(PublicationFixture))].into(),
        || {
            super::super::super::native_entity_refs::tests::with_game(|game| {
                game.options.dedicated = Some(Vec::new());
                game.settings.render_distance = 2;
                game.settings.simulation_distance = 2;
                let create = sunrise_client::title::WorldOptions {
                    world_type: sunrise_client::save::WorldPreset::Flat,
                    structures: false,
                    ..Default::default()
                };
                let mut request = crate::game::world_loading::WorldRequest::new(
                    "retired-recapture",
                    Some("retired-recapture"),
                    Some(1022),
                    true,
                    (phase == "return-and-stop").then_some(&create),
                    Arc::new(sunrise_client::gamerules::RuleRegistry::vanilla()),
                )
                .unwrap();
                request.save_root = root.clone();
                game.load_world(request).unwrap(); // Ordinary recovery, lock, metadata and owner readers.
                let _phase =
                    mod_api::__private::PhaseOwner::scope(mod_api::__private::ModPhase::World);
                game.native_lifecycle_world_ready();
                let _owner =
                    mod_api::content::crash::enter_native_mod(mod_api::content::crash::ModCall {
                        id: "pose_fixture",
                        hook: "server_tick",
                        part: None,
                    });
                super::super::super::native_entity_refs::tests::resolve_parts(game).unwrap();
                super::super::super::mods::register_block_entity_types(
                    &game.options.resources,
                    &[],
                )
                .unwrap();
                game.refresh_loop_context().unwrap();
                let parent = game.level("minecraft:overworld").unwrap();
                let parent_context: Vec<_> = TARGETS.iter().chain(OTHER_PARENT).copied().collect();
                if phase == "return-and-stop" {
                    load_columns(game, parent, &parent_context);
                    let source = open_source(game, A);
                    let other = open_source(game, B);
                    load_columns(game, source, MEMBERS);
                    seed_live_source(game, source);
                    load_columns(game, other, MEMBERS);
                    seed_live_source(game, other);
                    game.enter_level(Dim::Overworld).unwrap();
                    let projection = source
                        .project(
                            game,
                            parent,
                            MEMBERS,
                            LiveWorldPose::at([10., 16., 10.]),
                            LiveWorldLimits::default(),
                        )
                        .unwrap();
                    let other_projection = other
                        .project(
                            game,
                            parent,
                            MEMBERS,
                            LiveWorldPose::at([10., 16., -10.]),
                            LiveWorldLimits::default(),
                        )
                        .unwrap();
                    PUBLICATION
                        .set(&mut source.attachments(game), Holder::Level, metadata(1))
                        .unwrap();
                    PUBLICATION
                        .set(&mut other.attachments(game), Holder::Level, metadata(17))
                        .unwrap();
                    game.enter_level(Dim::from_id(A)).unwrap();
                    let stone = game.world.block(17, 64, 0);
                    game.with_level(|behaviours, level, ticks| {
                        assert_eq!(
                            ticks.schedule_block(
                                level.game_time(),
                                BlockPos::new(17, 64, 0),
                                behaviours.block(stone),
                                2000,
                                sunrise_core::ticks::TickPriority::High
                            ),
                            sunrise_core::ticks::ScheduleResult::Added
                        );
                    });
                    game.enter_level(Dim::Overworld).unwrap();
                    assert_eq!(schedule_rows(game, source, MEMBERS).len(), 1);
                    // Finish the ordinary initial generation/save cut before capturing a readset.
                    // This is not a retry of a refused token: preparation has not happened yet.
                    source.checkpoint(game).unwrap();
                    let source_save = game.levels.parked_save_of(A).unwrap().clone();
                    assert_saved_tick(&source_save, MEMBERS[2]);
                    let receipt = move_with_scalar(
                        game,
                        &root,
                        source,
                        parent,
                        (source, MEMBERS),
                        (parent, TARGETS),
                        2,
                    );
                    let retired = receipt
                        .levels
                        .iter()
                        .find(|row| row.level == source)
                        .unwrap();
                    assert_eq!(retired.binding, None);
                    assert_eq!(retired.retired_generation, Some(projection.generation()));
                    assert_eq!(retired.retired_members, Some(0));
                    assert_eq!(projection.sample(game), Err(LiveWorldError::Stale));
                    assert_eq!(other.live_world(game).unwrap(), Some(other_projection));
                    assert!(
                        receipt
                            .levels
                            .iter()
                            .any(|row| row.level == other && row.binding == Some(other_projection))
                    );
                    let saved = source.checkpoint(game).unwrap();
                    assert!(saved.levels.iter().any(|row| row.level == other));
                    let parent_save = game.save.clone();
                    assert_saved_tick(&parent_save, TARGETS[2]);
                    game.stop_integrated(false).unwrap(); // Not a direct SaveDir flush or manual table clear.
                    assert!(game.session_lock.is_none());
                    assert!(!game.live_world_session_failed());
                    assert!(!root.join("data/sunrise_live_world_journal.bin").exists());
                    eprintln!(
                        "LIVE_WORLD_RECAPTURE completed_return=true normal_whole_game_stop=true scalar=2 scheduled=1"
                    );
                } else {
                    let source = open_source(game, A);
                    let other = open_source(game, B);
                    let original_and_new: Vec<_> = MEMBERS.iter().chain(INBOUND).copied().collect();
                    load_columns(game, source, MEMBERS);
                    load_columns(game, parent, &parent_context);
                    load_columns(game, other, MEMBERS);
                    game.enter_level(Dim::Overworld).unwrap();
                    assert_saved_tick(&game.save, TARGETS[2]);
                    assert_eq!(source.live_world(game).unwrap(), None);
                    assert_eq!(source.saved_projection(game), Err(LiveWorldError::Stale));
                    let retirement = source.checkpoint(game).unwrap();
                    let row = retirement
                        .levels
                        .iter()
                        .find(|row| row.level == source)
                        .unwrap();
                    assert_eq!(row.binding, None);
                    assert!(row.retired_generation.is_some());
                    assert_eq!(row.retired_members, Some(0));
                    assert_eq!(
                        PUBLICATION
                            .get_existing(&mut source.attachments(game), Holder::Level)
                            .unwrap(),
                        Some(metadata(2))
                    );
                    load_columns(game, source, INBOUND);
                    game.enter_level(Dim::Overworld).unwrap();
                    assert_actual_empty(game, source, &original_and_new);
                    assert_actual_fixture_contents(game, parent, TARGETS);
                    assert_eq!(schedule_rows(game, parent, TARGETS).len(), 1);
                    let other_projection = other.restore_projection(game).unwrap_or_else(|error| {
                        eprintln!("LIVE_WORLD_RECAPTURE ancillary_restore={error:?} source_ready={:?} parent_ready={:?}",
                            MEMBERS.iter().map(|p| other.should_tick_blocks_at(game,*p)).collect::<Vec<_>>(),
                            OTHER_PARENT.iter().map(|p| parent.should_tick_blocks_at(game,*p)).collect::<Vec<_>>());
                        trace_cut(game, "ancillary B restore refusal", &[source, other, parent]);
                        panic!("ancillary B restore refused: {error:?}");
                    });
                    assert_actual_fixture_contents(game, other, MEMBERS);
                    assert_eq!(
                        PUBLICATION
                            .get_existing(&mut other.attachments(game), Holder::Level)
                            .unwrap(),
                        Some(metadata(17))
                    );
                    // The new destination columns above were loaded by actual Streaming, not
                    // prepopulated. Settle their completed save frontier before preparing Capture.
                    source.checkpoint(game).unwrap();
                    // Quiesce can finish another saved full column. Complete the same ordinary
                    // registration/ticket/unpack frontier against that final actual resident set.
                    game.enter_level(Dim::Overworld).unwrap();
                    game.sync_tick_chunks();
                    game.update_chunk_tickets();
                    game.start_ticking_chunks();
                    let receipt = move_with_scalar(
                        game,
                        &root,
                        source,
                        parent,
                        (parent, TARGETS),
                        (source, INBOUND),
                        3,
                    );
                    assert!(
                        receipt
                            .levels
                            .iter()
                            .any(|row| row.level == other && row.binding == Some(other_projection))
                    );
                    assert_eq!(source.live_world(game).unwrap(), None);
                    assert_actual_empty(game, source, MEMBERS);
                    assert_actual_empty(game, parent, TARGETS);
                    assert_actual_fixture_contents(game, source, INBOUND);
                    let saved = source.checkpoint(game).unwrap();
                    let row = saved.levels.iter().find(|row| row.level == source).unwrap();
                    assert_eq!(row.binding, None);
                    assert_eq!(
                        row.retired_members, None,
                        "a newly populated unbound Source is not an empty-body witness"
                    );
                    game.stop_integrated(false).unwrap();
                    assert!(game.session_lock.is_none());
                    assert!(!game.live_world_session_failed());
                    eprintln!(
                        "LIVE_WORLD_RECAPTURE normal_fresh_reopen=true retired_source_reused=true new_source_columns=true scalar=3 scheduled=1 other_binding_preserved=true"
                    );
                }
            });
        },
    );
}
