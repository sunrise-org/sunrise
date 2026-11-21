//! The `items` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0153

pub trait Hooks {
    /// Declare items in code once at startup on each side, before resources load.
    /// Pass each `mod_api::Item` to `event.register`. The declaration belongs to the
    /// submitting package and works like `data/<id>/sunrise/item/<path>.json` in a
    /// content mod. A block's item is declared with its block, not here.
    /// `register` returns a typed `DeclarationError` for invalid IDs or fields.
    /// The game records the refusal and stops startup even if you ignore the error.
    #[hook(
        target = "sunrise_client::game::native_hooks::base::Game<'_>::native_items",
        receiver = "associated",
        dispatch = "flat",
        side = "both",
        contract = "collect",
        out = "items",
        context = "registration",
        neoforge = "RegisterEvent",
        version = 2
    )]
    fn items(
        items: &mut dyn FnMut(
            mod_api::Answered<mod_api::Item>,
        ) -> Result<(), mod_api::DeclarationError>,
    );
}
