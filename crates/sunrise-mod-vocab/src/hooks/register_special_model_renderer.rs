//! The `register_special_model_renderer` hook: declarations the mod API generator reads (see
//! `hooks/mod.rs`).

pub trait Hooks {
    /// The special item model types a mod draws (NeoForge's `RegisterSpecialModelRendererEvent`):
    /// `register(type, renderer)`, and every `minecraft:special` item model whose `model` object
    /// has the type `type` (`mymod:skillet`) is drawn by `SpecialModelRenderer::extract`. A later
    /// registration of a type replaces the earlier one, vanilla's types included, and the log
    /// names both mods. Asked once at start on the client, before resources load; a special
    /// model of a type no mod registered and vanilla doesn't have is an error naming it, and its
    /// item draws the missing model.
    #[notes = r#"## Special item model renderers

A special item model (vanilla's `minecraft:special`, NeoForge's `RegisterSpecialModelRendererEvent`) draws an
item with code instead of one flat model: a skillet with what it cooks, a tool with moving parts.
`Hooks::register_special_model_renderer` registers one renderer per special model type once at start, before
resources load. The geometry stays in the packs: from the stack, the renderer answers which resource models
draw, each posed and tinted, and the game draws them as it draws any item model.

```rust,ignore
struct Skillet;
impl SpecialModelRenderer for Skillet {
    fn extract(
        &self,
        _host: &dyn ClientHost,
        stack: &ItemStack,
        _config: &Tag,
        _context: &SpecialModelContext,
    ) -> Result<Vec<SpecialModelPart>, String> {
        let mut parts = vec![SpecialModelPart::new("example:item/skillet_pan")];
        if stack.count() > 1 {
            // The food sits a pixel above the pan.
            parts.push(SpecialModelPart::new("example:item/skillet_food").translated([0.0, 0.0625, 0.0]));
        }
        Ok(parts)
    }
}

fn register_special_model_renderer(
    &self,
    _context: &RegistrationContext,
    event: &mut RegisterSpecialModelRendererEvent<'_>,
) {
    event.renderers.register("example:skillet".into(), SpecialModelPolicy::Stack, Arc::new(Skillet));
}
```

```json
{"model": {"type": "minecraft:special", "base": "example:item/skillet",
  "model": {"type": "example:skillet"}}}
```

`config` is the `model` object, converted to NBT once at load, so a type reads its own fields from it. Each
part names a model file of the packs, a row-major 3×4 transform in the special model's space (blocks, with the
origin at the model's corner, as an element's coordinates over 16) and its tints per tint index (ARGB, as an
item model's `tints`). The renderer never sees the `base` model: the game draws the parts with its display
transforms, particle and GUI light, as vanilla does, and the node's `transformation` applies to all of them.

Registration names a `SpecialModelPolicy`. `Stack` asks the renderer on the client's main thread when an item
of such a model is drawn with a stack other than the last one that model was asked about: an unchanged stack is
asked once, however many frames draw it, and every display context (a hand, a screen, the ground, a frame, a
head) draws that answer; the `context` it is given is always `SpecialModelContext::STACK`. This caching is a
deliberate difference from vanilla, which extracts every frame. `Frame` asks once per frame of the view for
each display context and game time the item is drawn with, and `context` carries the display context, the
level's game time (`None` outside a world) and the partial tick: for an ingredient that rises and turns over
a few ticks and must move smoothly between them. There is no cap on animated items: a `Frame` renderer costs one `extract` per drawn item per frame, for each
display context the item is drawn in (the same item in a hand and in an item frame is asked twice). A `Frame`
answer is not kept past its frame, and an item
drawn with one is never cached as a still GUI image. Nothing is asked, and no scan runs, for a model whose
type no mod registered.

A later registration of a type replaces the earlier one, whichever mod made it and vanilla's types included,
as in NeoForge; the log names both mods. An id that is not `<namespace>:<path>` is logged and left out. A
special model of a type no mod registered and vanilla doesn't have is an error naming the type and the item:
that item draws the missing model, and the rest of the resources load. A part can also be an item stack (`SpecialModelPart::stack`), resolved through the item model resolver with the
inner item's own models, tints and foil under the display context you name; a cycle or more than
`MAX_NESTED_STACK_PARTS` (64) stack parts in one draw refuses the answer. A renderer that returns an `Err`,
panics, returns more than `MAX_SPECIAL_MODEL_PARTS` (256) parts or returns a non-finite transform
is logged once with its type and asked no more until resources reload. Its special models draw the
missing model until then. A part naming a model no pack has makes that
answer draw the missing model, logged once for the model."#]
    #[hook(
        side = "client",
        contract = "listen",
        context = "registration",
        neoforge = "RegisterSpecialModelRendererEvent"
    )]
    fn register_special_model_renderer();
}
