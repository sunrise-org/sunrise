//! Stored player health and server damage for the resident scene.
//! World/audio/network consumers are queued explicitly; full death and tick
//! lifecycles (drops, respawn, effects) remain separate work.
use crate::{Result, items::ItemStack};
pub use mod_api::entity::health::{HurtSource, ShieldBlock, Victim};
#[cfg(test)]
use std::fs;
use std::{io, path::Path};
use sunrise_entity_data::damage::{Damage, DamageKind, NamedDamage};
use sunrise_game_data::who::Who;
use sunrise_world::{
    attributes::live::LiveAttributes,
    block_landing::LandingDamage,
    combat_rules::ArmorWorld,
    damage_immunity::{self, DamageRule, EntityImmunityWorld, ImmunityTag, ImmunityWorld},
    damage_reduction::Bypass,
    damage_source::{Catalog, DamageSource, DamageTypeId, Scaling},
    food::{Difficulty, FoodData},
    living_damage::{DamageStat, DamageState},
    living_hurt::{HurtState, HurtTag, HurtWorld, PendingHurt},
    player_health::PlayerFloatIds,
    player_hurt::{CausingEntity, PlayerHurtWorld, ServerPlayerHurtWorld},
    server_player_combat::CombatEnvironment,
    server_player_damage::{PlayerBodyRead, ServerDamageEnvironment},
    synced_floats::{FloatValue, Observer, SyncedFloats},
};
pub type Attributes = LiveAttributes<Box<dyn sunrise_world::attributes::live::Identities>>;
pub(crate) type State = HurtState<DamageTypeId>;

/// The typed body a heal or movement hook lends; the mob side builds it
/// (`mobs/lend.rs`) and the old `health::` path stays.
pub use crate::mobs::lend::{LendBody, LivingLend};
pub use crate::world::HealthRules;
mod accepted_hit;
pub mod attack;
mod kill_credit;
use kill_credit::Credit;
pub use kill_credit::{KnockbackCredit, PlayerMemory, PlayerResolver};
mod hit_attempt;
pub use accepted_hit::AcceptedHit;
pub use attack::{damage_type, melee, mob_source, source_damage_type};
include!("health/hit.rs");

include!("health/damage.rs");

include!("health/player.rs");

include!("health/environment.rs");

#[cfg(test)]
mod datapack_tests;
