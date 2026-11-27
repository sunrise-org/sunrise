use super::*;
pub(in crate::game::server_side) use crate::game::commands::targets::non_player_entity_id;
use sunrise_gameplay_common::game::entity::{
    DragonPartRef, EntityProjection, with_common_entity_projection,
};

/// Read-only lookup of a server entity target through [`EntityProjection`].
#[inline(always)]
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
        match t {
            Target::Knot(id) => self.natural_knot(id).map(|fence| {
                let p = sunrise_sim::mobs::leash::knot_state(fence).position;
                [p.x, p.y, p.z]
            }),
            other => with_server_entity_projection(self, other, |entity| {
                let p = entity.projected_position();
                [p.x, p.y, p.z]
            }),
        }
    }
}

use sunrise_gameplay_common::game::commands_common::CommandsCommonGameExt2 as _;

use sunrise_gameplay_common::game::dragon_common::DragonCommonGameExt1 as _;

#[cfg(test)]
mod projection_tests {
    use super::*;
    use sunrise_core::geometry::Vec3;
    use sunrise_gameplay_common::game::{entity::Entity, entity_memory::OffLevel};
    use sunrise_sim::mobs::Mob;

    #[test]
    fn mob_and_off_level_mob_projection_match_body() {
        let mut mob = Mob::new(
            301,
            sunrise_sim::mobs::sensor_scan_tests::kind(),
            Vec3::new(4.0, 64.0, -5.0),
            90.0,
        );
        mob.head_pitch = -12.0;
        mob.velocity = Vec3::new(0.1, -0.05, 0.2);
        mob.ground.on_ground = true;
        mob.fire_ticks = 42;
        mob.air = 195;
        mob.removed = true;
        assert_eq!(mob.projected_id(), mob.id);
        assert_eq!(mob.projected_kind(), Entity::native_type(&mob));
        assert_eq!(mob.projected_position(), mob.position);
        assert_eq!(mob.projected_velocity(), Some(mob.velocity));
        assert_eq!(mob.projected_rotation(), Some([mob.yaw, mob.head_pitch]));
        assert_eq!(mob.projected_bounds(), Some(mob.bounds()));
        assert_eq!(mob.projected_on_ground(), Some(mob.ground.on_ground));
        assert_eq!(mob.projected_body_fire_ticks(), Some(mob.fire_ticks));
        assert_eq!(mob.projected_air_supply(), Some(mob.air));
        assert_eq!(mob.projected_body_removed(), Some(mob.removed));
        assert_eq!(mob.projected_body_uuid(), None);

        let expected_bounds = mob.bounds();
        let off = OffLevel::Mob(mob);
        assert_eq!(off.projected_id(), 301);
        assert_eq!(off.projected_kind(), "minecraft:cow");
        assert_eq!(off.projected_bounds(), Some(expected_bounds));
        assert_eq!(off.projected_body_fire_ticks(), Some(42));
        assert_eq!(off.projected_air_supply(), Some(195));
        assert_eq!(off.projected_body_removed(), Some(true));
    }

    #[test]
    #[ignore = "daily"]
    fn player_motion_projection_matches_body() {
        let root = crate::game::native_entity_refs::tests::fixture_root();
        let assets = sunrise_level::assets::Assets::load(&root).unwrap();
        let resources = sunrise_sim::motion::MotionResources::load(&root, &assets).unwrap();
        let mut player =
            sunrise_sim::motion::PlayerMotion::new(&resources, Vec3::new(1.0, 64.0, 2.0)).unwrap();
        player.entity_id = 302;
        player.velocity = Vec3::new(0.2, 0.0, -0.1);
        player.yaw = 75.0;
        player.pitch = -10.0;
        player.ground.on_ground = true;
        player.fire_ticks = 35;
        player.air = 260;
        assert_eq!(player.projected_id(), player.entity_id as u32);
        assert_eq!(player.projected_kind(), "minecraft:player");
        assert_eq!(player.projected_position(), player.position);
        assert_eq!(player.projected_velocity(), Some(player.velocity));
        assert_eq!(
            player.projected_rotation(),
            Some([player.yaw, player.pitch])
        );
        assert_eq!(player.projected_bounds(), Some(player.bounds()));
        assert_eq!(player.projected_on_ground(), Some(player.ground.on_ground));
        assert_eq!(player.projected_body_fire_ticks(), Some(player.fire_ticks));
        assert_eq!(player.projected_air_supply(), Some(player.air));
        assert_eq!(player.projected_body_removed(), None);
        assert_eq!(player.projected_body_uuid(), None);
    }
}
