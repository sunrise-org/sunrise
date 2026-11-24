//! Each stage shares macro definitions with its includes. For branch arithmetic, hand naga's
//! GLSL preprocessor the condition and its referenced definitions; unrelated source and
//! earlier conditional arms never have to be replayed. [`Conditionals`] keeps the nesting of
//! `#if` ... `#endif` around them.
use super::{MAX_SCANNED, is_ident, replace_ident};
use pp_rs::{pp::Preprocessor, token::TokenValue};
use std::collections::{HashMap, HashSet};

#[derive(Default)]
pub(super) struct Conditions {
    macros: HashMap<String, String>,
    scanned: usize,
    has_line_macro: bool,
}

impl Conditions {
    pub(super) fn new(defines: &[&str]) -> Self {
        let mut state = Self::default();
        for define in defines {
            state.define(define);
        }
        state
    }

    pub(super) fn define(&mut self, definition: &str) {
        let end = definition
            .find(|c| !is_ident(c))
            .unwrap_or(definition.len());
        let name = &definition[..end];
        self.has_line_macro |= definition.contains("__LINE__");
        self.macros.insert(name.to_owned(), definition.to_owned());
    }

    pub(super) fn undef(&mut self, name: &str) {
        self.macros.remove(name);
    }

    /// Object-like and function-like definitions can refer to later definitions. Resolve
    /// their names at use, leaving actual token expansion to the GLSL frontend.
    fn definitions<'a>(&'a self, text: &'a str) -> Vec<&'a str> {
        let mut pending = vec![text];
        let mut seen = HashSet::new();
        let mut found = Vec::new();
        while let Some(source) = pending.pop() {
            for name in source.split(|c| !is_ident(c)).filter(|s| !s.is_empty()) {
                if let Some(definition) = self.macros.get(name)
                    && seen.insert(name)
                {
                    found.push(definition.as_str());
                    pending.push(definition);
                }
            }
        }
        found
    }

    pub(super) fn uses_line(&self, text: &str) -> bool {
        text.contains("__LINE__")
            || (self.has_line_macro
                && self
                    .definitions(text)
                    .iter()
                    .any(|d| d.contains("__LINE__")))
    }

    pub(super) fn test(&mut self, kind: &str, rest: &str, line: u32) -> Result<bool, String> {
        match kind {
            "ifdef" | "ifndef" => {
                if rest.is_empty() || !rest.chars().all(is_ident) {
                    return Err(format!("invalid #{kind} {rest}"));
                }
                let defined = rest == "__LINE__" || self.macros.contains_key(rest);
                Ok(defined == (kind == "ifdef"))
            }
            _ => self.number(rest, line, false).map(|n| n != 0),
        }
    }

    pub(super) fn line(&mut self, rest: &str, line: u32) -> Result<u32, String> {
        self.number(rest, line, true)
    }

    fn number(&mut self, expression: &str, line: u32, line_directive: bool) -> Result<u32, String> {
        let expression = resolve_defined(expression, |name| {
            name == "__LINE__" || self.macros.contains_key(name)
        })?;
        let mut source = String::new();
        for definition in self.definitions(&expression) {
            source.push_str("#define ");
            source.push_str(definition);
            source.push('\n');
        }
        source.push_str(&expression);
        // Expand macros as ordinary GLSL tokens. In particular, function-like macros take
        // their arguments here before the conditional expression parser evaluates the result.
        let source = replace_ident(&source, "__LINE__", &line.to_string());
        self.charge(source.len())?;
        let mut code = String::from(if line_directive { "#line " } else { "#if " });
        for token in Preprocessor::new(&source) {
            let token = token.map_err(|(why, _)| format!("GLSL directive: {why:?}"))?;
            let spelling = &source[token.location.start as usize..token.location.end as usize];
            self.charge(spelling.len() + 1)?;
            code.push_str(spelling);
            code.push(' ');
        }
        if line_directive {
            code.push_str("\n__LINE__\n");
        } else {
            code.push_str("\n1\n#else\n0\n#endif\n");
        }
        match Preprocessor::new(&code).next() {
            Some(Ok(token)) => match token.value {
                TokenValue::Integer(n) => n
                    .value
                    .try_into()
                    .map_err(|_| "line number outside GLSL range".into()),
                _ => Err("expected an integer from a GLSL directive".into()),
            },
            Some(Err((why, _))) => Err(format!("GLSL directive: {why:?}")),
            None => Err("empty GLSL directive".into()),
        }
    }

    fn charge(&mut self, bytes: usize) -> Result<(), String> {
        self.scanned += bytes;
        if self.scanned > MAX_SCANNED {
            return Err(super::ImportRefusal::TooMuchScanned.to_string());
        }
        Ok(())
    }
}

/// One level of `#if` ... `#endif`.
struct Branch {
    /// This branch's lines are read.
    on: bool,
    /// A branch of this `#if` was already taken (the rest are skipped).
    taken: bool,
    /// The enclosing level is read.
    outer: bool,
    seen_else: bool,
}

/// The state of nested conditional directives.
#[derive(Default)]
pub(super) struct Conditionals(Vec<Branch>);

impl Conditionals {
    /// Lines here are read.
    pub(super) fn active(&self) -> bool {
        self.0.last().is_none_or(|b| b.on)
    }

    /// An `#if` is still open.
    pub(super) fn open(&self) -> bool {
        !self.0.is_empty()
    }

    /// Applies `#word` when it is a conditional directive (`Ok(false)` when it is not), with the
    /// caller's evaluator for its condition. Skipped arms never evaluate their conditions; one
    /// that can't be evaluated reads as false and returns why.
    pub(super) fn apply_with(
        &mut self,
        word: &str,
        mut evaluate: impl FnMut(&str) -> Result<bool, String>,
    ) -> Result<bool, String> {
        let on = self.active();
        match word {
            "if" | "ifdef" | "ifndef" => {
                let value = if on { evaluate(word) } else { Ok(false) };
                let v = *value.as_ref().unwrap_or(&false);
                self.0.push(Branch {
                    on: v,
                    taken: v,
                    outer: on,
                    seen_else: false,
                });
                value.map(|_| true)
            }
            "elif" | "else" => {
                let b = self
                    .0
                    .last_mut()
                    .ok_or_else(|| format!("`#{word}` without `#if`"))?;
                if b.seen_else {
                    return Err(format!("`#{word}` after `#else`"));
                }
                b.seen_else = word == "else";
                let take = b.outer && !b.taken;
                let value = match word {
                    "else" => Ok(take),
                    _ if take => evaluate("if"),
                    _ => Ok(false),
                };
                b.on = *value.as_ref().unwrap_or(&false);
                b.taken |= b.on;
                value.map(|_| true)
            }
            "endif" => self
                .0
                .pop()
                .map(|_| true)
                .ok_or_else(|| "`#endif` without `#if`".into()),
            _ => Ok(false),
        }
    }
}

/// `defined X` and `defined(X)` become `1` or `0` before anything is expanded.
fn resolve_defined(expr: &str, is_defined: impl Fn(&str) -> bool) -> Result<String, String> {
    let mut out = String::with_capacity(expr.len());
    let mut rest = expr;
    while let Some(at) = find_word(rest, "defined") {
        out.push_str(&rest[..at]);
        let after = rest[at + "defined".len()..].trim_start();
        let (inner, tail) = match after.strip_prefix('(') {
            Some(open) => {
                let close = open.find(')').ok_or("`defined(` without `)`")?;
                (open[..close].trim(), &open[close + 1..])
            }
            None => {
                let len = after.find(|c: char| !is_ident(c)).unwrap_or(after.len());
                (&after[..len], &after[len..])
            }
        };
        if inner.is_empty()
            || !inner.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_')
            || !inner.chars().all(is_ident)
        {
            return Err("`defined` needs a macro name".into());
        }
        out.push_str(if is_defined(inner) { " 1 " } else { " 0 " });
        rest = tail;
    }
    out.push_str(rest);
    Ok(out)
}

/// Where `word` starts in `text` as a whole identifier.
fn find_word(text: &str, word: &str) -> Option<usize> {
    let mut from = 0;
    while let Some(i) = text[from..].find(word).map(|i| i + from) {
        let before = text[..i].chars().next_back().is_some_and(is_ident);
        let after = text[i + word.len()..].chars().next().is_some_and(is_ident);
        if !before && !after {
            return Some(i);
        }
        from = i + word.len();
    }
    None
}
