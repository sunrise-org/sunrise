//! Runtime-backed projection acceptance evidence; no synthetic block or inventory implementation.
use super::{Dim, Game, api_values::TryIntoApi as _};
use mod_api::{
    Hooks, LiveWorldError, LiveWorldLimits, LiveWorldPose, ModEntry, RuntimeGenerator, RuntimeLevel,
};
use std::{collections::HashMap, sync::Arc};
use sunrise_client::{
    chunk::Chunk,
    containers::{BlockEntity, Furnace, FurnaceKind},
    items::ItemStack,
};

struct Fixture;
#[mod_api::__subscriptions]
impl Hooks for Fixture {}

fn owner() -> mod_api::content::crash::ModGuard {
    mod_api::content::crash::enter_native_mod(mod_api::content::crash::ModCall {
        id: "live_fixture",
        hook: "level_tick_post",
        part: None,
    })
}

fn prepare(game: &mut Game<'_>) -> mod_api::LevelRef {
    game.options.dedicated = Some(Vec::new());
    let level = game
        .runtime_level_open(
            "live_fixture:local",
            RuntimeLevel::new("minecraft:overworld", RuntimeGenerator::Void, 1022),
        )
        .unwrap();
    game.apply_runtime_levels();
    game.enter_level(Dim::from_id("live_fixture:local"))
        .unwrap();
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
    mod_platform::testing::with_mods(
        vec![ModEntry::new("live_fixture", Box::new(Fixture))].into(),
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
                game.enter_level(Dim::Overworld).unwrap();
                let restored = reopened.restore_projection(game).unwrap();
                assert_ne!(restored, projection);
                assert_eq!(
                    restored.sample(game).unwrap().current.translation,
                    [11.0, 0.0, 10.0]
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
