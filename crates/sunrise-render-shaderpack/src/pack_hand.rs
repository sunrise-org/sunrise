//! The first-person hand and held items drawn by a shader pack's `gbuffers_hand` (spec 2.3):
//! `Frame::hand`'s entity vertices, read as entities are (`pack_entities::inputs`), with the
//! hand's own projection as `gl_ProjectionMatrix` (5.2: `gbufferProjection` stays the world's).
//! A `render_hand` mod's native terrain quads (its fluids and terrain models, drawn natively from
//! the terrain's sprite arrays) take the pack's block atlas, as an entity's do
//! (`pack_entities::BlockAtlas`, decision D-4).

use glam::Mat4;
use sunrise_render_core::{
    EntityVertex as Vertex, TRANSLUCENT_GLINT_ITEM_OVERLAY, TRANSLUCENT_ITEM_OVERLAY,
    entity::QuadSort, is_native_terrain,
};
use wgpu::util::DeviceExt;

use crate::pack_entities::{BlockAtlas, native_translucent};

/// `MC_HAND_DEPTH`'s value (1.5): the hand's depth is scaled by it through its projection, so
/// that it sits in front of the world in the shared depth buffer.
const HAND_DEPTH: f32 = 0.125;

/// `HzHand`: the hand's projection and its inverse.
const UNIFORM: u64 = 2 * 64;

/// `HzDraw`'s size (pack_draw.rs).
const SLOT: u64 = 32;

/// Adapts entity inputs for `gbuffers_hand`: overrides velocity attribute, enables item_id,
/// and binds `HzHand` projection uniform at set 1, binding 1.
pub fn adapt_hand_inputs(
    mut inputs: sunrise_shaders::GbuffersInputs,
) -> sunrise_shaders::GbuffersInputs {
    // No last frame's hand to measure from: the hand's `at_velocity` reads zero (and its set 1
    // has no previous vertices).
    for (name, value) in &mut inputs.attributes {
        if name == "at_velocity" {
            *value = "vec3(0.0)".to_owned();
        }
    }
    // Iris 1.11.7 declares `currentRenderedItemId` in the hand's programs (observed).
    inputs.item_id_declared = true;
    inputs.declarations.push_str(
        "layout(set = 1, binding = 1) uniform HzHand { mat4 hz_hand_projection; mat4 hz_hand_projection_inverse; };\n",
    );
    inputs.body.push_str(
        "    hz_ProjectionMatrix = hz_hand_projection;\n    \
         hz_ProjectionMatrixInverse = hz_hand_projection_inverse;\n    \
         hz_ModelViewProjectionMatrix = hz_hand_projection * hz_ModelViewMatrix;\n",
    );
    inputs
}

/// The hand's projection in GL's conventions: `hud_fov` degrees, depth scaled by
/// `MC_HAND_DEPTH` (the view bob is in `gbufferModelView`).
pub fn projection(hud_fov: f32, aspect: f32, far: f32) -> Mat4 {
    Mat4::from_scale(glam::Vec3::new(1.0, 1.0, HAND_DEPTH))
        * Mat4::perspective_rh_gl(hud_fov.to_radians(), aspect, 0.05, far)
}

/// Two triangles for each of `quads` quads, from vertex 0.
pub fn quad_indices(device: &wgpu::Device, quads: u32) -> wgpu::Buffer {
    let indices: Vec<u32> = (0..quads)
        .flat_map(|q| [0, 1, 2, 0, 2, 3].map(|i| q * 4 + i))
        .collect();
    device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("Shader pack hand indices"),
        contents: bytemuck::cast_slice(&indices),
        usage: wgpu::BufferUsages::INDEX,
    })
}

/// The hand's vertices (also read at set 1 for normals and tangents), its uniform and quad
/// indices.
pub struct PackHand {
    pub layout: wgpu::BindGroupLayout,
    vertices: wgpu::Buffer,
    uniform: wgpu::Buffer,
    pub group: wgpu::BindGroup,
    pub indices: wgpu::Buffer,
    capacity: u32,
    /// Quads uploaded this frame.
    pub quads: u32,
    /// `HzDraw` (set 2) of each run: the arm or one held item, and its `currentRenderedItemId`.
    slots: wgpu::Buffer,
    pub slot_group: wgpu::BindGroup,
    /// The layout of `HzDraw`, to bind a larger `slots` with.
    draws: wgpu::BindGroupLayout,
    /// `slots`' capacity, in slots.
    slot_capacity: u64,
    stride: u64,
    /// This frame's runs: first quad, quads, the dynamic offset of their slot, and whether they
    /// are native terrain quads placed in the block atlas, drawn with it bound as the atlas
    /// (`PackPost::block_atlas_group`). The opaque quads come first in the buffer, then the
    /// translucent ones (`translucent_runs`), which a pack's `gbuffers_hand_water` draws after
    /// deferred.
    pub runs: Vec<(u32, u32, u32, bool)>,
    pub translucent_runs: Vec<(u32, u32, u32, bool)>,
    order: Vec<Vertex>,
    /// Where the pack's block atlas holds each terrain sprite (`pack_entities::BlockAtlas`):
    /// none leaves no native terrain quad to the pack.
    block_atlas: Option<BlockAtlas>,
    /// Sorts translucent native terrain quads, kept between frames.
    sort: QuadSort,
}

impl PackHand {
    /// `draws`: the layout of `HzDraw` (pack_draw.rs).
    pub fn new(device: &wgpu::Device, draws: &wgpu::BindGroupLayout) -> Self {
        let entry = |binding, ty| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::VERTEX,
            ty: wgpu::BindingType::Buffer {
                ty,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Shader pack hand"),
            entries: &[
                entry(0, wgpu::BufferBindingType::Storage { read_only: true }),
                entry(1, wgpu::BufferBindingType::Uniform),
            ],
        });
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Shader pack hand"),
            size: UNIFORM,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let capacity = 256;
        let (vertices, group) = Self::vertex_buffer(device, &layout, &uniform, capacity);
        let stride = SLOT.max(u64::from(
            device.limits().min_uniform_buffer_offset_alignment,
        ));
        // The arm and the two hands' items, opaque and translucent: a few slots a frame, more
        // when a frame needs them (`prepare`).
        let slot_capacity = 8;
        let (slots, slot_group) = Self::slot_buffer(device, draws, slot_capacity, stride);
        Self {
            indices: quad_indices(device, capacity),
            layout,
            vertices,
            uniform,
            group,
            capacity,
            quads: 0,
            slots,
            slot_group,
            draws: draws.clone(),
            slot_capacity,
            stride,
            runs: Vec::new(),
            translucent_runs: Vec::new(),
            order: Vec::new(),
            block_atlas: None,
            sort: QuadSort::default(),
        }
    }

    /// The block atlas the pack's terrain samples, set with it ([`BlockAtlas`]).
    pub fn set_block_atlas(&mut self, atlas: Option<BlockAtlas>) {
        self.block_atlas = atlas;
    }

    /// `HzDraw` slots for `count` draws of `stride` bytes each, and their group.
    fn slot_buffer(
        device: &wgpu::Device,
        draws: &wgpu::BindGroupLayout,
        count: u64,
        stride: u64,
    ) -> (wgpu::Buffer, wgpu::BindGroup) {
        let slots = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Shader pack hand draws"),
            size: count * stride,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Shader pack hand draws"),
            layout: draws,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer: &slots,
                    offset: 0,
                    size: wgpu::BufferSize::new(SLOT),
                }),
            }],
        });
        (slots, group)
    }

    fn vertex_buffer(
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        uniform: &wgpu::Buffer,
        quads: u32,
    ) -> (wgpu::Buffer, wgpu::BindGroup) {
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Shader pack hand vertices"),
            size: u64::from(quads) * 4 * std::mem::size_of::<Vertex>() as u64,
            usage: wgpu::BufferUsages::VERTEX
                | wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Shader pack hand"),
            layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: uniform.as_entire_binding(),
                },
            ],
        });
        (buffer, group)
    }

    /// Uploads this frame's hand, its projection, and the `HzDraw` of each run `items` starts
    /// (first vertex, `currentRenderedItemId`): one slot for each `currentRenderedItemId` and
    /// translucency the runs draw with ([`slots`]), however many runs the atlas splits them into.
    pub fn prepare(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        hand: &[Vertex],
        items: &[(u32, i32)],
        projection: Mat4,
    ) {
        self.quads = (hand.len() / 4) as u32;
        self.runs.clear();
        self.translucent_runs.clear();
        if self.quads == 0 {
            return;
        }
        let split = split(hand, items, self.block_atlas.as_ref(), &mut self.sort);
        self.order = split.order;
        self.quads = (self.order.len() / 4) as u32;
        if self.quads == 0 {
            return;
        }
        let (pairs, of_run) = slots(&split.runs);
        let mut contents = Vec::with_capacity(pairs.len() * self.stride as usize);
        for &(id, translucent) in &pairs {
            contents.extend_from_slice(&draw_slot(id, translucent));
            contents.resize(contents.len() + (self.stride - SLOT) as usize, 0);
        }
        if pairs.len() as u64 > self.slot_capacity {
            self.slot_capacity = (pairs.len() as u64).next_power_of_two();
            (self.slots, self.slot_group) =
                Self::slot_buffer(device, &self.draws, self.slot_capacity, self.stride);
        }
        for (run, &slot) in split.runs.iter().zip(&of_run) {
            let translucent = run.translucent;
            let run = (
                run.first,
                run.quads,
                (slot as u64 * self.stride) as u32,
                run.block_atlas,
            );
            if translucent {
                self.translucent_runs.push(run);
            } else {
                self.runs.push(run);
            }
        }
        queue.write_buffer(&self.slots, 0, &contents);
        if self.quads > self.capacity {
            self.capacity = self.quads.next_power_of_two();
            (self.vertices, self.group) =
                Self::vertex_buffer(device, &self.layout, &self.uniform, self.capacity);
            self.indices = quad_indices(device, self.capacity);
        }
        queue.write_buffer(&self.vertices, 0, bytemuck::cast_slice(&self.order));
        let matrices = [projection, projection.inverse()].map(|m| m.to_cols_array());
        queue.write_buffer(&self.uniform, 0, bytemuck::cast_slice(&matrices));
    }

    pub fn vertices(&self) -> wgpu::BufferSlice<'_> {
        self.vertices.slice(..)
    }
}

/// The `HzDraw` slots `runs` draw with: each distinct (`currentRenderedItemId`, translucent)
/// pair once, in the order the runs first use it, and each run's slot among them. A run split
/// off by an atlas switch shares its neighbours' slot.
fn slots(runs: &[Run]) -> (Vec<(i32, bool)>, Vec<usize>) {
    let mut pairs: Vec<(i32, bool)> = Vec::new();
    let of_run = runs
        .iter()
        .map(|r| {
            let pair = (r.id, r.translucent);
            pairs.iter().position(|&p| p == pair).unwrap_or_else(|| {
                pairs.push(pair);
                pairs.len() - 1
            })
        })
        .collect();
    (pairs, of_run)
}

/// The hand's quads reordered, opaque first and translucent after, and their runs.
struct Split {
    order: Vec<Vertex>,
    runs: Vec<Run>,
}

/// A run of the reordered hand: one draw.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Run {
    first: u32,
    quads: u32,
    /// `currentRenderedItemId`.
    id: i32,
    translucent: bool,
    /// Native terrain quads placed in the block atlas.
    block_atlas: bool,
}

/// What a hand quad is to the pack's draw (`split`).
#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    /// Shared-atlas geometry: the arm, a held item.
    Plain { translucent: bool },
    /// A native terrain quad the block atlas holds (`BlockAtlas::holds`).
    Terrain { translucent: bool },
    /// A native terrain quad the block atlas lacks (a layer past its table, a fluid layer
    /// below 0, NaN or infinite; or no block atlas): its UVs and layer mean nothing in either
    /// atlas, so the pack does not draw it (decision D-4).
    Left,
}

/// The hand's quads as the pack draws them: the opaque ones in their order, then the
/// translucent ones. Native terrain quads the block atlas holds are placed in it
/// (`BlockAtlas::place`), each hand run's translucent ones together after its other
/// translucent quads, far to near (`QuadSort`, as Sunrise's own draw sorts them). Runs split
/// where the `currentRenderedItemId` (`items`), translucency or atlas changes.
fn split(
    hand: &[Vertex],
    items: &[(u32, i32)],
    atlas: Option<&BlockAtlas>,
    sort: &mut QuadSort,
) -> Split {
    let quads = (hand.len() / 4) as u32;
    let quad = |q: usize| &hand[q * 4..q * 4 + 4];
    let kind = |q: usize| {
        let layer = quad(q)[0].layer;
        if !is_native_terrain(layer) {
            Kind::Plain {
                translucent: translucent_quad(quad(q)),
            }
        } else if atlas.is_some_and(|a| a.holds(quad(q))) {
            Kind::Terrain {
                translucent: native_translucent(layer),
            }
        } else {
            Kind::Left
        }
    };
    let mut out = Split {
        order: Vec::with_capacity(hand.len()),
        runs: Vec::new(),
    };
    // (id, translucent, block atlas) of each quad of `out.order`.
    let mut keys = Vec::with_capacity(quads as usize);
    let hand_runs = runs(quads, items);
    for &(start, count, id) in &hand_runs {
        for q in start as usize..(start + count) as usize {
            match kind(q) {
                Kind::Plain { translucent: false } => keys.push((id, false, false)),
                Kind::Terrain { translucent: false } => keys.push((id, false, true)),
                _ => continue,
            }
            out.order.extend_from_slice(quad(q));
        }
    }
    for &(start, count, id) in &hand_runs {
        let range = start as usize..(start + count) as usize;
        for q in range.clone() {
            if kind(q) == (Kind::Plain { translucent: true }) {
                out.order.extend_from_slice(quad(q));
                keys.push((id, true, false));
            }
        }
        let sorted = out.order.len();
        for q in range {
            if kind(q) == (Kind::Terrain { translucent: true }) {
                out.order.extend_from_slice(quad(q));
                keys.push((id, true, true));
            }
        }
        sort.sort(out.order[sorted..].as_chunks_mut::<4>().0);
    }
    if let Some(atlas) = atlas {
        for (i, _) in keys.iter().enumerate().filter(|(_, k)| k.2) {
            atlas.place(&mut out.order[i * 4..i * 4 + 4]);
        }
    }
    let mut i = 0;
    while i < keys.len() {
        let key = keys[i];
        let n = keys[i..].iter().take_while(|&&k| k == key).count();
        out.runs.push(Run {
            first: i as u32,
            quads: n as u32,
            id: key.0,
            translucent: key.1,
            block_atlas: key.2,
        });
        i += n;
    }
    out
}

/// A held item's quad drawn from a translucent sheet (`item_render`: its overlay layer).
fn translucent_quad(quad: &[Vertex]) -> bool {
    matches!(
        quad[0].layer & 0xffff,
        TRANSLUCENT_ITEM_OVERLAY | TRANSLUCENT_GLINT_ITEM_OVERLAY
    )
}

/// `HzDraw` of a hand run: `renderStage` the solid or translucent hand's, `blockEntityId` -1 as
/// outside block entities, and `currentRenderedItemId`.
fn draw_slot(id: i32, translucent: bool) -> [u8; SLOT as usize] {
    let mut bytes = [0u8; SLOT as usize];
    let stage = sunrise_shaders::render_stage(if translucent {
        "HAND_TRANSLUCENT"
    } else {
        "HAND_SOLID"
    }) as i32;
    bytes[0..4].copy_from_slice(&stage.to_le_bytes());
    bytes[8..12].copy_from_slice(&(-1i32).to_le_bytes());
    bytes[12..16].copy_from_slice(&id.to_le_bytes());
    bytes
}

/// The runs of `quads` quads that `items` starts (first vertex, id), as first quad, quads and
/// id; geometry before the first start, or with no starts at all, reads 0, as the arm does.
fn runs(quads: u32, items: &[(u32, i32)]) -> Vec<(u32, u32, i32)> {
    let mut starts: Vec<(u32, i32)> = vec![(0, 0)];
    for &(start, id) in items {
        let q = (start / 4).min(quads);
        match starts.last_mut() {
            Some(last) if last.0 == q => last.1 = id,
            _ => starts.push((q, id)),
        }
    }
    (0..starts.len())
        .map(|i| {
            let end = starts.get(i + 1).map_or(quads, |s| s.0);
            (starts[i].0, end - starts[i].0, starts[i].1)
        })
        .filter(|r| r.1 > 0)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The arm, then an item whose middle quad is translucent, then a second item: the opaque
    /// quads keep their order first, the translucent one comes after them, and each run keeps
    /// its item's id.
    #[test]
    fn translucent_quads_go_after_the_opaque_ones() {
        let quad = |layer: u32, tag: f32| {
            [Vertex {
                layer,
                pos: [tag, 0.0, 0.0],
                ..bytemuck::Zeroable::zeroed()
            }; 4]
        };
        let glass = TRANSLUCENT_ITEM_OVERLAY;
        let hand: Vec<Vertex> = [
            quad(0, 0.),
            quad(0, 1.),
            quad(glass, 2.),
            quad(0, 3.),
            quad(0, 4.),
        ]
        .concat();
        // The arm is quad 0; item 7 quads 1..4; item 9 quad 4.
        let split = split(&hand, &[(4, 7), (16, 9)], None, &mut QuadSort::default());
        let order: Vec<f32> = split.order.chunks(4).map(|q| q[0].pos[0]).collect();
        assert_eq!(order, [0., 1., 3., 4., 2.]);
        assert_eq!(
            split.runs.iter().map(run).collect::<Vec<_>>(),
            [
                (0, 1, 0, false, false),
                (1, 2, 7, false, false),
                (3, 1, 9, false, false),
                (4, 1, 7, true, false)
            ]
        );
    }

    fn run(r: &Run) -> (u32, u32, i32, bool, bool) {
        (r.first, r.quads, r.id, r.translucent, r.block_atlas)
    }

    /// A native terrain quad of `overlay` on sprite layer `sprite` (a fluid's in `glint_uv.x`, a
    /// model's in the layer's high bits), UVs at the middle of its sprite, `tag` blocks ahead.
    fn native(overlay: u32, sprite: f32, tag: f32) -> [Vertex; 4] {
        let fluid = overlay == sunrise_render_core::FLUID_OVERLAY;
        [Vertex {
            layer: if fluid {
                overlay
            } else {
                overlay | (sprite as u32) << 16
            },
            uv: [0.5, 0.5],
            glint_uv: [if fluid { sprite } else { 0.0 }, 0.0],
            pos: [0.0, 0.0, tag],
            ..bytemuck::Zeroable::zeroed()
        }; 4]
    }

    /// Layer 0 at (0, 0) and layer 1 at (16, 0) of a 32 by 16 atlas, 16 pixels a side.
    fn atlas() -> BlockAtlas {
        BlockAtlas::new(&[0, 16, 16, 16], [32, 16]).unwrap()
    }

    /// A `render_hand` mod's native terrain quads beside the arm and a held item: the opaque
    /// kinds go with the opaque quads in their order, the translucent kinds after the run's
    /// other translucent quads; each is placed in the block atlas (its sprite's cell, overlay
    /// and layer cleared) and its own run says so; one whose sprite the atlas lacks is not drawn.
    #[test]
    fn native_terrain_quads_take_the_block_atlas() {
        use sunrise_render_core::{
            FLUID_OVERLAY, TERRAIN_MODEL_CUTOUT_MIPPED_OVERLAY, TERRAIN_MODEL_CUTOUT_OVERLAY,
            TERRAIN_MODEL_SOLID_OVERLAY, TERRAIN_MODEL_TRANSLUCENT_OVERLAY,
        };
        let plain = |layer: u32, tag: f32| {
            [Vertex {
                layer,
                pos: [0.0, 0.0, tag],
                uv: [0.25, 0.75],
                ..bytemuck::Zeroable::zeroed()
            }; 4]
        };
        let hand: Vec<Vertex> = [
            plain(0, 1.),
            native(TERRAIN_MODEL_SOLID_OVERLAY, 1.0, 2.),
            native(FLUID_OVERLAY, 0.0, 3.),
            plain(TRANSLUCENT_ITEM_OVERLAY, 4.),
            native(TERRAIN_MODEL_CUTOUT_OVERLAY, 0.0, 5.),
            // Past the atlas's table, and a fluid layer that is no number: not drawn.
            native(TERRAIN_MODEL_SOLID_OVERLAY, 2.0, 6.),
            native(FLUID_OVERLAY, f32::NAN, 7.),
            native(TERRAIN_MODEL_CUTOUT_MIPPED_OVERLAY, 1.0, 8.),
            native(TERRAIN_MODEL_TRANSLUCENT_OVERLAY, 1.0, 9.),
            plain(0, 10.),
        ]
        .concat();
        // The mod's run is quads 0..9 (id 0); item 33 holds quad 9.
        let split = split(&hand, &[(36, 33)], Some(&atlas()), &mut QuadSort::default());
        let order: Vec<f32> = split.order.chunks(4).map(|q| q[0].pos[2]).collect();
        // Opaque in order (1, 2, 5, 8, 10), then the run's translucent item quad (4), then its
        // native translucent ones far to near (9, 3).
        assert_eq!(order, [1., 2., 5., 8., 10., 4., 9., 3.]);
        assert_eq!(
            split.runs.iter().map(run).collect::<Vec<_>>(),
            [
                (0, 1, 0, false, false),
                (1, 3, 0, false, true),
                (4, 1, 33, false, false),
                (5, 1, 0, true, false),
                (6, 2, 0, true, true),
            ]
        );
        let uvs: Vec<[f32; 2]> = split.order.chunks(4).map(|q| q[0].uv).collect();
        // Layer 0's cell spans u 0..0.5, layer 1's 0.5..1; v 0..1. Plain quads keep theirs.
        assert_eq!(
            uvs,
            [
                [0.25, 0.75],
                [0.75, 0.5],
                [0.25, 0.5],
                [0.75, 0.5],
                [0.25, 0.75],
                [0.25, 0.75],
                [0.75, 0.5],
                [0.25, 0.5],
            ]
        );
        let layers: Vec<u32> = split.order.chunks(4).map(|q| q[0].layer).collect();
        assert_eq!(layers, [0, 0, 0, 0, 0, TRANSLUCENT_ITEM_OVERLAY, 0, 0]);
    }

    /// A mod's hand run whose quads switch atlas more than eight times, then a held item: the
    /// item's runs still draw with its own `currentRenderedItemId`, and every run of the mod's
    /// quads with the arm's (0), each pair in one slot.
    #[test]
    fn atlas_switches_share_their_runs_slot() {
        let plain = [Vertex {
            pos: [0.0, 0.0, 1.0],
            ..bytemuck::Zeroable::zeroed()
        }; 4];
        let terrain = native(sunrise_render_core::TERRAIN_MODEL_SOLID_OVERLAY, 0.0, 1.);
        let glass = [Vertex {
            layer: TRANSLUCENT_ITEM_OVERLAY,
            ..plain[0]
        }; 4];
        // Quads 0..10 the mod's (plain, terrain, plain, ...: ten runs), 10 and 11 item 33's.
        let mut hand: Vec<Vertex> = (0..10)
            .flat_map(|q| if q % 2 == 0 { plain } else { terrain })
            .collect();
        hand.extend_from_slice(&plain);
        hand.extend_from_slice(&glass);
        let split = split(&hand, &[(40, 33)], Some(&atlas()), &mut QuadSort::default());
        assert_eq!(split.runs.len(), 12);
        let (pairs, of_run) = slots(&split.runs);
        assert_eq!(pairs, [(0, false), (33, false), (33, true)]);
        let drawn: Vec<(i32, bool)> = of_run.iter().map(|&s| pairs[s]).collect();
        let expected: Vec<(i32, bool)> = split.runs.iter().map(|r| (r.id, r.translucent)).collect();
        assert_eq!(drawn, expected);
        assert_eq!(&drawn[10..], [(33, false), (33, true)]);
    }

    /// Without a block atlas the pack draws no native terrain quad (their UVs mean nothing in
    /// the shared atlas); the hand's other quads are as before.
    #[test]
    fn without_a_block_atlas_native_terrain_quads_are_not_drawn() {
        let arm = [Vertex {
            pos: [0.0, 0.0, 1.0],
            ..bytemuck::Zeroable::zeroed()
        }; 4];
        let hand: Vec<Vertex> = [
            arm,
            native(sunrise_render_core::TERRAIN_MODEL_SOLID_OVERLAY, 0.0, 2.),
            native(sunrise_render_core::FLUID_OVERLAY, 0.0, 3.),
        ]
        .concat();
        let split = split(&hand, &[], None, &mut QuadSort::default());
        assert_eq!(
            bytemuck::cast_slice::<_, u8>(&split.order),
            bytemuck::cast_slice::<_, u8>(&arm)
        );
        assert_eq!(
            split.runs.iter().map(run).collect::<Vec<_>>(),
            [(0, 1, 0, false, false)]
        );
    }

    /// Each item's quads draw with its id; geometry before the first item is the arm's (0).
    #[test]
    fn runs_split_the_hand_by_item() {
        assert_eq!(runs(3, &[]), [(0, 3, 0)]);
        assert_eq!(runs(5, &[(0, 11), (8, 65535)]), [(0, 2, 11), (2, 3, 65535)]);
        assert_eq!(runs(4, &[(4, 21)]), [(0, 1, 0), (1, 3, 21)]);
        // An item that drew nothing gives way to the next.
        assert_eq!(runs(2, &[(0, 11), (0, 22)]), [(0, 2, 22)]);
    }

    /// The hand's window depth lies within `MC_HAND_DEPTH` of the middle (packs tell the hand
    /// apart by it), and x, y match the plain projection.
    #[test]
    fn hand_depth_is_scaled_around_the_middle() {
        let p = projection(70.0, 16.0 / 9.0, 512.0);
        let plain = Mat4::perspective_rh_gl(70f32.to_radians(), 16.0 / 9.0, 0.05, 512.0);
        for z in [-0.06f32, -0.5, -3.0, -400.0] {
            let v = glam::Vec4::new(0.1, -0.2, z, 1.0);
            let (a, b) = (p * v, plain * v);
            let window = (a.z / a.w) * 0.5 + 0.5;
            assert!(
                (window - 0.5).abs() <= HAND_DEPTH * 0.5 + 1e-6,
                "{z}: {window}"
            );
            assert!((a.x / a.w - b.x / b.w).abs() < 1e-6);
        }
    }
}
