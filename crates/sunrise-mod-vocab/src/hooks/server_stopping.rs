//! The `server_stopping` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// Fired once at the start of an orderly server shutdown (NeoForge's
    /// `ServerStoppingEvent` in `MinecraftServer.runServer`, after the tick
    /// loop exits normally and before `stopServer`), before network shutdown,
    /// player disconnect and save, final world save, level unloads, and
    /// `server_stopped`. Worlds are still loaded and players are still
    /// connected so a mod can save its own data or notify them. The shutdown waits
    /// for each listener, so one that blocks here delays the world save and the player
    /// disconnects: keep it short, and put joins and flushes of the mod's own threads in
    /// `server_stopped`. Not fired on a
    /// failed startup or when the tick loop exits with an error or panic: those
    /// paths jump directly to `stopServer` teardown and `server_stopped`.
    #[hook(
        target = "sunrise_gameplay::game::native_hooks::base::ServerGame<'_>::native_server_stopping",
        dispatch = "flat",
        side = "server",
        contract = "listen",
        neoforge = "ServerStoppingEvent",
        cancellable = false,
        version = 2
    )]
    fn server_stopping(dedicated: bool);
}
