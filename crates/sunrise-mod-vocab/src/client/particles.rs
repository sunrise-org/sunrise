//! Client firework burst requests.
#[allow(unused_imports)]
use crate::*;

/// One explosion of a [`FireworksRequest`] (a firework star's `FireworkExplosion`): the
/// pattern its sparks fly out in and how they look.
#[derive(sunrise_mod_api_macros::Wire, Clone, Debug, Default, PartialEq)]
// exhaustive: mods build it with a literal, as a firework star's components spell it
pub struct FireworkExplosion {
    /// The pattern the sparks fly out in.
    pub shape: FireworkShape,
    /// The sparks' colours as `0xRRGGBB`, one picked at random for each spark; none draws
    /// black sparks, as vanilla's (`DyeColor.BLACK.getFireworkColor`).
    pub colors: Vec<u32>,
    /// The colours the sparks fade to as `0xRRGGBB`, one picked at random for each spark;
    /// none keeps their colour.
    pub fade_colors: Vec<u32>,
    /// Each spark leaves a trail of smaller sparks (`hasTrail`).
    pub trail: bool,
    /// The sparks flicker, and a crackle follows the burst (`hasTwinkle`).
    pub twinkle: bool,
}

/// A firework's explosions drawn in this client's world, for
/// [`crate::ClientHost::create_fireworks`] (`ClientLevel.createFireworks`): the picture and
/// the sound of a rocket bursting, with no rocket.
#[derive(sunrise_mod_api_macros::Wire, Clone, Debug, Default, PartialEq)]
// exhaustive: mods build it with a literal
pub struct FireworksRequest {
    /// Where it bursts, in blocks.
    pub at: [f64; 3],
    /// The rocket's velocity in blocks per tick, which the sparks carry.
    pub velocity: [f64; 3],
    /// The explosions, one every other tick in this order; none puffs a little smoke, as a
    /// rocket without stars does.
    pub explosions: Vec<FireworkExplosion>,
    /// The blast, and the crackle of a twinkling explosion, play; false bursts silently, as a
    /// silent rocket does (`!FireworkRocketEntity.isSilent()`).
    pub audible: bool,
}
