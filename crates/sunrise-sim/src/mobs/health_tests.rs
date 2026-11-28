//! The damage engine's tests, beside the mob body they build.

use crate::health::*;
#[test]
fn a_hook_can_refuse_one_damage_type_and_allow_another() {
    use crate::mobs::{Damage, DamageKind, Mob};
    fn immune_to_generic(_: DamageLevel<'_>, hurting: Hurting<'_>, vanilla: bool) -> bool {
        match hurting {
            Hurting::Mob(_, hit) if matches!(hit.kind, DamageKind::Generic) => true,
            _ => vanilla,
        }
    }
    let hooks = DamageHooks {
        invulnerable: Some(immune_to_generic),
        ..DamageHooks::NONE
    };
    let mut test_level = TestLevel::with_hooks(0, hooks);
    let level = &mut test_level.level();
    let mut mob = Mob::new(
        9201,
        crate::mobs::sensor_scan_tests::kind(),
        Default::default(),
        0.0,
    );
    assert!(!mob.damage(level, 2.0, Damage::of(DamageKind::Generic)));
    assert_eq!(mob.health, 10.0);
    assert!(mob.damage(level, 2.0, Damage::of(DamageKind::Fall)));
    assert_eq!(mob.health, 8.0);
}

#[test]
fn the_weapon_is_the_attackers_main_hand_only_for_its_own_hit() {
    let sword = crate::items::ItemStack::new("minecraft:iron_sword", 1);
    let attack = Attack {
        attacker: 7,
        projectile: None,
        direct: None,
        player_uuid: None,
        attacker_uuid: None,
        held: [Some(sword.clone()), None],
        ..Default::default()
    };
    assert_eq!(attack.weapon_of(7), Some(&sword));
    // An arrow the attacker shot is the direct entity, not the attacker: no weapon.
    assert_eq!(attack.weapon_of(9), None);
    assert_eq!(attack.weapon_of(0), None);
    assert_eq!(NO_ATTACK.weapon_of(0), None);
}
#[test]
fn magic_during_a_projectile_impact_names_no_direct_entity() {
    // DamageSources.magic() is entity-free. An ownerless wither skull and a
    // tipped arrow's instant harm both deal it while the projectile's attack
    // is still in scope; arrow, wither_skull and indirect_magic name it.
    let attack = Attack {
        attacker: 0,
        projectile: Some(12),
        direct: None,
        player_uuid: None,
        attacker_uuid: None,
        held: [None, None],
        ..Default::default()
    };
    assert_eq!(attack.projectile_direct("minecraft:arrow"), 12);
    assert_eq!(attack.projectile_direct("minecraft:wither_skull"), 12);
    assert_eq!(attack.projectile_direct("minecraft:spit"), 12);
    assert_eq!(attack.projectile_direct("minecraft:indirect_magic"), 12);
    assert_eq!(attack.projectile_direct("minecraft:magic"), 0);
    assert_eq!(attack.projectile_direct("minecraft:explosion"), 0);
    assert_eq!(NO_ATTACK.projectile_direct("minecraft:arrow"), 0);
}
#[test]
fn a_knockback_hook_cannot_make_the_push_nonfinite() {
    fn hook(_: DamageLevel<'_>, _: Hurting<'_>, knock: &mut Knock) -> bool {
        knock.strength = f64::NAN;
        knock.x *= 2.0;
        false
    }
    let mut level = crate::health::TestLevel::with_hooks(
        0,
        DamageHooks {
            knockback: Some(hook),
            ..DamageHooks::NONE
        },
    );
    let source = HurtSource {
        damage_type: "minecraft:generic",
        causing: 0,
        direct: 0,
        position: None,
        direct_type: None,
    };
    let mut knock = Knock {
        strength: 0.4,
        x: 1.0,
        z: 0.5,
    };
    assert!(
        !level
            .level()
            .knockback_canceled(Hurting::Player(1, source), &mut knock)
    );
    // A value left that is not a number is put back; one that is finite stays changed.
    assert_eq!(
        knock,
        Knock {
            strength: 0.4,
            x: 2.0,
            z: 0.5
        }
    );
}
/// What the hook of these tests saw: entity, damage type (a mob's kind)
/// and causing entity, amount.
static SEEN: std::sync::Mutex<Vec<(i32, String, f32)>> = std::sync::Mutex::new(Vec::new());
/// Sets the native damage hook of these tests (global: it stays set for
/// the process): for entities 9001 to 9004 and 9011 to 9013 it records the
/// hit, cancels x1's, doubles x2's, and gives 9005's hit armor and absorption modifiers.
fn set_hook() -> (crate::health::DamageHooks, fn(i32) -> Vec<(String, f32)>) {
    use crate::health::{DamageContainer, DamageLevel, Hurting, Victim};
    fn hook(
        _: DamageLevel<'_>,
        hurting: Hurting<'_>,
        _: Victim,
        container: &mut DamageContainer,
    ) -> bool {
        let (entity, seen) = match hurting {
            Hurting::Player(e, s)
            | Hurting::Dragon(e, s)
            | Hurting::ArmorStand(e, s)
            | Hurting::Entity(e, _, s) => (e, format!("{} {}", s.damage_type, s.causing)),
            Hurting::Mob(mob, source) => (mob.id as i32, format!("{:?}", source.kind)),
        };
        if !matches!(entity, 9001..=9005 | 9011..=9013) {
            return false;
        }
        let amount = container.new_damage();
        SEEN.lock().unwrap().push((entity, seen, amount));
        match entity {
            9002 | 9012 => container.record_new_damage(amount * 2.0),
            9005 => {
                use sunrise_world::living_damage::Reduction;
                container.add_reduction_modifier(
                    Reduction::Armor,
                    Box::new(|_: &DamageContainer, v: f32| v + 1.0),
                );
                container.add_reduction_modifier(
                    Reduction::Armor,
                    Box::new(|_: &DamageContainer, v: f32| v * 2.0),
                );
                container.add_reduction_modifier(
                    Reduction::Absorption,
                    Box::new(|_: &DamageContainer, v: f32| v * 0.5),
                );
            }
            _ => {}
        }
        entity % 10 == 1
    }
    let hooks = crate::health::DamageHooks {
        incoming: Some(hook),
        ..crate::health::DamageHooks::NONE
    };
    (hooks, |entity| {
        let seen = SEEN.lock().unwrap();
        let hits = seen.iter().filter(|s| s.0 == entity);
        hits.map(|s| (s.1.clone(), s.2)).collect()
    })
}

#[test]
fn the_native_damage_hook_changes_or_cancels_a_mobs_hit() {
    let (hooks, seen) = set_hook();
    let mut test_level = crate::health::TestLevel::with_hooks(0, hooks);
    let level = &mut test_level.level();
    use crate::mobs::{Damage, DamageKind, Mob};
    let mob = |id| {
        Mob::new(
            id,
            crate::mobs::sensor_scan_tests::kind(),
            Default::default(),
            0.0,
        )
    };
    let hit = Damage {
        direct: None,
        kind: DamageKind::PlayerAttack,
        attacker: None,
        from: Some([3.0, 0.0, 0.0]),
        attacker_species: None,
    };
    // Canceled: no damage, cooldown, sound or knockback.
    let mut kept = mob(9001);
    assert!(!kept.damage(level, 2.0, hit));
    assert_eq!(
        (kept.health, kept.invulnerable, kept.hurt_time),
        (10.0, 0, 0)
    );
    assert!(kept.outbox.is_empty() && kept.velocity == Default::default());
    assert_eq!(seen(9001), [("PlayerAttack".to_owned(), 2.0)]);
    let mut doubled = mob(9002);
    assert!(doubled.damage(level, 2.0, hit));
    assert_eq!(doubled.health, 6.0);
    // 8 damage on a mob with 1.5 absorption hearts. Armor took none: +1 then x2 is 2, so 6
    // goes on. The hearts would take their 1.5; half of it is 0.75, so 5.25 reaches the
    // health (10 - 5.25) and 0.75 hearts are left.
    let mut modified = mob(9005);
    modified.add_effect("absorption", 600, 0);
    modified.absorption = 1.5;
    assert!(modified.damage(level, 8.0, hit));
    assert_eq!((modified.health, modified.absorption), (4.75, 0.75));
    // The same hit with no modifier: 8 less the 1.5 hearts.
    let mut plain = mob(9006);
    plain.add_effect("absorption", 600, 0);
    plain.absorption = 1.5;
    assert!(plain.damage(level, 8.0, hit));
    assert_eq!((plain.health, plain.absorption), (3.5, 0.0));
}

/// What the knockback hook of these tests saw: the entity, the hit that
/// causes it (a mob's damage kind) and the push it was handed.
static PUSHES: std::sync::Mutex<Vec<(i32, String, crate::health::Knock)>> =
    std::sync::Mutex::new(Vec::new());
/// Sets the native knockback hook of these tests for mobs 9101 to 9106 only: it records the push, cancels
/// 9101's, doubles 9102's strength and turns its direction around, leaves
/// 9103 non-finite numbers, and gives 9105 (whose resistance leaves
/// nothing) a strength of 0.4.
fn set_knockback_hook() -> (
    crate::health::DamageHooks,
    fn(i32) -> Vec<(String, crate::health::Knock)>,
) {
    use crate::health::{DamageLevel, Hurting, Knock};
    fn hook(_: DamageLevel<'_>, pushed: Hurting<'_>, knock: &mut Knock) -> bool {
        let Hurting::Mob(mob, source) = pushed else {
            return false;
        };
        let entity = mob.id as i32;
        if !(9101..=9106).contains(&entity) {
            return false;
        }
        let hit = format!("{:?}", source.kind);
        PUSHES.lock().unwrap().push((entity, hit, *knock));
        match entity {
            9101 => return true,
            9102 => {
                knock.strength *= 2.0;
                knock.x = -knock.x;
            }
            9103 => {
                knock.strength = f64::NAN;
                knock.x = f64::INFINITY;
                knock.z = f64::NEG_INFINITY;
            }
            9105 => knock.strength = 0.4,
            _ => {}
        }
        false
    }
    let hooks = crate::health::DamageHooks {
        knockback: Some(hook),
        ..crate::health::DamageHooks::NONE
    };
    (hooks, |entity| {
        let seen = PUSHES.lock().unwrap();
        let pushes = seen.iter().filter(|p| p.0 == entity);
        pushes.map(|p| (p.1.clone(), p.2)).collect()
    })
}

/// A zombie in full netherite (0.1 a piece) is pushed 60 % as far as one
/// with no armour: `knockback` scales by the attribute's value with the
/// gear's modifiers, which the game hands the mob as `gear_knockback`.
#[test]
fn a_mobs_gear_resists_knockback() {
    use crate::mobs::{Damage, Mob};
    let mut test_level = crate::health::TestLevel::new(0);
    let level = &mut test_level.level();
    let hit = Damage::player([3.0, 0.0, 0.0]);
    let mut pushed = |gear: Option<f64>, own: f64| {
        let mut m = Mob::new(
            9200,
            crate::mobs::sensor_scan_tests::kind(),
            Default::default(),
            0.0,
        );
        m.ground.on_ground = true;
        m.attrs.knockback_resistance = own;
        m.extra.gear_knockback = gear.map(|amount| {
            Box::new(crate::mobs::GearKnockback {
                worn: Default::default(),
                modifiers: vec![(amount, crate::mobs::finalize::Op::AddValue)],
            })
        });
        m.knockback(level, 0.4, 3.0, 0.0, &hit, 0.0);
        m.velocity.x
    };
    assert_eq!(pushed(None, 0.0), -0.4);
    assert_eq!(pushed(None, 0.5), -0.2);
    assert!((pushed(Some(0.4), 0.0) - -0.4 * 0.6).abs() < 1e-12);
}
/// `LivingEntity.knockback`: the hook runs before knockback resistance
/// takes its share and before motion changes; a mod's strength or
/// direction is what pushes, a cancel pushes nothing, and with no mod in
/// the way the push is vanilla's. Expected values are worked out by hand
/// from vanilla's formula: the horizontal speed halves and the push is
/// subtracted along the normalized direction; on the ground the vertical
/// speed becomes `min(0.4, vy / 2 + strength)`.
#[test]
fn the_native_knockback_hook_changes_or_cancels_a_mobs_push() {
    let (hooks, seen) = set_knockback_hook();
    let mut test_level = crate::health::TestLevel::with_hooks(0, hooks);
    let level = &mut test_level.level();
    use crate::health::Knock;
    use crate::mobs::{Damage, Mob};
    let mob = |id, resistance| {
        let mut m = Mob::new(
            id,
            crate::mobs::sensor_scan_tests::kind(),
            Default::default(),
            0.0,
        );
        m.ground.on_ground = true;
        m.attrs.knockback_resistance = resistance;
        m
    };
    let velocity = |m: &Mob| (m.velocity.x, m.velocity.y, m.velocity.z);
    let hit = Damage::player([3.0, 0.0, 0.0]);
    let mut push = |id, resistance| {
        let mut m = mob(id, resistance);
        m.knockback(level, 0.4, 3.0, 0.0, &hit, 0.0);
        m
    };
    // No mod in the way (the hook leaves 9100 alone): vanilla's push, away from x = 3.
    let plain = push(9100, 0.0);
    assert_eq!(velocity(&plain), (-0.4, 0.4, 0.0));
    assert!(plain.needs_sync);
    // Canceled: no motion, no resync. The hook saw the push the game made.
    let kept = push(9101, 0.0);
    assert_eq!(velocity(&kept), (0.0, 0.0, 0.0));
    assert!(!kept.needs_sync);
    let made = Knock {
        strength: 0.4,
        x: 3.0,
        z: 0.0,
    };
    assert_eq!(seen(9101), [("PlayerAttack".to_owned(), made)]);
    // A stronger push the other way: twice the strength, x turned around.
    let turned = push(9102, 0.0);
    assert_eq!(velocity(&turned), (0.8, 0.4, 0.0));
    assert_eq!(velocity(&push(9102, 0.5)), (0.4, 0.4, 0.0));
    // Numbers that are not finite are put back as the game made them.
    assert_eq!(velocity(&push(9103, 0.0)), (-0.4, 0.4, 0.0));
    // Before the resistance (`LivingKnockBackEvent`): the hook is handed the raw 0.4, and
    // the push that follows is halved.
    let resisted = push(9104, 0.5);
    assert_eq!(velocity(&resisted), (-0.2, 0.2, 0.0));
    assert_eq!(seen(9104)[0].1.strength, 0.4);
    // A full resistance takes all of what the hook leaves: no push.
    let stopped = push(9105, 1.0);
    assert_eq!(seen(9105)[0].1.strength, 0.4);
    assert_eq!(velocity(&stopped), (0.0, 0.0, 0.0));
    // The game's own hit on a mob reaches the hook with its damage.
    let mut hurt = mob(9106, 0.0);
    assert!(hurt.damage(level, 2.0, hit));
    // dealDefaultKnockback pushes with the float 0.4F.
    let strength = f64::from(0.4_f32);
    assert_eq!(velocity(&hurt), (-strength, 0.4, 0.0));
    let made = Knock { strength, ..made };
    assert_eq!(seen(9106), [("PlayerAttack".to_owned(), made)]);
}

#[test]
fn hurt_sounds_follow_damage_effects() {
    // DamageEffects: HURT, THORNS, DROWNING, BURNING, POKING, FREEZING.
    for (effects, sound) in [
        ("hurt", "minecraft:entity.player.hurt"),
        ("thorns", "minecraft:entity.player.hurt"),
        ("drowning", "minecraft:entity.player.hurt_drown"),
        ("burning", "minecraft:entity.player.hurt_on_fire"),
        ("poking", "minecraft:entity.player.hurt_sweet_berry_bush"),
        ("freezing", "minecraft:entity.player.hurt_freeze"),
    ] {
        assert_eq!(crate::health::hurt_sound(effects), sound, "{effects}");
    }
}

#[test]
fn resolve_damage_kind_maps_builtins_and_rejects_unknown_without_catalog() {
    use crate::mobs::DamageKind;
    assert_eq!(
        resolve_damage_kind(None, "minecraft:in_fire"),
        Some(DamageKind::InFire)
    );
    assert_eq!(
        resolve_damage_kind(None, "generic"),
        Some(DamageKind::Generic)
    );
    assert_eq!(resolve_damage_kind(None, "mod:unknown"), None);
}
