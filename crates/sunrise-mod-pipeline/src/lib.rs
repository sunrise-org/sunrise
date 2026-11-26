//! Mod render pipelines' GLSL core shaders, translated and checked without a GPU on top of
//! [`sunrise_glsl`].
//!
//! A mod registers a `RenderPipeline` naming a core shader program (`example:core/glow` ->
//! `assets/example/shaders/core/glow.vsh` and `glow.fsh`), with an optional separate vertex
//! shader id (such as `minecraft:core/position_tex_color`), optional preprocessor defines, and
//! an optional sprite texture.
//!
//! Both the game's resource load (`sunrise-client-render`) and `cargo sunrise-mod check` run
//! through [`translate`], using the same vanilla-shaped [`BLOCKS`], [`TEXTURED_SAMPLERS`],
//! [`SPRITE_INCLUDE`], vertex [`Glue`](sunrise_glsl::Glue), and crash-containment bounds
//! ([`guard`]).
//!
//! Colour follows vanilla's gamma space: a mod program writes what vanilla core shaders write
//! (`texture * vertexColor * ColorModulator`). On frames storing vanilla's encoding, outputs are
//! stored as written; on frames holding linear light, [`linear_fragment_finish`] decodes the
//! fragment output after `main()` (sanitized against negatives/NaN, alpha clamped as coverage).

pub mod check;
pub mod guard;
pub mod stubs;

use crate::check::{Problem, located, problem, translate_backends};
use naga::front::glsl::InterfaceLimits;
use sunrise_glsl::{Glue, Includes, Inputs, ResourceKind, Stages, Translated};

/// The folder under `assets/<namespace>/` holding core shader programs (`.vsh` and `.fsh`).
pub const CORE_FOLDER: &str = "shaders/core";

/// The folder under `assets/<namespace>/` holding `<ns:path>` shader includes.
pub const INCLUDE_FOLDER: &str = "shaders/include/";

/// Entry point name of translated GLSL stages.
pub const ENTRY: &str = "main";

/// Backend define always provided to core shaders on a 0..1 depth backend (matching vanilla's
/// `GlslCompiler.createBaseShaderOptions` and `core_shader.rs`).
pub const DEPTH_ZERO_TO_ONE: &str = "RENDERPEARL_DEPTH_IS_ZERO_TO_ONE";

/// Resource id of the built-in sprite helper include (`#include <sunrise:pipeline_sprite.glsl>`).
pub const SPRITE_INCLUDE: &str = "sunrise:shaders/include/pipeline_sprite.glsl";

/// Set-0 binding of `SpriteRect` when a pipeline declares `PipelineTexture::Sprite`.
/// Placed above vanilla's `Sampler0..=2` slots (`4..=9`) so future samplers never collide.
pub const SPRITE_RECT_BINDING: u32 = 10;

/// Uniform blocks bound for an untextured mod pipeline in set 0.
pub const BLOCKS: [(&str, u32); 4] = [
    ("Globals", 0),
    ("Projection", 1),
    ("DynamicTransforms", 2),
    ("Fog", 3),
];

/// Uniform blocks bound for a sprite-textured mod pipeline in set 0.
pub const BLOCKS_WITH_SPRITE: [(&str, u32); 5] = [
    ("Globals", 0),
    ("Projection", 1),
    ("DynamicTransforms", 2),
    ("Fog", 3),
    ("SpriteRect", SPRITE_RECT_BINDING),
];

/// Samplers bound for a textured mod pipeline in set 0 (`Sampler0` at binding 4, sampler at 5).
pub const TEXTURED_SAMPLERS: [(&str, u32); 1] = [("Sampler0", 4)];

/// Samplers the OIT stages bind (`DepthBoundsSampler`, `Coeff0`, `Coeff1`), in the order
/// `Oit::cloud_samples` supplies their texture views.
pub const OIT_STAGE_SAMPLERS: [&str; 3] = ["DepthBoundsSampler", "Coeff0", "Coeff1"];

/// Preprocessor defines and extra samplers for one OIT sub-pass (`DepthBounds`, `Transmittance`,
/// `Accumulate`), matching vanilla 26.3's `RenderPipelines.OIT_*_SNIPPET`.
#[derive(Clone, Copy, Debug)]
pub struct OitStageSpec {
    /// Human-readable stage name for diagnostics.
    pub name: &'static str,
    /// Preprocessor defines appended after the pipeline's own defines.
    pub defines: &'static [&'static str],
    /// Extra sampler names allowed via `sunrise_glsl`'s `provided` callback for this stage.
    pub samplers: &'static [&'static str],
}

/// The three OIT stages in execution order (`DepthBounds`, `Transmittance`, `Accumulate`).
pub const OIT_STAGES: [OitStageSpec; 3] = [
    OitStageSpec {
        name: "depth_bounds",
        defines: &[
            "OIT",
            "OIT_WAVELET_RANK 2",
            "OIT_COEFF_COUNT 8",
            "OIT_COEFF_ATTACHMENT_COUNT 2",
            "OIT_ALPHA_ONLY",
            "OIT_DEPTH_BOUNDS",
        ],
        samplers: &[],
    },
    OitStageSpec {
        name: "transmittance",
        defines: &[
            "OIT",
            "OIT_WAVELET_RANK 2",
            "OIT_COEFF_COUNT 8",
            "OIT_COEFF_ATTACHMENT_COUNT 2",
            "OIT_ALPHA_ONLY",
            "OIT_TRANSMITTANCE",
        ],
        samplers: &["DepthBoundsSampler"],
    },
    OitStageSpec {
        name: "accumulate",
        defines: &[
            "OIT",
            "OIT_WAVELET_RANK 2",
            "OIT_COEFF_COUNT 8",
            "OIT_COEFF_ATTACHMENT_COUNT 2",
            "OIT_ACCUMULATE",
        ],
        samplers: &OIT_STAGE_SAMPLERS,
    },
];

/// Conservative stage interface limits used when checking shaders without a GPU device.
pub const DEVICELESS_INTERFACE_LIMITS: InterfaceLimits = InterfaceLimits {
    inter_stage_variables: 32,
    vertex_attributes: 32,
};

/// Built-in GLSL include `<sunrise:pipeline_sprite.glsl>` for `PipelineTexture::Sprite` pipelines:
/// provides `SpriteRect`, `Sampler0`, and `vec4 sampleSprite(vec2 uv)` matching [`repeat_texel`]
/// in vanilla's gamma space (nearest integer texel inside the sprite's atlas rect at mip 0,
/// repeating past 0..1 without bleeding neighbouring sprites).
pub const PIPELINE_SPRITE_GLSL: &str = "\
#ifndef SUNRISE_PIPELINE_SPRITE_GLSL
#define SUNRISE_PIPELINE_SPRITE_GLSL
layout(std140) uniform SpriteRect {
    vec4 SunriseSpriteRect;
};
uniform sampler2D Sampler0;
vec4 sampleSprite(vec2 uv) {
    vec2 size = vec2(textureSize(Sampler0, 0));
    vec2 origin = round(SunriseSpriteRect.xy * size);
    vec2 extent = max(round(SunriseSpriteRect.zw * size) - origin, vec2(1.0));
    vec2 local = clamp(floor((uv - floor(uv)) * extent), vec2(0.0), extent - vec2(1.0));
    return texelFetch(Sampler0, ivec2(origin + local), 0);
}
#endif
";

/// Post-`main()` fragment finish for frames holding linear light: sanitizes negative and NaN
/// RGB channels to 0 before applying vanilla's 2.2 gamma-to-linear power, and clamps alpha
/// coverage to `[0, 1]` (NaN covers nothing).
pub fn linear_fragment_finish(out_name: &str) -> String {
    format!(
        "vec3 hz_rgb = mix(vec3(0.0), {out_name}.rgb, greaterThan({out_name}.rgb, vec3(0.0)));\n\
         float hz_a = clamp({out_name}.a > 0.0 ? {out_name}.a : 0.0, 0.0, 1.0);\n\
         {out_name} = vec4(pow(hz_rgb, vec3(2.2)), hz_a);"
    )
}

/// Returns the GLSL output variable name bound to fragment `location = 0` in `module`, if any.
fn fragment_loc0_output_name(module: &naga::Module) -> Option<&str> {
    let ep = module
        .entry_points
        .iter()
        .find(|e| e.stage == naga::ShaderStage::Fragment)?;
    let res = ep.function.result.as_ref()?;
    if let Some(naga::Binding::Location { location: 0, .. }) = res.binding {
        // Trace the entry point's returned expression back to the GLSL output GlobalVariable.
        let ret_expr = ep.function.body.iter().rev().find_map(|stmt| match stmt {
            naga::Statement::Return { value: Some(expr) } => Some(*expr),
            _ => None,
        })?;
        let ptr = match ep.function.expressions[ret_expr] {
            naga::Expression::Load { pointer } => pointer,
            _ => ret_expr,
        };
        if let naga::Expression::GlobalVariable(gv) = ep.function.expressions[ptr] {
            return module.global_variables[gv].name.as_deref();
        }
        return None;
    }
    if let naga::TypeInner::Struct { ref members, .. } = module.types[res.ty].inner {
        for m in members {
            if let Some(naga::Binding::Location { location: 0, .. }) = m.binding {
                return m.name.as_deref();
            }
        }
    }
    None
}

/// `sampleSprite`'s texel arithmetic, for testing without a GPU: the integer texel of an atlas
/// `size` texels wide and high that `uv` picks inside the sprite at `rect` (`[u0, v0, u1, v1]`).
/// The fraction of `uv` (negative values wrap) spans the rect once; the texel is always one of
/// the rect's own, its far edge excluded.
pub fn repeat_texel(uv: [f32; 2], rect: [f32; 4], size: [u32; 2]) -> [u32; 2] {
    std::array::from_fn(|axis| {
        let size = size[axis] as f32;
        let origin = (rect[axis] * size).round();
        let extent = ((rect[axis + 2] * size).round() - origin).max(1.0);
        let f = uv[axis] - uv[axis].floor();
        let local = (f * extent).floor().clamp(0.0, extent - 1.0);
        (origin + local) as u32
    })
}

/// The vertex and fragment GLSL source texts and paths of one mod render pipeline program.
pub struct ProgramSources<'a> {
    /// Asset path of the vertex shader for diagnostics (e.g. `assets/mod/shaders/core/a.vsh`).
    pub vertex_file: &'a str,
    /// Include base folder for quoted relative includes in the vertex shader
    /// (e.g. `mod:shaders/core/`).
    pub vertex_dir: &'a str,
    pub vertex: &'a str,
    /// Asset path of the fragment shader for diagnostics (e.g. `assets/mod/shaders/core/a.fsh`).
    pub fragment_file: &'a str,
    /// Include base folder for quoted relative includes in the fragment shader.
    pub fragment_dir: &'a str,
    pub fragment: &'a str,
}

/// Default vertex inputs (`PipelineVertexFormat::PositionTexColor`) in deterministic
/// sequential location order (`0..3`).
pub const DEFAULT_VERTEX_INPUTS: [(&str, &str); 3] =
    [("Position", "vec3"), ("UV0", "vec2"), ("Color", "vec4")];

/// Bind-group set index for mod-declared `BindGroupLayout` entries (set 0 stays vanilla's shared
/// bindings from `BindGroupLayouts.java`).
pub const MOD_BIND_GROUP_SET: u32 = 1;

pub use mod_vocab::{MAX_PUSH_CONSTANT_SIZE, VANILLA_BIND_GROUP_LAYOUT_NAMES};

/// Resource kind of one mod-declared set-1 binding (`BindGroupLayout`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ModBindingKind {
    /// std140 uniform block (`binding = k`).
    UniformBuffer,
    /// Combined 2D image + sampler (`texture` at `k`, `sampler` at `k + 1`).
    CombinedImageSampler,
    /// Integer texel buffer (`isamplerBuffer`/`usamplerBuffer`, read-only storage at `k`).
    TexelBuffer,
    /// std430 storage buffer block (`buffer Name { ... }`, `binding = k`).
    StorageBuffer {
        /// Whether the block is `readonly`.
        read_only: bool,
    },
    /// `layout(<format>) uniform image2D` storage image (`binding = k`).
    StorageImage {
        /// What the declaration's `readonly`/`writeonly` qualifiers must say.
        access: sunrise_glsl::ImageAccess,
    },
}

/// One mod-declared set-1 binding passed into [`Options::mod_bindings`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ModBinding<'a> {
    pub name: &'a str,
    pub kind: ModBindingKind,
}

/// Deterministic set-1 binding indices assigned in declaration order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Set1Bindings<'a> {
    pub blocks: Vec<(&'a str, u32)>,
    pub samplers: Vec<(&'a str, u32)>,
    pub texel_buffers: Vec<(&'a str, u32)>,
    pub storage_buffers: Vec<(&'a str, u32, bool)>,
    pub storage_images: Vec<(&'a str, u32, sunrise_glsl::ImageAccess)>,
    pub push_constant_binding: Option<u32>,
    pub next_binding: u32,
}

/// Assign deterministic set-1 binding indices for `bindings` (in declaration order) followed by
/// an optional inline uniform slot for `push_constant_size > 0`.
pub fn assign_set1_bindings<'a>(
    bindings: &'a [ModBinding<'a>],
    push_constant_size: u32,
) -> Set1Bindings<'a> {
    let mut out = Set1Bindings::default();
    let mut next = 0u32;
    for b in bindings {
        match b.kind {
            ModBindingKind::UniformBuffer => {
                out.blocks.push((b.name, next));
                next += 1;
            }
            ModBindingKind::CombinedImageSampler => {
                out.samplers.push((b.name, next));
                next += 2;
            }
            ModBindingKind::TexelBuffer => {
                out.texel_buffers.push((b.name, next));
                next += 1;
            }
            ModBindingKind::StorageBuffer { read_only } => {
                out.storage_buffers.push((b.name, next, read_only));
                next += 1;
            }
            ModBindingKind::StorageImage { access } => {
                out.storage_images.push((b.name, next, access));
                next += 1;
            }
        }
    }
    if push_constant_size > 0 {
        out.push_constant_binding = Some(next);
        next += 1;
    }
    out.next_binding = next;
    out
}

/// The non-vanilla uniform blocks, `sampler2D`s, integer texel buffers, storage buffers and
/// storage images a mod shader pair declares, and the push-constant size its
/// `layout(push_constant)` block asks for ([`MAX_PUSH_CONSTANT_SIZE`], `0` without one), for
/// deviceless static checking (`cargo sunrise-mod check`), which has no `BindGroupLayout` to ask.
/// They are the declarations the translator binds, found by the same code
/// ([`sunrise_glsl::resources`]), each name once in the order the vertex stage then the fragment
/// stage first declares it. Reserved vanilla `BindGroupLayouts.java` names (plus `SpriteRect` when
/// `textured`) are never mod bindings, so undeclared vanilla names are still refused. The stages
/// are read as the translator reads them: with comments gone and includes inlined
/// ([`extract_program_bindings`]).
pub fn extract_mod_bindings(
    vertex: &str,
    fragment: &str,
    textured: bool,
) -> (Vec<(String, ModBindingKind)>, u32) {
    let mut out: Vec<(String, ModBindingKind)> = Vec::new();
    let mut push_constant_size = 0u32;
    let is_reserved = |name: &str| {
        VANILLA_BIND_GROUP_LAYOUT_NAMES.contains(&name) || (textured && name == "SpriteRect")
    };
    for resource in [vertex, fragment]
        .into_iter()
        .flat_map(sunrise_glsl::resources)
    {
        let kind = match resource.kind {
            ResourceKind::PushConstantBlock => {
                push_constant_size = MAX_PUSH_CONSTANT_SIZE;
                continue;
            }
            ResourceKind::UniformBlock => ModBindingKind::UniformBuffer,
            ResourceKind::Sampler2D => ModBindingKind::CombinedImageSampler,
            ResourceKind::TexelBuffer => ModBindingKind::TexelBuffer,
            ResourceKind::StorageBuffer { read_only } => {
                ModBindingKind::StorageBuffer { read_only }
            }
            ResourceKind::StorageImage { access } => ModBindingKind::StorageImage { access },
        };
        if !is_reserved(resource.name) && !out.iter().any(|(name, _)| name == resource.name) {
            out.push((resource.name.to_owned(), kind));
        }
    }
    (out, push_constant_size)
}

/// Compilation options for [`translate`].
#[derive(Clone, Copy)]
pub struct Options<'a> {
    /// Whether the pipeline declared `.textured(PipelineTexture::Sprite(...))`, exposing
    /// `Sampler0`, `SpriteRect`, and `#include <sunrise:pipeline_sprite.glsl>`.
    pub textured: bool,
    /// Pipeline defines (`NAME` or `NAME value`) in order; [`DEPTH_ZERO_TO_ONE`] is always
    /// prepended automatically.
    pub defines: &'a [&'a str],
    /// Declared vertex inputs `(name, glsl_type)` in deterministic sequential location order
    /// (`0..N`) across `(binding_index, attribute_index)`. Defaults to [`DEFAULT_VERTEX_INPUTS`].
    pub vertex_inputs: &'a [(&'a str, &'a str)],
    /// Mod-declared set-1 bindings (`BindGroupLayout`) in declaration order.
    pub mod_bindings: &'a [ModBinding<'a>],
    /// Declared push-constant block size in bytes (`0` = none, at most
    /// [`MAX_PUSH_CONSTANT_SIZE`] = 128 B).
    pub push_constant_size: u32,
    /// Whether the target frame stores linear light (attaching [`linear_fragment_finish`]).
    pub linear_light: bool,
    /// Also run SPIR-V, Metal and DirectX 12 backend translation checks (used by
    /// `cargo sunrise-mod check` and tests).
    pub check_backends: bool,
    /// Stage interface limits of the target device (or [`DEVICELESS_INTERFACE_LIMITS`]).
    pub interface: InterfaceLimits,
}

impl Default for Options<'_> {
    fn default() -> Self {
        Self {
            textured: false,
            defines: &[],
            vertex_inputs: &DEFAULT_VERTEX_INPUTS,
            mod_bindings: &[],
            push_constant_size: 0,
            linear_light: false,
            check_backends: false,
            interface: DEVICELESS_INTERFACE_LIMITS,
        }
    }
}

/// Result of [`translate`]: the validated naga program plus both stages' expanded GLSL text
/// (allowing a linear-light renderer to re-translate the fragment stage with
/// [`translate_linear_fragment`] without re-reading disk resources).
#[derive(Debug)]
pub struct BuiltProgram {
    pub translated: Translated,
    pub vertex_expanded: String,
    pub fragment_expanded: String,
}

/// Vertex & fragment `Glue` for `PipelineVertexFormat::PositionTexColor`.
pub fn pipeline_glue<'a>(interface: InterfaceLimits, fragment_finish: Option<&'a str>) -> Glue<'a> {
    Glue {
        inputs: &DEFAULT_VERTEX_INPUTS,
        prelude: "layout(location = 0) in vec3 hz_pos;\n\
                  layout(location = 1) in vec2 hz_uv;\n\
                  layout(location = 2) in vec4 hz_color;",
        setup: "Position = hz_pos; UV0 = hz_uv; Color = hz_color;",
        finish: "",
        vertex_id: "int(gl_VertexIndex)",
        fragment_finish,
        interface,
    }
}

/// Build the `prelude` and `setup` snippets for an arbitrary list of `(name, glsl_type)` vertex
/// inputs assigned deterministic sequential locations `0..N`.
fn build_owned_glue(vertex_inputs: &[(&str, &str)]) -> (String, String) {
    use std::fmt::Write as _;
    let mut prelude = String::new();
    let mut setup = String::new();
    for (loc, (name, ty)) in vertex_inputs.iter().enumerate() {
        writeln!(prelude, "layout(location = {loc}) in {ty} hz_attr_{loc};").unwrap();
        write!(setup, "{name} = hz_attr_{loc}; ").unwrap();
    }
    (prelude, setup)
}

/// The glue for `options.vertex_inputs` over the snippets from [`build_owned_glue`].
fn owned_glue<'a>(
    options: &Options<'a>,
    prelude: &'a str,
    setup: &'a str,
    fragment_finish: Option<&'a str>,
) -> Glue<'a> {
    Glue {
        inputs: options.vertex_inputs,
        prelude,
        setup,
        finish: "",
        vertex_id: "int(gl_VertexIndex)",
        fragment_finish,
        interface: options.interface,
    }
}

/// Whether `ty` is a GLSL scalar/vector type producible by `VertexAttributeFormat::glsl_type`.
fn is_supported_vertex_glsl_type(ty: &str) -> bool {
    matches!(
        ty,
        "float"
            | "vec2"
            | "vec3"
            | "vec4"
            | "int"
            | "ivec2"
            | "ivec3"
            | "ivec4"
            | "uint"
            | "uvec2"
            | "uvec3"
            | "uvec4"
    )
}

/// Parse top-level vertex `in <type> <name>;` declarations from preprocessed GLSL source,
/// returning `(byte_offset, name, glsl_type)`.
fn parse_vertex_in_decls(text: &str) -> Vec<(usize, &str, &str)> {
    let mut out = Vec::new();
    let mut brace_depth = 0u32;
    let mut in_block_comment = false;
    let mut line_start = 0usize;
    for raw_line in text.split_inclusive('\n') {
        let offset = line_start;
        line_start += raw_line.len();
        let mut line = raw_line.trim();
        if in_block_comment {
            if let Some(end) = line.find("*/") {
                in_block_comment = false;
                line = line[end + 2..].trim();
            } else {
                continue;
            }
        }
        if let Some(start) = line.find("/*") {
            if let Some(end) = line[start + 2..].find("*/") {
                // Single-line block comment; ignore only if it's the whole line.
                if start == 0 && start + 2 + end + 2 >= line.len() {
                    continue;
                }
            } else {
                in_block_comment = true;
                line = line[..start].trim();
            }
        }
        if let Some(comment) = line.find("//") {
            line = line[..comment].trim();
        }
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let open_braces = line.bytes().filter(|&b| b == b'{').count() as u32;
        let close_braces = line.bytes().filter(|&b| b == b'}').count() as u32;
        let top_level = brace_depth == 0 && open_braces == 0 && close_braces == 0;
        brace_depth = brace_depth
            .saturating_add(open_braces)
            .saturating_sub(close_braces);
        if !top_level {
            continue;
        }
        for stmt in line.split(';') {
            let mut rest = stmt.trim();
            if rest.is_empty() {
                continue;
            }
            // Strip optional `layout(...)`.
            if let Some(after_layout) = rest.strip_prefix("layout") {
                let trimmed = after_layout.trim_start();
                if let Some(after_open) = trimmed.strip_prefix('(')
                    && let Some(close) = after_open.find(')')
                {
                    rest = after_open[close + 1..].trim_start();
                }
            }
            let tokens = rest.split_whitespace();
            let mut saw_in = false;
            let mut ty = None;
            let mut name = None;
            for tok in tokens {
                if !saw_in {
                    if tok == "in" {
                        saw_in = true;
                    } else if matches!(
                        tok,
                        "flat"
                            | "smooth"
                            | "noperspective"
                            | "centroid"
                            | "sample"
                            | "invariant"
                            | "highp"
                            | "mediump"
                            | "lowp"
                    ) {
                        continue;
                    } else {
                        break;
                    }
                } else if ty.is_none() {
                    if matches!(tok, "highp" | "mediump" | "lowp") {
                        continue;
                    }
                    ty = Some(tok);
                } else if name.is_none() {
                    name = Some(tok);
                    break;
                }
            }
            if let (Some(ty), Some(name)) = (ty, name) {
                // Skip function prototypes or array-syntax edge cases only if invalid ident.
                if name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') {
                    out.push((offset, name, ty));
                }
            }
        }
    }
    out
}

/// Extract declared vertex `in` attributes `(name, glsl_type)` from expanded vertex GLSL for
/// standalone tooling (`cargo sunrise-mod check`) that validates `.vsh` files without running
/// the mod's Rust registration code.
pub fn extract_vertex_inputs(vertex_expanded: &str) -> Result<Vec<(String, String)>, String> {
    let decls = parse_vertex_in_decls(vertex_expanded);
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::with_capacity(decls.len());
    for (_, name, ty) in decls {
        if !is_supported_vertex_glsl_type(ty) {
            return Err(format!(
                "unsupported vertex input type `{ty}` for `{name}`: expected float/vecN/int/ivecN/uint/uvecN"
            ));
        }
        if !seen.insert(name) {
            return Err(format!("duplicate vertex input `{name}`"));
        }
        out.push((name.to_owned(), ty.to_owned()));
        if out.len() > 16 {
            return Err("vertex shader declares more than 16 vertex inputs".to_owned());
        }
    }
    Ok(out)
}

/// Refuse any top-level vertex `in` declaration that is not listed in `vertex_inputs` or whose
/// GLSL type disagrees with the pipeline's [`VertexFormat`].
fn validate_vertex_inputs(
    vertex_file: &str,
    vertex_expanded: &str,
    vertex_inputs: &[(&str, &str)],
) -> Result<(), Problem> {
    for (offset, name, ty) in parse_vertex_in_decls(vertex_expanded) {
        match vertex_inputs.iter().find(|(n, _)| *n == name) {
            None => {
                return Err(located(
                    vertex_file,
                    vertex_expanded,
                    Some(offset),
                    format!(
                        "undeclared vertex input `{name}`: declare it in RenderPipeline::vertex_binding"
                    ),
                ));
            }
            Some((_, expected)) if *expected != ty => {
                return Err(located(
                    vertex_file,
                    vertex_expanded,
                    Some(offset),
                    format!(
                        "vertex input `{name}` declared as `{ty}` in shader, expected `{expected}` from VertexFormat"
                    ),
                ));
            }
            Some(_) => {}
        }
    }
    Ok(())
}

/// [`extract_mod_bindings`] over a program's two stages with their `#include`s inlined and
/// comments removed under `defines`, as [`translate`] reads them: a uniform block, sampler or
/// buffer an included file declares is the program's too (#5905), and one named only in a comment
/// is not. A stage whose includes cannot be expanded is scanned as written; [`translate`] then
/// refuses the program and reports why.
pub fn extract_program_bindings(
    sources: &ProgramSources<'_>,
    textured: bool,
    defines: &[&str],
    include: &dyn Fn(&str) -> Result<Option<String>, String>,
) -> (Vec<(String, ModBindingKind)>, u32) {
    let read = |id: &str| -> Result<Option<String>, String> {
        if id == SPRITE_INCLUDE {
            return if textured {
                Ok(Some(PIPELINE_SPRITE_GLSL.to_owned()))
            } else {
                Err(format!("{SPRITE_INCLUDE} requires a sprite texture"))
            };
        }
        include(id)
    };
    let includes = Includes {
        read: &read,
        namespace: "minecraft",
        folder: INCLUDE_FOLDER,
        once: &["moj_import"],
    };
    let defines = effective_defines(defines);
    let expand = |text: &str, file: &str, dir: &str| {
        sunrise_glsl::expand_includes(text, file, dir, &includes, Some(&defines))
            .map_or_else(|_| text.to_owned(), |expanded| expanded.text().to_owned())
    };
    let vertex = expand(sources.vertex, sources.vertex_file, sources.vertex_dir);
    let fragment = expand(
        sources.fragment,
        sources.fragment_file,
        sources.fragment_dir,
    );
    extract_mod_bindings(&vertex, &fragment, textured)
}

/// Prepend [`DEPTH_ZERO_TO_ONE`] to caller-supplied defines.
fn effective_defines<'a>(defines: &[&'a str]) -> Vec<&'a str> {
    std::iter::once(DEPTH_ZERO_TO_ONE)
        .chain(defines.iter().copied())
        .collect()
}

/// Translate and validate one mod render pipeline GLSL program (`.vsh` + `.fsh`) on a guarded
/// 64 MiB stack thread with crash/memory containment bounds ([`guard`]).
pub fn translate(
    sources: &ProgramSources<'_>,
    options: &Options<'_>,
    include: &(dyn Fn(&str) -> Result<Option<String>, String> + Sync),
) -> Result<BuiltProgram, Vec<Problem>> {
    let defines = effective_defines(options.defines);
    guard::guarded(|| translate_inner(sources, options, &defines, &[], include))
        .unwrap_or_else(|reason| Err(vec![problem(sources.fragment_file, reason)]))
}

/// The GLSL source text and path of one mod compute shader (`.csh`).
pub struct ComputeSources<'a> {
    /// Asset path for diagnostics (e.g. `assets/mod/shaders/compute/a.csh`).
    pub file: &'a str,
    /// Include base folder for quoted relative includes (e.g. `mod:shaders/compute/`).
    pub dir: &'a str,
    pub source: &'a str,
}

/// Compilation options for [`translate_compute`].
#[derive(Clone, Copy)]
pub struct ComputeOptions<'a> {
    /// Preprocessor defines (`NAME` or `NAME value`) in order.
    pub defines: &'a [&'a str],
    /// Mod-declared bindings (`BindGroupLayout`) in declaration order, bound in group
    /// [`COMPUTE_BIND_GROUP`].
    pub mod_bindings: &'a [ModBinding<'a>],
    /// Also run SPIR-V, Metal and DirectX 12 backend translation checks.
    pub check_backends: bool,
    /// Stage interface limits of the target device (or [`DEVICELESS_INTERFACE_LIMITS`]).
    pub interface: InterfaceLimits,
}

/// The one bind group of a compute pipeline: the mod's own `BindGroupLayout`. A compute shader
/// reads no vanilla bindings, so there is no shared set 0 to sit beside.
pub const COMPUTE_BIND_GROUP: u32 = 0;

/// The folder under `assets/<namespace>/` holding compute shaders (`.csh`).
pub const COMPUTE_FOLDER: &str = "shaders/compute";

/// Translate and validate one mod compute shader (`.csh`) on a guarded 64 MiB stack thread with
/// the same crash/memory containment bounds as [`translate`].
pub fn translate_compute(
    sources: &ComputeSources<'_>,
    options: &ComputeOptions<'_>,
    include: &(dyn Fn(&str) -> Result<Option<String>, String> + Sync),
) -> Result<sunrise_glsl::ComputeTranslated, Vec<Problem>> {
    guard::guarded(|| translate_compute_inner(sources, options, include))
        .unwrap_or_else(|reason| Err(vec![problem(sources.file, reason)]))
}

fn translate_compute_inner(
    sources: &ComputeSources<'_>,
    options: &ComputeOptions<'_>,
    include: &dyn Fn(&str) -> Result<Option<String>, String>,
) -> Result<sunrise_glsl::ComputeTranslated, Vec<Problem>> {
    let includes = Includes {
        read: &include,
        namespace: "minecraft",
        folder: INCLUDE_FOLDER,
        once: &["moj_import"],
    };
    let expanded = sunrise_glsl::expand_includes(
        sources.source,
        sources.file,
        sources.dir,
        &includes,
        Some(options.defines),
    )
    .map_err(|e| vec![problem(sources.file, e.to_string())])?
    .text()
    .to_owned();
    if let Some((at, msg)) = guard::refusal(&expanded) {
        return Err(vec![located(sources.file, &expanded, at, msg)]);
    }
    let set = assign_set1_bindings(options.mod_bindings, 0);
    let inputs = Inputs {
        blocks: &[],
        samplers: &[],
        extra_set: COMPUTE_BIND_GROUP,
        extra_blocks: &set.blocks,
        extra_samplers: &set.samplers,
        extra_texel_buffers: &set.texel_buffers,
        extra_storage_buffers: &set.storage_buffers,
        extra_storage_images: &set.storage_images,
        push_constant_binding: None,
        loose: &[],
        defines: options.defines,
        includes: &includes,
    };
    let translated = sunrise_glsl::translate_compute(
        sources.source,
        sources.file,
        sources.dir,
        &inputs,
        options.interface,
    )
    .map_err(|errors| {
        errors
            .into_iter()
            .map(|e| problem(sources.file, e.to_string()))
            .collect::<Vec<_>>()
    })?;
    if options.check_backends {
        let errors: Vec<Problem> = translate_backends(&translated.module)
            .into_iter()
            .map(|err| problem(sources.file, err))
            .collect();
        if !errors.is_empty() {
            return Err(errors);
        }
    }
    Ok(translated)
}

/// Translate and validate all three OIT variants (`DepthBounds`, `Transmittance`, `Accumulate`)
/// of an `.order_independent()` mod render pipeline program on a guarded 64 MiB stack thread.
/// When `additive` is true (`PipelineBlend::Additive`), `OIT_ADDITIVE` is also defined so the
/// shader adds light with no coverage.
pub fn translate_oit(
    sources: &ProgramSources<'_>,
    options: &Options<'_>,
    additive: bool,
    include: &(dyn Fn(&str) -> Result<Option<String>, String> + Sync),
) -> Result<[BuiltProgram; 3], Vec<Problem>> {
    let oit_options = Options {
        linear_light: false,
        ..*options
    };
    let mut out = Vec::with_capacity(3);
    for spec in &OIT_STAGES {
        let mut defines = effective_defines(options.defines);
        defines.extend_from_slice(spec.defines);
        if additive {
            defines.push("OIT_ADDITIVE");
        }
        let built = guard::guarded(|| {
            translate_inner(sources, &oit_options, &defines, spec.samplers, include)
        })
        .unwrap_or_else(|reason| Err(vec![problem(sources.fragment_file, reason)]))
        .map_err(|mut problems| {
            for p in &mut problems {
                p.message = format!("OIT {}: {}", spec.name, p.message);
            }
            problems
        })?;
        out.push(built);
    }
    Ok(out.try_into().expect("three OIT stages"))
}

fn translate_inner(
    sources: &ProgramSources<'_>,
    options: &Options<'_>,
    defines: &[&str],
    extra_samplers: &[&str],
    include: &dyn Fn(&str) -> Result<Option<String>, String>,
) -> Result<BuiltProgram, Vec<Problem>> {
    let read = |id: &str| -> Result<Option<String>, String> {
        if id == SPRITE_INCLUDE {
            if options.textured {
                return Ok(Some(PIPELINE_SPRITE_GLSL.to_owned()));
            }
            return Err(format!(
                "{SPRITE_INCLUDE} requires a sprite texture (RenderPipeline::textured)"
            ));
        }
        include(id)
    };
    let includes = Includes {
        read: &read,
        namespace: "minecraft",
        folder: INCLUDE_FOLDER,
        once: &["moj_import"],
    };

    let vertex_expanded = sunrise_glsl::expand_includes(
        sources.vertex,
        sources.vertex_file,
        sources.vertex_dir,
        &includes,
        Some(defines),
    )
    .map_err(|e| vec![problem(sources.vertex_file, e.to_string())])?
    .text()
    .to_owned();
    if let Some((at, msg)) = guard::refusal(&vertex_expanded) {
        return Err(vec![located(
            sources.vertex_file,
            &vertex_expanded,
            at,
            msg,
        )]);
    }
    if let Err(prob) =
        validate_vertex_inputs(sources.vertex_file, &vertex_expanded, options.vertex_inputs)
    {
        return Err(vec![prob]);
    }

    let fragment_expanded = sunrise_glsl::expand_includes(
        sources.fragment,
        sources.fragment_file,
        sources.fragment_dir,
        &includes,
        Some(defines),
    )
    .map_err(|e| vec![problem(sources.fragment_file, e.to_string())])?
    .text()
    .to_owned();
    if let Some((at, msg)) = guard::refusal(&fragment_expanded) {
        return Err(vec![located(
            sources.fragment_file,
            &fragment_expanded,
            at,
            msg,
        )]);
    }

    // When both stages live in the same folder, pass original sources so `__LINE__` stays exact.
    // When vertex and fragment come from different folders/namespaces, pass pre-expanded vertex
    // text (already resolved against `vertex_dir`) alongside the fragment source.
    let stages = if sources.vertex_dir == sources.fragment_dir {
        Stages {
            dir: sources.fragment_dir,
            vertex: sources.vertex,
            fragment: sources.fragment,
            vertex_file: sources.vertex_file,
            fragment_file: sources.fragment_file,
        }
    } else {
        Stages {
            dir: sources.fragment_dir,
            vertex: &vertex_expanded,
            fragment: sources.fragment,
            vertex_file: sources.vertex_file,
            fragment_file: sources.fragment_file,
        }
    };

    if options.push_constant_size > MAX_PUSH_CONSTANT_SIZE {
        return Err(vec![problem(
            sources.fragment_file,
            format!(
                "push_constant_size {} exceeds maximum {MAX_PUSH_CONSTANT_SIZE} bytes",
                options.push_constant_size
            ),
        )]);
    }
    let set1 = assign_set1_bindings(options.mod_bindings, options.push_constant_size);
    let inputs = Inputs {
        blocks: if options.textured {
            &BLOCKS_WITH_SPRITE
        } else {
            &BLOCKS
        },
        samplers: if options.textured {
            &TEXTURED_SAMPLERS
        } else {
            &[]
        },
        extra_set: MOD_BIND_GROUP_SET,
        extra_blocks: &set1.blocks,
        extra_samplers: &set1.samplers,
        extra_texel_buffers: &set1.texel_buffers,
        extra_storage_buffers: &set1.storage_buffers,
        extra_storage_images: &set1.storage_images,
        push_constant_binding: set1.push_constant_binding,
        loose: &[],
        defines,
        includes: &includes,
    };
    let (glue_prelude, glue_setup) = build_owned_glue(options.vertex_inputs);
    let glue = owned_glue(options, &glue_prelude, &glue_setup, None);

    let provided = |name: &str| extra_samplers.contains(&name);
    let mut translated = sunrise_glsl::translate(&stages, &inputs, &glue, &provided)
        .map_err(|errs| map_glsl_errors(sources, &errs, options.textured))?;
    if let Some(pc_binding) = set1.push_constant_binding
        && let Some(actual_size) = translated.uniform_block_size(MOD_BIND_GROUP_SET, pc_binding)
        && actual_size > options.push_constant_size
    {
        return Err(vec![problem(
            sources.fragment_file,
            format!(
                "push_constant block size {actual_size} bytes exceeds declared push_constant_size {} bytes",
                options.push_constant_size
            ),
        )]);
    }

    if options.linear_light
        && let Some(out_name) = fragment_loc0_output_name(&translated.fragment)
    {
        let finish = linear_fragment_finish(out_name);
        let linear_glue = owned_glue(options, &glue_prelude, &glue_setup, Some(&finish));
        translated = sunrise_glsl::translate(&stages, &inputs, &linear_glue, &provided)
            .map_err(|errs| map_glsl_errors(sources, &errs, options.textured))?;
    }

    if options.check_backends {
        let mut backend_errors = Vec::new();
        for err in translate_backends(&translated.vertex) {
            backend_errors.push(problem(sources.vertex_file, err));
        }
        for err in translate_backends(&translated.fragment) {
            backend_errors.push(problem(sources.fragment_file, err));
        }
        if !backend_errors.is_empty() {
            return Err(backend_errors);
        }
    }

    Ok(BuiltProgram {
        translated,
        vertex_expanded,
        fragment_expanded,
    })
}

/// Re-translate an already-expanded program with [`linear_fragment_finish`] enabled, returning
/// the linear-light fragment [`naga::Module`]. Used by the GPU renderer only on linear-light
/// frames so standard gamma loads avoid translating twice.
pub fn translate_linear_fragment(
    vertex_expanded: &str,
    fragment_expanded: &str,
    options: &Options<'_>,
) -> Result<naga::Module, String> {
    guard::guarded(|| {
        let defines = effective_defines(options.defines);
        let noop = |_: &str| Ok(None);
        let includes = Includes {
            read: &noop,
            namespace: "minecraft",
            folder: INCLUDE_FOLDER,
            once: &["moj_import"],
        };
        let set1 = assign_set1_bindings(options.mod_bindings, options.push_constant_size);
        let inputs = Inputs {
            blocks: if options.textured {
                &BLOCKS_WITH_SPRITE
            } else {
                &BLOCKS
            },
            samplers: if options.textured {
                &TEXTURED_SAMPLERS
            } else {
                &[]
            },
            extra_set: MOD_BIND_GROUP_SET,
            extra_blocks: &set1.blocks,
            extra_samplers: &set1.samplers,
            extra_texel_buffers: &set1.texel_buffers,
            extra_storage_buffers: &set1.storage_buffers,
            extra_storage_images: &set1.storage_images,
            push_constant_binding: set1.push_constant_binding,
            loose: &[],
            defines: &defines,
            includes: &includes,
        };
        let (glue_prelude, glue_setup) = build_owned_glue(options.vertex_inputs);
        let stages = Stages {
            dir: "",
            vertex: vertex_expanded,
            fragment: fragment_expanded,
            vertex_file: "",
            fragment_file: "",
        };
        let base = sunrise_glsl::translate(
            &stages,
            &inputs,
            &owned_glue(options, &glue_prelude, &glue_setup, None),
            &|_| false,
        )
        .map_err(|errs| sunrise_glsl::describe(&errs))?;
        let Some(out_name) = fragment_loc0_output_name(&base.fragment) else {
            return Ok(base.fragment);
        };
        let finish = linear_fragment_finish(out_name);
        sunrise_glsl::translate(
            &stages,
            &inputs,
            &owned_glue(options, &glue_prelude, &glue_setup, Some(&finish)),
            &|_| false,
        )
        .map_err(|errs| sunrise_glsl::describe(&errs))
        .map(|t| t.fragment)
    })
    .unwrap_or_else(Err)
}

fn map_glsl_errors(
    sources: &ProgramSources<'_>,
    errors: &[sunrise_glsl::Error],
    textured: bool,
) -> Vec<Problem> {
    errors
        .iter()
        .map(|err| {
            let mut message = err.message.clone();
            if !textured
                && (message.contains("Sampler0 is not") || message.contains("SpriteRect is not"))
            {
                message += " (requires RenderPipeline::textured)";
            }
            let file = if !err.file.is_empty() {
                err.file.as_str()
            } else if message.starts_with("vertex shader: ") {
                sources.vertex_file
            } else {
                sources.fragment_file
            };
            Problem {
                file: file.to_owned(),
                line: err.line as usize,
                column: 0,
                message,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const VSH: &str = "\
#version 330
#include <minecraft:dynamictransforms.glsl>
#include <minecraft:projection.glsl>
in vec3 Position;
in vec2 UV0;
in vec4 Color;
out vec2 texCoord0;
out vec4 vertexColor;
void main() {
    gl_Position = ProjMat * ModelViewMat * vec4(Position, 1.0);
    texCoord0 = UV0;
    vertexColor = Color;
}
";

    const FSH: &str = "\
#version 330
#include <minecraft:globals.glsl>
#include <minecraft:dynamictransforms.glsl>
#include <minecraft:fog.glsl>
in vec2 texCoord0;
in vec4 vertexColor;
out vec4 fragColor;
void main() {
    vec4 color = vertexColor * ColorModulator;
#ifdef PULSE
    color.rgb *= 1.0 + sin(GameTime * 24000.0 * 0.2);
#endif
    fragColor = vec4(color.rgb * (1.0 - FogEnvironmentalStart * 0.0), color.a);
}
";

    fn stub_include(id: &str) -> Result<Option<String>, String> {
        Ok(stubs::read(id).map(str::to_owned))
    }

    fn sources<'a>(v: &'a str, f: &'a str) -> ProgramSources<'a> {
        ProgramSources {
            vertex_file: "assets/example/shaders/core/glow.vsh",
            vertex_dir: "example:shaders/core/",
            vertex: v,
            fragment_file: "assets/example/shaders/core/glow.fsh",
            fragment_dir: "example:shaders/core/",
            fragment: f,
        }
    }

    #[test]
    fn vanilla_style_program_translates_and_passes_all_backends() {
        let opts = Options {
            defines: &["PULSE"],
            check_backends: true,
            ..Default::default()
        };
        let built = translate(&sources(VSH, FSH), &opts, &stub_include).unwrap();
        assert_eq!(built.translated.vertex.entry_points.len(), 1);
        assert_eq!(built.translated.fragment.entry_points.len(), 1);
        assert!(!built.translated.stages_reading(0).is_empty()); // Globals
        assert!(!built.translated.stages_reading(1).is_empty()); // Projection
        assert!(!built.translated.stages_reading(2).is_empty()); // DynamicTransforms
        assert!(!built.translated.stages_reading(3).is_empty()); // Fog
    }

    #[test]
    fn linear_light_fragment_finish_validates_on_all_backends_and_skips_alpha_only() {
        let opts = Options {
            linear_light: true,
            check_backends: true,
            ..Default::default()
        };
        let built = translate(&sources(VSH, FSH), &opts, &stub_include).unwrap();
        assert!(
            built
                .translated
                .fragment
                .functions
                .iter()
                .any(|(_, f)| f.name.as_deref() == Some("hz_main"))
        );

        // Also verify lazy linear re-translation from expanded sources.
        let gamma = translate(
            &sources(VSH, FSH),
            &Options {
                linear_light: false,
                ..Default::default()
            },
            &stub_include,
        )
        .unwrap();
        let linear_frag = translate_linear_fragment(
            &gamma.vertex_expanded,
            &gamma.fragment_expanded,
            &Options::default(),
        )
        .unwrap();
        assert!(translate_backends(&linear_frag).is_empty());

        // Under OIT_ALPHA_ONLY with no location-0 output, fragment_finish is skipped cleanly.
        let oit_fsh = "\
#version 330
#include <minecraft:oit.glsl>
in vec2 texCoord0;
in vec4 vertexColor;
#ifndef OIT_ALPHA_ONLY
out vec4 fragColor;
#endif
void main() {
#ifdef OIT_ALPHA_ONLY
    executeAlphaOnlyPhase(gl_FragCoord.z, vertexColor.a);
#else
    fragColor = sampleColorForAccumulation(vertexColor);
#endif
}
";
        let alpha_only = translate(
            &sources(VSH, oit_fsh),
            &Options {
                defines: &["OIT_ALPHA_ONLY"],
                linear_light: true,
                check_backends: true,
                ..Default::default()
            },
            &stub_include,
        )
        .unwrap();
        assert!(alpha_only.translated.fragment_writes().is_empty());
    }

    #[test]
    fn linear_light_fragment_finish_resolves_loc0_output_past_earlier_globals() {
        let fsh = "\
#version 330
in vec2 texCoord0;
in vec4 vertexColor;
vec4 scratchGlobal = vec4(0.25);
out vec4 customOut;
void main() {
    scratchGlobal = vertexColor * 0.5;
    customOut = scratchGlobal;
}
";
        let built = translate(
            &sources(VSH, fsh),
            &Options {
                linear_light: true,
                check_backends: true,
                ..Default::default()
            },
            &stub_include,
        )
        .unwrap();
        assert_eq!(built.translated.fragment_writes(), [0]);
        assert!(
            built
                .translated
                .fragment
                .functions
                .iter()
                .any(|(_, f)| f.name.as_deref() == Some("hz_main")),
            "linear_light must wrap main as hz_main using customOut, not scratchGlobal"
        );
    }

    #[test]
    fn sprite_include_binds_sampler0_and_sprite_rect_and_is_refused_when_untextured() {
        let fsh = "\
#version 330
#include <sunrise:pipeline_sprite.glsl>
in vec2 texCoord0;
in vec4 vertexColor;
out vec4 fragColor;
void main() {
    fragColor = sampleSprite(texCoord0) * vertexColor;
}
";
        let built = translate(
            &sources(VSH, fsh),
            &Options {
                textured: true,
                check_backends: true,
                ..Default::default()
            },
            &stub_include,
        )
        .unwrap();
        assert!(!built.translated.stages_reading(4).is_empty()); // Sampler0 texture
        assert!(
            !built
                .translated
                .stages_reading(SPRITE_RECT_BINDING)
                .is_empty()
        ); // SpriteRect

        let err = translate(
            &sources(VSH, fsh),
            &Options {
                textured: false,
                ..Default::default()
            },
            &stub_include,
        )
        .unwrap_err();
        assert!(
            err[0].to_string().contains("RenderPipeline::textured"),
            "{err:?}"
        );

        // Raw `uniform sampler2D Sampler0;` without `.textured(...)` is also refused with hint.
        let raw_sampler = "\
#version 330
uniform sampler2D Sampler0;
in vec2 texCoord0;
out vec4 fragColor;
void main() { fragColor = texture(Sampler0, texCoord0); }
";
        let err = translate(
            &sources(VSH, raw_sampler),
            &Options::default(),
            &stub_include,
        )
        .unwrap_err();
        assert!(
            err[0].to_string().contains("RenderPipeline::textured"),
            "{err:?}"
        );
    }

    #[test]
    fn separate_vertex_and_fragment_directories_resolve_own_relative_includes() {
        let vsh = "#version 330\n#include \"vhelper.glsl\"\nin vec3 Position;\nout vec4 vertexColor;\n\
            void main() { gl_Position = vec4(Position, 1.0); vertexColor = vcolor(); }\n";
        let fsh = "#version 330\n#include \"fhelper.glsl\"\nin vec4 vertexColor;\nout vec4 fragColor;\n\
            void main() { fragColor = vertexColor * fscale(); }\n";
        let srcs = ProgramSources {
            vertex_file: "assets/libmod/shaders/core/v/shared.vsh",
            vertex_dir: "libmod:shaders/core/v/",
            vertex: vsh,
            fragment_file: "assets/appmod/shaders/core/f/glow.fsh",
            fragment_dir: "appmod:shaders/core/f/",
            fragment: fsh,
        };
        let include = |id: &str| match id {
            "libmod:shaders/core/v/vhelper.glsl" => {
                Ok(Some("vec4 vcolor() { return vec4(1.0); }\n".to_owned()))
            }
            "appmod:shaders/core/f/fhelper.glsl" => {
                Ok(Some("float fscale() { return 0.5; }\n".to_owned()))
            }
            other => stub_include(other),
        };
        let built = translate(&srcs, &Options::default(), &include).unwrap();
        assert_eq!(built.translated.fragment_writes(), [0]);
    }

    #[test]
    fn guard_bounds_refuse_pathological_glsl_with_location() {
        let huge_chain = format!(
            "#version 330\nin vec2 texCoord0;\nout vec4 fragColor;\n\
             void main() {{\n    float a = texCoord0.x;\n    float x = a{};\n    fragColor = vec4(x);\n}}\n",
            "+a".repeat(250)
        );
        let err = translate(
            &sources(VSH, &huge_chain),
            &Options::default(),
            &stub_include,
        )
        .unwrap_err();
        assert_eq!(err[0].file, "assets/example/shaders/core/glow.fsh");
        assert!(err[0].line > 0, "{err:?}");
        assert!(err[0].message.contains("chains more than 200"), "{err:?}");
    }

    /// `sampleSprite`'s texel arithmetic: `uv` 0 to 1 spans the sprite's rect once, the far edge
    /// and every repeat (negative too) land back inside it, never on a neighbour's texel.
    #[test]
    fn the_repeat_picks_integer_texels_inside_the_rect_for_any_uv() {
        let rect = [0.5, 0.25, 0.75, 0.5];
        let at = |u: f32, v: f32| repeat_texel([u, v], rect, [64, 64]);
        assert_eq!(at(0.0, 0.0), [32, 16], "the near corner");
        assert_eq!(at(0.5, 0.5), [40, 24]);
        assert_eq!(at(0.999_999, 0.999_999), [47, 31], "just inside far edge");
        assert_eq!(at(1.0, 1.0), [32, 16], "far edge wraps to start");
        assert_eq!(at(2.5, 3.0625), [40, 17], "repeated");
        assert_eq!(at(-0.0625, -0.25), [47, 28], "negative wraps from far side");
        assert_eq!(at(-1.0, -2.0), [32, 16]);
        assert_eq!(at(-1.0e-9, -1.0e-9), [47, 31]);
        assert_eq!(at(-1000.5, 1000.75), [40, 28]);
        for u in [0.0, 0.25, 0.999, 1.0, -0.5, 7.75] {
            assert_eq!(
                repeat_texel([u, u], [0.5, 0.0, 1.0, 1.0], [2, 1]),
                [1, 0],
                "{u}"
            );
        }
        for u in [0.0, 0.999, -0.001, 3.5] {
            assert_eq!(
                repeat_texel([u, 0.0], [0.0, 0.0, 0.5, 1.0], [2, 1]),
                [0, 0],
                "{u}"
            );
        }
    }

    #[test]
    fn order_independent_program_translates_all_three_oit_variants_and_refuses_alpha_only_color_output()
     {
        let oit_fsh = "\
#version 330
#include <minecraft:globals.glsl>
#include <minecraft:dynamictransforms.glsl>
#include <minecraft:oit.glsl>
in vec2 texCoord0;
in vec4 vertexColor;
#ifndef OIT_ALPHA_ONLY
out vec4 fragColor;
#endif
void main() {
    vec4 color = vertexColor * ColorModulator;
#ifdef OIT_ALPHA_ONLY
    executeAlphaOnlyPhase(gl_FragCoord.z, color.a);
#else
#ifdef OIT_ACCUMULATE
    color = sampleColorForAccumulation(color);
#endif
    fragColor = color;
#endif
}
";
        let opts = Options {
            check_backends: true,
            ..Default::default()
        };
        let translucent = translate_oit(&sources(VSH, oit_fsh), &opts, false, &stub_include)
            .expect("translucent OIT variants must translate");
        assert_eq!(translucent[0].translated.fragment_writes(), [0]);
        assert_eq!(translucent[1].translated.fragment_writes(), [0, 1]);
        assert_eq!(translucent[2].translated.fragment_writes(), [0]);
        assert!(translucent[0].translated.other.is_empty());
        assert_eq!(translucent[1].translated.other, ["DepthBoundsSampler"]);
        assert_eq!(
            translucent[2].translated.other,
            ["DepthBoundsSampler", "Coeff0", "Coeff1"]
        );

        let additive = translate_oit(&sources(VSH, oit_fsh), &opts, true, &stub_include)
            .expect("additive OIT variants must translate");
        assert_eq!(additive[2].translated.fragment_writes(), [0]);

        // Textured OIT program also translates cleanly.
        let tex_oit_fsh = "\
#version 330
#include <minecraft:dynamictransforms.glsl>
#include <minecraft:oit.glsl>
#include <sunrise:pipeline_sprite.glsl>
in vec2 texCoord0;
in vec4 vertexColor;
#ifndef OIT_ALPHA_ONLY
out vec4 fragColor;
#endif
void main() {
    vec4 color = sampleSprite(texCoord0) * vertexColor * ColorModulator;
#ifdef OIT_ALPHA_ONLY
    executeAlphaOnlyPhase(gl_FragCoord.z, color.a);
#else
#ifdef OIT_ACCUMULATE
    color = sampleColorForAccumulation(color);
#endif
    fragColor = color;
#endif
}
";
        let tex_opts = Options {
            textured: true,
            check_backends: true,
            ..Default::default()
        };
        translate_oit(&sources(VSH, tex_oit_fsh), &tex_opts, false, &stub_include)
            .expect("textured OIT variants must translate");

        // Writing `fragColor` under `OIT_ALPHA_ONLY` must be refused at the depth_bounds variant.
        let broken_fsh = "\
#version 330
#include <minecraft:oit.glsl>
in vec4 vertexColor;
out vec4 fragColor;
void main() {
#ifdef OIT_ALPHA_ONLY
    executeAlphaOnlyPhase(gl_FragCoord.z, vertexColor.a);
#else
    fragColor = sampleColorForAccumulation(vertexColor);
#endif
}
";
        let err = translate_oit(&sources(VSH, broken_fsh), &opts, false, &stub_include)
            .expect_err("fragColor under OIT_ALPHA_ONLY must be refused");
        assert!(
            err[0].message.contains("OIT depth_bounds"),
            "expected depth_bounds refusal, got: {err:?}"
        );
    }

    #[test]
    fn custom_integer_and_unorm_vertex_inputs_translate_and_refuse_undeclared_in() {
        let custom_vsh = "\
#version 330
#include <minecraft:projection.glsl>
#include <minecraft:dynamictransforms.glsl>
in uvec4 PackedPos;
in vec4 PackedColor;
in ivec2 CellOffset;
out vec4 vertexColor;
void main() {
    vec3 pos = vec3(PackedPos.xyz) + vec3(float(CellOffset.x), 0.0, float(CellOffset.y));
    gl_Position = ProjMat * ModelViewMat * vec4(pos, 1.0);
    vertexColor = PackedColor;
}
";
        let custom_fsh = "\
#version 330
in vec4 vertexColor;
out vec4 fragColor;
void main() {
    fragColor = vertexColor;
}
";
        let custom_inputs = [
            ("PackedPos", "uvec4"),
            ("PackedColor", "vec4"),
            ("CellOffset", "ivec2"),
        ];
        let opts = Options {
            vertex_inputs: &custom_inputs,
            check_backends: true,
            ..Default::default()
        };
        let built = translate(&sources(custom_vsh, custom_fsh), &opts, &stub_include)
            .expect("custom integer+unorm vertex shader must translate on all backends");
        assert_eq!(built.translated.vertex.entry_points.len(), 1);

        // Undeclared vertex `in` is refused naming the attribute.
        let undeclared_vsh = "\
#version 330
#include <minecraft:projection.glsl>
in uvec4 PackedPos;
in vec3 UndeclaredAttr;
out vec4 vertexColor;
void main() {
    gl_Position = ProjMat * vec4(vec3(PackedPos.xyz) + UndeclaredAttr, 1.0);
    vertexColor = vec4(1.0);
}
";
        let err = translate(&sources(undeclared_vsh, custom_fsh), &opts, &stub_include)
            .expect_err("undeclared vertex input must be refused");
        assert!(
            err[0].message.contains("UndeclaredAttr"),
            "expected UndeclaredAttr in error, got: {err:?}"
        );

        // Type mismatch between VertexFormat and shader `in` is refused.
        let wrong_type_vsh = "\
#version 330
#include <minecraft:projection.glsl>
in vec4 PackedPos;
out vec4 vertexColor;
void main() {
    gl_Position = ProjMat * PackedPos;
    vertexColor = vec4(1.0);
}
";
        let err = translate(&sources(wrong_type_vsh, custom_fsh), &opts, &stub_include)
            .expect_err("mismatched vertex input type must be refused");
        assert!(
            err[0].message.contains("PackedPos") && err[0].message.contains("uvec4"),
            "expected type mismatch in error, got: {err:?}"
        );
    }

    #[test]
    fn set1_bind_group_layout_and_push_constants_translate_and_refuse_undeclared_or_oversize() {
        let vsh = "\
#version 330
#include <minecraft:projection.glsl>
#include <minecraft:dynamictransforms.glsl>
layout(push_constant) uniform Push { vec4 Offset; };
uniform isamplerBuffer Lut;
in vec3 Position;
in vec2 UV0;
in vec4 Color;
out vec2 texCoord0;
out vec4 vertexColor;
void main() {
    int lut = texelFetch(Lut, 0).r;
    gl_Position = ProjMat * ModelViewMat * vec4(Position + Offset.xyz, 1.0);
    texCoord0 = UV0 + vec2(float(lut) * 0.0, 0.0);
    vertexColor = Color;
}
";
        let fsh = "\
#version 330
layout(std140) uniform TintBlock { vec4 TintColor; };
uniform sampler2D CustomTex;
in vec2 texCoord0;
in vec4 vertexColor;
out vec4 fragColor;
void main() {
    fragColor = texture(CustomTex, texCoord0) * vertexColor * TintColor;
}
";
        let mod_bindings = [
            ModBinding {
                name: "TintBlock",
                kind: ModBindingKind::UniformBuffer,
            },
            ModBinding {
                name: "CustomTex",
                kind: ModBindingKind::CombinedImageSampler,
            },
            ModBinding {
                name: "Lut",
                kind: ModBindingKind::TexelBuffer,
            },
        ];
        let opts = Options {
            mod_bindings: &mod_bindings,
            push_constant_size: 16,
            check_backends: true,
            ..Default::default()
        };
        let built = translate(&sources(vsh, fsh), &opts, &stub_include)
            .expect("set-1 uniform + sampler + texel buffer + push_constant must translate");
        assert!(built.translated.other.is_empty());

        // Undeclared block is refused naming the missing block.
        let missing_opts = Options {
            mod_bindings: &mod_bindings[1..],
            push_constant_size: 16,
            ..Default::default()
        };
        let err = translate(&sources(vsh, fsh), &missing_opts, &stub_include)
            .expect_err("undeclared TintBlock must fail");
        assert!(err[0].message.contains("TintBlock"), "got: {err:?}");

        // Push constant block exceeding declared push_constant_size is refused.
        let tiny_pc_opts = Options {
            mod_bindings: &mod_bindings,
            push_constant_size: 8,
            ..Default::default()
        };
        let err = translate(&sources(vsh, fsh), &tiny_pc_opts, &stub_include)
            .expect_err("push_constant larger than declared size must fail");
        assert!(err[0].message.contains("push_constant"), "got: {err:?}");

        // push_constant_size > 128 B is refused.
        let huge_pc_opts = Options {
            mod_bindings: &mod_bindings,
            push_constant_size: MAX_PUSH_CONSTANT_SIZE + 4,
            ..Default::default()
        };
        let err = translate(&sources(vsh, fsh), &huge_pc_opts, &stub_include)
            .expect_err("push_constant_size > 128 must fail");
        assert!(err[0].message.contains("128"), "got: {err:?}");

        // Static extraction helper finds the custom set-1 bindings and push constant.
        let (extracted, pc) = extract_mod_bindings(vsh, fsh, false);
        assert_eq!(pc, MAX_PUSH_CONSTANT_SIZE);
        assert_eq!(
            extracted,
            vec![
                ("Lut".to_owned(), ModBindingKind::TexelBuffer),
                ("TintBlock".to_owned(), ModBindingKind::UniformBuffer),
                ("CustomTex".to_owned(), ModBindingKind::CombinedImageSampler),
            ]
        );
    }
    #[test]
    fn storage_blocks_translate_into_set_one_and_refuse_undeclared_or_mismatched_access() {
        let vsh = "\
#version 330
in vec3 Position;
in vec2 UV0;
in vec4 Color;
layout(std430) readonly buffer Quads {
    uint words[];
};
out vec2 texCoord0;
out vec4 vertexColor;
void main() {
    float w = float(words[gl_VertexID]);
    gl_Position = vec4(Position + vec3(w * 0.0), 1.0);
    texCoord0 = UV0;
    vertexColor = Color;
}
";
        let fsh = "\
#version 330
in vec2 texCoord0;
in vec4 vertexColor;
out vec4 fragColor;
void main() {
    fragColor = vertexColor;
}
";
        let declared = |read_only| {
            [ModBinding {
                name: "Quads",
                kind: ModBindingKind::StorageBuffer { read_only },
            }]
        };
        let bindings = declared(true);
        let opts = Options {
            mod_bindings: &bindings,
            check_backends: true,
            ..Default::default()
        };
        translate(&sources(vsh, fsh), &opts, &stub_include)
            .expect("a read-only storage block translates");

        let err = translate(&sources(vsh, fsh), &Options::default(), &stub_include)
            .expect_err("an undeclared storage block is refused");
        assert!(err[0].message.contains("Quads"), "got: {err:?}");

        let bindings = declared(false);
        let opts = Options {
            mod_bindings: &bindings,
            ..Default::default()
        };
        let err = translate(&sources(vsh, fsh), &opts, &stub_include)
            .expect_err("a `readonly` block declared read-write is refused");
        assert!(err[0].message.contains("read-write"), "got: {err:?}");

        let (extracted, _) = extract_mod_bindings(vsh, fsh, false);
        assert_eq!(
            extracted,
            vec![(
                "Quads".to_owned(),
                ModBindingKind::StorageBuffer { read_only: true }
            )]
        );
    }

    #[test]
    fn storage_images_translate_into_set_one_with_their_declared_format_and_access() {
        let vsh = "\
#version 330
in vec3 Position;
in vec2 UV0;
in vec4 Color;
out vec2 texCoord0;
out vec4 vertexColor;
void main() {
    gl_Position = vec4(Position, 1.0);
    texCoord0 = UV0;
    vertexColor = Color;
}
";
        let fsh = |image: &str| {
            format!(
                "\
#version 330
in vec2 texCoord0;
in vec4 vertexColor;
{image}
out vec4 fragColor;
void main() {{
    imageStore(Pyramid, ivec2(0), vertexColor);
    fragColor = vertexColor;
}}
"
            )
        };
        let declared = |access| {
            [ModBinding {
                name: "Pyramid",
                kind: ModBindingKind::StorageImage { access },
            }]
        };
        let write_only = fsh("layout(rgba8) writeonly uniform image2D Pyramid;");
        let bindings = declared(sunrise_glsl::ImageAccess::WriteOnly);
        let opts = Options {
            mod_bindings: &bindings,
            check_backends: true,
            ..Default::default()
        };
        let built = translate(&sources(vsh, &write_only), &opts, &stub_include)
            .expect("a write-only rgba8 image translates");
        let module = &built.translated.fragment;
        let (_, image) = module
            .global_variables
            .iter()
            .find(|(_, var)| var.name.as_deref() == Some("Pyramid"))
            .expect("the image is a global");
        assert_eq!(
            image.binding,
            Some(naga::ResourceBinding {
                group: MOD_BIND_GROUP_SET,
                binding: 0
            })
        );
        assert!(matches!(
            module.types[image.ty].inner,
            naga::TypeInner::Image {
                class: naga::ImageClass::Storage {
                    format: naga::StorageFormat::Rgba8Unorm,
                    ..
                },
                ..
            }
        ));

        // The qualifiers must agree with the declared access, and the image must be declared.
        let read_write = declared(sunrise_glsl::ImageAccess::ReadWrite);
        let opts = Options {
            mod_bindings: &read_write,
            ..Default::default()
        };
        assert!(
            translate(&sources(vsh, &write_only), &opts, &stub_include).is_err(),
            "`writeonly` on a read-write image is refused"
        );
        assert!(
            translate(
                &sources(vsh, &write_only),
                &Options::default(),
                &stub_include,
            )
            .is_err(),
            "an undeclared image is refused"
        );

        // The texel format comes from `layout(<format>)`: without one the image is refused.
        let no_format = fsh("writeonly uniform image2D Pyramid;");
        let opts = Options {
            mod_bindings: &bindings,
            ..Default::default()
        };
        assert!(
            translate(&sources(vsh, &no_format), &opts, &stub_include).is_err(),
            "an image with no format is refused"
        );

        // `uniform readonly image2D` orders its qualifiers after the storage word.
        let (extracted, _) = extract_mod_bindings(
            vsh,
            &fsh("layout(r32f) uniform readonly image2D Pyramid;"),
            false,
        );
        assert_eq!(
            extracted,
            vec![(
                "Pyramid".to_owned(),
                ModBindingKind::StorageImage {
                    access: sunrise_glsl::ImageAccess::ReadOnly
                }
            )]
        );
    }

    /// The bindings come from the text the translator reads, where comments are gone: a
    /// declaration named only in a line or a block comment is not the mod's.
    #[test]
    fn a_name_only_in_a_comment_is_not_a_mod_binding() {
        let vsh = "#version 330\n// uniform sampler2D LineOnly;\nin vec3 Position;\n\
            void main() { gl_Position = vec4(Position, 1.0); }\n";
        let fsh = "#version 330\n/* uniform sampler2D Spanning;\n\
            layout(std140) uniform Hidden { vec4 h; };\n*/\n\
            uniform sampler2D Real; /* uniform sampler2D Trailing; */\n\
            out vec4 fragColor;\nvoid main() { fragColor = texture(Real, vec2(0.0)); }\n";
        let (bindings, push_constant_size) =
            extract_program_bindings(&sources(vsh, fsh), false, &[], &stub_include);
        assert_eq!(
            bindings,
            vec![("Real".to_owned(), ModBindingKind::CombinedImageSampler)]
        );
        assert_eq!(push_constant_size, 0);
    }
}
