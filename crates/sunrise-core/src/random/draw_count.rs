//! Per-phase and per-consumer LCG draw counter for the level-wide random source (`LevelRandom` / `World.random`).
use std::{cell::Cell, marker::PhantomData};

use super::{LegacyRandom, LevelRandom};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum Phase {
    RandomBlockTicks = 0,
    ScheduledTicks = 1,
    NaturalSpawning = 2,
    MobBatch = 3,
    Weather = 4,
    BlockEntities = 5,
    Raids = 6,
    PlayerActions = 7,
    Other = 8,
}

impl Phase {
    pub const COUNT: usize = 9;
    pub const ALL: [Self; Self::COUNT] = [
        Self::RandomBlockTicks,
        Self::ScheduledTicks,
        Self::NaturalSpawning,
        Self::MobBatch,
        Self::Weather,
        Self::BlockEntities,
        Self::Raids,
        Self::PlayerActions,
        Self::Other,
    ];

    pub const fn name(self) -> &'static str {
        match self {
            Self::RandomBlockTicks => "random_block_ticks",
            Self::ScheduledTicks => "scheduled_ticks",
            Self::NaturalSpawning => "natural_spawning",
            Self::MobBatch => "mob_batch",
            Self::Weather => "weather",
            Self::BlockEntities => "block_entities",
            Self::Raids => "raids",
            Self::PlayerActions => "player_actions",
            Self::Other => "other",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum Consumer {
    RandomBlockTicks = 0,
    ScheduledBlocks = 1,
    ScheduledFluids = 2,
    NaturalSpawning = 3,
    CustomSpawners = 4,
    MobBatch = 5,
    WeatherCycle = 6,
    Thunder = 7,
    MovingPistons = 8,
    BlockEntities = 9,
    Sculk = 10,
    BlockEntityServers = 11,
    ConduitHurtTeleports = 12,
    Raids = 13,
    PlayerEffects = 14,
    PlayerUsing = 15,
    LanPackets = 16,
    LanGuests = 17,
    LanGuestConnections = 18,
    LanEvents = 19,
    Other = 20,
}

impl Consumer {
    pub const COUNT: usize = 21;
    pub const ALL: [Self; Self::COUNT] = [
        Self::RandomBlockTicks,
        Self::ScheduledBlocks,
        Self::ScheduledFluids,
        Self::NaturalSpawning,
        Self::CustomSpawners,
        Self::MobBatch,
        Self::WeatherCycle,
        Self::Thunder,
        Self::MovingPistons,
        Self::BlockEntities,
        Self::Sculk,
        Self::BlockEntityServers,
        Self::ConduitHurtTeleports,
        Self::Raids,
        Self::PlayerEffects,
        Self::PlayerUsing,
        Self::LanPackets,
        Self::LanGuests,
        Self::LanGuestConnections,
        Self::LanEvents,
        Self::Other,
    ];

    pub const fn phase(self) -> Phase {
        match self {
            Self::RandomBlockTicks => Phase::RandomBlockTicks,
            Self::ScheduledBlocks | Self::ScheduledFluids => Phase::ScheduledTicks,
            Self::NaturalSpawning | Self::CustomSpawners => Phase::NaturalSpawning,
            Self::MobBatch => Phase::MobBatch,
            Self::WeatherCycle | Self::Thunder => Phase::Weather,
            Self::MovingPistons
            | Self::BlockEntities
            | Self::Sculk
            | Self::BlockEntityServers
            | Self::ConduitHurtTeleports => Phase::BlockEntities,
            Self::Raids => Phase::Raids,
            Self::PlayerEffects
            | Self::PlayerUsing
            | Self::LanPackets
            | Self::LanGuests
            | Self::LanGuestConnections
            | Self::LanEvents => Phase::PlayerActions,
            Self::Other => Phase::Other,
        }
    }

    pub const fn default_for(phase: Phase) -> Self {
        match phase {
            Phase::RandomBlockTicks => Self::RandomBlockTicks,
            Phase::ScheduledTicks => Self::ScheduledBlocks,
            Phase::NaturalSpawning => Self::NaturalSpawning,
            Phase::MobBatch => Self::MobBatch,
            Phase::Weather => Self::WeatherCycle,
            Phase::BlockEntities => Self::BlockEntities,
            Phase::Raids => Self::Raids,
            Phase::PlayerActions => Self::LanPackets,
            Phase::Other => Self::Other,
        }
    }

    pub const fn name(self) -> &'static str {
        match self {
            Self::RandomBlockTicks => "random_block_ticks",
            Self::ScheduledBlocks => "scheduled_blocks",
            Self::ScheduledFluids => "scheduled_fluids",
            Self::NaturalSpawning => "natural_spawning",
            Self::CustomSpawners => "custom_spawners",
            Self::MobBatch => "mob_batch",
            Self::WeatherCycle => "weather_cycle",
            Self::Thunder => "thunder",
            Self::MovingPistons => "moving_pistons",
            Self::BlockEntities => "block_entities",
            Self::Sculk => "sculk",
            Self::BlockEntityServers => "block_entity_servers",
            Self::ConduitHurtTeleports => "conduit_hurt_teleports",
            Self::Raids => "raids",
            Self::PlayerEffects => "player_effects",
            Self::PlayerUsing => "player_using",
            Self::LanPackets => "lan_packets",
            Self::LanGuests => "lan_guests",
            Self::LanGuestConnections => "lan_guest_connections",
            Self::LanEvents => "lan_events",
            Self::Other => "other",
        }
    }
}

thread_local! {
    static CONSUMER: Cell<Consumer> = const { Cell::new(Consumer::Other) };
    static COUNTS: [Cell<u64>; Consumer::COUNT] =
        const { [const { Cell::new(0) }; Consumer::COUNT] };
}

#[must_use = "the random-draw phase ends when the guard is dropped"]
pub struct PhaseGuard {
    previous: Consumer,
    _thread: PhantomData<*const ()>,
}

impl Drop for PhaseGuard {
    #[inline]
    fn drop(&mut self) {
        CONSUMER.with(|current| current.set(self.previous));
    }
}

#[inline]
pub fn enter(phase: Phase) -> PhaseGuard {
    enter_consumer(Consumer::default_for(phase))
}

#[inline]
pub fn enter_consumer(consumer: Consumer) -> PhaseGuard {
    PhaseGuard {
        previous: CONSUMER.with(|current| current.replace(consumer)),
        _thread: PhantomData,
    }
}

#[inline]
pub fn in_phase<T>(phase: Phase, f: impl FnOnce() -> T) -> T {
    let _guard = enter(phase);
    f()
}

#[inline]
pub fn in_consumer<T>(consumer: Consumer, f: impl FnOnce() -> T) -> T {
    let _guard = enter_consumer(consumer);
    f()
}

#[inline]
pub fn current() -> Phase {
    current_consumer().phase()
}

#[inline]
pub fn current_consumer() -> Consumer {
    CONSUMER.with(Cell::get)
}

#[inline]
pub(super) fn record() {
    let consumer = CONSUMER.with(Cell::get) as usize;
    COUNTS.with(|counts| {
        let slot = &counts[consumer];
        slot.set(slot.get().wrapping_add(1));
    });
}

fn restore(saved: &[u64; Consumer::COUNT]) {
    COUNTS.with(|counts| {
        for (slot, &value) in counts.iter().zip(saved) {
            slot.set(value);
        }
    });
}

/// Scopes per-consumer counts to one tick; dropping without [`Self::finish`] restores the enclosing counts.
#[must_use = "the random-draw scope ends when the guard is dropped"]
pub struct ScopeGuard {
    saved: [u64; Consumer::COUNT],
    _thread: PhantomData<*const ()>,
}

#[inline]
pub fn scope() -> ScopeGuard {
    ScopeGuard {
        saved: take_consumers(),
        _thread: PhantomData,
    }
}

impl ScopeGuard {
    #[inline]
    pub fn finish(self) -> [u64; Consumer::COUNT] {
        let current = take_consumers();
        restore(&self.saved);
        std::mem::forget(self);
        current
    }
}

impl Drop for ScopeGuard {
    #[inline]
    fn drop(&mut self) {
        restore(&self.saved);
    }
}

/// Copy a generator's 48-bit state and cached gaussian without marking it as the live level stream.
#[inline]
pub fn untracked_copy(random: &LegacyRandom) -> LegacyRandom {
    random.clone().with_level_owned(false)
}

/// Wrap `random` as a `LevelRandom` placeholder whose draws do not count as live level draws.
#[inline]
pub const fn placeholder_level(random: LegacyRandom) -> LevelRandom {
    LevelRandom::untracked(random)
}

pub fn sum_phases(consumers: &[u64; Consumer::COUNT]) -> [u64; Phase::COUNT] {
    let mut phases = [0u64; Phase::COUNT];
    for (consumer, &count) in Consumer::ALL.iter().zip(consumers) {
        let slot = &mut phases[consumer.phase() as usize];
        *slot = slot.wrapping_add(count);
    }
    phases
}

pub fn take_consumers() -> [u64; Consumer::COUNT] {
    COUNTS.with(|counts| std::array::from_fn(|i| counts[i].replace(0)))
}

pub fn take() -> [u64; Phase::COUNT] {
    sum_phases(&take_consumers())
}

pub fn reset() {
    let _ = take_consumers();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::position::BlockPos;
    use crate::random::{
        EntityRandom, InvalidBound, LegacyRandom, LevelRandom, PositionalRandom, RandomSource,
    };

    #[test]
    fn attributes_level_draws_by_phase_and_ignores_other_sources() {
        reset();
        let mut level = LevelRandom::for_level(LegacyRandom::new(1));
        let mut entity = EntityRandom::own(LegacyRandom::new(2));
        let mut plain = LegacyRandom::new(3);

        let _ = level.next_int();
        let _ = entity.next_int();
        let _ = plain.next_long();

        in_phase(Phase::RandomBlockTicks, || {
            let _ = level.next_float();
            let _ = level.next_boolean();
            let _ = entity.next_double();
        });

        {
            let _outer = enter(Phase::MobBatch);
            let _ = level.next_int();
            {
                let _inner = enter_consumer(Consumer::Thunder);
                assert_eq!(current(), Phase::Weather);
                assert_eq!(current_consumer(), Consumer::Thunder);
                let _ = level.next_double();
            }
            assert_eq!(current(), Phase::MobBatch);
            assert_eq!(current_consumer(), Consumer::MobBatch);
            let _ = level.next_float();
        }
        assert_eq!(current(), Phase::Other);

        // `fork` draws its seed (next_long = 2 LCG steps) from the level source;
        // draws on the forked source itself are not level-wide draws.
        let mut forked = in_phase(Phase::Raids, || level.fork());
        in_phase(Phase::Raids, || {
            let _ = forked.next_int();
            let _ = forked.next_long();
        });
        let pos_rng = in_consumer(Consumer::ScheduledFluids, || level.fork_positional());
        let mut at = pos_rng.at(BlockPos::new(1, 2, 3));
        in_consumer(Consumer::ScheduledFluids, || {
            let _ = at.next_int();
        });

        let consumers = take_consumers();
        assert_eq!(consumers[Consumer::Thunder as usize], 2);
        assert_eq!(consumers[Consumer::ScheduledFluids as usize], 2);
        let counts = sum_phases(&consumers);
        assert_eq!(counts[Phase::Other as usize], 1);
        assert_eq!(counts[Phase::RandomBlockTicks as usize], 2);
        assert_eq!(counts[Phase::MobBatch as usize], 2);
        assert_eq!(counts[Phase::Weather as usize], 2);
        assert_eq!(counts[Phase::Raids as usize], 2);
        assert_eq!(counts[Phase::ScheduledTicks as usize], 2);
        assert_eq!(counts[Phase::NaturalSpawning as usize], 0);
        assert_eq!(counts[Phase::BlockEntities as usize], 0);
        assert_eq!(counts[Phase::PlayerActions as usize], 0);
        assert_eq!(take(), [0; Phase::COUNT]);
    }

    #[test]
    fn lent_level_random_counts_while_swapped_out_and_after_return() {
        reset();
        let mut level = LevelRandom::for_level(LegacyRandom::new(99));

        // Matches `LentLevel::take`: swap the inner `LegacyRandom` out for a placeholder.
        let mut lent = std::mem::replace(&mut *level, LegacyRandom::new(0));
        in_phase(Phase::MobBatch, || {
            let _ = lent.next_int();
            let _ = lent.next_long();
        });
        // Drawing from the temporary placeholder does not count.
        in_phase(Phase::Other, || {
            let _ = level.next_int();
            let _ = level.next_double();
        });
        *level = lent;

        in_phase(Phase::BlockEntities, || {
            let _ = level.next_boolean();
        });

        let counts = take();
        assert_eq!(counts[Phase::MobBatch as usize], 3);
        assert_eq!(counts[Phase::Other as usize], 0);
        assert_eq!(counts[Phase::BlockEntities as usize], 1);
    }

    #[test]
    fn placeholder_level_swap_does_not_count_temporary_draws() {
        reset();
        let mut world_random = LevelRandom::for_level(LegacyRandom::new(55));
        let expected_placeholder = LegacyRandom::new(123).next_long();

        let mut taken =
            std::mem::replace(&mut world_random, placeholder_level(LegacyRandom::new(123)));
        // Temporary placeholder preserves seed/state but draws on it do not count.
        in_phase(Phase::Other, || {
            assert_eq!(world_random.next_long(), expected_placeholder);
        });
        in_phase(Phase::NaturalSpawning, || {
            let _ = taken.next_int();
        });
        world_random = taken;

        in_phase(Phase::Raids, || {
            let _ = world_random.next_float();
        });

        let counts = take();
        assert_eq!(counts[Phase::Other as usize], 0);
        assert_eq!(counts[Phase::NaturalSpawning as usize], 1);
        assert_eq!(counts[Phase::Raids as usize], 1);
    }

    #[test]
    fn live_clone_counts_while_untracked_fallback_copy_does_not() {
        reset();
        let mut level = LevelRandom::for_level(LegacyRandom::new(7));
        let mut live_clone = level.clone();
        let mut fallback = untracked_copy(&level);

        in_phase(Phase::PlayerActions, || {
            assert_eq!(level.next_int_bound(0), Err(InvalidBound));
            let a = live_clone.next_int();
            let b = live_clone.next_float();
            assert_eq!(fallback.next_int(), a);
            assert_eq!(fallback.next_float(), b);
        });

        let counts = take();
        assert_eq!(counts[Phase::PlayerActions as usize], 2);
    }

    #[test]
    fn early_return_and_nested_unwind_restore_phase_and_scope() {
        reset();
        let mut level = LevelRandom::for_level(LegacyRandom::new(19));

        fn early_return_step(level: &mut LevelRandom) -> Result<(), ()> {
            let _scope = scope();
            let _guard = enter(Phase::Raids);
            let _ = level.next_int();
            Err(())
        }

        // Pre-tick draw in outer scope.
        let outer = scope();
        in_phase(Phase::Weather, || {
            let _ = level.next_int();
        });

        // Early return drops both inner scope and inner phase guard.
        assert!(early_return_step(&mut level).is_err());
        assert_eq!(current(), Phase::Other);

        // Nested panic inside inner scope and nested phase guards restores both.
        let unwound = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _inner_scope = scope();
            let _outer_phase = enter(Phase::BlockEntities);
            let _ = level.next_int();
            let _inner_phase = enter(Phase::MobBatch);
            let _ = level.next_long();
            panic!("simulated tick failure");
        }));
        assert!(unwound.is_err());
        assert_eq!(current(), Phase::Other);

        // Outer scope still holds only its own draw and accepts subsequent draws cleanly.
        in_phase(Phase::RandomBlockTicks, || {
            let _ = level.next_float();
        });
        let counts = sum_phases(&outer.finish());
        assert_eq!(counts[Phase::Weather as usize], 1);
        assert_eq!(counts[Phase::RandomBlockTicks as usize], 1);
        assert_eq!(counts[Phase::Raids as usize], 0);
        assert_eq!(counts[Phase::BlockEntities as usize], 0);
        assert_eq!(counts[Phase::MobBatch as usize], 0);
        assert_eq!(take(), [0; Phase::COUNT]);
    }
}
