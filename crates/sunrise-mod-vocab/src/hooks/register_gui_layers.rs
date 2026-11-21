//! The `register_gui_layers` hooks: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// Register, wrap or replace a HUD layer with an owned draw callback (NeoForge's
    /// `RegisterGuiLayersEvent`). Asked once at client startup. The installed body runs
    /// between `gui_layer_pre` and `gui_layer_post` unless Pre cancels it. Use
    /// `event.layers.register(id, placement, Box::new(draw))` to insert a new slot,
    /// or `replace_layer` and `wrap_layer` to change an existing slot's body.
    #[notes = r#"## HUD layers

`Hooks::register_gui_layers` is the client registration-phase counterpart of NeoForge 26.3's
`RegisterGuiLayersEvent`. It receives a `RegistrationContext` and a `GuiLayerRegistrar` as
`event.layers`, once at client startup. Call `layers.register(id, placement, Box::new(draw))`
with a namespaced ID, a `GuiLayerPlacement` and an owned draw callback. The four placements
are `BelowAll`, `Below(id)`, `Above(id)` and `AboveAll`, as `registerBelowAll`, `registerBelow`,
`registerAbove` and `registerAboveAll`. `Below` and `Above` name a vanilla layer (the IDs
`Hooks::gui_layer_pre` lists) or a layer already registered. Calls take effect in order, in mod
load order. Invalid, reserved or duplicate IDs and unknown relative anchors are logged and
refused. The client calls each installed callback as the mod that registered it.

Use `layers.replace_layer(id, Box::new(draw))` to discard an existing body, or
`layers.wrap_layer(id, Box::new(wrap))` to add an outer wrapper. Both keep the slot's ID,
position and draw conditions. A wrapper uses `ctx.with_draw` to paint with the current
scoped drawing context and `ctx.draw_previous()` to draw the previous body at that point.
It can omit that call to skip the previous body. Both methods refuse an unknown ID.

A layer fires `Hooks::gui_layer_pre` with its ID, runs its installed body, and fires
`Hooks::gui_layer_post` once. No body or Post runs if Pre cancels. Drawing a previous
body does not fire layer hooks again.

A vanilla layer Sunrise does not draw this frame (the crosshair in third person, the mount's
health off a mount) still keeps its place: the inserted layers around it draw in order, at the
next layer that is drawn. F1 and a canceled `Hooks::render_gui_pre` hide all inserted layers;
a replacement or wrapper inherits its slot's draw conditions. A mod that needs a foreground
pass regardless of F1 uses `Hooks::gui_render_post`."#]
    #[hook(
        receiver = "associated",
        dispatch = "flat",
        side = "client",
        version = 2,
        contract = "listen",
        context = "registration",
        neoforge = "RegisterGuiLayersEvent"
    )]
    fn register_gui_layers(layers: &mut dyn GuiLayerRegistrar);
}
