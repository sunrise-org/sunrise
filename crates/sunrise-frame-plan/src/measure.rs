//! What the planner would save on a run of recorded frames: a measure, not a proof.
//!
//! Each frame is projected ([`crate::trace::project`]) and its blocks checked against their declared access sets
//! ([`crate::trace::check_blocks`]). The frames of each render size are planned apart (the spec plans per size).
//! Frames whose projection and block notes are equal are one state. A size's graph has every state, an edge from
//! every state to every state (a superset of the transitions the run saw, so the plan is only more conservative),
//! each block as one `Opaque` event over its declared set, and the textures of its frames as resources by their
//! runtime id. The plan's edits are then mapped back to each state's units and textures and
//! checked with [`crate::trace::check_edits`] against that state's frame.

use crate::model::{
    Access, Cover, Edge, EventKind, Graph, InputError, Region, Resource, ResourceId, State,
    StateId, Transition, Write,
};
use crate::plan::{Plan, Savings, plan};
use crate::record::{BlockNote, Recorded};
use crate::trace::{
    Edits, Load, Projection, Step, Texture, TextureId, TraceError, Unit, Use, check_blocks,
    check_edits, project,
};

/// What a block's callbacks into other code reach.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Callbacks {
    /// As the runtime declared: every texture its `unresolved` list names, unresolved (the spec's model).
    Unresolved,
    /// As if a lowering could show the callbacks touch nothing of the frame: the declared spans alone. Not sound
    /// today; it shows what the rules would remove if the callbacks were narrowed.
    Narrowed,
}

/// A run the measure refuses.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MeasureError {
    /// No frames.
    Empty,
    /// The projection refuses this frame (by its `seq`).
    Project(u64, TraceError),
    /// Two frames give one texture id different sizes.
    Texture(TextureId),
    /// The graph built from the run is refused (a block declared a span outside its texture).
    Input(InputError),
}

impl std::fmt::Display for MeasureError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "frame measure refused: {self:?}")
    }
}

impl std::error::Error for MeasureError {}

/// One frame's checks.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FrameCheck {
    pub seq: u64,
    /// The state the frame is, by its index in the report's states.
    pub state: usize,
    /// [`check_blocks`] on the frame's declared sets.
    pub blocks: Result<(), TraceError>,
    /// What the runtime could not record.
    pub errors: Vec<String>,
    /// The guard values the frame ran with.
    pub guards: Vec<(String, bool)>,
}

/// A removed draw or a folded clear, in a state's own numbers and labels.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Removed {
    pub state: usize,
    pub work: u32,
    pub label: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Folded {
    pub state: usize,
    pub clear: u32,
    pub texture: TextureId,
    pub draw: u32,
    pub draw_label: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Dead {
    pub texture: TextureId,
    pub label: String,
    pub bytes: u64,
}

/// What a state's frame holds, for scale.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Work {
    /// Units of work outside blocks that attach a texture (draws and attachment clears).
    pub passes: u32,
    /// Attachment clears (one per cleared texture) of `Clear` units and draws' clear-only attachments.
    pub clears: u32,
    /// Blocks.
    pub blocks: u32,
}

/// The plan at one render size: the spec plans per size.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sized {
    pub size: [u32; 2],
    /// The frames drawn at this size, each with its checks; its `state` indexes `states`.
    pub frames: Vec<FrameCheck>,
    /// One per state: what its frame holds, and [`check_edits`] of the plan's edits on it.
    pub states: Vec<(Work, Result<(), TraceError>)>,
    /// The most bytes one frame of this size holds: every texture, and the textures a rule may plan on (not pinned,
    /// one level).
    pub allocated: u64,
    pub plannable: u64,
    pub removed: Vec<Removed>,
    pub folded: Vec<Folded>,
    pub dead: Vec<Dead>,
    /// The most bytes of dead textures one frame of this size holds: the storage the plan saves at this size.
    pub dead_bytes: u64,
    pub savings: Savings,
}

/// A run's measure: one plan per render size, in the order the sizes were first drawn.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Report {
    pub sizes: Vec<Sized>,
}

struct StateOf<'a> {
    projection: Projection,
    first: &'a Recorded,
}

/// Measures a run of frames (any order; `seq` only names them): the frames of each render size apart.
pub fn measure(frames: &[Recorded], callbacks: Callbacks) -> Result<Report, MeasureError> {
    if frames.is_empty() {
        return Err(MeasureError::Empty);
    }
    let mut sizes: Vec<[u32; 2]> = Vec::new();
    for f in frames {
        if !sizes.contains(&f.size) {
            sizes.push(f.size);
        }
    }
    let sizes = sizes
        .into_iter()
        .map(|size| {
            let of: Vec<&Recorded> = frames.iter().filter(|f| f.size == size).collect();
            measure_size(size, &of, callbacks)
        })
        .collect::<Result<_, _>>()?;
    Ok(Report { sizes })
}

fn measure_size(
    size: [u32; 2],
    frames: &[&Recorded],
    callbacks: Callbacks,
) -> Result<Sized, MeasureError> {
    // The textures of the whole run, by id, each one resource.
    let mut textures: Vec<Texture> = Vec::new();
    let mut notes: Vec<(bool, Option<TextureId>, String)> = Vec::new();
    for &recorded in frames {
        for texture in &recorded.frame.textures {
            let note = recorded.textures.iter().find(|n| n.id == texture.id);
            let pinned = note.is_none_or(|n| n.pinned);
            match textures.iter().position(|t| t.id == texture.id) {
                Some(i) if textures[i] != *texture => {
                    return Err(MeasureError::Texture(texture.id));
                }
                Some(i) => notes[i].0 |= pinned,
                None => {
                    textures.push(*texture);
                    notes.push((
                        pinned,
                        note.and_then(|n| n.alias),
                        note.map_or_else(String::new, |n| n.label.clone()),
                    ));
                }
            }
        }
    }
    let global = |id: TextureId| -> ResourceId {
        ResourceId(
            textures
                .iter()
                .position(|t| t.id == id)
                .expect("every texture of the run is listed") as u32,
        )
    };
    let mut states: Vec<StateOf<'_>> = Vec::new();
    let mut checks = Vec::new();
    for &recorded in frames {
        let projection =
            project(&recorded.frame).map_err(|e| MeasureError::Project(recorded.seq, e))?;
        let declared: Vec<(u32, Vec<crate::trace::Span>)> = recorded
            .blocks
            .iter()
            .map(|b| (b.block, b.declared.clone()))
            .collect();
        let blocks = check_blocks(&recorded.frame, &declared);
        let state = match states
            .iter()
            .position(|s| s.projection == projection && s.first.blocks == recorded.blocks)
        {
            Some(i) => i,
            None => {
                states.push(StateOf {
                    projection,
                    first: recorded,
                });
                states.len() - 1
            }
        };
        checks.push(FrameCheck {
            seq: recorded.seq,
            state,
            blocks,
            errors: recorded.errors.clone(),
            guards: recorded.guards.clone(),
        });
    }
    let resources = textures
        .iter()
        .zip(&notes)
        .map(|(t, (pinned, alias, _))| Resource {
            levels: t.levels,
            channels: t.channels,
            bytes: t.bytes,
            pinned: *pinned,
            alias: alias
                .filter(|a| textures.iter().any(|t| t.id == *a))
                .map(global),
        })
        .collect();
    let mut graph = Graph {
        resources,
        ..Graph::default()
    };
    for state in &states {
        graph
            .states
            .push(state_events(state, &global, &textures, callbacks));
    }
    let count = graph.states.len() as u32;
    for from in 0..count {
        for to in 0..count {
            graph.edges.push(Edge {
                from: StateId(from),
                transition: Transition::Steady,
                to: StateId(to),
            });
        }
    }
    let plan = plan(&graph).map_err(MeasureError::Input)?;
    let mut sized = report(&states, &graph, &textures, &notes, plan, checks);
    sized.size = size;
    let most = |keep: &dyn Fn(&Texture) -> bool| {
        frames
            .iter()
            .map(|f| {
                f.frame
                    .textures
                    .iter()
                    .filter(|t| keep(t))
                    .map(|t| t.bytes)
                    .sum::<u64>()
            })
            .max()
            .unwrap_or(0)
    };
    let pinned = |t: &Texture| notes[textures.iter().position(|x| x.id == t.id).expect("listed")].0;
    sized.allocated = most(&|_| true);
    sized.plannable = most(&|t| !pinned(t) && t.levels == 1);
    sized.dead_bytes = most(&|t| sized.dead.iter().any(|d| d.texture == t.id));
    Ok(sized)
}

fn state_events(
    state: &StateOf<'_>,
    global: &dyn Fn(TextureId) -> ResourceId,
    textures: &[Texture],
    callbacks: Callbacks,
) -> State {
    let local = state.projection.to_graph();
    let remap = |id: ResourceId| global(state.projection.textures[id.0 as usize].id);
    let mut events = local.states.into_iter().next().unwrap_or_default().events;
    for (event, step) in events.iter_mut().zip(&state.projection.steps) {
        for access in event
            .reads
            .iter_mut()
            .chain(event.writes.iter_mut().map(|w| &mut w.access))
        {
            match access {
                Access::Resolved(region) => region.resource = remap(region.resource),
                Access::Unresolved(set) => set.iter_mut().for_each(|r| *r = remap(*r)),
            }
        }
        let Unit::Block(block) = step.unit else {
            continue;
        };
        // A block reaches its declared set, not only what this frame touched in it.
        let note: Option<&BlockNote> = state.first.blocks.iter().find(|b| b.block == block);
        event.reads.clear();
        event.writes.clear();
        let Some(note) = note else { continue };
        for span in &note.declared {
            let region = Access::Resolved(Region {
                resource: global(span.texture),
                levels: span.levels,
                channels: span.channels,
            });
            event.reads.push(region.clone());
            event.writes.push(Write {
                access: region,
                cover: Cover::Partial,
                attachment: false,
                clear: false,
            });
        }
        if callbacks == Callbacks::Unresolved && !note.unresolved.is_empty() {
            let set: Vec<ResourceId> = note
                .unresolved
                .iter()
                .filter(|t| textures.iter().any(|x| x.id == **t))
                .map(|&t| global(t))
                .collect();
            if !set.is_empty() {
                event.reads.push(Access::Unresolved(set.clone()));
                event.writes.push(Write {
                    access: Access::Unresolved(set),
                    cover: Cover::Partial,
                    attachment: false,
                    clear: false,
                });
            }
        }
    }
    State { events }
}

fn work_label(recorded: &Recorded, work: u32) -> String {
    recorded
        .works
        .iter()
        .find(|(w, _)| *w == work)
        .map_or_else(|| format!("unit {work}"), |(_, l)| l.clone())
}

fn unit_work(step: &Step) -> Option<u32> {
    match step.unit {
        Unit::Work(w) => Some(w),
        Unit::Block(_) => None,
    }
}

fn report(
    states: &[StateOf<'_>],
    graph: &Graph,
    textures: &[Texture],
    notes: &[(bool, Option<TextureId>, String)],
    plan: Plan,
    frames: Vec<FrameCheck>,
) -> Sized {
    let step = |id: crate::model::EventId| {
        &states[id.state.0 as usize].projection.steps[id.index as usize]
    };
    let removed: Vec<Removed> = plan
        .removed
        .iter()
        .filter_map(|&(id, _)| {
            let work = unit_work(step(id))?;
            let state = id.state.0 as usize;
            Some(Removed {
                state,
                work,
                label: work_label(states[state].first, work),
            })
        })
        .collect();
    let folded: Vec<Folded> = plan
        .folded
        .iter()
        .filter_map(|f| {
            let state = f.clear.state.0 as usize;
            let draw = unit_work(step(f.into))?;
            Some(Folded {
                state,
                clear: unit_work(step(f.clear))?,
                texture: textures[f.resource.0 as usize].id,
                draw,
                draw_label: work_label(states[state].first, draw),
            })
        })
        .collect();
    let dead: Vec<Dead> = plan
        .dead
        .iter()
        .map(|&r| Dead {
            texture: textures[r.0 as usize].id,
            label: notes[r.0 as usize].2.clone(),
            bytes: graph.resource(r).bytes,
        })
        .collect();
    let dead_ids: Vec<TextureId> = dead.iter().map(|d| d.texture).collect();
    let state_reports = states
        .iter()
        .enumerate()
        .map(|(index, state)| {
            let edits = Edits {
                removed: removed
                    .iter()
                    .filter(|r| r.state == index)
                    .map(|r| r.work)
                    .collect(),
                folded: folded
                    .iter()
                    .filter(|f| f.state == index)
                    .map(|f| (f.clear, f.texture, f.draw))
                    .collect(),
                dead: dead_ids.clone(),
            };
            let planned = apply(&state.projection, &edits);
            (
                work(&state.projection),
                check_edits(&state.projection, &planned, &edits),
            )
        })
        .collect();
    Sized {
        size: [0, 0],
        frames,
        states: state_reports,
        allocated: 0,
        plannable: 0,
        removed,
        folded,
        dead,
        dead_bytes: 0,
        savings: plan.savings,
    }
}

fn work(projection: &Projection) -> Work {
    let mut work = Work::default();
    for step in &projection.steps {
        match step.unit {
            Unit::Block(_) => work.blocks += 1,
            Unit::Work(_) => {
                if step
                    .touches
                    .iter()
                    .any(|t| matches!(t.how, Use::Attach(_) | Use::Clear(_)))
                {
                    work.passes += 1;
                }
                work.clears += step
                    .touches
                    .iter()
                    .filter(|t| matches!(t.how, Use::Clear(_)))
                    .count() as u32;
            }
        }
    }
    work
}

/// The frame a runtime executing `edits` would record, built from the reference: what [`check_edits`] holds it to.
pub fn apply(reference: &Projection, edits: &Edits) -> Projection {
    let mut steps: Vec<Step> = reference
        .steps
        .iter()
        .filter(|s| !matches!(s.unit, Unit::Work(w) if edits.removed.contains(&w)))
        .cloned()
        .collect();
    for &(clear, texture, draw) in &edits.folded {
        let mut value = None;
        if let Some(step) = steps.iter_mut().find(|s| s.unit == Unit::Work(clear))
            && let Some(at) = step
                .touches
                .iter()
                .position(|t| t.texture == texture && matches!(t.how, Use::Clear(_)))
            && let Use::Clear(v) = step.touches.remove(at).how
        {
            value = Some(v);
        }
        if let Some(v) = value
            && let Some(step) = steps.iter_mut().find(|s| s.unit == Unit::Work(draw))
        {
            for touch in &mut step.touches {
                if touch.texture == texture && touch.how == Use::Attach(Load::Keep) {
                    touch.how = Use::Attach(Load::Clear(v));
                    break;
                }
            }
            step.touches.sort_unstable();
        }
    }
    for step in &mut steps {
        step.touches
            .retain(|t| !(matches!(t.how, Use::Clear(_)) && edits.dead.contains(&t.texture)));
    }
    steps.retain(|s| s.kind != EventKind::Clear || s.effects || !s.touches.is_empty());
    Projection {
        textures: reference
            .textures
            .iter()
            .filter(|t| !edits.dead.contains(&t.id))
            .copied()
            .collect(),
        steps,
    }
}
