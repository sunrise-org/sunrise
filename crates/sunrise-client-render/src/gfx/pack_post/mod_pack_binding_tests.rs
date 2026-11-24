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
    let inputs = sunrise_shaders::GameInputs {
        mod_programs: requests,
        ..Default::default()
    };
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
        runtime.as_runtime(&mod_buffers),
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
    let screen = gpu.texture(T::Rgba8Unorm);
    let stride = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    let read = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("Mod pack binding readback"),
        size: u64::from(stride * SIZE[1]),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let hud = HudBatch::scratch(SIZE);
    let frame = fixture_frame(&hud);
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
        WorldBoundaries::default(),
        |_, _| false,
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
    let offset = (SIZE[1] / 2) as usize * stride as usize + (SIZE[0] / 2) as usize * 4;
    let px = [
        data[offset],
        data[offset + 1],
        data[offset + 2],
        data[offset + 3],
    ];
    drop(data);
    read.unmap();
    px
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
            depth_test: true,
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
    post.set_mod_draws(vec![ModDrawChunk {
        slot: 0,
        vertices: vbuf.clone(),
        indices: ibuf.clone(),
        quads: 0..1,
    }]);

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
    post.set_mod_draws(vec![ModDrawChunk {
        slot: 0,
        vertices: vbuf,
        indices: ibuf,
        quads: 0..1,
    }]);
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
            depth_test: true,
        }],
    };
    let (mut post, color, depth) = load_with_mods(&gpu, files, requests, &runtime);
    assert!(
        post.is_mod_pipeline_bound(0),
        "expected fallback to gbuffers_terrain: {:?}",
        post.notes
    );
    let (vbuf, ibuf) = mod_quad_buffers(&gpu.device, 0.0);
    post.set_mod_draws(vec![ModDrawChunk {
        slot: 0,
        vertices: vbuf,
        indices: ibuf,
        quads: 0..1,
    }]);
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
            depth_test: true,
        }],
    };
    let (mut post, color, depth) = load_with_mods(&gpu, files, requests, &runtime);
    assert!(post.is_mod_pipeline_bound(0));
    let (vbuf, ibuf) = mod_quad_buffers(&gpu.device, 0.0);
    post.set_mod_draws(vec![ModDrawChunk {
        slot: 0,
        vertices: vbuf,
        indices: ibuf,
        quads: 0..1,
    }]);
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
                depth_test: true,
            },
            ModPipelinePackSpec {
                slot: 1,
                pass_slot: ModPassSlot::AfterOpaqueGbuffers,
                depth_target: None,
                depth_write: true,
                depth_test: true,
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
