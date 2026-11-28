//! A pack's `separateAo` reaches the terrain as the game loads the pack: the pack's ids
//! (`PackIds::separate_ao`) when it loads, and the meshing settings (`Streaming::separate_ao`)
//! the frame after; and its `entity.properties` alone decides the id of an entity it lacks. Each
//! case runs in a process of its own (one event loop, its own resources).
use super::native_entity_refs::tests::{run_fresh_cases, with_client_game};
use crate::game::native_hooks::particle_count_tests::frame_scene;

const NAME: &str =
    "game::client_side::shader_pack_switch_tests::a_packs_separate_ao_reaches_the_terrain";
const CASE: &str = "SUNRISE_SHADER_PACK_SWITCH_CASE";

const VERTEX: &str = "#version 120\nvarying vec4 glcolor;\nvoid main() {\n    gl_Position = ftransform();\n    glcolor = gl_Color;\n}\n";
const FRAGMENT: &str =
    "#version 120\nvarying vec4 glcolor;\nvoid main() {\n    gl_FragData[0] = glcolor;\n}\n";

/// A pack with `separateAo = true`, picked by its path, loaded through `shader_pack_inputs`.
#[test]
#[ignore = "daily"]
fn a_packs_separate_ao_reaches_the_terrain() {
    if run_fresh_cases(NAME, CASE, &["on"]) {
        return;
    }
    let scratch = tempfile::tempdir().unwrap();
    let pack = scratch.path().join("separate-ao");
    let shaders = pack.join("shaders");
    std::fs::create_dir_all(&shaders).unwrap();
    for (name, text) in [
        ("shaders.properties", "separateAo = true\n"),
        ("gbuffers_terrain.vsh", VERTEX),
        ("gbuffers_terrain.fsh", FRAGMENT),
    ] {
        std::fs::write(shaders.join(name), text).unwrap();
    }
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

const MISSING_ENTITY: &str = "game::client_side::shader_pack_switch_tests::a_packs_missing_entity_id_follows_its_entity_file";
const MISSING_ENTITY_CASE: &str = "SUNRISE_SHADER_PACK_MISSING_ENTITY_CASE";

/// 5.3: an entity `entity.properties` lacks reads 65535, and 0 when the pack has no
/// `entity.properties`, whatever `item.properties` holds. Each case loads a pack with one of
/// the two files through `shader_pack_inputs` and reads the id of an entity span.
#[test]
#[ignore = "daily"]
fn a_packs_missing_entity_id_follows_its_entity_file() {
    if run_fresh_cases(MISSING_ENTITY, MISSING_ENTITY_CASE, &["entity", "item"]) {
        return;
    }
    let case = std::env::var(MISSING_ENTITY_CASE).unwrap();
    let (file, text, unmapped) = match case.as_str() {
        "entity" => ("entity.properties", "entity.37=minecraft:zombie\n", 65535),
        "item" => ("item.properties", "item.53=minecraft:stick\n", 0),
        other => panic!("unknown case {other}"),
    };
    let scratch = tempfile::tempdir().unwrap();
    let pack = scratch.path().join(format!("{case}-only"));
    let shaders = pack.join("shaders");
    std::fs::create_dir_all(&shaders).unwrap();
    for (name, text) in [
        ("shaders.properties", ""),
        ("gbuffers_terrain.vsh", VERTEX),
        ("gbuffers_terrain.fsh", FRAGMENT),
        (file, text),
    ] {
        std::fs::write(shaders.join(name), text).unwrap();
    }
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
        assert!(game.client.pack_ids.is_some(), "the pack loads");
        let id = |name| game.entity_part_span(0, name, false).unwrap().id;
        assert_eq!(id("minecraft:creeper"), unmapped);
        if case == "entity" {
            assert_eq!(id("minecraft:zombie"), 37);
        }
    });
}
