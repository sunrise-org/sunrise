//! Original GPU probes of the production terrain inputs, without a resource or shader pack.

use crate::gfx::pack_post::terrain_tables;
use crate::mesher::TerrainVertex;
use sunrise_render_core::UV_STEPS;
use sunrise_render_shaderpack::pack_atlas::PackAtlas;
use wgpu::util::DeviceExt;

#[test]
#[ignore = "daily"]
fn mid_tex_coord_follows_each_quads_mapped_corners() {
    let _gpu = crate::gfx::test_gpu::lock();
    let Some(adapter) = crate::gfx::test_gpu::adapter() else {
        return;
    };
    let (device, queue) = pollster::block_on(adapter.request_device(&Default::default())).unwrap();
    eprintln!("midpoint probe adapter: {:?}", adapter.get_info());
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: None,
        size: wgpu::Extent3d {
            width: 16,
            height: 16,
            depth_or_array_layers: 5,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    let atlas = PackAtlas::new(&device, &mut encoder, &[texture], 5, 5, 16).unwrap();
    queue.submit([encoder.finish()]);
    // Each row uses a different atlas cell. Expected atlas coordinates are independent
    // numbers, including the existing clamp of a model UV beyond the sprite's edge.
    let cases = [
        (
            "whole",
            [[0., 0.], [1., 0.], [1., 1.], [0., 1.]],
            [1. / 6., 0.25],
            [1. / 6., 0.25],
            [[0., 0.], [1. / 3., 0.], [1. / 3., 0.5], [0., 0.5]],
        ),
        (
            "half-height",
            [[0., 0.], [1., 0.], [1., 0.5], [0., 0.5]],
            [0.5, 0.125],
            [0.5, 0.25],
            [
                [1. / 3., 0.],
                [2. / 3., 0.],
                [2. / 3., 0.25],
                [1. / 3., 0.25],
            ],
        ),
        (
            "rotated partial",
            [[0.25, 0.75], [0.25, 0.25], [0.5, 0.25], [0.5, 0.75]],
            [19. / 24., 0.25],
            [5. / 6., 0.25],
            [
                [0.75, 0.375],
                [0.75, 0.125],
                [5. / 6., 0.125],
                [5. / 6., 0.375],
            ],
        ),
        (
            "clamped",
            [[0., 0.], [1.5, 0.], [1.5, 0.5], [0., 0.5]],
            [1. / 6., 0.625],
            [1. / 6., 0.75],
            [[0., 0.5], [1. / 3., 0.5], [1. / 3., 0.75], [0., 0.75]],
        ),
        (
            "fourth corner",
            [[0., 0.], [1., 0.], [1., 0.5], [0., 1.]],
            [0.5, 0.6875],
            [0.5, 0.75],
            [
                [1. / 3., 0.5],
                [2. / 3., 0.5],
                [2. / 3., 0.75],
                [1. / 3., 1.],
            ],
        ),
    ];
    let positions = [[0., 0., 0.], [1., 0., 0.], [1., 1., 0.], [0., 1., 0.]];
    let vertices: Vec<_> = cases
        .iter()
        .enumerate()
        .flat_map(|(layer, (_, uv, ..))| {
            (0..4).map(move |k| {
                TerrainVertex::new(positions[k], uv[k], layer as u16 | 0x8000, u32::MAX, 0)
            })
        })
        .collect();
    let mut failures = Vec::new();
    // Positive origins encode fade bits beside the power-of-two scale. Negative origins
    // use the older fractional scale encoding. Both must match the mapped vertex UVs.
    for origin in [
        f32::from_bits((1. / UV_STEPS).to_bits() | 0x12345),
        -3. - 1. / UV_STEPS,
    ] {
        for flat in [true, false] {
            for shadow in [false, true] {
                let pixels = sample(&device, &queue, &atlas, &vertices, origin, [flat, shadow]);
                for (quad, (name, _, midpoint, fallback, mapped)) in cases.iter().enumerate() {
                    let centre = if flat { midpoint } else { fallback };
                    for (corner, uv) in mapped.iter().enumerate() {
                        let actual = pixels[4 * quad + corner];
                        let expected = [centre[0], centre[1], uv[0], uv[1]];
                        if actual
                            .iter()
                            .zip(expected)
                            .any(|(a, b)| !a.is_finite() || (a - b).abs() > 1e-6)
                        {
                            failures.push(format!("{name} corner {corner}, flat={flat} shadow={shadow} origin={origin}: {actual:?} != {expected:?}"));
                        }
                    }
                }
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// A stitched atlas (the one `stitched_sprites_keep_their_own_size_and_padding` works out: 16 by
/// 8, sprites of 2, 4 and 1 px at (9, 1), (1, 1) and (9, 5)): flat draws map a quad's UVs onto its
/// sprite's own pixels, so a corner is half the sprite's own side from `mc_midTexCoord`.
#[test]
#[ignore = "daily"]
fn flat_draws_map_uvs_onto_a_stitched_sprite() {
    let _gpu = crate::gfx::test_gpu::lock();
    let Some(adapter) = crate::gfx::test_gpu::adapter() else {
        return;
    };
    let (device, queue) = pollster::block_on(adapter.request_device(&Default::default())).unwrap();
    let array = super::midpoint_support::upscaled_array(&device, &queue, &[2, 4, 1]);
    let mut encoder = device.create_command_encoder(&Default::default());
    let atlas =
        PackAtlas::stitched(&device, &mut encoder, &[array], 3, 4, &[2, 4, 1], 0, 0).unwrap();
    queue.submit([encoder.finish()]);
    let whole = [[0., 0.], [1., 0.], [1., 1.], [0., 1.]];
    let positions = [[0., 0., 0.], [1., 0., 0.], [1., 1., 0.], [0., 1., 0.]];
    let vertices: Vec<_> = (0..3)
        .flat_map(|layer| {
            (0..4).map(move |k| {
                TerrainVertex::new(positions[k], whole[k], layer as u16 | 0x8000, u32::MAX, 0)
            })
        })
        .collect();
    // Corner and side of each layer's sprite, in atlas pixels.
    let sprites = [([9., 1.], 2.), ([1., 1.], 4.), ([9., 5.], 1.)];
    let origin = f32::from_bits((1. / UV_STEPS).to_bits());
    let pixels = sample(&device, &queue, &atlas, &vertices, origin, [true, false]);
    let mut failures = Vec::new();
    for (layer, ([x, y], side)) in sprites.into_iter().enumerate() {
        let mid = [(x + side / 2.) / 16., (y + side / 2.) / 8.];
        for (corner, [u, v]) in whole.iter().enumerate() {
            let expected = [mid[0], mid[1], (x + u * side) / 16., (y + v * side) / 8.];
            let actual = pixels[4 * layer + corner];
            if actual
                .iter()
                .zip(expected)
                .any(|(a, b)| (a - b).abs() > 1e-6)
            {
                failures.push(format!(
                    "layer {layer} corner {corner}: {actual:?} != {expected:?}"
                ));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// Read back with a probe pack of ours, Iris 1.11.7's terrain `gl_Vertex` (camera on the block
/// grid) puts a water top, 8/9 - 0.001 of a block up, at 29094/32768, and plants offset by 13/15
/// and 0.1 of a block at 28398/32768 and 3276/32768 (#8464): each 32768th below the position.
/// Whole blocks and sixteenths stay where they are, and so does a place float rounding put a hair
/// below a grid line: Iris reads plants offset by 9/15 exactly on a half block.
#[test]
#[ignore = "daily"]
fn terrain_vertices_go_down_to_the_32768th_below_them() {
    let _gpu = crate::gfx::test_gpu::lock();
    let Some(adapter) = crate::gfx::test_gpu::adapter() else {
        return;
    };
    let (device, queue) = pollster::block_on(adapter.request_device(&Default::default())).unwrap();
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: None,
        size: wgpu::Extent3d {
            width: 16,
            height: 16,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    let atlas = PackAtlas::new(&device, &mut encoder, &[texture], 1, 1, 16).unwrap();
    queue.submit([encoder.finish()]);
    let water = 8.0f32 / 9.0 - 0.001;
    let cases = [
        ([0.1, water, 13.0 / 15.0], [3276.0, 29094.0, 28398.0]),
        ([13.0 / 15.0, 0.1, water], [28398.0, 3276.0, 29094.0]),
        ([1.0, 0.5, 15.9375], [32768.0, 16384.0, 522240.0]),
        // One float step below 1, 0.5 and 9.5.
        (
            [
                1.0 - f32::EPSILON / 2.0,
                0.5 - f32::EPSILON / 4.0,
                9.5 - f32::EPSILON * 8.0,
            ],
            [32768.0, 16384.0, 311296.0],
        ),
    ];
    let vertices: Vec<_> = cases
        .iter()
        .map(|&(pos, _)| TerrainVertex::new(pos, [0.0, 0.0], 0x8000, u32::MAX, 0))
        .collect();
    let origin = f32::from_bits((1. / UV_STEPS).to_bits());
    let mut failures = Vec::new();
    for flat in [true, false] {
        for shadow in [false, true] {
            let read = sample_probe(
                &device,
                &queue,
                &atlas,
                &vertices,
                origin,
                [flat, shadow],
                "gl_Vertex",
            );
            for ((pos, expected), got) in cases.iter().zip(&read) {
                let got = [got[0], got[1], got[2]].map(|v| v * 32768.0);
                if got != *expected {
                    failures.push(format!(
                        "{pos:?}, flat={flat} shadow={shadow}: {got:?} != {expected:?}"
                    ));
                }
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// A shader pack's flat terrain inputs read a compact record (`mesher::packed`) as the four
/// vertices it holds: a whole tinted face with its own ambient occlusion per corner
/// (`separateAo`), two more, and a slab's top kept as its vertices (`packed::FULL`).
#[test]
#[ignore = "daily"]
fn compact_records_feed_a_pack_the_vertices_they_hold() {
    let _gpu = crate::gfx::test_gpu::lock();
    use crate::mesher::packed;
    let Some(adapter) = crate::gfx::test_gpu::adapter() else {
        return;
    };
    let (device, queue) = pollster::block_on(adapter.request_device(&Default::default())).unwrap();
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: None,
        size: wgpu::Extent3d {
            width: 16,
            height: 16,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    let mut atlas = PackAtlas::new(&device, &mut encoder, &[texture], 1, 1, 16).unwrap();
    queue.submit([encoder.finish()]);
    atlas.separate_ao = true;
    let tint = 0x7F_B238u32;
    let ao = [255u32, 153, 102, 204];
    let face = |x: f32, y: f32| {
        let pos = [
            [x, y, 5.0],
            [x, y, 6.0],
            [x + 1.0, y, 6.0],
            [x + 1.0, y, 5.0],
        ];
        let uv = [[0, 0], [16 << 10, 0], [16 << 10, 16 << 10], [0, 16 << 10]];
        (0..4).map(move |i| {
            let [r, g, b] = [16, 8, 0].map(|s| 200 * (tint >> s & 255) / 255);
            TerrainVertex {
                pos: pos[i],
                uv: uv[i],
                layer: 0x8000,
                light: 0xF0 | 0x40 << 8,
                color: r | g << 8 | b << 16 | ao[i] << 24,
            }
        })
    };
    let solid: Vec<_> = [(3.0, 16.0), (5.0, 16.0), (7.0, 16.0), (3.0, 8.5)]
        .into_iter()
        .flat_map(|(x, y)| face(x, y))
        .collect();
    let records: Vec<[u32; 6]> = packed::layout(&solid, &[tint; 4]);
    assert_eq!(
        packed::packed_quads(&records, 4),
        3,
        "whole faces pack, the slab doesn't"
    );
    let page = Page::vertices(bytemuck::cast_slice(&records));
    let packed_range = packed::PACKED_BASE..packed::PACKED_BASE + 16;
    let origin = f32::from_bits((1. / UV_STEPS).to_bits());
    let mut failures = Vec::new();
    for expression in [
        "gl_Vertex",
        "gl_Color",
        "vec4(gl_MultiTexCoord0.xy, gl_MultiTexCoord1.xy)",
        "vec4(gl_Normal, 0.0)",
        "at_tangent",
        "at_midBlock",
        "vec4(mc_midTexCoord, mc_Entity.xy)",
    ] {
        for shadow in [false, true] {
            let flags = [true, shadow];
            let wide = sample_probe(&device, &queue, &atlas, &solid, origin, flags, expression);
            let compact = sample_page(
                &device,
                &queue,
                &atlas,
                page,
                packed_range.clone(),
                origin,
                flags,
                expression,
            );
            let bits = |v: &[[f32; 4]]| v.iter().flatten().map(|f| f.to_bits()).collect::<Vec<_>>();
            if bits(&wide) != bits(&compact) {
                failures.push(format!(
                    "{expression} shadow={shadow}: {wide:?} != {compact:?}"
                ));
            }
            if expression == "gl_Color" {
                for (corner, want) in ao.iter().enumerate() {
                    let got = compact[corner][3];
                    if (got - *want as f32 / 255.0).abs() > 1e-6 {
                        failures.push(format!("corner {corner}'s alpha {got} != {want}/255"));
                    }
                }
            }
        }
    }
    // A pack's frames keep the translucent quads in the section arena after the solid range: three
    // records and no `FULL` quad, then a water quad, whose corners must read as its own.
    let mut page: Vec<[u32; 6]> = packed::layout(&solid[..12], &[tint; 3]);
    let water: Vec<_> = face(9.0, 15.0).collect();
    let at = page.len() as u32;
    page.extend(water.iter().map(|v| bytemuck::cast::<_, [u32; 6]>(*v)));
    for expression in ["vec4(gl_Normal, 0.0)", "at_tangent", "at_midBlock"] {
        let flags = [true, false];
        let wide = sample_probe(&device, &queue, &atlas, &water, origin, flags, expression);
        let page = Page::vertices(bytemuck::cast_slice(&page));
        let after = sample_page(
            &device,
            &queue,
            &atlas,
            page,
            at..at + 4,
            origin,
            flags,
            expression,
        );
        if wide != after {
            failures.push(format!("water {expression}: {wide:?} != {after:?}"));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// A shader pack's flat terrain inputs read a far tile's compact record (`lod::packed`, drawn
/// past `FAR_BASE`) as the four vertices it holds, as terrain.wgsl's `section_corner` does: a
/// face of each axis and winding, and one no record holds, kept as its vertices
/// (`packed::FULL`). Read as a chunk section's record instead, far land lost its place (#5016).
/// Each face has its own sprite cell in a 4 by 4 atlas, its own size and its own alpha byte
/// (`separateAo`), and a UV scale small enough that no UV clamps shows the record's UV steps.
#[test]
#[ignore = "daily"]
fn far_records_feed_a_pack_the_vertices_they_hold() {
    let _gpu = crate::gfx::test_gpu::lock();
    use crate::lod::packed;
    let Some(adapter) = crate::gfx::test_gpu::adapter() else {
        return;
    };
    let (device, queue) = pollster::block_on(adapter.request_device(&Default::default())).unwrap();
    // 16 layers of 16 pixels: a 64 by 64 atlas, layer `l` at cell (l % 4, l / 4).
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: None,
        size: wgpu::Extent3d {
            width: 16,
            height: 16,
            depth_or_array_layers: 16,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    let mut atlas = PackAtlas::new(&device, &mut encoder, &[texture], 16, 16, 16).unwrap();
    queue.submit([encoder.finish()]);
    assert_eq!(atlas.size(), [64, 64]);
    atlas.separate_ao = true;
    let mut solid = packed::sample_faces();
    let faces = solid.len() / 4;
    // Half a block off the grid: no record holds it.
    let mut spilled: Vec<_> = solid[..4].to_vec();
    for v in &mut spilled {
        v.pos[1] += 0.5;
    }
    solid.extend(spilled);
    let records = packed::layout(&solid);
    assert_eq!(
        crate::mesher::packed::packed_quads(&records, faces + 1),
        faces,
        "the faces pack, the one off the grid doesn't"
    );
    let page = Page::vertices(bytemuck::cast_slice(&records));
    let far = packed::FAR_BASE..packed::FAR_BASE + solid.len() as u32;
    // Far tiles' own UV scale (`lod::instance`), where every UV past a block clamps to the
    // sprite's edge, and 1/4096, where none of the fixture's (at most 21 blocks) does.
    let production = crate::gfx::lod::instance([0.0; 3], 0)[3];
    let small = 1.0 / 4096.0;
    let mut failures = Vec::new();
    let mut read = |expression: &str, scale: f32, shadow: bool| {
        let flags = [true, shadow];
        let wide = sample_probe(&device, &queue, &atlas, &solid, scale, flags, expression);
        let compact = sample_page(
            &device,
            &queue,
            &atlas,
            page,
            far.clone(),
            scale,
            flags,
            expression,
        );
        let bits = |v: &[[f32; 4]]| v.iter().flatten().map(|f| f.to_bits()).collect::<Vec<_>>();
        if bits(&wide) != bits(&compact) {
            failures.push(format!(
                "{expression} scale={scale} shadow={shadow}: {wide:?} != {compact:?}"
            ));
        }
        compact
    };
    for scale in [production, small] {
        for expression in [
            "gl_Vertex",
            "gl_Color",
            "vec4(gl_MultiTexCoord0.xy, gl_MultiTexCoord1.xy)",
            "vec4(gl_Normal, 0.0)",
            "at_tangent",
            "vec4(mc_midTexCoord, mc_Entity.xy)",
        ] {
            for shadow in [false, true] {
                read(expression, scale, shadow);
            }
        }
    }
    let uv = read("gl_MultiTexCoord0", small, false);
    let color = read("gl_Color", production, false);
    for (face, quad) in solid.chunks(4).take(faces).enumerate() {
        let corners = &uv[face * 4..face * 4 + 4];
        // The face's sprite cell, its corner at UV 0: a quarter of the atlas a cell.
        let layer = u32::from(quad[0].layer);
        let cell = [(layer % 4) as f32 * 0.25, (layer / 4) as f32 * 0.25];
        let least = [0, 1].map(|i| corners.iter().map(|c| c[i]).fold(f32::INFINITY, f32::min));
        let most = [0, 1].map(|i| {
            corners
                .iter()
                .map(|c| c[i])
                .fold(f32::NEG_INFINITY, f32::max)
        });
        // Its rectangle's width and height in blocks, from the corners' positions: the axis
        // they share is the plane's, u and v are `lod::mesh::emit_quad`'s (Y: x, z; Z: x, y;
        // X: z, y).
        let extent = |a: usize| {
            let p = quad.iter().map(|v| v.pos[a]);
            p.clone().fold(f32::NEG_INFINITY, f32::max) - p.fold(f32::INFINITY, f32::min)
        };
        let plane = (0..3).find(|&a| extent(a) == 0.0).expect("a flat face");
        let (u, v) = match plane {
            1 => (0, 2),
            2 => (0, 1),
            _ => (2, 1),
        };
        // 32 UV steps a block (`mesher::TILED_UV_STEPS`), times the scale, times the cell's
        // quarter of the atlas: 1/512 of the atlas a block.
        let span = [extent(u) / 512.0, extent(v) / 512.0];
        let got = [most[0] - least[0], most[1] - least[1]];
        if least != cell || got != span {
            failures.push(format!(
                "face {face} (layer {layer}): UVs from {least:?} spanning {got:?}, \
                 expected from {cell:?} spanning {span:?}"
            ));
        }
        // `separateAo`: the colour's alpha byte, as the vertex fetch converts a Unorm8.
        let alpha = (quad[0].color >> 24) as f32 * (1.0 / 255.0);
        for (k, c) in color[face * 4..face * 4 + 4].iter().enumerate() {
            if c[3].to_bits() != alpha.to_bits() {
                failures.push(format!("face {face} corner {k}: alpha {} != {alpha}", c[3]));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

fn sample(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    atlas: &PackAtlas,
    vertices: &[TerrainVertex],
    scale: f32,
    flags: [bool; 2],
) -> Vec<[f32; 4]> {
    sample_probe(
        device,
        queue,
        atlas,
        vertices,
        scale,
        flags,
        "vec4(mc_midTexCoord, gl_MultiTexCoord0.xy)",
    )
}

/// What the pack's vertex stage sets `probe` to (`expression`), for each of `vertices`.
fn sample_probe(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    atlas: &PackAtlas,
    vertices: &[TerrainVertex],
    scale: f32,
    flags: [bool; 2],
    expression: &str,
) -> Vec<[f32; 4]> {
    let page = Page::vertices(bytemuck::cast_slice(vertices));
    let drawn = 0..vertices.len() as u32;
    sample_page(device, queue, atlas, page, drawn, scale, flags, expression)
}

/// An arena page as flat draws bind it: its slots' bytes, its sources' (`Arena::write_sources`),
/// and the ids each state's model and fluid map to (`terrain_tables`).
#[derive(Clone, Copy)]
pub(super) struct Page<'a> {
    pub slots: &'a [u8],
    pub sources: &'a [u8],
    pub ids: &'a [i32],
}

impl<'a> Page<'a> {
    /// `slots` with every source unknown.
    fn vertices(slots: &'a [u8]) -> Self {
        Self {
            slots,
            sources: &[0; 8],
            ids: &[],
        }
    }
}

/// `sample_probe` of the vertex indices `drawn` of an arena page holding `page`.
#[allow(clippy::too_many_arguments)]
pub(super) fn sample_page(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    atlas: &PackAtlas,
    page: Page<'_>,
    drawn: std::ops::Range<u32>,
    scale: f32,
    [flat, shadow]: [bool; 2],
    expression: &str,
) -> Vec<[f32; 4]> {
    let count = drawn.len() as u32;
    let pp = |code: String| sunrise_shaders::Preprocessed::from_source("", &code);
    let vertex = pp(format!(
        "attribute vec2 mc_midTexCoord;\nattribute vec4 at_tangent;\n\
         attribute vec4 at_midBlock;\nattribute vec4 mc_Entity;\nvarying vec4 probe;\nvoid main() {{\n\
         probe = {expression};\n\
         gl_Position = vec4((float(gl_VertexID - {}) + 0.5) * {} - 1.0, 0.0, 0.0, 1.0);\n}}",
        drawn.start,
        2. / count as f32,
    ));
    // Integer attachments retain the float bits without requiring float-render-target
    // capabilities on the adapter that the test runner picks.
    let fragment = pp(
        "varying vec4 probe;\nlayout(location = 0) out uvec4 midpoint_bits;\n\
        void main() { midpoint_bits = floatBitsToUint(probe); }"
            .into(),
    );
    let inputs = atlas.terrain_inputs(flat);
    let program = if shadow {
        sunrise_shaders::translate_shadow(
            &vertex,
            &fragment,
            &inputs,
            naga::front::glsl::InterfaceLimits::WGPU_DEFAULT,
        )
    } else {
        sunrise_shaders::translate_gbuffers(
            &vertex,
            &fragment,
            sunrise_shaders::Target::Buffers,
            &inputs,
            naga::front::glsl::InterfaceLimits::WGPU_DEFAULT,
        )
    }
    .unwrap();
    let storage = |binding| wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::VERTEX,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only: true },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    };
    let entries = [
        vec![wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        }],
        vec![storage(0), storage(1)],
        vec![],
        vec![storage(0), storage(1), storage(2)],
    ];
    let layouts: Vec<_> = entries
        .iter()
        .map(|entries| {
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: None,
                entries,
            })
        })
        .collect();
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: None,
        bind_group_layouts: &layouts.iter().map(Some).collect::<Vec<_>>(),
        ..Default::default()
    });
    let module = |module| {
        device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: None,
            source: wgpu::ShaderSource::Naga(std::borrow::Cow::Owned(module)),
        })
    };
    let vs = module(program.vertex);
    let fs = module(program.fragment);
    let vertex_layouts = [
        Some(wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<TerrainVertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &crate::gfx::TERRAIN_ATTRIBUTES,
        }),
        Some(wgpu::VertexBufferLayout {
            array_stride: 16,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &wgpu::vertex_attr_array![5 => Float32x4],
        }),
    ];
    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: None,
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: &vs,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            // Flat draws pull their vertices from the page (`PULL_FLAT`), as in the game.
            buffers: if flat { &[] } else { &vertex_layouts[..] },
        },
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::PointList,
            ..Default::default()
        },
        depth_stencil: None,
        multisample: Default::default(),
        fragment: Some(wgpu::FragmentState {
            module: &fs,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: wgpu::TextureFormat::Rgba32Uint,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        cache: None,
    });
    let buffer = |contents: &[u8], usage| {
        device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: None,
            contents,
            usage,
        })
    };
    let uniform = buffer(
        &vec![0; program.interface.uniform_size.max(16) as usize],
        wgpu::BufferUsages::UNIFORM,
    );
    // Every 256-slot block of the page names the one origin.
    let blocks = vec![0u32; page.slots.len() / 24 / 256 + 1];
    let slots = buffer(bytemuck::cast_slice(&blocks), wgpu::BufferUsages::STORAGE);
    let origin = buffer(
        bytemuck::bytes_of(&[0., 0., 0., scale]),
        wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::VERTEX,
    );
    let ids = buffer(page.sources, wgpu::BufferUsages::STORAGE);
    let data = buffer(
        page.slots,
        wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::VERTEX,
    );
    let rects = buffer(
        bytemuck::cast_slice(&terrain_tables(page.ids, &atlas.rects_table())),
        wgpu::BufferUsages::STORAGE,
    );
    let bindings = [
        vec![(0, &uniform)],
        vec![(0, &slots), (1, &origin)],
        vec![],
        vec![(0, &ids), (1, &data), (2, &rects)],
    ];
    let groups: Vec<_> = bindings
        .iter()
        .zip(&layouts)
        .map(|(buffers, layout)| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: None,
                layout,
                entries: &buffers
                    .iter()
                    .map(|&(binding, buffer)| wgpu::BindGroupEntry {
                        binding,
                        resource: buffer.as_entire_binding(),
                    })
                    .collect::<Vec<_>>(),
            })
        })
        .collect();
    let output = device.create_texture(&wgpu::TextureDescriptor {
        label: None,
        size: wgpu::Extent3d {
            width: count,
            height: 1,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba32Uint,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let read = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: u64::from(count) * 16,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    {
        let view = output.create_view(&Default::default());
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: None,
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
            })],
            ..Default::default()
        });
        pass.set_pipeline(&pipeline);
        for (k, group) in groups.iter().enumerate() {
            pass.set_bind_group(k as u32, group, &[]);
        }
        if !flat {
            pass.set_vertex_buffer(0, data.slice(..));
            pass.set_vertex_buffer(1, origin.slice(..));
        }
        pass.draw(drawn, 0..1);
    }
    encoder.copy_texture_to_buffer(
        output.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &read,
            layout: wgpu::TexelCopyBufferLayout::default(),
        },
        output.size(),
    );
    queue.submit([encoder.finish()]);
    read.slice(..).map_async(wgpu::MapMode::Read, |_| {});
    device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    let mapped = read.slice(..).get_mapped_range().unwrap();
    bytemuck::cast_slice::<u8, [f32; 4]>(&mapped).to_vec()
}
