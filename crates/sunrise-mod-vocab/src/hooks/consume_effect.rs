//! The `consume_effect` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0086

pub trait Hooks {
    /// A custom entry of the stack's effective `consumable`, in list order,
    /// interleaved with built-in effects. `stack` is the read-only snapshot
    /// before consumption shrinks it, including components. `user` is the
    /// player or mob consuming it. An unclaimed kind does nothing.
    #[hook(
        side = "server",
        contract = "listen",
        version = 2,
        neoforge = "none (runtime ConsumeEffect.apply; RegisterEvent registers effect types, not invocations)"
    )]
    #[notes = "`consume_effect` receives the custom kind, retained definition data, pre-shrink stack snapshot and consuming entity (player or mob). Entries come from the effective consumable, including stack patches, and run in list order between the built-in entries."]
    fn consume_effect();
}
