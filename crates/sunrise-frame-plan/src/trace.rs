//! A frame as the runtime ran it, and its projection onto the event model.
//!
//! A trace names physical textures and units of work (a draw, a dispatch, a clear pass, a copy) by the runtime's own
//! numbers, lists the textures allocated for the frame, records each texture use with its binding or attachment
//! slot and each clear with its value, and marks the span of each opaque block. The projection makes each texture a
//! resource and each block's span one `Opaque` event. Two checks run on it: [`check_blocks`] (everything a block
//! touched is in the access set the lowering declared for it) and [`check_edits`] (a planned frame is the reference
//! frame with only the plan's edits).

use crate::model::{
    Access, Cover, Event, EventKind, Graph, Guard, Levels, Region, Resource, ResourceId, State,
    Write,
};

/// A physical texture of the runtime, by a number that stays the same for that texture in every trace of a run.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TextureId(pub u32);

/// An allocated texture.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Texture {
    pub id: TextureId,
    pub levels: u8,
    pub channels: u8,
    pub bytes: u64,
}

/// The bits a clear writes, as the runtime gives them (four 32-bit words).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ClearValue(pub [u32; 4]);

/// An attachment's load operation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Load {
    Keep,
    Clear(ClearValue),
}

/// How a unit of work used a texture.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Use {
    /// Bound for reading (a sampler, a read-only image).
    Sample,
    /// Bound for reading and writing (a storage image).
    Storage,
    /// A colour attachment, with its load operation.
    Attach(Load),
    /// A copy's source levels.
    CopyFrom,
    /// A copy's destination levels.
    CopyTo,
    /// An attachment clear, with its value.
    Clear(ClearValue),
}

/// One texture use: the unit's binding or attachment slot, the texture, and the levels and channels it reached.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Touch {
    pub slot: u32,
    pub texture: TextureId,
    pub levels: Levels,
    pub channels: u8,
    pub how: Use,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Op {
    /// A unit of work the runtime ran, with every texture use it bound (its bind groups resolved).
    Run {
        work: u32,
        kind: EventKind,
        effects: bool,
        removable: bool,
        touches: Vec<Touch>,
    },
    /// Everything up to the matching end is one opaque block.
    BlockStart(u32),
    BlockEnd(u32),
}

/// One traced frame: the textures allocated for it, and what ran.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TraceFrame {
    pub textures: Vec<Texture>,
    pub ops: Vec<Op>,
}

/// What a projected step stands for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unit {
    Work(u32),
    Block(u32),
}

/// One event of the projection: a unit of work, or a whole block. Its touches are sorted, without repeats.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Step {
    pub unit: Unit,
    pub kind: EventKind,
    pub effects: bool,
    pub removable: bool,
    pub touches: Vec<Touch>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Projection {
    /// The allocated textures, in the trace's order.
    pub textures: Vec<Texture>,
    pub steps: Vec<Step>,
}

/// One texture's levels and channels, as a lowering declares a block's access set.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Span {
    pub texture: TextureId,
    pub levels: Levels,
    pub channels: u8,
}

/// A trace the projection or a check refuses, and where.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TraceError {
    /// A block start inside a block, or an end that doesn't match the open block, at this op.
    Block(usize),
    /// A block still open at the frame's end.
    Unclosed(u32),
    /// A texture listed twice.
    DuplicateTexture(TextureId),
    /// A use of a texture the frame didn't allocate, at this op.
    UnknownTexture(usize),
    /// A block with no declared access set.
    Undeclared(u32),
    /// A touch inside this block outside its declared access set.
    Outside { block: u32, texture: TextureId },
    /// A plan edit names something the reference frame doesn't have.
    Edit(EditError),
    /// After the edits, this step still touches a dead texture.
    DeadTouched(usize),
    /// The planned frame allocates other textures than the reference minus the dead ones; the first texture that
    /// differs.
    Allocation(TextureId),
    /// The planned frame differs from the edited reference from this step on.
    Differs(usize),
}

/// A plan edit the reference frame can't take.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditError {
    /// No clear of this texture in this unit.
    NoClear { work: u32, texture: TextureId },
    /// No kept-load attachment of this texture in this unit.
    NoAttachment { work: u32, texture: TextureId },
}

impl std::fmt::Display for TraceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "frame trace refused: {self:?}")
    }
}

impl std::error::Error for TraceError {}

/// Projects a trace: each run outside a block is a step, each block's span one `Opaque` step holding every touch
/// inside it.
pub fn project(frame: &TraceFrame) -> Result<Projection, TraceError> {
    for (i, texture) in frame.textures.iter().enumerate() {
        if frame.textures[..i].iter().any(|t| t.id == texture.id) {
            return Err(TraceError::DuplicateTexture(texture.id));
        }
    }
    let mut steps = Vec::new();
    let mut open: Option<Step> = None;
    for (index, op) in frame.ops.iter().enumerate() {
        match op {
            Op::BlockStart(block) => {
                if open.is_some() {
                    return Err(TraceError::Block(index));
                }
                open = Some(Step {
                    unit: Unit::Block(*block),
                    kind: EventKind::Opaque,
                    effects: true,
                    removable: false,
                    touches: Vec::new(),
                });
            }
            Op::BlockEnd(block) => match open.take() {
                Some(mut step) if step.unit == Unit::Block(*block) => {
                    tidy(&mut step.touches);
                    steps.push(step);
                }
                _ => return Err(TraceError::Block(index)),
            },
            Op::Run {
                work,
                kind,
                effects,
                removable,
                touches,
            } => {
                if touches
                    .iter()
                    .any(|t| !frame.textures.iter().any(|x| x.id == t.texture))
                {
                    return Err(TraceError::UnknownTexture(index));
                }
                match &mut open {
                    Some(block) => block.touches.extend_from_slice(touches),
                    None => {
                        let mut touches = touches.clone();
                        tidy(&mut touches);
                        steps.push(Step {
                            unit: Unit::Work(*work),
                            kind: *kind,
                            effects: *effects,
                            removable: *removable,
                            touches,
                        });
                    }
                }
            }
        }
    }
    if let Some(Step {
        unit: Unit::Block(block),
        ..
    }) = open
    {
        return Err(TraceError::Unclosed(block));
    }
    Ok(Projection {
        textures: frame.textures.clone(),
        steps,
    })
}

fn tidy(touches: &mut Vec<Touch>) {
    touches.sort_unstable();
    touches.dedup();
}

/// Checks that every touch inside each block is inside the access set the lowering declared for it: for each level
/// the touch reaches, the declared spans of its texture that hold that level, together, hold the touch's channels.
pub fn check_blocks(frame: &TraceFrame, declared: &[(u32, Vec<Span>)]) -> Result<(), TraceError> {
    let mut block = None;
    for op in &frame.ops {
        match op {
            Op::BlockStart(b) => {
                let set = declared
                    .iter()
                    .find(|(d, _)| d == b)
                    .ok_or(TraceError::Undeclared(*b))?;
                block = Some((*b, &set.1));
            }
            Op::BlockEnd(_) => block = None,
            Op::Run { touches, .. } => {
                let Some((b, set)) = block else { continue };
                for touch in touches {
                    let levels =
                        touch.levels.first..touch.levels.first.saturating_add(touch.levels.count);
                    let inside = touch.levels.count > 0
                        && levels.clone().all(|level| {
                            let declared: u8 = set
                                .iter()
                                .filter(|span| {
                                    span.texture == touch.texture
                                        && span.levels.first <= level
                                        && u16::from(level)
                                            < u16::from(span.levels.first)
                                                + u16::from(span.levels.count)
                                })
                                .fold(0, |channels, span| channels | span.channels);
                            touch.channels & !declared == 0
                        });
                    if !inside {
                        return Err(TraceError::Outside {
                            block: b,
                            texture: touch.texture,
                        });
                    }
                }
            }
        }
    }
    Ok(())
}

/// A plan's edits in the runtime's numbers.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Edits {
    /// Units that don't run: removed draws and their bind-time work.
    pub removed: Vec<u32>,
    /// (clear unit, texture, draw unit): the clear goes, and the draw's kept attachment to the texture loads as a
    /// clear with the clear's value.
    pub folded: Vec<(u32, TextureId, u32)>,
    /// Textures that lose their clears and their storage.
    pub dead: Vec<TextureId>,
}

/// Checks that `planned` is `reference` with only `edits` applied: removed units gone; folded clears moved, with
/// their values, into their draw's load at the same attachment slot; dead textures' clears and allocations gone and
/// nothing else touching them. Every other step, use, slot and allocation must be there unchanged: flips, triggers
/// and compute are the reference's own steps.
pub fn check_edits(
    reference: &Projection,
    planned: &Projection,
    edits: &Edits,
) -> Result<(), TraceError> {
    let mut expected: Vec<Step> = reference
        .steps
        .iter()
        .filter(|s| !matches!(s.unit, Unit::Work(w) if edits.removed.contains(&w)))
        .cloned()
        .collect();
    for &(clear, texture, draw) in &edits.folded {
        let no_clear = TraceError::Edit(EditError::NoClear {
            work: clear,
            texture,
        });
        let step = expected
            .iter_mut()
            .find(|s| s.unit == Unit::Work(clear))
            .ok_or(no_clear)?;
        let at = step
            .touches
            .iter()
            .position(|t| t.texture == texture && matches!(t.how, Use::Clear(_)))
            .ok_or(no_clear)?;
        let Use::Clear(value) = step.touches.remove(at).how else {
            unreachable!("found as a clear")
        };
        let attachment = expected
            .iter_mut()
            .find(|s| s.unit == Unit::Work(draw))
            .and_then(|s| {
                let touch = s
                    .touches
                    .iter_mut()
                    .find(|t| t.texture == texture && t.how == Use::Attach(Load::Keep))?;
                touch.how = Use::Attach(Load::Clear(value));
                tidy(&mut s.touches);
                Some(())
            });
        attachment.ok_or(TraceError::Edit(EditError::NoAttachment {
            work: draw,
            texture,
        }))?;
    }
    for step in &mut expected {
        if step.kind == EventKind::Clear {
            step.touches.retain(|t| !edits.dead.contains(&t.texture));
        }
    }
    expected.retain(|s| s.kind != EventKind::Clear || s.effects || !s.touches.is_empty());
    if let Some(index) = expected
        .iter()
        .position(|s| s.touches.iter().any(|t| edits.dead.contains(&t.texture)))
    {
        return Err(TraceError::DeadTouched(index));
    }
    let mut allocated: Vec<Texture> = reference
        .textures
        .iter()
        .filter(|t| !edits.dead.contains(&t.id))
        .copied()
        .collect();
    let mut planned_allocated = planned.textures.clone();
    allocated.sort_unstable_by_key(|t| t.id);
    planned_allocated.sort_unstable_by_key(|t| t.id);
    if allocated != planned_allocated {
        let first = allocated
            .iter()
            .zip(&planned_allocated)
            .find(|(e, p)| e != p)
            .map(|(e, p)| e.id.min(p.id))
            .or_else(|| {
                let longer = if allocated.len() > planned_allocated.len() {
                    &allocated
                } else {
                    &planned_allocated
                };
                longer
                    .get(allocated.len().min(planned_allocated.len()))
                    .map(|t| t.id)
            });
        return Err(TraceError::Allocation(
            first.expect("lists that differ differ somewhere"),
        ));
    }
    let first_difference = expected
        .iter()
        .zip(&planned.steps)
        .position(|(e, p)| e != p)
        .or_else(|| {
            (expected.len() != planned.steps.len()).then(|| expected.len().min(planned.steps.len()))
        });
    match first_difference {
        Some(index) => Err(TraceError::Differs(index)),
        None => Ok(()),
    }
}

impl Projection {
    /// The resource a texture is in [`Projection::to_graph`]'s graph: its place in the texture list.
    pub fn resource(&self, texture: TextureId) -> Option<ResourceId> {
        self.textures
            .iter()
            .position(|t| t.id == texture)
            .map(|i| ResourceId(i as u32))
    }

    /// The projection as a one-state graph with no edges: each allocated texture a resource, each step an
    /// unguarded event. A block reads and writes everything it touched. A kept attachment is a partial write that
    /// may take a folded clear; an attachment that loads as a clear, a copy's destination and a clear are full
    /// writes, which take none.
    pub fn to_graph(&self) -> Graph {
        let resources = self
            .textures
            .iter()
            .map(|t| Resource {
                levels: t.levels,
                channels: t.channels,
                bytes: t.bytes,
                pinned: false,
                alias: None,
            })
            .collect();
        let events = self.steps.iter().map(|step| self.event(step)).collect();
        Graph {
            resources,
            states: vec![State { events }],
            edges: Vec::new(),
            observers: Vec::new(),
        }
    }

    fn event(&self, step: &Step) -> Event {
        let mut reads = Vec::new();
        let mut writes = Vec::new();
        for touch in &step.touches {
            let resource = self
                .resource(touch.texture)
                .expect("the projection checked every texture");
            let region = Access::Resolved(Region {
                resource,
                levels: touch.levels,
                channels: touch.channels,
            });
            let write = |cover, attachment| Write {
                access: region.clone(),
                cover,
                attachment,
            };
            if step.kind == EventKind::Opaque {
                reads.push(region.clone());
                writes.push(write(Cover::Partial, false));
                continue;
            }
            match touch.how {
                Use::Sample | Use::CopyFrom => reads.push(region.clone()),
                Use::Storage => {
                    reads.push(region.clone());
                    writes.push(write(Cover::Partial, false));
                }
                Use::Attach(Load::Keep) => {
                    writes.push(write(Cover::Partial, step.kind == EventKind::Work))
                }
                Use::Attach(Load::Clear(_)) | Use::CopyTo | Use::Clear(_) => {
                    writes.push(write(Cover::Full, false))
                }
            }
        }
        Event {
            kind: step.kind,
            guard: Guard::Always,
            reads,
            writes,
            effects: step.effects,
            removable: step.removable,
        }
    }
}
