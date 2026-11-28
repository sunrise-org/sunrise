/// Install a part on glass that, when a neighbour changes, schedules its own tick 1 tick later
/// at `TickPriority::High` from the in-line level writer, as `DiodeBlock.checkTickOnNeighbor`
/// does (`level.scheduleTick(pos, block, delay, priority)`), and a water tick there 5 ticks later
/// at `TickPriority::VeryHigh` (`level.scheduleTick(pos, fluid, delay, priority)`), and records
/// each tick it runs, for real-game tests.
pub fn priority_tick_game_thread() {
    struct Scheduler(BlockKey);
    #[mod_api::sunrise_part]
    impl BlockPart for Scheduler {
        fn neighbor_changed(
            &self,
            level: &mut dyn crate::LevelWriteHost,
            event: &mut crate::NeighborChangedEvent,
        ) {
            let pos = [event.pos.x, event.pos.y, event.pos.z];
            let scheduled =
                level.schedule_block_tick_with_priority(pos, self.0, 1, crate::TickPriority::High);
            PRIORITY_TICKS.with(|r| r.borrow_mut().push(PriorityTick::Scheduled(pos, scheduled)));
            let fluid = level.schedule_fluid_tick_with_priority(
                pos,
                "minecraft:water",
                5,
                crate::TickPriority::VeryHigh,
            );
            PRIORITY_TICKS.with(|r| {
                r.borrow_mut()
                    .push(PriorityTick::ScheduledFluid(pos, fluid))
            });
        }
        fn tick_block(
            &self,
            _: &mut dyn crate::LevelWriteHost,
            event: &mut crate::TickBlockEvent,
        ) -> Flow {
            let pos = [event.pos.x, event.pos.y, event.pos.z];
            PRIORITY_TICKS.with(|r| r.borrow_mut().push(PriorityTick::Ticked(pos)));
            Flow::Continue
        }
    }
    struct SchedulerMod;
    #[mod_api::__subscriptions]
    impl Hooks for SchedulerMod {
        fn register(&self, r: &mut dyn Registrar, _: &crate::RegistrationContext) {
            let glass = r.block_key("minecraft:glass");
            r.block_part(glass, Scheduler(glass));
        }
    }
    let mods: crate::Mods =
        vec![ModEntry::new("priority_tick_test", Box::new(SchedulerMod))].into();
    crate::__private::install(mods.clone());
    install_parts(&mods, &crate::parts::tests::rule_schema(), |_| false).unwrap();
    mark_game_thread();
}

/// What the scheduling glass did: in `neighbor_changed`, the answers of its high-priority block
/// and very-high-priority fluid scheduling; or that its tick ran.
#[derive(Clone, Debug, PartialEq, Eq)]
// exhaustive: a test fixture's record, matched whole by the game's test
// no wire: testing-feature fixture state, not a mod boundary value.
pub enum PriorityTick {
    /// `neighbor_changed` at this position, and `schedule_block_tick_with_priority`'s answer.
    Scheduled([i32; 3], bool),
    /// `neighbor_changed` at this position, and `schedule_fluid_tick_with_priority`'s answer.
    ScheduledFluid([i32; 3], bool),
    /// `tick_block` ran at this position.
    Ticked([i32; 3]),
}

std::thread_local! {
    static PRIORITY_TICKS: std::cell::RefCell<Vec<PriorityTick>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// What the scheduling glass recorded on this thread, taken.
pub fn priority_ticks() -> Vec<PriorityTick> {
    PRIORITY_TICKS.with(|r| std::mem::take(&mut *r.borrow_mut()))
}
