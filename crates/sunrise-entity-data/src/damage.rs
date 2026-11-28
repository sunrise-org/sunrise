//! The damage data a hit carries: its type and tags, and who caused it.
//!
//! Vanilla keeps these next to the hurt code (`DamageSource`, `DamageType`); Sunrise keeps
//! them here, below both the damage rules and the mob simulation, so neither names the
//! other for them.

use sunrise_game_data::who::Who;

/// A damage type's registry name and the tags consumed by mob combat.
///
/// `NamedDamage::registered` builds both from a registry entry, interning
/// its name. Code constructing a source directly supplies a static name and
/// flags matching its registry entry. Unset tags are false.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NamedDamage {
    pub name: &'static str,
    pub tags: u32,
}
impl NamedDamage {
    pub const IS_FIRE: u32 = 1 << 0;
    pub const BYPASSES_ARMOR: u32 = 1 << 1;
    pub const NO_KNOCKBACK: u32 = 1 << 2;
    pub const BYPASSES_INVULNERABILITY: u32 = 1 << 3;
    pub const BYPASSES_EFFECTS: u32 = 1 << 4;
    pub const BYPASSES_RESISTANCE: u32 = 1 << 5;
    pub const PANIC_CAUSES: u32 = 1 << 6;
    pub const IS_PROJECTILE: u32 = 1 << 7;
    pub const IS_EXPLOSION: u32 = 1 << 8;
    pub const IS_FALL: u32 = 1 << 9;
    pub const BYPASSES_ENCHANTMENTS: u32 = 1 << 10;
    pub const BYPASSES_COOLDOWN: u32 = 1 << 11;
    pub const DAMAGES_HELMET: u32 = 1 << 12;
    pub const IS_FREEZING: u32 = 1 << 13;
    pub const WITCH_RESISTANT_TO: u32 = 1 << 14;
    pub const NO_ANGER: u32 = 1 << 15;
    pub const BYPASSES_WOLF_ARMOR: u32 = 1 << 16;
    pub const SULFUR_CUBE_WITH_BLOCK_IMMUNE_TO: u32 = 1 << 17;
    pub const PANIC_ENVIRONMENTAL_CAUSES: u32 = 1 << 18;

    /// Each flag with the damage tag it stands for.
    const TAG_FLAGS: [(u32, &'static str); 19] = [
        (Self::IS_FIRE, "minecraft:is_fire"),
        (Self::BYPASSES_ARMOR, "minecraft:bypasses_armor"),
        (Self::NO_KNOCKBACK, "minecraft:no_knockback"),
        (
            Self::BYPASSES_INVULNERABILITY,
            "minecraft:bypasses_invulnerability",
        ),
        (Self::BYPASSES_EFFECTS, "minecraft:bypasses_effects"),
        (Self::BYPASSES_RESISTANCE, "minecraft:bypasses_resistance"),
        (Self::PANIC_CAUSES, "minecraft:panic_causes"),
        (
            Self::PANIC_ENVIRONMENTAL_CAUSES,
            "minecraft:panic_environmental_causes",
        ),
        (Self::IS_PROJECTILE, "minecraft:is_projectile"),
        (Self::IS_EXPLOSION, "minecraft:is_explosion"),
        (Self::IS_FALL, "minecraft:is_fall"),
        (
            Self::BYPASSES_ENCHANTMENTS,
            "minecraft:bypasses_enchantments",
        ),
        (Self::BYPASSES_COOLDOWN, "minecraft:bypasses_cooldown"),
        (Self::DAMAGES_HELMET, "minecraft:damages_helmet"),
        (Self::IS_FREEZING, "minecraft:is_freezing"),
        (Self::WITCH_RESISTANT_TO, "minecraft:witch_resistant_to"),
        (Self::NO_ANGER, "minecraft:no_anger"),
        (Self::BYPASSES_WOLF_ARMOR, "minecraft:bypasses_wolf_armor"),
        (
            Self::SULFUR_CUBE_WITH_BLOCK_IMMUNE_TO,
            "minecraft:sulfur_cube_with_block_immune_to",
        ),
    ];

    pub const fn new(name: &'static str, tags: u32) -> Self {
        Self { name, tags }
    }

    /// The registered damage type `name` with the tags `catalog` gives it,
    /// or None when the registry has no such type. The name is interned: the
    /// registries hold a handful of ids, so one copy of each is kept for the
    /// rest of the run.
    pub fn registered(catalog: &sunrise_world::damage_source::Catalog, name: &str) -> Option<Self> {
        use std::{collections::HashSet, sync::Mutex};
        static NAMES: Mutex<Option<HashSet<&'static str>>> = Mutex::new(None);
        let id = catalog.id(name)?;
        let mut names = NAMES.lock().unwrap_or_else(|e| e.into_inner());
        let names = names.get_or_insert_with(HashSet::new);
        let name = match names.get(name) {
            Some(&found) => found,
            None => {
                let leaked: &'static str = Box::leak(name.to_owned().into_boxed_str());
                names.insert(leaked);
                leaked
            }
        };
        Some(Self::with_catalog_tags(catalog, id, name))
    }

    /// Contact keys already own the names for the installation's lifetime.
    pub fn from_contact_key(
        catalog: &sunrise_world::damage_source::Catalog,
        key: mod_api::DamageTypeKey,
    ) -> Option<Self> {
        let name = mod_api::__private::parts_installation::damage_type_name(key)?;
        Some(Self::with_catalog_tags(catalog, catalog.id(name)?, name))
    }

    fn with_catalog_tags(
        catalog: &sunrise_world::damage_source::Catalog,
        id: sunrise_world::damage_source::DamageTypeId,
        name: &'static str,
    ) -> Self {
        let tags = Self::TAG_FLAGS
            .iter()
            .filter(|(_, tag)| catalog.in_tag(id, tag))
            .fold(0, |tags, (flag, _)| tags | flag);
        Self { name, tags }
    }

    pub const fn has(self, tag: u32) -> bool {
        self.tags & tag != 0
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DamageKind {
    /// An exact registry type and its tags, vanilla or modded.
    Named(NamedDamage),
    PlayerAttack,
    /// `mace_smash`: a mace hit after a fall (the player attack tags too).
    MaceSmash,
    /// `spear`: a stab of a spear (the player attack tags too).
    Spear,
    MobAttack,
    /// `mob_attack_no_aggro`: a goat's ram, the one type in `#no_anger`.
    MobAttackNoAggro,
    Arrow,
    Thrown,
    Fireball,
    Explosion,
    Fall,
    /// `fly_into_wall`: gliding into a horizontal collision.
    FlyIntoWall,
    FallingBlock,
    FallingStalactite,
    OnFire,
    InFire,
    /// `campfire`: standing in a lit campfire (`CampfireBlock.entityInside`).
    Campfire,
    Lava,
    Drown,
    /// `dry_out`: an axolotl out of water for too long.
    DryOut,
    Magic,
    /// `indirect_magic`: a thrown potion's hit (`DamageSources.indirectMagic`),
    /// which, unlike `magic`, knocks back.
    IndirectMagic,
    Sting,
    Cactus,
    /// `sweet_berry_bush`.
    SweetBerryBush,
    Generic,
    InWall,
    Thorns,
    SonicBoom,
    /// `freeze`: a fully frozen mob in powder snow.
    Freeze,
    /// `fell_out_of_world`: below the world's floor.
    OutOfWorld,
    /// `cramming`: crushed by the mobs around it.
    Cramming,
    /// `falling_anvil`: an anvil landing on it (it damages a helmet).
    FallingAnvil,
    /// `fireworks`: a firework rocket's burst (an explosion for Blast Protection).
    Fireworks,
    /// `generic_kill`: a kill that bypasses invulnerability.
    GenericKill,
    /// `lightning_bolt`.
    LightningBolt,
    /// `outside_border`: past the world border.
    OutsideBorder,
    /// `spit`: a llama's spit.
    Spit,
    /// `stalagmite`: a fall onto a stalagmite (a fall for Feather Falling).
    Stalagmite,
    /// `starve`: no food left.
    Starve,
    /// `hot_floor`: stepping on a magma block.
    HotFloor,
    /// `sulfur_cube_hot`: a sulfur cube's hot floor (a fire).
    SulfurCubeHot,
    /// `wind_charge`: a wind charge's burst (a projectile).
    WindCharge,
    /// `wither`: the wither effect.
    Wither,
    /// `wither_skull`: an owned wither skull's hit (`DamageSources.witherSkull`).
    WitherSkull,
}
impl DamageKind {
    /// `DamageSource.is(DamageTypes.X)`: the registered damage type's identity,
    /// whatever variant carries it, so a contact `minecraft:thorns` reacts as
    /// `DamageKind::Thorns` does. Tags stay the level's (`has`).
    pub fn is(self, registry_id: &str) -> bool {
        damage_type(self, false) == registry_id
    }

    /// DamageTypeTags.IS_EXPLOSION.
    pub fn is_explosion(self) -> bool {
        match self {
            Self::Named(named) => named.has(NamedDamage::IS_EXPLOSION),
            Self::Explosion | Self::Fireworks => true,
            _ => false,
        }
    }

    /// DamageTypeTags.BYPASSES_INVULNERABILITY
    pub fn bypasses_invulnerability(self) -> bool {
        if let Self::Named(named) = self {
            return named.has(NamedDamage::BYPASSES_INVULNERABILITY);
        }
        matches!(self, DamageKind::OutOfWorld | DamageKind::GenericKill)
    }
    /// DamageTypeTags.BYPASSES_EFFECTS
    pub fn bypasses_effects(self) -> bool {
        if let Self::Named(named) = self {
            return named.has(NamedDamage::BYPASSES_EFFECTS);
        }
        matches!(self, DamageKind::Starve)
    }
    /// DamageTypeTags.BYPASSES_RESISTANCE
    pub fn bypasses_resistance(self) -> bool {
        if let Self::Named(named) = self {
            return named.has(NamedDamage::BYPASSES_RESISTANCE);
        }
        matches!(self, DamageKind::OutOfWorld | DamageKind::GenericKill)
    }
    /// DamageTypeTags.BYPASSES_ENCHANTMENTS: `sonic_boom` alone in vanilla.
    pub fn bypasses_enchantments(self) -> bool {
        if let Self::Named(named) = self {
            return named.has(NamedDamage::BYPASSES_ENCHANTMENTS);
        }
        matches!(self, DamageKind::SonicBoom)
    }
    /// DamageTypeTags.BYPASSES_COOLDOWN: empty in vanilla.
    pub fn bypasses_cooldown(self) -> bool {
        matches!(self, Self::Named(named) if named.has(NamedDamage::BYPASSES_COOLDOWN))
    }
    /// DamageTypeTags.DAMAGES_HELMET: falling anvils, blocks and stalactites.
    pub fn damages_helmet(self) -> bool {
        if let Self::Named(named) = self {
            return named.has(NamedDamage::DAMAGES_HELMET);
        }
        matches!(
            self,
            DamageKind::FallingBlock | DamageKind::FallingAnvil | DamageKind::FallingStalactite
        )
    }
    /// DamageTypeTags.IS_FREEZING: `freeze` alone in vanilla.
    pub fn is_freezing(self) -> bool {
        if let Self::Named(named) = self {
            return named.has(NamedDamage::IS_FREEZING);
        }
        self == DamageKind::Freeze
    }
    /// DamageTypeTags.WITCH_RESISTANT_TO: magic, indirect magic, sonic boom and thorns.
    pub fn witch_resistant_to(self) -> bool {
        if let Self::Named(named) = self {
            return named.has(NamedDamage::WITCH_RESISTANT_TO);
        }
        matches!(
            self,
            DamageKind::Magic
                | DamageKind::IndirectMagic
                | DamageKind::SonicBoom
                | DamageKind::Thorns
        )
    }
    /// DamageTypeTags.NO_ANGER: `mob_attack_no_aggro` alone in vanilla, a goat's ram.
    pub fn no_anger(self) -> bool {
        if let Self::Named(named) = self {
            return named.has(NamedDamage::NO_ANGER);
        }
        self == DamageKind::MobAttackNoAggro
    }
}
/// A damage source: its type, the attacking entity and where it came from.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Damage {
    pub kind: DamageKind,
    pub attacker: Option<Who>,
    pub from: Option<[f64; 3]>,
    /// The type of `DamageSource.getDirectEntity()` when it is not the
    /// attacker: the arrow, trident, potion, spit or primed TNT that dealt
    /// the blow, whoever (if anyone) owns it. None when the attacker dealt it
    /// itself or no entity did, which `DamageSource.isDirect` reads as direct.
    pub direct: Option<&'static str>,
    /// The attacker's species id (`species::vanilla_id`), when it was a mob.
    pub attacker_species: Option<&'static str>,
}
impl Damage {
    pub fn fall() -> Self {
        Self {
            kind: DamageKind::Fall,
            attacker: None,
            from: None,
            direct: None,
            attacker_species: None,
        }
    }
    pub fn of(kind: DamageKind) -> Self {
        Self {
            kind,
            attacker: None,
            from: None,
            direct: None,
            attacker_species: None,
        }
    }
    pub fn player(from: [f64; 3]) -> Self {
        Self {
            kind: DamageKind::PlayerAttack,
            attacker: Some(Who::Player),
            from: Some(from),
            direct: None,
            attacker_species: None,
        }
    }
    /// `DamageSources.mace`: a player's smash attack.
    pub fn mace(from: [f64; 3]) -> Self {
        Self {
            kind: DamageKind::MaceSmash,
            attacker: Some(Who::Player),
            from: Some(from),
            direct: None,
            attacker_species: None,
        }
    }
    /// `DamageSources.spear`: a player's stab with a spear.
    pub fn spear(from: [f64; 3]) -> Self {
        Self {
            kind: DamageKind::Spear,
            attacker: Some(Who::Player),
            from: Some(from),
            direct: None,
            attacker_species: None,
        }
    }
    /// `DamageSources.indirectMagic(potion, owner)`: `attacker` is the owner
    /// (none for a dispenser's potion), `from` the potion's position, which
    /// the knockback pushes away from.
    /// `direct` is the potion's or the cloud's entity type.
    pub fn indirect_magic(direct: &'static str, attacker: Option<Who>, from: [f64; 3]) -> Self {
        Self {
            kind: DamageKind::IndirectMagic,
            attacker,
            from: Some(from),
            direct: Some(direct),
            attacker_species: None,
        }
    }
    /// `DamageSources.fireworks(rocket, owner)`: `attacker` is the owner
    /// (none for a rocket nobody fired), `from` the rocket's position, which
    /// the knockback pushes away from (`calculateHorizontalHurtKnockbackDirection`).
    pub fn fireworks(attacker: Option<Who>, from: [f64; 3]) -> Self {
        Self {
            kind: DamageKind::Fireworks,
            attacker,
            from: Some(from),
            direct: Some("minecraft:firework_rocket"),
            attacker_species: None,
        }
    }
    pub fn mob(id: u32, from: [f64; 3]) -> Self {
        Self {
            kind: DamageKind::MobAttack,
            attacker: Some(Who::Mob(id)),
            from: Some(from),
            direct: None,
            attacker_species: None,
        }
    }
    /// The source with `kind` as its direct entity (`getDirectEntity`), an
    /// entity other than the attacker.
    pub fn with_direct(mut self, kind: &'static str) -> Self {
        self.direct = Some(kind);
        self
    }
    /// DamageTypeTags.PANIC_CAUSES
    pub fn causes_panic(&self) -> bool {
        if let DamageKind::Named(named) = self.kind {
            return named.has(NamedDamage::PANIC_CAUSES);
        }
        !matches!(
            self.kind,
            DamageKind::Fall
                | DamageKind::FlyIntoWall
                | DamageKind::Drown
                | DamageKind::DryOut
                | DamageKind::Generic
                | DamageKind::InWall
                | DamageKind::SweetBerryBush
                | DamageKind::OutOfWorld
                | DamageKind::FallingBlock
                | DamageKind::FallingAnvil
                | DamageKind::Cramming
                | DamageKind::GenericKill
                | DamageKind::OutsideBorder
                | DamageKind::Spit
                | DamageKind::Stalagmite
                | DamageKind::Starve
                | DamageKind::Campfire
                // `mob_attack` panics; its no-aggro twin is not in the tag.
                | DamageKind::MobAttackNoAggro
        )
    }
    /// DamageTypeTags.BYPASSES_ARMOR
    pub fn bypasses_armor(&self) -> bool {
        if let DamageKind::Named(named) = self.kind {
            return named.has(NamedDamage::BYPASSES_ARMOR);
        }
        matches!(
            self.kind,
            DamageKind::Fall
                | DamageKind::FlyIntoWall
                | DamageKind::OnFire
                | DamageKind::Drown
                | DamageKind::Magic
                | DamageKind::IndirectMagic
                | DamageKind::InWall
                | DamageKind::Generic
                | DamageKind::SonicBoom
                | DamageKind::Freeze
                | DamageKind::OutOfWorld
                | DamageKind::Cramming
                | DamageKind::GenericKill
                | DamageKind::OutsideBorder
                | DamageKind::Stalagmite
                | DamageKind::Starve
                | DamageKind::Wither
        )
    }
    pub fn is_fire(&self) -> bool {
        if let DamageKind::Named(named) = self.kind {
            return named.has(NamedDamage::IS_FIRE);
        }
        matches!(
            self.kind,
            DamageKind::OnFire
                | DamageKind::InFire
                | DamageKind::Campfire
                | DamageKind::HotFloor
                | DamageKind::Lava
                | DamageKind::Fireball
                | DamageKind::SulfurCubeHot
        )
    }
    /// DamageTypeTags.NO_KNOCKBACK
    pub fn no_knockback(&self) -> bool {
        if let DamageKind::Named(named) = self.kind {
            return named.has(NamedDamage::NO_KNOCKBACK);
        }
        matches!(
            self.kind,
            DamageKind::Explosion
                | DamageKind::Fall
                | DamageKind::FlyIntoWall
                | DamageKind::OnFire
                | DamageKind::InFire
                | DamageKind::Campfire
                | DamageKind::HotFloor
                | DamageKind::Lava
                | DamageKind::Drown
                | DamageKind::DryOut
                | DamageKind::Magic
                | DamageKind::Cactus
                | DamageKind::SweetBerryBush
                | DamageKind::Generic
                | DamageKind::InWall
                | DamageKind::Freeze
                | DamageKind::OutOfWorld
                | DamageKind::LightningBolt
                | DamageKind::Cramming
                | DamageKind::GenericKill
                | DamageKind::OutsideBorder
                | DamageKind::Stalagmite
                | DamageKind::Starve
                | DamageKind::SulfurCubeHot
                | DamageKind::Wither
                | DamageKind::Spear
        )
    }
    pub fn is_projectile(&self) -> bool {
        if let DamageKind::Named(named) = self.kind {
            return named.has(NamedDamage::IS_PROJECTILE);
        }
        matches!(
            self.kind,
            DamageKind::Arrow
                | DamageKind::Thrown
                | DamageKind::Fireball
                | DamageKind::WindCharge
                | DamageKind::WitherSkull
        )
    }
}

/// The damage type id of a Sunrise damage kind. `caused`: the source has a
/// causing entity (only a blast asks).
pub fn damage_type(kind: DamageKind, caused: bool) -> &'static str {
    use DamageKind as K;
    match kind {
        K::Named(named) => named.name,
        K::PlayerAttack => "minecraft:player_attack",
        K::MaceSmash => "minecraft:mace_smash",
        K::Spear => "minecraft:spear",
        K::MobAttack => "minecraft:mob_attack",
        K::MobAttackNoAggro => "minecraft:mob_attack_no_aggro",
        K::Arrow => "minecraft:arrow",
        K::Thrown => "minecraft:thrown",
        K::Fireball => "minecraft:fireball",
        K::Explosion if caused => "minecraft:player_explosion",
        K::Explosion => "minecraft:explosion",
        K::Fall => "minecraft:fall",
        K::FlyIntoWall => "minecraft:fly_into_wall",
        K::FallingBlock => "minecraft:falling_block",
        K::FallingStalactite => "minecraft:falling_stalactite",
        K::OnFire => "minecraft:on_fire",
        K::InFire => "minecraft:in_fire",
        K::Campfire => "minecraft:campfire",
        K::Lava => "minecraft:lava",
        K::Drown => "minecraft:drown",
        K::DryOut => "minecraft:dry_out",
        K::Magic => "minecraft:magic",
        K::IndirectMagic => "minecraft:indirect_magic",
        K::Sting => "minecraft:sting",
        K::Cactus => "minecraft:cactus",
        K::SweetBerryBush => "minecraft:sweet_berry_bush",
        K::Generic => "minecraft:generic",
        K::InWall => "minecraft:in_wall",
        K::Thorns => "minecraft:thorns",
        K::SonicBoom => "minecraft:sonic_boom",
        K::Freeze => "minecraft:freeze",
        K::OutOfWorld => "minecraft:out_of_world",
        K::Cramming => "minecraft:cramming",
        K::FallingAnvil => "minecraft:falling_anvil",
        K::Fireworks => "minecraft:fireworks",
        K::GenericKill => "minecraft:generic_kill",
        K::LightningBolt => "minecraft:lightning_bolt",
        K::OutsideBorder => "minecraft:outside_border",
        K::Spit => "minecraft:spit",
        K::Stalagmite => "minecraft:stalagmite",
        K::Starve => "minecraft:starve",
        K::HotFloor => "minecraft:hot_floor",
        K::SulfurCubeHot => "minecraft:sulfur_cube_hot",
        K::WindCharge => "minecraft:wind_charge",
        K::Wither => "minecraft:wither",
        K::WitherSkull => "minecraft:wither_skull",
    }
}
