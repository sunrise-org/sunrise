//! The post-processing programs of one dimension, ready for the renderer: `prepare`, `deferred`,
//! `composite` and `final` with their numbered variants, in the order they run (spec 2.1), each
//! preprocessed and translated, with the buffers it writes and the flips that follow it, and the
//! buffer settings their files carry (6.1).
//!
//! Decisions (impl-plan.md):
//! - OQ-12: a dimension's folder (`world0`, `world-1`, `world1`) replaces the base folder for that
//!   dimension when it exists; every stage file is read from it.
//! - OQ-29: a program with a geometry stage is reported and treated as missing.
//! - A program that fails to translate is reported and treated as missing (2.5).
//! - Terrain draws with the first program of its fallback chain that loads (2.3).

use std::collections::BTreeSet;

use crate::config::{self, Config};
use crate::custom::Custom;
use crate::{
    ComputeProgram, eval_condition, preprocess, translate_composite, translate_compute,
    translate_gbuffers, translate_shadow,
};
use crate::{GbuffersInputs, Macros, Pack, Preprocessed, Program, Properties, Target};

/// Which group of 2.1 a program belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Group {
    /// Compute initialization at load and after target resize.
    Setup,
    /// Composite-style work before shadow rendering.
    Begin,
    Prepare,
    Deferred,
    Composite,
    Final,
    /// A gbuffers program: the game draws its geometry with it, before deferred.
    Gbuffers,
    /// A gbuffers program drawing translucent geometry, after deferred (2.1).
    Translucent,
    /// A shadow program: the game's geometry from the shadow light, into the shadow buffers.
    Shadow,
    /// `shadowcomp`..`shadowcomp99`: composite-style programs over the shadow buffers.
    ShadowComp,
}

impl Group {
    /// Whether the group writes the shadow buffers, where buffer n is shadowcolor n.
    pub fn shadow(self) -> bool {
        matches!(self, Group::Shadow | Group::ShadowComp)
    }
}

/// The samplers whose use turns the shadow pass on (2.5, 6.3).
const SHADOW_SAMPLERS: [&str; 9] = [
    "shadow",
    "watershadow",
    "shadowtex0",
    "shadowtex1",
    "shadowtex0HW",
    "shadowtex1HW",
    "shadowcolor",
    "shadowcolor0",
    "shadowcolor1",
];

/// A blend factor of `blend.<program>` (3.2), as GL names it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Factor {
    Zero,
    One,
    SrcColor,
    OneMinusSrcColor,
    DstColor,
    OneMinusDstColor,
    SrcAlpha,
    OneMinusSrcAlpha,
    DstAlpha,
    OneMinusDstAlpha,
    SrcAlphaSaturate,
}

impl Factor {
    fn parse(word: &str) -> Option<Self> {
        Some(match word {
            "ZERO" => Self::Zero,
            "ONE" => Self::One,
            "SRC_COLOR" => Self::SrcColor,
            "ONE_MINUS_SRC_COLOR" => Self::OneMinusSrcColor,
            "DST_COLOR" => Self::DstColor,
            "ONE_MINUS_DST_COLOR" => Self::OneMinusDstColor,
            "SRC_ALPHA" => Self::SrcAlpha,
            "ONE_MINUS_SRC_ALPHA" => Self::OneMinusSrcAlpha,
            "DST_ALPHA" => Self::DstAlpha,
            "ONE_MINUS_DST_ALPHA" => Self::OneMinusDstAlpha,
            "SRC_ALPHA_SATURATE" => Self::SrcAlphaSaturate,
            _ => return None,
        })
    }
}

/// Source and destination factors for colour, then for alpha.
pub type Blend = [Factor; 4];

/// The default blend of translucent geometry (3.2).
pub const TRANSLUCENT_BLEND: Blend = [
    Factor::SrcAlpha,
    Factor::OneMinusSrcAlpha,
    Factor::One,
    Factor::OneMinusSrcAlpha,
];

/// `gbuffers_water` and its fallbacks (2.3).
const WATER: [&str; 5] = [
    "gbuffers_water",
    "gbuffers_terrain",
    "gbuffers_textured_lit",
    "gbuffers_textured",
    "gbuffers_basic",
];

/// `gbuffers_terrain` and its fallbacks (2.3).
const TERRAIN: [&str; 4] = [
    "gbuffers_terrain",
    "gbuffers_textured_lit",
    "gbuffers_textured",
    "gbuffers_basic",
];

/// `gbuffers_terrain_solid` and its fallbacks (ShaderDoc's "Iris Additional Programs", as
/// #9137 quotes it): solid terrain, the quads over opaque texels.
const TERRAIN_SOLID: [&str; 5] = [
    "gbuffers_terrain_solid",
    "gbuffers_terrain",
    "gbuffers_textured_lit",
    "gbuffers_textured",
    "gbuffers_basic",
];

/// `gbuffers_terrain_cutout` and its fallbacks (the same): cutout terrain.
const TERRAIN_CUTOUT: [&str; 5] = [
    "gbuffers_terrain_cutout",
    "gbuffers_terrain",
    "gbuffers_textured_lit",
    "gbuffers_textured",
    "gbuffers_basic",
];

/// `gbuffers_line` and its fallback, as in Iris: lines such as the block outline.
const LINES: [&str; 2] = ["gbuffers_line", "gbuffers_basic"];

/// `gbuffers_skybasic` and its fallback (2.3).
const SKY: [&str; 2] = ["gbuffers_skybasic", "gbuffers_basic"];

/// `gbuffers_skytextured` and its fallbacks (2.3).
const SKY_TEXTURED: [&str; 3] = [
    "gbuffers_skytextured",
    "gbuffers_textured",
    "gbuffers_basic",
];

/// `gbuffers_entities` and its fallbacks (2.3).
const ENTITIES: [&str; 4] = [
    "gbuffers_entities",
    "gbuffers_textured_lit",
    "gbuffers_textured",
    "gbuffers_basic",
];

/// `gbuffers_block` and its fallbacks (2.3).
const BLOCK: [&str; 5] = [
    "gbuffers_block",
    "gbuffers_terrain",
    "gbuffers_textured_lit",
    "gbuffers_textured",
    "gbuffers_basic",
];

/// `gbuffers_clouds` and its fallbacks, as in Iris (its ShaderDoc's gbuffers table): without
/// any of them the game draws its clouds.
const CLOUDS: [&str; 3] = ["gbuffers_clouds", "gbuffers_textured", "gbuffers_basic"];

/// `gbuffers_hand` and its fallbacks (2.3).
const HAND: [&str; 4] = [
    "gbuffers_hand",
    "gbuffers_textured_lit",
    "gbuffers_textured",
    "gbuffers_basic",
];

/// `gbuffers_hand_water` and its fallbacks (2.3): translucent held items, after deferred.
const HAND_WATER: [&str; 5] = [
    "gbuffers_hand_water",
    "gbuffers_hand",
    "gbuffers_textured_lit",
    "gbuffers_textured",
    "gbuffers_basic",
];

/// `gbuffers_weather` and its fallbacks (2.3).
const WEATHER: [&str; 4] = [
    "gbuffers_weather",
    "gbuffers_textured_lit",
    "gbuffers_textured",
    "gbuffers_basic",
];

/// `gbuffers_entities_translucent` and its fallbacks (2.3).
const ENTITIES_TRANSLUCENT: [&str; 5] = [
    "gbuffers_entities_translucent",
    "gbuffers_entities",
    "gbuffers_textured_lit",
    "gbuffers_textured",
    "gbuffers_basic",
];

/// `gbuffers_beaconbeam` and its fallbacks (the shaders.properties reference): beacon and End
/// gateway beams.
const BEACON_BEAM: [&str; 3] = ["gbuffers_beaconbeam", "gbuffers_textured", "gbuffers_basic"];

/// `gbuffers_particles` and its fallbacks (2.3).
const PARTICLES: [&str; 4] = [
    "gbuffers_particles",
    "gbuffers_textured_lit",
    "gbuffers_textured",
    "gbuffers_basic",
];

/// The sun and moon's default blend: added to the sky, and their own alpha replacing the
/// buffer's, as vanilla draws them (`RenderPipelines.CELESTIAL`, `BlendFunction.OVERLAY`). Iris
/// 1.11.7 left the sun's alpha (2.0) in an RGBA32F buffer the sky had written 1.0 to, with a
/// probe pack of ours (#10490).
pub const CELESTIAL_BLEND: Blend = [Factor::SrcAlpha, Factor::One, Factor::One, Factor::Zero];

const GROUPS: [(Group, &str); 6] = [
    (Group::Setup, "setup"),
    (Group::Begin, "begin"),
    (Group::Prepare, "prepare"),
    (Group::Deferred, "deferred"),
    (Group::Composite, "composite"),
    (Group::Final, "final"),
];

/// The stars' blend: added to the sky, their own alpha replacing the buffer's, as vanilla draws
/// them (`RenderPipelines.STARS`, `BlendFunction.OVERLAY`). Iris 1.11.7 drew them so with a probe
/// pack of ours (#10490): colour 0.25 × 0.5 over the disc's 1.0 read 1.125, alpha 0.5.
pub const STARS_BLEND: Blend = [Factor::SrcAlpha, Factor::One, Factor::One, Factor::Zero];

#[derive(Clone)]
pub struct PostProgram {
    /// The program's name (`composite3`).
    pub name: String,
    pub group: Group,
    /// The vertex and fragment stages; `None` for a program of compute stages only.
    pub program: Option<Program>,
    /// colortex buffer of each fragment output location (location n writes `buffers[n]`).
    pub buffers: Vec<u32>,
    /// `flip.<name>.<buffer>`: buffers forced to flip (true) or kept (false) after this program.
    pub flips: Vec<(u32, bool)>,
    /// How the program's output blends into its buffers; `None` writes them.
    pub blend: Option<Blend>,
    /// `gbuffers_skybasic` without a `blend.*` of its own: each part of the sky blends as
    /// vanilla draws it, the disc and the void without blending (`RenderPipelines.SKY`), the stars
    /// with [`STARS_BLEND`] and the sunset fan with `blend`.
    pub vanilla_sky_blends: bool,
    /// Per-buffer blend overrides, keyed by colortex index (not output location).
    pub buffer_blends: Vec<(u32, Option<Blend>)>,
    /// Bit n: `colortex<n>MipmapEnabled` in its fragment stage, so buffer n's mip chain is
    /// regenerated right before it (6.1.7; not for gbuffers programs).
    pub mipmaps: u16,
    /// Compute programs that run before this one, in order: `<name>.csh`, then `_a` to `_z`.
    pub compute: Vec<(String, ComputeProgram)>,
    /// A geometry stage folded into the vertex stage (`translate_geometry`): drawn non-indexed,
    /// six vertices per quad, with this many instances. 0 without one.
    pub geometry_triangles: u32,
    /// `scale.<name>`: (scale, offset x, offset y), the part of its buffers a program draws, as
    /// fractions of their size from GL's bottom-left corner; [1, 0, 0] (all of them) without.
    pub scale: [f32; 3],
}

/// `size.buffer.<buffer>`: a size of its own, relative to the render size (floats) or in
/// pixels (integers).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum BufferSize {
    Relative([f32; 2]),
    Absolute([u32; 2]),
}

impl BufferSize {
    fn parse(value: &str) -> Option<Self> {
        let words: Vec<&str> = value.split_whitespace().collect();
        let [w, h] = words[..] else {
            return None;
        };
        if w.contains('.') || h.contains('.') {
            let (w, h): (f32, f32) = (w.parse().ok()?, h.parse().ok()?);
            let fine = |f: f32| f.is_finite() && f > 0.0;
            (fine(w) && fine(h)).then_some(Self::Relative([w, h]))
        } else {
            Some(Self::Absolute([w.parse().ok()?, h.parse().ok()?]))
        }
    }

    /// The size for a render size of `render`, at least one pixel and at most
    /// [`BufferSize::MAX`].
    pub fn resolve(self, render: [u32; 2]) -> [u32; 2] {
        let size = match self {
            Self::Relative(s) => [0, 1].map(|k| (render[k] as f32 * s[k]) as u32),
            Self::Absolute(s) => s,
        };
        size.map(|n| n.clamp(1, Self::MAX))
    }

    /// The largest side a buffer may have, the size every device takes for a 2D texture.
    pub const MAX: u32 = 8192;
}

/// `bufferObject.<n>`: an SSBO's size in bytes, or bytes per pixel of the render size scaled.
#[derive(Clone, Debug, PartialEq)]
pub enum BufferObject {
    Bytes(u64),
    PerPixel {
        bytes: u64,
        scale: [f32; 2],
    },
    /// A fixed size with the bytes a file of the pack holds, read on load: the documented
    /// `bufferObject.<n> = <size> <file>` form, the file's path inside `shaders`.
    File {
        bytes: u64,
        path: String,
        data: Vec<u8>,
    },
}

impl BufferObject {
    fn parse(value: &str) -> Option<Self> {
        let words: Vec<&str> = value.split_whitespace().collect();
        let bytes = words.first()?.parse().ok()?;
        match words[..] {
            [_] | [_, "false", ..] => Some(Self::Bytes(bytes)),
            [_, "true", x, y] => Some(Self::PerPixel {
                bytes,
                scale: [x.parse().ok()?, y.parse().ok()?],
            }),
            [_, path] => Some(Self::File {
                bytes,
                path: path.to_owned(),
                data: Vec::new(),
            }),
            _ => None,
        }
    }

    /// The size in bytes for a render size of `render`, at least 16.
    pub fn size(&self, render: [u32; 2]) -> u64 {
        match self {
            Self::Bytes(b) => *b,
            Self::PerPixel { bytes, scale } => {
                let [w, h] = [0, 1].map(|k| (render[k] as f32 * scale[k]) as u64);
                bytes.saturating_mul(w).saturating_mul(h)
            }
            Self::File { bytes, .. } => *bytes,
        }
        .max(16)
    }

    /// The file's path inside `shaders`, when the buffer is initialized from one.
    pub fn file(&self) -> Option<&str> {
        match self {
            Self::File { path, .. } => Some(path),
            _ => None,
        }
    }

    /// The bytes the buffer starts with, when it is initialized from a file.
    pub fn initial_data(&self) -> Option<&[u8]> {
        match self {
            Self::File { data, .. } => Some(data),
            _ => None,
        }
    }
}

/// `image.<name>`: an image of the pack's, read and written as `name`, sampled as `sampler`.
#[derive(Clone, Debug, PartialEq)]
pub struct CustomImage {
    pub name: String,
    pub sampler: Option<String>,
    pub format: crate::Format,
    /// The internal format's GLSL name (`r32i`), for declarations without a format.
    pub qualifier: String,
    /// Relative to the render size (width and height), or in pixels.
    pub size: ImageSize,
    /// Cleared to zero before each frame.
    pub clear: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ImageSize {
    Relative([f32; 2]),
    Absolute([u32; 2]),
    /// Three sizes: a 3D image, whatever its depth.
    Volume([u32; 3]),
    /// One size in pixels: a 1D image (a lookup table by index).
    Line(u32),
}

impl CustomImage {
    fn parse(name: &str, value: &str) -> Option<Self> {
        let w: Vec<&str> = value.split_whitespace().collect();
        let [sampler, _format, internal, _ty, clear, relative, rest @ ..] = &w[..] else {
            return None;
        };
        let size = match (*relative, rest) {
            ("true", [x, y, ..]) => ImageSize::Relative([x.parse().ok()?, y.parse().ok()?]),
            ("false", [x]) => ImageSize::Line(x.parse().ok()?),
            ("false", [x, y]) => ImageSize::Absolute([x.parse().ok()?, y.parse().ok()?]),
            ("false", [x, y, z]) => {
                ImageSize::Volume([x.parse().ok()?, y.parse().ok()?, z.parse().ok()?])
            }
            _ => return None,
        };
        Some(Self {
            name: name.to_owned(),
            sampler: (*sampler != "none").then(|| sampler.to_string()),
            format: crate::Format::parse(internal)?,
            qualifier: internal.to_ascii_lowercase(),
            size,
            clear: *clear == "true",
        })
    }

    /// Width, height and depth for a render size of `render`.
    pub fn extent(&self, render: [u32; 2]) -> [u32; 3] {
        match self.size {
            ImageSize::Relative(s) => {
                let [w, h] = [0, 1].map(|k| ((render[k] as f32 * s[k]) as u32).max(1));
                [w, h, 1]
            }
            ImageSize::Absolute([w, h]) => [w.max(1), h.max(1), 1],
            ImageSize::Volume(s) => s.map(|n| n.max(1)),
            ImageSize::Line(w) => [w.max(1), 1, 1],
        }
    }

    /// A 1D image (`image1D`), not a 2D one.
    pub fn is_1d(&self) -> bool {
        matches!(self.size, ImageSize::Line(_))
    }

    /// A 3D image (`image3D`), not a 2D one.
    pub fn is_3d(&self) -> bool {
        matches!(self.size, ImageSize::Volume(_))
    }
}

/// `sky`, `stars`, `sun` and `moon` (3.2): which parts of the vanilla sky draw, whether the
/// pack's `gbuffers_skybasic` and `gbuffers_skytextured` or the game draws them. Each is on unless
/// its key is `false`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SkySwitches {
    /// The sky disc, the sunset fan and the void disc (`sky`).
    pub sky: bool,
    pub stars: bool,
    pub sun: bool,
    pub moon: bool,
}

impl Default for SkySwitches {
    fn default() -> Self {
        Self {
            sky: true,
            stars: true,
            sun: true,
            moon: true,
        }
    }
}

impl SkySwitches {
    fn parse(properties: &Properties) -> Self {
        Self {
            sky: on_unless_false(properties, "sky"),
            stars: on_unless_false(properties, "stars"),
            sun: on_unless_false(properties, "sun"),
            moon: on_unless_false(properties, "moon"),
        }
    }
}

/// A boolean key that is on by default: only `false` turns it off.
fn on_unless_false(properties: &Properties, key: &str) -> bool {
    properties.get(key).is_none_or(|v| v.trim() != "false")
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ShadowCulling {
    On,
    Off,
    Reversed,
}

#[derive(Clone)]
pub struct Post {
    /// The folder the programs came from: `""` for `shaders` itself, else `world0`...
    pub folder: String,
    pub programs: Vec<PostProgram>,
    /// The flips of the virtual `deferred_pre` and `composite_pre` (6.1.6).
    pub deferred_pre: Vec<(u32, bool)>,
    pub composite_pre: Vec<(u32, bool)>,
    /// What draws solid and cutout terrain, when the game asked for it and one loads; when
    /// terrain draws in two parts, solid terrain only.
    pub terrain: Option<PostProgram>,
    /// What draws cutout terrain when the game draws terrain in two parts
    /// (`GameInputs::terrain_parts`); `terrain` then draws the solid part. A pack with
    /// `gbuffers_terrain_solid` or `gbuffers_terrain_cutout` and no program for one part has
    /// only the other.
    pub terrain_cutout: Option<PostProgram>,
    /// What draws translucent terrain (water, stained glass) after deferred.
    pub water: Option<PostProgram>,
    /// The pack has no program for solid and cutout terrain (none of `gbuffers_terrain`'s
    /// chain, nor `_solid` or `_cutout`), or for translucent terrain (none of
    /// `gbuffers_water`'s): nothing draws that terrain, into no buffer nor depth. Iris 1.11.7
    /// drew no terrain with our probe packs having no gbuffers programs, at the root or in a
    /// dimension's folder (#10455). A program that is there but doesn't load still leaves its
    /// terrain to the game.
    pub terrain_undrawn: bool,
    pub water_undrawn: bool,
    /// What draws the sky (disc, sunset, stars, void) before terrain.
    pub sky: Option<PostProgram>,
    /// What draws the sun and moon, between the sunset and the stars.
    pub sky_textured: Option<PostProgram>,
    /// What draws entities, after terrain.
    pub entities: Option<PostProgram>,
    /// What draws block entities, after terrain.
    pub block: Option<PostProgram>,
    /// An authored translucent block variant; absent or failed keeps the ordinary block path.
    pub block_translucent: Option<PostProgram>,
    /// What draws translucent entities (name tags, shadows), after the others.
    pub entities_translucent: Option<PostProgram>,
    /// What draws particles.
    pub particles: Option<PostProgram>,
    /// The translucent particle variant for explicit mixed ordering, when it loads.
    pub particles_translucent: Option<PostProgram>,
    /// What draws beacon and End gateway beams (after deferred, blended).
    pub beacon_beam: Option<PostProgram>,
    pub damaged_block: Option<PostProgram>,
    /// What draws the game's clouds.
    pub clouds: Option<PostProgram>,
    /// What draws lines (the block outline), with the pack's own transform and look.
    pub lines: Option<PostProgram>,
    /// What draws rain and snow, after deferred. They never write depth, whatever `rain.depth`
    /// says: Iris 1.11.7 left none of either in `depthtex0` (probe packs of ours, #10492).
    pub weather: Option<PostProgram>,
    /// What draws the first-person hand and held items.
    pub hand: Option<PostProgram>,
    /// What draws their translucent parts (stained glass, ice...), after deferred and blended.
    pub hand_water: Option<PostProgram>,
    /// `clouds=off`: no clouds at all, neither the game's nor the pack's program for them.
    pub clouds_off: bool,
    /// `sky`, `stars`, `sun` and `moon`: the parts of the vanilla sky left out.
    pub sky_switches: SkySwitches,
    /// Some program reads a shadow buffer, or `shadow.enabled=true`: the shadow pass runs (2.5).
    pub shadow_pass: bool,
    /// When it runs: what draws solid and cutout terrain from the light (`shadow`), and
    /// translucent terrain (`shadow_water` or `shadow`), into the shadow buffers (2.4).
    pub shadow: Option<PostProgram>,
    pub shadow_water: Option<PostProgram>,
    /// `shadow_water` without the boats' water patches (`GbuffersInputs::water_patches`), for a
    /// GPU that refuses it with them: their vertex output is one more than a pack writing as
    /// many as the GPU takes has room for. The water's light then shows in the boats, as before
    /// the patches, and the rest of the shadow pass stays.
    pub shadow_water_plain: Option<PostProgram>,
    /// What draws entities (`shadow_entities` or `shadow`) and block entities (`shadow_block` or
    /// `shadow`) from the light, unless `shadowEntities`/`shadowBlockEntities` is false.
    pub shadow_entities: Option<PostProgram>,
    pub shadow_block: Option<PostProgram>,
    /// `shadowEntities=false` with `shadowPlayer=true`: of the entities, only the player casts.
    pub shadow_only_player: bool,
    /// `shadowBlockEntities=false` with `shadowLightBlockEntities=true`: only block entities
    /// whose block emits light cast.
    pub shadow_only_lit_blocks: bool,
    /// `shadowcomp`..`shadowcomp99`, after the shadow pass (2.1).
    pub shadow_comp: Vec<PostProgram>,
    /// Translucent entities draw after deferred (`separateEntityDraws`, 3.2).
    pub translucent_late: bool,
    /// Particles draw after deferred (`particles.ordering`, `particles.before.deferred`, 3.2).
    pub particles_late: bool,
    /// The custom uniforms of `shaders.properties` (3.4).
    pub custom: Custom,
    /// `texture.*`: the textures the pack binds to sampler names.
    pub textures: Vec<crate::CustomTexture>,
    /// `size.buffer.*`, by buffer.
    pub sizes: [Option<BufferSize>; crate::BUFFERS],
    /// The shadowcolor buffers the shadow programs have: 2, or all 8 when the pack names the
    /// `HIGHER_SHADOWCOLOR` feature (`iris.features.required` or `.optional`).
    pub shadow_colors: usize,
    /// `bufferObject.*`, by binding.
    pub buffer_objects: std::collections::BTreeMap<u32, BufferObject>,
    /// `image.*`.
    pub images: Vec<CustomImage>,
    /// `shadow.culling`: `false` draws everything within the shadow distance, every face, not
    /// only what casts a shadow into view; `reversed` within `voxelDistance` of the player.
    pub shadow_culling: ShadowCulling,
    pub config: Config,
    /// Programs skipped and why, as `program: message` or `file:line: message`.
    pub notes: Vec<String>,
}

impl Post {
    /// Every program that loaded: the passes', what draws the game's geometry, the shadow
    /// pass's and its composites.
    fn every_program(&self) -> impl Iterator<Item = &PostProgram> {
        let draws = [
            &self.terrain,
            &self.terrain_cutout,
            &self.water,
            &self.sky,
            &self.sky_textured,
            &self.entities,
            &self.block,
            &self.block_translucent,
            &self.entities_translucent,
            &self.particles,
            &self.particles_translucent,
            &self.damaged_block,
            &self.clouds,
            &self.weather,
            &self.hand,
            &self.shadow,
            &self.shadow_water,
            &self.shadow_entities,
            &self.shadow_block,
        ];
        self.programs
            .iter()
            .chain(draws.into_iter().flatten())
            .chain(&self.shadow_comp)
    }

    /// The fewest bytes each storage buffer (`bufferObject.<n>`, set 4) needs for every stage
    /// that binds it: its block's size as the stage lays it out (std430). OpenGL binds a smaller buffer and leaves what lies
    /// past its end undefined; wgpu refuses to draw or dispatch with one.
    pub fn buffer_minimums(&self) -> std::collections::BTreeMap<u32, u64> {
        let mut sizes = std::collections::BTreeMap::new();
        for p in self.every_program() {
            let stages = p.program.iter().flat_map(|s| [&s.vertex, &s.fragment]);
            for module in stages.chain(p.compute.iter().map(|(_, c)| &c.module)) {
                for (_, var) in module.global_variables.iter() {
                    if let (naga::AddressSpace::Storage { .. }, Some(b)) = (var.space, &var.binding)
                        && b.group == 4
                    {
                        let size = u64::from(module.types[var.ty].inner.size(module.to_ctx()));
                        let least = sizes.entry(b.binding).or_insert(0);
                        *least = size.max(*least);
                    }
                }
            }
        }
        sizes
    }
}

/// The folder for one of Minecraft's legacy dimension numbers. Named mappings use the
/// corresponding Minecraft identifier; callers with an identifier should use
/// [`dimension_folder_named`] instead.
pub fn dimension_folder(pack: &Pack, id: i32) -> String {
    let name = match id {
        -1 => "minecraft:the_nether",
        1 => "minecraft:the_end",
        0 => "minecraft:overworld",
        _ => "",
    };
    dimension_folder_named(pack, name, id)
}

/// Selects a dimension's program folder. `dimension.properties` mappings take precedence
/// over legacy folders: explicit identifiers beat `*`, and an unmapped identifier uses the
/// base folder. Without that file, preserve the `world<legacy_id>` selection.
pub fn dimension_folder_named(pack: &Pack, id: &str, legacy_id: i32) -> String {
    if let Some(text) = pack.read_text("dimension.properties") {
        let mappings = Properties::parse(&text, &Macros::default());
        let mut fallback = "";
        for entry in &mappings.entries {
            let Some(folder) = entry.key.strip_prefix("dimension.") else {
                continue;
            };
            // A folder must stay inside shaders/, including on folder-backed packs.
            let Some(path) = crate::pack::normalize(folder).filter(|p| !p.is_empty()) else {
                continue;
            };
            for name in entry.value.split_whitespace() {
                if name == "*" {
                    fallback = folder;
                } else if name == id
                    || (!name.contains(':') && id.strip_prefix("minecraft:") == Some(name))
                {
                    return path;
                }
            }
        }
        return crate::pack::normalize(fallback).unwrap_or_default();
    }
    let folder = format!("world{legacy_id}");
    if !pack.list(&folder).is_empty() {
        folder
    } else {
        String::new()
    }
}

/// The game's geometry a pack may draw with its gbuffers programs: each kind's vertex inputs,
/// when the game can draw it with a pack.
#[derive(Clone, Debug, Default)]
pub struct GameInputs {
    pub terrain: Option<GbuffersInputs>,
    pub sky: Option<GbuffersInputs>,
    pub celestial: Option<GbuffersInputs>,
    /// Entities and block entities (one vertex format).
    pub entities: Option<GbuffersInputs>,
    /// The clouds' mesh.
    pub clouds: Option<GbuffersInputs>,
    /// Rain and snow.
    pub weather: Option<GbuffersInputs>,
    /// Lines: the block outline.
    pub lines: Option<GbuffersInputs>,
    /// The first-person hand and held items.
    pub hand: Option<GbuffersInputs>,
    /// `terrain`, drawing only its solid quads, then only its cutout ones: given, a pack with
    /// `gbuffers_terrain_solid` or `gbuffers_terrain_cutout` draws terrain in these two parts.
    pub terrain_parts: Option<[GbuffersInputs; 2]>,
    /// `terrain` with both parts in one draw (`GbuffersInputs::parts`): given, a pack drawing
    /// both with one program draws them in one pass.
    pub terrain_one_pass: Option<GbuffersInputs>,
    /// Terrain read vertex by vertex, for a shadow program with a geometry stage.
    pub terrain_geometry: Option<crate::GeometryInputs>,
    /// Entities and block entities read vertex by vertex, for the same.
    pub entity_geometry: Option<crate::GeometryInputs>,
}

/// Loads the post-processing programs of `folder` and, for the geometry `inputs` describes, the
/// gbuffers programs it draws with.
/// The buffer object with its file read, when it has one: the documented
/// `bufferObject.<n> = <size> <file>` form reads the file's bytes from the pack. A file that
/// is not there, or that is bigger than the buffer it fills, is refused with a note.
fn buffer_object_file(pack: &Pack, object: BufferObject) -> Result<BufferObject, String> {
    let BufferObject::File { bytes, path, .. } = &object else {
        return Ok(object);
    };
    let Some(data) = pack.read(path) else {
        return Err(format!("`{path}`: no such file in the pack"));
    };
    // The size the pack asks for, not the size the buffer is made (which is at least 16).
    if data.len() as u64 > *bytes {
        return Err(format!("`{path}`: bigger than the buffer it fills"));
    }
    Ok(BufferObject::File {
        bytes: *bytes,
        path: path.clone(),
        data,
    })
}

pub fn load_post(
    pack: &Pack,
    folder: &str,
    properties: &Properties,
    macros: &Macros,
    inputs: &GameInputs,
) -> Post {
    let mut post = Post {
        folder: folder.to_owned(),
        programs: Vec::new(),
        deferred_pre: flips(properties, "deferred_pre"),
        composite_pre: flips(properties, "composite_pre"),
        terrain: None,
        terrain_cutout: None,
        water: None,
        terrain_undrawn: false,
        water_undrawn: false,
        sky: None,
        sky_textured: None,
        entities: None,
        block: None,
        block_translucent: None,
        entities_translucent: None,
        particles: None,
        particles_translucent: None,
        beacon_beam: None,
        damaged_block: None,
        clouds: None,
        lines: None,
        weather: None,
        hand: None,
        hand_water: None,
        clouds_off: false,
        sky_switches: SkySwitches::parse(properties),
        shadow_pass: false,
        shadow: None,
        shadow_water: None,
        shadow_water_plain: None,
        shadow_entities: None,
        shadow_block: None,
        shadow_only_player: false,
        shadow_only_lit_blocks: false,
        shadow_comp: Vec::new(),
        translucent_late: false,
        particles_late: false,
        custom: Custom::parse(properties),
        textures: Vec::new(),
        sizes: [None; crate::BUFFERS],
        shadow_colors: if higher_shadowcolor(properties) {
            crate::SHADOW_COLORS
        } else {
            2
        },
        buffer_objects: Default::default(),
        images: Vec::new(),
        shadow_culling: match properties.get("shadow.culling").map(str::trim) {
            Some("false") => ShadowCulling::Off,
            Some("reversed") => ShadowCulling::Reversed,
            _ => ShadowCulling::On,
        },
        config: Config::default(),
        notes: Vec::new(),
    };
    let options = crate::with_options(pack, macros);
    for e in &properties.entries {
        if let Some(n) = e.key.strip_prefix("bufferObject.") {
            match (n.parse(), BufferObject::parse(&e.value)) {
                (Ok(n), Some(b)) => match buffer_object_file(pack, b) {
                    Ok(b) => _ = post.buffer_objects.insert(n, b),
                    Err(note) => post.notes.push(note),
                },
                _ => post.notes.push(format!("`{}`: not a buffer object", e.key)),
            }
            continue;
        }
        if let Some(name) = e.key.strip_prefix("image.") {
            match CustomImage::parse(name, &options.expand(&e.value)) {
                Some(i) => post.images.push(i),
                None => post.notes.push(format!("`{}`: not an image", e.key)),
            }
            continue;
        }
        let Some(name) = e.key.strip_prefix("size.buffer.") else {
            continue;
        };
        match (crate::buffer_index(name), BufferSize::parse(&e.value)) {
            (Some(b), Some(size)) => post.sizes[b] = Some(size),
            _ => post
                .notes
                .push(format!("`{}`: not a buffer and a size", e.key)),
        }
    }
    let (textures, notes) = crate::load_textures(pack, properties);
    post.textures = textures;
    post.notes.extend(notes);
    let mut loader = Loader {
        pack,
        folder,
        properties,
        macros,
        switches: crate::switches_on(pack),
        options,
        geometry: None,
        post: &mut post,
    };
    for (group, base) in GROUPS {
        let names: Vec<String> = if group == Group::Final {
            vec![base.to_owned()]
        } else {
            std::iter::once(base.to_owned())
                .chain((1..=99).map(|n| format!("{base}{n}")))
                .collect()
        };
        let target = match group {
            Group::Final => Target::Window,
            _ => Target::Buffers,
        };
        for name in names {
            if let Some(mut p) = loader.load(name, group, |v, f| translate_composite(v, f, target))
            {
                // `blend.<program>` blends every buffer the program writes. Iris 1.11.7, probed:
                // it applies to begin, prepare, deferred and composite programs, and
                // `blend.<program>.<buffer>` doesn't apply to deferred and composite ones; the
                // other cases are assumed to match.
                p.blend = blend(loader.properties, &p.name).flatten();
                p.buffer_blends.clear();
                loader.post.programs.push(p);
            }
        }
    }
    let split_terrain = ["gbuffers_terrain_solid", "gbuffers_terrain_cutout"]
        .iter()
        .any(|n| loader.has(n));
    // Whether the pack has a program for each part, loaded or not.
    let [has_solid, has_cutout] =
        [&TERRAIN_SOLID, &TERRAIN_CUTOUT].map(|chain| chain.iter().any(|n| loader.has(n)));
    let mut gbuffers = |names: &[&str], group, inputs: &GbuffersInputs| {
        names.iter().find_map(|name| {
            // `alphaTest.<program>` replaces the geometry's default test (OQ-23).
            let key = properties.get(&format!("alphaTest.{name}"));
            let test = key.and_then(|v| crate::AlphaTest::parse(v.trim()));
            let mut inputs = inputs.clone();
            if let Some(test) = test {
                inputs.alpha_test = test;
                // One program's solid and cutout parts in one pass: both, as in two.
                if let Some(parts) = &mut inputs.parts {
                    parts.cutout_alpha_test = test;
                }
            }
            loader.load(name.to_string(), group, |v, f| {
                translate_gbuffers(v, f, Target::Buffers, &inputs)
            })
        })
    };
    // The sky's parts blend as vanilla draws them (the sunset fan translucent, the stars
    // [`STARS_BLEND`], the disc and the void not at all), unless `blend.*` says otherwise. Iris 1.11.7 left exactly what
    // `gbuffers_skybasic` wrote on the sky disc, alpha 0.25 included, with a probe pack of ours
    // (#10490).
    let sky = inputs
        .sky
        .as_ref()
        .and_then(|i| gbuffers(&SKY, Group::Gbuffers, i));
    let sky = sky.map(|mut p| {
        let own = blend(properties, &p.name);
        p.vanilla_sky_blends = own.is_none();
        p.blend = own.unwrap_or(Some(TRANSLUCENT_BLEND));
        p
    });
    let celestial = inputs.celestial.as_ref();
    let sky_textured = celestial.and_then(|i| gbuffers(&SKY_TEXTURED, Group::Gbuffers, i));
    let sky_textured = sky_textured.map(|mut p| {
        p.blend = blend(properties, &p.name).unwrap_or(Some(CELESTIAL_BLEND));
        p
    });
    // Entities and block entities draw without blending, as terrain does and as vanilla's
    // cutout entity types do, unless `blend.*` says otherwise (OQ-23).
    let entity_blend = |p: Option<PostProgram>| {
        p.map(|mut p| {
            p.blend = blend(properties, &p.name).flatten();
            p
        })
    };
    let (entities, block) = match &inputs.entities {
        Some(i) => (
            entity_blend(gbuffers(&ENTITIES, Group::Gbuffers, i)),
            entity_blend(gbuffers(&BLOCK, Group::Gbuffers, i)),
        ),
        None => (None, None),
    };
    let damaged_block = inputs
        .entities
        .as_ref()
        .and_then(|i| gbuffers(&["gbuffers_damagedblock"], Group::Gbuffers, i))
        .map(|mut p| {
            p.blend = blend(properties, &p.name).unwrap_or(Some([
                Factor::DstColor,
                Factor::SrcColor,
                Factor::One,
                Factor::Zero,
            ]));
            p
        });
    // Translucent entities and particles blend (OQ-23). Particles keep vanilla's alpha cut at
    // 0.1; translucent entities lose only fully transparent texels (a shadow fades out).
    let translucent =
        |names: &[&str],
         cut: f32,
         gbuffers: &mut dyn FnMut(&[&str], &GbuffersInputs) -> Option<PostProgram>| {
            let i = inputs.entities.as_ref()?;
            let mut i = i.clone();
            i.alpha_test = Some(crate::AlphaTest {
                op: ">",
                reference: cut,
            });
            gbuffers(names, &i).map(|mut p| {
                p.blend = blend(properties, &p.name).unwrap_or(Some(TRANSLUCENT_BLEND));
                p
            })
        };
    let mut gbuffers_dyn = |names: &[&str], i: &GbuffersInputs| gbuffers(names, Group::Gbuffers, i);
    let entities_translucent = translucent(&ENTITIES_TRANSLUCENT, 0.0, &mut gbuffers_dyn);
    let particles = translucent(&PARTICLES, 0.1, &mut gbuffers_dyn);
    // Optional slots deliberately have no fallback chain: an absent, disabled or rejected
    // variant must leave the legacy material grouping and state intact.
    let block_translucent = translucent(&["gbuffers_block_translucent"], 0.0, &mut gbuffers_dyn);
    let mixed = properties.get("particles.ordering").map(str::trim) == Some("mixed");
    let particles_translucent = mixed
        .then(|| translucent(&["gbuffers_particles_translucent"], 0.1, &mut gbuffers_dyn))
        .flatten();
    let beacon_beam = translucent(&BEACON_BEAM, 0.0, &mut gbuffers_dyn);
    // Clouds blend, as vanilla's do (OQ-23).
    let clouds = inputs
        .clouds
        .as_ref()
        .and_then(|i| gbuffers_dyn(&CLOUDS, i));
    let clouds = clouds.map(|mut p| {
        p.blend = blend(properties, &p.name).unwrap_or(Some(TRANSLUCENT_BLEND));
        p
    });
    // The block outline blends, as vanilla's does.
    let lines = inputs
        .lines
        .as_ref()
        .and_then(|i| gbuffers_dyn(&LINES, i))
        .map(|mut p| {
            p.blend = blend(properties, &p.name).unwrap_or(Some(TRANSLUCENT_BLEND));
            p
        });
    // Rain and snow draw after deferred and blend, as vanilla's do (OQ-23).
    let weather = inputs
        .weather
        .as_ref()
        .and_then(|i| gbuffers(&WEATHER, Group::Translucent, i));
    let weather = weather.map(|mut p| {
        p.blend = blend(properties, &p.name).unwrap_or(Some(TRANSLUCENT_BLEND));
        p
    });
    // The hand draws as entities do: no blending unless `blend.*` says otherwise (OQ-23).
    let hand = inputs
        .hand
        .as_ref()
        .and_then(|i| gbuffers(&HAND, Group::Gbuffers, i))
        .map(|mut p| {
            p.blend = blend(properties, &p.name).flatten();
            p
        });
    // Translucent held items draw after deferred as translucent terrain does, and blend.
    let hand_water = inputs
        .hand
        .as_ref()
        .and_then(|i| gbuffers(&HAND_WATER, Group::Translucent, i))
        .map(|mut p| {
            p.blend = blend(properties, &p.name).unwrap_or(Some(TRANSLUCENT_BLEND));
            p
        });
    if let Some(terrain) = &inputs.terrain {
        // Cutout terrain's alpha test, unless `alphaTest.*` says otherwise (3.2): alpha above
        // 0.5, which `alphaTestRef` reads. Iris 1.11.7 with probe packs of ours (#9307), through
        // gbuffers_terrain alone or gbuffers_terrain_cutout: cutout fragments written with alpha
        // 0.5 and below were discarded, 0.51 and above kept, and alphaTestRef read 0.5. Vanilla's
        // cutout terrain cuts at 0.5 too (`RenderPipelines.CUTOUT_TERRAIN`'s `ALPHA_CUTOUT`).
        let cut = Some(crate::AlphaTest {
            op: ">",
            reference: 0.5,
        });
        let cutout = |inputs: &GbuffersInputs| {
            let mut cutout = inputs.clone();
            cutout.alpha_test = cutout.alpha_test.or(cut);
            cutout
        };
        // A pack with Iris's gbuffers_terrain_solid or gbuffers_terrain_cutout draws solid and
        // cutout terrain apart, each with its own program, falling back to gbuffers_terrain's
        // chain (Iris 1.11.7 with probe packs having both, or one of them); any other pack draws
        // both parts with gbuffers_terrain's chain, each quad with its part's alpha test (#9307),
        // in one pass: two passes would run every vertex of solid terrain twice. Solid
        // terrain gets no default alpha test, and alphaTestRef 0: in those probes Iris kept
        // solid fragments written with alpha 0.05, through either program, and also 0.0
        // through gbuffers_terrain alone or gbuffers_terrain_solid (#9307).
        let parts = inputs.terrain_parts.as_ref().filter(|_| split_terrain);
        let mut lone = None;
        let (solid, terrain_cutout) = match parts {
            Some([solid, cut]) => {
                let solid = gbuffers(&TERRAIN_SOLID, Group::Gbuffers, solid);
                let cut = gbuffers(&TERRAIN_CUTOUT, Group::Gbuffers, &cutout(cut));
                // One part loaded: its name, the other part, and whether the pack has a
                // program for that one.
                let one = match (&solid, &cut) {
                    (Some(p), None) => Some((p.name.clone(), "cutout", has_cutout)),
                    (None, Some(p)) => Some((p.name.clone(), "solid", has_solid)),
                    _ => None,
                };
                match one {
                    // No program in the other part's chain: that part draws nothing, and this
                    // one draws alone. Iris 1.11.7 left the other part's terrain undrawn with
                    // our probes having only gbuffers_terrain_solid, or only _cutout.
                    Some((name, other, false)) => {
                        lone = Some(format!(
                            "{name}: no program for {other} terrain; nothing draws it"
                        ));
                        (solid, cut)
                    }
                    // The other part's program didn't load: this one doesn't draw alone, which
                    // would leave that part's terrain undrawn. The game draws all terrain, as
                    // when an unsplit pack's terrain program doesn't load.
                    Some((name, other, true)) => {
                        lone = Some(format!(
                            "{name}: {other} terrain's program didn't load; the game draws all terrain"
                        ));
                        (None, None)
                    }
                    None => (solid, cut),
                }
            }
            None => match &inputs.terrain_one_pass {
                Some(both) => {
                    let mut both = both.clone();
                    if let Some(parts) = &mut both.parts {
                        parts.cutout_alpha_test = parts.cutout_alpha_test.or(cut);
                    }
                    (gbuffers(&TERRAIN, Group::Gbuffers, &both), None)
                }
                // A game that draws terrain in one part: all of it with the cutout test.
                None => (gbuffers(&TERRAIN, Group::Gbuffers, &cutout(terrain)), None),
            },
        };
        // gbuffers_water falls back to gbuffers_terrain's chain (2.3).
        // Water discards what lies behind the boats' water patches (`GbuffersInputs::water_mask`).
        let water = gbuffers(
            &WATER,
            Group::Translucent,
            &GbuffersInputs {
                water_mask: true,
                ..terrain.clone()
            },
        );
        loader.post.notes.extend(lone);
        loader.post.terrain_undrawn = !has_solid && !has_cutout;
        loader.post.water_undrawn = !WATER.iter().any(|n| loader.has(n));
        for (undrawn, part) in [
            (loader.post.terrain_undrawn, "terrain"),
            (loader.post.water_undrawn, "translucent terrain"),
        ] {
            if undrawn {
                loader
                    .post
                    .notes
                    .push(format!("no program for {part}; nothing draws it"));
            }
        }
        loader.post.terrain = solid;
        loader.post.water = water.map(|mut p| {
            p.blend = blend(properties, &p.name).unwrap_or(Some(TRANSLUCENT_BLEND));
            p
        });
        loader.post.terrain_cutout = terrain_cutout;
        let programs = [&mut loader.post.terrain, &mut loader.post.terrain_cutout];
        for p in programs.into_iter().flatten() {
            p.blend = blend(properties, &p.name).flatten();
        }
    }
    loader.post.sky = sky;
    loader.post.sky_textured = sky_textured;
    loader.post.entities = entities;
    loader.post.block = block;
    loader.post.block_translucent = block_translucent;
    loader.post.entities_translucent = entities_translucent;
    loader.post.particles = particles;
    loader.post.particles_translucent = particles_translucent;
    loader.post.beacon_beam = beacon_beam;
    loader.post.damaged_block = damaged_block;
    loader.post.clouds_off = properties.get("clouds").is_some_and(|v| v.trim() == "off");
    loader.post.clouds = clouds.filter(|_| !loader.post.clouds_off);
    loader.post.lines = lines;
    loader.post.weather = weather;
    loader.post.hand = hand;
    loader.post.hand_water = hand_water;
    let flag = |key: &str| properties.get(key).map(|v| v.trim() == "true");
    loader.post.translucent_late = flag("separateEntityDraws").unwrap_or(false);
    // Keep the legacy timing here. The renderer moves ordinary mixed particles earlier
    // only after it has successfully built their translucent variant.
    loader.post.particles_late = match properties.get("particles.ordering").map(str::trim) {
        Some("before") => false,
        Some(_) => true,
        None => !flag("particles.before.deferred").unwrap_or(false),
    };
    load_shadow(
        &mut loader,
        inputs.terrain.as_ref(),
        inputs.entities.as_ref(),
        [&inputs.terrain_geometry, &inputs.entity_geometry].map(Option::as_ref),
    );
    post
}

/// Whether the shadow pass runs (2.5) and, when it does, its programs: `shadow_solid` and
/// `shadow_cutout` are not used (2.4, Legacy); the game's solid and cutout terrain is one draw.
/// `shadow_entities` and `shadow_block` read the entities' vertices (`entities`). A program
/// with a geometry stage reads its vertices itself: `geometry` holds how, for terrain and for
/// entities.
fn load_shadow(
    loader: &mut Loader<'_>,
    terrain: Option<&GbuffersInputs>,
    entities: Option<&GbuffersInputs>,
    geometry: [Option<&crate::GeometryInputs>; 2],
) {
    let post = &*loader.post;
    let gbuffers = [
        &post.terrain,
        &post.terrain_cutout,
        &post.water,
        &post.sky,
        &post.sky_textured,
        &post.entities,
        &post.block,
        &post.block_translucent,
        &post.entities_translucent,
        &post.particles,
        &post.particles_translucent,
        &post.beacon_beam,
        &post.damaged_block,
        &post.clouds,
        &post.weather,
        &post.hand,
        &post.hand_water,
    ];
    // A program's compute stages count as its other stages do: Iris 1.11.7 drew the shadow map
    // for a probe pack of ours whose only reader of it was `prepare.csh` (#10490).
    let reads = (post.programs.iter())
        .chain(gbuffers.into_iter().flatten())
        .flat_map(|p| {
            let stages = p.program.iter().map(|q| &q.interface);
            stages.chain(p.compute.iter().map(|(_, c)| &c.interface))
        })
        .flat_map(|i| &i.samplers)
        .any(|s| {
            SHADOW_SAMPLERS.contains(&s.name.as_str())
                || config::shadow_color(&s.name).is_some_and(|n| n < loader.post.shadow_colors)
        });
    let forced = match loader.properties.get("shadow.enabled").map(str::trim) {
        Some("true") => Some(true),
        Some("false") => Some(false),
        _ => None,
    };
    let on = forced.unwrap_or(reads);
    loader.post.shadow_pass = on;
    if !on {
        // Observed in probe packs of ours (p9063a, p9063c), not a rule Iris states: Iris 1.11.7
        // honoured a `colortexNFormat` declared only in `shadow.fsh` or `shadowcomp.fsh` of a
        // pack Sunrise runs without a shadow pass. So the colortex formats of the shadow
        // programs' fragment shaders are read here, with `shadow.fsh`'s shadow settings
        // (`shadowDistance` and `sunPathRotation` probed, #10490), and nothing else of them
        // (nothing else was probed). The other shadow programs' files were not probed; they are
        // read as `shadow`, formats only.
        let names = ["shadow", "shadow_water", "shadow_entities", "shadow_block"]
            .map(str::to_owned)
            .into_iter()
            .chain(std::iter::once("shadowcomp".to_owned()))
            .chain((1..=99).map(|n| format!("shadowcomp{n}")));
        for name in names {
            // `shadow.fsh`'s shadow settings count too (`Config::read_formats_and_shadow`).
            let shadow = name == "shadow";
            loader.read_formats(&name, shadow);
        }
        return;
    }
    let names =
        std::iter::once("shadowcomp".to_owned()).chain((1..=99).map(|n| format!("shadowcomp{n}")));
    for name in names {
        let program = loader.load(name, Group::ShadowComp, |v, f| {
            translate_composite(v, f, Target::Buffers)
        });
        loader.post.shadow_comp.extend(program);
    }
    let properties = loader.properties;
    let mut load =
        |names: &[&str], inputs: Option<&GbuffersInputs>, terrain: bool, cutout: bool| {
            let inputs = inputs?;
            names.iter().find_map(|name| {
                // Cutout geometry (leaves) casts the shadow of its texels only: the cutout test,
                // unless `alphaTest.<program>` says otherwise, as for the gbuffers programs.
                // Translucent terrain keeps no test, as gbuffers_water does.
                let test = match properties.get(&format!("alphaTest.{name}")) {
                    Some(v) => crate::AlphaTest::parse(v.trim()).flatten(),
                    None if !cutout => inputs.alpha_test,
                    None => inputs.alpha_test.or(Some(crate::AlphaTest {
                        op: ">",
                        reference: 0.1,
                    })),
                };
                let mut inputs = inputs.clone();
                inputs.alpha_test = test;
                let geometry = if terrain { geometry[0] } else { geometry[1] };
                loader.geometry = geometry.cloned().map(|mut g| {
                    g.inputs.alpha_test = test;
                    g
                });
                let p = loader.load(name.to_string(), Group::Shadow, |v, f| {
                    translate_shadow(v, f, &inputs)
                });
                p.map(|mut p| {
                    p.blend = blend(properties, &p.name).flatten();
                    p
                })
            })
        };
    // `shadowTerrain`, `shadowTranslucent`, `shadowEntities` and `shadowBlockEntities` (3.2)
    // leave their part out of the shadow pass; all are on by default (OQ-19). `shadowPlayer`
    // and `shadowLightBlockEntities` (E, off by default) keep the player, and block entities
    // that emit light, when the first two are off.
    let keep = |key: &str| on_unless_false(properties, key);
    let set = |key: &str| properties.get(key).is_some_and(|v| v.trim() == "true");
    let only_player = !keep("shadowEntities") && set("shadowPlayer");
    let only_lit = !keep("shadowBlockEntities") && set("shadowLightBlockEntities");
    let shadow = keep("shadowTerrain")
        .then(|| load(&["shadow"], terrain, true, true))
        .flatten();
    // Its water keeps out of the boats' water patches (`GbuffersInputs::water_patches`).
    let water_inputs = terrain.map(|t| GbuffersInputs {
        water_patches: true,
        ..t.clone()
    });
    let water = keep("shadowTranslucent")
        .then(|| {
            load(
                &["shadow_water", "shadow"],
                water_inputs.as_ref(),
                true,
                false,
            )
        })
        .flatten();
    let shadow_entities = (keep("shadowEntities") || only_player)
        .then(|| load(&["shadow_entities", "shadow"], entities, false, true))
        .flatten();
    let shadow_block = (keep("shadowBlockEntities") || only_lit)
        .then(|| load(&["shadow_block", "shadow"], entities, false, true))
        .flatten();
    let water_plain = water
        .as_ref()
        .and_then(|_| load(&["shadow_water", "shadow"], terrain, true, false));
    // The plain one's notes are the patched one's again: each once.
    let mut seen = std::collections::HashSet::new();
    loader.post.notes.retain(|n| seen.insert(n.clone()));
    if shadow.is_none() && terrain.is_some() {
        loader
            .post
            .notes
            .push("shadow: no program; the shadow map stays clear".to_owned());
    }
    loader.geometry = None;
    loader.post.shadow = shadow;
    loader.post.shadow_water = water;
    loader.post.shadow_water_plain = water_plain;
    loader.post.shadow_entities = shadow_entities;
    loader.post.shadow_block = shadow_block;
    loader.post.shadow_only_player = only_player;
    loader.post.shadow_only_lit_blocks = only_lit;
}

struct Loader<'a> {
    pack: &'a Pack,
    folder: &'a str,
    properties: &'a Properties,
    macros: &'a Macros,
    switches: BTreeSet<String>,
    /// `macros` with the pack's options, for the program switches.
    options: Macros,
    /// The game's geometry read vertex by vertex, while loading shadow programs.
    geometry: Option<crate::GeometryInputs>,
    post: &'a mut Post,
}

impl Loader<'_> {
    /// The pack has both stages of the program `name` in this folder, and doesn't turn it off
    /// (`program.<name>.enabled`), as `load` reads them.
    fn has(&self, name: &str) -> bool {
        ["vsh", "fsh"].iter().all(|ext| match self.folder {
            "" => self.pack.exists(&format!("{name}.{ext}")),
            f => self.pack.exists(&format!("{f}/{name}.{ext}")),
        }) && enabled(
            self.properties,
            self.folder,
            name,
            &self.options,
            &self.switches,
        )
    }

    /// The program `name` translated by `translate`, or `None` when it is missing, disabled or
    /// fails (with a note).
    fn load(
        &mut self,
        name: String,
        group: Group,
        translate: impl FnOnce(&Preprocessed, &Preprocessed) -> Result<Program, Vec<crate::Error>>,
    ) -> Option<PostProgram> {
        let path = |ext: &str| match self.folder {
            "" => format!("{name}.{ext}"),
            f => format!("{f}/{name}.{ext}"),
        };
        let (vsh, fsh) = (path("vsh"), path("fsh"));
        if group == Group::Setup || !self.pack.exists(&fsh) || !self.pack.exists(&vsh) {
            let on = enabled(
                self.properties,
                self.folder,
                &name,
                &self.options,
                &self.switches,
            );
            // A program of compute stages only (`.csh` without `.vsh` and `.fsh`).
            let composite = matches!(
                group,
                Group::Setup
                    | Group::Begin
                    | Group::Prepare
                    | Group::Deferred
                    | Group::Composite
                    | Group::ShadowComp
            );
            if !on || !composite {
                return None;
            }
            let compute = self.compute(&name);
            return (!compute.is_empty()).then(|| PostProgram {
                buffer_blends: buffer_blends(self.properties, &name),
                flips: flips(self.properties, &name),
                name,
                group,
                program: None,
                buffers: Vec::new(),
                blend: None,
                vanilla_sky_blends: false,
                mipmaps: 0,
                compute,
                geometry_triangles: 0,
                scale: FULL,
            });
        }
        if !enabled(
            self.properties,
            self.folder,
            &name,
            &self.options,
            &self.switches,
        ) {
            return None;
        }
        let tessellated = self.pack.exists(&path("tcs")) || self.pack.exists(&path("tes"));
        if tessellated {
            self.post.notes.push(format!(
                "{name}: uses a tessellation stage (unsupported); skipped"
            ));
            return None;
        }
        if self.pack.exists(&path("gsh")) {
            let Some(inputs) = self.geometry.clone() else {
                self.post.notes.push(format!(
                    "{name}: uses a geometry stage (supported for the shadow pass's terrain and entities only); skipped"
                ));
                return None;
            };
            return self.load_geometry(name.clone(), group, &vsh, &path("gsh"), &fsh, &inputs);
        }
        let vertex = self.preprocess(&vsh);
        let fragment = self.preprocess(&fsh);
        let notes = &mut self.post.notes;
        let problems: Vec<String> = [&vertex, &fragment]
            .iter()
            .flat_map(|pp| diagnostics(pp))
            .collect();
        if !problems.is_empty() {
            notes.extend(problems);
            notes.push(format!("{name}: skipped"));
            return None;
        }
        notes.extend([&vertex, &fragment].iter().flat_map(|pp| warnings(pp)));
        let program = match translate(&vertex, &fragment) {
            Ok(p) => p,
            Err(errors) => {
                notes.extend(
                    errors
                        .iter()
                        .map(|e| format!("{}:{}: {}", e.file, e.line, e.message)),
                );
                notes.push(format!("{name}: skipped"));
                return None;
            }
        };
        notes.extend(
            program
                .notes
                .iter()
                .map(|e| format!("{}:{}: {}", e.file, e.line, e.message)),
        );
        self.post.config.read_vertex(&vertex);
        self.post.config.read(&fragment);
        let mut buffers =
            config::draw_buffers(&fragment).unwrap_or_else(|| default_draw_buffers(group.shadow()));
        if group.shadow() {
            // Output n writes shadowcolor n (6.3); `shadow_buffers` says how many there are.
            let kept = buffers
                .iter()
                .position(|&b| b as usize >= self.shadow_buffers(&fragment))
                .unwrap_or(buffers.len());
            buffers.truncate(kept);
        }
        let mipmaps = match group {
            Group::Gbuffers | Group::Translucent | Group::Shadow | Group::ShadowComp => 0,
            _ => {
                let mut own = Config::default();
                own.read(&fragment);
                (own.buffers.iter().enumerate())
                    .filter(|b| b.1.mipmap)
                    .fold(0, |m, (i, _)| m | 1 << i)
            }
        };
        let compute = match group {
            Group::Setup
            | Group::Begin
            | Group::Prepare
            | Group::Deferred
            | Group::Composite
            | Group::Final
            | Group::ShadowComp => self.compute(&name),
            _ => Vec::new(),
        };
        Some(PostProgram {
            buffer_blends: buffer_blends(self.properties, &name),
            flips: flips(self.properties, &name),
            scale: scale(self.properties, &name, group),
            name,
            group,
            program: Some(program),
            buffers,
            blend: None,
            vanilla_sky_blends: false,
            mipmaps,
            compute,
            geometry_triangles: 0,
        })
    }

    /// Reads the colortex formats of program `name` without loading it, when it is there and
    /// enabled: those of its fragment shader, as [`Self::load`] reads them; with `shadow`, its
    /// shadow settings too.
    fn read_formats(&mut self, name: &str, shadow: bool) {
        let path = |ext: &str| match self.folder {
            "" => format!("{name}.{ext}"),
            f => format!("{f}/{name}.{ext}"),
        };
        let (vsh, fsh) = (path("vsh"), path("fsh"));
        let on = enabled(
            self.properties,
            self.folder,
            name,
            &self.options,
            &self.switches,
        );
        if !on || !self.pack.exists(&fsh) || !self.pack.exists(&vsh) {
            return;
        }
        let fragment = self.preprocess(&fsh);
        if shadow {
            self.post.config.read_formats_and_shadow(&fragment);
        } else {
            self.post.config.read_formats(&fragment);
        }
    }

    /// Stage file `path` through the preprocessor; the pack's images declared without a format
    /// (GLSL lets a write-only image omit it) get the one `image.*` gives them.
    fn preprocess(&self, path: &str) -> Preprocessed {
        let mut pp = preprocess(self.pack, path, self.macros);
        if self.post.images.is_empty() || !pp.code.contains("image") {
            return pp;
        }
        let source: Vec<&str> = pp.code.split('\n').collect();
        let lines: Vec<String> = source
            .iter()
            .enumerate()
            .map(|(n, &line)| {
                // A qualifier on the line before (`layout(r32ui)` then `uniform uimage3D v;`).
                let qualified = line.contains("layout")
                    || source[..n]
                        .iter()
                        .rev()
                        .find(|l| !l.trim().is_empty())
                        .is_some_and(|l| l.trim_start().starts_with("layout") && !l.contains(';'));
                let words: Vec<&str> = line
                    .split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
                    .collect();
                let image = self
                    .post
                    .images
                    .iter()
                    .find(|i| words.contains(&i.name.as_str()));
                match image {
                    Some(i)
                        if words.contains(&"uniform")
                            && !qualified
                            && words.iter().any(|w| {
                                ["image1D", "image2D", "image3D"]
                                    .iter()
                                    .any(|d| w.ends_with(d))
                            }) =>
                    {
                        format!("layout({}) {line}", i.qualifier)
                    }
                    _ => line.to_owned(),
                }
            })
            .collect();
        pp.code = lines.join("\n");
        pp
    }

    /// A program with a geometry stage, folded into its vertex stage.
    fn load_geometry(
        &mut self,
        name: String,
        group: Group,
        vsh: &str,
        gsh: &str,
        fsh: &str,
        inputs: &crate::GeometryInputs,
    ) -> Option<PostProgram> {
        let stages = [vsh, gsh, fsh].map(|p| self.preprocess(p));
        let notes = &mut self.post.notes;
        let problems: Vec<String> = stages.iter().flat_map(diagnostics).collect();
        if !problems.is_empty() {
            notes.extend(problems);
            notes.push(format!("{name}: skipped"));
            return None;
        }
        let [vertex, geometry, fragment] = &stages;
        let (program, triangles) =
            match crate::translate_geometry(vertex, geometry, fragment, inputs) {
                Ok(p) => p,
                Err(errors) => {
                    notes.extend(
                        errors
                            .iter()
                            .map(|e| format!("{}:{}: {}", e.file, e.line, e.message)),
                    );
                    notes.push(format!("{name}: skipped"));
                    return None;
                }
            };
        notes.extend(
            program
                .notes
                .iter()
                .map(|e| format!("{}:{}: {}", e.file, e.line, e.message)),
        );
        self.post.config.read_vertex(vertex);
        self.post.config.read(fragment);
        let mut buffers =
            config::draw_buffers(fragment).unwrap_or_else(|| default_draw_buffers(true));
        let kept = buffers
            .iter()
            .position(|&b| b as usize >= self.shadow_buffers(fragment))
            .unwrap_or(buffers.len());
        buffers.truncate(kept);
        Some(PostProgram {
            buffer_blends: buffer_blends(self.properties, &name),
            flips: flips(self.properties, &name),
            scale: scale(self.properties, &name, group),
            name,
            group,
            program: Some(program),
            buffers,
            blend: None,
            vanilla_sky_blends: false,
            mipmaps: 0,
            compute: Vec::new(),
            geometry_triangles: triangles,
        })
    }

    /// The shadowcolor buffers a shadow program's outputs can write: all the pack has with a
    /// `RENDERTARGETS` or `DRAWBUFFERS` directive, else shadowcolor0 and 1. Iris 1.11.7 with
    /// `HIGHER_SHADOWCOLOR`: a `shadow` program without a directive leaves shadowcolor2 to 7
    /// alone, one with `RENDERTARGETS: 0,1,2,3,4` writes all five (probe packs of our own).
    fn shadow_buffers(&self, fragment: &Preprocessed) -> usize {
        match config::draw_buffers(fragment) {
            Some(_) => self.post.shadow_colors,
            None => 2,
        }
    }

    /// The compute programs of composite-style program `name` that are on and translate.
    fn compute(&mut self, name: &str) -> Vec<(String, ComputeProgram)> {
        let mut out = Vec::new();
        let names =
            std::iter::once(name.to_owned()).chain(('a'..='z').map(|c| format!("{name}_{c}")));
        for name in names {
            let path = match self.folder {
                "" => format!("{name}.csh"),
                f => format!("{f}/{name}.csh"),
            };
            if !self.pack.exists(&path)
                || !enabled(
                    self.properties,
                    self.folder,
                    &name,
                    &self.options,
                    &self.switches,
                )
            {
                continue;
            }
            let pp = self.preprocess(&path);
            let problems = diagnostics(&pp);
            let notes = &mut self.post.notes;
            if !problems.is_empty() {
                notes.extend(problems);
                notes.push(format!("{name}: skipped"));
                continue;
            }
            match translate_compute(&pp) {
                Ok(program) => {
                    notes.extend(
                        program
                            .notes
                            .iter()
                            .map(|e| format!("{}:{}: {}", e.file, e.line, e.message)),
                    );
                    out.push((name, program));
                }
                Err(errors) => {
                    notes.extend(
                        errors
                            .iter()
                            .map(|e| format!("{}:{}: {}", e.file, e.line, e.message)),
                    );
                    notes.push(format!("{name}: skipped"));
                }
            }
        }
        out
    }
}

/// The buffers a program writes without `RENDERTARGETS` or `DRAWBUFFERS`: only buffer 0, except
/// in the shadow pass's own programs, where outputs 0 and 1 write shadowcolor0 and shadowcolor1.
/// Measured in Iris 1.11.7 with probe packs: a `gbuffers_terrain`, `begin`, `prepare`, `deferred`
/// or `composite` program that writes `gl_FragData[1..]` or `layout(location = n)` leaves those
/// buffers untouched, while the second output of `shadow` and `shadowcomp` lands in shadowcolor1.
fn default_draw_buffers(shadow: bool) -> Vec<u32> {
    if shadow { (0..8).collect() } else { vec![0] }
}

fn diagnostics(pp: &Preprocessed) -> Vec<String> {
    located(pp, &pp.diagnostics)
}

/// `pp`'s warnings (conditions read as false, unbalanced conditionals), located.
fn warnings(pp: &Preprocessed) -> Vec<String> {
    located(pp, &pp.warnings)
}

fn located(pp: &Preprocessed, list: &[crate::preprocess::Diagnostic]) -> Vec<String> {
    list.iter()
        .map(|d| {
            let file = pp
                .files
                .get(d.origin.file as usize)
                .map_or("?", String::as_str);
            format!("{file}:{}: {}", d.origin.line, d.message)
        })
        .collect()
}

/// `program.<name>.enabled` (or `program.<folder>/<name>.enabled`): a boolean expression over
/// switch options (3.3): those on by default (8.1) and the defined macros.
fn enabled(
    properties: &Properties,
    folder: &str,
    name: &str,
    macros: &Macros,
    switches: &BTreeSet<String>,
) -> bool {
    let keys = [
        format!("program.{folder}/{name}.enabled"),
        format!("program.{name}.enabled"),
    ];
    let Some(expr) = keys.iter().find_map(|k| properties.get(k)) else {
        return true;
    };
    // `true`/`false` as numbers, every other name as `defined(name)`, then the `#if` arithmetic.
    let mut condition = String::new();
    for token in crate::macros::lex(expr) {
        match token {
            "true" => condition.push('1'),
            "false" => condition.push('0'),
            t if switches.contains(t) => condition.push('1'),
            t if t.starts_with(|c: char| c.is_alphabetic() || c == '_') => {
                condition.push_str(&format!("defined({t})"));
            }
            t => condition.push_str(t),
        }
    }
    eval_condition(&condition, macros).ok() != Some(0)
}

/// `blend.<program>`: `Some(None)` for `off`, `Some(factors)`, or `None` when absent or not
/// four known factors.
fn blend(properties: &Properties, program: &str) -> Option<Option<Blend>> {
    let value = properties.get(&format!("blend.{program}"))?.trim();
    if value == "off" {
        return Some(None);
    }
    let factors: Vec<Factor> = value
        .split_whitespace()
        .map(Factor::parse)
        .collect::<Option<_>>()?;
    Some(Some(factors.try_into().ok()?))
}

/// Blend overrides apply to buffer names even when DRAWBUFFERS remaps locations.
fn buffer_blends(properties: &Properties, program: &str) -> Vec<(u32, Option<Blend>)> {
    let prefix = format!("blend.{program}.");
    properties
        .entries
        .iter()
        .filter_map(|e| {
            let buffer = config::buffer_index(e.key.strip_prefix(&prefix)?)?;
            let value = blend(
                properties,
                &format!("{program}.{}", e.key.strip_prefix(&prefix)?),
            )?;
            Some((buffer as u32, value))
        })
        .collect()
}

/// A program drawing all of its buffers.
const FULL: [f32; 3] = [1.0, 0.0, 0.0];

/// `scale.<program>=<scale> [<offsetX> <offsetY>]` of a deferred or composite program (OptiFine's
/// "Composite render scale"); prepare programs are treated the same, unverified. Iris 1.11.7
/// applies the offsets too (a probe pack of our own, composite). A value that is not a scale in (0, 1] with offsets in [0, 1) draws all of them.
fn scale(properties: &Properties, program: &str, group: Group) -> [f32; 3] {
    if !matches!(group, Group::Prepare | Group::Deferred | Group::Composite) {
        return FULL;
    }
    let Some(value) = properties.get(&format!("scale.{program}")) else {
        return FULL;
    };
    let numbers: Option<Vec<f32>> = value.split_whitespace().map(|n| n.parse().ok()).collect();
    let fraction = |s: f32| s > 0.0 && s <= 1.0;
    let offset = |o: f32| (0.0..1.0).contains(&o);
    match numbers.as_deref() {
        Some(&[s]) if fraction(s) => [s, 0.0, 0.0],
        Some(&[s, x, y]) if fraction(s) && offset(x) && offset(y) => [s, x, y],
        _ => FULL,
    }
}

/// Whether the pack names the `HIGHER_SHADOWCOLOR` feature, required or optional, in any case.
fn higher_shadowcolor(properties: &Properties) -> bool {
    ["iris.features.required", "iris.features.optional"]
        .iter()
        .filter_map(|k| properties.get(k))
        .any(|v| {
            v.split_whitespace()
                .any(|w| w.eq_ignore_ascii_case("HIGHER_SHADOWCOLOR"))
        })
}

/// `flip.<program>.<buffer>=<bool>` keys of one program.
fn flips(properties: &Properties, program: &str) -> Vec<(u32, bool)> {
    let prefix = format!("flip.{program}.");
    properties
        .entries
        .iter()
        .filter_map(|e| {
            let buffer = config::buffer_index(e.key.strip_prefix(&prefix)?)?;
            let value = match e.value.trim() {
                "true" => true,
                "false" => false,
                _ => return None,
            };
            Some((buffer as u32, value))
        })
        .collect()
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod lifecycle_tests;

#[cfg(test)]
mod buffer_blend_tests;

#[cfg(test)]
mod material_variant_tests;

#[cfg(test)]
mod sky_switch_tests;
