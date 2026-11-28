//! Mod pipelines hold shader-pack state only while a shader pack is selected (#6728): the pack's
//! uniform buffer, each pipeline's pack set-0 group and, under linear light, its gamma fragment
//! module are built on the selection, for pipelines built before it too, dropped when the pack
//! is turned off and built again when it is turned back on; nothing is uploaded for a pack
//! before. Draws on the world's target and on a pack's stay the same throughout.
use super::*;

impl ModPipelines {
    /// The thread that built the pack state held now.
    fn pack_built_on(&self) -> Option<std::thread::ThreadId> {
        Some(self.pack.as_ref()?.built_on)
    }

    /// `[pack uniform buffers, pack set-0 groups, pack fragment modules]` held now.
    fn pack_objects(&self) -> [usize; 3] {
        let Some(pack) = &self.pack else {
            return [0; 3];
        };
        let sources = || pack.sources.iter().flatten();
        [
            1,
            sources().count(),
            sources()
                .filter(|s| matches!(s.fragment, PackFragment::Gamma(_)))
                .count(),
        ]
    }
}

/// The quad's colour, as stored bytes: each channel distinct.
const COLOR: [u8; 4] = [200, 40, 120, 255];
/// The frame's clear colour as `render_target` reads it back in 8 bits.
const BACKGROUND: [f32; 4] = [64.0, 64.0, 64.0, 255.0];

/// `n` through-walls pipelines of load `generation` writing their vertex colour.
fn pipelines(generation: u64, n: usize) -> Pipelines {
    let shaders = vec![(PASS, PipelineBlend::Opaque, WorldDepth::ThroughWalls); n];
    load(generation, &shaders)
}

/// Vanilla blocks other than the ones `render_target` draws with: its projection, but a
/// `ColorModulator` of `(0.5, 0.25, 0.75, 1)`, under which the quad would show as
/// `[100, 10, 90, 255]`.
fn other_blocks() -> [u8; 1024] {
    let mut blocks = fallback_vanilla_blocks(view_proj(), [SIZE, SIZE], 0.0);
    // `DynamicTransforms` at byte 512: `ModelViewMat`, `TextureMat`, then `ColorModulator`.
    let modulator: [f32; 4] = [0.5, 0.25, 0.75, 1.0];
    blocks[640..656].copy_from_slice(bytemuck::cast_slice(&modulator));
    blocks
}

/// The 1024-byte vanilla block uploads one `prepare` of `vertices` drawn by `runs` with
/// [`other_blocks`] makes, or of no runs with those blocks when `runs` is empty.
fn block_uploads(
    (device, queue): &(wgpu::Device, wgpu::Queue),
    primitives: &mut WorldPrimitives,
    set: &Pipelines,
    vertices: &[PipelineVertex],
    runs: &[PipelineRun],
) -> usize {
    let atlas = device.create_texture(&wgpu::TextureDescriptor {
        label: None,
        size: wgpu::Extent3d {
            width: 1,
            height: 1,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    let blocks = other_blocks();
    let mut uploads = 0;
    primitives.prepare(
        device,
        &mut |buffer, bytes| {
            if buffer.size() == 1024 && bytes.len() == 1024 {
                uploads += 1;
            }
            queue.write_buffer(buffer, 0, bytes);
        },
        &world_primitives::Batch {
            pipelines: Batch {
                set: Some(set),
                vertices,
                runs,
                ..Default::default()
            },
            ..Default::default()
        },
        view_proj(),
        [SIZE, SIZE],
        &atlas.create_view(&Default::default()),
        Some(&blocks),
    );
    uploads
}

/// The world's target shows the quad over the frame's top left (screen rows); a pack's shows
/// it over its bottom left, its rows being GL's (`pack_clip_matrix`), which only the pack's
/// blocks give.
fn assert_drawn(at: &dyn Fn(u32, u32) -> [f32; 4], pack: bool, what: &str) {
    let color = COLOR.map(f32::from);
    let (drawn, clear) = if pack {
        ((16, 48), (16, 16))
    } else {
        ((16, 16), (16, 48))
    };
    assert_eq!(at(drawn.0, drawn.1), color, "{what}: the quad");
    assert_eq!(at(clear.0, clear.1), BACKGROUND, "{what}: beside it");
}

#[test]
#[ignore = "daily"]
fn pack_state_exists_only_while_a_pack_is_selected() {
    let _gpu = crate::gfx::test_gpu::lock();
    let Some(gpu) = device() else {
        panic!("no GPU adapter");
    };
    let format = wgpu::TextureFormat::Rgba8Unorm;
    let vertices = quad([8.0, 8.0, 24.0, 24.0], 2.0, COLOR);
    let runs = [PipelineRun {
        slot: 1,
        stage: world_primitives::STAGE_CUSTOM_GEOMETRY as u32,
        quads: 1,
        modifier_stack: 0,
    }];
    for linear in [false, true] {
        let what = |step: &str| format!("linear light {linear}, {step}");
        let mut primitives = WorldPrimitives::new(&gpu.0, (format, linear));
        let two = pipelines(61, 2);
        let batch = |set| Batch {
            set: Some(set),
            vertices: &vertices,
            runs: &runs,
            ..Default::default()
        };
        // No pack: the world draws, and nothing of a pack is built or uploaded.
        let at = render_target(
            &gpu,
            &mut primitives,
            format,
            batch(&two),
            STAGES - 1,
            false,
        );
        if !linear {
            assert_drawn(&at, false, &what("no pack"));
        }
        assert_eq!(
            primitives.pipelines.pack_objects(),
            [0, 0, 0],
            "{}",
            what("no pack")
        );
        assert_eq!(
            block_uploads(&gpu, &mut primitives, &two, &vertices, &runs),
            1
        );
        assert_eq!(block_uploads(&gpu, &mut primitives, &two, &[], &[]), 1);

        // A pack pass alone builds nothing: only the pack's selection does.
        primitives.prepare_pack_pass(&gpu.0, format, false, false);
        assert_eq!(
            primitives.pipelines.pack_objects(),
            [0, 0, 0],
            "{}",
            what("a pack pass before any selection")
        );

        // The selection, before the frame as the pack's installation does, builds them for the
        // two pipelines built before it. Its buffer starts with the frame before's blocks
        // (`other_blocks`), so the quad's colour shows the frame uploads its own.
        let gamma = usize::from(linear) * 2;
        let at = render_target(&gpu, &mut primitives, format, batch(&two), STAGES - 1, true);
        assert_drawn(&at, true, &what("pack on"));
        assert_eq!(
            primitives.pipelines.pack_objects(),
            [1, 2, gamma],
            "{}",
            what("pack on")
        );
        assert_eq!(
            block_uploads(&gpu, &mut primitives, &two, &vertices, &runs),
            2
        );
        assert_eq!(block_uploads(&gpu, &mut primitives, &two, &[], &[]), 2);

        // Off: all of it goes, the world still draws.
        primitives.pipelines.deselect_pack();
        assert_eq!(
            primitives.pipelines.pack_objects(),
            [0, 0, 0],
            "{}",
            what("pack off")
        );
        let at = render_target(
            &gpu,
            &mut primitives,
            format,
            batch(&two),
            STAGES - 1,
            false,
        );
        if !linear {
            assert_drawn(&at, false, &what("pack off"));
        }
        assert_eq!(
            block_uploads(&gpu, &mut primitives, &two, &vertices, &runs),
            1
        );

        // On again: built again, and the pack's target draws as before.
        let at = render_target(&gpu, &mut primitives, format, batch(&two), STAGES - 1, true);
        assert_drawn(&at, true, &what("pack on again"));
        assert_eq!(
            primitives.pipelines.pack_objects(),
            [1, 2, gamma],
            "{}",
            what("pack on again")
        );

        // A load built while the pack is on gets its own.
        let three = pipelines(62, 3);
        primitives.pipelines.build(&gpu.0, &three, None).unwrap();
        let gamma = usize::from(linear) * 3;
        assert_eq!(
            primitives.pipelines.pack_objects(),
            [1, 3, gamma],
            "{}",
            what("reload")
        );
        let at = render_target(
            &gpu,
            &mut primitives,
            format,
            batch(&three),
            STAGES - 1,
            true,
        );
        assert_drawn(&at, true, &what("reload"));
    }
}

/// `module` with an unused ray-query acceleration structure: naga accepts it under every
/// capability, while a device without ray queries (the test device asks for no feature)
/// refuses the module.
fn refused_here(mut module: naga::Module) -> naga::Module {
    let ty = module.types.insert(
        naga::Type {
            name: None,
            inner: naga::TypeInner::AccelerationStructure {
                vertex_return: false,
            },
        },
        naga::Span::UNDEFINED,
    );
    module.global_variables.append(
        naga::GlobalVariable {
            name: Some("refused_here".into()),
            space: naga::AddressSpace::Handle,
            binding: Some(naga::ResourceBinding {
                group: 3,
                binding: 0,
            }),
            ty,
            init: None,
            memory_decorations: naga::MemoryDecorations::empty(),
        },
        naga::Span::UNDEFINED,
    );
    module
}

/// Under linear light, a GPU refusal of a pipeline's gamma fragment module refuses only the
/// draws that need it: the mod's own pipeline on a pack's target. The pack group stays, which
/// a pack program running the mod's vertex stage binds (`pack_globals`), and the world draws.
#[test]
#[ignore = "daily"]
fn a_refused_gamma_module_keeps_the_pack_group() {
    let _gpu = crate::gfx::test_gpu::lock();
    let Some(gpu) = device() else {
        panic!("no GPU adapter");
    };
    let format = wgpu::TextureFormat::Rgba8Unorm;
    let set = pipelines(81, 1);
    let mut primitives = WorldPrimitives::new(&gpu.0, (format, true));
    // The world's fragment module under linear light is translated from the GLSL again
    // (`translate_linear_fragment`); only the gamma one is the program's, made refused here.
    primitives
        .pipelines
        .build_with(&set, |this, built| {
            let program = &built.program;
            let mut broken = (**built).clone();
            broken.program = Arc::new(crate::core_shader::validated(
                program.vertex.clone(),
                refused_here(program.fragment.clone()),
                program.other.clone(),
            ));
            let source = this.pipeline_source(&gpu.0, &Arc::new(broken), None)?;
            let frag = source.fragment_shader.clone();
            this.compile(
                &gpu.0,
                source,
                &frag,
                &VariantTarget::World,
                &SamplerLayout::EMPTY,
                None,
            )
        })
        .unwrap();
    let vertices = quad([8.0, 8.0, 24.0, 24.0], 2.0, COLOR);
    let runs = [PipelineRun {
        slot: 0,
        stage: world_primitives::STAGE_CUSTOM_GEOMETRY as u32,
        quads: 1,
        modifier_stack: 0,
    }];
    let batch = || Batch {
        set: Some(&set),
        vertices: &vertices,
        runs: &runs,
        ..Default::default()
    };
    let world = render_target(&gpu, &mut primitives, format, batch(), STAGES - 1, false);
    assert_ne!(world(16, 16), BACKGROUND, "the world draws the quad");
    let pack = render_target(&gpu, &mut primitives, format, batch(), STAGES - 1, true);
    assert_eq!(primitives.pipelines.pack_objects(), [1, 1, 0]);
    assert!(
        primitives.pipelines.pack_globals(0).is_some(),
        "the pack group stays"
    );
    assert_eq!(
        pack(16, 48),
        BACKGROUND,
        "the mod's own pipeline skips the pack target"
    );
}

/// Through the real `Renderer`: loading a shader pack builds the mod pipelines' pack state,
/// installing the pack publishes it before the first frame, and turning the pack off drops it at
/// once; turning it on again builds it again. A child process of the test binary (a window and
/// its event loop), with throwaway homes.
#[test]
#[ignore = "daily"]
fn a_renderer_builds_mod_pipeline_pack_state_only_with_a_pack() {
    const CHILD: &str = "SUNRISE_MOD_PIPELINE_PACK_STATE_CHILD";
    if std::env::var_os(CHILD).is_some() {
        renderer_child();
        return;
    }
    let _gpu = crate::gfx::test_gpu::lock();
    let scratch = tempfile::tempdir().unwrap();
    let mut command = std::process::Command::new(std::env::current_exe().unwrap());
    command
        .args([
            "--exact",
            "gfx::mod_pipelines::tests::pack_state::a_renderer_builds_mod_pipeline_pack_state_only_with_a_pack",
            "--ignored",
            "--nocapture",
        ])
        .current_dir(scratch.path())
        .env(CHILD, "1")
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

/// Quads all round the camera, 3 blocks away on each side and wound both ways, so the frame's
/// centre shows `color` whichever way the camera looks and whatever the pipeline culls.
fn camera_box(color: [u8; 4]) -> Vec<PipelineVertex> {
    let color = u32::from_le_bytes(color);
    let at = |pos: [f32; 3]| PipelineVertex {
        pos,
        uv: [0.0, 0.0],
        color,
    };
    let mut vertices = Vec::new();
    for axis in [0, 2] {
        for side in [-3.0, 3.0] {
            let corner = |u: f32, v: f32| {
                let mut pos = [u, v, u];
                pos[axis] = side;
                pos[1] = v;
                pos[2 - axis] = u;
                at(pos)
            };
            let face = [
                corner(-3.0, -3.0),
                corner(-3.0, 3.0),
                corner(3.0, 3.0),
                corner(3.0, -3.0),
            ];
            vertices.extend(face);
            vertices.extend(face.iter().rev());
        }
    }
    vertices
}

/// A pack of only a `final` showing `colortex0`.
fn final_pack(folder: &std::path::Path) -> sunrise_shaders::Pack {
    let shaders = folder.join("shaders");
    std::fs::create_dir_all(&shaders).unwrap();
    for (name, source) in [
        (
            "final.vsh",
            "#version 120\nvarying vec2 uv;\n\
             void main() { uv = gl_MultiTexCoord0.xy; gl_Position = ftransform(); }\n",
        ),
        (
            "final.fsh",
            "#version 120\nuniform sampler2D colortex0;\nvarying vec2 uv;\n\
             void main() { gl_FragColor = vec4(texture2D(colortex0, uv).rgb, 1.0); }\n",
        ),
    ] {
        std::fs::write(shaders.join(name), source).unwrap();
    }
    sunrise_shaders::Pack::open(folder).unwrap()
}

fn renderer_fixture(window: std::sync::Arc<winit::window::Window>) {
    use crate::gfx::Renderer;
    let white = [255; 16 * 16 * 4];
    let zero = [0; 4];
    let mut renderer = pollster::block_on(Renderer::new(
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
    eprintln!(
        "MOD_PIPELINE_PACK_STATE_ADAPTER {:?}",
        renderer.adapter.get_info()
    );
    let linear = renderer.output.frame.linear_light;
    let objects = |renderer: &Renderer| {
        renderer
            .world_primitives
            .as_ref()
            .expect("world primitives")
            .pipelines
            .pack_objects()
    };
    let vertices = camera_box(COLOR);
    let runs = [PipelineRun {
        slot: 0,
        stage: world_primitives::STAGE_CUSTOM_GEOMETRY as u32,
        quads: vertices.len() as u32 / 4,
        modifier_stack: 0,
    }];
    let hud = crate::hud::HudBatch::scratch([4, 4]);
    let shots = tempfile::tempdir().unwrap();
    // A world frame (not the title's, which prepares no world primitives and runs no pack)
    // looking level; the frame's centre pixel as saved.
    let render = |renderer: &mut Renderer, set: &Pipelines| -> [u8; 4] {
        let mut frame = crate::gfx::frame_tests::frame(&hud);
        frame.title = false;
        frame.pitch = 0.0;
        frame.world_primitives = world_primitives::Batch {
            pipelines: Batch {
                set: Some(set),
                vertices: &vertices,
                runs: &runs,
                ..Default::default()
            },
            ..Default::default()
        };
        renderer.screenshot = Some(crate::gfx::ScreenshotRequest {
            path: shots.path().join("frame.png"),
            size: None,
            scaled: false,
            background: false,
        });
        renderer.render(&frame, None).unwrap();
        let shot = renderer
            .screenshot_result
            .take()
            .expect("a readback")
            .expect("the frame's pixels");
        let image = &shot.image;
        let at = 4 * ((image.height / 2 * image.width + image.width / 2) as usize);
        image.rgba[at..at + 4].try_into().unwrap()
    };
    let folder = tempfile::tempdir().unwrap();
    let pack = final_pack(folder.path());
    let post = |renderer: &mut Renderer| {
        let mut macros = sunrise_shaders::Macros::standard();
        renderer.define_pack_macros(&mut macros);
        let (properties, _) = sunrise_shaders::read_properties(&pack, &mut macros);
        sunrise_shaders::load_post(&pack, "", &properties, &macros, &renderer.pack_inputs())
    };
    let set_pack = |renderer: &mut Renderer| {
        let post = post(renderer);
        renderer
            .set_shader_pack(0, Some(post), &[])
            .unwrap_or_else(|_| panic!("the pack is refused"))
            .expect("the pack runs");
    };
    // The game's load: on the pack's own thread, then installed by a poll between frames.
    let load_pack = |renderer: &mut Renderer, before_install: &dyn Fn(&mut Renderer)| {
        let post = post(renderer);
        let started = std::time::Instant::now();
        renderer.load_shader_pack(0, post, Vec::new(), "final only".into(), started);
        assert_eq!(objects(renderer), [0, 0, 0], "while the pack loads");
        before_install(renderer);
        let mut report = None;
        for _ in 0..6000 {
            report = renderer.poll_shader_pack();
            if report.is_some() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let (_, _, _, report) = report.expect("the pack's thread finishes within a minute");
        report
            .unwrap_or_else(|_| panic!("the pack is refused"))
            .expect("the pack runs");
    };
    let here = std::thread::current().id();
    let built_on = |renderer: &Renderer| {
        (renderer.world_primitives.as_ref())
            .and_then(|wp| wp.pipelines.pack_built_on())
            .expect("pack state")
    };
    let gamma = usize::from(linear);

    let one = pipelines(71, 1);
    renderer.build_mod_pipelines(&one, &|_| None).unwrap();
    let shown = render(&mut renderer, &one);
    eprintln!("MOD_PIPELINE_PACK_STATE_PIXEL no pack {shown:?}");
    assert_eq!(shown, COLOR, "no pack: the mod's quad");
    assert_eq!(objects(&renderer), [0, 0, 0], "no pack");

    // The pack's load builds the state and its installation publishes it, before any frame:
    // on the pack's thread.
    load_pack(&mut renderer, &|_| {});
    assert_eq!(
        objects(&renderer),
        [1, 1, gamma],
        "loaded, before its first frame (linear light {linear})"
    );
    assert_ne!(built_on(&renderer), here, "built on the pack's thread");
    let shown = render(&mut renderer, &one);
    eprintln!("MOD_PIPELINE_PACK_STATE_PIXEL pack on {shown:?}");
    assert_eq!(
        shown, COLOR,
        "the first pack frame: the mod's quad, through the pack's final"
    );
    assert_eq!(
        objects(&renderer),
        [1, 1, gamma],
        "after the first pack frame"
    );

    // Turned off: gone at once, and a reload then builds none.
    renderer.set_shader_pack(0, None, &[]).unwrap();
    assert_eq!(objects(&renderer), [0, 0, 0], "turned off");
    let off = pipelines(73, 1);
    renderer.build_mod_pipelines(&off, &|_| None).unwrap();
    assert_eq!(
        objects(&renderer),
        [0, 0, 0],
        "a reload right after the pack went off"
    );
    assert_eq!(
        render(&mut renderer, &off),
        COLOR,
        "pack off: the mod's quad"
    );

    // Set at once (`set_shader_pack`): built by the installation itself, before any frame.
    set_pack(&mut renderer);
    assert_eq!(objects(&renderer), [1, 1, gamma], "set");
    assert_eq!(built_on(&renderer), here, "built by the installation");
    assert_eq!(
        render(&mut renderer, &off),
        COLOR,
        "pack on again: the mod's quad"
    );

    // Refused (a pack whose thread could not start): gone at once.
    renderer.pack_thread_failed(0, "refused", &std::io::Error::other("test"));
    assert_eq!(objects(&renderer), [0, 0, 0], "refused");
    assert_eq!(
        render(&mut renderer, &off),
        COLOR,
        "refused: the mod's quad"
    );

    // The mods' pipelines rebuilt while the pack loads: the installation builds the state
    // for the load now running instead of publishing the pack thread's.
    let two = pipelines(72, 2);
    load_pack(&mut renderer, &|renderer| {
        renderer.build_mod_pipelines(&two, &|_| None).unwrap();
    });
    assert_eq!(
        objects(&renderer),
        [1, 2, 2 * gamma],
        "a reload while the pack loaded"
    );
    assert_eq!(built_on(&renderer), here, "built again by the installation");
    assert_eq!(
        render(&mut renderer, &two),
        COLOR,
        "a reload while the pack loaded: the quad"
    );

    // Dropped: gone at once.
    renderer.drop_shader_packs();
    assert_eq!(objects(&renderer), [0, 0, 0], "dropped");
    eprintln!("MOD_PIPELINE_PACK_STATE_FIXTURE linear light {linear}: passed");
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
