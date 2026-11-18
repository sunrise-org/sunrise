use mod_api::{ClientHost, Hooks, TickTargetMillisEvent};

#[derive(Default)]
pub struct MinimalMod;

#[mod_api::sunrise_mod]
impl Hooks for MinimalMod {
    fn tick_target_millis(&self, host: &dyn ClientHost, _event: &mut TickTargetMillisEvent) {
        let _ = host.math_profile();
    }
}
