//! A server-side roll of `LivingEntity.dropFromLootTable`, collected instead of spawned: for a
//! live entity, or for a kind created and never spawned.

#[cfg(test)]
#[path = "death_loot_tests.rs"]
mod tests;

use crate::game::{
    Game, commands::Target, creatures::from_loot, loot_rolls::ListenedRolls,
    native_server::PlayerRead,
};
use mod_api::{
    DeathLootError, EntityRef, Identifier, LevelRef, LivingRef, LootDamageSource, PlayerRef,
};
use sunrise_client::mobs::{
    Mob,
    drops::{self, PlainRolls},
};
use sunrise_core::geometry::Vec3;
use sunrise_items::items::ItemStack;
use sunrise_world::{
    attribute_map::ValueKind,
    loot::{EntityLootParams, LootDamage},
};

impl Game<'_> {
    pub(in crate::game) fn entity_roll_death_loot(
        &mut self,
        entity: EntityRef,
        table: Option<&Identifier>,
        source: LootDamageSource<'_>,
        last_damage_player: Option<PlayerRef>,
    ) -> Result<Vec<ItemStack>, DeathLootError> {
        if !self.is_logical_server() {
            return Err(DeathLootError::Unavailable);
        }
        let target = self
            .native_entity_target(entity)
            .ok_or(DeathLootError::StaleEntity)?;
        if !target.is_living() {
            return Err(DeathLootError::NotLiving);
        }
        if self.target_level(target) != self.dim {
            return Err(DeathLootError::StaleEntity);
        }
        let origin = self
            .target_position(target)
            .ok_or(DeathLootError::StaleEntity)?;
        if self
            .health_resources
            .damage
            .id(source.damage_type.as_str())
            .is_none()
        {
            return Err(DeathLootError::UnknownDamageType);
        }
        let mob = match target {
            Target::Mob(id) => Some(
                self.mobs
                    .iter()
                    .find(|m| m.id == id)
                    .ok_or(DeathLootError::StaleEntity)?,
            ),
            _ => None,
        };
        let seed = mob.map_or(0, |m| m.extra.death_table_seed);
        let table = table.map(ToString::to_string).unwrap_or_else(|| match mob {
            Some(m) => drops::mob_loot_table(m),
            None => drops::entity_type_loot_table(&self.target_type(target)),
        });
        let loot = self.server_loot.get().ok_or(DeathLootError::Unavailable)?;
        if !loot.has_table(&table) {
            return Err(DeathLootError::UnknownTable);
        }

        let this = self.target_loot_entity(target);
        self.roll_death_context(
            this,
            Some(LivingRef::new(entity)),
            &table,
            seed,
            origin,
            source,
            last_damage_player,
        )
    }

    /// `LevelRef::roll_entity_loot`: [`Self::entity_roll_death_loot`] for a `kind` created at
    /// `origin` and never spawned (`EntityType.create` without `finalizeSpawn`). The mob is
    /// built off the books: it takes no entity id, joins no list and fires nothing.
    pub(in crate::game) fn level_roll_entity_loot(
        &mut self,
        level: LevelRef,
        kind: &Identifier,
        table: Option<&Identifier>,
        origin: [f64; 3],
        source: LootDamageSource<'_>,
        last_damage_player: Option<PlayerRef>,
    ) -> Result<Vec<ItemStack>, DeathLootError> {
        if !self.is_logical_server() {
            return Err(DeathLootError::Unavailable);
        }
        if !self.level_handle_current(level) || self.dim.key() != level.dimension() {
            return Err(DeathLootError::StaleEntity);
        }
        let mob_type = self
            .res
            .mob_types()
            .get(kind.as_str())
            .cloned()
            .ok_or(DeathLootError::UnknownKind)?;
        if self
            .health_resources
            .damage
            .id(source.damage_type.as_str())
            .is_none()
        {
            return Err(DeathLootError::UnknownDamageType);
        }
        let mob = Mob::new(0, mob_type, Vec3::new(origin[0], origin[1], origin[2]), 0.0);
        let table = table
            .map(ToString::to_string)
            .unwrap_or_else(|| drops::mob_loot_table(&mob));
        let loot = self.server_loot.get().ok_or(DeathLootError::Unavailable)?;
        if !loot.has_table(&table) {
            return Err(DeathLootError::UnknownTable);
        }
        let this = drops::loot_entity_with_variants(&mob, None, Some(self.world_variants()));
        self.roll_death_context(this, None, &table, 0, origin, source, last_damage_player)
    }

    /// The `ENTITY` roll both share once `this` and the table are known: `source` and the
    /// credited player resolved, then `drops::roll_death_table`. `enchanted_entity_loot` is
    /// asked when there is a living entity to name.
    #[allow(clippy::too_many_arguments)]
    fn roll_death_context(
        &mut self,
        this: sunrise_world::loot::LootEntity,
        living: Option<LivingRef>,
        table: &str,
        seed: i64,
        origin: [f64; 3],
        source: LootDamageSource<'_>,
        last_damage_player: Option<PlayerRef>,
    ) -> Result<Vec<ItemStack>, DeathLootError> {
        let loot = self.server_loot.get().ok_or(DeathLootError::Unavailable)?;
        let damage_type = source.damage_type.to_string();
        let resolve = |handle: Option<EntityRef>| {
            handle
                .map(|e| {
                    self.native_entity_target(e)
                        .ok_or(DeathLootError::StaleEntity)
                })
                .transpose()
        };
        let attacker = resolve(source.causing)?;
        let direct = resolve(source.direct)?;
        let credited = last_damage_player
            .map(|player| {
                let read = self
                    .player_handle_read(player)
                    .ok_or(DeathLootError::StaleEntity)?;
                Ok(match read {
                    PlayerRead::Acting { body: Some(_), .. } => Target::Player,
                    PlayerRead::Guest(guest) => Target::Guest(guest.conn),
                    PlayerRead::Acting { body: None, .. } => {
                        return Err(DeathLootError::StaleEntity);
                    }
                })
            })
            .transpose()?;

        let source_entity = attacker.map(|t| self.target_loot_entity(t));
        let direct_entity = if direct == attacker {
            source_entity.clone()
        } else {
            direct.map(|t| self.target_loot_entity(t))
        };
        let credit = credited.map(|t| {
            let luck = self
                .native_target_attribute(t, "minecraft:luck", ValueKind::Computed)
                .unwrap_or(0.0) as f32;
            (self.target_loot_entity(t), luck)
        });
        let params = EntityLootParams::death(
            this,
            Some(LootDamage {
                kind: damage_type.clone(),
                source: source_entity,
                direct: direct_entity,
                is_direct: source.causing == source.direct,
            }),
            credit,
        );
        let world_seed = self.level.seed;
        let stacks = self.caller_loot(Some(origin), None).with(|| {
            let listened = living
                .filter(|_| mod_api::__private::listened(mod_api::HookId::EnchantedEntityLoot));
            if let Some(living) = listened {
                let hurt = mod_api::entity::health::HurtSource {
                    damage_type: &damage_type,
                    causing: source.causing.map_or(0, |e| e.id() as i32),
                    direct: source.direct.map_or(0, |e| e.id() as i32),
                    position: direct.and_then(|t| self.target_position(t)),
                    direct_type: None,
                };
                let mut rolls = ListenedRolls::new(self, living, &hurt);
                drops::roll_death_table(&loot, table, &params, &mut rolls, seed, world_seed)
            } else {
                let mut rolls = PlainRolls {
                    sequences: &mut self.rand_sequences,
                    level_random: &mut self.world.random,
                };
                drops::roll_death_table(&loot, table, &params, &mut rolls, seed, world_seed)
            }
        });
        Ok(stacks
            .iter()
            .filter(|stack| !stack.is_empty())
            .map(from_loot)
            .collect())
    }
}
