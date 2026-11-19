//! A line's macros are held to the bytes the line may take, each copy counted before it is made:
//! a few hundred bytes of macros that would make megabytes never allocate them. The allocator
//! counts on the test's own thread only, so the other tests of this binary don't disturb it.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::io::Write;

use sunrise_shader_api::guard::MAX_GLSL_BYTES;
use sunrise_shaders::{Bound, Macros, Pack, preprocess};

thread_local! {
    /// The largest allocation this thread made while counting; `None` when it is not counting.
    static LARGEST: Cell<Option<usize>> = const { Cell::new(None) };
}

fn note(size: usize) {
    // `try_with`: a thread being torn down still allocates.
    let _ = LARGEST.try_with(|largest| {
        if let Some(n) = largest.get() {
            largest.set(Some(n.max(size)));
        }
    });
}

/// The system allocator, recording the largest request of a counting thread.
struct Largest;

// SAFETY: every call forwards to `System` with the same arguments; only a counter is added.
unsafe impl GlobalAlloc for Largest {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        note(layout.size());
        // SAFETY: the caller's contract for `alloc` is passed on unchanged.
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: `ptr` came from `System` with this layout.
        unsafe { System.dealloc(ptr, layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        note(new_size);
        // SAFETY: the caller's contract for `realloc` is passed on unchanged.
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static ALLOCATOR: Largest = Largest;

/// What `work` returns, and the largest allocation it made on this thread.
fn largest_allocation<T>(work: impl FnOnce() -> T) -> (T, usize) {
    LARGEST.set(Some(0));
    let out = work();
    (out, LARGEST.replace(None).unwrap_or(0))
}

/// `depth` calls of `call`, each in the last one's argument, around `inner`.
fn nested(call: &str, depth: usize, inner: &str) -> String {
    format!(
        "{}{inner}{}",
        format!("{call}(").repeat(depth),
        ")".repeat(depth)
    )
}

/// An argument used many times inside its own calls (64 × 64 copies of a 1000-byte name: 4 MB)
/// and an argument pasted to itself call after call (a token doubling to 4 MiB) are refused for
/// their bytes within a 64 KiB budget, and nothing near their size is allocated on the way.
#[test]
fn macros_past_the_byte_budget_stop_before_their_text_is_made() {
    const BUDGET: usize = 64 << 10;
    // `W`, a long name; `T`, which uses its argument 64 times; `D`, which expands its argument
    // once and has `P` paste it to itself raw.
    let mut m = Macros::default();
    m.define("W", "w".repeat(1000));
    m.define_function("T", vec!["x".into()], &"x ".repeat(64));
    m.define_function("D", vec!["x".into()], "P(x)");
    m.define_function("P", vec!["a".into()], "a ## a");
    for text in [nested("T", 2, "W"), nested("D", 22, "w")] {
        let (expanded, largest) = largest_allocation(|| m.expand_bounded(&text, BUDGET));
        assert_eq!(expanded.map(|e| e.len()), Err(Bound::Bytes), "{text}");
        assert!(
            largest < 4 * BUDGET,
            "{text}: largest allocation {largest} bytes"
        );
    }
    // Within the budget the same macros expand.
    let fits = m.expand_bounded(&nested("D", 3, "w"), BUDGET);
    assert_eq!(fits.as_deref(), Ok("wwwwwwww"));
}

/// A line of more tokens than an expansion may make, within its bytes, is refused for its tokens
/// as the lexer reads it: no token is made for each of its million source tokens (48 MB of them).
#[test]
fn a_line_past_the_token_budget_is_not_lexed_to_its_end() {
    let text = "+".repeat(1 << 20);
    let (expanded, largest) =
        largest_allocation(|| Macros::default().expand_bounded(&text, 2 << 20));
    assert_eq!(expanded.map(|e| e.len()), Err(Bound::Tokens));
    assert!(largest < 8 << 20, "largest allocation {largest} bytes");
}

/// A pack's line whose macros would make twice what a stage may hold (8 MB from a few KB) is
/// refused at its own file and line before its text is made, and the stage reads no further.
#[test]
fn a_stage_line_past_the_budget_is_refused_where_it_is_written() {
    let (w, t) = ("w".repeat(2000), "x ".repeat(64));
    let defines =
        format!("#define W {w}\n#define T(x) {t}\n#define D(x) P(x)\n#define P(a) a ## a\n");
    for line in [
        "float a = T(T(W));".to_owned(),
        format!("float b = {};", nested("D", 23, "w")),
    ] {
        let lib = format!("{defines}{line}\n");
        let pack = pack(&[
            ("program/final.fsh", "#include \"lib.glsl\"\nfloat c;\n"),
            ("program/lib.glsl", &lib),
        ]);
        let (p, largest) =
            largest_allocation(|| preprocess(&pack, "program/final.fsh", &Macros::default()));
        let at: Vec<(&str, u32)> = p
            .diagnostics
            .iter()
            .map(|d| (p.files[d.origin.file as usize].as_str(), d.origin.line))
            .collect();
        assert_eq!(at, [("program/lib.glsl", 5)], "{line}");
        assert!(!p.code.contains("float c"), "{line}");
        assert!(
            largest < MAX_GLSL_BYTES,
            "{line}: largest allocation {largest} bytes"
        );
    }
}

/// A pack of `files` under `shaders/`.
fn pack(files: &[(&str, &str)]) -> Pack {
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (path, text) in files {
        let options = zip::write::SimpleFileOptions::default();
        zip.start_file(format!("shaders/{path}"), options).unwrap();
        zip.write_all(text.as_bytes()).unwrap();
    }
    Pack::from_zip_bytes("P.zip", zip.finish().unwrap().into_inner()).unwrap()
}
