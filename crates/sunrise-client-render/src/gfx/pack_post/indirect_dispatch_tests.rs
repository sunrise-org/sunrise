//! `indirect.<pass>` (spec 3.2): a compute shader runs as many work groups as an earlier pass
//! of the frame wrote into a storage buffer, at the byte offset the key gives. The packs are
//! original: `prepare.csh` writes the counts, `composite.csh` paints one pixel per work group,
//! green when `gl_NumWorkGroups` is the counts written, and `final` shows colortex0.
use super::destination_tests::Fixture;
use std::io::{Cursor, Write};
use sunrise_shaders::Post;

const FINAL_VSH: &str = "#version 120\nvarying vec2 uv;\nvoid main() { uv=gl_MultiTexCoord0.xy; gl_Position=ftransform(); }\n";
const FINAL_FSH: &str = "#version 330 compatibility\nuniform sampler2D colortex0;\nin vec2 uv;\n\
     void main() { gl_FragColor = texelFetch(colortex0, ivec2(gl_FragCoord.xy), 0); }\n";

/// `final` showing colortex0, and at the top-right pixel blue when word `at` of buffer 0 is
/// `value`, black otherwise.
fn final_with_counter(at: u32, value: u32) -> String {
    format!(
        "#version 430 compatibility\nuniform sampler2D colortex0;\n\
         layout(std430, binding=0) buffer Counts {{ uint words[]; }};\nin vec2 uv;\n\
         void main() {{\n    ivec2 p = ivec2(gl_FragCoord.xy);\n\
             gl_FragColor = p == ivec2(7, 7) ? vec4(0, 0, words[{at}] == {value}u ? 1 : 0, 1)\n\
                 : texelFetch(colortex0, p, 0);\n}}\n"
    )
}

/// What `composite.csh` does to the buffer it reads its counts from, besides painting.
struct Consume {
    /// The first of the three count words: group (0, 0, 0) sets them to zero.
    counts: u32,
    /// A word every invocation adds 1 to.
    counter: u32,
}

/// `composite.csh`: work group (x, y, z) paints pixel (x + 3y, z), green when the dispatch is
/// `expected` groups, red otherwise; with `consume`, it also writes the buffer it reads its
/// counts from.
fn composite(expected: [u32; 3], consume: Option<Consume>) -> String {
    let [x, y, z] = expected;
    let writes = consume.map_or_else(String::new, |c| {
        let k = c.counts;
        format!(
            "    atomicAdd(words[{}], 1u);\n\
                 if (g == uvec3(0u)) {{ words[{k}] = 0u; words[{}] = 0u; words[{}] = 0u; }}\n",
            c.counter,
            k + 1,
            k + 2
        )
    });
    format!(
        "#version 430\nlayout(local_size_x=1) in;\n\
         layout(rgba8) writeonly uniform image2D colorimg0;\n\
         layout(std430, binding=0) buffer Counts {{ uint words[]; }};\n\
         void main() {{\n    uvec3 g = gl_WorkGroupID;\n\
             bool ok = gl_NumWorkGroups == uvec3({x}u, {y}u, {z}u);\n\
             imageStore(colorimg0, ivec2(g.x + 3u * g.y, g.z), ok ? vec4(0, 1, 0, 1) : vec4(1, 0, 0, 1));\n\
         {writes}}}\n"
    )
}

/// `prepare.csh`: one invocation storing `words` from word `first` on, those the buffer holds;
/// with `once`, only while word `once` of the buffer is 0, which it then sets.
fn prepare(first: u32, words: &[u32], once: Option<u32>) -> String {
    let stores: String = words
        .iter()
        .enumerate()
        .map(|(k, w)| {
            let at = first as usize + k;
            format!("    if (words.length() > {at}) words[{at}] = {w}u;\n")
        })
        .collect();
    let guard = once.map_or_else(String::new, |f| {
        format!(
            "    if (words.length() <= {f} || words[{f}] != 0u) return;\n    words[{f}] = 1u;\n"
        )
    });
    format!(
        "#version 430\nlayout(local_size_x=1) in;\nconst ivec3 workGroups = ivec3(1, 1, 1);\n\
         layout(std430, binding=0) buffer Counts {{ uint words[]; }};\n\
         void main() {{\n{guard}{stores}}}\n"
    )
}

/// The pack of `files` (`shaders/` paths), loaded with no note.
fn load(name: &str, files: &[(&str, &[u8])]) -> Post {
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for (path, bytes) in files {
        zip.start_file(
            format!("shaders/{path}"),
            zip::write::SimpleFileOptions::default(),
        )
        .unwrap();
        zip.write_all(bytes).unwrap();
    }
    let pack =
        sunrise_shaders::Pack::from_zip_bytes(name, zip.finish().unwrap().into_inner()).unwrap();
    let mut macros = sunrise_shaders::Macros::standard();
    let (properties, _) = sunrise_shaders::read_properties(&pack, &mut macros);
    let post = sunrise_shaders::load_post(&pack, "", &properties, &macros, &Default::default());
    assert!(post.notes.is_empty(), "{:?}", post.notes);
    post
}

fn pack(properties: &str, prepare: &str, composite: &str) -> Post {
    pack_with_final(properties, prepare, composite, FINAL_FSH)
}

fn pack_with_final(properties: &str, prepare: &str, composite: &str, fragment: &str) -> Post {
    load(
        "original-indirect",
        &[
            ("shaders.properties", properties.as_bytes()),
            ("prepare.csh", prepare.as_bytes()),
            ("composite.csh", composite.as_bytes()),
            ("final.vsh", FINAL_VSH.as_bytes()),
            ("final.fsh", fragment.as_bytes()),
        ],
    )
}

const GREEN: [u8; 4] = [0, 255, 0, 255];
const RED: [u8; 4] = [255, 0, 0, 255];
const BLUE: [u8; 4] = [0, 0, 255, 255];

/// The pixels painted (green or red) in a frame of `width`, as (x, y) from the top-left
/// corner of the read-back, with their colour.
fn painted(pixels: &[[u8; 4]], width: usize) -> Vec<((usize, usize), [u8; 4])> {
    pixels
        .iter()
        .enumerate()
        .filter(|(_, p)| **p == GREEN || **p == RED)
        .map(|(k, p)| ((k % width, k / width), *p))
        .collect()
}

/// Pixels (x + 3y, z) for every group of a 3 x 2 x 4 dispatch, green, as `painted` lists them.
fn every_group_green(height: usize) -> Vec<((usize, usize), [u8; 4])> {
    let mut want: Vec<_> = (0..4)
        .flat_map(|z| (0..2).flat_map(move |y| (0..3).map(move |x| (x + 3 * y, z))))
        .map(|(x, z)| ((x, height - 1 - z), GREEN))
        .collect();
    want.sort_by_key(|((x, y), _)| (*y, *x));
    want
}

/// The issue's case: the counts sit after a word of something else (byte offset 4), and the
/// words around them differ from them, so a wrong offset or order paints red or elsewhere.
/// The shader then writes the buffer it read its counts from: it counts its invocations in
/// word 8 and sets the counts to zero, which `prepare` writes only once. On the same
/// `PackPost`, without wgpu's indirect validation (as release builds make it), the second
/// frame's in-range zero counts run nothing: no pixel, and the counter stays at 24.
#[test]
#[ignore = "daily"]
fn a_dispatch_runs_the_counts_an_earlier_pass_wrote_at_the_offset() {
    let _gpu = crate::gfx::test_gpu::lock();
    let Some(gpu) = Fixture::without_indirect_validation() else {
        return;
    };
    let post = pack_with_final(
        "bufferObject.0 = 64\nindirect.composite = 0 4\n",
        &prepare(0, &[99, 3, 2, 4, 7], Some(9)),
        &composite(
            [3, 2, 4],
            Some(Consume {
                counts: 1,
                counter: 8,
            }),
        ),
        &final_with_counter(8, 24),
    );
    let mut post = gpu.build(post);
    let first = gpu.render(&mut post, [8, 8]);
    assert_eq!(painted(&first, 8), every_group_green(8));
    assert_eq!(first[7], BLUE, "24 invocations");
    let second = gpu.render(&mut post, [8, 8]);
    assert_eq!(painted(&second, 8), Vec::new());
    assert_eq!(second[7], BLUE, "still 24 invocations");
}

/// A buffer sized by the render size holds the counts only once it is large enough, and is
/// made again when the size changes, or when the pack's history is reset at the same size:
/// the counts are read from the buffer made last. Each buffer gets its counts once, and the
/// shader zeroes them in the buffer it read them from, so counts read from a buffer made
/// before would run nothing.
#[test]
#[ignore = "daily"]
fn counts_past_a_relative_buffer_run_nothing_until_it_holds_them() {
    let _gpu = crate::gfx::test_gpu::lock();
    let Some(gpu) = Fixture::new() else { return };
    // 4 bytes per pixel: 64 words at 8 x 8, 256 at 16 x 16. Words 62 to 64 hold the counts.
    let post = pack(
        "bufferObject.0 = 4 true 1.0 1.0\nindirect.composite = 0 248\n",
        &prepare(62, &[3, 2, 4], Some(61)),
        &composite(
            [3, 2, 4],
            Some(Consume {
                counts: 62,
                counter: 60,
            }),
        ),
    );
    let mut post = gpu.build(post);
    let small = gpu.render(&mut post, [8, 8]);
    assert_eq!(painted(&small, 8), Vec::new());
    let large = gpu.render(&mut post, [16, 16]);
    assert_eq!(painted(&large, 16), every_group_green(16));
    let used = gpu.render(&mut post, [16, 16]);
    assert_eq!(painted(&used, 16), Vec::new(), "the counts were zeroed");
    post.reset_history(&gpu.device);
    let reset = gpu.render(&mut post, [16, 16]);
    assert_eq!(painted(&reset, 16), every_group_green(16), "after a reset");
}

/// Counts with one over the device's work groups per dimension run nothing, on an instance
/// without wgpu's own indirect validation, as release builds make it
/// (`gfx::device::instance_descriptor`).
#[test]
#[ignore = "daily"]
fn counts_over_the_device_limit_run_nothing() {
    let _gpu = crate::gfx::test_gpu::lock();
    let Some(gpu) = Fixture::without_indirect_validation() else {
        return;
    };
    let most = gpu.device.limits().max_compute_workgroups_per_dimension;
    assert!(most < u32::MAX, "no count is over {most}");
    for counts in [[most + 1, 1, 1], [1, most + 1, 1], [1, 1, most + 1]] {
        let post = pack(
            "bufferObject.0 = 64\nindirect.composite = 0 0\n",
            &prepare(0, &counts, None),
            &composite(counts, None),
        );
        let mut post = gpu.build(post);
        let pixels = gpu.render(&mut post, [8, 8]);
        assert_eq!(painted(&pixels, 8), Vec::new(), "{counts:?}");
    }
    // The same pack with counts at the limit runs.
    let counts = [most, 1, 1];
    let post = pack(
        "bufferObject.0 = 64\nindirect.composite = 0 0\n",
        &prepare(0, &counts, None),
        &composite(counts, None),
    );
    let mut post = gpu.build(post);
    let pixels = gpu.render(&mut post, [8, 8]);
    let row: Vec<_> = (0..8).map(|x| ((x, 7), GREEN)).collect();
    assert_eq!(painted(&pixels, 8), row);
}

/// A `shadowcomp` compute shader that reads its counts from a buffer, though it binds none,
/// runs what the buffer says each frame with the Shadow Cache on. Its counts start at 1 (from a
/// file) and `composite`, after the shadow pass, sets them to 2; nothing in the cache key
/// changes between the two frames. Group x stores red into shadowcolor0 texel (x, 0) when one
/// group runs, blue when two do, and `final` shows shadowcolor0.
#[test]
#[ignore = "daily"]
fn the_shadow_cache_reruns_a_shader_read_indirectly() {
    let _gpu = crate::gfx::test_gpu::lock();
    let Some(gpu) = Fixture::new() else { return };
    let counts: Vec<u8> = [1u32, 1, 1].iter().flat_map(|n| n.to_le_bytes()).collect();
    let shadowcomp = "#version 430\nlayout(local_size_x=1) in;\n\
         layout(rgba8) writeonly uniform image2D shadowcolorimg0;\n\
         void main() {\n    vec4 c = gl_NumWorkGroups.x == 1u ? vec4(1, 0, 0, 1) : vec4(0, 0, 1, 1);\n\
             imageStore(shadowcolorimg0, ivec2(gl_WorkGroupID.x, 0), c);\n}\n";
    let composite = "#version 430\nlayout(local_size_x=1) in;\nconst ivec3 workGroups = ivec3(1, 1, 1);\n\
         layout(std430, binding=0) buffer Counts { uint words[]; };\n\
         void main() { words[0] = 2u; }\n";
    let fragment = "#version 330 compatibility\nconst int shadowMapResolution = 8;\n\
         uniform sampler2D shadowcolor0;\n\
         void main() { gl_FragColor = texelFetch(shadowcolor0, ivec2(gl_FragCoord.xy), 0); }\n";
    let post = load(
        "original-indirect-shadow",
        &[
            (
                "shaders.properties",
                b"bufferObject.0 = 16 counts.bin\nindirect.shadowcomp = 0 0\n",
            ),
            ("counts.bin", &counts),
            ("shadowcomp.csh", shadowcomp.as_bytes()),
            ("composite.csh", composite.as_bytes()),
            ("final.vsh", FINAL_VSH.as_bytes()),
            ("final.fsh", fragment.as_bytes()),
        ],
    );
    assert_eq!(post.shadow_comp.len(), 1);
    let mut post = gpu.build(post);
    // Texels (0, 0) and (1, 0) are the bottom row's first two pixels.
    let first = gpu.render_cached(&mut post, [8, 8]);
    assert_eq!(first[56], RED);
    assert_ne!(first[57], BLUE);
    let second = gpu.render_cached(&mut post, [8, 8]);
    assert_eq!([second[56], second[57]], [BLUE, BLUE]);
}
