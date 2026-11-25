//! `EntityRef::with_random` through the handle a mod calls: a draw advances the entity's own
//! random (`Entity.getRandom`), so the game's next draw from that entity sees the following
//! value, and a refused call draws nothing.

use super::native_guest_turn_tests::with_guests;
use mod_api::{EntityRef, ServerHost, WorldEpoch};
use sunrise_client::mobs::Mob;
use sunrise_core::geometry::Vec3;
use sunrise_core::random::{EntityRandom, LegacyRandom, RandomSource};

const COW_SEED: i64 = 0x0C0F_FEE5;
const PLAYER_SEED: i64 = 0x5EED_0013;

/// A mod's draw takes the entity's next value and the engine's own next draw from the entity
/// takes the one after it, for a mob, and for a guest parked outside its turn.
#[test]
#[ignore = "daily"]
fn a_draw_through_with_random_advances_the_entitys_own_random() {
    crate::game::in_own_process(
        "game::native_entity_random_tests::a_draw_through_with_random_advances_the_entitys_own_random",
        || {
            with_guests(|game| {
                game.options.dedicated = Some(Vec::new());
                game.world.chunks.insert(
                    (0, 0),
                    std::sync::Arc::new(sunrise_client::chunk::Chunk::empty(0, 0)),
                );
                let kind = game.res.mob_types().get("minecraft:cow").unwrap().clone();
                let mut cow = Mob::new(40, kind, Vec3::new(3.0, 64.0, 4.0), 0.0);
                cow.random = LegacyRandom::new(COW_SEED);
                game.mobs.push(cow);
                game.as_guest(13, |g| {
                    g.player.as_mut().unwrap().random =
                        EntityRandom::own(LegacyRandom::new(PLAYER_SEED));
                })
                .unwrap();
                let cow = game.entity(40).unwrap();
                let player = game.player(13).unwrap().entity();

                let mut cow_expected = LegacyRandom::new(COW_SEED);
                let mut player_expected = LegacyRandom::new(PLAYER_SEED);
                let host: &mut dyn ServerHost = &mut *game;
                assert_eq!(
                    cow.with_random(host, |rng| rng.next_float()),
                    Some(cow_expected.next_float())
                );
                assert_eq!(
                    player.with_random(host, |rng| rng.next_int_bound(100)),
                    Some(player_expected.next_int_bound(100).unwrap())
                );
                // The stream is kept: a second closure continues it.
                assert_eq!(
                    cow.with_random(host, |rng| rng.next_long()),
                    Some(cow_expected.next_long())
                );

                // The game's own next draw from each entity is the value after the mod's.
                let cow_next = game
                    .mobs
                    .iter_mut()
                    .find(|m| m.id == 40)
                    .unwrap()
                    .random
                    .next_int();
                assert_eq!(cow_next, cow_expected.next_int());
                let player_next = game
                    .as_guest(13, |g| g.player.as_mut().unwrap().random.next_int())
                    .unwrap();
                assert_eq!(player_next, player_expected.next_int());
            });
        },
    );
}

/// A stale handle and an entity with no handle answer `None` and never call the closure.
#[test]
#[ignore = "daily"]
fn with_random_refuses_a_stale_handle_without_drawing() {
    crate::game::in_own_process(
        "game::native_entity_random_tests::with_random_refuses_a_stale_handle_without_drawing",
        || {
            with_guests(|game| {
                game.options.dedicated = Some(Vec::new());
                game.world.chunks.insert(
                    (0, 0),
                    std::sync::Arc::new(sunrise_client::chunk::Chunk::empty(0, 0)),
                );
                let kind = game.res.mob_types().get("minecraft:cow").unwrap().clone();
                let mut cow = Mob::new(40, kind, Vec3::new(3.0, 64.0, 4.0), 0.0);
                cow.random = LegacyRandom::new(COW_SEED);
                game.mobs.push(cow);
                let live = game.entity(40).unwrap();
                let stale = EntityRef::new(WorldEpoch::new(live.epoch().get() + 1), live.id());
                let host: &mut dyn ServerHost = &mut *game;
                let mut called = false;
                assert_eq!(stale.with_random(host, |_| called = true), None);
                assert!(!called);
                let mut expected = LegacyRandom::new(COW_SEED);
                assert_eq!(
                    live.with_random(host, |rng| rng.next_int()),
                    Some(expected.next_int()),
                    "the refusal drew nothing"
                );
            });
        },
    );
}
