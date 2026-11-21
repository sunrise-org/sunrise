//! `living_damage_pre` and `living_damage_post` heard by a trait mod, through the game's own
//! hooks: the damage container each hit fills, the hit's level host, and the mob and dragon
//! hits that fire them.
use super::dragon::{Dragon, SPECIAL};
use super::entity::damage_level_of;
use super::native_entity_refs::tests::{bind_local_player, with_game};
use mod_api::{
    self, HitHost, Hooks, HurtSource, LivingDamagePostEvent, LivingDamagePreEvent,
    LivingIncomingDamageEvent, LivingShieldBlockEvent, Reduction,
};
use std::sync::{Arc, Mutex};
use sunrise_client::command::args::GameMode;
use sunrise_client::items::ItemStack;
use sunrise_client::mobs::{Damage, DamageKind, Mob};
use sunrise_client::motion::PlayerMotion;
use sunrise_core::{geometry::Vec3, random::RandomSource};

/// What the probe heard: Pre's entity, original damage, new damage and armor's reduction, and
/// Post's health damage, then its entity, health damage, absorption reduction and blocked damage.
#[derive(Debug, Default, PartialEq)]
struct Heard {
    pre: Vec<(i32, f32, f32, f32)>,
    post: Vec<(i32, f32)>,
    post_values: Vec<(i32, f32, f32, f32)>,
    sources: Vec<HitSource>,
    weapons: Vec<(Option<String>, Option<String>)>,
    positions: Vec<(&'static str, Option<[f64; 3]>)>,
    side_effects: Vec<(i32, bool)>,
}

#[derive(Debug, PartialEq)]
struct HitSource {
    phase: &'static str,
    entity: i32,
    causing: i32,
    direct: i32,
}

impl Heard {
    fn source(&mut self, phase: &'static str, entity: i32, source: &HurtSource<'_>) {
        self.positions.push((phase, source.position));
        self.sources.push(HitSource {
            phase,
            entity,
            causing: source.causing,
            direct: source.direct,
        });
    }
}

/// The test mod: 225001's incoming hit becomes 4, Pre sets 225003's and 225008's damage to 2,
/// and draws once from the level's random for 225010.
struct Probe(Arc<Mutex<Heard>>);

#[mod_api::__subscriptions]
impl Hooks for Probe {
    fn living_incoming_damage(
        &self,
        _: &mut dyn HitHost,
        event: &mut LivingIncomingDamageEvent<'_>,
    ) -> mod_api::Flow {
        self.0
            .lock()
            .unwrap()
            .source("incoming", event.entity, event.source);
        let item_id = |stack: Option<&mod_api::ItemStack>| {
            stack.and_then(|s| mod_api::__private::item_key_id(s.item()).map(str::to_owned))
        };
        let direct = item_id(event.weapon);
        let causing = item_id(event.causing_weapon);
        if causing.as_deref() == Some("minecraft:iron_sword") {
            event.container.set_new_damage(1.0);
        }
        self.0.lock().unwrap().weapons.push((direct, causing));
        if event.entity == 225020 && event.source.position == Some([0.0, 0.0, -2.0]) {
            event.container.set_new_damage(4.0);
        }
        if event.entity == 225001 {
            event.container.set_new_damage(4.0);
        }
        mod_api::Flow::Continue
    }
    fn living_shield_block(&self, _: &mut dyn HitHost, event: &mut LivingShieldBlockEvent<'_>) {
        if event.entity == 227002 {
            event.block.blocked = true;
            event.block.blocked_damage = event.damage;
        }
    }
    fn living_damage_pre(&self, level: &mut dyn HitHost, event: &mut LivingDamagePreEvent<'_>) {
        let container = &mut *event.container;
        if matches!(event.entity, 225003 | 225008) {
            container.set_new_damage(2.0);
        }
        if event.entity == 225010 {
            level.with_random(|random| random.next_int()).unwrap();
        }
        let heard = (
            event.entity,
            container.original(),
            container.new_damage(),
            container.reduction(Reduction::Armor),
        );
        let mut log = self.0.lock().unwrap();
        log.pre.push(heard);
        log.source("pre", event.entity, event.source);
    }
    fn living_damage_post(&self, _: &mut dyn HitHost, event: &mut LivingDamagePostEvent<'_>) {
        let heard = (event.entity, event.container.health_damage());
        let mut log = self.0.lock().unwrap();
        log.post.push(heard);
        log.post_values.push((
            event.entity,
            event.container.health_damage(),
            event.container.reduction(Reduction::Absorption),
            event.container.blocked(),
        ));
        log.source("post", event.entity, event.source);
        log.side_effects
            .push((event.entity, event.container.should_cause_side_effects()));
    }
}

/// Runs `test` in a game with the probe installed, and returns what it heard.
fn heard(test: impl FnOnce(&mut super::Game<'_>)) -> Heard {
    let log = Arc::new(Mutex::new(Heard::default()));
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "probe",
        Box::new(Probe(log.clone())),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || with_game(test));
    std::mem::take(&mut *log.lock().unwrap())
}

fn mob(game: &super::Game<'_>, id: u32, kind: &str) -> Mob {
    let kind = game.res.mob_types().get(kind).unwrap().clone();
    Mob::new(id, kind, Vec3::ZERO, 0.0)
}

#[test]
#[ignore = "daily"]
fn the_original_damage_is_the_hit_before_the_incoming_hook() {
    let heard = heard(|game| {
        let mut pig = mob(game, 225001, "minecraft:pig");
        assert!(pig.damage(
            damage_level_of!(game, &sunrise_client::health::NO_ATTACK, no_push, None),
            10.0,
            Damage::of(DamageKind::Generic)
        ));
        assert_eq!(pig.health, 6.0);
    });
    assert_eq!(heard.pre, [(225001, 10.0, 4.0, 0.0)]);
    assert_eq!(heard.post, [(225001, 4.0)]);
}

#[test]
#[ignore = "daily"]
fn pre_sets_the_health_lost_and_the_last_hurt_the_cooldown_compares() {
    let heard = heard(|game| {
        let mut pig = mob(game, 225003, "minecraft:pig");
        assert!(pig.damage(
            damage_level_of!(game, &sunrise_client::health::NO_ATTACK, no_push, None),
            6.0,
            Damage::of(DamageKind::Generic)
        ));
        assert_eq!(pig.health, 8.0);
        // The inflicted damage reaches `lastHurt`.
        assert_eq!(pig.last_hurt, 2.0);
    });
    assert_eq!(heard.pre, [(225003, 6.0, 2.0, 0.0)]);
    assert_eq!(heard.post, [(225003, 2.0)]);
}

#[test]
#[ignore = "daily"]
fn pre_fires_for_a_hit_its_reductions_took_to_zero() {
    let heard = heard(|game| {
        let mut witch = mob(game, 225008, "minecraft:witch");
        // Witch.getDamageAfterMagicAbsorb: none from its own doing.
        let own = Damage::mob(225008, [2.0, 0.0, 2.0]);
        assert!(witch.damage(
            damage_level_of!(game, &sunrise_client::health::NO_ATTACK, no_push, None),
            6.0,
            own
        ));
        assert_eq!(witch.health, 24.0);
    });
    assert_eq!(heard.pre, [(225008, 6.0, 2.0, 0.0)]);
}

#[test]
#[ignore = "daily"]
fn pre_draws_from_the_level_random_of_the_hit() {
    heard(|game| {
        let mut expected = game.world.random.clone();
        expected.next_int();
        let mut pig = mob(game, 225010, "minecraft:pig");
        assert!(pig.damage(
            damage_level_of!(game, &sunrise_client::health::NO_ATTACK, no_push, None),
            1.0,
            Damage::of(DamageKind::Generic)
        ));
        assert_eq!(game.world.random.next_int(), expected.next_int());
    });
}

#[test]
#[ignore = "daily"]
fn a_hit_on_the_dragon_fires_pre_and_post() {
    let heard = heard(|game| {
        let mut dragon = Dragon::new(Vec3::ZERO, 0.0, 0);
        dragon.id = 225009;
        game.dragon.replace(dragon);
        let _ = game.hit_special(SPECIAL, 6.0, true, DamageKind::Generic, None);
        assert_eq!(game.dragon.as_ref().unwrap().health, 194.0);
    });
    assert_eq!(heard.pre, [(225009, 6.0, 6.0, 0.0)]);
    assert_eq!(heard.post, [(225009, 6.0)]);
}

#[test]
#[ignore = "daily"]
fn mob_and_dragon_post_records_fresh_hit_decision() {
    let heard = heard(|game| {
        let mut pig = mob(game, 225014, "minecraft:pig");
        let source = Damage::of(DamageKind::Generic);
        assert!(pig.damage(
            damage_level_of!(game, &sunrise_client::health::NO_ATTACK, no_push, None),
            2.0,
            source
        ));
        assert!(pig.damage(
            damage_level_of!(game, &sunrise_client::health::NO_ATTACK, no_push, None),
            4.0,
            source
        ));
        assert!(!pig.damage(
            damage_level_of!(game, &sunrise_client::health::NO_ATTACK, no_push, None),
            3.0,
            source
        ));
        assert_eq!(pig.health, 6.0);
        let mut dragon = Dragon::new(Vec3::ZERO, 0.0, 0);
        dragon.id = 225015;
        game.dragon.replace(dragon);
        for amount in [2.0, 4.0, 3.0] {
            let _ = game.hit_special(SPECIAL, amount, true, DamageKind::Generic, None);
        }
        assert_eq!(game.dragon.as_ref().unwrap().health, 196.0);
    });
    assert_eq!(
        heard.post,
        [(225014, 2.0), (225014, 2.0), (225015, 2.0), (225015, 2.0)]
    );
    assert_eq!(
        heard.side_effects,
        [
            (225014, true),
            (225014, false),
            (225015, true),
            (225015, false)
        ]
    );
}

fn survival_player(game: &mut super::Game<'_>, id: u32) {
    game.player = Some(PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
    bind_local_player(game, id, u128::from(id));
    game.set_game_mode(GameMode::Survival);
}

#[test]
#[ignore = "daily"]
fn post_fires_for_a_player_hit_that_absorption_took_whole() {
    let heard = heard(|game| {
        survival_player(game, 227001);
        // Absorption is cut to `max_absorption`, 0 without a modifier.
        let player = game.player.as_mut().unwrap();
        player
            .set_modifier(
                "minecraft:max_absorption",
                "test:absorption",
                10.0,
                "add_value",
                true,
            )
            .unwrap();
        player.set_health(2.0, true).unwrap();
        assert!(game.hurt_player_from(2.0, None, "minecraft:generic", Some(None), false));
        let health = &game.player.as_ref().unwrap().health;
        assert_eq!((health.health(), health.absorption()), (20.0, 0.0));
    });
    assert_eq!(heard.post_values, [(227001, 0.0, 2.0, 0.0)]);
}

#[test]
#[ignore = "daily"]
fn post_fires_for_a_player_hit_a_shield_blocked_whole() {
    let heard = heard(|game| {
        survival_player(game, 227002);
        game.hurt_player_from(2.0, None, "minecraft:generic", Some(None), false);
        assert_eq!(game.player.as_ref().unwrap().health.health(), 20.0);
    });
    assert_eq!(heard.post_values, [(227002, 0.0, 0.0, 2.0)]);
}

#[test]
#[ignore = "daily"]
fn a_hit_inside_the_cooldown_no_stronger_than_the_last_fires_no_post() {
    let heard = heard(|game| {
        // Pre sets 225003's damage to 2, which `lastHurt` keeps.
        let mut pig = mob(game, 225003, "minecraft:pig");
        let source = Damage::of(DamageKind::Generic);
        assert!(pig.damage(
            damage_level_of!(game, &sunrise_client::health::NO_ATTACK, no_push, None),
            4.0,
            source
        ));
        // Inside the cooldown and no stronger than the last hit: refused, no Post.
        assert!(!pig.damage(
            damage_level_of!(game, &sunrise_client::health::NO_ATTACK, no_push, None),
            2.0,
            source
        ));
    });
    assert_eq!(heard.post, [(225003, 2.0)]);
}

#[test]
#[ignore = "daily"]
fn post_fires_once_for_a_killing_blow_on_a_mob() {
    let heard = heard(|game| {
        let mut pig = mob(game, 225016, "minecraft:pig");
        assert!(pig.damage(
            damage_level_of!(game, &sunrise_client::health::NO_ATTACK, no_push, None),
            100.0,
            Damage::of(DamageKind::Generic)
        ));
        assert_eq!(pig.health, 0.0);
    });
    // `living_death` is settled by the game's next tick, after the hurt and its Post.
    assert_eq!(heard.post, [(225016, 100.0)]);
}

/// What a killing blow tells a mod, in order: `("post", entity)` and `("death", entity)`.
#[derive(Default)]
struct DeathOrder {
    heard: Mutex<Vec<(&'static str, i64)>>,
    cancel: std::sync::atomic::AtomicBool,
}

struct DeathProbe(Arc<DeathOrder>);

#[mod_api::__subscriptions]
impl Hooks for DeathProbe {
    fn living_damage_post(&self, _: &mut dyn HitHost, event: &mut LivingDamagePostEvent<'_>) {
        let entity = i64::from(event.entity);
        self.0.heard.lock().unwrap().push(("post", entity));
    }
    fn living_death(
        &self,
        _: &mut dyn mod_api::ServerHost,
        event: &mut mod_api::LivingDeathEvent<'_>,
    ) -> mod_api::Flow {
        let entity = i64::from(event.entity);
        self.0.heard.lock().unwrap().push(("death", entity));
        if self.0.cancel.load(std::sync::atomic::Ordering::Relaxed) {
            mod_api::Flow::Cancel
        } else {
            mod_api::Flow::Continue
        }
    }
}

/// A pig killed by a 100-damage hit through the game, then the creature tick that settles its
/// death and one more: what the probe heard, and the pig's health after.
fn killing_blow(cancel: bool) -> (Vec<(&'static str, i64)>, f32) {
    let probe = Arc::new(DeathOrder::default());
    probe
        .cancel
        .store(cancel, std::sync::atomic::Ordering::Relaxed);
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "death-order",
        Box::new(DeathProbe(probe.clone())),
    )]
    .into();
    let mut health = f32::NAN;
    mod_platform::testing::with_mods(mods, || {
        with_game(|game| {
            game.build_registries();
            game.world
                .chunks
                .insert((0, 0), Arc::new(sunrise_client::chunk::Chunk::empty(0, 0)));
            let pig = mob(game, 225017, "minecraft:pig");
            game.mobs.push(pig);
            let victim = game.entity(225017).unwrap();
            assert_eq!(
                victim.hurt(game, "minecraft:magic", 100.0, None, None),
                Some(true)
            );
            game.tick_creature_entities().unwrap();
            game.tick_creature_entities().unwrap();
            health = game.mobs.iter().find(|m| m.id == 225017).unwrap().health;
        });
    });
    let heard = std::mem::take(&mut *probe.heard.lock().unwrap());
    (heard, health)
}

/// Sunrise's order, not NeoForge's (whose Post follows the death processing): a killing blow's
/// Post fires once, then the death hooks, as the maintainers ruled on #3963.
#[test]
#[ignore = "daily"]
fn a_killing_blows_post_fires_once_before_the_death_hooks() {
    assert_eq!(
        killing_blow(false),
        (vec![("post", 225017), ("death", 225017)], 0.0)
    );
}

/// A death a mod cancels: the pig remains cancelled at 0 health with `death_canceled`,
/// without a second Post or death on the next tick.
#[test]
#[ignore = "daily"]
fn a_cancelled_deaths_post_fires_once_before_the_death_hooks() {
    assert_eq!(
        killing_blow(true),
        (vec![("post", 225017), ("death", 225017)], 0.0)
    );
}

#[test]
#[ignore = "daily"]
fn the_container_names_what_armor_took() {
    let heard = heard(|game| {
        let mut zombie = mob(game, 225011, "minecraft:zombie");
        // A mob's attack: armor takes its share, where a generic hit bypasses it.
        let attack = Damage::mob(225012, [2.0, 0.0, 2.0]);
        assert!(zombie.damage(
            damage_level_of!(game, &sunrise_client::health::NO_ATTACK, no_push, None),
            10.0,
            attack
        ));
    });
    // A zombie's two armor points against 10 damage: `CombatRules.getDamageAfterAbsorb`
    // clamps 2 - 10 / 2 up to a fifth of the armor, 0.4, and keeps 1 - 0.4 / 25 of the hit.
    let after = 10.0 * (1.0 - 0.4_f32 / 25.0);
    assert_eq!(heard.pre, [(225011, 10.0, after, 10.0 - after)]);
}

/// A level built the normal way lends the game's hooks to its hits: a path that built it
/// without them would run no mod's damage hook.
#[test]
#[ignore = "daily"]
fn a_game_built_level_has_every_damage_hook() {
    with_game(|game| {
        let hooks = &game.damage_hooks;
        assert!(hooks.incoming.is_some() && hooks.pre.is_some() && hooks.post.is_some());
        assert!(hooks.knockback.is_some() && hooks.totem.is_some() && hooks.shield.is_some());
    });
}

#[test]
#[ignore = "daily"]
fn player_melee_hit_preserves_the_attackers_position_and_direct_entity() {
    let heard = heard(|game| {
        let mut player = PlayerMotion::new(game.motion, Vec3::ZERO).unwrap();
        player.entity_id = 225020;
        game.player = Some(player);
        game.set_game_mode(GameMode::Survival);
        let mut zombie = mob(game, 42, "minecraft:zombie");
        zombie.position = Vec3::new(0.0, 0.0, -2.0);
        game.mobs.push(zombie);
        assert!(game.hurt_player_from(
            8.0,
            Some([0.0, 0.0, -2.0]),
            "minecraft:mob_attack",
            Some(Some(42)),
            false,
        ));
        assert_eq!(
            game.player.as_ref().unwrap().health.health(),
            16.0,
            "a position-based listener changes actual player damage"
        );
    });
    assert_eq!(
        heard.sources,
        ["incoming", "pre", "post"].map(|phase| HitSource {
            phase,
            entity: 225020,
            causing: 42,
            direct: 42,
        })
    );
    assert_eq!(
        heard.positions,
        ["incoming", "pre", "post"].map(|phase| (phase, Some([0.0, 0.0, -2.0])))
    );
}

/// The real projectile hit routes (not hand-built HurtSources) preserve the direct
/// entity through incoming, pre and post, independently of whether there is a shooter.
#[test]
#[ignore = "daily"]
fn arrow_hits_name_the_projectile_on_players_mobs_and_the_dragon() {
    use sunrise_client::command::args::GameMode;
    use sunrise_client::health::NO_ATTACK;
    use sunrise_client::mobs::Who;
    use sunrise_client::motion::PlayerMotion;
    use sunrise_client::projectiles::{Hit, Projectile, ProjectileKind};

    for (target, owner, gone, entity, causing) in [
        (Who::Player, Some(Who::Mob(42)), false, 7, 42),
        (Who::Player, None, false, 7, 0),
        (Who::Mob(43), Some(Who::Player), false, 43, 7),
        (Who::Mob(43), None, false, 43, 0),
        (Who::Mob(SPECIAL), Some(Who::Player), false, 44, 7),
        (Who::Mob(43), Some(Who::Mob(42)), true, 43, 42),
    ] {
        let heard = heard(|game| {
            let mut player = PlayerMotion::new(game.motion, Vec3::ZERO).unwrap();
            player.entity_id = 7;
            game.player = Some(player);
            game.set_game_mode(GameMode::Survival);
            game.mobs.push(mob(game, 42, "minecraft:skeleton"));
            game.mobs[0]
                .extra
                .set_main_hand_stack(Some(ItemStack::new("minecraft:stone_sword", 1)));
            *game.held_mut() = Some(ItemStack::new("minecraft:stone_sword", 1));
            game.offhand = Some(ItemStack::new("minecraft:crossbow", 1));
            game.mobs.push(mob(game, 43, "minecraft:pig"));
            let mut dragon = Dragon::new(Vec3::ZERO, 0.0, 0);
            // Hover is a sitting phase and correctly shrugs off arrows.
            dragon.set_native_phase(mod_api::DragonPhase::HoldingPattern);
            dragon.id = 44;
            game.dragon.replace(dragon);
            let mut arrow = Projectile::new(
                ProjectileKind::Arrow {
                    base_damage: 2.0,
                    critical: false,
                    effect: None,
                },
                Vec3::new(0.0, 1.0, 0.5),
                owner,
            );
            arrow.id = 912;
            arrow.arrow.as_mut().unwrap().fired_from_weapon =
                Some(ItemStack::new("minecraft:crossbow", 1));
            arrow.velocity = Vec3::new(0.0, 0.0, -2.0);
            // Equipment changes after launch: only the saved crossbow belongs to the arrow.
            *game.held_mut() = Some(ItemStack::new("minecraft:iron_sword", 1));
            game.mobs[0]
                .extra
                .set_main_hand_stack(Some(ItemStack::new("minecraft:iron_sword", 1)));
            if gone {
                game.mobs.retain(|m| m.id != 42);
            }
            game.projectile_hit(
                &arrow,
                Hit::Entity {
                    who: target,
                    at: Vec3::ZERO,
                },
            )
            .unwrap();
            assert_eq!(game.current_attack(), NO_ATTACK, "projectile scope leaked");
            // The listener sets 1 damage: 20 - 1 for a player, 10 - 1 for a pig.
            match target {
                Who::Player => {
                    let health = game.player.as_ref().unwrap().health.health();
                    if owner.is_some() && !gone {
                        assert_eq!(health, 19.0, "the causing-weapon rule changes real damage");
                    } else {
                        assert!(health < 19.0, "ownerless arrows do not use the weapon rule");
                    }
                }
                Who::Mob(SPECIAL) => assert!(game.dragon.as_ref().unwrap().health < 200.0),
                Who::Mob(_) => {
                    if owner.is_some() && !gone {
                        assert_eq!(game.mobs.iter().find(|m| m.id == 43).unwrap().health, 9.0);
                    } else {
                        assert!(game.mobs.iter().find(|m| m.id == 43).unwrap().health < 9.0);
                    }
                }
            }
        });
        let expected = ["incoming", "pre", "post"].map(|phase| HitSource {
            phase,
            entity,
            causing,
            direct: 912,
        });
        assert_eq!(
            heard.sources, expected,
            "target {target:?}, owner {owner:?}"
        );
        assert_eq!(
            heard.weapons,
            [(
                Some("minecraft:crossbow".into()),
                owner
                    .filter(|_| !gone)
                    .map(|_| "minecraft:iron_sword".into())
            )],
            "the saved projectile weapon and the current shooter's main hand are independent"
        );
        if target == Who::Player {
            assert_eq!(
                heard.positions,
                ["incoming", "pre", "post"].map(|phase| (phase, Some([0.0, 1.0, 0.5]))),
                "known projectile position reaches every player hit phase; owner {owner:?}"
            );
        }
    }
}

#[test]
#[ignore = "daily"]
fn guest_projectile_uses_current_main_hand_or_none_after_departure() {
    use sunrise_client::mobs::Who;
    use sunrise_client::projectiles::{Hit, Projectile, ProjectileKind};

    for gone in [false, true] {
        let log = Arc::new(Mutex::new(Heard::default()));
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
            "probe",
            Box::new(Probe(log.clone())),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            super::native_guest_turn_tests::with_guests(|game| {
                game.lan
                    .as_mut()
                    .unwrap()
                    .hub
                    .update_guest(12, |guest| {
                        guest.selected = 3;
                        guest.inventory[3] = Some(ItemStack::new("minecraft:stone_sword", 1));
                    })
                    .unwrap();
                game.mobs.push(mob(game, 43, "minecraft:pig"));
                let mut arrow = Projectile::new(
                    ProjectileKind::Arrow {
                        base_damage: 2.0,
                        critical: false,
                        effect: None,
                    },
                    Vec3::new(0.0, 1.0, 0.5),
                    Some(Who::Player),
                );
                arrow.id = 912;
                arrow.guest = Some(12);
                arrow.arrow.as_mut().unwrap().fired_from_weapon =
                    Some(ItemStack::new("minecraft:crossbow", 1));
                arrow.velocity = Vec3::new(0.0, 0.0, -2.0);
                game.lan
                    .as_mut()
                    .unwrap()
                    .hub
                    .update_guest(12, |guest| {
                        guest.inventory[3] = Some(ItemStack::new("minecraft:iron_sword", 1));
                    })
                    .unwrap();
                if gone {
                    // Model the departed connection being absent when its projectile lands.
                    game.lan
                        .as_mut()
                        .unwrap()
                        .hub
                        .guests
                        .retain(|guest| guest.conn != 12);
                }
                game.projectile_hit(
                    &arrow,
                    Hit::Entity {
                        who: Who::Mob(43),
                        at: Vec3::ZERO,
                    },
                )
                .unwrap();
                let health = game.mobs.iter().find(|m| m.id == 43).unwrap().health;
                if gone {
                    assert!(health < 9.0);
                } else {
                    assert_eq!(health, 9.0, "10 health minus the listener's 1 damage");
                }
            });
        });
        assert_eq!(
            log.lock().unwrap().weapons,
            [(
                Some("minecraft:crossbow".into()),
                (!gone).then(|| "minecraft:iron_sword".into())
            )]
        );
    }
}

#[test]
#[ignore = "daily"]
fn snapshot_crosses_the_real_damage_hook_with_reductions() {
    let log = heard(|game| {
        let mut container = sunrise_world::living_damage::DamageContainer::new(10.0);
        container.record_reduction(sunrise_world::living_damage::Reduction::Armor, 3.5);
        container.enter_pre(6.5);
        damage_level_of!(game, &sunrise_client::health::NO_ATTACK, no_push, None).damage_pre(
            sunrise_client::health::Hurting::Player(
                225003,
                HurtSource {
                    damage_type: "minecraft:generic",
                    causing: 0,
                    direct: 0,
                    position: None,
                },
            ),
            &mut container,
        );
        assert_eq!(container.capture_inflicted(), 2.0);
        assert_eq!(
            container.reduction(sunrise_world::living_damage::Reduction::Armor),
            3.5
        );
    });
    assert_eq!(log.pre, [(225003, 10.0, 2.0, 3.5)]);
}

#[test]
#[ignore = "daily"]
fn unsubscribed_damage_does_not_convert() {
    mod_platform::testing::with_mods(Vec::<mod_api::ModEntry>::new().into(), || {
        with_game(|game| {
            super::api_values::damage::CONVERSIONS.with(|count| count.set(0));
            let mut container = sunrise_world::living_damage::DamageContainer::new(10.0);
            damage_level_of!(game, &sunrise_client::health::NO_ATTACK, no_push, None).damage_pre(
                sunrise_client::health::Hurting::Player(
                    225003,
                    HurtSource {
                        damage_type: "minecraft:generic",
                        causing: 0,
                        direct: 0,
                        position: None,
                    },
                ),
                &mut container,
            );
            super::api_values::damage::CONVERSIONS.with(|count| assert_eq!(count.get(), 0));
        })
    });
}
