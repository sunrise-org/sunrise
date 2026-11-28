//! `coverage`'s static checks on translated composite programs, and the frame-start swap's
//! choice of buffers (#6858). The programs go through the pack loader, as a pack's would.

use super::*;

const FRAGMENT: &str = "#version 120\nvarying vec2 uv;\n/* DRAWBUFFERS:2 */\n\
                        void main() { gl_FragData[0] = vec4(uv, 0.0, 1.0); }\n";

/// The translated stages of a `composite` program.
fn program(vsh: &str, fsh: &str) -> sunrise_shaders::Program {
    let dir = tempfile::tempdir().unwrap();
    let shaders = dir.path().join("shaders");
    std::fs::create_dir_all(&shaders).unwrap();
    std::fs::write(shaders.join("composite.vsh"), vsh).unwrap();
    std::fs::write(shaders.join("composite.fsh"), fsh).unwrap();
    let pack = sunrise_shaders::Pack::open(dir.path()).unwrap();
    let properties = sunrise_shaders::Properties::parse("", &sunrise_shaders::Macros::default());
    let mut post = sunrise_shaders::load_post(
        &pack,
        "",
        &properties,
        &sunrise_shaders::Macros::standard(),
        &Default::default(),
    );
    let composite = post.programs.remove(0);
    assert_eq!(composite.name, "composite");
    composite
        .program
        .unwrap_or_else(|| panic!("translated: {:?}", post.notes))
}

/// Whether the `composite` vertex stage of `main`'s body is proven to cover its buffers.
fn covers(body: &str) -> bool {
    let vsh = format!(
        "#version 120\nvarying vec2 uv;\nuniform float frameTimeCounter;\n\
         vec4 same(vec4 v) {{ return v; }}\nvoid main() {{\n{body}\n}}\n"
    );
    covers_screen(&program(&vsh, FRAGMENT).vertex)
}

fn positions(body: &str) -> Option<Vec<[f32; 4]>> {
    let vsh = format!(
        "#version 120\nvarying vec2 uv;\nuniform float frameTimeCounter;\nvoid main() {{\n{body}\n}}\n"
    );
    quad_positions(&program(&vsh, FRAGMENT).vertex)
}

/// `hz_quad`'s corners through GL's quad projection, in wgpu's clip space: rows flipped for a
/// buffer, z from GL's -1..1 to 0..1.
#[test]
fn the_quad_runs_through_the_translated_vertex_stage() {
    let quad = vec![
        [-1.0, 1.0, 0.5, 1.0],
        [1.0, 1.0, 0.5, 1.0],
        [1.0, -1.0, 0.5, 1.0],
        [-1.0, 1.0, 0.5, 1.0],
        [1.0, -1.0, 0.5, 1.0],
        [-1.0, -1.0, 0.5, 1.0],
    ];
    assert_eq!(positions("gl_Position = ftransform();"), Some(quad));
    // The right half: known, and short of the screen.
    let half = positions("gl_Position = ftransform(); gl_Position.x = gl_Position.x * 0.5 + 0.5;");
    let xs: Vec<f32> = half.unwrap().iter().map(|p| p[0]).collect();
    assert_eq!(xs, [0.0, 1.0, 1.0, 0.0, 1.0, 0.0]);
    assert_eq!(
        positions("gl_Position = ftransform(); gl_Position.x += frameTimeCounter;"),
        None
    );
}

#[test]
fn the_quad_transforms_packs_use_cover_the_screen() {
    for body in [
        "gl_Position = ftransform(); uv = gl_MultiTexCoord0.xy;",
        "gl_Position = gl_ModelViewProjectionMatrix * gl_Vertex;",
        "gl_Position = gl_ProjectionMatrix * (gl_ModelViewMatrix * gl_Vertex);",
        "gl_Position = vec4(gl_Vertex.xy * 2.0 - 1.0, 0.0, 1.0);",
        "gl_Position = same(ftransform());",
        // What the position doesn't read may be anything.
        "gl_Position = ftransform(); uv = gl_MultiTexCoord0.xy * frameTimeCounter;",
        "gl_Position = ftransform(); if (frameTimeCounter > 1.0) uv = vec2(0.5);",
        "gl_Position = ftransform(); for (int i = 0; i < 4; i++) uv += vec2(0.25);",
        // One triangle over the screen; the second one has no area.
        "float x = float(gl_VertexID == 1) * 4.0 - 1.0; float y = float(gl_VertexID == 2) * 4.0 - 1.0;\n\
         gl_Position = vec4(x, y, 0.0, 1.0);",
    ] {
        assert!(covers(body), "{body}");
    }
}

/// A core-profile `composite` vertex stage of `position`.
fn core_covers(position: &str) -> bool {
    let vsh = format!(
        "#version 330 core\nin vec3 vaPosition;\nuniform mat4 modelViewMatrix;\n\
         uniform mat4 projectionMatrix;\nout vec2 uv;\nvoid main() {{\n\
         gl_Position = {position};\nuv = vaPosition.xy;\n}}\n"
    );
    let fsh = "#version 330 core\nin vec2 uv;\n/* RENDERTARGETS: 2 */\n\
               layout(location = 0) out vec4 color;\nvoid main() { color = vec4(uv, 0.0, 1.0); }\n";
    covers_screen(&program(&vsh, fsh).vertex)
}

#[test]
fn a_core_profile_quad_covers_the_screen() {
    assert!(core_covers("vec4(vaPosition.xy * 2.0 - 1.0, 0.0, 1.0)"));
    // In a composite program these are the frame's uniforms (`ScanResult::core_matrix`).
    assert!(!core_covers(
        "projectionMatrix * modelViewMatrix * vec4(vaPosition, 1.0)"
    ));
}

#[test]
fn a_quad_that_leaves_pixels_out_or_isnt_known_is_not_proven() {
    for body in [
        // The right half of the screen.
        "gl_Position = ftransform(); gl_Position.x = gl_Position.x * 0.5 + 0.5;",
        // Moved by a uniform.
        "gl_Position = ftransform(); gl_Position.x += frameTimeCounter;",
        "gl_Position = ftransform(); if (frameTimeCounter > 1.0) gl_Position.x = 0.0;",
        "gl_Position = ftransform(); for (int i = 0; i < 2; i++) gl_Position.x *= 1.0;",
        // The same quad, but through a division a GPU needn't round exactly.
        "gl_Position = vec4(gl_Vertex.xy * 2.0 - 1.0, 0.0, 1.0) / 1.0;",
        // A function's second call isn't run.
        "gl_Position = same(same(ftransform()));",
        // w of 2: the same square, through the GPU's own division.
        "gl_Position = ftransform() * 2.0;",
        // Beyond the far plane: z ends at (2 + 1) / 2.
        "gl_Position = vec4(gl_Vertex.xy * 2.0 - 1.0, 2.0, 1.0);",
    ] {
        assert!(!covers(body), "{body}");
    }
}

#[test]
fn a_discard_or_a_sample_mask_can_leave_a_pixel_unwritten() {
    let vsh = "#version 120\nvarying vec2 uv;\nvoid main() { gl_Position = ftransform(); uv = gl_MultiTexCoord0.xy; }\n";
    assert!(writes_every_covered_pixel(&program(vsh, FRAGMENT).fragment));
    let discards = "#version 120\nvarying vec2 uv;\n/* DRAWBUFFERS:2 */\n\
                    void main() { if (uv.x < 0.5) discard; gl_FragData[0] = vec4(1.0); }\n";
    assert!(!writes_every_covered_pixel(
        &program(vsh, discards).fragment
    ));
    // Neither GLSL reader makes a sample mask output; WGSL's does.
    let masked = "struct Out { @builtin(sample_mask) mask: u32, @location(0) color: vec4<f32> }\n\
                  @fragment fn main() -> Out { return Out(1u, vec4(1.0)); }\n";
    let module = naga::front::wgsl::parse_str(masked).unwrap();
    assert!(!writes_every_covered_pixel(&module));
}

const fn touch(written: u16, flip: u16, rewrites: u16, sometimes: bool) -> Touch {
    Touch {
        written,
        flip,
        rewrites,
        sometimes,
    }
}

#[test]
fn a_buffer_swaps_when_its_first_writer_replaces_it_whole() {
    // colortex2 (0b100) and colortex3 (0b1000), candidates both.
    let candidates = 0b1100;
    // A full write of colortex2 then a partial one of colortex3.
    let frame = [
        touch(0b100, 0b100, 0b100, false),
        touch(0b1000, 0b1000, 0, false),
    ];
    assert_eq!(swaps(frame, candidates), 0b100);
    // After its full first write, a partial one doesn't matter.
    let frame = [
        touch(0b100, 0b100, 0b100, false),
        touch(0b100, 0b100, 0, false),
    ];
    assert_eq!(swaps(frame, 0b100), 0b100);
    // Written whole without a flip (`flip.composite.colortex2 = false`).
    assert_eq!(swaps([touch(0b100, 0, 0b100, false)], 0b100), 0b100);
    // Never touched: its other side is never seen.
    assert_eq!(swaps([], 0b100), 0b100);
    // Only candidates: colortex4 is replaced whole but isn't one.
    assert_eq!(
        swaps([touch(0b1_0000, 0b1_0000, 0b1_0000, false)], 0b100),
        0b100
    );
}

#[test]
fn a_flip_without_a_write_or_a_partial_first_write_keeps_the_copy() {
    // A forced flip of a buffer the program doesn't write.
    assert_eq!(swaps([touch(0b1000, 0b1100, 0b1000, false)], 0b100), 0);
    // A partial write first, a full one after.
    let frame = [
        touch(0b100, 0b100, 0, false),
        touch(0b100, 0b100, 0b100, false),
    ];
    assert_eq!(swaps(frame, 0b100), 0);
}

#[test]
fn a_program_some_frames_skip_settles_nothing() {
    // A skipped full writer lets the next one decide: full keeps the swap...
    let frame = [
        touch(0b100, 0b100, 0b100, true),
        touch(0b100, 0b100, 0b100, false),
    ];
    assert_eq!(swaps(frame, 0b100), 0b100);
    // ...and partial ends it, as it is the first writer when the other is skipped.
    let frame = [
        touch(0b100, 0b100, 0b100, true),
        touch(0b100, 0b100, 0, false),
    ];
    assert_eq!(swaps(frame, 0b100), 0);
    // A skipped partial writer ends it, whatever follows.
    let frame = [
        touch(0b100, 0b100, 0, true),
        touch(0b100, 0b100, 0b100, false),
    ];
    assert_eq!(swaps(frame, 0b100), 0);
}

/// Calls nest as deep as the pack writes them: the interpreter keeps its own stack of frames.
#[test]
fn a_deep_chain_of_calls_runs_without_recursion() {
    let depth = 2000;
    let mut vsh = String::from("#version 120\nvec4 f0(vec4 v) { return v; }\n");
    for n in 1..depth {
        vsh.push_str(&format!("vec4 f{n}(vec4 v) {{ return f{}(v); }}\n", n - 1));
    }
    vsh.push_str(&format!(
        "void main() {{ gl_Position = f{}(ftransform()); }}\n",
        depth - 1
    ));
    assert!(covers_screen(&program(&vsh, FRAGMENT).vertex));
}
