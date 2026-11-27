use super::native_entity_refs::tests::{run_fresh_cases, with_client_game};
use super::native_shaders::outcome;
use super::shader_packs::TranslationTestAction;
use crate::game::ClientGame;
use crate::game::native_hooks::particle_count_tests::frame_scene;
use std::path::Path;
use sunrise_client_render::gfx::{Renderer, ShaderPackBuildTestMode};
use sunrise_gameplay_common::game::Dim;
use sunrise_mod_package::mod_list::{Installed, NativePage};

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

fn renderer<'a>(game: &'a ClientGame<'_>) -> &'a Renderer {
    game.client.renderer.0.as_deref().unwrap()
}

fn renderer_mut<'a>(game: &'a mut ClientGame<'_>) -> &'a mut Renderer {
    game.client.renderer.0.as_deref_mut().unwrap()
}

fn stone_index(game: &ClientGame<'_>) -> usize {
    game.common
        .pack
        .assets
        .registry
        .default_state("minecraft:stone")
        .unwrap()
        .0 as usize
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
    let stone = game
        .common
        .pack
        .assets
        .registry
        .default_state("minecraft:stone")
        .unwrap()
        .0 as usize;
    let previous = game
        .client
        .renderer
        .0
        .as_ref()
        .unwrap()
        .installed_pack_block_id_for_test(stone);
    game.client
        .native_shaders
        .translation
        .wait_for_translation();
    game.assert_shader_translation_matches_preflight(folder);
    frame(game);
    assert_boundary(game, Some(expected), notices);
    let building = layout(game);
    let renderer = game.client.renderer.0.as_mut().unwrap();
    assert_eq!(renderer.installed_pack_block_id_for_test(stone), previous);
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
    assert_eq!(
        game.client
            .renderer
            .0
            .as_ref()
            .unwrap()
            .installed_pack_block_id_for_test(stone),
        Some(expected.0)
    );
    assert_eq!(layout(game), building);
    assert_eq!(
        outcome(Path::new(&game.client.settings.sunrise.shader_pack)),
        Some(true)
    );
}

fn upload_test_section(game: &mut ClientGame<'_>) {
    use sunrise_client_render::mesher::{SectionMesh, TerrainSource, TerrainVertex};

    let mut mesh = SectionMesh::default();
    mesh.solid.extend(
        [[0., 1., 0.], [0., 1., 1.], [1., 1., 1.], [1., 1., 0.]]
            .into_iter()
            .zip([[0., 0.], [0., 1.], [1., 1.], [1., 0.]])
            .map(|(pos, uv)| TerrainVertex::new(pos, uv, 0, u32::MAX, 0xf0f0)),
    );
    mesh.solid_sources.push(TerrainSource::new(
        stone_index(game) as u32,
        [0, 0, 0],
        0,
        false,
    ));
    mesh.sort_solid();
    renderer_mut(game).upload((0, 4, 0), &mesh);
    assert_eq!(renderer(game).resident_sections(), 1);
}

fn assert_running_pack(game: &ClientGame<'_>, old_layout: Layout, revision: u64, notices: u64) {
    assert_boundary(game, Some((41, true)), notices);
    assert_eq!(layout(game), old_layout);
    assert_eq!(renderer(game).block_entity_revision(), revision);
    assert_eq!(renderer(game).shader_pack_kind(), Some("iris"));
    assert_eq!(
        renderer(game).installed_pack_block_id_for_test(stone_index(game)),
        Some(41)
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
            "immediate",
            "cancel-build",
            "refuse-build",
            "gpu-spawn-failure",
            "mod-off",
            "open-failure",
            "translation-panic",
            "translation-spawn-failure",
            "cancel-translation",
            "disconnect",
            "fingerprint",
        ],
    ) {
        return;
    }
    let case = std::env::var(CASE).unwrap();
    let scratch = tempfile::tempdir().unwrap();
    let a = scratch.path().join("native-first");
    let b = scratch.path().join("second");
    write_pack(&a, 41, "separateAo = true\n");
    write_pack(&b, 72, "separateAo = false\nbufferObject.0 = 64\n");
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
        renderer_mut(game).set_flat_draws(false).unwrap();
        renderer_mut(game).set_packed_layout(true);
        frame(game);
        let notices = game.client.chat_ui.chat.received;
        let vanilla = layout(game);
        assert_boundary(game, None, notices);

        if case == "dimensions" {
            game.common.dim = Dim::Extra(mod_api::__private::dimension_key("test:first"));
        }
        if case == "mod-off" {
            game.client.settings.mods.installed.push(Installed {
                id: "first".into(),
                title: "First".into(),
                description: String::new(),
                problem: None,
                dev: false,
                native: Some(NativePage::default()),
            });
        }
        game.client.native_shaders.translation.test_lava_layers = Some(vec![1, 2]);
        pick(game, &a);
        if case == "immediate" {
            game.client.native_shaders.translation.test_immediate = true;
            renderer_mut(game).set_shader_pack_build_test_mode(ShaderPackBuildTestMode::Immediate);
            frame(game);
            assert_boundary(game, None, notices);
            assert_eq!(layout(game), vanilla);
            assert!(game.client.native_shaders.translation.is_translating());
            assert!(!renderer(game).shader_pack_build_pending());
            frame(game);
            assert_boundary(game, Some((41, true)), notices);
            assert!(renderer(game).shader_pack_build_ready());
            assert_eq!(renderer(game).shader_pack_kind(), None);
            assert_eq!(
                renderer(game).installed_pack_block_id_for_test(stone_index(game)),
                None
            );
            frame(game);
            assert_boundary(game, Some((41, true)), notices);
            assert_eq!(renderer(game).shader_pack_kind(), Some("iris"));
            assert_eq!(
                renderer(game).installed_pack_block_id_for_test(stone_index(game)),
                Some(41)
            );
            assert_eq!(outcome(&a), Some(true));
            return;
        }
        let gate = game.client.native_shaders.translation.block_next();
        frame(game);
        assert!(game.client.native_shaders.translation.is_translating());
        assert_boundary(game, None, notices);
        assert_eq!(layout(game), vanilla);

        let mut translating_layout = vanilla;
        match case.as_str() {
            "on"
            | "switch"
            | "off"
            | "renderer"
            | "cancel-build"
            | "refuse-build"
            | "gpu-spawn-failure"
            | "open-failure"
            | "translation-spawn-failure"
            | "translation-panic"
            | "cancel-translation"
            | "disconnect"
            | "fingerprint" => {
                gate.send(TranslationTestAction::Complete).unwrap();
                finish_translation(game, (41, true), notices, "");
                if case == "on" {
                    return;
                }
                let old_layout = layout(game);
                assert_ne!(
                    old_layout, vanilla,
                    "per-section shader terrain changes the layout"
                );
                upload_test_section(game);
                let old_inputs = renderer(game).shader_pack_inputs_for_test();
                let old_revision = renderer(game).block_entity_revision();
                let sections = renderer(game).resident_sections();
                assert!(old_inputs.0);
                assert_eq!(old_inputs.1, Some(true));
                assert_eq!(old_inputs.2, [1, 2]);
                game.client.native_shaders.translation.test_lava_layers = Some(vec![3, 4]);
                pick(game, &b);
                let next = match case.as_str() {
                    "open-failure" => {
                        std::fs::remove_dir_all(&b).unwrap();
                        None
                    }
                    "translation-spawn-failure" => {
                        game.client.native_shaders.translation.test_spawn_failure = true;
                        None
                    }
                    _ => Some(game.client.native_shaders.translation.block_next()),
                };
                frame(game);
                assert_running_pack(game, old_layout, old_revision, notices);
                assert_eq!(renderer(game).resident_sections(), sections);
                assert_eq!(renderer(game).shader_pack_inputs_for_test(), old_inputs);
                match case.as_str() {
                    "switch" => {
                        next.unwrap().send(TranslationTestAction::Complete).unwrap();
                        finish_translation(game, (72, false), notices, "");
                        let inputs = renderer(game).shader_pack_inputs_for_test();
                        assert!(!inputs.0);
                        assert_eq!(inputs.1, Some(false));
                        assert_eq!(inputs.2, [3, 4]);
                        assert_eq!(inputs.3.buffer_objects, 1);
                    }
                    "off" => {
                        game.client.settings.sunrise.shader_pack.clear();
                        frame(game);
                        assert_boundary(game, None, notices);
                        assert_eq!(layout(game), vanilla);
                        next.unwrap().send(TranslationTestAction::Fail).unwrap();
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
                    "renderer" => {
                        #[allow(deprecated)]
                        let window = event_loop
                            .create_window(
                                winit::window::Window::default_attributes()
                                    .with_visible(false)
                                    .with_inner_size(winit::dpi::PhysicalSize::new(320, 240)),
                            )
                            .unwrap();
                        let replacement = game.new_renderer(std::sync::Arc::new(window)).unwrap();
                        game.set_renderer(replacement);
                        assert!(game.client.pack_ids.is_none());
                        next.unwrap().send(TranslationTestAction::Fail).unwrap();
                        game.client
                            .native_shaders
                            .translation
                            .wait_for_translation();
                        frame(game);
                        assert_boundary(game, None, notices);
                        assert_eq!(outcome(&b), None);
                        finish_translation(game, (72, false), notices, "");
                    }
                    "cancel-build" | "refuse-build" | "gpu-spawn-failure" => {
                        let mode = match case.as_str() {
                            "cancel-build" => ShaderPackBuildTestMode::Immediate,
                            "refuse-build" => ShaderPackBuildTestMode::Refuse,
                            _ => ShaderPackBuildTestMode::SpawnFailure,
                        };
                        renderer_mut(game).set_shader_pack_build_test_mode(mode);
                        if case == "gpu-spawn-failure" {
                            game.set_translucent_layers(vec![true]);
                        }
                        next.unwrap().send(TranslationTestAction::Complete).unwrap();
                        game.client
                            .native_shaders
                            .translation
                            .wait_for_translation();
                        frame(game);
                        if case != "gpu-spawn-failure" {
                            assert_boundary(game, Some((72, false)), notices);
                            assert_ne!(layout(game), vanilla);
                            assert!(renderer(game).shader_pack_build_ready());
                            if case == "cancel-build" {
                                game.client.settings.sunrise.shader_pack.clear();
                            }
                            frame(game);
                        }
                        let refused = case == "refuse-build";
                        assert_boundary(game, None, notices + u64::from(refused));
                        assert_eq!(layout(game), vanilla);
                        assert_eq!(renderer(game).shader_pack_kind(), None);
                        assert_eq!(
                            renderer(game).installed_pack_block_id_for_test(stone_index(game)),
                            None
                        );
                        assert!(game.client.streaming.translucent_layers.is_empty());
                        assert_eq!(outcome(&b), refused.then_some(false));
                        assert_eq!(
                            game.client.settings.sunrise.shader_pack,
                            if case == "gpu-spawn-failure" {
                                b.to_str().unwrap()
                            } else {
                                ""
                            },
                        );
                    }
                    _ => {
                        match case.as_str() {
                            "translation-panic" => {
                                next.unwrap().send(TranslationTestAction::Panic).unwrap()
                            }
                            "cancel-translation" | "disconnect" => drop(next.unwrap()),
                            "fingerprint" => {
                                std::fs::write(
                                    b.join("shaders/item.properties"),
                                    "item.1947=minecraft:stick
",
                                )
                                .unwrap();
                                next.unwrap().send(TranslationTestAction::Complete).unwrap();
                            }
                            _ => assert!(next.is_none()),
                        }
                        if matches!(
                            case.as_str(),
                            "translation-panic" | "cancel-translation" | "fingerprint"
                        ) {
                            game.client
                                .native_shaders
                                .translation
                                .wait_for_translation();
                        } else if case == "disconnect" {
                            game.client.native_shaders.translation.disconnect_worker();
                        }
                        let panicked = case == "translation-panic";
                        for _ in 0..2 {
                            frame(game);
                            assert!(!game.client.native_shaders.translation.is_translating());
                            assert_running_pack(
                                game,
                                old_layout,
                                old_revision,
                                notices + u64::from(panicked),
                            );
                            assert_eq!(renderer(game).shader_pack_inputs_for_test(), old_inputs);
                            assert_eq!(renderer(game).resident_sections(), sections);
                            assert_eq!(outcome(&a), Some(true));
                            assert_eq!(
                                outcome(&b),
                                (panicked || case == "open-failure").then_some(false)
                            );
                            assert_eq!(
                                game.client.settings.sunrise.shader_pack,
                                if panicked { "" } else { b.to_str().unwrap() }
                            );
                        }
                    }
                }
                return;
            }
            "mod-off" => {
                game.client.settings.mods.disabled.insert("first".into());
                frame(game);
                assert!(!game.client.native_shaders.translation.is_translating());
                assert_boundary(game, None, notices);
                assert_eq!(layout(game), vanilla);
                gate.send(TranslationTestAction::Fail).unwrap();
                game.client
                    .native_shaders
                    .translation
                    .wait_for_translation();
                frame(game);
                assert_boundary(game, None, notices);
                assert_eq!(layout(game), vanilla);
                assert_eq!(
                    game.client.settings.sunrise.shader_pack,
                    a.to_str().unwrap()
                );
                assert_eq!(outcome(&a), None);
                game.client.settings.mods.disabled.remove("first");
                frame(game);
                finish_translation(game, (41, true), notices, "");
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
                translating_layout = layout(game);
            }
            _ => panic!("unknown shader pack case {case}"),
        }
        game.client
            .native_shaders
            .translation
            .wait_for_translation();
        frame(game);
        assert_boundary(game, None, notices);
        assert_eq!(layout(game), translating_layout);
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
