//! Regression tests for #5370: mod `RenderLevelStage` world draws (`WorldPrimitives`)
//! inside an active shader pack (`AfterOpaqueBlocks` against `colortex0` + `depthtex0`
//! before `final`) and post-pack consumers depth-testing against the exported reversed-Z
//! `world_depth` after `final`.
use super::depth_test::{Gpu, SIZE, VERTEX, fixture_frame};
use super::*;
use crate::gfx::Vertex;
use crate::gfx::world_primitives::{self, Batch, DrawRanges, WorldPrimitives};
use crate::hud::HudBatch;
use glam::Mat4;
use wgpu::{TextureFormat as T, util::DeviceExt};

const RED: [u8; 3] = [255, 0, 0];
const GREEN: [u8; 3] = [0, 255, 0];
const BLUE: [u8; 3] = [0, 0, 255];

fn rgba(rgb: [u8; 3]) -> u32 {
    u32::from_le_bytes([rgb[0], rgb[1], rgb[2], 255])
}

/// Screen-space quad in `Mat4::IDENTITY` clip space (wgpu NDC: `+y` is screen top, reversed-Z
/// depth in `[0..1]` where larger is nearer).
fn ndc_quad([x0, y0, x1, y1]: [f32; 4], reverse_z: f32, rgb: [u8; 3]) -> [Vertex; 4] {
    let color = rgba(rgb);
    [[x0, y0], [x0, y1], [x1, y1], [x1, y0]].map(|[x, y]| Vertex {
        pos: [x, y, reverse_z],
        uv: [0.5, 0.5],
        layer: 0,
        color,
        light: 0,
        normal: [0; 4],
        glint_uv: [0.0, 0.0],
    })
}

/// What every mod stage test draws against: a device (required: a missing adapter fails the
/// test rather than passing it), cleared `colortex0`/depth inputs, the pack `files` loaded with
/// solid terrain, one solid terrain quad over the left half of the frame (`x in [-1, 0]`) and a
/// white 1x1 atlas.
struct StageFixture {
    gpu: Gpu,
    color: wgpu::TextureView,
    depth: wgpu::TextureView,
    post: PackPost,
    terrain_buf: wgpu::Buffer,
    atlas_view: wgpu::TextureView,
    screen: wgpu::TextureView,
    read: wgpu::Buffer,
}

/// Bytes per row of the readback buffer, aligned as texture copies require.
const READBACK_STRIDE: u32 = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;

fn stage_fixture(files: Vec<(String, String)>) -> StageFixture {
    stage_fixture_with_gpu(
        Gpu::new().expect("graphics adapter required for the mod stage tests"),
        files,
    )
}

fn stage_fixture_with_gpu(gpu: Gpu, files: Vec<(String, String)>) -> StageFixture {
    let terrain_inputs = sunrise_shaders::GbuffersInputs {
        declarations: "layout(location=0)in vec3 fixture_position;\n\
                       layout(location=2)in uint hz_v_layer;\n\
                       layout(location=3)in vec4 hz_v_color;\n"
            .into(),
        body: "hz_Vertex=vec4(fixture_position,1.0);hz_Color=vec4(1.0);hz_MultiTexCoord0=vec4(0.0);\
               hz_MultiTexCoord1=vec4(240.0);hz_MultiTexCoord2=vec4(240.0);hz_Normal=vec3(0.0,0.0,1.0);\n"
            .into(),
        ..Default::default()
    };
    let inputs = crate::gfx::pack_atlas::game_terrain(Some(terrain_inputs), &[]);
    const ATTRS: [wgpu::VertexAttribute; 3] =
        wgpu::vertex_attr_array![0=>Float32x3,2=>Uint32,3=>Unorm8x4];
    let layouts = [Some(wgpu::VertexBufferLayout {
        array_stride: 20,
        step_mode: wgpu::VertexStepMode::Vertex,
        attributes: &ATTRS,
    })];
    let color = gpu.texture(T::Rgba8Unorm);
    let depth = gpu.texture(T::Depth32Float);
    gpu.clear(&color, &depth, 0.0);
    let post = gpu.load(
        files,
        World {
            atlas: None,
            material_maps: None,
            lightmap: color.clone(),
            terrain: Some(Geometry {
                buffers: &layouts,
                table: None,
                depth_write: true,
                atlas: None,
                material_maps: false,
                ids: None,
                cull: false,
                pages: false,
            }),
            block_ids: &[],
            sprite_rects: &[],
            celestial: None,
            gui_size: SIZE,
            clouds: None,
            weather: false,
            hand: false,
        },
        inputs,
    );
    assert!(post.has_terrain(), "pack terrain refused: {:?}", post.notes);

    // One solid terrain quad covering the left half of the frame (`x in [-1, 0]`).
    let opaque = u32::from(crate::mesher::OPAQUE_LAYER) | 1;
    let corners = [[-1.0f32, -1.0], [0.0, -1.0], [0.0, 1.0], [-1.0, 1.0]];
    let mut terrain_verts: Vec<u32> = Vec::new();
    for c in [0, 1, 2, 0, 2, 3] {
        let [x, y] = corners[c];
        terrain_verts.extend([
            x.to_bits(),
            y.to_bits(),
            0f32.to_bits(),
            opaque,
            0xFFFF_FFFFu32,
        ]);
    }
    let terrain_buf = gpu
        .device
        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Mod stage terrain fixture"),
            contents: bytemuck::cast_slice(&terrain_verts),
            usage: wgpu::BufferUsages::VERTEX,
        });

    let atlas = gpu.device.create_texture_with_data(
        &gpu.queue,
        &wgpu::TextureDescriptor {
            label: Some("Mod stage white atlas"),
            size: wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: T::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        },
        wgpu::util::TextureDataOrder::LayerMajor,
        &[255, 255, 255, 255],
    );
    let atlas_view = atlas.create_view(&Default::default());
    let screen = gpu.texture(T::Rgba8Unorm);
    let read = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("Mod stage readback"),
        size: u64::from(READBACK_STRIDE * SIZE[1]),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    StageFixture {
        gpu,
        color,
        depth,
        post,
        terrain_buf,
        atlas_view,
        screen,
        read,
    }
}

/// Runs a shader pack whose `gbuffers_terrain` paints the left half (`x in [-1, 0]`) green at
/// GL depth `0.5` and whose `final` swaps R and B of `colortex0`. When `inside_pack` is true,
/// `quads` draw at `WorldBoundary::AfterOpaqueBlocks` against `colortex0` + `depthtex0`; when
/// false, `quads` draw onto `screen` against the exported reversed-Z `world_depth` after `final`.
/// Returns the screen pixels `[r, g, b]` at top-left `(2, 2)`, bottom-left `(2, 12)`, and
/// bottom-right `(12, 12)`.
fn render_stage_quads(inside_pack: bool, quads: &[Vertex]) -> [[u8; 3]; 3] {
    let files = vec![
        (
            "gbuffers_terrain.vsh".to_owned(),
            "#version 120\nvoid main(){gl_Position=vec4(gl_Vertex.xy,0.0,1.0);}".to_owned(),
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
             const vec4 colortex0ClearColor = vec4(0.0, 0.0, 0.0, 1.0);\
             void main(){vec4 c=texture2D(colortex0,uv);gl_FragColor=vec4(c.b,c.g,c.r,1.0);}"
                .to_owned(),
        ),
    ];
    let StageFixture {
        gpu,
        color,
        depth,
        mut post,
        terrain_buf,
        atlas_view,
        screen,
        read,
    } = stage_fixture(files);
    let mut primitives = WorldPrimitives::new(&gpu.device, (T::Rgba8Unorm, false));

    let quad_count = (quads.len() / 4) as u32;
    let stage = if inside_pack {
        world_primitives::STAGE_AFTER_OPAQUE_BLOCKS
    } else {
        world_primitives::STAGE_CUSTOM_GEOMETRY
    };
    let mut stage_ends = [[0u32; 4]; world_primitives::STAGES - 1];
    for end in stage_ends.iter_mut().skip(stage) {
        end[0] = quad_count;
    }
    let batch = Batch {
        quads,
        stage_ends,
        ..Default::default()
    };
    assert!(primitives.prepare(
        &gpu.device,
        &mut |buffer, bytes| gpu.queue.write_buffer(buffer, 0, bytes),
        &batch,
        Mat4::IDENTITY,
        SIZE,
        &atlas_view,
        None,
    ));
    if inside_pack {
        primitives.prepare_pack_pass(&gpu.device, post.formats[0], post.rgb & 1 != 0, false);
    }

    let draws = [DrawRanges {
        primitives: [0..quad_count, 0..0, 0..0, 0..0],
        ..Default::default()
    }];
    let hud = HudBatch::scratch(SIZE);
    let frame = fixture_frame(&hud);
    let mut encoder = gpu.device.create_command_encoder(&Default::default());
    let mut boundary_called = false;
    post.run(
        &gpu.device,
        &gpu.queue,
        &mut encoder,
        &color,
        &depth,
        &screen,
        &frame,
        &Default::default(),
        SIZE,
        SIZE,
        &[],
        &mut |rp, list, _, _| {
            if list == TerrainList::Solid {
                rp.set_vertex_buffer(0, terrain_buf.slice(..));
                rp.draw(0..6, 0..1);
            }
        },
        WorldBoundaries {
            opaque_blocks: inside_pack,
            ..Default::default()
        },
        |enc, boundary| {
            if let WorldBoundary::AfterOpaqueBlocks { color, depth } = boundary {
                boundary_called = true;
                return primitives.draw_pack_prepared(enc, color, depth, &draws);
            }
            false
        },
        &std::sync::atomic::AtomicBool::new(false),
        super::super::in_flight::poll_live,
    )
    .unwrap();
    assert_eq!(boundary_called, inside_pack);
    if !inside_pack {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Post-pack exported depth draw"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &screen,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &depth,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            ..Default::default()
        });
        assert!(primitives.draw_prepared(&mut pass, None, &draws));
    }
    encoder.copy_texture_to_buffer(
        screen.texture().as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &read,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(READBACK_STRIDE),
                rows_per_image: None,
            },
        },
        screen.texture().size(),
    );
    gpu.queue.submit([encoder.finish()]);
    let slice = read.slice(..);
    let (tx, rx) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |r| {
        let _ = tx.send(r);
    });
    let _ = gpu.device.poll(wgpu::PollType::wait_indefinitely());
    rx.recv().unwrap().unwrap();
    let mapped = slice.get_mapped_range().unwrap();
    let sample = |x: usize, y: usize| -> [u8; 3] {
        let base = y * READBACK_STRIDE as usize + x * 4;
        [mapped[base], mapped[base + 1], mapped[base + 2]]
    };
    let out = [sample(2, 2), sample(2, 12), sample(12, 12)];
    drop(mapped);
    read.unmap();
    out
}

#[test]
#[ignore = "daily"]
fn after_opaque_blocks_depth_tests_against_pack_terrain_and_runs_before_final() {
    let _gpu = crate::gfx::test_gpu::lock();
    // Farther full-screen red quad (reversed-Z 0.25 -> GL 0.75 > terrain 0.5) plus a nearer
    // top-half red quad (reversed-Z 0.75 -> GL 0.25 < terrain 0.5). `final` swaps R <-> B so
    // any red fragment drawn inside the pack reads back as blue.
    let mut quads = Vec::new();
    quads.extend_from_slice(&ndc_quad([-1.0, -1.0, 1.0, 1.0], 0.25, RED));
    quads.extend_from_slice(&ndc_quad([-1.0, 0.0, 1.0, 1.0], 0.75, RED));
    let [top_left, bottom_left, bottom_right] = render_stage_quads(true, &quads);
    assert_eq!(
        top_left, BLUE,
        "nearer AfterOpaqueBlocks quad must draw in front of pack terrain and pass through final"
    );
    assert_eq!(
        bottom_left, GREEN,
        "farther AfterOpaqueBlocks quad must be occluded by pack terrain depth"
    );
    assert_eq!(
        bottom_right, BLUE,
        "farther AfterOpaqueBlocks quad must appear where pack terrain is absent and pass through final"
    );
}

#[test]
#[ignore = "daily"]
fn post_pack_world_depth_export_depth_tests_against_pack_terrain() {
    let _gpu = crate::gfx::test_gpu::lock();
    // Same geometry drawn after `final` (red stays unswapped) against reversed-Z `world_depth`
    // exported from `depthtex0` by `fs_export_depth`.
    let mut quads = Vec::new();
    quads.extend_from_slice(&ndc_quad([-1.0, -1.0, 1.0, 1.0], 0.25, RED));
    quads.extend_from_slice(&ndc_quad([-1.0, 0.0, 1.0, 1.0], 0.75, RED));
    let [top_left, bottom_left, bottom_right] = render_stage_quads(false, &quads);
    assert_eq!(
        top_left, RED,
        "nearer post-pack quad must draw in front of exported pack depth without final's swap"
    );
    assert_eq!(
        bottom_left, GREEN,
        "farther post-pack quad must be occluded by exported pack depth"
    );
    assert_eq!(
        bottom_right, RED,
        "farther post-pack quad must appear where pack terrain is absent without final's swap"
    );
}

#[test]
#[ignore = "daily"]
fn pack_boundaries_fire_in_order_and_draw_pack_custom_depth_tests_mod_pipelines() {
    use crate::gfx::mod_pipelines::{Batch as PipelineBatch, PipelineRun, PipelineVertex};
    use crate::mod_pipelines::Pipelines;
    use mod_api::{Identifier, PipelineBlend, RenderPipeline, WorldDepth};

    let _gpu = crate::gfx::test_gpu::lock();
    let files = vec![
        (
            "gbuffers_terrain.vsh".to_owned(),
            "#version 120\nvoid main(){gl_Position=vec4(gl_Vertex.xy,0.0,1.0);}".to_owned(),
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
             const vec4 colortex0ClearColor = vec4(0.0, 0.0, 0.0, 1.0);\
             void main(){vec4 c=texture2D(colortex0,uv);gl_FragColor=vec4(c.b,c.g,c.r,1.0);}"
                .to_owned(),
        ),
    ];
    let StageFixture {
        gpu,
        color,
        depth,
        mut post,
        terrain_buf,
        atlas_view,
        screen,
        read,
    } = stage_fixture(files);

    let id = Identifier::parse("test:pack_custom").unwrap();
    let shader = Identifier::parse("test:core/pack_custom").unwrap();
    let registered =
        [
            RenderPipeline::new(id, shader, PipelineBlend::Translucent, WorldDepth::Tested)
                .vertex_shader(Identifier::parse("minecraft:core/position_tex_color").unwrap())
                .optional(),
        ];
    let fsh = "#version 330\n#include <minecraft:dynamictransforms.glsl>\n\
               in vec2 texCoord0;\nin vec4 vertexColor;\nout vec4 fragColor;\n\
               void main() { fragColor = vertexColor * ColorModulator; }\n";
    let set = Pipelines::build(&registered, 1, |file| match file {
        "assets/test/shaders/core/pack_custom.fsh" => Ok(Some(fsh.to_owned())),
        other => {
            let rest = other.strip_prefix("assets/").unwrap();
            let (ns, path) = rest.split_once('/').unwrap();
            Ok(sunrise_mod_pipeline::stubs::read(&format!("{ns}:{path}")).map(str::to_owned))
        }
    })
    .unwrap();
    let mut primitives = WorldPrimitives::new(&gpu.device, (T::Rgba8Unorm, false));
    primitives.pipelines.build(&gpu.device, &set, None).unwrap();

    let pipe_quad = |[x0, y0, x1, y1]: [f32; 4], reverse_z: f32, rgb: [u8; 3]| {
        let color = rgba(rgb);
        [[x0, y0], [x0, y1], [x1, y1], [x1, y0]].map(|[x, y]| PipelineVertex {
            pos: [x, y, reverse_z],
            uv: [0.5, 0.5],
            color,
        })
    };
    let mut pipe_verts = Vec::new();
    pipe_verts.extend_from_slice(&pipe_quad([-1.0, -1.0, 1.0, 1.0], 0.25, RED));
    pipe_verts.extend_from_slice(&pipe_quad([-1.0, 0.0, 1.0, 1.0], 0.75, RED));
    let runs = [PipelineRun {
        slot: 0,
        stage: world_primitives::STAGE_CUSTOM_GEOMETRY as u32,
        quads: 2,
        modifier_stack: 0,
    }];
    let batch = Batch {
        pipelines: PipelineBatch {
            set: Some(&set),
            vertices: &pipe_verts,
            runs: &runs,
            time: 0.0,
            ..Default::default()
        },
        ..Default::default()
    };
    assert!(primitives.prepare(
        &gpu.device,
        &mut |buffer, bytes| gpu.queue.write_buffer(buffer, 0, bytes),
        &batch,
        Mat4::IDENTITY,
        SIZE,
        &atlas_view,
        None,
    ));
    primitives.prepare_pack_pass(&gpu.device, post.formats[0], post.rgb & 1 != 0, false);

    let hud = HudBatch::scratch(SIZE);
    let frame = fixture_frame(&hud);
    let mut encoder = gpu.device.create_command_encoder(&Default::default());
    let mut seen = Vec::new();
    let mut custom_drew = false;
    post.run(
        &gpu.device,
        &gpu.queue,
        &mut encoder,
        &color,
        &depth,
        &screen,
        &frame,
        &Default::default(),
        SIZE,
        SIZE,
        &[],
        &mut |rp, list, _, _| {
            if list == TerrainList::Solid {
                rp.set_vertex_buffer(0, terrain_buf.slice(..));
                rp.draw(0..6, 0..1);
            }
        },
        WorldBoundaries {
            sky: true,
            opaque_blocks: true,
            features: false,
            translucent_blocks: true,
            weather: true,
            custom_geometry: true,
            export_opaque_depth: false,
        },
        |enc, boundary| match boundary {
            WorldBoundary::AfterSky => {
                seen.push("AfterSky");
                false
            }
            WorldBoundary::AfterOpaqueBlocks { .. } => {
                seen.push("AfterOpaqueBlocks");
                false
            }
            WorldBoundary::Features { .. } => {
                seen.push("Features");
                false
            }
            WorldBoundary::AfterTranslucentBlocks { .. } => {
                seen.push("AfterTranslucentBlocks");
                false
            }
            WorldBoundary::AfterWeather { .. } => {
                seen.push("AfterWeather");
                false
            }
            WorldBoundary::CustomGeometry { color, depth } => {
                seen.push("CustomGeometry");
                custom_drew = primitives.draw_pack_custom(enc, color, depth);
                custom_drew
            }
            WorldBoundary::AfterLevel => {
                seen.push("AfterLevel");
                false
            }
        },
        &std::sync::atomic::AtomicBool::new(false),
        super::super::in_flight::poll_live,
    )
    .unwrap();
    assert_eq!(
        seen,
        [
            "AfterSky",
            "AfterOpaqueBlocks",
            "AfterTranslucentBlocks",
            "AfterWeather",
            "CustomGeometry",
            "AfterLevel",
        ]
    );
    assert!(
        custom_drew,
        "draw_pack_custom must draw prepared ModPipelines runs"
    );

    encoder.copy_texture_to_buffer(
        screen.texture().as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &read,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(READBACK_STRIDE),
                rows_per_image: None,
            },
        },
        screen.texture().size(),
    );
    gpu.queue.submit([encoder.finish()]);
    let slice = read.slice(..);
    let (tx, rx) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |r| {
        let _ = tx.send(r);
    });
    let _ = gpu.device.poll(wgpu::PollType::wait_indefinitely());
    rx.recv().unwrap().unwrap();
    let mapped = slice.get_mapped_range().unwrap();
    let sample = |x: usize, y: usize| -> [u8; 3] {
        let base = y * READBACK_STRIDE as usize + x * 4;
        [mapped[base], mapped[base + 1], mapped[base + 2]]
    };
    assert_eq!(
        sample(2, 2),
        BLUE,
        "nearer ModPipelines quad in draw_pack_custom must beat pack terrain and pass through final"
    );
    assert_eq!(
        sample(2, 12),
        GREEN,
        "farther ModPipelines quad in draw_pack_custom must be occluded by pack terrain depth"
    );
    assert_eq!(
        sample(12, 12),
        BLUE,
        "farther ModPipelines quad in draw_pack_custom must appear where terrain is absent"
    );
    drop(mapped);
    read.unmap();
}

/// §3a design test: with a pack that draws terrain and a composite pass that inverts colour, a
/// mod GLSL pipeline quad submitted at `AfterOpaqueBlocks` draws once at `CustomGeometry`
/// (before the pack's composite chain) and is inverted by the composite pass.
#[test]
#[ignore = "daily"]
fn pack_composite_inverts_mod_glsl_quad_drawn_once_after_opaque_blocks() {
    use crate::gfx::mod_pipelines::{Batch as PipelineBatch, PipelineRun, PipelineVertex};
    use crate::mod_pipelines::Pipelines;
    use mod_api::{Identifier, PipelineBlend, RenderPipeline, WorldDepth};

    let _gpu = crate::gfx::test_gpu::lock();
    let files = vec![
        (
            "gbuffers_terrain.vsh".to_owned(),
            "#version 120\nvoid main(){gl_Position=vec4(gl_Vertex.xy,0.0,1.0);}".to_owned(),
        ),
        (
            "gbuffers_terrain.fsh".to_owned(),
            "#version 120\n/* DRAWBUFFERS:0 */\nvoid main(){gl_FragData[0]=vec4(0.0,1.0,0.0,1.0);}"
                .to_owned(),
        ),
        ("composite.vsh".to_owned(), VERTEX.to_owned()),
        (
            "composite.fsh".to_owned(),
            "#version 120\n/* DRAWBUFFERS:0 */\nuniform sampler2D colortex0;varying vec2 uv;\
             void main(){vec4 c=texture2D(colortex0,uv);gl_FragData[0]=vec4(1.0-c.rgb,1.0);}"
                .to_owned(),
        ),
        ("final.vsh".to_owned(), VERTEX.to_owned()),
        (
            "final.fsh".to_owned(),
            "#version 120\nuniform sampler2D colortex0;varying vec2 uv;\
             const vec4 colortex0ClearColor = vec4(0.0, 0.0, 0.0, 1.0);\
             void main(){gl_FragColor=texture2D(colortex0,uv);}"
                .to_owned(),
        ),
    ];
    let StageFixture {
        gpu,
        color,
        depth,
        mut post,
        terrain_buf,
        atlas_view,
        screen,
        read,
    } = stage_fixture(files);

    // Additive red (100, 0, 0) quad: drawn once over black gives (100, 0, 0), inverted by
    // composite to (155, 255, 255); drawn twice would give (200, 0, 0) -> (55, 255, 255).
    let id = Identifier::parse("test:after_opaque_mod").unwrap();
    let shader = Identifier::parse("test:core/after_opaque_mod").unwrap();
    let registered = [
        RenderPipeline::new(id, shader, PipelineBlend::Additive, WorldDepth::Tested)
            .vertex_shader(Identifier::parse("minecraft:core/position_tex_color").unwrap())
            .optional(),
    ];
    let fsh = "#version 330\nin vec4 vertexColor;\nout vec4 fragColor;\n\
               void main() { fragColor = vertexColor; }\n";
    let set = Pipelines::build(&registered, 1, |file| match file {
        "assets/test/shaders/core/after_opaque_mod.fsh" => Ok(Some(fsh.to_owned())),
        other => {
            let rest = other.strip_prefix("assets/").unwrap();
            let (ns, path) = rest.split_once('/').unwrap();
            Ok(sunrise_mod_pipeline::stubs::read(&format!("{ns}:{path}")).map(str::to_owned))
        }
    })
    .unwrap();
    let mut primitives = WorldPrimitives::new(&gpu.device, (T::Rgba8Unorm, false));
    primitives.pipelines.build(&gpu.device, &set, None).unwrap();

    // Additive red (100, 0, 0) quad over `gpu.clear`'s (16, 16, 16) background: drawn once
    // before composite gives (116, 16, 16), inverted to (139, 239, 239). Drawn twice would give
    // (216, 16, 16) -> (39, 239, 239); drawn after composite would not be inverted.
    let quad_color = rgba([100, 0, 0]);
    let pipe_verts: Vec<PipelineVertex> = [[0.0f32, -1.0], [0.0, 1.0], [0.5, 1.0], [0.5, -1.0]]
        .into_iter()
        .map(|[x, y]| PipelineVertex {
            pos: [x, y, 0.75],
            uv: [0.5, 0.5],
            color: quad_color,
        })
        .collect();
    let runs = [PipelineRun {
        slot: 0,
        stage: world_primitives::STAGE_AFTER_OPAQUE_BLOCKS as u32,
        quads: 1,
        modifier_stack: 0,
    }];
    let batch = Batch {
        pipelines: PipelineBatch {
            set: Some(&set),
            vertices: &pipe_verts,
            runs: &runs,
            time: 0.0,
            ..Default::default()
        },
        ..Default::default()
    };
    assert!(primitives.prepare(
        &gpu.device,
        &mut |buffer, bytes| gpu.queue.write_buffer(buffer, 0, bytes),
        &batch,
        Mat4::IDENTITY,
        SIZE,
        &atlas_view,
        None,
    ));
    primitives.prepare_pack_pass(&gpu.device, post.formats[0], post.rgb & 1 != 0, false);

    let hud = HudBatch::scratch(SIZE);
    let frame = fixture_frame(&hud);
    let mut encoder = gpu.device.create_command_encoder(&Default::default());
    let mut draw_count = 0usize;
    post.run(
        &gpu.device,
        &gpu.queue,
        &mut encoder,
        &color,
        &depth,
        &screen,
        &frame,
        &Default::default(),
        SIZE,
        SIZE,
        &[],
        &mut |rp, list, _, _| {
            if list == TerrainList::Solid {
                rp.set_vertex_buffer(0, terrain_buf.slice(..));
                rp.draw(0..6, 0..1);
            }
        },
        WorldBoundaries {
            custom_geometry: true,
            ..Default::default()
        },
        |enc, boundary| {
            if let WorldBoundary::CustomGeometry { color, depth } = boundary {
                draw_count += 1;
                return primitives.draw_pack_stages(
                    enc,
                    color,
                    depth,
                    0,
                    world_primitives::STAGE_CUSTOM_GEOMETRY,
                );
            }
            false
        },
        &std::sync::atomic::AtomicBool::new(false),
        super::super::in_flight::poll_live,
    )
    .unwrap();
    assert_eq!(draw_count, 1, "mod quad must draw once before composite");

    encoder.copy_texture_to_buffer(
        screen.texture().as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &read,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(READBACK_STRIDE),
                rows_per_image: None,
            },
        },
        screen.texture().size(),
    );
    gpu.queue.submit([encoder.finish()]);
    let slice = read.slice(..);
    let (tx, rx) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |r| {
        let _ = tx.send(r);
    });
    let _ = gpu.device.poll(wgpu::PollType::wait_indefinitely());
    rx.recv().unwrap().unwrap();
    let mapped = slice.get_mapped_range().unwrap();
    let sample = |x: usize, y: usize| -> [u8; 3] {
        let base = y * READBACK_STRIDE as usize + x * 4;
        [mapped[base], mapped[base + 1], mapped[base + 2]]
    };
    assert_eq!(
        sample(2, 8),
        [255, 0, 255],
        "pack terrain green inverted by composite to magenta"
    );
    assert_eq!(
        sample(14, 8),
        [239, 239, 239],
        "untouched background (16, 16, 16) inverted by composite to (239, 239, 239)"
    );
    assert_eq!(
        sample(10, 8),
        [139, 239, 239],
        "AfterOpaqueBlocks mod quad must draw once before composite over (16,16,16) and invert to (139,239,239)"
    );
    drop(mapped);
    read.unmap();
}

/// Minimal `gbuffers_terrain` + pass-through `final` pack so `colortex0` + `depthtex0` run in
/// GL-rows, standard-Z `[0..1]` and `final` copies `colortex0` directly back to screen rows.
fn passthrough_pack_files() -> Vec<(String, String)> {
    vec![
        (
            "gbuffers_terrain.vsh".to_owned(),
            "#version 120\nvoid main(){gl_Position=vec4(gl_Vertex.xy,0.0,1.0);}".to_owned(),
        ),
        (
            "gbuffers_terrain.fsh".to_owned(),
            "#version 120\n/* DRAWBUFFERS:0 */\nvoid main(){gl_FragData[0]=vec4(0.0,0.0,0.0,1.0);}"
                .to_owned(),
        ),
        ("final.vsh".to_owned(), VERTEX.to_owned()),
        (
            "final.fsh".to_owned(),
            "#version 120\nuniform sampler2D colortex0;varying vec2 uv;\
             const vec4 colortex0ClearColor = vec4(0.0, 0.0, 0.0, 1.0);\
             void main(){gl_FragColor=texture2D(colortex0,uv);}"
                .to_owned(),
        ),
    ]
}

fn read_rgba(gpu: &Gpu, view: &wgpu::TextureView, read: &wgpu::Buffer) -> Vec<[u8; 4]> {
    let mut encoder = gpu.device.create_command_encoder(&Default::default());
    encoder.copy_texture_to_buffer(
        view.texture().as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: read,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(READBACK_STRIDE),
                rows_per_image: None,
            },
        },
        view.texture().size(),
    );
    gpu.queue.submit([encoder.finish()]);
    let slice = read.slice(..);
    let (tx, rx) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |r| {
        let _ = tx.send(r);
    });
    let _ = gpu.device.poll(wgpu::PollType::wait_indefinitely());
    rx.recv().unwrap().unwrap();
    let mapped = slice.get_mapped_range().unwrap();
    let mut out = Vec::with_capacity((SIZE[0] * SIZE[1]) as usize);
    for y in 0..SIZE[1] as usize {
        for x in 0..SIZE[0] as usize {
            let base = y * READBACK_STRIDE as usize + x * 4;
            out.push([
                mapped[base],
                mapped[base + 1],
                mapped[base + 2],
                mapped[base + 3],
            ]);
        }
    }
    drop(mapped);
    read.unmap();
    out
}

fn build_test_pipelines(
    registered: &[mod_api::RenderPipeline],
    shaders: &[(&str, &str)],
) -> crate::mod_pipelines::Pipelines {
    crate::mod_pipelines::Pipelines::build(registered, 1, |file| {
        if let Some((_, src)) = shaders.iter().find(|(path, _)| *path == file) {
            return Ok(Some((*src).to_owned()));
        }
        let rest = file.strip_prefix("assets/").unwrap();
        let (ns, path) = rest.split_once('/').unwrap();
        Ok(sunrise_mod_pipeline::stubs::read(&format!("{ns}:{path}")).map(str::to_owned))
    })
    .unwrap()
}

/// Draw the same mod pipelines and optional follow-up entity draws on both the plain reversed-Z,
/// screen-rows route and the `PackPost` standard-Z, GL-rows route, returning both full `SIZE`
/// RGBA readbacks for exact byte-for-byte comparison.
fn render_plain_and_pack(
    gpu: Gpu,
    set: &crate::mod_pipelines::Pipelines,
    batch: &Batch<'_>,
    after_opaque: &[DrawRanges],
    after_features: &[DrawRanges],
) -> (Vec<[u8; 4]>, Vec<[u8; 4]>) {
    let StageFixture {
        gpu,
        color,
        depth,
        mut post,
        atlas_view,
        screen,
        read,
        ..
    } = stage_fixture_with_gpu(gpu, passthrough_pack_files());
    let caps = crate::gfx::mod_render_targets::probe_render_capabilities(&gpu.adapter, &gpu.device);
    let mut primitives = WorldPrimitives::new(&gpu.device, (T::Rgba8Unorm, false));
    primitives.pipelines.set_capabilities(&caps);
    primitives.pipelines.build(&gpu.device, set, None).unwrap();
    assert!(primitives.prepare(
        &gpu.device,
        &mut |buffer, bytes| gpu.queue.write_buffer(buffer, 0, bytes),
        batch,
        Mat4::IDENTITY,
        SIZE,
        &atlas_view,
        None,
    ));

    // 1. Plain route: screen rows, reversed-Z (far = 0.0, background = [16, 16, 16, 255]).
    let plain_color = gpu.texture(T::Rgba8Unorm);
    let plain_depth = gpu.texture(T::Depth32Float);
    gpu.clear(&plain_color, &plain_depth, 0.0);
    {
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Plain route convention compare"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &plain_color,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &plain_depth,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                ..Default::default()
            });
            assert!(primitives.draw_prepared(&mut pass, None, after_opaque));
            if !after_features.is_empty() {
                assert!(primitives.draw_prepared(&mut pass, None, after_features));
            }
        }
        gpu.queue.submit([encoder.finish()]);
    }
    let plain_pixels = read_rgba(&gpu, &plain_color, &read);

    // 2. Pack route: GL rows, standard-Z (`colortex0` + `depthtex0` -> pass-through `final`).
    primitives.prepare_pack_pass(&gpu.device, post.formats[0], post.rgb & 1 != 0, false);
    primitives.set_pack_target_size(&gpu.queue, SIZE);
    let hud = HudBatch::scratch(SIZE);
    let frame = fixture_frame(&hud);
    let mut encoder = gpu.device.create_command_encoder(&Default::default());
    let mut opaque_called = false;
    let mut features_called = false;
    post.run(
        &gpu.device,
        &gpu.queue,
        &mut encoder,
        &color,
        &depth,
        &screen,
        &frame,
        &Default::default(),
        SIZE,
        SIZE,
        &[],
        &mut |_, _, _, _| {},
        WorldBoundaries {
            opaque_blocks: true,
            features: !after_features.is_empty(),
            ..Default::default()
        },
        |enc, boundary| match boundary {
            WorldBoundary::AfterOpaqueBlocks { color, depth } => {
                opaque_called = true;
                primitives.draw_pack_prepared(enc, color, depth, after_opaque)
            }
            WorldBoundary::Features {
                translucent: false,
                color,
                depth,
            } if !after_features.is_empty() => {
                features_called = true;
                primitives.draw_pack_prepared(enc, color, depth, after_features)
            }
            _ => false,
        },
        &std::sync::atomic::AtomicBool::new(false),
        super::super::in_flight::poll_live,
    )
    .unwrap();
    assert!(opaque_called);
    assert_eq!(features_called, !after_features.is_empty());
    gpu.queue.submit([encoder.finish()]);
    let pack_pixels = read_rgba(&gpu, &screen, &read);
    (plain_pixels, pack_pixels)
}

#[test]
#[ignore = "daily"]
fn pack_target_fullscreen_composite_frag_depth_occludes_entities_equally() {
    use crate::gfx::mod_pipelines::{
        Batch as PipelineBatch, PipelineDraw, PipelineDrawSource, PipelineVertex,
    };
    use mod_api::{
        DepthState, Identifier, MeshDraw, PipelineBlend, PrimitiveTopology, RenderPipeline,
        WorldDepth,
    };

    let _gpu = crate::gfx::test_gpu::lock();
    let vsh = "\
#version 330
in vec3 Position;
in vec2 UV0;
in vec4 Color;
void main() {
    vec2 uv = vec2(float((gl_VertexIndex << 1) & 2), float(gl_VertexIndex & 2));
    gl_Position = vec4(uv * 2.0 - 1.0, 0.0, 1.0);
}
";
    let fsh = "\
#version 330
out vec4 fragColor;
void main() {
    if (gl_FragCoord.y < 8.0) {
        fragColor = vec4(1.0, 0.0, 0.0, 1.0);
        gl_FragDepth = 0.75;
        return;
    }
    fragColor = vec4(0.0, 0.0, 1.0, 1.0);
    gl_FragDepth = 0.25;
}
";
    let registered = [RenderPipeline::new(
        Identifier::parse("test:pack_conv_composite").unwrap(),
        Identifier::parse("test:core/pack_conv_composite").unwrap(),
        PipelineBlend::Opaque,
        WorldDepth::Tested,
    )
    .depth_stencil(Some(DepthState::DEFAULT))
    .topology(PrimitiveTopology::Triangles)
    .optional()];
    let set = build_test_pipelines(
        &registered,
        &[
            ("assets/test/shaders/core/pack_conv_composite.vsh", vsh),
            ("assets/test/shaders/core/pack_conv_composite.fsh", fsh),
        ],
    );
    let dummy_verts = [PipelineVertex {
        pos: [0.0; 3],
        uv: [0.0; 2],
        color: 0,
    }; 3];
    let vbytes: &[u8] = bytemuck::cast_slice(&dummy_verts);
    let draws = [PipelineDraw {
        slot: 0,
        stage: world_primitives::STAGE_AFTER_OPAQUE_BLOCKS as u32,
        modifier_stack: 0,
        draw: MeshDraw::vertices(0..3),
        source: PipelineDrawSource::Transient {
            vertex_ranges: vec![(0, 0..vbytes.len() as u64)],
            index_range: None,
        },
        bindings: None,
    }];
    let entity_quad = ndc_quad([-1.0, -1.0, 1.0, 1.0], 0.5, GREEN);
    let batch = Batch {
        quads: &entity_quad,
        pipelines: PipelineBatch {
            set: Some(&set),
            draws: &draws,
            transient_vertices: vbytes,
            ..Default::default()
        },
        ..Default::default()
    };
    let after_opaque = [DrawRanges {
        pipeline_draws: 0..1,
        ..Default::default()
    }];
    let after_features = [DrawRanges {
        primitives: [0..1, 0..0, 0..0, 0..0],
        ..Default::default()
    }];
    let gpu = Gpu::new().expect("graphics adapter required");
    let (plain, pack) = render_plain_and_pack(gpu, &set, &batch, &after_opaque, &after_features);
    assert_eq!(
        pack, plain,
        "fullscreen composite writing gl_FragDepth at AfterOpaqueBlocks must match plain route bit-for-bit"
    );
    for y in 0..SIZE[1] as usize {
        let expected = if y < 8 {
            [255, 0, 0, 255]
        } else {
            [0, 255, 0, 255]
        };
        for x in 0..SIZE[0] as usize {
            assert_eq!(
                plain[y * SIZE[0] as usize + x],
                expected,
                "pixel ({x}, {y}) depth occlusion mismatch"
            );
        }
    }
}

#[test]
#[ignore = "daily"]
fn pack_target_culled_mesh_keeps_same_visible_faces_and_front_facing() {
    use crate::gfx::mod_pipelines::{
        Batch as PipelineBatch, PipelineDraw, PipelineDrawSource, PipelineVertex,
    };
    use mod_api::{
        Identifier, MeshDraw, PipelineBlend, PrimitiveTopology, RenderPipeline, WorldDepth,
    };

    let _gpu = crate::gfx::test_gpu::lock();
    let vsh = "\
#version 330
#include <minecraft:dynamictransforms.glsl>
#include <minecraft:projection.glsl>
in vec3 Position;
in vec2 UV0;
in vec4 Color;
out vec4 vertexColor;
void main() {
    gl_Position = ProjMat * ModelViewMat * vec4(Position, 1.0);
    vertexColor = Color;
}
";
    let fsh_culled = "\
#version 330
in vec4 vertexColor;
out vec4 fragColor;
void main() {
    fragColor = vertexColor;
}
";
    let fsh_front = "\
#version 330
in vec4 vertexColor;
out vec4 fragColor;
void main() {
    fragColor = gl_FrontFacing ? vec4(0.0, 1.0, 0.0, 1.0) : vec4(1.0, 1.0, 0.0, 1.0);
}
";
    let registered = [
        RenderPipeline::new(
            Identifier::parse("test:pack_conv_culled").unwrap(),
            Identifier::parse("test:core/pack_conv_culled").unwrap(),
            PipelineBlend::Opaque,
            WorldDepth::Tested,
        )
        .topology(PrimitiveTopology::Triangles)
        .cull(true)
        .optional(),
        RenderPipeline::new(
            Identifier::parse("test:pack_conv_front").unwrap(),
            Identifier::parse("test:core/pack_conv_front").unwrap(),
            PipelineBlend::Opaque,
            WorldDepth::Tested,
        )
        .vertex_shader(Identifier::parse("test:core/pack_conv_culled").unwrap())
        .topology(PrimitiveTopology::Triangles)
        .cull(false)
        .optional(),
    ];
    let set = build_test_pipelines(
        &registered,
        &[
            ("assets/test/shaders/core/pack_conv_culled.vsh", vsh),
            ("assets/test/shaders/core/pack_conv_culled.fsh", fsh_culled),
            ("assets/test/shaders/core/pack_conv_front.fsh", fsh_front),
        ],
    );
    let tri_vert = |[x, y]: [f32; 2], rgb: [u8; 3]| PipelineVertex {
        pos: [x, y, 0.5],
        uv: [0.0, 0.0],
        color: rgba(rgb),
    };
    // Top-left: CCW in Y-up NDC (front-facing, kept); top-right: CW (back-facing, culled).
    // Bottom-left: CCW (gl_FrontFacing == true -> green); bottom-right: CW (false -> yellow).
    let verts = [
        tri_vert([-0.9, 0.1], RED),
        tri_vert([-0.1, 0.1], RED),
        tri_vert([-0.5, 0.9], RED),
        tri_vert([0.1, 0.1], BLUE),
        tri_vert([0.5, 0.9], BLUE),
        tri_vert([0.9, 0.1], BLUE),
        tri_vert([-0.9, -0.9], RED),
        tri_vert([-0.1, -0.9], RED),
        tri_vert([-0.5, -0.1], RED),
        tri_vert([0.1, -0.9], BLUE),
        tri_vert([0.5, -0.1], BLUE),
        tri_vert([0.9, -0.9], BLUE),
    ];
    let vbytes: &[u8] = bytemuck::cast_slice(&verts);
    let half = (6 * std::mem::size_of::<PipelineVertex>()) as u64;
    let draws = [
        PipelineDraw {
            slot: 0,
            stage: world_primitives::STAGE_AFTER_OPAQUE_BLOCKS as u32,
            modifier_stack: 0,
            draw: MeshDraw::vertices(0..6),
            source: PipelineDrawSource::Transient {
                vertex_ranges: vec![(0, 0..half)],
                index_range: None,
            },
            bindings: None,
        },
        PipelineDraw {
            slot: 1,
            stage: world_primitives::STAGE_AFTER_OPAQUE_BLOCKS as u32,
            modifier_stack: 0,
            draw: MeshDraw::vertices(0..6),
            source: PipelineDrawSource::Transient {
                vertex_ranges: vec![(0, half..2 * half)],
                index_range: None,
            },
            bindings: None,
        },
    ];
    let batch = Batch {
        pipelines: PipelineBatch {
            set: Some(&set),
            draws: &draws,
            transient_vertices: vbytes,
            ..Default::default()
        },
        ..Default::default()
    };
    let after_opaque = [DrawRanges {
        pipeline_draws: 0..2,
        ..Default::default()
    }];
    let gpu = Gpu::new().expect("graphics adapter required");
    let (plain, pack) = render_plain_and_pack(gpu, &set, &batch, &after_opaque, &[]);
    assert_eq!(
        pack, plain,
        "culled faces and gl_FrontFacing must match plain route bit-for-bit on pack targets"
    );
    let at = |x: usize, y: usize| plain[y * SIZE[0] as usize + x];
    assert_eq!(
        at(4, 4),
        [255, 0, 0, 255],
        "front-facing CCW triangle stays visible"
    );
    assert_eq!(
        at(12, 4),
        [16, 16, 16, 255],
        "back-facing CW triangle is culled"
    );
    assert_eq!(
        at(4, 12),
        [0, 255, 0, 255],
        "gl_FrontFacing is true for CCW triangle"
    );
    assert_eq!(
        at(12, 12),
        [255, 255, 0, 255],
        "gl_FrontFacing is false for CW triangle"
    );
}

#[test]
#[ignore = "daily"]
fn pack_target_frag_coord_and_derivative_normal_match_plain_route() {
    use crate::gfx::mod_pipelines::{
        Batch as PipelineBatch, PipelineDraw, PipelineDrawSource, PipelineVertex,
    };
    use mod_api::{
        Identifier, MeshDraw, PipelineBlend, PrimitiveTopology, RenderPipeline, WorldDepth,
    };

    let _gpu = crate::gfx::test_gpu::lock();
    let vsh = "\
#version 330
in vec3 Position;
in vec2 UV0;
in vec4 Color;
out vec3 surfacePos;
void main() {
    surfacePos = vec3(Position.xy, 0.75);
    gl_Position = vec4(Position.xy, 0.75, 1.0);
}
";
    let fsh_body = "\
in vec3 surfacePos;
out vec4 fragColor;
void main() {
    vec3 n = normalize(cross(dFdx(surfacePos), dFdy(surfacePos)));
    vec3 nf = normalize(cross(dFdxFine(surfacePos), dFdyFine(surfacePos)));
    vec3 nc = normalize(cross(dFdxCoarse(surfacePos), dFdyCoarse(surfacePos)));
    float bits = (gl_FragCoord.z > 0.6 ? 128.0 : 0.0)
        + (n.z < -0.5 ? 64.0 : 0.0)
        + (nf.z < -0.5 ? 32.0 : 0.0)
        + (nc.z < -0.5 ? 16.0 : 0.0);
    fragColor = vec4(
        floor(gl_FragCoord.x) / 255.0,
        floor(gl_FragCoord.y) / 255.0,
        bits / 255.0,
        1.0
    );
}
";
    let fsh_no_globals = format!("#version 330\n{fsh_body}");
    let fsh_with_globals = format!("#version 330\n#include <minecraft:globals.glsl>\n{fsh_body}");
    let registered = [
        RenderPipeline::new(
            Identifier::parse("test:pack_conv_coord_no_globals").unwrap(),
            Identifier::parse("test:core/pack_conv_coord_no_globals").unwrap(),
            PipelineBlend::Opaque,
            WorldDepth::Tested,
        )
        .topology(PrimitiveTopology::Quads)
        .optional(),
        RenderPipeline::new(
            Identifier::parse("test:pack_conv_coord_globals").unwrap(),
            Identifier::parse("test:core/pack_conv_coord_globals").unwrap(),
            PipelineBlend::Opaque,
            WorldDepth::Tested,
        )
        .vertex_shader(Identifier::parse("test:core/pack_conv_coord_no_globals").unwrap())
        .topology(PrimitiveTopology::Quads)
        .optional(),
    ];
    let set = build_test_pipelines(
        &registered,
        &[
            (
                "assets/test/shaders/core/pack_conv_coord_no_globals.vsh",
                vsh,
            ),
            (
                "assets/test/shaders/core/pack_conv_coord_no_globals.fsh",
                &fsh_no_globals,
            ),
            (
                "assets/test/shaders/core/pack_conv_coord_globals.fsh",
                &fsh_with_globals,
            ),
        ],
    );
    let quad = |[x0, x1]: [f32; 2]| {
        [[x0, -1.0], [x1, -1.0], [x1, 1.0], [x0, 1.0]].map(|[x, y]| PipelineVertex {
            pos: [x, y, 0.75],
            uv: [0.0, 0.0],
            color: 0xFFFF_FFFF,
        })
    };
    let verts: Vec<PipelineVertex> = [quad([-1.0, 0.0]), quad([0.0, 1.0])].concat();
    let vbytes: &[u8] = bytemuck::cast_slice(&verts);
    let qbytes = (4 * std::mem::size_of::<PipelineVertex>()) as u64;
    let draws = [
        PipelineDraw {
            slot: 0,
            stage: world_primitives::STAGE_AFTER_OPAQUE_BLOCKS as u32,
            modifier_stack: 0,
            draw: MeshDraw::vertices(0..4),
            source: PipelineDrawSource::Transient {
                vertex_ranges: vec![(0, 0..qbytes)],
                index_range: None,
            },
            bindings: None,
        },
        PipelineDraw {
            slot: 1,
            stage: world_primitives::STAGE_AFTER_OPAQUE_BLOCKS as u32,
            modifier_stack: 0,
            draw: MeshDraw::vertices(0..4),
            source: PipelineDrawSource::Transient {
                vertex_ranges: vec![(0, qbytes..2 * qbytes)],
                index_range: None,
            },
            bindings: None,
        },
    ];
    let batch = Batch {
        pipelines: PipelineBatch {
            set: Some(&set),
            draws: &draws,
            transient_vertices: vbytes,
            ..Default::default()
        },
        ..Default::default()
    };
    let after_opaque = [DrawRanges {
        pipeline_draws: 0..2,
        ..Default::default()
    }];
    let gpu = Gpu::new().expect("graphics adapter required");
    let (plain, pack) = render_plain_and_pack(gpu, &set, &batch, &after_opaque, &[]);
    assert_eq!(
        pack, plain,
        "gl_FragCoord.xyz and dFdy/dFdyFine/dFdyCoarse normals must match plain route bit-for-bit"
    );
    for y in 0..SIZE[1] as usize {
        for x in 0..SIZE[0] as usize {
            assert_eq!(
                plain[y * SIZE[0] as usize + x],
                [x as u8, y as u8, 240, 255],
                "pixel ({x}, {y}) FragCoord/derivative encoding mismatch"
            );
        }
    }
}

#[test]
#[ignore = "daily"]
fn pack_target_shader_without_frag_depth_skips_depth_write_finish() {
    use crate::gfx::mod_pipelines::{
        Batch as PipelineBatch, PipelineDraw, PipelineDrawSource, PipelineVertex,
        ResolvedDrawBindings, ResolvedSet1Entry, SamplerLayout,
    };
    use mod_api::{
        BindEntry, BindGroupLayout, BindingKind, DepthState, Identifier, MeshDraw, PipelineBlend,
        PrimitiveTopology, RenderPipeline, StorageBufferDescriptor, WorldDepth,
    };

    let _gpu = crate::gfx::test_gpu::lock();
    let vsh = "\
#version 330
in vec3 Position;
in vec2 UV0;
in vec4 Color;
void main() {
    gl_Position = vec4(Position, 1.0);
}
";
    let fsh_no_depth = "\
#version 330
out vec4 fragColor;
void main() {
    fragColor = gl_FragCoord.y < 8.0
        ? vec4(1.0, 0.0, 0.0, 1.0)
        : vec4(0.0, 0.0, 1.0, 1.0);
}
";
    // 1. Verify the standard-Z / GL-rows translation rewrites gl_FragCoord without adding a
    // gl_FragDepth write (which would turn off hardware early-Z).
    let stub = |id: &str| Ok(sunrise_mod_pipeline::stubs::read(id).map(str::to_owned));
    let built = sunrise_mod_pipeline::translate(
        &sunrise_mod_pipeline::ProgramSources {
            vertex_file: "assets/test/shaders/core/no_depth.vsh",
            vertex_dir: "test:shaders/core/",
            vertex: vsh,
            fragment_file: "assets/test/shaders/core/no_depth.fsh",
            fragment_dir: "test:shaders/core/",
            fragment: fsh_no_depth,
        },
        &sunrise_mod_pipeline::Options::default(),
        &stub,
    )
    .unwrap();
    let (_vs, fs) = sunrise_mod_pipeline::translate_standard_z_gl_rows(
        &built.vertex_expanded,
        &built.fragment_expanded,
        &built.translated.fragment,
        &sunrise_mod_pipeline::Options::default(),
    )
    .unwrap();
    let fs = fs.expect("gl_FragCoord shader needs adapted fragment module");
    let conv = sunrise_glsl::fragment_conventions(&fs);
    assert!(
        !conv.writes_frag_depth,
        "non-depth-writing shader must not receive a gl_FragDepth write finish"
    );
    assert!(conv.reads_frag_coord);

    // 2. Verify hardware interpolated vertex depth matches on plain and pack routes.
    let registered = [RenderPipeline::new(
        Identifier::parse("test:pack_conv_no_depth").unwrap(),
        Identifier::parse("test:core/pack_conv_no_depth").unwrap(),
        PipelineBlend::Opaque,
        WorldDepth::Tested,
    )
    .depth_stencil(Some(DepthState::DEFAULT))
    .topology(PrimitiveTopology::Quads)
    .optional()];
    let set = build_test_pipelines(
        &registered,
        &[
            ("assets/test/shaders/core/pack_conv_no_depth.vsh", vsh),
            (
                "assets/test/shaders/core/pack_conv_no_depth.fsh",
                fsh_no_depth,
            ),
        ],
    );
    let quad = |[y0, y1]: [f32; 2], z: f32| {
        [[-1.0, y0], [1.0, y0], [1.0, y1], [-1.0, y1]].map(|[x, y]| PipelineVertex {
            pos: [x, y, z],
            uv: [0.0, 0.0],
            color: 0xFFFF_FFFF,
        })
    };
    let verts: Vec<PipelineVertex> = [quad([0.0, 1.0], 0.75), quad([-1.0, 0.0], 0.25)].concat();
    let vbytes: &[u8] = bytemuck::cast_slice(&verts);
    let draws = [PipelineDraw {
        slot: 0,
        stage: world_primitives::STAGE_AFTER_OPAQUE_BLOCKS as u32,
        modifier_stack: 0,
        draw: MeshDraw::vertices(0..8),
        source: PipelineDrawSource::Transient {
            vertex_ranges: vec![(0, 0..vbytes.len() as u64)],
            index_range: None,
        },
        bindings: None,
    }];
    let entity_quad = ndc_quad([-1.0, -1.0, 1.0, 1.0], 0.5, GREEN);
    let batch = Batch {
        quads: &entity_quad,
        pipelines: PipelineBatch {
            set: Some(&set),
            draws: &draws,
            transient_vertices: vbytes,
            ..Default::default()
        },
        ..Default::default()
    };
    let after_opaque = [DrawRanges {
        pipeline_draws: 0..1,
        ..Default::default()
    }];
    let after_features = [DrawRanges {
        primitives: [0..1, 0..0, 0..0, 0..0],
        ..Default::default()
    }];
    let gpu = Gpu::new().expect("graphics adapter required");
    let (plain, pack) = render_plain_and_pack(gpu, &set, &batch, &after_opaque, &after_features);
    assert_eq!(pack, plain);
    for y in 0..SIZE[1] as usize {
        let expected = if y < 8 {
            [255, 0, 0, 255]
        } else {
            [0, 255, 0, 255]
        };
        for x in 0..SIZE[0] as usize {
            assert_eq!(plain[y * SIZE[0] as usize + x], expected);
        }
    }

    // 3. On devices supporting SHADER_EARLY_DEPTH_TEST, prove a layout(early_fragment_tests) in
    // + gl_FragCoord shader running on the pack route still skips occluded storage writes.
    let gpu = Gpu::with_features(wgpu::Features::SHADER_EARLY_DEPTH_TEST)
        .expect("graphics adapter required");
    let caps = crate::gfx::mod_render_targets::probe_render_capabilities(&gpu.adapter, &gpu.device);
    if !caps.shader_early_depth_test || !caps.fragment_writable_storage {
        return;
    }
    let StageFixture {
        gpu,
        color,
        depth,
        mut post,
        atlas_view,
        screen,
        ..
    } = stage_fixture_with_gpu(gpu, passthrough_pack_files());
    let early_fsh = "\
#version 330
layout(early_fragment_tests) in;
layout(std430) buffer Vis { uint vis[]; };
out vec4 fragColor;
void main() {
    if (gl_FragCoord.y < 16.0) {
        vis[0] = 1u;
    }
    fragColor = vec4(0.0, 0.0, 0.0, 1.0);
}
";
    let early_layout = BindGroupLayout::builder()
        .entry(BindEntry::new(
            "Vis",
            BindingKind::StorageBuffer {
                read_only: false,
                min_size: 4,
            },
        ))
        .build()
        .unwrap();
    let early_registered = [
        RenderPipeline::new(
            Identifier::parse("test:pack_conv_blocker").unwrap(),
            Identifier::parse("test:core/pack_conv_no_depth").unwrap(),
            PipelineBlend::Opaque,
            WorldDepth::Tested,
        )
        .depth_stencil(Some(DepthState::DEFAULT))
        .topology(PrimitiveTopology::Quads)
        .optional(),
        RenderPipeline::new(
            Identifier::parse("test:pack_conv_early").unwrap(),
            Identifier::parse("test:core/pack_conv_early").unwrap(),
            PipelineBlend::Opaque,
            WorldDepth::Tested,
        )
        .vertex_shader(Identifier::parse("test:core/pack_conv_no_depth").unwrap())
        .topology(PrimitiveTopology::Quads)
        .bind_group_layout(early_layout)
        .optional(),
    ];
    let early_set = build_test_pipelines(
        &early_registered,
        &[
            ("assets/test/shaders/core/pack_conv_no_depth.vsh", vsh),
            (
                "assets/test/shaders/core/pack_conv_no_depth.fsh",
                fsh_no_depth,
            ),
            ("assets/test/shaders/core/pack_conv_early.fsh", early_fsh),
        ],
    );
    let mut primitives = WorldPrimitives::new(&gpu.device, (T::Rgba8Unorm, false));
    primitives.pipelines.set_capabilities(&caps);
    primitives
        .pipelines
        .build(&gpu.device, &early_set, None)
        .unwrap();

    let mut storage = crate::gfx::mod_storage::ModStorageBuffers::default();
    let buffer = storage
        .create(
            &gpu.device,
            None,
            &StorageBufferDescriptor::new(512).copy_src(),
        )
        .unwrap();
    storage.write_range(None, buffer, 0, &[0u8; 4]).unwrap();
    storage.write_range(None, buffer, 256, &[0u8; 4]).unwrap();
    storage.flush(&gpu.device, &gpu.queue);
    let limits = gpu.device.limits();
    let resolve_slice = |offset: u64, len: u64| {
        storage
            .resolve(&limits, None, buffer.slice(offset, len), 4)
            .unwrap()
    };

    let half_quad = |[x0, x1]: [f32; 2], z: f32| {
        [[x0, -1.0], [x1, -1.0], [x1, 1.0], [x0, 1.0]].map(|[x, y]| PipelineVertex {
            pos: [x, y, z],
            uv: [0.0, 0.0],
            color: 0xFFFF_FFFF,
        })
    };
    let early_verts: Vec<PipelineVertex> = [
        half_quad([-1.0, 0.0], 0.8),
        half_quad([-1.0, 0.0], 0.2),
        half_quad([0.0, 1.0], 0.2),
    ]
    .concat();
    let early_vbytes: &[u8] = bytemuck::cast_slice(&early_verts);
    let qstride = (4 * std::mem::size_of::<PipelineVertex>()) as u64;
    let make_draw = |slot: u32, qi: u64, off: Option<u64>| PipelineDraw {
        slot,
        stage: world_primitives::STAGE_AFTER_OPAQUE_BLOCKS as u32,
        modifier_stack: 0,
        draw: MeshDraw::vertices(0..4),
        source: PipelineDrawSource::Transient {
            vertex_ranges: vec![(0, qi * qstride..(qi + 1) * qstride)],
            index_range: None,
        },
        bindings: off.map(|offset| {
            let r = resolve_slice(offset, 16);
            ResolvedDrawBindings {
                sampler_layout: SamplerLayout::EMPTY,
                entries: vec![ResolvedSet1Entry::StorageBuffer {
                    id: r.id,
                    buffer: r.buffer,
                    offset: r.offset,
                    size: r.length,
                }],
                push_constants: None,
                reads_world_depth: false,
            }
        }),
    };
    let early_draws = [
        make_draw(0, 0, None),
        make_draw(1, 1, Some(0)),
        make_draw(1, 2, Some(256)),
    ];
    let early_batch = Batch {
        pipelines: PipelineBatch {
            set: Some(&early_set),
            draws: &early_draws,
            transient_vertices: early_vbytes,
            ..Default::default()
        },
        ..Default::default()
    };
    assert!(primitives.prepare(
        &gpu.device,
        &mut |buf, bytes| gpu.queue.write_buffer(buf, 0, bytes),
        &early_batch,
        Mat4::IDENTITY,
        SIZE,
        &atlas_view,
        None,
    ));
    primitives.prepare_pack_pass(&gpu.device, post.formats[0], post.rgb & 1 != 0, false);
    primitives.set_pack_target_size(&gpu.queue, SIZE);

    let early_ranges = [DrawRanges {
        pipeline_draws: 0..3,
        ..Default::default()
    }];
    let hud = HudBatch::scratch(SIZE);
    let frame = fixture_frame(&hud);
    let validation = gpu.device.push_error_scope(wgpu::ErrorFilter::Validation);
    let mut encoder = gpu.device.create_command_encoder(&Default::default());
    post.run(
        &gpu.device,
        &gpu.queue,
        &mut encoder,
        &color,
        &depth,
        &screen,
        &frame,
        &Default::default(),
        SIZE,
        SIZE,
        &[],
        &mut |_, _, _, _| {},
        WorldBoundaries {
            opaque_blocks: true,
            ..Default::default()
        },
        |enc, boundary| {
            if let WorldBoundary::AfterOpaqueBlocks { color, depth } = boundary {
                return primitives.draw_pack_prepared(enc, color, depth, &early_ranges);
            }
            false
        },
        &std::sync::atomic::AtomicBool::new(false),
        super::super::in_flight::poll_live,
    )
    .unwrap();
    let resolved_all = resolve_slice(0, 512);
    let ssbo_read = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("Pack early-Z readback"),
        size: 512,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    encoder.copy_buffer_to_buffer(&resolved_all.buffer, 0, &ssbo_read, 0, 512);
    gpu.queue.submit([encoder.finish()]);
    assert!(
        pollster::block_on(validation.pop()).is_none(),
        "early_fragment_tests on pack target must pass wgpu validation"
    );
    let slice = ssbo_read.slice(..);
    let (tx, rx) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |r| {
        let _ = tx.send(r);
    });
    let _ = gpu.device.poll(wgpu::PollType::wait_indefinitely());
    rx.recv().unwrap().unwrap();
    let mapped = slice.get_mapped_range().unwrap();
    let occluded = u32::from_ne_bytes(mapped[0..4].try_into().unwrap());
    let visible = u32::from_ne_bytes(mapped[256..260].try_into().unwrap());
    drop(mapped);
    ssbo_read.unmap();
    assert_eq!(
        occluded, 0,
        "occluded early_fragment_tests draw on pack target must skip SSBO write (no forced gl_FragDepth)"
    );
    assert_eq!(
        visible, 1,
        "visible early_fragment_tests draw on pack target must execute SSBO write"
    );
}
