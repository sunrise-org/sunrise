//! GPU tests for #5371: translating and executing mod `RenderPipeline` pack bindings inside
//! `PackPost` (custom candidate programs, `gbuffers_*` fallback, lazy auxiliary depth targets,
//! typed per-frame uniforms with previous-frame tracking, type-mismatch tolerance, and unbound
//! pipeline reporting).
use super::depth_test::{Gpu, SIZE, VERTEX, fixture_frame};
use super::*;
use crate::gfx::mod_pipelines::PipelineVertex;
use crate::hud::HudBatch;
use wgpu::{TextureFormat as T, util::DeviceExt};

fn mod_quad_buffers(device: &wgpu::Device, z_gl: f32) -> (wgpu::Buffer, wgpu::Buffer) {
    let color = u32::from_le_bytes([255, 255, 255, 255]);
    // Full-screen quad in clip space (`gl_Position = vec4(mod_position, 1.0)`).
    let verts = [
        [-1.0f32, -1.0, z_gl],
        [-1.0, 1.0, z_gl],
        [1.0, 1.0, z_gl],
        [1.0, -1.0, z_gl],
    ]
    .map(|pos| PipelineVertex {
        pos,
        uv: [0.5, 0.5],
        color,
    });
    let indices: [u32; 6] = [0, 1, 2, 0, 2, 3];
    let vbuf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("Mod pack test vertices"),
        contents: bytemuck::cast_slice(&verts),
        usage: wgpu::BufferUsages::VERTEX,
    });
    let ibuf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("Mod pack test indices"),
        contents: bytemuck::cast_slice(&indices),
        usage: wgpu::BufferUsages::INDEX,
    });
    (vbuf, ibuf)
}

fn mod_gbuffers_inputs() -> sunrise_shaders::GbuffersInputs {
    sunrise_shaders::GbuffersInputs {
        declarations: "layout(location = 0) in vec3 mod_position;\n\
             layout(location = 1) in vec2 mod_uv;\n\
             layout(location = 2) in vec4 mod_color;\n"
            .into(),
        body: "hz_Vertex = vec4(mod_position, 1.0);\n\
             hz_Color = mod_color;\n\
             hz_MultiTexCoord0 = vec4(mod_uv, 0.0, 1.0);\n\
             hz_MultiTexCoord1 = vec4(240.0, 240.0, 0.0, 1.0);\n\
             hz_MultiTexCoord2 = hz_MultiTexCoord1;\n\
             hz_Normal = vec3(0.0, 1.0, 0.0);\n"
            .into(),
        ..Default::default()
    }
}

fn load_with_mods(
    gpu: &Gpu,
    files: Vec<(String, String)>,
    requests: Vec<sunrise_shaders::ModPackProgramRequest>,
    runtime: &OwnedModPackRuntime,
) -> (PackPost, wgpu::TextureView, wgpu::TextureView) {
    load_with_mod_layouts(gpu, files, requests, runtime, &[])
}

fn load_with_mod_layouts(
    gpu: &Gpu,
    files: Vec<(String, String)>,
    requests: Vec<sunrise_shaders::ModPackProgramRequest>,
    runtime: &OwnedModPackRuntime,
    layouts: &[Option<ModPipelineLayouts>],
) -> (PackPost, wgpu::TextureView, wgpu::TextureView) {
    let inputs = sunrise_shaders::GameInputs {
        mod_programs: requests,
        ..Default::default()
    };
    load_pack_with_mods(gpu, files, inputs, runtime, layouts)
}

fn load_pack_with_mods(
    gpu: &Gpu,
    files: Vec<(String, String)>,
    inputs: sunrise_shaders::GameInputs,
    runtime: &OwnedModPackRuntime,
    layouts: &[Option<ModPipelineLayouts>],
) -> (PackPost, wgpu::TextureView, wgpu::TextureView) {
    use std::io::{Cursor, Write};
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for (name, contents) in files {
        zip.start_file(
            format!("shaders/{name}"),
            zip::write::SimpleFileOptions::default(),
        )
        .unwrap();
        zip.write_all(contents.as_bytes()).unwrap();
    }
    let pack = sunrise_shaders::Pack::from_zip_bytes(
        "mod-pack-binding-fixture",
        zip.finish().unwrap().into_inner(),
    )
    .unwrap();
    let mut macros = sunrise_shaders::Macros::standard();
    let (props, _) = sunrise_shaders::read_properties(&pack, &mut macros);
    let post = sunrise_shaders::load_post(&pack, "", &props, &macros, &inputs);
    let color = gpu.texture(T::Rgba8Unorm);
    let depth = gpu.texture(T::Depth32Float);
    gpu.clear(&color, &depth, 0.0);
    let world = World {
        atlas: None,
        material_maps: None,
        lightmap: color.clone(),
        terrain: None,
        block_ids: &[],
        sprite_rects: &[],
        celestial: None,
        gui_size: SIZE,
        clouds: None,
        weather: false,
        hand: false,
    };
    let mod_buffers = [Some(PipelineVertex::buffer_layout())];
    let built = PackPost::new_with_mods(
        (&gpu.device, &gpu.queue, &gpu.adapter),
        post,
        T::Rgba8Unorm,
        world,
        runtime.as_runtime(&mod_buffers).with_mod_layouts(layouts),
        &std::sync::atomic::AtomicBool::new(false),
    )
    .expect("pack with mod bindings refused");
    (built, color, depth)
}

fn run_frame_and_sample_center(
    gpu: &Gpu,
    post: &mut PackPost,
    color: &wgpu::TextureView,
    depth: &wgpu::TextureView,
) -> [u8; 4] {
    run_frame_and_sample(gpu, post, color, depth, false)
}

fn run_frame_and_sample(
    gpu: &Gpu,
    post: &mut PackPost,
    color: &wgpu::TextureView,
    depth: &wgpu::TextureView,
    shadow_cache: bool,
) -> [u8; 4] {
    run_frame_and_sample_at(
        gpu,
        post,
        color,
        depth,
        shadow_cache,
        &[[SIZE[0] / 2, SIZE[1] / 2]],
        (
            WorldBoundaries::default(),
            |_: &mut wgpu::CommandEncoder, _: WorldBoundary<'_>| false,
        ),
    )[0]
}

/// Runs one frame and reads the final image at each `[x, y]` of `points`.
fn run_frame_and_sample_at(
    gpu: &Gpu,
    post: &mut PackPost,
    color: &wgpu::TextureView,
    depth: &wgpu::TextureView,
    shadow_cache: bool,
    points: &[[u32; 2]],
    (boundaries, boundary): (
        WorldBoundaries,
        impl FnMut(&mut wgpu::CommandEncoder, WorldBoundary<'_>) -> bool,
    ),
) -> Vec<[u8; 4]> {
    let screen = gpu.texture(T::Rgba8Unorm);
    let stride = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    let read = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("Mod pack binding readback"),
        size: u64::from(stride * SIZE[1]),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let hud = HudBatch::scratch(SIZE);
    let frame = Frame {
        shadow_cache,
        ..fixture_frame(&hud)
    };
    let mut encoder = gpu.device.create_command_encoder(&Default::default());
    post.run(
        &gpu.device,
        &gpu.queue,
        &mut encoder,
        color,
        depth,
        &screen,
        &frame,
        &Default::default(),
        SIZE,
        SIZE,
        &[],
        &mut |_, _, _, _| {},
        boundaries,
        boundary,
        &std::sync::atomic::AtomicBool::new(false),
        super::super::in_flight::poll_live,
    )
    .unwrap();
    encoder.copy_texture_to_buffer(
        screen.texture().as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &read,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(stride),
                rows_per_image: None,
            },
        },
        wgpu::Extent3d {
            width: SIZE[0],
            height: SIZE[1],
            depth_or_array_layers: 1,
        },
    );
    gpu.queue.submit([encoder.finish()]);
    let slice = read.slice(..);
    slice.map_async(wgpu::MapMode::Read, |_| {});
    let _ = gpu.device.poll(wgpu::PollType::wait_indefinitely());
    let data = slice.get_mapped_range().unwrap();
    let px = points
        .iter()
        .map(|&[x, y]| {
            let offset = y as usize * stride as usize + x as usize * 4;
            [
                data[offset],
                data[offset + 1],
                data[offset + 2],
                data[offset + 3],
            ]
        })
        .collect();
    drop(data);
    read.unmap();
    px
}

#[test]
#[ignore = "daily"]
fn pack_owned_feature_stages_draw_only_unbound_prepared_mod_pipelines() {
    use crate::gfx::mod_pipelines::{
        Batch as PipelineBatch, PipelineDraw, PipelineDrawSource, PipelineRun,
    };
    use crate::gfx::world_primitives::{self, Batch, DrawRanges, WorldPrimitives};
    use crate::mod_pipelines::{Pipelines, pack_program_requests_of, pack_runtime_specs_of};
    use mod_api::{
        Identifier, MeshDraw, PackBinding, PackPassSlot, PipelineBlend, RenderPipeline, WorldDepth,
    };

    let _gpu_guard = crate::gfx::test_gpu::lock();
    let gpu = Gpu::new().expect("graphics adapter required for prepared pack stage draws");
    let shader = Identifier::parse("example:core/prepared").unwrap();
    let pipeline = |name| {
        RenderPipeline::new(
            Identifier::parse(name).unwrap(),
            shader.clone(),
            PipelineBlend::Opaque,
            WorldDepth::Tested,
        )
        .vertex_shader(Identifier::parse("minecraft:core/position_tex_color").unwrap())
    };
    let registered = [
        pipeline("example:unbound"),
        pipeline("example:bound").pack_binding(
            PackBinding::new()
                .program("mod_prepared")
                .slot(PackPassSlot::AfterSky),
        ),
    ];
    let fsh = "#version 330\nin vec4 vertexColor;\nout vec4 fragColor;\n\
               void main() { fragColor = vertexColor; }\n";
    let set = Pipelines::build(&registered, 1, |file| {
        if file == "assets/example/shaders/core/prepared.fsh" {
            return Ok(Some(fsh.to_owned()));
        }
        let (ns, path) = file
            .strip_prefix("assets/")
            .unwrap()
            .split_once('/')
            .unwrap();
        Ok(sunrise_mod_pipeline::stubs::read(&format!("{ns}:{path}")).map(str::to_owned))
    })
    .unwrap();

    let files = [
        (
            "gbuffers_entities.vsh",
            "#version 120\nvoid main(){gl_Position=gl_Vertex;}",
        ),
        (
            "gbuffers_entities.fsh",
            "#version 120\n/* DRAWBUFFERS:0 */\nvoid main(){gl_FragData[0]=vec4(1.0);}",
        ),
        (
            "mod_prepared.vsh",
            "#version 120\nvoid main(){gl_Position=vec4(gl_Vertex.xy,0.0,1.0);}",
        ),
        (
            "mod_prepared.fsh",
            "#version 120\n/* DRAWBUFFERS:0 */\n\
             void main(){gl_FragData[0]=vec4(0.0,64.0/255.0,0.0,1.0);}",
        ),
        ("final.vsh", VERTEX),
        (
            "final.fsh",
            "#version 120\nuniform sampler2D colortex0;varying vec2 uv;\n\
             /*\nconst int colortex0Format = RGBA8;\n*/\n\
             void main(){gl_FragColor=vec4(texture2D(colortex0,uv).bgr,1.0);}",
        ),
    ]
    .map(|(name, source)| (name.into(), source.into()))
    .into();
    let inputs = sunrise_shaders::GameInputs {
        entities: Some(crate::gfx::pack_entities::inputs()),
        mod_programs: pack_program_requests_of(&registered),
        ..Default::default()
    };
    let (mut post, color, depth) = load_pack_with_mods(
        &gpu,
        files,
        inputs,
        &pack_runtime_specs_of(&registered),
        &[],
    );
    assert!(post.owns_feature_stages(), "{:?}", post.notes);
    assert_eq!(post.bound_mod_slots(), &[1], "{:?}", post.notes);

    // Four nonoverlapping stripes: unbound run, bound run, unbound direct draw, bound direct draw.
    let quad = |left, right| {
        [[left, -1.0], [left, 1.0], [right, 1.0], [right, -1.0]].map(|[x, y]| PipelineVertex {
            pos: [x, y, 0.75],
            uv: [0.5, 0.5],
            color: u32::from_le_bytes([64, 0, 0, 255]),
        })
    };
    let vertices = [quad(-1.0, -0.5), quad(-0.5, 0.0)];
    let direct_vertices = [quad(0.0, 0.5), quad(0.5, 1.0)];
    let bytes_per_quad = std::mem::size_of::<[PipelineVertex; 4]>() as u64;
    let runs = [0, 1].map(|slot| PipelineRun {
        slot,
        stage: world_primitives::STAGE_AFTER_OPAQUE_FEATURES as u32,
        quads: 1,
        modifier_stack: 0,
    });
    let direct = [0, 1].map(|slot| {
        let start = u64::from(slot) * bytes_per_quad;
        PipelineDraw {
            slot,
            stage: world_primitives::STAGE_AFTER_TRANSLUCENT_FEATURES as u32,
            modifier_stack: 0,
            draw: MeshDraw::vertices(0..4),
            source: PipelineDrawSource::Transient {
                vertex_ranges: vec![(0, start..start + bytes_per_quad)],
                index_range: None,
            },
            bindings: None,
        }
    });
    let batch = Batch {
        pipelines: PipelineBatch {
            set: Some(&set),
            vertices: bytemuck::cast_slice(&vertices),
            runs: &runs,
            draws: &direct,
            transient_vertices: bytemuck::cast_slice(&direct_vertices),
            modifier_defs: Some(&[]),
            ..Default::default()
        },
        ..Default::default()
    };
    let mut primitives = WorldPrimitives::new(&gpu.device, (T::Rgba8Unorm, false));
    primitives.pipelines.build(&gpu.device, &set, None).unwrap();
    assert!(primitives.prepare(
        &gpu.device,
        &mut |buffer, bytes| gpu.queue.write_buffer(buffer, 0, bytes),
        &batch,
        glam::Mat4::IDENTITY,
        SIZE,
        &color,
        None,
    ));
    primitives.prepare_pack_pass(&gpu.device, post.formats[0], post.rgb & 1 != 0, false);
    primitives.set_bound_mod_slots(post.bound_mod_slots());
    post.fill_mod_draws(|slots, out| primitives.pack_bound_chunks_into(slots, out));

    let mut drew = [false; 2];
    let pixels = run_frame_and_sample_at(
        &gpu,
        &mut post,
        &color,
        &depth,
        false,
        &[[2, 8], [6, 8], [10, 8], [14, 8]],
        (
            WorldBoundaries {
                features: true,
                ..Default::default()
            },
            |encoder: &mut wgpu::CommandEncoder, boundary: WorldBoundary<'_>| {
                let WorldBoundary::Features {
                    translucent,
                    color,
                    depth,
                } = boundary
                else {
                    return false;
                };
                let ranges = |range| {
                    if translucent {
                        DrawRanges {
                            pipeline_draws: range,
                            ..Default::default()
                        }
                    } else {
                        DrawRanges {
                            pipeline_runs: range,
                            ..Default::default()
                        }
                    }
                };
                assert!(!primitives.draw_pack_prepared(encoder, color, depth, &[ranges(1..2)]));
                let rendered =
                    primitives.draw_pack_prepared(encoder, color, depth, &[ranges(0..2)]);
                drew[usize::from(translucent)] = rendered;
                rendered
            },
        ),
    );
    // The opaque shaders write 64/255 (exactly byte 64); final swaps red and blue. Bound stripes
    // stay green: an extra fallback draw after AfterSky would overwrite them with the mod's red.
    assert_eq!(
        pixels,
        [
            [0, 0, 64, 255],
            [0, 64, 0, 255],
            [0, 0, 64, 255],
            [0, 64, 0, 255]
        ]
    );
    assert_eq!(drew, [true, true]);
}

#[test]
#[ignore = "daily"]
fn mod_pack_program_draws_aux_depth_and_tracks_previous_uniform() {
    let _gpu_guard = crate::gfx::test_gpu::lock();
    let Some(gpu) = Gpu::new() else {
        return;
    };
    let files = vec![
        (
            "mod_terrain.vsh".to_owned(),
            "#version 120\nvoid main(){gl_Position=vec4(gl_Vertex.xyz,1.0);}".to_owned(),
        ),
        (
            "mod_terrain.fsh".to_owned(),
            "#version 120\n/* DRAWBUFFERS:0 */\nvoid main(){gl_FragData[0]=vec4(1.0,0.0,0.0,1.0);}"
                .to_owned(),
        ),
        ("final.vsh".to_owned(), VERTEX.to_owned()),
        (
            "final.fsh".to_owned(),
            "#version 120\n\
             uniform sampler2D colortex0;\n\
             uniform sampler2D modDepth0;\n\
             uniform sampler2D modDepth1;\n\
             uniform float modScale;\n\
             uniform float prevModScale;\n\
             varying vec2 uv;\n\
             void main(){\n\
                 float d0 = texture2D(modDepth0, uv).r;\n\
                 float d1 = texture2D(modDepth1, uv).r;\n\
                 float saw = (d0 < 0.7 && d1 < 0.7) ? 1.0 : 0.0;\n\
                 gl_FragColor = vec4(saw, modScale, prevModScale, 1.0);\n\
             }"
            .to_owned(),
        ),
    ];
    let requests = vec![sunrise_shaders::ModPackProgramRequest {
        slot: 0,
        programs: vec!["mod_terrain".into(), "gbuffers_terrain".into()],
        group: sunrise_shaders::Group::Gbuffers,
        inputs: mod_gbuffers_inputs(),
        default_blend: None,
    }];
    let runtime = OwnedModPackRuntime {
        depth_targets: vec![ModDepthTargetDecl {
            label: "example:aux_depth".into(),
            depth_samplers: vec!["modDepth0".into()],
            opaque_depth_samplers: vec!["modDepth1".into()],
        }],
        uniforms: vec![ModUniformDecl {
            name: "modScale".into(),
            ty: ModUniformType::Float,
            previous_name: Some("prevModScale".into()),
        }],
        pipelines: vec![ModPipelinePackSpec {
            slot: 0,
            pass_slot: ModPassSlot::AfterOpaqueGbuffers,
            depth_target: Some(0),
            depth_write: true,
            depth_compare: wgpu::CompareFunction::LessEqual,
            depth_bias: Default::default(),
            cull: false,
        }],
    };
    let (mut post, color, depth) = load_with_mods(&gpu, files, requests, &runtime);
    assert!(
        post.is_mod_pipeline_bound(0),
        "mod_terrain not bound: {:?}",
        post.notes
    );

    // GL NDC z = -0.5 maps to window depth 0.25 (< 0.7).
    let (vbuf, ibuf) = mod_quad_buffers(&gpu.device, -0.5);
    post.set_mod_draws(vec![ModDrawChunk::quads(
        0,
        vbuf.clone(),
        ibuf.clone(),
        0..1,
    )]);

    // Frame 1: prev equals current (`0.25` -> ~64).
    assert!(post.set_mod_uniform("modScale", ModUniformValue::Float(0.25)));
    let px1 = run_frame_and_sample_center(&gpu, &mut post, &color, &depth);
    assert_eq!(post.allocated_mod_depth_targets(), (1, 1));
    assert!(
        px1[0] >= 250,
        "expected aux depth visible in final: {px1:?}"
    );
    assert!(
        (px1[1] as i16 - 64).abs() <= 2,
        "frame 1 current modScale mismatch: {px1:?}"
    );
    assert!(
        (px1[2] as i16 - 64).abs() <= 2,
        "frame 1 prevModScale should equal current: {px1:?}"
    );

    // Frame 2: resubmit per-frame mod draws; current updates to `0.75` (~191) and prev retains Frame 1's `0.25` (~64).
    post.set_mod_draws(vec![ModDrawChunk::quads(0, vbuf, ibuf, 0..1)]);
    assert!(post.set_mod_uniform("modScale", ModUniformValue::Float(0.75)));
    let px2 = run_frame_and_sample_center(&gpu, &mut post, &color, &depth);
    assert!(px2[0] >= 250, "frame 2 aux depth lost: {px2:?}");
    assert!(
        (px2[1] as i16 - 191).abs() <= 2,
        "frame 2 current modScale mismatch: {px2:?}"
    );
    assert!(
        (px2[2] as i16 - 64).abs() <= 2,
        "frame 2 prevModScale should retain frame 1: {px2:?}"
    );
}

#[test]
#[ignore = "daily"]
fn mod_pack_linked_pass_honors_depth_compare_and_write() {
    use crate::mod_pipelines::{pack_program_requests_of, pack_runtime_specs_of};
    use mod_api::{
        DepthCompare, Identifier, PackBinding, PackDepthTarget, PackPassSlot, PipelineBlend,
        RenderPipeline, WorldDepth,
    };

    let _gpu_guard = crate::gfx::test_gpu::lock();
    let gpu = Gpu::new().expect("an executed adapter is required for the linked depth fixture");
    let passthrough = "#version 120\nvoid main(){gl_Position=vec4(gl_Vertex.xyz,1.0);}";
    let red =
        "#version 120\n/* DRAWBUFFERS:0 */\nvoid main(){gl_FragData[0]=vec4(1.0,0.0,0.0,1.0);}";
    let files = [
        ("mod_depth.vsh", passthrough),
        ("mod_depth.fsh", red),
        (
            "mod_slope.vsh",
            "#version 120\nvoid main(){gl_Position=vec4(gl_Vertex.xy,0.5*gl_Vertex.x,1.0);}",
        ),
        ("mod_slope.fsh", red),
        ("gbuffers_damagedblock.vsh", passthrough),
        ("gbuffers_damagedblock.fsh", red),
        ("final.vsh", VERTEX),
        (
            "final.fsh",
            "#version 120\n\
             uniform sampler2D colortex0;\n\
             uniform sampler2D depthtex0;\n\
             varying vec2 uv;\n\
             void main(){\n\
                 bool drew = texture2D(colortex0, uv).r > 0.5;\n\
                 float depth = texture2D(depthtex0, uv).r;\n\
                 gl_FragColor = vec4(drew ? 1.0 : 0.0, depth == 0.5 ? 1.0 : 0.0,\n\
                                     depth == 0.75 ? 1.0 : 0.0, 1.0);\n\
             }",
        ),
    ]
    .map(|(name, source)| (name.into(), source.into()))
    .into();
    // All declarations draw in the same slot. A late pass without an explicit target may
    // test against main depth but cannot write it, even if it requests a write.
    // Each row is the depth compare, depth write, whether it names the main depth, the pack
    // program it links and the bias it declares (none for most).
    let states = [
        (DepthCompare::Equal, false, true, "mod_depth", None),
        (DepthCompare::Never, false, true, "mod_depth", None),
        (DepthCompare::Less, false, true, "mod_depth", None),
        (DepthCompare::Greater, false, true, "mod_depth", None),
        (DepthCompare::Always, true, true, "mod_depth", None),
        (DepthCompare::Always, true, false, "mod_depth", None),
        (
            DepthCompare::Greater,
            false,
            true,
            "mod_depth",
            Some((100, 0.0)),
        ),
        (
            DepthCompare::Greater,
            false,
            true,
            "mod_slope",
            Some((0, 2.0)),
        ),
        (
            DepthCompare::Greater,
            false,
            true,
            "gbuffers_damagedblock",
            None,
        ),
        (
            DepthCompare::Greater,
            false,
            true,
            "gbuffers_damagedblock",
            Some((-100, 0.0)),
        ),
    ];
    let pipelines: Vec<_> = states
        .iter()
        .enumerate()
        .map(|(slot, &(compare, write, names_main, program, bias))| {
            let id = Identifier::parse(&format!("example:linked_depth_{slot}")).unwrap();
            let mut binding = PackBinding::new()
                .program(program)
                .slot(PackPassSlot::AfterTranslucentGbuffers);
            if names_main {
                binding = binding.depth_target(PackDepthTarget::Main);
            }
            let pipeline =
                RenderPipeline::new(id.clone(), id, PipelineBlend::Opaque, WorldDepth::Tested)
                    .pack_binding(binding)
                    .depth_compare(compare)
                    .depth_write(write);
            match bias {
                Some((constant, slope)) => pipeline.depth_bias(constant, slope),
                None => pipeline,
            }
        })
        .collect();
    let (mut post, color, depth) = load_with_mods(
        &gpu,
        files,
        pack_program_requests_of(&pipelines),
        &pack_runtime_specs_of(&pipelines),
    );
    for slot in 0..pipelines.len() {
        assert!(
            post.is_mod_pipeline_bound(slot),
            "slot {slot}: {:?}",
            post.notes
        );
    }

    // Seed 0.5 anew each frame. GL clip-space z in [-1, 1] maps to pack depth in [0, 1].
    // Read red for a draw, green for the seed depth and blue for the far draw's depth.
    // The shader emits 0 or 1 per channel: multiplied by 255 these are exactly 0 or 255.
    // The constant bias lifts a coplanar quad. SIZE is 16 pixels across 2 clip-space units, and
    // the slope quad has depth 0.5 + 0.25*x, so one pixel steps the depth by 0.25 * 2 / 16 =
    // 0.03125. The center sample is at pixel centre 8.5 of 16, x = 0.0625, depth 0.5 +
    // 0.015625 (half a step), farther than the seed. The declared slope scale 2.0 is -2.0 in
    // standard-Z and biases by -2 * 0.03125 = -0.0625, so the draw lands at 0.453125, nearer
    // than the seed. Neither biased draw writes depth.
    // The two gbuffers_damagedblock slots are coplanar with the seed under Greater, which is Less
    // in the pack's forward depth and so refuses an unbiased draw. Declaring no bias keeps the
    // pack's polygon offset (-10 constant units toward the camera), which lets the draw pass;
    // a declared constant -100 is +100 in standard-Z, farther, and replaces that offset, so it
    // is refused.
    for (name, slot, clip_z, drew, stored_far) in [
        ("Equal at seed", 0, 0.0, true, false),
        ("Equal nearer", 0, -0.5, false, false),
        ("Never nearer", 1, -0.5, false, false),
        ("Less farther (reversed-Z)", 2, 0.5, true, false),
        ("Greater farther (reversed-Z)", 3, 0.5, false, false),
        ("Greater nearer without write", 3, -0.5, true, false),
        ("Always farther with write", 4, 0.5, true, true),
        ("Always farther in read-only slot", 5, 0.5, true, false),
        ("Greater coplanar with constant bias", 6, 0.0, true, false),
        ("Greater farther with slope bias", 7, 0.0, true, false),
        (
            "Undeclared bias keeps the pack's polygon offset",
            8,
            0.0,
            true,
            false,
        ),
        (
            "Declared bias replaces the pack's polygon offset",
            9,
            0.0,
            false,
            false,
        ),
    ] {
        gpu.clear(&color, &depth, 0.5);
        let (vertices, indices) = mod_quad_buffers(&gpu.device, clip_z);
        post.set_mod_draws(vec![ModDrawChunk::quads(slot, vertices, indices, 0..1)]);
        let pixel = run_frame_and_sample_center(&gpu, &mut post, &color, &depth);
        assert_eq!(
            pixel,
            [
                if drew { 255 } else { 0 },
                if stored_far { 0 } else { 255 },
                if stored_far { 255 } else { 0 },
                255,
            ],
            "{name}",
        );
    }
}

#[test]
#[ignore = "daily"]
fn mod_pack_falls_back_to_gbuffers_terrain_when_custom_program_absent() {
    let _gpu_guard = crate::gfx::test_gpu::lock();
    let Some(gpu) = Gpu::new() else {
        return;
    };
    let files = vec![
        (
            "gbuffers_terrain.vsh".to_owned(),
            "#version 120\nvoid main(){gl_Position=vec4(gl_Vertex.xyz,1.0);}".to_owned(),
        ),
        (
            "gbuffers_terrain.fsh".to_owned(),
            "#version 120\n/* DRAWBUFFERS:0 */\nvoid main(){gl_FragData[0]=vec4(0.0,1.0,0.0,1.0);}"
                .to_owned(),
        ),
        ("final.vsh".to_owned(), VERTEX.to_owned()),
        (
            "final.fsh".to_owned(),
            "#version 120\nuniform sampler2D colortex0;varying vec2 uv;\
             void main(){gl_FragColor=texture2D(colortex0,uv);}"
                .to_owned(),
        ),
    ];
    let requests = vec![sunrise_shaders::ModPackProgramRequest {
        slot: 0,
        programs: vec!["mod_terrain".into(), "gbuffers_terrain".into()],
        group: sunrise_shaders::Group::Gbuffers,
        inputs: mod_gbuffers_inputs(),
        default_blend: None,
    }];
    let runtime = OwnedModPackRuntime {
        depth_targets: Vec::new(),
        uniforms: Vec::new(),
        pipelines: vec![ModPipelinePackSpec {
            slot: 0,
            pass_slot: ModPassSlot::AfterOpaqueGbuffers,
            depth_target: None,
            depth_write: true,
            depth_compare: wgpu::CompareFunction::LessEqual,
            depth_bias: Default::default(),
            cull: false,
        }],
    };
    let (mut post, color, depth) = load_with_mods(&gpu, files, requests, &runtime);
    assert!(
        post.is_mod_pipeline_bound(0),
        "expected fallback to gbuffers_terrain: {:?}",
        post.notes
    );
    let (vbuf, ibuf) = mod_quad_buffers(&gpu.device, 0.0);
    post.set_mod_draws(vec![ModDrawChunk::quads(0, vbuf, ibuf, 0..1)]);
    let px = run_frame_and_sample_center(&gpu, &mut post, &color, &depth);
    assert!(
        px[1] >= 250 && px[0] <= 5 && px[2] <= 5,
        "expected green from gbuffers_terrain: {px:?}"
    );
}

#[test]
#[ignore = "daily"]
fn mod_pack_aux_depth_is_lazy_when_pack_never_samples_it() {
    let _gpu_guard = crate::gfx::test_gpu::lock();
    let Some(gpu) = Gpu::new() else {
        return;
    };
    let files = vec![
        (
            "mod_terrain.vsh".to_owned(),
            "#version 120\nvoid main(){gl_Position=vec4(gl_Vertex.xyz,1.0);}".to_owned(),
        ),
        (
            "mod_terrain.fsh".to_owned(),
            "#version 120\n/* DRAWBUFFERS:0 */\nvoid main(){gl_FragData[0]=vec4(0.0,0.0,1.0,1.0);}"
                .to_owned(),
        ),
        ("final.vsh".to_owned(), VERTEX.to_owned()),
        (
            "final.fsh".to_owned(),
            "#version 120\nuniform sampler2D colortex0;varying vec2 uv;\
             void main(){gl_FragColor=texture2D(colortex0,uv);}"
                .to_owned(),
        ),
    ];
    let requests = vec![sunrise_shaders::ModPackProgramRequest {
        slot: 0,
        programs: vec!["mod_terrain".into()],
        group: sunrise_shaders::Group::Gbuffers,
        inputs: mod_gbuffers_inputs(),
        default_blend: None,
    }];
    let runtime = OwnedModPackRuntime {
        depth_targets: vec![ModDepthTargetDecl {
            label: "example:unused_aux_depth".into(),
            depth_samplers: vec!["modDepth0".into()],
            opaque_depth_samplers: vec!["modDepth1".into()],
        }],
        uniforms: Vec::new(),
        pipelines: vec![ModPipelinePackSpec {
            slot: 0,
            pass_slot: ModPassSlot::AfterOpaqueGbuffers,
            depth_target: Some(0),
            depth_write: true,
            depth_compare: wgpu::CompareFunction::LessEqual,
            depth_bias: Default::default(),
            cull: false,
        }],
    };
    let (mut post, color, depth) = load_with_mods(&gpu, files, requests, &runtime);
    assert!(post.is_mod_pipeline_bound(0));
    let (vbuf, ibuf) = mod_quad_buffers(&gpu.device, 0.0);
    post.set_mod_draws(vec![ModDrawChunk::quads(0, vbuf, ibuf, 0..1)]);
    let px = run_frame_and_sample_center(&gpu, &mut post, &color, &depth);
    assert_eq!(
        post.allocated_mod_depth_targets(),
        (0, 0),
        "unsampled aux depth target must not allocate GPU textures"
    );
    assert!(
        px[2] >= 250 && px[0] <= 5 && px[1] <= 5,
        "expected blue from mod_terrain: {px:?}"
    );
}

#[test]
#[ignore = "daily"]
fn mod_pack_type_mismatch_uniform_and_unbound_pipeline_never_panic() {
    let _gpu_guard = crate::gfx::test_gpu::lock();
    let Some(gpu) = Gpu::new() else {
        return;
    };
    let files = vec![
        (
            "mod_terrain.vsh".to_owned(),
            "#version 120\nvoid main(){gl_Position=vec4(gl_Vertex.xyz,1.0);}".to_owned(),
        ),
        (
            "mod_terrain.fsh".to_owned(),
            "#version 120\n/* DRAWBUFFERS:0 */\nvoid main(){gl_FragData[0]=vec4(1.0,1.0,0.0,1.0);}"
                .to_owned(),
        ),
        ("final.vsh".to_owned(), VERTEX.to_owned()),
        (
            "final.fsh".to_owned(),
            "#version 120\n\
             uniform sampler2D colortex0;\n\
             uniform int modScale;\n\
             varying vec2 uv;\n\
             void main(){gl_FragColor=texture2D(colortex0,uv) + vec4(float(modScale));}"
                .to_owned(),
        ),
    ];
    let requests = vec![
        sunrise_shaders::ModPackProgramRequest {
            slot: 0,
            programs: vec!["mod_terrain".into()],
            group: sunrise_shaders::Group::Gbuffers,
            inputs: mod_gbuffers_inputs(),
            default_blend: None,
        },
        sunrise_shaders::ModPackProgramRequest {
            slot: 1,
            programs: vec!["missing_only_program".into()],
            group: sunrise_shaders::Group::Gbuffers,
            inputs: mod_gbuffers_inputs(),
            default_blend: None,
        },
    ];
    let runtime = OwnedModPackRuntime {
        depth_targets: Vec::new(),
        uniforms: vec![ModUniformDecl {
            name: "modScale".into(),
            ty: ModUniformType::Float,
            previous_name: None,
        }],
        pipelines: vec![
            ModPipelinePackSpec {
                slot: 0,
                pass_slot: ModPassSlot::AfterOpaqueGbuffers,
                depth_target: None,
                depth_write: true,
                depth_compare: wgpu::CompareFunction::LessEqual,
                depth_bias: Default::default(),
                cull: false,
            },
            ModPipelinePackSpec {
                slot: 1,
                pass_slot: ModPassSlot::AfterOpaqueGbuffers,
                depth_target: None,
                depth_write: true,
                depth_compare: wgpu::CompareFunction::LessEqual,
                depth_bias: Default::default(),
                cull: false,
            },
        ],
    };
    let (post, _, _) = load_with_mods(&gpu, files, requests, &runtime);
    assert!(post.is_mod_pipeline_bound(0));
    assert!(!post.is_mod_pipeline_bound(1));
    assert!(
        post.notes
            .iter()
            .any(|n| n.contains("modScale") && n.contains("left unbound")),
        "expected mismatch diagnostic in notes: {:?}",
        post.notes
    );
}

#[test]
#[ignore = "daily"]
fn mod_pack_frame_uniform_type_mismatch_draws_with_zero() {
    use crate::mod_pipelines::{pack_program_requests_of, pack_runtime_specs_of};
    use mod_api::{
        Identifier, PackBinding, PackPassSlot, PackUniformDecl, PackUniformType, PipelineBlend,
        RenderPipeline, WorldDepth,
    };

    let _gpu_guard = crate::gfx::test_gpu::lock();
    let gpu = Gpu::new().expect("graphics adapter required for mod pack frame uniform draws");
    let id = Identifier::parse("example:mod_uniform").unwrap();
    let pipelines = [RenderPipeline::new(
        id.clone(),
        id,
        PipelineBlend::Opaque,
        WorldDepth::ThroughWalls,
    )
    .pack_binding(
        PackBinding::new()
            .program("mod_terrain")
            .slot(PackPassSlot::AfterOpaqueGbuffers)
            .frame_uniform(PackUniformDecl::new("modScale", PackUniformType::Int)),
    )];
    let runtime = pack_runtime_specs_of(&pipelines);

    // Green marks a draw. Red is zero for the mismatched uniform and 1 for the exact staged int
    // in the control (bytes 0 and 255). Neither pack default is zero: 0.25 * 255 = 63.75 and
    // 0.75 * 255 = 191.25. The staged int's bits encode 1.0 as float, so a raw write also fails.
    for (decl, red, expected) in [
        ("float modScale = 0.25", "modScale", 0),
        ("int modScale", "modScale == 1065353216 ? 1.0 : 0.0", 255),
    ] {
        let fragment = format!(
            "#version 120\n/* DRAWBUFFERS:0 */\nuniform {decl};\n\
             void main(){{gl_FragData[0]=vec4({red},1.0,0.0,1.0);}}"
        );
        let files = [
            ("shaders.properties", "uniform.float.modScale = 0.75\n"),
            (
                "mod_terrain.vsh",
                "#version 120\nvoid main(){gl_Position=vec4(gl_Vertex.xyz,1.0);}",
            ),
            ("mod_terrain.fsh", fragment.as_str()),
            ("final.vsh", VERTEX),
            (
                "final.fsh",
                "#version 120\nuniform sampler2D colortex0;varying vec2 uv;\n\
                 /*\nconst int colortex0Format = RGBA8;\n*/\n\
                 void main(){gl_FragColor=texture2D(colortex0,uv);}",
            ),
        ]
        .map(|(name, source)| (name.into(), source.into()))
        .into();
        let (mut post, color, depth) =
            load_with_mods(&gpu, files, pack_program_requests_of(&pipelines), &runtime);
        assert!(post.is_mod_pipeline_bound(0), "{decl}: {:?}", post.notes);
        assert!(post.set_mod_uniform("modScale", ModUniformValue::Int(0x3f80_0000)));
        let (vertices, indices) = mod_quad_buffers(&gpu.device, -0.5);
        post.set_mod_draws(vec![ModDrawChunk::quads(0, vertices, indices, 0..1)]);
        assert_eq!(
            run_frame_and_sample_center(&gpu, &mut post, &color, &depth),
            [expected, 255, 0, 255],
            "{decl}",
        );
    }
}

/// A mod pipeline bound to the pack's shadow pass draws into the shadow map, and its geometry
/// forces the pass to run again while the shadow cache is on: the quad's colour comes from a
/// per-frame uniform, so a kept map would show the first frame's colour on the second. Covers
/// the transient quad chunk and the retained non-indexed chunk a `MeshRef` draw makes.
#[test]
#[ignore = "daily"]
fn mod_shadow_pass_geometry_is_drawn_and_invalidates_the_shadow_cache() {
    let _gpu_guard = crate::gfx::test_gpu::lock();
    let Some(gpu) = Gpu::new() else {
        return;
    };
    for retained in [false, true] {
        let files = vec![
            (
                "shadow.vsh".to_owned(),
                "#version 120\nvoid main(){gl_Position=vec4(2.0,2.0,2.0,1.0);}".to_owned(),
            ),
            (
                "shadow.fsh".to_owned(),
                "#version 120\n/* DRAWBUFFERS:0 */\nvoid main(){gl_FragData[0]=vec4(0.0);}"
                    .to_owned(),
            ),
            (
                "mod_shadow.vsh".to_owned(),
                "#version 120\nvoid main(){gl_Position=vec4(gl_Vertex.xyz,1.0);}".to_owned(),
            ),
            (
                "mod_shadow.fsh".to_owned(),
                "#version 120\n/* DRAWBUFFERS:0 */\nuniform float modColor;\n\
                 void main(){gl_FragData[0]=vec4(modColor,0.0,0.0,1.0);}"
                    .to_owned(),
            ),
            ("final.vsh".to_owned(), VERTEX.to_owned()),
            (
                "final.fsh".to_owned(),
                "#version 120\nuniform sampler2D shadowcolor0;varying vec2 uv;\n\
                 void main(){gl_FragColor=vec4(texture2D(shadowcolor0,uv).rgb,1.0);}"
                    .to_owned(),
            ),
        ];
        let requests = vec![sunrise_shaders::ModPackProgramRequest {
            slot: 0,
            programs: vec!["mod_shadow".into(), "shadow".into()],
            group: sunrise_shaders::Group::Shadow,
            inputs: mod_gbuffers_inputs(),
            default_blend: None,
        }];
        let runtime = OwnedModPackRuntime {
            depth_targets: Vec::new(),
            uniforms: vec![ModUniformDecl {
                name: "modColor".into(),
                ty: ModUniformType::Float,
                previous_name: None,
            }],
            pipelines: vec![ModPipelinePackSpec {
                slot: 0,
                pass_slot: ModPassSlot::Shadow,
                depth_target: None,
                depth_write: true,
                depth_compare: wgpu::CompareFunction::LessEqual,
                depth_bias: Default::default(),
                cull: false,
            }],
        };
        let (mut post, color, depth) = load_with_mods(&gpu, files, requests, &runtime);
        assert!(
            post.is_mod_pipeline_bound(0),
            "mod_shadow not bound: {:?}",
            post.notes
        );
        let (vbuf, ibuf) = mod_quad_buffers(&gpu.device, -0.5);
        let draw = || {
            if retained {
                ModDrawChunk {
                    slot: 0,
                    vertex_buffers: vec![(0, vbuf.clone(), 0..vbuf.size())],
                    kind: ModDrawKind::Indexed {
                        buffer: ibuf.clone(),
                        format: wgpu::IndexFormat::Uint32,
                        indices: 0..6,
                        base_vertex: 0,
                    },
                    instances: 0..1,
                    mod_bind_group: None,
                    mod_globals: None,
                }
            } else {
                ModDrawChunk::quads(0, vbuf.clone(), ibuf.clone(), 0..1)
            }
        };
        for (value, expected) in [(0.25, 64), (0.75, 191)] {
            post.set_mod_draws(vec![draw()]);
            assert!(post.set_mod_uniform("modColor", ModUniformValue::Float(value)));
            let px = run_frame_and_sample(&gpu, &mut post, &color, &depth, true);
            assert!(
                (px[0] as i16 - expected).abs() <= 2,
                "retained {retained}: shadow map shows {px:?}, expected {expected}"
            );
        }
    }
}

fn prelude_mod_layouts(
    device: &wgpu::Device,
    with_quad_vertices: bool,
    set1_layout: Option<wgpu::BindGroupLayout>,
) -> ModPipelineLayouts {
    let buffers = if with_quad_vertices {
        let layout = PipelineVertex::buffer_layout();
        vec![ModVertexBuffer {
            binding: 0,
            stride: layout.array_stride,
            step_mode: layout.step_mode,
            attributes: layout.attributes.to_vec(),
        }]
    } else {
        Vec::new()
    };
    ModPipelineLayouts {
        buffers,
        globals_layout: device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Mod pack prelude test globals"),
            entries: &[],
        }),
        set1_layout,
    }
}

fn prelude_runtime(uniforms: Vec<ModUniformDecl>) -> OwnedModPackRuntime {
    OwnedModPackRuntime {
        depth_targets: Vec::new(),
        uniforms,
        pipelines: vec![ModPipelinePackSpec {
            slot: 0,
            pass_slot: ModPassSlot::AfterOpaqueGbuffers,
            depth_target: None,
            depth_write: true,
            depth_compare: wgpu::CompareFunction::LessEqual,
            depth_bias: Default::default(),
            cull: false,
        }],
    }
}

/// A far-terrain `dh_terrain` program that declares `dhMaterialId`, `mc_Entity`, and `at_tangent`
/// (with `at_tangent` left unset as a private global) and calls a `fragment_prelude` helper
/// sampling a mod set-1 texture translates, binds, and renders expected colors.
#[test]
#[ignore = "daily"]
fn mod_pack_prelude_far_terrain_reads_attributes_helpers_and_mod_set1_texture() {
    let _gpu_guard = crate::gfx::test_gpu::lock();
    let Some(gpu) = Gpu::new() else {
        return;
    };
    let set1_layout = gpu
        .device
        .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Mod pack prelude atlas layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
    let tex = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("Mod pack prelude atlas"),
        size: wgpu::Extent3d {
            width: 1,
            height: 1,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: T::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    gpu.queue.write_texture(
        tex.as_image_copy(),
        &[204u8, 153, 51, 255],
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(4),
            rows_per_image: Some(1),
        },
        wgpu::Extent3d {
            width: 1,
            height: 1,
            depth_or_array_layers: 1,
        },
    );
    let view = tex.create_view(&Default::default());
    let sampler = gpu
        .device
        .create_sampler(&wgpu::SamplerDescriptor::default());
    let set1 = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("Mod pack prelude atlas group"),
        layout: &set1_layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&view),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::Sampler(&sampler),
            },
        ],
    });
    let files = vec![
        (
            "dh_terrain.vsh".to_owned(),
            "#version 150\n\
             in int dhMaterialId;\n\
             in vec4 mc_Entity;\n\
             in vec4 at_tangent;\n\
             out vec2 vUv;\n\
             flat out int vMat;\n\
             out vec2 vAttrs;\n\
             void main() {\n\
                 gl_Position = vec4(gl_Vertex.xyz, 1.0);\n\
                 vUv = gl_MultiTexCoord0.xy;\n\
                 vMat = dhMaterialId;\n\
                 vAttrs = vec2(mc_Entity.x, at_tangent.x);\n\
             }\n"
            .to_owned(),
        ),
        (
            "dh_terrain.fsh".to_owned(),
            "#version 150\n\
             /* DRAWBUFFERS:0 */\n\
             in vec2 vUv;\n\
             flat in int vMat;\n\
             in vec2 vAttrs;\n\
             void main() {\n\
                 vec4 atlas = dh_sampleAtlas(vUv);\n\
                 // Expect vMat == 2, mc_Entity.x == 1.0, at_tangent.x == 0.0.\n\
                 float ok = (vMat == 2 && vAttrs.x > 0.5 && vAttrs.y == 0.0) ? 1.0 : 0.0;\n\
                 gl_FragData[0] = vec4(atlas.rgb * ok, 1.0);\n\
             }\n"
            .to_owned(),
        ),
        ("final.vsh".to_owned(), VERTEX.to_owned()),
        (
            "final.fsh".to_owned(),
            "#version 120\nuniform sampler2D colortex0;varying vec2 uv;\
             void main(){gl_FragColor=texture2D(colortex0,uv);}"
                .to_owned(),
        ),
    ];
    let requests = vec![sunrise_shaders::ModPackProgramRequest {
        slot: 0,
        programs: vec!["dh_terrain".into(), "gbuffers_terrain".into()],
        group: sunrise_shaders::Group::Gbuffers,
        inputs: sunrise_shaders::GbuffersInputs {
            prelude: Some(sunrise_shaders::PackPrelude {
                vertex: Some(
                    "layout(location = 0) in vec3 inPos;\n\
                     layout(location = 1) in vec2 inUv;\n\
                     int dhMaterialId;\n\
                     vec4 mc_Entity;\n\
                     void sunrise_pack_vertex_init() {\n\
                         gl_Vertex = vec4(inPos, 1.0);\n\
                         gl_MultiTexCoord0 = vec4(inUv, 0.0, 1.0);\n\
                         dhMaterialId = 2;\n\
                         mc_Entity = vec4(1.0, 0.0, 0.0, 0.0);\n\
                     }\n"
                    .to_owned(),
                ),
                fragment: Some(
                    "layout(set = 1, binding = 0) uniform sampler2D dhAtlas;\n\
                     vec4 dh_sampleAtlas(vec2 uv) { return texture(dhAtlas, uv); }\n"
                        .to_owned(),
                ),
            }),
            ..Default::default()
        },
        default_blend: None,
    }];
    let runtime = prelude_runtime(Vec::new());
    let layouts = [Some(prelude_mod_layouts(
        &gpu.device,
        true,
        Some(set1_layout),
    ))];
    let (mut post, color, depth) = load_with_mod_layouts(&gpu, files, requests, &runtime, &layouts);
    assert!(
        post.is_mod_pipeline_bound(0),
        "dh_terrain prelude not bound: {:?}",
        post.notes
    );
    let (vbuf, ibuf) = mod_quad_buffers(&gpu.device, -0.5);
    let mut chunk = ModDrawChunk::quads(0, vbuf, ibuf, 0..1);
    chunk.mod_bind_group = Some((set1, Vec::new()));
    post.set_mod_draws(vec![chunk]);
    let px = run_frame_and_sample_center(&gpu, &mut post, &color, &depth);
    assert_eq!(
        px,
        [204, 153, 51, 255],
        "unexpected dh_terrain output {px:?}"
    );
}

/// A `vertex_prelude` with zero vertex inputs pulls its vertices from a mod set-1 storage buffer
/// via `gl_VertexIndex` and draws with no vertex buffers bound.
#[test]
#[ignore = "daily"]
fn mod_pack_prelude_vertex_pull_storage_buffer_with_no_vertex_inputs() {
    let _gpu_guard = crate::gfx::test_gpu::lock();
    let Some(gpu) = Gpu::new() else {
        return;
    };
    let set1_layout = gpu
        .device
        .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Mod pack prelude storage layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
    let verts: [[f32; 4]; 3] = [
        [-1.0, -1.0, -0.5, 1.0],
        [-1.0, 3.0, -0.5, 1.0],
        [3.0, -1.0, -0.5, 1.0],
    ];
    let storage_buf = gpu
        .device
        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Mod pack prelude vertices storage"),
            contents: bytemuck::cast_slice(&verts),
            usage: wgpu::BufferUsages::STORAGE,
        });
    let set1 = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("Mod pack prelude storage group"),
        layout: &set1_layout,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: storage_buf.as_entire_binding(),
        }],
    });
    let files = vec![
        (
            "dh_terrain.vsh".to_owned(),
            "#version 150\n\
             out vec4 vCol;\n\
             void main() {\n\
                 gl_Position = gl_Vertex;\n\
                 vCol = gl_Color;\n\
             }\n"
            .to_owned(),
        ),
        (
            "dh_terrain.fsh".to_owned(),
            "#version 150\n\
             /* DRAWBUFFERS:0 */\n\
             in vec4 vCol;\n\
             void main() { gl_FragData[0] = vCol; }\n"
                .to_owned(),
        ),
        ("final.vsh".to_owned(), VERTEX.to_owned()),
        (
            "final.fsh".to_owned(),
            "#version 120\nuniform sampler2D colortex0;varying vec2 uv;\
             void main(){gl_FragColor=texture2D(colortex0,uv);}"
                .to_owned(),
        ),
    ];
    let requests = vec![sunrise_shaders::ModPackProgramRequest {
        slot: 0,
        programs: vec!["dh_terrain".into()],
        group: sunrise_shaders::Group::Gbuffers,
        inputs: sunrise_shaders::GbuffersInputs {
            prelude: Some(sunrise_shaders::PackPrelude {
                vertex: Some(
                    "layout(set = 1, binding = 0, std430) readonly buffer FarVerts { vec4 pos[]; };\n\
                     void sunrise_pack_vertex_init() {\n\
                         gl_Vertex = pos[gl_VertexIndex];\n\
                         gl_Color = vec4(0.4, 0.8, 0.2, 1.0);\n\
                     }\n"
                        .to_owned(),
                ),
                fragment: None,
            }),
            ..Default::default()
        },
        default_blend: None,
    }];
    let runtime = prelude_runtime(Vec::new());
    let layouts = [Some(prelude_mod_layouts(
        &gpu.device,
        false,
        Some(set1_layout),
    ))];
    let (mut post, color, depth) = load_with_mod_layouts(&gpu, files, requests, &runtime, &layouts);
    assert!(
        post.is_mod_pipeline_bound(0),
        "vertex-pull prelude not bound: {:?}",
        post.notes
    );
    post.set_mod_draws(vec![ModDrawChunk {
        slot: 0,
        vertex_buffers: Vec::new(),
        kind: ModDrawKind::Vertices { vertices: 0..3 },
        instances: 0..1,
        mod_bind_group: Some((set1, Vec::new())),
        mod_globals: None,
    }]);
    let px = run_frame_and_sample_center(&gpu, &mut post, &color, &depth);
    assert_eq!(px, [102, 204, 51, 255], "vertex-pull pixel {px:?}");
}

/// Identical variable and uniform declarations in both the pack and the prelude are deduplicated
/// cleanly (the pack's declaration stands), binding and shading with the mod's frame uniform.
#[test]
#[ignore = "daily"]
fn mod_pack_prelude_deduplicates_matching_declarations_with_pack() {
    let _gpu_guard = crate::gfx::test_gpu::lock();
    let Some(gpu) = Gpu::new() else {
        return;
    };
    let files = vec![
        (
            "dh_terrain.vsh".to_owned(),
            "#version 150\n\
             in int dhMaterialId;\n\
             uniform vec3 modelOffset;\n\
             out vec4 vOut;\n\
             void main() {\n\
                 gl_Position = vec4(gl_Vertex.xyz, 1.0);\n\
                 vOut = vec4(modelOffset.xy, dhMaterialId == 1 ? 0.8 : 0.0, 1.0);\n\
             }\n"
            .to_owned(),
        ),
        (
            "dh_terrain.fsh".to_owned(),
            "#version 150\n\
             /* DRAWBUFFERS:0 */\n\
             in vec4 vOut;\n\
             void main() { gl_FragData[0] = vOut; }\n"
                .to_owned(),
        ),
        ("final.vsh".to_owned(), VERTEX.to_owned()),
        (
            "final.fsh".to_owned(),
            "#version 120\nuniform sampler2D colortex0;varying vec2 uv;\
             void main(){gl_FragColor=texture2D(colortex0,uv);}"
                .to_owned(),
        ),
    ];
    let requests = vec![sunrise_shaders::ModPackProgramRequest {
        slot: 0,
        programs: vec!["dh_terrain".into()],
        group: sunrise_shaders::Group::Gbuffers,
        inputs: sunrise_shaders::GbuffersInputs {
            prelude: Some(sunrise_shaders::PackPrelude {
                vertex: Some(
                    "layout(location = 0) in vec3 inPos;\n\
                     uniform vec3 modelOffset;\n\
                     int dhMaterialId;\n\
                     void sunrise_pack_vertex_init() {\n\
                         gl_Vertex = vec4(inPos, 1.0);\n\
                         dhMaterialId = 1;\n\
                     }\n"
                    .to_owned(),
                ),
                fragment: None,
            }),
            ..Default::default()
        },
        default_blend: None,
    }];
    let runtime = prelude_runtime(vec![ModUniformDecl {
        name: "modelOffset".to_owned(),
        ty: ModUniformType::Vec3,
        previous_name: None,
    }]);
    let layouts = [Some(prelude_mod_layouts(&gpu.device, true, None))];
    let (mut post, color, depth) = load_with_mod_layouts(&gpu, files, requests, &runtime, &layouts);
    assert!(
        post.is_mod_pipeline_bound(0),
        "deduplicated prelude not bound: {:?}",
        post.notes
    );
    let (vbuf, ibuf) = mod_quad_buffers(&gpu.device, -0.5);
    post.set_mod_draws(vec![ModDrawChunk::quads(0, vbuf, ibuf, 0..1)]);
    assert!(post.set_mod_uniform("modelOffset", ModUniformValue::Vec3([0.2, 0.4, 0.0])));
    let px = run_frame_and_sample_center(&gpu, &mut post, &color, &depth);
    assert_eq!(px, [51, 102, 204, 255], "dedup pixel {px:?}");
}

/// When the matched pack program fails translation (such as a function-name collision with the
/// prelude), the slot stays unbound with a logged note and does not fall back to `gbuffers_terrain`.
#[test]
#[ignore = "daily"]
fn mod_pack_prelude_collision_or_failure_leaves_slot_unbound_without_fallback() {
    let _gpu_guard = crate::gfx::test_gpu::lock();
    let Some(gpu) = Gpu::new() else {
        return;
    };
    let files = vec![
        (
            "dh_terrain.vsh".to_owned(),
            "#version 150\nvoid main(){gl_Position=vec4(gl_Vertex.xyz,1.0);}\n".to_owned(),
        ),
        (
            "dh_terrain.fsh".to_owned(),
            "#version 150\n\
             /* DRAWBUFFERS:0 */\n\
             vec4 dh_shade() { return vec4(1.0); }\n\
             void main(){gl_FragData[0]=dh_shade();}\n"
                .to_owned(),
        ),
        (
            "gbuffers_terrain.vsh".to_owned(),
            "#version 150\nvoid main(){gl_Position=vec4(gl_Vertex.xyz,1.0);}\n".to_owned(),
        ),
        (
            "gbuffers_terrain.fsh".to_owned(),
            "#version 150\n/* DRAWBUFFERS:0 */\nvoid main(){gl_FragData[0]=vec4(1.0,0.0,0.0,1.0);}\n"
                .to_owned(),
        ),
        ("final.vsh".to_owned(), VERTEX.to_owned()),
        (
            "final.fsh".to_owned(),
            "#version 120\nuniform sampler2D colortex0;varying vec2 uv;\
             void main(){gl_FragColor=texture2D(colortex0,uv);}"
                .to_owned(),
        ),
    ];
    let requests = vec![sunrise_shaders::ModPackProgramRequest {
        slot: 0,
        programs: vec!["dh_terrain".into(), "gbuffers_terrain".into()],
        group: sunrise_shaders::Group::Gbuffers,
        inputs: sunrise_shaders::GbuffersInputs {
            prelude: Some(sunrise_shaders::PackPrelude {
                vertex: Some(
                    "layout(location = 0) in vec3 inPos;\n\
                     void sunrise_pack_vertex_init() { gl_Vertex = vec4(inPos, 1.0); }\n"
                        .to_owned(),
                ),
                fragment: Some("vec4 dh_shade() { return vec4(0.5); }\n".to_owned()),
            }),
            ..Default::default()
        },
        default_blend: None,
    }];
    let runtime = prelude_runtime(Vec::new());
    let layouts = [Some(prelude_mod_layouts(&gpu.device, true, None))];
    let (post, _color, _depth) = load_with_mod_layouts(&gpu, files, requests, &runtime, &layouts);
    assert!(
        !post.is_mod_pipeline_bound(0),
        "a prelude function collision must leave slot 0 unbound without falling back to gbuffers_terrain"
    );
    assert!(
        post.notes.iter().any(|n| n.contains("dh_shade")),
        "expected collision note naming dh_shade: {:?}",
        post.notes
    );
}

/// A pack program reading `gl_FragCoord` (including from a `fragment_prelude` helper) observes
/// standard-Z (`[0, 1]`, near = 0, far = 1) and OpenGL rows (`y = 0.5` at the bottom, increasing
/// upward). Verified at two screen positions (near top-left and near bottom-right) and two known
/// GL clip-space depths (`z_gl = -0.5` -> `0.25`, `z_gl = +0.5` -> `0.75`).
#[test]
#[ignore = "daily"]
fn mod_pack_prelude_frag_coord_is_standard_z_and_gl_rows() {
    let _gpu_guard = crate::gfx::test_gpu::lock();
    let gpu = Gpu::new().expect("graphics adapter required for pack gl_FragCoord convention test");
    let files = vec![
        (
            "dh_terrain.vsh".to_owned(),
            "#version 150\n\
             out float vGlY;\n\
             void main() {\n\
                 gl_Position = vec4(gl_Vertex.xyz, 1.0);\n\
                 vGlY = gl_Vertex.y;\n\
             }\n"
            .to_owned(),
        ),
        (
            "dh_terrain.fsh".to_owned(),
            "#version 150\n\
             /* DRAWBUFFERS:0 */\n\
             in float vGlY;\n\
             void main() {\n\
                 vec4 fc = prelude_frag_coord();\n\
                 bool glRowOrder = (vGlY > 0.0) == (fc.y > 8.0);\n\
                 gl_FragData[0] = vec4(\n\
                     float(int(fc.x)) / 255.0,\n\
                     float(int(fc.y)) / 255.0,\n\
                     fc.z,\n\
                     glRowOrder ? 1.0 : 0.0\n\
                 );\n\
             }\n"
            .to_owned(),
        ),
        ("final.vsh".to_owned(), VERTEX.to_owned()),
        (
            "final.fsh".to_owned(),
            "#version 120\n\
             uniform sampler2D colortex0;\n\
             varying vec2 uv;\n\
             /*\nconst int colortex0Format = RGBA8;\n*/\n\
             void main() { gl_FragColor = texture2D(colortex0, uv); }\n"
                .to_owned(),
        ),
    ];
    let requests = vec![sunrise_shaders::ModPackProgramRequest {
        slot: 0,
        programs: vec!["dh_terrain".into()],
        group: sunrise_shaders::Group::Gbuffers,
        inputs: sunrise_shaders::GbuffersInputs {
            prelude: Some(sunrise_shaders::PackPrelude {
                vertex: Some(
                    "layout(location = 0) in vec3 inPos;\n\
                     void sunrise_pack_vertex_init() { gl_Vertex = vec4(inPos, 1.0); }\n"
                        .to_owned(),
                ),
                fragment: Some("vec4 prelude_frag_coord() { return gl_FragCoord; }\n".to_owned()),
            }),
            ..Default::default()
        },
        default_blend: None,
    }];
    let runtime = prelude_runtime(Vec::new());
    let layouts = [Some(prelude_mod_layouts(&gpu.device, true, None))];
    let (mut post, color, depth) = load_with_mod_layouts(&gpu, files, requests, &runtime, &layouts);
    assert!(
        post.is_mod_pipeline_bound(0),
        "dh_terrain gl_FragCoord test not bound: {:?}",
        post.notes
    );

    // `SIZE` is 16x16. Screen pixel `[2, 1]` (near top) sits at GL row `16 - 1 - 1 = 14`; screen
    // pixel `[13, 14]` (near bottom) sits at GL row `16 - 1 - 14 = 1`.
    // GL clip z = -0.5 -> standard-Z 0.25 (`round(0.25 * 255) = 64`).
    // GL clip z = +0.5 -> standard-Z 0.75 (`round(0.75 * 255) = 191`).
    for (z_gl, expected_z_byte) in [(-0.5f32, 64u8), (0.5f32, 191u8)] {
        gpu.clear(&color, &depth, 0.0);
        let (vbuf, ibuf) = mod_quad_buffers(&gpu.device, z_gl);
        post.set_mod_draws(vec![ModDrawChunk::quads(0, vbuf, ibuf, 0..1)]);
        let samples = run_frame_and_sample_at(
            &gpu,
            &mut post,
            &color,
            &depth,
            false,
            &[[2, 1], [13, 14]],
            (
                WorldBoundaries::default(),
                |_: &mut wgpu::CommandEncoder, _: WorldBoundary<'_>| false,
            ),
        );
        assert_eq!(
            samples,
            vec![[2, 14, expected_z_byte, 255], [13, 1, expected_z_byte, 255]],
            "z_gl = {z_gl}: expected standard-Z ({expected_z_byte}) and GL rows"
        );
    }
}

/// `bool sunrise_pack_fragment_discard()` samples a mod set-1 coverage-depth texture using the
/// documented `1.0 - gl_FragCoord.z` and GL-row flip to discard the occluded top half of a quad
/// while keeping the bottom half; in `PackPassSlot::Shadow` the function is never called even
/// when it returns `true`.
#[test]
#[ignore = "daily"]
fn mod_pack_prelude_fragment_discard_samples_mod_depth_and_skips_shadow() {
    let _gpu_guard = crate::gfx::test_gpu::lock();
    let gpu = Gpu::new().expect("graphics adapter required for prelude fragment discard test");

    // 1) Non-shadow pass: bind a 16x16 top-down reversed-Z coverage texture in set 1.
    // Top half (wgpu rows 0..8) holds 224/255 (~0.878 > quad revZ 0.75) -> discarded.
    // Bottom half (wgpu rows 8..16) holds 64/255 (~0.251 < quad revZ 0.75) -> kept.
    let set1_layout = gpu
        .device
        .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Mod pack fragment discard coverage layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
    let tex = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("Mod pack fragment discard coverage depth"),
        size: wgpu::Extent3d {
            width: SIZE[0],
            height: SIZE[1],
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: T::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let mut coverage_bytes = Vec::with_capacity((SIZE[0] * SIZE[1] * 4) as usize);
    for y in 0..SIZE[1] {
        let cov = if y < SIZE[1] / 2 { 224u8 } else { 64u8 };
        for _ in 0..SIZE[0] {
            coverage_bytes.extend_from_slice(&[cov, 0, 0, 255]);
        }
    }
    gpu.queue.write_texture(
        tex.as_image_copy(),
        &coverage_bytes,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(SIZE[0] * 4),
            rows_per_image: Some(SIZE[1]),
        },
        wgpu::Extent3d {
            width: SIZE[0],
            height: SIZE[1],
            depth_or_array_layers: 1,
        },
    );
    let view = tex.create_view(&Default::default());
    let sampler = gpu
        .device
        .create_sampler(&wgpu::SamplerDescriptor::default());
    let set1 = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("Mod pack fragment discard coverage group"),
        layout: &set1_layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&view),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::Sampler(&sampler),
            },
        ],
    });

    let files = vec![
        (
            "dh_terrain.vsh".to_owned(),
            "#version 150\nvoid main() { gl_Position = vec4(gl_Vertex.xyz, 1.0); }\n".to_owned(),
        ),
        (
            "dh_terrain.fsh".to_owned(),
            "#version 150\n/* DRAWBUFFERS:0 */\nvoid main() { gl_FragData[0] = vec4(0.0, 1.0, 0.0, 1.0); }\n"
                .to_owned(),
        ),
        ("final.vsh".to_owned(), VERTEX.to_owned()),
        (
            "final.fsh".to_owned(),
            "#version 120\n\
             uniform sampler2D colortex0;\n\
             varying vec2 uv;\n\
             /*\nconst int colortex0Format = RGBA8;\n*/\n\
             void main() { gl_FragColor = texture2D(colortex0, uv); }\n"
                .to_owned(),
        ),
    ];
    let requests = vec![sunrise_shaders::ModPackProgramRequest {
        slot: 0,
        programs: vec!["dh_terrain".into()],
        group: sunrise_shaders::Group::Gbuffers,
        inputs: sunrise_shaders::GbuffersInputs {
            prelude: Some(sunrise_shaders::PackPrelude {
                vertex: Some(
                    "layout(location = 0) in vec3 inPos;\n\
                     void sunrise_pack_vertex_init() { gl_Vertex = vec4(inPos, 1.0); }\n"
                        .to_owned(),
                ),
                fragment: Some(
                    "layout(set = 1, binding = 0) uniform sampler2D coverageDepth;\n\
                     bool sunrise_pack_fragment_discard() {\n\
                         ivec2 sz = textureSize(coverageDepth, 0);\n\
                         ivec2 px = ivec2(int(gl_FragCoord.x), sz.y - 1 - int(gl_FragCoord.y));\n\
                         float cov = texelFetch(coverageDepth, px, 0).r;\n\
                         float revZ = 1.0 - gl_FragCoord.z;\n\
                         return revZ < cov;\n\
                     }\n"
                    .to_owned(),
                ),
            }),
            ..Default::default()
        },
        default_blend: None,
    }];
    let runtime = prelude_runtime(Vec::new());
    let layouts = [Some(prelude_mod_layouts(
        &gpu.device,
        true,
        Some(set1_layout),
    ))];
    let (mut post, color, depth) = load_with_mod_layouts(&gpu, files, requests, &runtime, &layouts);
    assert!(
        post.is_mod_pipeline_bound(0),
        "dh_terrain fragment discard not bound: {:?}",
        post.notes
    );
    // GL clip z = -0.5 -> standard-Z 0.25 -> reversed-Z 0.75.
    let (vbuf, ibuf) = mod_quad_buffers(&gpu.device, -0.5);
    let mut chunk = ModDrawChunk::quads(0, vbuf, ibuf, 0..1);
    chunk.mod_bind_group = Some((set1, Vec::new()));
    post.set_mod_draws(vec![chunk]);
    let samples = run_frame_and_sample_at(
        &gpu,
        &mut post,
        &color,
        &depth,
        false,
        &[[4, 2], [4, 13]],
        (
            WorldBoundaries::default(),
            |_: &mut wgpu::CommandEncoder, _: WorldBoundary<'_>| false,
        ),
    );
    // Top half `[4, 2]` is discarded (keeps `Gpu::clear`'s 0.0625 -> 16/255); bottom half `[4, 13]`
    // stays green.
    assert_eq!(
        samples,
        vec![[16, 16, 16, 255], [0, 255, 0, 255]],
        "top half must be discarded and bottom half kept: {samples:?}"
    );

    // 2) Shadow pass: `sunrise_pack_fragment_discard()` returning `true` is never called.
    let shadow_files = vec![
        (
            "shadow.vsh".to_owned(),
            "#version 120\nvoid main(){gl_Position=vec4(2.0,2.0,2.0,1.0);}".to_owned(),
        ),
        (
            "shadow.fsh".to_owned(),
            "#version 120\n/* DRAWBUFFERS:0 */\nvoid main(){gl_FragData[0]=vec4(0.0);}".to_owned(),
        ),
        (
            "mod_shadow.vsh".to_owned(),
            "#version 150\nvoid main(){gl_Position=vec4(gl_Vertex.xyz,1.0);}\n".to_owned(),
        ),
        (
            "mod_shadow.fsh".to_owned(),
            "#version 150\n/* DRAWBUFFERS:0 */\nvoid main(){gl_FragData[0]=vec4(0.0,0.0,1.0,1.0);}\n"
                .to_owned(),
        ),
        ("final.vsh".to_owned(), VERTEX.to_owned()),
        (
            "final.fsh".to_owned(),
            "#version 120\nuniform sampler2D shadowcolor0;varying vec2 uv;\n\
             void main(){gl_FragColor=vec4(texture2D(shadowcolor0,uv).rgb,1.0);}"
                .to_owned(),
        ),
    ];
    let shadow_requests = vec![sunrise_shaders::ModPackProgramRequest {
        slot: 0,
        programs: vec!["mod_shadow".into(), "shadow".into()],
        group: sunrise_shaders::Group::Shadow,
        inputs: sunrise_shaders::GbuffersInputs {
            prelude: Some(sunrise_shaders::PackPrelude {
                vertex: Some(
                    "layout(location = 0) in vec3 inPos;\n\
                     void sunrise_pack_vertex_init() { gl_Vertex = vec4(inPos, 1.0); }\n"
                        .to_owned(),
                ),
                fragment: Some(
                    "bool sunrise_pack_fragment_discard() { return true; }\n".to_owned(),
                ),
            }),
            ..Default::default()
        },
        default_blend: None,
    }];
    let shadow_runtime = OwnedModPackRuntime {
        depth_targets: Vec::new(),
        uniforms: Vec::new(),
        pipelines: vec![ModPipelinePackSpec {
            slot: 0,
            pass_slot: ModPassSlot::Shadow,
            depth_target: None,
            depth_write: true,
            depth_compare: wgpu::CompareFunction::LessEqual,
            depth_bias: Default::default(),
            cull: false,
        }],
    };
    let shadow_layouts = [Some(prelude_mod_layouts(&gpu.device, true, None))];
    let (mut shadow_post, shadow_color, shadow_depth) = load_with_mod_layouts(
        &gpu,
        shadow_files,
        shadow_requests,
        &shadow_runtime,
        &shadow_layouts,
    );
    assert!(
        shadow_post.is_mod_pipeline_bound(0),
        "mod_shadow prelude not bound: {:?}",
        shadow_post.notes
    );
    let (svbuf, sibuf) = mod_quad_buffers(&gpu.device, -0.5);
    shadow_post.set_mod_draws(vec![ModDrawChunk::quads(0, svbuf, sibuf, 0..1)]);
    let shadow_px =
        run_frame_and_sample_center(&gpu, &mut shadow_post, &shadow_color, &shadow_depth);
    assert_eq!(
        shadow_px,
        [0, 0, 255, 255],
        "shadow pass must ignore sunrise_pack_fragment_discard: {shadow_px:?}"
    );
}

/// A pack-linked `ModDrawKind::Indirect` draws two indexed commands, the left and the right half
/// of the screen. Without a count buffer both draw; with one (where the adapter draws GPU-written
/// counts, #5908) holding 1, only command 0 does.
#[test]
#[ignore = "daily"]
fn mod_pack_indirect_draw_with_a_count_buffer_draws_only_the_counted_commands() {
    let _gpu_guard = crate::gfx::test_gpu::lock();
    let Some(gpu) = Gpu::with_features(wgpu::Features::MULTI_DRAW_INDIRECT_COUNT) else {
        return;
    };
    let counts = crate::gfx::mod_render_targets::multi_draw_indirect_count(
        &gpu.adapter.get_info(),
        gpu.device.features(),
    );
    let files = vec![
        (
            "mod_terrain.vsh".to_owned(),
            "#version 120\nvoid main(){gl_Position=vec4(gl_Vertex.xyz,1.0);}".to_owned(),
        ),
        (
            "mod_terrain.fsh".to_owned(),
            "#version 120\n/* DRAWBUFFERS:0 */\nvoid main(){gl_FragData[0]=vec4(1.0,0.0,0.0,1.0);}"
                .to_owned(),
        ),
        ("final.vsh".to_owned(), VERTEX.to_owned()),
        (
            "final.fsh".to_owned(),
            "#version 120\nuniform sampler2D colortex0;varying vec2 uv;\
             void main(){gl_FragColor=texture2D(colortex0,uv);}"
                .to_owned(),
        ),
    ];
    let init = |contents: &[u8], usage| {
        gpu.device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("Mod pack indirect test"),
                contents,
                usage,
            })
    };
    // Two quads: x in -1..0 (vertices 0..4) and 0..1 (vertices 4..8).
    let color = u32::from_le_bytes([255, 255, 255, 255]);
    let vertex = |x: f32, y: f32| PipelineVertex {
        pos: [x, y, -0.5],
        uv: [0.5, 0.5],
        color,
    };
    let vertices = [
        vertex(-1.0, -1.0),
        vertex(-1.0, 1.0),
        vertex(0.0, 1.0),
        vertex(0.0, -1.0),
        vertex(0.0, -1.0),
        vertex(0.0, 1.0),
        vertex(1.0, 1.0),
        vertex(1.0, -1.0),
    ];
    let vbuf = init(bytemuck::cast_slice(&vertices), wgpu::BufferUsages::VERTEX);
    let vbuf_len = vbuf.size();
    let indices: [u32; 12] = [0, 1, 2, 0, 2, 3, 4, 5, 6, 4, 6, 7];
    let ibuf = init(bytemuck::cast_slice(&indices), wgpu::BufferUsages::INDEX);
    // `index_count, instance_count, first_index, base_vertex, first_instance`.
    let commands: [[u32; 5]; 2] = [[6, 1, 0, 0, 0], [6, 1, 6, 0, 0]];
    let command_buffer = init(
        bytemuck::cast_slice(&commands),
        wgpu::BufferUsages::INDIRECT,
    );
    let count_buffer = init(bytemuck::cast_slice(&[1u32]), wgpu::BufferUsages::INDIRECT);
    let requests = vec![sunrise_shaders::ModPackProgramRequest {
        slot: 0,
        programs: vec!["mod_terrain".into()],
        group: sunrise_shaders::Group::Gbuffers,
        inputs: mod_gbuffers_inputs(),
        default_blend: None,
    }];
    let runtime = OwnedModPackRuntime {
        depth_targets: Vec::new(),
        uniforms: Vec::new(),
        pipelines: vec![ModPipelinePackSpec {
            slot: 0,
            pass_slot: ModPassSlot::AfterOpaqueGbuffers,
            depth_target: None,
            depth_write: true,
            depth_compare: wgpu::CompareFunction::LessEqual,
            depth_bias: Default::default(),
            cull: false,
        }],
    };
    // The final image at the middle of each half, drawn with `count_buffer`.
    let draw = |count_buffer: Option<(wgpu::Buffer, u64)>| {
        let (mut post, color, depth) =
            load_with_mods(&gpu, files.clone(), requests.clone(), &runtime);
        assert!(
            post.is_mod_pipeline_bound(0),
            "mod_terrain not bound: {:?}",
            post.notes
        );
        post.set_mod_draws(vec![ModDrawChunk {
            slot: 0,
            vertex_buffers: vec![(0, vbuf.clone(), 0..vbuf_len)],
            kind: ModDrawKind::Indirect {
                buffer: command_buffer.clone(),
                offset: 0,
                count: 2,
                count_buffer,
                index: Some((ibuf.clone(), wgpu::IndexFormat::Uint32)),
            },
            instances: 0..1,
            mod_bind_group: None,
            mod_globals: None,
        }]);
        run_frame_and_sample_at(
            &gpu,
            &mut post,
            &color,
            &depth,
            false,
            &[[SIZE[0] / 4, SIZE[1] / 2], [SIZE[0] * 3 / 4, SIZE[1] / 2]],
            (
                WorldBoundaries::default(),
                |_: &mut wgpu::CommandEncoder, _: WorldBoundary<'_>| false,
            ),
        )
    };
    let both = draw(None);
    assert!(both[0][0] >= 250, "command 0 without a count: {both:?}");
    assert!(both[1][0] >= 250, "command 1 without a count: {both:?}");
    if !counts {
        eprintln!(
            "no count draws on {:?}: the counted draw is not run",
            gpu.adapter.get_info()
        );
        return;
    }
    let counted = draw(Some((count_buffer, 0)));
    assert!(counted[0][0] >= 250, "command 0 is counted: {counted:?}");
    // Command 1 left the fixture's clear colour (`Gpu::clear`: 0.0625 per channel, 16 of 255).
    assert_eq!(
        counted[1],
        [16, 16, 16, 255],
        "command 1 is past the count of 1: {counted:?}"
    );
}
