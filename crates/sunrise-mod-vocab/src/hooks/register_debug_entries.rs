//! The `register_debug_entries` hooks: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// Register F3 debug entries once at client startup (NeoForge's `RegisterDebugEntriesEvent`).
    /// Each mod receives an empty batch. Push `DebugEntry::text` with its line callback or
    /// `DebugEntry::renderer` for a renderer toggle. Text callbacks run only while their entry
    /// is visible. Malformed or duplicate IDs, and IDs in the `minecraft` namespace, are
    /// logged and left out.
    #[notes = r#"## Debug screen entries

`Hooks::register_debug_entries` is the client registration-phase counterpart of NeoForge
26.3's `RegisterDebugEntriesEvent`. It receives a `RegistrationContext` once at client startup
and gives each mod an empty `event.entries` batch. Use a non-vanilla `<namespace>:<path>`
for the ID. A malformed or duplicate ID, or one in the `minecraft` namespace, is logged
and left out.

Declare text with `DebugEntry::text(id, group, lines)`. The `DebugGroup` chooses which
vanilla lines it joins: `Position`, `Light`, `Memory`, `LookingAtBlock`, `LookingAtEntity`,
`LookingAtFluid` or `ChunkGeneration`. The client owns the callback for its lifetime. Only
while the entry is visible, the client calls the callback under the registering mod's identity,
with the current `&dyn ClientHost` and a `&mut Vec<String>` to append lines to. Legacy colour
codes (§) work. If the callback appends nothing, nothing joins the group. The lines are
drawn locally, never uploaded or recorded.

Declare a renderer switch with `DebugEntry::renderer(id)` (vanilla's `DebugEntryNoop`). It has
no text group or callback. Use the same ID as its [renderer](#debug-renderers).

Each entry appears in the corresponding text or renderer section of the debug options,
sorted by ID with vanilla's, and starts off. Its status (`alwaysOn`, `inOverlay`, `never`) is
saved in `debug-profile.json` under the full ID. Reduced debug info hides native entries.
Native text is appended after vanilla's entries in registration order."#]
    #[hook(
        receiver = "associated",
        dispatch = "flat",
        side = "client",
        contract = "collect",
        out = "entries",
        context = "registration",
        answered_by = "mod",
        owned_answer_batch = true,
        neoforge = "RegisterDebugEntriesEvent",
        version = 2
    )]
    fn register_debug_entries(entries: &mut Vec<Answered<DebugEntry>>);
}
