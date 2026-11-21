//! Read-only keys and views of the engine-owned baked model cache (#861 part 1).
//!
//! Every successful resource load bakes each block state's models and the standalone models
//! once into one immutable store for that resource generation. A mod gets scoped keys into that
//! store and copies of its quads, never the renderer's models: nothing a mod holds can change
//! what the mesher draws.
use crate::{Direction, ModelRef};

/// One baked model of the published model store: a block state's variant model or a standalone
/// model, for one resource generation. Get it from
/// [`RenderAssets::block_state_models`](crate::RenderAssets::block_state_models) or from a
/// [`ModelRef`] (`BakedModelKey::from(model_ref)`), and read it with
/// [`RenderAssets::baked_model`](crate::RenderAssets::baked_model). A successful resource reload
/// supersedes it: the old key then answers [`ModelError::StaleModel`](crate::ModelError::StaleModel)
/// and a new one is asked for in `model_baking_completed`.
// no wire: decoding from_parts rejects generation zero (an unpublished model store).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
// exhaustive: private fields; only the engine issues keys
pub struct BakedModelKey(pub(crate) u64, pub(crate) u32);

impl BakedModelKey {
    pub(crate) fn from_parts(generation: u64, slot: u32) -> Result<Self, &'static str> {
        if generation == 0 {
            return Err("unpublished model generation");
        }
        Ok(Self(generation, slot))
    }
}

impl From<ModelRef> for BakedModelKey {
    /// The baked model a standalone model handle names, in the same generation.
    fn from(model: ModelRef) -> Self {
        Self(model.0, model.1)
    }
}

/// A read-only copy of the listed fields of one baked model: its smooth-lighting flag, particle
/// sprite and quads. It is not a complete copy of what the mesher reads: the material render
/// type, per-quad colour, per-quad ambient occlusion and shading overrides are not carried.
/// Changing it changes nothing the game draws.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub struct BakedModelView {
    /// Whether the model allows smooth lighting (`ambientocclusion`).
    pub ambient_occlusion: bool,
    /// The block atlas sprite its particles use (`minecraft:block/stone`), when it names one.
    pub particle: Option<String>,
    /// Its quads in bake order, already turned and UV-locked as the variant asks.
    pub quads: Vec<BakedQuad>,
}

/// One quad of a baked model ([`BakedModelView::quads`]).
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub struct BakedQuad {
    /// Corners in block units (0 to 1 across the block for a unit cube), counter-clockwise seen
    /// from the side the quad faces.
    pub positions: [[f32; 3]; 4],
    /// Each corner's texture coordinates, 0 to 1 across the sprite.
    pub uvs: [[f32; 2]; 4],
    /// The block atlas sprite it is drawn with (`minecraft:block/stone`).
    pub sprite: String,
    /// The side it faces, which decides its directional shading.
    pub facing: Direction,
    /// The face whose solid neighbour hides the quad; `None` never culls.
    pub cull: Option<Direction>,
    /// The tint index a block tint source colours it by; -1 is none.
    pub tint_index: i32,
    /// Whether it takes directional shading.
    pub shade: bool,
    /// Its `light_emission`, 0 to 15.
    pub light_emission: u8,
}

impl BakedQuad {
    /// A quad drawn with block atlas sprite `sprite`, facing `facing`: culled by no side, untinted
    /// (-1), directionally shaded and emitting no light. Set the other fields after.
    pub fn new(
        positions: [[f32; 3]; 4],
        uvs: [[f32; 2]; 4],
        sprite: impl Into<String>,
        facing: Direction,
    ) -> Self {
        Self {
            positions,
            uvs,
            sprite: sprite.into(),
            facing,
            cull: None,
            tint_index: -1,
            shade: true,
            light_emission: 0,
        }
    }
}
