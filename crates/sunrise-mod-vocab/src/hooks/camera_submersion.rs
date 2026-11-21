//! The `camera_submersion` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// The fluid the camera at `camera` is in (`Camera.getFluidInCamera`), as the fog type the
    /// client reads: `fog_type` comes in as the game's answer, after `state_at_viewpoint` has had
    /// its say on each block it reads. Set it to `Atmospheric` to clear water, lava and powder snow
    /// submersion, or to a fluid to force it; later listeners see the earlier ones' value. Every
    /// client reader of the camera's fluid reads the answer: the fluid field of view, the frame's
    /// fog, the sky pass and the overworld sky's dark disc (skipped under water), the screen
    /// overlays, water vision and conduit power's underwater vision, and a shader pack's
    /// `isEyeInWater`. Asked several times a frame, once per reader, and not always at the same
    /// point (the fog asks at the camera, the overlays at the eye): answer from the arguments alone,
    /// the same way for the same point within a frame, as nothing keeps the first answer. The
    /// camera's position can't be changed here. Server and gameplay fluid checks (an entity in
    /// water, drowning, swimming, the sculk vibrations' water test) don't read it. Client-side;
    /// with no listener the read costs nothing more. Beyond NeoForge: there is no counterpart.
    /// Diverges from upstream: new hook.
    #[hook(
        side = "client",
        contract = "listen",
        neoforge = "none (vanilla Camera.getFluidInCamera; NeoForge adds no event for it)"
    )]
    fn camera_submersion();
}
