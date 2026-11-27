//! Read-only row-local projection of entity body fields across all entity families.
use super::Entity;
use crate::game::{
    CommonGame, FallingBlock,
    boats_common::Boat,
    commands_common::{OtherEntity, Target},
    dragon_common::{Breath, Crystal, Dragon},
    end_state::EyeOfEnder,
    entity_memory::OffLevel,
    fangs_common::Fangs,
    fireworks::Rocket,
    fishing_common::Hook,
    general::GeneralRow,
    lightning_common::Bolt,
    minecarts::Minecart,
    ominous_item_spawner_common::OminousItemSpawner,
    potions_common::{ThrownPotion, thrown_kind},
    tnt_common::PrimedTnt,
};
use crate::leash_knot::Knot;
use std::borrow::Cow;
use sunrise_core::geometry::{Aabb, Vec3};
use sunrise_sim::{
    entities::{ArmorStand, Cushion, ExperienceOrb, ItemEntity, ItemFrame, Painting},
    lan::guest::Guest,
    mobs::Mob,
    motion::PlayerMotion,
    projectiles::Projectile,
    thrown_trident::ThrownTrident,
};

/// Row-local body fields shared across entity kinds; `None` means not stored on the body.
pub trait EntityProjection {
    fn projected_id(&self) -> u32;
    fn projected_kind(&self) -> Cow<'_, str>;
    fn projected_position(&self) -> Vec3;
    #[inline(always)]
    fn projected_velocity(&self) -> Option<Vec3> {
        None
    }
    #[inline(always)]
    fn projected_rotation(&self) -> Option<[f32; 2]> {
        None
    }
    #[inline(always)]
    fn projected_yaw(&self) -> Option<f32> {
        self.projected_rotation().map(|[yaw, _]| yaw)
    }
    #[inline(always)]
    fn projected_bounds(&self) -> Option<Aabb> {
        None
    }
    #[inline(always)]
    fn projected_raw_bounds(&self) -> Option<([f64; 3], [f64; 3])> {
        None
    }
    #[inline(always)]
    fn projected_on_ground(&self) -> Option<bool> {
        None
    }
    #[inline(always)]
    fn projected_body_fire_ticks(&self) -> Option<i32> {
        None
    }
    #[inline(always)]
    fn projected_air_supply(&self) -> Option<i32> {
        None
    }
    #[inline(always)]
    fn projected_body_removed(&self) -> Option<bool> {
        None
    }
    #[inline(always)]
    fn projected_body_uuid(&self) -> Option<[i32; 4]> {
        None
    }
}

/// Borrowed view of one dragon part together with its assigned runtime id.
#[derive(Clone, Copy)]
pub struct DragonPartRef<'a> {
    pub id: u32,
    pub dragon: &'a Dragon,
    pub part: usize,
}

impl<'a> DragonPartRef<'a> {
    #[inline(always)]
    pub const fn new(id: u32, dragon: &'a Dragon, part: usize) -> Self {
        Self { id, dragon, part }
    }
}

#[inline(always)]
pub fn extents_aabb((lo, hi): ([f64; 3], [f64; 3])) -> Aabb {
    Aabb::new(
        Vec3::new(lo[0], lo[1], lo[2]),
        Vec3::new(hi[0], hi[1], hi[2]),
    )
}

#[inline(always)]
fn bounds_center((lo, hi): ([f64; 3], [f64; 3])) -> Vec3 {
    Vec3::new(
        (lo[0] + hi[0]) / 2.0,
        (lo[1] + hi[1]) / 2.0,
        (lo[2] + hi[2]) / 2.0,
    )
}

#[inline(always)]
pub fn knot_bounds(fence: [i32; 3]) -> Aabb {
    let knot = sunrise_sim::mobs::leash::knot_state(fence);
    let (p, half) = (knot.position, knot.width / 2.0);
    Aabb::new(
        Vec3::new(p.x - half, p.y, p.z - half),
        Vec3::new(p.x + half, p.y + knot.height, p.z + half),
    )
}

macro_rules! impl_vanilla_typed_projection {
    ($ty:ty { $($extra:item)* }) => {
        impl EntityProjection for $ty {
            #[inline(always)]
            fn projected_id(&self) -> u32 {
                self.id
            }
            #[inline(always)]
            fn projected_kind(&self) -> Cow<'_, str> {
                Entity::native_type(self)
            }
            #[inline(always)]
            fn projected_position(&self) -> Vec3 {
                self.position
            }
            #[inline(always)]
            fn projected_velocity(&self) -> Option<Vec3> {
                Some(self.velocity)
            }
            #[inline(always)]
            fn projected_rotation(&self) -> Option<[f32; 2]> {
                Some(Entity::rotation(self))
            }
            #[inline(always)]
            fn projected_bounds(&self) -> Option<Aabb> {
                Some(Entity::bounds(self))
            }
            #[inline(always)]
            fn projected_on_ground(&self) -> Option<bool> {
                Some(self.ground.on_ground)
            }
            $($extra)*
        }
    };
}

impl_vanilla_typed_projection!(Mob {
    #[inline(always)]
    fn projected_body_fire_ticks(&self) -> Option<i32> {
        Some(self.fire_ticks)
    }
    #[inline(always)]
    fn projected_air_supply(&self) -> Option<i32> {
        Some(self.air)
    }
    #[inline(always)]
    fn projected_body_removed(&self) -> Option<bool> {
        Some(self.removed)
    }
});

impl_vanilla_typed_projection!(ItemEntity {
    #[inline(always)]
    fn projected_body_fire_ticks(&self) -> Option<i32> {
        Some(self.fire_ticks)
    }
});

impl_vanilla_typed_projection!(ExperienceOrb {
    #[inline(always)]
    fn projected_body_fire_ticks(&self) -> Option<i32> {
        Some(self.fire_ticks)
    }
    #[inline(always)]
    fn projected_air_supply(&self) -> Option<i32> {
        Some(self.air)
    }
});

impl_vanilla_typed_projection!(Boat {
    #[inline(always)]
    fn projected_body_fire_ticks(&self) -> Option<i32> {
        Some(self.fire_ticks)
    }
});

impl_vanilla_typed_projection!(Minecart {});

impl_vanilla_typed_projection!(ArmorStand {
    #[inline(always)]
    fn projected_body_fire_ticks(&self) -> Option<i32> {
        Some(self.fire_ticks)
    }
    #[inline(always)]
    fn projected_air_supply(&self) -> Option<i32> {
        Some(self.air)
    }
});

impl_vanilla_typed_projection!(PrimedTnt {});

impl_vanilla_typed_projection!(FallingBlock {
    #[inline(always)]
    fn projected_body_fire_ticks(&self) -> Option<i32> {
        Some(self.fire_ticks)
    }
    #[inline(always)]
    fn projected_body_uuid(&self) -> Option<[i32; 4]> {
        self.saved.as_deref().map(|(uuid, _)| *uuid)
    }
});

impl_vanilla_typed_projection!(Projectile {
    #[inline(always)]
    fn projected_body_fire_ticks(&self) -> Option<i32> {
        Some(self.arrow.as_ref().map_or(self.fire_ticks, |a| a.fire_ticks))
    }
    #[inline(always)]
    fn projected_air_supply(&self) -> Option<i32> {
        Some(self.air_supply)
    }
    #[inline(always)]
    fn projected_body_removed(&self) -> Option<bool> {
        Some(self.destroyed || self.arrow.as_ref().is_some_and(|a| a.removed))
    }
    #[inline(always)]
    fn projected_body_uuid(&self) -> Option<[i32; 4]> {
        self.uuid
    }
});

impl_vanilla_typed_projection!(Rocket {});

impl_vanilla_typed_projection!(ThrownTrident {
    #[inline(always)]
    fn projected_body_fire_ticks(&self) -> Option<i32> {
        Some(self.fire_ticks)
    }
});

impl EntityProjection for GeneralRow {
    #[inline(always)]
    fn projected_id(&self) -> u32 {
        self.id
    }
    #[inline(always)]
    fn projected_kind(&self) -> Cow<'_, str> {
        Entity::native_type(self)
    }
    #[inline(always)]
    fn projected_position(&self) -> Vec3 {
        self.position
    }
    #[inline(always)]
    fn projected_velocity(&self) -> Option<Vec3> {
        Some(self.velocity)
    }
    #[inline(always)]
    fn projected_rotation(&self) -> Option<[f32; 2]> {
        Some(self.rotation)
    }
    #[inline(always)]
    fn projected_bounds(&self) -> Option<Aabb> {
        Some(Entity::bounds(self))
    }
    #[inline(always)]
    fn projected_on_ground(&self) -> Option<bool> {
        Some(self.on_ground)
    }
    #[inline(always)]
    fn projected_body_fire_ticks(&self) -> Option<i32> {
        Some(self.base.fire_ticks)
    }
    #[inline(always)]
    fn projected_body_uuid(&self) -> Option<[i32; 4]> {
        Some(self.uuid)
    }
}

impl EntityProjection for Painting {
    #[inline(always)]
    fn projected_id(&self) -> u32 {
        self.id
    }
    #[inline(always)]
    fn projected_kind(&self) -> Cow<'_, str> {
        Cow::Borrowed("minecraft:painting")
    }
    #[inline(always)]
    fn projected_position(&self) -> Vec3 {
        bounds_center(self.bounds())
    }
    #[inline(always)]
    fn projected_yaw(&self) -> Option<f32> {
        Some(f32::from(self.facing) * 90.0)
    }
    #[inline(always)]
    fn projected_bounds(&self) -> Option<Aabb> {
        Some(extents_aabb(self.bounds()))
    }
    #[inline(always)]
    fn projected_raw_bounds(&self) -> Option<([f64; 3], [f64; 3])> {
        Some(self.bounds())
    }
}

impl EntityProjection for ItemFrame {
    #[inline(always)]
    fn projected_id(&self) -> u32 {
        self.id
    }
    #[inline(always)]
    fn projected_kind(&self) -> Cow<'_, str> {
        Cow::Borrowed(self.kind())
    }
    #[inline(always)]
    fn projected_position(&self) -> Vec3 {
        bounds_center(self.bounds())
    }
    #[inline(always)]
    fn projected_rotation(&self) -> Option<[f32; 2]> {
        Some(self.entity_rotation())
    }
    #[inline(always)]
    fn projected_bounds(&self) -> Option<Aabb> {
        Some(extents_aabb(self.bounds()))
    }
    #[inline(always)]
    fn projected_raw_bounds(&self) -> Option<([f64; 3], [f64; 3])> {
        Some(self.bounds())
    }
}

impl EntityProjection for Cushion {
    #[inline(always)]
    fn projected_id(&self) -> u32 {
        self.id
    }
    #[inline(always)]
    fn projected_kind(&self) -> Cow<'_, str> {
        Cow::Borrowed("minecraft:cushion")
    }
    #[inline(always)]
    fn projected_position(&self) -> Vec3 {
        self.position
    }
    #[inline(always)]
    fn projected_yaw(&self) -> Option<f32> {
        Some(self.yaw)
    }
    #[inline(always)]
    fn projected_bounds(&self) -> Option<Aabb> {
        Some(extents_aabb(self.bounds()))
    }
    #[inline(always)]
    fn projected_raw_bounds(&self) -> Option<([f64; 3], [f64; 3])> {
        Some(self.bounds())
    }
}

impl EntityProjection for Bolt {
    #[inline(always)]
    fn projected_id(&self) -> u32 {
        self.id
    }
    #[inline(always)]
    fn projected_kind(&self) -> Cow<'_, str> {
        Cow::Borrowed("minecraft:lightning_bolt")
    }
    #[inline(always)]
    fn projected_position(&self) -> Vec3 {
        Vec3::new(self.at[0], self.at[1], self.at[2])
    }
    #[inline(always)]
    fn projected_velocity(&self) -> Option<Vec3> {
        Some(self.velocity)
    }
    #[inline(always)]
    fn projected_rotation(&self) -> Option<[f32; 2]> {
        Some(self.rotation)
    }
    #[inline(always)]
    fn projected_bounds(&self) -> Option<Aabb> {
        let p = self.projected_position();
        Some(Aabb::new(p, p))
    }
    #[inline(always)]
    fn projected_on_ground(&self) -> Option<bool> {
        Some(self.on_ground)
    }
    #[inline(always)]
    fn projected_body_fire_ticks(&self) -> Option<i32> {
        Some(self.fire_ticks)
    }
    #[inline(always)]
    fn projected_air_supply(&self) -> Option<i32> {
        Some(self.air)
    }
}

impl EntityProjection for Dragon {
    #[inline(always)]
    fn projected_id(&self) -> u32 {
        self.id
    }
    #[inline(always)]
    fn projected_kind(&self) -> Cow<'_, str> {
        Cow::Borrowed("minecraft:ender_dragon")
    }
    #[inline(always)]
    fn projected_position(&self) -> Vec3 {
        self.position
    }
    #[inline(always)]
    fn projected_velocity(&self) -> Option<Vec3> {
        Some(self.velocity)
    }
    #[inline(always)]
    fn projected_rotation(&self) -> Option<[f32; 2]> {
        Some([self.yaw, self.pitch])
    }
    #[inline(always)]
    fn projected_bounds(&self) -> Option<Aabb> {
        Some(self.body_bounds())
    }
}

impl EntityProjection for DragonPartRef<'_> {
    #[inline(always)]
    fn projected_id(&self) -> u32 {
        self.id
    }
    #[inline(always)]
    fn projected_kind(&self) -> Cow<'_, str> {
        Cow::Borrowed("minecraft:ender_dragon")
    }
    #[inline(always)]
    fn projected_position(&self) -> Vec3 {
        self.dragon.parts[self.part].0
    }
    #[inline(always)]
    fn projected_bounds(&self) -> Option<Aabb> {
        Some(self.dragon.part_geometry(self.part).1)
    }
}

impl EntityProjection for Crystal {
    #[inline(always)]
    fn projected_id(&self) -> u32 {
        self.id
    }
    #[inline(always)]
    fn projected_kind(&self) -> Cow<'_, str> {
        Cow::Borrowed("minecraft:end_crystal")
    }
    #[inline(always)]
    fn projected_position(&self) -> Vec3 {
        self.position
    }
    #[inline(always)]
    fn projected_rotation(&self) -> Option<[f32; 2]> {
        Some(self.rotation)
    }
    #[inline(always)]
    fn projected_bounds(&self) -> Option<Aabb> {
        Some(self.bounds())
    }
    #[inline(always)]
    fn projected_on_ground(&self) -> Option<bool> {
        Some(self.fall.0)
    }
    #[inline(always)]
    fn projected_body_fire_ticks(&self) -> Option<i32> {
        Some(self.fire)
    }
}

impl EntityProjection for Breath {
    #[inline(always)]
    fn projected_id(&self) -> u32 {
        self.id
    }
    #[inline(always)]
    fn projected_kind(&self) -> Cow<'_, str> {
        Cow::Borrowed("minecraft:area_effect_cloud")
    }
    #[inline(always)]
    fn projected_position(&self) -> Vec3 {
        Vec3::new(self.at[0], self.at[1], self.at[2])
    }
    #[inline(always)]
    fn projected_velocity(&self) -> Option<Vec3> {
        Some(Vec3::new(self.motion[0], self.motion[1], self.motion[2]))
    }
    #[inline(always)]
    fn projected_body_fire_ticks(&self) -> Option<i32> {
        Some(self.fire_ticks)
    }
}

impl EntityProjection for Hook {
    #[inline(always)]
    fn projected_id(&self) -> u32 {
        self.id
    }
    #[inline(always)]
    fn projected_kind(&self) -> Cow<'_, str> {
        Cow::Borrowed("minecraft:fishing_bobber")
    }
    #[inline(always)]
    fn projected_position(&self) -> Vec3 {
        self.position
    }
    #[inline(always)]
    fn projected_velocity(&self) -> Option<Vec3> {
        Some(self.velocity)
    }
    #[inline(always)]
    fn projected_rotation(&self) -> Option<[f32; 2]> {
        Some([self.yaw, self.pitch])
    }
    #[inline(always)]
    fn projected_bounds(&self) -> Option<Aabb> {
        Some(self.bounds())
    }
    #[inline(always)]
    fn projected_on_ground(&self) -> Option<bool> {
        Some(self.ground.on_ground)
    }
    #[inline(always)]
    fn projected_body_fire_ticks(&self) -> Option<i32> {
        Some(self.base.fire_ticks)
    }
    #[inline(always)]
    fn projected_body_uuid(&self) -> Option<[i32; 4]> {
        Some(self.uuid)
    }
}

impl EntityProjection for OminousItemSpawner {
    #[inline(always)]
    fn projected_id(&self) -> u32 {
        self.id
    }
    #[inline(always)]
    fn projected_kind(&self) -> Cow<'_, str> {
        Cow::Borrowed("minecraft:ominous_item_spawner")
    }
    #[inline(always)]
    fn projected_position(&self) -> Vec3 {
        Vec3::new(self.position[0], self.position[1], self.position[2])
    }
    #[inline(always)]
    fn projected_rotation(&self) -> Option<[f32; 2]> {
        self.raw.as_deref().map(sunrise_sim::entity_nbt::rotation)
    }
    #[inline(always)]
    fn projected_body_fire_ticks(&self) -> Option<i32> {
        Some(self.fire_ticks)
    }
    #[inline(always)]
    fn projected_body_uuid(&self) -> Option<[i32; 4]> {
        Some(self.uuid)
    }
}

impl EntityProjection for Fangs {
    #[inline(always)]
    fn projected_id(&self) -> u32 {
        self.id
    }
    #[inline(always)]
    fn projected_kind(&self) -> Cow<'_, str> {
        Cow::Borrowed("minecraft:evoker_fangs")
    }
    #[inline(always)]
    fn projected_position(&self) -> Vec3 {
        Vec3::new(self.pos[0], self.pos[1], self.pos[2])
    }
    #[inline(always)]
    fn projected_velocity(&self) -> Option<Vec3> {
        Some(self.motion)
    }
    #[inline(always)]
    fn projected_yaw(&self) -> Option<f32> {
        Some(self.yaw)
    }
    #[inline(always)]
    fn projected_body_fire_ticks(&self) -> Option<i32> {
        Some(self.fire_ticks)
    }
}

impl EntityProjection for Knot {
    #[inline(always)]
    fn projected_id(&self) -> u32 {
        self.id
    }
    #[inline(always)]
    fn projected_kind(&self) -> Cow<'_, str> {
        Cow::Borrowed("minecraft:leash_knot")
    }
    #[inline(always)]
    fn projected_position(&self) -> Vec3 {
        sunrise_sim::mobs::leash::knot_state(self.fence).position
    }
    #[inline(always)]
    fn projected_bounds(&self) -> Option<Aabb> {
        Some(knot_bounds(self.fence))
    }
    #[inline(always)]
    fn projected_body_fire_ticks(&self) -> Option<i32> {
        Some(self.fire_ticks)
    }
}

impl EntityProjection for EyeOfEnder {
    #[inline(always)]
    fn projected_id(&self) -> u32 {
        self.id
    }
    #[inline(always)]
    fn projected_kind(&self) -> Cow<'_, str> {
        Cow::Borrowed("minecraft:eye_of_ender")
    }
    #[inline(always)]
    fn projected_position(&self) -> Vec3 {
        Vec3::new(self.position.x, self.position.y, self.position.z)
    }
    #[inline(always)]
    fn projected_velocity(&self) -> Option<Vec3> {
        Some(Vec3::new(self.velocity.x, self.velocity.y, self.velocity.z))
    }
}

impl EntityProjection for ThrownPotion {
    #[inline(always)]
    fn projected_id(&self) -> u32 {
        self.id
    }
    #[inline(always)]
    fn projected_kind(&self) -> Cow<'_, str> {
        Cow::Borrowed(thrown_kind(&self.stack))
    }
    #[inline(always)]
    fn projected_position(&self) -> Vec3 {
        self.position
    }
    #[inline(always)]
    fn projected_velocity(&self) -> Option<Vec3> {
        Some(self.velocity)
    }
}

impl EntityProjection for OtherEntity {
    #[inline(always)]
    fn projected_id(&self) -> u32 {
        self.id
    }
    #[inline(always)]
    fn projected_kind(&self) -> Cow<'_, str> {
        Cow::Borrowed(self.kind.as_str())
    }
    #[inline(always)]
    fn projected_position(&self) -> Vec3 {
        self.position
    }
    #[inline(always)]
    fn projected_velocity(&self) -> Option<Vec3> {
        self.eye.as_ref().map(|e| e.motion)
    }
    #[inline(always)]
    fn projected_rotation(&self) -> Option<[f32; 2]> {
        Some([self.yaw, self.pitch])
    }
    #[inline(always)]
    fn projected_bounds(&self) -> Option<Aabb> {
        self.display_marker_or_interaction_bounds()
    }
    #[inline(always)]
    fn projected_body_fire_ticks(&self) -> Option<i32> {
        self.eye.as_ref().map(|e| e.fire_ticks)
    }
}

impl EntityProjection for PlayerMotion {
    #[inline(always)]
    fn projected_id(&self) -> u32 {
        self.entity_id as u32
    }
    #[inline(always)]
    fn projected_kind(&self) -> Cow<'_, str> {
        Cow::Borrowed("minecraft:player")
    }
    #[inline(always)]
    fn projected_position(&self) -> Vec3 {
        self.position
    }
    #[inline(always)]
    fn projected_velocity(&self) -> Option<Vec3> {
        Some(self.velocity)
    }
    #[inline(always)]
    fn projected_rotation(&self) -> Option<[f32; 2]> {
        Some([self.yaw, self.pitch])
    }
    #[inline(always)]
    fn projected_bounds(&self) -> Option<Aabb> {
        Some(self.bounds())
    }
    #[inline(always)]
    fn projected_on_ground(&self) -> Option<bool> {
        Some(self.ground.on_ground)
    }
    #[inline(always)]
    fn projected_body_fire_ticks(&self) -> Option<i32> {
        Some(self.fire_ticks)
    }
    #[inline(always)]
    fn projected_air_supply(&self) -> Option<i32> {
        Some(self.air)
    }
}

impl EntityProjection for Guest {
    #[inline(always)]
    fn projected_id(&self) -> u32 {
        self.entity_id as u32
    }
    #[inline(always)]
    fn projected_kind(&self) -> Cow<'_, str> {
        Cow::Borrowed("minecraft:player")
    }
    #[inline(always)]
    fn projected_position(&self) -> Vec3 {
        Vec3::new(self.position[0], self.position[1], self.position[2])
    }
    #[inline(always)]
    fn projected_velocity(&self) -> Option<Vec3> {
        self.motion.as_ref().map(|m| m.velocity)
    }
    #[inline(always)]
    fn projected_rotation(&self) -> Option<[f32; 2]> {
        Some([self.yaw, self.pitch])
    }
    #[inline(always)]
    fn projected_bounds(&self) -> Option<Aabb> {
        Some(self.bounds())
    }
    #[inline(always)]
    fn projected_on_ground(&self) -> Option<bool> {
        Some(self.on_ground)
    }
    #[inline(always)]
    fn projected_body_fire_ticks(&self) -> Option<i32> {
        self.motion.as_ref().map(|m| m.fire_ticks)
    }
    #[inline(always)]
    fn projected_air_supply(&self) -> Option<i32> {
        self.motion.as_ref().map(|m| m.air)
    }
    #[inline(always)]
    fn projected_body_uuid(&self) -> Option<[i32; 4]> {
        Some(sunrise_sim::lan::playerdata::uuid_ints(self.uuid))
    }
}

impl EntityProjection for OffLevel {
    #[inline(always)]
    fn projected_id(&self) -> u32 {
        match self {
            Self::Mob(m) => m.projected_id(),
            Self::Boat(b) => b.projected_id(),
            Self::Cart(c) => c.projected_id(),
        }
    }
    #[inline(always)]
    fn projected_kind(&self) -> Cow<'_, str> {
        match self {
            Self::Mob(m) => m.projected_kind(),
            Self::Boat(b) => b.projected_kind(),
            Self::Cart(c) => c.projected_kind(),
        }
    }
    #[inline(always)]
    fn projected_position(&self) -> Vec3 {
        match self {
            Self::Mob(m) => m.projected_position(),
            Self::Boat(b) => b.projected_position(),
            Self::Cart(c) => c.projected_position(),
        }
    }
    #[inline(always)]
    fn projected_velocity(&self) -> Option<Vec3> {
        match self {
            Self::Mob(m) => m.projected_velocity(),
            Self::Boat(b) => b.projected_velocity(),
            Self::Cart(c) => c.projected_velocity(),
        }
    }
    #[inline(always)]
    fn projected_rotation(&self) -> Option<[f32; 2]> {
        match self {
            Self::Mob(m) => m.projected_rotation(),
            Self::Boat(b) => b.projected_rotation(),
            Self::Cart(c) => c.projected_rotation(),
        }
    }
    #[inline(always)]
    fn projected_bounds(&self) -> Option<Aabb> {
        match self {
            Self::Mob(m) => m.projected_bounds(),
            Self::Boat(b) => b.projected_bounds(),
            Self::Cart(c) => c.projected_bounds(),
        }
    }
    #[inline(always)]
    fn projected_on_ground(&self) -> Option<bool> {
        match self {
            Self::Mob(m) => m.projected_on_ground(),
            Self::Boat(b) => b.projected_on_ground(),
            Self::Cart(c) => c.projected_on_ground(),
        }
    }
    #[inline(always)]
    fn projected_body_fire_ticks(&self) -> Option<i32> {
        match self {
            Self::Mob(m) => m.projected_body_fire_ticks(),
            Self::Boat(b) => b.projected_body_fire_ticks(),
            Self::Cart(c) => c.projected_body_fire_ticks(),
        }
    }
    #[inline(always)]
    fn projected_air_supply(&self) -> Option<i32> {
        match self {
            Self::Mob(m) => m.projected_air_supply(),
            Self::Boat(b) => b.projected_air_supply(),
            Self::Cart(c) => c.projected_air_supply(),
        }
    }
    #[inline(always)]
    fn projected_body_removed(&self) -> Option<bool> {
        match self {
            Self::Mob(m) => m.projected_body_removed(),
            Self::Boat(b) => b.projected_body_removed(),
            Self::Cart(c) => c.projected_body_removed(),
        }
    }
    #[inline(always)]
    fn projected_body_uuid(&self) -> Option<[i32; 4]> {
        match self {
            Self::Mob(m) => m.projected_body_uuid(),
            Self::Boat(b) => b.projected_body_uuid(),
            Self::Cart(c) => c.projected_body_uuid(),
        }
    }
}

/// Local-body lookup on `CommonGame`; side-specific authority and replica fallbacks stay with callers.
#[inline(always)]
pub fn with_common_entity_projection<R>(
    common: &CommonGame<'_>,
    target: Target,
    visit: impl FnOnce(&dyn EntityProjection) -> R,
) -> Option<R> {
    match target {
        Target::Player => common.player.as_ref().map(|p| visit(p)),
        Target::Mob(id) => common.mobs.iter().find(|e| e.id == id).map(|e| visit(e)),
        Target::Item(id) => common.dropped.iter().find(|e| e.id == id).map(|e| visit(e)),
        Target::Orb(id) => common.orbs.iter().find(|e| e.id == id).map(|e| visit(e)),
        Target::Projectile(id) => common
            .creatures
            .projectiles
            .iter()
            .find(|e| e.id == id)
            .map(|e| visit(e)),
        Target::Stand(id) => common
            .armor_stands
            .iter()
            .find(|e| e.id == id)
            .map(|e| visit(e)),
        Target::Frame(id) => common
            .item_frames
            .iter()
            .find(|e| e.id == id)
            .map(|e| visit(e)),
        Target::Painting(id) => common
            .paintings
            .iter()
            .find(|e| e.id == id)
            .map(|e| visit(e)),
        Target::Boat(id) => common.boats.iter().find(|e| e.id == id).map(|e| visit(e)),
        Target::Cart(id) => common.carts.iter().find(|e| e.id == id).map(|e| visit(e)),
        Target::Crystal(id) => common
            .crystals
            .iter()
            .find(|e| e.id == id)
            .map(|e| visit(e)),
        Target::Dragon(id) => common
            .dragon
            .as_ref()
            .filter(|d| d.id == id)
            .map(|d| visit(d)),
        Target::Cushion(id) => common
            .cushions
            .iter()
            .find(|e| e.id == id)
            .map(|e| visit(e)),
        Target::Tnt(id) => common.tnts.iter().find(|e| e.id == id).map(|e| visit(e)),
        Target::Falling(id) => common.falling.iter().find(|e| e.id == id).map(|e| visit(e)),
        Target::Rocket(id) => common.rockets.iter().find(|e| e.id == id).map(|e| visit(e)),
        Target::Cloud(id) => common
            .breath_clouds
            .iter()
            .find(|e| e.id == id)
            .map(|e| visit(e)),
        Target::Bolt(id) => common.bolts.iter().find(|e| e.id == id).map(|e| visit(e)),
        Target::Hook(id) => common
            .hook
            .as_ref()
            .filter(|h| h.id() == id)
            .map(|h| visit(h)),
        Target::Trident(id) => common
            .tridents
            .iter()
            .find(|e| e.id == id)
            .map(|e| visit(e)),
        Target::Fangs(id) => common
            .fangs
            .list
            .iter()
            .find(|e| e.id == id)
            .map(|e| visit(e)),
        Target::Spawner(id) => common
            .ominous_spawners
            .iter()
            .find(|e| e.id == id)
            .map(|e| visit(e)),
        Target::Knot(id) => common
            .resident_entities
            .knots
            .iter()
            .find(|e| e.id == id)
            .map(|e| visit(e)),
        Target::General(id) => common.resident_entities.general.row(id).map(|e| visit(e)),
        Target::Guest(_) | Target::Other(_) | Target::DragonPart(_) => None,
    }
}

/// Context-aware read-only saved UUID lookup on `CommonGame` combining `EntityMemory` and body UUIDs.
#[inline(always)]
pub fn common_saved_uuid(common: &CommonGame<'_>, target: Target) -> Option<[i32; 4]> {
    match target {
        Target::Player => common.level.player_uuid,
        Target::Guest(_) | Target::Other(_) | Target::DragonPart(_) => None,
        other => {
            let id = crate::game::commands_common::target_id(other)?;
            common.persist.uuid_of(id).or_else(|| {
                with_common_entity_projection(common, other, |entity| entity.projected_body_uuid())
                    .flatten()
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::minecarts::CartKind;
    use std::sync::Arc;
    use sunrise_core::{nbt::Tag, position::BlockPos, random::LegacyRandom};
    use sunrise_items::items::ItemStack;
    use sunrise_sim::{entities::arrows::ArrowState, projectiles::ProjectileKind};

    fn test_item_entity() -> ItemEntity {
        let p = Vec3::new(1.5, 64.0, -2.5);
        ItemEntity {
            size_answer: None,
            attachment_lifetime: Default::default(),
            fall_distance: 0.0,
            random: LegacyRandom::new(0),
            rotation: [45.0, -10.0],
            join: Default::default(),
            health: 5,
            fire_ticks: 17,
            id: 101,
            stack: ItemStack::new("minecraft:stone", 3),
            thrower: None,
            target: None,
            position: p,
            previous: p,
            velocity: Vec3::new(0.1, 0.2, -0.3),
            ground: sunrise_world::ground_support::GroundState {
                on_ground: true,
                ..Default::default()
            },
            age: 0,
            stack_revision: 0,
            lifespan: ItemEntity::LIFETIME,
            pickup_delay: 0,
            bob_offset: 0.0,
            ticks: 0,
            needs_sync: false,
            stuck: Vec3::ZERO,
            was_touching_water: false,
            no_gravity: false,
            vertical_collision: false,
        }
    }

    #[test]
    fn item_entity_projection_matches_body() {
        let item = test_item_entity();
        assert_eq!(item.projected_id(), item.id);
        assert_eq!(item.projected_kind(), "minecraft:item");
        assert_eq!(item.projected_position(), item.position);
        assert_eq!(item.projected_velocity(), Some(item.velocity));
        assert_eq!(item.projected_rotation(), Some(item.rotation));
        assert_eq!(item.projected_bounds(), Some(item.bounds()));
        assert_eq!(item.projected_on_ground(), Some(item.ground.on_ground));
        assert_eq!(item.projected_body_fire_ticks(), Some(item.fire_ticks));
        assert_eq!(item.projected_air_supply(), None);
        assert_eq!(item.projected_body_removed(), None);
        assert_eq!(item.projected_body_uuid(), None);
    }

    #[test]
    fn experience_orb_projection_matches_body() {
        let mut orb = ExperienceOrb::new(
            102,
            Vec3::new(2.0, 65.0, 3.0),
            Vec3::new(0.2, -0.1, 0.05),
            7,
        );
        orb.rotation = [12.5, -5.0];
        orb.ground.on_ground = true;
        orb.fire_ticks = 9;
        orb.air = 240;
        assert_eq!(orb.projected_id(), orb.id);
        assert_eq!(orb.projected_kind(), "minecraft:experience_orb");
        assert_eq!(orb.projected_position(), orb.position);
        assert_eq!(orb.projected_velocity(), Some(orb.velocity));
        assert_eq!(orb.projected_rotation(), Some(orb.rotation));
        assert_eq!(orb.projected_bounds(), Some(orb.bounds()));
        assert_eq!(orb.projected_on_ground(), Some(orb.ground.on_ground));
        assert_eq!(orb.projected_body_fire_ticks(), Some(orb.fire_ticks));
        assert_eq!(orb.projected_air_supply(), Some(orb.air));
        assert_eq!(orb.projected_body_removed(), None);
        assert_eq!(orb.projected_body_uuid(), None);
    }

    #[test]
    fn boat_projection_matches_body() {
        let mut boat = Boat::new(103, Arc::from("birch"), Vec3::new(3.0, 63.0, -4.0), 90.0);
        boat.pitch = 4.0;
        boat.velocity = Vec3::new(0.25, 0.0, -0.25);
        boat.ground.on_ground = true;
        boat.fire_ticks = 14;
        assert_eq!(boat.projected_id(), boat.id);
        assert_eq!(boat.projected_kind(), boat.entity_type_name());
        assert_eq!(boat.projected_position(), boat.position);
        assert_eq!(boat.projected_velocity(), Some(boat.velocity));
        assert_eq!(boat.projected_rotation(), Some([boat.yaw, boat.pitch]));
        assert_eq!(boat.projected_bounds(), Some(boat.bounds()));
        assert_eq!(boat.projected_on_ground(), Some(boat.ground.on_ground));
        assert_eq!(boat.projected_body_fire_ticks(), Some(boat.fire_ticks));
        assert_eq!(boat.projected_air_supply(), None);
        assert_eq!(boat.projected_body_removed(), None);
        assert_eq!(boat.projected_body_uuid(), None);
    }

    #[test]
    fn minecart_projection_matches_body() {
        let mut cart = Minecart::new(104, CartKind::Chest, Vec3::new(4.5, 64.0, 5.5));
        cart.yaw = 180.0;
        cart.pitch = -3.0;
        cart.velocity = Vec3::new(0.4, 0.0, 0.0);
        cart.ground.on_ground = true;
        assert_eq!(cart.projected_id(), cart.id);
        assert_eq!(cart.projected_kind(), cart.kind.id());
        assert_eq!(cart.projected_position(), cart.position);
        assert_eq!(cart.projected_velocity(), Some(cart.velocity));
        assert_eq!(cart.projected_rotation(), Some([cart.yaw, cart.pitch]));
        assert_eq!(cart.projected_bounds(), Some(cart.bounds()));
        assert_eq!(cart.projected_on_ground(), Some(cart.ground.on_ground));
        assert_eq!(cart.projected_body_fire_ticks(), None);
        assert_eq!(cart.projected_air_supply(), None);
        assert_eq!(cart.projected_body_removed(), None);
        assert_eq!(cart.projected_body_uuid(), None);
    }

    #[test]
    fn armor_stand_projection_matches_body() {
        let mut stand = ArmorStand::new(105, Vec3::new(-1.0, 64.0, 2.0), 135.0);
        stand.pitch = 6.0;
        stand.velocity = Vec3::new(0.0, -0.08, 0.0);
        stand.ground.on_ground = true;
        stand.fire_ticks = 22;
        stand.air = 180;
        assert_eq!(stand.projected_id(), stand.id);
        assert_eq!(stand.projected_kind(), "minecraft:armor_stand");
        assert_eq!(stand.projected_position(), stand.position);
        assert_eq!(stand.projected_velocity(), Some(stand.velocity));
        assert_eq!(stand.projected_rotation(), Some([stand.yaw, stand.pitch]));
        assert_eq!(stand.projected_bounds(), Some(stand.bounds()));
        assert_eq!(stand.projected_on_ground(), Some(stand.ground.on_ground));
        assert_eq!(stand.projected_body_fire_ticks(), Some(stand.fire_ticks));
        assert_eq!(stand.projected_air_supply(), Some(stand.air));
        assert_eq!(stand.projected_body_removed(), None);
        assert_eq!(stand.projected_body_uuid(), None);
    }

    #[test]
    fn primed_tnt_projection_matches_body() {
        let pos = Vec3::new(5.5, 66.0, -6.5);
        let tnt = PrimedTnt {
            attachment_lifetime: Default::default(),
            rotation: [30.0, 5.0],
            join: Default::default(),
            id: 106,
            position: pos,
            previous: pos,
            velocity: Vec3::new(0.02, 0.2, -0.02),
            fuse: 80,
            ticks: 1,
            ground: sunrise_world::ground_support::GroundState {
                on_ground: true,
                ..Default::default()
            },
            fall_distance: 0.0,
            stuck: Vec3::ZERO,
            in_water: false,
            owner: None,
            guest: None,
            no_gravity: false,
            power: 4.0,
            used_portal: false,
            block_state: None,
            size_answer: None,
        };
        assert_eq!(tnt.projected_id(), tnt.id);
        assert_eq!(tnt.projected_kind(), "minecraft:tnt");
        assert_eq!(tnt.projected_position(), tnt.position);
        assert_eq!(tnt.projected_velocity(), Some(tnt.velocity));
        assert_eq!(tnt.projected_rotation(), Some(tnt.rotation));
        assert_eq!(tnt.projected_bounds(), Some(tnt.bounds()));
        assert_eq!(tnt.projected_on_ground(), Some(tnt.ground.on_ground));
        assert_eq!(tnt.projected_body_fire_ticks(), None);
        assert_eq!(tnt.projected_air_supply(), None);
        assert_eq!(tnt.projected_body_removed(), None);
        assert_eq!(tnt.projected_body_uuid(), None);
    }

    #[test]
    fn falling_block_projection_matches_body() {
        let pos = Vec3::new(6.5, 70.0, 7.5);
        let uuid = [11, 22, 33, 44];
        let falling = FallingBlock {
            attachment_lifetime: Default::default(),
            rotation: [15.0, -20.0],
            join: Default::default(),
            id: 107,
            state: sunrise_world::registry::BlockStateId(0),
            blocks_building: true,
            start_pos: BlockPos::new(6, 70, 7),
            position: pos,
            previous: pos,
            velocity: Vec3::new(0.0, -0.04, 0.0),
            age: 3,
            no_gravity: false,
            portal: Default::default(),
            drop_item: true,
            fall_distance: 1.5,
            ground: sunrise_world::ground_support::GroundState {
                on_ground: true,
                ..Default::default()
            },
            fire_ticks: 40,
            sync_velocity: false,
            stuck: Vec3::ZERO,
            damage: sunrise_sim::falling_block::Damage {
                hurt_entities: false,
                cancel_drop: false,
                per_distance: 0.0,
                max: 0,
            },
            saved: Some(Box::new((uuid, Tag::Compound(Default::default())))),
            size_answer: None,
        };
        assert_eq!(falling.projected_id(), falling.id);
        assert_eq!(falling.projected_kind(), "minecraft:falling_block");
        assert_eq!(falling.projected_position(), falling.position);
        assert_eq!(falling.projected_velocity(), Some(falling.velocity));
        assert_eq!(falling.projected_rotation(), Some(falling.rotation));
        assert_eq!(falling.projected_bounds(), Some(falling.bounds()));
        assert_eq!(
            falling.projected_on_ground(),
            Some(falling.ground.on_ground)
        );
        assert_eq!(
            falling.projected_body_fire_ticks(),
            Some(falling.fire_ticks)
        );
        assert_eq!(falling.projected_air_supply(), None);
        assert_eq!(falling.projected_body_removed(), None);
        assert_eq!(falling.projected_body_uuid(), Some(uuid));
    }

    #[test]
    fn projectile_projection_matches_body() {
        let mut shot = Projectile::new(ProjectileKind::Snowball, Vec3::new(7.0, 65.0, 8.0), None);
        shot.id = 108;
        shot.velocity = Vec3::new(0.5, 0.1, -0.4);
        shot.yaw = 25.0;
        shot.pitch = -12.0;
        shot.ground.on_ground = true;
        shot.fire_ticks = 33;
        shot.air_supply = 210;
        shot.destroyed = true;
        shot.uuid = Some([1, 2, 3, 4]);
        assert_eq!(shot.projected_id(), shot.id);
        assert_eq!(shot.projected_kind(), shot.entity_type());
        assert_eq!(shot.projected_position(), shot.position);
        assert_eq!(shot.projected_velocity(), Some(shot.velocity));
        assert_eq!(shot.projected_rotation(), Some([shot.yaw, shot.pitch]));
        assert_eq!(shot.projected_bounds(), Some(shot.bounds()));
        assert_eq!(shot.projected_on_ground(), Some(shot.ground.on_ground));
        assert_eq!(shot.projected_body_fire_ticks(), Some(shot.fire_ticks));
        assert_eq!(shot.projected_air_supply(), Some(shot.air_supply));
        assert_eq!(shot.projected_body_removed(), Some(shot.destroyed));
        assert_eq!(shot.projected_body_uuid(), shot.uuid);

        let mut arrow_state = ArrowState::new();
        arrow_state.fire_ticks = 99;
        arrow_state.removed = true;
        shot.destroyed = false;
        shot.arrow = Some(Box::new(arrow_state));
        assert_eq!(shot.projected_body_fire_ticks(), Some(99));
        assert_eq!(shot.projected_body_removed(), Some(true));
    }

    #[test]
    fn rocket_projection_matches_body() {
        let mut rocket = Rocket::remote(109, Vec3::new(8.0, 72.0, 9.0), Vec3::new(0.1, 0.6, 0.2));
        rocket.rotation = [60.0, -30.0];
        rocket.ground.on_ground = true;
        assert_eq!(rocket.projected_id(), rocket.id);
        assert_eq!(rocket.projected_kind(), "minecraft:firework_rocket");
        assert_eq!(rocket.projected_position(), rocket.position);
        assert_eq!(rocket.projected_velocity(), Some(rocket.velocity));
        assert_eq!(rocket.projected_rotation(), Some(rocket.rotation));
        assert_eq!(rocket.projected_bounds(), Some(rocket.bounds()));
        assert_eq!(rocket.projected_on_ground(), Some(rocket.ground.on_ground));
        assert_eq!(rocket.projected_body_fire_ticks(), None);
        assert_eq!(rocket.projected_air_supply(), None);
        assert_eq!(rocket.projected_body_removed(), None);
        assert_eq!(rocket.projected_body_uuid(), None);
    }

    #[test]
    fn thrown_trident_projection_matches_body() {
        let mut trident = ThrownTrident::remote(110, Vec3::new(9.0, 68.0, -1.0), 42.0, -18.0);
        trident.velocity = Vec3::new(-0.3, 0.2, 0.4);
        trident.ground.on_ground = true;
        trident.fire_ticks = 55;
        assert_eq!(trident.projected_id(), trident.id);
        assert_eq!(trident.projected_kind(), "minecraft:trident");
        assert_eq!(trident.projected_position(), trident.position);
        assert_eq!(trident.projected_velocity(), Some(trident.velocity));
        assert_eq!(
            trident.projected_rotation(),
            Some([trident.yaw, trident.pitch])
        );
        assert_eq!(trident.projected_bounds(), Some(Entity::bounds(&trident)));
        assert_eq!(
            trident.projected_on_ground(),
            Some(trident.ground.on_ground)
        );
        assert_eq!(
            trident.projected_body_fire_ticks(),
            Some(trident.fire_ticks)
        );
        assert_eq!(trident.projected_air_supply(), None);
        assert_eq!(trident.projected_body_removed(), None);
        assert_eq!(trident.projected_body_uuid(), None);
    }

    #[test]
    fn general_row_projection_matches_body() {
        use crate::game::remote::general::tests::Declare;
        let entries = || vec![mod_platform::ModEntry::new("mymod", Box::new(Declare))];
        mod_platform::testing::with_mods(entries().into(), || {
            mod_platform::__private::part_tests::install_parts_fixture(&entries()).unwrap();
            let row = GeneralRow {
                id: 111,
                uuid: [5, 6, 7, 8],
                join: sunrise_sim::join_seq::JoinSeq::UNJOINED,
                type_slot: 0,
                position: Vec3::new(1.0, 64.0, 2.0),
                previous: Vec3::new(1.0, 64.0, 2.0),
                velocity: Vec3::new(0.1, 0.0, -0.1),
                rotation: [33.0, -11.0],
                previous_rotation: [33.0, -11.0],
                owner: None,
                fall_distance: 0.0,
                on_ground: true,
                tick_count: 4,
                no_physics: false,
                horizontal_collision: false,
                vertical_collision: false,
                vertical_collision_below: false,
                fluids: Default::default(),
                under_water: false,
                custom_name: None,
                custom_name_visible: false,
                base: crate::game::native_entity_base::EntityBase {
                    fire_ticks: 29,
                    ..Default::default()
                },
                payload_version: 1,
                payload_bytes: Vec::new(),
                mount: None,
                vehicle_order: 0,
                boarding_cooldown: 0,
                attachment_lifetime: Default::default(),
            };
            assert_eq!(row.projected_id(), row.id);
            assert_eq!(row.projected_kind(), Entity::native_type(&row));
            assert_eq!(row.projected_position(), row.position);
            assert_eq!(row.projected_velocity(), Some(row.velocity));
            assert_eq!(row.projected_rotation(), Some(row.rotation));
            assert_eq!(row.projected_bounds(), Some(Entity::bounds(&row)));
            assert_eq!(row.projected_on_ground(), Some(row.on_ground));
            assert_eq!(row.projected_body_fire_ticks(), Some(row.base.fire_ticks));
            assert_eq!(row.projected_air_supply(), None);
            assert_eq!(row.projected_body_removed(), None);
            assert_eq!(row.projected_body_uuid(), Some(row.uuid));
        });
    }

    #[test]
    fn painting_projection_matches_body() {
        let painting = Painting {
            attachment_lifetime: Default::default(),
            join: Default::default(),
            id: 201,
            pos: [10, 65, -3],
            facing: 2,
            variant: Arc::from("minecraft:kebab"),
            asset: None,
            width: 2,
            height: 1,
            check_interval: 0,
            ticks: 5,
        };
        let (lo, hi) = painting.bounds();
        assert_eq!(painting.projected_id(), painting.id);
        assert_eq!(painting.projected_kind(), "minecraft:painting");
        assert_eq!(
            painting.projected_position(),
            Vec3::new(
                (lo[0] + hi[0]) / 2.0,
                (lo[1] + hi[1]) / 2.0,
                (lo[2] + hi[2]) / 2.0,
            )
        );
        assert_eq!(painting.projected_velocity(), None);
        assert_eq!(painting.projected_rotation(), None);
        assert_eq!(
            painting.projected_yaw(),
            Some(f32::from(painting.facing) * 90.0)
        );
        assert_eq!(painting.projected_bounds(), Some(extents_aabb((lo, hi))));
        assert_eq!(painting.projected_raw_bounds(), Some((lo, hi)));
        assert_eq!(painting.projected_on_ground(), None);
        assert_eq!(painting.projected_body_fire_ticks(), None);
        assert_eq!(painting.projected_air_supply(), None);
        assert_eq!(painting.projected_body_removed(), None);
        assert_eq!(painting.projected_body_uuid(), None);

        let neg = Painting {
            width: -2,
            ..painting
        };
        let (neg_lo, neg_hi) = neg.bounds();
        assert!(neg_lo[0] > neg_hi[0]);
        assert_eq!(neg.projected_raw_bounds(), Some((neg_lo, neg_hi)));
        let normalized = neg.projected_bounds().unwrap();
        assert_eq!(normalized.min.x, neg_hi[0]);
        assert_eq!(normalized.max.x, neg_lo[0]);
    }

    #[test]
    fn item_frame_projection_matches_body() {
        for glow in [false, true] {
            let frame = ItemFrame {
                attachment_lifetime: Default::default(),
                join: Default::default(),
                id: 202,
                pos: [-4, 66, 8],
                facing: 4,
                glow,
                item: None,
                rotation: 2,
                drop_chance: 1.0,
                fixed: false,
                invisible: false,
                check_interval: 0,
                ticks: 1,
                custom_name: None,
                random: LegacyRandom::new(0),
            };
            let (lo, hi) = frame.bounds();
            assert_eq!(frame.projected_id(), frame.id);
            assert_eq!(frame.projected_kind(), frame.kind());
            assert_eq!(
                frame.projected_position(),
                Vec3::new(
                    (lo[0] + hi[0]) / 2.0,
                    (lo[1] + hi[1]) / 2.0,
                    (lo[2] + hi[2]) / 2.0,
                )
            );
            assert_eq!(frame.projected_velocity(), None);
            assert_eq!(frame.projected_rotation(), Some(frame.entity_rotation()));
            assert_eq!(frame.projected_bounds(), Some(extents_aabb((lo, hi))));
            assert_eq!(frame.projected_raw_bounds(), Some((lo, hi)));
            assert_eq!(frame.projected_on_ground(), None);
            assert_eq!(frame.projected_body_fire_ticks(), None);
            assert_eq!(frame.projected_air_supply(), None);
            assert_eq!(frame.projected_body_removed(), None);
            assert_eq!(frame.projected_body_uuid(), None);
        }
    }

    #[test]
    fn cushion_projection_matches_body() {
        let cushion = Cushion {
            attachment_lifetime: Default::default(),
            join: Default::default(),
            id: 203,
            position: Vec3::new(2.5, 64.0, 3.5),
            pos: [2, 64, 3],
            yaw: 90.0,
            color: 14,
            check_interval: 0,
            custom_name: None,
        };
        assert_eq!(cushion.projected_id(), cushion.id);
        assert_eq!(cushion.projected_kind(), "minecraft:cushion");
        assert_eq!(cushion.projected_position(), cushion.position);
        assert_eq!(cushion.projected_velocity(), None);
        assert_eq!(cushion.projected_rotation(), None);
        assert_eq!(cushion.projected_yaw(), Some(cushion.yaw));
        assert_eq!(
            cushion.projected_bounds(),
            Some(extents_aabb(cushion.bounds()))
        );
        assert_eq!(cushion.projected_raw_bounds(), Some(cushion.bounds()));
        assert_eq!(cushion.projected_on_ground(), None);
        assert_eq!(cushion.projected_body_fire_ticks(), None);
        assert_eq!(cushion.projected_air_supply(), None);
        assert_eq!(cushion.projected_body_removed(), None);
        assert_eq!(cushion.projected_body_uuid(), None);
    }

    #[test]
    fn bolt_projection_matches_body() {
        let (mut bolt, _) = Bolt::new(204, [12.0, 70.0, -9.0], false);
        bolt.velocity = Vec3::new(0.0, -0.5, 0.0);
        bolt.rotation = [10.0, 20.0];
        bolt.on_ground = true;
        bolt.fire_ticks = 60;
        bolt.air = 280;
        let pos = Vec3::new(bolt.at[0], bolt.at[1], bolt.at[2]);
        assert_eq!(bolt.projected_id(), bolt.id);
        assert_eq!(bolt.projected_kind(), "minecraft:lightning_bolt");
        assert_eq!(bolt.projected_position(), pos);
        assert_eq!(bolt.projected_velocity(), Some(bolt.velocity));
        assert_eq!(bolt.projected_rotation(), Some(bolt.rotation));
        assert_eq!(bolt.projected_bounds(), Some(Aabb::new(pos, pos)));
        assert_eq!(bolt.projected_on_ground(), Some(bolt.on_ground));
        assert_eq!(bolt.projected_body_fire_ticks(), Some(bolt.fire_ticks));
        assert_eq!(bolt.projected_air_supply(), Some(bolt.air));
        assert_eq!(bolt.projected_body_removed(), None);
        assert_eq!(bolt.projected_body_uuid(), None);
    }

    #[test]
    fn dragon_and_part_projection_match_body() {
        let mut dragon = Dragon::new(Vec3::new(0.0, 128.0, 0.0), 45.0, 7);
        dragon.id = 205;
        dragon.pitch = -15.0;
        dragon.velocity = Vec3::new(0.5, 0.1, -0.5);
        assert_eq!(dragon.projected_id(), dragon.id);
        assert_eq!(dragon.projected_kind(), "minecraft:ender_dragon");
        assert_eq!(dragon.projected_position(), dragon.position);
        assert_eq!(dragon.projected_velocity(), Some(dragon.velocity));
        assert_eq!(
            dragon.projected_rotation(),
            Some([dragon.yaw, dragon.pitch])
        );
        assert_eq!(dragon.projected_bounds(), Some(dragon.body_bounds()));
        assert_eq!(dragon.projected_on_ground(), None);
        assert_eq!(dragon.projected_body_fire_ticks(), None);
        assert_eq!(dragon.projected_air_supply(), None);
        assert_eq!(dragon.projected_body_removed(), None);
        assert_eq!(dragon.projected_body_uuid(), None);

        let part = DragonPartRef::new(206, &dragon, 2);
        assert_eq!(part.projected_id(), 206);
        assert_eq!(part.projected_kind(), "minecraft:ender_dragon");
        assert_eq!(part.projected_position(), dragon.parts[2].0);
        assert_eq!(part.projected_velocity(), None);
        assert_eq!(part.projected_rotation(), None);
        assert_eq!(part.projected_bounds(), Some(dragon.part_geometry(2).1));
        assert_eq!(part.projected_on_ground(), None);
        assert_eq!(part.projected_body_fire_ticks(), None);
        assert_eq!(part.projected_air_supply(), None);
        assert_eq!(part.projected_body_removed(), None);
        assert_eq!(part.projected_body_uuid(), None);
    }

    #[test]
    fn crystal_projection_matches_body() {
        let mut crystal = Crystal::new(207, Vec3::new(15.5, 80.0, -15.5), true, 4);
        crystal.rotation = [75.0, 0.0];
        crystal.fall = (true, 0.0);
        crystal.fire = 12;
        assert_eq!(crystal.projected_id(), crystal.id);
        assert_eq!(crystal.projected_kind(), "minecraft:end_crystal");
        assert_eq!(crystal.projected_position(), crystal.position);
        assert_eq!(crystal.projected_velocity(), None);
        assert_eq!(crystal.projected_rotation(), Some(crystal.rotation));
        assert_eq!(crystal.projected_bounds(), Some(crystal.bounds()));
        assert_eq!(crystal.projected_on_ground(), Some(crystal.fall.0));
        assert_eq!(crystal.projected_body_fire_ticks(), Some(crystal.fire));
        assert_eq!(crystal.projected_air_supply(), None);
        assert_eq!(crystal.projected_body_removed(), None);
        assert_eq!(crystal.projected_body_uuid(), None);
    }

    #[test]
    fn breath_projection_matches_body() {
        let breath = Breath {
            attachment_lifetime: Default::default(),
            join: Default::default(),
            id: 208,
            at: [1.0, 64.0, 2.0],
            motion: [0.05, 0.0, -0.05],
            fall_distance: 0.0,
            fire_ticks: 8,
            first_tick: false,
            radius: 3.0,
            age: 5,
            duration: 600,
            from_dragon: true,
            victims: Vec::new(),
            potion: None,
        };
        assert_eq!(breath.projected_id(), breath.id);
        assert_eq!(breath.projected_kind(), "minecraft:area_effect_cloud");
        assert_eq!(
            breath.projected_position(),
            Vec3::new(breath.at[0], breath.at[1], breath.at[2])
        );
        assert_eq!(
            breath.projected_velocity(),
            Some(Vec3::new(
                breath.motion[0],
                breath.motion[1],
                breath.motion[2]
            ))
        );
        assert_eq!(breath.projected_rotation(), None);
        assert_eq!(breath.projected_bounds(), None);
        assert_eq!(breath.projected_on_ground(), None);
        assert_eq!(breath.projected_body_fire_ticks(), Some(breath.fire_ticks));
        assert_eq!(breath.projected_air_supply(), None);
        assert_eq!(breath.projected_body_removed(), None);
        assert_eq!(breath.projected_body_uuid(), None);
    }

    #[test]
    fn hook_projection_matches_body() {
        let mut hook = Hook::remote(209, Vec3::new(3.5, 62.0, 4.5), [9, 8, 7, 6]);
        hook.velocity = Vec3::new(0.2, -0.1, 0.3);
        hook.yaw = 120.0;
        hook.pitch = -25.0;
        hook.ground.on_ground = true;
        hook.base.fire_ticks = 19;
        assert_eq!(hook.projected_id(), hook.id);
        assert_eq!(hook.projected_kind(), "minecraft:fishing_bobber");
        assert_eq!(hook.projected_position(), hook.position);
        assert_eq!(hook.projected_velocity(), Some(hook.velocity));
        assert_eq!(hook.projected_rotation(), Some([hook.yaw, hook.pitch]));
        assert_eq!(hook.projected_bounds(), Some(hook.bounds()));
        assert_eq!(hook.projected_on_ground(), Some(hook.ground.on_ground));
        assert_eq!(hook.projected_body_fire_ticks(), Some(hook.base.fire_ticks));
        assert_eq!(hook.projected_air_supply(), None);
        assert_eq!(hook.projected_body_removed(), None);
        assert_eq!(hook.projected_body_uuid(), Some(hook.uuid));
    }

    #[test]
    fn ominous_item_spawner_projection_matches_body() {
        let spawner = OminousItemSpawner {
            attachment_lifetime: Default::default(),
            join: Default::default(),
            id: 210,
            item: ItemStack::new("minecraft:diamond", 1),
            position: [5.0, 68.0, 6.0],
            spawn_item_after: 60,
            tick_count: 10,
            fire_ticks: 11,
            synced_fire: false,
            has_visual_fire: false,
            uuid: [21, 22, 23, 24],
            raw: None,
        };
        assert_eq!(spawner.projected_id(), spawner.id);
        assert_eq!(spawner.projected_kind(), "minecraft:ominous_item_spawner");
        assert_eq!(
            spawner.projected_position(),
            Vec3::new(
                spawner.position[0],
                spawner.position[1],
                spawner.position[2]
            )
        );
        assert_eq!(spawner.projected_velocity(), None);
        assert_eq!(spawner.projected_rotation(), None);
        assert_eq!(spawner.projected_bounds(), None);
        assert_eq!(spawner.projected_on_ground(), None);
        assert_eq!(
            spawner.projected_body_fire_ticks(),
            Some(spawner.fire_ticks)
        );
        assert_eq!(spawner.projected_air_supply(), None);
        assert_eq!(spawner.projected_body_removed(), None);
        assert_eq!(spawner.projected_body_uuid(), Some(spawner.uuid));
    }

    #[test]
    fn fangs_projection_matches_body() {
        let fangs = Fangs {
            attachment_lifetime: Default::default(),
            join: Default::default(),
            id: 211,
            pos: [-2.0, 64.0, 9.0],
            motion: Vec3::new(0.0, 0.05, 0.0),
            yaw: 15.0,
            warmup: 5,
            life: 22,
            owner: 1,
            started: true,
            fire_ticks: 13,
            synced_fire: false,
            has_visual_fire: false,
            random: LegacyRandom::new(0),
        };
        assert_eq!(fangs.projected_id(), fangs.id);
        assert_eq!(fangs.projected_kind(), "minecraft:evoker_fangs");
        assert_eq!(
            fangs.projected_position(),
            Vec3::new(fangs.pos[0], fangs.pos[1], fangs.pos[2])
        );
        assert_eq!(fangs.projected_velocity(), Some(fangs.motion));
        assert_eq!(fangs.projected_rotation(), None);
        assert_eq!(fangs.projected_yaw(), Some(fangs.yaw));
        assert_eq!(fangs.projected_bounds(), None);
        assert_eq!(fangs.projected_on_ground(), None);
        assert_eq!(fangs.projected_body_fire_ticks(), Some(fangs.fire_ticks));
        assert_eq!(fangs.projected_air_supply(), None);
        assert_eq!(fangs.projected_body_removed(), None);
        assert_eq!(fangs.projected_body_uuid(), None);
    }

    #[test]
    fn knot_projection_matches_body() {
        let knot = Knot {
            attachment_lifetime: Default::default(),
            id: 212,
            join: Default::default(),
            fence: [4, 65, -7],
            fire_ticks: 6,
            invulnerable: false,
            no_gravity: false,
            silent: false,
            glowing: false,
        };
        assert_eq!(knot.projected_id(), knot.id);
        assert_eq!(knot.projected_kind(), "minecraft:leash_knot");
        assert_eq!(
            knot.projected_position(),
            sunrise_sim::mobs::leash::knot_state(knot.fence).position
        );
        assert_eq!(knot.projected_velocity(), None);
        assert_eq!(knot.projected_rotation(), None);
        assert_eq!(knot.projected_bounds(), Some(knot_bounds(knot.fence)));
        assert_eq!(knot.projected_on_ground(), None);
        assert_eq!(knot.projected_body_fire_ticks(), Some(knot.fire_ticks));
        assert_eq!(knot.projected_air_supply(), None);
        assert_eq!(knot.projected_body_removed(), None);
        assert_eq!(knot.projected_body_uuid(), None);
    }

    #[test]
    fn eye_of_ender_projection_matches_body() {
        let eye = EyeOfEnder {
            id: 213,
            position: glam::DVec3::new(1.0, 75.0, 2.0),
            previous: glam::DVec3::new(0.5, 74.5, 1.5),
            velocity: glam::DVec3::new(0.2, 0.1, -0.3),
            target: glam::DVec3::new(100.0, 64.0, -50.0),
            life: 12,
            survive: true,
        };
        assert_eq!(eye.projected_id(), eye.id);
        assert_eq!(eye.projected_kind(), "minecraft:eye_of_ender");
        assert_eq!(
            eye.projected_position(),
            Vec3::new(eye.position.x, eye.position.y, eye.position.z)
        );
        assert_eq!(
            eye.projected_velocity(),
            Some(Vec3::new(eye.velocity.x, eye.velocity.y, eye.velocity.z))
        );
        assert_eq!(eye.projected_rotation(), None);
        assert_eq!(eye.projected_bounds(), None);
        assert_eq!(eye.projected_on_ground(), None);
        assert_eq!(eye.projected_body_fire_ticks(), None);
        assert_eq!(eye.projected_air_supply(), None);
        assert_eq!(eye.projected_body_removed(), None);
        assert_eq!(eye.projected_body_uuid(), None);
    }

    #[test]
    fn thrown_potion_projection_matches_body() {
        let pos = Vec3::new(-3.0, 67.0, 4.0);
        let potion = ThrownPotion {
            id: 214,
            stack: ItemStack::new("minecraft:lingering_potion", 1),
            previous: pos,
            position: pos,
            velocity: Vec3::new(0.3, 0.2, -0.1),
            age: 4,
            owner: None,
            thrower: None,
            guest: None,
            left_owner: true,
            last_deflected_by: None,
            random: LegacyRandom::new(0),
        };
        assert_eq!(potion.projected_id(), potion.id);
        assert_eq!(potion.projected_kind(), "minecraft:lingering_potion");
        assert_eq!(potion.projected_position(), potion.position);
        assert_eq!(potion.projected_velocity(), Some(potion.velocity));
        assert_eq!(potion.projected_rotation(), None);
        assert_eq!(potion.projected_bounds(), None);
        assert_eq!(potion.projected_on_ground(), None);
        assert_eq!(potion.projected_body_fire_ticks(), None);
        assert_eq!(potion.projected_air_supply(), None);
        assert_eq!(potion.projected_body_removed(), None);
        assert_eq!(potion.projected_body_uuid(), None);
    }

    #[test]
    fn other_entity_projection_matches_body() {
        let interaction = OtherEntity {
            attachment_lifetime: Default::default(),
            join: Default::default(),
            id: 215,
            kind: "minecraft:interaction".into(),
            position: Vec3::new(2.0, 65.0, -5.0),
            yaw: 40.0,
            pitch: -5.0,
            display: None,
            interaction: Some(crate::game::interactions_common::InteractionState {
                width: 1.5,
                height: 2.0,
                response: true,
                attack: None,
                interaction: None,
            }),
            knot_check: None,
            eye: None,
            size_answer: None,
        };
        assert_eq!(interaction.projected_id(), interaction.id);
        assert_eq!(interaction.projected_kind(), interaction.kind.as_str());
        assert_eq!(interaction.projected_position(), interaction.position);
        assert_eq!(interaction.projected_velocity(), None);
        assert_eq!(
            interaction.projected_rotation(),
            Some([interaction.yaw, interaction.pitch])
        );
        assert_eq!(
            interaction.projected_bounds(),
            interaction.display_marker_or_interaction_bounds()
        );
        assert_eq!(interaction.projected_on_ground(), None);
        assert_eq!(interaction.projected_body_fire_ticks(), None);
        assert_eq!(interaction.projected_air_supply(), None);
        assert_eq!(interaction.projected_body_removed(), None);
        assert_eq!(interaction.projected_body_uuid(), None);

        let summoned_eye = OtherEntity {
            id: 216,
            kind: "minecraft:eye_of_ender".into(),
            interaction: None,
            eye: Some(crate::game::end_state::SummonedEye {
                motion: Vec3::new(0.1, 0.2, 0.3),
                life: 3,
                fire_ticks: 27,
                synced_fire: false,
                has_visual_fire: false,
            }),
            ..interaction
        };
        assert_eq!(
            summoned_eye.projected_velocity(),
            Some(Vec3::new(0.1, 0.2, 0.3))
        );
        assert_eq!(summoned_eye.projected_bounds(), None);
        assert_eq!(summoned_eye.projected_body_fire_ticks(), Some(27));
    }

    #[test]
    fn guest_projection_matches_body() {
        let (mut guest, _rx) =
            crate::game::server_side::server_travel::credits_respawn_tests::placed_guest(
                217,
                crate::game::Dim::Overworld,
                [11.0, 64.0, -12.0],
            );
        guest.yaw = 170.0;
        guest.pitch = 15.0;
        guest.on_ground = true;
        assert_eq!(guest.projected_id(), guest.entity_id as u32);
        assert_eq!(guest.projected_kind(), "minecraft:player");
        assert_eq!(
            guest.projected_position(),
            Vec3::new(guest.position[0], guest.position[1], guest.position[2])
        );
        assert_eq!(guest.projected_velocity(), None);
        assert_eq!(guest.projected_rotation(), Some([guest.yaw, guest.pitch]));
        assert_eq!(guest.projected_bounds(), Some(guest.bounds()));
        assert_eq!(guest.projected_on_ground(), Some(guest.on_ground));
        assert_eq!(guest.projected_body_fire_ticks(), None);
        assert_eq!(guest.projected_air_supply(), None);
        assert_eq!(guest.projected_body_removed(), None);
        assert_eq!(
            guest.projected_body_uuid(),
            Some(sunrise_sim::lan::playerdata::uuid_ints(guest.uuid))
        );
    }

    #[test]
    fn off_level_boat_and_cart_projection_match_body() {
        let boat = Boat::new(218, Arc::from("oak"), Vec3::new(1.0, 63.0, 2.0), 30.0);
        let expected_bounds = boat.bounds();
        let off_boat = OffLevel::Boat(boat);
        assert_eq!(off_boat.projected_id(), 218);
        assert_eq!(off_boat.projected_kind(), "minecraft:oak_boat");
        assert_eq!(off_boat.projected_bounds(), Some(expected_bounds));

        let cart = Minecart::new(219, CartKind::Hopper, Vec3::new(3.0, 64.0, 4.0));
        let expected_bounds = cart.bounds();
        let off_cart = OffLevel::Cart(cart);
        assert_eq!(off_cart.projected_id(), 219);
        assert_eq!(off_cart.projected_kind(), "minecraft:hopper_minecart");
        assert_eq!(off_cart.projected_bounds(), Some(expected_bounds));
    }
}
