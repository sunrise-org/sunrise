# Drawing resolved models

Register standalone models during `standalone_models`, then resolve their `ModelRef`
through `RenderAssets::model` after resources are published. World render hooks receive
`WorldDrawContext`: submit transient copies with `draw_model_instances`, or retain
copies in a group. GUI scenes, mobs and kept/spinning renderers use their
CPU routes; groups are a world-drawing facility, not another renderer.

## Local player skins

Implement `Hooks::resolve_player_skin` to change the local player's appearance
without changing their profile. Upload the skin with
[`create_dynamic_texture`](dynamic-textures.md) as a 64×64 RGBA image once per
skin file. Normalize legacy 64×32 skins before uploading. Keep the texture id
and its model width, then select them in the hook:

```rust,ignore
fn resolve_player_skin(&self, _: &dyn ClientHost, event: &mut ResolvePlayerSkinEvent<'_>) {
    if let Some(skin) = self.selected_skin() {
        *event.selection = Some(PlayerSkinOverride::new(skin.texture.clone(), skin.model));
    }
}
```

Use `PlayerModel::Wide` for four-pixel arms and `PlayerModel::Slim` for
three-pixel arms. The selection starts at `None` each frame; listeners can leave
an earlier answer alone or clear it with `*event.selection = None`. A missing,
freed or wrongly sized texture keeps the original texture **and** model.
The selection affects the third-person avatar and its skin layers, first-person
arms, and inventory and creative previews. It does not change the cape, tab
list, or anyone else's view.

## Third-person held items

Attach an `ItemPart` with `Registrar::item_part` (or `item_part_tag`) to tint an
item in players', mobs' or armor stands' hands in the world, including vanilla
items. `held_item_tint` runs on the client just before the resolved item is
submitted. Use the holder's current level entity ID and logical `Hand::Main`
or `Hand::Off`, not the left or right arm:

```rust,ignore
#[mod_api::sunrise_part]
impl ItemPart for ChargedTint {
    fn held_item_tint(
        &self,
        _host: &dyn ClientHost,
        event: &mut HeldItemTintEvent<'_>,
    ) -> Option<u32> {
        self.is_charged(event.entity_id, event.hand).then_some(0xFFA040)
    }
}
```

The event also borrows the rendered stack and carries the frame's `partial_tick`.
The first `Some(0xRRGGBB)` wins; return `None` to let the next Part answer or keep
the model's colors. The multiplier applies to that draw's RGB before lighting,
including a shield's base and banner patterns. It leaves alpha, glint, outline
and pose alone, and does not affect first person, GUI previews, worn armor, item
frames or dropped items. Keep the handler fast: it can run once for each held
stack submitted during a frame. Rendering is not a source of combat state; see
[Syncing remote item cooldowns](networking.md#syncing-remote-item-cooldowns).

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
and disables that provider until reload, with one diagnostic. A failure discards the custom drawing even if an earlier layer succeeded.
Callbacks work with references and layer values, not geometry or rebakes.

## Bake-time standalone forms

`standalone_models` contract version 2 keeps `event.models: &mut Vec<String>` for plain
file ids and adds `event.forms: &mut Vec<StandaloneModelForm>`. Listeners receive `RegistrationContext`, not a runtime `Host`. Forms are attributed
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


## Colour, hidden faces and translucency of copies

`ModelInstance` carries two more fields, set with `with_color` and `with_hidden_faces`:

- `color` (`0xAARRGGBB`, default white) multiplies every face of the copy, tinted or not, after
  `tint`; its alpha multiplies every face's alpha. `tint` keeps its meaning (tinted faces only).
- `hidden_faces` (bits 0 to 5: down, up, north, south, west, east) leaves out the faces whose
  `cull` side is set, so a structure of many copies can drop the sides its neighbours cover.

A copy whose colour alpha is below `0xFF` draws every quad in the translucent material described below.
A model with translucent faces otherwise keeps its per-quad routing: only those faces take the translucent
material. Copies with a colour other than white, hidden faces, a model with translucent faces or a block state model are baked on the CPU
instead of drawn as GPU instances; the 16,384-copy budget bounds that cost.
`overlay` (the hurt tint) has no effect on a copy on the translucent or block state model route, including any
opaque faces it holds. `color` (the whole baked block model, multiplied at draw time) and `hidden_faces` (a
cull mask the caller supplies) are Sunrise's options for mod copies.

`block_state_model(state)` (on `RenderAssets`) resolves a block state as one model: the parts
its blockstate file selects for its properties, weighted alternatives at the fixed seed
`BlockEntityDrawContext::block` uses, each tinted face in the block's default tint. It draws
through `draw_model_instances`, instance groups and `draw.terrain_model`, not `draw.model`.
Resolve it once per state in `client_setup` and `client_resources_reloaded`.

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

- Faces sample the animated block sprites (mipmapped, linear minification and nearest
  magnification, as vanilla samples a moving block), cull back faces and write colour and depth like
  opaque terrain. Light is the draw's own, raised by each quad's emission.
- `material`: `Model` keeps each quad's `render_type` (solid, `cutout`, `cutout_mipped`), and
  draws a translucent quad in the translucent material (below). `Solid`, `Cutout` (alpha under
  0.1 discarded) and `CutoutMipped` (under 0.5) choose one material for all quads.
- `shade`: `Model` follows each quad's `shade` flag, `Off` turns directional shading off, and
  `Entity` shades every quad by the level's entity light directions. Full-bright light never
  turns shading off.
- `uv_shift` draws faces of exactly one sprite with another, their UVs scaled then moved.
  `uv_shrink_ratio` (`0.0..=1.0`, default `0.0`) pulls UVs towards each face's centre first,
  for a model baked with an older `FaceBakery`'s atlas shrink. The engine never guesses it.
- `tint` (`0xRRGGBB`) multiplies only quads with a tint index.
- `color` (`0xAARRGGBB`, default white) multiplies every quad's colour after its own and the
  tint, alpha included; white skips the multiply. An alpha below `0xFF` puts every quad in the
  translucent material.
- **Translucent material.** The one vanilla draws a translucent block with outside the terrain
  (`translucent_moving_block`): alpha blended, back faces culled, depth tested **and written**,
  fragments under alpha 0.1 discarded, the quad's directional shade baked into its colour. The
  renderer sorts the quads of all translucent models far to near each frame, as vanilla's
  `sortOnUpload` does, so submission order does not matter.
- A stale handle returns `TerrainModelError::Model(ModelError)`. If any quad can't be drawn, the result is
  `TerrainModelError::Refused` and nothing is drawn. That covers an unknown render type, a
  missing sprite, a pose that flattens the model, a ratio out of range, and a shift
  whose numbers aren't finite, whose sprites are unknown or whose source sprite no face uses.

## Persistent groups

- `host.create_group(model, capacity)` reserves up to 16,384 copies of a model.
  Initial drawn length is zero.
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
overlays use the same CPU baking path as transient batches, including tint and light. Translucent
copies and copies of a block state model take the terrain model materials, not the
block-entity program a `shaders/` pack replaces.
There is no simulation-tick work.

Only the creating mod can access a group. A foreign handle returns `GroupError::Stale`,
as does a destroyed group, a group after resource reload or world close, or a group
whose mod was disabled. Re-enabling starts without groups: recreate them from the
newly published model handles. Slot reuse never revives an old handle.

## Still fluids in a block entity

A block entity, mob or player submit hook draws a tank's contents with
`draw.fluid_box(&stack, bounds, options)`, where `bounds` is
`[x0, y0, z0, x1, y1, z1]` in blocks under the draw's current pose:

```rust,ignore
let options = FluidBoxOptions::new(false, 4.0 / 1024.0);
draw.fluid_box(&tank.stack, [0.125, 0.0625, 0.125, 0.875, level, 0.875], options)?;
```

- The stack's fluid is `minecraft:water`, `minecraft:lava` or any fluid with a look
  (`data/<mod>/sunrise/fluid/<name>.json`: `sprite`, `tint`; [fluid looks](../content-mods/fluid-container-models.md)
  holds the format). A stack with components is drawn like one without: they change neither the
  sprite nor the tint, so a mod colours a potion-like stack with `with_argb` from its own
  components. A registered fluid with no look is `FluidBoxError::MissingSprite`; an unresolved
  fluid and the empty fluid are `FluidBoxError::UnsupportedFluid`.
- Each face tiles the fluid's animated still sprite (a look's `sprite`; a look with no sprite
  draws its tint as a flat colour) once per block, cut at whole-block lines.
  Each tile keeps its place in the sprite. The box is not inset. `uv_shrink_ratio` is the
  sprite's `TextureAtlasSprite.uvShrinkRatio` (`0.0..=1.0`), and `render_bottom` adds the bottom face.
- The colour is the fluid's tint (water `0xFF3F76E4`, lava white, a mod fluid's look `tint`, grey
  `0xFF9A9A9A` when it has none, as `DrawContext::fluid` draws it), never the biome's water colour. `with_argb` replaces it, alpha
  included. Faces have no directional shading. Sky light is the draw's own, and block light
  rises to the fluid's `light_level` (lava 15, water 0, a mod fluid's block's).
- Each fluid's look is resolved once per resource load; a box does one indexed read.
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

1. `render_player_pre(player, who, draw, hidden, hidden_layers, canceled)`, for a player only (`who` is its
   profile name and whether it is this window's own);
2. `render_living_pre(entity, draw, hidden, hidden_layers, canceled)`, before the entity's model;
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
  armour, held items and other layers keep every part, and the cape, a layer of its own, stays. Use
  the separate [`hidden_layers`](entity-layers.md#hiding-a-stock-layer) editor to suppress those
  submissions without hiding body parts.

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

Directional shading on `quad` and `cube` follows `spin`, but light stays fixed by default.
To use the level's sky and block light, call `BlockEntityDrawContext::level_light` in a world
block-entity or entity renderer. Each element uses one level-light sample. The centre is the
unit-cell centre for `block`, `model` and `terrain_model`, the box midpoint for `cube`, or the
mean of the corners for `quad`, transformed by the pose and any applied `spin`. Baked emission
and other per-channel light floors still apply. Items and text keep the fixed light.

After `keep`, spun elements sample the cell they turn into each frame without a new submit.
Stationary elements keep their original sample until the draw is invalidated. If the centre
falls outside representable cell coordinates, the element uses its fixed light instead.
Level-less scenes, previews and mob model frames also stay fixed. `light()` always reports the
fixed setting; `set_light` returns subsequent draws to fixed light.

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
