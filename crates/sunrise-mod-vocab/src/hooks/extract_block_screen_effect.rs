//! The `extract_block_screen_effect` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// Before one of the first-person view's block overlays (`ScreenEffectRenderer`) is drawn,
    /// once a frame each is shown: `Block` with the view-blocking `state` at `pos` the camera is
    /// inside, `Water` with the `state` at the eye's block `pos` while the eye is under water, and
    /// `Fire` with fire's default `state` at the player's block `pos` while they burn. Never for a
    /// spectator; `Fire` only in first person. Cancel to leave that one vanilla overlay out.
    /// Use [`ExtractBlockScreenEffectEvent::set_custom_renderer`] to prepend or replace its quads.
    /// `partial_tick` is the world's. Client-side.
    #[hook(
        receiver = "read",
        dispatch = "flat",
        side = "client",
        contract = "cancel",
        out = "canceled",
        neoforge = "ExtractBlockScreenEffectEvent",
        cancellable = true,
        version = 2
    )]
    #[notes = r##"## Block screen effects

`extract_block_screen_effect` fires before block, water and fire overlays are drawn. The event's `set_custom_renderer` is NeoForge's `setCustomRenderer`: the last eligible listener to set a renderer for an overlay wins. Cancellation skips listeners that did not opt in with `#[listener(receive_cancelled)]`, but it does not clear a renderer. A canceled overlay with no renderer draws nothing.

After extraction, the game calls each selected renderer's `submit` in block, water, fire order. It submits just before its own overlay and can return `true` to suppress that overlay's vanilla quads. Earlier overlays, later overlays and the HUD's camera overlays are unaffected. `CameraDrawContext` takes camera-space quads under the first-person hand's projection, after the level and hand and before the GUI. The player and camera can be read on `ClientHost`; `partial_tick` is `getWorldPartialTick` at both stages. Nothing drawn outlives the frame, and no GPU handle is exposed."##]
    fn extract_block_screen_effect(
        effect: mod_api::BlockScreenEffect,
        state: mod_api::BlockState,
        pos: BlockPos,
        partial_tick: f32,
        canceled: &mut bool,
        renderer: &mut Option<mod_api::__private::SelectedBlockScreenEffectRenderer>,
    );
}
