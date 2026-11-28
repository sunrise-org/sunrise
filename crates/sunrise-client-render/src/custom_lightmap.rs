//! Custom lightmaps: a pack's own light texture for a world, in place of the one computed from
//! the frame's `LightmapInfo` (`gfx::core_lightmap`). The game names the frame's world and weather
//! ([`Weather`]); the renderer asks the pack's [`Lightmaps`] for the texture, and computes it
//! as usual when there is none. A pack's `core/lightmap` shader still draws over both.

use crate::resource_source::ResourceSource;
use std::sync::Arc;

/// The light texture: RGBA rows of sky level by block level.
pub type Pixels = [u8; 16 * 16 * 4];

/// The frame's world and weather, as the game sees them.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Weather {
    /// The world's number (`-1` the Nether, `0` the Overworld and other worlds, `1` the End);
    /// none on the title screen.
    pub world: Option<i32>,
    /// The rain strength at the frame, 0 to 1, and the storm's: the thunder level already
    /// scaled by the rain (so at most `rain`), as the level reports it.
    pub rain: f32,
    pub thunder: f32,
    /// Lightning flashes the sky.
    pub lightning: bool,
}

/// What the light texture depends on, besides the world.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Input {
    /// The sky light factor, 0 to 1. In the End it carries the End flash, so an End lightmap's
    /// column follows the flash as the default texture does.
    pub sky: f32,
    /// The block light's flicker (`Frame::block_flicker`).
    pub flicker: f32,
    /// The brightness the default texture uses: the option, less the Darkness effect's blend.
    pub gamma: f32,
    /// Night vision of any strength (the effect, its fading pulse, conduit water vision): a
    /// lightmap has night vision rows or not, with nothing in between.
    pub night_vision: bool,
    /// The darkness effect's pulse.
    pub darkness: f32,
    pub weather: Weather,
}

/// A pack's custom lightmaps, by world.
pub trait Lightmaps: Send + Sync {
    /// Whether there are none at all.
    fn is_empty(&self) -> bool;
    /// Whether `world` has one.
    fn has(&self, world: i32) -> bool;
    /// `world`'s light texture for `input`; none to compute it as usual.
    fn pixels(&self, world: i32, input: &Input) -> Option<Pixels>;
}

/// No custom lightmap.
struct NoLightmaps;

impl Lightmaps for NoLightmaps {
    fn is_empty(&self) -> bool {
        true
    }

    fn has(&self, _: i32) -> bool {
        false
    }

    fn pixels(&self, _: i32, _: &Input) -> Option<Pixels> {
        None
    }
}

/// No custom lightmaps, including in resources loaded for a server.
pub fn empty() -> Arc<dyn Lightmaps> {
    Arc::new(NoLightmaps)
}

/// The custom lightmaps of the overlay at `root`, from the installed pack formats.
pub fn load(root: &ResourceSource) -> Arc<dyn Lightmaps> {
    crate::pack_hooks::formats()
        .lightmaps(root)
        .unwrap_or_else(empty)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_lightmaps_compute_the_texture_as_usual() {
        let input = Input {
            sky: 1.0,
            flicker: 0.0,
            gamma: 0.5,
            night_vision: false,
            darkness: 0.0,
            weather: Weather {
                world: Some(0),
                ..Default::default()
            },
        };
        let none = empty();
        assert!(none.is_empty() && !none.has(0));
        assert!(none.pixels(0, &input).is_none());
    }
}
