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
