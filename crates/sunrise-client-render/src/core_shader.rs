//! Resource packs' core shaders (`shaders/core/*.vsh` and `.fsh`: the GLSL vanilla compiles for its
//! pipelines) through [`sunrise_glsl`], so a pack that reshapes text from `core/text` (a HUD its
//! vertex shader moves to the screen's corners, effects in its fragment shader) draws as in
//! vanilla. This is the vanilla half: its uniform blocks and samplers, where its includes live
//! (26.3's `#include` and the older `#moj_import`, which pastes a file once, as
//! `GlslPreprocessor` imported them), and the define its compiler gives every program.

use naga::ShaderStage;
use naga::front::glsl::{InterfaceLimits, Options};
use sunrise_glsl::{Glue, ImportRefusal, Includes, Inputs, Stages, Translated};

/// Vanilla's uniform blocks (`shaders/include/*.glsl`) and their bindings in set 0.
pub const BLOCKS: [(&str, u32); 4] = [
    ("Globals", 0),
    ("Projection", 1),
    ("DynamicTransforms", 2),
    ("Fog", 3),
];

/// Vanilla's samplers: the texture's binding in set 0, the sampler object's the next one.
pub const SAMPLERS: [(&str, u32); 3] = [("Sampler0", 4), ("Sampler1", 6), ("Sampler2", 8)];

/// The members of [`BLOCKS`] that packs still declare as loose uniforms, beside the blocks'
/// imports in another branch (the layout before 26.x moved them into blocks).
const LOOSE: [&str; 18] = [
    "CameraBlockPos",
    "GlintAlpha",
    "CameraOffset",
    "GameTime",
    "ScreenSize",
    "MenuBlurRadius",
    "UseRgss",
    "ProjMat",
    "TextureMat",
    "ModelViewMat",
    "ColorModulator",
    "FogColor",
    "FogEnvironmentalStart",
    "FogRenderDistanceStart",
    "FogRenderDistanceEnd",
    "FogEnd",
    "FogStart",
    "FogShape",
];

/// What `GlslCompiler.createBaseShaderOptions` defines for every program it compiles, core and
/// post alike, on a backend whose depth runs from 0 to 1, as wgpu's does.
pub(crate) const DEPTH_ZERO_TO_ONE: &str = "RENDERPEARL_DEPTH_IS_ZERO_TO_ONE";

/// A device's stage interface limits, as naga's GLSL front end takes them: its
/// `max_inter_stage_shader_variables` and `max_vertex_attributes`.
pub fn interface_limits(limits: &wgpu::Limits) -> InterfaceLimits {
    InterfaceLimits {
        inter_stage_variables: limits.max_inter_stage_shader_variables,
        vertex_attributes: limits.max_vertex_attributes,
    }
}

/// The shaders of one pipeline (`text.vsh`, `text.fsh`) with vanilla's `defines` for it.
/// `include` answers a file id (`minecraft:shaders/include/fog.glsl`) with its text.
pub fn translate_core(
    vertex: &str,
    fragment: &str,
    defines: &[&str],
    include: &dyn Fn(&str) -> Option<String>,
    glue: &Glue,
) -> Result<Translated, String> {
    translate_core_with(vertex, fragment, defines, include, glue, &|_| false)
}

/// [`translate_core`] for a pipeline that also gives the blocks and samplers `provided` says
/// yes to, at their [`Translated::other`] bindings; the others are still an error.
pub fn translate_core_with(
    vertex: &str,
    fragment: &str,
    defines: &[&str],
    include: &dyn Fn(&str) -> Option<String>,
    glue: &Glue,
    provided: &dyn Fn(&str) -> bool,
) -> Result<Translated, String> {
    let sources = Stages {
        dir: "minecraft:shaders/core/",
        vertex,
        fragment,
    };
    translate_core_at(&sources, defines, &|id| Ok(include(id)), glue, provided)
}

/// [`translate_core_with`] for stages that aren't in `minecraft:shaders/core/`, with `read`
/// answering a file id with its text, none when the packs haven't it, or why it can't be read
/// ([`expand_imports`]).
pub fn translate_core_at(
    sources: &Stages,
    defines: &[&str],
    read: &dyn Fn(&str) -> Result<Option<String>, String>,
    glue: &Glue,
    provided: &dyn Fn(&str) -> bool,
) -> Result<Translated, String> {
    let read = with_polytone_globals(read);
    let defines = program_defines(defines);
    let inputs = Inputs {
        blocks: &BLOCKS,
        samplers: &SAMPLERS,
        loose: &LOOSE,
        defines: &defines,
        includes: &includes(&read),
    };
    sunrise_glsl::translate(sources, &inputs, glue, provided)
}

/// The packs' file with its includes inlined, every branch's: the one resolver of post shaders,
/// which keep their own preprocessing ([`sunrise_glsl::expand_includes`]). Only a missing
/// `polytone_globals.glsl` is the built-in one (`sunrise_polytone::globals::GLSL`).
pub(crate) fn expand_imports(
    text: &str,
    dir: &str,
    read: &dyn Fn(&str) -> Result<Option<String>, String>,
) -> Result<String, ImportRefusal> {
    let read = with_polytone_globals(read);
    sunrise_glsl::expand_includes(text, dir, &includes(&read), None)
}

/// [`expand_imports`] resolving only the includes a core pipeline reaches under its `defines`
/// and the backend's ([`DEPTH_ZERO_TO_ONE`]), as the compiler sees them.
pub(crate) fn expand_core_imports(
    text: &str,
    dir: &str,
    defines: &[&str],
    read: &dyn Fn(&str) -> Result<Option<String>, String>,
) -> Result<String, ImportRefusal> {
    let read = with_polytone_globals(read);
    let defines = program_defines(defines);
    sunrise_glsl::expand_includes(text, dir, &includes(&read), Some(&defines))
}

/// naga's options for one `stage` of a program with the pipeline's `defines`, the backend's
/// first ([`DEPTH_ZERO_TO_ONE`]), and the device's stage interface `limits`. The core shaders'
/// and the post shaders' one way in.
pub(crate) fn glsl_options(
    stage: ShaderStage,
    defines: &[&str],
    limits: InterfaceLimits,
) -> Options {
    sunrise_glsl::glsl_options(stage, &program_defines(defines), limits)
}

/// A program's defines as vanilla's compiler gives them: the backend's ([`DEPTH_ZERO_TO_ONE`]),
/// then the pipeline's `defines` (`NAME` or `NAME value`).
fn program_defines<'a>(defines: &[&'a str]) -> Vec<&'a str> {
    std::iter::once(DEPTH_ZERO_TO_ONE)
        .chain(defines.iter().copied())
        .collect()
}

/// Vanilla's include layout: `<path>` is `minecraft:shaders/include/path`, `#moj_import` pastes
/// a file once.
fn includes<'a>(read: &'a dyn Fn(&str) -> Result<Option<String>, String>) -> Includes<'a> {
    Includes {
        read,
        namespace: "minecraft",
        folder: "shaders/include/",
        once: &["moj_import"],
    }
}

/// `read`, with Polytone's built-in `polytone_globals.glsl` where the packs have none.
fn with_polytone_globals(
    read: &dyn Fn(&str) -> Result<Option<String>, String>,
) -> impl Fn(&str) -> Result<Option<String>, String> {
    move |id| {
        Ok(read(id)?.or_else(|| {
            sunrise_polytone::globals::is_include(id)
                .then(|| sunrise_polytone::globals::GLSL.to_owned())
        }))
    }
}

/// A program of stages not yet validated (tests that build their own).
#[cfg(test)]
pub(crate) fn validated(
    vertex: naga::Module,
    fragment: naga::Module,
    other: Vec<String>,
) -> Translated {
    use naga::valid::{Capabilities, ValidationFlags, Validator};
    let validate = |module: naga::Module| {
        let info = Validator::new(ValidationFlags::all(), Capabilities::all())
            .validate(&module)
            .expect("the module validates");
        (module, info)
    };
    let other_base = BLOCKS
        .iter()
        .map(|&(_, b)| b + 1)
        .chain(SAMPLERS.iter().map(|&(_, b)| b + 2))
        .max()
        .unwrap_or(0);
    Translated::new(validate(vertex), validate(fragment), other, other_base)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Only a missing `polytone_globals.glsl` is Polytone's built-in one: the packs' own file is
    /// read instead, one that can't be read refuses the shader with the reader's reason, and any
    /// other missing file is named.
    #[test]
    fn only_a_missing_poly_globals_is_built_in() {
        let import = "#moj_import <polytone_globals.glsl>\n";
        let dir = "minecraft:shaders/core/";
        let globals = "minecraft:shaders/include/polytone_globals.glsl";
        let built_in = expand_imports(import, dir, &|_| Ok(None)).unwrap();
        let pasted: String = (sunrise_polytone::globals::GLSL.lines())
            .map(|line| format!("{line}\n"))
            .collect();
        assert_eq!(built_in, format!("{pasted}\n"));
        let shipped = |id: &str| Ok((id == globals).then(|| "float shipped;\n".to_owned()));
        assert_eq!(
            expand_imports(import, dir, &shipped).unwrap().trim(),
            "float shipped;"
        );
        let unreadable = |id: &str| Err(format!("{id}: not text"));
        assert_eq!(
            expand_imports(import, dir, &unreadable),
            Err(ImportRefusal::Unreadable(format!("{globals}: not text")))
        );
        let other = expand_imports("#moj_import <fog.glsl>\n", dir, &|_| Ok(None));
        assert_eq!(
            other,
            Err(ImportRefusal::NotFound(
                "minecraft:shaders/include/fog.glsl".into()
            ))
        );
    }
}
