//! The planner's tests: hand-written cases per rule and per conservative case (each with a literal plan, and each
//! run through the interpreters on every short run), input refusals, the exhaustive liveness check, the seeded
//! property test, and the trace checks.

mod cases;
mod interp;
mod liveness_exhaustive;
mod property;
mod trace_cases;
mod validation;

use crate::model::{
    Access, Cover, Edge, Event, EventId, EventKind, Graph, Guard, GuardId, Levels, Observer,
    Region, Resource, ResourceId, State, StateId, Transition, Write,
};

/// A one-level resource with `channels` channels and `bytes` bytes.
fn res(channels: u8, bytes: u64) -> Resource {
    Resource {
        levels: 1,
        channels,
        bytes,
        pinned: false,
        alias: None,
    }
}

/// Every channel of level 0 of a four-channel resource.
fn rgba(resource: u32) -> Region {
    Region {
        resource: ResourceId(resource),
        levels: Levels::BASE,
        channels: 0b1111,
    }
}

fn whole(resource: u32) -> Access {
    Access::Resolved(rgba(resource))
}

fn id(state: u32, index: u32) -> EventId {
    EventId {
        state: StateId(state),
        index,
    }
}

/// A removable draw: reads `reads`, and draws onto `targets` as partial attachments.
fn draw(reads: &[u32], targets: &[u32]) -> Event {
    Event {
        kind: EventKind::Work,
        guard: Guard::Always,
        reads: reads.iter().map(|&r| whole(r)).collect(),
        writes: targets
            .iter()
            .map(|&r| Write {
                access: whole(r),
                cover: Cover::Partial,
                attachment: true,
            })
            .collect(),
        effects: false,
        removable: true,
    }
}

/// A draw the adapter keeps whatever (the output route, an image binding, effects).
fn kept(reads: &[u32], targets: &[u32]) -> Event {
    Event {
        removable: false,
        ..draw(reads, targets)
    }
}

/// An unguarded attachment clear of each resource.
fn clear(targets: &[u32]) -> Event {
    Event {
        kind: EventKind::Clear,
        guard: Guard::Always,
        reads: Vec::new(),
        writes: targets
            .iter()
            .map(|&r| Write {
                access: whole(r),
                cover: Cover::Full,
                attachment: false,
            })
            .collect(),
        effects: false,
        removable: false,
    }
}

/// An opaque block over `set`, with effects or not.
fn opaque(set: &[u32], effects: bool) -> Event {
    Event {
        kind: EventKind::Opaque,
        guard: Guard::Always,
        reads: set.iter().map(|&r| whole(r)).collect(),
        writes: set
            .iter()
            .map(|&r| Write {
                access: whole(r),
                cover: Cover::Partial,
                attachment: false,
            })
            .collect(),
        effects,
        removable: false,
    }
}

fn guarded(event: Event, guard: u32) -> Event {
    Event {
        guard: Guard::When(GuardId(guard)),
        ..event
    }
}

/// A one-state graph whose frames follow each other, with an observer of each `seen` resource at the frame's end.
fn looped(resources: Vec<Resource>, events: Vec<Event>, seen: &[u32]) -> Graph {
    let at = events.len() as u32;
    Graph {
        resources,
        states: vec![State { events }],
        edges: vec![Edge {
            from: StateId(0),
            transition: Transition::Steady,
            to: StateId(0),
        }],
        observers: seen
            .iter()
            .map(|&r| Observer {
                state: StateId(0),
                at,
                region: rgba(r),
            })
            .collect(),
    }
}
