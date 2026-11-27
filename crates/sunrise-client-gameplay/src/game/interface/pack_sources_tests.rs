//! The section arena keeps its quads' sources while a `shaders/` pack's meshes carry them, and
//! none once the pack is dropped (`Renderer::keep_pack_sources`), as the game loads, drops and
//! loads again a pack. The case runs in a process of its own (one event loop, its own resources).
use super::native_entity_refs::tests::{run_fresh_cases, with_client_game};
use crate::game::native_hooks::particle_count_tests::frame_scene;

const NAME: &str =
    "game::client_side::pack_sources_tests::terrain_sources_are_kept_only_while_a_pack_is_loaded";
const CASE: &str = "SUNRISE_PACK_SOURCES_CASE";

const VERTEX: &str = "#version 120\nattribute vec4 mc_Entity;\nvarying vec4 glcolor;\nvoid main() {\n    gl_Position = ftransform();\n    glcolor = vec4(mc_Entity.x);\n}\n";
const FRAGMENT: &str =
    "#version 120\nvarying vec4 glcolor;\nvoid main() {\n    gl_FragData[0] = glcolor;\n}\n";

#[test]
#[ignore = "daily"]
fn terrain_sources_are_kept_only_while_a_pack_is_loaded() {
    if run_fresh_cases(NAME, CASE, &["on"]) {
        return;
    }
    let scratch = tempfile::tempdir().unwrap();
    let pack = scratch.path().join("ids");
    let shaders = pack.join("shaders");
    std::fs::create_dir_all(&shaders).unwrap();
    for (name, text) in [
        ("block.properties", "block.10=stone\n"),
        ("gbuffers_terrain.vsh", VERTEX),
        ("gbuffers_terrain.fsh", FRAGMENT),
    ] {
        std::fs::write(shaders.join(name), text).unwrap();
    }
    with_client_game(|game| {
        sunrise_client::sprites::set_labpbr_maps(true);
        let _event_loop = frame_scene(game);
        let sky = sunrise_level::environment::Sky::default();
        let camera = glam::DVec3::new(0.5, 65.5, 0.5);
        let renderer = game.client.renderer.0.as_mut().expect("a renderer");
        // Flat draws are what read the sources: a device without them keeps none, and has
        // nothing for this case to check.
        if renderer.set_flat_draws(true).is_err() {
            eprintln!("skipped: {NAME}: this adapter has no flat section draws");
            return;
        }
        assert!(!renderer.keeps_pack_sources(), "no pack yet");
        let kept = |game: &crate::game::ClientGame<'_>| {
            game.client
                .renderer
                .0
                .as_ref()
                .unwrap()
                .keeps_pack_sources()
        };
        let frame = |game: &mut crate::game::ClientGame<'_>| {
            game.shader_pack_inputs(camera, camera, 0.0, &sky);
        };
        game.client.settings.sunrise.shader_pack = pack.to_str().unwrap().to_owned();
        frame(game);
        assert!(!kept(game), "translating");
        game.client
            .native_shaders
            .translation
            .wait_for_translation();
        frame(game);
        assert!(kept(game), "translated, while the pipelines build");
        assert!(game.client.streaming.shader_pack);
        game.client
            .renderer
            .0
            .as_mut()
            .unwrap()
            .wait_shader_pack_build_for_test();
        frame(game);
        assert!(kept(game), "installed");
        game.client.settings.sunrise.shader_pack.clear();
        frame(game);
        assert!(!kept(game), "dropped");
        assert!(!game.client.streaming.shader_pack);
        game.client.settings.sunrise.shader_pack = pack.to_str().unwrap().to_owned();
        frame(game);
        game.client
            .native_shaders
            .translation
            .wait_for_translation();
        frame(game);
        assert!(kept(game), "loaded again");
        game.client
            .renderer
            .0
            .as_mut()
            .unwrap()
            .wait_shader_pack_build_for_test();
    });
}
