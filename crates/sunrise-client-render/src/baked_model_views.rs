//! Keys and read-only copies of the published baked model store (#861 part 1): what
//! `RenderAssets::block_state_models` and `RenderAssets::baked_model` answer a mod. The store is
//! engine-owned and immutable for its resource generation; a mod only ever gets copies.
use crate::{assets::RenderAssets, models::catalog::BakedModel};
use mod_api::{BakedModelKey, BakedModelView, BakedQuad, ModelError};
use sunrise_assets::model::Direction as FaceDirection;
use sunrise_core::geometry::Direction;
use sunrise_world::registry::BlockStateId;

impl RenderAssets {
    /// Keys of every baked model the resolved `state` can select, in bake order without repeats.
    /// Empty for a state with no binding in this store.
    pub fn block_state_models(&self, state: BlockStateId) -> Vec<BakedModelKey> {
        self.model_catalog
            .state_slots(state)
            .unwrap_or_default()
            .into_iter()
            .map(|slot| mod_api::__private::built::baked_model_key(self.generation, slot))
            .collect()
    }

    /// A copy of the baked model `key` names, validated against this store's generation first.
    pub fn baked_model(&self, key: BakedModelKey) -> Result<BakedModelView, ModelError> {
        use mod_api::__private::game::Fields;
        let fields = key.fields();
        if fields.generation != self.generation {
            return Err(ModelError::StaleModel);
        }
        let model = self
            .model_catalog
            .model_by_slot(fields.slot)
            .ok_or(ModelError::UnknownModel)?;
        Ok(view(model, |layer| self.sprite_name(layer)))
    }

    /// The name of the sprite at atlas layer `layer`, when one is named there.
    fn sprite_name(&self, layer: u32) -> Option<String> {
        self.sprite_layers
            .iter()
            .find(|(_, at)| **at == layer)
            .map(|(name, _)| name.clone())
    }
}

/// The API's copy of `model`; `sprite_name` names its particle layer.
pub(crate) fn view(
    model: &BakedModel,
    sprite_name: impl FnOnce(u32) -> Option<String>,
) -> BakedModelView {
    let mut view = mod_api::__private::built::baked_model_view();
    view.ambient_occlusion = model.ambient_occlusion;
    view.particle = model.particle.and_then(sprite_name);
    view.quads = model
        .faces
        .iter()
        .map(|face| {
            let quad = &face.quad;
            let mut out = BakedQuad::new(
                quad.positions,
                quad.uv,
                quad.material.sprite.clone(),
                direction(quad.facing),
            );
            out.cull = quad.cull.map(direction);
            out.tint_index = quad.tint_index;
            out.shade = quad.shade;
            out.light_emission = quad.light_emission;
            out
        })
        .collect();
    view
}

/// The API's direction for a model face's.
pub(crate) fn direction(face: FaceDirection) -> Direction {
    match face {
        FaceDirection::Down => Direction::Down,
        FaceDirection::Up => Direction::Up,
        FaceDirection::North => Direction::North,
        FaceDirection::South => Direction::South,
        FaceDirection::West => Direction::West,
        FaceDirection::East => Direction::East,
    }
}

#[cfg(test)]
#[path = "baked_model_views_tests.rs"]
mod tests;
