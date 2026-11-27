//! Read-only projection of shared entity state across the typed entity lists
//! and every body owner outside them.
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

/// Read-only view of the values entities share. Absent fields return `None`
/// rather than inventing defaults.
pub trait EntityProjection {
    fn projected_id(&self) -> u32;
    fn projected_kind(&self) -> Cow<'_, str>;
    fn projected_position(&self) -> Vec3;
    fn projected_velocity(&self) -> Option<Vec3> {
        None
    }
    fn projected_rotation(&self) -> Option<[f32; 2]> {
        None
    }
    fn projected_bounds(&self) -> Option<Aabb> {
        None
    }
    fn projected_on_ground(&self) -> Option<bool> {
        None
    }
    fn projected_fire_ticks(&self) -> Option<i32> {
        None
    }
    fn projected_air_supply(&self) -> Option<i32> {
        None
    }
    fn projected_is_removed(&self) -> Option<bool> {
        None
    }
    fn projected_saved_uuid(&self) -> Option<[i32; 4]> {
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
    pub const fn new(id: u32, dragon: &'a Dragon, part: usize) -> Self {
        Self { id, dragon, part }
    }
}

pub fn extents_aabb((lo, hi): ([f64; 3], [f64; 3])) -> Aabb {
    Aabb::new(
        Vec3::new(lo[0], lo[1], lo[2]),
        Vec3::new(hi[0], hi[1], hi[2]),
    )
}

fn bounds_center((lo, hi): ([f64; 3], [f64; 3])) -> Vec3 {
    Vec3::new(
        (lo[0] + hi[0]) / 2.0,
        (lo[1] + hi[1]) / 2.0,
        (lo[2] + hi[2]) / 2.0,
    )
}

pub fn knot_bounds(fence: [i32; 3]) -> Aabb {
    let knot = sunrise_sim::mobs::leash::knot_state(fence);
    let (p, half) = (knot.position, knot.width / 2.0);
    Aabb::new(
        Vec3::new(p.x - half, p.y, p.z - half),
        Vec3::new(p.x + half, p.y + knot.height, p.z + half),
    )
}

macro_rules! impl_entity_projection_from_entity {
    ($ty:ty { $($extra:item)* }) => {
        impl EntityProjection for $ty {
            fn projected_id(&self) -> u32 {
                Entity::id(self)
            }
            fn projected_kind(&self) -> Cow<'_, str> {
                Entity::native_type(self)
            }
            fn projected_position(&self) -> Vec3 {
                Entity::position(self)
            }
            fn projected_velocity(&self) -> Option<Vec3> {
                Some(Entity::velocity(self))
            }
            fn projected_rotation(&self) -> Option<[f32; 2]> {
                Some(Entity::rotation(self))
            }
            fn projected_bounds(&self) -> Option<Aabb> {
                Some(Entity::bounds(self))
            }
            fn projected_on_ground(&self) -> Option<bool> {
                Some(Entity::on_ground(self))
            }
            $($extra)*
        }
    };
}

impl_entity_projection_from_entity!(Mob {
    fn projected_fire_ticks(&self) -> Option<i32> {
        Some(self.fire_ticks)
    }
    fn projected_air_supply(&self) -> Option<i32> {
        Some(self.air)
    }
    fn projected_is_removed(&self) -> Option<bool> {
        Some(self.removed)
    }
});

impl_entity_projection_from_entity!(ItemEntity {
    fn projected_fire_ticks(&self) -> Option<i32> {
        Some(self.fire_ticks)
    }
});

impl_entity_projection_from_entity!(ExperienceOrb {
    fn projected_fire_ticks(&self) -> Option<i32> {
        Some(self.fire_ticks)
    }
    fn projected_air_supply(&self) -> Option<i32> {
        Some(self.air)
    }
});

impl_entity_projection_from_entity!(Boat {
    fn projected_fire_ticks(&self) -> Option<i32> {
        Some(self.fire_ticks)
    }
});

impl_entity_projection_from_entity!(Minecart {});

impl_entity_projection_from_entity!(ArmorStand {
    fn projected_fire_ticks(&self) -> Option<i32> {
        Some(self.fire_ticks)
    }
    fn projected_air_supply(&self) -> Option<i32> {
        Some(self.air)
    }
});

impl_entity_projection_from_entity!(PrimedTnt {});

impl_entity_projection_from_entity!(FallingBlock {
    fn projected_fire_ticks(&self) -> Option<i32> {
        Some(self.fire_ticks)
    }
    fn projected_saved_uuid(&self) -> Option<[i32; 4]> {
        self.saved.as_deref().map(|(uuid, _)| *uuid)
    }
});

impl_entity_projection_from_entity!(Projectile {
    fn projected_fire_ticks(&self) -> Option<i32> {
        Some(self.arrow.as_ref().map_or(self.fire_ticks, |a| a.fire_ticks))
    }
    fn projected_air_supply(&self) -> Option<i32> {
        Some(self.air_supply)
    }
    fn projected_is_removed(&self) -> Option<bool> {
        Some(self.destroyed || self.arrow.as_ref().is_some_and(|a| a.removed))
    }
    fn projected_saved_uuid(&self) -> Option<[i32; 4]> {
        self.uuid
    }
});

impl_entity_projection_from_entity!(Rocket {});

impl_entity_projection_from_entity!(ThrownTrident {
    fn projected_fire_ticks(&self) -> Option<i32> {
        Some(self.fire_ticks)
    }
});

impl_entity_projection_from_entity!(GeneralRow {
    fn projected_fire_ticks(&self) -> Option<i32> {
        Some(self.base.fire_ticks)
    }
    fn projected_saved_uuid(&self) -> Option<[i32; 4]> {
        Some(self.uuid)
    }
});

impl EntityProjection for Painting {
    fn projected_id(&self) -> u32 {
        self.id
    }
    fn projected_kind(&self) -> Cow<'_, str> {
        Cow::Borrowed("minecraft:painting")
    }
    fn projected_position(&self) -> Vec3 {
        bounds_center(self.bounds())
    }
    fn projected_bounds(&self) -> Option<Aabb> {
        Some(extents_aabb(self.bounds()))
    }
}

impl EntityProjection for ItemFrame {
    fn projected_id(&self) -> u32 {
        self.id
    }
    fn projected_kind(&self) -> Cow<'_, str> {
        Cow::Borrowed(self.kind())
    }
    fn projected_position(&self) -> Vec3 {
        bounds_center(self.bounds())
    }
    fn projected_rotation(&self) -> Option<[f32; 2]> {
        Some(self.entity_rotation())
    }
    fn projected_bounds(&self) -> Option<Aabb> {
        Some(extents_aabb(self.bounds()))
    }
}

impl EntityProjection for Cushion {
    fn projected_id(&self) -> u32 {
        self.id
    }
    fn projected_kind(&self) -> Cow<'_, str> {
        Cow::Borrowed("minecraft:cushion")
    }
    fn projected_position(&self) -> Vec3 {
        self.position
    }
    fn projected_bounds(&self) -> Option<Aabb> {
        Some(extents_aabb(self.bounds()))
    }
}

impl EntityProjection for Bolt {
    fn projected_id(&self) -> u32 {
        self.id
    }
    fn projected_kind(&self) -> Cow<'_, str> {
        Cow::Borrowed("minecraft:lightning_bolt")
    }
    fn projected_position(&self) -> Vec3 {
        Vec3::new(self.at[0], self.at[1], self.at[2])
    }
    fn projected_velocity(&self) -> Option<Vec3> {
        Some(self.velocity)
    }
    fn projected_rotation(&self) -> Option<[f32; 2]> {
        Some(self.rotation)
    }
    fn projected_bounds(&self) -> Option<Aabb> {
        let p = self.projected_position();
        Some(Aabb::new(p, p))
    }
    fn projected_on_ground(&self) -> Option<bool> {
        Some(self.on_ground)
    }
    fn projected_fire_ticks(&self) -> Option<i32> {
        Some(self.fire_ticks)
    }
    fn projected_air_supply(&self) -> Option<i32> {
        Some(self.air)
    }
}

impl EntityProjection for Dragon {
    fn projected_id(&self) -> u32 {
        self.id
    }
    fn projected_kind(&self) -> Cow<'_, str> {
        Cow::Borrowed("minecraft:ender_dragon")
    }
    fn projected_position(&self) -> Vec3 {
        self.position
    }
    fn projected_velocity(&self) -> Option<Vec3> {
        Some(self.velocity)
    }
    fn projected_rotation(&self) -> Option<[f32; 2]> {
        Some([self.yaw, self.pitch])
    }
    fn projected_bounds(&self) -> Option<Aabb> {
        Some(self.body_bounds())
    }
}

impl EntityProjection for DragonPartRef<'_> {
    fn projected_id(&self) -> u32 {
        self.id
    }
    fn projected_kind(&self) -> Cow<'_, str> {
        Cow::Borrowed("minecraft:ender_dragon")
    }
    fn projected_position(&self) -> Vec3 {
        self.dragon.parts[self.part].0
    }
    fn projected_bounds(&self) -> Option<Aabb> {
        Some(self.dragon.part_box(self.part))
    }
    fn projected_saved_uuid(&self) -> Option<[i32; 4]> {
        Some(self.dragon.parts[self.part].3)
    }
}

impl EntityProjection for Crystal {
    fn projected_id(&self) -> u32 {
        self.id
    }
    fn projected_kind(&self) -> Cow<'_, str> {
        Cow::Borrowed("minecraft:end_crystal")
    }
    fn projected_position(&self) -> Vec3 {
        self.position
    }
    fn projected_rotation(&self) -> Option<[f32; 2]> {
        Some(self.rotation)
    }
    fn projected_bounds(&self) -> Option<Aabb> {
        Some(self.bounds())
    }
    fn projected_on_ground(&self) -> Option<bool> {
        Some(self.fall.0)
    }
    fn projected_fire_ticks(&self) -> Option<i32> {
        Some(self.fire)
    }
}

impl EntityProjection for Breath {
    fn projected_id(&self) -> u32 {
        self.id
    }
    fn projected_kind(&self) -> Cow<'_, str> {
        Cow::Borrowed("minecraft:area_effect_cloud")
    }
    fn projected_position(&self) -> Vec3 {
        Vec3::new(self.at[0], self.at[1], self.at[2])
    }
    fn projected_velocity(&self) -> Option<Vec3> {
        Some(Vec3::new(self.motion[0], self.motion[1], self.motion[2]))
    }
    fn projected_fire_ticks(&self) -> Option<i32> {
        Some(self.fire_ticks)
    }
}

impl EntityProjection for Hook {
    fn projected_id(&self) -> u32 {
        self.id
    }
    fn projected_kind(&self) -> Cow<'_, str> {
        Cow::Borrowed("minecraft:fishing_bobber")
    }
    fn projected_position(&self) -> Vec3 {
        self.position
    }
    fn projected_velocity(&self) -> Option<Vec3> {
        Some(self.velocity)
    }
    fn projected_rotation(&self) -> Option<[f32; 2]> {
        Some([self.yaw, self.pitch])
    }
    fn projected_bounds(&self) -> Option<Aabb> {
        Some(self.bounds())
    }
    fn projected_on_ground(&self) -> Option<bool> {
        Some(self.ground.on_ground)
    }
    fn projected_fire_ticks(&self) -> Option<i32> {
        Some(self.base.fire_ticks)
    }
    fn projected_saved_uuid(&self) -> Option<[i32; 4]> {
        Some(self.uuid)
    }
}

impl EntityProjection for OminousItemSpawner {
    fn projected_id(&self) -> u32 {
        self.id
    }
    fn projected_kind(&self) -> Cow<'_, str> {
        Cow::Borrowed("minecraft:ominous_item_spawner")
    }
    fn projected_position(&self) -> Vec3 {
        Vec3::new(self.position[0], self.position[1], self.position[2])
    }
    fn projected_rotation(&self) -> Option<[f32; 2]> {
        self.raw.as_deref().map(sunrise_sim::entity_nbt::rotation)
    }
    fn projected_fire_ticks(&self) -> Option<i32> {
        Some(self.fire_ticks)
    }
    fn projected_saved_uuid(&self) -> Option<[i32; 4]> {
        Some(self.uuid)
    }
}

impl EntityProjection for Fangs {
    fn projected_id(&self) -> u32 {
        self.id
    }
    fn projected_kind(&self) -> Cow<'_, str> {
        Cow::Borrowed("minecraft:evoker_fangs")
    }
    fn projected_position(&self) -> Vec3 {
        Vec3::new(self.pos[0], self.pos[1], self.pos[2])
    }
    fn projected_velocity(&self) -> Option<Vec3> {
        Some(self.motion)
    }
    fn projected_fire_ticks(&self) -> Option<i32> {
        Some(self.fire_ticks)
    }
}

impl EntityProjection for Knot {
    fn projected_id(&self) -> u32 {
        self.id
    }
    fn projected_kind(&self) -> Cow<'_, str> {
        Cow::Borrowed("minecraft:leash_knot")
    }
    fn projected_position(&self) -> Vec3 {
        sunrise_sim::mobs::leash::knot_state(self.fence).position
    }
    fn projected_bounds(&self) -> Option<Aabb> {
        Some(knot_bounds(self.fence))
    }
    fn projected_fire_ticks(&self) -> Option<i32> {
        Some(self.fire_ticks)
    }
}

impl EntityProjection for EyeOfEnder {
    fn projected_id(&self) -> u32 {
        self.id
    }
    fn projected_kind(&self) -> Cow<'_, str> {
        Cow::Borrowed("minecraft:eye_of_ender")
    }
    fn projected_position(&self) -> Vec3 {
        Vec3::new(self.position.x, self.position.y, self.position.z)
    }
    fn projected_velocity(&self) -> Option<Vec3> {
        Some(Vec3::new(self.velocity.x, self.velocity.y, self.velocity.z))
    }
}

impl EntityProjection for ThrownPotion {
    fn projected_id(&self) -> u32 {
        self.id
    }
    fn projected_kind(&self) -> Cow<'_, str> {
        Cow::Borrowed(thrown_kind(&self.stack))
    }
    fn projected_position(&self) -> Vec3 {
        self.position
    }
    fn projected_velocity(&self) -> Option<Vec3> {
        Some(self.velocity)
    }
}

impl EntityProjection for OtherEntity {
    fn projected_id(&self) -> u32 {
        self.id
    }
    fn projected_kind(&self) -> Cow<'_, str> {
        Cow::Borrowed(self.kind.as_str())
    }
    fn projected_position(&self) -> Vec3 {
        self.position
    }
    fn projected_velocity(&self) -> Option<Vec3> {
        self.eye.as_ref().map(|e| e.motion)
    }
    fn projected_rotation(&self) -> Option<[f32; 2]> {
        Some([self.yaw, self.pitch])
    }
    fn projected_bounds(&self) -> Option<Aabb> {
        self.display_marker_or_interaction_bounds()
    }
    fn projected_fire_ticks(&self) -> Option<i32> {
        self.eye.as_ref().map(|e| e.fire_ticks)
    }
}

impl EntityProjection for PlayerMotion {
    fn projected_id(&self) -> u32 {
        self.entity_id as u32
    }
    fn projected_kind(&self) -> Cow<'_, str> {
        Cow::Borrowed("minecraft:player")
    }
    fn projected_position(&self) -> Vec3 {
        self.position
    }
    fn projected_velocity(&self) -> Option<Vec3> {
        Some(self.velocity)
    }
    fn projected_rotation(&self) -> Option<[f32; 2]> {
        Some([self.yaw, self.pitch])
    }
    fn projected_bounds(&self) -> Option<Aabb> {
        Some(self.bounds())
    }
    fn projected_on_ground(&self) -> Option<bool> {
        Some(self.ground.on_ground)
    }
    fn projected_fire_ticks(&self) -> Option<i32> {
        Some(self.fire_ticks)
    }
    fn projected_air_supply(&self) -> Option<i32> {
        Some(self.air)
    }
}

impl EntityProjection for Guest {
    fn projected_id(&self) -> u32 {
        self.entity_id as u32
    }
    fn projected_kind(&self) -> Cow<'_, str> {
        Cow::Borrowed("minecraft:player")
    }
    fn projected_position(&self) -> Vec3 {
        Vec3::new(self.position[0], self.position[1], self.position[2])
    }
    fn projected_velocity(&self) -> Option<Vec3> {
        self.motion.as_ref().map(|m| m.velocity)
    }
    fn projected_rotation(&self) -> Option<[f32; 2]> {
        Some([self.yaw, self.pitch])
    }
    fn projected_bounds(&self) -> Option<Aabb> {
        Some(self.bounds())
    }
    fn projected_on_ground(&self) -> Option<bool> {
        Some(self.on_ground)
    }
    fn projected_fire_ticks(&self) -> Option<i32> {
        self.motion.as_ref().map(|m| m.fire_ticks)
    }
    fn projected_air_supply(&self) -> Option<i32> {
        self.motion.as_ref().map(|m| m.air)
    }
    fn projected_saved_uuid(&self) -> Option<[i32; 4]> {
        Some(sunrise_sim::lan::playerdata::uuid_ints(self.uuid))
    }
}

impl EntityProjection for OffLevel {
    fn projected_id(&self) -> u32 {
        match self {
            Self::Mob(m) => m.projected_id(),
            Self::Boat(b) => b.projected_id(),
            Self::Cart(c) => c.projected_id(),
        }
    }
    fn projected_kind(&self) -> Cow<'_, str> {
        match self {
            Self::Mob(m) => m.projected_kind(),
            Self::Boat(b) => b.projected_kind(),
            Self::Cart(c) => c.projected_kind(),
        }
    }
    fn projected_position(&self) -> Vec3 {
        match self {
            Self::Mob(m) => m.projected_position(),
            Self::Boat(b) => b.projected_position(),
            Self::Cart(c) => c.projected_position(),
        }
    }
    fn projected_velocity(&self) -> Option<Vec3> {
        match self {
            Self::Mob(m) => m.projected_velocity(),
            Self::Boat(b) => b.projected_velocity(),
            Self::Cart(c) => c.projected_velocity(),
        }
    }
    fn projected_rotation(&self) -> Option<[f32; 2]> {
        match self {
            Self::Mob(m) => m.projected_rotation(),
            Self::Boat(b) => b.projected_rotation(),
            Self::Cart(c) => c.projected_rotation(),
        }
    }
    fn projected_bounds(&self) -> Option<Aabb> {
        match self {
            Self::Mob(m) => m.projected_bounds(),
            Self::Boat(b) => b.projected_bounds(),
            Self::Cart(c) => c.projected_bounds(),
        }
    }
    fn projected_on_ground(&self) -> Option<bool> {
        match self {
            Self::Mob(m) => m.projected_on_ground(),
            Self::Boat(b) => b.projected_on_ground(),
            Self::Cart(c) => c.projected_on_ground(),
        }
    }
    fn projected_fire_ticks(&self) -> Option<i32> {
        match self {
            Self::Mob(m) => m.projected_fire_ticks(),
            Self::Boat(b) => b.projected_fire_ticks(),
            Self::Cart(c) => c.projected_fire_ticks(),
        }
    }
    fn projected_air_supply(&self) -> Option<i32> {
        match self {
            Self::Mob(m) => m.projected_air_supply(),
            Self::Boat(b) => b.projected_air_supply(),
            Self::Cart(c) => c.projected_air_supply(),
        }
    }
    fn projected_is_removed(&self) -> Option<bool> {
        match self {
            Self::Mob(m) => m.projected_is_removed(),
            Self::Boat(b) => b.projected_is_removed(),
            Self::Cart(c) => c.projected_is_removed(),
        }
    }
    fn projected_saved_uuid(&self) -> Option<[i32; 4]> {
        match self {
            Self::Mob(m) => m.projected_saved_uuid(),
            Self::Boat(b) => b.projected_saved_uuid(),
            Self::Cart(c) => c.projected_saved_uuid(),
        }
    }
}

/// Read-only lookup of a common entity target through [`EntityProjection`].
/// Server-only owners (`Guest`, `Other`, guest `Hook`, `DragonPart`, natural `Knot`)
/// return `None` here and are resolved on the owning side.
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
