//! The native travel decision affects real player reports and sends vanilla corrections.
use super::*;
use crate::game::execution_role::ExecutionRole;
use crate::game::native_entity_refs::tests::{fixture_root, run_fresh_cases, with_game_role};
use crate::game::native_item_places::tests::guest as online_guest;
use mod_api::{EntityMoveEvent, Flow, Hooks, LivingTravelEvent, LoopContext, MoveCause};
use std::sync::atomic::{AtomicUsize, Ordering};
use sunrise_client::{
    chunk::Chunk,
    lan::{NetConfig, Out, hub::HubConfig},
};
use sunrise_protocol::packets::{Empty, play::Serverbound as S};

const START: [f64; 3] = [3.0, 64.0, 5.0];
const ACCEPTED: [f64; 3] = [3.25, 64.0, 5.0];
const NEXT: [f64; 3] = [3.5, 64.0, 5.0];

#[derive(Clone, Copy)]
enum Reply {
    Continue,
    Veto,
    Velocity,
    TinyInput,
    OverflowingInputLength,
    Input,
    LargeInput,
    LargeVelocity,
    HugeVelocity,
    NonFiniteVelocityWithInput,
    NonFiniteInputWithVelocity,
    NonFiniteNoOp,
    VetoWithNonFiniteVelocity,
}

#[mod_api::__subscriptions]
impl Hooks for Reply {
    fn living_travel(&self, _: &dyn LoopContext, event: &mut LivingTravelEvent<'_>) -> Flow {
        assert_eq!(event.kind, "minecraft:player");
        match self {
            Self::Continue => {}
            Self::Veto => return Flow::Cancel,
            Self::Velocity => *event.velocity = [0.5, 0.0, 0.0],
            Self::TinyInput => *event.input = [0.000001, 0.0, 0.0],
            Self::OverflowingInputLength => *event.input = [f64::MAX, 0.0, 0.0],
            Self::Input => *event.input = [1.0, 0.0, 0.0],
            Self::LargeInput => *event.input = [2.0, 0.0, 0.0],
            Self::LargeVelocity => *event.velocity = [70.0, 0.0, 0.0],
            Self::HugeVelocity => *event.velocity = [1.0e9, 0.0, 1.0e9],
            Self::NonFiniteVelocityWithInput => {
                *event.velocity = [f64::NAN, 0.0, 0.0];
                *event.input = [1.0, 0.0, 0.0];
            }
            Self::NonFiniteInputWithVelocity => {
                *event.input = [f64::INFINITY, 0.0, 0.0];
                *event.velocity = [0.5, 0.0, 0.0];
            }
            Self::NonFiniteNoOp => {
                *event.input = [f64::NAN, 0.0, 0.0];
                *event.velocity = [f64::INFINITY, 0.0, 0.0];
            }
            Self::VetoWithNonFiniteVelocity => {
                *event.velocity = [f64::NAN, 0.0, 0.0];
                return Flow::Cancel;
            }
        }
        Flow::Continue
    }
}

fn with_player(integrated: bool, run: impl FnOnce(&mut Game<'_>, mpsc::Receiver<Out>)) {
    let role = if integrated {
        ExecutionRole::IntegratedServer
    } else {
        ExecutionRole::DedicatedServer
    };
    with_game_role(fixture_root(), role, |game| {
        game.lan_host(
            crate::game::lan::LanSettings {
                port: 0,
                guest_commands: false,
                force_game_mode: false,
            },
            NetConfig {
                listen: false,
                announce: false,
                ..NetConfig::loopback(0, "living-travel".into(), 2)
            },
            HubConfig {
                default_game_mode: 0,
                guest_commands: false,
                force_game_mode: false,
                hardcore: false,
                view_distance: 2,
                simulation_distance: 2,
                dimension: "minecraft:overworld".into(),
                allow_flight: false,
                seed: 0,
                max_players: 2,
                integrated,
                levels: Default::default(),
                level_seeds: Default::default(),
            },
        )
        .unwrap();
        game.streaming
            .insert_remote(&mut game.world, Chunk::empty(0, 0));
        let mut guest = online_guest(1);
        guest.loaded = true;
        guest.load_timeout = 0;
        guest.position = START;
        guest.first_good = START;
        guest.last_reported = (START, [0.0, 0.0]);
        guest.motion = Some(Box::new(
            PlayerMotion::new(game.motion, Vec3::new(3.0, 64.0, 5.0)).unwrap(),
        ));
        let (tx, rx) = mpsc::channel();
        guest.sender = PlayerSender::new(tx, Arc::default());
        let hub = &mut game.lan.as_mut().unwrap().hub;
        hub.ticks = 10;
        hub.singleplayer_owner = integrated.then_some(guest.uuid);
        hub.guests.push(guest);
        run(game, rx);
    });
}

fn with_reply(reply: Option<Reply>, run: impl FnOnce()) {
    let mods = reply
        .map(|reply| mod_api::ModEntry::new("travel-test", Box::new(reply)))
        .into_iter()
        .collect::<Vec<_>>();
    mod_platform::testing::with_mods(mods.into(), run);
}

fn send(game: &mut Game<'_>, packet: S) {
    game.lan.as_mut().unwrap().hub.update_guest(1, |g| {
        use sunrise_client::lan::guest::InboxItem;
        g.inbox.push_back(InboxItem::Packet(packet, None));
        g.inbox
            .push_back(InboxItem::Packet(S::ClientTickEnd(Empty), None));
    });
    game.level_pass.packets_drained = false;
    game.tick_lan_packets();
    // The packet phase holds output until the tick's usual network handoff.
    guest(game).sender.flush();
}

fn report(game: &mut Game<'_>, [x, y, z]: [f64; 3]) {
    send(
        game,
        S::MovePlayerPos(play::MovePlayerPos { x, y, z, flags: 0 }),
    );
}

#[test]
#[ignore = "daily"]
fn a_connected_players_report_never_exposes_stale_collision_flags() {
    for integrated in [true, false] {
        with_player(integrated, |game, _rx| {
            game.lan
                .as_mut()
                .unwrap()
                .hub
                .update_guest(1, |g| {
                    g.motion.as_mut().unwrap().collision.horizontal = true;
                })
                .unwrap();
            let player = game.player(guest(game).uuid).unwrap();
            report(game, ACCEPTED);
            assert_eq!(guest(game).position, ACCEPTED);
            assert_eq!(player.collision_flags(game), Ok(None));
            game.as_guest(1, |game| {
                assert_eq!(player.collision_flags(game), Ok(None));
            })
            .unwrap();
        });
    }
}

fn guest<'a>(game: &'a Game<'_>) -> &'a Guest {
    game.lan.as_ref().unwrap().hub.guest(1).unwrap()
}

fn packets(rx: &mpsc::Receiver<Out>) -> Vec<(i32, Vec<u8>)> {
    rx.try_iter()
        .flat_map(|out| match out {
            Out::Packet(body) => vec![body],
            Out::Packets(bodies) => bodies,
            _ => Vec::new(),
        })
        .map(|body| {
            let mut r = sunrise_protocol::buf::Reader::new(&body);
            (r.varint().unwrap(), r.rest().to_vec())
        })
        .collect()
}

fn correction(rx: &mpsc::Receiver<Out>) -> play::PlayerPosition {
    let positions = packets(rx)
        .into_iter()
        .filter_map(|(id, body)| match C::decode(id, &body).unwrap() {
            C::PlayerPosition(packet) => Some(packet),
            C::SetEntityMotion(_) => panic!("travel correction must carry velocity itself"),
            _ => None,
        })
        .collect::<Vec<_>>();
    let [packet]: [_; 1] = positions.try_into().expect("one position correction");
    assert_eq!(packet.relatives, 0);
    packet
}

fn acknowledge(game: &mut Game<'_>, packet: &play::PlayerPosition) {
    let [x, y, z] = packet.position;
    send(
        game,
        S::AcceptTeleportation(play::AcceptTeleportation {
            id: packet.teleport,
            x,
            y,
            z,
            yaw: packet.yaw,
            pitch: packet.pitch,
        }),
    );
}

#[test]
#[ignore = "daily"]
fn a_player_travel_veto_holds_new_reports_not_previously_accepted_ones() {
    for integrated in [true, false] {
        with_reply(Some(Reply::Veto), || {
            with_player(integrated, |game, rx| {
                report(game, ACCEPTED);
                assert_eq!(guest(game).position, ACCEPTED);
                game.guest_server_tick(1, true);
                assert_eq!(guest(game).position, ACCEPTED);
                assert!(guest(game).awaiting_teleport.is_none());
                packets(&rx);

                report(game, NEXT);
                let packet = correction(&rx);
                assert_eq!(packet.position, ACCEPTED);
                assert_eq!(guest(game).position, ACCEPTED);

                acknowledge(game, &packet);
                assert!(guest(game).awaiting_teleport.is_none());
                report(game, NEXT);
                assert_eq!(correction(&rx).position, ACCEPTED);
                assert_eq!(guest(game).position, ACCEPTED);
            })
        });
    }
}

#[test]
#[ignore = "daily"]
fn a_player_travel_replacement_commits_displacement_and_corrects_velocity() {
    for integrated in [true, false] {
        for reply in [Reply::Velocity, Reply::Input] {
            with_reply(Some(reply), || {
                with_player(integrated, |game, rx| {
                    game.guest_server_tick(1, true);
                    let position = [
                        3.0 + match reply {
                            Reply::Velocity => 0.5,
                            // The default off-ground movement speed is the widened vanilla 0.02f.
                            _ => f64::from(0.02_f32),
                        },
                        64.0,
                        5.0,
                    ];
                    let packet = correction(&rx);
                    assert_eq!(packet.position, position);
                    assert_eq!(guest(game).position, position);
                    let expected = Vec3::new(
                        (position[0] - START[0]) * f64::from(0.91_f32),
                        -0.08 * f64::from(0.98_f32),
                        0.0,
                    );
                    assert_eq!(
                        sunrise_client::player_sync::teleport_velocity(
                            Vec3::new(1.0, 1.0, 1.0),
                            (0.0, 0.0),
                            (0.0, 0.0),
                            &packet
                        ),
                        expected,
                    );
                    assert_eq!(guest(game).motion.as_ref().unwrap().velocity, expected);
                })
            });
        }
    }
}

#[test]
#[ignore = "daily"]
fn another_travel_replacement_waits_for_the_first_correction_acknowledgement() {
    with_reply(Some(Reply::Velocity), || {
        with_player(false, |game, rx| {
            game.guest_server_tick(1, true);
            let first = correction(&rx);
            assert_eq!(first.position, NEXT);

            for _ in 0..2 {
                game.lan.as_mut().unwrap().hub.ticks += 1;
                game.guest_server_tick(1, true);
            }
            report(game, [4.0, 64.0, 5.0]);
            assert_eq!(guest(game).position, NEXT);
            assert!(!packets(&rx).into_iter().any(|(id, body)| matches!(
                C::decode(id, &body).unwrap(),
                C::PlayerPosition(_) | C::SetEntityMotion(_)
            )));

            acknowledge(game, &first);
            assert!(guest(game).awaiting_teleport.is_none());
            report(game, [4.0, 64.0, 5.0]);
            assert_eq!(guest(game).position, [4.0, 64.0, 5.0]);
            game.guest_server_tick(1, true);
            let next = correction(&rx);
            assert_ne!(next.teleport, first.teleport);
            assert_eq!(next.position, [4.5, 64.0, 5.0]);
        });
    });
}

#[test]
#[ignore = "daily"]
fn an_edit_absorbed_by_collision_still_sends_one_correction() {
    if run_fresh_cases(
        "game::lan_actions::guest_body::tests::travel::an_edit_absorbed_by_collision_still_sends_one_correction",
        "SUNRISE_PLAYER_TRAVEL_CHILD",
        &["travel"],
    ) {
        return;
    }
    struct IntoWall {
        calls: Arc<AtomicUsize>,
        edit_input: bool,
    }

    #[mod_api::__subscriptions]
    impl Hooks for IntoWall {
        fn living_travel(&self, _: &dyn LoopContext, event: &mut LivingTravelEvent<'_>) -> Flow {
            self.calls.fetch_add(1, Ordering::Relaxed);
            if self.edit_input {
                *event.input = [-1.0, 0.0, 0.0];
            } else {
                *event.velocity = [-0.5, 0.0, 0.0];
            }
            Flow::Continue
        }
    }

    for edit_input in [false, true] {
        let calls = Arc::new(AtomicUsize::new(0));
        let mods = vec![mod_api::ModEntry::new(
            "wall-travel-test",
            Box::new(IntoWall {
                calls: calls.clone(),
                edit_input,
            }),
        )];
        mod_platform::testing::with_mods(mods.into(), || {
            with_player(false, |game, rx| {
                let stone = game
                    .pack
                    .assets
                    .registry
                    .default_state("minecraft:stone")
                    .unwrap();
                game.world.set(2, 64, 5, stone).unwrap();
                let mut at = START;
                let half_width =
                    f64::from(guest(game).motion.as_ref().unwrap().dimensions.width) / 2.0;
                at[0] += half_width; // The player's west face is flush with the block at x=2.
                game.lan.as_mut().unwrap().hub.update_guest(1, |g| {
                    g.position = at;
                    g.first_good = at;
                    g.last_reported.0 = at;
                    g.motion.as_mut().unwrap().position = Vec3::new(at[0], at[1], at[2]);
                });

                game.guest_server_tick(1, true);
                let guest = guest(game);
                let packet = correction(&rx);
                let expected_velocity = [0.0, -0.08 * f64::from(0.98_f32), 0.0].map(f64::to_bits);
                assert_eq!(packet.position.map(f64::to_bits), at.map(f64::to_bits));
                assert_eq!(packet.delta.map(f64::to_bits), expected_velocity);
                assert_eq!(guest.position.map(f64::to_bits), at.map(f64::to_bits));
                assert_eq!(
                    guest
                        .motion
                        .as_ref()
                        .unwrap()
                        .velocity
                        .components()
                        .map(f64::to_bits),
                    expected_velocity
                );
                assert_eq!(calls.load(Ordering::Relaxed), 1);
            });
        });
    }
}

#[test]
#[ignore = "daily"]
fn an_unchanged_player_travel_outcome_keeps_vanilla_reports_and_packets() {
    if run_fresh_cases(
        "game::lan_actions::guest_body::tests::travel::an_unchanged_player_travel_outcome_keeps_vanilla_reports_and_packets",
        "SUNRISE_PLAYER_TRAVEL_CHILD",
        &["travel"],
    ) {
        return;
    }
    for integrated in [true, false] {
        let mut baseline = None;
        for reply in [
            None,
            Some(Reply::Continue),
            Some(Reply::TinyInput),
            // The vector is finite, but its norm is infinite, so Vec3.normalize yields zero.
            Some(Reply::OverflowingInputLength),
        ] {
            with_reply(reply, || {
                with_player(integrated, |game, rx| {
                    report(game, ACCEPTED);
                    game.guest_server_tick(1, true);
                    report(game, NEXT);
                    let guest = guest(game);
                    assert_eq!(guest.position, NEXT);
                    assert!(guest.awaiting_teleport.is_none());
                    let sent = packets(&rx);
                    assert!(!sent.iter().any(|(id, body)| matches!(
                        C::decode(*id, body).unwrap(),
                        C::PlayerPosition(_) | C::SetEntityMotion(_)
                    )));
                    let velocity = guest.motion.as_ref().unwrap().velocity;
                    let outcome = (velocity.components().map(f64::to_bits), sent);
                    if let Some(baseline) = &baseline {
                        assert_eq!(&outcome, baseline);
                    } else {
                        baseline = Some(outcome);
                    }
                })
            });
        }
    }
}

struct MoveVeto {
    travel: Reply,
    moves: Arc<AtomicUsize>,
}

#[mod_api::__subscriptions]
impl Hooks for MoveVeto {
    fn living_travel(&self, context: &dyn LoopContext, event: &mut LivingTravelEvent<'_>) -> Flow {
        self.travel.living_travel(context, event)
    }

    fn entity_move(&self, _: &dyn LoopContext, event: &mut EntityMoveEvent<'_>) -> Flow {
        if event.kind == "minecraft:player" && event.cause == MoveCause::SelfMovement {
            self.moves.fetch_add(1, Ordering::Relaxed);
            return Flow::Cancel;
        }
        Flow::Continue
    }
}

#[test]
#[ignore = "daily"]
fn a_tiny_travel_edit_and_move_veto_leave_the_player_outcome_unchanged() {
    for travel in [Reply::Continue, Reply::TinyInput] {
        let moves = Arc::new(AtomicUsize::new(0));
        let mods = vec![mod_api::ModEntry::new(
            "travel-veto-test",
            Box::new(MoveVeto {
                travel,
                moves: moves.clone(),
            }),
        )];
        mod_platform::testing::with_mods(mods.into(), || {
            with_player(false, |game, rx| {
                game.lan.as_mut().unwrap().hub.update_guest(1, |g| {
                    g.motion.as_mut().unwrap().velocity.x = 0.5;
                });
                game.guest_server_tick(1, true);
                let g = guest(game);
                let body = g.motion.as_ref().unwrap();
                assert_eq!(
                    body.position.components().map(f64::to_bits),
                    START.map(f64::to_bits)
                );
                assert_eq!(g.position.map(f64::to_bits), START.map(f64::to_bits));
                assert_eq!(
                    body.velocity.components().map(f64::to_bits),
                    [0.5 * f64::from(0.91_f32), -0.08 * f64::from(0.98_f32), 0.0].map(f64::to_bits)
                );
                assert_eq!(moves.load(Ordering::Relaxed), 1);
                assert!(!packets(&rx).into_iter().any(|(id, body)| matches!(
                    C::decode(id, &body).unwrap(),
                    C::PlayerPosition(_) | C::SetEntityMotion(_)
                )));
            });
        });
    }
}

#[test]
#[ignore = "daily"]
fn vanilla_travel_operations_apply_large_and_non_finite_edits_independently() {
    if run_fresh_cases(
        "game::lan_actions::guest_body::tests::travel::vanilla_travel_operations_apply_large_and_non_finite_edits_independently",
        "SUNRISE_PLAYER_TRAVEL_CHILD",
        &["travel"],
    ) {
        return;
    }
    let cases = [
        (Reply::LargeInput, f64::from(0.02_f32)),
        (Reply::LargeVelocity, 70.0),
        (Reply::NonFiniteVelocityWithInput, f64::from(0.02_f32)),
        (Reply::NonFiniteInputWithVelocity, 0.5),
    ];
    for (reply, displacement_x) in cases {
        with_reply(Some(reply), || {
            with_player(false, |game, rx| {
                for cx in 1..=5 {
                    game.streaming
                        .insert_remote(&mut game.world, Chunk::empty(cx, 0));
                }
                game.guest_server_tick(1, true);
                let position = [START[0] + displacement_x, START[1], START[2]];
                let packet = correction(&rx);
                assert_eq!(packet.position, position);
                assert_eq!(guest(game).position, position);
                let expected = Vec3::new(
                    displacement_x * f64::from(0.91_f32),
                    -0.08 * f64::from(0.98_f32),
                    0.0,
                );
                assert_eq!(guest(game).motion.as_ref().unwrap().velocity, expected);
            });
        });
    }

    // A finite velocity whose swept box the collision scan cannot cover fails the travel. None of
    // its work is published: the velocity stands as it was and the player is corrected to the
    // server's position and velocity.
    with_reply(Some(Reply::HugeVelocity), || {
        with_player(false, |game, rx| {
            let before = guest(game).motion.as_ref().unwrap().velocity;
            game.guest_server_tick(1, true);
            let packet = correction(&rx);
            assert_eq!(packet.position, START);
            assert_eq!(guest(game).position, START);
            assert_eq!(guest(game).motion.as_ref().unwrap().velocity, before);
        });
    });

    with_reply(Some(Reply::NonFiniteNoOp), || {
        with_player(false, |game, rx| {
            report(game, ACCEPTED);
            game.guest_server_tick(1, true);
            report(game, NEXT);
            let guest = guest(game);
            assert_eq!(guest.position, NEXT);
            assert!(guest.awaiting_teleport.is_none());
            assert!(guest.motion.as_ref().unwrap().velocity.is_finite());
            assert!(!packets(&rx).into_iter().any(|(id, body)| matches!(
                C::decode(id, &body).unwrap(),
                C::PlayerPosition(_) | C::SetEntityMotion(_)
            )));
        });
    });

    with_reply(Some(Reply::VetoWithNonFiniteVelocity), || {
        with_player(false, |game, rx| {
            report(game, ACCEPTED);
            game.guest_server_tick(1, true);
            report(game, NEXT);
            let packet = correction(&rx);
            assert_eq!(packet.position, ACCEPTED);
            assert_eq!(guest(game).position, ACCEPTED);
        });
    });
}
