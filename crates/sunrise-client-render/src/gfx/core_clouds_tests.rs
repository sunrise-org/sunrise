use super::*;

const ALL: CloudsFrame = CloudsFrame {
    pack_shader: true,
    clouds: true,
    native_clouds: true,
    improved_transparency: false,
    shader_pack: false,
    title: false,
    cloud_marks: false,
};

/// The pack's clouds draw only with a pack's shader and the clouds on, and only while nothing
/// else draws them its own way: each condition alone leaves the native mesh.
#[test]
fn a_packs_clouds_draw_only_where_nothing_else_draws_them() {
    assert!(ALL.pack_draws());
    let each = [
        CloudsFrame {
            pack_shader: false,
            ..ALL
        },
        CloudsFrame {
            clouds: false,
            ..ALL
        },
        CloudsFrame {
            native_clouds: false,
            ..ALL
        },
        CloudsFrame {
            improved_transparency: true,
            ..ALL
        },
        CloudsFrame {
            shader_pack: true,
            ..ALL
        },
        CloudsFrame { title: true, ..ALL },
        CloudsFrame {
            cloud_marks: true,
            ..ALL
        },
    ];
    for (i, frame) in each.into_iter().enumerate() {
        assert!(!frame.pack_draws(), "condition {i}");
    }
}

/// The adapter's device, or none (said on stderr) where the machine has no adapter.
fn device() -> Option<(wgpu::Device, wgpu::Queue)> {
    let device = crate::gfx::test_gpu::optional_device();
    if device.is_none() {
        eprintln!("skipped: no adapter");
    }
    device.map(|(_, device, queue)| (device, queue))
}

const SIZE: u32 = 8;

/// Draws `core`'s prepared faces into an 8 by 8 target cleared to transparent black, with a
/// depth buffer cleared to the far plane (0, reversed Z), and reads it back.
fn draw(device: &wgpu::Device, queue: &wgpu::Queue, core: &CoreClouds, fancy: bool) -> Vec<u8> {
    let texture = |format, usage| {
        device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d {
                width: SIZE,
                height: SIZE,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage,
            view_formats: &[],
        })
    };
    let target = texture(
        wgpu::TextureFormat::Rgba8Unorm,
        wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
    );
    let depth = texture(
        wgpu::TextureFormat::Depth32Float,
        wgpu::TextureUsages::RENDER_ATTACHMENT,
    );
    let indices = crate::gfx::Renderer::quad_indices(device, core.quads.max(1));
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: 256 * u64::from(SIZE),
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    {
        let view = target.create_view(&Default::default());
        let depth_view = depth.create_view(&Default::default());
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
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &depth_view,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(0.0),
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            ..Default::default()
        });
        core.draw(&mut pass, fancy, &indices);
    }
    encoder.copy_texture_to_buffer(
        target.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &readback,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(256),
                rows_per_image: Some(SIZE),
            },
        },
        target.size(),
    );
    queue.submit([encoder.finish()]);
    readback.slice(..).map_async(wgpu::MapMode::Read, |_| {});
    device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    let data = readback.slice(..).get_mapped_range().unwrap();
    (0..SIZE as usize)
        .flat_map(|row| data[row * 256..row * 256 + SIZE as usize * 4].to_vec())
        .collect()
}

/// `core`'s faces and blocks for `input`, uploaded through the renderer's belt.
fn prepare(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    core: &mut CoreClouds,
    vanilla: &[u8; 1024],
    input: &crate::clouds::Input<'_>,
) -> Result<u32, TooManyFaces> {
    let mut uploads = super::super::upload::Uploads::new(device);
    let mut encoder = device.create_command_encoder(&Default::default());
    let quads = core.prepare(device, &mut uploads, &mut encoder, vanilla, input);
    uploads.finish(&encoder);
    queue.submit([encoder.finish()]);
    device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    quads
}

/// A one-cell texture at x -1 of the camera's cell, fast clouds, the colour opaque: one face,
/// `[-1, 0, -96]` (`clouds_pack_tests`).
fn flat_input(texture: &(u32, u32, Vec<bool>)) -> crate::clouds::Input<'_> {
    crate::clouds::Input {
        texture,
        uv: [0.0; 2],
        camera: glam::DVec3::new(0.0, f64::from(crate::clouds::CLOUD_HEIGHT) + 2.0, -3.96),
        time: 0,
        partial_tick: 0.0,
        range: 1,
        fast: true,
        color: [0.8, 1.0, 1.0, 1.0],
        height: crate::clouds::CLOUD_HEIGHT,
    }
}

/// A probe shader: the whole target covered by face 0's quad (wound clockwise, so a back face):
/// `CloudColor` in its left half, the face's three texels (plus 128) in red, green and blue in
/// its right half.
const PROBE_VSH: &str = "#version 330\n\
    layout(std140) uniform CloudInfo {\n    vec4 CloudColor;\n    vec3 CloudOffset;\n    vec3 CellSize;\n};\n\
    uniform isamplerBuffer CloudFaces;\n\
    layout(location = 0) out vec4 vertexColor;\nlayout(location = 1) out vec4 infoColor;\n\
    void main() {\n    int index = (gl_VertexIndex / 4) * 3;\n    int corner = gl_VertexIndex % 4;\n\
        ivec3 face = ivec3(texelFetch(CloudFaces, index).r, texelFetch(CloudFaces, index + 1).r,\n\
            texelFetch(CloudFaces, index + 2).r);\n\
        vec2 xy = vec2(corner == 1 || corner == 2 ? -1.0 : 1.0, corner >= 2 ? 1.0 : -1.0);\n\
        gl_Position = vec4(xy, 0.5, 1.0);\n\
        vertexColor = vec4(vec3(face + 128) / 255.0, 1.0);\n    infoColor = CloudColor;\n}\n";
const PROBE_FSH: &str = "#version 330\nlayout(location = 0) in vec4 vertexColor;\n\
    layout(location = 1) in vec4 infoColor;\nlayout(location = 0) out vec4 fragColor;\n\
    void main() {\n    fragColor = gl_FragCoord.x < 4.0 ? infoColor : vertexColor;\n}\n";

fn probe(device: &wgpu::Device) -> CoreClouds {
    let program = crate::core_shader::translate_core_with(
        PROBE_VSH,
        PROBE_FSH,
        &[],
        &|_| None,
        &GLUE,
        &|b| b == INFO_BLOCK || b == FACES,
    )
    .expect("the probe translates");
    CoreClouds::new(device, wgpu::TextureFormat::Rgba8Unorm, &program).expect("the pipelines")
}

/// The faces `clouds::pack` builds reach the pack's shader through `CloudFaces`, texel by
/// texel, and `CloudInfo` through its block: `[-1, 0, -96]` plus 128 is 127, 128, 32, and
/// `CloudColor` (0.8, 1, 1, 1) is 204, 255, 255, 255. Flat clouds don't cull, so the quad's
/// winding doesn't matter; fancy ones cull its back.
#[test]
#[ignore = "daily"]
fn a_pack_clouds_shader_reads_the_faces_and_cloud_info() {
    let _gpu = crate::gfx::test_gpu::lock();
    let Some((device, queue)) = device() else {
        return;
    };
    let mut core = probe(&device);
    let mut filled = vec![false; 16];
    filled[3] = true;
    let texture = (4, 4, filled);
    let input = flat_input(&texture);
    assert_eq!(
        prepare(&device, &queue, &mut core, &[0; 1024], &input),
        Ok(1)
    );
    let pixels = draw(&device, &queue, &core, false);
    for (i, texel) in pixels.chunks(4).enumerate() {
        let want = if i % (SIZE as usize) < 4 {
            [204, 255, 255, 255]
        } else {
            [127, 128, 32, 255]
        };
        assert_eq!(texel, want, "texel {i}");
    }
    // The same quad as fancy clouds: back-facing here, culled.
    let culled = draw(&device, &queue, &core, true);
    assert!(culled.iter().all(|&b| b == 0));
}

/// A frame's faces past what the device binds to one storage buffer are refused, named by
/// their own size (not the buffer's rounded one); ones that fit, to the byte, grow the buffer.
#[test]
#[ignore = "daily"]
fn faces_past_the_devices_storage_limit_are_refused() {
    let _gpu = crate::gfx::test_gpu::lock();
    let Some((device, queue)) = device() else {
        return;
    };
    let mut core = probe(&device);
    // Every cell filled, range 128: about 92 thousand cells of up to 12 faces.
    let texture = (256, 256, vec![true; 256 * 256]);
    let mut input = flat_input(&texture);
    input.fast = false;
    input.range = 128;
    let faces = crate::clouds::pack::faces(&input).len() as u64;
    let bytes = faces * 4;
    let limit = device.limits().max_storage_buffer_binding_size;
    assert_eq!(limit, 128 << 20, "wgpu's default limit");
    assert!(bytes <= limit, "{bytes} bytes fit");
    assert!(prepare(&device, &queue, &mut core, &[0; 1024], &input).is_ok());
    // Devices binding exactly the faces' bytes, and four bytes less: the first takes them, the
    // second refuses them, naming both sizes.
    let Some(adapter) = crate::gfx::test_gpu::adapter() else {
        return;
    };
    let binding = |limit: u64| {
        pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            required_limits: wgpu::Limits {
                max_storage_buffer_binding_size: limit,
                ..Default::default()
            },
            ..Default::default()
        }))
        .expect("a device binding less")
    };
    let (exact, exact_queue) = binding(bytes);
    let mut core = probe(&exact);
    assert!(prepare(&exact, &exact_queue, &mut core, &[0; 1024], &input).is_ok());
    let (small, small_queue) = binding(bytes - 4);
    let mut core = probe(&small);
    assert_eq!(
        prepare(&small, &small_queue, &mut core, &[0; 1024], &input),
        Err(TooManyFaces {
            bytes,
            limit: bytes - 4
        })
    );
}

fn resources() -> std::path::PathBuf {
    std::env::var_os("SUNRISE_RESOURCES")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../.local/sunrise-client")
                .join(sunrise_core::VANILLA_VERSION)
        })
}

/// Vanilla's own `clouds.vsh` and `clouds.fsh` through the pack path, one fancy cell above the
/// camera, which looks straight up at its bottom from its middle: the face `clouds.vsh` colours
/// with `faceColors[0]` (0.7) times `CloudColor` (0.8, 204 of 255), 0.56 or byte 142.8, so 143,
/// opaque (fog that far out is nothing). The cell's inward faces, wound the other way, don't
/// cover it: fancy clouds cull back faces.
#[test]
#[ignore = "daily"]
fn vanillas_clouds_shaders_draw_a_cell_above_the_camera() {
    let _gpu = crate::gfx::test_gpu::lock();
    let root = crate::resource_source::ResourceSource::directory(resources());
    let shader = |name: &str| root.read_to_string(&format!("assets/minecraft/shaders/core/{name}"));
    let (Ok(vertex), Ok(fragment)) = (shader("clouds.vsh"), shader("clouds.fsh")) else {
        eprintln!(
            "skipped: no prepared resources at {}",
            resources().display()
        );
        return;
    };
    let include = |id: &str| {
        let (namespace, path) = id.split_once(':')?;
        root.read_to_string(&format!("assets/{namespace}/{path}"))
            .ok()
    };
    let program =
        crate::core_shader::translate_core_with(&vertex, &fragment, &[], &include, &GLUE, &|b| {
            b == INFO_BLOCK || b == FACES
        })
        .expect("vanilla's clouds shaders translate");
    let Some((device, queue)) = device() else {
        return;
    };
    let mut core =
        CoreClouds::new(&device, wgpu::TextureFormat::Rgba8Unorm, &program).expect("pipelines");
    let mut filled = vec![false; 16];
    filled[0] = true;
    let texture = (4, 4, filled);
    let input = crate::clouds::Input {
        texture: &texture,
        uv: [0.0; 2],
        // The middle of cell 0, 0 (z is offset by 3.96), 10 below the layer.
        camera: glam::DVec3::new(
            6.0,
            f64::from(crate::clouds::CLOUD_HEIGHT) - 10.0,
            6.0 - 3.96,
        ),
        time: 0,
        partial_tick: 0.0,
        range: 1,
        fast: false,
        color: [0.8, 0.8, 0.8, 1.0],
        height: crate::clouds::CLOUD_HEIGHT,
    };
    let view = crate::gfx::View::screen(0.0, -90.0, 70.0, 1.0, 1024.0);
    let far = 1.0e9;
    let vanilla = crate::gfx::core_lines::uniforms(
        [0; 48],
        &view,
        glam::Mat4::IDENTITY,
        glam::Mat4::IDENTITY,
        1.0,
        [0.0; 4],
        [far; 6],
    );
    assert!(prepare(&device, &queue, &mut core, &vanilla, &input).is_ok());
    let pixels = draw(&device, &queue, &core, true);
    let middle = ((SIZE / 2) * SIZE + SIZE / 2) as usize * 4;
    assert_eq!(pixels[middle..middle + 4], [143, 143, 143, 255]);
}

/// A replacement shader that doesn't read `CloudFaces`: drawn all the same, one quad a face,
/// placed by `gl_VertexIndex` alone (here every quad covers the target in one colour, 51, 102,
/// 153 of 255).
#[test]
#[ignore = "daily"]
fn a_clouds_shader_that_reads_no_faces_still_draws_a_quad_a_face() {
    let _gpu = crate::gfx::test_gpu::lock();
    let Some((device, queue)) = device() else {
        return;
    };
    let vsh = "#version 330\nvoid main() {\n    int corner = gl_VertexIndex % 4;\n\
        vec2 xy = vec2(corner == 1 || corner == 2 ? 1.0 : -1.0, corner >= 2 ? 1.0 : -1.0);\n\
        gl_Position = vec4(xy, 0.5, 1.0);\n}\n";
    let fsh = "#version 330\nlayout(location = 0) out vec4 fragColor;\n\
        void main() {\n    fragColor = vec4(0.2, 0.4, 0.6, 1.0);\n}\n";
    let program = crate::core_shader::translate_core_with(vsh, fsh, &[], &|_| None, &GLUE, &|b| {
        b == INFO_BLOCK || b == FACES
    })
    .expect("the shaders translate");
    assert!(program.other.is_empty(), "reads neither block");
    let mut core =
        CoreClouds::new(&device, wgpu::TextureFormat::Rgba8Unorm, &program).expect("pipelines");
    let mut filled = vec![false; 16];
    filled[3] = true;
    let texture = (4, 4, filled);
    assert_eq!(
        prepare(
            &device,
            &queue,
            &mut core,
            &[0; 1024],
            &flat_input(&texture)
        ),
        Ok(1)
    );
    for texel in draw(&device, &queue, &core, false).chunks(4) {
        assert_eq!(texel, [51, 102, 153, 255]);
    }
}
