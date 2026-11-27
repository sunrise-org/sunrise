//! The seeded property test: random graphs, planned, then run through the reference and the plan interpreter on
//! random runs of up to eight frames, with random guard values per frame and random edges (resets and resizes
//! among them). A failure prints its seed.

use super::interp::{Frame, guard_mask, retained, run, subsets};
use super::*;
use crate::plan::plan;

/// xorshift64, seeded through splitmix64 so that nearby seeds differ.
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Rng {
        let mut z = seed.wrapping_add(0x9e37_79b9_7f4a_7c15);
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        Rng((z ^ (z >> 31)) | 1)
    }

    pub fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    /// A number in `0..n`.
    pub fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }

    /// True `percent` times in a hundred.
    pub fn chance(&mut self, percent: u64) -> bool {
        self.next() % 100 < percent
    }
}

/// How a random graph may use guards.
#[derive(Clone, Copy)]
pub enum Guards {
    /// Guards 0 and 1, shared by any number of events.
    Shared,
    /// At most one guarded event in the graph, so no two events' guards are correlated.
    One,
}

fn region(rng: &mut Rng, graph_resources: &[Resource]) -> Region {
    let resource = rng.below(graph_resources.len());
    let r = &graph_resources[resource];
    let all = ((1u16 << r.channels) - 1) as u8;
    if rng.chance(70) {
        return Region {
            resource: ResourceId(resource as u32),
            levels: Levels {
                first: 0,
                count: r.levels,
            },
            channels: all,
        };
    }
    let first = rng.below(usize::from(r.levels)) as u8;
    let count = 1 + rng.below(usize::from(r.levels - first)) as u8;
    let channels = loop {
        let c = (rng.next() as u8) & all;
        if c != 0 {
            break c;
        }
    };
    Region {
        resource: ResourceId(resource as u32),
        levels: Levels { first, count },
        channels,
    }
}

fn access(rng: &mut Rng, resources: &[Resource]) -> Access {
    if rng.chance(10) {
        let count = 1 + rng.below(2);
        Access::Unresolved(
            (0..count)
                .map(|_| ResourceId(rng.below(resources.len()) as u32))
                .collect(),
        )
    } else {
        Access::Resolved(region(rng, resources))
    }
}

fn event(rng: &mut Rng, resources: &[Resource], guards: Guards, guarded: &mut bool) -> Event {
    let guard = match guards {
        Guards::Shared if rng.chance(25) => Guard::When(GuardId(rng.below(2) as u32)),
        Guards::One if !*guarded && rng.chance(25) => {
            *guarded = true;
            Guard::When(GuardId(0))
        }
        _ => Guard::Always,
    };
    let roll = rng.below(100);
    let mut event = if roll < 45 {
        let reads = (0..rng.below(3)).map(|_| access(rng, resources)).collect();
        let writes = (0..1 + rng.below(2))
            .map(|_| {
                let access = access(rng, resources);
                let cover = if rng.chance(15) {
                    Cover::Full
                } else {
                    Cover::Partial
                };
                let attachment = matches!(access, Access::Resolved(_))
                    && cover == Cover::Partial
                    && rng.chance(70);
                let clear =
                    matches!(access, Access::Resolved(_)) && cover == Cover::Full && rng.chance(50);
                Write {
                    access,
                    cover,
                    attachment,
                    clear,
                }
            })
            .collect();
        let effects = rng.chance(10);
        Event {
            kind: EventKind::Work,
            guard,
            reads,
            writes,
            effects,
            removable: !effects && rng.chance(70),
        }
    } else if roll < 70 {
        Event {
            kind: EventKind::Clear,
            guard,
            reads: Vec::new(),
            writes: (0..1 + rng.below(2))
                .map(|_| Write {
                    access: Access::Resolved(region(rng, resources)),
                    cover: Cover::Full,
                    attachment: false,
                    clear: false,
                })
                .collect(),
            effects: false,
            removable: false,
        }
    } else if roll < 80 {
        Event {
            kind: EventKind::Copy,
            guard,
            reads: vec![Access::Resolved(region(rng, resources))],
            writes: vec![Write {
                access: Access::Resolved(region(rng, resources)),
                cover: Cover::Full,
                attachment: false,
                clear: false,
            }],
            effects: false,
            removable: rng.chance(30),
        }
    } else {
        Event {
            kind: EventKind::Opaque,
            guard,
            reads: (0..1 + rng.below(2))
                .map(|_| access(rng, resources))
                .collect(),
            // An opaque event's cover doesn't matter: it reads and writes its whole access set.
            writes: (0..rng.below(2))
                .map(|_| Write {
                    access: access(rng, resources),
                    cover: if rng.chance(30) {
                        Cover::Full
                    } else {
                        Cover::Partial
                    },
                    attachment: false,
                    clear: false,
                })
                .collect(),
            effects: rng.chance(50),
            removable: false,
        }
    };
    if event.writes.is_empty() && event.kind == EventKind::Clear {
        event.kind = EventKind::Work;
    }
    event
}

/// A random valid graph: up to four resources (some mipmapped, pinned or aliased), up to three states of up to
/// `events` events, up to two edges out of each state, and up to two observers.
pub fn random_graph(rng: &mut Rng, guards: Guards, events: usize, max_channels: u8) -> Graph {
    let mut resources: Vec<Resource> = Vec::new();
    for r in 0..1 + rng.below(4) {
        let alias = if r > 0 && rng.chance(30) {
            let j = rng.below(r);
            resources[j].alias.is_none().then_some(ResourceId(j as u32))
        } else {
            None
        };
        resources.push(Resource {
            levels: if rng.chance(15) { 2 } else { 1 },
            channels: 1 + rng.below(usize::from(max_channels)) as u8,
            bytes: 1 + rng.below(1000) as u64,
            pinned: rng.chance(10),
            alias,
        });
    }
    let mut guarded = false;
    let count = 1 + rng.below(3);
    let states: Vec<State> = (0..count)
        .map(|_| State {
            events: (0..rng.below(events + 1))
                .map(|_| event(rng, &resources, guards, &mut guarded))
                .collect(),
        })
        .collect();
    let mut edges = Vec::new();
    for from in 0..count {
        for _ in 0..rng.below(3) {
            let transition = match rng.below(4) {
                0 => Transition::Reset,
                1 => Transition::Resize([854, 480]),
                _ => Transition::Steady,
            };
            edges.push(Edge {
                from: StateId(from as u32),
                transition,
                to: StateId(rng.below(count) as u32),
            });
        }
    }
    let observers = (0..rng.below(3))
        .map(|_| {
            let state = rng.below(count);
            Observer {
                state: StateId(state as u32),
                at: rng.below(states[state].events.len() + 1) as u32,
                region: region(rng, &resources),
            }
        })
        .collect();
    let graph = Graph {
        resources,
        states,
        edges,
        observers,
    };
    graph.validate().expect("the generator makes valid graphs");
    graph
}

/// A random run of up to eight frames.
fn random_run(rng: &mut Rng, graph: &Graph) -> Vec<Frame> {
    let masks = subsets(guard_mask(graph));
    let mut state = rng.below(graph.states.len());
    let mut frames = Vec::new();
    for _ in 0..1 + rng.below(8) {
        frames.push(Frame {
            state,
            guards: masks[rng.below(masks.len())],
        });
        let next: Vec<&Edge> = graph
            .edges
            .iter()
            .filter(|e| e.from.0 as usize == state)
            .collect();
        if next.is_empty() {
            break;
        }
        state = next[rng.below(next.len())].to.0 as usize;
    }
    frames
}

/// Seeds the property test runs; each takes well under a millisecond.
const SEEDS: u64 = 3000;
const RUNS_PER_GRAPH: usize = 12;

#[test]
fn plans_keep_every_observation_on_seeded_random_runs() {
    let (mut removed, mut folded, mut dead) = (0, 0, 0);
    for seed in 0..SEEDS {
        let mut rng = Rng::new(seed);
        let graph = random_graph(&mut rng, Guards::Shared, 6, 4);
        let planned = plan(&graph).expect("valid");
        removed += planned.removed.len();
        folded += planned.folded.len();
        dead += planned.dead.len();
        for _ in 0..RUNS_PER_GRAPH {
            let frames = random_run(&mut rng, &graph);
            let reference = retained(&planned, &frames, run(&graph, None, &frames, None));
            let observed = run(&graph, Some(&planned), &frames, None);
            assert_eq!(
                reference, observed,
                "seed {seed}: the plan {planned:?} changes what run {frames:?} sees in {graph:?}"
            );
        }
    }
    // The generator must reach every rule often, or the test proves nothing about it.
    for (rule, count) in [("R1", removed), ("R4b", folded), ("R2", dead)] {
        assert!(count >= 25, "{rule} fired {count} times over {SEEDS} seeds");
    }
}
