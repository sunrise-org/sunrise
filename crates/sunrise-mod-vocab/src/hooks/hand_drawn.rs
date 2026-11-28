//! The `hand_drawn` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// The first-person hand is drawn (see [`HandDrawn`]), on every frame [`world_drawn`]
    /// runs, with no hand too. This is the frame's moment after the hand pass; the per-hand
    /// callback before submit, NeoForge's `RenderHandEvent`, is [`render_hand`].
    #[hook(
        side = "client",
        contract = "listen",
        target = "mod_render::render::hand_drawn",
        neoforge = "none (Sunrise-only; NeoForge has no counterpart)"
    )]
    fn hand_drawn(hand: &mut HandDrawn<'_>);
}
