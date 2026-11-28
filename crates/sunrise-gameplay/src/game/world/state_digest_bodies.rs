use super::{effect, optional_tag, stack, tag, vec3};
use std::{
    hash::{DefaultHasher, Hash},
    mem::discriminant,
};
use sunrise_core::nbt::Tag;
use sunrise_gameplay_common::game::{
    FallingBlock, boats_common::Boat, entity::Entity, fireworks::Rocket, general::GeneralRow,
    minecarts::Minecart, tnt_common::PrimedTnt,
};
use sunrise_items::{
    equipment::Equipment,
    items::{ItemStack, Items},
};
use sunrise_sim::{
    entities::{ArmorStand, ExperienceOrb, ItemEntity},
    living_attributes::{Live, LivingAttributes},
    mobs::{Mob, spawner::SpawnerExt as _},
    projectiles::Projectile,
    thrown_trident::ThrownTrident,
};

pub(in crate::game::server_side::state_digest) trait BodyDigest:
    Entity
{
    fn hash_body(&self, h: &mut DefaultHasher, items: &Items);
}

pub(super) fn slots<'a>(
    h: &mut DefaultHasher,
    slots: impl ExactSizeIterator<Item = &'a Option<ItemStack>>,
    items: &Items,
) {
    slots.len().hash(h);
    for slot in slots {
        stack(h, slot.as_ref(), items);
    }
}

pub(super) fn equipment(h: &mut DefaultHasher, equipment: &Equipment, items: &Items) {
    slots(h, equipment.iter(), items);
}

pub(super) fn attributes(h: &mut DefaultHasher, attributes: &Live) {
    let mut instances: Vec<_> = attributes.instances().collect();
    instances.sort_unstable_by_key(|&(_, key, _)| key.name());
    instances.len().hash(h);
    for (_, key, instance) in instances {
        attribute(h, key.name(), instance);
    }
}

fn attribute(h: &mut DefaultHasher, name: &str, instance: &sunrise_world::attributes::Instance) {
    (name, instance.base_value().to_bits()).hash(h);
    let modifiers = instance.modifiers();
    modifiers.len().hash(h);
    for modifier in modifiers {
        (
            modifier.id(),
            modifier.amount().to_bits(),
            discriminant(&modifier.operation()),
        )
            .hash(h);
    }
    let permanent = instance.pack().modifiers;
    permanent.len().hash(h);
    for modifier in permanent {
        modifier.id().hash(h);
    }
}

pub(super) fn living_attributes(h: &mut DefaultHasher, live: &LivingAttributes) {
    let mut saved = Vec::new();
    live.save_into(&mut saved);
    saved.sort_by_cached_key(|entry| match entry.get("id") {
        Some(Tag::String(id)) => Some(id.clone()),
        _ => None,
    });
    tag(h, &Tag::List(saved));
    live.built().is_some().hash(h);
    if let Some(live) = live.built() {
        attributes(h, live);
    }
}

impl BodyDigest for Mob {
    fn hash_body(&self, h: &mut DefaultHasher, items: &Items) {
        let e = &self.extra;
        (
            self.absorption.to_bits(),
            self.hurt_time,
            self.death_time,
            self.no_ai,
            self.persistent,
        )
            .hash(h);
        (
            e.age,
            e.variant,
            e.variant2,
            e.variant3,
            e.color,
            e.tame,
            e.owner,
            e.sitting,
            e.sheared,
            e.size,
            e.profession.id(),
            e.villager_level,
        )
            .hash(h);
        equipment(h, &e.equipment, items);
        slots(h, e.items.iter(), items);
        e.inventory.len().hash(h);
        for item in &e.inventory {
            stack(h, Some(item), items);
        }
        let a = self.attrs;
        (
            [
                a.speed,
                a.attack_knockback,
                a.follow_range,
                a.knockback_resistance,
                a.spawn_reinforcements,
                a.jump_strength,
            ]
            .map(f64::to_bits),
            [a.attack_damage, a.max_health].map(f32::to_bits),
        )
            .hash(h);
        e.generic_attributes.is_some().hash(h);
        if let Some(attributes) = &e.generic_attributes {
            let mut instances: Vec<_> = attributes
                .catalog()
                .attributes()
                .filter_map(|(id, def)| Some((def.name.as_str(), attributes.instance(id)?)))
                .collect();
            instances.sort_unstable_by_key(|&(name, _)| name);
            instances.len().hash(h);
            for (name, instance) in instances {
                attribute(h, name, instance);
            }
        }
        e.effects.len().hash(h);
        for e in &e.effects {
            (e.id, e.duration, e.amplifier, e.show_icon).hash(h);
        }
        e.effect_chains.len().hash(h);
        for chain in &e.effect_chains {
            effect(h, chain);
        }
        e.hidden_effects.hash(h);
    }
}

impl BodyDigest for ArmorStand {
    fn hash_body(&self, h: &mut DefaultHasher, items: &Items) {
        (
            self.show_arms,
            self.small,
            self.no_base_plate,
            self.invisible,
            self.marker,
            self.disabled_slots,
            self.name_visible,
            self.body_yaw.to_bits(),
            self.head_yaw.to_bits(),
            self.hurt_time,
            self.death_time,
        )
            .hash(h);
        self.pose.map(|angles| angles.map(f32::to_bits)).hash(h);
        equipment(h, &self.equipment, items);
        living_attributes(h, &self.live);
    }
}

impl BodyDigest for Boat {
    fn hash_body(&self, h: &mut DefaultHasher, items: &Items) {
        (
            &self.wood,
            self.chest,
            self.damage.to_bits(),
            self.hurt_time,
            self.hurt_dir,
            discriminant(&self.status),
            self.bubble_time,
            self.leash_spin.to_bits(),
            self.above_bubble_column,
            self.bubble_down,
            self.out_of_control.to_bits(),
            self.age,
        )
            .hash(h);
        slots(h, self.items.iter(), items);
    }
}

impl BodyDigest for Minecart {
    fn hash_body(&self, h: &mut DefaultHasher, items: &Items) {
        (
            self.kind.id(),
            self.fuel,
            self.fuse,
            self.enabled,
            self.on_rails,
            self.custom_offset,
            self.last_activated,
            self.command_due,
            self.tick_count,
            self.in_water,
        )
            .hash(h);
        vec3(h, self.push);
        slots(h, self.items.iter(), items);
        optional_tag(h, self.custom_display.as_ref());
        self.loot.is_some().hash(h);
        if let Some(loot) = &self.loot {
            tag(h, &loot.table);
            optional_tag(h, loot.seed.as_ref());
        }
        tag(h, &Tag::compound(self.command.save()));
        self.spawner.is_some().hash(h);
        if let Some(spawner) = &self.spawner {
            (
                spawner.spawn_delay,
                spawner.min_spawn_delay,
                spawner.max_spawn_delay,
                spawner.spawn_count,
                spawner.max_nearby_entities,
                spawner.required_player_range,
                spawner.spawn_range,
            )
                .hash(h);
            tag(h, &Tag::compound(spawner.save()));
        }
    }
}

impl BodyDigest for ItemEntity {
    fn hash_body(&self, h: &mut DefaultHasher, items: &Items) {
        (self.pickup_delay, self.thrower, self.target).hash(h);
        stack(h, Some(&self.stack), items);
    }
}

impl BodyDigest for Rocket {
    fn hash_body(&self, h: &mut DefaultHasher, items: &Items) {
        (
            self.life,
            self.lifetime,
            self.attached,
            self.attached_to,
            self.angled,
            self.owner,
            self.guest,
            self.owner_uuid,
            self.left_owner,
            self.last_deflected_by,
        )
            .hash(h);
        stack(h, Some(&self.item), items);
    }
}

impl BodyDigest for FallingBlock {
    fn hash_body(&self, h: &mut DefaultHasher, _items: &Items) {
        (self.state.0, self.start_pos, self.age, self.drop_item).hash(h);
    }
}

impl BodyDigest for ThrownTrident {
    fn hash_body(&self, h: &mut DefaultHasher, items: &Items) {
        (
            self.in_ground,
            self.life,
            self.loyalty,
            self.returning,
            discriminant(&self.pickup),
            self.owner,
            self.owner_uuid,
            self.guest,
            self.dealt_damage,
            self.in_ground_time,
        )
            .hash(h);
        stack(h, Some(&self.stack), items);
    }
}

impl BodyDigest for GeneralRow {
    fn hash_body(&self, h: &mut DefaultHasher, _items: &Items) {
        (
            self.type_slot,
            self.owner,
            self.payload_version,
            &self.payload_bytes,
            self.boarding_cooldown,
        )
            .hash(h);
    }
}

macro_rules! no_body_fields {
    ($($ty:ty),+ $(,)?) => {$(
        impl BodyDigest for $ty {
            fn hash_body(&self, _h: &mut DefaultHasher, _items: &Items) {}
        }
    )+};
}

no_body_fields!(PrimedTnt, ExperienceOrb, Projectile);
