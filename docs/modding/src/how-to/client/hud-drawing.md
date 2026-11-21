# Drawing on the HUD and in screens

A HUD layer, a screen or a tooltip hook draws through the `DrawContext` it is handed, in GUI pixels. Each call
is one of vanilla's `GuiGraphics` calls: `fill`, `text`, `sprite`, `item_stack` and the others. `gui_paint`
takes a buffer of `GuiDraw` ops for the same calls, with `Push` / `Pop` for the pose.

`ClientHost::debug_overlay_visible()` reports whether the client draws its debug overlay this frame. In
`render_gui_post`, use it to leave space for the overlay. Always-on debug entries can make it visible with
F3 off. F1 hides the overlay unless a screen is open. The read is available only on the client host.

## Registering a HUD layer

In `Hooks::register_gui_layers`, call `event.layers.register(id, placement, Box::new(draw))` on the
`GuiLayerRegistrar`. Registration runs once at client startup.
Use your own namespaced ID and a `GuiLayerPlacement`: below or above all layers, or immediately
below or above a vanilla anchor or a layer already registered. The client logs and refuses
invalid IDs, duplicate IDs and unknown anchors; `register` returns whether it installed the layer.

The client keeps the callback for the session and calls it as the mod that installed it. Capture
any state the layer needs in the closure, and use the current `ClientHost` and `DrawContext`
passed on each draw. The callback runs only for its own layer, and not when the HUD is hidden
or `gui_layer_pre` cancels the layer.
A vanilla anchor that doesn't draw that frame still determines the layer's position. The vehicle
health, player health and contextual bar anchors follow that paint order.

### Replacing or wrapping a layer

In the same registration hook, call `event.layers.replace_layer(id, Box::new(draw))` to discard
an existing layer's body, including any wrappers. Use `event.layers.wrap_layer(id, Box::new(wrap))`
to keep the previous body and draw around it. Both methods accept a vanilla ID or an already
registered mod ID and return `false` if the ID isn't registered. They keep the layer's ID, its
place in the HUD and its draw conditions.

A wrapper gets a `GuiLayerWrapContext`: read the client with `host()`, paint with the scoped
`DrawContext` from `with_draw`, and call `draw_previous()` where the previous body should paint.
The latest wrapper runs first. Don't call `draw_previous()` if you want to skip the previous body
for this draw without discarding it at registration.

```rust,ignore
event.layers.wrap_layer("minecraft:hotbar", Box::new(|ctx| {
    ctx.draw_previous();
    ctx.with_draw(&mut |draw| {
        let [w, h] = draw.size();
        draw.fill([w / 2.0 - 91.0, h - 23.0, 182.0, 1.0], 0xFF80_80FF);
    });
}));
```

`gui_layer_pre` and `gui_layer_post` fire once per slot, not per wrapper. Post still fires if the
wrapper omitted the previous body. If Pre cancels the slot, neither the installed body nor Post
runs. For work that must happen even when the HUD is hidden, use the independent foreground
`gui_render_post` hook.

## Status rows and fixed text positions

`minecraft:player_health` is still a single GUI layer for hearts, armour, food and air. Cancelling it
in `gui_layer_pre` hides all four. `health_hud` replaces their combined drawing, not the separate
`minecraft:vehicle_health` layer.

Adding a taller status bar does not move the selected item name or the action bar. The name starts
59 GUI pixels from the bottom (45 outside survival and adventure); the action bar is centred 68
pixels from the bottom. A mod that needs to move either can cancel and redraw its layer:
`minecraft:selected_item_name` or `minecraft:overlay_message`.

## Registering F3 text

Push a `DebugEntry::text(id, group, lines)` in `Hooks::register_debug_entries` at client
startup. Use a non-vanilla namespaced ID, which also identifies the entry in the debug options
and saved profile. The group says which vanilla lines yours join; the closure receives the
current client host and appends to the entry's lines.

```rust,ignore
use mod_api::{DebugEntry, DebugGroup, RegisterDebugEntriesEvent, RegistrationContext};

fn register_debug_entries(&self, _: &RegistrationContext, event: &mut RegisterDebugEntriesEvent<'_>) {
    event.entries.push(DebugEntry::text(
        "example:queue".into(),
        DebugGroup::Memory,
        |_host, lines| lines.push("Queue: ready".into()),
    ));
}
```

Each mod starts with an empty batch. The client keeps the closure and calls it as your mod only
while the entry is visible. Reduced debug info hides it, even if it is set to always on.

## Registering world-space debug renderers

Push `DebugEntry::renderer(id)` in `Hooks::register_debug_entries` at client startup to
create the toggle. In `Hooks::register_debug_renderers`, push `DebugRenderer::new(id, draw)`
with the same ID. Use the callback's `ClientHost`, `WorldRenderView` and `WorldDrawContext` to
draw, and capture any state specific to this renderer instead of handling an ID in a draw hook.

```rust,ignore
use mod_api::{
    DebugEntry, DebugRenderer, RegisterDebugEntriesEvent, RegisterDebugRenderersEvent,
    RegistrationContext, WorldDepth, WorldLine,
};

const GUIDE_LINE: &str = "example:guide_line";

fn register_debug_entries(&self, _: &RegistrationContext, event: &mut RegisterDebugEntriesEvent<'_>) {
    event.entries.push(DebugEntry::renderer(GUIDE_LINE.into()));
}

fn register_debug_renderers(&self, _: &RegistrationContext, event: &mut RegisterDebugRenderersEvent<'_>) {
    let line = WorldLine::new([0.0, 65.0, 0.0], [2.0, 65.0, 0.0], 0xFF00_FF00, 2.0, WorldDepth::Tested);
    event.renderers.push(DebugRenderer::new(GUIDE_LINE.into(), move |_host, _view, draw| {
        let _ = draw.lines(std::slice::from_ref(&line));
    }));
}
```

The client collects a fresh list at startup, login, the first world view and the next world view
after a selection rebuild or respawn. A rebuild refreshes the list even if the same toggles end up
enabled, such as when the overlay opens and closes between world views. Keep per-refresh state in
the new callbacks. Push all renderers every time, even if their toggles are off; the first active
registration for a switch wins. Only installed callbacks run in a world view, as their registering
mods, after `submit_custom_geometry` and within its shared draw limits. Off or hidden toggles do not
draw.

## Block overlays

Block, water and fire overlays draw in camera space, after the level and hand and before the GUI. In
`extract_block_screen_effect`, call `event.set_custom_renderer(&MY_RENDERER)` to choose a static
`BlockScreenEffectRenderer` for just that overlay. The last eligible listener that sets a renderer wins.
After cancellation, only `#[listener(receive_cancelled)]` listeners can change the choice; the selected
renderer still submits even if the overlay remains canceled.

`submit` receives a read-only `ClientHost`, the original extraction inputs and a `CameraDrawContext`.
Return `false` to prepend your quads to the vanilla overlay, or `true` to skip just that overlay's vanilla
quads. Returning `false` does not restore a canceled overlay. Submission runs in block, water, fire order;
HUD camera overlays such as the vignette still draw above them.

```rust,ignore
use mod_api::{
    BlockScreenEffect, BlockScreenEffectRender, BlockScreenEffectRenderer, ClientHost,
    ExtractBlockScreenEffectEvent, Flow,
};

struct WaterTint;
impl BlockScreenEffectRenderer for WaterTint {
    fn submit(&self, _: &dyn ClientHost, render: BlockScreenEffectRender<'_>) -> bool {
        render.draw.quad(
            [[-1.0, -1.0, -0.5], [1.0, -1.0, -0.5], [1.0, 1.0, -0.5], [-1.0, 1.0, -0.5]],
            0x3000_4040,
        );
        false
    }
}

// Inside your impl Hooks:
fn extract_block_screen_effect(
    &self,
    _: &dyn ClientHost,
    event: &mut ExtractBlockScreenEffectEvent<'_>,
) -> Flow {
    if event.effect == BlockScreenEffect::Water {
        event.set_custom_renderer(&WaterTint);
    }
    Flow::Continue
}
```

## Gradient fills

`DrawContext::fill_gradient(rect, top, bottom)` is vanilla's `GuiGraphics.fillGradient`, the call screen
backgrounds and tooltips are drawn with. `rect` is `[x, y, w, h]` in GUI pixels, as `fill` takes it. `top` and
`bottom` are straight-alpha ARGB colours (`0xAARRGGBB`): `top` at `y`, blending to `bottom` at `y + h`.
With positive extents these are the visual top and bottom edges. Alpha blends with the colours, so a gradient can
fade out. The current HUD shader discards interpolated alpha below 0.004, not just exact zero; near-transparent
fragments can therefore differ from vanilla's exact-zero discard.

```rust,ignore
fn gui_render_post(&self, _: &dyn ClientHost, event: &mut GuiRenderPostEvent<'_>) {
    // A panel shaded from dark purple at the top to black at the bottom.
    event.batch.fill_gradient([20.0, 20.0, 120.0, 60.0], 0xF010_0010, 0xF000_0000);
    // A shade that fades out downwards.
    event.batch.fill_gradient([20.0, 80.0, 120.0, 10.0], 0x8000_0000, 0x0000_0000);
}
```

In a `gui_paint` buffer the same call is the op `GuiDraw::FillGradient { x0, y0, x1, y1, top, bottom }`, with
vanilla's corner arguments: `top` stays on the `y0` edge and `bottom` on the `y1` edge.

- **Pose and clip.** A gradient follows the current pose (a `gui_paint` buffer's `Push`) and the hook call's
  clip (`push_clip`), as `fill` does. A clip that cuts the rectangle keeps the colour ramp: the cut edge has the
  colour that was there.
- **Empty or inverted.** Zero width or height draws nothing, as does inversion on exactly one axis.
  Inverting both axes (`w < 0` and `h < 0`, or `x1 < x0` and `y1 < y0`) draws sorted bounds with the
  colours flipped: the first colour stays on its input y edge, now the lower edge. This preserves vanilla's
  visible winding and colour placement. A `fill_gradient` rectangle with a NaN or infinite coordinate draws nothing.
  Zero-area calls are culled here; vanilla can retain their layering bounds without drawing pixels.
- **Cost.** One quad on the GUI batch, as `fill`; nothing is drawn or kept when a mod doesn't call it.

## Scaled text

`DrawContext::text_scaled(text, at, scale, argb, shadow)` draws `text` as `text` does, scaled by `scale` about
`at`, its top-left corner: what vanilla's HUD draws with `PoseStack.scale` around a line, without a `gui_paint`
buffer of `Push`, `Text` and `Pop`. A half-size line under a label is `scale` 0.5, starting a quarter of its
unscaled width left of where it is centred.

```rust,ignore
use mod_api::{GuiLayerPlacement, RegisterGuiLayersEvent, RegistrationContext};

fn register_gui_layers(&self, _: &RegistrationContext, event: &mut RegisterGuiLayersEvent<'_>) {
    event.layers.register(
        "example:cps".into(),
        GuiLayerPlacement::Above("minecraft:hotbar".into()),
        Box::new(|_host, draw| {
            let width = draw.text_width("12 CPS");
            // Centred at (40, 30), half size.
            draw.text_scaled("12 CPS", [40.0 - width / 4.0, 30.0], 0.5, 0xFFFF_FFFF, false);
        }),
    );
}
```
