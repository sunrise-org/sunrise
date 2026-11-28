//! The `creative_tabs` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// Declare creative tabs in code once at startup on each side, before resources load.
    /// Pass each `mod_api::CreativeTab` to `event.register`. The declaration belongs to the
    /// submitting package — in its own namespaces or in a content package it serves
    /// (`served_by`) — and works like `data/<id>/sunrise/creative_tab/<path>.json` in a
    /// content mod. A tab's icon and entries are full stacks, so an entry can carry
    /// components; an entry can also be shown to operator viewers only, stay out of the
    /// search tab, or add lines and tags the search tab matches it by.
    /// `register` returns a typed `DeclarationError` for invalid IDs or fields.
    /// The game records the refusal and stops startup even if a returning listener ignores it.
    /// A panic discards this listener's declarations and diagnostics without rejecting startup.
    /// Bare tab IDs use the submitting mod ID; valid explicit names retain their namespace.
    #[hook(
        target = "sunrise_gameplay_common::game::native_hooks::base::native_creative_tabs",
        receiver = "associated",
        dispatch = "flat",
        side = "both",
        contract = "collect",
        out = "creative_tabs",
        context = "registration",
        version = 1
    )]
    fn creative_tabs(
        creative_tabs: &mut dyn FnMut(
            mod_api::Answered<mod_api::CreativeTab>,
        ) -> Result<(), mod_api::DeclarationError>,
    );
}
