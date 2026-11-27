//! A block entity's and an entity's native terrain quads (a mod's still fluids and terrain models,
//! drawn natively from the terrain's sprite arrays) under a `shaders/` pack, through the real
//! `Renderer` (#5090): the pack's `gbuffers_block` and `gbuffers_entities` (solid, cutout and
//! cutout-mipped models) and their translucent programs (fluids, translucent models) draw them
//! from the pack's block atlas into the buffer their `/* DRAWBUFFERS:1 */` names, which `final`
//! shows (spec 2.3, 6.1.4); the pack's `shadow` draws the ones they cast into `shadowcolor0`. With flat section draws on (a
//! stitched block atlas) and off (a grid). `pack_hand_terrain_quads` does the same for the
//! first-person hand's (#5418): `gbuffers_hand` and `gbuffers_hand_water`. Original fixture, no pack assets. Run with throwaway
//! SUNRISE_HOME, SUNRISE_LAUNCHER_HOME, XDG_DATA_HOME and XDG_CONFIG_HOME (AGENTS.md):
//! `cargo test -p sunrise-client-render --test pack_block_entity_runs -- --ignored`
//! (under `python3 tools/ci/with_xvfb.py` without a display) gives each its own.
use client::{
    gfx::{Frame, Renderer, ScreenshotRequest, pack_draw::EntitySpan},
    graphics::GraphicsApi,
    hud::HudBatch,
};
use std::{path::Path, sync::Arc};
use sunrise_client_render as client;
use sunrise_render_core::EntityVertex as Vertex;
use winit::{
    application::ApplicationHandler,
    event::WindowEvent,
    event_loop::{ActiveEventLoop, EventLoop},
    window::{Window, WindowId},
};

#[test]
#[ignore = "daily"]
fn pack_block_entity_runs() {
    run_daily("pack_block_entity_runs", fixture);
}

#[test]
#[ignore = "daily"]
fn pack_hand_terrain_quads() {
    run_daily("pack_hand_terrain_quads", hand_fixture);
}

/// The two sprite layers' colour: each channel 0 or 255, so a texel times a vertex colour is
/// that colour's channel or 0, exactly.
const SPRITES: [[u8; 4]; 2] = [[255, 255, 0, 255], [0, 255, 255, 255]];

/// Each program's output channels (`PROGRAMS`), so the program that drew shows.
const BLOCK: [usize; 3] = [0, 1, 2];
const BLOCK_TRANSLUCENT: [usize; 3] = [2, 1, 0];
const ENTITIES: [usize; 3] = [1, 2, 0];
const ENTITIES_TRANSLUCENT: [usize; 3] = [2, 0, 1];
const HAND: [usize; 3] = [1, 0, 2];
const HAND_WATER: [usize; 3] = [0, 2, 1];

/// The programs and the swizzle each writes its colour with.
const PROGRAMS: [(&str, &str); 4] = [
    ("gbuffers_block", "rgb"),
    ("gbuffers_block_translucent", "bgr"),
    ("gbuffers_entities", "gbr"),
    ("gbuffers_entities_translucent", "brg"),
];

/// Writes a pack whose block and entity programs write the atlas texel times the vertex colour
/// into colortex1 (`/* DRAWBUFFERS:1 */`), each with its channels in an order of its own
/// (`PROGRAMS`), and whose `final` shows colortex1; with `shadow`, a `shadow` program writing the
/// same, unswizzled, into shadowcolor0 and a `final` showing shadowcolor0's middle.
fn pack(renderer: &mut Renderer, folder: &Path, shadow: bool) {
    write_pack(renderer, folder, &PROGRAMS, shadow, &[]);
}

/// The hand's programs and their swizzles: `gbuffers_hand` and, with `water`,
/// `gbuffers_hand_water`; and a `deferred` that inverts colortex1, so what draws before
/// deferred shows inverted and what draws after it (spec 2.3: `gbuffers_hand_water`, or its
/// fallback `gbuffers_hand` for translucent held objects) does not.
fn hand_pack(renderer: &mut Renderer, folder: &Path, water: bool) {
    let programs = [("gbuffers_hand", "grb"), ("gbuffers_hand_water", "rbg")];
    let deferred = [
        (
            "deferred.vsh",
            "#version 120\nvarying vec2 uv; void main() { gl_Position = ftransform(); uv = gl_MultiTexCoord0.xy; }\n",
        ),
        (
            "deferred.fsh",
            "#version 120\n/* DRAWBUFFERS:1 */\nuniform sampler2D colortex1; varying vec2 uv;\n\
             void main() { gl_FragData[0] = vec4(1.0 - texture2D(colortex1, uv).rgb, 1.0); }\n",
        ),
    ];
    write_pack(
        renderer,
        folder,
        &programs[..1 + usize::from(water)],
        false,
        &deferred,
    );
}

/// Writes `programs` as `pack` describes, and `files` (name, text) beside them, and loads the
/// pack.
fn write_pack(
    renderer: &mut Renderer,
    folder: &Path,
    programs: &[(&str, &str)],
    shadow: bool,
    files: &[(&str, &str)],
) {
    let shaders = folder.join("shaders");
    std::fs::create_dir_all(&shaders).unwrap();
    for (name, text) in files {
        std::fs::write(shaders.join(name), text).unwrap();
    }
    let vertex = "#version 120\nvarying vec2 uv; varying vec4 c;\n\
                  void main() { gl_Position = ftransform(); uv = gl_MultiTexCoord0.xy; c = gl_Color; }\n";
    let fragment = |drawbuffers: &str, swizzle: &str| {
        format!(
            "#version 120\n/* DRAWBUFFERS:{drawbuffers} */\nuniform sampler2D gtexture; varying vec2 uv; varying vec4 c;\n\
             void main() {{ gl_FragData[0] = vec4((texture2D(gtexture, uv) * c).{swizzle}, 1.0); }}\n"
        )
    };
    for &(program, swizzle) in programs {
        std::fs::write(shaders.join(format!("{program}.vsh")), vertex).unwrap();
        std::fs::write(
            shaders.join(format!("{program}.fsh")),
            fragment("1", swizzle),
        )
        .unwrap();
    }
    let shown = if shadow {
        std::fs::write(shaders.join("shadow.vsh"), vertex).unwrap();
        std::fs::write(shaders.join("shadow.fsh"), fragment("0", "rgb")).unwrap();
        "texture2D(shadowcolor0, vec2(0.5))"
    } else {
        "texture2D(colortex1, uv)"
    };
    std::fs::write(
        shaders.join("final.vsh"),
        "#version 120\nvarying vec2 uv; void main() { gl_Position = ftransform(); uv = gl_MultiTexCoord0.xy; }\n",
    )
    .unwrap();
    std::fs::write(
        shaders.join("final.fsh"),
        format!(
            "#version 120\nuniform sampler2D colortex1; uniform sampler2D shadowcolor0; varying vec2 uv;\n\
             void main() {{ gl_FragColor = vec4({shown}.rgb, 1.0); }}\n"
        ),
    )
    .unwrap();
    let pack = sunrise_shaders::Pack::open(folder).unwrap();
    let mut macros = sunrise_shaders::Macros::standard();
    renderer.define_pack_macros(&mut macros);
    let (properties, _) = sunrise_shaders::read_properties(&pack, &mut macros);
    let post = sunrise_shaders::load_post(&pack, "", &properties, &macros, &renderer.pack_inputs());
    renderer
        .set_shader_pack(0, Some(post), &[])
        .unwrap()
        .expect("the pack runs");
}

/// A quad across `[x0, x1] x [y0, y1]` on the plane `axis = at` (camera relative), UVs over its
/// whole sprite, overlay `overlay`, the sprite layer where its kind keeps it (`sprite`) and
/// vertex colour `color` (RGBA bytes).
fn quad(
    axis: usize,
    at: f32,
    [x0, x1, y0, y1]: [f32; 4],
    overlay: u32,
    sprite: u32,
    color: [u8; 4],
) -> [Vertex; 4] {
    let corner = |a: f32, b: f32| {
        let mut p = [0.0; 3];
        p[axis] = at;
        p[(axis + 1) % 3] = a;
        p[(axis + 2) % 3] = b;
        p
    };
    let fluid = overlay == client::gfx::FLUID_OVERLAY;
    [
        (corner(x0, y0), [0.0, 1.0]),
        (corner(x1, y0), [1.0, 1.0]),
        (corner(x1, y1), [1.0, 0.0]),
        (corner(x0, y1), [0.0, 0.0]),
    ]
    .map(|(pos, uv)| Vertex {
        pos,
        uv,
        // A terrain model's layer rides in the high bits; a fluid's in `glint_uv.x`.
        layer: if fluid {
            overlay
        } else {
            overlay | sprite << 16
        },
        color: u32::from_le_bytes(color),
        light: 0xf0f0,
        normal: [0; 4],
        glint_uv: [if fluid { sprite as f32 } else { 0.0 }, 0.0],
    })
}

/// A texel of `sprite` times `color`, its channels in `order`, opaque.
fn shaded(sprite: u32, color: [u8; 4], order: [usize; 3]) -> [u8; 4] {
    let s = SPRITES[sprite as usize];
    let c: [u8; 3] = std::array::from_fn(|i| if s[i] == 255 { color[i] } else { 0 });
    [c[order[0]], c[order[1]], c[order[2]], 255]
}

fn shoot(renderer: &mut Renderer, frame: &Frame<'_>, path: &Path) -> [u8; 4] {
    shoot_at(renderer, frame, path, &[[0.5, 0.5]])[0]
}

/// The frame's pixels at `points` (fractions of its width and height, from the top left).
fn shoot_at(
    renderer: &mut Renderer,
    frame: &Frame<'_>,
    path: &Path,
    points: &[[f32; 2]],
) -> Vec<[u8; 4]> {
    renderer.screenshot = Some(ScreenshotRequest {
        path: path.to_path_buf(),
        size: None,
        scaled: false,
        background: false,
    });
    renderer.render(frame, None).unwrap();
    renderer
        .screenshot_result
        .take()
        .expect("readback")
        .unwrap()
        .save()
        .unwrap();
    let decoder = png::Decoder::new(std::io::BufReader::new(std::fs::File::open(path).unwrap()));
    let mut reader = decoder.read_info().unwrap();
    let mut bytes = vec![0; reader.output_buffer_size().unwrap()];
    let info = reader.next_frame(&mut bytes).unwrap();
    assert_eq!(info.color_type, png::ColorType::Rgba);
    let (width, height) = (info.width as usize, info.height as usize);
    (points.iter())
        .map(|&[x, y]| {
            let at = 4 * ((y * height as f32) as usize * width + (x * width as f32) as usize);
            bytes[at..at + 4].try_into().unwrap()
        })
        .collect()
}

/// The span a case draws in: a block entity's or an entity's (`stage`).
fn span(stage: &str, hidden: bool) -> [EntitySpan; 1] {
    [EntitySpan {
        stage: sunrise_shaders::render_stage(stage),
        id: 41,
        hidden,
        ..Default::default()
    }]
}

/// A 320 by 180 renderer whose terrain sprite arrays hold `SPRITES` and whose shared atlas is
/// white.
fn renderer(window: Arc<Window>) -> Result<Renderer, Box<dyn std::error::Error>> {
    let sprites: Vec<u8> = SPRITES
        .iter()
        .flat_map(|texel| texel.repeat(16 * 16))
        .collect();
    // Each layer's own size, as a resource reload gives it: a stitched block atlas places
    // every layer by it.
    let mips = [client::mipmap::Settings {
        native_size: 16,
        ..Default::default()
    }; 2];
    let white = [255; 16 * 16 * 4];
    let zero = [0; 4];
    let mut renderer = pollster::block_on(Renderer::new(
        window,
        true,
        &sprites,
        16,
        16,
        2,
        &mips,
        &[],
        (&white, 16, 16),
        None,
        (&zero, 1, 1),
        (&zero, 1, 1),
        (&zero, 1),
        Arc::new(client::gfx::panorama::Faces::colour([0, 0, 0, 255])),
        None,
        sunrise_client_render::gfx::RendererDevice::New,
        GraphicsApi::Vulkan,
        true,
        (true, true),
        &client::graphics::Graphics::default(),
        Default::default(),
        Default::default(),
    ))?;
    renderer.resize(320, 180);
    Ok(renderer)
}

/// (overlay, sprite layer, vertex colour, drawn by the run's translucent program): each native
/// terrain kind, each with its own colour.
fn cases() -> [(u32, u32, [u8; 4], bool); 5] {
    use client::gfx::{
        FLUID_OVERLAY, TERRAIN_MODEL_CUTOUT_MIPPED_OVERLAY, TERRAIN_MODEL_CUTOUT_OVERLAY,
        TERRAIN_MODEL_SOLID_OVERLAY, TERRAIN_MODEL_TRANSLUCENT_OVERLAY,
    };
    [
        (TERRAIN_MODEL_SOLID_OVERLAY, 1, [200, 100, 50, 255], false),
        (TERRAIN_MODEL_CUTOUT_OVERLAY, 0, [60, 180, 220, 255], false),
        (
            TERRAIN_MODEL_CUTOUT_MIPPED_OVERLAY,
            1,
            [90, 30, 240, 255],
            false,
        ),
        (FLUID_OVERLAY, 0, [120, 70, 10, 255], true),
        (
            TERRAIN_MODEL_TRANSLUCENT_OVERLAY,
            1,
            [40, 160, 250, 255],
            true,
        ),
    ]
}

fn fixture(window: Arc<Window>, out: &Path) -> Result<(), Box<dyn std::error::Error>> {
    std::fs::create_dir_all(out)?;
    let mut renderer = renderer(window)?;
    let hud = HudBatch::scratch([320, 180]);
    let mut result = serde_json::Map::new();
    let cases = cases();
    for flat in [true, false] {
        renderer.set_flat_draws(flat)?;
        let draws = if flat { "flat" } else { "per-section" };
        // The camera's view: each case's quad 5 blocks ahead (yaw 0 looks along +z), across the
        // middle of the screen, in colortex1.
        pack(&mut renderer, &out.join(format!("pack-{draws}")), false);
        // A block entity's run (`gbuffers_block`) and an entity's (`gbuffers_entities`).
        for (stage, opaque, translucent) in [
            ("BLOCK_ENTITIES", BLOCK, BLOCK_TRANSLUCENT),
            ("ENTITIES", ENTITIES, ENTITIES_TRANSLUCENT),
        ] {
            for (overlay, sprite, color, is_translucent) in cases {
                let quad = quad(2, 5.0, [-2.0, 2.0, -2.0, 2.0], overlay, sprite, color);
                let spans = span(stage, false);
                let mut frame = fixture_frame(&hud);
                frame.entities = &quad;
                frame.entity_spans = &spans;
                let name = format!("{draws}-{stage}-{overlay}");
                let pixel = shoot(&mut renderer, &frame, &out.join(format!("{name}.png")));
                let order = if is_translucent { translucent } else { opaque };
                assert_eq!(
                    pixel,
                    shaded(sprite, color, order),
                    "{name}: the quad as its program draws it in colortex1"
                );
                result.insert(name, format!("{pixel:?}").into());
            }
        }
        // The pack's shadow pass: a box of the quads around the camera, hidden from it (it only
        // casts), so whichever way the light falls the middle of the shadow map sees one of
        // its faces. Both kinds a block entity casts, a solid model and a fluid, and an entity's
        // solid model (its translucent parts cast nothing).
        pack(&mut renderer, &out.join(format!("shadow-{draws}")), true);
        for (stage, (overlay, sprite, color, _)) in [
            ("BLOCK_ENTITIES", cases[0]),
            ("BLOCK_ENTITIES", cases[3]),
            ("ENTITIES", cases[0]),
        ] {
            let cube: Vec<Vertex> = (0..3)
                .flat_map(|axis| {
                    [-6.0, 6.0]
                        .map(|at| quad(axis, at, [-6.0, 6.0, -6.0, 6.0], overlay, sprite, color))
                })
                .flatten()
                .collect();
            let spans = span(stage, true);
            let mut frame = fixture_frame(&hud);
            frame.entities = &cube;
            frame.entity_spans = &spans;
            let name = format!("{draws}-shadow-{stage}-{overlay}");
            let pixel = shoot(&mut renderer, &frame, &out.join(format!("{name}.png")));
            assert_eq!(
                pixel,
                shaded(sprite, color, BLOCK),
                "{name}: the box as the shadow program draws it in shadowcolor0"
            );
            result.insert(name, format!("{pixel:?}").into());
        }
    }
    // Vulkan drew these frames. A software adapter (lavapipe) is fine: the comparisons are exact.
    let adapter = client::gfx::adapter().expect("the renderer's adapter");
    assert_eq!(adapter.backend, "Vulkan");
    eprintln!(
        "pack_block_entity_runs adapter: {} ({}, {}), driver {}",
        adapter.name, adapter.backend, adapter.kind, adapter.driver
    );
    std::fs::write(out.join("result.json"), serde_json::to_vec_pretty(&result)?)?;
    Ok(())
}

/// The first-person hand's native terrain quads (a `render_hand` mod's fluids and terrain models,
/// #5418): `gbuffers_hand` draws them from the pack's block atlas into colortex1 before
/// `deferred` (which inverts it), the translucent kinds by `gbuffers_hand_water` when the pack
/// has it, else by its fallback `gbuffers_hand`, after `deferred`. A plain hand quad and a
/// translucent held item's still sample the shared atlas (white here), so they show their vertex
/// colours, and a frame mixing both atlases binds each for its own quads. The pack has no other
/// gbuffers program, no render boundary and no mod pass: nothing but the hand's own translucent
/// quads needs the translucent phase. With a stitched block atlas and a grid.
fn hand_fixture(window: Arc<Window>, out: &Path) -> Result<(), Box<dyn std::error::Error>> {
    use client::gfx::TRANSLUCENT_ITEM_OVERLAY;
    std::fs::create_dir_all(out)?;
    let mut renderer = renderer(window)?;
    let hud = HudBatch::scratch([320, 180]);
    let mut result = serde_json::Map::new();
    // A plain hand quad and a translucent held item's, on the white shared atlas: their vertex
    // colours.
    let (plain, glass) = ([70, 140, 210, 255], [30, 200, 90, 255]);
    let shown = |c: [u8; 4], o: [usize; 3]| [c[o[0]], c[o[1]], c[o[2]], 255];
    for flat in [true, false] {
        renderer.set_flat_draws(flat)?;
        let draws = if flat { "flat" } else { "per-section" };
        for water in [true, false] {
            let programs = if water { "hand-water" } else { "hand" };
            hand_pack(
                &mut renderer,
                &out.join(format!("{programs}-{draws}")),
                water,
            );
            let late = if water { HAND_WATER } else { HAND };
            // Drawn before `deferred`: inverted by it.
            let early = |c: [u8; 4]| [255 - c[0], 255 - c[1], 255 - c[2], 255];
            let hand_cases = (cases().into_iter())
                .map(|(overlay, sprite, color, translucent)| {
                    let expected = if translucent {
                        shaded(sprite, color, late)
                    } else {
                        early(shaded(sprite, color, HAND))
                    };
                    (overlay, sprite, color, expected)
                })
                .chain([
                    (0, 0, plain, early(shown(plain, HAND))),
                    (TRANSLUCENT_ITEM_OVERLAY, 0, glass, shown(glass, late)),
                ]);
            for (overlay, sprite, color, expected) in hand_cases {
                // 5 blocks ahead, across the middle of the screen, as the hand's own
                // vertices are (camera relative).
                let quad = quad(2, 5.0, [-2.0, 2.0, -2.0, 2.0], overlay, sprite, color);
                let mut frame = fixture_frame(&hud);
                frame.hand = &quad;
                let name = format!("{draws}-{programs}-{overlay}");
                let pixel = shoot(&mut renderer, &frame, &out.join(format!("{name}.png")));
                assert_eq!(
                    pixel, expected,
                    "{name}: the hand's quad as its program draws it in colortex1"
                );
                result.insert(name, format!("{pixel:?}").into());
            }
            // One frame switching atlas both ways: a block-atlas run, a plain quad after it,
            // another block-atlas run. Left to right on screen (+x is the viewer's left
            // looking along +z), each its own colour.
            use client::gfx::TERRAIN_MODEL_SOLID_OVERLAY;
            let model = |x: f32, sprite: u32, color: [u8; 4]| {
                quad(
                    2,
                    5.0,
                    [x - 1.0, x + 1.0, -2.0, 2.0],
                    TERRAIN_MODEL_SOLID_OVERLAY,
                    sprite,
                    color,
                )
            };
            let (left, right) = ([200, 100, 50, 255], [60, 180, 220, 255]);
            let mixed: Vec<Vertex> = [
                model(2.5, 1, left),
                quad(2, 5.0, [-1.0, 1.0, -2.0, 2.0], 0, 0, plain),
                model(-2.5, 0, right),
            ]
            .concat();
            let mut frame = fixture_frame(&hud);
            frame.hand = &mixed;
            let name = format!("{draws}-{programs}-mixed");
            let pixels = shoot_at(
                &mut renderer,
                &frame,
                &out.join(format!("{name}.png")),
                &[[0.3, 0.5], [0.5, 0.5], [0.7, 0.5]],
            );
            assert_eq!(
                pixels,
                [
                    early(shaded(1, left, HAND)),
                    early(shown(plain, HAND)),
                    early(shaded(0, right, HAND)),
                ],
                "{name}: each quad from its own atlas in one frame"
            );
            result.insert(name, format!("{pixels:?}").into());
        }
    }
    let adapter = client::gfx::adapter().expect("the renderer's adapter");
    assert_eq!(adapter.backend, "Vulkan");
    eprintln!(
        "pack_hand_terrain_quads adapter: {} ({}, {}), driver {}",
        adapter.name, adapter.backend, adapter.kind, adapter.driver
    );
    std::fs::write(out.join("result.json"), serde_json::to_vec_pretty(&result)?)?;
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

/// Runs `fixture` in a child process of this test binary that runs only `test`, with
/// throwaway homes.
fn run_daily(test: &str, fixture: Fixture) {
    const OUTPUT: &str = "SUNRISE_BLOCK_ENTITY_RUNS_FIXTURE_OUTPUT";
    if let Some(output) = std::env::var_os(OUTPUT) {
        let mut descriptor = wgpu::InstanceDescriptor::new_without_display_handle();
        descriptor.backends = wgpu::Backends::VULKAN;
        let instance = wgpu::Instance::new(descriptor);
        pollster::block_on(instance.request_adapter(&Default::default()))
            .expect("pack_block_entity_runs refused: no Vulkan adapter");
        run(Path::new(&output), fixture);
        return;
    }

    // Keep the event loop and every writable home in a fresh process.
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
        "block entity run fixture {test} failed"
    );
}

/// A frame at the origin looking along +z, with no terrain, sky or weather.
fn fixture_frame(hud: &HudBatch) -> Frame<'_> {
    let eye = glam::DVec3::new(0.5, 100.0, 0.5);
    Frame {
        weather: &[],
        weather_time: client::weather::Time::new(0, 0.0),
        camera: eye,
        yaw: 0.0,
        pitch: 0.0,
        roll: 0.0,
        fov_degrees: 85.0,
        cull_fov_degrees: 85.0,
        eye,
        eye_rotation: [0.0, 0.0],
        first_person: true,
        bob: glam::Mat4::IDENTITY,
        view_transform: glam::Mat4::IDENTITY,
        spin: glam::Mat4::IDENTITY,
        sky: client::environment::Sky::default(),
        shadow_scene_revision: (0, 0),
        shadow_cache: false,
        distance: 64.0,
        detail_distance: 64.0,
        shadow_distance: None,
        depth_far: 1000.0,
        underwater: false,
        cave_culling: false,
        title: false,
        sky_pass: false,
        sky_blocked: false,
        skybox: false,
        end_sky: false,
        end_sky_sprite: None,
        boss_world_fog: false,
        environment_fog: None,
        water_fog: [0.0; 3],
        fog: None,
        dark_disc: false,
        outline: Vec::new(),
        outline_after_terrain: false,
        outline_high_contrast: false,
        pvp: Default::default(),
        debug_lines: &[],
        debug_lines_on_top: &[],
        debug_crosshair: &[],
        hud,
        glint_alpha: 0.75,
        glint_ticks: 0.0,
        nether_lighting: false,
        entities: &[],
        explicit_particles: None,
        static_block_entities: &[],
        visible_static_block_entity_sections: &[],
        static_block_entity_stats: Default::default(),
        entity_motion: &[],
        entity_motion_posed: &[],
        entity_spans: &[],
        skin: None,
        glowing: &[],
        dynamic_lights: &[],
        colored_lights: &[],
        emissive_layers: &[],
        clouds: None,
        lightning: Default::default(),
        additive: &[],
        see_through: &[],
        world_primitives: Default::default(),
        shadow_only: &[],
        celestial: &[],
        celestial_bodies: &[],
        stars: None,
        hand: &[],
        hand_items: &[],
        hand_fov_degrees: 70.0,
        screen: &[],
        pack: Default::default(),
        border: None,
        game_time: 0.0,
        shader_game_time: 0.0,
        end_portal_sprites: [[0.0; 4]; 2],
        crumbling_sprite: [0.0; 4],
        ticks: 0,
        darkness: (0.0, 0.0),
        night_vision: 0.0,
        block_flicker: 0.0,
        boss_darken: 0.0,
        chunk_fade: 0.0,
        post_effects: client::post_chain::Request::none(),
        menu_blur: None,
    }
}
