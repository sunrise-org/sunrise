//! The `gui_layer_pre` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0144

pub trait Hooks {
    /// Before one of vanilla's HUD layers is drawn (`RenderGuiLayerEvent.Pre`):
    /// draw here to put something below the layer, or set `canceled` to hide
    /// it. `layer` is the layer's id. The built-in registration anchors, in registration order, are:
    /// `minecraft:crosshair`, `hotbar`, `effects`, `boss_overlay`,
    /// `vehicle_health` (only riding a mount), `contextual_info_bar` (the
    /// mount's jump bar, the locator bar or the experience bar, with the level
    /// number), `player_health` (one layer for the health, armor, food and air
    /// rows), `selected_item_name`, `sleep_overlay`, `overlay_message` (the
    /// action bar), `title`, `scoreboard_sidebar`, `subtitle_overlay`,
    /// `chat` and `tab_list`. In the render pass, `player_health` fires before
    /// `contextual_info_bar`. A layer F1 hides fires nothing, and the chat
    /// screen draws the focused chat itself, so `chat` is only the unfocused
    /// lines. The camera overlays and the debug screen are not layers here:
    /// they are not drawn in this pass. The selected item name and action bar
    /// keep vanilla's fixed positions; adding bars doesn't move them. Cancel
    /// and redraw the corresponding layer if it needs to move. Draw with
    /// `native_fill`, `native_text`
    /// and the sprite helpers; `draw.size()` is the GUI
    /// size. A layer a mod registered (`register_gui_layers`) fires this
    /// and `gui_layer_post` too, with its own id, in its place among
    /// these.
    #[hook(
        side = "client",
        contract = "cancel",
        out = "canceled",
        neoforge = "RenderGuiLayerEvent.Pre",
        cancellable = true
    )]
    fn gui_layer_pre();
}
