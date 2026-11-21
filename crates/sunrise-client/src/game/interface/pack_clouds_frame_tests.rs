//! A pack's `core/clouds` through a game's frames: the menu bundle's atlas loads the shader,
//! the renderer the game builds from it takes it, and the world pass draws it in place of the
//! native clouds. Each case runs in a process of its own (one event loop, its own resources).
use super::native_entity_refs::tests::{run_fresh_cases, with_client_game};
use crate::game::native_hooks::particle_count_tests::frame_scene;
use sunrise_assets::resource_source::{Resource, ResourceSource};
use sunrise_client::gfx::ScreenshotRequest;

const NAME: &str = "game::pack_clouds_frame_tests::a_packs_clouds_draw_in_the_world_pass";
const CASE: &str = "SUNRISE_PACK_CLOUDS_FRAME_CASE";

/// A pack whose clouds cover the whole frame, nearest of all (depth 1, reversed Z), in one
/// colour a unorm byte holds exactly (0.2, 0.4, 0.6: 51, 102, 153), each face's quad placed by
/// `gl_VertexIndex` alone: its vertex stage reads no face.
fn pack() -> ResourceSource {
    let files = [
        (
            "assets/minecraft/shaders/core/clouds.vsh",
            "#version 330\nvoid main() {\n    int corner = gl_VertexIndex % 4;\n    \
             vec2 xy = vec2(corner == 1 || corner == 2 ? 1.0 : -1.0, corner >= 2 ? 1.0 : -1.0);\n    \
             gl_Position = vec4(xy, 1.0, 1.0);\n}\n",
        ),
        (
            "assets/minecraft/shaders/core/clouds.fsh",
            "#version 330\nlayout(location = 0) out vec4 fragColor;\n\
             void main() {\n    fragColor = vec4(0.2, 0.4, 0.6, 1.0);\n}\n",
        ),
    ];
    ResourceSource::indexed(
        files
            .into_iter()
            .map(|(path, text)| (path.to_owned(), Resource::bytes(text.as_bytes().to_vec())))
            .collect(),
    )
}

/// The frame's pixels, RGBA, and its width.
fn drawn_frame(game: &mut super::Game<'_>) -> (Vec<u8>, usize) {
    game.renderer.as_mut().unwrap().screenshot = Some(ScreenshotRequest {
        path: "unused.png".into(),
        size: None,
        scaled: false,
        background: false,
    });
    game.render_with(|renderer, frame| renderer.render(frame))
        .unwrap();
    let result = game.renderer.as_mut().unwrap().screenshot_result.take();
    let mut pending = result.unwrap().unwrap();
    let image = pending.image_mut();
    (std::mem::take(&mut image.rgba), image.width as usize)
}

/// With the pack, the world pass draws its clouds over the whole world: a pixel left of the
/// hotbar and the crosshair is the pack's colour. With Improved Transparency on, the native
/// clouds draw instead and that pixel is not.
#[test]
#[ignore = "daily"]
fn a_packs_clouds_draw_in_the_world_pass() {
    if run_fresh_cases(NAME, CASE, &["pack", "improved_transparency"]) {
        return;
    }
    let case = std::env::var(CASE).unwrap();
    with_client_game(|game| {
        game.gui = crate::game::UiResources::load(&pack(), std::path::Path::new("")).unwrap();
        assert!(game.gui.atlas.clouds_shader.is_some());
        let _event_loop = frame_scene(game);
        let renderer = game.renderer.as_mut().unwrap();
        renderer.improved_transparency = case == "improved_transparency";
        if renderer.improved_transparency
            && !renderer
                .device
                .features()
                .contains(wgpu::Features::FLOAT32_BLENDABLE)
        {
            // `oit::supported`: Improved Transparency needs blendable float targets.
            eprintln!("skipped: no Improved Transparency on this adapter");
            return;
        }
        game.screen = super::Screen::Playing;
        game.screen_fx.vignette_brightness = 0.0;
        let (pixels, width) = drawn_frame(game);
        let at = |x: usize, y: usize| &pixels[(y * width + x) * 4..][..4];
        let sample = [at(20, 40), at(20, 120), at(300, 40)];
        if case == "pack" {
            for texel in sample {
                assert_eq!(texel, [51, 102, 153, 255]);
            }
        } else {
            assert!(
                sample.iter().all(|t| *t != [51, 102, 153, 255]),
                "{sample:?}"
            );
        }
    });
}
