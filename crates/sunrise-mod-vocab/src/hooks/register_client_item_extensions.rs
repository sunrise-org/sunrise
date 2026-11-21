//! Client item extensions, rebuilt with the equipment resource bindings.
pub trait Hooks {
    /// Register this load's worn equipment models, textures and item providers together.
    /// Registration is client-only and repeats on every resource load. Models are baked once;
    /// frame callbacks choose references, never geometry. A duplicate item provider or an unknown
    /// texture refuses registration with a named diagnostic. Server-only packages cannot listen.
    #[hook(
        receiver = "associated",
        dispatch = "flat",
        side = "client",
        contract = "listen",
        context = "registration",
        neoforge = "RegisterClientExtensionsEvent"
    )]
    fn register_client_item_extensions(extensions: &mut dyn mod_api::ClientItemExtensionsRegistrar);
}
