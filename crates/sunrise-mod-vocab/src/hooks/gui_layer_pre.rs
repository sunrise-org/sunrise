//! The `gui_layer_pre` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0144

pub trait Hooks {
    /// Before a vanilla or mod HUD layer (`RenderGuiLayerEvent.Pre`). Draw below the
    /// layer through `event.draw`, or return `Flow::Cancel` to skip its entire body
    /// and `gui_layer_post`. Cancellation skips painting, not the layer's game logic.
    /// A wrapped or replaced layer fires this once, with the slot's original ID.
    /// To change a layer's body rather than veto the whole slot, wrap or replace it
    /// at startup in `register_gui_layers`.
    ///
    /// The vanilla anchors, in HUD order, are `minecraft:crosshair`, `hotbar`,
    /// `effects`, `boss_overlay`, `vehicle_health` (while riding a mount),
    /// `player_health` (one layer for the health, armor, food and air rows),
    /// `contextual_info_bar` (jump, locator or experience bar, with the level number),
    /// `selected_item_name`, `sleep_overlay`, `overlay_message` (action bar), `title`,
    /// `scoreboard_sidebar`, `subtitle_overlay`, `chat` and `tab_list`. A slot skipped
    /// by its draw conditions (for example, one F1 hides) fires neither hook. Focused
    /// chat still draws at the chat screen's paint site. The camera overlays and debug
    /// screen are not HUD layers. The selected item name and action bar keep vanilla's
    /// fixed positions; adding bars doesn't move them. Cancel and redraw the
    /// corresponding layer if it needs to move.
    #[hook(
        target = "sunrise_client::game::native_hooks::base::Game<'_>::native_gui_layer_pre",
        receiver = "read",
        dispatch = "flat",
        side = "client",
        contract = "cancel",
        out = "canceled",
        neoforge = "RenderGuiLayerEvent.Pre",
        cancellable = true
    )]
    fn gui_layer_pre(layer: &str, draw: &mut dyn DrawContext, canceled: &mut bool);
}
