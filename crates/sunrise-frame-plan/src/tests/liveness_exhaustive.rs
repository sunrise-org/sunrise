//! The fixed point checked against exhaustive runs of small graphs. A written value is live after its event when
//! flipping it changes something seen on some run: every run of up to one frame more than the graph has states,
//! from that event on, with every guard value in every frame. With no two events' guards correlated (one guarded
//! event per graph), the fixed point must say exactly that; and every plan must keep every observation on every one
//! of those runs.

use super::interp::{Perturb, assert_sound, cell_index, every_run, run};
use super::property::{Guards, Rng, random_graph};
use super::*;
use crate::live::liveness;
use crate::model::Guard;
use crate::plan::plan;

const SEEDS: u64 = 250;

/// Whether flipping `cell` right after event `index` of a first frame in `state` changes anything seen.
fn read_later(graph: &Graph, state: usize, index: usize, cell: usize, depth: usize) -> bool {
    let event = &graph.states[state].events[index];
    let mut read = false;
    every_run(graph, state, depth, &mut |frames| {
        let runs = match event.guard {
            Guard::Always => true,
            Guard::When(g) => frames[0].guards & (1 << g.0) != 0,
        };
        if read || !runs {
            return;
        }
        let perturb = Perturb { event: index, cell };
        read = run(graph, None, frames, None) != run(graph, None, frames, Some(perturb));
    });
    read
}

#[test]
fn the_fixed_point_matches_exhaustive_runs() {
    let mut checked = (0, 0);
    for seed in 0..SEEDS {
        let mut rng = Rng::new(seed);
        let graph = random_graph(&mut rng, Guards::One, 4, 2);
        let live = liveness(&graph).expect("valid");
        let depth = graph.states.len() + 1;
        for (s, state) in graph.states.iter().enumerate() {
            for (i, event) in state.events.iter().enumerate() {
                let mut written = Vec::new();
                for write in &event.writes {
                    match &write.access {
                        Access::Resolved(region) => written.push(*region),
                        Access::Unresolved(set) => written.extend(set.iter().map(|&r| Region {
                            resource: r,
                            levels: Levels {
                                first: 0,
                                count: graph.resource(r).levels,
                            },
                            channels: ((1u16 << graph.resource(r).channels) - 1) as u8,
                        })),
                    }
                }
                for region in written {
                    let levels = region.levels.first..region.levels.first + region.levels.count;
                    for level in levels {
                        for channel in 0..graph.resource(region.resource).channels {
                            if region.channels & (1 << channel) == 0 {
                                continue;
                            }
                            let one = Region {
                                resource: region.resource,
                                levels: Levels {
                                    first: level,
                                    count: 1,
                                },
                                channels: 1 << channel,
                            };
                            let planner = live.live_after(id(s as u32, i as u32), &one);
                            let cell = cell_index(&graph, region.resource, level, channel);
                            let truth = read_later(&graph, s, i, cell, depth);
                            assert_eq!(
                                planner, truth,
                                "seed {seed}: liveness of {one:?} after event {i} of state {s} in {graph:?}"
                            );
                            if truth {
                                checked.0 += 1;
                            } else {
                                checked.1 += 1;
                            }
                        }
                    }
                }
            }
        }
        let planned = plan(&graph).expect("valid");
        assert_sound(&graph, &planned, depth);
    }
    // Both answers must come up often, or the check proves little.
    assert!(
        checked.0 >= 200 && checked.1 >= 200,
        "live {}, dead {}",
        checked.0,
        checked.1
    );
}
