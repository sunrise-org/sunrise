//! `LivingEntity.tick` stabs in `updatingUsingItem`, before `aiStep` moves the attacker: the
//! game settles a mob's stab at its `Pause::Stab`, so the push's slowdown
//! (`LivingEntity.causeExtraKnockback`) is in the move of the same tick.
use std::sync::{Arc, Mutex};
use sunrise_core::geometry::Vec3;
use sunrise_gameplay::game::ServerGame;
use sunrise_gameplay::game::native_entity_refs::tests::with_game;
use sunrise_gameplay_common::game::{Dim, in_own_process};
use sunrise_items::items::ItemStack;
use sunrise_level::chunk::Chunk;
use sunrise_sim::mobs::{Mob, spear_use};

/// A server game with one empty, entity-ticking chunk at the origin.
fn with_stab_world(test: impl FnOnce(&mut ServerGame<'_>)) {
    let root = std::env::temp_dir().join(format!(
        "sunrise-stab-pause-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    with_game(|game| {
        game.server.world_root = root.clone();
        game.enter_level(Dim::Overworld).unwrap();
        game.options.dedicated = Some(Vec::new());
        game.world
            .chunks
            .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        test(game);
    });
    let _ = std::fs::remove_dir_all(&root);
}

/// A persistent mob of `kind` floating without gravity at `at`.
fn floating(game: &ServerGame<'_>, id: u32, kind: &str, at: Vec3) -> Mob {
    let kind = game.res.mob_types().get(kind).unwrap().clone();
    let mut m = Mob::new(id, kind, at, 0.0);
    m.no_gravity = true;
    m.persistent = true;
    m
}

/// Zombie 71 at (0.5, 80, 0.5), past its iron spear's delay and drifting `speed` a tick along +z
/// in the air, without goals or targets: nothing adds to its motion but the drift.
fn charging_zombie(game: &ServerGame<'_>, speed: f64) -> Mob {
    let mut attacker = floating(game, 71, "minecraft:zombie", Vec3::new(0.5, 80.0, 0.5));
    attacker
        .extra
        .set_main_hand_stack(Some(ItemStack::new("minecraft:iron_spear", 1)));
    assert!(spear_use::start_using_item(&mut attacker, false));
    attacker.extra.using_item = spear_use::main_hand_kinetic(&attacker)
        .expect("an iron spear")
        .delay_ticks
        + 1;
    attacker.previous = Vec3::new(0.5, 80.0, 0.5 - speed);
    attacker.velocity = Vec3::new(0.0, 0.0, speed);
    attacker.ai.goals = Default::default();
    attacker.ai.targets = Default::default();
    attacker
}

/// A zombie mid-charge drifts 0.1 a tick in the air toward a NoAI villager 1.6 blocks ahead. The
/// stab's tick moves it 0.6 of that (one push: the iron spear has no knockback), where settling
/// after the tick moved it the whole 0.1 and slowed it only for the next.
#[test]
#[ignore = "daily"]
fn a_mob_stab_slows_the_attacker_before_it_moves() {
    in_own_process(
        "cut_layout::game::server_side::creatures_server::borrowed_tick::stab_pause_tests::a_mob_stab_slows_the_attacker_before_it_moves",
        || {
            crate::game::api_values::install_test_platform();
            with_stab_world(|game| {
                let attacker = charging_zombie(game, 0.1);
                let mut target =
                    floating(game, 72, "minecraft:villager", Vec3::new(0.5, 80.0, 2.1));
                target.no_ai = true;
                game.mobs.push(attacker);
                game.mobs.push(target);
                game.tick_mobs().unwrap();
                let mob = |game: &ServerGame<'_>, id| {
                    game.mobs.iter().find(|m| m.id == id).unwrap().clone()
                };
                let (attacker, target) = (mob(game, 71), mob(game, 72));
                assert!(target.health < 20.0, "the stab hurt the villager");
                assert!(
                    (attacker.position.z - (0.5 + 0.1 * 0.6)).abs() < 1e-9,
                    "moved {} after the slowdown",
                    attacker.position.z - 0.5
                );
            });
        },
    );
}

/// Hears a stab's dismount of villager 72 and the travels of zombie 71, and discards the zombie
/// from that dismount's `entity_mount` when `discard`.
struct DiscardOnDismount {
    discard: bool,
    heard: Arc<Mutex<Vec<&'static str>>>,
}

#[mod_api::__subscriptions]
impl mod_api::Hooks for DiscardOnDismount {
    fn entity_mount(
        &self,
        host: &mut dyn mod_api::ServerHost,
        event: &mut mod_api::EntityMountEvent<'_>,
    ) -> mod_api::Flow {
        if !event.mounting && event.entity == 72 {
            let heard = if !self.discard {
                "dismount"
            } else if host
                .entity(71)
                .expect("the attacker is live at its stab")
                .discard(host)
            {
                "dismount, attacker discarded"
            } else {
                "dismount, discard refused"
            };
            self.heard.lock().unwrap().push(heard);
        }
        mod_api::Flow::Continue
    }

    fn living_travel(
        &self,
        _: &dyn mod_api::LoopContext,
        event: &mut mod_api::LivingTravelEvent<'_>,
    ) -> mod_api::Flow {
        if event.entity == 71 {
            self.heard.lock().unwrap().push("attacker travels");
        }
        mod_api::Flow::Continue
    }
}

/// `LivingEntity.tick` runs `aiStep` only `if (!isRemoved())`. Through the game's mob pass, a
/// zombie charging at 0.15 a tick stabs and unseats a villager riding a pig: the dismount's
/// `entity_mount` comes before the zombie travels, and a listener that discards the zombie there
/// ends its tick at the stab, with no travel after it, and the zombie leaves the level.
#[test]
#[ignore = "daily"]
fn a_mob_discarded_while_its_stab_is_settled_does_not_travel() {
    in_own_process(
        "cut_layout::game::server_side::creatures_server::borrowed_tick::stab_pause_tests::a_mob_discarded_while_its_stab_is_settled_does_not_travel",
        || {
            crate::game::api_values::install_test_platform();
            for (discard, heard) in [
                (false, ["dismount", "attacker travels"].as_slice()),
                (true, ["dismount, attacker discarded"].as_slice()),
            ] {
                let log = Arc::new(Mutex::new(Vec::new()));
                let mods = vec![mod_api::ModEntry::new(
                    "stab-discard-test",
                    Box::new(DiscardOnDismount {
                        discard,
                        heard: log.clone(),
                    }),
                )];
                mod_platform::testing::with_mods(mods.into(), || {
                    with_stab_world(|game| discard_on_dismount_case(game, discard));
                });
                assert_eq!(*log.lock().unwrap(), heard, "discard {discard}");
            }
        },
    );
}

/// Villager 72 rides pig 73 1.6 blocks ahead of zombie 71, charging at 0.15 a tick: fast enough
/// for the iron spear's dismount. One mob pass stabs and unseats it.
fn discard_on_dismount_case(game: &mut ServerGame<'_>, discard: bool) {
    let attacker = charging_zombie(game, 0.15);
    let mut target = floating(game, 72, "minecraft:villager", Vec3::new(0.5, 80.0, 2.1));
    let mut pig = floating(game, 73, "minecraft:pig", Vec3::new(0.5, 80.0, 2.1));
    target.no_ai = true;
    pig.no_ai = true;
    game.mobs.push(attacker);
    game.mobs.push(target);
    game.mobs.push(pig);
    assert!(game.try_mount_mob(72, 73));
    let seated = game.mob_object_mut(72).unwrap();
    seated.previous = seated.position;
    game.tick_mobs().unwrap();
    let target = game.mob_object(72).unwrap();
    assert!(target.health < 20.0, "the stab hurt the villager");
    assert_eq!(target.extra.mount, None, "the stab unseated the villager");
    assert_eq!(
        game.mobs.iter().any(|m| m.id == 71),
        !discard,
        "a discarded attacker leaves the level"
    );
}
