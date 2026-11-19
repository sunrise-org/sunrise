# Drawing resolved models

Register standalone models during `standalone_models`, then resolve their `ModelRef`
through `RenderAssets::model` after resources are published. World render hooks receive
`WorldDrawContext`: submit transient copies with `draw_model_instances`, or retain
copies in a group. GUI scenes, mobs and kept/spinning renderers use their
CPU routes; groups are a world-drawing facility, not another renderer.

## Worn humanoid equipment

An item attaches one `ClientItemExtensions` Part in
`Hooks::register_client_item_extensions`, on the client at every resource load.
The event's `extensions: &mut dyn ClientItemExtensionsRegistrar` registers the
models, textures and item providers together. There is no startup provider
registration path: request your `ItemKey` during ordinary registration, then
attach its provider during this resource-load event.

```rust,ignore
let inner = event.extensions.register_model("mymod:inner_chest", &inner_tree)?;
let outer = event.extensions.register_model("mymod:outer_chest", &outer_tree)?;
let inner_texture = event.extensions.register_texture("mymod:entity/equipment/inner")?;
let outer_texture = event.extensions.register_texture("mymod:entity/equipment/outer")?;
event.extensions.client_item_extensions(item, Arc::new(ChestLayers {
    models: [inner, outer],
    textures: [inner_texture, outer_texture],
}))?;
```

Trees are API-owned `ModelPart` trees, built directly or baked from a
`LayerDefinition`. Register them once per resource load, never during a frame.
Model names belong to the registering mod's namespace. Duplicate model names,
non-finite geometry or poses, an unknown texture, an unknown item and two
providers for one item refuse the resource-load registration by name. A duplicate
item diagnostic names both providers. Disabling the owning mod stops its Part.
A server-only package cannot declare this client registration hook.

The worn stack must first pass the equippable component's asset and matching-slot
checks, and the asset must have layers. Only the visible equipped item's provider
is queried; random-texture inventory enumeration does not run frame callbacks.
Every callback takes `&dyn ClientHost` and `&ItemStack`:

1. `default_dye_color` runs once and returns packed ARGB, or zero when undyed.
2. `armor_layer_tint_color` runs in asset layer order, with the layer and its index.
   Exactly zero skips the layer, without calling its texture or model method.
   The default is white for a nondyeable layer, or its dye/optional undyed colour
   for a dyeable layer. Without either colour, a dyeable layer is skipped.
3. `armor_texture` receives the layer type, layer and the resolved default texture
   reference, including pack/CIT selection. A returned override is final for that
   layer; it is not replaced by another random-texture or custom-model texture.
4. `armor_layer_model` chooses the registered tree for that layer. Returning
   `ArmorModelRef::VANILLA` keeps the original model. The humanoid pose is copied
   onto the chosen tree, with only the current slot's parts visible.

Per-layer model selection deliberately differs from NeoForge's one model per
piece. It lets a chest item draw an inner shape with one texture and an outer
shape with another, without painting both textures on both shapes. There are no
additional per-piece `humanoid_armor_model` or `generic_armor_model` callbacks.
The trim draws once on the last material layer actually drawn's tree; if no
material drew, it uses vanilla's model. The trim does not call the model method
again. Babies do not draw trims. Foil goes on the first drawn material when there
is no trim, and on the trim otherwise.

Model and texture references are scoped to the resource load. A stale reference
returned during drawing uses vanilla's model or the supplied default texture,
with one diagnostic per provider until reload, never a panic. A failed callback
discards all of the piece's custom layers, draws the piece with vanilla behaviour,
and disables that provider until reload, with one diagnostic. This includes refused
or trapped wasm callbacks, even after an earlier layer succeeded. Native and
wasm use the same Part, methods and handler tokens. Only references and layer
values cross Wire during a frame: no geometry, rebakes or native-only escape path.

## Bake-time standalone forms

`standalone_models` contract version 2 keeps `event.models: &mut Vec<String>` for plain
file ids and adds `event.forms: &mut Vec<StandaloneModelForm>`. Both native and wasm
listeners receive `RegistrationContext`, not a runtime `Host`. Forms are attributed
to the registering mod by dispatch:

```rust,ignore
event.forms.push(StandaloneModelForm::Rotated {
    id: "mymod:shaft_east".into(),
    model: "mymod:block/shaft".into(),
    turns: [0, 1, 0],
    uvlock: true,
});
event.forms.push(StandaloneModelForm::Combined {
    id: "mymod:machine".into(),
    parts: vec!["mymod:block/base".into(), "mymod:block/overlay".into()],
});
```

`turns` are negative quarter-turn counts, modulo four, applied X then Y then Z
about the block centre, exactly as blockstate baking. UV locking uses the inverse
face transformation during baking; rotating a draw pose is not equivalent. For a
Y quarter-turn a north wall becomes east but stays upright either way; a north-pointing
arrow on the top turns east without UV locking and retains its world orientation with it.

Combined parts bake unturned in declaration order, retaining each part's positions,
materials and quad metadata in one `ModelRef`. Their particle is the first available
part particle and model-level ambient occlusion is enabled only if every part enables it.
Dependencies, including parents and loader references, load through the normal model
library; they are not implicitly registered as standalone results. If any dependency
cannot load or bake, no part of that form is published and the warning names the
registering mod, even when the result or source has a different namespace.

Result ids follow the same rule as plain ids: a nonempty namespace and path are
required; they need not equal the registering mod id. Plain ids win duplicate result
ids, then the first successfully baked form wins. Registration runs once, but all
forms rebake from current resources on every successful load. Resolve result ids in
`model_baking_completed`; reload invalidates old handles and failed reloads retain them.
This API bakes quads only, not a mod-defined baked result type.


## Terrain materials in a block entity

`draw.model(model, tint)` draws a resolved model as an entity model. A block entity, mob or
player submit hook that wants the terrain's own materials calls
`draw.terrain_model(model, options)` instead. That covers a machine's belts, shafts and
casings, cut out and shaded as blocks are:

```rust,ignore
let belt = TerrainUvShift::new("mymod:block/belt", "mymod:block/belt_scroll", [1.0, 0.5], [0.0, scroll]);
let options = TerrainModelOptions::default()
    .with_material(TerrainMaterial::Cutout)
    .with_uv_shift(belt);
draw.terrain_model(self.belt, options)?;
```

- Faces sample the animated block sprites, cull back faces and write colour and depth like
  opaque terrain. Light is the draw's own, raised by each quad's emission.
- `material`: `Model` keeps each quad's `render_type` (solid, `cutout`, `cutout_mipped`), and
  a translucent quad refuses the draw. `Solid`, `Cutout` (alpha under 0.1 discarded, no
  mipmaps) and `CutoutMipped` (under 0.5, mipmapped) choose one material for all quads.
- `shade`: `Model` follows each quad's `shade` flag, `Off` turns directional shading off, and
  `Entity` shades every quad by the level's entity light directions. Full-bright light never
  turns shading off.
- `uv_shift` draws faces of exactly one sprite with another, their UVs scaled then moved.
  `uv_shrink_ratio` (`0.0..=1.0`, default `0.0`) pulls UVs towards each face's centre first,
  for a model baked with an older `FaceBakery`'s atlas shrink. The engine never guesses it.
- `tint` (`0xRRGGBB`) multiplies only quads with a tint index.
- A stale handle returns `TerrainModelError::Model(ModelError)`. If any quad can't be drawn, the result is
  `TerrainModelError::Refused` and nothing is drawn. That covers a translucent quad under
  `Model`, a missing sprite, a pose that flattens the model, a ratio out of range, and a shift
  whose numbers aren't finite, whose sprites are unknown or whose source sprite no face uses.

## Extra models in a block's mesh

A block whose look depends on data a block state can't hold (a cable showing each side's
channel count) gives the chunk mesher extra models to draw as part of it. On the client,
`host.set_block_extra_models(pos, &models)` sets the resolved standalone models drawn at `pos`:

```rust,ignore
let parts = [self.core, self.side_counts[count]];
host.set_block_extra_models(pos, &parts)?;
```

- The models are drawn after the block's own, unturned from its lower north-west corner (with its
  offset), in its light and tints. Each quad keeps its render layer (solid, cut out,
  translucent). A face with a cull direction is hidden by the neighbour there, as the block's
  own faces are. On air they draw nothing.
- A list replaces the position's last one; an empty list clears it. The section is meshed again
  only when the list changes, so set it when the data changes, not each frame.
- At most `BLOCK_EXTRA_MODELS_MAX` (64) models a position. `BlockExtraModelsError` names each
  refusal: `NotLoaded` (no client column there, or outside the level's height), `TooMany`,
  `Model` for a stale or unknown handle, and `Unavailable` outside the World phase (see below).
  A refused call changes nothing.
- A section's extra models share its quad and byte limits with its blocks. The blocks' own
  geometry is always drawn; past the limits, the section's remaining extra models (in block
  order, then list order) are not.
- The list is the client's own, never saved or sent. It is cleared when the block state at
  `pos` changes, when its column unloads (a column the server sends again comes without it) and
  when the player's level changes. Set it again from the block entity's data or the mod's own
  client map, on `client_block_changed`, `client_chunk_load` or a payload. It needs no block
  entity.
- The mesher reads the lists on its workers and never calls mod code. A handle a resource reload
  made stale draws nothing until you set the lists again from the new handles.

The setter is World-only. `client_resources_reloaded` runs in Reloading, where it answers
`Unavailable`. Resolve the new handles there, then set the lists again from the next
World-phase callback, such as the next client tick:

```rust,ignore
fn client_resources_reloaded(&self, host: &mut dyn ClientHost, _: &mut ClientResourcesReloadedEvent) {
    let mut parts = self.parts.lock().unwrap();
    parts.core = host.render_assets().model(&self.core_id).ok();
    parts.reapply = true;
}

fn client_tick_pre(&self, host: &mut dyn ClientHost, _: &mut ClientTickPreEvent) {
    let mut parts = self.parts.lock().unwrap();
    if std::mem::take(&mut parts.reapply) {
        for (&pos, cable) in &self.cables.lock().unwrap() {
            let _ = host.set_block_extra_models(pos, &parts.models_for(cable));
        }
    }
}
```

## Persistent groups

- `host.create_group(model, capacity)` reserves up to 16,384 copies of a solid model.
  Initial drawn length is zero. Translucent models are refused.
- `group.write(host, first, copies)` atomically overwrites a range and extends drawn
  length if needed. Unwritten slots start as white, full-bright identity copies at
  world origin. Invalid coordinates or light, or a range beyond capacity, change nothing.
- `group.set_len(host, len)` changes drawn length without changing reserved capacity.
  Shrinking to zero draws nothing; growing restores retained copies, including hidden
  slots written earlier.
- `group.set_group_transform(host, transform)` sets an identity-by-default row-major
  3×4 affine pose. For each local vertex the composition is exactly
  `world = origin + group_transform * (copy_transform * local)`.
  The world-space f64 origin is not rotated or scaled. Non-finite pose values change nothing.
- `draw.draw_instance_group(group)` draws the live length once per view. Repeated
  submissions in that view are ignored. Culling uses the composed pose.
- `group.destroy(host)` retires the handle and releases capacity and CPU caches.
  Copies already accepted for a view remain drawn; subsequent calls return `Stale`.

Each mod may reserve 65,536 copies across all its groups; a zero-capacity group
reserves one slot. Destroying a group releases that reservation immediately. A view
accepts 16,384 visible copies shared across transient batches, groups and listeners,
and at most 16,384 group submissions. Culled copies do not consume the visible-copy
budget. Refusal is atomic: no prefix of a refused submission draws.

Groups cache converted records on the CPU. Unchanged groups avoid pose, normal and
tint conversion, but cull and stage visible copies each frame. Every visible
GPU-routed copy uploads each frame; groups do not retain a private GPU buffer.
Frequent whole-group pose changes rebuild every live record and can cost more than
uncached transient copies. Shader-pack selection, missing prepared GPU records and
overlays use the same CPU baking path as transient batches, including tint and light.
There is no simulation-tick work.

Only the creating mod can access a group. A foreign handle returns `GroupError::Stale`,
as does a destroyed group, a group after resource reload or world close, or a group
whose mod was disabled. Re-enabling starts without groups: recreate them from the
newly published model handles. Slot reuse never revives an old handle. Native and
wasm use the same ownership, budgets, lifetime and rendering implementation.

## Still fluids in a block entity

A block entity, mob or player submit hook draws a tank's contents with
`draw.fluid_box(&stack, bounds, options)`, where `bounds` is
`[x0, y0, z0, x1, y1, z1]` in blocks under the draw's current pose:

```rust,ignore
let options = FluidBoxOptions::new(false, 4.0 / 1024.0);
draw.fluid_box(&tank.stack, [0.125, 0.0625, 0.125, 0.875, level, 0.875], options)?;
```

- The stack is `minecraft:water` or `minecraft:lava` with no components. Any other fluid,
  an unresolved one or a component patch is `FluidBoxError::UnsupportedFluid`.
- Each face tiles the fluid's animated still sprite once per block, cut at whole-block lines.
  Each tile keeps its place in the sprite. The box is not inset. `uv_shrink_ratio` is the
  sprite's `TextureAtlasSprite.uvShrinkRatio` (`0.0..=1.0`), and `render_bottom` adds the bottom face.
- The colour is the stack's tint (plain water `0xFF3F76E4`, lava white), never the biome's
  water colour. `with_argb` replaces it, alpha included. Faces have no directional shading.
  Sky light is the draw's own, and block light rises to the fluid's (lava 15).
- The faces use a material of their own: sorted far to near, alpha blended, culled from behind,
  and depth-writing. They draw with the block entities, before the level's translucent terrain,
  under shader packs too.
- A stack of 0 draws nothing and returns `Ok` without checking `bounds`. Non-finite or reversed
  bounds, bounds beyond 1,048,576 blocks, a box of more than 65,536 tiled quads and a ratio out of range are
  `FluidBoxError::InvalidBox`. Missing fluid sprites return
  `FluidBoxError::MissingSprite`. A refusal draws nothing.

## Before and after a living entity draws

Each mob and player in view gets the render hook pair once a frame, NeoForge's
`RenderLivingEvent` and `RenderPlayerEvent`:

1. `render_player_pre(player, who, draw, hidden, canceled)`, for a player only (`who` is its
   profile name and whether it is this window's own);
2. `render_living_pre(entity, draw, hidden, canceled)`, before the entity's model;
3. the model and its vanilla layers, or in their place the renderer a mod registered for its type
   ([entity renderers](entity-renderers.md)), then the layers mods added to its type
   ([entity layers](entity-layers.md));
4. `render_living_post(entity, parts, draw)`;
5. `render_player_post(player, who, parts, draw)`, for a player only.

The order today: the model and the layers are drawn before the post hooks; the entity's name tag, leash, flames
and shadow are drawn in the frame's later passes, after them. The reference draws the leash and the name tag
before the post (and queues the flames and the shadow after it): tracked in #3915. A type with a registered
renderer has the same order, the renderer in the model's place, so it changes with #3915 too.

`draw` starts at the entity's feet, turned with its body (+z the way it faces, +x its left, +y up,
in blocks), and keeps what each hook draws. Setting `canceled` in a pre hook skips everything else
of that entity this frame: its model (or its type's registered renderer), its layers, its name tag,
shadow, flames, leash and glowing outline, and the post hooks. A mod that draws one entity's model of
its own draws it in `render_living_pre` and cancels; one that draws every entity of a type registers
a renderer for the type instead; one that wants the name tag or the shadow then draws them itself, or doesn't cancel.
Canceling only `render_living_pre` of a player still asks its `render_player_post`. An entity the
frame culls gets neither hook. The game asks once a frame whether any mod listens to the four, and
with none builds nothing for them.

## A player's or humanoid's parts

`render_living_post` and `render_player_post` get a humanoid's parts as vanilla posed them this frame (`parts`,
`HumanoidModel.setupAnim`), read-only, to attach geometry to an arm or the head without
animating it again. Only `HumanoidMobRenderer`'s families have them (players, zombies and their
kin, skeletons, piglins); every other mob gets `None`, a cow included, though its model has a
head and a body. `PosedParts` is a fixed-size snapshot: `get(part)` or `iter()`. It is taken
from the vanilla body's own draw, so a part `render_player_pre` hid (below) has `visible` false.

`render_player_pre` can hide single parts of the vanilla player model instead of replacing all of
it: `hidden.hide(HumanoidPart::Head)`, as a mod drawing its own outer skin layer does.
`render_living_pre` hides parts of any mob's vanilla model the same way, by their names in the model:
`hidden.hide_named("nose")` leaves a villager's nose out, as a level of detail for a far crowd
does. A frame holds at most `MAX_HIDDEN_PARTS` (8) names of at most `MAX_PART_NAME` (24) bytes;
`hide_named` says `HideError::Full` or `HideError::BadName` past that. An unknown name refuses the draw at first use, reporting the mob type and part name.
Typed humanoid parts have separate storage, so all twelve fit alongside the eight names.

- Hiding happens after Skin Customization, for this frame only.
- A hidden part takes its children with it: the hat goes with the head, a sleeve with its arm.
- The body loses the part, and so do the layers drawn on the body's own model, as vanilla's
  `getParentModel()` layers do: a villager's type, profession and level, a spider's eyes. The
  armour, held items and other layers keep every part, and the cape, a layer of its own, stays.

## Text and animation in block entities

`BlockEntityDrawContext::text` draws a component under the current pose in font pixels,
with its styles and the frame's light. Its RGB colour is opaque. Glowing text uses full
light; `Some(0)` is a black outline, not an absent outline. `text` and `text_width`
return the styled advance rounded up, as vanilla `Font.width` does. Use half that width
to centre a line. Kept text retains its glyphs, including obfuscated glyph choices,
until it is drawn again. Resource reload invalidates kept geometry.

Spinning parts use the client's animation clock: level-running texture ticks plus the
level's partial tick, not the camera's partial tick or the server's day time. Pausing
holds that clock; `/tick freeze` holds the level partial at 1 and stops texture ticks.
Camera motion during a freeze does not spin a part backwards. Server time updates do
not reset animation. Held-arm, hand, mob and scene draws use this same level partial.

## Copycat faces

Ask `block_sprites` for the material's side faces, then use each face's `texture` and
`quad_on(side, from, to)` to draw it on your box. The result supplies outward-wound
corners and a UV rectangle in texels, preserving the material's rotations and mirrors
and cropping to your box. It returns `None` for a slanted or sheared texture mapping
that cannot be represented by a rectangle; use another drawing route rather than
substituting an upright texture.

Multiply tinted faces by `block_tint_color(state, face.tint_index, pos)` at their world
position. Untinted faces return `None`. Water faces use tint index 0 and the world's
water colour, including resource-pack water colormaps; other water indices return
`None`. This differs from `fluid_box`, whose colour comes from its fluid stack.
