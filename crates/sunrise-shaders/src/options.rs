//! Switch options (spec 8.1), as far as program-enable expressions (3.3) need them: which are on
//! by default. A switch is `#define NAME` (on) or `//#define NAME` (off) with nothing after the
//! name but a comment, in any stage or included file of the pack. Packs keep their options in
//! included `.glsl` files (or files of any name a source includes) and test them elsewhere
//! (OQ-13), so the test the spec asks for (`#ifdef NAME` / `#ifndef NAME`) may be in any file
//! of the pack. An include guard (`#ifndef NAME` then `#define NAME`, 1.4) is not a switch: one
//! scan decides what declares an option, for the options list and the switches alike. It reads
//! each line's code outside block comments, as the GLSL preprocessor does, and reads the pack as
//! written: the player's chosen values apply afterwards, to the options it found.

use std::collections::{BTreeMap, BTreeSet};

use crate::Pack;
use crate::macros::FEATURE_LISTS;

const SOURCES: [&str; 8] = ["vsh", "fsh", "gsh", "csh", "tcs", "tes", "glsl", "inc"];

/// The switch options that are on: the switches the scan [`options`] reads finds, tested with
/// `#ifdef NAME` or `#ifndef NAME` somewhere (8.1), at the player's chosen value or else at
/// their default. An include guard is not one, as it is not an option.
pub fn switches_on(pack: &Pack) -> BTreeSet<String> {
    let scan = scan(pack);
    let chosen = pack.chosen();
    scan.on
        .iter()
        .chain(&scan.off)
        .filter(|name| scan.switch_tested.contains(*name))
        .filter(|name| match chosen.get(*name).map(String::as_str) {
            Some("true") => true,
            Some("false") => false,
            // Defined both ways is ambiguous (8.1): it keeps its default only where it is on
            // everywhere.
            _ => scan.on.contains(*name) && !scan.off.contains(*name),
        })
        .cloned()
        .collect()
}

/// The name a line tests with `#ifdef NAME` or `#ifndef NAME` (any space after `#`), as
/// [`switches_on`] counts a test.
fn switch_test(line: &str) -> Option<&str> {
    let rest = line.trim().strip_prefix('#')?.trim_start();
    let (directive, rest) = rest.split_once(|c: char| c.is_whitespace())?;
    matches!(directive, "ifdef" | "ifndef")
        .then(|| rest.split_whitespace().next())
        .flatten()
}

/// The value options and their defaults: `#define NAME VALUE // [A B C]`, a value and the list
/// of the values it may take (8.1).
pub fn value_options(pack: &Pack) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for text in option_texts(pack, Pack::read_text) {
        for line in text.lines() {
            let Some(rest) = line.trim().strip_prefix('#') else {
                continue;
            };
            let Some(rest) = rest.trim_start().strip_prefix("define") else {
                continue;
            };
            let Some((code, comment)) = rest.split_once("//") else {
                continue;
            };
            let mut words = code.split_whitespace();
            let (Some(name), Some(value), None) = (words.next(), words.next(), words.next()) else {
                continue;
            };
            if comment.contains('[') && comment.contains(']') && !crate::macros::is_reserved(name) {
                out.entry(name.to_owned())
                    .or_insert_with(|| value.to_owned());
            }
        }
    }
    out
}

/// `macros` with the pack's options at their defaults: the switches that are on, and the
/// value options. `shaders.properties` and the program switches test them (3.3, 8.1). No
/// `IRIS_FEATURE_*`, `IS_IRIS` or `IRIS_VERSION` is among them (`is_reserved`): those macros are
/// Sunrise's answers (`Macros::define_features`, `Macros::set_identity`; decisions D-7, D-8).
pub fn with_options(pack: &Pack, macros: &crate::Macros) -> crate::Macros {
    let mut out = macros.clone();
    for name in switches_on(pack) {
        out.define(name, "");
    }
    for (name, value) in value_options(pack) {
        out.define(name, value);
    }
    out
}

/// A pack's `shaders.properties` as the game reads it before the programs, with the pack's
/// options over `macros` (the standard set and the renderer's), in two reads (spec 3.2,
/// decision D-7): the first, with no `IRIS_FEATURE_*`, settles the flags the pack lists;
/// `Macros::define_features` then defines the answers in `macros` (any `IRIS_FEATURE_*` it held
/// before is dropped first) and in the option set, and the second read, the one returned, sees
/// them. The file and the pack's options are read once, for both reads. A flag list only a feature macro reaches is not read, and the
/// returned properties hold the two lists as the first read gave them, so whatever decides by
/// them (`load_post`'s shadow colour buffers) follows the answers the programs got. Returns the
/// properties and a note for each required feature Sunrise lacks (`Macros::feature_notes`, of
/// the first read). Whatever reads a pack's programs and id files (the game, the `real_packs`
/// test) reads them with `macros` as this leaves it.
pub fn read_properties(
    pack: &Pack,
    macros: &mut crate::Macros,
) -> (crate::Properties, Vec<String>) {
    // The answers start from none: a set the caller reuses may hold another pack's.
    macros.drop_feature_answers();
    // The file and the options are read once, so both reads see the same pack.
    let text = pack.read_text("shaders.properties").unwrap_or_default();
    let mut options = with_options(pack, macros);
    let listed = crate::Properties::parse(&text, &options);
    let notes = macros.feature_notes(&listed);
    if !macros.define_features(&listed) {
        return (listed, notes);
    }
    // No option is named as an answer (`is_reserved`), so adding the answers to the
    // options read once is the same as the options over the answered `macros`.
    options.define_features(&listed);
    let mut properties = crate::Properties::parse(&text, &options);
    // The settled lists, back in file order: each before the first entry of a later line.
    for key in FEATURE_LISTS {
        properties.entries.retain(|e| e.key != key);
        if let Some(entry) = listed.entry(key) {
            let at = (properties.entries.iter()).position(|e| e.line > entry.line);
            let at = at.unwrap_or(properties.entries.len());
            properties.entries.insert(at, entry.clone());
        }
    }
    // And the first read's notes that one of them was set again.
    let settled = |key: Option<&String>| key.is_some_and(|k| FEATURE_LISTS.contains(&k.as_str()));
    properties.notes.retain(|n| !settled(n.repeated.as_ref()));
    let repeated = listed.notes.iter().filter(|n| settled(n.repeated.as_ref()));
    properties.notes.extend(repeated.cloned());
    properties.notes.sort_by_key(|n| n.line);
    (properties, notes)
}

/// An option the player can change (8.1): a switch (`values` are `false` and `true`), a value
/// option or a `const` with the list of values it may take.
#[derive(Clone, Debug, PartialEq)]
pub struct PackOption {
    pub name: String,
    pub default: String,
    pub values: Vec<String>,
    pub switch: bool,
    /// A switch some file tests; value options always count as tested.
    pub tested: bool,
}

/// A line that declares an option, as [`declaration`] reads it.
struct Declaration<'a> {
    name: &'a str,
    /// The value and its byte offset in the line; `None` for a switch.
    value: Option<(&'a str, usize)>,
    /// Behind `//`: a switch that is off.
    commented: bool,
    /// The values in the comment's `[...]`.
    values: Option<Vec<&'a str>>,
    /// `const bool`: a switch written as a value.
    boolean: bool,
}

/// `#define NAME`, `//#define NAME`, `#define NAME VALUE // [A B]`,
/// `const TYPE NAME = VALUE; // [A B]` or `const bool NAME = true;`.
fn declaration(line: &str) -> Option<Declaration<'_>> {
    let body = line.trim_start();
    let (commented, code) = match body.strip_prefix("//") {
        Some(rest) => (true, rest.trim_start()),
        None => (false, body),
    };
    let offset = |part: &str| part.as_ptr() as usize - line.as_ptr() as usize;
    if let Some(rest) = code.strip_prefix('#') {
        let rest = rest.trim_start().strip_prefix("define")?;
        if !rest.starts_with(char::is_whitespace) {
            return None;
        }
        let (code, comment) = rest.split_once("//").unwrap_or((rest, ""));
        let mut words = code.split_whitespace();
        let name = words.next()?;
        let value = words.next();
        if words.next().is_some() || crate::macros::is_reserved(name) {
            return None;
        }
        return Some(Declaration {
            name,
            value: value.map(|v| (v, offset(v))),
            commented,
            values: list_with_default(comment, value),
            boolean: false,
        });
    }
    if commented {
        return None;
    }
    // `const float shadowDistance = 128.0; // [64.0 128.0]`, `const bool x = true;`
    let rest = code.strip_prefix("const")?;
    let (decl, comment) = rest.split_once("//").unwrap_or((rest, ""));
    let (left, right) = decl.split_once('=')?;
    let mut words = left.split_whitespace();
    let (Some(ty), Some(name), None) = (words.next(), words.next(), words.next()) else {
        return None;
    };
    if crate::macros::is_reserved(name) {
        return None;
    }
    let value = right.trim().strip_suffix(';')?.trim_end();
    if value.is_empty() || value.contains(char::is_whitespace) {
        return None;
    }
    let boolean = ty == "bool" && matches!(value, "true" | "false");
    Some(Declaration {
        name,
        value: Some((value, offset(value))),
        commented: false,
        values: if boolean {
            Some(vec!["false", "true"])
        } else {
            Some(list_with_default(comment, Some(value))?)
        },
        boolean,
    })
}

/// Keep the declared default selectable without creating an option from an empty range.
fn list_with_default<'a>(comment: &'a str, default: Option<&'a str>) -> Option<Vec<&'a str>> {
    let mut values = list(comment)?;
    if !values.is_empty()
        && let Some(default) = default
        && !values.contains(&default)
    {
        values.insert(0, default);
    }
    Some(values)
}

/// The values in a comment's `[...]`.
fn list(comment: &str) -> Option<Vec<&str>> {
    let (_, rest) = comment.split_once('[')?;
    let (inner, _) = rest.split_once(']')?;
    Some(inner.split_whitespace().collect())
}

/// What one pass over the pack's option texts, as written, finds: the one place that decides
/// what declares an option, so [`options`] and [`switches_on`] agree on it.
struct Scan {
    /// The value and `const` options, by name.
    values: BTreeMap<String, PackOption>,
    /// The switches declared on (`#define NAME`) and off (`//#define NAME`); include guards are
    /// in neither.
    on: BTreeSet<String>,
    off: BTreeSet<String>,
    /// The names an `#ifdef`, `#ifndef`, `#if` or `#elif` line tests, as [`options`] counts it.
    tested: BTreeSet<String>,
    /// The names an `#ifdef` or `#ifndef` line tests, as [`switches_on`] counts it.
    switch_tested: BTreeSet<String>,
}

fn scan(pack: &Pack) -> Scan {
    let mut scan = Scan {
        values: BTreeMap::new(),
        on: BTreeSet::new(),
        off: BTreeSet::new(),
        tested: BTreeSet::new(),
        switch_tested: BTreeSet::new(),
    };
    // The names whose `#define NAME` closes a guard.
    let mut guarded = BTreeSet::new();
    for text in option_texts(pack, Pack::read_text_as_written) {
        let mut lines = Lines::default();
        for raw in text.lines() {
            let line = lines.read(raw);
            let directive = line.directive();
            for prefix in ["#ifdef", "#ifndef"] {
                if let Some(name) = directive.strip_prefix(prefix).map(str::trim) {
                    let name = name.split_whitespace().next().unwrap_or("");
                    scan.tested.insert(name.to_owned());
                }
            }
            if directive.starts_with("#if") || directive.starts_with("#elif") {
                scan.tested.extend(defined_names(directive));
            }
            if let Some(name) = switch_test(directive) {
                scan.switch_tested.insert(name.to_owned());
            }
            let d = declaration(&line.code);
            if lines.closes_guard(&line, d.as_ref()) {
                guarded.extend(d.map(|d| d.name.to_owned()));
                continue;
            }
            let Some(d) = d else { continue };
            match (d.value, d.values) {
                (None, _) => {
                    let set = if d.commented {
                        &mut scan.off
                    } else {
                        &mut scan.on
                    };
                    set.insert(d.name.to_owned());
                }
                (Some((value, _)), Some(values)) if !d.commented && !values.is_empty() => {
                    // A `const bool` is often the pack's own setting rather than the player's:
                    // it shows where a screen names it, not with `*`.
                    scan.values
                        .entry(d.name.to_owned())
                        .or_insert_with(|| PackOption {
                            name: d.name.to_owned(),
                            default: value.to_owned(),
                            values: values.iter().map(|v| (*v).to_owned()).collect(),
                            switch: d.boolean,
                            tested: !d.boolean,
                        });
                }
                _ => {}
            }
        }
    }
    // A name the pack also declares off (`//#define NAME`) anywhere is a switch the player can
    // turn on, never a guard: turned on, its line reads `#define NAME` after the `#ifndef NAME`.
    for name in guarded {
        if scan.off.contains(&name) {
            scan.on.insert(name);
        }
    }
    scan
}

/// One line of an option text: its code, block comments blanked to spaces so that byte offsets
/// stay those of the line, and where a `//` comment starts in it.
struct Line {
    code: String,
    comment_at: usize,
}

impl Line {
    /// The code before any `//` comment, trimmed: what a directive is read from.
    fn directive(&self) -> &str {
        self.code[..self.comment_at].trim()
    }
}

/// Reads a text's lines in order: a block comment carries from line to line, and an
/// `#ifndef NAME` waits for its `#define NAME` past comment lines.
#[derive(Default)]
struct Lines {
    in_block: bool,
    guard: Option<String>,
}

impl Lines {
    /// `raw` with its block comments blanked, walking each comment delimiter once.
    fn read(&mut self, raw: &str) -> Line {
        let mut code = raw.as_bytes().to_vec();
        let mut comment_at = code.len();
        let mut i = 0;
        while i < code.len() {
            let pair = code.get(i..i + 2);
            if self.in_block {
                if pair == Some(b"*/") {
                    self.in_block = false;
                    code[i..i + 2].fill(b' ');
                    i += 2;
                } else {
                    code[i] = b' ';
                    i += 1;
                }
            } else if pair == Some(b"//") {
                comment_at = i;
                break;
            } else if pair == Some(b"/*") {
                self.in_block = true;
                code[i..i + 2].fill(b' ');
                i += 2;
            } else {
                i += 1;
            }
        }
        // Only whole characters were blanked: a comment starts and ends at ASCII bytes.
        let code = String::from_utf8(code).unwrap_or_default();
        Line { code, comment_at }
    }

    /// Whether `d`, read from `line`, is the `#define NAME` that closes an include guard:
    /// `#ifndef NAME` (or `#if !defined NAME`) just before it, with only comment lines between.
    fn closes_guard(&mut self, line: &Line, d: Option<&Declaration<'_>>) -> bool {
        let directive = line.directive();
        if directive.is_empty() {
            return false;
        }
        let opened = guard_name(directive).map(str::to_owned);
        let pending = std::mem::replace(&mut self.guard, opened);
        d.is_some_and(|d| pending.as_deref() == Some(d.name) && d.value.is_none() && !d.commented)
    }
}

/// Every option of the pack, by name, at its default.
pub fn options(pack: &Pack) -> BTreeMap<String, PackOption> {
    let Scan {
        values: mut out,
        on,
        off,
        tested,
        ..
    } = scan(pack);
    for name in on.iter().chain(&off) {
        if out.contains_key(name) {
            continue;
        }
        // Defined both ways is ambiguous (8.1): on only where it is on everywhere.
        let default = on.contains(name) && !off.contains(name);
        out.insert(
            name.clone(),
            PackOption {
                name: name.clone(),
                default: default.to_string(),
                values: vec!["false".into(), "true".into()],
                switch: true,
                tested: tested.contains(name),
            },
        );
    }
    out
}

/// The name an `#ifndef NAME`, `#if !defined NAME` or `#if !defined(NAME)` directive tests,
/// when that test is the whole condition (any space after `#`). `line` is a [`Line::directive`]:
/// its comments are already gone. Any other condition (`#if !defined(A) && defined(B)`) opens no
/// include guard.
fn guard_name(line: &str) -> Option<&str> {
    let line = line.strip_prefix('#')?.trim_start();
    let (rest, paren) = match line.strip_prefix("ifndef") {
        Some(rest) => (rest.strip_prefix(char::is_whitespace)?, false),
        None => {
            let rest = line
                .strip_prefix("if")?
                .trim_start()
                .strip_prefix('!')?
                .trim_start()
                .strip_prefix("defined")?;
            match rest.trim_start().strip_prefix('(') {
                Some(rest) => (rest, true),
                None => (rest.strip_prefix(char::is_whitespace)?, false),
            }
        }
    };
    let rest = rest.trim_start();
    let end = rest
        .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .unwrap_or(rest.len());
    let (name, after) = rest.split_at(end);
    let after = after.trim_start();
    let after = if paren {
        after.strip_prefix(')')?.trim_start()
    } else {
        after
    };
    (!name.is_empty() && after.is_empty()).then_some(name)
}

/// The names `defined NAME` and `defined(NAME)` test in an `#if` line.
fn defined_names(line: &str) -> Vec<String> {
    line.split("defined")
        .skip(1)
        .filter_map(|rest| {
            let rest = rest.trim_start().trim_start_matches('(').trim_start();
            let end = rest
                .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
                .unwrap_or(rest.len());
            (end > 0).then(|| rest[..end].to_owned())
        })
        .collect()
}

/// `text` with the options in `chosen` at their chosen values: switches commented out or in,
/// values replaced. Lines keep their place. Declarations are read as the scan reads them, and
/// the `#define NAME` that closes an include guard is not an option, so it stays as written.
pub fn apply(text: &str, chosen: &BTreeMap<String, String>) -> String {
    let mut out = String::with_capacity(text.len() + 64);
    let mut lines = Lines::default();
    for raw in text.split_inclusive('\n') {
        let content = raw.trim_end_matches(['\r', '\n']);
        let end = &raw[content.len()..];
        let line = lines.read(content);
        let d = declaration(&line.code);
        let guard = lines.closes_guard(&line, d.as_ref());
        let replaced = d.filter(|_| !guard).and_then(|d| {
            let want = chosen.get(d.name)?;
            match (d.value, d.values) {
                (None, _) => {
                    let on = want == "true";
                    if on != d.commented {
                        return None;
                    }
                    // Where the declaration starts, past indent and block comments.
                    let start = line.code.len() - line.code.trim_start().len();
                    let (before, body) = content.split_at(start);
                    Some(if on {
                        format!("{before}{}", body.trim_start_matches('/').trim_start())
                    } else {
                        format!("{before}//{body}")
                    })
                }
                (Some((value, at)), Some(_)) if !d.commented => Some(format!(
                    "{}{want}{}",
                    &content[..at],
                    &content[at + value.len()..]
                )),
                _ => None,
            }
        });
        out.push_str(replaced.as_deref().unwrap_or(content));
        out.push_str(end);
    }
    out
}

/// Sets option `name` to `value` in `chosen` as the options screen does: a value the option doesn't
/// list, or an option the pack doesn't have, is ignored (`false`); the option's default takes it
/// out of `chosen`.
pub fn choose(
    chosen: &mut BTreeMap<String, String>,
    options: &BTreeMap<String, PackOption>,
    name: &str,
    value: String,
) -> bool {
    let Some(option) = options.get(name).filter(|o| o.values.contains(&value)) else {
        return false;
    };
    if value == option.default {
        chosen.remove(name);
    } else {
        chosen.insert(name.to_owned(), value);
    }
    true
}

/// The file the player's choices for pack `name` are kept in, beside it in `shaderpacks/`.
pub fn chosen_file(name: &str) -> String {
    format!("{name}.txt")
}

/// `NAME=value` lines; blank lines and `#` comments are skipped.
pub fn parse_chosen(text: &str) -> BTreeMap<String, String> {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .filter_map(|l| l.split_once('='))
        .map(|(k, v)| (k.trim().to_owned(), v.trim().to_owned()))
        .collect()
}

pub fn chosen_text(chosen: &BTreeMap<String, String>) -> String {
    chosen.iter().map(|(k, v)| format!("{k}={v}\n")).collect()
}

/// The texts of the pack's sources and of the files they include, as `read` reads them.
fn option_texts(pack: &Pack, read: fn(&Pack, &str) -> Option<String>) -> Vec<String> {
    let files = pack.all_files();
    let source = |path: &str| SOURCES.contains(&path.rsplit_once('.').map_or("", |(_, e)| e));
    let texts: Vec<(&String, String)> = files
        .iter()
        .filter(|p| source(p))
        .filter_map(|p| Some((p, read(pack, p)?)))
        .collect();
    // Files of other names count when a source includes them (`#include "shaders.settings"`).
    let included: BTreeSet<&str> = texts
        .iter()
        .flat_map(|(_, t)| t.lines())
        .filter_map(|l| l.trim().strip_prefix("#include"))
        .map(|r| r.trim().trim_matches(|c| c == '"' || c == '<' || c == '>'))
        .map(|r| r.rsplit('/').next().unwrap_or(r))
        .collect();
    let extra: Vec<(&String, String)> = files
        .iter()
        .filter(|p| !source(p) && included.contains(p.rsplit('/').next().unwrap_or(p)))
        .filter_map(|p| Some((p, read(pack, p)?)))
        .collect();
    texts.into_iter().chain(extra).map(|(_, t)| t).collect()
}

#[cfg(test)]
mod per_buffer_blending_tests;

#[cfg(test)]
mod tests {
    use super::*;

    /// `shaders.properties` read as the game reads it: its conditionals see the pack's options
    /// and the macros given (the renderer's vendor here), and the features it lists are then
    /// defined for the programs, with a note for a required one Sunrise lacks.
    #[test]
    fn properties_are_read_with_the_options_then_define_the_features() {
        let bytes = crate::pack::tests::zip(&[
            ("shaders/options.glsl", "#define LEVEL 2 // [1 2]\n"),
            (
                "shaders/shaders.properties",
                "#if LEVEL == 2\nscreen = x\n#endif\n\
                 #ifdef MC_GL_VENDOR_OTHER\nvendor = other\n#endif\n\
                 #if LEVEL == 2\niris.features.optional = SSBO\n#endif\n\
                 #ifdef MC_GL_VENDOR_OTHER\niris.features.required = NOT_A_FEATURE CUSTOM_IMAGES\n#endif\n",
            ),
        ]);
        let pack = Pack::from_zip_bytes("p.zip", bytes).unwrap();
        let mut macros = crate::Macros::standard();
        macros.define_renderer(4);
        let (properties, notes) = read_properties(&pack, &mut macros);
        assert_eq!(properties.get("screen"), Some("x"));
        assert_eq!(properties.get("vendor"), Some("other"));
        assert_eq!(macros.get("IRIS_FEATURE_SSBO"), Some("1"));
        assert_eq!(macros.get("IRIS_FEATURE_CUSTOM_IMAGES"), Some("1"));
        assert!(!macros.is_defined("IRIS_FEATURE_NOT_A_FEATURE"));
        // A flag Sunrise has that the pack doesn't list stays undefined.
        assert!(!macros.is_defined("IRIS_FEATURE_COMPUTE_SHADERS"));
        assert_eq!(
            notes,
            ["the pack requires `NOT_A_FEATURE`, which is not supported yet"]
        );
    }

    /// A pack with this `shaders.properties`, an `options.glsl` and a composite program whose
    /// fragment stage declares the uniform `seen_<name>` only where `IRIS_FEATURE_<NAME>` is
    /// defined, for each of `features`.
    pub(super) fn feature_pack(properties: &str, features: &[(&str, &str)]) -> Pack {
        let mut fsh = String::from("#version 330\nout vec4 c;\n");
        let mut sum = String::from("0.0");
        for (seen, feature) in features {
            fsh.push_str(&format!(
                "#ifdef IRIS_FEATURE_{feature}\nuniform float {seen};\n#else\n\
                 const float {seen} = 0.0;\n#endif\n"
            ));
            sum.push_str(&format!(" + {seen}"));
        }
        fsh.push_str(&format!("void main() {{ c = vec4({sum}); }}\n"));
        let vsh = "#version 330\nvoid main() { gl_Position = vec4(0.0); }\n";
        let bytes = crate::pack::tests::zip(&[
            ("shaders/shaders.properties", properties),
            ("shaders/composite.vsh", vsh),
            ("shaders/composite.fsh", &fsh),
        ]);
        Pack::from_zip_bytes("p.zip", bytes).unwrap()
    }

    /// The `seen_*` uniforms the pack's composite program declares, loaded as the game loads it.
    pub(super) fn composite_uniforms(
        pack: &Pack,
        properties: &crate::Properties,
        macros: &crate::Macros,
    ) -> Vec<String> {
        let post = crate::load_post(pack, "", properties, macros, &crate::GameInputs::default());
        let composite = post
            .programs
            .iter()
            .find(|p| p.name == "composite")
            .unwrap();
        let program = composite.program.as_ref().expect("composite translates");
        let mut names: Vec<String> = program
            .interface
            .uniforms
            .iter()
            .filter(|u| u.name.starts_with("seen_"))
            .map(|u| u.name.clone())
            .collect();
        names.sort();
        names
    }

    /// `IRIS_FEATURE_<NAME>` (spec 3.2, decision D-7): defined as `1` for a flag Sunrise has
    /// that `iris.features.optional` or `iris.features.required` lists, in any case, under the
    /// name as Sunrise writes it; undefined for one the pack doesn't list, one Sunrise lacks, and
    /// the name in the pack's own case. `shaders.properties`, the programs and the id files read
    /// with the macros the read leaves see the same answers.
    #[test]
    fn listed_features_sunrise_has_are_defined_for_properties_programs_and_id_files() {
        let features = [
            ("seen_optional", "CUSTOM_IMAGES"),
            ("seen_required", "COMPUTE_SHADERS"),
            ("seen_both", "HIGHER_SHADOWCOLOR"),
            ("seen_unlisted", "SSBO"),
            ("seen_unknown", "NOT_A_FEATURE"),
            ("seen_lower", "custom_images"),
            ("seen_missing_required", "NOT_REQUIRED_EITHER"),
        ];
        let mut text = String::from(
            "iris.features.optional = custom_images NOT_A_FEATURE HIGHER_SHADOWCOLOR\n\
             iris.features.required = Compute_Shaders NOT_REQUIRED_EITHER HIGHER_SHADOWCOLOR\n",
        );
        for (seen, feature) in features {
            text.push_str(&format!(
                "#ifdef IRIS_FEATURE_{feature}\n{seen} = 1\n#endif\n"
            ));
        }
        let pack = feature_pack(&text, &features);
        let mut macros = crate::Macros::standard();
        let (properties, notes) = read_properties(&pack, &mut macros);
        let read: Vec<&str> = features
            .iter()
            .filter(|(seen, _)| properties.get(seen).is_some())
            .map(|(seen, _)| *seen)
            .collect();
        assert_eq!(read, ["seen_optional", "seen_required", "seen_both"]);
        for name in ["CUSTOM_IMAGES", "COMPUTE_SHADERS", "HIGHER_SHADOWCOLOR"] {
            assert_eq!(
                macros.get(&format!("IRIS_FEATURE_{name}")),
                Some("1"),
                "{name}"
            );
        }
        let defined: Vec<&str> = macros.names().filter(|n| n.contains("IRIS")).collect();
        assert_eq!(defined.len(), 3, "{defined:?}");
        assert_eq!(
            notes,
            ["the pack requires `NOT_REQUIRED_EITHER`, which is not supported yet"]
        );
        assert_eq!(
            composite_uniforms(&pack, &properties, &macros),
            ["seen_both", "seen_optional", "seen_required"]
        );
        // `block.properties`, `entity.properties` and `item.properties` are read with the
        // same macros.
        let ids = crate::Properties::parse(
            "#ifdef IRIS_FEATURE_CUSTOM_IMAGES\nblock.7 = stone\n#endif\n\
             #ifdef IRIS_FEATURE_SSBO\nblock.8 = dirt\n#endif\n",
            &macros,
        );
        assert_eq!(ids.get("block.7"), Some("stone"));
        assert_eq!(ids.get("block.8"), None);
    }

    /// A pack that lists no flag Sunrise has gets no feature macro, its programs none either,
    /// and the macros are left as they were.
    #[test]
    fn a_pack_that_lists_no_feature_sunrise_has_gets_none() {
        let features = [("seen_ssbo", "SSBO"), ("seen_unknown", "NOT_A_FEATURE")];
        for text in [
            "",
            "iris.features.optional =\n",
            "iris.features.optional = NOT_A_FEATURE SSBO,\n",
            "#ifdef IRIS_FEATURE_SSBO\nseen_ssbo = 1\n#endif\n",
        ] {
            let pack = feature_pack(text, &features);
            let mut macros = crate::Macros::standard();
            let (properties, _) = read_properties(&pack, &mut macros);
            assert_eq!(macros, crate::Macros::standard(), "{text}");
            assert_eq!(properties.get("seen_ssbo"), None, "{text}");
            assert!(
                composite_uniforms(&pack, &properties, &macros).is_empty(),
                "{text}"
            );
        }
    }

    /// The flag lists are read once without any feature macro, then the answers are defined
    /// and the file read again: a list only a feature macro reaches is not read (its flag stays
    /// undefined), a list a feature macro would hide is read, and what is guarded by a listed
    /// flag's macro is read.
    #[test]
    fn flag_lists_are_settled_before_the_answers_are_defined() {
        let text = "iris.features.optional = CUSTOM_IMAGES\n\
                    #ifdef IRIS_FEATURE_CUSTOM_IMAGES\n\
                    iris.features.required = SSBO\nguarded = 1\n#endif\n\
                    #ifndef IRIS_FEATURE_CUSTOM_IMAGES\n\
                    iris.features.required = COMPUTE_SHADERS\n#endif\n";
        let pack = feature_pack(text, &[]);
        let mut macros = crate::Macros::standard();
        let (properties, notes) = read_properties(&pack, &mut macros);
        assert_eq!(properties.get("guarded"), Some("1"));
        assert!(macros.is_defined("IRIS_FEATURE_CUSTOM_IMAGES"));
        assert!(macros.is_defined("IRIS_FEATURE_COMPUTE_SHADERS"));
        assert!(!macros.is_defined("IRIS_FEATURE_SSBO"));
        assert!(notes.is_empty(), "{notes:?}");
    }

    /// What decides by the flag lists after the read (`load_post`'s shadow colour buffers)
    /// takes the lists the first read settled: `HIGHER_SHADOWCOLOR` listed where only its
    /// absence reaches gives the programs its macro and the pass its eight buffers.
    #[test]
    fn the_settled_lists_decide_the_shadow_colour_buffers() {
        for (text, buffers) in [
            (
                "#ifndef IRIS_FEATURE_HIGHER_SHADOWCOLOR\n\
                 iris.features.optional = HIGHER_SHADOWCOLOR\n#endif\n",
                crate::SHADOW_COLORS,
            ),
            (
                "iris.features.required = higher_shadowcolor\n",
                crate::SHADOW_COLORS,
            ),
            ("iris.features.optional = SSBO\n", 2),
            ("", 2),
        ] {
            let pack = feature_pack(text, &[]);
            let mut macros = crate::Macros::standard();
            let (properties, _) = read_properties(&pack, &mut macros);
            let higher = buffers == crate::SHADOW_COLORS;
            assert_eq!(
                macros.is_defined("IRIS_FEATURE_HIGHER_SHADOWCOLOR"),
                higher,
                "{text}"
            );
            let post = crate::load_post(
                &pack,
                "",
                &properties,
                &macros,
                &crate::GameInputs::default(),
            );
            assert_eq!(post.shadow_colors, buffers, "{text}");
        }
    }

    /// A pack's own option named `IRIS_FEATURE_*` (a value option or a switch) doesn't reach
    /// the first read, doesn't replace Sunrise's answer in the second, nor in
    /// `program.*.enabled`; the programs read the answer too.
    #[test]
    fn a_packs_option_named_as_a_feature_answer_changes_no_answer() {
        let options = "#define IRIS_FEATURE_SSBO 0 // [0 1]\n\
                       #define IRIS_FEATURE_COMPUTE_SHADERS\n\
                       #ifdef IRIS_FEATURE_COMPUTE_SHADERS\n#endif\n\
                       #define IRIS_FEATURE_FADE_VARIABLE\n\
                       #ifdef IRIS_FEATURE_FADE_VARIABLE\n#endif\n";
        let properties = "iris.features.optional = SSBO\n\
                          #ifdef IRIS_FEATURE_COMPUTE_SHADERS\n\
                          iris.features.required = CUSTOM_IMAGES\nreached = 1\n#endif\n\
                          #if IRIS_FEATURE_SSBO == 1\nssbo = 1\n#endif\n\
                          program.composite.enabled = IRIS_FEATURE_SSBO\n\
                          program.composite1.enabled = IRIS_FEATURE_FADE_VARIABLE\n";
        let fsh = |seen: &str| {
            format!(
                "#version 330\nout vec4 c;\n#if IRIS_FEATURE_SSBO == 1\nuniform float {seen};\n\
                 #else\nconst float {seen} = 0.0;\n#endif\n\
                 void main() {{ c = vec4({seen}); }}\n#include \"/options.glsl\"\n"
            )
        };
        let vsh = "#version 330\nvoid main() { gl_Position = vec4(0.0); }\n";
        let (composite, composite1) = (fsh("seen_one"), fsh("seen_other"));
        let bytes = crate::pack::tests::zip(&[
            ("shaders/shaders.properties", properties),
            ("shaders/options.glsl", options),
            ("shaders/composite.vsh", vsh),
            ("shaders/composite.fsh", &composite),
            ("shaders/composite1.vsh", vsh),
            ("shaders/composite1.fsh", &composite1),
        ]);
        let pack = Pack::from_zip_bytes("p.zip", bytes).unwrap();
        // Written as options, they are none: not found, so never injected.
        assert!(!value_options(&pack).contains_key("IRIS_FEATURE_SSBO"));
        assert!(!switches_on(&pack).contains("IRIS_FEATURE_COMPUTE_SHADERS"));
        assert!(
            super::options(&pack)
                .keys()
                .all(|n| !n.starts_with("IRIS_FEATURE_"))
        );
        let mut macros = crate::Macros::standard();
        let (read, _) = read_properties(&pack, &mut macros);
        assert_eq!(read.get("ssbo"), Some("1"));
        assert_eq!(read.get("reached"), None);
        assert_eq!(macros.get("IRIS_FEATURE_SSBO"), Some("1"));
        for name in ["CUSTOM_IMAGES", "COMPUTE_SHADERS", "FADE_VARIABLE"] {
            assert!(
                !macros.is_defined(&format!("IRIS_FEATURE_{name}")),
                "{name}"
            );
        }
        let post = crate::load_post(&pack, "", &read, &macros, &crate::GameInputs::default());
        let loaded: Vec<&str> = post.programs.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(loaded, ["composite"]);
        let program = post.programs[0]
            .program
            .as_ref()
            .expect("composite translates");
        assert!(
            program
                .interface
                .uniforms
                .iter()
                .any(|u| u.name == "seen_one")
        );
    }

    /// A saved choice for an `IRIS_FEATURE_*` name changes nothing: such a line is no option, so
    /// the choice is dropped and the line is never rewritten; a program sees only the answer.
    #[test]
    fn a_saved_choice_never_rewrites_a_feature_answer() {
        let include = "//#define IRIS_FEATURE_SSBO\n#define IRIS_FEATURE_FADE_VARIABLE 0 // [0 1]\n\
                       const int IRIS_FEATURE_ENTITY_TRANSLUCENT = 0; // [0 1]\n";
        let fsh = "#version 330\n#include \"/features.glsl\"\nout vec4 c;\n\
                   #ifdef IRIS_FEATURE_SSBO\nuniform float seen_ssbo;\n#else\n\
                   const float seen_ssbo = 0.0;\n#endif\n\
                   void main() { c = vec4(seen_ssbo); }\n";
        let vsh = "#version 330\nvoid main() { gl_Position = vec4(0.0); }\n";
        let bytes = crate::pack::tests::zip(&[
            (
                "shaders/shaders.properties",
                "iris.features.optional = CUSTOM_IMAGES\n",
            ),
            ("shaders/features.glsl", include),
            ("shaders/composite.vsh", vsh),
            ("shaders/composite.fsh", fsh),
        ]);
        let saved = parse_chosen(
            "IRIS_FEATURE_SSBO=true\nIRIS_FEATURE_FADE_VARIABLE=1\n\
             IRIS_FEATURE_ENTITY_TRANSLUCENT=1\n",
        );
        let pack = Pack::from_zip_bytes("p.zip", bytes).unwrap();
        let found = super::options(&pack);
        assert!(
            found.keys().all(|n| !n.starts_with("IRIS_FEATURE_")),
            "{found:?}"
        );
        let pack = pack.with_chosen(saved.clone());
        assert_eq!(pack.read_text("features.glsl").unwrap(), include);
        assert_eq!(apply(include, &saved), include);
        let mut macros = crate::Macros::standard();
        let (properties, _) = read_properties(&pack, &mut macros);
        assert!(!macros.is_defined("IRIS_FEATURE_SSBO"));
        let post = crate::load_post(
            &pack,
            "",
            &properties,
            &macros,
            &crate::GameInputs::default(),
        );
        let program = post.programs[0]
            .program
            .as_ref()
            .expect("composite translates");
        assert!(
            program
                .interface
                .uniforms
                .iter()
                .all(|u| u.name != "seen_ssbo")
        );
    }

    /// A pack's own `#define` / `#undef` of an `IRIS_FEATURE_*` name inside `shaders.properties`
    /// decides that file's conditionals from its line on (decision D-7): a define lets the first
    /// read reach a list behind it; an undef hides Sunrise's answer from the later `#ifdef`s,
    /// not from the lines above it nor from the programs.
    #[test]
    fn a_packs_own_directive_in_shaders_properties_applies_from_its_line() {
        let text = "#define IRIS_FEATURE_SSBO\n\
                    #ifdef IRIS_FEATURE_SSBO\niris.features.optional = CUSTOM_IMAGES\n#endif\n\
                    #ifdef IRIS_FEATURE_CUSTOM_IMAGES\nbefore = 1\n#endif\n\
                    #undef IRIS_FEATURE_CUSTOM_IMAGES\n\
                    #ifdef IRIS_FEATURE_CUSTOM_IMAGES\nafter = 1\n#endif\n";
        let pack = feature_pack(text, &[("seen_images", "CUSTOM_IMAGES")]);
        let mut macros = crate::Macros::standard();
        let (properties, _) = read_properties(&pack, &mut macros);
        assert_eq!(macros.get("IRIS_FEATURE_CUSTOM_IMAGES"), Some("1"));
        assert!(!macros.is_defined("IRIS_FEATURE_SSBO"));
        assert_eq!(properties.get("before"), Some("1"));
        assert_eq!(properties.get("after"), None);
        assert_eq!(
            composite_uniforms(&pack, &properties, &macros),
            ["seen_images"]
        );
    }

    /// A flag list set twice: the note says which one counts as the settled lists do. A list the
    /// second read adds behind an answer doesn't replace the first, and no note says it counts;
    /// another key set twice keeps the second read's note.
    #[test]
    fn a_repeated_flag_list_is_noted_as_the_lists_were_settled() {
        let text = "iris.features.optional = SSBO\n\
                    #ifdef IRIS_FEATURE_SSBO\niris.features.optional = HIGHER_SHADOWCOLOR\n\
                    screen = b\n#endif\nscreen = a\n";
        let pack = feature_pack(text, &[]);
        let mut macros = crate::Macros::standard();
        let (properties, _) = read_properties(&pack, &mut macros);
        assert_eq!(properties.get("iris.features.optional"), Some("SSBO"));
        assert!(!macros.is_defined("IRIS_FEATURE_HIGHER_SHADOWCOLOR"));
        let post = crate::load_post(
            &pack,
            "",
            &properties,
            &macros,
            &crate::GameInputs::default(),
        );
        assert_eq!(post.shadow_colors, 2);
        let repeated: Vec<(u32, Option<&str>)> = properties
            .notes
            .iter()
            .map(|n| (n.line, n.repeated.as_deref()))
            .collect();
        assert_eq!(repeated, [(6, Some("screen"))]);
        assert_eq!(properties.get("screen"), Some("a"));
    }

    /// The settled lists keep their place in the file's order among the other entries.
    #[test]
    fn the_settled_lists_keep_their_place_in_the_file() {
        let text = "a = 1\niris.features.optional = SSBO\nb = 2\n\
                    #ifdef IRIS_FEATURE_SSBO\niris.features.optional = HIGHER_SHADOWCOLOR\n#endif\n\
                    c = 3\niris.features.required = CUSTOM_IMAGES\nd = 4\n";
        let pack = feature_pack(text, &[]);
        let (properties, _) = read_properties(&pack, &mut crate::Macros::standard());
        let keys: Vec<(&str, u32)> = (properties.entries.iter())
            .map(|e| (e.key.as_str(), e.line))
            .collect();
        assert_eq!(
            keys,
            [
                ("a", 1),
                ("iris.features.optional", 2),
                ("b", 3),
                ("c", 7),
                ("iris.features.required", 8),
                ("d", 9)
            ]
        );
    }

    /// A list repeated in the first read whose earlier occurrence the second read doesn't
    /// reach: the note that the later one counts is still there, from the first read.
    #[test]
    fn a_repeat_the_second_read_cannot_see_is_still_noted() {
        let text = "#ifndef IRIS_FEATURE_SSBO\niris.features.optional = CUSTOM_IMAGES\n#endif\n\
                    iris.features.optional = SSBO\n";
        let pack = feature_pack(text, &[]);
        let mut macros = crate::Macros::standard();
        let (properties, _) = read_properties(&pack, &mut macros);
        assert!(macros.is_defined("IRIS_FEATURE_SSBO"));
        assert!(!macros.is_defined("IRIS_FEATURE_CUSTOM_IMAGES"));
        let repeated: Vec<(u32, Option<&str>)> = (properties.notes.iter())
            .map(|n| (n.line, n.repeated.as_deref()))
            .collect();
        assert_eq!(repeated, [(4, Some("iris.features.optional"))]);
    }

    /// A pack's `#undef` of an answer in `shaders.properties` decides the conditionals after it;
    /// the values the load expands again with its own macros (`program.<name>.enabled`,
    /// `size.buffer.*`, `image.*`) still see the answer, as they see every other macro the load
    /// has.
    #[test]
    fn an_undef_in_shaders_properties_hides_the_answer_from_conditionals_only() {
        let text = "iris.features.optional = SSBO\n#undef IRIS_FEATURE_SSBO\n\
                    #ifdef IRIS_FEATURE_SSBO\nguarded = 1\n#endif\n\
                    program.composite.enabled = IRIS_FEATURE_SSBO\n\
                    size.buffer.colortex4 = 8 IRIS_FEATURE_SSBO\n\
                    image.grid = none RGBA RGBA8 BYTE false false 16 IRIS_FEATURE_SSBO\n";
        let pack = feature_pack(text, &[("seen_ssbo", "SSBO")]);
        let mut macros = crate::Macros::standard();
        let (properties, _) = read_properties(&pack, &mut macros);
        assert_eq!(properties.get("guarded"), None);
        assert_eq!(
            composite_uniforms(&pack, &properties, &macros),
            ["seen_ssbo"]
        );
        let post = crate::load_post(
            &pack,
            "",
            &properties,
            &macros,
            &crate::GameInputs::default(),
        );
        assert_eq!(post.sizes[4], Some(crate::BufferSize::Absolute([8, 1])));
        let grid: Vec<_> = post
            .images
            .iter()
            .map(|i| (i.name.as_str(), i.size))
            .collect();
        assert_eq!(grid, [("grid", crate::ImageSize::Absolute([16, 1]))]);
    }

    /// A macro set reused for another pack starts its first read without the answers the
    /// previous pack got: a pack that lists no flag sees none.
    #[test]
    fn a_reused_macro_set_carries_no_answer_to_the_next_pack() {
        let features = [("seen_ssbo", "SSBO")];
        let asks = feature_pack("iris.features.optional = SSBO\n", &features);
        let plain = feature_pack("#ifdef IRIS_FEATURE_SSBO\nleaked = 1\n#endif\n", &features);
        let mut macros = crate::Macros::standard();
        read_properties(&asks, &mut macros);
        assert!(macros.is_defined("IRIS_FEATURE_SSBO"));
        let (properties, _) = read_properties(&plain, &mut macros);
        assert!(!macros.is_defined("IRIS_FEATURE_SSBO"));
        assert_eq!(properties.get("leaked"), None);
        assert!(composite_uniforms(&plain, &properties, &macros).is_empty());
    }

    /// A flag Sunrise can't keep on every draw ([`crate::macros`]' `UNANSWERED`,
    /// `BLOCK_EMISSION_ATTRIBUTE`) loads without a note when required, and gets no answer, listed
    /// either way; the others the pack lists still do.
    #[test]
    fn block_emission_is_supported_but_not_answered() {
        let text = "iris.features.optional = BLOCK_EMISSION_ATTRIBUTE SSBO\n\
                    iris.features.required = block_emission_attribute\n\
                    #ifdef IRIS_FEATURE_BLOCK_EMISSION_ATTRIBUTE\nemission = 1\n#endif\n";
        let pack = feature_pack(text, &[("seen_emission", "BLOCK_EMISSION_ATTRIBUTE")]);
        let mut macros = crate::Macros::standard();
        let (properties, notes) = read_properties(&pack, &mut macros);
        assert!(notes.is_empty(), "{notes:?}");
        assert!(!macros.is_defined("IRIS_FEATURE_BLOCK_EMISSION_ATTRIBUTE"));
        assert_eq!(macros.get("IRIS_FEATURE_SSBO"), Some("1"));
        assert_eq!(properties.get("emission"), None);
        assert!(composite_uniforms(&pack, &properties, &macros).is_empty());
    }

    #[test]
    fn declared_defaults_remain_selectable_and_apply() {
        for (source, selected) in [
            (
                "#define QUALITY 0 // [1 2]\n",
                "#define QUALITY 2 // [1 2]\n",
            ),
            (
                "const int QUALITY = 0; // [1 2]\n",
                "const int QUALITY = 2; // [1 2]\n",
            ),
        ] {
            let bytes = crate::pack::tests::zip(&[("shaders/options.glsl", source)]);
            let pack = Pack::from_zip_bytes("options.zip", bytes).unwrap();
            let available = options(&pack);
            let option = &available["QUALITY"];
            assert_eq!(option.default, "0");
            assert_eq!(option.values, ["0", "1", "2"]);
            let chosen = pack.with_chosen(BTreeMap::from([("QUALITY".into(), "2".into())]));
            assert_eq!(chosen.read_text("options.glsl").unwrap(), selected);
            let reset = chosen.with_chosen(BTreeMap::from([("QUALITY".into(), "0".into())]));
            assert_eq!(reset.read_text("options.glsl").unwrap(), source);
        }
    }

    #[test]
    fn declared_defaults_keep_authored_order_and_empty_ranges_stay_hidden() {
        let bytes = crate::pack::tests::zip(&[(
            "shaders/options.glsl",
            "#define QUALITY 1 // [0 1 2]\n#define IGNORED 0 // []\n",
        )]);
        let pack = Pack::from_zip_bytes("options.zip", bytes).unwrap();
        let available = options(&pack);
        assert_eq!(available["QUALITY"].default, "1");
        assert_eq!(available["QUALITY"].values, ["0", "1", "2"]);
        assert!(!available.contains_key("IGNORED"));
    }
    use crate::pack::tests::zip;

    #[test]
    fn switches_need_a_test_and_no_value() {
        let pack = Pack::from_zip_bytes(
            "P.zip",
            zip(&[
                (
                    "shaders/lib/settings.glsl",
                    "  #define AO // ambient occlusion\n//#define BLOOM\n#define LEVEL 2 //[1 2]\n\
                     #define SPLIT\n#define UNTESTED\n",
                ),
                (
                    "shaders/world0/deferred.fsh",
                    "#ifdef AO\n#endif\n#ifndef BLOOM\n#endif\n#ifdef SPLIT\n#endif\n#ifdef LEVEL\n#endif\n",
                ),
                ("shaders/world-1/deferred.fsh", "//#define SPLIT\n"),
                (
                    "shaders/final.fsh",
                    "#include \"shaders.settings\"\n#ifdef GODRAYS\n#endif\n",
                ),
                ("shaders/shaders.settings", "    #define GODRAYS //Toggle\n"),
                ("shaders/notes.settings", "#define NOT_INCLUDED\n#ifdef NOT_INCLUDED\n"),
                ("shaders/shaders.properties", "#define NOT_SOURCE\n#ifdef NOT_SOURCE\n"),
            ]),
        )
        .unwrap();
        assert_eq!(
            switches_on(&pack),
            BTreeSet::from(["AO".to_owned(), "GODRAYS".to_owned()])
        );
    }

    #[test]
    fn options_read_switches_values_and_consts() {
        let pack = Pack::from_zip_bytes(
            "P.zip",
            zip(&[
                (
                    "shaders/lib/settings.glsl",
                    "#define AO\n//#define BLOOM // glow\n#define LEVEL 2 //[1 2 3]\n\
                     #define FIXED 4\nconst int shadowMapResolution = 2048; // [1024 2048]\n\
                     //#define OFF_VALUE 1 // [1 2]\n",
                ),
                (
                    "shaders/final.fsh",
                    "#ifdef AO\n#endif\n#if defined(BLOOM) && LEVEL > 1\n#endif\n",
                ),
                (
                    "shaders/lib/common.glsl",
                    "#ifndef COMMON_GLSL\n#define COMMON_GLSL\n#endif\n\
                     #if !defined(INCLUDE_SKY)\n\n#define INCLUDE_SKY\n#endif\n",
                ),
            ]),
        )
        .unwrap();
        let options = options(&pack);
        let names: Vec<&str> = options.keys().map(String::as_str).collect();
        assert_eq!(names, ["AO", "BLOOM", "LEVEL", "shadowMapResolution"]);
        assert_eq!(options["AO"].default, "true");
        assert!(options["BLOOM"].switch && options["BLOOM"].tested);
        assert_eq!(options["BLOOM"].default, "false");
        assert_eq!(options["LEVEL"].values, ["1", "2", "3"]);
        assert_eq!(options["shadowMapResolution"].default, "2048");
    }

    #[test]
    fn const_bools_are_switches_screens_name() {
        let pack = Pack::from_zip_bytes(
            "P.zip",
            zip(&[(
                "shaders/lib/settings.glsl",
                "const bool shadowHardwareFiltering = true;\nconst bool generateShadowMipmap = false; // mips\n\
                 const bool notABool = 1;\n",
            )]),
        )
        .unwrap();
        let options = options(&pack);
        let filtering = &options["shadowHardwareFiltering"];
        assert!(filtering.switch && !filtering.tested);
        assert_eq!(filtering.default, "true");
        assert_eq!(options["generateShadowMipmap"].default, "false");
        assert!(!options.contains_key("notABool"));
        let chosen = parse_chosen("shadowHardwareFiltering=false\n");
        assert_eq!(
            apply("const bool shadowHardwareFiltering = true;\n", &chosen),
            "const bool shadowHardwareFiltering = false;\n"
        );
    }

    #[test]
    fn chosen_values_rewrite_their_lines() {
        let chosen = parse_chosen("AO=false\nBLOOM=true\nLEVEL=3\nshadowMapResolution=1024\n");
        let text = "  #define AO // occlusion\r\n//#define BLOOM\n#define LEVEL 2 //[1 2 3]\n\
                    #define LEVEL 9\nconst int shadowMapResolution = 2048; // [1024 2048]\n";
        assert_eq!(
            apply(text, &chosen),
            "  //#define AO // occlusion\r\n#define BLOOM\n#define LEVEL 3 //[1 2 3]\n\
             #define LEVEL 9\nconst int shadowMapResolution = 1024; // [1024 2048]\n"
        );
        let pack = Pack::from_zip_bytes(
            "P.zip",
            zip(&[
                ("shaders/lib/settings.glsl", "#define LEVEL 2 //[1 2 3]\n"),
                ("shaders/shaders.properties", "#define LEVEL 2 //[1 2 3]\n"),
            ]),
        )
        .unwrap()
        .with_chosen(chosen.clone());
        assert_eq!(value_options(&pack)["LEVEL"], "3");
        assert_eq!(
            pack.read_text("shaders.properties").unwrap(),
            "#define LEVEL 2 //[1 2 3]\n"
        );
        assert_eq!(parse_chosen(&chosen_text(&chosen)), chosen);
    }

    /// An include guard (`#ifndef NAME`, `#if !defined NAME` or `# ifndef NAME`, then
    /// `#define NAME` with nothing between but blank and comment lines) is not an option (8.1,
    /// 1.4): the options list leaves it out, and `shaders.properties` and the
    /// `program.<name>.enabled` expressions read it as undefined, through the real reader and
    /// loader. A switch declared and tested the same way elsewhere still counts.
    #[test]
    fn include_guards_are_not_options_anywhere() {
        const VSH: &str = "#version 120\nvoid main() { gl_Position = ftransform(); }";
        const FSH: &str = "#version 120\n#include \"/lib/a.glsl\"\n#include \"/lib/b.glsl\"\n\
                           #include \"/lib/c.glsl\"\n#include \"/lib/settings.glsl\"\n\
                           /* DRAWBUFFERS:0 */\nvoid main() { gl_FragData[0] = vec4(1.0); }";
        let mut files = vec![
            (
                "shaders/lib/a.glsl",
                "#ifndef A_GLSL\n#define A_GLSL\n#endif\n",
            ),
            (
                "shaders/lib/b.glsl",
                "#if !defined(B_GLSL)\n\n// guard\n#define B_GLSL\n#endif\n#ifdef B_GLSL\n#endif\n",
            ),
            (
                "shaders/lib/c.glsl",
                "# ifndef C_GLSL\n# define C_GLSL\n# endif\n",
            ),
            (
                "shaders/lib/settings.glsl",
                "#define AO\n#ifdef AO\n#endif\n#ifndef DEFAULTED\nconst int x = 1;\n\
                 #define DEFAULTED\n#endif\n",
            ),
            (
                "shaders/shaders.properties",
                "program.composite.enabled = A_GLSL\nprogram.composite1.enabled = B_GLSL\n\
                 program.composite2.enabled = C_GLSL\nprogram.composite3.enabled = AO\n\
                 program.composite4.enabled = DEFAULTED\n\
                 #if defined A_GLSL || defined B_GLSL || defined C_GLSL\nsun = false\n#endif\n\
                 #ifdef AO\nvignette = false\n#endif\n",
            ),
            ("shaders/final.vsh", VSH),
            ("shaders/final.fsh", FSH),
        ];
        let names = [
            "composite",
            "composite1",
            "composite2",
            "composite3",
            "composite4",
        ];
        let paths: Vec<_> = names
            .iter()
            .flat_map(|n| [format!("shaders/{n}.vsh"), format!("shaders/{n}.fsh")])
            .collect();
        for path in &paths {
            files.push((path.as_str(), if path.ends_with("vsh") { VSH } else { FSH }));
        }
        let pack = Pack::from_zip_bytes("guards.zip", zip(&files)).unwrap();
        let listed: Vec<String> = options(&pack).into_keys().collect();
        assert_eq!(listed, ["AO", "DEFAULTED"]);
        let mut macros = crate::Macros::standard();
        let (properties, _) = read_properties(&pack, &mut macros);
        assert_eq!(properties.get("sun"), None);
        assert_eq!(properties.get("vignette"), Some("false"));
        let post = crate::load_post(&pack, "", &properties, &macros, &Default::default());
        let loaded: Vec<&str> = post.programs.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(
            loaded,
            ["composite3", "composite4", "final"],
            "{:?}",
            post.notes
        );
        assert_eq!(
            switches_on(&pack),
            BTreeSet::from(["AO".to_owned(), "DEFAULTED".to_owned()])
        );
    }

    /// A guard is opened only by the bare negated test (a comment may follow it), and only
    /// comment lines may stand between it and its `#define`: `//` lines, a commented switch,
    /// and block comments on one line or several. Any other condition opens no guard, so its
    /// `#define` stays an ordinary option.
    #[test]
    fn guards_are_the_bare_negated_test_past_comment_lines() {
        let pack = Pack::from_zip_bytes(
            "P.zip",
            zip(&[
                (
                    "shaders/lib/guards.glsl",
                    "#ifndef BLOCK_GLSL\n/* note */\n#define BLOCK_GLSL\n#endif\n\
                     #ifndef LINES_GLSL // guard\n//#define OTHER\n// note\n#define LINES_GLSL\n#endif\n\
                     #if !defined( OPEN_GLSL ) /* a\nb\n*/\n/* c\n */ /* d */\n#define OPEN_GLSL\n#endif\n\
                     #if !defined(LIGHTING_GLSL /* include once */)\n#define LIGHTING_GLSL\n#endif\n",
                ),
                (
                    "shaders/lib/settings.glsl",
                    "#if !defined(AO) && defined(SUPPORTED)\n#define AO\n#endif\n\
                     #ifndef BLOOM_ON 1\n#define BLOOM_ON\n#endif\n\
                     #ifndef CODE\nint x; /* c */\n#define CODE\n#endif\n\
                     #ifndef SPLIT /* a\n*/ int y;\n#define SPLIT\n#endif\n",
                ),
                (
                    "shaders/final.fsh",
                    "#ifdef AO\n#endif\n#ifdef BLOOM_ON\n#endif\n#ifdef OTHER\n#endif\n\
                     #ifdef BLOCK_GLSL\n#endif\n#ifdef LINES_GLSL\n#endif\n#ifdef OPEN_GLSL\n#endif\n\
                     #ifdef LIGHTING_GLSL\n#endif\n",
                ),
            ]),
        )
        .unwrap();
        let listed: Vec<String> = options(&pack).into_keys().collect();
        assert_eq!(listed, ["AO", "BLOOM_ON", "CODE", "OTHER", "SPLIT"]);
        assert_eq!(options(&pack)["OTHER"].default, "false");
        assert_eq!(
            switches_on(&pack),
            BTreeSet::from(["AO", "BLOOM_ON", "CODE", "SPLIT"].map(str::to_owned))
        );
    }

    /// `#define NAME//comment` (no space before the comment) declares `NAME`, as the GLSL
    /// preprocessor reads it: listed, and on or off alike for the options list and for
    /// `shaders.properties` and the program enables.
    #[test]
    fn a_switch_glued_to_its_comment_is_the_same_switch_everywhere() {
        let pack = Pack::from_zip_bytes(
            "P.zip",
            zip(&[
                (
                    "shaders/lib/settings.glsl",
                    "#define GLUED//comment\n//#define GLUED_OFF//comment\n",
                ),
                (
                    "shaders/final.fsh",
                    "#ifdef GLUED\n#endif\n#ifdef GLUED_OFF\n#endif\n",
                ),
            ]),
        )
        .unwrap();
        let options = options(&pack);
        let listed: Vec<(&str, &str)> = options
            .values()
            .map(|o| (o.name.as_str(), o.default.as_str()))
            .collect();
        assert_eq!(listed, [("GLUED", "true"), ("GLUED_OFF", "false")]);
        assert_eq!(switches_on(&pack), BTreeSet::from(["GLUED".to_owned()]));
    }

    /// A name the pack declares off with `//#define NAME` anywhere is a switch, never a guard,
    /// even right after `#ifndef NAME`: at its default it is listed off and not on, and turned
    /// on (the line rewritten to `#define NAME`) it is still listed at its default, `switches_on`
    /// has it, and the program its enable names loads. Declared on in one file and off in another, it is
    /// listed off.
    #[test]
    fn a_commented_switch_after_its_ifndef_is_an_option_off_and_on() {
        const VSH: &str = "#version 120\nvoid main() { gl_Position = ftransform(); }";
        const FSH: &str =
            "#version 120\n/* DRAWBUFFERS:0 */\nvoid main() { gl_FragData[0] = vec4(1.0); }";
        let bytes = zip(&[
            (
                "shaders/lib/settings.glsl",
                "#ifndef BLOOM\n//#define BLOOM\n#endif\n",
            ),
            ("shaders/lib/fog.glsl", "#ifndef FOG\n#define FOG\n#endif\n"),
            ("shaders/lib/more.glsl", "//#define FOG\n"),
            (
                "shaders/lib/uses.glsl",
                "#ifdef BLOOM\n#endif\n#ifdef FOG\n#endif\n",
            ),
            (
                "shaders/shaders.properties",
                "program.composite.enabled = BLOOM\n",
            ),
            ("shaders/composite.vsh", VSH),
            ("shaders/composite.fsh", FSH),
            ("shaders/final.vsh", VSH),
            ("shaders/final.fsh", FSH),
        ]);
        let pack = Pack::from_zip_bytes("bloom.zip", bytes).unwrap();
        let loaded = |pack: &Pack| {
            let mut macros = crate::Macros::standard();
            let (properties, _) = read_properties(pack, &mut macros);
            let post = crate::load_post(pack, "", &properties, &macros, &Default::default());
            post.programs
                .iter()
                .map(|p| p.name.clone())
                .collect::<Vec<_>>()
        };
        let defaults = |pack: &Pack| {
            options(pack)
                .into_values()
                .map(|o| (o.name, o.default))
                .collect::<Vec<_>>()
        };
        let off = [("BLOOM", "false"), ("FOG", "false")].map(|(n, d)| (n.into(), d.into()));
        assert_eq!(defaults(&pack), off);
        assert_eq!(switches_on(&pack), BTreeSet::new());
        assert_eq!(loaded(&pack), ["final"]);

        let on = pack.with_chosen(BTreeMap::from([("BLOOM".into(), "true".into())]));
        assert_eq!(
            on.read_text("lib/settings.glsl").unwrap(),
            "#ifndef BLOOM\n#define BLOOM\n#endif\n"
        );
        // The options are read from the pack as written, so the defaults stay the pack's.
        assert_eq!(defaults(&on), off);
        assert_eq!(switches_on(&on), BTreeSet::from(["BLOOM".to_owned()]));
        assert_eq!(loaded(&on), ["composite", "final"]);
    }

    /// The scan reads each line's code outside block comments, through the real reader and
    /// loader: a guard opened after a block comment on its line is a guard, a `//#define` or
    /// `#define` inside a block comment over several lines declares nothing, and a block comment
    /// after a switch's name leaves it a switch.
    #[test]
    fn options_are_read_from_the_code_outside_block_comments() {
        const VSH: &str = "#version 120\nvoid main() { gl_Position = ftransform(); }";
        const FSH: &str =
            "#version 120\n/* DRAWBUFFERS:0 */\nvoid main() { gl_FragData[0] = vec4(1.0); }";
        let mut files = vec![
            (
                "shaders/lib/once.glsl",
                "/* include once */ #ifndef ONCE_GLSL\n#define ONCE_GLSL\n#endif\n\
                 #ifdef ONCE_GLSL\n#endif\n",
            ),
            (
                "shaders/lib/notes.glsl",
                "/*\n//#define HIDDEN\n#define HIDDEN_ON\n*/\n#ifdef HIDDEN\n#endif\n\
                 #ifdef HIDDEN_ON\n#endif\n#define AO /* on */\n#ifdef AO\n#endif\n",
            ),
            (
                "shaders/shaders.properties",
                "program.composite.enabled = ONCE_GLSL\nprogram.composite1.enabled = HIDDEN_ON\n\
                 program.composite2.enabled = AO\n#ifdef ONCE_GLSL\nsun = once\n#endif\n\
                 #ifdef AO\nvignette = ao\n#endif\n",
            ),
        ];
        let paths: Vec<_> = ["composite", "composite1", "composite2", "final"]
            .iter()
            .flat_map(|n| [format!("shaders/{n}.vsh"), format!("shaders/{n}.fsh")])
            .collect();
        for path in &paths {
            files.push((path.as_str(), if path.ends_with("vsh") { VSH } else { FSH }));
        }
        let pack = Pack::from_zip_bytes("comments.zip", zip(&files)).unwrap();
        let listed: Vec<String> = options(&pack).into_keys().collect();
        assert_eq!(listed, ["AO"]);
        let mut macros = crate::Macros::standard();
        let (properties, _) = read_properties(&pack, &mut macros);
        assert_eq!(properties.get("sun"), None);
        assert_eq!(properties.get("vignette"), Some("ao"));
        let post = crate::load_post(&pack, "", &properties, &macros, &Default::default());
        let loaded: Vec<&str> = post.programs.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(loaded, ["composite2", "final"], "{:?}", post.notes);
        assert_eq!(switches_on(&pack), BTreeSet::from(["AO".to_owned()]));
    }

    /// A choice applies to the declarations that are options, decided from the pack as written:
    /// choosing a `const bool` that shares its name with an include guard turns the `const` on
    /// and leaves the guard's name undefined for `shaders.properties` and the program enables,
    /// and turning off a switch that shares its name with a guard comments the switch out but
    /// leaves the guard's `#define` as written.
    #[test]
    fn a_choice_reaches_the_options_not_a_guard_of_the_same_name() {
        const VSH: &str = "#version 120\nvoid main() { gl_Position = ftransform(); }";
        const FSH: &str =
            "#version 120\n/* DRAWBUFFERS:0 */\nvoid main() { gl_FragData[0] = vec4(1.0); }";
        const SHARED_GUARD: &str =
            "#ifndef SHARED\n#define SHARED\n#endif\n#ifdef SHARED\n#endif\n";
        const REAL_GUARD: &str = "#ifndef REAL\n#define REAL\n#endif\n";
        let bytes = zip(&[
            ("shaders/lib/shared.glsl", SHARED_GUARD),
            ("shaders/lib/real.glsl", REAL_GUARD),
            (
                "shaders/lib/settings.glsl",
                "const bool SHARED = false;\n#define REAL\n#ifdef REAL\n#endif\n",
            ),
            (
                "shaders/shaders.properties",
                "program.composite.enabled = SHARED\nprogram.composite1.enabled = REAL\n\
                 #ifdef SHARED\nsun = shared\n#endif\n",
            ),
            ("shaders/composite.vsh", VSH),
            ("shaders/composite.fsh", FSH),
            ("shaders/composite1.vsh", VSH),
            ("shaders/composite1.fsh", FSH),
            ("shaders/final.vsh", VSH),
            ("shaders/final.fsh", FSH),
        ]);
        let pack = Pack::from_zip_bytes("shared.zip", bytes).unwrap();
        let options = options(&pack);
        let listed: Vec<(&str, &str, bool)> = options
            .values()
            .map(|o| (o.name.as_str(), o.default.as_str(), o.switch))
            .collect();
        assert_eq!(listed, [("REAL", "true", true), ("SHARED", "false", true)]);
        assert_eq!(switches_on(&pack), BTreeSet::from(["REAL".to_owned()]));

        let chosen = pack.with_chosen(BTreeMap::from([
            ("SHARED".into(), "true".into()),
            ("REAL".into(), "false".into()),
        ]));
        assert_eq!(switches_on(&chosen), BTreeSet::new());
        let mut macros = crate::Macros::standard();
        let (properties, _) = read_properties(&chosen, &mut macros);
        assert_eq!(properties.get("sun"), None);
        let post = crate::load_post(&chosen, "", &properties, &macros, &Default::default());
        let loaded: Vec<&str> = post.programs.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(loaded, ["final"], "{:?}", post.notes);
        assert_eq!(
            chosen.read_text("lib/settings.glsl").unwrap(),
            "const bool SHARED = true;\n//#define REAL\n#ifdef REAL\n#endif\n"
        );
        assert_eq!(chosen.read_text("lib/shared.glsl").unwrap(), SHARED_GUARD);
        assert_eq!(chosen.read_text("lib/real.glsl").unwrap(), REAL_GUARD);
    }

    /// A line of many block comments is read in one walk over its delimiters, and the switch
    /// after them counts.
    #[test]
    fn a_line_of_many_block_comments_is_read_once() {
        let line = format!(
            "{}#define WIDE\n#ifdef WIDE\n#endif\n",
            "/**/".repeat(50_000)
        );
        let pack = Pack::from_zip_bytes("P.zip", zip(&[("shaders/lib/wide.glsl", line.as_str())]))
            .unwrap();
        assert_eq!(options(&pack)["WIDE"].default, "true");
        assert_eq!(switches_on(&pack), BTreeSet::from(["WIDE".to_owned()]));
    }

    /// A custom uniform's or variable's line has the macros expanded: a value option, a
    /// `#define` of `shaders.properties` (function-like too), a standard macro, and a macro in
    /// the uniform's name. Our probe pack's shape; Iris 1.11.7 drew p0..p5 as 127, 191, 64, 255,
    /// 127 and 255 of 255 with the option at its default.
    #[test]
    fn custom_uniform_lines_read_the_options_and_macros() {
        let bytes = crate::pack::tests::zip(&[
            ("shaders/final.fsh", "#define MY_OPT 2 // [1 2 3]\n"),
            (
                "shaders/shaders.properties",
                "#define PROP_DEF 3\n#define HALF(x) ((x) * 0.5)\n#define KEYNAME p5\n\
                 uniform.float.p0 = MY_OPT * 0.25\n\
                 uniform.float.p1 = PROP_DEF * 0.25\n\
                 variable.float.v2 = MY_OPT * 0.125\nuniform.float.p2 = v2\n\
                 uniform.float.p3 = if(MC_VERSION > 10000, 1.0, 0.0)\n\
                 uniform.float.p4 = HALF(MY_OPT) * 0.5\n\
                 uniform.float.KEYNAME = 1.0\n",
            ),
        ]);
        for (chosen, p0) in [(None, 0.5), (Some("3"), 0.75)] {
            let mut pack = Pack::from_zip_bytes("p.zip", bytes.clone()).unwrap();
            if let Some(value) = chosen {
                pack = pack.with_chosen(BTreeMap::from([("MY_OPT".into(), value.into())]));
            }
            let (properties, _) = read_properties(&pack, &mut crate::Macros::standard());
            assert!(properties.notes.is_empty(), "{:?}", properties.notes);
            let mut custom = crate::Custom::parse(&properties);
            let values: Vec<f64> = custom
                .eval(0.016, &|_| None)
                .iter()
                .map(|v| v.v[0])
                .collect();
            let p4 = if chosen.is_some() { 0.75 } else { 0.5 };
            assert_eq!(values, [p0, 0.75, p0 / 2.0, 1.0, p4, 1.0]);
            let names: Vec<_> = custom.uniforms().map(|(name, _)| name.to_owned()).collect();
            assert_eq!(names, ["p0", "p1", "p2", "p3", "p4", "p5"]);
            assert!(custom.notes.is_empty(), "{:?}", custom.notes);
        }
    }
}
