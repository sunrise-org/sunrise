//! The `world_lifecycle` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0003

pub trait Hooks {
    /// The world's phases, read-only; `error_text` carries the engine's
    /// own open-failure text, empty otherwise. Server-side. A server loop or final
    /// save failure fires `WorldCloseFailed` with the error text.
    #[hook(
        target = "sunrise_client::game::native_hooks::base::Game<'_>::native_world_lifecycle",
        dispatch = "flat",
        side = "server",
        contract = "listen",
        version = 2,
        neoforge = "ServerStartingEvent, ServerStartedEvent, ServerStoppingEvent, ServerStoppedEvent"
    )]
    fn world_lifecycle(context: &LifecycleContext, phase: WorldPhase, error_text: &str);
}
