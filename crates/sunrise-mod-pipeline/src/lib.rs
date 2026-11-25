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
use sunrise_glsl::{Glue, Includes, Inputs, Stages, Translated};

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

/// Compilation options for [`translate`].
#[derive(Clone, Copy)]
pub struct Options<'a> {
    /// Whether the pipeline declared `.textured(PipelineTexture::Sprite(...))`, exposing
    /// `Sampler0`, `SpriteRect`, and `#include <sunrise:pipeline_sprite.glsl>`.
    pub textured: bool,
    /// Pipeline defines (`NAME` or `NAME value`) in order; [`DEPTH_ZERO_TO_ONE`] is always
    /// prepended automatically.
    pub defines: &'a [&'a str],
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
        inputs: &[("Position", "vec3"), ("UV0", "vec2"), ("Color", "vec4")],
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
    guard::guarded(|| translate_inner(sources, options, include))
        .unwrap_or_else(|reason| Err(vec![problem(sources.fragment_file, reason)]))
}

fn translate_inner(
    sources: &ProgramSources<'_>,
    options: &Options<'_>,
    include: &dyn Fn(&str) -> Result<Option<String>, String>,
) -> Result<BuiltProgram, Vec<Problem>> {
    let defines = effective_defines(options.defines);
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
        Some(&defines),
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

    let fragment_expanded = sunrise_glsl::expand_includes(
        sources.fragment,
        sources.fragment_file,
        sources.fragment_dir,
        &includes,
        Some(&defines),
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
        loose: &[],
        defines: &defines,
        includes: &includes,
    };
    let glue = pipeline_glue(options.interface, None);

    let mut translated = sunrise_glsl::translate(&stages, &inputs, &glue, &|_| false)
        .map_err(|errs| map_glsl_errors(sources, &errs, options.textured))?;

    if options.linear_light
        && let Some(out_name) = fragment_loc0_output_name(&translated.fragment)
    {
        let finish = linear_fragment_finish(out_name);
        let linear_glue = pipeline_glue(options.interface, Some(&finish));
        translated = sunrise_glsl::translate(&stages, &inputs, &linear_glue, &|_| false)
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
            loose: &[],
            defines: &defines,
            includes: &includes,
        };
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
            &pipeline_glue(options.interface, None),
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
            &pipeline_glue(options.interface, Some(&finish)),
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
}
