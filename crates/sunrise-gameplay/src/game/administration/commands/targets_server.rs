use super::*;
pub(in crate::game::server_side) use crate::game::commands::targets::non_player_entity_id;
use sunrise_gameplay_common::game::entity::{
    DragonPartRef, EntityProjection, with_common_entity_projection,
};

fn natural_knot_entity<'a>(
    game: &'a crate::game::ServerGame<'_>,
    id: u32,
) -> Option<&'a sunrise_gameplay_common::leash_knot::Knot> {
    use sunrise_sim::join_seq::residency::table::EntityLookup;
    let EntityLookup::Found {
        location,
        value: Target::Knot(_),
    } = game.server_resident_entity(id)
    else {
        return None;
    };
    if location.owner.dimension != game.common.dim.key() {
        return None;
    }
    let index = game.common.resident_entities.knots.locate(location.join)?;
    Some(&game.common.resident_entities.knots[index])
}

/// Read-only lookup of any server entity target through [`EntityProjection`].
pub(in crate::game) fn with_server_entity_projection<R>(
    game: &crate::game::ServerGame<'_>,
    target: Target,
    visit: impl FnOnce(&dyn EntityProjection) -> R,
) -> Option<R> {
    match target {
        Target::Guest(conn) => game.command_guest(conn).map(|g| visit(g)),
        Target::Other(id) => game
            .server
            .others
            .0
            .iter()
            .find(|o| o.id == id)
            .map(|o| visit(o)),
        Target::Hook(id) => game.fishing_hook_by_id(id).map(|(h, _)| visit(h)),
        Target::DragonPart(id) => game
            .resident_dragon_part(id)
            .map(|(d, i)| visit(&DragonPartRef::new(id, d, i))),
        Target::Knot(id) => natural_knot_entity(game, id).map(|k| visit(k)),
        other => with_common_entity_projection(&game.common, other, visit),
    }
}

impl crate::game::ServerGame<'_> {
    /// The level `t` is in (`Entity.level`): a server's players have their
    /// own, which need not be the one in place; everything else is in the
    /// level in place.
    pub fn target_level(&self, t: Target) -> Dim {
        let conn = match t {
            Target::Player => self.server.lan_acting,
            Target::Guest(conn) => Some(conn),
            _ => None,
        };
        conn.and_then(|c| self.command_guest(c))
            .map_or(self.common.dim, |g| Dim::from_id(&g.dimension))
    }
    pub fn target_position(&self, t: Target) -> Option<[f64; 3]> {
        with_server_entity_projection(self, t, |entity| {
            let p = entity.projected_position();
            [p.x, p.y, p.z]
        })
    }
}

use sunrise_gameplay_common::game::commands_common::CommandsCommonGameExt2 as _;

use sunrise_gameplay_common::game::dragon_common::DragonCommonGameExt1 as _;
