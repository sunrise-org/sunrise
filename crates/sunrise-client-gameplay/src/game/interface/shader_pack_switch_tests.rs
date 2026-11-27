use super::native_entity_refs::tests::{run_fresh_cases, with_client_game};
use super::native_shaders::{fingerprint, outcome, remember};
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

fn assert_stopped(game: &ClientGame<'_>, vanilla: Layout, notices: u64) {
    assert_boundary(game, None, notices);
    assert_eq!(layout(game), vanilla);
    assert!(!game.client.native_shaders.translation.is_translating());
    assert_eq!(renderer(game).shader_pack_kind(), None);
    assert_eq!(
        renderer(game).installed_pack_block_id_for_test(stone_index(game)),
        None
    );
    assert!(game.client.native_shaders.running.is_empty());
    assert!(game.client.streaming.translucent_layers.is_empty());
}

#[test]
#[ignore = "daily"]
fn a_packs_separate_ao_reaches_the_terrain() {
    if sunrise_gameplay_common::game::test_fixtures::run_fresh_cases_with_env(
        NAME,
        CASE,
        &["hdr", "memo-refused"],
        1,
        &[("SUNRISE_HDR_FRAME", "float")],
    ) {
        return;
    }
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
            "unsupported",
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
    let b = scratch.path().join("native-second");
    let c = scratch.path().join("third");
    write_pack(&a, 41, "separateAo = true\n");
    write_pack(&b, 72, "separateAo = false\nbufferObject.0 = 64\n");
    if case == "hdr" {
        write_pack(&a, 41, "separateAo = true\nsunrise.hdr = true\n");
        write_pack(
            &b,
            72,
            "separateAo = false\nsunrise.hdr = true\nsunrise.hdr.tonemap = sunrise\n",
        );
    }
    if case == "late-failure" {
        write_pack(&c, 93, "separateAo = false\n");
    }
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
                id: "second".into(),
                title: "Second".into(),
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

        gate.send(TranslationTestAction::Complete).unwrap();
        finish_translation(
            game,
            (41, true),
            notices,
            if case == "dimensions" { "first" } else { "" },
        );
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
        assert!(old_inputs.0);
        assert_eq!(old_inputs.1, Some(true));
        assert_eq!(old_inputs.2, [1, 2]);
        game.client.native_shaders.translation.test_lava_layers = Some(vec![3, 4]);
        let target = if case == "options" || case == "dimensions" {
            &a
        } else {
            &b
        };
        pick(game, target);

        match case.as_str() {
            "open-failure" | "translation-spawn-failure" | "unsupported" | "memo-refused" => {
                let (clear_pick, remembered) = match case.as_str() {
                    "open-failure" => {
                        std::fs::remove_dir_all(&b).unwrap();
                        (false, Some(false))
                    }
                    "translation-spawn-failure" => {
                        game.client.native_shaders.translation.test_spawn_failure = true;
                        (false, None)
                    }
                    "memo-refused" => {
                        remember(&b, fingerprint(&b), false);
                        (false, Some(false))
                    }
                    _ => {
                        std::fs::write(b.join("sunrise-shaders.toml"), "").unwrap();
                        (true, None)
                    }
                };
                frame(game);
                assert_stopped(game, vanilla, notices + u64::from(clear_pick));
                assert_eq!(outcome(&b), remembered);
                assert_eq!(
                    game.client.settings.sunrise.shader_pack,
                    if clear_pick { "" } else { b.to_str().unwrap() },
                );
                frame(game);
                assert_stopped(game, vanilla, notices + u64::from(clear_pick));
                return;
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
            }
            "dimensions" => {
                game.common.dim = Dim::Extra(mod_api::__private::dimension_key("test:second"));
            }
            "hdr" => {
                assert_eq!(
                    renderer(game).pack_hdr,
                    Some(sunrise_shaders::Tonemap::Pack)
                );
                game.follow_frame();
                assert_eq!(
                    renderer(game).pack_hdr,
                    Some(sunrise_shaders::Tonemap::Pack)
                );
            }
            _ => {}
        }

        let next = game.client.native_shaders.translation.block_next();
        frame(game);
        assert_running_pack(game, old_layout, old_revision, notices);
        assert_eq!(renderer(game).resident_sections(), 1);
        assert_eq!(renderer(game).shader_pack_inputs_for_test(), old_inputs);
        assert_eq!(game.client.native_shaders.running, a.to_str().unwrap());
        match case.as_str() {
            "switch" | "options" | "dimensions" => {
                next.send(TranslationTestAction::Complete).unwrap();
                let (expected, folder) = match case.as_str() {
                    "options" => ((41, false), ""),
                    "dimensions" => ((41, true), "second"),
                    _ => ((72, false), ""),
                };
                finish_translation(game, expected, notices, folder);
                let inputs = renderer(game).shader_pack_inputs_for_test();
                assert_eq!(inputs.0, expected.1);
                assert_eq!(inputs.1, Some(expected.1));
                assert_eq!(inputs.2, [3, 4]);
                assert_eq!(inputs.3.buffer_objects, u32::from(case == "switch"));
            }
            "hdr" => {
                use sunrise_shaders::Tonemap;
                game.follow_frame();
                assert_eq!(renderer(game).pack_hdr, Some(Tonemap::Pack));
                renderer_mut(game)
                    .set_shader_pack_build_test_mode(ShaderPackBuildTestMode::Immediate);
                next.send(TranslationTestAction::Complete).unwrap();
                game.client
                    .native_shaders
                    .translation
                    .wait_for_translation();
                frame(game);
                assert_boundary(game, Some((72, false)), notices);
                assert!(renderer(game).shader_pack_build_ready());
                assert_eq!(renderer(game).pack_hdr, Some(Tonemap::Sunrise));
                game.follow_frame();
                frame(game);
                assert!(!game.client.native_shaders.translation.is_translating());
                assert_boundary(game, Some((72, false)), notices);
                assert_eq!(renderer(game).shader_pack_kind(), Some("iris"));
                assert_eq!(outcome(&b), Some(true));
            }
            "off" | "mod-off" => {
                if case == "off" {
                    game.client.settings.sunrise.shader_pack.clear();
                } else {
                    game.client.settings.mods.disabled.insert("second".into());
                }
                frame(game);
                assert_stopped(game, vanilla, notices);
                next.send(TranslationTestAction::Fail).unwrap();
                game.client
                    .native_shaders
                    .translation
                    .wait_for_translation();
                frame(game);
                assert_stopped(game, vanilla, notices);
                assert_eq!(outcome(&b), None);
                assert_eq!(
                    game.client.settings.sunrise.shader_pack,
                    if case == "off" {
                        ""
                    } else {
                        b.to_str().unwrap()
                    }
                );
                if case == "mod-off" {
                    game.client.settings.mods.disabled.remove("second");
                    frame(game);
                    finish_translation(game, (72, false), notices, "");
                }
            }
            "late-failure" => {
                pick(game, &c);
                frame(game);
                assert_running_pack(game, old_layout, old_revision, notices);
                next.send(TranslationTestAction::Fail).unwrap();
                game.client
                    .native_shaders
                    .translation
                    .wait_for_translation();
                frame(game);
                assert_running_pack(game, old_layout, old_revision, notices);
                assert_eq!(outcome(&b), None);
                assert_eq!(
                    game.client.settings.sunrise.shader_pack,
                    c.to_str().unwrap()
                );
                finish_translation(game, (93, false), notices, "");
            }
            "resources" => {
                game.reload_packs();
                frame(game);
                assert_running_pack(game, old_layout, old_revision, notices);
                next.send(TranslationTestAction::Fail).unwrap();
                let mut reload = game
                    .client
                    .pack_reload
                    .0
                    .take()
                    .expect("resource reload started");
                let staged = reload.handle.take().unwrap().join().unwrap().unwrap();
                game.apply_staged_pack(staged).unwrap();
                assert_stopped(game, vanilla, notices);
                game.client
                    .native_shaders
                    .translation
                    .wait_for_translation();
                frame(game);
                assert_stopped(game, vanilla, notices);
                assert_eq!(outcome(&b), None);
                finish_translation(game, (72, false), notices, "");
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
                assert_boundary(game, None, notices);
                next.send(TranslationTestAction::Fail).unwrap();
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
                next.send(TranslationTestAction::Complete).unwrap();
                game.client
                    .native_shaders
                    .translation
                    .wait_for_translation();
                frame(game);
                if case != "gpu-spawn-failure" {
                    assert_boundary(game, Some((72, false)), notices);
                    assert_ne!(layout(game), vanilla);
                    assert!(renderer(game).shader_pack_build_ready());
                    game.set_translucent_layers(vec![true]);
                    assert_eq!(*game.client.streaming.translucent_layers, [true]);
                    if case == "cancel-build" {
                        game.client.settings.sunrise.shader_pack.clear();
                    }
                    frame(game);
                }
                let refused = case == "refuse-build";
                assert_stopped(game, vanilla, notices + u64::from(refused));
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
            "translation-panic" | "cancel-translation" | "disconnect" | "fingerprint" => {
                match case.as_str() {
                    "translation-panic" => next.send(TranslationTestAction::Panic).unwrap(),
                    "cancel-translation" | "disconnect" => drop(next),
                    _ => {
                        std::fs::write(
                            b.join("shaders/item.properties"),
                            "item.1947=minecraft:stick\n",
                        )
                        .unwrap();
                        next.send(TranslationTestAction::Complete).unwrap();
                    }
                }
                if case == "disconnect" {
                    game.client.native_shaders.translation.disconnect_worker();
                } else {
                    game.client
                        .native_shaders
                        .translation
                        .wait_for_translation();
                }
                let panicked = case == "translation-panic";
                frame(game);
                assert_stopped(game, vanilla, notices + u64::from(panicked));
                if panicked {
                    game.common.dim = Dim::Nether;
                }
                frame(game);
                assert_stopped(game, vanilla, notices + u64::from(panicked));
                assert_eq!(outcome(&a), Some(true));
                assert_eq!(outcome(&b), panicked.then_some(false));
                assert_eq!(
                    game.client.settings.sunrise.shader_pack,
                    if panicked { "" } else { b.to_str().unwrap() }
                );
            }
            _ => panic!("unknown shader pack case {case}"),
        }
    });
}
