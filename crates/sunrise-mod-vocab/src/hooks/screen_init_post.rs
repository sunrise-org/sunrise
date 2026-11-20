//! The `screen_init_post` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// Before the title or full pause menu initializes its stock listeners
    /// (`ScreenEvent.Init.Pre`). Cancelling skips those listeners; Post still
    /// fires on the resulting empty build and can register its own controls.
    #[hook(
        side = "client",
        contract = "cancel",
        out = "canceled",
        neoforge = "ScreenEvent.Init.Pre",
        cancellable = true
    )]
    fn screen_init_pre();

    /// After the screen builds its stock widgets and establishes initial focus,
    /// before it draws or accepts input (`ScreenEvent.Init.Post`). A rebuild
    /// fires again; an unchanged frame does not. `widgets` exposes the staged
    /// listener list on `title` and `paused`.
    #[hook(
        side = "client",
        contract = "listen",
        neoforge = "ScreenEvent.Init.Post",
        version = 2,
        cancellable = false
    )]
    fn screen_init_post();
}
