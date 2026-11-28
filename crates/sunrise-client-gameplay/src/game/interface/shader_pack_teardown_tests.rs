//! A shader pack's build never outlives the renderer it was started on, and never keeps its pack
//! once nobody wants it (#6721): through the game's own pack load (`shader_pack_inputs`), with
//! the build held by the renderer's test gate (`PackBuildWatch`) so each case decides when it
//! can end. A replacement renderer takes over the pending builds and the final drop joins the ones on
//! a live device; a build on a lost device may outlive its renderer. Each case runs in a process of
//! its own (one event loop, its own resources).
use super::native_entity_refs::tests::{run_fresh_cases, with_client_game};
use crate::game::native_hooks::particle_count_tests::frame_scene;

const NAME: &str =
    "game::client_side::shader_pack_teardown_tests::a_packs_build_ends_with_its_renderer";
const CASE: &str = "SUNRISE_SHADER_PACK_TEARDOWN_CASE";

const VERTEX: &str = "#version 120\nvarying vec4 glcolor;\nvoid main() {\n    gl_Position = ftransform();\n    glcolor = gl_Color;\n}\n";
const FRAGMENT: &str =
    "#version 120\nvarying vec4 glcolor;\nvoid main() {\n    gl_FragData[0] = glcolor;\n}\n";

/// How long a case waits for a released build to end before it fails: the gate's own limit
/// (`PackBuildWatch`, 60 s), as a build that never ends would hold it.
const ENDS_WITHIN: std::time::Duration = std::time::Duration::from_secs(60);

/// `drop`: the renderer's drop, as at exit, waits for the build still running. `off`: shaders
/// turned off after the build ended but before its pack was polled: the pack goes that frame, not
/// with the renderer. `off-held`: turned off while the build still runs: its pack goes the first
/// frame after it ends. `title` and `title-held`: the same on title screen frames,
/// which load no pack. `swap`: setting a renderer in place of the one the build started on
/// (`set_renderer`) doesn't wait for it, and the new one waits for it as it drops. `lost`: neither
/// graphics device recovery nor the renderer made after it waits for the lost device's build.
#[test]
#[ignore = "daily"]
fn a_packs_build_ends_with_its_renderer() {
    if run_fresh_cases(
        NAME,
        CASE,
        &[
            "drop",
            "off",
            "off-held",
            "title",
            "title-held",
            "swap",
            "lost",
        ],
    ) {
        return;
    }
    let case = std::env::var(CASE).unwrap();
    let scratch = tempfile::tempdir().unwrap();
    let pack = scratch.path().join("teardown");
    let shaders = pack.join("shaders");
    std::fs::create_dir_all(&shaders).unwrap();
    for (name, text) in [
        ("shaders.properties", ""),
        ("gbuffers_terrain.vsh", VERTEX),
        ("gbuffers_terrain.fsh", FRAGMENT),
    ] {
        std::fs::write(shaders.join(name), text).unwrap();
    }
    with_client_game(|game| {
        let _event_loop = frame_scene(game);
        let watch = game.client.renderer.0.as_ref().unwrap().pack_build_watch();
        watch.hold();
        game.client.settings.sunrise.shader_pack = pack.to_str().unwrap().to_owned();
        let sky = sunrise_level::environment::Sky::default();
        let camera = glam::DVec3::new(0.5, 65.5, 0.5);
        // The first frame turns the resource packs' material maps on and reloads them; the
        // next starts the pack's identity scan (Iris compatibility on Auto), and the first frame
        // that finds it landed starts the pack's build, held before its first program.
        let deadline = std::time::Instant::now() + ENDS_WITHIN;
        while watch.running() == 0 {
            assert!(
                std::time::Instant::now() < deadline,
                "the pack's build did not start"
            );
            game.shader_pack_inputs(camera, camera, 0.0, &sky);
        }
        assert!(game.client.pack_ids.is_some(), "the pack loads");
        let renderer = game.client.renderer.0.as_ref().unwrap();
        assert!(renderer.shader_pack_building(), "the pack still builds");
        assert_eq!((watch.running(), watch.joined()), (1, 0));
        let off = |game: &mut crate::game::ClientGame<'_>| {
            game.client.settings.sunrise.shader_pack = String::new();
        };
        // A world frame, or a title screen frame (which loads no pack) for the `title` cases.
        let title = case.starts_with("title");
        let frame = |game: &mut crate::game::ClientGame<'_>| {
            if title {
                if game.client.title.0.is_none() {
                    game.client.title.0 = Some(game.new_title(std::path::PathBuf::from("saves")));
                }
                game.render_with(|_, _| Ok(false)).unwrap();
            } else {
                game.shader_pack_inputs(camera, camera, 0.0, &sky);
            }
        };
        match case.as_str() {
            // Off before the next poll would have taken the built pack: it goes that frame.
            "off" | "title" => {
                watch.release();
                wait_until_built(game);
                off(game);
                frame(game);
                assert_eq!(watch.joined(), 1, "the built pack is kept with shaders off");
                assert!(game.client.pack_ids.is_none());
            }
            // Off while it builds: it goes the first frame after it ends.
            "off-held" | "title-held" => {
                off(game);
                frame(game);
                assert_eq!((watch.running(), watch.joined()), (1, 0));
                watch.release();
                let deadline = std::time::Instant::now() + ENDS_WITHIN;
                while watch.joined() == 0 {
                    assert!(
                        std::time::Instant::now() < deadline,
                        "the ended build is kept with shaders off"
                    );
                    frame(game);
                }
                assert_eq!(watch.joined(), 1);
            }
            "drop" => {
                drop(game.client.renderer.0.take());
                assert_eq!(
                    (watch.running(), watch.joined()),
                    (0, 1),
                    "a pack build outlived the renderer"
                );
            }
            "swap" => {
                let window = game.client.renderer.0.as_ref().unwrap().window().clone();
                let next = game.new_renderer(window).expect("a second renderer");
                let next_watch = next.pack_build_watch();
                game.set_renderer(next);
                // The swap went on with the build still held: nothing waited for it.
                assert_eq!((watch.running(), watch.joined()), (1, 0));
                drop(game.client.renderer.0.take());
                assert_eq!(
                    (watch.running(), next_watch.joined()),
                    (0, 1),
                    "the next renderer did not wait for the build it took"
                );
            }
            "lost" => {
                game.client.window.0 =
                    Some(game.client.renderer.0.as_ref().unwrap().window().clone());
                game.client.renderer.0.as_ref().unwrap().lose_device();
                game.recover_device().expect("the renderer is rebuilt");
                // Recovery went on with the build still held: nothing waited for it.
                assert_eq!((watch.running(), watch.joined()), (1, 0));
                let renderer = game.client.renderer.0.take().unwrap();
                let next = renderer.pack_build_watch();
                drop(renderer);
                // Nor did the next renderer's drop: a build on a lost device is let go.
                assert_eq!(
                    (watch.running(), next.joined()),
                    (1, 0),
                    "a lost device's build was waited for"
                );
                watch.release();
                let deadline = std::time::Instant::now() + ENDS_WITHIN;
                while watch.running() > 0 {
                    assert!(
                        std::time::Instant::now() < deadline,
                        "the released build did not end"
                    );
                    std::thread::yield_now();
                }
            }
            other => panic!("unknown case {other}"),
        }
        assert!(!watch.timed_out());
    });
}

/// Waits for the loading build's thread to end, failing after `ENDS_WITHIN`.
fn wait_until_built(game: &crate::game::ClientGame<'_>) {
    let deadline = std::time::Instant::now() + ENDS_WITHIN;
    while game
        .client
        .renderer
        .0
        .as_ref()
        .unwrap()
        .shader_pack_building()
    {
        assert!(
            std::time::Instant::now() < deadline,
            "the released build did not end"
        );
        std::thread::yield_now();
    }
}
