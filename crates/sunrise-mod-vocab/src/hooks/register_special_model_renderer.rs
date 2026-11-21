//! The `register_special_model_renderer` hook: declarations the mod API generator reads (see
//! `hooks/mod.rs`).

pub trait Hooks {
    /// Register factories for special item model types (NeoForge's `RegisterSpecialModelRendererEvent`).
    /// Called once at client startup, before resources load. The last registration of a type wins,
    /// including for vanilla's types. Each `minecraft:special` node is decoded and baked afresh on
    /// resource reload; the baked renderer submits to a scoped collector when the item is drawn.
    #[notes = r#"Use `event.renderers.register(kind, SpecialModelFactory::new(decode, bake))` with a
fully qualified type id. Use `SpecialModelConfig::decode` to read the inline `model` fields
other than `type` into your own config type. The bake callback consumes that config and can
keep geometry leases for the current resource load. Reloading rebuilds those leases and
renderers, without firing this hook again.

On each draw, `SpecialModelRenderer::extract_argument` reads a borrowed `SpecialModelStack`
and returns the argument for `submit`. Submit geometry or nested stacks through
`SpecialModelCollector`, using `with_pose` to scope local transforms. The game applies the
`base` model's display transforms, particle and GUI light, and handles foil, packed light,
overlay and outline through the normal item draw path.

An unknown type, decode or bake error, or panic during resource loading refuses that item
definition without falling back to a lower pack. At draw time a callback error, panic or
invalid geometry or pose rolls back the whole layer and disables the offending type until
resources reload. Unreadable input or a nested refusal rolls back the enclosing layer
without disabling the parent renderer. Only the outermost refused layer draws the missing
model."#]
    #[hook(
        receiver = "associated",
        dispatch = "flat",
        side = "client",
        contract = "listen",
        context = "registration",
        neoforge = "RegisterSpecialModelRendererEvent",
        version = 2
    )]
    fn register_special_model_renderer(renderers: &mut dyn SpecialModelRenderers);
}
