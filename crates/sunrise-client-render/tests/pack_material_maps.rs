//! A shader pack's material atlases (the LabPBR maps laid out as its block atlas, #355) through
//! the real `Renderer`: a renderer whose sprites have LabPBR maps makes them with the pack's
//! inputs, and frees them with the pack (`drop_shader_packs`, as picking no pack or saving
//! shader options does), as it frees the block atlas they share the layout of. Read in the
//! renderer's GPU census (`Renderer::gpu_census`, the `pack_material_atlases` row, which
//! `scene_replay --memory` prints as `pack_material_atlases_mb`). Asset-free; run with
//! throwaway SUNRISE_HOME, SUNRISE_LAUNCHER_HOME, XDG_DATA_HOME and XDG_CONFIG_HOME (AGENTS.md):
//! `cargo test -p sunrise-client-render --test pack_material_maps -- --ignored`
//! (under `python3 tools/ci/with_xvfb.py` without a display) gives it its own.
use client::{
    gfx::{PreparedTextures, Renderer},
    graphics::GraphicsApi,
};
use std::{path::Path, sync::Arc};
use sunrise_client_render as client;
use winit::{
    application::ApplicationHandler,
    event::WindowEvent,
    event_loop::{ActiveEventLoop, EventLoop},
    window::{Window, WindowId},
};

#[test]
#[ignore = "daily"]
fn pack_material_atlases_go_with_the_pack() {
    run_daily("pack_material_atlases_go_with_the_pack", fixture);
}

/// One 16 px sprite layer with a normal and a specular map.
fn renderer(window: Arc<Window>) -> Result<Renderer, Box<dyn std::error::Error>> {
    let white = [255; 16 * 16 * 4];
    let normals = [128, 128, 255, 255].repeat(16 * 16);
    let specular = [200, 30, 0, 0].repeat(16 * 16);
    let zero = [0; 4];
    let prepared =
        PreparedTextures::new(&white, 16, 1, &[], (&zero, 1)).with_pbr(&normals, &specular, 16, 1);
    let renderer = pollster::block_on(Renderer::new(
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
        Arc::new(client::gfx::panorama::Faces::colour([0, 0, 0, 255])),
        Some(prepared),
        client::gfx::RendererDevice::New,
        GraphicsApi::Vulkan,
        true,
        (true, false),
        &client::graphics::Graphics::default(),
        Default::default(),
        Default::default(),
    ))?;
    Ok(renderer)
}

fn census(renderer: &Renderer, name: &str) -> u64 {
    renderer
        .gpu_census()
        .into_iter()
        .find(|(row, _)| *row == name)
        .map(|(_, bytes)| bytes)
        .unwrap_or_else(|| panic!("no {name} row in the census"))
}

fn fixture(window: Arc<Window>, _out: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let mut renderer = renderer(window)?;
    let adapter = client::gfx::adapter().expect("the renderer's adapter");
    assert_eq!(adapter.backend, "Vulkan");
    eprintln!(
        "pack_material_maps adapter: {} ({}, {}), driver {}",
        adapter.name, adapter.backend, adapter.kind, adapter.driver
    );
    let maps = census(&renderer, "pbr_maps");
    assert!(maps > 0, "the sprites' LabPBR maps are on the GPU");
    assert_eq!(census(&renderer, "pack_material_atlases"), 0);
    // A pack about to load: its block atlas and, beside it, its material atlases.
    renderer.pack_inputs();
    let held = census(&renderer, "pack_material_atlases");
    assert!(held > 0, "the pack's material atlases are made");
    renderer.drop_shader_packs();
    assert_eq!(census(&renderer, "pack_material_atlases"), 0);
    // The sprites keep their maps for the next pack, which makes its atlases again.
    assert_eq!(census(&renderer, "pbr_maps"), maps);
    renderer.pack_inputs();
    assert_eq!(census(&renderer, "pack_material_atlases"), held);
    Ok(())
}

type Fixture = fn(Arc<Window>, &Path) -> Result<(), Box<dyn std::error::Error>>;

struct App<'a>(&'a Path, Fixture);
impl ApplicationHandler for App<'_> {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        let window = Arc::new(
            event_loop
                .create_window(Window::default_attributes().with_visible(false))
                .unwrap(),
        );
        if let Err(error) = (self.1)(window, self.0) {
            eprintln!("fixture failed: {error}");
            std::process::exit(1);
        }
        event_loop.exit();
    }
    fn window_event(&mut self, _: &ActiveEventLoop, _: WindowId, _: WindowEvent) {}
}

fn run(out: &Path, fixture: Fixture) {
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
    builder
        .build()
        .unwrap()
        .run_app(&mut App(out, fixture))
        .unwrap();
}

/// Runs `fixture` in a child process of this test binary that runs only `test`, with throwaway
/// homes.
fn run_daily(test: &str, fixture: Fixture) {
    const OUTPUT: &str = "SUNRISE_MATERIAL_MAPS_FIXTURE_OUTPUT";
    if let Some(output) = std::env::var_os(OUTPUT) {
        let mut descriptor = wgpu::InstanceDescriptor::new_without_display_handle();
        descriptor.backends = wgpu::Backends::VULKAN;
        let instance = wgpu::Instance::new(descriptor);
        pollster::block_on(instance.request_adapter(&Default::default()))
            .expect("pack_material_maps refused: no Vulkan adapter");
        run(Path::new(&output), fixture);
        return;
    }
    let scratch = tempfile::tempdir().unwrap();
    let mut command = std::process::Command::new(std::env::current_exe().unwrap());
    command
        .args(["--exact", test, "--ignored", "--nocapture"])
        .current_dir(scratch.path())
        .env(OUTPUT, scratch.path().join("output"))
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
    assert!(
        command.status().unwrap().success(),
        "material map fixture {test} failed"
    );
}
