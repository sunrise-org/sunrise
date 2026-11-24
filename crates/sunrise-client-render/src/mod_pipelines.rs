//! Render pipelines mods register (`Hooks::register_render_pipelines`, NeoForge's
//! `RegisterRenderPipelinesEvent`): the registrations, kept once at start, and what each
//! resource load makes of them. A load reads every registered pipeline's fragment file
//! (`assets/<namespace>/shaders/pipeline/<path>.wgsl`), puts Sunrise's prelude before it and its
//! entry points after it, and checks the whole shader, so the renderer (`gfx::mod_pipelines`) is
//! only ever handed WGSL that compiles. A required pipeline the load cannot build fails the load;
//! an optional one is logged and left out. A frame reads none of this by name.
//! The shader's composition and checks live in `sunrise_mod_pipeline`, shared with
//! `cargo sunrise-mod check`.
use mod_api::{
    Identifier, PackGlslInput, PackPassSlot, PackUniformType, PackUniformValue, PipelineBlend,
    PipelineTexture, PipelineTextureError, RenderPipeline, RenderPipelineError, RenderPipelineRef,
    WorldDepth,
};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;
use std::sync::OnceLock;
use sunrise_mod_pipeline::{FOLDER, compose_with};

static REGISTERED: OnceLock<Vec<RenderPipeline>> = OnceLock::new();
static PACK_UNIFORMS: OnceLock<HashMap<&'static str, PackUniformType>> = OnceLock::new();

/// The built-in tables exported by `RenderPipelines.requiredPipelines` and `optionalPipelines`.
#[derive(serde::Deserialize)]
struct VanillaPipelines {
    required: HashSet<String>,
    optional: HashSet<String>,
}

#[derive(Debug)]
pub enum RegistrationError {
    Read {
        path: std::path::PathBuf,
        error: std::io::Error,
    },
    Parse {
        path: std::path::PathBuf,
        error: serde_json::Error,
    },
    InvalidIdTable(std::path::PathBuf),
    VanillaCollision {
        owner: String,
        id: Identifier,
        optional: bool,
    },
    DuplicateModId {
        earlier: String,
        owner: String,
        id: Identifier,
    },
    InvalidPackBinding(String),
    AlreadyPublished,
}

impl std::fmt::Display for RegistrationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("register_render_pipelines: ")?;
        match self {
            Self::Read { path, error } => write!(
                f,
                "could not read {}: {error}; prepare the client resources again",
                path.display()
            ),
            Self::Parse { path, error } => write!(
                f,
                "invalid {}: {error}; prepare the client resources again",
                path.display()
            ),
            Self::InvalidIdTable(path) => write!(
                f,
                "invalid pipeline ID table in {}; prepare the client resources again",
                path.display()
            ),
            Self::VanillaCollision {
                owner,
                id,
                optional,
            } => {
                let kind = if *optional { "optional" } else { "required" };
                write!(
                    f,
                    "mod {owner}: {kind} pipeline {id} collides with vanilla {kind} pipeline"
                )
            }
            Self::DuplicateModId { earlier, owner, id } => write!(
                f,
                "mod {owner}: pipeline {id} collides with earlier registration from mod {earlier}"
            ),
            Self::InvalidPackBinding(reason) => f.write_str(reason),
            Self::AlreadyPublished => f.write_str("pipeline registrations were already published"),
        }
    }
}

impl std::error::Error for RegistrationError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Read { error, .. } => Some(error),
            Self::Parse { error, .. } => Some(error),
            _ => None,
        }
    }
}

/// Validate and publish the mods' registrations once at startup, as `register_render_pipelines`
/// left them: each ID at most once and never colliding with vanilla's same-kind table.
/// A pipeline's place in the list is its slot.
pub fn register(
    pipelines: Vec<RenderPipeline>,
    vanilla_root: &Path,
) -> Result<(), RegistrationError> {
    if pipelines.is_empty() {
        return Ok(());
    }
    let path = vanilla_root.join("render-pipelines.json");
    let bytes = std::fs::read(&path).map_err(|error| RegistrationError::Read {
        path: path.clone(),
        error,
    })?;
    let vanilla: VanillaPipelines =
        serde_json::from_slice(&bytes).map_err(|error| RegistrationError::Parse {
            path: path.clone(),
            error,
        })?;
    if [&vanilla.required, &vanilla.optional]
        .into_iter()
        .any(|ids| {
            ids.is_empty()
                || ids
                    .iter()
                    .any(|id| Identifier::parse_namespaced(id).is_none())
        })
    {
        return Err(RegistrationError::InvalidIdTable(path));
    }
    let mut seen = BTreeMap::new();
    for pipeline in &pipelines {
        let table = if pipeline.optional {
            &vanilla.optional
        } else {
            &vanilla.required
        };
        if table.contains(&pipeline.id.to_string()) {
            return Err(RegistrationError::VanillaCollision {
                owner: pipeline.owner().to_owned(),
                id: pipeline.id.clone(),
                optional: pipeline.optional,
            });
        }
        if let Some(earlier) = seen.insert(&pipeline.id, pipeline.owner()) {
            return Err(RegistrationError::DuplicateModId {
                earlier: earlier.to_owned(),
                owner: pipeline.owner().to_owned(),
                id: pipeline.id.clone(),
            });
        }
    }
    validate_pack_bindings(&pipelines).map_err(RegistrationError::InvalidPackBinding)?;
    REGISTERED
        .set(pipelines)
        .map_err(|_| RegistrationError::AlreadyPublished)
}

/// Built-in Iris/OptiFine pack GLSL sampler, image, and uniform names that mod `PackBinding`
/// declarations may not claim. Samplers, images, and value uniforms share GLSL's `uniform`
/// identifier namespace, so both `PackDepthTarget` samplers and `PackUniformDecl` names are
/// checked against this full table.
fn reserved_pack_identifier(name: &str) -> Option<&'static str> {
    let is_color_img = name
        .strip_prefix("colorimg")
        .and_then(|n| n.parse::<usize>().ok())
        .is_some_and(|n| n < sunrise_shaders::BUFFERS);
    let is_shadow_img = name
        .strip_prefix("shadowcolorimg")
        .and_then(|n| n.parse::<usize>().ok())
        .is_some_and(|n| n < 8);
    if sunrise_shaders::buffer_index(name).is_some()
        || sunrise_shaders::shadow_color(name).is_some()
        || is_color_img
        || is_shadow_img
        || matches!(
            name,
            "depthtex0"
                | "depthtex1"
                | "depthtex2"
                | "gdepthtex"
                | "shadowtex0"
                | "shadowtex1"
                | "shadowtex0HW"
                | "shadowtex1HW"
                | "watershadow"
                | "shadow"
                | "noisetex"
                | "texture"
                | "gtexture"
                | "tex"
                | "lightmap"
                | "normals"
                | "specular"
                | sunrise_shaders::WATER_MASK
        )
    {
        return Some("built-in shader-pack sampler or image");
    }
    if sunrise_render_shaderpack::pack_uniforms::NAMES.contains(&name) {
        return Some("built-in shader-pack uniform");
    }
    None
}

/// Validate GLSL syntax, reserved Iris/OptiFine names, and cross-mod uniqueness across every
/// registered `RenderPipeline::pack_binding`. Multiple pipelines owned by the same mod may
/// share one `PackDepthTarget.id` (with identical samplers), compatible defines, and typed
/// uniforms; cross-mod collisions and GLSL uniform/sampler namespace collisions fail startup
/// naming the offending mod(s) and identifier.
fn validate_pack_bindings(pipelines: &[RenderPipeline]) -> Result<(), String> {
    let mut depth_targets: BTreeMap<&Identifier, (&str, &mod_api::PackDepthTarget)> =
        BTreeMap::new();
    let mut samplers: BTreeMap<&str, (&str, &Identifier)> = BTreeMap::new();
    // Value uniforms keyed by GLSL name: `(owner, ty, is_previous)`.
    let mut uniforms: BTreeMap<&str, (&str, mod_api::PackUniformType, bool)> = BTreeMap::new();
    let mut defines: BTreeMap<&str, (&str, Option<&str>)> = BTreeMap::new();

    for pipeline in pipelines {
        let Some(binding) = &pipeline.pack_binding else {
            continue;
        };
        let owner = pipeline.owner();
        binding.validate_syntax().map_err(|reason| {
            format!(
                "Invalid pack binding on render pipeline {} (registered by {}): {reason}",
                pipeline.id, owner
            )
        })?;

        if let Some(target) = &binding.depth_target {
            if let Some(&(first_owner, first_target)) = depth_targets.get(&target.id) {
                if first_owner != owner {
                    return Err(format!(
                        "Duplicate pack depth target registration: {} (registered by {} and {})",
                        target.id, first_owner, owner
                    ));
                }
                if first_target != target {
                    return Err(format!(
                        "Conflicting pack depth target declaration for {} in mod {}: shared targets must declare identical samplers",
                        target.id, owner
                    ));
                }
            } else {
                depth_targets.insert(&target.id, (owner, target));
            }
            for sampler in target
                .depth_samplers
                .iter()
                .chain(target.opaque_depth_samplers.iter())
            {
                if let Some(kind) = reserved_pack_identifier(sampler) {
                    return Err(format!(
                        "Reserved pack sampler name {:?} on render pipeline {} (registered by {}) collides with a {kind}",
                        sampler, pipeline.id, owner
                    ));
                }
                if let Some(&(u_owner, _, _)) = uniforms.get(sampler.as_str()) {
                    if u_owner != owner {
                        return Err(format!(
                            "Pack GLSL identifier collision on {:?}: registered as a uniform by {} and as a sampler by {}",
                            sampler, u_owner, owner
                        ));
                    }
                    return Err(format!(
                        "Conflicting pack GLSL identifier {:?} in mod {}: declared as both a uniform and a sampler",
                        sampler, owner
                    ));
                }
                if let Some(&(first_owner, first_target_id)) = samplers.get(sampler.as_str()) {
                    if first_owner != owner {
                        return Err(format!(
                            "Duplicate pack sampler registration: {:?} (registered by {} and {})",
                            sampler, first_owner, owner
                        ));
                    }
                    if first_target_id != &target.id {
                        return Err(format!(
                            "Conflicting pack sampler registration {:?} in mod {}: bound to both {} and {}",
                            sampler, owner, first_target_id, target.id
                        ));
                    }
                } else {
                    samplers.insert(sampler.as_str(), (owner, &target.id));
                }
            }
        }

        for uniform in &binding.frame_uniforms {
            let names = std::iter::once((uniform.name.as_str(), false))
                .chain(uniform.previous_name.as_deref().map(|n| (n, true)));
            for (name, is_previous) in names {
                if let Some(kind) = reserved_pack_identifier(name) {
                    return Err(format!(
                        "Reserved pack uniform name {:?} on render pipeline {} (registered by {}) collides with a {kind}",
                        name, pipeline.id, owner
                    ));
                }
                if let Some(&(s_owner, _)) = samplers.get(name) {
                    if s_owner != owner {
                        return Err(format!(
                            "Pack GLSL identifier collision on {:?}: registered as a sampler by {} and as a uniform by {}",
                            name, s_owner, owner
                        ));
                    }
                    return Err(format!(
                        "Conflicting pack GLSL identifier {:?} in mod {}: declared as both a sampler and a uniform",
                        name, owner
                    ));
                }
                if let Some(&(first_owner, first_ty, first_is_previous)) = uniforms.get(name) {
                    if first_owner != owner {
                        return Err(format!(
                            "Duplicate pack uniform registration: {:?} (registered by {} and {})",
                            name, first_owner, owner
                        ));
                    }
                    if first_ty != uniform.ty {
                        return Err(format!(
                            "Conflicting pack uniform type for {:?} in mod {}: declared as {:?} and {:?}",
                            name, owner, first_ty, uniform.ty
                        ));
                    }
                    if first_is_previous != is_previous {
                        return Err(format!(
                            "Conflicting pack uniform role for {:?} in mod {}: declared as both current and previous_name",
                            name, owner
                        ));
                    }
                } else {
                    uniforms.insert(name, (owner, uniform.ty, is_previous));
                }
            }
        }

        for (name, value) in &binding.defines {
            let val = value.as_deref();
            if let Some(&(first_owner, first_val)) = defines.get(name.as_str()) {
                if first_owner != owner {
                    return Err(format!(
                        "Duplicate pack define registration: {:?} (registered by {} and {})",
                        name, first_owner, owner
                    ));
                }
                if first_val != val {
                    return Err(format!(
                        "Conflicting pack define value for {:?} in mod {}: {:?} vs {:?}",
                        name, owner, first_val, val
                    ));
                }
            } else {
                defines.insert(name.as_str(), (owner, val));
            }
        }
    }
    Ok(())
}

/// The registered pipelines; none until registration publishes them.
pub fn registered() -> &'static [RenderPipeline] {
    REGISTERED.get().map_or(&[], Vec::as_slice)
}

/// A registered pipeline one resource load built: a shader that compiles and its fixed state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Built {
    /// The owning mod, not necessarily the pipeline namespace.
    pub owner: String,
    /// The pipeline's id, for the log.
    pub id: Identifier,
    /// The whole shader: prelude, the mod's file, entry points.
    pub shader: String,
    pub blend: PipelineBlend,
    pub depth: WorldDepth,
    /// Whether the load may go on without it ([`RenderPipeline::optional()`]).
    pub optional: bool,
    /// What its shader samples, if anything.
    pub texture: Option<BuiltTexture>,
}

/// A textured pipeline's texture as a load found it ([`PipelineTexture`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BuiltTexture {
    /// A sprite of the 2D atlas, by its name there (`block/stone`, `powah:block/energy_charge`),
    /// looked up when the renderer builds the pipeline.
    Sprite(String),
}

/// The registered pipelines as one resource load built them, by slot. An optional pipeline the
/// load refused has no shader here and resolves to [`RenderPipelineError::Invalid`].
#[derive(Clone, Debug, Default)]
pub struct Pipelines {
    keys: Vec<Identifier>,
    built: Vec<Option<Built>>,
    /// The render assets' generation this set was loaded for; 0 for none.
    generation: u64,
    invalid: std::sync::Arc<std::sync::Mutex<Vec<usize>>>,
}

impl Pipelines {
    /// Build every registered pipeline from `source`, for the load of `generation`. An optional
    /// pipeline whose file is missing or refused is logged with its reason and the others build;
    /// a required one fails the load, naming every required pipeline refused and why.
    pub fn load(
        source: &crate::resource_source::ResourceSource,
        generation: u64,
    ) -> Result<Self, String> {
        Self::build(registered(), generation, |file| {
            source.read_to_string(file).map_err(|e| e.to_string())
        })
    }

    /// Build every registered pipeline, reading fragment files with `read`. A pipeline's
    /// texture is named here; its sprite is looked up when the renderer builds it.
    pub(crate) fn build(
        registered: &[RenderPipeline],
        generation: u64,
        read: impl Fn(&str) -> Result<String, String>,
    ) -> Result<Self, String> {
        let mut required = Vec::new();
        let built = registered
            .iter()
            .map(|pipeline| {
                let file = fragment_file(&pipeline.fragment);
                // A refusal reads `<file>[:<line>:<column>]: <message>`, the shader's own
                // `Problem`s printed as the pack checker prints them. The checks run contained:
                // a panic in naga on a mod's shader refuses the pipeline, with no crash report.
                let loaded = read(&file)
                    .map_err(|cause| format!("{file}: {cause}"))
                    .and_then(|text| {
                        crate::crash::contain(|| {
                            compose_with(&file, &text, pipeline.texture.is_some())
                        })
                        .map_err(|panic| format!("{file}: checking it panicked: {panic}"))?
                        .map_err(|problems| {
                            problems
                                .iter()
                                .map(ToString::to_string)
                                .collect::<Vec<_>>()
                                .join("; ")
                        })
                    })
                    .and_then(|shader| {
                        let texture = match &pipeline.texture {
                            None => None,
                            Some(texture) => Some(load_texture(texture).map_err(|cause| {
                                format!("its texture: {}", texture_refusal(cause))
                            })?),
                        };
                        Ok((shader, texture))
                    });
                match loaded {
                    Ok((shader, texture)) => Some(Built {
                        owner: pipeline.owner().to_owned(),
                        id: pipeline.id.clone(),
                        shader,
                        blend: pipeline.blend,
                        depth: pipeline.depth,
                        optional: pipeline.optional,
                        texture,
                    }),
                    Err(reason) => {
                        let refusal = format!(
                            "render pipeline {} from mod {} refused: {reason}",
                            pipeline.id,
                            pipeline.owner()
                        );
                        if pipeline.optional {
                            log::warn!("Sunrise: {refusal}");
                        } else {
                            required.push(format!("required {refusal}"));
                        }
                        None
                    }
                }
            })
            .collect();
        if !required.is_empty() {
            return Err(required.join("; "));
        }
        Ok(Self {
            keys: registered.iter().map(|p| p.id.clone()).collect(),
            built,
            generation,
            invalid: Default::default(),
        })
    }

    /// Marks `slot` as one the GPU refused; false when it already was, so it is reported once.
    pub(crate) fn invalidate(&self, slot: usize) -> bool {
        let mut invalid = self.invalid.lock().unwrap_or_else(|e| e.into_inner());
        let fresh = !invalid.contains(&slot);
        if fresh {
            invalid.push(slot);
        }
        fresh
    }

    /// Makes `slot` usable again: a renderer that built it took the refusal of another over.
    pub(crate) fn revalidate(&self, slot: usize) {
        self.invalid
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .retain(|&invalid| invalid != slot);
    }

    /// The load this set belongs to: a renderer holding another builds its pipelines again.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Every slot in order, with what the load built for it.
    pub fn built(&self) -> &[Option<Built>] {
        &self.built
    }

    /// The handle of the registered pipeline `id` in this load.
    pub fn resolve(&self, id: &Identifier) -> Result<RenderPipelineRef, RenderPipelineError> {
        let slot = self
            .keys
            .iter()
            .position(|registered| registered == id)
            .ok_or(RenderPipelineError::Unknown)?;
        if self.built[slot].is_none()
            || self
                .invalid
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .contains(&slot)
        {
            return Err(RenderPipelineError::Invalid);
        }
        Ok(mod_api::__private::built::render_pipeline_ref(
            self.generation,
            slot as u32,
        ))
    }

    /// The slot a handle draws with, checked once when a batch is submitted.
    pub fn slot(&self, pipeline: RenderPipelineRef) -> Result<u32, RenderPipelineError> {
        use mod_api::__private::game::Fields;
        let fields = pipeline.fields();
        if fields.generation != self.generation {
            return Err(RenderPipelineError::Stale);
        }
        if self
            .invalid
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .contains(&(fields.slot as usize))
        {
            return Err(RenderPipelineError::Invalid);
        }
        match self.built.get(fields.slot as usize) {
            Some(Some(_)) => Ok(fields.slot),
            Some(None) => Err(RenderPipelineError::Invalid),
            None => Err(RenderPipelineError::Unknown),
        }
    }
}

/// `mymod:beam/glow` is `assets/mymod/shaders/pipeline/beam/glow.wgsl`.
fn fragment_file(fragment: &Identifier) -> String {
    format!(
        "assets/{}/{FOLDER}/{}.wgsl",
        fragment.namespace(),
        fragment.path()
    )
}

/// `texture` as a load finds it, or its named refusal.
fn load_texture(texture: &PipelineTexture) -> Result<BuiltTexture, PipelineTextureError> {
    match texture {
        PipelineTexture::Sprite(id) => {
            // A file's path or name is not a sprite key.
            let path = id.path();
            if path.starts_with("textures/") || path.ends_with(".png") {
                return Err(PipelineTextureError::NotASpriteKey);
            }
            // Named as the 2D atlas names the blocks and items atlases' sprites.
            Ok(BuiltTexture::Sprite(match id.namespace() {
                "minecraft" => path.to_owned(),
                _ => id.to_string(),
            }))
        }
        // A texture kind a later API adds is one this game has no route for yet.
        _ => Err(PipelineTextureError::NoSuchTextureFile),
    }
}

/// The diagnostics line of a refused texture: its named cause, then what it means.
pub fn texture_refusal(cause: PipelineTextureError) -> String {
    format!("{cause:?}: {cause}")
}

/// Convert a mod-API [`PackUniformType`] to `sunrise-render-shaderpack`'s [`ModUniformType`].
pub fn mod_uniform_type(
    ty: PackUniformType,
) -> sunrise_render_shaderpack::pack_post::ModUniformType {
    use sunrise_render_shaderpack::pack_post::ModUniformType as M;
    match ty {
        PackUniformType::Float => M::Float,
        PackUniformType::Int => M::Int,
        PackUniformType::Vec2 => M::Vec2,
        PackUniformType::Vec3 => M::Vec3,
        PackUniformType::Vec4 => M::Vec4,
        PackUniformType::Mat3 => M::Mat3,
        PackUniformType::Mat4 => M::Mat4,
        _ => M::Float,
    }
}

/// Convert a mod-API [`PackUniformValue`] to `sunrise-render-shaderpack`'s [`ModUniformValue`].
pub fn mod_uniform_value(
    value: PackUniformValue,
) -> sunrise_render_shaderpack::pack_post::ModUniformValue {
    use sunrise_render_shaderpack::pack_post::ModUniformValue as M;
    match value {
        PackUniformValue::Float(v) => M::Float(v),
        PackUniformValue::Int(v) => M::Int(v),
        PackUniformValue::Vec2(v) => M::Vec2(v),
        PackUniformValue::Vec3(v) => M::Vec3(v),
        PackUniformValue::Vec4(v) => M::Vec4(v),
        PackUniformValue::Mat3(v) => M::Mat3(v),
        PackUniformValue::Mat4(v) => M::Mat4(v),
        _ => M::Float(0.0),
    }
}

fn mod_pass_slot(slot: PackPassSlot) -> sunrise_render_shaderpack::pack_post::ModPassSlot {
    use sunrise_render_shaderpack::pack_post::ModPassSlot as S;
    match slot {
        PackPassSlot::Shadow => S::Shadow,
        PackPassSlot::AfterSky => S::AfterSky,
        PackPassSlot::AfterOpaqueGbuffers => S::AfterOpaqueGbuffers,
        PackPassSlot::AfterTranslucentGbuffers => S::AfterTranslucentGbuffers,
        PackPassSlot::AfterWeather => S::AfterWeather,
        _ => S::AfterOpaqueGbuffers,
    }
}

fn shader_group_for_slot(slot: PackPassSlot) -> sunrise_shaders::Group {
    match slot {
        PackPassSlot::Shadow => sunrise_shaders::Group::Shadow,
        PackPassSlot::AfterSky | PackPassSlot::AfterOpaqueGbuffers => {
            sunrise_shaders::Group::Gbuffers
        }
        PackPassSlot::AfterTranslucentGbuffers | PackPassSlot::AfterWeather => {
            sunrise_shaders::Group::Translucent
        }
        _ => sunrise_shaders::Group::Gbuffers,
    }
}

fn pack_blend_for(blend: PipelineBlend) -> Option<sunrise_shaders::Blend> {
    use sunrise_shaders::Factor;
    match blend {
        PipelineBlend::Translucent => Some(sunrise_shaders::TRANSLUCENT_BLEND),
        PipelineBlend::Additive => Some([Factor::SrcAlpha, Factor::One, Factor::Zero, Factor::One]),
        _ => Some(sunrise_shaders::TRANSLUCENT_BLEND),
    }
}

fn glsl_input_expr(input: &PackGlslInput) -> String {
    match (input.source.as_str(), input.ty) {
        ("position" | "pos", PackUniformType::Vec3) => "mod_position".to_owned(),
        ("position" | "pos", PackUniformType::Vec4) => "vec4(mod_position, 1.0)".to_owned(),
        ("uv" | "uv0", PackUniformType::Vec2) => "mod_uv".to_owned(),
        ("uv" | "uv0", PackUniformType::Vec4) => "vec4(mod_uv, 0.0, 1.0)".to_owned(),
        ("color", PackUniformType::Vec4) => "mod_color".to_owned(),
        ("color", PackUniformType::Vec3) => "mod_color.rgb".to_owned(),
        ("normal", PackUniformType::Vec3) => "vec3(0.0, 1.0, 0.0)".to_owned(),
        ("light" | "uv2", PackUniformType::Vec2) => "vec2(240.0, 240.0)".to_owned(),
        (_, PackUniformType::Float) => "0.0".to_owned(),
        (_, PackUniformType::Int) => "0".to_owned(),
        (_, PackUniformType::Vec2) => "vec2(0.0)".to_owned(),
        (_, PackUniformType::Vec3) => "vec3(0.0)".to_owned(),
        (_, PackUniformType::Vec4) => "vec4(0.0)".to_owned(),
        (_, PackUniformType::Mat3) => "mat3(1.0)".to_owned(),
        (_, PackUniformType::Mat4) => "mat4(1.0)".to_owned(),
        _ => "0.0".to_owned(),
    }
}

fn mod_pipeline_gbuffers_inputs(
    binding: Option<&mod_api::PackBinding>,
) -> sunrise_shaders::GbuffersInputs {
    let mut attributes = vec![
        ("vaPosition".to_owned(), "mod_position".to_owned()),
        ("vaColor".to_owned(), "mod_color".to_owned()),
        ("vaUV0".to_owned(), "mod_uv".to_owned()),
        ("vaUV2".to_owned(), "ivec2(240, 240)".to_owned()),
        ("vaNormal".to_owned(), "vec3(0.0, 1.0, 0.0)".to_owned()),
    ];
    if let Some(binding) = binding {
        for input in &binding.glsl_bindings {
            let expr = glsl_input_expr(input);
            if let Some(existing) = attributes.iter_mut().find(|(k, _)| k == &input.glsl_name) {
                existing.1 = expr;
            } else {
                attributes.push((input.glsl_name.clone(), expr));
            }
        }
    }
    sunrise_shaders::GbuffersInputs {
        declarations: "layout(location = 0) in vec3 mod_position;\n\
             layout(location = 1) in vec2 mod_uv;\n\
             layout(location = 2) in vec4 mod_color;\n"
            .to_owned(),
        body: "    hz_Vertex = vec4(mod_position, 1.0);\n\
             hz_Color = mod_color;\n\
             hz_MultiTexCoord0 = vec4(mod_uv, 0.0, 1.0);\n\
             hz_MultiTexCoord1 = vec4(240.0, 240.0, 0.0, 1.0);\n\
             hz_MultiTexCoord2 = hz_MultiTexCoord1;\n\
             hz_Normal = vec3(0.0, 1.0, 0.0);\n"
            .to_owned(),
        attributes,
        ..Default::default()
    }
}

/// Deduplicated shader-pack preprocessor defines across all registered pipelines.
pub fn pack_defines() -> Vec<(&'static str, &'static str)> {
    pack_defines_of(registered())
}

pub(crate) fn pack_defines_of(pipelines: &[RenderPipeline]) -> Vec<(&str, &str)> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for pipeline in pipelines {
        let Some(binding) = &pipeline.pack_binding else {
            continue;
        };
        for (name, value) in &binding.defines {
            if seen.insert(name.as_str()) {
                out.push((name.as_str(), value.as_deref().unwrap_or("")));
            }
        }
    }
    out
}

/// Shader-pack program resolution requests for every registered pipeline with a pack slot
/// and candidate program chain.
pub fn pack_program_requests() -> Vec<sunrise_shaders::ModPackProgramRequest> {
    pack_program_requests_of(registered())
}

pub(crate) fn pack_program_requests_of(
    pipelines: &[RenderPipeline],
) -> Vec<sunrise_shaders::ModPackProgramRequest> {
    let mut out = Vec::new();
    for (slot, pipeline) in pipelines.iter().enumerate() {
        let Some(pass_slot) = pipeline.effective_pack_slot() else {
            continue;
        };
        let programs: Vec<String> = pipeline
            .effective_pack_programs()
            .into_iter()
            .map(str::to_owned)
            .collect();
        if programs.is_empty() {
            continue;
        }
        out.push(sunrise_shaders::ModPackProgramRequest {
            slot,
            programs,
            group: shader_group_for_slot(pass_slot),
            inputs: mod_pipeline_gbuffers_inputs(pipeline.pack_binding.as_ref()),
            default_blend: match pass_slot {
                PackPassSlot::AfterOpaqueGbuffers | PackPassSlot::Shadow => None,
                PackPassSlot::AfterSky
                | PackPassSlot::AfterTranslucentGbuffers
                | PackPassSlot::AfterWeather => pack_blend_for(pipeline.blend),
                _ => None,
            },
        });
    }
    out
}

/// Owned runtime descriptors (`depth_targets`, `uniforms`, `pipelines`) for `PackPost`.
pub fn pack_runtime_specs() -> sunrise_render_shaderpack::pack_post::OwnedModPackRuntime {
    pack_runtime_specs_of(registered())
}

pub(crate) fn pack_runtime_specs_of(
    pipelines: &[RenderPipeline],
) -> sunrise_render_shaderpack::pack_post::OwnedModPackRuntime {
    use sunrise_render_shaderpack::pack_post::{
        ModDepthTargetDecl, ModPipelinePackSpec, ModUniformDecl, OwnedModPackRuntime,
    };
    let mut depth_targets = Vec::new();
    let mut depth_target_ids: Vec<Identifier> = Vec::new();
    let mut uniforms = Vec::new();
    let mut seen_uniforms = HashSet::new();
    let mut specs = Vec::new();

    for (slot, pipeline) in pipelines.iter().enumerate() {
        if let Some(binding) = &pipeline.pack_binding {
            for uniform in &binding.frame_uniforms {
                if seen_uniforms.insert(uniform.name.as_str()) {
                    uniforms.push(ModUniformDecl {
                        name: uniform.name.clone(),
                        ty: mod_uniform_type(uniform.ty),
                        previous_name: uniform.previous_name.clone(),
                    });
                }
            }
        }
        let Some(pass_slot) = pipeline.effective_pack_slot() else {
            continue;
        };
        if pipeline.effective_pack_programs().is_empty() {
            continue;
        }
        let depth_target = pipeline
            .pack_binding
            .as_ref()
            .and_then(|b| b.depth_target.as_ref())
            .map(|target| {
                if let Some(idx) = depth_target_ids.iter().position(|id| id == &target.id) {
                    idx
                } else {
                    let idx = depth_targets.len();
                    depth_target_ids.push(target.id.clone());
                    depth_targets.push(ModDepthTargetDecl {
                        label: target.id.to_string(),
                        depth_samplers: target.depth_samplers.clone(),
                        opaque_depth_samplers: target.opaque_depth_samplers.clone(),
                    });
                    idx
                }
            });
        let depth_test = !matches!(pipeline.depth, WorldDepth::ThroughWalls);
        let depth_write = depth_test
            && (depth_target.is_some()
                || matches!(
                    pass_slot,
                    PackPassSlot::AfterOpaqueGbuffers | PackPassSlot::Shadow
                ));
        specs.push(ModPipelinePackSpec {
            slot,
            pass_slot: mod_pass_slot(pass_slot),
            depth_target,
            depth_write,
            depth_test,
        });
    }

    OwnedModPackRuntime {
        depth_targets,
        uniforms,
        pipelines: specs,
    }
}

fn pack_uniform_map(pipelines: &[RenderPipeline]) -> HashMap<&str, PackUniformType> {
    let mut map = HashMap::new();
    for pipeline in pipelines {
        let Some(binding) = &pipeline.pack_binding else {
            continue;
        };
        for uniform in &binding.frame_uniforms {
            map.entry(uniform.name.as_str()).or_insert(uniform.ty);
        }
    }
    map
}

/// Look up the registered `'static` name and [`PackUniformType`] of a pack frame uniform in O(1)
/// without per-frame allocation.
pub fn lookup_pack_uniform(name: &str) -> Option<(&'static str, PackUniformType)> {
    let pipelines = REGISTERED.get()?;
    let table = PACK_UNIFORMS.get_or_init(|| pack_uniform_map(pipelines));
    table.get_key_value(name).map(|(&k, &ty)| (k, ty))
}

/// Look up the declared [`PackUniformType`] of a registered pack frame uniform by name.
pub fn lookup_pack_uniform_type(name: &str) -> Option<PackUniformType> {
    lookup_pack_uniform(name).map(|(_, ty)| ty)
}

#[cfg(test)]
pub(crate) fn lookup_pack_uniform_type_in(
    pipelines: &[RenderPipeline],
    name: &str,
) -> Option<PackUniformType> {
    pack_uniform_map(pipelines).get(name).copied()
}

#[cfg(test)]
mod tests {
    use super::*;

    const GLOW: &str = "\
fn pipeline_fragment(input: PipelineInput) -> vec4<f32> {
    return vec4<f32>(input.color.rgb * (1.0 + sin(frame.time)), input.color.a * input.uv.y);
}
";

    fn pipeline(id: &str, fragment: &str) -> RenderPipeline {
        RenderPipeline::new(
            Identifier::parse(id).unwrap(),
            Identifier::parse(fragment).unwrap(),
            PipelineBlend::Additive,
            WorldDepth::ThroughWalls,
        )
    }
    fn id(text: &str) -> Identifier {
        Identifier::parse(text).unwrap()
    }

    #[test]
    fn an_optional_pipeline_that_fails_leaves_the_others_drawing_and_refuses_its_own_use() {
        let registered = [
            pipeline("example:glow", "example:glow"),
            pipeline("example:broken", "example:broken").optional(),
            pipeline("example:missing", "example:nested/missing").optional(),
            pipeline("example:driver", "example:glow").optional(),
        ];
        let read = |file: &str| match file {
            "assets/example/shaders/pipeline/glow.wgsl" => Ok(GLOW.to_owned()),
            "assets/example/shaders/pipeline/broken.wgsl" => Ok("fn nothing() {}".to_owned()),
            "assets/example/shaders/pipeline/nested/missing.wgsl" => Err("not found".to_owned()),
            other => panic!("read {other}"),
        };
        let set = Pipelines::build(&registered, 7, read).unwrap();

        let glow = set.resolve(&id("example:glow")).unwrap();
        assert_eq!(set.slot(glow), Ok(0));
        let built = set.built()[0].as_ref().unwrap();
        assert_eq!(
            (built.blend, built.depth),
            (PipelineBlend::Additive, WorldDepth::ThroughWalls)
        );
        assert_eq!(
            set.resolve(&id("example:broken")),
            Err(RenderPipelineError::Invalid)
        );
        assert_eq!(
            set.resolve(&id("example:missing")),
            Err(RenderPipelineError::Invalid)
        );
        assert_eq!(
            set.resolve(&id("example:never")),
            Err(RenderPipelineError::Unknown)
        );

        // The GPU turns down a pipeline the load built: it is reported once, a handle taken
        // before is refused, and the rest still draw.
        let driver = set.resolve(&id("example:driver")).unwrap();
        assert!(set.invalidate(3));
        assert!(!set.invalidate(3));
        assert_eq!(set.slot(driver), Err(RenderPipelineError::Invalid));
        assert_eq!(
            set.resolve(&id("example:driver")),
            Err(RenderPipelineError::Invalid)
        );
        assert_eq!(set.slot(glow), Ok(0));
    }

    #[test]
    fn a_required_pipeline_that_fails_fails_the_load_by_name() {
        let registered = [
            pipeline("example:glow", "example:glow"),
            pipeline("example:beam", "example:beam"),
            pipeline("example:extra", "example:extra").optional(),
        ];
        let before = Pipelines::build(&registered, 7, |_| Ok(GLOW.to_owned())).unwrap();
        let handle = before.resolve(&id("example:beam")).unwrap();

        // A pack broke the required beam and the optional extra: the load fails naming the
        // required one only, so nothing of it is published and the previous load still draws.
        let error = Pipelines::build(&registered, 8, |file| match file {
            "assets/example/shaders/pipeline/glow.wgsl" => Ok(GLOW.to_owned()),
            _ => Ok("garbage".to_owned()),
        })
        .unwrap_err();
        assert!(
            error.starts_with(
                "required render pipeline example:beam from mod  refused: assets/example/shaders/pipeline/beam.wgsl:1:1: "
            ),
            "{error}"
        );
        assert!(!error.contains("example:glow"), "{error}");
        assert!(!error.contains("example:extra"), "{error}");
        assert_eq!(before.slot(handle), Ok(1));
    }

    #[test]
    fn initial_required_failure_has_a_literal_message() {
        let registered = [pipeline("example:glow", "example:glow")];
        let error = Pipelines::build(&registered, 7, |_| Err("not found".to_owned())).unwrap_err();
        assert_eq!(
            error,
            "required render pipeline example:glow from mod  refused: assets/example/shaders/pipeline/glow.wgsl: not found"
        );
    }

    #[test]
    fn a_reload_makes_old_handles_stale_and_can_fix_or_break_a_pipeline() {
        let registered = [pipeline("example:glow", "example:glow").optional()];
        let before = Pipelines::build(&registered, 7, |_| Ok(GLOW.to_owned())).unwrap();
        let handle = before.resolve(&id("example:glow")).unwrap();

        // A pack replaced the file with one that doesn't compile: the pipeline is invalid in
        // the new load, and the old handle is stale there, not silently redirected.
        let broken = Pipelines::build(&registered, 8, |_| Ok("garbage".to_owned())).unwrap();
        assert_eq!(broken.slot(handle), Err(RenderPipelineError::Stale));
        assert_eq!(
            broken.resolve(&id("example:glow")),
            Err(RenderPipelineError::Invalid)
        );

        let fixed = Pipelines::build(&registered, 9, |_| Ok(GLOW.to_owned())).unwrap();
        assert_eq!(fixed.slot(handle), Err(RenderPipelineError::Stale));
        let again = fixed.resolve(&id("example:glow")).unwrap();
        assert_eq!(fixed.slot(again), Ok(0));
    }

    /// Sprites are named as the 2D atlas names them; an id naming a file is refused by name,
    /// and a required pipeline's refusal fails the load with the named cause.
    #[test]
    fn a_textured_pipeline_names_its_sprite_and_refuses_a_file_path() {
        use mod_api::PipelineTexture;
        let pipeline = |id: &str| {
            let name = Identifier::parse("test:glow").unwrap();
            RenderPipeline::new(
                name.clone(),
                name,
                PipelineBlend::Additive,
                WorldDepth::Tested,
            )
            .textured(PipelineTexture::Sprite(Identifier::parse(id).unwrap()))
        };
        let shader = |_: &str| Ok(GLOW.to_owned());
        let built = |id| {
            Pipelines::build(&[pipeline(id)], 1, shader)
                .map(|set| set.built()[0].as_ref().unwrap().texture.clone().unwrap())
        };
        assert_eq!(
            built("minecraft:block/stone"),
            Ok(BuiltTexture::Sprite("block/stone".into()))
        );
        assert_eq!(
            built("powah:block/energy_charge"),
            Ok(BuiltTexture::Sprite("powah:block/energy_charge".into()))
        );
        assert_eq!(
            load_texture(&PipelineTexture::Sprite(
                Identifier::parse("test:textures/glow").unwrap()
            )),
            Err(PipelineTextureError::NotASpriteKey)
        );
        assert_eq!(
            built("test:effect/glow.png"),
            Err("required render pipeline test:glow from mod  refused: its texture: NotASpriteKey: the id names a file, not a sprite key".into())
        );
    }

    #[test]
    fn register_validates_pack_bindings_collisions_reserved_names_and_ordering() {
        use mod_api::__private::game::RegistrationContextNew;
        use mod_api::{
            Hooks, ModEntry, PackBinding, PackDepthTarget, PackPassSlot, PackUniformDecl,
            PackUniformType, PipelineGeometryClass, RegisterRenderPipelinesEvent,
            RegistrationContext,
        };

        struct Custom(Box<dyn Fn(&mut RegisterRenderPipelinesEvent<'_>) + Send + Sync>);
        #[mod_api::__subscriptions]
        impl Hooks for Custom {
            fn register_render_pipelines(
                &self,
                _: &RegistrationContext,
                event: &mut RegisterRenderPipelinesEvent<'_>,
            ) {
                (self.0)(event);
            }
        }

        let run = |mods: Vec<ModEntry>| -> Result<Vec<RenderPipeline>, String> {
            let mut result = Err(String::new());
            mod_platform::testing::with_mods(mods.into(), || {
                let mut staged = Vec::new();
                mod_api::dispatch_register_render_pipelines(
                    &RegistrationContext::new(),
                    &mut staged,
                );
                result = validate_pack_bindings(&staged).map(|()| staged);
            });
            result
        };

        // Same mod sharing one auxiliary depth target, uniform, and define across opaque +
        // translucent pipelines succeeds, and preserves pack pass ordering.
        let shared_target = || {
            PackDepthTarget::new(id("alpha:aux_depth"))
                .depth_sampler("alphaDepthTex0")
                .opaque_depth_sampler("alphaDepthTex1")
        };
        let ok = run(vec![
            ModEntry::new(
                "alpha",
                Box::new(Custom(Box::new(move |ev| {
                    ev.register(
                        pipeline("alpha:opaque", "alpha:opaque")
                            .geometry_class(PipelineGeometryClass::Terrain)
                            .pack_binding(
                                PackBinding::new()
                                    .program("alpha_terrain")
                                    .slot(PackPassSlot::AfterOpaqueGbuffers)
                                    .define("ALPHA_LAYER")
                                    .define_value("ALPHA_MAT_LEAF", "1")
                                    .depth_target(shared_target())
                                    .frame_uniform(
                                        PackUniformDecl::new(
                                            "alphaProjection",
                                            PackUniformType::Mat4,
                                        )
                                        .with_previous("alphaPreviousProjection"),
                                    ),
                            ),
                    );
                    ev.register(
                        pipeline("alpha:water", "alpha:water")
                            .geometry_class(PipelineGeometryClass::Translucent)
                            .pack_binding(
                                PackBinding::new()
                                    .program("alpha_water")
                                    .program("alpha_terrain")
                                    .slot(PackPassSlot::AfterTranslucentGbuffers)
                                    .define("ALPHA_LAYER")
                                    .depth_target(shared_target())
                                    .frame_uniform(
                                        PackUniformDecl::new(
                                            "alphaProjection",
                                            PackUniformType::Mat4,
                                        )
                                        .with_previous("alphaPreviousProjection"),
                                    ),
                            ),
                    );
                }))),
            ),
            ModEntry::new(
                "beta",
                Box::new(Custom(Box::new(|ev| {
                    ev.register(
                        pipeline("beta:contraption", "beta:contraption")
                            .geometry_class(PipelineGeometryClass::Entity),
                    );
                }))),
            ),
        ])
        .unwrap();
        assert_eq!(ok.len(), 3);
        assert_eq!(
            ok[0].effective_pack_programs(),
            [
                "alpha_terrain",
                "gbuffers_terrain",
                "gbuffers_textured_lit",
                "gbuffers_textured",
                "gbuffers_basic",
            ]
        );
        assert_eq!(
            ok[1].effective_pack_programs(),
            [
                "alpha_water",
                "alpha_terrain",
                "gbuffers_water",
                "gbuffers_terrain",
                "gbuffers_textured_lit",
                "gbuffers_textured",
                "gbuffers_basic",
            ]
        );
        assert_eq!(
            ok[2].effective_pack_programs(),
            [
                "gbuffers_entities",
                "gbuffers_textured_lit",
                "gbuffers_textured",
                "gbuffers_basic",
            ]
        );
        assert!(ok[0].effective_pack_slot().unwrap() < ok[1].effective_pack_slot().unwrap());

        // Cross-mod sampler collision names both mods and the colliding sampler.
        let sampler_err = run(vec![
            ModEntry::new(
                "mod_one",
                Box::new(Custom(Box::new(|ev| {
                    ev.register(pipeline("mod_one:p", "mod_one:p").pack_binding(
                        PackBinding::new().depth_target(
                            PackDepthTarget::new(id("mod_one:d")).depth_sampler("sharedTex"),
                        ),
                    ));
                }))),
            ),
            ModEntry::new(
                "mod_two",
                Box::new(Custom(Box::new(|ev| {
                    ev.register(pipeline("mod_two:p", "mod_two:p").pack_binding(
                        PackBinding::new().depth_target(
                            PackDepthTarget::new(id("mod_two:d")).opaque_depth_sampler("sharedTex"),
                        ),
                    ));
                }))),
            ),
        ])
        .unwrap_err();
        assert!(sampler_err.contains("sharedTex"), "{sampler_err}");
        assert!(sampler_err.contains("mod_one"), "{sampler_err}");
        assert!(sampler_err.contains("mod_two"), "{sampler_err}");

        // Cross-mod uniform collision (including previous_name) names both mods.
        let uniform_err = run(vec![
            ModEntry::new(
                "mod_one",
                Box::new(Custom(Box::new(|ev| {
                    ev.register(
                        pipeline("mod_one:p", "mod_one:p").pack_binding(
                            PackBinding::new().frame_uniform(
                                PackUniformDecl::new("customProj", PackUniformType::Mat4)
                                    .with_previous("customPrevProj"),
                            ),
                        ),
                    );
                }))),
            ),
            ModEntry::new(
                "mod_two",
                Box::new(Custom(Box::new(|ev| {
                    ev.register(pipeline("mod_two:p", "mod_two:p").pack_binding(
                        PackBinding::new().frame_uniform(PackUniformDecl::new(
                            "customPrevProj",
                            PackUniformType::Mat4,
                        )),
                    ));
                }))),
            ),
        ])
        .unwrap_err();
        assert!(uniform_err.contains("customPrevProj"), "{uniform_err}");
        assert!(uniform_err.contains("mod_one"), "{uniform_err}");
        assert!(uniform_err.contains("mod_two"), "{uniform_err}");

        // Cross-mod define collision names both mods.
        let define_err = run(vec![
            ModEntry::new(
                "mod_one",
                Box::new(Custom(Box::new(|ev| {
                    ev.register(
                        pipeline("mod_one:p", "mod_one:p")
                            .pack_binding(PackBinding::new().define("SHARED_FLAG")),
                    );
                }))),
            ),
            ModEntry::new(
                "mod_two",
                Box::new(Custom(Box::new(|ev| {
                    ev.register(
                        pipeline("mod_two:p", "mod_two:p")
                            .pack_binding(PackBinding::new().define("SHARED_FLAG")),
                    );
                }))),
            ),
        ])
        .unwrap_err();
        assert!(define_err.contains("SHARED_FLAG"), "{define_err}");
        assert!(define_err.contains("mod_one"), "{define_err}");
        assert!(define_err.contains("mod_two"), "{define_err}");

        // Reserved vanilla/Iris samplers, built-in images, and uniforms are refused in either
        // sampler or uniform position (they share GLSL's `uniform` namespace).
        for reserved_sampler in [
            "depthtex0",
            "colortex0",
            "shadowtex0",
            "noisetex",
            "colorimg0",
            "colorimg15",
            "shadowcolorimg0",
            "shadowcolorimg1",
            "cameraPosition",
        ] {
            let err = run(vec![ModEntry::new(
                "mod_one",
                Box::new(Custom(Box::new(move |ev| {
                    ev.register(pipeline("mod_one:p", "mod_one:p").pack_binding(
                        PackBinding::new().depth_target(
                            PackDepthTarget::new(id("mod_one:d")).depth_sampler(reserved_sampler),
                        ),
                    ));
                }))),
            )])
            .unwrap_err();
            assert!(err.contains(reserved_sampler), "{err}");
            assert!(err.contains("mod_one"), "{err}");
        }
        for reserved_uniform in [
            "gbufferProjection",
            "cameraPosition",
            "worldTime",
            "depthtex0",
            "colorimg0",
            "shadowcolorimg0",
        ] {
            let err = run(vec![ModEntry::new(
                "mod_one",
                Box::new(Custom(Box::new(move |ev| {
                    ev.register(pipeline("mod_one:p", "mod_one:p").pack_binding(
                        PackBinding::new().frame_uniform(PackUniformDecl::new(
                            reserved_uniform,
                            PackUniformType::Mat4,
                        )),
                    ));
                }))),
            )])
            .unwrap_err();
            assert!(err.contains(reserved_uniform), "{err}");
            assert!(err.contains("mod_one"), "{err}");
        }

        // Same identifier claimed as both a sampler and a uniform is refused (same-mod and cross-mod).
        let same_mod_ns_err = run(vec![ModEntry::new(
            "mod_one",
            Box::new(Custom(Box::new(|ev| {
                ev.register(
                    pipeline("mod_one:p", "mod_one:p").pack_binding(
                        PackBinding::new()
                            .depth_target(
                                PackDepthTarget::new(id("mod_one:d")).depth_sampler("sharedId"),
                            )
                            .frame_uniform(PackUniformDecl::new(
                                "sharedId",
                                PackUniformType::Float,
                            )),
                    ),
                );
            }))),
        )])
        .unwrap_err();
        assert!(same_mod_ns_err.contains("sharedId"), "{same_mod_ns_err}");
        assert!(same_mod_ns_err.contains("mod_one"), "{same_mod_ns_err}");

        let cross_mod_ns_err =
            run(vec![
                ModEntry::new(
                    "mod_one",
                    Box::new(Custom(Box::new(|ev| {
                        ev.register(pipeline("mod_one:p", "mod_one:p").pack_binding(
                            PackBinding::new().frame_uniform(PackUniformDecl::new(
                                "crossNs",
                                PackUniformType::Mat4,
                            )),
                        ));
                    }))),
                ),
                ModEntry::new(
                    "mod_two",
                    Box::new(Custom(Box::new(|ev| {
                        ev.register(pipeline("mod_two:p", "mod_two:p").pack_binding(
                            PackBinding::new().depth_target(
                                PackDepthTarget::new(id("mod_two:d")).depth_sampler("crossNs"),
                            ),
                        ));
                    }))),
                ),
            ])
            .unwrap_err();
        assert!(cross_mod_ns_err.contains("crossNs"), "{cross_mod_ns_err}");
        assert!(cross_mod_ns_err.contains("mod_one"), "{cross_mod_ns_err}");
        assert!(cross_mod_ns_err.contains("mod_two"), "{cross_mod_ns_err}");

        // Within-mod uniform role collision across two pipelines (`name` vs `previous_name`).
        let role_err = run(vec![ModEntry::new(
            "mod_one",
            Box::new(Custom(Box::new(|ev| {
                ev.register(pipeline("mod_one:a", "mod_one:a").pack_binding(
                    PackBinding::new().frame_uniform(
                        PackUniformDecl::new("projA", PackUniformType::Mat4).with_previous("projB"),
                    ),
                ));
                ev.register(
                    pipeline("mod_one:b", "mod_one:b").pack_binding(
                        PackBinding::new()
                            .frame_uniform(PackUniformDecl::new("projB", PackUniformType::Mat4)),
                    ),
                );
            }))),
        )])
        .unwrap_err();
        assert!(role_err.contains("projB"), "{role_err}");
        assert!(role_err.contains("mod_one"), "{role_err}");

        // Invalid define syntax is refused at registration.
        let bad_define = run(vec![ModEntry::new(
            "mod_one",
            Box::new(Custom(Box::new(|ev| {
                ev.register(
                    pipeline("mod_one:p", "mod_one:p")
                        .pack_binding(PackBinding::new().define_value("VALID", "1\n#define BAD")),
                );
            }))),
        )])
        .unwrap_err();
        assert!(bad_define.contains("mod_one"), "{bad_define}");
        assert!(bad_define.contains("VALID"), "{bad_define}");
    }

    #[test]
    fn pack_runtime_helpers_deduplicate_targets_uniforms_and_defines() {
        use mod_api::{
            PackBinding, PackDepthTarget, PackGlslInput, PackUniformDecl, PipelineGeometryClass,
        };
        let shared_depth = PackDepthTarget::new(id("example:aux_depth"))
            .depth_sampler("modDepthTex0")
            .opaque_depth_sampler("modDepthTex1");
        let opaque = RenderPipeline::new(
            id("example:opaque"),
            id("example:opaque"),
            PipelineBlend::Translucent,
            WorldDepth::Tested,
        )
        .geometry_class(PipelineGeometryClass::Terrain)
        .pack_binding(
            PackBinding::new()
                .program("custom_terrain")
                .define("EXAMPLE_EXT")
                .define_value("EXAMPLE_LEVEL", "2")
                .depth_target(shared_depth.clone())
                .frame_uniform(
                    PackUniformDecl::new("modProj", PackUniformType::Mat4)
                        .with_previous("prevModProj"),
                )
                .glsl_binding(PackGlslInput::new(
                    "color",
                    "modColor",
                    PackUniformType::Vec4,
                )),
        );
        let water = RenderPipeline::new(
            id("example:water"),
            id("example:water"),
            PipelineBlend::Translucent,
            WorldDepth::Tested,
        )
        .geometry_class(PipelineGeometryClass::Translucent)
        .pack_binding(
            PackBinding::new()
                .program("custom_water")
                .define("EXAMPLE_EXT")
                .depth_target(shared_depth)
                .frame_uniform(PackUniformDecl::new("modNear", PackUniformType::Float)),
        );
        let uniform_only = RenderPipeline::new(
            id("example:uniform_only"),
            id("example:uniform_only"),
            PipelineBlend::Translucent,
            WorldDepth::Tested,
        )
        .pack_binding(
            PackBinding::new()
                .frame_uniform(PackUniformDecl::new("modFar", PackUniformType::Float)),
        );
        let plain = pipeline("example:plain", "example:plain");
        let list = [opaque, water, uniform_only, plain];

        let defines = pack_defines_of(&list);
        assert_eq!(defines, vec![("EXAMPLE_EXT", ""), ("EXAMPLE_LEVEL", "2")]);

        let reqs = pack_program_requests_of(&list);
        assert_eq!(reqs.len(), 2);
        assert_eq!(reqs[0].slot, 0);
        assert_eq!(reqs[0].programs[0], "custom_terrain");
        assert_eq!(reqs[0].programs[1], "gbuffers_terrain");
        assert_eq!(reqs[0].group, sunrise_shaders::Group::Gbuffers);
        assert_eq!(reqs[0].default_blend, None);
        assert!(
            reqs[0]
                .inputs
                .attributes
                .iter()
                .any(|(k, v)| k == "modColor" && v == "mod_color")
        );
        assert_eq!(reqs[1].slot, 1);
        assert_eq!(reqs[1].programs[0], "custom_water");
        assert_eq!(reqs[1].programs[1], "gbuffers_water");
        assert_eq!(reqs[1].group, sunrise_shaders::Group::Translucent);
        assert_eq!(
            reqs[1].default_blend,
            Some(sunrise_shaders::TRANSLUCENT_BLEND)
        );

        let runtime = pack_runtime_specs_of(&list);
        assert_eq!(runtime.depth_targets.len(), 1);
        assert_eq!(
            runtime.depth_targets[0].depth_samplers,
            vec!["modDepthTex0"]
        );
        assert_eq!(
            runtime.depth_targets[0].opaque_depth_samplers,
            vec!["modDepthTex1"]
        );
        assert_eq!(runtime.uniforms.len(), 3);
        assert!(runtime.uniforms.iter().any(|u| u.name == "modFar"));
        assert_eq!(runtime.pipelines.len(), 2);
        assert_eq!(runtime.pipelines[0].depth_target, Some(0));
        assert_eq!(runtime.pipelines[1].depth_target, Some(0));
        assert!(runtime.pipelines[0].depth_write);
        assert!(runtime.pipelines[1].depth_write);

        assert_eq!(
            lookup_pack_uniform_type_in(&list, "modProj"),
            Some(PackUniformType::Mat4)
        );
        assert_eq!(
            lookup_pack_uniform_type_in(&list, "modNear"),
            Some(PackUniformType::Float)
        );
        assert_eq!(lookup_pack_uniform_type_in(&list, "unknown"), None);
    }
}
