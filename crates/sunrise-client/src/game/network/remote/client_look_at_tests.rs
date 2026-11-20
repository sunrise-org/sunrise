//! `ClientPacketListener.handleLookAt` against pinned 26.3 bits: the real `Entity.lookAt(EYES, to)`
//! run on an entity standing at (0.5, 64, 0.5) with the player's 1.62 eye height, towards each
//! anchor below. Every row is one where `look_at_rotation`'s current degree conversion and
//! vanilla's `Mth.RAD_TO_DEG` multiply agree, so these pin the anchor and the snap, not #3246.
use crate::game::{
    Game, native_entity_refs::tests::with_client_game, native_lifecycle::NativeLifecycle,
};
use sunrise_client::{level_loading::Reason, motion::PlayerMotion};
use sunrise_core::geometry::Vec3;
use sunrise_protocol::{
    connection::Pipe,
    packets::play::{self, Clientbound as C, SpawnInfo},
};

const EYES: i32 = 1;
const FEET: i32 = 0;

/// A logged-in client whose player stands at (0.5, 64, 0.5) looking at yaw 45, pitch 20, its
/// third-person model's head too.
fn client_player(game: &mut Game<'_>) {
    game.native_lifecycle = NativeLifecycle::default();
    let (client, _server) = Pipe::pair();
    game.connect_local(client, "LookAtTest").unwrap();
    let spawn = SpawnInfo {
        dimension_type: 0,
        dimension: "minecraft:overworld".into(),
        seed: 0,
        game_type: 1,
        previous_game_type: None,
        debug: false,
        flat: true,
        last_death: None,
        portal_cooldown: 0,
        sea_level: 63,
    };
    game.enter_remote_dimension(&spawn, false, Reason::Other);
    let remote = game.remote.as_mut().unwrap();
    remote.logged_in = true;
    remote.entity_id = 7;
    game.player = Some(PlayerMotion::new(game.motion, Vec3::new(0.5, 64.0, 0.5)).unwrap());
    game.tick_player_model();
    turn_away(game);
}

fn turn_away(game: &mut Game<'_>) {
    (game.yaw, game.pitch) = (45.0, 20.0);
    game.rope_rot_old = (45.0, 20.0);
    let player = game.player.as_mut().unwrap();
    (player.yaw, player.head_yaw) = (45.0, 45.0);
    let model = game.player_model.as_mut().unwrap();
    (model.head_pitch, model.previous_head_pitch) = (20.0, 20.0);
}

fn add(game: &mut Game<'_>, id: i32, kind: &str, at: [f64; 3], data: i32) {
    let kind = game
        .remote
        .as_ref()
        .unwrap()
        .builtin
        .get("minecraft:entity_type")
        .unwrap()
        .id(kind)
        .unwrap();
    game.handle_packet(C::AddEntity(play::AddEntity {
        id,
        uuid: (100 + id) as _,
        kind,
        position: at,
        velocity: [0.0; 3],
        pitch: 0,
        yaw: 0,
        head_yaw: 0,
        data,
    }))
    .unwrap();
}

/// Look at `entity`'s `anchor` (or `target` without one) from the player's eyes, and the yaw and
/// pitch bits every snapped endpoint then holds, asserting they all agree.
fn look_at(game: &mut Game<'_>, target: [f64; 3], entity: Option<(i32, i32)>) -> (u32, u32) {
    turn_away(game);
    game.handle_packet(C::PlayerLookAt(play::PlayerLookAt {
        from: EYES,
        target,
        entity: entity.map(|(id, anchor)| play::LookAtEntity { id, anchor }),
    }))
    .unwrap();
    let (yaw, pitch) = (game.yaw.to_bits(), game.pitch.to_bits());
    let bits = |(a, b): (f32, f32)| (a.to_bits(), b.to_bits());
    assert_eq!(bits(game.rope_rot_old), (yaw, pitch), "yRotO/xRotO");
    let player = game.player.as_ref().unwrap();
    assert_eq!(
        bits((player.yaw, player.head_yaw)),
        (yaw, yaw),
        "yRot and yHeadRot"
    );
    let model = game.player_model.as_ref().unwrap();
    assert_eq!(
        bits((model.previous_head_pitch, model.head_pitch)),
        (pitch, pitch),
        "the third-person model's pitch, both ends"
    );
    (yaw, pitch)
}

#[test]
#[ignore = "daily"]
fn look_at_aims_at_the_clients_live_copy_and_snaps_every_old_angle() {
    with_client_game(|game| {
        client_player(game);
        // Player 8's hook: the server's last packet put it at (4.5, 64, 0.5); the
        // client's copy has since moved on, as a hook does between packets.
        add(game, 40, "minecraft:fishing_bobber", [4.5, 64.0, 0.5], 8);
        let hook = game.remote_hooks.iter_mut().find(|h| h.id() == 40).unwrap();
        hook.position = Vec3::new(2.25, 65.375, -1.75);
        assert_eq!(
            game.remote.as_ref().unwrap().tracked_entity(40).unwrap().1,
            [4.5, 64.0, 0.5],
            "the tracked row still holds the packet's position"
        );

        let packet_point = [-1.5, 66.0, 4.0];
        assert_eq!(
            look_at(game, packet_point, Some((40, FEET))),
            (0xc30e2002, 0x409d3409),
            "the hook's live feet, not its row or the packet's point"
        );
        // Anchor.EYES: the bobber's 0.2125F eye height on its live y.
        let eyes = game.look_at_anchor(40, true).unwrap();
        assert_eq!(eyes[1].to_bits(), 0x4050659999a00000);
        assert_eq!(
            look_at(game, packet_point, Some((40, EYES))),
            (0xc30e2002, 0x3f273b6b)
        );

        // No entity 99 here: the packet's point.
        assert_eq!(
            look_at(game, packet_point, Some((99, FEET))),
            (0x41edf580, 0xc0ac531e)
        );

        // A mob's eyes: getEyeY's y + 1.445F at a y the subtraction would not round-trip.
        add(game, 41, "minecraft:creeper", [3.0, 64.1, 2.0], 0);
        let eyes = game.look_at_anchor(41, true).unwrap();
        assert_eq!(
            eyes.map(f64::to_bits),
            [3.0f64.to_bits(), 0x405062e147e66666, 2.0f64.to_bits()]
        );
        assert_eq!(
            look_at(game, packet_point, Some((41, EYES))),
            (0xc26c2510, 0x3fbca0e5)
        );
    });
}
