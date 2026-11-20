//! Real client/player contact continuity over a slowly tilted, genuine viewed-block deck.
//! This tests the receiver/contact side; it does not substitute for a public server publisher test.
use super::*;
use sunrise_core::geometry::oriented::Frame;

fn pose(angle: f64) -> wire::Pose {
    axis_pose(Axis::Z, angle)
}

fn axis_pose(axis: Axis, angle: f64) -> wire::Pose {
    let mut public = mod_api::LiveWorldPose::at([4.0, 32.0, 4.0]);
    public.pivot = [0.0, 64.0, 0.0];
    public.rotation = [0.0, 0.0, 0.0, (angle * 0.5).cos()];
    public.rotation[match axis {
        Axis::X => 0,
        Axis::Y => 1,
        Axis::Z => 2,
    }] = (angle * 0.5).sin();
    wire::Pose {
        translation: public.translation,
        pivot: public.pivot,
        rotation: public.rotation,
    }
}

fn trace_contact(game: &mut Game<'_>, phase: &str, packets: &[S]) -> bool {
    let prepared = game.placed_moving_colliders();
    let player = game.player.as_ref().unwrap();
    let placed = prepared.placed().find(|p| p.id == COLLIDER).unwrap();
    let embedded = placed.collider.intersects(player.bounds());
    let downward = placed.collider.collide(player.bounds(), Axis::Y, -0.01);
    let hint = game.remote.as_ref().unwrap().carry_age.hint();
    eprintln!(
        "LIVE_WORLD_TILT phase={phase} position={:?} velocity={:?} ground={} embedded={} downward={} hint={hint:?} reported={:?}",
        player.position,
        player.velocity,
        player.ground.on_ground,
        embedded,
        downward,
        reported_position(packets).map(|(_, p)| p)
    );
    embedded
}

struct DeckSample {
    source_revision: u64,
    hash: [u8; 32],
    frame: u64,
    current: wire::Pose,
}
impl DeckSample {
    fn publish(&mut self, game: &mut Game<'_>, current: wire::Pose) {
        self.frame += 1;
        incoming(
            game,
            wire::Message::Pose {
                id: ID,
                sample: wire::Sample {
                    frame: self.frame,
                    source_revision: self.source_revision,
                    state_hash: self.hash,
                    previous: self.current,
                    current,
                    revision: self.frame,
                },
            },
        );
        self.current = current;
    }
}
fn with_deck(
    mods: mod_api::Mods,
    run_case: impl FnOnce(&mut Game<'_>, &mut ConnReader, &mut DeckSample),
) {
    with_deck_pose(mods, pose(0.0), run_case);
}
fn with_deck_pose(
    mods: mod_api::Mods,
    initial_pose: wire::Pose,
    run_case: impl FnOnce(&mut Game<'_>, &mut ConnReader, &mut DeckSample),
) {
    mod_platform::testing::with_mods(mods, || {
        with_client_game(|game| {
            let (mut reader, _writer) = crate::game::remote::tests::connect_menu_test(game);
            let mut registries = sunrise_protocol::registry::Dynamic::default();
            registries.registries.insert(
                "minecraft:dimension_type".into(),
                vec![("minecraft:overworld".into(), None)],
            );
            game.set_registries(Arc::new(registries));
            game.handle_packet(C::Login(play::Login {
                player_id: 7,
                hardcore: false,
                levels: vec!["minecraft:overworld".into(), SOURCE.into()],
                max_players: 20,
                chunk_radius: 8,
                simulation_distance: 8,
                reduced_debug_info: false,
                show_death_screen: true,
                limited_crafting: false,
                spawn: spawn("minecraft:overworld"),
                online_mode: false,
                enforces_secure_chat: false,
            }))
            .unwrap();
            game.handle_packet(layered(0, 0, 0)).unwrap();
            game.handle_packet(open(0, SOURCE)).unwrap();
            assert!(game.native_client_level().is_some());
            assert!(game.level(SOURCE).is_some());
            let stone = game
                .pack
                .assets
                .registry
                .default_state("minecraft:stone")
                .unwrap();
            let cells: Vec<_> = (0..5)
                .flat_map(|x| (0..5).map(move |z| [x, 64, z]))
                .collect();
            let mut blocks = vec![layered(0, 0, 0)];
            blocks.extend(cells.iter().map(|&pos| {
                C::BlockUpdate(play::BlockUpdate {
                    pos,
                    state: stone.0 as i32,
                })
            }));
            game.handle_packet(run(0, &blocks)).unwrap();
            let source = game.level(SOURCE).unwrap();
            let source_world = game.native_level_view_world(source).unwrap();
            let source_revision = source_world.revision;
            let hash = state_hash(source_world, &cells);

            incoming(
                game,
                wire::Message::Snapshot {
                    id: ID,
                    collider: COLLIDER,
                    boxes: cells.len() as u32,
                    source: SOURCE.into(),
                    destination: "minecraft:overworld".into(),
                    cells,
                    membership: 1,
                    sample: wire::Sample {
                        frame: 1,
                        source_revision,
                        state_hash: hash,
                        previous: initial_pose,
                        current: initial_pose,
                        revision: 1,
                    },
                },
            );
            game.title = None;
            game.screen = Screen::Playing;
            let remote = game.remote.as_mut().unwrap();
            remote.loaded_sent = true;
            remote.level_load.finish();
            // Positive X side of the pivot, well inside the deck, away from its edges.
            let initial = vector([6.5, 97.0, 6.5]);
            let mut player = PlayerMotion::new(game.motion, initial).unwrap();
            player.entity_id = 7;
            player.client_side = true;
            player.ground.on_ground = true;
            game.player = Some(player);
            game.previous_position = initial;
            for settling in 0..8 {
                let packets = tick(game, &mut reader);
                trace_contact(game, "initial-flat-idle", &packets);
                let player = game.player.as_ref().unwrap();
                // A fresh zero-velocity player first accumulates gravity without a downward
                // attempted move. The next ordinary tick obtains real collision grounding.
                if settling != 0 {
                    assert!(player.ground.on_ground);
                }
                assert_eq!(
                    player.position, initial,
                    "the resting fixture must first be stable"
                );
            }

            let mut sample = DeckSample {
                source_revision,
                hash,
                frame: 1,
                current: initial_pose,
            };
            run_case(game, &mut reader, &mut sample);
        });
    });
}

#[test]
#[ignore = "daily"]
fn ordinary_client_keeps_support_across_slow_z_axis_pose_steps_and_idle_ticks() {
    with_deck(Vec::new().into(), |game, reader, sample| {
        // Both signs and horizontal axes; each pose is followed by eight ordinary ticks
        // without publication. Reversing and flattening must not replay a previous carry.
        let steps = [
            (Axis::Z, 0.01),
            (Axis::Z, 0.02),
            (Axis::Z, 0.03),
            (Axis::Z, 0.02),
            (Axis::Z, 0.01),
            (Axis::Z, 0.0),
            (Axis::Z, -0.01),
            (Axis::Z, -0.02),
            (Axis::Z, -0.03),
            (Axis::Z, -0.02),
            (Axis::Z, -0.01),
            (Axis::Z, 0.0),
            (Axis::X, -0.01),
            (Axis::X, -0.02),
            (Axis::X, -0.03),
            (Axis::X, -0.02),
            (Axis::X, -0.01),
            (Axis::X, 0.0),
            (Axis::X, 0.01),
            (Axis::X, 0.02),
            (Axis::X, 0.03),
        ];
        for (step, (axis, angle)) in steps.into_iter().enumerate() {
            let before = game.player.as_ref().unwrap().position;
            sample.publish(game, axis_pose(axis, angle));
            let packets = tick(game, reader);
            assert!(
                !trace_contact(game, &format!("step-{step}-carried"), &packets),
                "a carried upright body must clear the actual support"
            );
            let (carried, velocity_y, carried_bounds) = {
                let player = game.player.as_ref().unwrap();
                (player.position, player.velocity.y, player.bounds())
            };
            let current = game.placed_moving_colliders();
            let gap = -current
                .placed()
                .find(|p| p.id == COLLIDER)
                .unwrap()
                .collider
                .collide(carried_bounds, Axis::Y, -0.01);
            // Source5's actual ordinary no-projection floor control records [false,true×7]
            // with zero initial Y velocity and a tiny gap. The existing Entity.move guard
            // suppresses applying a clipped displacement with squared length <=1e-7; this
            // is a test reference to that unchanged ordinary rule, not a collision tolerance.
            let ordinary_accumulation =
                carried.y < before.y && velocity_y == 0.0 && gap > 0.0 && gap * gap <= 1.0e-7;
            eprintln!(
                "LIVE_WORLD_TILT_REFERENCE step={step} velocity_y={velocity_y} gap={gap} ordinary_zero_y_tiny_gap={ordinary_accumulation}"
            );
            if step < 3 {
                assert!(
                    carried.y > before.y,
                    "the first positive-X contacts must rise"
                );
            }
            assert!(
                reported_position(&packets).is_some(),
                "ordinary move report includes carry"
            );
            let mut observations = Vec::new();
            for idle in 0..8 {
                let packets = tick(game, reader);
                let embedded = trace_contact(game, &format!("step-{step}-idle-{idle}"), &packets);
                let player = game.player.as_ref().unwrap();
                observations.push((
                    player.position,
                    player.ground.on_ground,
                    embedded,
                    game.remote
                        .as_ref()
                        .unwrap()
                        .carry_age
                        .hint()
                        .map(|hint| hint.collider),
                ));
            }
            assert!(
                observations
                    .iter()
                    .enumerate()
                    .all(|(idle, row)| !row.2 && row.1 == !(ordinary_accumulation && idle == 0)),
                "small {axis:?}-axis tilt differs from its actual ordinary support reference: {observations:?}"
            );
            assert!(
                observations
                    .iter()
                    .all(|row| row.0.x == carried.x && row.0.z == carried.z),
                "idle ticks cannot lose or replay horizontal carry: {observations:?}"
            );
            // Source7's actual no-projection reversal control starts with the same
            // downward velocity and measured gap. Ordinary gravity lands once through
            // that real gap; later ticks retain that exact position. The original first
            // three rising cases and pivot tests keep their strict carried-position check.
            let ordinary_landing = step >= 3
                && velocity_y < 0.0
                && gap > 0.0
                && gap * gap > 1.0e-7
                && -velocity_y >= gap;
            if ordinary_landing {
                let landed = carried.add_vector(Vec3::new(0.0, -gap, 0.0));
                assert!(
                    observations.iter().all(|row| row.0 == landed),
                    "ordinary downward collision lands exactly once through the actual support gap: {observations:?}"
                );
            } else if step < 3 || ordinary_accumulation || carried.y >= before.y {
                assert!(
                    observations.iter().all(|row| row.0 == carried),
                    "strict rising support and the ordinary tiny-gap case retain exact carried position: {observations:?}"
                );
            }
            if carried.y >= before.y {
                assert!(
                    observations.iter().all(|row| row.0.y >= before.y),
                    "a rising pose must never lose the pre-pose supported height: {observations:?}"
                );
            }
            assert!(
                observations.windows(2).all(|pair| pair[0].0 == pair[1].0),
                "idle contact must settle without replay or drift: {observations:?}"
            );
            assert!(
                observations
                    .iter()
                    .all(|row| row.3 == row.1.then_some(COLLIDER)),
                "the real carry hint follows actual ground and recovers with its ordinary collision: {observations:?}"
            );
        }
        game.keys.insert(KeyCode::Space);
        tick(game, reader);
        game.keys.remove(&KeyCode::Space);
        let jumped = game.player.as_ref().unwrap().position;
        assert!(!game.player.as_ref().unwrap().ground.on_ground);
        sample.publish(game, axis_pose(Axis::X, 0.04));
        let packets = tick(game, reader);
        trace_contact(game, "ordinary-jump", &packets);
        assert!(!game.player.as_ref().unwrap().ground.on_ground);
        assert!(game.player.as_ref().unwrap().position.y > jumped.y);
        assert!(game.remote.as_ref().unwrap().carry_age.hint().is_none());
    });
}

#[test]
#[ignore = "daily"]
fn ordinary_carried_upright_clearance_respects_ceiling_and_wall() {
    with_deck(Vec::new().into(), |game, reader, sample| {
        let stone = game
            .pack
            .assets
            .registry
            .default_state("minecraft:stone")
            .unwrap();
        game.handle_packet(C::BlockUpdate(play::BlockUpdate {
            pos: [6, 99, 6],
            state: stone.0 as i32,
        }))
        .unwrap();
        sample.publish(game, pose(0.1));
        let packets = tick(game, reader);
        trace_contact(game, "ceiling-clipped-carry", &packets);
        let player = game.player.as_ref().unwrap();
        assert!(
            player.bounds().max(Axis::Y) <= 99.0,
            "ordinary ceiling clips the requested clearance"
        );
        assert!(
            !player.ground.on_ground,
            "a clipped embedded lift cannot force ground"
        );
        assert!(game.remote.as_ref().unwrap().carry_age.hint().is_none());
    });
    with_deck(Vec::new().into(), |game, reader, sample| {
        let stone = game
            .pack
            .assets
            .registry
            .default_state("minecraft:stone")
            .unwrap();
        for y in 96..=101 {
            game.handle_packet(C::BlockUpdate(play::BlockUpdate {
                pos: [5, y, 6],
                state: stone.0 as i32,
            }))
            .unwrap();
        }
        sample.publish(game, pose(0.3));
        let packets = tick(game, reader);
        trace_contact(game, "wall-clipped-carry", &packets);
        let player = game.player.as_ref().unwrap();
        assert!(
            player.bounds().min(Axis::X) >= 6.0,
            "ordinary wall clips the carried horizontal move"
        );
        let bounds = player.bounds();
        let grounded = player.ground.on_ground;
        let embedded = game
            .placed_moving_colliders()
            .placed()
            .find(|p| p.id == COLLIDER)
            .unwrap()
            .collider
            .intersects(bounds);
        if embedded {
            assert!(!grounded, "blocked clearance never forges support");
        }
    });
}

#[test]
#[ignore = "daily"]
fn ordinary_client_at_rotation_pivot_still_lifts_its_upright_footprint() {
    for axis in [Axis::X, Axis::Z] {
        for angle in [-0.01_f64, 0.01] {
            let mut initial_pose = pose(0.0);
            // At identity this reanchors the same actual deck frame, with the rotation
            // pivot exactly at the initial player's feet centre. No body is reset later.
            initial_pose.pivot = [2.5, 65.0, 2.5];
            with_deck_pose(Vec::new().into(), initial_pose, |game, reader, sample| {
                let before = game.player.as_ref().unwrap().position;
                let mut next = sample.current;
                next.rotation = axis_pose(axis, angle).rotation;
                let old_frame = Frame::from_quaternion_about(
                    vector(sample.current.translation),
                    vector(sample.current.pivot),
                    sample.current.rotation,
                );
                let new_frame = Frame::from_quaternion_about(
                    vector(next.translation),
                    vector(next.pivot),
                    next.rotation,
                );
                let rigid = new_frame
                    .to_world(old_frame.to_local(before))
                    .subtract(before);
                assert!(
                    rigid.length_squared() < 1.0e-18,
                    "the represented pivot contact must hit the previous zero/tiny-delta skip"
                );
                sample.publish(game, next);
                let packets = tick(game, reader);
                assert!(!trace_contact(game, "pivot-centre-clearance", &packets));
                let after = game.player.as_ref().unwrap().position;
                assert!(
                    after.y > before.y,
                    "zero rigid delta still needs uphill footprint clearance"
                );
                assert!(
                    after.y - before.y <= 0.6,
                    "clearance retains the ordinary step allowance"
                );
                assert!(reported_position(&packets).is_some());
                for _ in 0..8 {
                    let packets = tick(game, reader);
                    assert!(!trace_contact(game, "pivot-idle", &packets));
                    let player = game.player.as_ref().unwrap();
                    assert!(player.ground.on_ground);
                    assert_eq!(
                        player.position, after,
                        "idle ticks cannot replay the pivot clearance"
                    );
                    assert_eq!(
                        game.remote
                            .as_ref()
                            .unwrap()
                            .carry_age
                            .hint()
                            .unwrap()
                            .collider,
                        COLLIDER
                    );
                }
            });
        }
    }
}

/// Independent ordinary baseline: the receiver has no live projection or moving collider.
/// A fresh zero-Y-velocity body starts a tiny, real gap above a resident vanilla stone floor.
#[test]
#[ignore = "daily"]
fn ordinary_static_floor_tiny_gap_zero_velocity_records_full_support_sequence() {
    with_deck(Vec::new().into(), |game, reader, _| {
        incoming(game, wire::Message::Clear { id: ID });
        let stone = game
            .pack
            .assets
            .registry
            .default_state("minecraft:stone")
            .unwrap();
        for x in 3..=9 {
            for z in 3..=9 {
                game.handle_packet(C::BlockUpdate(play::BlockUpdate {
                    pos: [x, 96, z],
                    state: stone.0 as i32,
                }))
                .unwrap();
            }
        }
        assert!(
            game.placed_moving_colliders().is_empty(),
            "ordinary control has no moving/projection geometry"
        );
        // One initial scene construction, never a per-tick position write or carry substitute.
        let initial = vector([6.5, 97.0001, 6.5]);
        let mut player = PlayerMotion::new(game.motion, initial).unwrap();
        player.entity_id = 7;
        player.client_side = true;
        assert_eq!(player.velocity, Vec3::ZERO);
        game.player = Some(player);
        game.previous_position = initial;
        let mut observed = Vec::new();
        for idle in 0..8 {
            let packets = tick(game, reader);
            let player = game.player.as_ref().unwrap();
            let hint = game
                .remote
                .as_ref()
                .unwrap()
                .carry_age
                .hint()
                .map(|hint| hint.collider);
            eprintln!(
                "STATIC_FLOOR_BASELINE idle={idle} position={:?} velocity={:?} ground={} hint={hint:?} reported={:?}",
                player.position,
                player.velocity,
                player.ground.on_ground,
                reported_position(&packets)
            );
            observed.push((
                player.position,
                player.velocity,
                player.ground.on_ground,
                hint,
            ));
        }
        assert!(
            observed
                .iter()
                .all(|row| row.0 == initial && row.3.is_none()),
            "ordinary control never carries/loses position: {observed:?}"
        );
        assert!(
            !observed[0].2,
            "the zero-Y invocation first accumulates ordinary gravity: {observed:?}"
        );
        assert!(
            observed[1..].iter().all(|row| row.2),
            "later real downward collision restores ordinary support: {observed:?}"
        );
    });
}

/// Independent control for source6 step9's observed overclearance: no projection/moving
/// geometry, same ordinary client body, resident stone floor, existing downward gravity.
#[test]
#[ignore = "daily"]
fn ordinary_static_floor_reversal_gap_with_downward_velocity_lands_once() {
    with_deck(Vec::new().into(), |game, reader, _| {
        let velocity = game.player.as_ref().unwrap().velocity;
        assert!(
            velocity.y < 0.0,
            "the ordinary settled body already has downward gravity"
        );
        incoming(game, wire::Message::Clear { id: ID });
        let stone = game
            .pack
            .assets
            .registry
            .default_state("minecraft:stone")
            .unwrap();
        for x in 3..=9 {
            for z in 3..=9 {
                game.handle_packet(C::BlockUpdate(play::BlockUpdate {
                    pos: [x, 96, z],
                    state: stone.0 as i32,
                }))
                .unwrap();
            }
        }
        assert!(game.placed_moving_colliders().is_empty());
        // This gap is from the retained OWN source6/887 step9 trace, not another caller.
        let initial = vector([6.5, 97.0 + 0.0032826785103189046, 6.5]);
        let mut player = PlayerMotion::new(game.motion, initial).unwrap();
        player.entity_id = 7;
        player.client_side = true;
        player.velocity = velocity;
        game.player = Some(player);
        game.previous_position = initial;
        let mut observed = Vec::new();
        for idle in 0..8 {
            let packets = tick(game, reader);
            let player = game.player.as_ref().unwrap();
            let hint = game
                .remote
                .as_ref()
                .unwrap()
                .carry_age
                .hint()
                .map(|hint| hint.collider);
            eprintln!(
                "STATIC_REVERSAL_BASELINE idle={idle} position={:?} velocity={:?} ground={} hint={hint:?} reported={:?}",
                player.position,
                player.velocity,
                player.ground.on_ground,
                reported_position(&packets)
            );
            observed.push((
                player.position,
                player.velocity,
                player.ground.on_ground,
                hint,
            ));
        }
        let floor = vector([initial.x, 97.0, initial.z]);
        assert!(
            observed
                .iter()
                .all(|row| row.0 == floor && row.2 && row.3.is_none()),
            "ordinary gravity lands once on the actual floor and stays supported: {observed:?}"
        );
    });
}
