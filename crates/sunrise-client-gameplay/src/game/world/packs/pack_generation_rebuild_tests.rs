//! `ShaderPackState::generation` continues across a renderer rebuild: a pack installed on the
//! replacement renderer never repeats a generation the previous renderer reported (#6313).
use super::cloud_pack_fixture::{INNER, write_pack};
use crate::game::ClientGame;
use crate::game::model_baking_completed_tests::reload;
use crate::game::native_entity_refs::tests::{test_window, with_client_game};
use mod_api::content::crash::ModCall;
use mod_api::{MeshBufferInit, MeshDescriptor, MeshError, ShaderPackState};
use mod_platform::content::crash::enter_native_mod;
use std::path::Path;
use sunrise_client::graphics::GraphicsApi;
use sunrise_gameplay_common::game::{Screen, test_fixtures::run_fresh_cases_with_env};

const CASE: &str = "SUNRISE_PACK_GENERATION_REBUILD_CASE";

/// Writes a pack of only `final` (colortex0 as it is) with `clouds` as its `clouds=` property.
fn write_shader_pack(folder: &Path, clouds: &str) {
    let shaders = folder.join("shaders");
    std::fs::create_dir_all(&shaders).unwrap();
    std::fs::write(
        shaders.join("final.vsh"),
        "#version 120\nvarying vec2 uv; void main(){gl_Position=ftransform();uv=gl_MultiTexCoord0.xy;}\n",
    )
    .unwrap();
    std::fs::write(
        shaders.join("final.fsh"),
        "#version 120\nvarying vec2 uv; uniform sampler2D colortex0; void main(){gl_FragColor=texture2D(colortex0,uv);}\n",
    )
    .unwrap();
    std::fs::write(
        shaders.join("shaders.properties"),
        format!("clouds={clouds}\n"),
    )
    .unwrap();
}

/// Writes that pack and installs it on the game's renderer, through the renderer's own install.
fn install_pack(game: &mut ClientGame<'_>, folder: &Path, clouds: &str) {
    write_shader_pack(folder, clouds);
    install_on(game.client.renderer.0.as_mut().unwrap(), folder);
}

/// Installs the pack in `folder` as the game's pack load does: the standard macros, then the
/// renderer's.
fn install_on(renderer: &mut sunrise_client::gfx::Renderer, folder: &Path) {
    let pack = sunrise_shaders::Pack::open(folder).unwrap();
    let mut macros = sunrise_shaders::Macros::standard();
    renderer.define_pack_macros(&mut macros);
    let (properties, _) = sunrise_shaders::read_properties(&pack, &mut macros);
    let post = sunrise_shaders::load_post(&pack, "", &properties, &macros, &renderer.pack_inputs());
    renderer
        .set_shader_pack(0, Some(post), &[])
        .unwrap_or_else(|_| panic!("the pack is refused"))
        .expect("the pack runs");
}

/// What a mod's `host.shader_pack_state()` reads after the next frame: the hides-clouds flag
/// (which pack runs) and the generation.
fn frame_state(game: &mut ClientGame<'_>) -> (bool, u32) {
    assert!(game.render().unwrap());
    let ShaderPackState {
        hides_clouds,
        generation,
        ..
    } = game.shader_pack_state().expect("a pack runs");
    (hides_clouds, generation)
}

/// Builds a renderer, installs a pack and then a second one, replaces the renderer through one
/// of the game's real paths, installs a different pack and reads the generation a mod sees:
/// - `reuse`: a resource reload rebuilds the renderer on the same device;
/// - `staged`: a same-device renderer that installed its pack before the game published it;
/// - `replace`: device-loss recovery, which drops the old renderer before opening a new device.
#[test]
#[ignore = "daily"]
fn shader_pack_generation_continues_across_a_renderer_rebuild() {
    if run_fresh_cases_with_env(
        "game::client_side::packs::pack_generation_rebuild_tests::shader_pack_generation_continues_across_a_renderer_rebuild",
        CASE,
        &["reuse", "staged", "replace"],
        1,
        &[("WGPU_BACKEND", "vulkan")],
    ) {
        return;
    }
    let _gpu = sunrise_client_render::gfx::lock_test_gpu();
    with_client_game(|game| {
        let (_event_loop, window) = test_window([160, 120]);
        game.common.options.offscreen = true;
        game.client.settings.sunrise.api = GraphicsApi::Vulkan;
        game.client.title.0 = None;
        game.client.screen = Screen::Playing;
        let renderer = game.new_renderer(window.clone()).unwrap();
        assert_eq!(renderer.info.backend, wgpu::Backend::Vulkan);
        game.set_renderer(renderer);
        game.client.window.0 = Some(window.clone());
        let packs = tempfile::tempdir().unwrap();
        install_pack(game, &packs.path().join("first"), "off");
        assert_eq!(frame_state(game), (true, 1));
        install_pack(game, &packs.path().join("second"), "off");
        assert_eq!(frame_state(game), (true, 2));

        // A retained mesh retires with its renderer: its going stale proves the rebuild.
        let _owner = enter_native_mod(ModCall {
            id: "pack_generation_probe",
            hook: "pack generation rebuild fixture",
            part: None,
        });
        let mesh_buffers = [MeshBufferInit::from_bytes(24, &[0; 96])];
        let mesh = game
            .create_mesh(&MeshDescriptor::new(&mesh_buffers))
            .unwrap();

        let case = std::env::var(CASE).unwrap();
        let other = packs.path().join("other");
        match case.as_str() {
            "reuse" => {
                write_pack("pack_generation", INNER);
                game.client.settings.resource_packs = vec!["file/pack_generation".into()];
                reload(game);
                assert_eq!(game.mesh_release(mesh), Err(MeshError::Stale));
                install_pack(game, &other, "fancy");
            }
            "staged" => {
                // The new renderer counts its own pack before the game publishes it.
                write_shader_pack(&other, "fancy");
                let mut replacement = game
                    .renderer_for(
                        window,
                        &game.common.pack,
                        &game.client.client_pack,
                        &game.client.gui,
                        None,
                    )
                    .unwrap();
                install_on(&mut replacement, &other);
                game.adopt_renderer(replacement);
                assert_eq!(game.mesh_release(mesh), Err(MeshError::Stale));
            }
            "replace" => {
                let renderer = game.client.renderer.0.as_mut().unwrap();
                renderer.wait_submissions();
                renderer.test_mark_device_lost();
                game.recover_device().unwrap();
                assert_eq!(game.mesh_release(mesh), Err(MeshError::Stale));
                install_pack(game, &other, "fancy");
            }
            case => panic!("unknown case {case}"),
        }
        assert_eq!(frame_state(game), (false, 3), "{case}");
        drop(game.client.renderer.0.take());
        drop(game.client.mod_gpu.take());
    });
}
