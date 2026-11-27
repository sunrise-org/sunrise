//! A fragment output at a location far past the buffers (#4912), however the pack writes it, is
//! not the size of the pass's list of colour targets: the program is refused by the device's
//! `max_color_attachments` when its pipeline is made, as any location past it is, and the rest
//! of the pack draws. The programs here are original.
use super::destination_tests::Fixture;
use std::io::{Cursor, Write};
use sunrise_shaders::Post;

const VERTEX: &str = "#version 120\nvoid main() { gl_Position=ftransform(); }\n";

/// The colour `composite1` writes, as 8-bit values over 255 (`64/255`, `128/255`, `191/255`
/// to nine digits) so that the `Rgba8Unorm` colortex0 and the `Rgba8Unorm` screen hold exactly
/// these bytes on every adapter: `[64, 128, 191, 255]` (see the assertion in the test).
const COMPOSITE1: &str = "#version 120\n/* DRAWBUFFERS:0 */\n\
     void main() { gl_FragData[0] = vec4(0.250980392, 0.501960784, 0.749019608, 1.0); }\n";

/// A pack whose `composite` fragment stage is `fragment`, whose `composite1` writes a known
/// colour to colortex0, and whose `final` shows colortex0.
fn pack(fragment: &str) -> Post {
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let mut add = |name: &str, source: &str| {
        zip.start_file(
            format!("shaders/{name}"),
            zip::write::SimpleFileOptions::default(),
        )
        .unwrap();
        zip.write_all(source.as_bytes()).unwrap();
    };
    add("composite.vsh", VERTEX);
    add("composite.fsh", fragment);
    add("composite1.vsh", VERTEX);
    add("composite1.fsh", COMPOSITE1);
    add(
        "final.vsh",
        "#version 120\nvarying vec2 uv;\nvoid main() { uv=gl_MultiTexCoord0.xy; gl_Position=ftransform(); }\n",
    );
    add(
        "final.fsh",
        "#version 120\nuniform sampler2D colortex0;\nvarying vec2 uv;\n\
         void main() { gl_FragColor=texture2D(colortex0,uv); }\n",
    );
    let pack = sunrise_shaders::Pack::from_zip_bytes(
        "original-far-location",
        zip.finish().unwrap().into_inner(),
    )
    .unwrap();
    let mut macros = sunrise_shaders::Macros::standard();
    let (properties, _) = sunrise_shaders::read_properties(&pack, &mut macros);
    sunrise_shaders::load_post(&pack, "", &properties, &macros, &Default::default())
}

#[test]
#[ignore = "daily"]
fn an_output_far_past_the_buffers_does_not_size_the_targets() {
    let _gpu = crate::gfx::test_gpu::lock();
    let Some(gpu) = Fixture::new() else { return };
    for fragment in [
        "#version 430\n/* RENDERTARGETS: 0 */\nlayout(location = 0) out vec4 c;\n\
         layout(location = 1000000000) out vec4 far;\n\
         void main() { c = vec4(1.0, 0.0, 0.0, 1.0); far = vec4(1.0); }\n",
        "#version 120\n/* DRAWBUFFERS:0 */\n\
         void main() { gl_FragData[0] = vec4(1.0, 0.0, 0.0, 1.0); gl_FragData[1000000000] = vec4(1.0); }\n",
    ] {
        let post = pack(fragment);
        // The translator keeps the location: only the pass's construction meets it.
        let composite = post.programs.iter().find(|p| p.name == "composite");
        let outputs = composite
            .and_then(|p| p.program.as_ref())
            .map(|p| &p.interface.outputs);
        assert_eq!(outputs, Some(&vec![0, 1000000000]), "{fragment}");
        let mut post = gpu.try_build(post).expect("the pack runs");
        assert!(
            !post.passes.iter().any(|p| p.name == "composite"),
            "{fragment}"
        );
        assert!(
            post.notes
                .iter()
                .any(|n| n.starts_with("composite: skipped")),
            "{fragment}\n{:?}",
            post.notes
        );
        // The rest of the pack draws: composite1's colour reached colortex0, which `final` shows.
        // Expected bytes, from the source values k/255 for k = 64, 128, 191 (alpha 1.0 = 255/255):
        // colortex0 and the screen are `Rgba8Unorm` (the pack declares no format; `final` only
        // samples and returns the texel), so the bytes are k. The float source is k/255 within
        // 1e-7 relative, so f*255 is k within 1e-4 of a unit, half a unit from any rounding tie:
        // no stage (store into colortex0, sample back as k/255, store into the screen) can round
        // to another byte.
        let pixels = gpu.render(&mut post, [4, 4]);
        assert_eq!(
            pixels[0],
            [64, 128, 191, 255],
            "{fragment}\n{:?}",
            post.notes
        );
    }
}
