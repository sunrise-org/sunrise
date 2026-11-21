//! The `render_player_post` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// After a player is drawn (NeoForge's `RenderPlayerEvent.Post`): once a frame for each player
    /// whose `render_player_pre` wasn't canceled, after its `render_living_post`, or right after its
    /// `render_living_pre` when only that one was canceled. `player`, `who` and `draw` are as
    /// `render_player_pre` has them, and what it draws is kept. `event.model` borrows the same
    /// final [`crate::FrameModel`] as `render_living_post`, read-only for the callback. Its poses
    /// and effective visibility include this draw's animation, Skin Customization and Pre edits.
    /// It is `None` if no body pose was produced, including when only Living Pre canceled. The
    /// player's name tag, leash, flames and shadow are drawn after it, as in `render_living_post`.
    /// In the ordered body pass, this player's pair finishes at its slot in level join order,
    /// before the next body submits, including bodies drawn by a general entity renderer.
    #[hook(
        receiver = "read",
        dispatch = "flat",
        side = "client",
        version = 5,
        contract = "listen",
        neoforge = "RenderPlayerEvent.Post",
        cancellable = false
    )]
    fn render_player_post(
        player: &MobRender<'_>,
        who: &PlayerRender<'_>,
        model: Option<&dyn FrameModel>,
        draw: &mut dyn BlockEntityDrawContext,
    );
}
