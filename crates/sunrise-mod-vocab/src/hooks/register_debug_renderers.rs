//! The `register_debug_renderers` hooks: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// Collect world-space debug renderers at each client renderer-list refresh (NeoForge's
    /// `RegisterDebugRenderersEvent`). Push `DebugRenderer::new(id, draw)` for each renderer
    /// on every refresh, using the ID of its startup `DebugEntry::renderer` toggle. A selection
    /// rebuild can refresh the list even when the same toggles end up enabled.
    #[notes = r#"## Debug renderers

`Hooks::register_debug_renderers` is the client counterpart of NeoForge 26.3's
`RegisterDebugRenderersEvent`, and like it fires during vanilla's
`DebugRenderer.refreshRendererList`: once as the client starts, on each login, on the first view
of a world since the start, and on the next view of a world after the debug selection changes.
That is a status set in the debug options or by a key, a profile loaded, the F3 overlay opened or
closed, or reduced debug info rebuilt. Respawn also invalidates the list for the next view.
The next world view refreshes after a rebuild even if the enabled toggles are unchanged, such as
when the overlay opens and closes between views or reduced debug info is set to the same value.
Drawing the title or connect screen doesn't refresh anything. The hook receives a
`RegistrationContext`, so it reads no world. Each mod receives an empty `event.renderers`
batch and pushes `DebugRenderer::new(id, draw)` per renderer.

Each refresh replaces the active list, so push every renderer you want to provide and capture
its per-refresh state in the callback. Pushing a renderer does not list an option: register
`DebugEntry::renderer(id)` once at startup with the same ID in `Hooks::register_debug_entries`.
The switch starts off, and its status (`alwaysOn`, `inOverlay`, `never`) is saved in
`debug-profile.json` under its full ID. Reduced debug info hides it. A renderer without a
matching renderer switch is logged and left out. Of active renderers with the same ID, the
first registered wins.

For each world view, the client calls only the installed callbacks, in registration order,
under their registering mods' identities, after `submit_custom_geometry`. It passes the current
`ClientHost`, `WorldRenderView` and the same `WorldDrawContext`, so
[its limits and rules](#custom-world-geometry) apply and are shared. The ID selects the switch;
it isn't passed to the callback. Output clears every view, and callbacks don't run on the title
or connect screens."#]
    #[hook(
        receiver = "associated",
        dispatch = "flat",
        side = "client",
        contract = "collect",
        out = "renderers",
        context = "registration",
        answered_by = "mod",
        owned_answer_batch = true,
        neoforge = "RegisterDebugRenderersEvent",
        version = 2
    )]
    fn register_debug_renderers(renderers: &mut Vec<Answered<DebugRenderer>>);
}
