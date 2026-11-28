//! The modifier hook reads the effective camera entity, not the local spectator.
use crate::game::ClientGame as Game;
use crate::game::native_entity_refs::tests::with_client_game;
use mod_api::{
    ClientHost, ComputeFovEvent, ComputeFovModifierEvent, Hooks, ModEntry,
    client::camera::CameraType,
};
use std::sync::{Arc, Mutex};
use sunrise_client::{mouse::MouseLook, options_file::V as OptV};
use sunrise_core::geometry::Vec3;
use sunrise_items::items::ItemStack;
use sunrise_protocol::{
    connection::Pipe,
    packets::play::{self, Clientbound as C},
};
use sunrise_sim::motion::PlayerMotion;

/// The player UUID, raw modifier, effect scale and starting interpolation seen by the hook.
type Calls = Arc<Mutex<Vec<(u128, f32, f32, f32)>>>;

struct Modifier(Calls);

#[mod_api::__subscriptions]
impl Hooks for Modifier {
    fn compute_fov_modifier(&self, _: &dyn ClientHost, event: &mut ComputeFovModifierEvent) {
        self.0.lock().unwrap().push((
            event.player.uuid(),
            event.raw,
            event.effect_scale,
            event.modifier,
        ));
        event.modifier = 0.5;
    }
}

const SPECTATED: u128 = 0x31;

fn with_spectator(kind: &str, test: impl FnOnce(&mut Game<'_>, &Calls)) {
    let calls = Calls::default();
    let mods = vec![ModEntry::new(
        "camera_fov",
        Box::new(Modifier(calls.clone())),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_client_game(|game| {
            let (client, _server) = Pipe::pair();
            game.connect_local(client, "CameraFovTest").unwrap();
            game.client.remote.as_mut().unwrap().entity_id = 7;
            game.client
                .remote
                .as_mut()
                .unwrap()
                .level_players
                .insert(31, SPECTATED);
            let mut player = PlayerMotion::new(game.motion, Vec3::ZERO);
            player.flying = true;
            player.speed = 0.4;
            game.common.player = Some(player);
            game.spawn_entity(31, kind, [10.0, 64.0, 12.0], [0.0; 3]);
            game.handle_packet(C::SetCamera(play::SetCamera { camera_id: 31 }))
                .unwrap();
            test(game, &calls);
        });
    });
}

#[test]
#[ignore = "daily"]
fn spyglass_mouse_sensitivity_only_applies_in_first_person() {
    // In a child process running only this test: the boot fixture fills the process-wide callback
    // tables, which no other test may inherit.
    if std::env::var_os("SUNRISE_FOV_CAMERA_CHILD").is_none() {
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "game::client_side::fov::camera_tests::spyglass_mouse_sensitivity_only_applies_in_first_person",
                "--include-ignored",
                "--nocapture",
            ])
            .env("SUNRISE_FOV_CAMERA_CHILD", "1")
            .status()
            .unwrap();
        assert!(status.success());
        return;
    }
    with_client_game(|game| {
        game.common.using = Some((0, 0));
        game.common.use_start.item = Some(ItemStack::new("minecraft:spyglass", 1));
        game.client.smooth_camera = false;
        game.client.settings.sensitivity = 0.5;
        game.client.settings.invert_x = false;
        game.client.settings.invert_y = false;
        game.apply_camera_type(CameraType::FirstPerson);

        // `MouseHandler.turnPlayer` at sensitivity 0.5 with the float constants 0.6F and 0.2F:
        // `ss = 0.5 * 0.6F + 0.2F`, `sensitivityMod = ss^3`, `sens = sensitivityMod * 8`.
        // Scoping turns by `sensitivityMod`, otherwise by `sens`. Expected values computed
        // independently in double arithmetic for the motion (2, -3).
        let mut first_person_mouse = MouseLook::default();
        first_person_mouse.add_motion(2.0, -3.0);
        let first_person_turn = first_person_mouse
            .handle_accumulated_movement(1.0, true, true, game.look_options())
            .unwrap();
        assert!(game.look_options().scoping);
        assert_eq!(
            first_person_turn,
            (0.25000002235174246, -0.3750000335276137)
        );

        game.apply_camera_type(CameraType::ThirdPersonBack);
        let mut third_person_mouse = MouseLook::default();
        third_person_mouse.add_motion(2.0, -3.0);
        let third_person_turn = third_person_mouse
            .handle_accumulated_movement(1.0, true, true, game.look_options())
            .unwrap();
        assert!(!game.look_options().scoping);
        assert_eq!(third_person_turn, (2.0000001788139397, -3.0000002682209095));
    });
}

fn sync_spectated_movement_speed(game: &mut Game<'_>) {
    let attribute = game
        .motion
        .attributes()
        .id("minecraft:movement_speed")
        .unwrap()
        .registry_id();
    game.handle_packet(C::UpdateAttributes(play::UpdateAttributes {
        id: 31,
        values: vec![play::AttributeSnapshot {
            attribute,
            base: 0.1,
            modifiers: vec![play::AttributeModifier {
                id: "test:movement_bonus".into(),
                amount: 0.1,
                operation: 0,
            }],
        }],
    }))
    .unwrap();
}

#[test]
#[ignore = "daily"]
fn spectated_player_fov_uses_their_own_attributes_and_item_use() {
    with_spectator("minecraft:player", |game, calls| {
        sync_spectated_movement_speed(game);
        let m = game
            .common
            .mob_index
            .get_mut(&mut game.common.mobs, 31)
            .unwrap();
        m.extra.sprinting = true;
        m.extra.using_item = 11;
        m.extra.using_off_hand = true;
        m.set_equipment_stack(
            sunrise_items::equipment::OFFHAND,
            Some(ItemStack::new("minecraft:spyglass", 1)),
        );

        game.client.view.fov_modifier = 1.0;
        game.client.camera_type = CameraType::FirstPerson;
        game.tick_fov();
        assert_eq!(
            game.client.view.fov_modifier, 0.55,
            "ease halfway toward the 0.1 first-person spyglass target"
        );
        assert!(
            calls.lock().unwrap().is_empty(),
            "first-person spyglass returns 0.1 before compute_fov_modifier"
        );

        game.client.camera_type = CameraType::ThirdPersonBack;
        game.tick_fov();
        assert_eq!(
            game.client.view.fov_modifier, 0.525,
            "ease halfway toward the mod's 0.5"
        );
        // Vanilla: (0.2 / 0.1 + 1) / 2 in third person; the packet's speed is
        // used directly and the sprint flag adds nothing to it. The listener is told the
        // spectated player, the raw modifier, the scale, and the interpolation it starts from.
        assert_eq!(*calls.lock().unwrap(), [(SPECTATED, 1.5, 1.0, 1.5)]);
    });
}

#[test]
#[ignore = "daily"]
fn at_effect_scale_zero_the_listener_still_sees_the_raw_modifier() {
    with_spectator("minecraft:player", |game, calls| {
        sync_spectated_movement_speed(game);
        game.client
            .settings
            .file
            .set("fovEffectScale", OptV::Double(0.0));
        game.client.camera_type = CameraType::ThirdPersonBack;
        game.tick_fov();
        // Mth.lerp(0, 1, 1.5) is 1: the start loses the raw value, the event keeps it.
        assert_eq!(*calls.lock().unwrap(), [(SPECTATED, 1.5, 0.0, 1.0)]);
    });
}

#[test]
#[ignore = "daily"]
fn spectated_non_player_fov_does_not_dispatch_the_player_hook() {
    with_spectator("minecraft:creeper", |game, calls| {
        game.client.view.fov_modifier = 0.5;
        game.tick_fov();
        assert_eq!(
            game.client.view.fov_modifier, 0.75,
            "ease halfway to neutral 1.0"
        );
        assert!(calls.lock().unwrap().is_empty());
    });
}

/// Narrows or widens the world angle, as a `compute_fov` listener would.
struct Projection(Arc<Mutex<Option<f32>>>);

#[mod_api::__subscriptions]
impl Hooks for Projection {
    fn compute_fov(&self, _: &dyn ClientHost, event: &mut ComputeFovEvent) {
        if let Some(value) = *self.0.lock().unwrap() {
            event.degrees = value;
        }
    }
}

/// The effective angle accepts a valid listener answer and keeps vanilla otherwise.
#[test]
#[ignore = "daily"]
fn the_world_angle_takes_valid_listener_values_and_ignores_the_rest() {
    let answer = Arc::new(Mutex::new(None));
    let mods = vec![ModEntry::new(
        "projection",
        Box::new(Projection(answer.clone())),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_client_game(|game| {
            let setting = game.client.settings.fov;
            game.client.view.old_fov_modifier = 1.0;
            game.client.view.fov_modifier = 1.0;
            for (value, expected) in [
                (None, setting),
                (Some(15.0), 15.0),
                (Some(120.0), 120.0),
                (Some(1.0), 1.0),
                (Some(179.0), 179.0),
                (Some(0.0), setting),
                (Some(180.0), setting),
                (Some(-1.0), setting),
                (Some(f32::NAN), setting),
                (Some(f32::INFINITY), setting),
            ] {
                *answer.lock().unwrap() = value;
                assert_eq!(
                    game.fov_degrees(0.25, [1.0, 2.0, 3.0]),
                    expected,
                    "listener answered {value:?}"
                );
            }
        });
    });
}
