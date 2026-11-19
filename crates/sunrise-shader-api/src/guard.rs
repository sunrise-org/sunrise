//! Keeps a mod's or a pack's shader from crashing the game. naga's WGSL parser, validator and
//! backends recurse over the shape of the source, so an expression chain of ten thousand terms
//! (`a + a + a + ...`) overflows a thread's stack, and a stack overflow aborts the process: a
//! shader file (a resource pack's, possibly pushed by a server) could take the game or the
//! check tool down.
//!
//! Three bounds, which together hold for any input:
//! - [`MAX_SOURCE_BYTES`]: a shader longer than that is refused before it is parsed. naga's
//!   other recursion (a chain of structs, aliases or calls) grows with the length of the
//!   source, and a shader this long is far inside what a thread's stack holds.
//! - [`MAX_CHAINED_OPERATORS`]: naga limits the nesting of parentheses, blocks and unary
//!   operators itself, but not a chain of binary operators, member accesses or indexings,
//!   which it lowers by recursion as deep as the chain is long. [`refusal`] counts them in
//!   the text, before it is parsed. This is the bound that makes a shader which passes safe
//!   to parse again on any thread, as wgpu does when it builds the pipeline.
//! - [`guarded`]: the parse, validation and translation run on a thread with a large stack,
//!   so a shape the two bounds above did not foresee still does not reach the limit of the
//!   caller's own stack, and a panic comes back as a refusal.
//!
//! GLSL, which naga's GLSL front end reads (resource packs' core and post shaders, OptiFine-format
//! shader packs), is bounded by [`glsl_refusal`] the same way, but naga limits no nesting there:
//! brackets, unary operators and statements without braces count too ([`MAX_GLSL_NESTING`]),
//! and packs' programs may be longer ([`MAX_GLSL_BYTES`]). The check reads the text naga's parser
//! will: callers preprocess it first (includes, `#define`s and `#if`s, each bounded as it runs),
//! and a directive naga's own preprocessor would still act on is refused.

/// The most bytes of WGSL a shader (the prelude, the files and the entry points as composed)
/// may hold.
pub const MAX_SOURCE_BYTES: usize = 128 * 1024;

/// The most binary operators, member accesses and indexings one expression may chain. The
/// count is generous for written code (a polynomial of 90 terms has about 180) and the deepest
/// parse it allows takes about a megabyte of a debug build's stack.
pub const MAX_CHAINED_OPERATORS: usize = 200;

/// The most bytes of GLSL a stage may hold as naga reads it: includes pasted and macros expanded.
/// Preprocessing stops at it as it goes, so it bounds the memory and the work a stage takes; the
/// other bounds hold whatever the length. Four times what a core shader's includes may grow to.
/// It was not measured against real packs' programs: a stage longer than it is refused.
pub const MAX_GLSL_BYTES: usize = 4 << 20;

/// How deep GLSL's brackets, unary operators and statements without braces may nest. naga's GLSL
/// parser recurses once a level with no limit of its own; this is the one its WGSL parser keeps
/// to (200 levels), so a module from GLSL nests about as deep as one from WGSL may.
pub const MAX_GLSL_NESTING: usize = 200;

/// The stack of the thread a shader is checked on. It is address space the system commits as
/// it is used.
const STACK_BYTES: usize = 64 << 20;

/// Why a shader may not be parsed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Refusal {
    pub excess: Excess,
    /// The byte offset it is found at; `None` for a shader too long.
    pub at: Option<usize>,
    pub message: String,
}

/// The bound a shader gets past.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Excess {
    /// It is longer than the cap ([`MAX_SOURCE_BYTES`], [`MAX_GLSL_BYTES`]).
    Bytes,
    /// An expression chains more than [`MAX_CHAINED_OPERATORS`] operators.
    Chain,
    /// GLSL nests deeper than [`MAX_GLSL_NESTING`].
    Nesting,
    /// A directive naga's GLSL preprocessor would act on is left in the text.
    Directive,
}

/// Why `text` may not be parsed, and the byte offset it is found at when it has one: it is too
/// long, or an expression chains more than [`MAX_CHAINED_OPERATORS`] operators.
pub fn refusal(text: &str) -> Option<(Option<usize>, String)> {
    refusal_as(text, MAX_SOURCE_BYTES, false).map(|r| (r.at, r.message))
}

/// [`refusal`] for GLSL as naga's parser reads it, preprocessed: longer than [`MAX_GLSL_BYTES`],
/// an expression chaining more than [`MAX_CHAINED_OPERATORS`] operators (a ternary's `?` among
/// them), nesting deeper than [`MAX_GLSL_NESTING`], or holding a directive naga's preprocessor
/// would act on (any but `#version`, `#extension`, `#pragma` and `#line` at the start of a line),
/// since what that expands or leaves out is not what was counted. Comments are read as GLSL's,
/// which don't nest.
pub fn glsl_refusal(text: &str) -> Option<Refusal> {
    refusal_as(text, MAX_GLSL_BYTES, true)
}

fn refusal_as(text: &str, max_bytes: usize, glsl: bool) -> Option<Refusal> {
    if text.len() > max_bytes {
        return Some(Refusal {
            excess: Excess::Bytes,
            at: None,
            message: format!(
                "{} bytes of shader; a shader may hold {max_bytes}",
                text.len()
            ),
        });
    }
    first_excess(text, glsl).map(|(at, excess, message)| Refusal {
        excess,
        at: Some(at),
        message,
    })
}

/// Where `text` first gets past a bound (its byte offset), and which: an expression chains more
/// than [`MAX_CHAINED_OPERATORS`] binary operators, member accesses or indexings, or, in GLSL,
/// nests deeper than [`MAX_GLSL_NESTING`] or holds a directive that changes code. An expression is
/// what lies between `;`, `{` and `}`, which no expression contains. Comments and numbers are read
/// as naga reads them, so that nothing naga takes for an operator is hidden from the count; what
/// it counts beyond them (such as the angle brackets of a template) only makes the bound stricter.
fn first_excess(text: &str, glsl: bool) -> Option<(usize, Excess, String)> {
    let mut chars = text.char_indices().peekable();
    let (mut count, mut after_operand) = (0, false);
    let mut nesting = Nesting::default();
    // Only blank space since the line began: where a directive may start.
    let mut line_start = true;
    while let Some((at, c)) = chars.next() {
        let directive_may_start = line_start;
        line_start = is_line_end(c) || (line_start && is_blank(c));
        match c {
            '/' if chars.peek().is_some_and(|(_, n)| *n == '/') => {
                while chars.next_if(|(_, n)| !is_line_end(*n)).is_some() {}
                continue;
            }
            '/' if chars.peek().is_some_and(|(_, n)| *n == '*') => {
                // naga's own scan: the character after `/*` only starts what follows it, so
                // `/**/` ends at once and in WGSL `/*/*` opens a nested comment (not in GLSL).
                chars.next();
                let (mut depth, mut prev) = (1, chars.next().map(|(_, c)| c));
                for (_, c) in &mut chars {
                    match (prev, c) {
                        (Some('*'), '/') => {
                            prev = None;
                            depth -= 1;
                            if depth == 0 {
                                break;
                            }
                        }
                        (Some('/'), '*') if !glsl => {
                            prev = None;
                            depth += 1;
                        }
                        _ => prev = Some(c),
                    }
                }
                continue;
            }
            // A directive that changes no code is skipped; any other would change what naga
            // parses after this count.
            '#' if glsl => {
                let rest = text[at + 1..].trim_start_matches([' ', '\t']);
                let end = rest
                    .find(|c: char| !c.is_ascii_alphanumeric() && c != '_')
                    .unwrap_or(rest.len());
                let word = &rest[..end];
                if !directive_may_start
                    || !matches!(word, "version" | "extension" | "pragma" | "line")
                {
                    return Some((
                        at,
                        Excess::Directive,
                        format!(
                            "`#{word}` is left for naga's preprocessor, which would change the \
                             code after this check"
                        ),
                    ));
                }
                while chars.next_if(|(_, n)| !is_line_end(*n)).is_some() {}
                continue;
            }
            ';' | '}' => {
                count = 0;
                after_operand = false;
                if c == '}' {
                    nesting.close();
                }
                nesting.end_statement();
            }
            '{' => {
                count = 0;
                after_operand = false;
                nesting.open();
            }
            c if is_blank(c) => {}
            c if c.is_ascii_digit() => {
                number(c, &mut chars);
                after_operand = true;
                nesting.operand();
            }
            c if c.is_alphanumeric() || c == '_' || !c.is_ascii() => {
                let mut end = at + c.len_utf8();
                while let Some((i, n)) =
                    chars.next_if(|(_, n)| n.is_alphanumeric() || *n == '_' || !n.is_ascii())
                {
                    end = i + n.len_utf8();
                }
                after_operand = true;
                nesting.word(&text[at..end]);
            }
            ')' | ']' => {
                after_operand = true;
                nesting.close();
                nesting.operand();
            }
            '+' | '-' | '*' | '/' | '%' | '&' | '|' | '^' | '<' | '>' | '=' | '!' | '.' | '['
            | '?'
                if after_operand =>
            {
                count += 1;
                if count == MAX_CHAINED_OPERATORS + 1 {
                    let split = if glsl { "a variable" } else { "`let`" };
                    return Some((
                        at,
                        Excess::Chain,
                        format!(
                            "an expression chains more than {MAX_CHAINED_OPERATORS} operators \
                             ({count} here); split it with {split}"
                        ),
                    ));
                }
                after_operand = false;
                if c == '[' {
                    nesting.open();
                }
            }
            '(' | '[' => {
                after_operand = false;
                nesting.open();
            }
            '+' | '-' | '*' | '/' | '%' | '&' | '|' | '^' | '<' | '>' | '=' | '!' | '.' | '?'
            | '~' => {
                after_operand = false;
                nesting.unary();
            }
            _ => after_operand = false,
        }
        if glsl && nesting.depth > MAX_GLSL_NESTING {
            return Some((
                at,
                Excess::Nesting,
                format!(
                    "brackets, unary operators and statements nest more than {MAX_GLSL_NESTING} \
                     deep ({} here)",
                    nesting.depth
                ),
            ));
        }
    }
    None
}

/// How deep GLSL nests at a point of the text, each level a recursion of naga's GLSL parser: open
/// brackets, unary operators waiting for their operand, and statements waiting for their end
/// (`if`, `for`, `while`, `do` and `switch`, whose body may be another one without braces).
#[derive(Default)]
struct Nesting {
    depth: usize,
    /// The current level's unary operators still open.
    unary: usize,
    /// The current level's statements still open, the innermost last.
    statements: Vec<Statement>,
    /// A `;` or `}` ended a statement: which of the open ones end with it depends on what comes
    /// next ([`Nesting::settle`]).
    ended: bool,
    /// The unary operators and statements of the level outside each open bracket.
    outer: Vec<(usize, Vec<Statement>)>,
}

#[derive(Clone, Copy)]
enum Statement {
    /// An `if` whose body may still be followed by its `else`.
    If,
    /// A `do` whose `while` has not come yet.
    Do,
    /// A loop, a `switch`, an `if` in its `else`, a `do` in its `while`.
    Other,
}

impl Nesting {
    fn open(&mut self) {
        self.settle(None);
        let statements = std::mem::take(&mut self.statements);
        self.outer.push((self.unary, statements));
        self.unary = 0;
        self.depth += 1;
    }

    fn close(&mut self) {
        self.settle(None);
        let level = 1 + self.unary + self.statements.len();
        // An unmatched closer, which naga refuses, never takes the count below zero.
        self.depth = self.depth.saturating_sub(level);
        (self.unary, self.statements) = self.outer.pop().unwrap_or_default();
    }

    fn unary(&mut self) {
        self.settle(None);
        self.unary += 1;
        self.depth += 1;
    }

    /// An operand: the unary operators before it are done.
    fn operand(&mut self) {
        self.settle(None);
        self.depth -= std::mem::take(&mut self.unary);
    }

    /// A `;`, or a `}` that ended a block. A `;` right after a `}` (an initializer's) ends
    /// nothing more: the statements wait for what follows it.
    fn end_statement(&mut self) {
        self.depth -= std::mem::take(&mut self.unary);
        self.ended = true;
    }

    fn word(&mut self, word: &str) {
        let tail = self.settle(Some(word));
        if !tail && matches!(word, "if" | "for" | "while" | "do" | "switch") {
            self.statements.push(match word {
                "if" => Statement::If,
                "do" => Statement::Do,
                _ => Statement::Other,
            });
            self.depth += 1;
        }
        self.operand();
    }

    /// Ends what the last `;` or `}` ended, now that `next` (a word; `None` for any other token)
    /// shows which: from the innermost out, every open statement ends, except an `if` that `else`
    /// follows and a `do`, which waits for its `while`. Nothing else stays open, so loops one
    /// after another do not nest. `true` when `next` is a `do`'s `while`, which opens no loop.
    fn settle(&mut self, next: Option<&str>) -> bool {
        if !std::mem::take(&mut self.ended) {
            return false;
        }
        while let Some(last) = self.statements.last_mut() {
            match (*last, next) {
                (Statement::If, Some("else")) => {
                    *last = Statement::Other;
                    return false;
                }
                (Statement::Do, Some("while")) => {
                    *last = Statement::Other;
                    return true;
                }
                // naga refuses a `do`'s body followed by anything but `while`.
                (Statement::Do, _) => return false,
                _ => {
                    self.statements.pop();
                    self.depth -= 1;
                }
            }
        }
        false
    }
}

/// The rest of a number whose first digit, `first`, was read, tokenized by its radix as naga
/// does, so that no operator is taken into it: a decimal number's digits, point and fraction,
/// then an exponent after `e`/`E` whose sign belongs to it (`1.5e-3`); a hexadecimal number
/// (`0x`), whose `e` is a digit, with its exponent only after `p`/`P` (`0x1p+4`). After the
/// exponent's sign only digits and a suffix follow, so `0x1e+0x1e` is two numbers and an
/// operator.
fn number(first: char, chars: &mut std::iter::Peekable<std::str::CharIndices<'_>>) {
    let hex = first == '0' && chars.next_if(|&(_, c)| matches!(c, 'x' | 'X')).is_some();
    let digit = |c: char| {
        if hex {
            c.is_ascii_hexdigit()
        } else {
            c.is_ascii_digit()
        }
    };
    while chars.next_if(|&(_, c)| digit(c) || c == '.').is_some() {}
    let exponent = if hex { ['p', 'P'] } else { ['e', 'E'] };
    if chars.next_if(|&(_, c)| exponent.contains(&c)).is_some() {
        chars.next_if(|&(_, c)| matches!(c, '+' | '-'));
    }
    // The exponent's digits and a suffix (`f`, `h`, `i`, `u`), or the rest of a malformed
    // number, which naga refuses: never a sign.
    while chars
        .next_if(|&(_, c)| c.is_alphanumeric() || c == '_')
        .is_some()
    {}
}

/// What ends a `//` comment in WGSL.
fn is_line_end(c: char) -> bool {
    matches!(c, '\u{a}'..='\u{d}' | '\u{85}' | '\u{2028}' | '\u{2029}')
}

/// WGSL's blank space.
fn is_blank(c: char) -> bool {
    matches!(
        c,
        ' ' | '\t'..='\r' | '\u{85}' | '\u{200e}' | '\u{200f}' | '\u{2028}' | '\u{2029}'
    )
}

/// Runs `work` on a thread with a large stack and gives its result, or why it could not: the
/// thread would not start, or `work` panicked.
pub fn guarded<T: Send>(work: impl FnOnce() -> T + Send) -> Result<T, String> {
    std::thread::scope(|scope| {
        let thread = std::thread::Builder::new()
            .name("shader-check".to_owned())
            .stack_size(STACK_BYTES)
            .spawn_scoped(scope, work)
            .map_err(|e| format!("the shader check could not start: {e}"))?;
        thread.join().map_err(|panic| {
            format!(
                "the shader check panicked: {}",
                panic
                    .downcast_ref::<String>()
                    .map(String::as_str)
                    .or(panic.downcast_ref::<&str>().copied())
                    .unwrap_or("no reason given")
            )
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chain(terms: usize) -> String {
        format!("fn f(x: f32) -> f32 {{ return x{}; }}", "+x".repeat(terms))
    }

    /// The count is of the chain, not of the file: the longest chain that passes has exactly
    /// the cap's operators, and a statement ends the count.
    #[test]
    fn the_cap_is_on_one_expression_exactly() {
        assert_eq!(refusal(&chain(MAX_CHAINED_OPERATORS)), None);
        let (at, message) = refusal(&chain(MAX_CHAINED_OPERATORS + 1)).unwrap();
        assert!(message.contains("201 here"), "{message}");
        // The 201st `+`: after `fn f(x: f32) -> f32 { return x` (30 bytes) and 200 `+x`.
        assert_eq!(at, Some(30 + 2 * 200));
        // Many statements of a chain each are many expressions.
        let statements = format!("var a = 1.0; {}", "a = a + a * a - a; ".repeat(5_000));
        assert_eq!(refusal(&statements), None);
    }

    /// A hexadecimal number's `e` is a digit, not an exponent: an unspaced chain of hex terms
    /// is counted term by term, so it cannot pass as one long number.
    #[test]
    fn hex_numbers_cannot_hide_a_chain() {
        for (term, first) in [
            ("+0x1e", "0x1e"),
            ("-0x1E", "0x1E"),
            ("+0xe", "0xe"),
            ("+1e+1", "1e+1"),
        ] {
            // The `=` is one of the chain's operators, as the other tests count it.
            let refused = format!("let a = {first}{};", term.repeat(MAX_CHAINED_OPERATORS));
            assert!(refusal(&refused).is_some(), "{term}");
            let accepted = format!("let a = {first}{};", term.repeat(MAX_CHAINED_OPERATORS - 1));
            assert_eq!(refusal(&accepted), None, "{term}");
        }
        // Exponent signs still belong to their numbers: one operator per term.
        let floats = format!(
            "let a = 0x1p+4{};",
            "+0x1p-4".repeat(MAX_CHAINED_OPERATORS - 1)
        );
        assert_eq!(refusal(&floats), None);
        let floats = format!("let a = 1e+4{};", "+1e-4f".repeat(MAX_CHAINED_OPERATORS));
        assert!(refusal(&floats).is_some());
    }

    /// What is not a chain is not counted: unary signs (negative table entries), the points and
    /// exponent signs of numbers, comments, and words with a digit before a point.
    #[test]
    fn only_binary_operators_members_and_indexings_count() {
        let table = format!("const T = array<f32, 600>({});", "-1.5e-3, ".repeat(600));
        assert_eq!(refusal(&table), None);
        let points = format!("let a = 0.5{};", "+0.25".repeat(MAX_CHAINED_OPERATORS - 1));
        assert_eq!(refusal(&points), None);
        let comments = format!(
            "let a = x{};\n// {}\n/* {} /* nested {} */ */",
            "+x".repeat(100),
            "+x".repeat(500),
            "+x".repeat(500),
            "+x".repeat(500)
        );
        assert_eq!(refusal(&comments), None);
        // A member after a name ending in a digit is one, as are indexings and swizzles.
        for tail in [".x1", "[0]", ".xy"] {
            let source = format!("let a = v{};", tail.repeat(MAX_CHAINED_OPERATORS + 1));
            assert!(refusal(&source).is_some(), "{tail}");
        }
        // A non-ASCII letter ends a name; the operator after it still counts.
        let accented = format!("let a = e\u{301}{};", "+e\u{301}".repeat(300));
        assert!(refusal(&accented).is_some());
    }

    /// A comment is read as naga reads it: what hides a chain's rest from the count would
    /// have to end where naga's comment does not, and every end of a line is one.
    #[test]
    fn a_comment_cannot_hide_or_break_a_chain() {
        let half = "+x".repeat(150);
        // `;` inside a comment does not end the expression.
        for comment in ["// ;\n", "/* ; */", "/* /* ; */ ; */", "// {\u{85}"] {
            let source = format!("let a = x{half}{comment}{half};");
            assert!(refusal(&source).is_some(), "{comment:?}");
        }
        // An unterminated comment hides what follows it from naga as well.
        assert_eq!(refusal(&format!("/* {}", "+x".repeat(500))), None);
        // The end of a line other than `\n` ends a `//` comment, so the chain after it counts.
        let source = format!("let a = x{half}// c\u{2028}{half};");
        assert!(refusal(&source).is_some());
    }

    /// What the cap lets through parses and validates on an ordinary test thread's 2 MiB stack,
    /// in a debug build, whose frames are the largest: the claim that a shader which passed
    /// can be parsed again anywhere.
    #[test]
    fn the_longest_chains_allowed_parse_on_a_small_stack() {
        let wgsl = |tail: &str| {
            format!(
                "fn f(x: array<f32, 1>) -> f32 {{ var v = vec2<f32>(1.0); return {}; }}",
                tail
            )
        };
        let sources = [
            chain(MAX_CHAINED_OPERATORS),
            wgsl(&format!(
                "x[0]{}",
                "+x[0]".repeat(MAX_CHAINED_OPERATORS / 3)
            )),
            wgsl(&format!("v{}.x", ".yx".repeat(MAX_CHAINED_OPERATORS - 1))),
        ];
        for source in sources {
            assert_eq!(refusal(&source), None);
            let module = naga::front::wgsl::parse_str(&source).unwrap();
            naga::valid::Validator::new(
                naga::valid::ValidationFlags::all(),
                naga::valid::Capabilities::default(),
            )
            .validate(&module)
            .unwrap();
        }
    }

    /// naga's GLSL parser has no nesting limit of its own: GLSL's brackets count, up to the cap
    /// exactly, and a GLSL shader may be longer than a WGSL one.
    #[test]
    fn glsl_brackets_nest_up_to_the_cap_exactly() {
        let parens = |n: usize| format!("float x = {}a{};", "(".repeat(n), ")".repeat(n));
        assert_eq!(glsl_refusal(&parens(MAX_GLSL_NESTING)), None);
        let refused = |text: &str| glsl_refusal(text).map(|r| (r.excess, r.at));
        // The 201st `(`, after `float x = ` (10 bytes).
        let at = Some(10 + MAX_GLSL_NESTING);
        assert_eq!(
            refused(&parens(MAX_GLSL_NESTING + 1)),
            Some((Excess::Nesting, at))
        );
        let long = format!("{}// {}", parens(3), "x".repeat(MAX_SOURCE_BYTES));
        assert_eq!(glsl_refusal(&long), None);
        let longer = " ".repeat(MAX_GLSL_BYTES + 1);
        assert_eq!(refused(&longer), Some((Excess::Bytes, None)));
    }

    /// Unary operators wait for their operand, and statements without braces for their end (an
    /// `if`'s for an `else`, a `do`'s for its `while`): each is a level of naga's recursion
    /// until then, and no longer counts after.
    #[test]
    fn glsl_unary_operators_and_open_statements_nest() {
        let main = |body: String| format!("void main() {{ float a = 1.0, x; {body} }}");
        let n = MAX_GLSL_NESTING;
        let deep = [
            format!("x = {}a;", "- ".repeat(n)),
            format!("{}x = 1.0;", "if (b) ".repeat(n)),
            format!("{}x = 1.0;", "for (int i = 0; i < 2; i++) ".repeat(n)),
            format!(
                "if (a > 0.0) x = 0.0;{}",
                " else if (a > 0.0) { x = 1.0; }".repeat(n)
            ),
            format!(
                "{}x = 1.0;{}",
                "do ".repeat(n),
                " while (a > 0.0);".repeat(n)
            ),
        ];
        for body in deep {
            let excess = glsl_refusal(&main(body)).map(|r| r.excess);
            assert_eq!(excess, Some(Excess::Nesting));
        }
        let flat = [
            format!(
                "float t[600] = float[600]({}-1.0);",
                "-1.5e-3, ".repeat(599)
            ),
            "if (a > 0.0) x = 1.0; else x = 2.0; ".repeat(1000),
            format!(
                "if (a > 0.0) x = 0.0;{}",
                " else if (a > 0.0) { x = 1.0; }".repeat(100)
            ),
            "for (int i = 0; i < 2; i++) { x += a; } ".repeat(1000),
            "do { x += a; } while (x < 1.0); ".repeat(1000),
        ];
        for body in flat {
            assert_eq!(glsl_refusal(&main(body)), None);
        }
    }

    /// Only a statement still waiting for its end is a level: a loop that ended is not reopened
    /// by the next `while` (only a `do` waits for one), so a long run of loops one after another
    /// stays flat, and `do`s nested within the cap pass.
    #[test]
    fn glsl_successive_loops_do_not_nest() {
        let main = |body: String| format!("void main() {{ float a = 1.0, x; {body} }}");
        let flat = [
            "while (x < 1.0) x += a; ".repeat(1000),
            "while (x < 1.0) { x += a; } ".repeat(1000),
            "do x += a; while (x < 1.0); while (x < 2.0) x += a; ".repeat(1000),
            "if (a > 0.0) x = 1.0; while (x < 1.0) x += a; ".repeat(1000),
            format!(
                "{}x += a;{}",
                "do ".repeat(150),
                " while (x < 1.0);".repeat(150)
            ),
            format!(
                "{}x += a;{} ",
                "do { ".repeat(90),
                " } while (x < 1.0);".repeat(90)
            )
            .repeat(20),
        ];
        for body in flat {
            assert_eq!(glsl_refusal(&main(body)), None);
        }
    }

    /// What naga's preprocessor would act on is refused, not skipped: a `#define` or `#if` left in
    /// the text would change what naga parses after the count. `#version`, `#extension`, `#pragma`
    /// and `#line` at the start of a line change no code and are skipped.
    #[test]
    fn glsl_directives_naga_would_act_on_are_refused() {
        let kept = "#version 450 core\n  #extension GL_X : enable\n#pragma optimize(on)\n\
                    #line 4\nvoid main() {}";
        assert_eq!(glsl_refusal(kept), None);
        let define = format!(
            "#version 450\n#define T a{}\nfloat x = T;",
            "+a".repeat(150)
        );
        let refused = |text: &str| glsl_refusal(text).map(|r| (r.excess, r.at));
        // The `#` of the second line.
        assert_eq!(refused(&define), Some((Excess::Directive, Some(13))));
        for text in [
            "#if 0\n#endif",
            "#ifdef A\n#endif",
            "  # define A",
            "#undef A",
            "#include <a.glsl>",
            "#",
            // Not at the start of a line: no directive of naga's starts there.
            "float x = 1.0; #version 450",
        ] {
            let excess = glsl_refusal(text).map(|r| r.excess);
            assert_eq!(excess, Some(Excess::Directive), "{text}");
        }
    }

    /// A GLSL comment ends at its first `*/` (a WGSL one nests): what follows is code, counted.
    #[test]
    fn glsl_comments_do_not_nest() {
        let source = format!(
            "/* /* */ float x = a{};",
            "+a".repeat(MAX_CHAINED_OPERATORS)
        );
        assert_eq!(refusal(&source), None);
        // The 200th `+` (the `=` is the first operator), after `/* /* */ float x = a`.
        let refused = glsl_refusal(&source).map(|r| (r.excess, r.at));
        assert_eq!(refused, Some((Excess::Chain, Some(20 + 2 * 199))));
    }

    #[test]
    fn a_long_shader_is_refused_before_parsing() {
        let long = format!("{}// {}", chain(3), "x".repeat(MAX_SOURCE_BYTES));
        let (at, message) = refusal(&long).unwrap();
        assert_eq!(at, None);
        assert!(message.ends_with("a shader may hold 131072"), "{message}");
    }

    #[test]
    fn guarded_work_runs_on_a_large_stack_and_a_panic_is_a_refusal() {
        // Nested calls a thread's default 2 MiB cannot hold: 8 MiB of frames.
        fn deep(n: usize) -> usize {
            let pad = [n as u8; 1024];
            if n == 0 {
                0
            } else {
                deep(n - 1) + usize::from(pad[0] & 1)
            }
        }
        assert_eq!(guarded(|| deep(8_192).min(1)), Ok(1));
        let reason = guarded(|| -> () { panic!("boom") }).unwrap_err();
        assert_eq!(reason, "the shader check panicked: boom");
    }
}
