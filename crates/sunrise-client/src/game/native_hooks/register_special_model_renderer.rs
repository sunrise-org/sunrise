use super::*;

impl Game<'_> {
    /// Collect the mods' special item model factories once at client startup, before loading
    /// resources (NeoForge's `RegisterSpecialModelRendererEvent`). The last registration of a
    /// type wins over any earlier mod or builtin registration. Resource loads reuse the factories
    /// to decode and bake each `minecraft:special` node, rather than firing this hook again.
    pub(in crate::game) fn native_register_special_model_renderer(
        context: &mod_api::RegistrationContext,
        renderers: &mut dyn SpecialModelRenderers,
    ) {
        Self::fire_register_special_model_renderer(context, renderers);
    }
}
