//! Sunrise's HDR extension served to a `shaders/` pack (#5827 step 2): its `final` writes linear
//! light, which reaches the frame in the frame's gamma, once, whatever form its output 0 takes;
//! without `final` the colortex0 copy does the same; not served, the frame is the pack's picture
//! as before. Expected values are the 2.2 power worked out apart from the shader and rounded as
//! the frame stores them: 3.0 is 1687/1024 in half float, 5.0 is 1064/512, 0.1 is 1438/4096,
//! and 0.5 is 186 of 255. Each lies under half a step above its half float, so a store that
//! rounds to nearest and one that rounds toward zero (Vulkan allows either) keep the same.
use super::destination_tests::{Fixture, fixture_frame};
use super::*;
use crate::gfx::LINEAR_FRAME;
use crate::gfx::hdr::{Hdr, Output};
use crate::hud::HudBatch;
use std::io::{Cursor, Write};
use sunrise_shaders::{Post, Tonemap};
use wgpu::TextureFormat as T;

const VERTEX: &str = "#version 120\nvarying vec2 uv;\nvoid main() { uv=gl_MultiTexCoord0.xy; gl_Position=ftransform(); }\n";

/// 3.0 and 0.1 in the frame's gamma, as half floats store them.
const THREE: f32 = 1687.0 / 1024.0;
const TENTH: f32 = 1438.0 / 4096.0;

/// The pack of `files` (under `shaders/`, each `.fsh` with `VERTEX` beside it), loaded as the
/// loader loads it with Sunrise's HDR served as `hdr` (`serve_hdr`: the macros after the
/// properties, and `GameInputs::hdr`).
fn pack(files: &[(&str, &str)], hdr: Option<Tonemap>) -> Post {
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for (name, source) in files {
        for (file, text) in [
            (format!("shaders/{name}.fsh"), *source),
            (format!("shaders/{name}.vsh"), VERTEX),
        ] {
            zip.start_file(file, zip::write::SimpleFileOptions::default())
                .unwrap();
            zip.write_all(text.as_bytes()).unwrap();
        }
    }
    let pack =
        sunrise_shaders::Pack::from_zip_bytes("hdr-extension", zip.finish().unwrap().into_inner())
            .unwrap();
    let mut macros = sunrise_shaders::Macros::standard();
    let (properties, _) = sunrise_shaders::read_properties(&pack, &mut macros);
    if let Some(tonemap) = hdr {
        macros.define_sunrise_hdr(tonemap);
    }
    let inputs = sunrise_shaders::GameInputs {
        hdr,
        ..Default::default()
    };
    let post = sunrise_shaders::load_post(&pack, "", &properties, &macros, &inputs);
    assert_eq!(post.programs.len(), files.len(), "{:?}", post.notes);
    assert_eq!(post.hdr, hdr);
    post
}

/// A `final` writing `served` under `#ifdef SUNRISE_HDR`, else `plain` (its SDR path).
fn final_pack(served: &str, plain: &str, hdr: Option<Tonemap>) -> Post {
    let fsh = format!(
        "#version 120\nvoid main() {{\n#ifdef SUNRISE_HDR\n gl_FragColor = vec4({served});\n#else\n gl_FragColor = vec4({plain});\n#endif\n}}\n"
    );
    pack(&[("final", &fsh)], hdr)
}

pub(super) fn texture(gpu: &Fixture, size: [u32; 2], format: T) -> wgpu::Texture {
    gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("HDR extension fixture"),
        size: wgpu::Extent3d {
            width: size[0],
            height: size[1],
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    })
}

/// `post` built for a frame of `screen`'s format.
fn build(gpu: &Fixture, post: Post, screen: T) -> PackPost {
    let lightmap = texture(gpu, [1, 1], T::Rgba8Unorm).create_view(&Default::default());
    let post = PackPost::new(
        &gpu.device,
        &gpu.queue,
        &gpu.adapter,
        post,
        screen,
        World {
            atlas: None,
            material_maps: None,
            lightmap,
            terrain: None,
            block_ids: &[],
            sprite_rects: &[],
            celestial: None,
            gui_size: [16, 16],
            clouds: None,
            weather: false,
            hand: false,
        },
        &Default::default(),
    )
    .expect("the pack runs");
    assert!(post.notes.is_empty(), "{:?}", post.notes);
    post
}

/// One frame of `post` at the world's `size` onto `screen` (the window's size), in `encoder`.
fn run(
    gpu: &Fixture,
    post: &mut PackPost,
    encoder: &mut wgpu::CommandEncoder,
    screen: &wgpu::Texture,
    size: [u32; 2],
) {
    let window = [screen.width(), screen.height()];
    let world = texture(gpu, size, T::Rgba8Unorm).create_view(&Default::default());
    let depth = texture(gpu, size, T::Depth32Float).create_view(&Default::default());
    let hud = HudBatch::scratch(window);
    post.run(
        &gpu.device,
        &gpu.queue,
        encoder,
        &world,
        &depth,
        &screen.create_view(&Default::default()),
        &fixture_frame(&hud),
        &Default::default(),
        Default::default(),
        size,
        window,
        &[],
        &mut |_, _, _, _| {},
        Default::default(),
        |_, _| false,
        &std::sync::atomic::AtomicBool::new(false),
        super::super::in_flight::poll_live,
    )
    .unwrap();
}

/// Every texel of `texture` once `encoder` ran, as floats: half floats as stored, 8-bit
/// unorm values as their byte.
pub(super) fn texels(
    gpu: &Fixture,
    mut encoder: wgpu::CommandEncoder,
    texture: &wgpu::Texture,
) -> Vec<[f32; 4]> {
    let texel = texture.format().block_copy_size(None).unwrap();
    let stride = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    let read = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: u64::from(stride * texture.height()),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    encoder.copy_texture_to_buffer(
        texture.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &read,
            layout: wgpu::TexelCopyBufferLayout {
                bytes_per_row: Some(stride),
                ..Default::default()
            },
        },
        texture.size(),
    );
    gpu.queue.submit([encoder.finish()]);
    read.slice(..).map_async(wgpu::MapMode::Read, |_| {});
    gpu.device
        .poll(wgpu::PollType::wait_indefinitely())
        .unwrap();
    let bytes = read.slice(..).get_mapped_range().unwrap();
    bytes
        .chunks_exact(stride as usize)
        .flat_map(|row| row[..(texture.width() * texel) as usize].chunks_exact(texel as usize))
        .map(|t| match texture.format() {
            T::Rgba16Float => {
                std::array::from_fn(|c| half::f16::from_le_bytes([t[2 * c], t[2 * c + 1]]).to_f32())
            }
            T::Rgba8Unorm => std::array::from_fn(|c| f32::from(t[c])),
            other => panic!("no readback for {other:?}"),
        })
        .collect()
}

/// `post` drawn at `size` onto a frame of `format` at `window`: the frame's texels.
fn frame(gpu: &Fixture, post: Post, format: T, size: [u32; 2], window: [u32; 2]) -> Vec<[f32; 4]> {
    let mut post = build(gpu, post, format);
    let screen = texture(gpu, window, format);
    let mut encoder = gpu.device.create_command_encoder(&Default::default());
    run(gpu, &mut post, &mut encoder, &screen, size);
    texels(gpu, encoder, &screen)
}

/// Each texel of `frame` is `want`.
fn all(frame: &[[f32; 4]], want: [f32; 4]) -> bool {
    frame.iter().all(|t| *t == want)
}

/// (a) Served with the pack's curve, on the layered frame's format: linear 3.0 is held as
/// 3.0^(1/2.2), a negative or NaN channel as 0, infinity as half float's largest, alpha as
/// written. Every form a `final` writes output 0 with gets the same encode. The tone curve
/// that then doesn't run is `frame_policy`'s (`one_tone_curve_runs_on_the_layered_world`).
#[test]
#[ignore = "daily"]
fn a_served_final_writes_linear_light_in_the_frames_gamma() {
    let _gpu = crate::gfx::test_gpu::lock();
    let Some(gpu) = Fixture::new() else { return };
    let served = Some(Tonemap::Pack);
    let post = final_pack("3.0, 0.1, 0.0, 0.75", "0.25", served);
    assert!(all(
        &frame(&gpu, post, LINEAR_FRAME, [4, 4], [4, 4]),
        [THREE, TENTH, 0.0, 0.75]
    ));
    let post = final_pack(
        "intBitsToFloat(0x7fc00000), -2.0, intBitsToFloat(0x7f800000), 0.5",
        "0.25",
        served,
    );
    let got = frame(&gpu, post, LINEAR_FRAME, [4, 4], [4, 4]);
    assert!(all(&got, [0.0, 0.0, 65504.0, 0.5]), "{:?}", got[0]);
    for (form, fsh) in [
        (
            "gl_FragData",
            "#version 120\nvoid main() { gl_FragData[0] = vec4(3.0, 0.1, 0.0, 0.75); }\n",
        ),
        (
            "out",
            "#version 330\nout vec4 c;\nvoid main() { c = vec4(3.0, 0.1, 0.0, 0.75); }\n",
        ),
        (
            "location",
            "#version 330\nlayout(location = 0) out vec4 c;\nvoid main() { c = vec4(3.0, 0.1, 0.0, 0.75); }\n",
        ),
        (
            "array",
            "#version 330\nout vec4 c[2];\nvoid main() { c[0] = vec4(3.0, 0.1, 0.0, 0.75); c[1] = vec4(0.0); }\n",
        ),
        (
            "narrow",
            "#version 330\nlayout(location = 0) out vec3 c;\nvoid main() { c = vec3(3.0, 0.1, 0.0); }\n",
        ),
        (
            "components",
            "#version 440\nlayout(location = 0, component = 0) out vec2 rg;\n\
             layout(location = 0, component = 2) out vec2 ba;\n\
             void main() { rg = vec2(3.0, 0.1); ba = vec2(0.0, 0.75); }\n",
        ),
    ] {
        let alpha = if form == "narrow" { 1.0 } else { 0.75 };
        let got = frame(
            &gpu,
            pack(&[("final", fsh)], served),
            LINEAR_FRAME,
            [4, 4],
            [4, 4],
        );
        assert!(
            all(&got, [THREE, TENTH, 0.0, alpha]),
            "{form}: {:?}",
            got[0]
        );
    }
}

/// (b) Served with Sunrise's curve on the layered frame (paper white 203, peak 1000): after
/// `Hdr::tonemap` the frame holds what the curve makes of the same light stored by a frame
/// without a pack, with HDR Lighting off as under any pack; and the curve changed it (5.0 is
/// past its knee), so it ran there once.
#[test]
#[ignore = "daily"]
fn a_served_final_with_sunrises_curve_is_mapped_as_a_no_pack_frame() {
    let _gpu = crate::gfx::test_gpu::lock();
    let Some(gpu) = Fixture::new() else { return };
    let mut hdr = Hdr::new(&gpu.device, &gpu.queue, LINEAR_FRAME, Output::Pq, true);
    (hdr.paper_white, hdr.peak, hdr.lighting) = (203.0, 1000.0, false);
    let frame_texture = hdr.frame(&gpu.device, [4, 4]);
    let stored = 1064.0 / 512.0;
    // The pack's `final` writes linear 5.0, then the curve.
    let mut post = build(
        &gpu,
        final_pack("5.0, 5.0, 5.0, 1.0", "0.25", Some(Tonemap::Sunrise)),
        LINEAR_FRAME,
    );
    let mut encoder = gpu.device.create_command_encoder(&Default::default());
    run(&gpu, &mut post, &mut encoder, &frame_texture, [4, 4]);
    hdr.begin(&gpu.queue, true);
    hdr.tonemap(&mut encoder, None);
    let pack = texels(&gpu, encoder, &frame_texture);
    // The same light as a frame without a pack stores it, then the curve.
    let mut encoder = gpu.device.create_command_encoder(&Default::default());
    let view = frame_texture.create_view(&Default::default());
    encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view: &view,
            resolve_target: None,
            depth_slice: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Clear(wgpu::Color {
                    r: stored.into(),
                    g: stored.into(),
                    b: stored.into(),
                    a: 1.0,
                }),
                store: wgpu::StoreOp::Store,
            },
        })],
        ..Default::default()
    });
    hdr.begin(&gpu.queue, true);
    hdr.tonemap(&mut encoder, None);
    let native = texels(&gpu, encoder, &frame_texture);
    assert_eq!(pack, native);
    assert!(native[0][0] < stored, "{:?}", native[0]);
}

/// (c) Served on an SDR 8-bit frame: linear 0.5 lands at 0.5^(1/2.2), 186 of 255, and 2.0
/// clips at white.
#[test]
#[ignore = "daily"]
fn a_served_final_on_an_sdr_frame_clips_at_white() {
    let _gpu = crate::gfx::test_gpu::lock();
    let Some(gpu) = Fixture::new() else { return };
    let post = final_pack("0.5, 2.0, 0.0, 1.0", "0.25", Some(Tonemap::Pack));
    let got = frame(&gpu, post, T::Rgba8Unorm, [4, 4], [4, 4]);
    assert!(all(&got, [186.0, 255.0, 0.0, 255.0]), "{:?}", got[0]);
}

/// (d) Not served (the switch off, or no key: no macro, no `GameInputs::hdr`): the pack takes
/// its SDR path and the frame holds what it writes, as on main: 0.25 on the float frame and 64
/// of 255 on the 8-bit one; without `final`, colortex0's 3.0 as it is.
#[test]
#[ignore = "daily"]
fn a_pack_not_served_draws_its_own_picture() {
    let _gpu = crate::gfx::test_gpu::lock();
    let Some(gpu) = Fixture::new() else { return };
    let post = || final_pack("3.0, 3.0, 3.0, 1.0", "0.25, 0.25, 0.25, 1.0", None);
    assert!(all(
        &frame(&gpu, post(), LINEAR_FRAME, [4, 4], [4, 4]),
        [0.25, 0.25, 0.25, 1.0]
    ));
    assert!(all(
        &frame(&gpu, post(), T::Rgba8Unorm, [4, 4], [4, 4]),
        [64.0, 64.0, 64.0, 255.0]
    ));
    let got = frame(&gpu, colortex0_pack(None), LINEAR_FRAME, [4, 4], [4, 4]);
    assert!(
        all(&got, [3.0, 1638.0 / 16384.0, -2.0, 0.75]),
        "{:?}",
        got[0]
    );
}

/// A pack without `final` whose `composite` writes 3.0, 0.1, -2.0, 0.75 into a float colortex0.
fn colortex0_pack(hdr: Option<Tonemap>) -> Post {
    colortex0_writing("3.0, 0.1, -2.0, 0.75", hdr)
}

/// A pack without `final` whose `composite` writes `value` into a float colortex0.
fn colortex0_writing(value: &str, hdr: Option<Tonemap>) -> Post {
    let fsh = format!(
        "#version 330\n/*\nconst int colortex0Format = RGBA16F;\n*/\n/* RENDERTARGETS:0 */\n\
         out vec4 c;\nvoid main() {{ c = vec4({value}); }}\n"
    );
    pack(&[("composite", &fsh)], hdr)
}

/// Served without `final`, an `RGBA32F` colortex0 is copied on a device without
/// `FLOAT32_FILTERABLE`, where it can't be filtered: the served copies only load texels, so
/// their layout takes it, unscaled and with a render scale.
#[test]
#[ignore = "daily"]
fn an_unfilterable_colortex0_is_copied_served() {
    let _gpu = crate::gfx::test_gpu::lock();
    let Some(gpu) = Fixture::new() else { return };
    assert!(
        !gpu.device
            .features()
            .contains(wgpu::Features::FLOAT32_FILTERABLE)
    );
    let fsh = "#version 330\n/*\nconst int colortex0Format = RGBA32F;\n*/\n/* RENDERTARGETS:0 */\n\
               out vec4 c;\nvoid main() { c = vec4(3.0, 0.1, 0.0, 0.75); }\n";
    let post = || pack(&[("composite", fsh)], Some(Tonemap::Pack));
    assert_eq!(build(&gpu, post(), LINEAR_FRAME).formats[0], T::Rgba32Float);
    for window in [[4, 4], [8, 8]] {
        let got = frame(&gpu, post(), LINEAR_FRAME, [4, 4], window);
        assert!(
            all(&got, [THREE, TENTH, 0.0, 0.75]),
            "{window:?}: {:?}",
            got[0]
        );
    }
}

/// Served without `final`, the colortex0 copy stands for it: the same encode, a negative
/// channel 0, alpha as written; a NaN, an infinity and a negative infinity in colortex0 come out
/// 0, half float's largest and 0, whether the copy filters (a render scale) or not; alpha is
/// the centre texel's, never filtered.
#[test]
#[ignore = "daily"]
fn without_final_the_colortex0_copy_encodes_served_light() {
    let _gpu = crate::gfx::test_gpu::lock();
    let Some(gpu) = Fixture::new() else { return };
    for tonemap in [Tonemap::Pack, Tonemap::Sunrise] {
        let got = frame(
            &gpu,
            colortex0_pack(Some(tonemap)),
            LINEAR_FRAME,
            [4, 4],
            [4, 4],
        );
        assert!(
            all(&got, [THREE, TENTH, 0.0, 0.75]),
            "{tonemap:?}: {:?}",
            got[0]
        );
        let odd = colortex0_writing(
            "intBitsToFloat(0x7fc00000), intBitsToFloat(0x7f800000), -intBitsToFloat(0x7f800000), 0.5",
            Some(tonemap),
        );
        let got = frame(&gpu, odd.clone(), LINEAR_FRAME, [4, 4], [4, 4]);
        assert!(
            all(&got, [0.0, 65504.0, 0.0, 0.5]),
            "{tonemap:?}: {:?}",
            got[0]
        );
    }
    // With a render scale the copy filters colortex0 up to the window, from the four texels
    // around each pixel, each weighed above 0: the same light comes out, an infinity too.
    let scaled = frame(
        &gpu,
        colortex0_pack(Some(Tonemap::Pack)),
        LINEAR_FRAME,
        [4, 4],
        [8, 8],
    );
    assert!(all(&scaled, [THREE, TENTH, 0.0, 0.75]), "{:?}", scaled[0]);
    let odd = colortex0_writing(
        "intBitsToFloat(0x7fc00000), intBitsToFloat(0x7f800000), -intBitsToFloat(0x7f800000), 0.5",
        Some(Tonemap::Pack),
    );
    let scaled = frame(&gpu, odd, LINEAR_FRAME, [4, 4], [8, 8]);
    assert!(all(&scaled, [0.0, 65504.0, 0.0, 0.5]), "{:?}", scaled[0]);
    // Alpha isn't filtered: each window pixel has its centre texel's, here 1/8 to 4/8 across
    // colortex0's four columns, two window pixels each, while the light is filtered.
    let columns = colortex0_writing(
        "3.0, 0.1, 0.0, (floor(gl_FragCoord.x) + 1.0) / 8.0",
        Some(Tonemap::Pack),
    );
    let scaled = frame(&gpu, columns, LINEAR_FRAME, [4, 4], [8, 8]);
    for (n, texel) in scaled.iter().enumerate() {
        let alpha = ((n % 8 / 2) as f32 + 1.0) / 8.0;
        assert_eq!(*texel, [THREE, TENTH, 0.0, alpha], "pixel {n}");
    }
    // Without a render scale each pixel is its own texel: an infinity at x = 5 of a window 11
    // pixels wide (a width whose texel coordinates don't come out exact) stays in its pixel,
    // and its neighbours keep their own light.
    let lone = colortex0_writing(
        "floor(gl_FragCoord.x) == 5.0 ? intBitsToFloat(0x7f800000) : 0.1, 0.1, 0.1, 1.0",
        Some(Tonemap::Pack),
    );
    let got = frame(&gpu, lone, LINEAR_FRAME, [11, 4], [11, 4]);
    for (n, texel) in got.iter().enumerate() {
        let red = if n % 11 == 5 { 65504.0 } else { TENTH };
        assert_eq!(*texel, [red, TENTH, TENTH, 1.0], "pixel {n}");
    }
}

/// (e) A render scale: `final` draws at the world's size into a picture of the frame's float
/// format, and the upscale takes it to the window without encoding it again (3.0^(1/4.84)
/// would be 1.255) or clipping it (an 8-bit picture would hold 1.0), and without filtering its
/// alpha.
#[test]
#[ignore = "daily"]
fn a_render_scale_encodes_once_and_upscales_float() {
    let _gpu = crate::gfx::test_gpu::lock();
    let Some(gpu) = Fixture::new() else { return };
    let post = final_pack("3.0, 0.1, 0.0, 0.75", "0.25", Some(Tonemap::Pack));
    let got = frame(&gpu, post, LINEAR_FRAME, [4, 4], [8, 8]);
    assert_eq!(got.len(), 64);
    assert!(all(&got, [THREE, TENTH, 0.0, 0.75]), "{:?}", got[0]);
    // Its alpha isn't filtered: each window pixel has its centre texel's, 1/8 to 4/8 across the
    // picture's four columns, two window pixels each.
    let columns = final_pack(
        "3.0, 0.1, 0.0, (floor(gl_FragCoord.x) + 1.0) / 8.0",
        "0.25",
        Some(Tonemap::Pack),
    );
    let got = frame(&gpu, columns, LINEAR_FRAME, [4, 4], [8, 8]);
    for (n, texel) in got.iter().enumerate() {
        let alpha = ((n % 8 / 2) as f32 + 1.0) / 8.0;
        assert_eq!(*texel, [THREE, TENTH, 0.0, alpha], "pixel {n}");
    }
}
