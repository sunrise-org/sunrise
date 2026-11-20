//! Observational bounded real-Game account; no unmatched-profile before/after claim.
use super::*;
use std::time::Instant;
struct Fixture;
#[mod_api::__subscriptions]
impl mod_api::Hooks for Fixture {}

#[test]
#[ignore = "daily"]
fn actual_game_empty_stable_and_inventory_only_projection_account() {
    if super::super::native_entity_refs::tests::run_fresh_cases(
        "game::native_live_worlds::perf_tests::actual_game_empty_stable_and_inventory_only_projection_account",
        "SUNRISE_LIVE_WORLD_ACCOUNT_CHILD",
        &["account"],
    ) {
        return;
    }
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
                let scratch = tempfile::tempdir().unwrap();
                game.world_root = scratch.path().to_path_buf();
                let source = super::super::native_live_worlds_tests::prepare_named(
                    game,
                    "live_fixture:account",
                );
                game.native_lifecycle_world_ready();
                const N: usize = 20;
                let start = Instant::now();
                for _ in 0..N {
                    game.tick_server(true).unwrap();
                }
                let empty = start.elapsed();
                assert_eq!(game.live_worlds.borrow().binding_count(), 0);
                let parent = game.level("minecraft:overworld").unwrap();
                let binding = source
                    .project(
                        game,
                        parent,
                        &[[0, 64, 0], [1, 64, 0], [17, 64, 0]],
                        LiveWorldPose::at([10.0, 0.0, 10.0]),
                        LiveWorldLimits::default(),
                    )
                    .unwrap();
                let geometry = {
                    let entry = game.projection_entry(binding).unwrap();
                    game.projection_geometry(&entry).unwrap()
                };
                let start = Instant::now();
                for _ in 0..N {
                    game.tick_server(true).unwrap();
                    let entry = game.projection_entry(binding).unwrap();
                    assert!(Rc::ptr_eq(
                        &geometry,
                        &game.projection_geometry(&entry).unwrap()
                    ));
                }
                let stable = start.elapsed();
                let before = source.revisions(game).unwrap();
                let start = Instant::now();
                for index in 0..N {
                    let world = if game.dim.key() == source.dimension() {
                        &mut game.world
                    } else {
                        &mut game
                            .levels
                            .parked_mut_of(source.dimension_id())
                            .unwrap()
                            .world
                    };
                    let Some(sunrise_client::containers::BlockEntity::Chest(items)) =
                        world.block_entities.get_mut(&(0, 64, 0))
                    else {
                        panic!("actual chest absent");
                    };
                    items[0].as_mut().unwrap().count = 7 + (index % 2) as u32;
                    game.tick_server(true).unwrap();
                    let entry = game.projection_entry(binding).unwrap();
                    assert!(Rc::ptr_eq(
                        &geometry,
                        &game.projection_geometry(&entry).unwrap()
                    ));
                }
                let inventory = start.elapsed();
                let after = source.revisions(game).unwrap();
                assert_eq!(before.block_revision, after.block_revision);
                assert!(after.content_revision > before.content_revision);
                eprintln!(
                    "LIVE_WORLD_ACCOUNT {}",
                    serde_json::json!({"arch":std::env::consts::ARCH,"os":std::env::consts::OS,"profile":"fast with recorded build overrides","paused_server_ticks_per_case":N,"empty_ns":empty.as_nanos(),"stable_ns":stable.as_nanos(),"inventory_only_ns":inventory.as_nanos(),"same_geometry_all_cases":true,"causal_before_after":false})
                );
                source.checkpoint(game).unwrap();
            });
        },
    );
}
