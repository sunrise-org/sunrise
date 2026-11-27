//! A pack's `separateAo` reaches the terrain as the game loads the pack: the pack's ids
//! (`PackIds::separate_ao`) when it loads, and the meshing settings (`Streaming::separate_ao`)
//! the frame after. The case runs in a process of its own (one event loop, its own resources).
use super::native_entity_refs::tests::{run_fresh_cases, test_window, with_client_game};
use crate::game::ClientGame;
use crate::game::model_baking_completed_tests::reload;
use crate::game::native_hooks::particle_count_tests::frame_scene;
use crate::game::packs::cloud_pack_fixture::{INNER, write_pack};
use sunrise_gameplay_common::game::Screen;

const NAME: &str =
    "game::client_side::shader_pack_switch_tests::a_packs_separate_ao_reaches_the_terrain";
const CASE: &str = "SUNRISE_SHADER_PACK_SWITCH_CASE";
const GENERATION_NAME: &str = "game::client_side::shader_pack_switch_tests::shader_pack_generation_continues_across_renderer_replacement";
const GENERATION_CASE: &str = "SUNRISE_SHADER_PACK_GENERATION_CASE";

const VERTEX: &str = "#version 120\nvarying vec4 glcolor;\nvoid main() {\n    gl_Position = ftransform();\n    glcolor = gl_Color;\n}\n";
const FRAGMENT: &str =
    "#version 120\nvarying vec4 glcolor;\nvoid main() {\n    gl_FragData[0] = glcolor;\n}\n";

fn write_shader_pack(root: &std::path::Path, properties: &str) {
    let shaders = root.join("shaders");
    std::fs::create_dir_all(&shaders).unwrap();
    for (name, text) in [
        ("shaders.properties", properties),
        ("gbuffers_terrain.vsh", VERTEX),
        ("gbuffers_terrain.fsh", FRAGMENT),
    ] {
        std::fs::write(shaders.join(name), text).unwrap();
    }
}

fn wait_for_pack(
    game: &mut ClientGame<'_>,
    expected_hides_clouds: bool,
) -> mod_api::ShaderPackState {
    let started = std::time::Instant::now();
    loop {
        assert!(game.render().unwrap());
        if let Some(state) = game.shader_pack_state() {
            assert_eq!(state.hides_clouds, expected_hides_clouds);
            return state;
        }
        assert!(
            started.elapsed().as_secs() < 30,
            "shader pack never became active"
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

/// A pack with `separateAo = true`, picked by its path, loaded through `shader_pack_inputs`.
#[test]
#[ignore = "daily"]
fn a_packs_separate_ao_reaches_the_terrain() {
    if run_fresh_cases(NAME, CASE, &["on"]) {
        return;
    }
    let scratch = tempfile::tempdir().unwrap();
    let pack = scratch.path().join("separate-ao");
    write_shader_pack(&pack, "separateAo = true\n");
    with_client_game(|game| {
        let _event_loop = frame_scene(game);
        game.client.settings.sunrise.shader_pack = pack.to_str().unwrap().to_owned();
        let sky = sunrise_level::environment::Sky::default();
        let camera = glam::DVec3::new(0.5, 65.5, 0.5);
        // The first frame turns the resource packs' material maps on and reloads them; the
        // next loads the pack.
        for _ in 0..2 {
            game.shader_pack_inputs(camera, camera, 0.0, &sky);
        }
        let ids = game.client.pack_ids.as_ref().expect("the pack loads");
        assert!(ids.separate_ao);
        game.shader_pack_inputs(camera, camera, 0.0, &sky);
        assert!(game.client.streaming.separate_ao);
    });
}

/// `ShaderPackState::generation` continues across both a same-device resource reload and a
/// replacement-device renderer adoption: switching from pack A to pack B across a new renderer
/// never collides with pack A's generation even though both are the first active pack on their
/// renderer.
#[test]
#[ignore = "daily"]
fn shader_pack_generation_continues_across_renderer_replacement() {
    if run_fresh_cases(GENERATION_NAME, GENERATION_CASE, &["reuse", "replace"]) {
        return;
    }
    let scratch = tempfile::tempdir().unwrap();
    let pack_a = scratch.path().join("pack-a");
    let pack_b = scratch.path().join("pack-b");
    write_shader_pack(&pack_a, "clouds = off\n");
    write_shader_pack(&pack_b, "");
    with_client_game(|game| {
        let (_event_loop, window) = test_window([160, 120]);
        game.common.options.offscreen = true;
        game.client.title.0 = None;
        game.client.screen = Screen::Playing;
        sunrise_client::sprites::set_labpbr_maps(true);
        let renderer = game.new_renderer(window.clone()).unwrap();
        game.set_renderer(renderer);
        game.client.window.0 = Some(window.clone());

        assert!(game.render().unwrap());
        assert_eq!(game.shader_pack_state(), None);

        game.client.settings.sunrise.shader_pack = pack_a.to_str().unwrap().to_owned();
        let state_a = wait_for_pack(game, true);
        assert!(game.render().unwrap());
        assert_eq!(game.shader_pack_state(), Some(state_a));

        let case = std::env::var(GENERATION_CASE).unwrap();
        if case == "reuse" {
            write_pack("generation_reload", INNER);
            game.client.settings.resource_packs = vec!["file/generation_reload".into()];
            reload(game);
        } else {
            game.client.renderer.0.as_mut().unwrap().wait_submissions();
            let old = game.client.renderer.0.take().unwrap();
            let carried = old.continuity();
            game.client.mod_gpu = None;
            drop(old);
            let opened = pollster::block_on(sunrise_client::gfx::Opened::for_window(
                &window,
                true,
                game.client.settings.sunrise.api,
                game.client.client_pack.render.texture_layers,
            ))
            .unwrap();
            let mut replacement = game
                .renderer_for(
                    window,
                    &game.common.pack,
                    &game.client.client_pack,
                    &game.client.gui,
                    Some(opened),
                )
                .unwrap();
            replacement.continue_from(carried);
            game.adopt_renderer(replacement);
        }

        game.client.settings.sunrise.shader_pack = pack_b.to_str().unwrap().to_owned();
        let state_b = wait_for_pack(game, false);
        assert_ne!(
            state_b.generation, state_a.generation,
            "{case}: generation collided across renderer replacement ({state_a:?} vs {state_b:?})"
        );
        assert!(game.render().unwrap());
        assert_eq!(game.shader_pack_state(), Some(state_b));

        drop(game.client.renderer.0.take());
        drop(game.client.mod_gpu.take());
    });
}
