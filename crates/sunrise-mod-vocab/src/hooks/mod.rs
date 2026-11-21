//! Declarations of the mod API's hooks and draw contexts: the catalog.
//!
//! This module is never compiled (`#[cfg(any())]` in `lib.rs`): `sunrise-mod-api-gen` reads these
//! files with `syn` and builds the `Hooks` trait, its dispatch, the game's entry points and the
//! rustdoc from them. Lint checks these files directly with `rustfmt`. What the game does for a mod, the host
//! methods and the handles', is not declared here but written by hand, one file per area, in `../host/<area>.rs`.
//! Each host method has one `#[access(Level)]` declaration; its default and the game's forwarder
//! are generated. Implement the same-named inherent `Game` method, without a `native_` prefix.
//! Hook entry points keep their `native_` names.
//!
//! Each hook has its own file: a bodiless `fn name(parameters);` with its rustdoc. List the
//! parameters in vocabulary types, before they become event fields, a reply, or a cancellation.
//! Do not repeat the receiver or a phase context in the parameter list. Only a parameter used by
//! the engine but not by the listener is marked `#[engine]`, such as a level hook's tick queue.
//! Separate files hold draw contexts and general notes. The declaration traits are:
//!
//! - `Hooks`: `#[hook(side = "...", contract = "...", ...)]`, one per moment the game tells mods about.
//! - `DrawContexts`: `#[draw_context(surface = "...", name = "...")]`, a drawing surface hooks draw on.
//! - `Notes`: `#[notes = "..."]`, prose for the hook-notes page that belongs to no one hook.
//!   A hook's own prose is a `#[notes]` beside its declaration; a host method's, a `Notes` entry.
//!
//! A hook's arguments:
//!
//! | Argument | Meaning |
//! |---|---|
//! | `side` | `"server"`, `"client"` or `"both"`; required |
//! | `contract` | `"listen"`, `"cancel"`, `"collect"` or `"replace"`; required |
//! | `out` | the event field a cancel, collect or replace hook writes; not for `"listen"` |
//! | `scope`, `key` | a Part method: the kind of Part (`"block"`, `"block_entity"`, `"item"`, `"menu"`, `"fluid"`, `"entity"`) and the hook argument naming what it is attached to |
//! | `answered_by` | `"mod"`: a global replace hook whose output also names the mod that answered, or a global collect hook whose output is `Vec<Answered<T>>`: each mod pushes `T`s into its own empty batch and dispatch names that mod on each entry |
//! | `isolate` | `true` on a global cancel hook: its dispatch runs every listener inside `catch_unwind` — one that unwinds keeps the **requested** outputs (its writes roll back), is skipped by this hook for the session, and no later listener runs on that invocation (#2414 ruling). On a global collect hook with `answered_by = "mod"` a listener that unwinds loses only its own batch and is skipped by this hook for the session; the other listeners still answer |
//! | `output_validator` | a path on an `isolate` cancel hook: `fn(&accepted, ..., &proposed, ...) -> Result<(), &'static str>` takes the scalar/vector outputs in declaration order for each set and checks a listener's proposal against the preceding accepted one. A refusal rolls back all outputs including cancellation, logs the mod and field once per tick, and lets later listeners run. An accepted cancellation cannot be cleared. |
//! | `host`, `context` | `"read"` or `"level"`; `"registration"`, `"data_load"`, `"loop"` or `"worldgen"` |
//! | `neoforge`, `fabric`, `paper` | the counterpart's name, or a list of them, for the rustdoc and the gap lists |
//! | `cancellable` | whether the NeoForge counterpart event is cancellable (documentation, not the contract) |
//! | `version` | the hook's contract version; left out when it is 1 |
//! | `target` | the function that fires the hook; defaults to `Game::native_<name>` in the `native_hooks` module with the same filename as the declaration |
//! | `receiver` | a game target borrows `&mut self` by default; `"read"` borrows `&self`, `"associated"` has no receiver |
//! | `dispatch` | a game target takes the generated engine carrier by default; existing flat, already-converted (`"api_carrier"`), and direct bindings name their shape explicitly |
//!
//! Every hook needs `///` lines: they are the rustdoc of its trait method, and a hook without them
//! is refused. `HookId` values are process-local dispatch indexes, not persistent API identities.
//! Native mods compile for the selected revision. Existing split hooks retain their generated order
//! through each file's `// catalog-order: N` comment.
//! These frozen ordinals only keep the split's output byte-identical; they provide no cross-build
//! compatibility. New hooks omit the comment and follow in file-name order. No shared list is
//! edited: the generator discovers the files directly.
//! Each new hook goes in a file of its own.
//! The JSON catalog (`native-hooks.json`,
//! `tools/native-hook-fragments/`) is retired and a file there fails the build.
//!
//! A new game hook takes its generated `fire::<Name>Call` carrier, built from these declarations
//! and the generator's engine value and draw mappings. A hook with no event parameters needs no
//! carrier. Start the method body with `let _ = call;` (or `let _ = (every, parameter);` with
//! additional inputs); `cargo run -p xtask -- mod-api` fills in the call to the typed `fire_*`
//! adapter. A mismatch fails to compile at that call.
