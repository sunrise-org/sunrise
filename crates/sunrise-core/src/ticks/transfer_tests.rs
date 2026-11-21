//! Actual scheduler ownership, pending-work and import regressions.
use super::*;

#[test]
fn pending_refuses_then_settled_queued_and_held_duplicates_move_once() {
    let from = BlockPos::new(1, 64, 1);
    let to = BlockPos::new(17, 72, 1);
    let unrelated = BlockPos::new(2, 64, 1);
    let saved = |pos, kind, delay, priority| SavedTick {
        kind: TickTypeId(kind),
        pos,
        delay,
        priority,
    };
    let mut source = LevelTicks::default();
    source
        .add_chunk(
            from.chunk(),
            ChunkTicks::from_saved(vec![
                saved(from, 7, 0, TickPriority::High),
                saved(from, 7, 0, TickPriority::High),
                saved(from, 8, 5, TickPriority::Low),
                saved(unrelated, 9, 9, TickPriority::Normal),
            ]),
        )
        .unwrap();
    let selected = [[from.x, from.y, from.z]].into();
    assert_eq!(
        source.capture_transfer_cells(&selected, 4).unwrap_err(),
        TransferTickError::Busy
    );
    source.unpack_chunk(from.chunk(), 100);
    source
        .begin_tick_where(100, 64, |_| true, |_| false)
        .unwrap();
    assert_eq!(
        source.capture_transfer_cells(&selected, 4).unwrap_err(),
        TransferTickError::Busy
    );
    source.finish_tick().unwrap();
    let witness = source.capture_transfer_cells(&selected, 4).unwrap();
    assert_eq!(witness.occurrences().len(), 3);
    assert_eq!(witness.occurrences().iter().filter(|t| t.held).count(), 2);
    let mut destination = LevelTicks::default();
    destination
        .add_chunk(to.chunk(), ChunkTicks::default())
        .unwrap();
    let source_revision = source.chunk(from.chunk()).unwrap().persistence_revision();
    assert!(source.acknowledge_persistent_save(from.chunk(), source_revision));
    let mut owned = source.extract_transfer_cells(&witness).unwrap();
    assert!(source.chunk(from.chunk()).unwrap().needs_persistent_save());
    assert!(!source.acknowledge_persistent_save(from.chunk(), source_revision));
    assert_eq!(source.count(), 1);
    assert!(source.has_scheduled(unrelated, TickTypeId(9)));
    for (order, occurrence) in (20..).zip(&mut owned) {
        occurrence.tick.trigger_tick += 200;
        occurrence.tick.pos = to;
        occurrence.tick.sub_tick_order = order;
    }
    destination.install_transfer_occurrences(owned).unwrap();
    assert!(
        destination
            .chunk(to.chunk())
            .unwrap()
            .needs_persistent_save()
    );
    let packed = destination.chunk(to.chunk()).unwrap().pack(300);
    assert_eq!(
        packed,
        [
            saved(to, 7, 0, TickPriority::High),
            saved(to, 7, 0, TickPriority::High),
            saved(to, 8, 5, TickPriority::Low)
        ]
    );
    destination
        .begin_tick_where(300, 64, |_| true, |_| true)
        .unwrap();
    let mut due = Vec::new();
    while let Some(tick) = destination.next_tick().unwrap() {
        due.push(tick);
    }
    destination.finish_tick().unwrap();
    assert_eq!(due.len(), 2); // Both loaded duplicates ran once; held work was not deduplicated.
    assert!(due.iter().all(|t| t.pos == to && t.kind == TickTypeId(7)));
    assert_eq!(destination.count(), 1);
}

#[test]
fn occupied_install_refuses_without_consuming_a_source_witness() {
    let at = BlockPos::new(1, 64, 1);
    let tick = ScheduledTick {
        kind: TickTypeId(3),
        pos: at,
        trigger_tick: 8,
        priority: TickPriority::Normal,
        sub_tick_order: 2,
    };
    let mut source = LevelTicks::default();
    source.add_chunk(at.chunk(), ChunkTicks::default()).unwrap();
    source.schedule(tick);
    let witness = source
        .capture_transfer_cells(&[[1, 64, 1]].into(), 1)
        .unwrap();
    let mut destination = LevelTicks::default();
    destination
        .add_chunk(at.chunk(), ChunkTicks::default())
        .unwrap();
    destination.schedule(ScheduledTick {
        kind: TickTypeId(99),
        ..tick
    });
    assert_eq!(
        destination.preflight_transfer_install(witness.occurrences()),
        Err(TransferTickError::Occupied)
    );
    source.validate_transfer_cells(&witness).unwrap();
    assert_eq!(source.count(), 1);
    assert_eq!(destination.count(), 1);
}

#[test]
fn empty_saved_sentinels_capture_and_import_actual_work_without_unpacking() {
    let from = BlockPos::new(1, 64, 1);
    let to = BlockPos::new(17, 64, 1);
    let mut source = LevelTicks::default();
    let mut destination = LevelTicks::default();
    source
        .add_chunk(from.chunk(), ChunkTicks::from_saved(Vec::new()))
        .unwrap();
    destination
        .add_chunk(to.chunk(), ChunkTicks::from_saved(Vec::new()))
        .unwrap();
    // Same containers as ordinary register_chunk_ticks when there was no saved work.
    assert_eq!(
        source.chunk(from.chunk()).unwrap().pending,
        Some(Vec::new())
    );
    assert_eq!(
        destination.chunk(to.chunk()).unwrap().pending,
        Some(Vec::new())
    );
    let selected = [[from.x, from.y, from.z]].into();
    assert!(
        source
            .capture_transfer_cells(&selected, 0)
            .unwrap()
            .occurrences()
            .is_empty()
    );
    assert_eq!(
        source.schedule(ScheduledTick {
            kind: TickTypeId(3),
            pos: from,
            trigger_tick: 8,
            priority: TickPriority::Normal,
            sub_tick_order: 2,
        }),
        ScheduleResult::Added
    );
    let witness = source.capture_transfer_cells(&selected, 1).unwrap();
    let mut owned = source.extract_transfer_cells(&witness).unwrap();
    owned[0].tick.pos = to;
    destination.preflight_transfer_install(&owned).unwrap();
    destination.install_transfer_occurrences(owned).unwrap();
    assert_eq!(source.count(), 0);
    assert_eq!(destination.count(), 1);
    destination
        .begin_tick_where(8, 64, |_| true, |_| true)
        .unwrap();
    let actual = destination.next_tick().unwrap().unwrap();
    assert_eq!(actual.pos, to);
    assert_eq!(actual.kind, TickTypeId(3));
    assert!(destination.next_tick().unwrap().is_none());
    destination.finish_tick().unwrap();
    assert_eq!(destination.count(), 0);
    assert_eq!(source.count(), 0);
}
