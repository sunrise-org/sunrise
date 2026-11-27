//! The reference interpreter and the plan interpreter, independent of the planner's liveness.
//!
//! Each holds one value per (resource, level, channel). An event that runs hashes everything it reads into a digest;
//! each value it writes is a hash of its own key, the value's place, that digest and, for a value it may not replace
//! (a partial, unresolved or opaque write), the old value. The interpreters record, as observations, every observer's
//! values and every event's digest together with the old values its partial writes merge (a partial write reads the
//! old value, as the model says). A plan is sound when
//! every event it keeps, and every observer, sees the same thing in both.

use crate::model::{Access, Cover, EventId, EventKind, Graph, Guard, Region, ResourceId, StateId};
use crate::plan::Plan;

/// One frame of a run: its state, and the guards that hold in it (bit `g` for guard `g`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Frame {
    pub state: usize,
    pub guards: u32,
}

/// What an observation is of.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Seen {
    Event(usize),
    Observer(usize),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Observation {
    pub frame: usize,
    pub what: Seen,
    pub digest: u64,
}

/// Flips one written value right after an event of the run's first frame, to see whether anything reads it.
#[derive(Clone, Copy, Debug)]
pub struct Perturb {
    pub event: usize,
    pub cell: usize,
}

/// The value a dead resource holds on the planned path: storage it doesn't have. Any read of it shows.
const POISON: u64 = 0xdead_dead_dead_dead;
/// What a full write mixes in where a partial one mixes the old value.
const REPLACED: u64 = 0x5eed_0000_0000_0001;

fn mix(a: u64, b: u64) -> u64 {
    // splitmix64's finaliser over a combination of both.
    let mut z = a ^ b.wrapping_mul(0x9e37_79b9_7f4a_7c15).rotate_left(29);
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

pub struct Cells {
    offsets: Vec<usize>,
    pub total: usize,
}

impl Cells {
    pub fn new(graph: &Graph) -> Cells {
        let mut offsets = Vec::new();
        let mut total = 0;
        for r in &graph.resources {
            offsets.push(total);
            total += usize::from(r.levels) * usize::from(r.channels);
        }
        Cells { offsets, total }
    }

    fn region(&self, graph: &Graph, region: &Region, out: &mut Vec<usize>) {
        let channels = usize::from(graph.resource(region.resource).channels);
        let base = self.offsets[region.resource.0 as usize];
        for level in region.levels.first..region.levels.first + region.levels.count {
            for c in 0..channels {
                if region.channels & (1 << c) != 0 {
                    out.push(base + usize::from(level) * channels + c);
                }
            }
        }
    }

    fn resource(&self, graph: &Graph, id: ResourceId, out: &mut Vec<usize>) {
        let r = graph.resource(id);
        let base = self.offsets[id.0 as usize];
        out.extend(base..base + usize::from(r.levels) * usize::from(r.channels));
    }

    pub fn access(&self, graph: &Graph, access: &Access, out: &mut Vec<usize>) {
        match access {
            Access::Resolved(region) => self.region(graph, region, out),
            Access::Unresolved(set) => {
                for &id in set {
                    self.resource(graph, id, out);
                }
            }
        }
    }
}

fn digest(values: &[u64], cells: &[usize]) -> u64 {
    cells.iter().fold(0x0123_4567_89ab_cdef, |h, &c| {
        mix(mix(h, c as u64), values[c])
    })
}

fn sorted(mut cells: Vec<usize>) -> Vec<usize> {
    cells.sort_unstable();
    cells.dedup();
    cells
}

fn key(state: usize, index: usize) -> u64 {
    mix(state as u64 + 1, index as u64 + 0x1000)
}

/// Runs `frames` from the initial values. With a plan, runs the planned path: removed events don't run, folded and
/// dead clears don't run, a folded clear's values are written as the draw it folds into loads its attachments
/// (after the draw's reads, before what its attachments pass through), and dead
/// resources hold [`POISON`].
pub fn run(
    graph: &Graph,
    plan: Option<&Plan>,
    frames: &[Frame],
    perturb: Option<Perturb>,
) -> Vec<Observation> {
    let cells = Cells::new(graph);
    let mut values: Vec<u64> = (0..cells.total).map(|c| mix(0xface, c as u64)).collect();
    if let Some(plan) = plan {
        for &dead in &plan.dead {
            let mut out = Vec::new();
            cells.resource(graph, dead, &mut out);
            for c in out {
                values[c] = POISON;
            }
        }
    }
    let mut seen = Vec::new();
    for (f, frame) in frames.iter().enumerate() {
        let events = &graph.states[frame.state].events;
        let observe = |values: &[u64], at: usize, seen: &mut Vec<Observation>| {
            for (o, observer) in graph.observers.iter().enumerate() {
                if observer.state.0 as usize == frame.state && observer.at as usize == at {
                    let mut out = Vec::new();
                    cells.region(graph, &observer.region, &mut out);
                    seen.push(Observation {
                        frame: f,
                        what: Seen::Observer(o),
                        digest: digest(values, &sorted(out)),
                    });
                }
            }
        };
        observe(&values, 0, &mut seen);
        for (i, event) in events.iter().enumerate() {
            let id = EventId {
                state: StateId(frame.state as u32),
                index: i as u32,
            };
            let runs = match event.guard {
                Guard::Always => true,
                Guard::When(g) => frame.guards & (1 << g.0) != 0,
            };
            let skipped = plan.is_some_and(|p| p.is_removed(id));
            if runs && !skipped {
                let digest = execute(graph, &cells, plan, id, &mut values);
                seen.push(Observation {
                    frame: f,
                    what: Seen::Event(i),
                    digest,
                });
            }
            if let Some(p) = perturb.filter(|p| f == 0 && p.event == i) {
                values[p.cell] ^= 0x8000_0000_0000_0001;
            }
            observe(&values, i + 1, &mut seen);
        }
    }
    seen
}

/// The digest of an empty read set.
const EMPTY: u64 = 0x0123_4567_89ab_cdef;

/// Runs one event; returns what it saw.
fn execute(
    graph: &Graph,
    cells: &Cells,
    plan: Option<&Plan>,
    id: EventId,
    values: &mut [u64],
) -> u64 {
    let event = graph.event(id);
    let mut read = Vec::new();
    for access in &event.reads {
        cells.access(graph, access, &mut read);
    }
    let mut full = Vec::new();
    let mut partial = Vec::new();
    for write in &event.writes {
        // A draw's clear-only attachment of a dead resource leaves the draw's attachments.
        if let Some(plan) = plan
            && write.clear
            && let Access::Resolved(region) = &write.access
            && plan.dead.contains(&region.resource)
        {
            continue;
        }
        if let Some(plan) = plan
            && event.kind == EventKind::Clear
        {
            let target = match &write.access {
                Access::Resolved(region) => region.resource,
                Access::Unresolved(_) => unreachable!("validated: a clear's writes are resolved"),
            };
            let folded = plan
                .folded
                .iter()
                .any(|f| f.clear == id && f.resource == target);
            if folded || plan.dead.contains(&target) {
                continue;
            }
        }
        let merges = write.cover == Cover::Partial
            || matches!(write.access, Access::Unresolved(_))
            || event.kind == EventKind::Opaque;
        if event.kind == EventKind::Opaque {
            cells.access(graph, &write.access, &mut read);
        }
        cells.access(
            graph,
            &write.access,
            if merges { &mut partial } else { &mut full },
        );
    }
    let read = sorted(read);
    let partial = sorted(partial);
    let mut written = full;
    written.extend_from_slice(&partial);
    let written = sorted(written);
    // The draw's bindings are read as the pass begins, before its attachments load; a folded clear is the load, so
    // its values land after the reads and before the attachments are drawn over.
    let reads = digest(values, &read);
    if let Some(plan) = plan {
        for fold in plan.folded.iter().filter(|fold| fold.into == id) {
            let mut out = Vec::new();
            cells.resource(graph, fold.resource, &mut out);
            let clear_key = key(fold.clear.state.0 as usize, fold.clear.index as usize);
            for c in out {
                values[c] = mix(mix(clear_key, c as u64), mix(EMPTY, REPLACED));
            }
        }
    }
    let olds = digest(values, &partial);
    let k = key(id.state.0 as usize, id.index as usize);
    let outputs: Vec<(usize, u64)> = written
        .iter()
        .map(|&c| {
            let old = if partial.binary_search(&c).is_ok() {
                values[c]
            } else {
                REPLACED
            };
            (c, mix(mix(k, c as u64), mix(reads, old)))
        })
        .collect();
    for (c, v) in outputs {
        values[c] = v;
    }
    mix(reads, olds)
}

/// The reference's observations minus those of events the plan removed.
pub fn retained(plan: &Plan, frames: &[Frame], seen: Vec<Observation>) -> Vec<Observation> {
    seen.into_iter()
        .filter(|o| match o.what {
            Seen::Event(i) => !plan.is_removed(EventId {
                state: StateId(frames[o.frame].state as u32),
                index: i as u32,
            }),
            Seen::Observer(_) => true,
        })
        .collect()
}

/// The guards any event of the graph names, as a mask.
pub fn guard_mask(graph: &Graph) -> u32 {
    let mut mask = 0;
    for state in &graph.states {
        for event in &state.events {
            if let Guard::When(g) = event.guard {
                mask |= 1 << g.0;
            }
        }
    }
    mask
}

/// Every subset of `mask`.
pub fn subsets(mask: u32) -> Vec<u32> {
    let mut out = vec![0];
    let mut sub = mask;
    while sub != 0 {
        out.push(sub);
        sub = (sub - 1) & mask;
    }
    out
}

/// Every run of at most `depth` frames that starts in `state` and follows the edges, with every guard value in every
/// frame; `visit` gets each run (every prefix that ends where no edge leaves, or at `depth`).
pub fn every_run(graph: &Graph, state: usize, depth: usize, visit: &mut impl FnMut(&[Frame])) {
    let masks = subsets(guard_mask(graph));
    let mut path = Vec::new();
    extend(graph, state, depth, &masks, &mut path, visit);
}

fn extend(
    graph: &Graph,
    state: usize,
    depth: usize,
    masks: &[u32],
    path: &mut Vec<Frame>,
    visit: &mut impl FnMut(&[Frame]),
) {
    for &guards in masks {
        path.push(Frame { state, guards });
        let next: Vec<usize> = graph
            .edges
            .iter()
            .filter(|e| e.from.0 as usize == state)
            .map(|e| e.to.0 as usize)
            .collect();
        if path.len() == depth || next.is_empty() {
            visit(path);
        } else {
            for to in next {
                extend(graph, to, depth, masks, path, visit);
            }
        }
        path.pop();
    }
}

/// Checks the plan against the reference on every run of up to `depth` frames from every state.
pub fn assert_sound(graph: &Graph, plan: &Plan, depth: usize) {
    for state in 0..graph.states.len() {
        every_run(graph, state, depth, &mut |frames| {
            let reference = retained(plan, frames, run(graph, None, frames, None));
            let planned = run(graph, Some(plan), frames, None);
            assert_eq!(
                reference, planned,
                "plan {plan:?} changes what run {frames:?} sees"
            );
        });
    }
}

/// The index of one value, in the interpreters' layout.
pub fn cell_index(graph: &Graph, resource: ResourceId, level: u8, channel: u8) -> usize {
    let cells = Cells::new(graph);
    let channels = usize::from(graph.resource(resource).channels);
    cells.offsets[resource.0 as usize] + usize::from(level) * channels + usize::from(channel)
}
