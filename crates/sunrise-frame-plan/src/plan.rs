//! The rules of step 0: R1 (dead draw), R4b (folded clear) and R2's first version (dead side).

use crate::live::{Liveness, liveness};
use crate::model::{
    Access, Event, EventId, EventKind, Graph, Guard, InputError, Region, ResourceId, StateId,
};

/// Why an event was removed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rule {
    /// R1: a draw nothing reads before a kill, on any path.
    DeadDraw,
}

/// R4b: the clear of `resource` in event `clear` becomes the clear load of event `into`'s attachment to it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Fold {
    pub clear: EventId,
    pub resource: ResourceId,
    pub into: EventId,
}

/// What the plan saves, at the graph's render size.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Savings {
    /// Events removed.
    pub events: u32,
    /// Attachment clears folded into a later draw's load.
    pub clears_folded: u32,
    /// Attachment clears of dead resources, dropped.
    pub clears_dropped: u32,
    /// Bytes of the dead resources.
    pub bytes: u64,
}

/// Which events can go, and why. Ids are in the input's order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Plan {
    pub removed: Vec<(EventId, Rule)>,
    pub folded: Vec<Fold>,
    /// R2: resources whose only remaining uses are clears. Each one's clears go, and it needs no storage of its
    /// own: its bindings use its alias, or nothing.
    pub dead: Vec<ResourceId>,
    pub savings: Savings,
}

impl Plan {
    pub fn is_removed(&self, event: EventId) -> bool {
        self.removed.iter().any(|&(id, _)| id == event)
    }
}

/// Plans `graph`, or refuses an invalid input.
pub fn plan(graph: &Graph) -> Result<Plan, InputError> {
    let live = liveness(graph)?;
    let excluded = excluded(graph);
    let mut plan = Plan::default();
    for_each_event(graph, |id, event| {
        if dead_draw(&live, &excluded, id, event) {
            plan.removed.push((id, Rule::DeadDraw));
        }
    });
    for (s, state) in graph.states.iter().enumerate() {
        for (c, event) in state.events.iter().enumerate() {
            let clear = EventId {
                state: StateId(s as u32),
                index: c as u32,
            };
            for write in &event.writes {
                if let Some(into) = fold_target(graph, &plan, &excluded, clear, event, write)
                    && let Access::Resolved(region) = write.access
                {
                    let fold = Fold {
                        clear,
                        resource: region.resource,
                        into,
                    };
                    // A clear that names the same resource twice folds once.
                    if !plan.folded.contains(&fold) {
                        plan.folded.push(fold);
                    }
                }
            }
        }
    }
    plan.dead = dead_resources(graph, &plan, &excluded);
    plan.savings = savings(graph, &plan);
    Ok(plan)
}

fn for_each_event(graph: &Graph, mut f: impl FnMut(EventId, &Event)) {
    for (s, state) in graph.states.iter().enumerate() {
        for (i, event) in state.events.iter().enumerate() {
            let id = EventId {
                state: StateId(s as u32),
                index: i as u32,
            };
            f(id, event);
        }
    }
}

/// The resources every rule leaves alone: pinned ones, mipmapped ones (mip state isn't carried), and every one an
/// unresolved access may reach.
fn excluded(graph: &Graph) -> Vec<bool> {
    let mut excluded: Vec<bool> = graph
        .resources
        .iter()
        .map(|r| r.pinned || r.levels > 1)
        .collect();
    for state in &graph.states {
        for event in &state.events {
            let accesses = event
                .reads
                .iter()
                .chain(event.writes.iter().map(|w| &w.access));
            for access in accesses {
                if let Access::Unresolved(set) = access {
                    for id in set {
                        excluded[id.0 as usize] = true;
                    }
                }
            }
        }
    }
    excluded
}

fn accesses(event: &Event) -> impl Iterator<Item = &Access> {
    event
        .reads
        .iter()
        .chain(event.writes.iter().map(|w| &w.access))
}

fn reaches(access: &Access, resource: ResourceId) -> bool {
    match access {
        Access::Resolved(region) => region.resource == resource,
        Access::Unresolved(set) => set.contains(&resource),
    }
}

fn touches(event: &Event, resource: ResourceId) -> bool {
    accesses(event).any(|a| reaches(a, resource))
}

/// Every rule excludes an event that touches a mipmapped resource.
fn touches_mipmapped(graph: &Graph, event: &Event) -> bool {
    accesses(event).any(|access| match access {
        Access::Resolved(region) => graph.resource(region.resource).levels > 1,
        Access::Unresolved(set) => set.iter().any(|&id| graph.resource(id).levels > 1),
    })
}

/// R1: a removable draw each of whose accesses, reads included, is resolved onto a resource no rule excludes (so
/// a draw that reads a pinned, mipmapped or unresolved side stays), and whose writes are dead right after it.
fn dead_draw(live: &Liveness, excluded: &[bool], id: EventId, event: &Event) -> bool {
    event.kind == EventKind::Work
        && event.removable
        && accesses(event)
            .all(|access| matches!(access, Access::Resolved(r) if !excluded[r.resource.0 as usize]))
        && event
            .writes
            .iter()
            .all(|write| matches!(write.access, Access::Resolved(r) if !live.live_after(id, &r)))
}

/// R4b: the draw a clear's write folds into, if any. The clear (whose writes are `Full`, as validated) is unguarded
/// and kills the whole of a one-level resource no rule excludes; the first later event of the same frame that
/// touches the resource is an unguarded, retained draw whose only access to it is attachments of the whole of it; and
/// no observer sees the resource in between.
fn fold_target(
    graph: &Graph,
    plan: &Plan,
    excluded: &[bool],
    clear: EventId,
    event: &Event,
    write: &crate::model::Write,
) -> Option<EventId> {
    let region = match write.access {
        Access::Resolved(region) => region,
        Access::Unresolved(_) => return None,
    };
    let s = region.resource;
    if event.kind != EventKind::Clear
        || event.guard != Guard::Always
        || excluded[s.0 as usize]
        || !whole(graph, &region)
        || touches_mipmapped(graph, event)
    {
        return None;
    }
    let events = &graph.states[clear.state.0 as usize].events;
    for j in clear.index as usize + 1..events.len() {
        let observed = graph
            .observers
            .iter()
            .any(|o| o.state == clear.state && o.at as usize == j && o.region.resource == s);
        if observed {
            return None;
        }
        let candidate = &events[j];
        if !touches(candidate, s) {
            continue;
        }
        let into = EventId {
            state: clear.state,
            index: j as u32,
        };
        let only_attachments = candidate
            .writes
            .iter()
            .filter(|w| reaches(&w.access, s))
            .all(|w| w.attachment && matches!(w.access, Access::Resolved(r) if whole(graph, &r)));
        // Only a `Work` event has attachments (validated), so `only_attachments` makes the candidate a draw.
        let folds = candidate.guard == Guard::Always
            && !plan.is_removed(into)
            && !touches_mipmapped(graph, candidate)
            && !candidate.reads.iter().any(|r| reaches(r, s))
            && only_attachments;
        return folds.then_some(into);
    }
    None
}

/// Whether a region is every level and channel of its resource.
fn whole(graph: &Graph, region: &Region) -> bool {
    let resource = graph.resource(region.resource);
    region.levels.first == 0
        && region.levels.count == resource.levels
        && u16::from(region.channels) == (1u16 << resource.channels) - 1
}

/// R2, first version: a resource no rule excludes and no observer reads, which every retained event touching it
/// only clears, with a clear that touches no mipmapped resource. A resource another one aliases is dead only when
/// that one is too.
fn dead_resources(graph: &Graph, plan: &Plan, excluded: &[bool]) -> Vec<ResourceId> {
    let mut candidate: Vec<bool> = excluded.iter().map(|&e| !e).collect();
    for observer in &graph.observers {
        candidate[observer.region.resource.0 as usize] = false;
    }
    for_each_event(graph, |id, event| {
        // A clear that touches a mipmapped resource is left out of every rule, so each of its attachments is a use.
        let dropped_clear = event.kind == EventKind::Clear && !touches_mipmapped(graph, event);
        if dropped_clear || plan.is_removed(id) {
            return;
        }
        for access in accesses(event) {
            match access {
                Access::Resolved(region) => candidate[region.resource.0 as usize] = false,
                Access::Unresolved(set) => {
                    for r in set {
                        candidate[r.0 as usize] = false;
                    }
                }
            }
        }
    });
    let mut dead = candidate.clone();
    for (index, resource) in graph.resources.iter().enumerate() {
        if let Some(target) = resource.alias
            && !candidate[index]
        {
            dead[target.0 as usize] = false;
        }
    }
    dead.iter()
        .enumerate()
        .filter(|&(_, &d)| d)
        .map(|(i, _)| ResourceId(i as u32))
        .collect()
}

fn savings(graph: &Graph, plan: &Plan) -> Savings {
    let mut clears_dropped = 0;
    for_each_event(graph, |_, event| {
        if event.kind == EventKind::Clear {
            clears_dropped += event
                .writes
                .iter()
                .filter(|w| plan.dead.iter().any(|&d| reaches(&w.access, d)))
                .count() as u32;
        }
    });
    Savings {
        events: plan.removed.len() as u32,
        clears_folded: plan.folded.len() as u32,
        clears_dropped,
        bytes: plan.dead.iter().map(|&d| graph.resource(d).bytes).sum(),
    }
}
