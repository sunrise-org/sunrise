//! The `health_hud` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0056

pub trait Hooks {
    /// The player-health HUD rows, once a frame the HUD draws them
    /// (`HealthHud::layout`, not under F1): `inputs` carries the health,
    /// food, air and vehicle state the layout reads, `wi`/`hi` the screen's
    /// GUI size. A mod that sets `draws` replaces every row — armour,
    /// hearts, food and air are the answer's to give; `bubble_pop_sound`
    /// too.
    /// Nothing set draws Sunrise's own.
    #[hook(
        target = "sunrise_client::game::native_hooks::base::Game<'_>::native_health_hud",
        dispatch = "flat",
        side = "client",
        contract = "replace",
        out = "draws",
        neoforge = "RenderGuiLayerEvent.Pre"
    )]
    fn health_hud(
        inputs: &mod_api::client::hud_layout::HealthInputs,
        wi: i32,
        hi: i32,
        draws: &mut Option<Vec<GuiDraw>>,
    );
}
