//! The `spawn_change` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0267

pub trait Hooks {
    /// The world's spawn point changed to `pos` in level `dimension`, facing `angle`.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)",
        version = 2
    )]
    fn spawn_change();
}
