//! How a pack's files take a conditional directive (spec 1.5, GLSL 4.60 §3.3), over
//! `sunrise-glsl`'s preprocessor ([`sunrise_glsl::condition`], which has the arithmetic of `#if`:
//! float literals count, an identifier that is not a macro counts as 0, so a pack doesn't lose a
//! whole section of its settings over one unknown name). Here: `#ifdef` reads the first word
//! after it, and a condition that can't be evaluated reads as false and says why, as Iris loads
//! such a pack.

use crate::Macros;
pub use sunrise_glsl::condition::{Conditionals, eval_condition, int_literal, resolve_defined};

/// Applies `#word rest` to `conditionals` when it is a conditional directive (`Ok(false)` when
/// it is not). A condition that can't be evaluated reads as false and returns why.
pub(crate) fn apply(
    conditionals: &mut Conditionals,
    word: &str,
    rest: &str,
    macros: &Macros,
) -> Result<bool, String> {
    conditionals.apply_with(word, |kind| test(kind, rest, macros, &mut |_| {}))
}

/// [`apply`], calling `lookup` with each name the condition it evaluates looks up: the name
/// `#ifdef` / `#ifndef` tests, and each name `#if` / `#elif` reads
/// ([`sunrise_glsl::condition::eval_condition_noting`]). A condition that isn't evaluated (a
/// branch not reached, an `#elif` after a taken one) looks up nothing.
pub(crate) fn apply_noting(
    conditionals: &mut Conditionals,
    word: &str,
    rest: &str,
    macros: &Macros,
    lookup: &mut dyn FnMut(&str),
) -> Result<bool, String> {
    conditionals.apply_with(word, |kind| test(kind, rest, macros, lookup))
}

/// The text after a line's `#`, split into the directive word and the rest without its `//`
/// comment. `None` when the first word is followed by something other than white space, `//` or
/// the end of the line (`#ifdefault=1` is a comment, not `#ifdef`).
pub(crate) fn split_directive(after_hash: &str) -> Option<(&str, &str)> {
    let line = after_hash.trim_start();
    let len = line
        .find(|c: char| !c.is_ascii_alphabetic())
        .unwrap_or(line.len());
    let (word, rest) = line.split_at(len);
    let ok = rest.is_empty() || rest.starts_with(char::is_whitespace) || rest.starts_with("//");
    ok.then(|| (word, rest.split("//").next().unwrap_or("").trim()))
}

fn test(
    word: &str,
    rest: &str,
    macros: &Macros,
    lookup: &mut dyn FnMut(&str),
) -> Result<bool, String> {
    let name = rest.split_whitespace().next().unwrap_or("");
    match word {
        "ifdef" | "ifndef" => {
            lookup(name);
            Ok(macros.is_defined(name) == (word == "ifdef"))
        }
        _ => sunrise_glsl::condition::eval_condition_noting(rest, macros, lookup)
            .map(|v| v != 0)
            .map_err(|e| format!("`#{word} {rest}`: {e}; read as false")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The pack's own macros take part in the arithmetic.
    #[test]
    fn the_packs_macros_take_part() {
        let m = Macros::standard();
        assert_eq!(eval_condition("MC_VERSION >= 11300", &m), Ok(1));
        assert_eq!(eval_condition("MC_VERSION <= 12111", &m), Ok(0));
    }
}
