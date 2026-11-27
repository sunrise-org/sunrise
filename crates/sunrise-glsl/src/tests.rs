use super::*;

/// Blocks, samplers and loose uniforms in a game's shape: the tests' own.
pub(crate) const BLOCKS: [(&str, u32); 4] = [
    ("Globals", 0),
    ("Projection", 1),
    ("DynamicTransforms", 2),
    ("Fog", 3),
];
pub(crate) const SAMPLERS: [(&str, u32); 3] = [("Sampler0", 4), ("Sampler1", 6), ("Sampler2", 8)];
const LOOSE: [&str; 3] = ["ProjMat", "ModelViewMat", "GameTime"];
/// The lowest stage interface limits a device has.
pub(crate) const LIMITS: InterfaceLimits = InterfaceLimits {
    inter_stage_variables: 32,
    vertex_attributes: 32,
};

/// The stages' files in the tests.
pub(crate) const VSH_FILE: &str = "minecraft:shaders/core/test.vsh";
pub(crate) const FSH_FILE: &str = "minecraft:shaders/core/test.fsh";

fn includes<'a>(read: &'a dyn Fn(&str) -> Result<Option<String>, String>) -> Includes<'a> {
    Includes {
        read,
        namespace: "minecraft",
        folder: "shaders/include/",
        once: &["moj_import"],
    }
}

pub(crate) fn translate_core_at(
    sources: &Stages,
    defines: &[&str],
    read: &dyn Fn(&str) -> Result<Option<String>, String>,
    glue: &Glue,
    provided: &dyn Fn(&str) -> bool,
) -> Result<Translated, Vec<Error>> {
    let inputs = Inputs {
        blocks: &BLOCKS,
        samplers: &SAMPLERS,
        extra_set: 0,
        extra_blocks: &[],
        extra_samplers: &[],
        extra_texel_buffers: &[],
        extra_storage_buffers: &[],
        extra_storage_images: &[],
        push_constant_binding: None,
        loose: &LOOSE,
        defines,
        includes: &includes(read),
    };
    translate(sources, &inputs, glue, provided)
}

pub(crate) fn translate_core_with(
    vertex: &str,
    fragment: &str,
    defines: &[&str],
    include: &dyn Fn(&str) -> Option<String>,
    glue: &Glue,
    provided: &dyn Fn(&str) -> bool,
) -> Result<Translated, Vec<Error>> {
    let sources = Stages {
        vertex_dir: "minecraft:shaders/core/",
        fragment_dir: "minecraft:shaders/core/",
        vertex,
        fragment,
        vertex_file: VSH_FILE,
        fragment_file: FSH_FILE,
    };
    translate_core_at(&sources, defines, &|id| Ok(include(id)), glue, provided)
}

pub(crate) fn translate_core(
    vertex: &str,
    fragment: &str,
    defines: &[&str],
    include: &dyn Fn(&str) -> Option<String>,
    glue: &Glue,
) -> Result<Translated, Vec<Error>> {
    translate_core_with(vertex, fragment, defines, include, glue, &|_| false)
}

fn expand_imports(
    text: &str,
    dir: &str,
    read: &dyn Fn(&str) -> Result<Option<String>, String>,
) -> Result<String, ImportRefusal> {
    expand_includes(text, VSH_FILE, dir, &includes(read), None)
        .map(|e| e.text().to_owned())
        .map_err(|e| e.refusal)
}

fn expand(
    text: &str,
    dir: &str,
    include: &dyn Fn(&str) -> Option<String>,
) -> Result<String, ImportRefusal> {
    expand_imports(text, dir, &|id| Ok(include(id)))
}

fn expand_core_imports(
    text: &str,
    dir: &str,
    defines: &[&str],
    read: &dyn Fn(&str) -> Result<Option<String>, String>,
) -> Result<String, ImportRefusal> {
    expand_includes(text, VSH_FILE, dir, &includes(read), Some(defines))
        .map(|e| e.text().to_owned())
        .map_err(|e| e.refusal)
}

const INCLUDES: [(&str, &str); 4] = [
    (
        "minecraft:shaders/include/dynamictransforms.glsl",
        "#ifndef DT\n#define DT\nlayout(std140) uniform DynamicTransforms {\n    mat4 ModelViewMat;\n    mat4 TextureMat;\n    vec4 ColorModulator;\n    vec3 ModelOffset;\n};\n#endif\n",
    ),
    (
        "minecraft:shaders/include/projection.glsl",
        "layout(std140) uniform Projection {\n    mat4 ProjMat;\n};\n",
    ),
    (
        "minecraft:shaders/include/globals.glsl",
        "layout(std140) uniform Globals {\n    ivec3 CameraBlockPos;\n    float GlintAlpha;\n    vec3 CameraOffset;\n    float GameTime;\n    vec2 ScreenSize;\n    int MenuBlurRadius;\n    int UseRgss;\n};\n",
    ),
    (
        "minecraft:shaders/include/fog.glsl",
        "layout(std140) uniform Fog {\n    vec4 FogColor;\n    float FogEnvironmentalStart;\n};\nvec4 apply_fog(vec4 c) { return mix(c, FogColor, FogEnvironmentalStart); }\n",
    ),
];

fn include(id: &str) -> Option<String> {
    INCLUDES
        .iter()
        .find(|(path, _)| *path == id)
        .map(|(_, text)| (*text).to_owned())
}

const GLUE: Glue = Glue {
    inputs: &[("Position", "vec3"), ("Color", "vec4"), ("UV0", "vec2")],
    prelude: "layout(location = 0) in vec2 hz_pos;\nlayout(location = 1) in vec2 hz_uv;\nlayout(location = 2) in vec4 hz_color;\n\
              const int hz_corner[6] = int[6](0, 3, 2, 0, 2, 1);\n\
              int hz_vertex_id() { int i = int(gl_VertexIndex) - int(gl_InstanceIndex); return i / 6 * 4 + hz_corner[i % 6]; }",
    setup: "Position = vec3(hz_pos, 0.0); Color = hz_color; UV0 = hz_uv;",
    finish: "gl_Position.z = 0.5 * gl_Position.w;",
    vertex_id: "hz_vertex_id()",
    fragment_finish: None,
    interface: LIMITS,
};

/// 26.3's `text.vsh` and `text.fsh` in shape: `#include`, explicit locations, an extension.
const VANILLA_VSH: &str = "#version 330\n#extension GL_ARB_separate_shader_objects : require\n\
    #if !defined(IS_GUI)\n#include <minecraft:fog.glsl>\n#endif\n\
    #include <minecraft:dynamictransforms.glsl>\n#include <minecraft:projection.glsl>\n\
    layout(location = 0) in vec3 Position;\nlayout(location = 1) in vec4 Color;\nlayout(location = 2) in vec2 UV0;\n\
    layout(location = 2) out vec4 vertexColor;\nlayout(location = 3) out vec2 texCoord0;\n\
    void main() {\n    gl_Position = ProjMat * ModelViewMat * vec4(Position, 1.0);\n    vertexColor = Color;\n    texCoord0 = UV0;\n}\n";
const VANILLA_FSH: &str = "#version 330\n#extension GL_ARB_separate_shader_objects : require\n\
    #include <minecraft:dynamictransforms.glsl>\n\
    uniform sampler2D Sampler0;\n\
    layout(location = 2) in vec4 vertexColor;\nlayout(location = 3) in vec2 texCoord0;\n\
    layout(location = 0) out vec4 fragColor;\n\
    void main() {\n    vec4 color = texture(Sampler0, texCoord0) * vertexColor * ColorModulator;\n    if (color.a < 0.1) {\n        discard;\n    }\n    fragColor = color;\n}\n";

#[test]
fn vanilla_26_3_text_shaders_translate() {
    let program = translate_core(VANILLA_VSH, VANILLA_FSH, &["IS_GUI"], &include, &GLUE).unwrap();
    assert_eq!(program.vertex.entry_points.len(), 1);
    assert_eq!(program.fragment.entry_points.len(), 1);
}

/// Whether `function`'s body calls the function named `callee`.
fn calls(module: &naga::Module, function: &naga::Function, callee: &str) -> bool {
    function.body.iter().any(|s| {
        matches!(s, naga::Statement::Call { function, .. }
            if module.functions[*function].name.as_deref() == Some(callee))
    })
}

/// The fragment stage's GLSL `main` (naga's entry point calls it).
fn glsl_main(module: &naga::Module) -> &naga::Function {
    let (_, main) = (module.functions.iter())
        .find(|(_, f)| f.name.as_deref() == Some("main"))
        .expect("main");
    main
}

/// `fragment_finish` runs in the fragment stage's `main` after the program's `main`, renamed
/// `hz_main` (whose `discard` stays its own).
#[test]
fn a_fragment_finish_runs_after_the_programs_main() {
    let glue = Glue {
        fragment_finish: Some("fragColor = fragColor.bgra;"),
        ..GLUE
    };
    let program = translate_core(VANILLA_VSH, VANILLA_FSH, &["IS_GUI"], &include, &glue)
        .unwrap_or_else(|e| panic!("{e:?}"));
    let fragment = &program.fragment;
    let main = glsl_main(fragment);
    let body: Vec<&naga::Statement> = main.body.iter().collect();
    let call = (body.iter())
        .position(|s| {
            matches!(s, naga::Statement::Call { function, .. }
            if fragment.functions[*function].name.as_deref() == Some("hz_main"))
        })
        .unwrap_or_else(|| panic!("main calls hz_main: {:#?}", main.body));
    let bgra = [
        naga::SwizzleComponent::Z,
        naga::SwizzleComponent::Y,
        naga::SwizzleComponent::X,
        naga::SwizzleComponent::W,
    ];
    let swizzled = body[call..].iter().any(|s| {
        matches!(s, naga::Statement::Store { value, .. }
            if matches!(main.expressions[*value], naga::Expression::Swizzle { pattern, .. } if pattern == bgra))
    });
    assert!(swizzled, "{:#?}", main.body);
    let (_, program_main) = (fragment.functions.iter())
        .find(|(_, f)| f.name.as_deref() == Some("hz_main"))
        .expect("the program's main, renamed");
    assert!(
        (program_main.body.iter()).any(|s| matches!(s, naga::Statement::If { .. })),
        "the program's discard stays in its main"
    );
}

/// Without `fragment_finish` the fragment stage's `main` is the program's, as written.
#[test]
fn no_fragment_finish_leaves_the_fragment_main_as_written() {
    let program = translate_core(VANILLA_VSH, VANILLA_FSH, &["IS_GUI"], &include, &GLUE)
        .unwrap_or_else(|e| panic!("{e:?}"));
    let fragment = &program.fragment;
    assert!(!calls(fragment, glsl_main(fragment), "hz_main"));
    assert!(
        (fragment.functions.iter()).all(|(_, f)| f.name.as_deref() != Some("hz_main")),
        "no hz_main"
    );
}

/// A 26.3 pack's `gl_VertexIndex` means vanilla's vertex number, four per quad. The text
/// pipeline draws six per glyph, so the pack's code must read the glue's `hz_vertex_id()`
/// (as `gl_VertexID` does), never the raw index.
#[test]
fn a_packs_gl_vertex_index_reads_the_glues_vertex_number() {
    let vsh = "#version 330\n#include <minecraft:dynamictransforms.glsl>\n#include <minecraft:projection.glsl>\n\
        layout(location = 0) in vec3 Position;\nlayout(location = 1) in vec4 Color;\nlayout(location = 2) in vec2 UV0;\n\
        layout(location = 2) out vec4 vertexColor;\nlayout(location = 3) out vec2 texCoord0;\n\
        void main() {\n    int corner = gl_VertexIndex % 4;\n\
            gl_Position = ProjMat * ModelViewMat * vec4(Position + vec3(float(corner), 0.0, 0.0), 1.0);\n\
            vertexColor = Color;\n    texCoord0 = UV0;\n}\n";
    let program = translate_core(vsh, VANILLA_FSH, &["IS_GUI"], &include, &GLUE).unwrap();
    let module = &program.vertex;
    let named = |name: &str| {
        module
            .functions
            .iter()
            .find(|(_, f)| f.name.as_deref() == Some(name))
            .unwrap_or_else(|| panic!("no {name}"))
    };
    let (vertex_id, _) = named("hz_vertex_id");
    let (_, pack_main) = named("hz_main");
    fn calls(block: &naga::Block, target: naga::Handle<naga::Function>) -> bool {
        block.iter().any(|statement| match statement {
            naga::Statement::Call { function, .. } => *function == target,
            naga::Statement::Block(inner) => calls(inner, target),
            naga::Statement::If { accept, reject, .. } => {
                calls(accept, target) || calls(reject, target)
            }
            naga::Statement::Loop {
                body, continuing, ..
            } => calls(body, target) || calls(continuing, target),
            _ => false,
        })
    }
    assert!(calls(&pack_main.body, vertex_id));
}

/// A 26.2-style pack shader as a HUD pack writes it: `#moj_import`, no locations, a flat
/// varying, `gl_VertexID`, `texelFetch` and `textureSize` on the glyph atlas, `GameTime`.
#[test]
fn a_hud_pack_text_shader_translates() {
    let vsh = "#version 330\n\
        #moj_import <minecraft:dynamictransforms.glsl>\n#moj_import <minecraft:projection.glsl>\n\
        in vec3 Position;\nin vec4 Color;\nin vec2 UV0;\n\
        out vec4 vertexColor;\nout vec2 texCoord0;\n\
        #ifdef IS_GUI\n#moj_import <minecraft:globals.glsl>\nuniform sampler2D Sampler0; // the glyphs\nflat out vec4 effect;\n#endif\n\
        /* moved\n   glyphs */\n\
        void main() {\n    vec3 p = Position;\n#ifdef IS_GUI\n    int corner = gl_VertexID % 4;\n\
            ivec2 size = textureSize(Sampler0, 0);\n\
            if (ivec4(round(texelFetch(Sampler0, ivec2(UV0 * vec2(size)), 0) * 255.0)) == ivec4(177, 126, 91, 1)) {\n\
                p.x += float(corner) + ScreenSize.x * GameTime;\n    }\n    effect = vec4(float((ivec3(Color.rgb * 255.0) >> 4) == ivec3(3, 14, 10)));\n#endif\n\
            gl_Position = ProjMat * ModelViewMat * vec4(p, 1.0);\n    vertexColor = Color;\n    texCoord0 = UV0;\n}\n";
    let fsh = "#version 330\n#moj_import <minecraft:dynamictransforms.glsl>\n#moj_import <minecraft:globals.glsl>\n\
        uniform sampler2D Sampler0;\nin vec4 vertexColor;\nin vec2 texCoord0;\nflat in vec4 effect;\nout vec4 fragColor;\n\
        void main() {\n    vec4 color = texture(Sampler0, texCoord0) * vertexColor;\n    if (effect.x > 0.5) {\n        fragColor = vertexColor * ColorModulator;\n        return;\n    }\n\
            if (color.a < 0.1) {\n        discard;\n    }\n    fragColor = color * sin(GameTime);\n}\n";
    translate_core(vsh, fsh, &["IS_GUI"], &include, &GLUE).unwrap();
}

#[test]
fn varyings_meet_at_the_same_locations() {
    let vsh = "#version 330\nin vec3 Position;\nout float b;\nout vec2 m;\nout vec4 a;\n\
        void main() { gl_Position = vec4(Position, 1.0); b = 1.0; m = vec2(1.0); a = vec4(b); }\n";
    let fsh = "#version 330\nin vec4 a;\nin float b;\nout vec4 fragColor;\nvoid main() { fragColor = a * b; }\n";
    let program = translate_core(vsh, fsh, &[], &include, &GLUE).unwrap();
    let mut read = locations(&program.fragment, true);
    read.sort_unstable();
    assert_eq!(read, [0, 2]);
}

#[test]
fn explicit_varying_locations_are_kept() {
    let vsh = "#version 330\nin vec3 Position;\n\
        #ifdef IS_GUI\nlayout(location = 4) out float gui;\n#else\nlayout(location = 4) out float world;\n#endif\n\
        layout(location = 13) out vec4 a;\nout float b;\n\
        void main() { gl_Position = vec4(Position, 1.0); a = vec4(1.0); b = 1.0;\n\
        #ifdef IS_GUI\ngui = 1.0;\n#else\nworld = 1.0;\n#endif\n}\n";
    let fsh = "#version 330\n#ifdef IS_GUI\nlayout(location = 4) in float gui;\n#endif\n\
        layout(location = 13) in vec4 a;\nin float b;\nout vec4 fragColor;\n\
        void main() { fragColor = a * b;\n#ifdef IS_GUI\nfragColor *= gui;\n#endif\n}\n";
    let program = translate_core(vsh, fsh, &["IS_GUI"], &include, &GLUE).unwrap();
    let mut read = locations(&program.fragment, true);
    read.sort_unstable();
    assert_eq!(read, [0, 4, 13]);
}

/// A vertex stage writing `written` (a qualified `out` declaration) at location 1, and a
/// fragment stage reading `read` there through `value`, each translated with a stage that
/// meets it: the two modules [`link`] is given.
fn stages(written: &str, read: &str, value: &str) -> (naga::Module, naga::Module) {
    let ty = |declaration: &str| declaration.rsplit(' ').next().unwrap().to_owned();
    let vsh = |written: &str| {
        format!(
            "#version 330\nin vec3 Position;\nlayout(location = 1) {written} tag;\n\
             layout(location = 2) out vec3 unread;\nvoid main() {{ gl_Position = vec4(Position, 1.0); \
             tag = {}(1); unread = vec3(1.0); }}\n",
            ty(written)
        )
    };
    let fsh = |read: &str, value: &str| {
        format!(
            "#version 330\nlayout(location = 1) {read} tag;\nout vec4 fragColor;\n\
             void main() {{ fragColor = vec4({value}); }}\n"
        )
    };
    let plain = "#version 330\nout vec4 fragColor;\nvoid main() { fragColor = vec4(1.0); }\n";
    let vertex = translate_core(&vsh(written), plain, &[], &include, &GLUE).unwrap();
    let meeting = read
        .replacen("in ", "out ", 1)
        .replace("noperspective ", "");
    let meeting = meeting.replace("centroid ", "").replace("sample ", "");
    let fragment = translate_core(&vsh(&meeting), &fsh(read, value), &[], &include, &GLUE).unwrap();
    (vertex.vertex, fragment.fragment)
}

/// Vanilla's link check (`PipelineBuilder`): a varying the fragment stage reads with another
/// base type, another number of components, or flat in one stage only, than the vertex stage
/// writes at its location is refused, named; nothing else about interpolation or sampling is
/// compared, and a vertex output the fragment stage doesn't read is fine.
#[test]
fn the_stages_meet_as_vanillas_link_check_requires() {
    for (written, read, value, refused) in [
        ("out vec2", "in vec2", "tag.x", None),
        ("flat out ivec2", "flat in ivec2", "float(tag.x)", None),
        ("out vec2", "noperspective in vec2", "tag.x", None),
        ("centroid out vec2", "in vec2", "tag.x", None),
        ("out vec2", "sample in vec2", "tag.x", None),
        (
            "flat out ivec2",
            "flat in vec2",
            "tag.x",
            Some(LinkRefusal::BaseType(1)),
        ),
        (
            "flat out uvec2",
            "flat in ivec2",
            "float(tag.x)",
            Some(LinkRefusal::BaseType(1)),
        ),
        (
            "out vec3",
            "in vec2",
            "tag.x",
            Some(LinkRefusal::Components(1)),
        ),
        (
            "out vec2",
            "in float",
            "tag",
            Some(LinkRefusal::Components(1)),
        ),
        (
            "flat out vec2",
            "in vec2",
            "tag.x",
            Some(LinkRefusal::Flat(1)),
        ),
        (
            "out vec2",
            "flat in vec2",
            "tag.x",
            Some(LinkRefusal::Flat(1)),
        ),
    ] {
        let (vertex, fragment) = stages(written, read, value);
        assert_eq!(
            link(&vertex, &fragment).err(),
            refused,
            "{written} / {read}"
        );
    }
    let (vertex, _) = stages("out vec2", "in vec2", "tag.x");
    let (_, fragment) = stages("out vec2", "in vec4", "tag.x");
    assert_eq!(link(&vertex, &fragment), Err(LinkRefusal::Components(1)));
    let fsh = "#version 330\nlayout(location = 3) in vec2 tag;\nout vec4 fragColor;\n\
               void main() { fragColor = vec4(tag.x); }\n";
    let vsh = "#version 330\nin vec3 Position;\nlayout(location = 3) out vec2 tag;\n\
               void main() { gl_Position = vec4(Position, 1.0); tag = vec2(1.0); }\n";
    let missing = translate_core(vsh, fsh, &[], &include, &GLUE)
        .unwrap()
        .fragment;
    assert_eq!(link(&vertex, &missing), Err(LinkRefusal::Missing(3)));
}

/// The vertex stage's output takes the interpolation and sampling the fragment stage reads
/// it with, as the device needs them equal: a program whose stages differ only there
/// translates, its vertex output `noperspective`, `centroid` or `sample` as its fragment
/// input is, and an output the fragment stage doesn't read keeps its own.
#[test]
fn the_vertex_output_takes_the_fragment_inputs_interpolation() {
    let qualifiers = |module: &naga::Module, inputs: bool, location: u32| {
        varyings(module, inputs)
            .into_iter()
            .find(|(l, _)| *l == location)
            .map(|(_, v)| (v.interpolation, v.sampling))
            .unwrap()
    };
    for (written, read) in [
        ("out vec2", "noperspective in vec2"),
        ("centroid out vec2", "in vec2"),
        ("out vec2", "sample in vec2"),
        ("noperspective out vec2", "centroid in vec2"),
    ] {
        let vsh = format!(
            "#version 330\nin vec3 Position;\nlayout(location = 1) {written} tag;\n\
             layout(location = 2) flat out ivec3 unread;\nvoid main() {{ gl_Position = vec4(Position, 1.0); \
             tag = vec2(1.0); unread = ivec3(1); }}\n"
        );
        let fsh = format!(
            "#version 330\nlayout(location = 1) {read} tag;\nout vec4 fragColor;\n\
             void main() {{ fragColor = vec4(tag.x); }}\n"
        );
        let program = translate_core(&vsh, &fsh, &[], &include, &GLUE).unwrap();
        assert_eq!(
            qualifiers(&program.vertex, false, 1),
            qualifiers(&program.fragment, true, 1),
            "{written} / {read}"
        );
        assert_eq!(
            qualifiers(&program.vertex, false, 2).0,
            Some(naga::Interpolation::Flat),
            "{written} / {read}: the unread output"
        );
    }
}

/// wgpu's default device takes 16 varyings at locations 0 to 15 and 12 uniform blocks a
/// stage, where vanilla's device takes more: a program past either is told by the typed note
/// the pipelines draw natively with (`within_device_limits`).
#[test]
fn a_program_past_the_default_device_limits_is_told() {
    let limits = wgpu_types::Limits::default();
    let program = |location: u32, blocks: usize| {
        let declared: String = (0..blocks)
            .map(|i| format!("layout(std140) uniform B{i} {{\n    float v{i};\n}};\n"))
            .collect();
        let sum: String = (0..blocks).map(|i| format!(" + v{i}")).collect();
        let vsh = format!(
            "#version 330\nin vec3 Position;\nlayout(location = {location}) out float tag;\n\
             void main() {{ gl_Position = vec4(Position, 1.0); tag = 1.0; }}\n"
        );
        let fsh = format!(
            "#version 330\n{declared}layout(location = {location}) in float tag;\nout vec4 fragColor;\n\
             void main() {{ fragColor = vec4(tag{sum}); }}\n"
        );
        translate_core_with(&vsh, &fsh, &[], &include, &GLUE, &|_| true).unwrap()
    };
    assert_eq!(
        program(15, 12).past_device_limit(&limits, &[wgpu_types::TextureFormat::Rgba8Unorm]),
        None
    );
    assert_eq!(
        program(16, 0).past_device_limit(&limits, &[wgpu_types::TextureFormat::Rgba8Unorm]),
        Some(PastDeviceLimit::Varyings {
            count: 1,
            highest: 16,
            limit: 16
        })
    );
    assert_eq!(
        program(0, 13).past_device_limit(&limits, &[wgpu_types::TextureFormat::Rgba8Unorm]),
        Some(PastDeviceLimit::UniformBlocks {
            stage: wgpu_types::ShaderStages::FRAGMENT,
            blocks: 13,
            limit: 12
        })
    );
}

/// The device takes a colour target written with the format's base type and at least as many
/// components; vanilla's builder checks no output, so a program writing a 4-channel float
/// target with a `vec3` or a `uvec4` is told by the note it draws natively with, while a
/// single-channel target takes a `float`.
#[test]
fn a_colour_output_the_device_doesnt_take_is_told() {
    let limits = wgpu_types::Limits::default();
    let program = |output: &str, value: &str| {
        let fsh = format!(
            "#version 330\nlayout(location = 0) out {output} fragColor;\nvoid main() {{ fragColor = {value}; }}\n"
        );
        let vsh =
            "#version 330\nin vec3 Position;\nvoid main() { gl_Position = vec4(Position, 1.0); }\n";
        translate_core(vsh, &fsh, &[], &include, &GLUE).unwrap()
    };
    let rgba = [wgpu_types::TextureFormat::Rgba8Unorm];
    assert_eq!(
        program("vec4", "vec4(1.0)").past_device_limit(&limits, &rgba),
        None
    );
    for (output, value, kind, components) in [
        ("vec3", "vec3(1.0)", naga::ScalarKind::Float, 3),
        ("uvec4", "uvec4(1u)", naga::ScalarKind::Uint, 4),
    ] {
        assert_eq!(
            program(output, value).past_device_limit(&limits, &rgba),
            Some(PastDeviceLimit::ColourOutput {
                location: 0,
                kind: Some(kind),
                components,
                target: wgpu_types::TextureFormat::Rgba8Unorm
            }),
            "{output}"
        );
    }
    let single = [wgpu_types::TextureFormat::R16Float];
    assert_eq!(
        program("float", "1.0").past_device_limit(&limits, &single),
        None
    );
}

#[test]
fn an_include_is_pasted_every_time_and_a_moj_import_once() {
    let file = |id: &str| {
        (id == "minecraft:shaders/include/g.glsl")
            .then(|| "#ifndef G\n#define G\nconst float g = 1.0;\n#endif\n".to_owned())
    };
    let twice =
        "#ifdef A\n#include <minecraft:g.glsl>\n#else\n#include <minecraft:g.glsl>\n#endif\n";
    let expanded = expand(twice, "minecraft:shaders/core/", &file).unwrap();
    assert_eq!(expanded.matches("const float g").count(), 2);
    let imported = expand(
        &twice.replace("#include", "#moj_import"),
        "minecraft:shaders/core/",
        &file,
    )
    .unwrap();
    assert_eq!(imported.matches("const float g").count(), 1);
}

#[test]
fn a_location_another_branch_gave_the_same_name_isnt_reused() {
    let vsh = "#version 330\nin vec3 Position;\n\
        #ifdef IS_GUI\nlayout(location = 0) out vec4 vertexColor;\n\
        #else\nlayout(location = 1) out vec4 vertexColor;\nlayout(location = 0) out vec4 worldOther;\n#endif\n\
        void main() { gl_Position = vec4(Position, 1.0); vertexColor = vec4(1.0);\n\
        #ifndef IS_GUI\nworldOther = vec4(0.5);\n#endif\n}\n";
    let fsh = "#version 330\n\
        #ifdef IS_GUI\nlayout(location = 0) in vec4 vertexColor;\n\
        #else\nlayout(location = 1) in vec4 vertexColor;\nlayout(location = 0) in vec4 worldOther;\n#endif\n\
        out vec4 fragColor;\nvoid main() { fragColor = vertexColor;\n\
        #ifndef IS_GUI\nfragColor *= worldOther;\n#endif\n}\n";
    let world = translate_core(vsh, fsh, &[], &include, &GLUE).unwrap();
    let mut written = locations(&world.vertex, false);
    written.sort_unstable();
    assert_eq!(written, [0, 1]);
    let mut read = locations(&world.fragment, true);
    read.sort_unstable();
    assert_eq!(read, [0, 1]);
    let gui = translate_core(vsh, fsh, &["IS_GUI"], &include, &GLUE).unwrap();
    assert_eq!(locations(&gui.vertex, false), [0]);
}

#[test]
fn includes_that_multiply_are_refused_once_over_budget() {
    let file = |id: &str| {
        let level: u32 = id
            .strip_prefix("minecraft:shaders/include/l")?
            .strip_suffix(".glsl")?
            .parse()
            .ok()?;
        let next = format!("#include <minecraft:l{}.glsl>\n", level + 1).repeat(4);
        Some(format!(
            "#ifndef L{level}\n#define L{level}\n{}#endif\n",
            if level < 15 { next.as_str() } else { "" }
        ))
    };
    let error = expand(
        "#include <minecraft:l0.glsl>\n",
        "minecraft:shaders/core/",
        &file,
    )
    .unwrap_err();
    assert_eq!(error, ImportRefusal::TooManyPastes);
}

/// The bytes scanned are bounded too, not only those kept: a file of imports already pasted
/// keeps a line break each, and is still read in full every time it is included. Seven
/// pastes of 1.05 MB fit [`MAX_SCANNED`] (8 MiB), the eighth is refused.
#[test]
fn includes_that_keep_little_are_refused_once_scanned_past_the_budget() {
    let seen = "#moj_import <b.glsl>\n".repeat(50_000);
    let file = |id: &str| match id {
        "minecraft:shaders/include/a.glsl" => Some(seen.clone()),
        "minecraft:shaders/include/b.glsl" => Some(String::new()),
        _ => None,
    };
    let main = |n: usize| format!("#moj_import <b.glsl>\n{}", "#include <a.glsl>\n".repeat(n));
    let dir = "minecraft:shaders/core/";
    assert!(expand(&main(7), dir, &file).is_ok());
    assert_eq!(
        expand(&main(8), dir, &file),
        Err(ImportRefusal::TooMuchScanned)
    );
}

#[test]
fn a_define_with_a_value_is_defined_to_it() {
    let vsh = "#version 330\nin vec3 Position;\n\
        void main() { gl_Position = vec4(Position, float(COUNT)); }\n";
    let fsh = "#version 330\nout vec4 fragColor;\nvoid main() { fragColor = vec4(COUNT / 4); }\n";
    assert!(translate_core(vsh, fsh, &["COUNT 8"], &include, &GLUE).is_ok());
}

#[test]
fn minified_varying_declarations_are_rewritten() {
    let vsh = "#version 330\nout float sphericalVertexDistance;out float cylindricalVertexDistance;\
        void main() { sphericalVertexDistance = 0.0; cylindricalVertexDistance = 0.0; \
        gl_Position = vec4(0.0); }\n";
    let fsh = "#version 330\nin float sphericalVertexDistance;in float cylindricalVertexDistance;\
        out vec4 fragColor;void main() { fragColor = vec4(sphericalVertexDistance + cylindricalVertexDistance); }\n";
    translate_core(vsh, fsh, &[], &include, &GLUE).unwrap();
}

#[test]
fn legacy_uniforms_next_to_modern_blocks_are_ignored() {
    let vsh = "#version 330\n#include <minecraft:projection.glsl>\n\
        uniform mat4 ProjMat;out vec4 vertexColor;\
        void main() { vertexColor = vec4(1.0); gl_Position = ProjMat[0]; }\n";
    let fsh = "#version 330\nin vec4 vertexColor;out vec4 fragColor;\
        void main() { fragColor = vertexColor; }\n";
    translate_core(vsh, fsh, &[], &include, &GLUE).unwrap();
}

/// A block declared on one line, as packs write a one-float uniform, is bound like one
/// over several lines; the blocks and samplers the caller provides are accepted, in binding
/// order, and one it doesn't provide is not.
#[test]
fn a_block_on_one_line_is_bound_and_provided_blocks_are_accepted() {
    let vsh = "#version 330\nlayout(std140) uniform Day { float daylight; };\nout vec4 vertexColor;\n\
        void main() { vertexColor = vec4(daylight); gl_Position = vec4(0.0); }\n";
    let fsh = "#version 330\nuniform sampler2D Bounds;\nin vec4 vertexColor;\nout vec4 fragColor;\n\
        void main() { fragColor = vertexColor * texture(Bounds, vec2(0.5)); }\n";
    let both = |name: &str| name == "Day" || name == "Bounds";
    let program = translate_core_with(vsh, fsh, &[], &include, &GLUE, &both).unwrap();
    assert_eq!(program.other, ["Day", "Bounds"]);
    // The bindings the caller gives them at.
    let bindings = |module: &naga::Module| -> Vec<(Option<String>, u32)> {
        module
            .global_variables
            .iter()
            .filter_map(|(_, g)| Some((g.name.clone(), g.binding.as_ref()?.binding)))
            .collect()
    };
    assert!(
        bindings(&program.vertex)
            .iter()
            .any(|(_, b)| *b == program.other_binding(0)),
        "{:?}",
        bindings(&program.vertex)
    );
    let fragment = bindings(&program.fragment);
    assert!(
        fragment.contains(&(Some("hz_tex_Bounds".into()), program.other_binding(1))),
        "{fragment:?}"
    );
    assert!(
        fragment.contains(&(Some("hz_smp_Bounds".into()), program.other_binding(1) + 1)),
        "{fragment:?}"
    );
    // Each at the line that declares it.
    let error = translate_core(vsh, fsh, &[], &include, &GLUE).unwrap_err();
    assert_eq!(
        error,
        vec![Error {
            file: VSH_FILE.into(),
            line: 2,
            column: 24,
            stage: Some(ShaderStage::Vertex),
            unprovided: Some("Day".into()),
            message: "Day is not a block or sampler of this pipeline".into(),
        }]
    );
    let only_day = |name: &str| name == "Day";
    let error = translate_core_with(vsh, fsh, &[], &include, &GLUE, &only_day).unwrap_err();
    assert_eq!(
        error,
        vec![Error {
            file: FSH_FILE.into(),
            line: 2,
            column: 1,
            stage: Some(ShaderStage::Fragment),
            unprovided: Some("Bounds".into()),
            message: "Bounds is not a block or sampler of this pipeline".into(),
        }]
    );
}

fn block_named(line: &str) -> Option<(&str, &str)> {
    block_name(layout_prefix(line)?.1)
}

fn storage_of(line: &str) -> Option<StorageBlock<'_>> {
    let (layout, rest, _) = layout_prefix(line)?;
    storage_block(layout, rest)
}

/// Only a uniform block's declaration names a block, written over several lines or on one.
#[test]
fn block_names_are_read_from_block_declarations_only() {
    let name = |line| block_named(line).map(|(name, _)| name);
    assert_eq!(name("layout(std140) uniform Day {"), Some("Day"));
    assert_eq!(name("uniform Day { float daylight; };"), Some("Day"));
    assert_eq!(name("uniform Foo{"), Some("Foo"));
    assert_eq!(name("uniform Globals"), Some("Globals"));
    assert_eq!(
        block_named("layout(std140) uniform Day {").map(|(_, tail)| tail),
        Some("uniform Day {")
    );
    assert_eq!(block_named("uniform float x;"), None);
    // Only `layout(...)` may stand before `uniform`, and it must close.
    assert_eq!(block_named("layout(std140) flat uniform Day {"), None);
    assert_eq!(block_named("layout(std140 uniform Day {"), None);
    assert_eq!(block_named("uniform sampler2D Sampler0;"), None);
}

/// `uniform` is a keyword only as a whole token: `uniformColor` opens no block, and a block
/// after code on its line is listed and bound.
#[test]
fn uniform_is_a_whole_token_and_a_declaration_after_code_is_read() {
    let kinds = |text| {
        resources(text)
            .iter()
            .map(|r| (r.name, r.kind))
            .collect::<Vec<_>>()
    };
    assert_eq!(kinds("vec4 uniformColor = vec4(1.0);"), []);
    assert_eq!(
        kinds("layout(location = 0) vec4 uniformColor\n    = vec4(1.0);"),
        []
    );
    assert_eq!(
        kinds("void f() { } uniform sampler2D Late;"),
        [("Late", ResourceKind::Sampler2D)]
    );
    assert_eq!(
        kinds("void f() { int x; } uniform Late { vec4 a; };"),
        [("Late", ResourceKind::UniformBlock)]
    );
    // Bound: the sampler gets its texture and sampler globals and `f` stays.
    let fsh = "#version 330\nvoid f() { } uniform sampler2D Late;\nout vec4 fragColor;\n\
        void main() { fragColor = texture(Late, vec2(0.5)); f(); }\n";
    let vsh = "#version 330\nvoid main() { gl_Position = vec4(0.0); }\n";
    translate_core_with(vsh, fsh, &[], &include, &GLUE, &|name| name == "Late").unwrap();
}

/// The `in` declarations of a stage, name and type in order, whatever else is on the line.
#[test]
fn stage_inputs_lists_every_in_declaration() {
    let text = "in vec3 Position; in vec4 Color;\nuniform float u;\nvoid f() { } in vec2 UV0;\nout vec4 o;\n";
    assert_eq!(
        stage_inputs(text),
        [
            (text.find("Position").unwrap(), "Position", "vec3"),
            (text.find("Color").unwrap(), "Color", "vec4"),
            (text.find("UV0").unwrap(), "UV0", "vec2"),
        ]
    );
}

/// Only real GLSL qualifiers may stand before `in`, `out` or `uniform`: a statement that has
/// another word there (an assignment, a `return`) is no declaration, and each real
/// qualifier form still is one.
#[test]
fn a_statement_with_junk_before_the_storage_word_is_no_declaration() {
    let junk = "x = in vec2 y;\nreturn out vec4 c;\nreturn uniform sampler2D s;\n\
        foo(a) uniform image2D img;\nint k in vec2 z;\n";
    assert_eq!(resources(junk), []);
    assert_eq!(stage_inputs(junk), []);
    let real = "flat in uint a;\nsmooth in vec2 b;\nnoperspective in vec2 c;\n\
        centroid in vec2 d;\nsample in vec2 e;\ninvariant out vec4 f;\n\
        precise out vec4 g;\nlayout(location = 2) flat in int h;\n\
        highp uniform sampler2D S;\nlowp in vec2 i;\nmediump in vec2 j;\n\
        layout(rgba8) coherent volatile restrict writeonly uniform image2D Img;\n";
    assert_eq!(
        stage_inputs(real),
        [
            (real.find("a;").unwrap(), "a", "uint"),
            (real.find("b;").unwrap(), "b", "vec2"),
            (real.find("c;").unwrap(), "c", "vec2"),
            (real.find("d;").unwrap(), "d", "vec2"),
            (real.find("e;").unwrap(), "e", "vec2"),
            (real.find("h;").unwrap(), "h", "int"),
            (real.find("i;").unwrap(), "i", "vec2"),
            (real.find("j;").unwrap(), "j", "vec2"),
        ]
    );
    assert_eq!(
        resources(real),
        [
            Resource {
                name: "S",
                kind: ResourceKind::Sampler2D
            },
            Resource {
                name: "Img",
                kind: ResourceKind::StorageImage {
                    access: ImageAccess::WriteOnly
                }
            },
        ]
    );
}

/// A preprocessor line is no declaration, whatever words it holds: the classifier lists
/// nothing from it, and the translator leaves it as it is (a loose `uniform` declaration
/// of an unknown name would be refused if it read as one).
#[test]
fn a_preprocessor_line_is_no_declaration() {
    let text = "#version 330\n#define IN in vec3 X;\n#define U uniform float Nope;\n\
        #define S uniform sampler2D T;\nvoid main() { }\n";
    assert_eq!(resources(text), []);
    assert_eq!(stage_inputs(text), []);
    let read = |_: &str| -> Result<Option<String>, String> { Ok(None) };
    let includes = includes(&read);
    let inputs = Inputs {
        blocks: &BLOCKS,
        samplers: &SAMPLERS,
        extra_set: 1,
        extra_blocks: &[],
        extra_samplers: &[],
        extra_texel_buffers: &[],
        extra_storage_buffers: &[],
        extra_storage_images: &[],
        push_constant_binding: None,
        loose: &LOOSE,
        defines: &[],
        includes: &includes,
    };
    let expanded = expand_includes(text, VSH_FILE, "minecraft:shaders/core/", &includes, None)
        .unwrap_or_else(|e| panic!("{e}"));
    let rewritten = rewrite(
        &expanded,
        ShaderStage::Vertex,
        &inputs,
        &GLUE,
        &mut Names::new(10),
    )
    .unwrap_or_else(|e| panic!("{e}"));
    let rewritten = rewritten.text();
    for line in text.lines().skip(1).take(3) {
        assert!(rewritten.contains(&format!("{line}\n")), "{rewritten}");
    }
}

/// GLSL takes a precision word after the storage word too (`in highp vec4 X;`): such a
/// line is a declaration of its type and name, as with the word before it.
#[test]
fn a_precision_word_after_the_storage_word_is_a_declaration() {
    let text = "#version 330\nin highp vec4 X;\nuniform lowp sampler2D T;\nvoid main() { }\n";
    assert_eq!(
        stage_inputs(text),
        [(text.find("X;").unwrap(), "X", "vec4")]
    );
    assert_eq!(
        resources(text),
        [Resource {
            name: "T",
            kind: ResourceKind::Sampler2D
        }]
    );
}

/// Only a storage buffer's declaration names a storage block, with its memory qualifiers.
#[test]
fn storage_blocks_are_read_from_buffer_declarations_only() {
    let block = storage_of("layout(std430) readonly buffer Quads {").unwrap();
    assert_eq!(
        (block.name, block.qualifiers, block.tail),
        ("Quads", vec!["readonly"], "{")
    );
    let block = storage_of("buffer Free { uint slots[]; };").unwrap();
    assert_eq!((block.name, block.qualifiers.len()), ("Free", 0));
    assert_eq!(storage_of("buffer Next").unwrap().tail, "");
    assert_eq!(storage_of("uniform isamplerBuffer Faces;"), None);
    assert_eq!(storage_of("uniform Day {"), None);
    assert_eq!(storage_of("vec4 buffer;"), None);
    assert_eq!(storage_of("uint data[];"), None);
    assert_eq!(
        storage_of("layout(std430, binding = 3) buffer Q {")
            .unwrap()
            .layout,
        "std430, binding = 3"
    );
}

fn storage_stages<'a>(vertex: &'a str, fragment: &'a str) -> Stages<'a> {
    Stages {
        vertex_dir: "minecraft:shaders/core/",
        fragment_dir: "minecraft:shaders/core/",
        vertex,
        fragment,
        vertex_file: VSH_FILE,
        fragment_file: FSH_FILE,
    }
}

fn translate_storage(vertex: &str) -> Result<Translated, Vec<Error>> {
    let fragment = "#version 330\nin vec4 vertexColor;\nout vec4 fragColor;\n\
        void main() { fragColor = vertexColor; }\n";
    let read = |_: &str| -> Result<Option<String>, String> { Ok(None) };
    let includes = includes(&read);
    let inputs = Inputs {
        blocks: &BLOCKS,
        samplers: &SAMPLERS,
        extra_set: 1,
        extra_blocks: &[],
        extra_samplers: &[],
        extra_texel_buffers: &[],
        extra_storage_buffers: &[("Data", 0, true)],
        extra_storage_images: &[],
        push_constant_binding: None,
        loose: &LOOSE,
        defines: &[],
        includes: &includes,
    };
    translate(&storage_stages(vertex, fragment), &inputs, &GLUE, &|_| {
        false
    })
}

/// A storage block is laid out std430 (GLSL 4.60 §7.6.2.2): `vec3` has base alignment 16 and
/// size 12, so `{ vec3 a; float b; vec3 c[]; }` puts `b` in `a`'s padding at 12 and `c` at 16,
/// and an array's stride is its element's alignment (16 for `vec3`), not rounded up as std140
/// rounds a struct.
#[test]
fn a_storage_block_is_laid_out_std430() {
    let vsh = "#version 330\nlayout(std430) readonly buffer Data { vec3 a; float b; vec3 c[]; };\n\
        out vec4 vertexColor;\n\
        void main() { vertexColor = vec4(a + c[0], b); gl_Position = vec4(0.0); }\n";
    let program = translate_storage(vsh).unwrap_or_else(|e| panic!("{e:?}"));
    let module = &program.vertex;
    let (_, global) = module
        .global_variables
        .iter()
        .find(|(_, g)| matches!(g.space, naga::AddressSpace::Storage { .. }))
        .expect("the storage block");
    let naga::TypeInner::Struct { members, .. } = &module.types[global.ty].inner else {
        panic!("a storage block is a struct");
    };
    let offsets: Vec<u32> = members.iter().map(|m| m.offset).collect();
    assert_eq!(offsets, [0, 12, 16]);
    let naga::TypeInner::Array { stride, .. } = module.types[members[2].ty].inner else {
        panic!("`c` is an array");
    };
    assert_eq!(stride, 16);
}

/// An invalid member name points inside the storage block, not to its rewritten layout.
#[test]
fn an_error_in_a_rewritten_storage_block_names_the_member_column() {
    let vsh = "#version 330\nlayout(std430) readonly buffer Data { vec4 in; };\n\
        out vec4 vertexColor;\n\
        void main() { vertexColor = vec4(1.0); gl_Position = vec4(0.0); }\n";
    let errors = translate_storage(vsh).unwrap_err();
    let error = &errors[0];
    assert!(error.message.ends_with("found In"), "{errors:?}");
    assert_eq!(
        (error.file.as_str(), error.line, error.column),
        (VSH_FILE, 2, 44)
    );
}

/// A storage block that names another packing is refused, not rewritten to std430.
#[test]
fn a_storage_block_naming_another_packing_is_refused() {
    for packing in ["std140", "shared", "packed"] {
        let vsh = format!(
            "#version 330\nlayout({packing}) readonly buffer Data {{ vec4 a[]; }};\n\
            out vec4 vertexColor;\n\
            void main() {{ vertexColor = a[0]; gl_Position = vec4(0.0); }}\n"
        );
        let errors = translate_storage(&vsh).expect_err(packing);
        assert!(!errors.is_empty(), "got: {errors:?}");
    }
}

/// The resources of a stage are each declaration `rewrite` binds, whatever else is on its
/// line and whatever qualifier stands before `uniform`, and nothing else.
#[test]
fn resources_lists_each_declaration_the_translator_binds() {
    let text = "\
layout(std140) uniform Day { float daylight; };
layout(push_constant) uniform Push { vec4 offset; };
highp uniform sampler2D Tex;
uniform sampler2D TexA; uniform sampler2D TexB;
uniform sampler2D TexC; uniform Scene { vec4 tint; };
uniform sampler2D TexD; layout(std430) readonly buffer Data { vec4 d[]; };
uniform isamplerBuffer Faces;
uniform usamplerBuffer Ids;
buffer Free { uint slots[]; };
layout(rgba16f) writeonly uniform image2D Out;
layout(r32f) uniform readonly image2D In;
uniform image2D Both;
uniform image3D Volume;
uniform float loose;
uniform sampler2D A, B;
in vec2 uv;
void main() { }
";
    let found: Vec<(&str, ResourceKind)> =
        resources(text).iter().map(|r| (r.name, r.kind)).collect();
    use ResourceKind::*;
    assert_eq!(
        found,
        [
            ("Day", UniformBlock),
            ("Push", PushConstantBlock),
            ("Tex", Sampler2D),
            ("TexA", Sampler2D),
            ("TexB", Sampler2D),
            ("TexC", Sampler2D),
            ("Scene", UniformBlock),
            ("TexD", Sampler2D),
            ("Data", StorageBuffer { read_only: true }),
            ("Faces", TexelBuffer),
            ("Ids", TexelBuffer),
            ("Free", StorageBuffer { read_only: false }),
            (
                "Out",
                StorageImage {
                    access: ImageAccess::WriteOnly
                }
            ),
            (
                "In",
                StorageImage {
                    access: ImageAccess::ReadOnly
                }
            ),
            (
                "Both",
                StorageImage {
                    access: ImageAccess::ReadWrite
                }
            ),
        ]
    );
}

/// An array too large to lay out (#4134) is the shader's error at its line, not a panic.
#[test]
fn an_array_too_large_to_lay_out_is_an_error() {
    let vsh = "#version 330\nout vec4 vertexColor;\nvoid main() { vertexColor = vec4(1.0); }\n";
    let fsh = "#version 330\nin vec4 vertexColor;\nfloat big[0x7fffffff];\nout vec4 fragColor;\n\
        void main() { fragColor = vertexColor; }\n";
    let error = translate_core(vsh, fsh, &[], &include, &GLUE).unwrap_err();
    assert_eq!((error[0].file.as_str(), error[0].line), (FSH_FILE, 3));
    assert!(
        error[0].message.starts_with("(float big[0x7fffffff];)"),
        "{error:?}"
    );
}

/// An output array whose second element would take location 2^32 (#4912) is the shader's
/// error at its line, not an overflow.
#[test]
fn an_output_whose_locations_pass_u32_is_an_error() {
    let vsh = "#version 330\nout vec4 vertexColor;\nvoid main() { vertexColor = vec4(1.0); }\n";
    let fsh = "#version 330\nin vec4 vertexColor;\nout vec4 fragColor;\n\
        layout(location = 4294967295u) out vec4 extra[2];\n\
        void main() { fragColor = vertexColor; extra[0] = vertexColor; extra[1] = vertexColor; }\n";
    let error = translate_core(vsh, fsh, &[], &include, &GLUE).unwrap_err();
    assert_eq!((error[0].file.as_str(), error[0].line), (FSH_FILE, 4));
    assert!(
        error[0]
            .message
            .starts_with("(layout(location = 4294967295u) out vec4 extra[2];)"),
        "{error:?}"
    );
}

#[test]
fn what_sunrise_does_not_provide_is_an_error() {
    let fsh = "#version 330\nin vec4 vertexColor;\nout vec4 fragColor;\nvoid main() { fragColor = vertexColor; }\n";
    // Another format's input is a global the shader may read: not an error.
    let normal = "#version 330\nin vec3 Normal;\nout vec4 vertexColor;\nvoid main() { vertexColor = vec4(Normal, 1.0); }\n";
    translate_core(normal, fsh, &[], &include, &GLUE).unwrap();
    let lightmap = "#version 330\nuniform sampler2D Lightmap;\nout vec4 vertexColor;\nvoid main() { vertexColor = texelFetch(Lightmap, ivec2(0), 0); }\n";
    let error = translate_core(lightmap, fsh, &[], &include, &GLUE).unwrap_err();
    assert_eq!(
        error,
        vec![Error {
            file: VSH_FILE.into(),
            line: 2,
            column: 1,
            stage: Some(ShaderStage::Vertex),
            unprovided: Some("Lightmap".into()),
            message: "Lightmap is not a block or sampler of this pipeline".into(),
        }]
    );
    // Behind a define the pipeline doesn't have, it is no error.
    let oit = "#version 330\n#ifdef OIT\nuniform sampler2D Coeff0;\n#endif\nout vec4 vertexColor;\n\
        void main() {\n#ifdef OIT\n    vertexColor = texelFetch(Coeff0, ivec2(0), 0);\n#endif\n}\n";
    translate_core(oit, fsh, &[], &include, &GLUE).unwrap();
    let missing = "#version 330\n#moj_import <minecraft:nothing.glsl>\n\
        out vec4 vertexColor;\nvoid main() { vertexColor = vec4(1.0); }\n";
    // At the line that imports it.
    let error = translate_core(missing, fsh, &[], &include, &GLUE).unwrap_err();
    assert_eq!(
        error,
        vec![Error {
            file: VSH_FILE.into(),
            line: 2,
            column: 0,
            stage: Some(ShaderStage::Vertex),
            unprovided: None,
            message: "minecraft:shaders/include/nothing.glsl not found".into(),
        }]
    );
    // An include the defines drop is never read, as shaderc preprocesses: missing, it is no
    // error; kept by a define, it is.
    let branch = "#version 330\n#ifdef WANTS\n#moj_import <minecraft:nothing.glsl>\n#endif\n\
        out vec4 vertexColor;\nvoid main() { vertexColor = vec4(1.0); }\n";
    translate_core(branch, fsh, &[], &include, &GLUE).unwrap();
    let error = translate_core(branch, fsh, &["WANTS"], &include, &GLUE).unwrap_err();
    assert_eq!(
        error,
        vec![Error {
            file: VSH_FILE.into(),
            line: 3,
            column: 0,
            stage: Some(ShaderStage::Vertex),
            unprovided: None,
            message: "minecraft:shaders/include/nothing.glsl not found".into(),
        }]
    );
    let active_after_skipped =
        branch.replace("#endif", "#endif\n#moj_import <minecraft:nothing.glsl>");
    let error = translate_core(&active_after_skipped, fsh, &[], &include, &GLUE).unwrap_err();
    assert_eq!(
        error,
        vec![Error {
            file: VSH_FILE.into(),
            line: 5,
            column: 0,
            stage: Some(ShaderStage::Vertex),
            unprovided: None,
            message: "minecraft:shaders/include/nothing.glsl not found".into(),
        }]
    );
    let unreadable = |id: &str| -> Result<Option<String>, String> {
        if id.ends_with("nothing.glsl") {
            Err(format!("{id}: not text"))
        } else {
            Ok(include(id))
        }
    };
    let stages = Stages {
        vertex_dir: "minecraft:shaders/core/",
        fragment_dir: "minecraft:shaders/core/",
        vertex: branch,
        fragment: fsh,
        vertex_file: VSH_FILE,
        fragment_file: FSH_FILE,
    };
    translate_core_at(&stages, &[], &unreadable, &GLUE, &|_| false).unwrap();
    let error = translate_core_at(&stages, &["WANTS"], &unreadable, &GLUE, &|_| false).unwrap_err();
    assert_eq!(
        error,
        vec![Error {
            file: VSH_FILE.into(),
            line: 3,
            column: 0,
            stage: Some(ShaderStage::Vertex),
            unprovided: None,
            message: "minecraft:shaders/include/nothing.glsl: not text".into(),
        }]
    );
}

#[test]
fn unconditional_imports_share_their_dependency() {
    let large = " ".repeat(MAX_EXPANDED / 2 + 1);
    let calls = std::cell::Cell::new(0);
    let read = |id: &str| {
        if id.ends_with("/shared") {
            calls.set(calls.get() + 1);
            Ok(Some(large.clone()))
        } else {
            Ok(Some("#moj_import <shared>".to_owned()))
        }
    };
    expand_core_imports("#moj_import <left>\n#moj_import <right>", "", &[], &read).unwrap();
    assert_eq!(calls.get(), 1);
    calls.set(0);
    expand_core_imports(
        "#ifdef ENABLED\n#moj_import <left>\n#moj_import <right>\n#endif",
        "",
        &["ENABLED"],
        &read,
    )
    .unwrap();
    assert_eq!(calls.get(), 1);
    let repeated = expand_core_imports("#include <shared>\n#include <shared>", "", &[], &read);
    assert_eq!(repeated.unwrap_err(), ImportRefusal::TooLarge);
}

#[test]
fn includes_follow_glsl_line_numbers_and_macros_from_the_caller_and_child() {
    let read = |id: &str| match id.rsplit('/').next() {
        Some("switch") => Ok(Some("#define ENABLE(x) ((x) + 2)".into())),
        Some("active") => Ok(Some("float kept;".into())),
        _ => panic!("read skipped include {id}"),
    };
    let source = "#version 330\n#if __LINE__ == 2\n#include <switch>\n#endif\n#if __LINE__ == 5 && ENABLE(3) == 5\n#include <active>\n#else\n#include <skipped>\n#endif";
    assert!(
        expand_core_imports(source, "", &[], &read)
            .unwrap()
            .contains("float kept;")
    );
}

#[test]
fn line_directives_and_continuations_keep_the_callers_include_location() {
    let source = "#line 40\n#if defined(__LINE__) && \\\n__LINE__ == 41\n#include <inner>\n#endif\n#if __LINE__ == 46\n#include <wrong>\n#endif";
    let read = |id: &str| {
        assert!(id.ends_with("/inner"), "{id}");
        Ok(Some("#if __LINE__ == 1\nfloat included;\n#endif".into()))
    };
    assert!(
        expand_core_imports(source, "", &[], &read)
            .unwrap()
            .contains("float included;")
    );
}

#[test]
fn independent_guards_do_not_replay_unrelated_definitions() {
    let source = format!(
        "#define UNUSED {}\n{}",
        "a".repeat(50_000),
        "#if 1\nfloat a;\n#endif\n".repeat(210)
    );
    assert_eq!(
        expand_core_imports(&source, "", &[], &|_| Ok(None))
            .unwrap()
            .matches("float a;")
            .count(),
        210
    );
}

#[test]
fn only_an_active_include_consumes_a_legacy_import() {
    let fragment = "out vec4 fragColor;\nvoid main() { fragColor = vec4(1.0); }";
    let include = |_: &str| Some("float imported() { return 1.0; }".to_owned());
    let siblings = "#if 0\n#moj_import <a>\n#elif 0\n#moj_import <a>\n#else\n#moj_import <a>\n#endif\nvoid main() { gl_Position = vec4(imported()); }";
    translate_core(siblings, fragment, &[], &include, &GLUE).unwrap();
    for first in ["#include", "#moj_import"] {
        let vertex = format!(
            "#define hz_moj_import_0\n#if 0\n#moj_import <a>\n#endif\n{first} <a>\n#moj_import <a>\nvoid main() {{ gl_Position = vec4(imported()); }}"
        );
        translate_core(&vertex, fragment, &[], &include, &GLUE).unwrap();
    }
}

#[test]
fn includes_resolve_by_id_or_from_the_including_folder() {
    assert_eq!(
        include_id(
            "<fog.glsl>",
            "minecraft:shaders/core/",
            &includes(&|_| Ok(None))
        )
        .as_deref(),
        Some("minecraft:shaders/include/fog.glsl")
    );
    assert_eq!(
        include_id(
            "<bitmade:hud/lanes.glsl>",
            "minecraft:shaders/core/",
            &includes(&|_| Ok(None))
        )
        .as_deref(),
        Some("bitmade:shaders/include/hud/lanes.glsl")
    );
    assert_eq!(
        include_id(
            "\"../include/fog.glsl\"",
            "minecraft:shaders/core/",
            &includes(&|_| Ok(None))
        )
        .as_deref(),
        Some("minecraft:shaders/include/fog.glsl")
    );
}

#[test]
fn translate_supports_caller_high_bindings_and_custom_includes() {
    let read = |id: &str| -> Result<Option<String>, String> {
        match id {
            "demo:custom/inc/scale.glsl" => {
                Ok(Some("float scale_it(float v) { return v * 0.5; }\n".into()))
            }
            _ => Ok(None),
        }
    };
    let custom_includes = Includes {
        read: &read,
        namespace: "demo",
        folder: "custom/inc/",
        once: &[],
    };
    let inputs = Inputs {
        blocks: &[("Camera", 17), ("Params", 24)],
        samplers: &[("Albedo", 15)],
        extra_set: 0,
        extra_blocks: &[],
        extra_samplers: &[],
        extra_texel_buffers: &[],
        extra_storage_buffers: &[],
        extra_storage_images: &[],
        push_constant_binding: None,
        loose: &[],
        defines: &[],
        includes: &custom_includes,
    };
    let vsh = "#version 330\n\
        #include <scale.glsl>\n\
        layout(std140) uniform Camera { mat4 ViewProj; };\n\
        layout(std140) uniform Extra { float extra_v; };\n\
        out vec2 uv;\n\
        void main() {\n\
            uv = vec2(scale_it(extra_v), 0.5);\n\
            gl_Position = ViewProj * vec4(0.0, 0.0, 0.0, 1.0);\n\
        }\n";
    let fsh = "#version 330\n\
        layout(std140) uniform Params { vec4 Tint; };\n\
        uniform sampler2D Albedo;\n\
        uniform sampler2D ExtraTex;\n\
        in vec2 uv;\n\
        out vec4 fragColor;\n\
        void main() {\n\
            fragColor = Tint * texture(Albedo, uv) * texture(ExtraTex, uv);\n\
        }\n";
    let stages = Stages {
        vertex_dir: "demo:custom/core/",
        fragment_dir: "demo:custom/core/",
        vertex: vsh,
        fragment: fsh,
        vertex_file: "demo:custom/core/test.vsh",
        fragment_file: "demo:custom/core/test.fsh",
    };
    let provided = |name: &str| name == "Extra" || name == "ExtraTex";
    let program = translate(&stages, &inputs, &GLUE, &provided)
        .expect("caller bindings >= 15 and provided others must translate");
    assert_eq!(program.other, ["Extra", "ExtraTex"]);
    // Highest caller binding is Params at 24 (Albedo uses 15 and 16, Camera uses 17),
    // so `other` starts at 25 without colliding with any caller binding.
    assert_eq!(program.other_binding(0), 25);
    assert_eq!(program.other_binding(1), 27);
    assert_eq!(
        program.stages_reading(15),
        wgpu_types::ShaderStages::FRAGMENT
    );
    assert_eq!(
        program.stages_reading(16),
        wgpu_types::ShaderStages::FRAGMENT
    );
    assert_eq!(program.stages_reading(17), wgpu_types::ShaderStages::VERTEX);
    assert_eq!(
        program.stages_reading(24),
        wgpu_types::ShaderStages::FRAGMENT
    );
    assert_eq!(program.stages_reading(25), wgpu_types::ShaderStages::VERTEX);
    assert_eq!(
        program.stages_reading(27),
        wgpu_types::ShaderStages::FRAGMENT
    );

    // An unprovided extra block is still refused by its real name.
    let err = translate(&stages, &inputs, &GLUE, &|_| false).unwrap_err();
    assert_eq!(
        err,
        vec![Error {
            file: "demo:custom/core/test.vsh".into(),
            line: 4,
            column: 24,
            stage: Some(ShaderStage::Vertex),
            unprovided: Some("Extra".into()),
            message: "Extra is not a block or sampler of this pipeline".into(),
        }]
    );

    // With `once: &[]`, `#moj_import` is not treated as an include directive.
    let untouched = expand_includes(
        "#moj_import <scale.glsl>\nfloat x;\n",
        "demo:custom/core/test.vsh",
        "demo:custom/core/",
        &custom_includes,
        None,
    )
    .unwrap();
    assert!(
        untouched.text().contains("#moj_import <scale.glsl>"),
        "{}",
        untouched.text()
    );
}

#[test]
fn set1_extra_bindings_and_push_constants_bind_into_group_1() {
    let read = |_: &str| -> Result<Option<String>, String> { Ok(None) };
    let inc = Includes {
        read: &read,
        namespace: "minecraft",
        folder: "shaders/include/",
        once: &[],
    };
    let inputs = Inputs {
        blocks: &BLOCKS,
        samplers: &[],
        extra_set: 1,
        extra_blocks: &[("ModParams", 0)],
        extra_samplers: &[("ModTex", 1)],
        extra_texel_buffers: &[("ModLut", 3)],
        extra_storage_buffers: &[],
        extra_storage_images: &[],
        push_constant_binding: Some(4),
        loose: &[],
        defines: &[],
        includes: &inc,
    };
    let vsh = "#version 330\n\
        layout(std140) uniform Projection { mat4 ProjMat; };\n\
        layout(push_constant) uniform PushData { vec4 Offset; };\n\
        uniform isamplerBuffer ModLut;\n\
        out vec2 uv;\n\
        void main() {\n\
            int idx = texelFetch(ModLut, 0).r;\n\
            uv = vec2(float(idx), 0.5);\n\
            gl_Position = ProjMat * Offset;\n\
        }\n";
    let fsh = "#version 330\n\
        layout(std140) uniform ModParams { vec4 Tint; };\n\
        uniform sampler2D ModTex;\n\
        in vec2 uv;\n\
        out vec4 fragColor;\n\
        void main() {\n\
            fragColor = Tint * texture(ModTex, uv);\n\
        }\n";
    let stages = Stages {
        vertex_dir: "demo:shaders/core/",
        fragment_dir: "demo:shaders/core/",
        vertex: vsh,
        fragment: fsh,
        vertex_file: VSH_FILE,
        fragment_file: FSH_FILE,
    };
    let glue = Glue {
        inputs: &[],
        prelude: "",
        setup: "",
        finish: "",
        vertex_id: "int(gl_VertexIndex)",
        fragment_finish: None,
        interface: LIMITS,
    };
    let translated = translate(&stages, &inputs, &glue, &|_| false)
        .expect("set-1 bindings and push_constant must translate");
    assert!(translated.other.is_empty());
    assert_eq!(
        translated.stages_reading_in_group(1, 0),
        wgpu_types::ShaderStages::FRAGMENT
    );
    assert_eq!(
        translated.stages_reading_in_group(1, 1),
        wgpu_types::ShaderStages::FRAGMENT
    );
    assert_eq!(
        translated.stages_reading_in_group(1, 2),
        wgpu_types::ShaderStages::FRAGMENT
    );
    assert_eq!(
        translated.stages_reading_in_group(1, 3),
        wgpu_types::ShaderStages::VERTEX
    );
    assert_eq!(
        translated.stages_reading_in_group(1, 4),
        wgpu_types::ShaderStages::VERTEX
    );
    assert_eq!(translated.uniform_block_size(1, 4), Some(16));

    // Without push_constant_binding, layout(push_constant) is refused.
    let no_pc = Inputs {
        push_constant_binding: None,
        ..inputs
    };
    let err = translate(&stages, &no_pc, &glue, &|_| false)
        .expect_err("undeclared push_constant must fail");
    assert!(describe(&err).contains("push_constant"), "got: {err:?}");
}

fn compute_uses_subgroup(source: &str) -> bool {
    let read = |_: &str| -> Result<Option<String>, String> { Ok(None) };
    let includes = includes(&read);
    let inputs = Inputs {
        blocks: &[],
        samplers: &[],
        extra_set: 0,
        extra_blocks: &[],
        extra_samplers: &[],
        extra_texel_buffers: &[],
        extra_storage_buffers: &[("Out", 0, false)],
        extra_storage_images: &[],
        push_constant_binding: None,
        loose: &[],
        defines: &[],
        includes: &includes,
    };
    translate_compute(source, "t:test.csh", "", &inputs, LIMITS)
        .unwrap_or_else(|e| panic!("{e:?}"))
        .uses_subgroup
}

/// Subgroup use is read from the translated module, not the source text: an identifier that
/// merely starts with `subgroup` is not an operation.
#[test]
fn only_a_real_subgroup_operation_sets_uses_subgroup() {
    let body = "layout(local_size_x = 8) in;\nlayout(std430) buffer Out { uint values[]; };\n";
    let named = format!(
        "#version 450\n{body}void main() {{ uint subgroupCount = 3u; values[gl_GlobalInvocationID.x] = subgroupCount; }}\n"
    );
    assert!(!compute_uses_subgroup(&named));
    let built_in = format!(
        "#version 450\n#extension GL_KHR_shader_subgroup_basic : enable\n{body}void main() {{ values[gl_GlobalInvocationID.x] = gl_SubgroupSize; }}\n"
    );
    assert!(compute_uses_subgroup(&built_in));
}
