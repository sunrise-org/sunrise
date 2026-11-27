use crate::game::dragon_common::{Breath, FightWho};
use sunrise_core::{geometry::Vec3, nbt::Tag, random::LegacyRandom};
use sunrise_game_data::text::Text;
use sunrise_items::items::ItemStack;
use sunrise_level::{
    brewing::PotionContents,
    effects::PlayerEffects,
    particle_options::{ParticleOptions, ParticleType},
};
use sunrise_sim::{entity_nbt, mobs::Who};

/// Per-cloud settings; natural lingering clouds keep their creation defaults.
pub struct CloudSettings {
    pub wait_time: i32,
    pub reapplication_delay: i32,
    pub radius_on_use: f32,
    pub duration_on_use: i32,
    pub duration_scale: f32,
}

/// A thrown splash or lingering potion in flight.
#[derive(Clone, Debug)]
pub struct ThrownPotion {
    /// Its entity id once a server's clients track it (0 until then).
    pub id: u32,
    pub stack: ItemStack,
    /// The position before the last tick, for drawing between ticks.
    pub previous: Vec3,
    pub position: Vec3,
    pub velocity: Vec3,
    pub age: u32,
    /// `Projectile.getOwner`'s display name: the player who threw it (none
    /// for a dispenser's).
    pub owner: Option<Text>,
    /// `Projectile.getOwner` as a target: the thrower (`Who::Player`, a
    /// guest's with `guest` set), none for a dispenser's or a server's.
    pub thrower: Option<Who>,
    /// On a server, the guest (by connection) who threw it.
    pub guest: Option<u32>,
    /// `Projectile.leftOwner`: until then it can't hit its thrower.
    pub left_owner: bool,
    /// `Projectile.lastDeflectedBy`: a mob deflects it once in a row.
    pub last_deflected_by: Option<u32>,
    /// `Entity.random`: `ProjectileDeflection.REVERSE` draws from the
    /// projectile's own source, not the level's.
    pub random: LegacyRandom,
}

pub struct CloudParticle {
    pub options: ParticleOptions,
    /// Retain the pre-existing save representation only for built-in options.
    pub vanilla_tag: Option<Tag>,
}

/// A potion-bearing `AreaEffectCloud`: its effects, settings and recent victims.
pub struct PotionCloud {
    pub contents: PotionContents,
    pub settings: CloudSettings,
    /// `AreaEffectCloud.radiusPerTick`: natural clouds use `-radius / duration`.
    pub radius_per_tick: f32,
    /// The potion's colour (`PotionContents.getColor`).
    pub color: u32,
    /// `AreaEffectCloud.customParticle`: the value decoded when the cloud was loaded.
    pub custom_particle: Option<CloudParticle>,
    /// Each victim and the tick it can be affected again.
    pub victims: Vec<(Victim, i32)>,
    /// Who its instant damage names (`indirectMagic(cloud, owner)`): the
    /// thrower, else the cloud.
    pub source: Text,
    /// `AreaEffectCloud.getOwner`: the thrower (`ThrownLingeringPotion.onHitAsPotion`), whose
    /// kill a mob's death to its instant damage is.
    pub owner: Option<Who>,
    /// On a server, the guest (by connection) who threw it.
    pub guest: Option<u32>,
}

/// `AreaEffectCloud.addAdditionalSaveData` on the live cloud, shared by
/// chunk persistence and command reads. Raw NBT supplies unmodeled fields.
pub fn area_effect_cloud_to_nbt(c: &Breath, uuid: [i32; 4], raw: Option<&Tag>) -> Tag {
    let [x, y, z] = c.at;
    let [vx, vy, vz] = c.motion;
    let mut tag = entity_nbt::entity_base_tag(
        "minecraft:area_effect_cloud",
        Vec3::new(x, y, z),
        Vec3::new(vx, vy, vz),
        // The cloud never turns, so keep its loaded rotation.
        raw.map_or([0.0; 2], entity_nbt::loaded_rotation),
        uuid,
        raw,
    );
    let Tag::Compound(map) = &mut tag else {
        unreachable!("entity_base_tag always writes a compound");
    };
    let potion = c.potion.as_ref();
    let settings = potion.map(|p| &p.settings);
    for (key, value) in [
        (
            "fall_distance",
            Tag::Double(if c.fall_distance == 0.0 {
                0
            } else {
                c.fall_distance.to_bits()
            }),
        ),
        ("Fire", Tag::Short(c.fire_ticks as i16)),
        ("Age", Tag::Int(c.age)),
        ("Duration", Tag::Int(c.duration)),
        ("WaitTime", Tag::Int(settings.map_or(20, |s| s.wait_time))),
        (
            "ReapplicationDelay",
            Tag::Int(settings.map_or(REAPPLICATION_DELAY, |s| s.reapplication_delay)),
        ),
        (
            "DurationOnUse",
            Tag::Int(settings.map_or(0, |s| s.duration_on_use)),
        ),
        (
            "RadiusOnUse",
            Tag::Float(settings.map_or(0.0, |s| s.radius_on_use).to_bits()),
        ),
        (
            "RadiusPerTick",
            Tag::Float(potion.map_or(0.0, |p| p.radius_per_tick).to_bits()),
        ),
        ("Radius", Tag::Float(c.radius.to_bits())),
    ] {
        map.insert(key.into(), value);
    }

    if let Some(potion) = potion {
        map.remove(&"custom_particle".into());
        if let Some(particle) = &potion.custom_particle {
            let saved = match &particle.options.typed_particle {
                Some((value, _)) => match mod_api::__private::encode_particle_map(value) {
                    Ok(fields) => Some(Tag::Compound(fields)),
                    Err(error) => {
                        log::warn!(
                            "Cannot save cloud particle {:?}: {error}",
                            particle.options.mod_particle
                        );
                        None
                    }
                },
                None => particle.vanilla_tag.clone(),
            };
            if let Some(saved) = saved {
                map.insert("custom_particle".into(), saved);
            }
        }

        map.remove(&"potion_contents".into());
        if let Some(contents) = potion.contents.to_nbt() {
            map.insert("potion_contents".into(), contents);
        }
    }
    let scale = settings.map_or(1.0, |s| s.duration_scale);
    if scale != 1.0 {
        map.insert("potion_duration_scale".into(), Tag::Float(scale.to_bits()));
    } else {
        map.remove(&"potion_duration_scale".into());
    }
    tag
}

/// What a player's body was last given for its effects and enchantments:
/// a guest keeps their own while the host's are in `PotionState`.
#[derive(Default)]
pub struct BodyPotions {
    pub protection_for: Option<(u64, Vec<Option<ItemStack>>)>,
    pub enchant_modifiers: Vec<(String, String, f64, String)>,
}

/// Effects, thrown potions and the cached armor and enchantment modifiers.
#[derive(Default)]
pub struct PotionState {
    pub effects: PlayerEffects,
    /// Reused holder snapshot for the read-only mod effect tick event.
    pub tick_keys: std::cell::RefCell<Vec<mod_api::EffectKey>>,
    pub thrown: Vec<ThrownPotion>,
    /// Armor the protection table was computed for.
    pub protection_for: Option<(u64, Vec<Option<ItemStack>>)>,

    /// Enchantment attribute modifiers on the player: (attribute, id, amount, operation).
    pub enchant_modifiers: Vec<(String, String, f64, String)>,
}

/// An entity a cloud has affected (`AreaEffectCloud.victims`).
#[derive(Clone, Copy, PartialEq)]
pub enum Victim {
    Mob(u32),
    Player(FightWho),
}

/// `AreaEffectCloud` wait time set by `ThrownLingeringPotion.onHitAsPotion`.
pub(in crate::game) const LINGERING_WAIT: i32 = 10;

/// `AreaEffectCloud.DEFAULT_REAPPLICATION_DELAY`.
pub(in crate::game) const REAPPLICATION_DELAY: i32 = 20;

/// The entity type of a thrown potion or bottle.
fn thrown_kind(stack: &ItemStack) -> &'static str {
    match &*stack.item {
        "minecraft:lingering_potion" => "minecraft:lingering_potion",
        "minecraft:experience_bottle" => "minecraft:experience_bottle",
        _ => "minecraft:splash_potion",
    }
}

impl CloudSettings {
    pub fn lingering() -> Self {
        Self {
            wait_time: LINGERING_WAIT,
            reapplication_delay: REAPPLICATION_DELAY,
            radius_on_use: -0.5,
            duration_on_use: 0,
            duration_scale: 0.25,
        }
    }
}

impl PotionCloud {
    pub fn new(contents: PotionContents, settings: CloudSettings, radius_per_tick: f32) -> Self {
        let color = contents.color();
        Self {
            contents,
            settings,
            radius_per_tick,
            color,
            custom_particle: None,
            victims: Vec::new(),
            source: Text::translate("entity.minecraft.area_effect_cloud", &[]),
            owner: None,
            guest: None,
        }
    }

    /// `AreaEffectCloud.updateParticle`: the custom particle when one is set,
    /// else an entity effect in the potion's colour.
    pub fn particle(&self) -> ParticleOptions {
        self.custom_particle
            .as_ref()
            .map(|particle| particle.options.clone())
            .unwrap_or_else(|| {
                let c = |shift: u32| (self.color >> shift & 255) as f32 / 255.0;
                ParticleOptions::colored(ParticleType::EntityEffect, [c(16), c(8), c(0), 1.0])
            })
    }
}

impl PotionState {
    /// Thrown potions in flight as (entity type, position, motion), for the
    /// dispenser parity fixture.
    pub fn thrown_entities(&self) -> Vec<(&'static str, Vec3, Vec3)> {
        self.thrown
            .iter()
            .map(|t| (thrown_kind(&t.stack), t.position, t.velocity))
            .collect()
    }
    pub fn clear_thrown(&mut self) {
        self.thrown.clear();
    }
    /// The numbered ones as a server's clients track them: (id, entity
    /// type, item, position, motion).
    pub fn thrown_tracked(
        &self,
    ) -> impl Iterator<Item = (u32, &'static str, &ItemStack, Vec3, Vec3)> {
        self.thrown.iter().filter(|t| t.id != 0).map(|t| {
            (
                t.id,
                thrown_kind(&t.stack),
                &t.stack,
                t.position,
                t.velocity,
            )
        })
    }
    /// A server's move: thrown potions have no interpolation handler, so
    /// `moveOrInterpolateTo` puts them there at once. False when `id` isn't one.
    pub fn remote_thrown_pose(&self, id: u32) -> Option<([f64; 3], [f64; 3])> {
        let t = self.thrown.iter().find(|t| t.id == id)?;
        Some((
            [t.position.x, t.position.y, t.position.z],
            [t.previous.x, t.previous.y, t.previous.z],
        ))
    }
    pub fn has_thrown(&self, id: u32) -> bool {
        self.thrown.iter().any(|t| t.id == id)
    }
}
