use crate::game::ServerGame;
use sunrise_items::items::ItemStack;

impl ServerGame<'_> {
    pub(in crate::game) fn server_item_combat_actor(
        &self,
        actor: mod_api::LivingRef,
    ) -> Option<(bool, bool, f64, bool)> {
        use super::commands::Target;
        let target = self.server_living_read_target(actor)?;
        Some(match target {
            Target::Player => {
                let p = self.common.player.as_ref()?;
                (true, self.common.creative, p.fall_distance, p.fall_flying)
            }
            Target::Guest(conn) => {
                let guest = self.command_guest(conn)?;
                let p = guest.motion.as_ref()?;
                (true, guest.creative(), p.fall_distance, p.fall_flying)
            }
            Target::Mob(id) => {
                let mob = self.common.mobs.iter().find(|m| m.id == id && !m.removed)?;
                (false, false, mob.fall_distance, mob.extra.fall_flying)
            }
            Target::Stand(_) | Target::Dragon(_) => (false, false, 0.0, false),
            _ => return None,
        })
    }

    /// `ItemStack.hurtAndBreak(amount, level, player, onBreak)` on a stack nobody wears: the
    /// stack after the wear, None once it broke away. A named player is checked as the other
    /// player reads check it (`player_handle_read`) and must be in `level`, so the wear and its
    /// random draws stay in the level in place; the player is then swapped in for the wear
    /// (their infinite materials, their durability trigger) without changing levels. Refused
    /// off the server, for another level than the one in place, or for a player who is not
    /// here or is in another level.
    pub(in crate::game) fn item_stack_hurt_and_break(
        &mut self,
        stack: &ItemStack,
        level: mod_api::LevelRef,
        amount: i32,
        player: Option<mod_api::PlayerRef>,
    ) -> Result<Option<ItemStack>, mod_api::StackWriteError> {
        use super::commands::Target;
        use super::durability::Wearer;
        use mod_api::StackWriteError::Unavailable;
        if !self.common.is_logical_server()
            || level.dimension() != self.common.dim.key()
            || self.native_level_world(level).is_none()
        {
            return Err(Unavailable);
        }
        let (worn, _broken) = match player {
            None => self.damage_stack_worn(stack, amount, Wearer::Nobody),
            Some(player) => {
                self.player_handle_read(player).ok_or(Unavailable)?;
                let target = self
                    .native_entity_target(player.entity())
                    .ok_or(Unavailable)?;
                let here = match target {
                    Target::Player => self.acting_in_level_in_place(),
                    Target::Guest(conn) => self
                        .server
                        .lan
                        .as_ref()
                        .and_then(|lan| lan.hub.guest(conn))
                        .is_some_and(|guest| {
                            sunrise_gameplay_common::game::Dim::from_id(&guest.dimension)
                                == self.common.dim
                        }),
                    _ => false,
                };
                if !here {
                    return Err(Unavailable);
                }
                self.for_player(target, |game| {
                    game.damage_stack_worn(stack, amount, Wearer::Player)
                })
                .ok_or(Unavailable)?
            }
        };
        Ok(worn)
    }

    /// `EnchantmentHelper.modifyKnockback` for a stack nobody wears, behind
    /// `ItemResource::modify_knockback`. As in vanilla, an effect's requirements are checked on
    /// the level's random and its value applied on the victim's own (`victim.getRandom()`), here
    /// each on a copy, so neither stream moves. The context carries the three entities' kinds
    /// and their fall distance and flying state, from the same place an attack reads them. A
    /// stale handle is an absent entity (`causing` falls back to `direct`, as in
    /// `EntityRef::hurt`); an entity in another level than the one in place refuses the read.
    pub(in crate::game) fn item_resource_modify_knockback(
        &self,
        stack: &ItemStack,
        victim: mod_api::EntityRef,
        damage_type: &str,
        direct: Option<mod_api::EntityRef>,
        causing: Option<mod_api::EntityRef>,
        base: f32,
    ) -> Option<f32> {
        if !self.common.is_logical_server() {
            return None;
        }
        let items = server_world_items!(self);
        items.get(&stack.item)?;
        let in_place = |entity: mod_api::EntityRef| -> Option<bool> {
            Some(self.entity_level(entity)?.dimension() == self.common.dim.key())
        };
        if !in_place(victim)? {
            return None;
        }
        self.common.health_resources.damage.id(damage_type)?;
        let resolve = |entity: Option<mod_api::EntityRef>| match entity {
            None => Some(None),
            Some(entity) => match in_place(entity) {
                None => Some(None),
                Some(true) => Some(Some(entity)),
                Some(false) => None,
            },
        };
        let direct = resolve(direct)?;
        let causing = resolve(causing)?.or(direct);
        // `DamageSource.isDirect`: the direct entity is the causing one.
        let is_direct = direct.map(|e| e.id()) == causing.map(|e| e.id());
        let this = self.entity_kind(victim)?.id(self)?;
        let kind =
            |entity: Option<mod_api::EntityRef>| entity.and_then(|e| self.entity_kind(e)?.id(self));
        let direct_kind = kind(direct);
        let causing_kind = kind(causing);
        let motion = |entity: Option<mod_api::EntityRef>| {
            let id = i32::try_from(entity?.id()).ok()?;
            self.enchantment_actor(id)?.motion
        };
        let ctx = sunrise_items::enchant::EffectContext {
            this: Some(this.as_str()),
            attacker: causing_kind.as_deref(),
            direct_attacker: direct_kind.as_deref(),
            is_direct,
            damage_type: Some(damage_type),
            items: Some(items),
            this_motion: motion(Some(victim)),
            attacker_motion: motion(causing),
            direct_motion: motion(direct),
            ..Default::default()
        };
        let effective = self.common.item_combat_stack(stack);
        let levels = effective.all_enchantments();
        let mut level_random = (*self.common.world.random).clone();
        let mut victim_random = self
            .damage_victim_random_copy(victim.id())
            .unwrap_or_else(|| level_random.clone());
        Some(self.item_enchantments().modify_entity_value(
            &levels,
            "minecraft:knockback",
            &ctx,
            &mut level_random,
            &mut victim_random,
            base,
        ))
    }

    /// Roll through the game's enchantment registry and the active level's random source.
    pub(in crate::game) fn item_resource_enchant_randomly(
        &mut self,
        stack: &ItemStack,
        level: mod_api::LevelRef,
        power: i32,
        pool: &mod_api::Identifier,
    ) -> Result<Option<ItemStack>, mod_api::StackWriteError> {
        if level.dimension() != self.common.dim.key() || self.native_level_world(level).is_none() {
            return Ok(None);
        }
        let items = server_world_items!(self);
        let enchantments = std::sync::Arc::clone(self.item_enchantments());
        let pool = enchantments.tag(&pool.to_string());
        if pool.is_empty() {
            return Ok(None);
        }
        let value = enchantments.enchantable_of(stack);
        let picked = enchantments.select_with_value(
            items,
            &mut self.common.world.random,
            &stack.item,
            power,
            pool,
            value,
        );
        Ok(sunrise_sim::stations::apply_enchantment_list(stack, picked))
    }

    /// `EnchantmentHelper.modifyDurabilityToRepairFromXp` for `stack` and `points`, by the world's
    /// enchantment definitions, drawing from the active level's random as Mending's repair does,
    /// behind `ItemResource::durability_to_repair_from_xp`. API @API:item_components@.
    pub(in crate::game) fn item_resource_durability_to_repair_from_xp(
        &mut self,
        stack: &ItemStack,
        points: i32,
    ) -> u32 {
        super::enchant_effects::repair_from_xp(
            &server_world_stations!(self).enchantments,
            Some(server_world_items!(self)),
            stack,
            points,
            &mut self.common.world.random,
        )
    }
}

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt5 as _;

use sunrise_gameplay_common::game::tables::GameItems as _;

use sunrise_gameplay_common::game::commands_common::CommandsCommonGameExt2 as _;

use sunrise_gameplay_common::game::native_level_refs_common::NativeLevelRefsCommonGameExt1 as _;
