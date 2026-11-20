//! Ordinary client tick regression for a caller-owned sparse real viewed-world projection.
//! Needs prepared resources and the existing local connection fixture.
use super::*;
use crate::game::{
    Game, Screen,
    native_entity_refs::tests::with_client_game,
    viewed_worlds::tests::{layered, open, run, spawn},
};
use sunrise_client::motion::PlayerMotion;
use sunrise_protocol::{
    connection::ConnReader,
    packets::{
        Empty,
        configuration::CustomPayload,
        play::{self, Clientbound as C, Serverbound as S},
    },
};
use winit::keyboard::KeyCode;

const SOURCE: &str = "carry_fixture:local";
const ID: u64 = 7;
const COLLIDER: u32 = 991;

fn incoming(game: &mut Game<'_>, message: wire::Message) {
    game.handle_packet(C::CustomPayload(CustomPayload {
        channel: wire::CHANNEL.into(),
        data: wire::encode(&message).unwrap(),
    }))
    .unwrap();
}

/// The real outgoing play stream through this tick's ordinary CLIENT_TICK_END, fenced by a
/// later packet. This catches a carried move emitted one tick late or after CLIENT_TICK_END.
fn tick(game: &mut Game<'_>, reader: &mut ConnReader) -> Vec<S> {
    game.tick().unwrap();
    game.send(S::Punch(Empty));
    let mut packets = Vec::new();
    loop {
        let (id, body) = reader.recv().unwrap();
        let packet = S::decode(id, &body).unwrap();
        if matches!(packet, S::Punch(_)) {
            return packets;
        }
        packets.push(packet);
    }
}

fn reported_position(packets: &[S]) -> Option<(usize, [f64; 3])> {
    packets
        .iter()
        .enumerate()
        .find_map(|(index, packet)| match packet {
            S::MovePlayerPos(p) => Some((index, [p.x, p.y, p.z])),
            S::MovePlayerPosRot(p) => Some((index, [p.x, p.y, p.z])),
            _ => None,
        })
}

/// Real emitted horizontal model positions relative to this tick's actual camera. Eye-height
/// easing is a separate vanilla vertical motion; this fixture translates along X alone.
fn presented_positions(game: &mut Game<'_>, partial: f64) -> Vec<[f32; 2]> {
    let (camera, _, _) = game
        .camera_view(partial as f32)
        .expect("ordinary player camera");
    let mut vertices = Vec::new();
    game.live_world_geometry(camera, partial, &mut vertices);
    assert!(
        !vertices.is_empty(),
        "real prepared source models were emitted"
    );
    vertices
        .into_iter()
        .map(|vertex| [vertex.pos[0], vertex.pos[2]])
        .collect()
}

/// The outline uses the authoritative current pick, then the same visual tick as the model.
fn outlined_positions(game: &mut Game<'_>, partial: f64) -> Vec<[f64; 4]> {
    let player = game.player.as_ref().unwrap();
    let eye = [
        player.position.x,
        player.position.y + f64::from(player.dimensions.eye_height),
        player.position.z,
    ];
    game.refresh_projection_pick(eye, [0.0, -1.0, 0.0], None);
    assert_eq!(game.projection_pick().unwrap().hit.cell, [1, 64, 0]);
    let (camera, _, _) = game.camera_view(partial as f32).unwrap();
    let edges = game.projection_pick_outline(partial);
    assert!(
        !edges.is_empty(),
        "the ordinary picked stone has real outline edges"
    );
    edges
        .into_iter()
        .map(|edge| {
            [
                edge[0] - camera.x,
                edge[2] - camera.z,
                edge[3] - camera.x,
                edge[5] - camera.z,
            ]
        })
        .collect()
}

#[test]
#[ignore = "daily"]
fn client_tick_consumes_coalesced_projection_contact_before_ordinary_position_report() {
    mod_platform::testing::with_mods(Vec::new().into(), || {
        with_client_game(|game| {
            let (mut reader, _writer) = crate::game::remote::tests::connect_menu_test(game);
            // The connection fixture stops after configuration without sending registry data.
            // Supply the actual dimension-type table that this synthetic server configures,
            // through the same registry apply route, before ordinary Login creates the epoch.
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
            assert!(
                game.native_client_level().is_some(),
                "Login creates a real client level"
            );
            game.handle_packet(layered(0, 0, 0)).unwrap();
            game.handle_packet(open(0, SOURCE)).unwrap();
            assert!(
                game.remote.is_some(),
                "a refused open must not look like a missing source"
            );
            assert!(
                game.level(SOURCE).is_some(),
                "the configured source was really opened"
            );
            let stone = game
                .pack
                .assets
                .registry
                .default_state("minecraft:stone")
                .unwrap();
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
            // L-shaped sparse support, real stock chest/furnace and a detached member. The missing
            // (1,64,1) cell remains a real hole, not a bounding-box floor.
            let cells = vec![
                [0, 64, 0],
                [0, 64, 1],
                [0, 65, 0],
                [1, 64, 0],
                [2, 64, 0],
                [4, 64, 0],
            ];
            let mut source_packets = vec![layered(0, 0, 0)];
            for &pos in &cells {
                source_packets.push(C::BlockUpdate(play::BlockUpdate {
                    pos,
                    state: if pos == [0, 65, 0] {
                        chest.0 as i32
                    } else if pos == [2, 64, 0] {
                        furnace.0 as i32
                    } else {
                        stone.0 as i32
                    },
                }));
            }
            game.handle_packet(run(0, &source_packets)).unwrap();
            let source = game.level(SOURCE).unwrap();
            let world = game.native_level_view_world(source).unwrap();
            let revision = world.revision;
            let hash = state_hash(world, &cells);
            let pose = |x| wire::Pose {
                translation: [x, 32.0, 4.0],
                pivot: [0.5, 64.0, 0.5],
                rotation: [0.0, 0.0, 0.0, 1.0],
            };
            let sample = |frame, before, now| wire::Sample {
                frame,
                source_revision: revision,
                state_hash: hash,
                previous: pose(before),
                current: pose(now),
                revision: frame,
            };
            incoming(
                game,
                wire::Message::Snapshot {
                    id: ID,
                    collider: COLLIDER,
                    boxes: 8,
                    source: SOURCE.into(),
                    destination: "minecraft:overworld".into(),
                    cells: cells.clone(),
                    membership: 1,
                    sample: sample(1, 3.9, 4.0),
                },
            );
            game.title = None;
            game.screen = Screen::Playing;
            let remote = game.remote.as_mut().unwrap();
            remote.loaded_sent = true;
            remote.level_load.finish();
            let mut player = PlayerMotion::new(game.motion, vector([5.5, 97.0, 4.5])).unwrap();
            player.entity_id = 7;
            player.client_side = true;
            player.ground.on_ground = true;
            game.player = Some(player);
            game.previous_position = vector([5.5, 97.0, 4.5]);
            // Late join starts settled even if the first authoritative sample is moving.
            tick(game, &mut reader);
            assert_eq!(game.player.as_ref().unwrap().position.x, 5.5);
            let settled_positions = presented_positions(game, 0.5);
            let settled_outline = outlined_positions(game, 0.5);
            for partial in [0.0, 1.0] {
                assert_eq!(
                    presented_positions(game, partial),
                    settled_positions,
                    "a first moving snapshot starts visually settled"
                );
            }

            // Both updates are applied before the ordinary tick, as the real packet poll does.
            // The latest authoritative previous pose is 4.3, but the actual contact frontier
            // is still 4.0: no part of the total 0.4 translation may be dropped.
            incoming(
                game,
                wire::Message::Pose {
                    id: ID,
                    sample: sample(2, 4.0, 4.1),
                },
            );
            let moved_sample = sample(3, 4.3, 4.4);
            incoming(
                game,
                wire::Message::Pose {
                    id: ID,
                    sample: moved_sample,
                },
            );
            let packets = tick(game, &mut reader);
            let player = game.player.as_ref().unwrap();
            assert_eq!(player.position, vector([5.9, 97.0, 4.5]));
            assert!(player.ground.on_ground);
            assert_eq!(game.previous_position, vector([5.5, 97.0, 4.5]));
            assert_eq!(game.player_camera_entity().unwrap().previous[0], 5.5);
            assert_eq!(game.camera_view(0.5).unwrap().0.x, 5.7);
            for partial in [0.0, 0.5, 1.0] {
                assert_eq!(
                    presented_positions(game, partial),
                    settled_positions,
                    "real emitted source geometry stays at the same camera-relative contact across the coalesced tick"
                );
                assert_eq!(
                    outlined_positions(game, partial),
                    settled_outline,
                    "the visible pick outline shares the coalesced model/camera endpoints"
                );
            }
            let (position_index, position) =
                reported_position(&packets).expect("same-tick position");
            assert_eq!(position, [5.9, 97.0, 4.5]);
            let end_index = packets
                .iter()
                .position(|p| matches!(p, S::ClientTickEnd(_)))
                .unwrap();
            let hint_index = packets
                .iter()
                .position(|p| match p {
                    S::CustomPayload(p) if p.channel == sunrise_protocol::carry_age::CHANNEL => {
                        assert_eq!(
                            sunrise_protocol::carry_age::decode(&p.data).unwrap(),
                            sunrise_protocol::carry_age::Hint {
                                collider: COLLIDER,
                                ticks: 0
                            }
                        );
                        true
                    }
                    _ => false,
                })
                .expect("live projection support age even with no ordinary entity colliders");
            assert!(hint_index < position_index && position_index < end_index);
            assert!(game.remote.as_ref().unwrap().colliders.is_empty());
            assert_eq!(
                game.live_worlds.borrow().sync.replicas[&ID].sample,
                api_sample(moved_sample),
                "physical consumption cannot rewrite the published render/action sample"
            );

            // Duplicate packets and ordinary idle ticks cannot replay the carried displacement.
            incoming(
                game,
                wire::Message::Pose {
                    id: ID,
                    sample: moved_sample,
                },
            );
            for _ in 0..3 {
                tick(game, &mut reader);
                assert_eq!(
                    game.player.as_ref().unwrap().position,
                    vector([5.9, 97.0, 4.5])
                );
                for partial in [0.0, 0.5, 1.0] {
                    assert_eq!(
                        presented_positions(game, partial),
                        settled_positions,
                        "idle completed ticks cannot replay a visual displacement"
                    );
                    assert_eq!(
                        outlined_positions(game, partial),
                        settled_outline,
                        "an idle outline cannot replay the prior visual displacement"
                    );
                }
            }

            // Jumping uses the ordinary player motion and leaves support before the carry.
            game.keys.insert(KeyCode::Space);
            incoming(
                game,
                wire::Message::Pose {
                    id: ID,
                    sample: sample(4, 4.4, 4.5),
                },
            );
            tick(game, &mut reader);
            assert_eq!(game.player.as_ref().unwrap().position.x, 5.9);
            assert!(game.player.as_ref().unwrap().position.y > 97.0);
            assert!(!game.player.as_ref().unwrap().ground.on_ground);
            game.keys.clear();

            // A body over the explicit sparse hole falls and gains no horizontal carry.
            let mut player = PlayerMotion::new(game.motion, vector([6.0, 97.0, 5.5])).unwrap();
            player.entity_id = 7;
            player.client_side = true;
            player.ground.on_ground = true;
            game.player = Some(player);
            incoming(
                game,
                wire::Message::Pose {
                    id: ID,
                    sample: sample(5, 4.5, 4.6),
                },
            );
            tick(game, &mut reader);
            assert_eq!(game.player.as_ref().unwrap().position.x, 6.0);
            tick(game, &mut reader);
            assert_eq!(game.player.as_ref().unwrap().position.x, 6.0);
            assert!(game.player.as_ref().unwrap().position.y < 97.0);
            assert!(!game.player.as_ref().unwrap().ground.on_ground);

            // Return to the real stone support in the current pose. Changing a stock furnace's
            // lit property rebuilds its state witness and model while preserving its collision
            // boxes; the same tick's platform motion must still carry the standing body.
            let mut player = PlayerMotion::new(game.motion, vector([6.1, 97.0, 4.5])).unwrap();
            player.entity_id = 7;
            player.client_side = true;
            player.ground.on_ground = true;
            game.player = Some(player);
            tick(game, &mut reader);
            let before_toggle_positions = presented_positions(game, 0.5);
            let lit_furnace = game.with_props(furnace, &[("lit", "true")]).unwrap();
            let before_lit_revision = game.native_level_view_world(source).unwrap().revision;
            game.handle_packet(run(
                0,
                &[C::BlockUpdate(play::BlockUpdate {
                    pos: [2, 64, 0],
                    state: lit_furnace.0 as i32,
                })],
            ))
            .unwrap();
            let world = game.native_level_view_world(source).unwrap();
            let lit_hash = state_hash(world, &cells);
            assert_eq!(
                world.revision,
                before_lit_revision.wrapping_add(1),
                "the real raw viewed BlockUpdate invalidates the observed source revision"
            );
            assert_ne!(lit_hash, hash, "the actual state changed");
            let mut lit_sample = sample(6, 4.6, 4.7);
            lit_sample.source_revision = world.revision;
            lit_sample.state_hash = lit_hash;
            incoming(
                game,
                wire::Message::Pose {
                    id: ID,
                    sample: lit_sample,
                },
            );
            tick(game, &mut reader);
            assert_eq!(
                game.player.as_ref().unwrap().position,
                vector([6.2, 97.0, 4.5])
            );
            assert!(game.player.as_ref().unwrap().ground.on_ground);
            for partial in [0.0, 0.5, 1.0] {
                assert_eq!(
                    presented_positions(game, partial),
                    before_toggle_positions,
                    "collision-equivalent material state retains physical and visual contact"
                );
            }

            // An identical state update must not advance the block-state revision again.
            game.handle_packet(run(
                0,
                &[C::BlockUpdate(play::BlockUpdate {
                    pos: [2, 64, 0],
                    state: lit_furnace.0 as i32,
                })],
            ))
            .unwrap();
            assert_eq!(
                game.native_level_view_world(source).unwrap().revision,
                before_lit_revision.wrapping_add(1)
            );

            // A real viewed chest inventory update is independent of block-state geometry.
            let kind = game
                .remote
                .as_ref()
                .unwrap()
                .builtin
                .get("minecraft:block_entity_type")
                .unwrap()
                .id("minecraft:chest")
                .unwrap();
            let tag = sunrise_core::nbt::parse_snbt(
                r#"{Items:[{Slot:3b,id:"minecraft:diamond",count:7}]}"#,
            )
            .unwrap()
            .0;
            game.handle_packet(run(
                0,
                &[C::BlockEntityData(play::BlockEntityData {
                    pos: [0, 65, 0],
                    kind,
                    tag: tag.clone(),
                })],
            ))
            .unwrap();
            let viewed = game.remote.as_ref().unwrap().viewed.get(0).unwrap();
            let info = viewed.block_entities[&(0, 0)]
                .iter()
                .find(|info| info.packed_xz == 0 && info.y == 65)
                .unwrap();
            assert_eq!(
                info.tag, tag,
                "the ordinary viewed chest really received the inventory"
            );
            let world = game.native_level_view_world(source).unwrap();
            assert_eq!(state_hash(world, &cells), lit_hash);
            assert_eq!(
                world.revision,
                before_lit_revision.wrapping_add(1),
                "block-entity-only data does not invalidate block geometry"
            );
            let mut inventory_sample = sample(7, 4.7, 4.8);
            inventory_sample.source_revision = world.revision;
            inventory_sample.state_hash = lit_hash;
            incoming(
                game,
                wire::Message::Pose {
                    id: ID,
                    sample: inventory_sample,
                },
            );
            tick(game, &mut reader);
            assert_eq!(
                game.player.as_ref().unwrap().position,
                vector([6.3, 97.0, 4.5])
            );
            assert!(game.player.as_ref().unwrap().ground.on_ground);
            for partial in [0.0, 0.5, 1.0] {
                assert_eq!(
                    presented_positions(game, partial),
                    before_toggle_positions,
                    "inventory-only updates retain the same ordinary camera-relative contact"
                );
            }

            // Full column input/removal also changes the actual viewed-world identity. Use a
            // separate resident column outside membership, preserving all selected payloads.
            let before_column_revision = game.native_level_view_world(source).unwrap().revision;
            game.handle_packet(run(0, &[layered(1, 0, 0)])).unwrap();
            assert_eq!(
                game.native_level_view_world(source).unwrap().revision,
                before_column_revision.wrapping_add(1),
                "real column insertion invalidates caches"
            );
            game.handle_packet(run(0, &[layered(1, 0, 0)])).unwrap();
            assert_eq!(
                game.native_level_view_world(source).unwrap().revision,
                before_column_revision.wrapping_add(2),
                "full replacement invalidates caches"
            );
            let forget = || C::ForgetLevelChunk(play::ForgetLevelChunk { pos: [1, 0] });
            game.handle_packet(run(0, &[forget()])).unwrap();
            assert_eq!(
                game.native_level_view_world(source).unwrap().revision,
                before_column_revision.wrapping_add(3),
                "actual removal invalidates caches"
            );
            game.handle_packet(run(0, &[forget()])).unwrap();
            assert_eq!(
                game.native_level_view_world(source).unwrap().revision,
                before_column_revision.wrapping_add(3),
                "absent removal has no block-state change"
            );
            let C::LevelChunkWithLight(column) = layered(0, 0, 0) else {
                unreachable!();
            };
            game.handle_packet(run(
                0,
                &[C::LightUpdate(play::LightUpdate {
                    x: 0,
                    z: 0,
                    light: column.light,
                })],
            ))
            .unwrap();
            assert_eq!(
                game.native_level_view_world(source).unwrap().revision,
                before_column_revision.wrapping_add(3),
                "light-only data leaves block geometry valid"
            );
            tick(game, &mut reader);
            assert_eq!(
                game.player.as_ref().unwrap().position,
                vector([6.3, 97.0, 4.5])
            );
            assert!(game.player.as_ref().unwrap().ground.on_ground);
        });
    });
}

#[path = "sync_carry_tests/tilt_tests.rs"]
mod tilt_tests;
