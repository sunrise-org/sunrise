//! Per-phase and per-consumer level-random draw attribution and capture-counter publishing (`--features rng-count`).

#[cfg(feature = "rng-count")]
pub use sunrise_core::random::draw_count::{
    Consumer, Phase, PhaseGuard, enter as guard, enter_consumer as guard_consumer, in_consumer,
    in_phase, placeholder_level, untracked_copy,
};

#[cfg(not(feature = "rng-count"))]
#[allow(dead_code)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Phase {
    RandomBlockTicks,
    ScheduledTicks,
    NaturalSpawning,
    MobBatch,
    Weather,
    BlockEntities,
    Raids,
    PlayerActions,
    Other,
}

#[cfg(not(feature = "rng-count"))]
#[allow(dead_code)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Consumer {
    RandomBlockTicks,
    ScheduledBlocks,
    ScheduledFluids,
    NaturalSpawning,
    CustomSpawners,
    MobBatch,
    WeatherCycle,
    Thunder,
    MovingPistons,
    BlockEntities,
    Sculk,
    BlockEntityServers,
    ConduitHurtTeleports,
    Raids,
    PlayerEffects,
    PlayerUsing,
    LanPackets,
    LanGuests,
    LanGuestConnections,
    LanEvents,
    Other,
}

#[cfg(not(feature = "rng-count"))]
pub struct PhaseGuard;

#[cfg(not(feature = "rng-count"))]
#[inline(always)]
pub fn guard(_phase: Phase) -> PhaseGuard {
    PhaseGuard
}

#[cfg(not(feature = "rng-count"))]
#[inline(always)]
pub fn guard_consumer(_consumer: Consumer) -> PhaseGuard {
    PhaseGuard
}

#[cfg(not(feature = "rng-count"))]
#[inline(always)]
pub fn in_phase<T>(_phase: Phase, f: impl FnOnce() -> T) -> T {
    f()
}

#[cfg(not(feature = "rng-count"))]
#[inline(always)]
pub fn in_consumer<T>(_consumer: Consumer, f: impl FnOnce() -> T) -> T {
    f()
}

#[cfg(not(feature = "rng-count"))]
#[inline(always)]
pub fn untracked_copy(
    random: &sunrise_core::random::LegacyRandom,
) -> sunrise_core::random::LegacyRandom {
    random.clone()
}

#[cfg(not(feature = "rng-count"))]
#[inline(always)]
pub const fn placeholder_level(
    random: sunrise_core::random::LegacyRandom,
) -> sunrise_core::random::LevelRandom {
    sunrise_core::random::LevelRandom::for_level(random)
}

#[cfg(feature = "rng-count")]
#[must_use = "the tick draw scope ends when dropped or published"]
pub struct TickScope(sunrise_core::random::draw_count::ScopeGuard);

#[cfg(not(feature = "rng-count"))]
pub struct TickScope;

#[cfg(feature = "rng-count")]
#[inline]
pub fn begin_tick() -> TickScope {
    TickScope(sunrise_core::random::draw_count::scope())
}

#[cfg(not(feature = "rng-count"))]
#[inline(always)]
pub fn begin_tick() -> TickScope {
    TickScope
}

impl TickScope {
    #[cfg(feature = "rng-count")]
    pub fn publish(self) {
        use std::sync::OnceLock;
        use sunrise_perf_format::Counter;

        struct Counters {
            phases: [Counter; Phase::COUNT],
            consumers: [Counter; Consumer::COUNT],
        }

        static COUNTERS: OnceLock<Counters> = OnceLock::new();
        let counters = COUNTERS.get_or_init(|| {
            let registry = sunrise_perf_format::counter_registry();
            Counters {
                phases: Phase::ALL.map(|phase| {
                    registry.counter_with_unit(
                        "level_random.draws",
                        &[("phase", phase.name())],
                        "draws",
                    )
                }),
                consumers: Consumer::ALL.map(|consumer| {
                    registry.counter_with_unit(
                        "level_random.draws_by_consumer",
                        &[
                            ("phase", consumer.phase().name()),
                            ("consumer", consumer.name()),
                        ],
                        "draws",
                    )
                }),
            }
        });
        let consumer_counts = self.0.finish();
        let phase_counts = sunrise_core::random::draw_count::sum_phases(&consumer_counts);
        for (counter, count) in counters.phases.iter().zip(phase_counts) {
            if count != 0 {
                counter.add(count);
            }
        }
        for (counter, count) in counters.consumers.iter().zip(consumer_counts) {
            if count != 0 {
                counter.add(count);
            }
        }
    }

    #[cfg(not(feature = "rng-count"))]
    #[inline(always)]
    pub fn publish(self) {}
}

#[cfg(all(test, feature = "rng-count"))]
mod tests {
    use super::*;
    use sunrise_core::random::{EntityRandom, LegacyRandom, RandomSource};
    use sunrise_level::world::World;
    use sunrise_perf_format::{CounterReporter, Interner, counter_registry, v1::Side};
    use sunrise_sim::health::{DamageHooks, DamageLevel, LentLevel};
    use sunrise_world::registry::{BlockRegistry, BlockStateId};

    #[test]
    fn lent_world_random_attributes_to_mob_batch_and_publishes_capture_counters() {
        let registry = counter_registry();
        let before_phase: [u64; Phase::COUNT] = Phase::ALL.map(|phase| {
            registry
                .counter_with_unit("level_random.draws", &[("phase", phase.name())], "draws")
                .get()
        });
        let before_consumer: [u64; Consumer::COUNT] = Consumer::ALL.map(|consumer| {
            registry
                .counter_with_unit(
                    "level_random.draws_by_consumer",
                    &[
                        ("phase", consumer.phase().name()),
                        ("consumer", consumer.name()),
                    ],
                    "draws",
                )
                .get()
        });
        sunrise_core::random::draw_count::reset();

        let facts =
            br#"{"defaults":{"minecraft:air":0},"states":[["air",true,false,"","minecraft:air"]]}"#;
        let blocks = BlockRegistry::from_facts_json(&facts[..]).unwrap();
        let mut world = World::new(BlockStateId(0), 42);
        let mut entity_rng = EntityRandom::own(LegacyRandom::new(7));

        // Pre-tick draw must not bleed into the next tick scope.
        let _ = world.random.next_long();

        let tick = begin_tick();
        in_consumer(Consumer::WeatherCycle, || {
            let _ = world.random.next_int();
        });
        in_consumer(Consumer::RandomBlockTicks, || {
            let _ = world.random.next_float();
            let _ = world.random.next_boolean();
            let _ = entity_rng.next_int();
        });
        in_consumer(Consumer::MobBatch, || {
            let mut lent = LentLevel::take(&mut world);
            {
                let mut damage_level =
                    DamageLevel::lent(&world, &blocks, &mut lent, None, None, &DamageHooks::NONE);
                let _ = damage_level.random().next_int();
                let _ = damage_level.random().next_double();
            }
            // Placeholder installed in `world.random` while lent is not the level source.
            let _ = world.random.next_long();
            lent.give_back(&mut world);
        });
        in_consumer(Consumer::NaturalSpawning, || {
            let mut taken =
                std::mem::replace(&mut world.random, placeholder_level(LegacyRandom::new(0)));
            // Temporary whole-wrapper placeholder does not count.
            let _ = world.random.next_long();
            let _ = taken.next_int();
            world.random = taken;
        });
        in_consumer(Consumer::PlayerEffects, || {
            let mut live_clone = world.random.clone();
            let mut fallback = untracked_copy(&live_clone);
            let _ = live_clone.next_int();
            let _ = fallback.next_long();
            world.random = live_clone;
        });
        in_consumer(Consumer::ConduitHurtTeleports, || {
            let _ = world.random.next_float();
        });
        // An unwound inner tick must neither publish nor contaminate the enclosing tick.
        let unwound = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _aborted = begin_tick();
            in_consumer(Consumer::Raids, || {
                let _ = world.random.next_long();
                panic!("aborted tick");
            });
        }));
        assert!(unwound.is_err());
        let _ = world.random.next_float();

        tick.publish();

        let delta_phase = |phase: Phase| {
            registry
                .counter("level_random.draws", &[("phase", phase.name())])
                .get()
                - before_phase[phase as usize]
        };
        let delta_consumer = |consumer: Consumer| {
            registry
                .counter(
                    "level_random.draws_by_consumer",
                    &[
                        ("phase", consumer.phase().name()),
                        ("consumer", consumer.name()),
                    ],
                )
                .get()
                - before_consumer[consumer as usize]
        };
        assert_eq!(delta_phase(Phase::Weather), 1);
        assert_eq!(delta_phase(Phase::RandomBlockTicks), 2);
        assert_eq!(delta_phase(Phase::MobBatch), 3);
        assert_eq!(delta_phase(Phase::NaturalSpawning), 1);
        assert_eq!(delta_phase(Phase::PlayerActions), 1);
        assert_eq!(delta_phase(Phase::BlockEntities), 1);
        assert_eq!(delta_phase(Phase::Other), 1);
        assert_eq!(delta_phase(Phase::ScheduledTicks), 0);
        assert_eq!(delta_phase(Phase::Raids), 0);
        assert_eq!(delta_consumer(Consumer::PlayerEffects), 1);
        assert_eq!(delta_consumer(Consumer::ConduitHurtTeleports), 1);
        assert_eq!(delta_consumer(Consumer::MobBatch), 3);
        assert_eq!(delta_consumer(Consumer::Raids), 0);

        let mut strings = Interner::default();
        let mut reporter = CounterReporter::new(Side::Server);
        let report = reporter.report(registry, &mut strings, 1.0);
        let entries = strings.take_new(Side::Server);
        let text = |id: u32| {
            entries
                .iter()
                .find(|entry| entry.id == id)
                .map(|entry| entry.text.as_str())
        };
        let mob_def = report
            .defs
            .iter()
            .find(|def| {
                text(def.name) == Some("level_random.draws")
                    && def
                        .labels
                        .iter()
                        .any(|l| text(l.key) == Some("phase") && text(l.value) == Some("mob_batch"))
            })
            .expect("level_random.draws{phase=mob_batch} counter definition");
        assert_eq!(text(mob_def.unit), Some("draws"));
        let conduit_def = report
            .defs
            .iter()
            .find(|def| {
                text(def.name) == Some("level_random.draws_by_consumer")
                    && def.labels.iter().any(|l| {
                        text(l.key) == Some("consumer")
                            && text(l.value) == Some("conduit_hurt_teleports")
                    })
            })
            .expect("level_random.draws_by_consumer{consumer=conduit_hurt_teleports} definition");
        assert_eq!(text(conduit_def.unit), Some("draws"));
    }
}
