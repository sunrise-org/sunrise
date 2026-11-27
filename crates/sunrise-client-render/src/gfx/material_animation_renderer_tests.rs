//! `Renderer::animate` carrying an animated LabPBR map into a shader pack's material atlases
//! (#355): a real `Renderer` with three sprite layers, two of them interpolated animations (their
//! colour changes every tick), only one map of which (layer 0's specular) has a second frame. The
//! sprite arrays are overwritten after the atlases are made, so a layer the atlas copies that was
//! not written shows the overwrite; only the written map may reach its atlas. Runs in a child
//! process of the test binary (a window and its event loop), with throwaway homes. Then a pack
//! whose build thread can't start is refused with its atlases (`pack_thread_failed`).
use super::Renderer;
use std::sync::Arc;
use winit::{
    application::ApplicationHandler,
    event::WindowEvent,
    event_loop::{ActiveEventLoop, EventLoop},
    window::{Window, WindowId},
};

const CHILD: &str = "SUNRISE_MATERIAL_ANIMATION_RENDERER_CHILD";
const SIDE: u32 = 16;
const LAYERS: u32 = 3;

/// Each layer's normal and specular texel, the same over the layer: distinct per layer and kind.
fn sentinel(kind: usize, layer: u32) -> [u8; 4] {
    let base = 10 + 40 * kind as u8 + layer as u8;
    [base, base + 100, 3 + layer as u8, 200 - layer as u8]
}
/// Layer 0's second specular frame.
const SECOND: [u8; 4] = [222, 111, 33, 44];
/// What overwrites every layer of the sprite arrays after the atlases are made.
const OVERWRITE: [u8; 4] = [7, 8, 9, 6];

fn layer_pixels(texel: [u8; 4]) -> Vec<u8> {
    texel.repeat((SIDE * SIDE) as usize)
}

fn animations() -> Vec<crate::sprites::SpriteAnimation> {
    let interpolated = r#"{"frametime":2,"interpolate":true,"frames":[0,1]}"#;
    let colours = || {
        vec![
            layer_pixels([255, 0, 0, 255]),
            layer_pixels([0, 0, 255, 255]),
        ]
    };
    let mut zero = crate::sprites::test_frames(0, colours(), interpolated);
    zero.set_materials([
        Some(vec![layer_pixels(sentinel(0, 0))]),
        Some(vec![layer_pixels(sentinel(1, 0)), layer_pixels(SECOND)]),
    ]);
    let mut one = crate::sprites::test_frames(1, colours(), interpolated);
    one.set_materials([Some(vec![layer_pixels(sentinel(0, 1))]), None]);
    vec![zero, one]
}

fn renderer(window: Arc<Window>) -> Renderer {
    let white = [255; (SIDE * SIDE * 4 * LAYERS) as usize];
    let maps = |kind| -> Vec<u8> {
        (0..LAYERS)
            .flat_map(|l| layer_pixels(sentinel(kind, l)))
            .collect()
    };
    let zero = [0; 4];
    // Each layer's native size: a stitched atlas places a sprite for each.
    let mips = [crate::mipmap::Settings {
        native_size: SIDE,
        ..Default::default()
    }; LAYERS as usize];
    let prepared = super::PreparedTextures::new(&white, SIDE, LAYERS, &mips, (&zero, 1)).with_pbr(
        &maps(0),
        &maps(1),
        SIDE,
        LAYERS,
    );
    pollster::block_on(Renderer::new(
        window,
        true,
        &white,
        SIDE,
        SIDE,
        LAYERS,
        &mips,
        &animations(),
        (&white[..(SIDE * SIDE * 4) as usize], SIDE, SIDE),
        None,
        (&zero, 1, 1),
        (&zero, 1, 1),
        (&zero, 1),
        Arc::new(super::panorama::Faces::colour([0, 0, 0, 255])),
        Some(prepared),
        super::RendererDevice::New,
        crate::graphics::GraphicsApi::Vulkan,
        true,
        (true, false),
        &crate::graphics::Graphics::default(),
        Default::default(),
        Default::default(),
    ))
    .expect("a renderer")
}

/// The texel at the middle of each layer's rectangle in `atlas`.
fn atlas_texels(renderer: &Renderer, atlas: &super::pack_atlas::PackAtlas) -> Vec<[u8; 4]> {
    let texture = atlas.texture();
    let [width, height] = atlas.size();
    let row = (width * 4).next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
    let read = renderer.device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: u64::from(row * height),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = renderer.device.create_command_encoder(&Default::default());
    encoder.copy_texture_to_buffer(
        texture.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &read,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(row),
                rows_per_image: Some(height),
            },
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
    renderer.queue.submit([encoder.finish()]);
    read.slice(..).map_async(wgpu::MapMode::Read, |_| {});
    renderer
        .device
        .poll(wgpu::PollType::wait_indefinitely())
        .unwrap();
    let data = read.slice(..).get_mapped_range().unwrap();
    let rects = atlas.rects_table();
    (0..LAYERS as usize)
        .map(|layer| {
            let (corner, side) = (rects[2 * layer], rects[2 * layer + 1]);
            let x = (corner & 0xffff) + side / 2;
            let y = (corner >> 16) + side / 2;
            let at = (y * row + x * 4) as usize;
            data[at..at + 4].try_into().unwrap()
        })
        .collect()
}

fn fixture(window: Arc<Window>, stitched: bool) {
    let mut renderer = renderer(window);
    eprintln!("adapter {:?}", renderer.adapter.get_info());
    renderer.pack_inputs();
    let atlases = renderer
        .pack_material_atlases
        .as_ref()
        .expect("the pack's material atlases");
    for atlas in atlases {
        assert_eq!(
            atlas.is_stitched(),
            stitched,
            "the atlas layout this fixture is for"
        );
    }
    for (kind, atlas) in atlases.iter().enumerate() {
        let made: Vec<_> = (0..LAYERS).map(|l| sentinel(kind, l)).collect();
        assert_eq!(atlas_texels(&renderer, atlas), made, "kind {kind} as made");
    }
    // Every layer of both arrays changes behind the atlases' back.
    let arrays = renderer
        .pbr_maps
        .as_ref()
        .expect("the sprites' LabPBR maps");
    for texture in arrays.iter().flatten() {
        renderer.queue.write_texture(
            texture.as_image_copy(),
            &layer_pixels(OVERWRITE).repeat(texture.depth_or_array_layers() as usize),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(SIDE * 4),
                rows_per_image: Some(SIDE),
            },
            wgpu::Extent3d {
                width: SIDE,
                height: SIDE,
                depth_or_array_layers: texture.depth_or_array_layers(),
            },
        );
    }
    // Tick 1 mixes the colours (layers 0 and 1 change) with every map on its first frame; tick 2
    // shows the second colour frame, and layer 0's specular its second frame.
    for tick in [1, 2] {
        let mut encoder = renderer.device.create_command_encoder(&Default::default());
        renderer.animate(&mut encoder, tick, false);
        assert!(
            renderer.animations.changed().contains(&0)
                && renderer.animations.changed().contains(&1),
            "tick {tick}: the interpolated colours changed"
        );
        renderer.uploads.finish(&encoder);
        renderer.queue.submit([encoder.finish()]);
    }
    let atlases = renderer.pack_material_atlases.as_ref().unwrap();
    assert_eq!(
        atlas_texels(&renderer, &atlases[0]),
        [sentinel(0, 0), sentinel(0, 1), sentinel(0, 2)],
        "normals: no map changed"
    );
    assert_eq!(
        atlas_texels(&renderer, &atlases[1]),
        [SECOND, sentinel(1, 1), sentinel(1, 2)],
        "specular: layer 0's second frame alone"
    );
    // A pack whose build thread can't start is refused, and its atlases go with it.
    renderer.pack_thread_failed(0, "fixture", &std::io::Error::other("no thread"));
    assert!(renderer.pack_material_atlases.is_none() && renderer.pack_atlas.is_none());
    assert!(matches!(renderer.pack_post, Some((0, None))));
}

#[test]
#[ignore = "daily"]
fn an_animated_map_reaches_only_its_own_atlas_layer() {
    in_child("an_animated_map_reaches_only_its_own_atlas_layer", true);
}

/// The same without flat draws (`SUNRISE_FLAT_DRAWS=0`): the atlases are a grid, copied level by
/// level (`PackAtlas::copy`) rather than drawn.
#[test]
#[ignore = "daily"]
fn an_animated_map_reaches_only_its_own_grid_atlas_layer() {
    in_child(
        "an_animated_map_reaches_only_its_own_grid_atlas_layer",
        false,
    );
}

/// Runs the fixture in a child process of this test binary that runs only `test`, with flat
/// draws on (a stitched atlas) or off (a grid) and throwaway homes.
fn in_child(test: &str, stitched: bool) {
    if std::env::var_os(CHILD).is_some() {
        let _gpu = crate::gfx::test_gpu::lock();
        run(stitched);
        return;
    }
    let scratch = tempfile::tempdir().unwrap();
    let mut command = std::process::Command::new(std::env::current_exe().unwrap());
    command
        .args([
            "--exact",
            &format!("gfx::material_animation_renderer_tests::{test}"),
            "--ignored",
            "--nocapture",
        ])
        .current_dir(scratch.path())
        .env(CHILD, "1")
        .env("SUNRISE_LAUNCHER_NO_SELF_UPDATE", "1")
        .env("SUNRISE_NO_AUDIO", "1")
        .env("SUNRISE_TELEMETRY_URL", "off");
    if !stitched {
        command.env("SUNRISE_FLAT_DRAWS", "0");
    }
    for name in [
        "SUNRISE_HOME",
        "SUNRISE_LAUNCHER_HOME",
        "XDG_DATA_HOME",
        "XDG_CONFIG_HOME",
    ] {
        let home = scratch.path().join(name);
        std::fs::create_dir(&home).unwrap();
        command.env(name, home);
    }
    assert!(
        command.status().unwrap().success(),
        "the material animation renderer fixture {test} failed"
    );
}

/// Whether the fixture expects a stitched atlas.
struct App(bool);
impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        let window = Arc::new(
            event_loop
                .create_window(Window::default_attributes().with_visible(false))
                .unwrap(),
        );
        fixture(window, self.0);
        event_loop.exit();
    }
    fn window_event(&mut self, _: &ActiveEventLoop, _: WindowId, _: WindowEvent) {}
}

fn run(stitched: bool) {
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
        .run_app(&mut App(stitched))
        .unwrap();
}
