//! I/O afterimage relocation regressions over actual Chunk payload structures.
use super::*;
use sunrise_client::containers::{Furnace, FurnaceKind};
use sunrise_core::ticks::{SavedTick, TickPriority, TickTypeId};

#[test]
fn io_only_chunk_relocation_preserves_furnace_overlay_components_and_saved_tick_fields() {
    let (from, to) = ((1, 64, 1), (17, 72, 1));
    let mut source = Chunk::empty(0, 0);
    source.set(1, 64, 1, 1);
    source.set(2, 64, 1, 2); // unrelated cell must remain exactly as it was.
    source.block_entities.push((
        from,
        BlockEntity::Furnace(Furnace {
            kind: FurnaceKind::Furnace,
            items: [None, None, None],
            lit_remaining: 173,
            lit_total: 200,
            cooking_timer: 43,
            cooking_total: 200,
            speed: 1.0,
            recipes_used: Default::default(),
        }),
    ));
    let components = Tag::compound([("minecraft:custom_name", Tag::String("saved name".into()))]);
    let original = Tag::compound([
        ("id", Tag::String("minecraft:furnace".into())),
        ("x", Tag::Int(from.0)),
        ("y", Tag::Int(from.1)),
        ("z", Tag::Int(from.2)),
        ("components", components.clone()),
    ]);
    let saved = extras(&mut source);
    saved.block_entity_sources.push((from, original));
    saved.block_entity_order.push(from);
    let value = Tag::compound([("n", Tag::Int(17))]);
    let envelope = Tag::compound([("value", value)]);
    let namespace = Tag::compound([("counter", envelope)]);
    saved.block_entity_attachments.push((
        from,
        Attachments::from_nbt([("transfer_test".into(), namespace)].into()),
    ));
    saved.block_ticks.push(SavedTick {
        kind: TickTypeId(7),
        pos: point([1, 64, 1]),
        delay: 9,
        priority: TickPriority::High,
    });
    let mut destination = Chunk::empty(1, 0);
    let payload = take_saved(&mut source, from, BlockStateId(0));
    put_saved(&mut destination, to, BlockStateId(1), payload).unwrap();
    assert_eq!(source.block(1, 64, 1), BlockStateId(0));
    assert_eq!(source.block(2, 64, 1), BlockStateId(2));
    assert!(source.block_entities.is_empty());
    assert!(
        source
            .saved
            .as_ref()
            .unwrap()
            .block_entity_sources
            .is_empty()
    );
    let Some((at, BlockEntity::Furnace(furnace))) = destination.block_entities.first() else {
        panic!("furnace missing");
    };
    assert_eq!(*at, to);
    assert_eq!((furnace.lit_remaining, furnace.cooking_timer), (173, 43));
    let saved = destination.saved.as_ref().unwrap();
    let original = &saved.block_entity_sources[0].1;
    assert_eq!(original.get("x"), Some(&Tag::Int(17)));
    assert_eq!(original.get("y"), Some(&Tag::Int(72)));
    assert_eq!(original.get("components"), Some(&components));
    assert!(
        !saved.block_entity_attachments[0]
            .1
            .try_to_nbt()
            .unwrap()
            .is_empty()
    );
    // Tick relocation is the separate scheduler-derived step, not a second inventory/tick store.
    let moved = SavedTick {
        pos: point([17, 72, 1]),
        ..source.saved.as_ref().unwrap().block_ticks[0]
    };
    let tag =
        sunrise_world::tick_data::write_list(&[moved], |_| Some("minecraft:stone".into())).unwrap();
    let decoded = sunrise_world::tick_data::read_list(&tag, |_| Some(TickTypeId(7))).unwrap();
    assert_eq!(
        decoded,
        [SavedTick {
            kind: TickTypeId(7),
            pos: point([17, 72, 1]),
            delay: 9,
            priority: TickPriority::High
        }]
    );
}
