//! The camera read after leaving a world (#6731): no level, no camera.
use super::native_entity_refs::tests::with_client_game;
use glam::Mat4;
use sunrise_protocol::packets::play::{self, Clientbound as C};

fn camera() -> mod_api::Camera {
    mod_api::Camera {
        position: [100.0, 64.0, 100.0],
        eye_height: 1.62,
        partial_tick: 0.0,
        yaw: 0.0,
        pitch: 0.0,
        roll: 0.0,
        fov_degrees: 70.0,
        near: 0.05,
        far: 128.0,
        view: Mat4::IDENTITY.to_cols_array(),
        projection: Mat4::IDENTITY.to_cols_array(),
        dimension: "minecraft:overworld".into(),
    }
}

fn spawn() -> play::SpawnInfo {
    play::SpawnInfo {
        dimension_type: 0,
        dimension: "minecraft:overworld".into(),
        seed: 1,
        game_type: 0,
        previous_game_type: None,
        debug: false,
        flat: false,
        last_death: None,
        portal_cooldown: 0,
        sea_level: 63,
    }
}

/// A populated camera, then the server left: the camera reads `None`, and stays `None` through
/// the title frame that follows (it extracts no camera), until the next world's first frame.
#[test]
#[ignore = "daily"]
fn leaving_a_server_leaves_no_camera_behind() {
    with_client_game(|game| {
        let (pipe, _server) = sunrise_protocol::connection::Pipe::pair();
        game.connect_local(pipe, "Steve").unwrap();
        game.handle_packet(C::Login(play::Login {
            player_id: 7,
            hardcore: false,
            levels: vec!["minecraft:overworld".into()],
            max_players: 20,
            chunk_radius: 8,
            simulation_distance: 8,
            reduced_debug_info: false,
            show_death_screen: true,
            limited_crafting: false,
            spawn: spawn(),
            online_mode: false,
            enforces_secure_chat: false,
        }))
        .unwrap();
        game.handle_packet(C::PlayerPosition(play::PlayerPosition {
            teleport: 1,
            position: [0.5, 65.0, 0.5],
            delta: [0.0; 3],
            yaw: 0.0,
            pitch: 0.0,
            relatives: 0,
        }))
        .unwrap();
        assert!(game.common.player.is_some(), "in a world");
        game.client.native_camera_frame = Some(camera());
        assert_eq!(
            game.camera().map(|c| c.position),
            Some([100.0, 64.0, 100.0])
        );

        game.leave_server(None).unwrap();
        assert!(game.common.player.is_none(), "out of the world");
        assert!(game.camera().is_none());
        assert!(
            game.client.native_camera_frame.is_none(),
            "the snapshot went too"
        );
    });
}

/// With no level, a snapshot left from any path still reads `None`.
#[test]
#[ignore = "daily"]
fn no_level_reads_no_camera() {
    with_client_game(|game| {
        game.common.player = None;
        game.client.native_camera_frame = Some(camera());
        assert!(game.camera().is_none());
    });
}
