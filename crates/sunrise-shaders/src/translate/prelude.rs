//! Mod-supplied GLSL preludes for shader-pack programs (`PackBinding::{vertex_prelude,
//! fragment_prelude}`).
//!
//! A prelude declares optional vertex inputs (from the mod's `VertexFormat` by location),
//! mod bind-group resources (`set = 0` and `set = 1`, mapped to `MOD_SET_BASE` = 6 and
//! `MOD_SET_BASE + 1` = 7), globals, varyings, loose pack uniforms/samplers, constants, and
//! helper functions. A vertex prelude must define `void sunrise_pack_vertex_init()`, which the
//! engine calls first in the pack's vertex `main`; a fragment prelude must not define it.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fmt::Write;

use naga::ShaderStage;

use super::scan::{
    IMAGE_FORMATS, QUALIFIERS, STORAGE, Stage, UNIFORM_STATE, VERTEX_INPUTS, Var, is_ident,
    is_sampler_type, significant,
};
use super::{Error, Sampler, split_sampler};
use crate::macros::lex;
use crate::reserved::{is_valid_glsl_identifier, reserved_prelude_identifier};

/// First bind-group index of a mod's own sets in a pack pass, after the pack's own sets
/// `0..=5` (the frame's, the geometry's, the draws', the block ids', the storage buffers', and
/// the geometry stage's indices). Prelude `set = 0` maps to `MOD_SET_BASE` (6) and prelude
/// `set = 1` maps to `MOD_SET_BASE + 1` (7).
pub const MOD_SET_BASE: u32 = 6;

/// Name of the required vertex prelude entry point called before the pack's vertex `main`.
pub const VERTEX_INIT_FN: &str = "sunrise_pack_vertex_init";

/// Optional GLSL preludes attached to a mod pipeline's shader-pack binding.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PackPrelude {
    /// GLSL prepended into the matched pack program's vertex stage; must define
    /// `void sunrise_pack_vertex_init()`.
    pub vertex: Option<String>,
    /// GLSL prepended into the matched pack program's fragment stage; must not define
    /// `sunrise_pack_vertex_init`.
    pub fragment: Option<String>,
}

impl PackPrelude {
    pub fn is_empty(&self) -> bool {
        self.vertex.is_none() && self.fragment.is_none()
    }
}

/// Which shader stages may access one binding in a mod pipeline's bind-group layout.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PreludeStages {
    pub vertex: bool,
    pub fragment: bool,
}

impl PreludeStages {
    pub const VERTEX_FRAGMENT: Self = Self {
        vertex: true,
        fragment: true,
    };
    pub const VERTEX: Self = Self {
        vertex: true,
        fragment: false,
    };
    pub const FRAGMENT: Self = Self {
        vertex: false,
        fragment: true,
    };

    fn allows(self, stage: ShaderStage) -> bool {
        match stage {
            ShaderStage::Vertex => self.vertex,
            ShaderStage::Fragment => self.fragment,
            _ => false,
        }
    }
}

/// Storage-image access mode of one mod set-1 binding.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PreludeImageAccess {
    ReadOnly,
    WriteOnly,
    ReadWrite,
}

/// Resource kind of one binding in a mod pipeline's set 0 or set 1.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PreludeResourceKind {
    UniformBuffer,
    Texture2D,
    Sampler,
    /// Combined 2D texture at `binding` + sampler at `binding + 1`.
    CombinedSampler2D,
    StorageBuffer {
        read_only: bool,
    },
    StorageImage {
        access: PreludeImageAccess,
    },
}

/// One resource binding available to a prelude at `(set, binding)`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PreludeBinding {
    pub set: u32,
    pub binding: u32,
    pub kind: PreludeResourceKind,
    pub stages: PreludeStages,
}

/// One vertex input attribute from a mod pipeline's `VertexFormat`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PreludeVertexInput<'a> {
    pub location: u32,
    pub glsl_type: &'a str,
}

/// Pipeline interface against which a [`PackPrelude`] is validated at mod registration.
#[derive(Clone, Copy, Debug, Default)]
pub struct PreludeLayout<'a> {
    /// Vertex attributes from `effective_vertex_bindings()` in location order.
    pub vertex_inputs: &'a [PreludeVertexInput<'a>],
    /// Available resources in mod set 0 and mod set 1.
    pub bindings: &'a [PreludeBinding],
}

/// Typed refusal returned when validating a [`PackPrelude`] at mod registration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PreludeError {
    /// GLSL parse or validation failure on the prelude alone.
    Parse {
        stage: &'static str,
        message: String,
    },
    /// A `#...` preprocessor directive appeared inside the prelude.
    UnsupportedDirective {
        stage: &'static str,
        line: usize,
        directive: String,
    },
    /// A declared name is not a valid GLSL identifier.
    InvalidIdentifier { stage: &'static str, name: String },
    /// A declared name collides with a reserved pack, implicit-input, or engine-injected name.
    ReservedIdentifier {
        stage: &'static str,
        name: String,
        kind: &'static str,
    },
    /// Vertex prelude did not define `void sunrise_pack_vertex_init()`.
    MissingVertexInit,
    /// `sunrise_pack_vertex_init` had a bad signature, appeared more than once, or appeared in
    /// `fragment_prelude`.
    InvalidVertexInit { stage: &'static str, reason: String },
    /// A mod resource binding had an invalid set, omitted `set`, or named an unknown/mismatched
    /// binding in the pipeline's layout.
    InvalidBinding {
        stage: &'static str,
        message: String,
    },
    /// A prelude declared a resource in a stage where the pipeline's layout does not expose it.
    StageVisibility {
        stage: &'static str,
        set: u32,
        binding: u32,
    },
    /// A vertex prelude `in` declaration lacked `layout(location = N)`, reused a location, or
    /// disagreed with the pipeline's `VertexFormat`.
    InvalidVertexInput { message: String },
    /// A prelude varying declared an explicit `layout(location = ...)`.
    ExplicitVaryingLocation { stage: &'static str, name: String },
    /// A fragment prelude declared an `out` variable (the pack owns draw-buffer outputs).
    FragmentOutput { name: String },
}

impl std::fmt::Display for PreludeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Parse { stage, message } => {
                write!(f, "{stage} prelude failed to parse: {message}")
            }
            Self::UnsupportedDirective {
                stage,
                line,
                directive,
            } => write!(
                f,
                "{stage} prelude line {line}: preprocessor directive `{directive}` is not allowed (omit `#version` and use `PackBinding::define` for macros)"
            ),
            Self::InvalidIdentifier { stage, name } => {
                write!(f, "{stage} prelude: invalid GLSL identifier `{name}`")
            }
            Self::ReservedIdentifier { stage, name, kind } => {
                write!(
                    f,
                    "{stage} prelude: identifier `{name}` collides with a {kind}"
                )
            }
            Self::MissingVertexInit => {
                write!(f, "vertex prelude must define `void {VERTEX_INIT_FN}()`")
            }
            Self::InvalidVertexInit { stage, reason } => {
                write!(f, "{stage} prelude: `{VERTEX_INIT_FN}` {reason}")
            }
            Self::InvalidBinding { stage, message } => {
                write!(f, "{stage} prelude: {message}")
            }
            Self::StageVisibility {
                stage,
                set,
                binding,
            } => write!(
                f,
                "{stage} prelude: resource `(set = {set}, binding = {binding})` is not visible in the {stage} stage"
            ),
            Self::InvalidVertexInput { message } => {
                write!(f, "vertex prelude: {message}")
            }
            Self::ExplicitVaryingLocation { stage, name } => write!(
                f,
                "{stage} prelude: varying `{name}` must not declare `layout(location = ...)` (prelude varyings link by name)"
            ),
            Self::FragmentOutput { name } => write!(
                f,
                "fragment prelude: output `{name}` is not allowed (the shader pack owns draw-buffer outputs)"
            ),
        }
    }
}

impl std::error::Error for PreludeError {}

/// Validate a [`PackPrelude`] on its own against `layout` when a mod registers its pipeline.
pub fn validate_pack_prelude(
    prelude: &PackPrelude,
    layout: &PreludeLayout<'_>,
) -> Result<(), PreludeError> {
    if let Some(vertex) = &prelude.vertex {
        let parsed = ParsedStagePrelude::parse(vertex, ShaderStage::Vertex)?;
        parsed.validate_against_layout(layout)?;
        parsed.validate_standalone()?;
    }
    if let Some(fragment) = &prelude.fragment {
        let parsed = ParsedStagePrelude::parse(fragment, ShaderStage::Fragment)?;
        parsed.validate_against_layout(layout)?;
        parsed.validate_standalone()?;
    }
    Ok(())
}

#[derive(Clone, Debug)]
enum ParsedModResourceKind {
    UniformBlock,
    StorageBuffer { read_only: bool },
    Texture2D,
    Sampler,
    CombinedSampler { ty: String },
    StorageImage { access: PreludeImageAccess },
}

#[derive(Clone, Debug)]
enum PreludeItem {
    Struct {
        name: String,
        vars: Vec<String>,
        raw: String,
    },
    VertexInput {
        location: u32,
        ty: String,
        name: String,
        raw: String,
    },
    Varying {
        name: String,
        ty: String,
        decl_at_name: String,
    },
    LooseUniform {
        name: String,
        ty: String,
    },
    PackSampler {
        name: String,
        ty: String,
    },
    ModResource {
        set: u32,
        binding: u32,
        kind: ParsedModResourceKind,
        primary_name: String,
        declared_names: Vec<String>,
        raw: String,
    },
    Global {
        name: String,
        ty: String,
        raw: String,
    },
    Function {
        name: String,
        is_vertex_init: bool,
        raw: String,
    },
}

pub(super) struct ParsedStagePrelude {
    stage: ShaderStage,
    items: Vec<PreludeItem>,
    uses_frag_coord: bool,
    writes_texcoord1: bool,
    writes_texcoord2: bool,
    /// Identifiers mentioned anywhere in function bodies / global initializers (for keeping
    /// prelude-only varyings alive across stages).
    referenced: BTreeSet<String>,
}

/// Output of merging a validated stage prelude with a scanned pack stage.
pub(super) struct MergedStagePrelude {
    /// Header declarations & helper functions to append at the end of `header`.
    pub header_decls: String,
    /// Rewritten `void sunrise_pack_vertex_init() { ... }` placed after the pack's code.
    pub vertex_init_fn: Option<String>,
    /// Mod set 6/7 combined samplers so `Stage::rewrite` also splits calls inside pack code.
    pub extra_samplers: Vec<Sampler>,
    /// Pack `v.attributes` names shadowed by a matching-type `VertexInput` from the prelude.
    pub shadowed_pack_attributes: BTreeSet<String>,
    /// Whether the fragment prelude references `gl_FragCoord`.
    pub uses_frag_coord: bool,
    /// Whether the vertex prelude sets `gl_MultiTexCoord2` without `gl_MultiTexCoord1`.
    pub lightmap_from_texcoord2: bool,
}

fn stage_label(stage: ShaderStage) -> &'static str {
    match stage {
        ShaderStage::Vertex => "vertex",
        ShaderStage::Fragment => "fragment",
        _ => "compute",
    }
}

/// Strip `//` and `/* ... */` comments while preserving every `\n` for line accuracy.
fn strip_comments(source: &str, stage: &'static str) -> Result<String, PreludeError> {
    let bytes = source.as_bytes();
    let mut out = String::with_capacity(source.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'/' && bytes.get(i + 1) == Some(&b'/') {
            i += 2;
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        if bytes[i] == b'/' && bytes.get(i + 1) == Some(&b'*') {
            i += 2;
            let mut closed = false;
            while i < bytes.len() {
                if bytes[i] == b'*' && bytes.get(i + 1) == Some(&b'/') {
                    i += 2;
                    closed = true;
                    break;
                }
                if bytes[i] == b'\n' {
                    out.push('\n');
                } else {
                    out.push(' ');
                }
                i += 1;
            }
            if !closed {
                return Err(PreludeError::Parse {
                    stage,
                    message: "unterminated block comment".to_owned(),
                });
            }
            continue;
        }
        let ch = source[i..].chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
    }
    Ok(out)
}

fn check_no_directives(source: &str, stage: &'static str) -> Result<(), PreludeError> {
    for (idx, line) in source.lines().enumerate() {
        let trimmed = line.trim_start();
        if let Some(rest) = trimmed.strip_prefix('#') {
            let word = rest.split_whitespace().next().unwrap_or("");
            return Err(PreludeError::UnsupportedDirective {
                stage,
                line: idx + 1,
                directive: format!("#{word}"),
            });
        }
    }
    Ok(())
}

fn check_ident(name: &str, stage: &'static str) -> Result<(), PreludeError> {
    if !is_valid_glsl_identifier(name) {
        return Err(PreludeError::InvalidIdentifier {
            stage,
            name: name.to_owned(),
        });
    }
    if let Some(kind) = reserved_prelude_identifier(name) {
        return Err(PreludeError::ReservedIdentifier {
            stage,
            name: name.to_owned(),
            kind,
        });
    }
    Ok(())
}

#[derive(Default)]
struct LayoutQuals {
    set: Option<u32>,
    binding: Option<u32>,
    location: Option<u32>,
    image_format: Option<String>,
    has_set_key: bool,
    has_binding_key: bool,
    has_location_key: bool,
}

fn parse_layout_quals(
    tokens: &[&str],
    sig: &[usize],
    start_k: usize,
    stage: &'static str,
) -> Result<(LayoutQuals, usize), PreludeError> {
    let mut quals = LayoutQuals::default();
    let mut k = start_k;
    while k < sig.len() && tokens[sig[k]] == "layout" {
        k += 1;
        if k >= sig.len() || tokens[sig[k]] != "(" {
            return Err(PreludeError::Parse {
                stage,
                message: "expected `(` after `layout`".to_owned(),
            });
        }
        k += 1;
        let mut depth = 1usize;
        while k < sig.len() && depth > 0 {
            let w = tokens[sig[k]];
            match w {
                "(" => depth += 1,
                ")" => {
                    depth -= 1;
                    k += 1;
                    continue;
                }
                "set" | "binding" | "location" if depth == 1 => {
                    let key = w;
                    if k + 2 >= sig.len() || tokens[sig[k + 1]] != "=" {
                        return Err(PreludeError::InvalidBinding {
                            stage,
                            message: format!(
                                "expected `= <integer>` after `{key}` in `layout(...)`"
                            ),
                        });
                    }
                    let val_tok = tokens[sig[k + 2]];
                    let val: u32 = val_tok.trim_end_matches(['u', 'U']).parse().map_err(|_| {
                        PreludeError::InvalidBinding {
                            stage,
                            message: format!(
                                "invalid integer `{val_tok}` for `{key}` in `layout(...)`"
                            ),
                        }
                    })?;
                    match key {
                        "set" => {
                            quals.set = Some(val);
                            quals.has_set_key = true;
                        }
                        "binding" => {
                            quals.binding = Some(val);
                            quals.has_binding_key = true;
                        }
                        "location" => {
                            quals.location = Some(val);
                            quals.has_location_key = true;
                        }
                        _ => {}
                    }
                    k += 3;
                    continue;
                }
                f if depth == 1 && IMAGE_FORMATS.contains(&f) => {
                    quals.image_format = Some(f.to_owned());
                }
                _ => {}
            }
            k += 1;
        }
        if depth != 0 {
            return Err(PreludeError::Parse {
                stage,
                message: "unclosed `layout(...)` qualifier".to_owned(),
            });
        }
    }
    Ok((quals, k))
}

/// Extract member identifier names declared inside `{ ... }` of an interface block.
fn extract_block_member_names(
    tokens: &[&str],
    open_brace: usize,
    close_brace: usize,
) -> Vec<String> {
    let mut names = Vec::new();
    let mut stmt_start = open_brace + 1;
    let mut depth = 0usize;
    for i in open_brace + 1..close_brace {
        match tokens[i] {
            "{" | "(" | "[" => depth += 1,
            "}" | ")" | "]" => depth = depth.saturating_sub(1),
            ";" if depth == 0 => {
                let words: Vec<&str> = tokens[stmt_start..i]
                    .iter()
                    .copied()
                    .filter(|t| significant(t))
                    .collect();
                let mut k = 0;
                while k < words.len() {
                    if words[k] == "layout" {
                        k += 1;
                        let mut p = 0usize;
                        while k < words.len() {
                            match words[k] {
                                "(" => p += 1,
                                ")" => {
                                    p = p.saturating_sub(1);
                                    k += 1;
                                    if p == 0 {
                                        break;
                                    }
                                    continue;
                                }
                                _ => {}
                            }
                            k += 1;
                        }
                        continue;
                    }
                    if QUALIFIERS.contains(&words[k])
                        || matches!(
                            words[k],
                            "const"
                                | "readonly"
                                | "writeonly"
                                | "coherent"
                                | "volatile"
                                | "restrict"
                        )
                    {
                        k += 1;
                        continue;
                    }
                    break;
                }
                // words[k] is the member type; declarators follow.
                let mut d = 0i32;
                let mut expect_name = true;
                for &w in words.iter().skip(k + 1) {
                    if d == 0 && expect_name && is_ident(w) {
                        names.push(w.to_owned());
                        expect_name = false;
                    }
                    match w {
                        "(" | "[" | "{" => d += 1,
                        ")" | "]" | "}" => d -= 1,
                        "," if d == 0 => expect_name = true,
                        _ => {}
                    }
                }
                stmt_start = i + 1;
            }
            _ => {}
        }
    }
    names
}

/// Split lexed tokens into top-level GLSL items (`tokens[start..end]`).
fn split_top_level_items<'a>(
    tokens: &'a [&'a str],
    stage: &'static str,
) -> Result<Vec<&'a [&'a str]>, PreludeError> {
    let mut items = Vec::new();
    let mut start = 0usize;
    let mut brace_depth = 0usize;
    let mut paren_depth = 0usize;
    let mut block_is_fn = false;

    for (i, &t) in tokens.iter().enumerate() {
        match t {
            "(" => paren_depth += 1,
            ")" => {
                if paren_depth == 0 {
                    return Err(PreludeError::Parse {
                        stage,
                        message: "unexpected `)`".to_owned(),
                    });
                }
                paren_depth -= 1;
            }
            "{" => {
                if brace_depth == 0 && paren_depth == 0 {
                    // Distinguish function body from struct / uniform / buffer / interface block.
                    let head_sig: Vec<&str> = tokens[start..i]
                        .iter()
                        .copied()
                        .filter(|tok| significant(tok))
                        .collect();
                    let is_block = head_sig
                        .iter()
                        .any(|w| matches!(*w, "struct" | "uniform" | "buffer"))
                        || (!head_sig.contains(&"(")
                            && head_sig.iter().any(|w| matches!(*w, "in" | "out")));
                    block_is_fn = !is_block;
                }
                brace_depth += 1;
            }
            "}" => {
                if brace_depth == 0 {
                    return Err(PreludeError::Parse {
                        stage,
                        message: "unexpected `}`".to_owned(),
                    });
                }
                brace_depth -= 1;
                if brace_depth == 0 && paren_depth == 0 && block_is_fn {
                    items.push(&tokens[start..=i]);
                    start = i + 1;
                    block_is_fn = false;
                }
            }
            ";" if brace_depth == 0 && paren_depth == 0 => {
                items.push(&tokens[start..=i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    if brace_depth != 0 || paren_depth != 0 {
        return Err(PreludeError::Parse {
            stage,
            message: "unclosed `{` or `(`".to_owned(),
        });
    }
    if tokens[start..].iter().any(|t| significant(t)) {
        return Err(PreludeError::Parse {
            stage,
            message: "incomplete declaration at end of prelude (missing `;` or `}`)".to_owned(),
        });
    }
    Ok(items)
}

impl ParsedStagePrelude {
    pub(super) fn parse(source: &str, stage: ShaderStage) -> Result<Self, PreludeError> {
        let label = stage_label(stage);
        let stripped = strip_comments(source, label)?;
        check_no_directives(&stripped, label)?;
        let tokens = lex(&stripped);
        let raw_items = split_top_level_items(&tokens, label)?;

        let mut items = Vec::new();
        let mut init_defs = 0usize;
        let mut uses_frag_coord = false;
        let mut writes_texcoord1 = false;
        let mut writes_texcoord2 = false;
        let mut referenced = BTreeSet::new();

        for item in raw_items {
            let sig: Vec<usize> = (0..item.len()).filter(|&i| significant(item[i])).collect();
            if sig.is_empty() || (sig.len() == 1 && item[sig[0]] == ";") {
                continue;
            }

            // Track references to `gl_FragCoord`, `gl_MultiTexCoord1/2`, and all identifiers
            // inside bodies/initializers.
            let has_brace = item.contains(&"{");
            if has_brace {
                for &tok in item {
                    if tok == "gl_FragCoord" {
                        uses_frag_coord = true;
                    } else if tok == "gl_MultiTexCoord1" || tok == "hz_MultiTexCoord1" {
                        writes_texcoord1 = true;
                    } else if tok == "gl_MultiTexCoord2" || tok == "hz_MultiTexCoord2" {
                        writes_texcoord2 = true;
                    }
                    if is_ident(tok) {
                        referenced.insert(tok.to_owned());
                    }
                }
            }

            // Find first top-level `{` if any.
            let mut pdepth = 0usize;
            let open_brace = sig.iter().copied().find(|&idx| match item[idx] {
                "(" => {
                    pdepth += 1;
                    false
                }
                ")" => {
                    pdepth = pdepth.saturating_sub(1);
                    false
                }
                "{" if pdepth == 0 => true,
                _ => false,
            });

            if let Some(open_idx) = open_brace {
                let head_sig: Vec<usize> = sig
                    .iter()
                    .copied()
                    .take_while(|&idx| idx < open_idx)
                    .collect();
                let (quals, after_layout) = parse_layout_quals(item, &head_sig, 0, label)?;
                let rest: Vec<&str> = head_sig[after_layout..].iter().map(|&i| item[i]).collect();

                if let Some(s_pos) = rest.iter().position(|&w| w == "struct") {
                    let Some(&struct_name) = rest.get(s_pos + 1) else {
                        return Err(PreludeError::Parse {
                            stage: label,
                            message: "anonymous struct at top level is not supported".to_owned(),
                        });
                    };
                    check_ident(struct_name, label)?;
                    // Check trailing declarators after matching `}`.
                    let close_idx = item.iter().rposition(|&t| t == "}").unwrap_or(open_idx);
                    let mut vars = Vec::new();
                    let mut d = 0i32;
                    let mut expect_name = true;
                    for &tok in &item[close_idx + 1..] {
                        if !significant(tok) {
                            continue;
                        }
                        if tok == ";" {
                            break;
                        }
                        if d == 0 && expect_name && is_ident(tok) {
                            check_ident(tok, label)?;
                            vars.push(tok.to_owned());
                            expect_name = false;
                        }
                        match tok {
                            "(" | "[" | "{" => d += 1,
                            ")" | "]" | "}" => d -= 1,
                            "," if d == 0 => expect_name = true,
                            _ => {}
                        }
                    }
                    items.push(PreludeItem::Struct {
                        name: struct_name.to_owned(),
                        vars,
                        raw: item.concat(),
                    });
                    continue;
                }

                if let Some(kind_pos) = rest.iter().position(|&w| matches!(w, "uniform" | "buffer"))
                {
                    let kw = rest[kind_pos];
                    let set = quals.set.ok_or_else(|| PreludeError::InvalidBinding {
                        stage: label,
                        message: format!(
                            "{kw} block must declare `layout(set = 0, binding = N)` or `layout(set = 1, binding = N)`"
                        ),
                    })?;
                    if set > 1 {
                        return Err(PreludeError::InvalidBinding {
                            stage: label,
                            message: format!(
                                "invalid descriptor set {set}: prelude resources may only bind set 0 or set 1"
                            ),
                        });
                    }
                    let binding = quals.binding.ok_or_else(|| PreludeError::InvalidBinding {
                        stage: label,
                        message: format!("{kw} block is missing `binding = N` in `layout(...)`"),
                    })?;
                    let Some(&block_name) = rest.get(kind_pos + 1) else {
                        return Err(PreludeError::Parse {
                            stage: label,
                            message: format!("expected block name after `{kw}`"),
                        });
                    };
                    check_ident(block_name, label)?;
                    let close_idx = item.iter().rposition(|&t| t == "}").unwrap_or(open_idx);
                    let instance_name = item[close_idx + 1..]
                        .iter()
                        .copied()
                        .filter(|t| significant(t))
                        .take_while(|&t| t != ";" && t != "[")
                        .find(|t| is_ident(t));
                    let mut declared_names = vec![block_name.to_owned()];
                    let primary_name;
                    if let Some(inst) = instance_name {
                        check_ident(inst, label)?;
                        declared_names.push(inst.to_owned());
                        primary_name = inst.to_owned();
                    } else {
                        let members = extract_block_member_names(item, open_idx, close_idx);
                        for m in &members {
                            check_ident(m, label)?;
                        }
                        declared_names.extend(members);
                        primary_name = block_name.to_owned();
                    }
                    let kind = if kw == "uniform" {
                        ParsedModResourceKind::UniformBlock
                    } else {
                        let read_only = rest[..kind_pos].contains(&"readonly");
                        ParsedModResourceKind::StorageBuffer { read_only }
                    };
                    items.push(PreludeItem::ModResource {
                        set,
                        binding,
                        kind,
                        primary_name,
                        declared_names,
                        raw: item.concat(),
                    });
                    continue;
                }

                if !rest.contains(&"(") && rest.iter().any(|w| matches!(*w, "in" | "out")) {
                    return Err(PreludeError::Parse {
                        stage: label,
                        message: "interface blocks (`in`/`out Block { ... }`) are not supported in preludes; declare individual varyings instead".to_owned(),
                    });
                }

                // Function definition.
                let (fn_name, is_vertex_init) = parse_function_header(&rest, stage, label, true)?;
                if is_vertex_init {
                    init_defs += 1;
                    if init_defs > 1 {
                        return Err(PreludeError::InvalidVertexInit {
                            stage: label,
                            reason: "may be defined only once".to_owned(),
                        });
                    }
                }
                items.push(PreludeItem::Function {
                    name: fn_name,
                    is_vertex_init,
                    raw: item.concat(),
                });
                continue;
            }

            // No top-level `{`: declaration or function prototype ending with `;`.
            let stmt_sig = if item[*sig.last().unwrap()] == ";" {
                &sig[..sig.len() - 1]
            } else {
                &sig[..]
            };
            let (quals, after_layout) = parse_layout_quals(item, stmt_sig, 0, label)?;
            let rest_sig = &stmt_sig[after_layout..];
            let rest: Vec<&str> = rest_sig.iter().map(|&i| item[i]).collect();
            if rest.is_empty() {
                continue;
            }

            // Function prototype: `(` before `=` / `[`, and no storage qualifier before `(`.
            let first_delim = rest.iter().position(|&w| matches!(w, "(" | "=" | "["));
            let is_proto = first_delim.is_some_and(|p| rest[p] == "(")
                && !rest[..first_delim.unwrap()]
                    .iter()
                    .any(|w| STORAGE.contains(w));
            if is_proto {
                let (fn_name, is_vertex_init) = parse_function_header(&rest, stage, label, false)?;
                items.push(PreludeItem::Function {
                    name: fn_name,
                    is_vertex_init,
                    raw: item.concat(),
                });
                continue;
            }

            // Parse qualifiers + storage class + type + declarators.
            let mut k = 0usize;
            let mut storage: Option<&str> = None;
            let mut access: Option<&str> = None;
            let mut is_const = false;
            let mut interpolation = String::new();
            let mut extra_quals = String::new();
            while k < rest.len() {
                let w = rest[k];
                if STORAGE.contains(&w) {
                    storage = Some(w);
                } else if w == "const" {
                    is_const = true;
                } else if matches!(w, "readonly" | "writeonly") {
                    access = Some(w);
                } else if QUALIFIERS.contains(&w)
                    || matches!(w, "coherent" | "volatile" | "restrict")
                {
                    if matches!(
                        w,
                        "flat" | "smooth" | "noperspective" | "centroid" | "sample"
                    ) {
                        write!(interpolation, "{w} ").unwrap();
                    } else {
                        write!(extra_quals, "{w} ").unwrap();
                    }
                } else {
                    break;
                }
                k += 1;
            }
            if interpolation.split_whitespace().any(|w| w == "flat") {
                interpolation = "flat ".to_owned();
            }
            let Some(&ty) = rest.get(k) else {
                return Err(PreludeError::Parse {
                    stage: label,
                    message: format!("expected type in declaration `{}`", item.concat().trim()),
                });
            };
            let declarators = split_declarators(&rest[k + 1..], label)?;
            if declarators.is_empty() {
                return Err(PreludeError::Parse {
                    stage: label,
                    message: format!(
                        "expected variable name after `{ty}` in `{}`",
                        item.concat().trim()
                    ),
                });
            }

            // Mod bind-group resource (`layout(set = ..., binding = ...) uniform ...`).
            if quals.has_set_key || quals.has_binding_key {
                if storage != Some("uniform") {
                    return Err(PreludeError::InvalidBinding {
                        stage: label,
                        message: "resource with `set`/`binding` must be a `uniform` or `buffer` declaration".to_owned(),
                    });
                }
                let set = quals.set.ok_or_else(|| PreludeError::InvalidBinding {
                    stage: label,
                    message: "resource with `binding` must also declare `set = 0` or `set = 1`"
                        .to_owned(),
                })?;
                if set > 1 {
                    return Err(PreludeError::InvalidBinding {
                        stage: label,
                        message: format!(
                            "invalid descriptor set {set}: prelude resources may only bind set 0 or set 1"
                        ),
                    });
                }
                let binding = quals.binding.ok_or_else(|| PreludeError::InvalidBinding {
                    stage: label,
                    message: "resource with `set` must also declare `binding = N`".to_owned(),
                })?;
                if declarators.len() != 1 || !declarators[0].array.is_empty() {
                    return Err(PreludeError::InvalidBinding {
                        stage: label,
                        message: "declare one non-array resource per `layout(set, binding)`"
                            .to_owned(),
                    });
                }
                let name = &declarators[0].name;
                check_ident(name, label)?;
                let kind = if matches!(ty, "texture2D" | "itexture2D" | "utexture2D") {
                    ParsedModResourceKind::Texture2D
                } else if matches!(ty, "sampler" | "samplerShadow") {
                    ParsedModResourceKind::Sampler
                } else if is_sampler_type(ty) {
                    ParsedModResourceKind::CombinedSampler { ty: ty.to_owned() }
                } else if ty.contains("image") {
                    if quals.image_format.is_none() {
                        return Err(PreludeError::InvalidBinding {
                            stage: label,
                            message: format!(
                                "storage image `{name}` requires a format qualifier such as `layout(set = 1, binding = {binding}, rgba8)`"
                            ),
                        });
                    }
                    let img_access = match access {
                        Some("readonly") => PreludeImageAccess::ReadOnly,
                        Some("writeonly") => PreludeImageAccess::WriteOnly,
                        _ => PreludeImageAccess::ReadWrite,
                    };
                    ParsedModResourceKind::StorageImage { access: img_access }
                } else {
                    return Err(PreludeError::InvalidBinding {
                        stage: label,
                        message: format!(
                            "unsupported resource type `{ty}` at `(set = {set}, binding = {binding})`: use a uniform/storage block, `texture2D`, `sampler`, `sampler2D`, or `image2D`"
                        ),
                    });
                };
                items.push(PreludeItem::ModResource {
                    set,
                    binding,
                    kind,
                    primary_name: name.clone(),
                    declared_names: vec![name.clone()],
                    raw: item.concat(),
                });
                continue;
            }

            match storage {
                Some("uniform") if is_sampler_type(ty) => {
                    for decl in declarators {
                        check_ident(&decl.name, label)?;
                        items.push(PreludeItem::PackSampler {
                            name: decl.name,
                            ty: ty.to_owned(),
                        });
                    }
                }
                Some("uniform") if ty.contains("image") => {
                    return Err(PreludeError::InvalidBinding {
                        stage: label,
                        message: "storage image in prelude must declare `layout(set = 1, binding = N, <format>)`"
                            .to_owned(),
                    });
                }
                Some("uniform") => {
                    for decl in declarators {
                        check_ident(&decl.name, label)?;
                        items.push(PreludeItem::LooseUniform {
                            name: decl.name,
                            ty: format!("{ty}{}", decl.array),
                        });
                    }
                }
                Some("in" | "attribute") if stage == ShaderStage::Vertex => {
                    let Some(loc) = quals.location else {
                        return Err(PreludeError::InvalidVertexInput {
                            message: format!(
                                "vertex input `{}` must declare `layout(location = N)` matching the pipeline's `VertexFormat`",
                                declarators[0].name
                            ),
                        });
                    };
                    if declarators.len() != 1 {
                        return Err(PreludeError::InvalidVertexInput {
                            message: "declare one vertex input per `layout(location = N)`"
                                .to_owned(),
                        });
                    }
                    let decl = &declarators[0];
                    check_ident(&decl.name, label)?;
                    items.push(PreludeItem::VertexInput {
                        location: loc,
                        ty: format!("{ty}{}", decl.array),
                        name: decl.name.clone(),
                        raw: item.concat(),
                    });
                }
                Some("out") if stage == ShaderStage::Fragment => {
                    return Err(PreludeError::FragmentOutput {
                        name: declarators[0].name.clone(),
                    });
                }
                Some("out" | "in" | "varying") => {
                    if quals.has_location_key {
                        return Err(PreludeError::ExplicitVaryingLocation {
                            stage: label,
                            name: declarators[0].name.clone(),
                        });
                    }
                    for decl in declarators {
                        check_ident(&decl.name, label)?;
                        let decl_at_name = format!("{interpolation}{ty} @{};", decl.array);
                        items.push(PreludeItem::Varying {
                            name: decl.name,
                            ty: format!("{ty}{}", decl.array),
                            decl_at_name,
                        });
                    }
                }
                None => {
                    let prefix = if is_const {
                        format!("const {extra_quals}")
                    } else {
                        extra_quals
                    };
                    for decl in declarators {
                        check_ident(&decl.name, label)?;
                        if let Some(init) = &decl.init {
                            for tok in lex(init) {
                                if is_ident(tok) {
                                    referenced.insert(tok.to_owned());
                                }
                            }
                        }
                        let init_str = match &decl.init {
                            Some(init) => format!(" = {init}"),
                            None => String::new(),
                        };
                        items.push(PreludeItem::Global {
                            name: decl.name.clone(),
                            ty: format!("{ty}{}", decl.array),
                            raw: format!("{prefix}{ty} {}{}{init_str};\n", decl.name, decl.array),
                        });
                    }
                }
                Some(other) => {
                    return Err(PreludeError::Parse {
                        stage: label,
                        message: format!("unsupported storage qualifier `{other}`"),
                    });
                }
            }
        }

        if stage == ShaderStage::Vertex && init_defs == 0 {
            return Err(PreludeError::MissingVertexInit);
        }

        Ok(Self {
            stage,
            items,
            uses_frag_coord,
            writes_texcoord1,
            writes_texcoord2,
            referenced,
        })
    }

    pub(super) fn validate_against_layout(
        &self,
        layout: &PreludeLayout<'_>,
    ) -> Result<(), PreludeError> {
        let label = stage_label(self.stage);
        let mut seen_locations = HashSet::new();
        for item in &self.items {
            match item {
                PreludeItem::VertexInput {
                    location, ty, name, ..
                } => {
                    if !seen_locations.insert(*location) {
                        return Err(PreludeError::InvalidVertexInput {
                            message: format!(
                                "duplicate vertex input `layout(location = {location})` on `{name}`"
                            ),
                        });
                    }
                    let Some(expected) = layout
                        .vertex_inputs
                        .iter()
                        .find(|v| v.location == *location)
                    else {
                        return Err(PreludeError::InvalidVertexInput {
                            message: format!(
                                "vertex input `{name}` at location {location} has no matching attribute in the pipeline's `VertexFormat`"
                            ),
                        });
                    };
                    if expected.glsl_type != ty {
                        return Err(PreludeError::InvalidVertexInput {
                            message: format!(
                                "vertex input `{name}` at location {location} declared as `{ty}`, expected `{}` from `VertexFormat`",
                                expected.glsl_type
                            ),
                        });
                    }
                }
                PreludeItem::ModResource {
                    set,
                    binding,
                    kind,
                    primary_name,
                    ..
                } => {
                    validate_mod_resource_binding(
                        layout,
                        self.stage,
                        label,
                        *set,
                        *binding,
                        kind,
                        primary_name,
                    )?;
                }
                _ => {}
            }
        }
        Ok(())
    }

    /// Parse and validate the prelude on its own with naga's GLSL frontend.
    pub(super) fn validate_standalone(&self) -> Result<(), PreludeError> {
        let label = stage_label(self.stage);
        let mut header = String::from(
            "#version 450\n\
             mat4 hz_ModelViewMatrix; mat4 hz_ProjectionMatrix; mat4 hz_ModelViewProjectionMatrix;\n\
             mat4 hz_ModelViewMatrixInverse; mat4 hz_ProjectionMatrixInverse; mat3 hz_NormalMatrix; mat4 hz_TextureMatrix[8];\n\
             struct hz_FogParameters { vec4 color; float density; float start; float end; float scale; };\n\
             hz_FogParameters hz_Fog;\n",
        );
        if self.stage == ShaderStage::Vertex {
            header.push_str(
                "vec4 hz_Vertex; vec4 hz_Color; vec4 hz_SecondaryColor; vec3 hz_Normal; float hz_FogCoord;\n\
                 vec4 hz_MultiTexCoord0; vec4 hz_MultiTexCoord1; vec4 hz_MultiTexCoord2; vec4 hz_MultiTexCoord3;\n\
                 vec4 hz_MultiTexCoord4; vec4 hz_MultiTexCoord5; vec4 hz_MultiTexCoord6; vec4 hz_MultiTexCoord7;\n\
                 uint hz_vertex_index;\n",
            );
        } else {
            header.push_str("vec4 hz_FragCoord;\n");
        }

        // Collect loose uniforms, loose pack samplers, and combined mod samplers for standalone
        // naga checking.
        let mut loose_uniforms: Vec<(&str, &str)> = Vec::new();
        let mut pack_samplers: Vec<(&str, &str)> = Vec::new();
        let mut combined_samplers: HashMap<String, String> = HashMap::new();
        let mut bool_uniforms: HashSet<String> = HashSet::new();

        for item in &self.items {
            match item {
                PreludeItem::LooseUniform { name, ty } => {
                    loose_uniforms.push((name.as_str(), ty.as_str()));
                    if ty == "bool" {
                        bool_uniforms.insert(name.clone());
                    }
                }
                PreludeItem::PackSampler { name, ty } => {
                    pack_samplers.push((name.as_str(), ty.as_str()));
                    combined_samplers.insert(name.clone(), ty.clone());
                }
                PreludeItem::ModResource {
                    kind: ParsedModResourceKind::CombinedSampler { ty },
                    primary_name,
                    ..
                } => {
                    combined_samplers.insert(primary_name.clone(), ty.clone());
                }
                _ => {}
            }
        }

        if !loose_uniforms.is_empty() {
            header.push_str("layout(set = 0, binding = 0) uniform HzPreludeUniforms {");
            for (name, ty) in &loose_uniforms {
                if *ty == "bool" {
                    write!(header, " int hz_bool_{name};").unwrap();
                } else if let Some((base, size)) = ty.split_once('[') {
                    write!(header, " {base} {name}[{size};").unwrap();
                } else {
                    write!(header, " {ty} {name};").unwrap();
                }
            }
            header.push_str(" };\n");
        }
        for (i, (name, ty)) in pack_samplers.iter().enumerate() {
            let (tex_ty, samp_ty) = split_sampler(ty);
            let b = 1 + 2 * i as u32;
            writeln!(
                header,
                "layout(set = 0, binding = {b}) uniform {tex_ty} {name}; \
                 layout(set = 0, binding = {}) uniform {samp_ty} hz_s_{name};",
                b + 1
            )
            .unwrap();
        }

        let mut next_varying_loc = 0u32;
        let dir = if self.stage == ShaderStage::Vertex {
            "out"
        } else {
            "in"
        };
        let mut body = String::new();
        for item in &self.items {
            match item {
                PreludeItem::LooseUniform { .. } | PreludeItem::PackSampler { .. } => {}
                PreludeItem::Varying {
                    name, decl_at_name, ..
                } => {
                    writeln!(
                        body,
                        "layout(location = {next_varying_loc}) {dir} {}",
                        decl_at_name.replace('@', name)
                    )
                    .unwrap();
                    next_varying_loc += 1;
                }
                PreludeItem::VertexInput { raw, .. } | PreludeItem::Struct { raw, .. } => {
                    body.push_str(raw);
                    body.push('\n');
                }
                PreludeItem::ModResource {
                    set,
                    binding,
                    kind,
                    primary_name,
                    raw,
                    ..
                } => {
                    body.push_str(&emit_mod_resource(*set, *binding, kind, primary_name, raw));
                    body.push('\n');
                }
                PreludeItem::Global { raw, .. } | PreludeItem::Function { raw, .. } => {
                    body.push_str(&rewrite_prelude_code(
                        raw,
                        self.stage,
                        &combined_samplers,
                        &bool_uniforms,
                    ));
                    body.push('\n');
                }
            }
        }
        if self.stage == ShaderStage::Vertex {
            body.push_str(
                "void main() { hz_vertex_index = gl_VertexIndex; sunrise_pack_vertex_init(); gl_Position = vec4(0.0); }\n",
            );
        } else {
            body.push_str("void main() {}\n");
        }

        let locate = |line: usize| sunrise_glsl::Location {
            file: format!("<{label}_prelude>"),
            line: line as u32 + 1,
        };
        sunrise_glsl::compile_stage(
            &sunrise_glsl::Source {
                stage: self.stage,
                header: &header,
                code: &body,
                locate: &locate,
            },
            &sunrise_glsl::BindingMap::new(),
            crate::DEVICELESS_INTERFACE_LIMITS,
        )
        .map(|_| ())
        .map_err(|errors| PreludeError::Parse {
            stage: label,
            message: errors
                .into_iter()
                .map(|e| e.message)
                .collect::<Vec<_>>()
                .join("; "),
        })
    }

    /// Merge this parsed prelude with a scanned pack stage, deduplicating identical variable/
    /// input/uniform/varying declarations and reporting any incompatible collision as an error.
    pub(super) fn merge_into_stage(
        &self,
        stage: &mut Stage<'_>,
        uniforms: &mut BTreeMap<String, String>,
        pack_samplers: &mut BTreeMap<String, String>,
        errors: &mut Vec<Error>,
    ) -> MergedStagePrelude {
        let pp = stage.pp;
        let stage_file = pp.file();
        let label = stage_label(self.stage);
        let mut shadowed_pack_attributes = BTreeSet::new();
        let mut combined_samplers: HashMap<String, String> = pack_samplers
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        let mut extra_samplers: Vec<Sampler> = Vec::new();
        let mut mod_resource_emits: Vec<String> = Vec::new();
        let mut vertex_input_emits: Vec<String> = Vec::new();
        let mut other_decls: Vec<String> = Vec::new();
        let mut helper_fns: Vec<String> = Vec::new();
        let mut vertex_init_raw: Option<String> = None;

        // First collect all prelude-declared combined samplers (pack samplers + mod set-6/7
        // combined samplers) so function bodies can rewrite every sampler call consistently.
        for item in &self.items {
            match item {
                PreludeItem::PackSampler { name, ty } => {
                    combined_samplers
                        .entry(name.clone())
                        .or_insert_with(|| ty.clone());
                }
                PreludeItem::ModResource {
                    binding,
                    kind: ParsedModResourceKind::CombinedSampler { ty },
                    primary_name,
                    ..
                } => {
                    combined_samplers.insert(primary_name.clone(), ty.clone());
                    extra_samplers.push(Sampler {
                        name: primary_name.clone(),
                        ty: ty.clone(),
                        binding: *binding,
                    });
                }
                _ => {}
            }
        }

        // Helper: find what (if anything) the pack stage already declared under `name`.
        fn pack_decl_of<'a>(stage: &'a Stage<'_>, name: &str) -> Option<PackDecl<'a>> {
            if let Some((_, ty, _)) = stage.uniforms.iter().find(|(n, _, _)| n == name) {
                return Some(PackDecl::Uniform(ty.as_str()));
            }
            if let Some((_, ty)) = stage.samplers.iter().find(|(n, _)| n == name) {
                return Some(PackDecl::Sampler(ty.as_str()));
            }
            if stage.images.iter().any(|i| i.name == name) {
                return Some(PackDecl::Image);
            }
            if let Some((_, _, ty)) = stage.attributes.iter().find(|(n, _, _)| n == name) {
                return Some(PackDecl::VertexAttr(ty.as_str()));
            }
            if stage.kind == ShaderStage::Vertex
                && let Some(v) = stage.outs.iter().find(|v| v.name == name)
            {
                return Some(PackDecl::Varying(v.decl.as_str()));
            }
            if stage.kind == ShaderStage::Fragment
                && let Some(v) = stage.ins.iter().find(|v| v.name == name)
            {
                return Some(PackDecl::Varying(v.decl.as_str()));
            }
            if stage.functions.contains(name) {
                return Some(PackDecl::Function);
            }
            if let Some(ty) = stage.global_types.get(name) {
                return Some(PackDecl::Global(ty.as_str()));
            }
            if stage.structs.contains_key(name) || stage.declared.contains(name) {
                return Some(PackDecl::Other);
            }
            None
        }

        for item in &self.items {
            match item {
                PreludeItem::LooseUniform { name, ty } => {
                    // Check cross-stage or same-stage collision.
                    if let Some(existing) = pack_decl_of(stage, name) {
                        match existing {
                            PackDecl::Uniform(pack_ty) if pack_ty == ty => {
                                // Exact match: pack's declaration stands, drop prelude copy.
                                continue;
                            }
                            _ => {
                                errors.push(Error::new(
                                    stage_file,
                                    0,
                                    format!(
                                        "{label} prelude uniform `{ty} {name}` collides with pack's declaration of `{name}`"
                                    ),
                                ));
                                continue;
                            }
                        }
                    }
                    if let Some(other_ty) = uniforms.get(name) {
                        if other_ty != ty {
                            errors.push(Error::new(
                                stage_file,
                                0,
                                format!(
                                    "{label} prelude uniform `{name}` is `{ty}`, colliding with `{other_ty}` in the other stage"
                                ),
                            ));
                        }
                        continue;
                    }
                    uniforms.insert(name.clone(), ty.clone());
                }
                PreludeItem::PackSampler { name, ty } => {
                    if let Some(existing) = pack_decl_of(stage, name) {
                        match existing {
                            PackDecl::Sampler(pack_ty) if pack_ty == ty => {
                                continue;
                            }
                            _ => {
                                errors.push(Error::new(
                                    stage_file,
                                    0,
                                    format!(
                                        "{label} prelude sampler `{ty} {name}` collides with pack's declaration of `{name}`"
                                    ),
                                ));
                                continue;
                            }
                        }
                    }
                    if let Some(other_ty) = pack_samplers.get(name) {
                        if other_ty != ty {
                            errors.push(Error::new(
                                stage_file,
                                0,
                                format!(
                                    "{label} prelude sampler `{name}` is `{ty}`, colliding with `{other_ty}` in the other stage"
                                ),
                            ));
                        }
                        continue;
                    }
                    pack_samplers.insert(name.clone(), ty.clone());
                }
                PreludeItem::VertexInput { ty, name, raw, .. } => {
                    if let Some(existing) = pack_decl_of(stage, name) {
                        match existing {
                            PackDecl::VertexAttr(pack_ty) if pack_ty == ty => {
                                // Real vertex input takes precedence over pack's private global of
                                // identical name and type.
                                shadowed_pack_attributes.insert(name.clone());
                            }
                            _ => {
                                errors.push(Error::new(
                                    stage_file,
                                    0,
                                    format!(
                                        "vertex prelude input `{ty} {name}` collides with pack's declaration of `{name}`"
                                    ),
                                ));
                                continue;
                            }
                        }
                    }
                    vertex_input_emits.push(raw.clone());
                }
                PreludeItem::Varying {
                    name,
                    ty,
                    decl_at_name,
                } => {
                    enum VaryingMatch {
                        None,
                        Same,
                        Conflict,
                    }
                    let matched = match pack_decl_of(stage, name) {
                        None => VaryingMatch::None,
                        Some(PackDecl::Varying(pack_decl)) if pack_decl == decl_at_name => {
                            VaryingMatch::Same
                        }
                        Some(_) => VaryingMatch::Conflict,
                    };
                    match matched {
                        VaryingMatch::Same => {
                            if self.stage == ShaderStage::Fragment {
                                stage.prelude_reads.insert(name.clone());
                            }
                            continue;
                        }
                        VaryingMatch::Conflict => {
                            errors.push(Error::new(
                                stage_file,
                                0,
                                format!(
                                    "{label} prelude varying `{ty} {name}` collides with pack's declaration of `{name}`"
                                ),
                            ));
                            continue;
                        }
                        VaryingMatch::None => {}
                    }
                    let var = Var {
                        name: name.clone(),
                        decl: decl_at_name.clone(),
                        structure: false,
                        at: 0,
                    };
                    if self.stage == ShaderStage::Vertex {
                        stage.outs.push(var);
                    } else {
                        stage.ins.push(var);
                        stage.prelude_reads.insert(name.clone());
                    }
                }
                PreludeItem::Global { name, ty, raw } => {
                    if let Some(existing) = pack_decl_of(stage, name) {
                        match existing {
                            PackDecl::VertexAttr(pack_ty) | PackDecl::Global(pack_ty)
                                if pack_ty == ty =>
                            {
                                // Same name and same type: pack's declaration stands, drop prelude copy.
                                continue;
                            }
                            _ => {
                                errors.push(Error::new(
                                    stage_file,
                                    0,
                                    format!(
                                        "{label} prelude variable `{ty} {name}` collides with pack's declaration of `{name}`"
                                    ),
                                ));
                                continue;
                            }
                        }
                    }
                    other_decls.push(raw.clone());
                }
                PreludeItem::Struct { name, vars, raw } => {
                    let mut collided = false;
                    for n in std::iter::once(name).chain(vars.iter()) {
                        if pack_decl_of(stage, n).is_some() {
                            errors.push(Error::new(
                                stage_file,
                                0,
                                format!(
                                    "{label} prelude struct/variable `{n}` collides with pack's declaration of `{n}`"
                                ),
                            ));
                            collided = true;
                        }
                    }
                    if !collided {
                        other_decls.push(raw.clone());
                    }
                }
                PreludeItem::ModResource {
                    set,
                    binding,
                    kind,
                    primary_name,
                    declared_names,
                    raw,
                } => {
                    let mut collided = false;
                    for n in declared_names {
                        if pack_decl_of(stage, n).is_some() {
                            errors.push(Error::new(
                                stage_file,
                                0,
                                format!(
                                    "{label} prelude resource `{n}` collides with pack's declaration of `{n}`"
                                ),
                            ));
                            collided = true;
                        }
                    }
                    if !collided {
                        mod_resource_emits.push(emit_mod_resource(
                            *set,
                            *binding,
                            kind,
                            primary_name,
                            raw,
                        ));
                    }
                }
                PreludeItem::Function {
                    name,
                    is_vertex_init,
                    raw,
                } => {
                    if pack_decl_of(stage, name).is_some() {
                        errors.push(Error::new(
                            stage_file,
                            0,
                            format!(
                                "{label} prelude function `{name}` collides with pack's declaration of `{name}`"
                            ),
                        ));
                        continue;
                    }
                    if *is_vertex_init {
                        // Keep prototypes out of `vertex_init_raw`, store the definition for
                        // placement after the pack's rewritten code.
                        if raw.contains('{') {
                            vertex_init_raw = Some(raw.clone());
                        } else {
                            helper_fns.push(raw.clone());
                        }
                    } else {
                        helper_fns.push(raw.clone());
                    }
                }
            }
        }

        // Any varying referenced by fragment prelude functions must stay active.
        if self.stage == ShaderStage::Fragment {
            for name in &self.referenced {
                stage.prelude_reads.insert(name.clone());
            }
        }

        // Bool uniforms currently in `uniforms` (after merging loose uniforms).
        let bool_uniforms: HashSet<String> = uniforms
            .iter()
            .filter_map(|(n, ty)| (ty == "bool").then(|| n.clone()))
            .collect();

        let mut header_decls = String::new();
        for decl in vertex_input_emits {
            header_decls.push_str(&decl);
            header_decls.push('\n');
        }
        for decl in mod_resource_emits {
            header_decls.push_str(&decl);
            header_decls.push('\n');
        }
        for decl in other_decls {
            header_decls.push_str(&rewrite_prelude_code(
                &decl,
                self.stage,
                &combined_samplers,
                &bool_uniforms,
            ));
            header_decls.push('\n');
        }
        for func in helper_fns {
            header_decls.push_str(&rewrite_prelude_code(
                &func,
                self.stage,
                &combined_samplers,
                &bool_uniforms,
            ));
            header_decls.push('\n');
        }

        let vertex_init_fn = vertex_init_raw
            .map(|raw| rewrite_prelude_code(&raw, self.stage, &combined_samplers, &bool_uniforms));

        MergedStagePrelude {
            header_decls,
            vertex_init_fn,
            extra_samplers,
            shadowed_pack_attributes,
            uses_frag_coord: self.uses_frag_coord,
            lightmap_from_texcoord2: self.writes_texcoord2 && !self.writes_texcoord1,
        }
    }
}

enum PackDecl<'a> {
    Uniform(&'a str),
    Sampler(&'a str),
    Image,
    VertexAttr(&'a str),
    Varying(&'a str),
    Global(&'a str),
    Function,
    Other,
}

fn parse_function_header(
    rest: &[&str],
    stage: ShaderStage,
    label: &'static str,
    is_definition: bool,
) -> Result<(String, bool), PreludeError> {
    let open = rest
        .iter()
        .position(|&w| w == "(")
        .ok_or_else(|| PreludeError::Parse {
            stage: label,
            message: "expected `(` in function declaration".to_owned(),
        })?;
    if open < 2 {
        return Err(PreludeError::Parse {
            stage: label,
            message: "expected return type and name before `(`".to_owned(),
        });
    }
    let fn_name = rest[open - 1];
    let ret_tokens: Vec<&str> = rest[..open - 1]
        .iter()
        .copied()
        .filter(|w| !QUALIFIERS.contains(w))
        .collect();
    let return_ty = ret_tokens.concat();

    // Count parameters inside `(...)`.
    let close = rest
        .iter()
        .rposition(|&w| w == ")")
        .ok_or_else(|| PreludeError::Parse {
            stage: label,
            message: format!("unclosed parameter list on `{fn_name}`"),
        })?;
    let params = &rest[open + 1..close];
    let param_count = if params.is_empty() || params == ["void"] {
        0
    } else {
        let mut count = 1usize;
        let mut depth = 0usize;
        for &tok in params {
            match tok {
                "(" | "[" => depth += 1,
                ")" | "]" => depth = depth.saturating_sub(1),
                "," if depth == 0 => count += 1,
                _ => {}
            }
        }
        count
    };

    if fn_name == VERTEX_INIT_FN {
        if stage != ShaderStage::Vertex {
            return Err(PreludeError::InvalidVertexInit {
                stage: label,
                reason: "is only allowed in `vertex_prelude`, not `fragment_prelude`".to_owned(),
            });
        }
        if return_ty != "void" || param_count != 0 {
            return Err(PreludeError::InvalidVertexInit {
                stage: label,
                reason: "must have signature `void sunrise_pack_vertex_init()`".to_owned(),
            });
        }
        if !is_definition {
            // Allow a forward prototype if it matches the signature; definition count is checked
            // separately.
            return Ok((fn_name.to_owned(), true));
        }
        return Ok((fn_name.to_owned(), true));
    }

    check_ident(fn_name, label)?;
    Ok((fn_name.to_owned(), false))
}

struct Declarator {
    name: String,
    array: String,
    init: Option<String>,
}

fn split_declarators(words: &[&str], stage: &'static str) -> Result<Vec<Declarator>, PreludeError> {
    let mut out = Vec::new();
    let mut i = 0usize;
    while i < words.len() {
        let name = words[i];
        if !is_ident(name) {
            return Err(PreludeError::InvalidIdentifier {
                stage,
                name: name.to_owned(),
            });
        }
        i += 1;
        let mut array = String::new();
        let mut init = None;
        let mut depth = 0i32;
        while i < words.len() {
            let w = words[i];
            if depth == 0 && w == "," {
                i += 1;
                break;
            }
            if depth == 0 && w == "=" {
                i += 1;
                let init_start = i;
                let mut id = 0i32;
                while i < words.len() {
                    match words[i] {
                        "," if id == 0 => break,
                        "(" | "[" | "{" => id += 1,
                        ")" | "]" | "}" => id -= 1,
                        _ => {}
                    }
                    i += 1;
                }
                init = Some(words[init_start..i].join(" "));
                if i < words.len() && words[i] == "," {
                    i += 1;
                }
                break;
            }
            match w {
                "(" | "[" | "{" => depth += 1,
                ")" | "]" | "}" => depth -= 1,
                _ => {}
            }
            array.push_str(w);
            i += 1;
        }
        out.push(Declarator {
            name: name.to_owned(),
            array,
            init,
        });
    }
    Ok(out)
}

fn validate_mod_resource_binding(
    layout: &PreludeLayout<'_>,
    stage: ShaderStage,
    label: &'static str,
    set: u32,
    binding: u32,
    kind: &ParsedModResourceKind,
    name: &str,
) -> Result<(), PreludeError> {
    // Look for exact binding entry, or for `Sampler` at `b`, a `CombinedSampler2D` at `b - 1`.
    let direct = layout
        .bindings
        .iter()
        .find(|e| e.set == set && e.binding == binding);
    let combined_sampler_second = matches!(kind, ParsedModResourceKind::Sampler)
        .then(|| {
            binding.checked_sub(1).and_then(|prev| {
                layout.bindings.iter().find(|e| {
                    e.set == set
                        && e.binding == prev
                        && e.kind == PreludeResourceKind::CombinedSampler2D
                })
            })
        })
        .flatten();

    let Some(entry) = direct.or(combined_sampler_second) else {
        return Err(PreludeError::InvalidBinding {
            stage: label,
            message: format!(
                "resource `{name}` at `(set = {set}, binding = {binding})` does not exist in this pipeline's layout"
            ),
        });
    };

    if !entry.stages.allows(stage) {
        return Err(PreludeError::StageVisibility {
            stage: label,
            set,
            binding,
        });
    }

    let ok = match (kind, entry.kind) {
        (ParsedModResourceKind::UniformBlock, PreludeResourceKind::UniformBuffer) => true,
        (
            ParsedModResourceKind::StorageBuffer { read_only },
            PreludeResourceKind::StorageBuffer {
                read_only: expected_ro,
            },
        ) => *read_only == expected_ro,
        (
            ParsedModResourceKind::Texture2D,
            PreludeResourceKind::Texture2D | PreludeResourceKind::CombinedSampler2D,
        ) => entry.binding == binding,
        (ParsedModResourceKind::Sampler, PreludeResourceKind::Sampler) => entry.binding == binding,
        (ParsedModResourceKind::Sampler, PreludeResourceKind::CombinedSampler2D) => {
            entry.binding + 1 == binding
        }
        (ParsedModResourceKind::CombinedSampler { ty }, PreludeResourceKind::CombinedSampler2D) => {
            entry.binding == binding && ty == "sampler2D"
        }
        (ParsedModResourceKind::CombinedSampler { ty }, PreludeResourceKind::Texture2D) => {
            ty == "sampler2D"
                && layout.bindings.iter().any(|s| {
                    s.set == set
                        && s.binding == binding + 1
                        && s.kind == PreludeResourceKind::Sampler
                        && s.stages.allows(stage)
                })
        }
        (
            ParsedModResourceKind::StorageImage { access },
            PreludeResourceKind::StorageImage { access: expected },
        ) => *access == expected,
        _ => false,
    };

    if !ok {
        return Err(PreludeError::InvalidBinding {
            stage: label,
            message: format!(
                "resource `{name}` at `(set = {set}, binding = {binding})` does not match the pipeline layout's resource kind or access qualifier"
            ),
        });
    }
    Ok(())
}

/// Emit a mod resource declaration with `set = 0|1` mapped to `MOD_SET_BASE + set` (6 or 7),
/// splitting a combined `uniform sampler2D` into `texture2D` at `binding` and `sampler` at
/// `binding + 1`.
fn emit_mod_resource(
    set: u32,
    binding: u32,
    kind: &ParsedModResourceKind,
    primary_name: &str,
    raw: &str,
) -> String {
    let target_set = MOD_SET_BASE + set;
    if let ParsedModResourceKind::CombinedSampler { ty } = kind {
        let (tex_ty, samp_ty) = split_sampler(ty);
        return format!(
            "layout(set = {target_set}, binding = {binding}) uniform {tex_ty} {primary_name};\n\
             layout(set = {target_set}, binding = {}) uniform {samp_ty} hz_s_{primary_name};",
            binding + 1
        );
    }
    let tokens = lex(raw);
    let mut out = String::with_capacity(raw.len() + 8);
    let mut in_layout = false;
    let mut layout_depth = 0usize;
    let mut i = 0usize;
    while i < tokens.len() {
        let t = tokens[i];
        if t == "layout" && !in_layout {
            in_layout = true;
            out.push_str(t);
            i += 1;
            continue;
        }
        if in_layout {
            match t {
                "(" => layout_depth += 1,
                ")" => {
                    layout_depth = layout_depth.saturating_sub(1);
                    if layout_depth == 0 {
                        in_layout = false;
                    }
                }
                "set" if layout_depth == 1 => {
                    out.push_str("set");
                    // Copy up to `=`, then replace the set integer literal.
                    let mut j = i + 1;
                    while j < tokens.len() && !significant(tokens[j]) {
                        out.push_str(tokens[j]);
                        j += 1;
                    }
                    if j < tokens.len() && tokens[j] == "=" {
                        out.push('=');
                        j += 1;
                        while j < tokens.len() && !significant(tokens[j]) {
                            out.push_str(tokens[j]);
                            j += 1;
                        }
                        if j < tokens.len() {
                            write!(out, "{target_set}").unwrap();
                            j += 1;
                        }
                    }
                    i = j;
                    continue;
                }
                _ => {}
            }
        }
        out.push_str(t);
        i += 1;
    }
    out
}

/// Rewrite compatibility built-ins, `bool` uniforms, and combined sampler arguments inside a
/// prelude's functions and initializers.
fn rewrite_prelude_code(
    raw: &str,
    stage: ShaderStage,
    combined_samplers: &HashMap<String, String>,
    bool_uniforms: &HashSet<String>,
) -> String {
    let fragment = stage == ShaderStage::Fragment;
    let tokens = lex(raw);
    let prev_sig = |idx: usize| (0..idx).rev().find(|&j| significant(tokens[j]));
    let next_sig = |idx: usize| (idx + 1..tokens.len()).find(|&j| significant(tokens[j]));
    let mut out = String::with_capacity(raw.len() + 64);

    for (i, &t) in tokens.iter().enumerate() {
        let prev = prev_sig(i).map(|j| tokens[j]);
        let next = next_sig(i).map(|j| tokens[j]);
        if prev != Some(".") {
            if let Some(sampler_ty) = combined_samplers.get(t)
                && matches!(prev, Some("(" | ","))
                && matches!(next, Some("," | ")"))
            {
                write!(out, "{sampler_ty}({t}, hz_s_{t})").unwrap();
                continue;
            }
            if bool_uniforms.contains(t) && next != Some("(") {
                write!(out, "(hz_bool_{t} != 0)").unwrap();
                continue;
            }
            let replaced = match t {
                "gl_VertexID" if !fragment => Some("int(hz_vertex_index)".to_owned()),
                "gl_VertexIndex" if !fragment => Some("hz_vertex_index".to_owned()),
                "gl_InstanceID" => Some("int(gl_InstanceIndex)".to_owned()),
                "gl_Fog" => Some("hz_Fog".to_owned()),
                "gl_FragCoord" if fragment => Some("hz_FragCoord".to_owned()),
                "texture2D" if next == Some("(") => Some("texture".to_owned()),
                "texture2DLod" if next == Some("(") => Some("textureLod".to_owned()),
                "texture2DGrad" if next == Some("(") => Some("textureGrad".to_owned()),
                "texture2DProj" if next == Some("(") => Some("textureProj".to_owned()),
                _ if UNIFORM_STATE.iter().any(|b| t.starts_with(b)) => {
                    Some(format!("hz_{}", &t[3..]))
                }
                _ if !fragment && VERTEX_INPUTS.iter().any(|b| t.starts_with(b)) => {
                    Some(format!("hz_{}", &t[3..]))
                }
                _ => None,
            };
            if let Some(rep) = replaced {
                out.push_str(&rep);
                continue;
            }
        }
        out.push_str(t);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{GbuffersInputs, Macros, Pack, Target, preprocess, translate_gbuffers};

    fn default_layout() -> PreludeLayout<'static> {
        static V_INPUTS: [PreludeVertexInput<'static>; 3] = [
            PreludeVertexInput {
                location: 0,
                glsl_type: "vec3",
            },
            PreludeVertexInput {
                location: 1,
                glsl_type: "vec2",
            },
            PreludeVertexInput {
                location: 2,
                glsl_type: "vec4",
            },
        ];
        static BINDINGS: [PreludeBinding; 6] = [
            PreludeBinding {
                set: 0,
                binding: 0,
                kind: PreludeResourceKind::UniformBuffer,
                stages: PreludeStages::VERTEX_FRAGMENT,
            },
            PreludeBinding {
                set: 0,
                binding: 1,
                kind: PreludeResourceKind::UniformBuffer,
                stages: PreludeStages::VERTEX_FRAGMENT,
            },
            PreludeBinding {
                set: 1,
                binding: 0,
                kind: PreludeResourceKind::StorageBuffer { read_only: true },
                stages: PreludeStages::VERTEX_FRAGMENT,
            },
            PreludeBinding {
                set: 1,
                binding: 1,
                kind: PreludeResourceKind::CombinedSampler2D,
                stages: PreludeStages::VERTEX_FRAGMENT,
            },
            PreludeBinding {
                set: 1,
                binding: 3,
                kind: PreludeResourceKind::UniformBuffer,
                stages: PreludeStages::FRAGMENT,
            },
            PreludeBinding {
                set: 1,
                binding: 4,
                kind: PreludeResourceKind::StorageBuffer { read_only: false },
                stages: PreludeStages::FRAGMENT,
            },
        ];
        PreludeLayout {
            vertex_inputs: &V_INPUTS,
            bindings: &BINDINGS,
        }
    }

    fn make_pack(vsh: &str, fsh: &str) -> (crate::Preprocessed, crate::Preprocessed) {
        use std::io::{Cursor, Write};
        let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let opts = zip::write::SimpleFileOptions::default();
        zip.start_file("shaders/dh_terrain.vsh", opts).unwrap();
        zip.write_all(vsh.as_bytes()).unwrap();
        zip.start_file("shaders/dh_terrain.fsh", opts).unwrap();
        zip.write_all(fsh.as_bytes()).unwrap();
        let pack =
            Pack::from_zip_bytes("prelude-test", zip.finish().unwrap().into_inner()).unwrap();
        let macros = Macros::standard();
        let v = preprocess(&pack, "dh_terrain.vsh", &macros);
        let f = preprocess(&pack, "dh_terrain.fsh", &macros);
        (v, f)
    }

    #[test]
    fn validates_vertex_and_fragment_preludes_including_zero_vertex_inputs() {
        let prelude = PackPrelude {
            vertex: Some(
                "layout(set = 1, binding = 0, std430) readonly buffer Verts { vec4 pos[]; };\n\
                 int customMaterial;\n\
                 out vec2 customUv;\n\
                 void sunrise_pack_vertex_init() {\n\
                     gl_Vertex = pos[gl_VertexIndex];\n\
                     gl_Color = vec4(1.0);\n\
                     customMaterial = int(gl_InstanceIndex);\n\
                     customUv = vec2(0.5, 0.25);\n\
                 }\n"
                .to_owned(),
            ),
            fragment: Some(
                "layout(set = 1, binding = 1) uniform sampler2D customAtlas;\n\
                 in vec2 customUv;\n\
                 vec4 customSample() { return texture(customAtlas, customUv); }\n"
                    .to_owned(),
            ),
        };
        validate_pack_prelude(&prelude, &default_layout()).expect("valid prelude");
    }

    #[test]
    fn refuses_missing_init_in_vertex_and_init_in_fragment() {
        let missing = PackPrelude {
            vertex: Some("int customFlag;\n".to_owned()),
            fragment: None,
        };
        assert_eq!(
            validate_pack_prelude(&missing, &default_layout()),
            Err(PreludeError::MissingVertexInit)
        );

        let in_frag = PackPrelude {
            vertex: None,
            fragment: Some("void sunrise_pack_vertex_init() {}\n".to_owned()),
        };
        assert!(matches!(
            validate_pack_prelude(&in_frag, &default_layout()),
            Err(PreludeError::InvalidVertexInit {
                stage: "fragment",
                ..
            })
        ));
    }

    #[test]
    fn refuses_reserved_implicit_and_engine_identifiers() {
        for bad in [
            "vec4 colortex0;",
            "float cameraPosition;",
            "vec3 vaPosition;",
            "float mc_chunkFade;",
            "int renderStage;",
            "vec4 hz_custom;",
            "vec4 sunrisePatchPos;",
        ] {
            let prelude = PackPrelude {
                vertex: Some(format!("{bad}\nvoid sunrise_pack_vertex_init() {{}}\n")),
                fragment: None,
            };
            assert!(
                matches!(
                    validate_pack_prelude(&prelude, &default_layout()),
                    Err(PreludeError::ReservedIdentifier { .. })
                ),
                "expected refusal for `{bad}`"
            );
        }
    }

    #[test]
    fn refuses_bad_set_unknown_binding_and_stage_visibility() {
        let bad_set = PackPrelude {
            vertex: Some(
                "layout(set = 2, binding = 0) uniform Bad { vec4 v; };\n\
                 void sunrise_pack_vertex_init() {}\n"
                    .to_owned(),
            ),
            fragment: None,
        };
        assert!(matches!(
            validate_pack_prelude(&bad_set, &default_layout()),
            Err(PreludeError::InvalidBinding { .. })
        ));

        let no_set = PackPrelude {
            vertex: Some(
                "layout(binding = 0) uniform Bad { vec4 v; };\n\
                 void sunrise_pack_vertex_init() {}\n"
                    .to_owned(),
            ),
            fragment: None,
        };
        assert!(matches!(
            validate_pack_prelude(&no_set, &default_layout()),
            Err(PreludeError::InvalidBinding { .. })
        ));

        let wrong_stage = PackPrelude {
            vertex: Some(
                "layout(set = 1, binding = 3) uniform FragOnly { vec4 v; };\n\
                 void sunrise_pack_vertex_init() { gl_Color = v; }\n"
                    .to_owned(),
            ),
            fragment: None,
        };
        assert_eq!(
            validate_pack_prelude(&wrong_stage, &default_layout()),
            Err(PreludeError::StageVisibility {
                stage: "vertex",
                set: 1,
                binding: 3,
            })
        );
    }

    #[test]
    fn pack_declared_attributes_become_globals_in_prelude_mode_and_dedup_works() {
        let (v, f) = make_pack(
            "#version 150\n\
             in vec4 mc_Entity;\n\
             in vec4 at_tangent;\n\
             in int customMat;\n\
             uniform vec3 modelOffset;\n\
             out vec4 vOut;\n\
             void main() {\n\
                 gl_Position = vec4(gl_Vertex.xyz + modelOffset, 1.0);\n\
                 vOut = vec4(float(customMat), mc_Entity.x, at_tangent.x, 1.0);\n\
             }\n",
            "#version 150\n\
             /* DRAWBUFFERS:0 */\n\
             in vec4 vOut;\n\
             void main() {\n\
                 gl_FragData[0] = vOut;\n\
             }\n",
        );
        let inputs = GbuffersInputs {
            prelude: Some(PackPrelude {
                vertex: Some(
                    "layout(set = 1, binding = 0, std430) readonly buffer Verts { vec4 pos[]; };\n\
                     uniform vec3 modelOffset;\n\
                     int customMat;\n\
                     void sunrise_pack_vertex_init() {\n\
                         gl_Vertex = pos[gl_VertexIndex];\n\
                         customMat = 3;\n\
                     }\n"
                    .to_owned(),
                ),
                fragment: None,
            }),
            ..Default::default()
        };
        let program = translate_gbuffers(
            &v,
            &f,
            Target::Buffers,
            &inputs,
            crate::DEVICELESS_INTERFACE_LIMITS,
        )
        .expect("prelude with pack-declared mc_Entity and at_tangent translates");
        // No real vertex inputs exist because the prelude declared none.
        let ep = program
            .vertex
            .entry_points
            .iter()
            .find(|e| e.stage == ShaderStage::Vertex)
            .unwrap();
        let has_location_input = ep
            .function
            .arguments
            .iter()
            .any(|a| matches!(a.binding, Some(naga::Binding::Location { .. })));
        assert!(!has_location_input, "pack attributes must stay globals");
        // Storage buffer relocated from set 1 -> set 7 (MOD_SET_BASE + 1).
        let has_set7 = program.vertex.global_variables.iter().any(|(_, g)| {
            g.binding
                == Some(naga::ResourceBinding {
                    group: 7,
                    binding: 0,
                })
        });
        assert!(has_set7, "mod set 1 must relocate to set 7");
    }

    #[test]
    fn type_or_function_collision_with_pack_is_refused() {
        let (v, f) = make_pack(
            "#version 150\n\
             in float customMat;\n\
             void main() { gl_Position = gl_Vertex; }\n",
            "#version 150\n\
             /* DRAWBUFFERS:0 */\n\
             vec4 customHelper() { return vec4(1.0); }\n\
             void main() { gl_FragData[0] = customHelper(); }\n",
        );
        let inputs = GbuffersInputs {
            prelude: Some(PackPrelude {
                vertex: Some(
                    "int customMat;\n\
                     void sunrise_pack_vertex_init() { customMat = 1; }\n"
                        .to_owned(),
                ),
                fragment: Some("vec4 customHelper() { return vec4(0.5); }\n".to_owned()),
            }),
            ..Default::default()
        };
        let err = translate_gbuffers(
            &v,
            &f,
            Target::Buffers,
            &inputs,
            crate::DEVICELESS_INTERFACE_LIMITS,
        )
        .err()
        .expect("type and function collisions must fail translation");
        assert!(
            err.iter().any(|e| e.message.contains("customMat")),
            "{err:?}"
        );
        assert!(
            err.iter().any(|e| e.message.contains("customHelper")),
            "{err:?}"
        );
    }

    #[test]
    fn undeclared_pack_name_stays_unknown_variable_error() {
        let (v, f) = make_pack(
            "#version 150\n\
             void main() { gl_Position = vec4(gl_Vertex.xyz + dh_neverDeclared, 1.0); }\n",
            "#version 150\n/* DRAWBUFFERS:0 */\nvoid main() { gl_FragData[0] = vec4(1.0); }\n",
        );
        let inputs = GbuffersInputs {
            prelude: Some(PackPrelude {
                vertex: Some(
                    "void sunrise_pack_vertex_init() { gl_Vertex = vec4(0.0, 0.0, 0.0, 1.0); }\n"
                        .to_owned(),
                ),
                fragment: None,
            }),
            ..Default::default()
        };
        let err = translate_gbuffers(
            &v,
            &f,
            Target::Buffers,
            &inputs,
            crate::DEVICELESS_INTERFACE_LIMITS,
        )
        .err()
        .expect("undeclared pack variable must fail front-end translation");
        assert!(
            err.iter().any(|e| e.message.contains("dh_neverDeclared")),
            "{err:?}"
        );
    }

    #[test]
    fn geometry_stage_program_with_prelude_varyings_is_unbound_without_fallback() {
        use std::io::{Cursor, Write};
        let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let opts = zip::write::SimpleFileOptions::default();
        for (path, body) in [
            (
                "shaders/dh_terrain.vsh",
                "#version 330\nvoid main() { gl_Position = gl_Vertex; }\n",
            ),
            (
                "shaders/dh_terrain.gsh",
                "#version 330\nlayout(triangles) in;\nlayout(triangle_strip, max_vertices = 3) out;\n\
                 void main() { for (int i = 0; i < 3; i++) { gl_Position = gl_in[i].gl_Position; EmitVertex(); } EndPrimitive(); }\n",
            ),
            (
                "shaders/dh_terrain.fsh",
                "#version 330\n/* DRAWBUFFERS:0 */\nvoid main() { gl_FragData[0] = vec4(1.0); }\n",
            ),
            (
                "shaders/gbuffers_terrain.vsh",
                "#version 330\nvoid main() { gl_Position = gl_Vertex; }\n",
            ),
            (
                "shaders/gbuffers_terrain.fsh",
                "#version 330\n/* DRAWBUFFERS:0 */\nvoid main() { gl_FragData[0] = vec4(1.0); }\n",
            ),
            (
                "shaders/final.vsh",
                "#version 120\nvoid main() { gl_Position = gl_Vertex; }\n",
            ),
            (
                "shaders/final.fsh",
                "#version 120\nvoid main() { gl_FragColor = vec4(1.0); }\n",
            ),
        ] {
            zip.start_file(path, opts).unwrap();
            zip.write_all(body.as_bytes()).unwrap();
        }
        let pack =
            Pack::from_zip_bytes("gsh-prelude-test", zip.finish().unwrap().into_inner()).unwrap();
        let mut macros = Macros::standard();
        let (props, _) = crate::read_properties(&pack, &mut macros);
        let inputs = crate::GameInputs {
            mod_programs: vec![crate::ModPackProgramRequest {
                slot: 0,
                programs: vec!["dh_terrain".into(), "gbuffers_terrain".into()],
                group: crate::Group::Gbuffers,
                default_blend: None,
                inputs: GbuffersInputs {
                    prelude: Some(PackPrelude {
                        vertex: Some(
                            "out vec2 customUv;\n\
                             void sunrise_pack_vertex_init() { customUv = vec2(0.0); }\n"
                                .to_owned(),
                        ),
                        fragment: Some(
                            "in vec2 customUv;\nfloat customRead() { return customUv.x; }\n"
                                .to_owned(),
                        ),
                    }),
                    ..Default::default()
                },
            }],
            ..Default::default()
        };
        let post = crate::load_post(&pack, "", &props, &macros, &inputs);
        assert!(
            post.mod_programs[0].program.is_none(),
            "geometry-stage program must leave slot unbound with no fallback stem"
        );
        assert!(
            post.notes.iter().any(|n| n.contains("geometry stage")),
            "expected geometry-stage note: {:?}",
            post.notes
        );
    }
}
