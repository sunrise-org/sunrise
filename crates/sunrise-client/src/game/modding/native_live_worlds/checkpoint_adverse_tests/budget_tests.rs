//! Real ordinary/common SaveDir decisions over resident near/far terrain, not a mock save.
use super::*;
use sunrise_core::{
    nbt::Tag,
    position::{BlockPos, ChunkPos},
    ticks::{ScheduleResult, TickPriority},
};

const PAD: &str = "fixture:opaque_terrain";

fn opaque(game: &mut Game<'_>, x: i32, bytes: usize) {
    if !game.world.chunks.contains_key(&(x, 0)) {
        game.world
            .chunks
            .insert((x, 0), Arc::new(Chunk::empty(x, 0)));
    }
    game.streaming.forced.insert((x, 0));
    game.world.with_chunk_extras((x * 16, 0, 0), |extras| {
        if bytes == 0 {
            extras.root.remove(&PAD.into());
        } else {
            extras
                .root
                .insert(PAD.into(), Tag::ByteArray(vec![37; bytes]));
        }
    });
}

// actual_game replaces the current SaveDir to align its real scratch root. Bind the same
// asset-derived stock SaveTables used by ordinary host setup; never invent tick-name maps.
fn bind_actual_save_tables(game: &mut Game<'_>, source: mod_api::LevelRef) {
    for dimension in [Dim::from_id(source.dimension_id()), Dim::Overworld] {
        game.enter_level(dimension).unwrap();
        game.save
            .tables
            .get_or_init(|| Arc::new(sunrise_client::save::SaveTables::new(&game.pack.assets)));
    }
}

fn reopen_column(game: &Game<'_>, x: i32) -> Chunk {
    let reader = SaveDir::new(
        game.save.root.clone(),
        game.world.shape,
        Some(Default::default()),
    );
    assert!(
        reader
            .tables
            .set(
                game.save
                    .tables
                    .get()
                    .expect("actual producer registry tables")
                    .clone()
            )
            .is_ok()
    );
    reader
        .load(x, 0, game.pack.assets.registry, world_biomes!(game))
        .unwrap()
}

fn saved_poi_sections(game: &Game<'_>, x: i32) -> usize {
    let raw = sunrise_world::carrier::region_strict::read_chunk(
        &game.save.root.join("poi"),
        ChunkPos::new(x, 0),
    )
    .unwrap()
    .expect("actual independently read POI record");
    let Some(Tag::Compound(sections)) = raw.tag.get("Sections") else {
        panic!("ordinary POI Sections missing");
    };
    sections.len()
}

fn assert_padding(chunk: &Chunk, bytes: usize) {
    let value = chunk
        .saved
        .as_ref()
        .and_then(|extra| extra.root.get(&PAD.into()));
    if bytes == 0 {
        assert!(value.is_none());
    } else {
        assert!(
            matches!(value, Some(Tag::ByteArray(value)) if value.len() == bytes && value.iter().all(|b| *b == 37))
        );
    }
}

#[test]
#[ignore = "daily"]
fn deferred_stream_retirement_replans_at_common_frontier_and_stops_repeating_checkpoints() {
    const NAME: &str = "game::native_live_worlds_tests::checkpoint_adverse_tests::budget_tests::deferred_stream_retirement_replans_at_common_frontier_and_stops_repeating_checkpoints";
    if fresh(NAME) {
        return;
    }
    actual_game(
        vec![ModEntry::new("pose_fixture", Box::new(PublicationFixture))],
        |game, root| {
            let source = prepare_named(game, A);
            bind_actual_save_tables(game, source);
            let parent = game.level("minecraft:overworld").unwrap();
            let projection = source
                .project(
                    game,
                    parent,
                    MEMBERS,
                    LiveWorldPose::at([10., 16., 10.]),
                    LiveWorldLimits::default(),
                )
                .unwrap();
            let sample = projection.sample(game).unwrap();
            // Keep unrelated generated/meshed arrivals out of this constructed fixture's
            // repeated save observations. Streaming remains unpaused and its real planning,
            // common admission, ownership removal and unload callbacks run normally.
            for dimension in [Dim::Overworld, Dim::from_id(source.dimension_id())] {
                game.enter_level(dimension).unwrap();
                game.streaming.abort_generation();
                game.streaming.paused = false;
                game.streaming.mesh = false;
                game.streaming.center = (0, 0);
                game.streaming.radius = 0;
            }
            let far = (64, 0);
            game.world
                .chunks
                .insert(far, Arc::new(Chunk::empty(far.0, far.1)));
            let stone = game
                .pack
                .assets
                .registry
                .default_state("minecraft:stone")
                .unwrap();
            game.world.set(far.0 * 16 + 5, 71, 5, stone).unwrap();
            game.world.with_chunk_extras((far.0 * 16, 0, 0), |extras| {
                extras
                    .root
                    .insert(PAD.into(), Tag::ByteArray(vec![37; 128]));
            });
            source.checkpoint(game).unwrap();
            let before = wal::read_epoch(root).unwrap();
            assert!(before > 0);
            assert!(!game.save.live_world_barrier_pending());
            assert!(!game.streaming.forced.contains(&far));

            // A due ordinary sweep requests common admission and keeps the live owner.
            // Frontier service must re-plan that actual retirement, rather than save an
            // empty fresh plan and leave the same far column for every later sweep.
            game.update_world();
            assert!(game.world.chunks.contains_key(&far));
            assert!(game.save.live_world_barrier_pending());
            assert_eq!(wal::read_epoch(root).unwrap(), before);
            game.service_live_world_saves().unwrap();
            assert!(
                !game.world.chunks.contains_key(&far),
                "the acknowledged common decision must actually retire the deferred column"
            );
            assert!(wal::read_epoch(root).unwrap() > before);
            let reopened = reopen_column(game, far.0);
            assert_eq!(reopened.block(5, 71, 5), stone);
            assert_padding(&reopened, 128);

            // Ordinary post-unload entity cleanup may itself queue a save. Preserve and
            // acknowledge it before checking that unchanged future sweeps need no more WAL.
            for _ in 0..4 {
                if !game.save.live_world_barrier_pending() {
                    break;
                }
                game.service_live_world_saves().unwrap();
            }
            assert!(!game.save.live_world_barrier_pending());
            let settled = wal::read_epoch(root).unwrap();
            for _ in 0..27 {
                game.update_world();
                game.service_live_world_saves().unwrap();
                assert!(!game.world.chunks.contains_key(&far));
                assert_eq!(
                    wal::read_epoch(root).unwrap(),
                    settled,
                    "unchanged ordinary sweeps cannot repeatedly checkpoint a retired column"
                );
            }
            assert_eq!(projection.sample(game).unwrap(), sample);
            assert_eq!(source.live_world(game).unwrap(), Some(projection));
            assert!(game.world.chunks.contains_key(&(0, 0)));
            assert!(game.world.chunks.contains_key(&(1, 0)));
            assert_actual_fixture_contents(game, source, MEMBERS);
            assert!(!root.join("data/sunrise_live_world_journal.bin").exists());
        },
    );
}

#[test]
#[ignore = "daily"]
fn common_ordinary_save_omits_clean_far_terrain_keeps_opaque_and_dirty_budget_ack() {
    const NAME: &str = "game::native_live_worlds_tests::checkpoint_adverse_tests::budget_tests::common_ordinary_save_omits_clean_far_terrain_keeps_opaque_and_dirty_budget_ack";
    if fresh(NAME) {
        return;
    }
    actual_game(
        vec![ModEntry::new("pose_fixture", Box::new(PublicationFixture))],
        |game, root| {
            let source = prepare_named(game, A);
            bind_actual_save_tables(game, source);
            let parent = game.level("minecraft:overworld").unwrap();
            // Ordinary save before common mode: five already-durable near/far columns contain
            // 60 MiB of raw, compressible keep-as-loaded terrain. No dirty flag is manually cleared.
            for x in [0, 1, 64, 65, 128] {
                opaque(game, x, 12 << 20);
            }
            let poi = game
                .pack
                .assets
                .registry
                .default_state("minecraft:composter")
                .unwrap();
            game.world.set(64 * 16 + 5, 71, 5, poi).unwrap();
            assert!(!game.live_world_durability_active());
            game.save_level(true).unwrap();
            assert_eq!(saved_poi_sections(game, 64), 1);
            assert_eq!(wal::read_epoch(root).unwrap(), 0);
            assert!(!root.join("data/sunrise_live_world_journal.bin").exists());
            assert!(game.world.modified.is_empty());
            assert_padding(&reopen_column(game, 64), 12 << 20);

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
            let token = source
                .prepare_transfer(
                    game,
                    &cells,
                    mod_api::LiveWorldTransferLimits {
                        bytes: 64 << 20,
                        ..Default::default()
                    },
                )
                .unwrap();
            (game as &mut dyn ServerHost)
                .transaction(|host, tx| token.stage(host, tx))
                .unwrap();
            let receipt = token.receipt(game).unwrap();
            assert_eq!(receipt.retired_bindings, vec![projection]);
            assert_eq!(source.live_world(game).unwrap(), None);
            assert_actual_empty(game, source, MEMBERS);
            assert_actual_fixture_contents(game, parent, &targets);
            assert!(
                game.live_world_durability_active(),
                "ordinary saving stays common after the last binding"
            );

            game.enter_level(Dim::from_id(A)).unwrap();
            opaque(game, 0, 8 << 20);
            game.enter_level(Dim::Overworld).unwrap();
            game.save_level(true).unwrap();
            let report = game.control_live_worlds(None);
            let usage = &report["last_checkpoint_capture"];
            assert_eq!(usage["fits_byte_budget"], true);
            assert_eq!(usage["exact"], true);
            assert!(
                usage["poi_records"].as_u64().unwrap() > 0,
                "the real POI in a clean omitted terrain column still participates"
            );
            assert_eq!(saved_poi_sections(game, 64), 1);
            eprintln!("LIVE_WORLD_BUDGET exact_after_common_save={usage}");
            assert!(
                usage["payload_bytes"].as_u64().unwrap() < 40 << 20,
                "clean far terrain is absent from this common payload: {usage:?}"
            );
            assert!(
                usage["entity_records"].as_u64().unwrap() >= game.world.chunks.len() as u64,
                "all resident entity/deletion columns remain covered"
            );
            assert!(game.world.modified.is_empty());
            for x in [64, 65, 128] {
                assert_padding(&reopen_column(game, x), 12 << 20);
            }
            game.enter_level(Dim::from_id(A)).unwrap();
            assert_padding(&reopen_column(game, 0), 8 << 20);
            assert!(game.world.modified.is_empty());

            // Establish a truly clean persisted queue, consume its last stale-kind occurrence
            // through ordinary behavior, then require the later Budget refusal to retain it.
            game.enter_level(Dim::Overworld).unwrap();
            let pos = BlockPos::new(5, 64, 0);
            let column = ChunkPos::new(0, 0);
            let stone = game
                .pack
                .assets
                .registry
                .default_state("minecraft:stone")
                .unwrap();
            game.with_level(|behaviours, level, ticks| {
                use sunrise_world::level::LevelAccess as _;
                assert_eq!(
                    ticks.schedule_block(
                        level.game_time(),
                        pos,
                        behaviours.block(stone),
                        0,
                        TickPriority::Normal
                    ),
                    ScheduleResult::Added
                );
            });
            game.save_level(true).unwrap();
            assert!(
                !game
                    .block_ticks
                    .blocks
                    .chunk(column)
                    .unwrap()
                    .needs_persistent_save()
            );
            game.with_level(|behaviours, level, ticks| {
                assert!(behaviours.scheduled_begin(level, ticks, false));
                assert!(behaviours.scheduled_step(level, ticks, false));
                assert!(!behaviours.scheduled_step(level, ticks, false));
                behaviours.scheduled_finish(ticks, false);
            });
            assert_eq!(game.block_ticks.blocks.chunk(column).unwrap().count(), 0);
            assert!(
                game.block_ticks
                    .blocks
                    .chunk(column)
                    .unwrap()
                    .needs_persistent_save()
            );
            assert!(game.world.modified.is_empty());
            game.enter_level(Dim::from_id(A)).unwrap();

            // A genuine oversized dirty owner still refuses before Prepared. Its lower-bound
            // telemetry is labelled partial, all dirty columns and the old checkpoint survive.
            for x in 128..133 {
                opaque(game, x, 14 << 20);
            }
            game.enter_level(Dim::Overworld).unwrap();
            let epoch = wal::read_epoch(root).unwrap();
            let files = core_images(root, &[A, "minecraft:overworld"]);
            assert_eq!(source.checkpoint(game), Err(LiveWorldError::Budget));
            let report = game.control_live_worlds(None);
            let usage = &report["last_checkpoint_capture"];
            assert_eq!(usage["fits_byte_budget"], false);
            assert_eq!(usage["exact"], false);
            eprintln!("LIVE_WORLD_BUDGET refused_dirty_payload={usage}");
            assert!(usage["required_bytes"].is_null());
            assert!(
                usage["required_bytes_at_least"].as_u64().unwrap()
                    > usage["limit_bytes"].as_u64().unwrap()
            );
            assert_eq!(wal::read_epoch(root).unwrap(), epoch);
            assert_eq!(core_images(root, &[A, "minecraft:overworld"]), files);
            assert!(!root.join("data/sunrise_live_world_journal.bin").exists());
            assert!(!game.live_world_session_failed());
            assert!(
                game.block_ticks
                    .blocks
                    .chunk(column)
                    .unwrap()
                    .needs_persistent_save(),
                "failed capture cannot acknowledge the empty queue"
            );
            game.enter_level(Dim::from_id(A)).unwrap();
            for x in 128..133 {
                assert!(game.world.modified.contains(&(x, 0)));
                opaque(game, x, 0); // Actual caller deletion is a new dirty afterimage, never a flag reset.
            }
            opaque(game, 0, 0);
            game.enter_level(Dim::Overworld).unwrap();
            game.world.set(64 * 16 + 5, 71, 5, game.world.air).unwrap();
            game.save_level(true).unwrap();
            assert_eq!(
                saved_poi_sections(game, 64),
                0,
                "the actual last-POI deletion writes empty trusted Sections"
            );
            assert_padding(&reopen_column(game, 64), 12 << 20);
            assert!(
                !game
                    .block_ticks
                    .blocks
                    .chunk(column)
                    .unwrap()
                    .needs_persistent_save()
            );
            assert!(
                reopen_column(game, 0)
                    .saved
                    .as_ref()
                    .unwrap()
                    .block_ticks
                    .is_empty()
            );
            game.enter_level(Dim::from_id(A)).unwrap();
            assert!(game.world.modified.is_empty());
            for x in [0, 128, 129, 130, 131, 132] {
                assert_padding(&reopen_column(game, x), 0);
            }
        },
    );
}

#[test]
#[ignore = "daily"]
fn stale_last_scheduled_tick_saves_empty_afterimage_and_reopens_without_replay() {
    const NAME: &str = "game::native_live_worlds_tests::checkpoint_adverse_tests::budget_tests::stale_last_scheduled_tick_saves_empty_afterimage_and_reopens_without_replay";
    if fresh(NAME) {
        return;
    }
    actual_game(
        vec![ModEntry::new("pose_fixture", Box::new(PublicationFixture))],
        |game, root| {
            let source = prepare_named(game, A);
            bind_actual_save_tables(game, source);
            let parent = game.level("minecraft:overworld").unwrap();
            settle_transfer_columns(game, parent);
            let pos = BlockPos::new(5, 64, 0);
            let chunk = ChunkPos::new(0, 0);
            let stone = game
                .pack
                .assets
                .registry
                .default_state("minecraft:stone")
                .unwrap();
            assert!(
                game.res
                    .behaviours()
                    .is_air(game.world.block(pos.x, pos.y, pos.z))
            );
            // Scheduling alone is not an ordinary World terrain edit. Persist this real column
            // with an unrelated actual block mutation; the stale tick's target remains AIR.
            game.world.set(4, 64, 0, stone).unwrap();
            game.with_level(|behaviours, level, ticks| {
                use sunrise_world::level::LevelAccess as _;
                assert_eq!(
                    ticks.schedule_block(
                        level.game_time(),
                        pos,
                        behaviours.block(stone),
                        0,
                        TickPriority::Normal
                    ),
                    ScheduleResult::Added
                );
            });
            game.save_level(true).unwrap();
            assert_eq!(wal::read_epoch(root).unwrap(), 0);
            let raw = game.save.raw_chunk(0, 0).unwrap();
            assert!(
                matches!(raw.tag.get("block_ticks"), Some(Tag::List(ticks)) if ticks.len() == 1),
                "the actual ordinary writer persisted the initial occurrence"
            );
            assert_eq!(
                reopen_column(game, 0)
                    .saved
                    .as_ref()
                    .unwrap()
                    .block_ticks
                    .len(),
                1
            );
            source.checkpoint(game).unwrap();
            assert!(
                !game
                    .block_ticks
                    .blocks
                    .chunk(chunk)
                    .unwrap()
                    .needs_persistent_save()
            );
            let before = (
                game.world.revision,
                game.world
                    .modified
                    .iter()
                    .copied()
                    .collect::<std::collections::BTreeSet<_>>(),
            );
            let steps = game.with_level(|behaviours, level, ticks| {
                assert!(behaviours.scheduled_begin(level, ticks, false));
                let mut steps = 0;
                while behaviours.scheduled_step(level, ticks, false) {
                    steps += 1;
                }
                behaviours.scheduled_finish(ticks, false);
                steps
            });
            assert_eq!(
                steps, 1,
                "the real mismatched-kind path consumes exactly one persisted occurrence"
            );
            assert_eq!(
                (
                    game.world.revision,
                    game.world
                        .modified
                        .iter()
                        .copied()
                        .collect::<std::collections::BTreeSet<_>>()
                ),
                before
            );
            let queue = game.block_ticks.blocks.chunk(chunk).unwrap();
            assert_eq!(queue.count(), 0);
            assert!(queue.needs_persistent_save());
            game.save_level(true).unwrap();
            assert!(
                !game
                    .block_ticks
                    .blocks
                    .chunk(chunk)
                    .unwrap()
                    .needs_persistent_save()
            );
            let reopened = reopen_column(game, 0);
            assert!(reopened.saved.as_ref().unwrap().block_ticks.is_empty());
            assert!(game.world.modified.is_empty());
            assert!(!root.join("data/sunrise_live_world_journal.bin").exists());
            game.with_level(|behaviours, level, ticks| {
                assert!(behaviours.scheduled_begin(level, ticks, false));
                assert!(!behaviours.scheduled_step(level, ticks, false));
                behaviours.scheduled_finish(ticks, false);
            });
        },
    );
}
