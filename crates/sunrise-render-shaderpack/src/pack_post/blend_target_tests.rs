//! What composite-style programs draw for their `blend.*` keys, as `docs/cleanroom/decisions.md`
//! D-13 records them: `shadowcomp`'s per-buffer lines, `final` never blending, and only
//! `BLENDABLE` formats blending. The keys themselves are pinned in `sunrise-shaders`'
//! `post::blend_key_tests`. Each test builds the pass with `build_pass`, draws it once over
//! buffers cleared to a known texel, and reads each buffer's first texel back.

use sunrise_shaders::{BUFFERS, Group};
use wgpu::TextureFormat as T;

use super::*;

pub(super) const VSH: &str = "#version 120\nvoid main() { gl_Position = ftransform(); }\n";

/// The program `name` of a pack of `files`, loaded with `properties` (the shadow pass forced on).
pub(super) fn program(
    files: &[(&str, &str)],
    properties: &str,
    name: &str,
) -> sunrise_shaders::PostProgram {
    let dir = tempfile::tempdir().unwrap();
    let shaders = dir.path().join("shaders");
    std::fs::create_dir_all(&shaders).unwrap();
    for (file, text) in files {
        std::fs::write(shaders.join(file), text).unwrap();
    }
    let pack = sunrise_shaders::Pack::open(dir.path()).unwrap();
    let properties = sunrise_shaders::Properties::parse(
        &format!("shadow.enabled = true\n{properties}"),
        &sunrise_shaders::Macros::default(),
    );
    let post = sunrise_shaders::load_post(
        &pack,
        "",
        &properties,
        &sunrise_shaders::Macros::standard(),
        &Default::default(),
    );
    (post.programs.into_iter().chain(post.shadow_comp))
        .find(|p| p.name == name)
        .unwrap_or_else(|| panic!("{name} loads: {:?}", post.notes))
}

/// Builds `p` with its buffers in `formats` and the screen in `Rgba8Unorm` (a format that
/// blends), draws it once into one attachment per output location, each of `attachments`'
/// format and cleared to its colour first, and returns the first texel of each, as bytes.
pub(super) fn draw(
    p: sunrise_shaders::PostProgram,
    formats: &[wgpu::TextureFormat; BUFFERS],
    attachments: &[(wgpu::TextureFormat, wgpu::Color)],
) -> Option<Vec<Vec<u8>>> {
    let _gpu = crate::test_gpu::lock();
    let adapter = crate::test_gpu::adapter()?;
    // A pass's pipeline layout has six bind group slots. No optional feature is asked for, so
    // `Rgba32Float` doesn't blend on this device (wgpu needs `FLOAT32_BLENDABLE` for it).
    let desc = wgpu::DeviceDescriptor {
        required_limits: adapter.limits(),
        ..Default::default()
    };
    let (device, queue) = crate::test_gpu::optional_device_with(&adapter, &desc)?;
    eprintln!("adapter: {:?}", adapter.get_info().name);
    assert!(
        !(T::Rgba32Float
            .guaranteed_format_features(device.features())
            .flags)
            .contains(wgpu::TextureFormatFeatureFlags::BLENDABLE),
        "the fixture needs a device where Rgba32Float doesn't blend"
    );
    let buffers = crate::pack_images::Buffers::new(
        &device,
        Default::default(),
        Default::default(),
        wgpu::ShaderStages::FRAGMENT,
    );
    let stand_ins = std::cell::RefCell::new(Vec::new());
    let mismatches = std::cell::RefCell::new(std::collections::HashSet::new());
    let pack = Resources {
        textures: &[],
        images: &[],
        left_out: &[],
        buffers: &buffers,
        indices: None,
        gpu: Gpu::of(&adapter, device.features()),
        storable: 0,
        rgb: 0,
        packed: [None; BUFFERS],
        released: std::cell::Cell::new(("", 0)),
        stand_ins: &stand_ins,
        shadow_linear: 0,
        jobs: None,
        mod_depth_targets: &[],
        mod_uniforms: &[],
        display_uniforms: false,
        mod_uniform_mismatches: &mismatches,
        mod_depth_target: std::cell::Cell::new(None),
        depth_compare: std::cell::Cell::new(wgpu::CompareFunction::LessEqual),
        mod_depth_bias: Default::default(),
        mod_vertex: Default::default(),
    };
    let draws = crate::pack_draw::DrawUniforms::new(&device);
    let mut notes = Vec::new();
    let pass = build_pass(
        &device,
        p,
        formats,
        None,
        T::Rgba8Unorm,
        &mut notes,
        None,
        &[],
        &draws.layout,
        &pack,
        false,
        false,
    )
    .unwrap_or_else(|| panic!("the pass builds: {notes:?}"));
    const WIDTH: u32 = 64;
    let textures: Vec<wgpu::Texture> = (attachments.iter())
        .map(|&(format, _)| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: None,
                size: wgpu::Extent3d {
                    width: WIDTH,
                    height: 1,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            })
        })
        .collect();
    let views: Vec<wgpu::TextureView> = (textures.iter())
        .map(|t| t.create_view(&Default::default()))
        .collect();
    let head = [WIDTH as f32, 1.0, 1.0 / WIDTH as f32, 1.0];
    queue.write_buffer(&pass.ubo, 0, bytemuck::cast_slice(&head));
    let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None,
        layout: &pass.layout,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: pass.ubo.as_entire_binding(),
        }],
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    let clear: Vec<_> = (views.iter().zip(attachments))
        .map(|(view, &(_, before))| {
            attach(
                view,
                wgpu::Operations {
                    load: wgpu::LoadOp::Clear(before),
                    store: wgpu::StoreOp::Store,
                },
            )
        })
        .collect();
    encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        color_attachments: &clear,
        ..Default::default()
    });
    {
        // Loaded, as `PackPost::render` attaches a buffer a composite pass writes.
        let load: Vec<_> = (views.iter())
            .map(|view| {
                attach(
                    view,
                    wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                )
            })
            .collect();
        let mut rp = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            color_attachments: &load,
            ..Default::default()
        });
        rp.set_pipeline(pass.render());
        rp.set_bind_group(0, &group, &[]);
        rp.draw(0..6, 0..1);
    }
    let reads: Vec<(wgpu::Buffer, u32)> = (textures.iter())
        .map(|texture| {
            let texel = texture.format().block_copy_size(None).unwrap();
            let read = device.create_buffer(&wgpu::BufferDescriptor {
                label: None,
                size: u64::from(texel * WIDTH),
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });
            encoder.copy_texture_to_buffer(
                texture.as_image_copy(),
                wgpu::TexelCopyBufferInfo {
                    buffer: &read,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(texel * WIDTH),
                        rows_per_image: None,
                    },
                },
                texture.size(),
            );
            (read, texel)
        })
        .collect();
    queue.submit([encoder.finish()]);
    for (read, _) in &reads {
        read.slice(..).map_async(wgpu::MapMode::Read, |_| {});
    }
    device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    Some(
        reads
            .iter()
            .map(|(read, texel)| {
                read.slice(..).get_mapped_range().unwrap()[..*texel as usize].to_vec()
            })
            .collect(),
    )
}

fn color(r: f64, g: f64, b: f64, a: f64) -> wgpu::Color {
    wgpu::Color { r, g, b, a }
}

fn floats(bytes: &[u8]) -> Vec<f32> {
    bytemuck::pod_collect_to_vec(bytes)
}

/// D-13: `blend.shadowcomp.colortex0` blends shadowcolor0, the buffer of index 0, whichever
/// output location writes it: here location 1 (`DRAWBUFFERS:10`). `blend.shadowcomp` is not
/// read and `blend.shadowcomp.shadowcolor1` names no buffer, so shadowcolor1, at location 0, is
/// written as the program wrote it.
#[test]
#[ignore = "daily"]
fn shadowcomp_blends_the_shadow_buffer_its_colortex_line_numbers() {
    let p = program(
        &[
            ("shadowcomp.vsh", VSH),
            (
                "shadowcomp.fsh",
                "#version 120\n/* DRAWBUFFERS:10 */\nvoid main() {\n\
                 gl_FragData[0] = vec4(1.0, 0.0, 1.0, 0.0);\n\
                 gl_FragData[1] = vec4(0.0, 0.0, 1.0, 1.0);\n}\n",
            ),
        ],
        "blend.shadowcomp = ZERO ONE ZERO ONE\n\
         blend.shadowcomp.colortex0 = ONE ONE ONE ONE\n\
         blend.shadowcomp.shadowcolor1 = ZERO ZERO ZERO ZERO\n",
        "shadowcomp",
    );
    assert_eq!((p.group, &p.buffers[..]), (Group::ShadowComp, &[1, 0][..]));
    let Some(texels) = draw(
        p,
        &[T::Rgba8Unorm; BUFFERS],
        &[
            (T::Rgba8Unorm, color(0.0, 1.0, 0.0, 1.0)),
            (T::Rgba8Unorm, color(1.0, 0.0, 0.0, 0.0)),
        ],
    ) else {
        return;
    };
    // shadowcolor1 as written; shadowcolor0 the sum of the two, ONE ONE ONE ONE.
    assert_eq!(texels, [vec![255, 0, 255, 0], vec![255, 0, 255, 255]]);
}

/// D-13: `final` writes the screen without blending, though the loader keeps `blend.final`'s
/// factors on the program and the screen's format blends.
#[test]
#[ignore = "daily"]
fn final_writes_the_screen_without_blending() {
    let p = program(
        &[
            ("final.vsh", VSH),
            (
                "final.fsh",
                "#version 120\nvoid main() { gl_FragData[0] = vec4(1.0, 0.0, 1.0, 0.0); }\n",
            ),
        ],
        "blend.final = ONE ONE ONE ONE\n",
        "final",
    );
    assert!(p.group == Group::Final && p.blend.is_some());
    let Some(texels) = draw(
        p,
        &[T::Rgba8Unorm; BUFFERS],
        &[(T::Rgba8Unorm, color(0.0, 1.0, 0.0, 1.0))],
    ) else {
        return;
    };
    // As written; blended, ONE ONE ONE ONE would read 255 in every channel.
    assert_eq!(texels, [vec![255, 0, 255, 0]]);
}

/// D-13: factors apply only to an output whose buffer format wgpu reports as `BLENDABLE`.
/// `R32Uint` (an integer format) and `Rgba32Float` (a float format, on a device without
/// `FLOAT32_BLENDABLE`) are written without blending, under `blend.<program>` (composite) and
/// under a per-buffer line (shadowcomp), while the `Rgba8Unorm` buffer beside them blends.
#[test]
#[ignore = "daily"]
fn only_blendable_formats_blend() {
    let composite = program(
        &[
            ("composite.vsh", VSH),
            (
                "composite.fsh",
                "#version 120\n/* DRAWBUFFERS:345 */\nvoid main() {\n\
                 gl_FragData[0] = vec4(1.0);\n\
                 gl_FragData[1] = vec4(1.0, 0.0, 1.0, 0.0);\n\
                 gl_FragData[2] = vec4(0.25, 0.5, 0.75, 1.0);\n}\n",
            ),
        ],
        "blend.composite = ONE ONE ONE ONE\n",
        "composite",
    );
    assert_eq!(&composite.buffers[..], &[3, 4, 5]);
    let mut formats = [T::Rgba8Unorm; BUFFERS];
    formats[3] = T::R32Uint;
    formats[5] = T::Rgba32Float;
    let Some(texels) = draw(
        composite,
        &formats,
        &[
            (T::R32Uint, color(7.0, 0.0, 0.0, 0.0)),
            (T::Rgba8Unorm, color(0.0, 1.0, 0.0, 1.0)),
            (T::Rgba32Float, color(0.5, 0.5, 0.5, 0.5)),
        ],
    ) else {
        return;
    };
    // The integer buffer holds the float's bits (`packed::bitcast_outputs`), 1.0 = 0x3f800000.
    assert_eq!(texels[0], 0x3f80_0000u32.to_le_bytes());
    assert_eq!(texels[1], [255, 255, 255, 255]);
    assert_eq!(floats(&texels[2]), [0.25, 0.5, 0.75, 1.0]);

    let shadowcomp = program(
        &[
            ("shadowcomp.vsh", VSH),
            (
                "shadowcomp.fsh",
                "#version 120\nvoid main() {\n\
                 gl_FragData[0] = vec4(0.25, 0.5, 0.75, 1.0);\n\
                 gl_FragData[1] = vec4(1.0, 0.0, 1.0, 0.0);\n}\n",
            ),
        ],
        "blend.shadowcomp.colortex0 = ONE ONE ONE ONE\n\
         blend.shadowcomp.colortex1 = ONE ONE ONE ONE\n",
        "shadowcomp",
    );
    let mut formats = [T::Rgba8Unorm; BUFFERS];
    formats[0] = T::Rgba32Float;
    let Some(texels) = draw(
        shadowcomp,
        &formats,
        &[
            (T::Rgba32Float, color(0.5, 0.5, 0.5, 0.5)),
            (T::Rgba8Unorm, color(0.0, 1.0, 0.0, 1.0)),
        ],
    ) else {
        return;
    };
    assert_eq!(floats(&texels[0]), [0.25, 0.5, 0.75, 1.0]);
    assert_eq!(texels[1], [255, 255, 255, 255]);
}
