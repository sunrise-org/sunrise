//! The planner's input: resources, the states a frame can start in, each state's ordered events, the edges between
//! states, and the observers that read a value from outside the events.

/// A physical texture or buffer, by its index in [`Graph::resources`]. Each side of a double-buffered target is a
/// resource of its own.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ResourceId(pub u32);

/// A state a frame can start in, by its index in [`Graph::states`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct StateId(pub u32);

/// A runtime value that decides whether a guarded event runs. It may differ from frame to frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct GuardId(pub u32);

/// An event: the `index`-th event of `state`'s list.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct EventId {
    pub state: StateId,
    pub index: u32,
}

/// The most channels a resource can have: a channel set is one bit each in a `u8`.
pub const MAX_CHANNELS: u8 = u8::BITS as u8;

/// A physical texture or buffer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Resource {
    /// Mip levels, at least 1. A resource with more than one is never planned on (its mip state isn't carried).
    pub levels: u8,
    /// Channels, 1 to [`MAX_CHANNELS`].
    pub channels: u8,
    /// Bytes allocated for it, counted in [`crate::Savings::bytes`] when it is dead.
    pub bytes: u64,
    /// Never planned on: the adapter's own exclusions (a buffer the screen reads, a namespace whose state isn't
    /// carried).
    pub pinned: bool,
    /// The resource whose storage this one's bindings use once this one is dead (the B side of a double-buffered
    /// target names its A side). A resource that others alias is dead only when they all are.
    pub alias: Option<ResourceId>,
}

/// Contiguous mip levels, `first..first + count`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Levels {
    pub first: u8,
    pub count: u8,
}

impl Levels {
    /// Level 0 alone.
    pub const BASE: Levels = Levels { first: 0, count: 1 };
}

/// A resolved part of one resource: some levels, some channels (bit `c` is channel `c`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Region {
    pub resource: ResourceId,
    pub levels: Levels,
    pub channels: u8,
}

/// What an event reads or writes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Access {
    Resolved(Region),
    /// The runtime can't say which of these it reaches: it reaches every level and channel of each. Every rule
    /// excludes every resource an unresolved access names, anywhere in the graph.
    Unresolved(Vec<ResourceId>),
}

/// How much of its region a write replaces.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cover {
    /// Every value of the region. An unguarded, resolved `Full` write of a non-`Opaque` event is a kill.
    Full,
    /// Some values, which ones unknown: the rest pass through, so the write reads the old value.
    Partial,
}

/// One write of an event.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Write {
    pub access: Access,
    pub cover: Cover,
    /// A render-pass attachment that loads what the resource holds, whose load operation the plan may turn into a
    /// clear: only such a write can take a folded clear (R4b). Only on `Work` events, with a resolved access and
    /// `Partial` cover; an attachment that already loads as a clear is a `Full` write without this flag.
    pub attachment: bool,
    /// An attachment a draw only clears as it begins and draws nothing into (a world blit's extra targets): a
    /// `Full`, resolved write of a `Work` event that R2 counts as a clear, not a use, and R4b never folds.
    pub clear: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum EventKind {
    /// A draw or a dispatch.
    Work,
    /// An attachment clear. It reads nothing, has no effects, and its writes are resolved and `Full`: any other
    /// work done with it is an event of its own.
    Clear,
    /// A copy: reads its source, writes its destination's levels.
    Copy,
    /// A block of work whose internal order isn't modelled: it reads and writes its whole access set (reads and
    /// writes together), with `Partial` cover.
    Opaque,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Guard {
    Always,
    /// Runs only in frames where the guard holds. The plan holds for every value, in every frame.
    When(GuardId),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Event {
    pub kind: EventKind,
    pub guard: Guard,
    pub reads: Vec<Access>,
    pub writes: Vec<Write>,
    /// Touches something outside `reads` and `writes` (an image, a storage buffer, the CPU). Never removed.
    pub effects: bool,
    /// The adapter allows R1 to remove it (a draw with no image binding, no scratch copy, no mip request, not the
    /// output route). Never set on an `Opaque` event or one with effects.
    pub removable: bool,
}

/// A state a frame can start in, with the events a frame in it runs, in order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct State {
    pub events: Vec<Event>,
}

/// What happens between two frames. Every kind carries the resources' contents to the next frame: a reset or a
/// resize that remakes a resource only makes the plan more conservative than it needs to be.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Transition {
    Steady,
    Reset,
    Resize([u32; 2]),
}

/// The next frame after a frame in `from` may start in `to`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Edge {
    pub from: StateId,
    pub transition: Transition,
    pub to: StateId,
}

/// A read from outside the events (a mod slot's readback, the screen, a copy's source at the copy): in a frame in
/// `state`, after its first `at` events ran (0 is the frame's start, the event count its end).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Observer {
    pub state: StateId,
    pub at: u32,
    pub region: Region,
}

/// The whole input, for one render size. Every state is a possible first frame, so the plan holds for any
/// reachable part of the graph.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Graph {
    pub resources: Vec<Resource>,
    pub states: Vec<State>,
    pub edges: Vec<Edge>,
    pub observers: Vec<Observer>,
}

/// Where in the graph an input error is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Site {
    Resource(ResourceId),
    Event(EventId),
    Edge(usize),
    Observer(usize),
}

/// An input the planner refuses, and where.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InputError {
    /// A resource with no levels.
    NoLevels(ResourceId),
    /// A resource with no channels, or more than [`MAX_CHANNELS`].
    Channels(ResourceId),
    /// An alias naming the resource itself, or a resource that aliases another one.
    Alias(ResourceId),
    /// A resource id past the resource list.
    UnknownResource(Site),
    /// A state id past the state list.
    UnknownState(Site),
    /// An empty level range, or one past the resource's levels.
    LevelsOutOfRange(Site),
    /// An empty channel set, or one naming a channel the resource doesn't have.
    ChannelsOutOfRange(Site),
    /// An unresolved access that names no resource.
    EmptyUnresolved(Site),
    /// `removable` on an `Opaque` event or one with effects.
    Removable(EventId),
    /// A `Clear` event that reads, has effects, or writes an unresolved access or with `Partial` cover.
    Clear(EventId),
    /// `attachment` on a write of an event that isn't `Work`, or on an unresolved or `Full` write.
    Attachment(EventId),
    /// `clear` on a write of an event that isn't `Work`, or on an unresolved, `Partial` or attachment write.
    ClearWrite(EventId),
    /// An observer past the end of its state's events.
    ObserverPosition(usize),
}

impl std::fmt::Display for InputError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "frame plan input refused: {self:?}")
    }
}

impl std::error::Error for InputError {}

impl Graph {
    pub fn resource(&self, id: ResourceId) -> &Resource {
        &self.resources[id.0 as usize]
    }

    pub fn event(&self, id: EventId) -> &Event {
        &self.states[id.state.0 as usize].events[id.index as usize]
    }

    /// Checks every id, range and flag; the planner refuses an input that fails.
    pub fn validate(&self) -> Result<(), InputError> {
        for (index, resource) in self.resources.iter().enumerate() {
            let id = ResourceId(index as u32);
            if resource.levels == 0 {
                return Err(InputError::NoLevels(id));
            }
            if resource.channels == 0 || resource.channels > MAX_CHANNELS {
                return Err(InputError::Channels(id));
            }
            if let Some(target) = resource.alias {
                let target = self
                    .resources
                    .get(target.0 as usize)
                    .ok_or(InputError::UnknownResource(Site::Resource(id)))?;
                if resource.alias == Some(id) || target.alias.is_some() {
                    return Err(InputError::Alias(id));
                }
            }
        }
        for (s, state) in self.states.iter().enumerate() {
            for (index, event) in state.events.iter().enumerate() {
                let id = EventId {
                    state: StateId(s as u32),
                    index: index as u32,
                };
                self.validate_event(id, event)?;
            }
        }
        for (index, edge) in self.edges.iter().enumerate() {
            for state in [edge.from, edge.to] {
                if state.0 as usize >= self.states.len() {
                    return Err(InputError::UnknownState(Site::Edge(index)));
                }
            }
        }
        for (index, observer) in self.observers.iter().enumerate() {
            let state = self
                .states
                .get(observer.state.0 as usize)
                .ok_or(InputError::UnknownState(Site::Observer(index)))?;
            if observer.at as usize > state.events.len() {
                return Err(InputError::ObserverPosition(index));
            }
            self.validate_region(Site::Observer(index), &observer.region)?;
        }
        Ok(())
    }

    fn validate_event(&self, id: EventId, event: &Event) -> Result<(), InputError> {
        let site = Site::Event(id);
        for access in event
            .reads
            .iter()
            .chain(event.writes.iter().map(|w| &w.access))
        {
            self.validate_access(site, access)?;
        }
        if event.removable && (event.kind == EventKind::Opaque || event.effects) {
            return Err(InputError::Removable(id));
        }
        if event.kind == EventKind::Clear
            && (!event.reads.is_empty()
                || event.effects
                || event
                    .writes
                    .iter()
                    .any(|w| w.cover != Cover::Full || matches!(w.access, Access::Unresolved(_))))
        {
            return Err(InputError::Clear(id));
        }
        for write in &event.writes {
            if write.clear
                && (event.kind != EventKind::Work
                    || write.cover != Cover::Full
                    || write.attachment
                    || matches!(write.access, Access::Unresolved(_)))
            {
                return Err(InputError::ClearWrite(id));
            }
            if write.attachment
                && (event.kind != EventKind::Work
                    || write.cover == Cover::Full
                    || matches!(write.access, Access::Unresolved(_)))
            {
                return Err(InputError::Attachment(id));
            }
        }
        Ok(())
    }

    fn validate_access(&self, site: Site, access: &Access) -> Result<(), InputError> {
        match access {
            Access::Resolved(region) => self.validate_region(site, region),
            Access::Unresolved(set) => {
                if set.is_empty() {
                    return Err(InputError::EmptyUnresolved(site));
                }
                for id in set {
                    if id.0 as usize >= self.resources.len() {
                        return Err(InputError::UnknownResource(site));
                    }
                }
                Ok(())
            }
        }
    }

    fn validate_region(&self, site: Site, region: &Region) -> Result<(), InputError> {
        let resource = self
            .resources
            .get(region.resource.0 as usize)
            .ok_or(InputError::UnknownResource(site))?;
        let Levels { first, count } = region.levels;
        if count == 0 || u16::from(first) + u16::from(count) > u16::from(resource.levels) {
            return Err(InputError::LevelsOutOfRange(site));
        }
        if region.channels == 0 || u16::from(region.channels) >= 1u16 << resource.channels {
            return Err(InputError::ChannelsOutOfRange(site));
        }
        Ok(())
    }
}
