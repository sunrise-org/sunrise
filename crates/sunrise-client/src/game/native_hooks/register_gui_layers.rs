use super::*;

impl Game<'_> {
    /// Registers HUD layers and their draw callbacks (NeoForge's `RegisterGuiLayersEvent`).
    /// Asked once at client startup.
    pub(in crate::game) fn native_register_gui_layers(
        context: &mod_api::RegistrationContext,
        layers: &mut dyn GuiLayerRegistrar,
    ) {
        Self::fire_register_gui_layers(context, layers);
    }
}
