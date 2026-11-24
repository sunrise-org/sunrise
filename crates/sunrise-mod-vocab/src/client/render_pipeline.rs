//! Render pipelines a client mod registers as plain data, and the quads it draws with them.
use crate::{Identifier, WorldDepth};

/// A pipeline registered by `Hooks::register_render_pipelines`: a fragment shader from the
/// mod's assets and the fixed state it draws with. This is a restricted form: the mod supplies
/// the fragment stage and the host fixes the rest. Vanilla requires a vertex shader and a
/// primitive type; this API fixes both. The engine builds every GPU object.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
// no wire: local client render-pipeline registration descriptor.
pub struct RenderPipeline {
    owner: String,
    /// The pipeline's own name (`mymod:glow`), unique among mod pipelines.
    pub id: Identifier,
    /// The fragment shader (`mymod:glow` is `assets/mymod/shaders/pipeline/glow.wgsl`), as the
    /// mod's assets or a resource pack supply it.
    pub fragment: Identifier,
    /// What the pipeline's draws are made of.
    pub vertex_format: PipelineVertexFormat,
    /// How the shader's result joins the frame.
    pub blend: PipelineBlend,
    /// Whether nearer world surfaces hide the draw. No pipeline writes depth.
    pub depth: WorldDepth,
    /// Whether a resource load may go on without this pipeline (NeoForge's
    /// `registerOptionalPipeline`). A required pipeline (NeoForge's `registerPipeline`, the
    /// default) that a load cannot build fails the load: the game does not start, and a reload
    /// fails and keeps the previous resources. An optional one is logged once by id and resolves
    /// to [`RenderPipelineError::Invalid`] while the other pipelines draw.
    pub optional: bool,
    /// What the shader samples with `pipeline_texture(uv)`, if anything ([`Self::textured`]).
    /// `None`, the default, binds no texture: the shader reads only its input and `frame`.
    pub texture: Option<PipelineTexture>,
    /// What kind of world geometry this pipeline draws ([`Self::geometry_class`]), so an active
    /// Iris/OptiFine shader pack can route it through the matching standard `gbuffers_*`
    /// fallback chain. `None`, the default, draws only with the pipeline's own shader.
    pub geometry_class: Option<PipelineGeometryClass>,
    /// Optional shader-pack integration extras ([`Self::pack_binding`]): prepended pack program
    /// names, pass slot override, auxiliary depth target, per-frame uniforms, preprocessor
    /// defines, and GLSL compatibility bindings. All names are supplied by the mod; the engine
    /// hardcodes none.
    pub pack_binding: Option<PackBinding>,
}
impl RenderPipeline {
    /// Describe a required pipeline that draws [`PipelineVertexFormat::PositionTexColor`]
    /// geometry.
    pub fn new(
        id: Identifier,
        fragment: Identifier,
        blend: PipelineBlend,
        depth: WorldDepth,
    ) -> Self {
        Self {
            owner: String::new(),
            id,
            fragment,
            vertex_format: PipelineVertexFormat::PositionTexColor,
            blend,
            depth,
            optional: false,
            texture: None,
            geometry_class: None,
            pack_binding: None,
        }
    }
    /// The owning mod, assigned by the host after registration.
    pub fn owner(&self) -> &str {
        &self.owner
    }

    /// Let a resource load that cannot build this pipeline go on without it (NeoForge's
    /// `registerOptionalPipeline`).
    pub fn optional(mut self) -> Self {
        self.optional = true;
        self
    }

    /// Let the shader sample `texture` with `pipeline_texture(uv)` (vanilla's `Sampler0`, as
    /// `RenderPipeline.withSampler` declares it). A texture the load refuses
    /// ([`PipelineTextureError`]) refuses the pipeline as a missing shader does: a required one
    /// fails the load, an optional one is logged with the named cause and resolves to
    /// [`RenderPipelineError::Invalid`].
    pub fn textured(mut self, texture: PipelineTexture) -> Self {
        self.texture = Some(texture);
        self
    }

    /// Declare the world-geometry class this pipeline draws, so an active shader pack routes
    /// its draws through the matching standard `gbuffers_*` program chain before falling back
    /// to the pipeline's own shader.
    pub fn geometry_class(mut self, class: PipelineGeometryClass) -> Self {
        self.geometry_class = Some(class);
        self
    }

    /// Attach optional shader-pack integration extras (prepended program names, auxiliary
    /// depth target, frame uniforms, preprocessor defines, GLSL input bindings).
    pub fn pack_binding(mut self, binding: PackBinding) -> Self {
        self.pack_binding = Some(binding);
        self
    }

    /// The effective pack-program fallback chain in priority order: any names prepended in
    /// [`PackBinding::programs`], followed by [`PipelineGeometryClass::default_programs`],
    /// deduplicated while keeping the first occurrence. If none exist in the active pack or
    /// fail to translate, the pipeline falls back to its own shader.
    pub fn effective_pack_programs(&self) -> Vec<&str> {
        let mut chain = Vec::new();
        if let Some(binding) = &self.pack_binding {
            for name in &binding.programs {
                if !chain.contains(&name.as_str()) {
                    chain.push(name.as_str());
                }
            }
        }
        if let Some(class) = self.geometry_class {
            for &name in class.default_programs() {
                if !chain.contains(&name) {
                    chain.push(name);
                }
            }
        }
        chain
    }

    /// The effective shader-pack pass slot: [`PackBinding::slot`] when set, otherwise
    /// [`PipelineGeometryClass::default_slot`], or `None` when neither is declared.
    pub fn effective_pack_slot(&self) -> Option<PackPassSlot> {
        self.pack_binding
            .as_ref()
            .and_then(|binding| binding.slot)
            .or_else(|| self.geometry_class.map(PipelineGeometryClass::default_slot))
    }
}

/// What kind of world geometry a [`RenderPipeline`] draws, matching how Iris routes mod render
/// types through an active shader pack's standard `gbuffers_*` fallback chain.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
// no wire: local client render-pipeline geometry classification.
pub enum PipelineGeometryClass {
    /// Opaque or cutout block-like terrain (`gbuffers_terrain` fallback chain).
    Terrain,
    /// Translucent terrain or fluid geometry (`gbuffers_water` fallback chain).
    Translucent,
    /// Entity, block-entity, or contraption geometry (`gbuffers_entities` fallback chain).
    Entity,
    /// Textured lit particle geometry (`gbuffers_particles` fallback chain).
    Particles,
    /// Sky dome or horizon geometry (`gbuffers_skybasic` fallback chain).
    Sky,
    /// Cloud geometry (`gbuffers_clouds` fallback chain).
    Clouds,
    /// Rain, snow, or weather geometry (`gbuffers_weather` fallback chain).
    Weather,
    /// Wireframe, outline, or line geometry (`gbuffers_line` fallback chain).
    Lines,
    /// First-person held item or hand geometry (`gbuffers_hand` fallback chain).
    Hand,
}

impl PipelineGeometryClass {
    /// The standard Iris/OptiFine pack-program fallback chain for this geometry class, in
    /// priority order. When none exist in the active pack or fail to translate, the pipeline
    /// falls back to its own shader.
    pub const fn default_programs(self) -> &'static [&'static str] {
        match self {
            Self::Terrain => &[
                "gbuffers_terrain",
                "gbuffers_textured_lit",
                "gbuffers_textured",
                "gbuffers_basic",
            ],
            Self::Translucent => &[
                "gbuffers_water",
                "gbuffers_terrain",
                "gbuffers_textured_lit",
                "gbuffers_textured",
                "gbuffers_basic",
            ],
            Self::Entity => &[
                "gbuffers_entities",
                "gbuffers_textured_lit",
                "gbuffers_textured",
                "gbuffers_basic",
            ],
            Self::Particles => &[
                "gbuffers_particles",
                "gbuffers_textured_lit",
                "gbuffers_textured",
                "gbuffers_basic",
            ],
            Self::Sky => &["gbuffers_skybasic", "gbuffers_basic"],
            Self::Clouds => &["gbuffers_clouds", "gbuffers_textured", "gbuffers_basic"],
            Self::Weather => &[
                "gbuffers_weather",
                "gbuffers_textured_lit",
                "gbuffers_textured",
                "gbuffers_basic",
            ],
            Self::Lines => &["gbuffers_line", "gbuffers_basic"],
            Self::Hand => &[
                "gbuffers_hand",
                "gbuffers_textured_lit",
                "gbuffers_textured",
                "gbuffers_basic",
            ],
        }
    }

    /// The default shader-pack execution slot for this geometry class.
    pub const fn default_slot(self) -> PackPassSlot {
        match self {
            Self::Sky => PackPassSlot::AfterSky,
            Self::Terrain | Self::Entity | Self::Lines | Self::Hand => {
                PackPassSlot::AfterOpaqueGbuffers
            }
            Self::Translucent | Self::Particles | Self::Clouds => {
                PackPassSlot::AfterTranslucentGbuffers
            }
            Self::Weather => PackPassSlot::AfterWeather,
        }
    }
}

/// Execution slot inside an Iris/OptiFine shader pack's frame, ordered from earliest to latest.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[non_exhaustive]
// no wire: local client shader-pack execution slot.
pub enum PackPassSlot {
    /// Inside the pack's shadow pass, after vanilla shadow geometry and before `shadowcomp*`.
    Shadow,
    /// After the pack's sky pass (`gbuffers_skybasic`), before opaque terrain.
    AfterSky,
    /// After vanilla opaque gbuffers and before `deferred*`.
    AfterOpaqueGbuffers,
    /// After vanilla translucent gbuffers and before `composite*`.
    AfterTranslucentGbuffers,
    /// After weather gbuffers and before `composite*` and `final`.
    AfterWeather,
}

/// Optional Iris/OptiFine shader-pack extras attached to a [`RenderPipeline`]
/// ([`RenderPipeline::pack_binding`]). All identifiers are supplied by the mod; the engine
/// hardcodes none.
///
/// Startup refuses invalid GLSL identifiers, multi-line or comment-bearing define values,
/// collisions with reserved built-in pack samplers or uniforms, and cross-mod collisions on
/// pack samplers, uniforms, defines, or depth-target IDs (naming both mods).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
// no wire: local client render-pipeline shader-pack binding descriptor.
pub struct PackBinding {
    /// Pack program stems prepended ahead of [`PipelineGeometryClass::default_programs`].
    pub programs: Vec<String>,
    /// Optional pass slot override; defaults to [`PipelineGeometryClass::default_slot`].
    pub slot: Option<PackPassSlot>,
    /// Preprocessor defines exposed while compiling the active shader pack when this pipeline
    /// is active: `(name, optional_value)`. `name` must be a valid GLSL identifier (not
    /// `GL_`-prefixed) and `value`, when present, must be a non-empty single-line token.
    /// Toggling active defines triggers at most one shader-pack reload.
    pub defines: Vec<(String, Option<String>)>,
    /// Optional auxiliary depth-plane target, allocated and populated only while an active
    /// pack is bound and samples one of its declared samplers.
    pub depth_target: Option<PackDepthTarget>,
    /// Per-frame pack uniforms supplied by the mod under pack-side GLSL names. If a pack
    /// declares a uniform with a different GLSL type, the engine leaves it unbound and logs
    /// once without panicking.
    pub frame_uniforms: Vec<PackUniformDecl>,
    /// Mapping from the pipeline's vertex or per-draw inputs to pack-side GLSL names.
    pub glsl_bindings: Vec<PackGlslInput>,
}

impl PackBinding {
    /// An empty shader-pack binding with no extra programs, defines, depth target, or uniforms.
    pub fn new() -> Self {
        Self::default()
    }

    /// Prepend one pack program stem in front of the geometry class's default chain.
    pub fn program(mut self, name: impl Into<String>) -> Self {
        self.programs.push(name.into());
        self
    }

    /// Override the pack execution slot instead of using the geometry class default.
    pub fn slot(mut self, slot: PackPassSlot) -> Self {
        self.slot = Some(slot);
        self
    }

    /// Declare a preprocessor define `#define NAME` (no value) while this pipeline is active.
    pub fn define(mut self, name: impl Into<String>) -> Self {
        self.defines.push((name.into(), None));
        self
    }

    /// Declare a preprocessor define `#define NAME VALUE` while this pipeline is active.
    pub fn define_value(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.defines.push((name.into(), Some(value.into())));
        self
    }

    /// Attach an auxiliary depth-plane target shared by pipelines using the same target ID.
    pub fn depth_target(mut self, target: PackDepthTarget) -> Self {
        self.depth_target = Some(target);
        self
    }

    /// Declare a per-frame pack uniform supplied by this pipeline.
    pub fn frame_uniform(mut self, uniform: PackUniformDecl) -> Self {
        self.frame_uniforms.push(uniform);
        self
    }

    /// Declare one mapping from a pipeline input to a pack-side GLSL input or uniform name.
    pub fn glsl_binding(mut self, input: PackGlslInput) -> Self {
        self.glsl_bindings.push(input);
        self
    }

    /// Validate GLSL identifier syntax and single-line define token rules on all declared
    /// entries. Called by the engine during pipeline registration freeze.
    pub fn validate_syntax(&self) -> Result<(), String> {
        for program in &self.programs {
            if !is_valid_pack_program_name(program) {
                return Err(format!(
                    "invalid pack program name {program:?}: expected ASCII [A-Za-z0-9_]+"
                ));
            }
        }
        for (name, value) in &self.defines {
            if !is_valid_glsl_identifier(name) || name.starts_with("GL_") {
                return Err(format!(
                    "invalid pack define name {name:?}: expected a valid non-GL_ GLSL identifier"
                ));
            }
            if let Some(val) = value
                && !is_valid_glsl_define_value(val)
            {
                return Err(format!(
                    "invalid pack define value for {name}: {val:?} must be a non-empty single-line token without comments, backslashes, or '#'"
                ));
            }
        }
        if let Some(target) = &self.depth_target {
            let mut seen_samplers = std::collections::HashSet::new();
            for sampler in &target.depth_samplers {
                if !is_valid_glsl_identifier(sampler) || sampler.starts_with("gl_") {
                    return Err(format!(
                        "invalid pack depth sampler name {sampler:?}: expected a valid non-gl_ GLSL identifier"
                    ));
                }
                if !seen_samplers.insert(sampler.as_str()) {
                    return Err(format!(
                        "duplicate pack depth sampler {sampler:?} on target {}",
                        target.id
                    ));
                }
            }
            for sampler in &target.opaque_depth_samplers {
                if !is_valid_glsl_identifier(sampler) || sampler.starts_with("gl_") {
                    return Err(format!(
                        "invalid pack depth sampler name {sampler:?}: expected a valid non-gl_ GLSL identifier"
                    ));
                }
                if target.depth_samplers.iter().any(|s| s == sampler) {
                    return Err(format!(
                        "pack depth sampler {sampler:?} on target {} cannot be both a full depth sampler and an opaque depth sampler",
                        target.id
                    ));
                }
                if !seen_samplers.insert(sampler.as_str()) {
                    return Err(format!(
                        "duplicate pack opaque depth sampler {sampler:?} on target {}",
                        target.id
                    ));
                }
            }
        }
        let mut current_uniforms = std::collections::HashSet::new();
        let mut previous_uniforms = std::collections::HashSet::new();
        for uniform in &self.frame_uniforms {
            if !is_valid_glsl_identifier(&uniform.name) || uniform.name.starts_with("gl_") {
                return Err(format!(
                    "invalid pack uniform name {:?}: expected a valid non-gl_ GLSL identifier",
                    uniform.name
                ));
            }
            if previous_uniforms.contains(uniform.name.as_str()) {
                return Err(format!(
                    "pack uniform {:?} collides with another uniform's previous_name",
                    uniform.name
                ));
            }
            current_uniforms.insert(uniform.name.as_str());
            if let Some(prev) = &uniform.previous_name {
                if !is_valid_glsl_identifier(prev) || prev.starts_with("gl_") {
                    return Err(format!(
                        "invalid pack previous uniform name {prev:?}: expected a valid non-gl_ GLSL identifier"
                    ));
                }
                if prev == &uniform.name {
                    return Err(format!(
                        "pack uniform {:?} cannot reuse its own name as previous_name",
                        uniform.name
                    ));
                }
                if current_uniforms.contains(prev.as_str()) {
                    return Err(format!(
                        "pack previous uniform {prev:?} collides with another uniform's current name"
                    ));
                }
                previous_uniforms.insert(prev.as_str());
            }
        }
        for input in &self.glsl_bindings {
            if !is_valid_glsl_identifier(&input.source) {
                return Err(format!(
                    "invalid pack GLSL binding source {:?}: expected a valid GLSL identifier",
                    input.source
                ));
            }
            if !is_valid_glsl_identifier(&input.glsl_name) {
                return Err(format!(
                    "invalid pack GLSL binding target {:?}: expected a valid GLSL identifier",
                    input.glsl_name
                ));
            }
        }
        Ok(())
    }
}

/// An auxiliary depth target declared by a [`PackBinding`]. Multiple pipelines owned by the
/// same mod may reference the same [`Self::id`] to share one `Depth32Float` attachment (cleared
/// to OpenGL depth `1.0`, matching `depthtex0`). Allocated and populated only while an active
/// pack is bound and samples one of its declared samplers.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
// no wire: local client shader-pack auxiliary depth target descriptor.
pub struct PackDepthTarget {
    /// Mod-namespaced key identifying the shared depth attachment.
    pub id: Identifier,
    /// Sampler names exposed to pack passes for full depth after draws to this target.
    pub depth_samplers: Vec<String>,
    /// Optional sampler names exposed for an opaque-depth copy taken after
    /// [`PackPassSlot::AfterOpaqueGbuffers`] and before
    /// [`PackPassSlot::AfterTranslucentGbuffers`], populated only while sampled by the pack.
    pub opaque_depth_samplers: Vec<String>,
}

impl PackDepthTarget {
    /// Describe an auxiliary depth target keyed by `id`.
    pub fn new(id: Identifier) -> Self {
        Self {
            id,
            depth_samplers: Vec::new(),
            opaque_depth_samplers: Vec::new(),
        }
    }

    /// Expose the full depth buffer under GLSL sampler name `name`.
    pub fn depth_sampler(mut self, name: impl Into<String>) -> Self {
        self.depth_samplers.push(name.into());
        self
    }

    /// Expose the post-opaque depth snapshot under GLSL sampler name `name`.
    pub fn opaque_depth_sampler(mut self, name: impl Into<String>) -> Self {
        self.opaque_depth_samplers.push(name.into());
        self
    }
}

/// A per-frame pack uniform declared by a [`PackBinding`].
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
// no wire: local client shader-pack frame uniform declaration.
pub struct PackUniformDecl {
    /// GLSL uniform name read by the active shader pack.
    pub name: String,
    /// GLSL value type supplied by the mod.
    pub ty: PackUniformType,
    /// Optional previous-frame uniform alias auto-tracked by the engine.
    pub previous_name: Option<String>,
}

impl PackUniformDecl {
    /// Declare a per-frame pack uniform of type `ty`.
    pub fn new(name: impl Into<String>, ty: PackUniformType) -> Self {
        Self {
            name: name.into(),
            ty,
            previous_name: None,
        }
    }

    /// Also expose the previous frame's value under `previous_name`.
    pub fn with_previous(mut self, previous_name: impl Into<String>) -> Self {
        self.previous_name = Some(previous_name.into());
        self
    }
}

/// GLSL value type for a [`PackUniformDecl`] or [`PackGlslInput`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
// no wire: local client shader-pack uniform GLSL type selector.
pub enum PackUniformType {
    /// `float`
    Float,
    /// `int`
    Int,
    /// `vec2`
    Vec2,
    /// `vec3`
    Vec3,
    /// `vec4`
    Vec4,
    /// `mat3`
    Mat3,
    /// `mat4`
    Mat4,
}

/// How an active shader pack binds a registered [`RenderPipeline`], reported by
/// [`crate::WorldDrawContext::pack_override_state`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
// no wire: local client query value describing active shader-pack override state.
pub enum PackOverrideState {
    /// No translated shader pack is active.
    NoPack,
    /// A shader pack is active, but the pipeline was not bound to a translated pack program
    /// (either it has no [`PackBinding`] or no candidate program compiled); the mod's own shader
    /// runs instead.
    UnboundPack,
    /// The active shader pack translated and bound a pack program for this pipeline.
    Bound,
}

/// Per-frame value for a mod-declared shader-pack uniform ([`PackUniformDecl`]),
/// supplied through [`crate::WorldDrawContext::set_pack_uniform`].
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
// no wire: local client shader-pack per-frame uniform value.
pub enum PackUniformValue {
    /// GLSL `float`
    Float(f32),
    /// GLSL `int`
    Int(i32),
    /// GLSL `vec2`
    Vec2([f32; 2]),
    /// GLSL `vec3`
    Vec3([f32; 3]),
    /// GLSL `vec4`
    Vec4([f32; 4]),
    /// GLSL `mat3` (column-major)
    Mat3([[f32; 3]; 3]),
    /// GLSL `mat4` (column-major)
    Mat4([[f32; 4]; 4]),
}

impl PackUniformValue {
    /// The [`PackUniformType`] matching this value's variant.
    pub const fn ty(&self) -> PackUniformType {
        match self {
            Self::Float(_) => PackUniformType::Float,
            Self::Int(_) => PackUniformType::Int,
            Self::Vec2(_) => PackUniformType::Vec2,
            Self::Vec3(_) => PackUniformType::Vec3,
            Self::Vec4(_) => PackUniformType::Vec4,
            Self::Mat3(_) => PackUniformType::Mat3,
            Self::Mat4(_) => PackUniformType::Mat4,
        }
    }
}

/// Mapping from a pipeline vertex attribute or per-draw value to a pack-side GLSL input name.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
// no wire: local client shader-pack GLSL input binding descriptor.
pub struct PackGlslInput {
    /// Pipeline-side attribute or per-draw input name.
    pub source: String,
    /// Target GLSL identifier seen by the shader pack program (e.g. `gl_Vertex` or a named
    /// attribute or uniform).
    pub glsl_name: String,
    /// GLSL value type of the bound input.
    pub ty: PackUniformType,
}

impl PackGlslInput {
    /// Map pipeline input `source` to pack-side GLSL name `glsl_name` of type `ty`.
    pub fn new(
        source: impl Into<String>,
        glsl_name: impl Into<String>,
        ty: PackUniformType,
    ) -> Self {
        Self {
            source: source.into(),
            glsl_name: glsl_name.into(),
            ty,
        }
    }
}

/// Whether `name` is a valid ASCII GLSL identifier (`[A-Za-z_][A-Za-z0-9_]*`).
fn is_valid_glsl_identifier(name: &str) -> bool {
    let mut bytes = name.bytes();
    let Some(first) = bytes.next() else {
        return false;
    };
    (first.is_ascii_alphabetic() || first == b'_')
        && bytes.all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

/// Whether `value` is a safe, non-empty single-line GLSL `#define` value token sequence:
/// no ASCII control characters (including newlines), no leading/trailing whitespace, no
/// backslash continuations, no preprocessor `#`, and no `//`, `/*`, or `*/` comment tokens.
fn is_valid_glsl_define_value(value: &str) -> bool {
    !value.is_empty()
        && value.trim() == value
        && !value.bytes().any(|b| b.is_ascii_control())
        && !value.contains('\\')
        && !value.contains('#')
        && !value.contains("//")
        && !value.contains("/*")
        && !value.contains("*/")
}

/// Whether `name` is a valid pack program stem (`[A-Za-z0-9_]+`, no path separators or dots).
fn is_valid_pack_program_name(name: &str) -> bool {
    !name.is_empty() && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

/// A texture a pipeline's shader samples ([`RenderPipeline::textured`]): always a rectangle of
/// the atlas the host binds, never a texture of its own. `pipeline_texture(uv)` maps `fract(uv)`
/// into that rectangle and loads one texel by integer coordinates at mip level 0 (nearest, no
/// filtering, so no neighbouring sprite's texel enters): `uv` 0 to 1 spans it once and beyond
/// that it repeats. It returns linear light (the mod render colour rule) with the texel's alpha.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
// no wire: local client render-pipeline texture binding.
pub enum PipelineTexture {
    /// A sprite of the game's block or item atlas, by its texture id: `powah:block/energy_charge`
    /// is `assets/powah/textures/block/energy_charge.png` as the `minecraft:blocks` atlas holds
    /// it, `minecraft:item/diamond` an item sprite. An animated sprite animates. A sprite in
    /// neither atlas refuses the pipeline ([`PipelineTextureError::NoSuchSprite`]).
    Sprite(Identifier),
}

/// Why a pipeline's texture was refused: the named cause its refusal is logged with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
// no wire: refusal of a local client render-pipeline texture load.
pub enum PipelineTextureError {
    /// No sprite of that id in the block or item atlas.
    NoSuchSprite,
    /// No texture file of that id in the mod's assets or a resource pack.
    NoSuchTextureFile,
    /// The id names a file (a `textures/` path or a `.png` name), not a sprite key.
    NotASpriteKey,
    /// A texture file that isn't an image the game can decode.
    Undecodable,
}

impl std::fmt::Display for PipelineTextureError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::NoSuchSprite => "no such sprite in the block or item atlas",
            Self::NoSuchTextureFile => "no such texture file",
            Self::NotASpriteKey => "the id names a file, not a sprite key",
            Self::Undecodable => "the texture file cannot be decoded",
        })
    }
}

impl std::error::Error for PipelineTextureError {}

/// The vertex data a pipeline's draws carry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
// no wire: local client render-pipeline vertex format.
pub enum PipelineVertexFormat {
    /// A position, a texture coordinate and a colour: what [`PipelineQuad`] gives.
    PositionTexColor,
}

/// How a pipeline's fragments join the frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
// no wire: local client render-pipeline blend mode.
pub enum PipelineBlend {
    /// Laid over the frame by the fragment's alpha, as [`crate::WorldQuad`] is.
    Translucent,
    /// Added to the frame, scaled by the fragment's alpha.
    Additive,
}

/// A registered pipeline as the current resources built it, resolved through
/// [`crate::RenderAssets::render_pipeline`]. Retain it between frames; resolve it again after a
/// successful resource reload.
// no wire: process-local handle to a client render pipeline.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
// exhaustive: private fields; only the engine resolves pipeline handles
pub struct RenderPipelineRef(pub(crate) u64, pub(crate) u32);

impl RenderPipelineRef {
    pub(crate) fn from_parts(generation: u64, slot: u32) -> Result<Self, &'static str> {
        if generation == 0 {
            return Err("unpublished pipeline generation");
        }
        Ok(Self(generation, slot))
    }
}

/// Why a pipeline could not be resolved or drawn with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
// no wire: refusal of a local client render-pipeline lookup or draw.
pub enum RenderPipelineError {
    /// No mod registered a pipeline of this id.
    Unknown,
    /// The pipeline is registered, but the current resources give it no usable shader. The log
    /// names the pipeline and the reason.
    Invalid,
    /// A successful resource reload superseded the handle's generation.
    Stale,
}

/// A two-sided quad drawn by a mod pipeline. Corners run top left, bottom left, bottom right,
/// top right as seen from the front.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
// no wire: local client render-pipeline quad primitive.
pub struct PipelineQuad {
    /// Four world-space corners in blocks, before camera subtraction.
    pub corners: [[f64; 3]; 4],
    /// Each corner's texture coordinate, handed to the shader as given.
    pub uv: [[f32; 2]; 4],
    /// Packed 0xAARRGGBB colour in vanilla's space; the shader reads it as linear light.
    pub color: u32,
}
impl PipelineQuad {
    /// Describe a quad; the draw context validates it when submitting the batch.
    pub fn new(corners: [[f64; 3]; 4], uv: [[f32; 2]; 4], color: u32) -> Self {
        Self { corners, uv, color }
    }
    /// A quad whose texture coordinates span 0 to 1: (0, 0) at the top left corner and (1, 1)
    /// at the bottom right.
    pub fn spanning(corners: [[f64; 3]; 4], color: u32) -> Self {
        Self::new(
            corners,
            [[0.0, 0.0], [0.0, 1.0], [1.0, 1.0], [1.0, 0.0]],
            color,
        )
    }
}

/// Why a pipeline draw call was refused; nothing from that call was appended.
///
/// Individual quads with non-finite coordinates or UVs are skipped without refusing valid quads
/// in the same slice, and quad storage grows dynamically into the GPU buffers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
// no wire: refusal of a local client render-pipeline draw submission.
pub enum PipelineDrawError {
    /// The handle is stale, or names no usable pipeline.
    Pipeline(RenderPipelineError),
}

impl crate::RegisterRenderPipelinesEvent<'_> {
    /// Register one render pipeline without exposing earlier registrations.
    ///
    /// The entry is attributed to this listener immediately. Any valid identifier is accepted
    /// here; duplicate IDs and vanilla collisions are refused when the engine freezes the
    /// registrations at startup.
    pub fn register(&mut self, mut entry: RenderPipeline) {
        entry.owner = self.mod_id.to_owned();
        self.pipelines.push(entry);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pipeline(id: &str) -> RenderPipeline {
        let id = Identifier::parse(id).unwrap();
        RenderPipeline::new(
            id.clone(),
            id,
            PipelineBlend::Translucent,
            WorldDepth::Tested,
        )
    }
    fn ids(pipelines: &[RenderPipeline]) -> Vec<String> {
        pipelines.iter().map(|p| p.id.to_string()).collect()
    }

    #[test]
    fn stamps_owner_on_new_pipelines_across_namespaces_and_preserves_duplicates_for_freeze() {
        let mut pipelines = Vec::new();
        let mut event = crate::RegisterRenderPipelinesEvent {
            mod_id: "first_mod",
            pipelines: &mut pipelines,
        };
        event.register(pipeline("other:glow"));
        event.mod_id = "owning_mod";
        for id in ["minecraft:glow", "other:beam", "other:glow", "mine:beam"] {
            event.register(pipeline(id));
        }
        assert_eq!(
            ids(&pipelines),
            [
                "other:glow".to_owned(),
                "minecraft:glow".to_owned(),
                "other:beam".to_owned(),
                "other:glow".to_owned(),
                "mine:beam".to_owned(),
            ]
        );
        assert_eq!(pipelines[0].owner(), "first_mod");
        assert_eq!(pipelines[1].owner(), "owning_mod");
        assert_eq!(pipelines[2].owner(), "owning_mod");
        assert_eq!(pipelines[3].owner(), "owning_mod");
        assert_eq!(pipelines[4].owner(), "owning_mod");
    }

    #[test]
    fn a_mod_can_register_more_than_vanillas_pipeline_count() {
        let mut pipelines = Vec::new();
        {
            let mut event = crate::RegisterRenderPipelinesEvent {
                mod_id: "mine",
                pipelines: &mut pipelines,
            };
            for i in 0..131 {
                event.register(pipeline(&format!("mine:p{i}")));
            }
        }
        assert_eq!(pipelines.len(), 131);
        assert_eq!(pipelines.last().unwrap().id.to_string(), "mine:p130");

        crate::RegisterRenderPipelinesEvent {
            mod_id: "next",
            pipelines: &mut pipelines,
        }
        .register(pipeline("next:glow"));
        assert_eq!(pipelines.len(), 132);
    }

    #[test]
    fn effective_pack_programs_prepends_custom_names_ahead_of_geometry_class_chain() {
        let plain = pipeline("example:plain");
        assert!(plain.effective_pack_programs().is_empty());
        assert_eq!(plain.effective_pack_slot(), None);

        let entity = pipeline("example:entity").geometry_class(PipelineGeometryClass::Entity);
        assert_eq!(
            entity.effective_pack_programs(),
            [
                "gbuffers_entities",
                "gbuffers_textured_lit",
                "gbuffers_textured",
                "gbuffers_basic",
            ]
        );
        assert_eq!(
            PipelineGeometryClass::Particles.default_programs(),
            [
                "gbuffers_particles",
                "gbuffers_textured_lit",
                "gbuffers_textured",
                "gbuffers_basic",
            ]
        );
        assert_eq!(
            entity.effective_pack_slot(),
            Some(PackPassSlot::AfterOpaqueGbuffers)
        );

        let custom = pipeline("example:custom_water")
            .geometry_class(PipelineGeometryClass::Translucent)
            .pack_binding(
                PackBinding::new()
                    .program("custom_water")
                    .program("gbuffers_terrain")
                    .slot(PackPassSlot::AfterTranslucentGbuffers),
            );
        assert_eq!(
            custom.effective_pack_programs(),
            [
                "custom_water",
                "gbuffers_terrain",
                "gbuffers_water",
                "gbuffers_textured_lit",
                "gbuffers_textured",
                "gbuffers_basic",
            ]
        );
        assert_eq!(
            custom.effective_pack_slot(),
            Some(PackPassSlot::AfterTranslucentGbuffers)
        );
    }

    #[test]
    fn pack_pass_slots_follow_shader_pack_frame_order() {
        let slots = [
            PackPassSlot::Shadow,
            PackPassSlot::AfterSky,
            PackPassSlot::AfterOpaqueGbuffers,
            PackPassSlot::AfterTranslucentGbuffers,
            PackPassSlot::AfterWeather,
        ];
        assert!(slots.windows(2).all(|pair| pair[0] < pair[1]));
        assert_eq!(
            PipelineGeometryClass::Sky.default_slot(),
            PackPassSlot::AfterSky
        );
        assert_eq!(
            PipelineGeometryClass::Terrain.default_slot(),
            PackPassSlot::AfterOpaqueGbuffers
        );
        assert_eq!(
            PipelineGeometryClass::Translucent.default_slot(),
            PackPassSlot::AfterTranslucentGbuffers
        );
        assert_eq!(
            PipelineGeometryClass::Weather.default_slot(),
            PackPassSlot::AfterWeather
        );
    }

    #[test]
    fn pack_binding_syntax_rejects_bad_identifiers_and_multiline_defines() {
        let valid = PackBinding::new()
            .program("custom_terrain")
            .define("MOD_ACTIVE")
            .define_value("MOD_MATERIAL_LEAVES", "1")
            .depth_target(
                PackDepthTarget::new(Identifier::parse("example:aux_depth").unwrap())
                    .depth_sampler("modDepthTex0")
                    .opaque_depth_sampler("modDepthTex1"),
            )
            .frame_uniform(
                PackUniformDecl::new("modProjection", PackUniformType::Mat4)
                    .with_previous("modPreviousProjection"),
            )
            .glsl_binding(PackGlslInput::new(
                "position",
                "gl_Vertex",
                PackUniformType::Vec4,
            ));
        assert_eq!(valid.validate_syntax(), Ok(()));

        for bad_define in ["", "1BAD", "BAD-NAME", "GL_RESERVED", "has space"] {
            assert!(
                PackBinding::new()
                    .define(bad_define)
                    .validate_syntax()
                    .is_err(),
                "expected invalid define name {bad_define:?} to fail"
            );
        }
        for bad_val in [
            "",
            "   ",
            "1\n#define INJECT 1",
            "1\r0",
            "1 // comment",
            "1 /* comment */",
            "1 \\",
            "#include",
            " leading",
        ] {
            assert!(
                PackBinding::new()
                    .define_value("GOOD_NAME", bad_val)
                    .validate_syntax()
                    .is_err(),
                "expected invalid define value {bad_val:?} to fail"
            );
        }
        for bad_program in ["", "../escape", "gbuffers_terrain.fsh", "has space"] {
            assert!(
                PackBinding::new()
                    .program(bad_program)
                    .validate_syntax()
                    .is_err(),
                "expected invalid program {bad_program:?} to fail"
            );
        }
        assert!(
            PackBinding::new()
                .depth_target(
                    PackDepthTarget::new(Identifier::parse("example:aux_depth").unwrap())
                        .depth_sampler("gl_FragDepth")
                )
                .validate_syntax()
                .is_err()
        );
        assert!(
            PackBinding::new()
                .depth_target(
                    PackDepthTarget::new(Identifier::parse("example:aux_depth").unwrap())
                        .depth_sampler("sharedDepth")
                        .opaque_depth_sampler("sharedDepth")
                )
                .validate_syntax()
                .is_err()
        );
        assert!(
            PackBinding::new()
                .depth_target(
                    PackDepthTarget::new(Identifier::parse("example:aux_depth").unwrap())
                        .depth_sampler("dupDepth")
                        .depth_sampler("dupDepth")
                )
                .validate_syntax()
                .is_err()
        );
        assert!(
            PackBinding::new()
                .depth_target(
                    PackDepthTarget::new(Identifier::parse("example:aux_depth").unwrap())
                        .opaque_depth_sampler("dupOpaque")
                        .opaque_depth_sampler("dupOpaque")
                )
                .validate_syntax()
                .is_err()
        );
        assert!(
            PackBinding::new()
                .frame_uniform(
                    PackUniformDecl::new("modProj", PackUniformType::Mat4).with_previous("modProj")
                )
                .validate_syntax()
                .is_err()
        );
        assert!(
            PackBinding::new()
                .frame_uniform(
                    PackUniformDecl::new("projA", PackUniformType::Mat4).with_previous("projB")
                )
                .frame_uniform(PackUniformDecl::new("projB", PackUniformType::Mat4))
                .validate_syntax()
                .is_err()
        );
    }
}
