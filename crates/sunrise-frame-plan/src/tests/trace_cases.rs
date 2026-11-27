//! The trace projection and its two checks.

use super::*;
use crate::plan::{Fold, Plan, Savings, plan};
use crate::trace::{
    ClearValue, EditError, Edits, Load, Op, Projection, Span, Step, Texture, TextureId, Touch,
    TraceError, TraceFrame, Unit, Use, check_blocks, check_edits, project,
};

/// Black with alpha 1, as the clear bits of an RGBA32F attachment.
const BLACK: ClearValue = ClearValue([0, 0, 0, 0x3f80_0000]);
/// Transparent black.
const NONE: ClearValue = ClearValue([0; 4]);

/// A use of level 0, every channel, of `texture` at `slot`.
fn t(slot: u32, texture: u32, how: Use) -> Touch {
    Touch {
        slot,
        texture: TextureId(texture),
        levels: Levels::BASE,
        channels: 0b1111,
        how,
    }
}

fn op(work: u32, kind: EventKind, removable: bool, touches: Vec<Touch>) -> Op {
    Op::Run {
        work,
        kind,
        effects: false,
        removable,
        touches,
    }
}

/// Textures 0..count: one level, four channels, 64 bytes; texture 4 has two levels.
fn textures(count: u32) -> Vec<Texture> {
    (0..count)
        .map(|id| Texture {
            id: TextureId(id),
            levels: if id == 4 { 2 } else { 1 },
            channels: 4,
            bytes: 64,
        })
        .collect()
}

fn keep() -> Use {
    Use::Attach(Load::Keep)
}

fn block_frame() -> TraceFrame {
    TraceFrame {
        textures: textures(3),
        ops: vec![
            op(1, EventKind::Clear, false, vec![t(0, 0, Use::Clear(BLACK))]),
            Op::BlockStart(7),
            op(
                2,
                EventKind::Work,
                false,
                vec![t(0, 1, Use::Sample), t(0, 0, keep())],
            ),
            op(
                3,
                EventKind::Work,
                false,
                vec![t(0, 1, keep()), t(1, 1, Use::Sample)],
            ),
            Op::BlockEnd(7),
            op(
                4,
                EventKind::Work,
                true,
                vec![t(0, 2, keep()), t(0, 0, Use::Sample)],
            ),
        ],
    }
}

#[test]
fn a_block_projects_to_one_opaque_step() {
    let expected = Projection {
        textures: textures(3),
        steps: vec![
            Step {
                unit: Unit::Work(1),
                kind: EventKind::Clear,
                effects: false,
                removable: false,
                touches: vec![t(0, 0, Use::Clear(BLACK))],
            },
            Step {
                unit: Unit::Block(7),
                kind: EventKind::Opaque,
                effects: true,
                removable: false,
                touches: vec![
                    t(0, 0, keep()),
                    t(0, 1, Use::Sample),
                    t(0, 1, keep()),
                    t(1, 1, Use::Sample),
                ],
            },
            Step {
                unit: Unit::Work(4),
                kind: EventKind::Work,
                effects: false,
                removable: true,
                touches: vec![t(0, 0, Use::Sample), t(0, 2, keep())],
            },
        ],
    };
    assert_eq!(project(&block_frame()), Ok(expected));
}

#[test]
fn the_projection_refuses_unbalanced_blocks_and_unknown_textures() {
    let mut nested = block_frame();
    nested.ops.insert(2, Op::BlockStart(8));
    assert_eq!(project(&nested), Err(TraceError::Block(2)));
    let mut mismatched = block_frame();
    mismatched.ops[4] = Op::BlockEnd(8);
    assert_eq!(project(&mismatched), Err(TraceError::Block(4)));
    let mut stray = block_frame();
    stray.ops.push(Op::BlockEnd(7));
    assert_eq!(project(&stray), Err(TraceError::Block(6)));
    let mut unclosed = block_frame();
    unclosed.ops.remove(4);
    assert_eq!(project(&unclosed), Err(TraceError::Unclosed(7)));
    let mut unknown = block_frame();
    unknown.ops[5] = op(4, EventKind::Work, true, vec![t(0, 3, Use::Sample)]);
    assert_eq!(project(&unknown), Err(TraceError::UnknownTexture(5)));
    let mut twice = block_frame();
    twice.textures.push(twice.textures[1]);
    assert_eq!(
        project(&twice),
        Err(TraceError::DuplicateTexture(TextureId(1)))
    );
}

fn span(texture: u32, first: u8, count: u8, channels: u8) -> Span {
    Span {
        texture: TextureId(texture),
        levels: Levels { first, count },
        channels,
    }
}

#[test]
fn block_coverage_holds_only_inside_the_declared_set() {
    let frame = block_frame();
    let set = |spans: &[Span]| vec![(7, spans.to_vec())];
    assert_eq!(
        check_blocks(
            &frame,
            &set(&[span(0, 0, 1, 0b1111), span(1, 0, 1, 0b1111)])
        ),
        Ok(())
    );
    // Texture 2 is touched outside the block, so it needn't be declared.
    assert_eq!(
        check_blocks(&frame, &set(&[span(0, 0, 1, 0b1111)])),
        Err(TraceError::Outside {
            block: 7,
            texture: TextureId(1)
        })
    );
    assert_eq!(
        check_blocks(
            &frame,
            &set(&[span(0, 0, 1, 0b1111), span(1, 0, 1, 0b0011)])
        ),
        Err(TraceError::Outside {
            block: 7,
            texture: TextureId(1)
        })
    );
    // Channels declared in two spans of the same level count together.
    assert_eq!(
        check_blocks(
            &frame,
            &set(&[
                span(0, 0, 1, 0b1111),
                span(1, 0, 1, 0b0011),
                span(1, 0, 1, 0b1100)
            ])
        ),
        Ok(())
    );
    assert_eq!(check_blocks(&frame, &[]), Err(TraceError::Undeclared(7)));
}

#[test]
fn block_coverage_takes_levels_from_every_declared_span() {
    // A mip generation inside the block reaches both levels of texture 4.
    let frame = TraceFrame {
        textures: textures(5),
        ops: vec![
            Op::BlockStart(7),
            op(
                2,
                EventKind::Work,
                false,
                vec![Touch {
                    levels: Levels { first: 0, count: 2 },
                    ..t(0, 4, Use::Storage)
                }],
            ),
            Op::BlockEnd(7),
        ],
    };
    let both = vec![(7, vec![span(4, 0, 2, 0b1111)])];
    assert_eq!(check_blocks(&frame, &both), Ok(()));
    let split = vec![(7, vec![span(4, 0, 1, 0b1111), span(4, 1, 1, 0b1111)])];
    assert_eq!(check_blocks(&frame, &split), Ok(()));
    let level_0 = vec![(7, vec![span(4, 0, 1, 0b1111)])];
    assert_eq!(
        check_blocks(&frame, &level_0),
        Err(TraceError::Outside {
            block: 7,
            texture: TextureId(4)
        })
    );
    let level_1 = vec![(7, vec![span(4, 1, 1, 0b1111)])];
    assert_eq!(
        check_blocks(&frame, &level_1),
        Err(TraceError::Outside {
            block: 7,
            texture: TextureId(4)
        })
    );
}

/// Clears 0 (black) and 1 (transparent); a removed draw with its bind-time copy; compute kept between; a draw that
/// samples 2 and 3 and draws onto 0.
fn reference() -> Projection {
    project(&TraceFrame {
        textures: textures(4),
        ops: vec![
            op(
                1,
                EventKind::Clear,
                false,
                vec![t(0, 0, Use::Clear(BLACK)), t(1, 1, Use::Clear(NONE))],
            ),
            op(
                2,
                EventKind::Copy,
                false,
                vec![t(0, 2, Use::CopyFrom), t(1, 3, Use::CopyTo)],
            ),
            op(
                3,
                EventKind::Work,
                true,
                vec![t(0, 3, Use::Sample), t(0, 2, keep())],
            ),
            op(4, EventKind::Work, false, vec![t(0, 2, Use::Storage)]),
            op(
                5,
                EventKind::Work,
                false,
                vec![t(0, 2, Use::Sample), t(1, 3, Use::Sample), t(0, 0, keep())],
            ),
        ],
    })
    .expect("valid")
}

/// The planned frame allocates every texture but the dead one, 1.
fn planned(ops: Vec<Op>) -> Projection {
    project(&TraceFrame {
        textures: textures(4)
            .into_iter()
            .filter(|t| t.id != TextureId(1))
            .collect(),
        ops,
    })
    .expect("valid")
}

fn edits() -> Edits {
    Edits {
        removed: vec![2, 3],
        folded: vec![(1, TextureId(0), 5)],
        dead: vec![TextureId(1)],
    }
}

fn compute() -> Op {
    op(4, EventKind::Work, false, vec![t(0, 2, Use::Storage)])
}

fn folded_draw(value: ClearValue) -> Op {
    op(
        5,
        EventKind::Work,
        false,
        vec![
            t(0, 2, Use::Sample),
            t(1, 3, Use::Sample),
            t(0, 0, Use::Attach(Load::Clear(value))),
        ],
    )
}

#[test]
fn a_planned_frame_is_the_reference_with_the_plans_edits() {
    assert_eq!(
        check_edits(
            &reference(),
            &planned(vec![compute(), folded_draw(BLACK)]),
            &edits()
        ),
        Ok(())
    );
}

#[test]
fn a_planned_frame_with_any_other_change_differs() {
    let check = |ops| check_edits(&reference(), &planned(ops), &edits());
    // The compute a removed draw sat beside is gone.
    assert_eq!(check(vec![folded_draw(BLACK)]), Err(TraceError::Differs(0)));
    // The dead texture is still cleared (and so, though nothing else names it, still in use).
    let cleared = op(1, EventKind::Clear, false, vec![t(1, 1, Use::Clear(NONE))]);
    let still_cleared = Projection {
        steps: project(&TraceFrame {
            textures: textures(4),
            ops: vec![cleared, compute(), folded_draw(BLACK)],
        })
        .expect("valid")
        .steps,
        ..planned(Vec::new())
    };
    assert_eq!(
        check_edits(&reference(), &still_cleared, &edits()),
        Err(TraceError::Differs(0))
    );
    // The draw kept its load.
    let kept = op(
        5,
        EventKind::Work,
        false,
        vec![t(0, 2, Use::Sample), t(1, 3, Use::Sample), t(0, 0, keep())],
    );
    assert_eq!(check(vec![compute(), kept]), Err(TraceError::Differs(1)));
    // The folded load clears to other bits than the clear did.
    assert_eq!(
        check(vec![compute(), folded_draw(NONE)]),
        Err(TraceError::Differs(1))
    );
    // The draw samples 2 and 3 in each other's slots.
    let swapped = op(
        5,
        EventKind::Work,
        false,
        vec![
            t(1, 2, Use::Sample),
            t(0, 3, Use::Sample),
            t(0, 0, Use::Attach(Load::Clear(BLACK))),
        ],
    );
    assert_eq!(check(vec![compute(), swapped]), Err(TraceError::Differs(1)));
    // One step too many at the end.
    assert_eq!(
        check(vec![compute(), folded_draw(BLACK), compute()]),
        Err(TraceError::Differs(2))
    );
}

#[test]
fn a_planned_frame_allocates_exactly_the_live_textures() {
    let ops = vec![compute(), folded_draw(BLACK)];
    // The dead texture is still allocated.
    let kept_dead = project(&TraceFrame {
        textures: textures(4),
        ops: ops.clone(),
    })
    .expect("valid");
    assert_eq!(
        check_edits(&reference(), &kept_dead, &edits()),
        Err(TraceError::Allocation(TextureId(1)))
    );
    // A live texture is gone.
    let lost_live = Projection {
        textures: textures(4)
            .into_iter()
            .filter(|t| t.id != TextureId(1) && t.id != TextureId(3))
            .collect(),
        ..planned(ops.clone())
    };
    assert_eq!(
        check_edits(&reference(), &lost_live, &edits()),
        Err(TraceError::Allocation(TextureId(3)))
    );
    // The same textures listed in another order allocate the same.
    let mut reordered = planned(ops);
    reordered.textures.reverse();
    assert_eq!(check_edits(&reference(), &reordered, &edits()), Ok(()));
}

#[test]
fn edits_must_fit_the_reference() {
    let planned = planned(vec![compute(), folded_draw(BLACK)]);
    let dead_but_used = Edits {
        dead: vec![TextureId(1), TextureId(2)],
        ..edits()
    };
    assert_eq!(
        check_edits(&reference(), &planned, &dead_but_used),
        Err(TraceError::DeadTouched(0))
    );
    let no_clear = Edits {
        folded: vec![(1, TextureId(2), 5)],
        ..edits()
    };
    assert_eq!(
        check_edits(&reference(), &planned, &no_clear),
        Err(TraceError::Edit(EditError::NoClear {
            work: 1,
            texture: TextureId(2)
        }))
    );
    let no_attachment = Edits {
        folded: vec![(1, TextureId(0), 4)],
        ..edits()
    };
    assert_eq!(
        check_edits(&reference(), &planned, &no_attachment),
        Err(TraceError::Edit(EditError::NoAttachment {
            work: 4,
            texture: TextureId(0)
        }))
    );
}

#[test]
fn a_projected_frame_plans_like_a_hand_written_graph() {
    let projection = project(&TraceFrame {
        textures: textures(4),
        ops: vec![
            op(1, EventKind::Clear, false, vec![t(0, 0, Use::Clear(BLACK))]),
            op(2, EventKind::Work, true, vec![t(0, 1, keep())]),
            op(
                3,
                EventKind::Work,
                false,
                vec![t(0, 0, keep()), t(0, 1, Use::Sample)],
            ),
        ],
    })
    .expect("valid");
    assert_eq!(
        plan(&projection.to_graph()),
        Ok(Plan {
            removed: Vec::new(),
            folded: vec![Fold {
                clear: id(0, 0),
                resource: ResourceId(0),
                into: id(0, 2),
            }],
            // Textures 2 and 3 are never touched in this frame.
            dead: vec![ResourceId(2), ResourceId(3)],
            savings: Savings {
                events: 0,
                clears_folded: 1,
                clears_dropped: 0,
                bytes: 128,
            },
        })
    );
}

#[test]
fn a_draw_that_already_clears_takes_no_folded_clear() {
    // The draw's own clear load would be replaced by the earlier clear's value.
    let projection = project(&TraceFrame {
        textures: textures(1),
        ops: vec![
            op(1, EventKind::Clear, false, vec![t(0, 0, Use::Clear(BLACK))]),
            op(
                2,
                EventKind::Work,
                false,
                vec![t(0, 0, Use::Attach(Load::Clear(NONE)))],
            ),
        ],
    })
    .expect("valid");
    assert_eq!(plan(&projection.to_graph()), Ok(Plan::default()));
}

#[test]
fn a_clear_step_with_effects_stays_when_its_clears_go() {
    let mut frame = TraceFrame {
        textures: textures(2),
        ops: vec![
            op(1, EventKind::Clear, false, vec![t(0, 1, Use::Clear(NONE))]),
            op(2, EventKind::Work, false, vec![t(0, 0, keep())]),
        ],
    };
    if let Op::Run { effects, .. } = &mut frame.ops[0] {
        *effects = true;
    }
    let reference = project(&frame).expect("valid");
    let edits = Edits {
        dead: vec![TextureId(1)],
        ..Edits::default()
    };
    let without = project(&TraceFrame {
        textures: textures(1),
        ops: vec![op(2, EventKind::Work, false, vec![t(0, 0, keep())])],
    })
    .expect("valid");
    assert_eq!(
        check_edits(&reference, &without, &edits),
        Err(TraceError::Differs(0))
    );
    let mut with = project(&TraceFrame {
        textures: textures(1),
        ops: vec![
            op(1, EventKind::Clear, false, Vec::new()),
            op(2, EventKind::Work, false, vec![t(0, 0, keep())]),
        ],
    })
    .expect("valid");
    with.steps[0].effects = true;
    assert_eq!(check_edits(&reference, &with, &edits), Ok(()));
}
