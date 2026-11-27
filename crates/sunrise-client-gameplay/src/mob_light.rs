//! The light a mob is drawn with.
use sunrise_client::mobs::{Mob, Species};
use sunrise_client::world::World;

/// `EntityRenderer.getPackedLightCoords`: the light `mob` is drawn with at `alpha`, sky in byte 0, block in byte 1.
pub fn packed_light(world: &World, mob: &Mob, alpha: f64, synced_on_fire: bool) -> u32 {
    // The probe is the mob's eye at the position this frame draws, not this tick's.
    let probe = mob.light_probe(alpha);
    let light = world.light(probe[0], probe[1], probe[2]);
    let mut packed = (u32::from(light >> 4) * 16) | ((u32::from(light & 15) * 16) << 8);
    // EntityRenderer.getBlockLightLevel: burning mobs and the Allay, Blaze, Magma Cube, Vex, Wither get block light 15.
    if mob.is_on_fire_with_synced(synced_on_fire)
        || matches!(
            mob.species(),
            Species::Allay | Species::Blaze | Species::MagmaCube | Species::Vex | Species::Wither
        )
    {
        packed = (packed & 0xFF) | ((15 * 16) << 8);
    }
    // GlowSquidRenderer.getBlockLightLevel: the body brightens as the dark ticks run out.
    if mob.species() == Species::GlowSquid {
        let lit = (packed >> 12) & 15;
        let block = sunrise_client::mobs::aquatic::glow_squid_block_light(mob, lit);
        packed = (packed & 0xFF) | ((block * 16) << 8);
    }
    packed
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use sunrise_client::mobs::sensor_scan_tests::kind;
    use sunrise_core::geometry::Vec3;
    use sunrise_world::registry::BlockStateId;

    #[test]
    fn fire_immune_strider_keeps_the_world_block_light() {
        let mut kind = kind();
        let strider = Arc::make_mut(&mut kind);
        strider.id = "minecraft:strider".into();
        strider.species = Species::Strider;
        strider.flags.fire_immune = true;
        let mut mob = Mob::new(1, kind, Vec3::ZERO, 0.0);
        let world = World::new(BlockStateId(0), 0);

        for (fire_ticks, synced_on_fire) in [(20, false), (0, true)] {
            mob.fire_ticks = fire_ticks;
            let light = packed_light(&world, &mob, 1.0, synced_on_fire);
            assert_eq!((light >> 12) & 15, 0);
        }
    }
}
