use super::*;

impl Game<'_> {
    /// The speed minecart `cart` takes from the rail `state` at `pos` (shaped `shape` for it),
    /// where `moveAlongTrack` applies a powered rail's halt and boost, under the loop `context` of
    /// the side that moves it. `velocity` is the cart's velocity after vanilla's slope, input and
    /// halt; the parts of `state` may change it. A non-finite answer keeps `velocity` as it was.
    pub(in crate::game) fn native_rail_step(
        context: &dyn mod_api::LoopContext,
        cart: u32,
        state: BlockStateId,
        pos: BlockPos,
        shape: RailShape,
        max_speed: f64,
        velocity: &mut [f64; 3],
    ) {
        use crate::game::api_values::TryIntoApi;
        let Ok(api_state) = state.try_into_api() else {
            return;
        };
        let mut answer = *velocity;
        mod_api::dispatch_rail_step(context, cart, api_state, pos, shape, max_speed, &mut answer);
        if answer.iter().all(|v| v.is_finite()) {
            *velocity = answer;
        }
    }
}
