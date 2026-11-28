//! A buffer that isn't cleared and ended a frame on B starts the next one there, instead of being
//! copied back to A, only when the first program to write or flip its other side replaces it
//! whole (#6858). Each fixture accumulates into colortex2 over frames, so a frame that started
//! on the older side would show it.
use super::destination_tests::Fixture;
use std::io::{Cursor, Write};
use sunrise_shaders::Post;

const VERTEX: &str = "#version 130\nvoid main() { gl_Position = ftransform(); }\n";

/// The quad's right half only: pixel columns 8 to 15 of 16.
const RIGHT_HALF: &str = "#version 130\nvoid main() { gl_Position = ftransform(); gl_Position.x = gl_Position.x * 0.5 + 0.5; }\n";

const COLORTEX2: u16 = 1 << 2;

/// colortex2 plus `add` units of 1/255, at the pixel being drawn.
fn adding(add: u32, extra: &str) -> String {
    format!(
        "#version 130\nuniform sampler2D colortex2;\n/* RENDERTARGETS: 2 */\n\
         /*\nconst int colortex2Format = RGBA8;\n*/\nconst bool colortex2Clear = false;\n\
         void main() {{\n{extra}    vec4 c = texelFetch(colortex2, ivec2(gl_FragCoord.xy), 0);\n\
         gl_FragData[0] = vec4(c.rgb + {add}.0 / 255.0, 1.0);\n}}\n"
    )
}

/// A pack of `programs` (name, vertex, fragment) and `properties`, its `final` showing colortex2.
fn pack(programs: &[(&str, &str, String)], properties: &str) -> Post {
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let mut add = |name: &str, source: &str| {
        zip.start_file(
            format!("shaders/{name}"),
            zip::write::SimpleFileOptions::default(),
        )
        .unwrap();
        zip.write_all(source.as_bytes()).unwrap();
    };
    for (name, vertex, fragment) in programs {
        add(&format!("{name}.vsh"), vertex);
        add(&format!("{name}.fsh"), fragment);
    }
    add("final.vsh", VERTEX);
    add(
        "final.fsh",
        "#version 130\nuniform sampler2D colortex2;\n\
         void main() { gl_FragColor = texelFetch(colortex2, ivec2(gl_FragCoord.xy), 0); }\n",
    );
    add("shaders.properties", properties);
    let pack = sunrise_shaders::Pack::from_zip_bytes(
        "frame-start-swap",
        zip.finish().unwrap().into_inner(),
    )
    .unwrap();
    let mut macros = sunrise_shaders::Macros::standard();
    let (properties, _) = sunrise_shaders::read_properties(&pack, &mut macros);
    sunrise_shaders::load_post(&pack, "", &properties, &macros, &Default::default())
}

/// Three programs adding 8, 16 and 32 to colortex2: three flips a frame, so every frame ends on
/// B. The first one draws `first` as its vertex stage, with `extra` at the start of its
/// fragment stage's `main`.
fn accumulating(first: &str, extra: &str, properties: &str) -> Post {
    pack(
        &[
            ("composite", first, adding(8, extra)),
            ("composite1", VERTEX, adding(16, "")),
            ("composite2", VERTEX, adding(32, "")),
        ],
        properties,
    )
}

/// The red channel of each pixel in a row, for `frames` frames at 16×16.
fn frames(gpu: &Fixture, post: &mut super::PackPost, frames: usize) -> Vec<Vec<u8>> {
    (0..frames)
        .map(|_| {
            let pixels = gpu.render(post, [16, 16]);
            // Rows are alike: the first one.
            assert!(
                pixels.chunks(16).all(|row| row == &pixels[..16]),
                "{pixels:?}"
            );
            pixels[..16].iter().map(|p| p[0]).collect()
        })
        .collect()
}

/// A row of 8 pixels of `left` then 8 of `right`.
fn row(left: u8, right: u8) -> Vec<u8> {
    [[left; 8], [right; 8]].concat()
}

/// Each frame adds 8 + 16 + 32 = 56 everywhere. The first program replaces colortex2 whole, so
/// frames start on B, and draw exactly as with the copy.
#[test]
#[ignore = "daily"]
fn a_buffer_whose_first_write_covers_it_swaps_and_draws_as_with_the_copy() {
    let _gpu = crate::gfx::test_gpu::lock();
    let Some(gpu) = Fixture::new() else { return };
    let mut swapped = gpu.build(accumulating(VERTEX, "", ""));
    assert_eq!(swapped.swaps & COLORTEX2, COLORTEX2);
    let mut copied = gpu.build(accumulating(VERTEX, "", ""));
    copied.swaps = 0;
    let expected: Vec<Vec<u8>> = [56, 112, 168, 224].map(|v| row(v, v)).into();
    assert_eq!(frames(&gpu, &mut swapped, 4), expected);
    assert_eq!(frames(&gpu, &mut copied, 4), expected);
}

/// The first program draws the right half only. Frame 1: the left half gets 16 + 32, the right
/// 56. Frame 2, from the copy: 48 + 48 on the left, 56 + 56 on the right. Started on B, the left
/// half would read frame 1's side of 16 instead (16 + 48).
#[test]
#[ignore = "daily"]
fn a_buffer_whose_first_write_leaves_pixels_out_keeps_the_copy() {
    let _gpu = crate::gfx::test_gpu::lock();
    let Some(gpu) = Fixture::new() else { return };
    let mut post = gpu.build(accumulating(RIGHT_HALF, "", ""));
    assert_eq!(post.swaps & COLORTEX2, 0);
    let expected = vec![row(48, 56), row(96, 112), row(144, 168)];
    assert_eq!(frames(&gpu, &mut post, 3), expected);
    // The fixture shows the difference: forced to start on B, frame 2 differs.
    let mut forced = gpu.build(accumulating(RIGHT_HALF, "", ""));
    forced.swaps |= COLORTEX2;
    assert_eq!(
        frames(&gpu, &mut forced, 2),
        vec![row(48, 56), row(64, 112)]
    );
}

/// The full fixture with one change that fails a check: colortex2 keeps the copy.
fn keeps_the_copy(post: Post) {
    let _gpu = crate::gfx::test_gpu::lock();
    let Some(gpu) = Fixture::new() else { return };
    let post = gpu.try_build(post).expect("the pack runs");
    assert_eq!(post.swaps & COLORTEX2, 0, "{:?}", post.notes);
}

#[test]
#[ignore = "daily"]
fn a_flip_without_a_write_before_the_first_write_keeps_the_copy() {
    let flips_first = pack(
        &[
            (
                "prepare",
                VERTEX,
                "#version 130\n/* RENDERTARGETS: 3 */\nvoid main() { gl_FragData[0] = vec4(1.0); }\n"
                    .into(),
            ),
            ("composite", VERTEX, adding(8, "")),
            ("composite1", VERTEX, adding(16, "")),
            ("composite2", VERTEX, adding(32, "")),
        ],
        "flip.prepare.colortex2=true\n",
    );
    keeps_the_copy(flips_first);
}

#[test]
#[ignore = "daily"]
fn a_pre_flip_keeps_the_copy() {
    keeps_the_copy(accumulating(
        VERTEX,
        "",
        "flip.composite_pre.colortex2=true\n",
    ));
}

#[test]
#[ignore = "daily"]
fn a_blended_first_write_keeps_the_copy() {
    keeps_the_copy(accumulating(
        VERTEX,
        "",
        "blend.composite=ONE ONE ONE ONE\n",
    ));
}

#[test]
#[ignore = "daily"]
fn a_discard_in_the_first_write_keeps_the_copy() {
    keeps_the_copy(accumulating(
        VERTEX,
        "    if (gl_FragCoord.x < 0.0) discard;\n",
        "",
    ));
}

#[test]
#[ignore = "daily"]
fn a_scaled_first_write_keeps_the_copy() {
    keeps_the_copy(accumulating(VERTEX, "", "scale.composite=0.5\n"));
}

/// A three-channel buffer's passes write colour only; its alpha is 1 on both sides, so it
/// swaps, unless an image can write that alpha.
#[test]
#[ignore = "daily"]
fn a_three_channel_buffer_swaps_unless_an_image_binds_it() {
    let _gpu = crate::gfx::test_gpu::lock();
    let Some(gpu) = Fixture::new() else { return };
    let rgb = |image: Option<&str>| {
        let rgb8 = |add| adding(add, "").replace("RGBA8", "RGB8");
        let mut programs = vec![
            ("composite", VERTEX, rgb8(8)),
            ("composite1", VERTEX, rgb8(16)),
            ("composite2", VERTEX, rgb8(32)),
        ];
        programs.extend(image.map(|fragment| ("composite3", VERTEX, fragment.to_owned())));
        gpu.try_build(pack(&programs, "")).expect("the pack runs")
    };
    let mut plain = rgb(None);
    assert_ne!(plain.rgb & COLORTEX2, 0, "{:?}", plain.notes);
    assert_eq!(plain.swaps & COLORTEX2, COLORTEX2, "{:?}", plain.notes);
    let mut copied = rgb(None);
    copied.swaps = 0;
    let expected: Vec<Vec<u8>> = [56, 112, 168].map(|v| row(v, v)).into();
    assert_eq!(frames(&gpu, &mut plain, 3), expected);
    assert_eq!(frames(&gpu, &mut copied, 3), expected);
    let bound = rgb(Some(
        "#version 430\nlayout(rgba8) writeonly uniform image2D colorimg2;\n/* RENDERTARGETS: 4 */\n\
         layout(location = 0) out vec4 o;\n\
         void main() { imageStore(colorimg2, ivec2(0), vec4(0.5)); o = vec4(1.0); }\n",
    ));
    assert_ne!(bound.storage & COLORTEX2, 0, "{:?}", bound.notes);
    assert_eq!(bound.swaps & COLORTEX2, 0, "{:?}", bound.notes);
}
