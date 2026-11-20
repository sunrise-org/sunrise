//! Real Game admission for externally resolved poses; no external dynamics or clamping.
use super::*;
struct Fixture;
#[mod_api::__subscriptions]
impl mod_api::Hooks for Fixture {}
#[test]
#[ignore = "daily"]
fn trusted_external_pose_consumes_overlap_while_default_identity_and_step_guards_hold() {
    const NAME: &str = "game::native_live_worlds::publication_tests::trusted_external_pose_consumes_overlap_while_default_identity_and_step_guards_hold";
    if super::super::native_entity_refs::tests::run_fresh_cases(
        NAME,
        "SUNRISE_LIVE_POLICY_CHILD",
        &["policy"],
    ) {
        return;
    }
    mod_platform::testing::with_mods(
        vec![mod_api::ModEntry::new("live_fixture", Box::new(Fixture))].into(),
        || {
            super::super::native_entity_refs::tests::with_game(|game| {
                let scratch = tempfile::tempdir().unwrap();
                game.world_root = scratch.path().to_path_buf();
                game.session_lock =
                    Some(sunrise_client::save::SessionLock::acquire(scratch.path()).unwrap());
                game.save = Arc::new(sunrise_client::save::SaveDir::new(
                    sunrise_client::save::dimension_dir(scratch.path(), game.dim.id()),
                    game.world.shape,
                    Some(Default::default()),
                ));
                game.options.dedicated = Some(Vec::new());
                game.native_lifecycle_world_ready();
                let _phase =
                    mod_api::__private::PhaseOwner::scope(mod_api::__private::ModPhase::World);
                let _owner =
                    mod_api::content::crash::enter_native_mod(mod_api::content::crash::ModCall {
                        id: "live_fixture",
                        hook: "server_tick",
                        part: None,
                    });
                let source = super::super::native_live_worlds_tests::prepare_named(
                    game,
                    "live_fixture:publication",
                );
                let parent = game.level("minecraft:overworld").unwrap();
                let stone = game
                    .pack
                    .assets
                    .registry
                    .default_state("minecraft:stone")
                    .unwrap();
                game.world.set(17, 64, 0, stone).unwrap();
                assert_eq!(game.runtime_caller(), Ok("live_fixture"));
                assert!(mod_api::__private::access_allowed(
                    "level_project",
                    mod_api::__private::LoadAccess::WorldOnly
                ));
                let cells = &[[17, 64, 0]];
                let pose = LiveWorldPose::at([0.0; 3]);
                assert_eq!(
                    source.project(game, parent, cells, pose, LiveWorldLimits::default()),
                    Err(Error::Obstructed)
                );
                assert_eq!(source.live_world(game).unwrap(), None);
                let trusted = LiveWorldLimits {
                    publication: mod_api::LiveWorldPublication::ExternallyResolved,
                    ..Default::default()
                };
                assert_eq!(
                    source.project(game, parent, cells, pose, trusted),
                    Err(Error::Denied)
                );
                game.read_live_world_quotas(|key| {
                    (key == "sunrise-live-world-external-pose-owners")
                        .then(|| "live_fixture".to_owned())
                });
                let projection = source.project(game, parent, cells, pose, trusted).unwrap();
                let before = projection.sample(game).unwrap();
                let too_far = LiveWorldPose::at([trusted.translation + 1.0, 0.0, 0.0]);
                assert_eq!(
                    projection.publish_pose(game, before.revision, too_far),
                    Err(Error::Invalid)
                );
                assert_eq!(projection.sample(game).unwrap(), before);
                let mut invalid = pose;
                invalid.translation[0] = f64::NAN;
                assert_eq!(
                    projection.publish_pose(game, before.revision, invalid),
                    Err(Error::Invalid)
                );
                assert_eq!(projection.sample(game).unwrap(), before);
                let angle = 0.5_f64.to_radians() / 2.0;
                let request = LiveWorldPose {
                    translation: [0.1, -0.001, 0.0],
                    pivot: [0.0; 3],
                    rotation: [0.0, angle.sin(), 0.0, angle.cos()],
                };
                let accepted = projection
                    .publish_pose(game, before.revision, request)
                    .unwrap();
                assert_eq!(
                    accepted.current.translation.map(f64::to_bits),
                    request.translation.map(f64::to_bits),
                    "authoritative external contact overlap is consumed without a correction"
                );
                assert_eq!(accepted.previous, before.current);
                assert_eq!(accepted.revision, before.revision + 1);
                assert_eq!(
                    projection.publish_pose(game, accepted.revision, request),
                    Err(Error::Revision),
                    "once per global tick remains authoritative"
                );
                game.read_live_world_quotas(|_| None);
                assert_eq!(
                    projection.publish_pose(game, accepted.revision, request),
                    Err(Error::Denied)
                );
                assert_eq!(projection.sample(game).unwrap(), accepted);
            });
        },
    );
}

#[test]
#[ignore = "daily"]
fn ordinary_destination_shapes_use_real_position_context_without_weakening_source_admission() {
    const NAME: &str = "game::native_live_worlds::publication_tests::ordinary_destination_shapes_use_real_position_context_without_weakening_source_admission";
    const FLAG: &str = "SUNRISE_DESTINATION_SHAPE_CHILD";
    if super::super::native_entity_refs::tests::run_fresh_cases(
        NAME,
        FLAG,
        &["water", "positioned", "source-refusal", "residency-budget"],
    ) {
        return;
    }
    let case = std::env::var(FLAG).unwrap();
    mod_platform::testing::with_mods(
        vec![mod_api::ModEntry::new("live_fixture", Box::new(Fixture))].into(),
        || {
            super::super::native_entity_refs::tests::with_game(|game| {
                let scratch = tempfile::tempdir().unwrap();
                game.world_root = scratch.path().to_path_buf();
                game.session_lock =
                    Some(sunrise_client::save::SessionLock::acquire(scratch.path()).unwrap());
                game.save = Arc::new(sunrise_client::save::SaveDir::new(
                    sunrise_client::save::dimension_dir(scratch.path(), game.dim.id()),
                    game.world.shape,
                    Some(Default::default()),
                ));
                game.options.dedicated = Some(Vec::new());
                game.native_lifecycle_world_ready();
                let _phase =
                    mod_api::__private::PhaseOwner::scope(mod_api::__private::ModPhase::World);
                let _owner =
                    mod_api::content::crash::enter_native_mod(mod_api::content::crash::ModCall {
                        id: "live_fixture",
                        hook: "server_tick",
                        part: None,
                    });
                let source = super::super::native_live_worlds_tests::prepare_named(
                    game,
                    "live_fixture:destination",
                );
                let parent = game.level("minecraft:overworld").unwrap();
                let registry = game.pack.assets.registry;
                let stone = registry.default_state("minecraft:stone").unwrap();
                let water = registry.default_state("minecraft:water").unwrap();
                let cells = &[[17, 64, 0]];
                let pose = LiveWorldPose::at([10., 0., 10.]);
                if case == "source-refusal" {
                    game.enter_level(super::super::Dim::from_id(source.dimension_id()))
                        .unwrap();
                    game.world.set(17, 64, 0, water).unwrap();
                    game.enter_level(super::super::Dim::Overworld).unwrap();
                    assert_eq!(
                        source.project(game, parent, cells, pose, LiveWorldLimits::default()),
                        Err(Error::Unsupported)
                    );
                    assert_eq!(source.live_world(game).unwrap(), None);
                    return;
                }
                if case == "positioned" {
                    // A real moving-piston entity makes the same canonical block state
                    // collide at one position and remain empty where no entity exists.
                    // The source whitelist still refuses that dynamic state.
                    let moving = registry.default_state("minecraft:moving_piston").unwrap();
                    assert!(matches!(state_shape(game, moving), Err(Error::Unsupported)));
                    for x in [27, 29] {
                        game.world.set(x, 64, 10, moving).unwrap();
                    }
                    game.block_ticks.moving_pistons.push((
                        BlockPos::new(27, 64, 10),
                        sunrise_world::level::pistons::MovingPiston {
                            moved_state: stone,
                            direction: sunrise_core::geometry::Direction::East,
                            extending: true,
                            source: false,
                            progress: 0.5,
                            progress_o: 0.5,
                            last_ticked: game.world.game_time,
                            death_ticks: 0,
                            id: 1,
                        },
                    ));
                    game.sync_moving_pistons();
                    assert!(
                        !destination_shape(game, &game.world, [27, 64, 10])
                            .unwrap()
                            .is_empty()
                    );
                    assert!(
                        destination_shape(game, &game.world, [29, 64, 10])
                            .unwrap()
                            .is_empty()
                    );
                    assert_eq!(
                        source.project(game, parent, cells, pose, LiveWorldLimits::default()),
                        Err(Error::Obstructed)
                    );
                    assert_eq!(source.live_world(game).unwrap(), None);
                    return;
                }
                if case == "residency-budget" {
                    let limits = LiveWorldLimits {
                        candidates: 1,
                        ..Default::default()
                    };
                    assert_eq!(
                        source.project(game, parent, cells, pose, limits),
                        Err(Error::Budget)
                    );
                    assert_eq!(source.live_world(game).unwrap(), None);
                } else {
                    assert!(game.collision.has_context_rule(water));
                    game.world.set(27, 64, 10, water).unwrap();
                }
                let projection = source
                    .project(game, parent, cells, pose, LiveWorldLimits::default())
                    .unwrap();
                let before = projection.sample(game).unwrap();
                let mut request = pose;
                request.translation[0] += 0.125;
                if case == "residency-budget" {
                    let actual = game.world.chunks.remove(&(1, 0)).unwrap();
                    assert_eq!(
                        projection.publish_pose(game, before.revision, request),
                        Err(Error::NotResident)
                    );
                    assert_eq!(projection.sample(game).unwrap(), before);
                    game.world.chunks.insert((1, 0), actual);
                } else {
                    game.world.set(27, 64, 10, stone).unwrap();
                    assert_eq!(
                        projection.publish_pose(game, before.revision, request),
                        Err(Error::Obstructed)
                    );
                    assert_eq!(projection.sample(game).unwrap(), before);
                    game.world.set(27, 64, 10, water).unwrap();
                }
                let accepted = projection
                    .publish_pose(game, before.revision, request)
                    .unwrap();
                assert_eq!(accepted.current.translation, request.translation);
                assert_eq!(accepted.revision, before.revision + 1);
            });
        },
    );
}
