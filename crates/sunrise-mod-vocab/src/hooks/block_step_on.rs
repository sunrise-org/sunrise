//! The `block_step_on` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0085

pub trait Hooks {
    /// `Block.stepOn` for a mod block, on its authoritative server level.
    /// No self: the simulation owns the entity during this callback.
    #[hook(
        target = "sunrise_client::game::native_hooks::base::Game<'_>::native_block_step_on",
        receiver = "associated",
        dispatch = "flat",
        side = "server",
        contract = "listen",
        neoforge = "none (vanilla Block.stepOn; NeoForge adds no event or extension for it)"
    )]
    fn block_step_on(
        dimension: &str,
        entity: i32,
        kind: &str,
        state: BlockState,
        position: [i32; 3],
    );
}
