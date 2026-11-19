//! Object-like macros: the ones packs test in `#if` lines and use inside property values
//! (spec 1.5, 1.5.1). The GLSL preprocessor's function-like macros are not needed here.

use std::collections::BTreeMap;

/// The feature flags of `iris.features.*` (spec 3.2) Sunrise has. A pack that requires one of
/// them loads; one it lacks is noted. Sunrise defines no `IRIS_FEATURE_<name>` macro for them
/// (spec OQ-2, docs/cleanroom/README.md): that macro family names another loader, so packs take
/// the path they take without it. `BLOCK_EMISSION_ATTRIBUTE`: terrain's
/// `at_midBlock.w` is the light its block gives. `SEPARATE_HARDWARE_SAMPLERS` (ShaderDoc,
/// "Separate Hardware Shadow Samplers"): `shadowtex0`/`shadowtex1` stay plain depth samplers when
/// hardware filtering is on, `shadowtex0HW`/`shadowtex1HW` compare; Sunrise picks a comparison
/// sampler by the declared type (`sampler2DShadow`), so they always are. `HIGHER_SHADOWCOLOR`:
/// `shadowcolor2` to `shadowcolor7` beside the first two. `FADE_VARIABLE`: terrain vertex
/// programs can read the section's visibility transition as `mc_chunkFade`.
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

/// Whether `name` is one of [`FEATURES`], in any case.
fn is_feature(name: &str) -> bool {
    FEATURES.iter().any(|f| f.eq_ignore_ascii_case(name))
}

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

#[derive(Clone, Debug, PartialEq)]
struct Macro {
    /// `None` for an object-like macro; the parameter names of a function-like one.
    params: Option<Vec<String>>,
    body: String,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Macros(BTreeMap<String, Macro>);

/// The most one expansion may make: each macro it replaces counts one, and each token of the
/// replacement one, plus one for each name that token hides. Past it the expansion stops, so a
/// pack's macros are bounded before what they make is held (macros that each use the next twice
/// double the text at every level).
pub(crate) const MAX_EXPANSION: usize = 1 << 18;

/// How deep calls may nest in calls' arguments, each expanded by recursion before the call.
const MAX_CALL_DEPTH: usize = 64;

/// The bytes an expansion may make when its caller holds it to nothing smaller: what naga may
/// read of a stage.
pub(crate) const MAX_EXPANSION_BYTES: usize = sunrise_shader_api::guard::MAX_GLSL_BYTES;

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
        // No `IS_IRIS`, `IRIS_VERSION` or `IRIS_HAS_*`: Sunrise is not that loader and does not
        // claim its identity (spec OQ-2, docs/cleanroom/README.md). Packs take their other path.
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

    /// A note for each feature `iris.features.required` names that Sunrise lacks; a name matches
    /// in any case. Optional features need nothing: no macro is defined for them (see `FEATURES`).
    pub fn feature_notes(properties: &crate::Properties) -> Vec<String> {
        properties
            .get("iris.features.required")
            .unwrap_or_default()
            .split_whitespace()
            .filter(|f| !is_feature(f))
            .map(|f| format!("the pack requires `{f}`, which is not supported yet"))
            .collect()
    }

    pub fn define(&mut self, name: impl Into<String>, value: impl Into<String>) {
        let body = value.into();
        self.0.insert(name.into(), Macro { params: None, body });
    }

    /// The rest of a `#define` line: `NAME body`, or `NAME(a, b) body` (no space before the `(`)
    /// for a function-like macro.
    pub(crate) fn define_line(&mut self, rest: &str) -> Result<(), String> {
        let len = rest
            .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
            .unwrap_or(rest.len());
        let (name, tail) = rest.split_at(len);
        if name.is_empty() {
            return Err("`#define` without a name".into());
        }
        if let Some(params) = tail.strip_prefix('(') {
            let Some((params, body)) = params.split_once(')') else {
                return Err(format!("`#define {name}(` without `)`"));
            };
            let params = params
                .split(',')
                .map(|p| p.trim().to_owned())
                .filter(|p| !p.is_empty())
                .collect();
            self.define_function(name, params, body.trim());
        } else {
            self.define(name, tail.trim());
        }
        Ok(())
    }

    pub fn define_function(&mut self, name: &str, params: Vec<String>, body: &str) {
        let body = body.to_owned();
        self.0.insert(
            name.to_owned(),
            Macro {
                params: Some(params),
                body,
            },
        );
    }

    pub fn undef(&mut self, name: &str) {
        self.0.remove(name);
    }

    pub fn is_defined(&self, name: &str) -> bool {
        self.0.contains_key(name)
    }

    /// The value of an object-like macro.
    pub fn get(&self, name: &str) -> Option<&str> {
        self.0
            .get(name)
            .filter(|m| m.params.is_none())
            .map(|m| m.body.as_str())
    }

    /// Whether upcoming source lines continue a call, including one whose expanded name
    /// ends `text` and whose opening parenthesis is on the next nonempty line. The expansion
    /// looked at is held to `max_bytes`, as [`Macros::expand_bounded`]'s.
    pub(crate) fn unfinished_call(&self, text: &str, upcoming: &[&str], max_bytes: usize) -> bool {
        let mut budget = Budget::new(text, max_bytes);
        // A line the budget refuses is not lexed: `expand_bounded` reports it.
        if budget.stopped() {
            return false;
        }
        let tokens = expansion_tokens(text, &mut budget);
        let (tokens, unfinished) = self.expand_tokens(tokens, &mut budget, 0);
        unfinished
            || (tokens.iter().rfind(|t| !t.blank()).is_some_and(|t| {
                self.0.get_key_value(&t.text).is_some_and(|(name, m)| {
                    m.params.is_some() && !t.hidden.contains(&name.as_str())
                })
            }) && upcoming
                .iter()
                .map(|line| line.trim_start())
                .find(|line| !line.is_empty())
                .is_some_and(|line| line.starts_with('(')))
    }

    /// `text` with every macro expanded, repeatedly; a macro is not expanded again inside its own
    /// expansion, so `A` defined as `A + 1` stops. Function-like macros take their arguments
    /// (expanded first, raw beside `##`, which pastes) when a `(` follows their name. `Err` names
    /// the bound the expansion gets past: [`MAX_EXPANSION`] tokens, or `max_bytes`, which every
    /// copy of text it makes is paid from before it is made, so the text is never longer.
    pub fn expand_bounded(&self, text: &str, max_bytes: usize) -> Result<String, Bound> {
        let mut budget = Budget::new(text, max_bytes);
        if let Some(bound) = budget.past {
            return Err(bound);
        }
        let tokens = expansion_tokens(text, &mut budget);
        let tokens = self.expand_tokens(tokens, &mut budget, 0).0;
        let mut out = String::new();
        let mut prev = "";
        for t in &tokens {
            // Tokens the expansion made adjacent are kept apart where they would read as one
            // (`-` `-`); source tokens and explicit pastes stay as written. The space is paid for.
            if t.apart && merges(prev, &t.text) && budget.spend(0, 1) {
                out.push(' ');
            }
            out.push_str(&t.text);
            prev = &t.text;
        }
        match budget.past {
            Some(bound) => Err(bound),
            None => Ok(out),
        }
    }

    /// [`Macros::expand_bounded`] within [`MAX_EXPANSION_BYTES`], or `text` as it is when the
    /// expansion gets past a bound.
    pub fn expand(&self, text: &str) -> String {
        self.expand_bounded(text, MAX_EXPANSION_BYTES)
            .unwrap_or_else(|_| text.to_owned())
    }

    /// The expansion of `tokens`, an argument `depth` calls deep, spending `budget`; it stops
    /// where it is when the budget runs out or the calls nest too deep, and leaves it stopped.
    fn expand_tokens<'a>(
        &'a self,
        mut tokens: Vec<ExpansionToken<'a>>,
        budget: &mut Budget,
        depth: usize,
    ) -> (Vec<ExpansionToken<'a>>, bool) {
        if depth > MAX_CALL_DEPTH {
            budget.stop(Bound::Tokens);
        }
        let mut i = 0;
        let mut unfinished = false;
        while i < tokens.len() && !budget.stopped() {
            let Some((name, m)) = self.0.get_key_value(tokens[i].text.as_str()) else {
                i += 1;
                continue;
            };
            if tokens[i].hidden.contains(&name.as_str()) {
                i += 1;
                continue;
            }
            let mut hidden = tokens[i].hidden.clone();
            let (mut body, next) = match &m.params {
                None => {
                    // The replacement is a copy of the body: paid for first.
                    if !budget.spend(0, m.body.len()) {
                        break;
                    }
                    (expansion_tokens(&m.body, budget), i + 1)
                }
                Some(params) => {
                    let Some((args, next)) = arguments(&tokens, i + 1) else {
                        unfinished |= tokens[i + 1..]
                            .iter()
                            .find(|t| !t.blank())
                            .is_some_and(|t| t.text == "(");
                        i += 1;
                        continue;
                    };
                    // The arguments are copied where they are used, expanded by recursion:
                    // paid for first.
                    if !budget.spend(args.iter().map(|a| a.len()).sum(), 0) {
                        break;
                    }
                    // A call can cross a replacement boundary. Only disabling shared by
                    // its name and closing parenthesis carries into the new replacement.
                    hidden.retain(|h| tokens[next - 1].hidden.contains(h));
                    let Some(body) = self.substitute(params, &args, &m.body, budget, depth) else {
                        budget.stop(Bound::Tokens);
                        break;
                    };
                    (body, next)
                }
            };
            hidden.push(name);
            // The replacement and the names each of its tokens will hide.
            if !budget.spend(1 + body.len() * (1 + hidden.len()), 0) {
                break;
            }
            for token in &mut body {
                for h in &hidden {
                    if !token.hidden.contains(h) {
                        token.hidden.push(h);
                    }
                }
            }
            // Rescan the replacement together with the remaining source. Its final name
            // may consume a call there; source tokens keep their own disabling context.
            let end = i + body.len();
            tokens.splice(i..next, body);
            // The replacement's ends now touch tokens they were not written beside.
            for k in [i, end] {
                if let Some(t) = tokens.get_mut(k) {
                    t.apart = true;
                }
            }
        }
        (tokens, unfinished)
    }

    /// A function-like macro's body with its parameters replaced, for a call `depth` calls deep;
    /// `None` when that alone gets past `budget`, which the arguments' own expansions spend.
    fn substitute<'a>(
        &'a self,
        params: &[String],
        args: &[&[ExpansionToken<'a>]],
        body: &str,
        budget: &mut Budget,
        depth: usize,
    ) -> Option<Vec<ExpansionToken<'a>>> {
        let tokens = lex_within(body, budget.tokens)?;
        let pasted = |at: usize, step: isize| {
            let mut j = at as isize + step;
            while (0..tokens.len() as isize).contains(&j) && tokens[j as usize].trim().is_empty() {
                j += step;
            }
            (0..tokens.len() as isize).contains(&j) && tokens[j as usize] == "##"
        };
        let mut out = Vec::new();
        // The token after an argument touches it without being written beside it.
        let mut after_arg = false;
        for (at, token) in tokens.iter().enumerate() {
            match params.iter().position(|p| p == token) {
                Some(k) => {
                    let arg = args.get(k).copied().unwrap_or_default();
                    let from = arg.iter().position(|t| !t.blank()).unwrap_or(arg.len());
                    let end = arg.iter().rposition(|t| !t.blank()).map_or(from, |n| n + 1);
                    // An argument used many times multiplies: each copy is paid for before it
                    // is made, and its expansion as it grows.
                    let bytes = arg[from..end].iter().map(|t| t.text.len()).sum();
                    if !budget.spend(0, bytes) {
                        return None;
                    }
                    let arg = arg[from..end].to_vec();
                    let first = out.len();
                    after_arg = true;
                    if pasted(at, -1) || pasted(at, 1) {
                        if arg.is_empty() {
                            // An empty paste operand must not capture the preceding token.
                            out.push(ExpansionToken::new(""));
                        } else {
                            out.extend(arg);
                        }
                    } else {
                        out.extend(self.expand_tokens(arg, budget, depth + 1).0);
                    }
                    if let Some(t) = out.get_mut(first) {
                        t.apart = true;
                    }
                    if out.len() >= budget.tokens {
                        return None;
                    }
                }
                None if budget.spend(0, token.len()) => {
                    let mut token = ExpansionToken::new(token);
                    token.apart = std::mem::take(&mut after_arg);
                    out.push(token);
                }
                None => return None,
            }
        }
        // `a ## b` → `ab`.
        let mut joined: Vec<ExpansionToken<'a>> = Vec::new();
        let mut paste = false;
        for token in out {
            if token.text == "##" {
                while joined.last().is_some_and(|t| t.blank()) {
                    joined.pop();
                }
                paste = true;
            } else if paste && token.blank() {
                continue;
            } else if paste {
                if let Some(mut left) = joined.pop() {
                    // An empty operand leaves the other token, including its hiding, intact.
                    if left.text.is_empty() {
                        left = token;
                    } else if !token.text.is_empty() {
                        // The right token is copied onto the left one: paid for first.
                        if !budget.spend(0, token.text.len()) {
                            return None;
                        }
                        left.text.push_str(&token.text);
                        left.hidden.retain(|h| token.hidden.contains(h));
                    }
                    joined.push(left);
                } else {
                    joined.push(token);
                }
                paste = false;
            } else {
                joined.push(token);
            }
        }
        joined.retain(|t| !t.text.is_empty());
        // A paste that makes no single token (`b ## (`, Alpha Piscium's
        // `colors2_colorspaces_ ## a ## _to_ ## b ##(x)`) stays the tokens it is made of, as
        // GL drivers' preprocessors leave it: the name before the `(` is then rescanned and can
        // call a macro. The parts are copies, paid for first.
        let mut split = Vec::with_capacity(joined.len());
        for token in joined {
            let parts = lex(&token.text);
            if parts.len() < 2 {
                split.push(token);
                continue;
            }
            if !budget.spend(0, token.text.len()) {
                return None;
            }
            // Only the first part keeps its place; the rest stay joined as pasted.
            split.extend(
                parts
                    .into_iter()
                    .enumerate()
                    .map(|(k, text)| ExpansionToken {
                        text: text.into(),
                        hidden: token.hidden.clone(),
                        apart: token.apart && k == 0,
                    }),
            );
        }
        Some(split)
    }
}

/// The bound an expansion gets past.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Bound {
    /// It makes more than [`MAX_EXPANSION`] tokens, or nests calls in arguments too deep.
    Tokens,
    /// Its text grows past the bytes its caller allows.
    Bytes,
}

/// What an expansion may still make: [`MAX_EXPANSION`] tokens, and the bytes its caller allows,
/// from which every copy of text (the text's own, a replacement, an argument, a paste) is paid
/// before it is made.
struct Budget {
    tokens: usize,
    bytes: usize,
    /// The bound it got past, once it has: the expansion stops.
    past: Option<Bound>,
}

impl Budget {
    /// `max_bytes` with `text` paid for: it is copied into tokens first.
    fn new(text: &str, max_bytes: usize) -> Self {
        let mut budget = Self {
            tokens: MAX_EXPANSION,
            bytes: max_bytes,
            past: None,
        };
        budget.spend(0, text.len());
        budget
    }

    /// Takes `tokens` and `bytes`, or stops the expansion when either does not fit.
    fn spend(&mut self, tokens: usize, bytes: usize) -> bool {
        match (
            self.tokens.checked_sub(tokens),
            self.bytes.checked_sub(bytes),
        ) {
            (Some(tokens), Some(bytes)) if tokens > 0 => {
                (self.tokens, self.bytes) = (tokens, bytes);
                true
            }
            (Some(tokens), None) if tokens > 0 => {
                self.stop(Bound::Bytes);
                false
            }
            _ => {
                self.stop(Bound::Tokens);
                false
            }
        }
    }

    /// Stops the expansion, which keeps the first bound it got past.
    fn stop(&mut self, bound: Bound) {
        (self.tokens, self.bytes) = (0, 0);
        self.past.get_or_insert(bound);
    }

    fn stopped(&self) -> bool {
        self.past.is_some()
    }
}

/// Disabling belongs to each token: rescanning a boundary must neither revive a recursive
/// replacement token nor hide an independent invocation in the surrounding source.
#[derive(Clone)]
struct ExpansionToken<'a> {
    text: String,
    hidden: Vec<&'a str>,
    /// Made adjacent to the token before it by the expansion, not written beside it.
    apart: bool,
}

impl ExpansionToken<'_> {
    fn new(text: &str) -> Self {
        Self {
            text: text.into(),
            hidden: Vec::new(),
            apart: false,
        }
    }

    fn blank(&self) -> bool {
        !self.text.is_empty() && self.text.trim().is_empty()
    }
}

/// `text` as tokens, or none (the budget stopped) when it has more than `budget` has left: the
/// lexer stops there, before it makes a token past it.
fn expansion_tokens<'a>(text: &'a str, budget: &mut Budget) -> Vec<ExpansionToken<'a>> {
    let Some(tokens) = lex_within(text, budget.tokens) else {
        budget.stop(Bound::Tokens);
        return Vec::new();
    };
    tokens.into_iter().map(ExpansionToken::new).collect()
}

/// Operators of more than one character, which two tokens written together would make.
const OPERATORS: [&str; 23] = [
    "++", "--", "+=", "-=", "*=", "/=", "%=", "&=", "|=", "^=", "<<", ">>", "<=", ">=", "==", "!=",
    "&&", "||", "^^", "->", "//", "/*", "*/",
];

/// Whether `a` then `b`, written together, would not read as the two tokens they are.
fn merges(a: &str, b: &str) -> bool {
    if a.trim().is_empty() || b.trim().is_empty() {
        return false;
    }
    let joined = [a, b].concat();
    lex(&joined) != [a, b] || OPERATORS.contains(&joined.as_str())
}

/// The arguments of a call whose `(` is the next non-blank token from `i`, split at top-level
/// commas, and the index after its `)`. `None` when no `(` follows or it never closes. They are
/// borrowed: each use copies one.
fn arguments<'t, 'a>(
    tokens: &'t [ExpansionToken<'a>],
    mut i: usize,
) -> Option<(Vec<&'t [ExpansionToken<'a>]>, usize)> {
    while tokens.get(i)?.blank() {
        i += 1;
    }
    if tokens[i].text != "(" {
        return None;
    }
    let (mut depth, mut args, mut from) = (0, Vec::new(), i + 1);
    for (j, t) in tokens.iter().enumerate().skip(i + 1) {
        match t.text.as_str() {
            "(" => depth += 1,
            ")" if depth == 0 => {
                args.push(&tokens[from..j]);
                return Some((args, j + 1));
            }
            ")" => depth -= 1,
            "," if depth == 0 => {
                args.push(&tokens[from..j]);
                from = j + 1;
            }
            _ => {}
        }
    }
    None
}

/// Preprocessing tokens: identifiers, numbers (with their suffixes and exponents), runs of
/// white space, `##`, and single characters. Concatenated they give back `text`.
pub(crate) fn lex(text: &str) -> Vec<&str> {
    lex_within(text, usize::MAX).unwrap_or_default()
}

/// [`lex`], or `None` when `text` has more than `max` tokens: found before one past `max` is kept.
fn lex_within(text: &str, max: usize) -> Option<Vec<&str>> {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i] as char;
        let start = i;
        i += 1;
        if is_ident_start(c) {
            while i < bytes.len() && is_ident(bytes[i] as char) {
                i += 1;
            }
        } else if c.is_ascii_digit() || (c == '.' && bytes.get(i).is_some_and(u8::is_ascii_digit)) {
            while i < bytes.len() {
                let d = bytes[i] as char;
                let exp_sign = (d == '+' || d == '-') && matches!(bytes[i - 1], b'e' | b'E');
                if !(is_ident(d) || d == '.' || exp_sign) {
                    break;
                }
                i += 1;
            }
        } else if c.is_whitespace() {
            while i < bytes.len() && (bytes[i] as char).is_whitespace() {
                i += 1;
            }
        } else if c == '#' && bytes.get(i) == Some(&b'#') {
            i += 1;
        } else {
            while !text.is_char_boundary(i) {
                i += 1;
            }
        }
        if out.len() == max {
            return None;
        }
        out.push(&text[start..i]);
    }
    Some(out)
}

pub(crate) fn is_ident_start(c: char) -> bool {
    c.is_ascii_alphabetic() || c == '_'
}

pub(crate) fn is_ident(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
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
    /// case it is written in; no `IRIS_FEATURE_*` macro is defined either way (spec OQ-2).
    #[test]
    fn required_features_are_noted_and_never_defined() {
        let properties = crate::Properties::parse(
            "iris.features.optional = COMPUTE_SHADERS ssbo NOT_A_FEATURE\n\
             iris.features.required = BLOCK_EMISSION_ATTRIBUTE HIGHER_SHADOWCOLOR custom_images \
             Not_A_Feature\n",
            &Macros::standard(),
        );
        assert_eq!(
            Macros::feature_notes(&properties),
            ["the pack requires `Not_A_Feature`, which is not supported yet"]
        );
        let mut m = Macros::standard();
        m.define_renderer(4);
        let iris: Vec<_> = m.0.keys().filter(|name| name.contains("IRIS")).collect();
        assert!(iris.is_empty(), "{iris:?}");
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

    #[test]
    fn expansion_replaces_whole_words_and_stops_on_recursion() {
        let mut m = Macros::default();
        m.define("RES", "SIZE");
        m.define("SIZE", "512");
        m.define("LOOP", "LOOP + 1");
        assert_eq!(m.expand("RES RES 0.5"), "512 512 0.5");
        assert_eq!(
            m.expand("RESOLUTION colortex4 LOOP"),
            "RESOLUTION colortex4 LOOP + 1"
        );
        m.define("e5", "x");
        assert_eq!(m.expand("1e5"), "1e5");
    }

    #[test]
    fn function_like_macros_take_arguments_and_paste() {
        let mut m = Macros::default();
        m.define("N", "4");
        m.define_function("SQ", vec!["x".into()], "((x) * (x))");
        m.define_function("CAT", vec!["a".into(), "b".into()], "a ## b");
        m.define_function("PI", vec![], "3.14");
        assert_eq!(m.expand("SQ(N + 1)"), "((4 + 1) * (4 + 1))");
        assert_eq!(m.expand("SQ(min(N, 2))"), "((min(4, 2)) * (min(4, 2)))");
        assert_eq!(m.expand("CAT(N, 2) CAT(col, ortex4)"), "N2 colortex4");
        assert_eq!(m.expand("PI() SQ SQ(1"), "3.14 SQ SQ(1");
        assert_eq!(m.get("SQ"), None);
    }

    /// A line the budget refuses (its bytes, or more tokens than it has left) is not lexed by the
    /// lookahead, whatever follows it, and the lexer stops at the budget.
    #[test]
    fn a_call_lookahead_past_the_budget_stops_before_it_lexes() {
        let mut m = Macros::default();
        m.define_function("F", vec!["x".into()], "x");
        let (open, long) = (["(1)"], "+ ".repeat(MAX_EXPANSION) + "F");
        assert!(m.unfinished_call("F", &open, 1 << 20));
        // Past its bytes: refused before it is lexed.
        assert!(!m.unfinished_call("F", &open, 0));
        // Past its tokens, within its bytes: the lexer stops at the budget.
        assert!(!m.unfinished_call(&long, &open, 1 << 30));
        assert_eq!(m.expand_bounded(&long, 1 << 30), Err(Bound::Tokens));
        assert_eq!(lex_within("a b c", 5).map(|t| t.len()), Some(5));
        assert!(lex_within("a b c", 4).is_none());
    }

    /// Tokens the expansion made adjacent are kept apart where they would read as one; source
    /// tokens and explicit pastes are not.
    #[test]
    fn expanded_tokens_are_separated_where_joining_them_changes_how_they_read() {
        let mut m = Macros::default();
        m.define("E", "");
        m.define("P", "+");
        m.define("N", "1");
        m.define_function("NEG", vec!["x".into()], "-x");
        m.define_function("CAT", vec!["a".into(), "b".into()], "a ## b");
        assert_eq!(m.expand("a+P"), "a+ +");
        assert_eq!(m.expand("NEG(-N)"), "- -1");
        assert_eq!(m.expand("-E-"), "- -");
        assert_eq!(m.expand("NEG(N)N"), "-1 1");
        assert_eq!(m.expand("i++ - -j"), "i++ - -j");
        assert_eq!(m.expand("CAT(+, +) CAT(-, >) CAT(a, b)"), "++ -> ab");
    }
}
