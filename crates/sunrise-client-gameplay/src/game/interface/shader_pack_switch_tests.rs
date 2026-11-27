use super::native_entity_refs::tests::{run_fresh_cases, with_client_game};
use super::native_shaders::outcome;
use super::shader_packs::TranslationTestAction;
use crate::game::ClientGame;
use crate::game::native_hooks::particle_count_tests::frame_scene;
use std::path::Path;
use sunrise_gameplay_common::game::Dim;

const NAME: &str =
    "game::client_side::shader_pack_switch_tests::a_packs_separate_ao_reaches_the_terrain";
const CASE: &str = "SUNRISE_SHADER_PACK_SWITCH_CASE";

const VERTEX: &str = "#version 120\n#define TEST_AO\nvarying vec4 glcolor;\nvoid main() {\n    gl_Position = ftransform();\n    glcolor = gl_Color;\n}\n";
const FRAGMENT: &str =
    "#version 120\nvarying vec4 glcolor;\nvoid main() {\n    gl_FragData[0] = glcolor;\n}\n";

fn write_programs(folder: &Path) {
    std::fs::create_dir_all(folder).unwrap();
    std::fs::write(folder.join("gbuffers_terrain.vsh"), VERTEX).unwrap();
    std::fs::write(folder.join("gbuffers_terrain.fsh"), FRAGMENT).unwrap();
}

fn write_pack(pack: &Path, id: i32, properties: &str) {
    let shaders = pack.join("shaders");
    write_programs(&shaders);
    std::fs::write(shaders.join("shaders.properties"), properties).unwrap();
    std::fs::write(
        shaders.join("block.properties"),
        format!("block.{id}=stone\nlayer.translucent=oak_slab:type=bottom\n"),
    )
    .unwrap();
    std::fs::write(
        shaders.join("item.properties"),
        format!("item.{id}=stick\n"),
    )
    .unwrap();
    std::fs::write(
        shaders.join("entity.properties"),
        format!("entity.{id}=zombie\n"),
    )
    .unwrap();
}

fn frame(game: &mut ClientGame<'_>) {
    let camera = glam::DVec3::new(0.5, 65.5, 0.5);
    game.shader_pack_inputs(camera, camera, 0.0, &Default::default());
}

fn pick(game: &mut ClientGame<'_>, path: &Path) {
    game.client.settings.sunrise.shader_pack = path.to_str().unwrap().to_owned();
}

type Layout = (bool, bool, bool, bool);

fn layout(game: &ClientGame<'_>) -> Layout {
    game.client
        .renderer
        .0
        .as_ref()
        .unwrap()
        .shader_pack_section_layout()
}

fn assert_boundary(game: &ClientGame<'_>, expected: Option<(i32, bool)>, notices: u64) {
    let ids = game.client.pack_ids.as_ref();
    assert_eq!(ids.is_some(), expected.is_some());
    assert_eq!(game.client.streaming.shader_pack, expected.is_some());
    assert_eq!(
        game.client.streaming.shader_pack_lighting,
        expected.is_some()
    );
    assert_eq!(
        game.client.streaming.separate_ao,
        expected.is_some_and(|(_, ao)| ao)
    );
    if let Some((id, ao)) = expected {
        let ids = ids.expect("a translated pack publishes ids");
        let registry = game.common.pack.assets.registry;
        let stone = registry.default_state("minecraft:stone").unwrap();
        assert_eq!(ids.block_state_id(stone.0 as usize), Some(id));
        assert_eq!(ids.items.get("minecraft:stick"), Some(&id));
        assert_eq!(ids.entities.get("minecraft:zombie"), Some(&id));
        assert_eq!(ids.separate_ao, ao);
        assert!(
            game.client
                .streaming
                .layer_overrides
                .contains(&Some(sunrise_shaders::RenderLayer::Translucent))
        );
    } else {
        assert!(game.client.streaming.layer_overrides.is_empty());
    }
    assert_eq!(game.client.chat_ui.chat.received, notices);
}

fn finish_translation(
    game: &mut ClientGame<'_>,
    expected: (i32, bool),
    notices: u64,
    folder: &str,
) {
    game.client
        .native_shaders
        .translation
        .wait_for_translation();
    game.assert_shader_translation_matches_preflight(folder);
    frame(game);
    assert_boundary(game, Some(expected), notices);
    let building = layout(game);
    let renderer = game.client.renderer.0.as_mut().unwrap();
    assert!(renderer.shader_pack_build_pending());
    assert_eq!(renderer.shader_pack_kind(), None);
    renderer.wait_shader_pack_build_for_test();
    assert_boundary(game, Some(expected), notices);
    assert_eq!(layout(game), building);
    frame(game);
    assert_boundary(game, Some(expected), notices);
    assert_eq!(
        game.client.renderer.0.as_ref().unwrap().shader_pack_kind(),
        Some("iris")
    );
    assert_eq!(layout(game), building);
    assert_eq!(
        outcome(Path::new(&game.client.settings.sunrise.shader_pack)),
        Some(true)
    );
}

#[test]
#[ignore = "daily"]
fn a_packs_separate_ao_reaches_the_terrain() {
    if run_fresh_cases(
        NAME,
        CASE,
        &[
            "on",
            "switch",
            "late-failure",
            "off",
            "options",
            "dimensions",
            "resources",
            "renderer",
        ],
    ) {
        return;
    }
    let case = std::env::var(CASE).unwrap();
    let scratch = tempfile::tempdir().unwrap();
    let a = scratch.path().join("first");
    let b = scratch.path().join("second");
    write_pack(&a, 41, "separateAo = true\n");
    write_pack(&b, 72, "separateAo = false\n");
    if case == "options" {
        std::fs::write(
            a.join("shaders/shaders.properties"),
            "#ifdef TEST_AO\nseparateAo = true\n#else\nseparateAo = false\n#endif\n",
        )
        .unwrap();
    }
    if case == "dimensions" {
        let shaders = a.join("shaders");
        std::fs::write(
            shaders.join("dimension.properties"),
            "dimension.first = test:first\ndimension.second = test:second\n",
        )
        .unwrap();
        write_programs(&shaders.join("first"));
        write_programs(&shaders.join("second"));
    }
    with_client_game(|game| {
        sunrise_client::sprites::set_labpbr_maps(true);
        let event_loop = frame_scene(game);
        frame(game);
        let notices = game.client.chat_ui.chat.received;
        let vanilla = layout(game);
        assert_boundary(game, None, notices);

        if case == "dimensions" {
            game.common.dim = Dim::Extra(mod_api::__private::dimension_key("test:first"));
        }
        pick(game, &a);
        let gate = game.client.native_shaders.translation.block_next();
        frame(game);
        assert!(
            game.client
                .renderer
                .0
                .as_ref()
                .unwrap()
                .is_translating_shader_pack()
        );
        assert_boundary(game, None, notices);
        assert_eq!(layout(game), vanilla);

        match case.as_str() {
            "on" | "switch" | "off" => {
                gate.send(TranslationTestAction::Complete).unwrap();
                finish_translation(game, (41, true), notices, "");
                if case == "on" {
                    return;
                }
                let old_layout = layout(game);
                pick(game, &b);
                let next = game.client.native_shaders.translation.block_next();
                frame(game);
                assert_boundary(game, Some((41, true)), notices);
                assert_eq!(layout(game), old_layout);
                if case == "switch" {
                    next.send(TranslationTestAction::Complete).unwrap();
                    finish_translation(game, (72, false), notices, "");
                } else {
                    game.client.settings.sunrise.shader_pack.clear();
                    frame(game);
                    assert_boundary(game, None, notices);
                    assert_eq!(layout(game), vanilla);
                    next.send(TranslationTestAction::Fail).unwrap();
                    game.client
                        .native_shaders
                        .translation
                        .wait_for_translation();
                    frame(game);
                    assert!(game.client.settings.sunrise.shader_pack.is_empty());
                    assert_eq!(outcome(&b), None);
                    assert_boundary(game, None, notices);
                    assert_eq!(layout(game), vanilla);
                }
                return;
            }
            "late-failure" => {
                pick(game, &b);
                frame(game);
                assert_boundary(game, None, notices);
                assert_eq!(layout(game), vanilla);
                gate.send(TranslationTestAction::Fail).unwrap();
            }
            "options" => {
                game.open_shader_options();
                game.client
                    .settings
                    .shader_options
                    .as_mut()
                    .unwrap()
                    .chosen
                    .insert("TEST_AO".into(), "false".into());
                game.save_shader_options();
                gate.send(TranslationTestAction::Complete).unwrap();
            }
            "dimensions" => {
                game.common.dim = Dim::Extra(mod_api::__private::dimension_key("test:second"));
                gate.send(TranslationTestAction::Complete).unwrap();
            }
            "resources" => {
                game.reload_packs();
                frame(game);
                assert_boundary(game, None, notices);
                assert_eq!(layout(game), vanilla);
                gate.send(TranslationTestAction::Fail).unwrap();
                let mut reload = game
                    .client
                    .pack_reload
                    .0
                    .take()
                    .expect("resource reload started");
                let staged = reload.handle.take().unwrap().join().unwrap().unwrap();
                game.apply_staged_pack(staged).unwrap();
            }
            "renderer" => {
                #[allow(deprecated)]
                let window = event_loop
                    .create_window(
                        winit::window::Window::default_attributes()
                            .with_visible(false)
                            .with_inner_size(winit::dpi::PhysicalSize::new(320, 240)),
                    )
                    .unwrap();
                let renderer = game.new_renderer(std::sync::Arc::new(window)).unwrap();
                game.set_renderer(renderer);
                gate.send(TranslationTestAction::Fail).unwrap();
            }
            _ => panic!("unknown shader pack case {case}"),
        }
        game.client
            .native_shaders
            .translation
            .wait_for_translation();
        frame(game);
        assert_boundary(game, None, notices);
        assert_eq!(layout(game), vanilla);
        assert_eq!(
            game.client.settings.sunrise.shader_pack,
            if case == "late-failure" {
                b.to_str().unwrap()
            } else {
                a.to_str().unwrap()
            }
        );
        assert_eq!(outcome(&a), None);
        let expected = match case.as_str() {
            "late-failure" => (72, false),
            "options" => (41, false),
            _ => (41, true),
        };
        finish_translation(
            game,
            expected,
            notices,
            if case == "dimensions" { "second" } else { "" },
        );
    });
}
