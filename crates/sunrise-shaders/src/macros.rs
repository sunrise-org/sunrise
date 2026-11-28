//! The macros a pack's programs and property files start with (spec 1.5, 1.5.1): `MC_VERSION`,
//! `MC_*`, the render stages, the biome categories and the renderer's. Defining and expanding
//! them is `sunrise-glsl`'s preprocessor's ([`sunrise_glsl::Macros`]), which this wraps.

pub(crate) use sunrise_glsl::macros::lex;

/// The feature flags of `iris.features.*` (spec 3.2) Sunrise has on every device (and
/// [`PER_BUFFER_BLENDING`] on a device that draws it). A pack that requires one of them loads;
/// one it lacks is noted. For each of them the pack lists, in either list, its programs and
/// property files see `IRIS_FEATURE_<NAME>` (spec 1.5.1, 3.2; decision D-7 and OQ-2 in
/// docs/cleanroom/): `Macros::define_features`, all but [`UNANSWERED`].
/// `BLOCK_EMISSION_ATTRIBUTE`: terrain's `at_midBlock.w` is the light its block gives, where the
/// draw reaches the quad's source block. `SEPARATE_HARDWARE_SAMPLERS` (ShaderDoc,
/// "Separate Hardware Shadow Samplers"): `shadowtex0`/`shadowtex1` stay plain depth samplers when
/// hardware filtering is on, `shadowtex0HW`/`shadowtex1HW` compare; Sunrise picks a comparison
/// sampler by the declared type (`sampler2DShadow`), so they always are. `HIGHER_SHADOWCOLOR`:
/// `shadowcolor2` to `shadowcolor7` beside the first two. `FADE_VARIABLE`: gbuffers programs read
/// `mc_chunkFade`, terrain's the section's visibility transition and every other draw's `-1.0`
/// (spec 5.1, `post::off_terrain_fade`).
const FEATURES: [&str; 9] = [
    "BLOCK_EMISSION_ATTRIBUTE",
    "COMPUTE_SHADERS",
    "CUSTOM_IMAGES",
    "ENTITY_TRANSLUCENT",
    "FADE_VARIABLE",
    "HIGHER_SHADOWCOLOR",
    "REVERSED_CULLING",
    "SEPARATE_HARDWARE_SAMPLERS",
    "SSBO",
];

/// The flags of [`FEATURES`] a pack requiring them loads with, but that get no
/// `IRIS_FEATURE_<NAME>`: Sunrise can't keep their promise on every draw it may take.
/// `BLOCK_EMISSION_ATTRIBUTE`: terrain drawn from its attributes alone
/// (`TerrainDraws::Attributes`, when neither the arena's pages nor the normal stream fit the
/// pack's vertex stage) has no source block, and `at_midBlock.w` is `0` there, so a pack's
/// emission branch would lose every block's light on that path.
const UNANSWERED: [&str; 1] = ["BLOCK_EMISSION_ATTRIBUTE"];

/// The one of [`FEATURES`] `name` is, in any case.
fn feature(name: &str) -> Option<&'static str> {
    FEATURES
        .iter()
        .copied()
        .find(|f| f.eq_ignore_ascii_case(name))
}

/// The keys that list the feature flags a pack asks for (spec 3.2).
pub(crate) const FEATURE_LISTS: [&str; 2] = ["iris.features.optional", "iris.features.required"];

/// The prefix of the macros that answer a pack's feature flags.
const FEATURE_MACRO: &str = "IRIS_FEATURE_";

/// Whether `name` is one of the macros that answer a pack's feature flags. Sunrise alone
/// answers them (`Macros::define_features`): a pack's line declaring such a name is no option.
/// It is not found as one (`options`, `switches_on`, `value_options`), so it is never defined for
/// the pack's property files, never counted as on in `program.*.enabled`, and a saved choice
/// never rewrites it (`Pack::with_chosen`, `options::apply`). As written, uncommented, it is the
/// pack's own directive, with a directive's scope (decision D-7): in a program, the rest of the
/// stage's preprocessed source; in `shaders.properties`, the rest of that file.
pub(crate) fn is_feature_answer(name: &str) -> bool {
    name.starts_with(FEATURE_MACRO)
}

/// Whether `name` is one of the macros Sunrise alone answers: a feature answer
/// ([`is_feature_answer`]) or an identity macro (`IS_IRIS`, `IRIS_VERSION`, decision D-8). A
/// pack's line declaring one is no option (`options`, `switches_on`, `value_options`): it is
/// never defined for the pack's property files, never counted as on in `program.*.enabled`, and
/// a saved choice never rewrites it, so neither an option nor a profile changes the answer. As
/// written, uncommented, it is the pack's own directive, with a directive's scope.
pub(crate) fn is_reserved(name: &str) -> bool {
    is_feature_answer(name) || crate::identity::NAMES.contains(&name)
}

/// The flags `properties` gives under `key`, as the pack wrote them.
fn flags<'a>(properties: &'a crate::Properties, key: &str) -> std::str::SplitWhitespace<'a> {
    properties.get(key).unwrap_or_default().split_whitespace()
}

/// The flags of [`FEATURES`] that `properties` lists in either of [`FEATURE_LISTS`], as
/// `FEATURES` writes them; a name matches in any case.
pub(crate) fn listed_features(
    properties: &crate::Properties,
) -> impl Iterator<Item = &'static str> {
    FEATURE_LISTS
        .iter()
        .flat_map(|key| flags(properties, key))
        .filter_map(feature)
}

/// The feature flag of `iris.features.*` (spec 3.2) Sunrise has only on a device that can draw
/// it: `PER_BUFFER_BLENDING`, a `blend.<program>.<buffer>` that differs from the program's other
/// buffers. wgpu creates a pipeline whose colour targets differ in blend or write mask only on a
/// device with `DownlevelFlags::INDEPENDENT_BLEND` ("Supports different blend operations per
/// color attachment"); the renderer gives its device's answer (`Macros::set_per_buffer_blending`).
/// Where it is supported it is answered as [`FEATURES`] are: listed, it gets
/// `IRIS_FEATURE_PER_BUFFER_BLENDING` (`Macros::define_features`).
const PER_BUFFER_BLENDING: &str = "PER_BUFFER_BLENDING";

/// `MC_VERSION` of the version Sunrise plays: 26.3 → 260300.
pub const MC_VERSION: u32 = mc_version(sunrise_core::version::VERSION_ID);

/// The documented rule, `major * 10000 + minor * 100 + patch` (spec 1.5.1, OQ-1): `1.21` → 12100,
/// `1.7.10` → 10710. Parts that are not digits count as 0.
pub const fn mc_version(id: &str) -> u32 {
    let bytes = id.as_bytes();
    let (mut parts, mut part, mut i) = ([0u32; 3], 0, 0);
    while i < bytes.len() && part < 3 {
        match bytes[i] {
            b'.' => part += 1,
            b @ b'0'..=b'9' => parts[part] = parts[part] * 10 + (b - b'0') as u32,
            _ => break,
        }
        i += 1;
    }
    parts[0] * 10000 + parts[1] * 100 + parts[2]
}

/// The macros at a point of a pack's file: `sunrise-glsl`'s, with the pack's own starting set.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Macros(sunrise_glsl::Macros, Device);

/// What the device the pack's programs run on can do, as far as a pack's feature flags ask
/// (spec 3.2). No macro: it decides which flags are supported. Without a renderer (the
/// standard set alone) the device can do none of it.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Device {
    /// [`PER_BUFFER_BLENDING`].
    per_buffer_blending: bool,
}

impl std::ops::Deref for Macros {
    type Target = sunrise_glsl::Macros;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl std::ops::DerefMut for Macros {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

/// The macro expansion work one file of a pack may spend ([`Macros::for_file`]), in the units
/// of [`sunrise_glsl::Macros::bound`] (for each token made, read or copied: 1, its bytes and the
/// names it hides): a stage file with its includes, a `.properties` file, the option values of
/// `shaders.properties`' sizes. As many as the tokens of work a core shader's directives may
/// spend ([`sunrise_glsl::MAX_SCANNED`]). A line of code is charged up to twice: telling whether
/// a call on it goes on past it expands it too ([`sunrise_glsl::Macros::join_call`]). A line of
/// expansions that reached the bound held 325 MiB at its peak (a release build); the ten packs
/// measured for #4135 spend at most 74,004 in a stage file and 104 in a `.properties` file, with
/// their default options.
pub const MAX_EXPANSION_WORK: usize = sunrise_glsl::MAX_SCANNED;

/// `MC_RENDER_STAGE_*` in the order the stages draw (spec 5.4); each is its index (OQ-7).
pub const RENDER_STAGES: [&str; 24] = [
    "NONE",
    "SKY",
    "SUNSET",
    "CUSTOM_SKY",
    "SUN",
    "MOON",
    "STARS",
    "VOID",
    "TERRAIN_SOLID",
    "TERRAIN_CUTOUT_MIPPED",
    "TERRAIN_CUTOUT",
    "ENTITIES",
    "BLOCK_ENTITIES",
    "DESTROY",
    "OUTLINE",
    "DEBUG",
    "HAND_SOLID",
    "TERRAIN_TRANSLUCENT",
    "TRIPWIRE",
    "PARTICLES",
    "CLOUDS",
    "RAIN_SNOW",
    "WORLD_BORDER",
    "HAND_TRANSLUCENT",
];

/// `DH_BLOCK_*`, each its index: Distant Sunrises' block materials.
pub const DH_BLOCK_MATERIALS: [&str; 16] = [
    "UNKNOWN",
    "LEAVES",
    "STONE",
    "WOOD",
    "METAL",
    "DIRT",
    "LAVA",
    "DEEPSLATE",
    "SNOW",
    "SAND",
    "TERRACOTTA",
    "NETHER_STONE",
    "WATER",
    "GRASS",
    "AIR",
    "ILLUMINATED",
];

/// The value of `MC_RENDER_STAGE_<name>` (OQ-7: its index in `RENDER_STAGES`).
pub fn render_stage(name: &str) -> u32 {
    RENDER_STAGES
        .iter()
        .position(|s| *s == name)
        .unwrap_or_else(|| panic!("no render stage {name}")) as u32
}

impl Macros {
    /// These macros, for one file of the pack: with [`MAX_EXPANSION_WORK`] to spend, whatever
    /// was spent before. Past it an expansion stops, named at its line by whoever reads the file.
    pub(crate) fn for_file(&self) -> Self {
        let mut m = self.clone();
        m.0.bound(MAX_EXPANSION_WORK);
        m
    }

    /// The standard macros that need no renderer (spec 1.5.1): `MC_VERSION`, this system's
    /// `MC_OS_*`, the GL and GLSL versions Iris reports, the render stages and the host
    /// settings at their defaults. The GPU vendor and family arrive with the renderer
    /// (`define_renderer`).
    pub fn standard() -> Self {
        let mut m = Self::default();
        m.define("MC_VERSION", MC_VERSION.to_string());
        let os = match std::env::consts::OS {
            "windows" => "MC_OS_WINDOWS",
            "macos" => "MC_OS_MAC",
            "linux" => "MC_OS_LINUX",
            _ => "MC_OS_OTHER",
        };
        m.define(os, "");
        // What Iris 1.11.7 reports on 26.3, whatever the pack's `#version` (observed with a probe
        // pack): its GL context's 3.3. Packs then take the paths they take there; the
        // translation itself reads GLSL up to 4.60.
        m.define("MC_GL_VERSION", "330");
        m.define("MC_GLSL_VERSION", "330");
        for (i, stage) in RENDER_STAGES.iter().enumerate() {
            m.define(format!("MC_RENDER_STAGE_{stage}"), i.to_string());
        }
        m.define("MC_RENDER_QUALITY", "1.0");
        m.define("MC_SHADOW_QUALITY", "1.0");
        m.define("MC_HAND_DEPTH", "0.125");
        m.define("MAX_COLOR_BUFFERS", "16");
        // The normal and specular maps are on (OptiFine's shaders.txt, H: their options, on by
        // default; Iris 1.11.7 defines both too, with or without a resource pack's maps).
        m.define("MC_NORMAL_MAP", "");
        m.define("MC_SPECULAR_MAP", "");
        // No `IS_IRIS`, `IRIS_VERSION` or `IRIS_HAS_*` here (spec OQ-2, docs/cleanroom/README.md):
        // the identity macros are defined only for a pack whose Iris compatibility is on
        // (`set_identity`, decision D-8), and the `IRIS_FEATURE_*` answers depend on the pack
        // (`define_features`).
        // Distant Sunrises' block materials, which Iris defines with or without the mod (packs
        // compare ids with them unguarded: Exposa's `blockID == DH_BLOCK_LEAVES`), numbered as
        // the mod's `EDhApiBlockMaterial` numbers them.
        for (i, material) in DH_BLOCK_MATERIALS.iter().enumerate() {
            m.define(format!("DH_BLOCK_{material}"), i.to_string());
        }
        // The biome categories and precipitations the `biome_category` and
        // `biome_precipitation` uniforms compare with, as custom uniforms see them (Iris's
        // replacements define them for the stage files too: Sildur's Enhanced Default reads
        // `biome_category == CAT_THE_END`). `BIOME_*` needs the world's registry.
        for (i, category) in crate::biomes::CATEGORIES.iter().enumerate() {
            m.define(format!("CAT_{category}"), i.to_string());
        }
        for (i, precipitation) in ["NONE", "RAIN", "SNOW"].iter().enumerate() {
            m.define(format!("PPT_{precipitation}"), i.to_string());
        }
        m
    }

    /// The macros the renderer adds: the mipmap levels terrain is drawn with (`MC_MIPMAP_LEVEL`,
    /// Iris's: it follows the Mipmap Levels option), and `OTHER` as the GL vendor and renderer
    /// (OptiFine's shaders.txt, E and F: one of each is defined). Sunrise draws through wgpu, not
    /// a GL driver, so a pack's paths for one driver (an NVIDIA extension it takes behind
    /// `MC_GL_VENDOR_NVIDIA`) are not there to take.
    pub fn define_renderer(&mut self, mipmap_levels: u32) {
        self.define("MC_GL_VENDOR_OTHER", "");
        self.define("MC_GL_RENDERER_OTHER", "");
        self.define("MC_MIPMAP_LEVEL", mipmap_levels.to_string());
    }

    /// The `MC_GL_KHR_shader_subgroup_*` macros of a device with subgroup operations
    /// (`wgpu::Features::SUBGROUP`): the GL extensions the translator then takes.
    pub fn define_subgroups(&mut self) {
        for ext in [
            "basic",
            "vote",
            "arithmetic",
            "ballot",
            "shuffle",
            "shuffle_relative",
        ] {
            self.define(format!("MC_GL_KHR_shader_subgroup_{ext}"), "");
        }
    }

    /// Sunrise's HDR extension, served to a pack that asked for it (`sunrise.hdr`,
    /// [`crate::Properties::sunrise_hdr`]) and got a frame made for it: `SUNRISE_HDR` at the
    /// extension's version, 1 (its `final` writes linear light), and `SUNRISE_HDR_TONEMAP_PACK`
    /// when the pack owns the tone curve (`tonemap`). Sunrise's own names, which no other loader
    /// defines: there the pack takes its other path. Never part of [`Self::standard`].
    pub fn define_sunrise_hdr(&mut self, tonemap: crate::Tonemap) {
        self.define("SUNRISE_HDR", "1");
        if tonemap == crate::Tonemap::Pack {
            self.define("SUNRISE_HDR_TONEMAP_PACK", "");
        }
    }

    /// Whether the device the pack's programs run on draws a `blend.<program>.<buffer>` that
    /// differs from the program's other buffers ([`PER_BUFFER_BLENDING`]): the renderer's
    /// answer, from its device. Defines no macro itself: a pack that lists the flag gets its
    /// answer from `define_features`. A device without it drops the answer these macros held,
    /// at once.
    pub fn set_per_buffer_blending(&mut self, supported: bool) {
        self.1.per_buffer_blending = supported;
        if !supported {
            self.undef(&format!("{FEATURE_MACRO}{PER_BUFFER_BLENDING}"));
        }
    }

    /// Drops every answer to a pack's feature flags these macros hold ([`is_feature_answer`]),
    /// whoever defined it: a set reused for another pack, or after the device changed, holds
    /// none of the previous answers.
    pub(crate) fn drop_feature_answers(&mut self) {
        let answers: Vec<String> = (self.names())
            .filter(|n| is_feature_answer(n))
            .map(str::to_owned)
            .collect();
        for name in &answers {
            self.undef(name);
        }
    }

    /// The feature flag `name` is (spec 3.2), in any case, as Sunrise writes it, if Sunrise has
    /// it with these macros' device: one of [`FEATURES`], or [`PER_BUFFER_BLENDING`] on a device
    /// that draws it.
    fn supported(&self, name: &str) -> Option<&'static str> {
        let device = self.1.per_buffer_blending && PER_BUFFER_BLENDING.eq_ignore_ascii_case(name);
        feature(name).or(device.then_some(PER_BUFFER_BLENDING))
    }

    /// A note for each feature `iris.features.required` names that Sunrise lacks with these
    /// macros' device (`supported`); a name matches in any case. The pack still loads.
    pub fn feature_notes(&self, properties: &crate::Properties) -> Vec<String> {
        flags(properties, "iris.features.required")
            .filter(|f| self.supported(f).is_none())
            .map(|f| format!("the pack requires `{f}`, which is not supported yet"))
            .collect()
    }

    /// Defines `IRIS_FEATURE_<NAME>` as `1` for each flag Sunrise has with these macros' device
    /// (`supported`: one of [`FEATURES`], or [`PER_BUFFER_BLENDING`] where the device draws it)
    /// that `properties` lists in `iris.features.optional` (spec 3.2) or
    /// `iris.features.required` (decision D-7), a name matching in any case and the macro taking
    /// the name as Sunrise writes it. A flag Sunrise lacks, and one of [`UNANSWERED`], stays
    /// undefined. The answers these macros held before are dropped first
    /// (`drop_feature_answers`), so the set holds this read's answers alone. Whether any was
    /// defined.
    pub fn define_features(&mut self, properties: &crate::Properties) -> bool {
        self.drop_feature_answers();
        let mut defined = false;
        let answered: Vec<&'static str> = (FEATURE_LISTS.iter())
            .flat_map(|key| flags(properties, key))
            .filter_map(|f| self.supported(f))
            .filter(|f| !UNANSWERED.contains(f))
            .collect();
        for name in answered {
            self.define(format!("{FEATURE_MACRO}{name}"), "1");
            defined = true;
        }
        defined
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Whether `fsh` translates with `m`: a program that declares what it reads only under
    /// a macro's condition translates when the condition holds.
    fn translates(fsh: &str, m: &Macros) -> Result<(), String> {
        let vsh = "#version 330\nvoid main() { gl_Position = vec4(0.0); }\n";
        let pack = crate::Pack::from_zip_bytes(
            "P.zip",
            crate::pack::tests::zip(&[
                ("shaders/composite.vsh", vsh),
                ("shaders/composite.fsh", fsh),
            ]),
        )
        .unwrap();
        crate::translate_composite(
            &crate::preprocess(&pack, "composite.vsh", m),
            &crate::preprocess(&pack, "composite.fsh", m),
            crate::Target::Buffers,
            naga::front::glsl::InterfaceLimits::WGPU_DEFAULT,
        )
        .map(|_| ())
        .map_err(|e| format!("{e:?}"))
    }

    /// Packs see GL and GLSL 3.3, and no other loader's identity (spec OQ-2): a declaration made
    /// only under that condition is seen.
    #[test]
    fn programs_see_gl_33_and_no_other_loader() {
        let fsh = "#version 330\nout vec4 c;\n\
                   #if MC_GL_VERSION == 330 && MC_GLSL_VERSION == 330 && !defined IS_IRIS \\\n\
                   && !defined IRIS_VERSION && !defined IRIS_HAS_TRANSLUCENCY_SORTING\n\
                   const float v = 1.0;\n#endif\n\
                   void main() { c = vec4(v); }\n";
        translates(fsh, &Macros::standard()).unwrap();
    }

    /// With the renderer's macros a pack sees one GL vendor and renderer, `OTHER`, and the
    /// mipmap levels terrain is drawn with.
    #[test]
    fn programs_see_the_renderers_vendor_and_mipmap_levels() {
        let fsh = "#version 330\nout vec4 c;\n\
                   #if defined MC_GL_VENDOR_OTHER && defined MC_GL_RENDERER_OTHER && MC_MIPMAP_LEVEL == 2\n\
                   const float v = 1.0;\n#endif\n\
                   #if defined MC_GL_VENDOR_NVIDIA || defined MC_GL_RENDERER_GEFORCE\nconst float v = 2.0;\n#endif\n\
                   void main() { c = vec4(v); }\n";
        let mut m = Macros::standard();
        m.define_renderer(2);
        translates(fsh, &m).unwrap();
    }

    /// Packs take their normal- and specular-map branches, as they do with the maps on: a
    /// declaration made only there is seen.
    #[test]
    fn programs_take_the_normal_and_specular_map_branches() {
        let fsh = "#version 330\nout vec4 c;\n\
                   #ifdef MC_NORMAL_MAP\nconst float bump = 1.0;\n#endif\n\
                   #ifdef MC_SPECULAR_MAP\nconst float shine = 2.0;\n#endif\n\
                   void main() { c = vec4(bump, shine, 0.0, 1.0); }\n";
        translates(fsh, &Macros::standard()).unwrap();
    }

    /// A required feature Sunrise has loads without a note, one it lacks gets a note, whatever the
    /// case it is written in; the standard and renderer sets hold no `IRIS_*` macro (the
    /// feature answers come from the pack's lists, `define_features`).
    /// The feature flags a list of notes names, in order: tests check which flags are noted, not the
    /// English of the note.
    fn noted_flags(notes: &[String]) -> Vec<&str> {
        notes.iter().filter_map(|n| n.split('`').nth(1)).collect()
    }

    #[test]
    fn required_features_are_noted_and_the_standard_set_names_no_feature() {
        let properties = crate::Properties::parse(
            "iris.features.optional = COMPUTE_SHADERS ssbo NOT_A_FEATURE\n\
             iris.features.required = BLOCK_EMISSION_ATTRIBUTE HIGHER_SHADOWCOLOR custom_images \
             Not_A_Feature\n",
            &Macros::standard(),
        );
        assert_eq!(
            noted_flags(&Macros::standard().feature_notes(&properties)),
            ["Not_A_Feature"]
        );
        let mut m = Macros::standard();
        m.define_renderer(4);
        let iris: Vec<_> = m.names().filter(|name| name.contains("IRIS")).collect();
        assert!(iris.is_empty(), "{iris:?}");
    }

    /// Sunrise's HDR macros are absent from the standard set and the renderer's; served, the
    /// version is 1, and the tone curve's macro follows its owner.
    #[test]
    fn sunrise_hdr_macros_appear_only_when_served() {
        const NAMES: [&str; 2] = ["SUNRISE_HDR", "SUNRISE_HDR_TONEMAP_PACK"];
        let mut m = Macros::standard();
        m.define_renderer(4);
        m.define_subgroups();
        assert!(NAMES.iter().all(|n| !m.is_defined(n)));
        let mut pack = m.clone();
        pack.define_sunrise_hdr(crate::Tonemap::Pack);
        assert_eq!(pack.get("SUNRISE_HDR"), Some("1"));
        assert_eq!(pack.get("SUNRISE_HDR_TONEMAP_PACK"), Some(""));
        let mut sunrise = m.clone();
        sunrise.define_sunrise_hdr(crate::Tonemap::Sunrise);
        assert_eq!(sunrise.get("SUNRISE_HDR"), Some("1"));
        assert!(!sunrise.is_defined("SUNRISE_HDR_TONEMAP_PACK"));
        // A program's `#ifdef SUNRISE_HDR` path is taken only when served.
        let fsh = "#version 330\nout vec4 c;\n#ifdef SUNRISE_HDR\nconst float v = 3.0;\n#endif\n\
                   void main() { c = vec4(v); }\n";
        assert!(translates(fsh, &m).is_err());
        translates(fsh, &pack).unwrap();
    }

    /// A required `PER_BUFFER_BLENDING`, in any case, is supported where the device draws it
    /// and noted where it does not, as written; the other flags keep their answers, and the
    /// device's answer defines no macro.
    #[test]
    fn per_buffer_blending_follows_the_device() {
        let properties = crate::Properties::parse(
            "iris.features.optional = PER_BUFFER_BLENDING
             iris.features.required = SSBO PER_BUFFER_BLENDING per_Buffer_blending NOT_A_FEATURE
",
            &Macros::standard(),
        );
        let mut able = Macros::standard();
        able.define_renderer(4);
        let mut unable = able.clone();
        able.set_per_buffer_blending(true);
        unable.set_per_buffer_blending(false);
        assert_eq!(
            noted_flags(&able.feature_notes(&properties)),
            ["NOT_A_FEATURE"]
        );
        assert_eq!(
            noted_flags(&unable.feature_notes(&properties)),
            [
                "PER_BUFFER_BLENDING",
                "per_Buffer_blending",
                "NOT_A_FEATURE"
            ]
        );
        // Without a renderer's answer the device draws none of it.
        assert_eq!(
            Macros::standard().feature_notes(&properties),
            unable.feature_notes(&properties)
        );
        assert!(able.names().eq(unable.names()));
        // The answer reaches every file's macros and the properties' read.
        assert_eq!(
            able.for_file().feature_notes(&properties),
            able.feature_notes(&properties)
        );
    }

    /// The macro set itself holds no stale answer: a device without per-buffer blending drops
    /// `IRIS_FEATURE_PER_BUFFER_BLENDING` at once, and `define_features` holds only its own
    /// read's answers, whatever the set held before.
    #[test]
    fn a_stale_feature_answer_leaves_the_macro_set() {
        const PBB: &str = "IRIS_FEATURE_PER_BUFFER_BLENDING";
        let parse = |text| crate::Properties::parse(text, &Macros::standard());
        let listing = parse("iris.features.optional = PER_BUFFER_BLENDING SSBO\n");
        let other = parse("iris.features.optional = CUSTOM_IMAGES\n");
        let mut m = Macros::standard();
        m.set_per_buffer_blending(true);
        assert!(m.define_features(&listing));
        assert_eq!(m.get(PBB), Some("1"));
        assert_eq!(m.get("IRIS_FEATURE_SSBO"), Some("1"));
        // The device answer turned no: the macro goes with it, the other answers stay.
        m.set_per_buffer_blending(false);
        assert!(!m.is_defined(PBB));
        assert_eq!(m.get("IRIS_FEATURE_SSBO"), Some("1"));
        // Answered again on that device: still undefined.
        assert!(m.define_features(&listing));
        assert!(!m.is_defined(PBB));
        // A read of another pack holds its answers alone.
        m.set_per_buffer_blending(true);
        assert!(m.define_features(&listing));
        assert_eq!(m.get(PBB), Some("1"));
        assert!(m.define_features(&other));
        let answers: Vec<&str> = m
            .names()
            .filter(|n| n.starts_with("IRIS_FEATURE_"))
            .collect();
        assert_eq!(answers, ["IRIS_FEATURE_CUSTOM_IMAGES"]);
        // One that lists none Sunrise has holds none.
        assert!(!m.define_features(&parse("iris.features.optional = NOT_A_FEATURE\n")));
        assert_eq!(m, {
            let mut able = Macros::standard();
            able.set_per_buffer_blending(true);
            able
        });
    }

    #[test]
    fn versions_follow_the_documented_rule() {
        assert_eq!(mc_version("1.21"), 12100);
        assert_eq!(mc_version("1.21.0"), 12100);
        assert_eq!(mc_version("1.7.10"), 10710);
        assert_eq!(mc_version("26.3"), 260300);
        assert_eq!(MC_VERSION, 260300);
        assert_eq!(Macros::standard().get("MC_VERSION"), Some("260300"));
    }

    #[test]
    fn distant_sunrises_block_materials_are_defined_without_the_mod() {
        let m = Macros::standard();
        assert_eq!(m.get("DH_BLOCK_UNKNOWN"), Some("0"));
        assert_eq!(m.get("DH_BLOCK_LEAVES"), Some("1"));
        assert_eq!(m.get("DH_BLOCK_WATER"), Some("12"));
        assert_eq!(m.get("DH_BLOCK_ILLUMINATED"), Some("15"));
        assert_eq!(m.get("DISTANT_SUNRISES"), None);
    }

    #[test]
    fn biome_categories_and_precipitations_are_macros_as_custom_uniforms_see_them() {
        let m = Macros::standard();
        let constants = crate::biomes::constants(&[]);
        for (name, value) in &constants {
            assert_eq!(m.get(name), Some(value.to_string().as_str()), "{name}");
        }
        assert_eq!(m.get("CAT_THE_END"), Some("8"));
        assert_eq!(m.get("PPT_SNOW"), Some("2"));
    }
}
