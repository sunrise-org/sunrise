//! One case through the game's load path, twice, and the invariants every load must keep.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::hash::Hasher;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::Path;

use sunrise_render_shaderpack::pack_atlas::{TerrainDraws, deviceless_game_inputs};
use sunrise_shaders::{Macros, Pack, Post, PostProgram, Properties};

use crate::cases::DIMENSIONS;

/// What a case asks of the checker besides its files.
#[derive(Clone, Debug, PartialEq)]
pub struct Request {
    pub dim: usize,
    pub flat: bool,
    /// Programs that must load or have a correctly named refusal diagnostic: the unmutated
    /// case loaded them and this candidate's active files still include them.
    pub expect: Vec<String>,
    /// Enabled compute-stage names the loader should dispatch for this candidate.
    pub expect_compute: Vec<String>,
    /// Bytes a stage file may grow to through `#include` and macros.
    pub size_limit: usize,
}

/// A broken invariant: its name, and what about it (a program, a location, an error kind).
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Failure {
    pub invariant: String,
    pub detail: String,
}

impl Failure {
    pub fn new(invariant: &str, detail: impl Into<String>) -> Self {
        Self {
            invariant: invariant.to_owned(),
            detail: detail.into(),
        }
    }

    pub fn signature(&self) -> String {
        // Keep the measured size in `detail` for reports, but shrinking the same oversized
        // stage must preserve its failure even as its byte count changes.
        if self.invariant == "size"
            && let Some((stage, _)) = self.detail.split_once(": ")
        {
            return format!("{}|{stage}", self.invariant);
        }
        format!("{}|{}", self.invariant, self.detail)
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Outcome {
    pub failures: Vec<Failure>,
    /// The programs that loaded, in the order `observe` lists them.
    pub loaded: Vec<String>,
    /// The compute stages that loaded, by exact stage name rather than owner program.
    pub compute_loaded: Vec<String>,
    pub shadow_pass: bool,
}

/// What one load gave, reduced to what the invariants compare.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Observed {
    pub loaded: Vec<String>,
    pub compute_loaded: Vec<String>,
    pub folder: String,
    pub stage_files: Vec<String>,
    pub notes: Vec<String>,
    /// Loader notes that actually say a program was skipped; property/custom diagnostics are not refusals.
    pub refusals: Vec<String>,
    /// Loader-typed rejections; prose alone never excuses a missing program or stage.
    pub loader_refusals: Vec<sunrise_shaders::LoaderRefusal>,
    /// Exact enabled compute stages the loader attempted, including rejected stages.
    pub compute_stage_attempts: Vec<sunrise_shaders::ComputeStageAttempt>,
    /// Diagnostics whose subjects could not be tied to the loaded pack's source or properties.
    pub ungrounded_diagnostics: Vec<String>,
    pub shadow_pass: bool,
    pub custom: sunrise_shaders::Custom,
    pub stage_sizes: BTreeMap<String, usize>,
    /// Each module that does not validate: `program stage`, and naga's error.
    pub invalid: Vec<(String, String)>,
    /// A hash of each part of the output, labelled.
    pub pieces: Vec<(String, u64)>,
}

/// The pack in `dir` loaded as the game loads a pack for a dimension (`shader_packs.rs`): the
/// standard macros and the renderer's at Mipmap Levels 4, `shaders.properties` with the
/// features it turns on, the dimension's folder, and the game's own inputs.
pub fn load(dir: &Path, request: &Request) -> Observed {
    load_with_post(dir, request).0
}

/// Load a pack once and return both the checker observation and the `Post` from that same load.
/// Fixtures that audit resources can assert against the exact sample represented by `Observed`.
pub fn load_with_post(dir: &Path, request: &Request) -> (Observed, Post) {
    let pack = Pack::open(dir).expect("the case folder is a pack");
    let mut macros = Macros::standard();
    macros.define_renderer(4);
    let (properties, feature_notes) = sunrise_shaders::read_properties(&pack, &mut macros);
    let property_text = pack.read_text("shaders.properties").unwrap_or_default();
    let mut notes = feature_notes.clone();
    notes.extend(
        properties
            .notes
            .iter()
            .map(|note| format!("shaders.properties:{}: {}", note.line, note.message)),
    );
    let (id, legacy) = DIMENSIONS[request.dim];
    let folder = sunrise_shaders::dimension_folder_named(&pack, id, legacy);
    let (post, stage_sizes) = sunrise_shaders::load_post_with_sizes(
        &pack,
        &folder,
        &properties,
        &macros,
        &inputs(&properties, request.flat),
    );
    let property_keys: BTreeMap<&str, &str> = properties
        .entries
        .iter()
        .map(|entry| (entry.key.as_str(), entry.value.as_str()))
        .collect();
    let all_paths = pack.all_files();
    let source_texts: BTreeMap<String, String> = all_paths
        .iter()
        .filter(|path| *path == "shaders.properties" || shader_text_source(path))
        .filter_map(|path| Some((path.clone(), pack.read_text(path)?)))
        .collect();
    let mut ungrounded_diagnostics = Vec::new();
    for note in &feature_notes {
        if !feature_note_has_subject(note, &properties) {
            ungrounded_diagnostics.push(note.clone());
        }
    }
    for note in &properties.notes {
        let rendered = format!("shaders.properties:{}: {}", note.line, note.message);
        if !source_location_has_subject(note.line, &property_text) {
            ungrounded_diagnostics.push(rendered);
        }
    }
    let loaded_programs: Vec<String> = programs(&post)
        .into_iter()
        .map(|(_, program)| program.name.clone())
        .collect();
    let shadow_gap = post.shadow.is_none();
    for note in &post.notes {
        if !post_note_has_subject(
            note,
            &property_keys,
            &source_texts,
            &post.loader_refusals,
            &loaded_programs,
            post.terrain_undrawn,
            post.water_undrawn,
            shadow_gap,
        ) {
            ungrounded_diagnostics.push(note.clone());
        }
    }
    for note in &post.config.notes {
        if !located_note_has_subject(note, &source_texts) {
            ungrounded_diagnostics.push(note.clone());
        }
    }
    for note in &post.custom.notes {
        if !custom_note_has_subject(note, &property_keys) {
            ungrounded_diagnostics.push(note.clone());
        }
    }
    let refusals = post
        .notes
        .iter()
        .filter(|note| note.ends_with("skipped"))
        .cloned()
        .collect();
    let prefix = if post.folder.is_empty() {
        String::new()
    } else {
        format!("{}/", post.folder)
    };
    let stage_files = pack
        .all_files()
        .into_iter()
        .filter(|path| {
            let Some(relative) = path.strip_prefix(&prefix) else {
                return false;
            };
            !relative.contains('/')
                && [".vsh", ".fsh", ".gsh", ".csh", ".tcs", ".tes"]
                    .iter()
                    .any(|extension| relative.ends_with(extension))
        })
        .collect();
    notes.extend(post.notes.iter().cloned());
    notes.extend(post.config.notes.iter().cloned());
    notes.extend(post.custom.notes.iter().cloned());
    let mut observed = observe(&post, notes, refusals, stage_files, stage_sizes);
    observed.loader_refusals = post.loader_refusals.clone();
    observed.compute_stage_attempts = post.compute_stage_attempts.clone();
    observed.ungrounded_diagnostics = ungrounded_diagnostics;
    (observed, post)
}

fn shader_text_source(path: &str) -> bool {
    [
        ".vsh",
        ".fsh",
        ".gsh",
        ".csh",
        ".tcs",
        ".tes",
        ".glsl",
        ".inc",
        ".settings",
    ]
    .iter()
    .any(|extension| path.ends_with(extension))
}

fn source_location_has_subject(line: u32, text: &str) -> bool {
    line == 0 || line as usize <= text.lines().count().max(1)
}

fn located_note_has_subject(note: &str, source_texts: &BTreeMap<String, String>) -> bool {
    source_texts.iter().any(|(path, text)| {
        let Some(rest) = note.strip_prefix(&format!("{path}:")) else {
            return false;
        };
        let rest = rest.trim_start();
        if rest.is_empty() {
            return false;
        }
        let Some((line, suffix)) = rest.split_once(':') else {
            // Some preprocessor and file-level diagnostics identify only their source file.
            return true;
        };
        let Ok(line) = line.trim().parse::<u32>() else {
            // `file:message` has a source subject even though it has no line location.
            return true;
        };

        let suffix = suffix.trim_start();
        let message = match suffix.split_once(':') {
            // Naga-style diagnostics may report `file:line:column: message`.
            Some((column, message)) if column.trim().parse::<u32>().is_ok() => message.trim_start(),
            // The existing preprocessor form is `file:line: message`.
            _ => suffix,
        };
        !message.trim().is_empty() && source_location_has_subject(line, text)
    })
}

fn feature_note_has_subject(note: &str, properties: &sunrise_shaders::Properties) -> bool {
    let Some(feature) = note.strip_prefix("the pack requires `").and_then(|rest| {
        rest.split_once("`, which is not supported yet")
            .map(|(f, _)| f)
    }) else {
        return false;
    };
    properties
        .get("iris.features.required")
        .unwrap_or_default()
        .split_whitespace()
        .any(|required| required == feature)
}

fn first_quoted(note: &str) -> Option<&str> {
    note.split_once('`')?
        .1
        .split_once('`')
        .map(|(subject, _)| subject)
}

fn post_note_has_subject(
    note: &str,
    property_keys: &BTreeMap<&str, &str>,
    source_texts: &BTreeMap<String, String>,
    loader_refusals: &[sunrise_shaders::LoaderRefusal],
    loaded_programs: &[String],
    terrain_undrawn: bool,
    water_undrawn: bool,
    shadow_gap: bool,
) -> bool {
    if located_note_has_subject(note, source_texts) {
        return true;
    }
    if loaded_programs.iter().any(|name| {
        note.strip_prefix(name)
            .is_some_and(|rest| rest.starts_with(": "))
    }) {
        return true;
    }
    if (note == "no program for terrain; nothing draws it" && terrain_undrawn)
        || (note == "no program for translucent terrain; nothing draws it" && water_undrawn)
        || (note == "shadow: no program; the shadow map stays clear" && shadow_gap)
    {
        return true;
    }
    if let Some(name) = note.split_once(": ").map(|(name, _)| name)
        && loader_refusals.iter().any(|refusal| match refusal {
            sunrise_shaders::LoaderRefusal::Program(program) => program == name,
            sunrise_shaders::LoaderRefusal::ComputeStage(path) => {
                path.rsplit('/')
                    .next()
                    .and_then(|file| file.strip_suffix(".csh"))
                    == Some(name)
            }
        })
    {
        return true;
    }
    first_quoted(note).is_some_and(|subject| {
        property_keys.contains_key(subject)
            || property_keys.values().any(|value| value.contains(subject))
    })
}

fn custom_note_has_subject(note: &str, property_keys: &BTreeMap<&str, &str>) -> bool {
    if let Some((key, _)) = note.split_once(": ")
        && (key.starts_with("uniform.") || key.starts_with("variable."))
    {
        return property_keys.contains_key(key);
    }
    let Some(subject) = first_quoted(note) else {
        return false;
    };
    property_keys.iter().any(|(key, value)| {
        (key.starts_with("uniform.") || key.starts_with("variable."))
            && (key.ends_with(subject)
                || value
                    .split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
                    .any(|token| token == subject))
    })
}

/// The pack's `shaders.properties` and the macros its programs are read with, as the game reads
/// them (`shader_packs.rs`): the standard macros and the renderer's at Mipmap Levels 4, then
/// `shaders.properties` with the features it turns on.
pub fn pack_macros(pack: &Pack) -> (Properties, Macros) {
    let mut macros = Macros::standard();
    macros.define_renderer(4);
    let (properties, _) = sunrise_shaders::read_properties(pack, &mut macros);
    (properties, macros)
}

/// The game's inputs without a device ([`deviceless_game_inputs`]): `flat` picks the game's flat
/// draws, the others per-section ones as attributes alone.
pub fn inputs(properties: &Properties, flat: bool) -> sunrise_shaders::GameInputs {
    deviceless_game_inputs(properties.separate_ao(), TerrainDraws::flat(flat))
}

/// Every program of `post` with a slot, by slot.
fn programs(post: &Post) -> Vec<(&'static str, &PostProgram)> {
    let slots: [(&'static str, &Option<PostProgram>); 23] = [
        ("terrain", &post.terrain),
        ("terrain_cutout", &post.terrain_cutout),
        ("water", &post.water),
        ("sky", &post.sky),
        ("sky_textured", &post.sky_textured),
        ("entities", &post.entities),
        ("block", &post.block),
        ("block_translucent", &post.block_translucent),
        ("entities_translucent", &post.entities_translucent),
        ("particles", &post.particles),
        ("particles_translucent", &post.particles_translucent),
        ("beacon_beam", &post.beacon_beam),
        ("damaged_block", &post.damaged_block),
        ("clouds", &post.clouds),
        ("lines", &post.lines),
        ("weather", &post.weather),
        ("hand", &post.hand),
        ("hand_water", &post.hand_water),
        ("shadow", &post.shadow),
        ("shadow_water", &post.shadow_water),
        ("shadow_water_plain", &post.shadow_water_plain),
        ("shadow_entities", &post.shadow_entities),
        ("shadow_block", &post.shadow_block),
    ];
    let mut out: Vec<(&'static str, &PostProgram)> =
        post.programs.iter().map(|p| ("pass", p)).collect();
    out.extend(post.shadow_comp.iter().map(|p| ("shadowcomp", p)));
    out.extend(
        slots
            .into_iter()
            .filter_map(|(slot, p)| Some((slot, p.as_ref()?))),
    );
    out
}

/// naga's error for `module`, validated as the translator validates (every check, every
/// capability), with its sources; `None` when it validates.
pub fn validate(module: &naga::Module) -> Option<String> {
    let flags = naga::valid::ValidationFlags::all();
    let caps = naga::valid::Capabilities::all();
    let error = naga::valid::Validator::new(flags, caps)
        .validate(module)
        .err()?;
    let mut text = error.as_inner().to_string();
    let mut source = std::error::Error::source(error.as_inner());
    while let Some(e) = source {
        let _ = write!(text, ": {e}");
        source = e.source();
    }
    Some(text)
}

/// Hashes what `Debug` writes, without holding the text.
struct HashWriter(std::hash::DefaultHasher);

impl std::fmt::Write for HashWriter {
    fn write_str(&mut self, s: &str) -> std::fmt::Result {
        self.0.write(s.as_bytes());
        Ok(())
    }
}

fn hash(value: &dyn std::fmt::Debug) -> u64 {
    let mut w = HashWriter(std::hash::DefaultHasher::new());
    let _ = write!(w, "{value:?}");
    w.0.finish()
}

/// The compute interface and dispatch settings that determine an observed compute program.
fn compute_observation(c: &sunrise_shaders::ComputeProgram) -> impl std::fmt::Debug + '_ {
    (
        &c.interface,
        c.work_groups,
        c.work_groups_render,
        c.indirect,
    )
}

pub fn observe(
    post: &Post,
    notes: Vec<String>,
    refusals: Vec<String>,
    stage_files: Vec<String>,
    stage_sizes: BTreeMap<String, usize>,
) -> Observed {
    let mut seen = Observed {
        folder: post.folder.clone(),
        stage_files,
        refusals,
        shadow_pass: post.shadow_pass,
        custom: post.custom.clone(),
        stage_sizes,
        notes,
        ..Default::default()
    };
    seen.pieces.push((
        "render settings".into(),
        hash(&(
            &post.folder,
            &post.deferred_pre,
            &post.composite_pre,
            post.terrain_undrawn,
            post.water_undrawn,
            post.clouds_off,
            &post.sky_switches,
            post.shadow_pass,
            post.shadow_only_player,
            post.shadow_only_lit_blocks,
            post.translucent_late,
            post.particles_late,
        )),
    ));
    seen.pieces.push((
        "render resources".into(),
        hash(&(
            &post.textures,
            &post.sizes,
            post.shadow_colors,
            &post.buffer_objects,
            &post.images,
        )),
    ));
    seen.pieces.push((
        "render configuration".into(),
        hash(&(&post.shadow_culling, &post.config)),
    ));
    seen.pieces.push(("notes".into(), hash(&seen.notes)));
    for (slot, p) in programs(post) {
        if !seen.loaded.contains(&p.name) {
            seen.loaded.push(p.name.clone());
        }
        let label = format!("{slot} {}", p.name);
        let parts = (
            &p.buffers,
            &p.flips,
            &p.blend,
            &p.buffer_blends,
            p.mipmaps,
            p.group,
            p.vanilla_sky_blends,
            p.geometry_triangles,
            p.scale,
        );
        seen.pieces
            .push((format!("{label} settings"), hash(&parts)));
        let mut modules: Vec<(String, &naga::Module)> = Vec::new();
        if let Some(program) = &p.program {
            modules.push((format!("{label} vertex"), &program.vertex));
            modules.push((format!("{label} fragment"), &program.fragment));
            seen.pieces
                .push((format!("{label} interface"), hash(&program.interface)));
        }
        for (name, c) in &p.compute {
            if !seen.compute_loaded.contains(name) {
                seen.compute_loaded.push(name.clone());
            }
            modules.push((format!("{label} compute {name}"), &c.module));
            seen.pieces.push((
                format!("{label} compute {name} interface"),
                hash(&compute_observation(c)),
            ));
            seen.pieces
                .push((format!("{label} compute {name} notes"), hash(&c.notes)));
        }
        for (what, module) in modules {
            seen.pieces.push((what.clone(), hash(module)));
            if let Some(error) = validate(module) {
                seen.invalid.push((what, error));
            }
        }
    }
    seen
}

fn stage_owner(path: &str) -> Option<String> {
    let filename = path.rsplit('/').next()?;
    let (stem, extension) = filename.rsplit_once('.')?;
    if !["vsh", "fsh", "gsh", "csh", "tcs", "tes"].contains(&extension) {
        return None;
    }
    Some(if extension == "csh" {
        crate::cases::compute_stage_owner(stem).to_owned()
    } else {
        stem.to_owned()
    })
}

fn names_program(note: &str, program: &str, stage_files: &[String]) -> bool {
    if note
        .strip_prefix(program)
        .is_some_and(|rest| rest.starts_with(": "))
    {
        return true;
    }
    stage_files.iter().any(|path| {
        if stage_owner(path).as_deref() != Some(program) {
            return false;
        }
        let filename = path.rsplit('/').next().unwrap_or(path);
        let stem = filename.rsplit_once('.').map_or(filename, |(stem, _)| stem);
        note.strip_prefix(path)
            .or_else(|| note.strip_prefix(filename))
            .is_some_and(valid_location)
            || note
                .strip_prefix(stem)
                .is_some_and(|rest| rest.starts_with(": "))
    })
}

fn valid_location(rest: &str) -> bool {
    let Some(rest) = rest.strip_prefix(':') else {
        return false;
    };
    let Some((line, suffix)) = rest.split_once(": ") else {
        return false;
    };
    !suffix.is_empty() && line.parse::<u32>().is_ok()
}

/// A diagnostic names a subject only when it identifies a loaded program or one of this
/// candidate's active stage files. Property keys, feature names and arbitrary prefixes do not.
pub fn named(note: &str, loaded: &[String], stage_files: &[String]) -> bool {
    loaded
        .iter()
        .any(|program| names_program(note, program, stage_files))
        || stage_files.iter().any(|path| {
            stage_owner(path).is_some_and(|program| names_program(note, &program, stage_files))
        })
}

/// Text that groups alike: numbers (handles, lines, sizes) read `#`, and at most 120 chars.
pub fn shape(text: &str) -> String {
    let mut out = String::new();
    let mut digits = false;
    for c in text.chars() {
        if c.is_ascii_digit() {
            if !digits {
                out.push('#');
            }
            digits = true;
        } else {
            digits = false;
            out.push(c);
        }
        if out.len() >= 120 {
            break;
        }
    }
    out
}

/// The invariants over two loads of the same case (each a panic message when it panicked).
pub fn judge(
    first: Result<Observed, String>,
    second: Result<Observed, String>,
    request: &Request,
) -> Outcome {
    let mut failures = Vec::new();
    for (label, result) in [("first", &first), ("second", &second)] {
        match result {
            Err(panic) => failures.push(Failure::new("panic", format!("{label} load: {panic}"))),
            Ok(observed) => {
                for note in &observed.ungrounded_diagnostics {
                    failures.push(Failure::new(
                        "diagnostic subject",
                        format!("unmatched note: {}", shape(note)),
                    ));
                }
                for (what, error) in &observed.invalid {
                    failures.push(Failure::new("naga", format!("{what}: {}", shape(error))));
                }
                for note in observed
                    .refusals
                    .iter()
                    .filter(|note| !named(note, &observed.loaded, &observed.stage_files))
                {
                    failures.push(Failure::new(
                        "refusal",
                        format!("unnamed note: {}", shape(note)),
                    ));
                }
                for name in &request.expect {
                    let gone = !observed.loaded.contains(name);
                    let explained = observed.loader_refusals.iter().any(|refusal| {
                        matches!(refusal, sunrise_shaders::LoaderRefusal::Program(refused) if refused == name)
                    });
                    // The shadow pass may be off now: its programs then don't load, by design.
                    let shadow = name.starts_with("shadow") && !observed.shadow_pass;
                    if gone && !explained && !shadow {
                        failures.push(Failure::new(
                            "refusal",
                            format!("{name}: not loaded, no note"),
                        ));
                    }
                }
                for stage in &request.expect_compute {
                    let owner = crate::cases::compute_stage_owner(stage);
                    if crate::cases::is_shadowcomp_program(owner) && !observed.shadow_pass {
                        continue;
                    }
                    if observed.compute_loaded.contains(stage) {
                        continue;
                    }
                    let stage_path = if observed.folder.is_empty() {
                        format!("{stage}.csh")
                    } else {
                        format!("{}/{stage}.csh", observed.folder)
                    };
                    let stage_refused = observed.loader_refusals.iter().any(|refusal| {
                        matches!(refusal, sunrise_shaders::LoaderRefusal::ComputeStage(path)
                            if path == &stage_path)
                    });
                    let attempted = observed
                        .compute_stage_attempts
                        .iter()
                        .any(|attempt| attempt.name == *stage && attempt.path == stage_path);
                    let parent_refused = !attempted
                        && !observed.loaded.iter().any(|loaded| loaded == owner)
                        && observed.loader_refusals.iter().any(|refusal| {
                            matches!(refusal, sunrise_shaders::LoaderRefusal::Program(refused) if refused == owner)
                        });
                    if !stage_refused && !parent_refused {
                        failures.push(Failure::new(
                            "refusal",
                            format!("{stage}: compute stage not loaded, no note"),
                        ));
                    }
                }
            }
        }
    }
    if let (Ok(first), Ok(second)) = (&first, &second) {
        let differs = first
            .pieces
            .iter()
            .zip(&second.pieces)
            .find(|(a, b)| a != b)
            .map(|(a, _)| a.0.clone())
            .or_else(|| (first.pieces.len() != second.pieces.len()).then(|| "program list".into()))
            .or_else(|| (first.loaded != second.loaded).then(|| "loaded programs".into()))
            .or_else(|| {
                (first.compute_loaded != second.compute_loaded)
                    .then(|| "loaded compute stages".into())
            })
            .or_else(|| (first.folder != second.folder).then(|| "dimension folder".into()))
            .or_else(|| {
                (first.stage_files != second.stage_files).then(|| "active stage files".into())
            })
            .or_else(|| (first.shadow_pass != second.shadow_pass).then(|| "shadow pass".into()))
            .or_else(|| (first.custom != second.custom).then(|| "custom definitions".into()))
            .or_else(|| {
                (first.stage_sizes != second.stage_sizes).then(|| "preprocessed stage sizes".into())
            });
        if let Some(piece) = differs {
            failures.push(Failure::new("determinism", piece));
        }
    }
    failures.sort();
    failures.dedup();
    Outcome {
        failures,
        loaded: first
            .as_ref()
            .ok()
            .map_or_else(Vec::new, |o| o.loaded.clone()),
        compute_loaded: first
            .as_ref()
            .ok()
            .map_or_else(Vec::new, |o| o.compute_loaded.clone()),
        shadow_pass: first.as_ref().ok().is_some_and(|o| o.shadow_pass),
    }
}

thread_local! {
    /// Where the last panic on this thread happened, set by the worker's hook.
    pub static PANIC_AT: RefCell<Option<String>> = const { RefCell::new(None) };
}

fn panic_text(payload: Box<dyn std::any::Any + Send>) -> String {
    if let Some(at) = PANIC_AT.with(|p| p.borrow_mut().take()) {
        return at;
    }
    let message = payload
        .downcast_ref::<&str>()
        .map(|s| (*s).to_owned())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_default();
    shape(&message)
}

/// The stage files' preprocessed sizes, including stages from disabled and rejected programs;
/// failures state the measured size and limit.
pub fn oversized(stage_sizes: &BTreeMap<String, usize>, limit: usize) -> Vec<Failure> {
    stage_sizes
        .iter()
        .filter(|(_, size)| **size > limit)
        .map(|(file, size)| {
            Failure::new(
                "size",
                format!("{file}: {size} bytes after preprocessing exceeds {limit} bytes"),
            )
        })
        .collect()
}

fn oversized_loads(
    first: &Result<Observed, String>,
    second: &Result<Observed, String>,
    limit: usize,
) -> Vec<Failure> {
    [&first, &second]
        .into_iter()
        .filter_map(|result| result.as_ref().ok())
        .flat_map(|observed| oversized(&observed.stage_sizes, limit))
        .collect()
}

/// The case in `dir` checked: no panic, every program valid, every refusal named, the same
/// output twice, and every active-folder stage within the size limit. Time and memory bounds
/// are the worker's.
pub fn check(dir: &Path, request: &Request) -> Outcome {
    let once = || catch_unwind(AssertUnwindSafe(|| load(dir, request))).map_err(panic_text);
    let first = once();
    let second = once();
    let mut size_failures = oversized_loads(&first, &second, request.size_limit);
    let mut outcome = judge(first, second, request);
    outcome.failures.append(&mut size_failures);
    outcome.failures.sort();
    outcome.failures.dedup();
    outcome
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(expect: &[&str]) -> Request {
        Request {
            dim: 0,
            flat: true,
            expect: expect.iter().map(|s| (*s).to_owned()).collect(),
            expect_compute: Vec::new(),
            size_limit: 1 << 24,
        }
    }

    fn seen(loaded: &[&str], notes: &[&str]) -> Observed {
        Observed {
            loaded: loaded.iter().map(|s| (*s).to_owned()).collect(),
            compute_loaded: Vec::new(),
            folder: String::new(),
            stage_files: [
                "composite.vsh",
                "composite.fsh",
                "final.vsh",
                "final.fsh",
                "shadow.vsh",
                "shadow.fsh",
            ]
            .map(str::to_owned)
            .to_vec(),
            notes: notes.iter().map(|s| (*s).to_owned()).collect(),
            refusals: notes
                .iter()
                .filter(|note| note.ends_with("skipped"))
                .map(|s| (*s).to_owned())
                .collect(),
            loader_refusals: notes
                .iter()
                .filter_map(|note| note.strip_suffix(": skipped"))
                .map(|name| {
                    if name.contains('_') {
                        sunrise_shaders::LoaderRefusal::ComputeStage(format!("{name}.csh"))
                    } else {
                        sunrise_shaders::LoaderRefusal::Program(name.to_owned())
                    }
                })
                .collect(),
            compute_stage_attempts: Vec::new(),
            ungrounded_diagnostics: Vec::new(),
            shadow_pass: false,
            custom: Default::default(),
            stage_sizes: BTreeMap::new(),
            invalid: Vec::new(),
            pieces: vec![("composite fragment".into(), 7)],
        }
    }

    #[test]
    fn notes_name_their_subject() {
        let loaded = ["composite".to_owned()];
        let stages = [
            "composite.vsh".to_owned(),
            "composite.fsh".to_owned(),
            "composite1_a.csh".to_owned(),
        ];
        for ok in [
            "composite: skipped",
            "composite1_a: skipped",
            "composite1_a.csh:12: unknown directive `#foo`",
        ] {
            assert!(named(ok, &loaded, &stages), "{ok}");
        }
        for unnamed in [
            "skipped",
            ": skipped",
            " composite: x",
            "arbitrary-prefix: refused",
            "arbitrary-prefix: skipped",
            "iris.features.required: unknown feature",
            "shaders.properties:7: not a key=value line",
            "uniform.float.fzA: expression did not parse",
            "`fzA` reads 0: unknown",
            "no program for translucent terrain; nothing draws it",
        ] {
            assert!(!named(unnamed, &loaded, &stages), "{unnamed}");
        }
    }

    #[test]
    fn a_panic_is_the_only_failure_of_its_load() {
        let out = judge(
            Err("post.rs:1".into()),
            Ok(seen(&["final"], &[])),
            &request(&["final"]),
        );
        assert_eq!(
            out.failures,
            [Failure::new("panic", "first load: post.rs:1")]
        );
        let out = judge(
            Ok(seen(&["final"], &[])),
            Err("post.rs:1".into()),
            &request(&["final"]),
        );
        assert_eq!(
            out.failures,
            [Failure::new("panic", "second load: post.rs:1")]
        );
    }

    #[test]
    fn a_program_dropped_without_a_note_is_an_unnamed_refusal() {
        let req = request(&["composite", "final", "shadow"]);
        let out = judge(
            Ok(seen(&["deferred"], &["final: skipped"])),
            Ok(seen(&["deferred"], &["final: skipped"])),
            &req,
        );
        // `final` is named and the shadow pass is off: only `composite` went silently.
        assert_eq!(
            out.failures,
            [Failure::new("refusal", "composite: not loaded, no note")]
        );
        let out = judge(
            Ok(seen(&[], &["skipped"])),
            Ok(seen(&[], &["skipped"])),
            &request(&[]),
        );
        assert_eq!(
            out.failures,
            [Failure::new("refusal", "unnamed note: skipped")]
        );
    }

    #[test]
    fn feature_and_parser_diagnostics_do_not_explain_a_silent_drop() {
        let notes = [
            "iris.features.required: unknown feature",
            "shaders.properties:7: not a key=value line",
            "uniform.float.fzA: expression did not parse",
        ];
        let out = judge(
            Ok(seen(&[], &notes)),
            Ok(seen(&[], &notes)),
            &request(&["composite"]),
        );
        assert_eq!(
            out.failures,
            [Failure::new("refusal", "composite: not loaded, no note")]
        );
    }

    #[test]
    fn diagnostics_are_tied_to_real_property_and_source_subjects() {
        let properties = sunrise_shaders::Properties::parse(
            "iris.features.required = MISSING_FEATURE\nimage.bad = not-an-image\nuniform.float.fzA = unknown_name + 1\n",
            &Macros::standard(),
        );
        let property_keys: BTreeMap<&str, &str> = properties
            .entries
            .iter()
            .map(|entry| (entry.key.as_str(), entry.value.as_str()))
            .collect();
        let sources = BTreeMap::from([
            ("composite.fsh".to_owned(), "void main() {}\n".to_owned()),
            ("shaders.settings".to_owned(), "#define FOO\n".to_owned()),
        ]);
        assert!(feature_note_has_subject(
            "the pack requires `MISSING_FEATURE`, which is not supported yet",
            &properties
        ));
        for note in [
            "composite.fsh:1: bad token",
            "composite.fsh:1:3: bad token",
            "composite.fsh: bad token",
            "composite.fsh:bad token",
            "composite.fsh:0: bad token",
            "composite.fsh:0:3: bad token",
            "shaders.settings:1: bad setting",
        ] {
            assert!(located_note_has_subject(note, &sources), "{note}");
        }
        assert!(shader_text_source("shaders.settings"));
        assert!(!located_note_has_subject(
            "composite.fsh:2: bad token",
            &sources
        ));
        assert!(!located_note_has_subject("composite.fsh:1:", &sources));
        assert!(post_note_has_subject(
            "`image.bad`: not an image",
            &property_keys,
            &sources,
            &[],
            &[],
            false,
            false,
            false
        ));
        assert!(custom_note_has_subject(
            "`unknown_name` is unknown; it reads 0",
            &property_keys
        ));
        assert!(!located_note_has_subject(
            "missing.fsh:1: bad token",
            &sources
        ));
        assert!(!post_note_has_subject(
            "`image.typo`: not an image",
            &property_keys,
            &sources,
            &[],
            &[],
            false,
            false,
            false
        ));
        assert!(!custom_note_has_subject(
            "`not_in_pack` is unknown; it reads 0",
            &property_keys
        ));
    }

    #[test]
    fn a_named_skip_explains_the_expected_program() {
        let out = judge(
            Ok(seen(&[], &["composite: skipped"])),
            Ok(seen(&[], &["composite: skipped"])),
            &request(&["composite"]),
        );
        assert!(out.failures.is_empty(), "{:?}", out.failures);
    }

    #[test]
    fn a_compute_sibling_refusal_does_not_explain_another_stage() {
        let mut observed = seen(&["composite"], &[]);
        observed.loader_refusals = vec![sunrise_shaders::LoaderRefusal::ComputeStage(
            "composite1_b.csh".to_owned(),
        )];
        let mut request = request(&[]);
        request.expect_compute = vec!["composite1_a".to_owned()];
        let out = judge(Ok(observed.clone()), Ok(observed), &request);
        assert_eq!(
            out.failures,
            [Failure::new(
                "refusal",
                "composite1_a: compute stage not loaded, no note"
            )]
        );
    }

    #[test]
    fn the_second_load_is_also_checked_against_the_size_limit() {
        let first = Ok(Observed::default());
        let second = Ok(Observed {
            stage_sizes: [("composite.fsh".to_owned(), 17)].into(),
            ..Default::default()
        });
        assert_eq!(
            oversized_loads(&first, &second, 16),
            [Failure::new(
                "size",
                "composite.fsh: 17 bytes after preprocessing exceeds 16 bytes"
            )]
        );

        let smaller = oversized(&[("composite.fsh".to_owned(), 17)].into(), 16).remove(0);
        let larger = oversized(&[("composite.fsh".to_owned(), 23)].into(), 16).remove(0);
        assert_ne!(smaller.detail, larger.detail);
        assert_eq!(smaller.signature(), larger.signature());
        let another_stage = oversized(&[("final.fsh".to_owned(), 17)].into(), 16).remove(0);
        assert_ne!(smaller.signature(), another_stage.signature());
    }

    #[test]
    fn indirect_compute_dispatch_is_part_of_observed_output() {
        let compute = |indirect| sunrise_shaders::ComputeProgram {
            module: naga::Module::default(),
            interface: Default::default(),
            notes: Vec::new(),
            work_groups: None,
            work_groups_render: [1.0, 1.0],
            indirect,
        };
        let direct = compute(None);
        let indirect = compute(Some(sunrise_shaders::Indirect {
            buffer: 2,
            offset: 16,
        }));
        assert_ne!(
            hash(&compute_observation(&direct)),
            hash(&compute_observation(&indirect))
        );
    }

    #[test]
    fn two_loads_that_differ_name_the_first_part() {
        let mut other = seen(&["composite"], &[]);
        other.pieces[0].1 = 8;
        let out = judge(Ok(seen(&["composite"], &[])), Ok(other), &request(&[]));
        assert_eq!(
            out.failures,
            [Failure::new("determinism", "composite fragment")]
        );
        let out = judge(
            Ok(seen(&["composite"], &[])),
            Err("x".into()),
            &request(&[]),
        );
        assert_eq!(out.failures, [Failure::new("panic", "second load: x")]);
        let mut shadow = seen(&["composite"], &[]);
        shadow.shadow_pass = true;
        let out = judge(Ok(seen(&["composite"], &[])), Ok(shadow), &request(&[]));
        assert_eq!(out.failures, [Failure::new("determinism", "shadow pass")]);
    }

    #[test]
    fn render_driving_load_results_are_compared() {
        fn loaded(properties: &str) -> Observed {
            let nonce = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let dir = std::env::temp_dir().join(format!(
                "sunrise-shader-fuzz-{}-{nonce}",
                std::process::id()
            ));
            std::fs::create_dir_all(&dir).unwrap();
            let files = BTreeMap::from([
                (
                    "composite.vsh".to_owned(),
                    b"#version 120\nvoid main() { gl_Position = gl_Vertex; }".to_vec(),
                ),
                (
                    "composite.fsh".to_owned(),
                    b"#version 120\nvoid main() { gl_FragColor = vec4(1.0); }".to_vec(),
                ),
                (
                    "shaders.properties".to_owned(),
                    properties.as_bytes().to_vec(),
                ),
            ]);
            crate::write_pack(&dir, &files).unwrap();
            let seen = load(&dir, &request(&[]));
            std::fs::remove_dir_all(dir).unwrap();
            seen
        }

        let base = loaded("");
        for (property, expected) in [
            ("size.buffer.colortex0=128 128", "render resources"),
            ("bufferObject.0=16", "render resources"),
            ("shadow.enabled=true", "render settings"),
            ("uniform.float.fzAudit=1.0", "custom definitions"),
        ] {
            let changed = loaded(property);
            let outcome = judge(Ok(base.clone()), Ok(changed), &request(&[]));
            assert_eq!(
                outcome.failures,
                [Failure::new("determinism", expected)],
                "{property}"
            );
        }
    }

    #[test]
    fn an_authored_shader_diagnostic_is_grounded_by_its_file_and_line() {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "sunrise-shader-fuzz-diagnostic-{}-{nonce}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let files = BTreeMap::from([
            (
                "composite.vsh".to_owned(),
                b"#version 120\nvoid main() { gl_Position = gl_Vertex; }".to_vec(),
            ),
            (
                "composite.fsh".to_owned(),
                b"#version 120\n#include \"lib/format.settings\"\nvoid main() { gl_FragColor = vec4(1.0); }".to_vec(),
            ),
            (
                "lib/format.settings".to_owned(),
                b"const int colortex0Format = XYZ;\n".to_vec(),
            ),
        ]);
        crate::write_pack(&dir, &files).unwrap();

        let observed = load(&dir, &request(&[]));
        assert!(
            observed
                .notes
                .iter()
                .any(|note| note.starts_with("lib/format.settings:1:")),
            "expected the loader's authored format diagnostic, got {:?}",
            observed.notes
        );
        assert!(
            observed.ungrounded_diagnostics.is_empty(),
            "authored diagnostic was misclassified: {:?}",
            observed.ungrounded_diagnostics
        );

        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn authored_refusals_and_file_level_warnings_pass_attribution() {
        let load_fragment = |fragment: &[u8]| {
            let nonce = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let dir = std::env::temp_dir().join(format!(
                "sunrise-shader-fuzz-authored-note-{}-{nonce}",
                std::process::id()
            ));
            std::fs::create_dir_all(&dir).unwrap();
            let files = BTreeMap::from([
                (
                    "composite.vsh".to_owned(),
                    b"#version 120\nvoid main() { gl_Position = gl_Vertex; }".to_vec(),
                ),
                ("composite.fsh".to_owned(), fragment.to_vec()),
            ]);
            crate::write_pack(&dir, &files).unwrap();
            let observed = load(&dir, &request(&[]));
            std::fs::remove_dir_all(dir).unwrap();
            observed
        };

        let refused = load_fragment(b"#version 120\nvoid main() { gl_FragColor = vec4(1.0) }\n");
        assert!(
            refused
                .notes
                .iter()
                .any(|note| note.starts_with("composite.fsh:2:")),
            "expected a source-located compiler diagnostic, got {:?}",
            refused.notes
        );
        assert!(
            refused
                .refusals
                .iter()
                .any(|note| note == "composite: skipped"),
            "expected the invalid program to be refused, got {:?}",
            refused.notes
        );
        assert!(
            refused.ungrounded_diagnostics.is_empty(),
            "compiler refusal was misclassified: {:?}",
            refused.ungrounded_diagnostics
        );

        let warned =
            load_fragment(b"#version 120\nvoid main() { gl_FragColor = vec4(1.0); }\n#if 1\n");
        assert!(
            warned
                .notes
                .iter()
                .any(|note| note.starts_with("composite.fsh:0:")),
            "expected the authored file-level warning, got {:?}",
            warned.notes
        );
        assert!(
            warned.ungrounded_diagnostics.is_empty(),
            "file-level warning was misclassified: {:?}",
            warned.ungrounded_diagnostics
        );
    }

    #[test]
    fn an_invalid_module_fails_validation_by_program() {
        // A uniform without a binding.
        let mut module = naga::Module::default();
        let ty = module.types.insert(
            naga::Type {
                name: None,
                inner: naga::TypeInner::Scalar(naga::Scalar::F32),
            },
            naga::Span::UNDEFINED,
        );
        module.global_variables.append(
            naga::GlobalVariable {
                name: Some("x".into()),
                space: naga::AddressSpace::Uniform,
                binding: None,
                ty,
                init: None,
                memory_decorations: Default::default(),
            },
            naga::Span::UNDEFINED,
        );
        let error = validate(&module).expect("a uniform needs a binding");
        assert_eq!(validate(&naga::Module::default()), None);
        let mut bad = seen(&["composite"], &[]);
        bad.invalid
            .push(("pass composite fragment".into(), error.clone()));
        let out = judge(Ok(bad.clone()), Ok(bad), &request(&[]));
        assert_eq!(
            out.failures,
            [Failure::new(
                "naga",
                format!("pass composite fragment: {}", shape(&error))
            )]
        );
    }

    #[test]
    fn validation_errors_on_the_second_load_are_still_naga_failures() {
        let mut bad = seen(&["composite"], &[]);
        bad.invalid
            .push(("pass composite fragment".into(), "invalid".into()));
        let good = seen(&["composite"], &[]);
        let second = judge(Ok(good.clone()), Ok(bad.clone()), &request(&[]));
        let first = judge(Ok(bad), Ok(good), &request(&[]));
        assert_eq!(first.failures, second.failures);
        assert_eq!(
            first.failures,
            [Failure::new("naga", "pass composite fragment: invalid")]
        );
    }

    #[test]
    fn custom_definitions_are_compared_as_loaded_state() {
        let macros = Macros::default();
        let first_properties =
            sunrise_shaders::Properties::parse("uniform.float.answer = 1.0\n", &macros);
        let second_properties =
            sunrise_shaders::Properties::parse("uniform.float.answer = 2.0\n", &macros);
        let mut first = seen(&["composite"], &[]);
        first.custom = sunrise_shaders::Custom::parse(&first_properties);
        let mut second = seen(&["composite"], &[]);
        second.custom = sunrise_shaders::Custom::parse(&second_properties);
        let out = judge(Ok(first), Ok(second), &request(&[]));
        assert_eq!(
            out.failures,
            [Failure::new("determinism", "custom definitions")]
        );
    }

    #[test]
    fn numbers_do_not_split_groups() {
        assert_eq!(shape("Function [12] at 3:4"), "Function [#] at #:#");
    }
}
