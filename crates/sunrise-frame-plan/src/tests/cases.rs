//! One input per rule with its literal plan, one per guard of each rule that makes the rule refuse, and one per
//! conservative case of the spec. Every plan is also run through the interpreters on every run of up to four frames
//! from every state, with every guard value.

use super::interp::assert_sound;
use super::*;
use crate::plan::{Fold, Plan, Rule, Savings, plan};

const A: u32 = 0;
/// A mipmapped resource (two levels), which every rule leaves alone.
const M: u32 = 1;
const B: u32 = 2;
const C: u32 = 3;

/// The first `count` of a, m, b and c: four channels each, 100, 400, 200 and 300 bytes, m with two levels. A test
/// takes only the resources it uses (m is never dead), since R2 finds a resource nothing uses dead.
fn resources(count: usize) -> Vec<Resource> {
    let mut all = vec![
        res(4, 100),
        Resource {
            levels: 2,
            ..res(4, 400)
        },
        res(4, 200),
        res(4, 300),
    ];
    all.truncate(count);
    all
}

struct Expect<'a> {
    removed: &'a [(u32, u32)],
    folded: &'a [((u32, u32), u32, (u32, u32))],
    dead: &'a [u32],
    clears_dropped: u32,
    bytes: u64,
}

const NOTHING: Expect<'static> = Expect {
    removed: &[],
    folded: &[],
    dead: &[],
    clears_dropped: 0,
    bytes: 0,
};

fn check(graph: &Graph, expect: Expect) {
    let expected = Plan {
        removed: expect
            .removed
            .iter()
            .map(|&(s, i)| (id(s, i), Rule::DeadDraw))
            .collect(),
        folded: expect
            .folded
            .iter()
            .map(|&((cs, ci), r, (ds, di))| Fold {
                clear: id(cs, ci),
                resource: ResourceId(r),
                into: id(ds, di),
            })
            .collect(),
        dead: expect.dead.iter().map(|&r| ResourceId(r)).collect(),
        savings: Savings {
            events: expect.removed.len() as u32,
            clears_folded: expect.folded.len() as u32,
            clears_dropped: expect.clears_dropped,
            bytes: expect.bytes,
        },
    };
    let planned = plan(graph).expect("valid input");
    assert_eq!(planned, expected);
    assert_sound(graph, &planned, 4);
}

fn copy(from: u32, to: u32, removable: bool) -> Event {
    Event {
        kind: EventKind::Copy,
        guard: Guard::Always,
        reads: vec![whole(from)],
        writes: vec![Write {
            access: whole(to),
            cover: Cover::Full,
            attachment: false,
        }],
        effects: false,
        removable,
    }
}

fn unresolved_reader(set: &[u32]) -> Event {
    Event {
        reads: vec![Access::Unresolved(
            set.iter().map(|&r| ResourceId(r)).collect(),
        )],
        writes: Vec::new(),
        ..opaque(&[], true)
    }
}

/// A second state, not reachable from the first, whose only event reaches `set` unresolved.
fn with_unresolved(mut graph: Graph, set: &[u32]) -> Graph {
    graph.states.push(State {
        events: vec![unresolved_reader(set)],
    });
    graph
}

fn chain(resources: Vec<Resource>, states: Vec<Vec<Event>>, transitions: &[Transition]) -> Graph {
    let count = states.len() as u32;
    Graph {
        resources,
        states: states.into_iter().map(|events| State { events }).collect(),
        edges: (0..count)
            .map(|s| Edge {
                from: StateId(s),
                transition: transitions[s as usize % transitions.len()],
                to: StateId((s + 1) % count),
            })
            .collect(),
        observers: Vec::new(),
    }
}

fn observe(graph: &mut Graph, state: u32, at: u32, resource: u32) {
    graph.observers.push(Observer {
        state: StateId(state),
        at,
        region: rgba(resource),
    });
}

// R1, dead draw.

#[test]
fn r1_removes_a_draw_a_clear_overwrites_before_any_read() {
    let graph = looped(
        resources(3),
        vec![draw(&[], &[A]), clear(&[A]), kept(&[A], &[B])],
        &[B],
    );
    check(
        &graph,
        Expect {
            removed: &[(0, 0)],
            ..NOTHING
        },
    );
}

#[test]
fn r1_keeps_a_draw_read_later_in_its_frame() {
    let graph = looped(resources(3), vec![draw(&[], &[A]), kept(&[A], &[B])], &[B]);
    check(&graph, NOTHING);
}

#[test]
fn r1_removes_only_work() {
    let graph = looped(
        resources(3),
        vec![copy(B, A, true), clear(&[A]), kept(&[A], &[B])],
        &[B],
    );
    check(&graph, NOTHING);
}

#[test]
fn r1_keeps_what_the_adapter_keeps() {
    // Effects (an image, a storage buffer, compute, setup, the dump) and the output route are never removable.
    let graph = looped(
        resources(3),
        vec![kept(&[], &[A]), clear(&[A]), kept(&[A], &[B])],
        &[B],
    );
    check(&graph, NOTHING);
}

#[test]
fn r1_keeps_a_draw_touching_a_mipmapped_resource() {
    // A mip request: the draw samples m, whose mip state isn't carried.
    let graph = looped(
        resources(3),
        vec![draw(&[M], &[A]), clear(&[A]), kept(&[A], &[B])],
        &[B],
    );
    check(&graph, NOTHING);
}

#[test]
fn r1_keeps_a_draw_onto_a_pinned_resource() {
    let mut resources = resources(3);
    resources[A as usize].pinned = true;
    let graph = looped(
        resources,
        vec![draw(&[], &[A]), clear(&[A]), kept(&[A], &[B])],
        &[B],
    );
    check(&graph, NOTHING);
}

#[test]
fn r1_keeps_a_draw_onto_a_resource_an_unresolved_access_reaches() {
    let graph = with_unresolved(
        looped(
            resources(3),
            vec![draw(&[], &[A]), clear(&[A]), kept(&[A], &[B])],
            &[B],
        ),
        &[A],
    );
    check(&graph, NOTHING);
}

#[test]
fn r1_keeps_a_draw_with_an_unresolved_write() {
    let mut unresolved = draw(&[], &[]);
    unresolved.writes.push(Write {
        access: Access::Unresolved(vec![ResourceId(A)]),
        cover: Cover::Partial,
        attachment: false,
    });
    let graph = looped(
        resources(3),
        vec![unresolved, clear(&[A]), kept(&[A], &[B])],
        &[B],
    );
    check(&graph, NOTHING);
}

#[test]
fn r1_keeps_a_draw_that_reads_a_shadow_side() {
    // c stands for a shadow side (pinned): the draw's only reason to stay is that it samples it, resolved or not.
    let mut resources = resources(4);
    resources[C as usize].pinned = true;
    let resolved = looped(
        resources.clone(),
        vec![draw(&[C], &[A]), clear(&[A]), kept(&[A], &[B])],
        &[B],
    );
    check(&resolved, NOTHING);
    let mut sampler = draw(&[], &[A]);
    sampler.reads.push(Access::Unresolved(vec![ResourceId(C)]));
    let unresolved = looped(
        resources,
        vec![sampler, clear(&[A]), kept(&[A], &[B])],
        &[B],
    );
    check(&unresolved, NOTHING);
}

#[test]
fn r1_keeps_a_value_read_two_frames_later() {
    // Written in a frame of state 0, read in state 2, two frames on.
    let mut graph = chain(
        resources(3),
        vec![vec![draw(&[], &[A])], vec![], vec![kept(&[A], &[B])]],
        &[Transition::Steady],
    );
    observe(&mut graph, 2, 1, B);
    check(&graph, NOTHING);
}

#[test]
fn r1_removes_a_value_cleared_in_the_frame_between() {
    let mut graph = chain(
        resources(3),
        vec![
            vec![draw(&[], &[A])],
            vec![clear(&[A])],
            vec![kept(&[A], &[B])],
        ],
        &[Transition::Steady],
    );
    observe(&mut graph, 2, 1, B);
    check(
        &graph,
        Expect {
            removed: &[(0, 0)],
            ..NOTHING
        },
    );
}

#[test]
fn r1_keeps_a_value_read_after_a_reset_or_a_resize() {
    for transition in [Transition::Reset, Transition::Resize([854, 480])] {
        let mut graph = chain(
            resources(3),
            vec![vec![draw(&[], &[A])], vec![kept(&[A], &[B])]],
            &[transition],
        );
        observe(&mut graph, 1, 1, B);
        check(&graph, NOTHING);
    }
}

#[test]
fn r1_follows_a_resize_that_skips_a_pass() {
    // At the first size the second pass (a clear standing for a pass that replaces a) runs; at the second size its
    // buffers' sizes differ and it is skipped, so the next pass reads what the first drew.
    let mut graph = chain(
        resources(3),
        vec![
            vec![draw(&[], &[A]), clear(&[A]), kept(&[A], &[B])],
            vec![draw(&[], &[A]), kept(&[A], &[B])],
        ],
        &[
            Transition::Resize([1920, 1080]),
            Transition::Resize([854, 480]),
        ],
    );
    observe(&mut graph, 0, 3, B);
    observe(&mut graph, 1, 2, B);
    check(
        &graph,
        Expect {
            removed: &[(0, 0)],
            ..NOTHING
        },
    );
}

#[test]
fn r1_keeps_a_draw_a_guarded_clear_may_not_overwrite() {
    let graph = looped(
        resources(3),
        vec![draw(&[], &[A]), guarded(clear(&[A]), 0), kept(&[A], &[B])],
        &[B],
    );
    check(&graph, NOTHING);
}

#[test]
fn r1_keeps_a_draw_a_partial_write_passes_through() {
    let graph = looped(
        resources(3),
        vec![draw(&[], &[A]), draw(&[], &[A]), kept(&[A], &[B])],
        &[B],
    );
    check(&graph, NOTHING);
}

#[test]
fn r1_keeps_a_draw_an_opaque_block_may_read() {
    // A gbuffers block, callback or mod slot, guarded, between the draw and the clear.
    let graph = looped(
        resources(3),
        vec![
            draw(&[], &[A]),
            guarded(opaque(&[A], false), 1),
            clear(&[A]),
            kept(&[A], &[B]),
        ],
        &[B],
    );
    check(&graph, NOTHING);
}

#[test]
fn r1_keeps_a_draw_an_opaque_block_writes_over() {
    // An opaque block's writes pass every value through, whatever cover they claim: it kills nothing.
    let mut block = opaque(&[], false);
    block.writes.push(Write {
        access: whole(A),
        cover: Cover::Full,
        attachment: false,
    });
    let graph = looped(
        resources(3),
        vec![draw(&[], &[A]), block, kept(&[A], &[B])],
        &[B],
    );
    check(&graph, NOTHING);
}

#[test]
fn r1_keeps_a_draw_an_observer_reads() {
    let mut graph = looped(resources(1), vec![draw(&[], &[A]), clear(&[A])], &[]);
    observe(&mut graph, 0, 1, A);
    check(&graph, NOTHING);
}

// R4b, folded clear.

#[test]
fn r4b_folds_a_clear_into_the_next_draw_onto_it() {
    let graph = looped(resources(1), vec![clear(&[A]), kept(&[], &[A])], &[A]);
    check(
        &graph,
        Expect {
            folded: &[((0, 0), A, (0, 1))],
            ..NOTHING
        },
    );
}

#[test]
fn r4b_folds_one_attachment_of_a_shared_clear() {
    // The clear pass keeps b, which the draw doesn't attach.
    let graph = looped(
        resources(4),
        vec![clear(&[A, B]), kept(&[], &[A]), kept(&[B], &[C])],
        &[A, C],
    );
    check(
        &graph,
        Expect {
            folded: &[((0, 0), A, (0, 1))],
            ..NOTHING
        },
    );
}

#[test]
fn r4b_refuses_a_guarded_clear() {
    let graph = looped(
        resources(1),
        vec![guarded(clear(&[A]), 0), kept(&[], &[A])],
        &[A],
    );
    check(&graph, NOTHING);
}

#[test]
fn r4b_refuses_a_guarded_draw() {
    let graph = looped(
        resources(1),
        vec![clear(&[A]), guarded(kept(&[], &[A]), 0)],
        &[A],
    );
    check(&graph, NOTHING);
}

#[test]
fn r4b_refuses_a_clear_of_part_of_the_resource() {
    let mut part = clear(&[]);
    part.writes.push(Write {
        access: Access::Resolved(Region {
            channels: 0b0111,
            ..rgba(A)
        }),
        cover: Cover::Full,
        attachment: false,
    });
    let graph = looped(resources(1), vec![part, kept(&[], &[A])], &[A]);
    check(&graph, NOTHING);
}

#[test]
fn r4b_refuses_a_pinned_resource() {
    let mut resources = resources(1);
    resources[A as usize].pinned = true;
    let graph = looped(resources, vec![clear(&[A]), kept(&[], &[A])], &[A]);
    check(&graph, NOTHING);
}

#[test]
fn r4b_refuses_a_resource_an_unresolved_access_reaches() {
    let graph = with_unresolved(
        looped(resources(1), vec![clear(&[A]), kept(&[], &[A])], &[A]),
        &[A],
    );
    check(&graph, NOTHING);
}

#[test]
fn r4b_refuses_a_clear_that_touches_a_mipmapped_resource() {
    let mut both = clear(&[A]);
    both.writes.push(Write {
        access: Access::Resolved(Region {
            resource: ResourceId(M),
            levels: Levels { first: 0, count: 2 },
            channels: 0b1111,
        }),
        cover: Cover::Full,
        attachment: false,
    });
    let graph = looped(resources(2), vec![both, kept(&[], &[A])], &[A]);
    check(&graph, NOTHING);
}

#[test]
fn r4b_refuses_a_draw_that_touches_a_mipmapped_resource() {
    let graph = looped(resources(2), vec![clear(&[A]), kept(&[M], &[A])], &[A]);
    check(&graph, NOTHING);
}

#[test]
fn r4b_refuses_an_observer_at_the_clear_boundary() {
    let mut graph = looped(resources(1), vec![clear(&[A]), kept(&[], &[A])], &[A]);
    observe(&mut graph, 0, 1, A);
    check(&graph, NOTHING);
}

#[test]
fn r4b_refuses_a_mod_slot_that_samples_the_cleared_side() {
    // The spec's counter-case: the mod slot samples a between the clear and the draw and writes b, which nothing
    // reads. The final pixels would agree with the fold; the mod slot's read would not.
    let graph = looped(
        resources(3),
        vec![clear(&[A]), opaque(&[A, B], false), kept(&[], &[A])],
        &[A],
    );
    check(&graph, NOTHING);
}

#[test]
fn r4b_folds_only_the_last_of_two_clears() {
    let graph = looped(
        resources(1),
        vec![clear(&[A]), clear(&[A]), kept(&[], &[A])],
        &[A],
    );
    check(
        &graph,
        Expect {
            folded: &[((0, 1), A, (0, 2))],
            ..NOTHING
        },
    );
}

#[test]
fn r4b_refuses_a_side_the_draws_bind_time_copy_reads() {
    let graph = looped(
        resources(3),
        vec![clear(&[A]), copy(A, B, false), kept(&[B], &[A])],
        &[A],
    );
    check(&graph, NOTHING);
}

#[test]
fn r4b_refuses_a_draw_that_samples_the_side() {
    let graph = looped(resources(1), vec![clear(&[A]), kept(&[A], &[A])], &[A]);
    check(&graph, NOTHING);
}

#[test]
fn r4b_refuses_a_write_that_is_not_an_attachment() {
    let mut storage = kept(&[], &[A]);
    storage.writes[0].attachment = false;
    let graph = looped(resources(1), vec![clear(&[A]), storage], &[A]);
    check(&graph, NOTHING);
}

#[test]
fn r4b_refuses_an_attachment_of_part_of_the_side() {
    let mut part = kept(&[], &[A]);
    part.writes[0].access = Access::Resolved(Region {
        channels: 0b0011,
        ..rgba(A)
    });
    let graph = looped(resources(1), vec![clear(&[A]), part], &[A]);
    check(&graph, NOTHING);
}

#[test]
fn r4b_refuses_a_draw_r1_removes() {
    let graph = looped(
        resources(3),
        vec![clear(&[A]), draw(&[], &[A]), clear(&[A]), kept(&[A], &[B])],
        &[B],
    );
    check(
        &graph,
        Expect {
            removed: &[(0, 1)],
            ..NOTHING
        },
    );
}

// R2, dead side.

#[test]
fn r2_finds_a_side_that_is_only_cleared() {
    let graph = looped(resources(3), vec![clear(&[A]), kept(&[], &[B])], &[B]);
    check(
        &graph,
        Expect {
            dead: &[A],
            clears_dropped: 1,
            bytes: 100,
            ..NOTHING
        },
    );
}

#[test]
fn r2_counts_after_r1() {
    // The only reader of a is a removed draw, whose own target c nothing reads before the next frame's clear.
    let graph = looped(
        resources(4),
        vec![clear(&[A, C]), draw(&[A], &[C]), kept(&[], &[B])],
        &[B],
    );
    check(
        &graph,
        Expect {
            removed: &[(0, 1)],
            dead: &[A, C],
            clears_dropped: 2,
            bytes: 400,
            ..NOTHING
        },
    );
}

#[test]
fn r2_keeps_a_side_a_mipmapped_clear_also_clears() {
    // The shared clear is a's only use, but it also clears m, so every rule leaves it, and a, alone.
    let mut shared = clear(&[A]);
    shared.writes.push(Write {
        access: Access::Resolved(Region {
            resource: ResourceId(M),
            levels: Levels { first: 0, count: 2 },
            channels: 0b1111,
        }),
        cover: Cover::Full,
        attachment: false,
    });
    let graph = looped(resources(3), vec![shared, kept(&[], &[B])], &[B]);
    check(&graph, NOTHING);
}

#[test]
fn r2_keeps_a_side_a_retained_event_reads() {
    let graph = looped(resources(3), vec![clear(&[A]), kept(&[A], &[B])], &[B]);
    check(&graph, NOTHING);
}

#[test]
fn r2_keeps_a_side_an_opaque_block_reaches() {
    let graph = looped(
        resources(3),
        vec![clear(&[A]), opaque(&[A], true), kept(&[], &[B])],
        &[B],
    );
    check(&graph, NOTHING);
}

#[test]
fn r2_keeps_an_observed_side() {
    let graph = looped(resources(3), vec![clear(&[A]), kept(&[], &[B])], &[A, B]);
    check(&graph, NOTHING);
}

#[test]
fn r2_keeps_a_copy_destination() {
    let graph = looped(resources(3), vec![copy(B, A, false), kept(&[], &[B])], &[B]);
    check(&graph, NOTHING);
}

#[test]
fn r2_keeps_a_pinned_side() {
    let mut resources = resources(3);
    resources[A as usize].pinned = true;
    let graph = looped(resources, vec![clear(&[A]), kept(&[], &[B])], &[B]);
    check(&graph, NOTHING);
}

/// a, m, b as a's B side (aliasing a), and c.
fn sides() -> Vec<Resource> {
    let mut sides = resources(4);
    sides[B as usize].alias = Some(ResourceId(A));
    sides
}

#[test]
fn r2_finds_a_dead_b_side_beside_a_live_a_side() {
    let graph = looped(sides(), vec![clear(&[A, B]), kept(&[A], &[C])], &[C]);
    check(
        &graph,
        Expect {
            dead: &[B],
            clears_dropped: 1,
            bytes: 200,
            ..NOTHING
        },
    );
}

#[test]
fn r2_keeps_a_dead_a_side_beside_a_live_b_side() {
    let graph = looped(sides(), vec![clear(&[A, B]), kept(&[B], &[C])], &[C]);
    check(&graph, NOTHING);
}

#[test]
fn r2_finds_both_sides_dead() {
    let graph = looped(sides(), vec![clear(&[A, B]), kept(&[], &[C])], &[C]);
    check(
        &graph,
        Expect {
            dead: &[A, B],
            clears_dropped: 2,
            bytes: 300,
            ..NOTHING
        },
    );
}

// Rules together.

#[test]
fn rules_combine_in_one_frame() {
    // a: drawn, then cleared, then drawn again and read: R1 removes the first draw, and the clear folds into the
    // second. b: cleared, then attached by the last draw: folded. c: only cleared: dead.
    let graph = looped(
        resources(4),
        vec![
            draw(&[], &[A]),
            clear(&[A, B, C]),
            kept(&[], &[A]),
            kept(&[A], &[B]),
        ],
        &[B],
    );
    check(
        &graph,
        Expect {
            removed: &[(0, 0)],
            folded: &[((0, 1), A, (0, 2)), ((0, 1), B, (0, 3))],
            dead: &[C],
            clears_dropped: 1,
            bytes: 300,
        },
    );
}

// The interpreters themselves.

#[test]
fn the_interpreters_see_a_fold_into_a_draw_that_samples_the_side() {
    // A plan the planner refuses (r4b_refuses_a_draw_that_samples_the_side), given by hand: the draw's sampler
    // reads a as the pass begins, before the folded load would clear it, so it sees the previous frame's a instead
    // of the cleared one.
    let graph = looped(resources(1), vec![clear(&[A]), kept(&[A], &[A])], &[A]);
    let unsound = Plan {
        folded: vec![Fold {
            clear: id(0, 0),
            resource: ResourceId(A),
            into: id(0, 1),
        }],
        ..Plan::default()
    };
    let frames = [super::interp::Frame {
        state: 0,
        guards: 0,
    }];
    let reference = super::interp::run(&graph, None, &frames, None);
    let planned = super::interp::run(&graph, Some(&unsound), &frames, None);
    assert_eq!(reference.len(), planned.len());
    // The clear reads nothing either way; the draw's own observation differs.
    assert_eq!(reference[0], planned[0]);
    assert_eq!(reference[1].what, planned[1].what);
    assert_ne!(reference[1], planned[1]);
}
