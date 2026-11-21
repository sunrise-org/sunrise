use super::{
    GameplayAttributes, LegacyRandom, LevelRandom, RandomSource, Species, Vec3, VillageSiege,
};
use crate::game::{creatures::SpawnSite, native_entity_refs::tests::with_game};
use mod_api::{
    BlockPos, EntityTypeKey, Hooks, Host, LegacyRandomSource, MobSpawnPlacementCheckEvent,
    ModEntry, Registrar, RegistrationContext, SpawnCheck, SpawnPlacementContext,
    SpawnPlacementOperation, SpawnPlacementPredicate, SpawnReason,
};
use std::sync::{Arc, Mutex};
use sunrise_client::{
    chunk::{Chunk, Light},
    mobs::{spawn_rules::check_spawn_rules, spawn_selection::NaturalLevel},
    motion::PlayerMotion,
};

#[derive(Debug, PartialEq)]
enum Draw {
    Predicate(i32),
    Listener {
        default_result: bool,
        result: SpawnCheck,
        value: i32,
    },
}

struct SpawnProbe {
    predicate_result: bool,
    draws: Arc<Mutex<Vec<Draw>>>,
}

#[mod_api::__subscriptions]
impl Hooks for SpawnProbe {
    fn register(&self, registrar: &mut dyn Registrar, _: &RegistrationContext) {
        let draws = self.draws.clone();
        let answer = self.predicate_result;
        let predicate: Arc<dyn SpawnPlacementPredicate> = Arc::new(
            move |_: EntityTypeKey,
                  ctx: &mut dyn SpawnPlacementContext,
                  _: SpawnReason,
                  _: BlockPos| {
                let value = ctx.random().next_int();
                draws.lock().unwrap().push(Draw::Predicate(value));
                answer
            },
        );
        for id in ["minecraft:pillager", "minecraft:zombie"] {
            let kind = registrar.entity_type_key(id);
            registrar
                .register_spawn_placement(
                    kind,
                    None,
                    None,
                    predicate.clone(),
                    SpawnPlacementOperation::Replace,
                )
                .unwrap();
        }
    }

    fn mob_spawn_placement_check(&self, _: &dyn Host, event: &mut MobSpawnPlacementCheckEvent<'_>) {
        self.draws.lock().unwrap().push(Draw::Listener {
            default_result: event.default_result,
            result: *event.result,
            value: event.random.next_int(),
        });
        *event.result = SpawnCheck::Fail;
        // A mod-owned stream can outlive the event. Swapping this reference must
        // not send the next listener to that stream.
        event.random = Box::leak(Box::new(LegacyRandomSource::new(19)));
    }
}

struct NextListener(Arc<Mutex<Vec<Draw>>>);

#[mod_api::__subscriptions]
impl Hooks for NextListener {
    fn mob_spawn_placement_check(&self, _: &dyn Host, event: &mut MobSpawnPlacementCheckEvent<'_>) {
        self.0.lock().unwrap().push(Draw::Listener {
            default_result: event.default_result,
            result: *event.result,
            value: event.random.next_int(),
        });
    }
}

#[test]
#[ignore = "daily"]
fn placement_listener_draws_after_the_registered_predicate_on_the_callers_stream() {
    let draws = Arc::new(Mutex::new(Vec::new()));
    let probe = SpawnProbe {
        predicate_result: true,
        draws: draws.clone(),
    };
    mod_platform::testing::with_mods(
        vec![
            ModEntry::new("spawn-probe", Box::new(probe)),
            ModEntry::new("next-listener", Box::new(NextListener(draws.clone()))),
        ]
        .into(),
        || {
            with_game(|game| {
                game.set_difficulty_forced(2);
                let level = game.natural_level();
                let zombie = level.mob_type("minecraft:zombie").unwrap();
                let mut random = LegacyRandom::new(7);
                assert!(!check_spawn_rules(
                    zombie,
                    &level,
                    SpawnReason::Natural,
                    &mut random,
                    [0, 64, 0]
                ));
                // The first four nextInt() values of Java's Random(7), in order.
                assert_eq!(
                    *draws.lock().unwrap(),
                    [
                        Draw::Predicate(-1_156_638_823),
                        Draw::Listener {
                            default_result: true,
                            result: SpawnCheck::Default,
                            value: -1_552_468_968,
                        },
                        Draw::Listener {
                            default_result: true,
                            result: SpawnCheck::Fail,
                            value: -1_077_308_326,
                        },
                    ]
                );
                assert_eq!(
                    random.next_int(),
                    41_356_089,
                    "both listeners advanced the caller's stream despite Fail"
                );
            });
        },
    );
}

#[test]
#[ignore = "daily"]
fn patrols_and_sieges_bypass_registered_predicates_and_placement_listeners() {
    let draws = Arc::new(Mutex::new(Vec::new()));
    let probe = SpawnProbe {
        predicate_result: false,
        draws: draws.clone(),
    };
    mod_platform::testing::with_mods(
        vec![ModEntry::new("spawn-probe", Box::new(probe))].into(),
        || {
            with_game(|game| {
                game.set_difficulty_forced(2);
                game.world.day_time = 6000;
                game.raiding.attributes = Some(Arc::new(GameplayAttributes::default()));
                game.player =
                    Some(PlayerMotion::new(game.motion, Vec3::new(0.5, 64.0, 0.5)).unwrap());
                let stone = game
                    .pack
                    .assets
                    .registry
                    .default_state("minecraft:stone")
                    .unwrap();

                // Java's Random(3) wins the 1-in-5 roll and picks (37, -42) from the origin.
                game.world.random = LevelRandom::for_level(LegacyRandom::new(3));
                for cx in 1..=2 {
                    for cz in -4..=-2 {
                        game.world
                            .chunks
                            .insert((cx, cz), Arc::new(Chunk::empty(cx, cz)));
                    }
                }
                game.world.set(37, 63, -42, stone).unwrap();
                game.tick_patrol_spawner();
                assert!(
                    game.mobs
                        .iter()
                        .any(|mob| mob.species() == Species::Pillager
                            && mob.position == Vec3::new(37.0, 64.0, -42.0)),
                    "the patrol leader passes its direct vanilla predicate"
                );

                // Java's Random(7) picks (11, 10) from (8, 8); the dark stone column is in the village.
                game.world.day_time = 18001;
                let mut chunk = Chunk::empty(0, 0);
                let section = chunk.section_index(64).unwrap();
                Arc::make_mut(&mut chunk.sections[section]).light = Light::Uniform(0);
                game.world.chunks.insert((0, 0), Arc::new(chunk));
                game.world.set(11, 63, 10, stone).unwrap();
                let id = game
                    .spawn_mob(
                        "minecraft:villager",
                        [8.5, 64.0, 8.5],
                        false,
                        SpawnSite::Plain,
                    )
                    .unwrap();
                game.mobs
                    .iter_mut()
                    .find(|mob| mob.id == id)
                    .unwrap()
                    .extra
                    .home = Some([8, 64, 8]);
                game.world.random = LevelRandom::for_level(LegacyRandom::new(7));
                game.raiding.siege = VillageSiege {
                    tonight: true,
                    set_up: true,
                    zombies_to_spawn: 1,
                    spot: [8, 64, 8],
                    ..VillageSiege::default()
                };
                game.tick_village_siege();
                assert!(
                    game.mobs.iter().any(|mob| mob.species() == Species::Zombie
                        && mob.position == Vec3::new(11.5, 64.0, 10.5)),
                    "the siege zombie passes its direct vanilla predicate"
                );
                assert!(
                    draws.lock().unwrap().is_empty(),
                    "neither placement callbacks nor registered replacements ran"
                );
            });
        },
    );
}
