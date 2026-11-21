# Baked models and their keys

Every successful resource load bakes each block state's models and the standalone models once,
into one store for that load. That store belongs to the game and never changes while it is
published: the mesher, block entity drawing and a mod all read the same baked results. A mod
holds keys into it and read-only copies of the geometry and metadata it lists, never the models
themselves. The [design](https://github.com/sunrise-org/sunrise/blob/main/docs/design/mod-api/baked-models.md)
covers it; block-entity model data and editing a baking result are separate changes.

## Keys

A `BakedModelKey` names one baked model in one published store: its resource generation and its
slot. Ask for them in `model_baking_completed`, when the new store is published, and keep them
until the next one:

```rust,ignore
fn model_baking_completed(&self, host: &mut dyn ClientHost, _: &mut ModelBakingCompletedEvent) {
    let assets = host.render_assets();
    // Use the same BlockState you get from a level read or a placement event.
    let Some(state) = host.block_state_id("minecraft:stone") else { return };
    let Ok(keys) = assets.block_state_models(state) else { return };
    // A standalone model's handle names a baked model too.
    let part = Identifier::parse("mymod:block/part")
        .ok()
        .and_then(|id| assets.model(&id).ok())
        .map(BakedModelKey::from);
    // A state no blockstate file binds has no keys.
    let Some(first) = keys.first().copied().or(part) else { return };
    let Ok(view) = assets.baked_model(first) else { return };
    for quad in &view.quads {
        // quad.positions, quad.uvs, quad.sprite, quad.facing, quad.cull, quad.tint_index ...
    }
}
```

| Call | Answers |
| --- | --- |
| `block_state_models(state: BlockState)` | Every model the state can select, in bake order without repeats, whichever one a position's seed picks. Empty for a valid state with no baked binding. `UnknownModel` if the live state index lacks the state. |
| `block_state_model(state: BlockState)` | The state as one drawable `ModelRef` (see [Model drawing](model-drawing.md)). `UnknownModel` for air, a state no blockstate file binds, or a state the live state index lacks. |
| `baked_model(key)` | A read-only copy of the listed fields: `ambient_occlusion`, the `particle` sprite and its `quads`. `StaleModel` once a reload superseded the key; `UnknownModel` for a slot the store lacks. |

`BakedModelView` and `BakedQuad` are copies: changing them changes nothing the game draws. They
carry the listed geometry and metadata only, not everything the mesher reads: no material render
type, per-quad colour, per-quad ambient occlusion or shading override. Each
`baked_model` call copies every quad, so read what a mod needs once per load, not each frame.

## Generations

A successful reload (F3+T, a pack turned on or off, a server's pack) bakes a new store from the
resources as they are now, never from the previous store, and publishes it before
`model_baking_completed`. Keys from the previous load then answer `StaleModel`; ask again. A
failed reload publishes nothing, and the keys of the last successful load stay valid. A dedicated
server bakes no models: its store is empty.

All three calls are available through `ClientHost::render_assets()` in Setup, Reloading and World.
A state the live index lacks gives `UnknownModel` on both.
