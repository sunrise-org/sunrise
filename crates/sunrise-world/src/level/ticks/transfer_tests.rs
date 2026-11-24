//! Actual shared level order-allocation regressions for prepared ownership handoff.
use super::*;
use sunrise_core::{
    position::{BlockPos, ChunkPos},
    ticks::{ScheduledTick, TickPriority, TickTypeId},
};

#[test]
fn same_level_take_does_not_recompute_a_prepared_destination_order_range() {
    let mut ticks = Ticks::default();
    let p = BlockPos::new(1, 64, 1);
    ticks.add_empty_chunk(ChunkPos::new(0, 0)).unwrap();
    ticks.blocks.schedule(ScheduledTick {
        kind: TickTypeId(1),
        pos: p,
        trigger_tick: 20,
        priority: TickPriority::Normal,
        sub_tick_order: 99,
    });
    let selected = [[p.x, p.y, p.z]].into();
    let captured = ticks.blocks.capture_transfer_cells(&selected, 1).unwrap();
    let cursor = ticks.transfer_order_cursor();
    let range = ticks.transfer_order_range(1).unwrap();
    assert_eq!(range, 100..101);
    let mut owned = ticks.blocks.extract_transfer_cells(&captured).unwrap();
    assert_eq!(ticks.blocks.count(), 0);
    // A recomputation here would choose 0..1. Consume exactly the range checked before take.
    ticks
        .reserve_prepared_transfer_orders(cursor, range)
        .unwrap();
    owned[0].tick.pos = BlockPos::new(2, 64, 1);
    owned[0].tick.sub_tick_order = 100;
    ticks.blocks.install_transfer_occurrences(owned).unwrap();
    assert_eq!(ticks.next_sub_tick(), 101);
    assert!(
        ticks
            .blocks
            .has_scheduled(BlockPos::new(2, 64, 1), TickTypeId(1))
    );
    assert!(!ticks.blocks.has_scheduled(p, TickTypeId(1)));
}
