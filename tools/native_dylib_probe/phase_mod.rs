//! A native mod whose hook calls a Host method its load phase refuses, then one it permits (#3123). The probe builds
//! it as the shipped route builds a mod, and fires the hook under the Setup phase.
use mod_api::{Flow, Hooks, PotionBrewPreEvent, ServerHost};
use std::panic::{AssertUnwindSafe, catch_unwind};

#[derive(Default)]
struct Mod;

#[mod_api::sunrise_mod]
impl Hooks for Mod {
    fn potion_brew_pre(
        &self,
        host: &mut dyn ServerHost,
        event: &mut PotionBrewPreEvent<'_>,
    ) -> Flow {
        // Undone by the game when this listener is refused.
        event.cancelled = true;
        // Positive control: the Setup phase permits this Metadata read, which answers before any
        // refusal (a panic here is no refusal and fails the probe); the same call refused later can
        // only be the latch.
        let answered = host.fluid_is_same("minecraft:water", "minecraft:water");
        let refusal = |call: &mut dyn FnMut()| {
            catch_unwind(AssertUnwindSafe(call))
                .err()
                .is_some_and(|payload| payload.is::<mod_api::__private::PhaseRefusal>())
        };
        let first = refusal(&mut || {
            host.entity(1);
        });
        // The mod's own catch does not undo the refusal: the permitted call now refuses too.
        let latched = refusal(&mut || {
            host.fluid_is_same("minecraft:water", "minecraft:water");
        });
        mod_api::content::crash::set_detail(
            "phase probe",
            format!("answered {answered} first {first} latched {latched}"),
        );
        Flow::Cancel
    }
}
