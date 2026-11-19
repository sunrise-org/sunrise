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
fn prepare_named(game: &mut Game<'_>, id: &str) -> mod_api::LevelRef {
    game.options.dedicated = Some(Vec::new());
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
                let _owner = owner();
                let source = prepare(game);
                let destination = game.level("minecraft:overworld").unwrap();
                assert_eq!(
                    source.restore_projection(game),
                    Err(LiveWorldError::NotSaved)
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
                let checkpoint = projection.checkpoint(game).unwrap();
                assert_eq!(checkpoint.source, "live_fixture:local");
                // Drop the in-memory projection, then actually close and reopen the real runtime level.
                game.live_worlds.get_mut().clear();
                assert_eq!(projection.sample(game), Err(LiveWorldError::Stale));
                game.runtime_level_unload("live_fixture:local").unwrap();
                game.apply_runtime_levels();
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
                let restored = reopened.restore_projection(game).unwrap();
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
                restored.retire(game).unwrap();
                assert_eq!(restored.sample(game), Err(LiveWorldError::Stale));
                eprintln!(
                    "LIVE_WORLD_EVIDENCE {}",
                    serde_json::json!({"source":checkpoint.source,
                "generation":checkpoint.generation,"pose_revision":checkpoint.pose_revision,"source_revision":checkpoint.source_revision,
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
                assert_eq!(projection.sample(game), Err(LiveWorldError::Stale));
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
