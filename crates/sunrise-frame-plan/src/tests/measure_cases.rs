//! The measure on hand-written runs, each with its literal outcome.

use crate::measure::{
    Callbacks, Dead, Folded, MeasureError, Removed, Report, Sized, Work, measure,
};
use crate::model::{EventKind, Levels};
use crate::plan::Savings;
use crate::record::{BlockNote, Recorded, Start, TextureNote};
use crate::trace::{
    ClearValue, Load, Op, Span, Texture, TextureId, Touch, TraceError, TraceFrame, Use,
};

const BLACK: ClearValue = ClearValue([0, 0, 0, 0x3f80_0000]);

fn t(slot: u32, texture: u32, how: Use) -> Touch {
    Touch {
        slot,
        texture: TextureId(texture),
        levels: Levels::BASE,
        channels: 0b1111,
        how,
    }
}

fn run(work: u32, kind: EventKind, removable: bool, touches: Vec<Touch>) -> Op {
    Op::Run {
        work,
        kind,
        effects: false,
        removable,
        touches,
    }
}

fn keep() -> Use {
    Use::Attach(Load::Keep)
}

/// Texture 0 is colortex0 (pinned), 1 and 2 colortex1's A and B (B falls back to A), 3 the screen (pinned). Each
/// has its own size, so a dead texture's bytes say which it is.
fn notes() -> (Vec<Texture>, Vec<TextureNote>) {
    let textures = (0..4)
        .map(|id| Texture {
            id: TextureId(id),
            levels: 1,
            channels: 4,
            bytes: 100 + u64::from(id),
        })
        .collect();
    let note = |id, pinned, alias: Option<u32>, label: &str| TextureNote {
        id: TextureId(id),
        pinned,
        alias: alias.map(TextureId),
        label: label.to_owned(),
    };
    let notes = vec![
        note(0, true, None, "colortex0 A"),
        note(1, false, None, "colortex1 A"),
        note(2, false, Some(1), "colortex1 B"),
        note(3, true, None, "screen"),
    ];
    (textures, notes)
}

/// Clear colortex1 B (unit 0) and A (unit 1); draw 2 writes colortex1 B, which nothing reads before the next frame
/// clears it; draw 3 writes colortex1 A, which `final` (4) reads onto the screen; then the after-level callback
/// block 9, which may reach both sides of colortex1.
fn frame(seq: u64) -> Recorded {
    let (textures, notes) = notes();
    Recorded {
        seq,
        size: [854, 480],
        start: if seq == 0 {
            Start::First
        } else {
            Start::Steady
        },
        frame: TraceFrame {
            textures,
            ops: vec![
                run(0, EventKind::Clear, false, vec![t(0, 2, Use::Clear(BLACK))]),
                run(1, EventKind::Clear, false, vec![t(0, 1, Use::Clear(BLACK))]),
                run(2, EventKind::Work, true, vec![t(0, 2, keep())]),
                run(3, EventKind::Work, true, vec![t(0, 1, keep())]),
                Op::BlockStart(9),
                Op::BlockEnd(9),
                run(
                    4,
                    EventKind::Work,
                    false,
                    vec![
                        t(0, 1, Use::Sample),
                        t(0, 3, Use::Attach(Load::Clear(BLACK))),
                    ],
                ),
            ],
        },
        textures: notes,
        works: vec![
            (0, "clear B".to_owned()),
            (1, "clear".to_owned()),
            (2, "composite".to_owned()),
            (3, "composite1".to_owned()),
            (4, "final".to_owned()),
        ],
        blocks: vec![BlockNote {
            block: 9,
            label: "after level".to_owned(),
            declared: Vec::new(),
            unresolved: vec![TextureId(1), TextureId(2)],
        }],
        guards: vec![("a guard".to_owned(), seq.is_multiple_of(2))],
        errors: Vec::new(),
    }
}

/// The run's only size.
fn one(report: Report) -> Sized {
    assert_eq!(report.sizes.len(), 1);
    report.sizes.into_iter().next().expect("one size")
}

#[test]
fn narrowed_callbacks_let_each_rule_fire_once() {
    let report = one(measure(&[frame(0), frame(1)], Callbacks::Narrowed).expect("measured"));
    assert_eq!(report.frames.len(), 2);
    assert!(
        report
            .frames
            .iter()
            .all(|f| f.state == 0 && f.blocks.is_ok())
    );
    assert_eq!(
        report.states,
        vec![(
            Work {
                passes: 5,
                clears: 2,
                blocks: 1
            },
            Ok(())
        )]
    );
    assert_eq!(report.size, [854, 480]);
    assert_eq!(
        (report.allocated, report.plannable),
        (100 + 101 + 102 + 103, 101 + 102)
    );
    assert_eq!(report.dead_bytes, 102);
    assert_eq!(report.frames[1].guards, [("a guard".to_owned(), false)]);
    assert_eq!(
        report.removed,
        vec![Removed {
            state: 0,
            work: 2,
            label: "composite".to_owned()
        }]
    );
    assert_eq!(
        report.folded,
        vec![Folded {
            state: 0,
            clear: 1,
            texture: TextureId(1),
            draw: 3,
            draw_label: "composite1".to_owned()
        }]
    );
    assert_eq!(
        report.dead,
        vec![Dead {
            texture: TextureId(2),
            label: "colortex1 B".to_owned(),
            bytes: 102
        }]
    );
    assert_eq!(
        report.savings,
        Savings {
            events: 1,
            clears_folded: 1,
            clears_dropped: 1,
            bytes: 102
        }
    );
}

#[test]
fn an_unresolved_callback_keeps_every_side_it_may_reach() {
    let report = one(measure(&[frame(0), frame(1)], Callbacks::Unresolved).expect("measured"));
    assert_eq!(report.removed, Vec::new());
    assert_eq!(report.folded, Vec::new());
    assert_eq!(report.dead, Vec::new());
    assert_eq!(report.savings, Savings::default());
    assert_eq!(report.states[0].1, Ok(()));
}

#[test]
fn a_declared_span_is_part_of_the_block_even_when_nothing_touched_it() {
    // The block declares colortex1 B: it may read it, so draw 2's write stays, B is no longer dead, and B's clear
    // folds into draw 2 as A's does into draw 3.
    let mut f = frame(0);
    f.blocks[0].declared.push(Span {
        texture: TextureId(2),
        levels: Levels::BASE,
        channels: 0b1111,
    });
    let report = one(measure(&[f], Callbacks::Narrowed).expect("measured"));
    assert_eq!(report.removed, Vec::new());
    assert_eq!(report.dead, Vec::new());
    let folds: Vec<(u32, TextureId, u32)> = report
        .folded
        .iter()
        .map(|f| (f.clear, f.texture, f.draw))
        .collect();
    assert_eq!(folds, vec![(0, TextureId(2), 2), (1, TextureId(1), 3)]);
}

#[test]
fn a_touch_outside_the_declared_set_fails_that_frame_s_block_check() {
    let mut f = frame(3);
    let Op::BlockStart(_) = f.frame.ops[4] else {
        unreachable!("the block starts at op 4")
    };
    f.frame.ops.insert(
        5,
        run(5, EventKind::Work, false, vec![t(0, 2, Use::Sample)]),
    );
    let report = one(measure(&[frame(2), f], Callbacks::Narrowed).expect("measured"));
    assert_eq!(report.frames[0].blocks, Ok(()));
    assert_eq!(
        report.frames[1].blocks,
        Err(TraceError::Outside {
            block: 9,
            texture: TextureId(2)
        })
    );
    // A frame with other steps is a state of its own.
    assert_eq!(report.frames[1].state, 1);
    assert_eq!(report.states.len(), 2);
}

#[test]
fn a_value_one_state_writes_and_another_reads_stays_live() {
    // Frame 1 is another state whose first draw samples colortex1 B before its clear: what draw 2 wrote in the
    // frame before is read, so draw 2 stays in both states.
    let mut reader = frame(1);
    reader.frame.ops.insert(
        0,
        run(
            6,
            EventKind::Work,
            true,
            vec![t(0, 2, Use::Sample), t(1, 1, keep())],
        ),
    );
    let report = one(measure(&[frame(0), reader], Callbacks::Narrowed).expect("measured"));
    assert_eq!(report.states.len(), 2);
    assert!(!report.removed.iter().any(|r| r.work == 2));
    assert_eq!(report.dead, Vec::new());
}

#[test]
fn a_draw_that_only_builds_on_its_own_last_write_stays() {
    // Without the clear of colortex1 B, draw 2's next write is `Partial`: it reads what it wrote a frame before,
    // so the write stays live and the draw stays, though nothing else reads B.
    let mut f = frame(0);
    f.frame.ops.remove(0);
    let report = one(measure(&[f], Callbacks::Narrowed).expect("measured"));
    assert_eq!(report.removed, Vec::new());
    assert_eq!(report.dead, Vec::new());
}

#[test]
fn a_run_without_frames_or_with_conflicting_sizes_is_refused() {
    assert_eq!(
        measure(&[], Callbacks::Unresolved),
        Err(MeasureError::Empty)
    );
    let mut other = frame(1);
    other.frame.textures[2].bytes = 5;
    assert_eq!(
        measure(&[frame(0), other], Callbacks::Unresolved),
        Err(MeasureError::Texture(TextureId(2)))
    );
    let mut open = frame(0);
    open.frame.ops.push(Op::BlockStart(4));
    assert_eq!(
        measure(&[open], Callbacks::Unresolved),
        Err(MeasureError::Project(0, TraceError::Unclosed(4)))
    );
}

#[test]
fn each_render_size_is_planned_and_counted_apart() {
    // Frame 2 is drawn at another size with textures of its own (ids 10 to 13, ten times the bytes): two plans, each
    // counting only its own frames' textures.
    let mut small = frame(2);
    small.size = [427, 240];
    for t in &mut small.frame.textures {
        t.id = TextureId(t.id.0 + 10);
        t.bytes *= 10;
    }
    for n in &mut small.textures {
        n.id = TextureId(n.id.0 + 10);
        n.alias = n.alias.map(|a| TextureId(a.0 + 10));
    }
    for op in &mut small.frame.ops {
        if let Op::Run { touches, .. } = op {
            for t in touches {
                t.texture = TextureId(t.texture.0 + 10);
            }
        }
    }
    let report = measure(&[frame(0), small, frame(1)], Callbacks::Narrowed).expect("measured");
    let sizes: Vec<_> = report
        .sizes
        .iter()
        .map(|s| {
            (
                s.size,
                s.frames.len(),
                s.allocated,
                s.plannable,
                s.dead_bytes,
            )
        })
        .collect();
    assert_eq!(
        sizes,
        [
            ([854, 480], 2, 406, 203, 102),
            ([427, 240], 1, 4060, 2030, 1020)
        ]
    );
}
