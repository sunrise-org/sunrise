//! Runtime-backed projection acceptance evidence; no synthetic block or inventory implementation.
use super::{Dim, Game, api_values::TryIntoApi as _};
use mod_api::{
    Hooks, LiveWorldError, LiveWorldLimits, LiveWorldPose, ModEntry, RuntimeGenerator, RuntimeLevel,
};
use std::{
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicU8, Ordering},
    },
};
use sunrise_client::{
    chunk::Chunk,
    containers::{BlockEntity, Furnace, FurnaceKind},
    items::ItemStack,
};

struct Fixture(Arc<AtomicU8>);
#[mod_api::__subscriptions]
impl Hooks for Fixture {
    fn break_block(
        &self,
        host: &mut dyn mod_api::ServerHost,
        _: &mut mod_api::BreakBlockEvent,
    ) -> mod_api::Flow {
        match self.0.load(Ordering::Relaxed) {
            3 => {
                let source = host.level("live_fixture:local").unwrap();
                source
                    .live_world(host)
                    .unwrap()
                    .unwrap()
                    .retire(host)
                    .unwrap();
            }
            4 => {
                host.player(123)
                    .unwrap()
                    .set_inventory_slot(host, 0, None)
                    .unwrap();
            }
            _ => {}
        }
        mod_api::Flow::Continue
    }
    fn live_world_admission(
        &self,
        host: &mut dyn mod_api::ServerHost,
        event: &mut mod_api::LiveWorldAdmissionEvent,
    ) -> mod_api::Flow {
        assert!(!event.changes.is_empty() && event.changes.len() <= 3);
        match self.0.load(Ordering::Relaxed) {
            1 => mod_api::Flow::Cancel,
            2 => {
                event
                    .player
                    .unwrap()
                    .set_inventory_slot(host, 0, None)
                    .unwrap();
                mod_api::Flow::Continue
            }
            _ => mod_api::Flow::Continue,
        }
    }
}

fn owner() -> mod_api::content::crash::ModGuard {
    mod_api::content::crash::enter_native_mod(mod_api::content::crash::ModCall {
        id: "live_fixture",
        hook: "level_tick_post",
        part: None,
    })
}

fn prepare(game: &mut Game<'_>) -> mod_api::LevelRef {
    prepare_named(game, "live_fixture:local")
}
pub(super) fn prepare_named(game: &mut Game<'_>, id: &str) -> mod_api::LevelRef {
    if game.session_lock.is_none() {
        std::fs::create_dir_all(&game.world_root).unwrap();
        game.session_lock =
            Some(sunrise_client::save::SessionLock::acquire(&game.world_root).unwrap());
        game.save = Arc::new(sunrise_client::save::SaveDir::new(
            sunrise_client::save::dimension_dir(&game.world_root, game.dim.id()),
            game.world.shape,
            Some(Default::default()),
        ));
    }
    game.options.dedicated = Some(Vec::new());
    super::native_entity_refs::tests::resolve_parts(game).unwrap();
    super::mods::register_block_entity_types(&game.options.resources, &[]).unwrap();
    game.refresh_loop_context().unwrap();
    let level = game
        .runtime_level_open(
            id,
            RuntimeLevel::new("minecraft:overworld", RuntimeGenerator::Void, 1022),
        )
        .unwrap();
    game.apply_runtime_levels();
    game.enter_level(Dim::from_id(id)).unwrap();
    for x in [0, 1] {
        game.world
            .chunks
            .insert((x, 0), Arc::new(Chunk::empty(x, 0)));
        game.streaming.forced.insert((x, 0));
    }
    let chest = game
        .pack
        .assets
        .registry
        .default_state("minecraft:chest")
        .unwrap();
    let furnace = game
        .pack
        .assets
        .registry
        .default_state("minecraft:furnace")
        .unwrap();
    let stone = game
        .pack
        .assets
        .registry
        .default_state("minecraft:stone")
        .unwrap();
    game.world.set(0, 64, 0, chest).unwrap();
    game.world.set(1, 64, 0, furnace).unwrap();
    game.world.set(17, 64, 0, stone).unwrap();
    let mut inventory = vec![None; 27];
    inventory[0] = Some(ItemStack::new("minecraft:stone", 7));
    game.world
        .block_entities
        .insert((0, 64, 0), BlockEntity::Chest(inventory));
    game.world.block_entities.insert(
        (1, 64, 0),
        BlockEntity::Furnace(Furnace {
            kind: FurnaceKind::Furnace,
            items: [
                Some(ItemStack::new("minecraft:raw_iron", 1)),
                Some(ItemStack::new("minecraft:coal", 1)),
                None,
            ],
            lit_remaining: 5,
            lit_total: 200,
            cooking_timer: 40,
            cooking_total: 200,
            speed: 1.0,
            recipes_used: HashMap::new(),
        }),
    );
    game.enter_level(Dim::Overworld).unwrap();
    for x in [-1, 0, 1, 2] {
        for z in [-1, 0, 1] {
            game.world
                .chunks
                .insert((x, z), Arc::new(Chunk::empty(x, z)));
        }
    }
    level
}

#[test]
#[ignore = "daily"]
fn runtime_projection_keeps_real_multichunk_inventory_ticker_and_sparse_shapes() {
    if super::native_entity_refs::tests::run_fresh_cases(
        "game::native_live_worlds_tests::runtime_projection_keeps_real_multichunk_inventory_ticker_and_sparse_shapes",
        "SUNRISE_LIVE_WORLD_RUNTIME_CHILD",
        &["runtime"],
    ) {
        return;
    }
    mod_platform::testing::with_mods(
        vec![ModEntry::new(
            "live_fixture",
            Box::new(Fixture(Arc::new(AtomicU8::new(0)))),
        )]
        .into(),
        || {
            super::native_entity_refs::tests::with_game(|game| {
                let _phase =
                    mod_api::__private::PhaseOwner::scope(mod_api::__private::ModPhase::World);
                let scratch = tempfile::tempdir().unwrap();
                game.world_root = scratch.path().to_path_buf();
                game.session_lock =
                    Some(sunrise_client::save::SessionLock::acquire(&game.world_root).unwrap());
                game.save = Arc::new(sunrise_client::save::SaveDir::new(
                    sunrise_client::save::dimension_dir(&game.world_root, "minecraft:overworld"),
                    game.world.shape,
                    Some(Default::default()),
                ));
                let _owner = owner();
                let source = prepare(game);
                let destination = game.level("minecraft:overworld").unwrap();
                assert_eq!(
                    source.restore_projection(game),
                    Err(LiveWorldError::NotSaved)
                );
                let footprint = source
                    .projection_footprint(game, &[[0, 64, 0], [1, 64, 0], [17, 64, 0]])
                    .unwrap();
                assert_eq!(
                    (
                        footprint.cells,
                        footprint.source_chunks,
                        footprint.retained_columns,
                        footprint.view_areas
                    ),
                    (3, 2, 90, 2)
                );
                let pose = LiveWorldPose::at([10.0, 0.0, 10.0]);
                let projection = source
                    .project(
                        game,
                        destination,
                        &[[0, 64, 0], [1, 64, 0], [17, 64, 0]],
                        pose,
                        LiveWorldLimits::default(),
                    )
                    .unwrap();
                let cells = projection.cells(game).unwrap();
                assert_eq!(
                    cells.iter().map(|c| c.pos).collect::<Vec<_>>(),
                    [[0, 64, 0], [1, 64, 0], [17, 64, 0]]
                );
                // Losing a real source chunk cannot turn a previously coherent support into air.
                let missing = {
                    let parked = game.levels.parked_mut_of(source.dimension_id()).unwrap();
                    let chunk = parked.world.chunks.remove(&(1, 0)).unwrap();
                    let stone = game
                        .pack
                        .assets
                        .registry
                        .default_state("minecraft:stone")
                        .unwrap();
                    parked.world.set(2, 65, 0, stone).unwrap(); // actual unrelated state invalidation
                    chunk
                };
                assert_eq!(
                    projection.publish_pose(game, 0, pose),
                    Err(LiveWorldError::NotResident)
                );
                let colliders = game.live_world_colliders(
                    sunrise_client_render::moving_colliders::Prepared::empty(),
                );
                let platform = colliders
                    .placed()
                    .next()
                    .expect("last coherent source support remains placed");
                let body = sunrise_core::geometry::Aabb::new(
                    sunrise_core::geometry::Vec3::new(27.2, 65.0, 10.2),
                    sunrise_core::geometry::Vec3::new(27.8, 66.8, 10.8),
                );
                assert!(
                    platform
                        .collider
                        .collide(body, sunrise_core::geometry::Axis::Y, -0.25)
                        .abs()
                        < 1.0e-7,
                    "ordinary moving collider still supports the body over the missing source wing"
                );
                game.levels
                    .parked_mut_of(source.dimension_id())
                    .unwrap()
                    .world
                    .chunks
                    .insert((1, 0), missing);
                assert_eq!(
                    projection.cells(game).unwrap().len(),
                    3,
                    "fresh coherent source resumes without reconstruction"
                );
                let chest = cells[0].entity.unwrap();
                assert_eq!(chest.level(game), Some(source));
                assert_eq!(
                    cells[2].state,
                    game.pack
                        .assets
                        .registry
                        .default_state("minecraft:stone")
                        .unwrap()
                        .try_into_api()
                        .unwrap()
                );
                assert!(
                    projection
                        .clip(game, [12.5, 66.0, 10.5], [12.5, 63.0, 10.5])
                        .unwrap()
                        .is_none(),
                    "sparse gap stays empty"
                );
                let hit = projection
                    .clip(game, [27.5, 66.0, 10.5], [27.5, 63.0, 10.5])
                    .unwrap()
                    .unwrap();
                assert_eq!(hit.cell, [17, 64, 0]);
                assert_eq!(projection.validate_hit(game, hit), Ok(()));
                assert_eq!(
                    projection.publish_pose(game, 99, LiveWorldPose::at([11.0, 0.0, 10.0])),
                    Err(LiveWorldError::Revision)
                );
                game.live_world_begin_tick();
                let start = game.placed_moving_colliders();
                assert_eq!(start.len(), 1);
                let moved = projection
                    .publish_pose(game, 0, LiveWorldPose::at([11.0, 0.0, 10.0]))
                    .unwrap();
                assert_eq!(moved.previous, pose);
                assert_eq!(moved.current.translation, [11.0, 0.0, 10.0]);
                assert_eq!(
                    projection.validate_hit(game, hit),
                    Err(LiveWorldError::Revision)
                );
                game.live_world_end_level();
                assert_ne!(
                    start.placed().next().unwrap().collider.frame,
                    game.collider_now_frame(start.placed().next().unwrap().id)
                        .unwrap()
                );
                assert_eq!(
                    projection.cells(game).unwrap()[0].entity,
                    Some(chest),
                    "motion preserves the live holder generation"
                );
                game.live_world_begin_tick();
                let mut coupled = LiveWorldPose::at([11.2, -0.1, 10.0]);
                let half = 0.5_f64.to_radians();
                coupled.rotation = [0.0, half.sin(), 0.0, half.cos()];
                let accepted = projection.publish_pose(game, 1, coupled).unwrap();
                assert_eq!(accepted.current, coupled);
                for _ in 0..3 {
                    game.live_world_begin_tick();
                    let idle = projection.sample(game).unwrap();
                    assert_eq!(
                        idle.previous, idle.current,
                        "idle external publisher settles without repeated delta"
                    );
                }
                // This fixture is a logical server; the separate client/window route owns drawing evidence.
                game.enter_level(Dim::from_id("live_fixture:local"))
                    .unwrap();
                game.tick_block_entities().unwrap();
                let Some(BlockEntity::Furnace(furnace)) =
                    game.world.block_entities.get(&(1, 64, 0))
                else {
                    panic!("real furnace missing")
                };
                assert_eq!(
                    furnace.cooking_timer, 41,
                    "the one real source-level ticker advances"
                );
                let Some(BlockEntity::Chest(items)) = game.world.block_entities.get(&(0, 64, 0))
                else {
                    panic!("real chest missing")
                };
                assert_eq!(items[0].as_ref().unwrap().count, 7);
                game.enter_level(Dim::Overworld).unwrap();
                assert!(
                    game.world.block_entities.get(&(0, 64, 0)).is_none(),
                    "no destination copy of the local inventory"
                );
                let saved_sample = projection.sample(game).unwrap();
                let checkpoint = source.checkpoint(game).unwrap();
                let saved_receipt = checkpoint
                    .levels
                    .iter()
                    .find(|row| row.level == source)
                    .unwrap();
                assert_eq!(saved_receipt.binding, Some(projection));
                assert!(checkpoint.barrier_epoch > 0);
                // Actual source close retires its binding and reservations, without a manual table reset.
                assert!(projection.sample(game).is_ok());
                game.runtime_level_unload("live_fixture:local").unwrap();
                game.apply_runtime_levels();
                assert_eq!(projection.sample(game), Err(LiveWorldError::Stale));
                assert_eq!(game.live_worlds.borrow().binding_count(), 0);
                let reopened = game
                    .runtime_level_open(
                        "live_fixture:local",
                        RuntimeLevel::new("minecraft:overworld", RuntimeGenerator::Void, 1022),
                    )
                    .unwrap();
                game.apply_runtime_levels();
                assert_ne!(source, reopened);
                game.enter_level(Dim::from_id("live_fixture:local"))
                    .unwrap();
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
                        && game.world.chunks.contains_key(&(1, 0)),
                    "saved source chunks reload through normal streaming"
                );
                let Some(BlockEntity::Furnace(saved)) = game.world.block_entities.get(&(1, 64, 0))
                else {
                    panic!("saved real furnace missing")
                };
                assert_eq!(saved.cooking_timer, 41);
                let Some(BlockEntity::Chest(saved)) = game.world.block_entities.get(&(0, 64, 0))
                else {
                    panic!("saved real chest missing")
                };
                assert_eq!(saved[0].as_ref().unwrap().count, 7);
                // Complete the real ServerLevel entity-manager/residency barrier without
                // advancing the furnace again. Never clear pending flags by hand.
                game.tick_world(false).unwrap();
                assert!(game.live_world_tick_ready(reopened, [0, 64, 0]));
                game.enter_level(Dim::Overworld).unwrap();
                let mut restored = reopened.restore_projection(game).unwrap();
                let sample = restored.sample(game).unwrap();
                assert_eq!(
                    sample.current.translation.map(f64::to_bits),
                    saved_sample.current.translation.map(f64::to_bits)
                );
                assert_eq!(
                    sample.current.pivot.map(f64::to_bits),
                    saved_sample.current.pivot.map(f64::to_bits)
                );
                assert_eq!(
                    sample.current.rotation.map(f64::to_bits),
                    saved_sample.current.rotation.map(f64::to_bits)
                );
                assert_eq!(sample.revision, saved_sample.revision);
                assert_eq!(
                    sample.previous, sample.current,
                    "reopen settles presentation rather than replaying historical motion"
                );
                for _ in 0..2 {
                    reopened.checkpoint(game).unwrap();
                    restored.retire(game).unwrap();
                    restored = reopened.restore_projection(game).unwrap();
                    let again = restored.sample(game).unwrap();
                    assert_eq!(
                        again.current.rotation.map(f64::to_bits),
                        saved_sample.current.rotation.map(f64::to_bits)
                    );
                    assert_eq!(
                        again.current.translation.map(f64::to_bits),
                        saved_sample.current.translation.map(f64::to_bits)
                    );
                    assert_eq!(
                        again.current.pivot.map(f64::to_bits),
                        saved_sample.current.pivot.map(f64::to_bits)
                    );
                    assert_eq!(again.revision, saved_sample.revision);
                    assert_eq!(again.previous, again.current);
                }
                assert_ne!(restored, projection);
                assert_eq!(
                    restored.sample(game).unwrap().current.translation,
                    [11.2, -0.1, 10.0]
                );
                assert_ne!(
                    restored.cells(game).unwrap()[0].entity,
                    Some(chest),
                    "old source-open handles stay stale after the real reopen"
                );
                let inspected = game.control_live_worlds(Some("live_fixture:local"));
                assert_eq!(inspected["cell_count"], 3);
                let actual = &inspected["projections"][0];
                assert_eq!(actual["generation"], restored.generation());
                assert_eq!(actual["source_generation"], restored.level().generation());
                let inspected_cells = actual["cells"].as_array().unwrap();
                let chest_cell = inspected_cells
                    .iter()
                    .find(|cell| cell["pos"] == serde_json::json!([0, 64, 0]))
                    .unwrap();
                assert_eq!(chest_cell["block_entity"]["slots"][0]["count"], 7);
                let counter_cell = inspected_cells
                    .iter()
                    .find(|cell| cell["pos"] == serde_json::json!([1, 64, 0]))
                    .unwrap();
                assert_eq!(
                    counter_cell["block_entity"]["progress"]["cooking_timer"],
                    41
                );
                assert_eq!(
                    actual["sample"]["current"]["translation_bits"],
                    serde_json::json!(
                        restored
                            .sample(game)
                            .unwrap()
                            .current
                            .translation
                            .map(f64::to_bits)
                    )
                );
                restored.retire(game).unwrap();
                assert_eq!(restored.sample(game), Err(LiveWorldError::Stale));
                assert_eq!(game.control_live_worlds(None)["projection_count"], 0);
                eprintln!(
                    "LIVE_WORLD_EVIDENCE {}",
                    serde_json::json!({"source":source.dimension_id(),
                "barrier_epoch":checkpoint.barrier_epoch,"generation":projection.generation(),"source_revision":saved_receipt.block_revision,
                "cells":[[0,64,0],[1,64,0],[17,64,0]],"inventory_count":7,"counter":41,
                "sparse_gap_missed":true,"client_render":"separate_route_pending","restored_generation":restored.generation()})
                );
            });
        },
    );
}

#[test]
#[ignore = "daily"]
fn native_live_world_ordinary_actions_admit_revalidate_and_update_real_membership() {
    if super::native_entity_refs::tests::run_fresh_cases(
        "game::native_live_worlds_tests::native_live_world_ordinary_actions_admit_revalidate_and_update_real_membership",
        "SUNRISE_LIVE_WORLD_ACTION_CHILD",
        &["actions"],
    ) {
        return;
    }
    let policy = Arc::new(AtomicU8::new(1));
    mod_platform::testing::with_mods(
        vec![
            ModEntry::new("live_fixture", Box::new(Fixture(policy.clone()))),
            ModEntry::new(
                "other_fixture",
                Box::new(Fixture(Arc::new(AtomicU8::new(0)))),
            ),
        ]
        .into(),
        || {
            super::native_entity_refs::tests::with_game(|game| {
                let _phase =
                    mod_api::__private::PhaseOwner::scope(mod_api::__private::ModPhase::World);
                let scratch = tempfile::tempdir().unwrap();
                game.world_root = scratch.path().to_path_buf();
                let _owner = owner();
                let source = prepare(game);
                let destination = game.level("minecraft:overworld").unwrap();
                let projection = source
                    .project(
                        game,
                        destination,
                        &[[0, 64, 0], [1, 64, 0], [17, 64, 0]],
                        LiveWorldPose::at([10.0, 0.0, 10.0]),
                        LiveWorldLimits::default(),
                    )
                    .unwrap();
                let physical = sunrise_core::geometry::Vec3::new(27.5, 65.5, 13.0);
                game.player =
                    Some(sunrise_client::motion::PlayerMotion::new(game.motion, physical).unwrap());
                super::native_entity_refs::tests::bind_local_player(game, 0, 123);
                game.lan_acting_name = Some("Fixture".into());
                game.lan_acting_permission = Some(0);
                game.set_mode(sunrise_client::command::args::GameMode::Survival);
                game.selected = 0;
                game.inventory[0] = Some(ItemStack::new("minecraft:stone", 3));
                let player = game.native_acting_player().unwrap();
                let eye = game.eye_position().unwrap();
                let hit = projection
                    .clip(game, eye, [27.5, 64.9, 10.5])
                    .unwrap()
                    .unwrap();
                assert_eq!(hit.face, sunrise_core::geometry::Direction::Up);
                assert_eq!(
                    projection.use_block(game, player, hit, mod_api::Hand::Main),
                    Err(LiveWorldError::Denied)
                );
                assert_eq!(game.inventory[0].as_ref().unwrap().count, 3);
                assert_eq!(projection.cells(game).unwrap().len(), 3);
                policy.store(2, Ordering::Relaxed);
                assert_eq!(
                    projection.use_block(game, player, hit, mod_api::Hand::Main),
                    Err(LiveWorldError::Denied)
                );
                assert_eq!(
                    game.inventory[0].as_ref().unwrap().count,
                    3,
                    "stale listener inventory mutation rolls back in host transaction"
                );
                policy.store(0, Ordering::Relaxed);
                let wall = {
                    let _other = mod_api::content::crash::enter_native_mod(
                        mod_api::content::crash::ModCall {
                            id: "other_fixture",
                            hook: "level_tick_post",
                            part: None,
                        },
                    );
                    let wall_source = prepare_named(game, "other_fixture:local");
                    wall_source
                        .project(
                            game,
                            destination,
                            &[[17, 64, 0]],
                            LiveWorldPose::at([10.0, 2.0, 12.0]),
                            LiveWorldLimits::default(),
                        )
                        .unwrap()
                };
                assert_eq!(
                    projection.use_block(game, player, hit, mod_api::Hand::Main),
                    Err(LiveWorldError::Obstructed),
                    "another owner’s nearer wall occludes the farther cell"
                );
                {
                    let _other = mod_api::content::crash::enter_native_mod(
                        mod_api::content::crash::ModCall {
                            id: "other_fixture",
                            hook: "level_tick_post",
                            part: None,
                        },
                    );
                    wall.retire(game).unwrap();
                }
                assert_eq!(
                    projection.use_block(game, player, hit, mod_api::Hand::Main),
                    Ok(true)
                );
                assert_eq!(game.inventory[0].as_ref().unwrap().count, 2);
                assert!(
                    projection
                        .cells(game)
                        .unwrap()
                        .iter()
                        .any(|c| c.pos == [17, 65, 0])
                );
                let new_hit = projection
                    .clip(game, eye, [27.5, 65.9, 10.5])
                    .unwrap()
                    .unwrap();
                assert_eq!(new_hit.cell, [17, 65, 0]);
                policy.store(4, Ordering::Relaxed);
                assert_eq!(
                    projection.destroy_block(game, player, new_hit, false),
                    Ok(false)
                );
                assert_eq!(
                    game.inventory[0].as_ref().unwrap().count,
                    2,
                    "break callback write rolls back before removal"
                );
                assert!(
                    projection
                        .cells(game)
                        .unwrap()
                        .iter()
                        .any(|c| c.pos == [17, 65, 0])
                );
                policy.store(0, Ordering::Relaxed);
                assert_eq!(
                    projection.destroy_block(game, player, new_hit, false),
                    Ok(true)
                );
                assert!(
                    !projection
                        .cells(game)
                        .unwrap()
                        .iter()
                        .any(|c| c.pos == [17, 65, 0])
                );
                let final_hit = projection
                    .clip(game, eye, [27.5, 64.9, 10.5])
                    .unwrap()
                    .unwrap();
                let before_retirement = projection.sample(game).unwrap();
                let members_before_retirement: Vec<_> = projection
                    .cells(game)
                    .unwrap()
                    .iter()
                    .map(|cell| cell.pos)
                    .collect();
                policy.store(3, Ordering::Relaxed);
                assert_eq!(
                    projection.destroy_block(game, player, final_hit, false),
                    Ok(false)
                );
                let world = game.native_level_world(source).unwrap();
                assert!(
                    !game.res.behaviours().is_air(world.block(17, 64, 0)),
                    "retirement callback cannot remove old target"
                );
                assert_eq!(
                    projection.sample(game),
                    Ok(before_retirement),
                    "refused ordinary transaction restores callback retirement exactly"
                );
                assert_eq!(source.live_world(game).unwrap(), Some(projection));
                assert_eq!(
                    projection
                        .cells(game)
                        .unwrap()
                        .iter()
                        .map(|cell| cell.pos)
                        .collect::<Vec<_>>(),
                    members_before_retirement
                );
                assert_eq!(game.dim, Dim::Overworld);
                assert_eq!(
                    game.player.as_ref().unwrap().position,
                    physical,
                    "source context never teleports physical player"
                );
            });
        },
    );
}

struct SchedulerFixture {
    activate: Arc<AtomicU8>,
    seeds: Arc<AtomicU8>,
    observations: Arc<std::sync::Mutex<Vec<(bool, Option<bool>)>>>,
}
#[mod_api::__subscriptions]
impl Hooks for SchedulerFixture {
    fn server_tick(
        &self,
        host: &mut dyn mod_api::ServerHost,
        event: &mut mod_api::ServerTickEvent<'_>,
    ) {
        if !matches!(
            event.phase,
            mod_api::ServerTickPhase::ServerTickStart {
                world_running: true
            }
        ) || self.activate.load(Ordering::Relaxed) != 1
        {
            return;
        }
        // The source is inactive: its own level callback cannot request its first activation.
        let source = host.level("live_scheduler:local").unwrap();
        assert!(!source.in_place(host));
        let stone = host.block_state_id("minecraft:stone").unwrap();
        let before = source.revisions(host).unwrap();
        assert_eq!(
            source.set_block(host, [2, 64, 0], stone, mod_api::BlockUpdateFlags::CLIENTS),
            Err(mod_api::LevelWriteError::Unavailable)
        );
        assert_eq!(
            mod_api::HostTransaction::transaction(host, |host, _| source.set_block(
                host,
                [2, 64, 0],
                stone,
                mod_api::BlockUpdateFlags::CLIENTS
            )),
            Err(mod_api::LevelWriteError::Unavailable),
            "a transaction never switches a parked Source into place"
        );
        assert_eq!(source.revisions(host).unwrap(), before);
        for chunk in [[0, 0], [1, 0]] {
            assert!(
                source
                    .chunk_ref(host, chunk)
                    .unwrap()
                    .set_forced(host, true)
            );
        }
        self.activate.store(2, Ordering::Relaxed);
    }
    fn level_tick_post(
        &self,
        host: mod_api::HostSide<'_>,
        event: &mut mod_api::LevelTickPostEvent,
    ) {
        let mod_api::HostSide::Server(host) = host else {
            return;
        };
        let source = host.level("live_scheduler:local").unwrap();
        let parent = host.level("minecraft:overworld").unwrap();
        // A source that already ticked must not hand a live BE to a later level in this
        // same ordinary scheduler pass. The global-frontier service refuses before capture.
        assert_eq!(
            source.prepare_transfer(
                host,
                &[mod_api::LiveWorldTransferCell::new(
                    source,
                    [1, 64, 0],
                    parent,
                    [2, 64, 0],
                )],
                mod_api::LiveWorldTransferLimits::default()
            ),
            Err(LiveWorldError::Busy),
        );
        if event.level == source
            && self.activate.load(Ordering::Relaxed) == 2
            && self.seeds.load(Ordering::Relaxed) == 0
        {
            assert!(source.in_place(host));
            assert_eq!(
                host.transaction_lifecycle(),
                mod_api::TransactionLifecycle::None
            );
            self.seeds.store(1, Ordering::Relaxed);
            let flags = mod_api::BlockUpdateFlags::CLIENTS;
            let stone = host.block_state_id("minecraft:stone").unwrap();
            let chest = host.block_state_id("minecraft:chest").unwrap();
            assert_eq!(source.set_block(host, [2, 64, 0], stone, flags), Ok(true));
            assert_eq!(source.set_block(host, [3, 65, 0], chest, flags), Ok(true));
            assert!(
                host.block_entity(source, [3, 65, 0])
                    .unwrap()
                    .merge_data(
                        host,
                        [(
                            "Items".into(),
                            mod_api::Tag::List(vec![mod_api::Tag::compound([
                                ("Slot", mod_api::Tag::Byte(0)),
                                ("id", mod_api::Tag::String("minecraft:stone".into())),
                                ("count", mod_api::Tag::Int(7)),
                            ])])
                        )]
                        .into()
                    )
                    .unwrap()
            );
        }
        if event.level.dimension_id() == "live_scheduler:local"
            || event.level.dimension_id() == "minecraft:overworld"
        {
            self.observations.lock().unwrap().push((
                source.in_place(host),
                source.should_tick_blocks_at(host, [1, 64, 0]),
            ));
        }
    }
}

#[test]
#[ignore = "daily"]
fn runtime_source_global_activation_and_parked_readiness_use_ordinary_scheduler() {
    if super::native_entity_refs::tests::run_fresh_cases(
        "game::native_live_worlds_tests::runtime_source_global_activation_and_parked_readiness_use_ordinary_scheduler",
        "SUNRISE_LIVE_WORLD_SCHEDULER_CHILD",
        &["scheduler"],
    ) {
        return;
    }
    let activate = Arc::new(AtomicU8::new(0));
    let seeds = Arc::new(AtomicU8::new(0));
    let observations = Arc::new(std::sync::Mutex::new(Vec::new()));
    mod_platform::testing::with_mods(
        vec![ModEntry::new(
            "live_scheduler",
            Box::new(SchedulerFixture {
                activate: activate.clone(),
                seeds: seeds.clone(),
                observations: observations.clone(),
            }),
        )]
        .into(),
        || {
            super::native_entity_refs::tests::with_game(|game| {
                let _phase =
                    mod_api::__private::PhaseOwner::scope(mod_api::__private::ModPhase::World);
                let scratch = tempfile::tempdir().unwrap();
                game.world_root = scratch.path().to_path_buf();
                let _owner =
                    mod_api::content::crash::enter_native_mod(mod_api::content::crash::ModCall {
                        id: "live_scheduler",
                        hook: "server_tick",
                        part: None,
                    });
                // The shared fixture stops before world-open completion. Finish the ordinary
                // server lifecycle before exercising tick_server; a merely constructed Game
                // has neither a running server nor enabled lifecycle callbacks.
                game.options.dedicated = Some(Vec::new());
                game.native_lifecycle_world_ready();
                let source = prepare_named(game, "live_scheduler:local");
                for chunk in [[0, 0], [1, 0]] {
                    source
                        .chunk_ref(game, chunk)
                        .unwrap()
                        .set_forced(game, false);
                }
                let parent = game.level("minecraft:overworld").unwrap();
                assert!(
                    parent
                        .chunk_ref(game, [0, 0])
                        .unwrap()
                        .set_forced(game, true)
                );
                assert!(
                    !game
                        .levels_to_tick()
                        .contains(&Dim::from_id("live_scheduler:local"))
                );
                for _ in 0..2 {
                    game.tick_server(false).unwrap();
                }
                assert!(
                    observations
                        .lock()
                        .unwrap()
                        .iter()
                        .all(|(in_place, _)| !in_place)
                );
                let furnace = |game: &Game<'_>| {
                    let world = game.native_level_world(source).unwrap();
                    let Some(BlockEntity::Furnace(value)) = world.block_entities.get(&(1, 64, 0))
                    else {
                        panic!("actual source furnace disappeared")
                    };
                    value.cooking_timer
                };
                assert_eq!(
                    furnace(game),
                    40,
                    "an inactive source received no block tick"
                );
                observations.lock().unwrap().clear();
                activate.store(1, Ordering::Relaxed);
                let blocks_before_seed = source.revisions(game).unwrap().block_revision;
                for _ in 0..4 {
                    game.tick_server(false).unwrap();
                }
                assert_eq!(activate.load(Ordering::Relaxed), 2);
                assert_eq!(
                    seeds.load(Ordering::Relaxed),
                    1,
                    "repeated ordinary Source callbacks do not seed again"
                );
                assert_eq!(
                    source.revisions(game).unwrap().block_revision,
                    blocks_before_seed + 2,
                    "only the once-admitted stone and chest state writes occurred"
                );
                let source_world = game.native_level_world(source).unwrap();
                let Some(BlockEntity::Chest(chest)) = source_world.block_entities.get(&(3, 65, 0))
                else {
                    panic!("real seeded chest absent")
                };
                assert_eq!(chest[0].as_ref().unwrap().count, 7);
                let seen = observations.lock().unwrap();
                assert_eq!(
                    seen.iter().filter(|(in_place, _)| *in_place).count(),
                    4,
                    "one real source callback per ordinary running tick"
                );
                assert!(
                    seen.iter()
                        .any(|(in_place, ready)| !in_place && *ready == Some(true)),
                    "parent callback can read the parked source's own readiness"
                );
                assert!(
                    seen.iter()
                        .filter(|(in_place, _)| *in_place)
                        .all(|(_, ready)| *ready == Some(true))
                );
                drop(seen);
                assert_eq!(
                    furnace(game),
                    44,
                    "actual furnace ticks once per source scheduler turn"
                );
                let before = observations.lock().unwrap().len();
                game.tick_server(true).unwrap();
                assert_eq!(observations.lock().unwrap().len(), before);
                assert_eq!(
                    furnace(game),
                    44,
                    "ordinary server pause holds its live ticker"
                );
            });
        },
    );
}

static PUBLICATION: mod_api::AttachmentType<mod_api::Compound> = mod_api::AttachmentType {
    namespace: "pose_fixture",
    name: "publication",
    default: mod_api::Compound::new,
    codec: Some(mod_api::AttachmentCodec {
        encode: |value| Ok(value.clone()),
        decode: |value| Ok(value.clone()),
    }),
    copy_on_death: false,
    sync: false,
};
static BAD_PUBLICATION: mod_api::AttachmentType<mod_api::Compound> = mod_api::AttachmentType {
    namespace: "pose_fixture",
    name: "publication",
    default: mod_api::Compound::new,
    codec: Some(mod_api::AttachmentCodec {
        encode: |_| Err(mod_api::AttachmentError::Encode),
        decode: |value| Ok(value.clone()),
    }),
    copy_on_death: false,
    sync: false,
};
struct PublicationFixture;
#[mod_api::__subscriptions]
impl Hooks for PublicationFixture {
    fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
        r.register_attachment_type("pose_fixture:publication", false)
            .unwrap();
    }
}
fn publication(sample: mod_api::LiveWorldSample) -> mod_api::Compound {
    let current = sample
        .current
        .translation
        .into_iter()
        .chain(sample.current.pivot)
        .chain(sample.current.rotation)
        .map(|value| value.to_bits() as i64)
        .collect();
    [
        (
            "revision".into(),
            mod_api::Tag::Long(sample.revision as i64),
        ),
        ("current".into(), mod_api::Tag::LongArray(current)),
    ]
    .into()
}

#[test]
#[ignore = "daily"]
fn publication_and_owned_parked_metadata_abort_together_and_allow_same_tick_retry() {
    use mod_api::{AttachmentError, Holder, HostTransaction as _, ServerHost};
    if super::native_entity_refs::tests::run_fresh_cases(
        "game::native_live_worlds_tests::publication_and_owned_parked_metadata_abort_together_and_allow_same_tick_retry",
        "SUNRISE_LIVE_WORLD_PUBLICATION_CHILD",
        &["publication"],
    ) {
        return;
    }
    mod_platform::testing::with_mods(
        vec![
            ModEntry::new("pose_fixture", Box::new(PublicationFixture)),
            ModEntry::new(
                "foreign_fixture",
                Box::new(Fixture(Arc::new(AtomicU8::new(0)))),
            ),
        ]
        .into(),
        || {
            super::native_entity_refs::tests::with_game(|game| {
                let _phase =
                    mod_api::__private::PhaseOwner::scope(mod_api::__private::ModPhase::World);
                let scratch = tempfile::tempdir().unwrap();
                game.world_root = scratch.path().to_path_buf();
                let _owner =
                    mod_api::content::crash::enter_native_mod(mod_api::content::crash::ModCall {
                        id: "pose_fixture",
                        hook: "server_tick",
                        part: None,
                    });
                let source = prepare_named(game, "pose_fixture:local");
                let destination = game.level("minecraft:overworld").unwrap();
                let parent_revisions = destination.revisions(game).unwrap();
                assert_eq!(parent_revisions.block_revision, game.world.revision);
                let before_contents = source.revisions(game).unwrap();
                assert!(before_contents.block_revision > 0 && before_contents.content_revision > 0);
                {
                    let world = &mut game
                        .levels
                        .parked_mut_of(source.dimension_id())
                        .unwrap()
                        .world;
                    let Some(BlockEntity::Chest(items)) = world.block_entities.get_mut(&(0, 64, 0))
                    else {
                        panic!("actual source chest missing");
                    };
                    items[0].as_mut().unwrap().count += 1;
                }
                let changed_contents = source.revisions(game).unwrap();
                assert_eq!(
                    changed_contents.block_revision,
                    before_contents.block_revision
                );
                assert!(changed_contents.content_revision > before_contents.content_revision);
                {
                    let world = &mut game
                        .levels
                        .parked_mut_of(source.dimension_id())
                        .unwrap()
                        .world;
                    let Some(BlockEntity::Chest(items)) = world.block_entities.get_mut(&(0, 64, 0))
                    else {
                        unreachable!();
                    };
                    items[0].as_mut().unwrap().count -= 1;
                }
                let empty = game
                    .runtime_level_open(
                        "pose_fixture:empty",
                        RuntimeLevel::new("minecraft:overworld", RuntimeGenerator::Void, 1022),
                    )
                    .unwrap();
                game.apply_runtime_levels();
                let actual_empty = game.native_level_world(empty).unwrap();
                assert!(actual_empty.chunks.is_empty());
                assert_eq!(
                    empty.revisions(game).unwrap(),
                    mod_api::LevelRevisions::new(
                        actual_empty.revision,
                        actual_empty.block_entities.content_revision()
                    )
                );
                let stale_source = mod_api::LevelRef::with_generation(
                    source.epoch(),
                    source.dimension(),
                    source.generation() + 1,
                );
                assert_eq!(
                    stale_source.revisions(game),
                    Err(mod_api::LevelReadError::Stale)
                );
                let mut initial = LiveWorldPose::at([10.0, 16.0, 10.0]);
                initial.pivot = [2.0, 64.0, 2.0];
                let projection = source
                    .project(
                        game,
                        destination,
                        &[[0, 64, 0], [1, 64, 0], [17, 64, 0]],
                        initial,
                        LiveWorldLimits::default(),
                    )
                    .unwrap();
                let before = projection.sample(game).unwrap();
                let before_admission = game.capture_projection_pose(projection).unwrap();
                let old = publication(before);
                PUBLICATION
                    .set(&mut source.attachments(game), Holder::Level, old.clone())
                    .unwrap();
                assert!(!source.in_place(game));
                assert_eq!(
                    PUBLICATION.has(game, Holder::Level),
                    Ok(false),
                    "source write never addresses parent Holder::Level"
                );
                let mut requested = initial;
                requested.translation[0] += 0.125;
                requested.rotation = [0.0, 0.008726535498373935, 0.0, 0.9999619230641713];
                let host: &mut dyn ServerHost = game;
                let canceled = host.transaction(|host, _| {
                    let accepted = projection
                        .publish_pose(host, before.revision, requested)
                        .unwrap();
                    let metadata = publication(accepted);
                    PUBLICATION.set(
                        &mut source.attachments(host),
                        Holder::Level,
                        metadata.clone(),
                    )?;
                    assert_eq!(
                        PUBLICATION.get_existing(&mut source.attachments(host), Holder::Level)?,
                        Some(metadata)
                    );
                    assert!(!sunrise_client::join_seq::visibility::passes_settled());
                    BAD_PUBLICATION.set(
                        &mut source.attachments(host),
                        Holder::Level,
                        publication(accepted),
                    )
                });
                assert_eq!(canceled, Err(AttachmentError::Encode));
                assert_eq!(projection.sample(game).unwrap(), before);
                assert_eq!(
                    game.capture_projection_pose(projection).unwrap(),
                    before_admission,
                    "abort restores sample and per-tick admission exactly"
                );
                assert_eq!(
                    PUBLICATION
                        .get_existing(&mut source.attachments(game), Holder::Level)
                        .unwrap(),
                    Some(old.clone())
                );
                assert!(sunrise_client::join_seq::visibility::passes_settled());
                let host: &mut dyn ServerHost = game;
                let nested = host.transaction(|host, _| {
                    host.transaction(|host, _| {
                        let accepted = projection
                            .publish_pose(host, before.revision, requested)
                            .unwrap();
                        PUBLICATION.set(
                            &mut source.attachments(host),
                            Holder::Level,
                            publication(accepted),
                        )
                    })?;
                    Err::<(), _>(AttachmentError::Encode)
                });
                assert_eq!(nested, Err(AttachmentError::Encode));
                assert_eq!(
                    game.capture_projection_pose(projection).unwrap(),
                    before_admission,
                    "nested commit retains root's oldest before-image"
                );
                assert_eq!(
                    PUBLICATION
                        .get_existing(&mut source.attachments(game), Holder::Level)
                        .unwrap(),
                    Some(old)
                );
                let host: &mut dyn ServerHost = game;
                let accepted = host
                    .transaction(|host, _| {
                        let sample = projection
                            .publish_pose(host, before.revision, requested)
                            .unwrap();
                        PUBLICATION.set(
                            &mut source.attachments(host),
                            Holder::Level,
                            publication(sample),
                        )?;
                        Ok::<_, AttachmentError>(sample)
                    })
                    .unwrap();
                assert_eq!(
                    accepted.revision,
                    before.revision + 1,
                    "refused prior groups consume no tick/revision"
                );
                assert_eq!(projection.sample(game).unwrap(), accepted);
                assert_eq!(
                    PUBLICATION
                        .get_existing(&mut source.attachments(game), Holder::Level)
                        .unwrap(),
                    Some(publication(accepted))
                );
                {
                    let _foreign = mod_api::content::crash::enter_native_mod(
                        mod_api::content::crash::ModCall {
                            id: "foreign_fixture",
                            hook: "server_tick",
                            part: None,
                        },
                    );
                    assert_eq!(
                        source.revisions(game),
                        Err(mod_api::LevelReadError::Unavailable)
                    );
                    assert_eq!(
                        empty.revisions(game),
                        Err(mod_api::LevelReadError::Unavailable)
                    );
                    assert_eq!(
                        PUBLICATION.set(
                            &mut source.attachments(game),
                            Holder::Level,
                            mod_api::Compound::new()
                        ),
                        Err(AttachmentError::MissingHolder)
                    );
                }
                assert_eq!(
                    PUBLICATION
                        .get_existing(&mut source.attachments(game), Holder::Level)
                        .unwrap(),
                    Some(publication(accepted))
                );
            });
        },
    );
}

#[path = "native_live_worlds/checkpoint_adverse_tests.rs"]
mod checkpoint_adverse_tests;
