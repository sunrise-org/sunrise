//! The built-in player layer's draw-scoped texture decision (see `hooks/mod.rs`).

pub trait Hooks {
    /// Select the texture of `layer` for `player` (its UUID), after the draw's
    /// avatar and layer visibility checks. `local` identifies this client's own
    /// player; `view` distinguishes the world from either inventory preview.
    /// Called at `CapeLayer.submit` and `WingsLayer.submit`, with a read-only
    /// client host. It does not change the profile, equipment, or any other draw.
    ///
    /// The choice starts at `Keep`. Use `event.choose(...)` to propose a
    /// choice. Listeners run in dependency order, ignoring listener priority, and
    /// the last accepted choice wins. A refusal preserves the previous choice;
    /// a failing listener's changes are discarded. A cape may be supplied even
    /// if the profile has none. Neither choice can hide a layer, and a layer
    /// hidden by a world render pre-hook is not queried.
    ///
    /// The decision borrows the frame's live texture lookup for this draw.
    #[hook(
        side = "client",
        contract = "collect",
        out = "selection",
        isolate = true,
        event_per_listener = true,
        neoforge = "none (vanilla CapeLayer.submit and WingsLayer.submit)"
    )]
    fn resolve_player_layer();
}
