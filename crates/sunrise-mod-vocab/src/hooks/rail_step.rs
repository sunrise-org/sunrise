//! The `rail_step` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// The speed minecart `cart` takes from the rail `state` at `pos` it is moving on, so a
    /// booster, brake or station rail can push, slow or hold a cart as vanilla's powered rail
    /// does in `moveAlongTrack`. Movement phase 5, the rail step. `shape` is the track the rail
    /// lays for the cart (after `rail_direction`). `velocity` is the cart's velocity after
    /// vanilla's slope, rider input and powered-rail halt; set it to change the speed the cart
    /// moves with. `max_speed` is the cart's `getMaxSpeed`. Vanilla's speed cap and its
    /// powered-rail boost still apply after the hook. With the old minecart physics it is asked
    /// once per tick, before the cart moves; with `minecart_improvements` once per rail the cart
    /// crosses in a tick, before the first rail's slowdown and cap. A non-finite answer leaves
    /// the velocity as it was before the hook.
    ///
    /// It fires on every side that simulates the cart's movement, and a client never drives a
    /// minecart, so a `side = "server"` mod's listener runs for every cart. Keep the listener a
    /// pure function of its inputs and what both sides know that tick. It is given the loop
    /// `context` of the side that ticks the cart, and takes no `Host`. With no part attached to
    /// the state, the cost is one table lookup per rail step. NeoForge 26.3 lets a mod rail
    /// change a cart's speed only by being a `PoweredRailBlock`; this is that halt and boost made
    /// general. Diverges from upstream: new hook.
    #[hook(
        receiver = "associated",
        dispatch = "direct",
        side = "both",
        contract = "collect",
        out = "velocity",
        context = "loop",
        scope = "block",
        key = "state",
        neoforge = "none (vanilla OldMinecartBehavior/NewMinecartBehavior.moveAlongTrack's powered-rail halt and boost)"
    )]
    fn rail_step(
        cart: u32,
        state: BlockState,
        pos: BlockPos,
        shape: RailShape,
        max_speed: f64,
        velocity: &mut [f64; 3],
    );
}
