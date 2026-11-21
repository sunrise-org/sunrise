use super::*;
impl Game<'_> {
    /// Registers this resource load's item-owned client equipment extensions.
    pub(in crate::game) fn native_register_client_item_extensions(
        context: &mod_api::RegistrationContext,
        extensions: &mut dyn mod_api::ClientItemExtensionsRegistrar,
    ) {
        Self::fire_register_client_item_extensions(context, extensions);
    }
}
