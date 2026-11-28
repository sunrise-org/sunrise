//! Sunrise's display uniforms (#5827 step 3) on the GPU: a program writes each one it reads
//! into the alpha of a pixel of its own (alpha is the one channel the served encode leaves as
//! written). Served, it reads the `Display` each frame is run with, ahead of a mod's uniform of
//! the same name, in `final` and in a cached shadow pass's `shadowcomp` and compute stage; not
//! served, it reads what it read before (its initializer, or 0), or a custom or mod uniform of
//! that name. Every value here is a half float, so the frame stores it exactly.
use super::destination_tests::{Fixture, fixture_frame};
use super::hdr_extension_tests::{texels, texture};
use super::*;
use crate::gfx::LINEAR_FRAME;
use crate::hud::HudBatch;
use std::io::{Cursor, Write};
use sunrise_render_shaderpack::pack_uniforms::Display;
use sunrise_shaders::{Post, Tonemap};
use wgpu::TextureFormat as T;

const VERTEX: &str = "#version 120\nvoid main() { gl_Position = ftransform(); }\n";

/// Pixel x of the frame (5 wide) gets the x-th display uniform in its alpha.
const FINAL: &str = "#version 120\n\
    uniform int sunriseHdr;\n\
    uniform float sunriseHeadroom;\n\
    uniform float sunrisePaperWhite;\n\
    uniform float sunrisePeak;\n\
    uniform float sunriseBlackLevel;\n\
    void main() {\n\
        float x = floor(gl_FragCoord.x);\n\
        float v = x == 0.0 ? float(sunriseHdr) : x == 1.0 ? sunriseHeadroom\n\
            : x == 2.0 ? sunrisePaperWhite : x == 3.0 ? sunrisePeak : sunriseBlackLevel;\n\
        gl_FragColor = vec4(0.0, 0.0, 0.0, v);\n\
    }\n";

/// The shadow pass's `shadowcomp` writes `sunriseHeadroom` (0.375 where nothing binds it) into
/// shadowcolor0, and its compute stage stores `sunrisePeak` (0.625 so) into shadowcolor1; `final`
/// shows shadowcolor0's in pixel 0's alpha and shadowcolor1's in pixel 1's.
const SHADOW: [(&str, &str); 4] = [
    (
        "shadowcomp.fsh",
        "#version 120\n/* RENDERTARGETS:0 */\n\
         uniform float sunriseHeadroom = 0.375;\n\
         void main() { gl_FragData[0] = vec4(sunriseHeadroom); }\n",
    ),
    ("shadowcomp.vsh", VERTEX),
    (
        "shadowcomp.csh",
        "#version 430\nlayout(local_size_x = 1, local_size_y = 1) in;\n\
         const ivec3 workGroups = ivec3(1, 1, 1);\n\
         layout(rgba16f) writeonly uniform image2D shadowcolorimg1;\n\
         uniform float sunrisePeak = 0.625;\n\
         void main() { imageStore(shadowcolorimg1, ivec2(0), vec4(sunrisePeak)); }\n",
    ),
    (
        "final.fsh",
        "#version 330 compatibility\n\
         /*\nconst int shadowcolor0Format = RGBA16F;\nconst int shadowcolor1Format = RGBA16F;\n*/\n\
         const int shadowMapResolution = 4;\n\
         uniform sampler2D shadowcolor0;\nuniform sampler2D shadowcolor1;\n\
         void main() {\n\
             float a = gl_FragCoord.x < 1.0 ? texelFetch(shadowcolor0, ivec2(0), 0).a\n\
                 : texelFetch(shadowcolor1, ivec2(0), 0).a;\n\
             gl_FragColor = vec4(0.0, 0.0, 0.0, a);\n\
         }\n",
    ),
];

/// A display whose five values all differ, none of them SDR's.
const HDR: Display = Display {
    hdr: true,
    headroom: 4.5,
    paper_white: 250.0,
    peak: 1125.0,
    black_level: 0.25,
};

/// The pack of `files` (under `shaders/`; `final.vsh` is `VERTEX`), loaded as the loader loads
/// it with Sunrise's HDR served as `hdr` (the macros after the properties, and
/// `GameInputs::hdr`).
fn pack(files: &[(&str, &str)], hdr: Option<Tonemap>) -> Post {
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for (file, text) in files.iter().chain(&[("final.vsh", VERTEX)]) {
        zip.start_file(
            format!("shaders/{file}"),
            zip::write::SimpleFileOptions::default(),
        )
        .unwrap();
        zip.write_all(text.as_bytes()).unwrap();
    }
    let pack = sunrise_shaders::Pack::from_zip_bytes(
        "display-uniforms",
        zip.finish().unwrap().into_inner(),
    )
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
    sunrise_shaders::load_post(&pack, "", &properties, &macros, &inputs)
}

/// The `final` pack, with `properties`.
fn final_pack(properties: &str, hdr: Option<Tonemap>) -> Post {
    pack(
        &[("final.fsh", FINAL), ("shaders.properties", properties)],
        hdr,
    )
}

/// A mod's float uniform named `sunriseHeadroom`.
fn mod_headroom() -> Vec<ModUniformDecl> {
    vec![ModUniformDecl {
        name: "sunriseHeadroom".into(),
        ty: ModUniformType::Float,
        previous_name: None,
    }]
}

/// `post` built for the float frame, with the mod uniforms `mods`.
fn build(gpu: &Fixture, post: Post, mods: &[ModUniformDecl]) -> PackPost {
    let lightmap = texture(gpu, [1, 1], T::Rgba8Unorm).create_view(&Default::default());
    PackPost::new_with_mods(
        (&gpu.device, &gpu.queue, &gpu.adapter),
        post,
        LINEAR_FRAME,
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
        ModPackRuntime {
            uniforms: mods,
            ..Default::default()
        },
        &std::sync::atomic::AtomicBool::new(false),
    )
    .expect("the pack runs")
}

/// The alphas of the first `N` pixels of one frame of `post` (N×1, shadow cache on), run with
/// `display`, a mod's `sunriseHeadroom` set to 7.
fn frame<const N: usize>(gpu: &Fixture, post: &mut PackPost, display: Display) -> [f32; N] {
    post.set_mod_uniform("sunriseHeadroom", ModUniformValue::Float(7.0));
    let size = [N as u32, 1];
    let screen = texture(gpu, size, LINEAR_FRAME);
    let world = texture(gpu, size, T::Rgba8Unorm).create_view(&Default::default());
    let depth = texture(gpu, size, T::Depth32Float).create_view(&Default::default());
    let hud = HudBatch::scratch(size);
    let mut encoder = gpu.device.create_command_encoder(&Default::default());
    post.run(
        &gpu.device,
        &gpu.queue,
        &mut encoder,
        &world,
        &depth,
        &screen.create_view(&Default::default()),
        &crate::gfx::Frame {
            shadow_cache: true,
            ..fixture_frame(&hud)
        },
        &Default::default(),
        display,
        size,
        size,
        &[],
        &mut |_, _, _, _| {},
        Default::default(),
        |_, _| false,
        &std::sync::atomic::AtomicBool::new(false),
        super::super::in_flight::poll_live,
    )
    .unwrap();
    let got = texels(gpu, encoder, &screen);
    std::array::from_fn(|x| got[x][3])
}

/// Served, a program reads the display each frame is run with, each uniform its own field: the
/// HDR values, then on the next frame of the same pack SDR's 0, 1, 80, 80 and 0 (`sunriseHdr`
/// read as an int, the others as floats).
#[test]
#[ignore = "daily"]
fn a_served_pack_reads_the_display_it_is_run_with() {
    let _gpu = crate::gfx::test_gpu::lock();
    let Some(gpu) = Fixture::new() else { return };
    for tonemap in [Tonemap::Pack, Tonemap::Sunrise] {
        let mut post = build(&gpu, final_pack("", Some(tonemap)), &[]);
        assert_eq!(
            frame(&gpu, &mut post, HDR),
            [1.0, 4.5, 250.0, 1125.0, 0.25],
            "{tonemap:?}"
        );
        assert_eq!(
            frame(&gpu, &mut post, Display::SDR),
            [0.0, 1.0, 80.0, 80.0, 0.0],
            "{tonemap:?}"
        );
    }
}

/// Not served, a program that declares the display uniforms reads none of them, whatever the
/// display: 0, as before the extension. A custom uniform of the same name binds there; served,
/// the display's value does.
#[test]
#[ignore = "daily"]
fn a_pack_not_served_reads_no_display_uniform() {
    let _gpu = crate::gfx::test_gpu::lock();
    let Some(gpu) = Fixture::new() else { return };
    let mut plain = build(&gpu, final_pack("", None), &[]);
    assert_eq!(frame(&gpu, &mut plain, HDR), [0.0; 5]);
    let custom = "uniform.float.sunriseHeadroom = 0.75\nuniform.float.sunrisePeak = 2.5\n";
    let mut unserved = build(&gpu, final_pack(custom, None), &[]);
    assert_eq!(frame(&gpu, &mut unserved, HDR), [0.0, 0.75, 0.0, 2.5, 0.0]);
    let mut served = build(&gpu, final_pack(custom, Some(Tonemap::Pack)), &[]);
    assert_eq!(
        frame(&gpu, &mut served, HDR),
        [1.0, 4.5, 250.0, 1125.0, 0.25]
    );
}

/// A mod's uniform named `sunriseHeadroom` (set to 7) binds where the pack isn't served, as
/// before the extension; served, the display's headroom binds instead.
#[test]
#[ignore = "daily"]
fn a_served_display_uniform_outranks_a_mods_of_its_name() {
    let _gpu = crate::gfx::test_gpu::lock();
    let Some(gpu) = Fixture::new() else { return };
    let mut unserved = build(&gpu, final_pack("", None), &mod_headroom());
    assert_eq!(frame(&gpu, &mut unserved, HDR), [0.0, 7.0, 0.0, 0.0, 0.0]);
    let mut served = build(&gpu, final_pack("", Some(Tonemap::Pack)), &mod_headroom());
    assert_eq!(
        frame(&gpu, &mut served, HDR),
        [1.0, 4.5, 250.0, 1125.0, 0.25]
    );
}

/// The shadow pass under its cache, frame after frame on one pack: served, `shadowcomp` and its
/// compute stage read the display's headroom and peak, ahead of a mod's `sunriseHeadroom`; the
/// next frame with the same display keeps the cached pass; a frame whose display alone changes
/// draws it again with the new values. Not served, the compute stage reads its initializer and
/// `shadowcomp` the mod's uniform, or its own initializer without a mod.
#[test]
#[ignore = "daily"]
fn a_cached_shadow_pass_follows_the_display() {
    let _gpu = crate::gfx::test_gpu::lock();
    let Some(gpu) = Fixture::new() else { return };
    let mut served = build(&gpu, pack(&SHADOW, Some(Tonemap::Pack)), &mod_headroom());
    assert_eq!(frame(&gpu, &mut served, HDR), [4.5, 1125.0]);
    let kept = served.shadow_cache.key.expect("the shadow pass is cached");
    assert_eq!(frame(&gpu, &mut served, HDR), [4.5, 1125.0]);
    assert_eq!(
        served.shadow_cache.key,
        Some(kept),
        "the same display keeps it"
    );
    let dimmer = Display {
        headroom: 2.0,
        peak: 500.0,
        ..HDR
    };
    assert_eq!(frame(&gpu, &mut served, dimmer), [2.0, 500.0]);
    assert_ne!(
        served.shadow_cache.key,
        Some(kept),
        "a new display draws it again"
    );
    let mut unserved = build(&gpu, pack(&SHADOW, None), &mod_headroom());
    assert_eq!(frame(&gpu, &mut unserved, HDR), [7.0, 0.625]);
    let mut plain = build(&gpu, pack(&SHADOW, None), &[]);
    assert_eq!(frame(&gpu, &mut plain, HDR), [0.375, 0.625]);
}
