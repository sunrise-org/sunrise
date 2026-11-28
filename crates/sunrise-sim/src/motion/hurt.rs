//! A player's hurt path (`hurtServer`), on the stored movement state: the hit's
//! pre-checks, the damage site it lends, and the health/food writes it leaves.
use super::{LegacyRandom, MotionEvent, PlayerMotion, Vec3};
use crate::Result;
use crate::health::{
    ArmorWeapon, Attributes, BlockOutcome, Blocking, DamageLevel, Environment, HURT_LOG,
    HealthEvent, HealthResources, HealthRules, HurtRecord, Hurting, PausedHurt, State, Updates,
    Victim,
};
use sunrise_world::damage_source::DamageTypeId;
use sunrise_world::{
    player_health::PlayerAttributeHealth, server_player_combat::ServerPlayerCombat,
};

impl PlayerMotion {
    /// Where the player stands and looks (`getViewVector`), as a hit's [`Victim`].
    pub(crate) fn victim(&self) -> Victim {
        let view = sunrise_world::view_direction::view_vector(self.pitch, self.yaw);
        Victim {
            position: [self.position.x, self.position.y, self.position.z],
            view: [view.x, view.y, view.z],
        }
    }
    /// `hurtServer` of `amount` from `source` in `level`.
    pub fn hurt(
        &mut self,
        level: &mut DamageLevel<'_>,
        resources: &HealthResources,
        rules: HealthRules,
        source: DamageTypeId,
        amount: f32,
    ) -> Result<bool> {
        Ok(self
            .hurt_blocking(level, resources, rules, source, amount, None)?
            .hurt)
    }
    pub fn hurt_blocking(
        &mut self,
        level: &mut DamageLevel<'_>,
        resources: &HealthResources,
        rules: HealthRules,
        source: DamageTypeId,
        amount: f32,
        blocking: Option<Blocking>,
    ) -> Result<BlockOutcome> {
        self.hurt_caused_by(
            level, resources, rules, source, amount, blocking, false, 0, false,
        )
    }
    /// `hurt_blocking` from a source whose causing entity is a living
    /// non-player (`living_cause`), which the `when_caused_by_living_non_player`
    /// damage types scale with the difficulty. `causing` is that entity's id
    /// when it is known (0 otherwise), for the native damage hook.
    pub fn hurt_caused_by(
        &mut self,
        level: &mut DamageLevel<'_>,
        resources: &HealthResources,
        rules: HealthRules,
        source: DamageTypeId,
        amount: f32,
        blocking: Option<Blocking>,
        living_cause: bool,
        causing: i32,
        creative_source: bool,
    ) -> Result<BlockOutcome> {
        let hit = self.start_hurt(
            level,
            resources,
            rules,
            source,
            amount,
            blocking,
            living_cause,
            causing,
            creative_source,
            false,
            true,
            None,
        )?;
        self.finish_hurt(level, resources, rules, hit)
    }
    /// `hurtServer` up to `hurtHelmet`, for a player who wears a `helmet`; the arguments are
    /// `hurt_caused_by`'s. The caller has the player and the level back to wear the helmet
    /// with ([`PausedHurt::helmet_damage`]), and ends the hit with [`Self::finish_hurt`].
    /// `can_harm_player` is the server PvP decision, applied after the invulnerability
    /// observation. Non-PvP damage passes true. The base observation is retained across
    /// all continuations belonging to this hit.
    pub fn start_hurt(
        &mut self,
        level: &mut DamageLevel<'_>,
        resources: &HealthResources,
        rules: HealthRules,
        source: DamageTypeId,
        amount: f32,
        blocking: Option<Blocking>,
        living_cause: bool,
        causing: i32,
        creative_source: bool,
        helmet: bool,
        can_harm_player: bool,
        armor_weapon: Option<ArmorWeapon>,
    ) -> Result<PausedHurt> {
        let mut hit = PausedHurt {
            pending: None,
            invulnerability_checked: None,
            can_harm_player,
            selected: None,
            inner: None,
            source,
            source_name: String::new(),
            amount,
            before: self.health.health() + self.health.absorption(),
            fire_ticks: self.fire_ticks,
            helmet,
            living_cause,
            causing,
            armor_weapon,
            // The invulnerability refusal's inputs (#916), kept as the hit
            // was, for the Environment the pipeline's own rule reads.
            invulnerable: self.invulnerable,
            permanently_invulnerable: self.health.permanently_invulnerable,
            invulnerable_time: self.health.invulnerable_time > 0,
            creative_source,
            blocking,
            blocked: 0.0,
            shield_damage: 0,
            blocking_item_used: false,
            fully_blocked: false,
            block_sound: false,
            knocked: false,
            canceled: false,
            dealt: 0.0,
        };
        if self.client_side {
            return Ok(hit);
        }
        hit.source_name = resources
            .damage
            .definition(source)
            .ok_or("unknown resident damage source")?
            .name
            .clone();
        hit.pending = self.in_combat(level, resources, rules, &mut hit, |combat, state| {
            Ok(combat.hurt_until_helmet(state, amount)?)
        })?;
        if let Some(pending) = &mut hit.pending {
            pending.record_shield_damage(hit.shield_damage as f32);
        }
        Ok(hit)
    }
    /// Select the inner hit before synchronous ordinary equipment wear.
    pub fn select_equipment_hurt(
        &mut self,
        level: &mut DamageLevel<'_>,
        resources: &HealthResources,
        rules: HealthRules,
        hit: &mut PausedHurt,
    ) -> Result<Option<f32>> {
        let Some(pending) = hit.pending.take() else {
            return Ok(None);
        };
        hit.selected = self.in_combat(level, resources, rules, hit, |combat, state| {
            Ok(sunrise_world::living_hurt::select_equipment_hurt(
                state, combat, pending,
            )?)
        })?;
        if let Some(selected) = &hit.selected {
            let damage = selected.damage;
            hit.inner = self.in_combat(level, resources, rules, hit, |combat, _| {
                Ok(combat.prepare_equipment_inner()?)
            })?;
            if hit.inner == Some(false) {
                return Ok(Some(damage));
            }
        }
        Ok(None)
    }
    /// The rest of a hit from `hurtHelmet` on, against the player as the helmet's wear left
    /// them: its armor and protection, and whatever a listener changed.
    pub fn finish_hurt(
        &mut self,
        level: &mut DamageLevel<'_>,
        resources: &HealthResources,
        rules: HealthRules,
        mut hit: PausedHurt,
    ) -> Result<BlockOutcome> {
        if self.client_side {
            return Ok(BlockOutcome::default());
        }
        let hurt = if let Some(selected) = hit.selected.take() {
            let inner = hit.inner;
            self.in_combat(level, resources, rules, &mut hit, |combat, state| {
                Ok(combat.finish_selected_equipment(state, selected, inner)?)
            })?
        } else {
            match hit.pending.take() {
                Some(pending) => {
                    self.in_combat(level, resources, rules, &mut hit, |combat, state| {
                        Ok(combat.hurt_after_helmet(state, pending)?)
                    })?
                }
                None => false,
            }
        };
        let h = &mut self.health;
        let outcome = BlockOutcome {
            hurt,
            blocked: hit.blocked,
            shield_damage: hit.shield_damage,
            blocking_item_used: hit.blocking_item_used,
            fully_blocked: hit.fully_blocked,
            sound: hit.block_sound,
            knockback: hit.knocked,
            canceled: hit.canceled,
            dealt: hit.dealt,
        };
        if outcome.hurt {
            h.accepted_hit = if h.health() <= 0.0 {
                let source = level.attack().player_source(&hit.source_name, hit.causing);
                level.accepted_hit(Hurting::Player(self.entity_id, source))
            } else {
                None
            };
        }
        if outcome.hurt {
            h.hurt_animation_time = h.hurt.hurt_time;
        }
        // `dealDefaultKnockback` ends in `indicateDamage` unless blocked: a
        // hit with no direction of its own (`xd` and `zd` zero) is sent as
        // the opposite of the player's yaw. A hit that has a direction
        // replaces this (`mob_hits_player`).
        if outcome.knockback && outcome.blocked <= 0.0 {
            h.indicate_damage(0.0, 0.0, self.yaw);
        }
        let after = h.health() + h.absorption();
        // `LivingEntity.getDamageAfterArmorAbsorb` is the hit's only reader of
        // these two instances, and a source in `#minecraft:bypasses_armor`
        // makes it return before `getArmorValue` and
        // `getAttributeValue(ARMOR_TOUGHNESS)`. Native `getValue` is what
        // clears an instance's dirty flag, so the record takes the caches the
        // hit left instead of recomputing them: one that does meet the armour
        // has already recomputed both in `after_armor`, and one that bypassed
        // it leaves creation's zero cache untouched.
        let armor = self
            .attributes
            .cached_value_of("minecraft:armor")
            .unwrap_or(0.0) as f32;
        let toughness = self
            .attributes
            .cached_value_of("minecraft:armor_toughness")
            .unwrap_or(0.0) as f32;
        if self.hurt_log.len() >= HURT_LOG {
            self.hurt_log.remove(0);
        }
        self.hurt_log.push(HurtRecord {
            game_time: h.game_time,
            source: hit.source_name,
            amount: hit.amount,
            blocked: outcome.blocked,
            taken: hit.before - after,
            hurt: outcome.hurt,
            armor,
            toughness,
            health: after,
            fire_ticks: hit.fire_ticks,
            knockback: None,
        });
        Ok(outcome)
    }
    /// `run` on the player's combat, the hit's own state lent to it and kept for its next half.
    fn in_combat<R>(
        &mut self,
        level: &mut DamageLevel<'_>,
        resources: &HealthResources,
        rules: HealthRules,
        hit: &mut PausedHurt,
        run: impl FnOnce(
            &mut ServerPlayerCombat<'_, Environment<'_>, Attributes>,
            &mut State,
        ) -> Result<R>,
    ) -> Result<R> {
        let source = resources
            .damage
            .source::<()>(hit.source, None, None, None)
            .ok_or("unknown resident damage source")?;
        let victim = self.victim();
        let h = &mut self.health;
        let mut environment = Environment {
            level: level.reborrow(),
            invulnerability_checked: hit.invulnerability_checked,
            can_harm_player: hit.can_harm_player,
            entity: self.entity_id,
            victim,
            victim_motion: (self.fall_distance, self.fall_flying || self.flying),
            causing: hit.causing,
            invulnerable: hit.invulnerable,
            permanently_invulnerable: hit.permanently_invulnerable,
            invulnerable_time: hit.invulnerable_time,
            fire_immune: self.mod_fire_immune,
            creative_source: hit.creative_source,
            client_loaded: self.client_loaded,
            rules,
            source,
            weapon: hit.armor_weapon.as_ref(),
            random: &mut self.random,
            living_cause: hit.living_cause,
            events: &mut self.events,
            dead: &mut h.dead,
            damage_completion: &mut h.damage_completion,
            sync_velocity: &mut h.sync_velocity,
            game_time: h.game_time,
            status: self.status.clone(),
            helmet: hit.helmet,
            blocking: hit.blocking.take(),
            blocked: hit.blocked,
            shield_damage: hit.shield_damage,
            blocking_item_used: hit.blocking_item_used,
            fully_blocked: hit.fully_blocked,
            block_sound: hit.block_sound,
            knocked: hit.knocked,
            protection: None,
            canceled: hit.canceled,
            dealt: hit.dealt,
        };
        let result = run(
            &mut ServerPlayerCombat::new(
                &mut self.attributes,
                &mut h.data,
                &mut h.food,
                h.ids,
                &mut environment,
            )?,
            &mut h.hurt,
        )?;
        hit.invulnerability_checked = environment.invulnerability_checked;
        hit.blocking = environment.blocking;
        hit.blocked = environment.blocked;
        hit.shield_damage = environment.shield_damage;
        hit.blocking_item_used = environment.blocking_item_used;
        hit.fully_blocked = environment.fully_blocked;
        hit.block_sound = environment.block_sound;
        hit.knocked = environment.knocked;
        hit.canceled = environment.canceled;
        hit.dealt = environment.dealt;
        Ok(result)
    }
    pub fn set_health(&mut self, value: f32, absorption: bool) -> Result<()> {
        let h = &mut self.health;
        let mut observer = Updates(&mut self.events);
        let mut binding = PlayerAttributeHealth::new(
            &mut self.attributes,
            &mut h.data,
            h.ids,
            h.dead,
            &mut observer,
        )?;
        if absorption {
            binding.set_absorption(value)?;
        } else {
            binding.set_health(value)?;
        }
        Ok(())
    }
}

impl crate::health::HealthEventSink for Vec<MotionEvent> {
    fn push_health(&mut self, event: HealthEvent) {
        self.push(MotionEvent::Health(event));
    }
    fn push_hurt_sound(&mut self, sound: &'static str) {
        self.push(MotionEvent::HurtSound(sound));
    }
}

impl crate::health::PlayerHitBody for PlayerMotion {
    fn entity_id(&self) -> i32 {
        self.entity_id
    }
    fn position(&self) -> Vec3 {
        self.position
    }
    fn random(&mut self) -> &mut LegacyRandom {
        &mut self.random
    }
    fn health(&self) -> &crate::health::PlayerHealth {
        &self.health
    }
    fn attributes(&self) -> &crate::health::Attributes {
        &self.attributes
    }
}
