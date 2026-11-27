//! A pack's `PER_BUFFER_BLENDING` (spec 3.2) is answered by the device it runs on (#343): a
//! device that draws colour targets differing in blend (wgpu's `INDEPENDENT_BLEND`) supports
//! it, so a pack that requires it loads without a note, and its `blend.<program>.<buffer>`
//! draws that buffer alone with its own state; the real `Renderer` gives that answer from the
//! adapter it opened. Original fixtures (`water_blend_tests`, and a pack written here), no pack
//! files.
use super::depth_test::Gpu;
use super::water_blend_tests::{halves, picture_with};

#[test]
#[ignore = "daily"]
fn a_device_that_draws_per_buffer_blends_supports_the_required_flag() {
    let _gpu = crate::gfx::test_gpu::lock();
    let gpu = Gpu::new().expect("a Vulkan adapter for the per-buffer blending fixture");
    let flags = gpu.adapter.get_downlevel_capabilities().flags;
    // The macros the game's load starts with on this device (`Renderer::define_pack_macros`).
    let mut macros = sunrise_shaders::Macros::standard();
    macros.define_renderer(4);
    crate::gfx::shader_pack::define_device_features(&mut macros, flags);
    let properties = "iris.features.required = PER_BUFFER_BLENDING\n\
                      blend.gbuffers_water.colortex2=off\n";
    if !flags.contains(wgpu::DownlevelFlags::INDEPENDENT_BLEND) {
        // A device without it keeps the note; drawing the pack's two states is not tried.
        let mut unable = macros.clone();
        unable.set_per_buffer_blending(false);
        assert_eq!(macros, unable, "{flags:?}");
        eprintln!("PER_BUFFER_BLENDING_FIXTURE the adapter lacks INDEPENDENT_BLEND: {flags:?}");
        return;
    }
    let (pixels, notes) = picture_with(&gpu, properties, macros);
    assert!(notes.is_empty(), "{notes:?}");
    // Glass (alpha 0.25) over water in `gbuffers_water`'s default factors (spec 3.2,
    // `SRC_ALPHA ONE_MINUS_SRC_ALPHA ONE ONE_MINUS_SRC_ALPHA`) is src * 0.25 + dst * 0.75 per
    // channel in colortex0: 0.75*0.25+0.25*0.75 = 0.375, 0.125*0.25+0.5*0.75 = 0.40625,
    // 0.875*0.25+0.0625*0.75 = 0.265625, as Rgba8Unorm bytes round(c * 255).
    let color_blended = [96, 104, 68, 255];
    // colortex2's own `off`: what the glass wrote, (0.5625, 0.75, 0.3125).
    let normal_glass = [143, 191, 80, 255];
    halves(
        &pixels,
        color_blended,
        normal_glass,
        "required PER_BUFFER_BLENDING",
    );
}

/// The renderer's answer follows `INDEPENDENT_BLEND` alone: a device without it keeps the
/// required flag's note, whatever else it has.
#[test]
fn the_answer_follows_independent_blend() {
    use wgpu::DownlevelFlags as D;
    let properties = sunrise_shaders::Properties::parse(
        "iris.features.required = PER_BUFFER_BLENDING\n",
        &sunrise_shaders::Macros::standard(),
    );
    let note = "the pack requires `PER_BUFFER_BLENDING`, which is not supported yet";
    for (flags, noted) in [
        (D::all(), false),
        (D::INDEPENDENT_BLEND, false),
        (D::all() - D::INDEPENDENT_BLEND, true),
        (D::empty(), true),
    ] {
        let mut macros = sunrise_shaders::Macros::standard();
        crate::gfx::shader_pack::define_device_features(&mut macros, flags);
        let expected: &[&str] = if noted { &[note] } else { &[] };
        assert_eq!(macros.feature_notes(&properties), expected, "{flags:?}");
    }
}

const RENDERER_CHILD: &str = "SUNRISE_PER_BUFFER_BLENDING_RENDERER_CHILD";

/// The real `Renderer`'s answer (#343): the macros `Renderer::define_pack_macros` gives a pack
/// decide whether `read_properties` notes a required `PER_BUFFER_BLENDING`, by the downlevel
/// flags of the adapter that renderer opened, whichever backend and adapter its device search
/// picked. With the answer yes, the pack's `gbuffers_water`, whose two buffers differ in blend,
/// is installed with it. A child process of this test binary with throwaway homes holds the
/// window's event loop; on Wayland, unset `WAYLAND_DISPLAY` (the child's `XDG_RUNTIME_DIR` has
/// no socket), or run under `python3 tools/ci/with_xvfb.py`.
#[test]
#[ignore = "daily"]
fn the_renderers_macros_answer_per_buffer_blending() {
    if std::env::var_os(RENDERER_CHILD).is_some() {
        renderer_child();
        return;
    }
    let _gpu = crate::gfx::test_gpu::lock();
    let scratch = tempfile::tempdir().unwrap();
    let mut command = std::process::Command::new(std::env::current_exe().unwrap());
    command
        .args([
            "--exact",
            "gfx::pack_post::per_buffer_blending_tests::the_renderers_macros_answer_per_buffer_blending",
            "--ignored",
            "--nocapture",
        ])
        .current_dir(scratch.path())
        .env(RENDERER_CHILD, "1")
        .env("SUNRISE_LAUNCHER_NO_SELF_UPDATE", "1")
        .env("SUNRISE_NO_AUDIO", "1")
        .env("SUNRISE_TELEMETRY_URL", "off");
    for name in [
        "SUNRISE_HOME",
        "SUNRISE_LAUNCHER_HOME",
        "XDG_DATA_HOME",
        "XDG_CONFIG_HOME",
        "XDG_RUNTIME_DIR",
        "SUNRISE_LAUNCHER_TRASH",
        "SUNRISE_LAUNCHER_CLIPBOARD",
        "SUNRISE_LAUNCHER_DESKTOP",
    ] {
        let home = scratch.path().join(name);
        std::fs::create_dir(&home).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&home, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        command.env(name, home);
    }
    let output = command.output().unwrap();
    eprint!("{}", String::from_utf8_lossy(&output.stderr));
    let stdout = String::from_utf8_lossy(&output.stdout);
    print!("{stdout}");
    assert!(output.status.success(), "the renderer fixture failed");
    // The child ran the fixture: a filtered-out run would pass without it.
    assert!(
        stdout.contains("1 passed"),
        "the child ran no test: {stdout}"
    );
}

/// A pack that requires `PER_BUFFER_BLENDING` and whose `gbuffers_water` writes colortex0 with
/// the translucent default and colortex2 unblended (`blend.gbuffers_water.colortex2=off`): two
/// colour targets that differ in blend, which wgpu builds only with `INDEPENDENT_BLEND`. Not a
/// composite: Sunrise drops `blend.<program>.<buffer>` of deferred and composite programs
/// (`sunrise_shaders::load_post`).
fn blending_pack(folder: &std::path::Path) -> sunrise_shaders::Pack {
    let shaders = folder.join("shaders");
    std::fs::create_dir_all(&shaders).unwrap();
    for (name, source) in [
        (
            "shaders.properties",
            "iris.features.required = PER_BUFFER_BLENDING\n\
             blend.gbuffers_water.colortex2=off\n",
        ),
        (
            "gbuffers_water.vsh",
            "#version 120\nvoid main() { gl_Position = ftransform(); }\n",
        ),
        (
            "gbuffers_water.fsh",
            "#version 120\n/* RENDERTARGETS: 0,2 */\n\
             void main() {\n    gl_FragData[0] = vec4(0.75, 0.125, 0.875, 0.25);\n\
             \x20   gl_FragData[1] = vec4(0.5625, 0.75, 0.3125, 0.25);\n}\n",
        ),
        (
            "final.vsh",
            "#version 120\nvarying vec2 uv;\n\
             void main() { uv = gl_MultiTexCoord0.xy; gl_Position = ftransform(); }\n",
        ),
        (
            "final.fsh",
            "#version 120\nuniform sampler2D colortex2;\nvarying vec2 uv;\n\
             void main() { gl_FragColor = vec4(texture2D(colortex2, uv).rgb, 1.0); }\n",
        ),
    ] {
        std::fs::write(shaders.join(name), source).unwrap();
    }
    sunrise_shaders::Pack::open(folder).unwrap()
}

fn renderer_fixture(window: std::sync::Arc<winit::window::Window>) {
    let white = [255; 16 * 16 * 4];
    let zero = [0; 4];
    let mut renderer = pollster::block_on(crate::gfx::Renderer::new(
        window,
        true,
        &white,
        16,
        16,
        1,
        &[],
        &[],
        (&white, 16, 16),
        None,
        (&zero, 1, 1),
        (&zero, 1, 1),
        (&zero, 1),
        std::sync::Arc::new(crate::gfx::panorama::Faces::colour([0, 0, 0, 255])),
        None,
        crate::gfx::RendererDevice::New,
        crate::graphics::GraphicsApi::Vulkan,
        true,
        (true, false),
        &crate::graphics::Graphics::default(),
        Default::default(),
        Default::default(),
    ))
    .expect("a renderer");
    renderer.resize(64, 64);
    // The adapter the renderer opened, read raw.
    eprintln!(
        "PER_BUFFER_BLENDING_RENDERER_ADAPTER {:?}",
        renderer.adapter.get_info()
    );
    let able = renderer
        .adapter
        .get_downlevel_capabilities()
        .flags
        .contains(wgpu::DownlevelFlags::INDEPENDENT_BLEND);
    let folder = tempfile::tempdir().unwrap();
    let pack = blending_pack(folder.path());
    // The game's load: the standard macros, then the renderer's.
    let mut macros = sunrise_shaders::Macros::standard();
    renderer.define_pack_macros(&mut macros);
    let (properties, notes) = sunrise_shaders::read_properties(&pack, &mut macros);
    let note = "the pack requires `PER_BUFFER_BLENDING`, which is not supported yet";
    let expected: &[&str] = if able { &[] } else { &[note] };
    assert_eq!(notes, expected, "INDEPENDENT_BLEND {able}");
    eprintln!("PER_BUFFER_BLENDING_FIXTURE answer {able}, notes {notes:?}");
    if able {
        let post =
            sunrise_shaders::load_post(&pack, "", &properties, &macros, &renderer.pack_inputs());
        assert!(post.water.is_some(), "{:?}", post.notes);
        let (programs, notes) = renderer
            .set_shader_pack(0, Some(post), &[])
            .unwrap_or_else(|_| panic!("the pack is refused"))
            .expect("a pack is installed");
        assert!(programs.iter().any(|p| p == "final"), "{notes:?}");
        // The installed pack draws water with its two blend states.
        let Some((_, Some(installed))) = &renderer.pack_post else {
            panic!("no pack installed: {notes:?}");
        };
        assert!(installed.has_water(), "{notes:?}");
        eprintln!("PER_BUFFER_BLENDING_FIXTURE programs {programs:?}, water installed");
    }
}

fn renderer_child() {
    use winit::{
        application::ApplicationHandler,
        event::WindowEvent,
        event_loop::{ActiveEventLoop, EventLoop},
        window::{Window, WindowId},
    };
    struct App;
    impl ApplicationHandler for App {
        fn resumed(&mut self, event_loop: &ActiveEventLoop) {
            let window = std::sync::Arc::new(
                event_loop
                    .create_window(Window::default_attributes().with_visible(false))
                    .unwrap(),
            );
            renderer_fixture(window);
            event_loop.exit();
        }
        fn window_event(&mut self, _: &ActiveEventLoop, _: WindowId, _: WindowEvent) {}
    }
    for key in [
        "SUNRISE_HOME",
        "SUNRISE_LAUNCHER_HOME",
        "XDG_DATA_HOME",
        "XDG_CONFIG_HOME",
    ] {
        assert!(
            std::env::var_os(key).is_some(),
            "set throwaway {key} (AGENTS.md)"
        );
    }
    let mut builder = EventLoop::builder();
    #[cfg(target_os = "linux")]
    {
        use winit::platform::x11::EventLoopBuilderExtX11;
        builder.with_any_thread(true);
    }
    #[cfg(target_os = "windows")]
    {
        use winit::platform::windows::EventLoopBuilderExtWindows;
        builder.with_any_thread(true);
    }
    builder.build().unwrap().run_app(&mut App).unwrap();
}
