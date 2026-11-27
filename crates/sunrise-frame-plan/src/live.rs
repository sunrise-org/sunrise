//! Liveness: the backward fixed point over the state graph. A value is live at a point when some path, through any
//! guard values, edges and frames, reaches a read of it before a kill.

use crate::model::{
    Access, Cover, Event, EventId, EventKind, Graph, Guard, InputError, Region, ResourceId, StateId,
};

/// A set of values: one channel mask per (resource, level).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Cells(Vec<u8>);

/// Where each resource's levels start in a [`Cells`].
pub(crate) struct Layout {
    offsets: Vec<usize>,
    all_channels: Vec<u8>,
    levels: Vec<u8>,
    total: usize,
}

impl Layout {
    pub(crate) fn new(graph: &Graph) -> Layout {
        let mut offsets = Vec::with_capacity(graph.resources.len());
        let mut total = 0;
        for resource in &graph.resources {
            offsets.push(total);
            total += usize::from(resource.levels);
        }
        Layout {
            offsets,
            all_channels: graph
                .resources
                .iter()
                .map(|r| (((1u16 << r.channels) - 1) & 0xff) as u8)
                .collect(),
            levels: graph.resources.iter().map(|r| r.levels).collect(),
            total,
        }
    }

    pub(crate) fn empty(&self) -> Cells {
        Cells(vec![0; self.total])
    }

    fn span(&self, region: &Region) -> std::ops::Range<usize> {
        let start = self.offsets[region.resource.0 as usize] + usize::from(region.levels.first);
        start..start + usize::from(region.levels.count)
    }

    fn whole(&self, id: ResourceId) -> (std::ops::Range<usize>, u8) {
        let start = self.offsets[id.0 as usize];
        (
            start..start + usize::from(self.levels[id.0 as usize]),
            self.all_channels[id.0 as usize],
        )
    }

    fn add_region(&self, cells: &mut Cells, region: &Region) {
        for cell in &mut cells.0[self.span(region)] {
            *cell |= region.channels;
        }
    }

    fn add_resource(&self, cells: &mut Cells, id: ResourceId) {
        let (span, channels) = self.whole(id);
        for cell in &mut cells.0[span] {
            *cell |= channels;
        }
    }

    fn add_access(&self, cells: &mut Cells, access: &Access) {
        match access {
            Access::Resolved(region) => self.add_region(cells, region),
            Access::Unresolved(set) => {
                for &id in set {
                    self.add_resource(cells, id);
                }
            }
        }
    }

    fn remove_region(&self, cells: &mut Cells, region: &Region) {
        for cell in &mut cells.0[self.span(region)] {
            *cell &= !region.channels;
        }
    }

    pub(crate) fn intersects_region(&self, cells: &Cells, region: &Region) -> bool {
        cells.0[self.span(region)]
            .iter()
            .any(|&cell| cell & region.channels != 0)
    }

    /// The live set before `event`, from the live set after it: its kills leave, its reads join.
    fn transfer(&self, live: &mut Cells, event: &Event) {
        // An opaque event's writes kill nothing: it reads its whole access set, below.
        if event.guard == Guard::Always {
            for write in &event.writes {
                if let (Cover::Full, Access::Resolved(region)) = (write.cover, &write.access) {
                    self.remove_region(live, region);
                }
            }
        }
        for read in &event.reads {
            self.add_access(live, read);
        }
        for write in &event.writes {
            // A partial write passes the values it doesn't replace through, an unresolved one may miss any value it
            // names, and an opaque event reads its whole access set.
            if write.cover == Cover::Partial
                || matches!(write.access, Access::Unresolved(_))
                || event.kind == EventKind::Opaque
            {
                self.add_access(live, &write.access);
            }
        }
    }
}

impl Cells {
    /// Adds `other`; true when that changed the set.
    fn union(&mut self, other: &Cells) -> bool {
        let mut changed = false;
        for (cell, &add) in self.0.iter_mut().zip(&other.0) {
            let joined = *cell | add;
            changed |= joined != *cell;
            *cell = joined;
        }
        changed
    }
}

/// The fixed point's result: what is live after each event and at each state's start.
pub struct Liveness {
    layout: Layout,
    /// Per state, per event: the live set right after it, its observers included.
    after: Vec<Vec<Cells>>,
    start: Vec<Cells>,
}

impl Liveness {
    /// Whether any value of `region` is live right after `event`.
    pub fn live_after(&self, event: EventId, region: &Region) -> bool {
        self.layout.intersects_region(
            &self.after[event.state.0 as usize][event.index as usize],
            region,
        )
    }

    /// Whether any value of `region` is live when a frame in `state` starts.
    pub fn live_at_start(&self, state: StateId, region: &Region) -> bool {
        self.layout
            .intersects_region(&self.start[state.0 as usize], region)
    }
}

/// Runs the backward fixed point over every state (each is a possible first frame) and every edge.
pub fn liveness(graph: &Graph) -> Result<Liveness, InputError> {
    graph.validate()?;
    let layout = Layout::new(graph);
    let count = graph.states.len();
    let mut successors = vec![Vec::new(); count];
    let mut predecessors = vec![Vec::new(); count];
    for edge in &graph.edges {
        successors[edge.from.0 as usize].push(edge.to.0 as usize);
        predecessors[edge.to.0 as usize].push(edge.from.0 as usize);
    }
    let mut observers = vec![Vec::new(); count];
    for observer in &graph.observers {
        observers[observer.state.0 as usize].push(*observer);
    }
    let mut start = vec![layout.empty(); count];
    // A worklist: each update only grows a state's start set, which is finite, so it stops.
    let mut queued = vec![true; count];
    let mut work: Vec<usize> = (0..count).rev().collect();
    while let Some(state) = work.pop() {
        queued[state] = false;
        let end = end_set(&layout, &successors[state], &start);
        let mut live = end;
        walk(
            &layout,
            graph,
            state,
            &observers[state],
            &mut live,
            |_, _| {},
        );
        if start[state].union(&live) {
            for &p in &predecessors[state] {
                if !queued[p] {
                    queued[p] = true;
                    work.push(p);
                }
            }
        }
    }
    let mut after = Vec::with_capacity(count);
    for state in 0..count {
        let mut live = end_set(&layout, &successors[state], &start);
        let mut per_event = vec![layout.empty(); graph.states[state].events.len()];
        walk(
            &layout,
            graph,
            state,
            &observers[state],
            &mut live,
            |i, set| per_event[i] = set.clone(),
        );
        after.push(per_event);
    }
    Ok(Liveness {
        layout,
        after,
        start,
    })
}

fn end_set(layout: &Layout, successors: &[usize], start: &[Cells]) -> Cells {
    let mut end = layout.empty();
    for &s in successors {
        end.union(&start[s]);
    }
    end
}

/// Walks `state`'s events backwards from the live set at its end, calling `seen(i, live)` with the set right after
/// event `i`; leaves the set at its start in `live`.
fn walk(
    layout: &Layout,
    graph: &Graph,
    state: usize,
    observers: &[crate::model::Observer],
    live: &mut Cells,
    mut seen: impl FnMut(usize, &Cells),
) {
    let events = &graph.states[state].events;
    let observe = |live: &mut Cells, at: usize| {
        for observer in observers.iter().filter(|o| o.at as usize == at) {
            layout.add_region(live, &observer.region);
        }
    };
    observe(live, events.len());
    for (i, event) in events.iter().enumerate().rev() {
        seen(i, live);
        layout.transfer(live, event);
        observe(live, i);
    }
}
