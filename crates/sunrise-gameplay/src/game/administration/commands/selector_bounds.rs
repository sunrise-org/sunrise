//! The physical bounds already kept for selector targets. This is not a
//! rendered box or a new source of entity dimensions.
use super::Target;
use crate::game::server_side::commands_server::targets::with_server_entity_projection;
use sunrise_core::geometry::Aabb;
use sunrise_gameplay_common::game::entity::{extents_aabb, knot_bounds};

/// `EntitySelectorParser.createAabb`, moved by the selector origin.
pub(in crate::game) fn selector_box(origin: [f64; 3], delta: [f64; 3]) -> Aabb {
    let lo = std::array::from_fn(|i| origin[i] + delta[i].min(0.0));
    let hi = std::array::from_fn(|i| origin[i] + (delta[i].max(0.0) + 1.0));
    extents_aabb((lo, hi))
}

impl crate::game::ServerGame<'_> {
    /// `Entity.getBoundingBox`: use each simulated kind's existing physical
    /// box. None means no authoritative box is modeled here yet; the selector
    /// retains its existing point fallback for those kinds.
    pub(in crate::game) fn selector_target_bounds(&self, target: Target) -> Option<Aabb> {
        match target {
            Target::Other(_)
            | Target::Fangs(_)
            | Target::Dragon(_)
            | Target::Spawner(_)
            | Target::Cloud(_) => None,
            Target::Knot(id) => self.natural_knot(id).map(knot_bounds),
            other => with_server_entity_projection(self, other, |entity| entity.projected_bounds())
                .flatten(),
        }
    }
}
