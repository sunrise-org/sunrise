//! The frame trace (#6552, step 1): with the `plan-trace` feature built and `SUNRISE_PLAN_TRACE=DIR` set, each
//! frame `run_frame` draws is recorded as the runtime resolved it, and some are written to DIR as
//! `sunrise_frame_plan::record` frames for `frame-plan-measure` (`tools/pack_plan_trace.py`).
//!
//! What a frame records, in the order the runtime encodes it:
//! - the textures the frame may write, each by a number that stays the texture's for the run: the colortex sides
//!   (colortex1 to 15 the only ones the planner may plan on, a B side falling back to its A side), their scratch
//!   copies, depth and its copies, the water patches, mod depth targets, the sink, the shadow targets, the pack's
//!   images, their copies and stand-ins, the shadow stand-in, `final`'s picture, the world, its depth and the screen;
//! - every unit of work where it is encoded (a render or compute pass, a clear pass, a copy; nothing where nothing is
//!   encoded): each clear with its bits, each copy, each draw and dispatch with the textures of every bind group set
//!   on it (side and mip levels, as the group was made) and every attachment with its load operation; an attachment
//!   a draw only clears as it begins (the world blit's) is a clear of that unit, not an output;
//! - the shadow pass, the opaque and translucent gbuffers and the after-level callback as blocks, each with the
//!   access set declared for it from its passes' bindings and copies (`PackPost::declared`);
//! - the guards the frame ran with: the Shadow Cache on and hit, mod draws, whether a world callback drew, and a
//!   size-skipped first `deferred` followed by one that ran.
//!
//! The recorder keeps no GPU handle: textures, views and bind groups are known by the hash of their unique id
//! (wgpu's own `Hash`), and textures numbered in the order they are first seen, so the runtime frees what it drops
//! (a resize's old targets) as without the trace.
//!
//! A texture the frame only reads and never writes (the atlas, noise, a pack's own textures, stand-in samplers) is
//! not listed: it carries nothing from one draw to another. A write into a texture the frame doesn't list is
//! recorded as an error of the frame.
//!
//! A frame is written when it is one of the first `SUNRISE_PLAN_TRACE_FRAMES` (4 when unset) after the pack's
//! first frame, a reset of its history, a change of render size or a switch of the Shadow Cache.

use std::collections::HashMap;
use std::path::PathBuf;

use sunrise_frame_plan::model::{EventKind, Levels};
use sunrise_frame_plan::record::{BlockNote, Recorded, Start, TextureNote};
use sunrise_frame_plan::trace::{
    ClearValue, Load, Op, Span, Texture, TextureId, Touch, TraceFrame, Use,
};
use sunrise_shaders::{BUFFERS, SHADOW_COLORS};

use super::construction::PackPost;
use super::execution::WorldBoundaries;
use super::mod_pack::ModPassSlot;
use super::types::{Image, Pass, Source};

/// The folder frames are written to; unset, nothing is recorded.
pub const ENV: &str = "SUNRISE_PLAN_TRACE";
/// How many frames are written after each trigger.
pub const FRAMES_ENV: &str = "SUNRISE_PLAN_TRACE_FRAMES";
/// The frames written after a trigger when `FRAMES_ENV` is unset: the trigger's frame, then three that show
/// whether the frames after it settle into one state or alternate (a buffer that flips every frame).
const FRAMES: u32 = 4;

/// The blocks a frame can hold, by their number in the trace.
pub const SHADOW: u32 = 0;
pub const OPAQUE: u32 = 1;
pub const TRANSLUCENT: u32 = 2;
pub const AFTER_LEVEL: u32 = 3;

/// A colour clear's value as the bits of its four `f32`s; a depth clear's as its `f32` in the first word.
pub fn colour(c: wgpu::Color) -> ClearValue {
    ClearValue([c.r, c.g, c.b, c.a].map(|v| (v as f32).to_bits()))
}

pub fn depth(d: f32) -> ClearValue {
    ClearValue([d.to_bits(), 0, 0, 0])
}

/// A buffer's clear colour (`PackPost::clears`) as the bits a clear of it writes.
pub fn colour4(c: [f32; 4]) -> ClearValue {
    ClearValue(c.map(f32::to_bits))
}

/// The slot a depth attachment is recorded at, past every colour attachment slot.
pub const DEPTH_SLOT: u32 = 64;

/// The uses of a render pass's colour attachments, each at its slot.
pub fn attachments(
    r: &mut Recorder,
    attachments: &[Option<wgpu::RenderPassColorAttachment<'_>>],
    what: &str,
) -> Vec<Option<Touch>> {
    attachments
        .iter()
        .enumerate()
        .filter_map(|(slot, a)| {
            let a = a.as_ref()?;
            let how = match a.ops.load {
                wgpu::LoadOp::Clear(c) => Use::Attach(Load::Clear(colour(c))),
                _ => Use::Attach(Load::Keep),
            };
            Some(r.touch(a.view, slot as u32, how, what))
        })
        .collect()
}

/// The use of a render pass's depth attachment.
pub fn depth_attachment(
    r: &mut Recorder,
    view: &wgpu::TextureView,
    load: wgpu::LoadOp<f32>,
    what: &str,
) -> Option<Touch> {
    let how = match load {
        wgpu::LoadOp::Clear(d) => Use::Attach(Load::Clear(depth(d))),
        _ => Use::Attach(Load::Keep),
    };
    r.touch(view, DEPTH_SLOT, how, what)
}

struct Frame {
    seq: u64,
    size: [u32; 2],
    start: Start,
    ops: Vec<Op>,
    works: Vec<(u32, String)>,
    blocks: Vec<BlockNote>,
    guards: Vec<(String, bool)>,
    /// Whether the frame's first `deferred` pass was size-skipped, once one was met.
    first_deferred_skipped: Option<bool>,
    errors: Vec<String>,
    textures: Vec<(Texture, TextureNote)>,
}

/// The guards a frame records (`Recorder::guard`).
pub const SHADOW_CACHE_ON: &str = "shadow cache on";
pub const SHADOW_CACHE_HIT: &str = "shadow cache hit";
pub const MOD_DRAWS: &str = "mod draws";
pub const CALLBACK_DREW: &str = "a world callback drew";
pub const DEFERRED_SKIPPED_THEN_RAN: &str = "first deferred size-skipped, a later one ran";

/// A GPU handle's identity, kept without keeping the handle: wgpu hashes a texture, a view or a bind group by its
/// own unique id, so the recorder holds no texture alive past the runtime's last use of it.
type Key = u64;

fn key<T: std::hash::Hash>(handle: &T) -> Key {
    use std::hash::Hasher;
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    handle.hash(&mut hasher);
    hasher.finish()
}

/// A bind group's texture entries, as it was made: binding, the view and its texture, bound as an image.
type Entries = Vec<(u32, Key, Key, bool)>;

pub struct Recorder {
    dir: PathBuf,
    per_trigger: u32,
    /// Frames still to write.
    left: u32,
    seq: u64,
    /// Each texture's number, and the next number: numbers are never reused.
    ids: HashMap<Key, TextureId>,
    next: u32,
    /// This frame's views, by what they show.
    views: HashMap<Key, (TextureId, Levels)>,
    groups: HashMap<Key, Entries>,
    frame: Option<Frame>,
    reset: bool,
    last: Option<([u32; 2], bool)>,
}

impl Recorder {
    /// The recorder `SUNRISE_PLAN_TRACE` asks for, if any. A folder that can't be made turns it off with a warning.
    pub fn from_env() -> Option<Self> {
        let dir = PathBuf::from(std::env::var_os(ENV)?);
        if let Err(e) = std::fs::create_dir_all(&dir) {
            log::warn!(
                "frame trace: can't make {}: {e}; not recording",
                dir.display()
            );
            return None;
        }
        let per_trigger = std::env::var(FRAMES_ENV)
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(FRAMES);
        Some(Self::new(dir, per_trigger))
    }

    pub fn new(dir: PathBuf, per_trigger: u32) -> Self {
        Self {
            dir,
            per_trigger,
            left: 0,
            seq: 0,
            ids: HashMap::new(),
            next: 0,
            views: HashMap::new(),
            groups: HashMap::new(),
            frame: None,
            reset: false,
            last: None,
        }
    }

    /// The runtime forgot its history: the next frame starts over.
    pub fn reset(&mut self) {
        self.reset = true;
        self.groups.clear();
    }

    /// The runtime dropped its bind groups.
    pub fn forget_groups(&mut self) {
        self.groups.clear();
    }

    /// The runtime dropped `group` from its caches.
    pub fn forget_group(&mut self, group: &wgpu::BindGroup) {
        self.groups.remove(&key(group));
    }

    /// The bind groups the recorder knows: what it keeps (as numbers) for groups the runtime still caches.
    pub fn groups_known(&self) -> usize {
        self.groups.len()
    }

    /// A new frame at `size`; an earlier frame still open is dropped (its `run_frame` returned early).
    pub fn begin(&mut self, size: [u32; 2], shadow_cache: bool) {
        let start = if std::mem::take(&mut self.reset) {
            Start::Reset
        } else {
            match self.last {
                None => Start::First,
                Some((last, _)) if last != size => Start::Resize,
                _ => Start::Steady,
            }
        };
        let switched = self.last.is_some_and(|(_, cache)| cache != shadow_cache);
        if start != Start::Steady || switched {
            self.left = self.per_trigger;
        }
        self.last = Some((size, shadow_cache));
        self.views.clear();
        self.seq += 1;
        self.frame = Some(Frame {
            seq: self.seq,
            size,
            start,
            ops: Vec::new(),
            works: Vec::new(),
            blocks: Vec::new(),
            guards: vec![(SHADOW_CACHE_ON.to_owned(), shadow_cache)],
            first_deferred_skipped: None,
            errors: Vec::new(),
            textures: Vec::new(),
        });
    }

    /// Notes the value guard `name` had in this frame (once per name: a later value is or-ed in).
    pub fn guard(&mut self, name: &str, value: bool) {
        let Some(frame) = &mut self.frame else { return };
        match frame.guards.iter_mut().find(|(n, _)| n == name) {
            Some((_, v)) => *v |= value,
            None => frame.guards.push((name.to_owned(), value)),
        }
    }

    /// A `deferred` pass, skipped for its buffers' sizes or not.
    pub fn deferred(&mut self, skipped: bool) {
        let Some(frame) = &mut self.frame else { return };
        match frame.first_deferred_skipped {
            None => frame.first_deferred_skipped = Some(skipped),
            Some(first) => self.guard(DEFERRED_SKIPPED_THEN_RAN, first && !skipped),
        }
    }

    fn id(&mut self, texture: &wgpu::Texture) -> TextureId {
        let next = &mut self.next;
        *self.ids.entry(key(texture)).or_insert_with(|| {
            *next += 1;
            TextureId(*next - 1)
        })
    }

    /// Lists `texture` for this frame (once; a later listing of it adds nothing).
    pub fn texture(
        &mut self,
        texture: &wgpu::Texture,
        label: &str,
        pinned: bool,
        alias: Option<&wgpu::Texture>,
    ) {
        let id = self.id(texture);
        let alias = alias.filter(|a| *a != texture).map(|a| self.id(a));
        let Some(frame) = &mut self.frame else { return };
        if frame.textures.iter().any(|(t, _)| t.id == id) {
            return;
        }
        let format = texture.format();
        let size = texture.size();
        let texel = u64::from(
            format
                .block_copy_size(None)
                .or_else(|| format.block_copy_size(Some(wgpu::TextureAspect::DepthOnly)))
                .unwrap_or(4),
        );
        let levels = texture.mip_level_count();
        let bytes = (0..levels)
            .map(|l| {
                u64::from((size.width >> l).max(1))
                    * u64::from((size.height >> l).max(1))
                    * u64::from(size.depth_or_array_layers)
                    * texel
            })
            .sum();
        frame.textures.push((
            Texture {
                id,
                levels: levels.min(u32::from(u8::MAX)) as u8,
                channels: format.components().max(1),
                bytes,
            },
            TextureNote {
                id,
                pinned,
                alias,
                label: label.to_owned(),
            },
        ));
    }

    /// `view` shows `levels` of its texture, which is listed.
    pub fn view(&mut self, view: &wgpu::TextureView, levels: Levels) {
        if let Some(&id) = self.ids.get(&key(view.texture())) {
            self.views.insert(key(view), (id, levels));
        }
    }

    fn listed(&self, id: TextureId) -> Option<&Texture> {
        let frame = self.frame.as_ref()?;
        frame
            .textures
            .iter()
            .find(|(t, _)| t.id == id)
            .map(|(t, _)| t)
    }

    /// The texture and levels a view shows, if its texture is listed: as the view was noted, else every level.
    fn resolve(&self, view: Key, texture: Key) -> Option<(TextureId, Levels)> {
        if let Some(&found) = self.views.get(&view) {
            return Some(found);
        }
        self.texture_levels(texture, None)
    }

    fn texture_levels(&self, texture: Key, levels: Option<Levels>) -> Option<(TextureId, Levels)> {
        let id = *self.ids.get(&texture)?;
        let listed = self.listed(id)?;
        Some((
            id,
            levels.unwrap_or(Levels {
                first: 0,
                count: listed.levels,
            }),
        ))
    }

    fn touch_of(
        &mut self,
        found: Option<(TextureId, Levels)>,
        slot: u32,
        how: Use,
        what: &str,
    ) -> Option<Touch> {
        let Some((texture, levels)) = found else {
            let writes = !matches!(how, Use::Sample | Use::CopyFrom);
            if writes && let Some(frame) = &mut self.frame {
                frame.errors.push(format!(
                    "{what}: a write into a texture the frame doesn't list"
                ));
            }
            return None;
        };
        let channels = self.listed(texture).map_or(1, |t| t.channels);
        Some(Touch {
            slot,
            texture,
            levels,
            channels: ((1u16 << channels) - 1) as u8,
            how,
        })
    }

    /// A use of `view`, for a unit of work labelled `what`.
    pub fn touch(
        &mut self,
        view: &wgpu::TextureView,
        slot: u32,
        how: Use,
        what: &str,
    ) -> Option<Touch> {
        let found = self.resolve(key(view), key(view.texture()));
        self.touch_of(found, slot, how, what)
    }

    /// A use of `levels` (every level when `None`) of `texture`.
    pub fn touch_texture(
        &mut self,
        texture: &wgpu::Texture,
        levels: Option<Levels>,
        slot: u32,
        how: Use,
        what: &str,
    ) -> Option<Touch> {
        let found = self.texture_levels(key(texture), levels);
        self.touch_of(found, slot, how, what)
    }

    /// A unit of work: a render or compute pass, a clear or a copy, as it is encoded.
    pub fn run(
        &mut self,
        label: &str,
        kind: EventKind,
        effects: bool,
        removable: bool,
        touches: impl IntoIterator<Item = Option<Touch>>,
    ) {
        let Some(frame) = &mut self.frame else { return };
        let work = frame.works.len() as u32;
        frame.works.push((work, label.to_owned()));
        frame.ops.push(Op::Run {
            work,
            kind,
            effects,
            removable,
            touches: touches.into_iter().flatten().collect(),
        });
    }

    /// A copy of level 0 of `from` onto level 0 of `to`, whole.
    pub fn copy(&mut self, label: &str, from: &wgpu::Texture, to: &wgpu::Texture, effects: bool) {
        let touches = [
            self.touch_texture(from, Some(Levels::BASE), 0, Use::CopyFrom, label),
            self.touch_texture(to, Some(Levels::BASE), 1, Use::CopyTo, label),
        ];
        self.run(label, EventKind::Copy, effects, false, touches);
    }

    /// One render pass of attachment clears of `views`; none when `views` is empty (no pass is encoded).
    pub fn clears<'v>(
        &mut self,
        label: &str,
        views: impl IntoIterator<Item = (&'v wgpu::TextureView, ClearValue)>,
    ) {
        let touches: Vec<_> = views
            .into_iter()
            .enumerate()
            .map(|(slot, (view, value))| self.touch(view, slot as u32, Use::Clear(value), label))
            .collect();
        if !touches.is_empty() {
            self.run(label, EventKind::Clear, false, false, touches);
        }
    }

    /// A bind group made with `entries`: its texture views and whether each is bound as an image.
    pub fn group_made<'v>(
        &mut self,
        group: &wgpu::BindGroup,
        entries: impl IntoIterator<Item = (u32, &'v wgpu::TextureView, bool)>,
    ) {
        let entries = entries
            .into_iter()
            .map(|(binding, view, image)| (binding, key(view), key(view.texture()), image))
            .collect();
        self.groups.insert(key(group), entries);
    }

    /// The pass encoded last binds `group`: its textures are uses of that pass. A group the trace didn't see made,
    /// or a bind with no pass begun, is an error of the frame.
    pub fn bound(&mut self, group: &wgpu::BindGroup, what: &str) {
        let Some(entries) = self.groups.get(&key(group)).cloned() else {
            if let Some(frame) = &mut self.frame {
                frame
                    .errors
                    .push(format!("{what}: a bind group the trace didn't see made"));
            }
            return;
        };
        let touches: Vec<Touch> = entries
            .into_iter()
            .filter_map(|(binding, view, texture, image)| {
                let how = if image { Use::Storage } else { Use::Sample };
                let found = self.resolve(view, texture);
                self.touch_of(found, binding, how, what)
            })
            .collect();
        let Some(frame) = &mut self.frame else { return };
        match frame.ops.last_mut() {
            Some(Op::Run {
                kind: EventKind::Work,
                touches: into,
                ..
            }) => into.extend(touches),
            _ => frame
                .errors
                .push(format!("{what}: a bind group bound outside a pass")),
        }
    }

    /// Block `block` starts, reaching `declared` and, unresolved, `unresolved`.
    pub fn block_start(
        &mut self,
        block: u32,
        label: &str,
        declared: Vec<(wgpu::Texture, Option<Levels>)>,
        unresolved: Vec<wgpu::Texture>,
    ) {
        let mut spans: Vec<Span> = Vec::new();
        for (texture, levels) in declared {
            if let Some((id, levels)) = self.texture_levels(key(&texture), levels) {
                let channels = self.listed(id).map_or(1, |t| t.channels);
                let span = Span {
                    texture: id,
                    levels,
                    channels: ((1u16 << channels) - 1) as u8,
                };
                if !spans.contains(&span) {
                    spans.push(span);
                }
            }
        }
        let mut reached: Vec<TextureId> = unresolved
            .iter()
            .filter_map(|t| self.texture_levels(key(t), None).map(|(id, _)| id))
            .collect();
        reached.sort_unstable();
        reached.dedup();
        let Some(frame) = &mut self.frame else { return };
        frame.blocks.retain(|b| b.block != block);
        frame.blocks.push(BlockNote {
            block,
            label: label.to_owned(),
            declared: spans,
            unresolved: reached,
        });
        frame.ops.push(Op::BlockStart(block));
    }

    pub fn block_end(&mut self, block: u32) {
        if let Some(frame) = &mut self.frame {
            frame.ops.push(Op::BlockEnd(block));
        }
    }

    /// The frame is done: written when it is one of those asked for.
    pub fn end(&mut self) {
        self.guard(DEFERRED_SKIPPED_THEN_RAN, false);
        let Some(frame) = self.frame.take() else {
            return;
        };
        self.views.clear();
        if self.left == 0 {
            return;
        }
        self.left -= 1;
        let (textures, notes) = frame.textures.into_iter().unzip();
        let recorded = Recorded {
            seq: frame.seq,
            size: frame.size,
            start: frame.start,
            frame: TraceFrame {
                textures,
                ops: frame.ops,
            },
            textures: notes,
            works: frame.works,
            blocks: frame.blocks,
            guards: frame.guards,
            errors: frame.errors,
        };
        let path = self.dir.join(format!("frame-{:08}.trace", frame.seq));
        if let Err(e) = std::fs::write(&path, recorded.write()) {
            log::warn!("frame trace: can't write {}: {e}", path.display());
        }
    }
}

/// Lists every texture of `post`'s frame, and the views that show part of one.
pub fn list(
    r: &mut Recorder,
    post: &PackPost,
    world: &wgpu::TextureView,
    world_depth: &wgpu::TextureView,
    screen: &wgpu::TextureView,
) {
    let view = |r: &mut Recorder, v: &wgpu::TextureView, label: &str, pinned: bool| {
        r.texture(v.texture(), label, pinned, None);
        r.view(v, Levels::BASE);
    };
    view(r, world, "world", true);
    view(r, world_depth, "world depth", true);
    view(r, screen, "screen", true);
    if let Some(t) = &post.targets {
        for (i, sides) in t.color.iter().enumerate() {
            let Some([a, b]) = sides else { continue };
            r.texture(a.texture(), &format!("colortex{i} A"), i == 0, None);
            r.texture(
                b.texture(),
                &format!("colortex{i} B"),
                i == 0,
                Some(a.texture()),
            );
            r.view(a, Levels::BASE);
            r.view(b, Levels::BASE);
            if let Some(mips) = &t.mips[i] {
                for side in mips {
                    let all = Levels {
                        first: 0,
                        count: side.all.texture().mip_level_count() as u8,
                    };
                    r.view(&side.all, all);
                    for (level, (attachment, _)) in side.levels.iter().enumerate() {
                        r.view(
                            attachment,
                            Levels {
                                first: level as u8 + 1,
                                count: 1,
                            },
                        );
                    }
                }
            }
        }
        for (i, scratch) in t.scratch.iter().enumerate() {
            if let Some(v) = scratch {
                view(r, v, &format!("colortex{i} copy"), true);
            }
        }
        view(r, &t.depth, "depthtex0", true);
        for (label, d) in [
            ("depthtex1", &t.depth1),
            ("depthtex2", &t.depth2),
            ("water patches", &t.water_mask),
        ] {
            if let Some((_, v)) = d {
                view(r, v, label, true);
            }
        }
        for (i, d) in t.mod_depth.iter().enumerate() {
            if let Some((_, v)) = d {
                view(r, v, &format!("mod depth {i}"), true);
            }
        }
        for (i, d) in t.mod_opaque_depth.iter().enumerate() {
            if let Some((_, v)) = d {
                view(r, v, &format!("mod opaque depth {i}"), true);
            }
        }
        view(r, &t.sink, "sink", true);
    }
    if let Some(s) = &post.shadow_targets {
        for (n, sides) in s.color.iter().enumerate() {
            if let Some(sides) = sides {
                for (side, v) in sides.iter().enumerate() {
                    view(r, v, &format!("shadowcolor{n} {}", ["A", "B"][side]), true);
                }
            }
        }
        for (n, v) in s.scratch.iter().enumerate() {
            if let Some(v) = v {
                view(r, v, &format!("shadowcolor{n} copy"), true);
            }
        }
        view(r, &s.depth.1, "shadowtex0", true);
        if let Some((_, v)) = &s.depth1 {
            view(r, v, "shadowtex1", true);
        }
        if let Some(v) = &s.sink {
            view(r, v, "shadow sink", true);
        }
    }
    view(r, &post.shadow.0, "shadow stand-in", true);
    for image in &post.images {
        let name = &image.def.name;
        for (part, v) in [
            ("", &image.view),
            (" copy", &image.copy),
            (" before", &image.before),
        ] {
            if let Some(v) = v {
                view(r, v, &format!("image {name}{part}"), true);
            }
        }
    }
    for (k, s) in post.stand_ins.iter().enumerate() {
        view(r, &s.view, &format!("image stand-in {k}"), true);
    }
    if let Some(f) = &post.final_target {
        view(r, &f.view, "final picture", true);
    }
    if let Some((_, v)) = &post.portal_texture {
        view(r, v, "End portal sprite", true);
    }
}

/// What a declared set names: a texture and its levels (every level when `None`).
type Declared = Vec<(wgpu::Texture, Option<Levels>)>;

impl PackPost {
    /// The side of colortex `i` passes read and write now, and its levels: every level of a buffer with mipmaps.
    fn main_side(&self, i: usize, out: &mut Declared) {
        let Some(t) = &self.targets else { return };
        if let Some(sides) = &t.color[i] {
            let side = &sides[usize::from(self.main & 1 << i != 0)];
            let levels = (self.mipped & 1 << i == 0).then_some(Levels::BASE);
            out.push((side.texture().clone(), levels));
        }
    }

    /// Both sides of shadowcolor `n` and its copy: the shadow pass's A/B mapping isn't carried.
    fn shadow_color(&self, n: usize, out: &mut Declared) {
        let Some(s) = &self.shadow_targets else {
            return;
        };
        let views = s.color.get(n).into_iter().flatten().flatten();
        for v in views.chain(s.scratch.get(n).into_iter().flatten()) {
            out.push((v.texture().clone(), None));
        }
    }

    /// Both sides of every colortex buffer, and depth: what a callback into other code may reach.
    fn every_side(&self) -> Vec<wgpu::Texture> {
        let Some(t) = &self.targets else {
            return Vec::new();
        };
        let mut out: Vec<wgpu::Texture> = t
            .color
            .iter()
            .flatten()
            .flat_map(|sides| sides.iter().map(|v| v.texture().clone()))
            .collect();
        out.push(t.depth_texture.clone());
        out
    }

    /// Everything a gbuffers-style `pass` binds or attaches, from its bindings resolved through their sources
    /// (`shadow`: a pass of the shadow pass, whose colortex names are shadowcolor buffers, all declared already).
    fn declare_pass(&self, pass: &Pass, shadow: bool, out: &mut Declared) {
        let Some(t) = &self.targets else { return };
        let push =
            |out: &mut Declared, v: &wgpu::TextureView| out.push((v.texture().clone(), None));
        let opt = |out: &mut Declared, v: Option<&wgpu::TextureView>| {
            if let Some(v) = v {
                out.push((v.texture().clone(), None));
            }
        };
        let image = |out: &mut Declared, k: usize| {
            if let Some(image) = self.images.get(k) {
                opt(out, image.view.as_ref());
                opt(out, image.copy.as_ref());
                opt(out, image.before.as_ref());
            }
        };
        for s in &pass.samplers {
            match s.source {
                Source::Color(i) if !shadow => {
                    self.main_side(i, out);
                    opt(out, t.scratch[i].as_ref());
                }
                Source::Depth | Source::Depth1 | Source::Depth2 => {
                    push(out, &t.depth);
                    opt(out, t.depth1.as_ref().map(|d| &d.1));
                    opt(out, t.depth2.as_ref().map(|d| &d.1));
                }
                Source::WaterMask => opt(out, t.water_mask.as_ref().map(|d| &d.1)),
                Source::ModDepth(i) | Source::ModOpaqueDepth(i) => {
                    push(out, &t.depth);
                    opt(
                        out,
                        t.mod_depth.get(i).and_then(|d| d.as_ref()).map(|d| &d.1),
                    );
                    opt(
                        out,
                        t.mod_opaque_depth
                            .get(i)
                            .and_then(|d| d.as_ref())
                            .map(|d| &d.1),
                    );
                }
                Source::Image(k) => image(out, k),
                Source::ShadowColor(n) => self.shadow_color(n, out),
                Source::ShadowDepth(..) => {
                    push(out, &self.shadow.0);
                    if let Some(s) = &self.shadow_targets {
                        out.push((s.depth.0.clone(), None));
                        opt(out, s.depth1.as_ref().map(|d| &d.1));
                    }
                }
                Source::Shadow => push(out, &self.shadow.0),
                // The pack's own textures, the atlas, noise and stand-in samplers: never written in a frame.
                Source::Noise
                | Source::Dummy
                | Source::Normals
                | Source::Specular
                | Source::Custom { .. }
                | Source::Atlas
                | Source::Lightmap
                | Source::Color(_) => {}
            }
        }
        for &(_, i) in &pass.images {
            match i {
                Image::Color(b) if !shadow => self.main_side(b, out),
                Image::ColorBefore(b) => opt(out, t.scratch[b].as_ref()),
                Image::Custom(k) | Image::CustomBefore(k) => image(out, k),
                Image::StandIn(..) => {
                    for s in &self.stand_ins {
                        push(out, &s.view);
                    }
                }
                Image::ShadowColor(n) | Image::ShadowColorBefore(n) => self.shadow_color(n, out),
                // A pass of the shadow pass: its colortex names are shadowcolor buffers, declared whole.
                Image::Color(_) => {}
            }
        }
        for k in
            (0..self.images.len()).filter(|k| (pass.image_copies | pass.snapshots) & 1 << k != 0)
        {
            image(out, k);
        }
        for n in (0..SHADOW_COLORS).filter(|n| pass.shadow_copies & 1 << n != 0) {
            self.shadow_color(n, out);
        }
        for i in (0..BUFFERS).filter(|i| pass.copies & 1 << i != 0 && !shadow) {
            self.main_side(i, out);
            opt(out, t.scratch[i].as_ref());
        }
        if !shadow {
            for &(_, b) in &pass.writes {
                self.main_side(b, out);
            }
            push(out, &t.depth);
            opt(out, t.water_mask.as_ref().map(|d| &d.1));
        }
        for c in &pass.compute {
            self.declare_pass(c, shadow, out);
        }
    }

    /// The mod passes of `slot` that draw this frame: each one's pass, and whether one binds groups of a mod's own
    /// (which may reach any colortex side).
    fn declare_mod_slot(&self, slot: ModPassSlot, out: &mut Declared) -> bool {
        let mut own = false;
        for m in self.mod_passes.iter().filter(|m| m.pass_slot == slot) {
            let chunks: Vec<_> = self
                .mod_draws
                .iter()
                .filter(|d| d.slot == m.slot && !d.is_empty())
                .collect();
            if chunks.is_empty() {
                continue;
            }
            self.declare_pass(&m.pass, slot == ModPassSlot::Shadow, out);
            if let Some(i) = m.depth_target
                && let Some((t, _)) = self
                    .targets
                    .as_ref()
                    .and_then(|t| t.mod_depth.get(i))
                    .and_then(Option::as_ref)
            {
                out.push((t.clone(), None));
            }
            own |= chunks
                .iter()
                .any(|c| c.mod_bind_group.is_some() || c.mod_globals.is_some());
        }
        own
    }

    /// Block `block`'s declared access set and the textures it may reach unresolved.
    pub fn declared(
        &self,
        block: u32,
        boundaries: WorldBoundaries,
    ) -> (Declared, Vec<wgpu::Texture>) {
        let mut out = Declared::new();
        let mut unresolved = Vec::new();
        let entity_passes = |late: bool| {
            self.entities.iter().flat_map(move |e| {
                e.passes
                    .iter()
                    .zip(e.late)
                    .filter(move |(_, l)| *l == late)
                    .filter_map(|(p, _)| p.as_ref())
            })
        };
        // A callback handed colortex0's side and depth.
        let target_callback = |out: &mut Declared| {
            self.main_side(0, out);
            if let Some(t) = &self.targets {
                out.push((t.depth_texture.clone(), None));
            }
        };
        match block {
            SHADOW => {
                if let Some(s) = &self.shadow_targets {
                    for v in s
                        .color
                        .iter()
                        .flatten()
                        .flatten()
                        .chain(s.scratch.iter().flatten())
                    {
                        out.push((v.texture().clone(), None));
                    }
                    out.push((s.depth.0.clone(), None));
                    if let Some((t, _)) = &s.depth1 {
                        out.push((t.clone(), None));
                    }
                    if let Some(v) = &s.sink {
                        out.push((v.texture().clone(), None));
                    }
                }
                let shadow_entities = self.entities.iter().flat_map(|e| e.shadow.iter().flatten());
                for pass in self
                    .shadow_terrain
                    .iter()
                    .chain(&self.shadow_water)
                    .chain(shadow_entities)
                    .chain(&self.shadow_comp)
                {
                    self.declare_pass(pass, true, &mut out);
                }
                if self.declare_mod_slot(ModPassSlot::Shadow, &mut out) {
                    unresolved = self.every_side();
                }
            }
            OPAQUE => {
                let sky = self
                    .sky
                    .iter()
                    .flat_map(|s| s.basic.iter().chain(&s.textured));
                for pass in sky
                    .chain(&self.terrain)
                    .chain(&self.terrain_cutout)
                    .chain(entity_passes(false))
                    .chain(&self.lines)
                    .chain(&self.clouds)
                    .chain(self.hand.iter().map(|h| &h.0))
                {
                    self.declare_pass(pass, false, &mut out);
                }
                if let Some(t) = &self.targets {
                    // The depth snapshots this block copies: their source and their copies.
                    for d in [&t.depth1, &t.depth2].into_iter().flatten() {
                        out.push((t.depth_texture.clone(), None));
                        out.push((d.0.clone(), None));
                    }
                    for d in t.mod_depth.iter().chain(&t.mod_opaque_depth).flatten() {
                        out.push((d.0.clone(), None));
                    }
                }
                let mut own = self.declare_mod_slot(ModPassSlot::AfterSky, &mut out);
                own |= self.declare_mod_slot(ModPassSlot::AfterOpaqueGbuffers, &mut out);
                if boundaries.opaque_blocks || boundaries.features {
                    target_callback(&mut out);
                }
                if own || boundaries.sky {
                    unresolved = self.every_side();
                }
            }
            TRANSLUCENT => {
                for pass in self
                    .water
                    .iter()
                    .chain(entity_passes(true))
                    .chain(&self.hand_water)
                    .chain(self.weather.iter().flatten())
                {
                    self.declare_pass(pass, false, &mut out);
                }
                let mut own =
                    self.declare_mod_slot(ModPassSlot::AfterTranslucentGbuffers, &mut out);
                own |= self.declare_mod_slot(ModPassSlot::AfterWeather, &mut out);
                if boundaries.translucent_blocks
                    || boundaries.weather
                    || boundaries.custom_geometry
                    || boundaries.features
                {
                    target_callback(&mut out);
                }
                if own {
                    unresolved = self.every_side();
                }
            }
            _ => unresolved = self.every_side(),
        }
        (out, unresolved)
    }

    /// Records the render passes `ShadowTargets::clear` encodes for `buffers` (bit n: shadowcolor n) and, with
    /// `depth`, the shadow map: one clear unit each, none when it encodes none.
    pub fn trace_shadow_clears(&self, r: &mut Recorder, buffers: u8, depth: bool) {
        let Some(s) = &self.shadow_targets else {
            return;
        };
        for (colour, with_depth) in s.clear_passes(buffers, depth) {
            let what = "shadow clear";
            let mut touches = Vec::new();
            if let Some((view, c)) = colour {
                touches.push(r.touch(view, 0, Use::Clear(colour4(c)), what));
            }
            if with_depth {
                touches.push(r.touch(&s.depth.1, DEPTH_SLOT, Use::Clear(self::depth(1.0)), what));
            }
            r.run(what, EventKind::Clear, false, false, touches);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Gpu {
        device: wgpu::Device,
        layout: wgpu::BindGroupLayout,
    }

    impl Gpu {
        fn new() -> Option<Self> {
            let adapter = crate::test_gpu::adapter()?;
            let (device, _queue) = crate::test_gpu::device(&adapter);
            let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: None,
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                }],
            });
            Some(Self { device, layout })
        }

        fn texture(&self) -> wgpu::TextureView {
            self.device
                .create_texture(&wgpu::TextureDescriptor {
                    label: None,
                    size: wgpu::Extent3d {
                        width: 4,
                        height: 4,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    usage: wgpu::TextureUsages::TEXTURE_BINDING
                        | wgpu::TextureUsages::RENDER_ATTACHMENT,
                    view_formats: &[],
                })
                .create_view(&Default::default())
        }

        /// A group sampling `view` at binding 0, as the recorder is told it was made.
        fn group(&self, r: &mut Recorder, view: &wgpu::TextureView) -> wgpu::BindGroup {
            let group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: None,
                layout: &self.layout,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(view),
                }],
            });
            r.group_made(&group, [(0, view, false)]);
            group
        }
    }

    fn recorder() -> (Recorder, PathBuf) {
        let dir = std::env::temp_dir().join(format!(
            "plan-trace-unit-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        (Recorder::new(dir.clone(), 8), dir)
    }

    /// The frame `end` wrote, read back.
    fn written(dir: &std::path::Path, seq: u64) -> Recorded {
        let text = std::fs::read_to_string(dir.join(format!("frame-{seq:08}.trace"))).unwrap();
        Recorded::parse(&text).unwrap()
    }

    /// Each unit's label and the textures (by label) it sampled.
    fn sampled(frame: &Recorded) -> Vec<(String, Vec<String>)> {
        let label = |id| {
            frame
                .textures
                .iter()
                .find(|n| n.id == id)
                .map(|n| n.label.clone())
                .unwrap()
        };
        frame
            .frame
            .ops
            .iter()
            .filter_map(|op| match op {
                Op::Run { work, touches, .. } => Some((
                    frame
                        .works
                        .iter()
                        .find(|(w, _)| w == work)
                        .unwrap()
                        .1
                        .clone(),
                    touches
                        .iter()
                        .filter(|t| t.how == Use::Sample)
                        .map(|t| label(t.texture))
                        .collect(),
                )),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_bind_records_its_textures_on_the_pass_it_is_bound_in() {
        let _gpu = crate::test_gpu::lock();
        let Some(gpu) = Gpu::new() else { return };
        let (mut r, dir) = recorder();
        let (a, b, target) = (gpu.texture(), gpu.texture(), gpu.texture());
        r.begin([4, 4], false);
        for (view, label) in [(&a, "a"), (&b, "b"), (&target, "target")] {
            r.texture(view.texture(), label, false, None);
        }
        let ga = gpu.group(&mut r, &a);
        let gb = gpu.group(&mut r, &b);
        // Both groups made before any pass: neither is a use until a pass binds it.
        let keep = |r: &mut Recorder, what| {
            let attached = r.touch(&target, 0, Use::Attach(Load::Keep), what);
            r.run(what, EventKind::Work, false, false, [attached]);
        };
        keep(&mut r, "first");
        r.bound(&ga, "first");
        // A switch inside the pass: the pass samples both.
        r.bound(&gb, "first");
        keep(&mut r, "second");
        // The first group again, in a second pass.
        r.bound(&ga, "second");
        keep(&mut r, "third");
        r.end();
        let frame = written(&dir, 1);
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(frame.errors, Vec::<String>::new());
        assert_eq!(
            sampled(&frame),
            [
                ("first".to_owned(), vec!["a".to_owned(), "b".to_owned()]),
                ("second".to_owned(), vec!["a".to_owned()]),
                ("third".to_owned(), vec![]),
            ]
        );
    }

    #[test]
    fn a_bind_outside_a_pass_or_of_an_unknown_group_is_an_error() {
        let _gpu = crate::test_gpu::lock();
        let Some(gpu) = Gpu::new() else { return };
        let (mut r, dir) = recorder();
        let a = gpu.texture();
        r.begin([4, 4], false);
        r.texture(a.texture(), "a", false, None);
        let ga = gpu.group(&mut r, &a);
        r.bound(&ga, "before any pass");
        r.forget_group(&ga);
        r.run("pass", EventKind::Work, false, false, []);
        r.bound(&ga, "forgotten");
        r.end();
        let frame = written(&dir, 1);
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(
            frame.errors,
            [
                "before any pass: a bind group bound outside a pass",
                "forgotten: a bind group the trace didn't see made"
            ]
        );
    }

    #[test]
    fn the_recorder_keeps_no_texture_and_numbers_new_ones_anew() {
        let _gpu = crate::test_gpu::lock();
        let Some(gpu) = Gpu::new() else { return };
        let (mut r, dir) = recorder();
        let old = gpu.texture();
        r.begin([4, 4], false);
        r.texture(old.texture(), "old", false, None);
        let group = gpu.group(&mut r, &old);
        r.end();
        // The recorder holds numbers, not handles (its fields hold no wgpu type); what it keeps of a group goes
        // when the runtime drops the group.
        assert_eq!(r.groups_known(), 1);
        drop((old, group));
        r.forget_groups();
        assert_eq!(r.groups_known(), 0);
        let new = gpu.texture();
        r.begin([4, 4], false);
        r.texture(new.texture(), "new", false, None);
        r.end();
        let (first, second) = (written(&dir, 1), written(&dir, 2));
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(first.frame.textures[0].id, TextureId(0));
        assert_eq!(second.frame.textures[0].id, TextureId(1));
    }
}
