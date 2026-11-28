//! Player hurt-path tests that drive a real `PlayerMotion`: the cooldown, the Post's
//! freshness decision, difficulty scaling, shields, arrows, the native hooks, the void
//! and the armor instances a hit leaves behind.
use super::PlayerMotion;
use crate::health::{self, *};

/// What the hook of these tests saw: entity, damage type (a mob's kind)
/// and causing entity, amount.
static SEEN: std::sync::Mutex<Vec<(i32, String, f32)>> = std::sync::Mutex::new(Vec::new());
/// The native damage hook these tests install on their own `TestLevel`: each test builds a
/// fresh level and lends it this hook, so the hook is scoped to that level, not process-wide
/// (only [`SEEN`] is a process-wide record). For entities 9001 to 9004 and 9011 to 9013 it
/// records the hit, cancels x1's, doubles x2's, and gives 9005's hit armor and absorption
/// modifiers.
fn set_hook() -> (health::DamageHooks, fn(i32) -> Vec<(String, f32)>) {
    use health::{DamageContainer, DamageLevel, Hurting, Victim};
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
    let hooks = health::DamageHooks {
        incoming: Some(hook),
        ..health::DamageHooks::NONE
    };
    (hooks, |entity| {
        let seen = SEEN.lock().unwrap();
        let hits = seen.iter().filter(|s| s.0 == entity);
        hits.map(|s| (s.1.clone(), s.2)).collect()
    })
}

#[test]
#[ignore = "daily"]
fn a_hit_in_the_hurt_cooldown_knocks_nobody_back() {
    let mut test_level = crate::health::TestLevel::new(0);
    let level = &mut test_level.level();
    let root = crate::prepared_test_root::prepared_root(&[]);
    let assets = crate::assets::Assets::load(&root).unwrap();
    let resources = crate::motion::MotionResources::load(&root, &assets).unwrap();
    let mut player =
        crate::motion::PlayerMotion::new(&resources, sunrise_core::geometry::Vec3::ZERO);
    let (health, rules) = (&resources.health, health::HealthRules::default());
    let id = health.damage.id("minecraft:mob_attack").unwrap();
    let mut hit = |amount| {
        player
            .hurt_blocking(level, health, rules, id, amount, None)
            .unwrap()
    };
    let first = hit(1.0);
    assert!(first.hurt && first.knockback);
    // A stronger hit inside the cooldown hurts by the difference only
    // (LivingEntity.hurtServer), with no dealDefaultKnockback.
    let second = hit(2.0);
    assert!(second.hurt && !second.knockback);
    // A weaker one doesn't hurt at all.
    let third = hit(1.5);
    assert!(!third.hurt && !third.knockback);
}

#[test]
#[ignore = "daily"]
fn player_post_records_fresh_hit_decision() {
    struct Observed {
        should_cause_side_effects: bool,
        health_damage: f32,
        post_attack_invulnerability_ticks: i32,
        invulnerability_reduction: f32,
    }
    static SEEN: std::sync::Mutex<Vec<Observed>> = std::sync::Mutex::new(Vec::new());
    fn post(
        _: health::DamageLevel<'_>,
        _: health::Hurting<'_>,
        container: &health::DamageContainer,
    ) {
        SEEN.lock().unwrap().push(Observed {
            should_cause_side_effects: container.should_cause_side_effects(),
            health_damage: container.health_damage(),
            post_attack_invulnerability_ticks: container.post_attack_invulnerability_ticks(),
            invulnerability_reduction: container.reduction(health::Reduction::Invulnerability),
        });
    }
    let root = crate::prepared_test_root::prepared_root(&[]);
    let assets = crate::assets::Assets::load(&root).unwrap();
    let resources = crate::motion::MotionResources::load(&root, &assets).unwrap();
    let mut player = crate::motion::PlayerMotion::new(&resources, Default::default());
    let hooks = health::DamageHooks {
        post: Some(post),
        ..health::DamageHooks::NONE
    };
    let mut level = health::TestLevel::with_hooks(0, hooks);
    let health = &resources.health;
    let rules = health::HealthRules::default();
    let attack = health.damage.id("minecraft:mob_attack").unwrap();
    assert!(
        player
            .hurt(&mut level.level(), health, rules, attack, 2.0)
            .unwrap()
    );
    assert!(
        player
            .hurt(&mut level.level(), health, rules, attack, 4.0)
            .unwrap()
    );
    assert!(
        !player
            .hurt(&mut level.level(), health, rules, attack, 3.0)
            .unwrap()
    );
    let seen = SEEN.lock().unwrap();
    assert_eq!(seen.len(), 2, "the refused hit must not fire Post");
    let flags: Vec<_> = seen
        .iter()
        .map(|hit| hit.should_cause_side_effects)
        .collect();
    assert_eq!(flags, [true, false]);
    let invuln: Vec<_> = seen
        .iter()
        .map(|hit| hit.invulnerability_reduction)
        .collect();
    assert_eq!(invuln, [0.0, 2.0]);
    // Both first hits deal 2 and leave 20 ticks: neither value determines freshness.
    for hit in &seen[..2] {
        assert_eq!(hit.health_damage, 2.0);
        assert_eq!(hit.post_attack_invulnerability_ticks, 20);
    }
}

/// #2800: on Easy a mob's 4 is scaled to 3 before `applyItemBlocking`,
/// so a shield that stops all 3 blocks fully (`damageBlocked >= damage`)
/// and `blockedByItem` must not pull the player toward the mob.
#[test]
#[ignore = "daily"]
fn a_shield_fully_blocks_a_hit_scaled_down_by_easy() {
    let mut test_level = crate::health::TestLevel::new(0);
    let level = &mut test_level.level();
    use sunrise_world::blocking_rules::{BlockingRules, DamageReduction, ItemDamageFunction};
    let root = crate::prepared_test_root::prepared_root(&[]);
    let assets = crate::assets::Assets::load(&root).unwrap();
    let resources = crate::motion::MotionResources::load(&root, &assets).unwrap();
    let mut player =
        crate::motion::PlayerMotion::new(&resources, sunrise_core::geometry::Vec3::ZERO);
    let health = &resources.health;
    let rules = health::HealthRules {
        difficulty: sunrise_world::food::Difficulty::Easy,
        ..Default::default()
    };
    let shield = health::Blocking {
        rules: BlockingRules {
            delay_seconds: 0.25,
            disable_scale: 1.0,
            reductions: vec![DamageReduction {
                horizontal_angle: 90.0,
                types: None,
                base: 0.0,
                factor: 1.0,
            }],
            item_damage: ItemDamageFunction {
                threshold: 3.0,
                base: 1.0,
                factor: 1.0,
            },
        },
        bypassed: false,
        pierce: false,
        angle: 0.0,
    };
    let id = health.damage.id("minecraft:mob_attack").unwrap();
    let o = player
        .hurt_caused_by(level, health, rules, id, 4.0, Some(shield), true, 0, false)
        .unwrap();
    assert_eq!(o.blocked, 3.0, "Easy scales 4 to 3 before the shield");
    assert!(o.fully_blocked && !o.knockback);
}

/// #7309: on Hard a mob's 4 is scaled to 6 (`Player.hurtServer`), on
/// Normal it stays 4, and a non-mob source is never scaled.
#[test]
#[ignore = "daily"]
fn hard_scales_a_mob_hit_by_one_and_a_half() {
    use sunrise_world::food::Difficulty;
    let root = crate::prepared_test_root::prepared_root(&[]);
    let assets = crate::assets::Assets::load(&root).unwrap();
    let resources = crate::motion::MotionResources::load(&root, &assets).unwrap();
    let health = &resources.health;
    let id = health.damage.id("minecraft:mob_attack").unwrap();
    let lost = |difficulty, living| {
        let mut player =
            crate::motion::PlayerMotion::new(&resources, sunrise_core::geometry::Vec3::ZERO);
        let rules = health::HealthRules {
            difficulty,
            ..Default::default()
        };
        let before = player.health.health();
        player
            .hurt_caused_by(
                &mut health::TestLevel::new(0).level(),
                health,
                rules,
                id,
                4.0,
                None,
                living,
                0,
                false,
            )
            .unwrap();
        before - player.health.health()
    };
    assert_eq!(lost(Difficulty::Hard, true), 6.0);
    assert_eq!(lost(Difficulty::Normal, true), 4.0);
    assert_eq!(lost(Difficulty::Easy, true), 3.0);
    assert_eq!(lost(Difficulty::Hard, false), 4.0);
}

/// #2141: an arrow that takes the last of the player's health must kill
/// them: `LivingEntity.hurt` leaves the health at zero and `Player.die`
/// follows.
#[test]
#[ignore = "daily"]
fn an_arrow_kills_a_player_at_one_health() {
    let mut test_level = crate::health::TestLevel::new(0);
    let level = &mut test_level.level();
    let root = crate::prepared_test_root::prepared_root(&[]);
    let assets = crate::assets::Assets::load(&root).unwrap();
    let resources = crate::motion::MotionResources::load(&root, &assets).unwrap();
    let mut player =
        crate::motion::PlayerMotion::new(&resources, sunrise_core::geometry::Vec3::ZERO);
    player.set_health(1.0, false).unwrap();
    let (health, rules) = (&resources.health, health::HealthRules::default());
    let id = health.damage.id("minecraft:arrow").unwrap();
    let hurt = player.hurt(level, health, rules, id, 5.0).unwrap();
    assert!(hurt, "the arrow must hurt");
    assert_eq!(player.health.health(), 0.0, "the health must be spent");
    assert!(player.health.dead_or_dying(), "the player must be dead");
}

/// Needs the prepared data in `.local/sunrise-client/26.3`; skipped
/// without it.
#[test]
#[ignore = "daily"]
fn the_native_damage_hook_changes_or_cancels_a_players_hit() {
    let (hooks, seen) = set_hook();
    let mut test_level = crate::health::TestLevel::with_hooks(0, hooks);
    let level = &mut test_level.level();
    let root = crate::prepared_test_root::prepared_root(&[]);
    let assets = crate::assets::Assets::load(&root).unwrap();
    let resources = crate::motion::MotionResources::load(&root, &assets).unwrap();
    let (health, rules) = (&resources.health, health::HealthRules::default());
    let id = health.damage.id("minecraft:arrow").unwrap();
    let player = |entity| {
        let mut p = crate::motion::PlayerMotion::new(&resources, Default::default());
        p.entity_id = entity;
        p
    };
    let mut kept = player(9011);
    assert!(!kept.hurt(level, health, rules, id, 5.0).unwrap());
    assert_eq!((kept.health.health(), kept.health.hurt.cooldown), (20.0, 0));
    let mut doubled = player(9012);
    assert!(doubled.hurt(level, health, rules, id, 5.0).unwrap());
    assert_eq!(doubled.health.health(), 10.0);
    // The causing mob when it is known, else 0.
    let hurt = player(9013).hurt_caused_by(level, health, rules, id, 5.0, None, false, 44, false);
    assert!(hurt.unwrap().hurt);
    assert_eq!(seen(9011), [("minecraft:arrow 0".to_owned(), 5.0)]);
    assert_eq!(seen(9013), [("minecraft:arrow 44".to_owned(), 5.0)]);
}

/// #4301: `ServerPlayer.isInvulnerableTo`'s `!connection.hasClientLoaded`
/// term spares a respawned player the void until the client has loaded the
/// world; once it has, the void hurts again.
#[test]
#[ignore = "daily"]
fn the_void_spares_a_player_whose_client_has_not_loaded() {
    let mut test_level = crate::health::TestLevel::new(0);
    let level = &mut test_level.level();
    let root = crate::prepared_test_root::prepared_root(&[]);
    let assets = crate::assets::Assets::load(&root).unwrap();
    let resources = crate::motion::MotionResources::load(&root, &assets).unwrap();
    let mut player =
        crate::motion::PlayerMotion::new(&resources, sunrise_core::geometry::Vec3::ZERO);
    let (health, rules) = (&resources.health, health::HealthRules::default());
    let id = health.damage.id("minecraft:out_of_world").unwrap();
    player.client_loaded = false;
    assert!(
        !player.hurt(level, health, rules, id, 4.0).unwrap(),
        "an unloaded client must not take void damage"
    );
    assert_eq!(player.health.health(), 20.0);
    player.client_loaded = true;
    assert!(
        player.hurt(level, health, rules, id, 4.0).unwrap(),
        "a loaded client takes the void's 4.0"
    );
    assert_eq!(player.health.health(), 16.0);
}

/// `LivingEntity.getDamageAfterArmorAbsorb` returns before
/// `getArmorValue` and `getAttributeValue(ARMOR_TOUGHNESS)` for a source in
/// `#minecraft:bypasses_armor`, and native `AttributeInstance.getValue` is
/// what clears an instance's dirty flag. A hit must therefore leave both
/// instances dirty with creation's zero cache when it bypasses the armour,
/// and recompute them when it does not.
#[test]
#[ignore = "daily"]
fn a_bypassing_hit_leaves_the_armor_instances_dirty() {
    let mut test_level = crate::health::TestLevel::new(0);
    let level = &mut test_level.level();
    let root = crate::prepared_test_root::prepared_root(&[]);
    let assets = crate::assets::Assets::load(&root).unwrap();
    let resources = crate::motion::MotionResources::load(&root, &assets).unwrap();
    let (health, rules) = (&resources.health, health::HealthRules::default());
    // `setBaseValue` is what a command or a spawn uses; a base other than
    // the attribute's default is what makes the fresh instance's cache
    // stale, and it is the value a recompute would produce.
    let armoured = |resources: &crate::motion::MotionResources| {
        let mut player =
            crate::motion::PlayerMotion::new(resources, sunrise_core::geometry::Vec3::ZERO);
        let mut handles = Vec::new();
        for name in ["minecraft:armor", "minecraft:armor_toughness"] {
            let key = player.attributes.key(name).unwrap();
            let handle = player
                .attributes
                .get_instance(Some(key))
                .unwrap()
                .expect("the player supplier has this attribute");
            player
                .attributes
                .edit(&handle, |a, o| a.set_base_value(7.0, o))
                .unwrap();
            handles.push(handle);
        }
        (player, handles)
    };
    let states = |player: &crate::motion::PlayerMotion,
                  handles: &[sunrise_world::attributes::live::Handle]| {
        handles
            .iter()
            .map(|handle| {
                let a = player.attributes.instance(handle).unwrap();
                (a.base_value(), a.is_dirty(), a.cached_value())
            })
            .collect::<Vec<_>>()
    };
    // `minecraft:fall` is in the tag: neither instance is read.
    let (mut bypassing, bypassing_handles) = armoured(&resources);
    let fall = health.damage.id("minecraft:fall").unwrap();
    assert!(bypassing.hurt(level, health, rules, fall, 5.0).unwrap());
    assert_eq!(
        states(&bypassing, &bypassing_handles),
        [(7.0, true, 0.0), (7.0, true, 0.0)],
        "a bypassing hit must leave both instances dirty and uncached"
    );
    let record = bypassing.hurt_log.last().unwrap();
    assert_eq!(
        (record.armor, record.toughness),
        (0.0, 0.0),
        "the hit reports the cache it left"
    );
    // `minecraft:mob_attack` is not: `getDamageAfterAbsorb` asks for both
    // values, so the hit recomputes them and the record reads them back.
    let (mut meeting, meeting_handles) = armoured(&resources);
    let attack = health.damage.id("minecraft:mob_attack").unwrap();
    assert!(meeting.hurt(level, health, rules, attack, 5.0).unwrap());
    assert_eq!(
        states(&meeting, &meeting_handles),
        [(7.0, false, 7.0), (7.0, false, 7.0)],
        "a hit the armour meets must recompute both instances"
    );
    let record = meeting.hurt_log.last().unwrap();
    assert_eq!((record.armor, record.toughness), (7.0, 7.0));
}

/// Ordinary player hurt, using a custom source and a frozen-registry tag reload.
#[test]
#[ignore = "daily"]
fn datapack_damage_uses_custom_identity_armor_fire_and_reloaded_tags() {
    use sunrise_world::loot::PackOverlay;
    let root = crate::prepared_test_root::prepared_root(&[]);
    let assets = crate::assets::Assets::load(&root).unwrap();
    let resources = crate::motion::MotionResources::load(&root, &assets).unwrap();
    let files = vec![
        ("damage_type/demo:heated_air".into(), vec![br#"{"message_id":"heated_air","scaling":"never","exhaustion":0.25,"effects":"burning"}"#.to_vec()]),
        ("tags/minecraft:damage_type/is_fire".into(), vec![br#"{"values":["demo:heated_air"]}"#.to_vec()]),
        ("tags/minecraft:damage_type/bypasses_armor".into(), vec![br#"{"values":["demo:heated_air"]}"#.to_vec()]),
    ];
    let open = resources
        .health
        .with_packs(&root, &PackOverlay::new(&files), false)
        .unwrap();
    let id = open.damage.id("demo:heated_air").unwrap();
    let player = || {
        let mut p = PlayerMotion::new(&resources, sunrise_core::geometry::Vec3::ZERO);
        p.entity_id = 9701;
        let key = p.attributes.key("minecraft:armor").unwrap();
        let armor = p.attributes.get_instance(Some(key)).unwrap().unwrap();
        p.attributes
            .edit(&armor, |a, o| a.set_base_value(20.0, o))
            .unwrap();
        p
    };
    let rules = HealthRules::default();
    let mut test_level = TestLevel::new(0);
    let mut armored = player();
    assert!(
        armored
            .hurt(&mut test_level.level(), &open, rules, id, 4.0)
            .unwrap()
    );
    assert_eq!(armored.health.health(), 16.0);
    assert_eq!(armored.health.food.exhaustion, 0.25);
    assert_eq!(armored.hurt_log.last().unwrap().source, "demo:heated_air");
    let mut resistant = player();
    resistant.status.fire_resistance = true;
    assert!(
        !resistant
            .hurt(&mut test_level.level(), &open, rules, id, 4.0)
            .unwrap()
    );
    assert_eq!(resistant.health.health(), 20.0);
    let mut item_rules = crate::enchant::Enchantments::default();
    item_rules.bind_damage_catalog(&open.damage);
    assert!(item_rules.damage_tags["minecraft:is_fire"].contains("demo:heated_air"));
    // Disable the pack: the registered type remains, its added tags do not.
    let reload = open.with_packs(&root, &PackOverlay::NONE, true).unwrap();
    assert_eq!(reload.damage.id("demo:heated_air"), Some(id));
    let mut after = player();
    after.status.fire_resistance = true;
    assert!(
        after
            .hurt(&mut test_level.level(), &reload, rules, id, 4.0)
            .unwrap()
    );
    assert!(
        after.health.health() > 16.0 && after.health.health() < 20.0,
        "the same source now encounters armor and ignores fire resistance"
    );
    item_rules.bind_damage_catalog(&reload.damage);
    assert!(!item_rules.damage_tags["minecraft:is_fire"].contains("demo:heated_air"));
    assert!(
        open.damage.in_tag(id, "minecraft:is_fire"),
        "another snapshot remains unchanged"
    );
}
