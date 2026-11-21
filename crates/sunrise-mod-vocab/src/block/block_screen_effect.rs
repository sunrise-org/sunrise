use crate::{BlockPos, BlockState, CameraDrawContext, ClientHost, ExtractBlockScreenEffectEvent};

/// Which of `ScreenEffectRenderer`'s overlays a block puts over the first-person view
/// (NeoForge's `ExtractBlockScreenEffectEvent.OverlayType`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
// exhaustive: a closed set the game matches without a wildcard
#[derive(sunrise_mod_api_macros::Wire)]
pub enum BlockScreenEffect {
    /// The player is on fire: the fire sprite over the lower view.
    Fire,
    /// The camera is inside a view-blocking block: its particle texture.
    Block,
    /// The eye is under water: the underwater texture.
    Water,
}

// no wire: the renderer is an in-process callback.
/// A renderer selected for one block overlay by [`ExtractBlockScreenEffectEvent::set_custom_renderer`].
/// Reuse a static renderer rather than allocating one at every extraction.
pub trait BlockScreenEffectRenderer: Sync {
    /// Submit this overlay with read-only access to the client. Return `true` to suppress
    /// its vanilla quads, or `false` to leave them after yours unless extraction canceled them.
    /// Neither the host nor the drawing context can outlive the call.
    fn submit(&self, host: &dyn ClientHost, render: BlockScreenEffectRender<'_>) -> bool;
}

// no wire: the drawing context is a frame-local borrow.
/// The overlay's original extraction inputs and its camera-space drawing scope.
#[non_exhaustive]
pub struct BlockScreenEffectRender<'a> {
    /// The overlay selected at extraction.
    pub effect: BlockScreenEffect,
    /// The block state associated with this overlay.
    pub state: BlockState,
    /// The position associated with this overlay.
    pub pos: BlockPos,
    /// The same world partial tick as at extraction.
    pub partial_tick: f32,
    /// Whether the HUD is hidden.
    pub hide_gui: bool,
    /// Camera-space quads, under the first-person hand's projection and before the GUI.
    pub draw: &'a mut dyn CameraDrawContext,
}

// hidden: only the engine has the original extraction, frame timing, and draw scope.
#[doc(hidden)]
pub fn block_screen_effect_render(
    effect: BlockScreenEffect,
    state: BlockState,
    pos: BlockPos,
    partial_tick: f32,
    hide_gui: bool,
    draw: &mut dyn CameraDrawContext,
) -> BlockScreenEffectRender<'_> {
    BlockScreenEffectRender {
        effect,
        state,
        pos,
        partial_tick,
        hide_gui,
        draw,
    }
}

impl ExtractBlockScreenEffectEvent<'_> {
    /// Select a renderer for this overlay. The last eligible listener to select one wins;
    /// listeners that do not select one leave the previous choice in place. Cancellation
    /// skips subsequent listeners unless they opt into `#[listener(receive_cancelled)]`.
    /// The selected renderer still submits even if the overlay is canceled.
    pub fn set_custom_renderer(&mut self, renderer: &'static dyn BlockScreenEffectRenderer) {
        *self.renderer = Some(SelectedBlockScreenEffectRenderer {
            owner: crate::content::crash::current_native_mod(),
            renderer,
        });
    }
}

// no wire: a selected callback and its attribution belong to this process.
/// Engine-owned selection, carrying the listener's attribution to submission.
// hidden: only the game may submit a selected callback.
#[doc(hidden)]
#[derive(Clone, Copy)]
// exhaustive: only the extraction event can construct this private-field selection.
pub struct SelectedBlockScreenEffectRenderer {
    owner: Option<&'static str>,
    renderer: &'static dyn BlockScreenEffectRenderer,
}

impl SelectedBlockScreenEffectRenderer {
    /// Submit the selected overlay under the selecting listener's mod scope.
    pub fn submit(self, host: &dyn ClientHost, render: BlockScreenEffectRender<'_>) -> bool {
        let _scope = self
            .owner
            .map(|id| crate::mark_mod(id, "block_screen_effect_renderer::submit"));
        match crate::runtime::refusable(crate::runtime::outside_world(), || {
            self.renderer.submit(host, render)
        }) {
            Ok(replace) => replace,
            Err(refusal) => {
                refusal.report();
                false
            }
        }
    }
}
