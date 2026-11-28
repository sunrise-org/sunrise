use super::entity::{Entity as _, EntityHandle, with_entity};
use mod_api::EntityRef;
use sunrise_core::geometry::Vec3;
use sunrise_gameplay_common::game::commands_common::Target;
use sunrise_gameplay_common::game::entity::{
    DragonPartRef, EntityProjection as _, with_common_entity_projection,
};
use sunrise_gameplay_common::game::native_entity_motion_values::corners;

impl crate::game::ClientGame<'_> {
    fn client_walk_entity(&self, id: u32) -> Option<EntityHandle> {
        if let Some(i) = self.common.mobs.iter().position(|e| e.id == id) {
            return Some(EntityHandle::Mob(i));
        }
        if let Some(i) = self.common.dropped.iter().position(|e| e.id == id) {
            return Some(EntityHandle::Item(i));
        }
        if let Some(i) = self.common.boats.iter().position(|e| e.id == id) {
            return Some(EntityHandle::Boat(i));
        }
        if let Some(i) = self.common.carts.iter().position(|e| e.id == id) {
            return Some(EntityHandle::Cart(i));
        }
        if let Some(i) = self.common.armor_stands.iter().position(|e| e.id == id) {
            return Some(EntityHandle::ArmorStand(i));
        }
        if let Some(i) = self.common.tnts.iter().position(|e| e.id == id) {
            return Some(EntityHandle::Tnt(i));
        }
        if let Some(i) = self.common.falling.iter().position(|e| e.id == id) {
            return Some(EntityHandle::Falling(i));
        }
        if let Some(i) = self.common.orbs.iter().position(|e| e.id == id) {
            return Some(EntityHandle::Orb(i));
        }
        if let Some(i) = self
            .common
            .creatures
            .projectiles
            .iter()
            .position(|e| e.id == id)
        {
            return Some(EntityHandle::Projectile(i));
        }
        if let Some(i) = self.common.rockets.iter().position(|e| e.id == id) {
            return Some(EntityHandle::Rocket(i));
        }
        if let Some(i) = self.common.tridents.iter().position(|e| e.id == id) {
            return Some(EntityHandle::Trident(i));
        }
        self.common
            .resident_entities
            .general
            .rows
            .iter()
            .position(|e| e.id == id)
            .map(EntityHandle::General)
    }

    pub(in crate::game) fn client_entity_velocity(&self, entity: EntityRef) -> Option<[f64; 3]> {
        let t = self.native_client_entity_target(entity)?;
        let v = match t {
            Target::Player => self.common.player.as_ref()?.velocity,
            _ => match self.client_walk_entity(entity.id()) {
                Some(h) => with_entity!(self, h, e => e.velocity()),
                None => Vec3::ZERO,
            },
        };
        Some([v.x, v.y, v.z])
    }

    pub(in crate::game) fn client_entity_bounding_box(
        &self,
        entity: EntityRef,
    ) -> Option<[[f64; 3]; 2]> {
        if let Some(pos) = self.side_client_display_position(entity) {
            return Some([pos; 2]);
        }
        if let Ok((remote, id)) = self.native_client_entity_context(entity)
            && let Some(row) = remote.general.rows.get(&id)
        {
            return Some(corners(row.bounds()));
        }
        let t = self.native_client_entity_target(entity)?;
        if t == Target::Player && self.client.remote.is_some() {
            return Some(corners(self.common.player.as_ref()?.bounds()));
        }
        if let Some(h) = self.client_walk_entity(entity.id()) {
            return Some(corners(with_entity!(self, h, e => {
                self.client_copy_box(e.id(), e.position(), e.bounds())
            })));
        }
        let (lo, hi) = match t {
            Target::Other(id) => {
                let i = self.client.interactions.get(&(id as i32))?;
                return Some(corners(i.bounds()));
            }
            Target::DragonPart(id) => {
                let dragon = self.common.dragon.as_ref()?;
                let part =
                    sunrise_gameplay_common::game::dragon_common::dragon_part_index(dragon.id, id)?;
                return DragonPartRef::new(id, dragon, part)
                    .projected_bounds()
                    .map(corners);
            }
            Target::Crystal(id) => {
                let c = self.common.crystals.iter().find(|c| c.id == id)?;
                return Some(corners(self.client_copy_box(
                    id,
                    c.projected_position(),
                    c.projected_bounds()?,
                )));
            }
            Target::Cushion(_) | Target::Frame(_) | Target::Painting(_) => {
                with_common_entity_projection(&self.common, t, |entity| {
                    entity.projected_raw_bounds()
                })
                .flatten()?
            }
            _ => return None,
        };
        if lo.iter().chain(&hi).any(|v| !v.is_finite()) {
            return None;
        }
        Some([lo, hi])
    }

    pub(in crate::game) fn client_entity_eye_height(&self, entity: EntityRef) -> Option<f64> {
        if let Some(target) = self.native_client_entity_target(entity) {
            return Some(self.target_eye_height(target));
        }
        let (remote, id) = self.native_client_entity_context(entity).ok()?;
        remote.tracked(id)?;
        self.client
            .interactions
            .get(&id)
            .map(|i| f64::from(i.eye_height()))
    }
}

use sunrise_gameplay_common::game::side_ops::player::PlayerSideGameExt as _;

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use sunrise_protocol::entity_data::{PaintingHolder, PaintingVariant, Value};
    use sunrise_sim::entities::Painting;

    #[test]
    #[ignore = "daily"]
    fn negative_width_inline_painting_keeps_raw_corners() {
        use crate::game::client_side::native_client_session::tests::logged_client;
        use crate::game::client_side::native_entity_refs::tests::with_client_game;
        use crate::game::client_side::remote_client::Tracked;

        with_client_game(|game| {
            let _server = logged_client(game);
            let id = 901;
            game.common.paintings.push(Painting {
                attachment_lifetime: Default::default(),
                join: Default::default(),
                id,
                pos: [10, 65, -3],
                facing: 2,
                variant: Arc::from("minecraft:kebab"),
                asset: None,
                width: 1,
                height: 1,
                check_interval: 0,
                ticks: 0,
            });
            game.remote_object_data(
                id as i32,
                &[(
                    9,
                    0,
                    Value::PaintingVariant(PaintingHolder::Direct(PaintingVariant {
                        width: -2,
                        height: 1,
                        asset_id: "minecraft:kebab".to_owned(),
                        title: None,
                        author: None,
                    })),
                )],
            );
            game.client.remote.as_mut().unwrap().entities.insert(
                id as i32,
                Tracked {
                    kind: "minecraft:painting".to_owned(),
                    position: [10.5, 65.5, -2.5],
                    previous: [10.5, 65.5, -2.5],
                    yaw: 180.0,
                    pitch: 0.0,
                    previous_yaw: 180.0,
                    previous_pitch: 0.0,
                    codec_base: [10.5, 65.5, -2.5],
                    on_fire: false,
                    mod_fire_immune: false,
                    silent: false,
                    vehicle: None,
                    tick_count: 0,
                    size_answer: None,
                },
            );
            let (lo, hi) = game
                .common
                .paintings
                .iter()
                .find(|p| p.id == id)
                .unwrap()
                .bounds();
            assert!(lo[0] > hi[0]);
            let epoch = game.common.native_lifecycle.epoch().unwrap();
            assert_eq!(
                game.client_entity_bounding_box(EntityRef::new(epoch, id)),
                Some([lo, hi])
            );
        });
    }
}
