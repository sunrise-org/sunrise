//! Mods' render pipelines on the GPU (`Hooks::register_render_pipelines`): the pipelines built
//! from a resource load's translated GLSL core shaders (`crate::mod_pipelines`), and the quads
//! a frame draws with them. They draw with the world primitives, at their stages and into the
//! same targets, so they take the world's reversed depth, its draw resolution and TAA's jitter.
//! Pipelines are built with the renderer a load is applied with
//! ([`super::Renderer::build_mod_pipelines`]), never while quads are being submitted, so a
//! required pipeline the GPU refuses fails that load's renderer and the reload with it.
//!
//! Each pipeline binds vanilla's four std140 uniform blocks (`Globals` 0, `Projection` 1,
//! `DynamicTransforms` 2, `Fog` 3), plus `Sampler0` (4/5) and `SpriteRect` (10) when
//! `.textured(PipelineTexture::Sprite(_))` is declared, only for the bindings its translated
//! stages actually read.

use super::expression_blocks::{VANILLA_OFFSETS, VANILLA_SIZES, block_entry};
use super::mod_meshes::{ResolvedMeshDraw, wgpu_index_format};
use super::mod_render_targets::{TargetMips, TargetUse};
use super::world_primitives::{PreparedPass, Upload, VertexPages};
use crate::mod_pipelines::{
    Built, BuiltTexture, ModifierStacks, Pipelines, Refusal, wgpu_compare, wgpu_depth_bias,
    wgpu_stencil_state,
};
use glam::Mat4;
use mod_api::{
    BindingKind, BlendFactor, BlendOp, ColorTargetState, ColorWrite, DrawRange, Identifier,
    IndexFormat, MeshDraw, PipelineBlend, PipelineDrawError, PipelineState, PolygonMode,
    PrimitiveTopology, Sampler, SamplerAddress, SamplerFilter, VertexAttributeFormat, VertexFormat,
    VertexStepMode,
};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use sunrise_mod_pipeline::{
    ENTRY, OIT_STAGE_SAMPLERS, Options as PipelineOptions, SPRITE_RECT_BINDING,
};

/// One corner of a pipeline quad, camera-relative.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct PipelineVertex {
    pub pos: [f32; 3],
    pub uv: [f32; 2],
    /// Red in the low byte, then green, blue and alpha, as vanilla encodes them.
    pub color: u32,
}

impl PipelineVertex {
    /// Vertex attributes matching `PipelineVertex` (`pos`, `uv`, `color`).
    pub const ATTRIBUTES: [wgpu::VertexAttribute; 3] =
        wgpu::vertex_attr_array![0=>Float32x3,1=>Float32x2,2=>Unorm8x4];

    /// Vertex buffer layout shared by `ModPipelines` and translated shader-pack mod passes.
    pub const fn buffer_layout() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Self>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &Self::ATTRIBUTES,
        }
    }
}

/// Quads that follow each other in a frame's vertices and draw with one pipeline at one stage.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PipelineRun {
    /// The pipeline's slot in the frame's [`Pipelines`].
    pub slot: u32,
    /// The stage it draws at (`STAGE_AFTER_OPAQUE_BLOCKS..=STAGE_CUSTOM_GEOMETRY`, or OIT stage
    /// `0..=2` for `Batch::oit_runs`).
    pub stage: u32,
    pub quads: u32,
    /// Interned modifier stack id (`0` = no active modifier).
    pub modifier_stack: u32,
}

/// Source of vertex and optional index buffers for one unified [`PipelineDraw`].
#[derive(Clone, Debug)]
pub enum PipelineDrawSource {
    /// Transient per-frame byte ranges inside `Batch::transient_vertices` and
    /// `Batch::transient_indices` (used by `draw_vertices` and the `pipeline_quads` shorthand).
    Transient {
        /// `(binding_index, byte_range_in_transient_vertices)`
        vertex_ranges: Vec<(u32, std::ops::Range<u64>)>,
        /// Optional `(index_format, byte_range_in_transient_indices)`
        index_range: Option<(IndexFormat, std::ops::Range<u64>)>,
    },
    /// Zero-copy retained GPU buffers resolved from a [`mod_api::MeshRef`].
    Retained(ResolvedMeshDraw),
    /// Commands read on the GPU from a retained storage buffer ([`mod_api::IndirectDraw`]). The
    /// command's `draw` range is unused.
    Indirect(IndirectSource),
}

/// What an [`mod_api::IndirectDraw`] resolved to: the mesh buffers it binds, and the commands
/// it draws (vanilla `RenderPass.drawIndexedIndirect(commands, drawCount)`).
#[derive(Clone, Debug)]
pub struct IndirectSource {
    /// The buffers of the draw's [`mod_api::MeshRef`]; `None` for a pipeline that pulls its
    /// geometry from a storage buffer. The commands are indexed iff it has an index buffer.
    pub mesh: Option<ResolvedMeshDraw>,
    /// The buffer holding the commands, created with `INDIRECT` usage.
    pub buffer: wgpu::Buffer,
    /// Byte offset of the first command, a multiple of 4 inside `buffer` for all `max_count`.
    pub offset: u64,
    /// How many commands are drawn, or with `count` the most that are.
    pub max_count: u32,
    /// The buffer and byte offset of the GPU-written `u32` count, created with `INDIRECT`
    /// usage (`multi_draw_indirect_count`).
    pub count: Option<(wgpu::Buffer, u64)>,
}

impl IndirectSource {
    /// Check `max_count` commands from `offset` of `buffer` against how they are drawn, before
    /// any GPU call: a non-indexed draw (`mesh` without an index buffer) cannot take a pipeline
    /// of a `Quads`, `Lines` or `TriangleFan` `topology`, which the engine expands from a vertex
    /// range an indirect command does not give it; `buffer` needs `INDIRECT` usage and a
    /// 4-byte `offset`, and must hold `max_count` commands of vanilla's
    /// `INDIRECT_INDEXED_DRAW_SIZE` (indexed) or `INDIRECT_DRAW_SIZE` bytes. A `count` buffer
    /// is refused where `counts` (`RenderCapabilities::multi_draw_indirect_count`) is false,
    /// and needs `INDIRECT` usage and a 4-byte offset with its `u32` inside it.
    pub fn new(
        topology: PrimitiveTopology,
        mesh: Option<ResolvedMeshDraw>,
        buffer: wgpu::Buffer,
        offset: u64,
        max_count: u32,
        count: Option<(wgpu::Buffer, u64)>,
        counts: bool,
    ) -> Result<Self, mod_api::PipelineDrawError> {
        if count.is_some() && !counts {
            return Err(mod_api::PipelineDrawError::UnsupportedIndirectCount);
        }
        if let Some((count, at)) = &count
            && (!count.usage().contains(wgpu::BufferUsages::INDIRECT)
                || !at.is_multiple_of(wgpu::COPY_BUFFER_ALIGNMENT)
                || at.checked_add(4).is_none_or(|end| end > count.size()))
        {
            return Err(mod_api::PipelineDrawError::InvalidIndirect);
        }
        let source = Self {
            mesh,
            buffer,
            offset,
            max_count,
            count,
        };
        let stride = if source.index_buffer().is_some() {
            mod_api::INDIRECT_INDEXED_DRAW_SIZE
        } else {
            if matches!(
                topology,
                PrimitiveTopology::Quads
                    | PrimitiveTopology::Lines
                    | PrimitiveTopology::TriangleFan
            ) {
                return Err(mod_api::PipelineDrawError::TopologyMismatch);
            }
            mod_api::INDIRECT_DRAW_SIZE
        };
        if !source.buffer.usage().contains(wgpu::BufferUsages::INDIRECT)
            || !offset.is_multiple_of(wgpu::COPY_BUFFER_ALIGNMENT)
            || u64::from(max_count)
                .checked_mul(stride)
                .and_then(|bytes| bytes.checked_add(offset))
                .is_none_or(|end| end > source.buffer.size())
        {
            return Err(mod_api::PipelineDrawError::InvalidIndirect);
        }
        Ok(source)
    }

    /// The index buffer and format that make the commands indexed, if any.
    fn index_buffer(&self) -> Option<&(wgpu::Buffer, wgpu::IndexFormat)> {
        self.mesh
            .as_ref()
            .and_then(|mesh| mesh.index_buffer.as_ref())
    }
}

/// One resolved set-1 binding entry for a [`PipelineDraw`] (`#5428`).
#[derive(Clone, Debug)]
pub enum ResolvedSet1Entry {
    /// Retained GPU uniform buffer (`UniformBufferRef`).
    UniformBuffer {
        id: u64,
        buffer: wgpu::Buffer,
        byte_len: u64,
    },
    /// Inline std140 uniform byte range in `Batch::inline_uniforms` / `Batch::oit_inline_uniforms`.
    InlineUniform { byte_range: std::ops::Range<usize> },
    /// Resolved 2D texture view with a unique cache key id.
    Texture {
        view_id: u64,
        view: wgpu::TextureView,
        /// The render target mip range the view covers, when it is one.
        mips: Option<TargetMips>,
    },
    /// Expanded `i32`/`u32` texel buffer byte range in `Batch::texel_data` / `Batch::oit_texel_data`.
    TexelBuffer { byte_range: std::ops::Range<usize> },
    /// A range of a retained storage buffer (`StorageBufferRef`), already checked against the
    /// device's binding limits.
    StorageBuffer {
        id: u64,
        buffer: wgpu::Buffer,
        offset: u64,
        size: u64,
    },
    /// One mip level of a render target colour attachment bound as a storage image, already
    /// checked against the target's storage access, with the texel `format` of its texture.
    StorageImage {
        view_id: u64,
        view: wgpu::TextureView,
        format: wgpu::TextureFormat,
        mips: TargetMips,
    },
    /// Transient frame-graph target colour attachment, resolved to a pooled GPU texture view
    /// right before executing the pass in [`super::mod_frame_graph`].
    TransientColor {
        target: mod_api::TransientTargetId,
        attachment: u8,
    },
    /// Transient frame-graph target depth attachment, resolved to a pooled GPU texture view
    /// right before executing the pass in [`super::mod_frame_graph`].
    TransientDepth { target: mod_api::TransientTargetId },
}

/// The sampled-texture ordinals that need a different sampler layout from the base pipeline.
/// Empty and sparse layouts are inline regardless of the ordinals; clones share any spill.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct SamplerLayout(NearestSamples);

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum NearestSamples {
    Inline { ordinals: [u32; 4], len: u8 },
    Spilled(Arc<Vec<u32>>),
}

impl Default for SamplerLayout {
    fn default() -> Self {
        Self::EMPTY
    }
}

impl SamplerLayout {
    pub(super) const EMPTY: Self = Self(NearestSamples::Inline {
        ordinals: [0; 4],
        len: 0,
    });

    /// Record the format of the next sampled texture at submit, in ascending sampled-texture
    /// ordinal order (excluding uniform buffers, texel buffers and storage resources).
    pub fn record(
        &mut self,
        ordinal: u32,
        sampler: &Sampler,
        format: wgpu::TextureFormat,
        float32_filterable: bool,
    ) {
        if !sampler_filtering(sampler)
            || !(format.has_depth_aspect()
                || (format == wgpu::TextureFormat::R32Float && !float32_filterable))
        {
            return;
        }
        match &mut self.0 {
            NearestSamples::Inline { ordinals, len } => {
                if let Some(next) = ordinals.get_mut(usize::from(*len)) {
                    *next = ordinal;
                    *len += 1;
                } else {
                    let mut nearest = Vec::with_capacity(2 * ordinals.len());
                    nearest.extend_from_slice(ordinals);
                    nearest.push(ordinal);
                    self.0 = NearestSamples::Spilled(Arc::new(nearest));
                }
            }
            NearestSamples::Spilled(ordinals) => Arc::make_mut(ordinals).push(ordinal),
        }
    }

    /// Whether the base pipeline can serve this layout without a sampler variant.
    pub fn is_empty(&self) -> bool {
        self.ordinals().is_empty()
    }

    pub(super) fn ordinals(&self) -> &[u32] {
        match &self.0 {
            NearestSamples::Inline { ordinals, len } => &ordinals[..usize::from(*len)],
            NearestSamples::Spilled(ordinals) => ordinals,
        }
    }

    pub(super) fn needs_nearest(&self, ordinal: u32) -> bool {
        self.ordinals().binary_search(&ordinal).is_ok()
    }
}

/// Resolved set-1 resource bindings for one [`PipelineDraw`] in exact declaration order.
#[derive(Clone, Debug, Default)]
pub struct ResolvedDrawBindings {
    /// Sampler layout chosen from the submitted texture formats.
    pub sampler_layout: SamplerLayout,
    pub entries: Vec<ResolvedSet1Entry>,
    /// Optional push-constant byte range in `Batch::inline_uniforms` / `Batch::oit_inline_uniforms`.
    pub push_constants: Option<std::ops::Range<usize>>,
    /// Whether any binding samples [`mod_api::TextureSource::WorldDepth`].
    pub reads_world_depth: bool,
}

/// One unified mod-pipeline draw command (`draw_mesh`, `draw_vertices`, or `pipeline_quads`).
#[derive(Clone, Debug)]
pub struct PipelineDraw {
    /// The pipeline's slot in the frame's [`Pipelines`].
    pub slot: u32,
    /// The stage it draws at (`STAGE_AFTER_OPAQUE_BLOCKS..=STAGE_CUSTOM_GEOMETRY`, or OIT
    /// stage `0..=2` for `Batch::oit_draws`).
    pub stage: u32,
    /// Interned modifier stack id (`0` = no active modifier).
    pub modifier_stack: u32,
    /// Vertex/index range and instance range.
    pub draw: MeshDraw,
    /// Transient byte ranges or zero-copy retained GPU buffers.
    pub source: PipelineDrawSource,
    /// Resolved set-1 resource bindings (`None` when the pipeline has no set-1 layout or push
    /// constants).
    pub bindings: Option<ResolvedDrawBindings>,
}

impl PipelineDraw {
    fn sampler_layout(&self) -> &SamplerLayout {
        static EMPTY: SamplerLayout = SamplerLayout::EMPTY;
        self.bindings
            .as_ref()
            .map_or(&EMPTY, |bindings| &bindings.sampler_layout)
    }
}

/// A frame's pipeline draws, as the game queued them: legacy quad runs plus unified
/// `draw_vertices` / `draw_mesh` commands.
#[derive(Clone, Copy, Debug, Default)]
pub struct Batch<'a> {
    /// The pipelines of the resources the frame draws with; `None` draws nothing.
    pub set: Option<&'a Pipelines>,
    pub vertices: &'a [PipelineVertex],
    pub runs: &'a [PipelineRun],
    /// Unified `draw_vertices` / `draw_mesh` / `pipeline_quads` commands for standard stages.
    pub draws: &'a [PipelineDraw],
    /// Transient per-frame vertex byte pool referenced by `draws`.
    pub transient_vertices: &'a [u8],
    /// Transient per-frame index byte pool referenced by `draws`.
    pub transient_indices: &'a [u8],
    /// Per-frame inline uniform & push-constant byte pool referenced by `draws`.
    pub inline_uniforms: &'a [u8],
    /// Per-frame expanded texel-buffer byte pool referenced by `draws`.
    pub texel_data: &'a [u8],
    /// Interned modifier stacks referenced by `runs[*].modifier_stack` and
    /// `draws[*].modifier_stack`; `None` means empty.
    pub modifiers: Option<&'a ModifierStacks>,
    /// Modifier definitions for `modifiers`; `None` reads `registered_modifiers()`.
    pub modifier_defs: Option<&'a [mod_api::PipelineModifier]>,
    /// Order-independent transparency pipeline quad vertices and runs (`OitTranslucent`).
    pub oit_vertices: &'a [PipelineVertex],
    pub oit_runs: &'a [PipelineRun],
    /// Unified `draw_vertices` / `draw_mesh` / `pipeline_quads` commands for OIT stages.
    pub oit_draws: &'a [PipelineDraw],
    /// Transient per-frame vertex byte pool referenced by `oit_draws`.
    pub oit_transient_vertices: &'a [u8],
    /// Transient per-frame index byte pool referenced by `oit_draws`.
    pub oit_transient_indices: &'a [u8],
    /// Per-frame inline uniform & push-constant byte pool referenced by `oit_draws`.
    pub oit_inline_uniforms: &'a [u8],
    /// Per-frame expanded texel-buffer byte pool referenced by `oit_draws`.
    pub oit_texel_data: &'a [u8],
    /// Seconds of game time (fallback `Globals.GameTime` when no full vanilla block is supplied).
    pub time: f32,
}

/// A run as uploaded: where its quads start in the vertex buffer.
struct Run {
    slot: usize,
    stage: usize,
    quads: std::ops::Range<u32>,
    modifier_stack: u32,
}

/// A unified draw command as prepared for the GPU render pass (zero-copy for retained meshes).
struct PreparedDraw {
    // Stage callbacks select indices in the submitted batch, even if an earlier draw failed.
    submitted_index: usize,
    slot: usize,
    stage: usize,
    modifier_stack: u32,
    topology: PrimitiveTopology,
    draw: MeshDraw,
    source: PipelineDrawSource,
    /// An empty layout draws with the base pipeline; otherwise use its nearest-sampler variant.
    sampler_layout: SamplerLayout,
    set1_bind_group: Option<(wgpu::BindGroup, Vec<u32>)>,
    /// Whether this draw samples `TextureSource::WorldDepth` and therefore runs in a read-only
    /// depth world render pass (`depth_ops: None`).
    reads_world_depth: bool,
}

/// Where textured pipelines' textures come from when the renderer builds them
/// ([`mod_api::RenderPipeline::textured`]).
#[derive(Clone, Copy)]
pub struct TextureSources<'a> {
    /// The 2D atlas the blocks and items atlases' sprites are drawn from.
    pub atlas: &'a wgpu::TextureView,
    /// A sprite's `[u0, v0, u1, v1]` in `atlas`, by its name there; `None` for no such sprite.
    pub sprite: &'a dyn Fn(&str) -> Option<[f32; 4]>,
}

/// Clip-space transform from Sunrise's world target (screen rows, reversed-Z `[1..0]`) to a
/// shader pack's `colortex0` + `depthtex0` (GL rows, GL `[0..1]` depth): maps `(x, y, z, w)`
/// to `(x, -y, w - z, w)` so `ndc_y' = -ndc_y` and `ndc_z' = 1 - ndc_z`.
fn pack_clip_matrix(view_proj: Mat4) -> Mat4 {
    let adapt = Mat4::from_cols_array(&[
        1.0, 0.0, 0.0, 0.0, //
        0.0, -1.0, 0.0, 0.0, //
        0.0, 0.0, -1.0, 0.0, //
        0.0, 0.0, 1.0, 1.0,
    ]);
    adapt * view_proj
}

/// Rewrite the `Projection` matrix (at byte offset 256..320) inside a `core_lines::uniforms`
/// 1024-byte vanilla block buffer with `pack_clip_matrix(projection)`.
fn pack_vanilla_blocks(vanilla: &[u8; 1024]) -> [u8; 1024] {
    let mut out = *vanilla;
    let proj_floats: [f32; 16] = bytemuck::pod_read_unaligned(&vanilla[256..320]);
    let adapted = pack_clip_matrix(Mat4::from_cols_array(&proj_floats));
    out[256..320].copy_from_slice(bytemuck::cast_slice(&adapted.to_cols_array()));
    out
}

/// Construct a default 1024-byte vanilla uniform buffer (`Globals`, `Projection = view_proj`,
/// `DynamicTransforms` with identity `ModelViewMat`/`TextureMat` and white `ColorModulator`,
/// zero `Fog`) for callers that have only `(view_proj, viewport, time)` such as standalone tests.
pub(crate) fn fallback_vanilla_blocks(
    view_proj: Mat4,
    viewport: [u32; 2],
    time: f32,
) -> [u8; 1024] {
    let mut data = [0f32; 256];
    // Globals (offset 0, 48 bytes): CameraBlockPos(0..3), GlintAlpha(3), CameraOffset(4..7),
    // GameTime(7), ScreenSize(8..10), MenuBlurRadius(10), UseRgss(11).
    data[7] = time;
    data[8] = viewport[0] as f32;
    data[9] = viewport[1] as f32;
    // Projection (offset 256 -> float 64): ProjMat.
    data[64..80].copy_from_slice(&view_proj.to_cols_array());
    // DynamicTransforms (offset 512 -> float 128): ModelViewMat, TextureMat, ColorModulator, ModelOffset.
    data[128..144].copy_from_slice(&Mat4::IDENTITY.to_cols_array());
    data[144..160].copy_from_slice(&Mat4::IDENTITY.to_cols_array());
    data[160..164].fill(1.0);
    let mut bytes = [0u8; 1024];
    bytes.copy_from_slice(bytemuck::cast_slice(&data));
    bytes
}

/// Per-pass draw payload for [`ModPipelines::execute_offscreen_pass`].
pub struct OffscreenPassData<'a> {
    pub pass: &'a mod_api::OffscreenPass,
    pub target: &'a super::mod_render_targets::ResolvedOffscreenTarget,
    pub draws: &'a [PipelineDraw],
    pub transient_vertices: &'a [u8],
    pub transient_indices: &'a [u8],
    pub inline_uniforms: &'a [u8],
    pub texel_data: &'a [u8],
    pub modifiers: Option<&'a ModifierStacks>,
    pub modifier_defs: Option<&'a [mod_api::PipelineModifier]>,
}

/// Target attachment and Z-direction state for [`ModPipelines::build_render_pipeline`].
#[derive(Clone, Copy)]
struct TargetState<'a> {
    color_formats: &'a [wgpu::TextureFormat],
    depth_format: Option<wgpu::TextureFormat>,
    force_depth_attachment: bool,
    standard_z: bool,
    write_mask: wgpu::ColorWrites,
}

fn wgpu_blend_factor(factor: BlendFactor) -> wgpu::BlendFactor {
    match factor {
        BlendFactor::Zero => wgpu::BlendFactor::Zero,
        BlendFactor::One => wgpu::BlendFactor::One,
        BlendFactor::SrcColor => wgpu::BlendFactor::Src,
        BlendFactor::OneMinusSrcColor => wgpu::BlendFactor::OneMinusSrc,
        BlendFactor::SrcAlpha => wgpu::BlendFactor::SrcAlpha,
        BlendFactor::OneMinusSrcAlpha => wgpu::BlendFactor::OneMinusSrcAlpha,
        BlendFactor::DstColor => wgpu::BlendFactor::Dst,
        BlendFactor::OneMinusDstColor => wgpu::BlendFactor::OneMinusDst,
        BlendFactor::DstAlpha => wgpu::BlendFactor::DstAlpha,
        BlendFactor::OneMinusDstAlpha => wgpu::BlendFactor::OneMinusDstAlpha,
        BlendFactor::SrcAlphaSaturate => wgpu::BlendFactor::SrcAlphaSaturated,
        BlendFactor::ConstantColor | BlendFactor::ConstantAlpha => wgpu::BlendFactor::Constant,
        BlendFactor::OneMinusConstantColor | BlendFactor::OneMinusConstantAlpha => {
            wgpu::BlendFactor::OneMinusConstant
        }
        _ => wgpu::BlendFactor::One,
    }
}

fn wgpu_blend_op(op: BlendOp) -> wgpu::BlendOperation {
    match op {
        BlendOp::Add => wgpu::BlendOperation::Add,
        BlendOp::Subtract => wgpu::BlendOperation::Subtract,
        BlendOp::ReverseSubtract => wgpu::BlendOperation::ReverseSubtract,
        BlendOp::Min => wgpu::BlendOperation::Min,
        BlendOp::Max => wgpu::BlendOperation::Max,
        _ => wgpu::BlendOperation::Add,
    }
}

fn wgpu_blend_state(blend: PipelineBlend) -> Option<wgpu::BlendState> {
    let func = blend.to_blend_function()?;
    Some(wgpu::BlendState {
        color: wgpu::BlendComponent {
            src_factor: wgpu_blend_factor(func.color.src),
            dst_factor: wgpu_blend_factor(func.color.dst),
            operation: wgpu_blend_op(func.color.op),
        },
        alpha: wgpu::BlendComponent {
            src_factor: wgpu_blend_factor(func.alpha.src),
            dst_factor: wgpu_blend_factor(func.alpha.dst),
            operation: wgpu_blend_op(func.alpha.op),
        },
    })
}

fn wgpu_color_writes(write: ColorWrite) -> wgpu::ColorWrites {
    let mut mask = wgpu::ColorWrites::empty();
    if write.color {
        mask |= wgpu::ColorWrites::COLOR;
    }
    if write.alpha {
        mask |= wgpu::ColorWrites::ALPHA;
    }
    mask
}

fn wgpu_vertex_format(fmt: VertexAttributeFormat) -> wgpu::VertexFormat {
    match fmt {
        VertexAttributeFormat::Float32 => wgpu::VertexFormat::Float32,
        VertexAttributeFormat::Float32x2 => wgpu::VertexFormat::Float32x2,
        VertexAttributeFormat::Float32x3 => wgpu::VertexFormat::Float32x3,
        VertexAttributeFormat::Float32x4 => wgpu::VertexFormat::Float32x4,
        VertexAttributeFormat::Float16x2 => wgpu::VertexFormat::Float16x2,
        VertexAttributeFormat::Float16x4 => wgpu::VertexFormat::Float16x4,
        VertexAttributeFormat::Unorm8x2 => wgpu::VertexFormat::Unorm8x2,
        VertexAttributeFormat::Unorm8x4 => wgpu::VertexFormat::Unorm8x4,
        VertexAttributeFormat::Snorm8x2 => wgpu::VertexFormat::Snorm8x2,
        VertexAttributeFormat::Snorm8x4 => wgpu::VertexFormat::Snorm8x4,
        VertexAttributeFormat::Uint8x2 => wgpu::VertexFormat::Uint8x2,
        VertexAttributeFormat::Uint8x4 => wgpu::VertexFormat::Uint8x4,
        VertexAttributeFormat::Sint8x2 => wgpu::VertexFormat::Sint8x2,
        VertexAttributeFormat::Sint8x4 => wgpu::VertexFormat::Sint8x4,
        VertexAttributeFormat::Unorm16x2 => wgpu::VertexFormat::Unorm16x2,
        VertexAttributeFormat::Unorm16x4 => wgpu::VertexFormat::Unorm16x4,
        VertexAttributeFormat::Snorm16x2 => wgpu::VertexFormat::Snorm16x2,
        VertexAttributeFormat::Snorm16x4 => wgpu::VertexFormat::Snorm16x4,
        VertexAttributeFormat::Uint16x2 => wgpu::VertexFormat::Uint16x2,
        VertexAttributeFormat::Uint16x4 => wgpu::VertexFormat::Uint16x4,
        VertexAttributeFormat::Sint16x2 => wgpu::VertexFormat::Sint16x2,
        VertexAttributeFormat::Sint16x4 => wgpu::VertexFormat::Sint16x4,
        VertexAttributeFormat::Uint32 => wgpu::VertexFormat::Uint32,
        VertexAttributeFormat::Uint32x2 => wgpu::VertexFormat::Uint32x2,
        VertexAttributeFormat::Uint32x3 => wgpu::VertexFormat::Uint32x3,
        VertexAttributeFormat::Uint32x4 => wgpu::VertexFormat::Uint32x4,
        VertexAttributeFormat::Sint32 => wgpu::VertexFormat::Sint32,
        VertexAttributeFormat::Sint32x2 => wgpu::VertexFormat::Sint32x2,
        VertexAttributeFormat::Sint32x3 => wgpu::VertexFormat::Sint32x3,
        VertexAttributeFormat::Sint32x4 => wgpu::VertexFormat::Sint32x4,
        VertexAttributeFormat::Unorm10_10_10_2 => wgpu::VertexFormat::Unorm10_10_10_2,
        _ => wgpu::VertexFormat::Float32,
    }
}

fn wgpu_step_mode(mode: VertexStepMode) -> wgpu::VertexStepMode {
    match mode {
        VertexStepMode::Vertex => wgpu::VertexStepMode::Vertex,
        VertexStepMode::Instance => wgpu::VertexStepMode::Instance,
        _ => wgpu::VertexStepMode::Vertex,
    }
}

/// Owned vertex buffer layout for one binding slot of a compiled mod pipeline.
struct OwnedVertexBufferLayout {
    binding: u8,
    stride: u64,
    step_mode: wgpu::VertexStepMode,
    attributes: Vec<wgpu::VertexAttribute>,
}

/// Convert a pipeline's sorted `(binding, VertexFormat)` pairs into owned wgpu vertex buffer
/// layouts with deterministic sequential locations `0..N`, refusing binding indices that exceed
/// `device.limits().max_vertex_buffers`.
fn build_vertex_layouts(
    bindings: &[(u8, VertexFormat)],
    max_vertex_buffers: u32,
) -> Result<Vec<OwnedVertexBufferLayout>, String> {
    let mut out = Vec::with_capacity(bindings.len());
    let mut next_location = 0u32;
    for (binding, fmt) in bindings {
        if u32::from(*binding) >= max_vertex_buffers {
            return Err(format!(
                "vertex binding {binding} exceeds adapter limit max_vertex_buffers ({max_vertex_buffers})"
            ));
        }
        let mut attributes = Vec::with_capacity(fmt.attributes.len());
        for attr in &fmt.attributes {
            attributes.push(wgpu::VertexAttribute {
                format: wgpu_vertex_format(attr.format),
                offset: u64::from(attr.offset),
                shader_location: next_location,
            });
            next_location += 1;
        }
        out.push(OwnedVertexBufferLayout {
            binding: *binding,
            stride: u64::from(fmt.stride),
            step_mode: wgpu_step_mode(fmt.step_mode),
            attributes,
        });
    }
    Ok(out)
}

fn wgpu_buffer_layouts(
    owned: &[OwnedVertexBufferLayout],
) -> Vec<Option<wgpu::VertexBufferLayout<'_>>> {
    let slot_count = owned
        .iter()
        .map(|l| usize::from(l.binding) + 1)
        .max()
        .unwrap_or(0);
    let mut layouts = vec![None; slot_count];
    for layout in owned {
        layouts[usize::from(layout.binding)] = Some(wgpu::VertexBufferLayout {
            array_stride: layout.stride,
            step_mode: layout.step_mode,
            attributes: &layout.attributes,
        });
    }
    layouts
}

fn wgpu_topology(topology: PrimitiveTopology) -> wgpu::PrimitiveTopology {
    match topology {
        PrimitiveTopology::Quads
        | PrimitiveTopology::Lines
        | PrimitiveTopology::Triangles
        | PrimitiveTopology::TriangleFan => wgpu::PrimitiveTopology::TriangleList,
        PrimitiveTopology::TriangleStrip => wgpu::PrimitiveTopology::TriangleStrip,
        PrimitiveTopology::DebugLines => wgpu::PrimitiveTopology::LineList,
        PrimitiveTopology::DebugLineStrip => wgpu::PrimitiveTopology::LineStrip,
        PrimitiveTopology::Points => wgpu::PrimitiveTopology::PointList,
        _ => wgpu::PrimitiveTopology::TriangleList,
    }
}

/// Generate triangle-list indices (`[0, i, i + 1]` for `i` in `1..vertex_count - 1`) to emulate
/// [`PrimitiveTopology::TriangleFan`] on WebGPU/wgpu.
pub(crate) fn fan_indices_for(vertex_count: u32) -> Vec<u32> {
    if vertex_count < 3 {
        return Vec::new();
    }
    let triangles = (vertex_count - 2) as usize;
    let mut indices = Vec::with_capacity(triangles * 3);
    for i in 1..(vertex_count - 1) {
        indices.extend_from_slice(&[0, i, i + 1]);
    }
    indices
}

/// Engine-generated index buffer emulating [`PrimitiveTopology::TriangleFan`] as a triangle list.
#[derive(Default)]
pub(crate) struct FanIndices {
    buffer: Option<wgpu::Buffer>,
    vertex_capacity: u32,
}

impl FanIndices {
    /// Ensure the GPU fan index buffer covers at least `vertex_count` vertices using `upload`.
    pub(crate) fn ensure_with_upload(
        &mut self,
        device: &wgpu::Device,
        upload: Upload<'_>,
        vertex_count: u32,
    ) -> &wgpu::Buffer {
        let needed = vertex_count.max(3);
        if self.buffer.is_none() || needed > self.vertex_capacity {
            let capacity = needed.next_power_of_two().max(64);
            let data = fan_indices_for(capacity);
            let bytes: &[u8] = bytemuck::cast_slice(&data);
            let buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("Mod pipeline triangle-fan indices"),
                size: bytes.len() as u64,
                usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            upload(&buffer, bytes);
            self.buffer = Some(buffer);
            self.vertex_capacity = capacity;
        }
        self.buffer.as_ref().expect("fan index buffer created")
    }

    pub(crate) fn buffer(&self) -> Option<&wgpu::Buffer> {
        self.buffer.as_ref()
    }
}

/// Append `bytes` to the persistent offscreen buffer `slot` at the next `align`ed offset after
/// `cursor`, growing the buffer (a power of two, capped at the device's legal buffer size) when
/// it does not fit, and return that offset. Passes recorded earlier this frame keep the buffer
/// they were written to alive. `None` when `bytes` is empty or the frame's passes exceed the
/// device's largest legal buffer.
#[allow(clippy::too_many_arguments)]
fn stage_offscreen_bytes(
    slot: &mut Option<wgpu::Buffer>,
    cursor: &mut u64,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    label: &'static str,
    usage: wgpu::BufferUsages,
    align: u64,
    bytes: &[u8],
) -> Option<u64> {
    if bytes.is_empty() {
        return None;
    }
    let limit = super::upload::legal_buffer_limit(device);
    let base = cursor.next_multiple_of(align.max(4));
    let end = base + (bytes.len() as u64).next_multiple_of(4);
    if end > limit {
        return None;
    }
    if slot.as_ref().is_none_or(|buffer| buffer.size() < end) {
        *slot = Some(device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(label),
            size: end.next_power_of_two().clamp(4096, limit.max(4096)),
            usage: usage | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        }));
    }
    let buffer = slot.as_ref()?;
    if bytes.len().is_multiple_of(4) {
        queue.write_buffer(buffer, base, bytes);
    } else {
        let mut padded = bytes.to_vec();
        padded.resize(bytes.len().next_multiple_of(4), 0);
        queue.write_buffer(buffer, base, &padded);
    }
    *cursor = end;
    Some(base)
}

/// Ensure a resizable transient GPU byte buffer holds at least `bytes.len()` bytes (capped at
/// `legal_buffer_limit(device)`) and upload `bytes` to it.
fn upload_transient_bytes(
    slot: &mut Option<wgpu::Buffer>,
    device: &wgpu::Device,
    upload: Upload<'_>,
    label: &'static str,
    usage: wgpu::BufferUsages,
    bytes: &[u8],
) {
    if bytes.is_empty() {
        return;
    }
    let limit = super::upload::legal_buffer_limit(device);
    let needed = (bytes.len() as u64).min(limit);
    let needs_alloc = slot
        .as_ref()
        .is_none_or(|b| b.size() < needed || b.size() > limit);
    if needs_alloc {
        let size = needed.next_power_of_two().clamp(4096, limit.max(4096));
        *slot = Some(device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(label),
            size,
            usage: usage | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        }));
    }
    if let Some(buf) = slot.as_ref() {
        let len = (bytes.len() as u64).min(buf.size()) as usize;
        let aligned = len & !3;
        if aligned > 0 {
            upload(buf, &bytes[..aligned]);
        }
    }
}

/// Cached lazy variant for one `(pipeline, modifier slot sequence)` pair.
enum CachedVariant {
    /// Modifier changed no effective state for this target; reuse the base pipeline.
    Base,
    /// Compiled variant pipeline.
    Compiled(wgpu::RenderPipeline),
    /// GPU refused this variant (already logged once with both ids); skip its runs.
    Refused,
}

/// One compiled OIT sub-pass (`DepthBounds`, `Transmittance`, or `Accumulate`) of a mod pipeline.
struct CompiledOitStage {
    pipeline: wgpu::RenderPipeline,
    pipeline_layout: wgpu::PipelineLayout,
    group_layout: wgpu::BindGroupLayout,
    vertex_shader: wgpu::ShaderModule,
    fragment_shader: wgpu::ShaderModule,
    vanilla_mask: u8,
    reads_sampler0: bool,
    reads_sprite_rect: bool,
    /// `(binding, cloud_samples_index)` for `DepthBoundsSampler` (0), `Coeff0` (1), `Coeff1` (2).
    oit_samplers: Vec<(u32, usize)>,
    /// Cached bind group for `CompiledOit::group_generation`.
    group: Option<wgpu::BindGroup>,
    /// Lazy modifier variants indexed by variant id (`ModPipelines::variant_ids`).
    variants: Vec<Option<CachedVariant>>,
}

struct CompiledOit {
    group_generation: Option<u64>,
    stages: [CompiledOitStage; 3],
    /// Sprite atlas texture view and rect buffer kept alive for OIT bind groups.
    sprite: Option<(wgpu::TextureView, wgpu::Buffer)>,
}

/// One compiled entry in a pipeline's set-1 `BindGroupLayout`.
#[derive(Clone, Debug)]
struct CompiledSet1Entry {
    binding: u32,
    kind: BindingKind,
    /// The device sampler of a [`BindingKind::CombinedImageSampler`] entry.
    sampler: Option<wgpu::Sampler>,
    /// The texel format the shader declares for a [`BindingKind::StorageImage`] entry.
    storage_format: Option<wgpu::TextureFormat>,
}

/// Key of one render target mip level within a pass scope: `(scope, texture, mip level)`.
type TargetLevel = (u32, u64, u32);

/// Whether a conflicting use was found on a level of `mips` in pass `scope`.
fn mips_conflict(conflicts: &HashSet<TargetLevel>, scope: u32, mips: &TargetMips) -> bool {
    mips.levels()
        .any(|level| conflicts.contains(&(scope, mips.texture, level)))
}

/// Cache key for one resource slot in a cached set-1 `wgpu::BindGroup`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Set1ResourceKey {
    /// Retained uniform buffer `(unique_id, byte_len)`.
    RetainedUniform(u64, u64),
    /// Inline uniform slice bound with dynamic offset (`bound_size`).
    InlineUniform(u64),
    /// Texture view `(unique_view_id)`.
    Texture(u64),
    /// Texel storage buffer range `(offset, size)`.
    TexelBuffer(u64, u64),
    /// Retained storage buffer range `(unique_id, offset, size)`.
    Storage(u64, u64, u64),
    /// Storage image view `(unique_view_id)`.
    StorageImage(u64),
}

/// Cache key for reusing a set-1 `wgpu::BindGroup` while the same resources are drawn.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct Set1CacheKey {
    slot: usize,
    sampler_layout: SamplerLayout,
    generation: u64,
    resources: Vec<Set1ResourceKey>,
}

/// GPU inputs shared by a pipeline's base and all its target / sampler variants.
struct PipelineSource {
    recipe: Arc<Built>,
    group: wgpu::BindGroup,
    pack_group: wgpu::BindGroup,
    globals_layout: wgpu::BindGroupLayout,
    /// Full set-0 layout and bind group (all frame uniform blocks + sprite texture when textured,
    /// `VERTEX_FRAGMENT`) for shader-pack preludes that read `set = 0` resources the mod's own
    /// standalone shader did not read.
    pack_globals_layout: Option<wgpu::BindGroupLayout>,
    pack_globals_group: Option<wgpu::BindGroup>,
    sprite: Option<(wgpu::TextureView, [f32; 4])>,
    vertex_shader: wgpu::ShaderModule,
    fragment_shader: wgpu::ShaderModule,
    pack_fragment_shader: Option<wgpu::ShaderModule>,
    vertex_layouts: Vec<OwnedVertexBufferLayout>,
}

/// A GPU pipeline built for one sampler layout and one set of attachments.
struct Compiled {
    source: Arc<PipelineSource>,
    pipeline: wgpu::RenderPipeline,
    pipeline_layout: wgpu::PipelineLayout,
    set1_layout: Option<wgpu::BindGroupLayout>,
    set1_entries: Vec<CompiledSet1Entry>,
    push_constant_slot: Option<(u32, u32)>,
    /// Lazy modifier variants indexed by variant id (`ModPipelines::variant_ids`; `0` unused).
    variants: Vec<Option<CachedVariant>>,
    /// Effective modifier states for this world variant, reused by direct shader-pack draws.
    variant_states: Vec<Option<(PipelineState, String)>>,
    oit: Option<CompiledOit>,
}

/// The target part of the lazy variant key. A pack's format and write mask are distinct from
/// the world target's reversed Z and the offscreen target's declared attachments.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum VariantTarget {
    World,
    Pack(wgpu::TextureFormat, bool),
    Offscreen(Vec<wgpu::TextureFormat>, Option<wgpu::TextureFormat>),
}

type VariantKey = (usize, SamplerLayout, VariantTarget);
type DrawVariants = HashMap<VariantKey, Result<Compiled, PipelineDrawError>>;

/// A pipeline-build failure carrying the refusal classification (`Invalid` vs `UnsupportedFeature`).
#[derive(Debug)]
struct BuildError {
    reason: Refusal,
    message: String,
}

impl From<String> for BuildError {
    fn from(message: String) -> Self {
        Self {
            reason: Refusal::Invalid,
            message,
        }
    }
}

impl From<&str> for BuildError {
    fn from(message: &str) -> Self {
        Self::from(message.to_owned())
    }
}

/// The pipelines and the buffer that draw a [`Batch`].
pub struct ModPipelines {
    color_format: wgpu::TextureFormat,
    linear_light: bool,
    /// Whether the active adapter/device supports `layout(early_fragment_tests) in;` reliably
    /// (`RenderCapabilities::shader_early_depth_test`).
    shader_early_depth_test: bool,
    /// Whether the active adapter/device allows fragment shaders to write storage buffers/images
    /// (`RenderCapabilities::fragment_writable_storage`).
    fragment_writable_storage: bool,
    /// Shared nearest/default sampler for `Sampler0`.
    sampler: wgpu::Sampler,
    /// Non-filtering nearest sampler for OIT float textures (`DepthBoundsSampler`, `Coeff0`, `Coeff1`).
    non_filtering_sampler: wgpu::Sampler,
    /// Shared 1024-byte vanilla uniform buffer (`Globals`, `Projection`, `DynamicTransforms`, `Fog`).
    vanilla: wgpu::Buffer,
    /// CPU copy of the most recently uploaded `vanilla` blocks so offscreen passes can override
    /// `Projection`/`ModelViewMat`/`ScreenSize` and restore afterwards in command-buffer order.
    last_vanilla_blocks: [u8; 1024],
    /// Same blocks with `Projection` adapted to shader-pack GL coordinates (`pack_clip_matrix`).
    pack_vanilla: wgpu::Buffer,
    vertices: VertexPages,
    oit_vertices: VertexPages,
    transient_vbuf: Option<wgpu::Buffer>,
    transient_ibuf: Option<wgpu::Buffer>,
    /// Persistent grow-only buffers the frame's offscreen passes share, one per kind (vertex,
    /// index, inline uniform, texel): each pass writes its own range after the previous one's,
    /// from the cursor in `offscreen_cursors`, which [`Self::begin_frame`] resets.
    offscreen_buffers: [Option<wgpu::Buffer>; 4],
    offscreen_cursors: [u64; 4],
    oit_transient_vbuf: Option<wgpu::Buffer>,
    oit_transient_ibuf: Option<wgpu::Buffer>,
    /// Per-frame growing GPU uniform ring buffer for `UniformValue::Inline` and push constants.
    inline_uniform_buf: Option<wgpu::Buffer>,
    inline_uniform_staging: Vec<u8>,
    /// Per-frame growing GPU read-only storage buffer for `TexelBuffer` bindings.
    texel_storage_buf: Option<wgpu::Buffer>,
    texel_staging: Vec<u8>,
    /// Set-1 `wgpu::BindGroup`s the last prepared frame used (kept one frame at a time, cleared on
    /// reload or ring reallocation).
    set1_bind_group_cache: HashMap<Set1CacheKey, wgpu::BindGroup>,
    /// Scratch for the frame's storage-buffer access check: how each `(pass scope, buffer id)`
    /// is bound, `(read-only, read-write)`. Cleared per frame; keeps its capacity.
    storage_uses: HashMap<(u32, u64), (bool, bool)>,
    /// Scratch: the `(pass scope, buffer id)` pairs bound both ways, whose draws are skipped.
    storage_conflicts: HashSet<(u32, u64)>,
    /// Buffer ids already warned about, so a persisting conflict logs once. Cleared on reload
    /// and trimmed when a buffer is released.
    warned_storage_conflicts: HashSet<u64>,
    /// Scratch for the frame's render-target usage check: how each `(pass scope, texture id,
    /// mip level)` is used, as [`TargetUse::bit`] sets. Cleared per frame; keeps its capacity.
    target_uses: HashMap<TargetLevel, u8>,
    /// Scratch: the levels used in a way one pass cannot combine, whose draws are skipped.
    target_conflicts: HashSet<TargetLevel>,
    /// Texture ids already warned about, so a persisting conflict logs once. Cleared on reload
    /// and trimmed when a render target is released.
    warned_target_conflicts: HashSet<u64>,
    fan_indices: FanIndices,
    /// The load `pipelines` was built from; 0 before any.
    generation: u64,
    /// Load-time world pipelines by slot, `None` for an optional/refused registration.
    pipelines: Vec<Option<Compiled>>,
    /// The lazy table for all target and sampler variants of this load.
    variants: DrawVariants,
    warned_variants: HashSet<usize>,
    /// The current direct shader-pack target, if one has been prepared.
    pack_target: Option<(wgpu::TextureFormat, bool)>,
    runs: Vec<Run>,
    oit_runs: Vec<Run>,
    /// Modifier slot sequences seen since `pipelines` was built, each with its stable variant
    /// id (`1..`): the frame's interned stack ids restart every frame, so `Compiled::variants`
    /// and `CompiledOitStage::variants` index these instead.
    variant_ids: HashMap<Vec<u32>, u32>,
    draws: Vec<PreparedDraw>,
    oit_draws: Vec<PreparedDraw>,
    /// The compute pipelines of the same load (`mod_compute`).
    pub(super) compute: super::mod_compute::ModCompute,
}

/// Whether `sampler` blends texels: a linear minification or magnification filter, or linear
/// blending between mip levels, which vanilla's backends do whenever `maxLod` allows more than
/// the base level (`GlSampler`'s `*_MIPMAP_LINEAR`; Vulkan's linear mipmap mode for
/// `maxLod > 0.25`). Wgpu lets such a sampler read a filterable texture only, so a binding of a
/// texture that is not filterable reads it through the nearest sampler instead.
pub(super) fn sampler_filtering(sampler: &Sampler) -> bool {
    sampler.min == SamplerFilter::Linear
        || sampler.mag == SamplerFilter::Linear
        || blends_mip_levels(sampler)
}

/// Vanilla's rule for linear blending between mip levels: `maxLod > 0.25`, where an absent
/// `maxLod` is its 1000.
fn blends_mip_levels(sampler: &Sampler) -> bool {
    sampler.max_lod.is_none_or(|lod| lod > 0.25)
}

fn address_mode(address: SamplerAddress) -> wgpu::AddressMode {
    match address {
        SamplerAddress::Repeat => wgpu::AddressMode::Repeat,
        SamplerAddress::MirroredRepeat => wgpu::AddressMode::MirrorRepeat,
        _ => wgpu::AddressMode::ClampToEdge,
    }
}

fn filter_mode(filter: SamplerFilter) -> wgpu::FilterMode {
    match filter {
        SamplerFilter::Linear => wgpu::FilterMode::Linear,
        _ => wgpu::FilterMode::Nearest,
    }
}

/// A sampler and its texture must agree on filtering in both render and compute layouts.
pub(super) fn sampled_layout_entries(
    binding: u32,
    visibility: wgpu::ShaderStages,
    sampler: &Sampler,
    nearest: bool,
) -> [wgpu::BindGroupLayoutEntry; 2] {
    let filterable = !nearest && sampler_filtering(sampler);
    [
        wgpu::BindGroupLayoutEntry {
            binding,
            visibility,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        },
        wgpu::BindGroupLayoutEntry {
            binding: binding + 1,
            visibility,
            ty: wgpu::BindingType::Sampler(if filterable {
                wgpu::SamplerBindingType::Filtering
            } else {
                wgpu::SamplerBindingType::NonFiltering
            }),
            count: None,
        },
    ]
}

/// The device sampler for a mod set-1 [`Sampler`] entry, made once when its pipeline is
/// compiled. With `nearest` (the entry's texture is unfilterable) it keeps the sampler's
/// addressing and level range but takes the nearest filter for minification, magnification and
/// between mip levels, whatever `max_lod` says.
pub(super) fn create_mod_sampler(
    device: &wgpu::Device,
    sampler: &Sampler,
    nearest: bool,
) -> wgpu::Sampler {
    device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("Mod set-1 sampler"),
        address_mode_u: address_mode(sampler.address_u),
        address_mode_v: address_mode(sampler.address_v),
        address_mode_w: wgpu::AddressMode::ClampToEdge,
        mag_filter: filter_mode(if nearest {
            SamplerFilter::Nearest
        } else {
            sampler.mag
        }),
        min_filter: filter_mode(if nearest {
            SamplerFilter::Nearest
        } else {
            sampler.min
        }),
        mipmap_filter: if !nearest && blends_mip_levels(sampler) {
            wgpu::MipmapFilterMode::Linear
        } else {
            wgpu::MipmapFilterMode::Nearest
        },
        lod_min_clamp: 0.0,
        lod_max_clamp: sampler.max_lod.unwrap_or(32.0),
        ..Default::default()
    })
}

/// The texel format `built`'s shader declares (`layout(rgba8) uniform image2D`) for the storage
/// image at set-1 `binding`, when it is one a render target can have.
fn storage_image_format(built: &Built, binding: u32) -> Option<wgpu::TextureFormat> {
    [&built.program.vertex, &built.program.fragment]
        .into_iter()
        .find_map(|module| {
            module
                .global_variables
                .iter()
                .filter(|(_, var)| {
                    var.binding.as_ref().is_some_and(|at| {
                        at.group == sunrise_mod_pipeline::MOD_BIND_GROUP_SET
                            && at.binding == binding
                    })
                })
                .find_map(|(_, var)| match module.types[var.ty].inner {
                    naga::TypeInner::Image {
                        class: naga::ImageClass::Storage { format, .. },
                        ..
                    } => Some(format),
                    _ => None,
                })
        })
        .and_then(|format| match format {
            naga::StorageFormat::Rgba8Unorm => Some(wgpu::TextureFormat::Rgba8Unorm),
            naga::StorageFormat::Rgba16Float => Some(wgpu::TextureFormat::Rgba16Float),
            naga::StorageFormat::R32Float => Some(wgpu::TextureFormat::R32Float),
            _ => None,
        })
}

/// The set-1 layout of `built`. Sampled textures selected by `sampler_layout` are declared
/// unfilterable and read through a nearest sampler.
fn build_set1_layout(
    device: &wgpu::Device,
    built: &Built,
    sampler_layout: &SamplerLayout,
) -> Result<
    (
        Option<wgpu::BindGroupLayout>,
        Vec<CompiledSet1Entry>,
        Option<(u32, u32)>,
    ),
    String,
> {
    if built.bind_group_layout.is_empty() && built.push_constant_size == 0 {
        return Ok((None, Vec::new(), None));
    }
    let limits = device.limits();
    let mut layout_entries = Vec::new();
    let mut compiled_entries = Vec::new();
    let mut next_binding = 0u32;
    let mut uniform_count = 0u32;
    let mut dynamic_uniform_count = 0u32;
    let mut texture_count = 0u32;
    let mut storage_count = 0u32;
    let mut storage_image_count = 0u32;
    for entry in &built.bind_group_layout.entries {
        match entry.kind {
            BindingKind::UniformBuffer { min_size } => {
                if u64::from(min_size) > limits.max_uniform_buffer_binding_size {
                    return Err(format!(
                        "uniform buffer `{}` min_size ({min_size}) exceeds device max_uniform_buffer_binding_size ({})",
                        entry.name, limits.max_uniform_buffer_binding_size
                    ));
                }
                uniform_count += 1;
                dynamic_uniform_count += 1;
                layout_entries.push(wgpu::BindGroupLayoutEntry {
                    binding: next_binding,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: true,
                        min_binding_size: wgpu::BufferSize::new(u64::from(min_size)),
                    },
                    count: None,
                });
                compiled_entries.push(CompiledSet1Entry {
                    binding: next_binding,
                    kind: entry.kind,
                    sampler: None,
                    storage_format: None,
                });
                next_binding += 1;
            }
            BindingKind::CombinedImageSampler { sampler } => {
                let nearest = sampler_layout.needs_nearest(texture_count);
                texture_count += 1;
                layout_entries.extend(sampled_layout_entries(
                    next_binding,
                    wgpu::ShaderStages::VERTEX_FRAGMENT,
                    &sampler,
                    nearest,
                ));
                compiled_entries.push(CompiledSet1Entry {
                    binding: next_binding,
                    kind: entry.kind,
                    sampler: Some(create_mod_sampler(device, &sampler, nearest)),
                    storage_format: None,
                });
                next_binding += 2;
            }
            BindingKind::TexelBuffer { .. } => {
                storage_count += 1;
                layout_entries.push(wgpu::BindGroupLayoutEntry {
                    binding: next_binding,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: wgpu::BufferSize::new(4),
                    },
                    count: None,
                });
                compiled_entries.push(CompiledSet1Entry {
                    binding: next_binding,
                    kind: entry.kind,
                    sampler: None,
                    storage_format: None,
                });
                next_binding += 1;
            }
            BindingKind::StorageBuffer {
                read_only,
                min_size,
            } => {
                if u64::from(min_size) > limits.max_storage_buffer_binding_size {
                    return Err(format!(
                        "storage buffer `{}` min_size ({min_size}) exceeds device max_storage_buffer_binding_size ({})",
                        entry.name, limits.max_storage_buffer_binding_size
                    ));
                }
                storage_count += 1;
                // wgpu's gate: a writable storage buffer is visible to the vertex stage only on
                // a device with `VERTEX_WRITABLE_STORAGE`; a shader that writes one there is
                // refused when the pipeline is created.
                let visibility = if read_only
                    || device
                        .features()
                        .contains(wgpu::Features::VERTEX_WRITABLE_STORAGE)
                {
                    wgpu::ShaderStages::VERTEX_FRAGMENT
                } else {
                    wgpu::ShaderStages::FRAGMENT
                };
                layout_entries.push(wgpu::BindGroupLayoutEntry {
                    binding: next_binding,
                    visibility,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only },
                        has_dynamic_offset: false,
                        // The declared minimum is what the draw path checks each range against
                        // (`BufferError::BelowMinSize`), so wgpu's own check at draw time, a
                        // validation error inside the pass, cannot fire.
                        min_binding_size: wgpu::BufferSize::new(u64::from(min_size)),
                    },
                    count: None,
                });
                compiled_entries.push(CompiledSet1Entry {
                    binding: next_binding,
                    kind: entry.kind,
                    sampler: None,
                    storage_format: None,
                });
                next_binding += 1;
            }
            BindingKind::StorageImage { access } => {
                storage_image_count += 1;
                let Some(format) = storage_image_format(built, next_binding) else {
                    return Err(format!(
                        "storage image `{}` needs a `layout(rgba8)`, `layout(rgba16f)` or `layout(r32f)` declaration in its shader",
                        entry.name
                    ));
                };
                let (access, read_only) = match access {
                    mod_api::StorageImageAccess::WriteOnly => {
                        (wgpu::StorageTextureAccess::WriteOnly, false)
                    }
                    mod_api::StorageImageAccess::ReadOnly => {
                        (wgpu::StorageTextureAccess::ReadOnly, true)
                    }
                    mod_api::StorageImageAccess::ReadWrite => {
                        (wgpu::StorageTextureAccess::ReadWrite, false)
                    }
                    _ => {
                        return Err(format!(
                            "unsupported storage image access for `{}`",
                            entry.name
                        ));
                    }
                };
                // As for storage buffers, wgpu lets only a device with
                // `VERTEX_WRITABLE_STORAGE` write a storage texture in the vertex stage.
                let visibility = if read_only
                    || device
                        .features()
                        .contains(wgpu::Features::VERTEX_WRITABLE_STORAGE)
                {
                    wgpu::ShaderStages::VERTEX_FRAGMENT
                } else {
                    wgpu::ShaderStages::FRAGMENT
                };
                layout_entries.push(wgpu::BindGroupLayoutEntry {
                    binding: next_binding,
                    visibility,
                    ty: wgpu::BindingType::StorageTexture {
                        access,
                        format,
                        view_dimension: wgpu::TextureViewDimension::D2,
                    },
                    count: None,
                });
                compiled_entries.push(CompiledSet1Entry {
                    binding: next_binding,
                    kind: entry.kind,
                    sampler: None,
                    storage_format: Some(format),
                });
                next_binding += 1;
            }
            _ => {
                return Err(format!(
                    "unsupported bind-group entry kind for `{}`",
                    entry.name
                ));
            }
        }
    }
    let push_constant_slot = if built.push_constant_size > 0 {
        uniform_count += 1;
        dynamic_uniform_count += 1;
        layout_entries.push(wgpu::BindGroupLayoutEntry {
            binding: next_binding,
            visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: true,
                min_binding_size: wgpu::BufferSize::new(u64::from(built.push_constant_size)),
            },
            count: None,
        });
        Some((next_binding, built.push_constant_size))
    } else {
        None
    };
    if dynamic_uniform_count > limits.max_dynamic_uniform_buffers_per_pipeline_layout {
        return Err(format!(
            "set-1 uniform buffer count ({dynamic_uniform_count}) exceeds device max_dynamic_uniform_buffers_per_pipeline_layout ({})",
            limits.max_dynamic_uniform_buffers_per_pipeline_layout
        ));
    }
    if uniform_count > limits.max_uniform_buffers_per_shader_stage {
        return Err(format!(
            "set-1 uniform buffer count ({uniform_count}) exceeds device max_uniform_buffers_per_shader_stage ({})",
            limits.max_uniform_buffers_per_shader_stage
        ));
    }
    if texture_count > limits.max_sampled_textures_per_shader_stage {
        return Err(format!(
            "set-1 sampled texture count ({texture_count}) exceeds device max_sampled_textures_per_shader_stage ({})",
            limits.max_sampled_textures_per_shader_stage
        ));
    }
    if texture_count > limits.max_samplers_per_shader_stage {
        return Err(format!(
            "set-1 sampler count ({texture_count}) exceeds device max_samplers_per_shader_stage ({})",
            limits.max_samplers_per_shader_stage
        ));
    }
    if storage_count > limits.max_storage_buffers_per_shader_stage {
        return Err(format!(
            "set-1 storage/texel buffer count ({storage_count}) exceeds device max_storage_buffers_per_shader_stage ({})",
            limits.max_storage_buffers_per_shader_stage
        ));
    }
    if storage_image_count > limits.max_storage_textures_per_shader_stage {
        return Err(format!(
            "set-1 storage image count ({storage_image_count}) exceeds device max_storage_textures_per_shader_stage ({})",
            limits.max_storage_textures_per_shader_stage
        ));
    }
    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("Mod pipeline set-1"),
        entries: &layout_entries,
    });
    Ok((Some(layout), compiled_entries, push_constant_slot))
}

/// A world pipeline with the submitted sampler layout.
fn compiled_in<'a>(
    pipelines: &'a [Option<Compiled>],
    variants: &'a DrawVariants,
    slot: usize,
    sampler_layout: &SamplerLayout,
) -> Option<&'a Compiled> {
    if sampler_layout.is_empty() {
        pipelines.get(slot)?.as_ref()
    } else {
        variants
            .get(&(slot, sampler_layout.clone(), VariantTarget::World))?
            .as_ref()
            .ok()
    }
}

fn compiled_mut_in<'a>(
    pipelines: &'a mut [Option<Compiled>],
    variants: &'a mut DrawVariants,
    key: &VariantKey,
) -> Option<&'a mut Compiled> {
    if key.1.is_empty() {
        pipelines.get_mut(key.0)?.as_mut()
    } else {
        variants.get_mut(key)?.as_mut().ok()
    }
}

fn align_up_usize(value: usize, align: usize) -> usize {
    if align <= 1 {
        return value;
    }
    let rem = value % align;
    if rem == 0 {
        value
    } else {
        value + (align - rem)
    }
}

impl ModPipelines {
    /// Storage for pipelines drawing into `(color_format, linear_light)` over a reversed
    /// Depth32Float depth.
    pub fn new(
        device: &wgpu::Device,
        (color_format, linear_light): (wgpu::TextureFormat, bool),
    ) -> Self {
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Mod pipeline texture"),
            ..Default::default()
        });
        let non_filtering_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Mod pipeline OIT sampler"),
            ..Default::default()
        });
        let buffer = |label| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size: 1024,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        };
        let vanilla = buffer("Mod pipeline vanilla uniforms");
        let pack_vanilla = buffer("Mod pipeline pack vanilla uniforms");
        Self {
            color_format,
            linear_light,
            shader_early_depth_test: device
                .features()
                .contains(wgpu::Features::SHADER_EARLY_DEPTH_TEST),
            fragment_writable_storage: true,
            sampler,
            non_filtering_sampler,
            vanilla,
            last_vanilla_blocks: fallback_vanilla_blocks(Mat4::IDENTITY, [1, 1], 0.0),
            pack_vanilla,
            vertices: VertexPages::default(),
            oit_vertices: VertexPages::default(),
            transient_vbuf: None,
            transient_ibuf: None,
            offscreen_buffers: [None, None, None, None],
            offscreen_cursors: [0; 4],
            oit_transient_vbuf: None,
            oit_transient_ibuf: None,
            inline_uniform_buf: None,
            inline_uniform_staging: Vec::new(),
            texel_storage_buf: None,
            texel_staging: Vec::new(),
            set1_bind_group_cache: HashMap::new(),
            storage_uses: HashMap::new(),
            storage_conflicts: HashSet::new(),
            warned_storage_conflicts: HashSet::new(),
            target_uses: HashMap::new(),
            target_conflicts: HashSet::new(),
            warned_target_conflicts: HashSet::new(),
            compute: Default::default(),
            fan_indices: FanIndices::default(),
            generation: 0,
            pipelines: Vec::new(),
            variants: HashMap::new(),
            warned_variants: HashSet::new(),
            pack_target: None,
            runs: Vec::new(),
            oit_runs: Vec::new(),
            variant_ids: HashMap::new(),
            draws: Vec::new(),
            oit_draws: Vec::new(),
        }
    }

    fn build_render_pipeline(
        device: &wgpu::Device,
        label: &str,
        pipeline_layout: &wgpu::PipelineLayout,
        vertex_shader: &wgpu::ShaderModule,
        fragment_shader: &wgpu::ShaderModule,
        vertex_layouts: &[OwnedVertexBufferLayout],
        state: PipelineState,
        color_targets: &[Option<ColorTargetState>],
        target: TargetState<'_>,
    ) -> Result<wgpu::RenderPipeline, String> {
        if state.polygon == PolygonMode::Line
            && !device
                .features()
                .contains(wgpu::Features::POLYGON_MODE_LINE)
        {
            return Err("PolygonMode::Line requires the POLYGON_MODE_LINE device feature".into());
        }
        let targets: Vec<Option<wgpu::ColorTargetState>> = if color_targets.is_empty() {
            let blend = wgpu_blend_state(state.blend);
            let write_mask = wgpu_color_writes(state.color_write) & target.write_mask;
            target
                .color_formats
                .iter()
                .map(|&format| {
                    Some(wgpu::ColorTargetState {
                        format,
                        blend,
                        write_mask,
                    })
                })
                .collect()
        } else {
            target
                .color_formats
                .iter()
                .enumerate()
                .map(|(idx, &format)| {
                    let ct = color_targets.get(idx).copied().flatten()?;
                    let (blend, write) = if idx == 0 {
                        (state.blend, state.color_write)
                    } else {
                        (ct.blend, ct.write)
                    };
                    Some(wgpu::ColorTargetState {
                        format,
                        blend: wgpu_blend_state(blend),
                        write_mask: wgpu_color_writes(write) & target.write_mask,
                    })
                })
                .collect()
        };
        let depth_stencil = match (target.depth_format, state.depth_stencil) {
            (Some(format), Some(ds)) => {
                let compare = if target.standard_z {
                    ds.compare.flip_for_standard_z()
                } else {
                    ds.compare
                };
                // Targets without a stencil plane (e.g. Depth32Float, main world pass) ignore
                // stencil state rather than refusing the pipeline, matching NeoForge.
                let stencil = if format.has_stencil_aspect() {
                    wgpu_stencil_state(ds.stencil_test)
                } else {
                    wgpu::StencilState::default()
                };
                Some(wgpu::DepthStencilState {
                    format,
                    depth_write_enabled: Some(ds.write),
                    depth_compare: Some(wgpu_compare(compare)),
                    stencil,
                    bias: wgpu_depth_bias(ds, target.standard_z),
                })
            }
            (Some(format), None) if target.force_depth_attachment => {
                Some(wgpu::DepthStencilState {
                    format,
                    depth_write_enabled: Some(false),
                    depth_compare: Some(wgpu::CompareFunction::Always),
                    stencil: Default::default(),
                    bias: Default::default(),
                })
            }
            _ => None,
        };

        let buffers = wgpu_buffer_layouts(vertex_layouts);
        let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let internal = device.push_error_scope(wgpu::ErrorFilter::Internal);
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some(label),
            layout: Some(pipeline_layout),
            vertex: wgpu::VertexState {
                module: vertex_shader,
                entry_point: Some(ENTRY),
                compilation_options: Default::default(),
                buffers: &buffers,
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu_topology(state.topology),
                cull_mode: state.cull.then_some(wgpu::Face::Back),
                polygon_mode: match state.polygon {
                    PolygonMode::Line => wgpu::PolygonMode::Line,
                    _ => wgpu::PolygonMode::Fill,
                },
                ..Default::default()
            },
            depth_stencil,
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: fragment_shader,
                entry_point: Some(ENTRY),
                compilation_options: Default::default(),
                targets: &targets,
            }),
            multiview_mask: None,
            cache: None,
        });
        let internal = pollster::block_on(internal.pop());
        let validation = pollster::block_on(validation.pop());
        match internal.or(validation) {
            None => Ok(pipeline),
            Some(error) => Err(error.to_string()),
        }
    }

    fn build_oit_render_pipeline(
        device: &wgpu::Device,
        label: &str,
        pipeline_layout: &wgpu::PipelineLayout,
        vertex_shader: &wgpu::ShaderModule,
        fragment_shader: &wgpu::ShaderModule,
        vertex_layouts: &[OwnedVertexBufferLayout],
        state: PipelineState,
        stage: super::oit::Stage,
    ) -> Result<wgpu::RenderPipeline, String> {
        if state.polygon == PolygonMode::Line
            && !device
                .features()
                .contains(wgpu::Features::POLYGON_MODE_LINE)
        {
            return Err("PolygonMode::Line requires the POLYGON_MODE_LINE device feature".into());
        }
        let targets = super::oit::stage_targets(stage);
        let (depth_compare, bias) = match state.depth_stencil {
            Some(ds) => (wgpu_compare(ds.compare), wgpu_depth_bias(ds, false)),
            None => (wgpu::CompareFunction::GreaterEqual, Default::default()),
        };
        let buffers = wgpu_buffer_layouts(vertex_layouts);
        let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let internal = device.push_error_scope(wgpu::ErrorFilter::Internal);
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some(label),
            layout: Some(pipeline_layout),
            vertex: wgpu::VertexState {
                module: vertex_shader,
                entry_point: Some(ENTRY),
                compilation_options: Default::default(),
                buffers: &buffers,
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu_topology(state.topology),
                cull_mode: state.cull.then_some(wgpu::Face::Back),
                polygon_mode: match state.polygon {
                    PolygonMode::Line => wgpu::PolygonMode::Line,
                    _ => wgpu::PolygonMode::Fill,
                },
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                // OIT passes attach depth read-only (`depth_ops: None`).
                depth_write_enabled: Some(false),
                depth_compare: Some(depth_compare),
                stencil: Default::default(),
                bias,
            }),
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: fragment_shader,
                entry_point: Some(ENTRY),
                compilation_options: Default::default(),
                targets: &targets,
            }),
            multiview_mask: None,
            cache: None,
        });
        let internal = pollster::block_on(internal.pop());
        let validation = pollster::block_on(validation.pop());
        match internal.or(validation) {
            None => Ok(pipeline),
            Some(error) => Err(error.to_string()),
        }
    }

    fn build_oit_stages(
        device: &wgpu::Device,
        id: &Identifier,
        vertex_layouts: &[OwnedVertexBufferLayout],
        state: PipelineState,
        variants: &[sunrise_glsl::Translated; 3],
        sprite_binding: Option<&(wgpu::TextureView, [f32; 4])>,
        set1_layout: Option<&wgpu::BindGroupLayout>,
    ) -> Result<CompiledOit, String> {
        use wgpu::util::DeviceExt;
        let sprite = sprite_binding.map(|(view, rect)| {
            let buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("Mod pipeline OIT sprite rect"),
                contents: bytemuck::cast_slice(rect),
                usage: wgpu::BufferUsages::UNIFORM,
            });
            (view.clone(), buf)
        });
        let mut built_stages = Vec::with_capacity(3);
        for (stage_idx, (stage, program)) in super::oit::Stage::ALL
            .into_iter()
            .zip(variants.iter())
            .enumerate()
        {
            let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);
            let internal = device.push_error_scope(wgpu::ErrorFilter::Internal);
            let vertex_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("Mod pipeline OIT (vertex)"),
                source: wgpu::ShaderSource::Naga(std::borrow::Cow::Owned(program.vertex.clone())),
            });
            let fragment_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("Mod pipeline OIT (fragment)"),
                source: wgpu::ShaderSource::Naga(std::borrow::Cow::Owned(program.fragment.clone())),
            });

            let mut layout_entries = Vec::new();
            let mut vanilla_mask = 0u8;
            for b in 0..4u32 {
                let stages = program.stages_reading(b);
                if !stages.is_empty() {
                    vanilla_mask |= 1 << b;
                    layout_entries.push(block_entry(b, VANILLA_SIZES[b as usize], stages));
                }
            }
            let sampler0_stages = program.stages_reading(4) | program.stages_reading(5);
            let reads_sampler0 = !sampler0_stages.is_empty() && sprite.is_some();
            if reads_sampler0 {
                layout_entries.push(wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: sampler0_stages,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                });
                layout_entries.push(wgpu::BindGroupLayoutEntry {
                    binding: 5,
                    visibility: sampler0_stages,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                });
            }
            let sprite_rect_stages = program.stages_reading(SPRITE_RECT_BINDING);
            let reads_sprite_rect = !sprite_rect_stages.is_empty() && sprite.is_some();
            if reads_sprite_rect {
                layout_entries.push(block_entry(SPRITE_RECT_BINDING, 16, sprite_rect_stages));
            }
            let mut oit_samplers = Vec::new();
            for (other_idx, name) in program.other.iter().enumerate() {
                let Some(sample_idx) = OIT_STAGE_SAMPLERS.iter().position(|s| s == name) else {
                    continue;
                };
                let binding = program.other_binding(other_idx);
                let stages = program.stages_reading(binding) | program.stages_reading(binding + 1);
                if stages.is_empty() {
                    continue;
                }
                oit_samplers.push((binding, sample_idx));
                layout_entries.push(wgpu::BindGroupLayoutEntry {
                    binding,
                    visibility: stages,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                });
                layout_entries.push(wgpu::BindGroupLayoutEntry {
                    binding: binding + 1,
                    visibility: stages,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::NonFiltering),
                    count: None,
                });
            }
            let group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("Mod pipeline OIT"),
                entries: &layout_entries,
            });
            let mut layouts = vec![Some(&group_layout)];
            if let Some(set1) = set1_layout {
                layouts.push(Some(set1));
            }
            let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("Mod pipeline OIT"),
                bind_group_layouts: &layouts,
                ..Default::default()
            });
            let internal = pollster::block_on(internal.pop());
            let validation = pollster::block_on(validation.pop());
            if let Some(error) = internal.or(validation) {
                return Err(format!("OIT stage {stage_idx}: {error}"));
            }
            let label = format!("{id}/oit/{stage_idx}");
            let pipeline = Self::build_oit_render_pipeline(
                device,
                &label,
                &pipeline_layout,
                &vertex_shader,
                &fragment_shader,
                vertex_layouts,
                state,
                stage,
            )
            .map_err(|e| format!("OIT stage {stage_idx}: {e}"))?;
            built_stages.push(CompiledOitStage {
                pipeline,
                pipeline_layout,
                group_layout,
                vertex_shader,
                fragment_shader,
                vanilla_mask,
                reads_sampler0,
                reads_sprite_rect,
                oit_samplers,
                group: None,
                variants: Vec::new(),
            });
        }
        Ok(CompiledOit {
            group_generation: None,
            stages: built_stages.try_into().ok().expect("three OIT stages"),
            sprite,
        })
    }

    /// One load's pipeline: its translated GLSL naga modules with the fixed state it registered,
    /// its per-pipeline set-0 layout and bind groups (`vanilla` and `pack_vanilla`), and its
    /// sprite texture/rect when textured — or the GPU's error.
    /// Update feature gates from the active adapter's probed [`mod_api::RenderCapabilities`]
    /// (including vendor quirks such as Adreno Vulkan and downlevel storage flags).
    pub(super) fn set_capabilities(&mut self, capabilities: &mod_api::RenderCapabilities) {
        self.shader_early_depth_test = capabilities.shader_early_depth_test;
        self.fragment_writable_storage = capabilities.fragment_writable_storage;
    }

    fn pipeline_source(
        &self,
        device: &wgpu::Device,
        built: &Arc<Built>,
        textures: Option<TextureSources<'_>>,
    ) -> Result<Arc<PipelineSource>, BuildError> {
        use wgpu::util::DeviceExt;
        if built.program.uses_early_depth_test() && !self.shader_early_depth_test {
            return Err(BuildError {
                reason: Refusal::UnsupportedFeature,
                message: "fragment shader uses layout(early_fragment_tests) in;, which this adapter does not support (RenderCapabilities::shader_early_depth_test is false)".to_owned(),
            });
        }
        if built.program.uses_fragment_writable_storage() && !self.fragment_writable_storage {
            return Err(BuildError {
                reason: Refusal::UnsupportedFeature,
                message: "fragment shader writes storage, which this adapter does not support (RenderCapabilities::fragment_writable_storage is false)".to_owned(),
            });
        }
        let sprite_binding = match &built.texture {
            None => None,
            Some(BuiltTexture::Sprite(name)) => {
                let sources = textures.ok_or("no texture can be bound here")?;
                let Some(rect) = (sources.sprite)(name) else {
                    return Err(format!(
                        "its texture sprite {name}: {}",
                        crate::mod_pipelines::texture_refusal(
                            mod_api::PipelineTextureError::NoSuchSprite
                        )
                    )
                    .into());
                };
                Some((sources.atlas.clone(), rect))
            }
        };

        let vertex_layouts =
            build_vertex_layouts(&built.vertex_bindings, device.limits().max_vertex_buffers)?;
        let linear_fragment_module = if self.linear_light {
            let define_refs: Vec<&str> = built.defines.iter().map(String::as_str).collect();
            let vertex_inputs_owned: Vec<(String, &'static str)> = built
                .vertex_bindings
                .iter()
                .flat_map(|(_, fmt)| {
                    fmt.attributes
                        .iter()
                        .map(|a| (a.name.clone(), a.format.glsl_type()))
                })
                .collect();
            let vertex_input_refs: Vec<(&str, &str)> = vertex_inputs_owned
                .iter()
                .map(|(n, t)| (n.as_str(), *t))
                .collect();
            let mod_bindings = built.mod_bindings();
            let options = PipelineOptions {
                textured: built.texture.is_some(),
                defines: &define_refs,
                vertex_inputs: &vertex_input_refs,
                mod_bindings: &mod_bindings,
                push_constant_size: built.push_constant_size,
                linear_light: true,
                check_backends: false,
                interface: crate::core_shader::interface_limits(&device.limits()),
            };
            Some(
                sunrise_mod_pipeline::translate_linear_fragment(
                    &built.vertex_expanded,
                    &built.fragment_expanded,
                    &options,
                )
                .map_err(|e| format!("linear-light fragment decode failed: {e}"))?,
            )
        } else {
            None
        };

        let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let internal = device.push_error_scope(wgpu::ErrorFilter::Internal);

        let make_module = |label: &str, module: &naga::Module| {
            device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some(label),
                source: wgpu::ShaderSource::Naga(std::borrow::Cow::Owned(module.clone())),
            })
        };
        let vertex_shader = make_module("Mod pipeline (vertex)", &built.program.vertex);
        let (fragment_shader, pack_fragment_shader) = match &linear_fragment_module {
            Some(linear_mod) => (
                make_module("Mod pipeline (fragment, linear)", linear_mod),
                Some(make_module(
                    "Mod pipeline (fragment, gamma)",
                    &built.program.fragment,
                )),
            ),
            None => (
                make_module("Mod pipeline (fragment)", &built.program.fragment),
                None,
            ),
        };

        let sprite_rect_buf = sprite_binding.as_ref().map(|(_, rect)| {
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("Mod pipeline sprite rect"),
                contents: bytemuck::cast_slice(rect),
                usage: wgpu::BufferUsages::UNIFORM,
            })
        });

        let program = &built.program;
        let mut layout_entries = Vec::new();
        for b in 0..4u32 {
            let stages = program.stages_reading(b);
            if !stages.is_empty() {
                layout_entries.push(block_entry(b, VANILLA_SIZES[b as usize], stages));
            }
        }
        let sampler0_stages = program.stages_reading(4) | program.stages_reading(5);
        if !sampler0_stages.is_empty() && sprite_binding.is_some() {
            layout_entries.push(wgpu::BindGroupLayoutEntry {
                binding: 4,
                visibility: sampler0_stages,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            });
            layout_entries.push(wgpu::BindGroupLayoutEntry {
                binding: 5,
                visibility: sampler0_stages,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            });
        }
        let sprite_rect_stages = program.stages_reading(SPRITE_RECT_BINDING);
        if !sprite_rect_stages.is_empty() && sprite_rect_buf.is_some() {
            layout_entries.push(block_entry(SPRITE_RECT_BINDING, 16, sprite_rect_stages));
        }

        let group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Mod pipeline"),
            entries: &layout_entries,
        });
        let make_group = |label: &str, uniform_buf: &wgpu::Buffer| {
            let mut entries = Vec::with_capacity(layout_entries.len());
            for b in 0..4u32 {
                if !program.stages_reading(b).is_empty() {
                    entries.push(wgpu::BindGroupEntry {
                        binding: b,
                        resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                            buffer: uniform_buf,
                            offset: VANILLA_OFFSETS[b as usize],
                            size: wgpu::BufferSize::new(VANILLA_SIZES[b as usize]),
                        }),
                    });
                }
            }
            if !sampler0_stages.is_empty()
                && let Some((view, _)) = &sprite_binding
            {
                entries.push(wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::TextureView(view),
                });
                entries.push(wgpu::BindGroupEntry {
                    binding: 5,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                });
            }
            if !sprite_rect_stages.is_empty()
                && let Some(buf) = &sprite_rect_buf
            {
                entries.push(wgpu::BindGroupEntry {
                    binding: SPRITE_RECT_BINDING,
                    resource: buf.as_entire_binding(),
                });
            }
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some(label),
                layout: &group_layout,
                entries: &entries,
            })
        };
        let group = make_group("Mod pipeline", &self.vanilla);
        let pack_group = make_group("Mod pipeline (pack)", &self.pack_vanilla);
        let (pack_globals_layout, pack_globals_group) = if built.has_pack_prelude {
            let all_stages = wgpu::ShaderStages::VERTEX_FRAGMENT;
            let mut full_layout_entries = Vec::with_capacity(7);
            let mut full_entries = Vec::with_capacity(7);
            for b in 0..4u32 {
                full_layout_entries.push(block_entry(b, VANILLA_SIZES[b as usize], all_stages));
                full_entries.push(wgpu::BindGroupEntry {
                    binding: b,
                    resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                        buffer: &self.pack_vanilla,
                        offset: VANILLA_OFFSETS[b as usize],
                        size: wgpu::BufferSize::new(VANILLA_SIZES[b as usize]),
                    }),
                });
            }
            if let Some((view, _)) = &sprite_binding {
                full_layout_entries.push(wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: all_stages,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                });
                full_layout_entries.push(wgpu::BindGroupLayoutEntry {
                    binding: 5,
                    visibility: all_stages,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                });
                full_entries.push(wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::TextureView(view),
                });
                full_entries.push(wgpu::BindGroupEntry {
                    binding: 5,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                });
            }
            if let Some(buf) = &sprite_rect_buf {
                full_layout_entries.push(block_entry(SPRITE_RECT_BINDING, 16, all_stages));
                full_entries.push(wgpu::BindGroupEntry {
                    binding: SPRITE_RECT_BINDING,
                    resource: buf.as_entire_binding(),
                });
            }
            let full_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("Mod pipeline (pack prelude globals)"),
                entries: &full_layout_entries,
            });
            let full_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("Mod pipeline (pack prelude globals)"),
                layout: &full_layout,
                entries: &full_entries,
            });
            (Some(full_layout), Some(full_group))
        } else {
            (None, None)
        };
        let internal = pollster::block_on(internal.pop());
        let validation = pollster::block_on(validation.pop());
        if let Some(error) = internal.or(validation) {
            return Err(error.to_string().into());
        }
        Ok(Arc::new(PipelineSource {
            recipe: Arc::clone(built),
            group,
            pack_group,
            globals_layout: group_layout,
            pack_globals_layout,
            pack_globals_group,
            sprite: sprite_binding,
            vertex_shader,
            fragment_shader,
            pack_fragment_shader,
            vertex_layouts,
        }))
    }

    /// The sole constructor for GPU render pipeline variants. Pack passes drawing the mod's
    /// own shaders reuse the world variant's layout so the prepared bind group is shared.
    fn compile(
        &self,
        device: &wgpu::Device,
        source: Arc<PipelineSource>,
        target: &VariantTarget,
        sampler_layout: &SamplerLayout,
        shared_layout: Option<&Compiled>,
    ) -> Result<Compiled, String> {
        let built = &source.recipe;
        let (pipeline_layout, set1_layout, set1_entries, push_constant_slot) = if let Some(shared) =
            shared_layout
        {
            (
                shared.pipeline_layout.clone(),
                shared.set1_layout.clone(),
                shared.set1_entries.clone(),
                shared.push_constant_slot,
            )
        } else {
            let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);
            let internal = device.push_error_scope(wgpu::ErrorFilter::Internal);
            let layout =
                build_set1_layout(device, built, sampler_layout).map(|(set1, entries, push)| {
                    let mut layouts = vec![Some(&source.globals_layout)];
                    if let Some(layout) = &set1 {
                        layouts.push(Some(layout));
                    }
                    let pipeline_layout =
                        device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                            label: Some("Mod pipeline"),
                            bind_group_layouts: &layouts,
                            ..Default::default()
                        });
                    (pipeline_layout, set1, entries, push)
                });
            let internal = pollster::block_on(internal.pop());
            let validation = pollster::block_on(validation.pop());
            if let Some(error) = internal.or(validation) {
                return Err(error.to_string());
            }
            layout?
        };
        // Only `VariantTarget::World` holds HDR linear light and needs gamma-to-linear decode;
        // shader-pack buffers and mod offscreen targets receive the unmodified fragment shader.
        let frag = if matches!(target, VariantTarget::World) {
            &source.fragment_shader
        } else {
            source
                .pack_fragment_shader
                .as_ref()
                .unwrap_or(&source.fragment_shader)
        };
        let pipeline = Self::build_render_pipeline(
            device,
            &built.id.to_string(),
            &pipeline_layout,
            &source.vertex_shader,
            frag,
            &source.vertex_layouts,
            built.state,
            &built.color_targets,
            self.target_state(target),
        )?;
        let oit = if matches!(target, VariantTarget::World)
            && let Some(variants) = &built.oit
            && super::oit::supported(device)
        {
            Some(Self::build_oit_stages(
                device,
                &built.id,
                &source.vertex_layouts,
                built.state,
                variants,
                source.sprite.as_ref(),
                set1_layout.as_ref(),
            )?)
        } else {
            None
        };
        Ok(Compiled {
            source,
            pipeline,
            pipeline_layout,
            set1_layout,
            set1_entries,
            push_constant_slot,
            variants: Vec::new(),
            variant_states: Vec::new(),
            oit,
        })
    }

    fn target_state<'a>(&'a self, target: &'a VariantTarget) -> TargetState<'a> {
        let (color_formats, depth_format, force_depth_attachment, standard_z, write_mask) =
            match target {
                VariantTarget::World => (
                    std::slice::from_ref(&self.color_format),
                    Some(wgpu::TextureFormat::Depth32Float),
                    false,
                    false,
                    wgpu::ColorWrites::ALL,
                ),
                VariantTarget::Pack(format, rgb) => (
                    std::slice::from_ref(format),
                    Some(wgpu::TextureFormat::Depth32Float),
                    false,
                    true,
                    if *rgb {
                        wgpu::ColorWrites::COLOR
                    } else {
                        wgpu::ColorWrites::ALL
                    },
                ),
                VariantTarget::Offscreen(colors, depth) => (
                    colors.as_slice(),
                    *depth,
                    depth.is_some(),
                    false,
                    wgpu::ColorWrites::ALL,
                ),
            };
        TargetState {
            color_formats,
            depth_format,
            force_depth_attachment,
            standard_z,
            write_mask,
        }
    }

    /// Cache the build result per slot, target and sampler layout. A GPU refusal of one combination
    /// neither poisons another combination nor invalidates the handle.
    fn ensure_variant(
        &mut self,
        device: &wgpu::Device,
        key: &VariantKey,
    ) -> Result<(), PipelineDrawError> {
        let (slot, sampler_layout, target) = key;
        let slot = *slot;
        let base_world = sampler_layout.is_empty() && matches!(target, VariantTarget::World);
        if !base_world && let Some(compiled) = self.variants.get(key) {
            return compiled.as_ref().map(|_| ()).map_err(|error| *error);
        }
        let base = self.pipelines.get(slot).and_then(Option::as_ref).ok_or(
            PipelineDrawError::Pipeline(mod_api::RenderPipelineError::Invalid),
        )?;
        if base_world {
            return Ok(());
        }
        let shared = match target {
            VariantTarget::Pack(..) => Some(
                compiled_in(&self.pipelines, &self.variants, slot, sampler_layout)
                    .ok_or(PipelineDrawError::VariantBuildFailed)?,
            ),
            _ if sampler_layout.is_empty() => Some(base),
            _ => None,
        };
        let id = base.source.recipe.id.clone();
        let result = self.compile(device, base.source.clone(), target, sampler_layout, shared).map_err(|cause| {
            let error = PipelineDrawError::VariantBuildFailed;
            if self.warned_variants.insert(slot) {
                log::warn!("Sunrise: mod pipeline {id} {error:?} ({target:?}, nearest sampler ordinals {:?}): {cause}", sampler_layout.ordinals());
            }
            error
        });
        let prepared = result.as_ref().map(|_| ()).map_err(|error| *error);
        self.variants.insert(key.clone(), result);
        prepared
    }

    /// Prepare the uploaded batch's direct mod pipelines into a pack's `colortex0` + standard-Z
    /// depth. Linked translated pack programs are distinct and checked at draw admission.
    pub fn ensure_pack_pipelines(
        &mut self,
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        rgb: bool,
    ) {
        if self.pack_target != Some((format, rgb)) {
            self.variants
                .retain(|(_, _, target), _| !matches!(target, VariantTarget::Pack(..)));
            self.pack_target = Some((format, rgb));
        }
        // The caches deduplicate these requests, including refusals. Each request takes only
        // the key from the batch entry (sharing any sampler spill) before borrowing the cache.
        for i in 0..self.runs.len() {
            let run = &self.runs[i];
            self.ensure_pack_variant(
                device,
                (format, rgb),
                run.slot,
                SamplerLayout::EMPTY,
                run.modifier_stack,
            );
        }
        for i in 0..self.draws.len() {
            let draw = &self.draws[i];
            self.ensure_pack_variant(
                device,
                (format, rgb),
                draw.slot,
                draw.sampler_layout.clone(),
                draw.modifier_stack,
            );
        }
    }

    fn ensure_pack_variant(
        &mut self,
        device: &wgpu::Device,
        (format, rgb): (wgpu::TextureFormat, bool),
        slot: usize,
        sampler_layout: SamplerLayout,
        variant: u32,
    ) {
        let key = (slot, sampler_layout, VariantTarget::Pack(format, rgb));
        let idx = variant as usize;
        match self.variants.get(&key) {
            Some(Ok(pack)) => {
                if variant == ModifierStacks::EMPTY
                    || pack.variants.get(idx).is_some_and(Option::is_some)
                {
                    return;
                }
            }
            Some(Err(_)) => return,
            None => {
                if let Err(error) = self.ensure_variant(device, &key) {
                    self.warn_prepare_error(slot, error);
                    return;
                }
            }
        }
        let Some((modified, label)) = compiled_in(&self.pipelines, &self.variants, slot, &key.1)
            .and_then(|world| world.variant_states.get(idx))
            .and_then(Option::as_ref)
        else {
            return;
        };
        let pack = self.variants[&key].as_ref().expect("prepared pack variant");
        let source = &pack.source;
        let result = if *modified == source.recipe.state {
            CachedVariant::Base
        } else {
            match Self::build_render_pipeline(
                device,
                &format!("{label}/pack"),
                &pack.pipeline_layout,
                &source.vertex_shader,
                source
                    .pack_fragment_shader
                    .as_ref()
                    .unwrap_or(&source.fragment_shader),
                &source.vertex_layouts,
                *modified,
                &source.recipe.color_targets,
                self.target_state(&key.2),
            ) {
                Ok(pipeline) => CachedVariant::Compiled(pipeline),
                Err(error) => {
                    log::warn!(
                        "Sunrise: pipeline modifier variant {label} refused for shader-pack target {format:?}: {error}"
                    );
                    CachedVariant::Refused
                }
            }
        };
        let pack = self
            .variants
            .get_mut(&key)
            .expect("prepared pack variant")
            .as_mut()
            .expect("prepared pack variant");
        if pack.variants.len() <= idx {
            pack.variants.resize_with(idx + 1, || None);
        }
        pack.variants[idx] = Some(result);
    }

    fn warn_prepare_error(&mut self, slot: usize, error: PipelineDrawError) {
        if self.warned_variants.insert(slot) {
            log::warn!("Sunrise: mod pipeline slot {slot} refused in preparation: {error:?}");
        }
    }

    /// Builds the pipelines of `set` unless they are built already. A required pipeline the GPU
    /// refuses is the error, by id, and leaves this renderer and `set` as they were; an optional
    /// one is logged once, draws nothing and resolves invalid.
    /// A textured pipeline whose texture can't be bound (`textures`) is refused the same way.
    pub(super) fn build(
        &mut self,
        device: &wgpu::Device,
        set: &Pipelines,
        textures: Option<TextureSources<'_>>,
    ) -> Result<(), String> {
        self.build_with(set, |this, built| {
            let source = this.pipeline_source(device, built, textures)?;
            this.compile(
                device,
                source,
                &VariantTarget::World,
                &SamplerLayout::EMPTY,
                None,
            )
            .map_err(BuildError::from)
        })?;
        self.compute.build(device, set.compute());
        Ok(())
    }

    fn build_with(
        &mut self,
        set: &Pipelines,
        compile: impl Fn(&Self, &Arc<Built>) -> Result<Compiled, BuildError>,
    ) -> Result<(), String> {
        if set.generation() == self.generation {
            return Ok(());
        }
        let mut refused = Vec::new();
        let pipelines: Vec<_> = set
            .built()
            .iter()
            .enumerate()
            .map(|(slot, built)| {
                let built = built.as_ref()?;
                compile(self, built)
                    .map_err(|error| refused.push((slot, built, error)))
                    .ok()
            })
            .collect();
        let required: Vec<_> = refused
            .iter()
            .filter(|(_, built, _)| !built.optional)
            .map(|(_, built, error)| format!("required {}", gpu_refusal(built, &error.message)))
            .collect();
        if !required.is_empty() {
            return Err(required.join("; "));
        }
        for (slot, built, error) in refused {
            if set.refuse(slot, error.reason) {
                log::warn!("Sunrise: {}", gpu_refusal(built, &error.message));
            }
        }
        for (slot, pipeline) in pipelines.iter().enumerate() {
            if pipeline.is_some() {
                set.revalidate(slot);
            }
        }
        self.generation = set.generation();
        self.pipelines = pipelines;
        self.variants.clear();
        self.warned_variants.clear();
        self.variant_ids.clear();
        self.pack_target = None;
        self.set1_bind_group_cache.clear();
        self.warned_storage_conflicts.clear();
        self.warned_target_conflicts.clear();
        Ok(())
    }

    /// Each pipeline slot's vertex buffer layouts and set-0/set-1 bind-group layouts for
    /// shader-pack passes (prelude `set = 0` maps to `MOD_SET_BASE`, `set = 1` to
    /// `MOD_SET_BASE + 1`).
    pub fn pack_mod_layouts(
        &self,
    ) -> Vec<Option<sunrise_render_shaderpack::pack_post::ModPipelineLayouts>> {
        self.pipelines
            .iter()
            .map(|slot| {
                let c = slot.as_ref()?;
                Some(sunrise_render_shaderpack::pack_post::ModPipelineLayouts {
                    buffers: c
                        .source
                        .vertex_layouts
                        .iter()
                        .map(|l| sunrise_render_shaderpack::pack_post::ModVertexBuffer {
                            binding: u32::from(l.binding),
                            stride: l.stride,
                            step_mode: l.step_mode,
                            attributes: l.attributes.clone(),
                        })
                        .collect(),
                    globals_layout: c
                        .source
                        .pack_globals_layout
                        .as_ref()
                        .unwrap_or(&c.source.globals_layout)
                        .clone(),
                    set1_layout: c.set1_layout.clone(),
                })
            })
            .collect()
    }

    /// Ensure OIT bind groups (`DepthBoundsSampler`, `Coeff0`, `Coeff1`) match the current OIT
    /// targets generation. Rebuilt only when `samples.generation` changes (e.g. resize/reload).
    pub(super) fn prepare_oit_groups(
        &mut self,
        device: &wgpu::Device,
        samples: Option<super::oit::CloudSamples<'_>>,
    ) {
        let Some(samples) = samples else {
            return;
        };
        for compiled in self.pipelines.iter_mut().flatten().chain(
            self.variants
                .values_mut()
                .filter_map(|entry| entry.as_mut().ok()),
        ) {
            let Some(oit) = compiled.oit.as_mut() else {
                continue;
            };
            if oit.group_generation == Some(samples.generation) {
                continue;
            }
            for stage in &mut oit.stages {
                let mut entries = Vec::new();
                for b in 0..4u32 {
                    if (stage.vanilla_mask & (1 << b)) != 0 {
                        entries.push(wgpu::BindGroupEntry {
                            binding: b,
                            resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                                buffer: &self.vanilla,
                                offset: VANILLA_OFFSETS[b as usize],
                                size: wgpu::BufferSize::new(VANILLA_SIZES[b as usize]),
                            }),
                        });
                    }
                }
                if stage.reads_sampler0
                    && let Some((view, _)) = &oit.sprite
                {
                    entries.push(wgpu::BindGroupEntry {
                        binding: 4,
                        resource: wgpu::BindingResource::TextureView(view),
                    });
                    entries.push(wgpu::BindGroupEntry {
                        binding: 5,
                        resource: wgpu::BindingResource::Sampler(&self.sampler),
                    });
                }
                if stage.reads_sprite_rect
                    && let Some((_, buf)) = &oit.sprite
                {
                    entries.push(wgpu::BindGroupEntry {
                        binding: SPRITE_RECT_BINDING,
                        resource: buf.as_entire_binding(),
                    });
                }
                for &(binding, sample_idx) in &stage.oit_samplers {
                    entries.push(wgpu::BindGroupEntry {
                        binding,
                        resource: wgpu::BindingResource::TextureView(samples.views[sample_idx]),
                    });
                    entries.push(wgpu::BindGroupEntry {
                        binding: binding + 1,
                        resource: wgpu::BindingResource::Sampler(&self.non_filtering_sampler),
                    });
                }
                stage.group = Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("Mod pipeline OIT"),
                    layout: &stage.group_layout,
                    entries: &entries,
                }));
            }
            oit.group_generation = Some(samples.generation);
        }
    }

    /// The stable variant id of a modifier slot sequence (`1..`), assigned on first sight.
    fn variant_id(ids: &mut HashMap<Vec<u32>, u32>, slots: &[u32]) -> u32 {
        if let Some(&id) = ids.get(slots) {
            return id;
        }
        let id = ids.len() as u32 + 1;
        ids.insert(slots.to_vec(), id);
        id
    }

    /// Lazily compile a modified variant for `(compiled, variant id)` targeting `target` if not
    /// cached yet, returning the effective [`PipelineState`].
    #[allow(clippy::too_many_arguments)]
    fn ensure_target_modifier_variant(
        device: &wgpu::Device,
        compiled: &mut Compiled,
        target: TargetState<'_>,
        raw_fragment: bool,
        variant: u32,
        slots: &[u32],
        defs: &[mod_api::PipelineModifier],
    ) -> PipelineState {
        let idx = variant as usize;
        if idx == 0 {
            return compiled.source.recipe.state;
        }
        if compiled.variants.len() <= idx {
            compiled.variants.resize_with(idx + 1, || None);
        }
        if compiled.variants[idx].is_some() {
            return compiled
                .variant_states
                .get(idx)
                .and_then(Option::as_ref)
                .map_or(compiled.source.recipe.state, |(state, _)| *state);
        }
        let modified = ModifierStacks::apply_slots(slots, compiled.source.recipe.state, defs);
        let label =
            ModifierStacks::label_slots(slots, &compiled.source.recipe.id.to_string(), defs);
        if compiled.variant_states.len() <= idx {
            compiled.variant_states.resize_with(idx + 1, || None);
        }
        compiled.variant_states[idx] = Some((modified, label.clone()));
        if modified == compiled.source.recipe.state {
            compiled.variants[idx] = Some(CachedVariant::Base);
            return modified;
        }
        let frag = if raw_fragment {
            compiled
                .source
                .pack_fragment_shader
                .as_ref()
                .unwrap_or(&compiled.source.fragment_shader)
        } else {
            &compiled.source.fragment_shader
        };
        match Self::build_render_pipeline(
            device,
            &label,
            &compiled.pipeline_layout,
            &compiled.source.vertex_shader,
            frag,
            &compiled.source.vertex_layouts,
            modified,
            &compiled.source.recipe.color_targets,
            target,
        ) {
            Ok(pipeline) => compiled.variants[idx] = Some(CachedVariant::Compiled(pipeline)),
            Err(error) => {
                log::warn!("Sunrise: pipeline modifier variant {label} refused: {error}");
                compiled.variants[idx] = Some(CachedVariant::Refused);
            }
        }
        modified
    }

    /// Lazily compile a modified variant for `(slot, variant id)` if not cached yet.
    fn ensure_modifier_variant(
        device: &wgpu::Device,
        color_format: wgpu::TextureFormat,
        compiled: &mut Compiled,
        variant: u32,
        slots: &[u32],
        defs: &[mod_api::PipelineModifier],
    ) {
        let color_formats = [color_format];
        let _ = Self::ensure_target_modifier_variant(
            device,
            compiled,
            TargetState {
                color_formats: &color_formats,
                depth_format: Some(wgpu::TextureFormat::Depth32Float),
                force_depth_attachment: false,
                standard_z: false,
                write_mask: wgpu::ColorWrites::ALL,
            },
            false,
            variant,
            slots,
            defs,
        );
    }

    /// Lazily compile a modified OIT variant for `(slot, oit_stage, variant id)` if not cached yet.
    /// `blend` and `depth_write` overrides are ignored because OIT owns target blending and
    /// read-only depth.
    fn ensure_oit_modifier_variant(
        device: &wgpu::Device,
        compiled: &mut Compiled,
        stage_idx: usize,
        variant: u32,
        slots: &[u32],
        defs: &[mod_api::PipelineModifier],
    ) {
        let idx = variant as usize;
        if idx == 0 {
            return;
        }
        let Some(oit) = compiled.oit.as_mut() else {
            return;
        };
        let oit_stage = &mut oit.stages[stage_idx];
        if oit_stage.variants.len() <= idx {
            oit_stage.variants.resize_with(idx + 1, || None);
        }
        if oit_stage.variants[idx].is_some() {
            return;
        }
        let modified = ModifierStacks::apply_slots(slots, compiled.source.recipe.state, defs);
        let depth_key = |ds: Option<mod_api::DepthState>| {
            ds.map(|d| {
                (
                    d.compare,
                    d.depth_bias_constant,
                    d.depth_bias_slope_scale.to_bits(),
                )
            })
        };
        if depth_key(modified.depth_stencil)
            == depth_key(compiled.source.recipe.state.depth_stencil)
            && modified.cull == compiled.source.recipe.state.cull
            && modified.polygon == compiled.source.recipe.state.polygon
            && modified.topology == compiled.source.recipe.state.topology
        {
            oit_stage.variants[idx] = Some(CachedVariant::Base);
            return;
        }
        let label = format!(
            "{}/oit/{stage_idx}",
            ModifierStacks::label_slots(slots, &compiled.source.recipe.id.to_string(), defs)
        );
        match Self::build_oit_render_pipeline(
            device,
            &label,
            &oit_stage.pipeline_layout,
            &oit_stage.vertex_shader,
            &oit_stage.fragment_shader,
            &compiled.source.vertex_layouts,
            modified,
            super::oit::Stage::ALL[stage_idx],
        ) {
            Ok(pipeline) => oit_stage.variants[idx] = Some(CachedVariant::Compiled(pipeline)),
            Err(error) => {
                log::warn!("Sunrise: OIT pipeline modifier variant {label} refused: {error}");
                oit_stage.variants[idx] = Some(CachedVariant::Refused);
            }
        }
    }

    /// Upload the frame's quads and vanilla uniform blocks (`vanilla` as `core_lines::uniforms`
    /// lays them out, or synthesized from `view_proj`/`viewport`/`batch.time` when `None`),
    /// returning the number of indices needed for its largest page.
    pub fn prepare(
        &mut self,
        device: &wgpu::Device,
        upload: Upload<'_>,
        batch: &Batch<'_>,
        view_proj: Mat4,
        viewport: [u32; 2],
        vanilla_blocks: Option<&[u8; 1024]>,
    ) -> u32 {
        self.runs.clear();
        self.oit_runs.clear();
        self.draws.clear();
        self.oit_draws.clear();
        if batch
            .set
            .is_none_or(|set| set.generation() != self.generation)
        {
            return 0;
        }
        let modifier_defs = match batch.modifier_defs {
            Some(defs) => defs,
            None => crate::mod_pipelines::registered_modifiers(),
        };
        let mut next = 0;
        for run in batch.runs {
            let quads = next..next + run.quads;
            next = quads.end;
            if quads.end as usize * 4 > batch.vertices.len() {
                continue;
            }
            let Some(Some(compiled)) = self.pipelines.get_mut(run.slot as usize) else {
                continue;
            };
            let variant = match batch.modifiers {
                _ if run.modifier_stack == ModifierStacks::EMPTY => ModifierStacks::EMPTY,
                Some(stacks) => {
                    let slots = stacks.get(run.modifier_stack);
                    let variant = Self::variant_id(&mut self.variant_ids, slots);
                    Self::ensure_modifier_variant(
                        device,
                        self.color_format,
                        compiled,
                        variant,
                        slots,
                        modifier_defs,
                    );
                    variant
                }
                // A stack with no table to resolve it draws nothing.
                None => u32::MAX,
            };
            self.runs.push(Run {
                slot: run.slot as usize,
                stage: run.stage as usize,
                quads,
                modifier_stack: variant,
            });
        }
        let mut oit_next = 0;
        for run in batch.oit_runs {
            let quads = oit_next..oit_next + run.quads;
            oit_next = quads.end;
            if quads.end as usize * 4 > batch.oit_vertices.len() || run.stage >= 3 {
                continue;
            }
            let Some(Some(compiled)) = self.pipelines.get_mut(run.slot as usize) else {
                continue;
            };
            if compiled.oit.is_none() {
                continue;
            }
            let variant = match batch.modifiers {
                _ if run.modifier_stack == ModifierStacks::EMPTY => ModifierStacks::EMPTY,
                Some(stacks) => {
                    let slots = stacks.get(run.modifier_stack);
                    let variant = Self::variant_id(&mut self.variant_ids, slots);
                    Self::ensure_oit_modifier_variant(
                        device,
                        compiled,
                        run.stage as usize,
                        variant,
                        slots,
                        modifier_defs,
                    );
                    variant
                }
                None => u32::MAX,
            };
            self.oit_runs.push(Run {
                slot: run.slot as usize,
                stage: run.stage as usize,
                quads,
                modifier_stack: variant,
            });
        }
        let mut max_index_quads = 0u32;
        let mut max_fan_verts = 0u32;
        for (submitted_index, cmd) in batch.draws.iter().enumerate() {
            let slot = cmd.slot as usize;
            let key = (slot, cmd.sampler_layout().clone(), VariantTarget::World);
            if let Err(error) = self.ensure_variant(device, &key) {
                self.warn_prepare_error(slot, error);
                continue;
            }
            let Some(compiled) = compiled_mut_in(&mut self.pipelines, &mut self.variants, &key)
            else {
                continue;
            };
            let reads_world_depth = cmd.bindings.as_ref().is_some_and(|b| b.reads_world_depth);
            let (variant, topology, writes_depth) = match batch.modifiers {
                _ if cmd.modifier_stack == ModifierStacks::EMPTY => (
                    ModifierStacks::EMPTY,
                    compiled.source.recipe.state.topology,
                    compiled
                        .source
                        .recipe
                        .state
                        .depth_stencil
                        .is_some_and(|ds| ds.write),
                ),
                Some(stacks) => {
                    let slots = stacks.get(cmd.modifier_stack);
                    let variant = Self::variant_id(&mut self.variant_ids, slots);
                    Self::ensure_modifier_variant(
                        device,
                        self.color_format,
                        compiled,
                        variant,
                        slots,
                        modifier_defs,
                    );
                    let state = ModifierStacks::apply_slots(
                        slots,
                        compiled.source.recipe.state,
                        modifier_defs,
                    );
                    (
                        variant,
                        state.topology,
                        state.depth_stencil.is_some_and(|ds| ds.write),
                    )
                }
                // A stack with no table to resolve it draws nothing.
                None => (u32::MAX, compiled.source.recipe.state.topology, false),
            };
            // A modifier stack that turns depth writes on cannot run in a read-only depth pass.
            if reads_world_depth && writes_depth {
                continue;
            }
            if let DrawRange::Vertices(ref range) = cmd.draw.range {
                let count = range.end.saturating_sub(range.start);
                match topology {
                    PrimitiveTopology::Quads | PrimitiveTopology::Lines => {
                        max_index_quads = max_index_quads.max(count / 4);
                    }
                    PrimitiveTopology::TriangleFan => {
                        max_fan_verts = max_fan_verts.max(count);
                    }
                    _ => {}
                }
            }
            self.draws.push(PreparedDraw {
                submitted_index,
                slot: cmd.slot as usize,
                stage: cmd.stage as usize,
                modifier_stack: variant,
                topology,
                draw: cmd.draw.clone(),
                source: cmd.source.clone(),
                sampler_layout: key.1,
                set1_bind_group: None,
                reads_world_depth,
            });
        }
        for (submitted_index, cmd) in batch.oit_draws.iter().enumerate() {
            if cmd.stage >= 3 {
                continue;
            }
            let slot = cmd.slot as usize;
            let key = (slot, cmd.sampler_layout().clone(), VariantTarget::World);
            if let Err(error) = self.ensure_variant(device, &key) {
                self.warn_prepare_error(slot, error);
                continue;
            }
            let Some(compiled) = compiled_mut_in(&mut self.pipelines, &mut self.variants, &key)
            else {
                continue;
            };
            if compiled.oit.is_none() {
                continue;
            }
            let (variant, topology) = match batch.modifiers {
                _ if cmd.modifier_stack == ModifierStacks::EMPTY => {
                    (ModifierStacks::EMPTY, compiled.source.recipe.state.topology)
                }
                Some(stacks) => {
                    let slots = stacks.get(cmd.modifier_stack);
                    let variant = Self::variant_id(&mut self.variant_ids, slots);
                    Self::ensure_oit_modifier_variant(
                        device,
                        compiled,
                        cmd.stage as usize,
                        variant,
                        slots,
                        modifier_defs,
                    );
                    let state = ModifierStacks::apply_slots(
                        slots,
                        compiled.source.recipe.state,
                        modifier_defs,
                    );
                    (variant, state.topology)
                }
                // A stack with no table to resolve it draws nothing.
                None => (u32::MAX, compiled.source.recipe.state.topology),
            };
            if let DrawRange::Vertices(ref range) = cmd.draw.range {
                let count = range.end.saturating_sub(range.start);
                match topology {
                    PrimitiveTopology::Quads | PrimitiveTopology::Lines => {
                        max_index_quads = max_index_quads.max(count / 4);
                    }
                    PrimitiveTopology::TriangleFan => {
                        max_fan_verts = max_fan_verts.max(count);
                    }
                    _ => {}
                }
            }
            self.oit_draws.push(PreparedDraw {
                submitted_index,
                slot: cmd.slot as usize,
                stage: cmd.stage as usize,
                modifier_stack: variant,
                topology,
                draw: cmd.draw.clone(),
                source: cmd.source.clone(),
                sampler_layout: key.1,
                set1_bind_group: None,
                reads_world_depth: cmd.bindings.as_ref().is_some_and(|b| b.reads_world_depth),
            });
        }
        if self.runs.is_empty()
            && self.oit_runs.is_empty()
            && self.draws.is_empty()
            && self.oit_draws.is_empty()
        {
            if let Some(blocks) = vanilla_blocks {
                self.last_vanilla_blocks.copy_from_slice(blocks);
                upload(&self.vanilla, blocks);
                let pack_blocks = pack_vanilla_blocks(blocks);
                upload(&self.pack_vanilla, &pack_blocks);
            }
            return 0;
        }
        self.prepare_set1_bindings(device, upload, batch);
        if let Some(last) = self.runs.last() {
            let quads = last.quads.end;
            self.vertices.upload(
                device,
                upload,
                "Mod pipeline quads",
                bytemuck::cast_slice(&batch.vertices[..quads as usize * 4]),
                4 * std::mem::size_of::<PipelineVertex>() as u64,
                0,
            );
            max_index_quads = max_index_quads.max(self.vertices.index_quads(quads));
        }
        if let Some(last) = self.oit_runs.last() {
            let quads = last.quads.end;
            self.oit_vertices.upload(
                device,
                upload,
                "Mod pipeline OIT quads",
                bytemuck::cast_slice(&batch.oit_vertices[..quads as usize * 4]),
                4 * std::mem::size_of::<PipelineVertex>() as u64,
                0,
            );
            max_index_quads = max_index_quads.max(self.oit_vertices.index_quads(quads));
        }
        upload_transient_bytes(
            &mut self.transient_vbuf,
            device,
            upload,
            "Mod pipeline transient vertices",
            wgpu::BufferUsages::VERTEX,
            batch.transient_vertices,
        );
        upload_transient_bytes(
            &mut self.transient_ibuf,
            device,
            upload,
            "Mod pipeline transient indices",
            wgpu::BufferUsages::INDEX,
            batch.transient_indices,
        );
        upload_transient_bytes(
            &mut self.oit_transient_vbuf,
            device,
            upload,
            "Mod pipeline OIT transient vertices",
            wgpu::BufferUsages::VERTEX,
            batch.oit_transient_vertices,
        );
        upload_transient_bytes(
            &mut self.oit_transient_ibuf,
            device,
            upload,
            "Mod pipeline OIT transient indices",
            wgpu::BufferUsages::INDEX,
            batch.oit_transient_indices,
        );
        if max_fan_verts >= 3 {
            self.fan_indices
                .ensure_with_upload(device, upload, max_fan_verts);
        }
        let fallback;
        let blocks = match vanilla_blocks {
            Some(b) => b,
            None => {
                fallback = fallback_vanilla_blocks(view_proj, viewport, batch.time);
                &fallback
            }
        };
        self.last_vanilla_blocks.copy_from_slice(blocks);
        upload(&self.vanilla, blocks);
        let pack_blocks = pack_vanilla_blocks(blocks);
        upload(&self.pack_vanilla, &pack_blocks);
        max_index_quads
    }

    /// Start a frame's offscreen passes: their persistent buffers are written from the start
    /// again, since the previous frame's passes have been submitted.
    pub fn begin_frame(&mut self) {
        self.offscreen_cursors = [0; 4];
    }

    /// Maximum quad index count needed by `draws` for an offscreen pass.
    pub fn offscreen_needed_quads(&self, draws: &[PipelineDraw]) -> u32 {
        let mut max_quads = 0u32;
        for cmd in draws {
            let Some(Some(compiled)) = self.pipelines.get(cmd.slot as usize) else {
                continue;
            };
            if let DrawRange::Vertices(ref range) = cmd.draw.range
                && matches!(
                    compiled.source.recipe.state.topology,
                    PrimitiveTopology::Quads | PrimitiveTopology::Lines
                )
            {
                max_quads = max_quads.max(range.end.saturating_sub(range.start) / 4);
            }
        }
        max_quads
    }

    /// Record an offscreen render pass targeting `data.target` into `encoder`.
    ///
    /// Overrides `Globals.ScreenSize`, optional `Projection.ProjMat`, and optional
    /// `DynamicTransforms.ModelViewMat` in command-buffer order via `copy_buffer_to_buffer`
    /// and restores the main pass's vanilla uniform blocks immediately after the pass ends.
    pub fn execute_offscreen_pass(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        indices: &wgpu::Buffer,
        data: &OffscreenPassData<'_>,
    ) {
        use wgpu::util::DeviceExt;
        let upload: Upload<'_> = &mut |buffer, bytes| queue.write_buffer(buffer, 0, bytes);
        let mut max_fan_verts = 0u32;
        let modifier_defs = match data.modifier_defs {
            Some(defs) => defs,
            None => crate::mod_pipelines::registered_modifiers(),
        };
        let target =
            VariantTarget::Offscreen(data.target.color_formats.clone(), data.target.depth_format);
        let offscreen_target_state = TargetState {
            color_formats: &data.target.color_formats,
            depth_format: data.target.depth_format,
            force_depth_attachment: data.target.depth_format.is_some(),
            standard_z: false,
            write_mask: wgpu::ColorWrites::ALL,
        };
        let mut ready = Vec::with_capacity(data.draws.len());
        for cmd in data.draws {
            let slot = cmd.slot as usize;
            let key = (slot, cmd.sampler_layout().clone(), target.clone());
            if let Err(error) = self.ensure_variant(device, &key) {
                self.warn_prepare_error(slot, error);
                continue;
            }
            let (variant, topology) = match data.modifiers {
                _ if cmd.modifier_stack == ModifierStacks::EMPTY => {
                    let compiled = self.variants[&key]
                        .as_ref()
                        .expect("prepared offscreen variant");
                    (ModifierStacks::EMPTY, compiled.source.recipe.state.topology)
                }
                Some(stacks) => {
                    let slots = stacks.get(cmd.modifier_stack);
                    let variant = Self::variant_id(&mut self.variant_ids, slots);
                    let compiled = self
                        .variants
                        .get_mut(&key)
                        .expect("prepared offscreen variant")
                        .as_mut()
                        .expect("prepared offscreen variant");
                    let state = Self::ensure_target_modifier_variant(
                        device,
                        compiled,
                        offscreen_target_state,
                        true,
                        variant,
                        slots,
                        modifier_defs,
                    );
                    (variant, state.topology)
                }
                // A stack with no table to resolve it draws nothing.
                None => {
                    let compiled = self.variants[&key]
                        .as_ref()
                        .expect("prepared offscreen variant");
                    (u32::MAX, compiled.source.recipe.state.topology)
                }
            };
            if topology == PrimitiveTopology::TriangleFan
                && let DrawRange::Vertices(ref range) = cmd.draw.range
            {
                max_fan_verts = max_fan_verts.max(range.end.saturating_sub(range.start));
            }
            ready.push((cmd, key, variant, topology));
        }
        if max_fan_verts >= 3 {
            self.fan_indices
                .ensure_with_upload(device, upload, max_fan_verts);
        }
        // Each pass takes its own range of the frame's persistent offscreen buffers so several
        // passes in one CommandEncoder (e.g. a multi-pass frame graph) never clobber each
        // other's vertices, indices, or set-1 staging data.
        let [vertex_slot, index_slot, ..] = &mut self.offscreen_buffers;
        let [vertex_cursor, index_cursor, ..] = &mut self.offscreen_cursors;
        let vertex_base = stage_offscreen_bytes(
            vertex_slot,
            vertex_cursor,
            device,
            queue,
            "Mod offscreen transient vertices",
            wgpu::BufferUsages::VERTEX,
            16,
            data.transient_vertices,
        );
        let index_base = stage_offscreen_bytes(
            index_slot,
            index_cursor,
            device,
            queue,
            "Mod offscreen transient indices",
            wgpu::BufferUsages::INDEX,
            16,
            data.transient_indices,
        );
        let transient_vbuf = vertex_base.and(self.offscreen_buffers[0].clone());
        let transient_ibuf = index_base.and(self.offscreen_buffers[1].clone());
        let (vertex_base, index_base) = (vertex_base.unwrap_or(0), index_base.unwrap_or(0));

        // Stage per-draw set-1 inline uniforms and texel buffers into dedicated offscreen
        // buffers so main-pass buffers are never overwritten before submission.
        let ubo_align = (device.limits().min_uniform_buffer_offset_alignment as usize).max(16);
        let storage_align = (device.limits().min_storage_buffer_offset_alignment as usize).max(4);
        let mut inline_staging: Vec<u8> = Vec::new();
        let mut texel_staging: Vec<u8> = Vec::new();
        let mut staged_meta: Vec<Option<(Vec<(u32, u64)>, Vec<(u64, u64)>)>> =
            Vec::with_capacity(ready.len());

        for (cmd, key, _, _) in &ready {
            let compiled = self.variants[key]
                .as_ref()
                .expect("prepared offscreen variant");
            if compiled.set1_layout.is_none() {
                staged_meta.push(None);
                continue;
            }
            let Some(bindings) = &cmd.bindings else {
                staged_meta.push(None);
                continue;
            };
            if bindings.entries.len() != compiled.set1_entries.len() {
                staged_meta.push(None);
                continue;
            }
            let mut inline_spans = Vec::new();
            let mut texel_spans = Vec::new();
            let mut ok = true;
            for (entry, spec) in bindings.entries.iter().zip(&compiled.set1_entries) {
                match (entry, spec.kind) {
                    (
                        ResolvedSet1Entry::InlineUniform { byte_range },
                        BindingKind::UniformBuffer { min_size },
                    ) => {
                        let Some(slice) = data.inline_uniforms.get(byte_range.clone()) else {
                            ok = false;
                            break;
                        };
                        let start = align_up_usize(inline_staging.len(), ubo_align);
                        inline_staging.resize(start, 0);
                        let padded_len = align_up_usize(slice.len().max(min_size as usize), 16);
                        inline_staging.extend_from_slice(slice);
                        inline_staging.resize(start + padded_len, 0);
                        inline_spans.push((start as u32, padded_len as u64));
                    }
                    (
                        ResolvedSet1Entry::TexelBuffer { byte_range },
                        BindingKind::TexelBuffer { .. },
                    ) => {
                        let Some(slice) = data.texel_data.get(byte_range.clone()) else {
                            ok = false;
                            break;
                        };
                        if slice.is_empty() || !slice.len().is_multiple_of(4) {
                            ok = false;
                            break;
                        }
                        let start = align_up_usize(texel_staging.len(), storage_align);
                        texel_staging.resize(start, 0);
                        texel_staging.extend_from_slice(slice);
                        texel_spans.push((start as u64, slice.len() as u64));
                    }
                    _ => {}
                }
            }
            if ok && let Some((_, pc_size)) = compiled.push_constant_slot {
                if let Some(ref pc_range) = bindings.push_constants
                    && let Some(slice) = data.inline_uniforms.get(pc_range.clone())
                {
                    let start = align_up_usize(inline_staging.len(), ubo_align);
                    inline_staging.resize(start, 0);
                    let padded_len = align_up_usize(slice.len().max(pc_size as usize), 16);
                    inline_staging.extend_from_slice(slice);
                    inline_staging.resize(start + padded_len, 0);
                    inline_spans.push((start as u32, padded_len as u64));
                } else {
                    ok = false;
                }
            }
            staged_meta.push(ok.then_some((inline_spans, texel_spans)));
        }

        let inline_base = stage_offscreen_bytes(
            &mut self.offscreen_buffers[2],
            &mut self.offscreen_cursors[2],
            device,
            queue,
            "Mod offscreen inline uniforms",
            wgpu::BufferUsages::UNIFORM,
            ubo_align as u64,
            &inline_staging,
        );
        let texel_base = stage_offscreen_bytes(
            &mut self.offscreen_buffers[3],
            &mut self.offscreen_cursors[3],
            device,
            queue,
            "Mod offscreen texel buffers",
            wgpu::BufferUsages::STORAGE,
            storage_align as u64,
            &texel_staging,
        );
        let inline_buf = inline_base.and(self.offscreen_buffers[2].clone());
        let texel_buf = texel_base.and(self.offscreen_buffers[3].clone());
        let inline_base = inline_base.unwrap_or(0) as u32;
        let texel_base = texel_base.unwrap_or(0);

        // wgpu gives a buffer written by shaders one usage for the whole pass: refuse draws
        // binding the same storage buffer both read-only and read-write in this offscreen pass.
        self.storage_uses.clear();
        self.storage_conflicts.clear();
        for (cmd, key, _, _) in &ready {
            let Some(bindings) = &cmd.bindings else {
                continue;
            };
            let compiled = self.variants[key]
                .as_ref()
                .expect("prepared offscreen variant");
            for (entry, spec) in bindings.entries.iter().zip(&compiled.set1_entries) {
                if let (
                    ResolvedSet1Entry::StorageBuffer { id, .. },
                    BindingKind::StorageBuffer { read_only, .. },
                ) = (entry, spec.kind)
                {
                    let used = self.storage_uses.entry((0, *id)).or_default();
                    if read_only {
                        used.0 = true;
                    } else {
                        used.1 = true;
                    }
                    if used.0 && used.1 {
                        self.storage_conflicts.insert((0, *id));
                    }
                }
            }
        }
        for &(_, id) in &self.storage_conflicts {
            if self.warned_storage_conflicts.insert(id) {
                log::warn!(
                    "Sunrise: mod offscreen draws binding storage buffer {id} both read-only and read-write in one pass are skipped"
                );
            }
        }

        let mut prepared_draws = Vec::with_capacity(ready.len());
        for (submitted_index, ((cmd, key, variant, topology), meta)) in
            ready.into_iter().zip(staged_meta).enumerate()
        {
            let compiled = self.variants[&key]
                .as_ref()
                .expect("prepared offscreen variant");
            let set1_bind_group = if let Some(ref set1_layout) = compiled.set1_layout {
                let (Some(bindings), Some((inline_spans, texel_spans))) = (&cmd.bindings, meta)
                else {
                    continue;
                };
                let mut dynamic_offsets: Vec<u32> = Vec::new();
                let mut bg_entries: Vec<wgpu::BindGroupEntry<'_>> = Vec::new();
                let mut inline_i = 0usize;
                let mut texel_i = 0usize;
                let mut valid = true;
                for (entry, spec) in bindings.entries.iter().zip(&compiled.set1_entries) {
                    match (entry, spec.kind) {
                        (
                            ResolvedSet1Entry::UniformBuffer {
                                buffer, byte_len, ..
                            },
                            BindingKind::UniformBuffer { .. },
                        ) => {
                            dynamic_offsets.push(0);
                            bg_entries.push(wgpu::BindGroupEntry {
                                binding: spec.binding,
                                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                                    buffer,
                                    offset: 0,
                                    size: wgpu::BufferSize::new(*byte_len),
                                }),
                            });
                        }
                        (
                            ResolvedSet1Entry::InlineUniform { .. },
                            BindingKind::UniformBuffer { .. },
                        ) => {
                            let (offset, size) = inline_spans[inline_i];
                            inline_i += 1;
                            dynamic_offsets.push(inline_base + offset);
                            let Some(buf) = inline_buf.as_ref() else {
                                valid = false;
                                break;
                            };
                            bg_entries.push(wgpu::BindGroupEntry {
                                binding: spec.binding,
                                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                                    buffer: buf,
                                    offset: 0,
                                    size: wgpu::BufferSize::new(size),
                                }),
                            });
                        }
                        (
                            ResolvedSet1Entry::Texture { view, .. },
                            BindingKind::CombinedImageSampler { .. },
                        ) => {
                            let Some(sampler) = spec.sampler.as_ref() else {
                                valid = false;
                                break;
                            };
                            bg_entries.push(wgpu::BindGroupEntry {
                                binding: spec.binding,
                                resource: wgpu::BindingResource::TextureView(view),
                            });
                            bg_entries.push(wgpu::BindGroupEntry {
                                binding: spec.binding + 1,
                                resource: wgpu::BindingResource::Sampler(sampler),
                            });
                        }
                        (
                            ResolvedSet1Entry::TexelBuffer { .. },
                            BindingKind::TexelBuffer { .. },
                        ) => {
                            let (offset, size) = texel_spans[texel_i];
                            texel_i += 1;
                            let Some(buf) = texel_buf.as_ref() else {
                                valid = false;
                                break;
                            };
                            bg_entries.push(wgpu::BindGroupEntry {
                                binding: spec.binding,
                                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                                    buffer: buf,
                                    offset: texel_base + offset,
                                    size: wgpu::BufferSize::new(size),
                                }),
                            });
                        }
                        (
                            ResolvedSet1Entry::StorageBuffer {
                                id,
                                buffer,
                                offset,
                                size,
                            },
                            BindingKind::StorageBuffer { .. },
                        ) => {
                            if self.storage_conflicts.contains(&(0, *id)) {
                                valid = false;
                                break;
                            }
                            bg_entries.push(wgpu::BindGroupEntry {
                                binding: spec.binding,
                                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                                    buffer,
                                    offset: *offset,
                                    size: wgpu::BufferSize::new(*size),
                                }),
                            });
                        }
                        (
                            ResolvedSet1Entry::StorageImage { view, format, .. },
                            BindingKind::StorageImage { .. },
                        ) => {
                            if spec.storage_format != Some(*format) {
                                valid = false;
                                break;
                            }
                            bg_entries.push(wgpu::BindGroupEntry {
                                binding: spec.binding,
                                resource: wgpu::BindingResource::TextureView(view),
                            });
                        }
                        _ => {
                            valid = false;
                            break;
                        }
                    }
                }
                if valid && let Some((pc_binding, _)) = compiled.push_constant_slot {
                    let (offset, size) = inline_spans[inline_i];
                    dynamic_offsets.push(inline_base + offset);
                    if let Some(buf) = inline_buf.as_ref() {
                        bg_entries.push(wgpu::BindGroupEntry {
                            binding: pc_binding,
                            resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                                buffer: buf,
                                offset: 0,
                                size: wgpu::BufferSize::new(size),
                            }),
                        });
                    } else {
                        valid = false;
                    }
                }
                if !valid {
                    continue;
                }
                let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);
                let bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("Mod offscreen set-1"),
                    layout: set1_layout,
                    entries: &bg_entries,
                });
                if let Some(error) = pollster::block_on(validation.pop()) {
                    log::warn!(
                        "Sunrise: a mod offscreen draw's set-1 bindings were refused: {error}"
                    );
                    continue;
                }
                Some((bg, dynamic_offsets))
            } else {
                None
            };
            prepared_draws.push((
                compiled,
                PreparedDraw {
                    submitted_index,
                    slot: cmd.slot as usize,
                    stage: cmd.stage as usize,
                    modifier_stack: variant,
                    topology,
                    draw: cmd.draw.clone(),
                    source: match &cmd.source {
                        PipelineDrawSource::Transient {
                            vertex_ranges,
                            index_range,
                        } => PipelineDrawSource::Transient {
                            vertex_ranges: vertex_ranges
                                .iter()
                                .map(|(binding, range)| {
                                    (*binding, range.start + vertex_base..range.end + vertex_base)
                                })
                                .collect(),
                            index_range: index_range.as_ref().map(|(format, range)| {
                                (*format, range.start + index_base..range.end + index_base)
                            }),
                        },
                        retained => retained.clone(),
                    },
                    sampler_layout: key.1,
                    set1_bind_group,
                    reads_world_depth: false,
                },
            ));
        }

        // Patch vanilla uniform blocks for this offscreen pass (ScreenSize, optional ProjMat,
        // optional ModelViewMat) in command-buffer order so consecutive passes never clobber.
        let mut patched = self.last_vanilla_blocks;
        let screen_size = [data.target.size[0] as f32, data.target.size[1] as f32];
        patched[32..40].copy_from_slice(bytemuck::cast_slice(&screen_size));
        if let Some(proj) = data.pass.projection {
            patched[256..320].copy_from_slice(bytemuck::cast_slice(&proj));
        }
        if let Some(view) = data.pass.view {
            patched[512..576].copy_from_slice(bytemuck::cast_slice(&view));
        }
        let patched_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Mod offscreen vanilla override"),
            contents: &patched,
            usage: wgpu::BufferUsages::COPY_SRC,
        });
        let restore_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Mod offscreen vanilla restore"),
            contents: &self.last_vanilla_blocks,
            usage: wgpu::BufferUsages::COPY_SRC,
        });
        encoder.copy_buffer_to_buffer(&patched_buf, 0, &self.vanilla, 0, 1024);

        let color_attachments: Vec<Option<wgpu::RenderPassColorAttachment<'_>>> = data
            .target
            .color_views
            .iter()
            .enumerate()
            .map(|(idx, view)| {
                let load = match data.pass.clear_color.get(idx).copied().flatten() {
                    Some([r, g, b, a]) => wgpu::LoadOp::Clear(wgpu::Color {
                        r: f64::from(r),
                        g: f64::from(g),
                        b: f64::from(b),
                        a: f64::from(a),
                    }),
                    None => wgpu::LoadOp::Load,
                };
                Some(wgpu::RenderPassColorAttachment {
                    view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load,
                        store: wgpu::StoreOp::Store,
                    },
                })
            })
            .collect();

        let depth_stencil_attachment = data.target.depth_view.as_ref().map(|view| {
            let depth_load = match data.pass.clear_depth {
                Some(d) => wgpu::LoadOp::Clear(d),
                None => wgpu::LoadOp::Load,
            };
            let stencil_ops = data.target.has_stencil.then_some(wgpu::Operations {
                load: match data.pass.clear_stencil {
                    Some(s) => wgpu::LoadOp::Clear(s),
                    None => wgpu::LoadOp::Load,
                },
                store: wgpu::StoreOp::Store,
            });
            wgpu::RenderPassDepthStencilAttachment {
                view,
                depth_ops: Some(wgpu::Operations {
                    load: depth_load,
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops,
            }
        });

        {
            let mut rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Mod offscreen pass"),
                color_attachments: &color_attachments,
                depth_stencil_attachment,
                ..Default::default()
            });
            for (compiled, cmd) in &prepared_draws {
                let (pipeline, state) = if cmd.modifier_stack == ModifierStacks::EMPTY {
                    (&compiled.pipeline, compiled.source.recipe.state)
                } else {
                    let idx = cmd.modifier_stack as usize;
                    let Some((modified, _)) =
                        compiled.variant_states.get(idx).and_then(Option::as_ref)
                    else {
                        continue;
                    };
                    match compiled.variants.get(idx) {
                        Some(Some(CachedVariant::Base)) => (&compiled.pipeline, *modified),
                        Some(Some(CachedVariant::Compiled(p))) => (p, *modified),
                        _ => continue,
                    }
                };
                rpass.set_pipeline(pipeline);
                if data.target.has_stencil {
                    let stencil_ref = state
                        .depth_stencil
                        .and_then(|ds| ds.stencil_test)
                        .map_or(0, |st| st.reference_value);
                    rpass.set_stencil_reference(stencil_ref);
                }
                rpass.set_bind_group(0, &compiled.source.group, &[]);
                if compiled.set1_layout.is_some() {
                    let Some((ref bg, ref offsets)) = cmd.set1_bind_group else {
                        continue;
                    };
                    rpass.set_bind_group(1, bg, offsets);
                }
                self.issue_prepared_draw(
                    &mut rpass,
                    indices,
                    transient_vbuf.as_ref(),
                    transient_ibuf.as_ref(),
                    cmd,
                );
            }
        }

        encoder.copy_buffer_to_buffer(&restore_buf, 0, &self.vanilla, 0, 1024);
    }

    /// Whether any prepared runs or unified draws fall in stages `first..=last`.
    pub fn has_runs(&self, first: usize, last: usize) -> bool {
        self.runs
            .iter()
            .any(|run| first <= run.stage && run.stage <= last)
            || self
                .draws
                .iter()
                .any(|draw| first <= draw.stage && draw.stage <= last)
    }

    /// Whether any prepared runs or unified draws fall in stages `first..=last` for slots not
    /// in `bound_slots`.
    pub fn has_unbound_runs(&self, first: usize, last: usize, bound_slots: &[usize]) -> bool {
        self.runs
            .iter()
            .any(|run| first <= run.stage && run.stage <= last && !bound_slots.contains(&run.slot))
            || self.draws.iter().any(|draw| {
                first <= draw.stage && draw.stage <= last && !bound_slots.contains(&draw.slot)
            })
    }

    /// Append uploaded vertex chunks (for both transient draws and retained `MeshRef` draws)
    /// whose slot is bound to `PackPost` into `out`, reusing `out`'s existing allocation.
    pub fn pack_bound_chunks_into(
        &self,
        indices: &wgpu::Buffer,
        bound_slots: &[usize],
        out: &mut Vec<super::pack_post::ModDrawChunk>,
    ) {
        if (self.runs.is_empty() && self.draws.is_empty()) || bound_slots.is_empty() {
            return;
        }
        for run in &self.runs {
            if !bound_slots.contains(&run.slot) {
                continue;
            }
            for (buffer, quads) in self.vertices.ranges(0, run.quads.clone()) {
                let mut chunk = super::pack_post::ModDrawChunk::quads(
                    run.slot,
                    buffer.clone(),
                    indices.clone(),
                    quads,
                );
                chunk.mod_globals = self.pack_globals(run.slot);
                out.push(chunk);
            }
        }
        for cmd in &self.draws {
            if !bound_slots.contains(&cmd.slot) || cmd.draw.instances.is_empty() {
                continue;
            }
            let (vertex_buffers, explicit_index) = match &cmd.source {
                PipelineDrawSource::Transient {
                    vertex_ranges,
                    index_range,
                } => {
                    let Some(vbuf) = &self.transient_vbuf else {
                        continue;
                    };
                    let vbs = vertex_ranges
                        .iter()
                        .map(|(binding, range)| (*binding, vbuf.clone(), range.clone()))
                        .collect();
                    let ib = index_range.as_ref().and_then(|(fmt, range)| {
                        self.transient_ibuf
                            .as_ref()
                            .map(|buf| (buf.clone(), wgpu_index_format(*fmt), range.clone()))
                    });
                    (vbs, ib)
                }
                PipelineDrawSource::Retained(resolved) => {
                    let vbs = resolved
                        .vertex_buffers
                        .iter()
                        .map(|(binding, buf, len)| (*binding, buf.clone(), 0..*len))
                        .collect();
                    let ib = resolved
                        .index_buffer
                        .as_ref()
                        .map(|(buf, fmt)| (buf.clone(), *fmt, 0..buf.size()));
                    (vbs, ib)
                }
                PipelineDrawSource::Indirect(indirect) => {
                    if indirect.max_count > 0 {
                        let vertex_buffers = indirect
                            .mesh
                            .iter()
                            .flat_map(|mesh| mesh.vertex_buffers.iter())
                            .map(|(binding, buf, len)| (*binding, buf.clone(), 0..*len))
                            .collect();
                        out.push(super::pack_post::ModDrawChunk {
                            slot: cmd.slot,
                            vertex_buffers,
                            kind: super::pack_post::ModDrawKind::Indirect {
                                buffer: indirect.buffer.clone(),
                                offset: indirect.offset,
                                count: indirect.max_count,
                                count_buffer: indirect.count.clone(),
                                index: indirect
                                    .index_buffer()
                                    .map(|(buf, format)| (buf.clone(), *format)),
                            },
                            instances: 0..1,
                            mod_bind_group: cmd.set1_bind_group.clone(),
                            mod_globals: self.pack_globals(cmd.slot),
                        });
                    }
                    continue;
                }
            };
            let kind = match &cmd.draw.range {
                DrawRange::Vertices(range) => {
                    let count = range.end.saturating_sub(range.start);
                    if count == 0 {
                        continue;
                    }
                    match cmd.topology {
                        PrimitiveTopology::Quads | PrimitiveTopology::Lines => {
                            let quads = count / 4;
                            if quads == 0 {
                                continue;
                            }
                            super::pack_post::ModDrawKind::Indexed {
                                buffer: indices.clone(),
                                format: wgpu::IndexFormat::Uint32,
                                indices: 0..quads * 6,
                                base_vertex: range.start as i32,
                            }
                        }
                        PrimitiveTopology::TriangleFan => {
                            let Some(fan_buf) = self.fan_indices.buffer() else {
                                continue;
                            };
                            let idx_count = cmd.topology.index_count(count);
                            if idx_count == 0 {
                                continue;
                            }
                            super::pack_post::ModDrawKind::Indexed {
                                buffer: fan_buf.clone(),
                                format: wgpu::IndexFormat::Uint32,
                                indices: 0..idx_count,
                                base_vertex: range.start as i32,
                            }
                        }
                        _ => super::pack_post::ModDrawKind::Vertices {
                            vertices: range.clone(),
                        },
                    }
                }
                DrawRange::Indexed {
                    indices: idx_range,
                    base_vertex,
                } => {
                    let Some((buf, format, _byte_range)) = explicit_index else {
                        continue;
                    };
                    if idx_range.is_empty() {
                        continue;
                    }
                    super::pack_post::ModDrawKind::Indexed {
                        buffer: buf,
                        format,
                        indices: idx_range.clone(),
                        base_vertex: *base_vertex,
                    }
                }
                _ => continue,
            };
            out.push(super::pack_post::ModDrawChunk {
                slot: cmd.slot,
                vertex_buffers,
                kind,
                instances: cmd.draw.instances.clone(),
                mod_bind_group: cmd.set1_bind_group.clone(),
                mod_globals: self.pack_globals(cmd.slot),
            });
        }
    }

    /// Slot's frame-uniform group for a shader pack's target.
    fn pack_globals(&self, slot: usize) -> Option<wgpu::BindGroup> {
        let compiled = self.pipelines.get(slot)?.as_ref()?;
        Some(
            compiled
                .source
                .pack_globals_group
                .as_ref()
                .unwrap_or(&compiled.source.pack_group)
                .clone(),
        )
    }

    /// Stage all per-frame inline uniforms, push constants, and texel buffers from `batch` into
    /// growing GPU buffers and build/lookup cached set-1 `wgpu::BindGroup`s for each prepared draw.
    fn prepare_set1_bindings<'b>(
        &mut self,
        device: &wgpu::Device,
        upload: Upload<'_>,
        batch: &Batch<'b>,
    ) {
        let ubo_align = (device.limits().min_uniform_buffer_offset_alignment as usize).max(16);
        let storage_align = (device.limits().min_storage_buffer_offset_alignment as usize).max(4);
        let limits = device.limits();
        let max_buffer = limits.max_buffer_size;
        let max_uniform = limits.max_uniform_buffer_binding_size.min(max_buffer);
        let max_texel = limits.max_storage_buffer_binding_size.min(max_buffer);
        self.inline_uniform_staging.clear();
        self.texel_staging.clear();

        struct StagedDraw {
            is_oit: bool,
            /// The pass the draw runs in as far as it is known before encoding: each OIT
            /// stage is its own pass (`1 + stage`); the standard stages share scope `0`.
            scope: u32,
            prepared_idx: usize,
            slot: usize,
            inline_spans: Vec<(u32, u64)>,
            texel_spans: Vec<(u64, u64)>,
        }
        let mut staged: Vec<(StagedDraw, &'b ResolvedDrawBindings)> = Vec::new();

        let mut stage_list = |is_oit: bool,
                              cmds: &'b [PipelineDraw],
                              prepared: &[PreparedDraw],
                              inline_pool: &[u8],
                              texel_pool: &[u8]| {
            for (idx, prepared) in prepared.iter().enumerate() {
                let cmd = &cmds[prepared.submitted_index];
                let compiled = compiled_in(
                    &self.pipelines,
                    &self.variants,
                    prepared.slot,
                    &prepared.sampler_layout,
                )
                .expect("prepared world variant");
                if compiled.set1_layout.is_none() {
                    continue;
                }
                let Some(bindings) = &cmd.bindings else {
                    continue;
                };
                if bindings.entries.len() != compiled.set1_entries.len() {
                    continue;
                }
                let mut inline_spans = Vec::new();
                let mut texel_spans = Vec::new();
                let mut ok = true;
                for (entry, spec) in bindings.entries.iter().zip(&compiled.set1_entries) {
                    match (entry, spec.kind) {
                        (
                            ResolvedSet1Entry::InlineUniform { byte_range },
                            BindingKind::UniformBuffer { min_size },
                        ) => {
                            let Some(slice) = inline_pool.get(byte_range.clone()) else {
                                ok = false;
                                break;
                            };
                            let start =
                                align_up_usize(self.inline_uniform_staging.len(), ubo_align);
                            let padded_len = align_up_usize(slice.len().max(min_size as usize), 16);
                            if padded_len as u64 > max_uniform
                                || (start + padded_len) as u64 > max_buffer
                            {
                                ok = false;
                                break;
                            }
                            self.inline_uniform_staging.resize(start, 0);
                            self.inline_uniform_staging.extend_from_slice(slice);
                            self.inline_uniform_staging.resize(start + padded_len, 0);
                            inline_spans.push((start as u32, padded_len as u64));
                        }
                        (
                            ResolvedSet1Entry::TexelBuffer { byte_range },
                            BindingKind::TexelBuffer { .. },
                        ) => {
                            let Some(slice) = texel_pool.get(byte_range.clone()) else {
                                ok = false;
                                break;
                            };
                            if slice.is_empty() || !slice.len().is_multiple_of(4) {
                                ok = false;
                                break;
                            }
                            let start = align_up_usize(self.texel_staging.len(), storage_align);
                            if slice.len() as u64 > max_texel
                                || (start + slice.len()) as u64 > max_buffer
                            {
                                ok = false;
                                break;
                            }
                            self.texel_staging.resize(start, 0);
                            self.texel_staging.extend_from_slice(slice);
                            texel_spans.push((start as u64, slice.len() as u64));
                        }
                        _ => {}
                    }
                }
                if ok && let Some((_, pc_size)) = compiled.push_constant_slot {
                    if let Some(ref pc_range) = bindings.push_constants
                        && let Some(slice) = inline_pool.get(pc_range.clone())
                    {
                        let start = align_up_usize(self.inline_uniform_staging.len(), ubo_align);
                        let padded_len = align_up_usize(slice.len().max(pc_size as usize), 16);
                        if padded_len as u64 > max_uniform
                            || (start + padded_len) as u64 > max_buffer
                        {
                            ok = false;
                        } else {
                            self.inline_uniform_staging.resize(start, 0);
                            self.inline_uniform_staging.extend_from_slice(slice);
                            self.inline_uniform_staging.resize(start + padded_len, 0);
                            inline_spans.push((start as u32, padded_len as u64));
                        }
                    } else {
                        ok = false;
                    }
                }
                if ok {
                    staged.push((
                        StagedDraw {
                            is_oit,
                            scope: if is_oit { 1 + cmd.stage } else { 0 },
                            prepared_idx: idx,
                            slot: cmd.slot as usize,
                            inline_spans,
                            texel_spans,
                        },
                        bindings,
                    ));
                }
            }
        };

        stage_list(
            false,
            batch.draws,
            &self.draws,
            batch.inline_uniforms,
            batch.texel_data,
        );
        stage_list(
            true,
            batch.oit_draws,
            &self.oit_draws,
            batch.oit_inline_uniforms,
            batch.oit_texel_data,
        );
        if staged.is_empty() {
            self.set1_bind_group_cache.clear();
            return;
        }

        if !self.inline_uniform_staging.is_empty() {
            let need = self.inline_uniform_staging.len() as u64;
            let grew = self
                .inline_uniform_buf
                .as_ref()
                .is_none_or(|b| b.size() < need);
            if grew {
                let cap = need.next_power_of_two().max(4096).min(max_buffer);
                self.inline_uniform_buf = Some(device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("Mod pipeline inline uniforms"),
                    size: cap,
                    usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                }));
                self.set1_bind_group_cache.clear();
            }
            if let Some(buf) = &self.inline_uniform_buf {
                upload(buf, &self.inline_uniform_staging);
            }
        }

        if !self.texel_staging.is_empty() {
            let need = self.texel_staging.len() as u64;
            let grew = self
                .texel_storage_buf
                .as_ref()
                .is_none_or(|b| b.size() < need);
            if grew {
                let cap = need.next_power_of_two().max(4096).min(max_buffer);
                self.texel_storage_buf = Some(device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("Mod pipeline texel buffers"),
                    size: cap,
                    usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                }));
                self.set1_bind_group_cache.clear();
            }
            if let Some(buf) = &self.texel_storage_buf {
                upload(buf, &self.texel_staging);
            }
        }

        // wgpu gives a buffer written by shaders one usage for the whole pass: a buffer bound
        // read-write and read-only within one pass is refused before any GPU call, and so is
        // one bound twice in a draw with a writable binding. A buffer may be read-only in one
        // pass and read-write in another, so the check is scoped to the pass: each OIT stage
        // is a pass of its own, and the standard stages, which a frame may encode into one
        // pass, share a scope.
        self.storage_uses.clear();
        self.storage_conflicts.clear();
        for (sd, bindings) in &staged {
            let Some(compiled) = compiled_in(
                &self.pipelines,
                &self.variants,
                sd.slot,
                &bindings.sampler_layout,
            ) else {
                continue;
            };
            for (entry, spec) in bindings.entries.iter().zip(&compiled.set1_entries) {
                if let (
                    ResolvedSet1Entry::StorageBuffer { id, .. },
                    BindingKind::StorageBuffer { read_only, .. },
                ) = (entry, spec.kind)
                {
                    let key = (sd.scope, *id);
                    let used = self.storage_uses.entry(key).or_default();
                    if read_only {
                        used.0 = true;
                    } else {
                        used.1 = true;
                    }
                    if used.0 && used.1 {
                        self.storage_conflicts.insert(key);
                    }
                }
            }
        }
        for &(_, id) in &self.storage_conflicts {
            if self.warned_storage_conflicts.insert(id) {
                log::warn!(
                    "Sunrise: mod draws binding storage buffer {id} both read-only and read-write in one pass are skipped"
                );
            }
        }

        // The same holds for a render target's mip levels: wgpu gives each one usage for the
        // whole pass, so a level one draw writes as a storage image cannot be sampled, or bound
        // with another storage access, by any other draw of the pass, nor by the draw itself.
        // Read-only storage and sampling share a usage and may be combined.
        self.target_uses.clear();
        self.target_conflicts.clear();
        for (sd, bindings) in &staged {
            for entry in &bindings.entries {
                let (ResolvedSet1Entry::Texture {
                    mips: Some(mips), ..
                }
                | ResolvedSet1Entry::StorageImage { mips, .. }) = entry
                else {
                    continue;
                };
                for level in mips.levels() {
                    let key = (sd.scope, mips.texture, level);
                    let used = self.target_uses.entry(key).or_default();
                    *used |= mips.usage.bit();
                    if TargetUse::conflict(*used) {
                        self.target_conflicts.insert(key);
                    }
                }
            }
        }
        for &(_, texture, _) in &self.target_conflicts {
            if self.warned_target_conflicts.insert(texture) {
                log::warn!(
                    "Sunrise: mod draws using a mip level of render target texture {texture} in conflicting ways (written as a storage image and also sampled or bound with another access) in one pass are skipped"
                );
            }
        }

        // Only bind groups the previous frame used survive into this one, so a released
        // uniform buffer, a resized or retired texture and a disabled mod's resources are
        // dropped with the first frame that no longer draws them.
        let mut previous = std::mem::take(&mut self.set1_bind_group_cache);
        for (sd, bindings) in staged {
            let Some(compiled) = compiled_in(
                &self.pipelines,
                &self.variants,
                sd.slot,
                &bindings.sampler_layout,
            ) else {
                continue;
            };
            let Some(ref set1_layout) = compiled.set1_layout else {
                continue;
            };
            let mut key_resources = Vec::with_capacity(
                compiled.set1_entries.len() + usize::from(compiled.push_constant_slot.is_some()),
            );
            let mut dynamic_offsets: Vec<u32> = Vec::new();
            let mut inline_cursor = 0usize;
            let mut texel_cursor = 0usize;
            let mut valid = true;
            for (entry, spec) in bindings.entries.iter().zip(&compiled.set1_entries) {
                match (entry, spec.kind) {
                    (
                        ResolvedSet1Entry::UniformBuffer { id, byte_len, .. },
                        BindingKind::UniformBuffer { .. },
                    ) => {
                        key_resources.push(Set1ResourceKey::RetainedUniform(*id, *byte_len));
                        dynamic_offsets.push(0);
                    }
                    (
                        ResolvedSet1Entry::InlineUniform { .. },
                        BindingKind::UniformBuffer { .. },
                    ) => {
                        let (offset, size) = sd.inline_spans[inline_cursor];
                        inline_cursor += 1;
                        key_resources.push(Set1ResourceKey::InlineUniform(size));
                        dynamic_offsets.push(offset);
                    }
                    (
                        ResolvedSet1Entry::Texture { view_id, mips, .. },
                        BindingKind::CombinedImageSampler { .. },
                    ) => {
                        if mips.as_ref().is_some_and(|mips| {
                            mips_conflict(&self.target_conflicts, sd.scope, mips)
                        }) {
                            valid = false;
                            break;
                        }
                        key_resources.push(Set1ResourceKey::Texture(*view_id));
                    }
                    (ResolvedSet1Entry::TexelBuffer { .. }, BindingKind::TexelBuffer { .. }) => {
                        let (offset, size) = sd.texel_spans[texel_cursor];
                        texel_cursor += 1;
                        key_resources.push(Set1ResourceKey::TexelBuffer(offset, size));
                    }
                    (
                        ResolvedSet1Entry::StorageBuffer {
                            id, offset, size, ..
                        },
                        BindingKind::StorageBuffer { .. },
                    ) => {
                        if self.storage_conflicts.contains(&(sd.scope, *id)) {
                            valid = false;
                            break;
                        }
                        key_resources.push(Set1ResourceKey::Storage(*id, *offset, *size));
                    }
                    (
                        ResolvedSet1Entry::StorageImage {
                            view_id,
                            format,
                            mips,
                            ..
                        },
                        BindingKind::StorageImage { .. },
                    ) => {
                        if spec.storage_format != Some(*format)
                            || mips_conflict(&self.target_conflicts, sd.scope, mips)
                        {
                            valid = false;
                            break;
                        }
                        key_resources.push(Set1ResourceKey::StorageImage(*view_id));
                    }
                    _ => {
                        valid = false;
                        break;
                    }
                }
            }
            if !valid {
                continue;
            }
            if compiled.push_constant_slot.is_some() {
                let (offset, size) = sd.inline_spans[inline_cursor];
                key_resources.push(Set1ResourceKey::InlineUniform(size));
                dynamic_offsets.push(offset);
            }

            let cache_key = Set1CacheKey {
                slot: sd.slot,
                sampler_layout: bindings.sampler_layout.clone(),
                generation: self.generation,
                resources: key_resources,
            };
            let bind_group = if let Some(cached) = self.set1_bind_group_cache.get(&cache_key) {
                cached.clone()
            } else if let Some(kept) = previous.remove(&cache_key) {
                self.set1_bind_group_cache.insert(cache_key, kept.clone());
                kept
            } else {
                let mut bg_entries: Vec<wgpu::BindGroupEntry<'_>> = Vec::new();
                let mut inline_i = 0usize;
                let mut texel_i = 0usize;
                for (entry, spec) in bindings.entries.iter().zip(&compiled.set1_entries) {
                    match (entry, spec.kind) {
                        (
                            ResolvedSet1Entry::UniformBuffer {
                                buffer, byte_len, ..
                            },
                            BindingKind::UniformBuffer { .. },
                        ) => {
                            bg_entries.push(wgpu::BindGroupEntry {
                                binding: spec.binding,
                                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                                    buffer,
                                    offset: 0,
                                    size: wgpu::BufferSize::new(*byte_len),
                                }),
                            });
                        }
                        (
                            ResolvedSet1Entry::InlineUniform { .. },
                            BindingKind::UniformBuffer { .. },
                        ) => {
                            let (_, size) = sd.inline_spans[inline_i];
                            inline_i += 1;
                            let Some(buf) = self.inline_uniform_buf.as_ref() else {
                                valid = false;
                                break;
                            };
                            bg_entries.push(wgpu::BindGroupEntry {
                                binding: spec.binding,
                                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                                    buffer: buf,
                                    offset: 0,
                                    size: wgpu::BufferSize::new(size),
                                }),
                            });
                        }
                        (
                            ResolvedSet1Entry::Texture { view, .. },
                            BindingKind::CombinedImageSampler { .. },
                        ) => {
                            let Some(sampler) = spec.sampler.as_ref() else {
                                valid = false;
                                break;
                            };
                            bg_entries.push(wgpu::BindGroupEntry {
                                binding: spec.binding,
                                resource: wgpu::BindingResource::TextureView(view),
                            });
                            bg_entries.push(wgpu::BindGroupEntry {
                                binding: spec.binding + 1,
                                resource: wgpu::BindingResource::Sampler(sampler),
                            });
                        }
                        (
                            ResolvedSet1Entry::TexelBuffer { .. },
                            BindingKind::TexelBuffer { .. },
                        ) => {
                            let (offset, size) = sd.texel_spans[texel_i];
                            texel_i += 1;
                            let Some(buf) = self.texel_storage_buf.as_ref() else {
                                valid = false;
                                break;
                            };
                            bg_entries.push(wgpu::BindGroupEntry {
                                binding: spec.binding,
                                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                                    buffer: buf,
                                    offset,
                                    size: wgpu::BufferSize::new(size),
                                }),
                            });
                        }
                        (
                            ResolvedSet1Entry::StorageBuffer {
                                buffer,
                                offset,
                                size,
                                ..
                            },
                            BindingKind::StorageBuffer { .. },
                        ) => {
                            bg_entries.push(wgpu::BindGroupEntry {
                                binding: spec.binding,
                                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                                    buffer,
                                    offset: *offset,
                                    size: wgpu::BufferSize::new(*size),
                                }),
                            });
                        }
                        (
                            ResolvedSet1Entry::StorageImage { view, .. },
                            BindingKind::StorageImage { .. },
                        ) => {
                            bg_entries.push(wgpu::BindGroupEntry {
                                binding: spec.binding,
                                resource: wgpu::BindingResource::TextureView(view),
                            });
                        }
                        _ => {
                            valid = false;
                            break;
                        }
                    }
                }
                if valid && let Some((pc_binding, _)) = compiled.push_constant_slot {
                    let (_, size) = sd.inline_spans[inline_i];
                    if let Some(buf) = self.inline_uniform_buf.as_ref() {
                        bg_entries.push(wgpu::BindGroupEntry {
                            binding: pc_binding,
                            resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                                buffer: buf,
                                offset: 0,
                                size: wgpu::BufferSize::new(size),
                            }),
                        });
                    } else {
                        valid = false;
                    }
                }
                if !valid {
                    continue;
                }
                let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);
                let created = device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("Mod pipeline set-1"),
                    layout: set1_layout,
                    entries: &bg_entries,
                });
                if let Some(error) = pollster::block_on(validation.pop()) {
                    log::warn!("Sunrise: a mod draw's set-1 bindings were refused: {error}");
                    continue;
                }
                self.set1_bind_group_cache
                    .insert(cache_key, created.clone());
                created
            };

            let target = if sd.is_oit {
                self.oit_draws.get_mut(sd.prepared_idx)
            } else {
                self.draws.get_mut(sd.prepared_idx)
            };
            if let Some(prepared) = target {
                prepared.set1_bind_group = Some((bind_group, dynamic_offsets));
            }
        }
    }

    /// Drop cached set-1 bind groups holding storage buffers `released` since the last frame, so
    /// a released buffer is freed once the frame that still draws it is done.
    pub fn forget_storage(&mut self, released: &[u64]) {
        if released.is_empty() {
            return;
        }
        self.warned_storage_conflicts
            .retain(|id| !released.contains(id));
        self.set1_bind_group_cache.retain(|key, _| {
            !key.resources
                .iter()
                .any(|r| matches!(r, Set1ResourceKey::Storage(id, ..) if released.contains(id)))
        });
    }

    /// Forget the conflict warnings of render target textures `released` since the last frame.
    pub fn forget_render_targets(&mut self, released: &[u64]) {
        self.warned_target_conflicts
            .retain(|texture| !released.contains(texture));
    }

    /// Whether any prepared standard-pass draw samples `TextureSource::WorldDepth` and therefore
    /// needs world depth stored before opening a read-only depth pass.
    pub fn reads_world_depth(&self) -> bool {
        self.draws.iter().any(|cmd| cmd.reads_world_depth)
    }

    /// Whether any prepared standard-pass draw in stages `first..=last` (excluding `skip_slots`)
    /// samples `TextureSource::WorldDepth`.
    pub fn stage_reads_world_depth(&self, first: usize, last: usize, skip_slots: &[usize]) -> bool {
        self.draws.iter().any(|cmd| {
            cmd.reads_world_depth
                && first <= cmd.stage
                && cmd.stage <= last
                && !skip_slots.contains(&cmd.slot)
        })
    }

    /// Whether any prepared standard-pass draw in `ranges` samples `TextureSource::WorldDepth`.
    pub fn prepared_draws_read_world_depth(&self, ranges: &[std::ops::Range<u32>]) -> bool {
        ranges.iter().any(|r| {
            let start = self
                .draws
                .partition_point(|cmd| cmd.submitted_index < r.start as usize);
            let end = self
                .draws
                .partition_point(|cmd| cmd.submitted_index < r.end as usize);
            self.draws[start..end]
                .iter()
                .any(|cmd| cmd.reads_world_depth)
        })
    }

    /// Draws the prepared runs and leading non-`WorldDepth` draws of stages `first..=last`
    /// (excluding `skip_slots`) into the already-open writable `pass`, returning how many
    /// matching draws were issued before the first `WorldDepth`-sampling draw.
    pub fn draw_excluding_prefix(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        indices: &wgpu::Buffer,
        first: usize,
        last: usize,
        skip_slots: &[usize],
    ) -> usize {
        self.draw_runs(
            pass,
            indices,
            false,
            self.runs
                .iter()
                .filter(|run| {
                    first <= run.stage && run.stage <= last && !skip_slots.contains(&run.slot)
                })
                .map(|run| (run, run.quads.clone())),
        );
        self.draw_writable_prefix(
            pass,
            indices,
            self.draws.iter().filter(|cmd| {
                first <= cmd.stage && cmd.stage <= last && !skip_slots.contains(&cmd.slot)
            }),
        )
    }

    /// Draws the remaining draws of stages `first..=last` (skipping `skip` already issued by
    /// [`Self::draw_excluding_prefix`]) in exact submission order, switching between read-only
    /// (`depth_ops: None`) and writable depth passes on `(view, depth)` as contiguous runs
    /// alternate.
    #[allow(clippy::too_many_arguments)]
    pub fn draw_excluding_split(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        view: &wgpu::TextureView,
        depth: &wgpu::TextureView,
        indices: &wgpu::Buffer,
        first: usize,
        last: usize,
        skip_slots: &[usize],
        skip: usize,
    ) {
        self.draw_split_rest(
            encoder,
            view,
            depth,
            indices,
            self.draws
                .iter()
                .filter(|cmd| {
                    first <= cmd.stage && cmd.stage <= last && !skip_slots.contains(&cmd.slot)
                })
                .skip(skip),
        );
    }

    /// Draws the prepared runs and unified draws of stages `first..=last` in the order queued,
    /// with `indices` (6 a quad, `u32`), skipping slots in `skip_slots` (e.g. slots bound to a
    /// shader pack's translated mod pass). Leaves the pass with its own pipeline, bind group 0
    /// and buffers.
    pub fn draw_excluding(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        indices: &wgpu::Buffer,
        first: usize,
        last: usize,
        skip_slots: &[usize],
    ) {
        self.draw_runs(
            pass,
            indices,
            false,
            self.runs
                .iter()
                .filter(|run| {
                    first <= run.stage && run.stage <= last && !skip_slots.contains(&run.slot)
                })
                .map(|run| (run, run.quads.clone())),
        );
        self.draw_prepared_cmds(
            pass,
            indices,
            false,
            self.draws.iter().filter(|cmd| {
                first <= cmd.stage && cmd.stage <= last && !skip_slots.contains(&cmd.slot)
            }),
        );
    }

    /// Same as [`Self::draw_excluding`], targeting a shader pack's `colortex0` + `depthtex0`
    /// prepared with [`Self::ensure_pack_pipelines`], skipping slots bound to translated pack
    /// programs.
    pub fn draw_pack(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        indices: &wgpu::Buffer,
        first: usize,
        last: usize,
        bound_slots: &[usize],
    ) {
        self.draw_runs(
            pass,
            indices,
            true,
            self.runs
                .iter()
                .filter(|run| {
                    first <= run.stage && run.stage <= last && !bound_slots.contains(&run.slot)
                })
                .map(|run| (run, run.quads.clone())),
        );
        self.draw_prepared_cmds(
            pass,
            indices,
            true,
            self.draws.iter().filter(|cmd| {
                first <= cmd.stage && cmd.stage <= last && !bound_slots.contains(&cmd.slot)
            }),
        );
    }

    fn prepared_runs(&self, range: std::ops::Range<u32>) -> &[Run] {
        let start = (range.start as usize).min(self.runs.len());
        let end = (range.end as usize).min(self.runs.len());
        &self.runs[start..end]
    }

    fn prepared_draws(&self, range: std::ops::Range<u32>) -> &[PreparedDraw] {
        let start = self
            .draws
            .partition_point(|cmd| cmd.submitted_index < range.start as usize);
        let end = self
            .draws
            .partition_point(|cmd| cmd.submitted_index < range.end as usize);
        &self.draws[start..end]
    }

    /// Whether the given prepared ranges have work not owned by a translated pack pass.
    pub(super) fn has_unbound_prepared(
        &self,
        runs: std::ops::Range<u32>,
        draws: std::ops::Range<u32>,
        bound_slots: &[usize],
    ) -> bool {
        (!runs.is_empty()
            && self
                .prepared_runs(runs)
                .iter()
                .any(|run| !bound_slots.contains(&run.slot)))
            || (!draws.is_empty()
                && self
                    .prepared_draws(draws)
                    .iter()
                    .any(|cmd| !bound_slots.contains(&cmd.slot)))
    }

    /// Draw prepared pipeline runs for the selected pass, omitting pack-bound slots in a pack pass.
    pub(super) fn draw_prepared_runs(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        indices: &wgpu::Buffer,
        ranges: impl IntoIterator<Item = std::ops::Range<u32>>,
        target: PreparedPass<'_>,
    ) {
        if let PreparedPass::Oit(stage) = target {
            self.draw_prepared_oit(pass, stage, indices, ranges);
            return;
        }
        let skip_slots = target.skip_slots();
        self.draw_runs(
            pass,
            indices,
            matches!(target, PreparedPass::Pack { .. }),
            ranges
                .into_iter()
                .flat_map(|range| self.prepared_runs(range))
                .filter(|run| !skip_slots.contains(&run.slot))
                .map(|run| (run, run.quads.clone())),
        );
    }

    /// Draw the leading non-`WorldDepth` draws in `ranges` into the already-open writable `pass`,
    /// returning how many were issued before the first `WorldDepth`-sampling draw.
    pub fn draw_prepared_draws_prefix(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        indices: &wgpu::Buffer,
        ranges: &[std::ops::Range<u32>],
    ) -> usize {
        self.draw_writable_prefix(
            pass,
            indices,
            ranges.iter().flat_map(|r| {
                let start = self
                    .draws
                    .partition_point(|cmd| cmd.submitted_index < r.start as usize);
                let end = self
                    .draws
                    .partition_point(|cmd| cmd.submitted_index < r.end as usize);
                self.draws[start..end].iter()
            }),
        )
    }

    /// Draw the remaining draws in `ranges` (skipping `skip` already issued by
    /// [`Self::draw_prepared_draws_prefix`]) in exact submission order across read-only and
    /// writable depth passes on `(view, depth)`.
    pub fn draw_prepared_draws_split(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        view: &wgpu::TextureView,
        depth: &wgpu::TextureView,
        indices: &wgpu::Buffer,
        ranges: &[std::ops::Range<u32>],
        skip: usize,
    ) {
        self.draw_split_rest(
            encoder,
            view,
            depth,
            indices,
            ranges
                .iter()
                .flat_map(|r| {
                    let start = self
                        .draws
                        .partition_point(|cmd| cmd.submitted_index < r.start as usize);
                    let end = self
                        .draws
                        .partition_point(|cmd| cmd.submitted_index < r.end as usize);
                    self.draws[start..end].iter()
                })
                .skip(skip),
        );
    }

    /// Draw prepared unified pipeline draws (`draw_mesh`, `draw_vertices`, `pipeline_quads`)
    /// for the selected pass, omitting pack-bound slots in a pack pass.
    pub(super) fn draw_prepared_draws(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        indices: &wgpu::Buffer,
        ranges: impl IntoIterator<Item = std::ops::Range<u32>>,
        target: PreparedPass<'_>,
    ) {
        if let PreparedPass::Oit(stage) = target {
            self.draw_prepared_oit_draws(pass, stage, indices, ranges);
            return;
        }
        let skip_slots = target.skip_slots();
        self.draw_prepared_cmds(
            pass,
            indices,
            matches!(target, PreparedPass::Pack { .. }),
            ranges
                .into_iter()
                .flat_map(|range| self.prepared_draws(range))
                .filter(|cmd| !skip_slots.contains(&cmd.slot)),
        );
    }

    /// Draw all OIT mod-pipeline runs and unified draws for `stage` into the active OIT pass.
    pub(super) fn draw_oit(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        stage: super::oit::Stage,
        indices: &wgpu::Buffer,
    ) {
        let stage_idx = stage as usize;
        self.draw_oit_runs(
            pass,
            stage_idx,
            indices,
            self.oit_runs.iter().filter(|run| run.stage == stage_idx),
        );
        self.draw_oit_cmds(
            pass,
            stage_idx,
            indices,
            self.oit_draws.iter().filter(|cmd| cmd.stage == stage_idx),
        );
    }

    /// Draw prepared OIT mod-pipeline runs for `stage` and the given run-index ranges.
    pub(super) fn draw_prepared_oit(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        stage: super::oit::Stage,
        indices: &wgpu::Buffer,
        ranges: impl IntoIterator<Item = std::ops::Range<u32>>,
    ) {
        let stage_idx = stage as usize;
        self.draw_oit_runs(
            pass,
            stage_idx,
            indices,
            ranges.into_iter().flat_map(|r| {
                let start = (r.start as usize).min(self.oit_runs.len());
                let end = (r.end as usize).min(self.oit_runs.len());
                self.oit_runs[start..end]
                    .iter()
                    .filter(move |run| run.stage == stage_idx)
            }),
        );
    }

    /// Draw prepared OIT unified pipeline draws for `stage` and the given draw-index ranges.
    pub(super) fn draw_prepared_oit_draws(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        stage: super::oit::Stage,
        indices: &wgpu::Buffer,
        ranges: impl IntoIterator<Item = std::ops::Range<u32>>,
    ) {
        let stage_idx = stage as usize;
        self.draw_oit_cmds(
            pass,
            stage_idx,
            indices,
            ranges.into_iter().flat_map(|r| {
                let start = self
                    .oit_draws
                    .partition_point(|cmd| cmd.submitted_index < r.start as usize);
                let end = self
                    .oit_draws
                    .partition_point(|cmd| cmd.submitted_index < r.end as usize);
                self.oit_draws[start..end]
                    .iter()
                    .filter(move |cmd| cmd.stage == stage_idx)
            }),
        );
    }

    fn issue_prepared_draw(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        quad_indices: &wgpu::Buffer,
        transient_vbuf: Option<&wgpu::Buffer>,
        transient_ibuf: Option<&wgpu::Buffer>,
        cmd: &PreparedDraw,
    ) {
        if cmd.draw.instances.is_empty() {
            return;
        }
        match &cmd.source {
            PipelineDrawSource::Transient {
                vertex_ranges,
                index_range,
            } => {
                let Some(vbuf) = transient_vbuf else {
                    return;
                };
                for (binding, range) in vertex_ranges {
                    if range.end > vbuf.size() {
                        return;
                    }
                    pass.set_vertex_buffer(*binding, vbuf.slice(range.clone()));
                }
                match &cmd.draw.range {
                    DrawRange::Vertices(range) => {
                        self.issue_non_indexed_draw(
                            pass,
                            quad_indices,
                            cmd.topology,
                            range.clone(),
                            cmd.draw.instances.clone(),
                        );
                    }
                    DrawRange::Indexed {
                        indices,
                        base_vertex,
                    } => {
                        let Some((fmt, ib_range)) = index_range else {
                            return;
                        };
                        let Some(ibuf) = transient_ibuf else {
                            return;
                        };
                        if ib_range.end > ibuf.size() || indices.is_empty() {
                            return;
                        }
                        pass.set_index_buffer(
                            ibuf.slice(ib_range.clone()),
                            wgpu_index_format(*fmt),
                        );
                        pass.draw_indexed(
                            indices.clone(),
                            *base_vertex,
                            cmd.draw.instances.clone(),
                        );
                    }
                    _ => {}
                }
            }
            PipelineDrawSource::Retained(resolved) => {
                for (binding, buf, len) in &resolved.vertex_buffers {
                    pass.set_vertex_buffer(*binding, buf.slice(0..*len));
                }
                match &cmd.draw.range {
                    DrawRange::Vertices(range) => {
                        self.issue_non_indexed_draw(
                            pass,
                            quad_indices,
                            cmd.topology,
                            range.clone(),
                            cmd.draw.instances.clone(),
                        );
                    }
                    DrawRange::Indexed {
                        indices,
                        base_vertex,
                    } => {
                        let Some((ibuf, fmt)) = &resolved.index_buffer else {
                            return;
                        };
                        if indices.is_empty() {
                            return;
                        }
                        pass.set_index_buffer(ibuf.slice(..), *fmt);
                        pass.draw_indexed(
                            indices.clone(),
                            *base_vertex,
                            cmd.draw.instances.clone(),
                        );
                    }
                    _ => {}
                }
            }
            PipelineDrawSource::Indirect(indirect) => {
                if indirect.max_count == 0 {
                    return;
                }
                if let Some(mesh) = &indirect.mesh {
                    for (binding, buf, len) in &mesh.vertex_buffers {
                        pass.set_vertex_buffer(*binding, buf.slice(0..*len));
                    }
                }
                // `max_count` commands, those the mod's culling shader zeroed
                // (`instance_count = 0`) drawing nothing; or, with a count buffer (checked
                // against `RenderCapabilities::multi_draw_indirect_count`), as many as the GPU wrote.
                if let Some((ibuf, fmt)) = indirect.index_buffer() {
                    pass.set_index_buffer(ibuf.slice(..), *fmt);
                }
                match (indirect.index_buffer(), &indirect.count) {
                    (Some(_), None) => pass.multi_draw_indexed_indirect(
                        &indirect.buffer,
                        indirect.offset,
                        indirect.max_count,
                    ),
                    (None, None) => pass.multi_draw_indirect(
                        &indirect.buffer,
                        indirect.offset,
                        indirect.max_count,
                    ),
                    (Some(_), Some((count, at))) => pass.multi_draw_indexed_indirect_count(
                        &indirect.buffer,
                        indirect.offset,
                        count,
                        *at,
                        indirect.max_count,
                    ),
                    (None, Some((count, at))) => pass.multi_draw_indirect_count(
                        &indirect.buffer,
                        indirect.offset,
                        count,
                        *at,
                        indirect.max_count,
                    ),
                }
            }
        }
    }

    fn issue_non_indexed_draw(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        quad_indices: &wgpu::Buffer,
        topology: PrimitiveTopology,
        range: std::ops::Range<u32>,
        instances: std::ops::Range<u32>,
    ) {
        let count = range.end.saturating_sub(range.start);
        if count == 0 || instances.is_empty() {
            return;
        }
        match topology {
            PrimitiveTopology::Quads | PrimitiveTopology::Lines => {
                let quads = count / 4;
                if quads == 0 {
                    return;
                }
                pass.set_index_buffer(quad_indices.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..quads * 6, range.start as i32, instances);
            }
            PrimitiveTopology::TriangleFan => {
                let Some(fan_buf) = self.fan_indices.buffer() else {
                    return;
                };
                let idx_count = topology.index_count(count);
                if idx_count == 0 {
                    return;
                }
                pass.set_index_buffer(fan_buf.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..idx_count, range.start as i32, instances);
            }
            _ => {
                pass.draw(range, instances);
            }
        }
    }

    fn draw_writable_prefix<'a>(
        &'a self,
        pass: &mut wgpu::RenderPass<'_>,
        indices: &wgpu::Buffer,
        cmds: impl Iterator<Item = &'a PreparedDraw>,
    ) -> usize {
        let mut issued = 0;
        self.draw_prepared_cmds(
            pass,
            indices,
            false,
            cmds.take_while(|cmd| {
                if cmd.reads_world_depth {
                    false
                } else {
                    issued += 1;
                    true
                }
            }),
        );
        issued
    }

    fn draw_split_rest<'a>(
        &'a self,
        encoder: &mut wgpu::CommandEncoder,
        view: &wgpu::TextureView,
        depth: &wgpu::TextureView,
        indices: &wgpu::Buffer,
        mut cmds: impl Iterator<Item = &'a PreparedDraw>,
    ) {
        let Some(mut first) = cmds.next() else {
            return;
        };
        loop {
            let read_only = first.reads_world_depth;
            let mut pass = Self::open_world_depth_pass(encoder, view, depth, read_only);
            self.draw_prepared_cmds(&mut pass, indices, false, std::iter::once(first));
            let mut next_group = None;
            for cmd in cmds.by_ref() {
                if cmd.reads_world_depth != read_only {
                    next_group = Some(cmd);
                    break;
                }
                self.draw_prepared_cmds(&mut pass, indices, false, std::iter::once(cmd));
            }
            drop(pass);
            match next_group {
                Some(cmd) => first = cmd,
                None => break,
            }
        }
    }

    /// Open a world render pass over `(view, depth)` whose depth attachment is read-only
    /// (`depth_ops: None`, vanilla `OitRenderPassProvider`) when `read_only` is true, or writable
    /// (`LoadOp::Load`, `StoreOp::Store`) when resuming ordinary draws.
    fn open_world_depth_pass<'e>(
        encoder: &'e mut wgpu::CommandEncoder,
        view: &wgpu::TextureView,
        depth: &wgpu::TextureView,
        read_only: bool,
    ) -> wgpu::RenderPass<'e> {
        encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some(if read_only {
                "Sunrise world, read-only depth"
            } else {
                "Sunrise world, after read-only depth"
            }),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: depth,
                depth_ops: (!read_only).then_some(wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            ..Default::default()
        })
    }

    fn draw_prepared_cmds<'a>(
        &'a self,
        pass: &mut wgpu::RenderPass<'_>,
        indices: &wgpu::Buffer,
        pack: bool,
        cmds: impl Iterator<Item = &'a PreparedDraw>,
    ) {
        let pack_target = if pack {
            let Some((format, rgb)) = self.pack_target else {
                return;
            };
            Some(VariantTarget::Pack(format, rgb))
        } else {
            None
        };
        for cmd in cmds {
            let compiled = match &pack_target {
                Some(target) => self
                    .variants
                    .get(&(cmd.slot, cmd.sampler_layout.clone(), target.clone()))
                    .and_then(|entry| entry.as_ref().ok()),
                None => compiled_in(
                    &self.pipelines,
                    &self.variants,
                    cmd.slot,
                    &cmd.sampler_layout,
                ),
            };
            let Some(compiled) = compiled else { continue };
            if compiled.source.recipe.state.depth_stencil.is_none() {
                continue;
            }
            let pipeline = if cmd.modifier_stack == ModifierStacks::EMPTY {
                &compiled.pipeline
            } else {
                match compiled.variants.get(cmd.modifier_stack as usize) {
                    Some(Some(CachedVariant::Base)) => &compiled.pipeline,
                    Some(Some(CachedVariant::Compiled(p))) => p,
                    _ => continue,
                }
            };
            pass.set_pipeline(pipeline);
            pass.set_bind_group(
                0,
                if pack {
                    &compiled.source.pack_group
                } else {
                    &compiled.source.group
                },
                &[],
            );
            if compiled.set1_layout.is_some() {
                let Some((ref bg, ref offsets)) = cmd.set1_bind_group else {
                    continue;
                };
                pass.set_bind_group(1, bg, offsets);
            }
            self.issue_prepared_draw(
                pass,
                indices,
                self.transient_vbuf.as_ref(),
                self.transient_ibuf.as_ref(),
                cmd,
            );
        }
    }

    fn draw_oit_cmds<'a>(
        &'a self,
        pass: &mut wgpu::RenderPass<'_>,
        stage_idx: usize,
        indices: &wgpu::Buffer,
        cmds: impl Iterator<Item = &'a PreparedDraw>,
    ) {
        for cmd in cmds {
            let Some(compiled) = compiled_in(
                &self.pipelines,
                &self.variants,
                cmd.slot,
                &cmd.sampler_layout,
            ) else {
                continue;
            };
            let Some(oit) = &compiled.oit else {
                continue;
            };
            let oit_stage = &oit.stages[stage_idx];
            let Some(group) = &oit_stage.group else {
                continue;
            };
            let pipeline = if cmd.modifier_stack == ModifierStacks::EMPTY {
                &oit_stage.pipeline
            } else {
                match oit_stage.variants.get(cmd.modifier_stack as usize) {
                    Some(Some(CachedVariant::Base)) => &oit_stage.pipeline,
                    Some(Some(CachedVariant::Compiled(p))) => p,
                    _ => continue,
                }
            };
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, group, &[]);
            if compiled.set1_layout.is_some() {
                let Some((ref bg, ref offsets)) = cmd.set1_bind_group else {
                    continue;
                };
                pass.set_bind_group(1, bg, offsets);
            }
            self.issue_prepared_draw(
                pass,
                indices,
                self.oit_transient_vbuf.as_ref(),
                self.oit_transient_ibuf.as_ref(),
                cmd,
            );
        }
    }

    fn draw_oit_runs<'a>(
        &'a self,
        pass: &mut wgpu::RenderPass<'_>,
        stage_idx: usize,
        indices: &wgpu::Buffer,
        runs: impl Iterator<Item = &'a Run>,
    ) {
        let mut index_bound = false;
        for run in runs {
            let Some(Some(compiled)) = self.pipelines.get(run.slot) else {
                continue;
            };
            let Some(oit) = &compiled.oit else {
                continue;
            };
            let oit_stage = &oit.stages[stage_idx];
            let Some(group) = &oit_stage.group else {
                continue;
            };
            let pipeline = if run.modifier_stack == ModifierStacks::EMPTY {
                &oit_stage.pipeline
            } else {
                match oit_stage.variants.get(run.modifier_stack as usize) {
                    Some(Some(CachedVariant::Base)) => &oit_stage.pipeline,
                    Some(Some(CachedVariant::Compiled(p))) => p,
                    _ => continue,
                }
            };
            if !index_bound {
                index_bound = true;
                pass.set_index_buffer(indices.slice(..), wgpu::IndexFormat::Uint32);
            }
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, group, &[]);
            for (buffer, range) in self.oit_vertices.ranges(0, run.quads.clone()) {
                pass.set_vertex_buffer(0, buffer.slice(..));
                pass.draw_indexed(range.start * 6..range.end * 6, 0, 0..1);
            }
        }
    }

    fn draw_runs<'a>(
        &'a self,
        pass: &mut wgpu::RenderPass<'_>,
        indices: &wgpu::Buffer,
        pack: bool,
        runs: impl Iterator<Item = (&'a Run, std::ops::Range<u32>)>,
    ) {
        let pack_target = if pack {
            let Some((format, rgb)) = self.pack_target else {
                return;
            };
            Some(VariantTarget::Pack(format, rgb))
        } else {
            None
        };
        let mut index_bound = false;
        for (run, range) in runs {
            let compiled = match &pack_target {
                Some(target) => self
                    .variants
                    .get(&(run.slot, SamplerLayout::EMPTY, target.clone()))
                    .and_then(|entry| entry.as_ref().ok()),
                None => self.pipelines.get(run.slot).and_then(Option::as_ref),
            };
            let Some(compiled) = compiled else { continue };
            if compiled.source.recipe.state.depth_stencil.is_none() {
                continue;
            }
            let pipeline = if run.modifier_stack == ModifierStacks::EMPTY {
                &compiled.pipeline
            } else {
                match compiled.variants.get(run.modifier_stack as usize) {
                    Some(Some(CachedVariant::Base)) => &compiled.pipeline,
                    Some(Some(CachedVariant::Compiled(p))) => p,
                    _ => continue,
                }
            };
            if !index_bound {
                index_bound = true;
                pass.set_index_buffer(indices.slice(..), wgpu::IndexFormat::Uint32);
            }
            pass.set_pipeline(pipeline);
            pass.set_bind_group(
                0,
                if pack {
                    &compiled.source.pack_group
                } else {
                    &compiled.source.group
                },
                &[],
            );
            for (buffer, range) in self.vertices.ranges(0, range) {
                pass.set_vertex_buffer(0, buffer.slice(..));
                pass.draw_indexed(range.start * 6..range.end * 6, 0, 0..1);
            }
        }
    }
}

fn gpu_refusal(built: &Built, error: &str) -> String {
    format!(
        "render pipeline {} from mod {} refused by the GPU: {error}",
        built.id, built.owner
    )
}

impl super::Renderer {
    /// Builds the mods' pipelines of the load this renderer is made for, before the load is
    /// published: the required ones the GPU refuses are the error, by id. A textured pipeline's
    /// sprite is looked up with `sprite` (its `[u0, v0, u1, v1]` in the entity atlas, the 2D
    /// atlas world primitives draw from), and a sprite it doesn't find refuses that pipeline.
    pub fn build_mod_pipelines(
        &mut self,
        set: &Pipelines,
        sprite: &dyn Fn(&str) -> Option<[f32; 4]>,
    ) -> Result<(), String> {
        let textures = TextureSources {
            atlas: &self.entity_atlas,
            sprite,
        };
        let capabilities = self.mod_gpu.render_capabilities();
        let pipelines = &mut self
            .world_primitives
            .as_mut()
            .expect("world primitives")
            .pipelines;
        pipelines.set_capabilities(&capabilities);
        pipelines.build(&self.device, set, Some(textures))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Vanilla's sampler blends between mip levels whenever `maxLod` is above 0.25, an absent
    /// one being 1000 (#6000): `Sampler::new` blends. A texture wgpu cannot filter (a depth
    /// target) is read with nearest filtering whatever the sampler says (#6033).
    #[test]
    fn a_sampler_without_max_lod_blends_mip_levels_as_vanilla_does() {
        let default = Sampler::new(SamplerFilter::Nearest, SamplerAddress::ClampToEdge);
        assert!(blends_mip_levels(&default) && sampler_filtering(&default));
        let mut base_only = default;
        base_only.max_lod = Some(0.0);
        assert!(!blends_mip_levels(&base_only) && !sampler_filtering(&base_only));
        let mut quarter = default;
        quarter.max_lod = Some(0.25);
        assert!(!blends_mip_levels(&quarter));
        quarter.max_lod = Some(0.5);
        assert!(blends_mip_levels(&quarter));
        let depth = mod_api::RenderTargetDescriptor::rgba8(
            [4, 4],
            mod_api::RenderTargetDepth::Depth32Float,
        );
        let mut layout = SamplerLayout::default();
        layout.record(
            0,
            &default,
            super::super::mod_render_targets::wgpu_depth_format(depth.depth).unwrap(),
            true,
        );
        assert_eq!(layout.ordinals(), [0]);
    }
    use crate::gfx::world_primitives::{self, STAGES, WorldPrimitives};
    use glam::Vec3;
    use mod_api::{BlendFunction, DepthCompare, Identifier, RenderPipeline, WorldDepth};

    const SIZE: u32 = 64;
    /// The frame before any quad: a quarter grey, stored as 64 in 8 bits.
    const CLEAR: f64 = 64.0 / 255.0;

    #[test]
    fn pack_clip_matrix_flips_y_and_inverts_reversed_z_depth() {
        let proj = Mat4::perspective_infinite_reverse_rh(std::f32::consts::FRAC_PI_2, 1.5, 0.05);
        let adapted = pack_clip_matrix(proj);
        for p in [
            glam::Vec4::new(0.0, 0.0, -0.05, 1.0),
            glam::Vec4::new(0.25, 0.5, -4.0, 1.0),
            glam::Vec4::new(-1.5, -0.75, -20.0, 1.0),
        ] {
            let orig = proj * p;
            let got = adapted * p;
            assert_eq!(got.x, orig.x);
            assert_eq!(got.y, -orig.y);
            assert_eq!(got.z, orig.w - orig.z);
            assert_eq!(got.w, orig.w);
        }
    }

    const VSH: &str = "\
#version 330
#include <minecraft:dynamictransforms.glsl>
#include <minecraft:projection.glsl>
in vec3 Position;
in vec2 UV0;
in vec4 Color;
out vec2 texCoord0;
out vec4 vertexColor;
void main() {
    gl_Position = ProjMat * ModelViewMat * vec4(Position, 1.0);
    texCoord0 = UV0;
    vertexColor = Color;
}
";

    const PASS: &str = "\
#version 330
#include <minecraft:dynamictransforms.glsl>
in vec2 texCoord0;
in vec4 vertexColor;
out vec4 fragColor;
void main() {
    fragColor = vertexColor * ColorModulator;
}
";

    const HALF_GAMMA: &str = "\
#version 330
in vec2 texCoord0;
in vec4 vertexColor;
out vec4 fragColor;
void main() {
    // Vanilla gamma value 0.7297 (~186/255), whose linear 2.2 power is 0.5.
    fragColor = vec4(vec3(0.72974), 1.0);
}
";

    const INPUTS: &str = "\
#version 330
#include <minecraft:globals.glsl>
in vec2 texCoord0;
in vec4 vertexColor;
out vec4 fragColor;
void main() {
    fragColor = vec4(step(0.5, texCoord0.x), GameTime, step(0.5, texCoord0.y), 1.0);
}
";

    fn stub_asset(file: &str) -> Option<String> {
        let rest = file.strip_prefix("assets/")?;
        let (ns, path) = rest.split_once('/')?;
        sunrise_mod_pipeline::stubs::read(&format!("{ns}:{path}")).map(str::to_owned)
    }

    fn device() -> Option<(wgpu::Device, wgpu::Queue)> {
        let (_, device, queue) = crate::gfx::test_gpu::optional_device()?;
        Some((device, queue))
    }

    /// The load of `generation` with one optional pipeline for each fragment shader, in slot order.
    fn load(generation: u64, shaders: &[(&str, PipelineBlend, WorldDepth)]) -> Pipelines {
        let registered: Vec<_> = (0..shaders.len())
            .map(|slot| {
                let id = Identifier::parse(&format!("test:p{slot}")).unwrap();
                let shader = Identifier::parse(&format!("test:core/p{slot}")).unwrap();
                RenderPipeline::new(id, shader, shaders[slot].1, shaders[slot].2).optional()
            })
            .collect();
        Pipelines::build(&registered, generation, |file| {
            if let Some(rest) = file.strip_prefix("assets/test/shaders/core/p") {
                if rest.ends_with(".vsh") {
                    return Ok(Some(VSH.to_owned()));
                }
                if let Some(num) = rest.strip_suffix(".fsh") {
                    let slot: usize = num.parse().unwrap();
                    return Ok(Some(shaders[slot].0.to_owned()));
                }
            }
            Ok(stub_asset(file))
        })
        .unwrap()
    }

    fn view_proj() -> Mat4 {
        crate::gfx::world_projection(70.0, 1.0, 512.0)
    }

    /// A quad facing the camera `z` blocks ahead of it, over the pixels `x0..x1`, `y0..y1` of
    /// the frame (y down), its texture coordinates spanning 0 to 1 and `color` as stored bytes.
    fn quad([x0, y0, x1, y1]: [f32; 4], z: f32, color: [u8; 4]) -> [PipelineVertex; 4] {
        let depth = view_proj().project_point3(Vec3::new(0.0, 0.0, -z)).z;
        let inverse = view_proj().inverse();
        let at = |x: f32, y: f32, uv: [f32; 2]| PipelineVertex {
            pos: inverse
                .project_point3(Vec3::new(
                    x / SIZE as f32 * 2.0 - 1.0,
                    1.0 - y / SIZE as f32 * 2.0,
                    depth,
                ))
                .to_array(),
            uv,
            color: u32::from_le_bytes(color),
        };
        [
            at(x0, y0, [0.0, 0.0]),
            at(x0, y1, [0.0, 1.0]),
            at(x1, y1, [1.0, 1.0]),
            at(x1, y0, [1.0, 0.0]),
        ]
    }

    /// Draws `batch`'s stages up to `last` over the cleared frame of `format`, behind a wall
    /// 5 blocks ahead, and reads the frame back as RGBA floats.
    fn render(
        gpu: &(wgpu::Device, wgpu::Queue),
        primitives: &mut WorldPrimitives,
        format: wgpu::TextureFormat,
        batch: Batch<'_>,
        last: usize,
    ) -> impl Fn(u32, u32) -> [f32; 4] + use<> {
        render_target(gpu, primitives, format, batch, last, false)
    }

    fn render_target(
        (device, queue): &(wgpu::Device, wgpu::Queue),
        primitives: &mut WorldPrimitives,
        format: wgpu::TextureFormat,
        batch: Batch<'_>,
        last: usize,
        pack: bool,
    ) -> impl Fn(u32, u32) -> [f32; 4] + use<> {
        let texture = |size: u32, format, usage| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: None,
                size: wgpu::Extent3d {
                    width: size,
                    height: size,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage,
                view_formats: &[],
            })
        };
        let color = texture(
            SIZE,
            format,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        );
        let depth = texture(
            SIZE,
            wgpu::TextureFormat::Depth32Float,
            wgpu::TextureUsages::RENDER_ATTACHMENT,
        );
        let atlas = texture(
            1,
            wgpu::TextureFormat::Rgba8Unorm,
            wgpu::TextureUsages::TEXTURE_BINDING,
        );
        let wall = view_proj().project_point3(Vec3::new(0.0, 0.0, -5.0)).z;
        if let Some(set) = batch.set {
            primitives.pipelines.build(device, set, None).unwrap();
        }
        let drawn = primitives.prepare(
            device,
            &mut |buffer, bytes| queue.write_buffer(buffer, 0, bytes),
            &world_primitives::Batch {
                pipelines: batch,
                ..Default::default()
            },
            view_proj(),
            [SIZE, SIZE],
            &atlas.create_view(&Default::default()),
            None,
        );
        let mut encoder = device.create_command_encoder(&Default::default());
        {
            let color_view = color.create_view(&Default::default());
            let depth_view = depth.create_view(&Default::default());
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: None,
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &color_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: CLEAR,
                            g: CLEAR,
                            b: CLEAR,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wall),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                ..Default::default()
            });
            if drawn && !pack {
                primitives.draw_through(&mut pass, &mut 0, last);
            }
        }
        if pack {
            primitives.prepare_pack_pass(device, format, false, false);
            assert!(primitives.draw_pack_custom(
                &mut encoder,
                &color.create_view(&Default::default()),
                &depth.create_view(&Default::default()),
            ));
        }
        let float = format == wgpu::TextureFormat::Rgba16Float;
        let texel = if float { 8 } else { 4 };
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: u64::from(SIZE * SIZE * texel),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        encoder.copy_texture_to_buffer(
            color.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(SIZE * texel),
                    rows_per_image: None,
                },
            },
            color.size(),
        );
        queue.submit([encoder.finish()]);
        readback.slice(..).map_async(wgpu::MapMode::Read, |_| {});
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        let bytes = readback.slice(..).get_mapped_range().unwrap().to_vec();
        move |x: u32, y: u32| {
            let i = ((y * SIZE + x) * texel) as usize;
            std::array::from_fn(|channel| {
                if float {
                    let at = i + channel * 2;
                    half::f16::from_le_bytes([bytes[at], bytes[at + 1]]).to_f32()
                } else {
                    f32::from(bytes[i + channel])
                }
            })
        }
    }

    fn assert_near(got: [f32; 4], expected: [f32; 4], tolerance: f32, what: &str) {
        assert!(
            got.iter()
                .zip(expected)
                .all(|(got, expected)| (got - expected).abs() <= tolerance),
            "{what}: got {got:?}, expected {expected:?}"
        );
    }

    #[test]
    #[ignore = "daily"]
    fn gpu_validation_failure_is_contained_and_invalidates_handle() {
        let _gpu = crate::gfx::test_gpu::lock();
        let Some((device, _)) = device() else {
            return;
        };
        let set = load(
            41,
            &[(PASS, PipelineBlend::Translucent, WorldDepth::Tested)],
        );
        let id = Identifier::parse("test:p0").unwrap();
        let handle = set.resolve(&id).unwrap();
        let renderer = ModPipelines::new(&device, (wgpu::TextureFormat::Rgba8Unorm, false));
        let mut broken = set.built()[0].as_deref().unwrap().clone();
        broken.owner = "owning_mod".into();
        assert_eq!(
            gpu_refusal(&broken, "compiler refusal"),
            "render pipeline test:p0 from mod owning_mod refused by the GPU: compiler refusal"
        );
        // An empty naga module has no `main` entry point, so pipeline creation fails cleanly.
        broken.program = std::sync::Arc::new(crate::core_shader::validated(
            broken.program.vertex.clone(),
            naga::Module::default(),
            Vec::new(),
        ));
        let result = renderer
            .pipeline_source(&device, &Arc::new(broken), None)
            .and_then(|source| {
                renderer
                    .compile(
                        &device,
                        source,
                        &VariantTarget::World,
                        &SamplerLayout::EMPTY,
                        None,
                    )
                    .map_err(BuildError::from)
            });
        assert!(result.is_err());
        assert!(set.refuse(0, Refusal::Invalid));
        assert_eq!(set.resolve(&id), Err(mod_api::RenderPipelineError::Invalid));
        assert_eq!(set.slot(handle), Err(mod_api::RenderPipelineError::Invalid));
    }

    #[test]
    #[ignore = "daily"]
    fn optional_gpu_refusal_is_published_only_once() {
        let _gpu = crate::gfx::test_gpu::lock();
        let Some((device, _)) = device() else {
            return;
        };
        let set = load(
            41,
            &[(PASS, PipelineBlend::Translucent, WorldDepth::Tested)],
        );
        let id = Identifier::parse("test:p0").unwrap();
        let handle = set.resolve(&id).unwrap();
        let mut renderer = ModPipelines::new(&device, (wgpu::TextureFormat::Rgba8Unorm, false));
        renderer
            .build_with(&set, |_, _| Err("injected refusal".into()))
            .unwrap();
        assert_eq!(renderer.generation, 41);
        assert!(renderer.pipelines[0].is_none());
        assert_eq!(set.slot(handle), Err(mod_api::RenderPipelineError::Invalid));
        assert!(!set.refuse(0, Refusal::Invalid));
        renderer
            .build_with(&set, |_, _| panic!("already compiled"))
            .unwrap();
    }

    #[test]
    #[ignore = "daily"]
    fn a_later_successful_build_makes_a_refused_slot_usable_again() {
        let _gpu = crate::gfx::test_gpu::lock();
        let Some((device, _)) = device() else {
            return;
        };
        let set = load(
            41,
            &[(PASS, PipelineBlend::Translucent, WorldDepth::Tested)],
        );
        let id = Identifier::parse("test:p0").unwrap();
        let handle = set.resolve(&id).unwrap();
        let mut refusing = ModPipelines::new(&device, (wgpu::TextureFormat::Rgba8Unorm, false));
        refusing
            .build_with(&set, |_, _| Err("injected refusal".into()))
            .unwrap();
        assert_eq!(set.slot(handle), Err(mod_api::RenderPipelineError::Invalid));
        let mut recovered = ModPipelines::new(&device, (wgpu::TextureFormat::Rgba8Unorm, false));
        recovered.build(&device, &set, None).unwrap();
        assert!(recovered.pipelines[0].is_some());
        assert_eq!(set.slot(handle), Ok(0));
        assert!(set.resolve(&id).is_ok());
    }

    #[test]
    #[ignore = "daily"]
    fn required_gpu_failure_does_not_publish_and_mixed_success_does() {
        let _gpu = crate::gfx::test_gpu::lock();
        let Some((device, _)) = device() else {
            return;
        };
        let set = load(
            41,
            &[(PASS, PipelineBlend::Translucent, WorldDepth::Tested)],
        );
        let mut renderer = ModPipelines::new(&device, (wgpu::TextureFormat::Rgba8Unorm, false));
        renderer.build(&device, &set, None).unwrap();
        let shaders = [
            (PASS, PipelineBlend::Translucent, WorldDepth::Tested),
            (PASS, PipelineBlend::Translucent, WorldDepth::Tested),
        ];
        let mut registered: Vec<_> = (0..shaders.len())
            .map(|slot| {
                let id = Identifier::parse(&format!("test:p{slot}")).unwrap();
                let shader = Identifier::parse(&format!("test:core/p{slot}")).unwrap();
                RenderPipeline::new(id, shader, shaders[slot].1, shaders[slot].2).optional()
            })
            .collect();
        registered[0].optional = false;
        let next = Pipelines::build(&registered, 42, |file| {
            if file.ends_with(".vsh") {
                Ok(Some(VSH.to_owned()))
            } else if file.ends_with(".fsh") {
                Ok(Some(PASS.to_owned()))
            } else {
                Ok(stub_asset(file))
            }
        })
        .unwrap();
        let id = Identifier::parse("test:p0").unwrap();
        let handle = next.resolve(&id).unwrap();
        let error = renderer
            .build_with(&next, |_, _| Err("injected refusal".into()))
            .unwrap_err();
        assert_eq!(
            error,
            "required render pipeline test:p0 from mod  refused by the GPU: injected refusal"
        );
        assert_eq!(renderer.generation, 41);
        assert_eq!(renderer.pipelines.len(), 1);
        assert_eq!(next.slot(handle), Ok(0));
        assert!(next.resolve(&Identifier::parse("test:p1").unwrap()).is_ok());
        renderer.build(&device, &next, None).unwrap();
        assert_eq!(renderer.generation, 42);
        assert_eq!(renderer.pipelines.len(), 2);
        assert!(renderer.pipelines.iter().all(Option::is_some));
    }

    #[test]
    #[ignore = "daily"]
    fn pipelines_convert_colour_both_ways_and_keep_their_blend_depth_and_stage() {
        let _gpu = crate::gfx::test_gpu::lock();
        let Some(gpu) = device() else {
            eprintln!("no GPU adapter: skipped");
            return;
        };
        use PipelineBlend::{Additive, Translucent};
        use WorldDepth::{Tested, ThroughWalls};
        let set = load(
            3,
            &[
                (PASS, Translucent, Tested),
                (HALF_GAMMA, Translucent, ThroughWalls),
                // Refused by the load: the pipelines after it still draw.
                ("void nothing() {}", Translucent, Tested),
                (HALF_GAMMA, Additive, Tested),
                (INPUTS, Translucent, Tested),
            ],
        );
        assert!(set.built()[2].is_none());
        let grey = [128, 128, 128, 255];
        let white = [255; 4];
        let vertices = [
            // Top left: a grey quad returned as it came, in front of the wall and behind it.
            quad([0.0, 0.0, 16.0, 32.0], 2.0, grey),
            quad([16.0, 0.0, 32.0, 32.0], 10.0, grey),
            // Top right: 0.72974 (~186) through walls, at a later stage.
            quad([32.0, 0.0, 64.0, 32.0], 10.0, white),
            // Bottom left: 0.72974 (~186) added to the frame.
            quad([0.0, 32.0, 32.0, 64.0], 2.0, white),
            // Bottom right: the texture coordinate and Globals.GameTime.
            quad([32.0, 32.0, 64.0, 64.0], 2.0, white),
        ]
        .concat();
        let run = |slot, stage, quads| PipelineRun {
            slot,
            stage,
            quads,
            modifier_stack: 0,
        };
        let runs = [run(0, 0, 2), run(1, 2, 1), run(3, 0, 1), run(4, 0, 1)];
        let batch = Batch {
            set: Some(&set),
            vertices: &vertices,
            runs: &runs,
            time: 1.0,
            ..Default::default()
        };
        let format = wgpu::TextureFormat::Rgba8Unorm;
        let mut primitives = WorldPrimitives::new(&gpu.0, (format, false));

        let at = render(&gpu, &mut primitives, format, batch, STAGES - 1);
        let clear = [64.0, 64.0, 64.0, 255.0];
        assert_near(
            at(8, 16),
            [128.0, 128.0, 128.0, 255.0],
            1.0,
            "vanilla gamma colour passes through unchanged",
        );
        assert_near(
            at(24, 16),
            clear,
            0.0,
            "depth tested: hidden behind the wall",
        );
        assert_near(
            at(48, 16),
            [186.0, 186.0, 186.0, 255.0],
            1.0,
            "gamma 0.72974 through walls",
        );
        assert_near(
            at(16, 48),
            [250.0, 250.0, 250.0, 255.0],
            1.0,
            "additive: 186 over the frame's 64",
        );
        assert_near(
            at(40, 40),
            [0.0, 255.0, 0.0, 255.0],
            0.0,
            "the quad's first quarter: uv under 0.5, GameTime 1",
        );
        assert_near(
            at(56, 56),
            [255.0, 255.0, 255.0, 255.0],
            0.0,
            "the quad's last quarter: uv over 0.5",
        );

        // Only the first stage: the later stage's quad is not drawn yet.
        let at = render(&gpu, &mut primitives, format, batch, 0);
        assert_near(
            at(48, 16),
            clear,
            0.0,
            "a later stage's quad waits for its stage",
        );
        assert_near(
            at(8, 16),
            [128.0, 128.0, 128.0, 255.0],
            1.0,
            "the first stage draws",
        );
    }

    /// In a float frame light above white survives as written (gamma encoding), in a linear-light
    /// float frame `fragment_finish` decodes gamma 0.72974 to linear 0.5 (and sanitizes negative
    /// RGB to 0 while preserving alpha), and a new load's shader replaces the old one's pipeline.
    #[test]
    #[ignore = "daily"]
    fn a_float_frame_keeps_light_above_white_and_a_reload_builds_the_new_shader() {
        let _gpu = crate::gfx::test_gpu::lock();
        let Some(gpu) = device() else {
            eprintln!("no GPU adapter: skipped");
            return;
        };
        let bright = "\
#version 330
out vec4 fragColor;
void main() { fragColor = vec4(1.37035, 1.0, 0.0, 1.0); }
";
        let green = "\
#version 330
out vec4 fragColor;
void main() { fragColor = vec4(0.0, 1.0, 0.0, 1.0); }
";
        let half_and_neg = "\
#version 330
out vec4 fragColor;
void main() { fragColor = vec4(0.72974, -0.5, 1.0, 1.0); }
";
        let state = (PipelineBlend::Translucent, WorldDepth::Tested);
        let vertices = quad([0.0, 0.0, 64.0, 64.0], 2.0, [255; 4]);
        let runs = [PipelineRun {
            slot: 0,
            stage: 4,
            quads: 1,
            modifier_stack: 0,
        }];
        let format = wgpu::TextureFormat::Rgba16Float;

        // Gamma float frame: values above 1.0 survive verbatim.
        let mut primitives = WorldPrimitives::new(&gpu.0, (format, false));
        let draw = |p: &mut WorldPrimitives, set: &Pipelines| {
            let batch = Batch {
                set: Some(set),
                vertices: &vertices,
                runs: &runs,
                time: 0.0,
                ..Default::default()
            };
            render(&gpu, p, format, batch, STAGES - 1)(32, 32)
        };
        let before = load(1, &[(bright, state.0, state.1)]);
        assert_near(
            draw(&mut primitives, &before),
            [1.37035, 1.0, 0.0, 1.0],
            0.002,
            "gamma value above white in a float frame",
        );
        let after = load(2, &[(green, state.0, state.1)]);
        assert_near(
            draw(&mut primitives, &after),
            [0.0, 1.0, 0.0, 1.0],
            0.002,
            "the reloaded shader",
        );

        // Linear-light float frame: gamma 0.72974 decodes to linear 0.5, negative clamps to 0.
        let mut linear_primitives = WorldPrimitives::new(&gpu.0, (format, true));
        let linear_set = load(3, &[(half_and_neg, state.0, state.1)]);
        assert_near(
            draw(&mut linear_primitives, &linear_set),
            [0.5, 0.0, 1.0, 1.0],
            0.005,
            "linear-light frame decodes gamma 0.72974 to 0.5 and sanitizes negatives",
        );
    }

    /// Two pipelines sharing one GLSL program but differing in `.define(...)` draw different
    /// pixels on the GPU (#5177).
    #[test]
    #[ignore = "daily"]
    fn defines_specialize_one_glsl_program_into_distinct_gpu_pipelines() {
        let _gpu = crate::gfx::test_gpu::lock();
        let Some(gpu) = device() else {
            eprintln!("no GPU adapter: skipped");
            return;
        };
        let fsh = "\
#version 330
out vec4 fragColor;
void main() {
#ifdef RED_MODE
    fragColor = vec4(1.0, 0.0, 0.0, 1.0);
#else
    fragColor = vec4(0.0, float(GREEN_LEVEL) / 2.0, 1.0, 1.0);
#endif
}
";
        let shared = Identifier::parse("test:core/shared").unwrap();
        let registered = [
            RenderPipeline::new(
                Identifier::parse("test:red").unwrap(),
                shared.clone(),
                PipelineBlend::Translucent,
                WorldDepth::Tested,
            )
            .vertex_shader(Identifier::parse("minecraft:core/position_tex_color").unwrap())
            .define("RED_MODE"),
            RenderPipeline::new(
                Identifier::parse("test:cyan").unwrap(),
                shared,
                PipelineBlend::Translucent,
                WorldDepth::Tested,
            )
            .vertex_shader(Identifier::parse("minecraft:core/position_tex_color").unwrap())
            .define_value("GREEN_LEVEL", "2"),
        ];
        let set = Pipelines::build(&registered, 11, |file| match file {
            "assets/test/shaders/core/shared.fsh" => Ok(Some(fsh.to_owned())),
            other => Ok(stub_asset(other)),
        })
        .unwrap();
        let vertices = [
            quad([0.0, 0.0, 32.0, 64.0], 2.0, [255; 4]),
            quad([32.0, 0.0, 64.0, 64.0], 2.0, [255; 4]),
        ]
        .concat();
        let runs = [
            PipelineRun {
                slot: 0,
                stage: 0,
                quads: 1,
                modifier_stack: 0,
            },
            PipelineRun {
                slot: 1,
                stage: 0,
                quads: 1,
                modifier_stack: 0,
            },
        ];
        let format = wgpu::TextureFormat::Rgba8Unorm;
        let mut primitives = WorldPrimitives::new(&gpu.0, (format, false));
        let at = render(
            &gpu,
            &mut primitives,
            format,
            Batch {
                set: Some(&set),
                vertices: &vertices,
                runs: &runs,
                time: 0.0,
                ..Default::default()
            },
            STAGES - 1,
        );
        assert_near(at(16, 32), [255.0, 0.0, 0.0, 255.0], 1.0, "RED_MODE define");
        assert_near(
            at(48, 32),
            [0.0, 255.0, 255.0, 255.0],
            1.0,
            "GREEN_LEVEL 2 define",
        );
    }

    /// A textured pipeline samples a 2x1 sprite's two texels across its quad via
    /// `<sunrise:pipeline_sprite.glsl>`'s `sampleSprite(texCoord0)`, and one naming half the
    /// atlas samples only that sprite's part, repeating it past 1.
    #[test]
    #[ignore = "daily"]
    fn textured_pipelines_sample_their_file_or_their_sprite() {
        use mod_api::PipelineTexture;
        use wgpu::util::DeviceExt;
        let _gpu = crate::gfx::test_gpu::lock();
        let Some(gpu) = device() else {
            eprintln!("no GPU adapter: skipped");
            return;
        };
        let (device, queue) = (&gpu.0, &gpu.1);
        let sample = "\
#version 330
#include <sunrise:pipeline_sprite.glsl>
in vec2 texCoord0;
in vec4 vertexColor;
out vec4 fragColor;
void main() {
    fragColor = sampleSprite(texCoord0);
}
";
        let id = |slot: usize| Identifier::parse(&format!("test:p{slot}")).unwrap();
        let shader_id = |slot: usize| Identifier::parse(&format!("test:core/p{slot}")).unwrap();
        let state = (PipelineBlend::Translucent, WorldDepth::Tested);
        let pipeline =
            |slot| RenderPipeline::new(id(slot), shader_id(slot), state.0, state.1).optional();
        let registered = [
            pipeline(0).textured(PipelineTexture::Sprite(id(0))),
            pipeline(1).textured(PipelineTexture::Sprite(id(1))),
            pipeline(2).textured(PipelineTexture::Sprite(
                Identifier::parse("test:none").unwrap(),
            )),
            pipeline(3),
        ];
        let red_green = vec![255, 0, 0, 255, 0, 255, 0, 255];
        let set = Pipelines::build(&registered, 7, |file| {
            if file.ends_with(".vsh") {
                Ok(Some(VSH.to_owned()))
            } else if file.ends_with("p3.fsh") {
                Ok(Some(PASS.to_owned()))
            } else if file.ends_with(".fsh") {
                Ok(Some(sample.to_owned()))
            } else {
                Ok(stub_asset(file))
            }
        })
        .unwrap();
        let atlas = device.create_texture_with_data(
            queue,
            &wgpu::TextureDescriptor {
                label: None,
                size: wgpu::Extent3d {
                    width: 2,
                    height: 1,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            },
            wgpu::util::TextureDataOrder::LayerMajor,
            &red_green,
        );
        let atlas = atlas.create_view(&Default::default());
        let sprite = |name: &str| match name {
            "test:p0" => Some([0.0, 0.0, 1.0, 1.0]),
            "test:p1" => Some([0.5, 0.0, 1.0, 1.0]),
            _ => None,
        };
        let format = wgpu::TextureFormat::Rgba8Unorm;
        let mut primitives = WorldPrimitives::new(device, (format, false));
        let sources = TextureSources {
            atlas: &atlas,
            sprite: &sprite,
        };
        primitives
            .pipelines
            .build(device, &set, Some(sources))
            .unwrap();
        assert_eq!(
            set.resolve(&id(2)),
            Err(mod_api::RenderPipelineError::Invalid),
            "a sprite the atlas lacks"
        );
        assert!(
            primitives.pipelines.pipelines[0]
                .as_ref()
                .unwrap()
                .source
                .sprite
                .is_some()
        );
        assert!(
            !primitives.pipelines.pipelines[3]
                .as_ref()
                .unwrap()
                .source
                .sprite
                .is_some()
        );
        let mut vertices = Vec::new();
        for (row, slot) in [(0.0, 0), (16.0, 1), (32.0, 3)] {
            let mut quad = quad([0.0, row, 64.0, row + 16.0], 2.0, [255, 255, 255, 255]);
            if slot == 1 {
                quad.iter_mut().for_each(|v| v.uv[0] *= 2.0);
            }
            vertices.extend(quad);
        }
        let runs = [0, 1, 3].map(|slot| PipelineRun {
            slot,
            stage: 4,
            quads: 1,
            modifier_stack: 0,
        });
        let pixel = render(
            &gpu,
            &mut primitives,
            format,
            Batch {
                set: Some(&set),
                vertices: &vertices,
                runs: &runs,
                time: 0.0,
                ..Default::default()
            },
            STAGES - 1,
        );
        let (red, green) = ([255.0, 0.0, 0.0, 255.0], [0.0, 255.0, 0.0, 255.0]);
        assert_near(pixel(8, 8), red, 1.0, "the 2x1 sprite's left texel");
        assert_near(pixel(56, 8), green, 1.0, "the 2x1 sprite's right texel");
        assert_near(pixel(8, 24), green, 1.0, "the sprite, not the atlas's red");
        assert_near(pixel(40, 24), green, 1.0, "the sprite repeated");
        assert_near(pixel(8, 40), [255.0; 4], 1.0, "the untextured pipeline");
    }

    #[test]
    #[ignore = "daily"]
    fn pipeline_quads_chunk_across_small_max_buffer_size_and_preserve_order() {
        let _gpu = crate::gfx::test_gpu::lock();
        let Some(adapter) = crate::gfx::test_gpu::adapter() else {
            return;
        };
        let limit = 20 * 1024;
        let gpu = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            required_limits: wgpu::Limits {
                max_buffer_size: limit,
                max_uniform_buffer_binding_size: limit,
                max_storage_buffer_binding_size: limit,
                ..wgpu::Limits::default()
            },
            ..Default::default()
        }))
        .expect("GPU device with small max_buffer_size");
        let set = load(
            77,
            &[(PASS, PipelineBlend::Translucent, WorldDepth::Tested)],
        );
        let format = wgpu::TextureFormat::Rgba8Unorm;
        let mut primitives = WorldPrimitives::new(&gpu.0, (format, false));
        let quad_bytes = 4 * std::mem::size_of::<PipelineVertex>() as u64;
        let max_quads = world_primitives::VertexPages::limit(limit, quad_bytes);
        assert!(max_quads > 0);
        let first_count = max_quads + 20;
        let mut vertices = Vec::with_capacity((first_count as usize + 1) * 4);
        for _ in 0..first_count {
            vertices.extend_from_slice(&quad([0.0, 0.0, 64.0, 64.0], 2.0, [255, 0, 0, 255]));
        }
        vertices.extend_from_slice(&quad([32.0, 0.0, 64.0, 64.0], 2.0, [0, 255, 0, 255]));
        let runs = [
            PipelineRun {
                slot: 0,
                stage: 0,
                quads: first_count,
                modifier_stack: 0,
            },
            PipelineRun {
                slot: 0,
                stage: (STAGES - 1) as u32,
                quads: 1,
                modifier_stack: 0,
            },
        ];
        let at = render(
            &gpu,
            &mut primitives,
            format,
            Batch {
                set: Some(&set),
                vertices: &vertices,
                runs: &runs,
                time: 0.0,
                ..Default::default()
            },
            STAGES - 1,
        );
        assert!(primitives.pipelines.vertices.buffers.len() >= 2);
        assert!(
            primitives
                .pipelines
                .vertices
                .buffers
                .iter()
                .all(|buffer| buffer.size() <= limit)
        );
        assert_eq!(at(16, 32), [255.0, 0.0, 0.0, 255.0], "earlier stage");
        assert_eq!(
            at(48, 32),
            [0.0, 255.0, 0.0, 255.0],
            "later stage over earlier stage"
        );
    }

    #[test]
    #[ignore = "daily"]
    fn modifier_through_walls_nested_outermost_wins_and_refused_line_skips_only_its_runs() {
        use mod_api::PipelineModifier;
        let _gpu = crate::gfx::test_gpu::lock();
        let Some(adapter) = crate::gfx::test_gpu::adapter() else {
            return;
        };
        // Request a device WITHOUT POLYGON_MODE_LINE so a Line modifier is guaranteed refused.
        let gpu = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            required_features: wgpu::Features::empty(),
            ..Default::default()
        }))
        .expect("GPU device without POLYGON_MODE_LINE");

        let set = load(
            91,
            &[(PASS, PipelineBlend::Translucent, WorldDepth::Tested)],
        );
        let modifiers = [
            PipelineModifier::new(Identifier::parse("test:xray").unwrap())
                .depth(WorldDepth::ThroughWalls),
            PipelineModifier::new(Identifier::parse("test:tested").unwrap())
                .depth(WorldDepth::Tested),
            PipelineModifier::new(Identifier::parse("test:wireframe").unwrap())
                .polygon(PolygonMode::Line),
        ];
        let mut stacks = ModifierStacks::new();
        let xray = stacks.intern(&[0]);
        // Nested: outer = xray (0), inner = tested (1) -> outermost (xray) wins!
        let nested_xray = stacks.intern(&[0, 1]);
        // Nested: outer = tested (1), inner = xray (0) -> outermost (tested) wins!
        let nested_tested = stacks.intern(&[1, 0]);
        let wireframe = stacks.intern(&[2]);

        // render() puts a nearer depth wall on x=16..32 at z=4.0 (quads at z=8.0 are behind it).
        let vertices = [
            // Behind wall, with xray modifier -> visible red.
            quad([16.0, 0.0, 24.0, 32.0], 8.0, [255, 0, 0, 255]),
            // Behind wall, nested [xray, tested] -> outermost xray wins -> visible green.
            quad([24.0, 0.0, 32.0, 32.0], 8.0, [0, 255, 0, 255]),
            // Behind wall, nested [tested, xray] -> outermost tested wins -> hidden by wall.
            quad([16.0, 32.0, 32.0, 64.0], 8.0, [0, 0, 255, 255]),
            // Refused Line modifier on right half -> skipped, leaving clear colour.
            quad([32.0, 0.0, 48.0, 64.0], 2.0, [255, 255, 0, 255]),
            // Unmodified quad beside refused run -> still draws normally!
            quad([48.0, 0.0, 64.0, 64.0], 2.0, [0, 255, 255, 255]),
        ]
        .concat();
        let runs = [
            PipelineRun {
                slot: 0,
                stage: 0,
                quads: 1,
                modifier_stack: xray,
            },
            PipelineRun {
                slot: 0,
                stage: 0,
                quads: 1,
                modifier_stack: nested_xray,
            },
            PipelineRun {
                slot: 0,
                stage: 0,
                quads: 1,
                modifier_stack: nested_tested,
            },
            PipelineRun {
                slot: 0,
                stage: 0,
                quads: 1,
                modifier_stack: wireframe,
            },
            PipelineRun {
                slot: 0,
                stage: 0,
                quads: 1,
                modifier_stack: 0,
            },
        ];
        let format = wgpu::TextureFormat::Rgba8Unorm;
        let mut primitives = WorldPrimitives::new(&gpu.0, (format, false));
        let at = render(
            &gpu,
            &mut primitives,
            format,
            Batch {
                set: Some(&set),
                vertices: &vertices,
                runs: &runs,
                modifiers: Some(&stacks),
                modifier_defs: Some(&modifiers),
                time: 0.0,
                ..Default::default()
            },
            STAGES - 1,
        );
        let clear = [64.0, 64.0, 64.0, 255.0];
        assert_near(
            at(20, 16),
            [255.0, 0.0, 0.0, 255.0],
            1.0,
            "ThroughWalls modifier draws behind nearer depth",
        );
        assert_near(
            at(28, 16),
            [0.0, 255.0, 0.0, 255.0],
            1.0,
            "nested [xray, tested]: outermost ThroughWalls wins",
        );
        assert_near(
            at(24, 48),
            clear,
            0.0,
            "nested [tested, xray]: outermost Tested wins and hides behind wall",
        );
        assert_near(
            at(40, 32),
            clear,
            0.0,
            "refused Line modifier variant skips its run",
        );
        assert_near(
            at(56, 32),
            [0.0, 255.0, 255.0, 255.0],
            1.0,
            "other runs still draw beside a refused modifier variant",
        );
    }

    /// The frame's interned stack ids restart every frame; a stack id reused by a different
    /// stack in the next frame must draw with its own state, not the first frame's variant.
    #[test]
    #[ignore = "daily"]
    fn a_stack_id_reused_by_a_different_stack_next_frame_draws_with_the_new_stack() {
        use mod_api::PipelineModifier;
        let _gpu = crate::gfx::test_gpu::lock();
        let Some(gpu) = device() else {
            eprintln!("no GPU adapter: skipped");
            return;
        };
        let set = load(
            93,
            &[(PASS, PipelineBlend::Translucent, WorldDepth::Tested)],
        );
        let modifiers = [
            PipelineModifier::new(Identifier::parse("test:xray").unwrap())
                .depth(WorldDepth::ThroughWalls),
            PipelineModifier::new(Identifier::parse("test:tested").unwrap())
                .depth(WorldDepth::Tested),
        ];
        // A quad behind the wall `render` puts on x=16..32 at z=4.
        let vertices = quad([16.0, 0.0, 24.0, 32.0], 8.0, [255, 0, 0, 255]);
        let format = wgpu::TextureFormat::Rgba8Unorm;
        let mut primitives = WorldPrimitives::new(&gpu.0, (format, false));
        let mut frame = |slots: &[u32]| {
            let mut stacks = ModifierStacks::new();
            let id = stacks.intern(slots);
            assert_eq!(id, 1, "both frames intern their stack as id 1");
            let runs = [PipelineRun {
                slot: 0,
                stage: 0,
                quads: 1,
                modifier_stack: id,
            }];
            render(
                &gpu,
                &mut primitives,
                format,
                Batch {
                    set: Some(&set),
                    vertices: &vertices,
                    runs: &runs,
                    modifiers: Some(&stacks),
                    modifier_defs: Some(&modifiers),
                    time: 0.0,
                    ..Default::default()
                },
                STAGES - 1,
            )(20, 16)
        };
        assert_near(
            frame(&[0]),
            [255.0, 0.0, 0.0, 255.0],
            1.0,
            "frame 1: the ThroughWalls stack draws behind the wall",
        );
        assert_near(
            frame(&[1]),
            [64.0, 64.0, 64.0, 255.0],
            0.0,
            "frame 2: the same id now names the Tested stack, so the wall hides the quad",
        );
    }

    #[test]
    #[ignore = "daily"]
    fn opaque_pipeline_writes_solid_color_without_blending() {
        let _gpu = crate::gfx::test_gpu::lock();
        let Some(gpu) = device() else {
            eprintln!("no GPU adapter: skipped");
            return;
        };
        let registered = [RenderPipeline::new(
            Identifier::parse("test:opaque").unwrap(),
            Identifier::parse("test:core/opaque").unwrap(),
            PipelineBlend::Opaque,
            WorldDepth::Tested,
        )
        .depth_write(true)
        .optional()];
        let set = Pipelines::build(&registered, 92, |file| match file {
            "assets/test/shaders/core/opaque.vsh" => Ok(Some(VSH.to_owned())),
            "assets/test/shaders/core/opaque.fsh" => Ok(Some(PASS.to_owned())),
            other => Ok(stub_asset(other)),
        })
        .unwrap();
        // Half-alpha red quad: Opaque writes raw RGB (255, 0, 0) instead of alpha-blending with clear 64.
        let vertices = quad([0.0, 0.0, 64.0, 64.0], 2.0, [255, 0, 0, 128]);
        let runs = [PipelineRun {
            slot: 0,
            stage: 0,
            quads: 1,
            modifier_stack: 0,
        }];
        let format = wgpu::TextureFormat::Rgba8Unorm;
        let mut primitives = WorldPrimitives::new(&gpu.0, (format, false));
        let at = render(
            &gpu,
            &mut primitives,
            format,
            Batch {
                set: Some(&set),
                vertices: &vertices,
                runs: &runs,
                time: 0.0,
                ..Default::default()
            },
            STAGES - 1,
        );
        assert_near(
            at(32, 32),
            [255.0, 0.0, 0.0, 128.0],
            1.0,
            "Opaque writes raw colour without alpha blending",
        );
    }

    #[test]
    #[ignore = "daily"]
    fn depth_bias_lifts_coplanar_quad_over_wall_and_strict_greater_rejects_without_bias() {
        let _gpu = crate::gfx::test_gpu::lock();
        let Some(gpu) = device() else {
            eprintln!("no GPU adapter: skipped");
            return;
        };
        let registered = [
            RenderPipeline::new(
                Identifier::parse("test:unbiased").unwrap(),
                Identifier::parse("test:core/unbiased").unwrap(),
                PipelineBlend::Opaque,
                WorldDepth::Tested,
            )
            .depth_compare(DepthCompare::Greater)
            .optional(),
            RenderPipeline::new(
                Identifier::parse("test:biased").unwrap(),
                Identifier::parse("test:core/biased").unwrap(),
                PipelineBlend::Opaque,
                WorldDepth::Tested,
            )
            .depth_compare(DepthCompare::Greater)
            .depth_bias(100, 1.0)
            .optional(),
        ];
        let set = Pipelines::build(&registered, 93, |file| match file {
            "assets/test/shaders/core/unbiased.vsh" | "assets/test/shaders/core/biased.vsh" => {
                Ok(Some(VSH.to_owned()))
            }
            "assets/test/shaders/core/unbiased.fsh" | "assets/test/shaders/core/biased.fsh" => {
                Ok(Some(PASS.to_owned()))
            }
            other => Ok(stub_asset(other)),
        })
        .unwrap();
        // render() clears depth to the wall at z = 5.0.
        // Left half: coplanar at z = 5.0 with DepthCompare::Greater and no bias -> rejected.
        // Right half: coplanar at z = 5.0 with DepthCompare::Greater and positive bias -> lifted.
        let vertices = [
            quad([0.0, 0.0, 32.0, 64.0], 5.0, [255, 0, 0, 255]),
            quad([32.0, 0.0, 64.0, 64.0], 5.0, [0, 255, 0, 255]),
        ]
        .concat();
        let runs = [
            PipelineRun {
                slot: 0,
                stage: 0,
                quads: 1,
                modifier_stack: 0,
            },
            PipelineRun {
                slot: 1,
                stage: 0,
                quads: 1,
                modifier_stack: 0,
            },
        ];
        let format = wgpu::TextureFormat::Rgba8Unorm;
        let mut primitives = WorldPrimitives::new(&gpu.0, (format, false));
        let at = render(
            &gpu,
            &mut primitives,
            format,
            Batch {
                set: Some(&set),
                vertices: &vertices,
                runs: &runs,
                time: 0.0,
                ..Default::default()
            },
            STAGES - 1,
        );
        assert_near(
            at(16, 32),
            [64.0, 64.0, 64.0, 255.0],
            1.0,
            "coplanar quad without bias fails strict Greater against wall",
        );
        assert_near(
            at(48, 32),
            [0.0, 255.0, 0.0, 255.0],
            1.0,
            "positive depth bias lifts coplanar quad over wall",
        );
    }

    #[test]
    #[ignore = "daily"]
    fn custom_blend_invert_produces_exact_inverted_pixels() {
        let _gpu = crate::gfx::test_gpu::lock();
        let Some(gpu) = device() else {
            eprintln!("no GPU adapter: skipped");
            return;
        };
        let registered = [RenderPipeline::new(
            Identifier::parse("test:invert").unwrap(),
            Identifier::parse("test:core/invert").unwrap(),
            PipelineBlend::Custom(BlendFunction::INVERT),
            WorldDepth::ThroughWalls,
        )
        .optional()];
        let set = Pipelines::build(&registered, 94, |file| match file {
            "assets/test/shaders/core/invert.vsh" => Ok(Some(VSH.to_owned())),
            "assets/test/shaders/core/invert.fsh" => Ok(Some(PASS.to_owned())),
            other => Ok(stub_asset(other)),
        })
        .unwrap();
        // Clear is 64/255; BlendFunction::INVERT with red (255, 0, 0, 255):
        // R = 1*(1 - 64/255) + (64/255)*0 = 191/255, G = B = 64/255, A = 255.
        let vertices = quad([0.0, 0.0, 64.0, 64.0], 2.0, [255, 0, 0, 255]);
        let runs = [PipelineRun {
            slot: 0,
            stage: 0,
            quads: 1,
            modifier_stack: 0,
        }];
        let format = wgpu::TextureFormat::Rgba8Unorm;
        let mut primitives = WorldPrimitives::new(&gpu.0, (format, false));
        let at = render(
            &gpu,
            &mut primitives,
            format,
            Batch {
                set: Some(&set),
                vertices: &vertices,
                runs: &runs,
                time: 0.0,
                ..Default::default()
            },
            STAGES - 1,
        );
        assert_near(
            at(32, 32),
            [191.0, 64.0, 64.0, 255.0],
            1.5,
            "BlendFunction::INVERT inverts cleared destination with red source",
        );
    }

    #[test]
    #[ignore = "daily"]
    fn depthless_triangles_and_debug_lines_draw_expected_pixels() {
        use wgpu::util::DeviceExt;
        let _gpu = crate::gfx::test_gpu::lock();
        let Some((device, queue)) = device() else {
            eprintln!("no GPU adapter: skipped");
            return;
        };
        let registered = [
            RenderPipeline::new(
                Identifier::parse("test:tri").unwrap(),
                Identifier::parse("test:core/tri").unwrap(),
                PipelineBlend::Opaque,
                WorldDepth::ThroughWalls,
            )
            .depth_stencil(None)
            .topology(PrimitiveTopology::Triangles)
            .optional(),
            RenderPipeline::new(
                Identifier::parse("test:line").unwrap(),
                Identifier::parse("test:core/line").unwrap(),
                PipelineBlend::Opaque,
                WorldDepth::ThroughWalls,
            )
            .depth_stencil(None)
            .topology(PrimitiveTopology::DebugLines)
            .optional(),
        ];
        let set = Pipelines::build(&registered, 95, |file| match file {
            "assets/test/shaders/core/tri.vsh" | "assets/test/shaders/core/line.vsh" => {
                Ok(Some(VSH.to_owned()))
            }
            "assets/test/shaders/core/tri.fsh" | "assets/test/shaders/core/line.fsh" => {
                Ok(Some(PASS.to_owned()))
            }
            other => Ok(stub_asset(other)),
        })
        .unwrap();

        let format = wgpu::TextureFormat::Rgba8Unorm;
        let mut mod_pipelines = ModPipelines::new(&device, (format, false));
        mod_pipelines.build(&device, &set, None).unwrap();
        let blocks = fallback_vanilla_blocks(view_proj(), [SIZE, SIZE], 0.0);
        queue.write_buffer(&mod_pipelines.vanilla, 0, &blocks);

        // Build test vertices:
        // - Triangle on top-left (x=0..32, y=0..32) in red.
        // - Horizontal DebugLines across y=48 (x=4..60) in blue.
        let tri_quad = quad([0.0, 0.0, 32.0, 32.0], 2.0, [255, 0, 0, 255]);
        let tri_verts = [tri_quad[0], tri_quad[1], tri_quad[2]];
        let line_quad = quad([4.0, 48.5, 60.0, 49.5], 2.0, [0, 0, 255, 255]);
        let line_verts = [line_quad[0], line_quad[3]];

        let tri_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: None,
            contents: bytemuck::cast_slice(&tri_verts),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let line_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: None,
            contents: bytemuck::cast_slice(&line_verts),
            usage: wgpu::BufferUsages::VERTEX,
        });

        let color = device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d {
                width: SIZE,
                height: SIZE,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let mut encoder = device.create_command_encoder(&Default::default());
        {
            let color_view = color.create_view(&Default::default());
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: None,
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &color_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: CLEAR,
                            g: CLEAR,
                            b: CLEAR,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                ..Default::default()
            });
            // Draw Triangles
            let tri_c = mod_pipelines.pipelines[0].as_ref().unwrap();
            pass.set_pipeline(&tri_c.pipeline);
            pass.set_bind_group(0, &tri_c.source.group, &[]);
            pass.set_vertex_buffer(0, tri_buf.slice(..));
            pass.draw(0..3, 0..1);

            // Draw DebugLines
            let line_c = mod_pipelines.pipelines[1].as_ref().unwrap();
            pass.set_pipeline(&line_c.pipeline);
            pass.set_bind_group(0, &line_c.source.group, &[]);
            pass.set_vertex_buffer(0, line_buf.slice(..));
            pass.draw(0..2, 0..1);
        }
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: u64::from(SIZE * SIZE * 4),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        encoder.copy_texture_to_buffer(
            color.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(SIZE * 4),
                    rows_per_image: None,
                },
            },
            color.size(),
        );
        queue.submit([encoder.finish()]);
        readback.slice(..).map_async(wgpu::MapMode::Read, |_| {});
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        let bytes = readback.slice(..).get_mapped_range().unwrap().to_vec();
        let at = |x: u32, y: u32| -> [f32; 4] {
            let i = ((y * SIZE + x) * 4) as usize;
            std::array::from_fn(|c| f32::from(bytes[i + c]))
        };
        assert_near(
            at(8, 24),
            [255.0, 0.0, 0.0, 255.0],
            1.0,
            "depthless Triangles draws red triangle",
        );
        assert_near(
            at(32, 48),
            [0.0, 0.0, 255.0, 255.0],
            1.0,
            "depthless DebugLines draws horizontal blue line",
        );
    }

    /// Retained GPU `MeshRef` with a compact 8-byte vertex format (`Uint8x4` + `Unorm8x4`) and
    /// `Uint16` index buffer draws identically across two frames without re-uploading, and
    /// returns `MeshError::Stale` after `release` (#1159).
    #[test]
    #[ignore = "daily"]
    fn retained_compact_8byte_mesh_draws_two_frames_and_release_returns_stale() {
        let _gpu = crate::gfx::test_gpu::lock();
        let Some((device, queue)) = device() else {
            eprintln!("no GPU adapter: skipped");
            return;
        };
        let vsh = "\
#version 330
in uvec4 PackedPos;
in vec4 PackedColor;
out vec4 vertexColor;
void main() {
    vec2 ndc = vec2(float(PackedPos.x) / 32.0 - 1.0, 1.0 - float(PackedPos.y) / 32.0);
    gl_Position = vec4(ndc, 0.5, 1.0);
    vertexColor = PackedColor;
}
";
        let fsh = "\
#version 330
in vec4 vertexColor;
out vec4 fragColor;
void main() {
    fragColor = vertexColor;
}
";
        let compact_fmt = VertexFormat::builder(VertexStepMode::Vertex)
            .attribute("PackedPos", VertexAttributeFormat::Uint8x4)
            .attribute("PackedColor", VertexAttributeFormat::Unorm8x4)
            .build()
            .unwrap();
        assert_eq!(compact_fmt.stride, 8);
        let id = Identifier::parse("example:compact_mesh").unwrap();
        let reg = [RenderPipeline::new(
            id.clone(),
            id,
            PipelineBlend::Opaque,
            WorldDepth::ThroughWalls,
        )
        .topology(PrimitiveTopology::Triangles)
        .vertex_binding(0, compact_fmt)];
        let set = Pipelines::build(&reg, 1, |file| {
            if file.ends_with(".vsh") {
                Ok(Some(vsh.to_owned()))
            } else if file.ends_with(".fsh") {
                Ok(Some(fsh.to_owned()))
            } else {
                Ok(sunrise_mod_pipeline::stubs::read(
                    file.strip_prefix("assets/")
                        .and_then(|r| r.split_once('/'))
                        .map(|(ns, p)| format!("{ns}:{p}"))
                        .as_deref()
                        .unwrap_or(""),
                )
                .map(str::to_owned))
            }
        })
        .unwrap();

        #[repr(C)]
        #[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
        struct CompactVertex {
            pos: [u8; 4],
            color: [u8; 4],
        }
        let verts = [
            CompactVertex {
                pos: [0, 0, 0, 0],
                color: [20, 220, 40, 255],
            },
            CompactVertex {
                pos: [0, 64, 0, 0],
                color: [20, 220, 40, 255],
            },
            CompactVertex {
                pos: [64, 64, 0, 0],
                color: [20, 220, 40, 255],
            },
            CompactVertex {
                pos: [64, 0, 0, 0],
                color: [20, 220, 40, 255],
            },
        ];
        let indices: [u16; 6] = [0, 1, 2, 0, 2, 3];
        let vb_inits = [mod_api::MeshBufferInit::from_bytes(
            8,
            bytemuck::cast_slice(&verts),
        )];
        let desc = mod_api::MeshDescriptor::new(&vb_inits).with_indices(
            mod_api::MeshIndexInit::from_bytes(IndexFormat::Uint16, bytemuck::cast_slice(&indices)),
        );
        let mut meshes = crate::gfx::mod_meshes::ModMeshes::default();
        let mesh = meshes.create_mesh(&device, &queue, None, &desc).unwrap();
        let mesh_draw = MeshDraw::indexed(0..6, 0);
        let resolved = meshes
            .resolve_draw(
                None,
                mesh,
                set.vertex_bindings(0).unwrap(),
                PrimitiveTopology::Triangles,
                &mesh_draw,
            )
            .unwrap();

        let format = wgpu::TextureFormat::Rgba8Unorm;
        let mut primitives = WorldPrimitives::new(&device, (format, false));
        let draw_cmd = PipelineDraw {
            slot: 0,
            stage: 4,
            modifier_stack: 0,
            draw: mesh_draw.clone(),
            source: PipelineDrawSource::Retained(resolved),
            bindings: None,
        };
        let gpu = (device, queue);
        for frame in 0..2 {
            let batch = Batch {
                set: Some(&set),
                draws: std::slice::from_ref(&draw_cmd),
                time: 0.0,
                ..Default::default()
            };
            let px = render(&gpu, &mut primitives, format, batch, STAGES - 1)(32, 32);
            assert_near(
                px,
                [20.0, 220.0, 40.0, 255.0],
                1.0,
                &format!("retained compact 8-byte mesh frame {frame}"),
            );
        }

        meshes.release(None, mesh).unwrap();
        assert_eq!(
            meshes
                .resolve_draw(
                    None,
                    mesh,
                    set.vertex_bindings(0).unwrap(),
                    PrimitiveTopology::Triangles,
                    &mesh_draw,
                )
                .unwrap_err(),
            mod_api::PipelineDrawError::Mesh(mod_api::MeshError::Stale)
        );
    }

    #[test]
    fn fan_emulation_makes_a_triangle_list_of_n_minus_two_triangles_from_vertex_zero() {
        assert_eq!(fan_indices_for(0), Vec::<u32>::new());
        assert_eq!(fan_indices_for(2), Vec::<u32>::new());
        assert_eq!(fan_indices_for(3), vec![0, 1, 2]);
        let five = fan_indices_for(5);
        assert_eq!(five, vec![0, 1, 2, 0, 2, 3, 0, 3, 4]);
        assert_eq!(
            five.len() as u32,
            PrimitiveTopology::TriangleFan.index_count(5)
        );
    }

    /// A `TriangleFan` pipeline draws its four vertices through the engine's fan index buffer.
    #[test]
    #[ignore = "daily"]
    fn triangle_fan_draws_a_quad_through_the_engine_fan_index_buffer() {
        let _gpu = crate::gfx::test_gpu::lock();
        let Some((device, queue)) = device() else {
            eprintln!("no GPU adapter: skipped");
            return;
        };
        let vsh = "\
#version 330
in vec2 LocalPos;
void main() {
    vec2 ndc = vec2(LocalPos.x / 32.0 - 1.0, 1.0 - LocalPos.y / 32.0);
    gl_Position = vec4(ndc, 0.5, 1.0);
}
";
        let fsh = "\
#version 330
out vec4 fragColor;
void main() {
    fragColor = vec4(0.0, 1.0, 0.0, 1.0);
}
";
        let fmt = VertexFormat::builder(VertexStepMode::Vertex)
            .attribute("LocalPos", VertexAttributeFormat::Float32x2)
            .build()
            .unwrap();
        let id = Identifier::parse("example:fan").unwrap();
        let reg = [RenderPipeline::new(
            id.clone(),
            id,
            PipelineBlend::Opaque,
            WorldDepth::ThroughWalls,
        )
        .topology(PrimitiveTopology::TriangleFan)
        .vertex_binding(0, fmt)];
        let set = Pipelines::build(&reg, 1, |file| {
            if file.ends_with(".vsh") {
                Ok(Some(vsh.to_owned()))
            } else if file.ends_with(".fsh") {
                Ok(Some(fsh.to_owned()))
            } else {
                Ok(stub_asset(file))
            }
        })
        .unwrap();

        // The fan's corners, as a quad in the top-left quarter of the 64 px target.
        let corners: [[f32; 2]; 4] = [[0.0, 0.0], [32.0, 0.0], [32.0, 32.0], [0.0, 32.0]];
        let inits = [mod_api::MeshBufferInit::from_bytes(
            8,
            bytemuck::cast_slice(&corners),
        )];
        let desc = mod_api::MeshDescriptor::new(&inits);
        let mut meshes = crate::gfx::mod_meshes::ModMeshes::default();
        let mesh = meshes.create_mesh(&device, &queue, None, &desc).unwrap();
        let mesh_draw = MeshDraw::vertices(0..4);
        let resolved = meshes
            .resolve_draw(
                None,
                mesh,
                set.vertex_bindings(0).unwrap(),
                PrimitiveTopology::TriangleFan,
                &mesh_draw,
            )
            .unwrap();
        let format = wgpu::TextureFormat::Rgba8Unorm;
        let mut primitives = WorldPrimitives::new(&device, (format, false));
        let draw_cmd = PipelineDraw {
            slot: 0,
            stage: 4,
            modifier_stack: 0,
            draw: mesh_draw,
            source: PipelineDrawSource::Retained(resolved),
            bindings: None,
        };
        let gpu = (device, queue);
        let at = render(
            &gpu,
            &mut primitives,
            format,
            Batch {
                set: Some(&set),
                draws: std::slice::from_ref(&draw_cmd),
                time: 0.0,
                ..Default::default()
            },
            STAGES - 1,
        );
        assert_near(
            at(24, 8),
            [0.0, 255.0, 0.0, 255.0],
            1.0,
            "the fan's second triangle covers the quad's upper right",
        );
        assert_near(
            at(8, 24),
            [0.0, 255.0, 0.0, 255.0],
            1.0,
            "the fan's first triangle covers the quad's lower left",
        );
        assert_near(
            at(48, 48),
            [64.0, 64.0, 64.0, 255.0],
            0.0,
            "outside the fan keeps the clear colour",
        );
    }

    /// Multi-binding vertex format with `VertexStepMode::Instance` draws 4 offset coloured
    /// quads in a single instanced draw call (#1159).
    #[test]
    #[ignore = "daily"]
    fn instanced_vertex_binding_draws_four_offset_quads_in_one_draw_call() {
        let _gpu = crate::gfx::test_gpu::lock();
        let Some((device, queue)) = device() else {
            eprintln!("no GPU adapter: skipped");
            return;
        };
        let vsh = "\
#version 330
in vec2 LocalPos;
in vec2 InstOffset;
in vec4 InstColor;
out vec4 vertexColor;
void main() {
    vec2 p = LocalPos + InstOffset;
    vec2 ndc = vec2(p.x / 32.0 - 1.0, 1.0 - p.y / 32.0);
    gl_Position = vec4(ndc, 0.5, 1.0);
    vertexColor = InstColor;
}
";
        let fsh = "\
#version 330
in vec4 vertexColor;
out vec4 fragColor;
void main() {
    fragColor = vertexColor;
}
";
        let vert_fmt = VertexFormat::builder(VertexStepMode::Vertex)
            .attribute("LocalPos", VertexAttributeFormat::Float32x2)
            .build()
            .unwrap();
        let inst_fmt = VertexFormat::builder(VertexStepMode::Instance)
            .attribute("InstOffset", VertexAttributeFormat::Float32x2)
            .attribute("InstColor", VertexAttributeFormat::Unorm8x4)
            .build()
            .unwrap();
        let id = Identifier::parse("example:instanced").unwrap();
        let reg = [RenderPipeline::new(
            id.clone(),
            id,
            PipelineBlend::Opaque,
            WorldDepth::ThroughWalls,
        )
        .topology(PrimitiveTopology::Quads)
        .vertex_binding(0, vert_fmt)
        .vertex_binding(1, inst_fmt)];
        let set = Pipelines::build(&reg, 1, |file| {
            if file.ends_with(".vsh") {
                Ok(Some(vsh.to_owned()))
            } else if file.ends_with(".fsh") {
                Ok(Some(fsh.to_owned()))
            } else {
                Ok(sunrise_mod_pipeline::stubs::read(
                    file.strip_prefix("assets/")
                        .and_then(|r| r.split_once('/'))
                        .map(|(ns, p)| format!("{ns}:{p}"))
                        .as_deref()
                        .unwrap_or(""),
                )
                .map(str::to_owned))
            }
        })
        .unwrap();

        let unit_quad: [[f32; 2]; 4] = [[0.0, 0.0], [0.0, 32.0], [32.0, 32.0], [32.0, 0.0]];
        #[repr(C)]
        #[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
        struct InstanceRec {
            offset: [f32; 2],
            color: [u8; 4],
        }
        let instances = [
            InstanceRec {
                offset: [0.0, 0.0],
                color: [255, 0, 0, 255],
            },
            InstanceRec {
                offset: [32.0, 0.0],
                color: [0, 255, 0, 255],
            },
            InstanceRec {
                offset: [0.0, 32.0],
                color: [0, 0, 255, 255],
            },
            InstanceRec {
                offset: [32.0, 32.0],
                color: [255, 255, 0, 255],
            },
        ];
        let vb_inits = [
            mod_api::MeshBufferInit::from_bytes(8, bytemuck::cast_slice(&unit_quad)),
            mod_api::MeshBufferInit::from_bytes(12, bytemuck::cast_slice(&instances)),
        ];
        let desc = mod_api::MeshDescriptor::new(&vb_inits);
        let mut meshes = crate::gfx::mod_meshes::ModMeshes::default();
        let mesh = meshes.create_mesh(&device, &queue, None, &desc).unwrap();
        let mesh_draw = MeshDraw::vertices(0..4).with_instances(0..4);
        let resolved = meshes
            .resolve_draw(
                None,
                mesh,
                set.vertex_bindings(0).unwrap(),
                PrimitiveTopology::Quads,
                &mesh_draw,
            )
            .unwrap();

        let format = wgpu::TextureFormat::Rgba8Unorm;
        let mut primitives = WorldPrimitives::new(&device, (format, false));
        let draw_cmd = PipelineDraw {
            slot: 0,
            stage: 4,
            modifier_stack: 0,
            draw: mesh_draw,
            source: PipelineDrawSource::Retained(resolved),
            bindings: None,
        };
        let gpu = (device, queue);
        let batch = Batch {
            set: Some(&set),
            draws: std::slice::from_ref(&draw_cmd),
            time: 0.0,
            ..Default::default()
        };
        let at = render(&gpu, &mut primitives, format, batch, STAGES - 1);
        assert_near(at(16, 16), [255.0, 0.0, 0.0, 255.0], 1.0, "instance 0 red");
        assert_near(
            at(48, 16),
            [0.0, 255.0, 0.0, 255.0],
            1.0,
            "instance 1 green",
        );
        assert_near(at(16, 48), [0.0, 0.0, 255.0, 255.0], 1.0, "instance 2 blue");
        assert_near(
            at(48, 48),
            [255.0, 255.0, 0.0, 255.0],
            1.0,
            "instance 3 yellow",
        );
    }

    /// A strip-topology pipeline whose modifier switches it to `Quads` draws the quad's list
    /// indices on a shader-pack target: the pack path builds the modifier's own variant rather
    /// than drawing the base strip pipeline (which has no strip index format).
    #[test]
    #[ignore = "daily"]
    fn strip_pipeline_with_quads_modifier_draws_on_a_shader_pack_target() {
        use mod_api::PipelineModifier;
        let _gpu = crate::gfx::test_gpu::lock();
        let Some((device, queue)) = device() else {
            eprintln!("no GPU adapter: skipped");
            return;
        };
        let registered = [RenderPipeline::new(
            Identifier::parse("test:strip").unwrap(),
            Identifier::parse("test:core/p0").unwrap(),
            PipelineBlend::Opaque,
            WorldDepth::Tested,
        )
        .topology(PrimitiveTopology::TriangleStrip)
        .optional()];
        let set = Pipelines::build(&registered, 96, |file| match file {
            "assets/test/shaders/core/p0.vsh" => Ok(Some(VSH.to_owned())),
            "assets/test/shaders/core/p0.fsh" => Ok(Some(PASS.to_owned())),
            other => Ok(stub_asset(other)),
        })
        .unwrap();
        let modifiers = [
            PipelineModifier::new(Identifier::parse("test:quads").unwrap())
                .topology(PrimitiveTopology::Quads),
        ];
        let mut stacks = ModifierStacks::new();
        let quads = stacks.intern(&[0]);
        let vertices = quad([16.0, 16.0, 48.0, 48.0], 2.0, [255, 0, 0, 255]);
        let runs = [PipelineRun {
            slot: 0,
            stage: 0,
            quads: 1,
            modifier_stack: quads,
        }];

        let format = wgpu::TextureFormat::Rgba8Unorm;
        let mut primitives = WorldPrimitives::new(&device, (format, false));
        primitives.pipelines.build(&device, &set, None).unwrap();
        let texture = |format, usage| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: None,
                size: wgpu::Extent3d {
                    width: SIZE,
                    height: SIZE,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage,
                view_formats: &[],
            })
        };
        let color = texture(
            format,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        );
        let depth = texture(
            wgpu::TextureFormat::Depth32Float,
            wgpu::TextureUsages::RENDER_ATTACHMENT,
        );
        let atlas = texture(
            wgpu::TextureFormat::Rgba8Unorm,
            wgpu::TextureUsages::TEXTURE_BINDING,
        );
        assert!(primitives.prepare(
            &device,
            &mut |buffer, bytes| queue.write_buffer(buffer, 0, bytes),
            &world_primitives::Batch {
                pipelines: Batch {
                    set: Some(&set),
                    vertices: &vertices,
                    runs: &runs,
                    modifiers: Some(&stacks),
                    modifier_defs: Some(&modifiers),
                    ..Default::default()
                },
                ..Default::default()
            },
            view_proj(),
            [SIZE, SIZE],
            &atlas.create_view(&Default::default()),
            None,
        ));
        primitives.prepare_pack_pass(&device, format, false, false);
        let color_view = color.create_view(&Default::default());
        let depth_view = depth.create_view(&Default::default());
        let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let mut encoder = device.create_command_encoder(&Default::default());
        drop(encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: None,
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &color_view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color {
                        r: CLEAR,
                        g: CLEAR,
                        b: CLEAR,
                        a: 1.0,
                    }),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &depth_view,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(1.0),
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            ..Default::default()
        }));
        assert!(primitives.draw_pack_stages(&mut encoder, &color_view, &depth_view, 0, STAGES - 1));
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: u64::from(SIZE * SIZE * 4),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        encoder.copy_texture_to_buffer(
            color.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(SIZE * 4),
                    rows_per_image: None,
                },
            },
            color.size(),
        );
        queue.submit([encoder.finish()]);
        readback.slice(..).map_async(wgpu::MapMode::Read, |_| {});
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        assert!(
            pollster::block_on(validation.pop()).is_none(),
            "drawing the modifier's quads variant on a pack target is valid"
        );
        let bytes = readback.slice(..).get_mapped_range().unwrap().to_vec();
        let at = |x: u32, y: u32| {
            let i = ((y * SIZE + x) * 4) as usize;
            std::array::from_fn(|channel| f32::from(bytes[i + channel]))
        };
        assert_near(
            at(32, 32),
            [255.0, 0.0, 0.0, 255.0],
            1.0,
            "the quads variant fills the pack target",
        );
        assert_near(
            at(4, 4),
            [64.0, 64.0, 64.0, 255.0],
            0.0,
            "outside the quad stays clear",
        );
    }

    #[test]
    #[ignore = "daily"]
    fn set1_uniform_buffer_inline_uniform_push_constant_and_texel_buffer_draw_expected_pixels() {
        let _guard = crate::gfx::test_gpu::lock();
        let Some((device, queue)) = device() else {
            return;
        };
        let id = Identifier::parse("mod_c3:set1_combo").unwrap();
        let vsh = "#version 330\n\
            #include <minecraft:projection.glsl>\n\
            #include <minecraft:dynamictransforms.glsl>\n\
            layout(push_constant) uniform Push { vec4 Offset; };\n\
            in vec3 Position;\n\
            in vec2 UV0;\n\
            in vec4 Color;\n\
            out vec4 vColor;\n\
            void main() {\n\
                vec3 p = Position + Offset.xyz;\n\
                vec2 ndc = vec2(p.x / 32.0 - 1.0, 1.0 - p.y / 32.0);\n\
                gl_Position = vec4(ndc, 0.5, 1.0);\n\
                vColor = Color;\n\
            }\n";
        let fsh = "#version 330\n\
            layout(std140) uniform BaseTint { vec4 Base; };\n\
            layout(std140) uniform DrawTint { vec4 Scale; };\n\
            uniform isamplerBuffer Lut;\n\
            in vec4 vColor;\n\
            out vec4 fragColor;\n\
            void main() {\n\
                float lutScale = float(texelFetch(Lut, 1).r) / 255.0;\n\
                fragColor = vec4(Base.rgb * Scale.rgb * lutScale, 1.0) * vColor;\n\
            }\n";
        let p = RenderPipeline::new(
            id.clone(),
            id,
            PipelineBlend::Opaque,
            mod_api::WorldDepth::ThroughWalls,
        )
        .uniform_buffer("BaseTint", 16)
        .uniform_buffer("DrawTint", 16)
        .texel_buffer("Lut", mod_api::TexelBufferFormat::R32Sint)
        .push_constant_size(16);
        let set = Pipelines::build(&[p], 1, |file| {
            if file.ends_with(".vsh") {
                Ok(Some(vsh.to_owned()))
            } else if file.ends_with(".fsh") {
                Ok(Some(fsh.to_owned()))
            } else {
                Ok(sunrise_mod_pipeline::stubs::read(
                    file.strip_prefix("assets/")
                        .and_then(|r| r.split_once('/'))
                        .map(|(ns, p)| format!("{ns}:{p}"))
                        .as_deref()
                        .unwrap_or(""),
                )
                .map(str::to_owned))
            }
        })
        .expect("set-1 pipeline must translate");

        let mut uniforms = crate::gfx::mod_uniforms::ModUniformBuffers::default();
        let base_bytes: [f32; 4] = [1.0, 0.5, 1.0, 1.0];
        let ubo = uniforms
            .create_uniform_buffer(&device, &queue, None, bytemuck::cast_slice(&base_bytes))
            .unwrap();
        let (ubo_id, ubo_buf, ubo_len) = uniforms.resolve(None, ubo, 16).unwrap();

        let quad = [
            PipelineVertex {
                pos: [0.0, 0.0, 0.0],
                uv: [0.0, 0.0],
                color: 0xFFFF_FFFF,
            },
            PipelineVertex {
                pos: [0.0, 32.0, 0.0],
                uv: [0.0, 1.0],
                color: 0xFFFF_FFFF,
            },
            PipelineVertex {
                pos: [32.0, 32.0, 0.0],
                uv: [1.0, 1.0],
                color: 0xFFFF_FFFF,
            },
            PipelineVertex {
                pos: [32.0, 0.0, 0.0],
                uv: [1.0, 0.0],
                color: 0xFFFF_FFFF,
            },
        ];
        let vbytes = bytemuck::cast_slice(&quad);
        let inline1: [f32; 4] = [1.0, 1.0, 0.0, 1.0];
        let pc1: [f32; 4] = [0.0, 0.0, 0.0, 0.0];
        let inline2: [f32; 4] = [0.0, 1.0, 1.0, 1.0];
        let pc2: [f32; 4] = [32.0, 0.0, 0.0, 0.0];
        let mut inline_pool = Vec::new();
        inline_pool.extend_from_slice(bytemuck::cast_slice(&inline1));
        inline_pool.extend_from_slice(bytemuck::cast_slice(&pc1));
        inline_pool.extend_from_slice(bytemuck::cast_slice(&inline2));
        inline_pool.extend_from_slice(bytemuck::cast_slice(&pc2));
        let lut_values: [i32; 2] = [0, 255];
        let texel_pool: &[u8] = bytemuck::cast_slice(&lut_values);

        let ubo_keep = ubo_buf.clone();
        let draws = [
            PipelineDraw {
                slot: 0,
                stage: 4,
                modifier_stack: 0,
                draw: MeshDraw::vertices(0..4),
                source: PipelineDrawSource::Transient {
                    vertex_ranges: vec![(0, 0..vbytes.len() as u64)],
                    index_range: None,
                },
                bindings: Some(ResolvedDrawBindings {
                    sampler_layout: SamplerLayout::EMPTY,
                    entries: vec![
                        ResolvedSet1Entry::UniformBuffer {
                            id: ubo_id,
                            buffer: ubo_buf.clone(),
                            byte_len: ubo_len,
                        },
                        ResolvedSet1Entry::InlineUniform { byte_range: 0..16 },
                        ResolvedSet1Entry::TexelBuffer {
                            byte_range: 0..texel_pool.len(),
                        },
                    ],
                    push_constants: Some(16..32),
                    reads_world_depth: false,
                }),
            },
            PipelineDraw {
                slot: 0,
                stage: 4,
                modifier_stack: 0,
                draw: MeshDraw::vertices(0..4),
                source: PipelineDrawSource::Transient {
                    vertex_ranges: vec![(0, 0..vbytes.len() as u64)],
                    index_range: None,
                },
                bindings: Some(ResolvedDrawBindings {
                    sampler_layout: SamplerLayout::EMPTY,
                    entries: vec![
                        ResolvedSet1Entry::UniformBuffer {
                            id: ubo_id,
                            buffer: ubo_buf,
                            byte_len: ubo_len,
                        },
                        ResolvedSet1Entry::InlineUniform { byte_range: 32..48 },
                        ResolvedSet1Entry::TexelBuffer {
                            byte_range: 0..texel_pool.len(),
                        },
                    ],
                    push_constants: Some(48..64),
                    reads_world_depth: false,
                }),
            },
        ];

        let format = wgpu::TextureFormat::Rgba8Unorm;
        let mut primitives = WorldPrimitives::new(&device, (format, false));
        let max_uniform = device.limits().max_uniform_buffer_binding_size as usize;
        let gpu = (device, queue);
        let batch = Batch {
            set: Some(&set),
            draws: &draws,
            transient_vertices: vbytes,
            inline_uniforms: &inline_pool,
            texel_data: texel_pool,
            time: 0.0,
            ..Default::default()
        };
        let at = render(&gpu, &mut primitives, format, batch, STAGES - 1);
        assert_near(at(16, 16), [255.0, 128.0, 0.0, 255.0], 2.0, "draw 1");
        assert_near(at(48, 16), [0.0, 128.0, 255.0, 255.0], 2.0, "draw 2");

        // An inline payload past the device's uniform binding limit is dropped before it
        // reaches wgpu: the draw leaves the clear colour and nothing is raised.
        if max_uniform <= 1 << 24 {
            let oversized = vec![0u8; max_uniform + 16];
            let mut pool = oversized;
            pool.extend_from_slice(bytemuck::cast_slice(&pc1));
            let pc_start = pool.len() - 16;
            let huge = [PipelineDraw {
                slot: 0,
                stage: 4,
                modifier_stack: 0,
                draw: MeshDraw::vertices(0..4),
                source: PipelineDrawSource::Transient {
                    vertex_ranges: vec![(0, 0..vbytes.len() as u64)],
                    index_range: None,
                },
                bindings: Some(ResolvedDrawBindings {
                    sampler_layout: SamplerLayout::EMPTY,
                    entries: vec![
                        ResolvedSet1Entry::UniformBuffer {
                            id: ubo_id,
                            buffer: ubo_keep,
                            byte_len: ubo_len,
                        },
                        ResolvedSet1Entry::InlineUniform {
                            byte_range: 0..max_uniform + 16,
                        },
                        ResolvedSet1Entry::TexelBuffer {
                            byte_range: 0..texel_pool.len(),
                        },
                    ],
                    push_constants: Some(pc_start..pool.len()),
                    reads_world_depth: false,
                }),
            }];
            let batch = Batch {
                set: Some(&set),
                draws: &huge,
                transient_vertices: vbytes,
                inline_uniforms: &pool,
                texel_data: texel_pool,
                time: 0.0,
                ..Default::default()
            };
            let at = render(&gpu, &mut primitives, format, batch, STAGES - 1);
            assert_near(at(16, 16), [64.0, 64.0, 64.0, 255.0], 1.0, "oversized draw");
        }
        uniforms.release(None, ubo).unwrap();
        assert_eq!(
            uniforms.resolve(None, ubo, 16).unwrap_err(),
            mod_api::PipelineDrawError::Buffer(mod_api::BufferError::Stale)
        );
    }

    #[test]
    #[ignore = "daily"]
    fn storage_buffer_ranges_written_with_write_range_are_read_by_the_fragment_shader() {
        let _guard = crate::gfx::test_gpu::lock();
        let Some((device, queue)) = device() else {
            return;
        };
        let id = Identifier::parse("mod_prim2:storage").unwrap();
        let vsh = "#version 330\n\
            in vec3 Position;\n\
            in vec2 UV0;\n\
            in vec4 Color;\n\
            void main() {\n\
                vec2 ndc = vec2(Position.x / 32.0 - 1.0, 1.0 - Position.y / 32.0);\n\
                gl_Position = vec4(ndc, 0.5, 1.0);\n\
            }\n";
        let fsh = "#version 330\n\
            layout(std430) readonly buffer Tints { vec4 tint[]; };\n\
            out vec4 fragColor;\n\
            void main() {\n\
                fragColor = tint[0];\n\
            }\n";
        let p = RenderPipeline::new(
            id.clone(),
            id,
            PipelineBlend::Opaque,
            mod_api::WorldDepth::ThroughWalls,
        )
        .bind_group_layout(
            mod_api::BindGroupLayout::builder()
                .entry(mod_api::BindEntry::new(
                    "Tints",
                    mod_api::BindingKind::StorageBuffer {
                        read_only: true,
                        min_size: 16,
                    },
                ))
                .build()
                .unwrap(),
        );
        let set = Pipelines::build(&[p], 1, |file| {
            if file.ends_with(".vsh") {
                Ok(Some(vsh.to_owned()))
            } else if file.ends_with(".fsh") {
                Ok(Some(fsh.to_owned()))
            } else {
                Ok(sunrise_mod_pipeline::stubs::read(
                    file.strip_prefix("assets/")
                        .and_then(|r| r.split_once('/'))
                        .map(|(ns, p)| format!("{ns}:{p}"))
                        .as_deref()
                        .unwrap_or(""),
                )
                .map(str::to_owned))
            }
        })
        .expect("storage buffer pipeline must translate");

        // Two sub-ranges 256 bytes apart: a multiple of every device's offset alignment.
        let mut storage = crate::gfx::mod_storage::ModStorageBuffers::default();
        let buffer = storage
            .create(&device, None, &mod_api::StorageBufferDescriptor::new(512))
            .unwrap();
        storage
            .write_range(
                None,
                buffer,
                0,
                bytemuck::cast_slice(&[1.0f32, 0.0, 0.0, 1.0]),
            )
            .unwrap();
        storage
            .write_range(
                None,
                buffer,
                256,
                bytemuck::cast_slice(&[0.0f32, 1.0, 0.0, 1.0]),
            )
            .unwrap();
        storage.flush(&device, &queue);
        let limits = device.limits();
        let range = |offset| {
            storage
                .resolve(&limits, None, buffer.slice(offset, 16), 16)
                .unwrap()
        };
        let quad = |x: f32| {
            [[x, 0.0], [x, 32.0], [x + 32.0, 32.0], [x + 32.0, 0.0]].map(|[x, y]| PipelineVertex {
                pos: [x, y, 0.0],
                uv: [0.0, 0.0],
                color: 0xFFFF_FFFF,
            })
        };
        let vertices: Vec<PipelineVertex> = [quad(0.0), quad(32.0)].concat();
        let vbytes = bytemuck::cast_slice(&vertices);
        let draw = |offset: u64, first: u64| {
            let r = range(offset);
            PipelineDraw {
                slot: 0,
                stage: 4,
                modifier_stack: 0,
                draw: MeshDraw::vertices(0..4),
                source: PipelineDrawSource::Transient {
                    vertex_ranges: vec![(
                        0,
                        first * 4 * std::mem::size_of::<PipelineVertex>() as u64
                            ..(first + 1) * 4 * std::mem::size_of::<PipelineVertex>() as u64,
                    )],
                    index_range: None,
                },
                bindings: Some(ResolvedDrawBindings {
                    sampler_layout: SamplerLayout::EMPTY,
                    entries: vec![ResolvedSet1Entry::StorageBuffer {
                        id: r.id,
                        buffer: r.buffer,
                        offset: r.offset,
                        size: r.length,
                    }],
                    push_constants: None,
                    reads_world_depth: false,
                }),
            }
        };
        let draws = [draw(0, 0), draw(256, 1)];
        let format = wgpu::TextureFormat::Rgba8Unorm;
        let mut primitives = WorldPrimitives::new(&device, (format, false));
        let gpu = (device, queue);
        let batch = Batch {
            set: Some(&set),
            draws: &draws,
            transient_vertices: vbytes,
            time: 0.0,
            ..Default::default()
        };
        let at = render(&gpu, &mut primitives, format, batch, STAGES - 1);
        assert_near(at(16, 16), [255.0, 0.0, 0.0, 255.0], 2.0, "first range");
        assert_near(at(48, 16), [0.0, 255.0, 0.0, 255.0], 2.0, "second range");
    }

    /// `max_count` indexed commands a shader-less CPU wrote into an `INDIRECT` storage buffer
    /// draw their quads: a `base_vertex` other than zero reaches the vertex fetch (the DirectX 12
    /// case), a command with `instance_count = 0` draws nothing (how a mod's culling shader drops
    /// a slot), and the buffer is refused unless it is `INDIRECT`, 4-aligned and large enough.
    /// Where the adapter draws GPU-written counts (#5908), a count of 2 in ten indexed slots
    /// draws the first two and a count of 1 in ten non-indexed slots the first; without the
    /// capability a count buffer is refused, and it is refused on the same terms as the command
    /// buffer.
    #[test]
    #[ignore = "daily"]
    fn indirect_commands_draw_with_base_vertex_skip_zeroed_slots_and_are_refused_when_unfit() {
        let _gpu = crate::gfx::test_gpu::lock();
        let Some(adapter) = crate::gfx::test_gpu::adapter() else {
            return;
        };
        // Count draws where the adapter has them, as the game's device asks for them.
        let counts = adapter.features() & wgpu::Features::MULTI_DRAW_INDIRECT_COUNT;
        let descriptor = wgpu::DeviceDescriptor {
            required_features: counts,
            ..Default::default()
        };
        let Some((device, queue)) =
            crate::gfx::test_gpu::optional_device_with(&adapter, &descriptor)
        else {
            return;
        };
        let vsh = "\
#version 330
in uvec4 PackedPos;
in vec4 PackedColor;
out vec4 vertexColor;
void main() {
    vec2 ndc = vec2(float(PackedPos.x) / 32.0 - 1.0, 1.0 - float(PackedPos.y) / 32.0);
    gl_Position = vec4(ndc, 0.5, 1.0);
    vertexColor = PackedColor;
    // Red says whether `gl_VertexID` counted the command's base vertex: the second and third
    // quad's vertices are 4 and up only if it did.
    vertexColor.r = gl_VertexID >= 4 ? 1.0 : 0.0;
}
";
        let fsh = "\
#version 330
in vec4 vertexColor;
out vec4 fragColor;
void main() {
    fragColor = vertexColor;
}
";
        let fmt = VertexFormat::builder(VertexStepMode::Vertex)
            .attribute("PackedPos", VertexAttributeFormat::Uint8x4)
            .attribute("PackedColor", VertexAttributeFormat::Unorm8x4)
            .build()
            .unwrap();
        let id = Identifier::parse("example:indirect_mesh").unwrap();
        let reg = [RenderPipeline::new(
            id.clone(),
            id,
            PipelineBlend::Opaque,
            WorldDepth::ThroughWalls,
        )
        .topology(PrimitiveTopology::Triangles)
        .vertex_binding(0, fmt)];
        let set = Pipelines::build(&reg, 1, |file| {
            if file.ends_with(".vsh") {
                Ok(Some(vsh.to_owned()))
            } else if file.ends_with(".fsh") {
                Ok(Some(fsh.to_owned()))
            } else {
                Ok(stub_asset(file))
            }
        })
        .unwrap();

        // Three quads of the same four-vertex shape, side by side at y 0..16: a green one at
        // x 0..16 (vertices 0..4), a blue one at x 16..32 (4..8) and a red one at x 32..48 (8..12).
        let quad = |x: u8, color: [u8; 4]| {
            [[x, 0], [x, 16], [x + 16, 16], [x + 16, 0]]
                .map(|[x, y]| [x, y, 0, 0, color[0], color[1], color[2], color[3]])
        };
        let vertices: Vec<[u8; 8]> = [
            quad(0, [0, 220, 40, 255]),
            quad(16, [255, 40, 220, 255]),
            quad(32, [220, 20, 40, 255]),
        ]
        .concat();
        let indices: [u16; 6] = [0, 1, 2, 0, 2, 3];
        let vb_inits = [mod_api::MeshBufferInit::from_bytes(
            8,
            bytemuck::cast_slice(&vertices),
        )];
        let desc = mod_api::MeshDescriptor::new(&vb_inits).with_indices(
            mod_api::MeshIndexInit::from_bytes(IndexFormat::Uint16, bytemuck::cast_slice(&indices)),
        );
        let mut meshes = crate::gfx::mod_meshes::ModMeshes::default();
        let mesh = meshes.create_mesh(&device, &queue, None, &desc).unwrap();
        let resolved = meshes
            .resolve_indirect(None, mesh, set.vertex_bindings(0).unwrap())
            .unwrap();

        // Command 0: base vertex 0. Command 1: base vertex 4, the case DirectX 12's count draws
        // lose. Command 2: base vertex 8 with `instance_count = 0`, culled.
        let commands: [[u32; 5]; 3] = [[6, 1, 0, 0, 0], [6, 1, 0, 4, 0], [6, 0, 0, 8, 0]];
        let mut storage = crate::gfx::mod_storage::ModStorageBuffers::default();
        let buffer = storage
            .create(
                &device,
                None,
                &mod_api::StorageBufferDescriptor::new(60).indirect(true),
            )
            .unwrap();
        storage
            .write_range(None, buffer, 0, bytemuck::cast_slice(&commands))
            .unwrap();
        storage.flush(&device, &queue);
        let commands_buffer = storage.resolve_buffer(None, buffer).unwrap();
        let indirect = IndirectSource::new(
            PrimitiveTopology::Triangles,
            Some(resolved.clone()),
            commands_buffer.clone(),
            0,
            3,
            None,
            false,
        )
        .unwrap();

        let format = wgpu::TextureFormat::Rgba8Unorm;
        let mut primitives = WorldPrimitives::new(&device, (format, false));
        let draw_cmd = PipelineDraw {
            slot: 0,
            stage: 4,
            modifier_stack: 0,
            draw: MeshDraw::vertices(0..0),
            source: PipelineDrawSource::Indirect(indirect),
            bindings: None,
        };
        let gpu = (device, queue);
        let batch = Batch {
            set: Some(&set),
            draws: std::slice::from_ref(&draw_cmd),
            time: 0.0,
            ..Default::default()
        };
        let at = render(&gpu, &mut primitives, format, batch, STAGES - 1);
        assert_near(at(4, 4), [0.0, 220.0, 40.0, 255.0], 1.0, "command 0");
        assert_near(
            at(20, 4),
            [255.0, 40.0, 220.0, 255.0],
            1.0,
            "command 1: base vertex 4 must reach the vertex fetch and gl_VertexID on this backend",
        );
        assert_eq!(at(36, 4), at(36, 40), "command 2 is culled");

        let ok = |topology, offset, count| {
            IndirectSource::new(
                topology,
                Some(resolved.clone()),
                commands_buffer.clone(),
                offset,
                count,
                None,
                false,
            )
            .map(|_| ())
        };
        let triangles = PrimitiveTopology::Triangles;
        assert_eq!(ok(triangles, 0, 0), Ok(()));
        assert_eq!(ok(triangles, 0, 3), Ok(()));
        assert_eq!(ok(triangles, 40, 1), Ok(()));
        assert_eq!(
            ok(triangles, 0, 4),
            Err(mod_api::PipelineDrawError::InvalidIndirect)
        );
        assert_eq!(
            ok(triangles, 44, 1),
            Err(mod_api::PipelineDrawError::InvalidIndirect)
        );
        assert_eq!(
            ok(triangles, 2, 1),
            Err(mod_api::PipelineDrawError::InvalidIndirect)
        );
        assert_eq!(
            ok(triangles, u64::MAX - 3, u32::MAX),
            Err(mod_api::PipelineDrawError::InvalidIndirect)
        );
        let plain = {
            let device = &gpu.0;
            device.create_buffer(&wgpu::BufferDescriptor {
                label: None,
                size: 60,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        };
        assert_eq!(
            IndirectSource::new(
                triangles,
                Some(resolved.clone()),
                plain.clone(),
                0,
                1,
                None,
                false
            )
            .map(|_| ()),
            Err(mod_api::PipelineDrawError::InvalidIndirect)
        );
        // Without an index buffer the commands are 16 bytes each, and an expanded topology has
        // no vertex range to expand.
        let bare = crate::gfx::mod_meshes::ResolvedMeshDraw {
            vertex_buffers: resolved.vertex_buffers.clone(),
            index_buffer: None,
        };
        let non_indexed = |topology, count| {
            IndirectSource::new(
                topology,
                Some(bare.clone()),
                commands_buffer.clone(),
                0,
                count,
                None,
                false,
            )
            .map(|_| ())
        };
        assert_eq!(non_indexed(triangles, 3), Ok(()));
        assert_eq!(
            non_indexed(triangles, 4),
            Err(mod_api::PipelineDrawError::InvalidIndirect)
        );
        assert_eq!(
            non_indexed(PrimitiveTopology::Quads, 1),
            Err(mod_api::PipelineDrawError::TopologyMismatch)
        );

        // A GPU-written count (#5908) draws that many commands, at most `max_count`, indexed
        // (`multi_draw_indexed_indirect_count`) and not (`multi_draw_indirect_count`).
        let mut indirect_buffer = |bytes: &[u8]| {
            let descriptor = mod_api::StorageBufferDescriptor::new(bytes.len() as u64);
            let handle = storage
                .create(&gpu.0, None, &descriptor.indirect(true))
                .unwrap();
            storage.write_range(None, handle, 0, bytes).unwrap();
            storage.flush(&gpu.0, &gpu.1);
            storage.resolve_buffer(None, handle).unwrap()
        };
        // Ten indexed slots: quads 0 and 1, then quad 2 (red, drawn) in every slot after them;
        // the count at byte 4 is 2.
        let mut slots = vec![[6u32, 1, 0, 0, 0], [6, 1, 0, 4, 0]];
        slots.resize(10, [6, 1, 0, 8, 0]);
        let indexed_commands = indirect_buffer(bytemuck::cast_slice(&slots));
        let count_buffer = indirect_buffer(bytemuck::cast_slice(&[0u32, 2]));
        // Ten non-indexed slots: quad 0's lower-left triangle, then quad 1's; the count is 1.
        let mut triangle_slots = vec![[3u32, 1, 0, 0]];
        triangle_slots.resize(10, [3, 1, 4, 0]);
        let triangle_commands = indirect_buffer(bytemuck::cast_slice(&triangle_slots));
        let one = indirect_buffer(bytemuck::cast_slice(&[1u32]));
        let counts = crate::gfx::mod_render_targets::multi_draw_indirect_count(
            &adapter.get_info(),
            gpu.0.features(),
        );
        let counted = |mesh, commands: &wgpu::Buffer, count: &wgpu::Buffer, at, counts| {
            IndirectSource::new(
                triangles,
                Some(mesh),
                commands.clone(),
                0,
                10,
                Some((count.clone(), at)),
                counts,
            )
        };
        if counts {
            for (source, check) in [
                (
                    counted(resolved.clone(), &indexed_commands, &count_buffer, 4, true),
                    0,
                ),
                (counted(bare.clone(), &triangle_commands, &one, 0, true), 1),
            ] {
                let draw_cmd = PipelineDraw {
                    slot: 0,
                    stage: 4,
                    modifier_stack: 0,
                    draw: MeshDraw::vertices(0..0),
                    source: PipelineDrawSource::Indirect(source.unwrap()),
                    bindings: None,
                };
                let batch = Batch {
                    set: Some(&set),
                    draws: std::slice::from_ref(&draw_cmd),
                    time: 0.0,
                    ..Default::default()
                };
                let at = render(&gpu, &mut primitives, format, batch, STAGES - 1);
                if check == 0 {
                    assert_near(at(4, 4), [0.0, 220.0, 40.0, 255.0], 1.0, "counted quad 0");
                    assert_near(
                        at(20, 4),
                        [255.0, 40.0, 220.0, 255.0],
                        1.0,
                        "counted quad 1",
                    );
                    assert_eq!(
                        at(36, 4),
                        at(36, 40),
                        "slots 2 to 9 are past the count of 2"
                    );
                } else {
                    assert_near(
                        at(4, 12),
                        [0.0, 220.0, 40.0, 255.0],
                        1.0,
                        "counted triangle",
                    );
                    assert_eq!(
                        at(20, 12),
                        at(20, 40),
                        "slots 1 to 9 are past the count of 1"
                    );
                }
            }
        } else {
            eprintln!(
                "no count draws on {:?}: the counted draws are not run",
                adapter.get_info()
            );
        }
        // Without the capability a count buffer is refused, whatever the adapter.
        assert_eq!(
            counted(resolved.clone(), &indexed_commands, &count_buffer, 4, false).map(|_| ()),
            Err(mod_api::PipelineDrawError::UnsupportedIndirectCount)
        );
        for (count, at) in [
            (count_buffer.clone(), 2),
            (count_buffer.clone(), 8),
            (count_buffer.clone(), u64::MAX - 1),
            (plain.clone(), 0),
        ] {
            assert_eq!(
                counted(resolved.clone(), &indexed_commands, &count, at, true).map(|_| ()),
                Err(mod_api::PipelineDrawError::InvalidIndirect),
                "a count at {at}"
            );
        }
        meshes.release(None, mesh).unwrap();
        assert_eq!(
            meshes
                .resolve_indirect(None, mesh, set.vertex_bindings(0).unwrap())
                .unwrap_err(),
            mod_api::PipelineDrawError::Mesh(mod_api::MeshError::Stale)
        );
    }

    #[test]
    #[ignore = "daily"]
    fn set1_dynamic_and_standalone_textures_sample_with_repeat_addressing() {
        let _guard = crate::gfx::test_gpu::lock();
        let Some((device, queue)) = device() else {
            return;
        };
        let id = Identifier::parse("mod_c3:repeat_tex").unwrap();
        let vsh = "#version 330\n\
            #include <minecraft:projection.glsl>\n\
            #include <minecraft:dynamictransforms.glsl>\n\
            in vec3 Position;\n\
            in vec2 UV0;\n\
            in vec4 Color;\n\
            out vec2 vUv;\n\
            void main() {\n\
                vec2 ndc = vec2(Position.x / 32.0 - 1.0, 1.0 - Position.y / 32.0);\n\
                gl_Position = vec4(ndc, 0.5, 1.0);\n\
                vUv = UV0;\n\
            }\n";
        let fsh = "#version 330\n\
            uniform sampler2D CustomTex;\n\
            in vec2 vUv;\n\
            out vec4 fragColor;\n\
            void main() {\n\
                fragColor = texture(CustomTex, vUv);\n\
            }\n";
        let p = RenderPipeline::new(
            id.clone(),
            id,
            PipelineBlend::Opaque,
            mod_api::WorldDepth::ThroughWalls,
        )
        .sampler(
            "CustomTex",
            Sampler::new(SamplerFilter::Nearest, SamplerAddress::Repeat),
        );
        let set = Pipelines::build(&[p], 1, |file| {
            if file.ends_with(".vsh") {
                Ok(Some(vsh.to_owned()))
            } else if file.ends_with(".fsh") {
                Ok(Some(fsh.to_owned()))
            } else {
                Ok(sunrise_mod_pipeline::stubs::read(
                    file.strip_prefix("assets/")
                        .and_then(|r| r.split_once('/'))
                        .map(|(ns, p)| format!("{ns}:{p}"))
                        .as_deref()
                        .unwrap_or(""),
                )
                .map(str::to_owned))
            }
        })
        .expect("textured set-1 pipeline must translate");

        let rgba: [u8; 8] = [255, 0, 0, 255, 0, 0, 255, 255];
        let (_texture, view) = crate::gfx::mod_uniforms::create_rgba_texture(
            &device,
            &queue,
            "Mod dynamic texture",
            [2, 1],
            &rgba,
        );
        let view_id = crate::gfx::mod_uniforms::DYNAMIC_VIEW_ID_BIT | 1;

        let quad = [
            PipelineVertex {
                pos: [0.0, 0.0, 0.0],
                uv: [0.0, 0.0],
                color: 0xFFFF_FFFF,
            },
            PipelineVertex {
                pos: [0.0, 64.0, 0.0],
                uv: [0.0, 1.0],
                color: 0xFFFF_FFFF,
            },
            PipelineVertex {
                pos: [64.0, 64.0, 0.0],
                uv: [2.0, 1.0],
                color: 0xFFFF_FFFF,
            },
            PipelineVertex {
                pos: [64.0, 0.0, 0.0],
                uv: [2.0, 0.0],
                color: 0xFFFF_FFFF,
            },
        ];
        let vbytes = bytemuck::cast_slice(&quad);
        let draw_cmd = PipelineDraw {
            slot: 0,
            stage: 4,
            modifier_stack: 0,
            draw: MeshDraw::vertices(0..4),
            source: PipelineDrawSource::Transient {
                vertex_ranges: vec![(0, 0..vbytes.len() as u64)],
                index_range: None,
            },
            bindings: Some(ResolvedDrawBindings {
                sampler_layout: SamplerLayout::EMPTY,
                entries: vec![ResolvedSet1Entry::Texture {
                    view_id,
                    view,
                    mips: None,
                }],
                push_constants: None,
                reads_world_depth: false,
            }),
        };

        let format = wgpu::TextureFormat::Rgba8Unorm;
        let mut primitives = WorldPrimitives::new(&device, (format, false));
        let gpu = (device, queue);
        let batch = Batch {
            set: Some(&set),
            draws: std::slice::from_ref(&draw_cmd),
            transient_vertices: vbytes,
            time: 0.0,
            ..Default::default()
        };
        let at = render(&gpu, &mut primitives, format, batch, STAGES - 1);
        assert_near(at(8, 32), [255.0, 0.0, 0.0, 255.0], 1.0, "repeat 0 red");
        assert_near(at(24, 32), [0.0, 0.0, 255.0, 255.0], 1.0, "repeat 0 blue");
        assert_near(at(40, 32), [255.0, 0.0, 0.0, 255.0], 1.0, "repeat 1 red");
        assert_near(at(56, 32), [0.0, 0.0, 255.0, 255.0], 1.0, "repeat 1 blue");
    }

    /// Clears the depth texture `attachment` (a view of every aspect) to a quarter, the way a
    /// depth target a mod owns, the world's or a frame graph's holds its depth.
    fn clear_depth_to_a_quarter(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        attachment: &wgpu::TextureView,
    ) {
        clear_depth(device, queue, attachment, 0.25);
    }

    fn clear_depth(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        attachment: &wgpu::TextureView,
        value: f32,
    ) {
        let mut encoder = device.create_command_encoder(&Default::default());
        drop(encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: None,
            color_attachments: &[],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: attachment,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(value),
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            ..Default::default()
        }));
        queue.submit([encoder.finish()]);
    }

    /// Draws a quad whose pipeline binds `DepthTex` to a default `Sampler::new` sampler, with
    /// `view` (a depth texture cleared to a quarter) bound to it, and checks the frame holds
    /// the stored depth. `Sampler::new` blends mip levels, which wgpu refuses on a depth
    /// texture, so the engine reads the depth through a nearest sampler instead (#6033): the
    /// draw is not skipped.
    fn sampled_quad_setup(fragment: &str, oit: bool) -> (Pipelines, [PipelineVertex; 4], Sampler) {
        let sampler = Sampler::new(SamplerFilter::Nearest, SamplerAddress::ClampToEdge);
        let layout = mod_api::BindGroupLayout::builder()
            .sampler("DepthTex", sampler)
            .build()
            .unwrap();
        let (set, quad) = sampled_quad_with_layout(fragment, oit, layout);
        (set, quad, sampler)
    }

    fn sampled_quad_with_layout(
        fragment: &str,
        oit: bool,
        layout: mod_api::BindGroupLayout,
    ) -> (Pipelines, [PipelineVertex; 4]) {
        let id = Identifier::parse("mod_c3:depth_tex").unwrap();
        let vsh = "#version 330\n\
            #include <minecraft:projection.glsl>\n\
            #include <minecraft:dynamictransforms.glsl>\n\
            in vec3 Position;\n\
            in vec2 UV0;\n\
            in vec4 Color;\n\
            out vec2 vUv;\n\
            void main() {\n\
                vec2 ndc = vec2(Position.x / 32.0 - 1.0, 1.0 - Position.y / 32.0);\n\
                gl_Position = vec4(ndc, 0.5, 1.0);\n\
                vUv = UV0;\n\
            }\n";
        let mut p = RenderPipeline::new(
            id.clone(),
            id,
            if oit {
                PipelineBlend::Translucent
            } else {
                PipelineBlend::Opaque
            },
            WorldDepth::ThroughWalls,
        )
        .bind_group_layout(layout);
        if oit {
            p = p.order_independent();
        }
        let set = Pipelines::build(&[p], 1, |file| {
            if file.ends_with(".vsh") {
                Ok(Some(vsh.to_owned()))
            } else if file.ends_with(".fsh") {
                Ok(Some(fragment.to_owned()))
            } else {
                Ok(sunrise_mod_pipeline::stubs::read(
                    file.strip_prefix("assets/")
                        .and_then(|r| r.split_once('/'))
                        .map(|(ns, p)| format!("{ns}:{p}"))
                        .as_deref()
                        .unwrap_or(""),
                )
                .map(str::to_owned))
            }
        })
        .expect("textured set-1 pipeline must translate");

        let corner = |x: f32, y: f32, uv: [f32; 2]| PipelineVertex {
            pos: [x, y, 0.0],
            uv,
            color: 0xFFFF_FFFF,
        };
        let quad = [
            corner(0.0, 0.0, [0.0, 0.0]),
            corner(0.0, 64.0, [0.0, 1.0]),
            corner(64.0, 64.0, [1.0, 1.0]),
            corner(64.0, 0.0, [1.0, 0.0]),
        ];
        (set, quad)
    }

    const SAMPLE_DEPTH: &str = "#version 330
\
        uniform sampler2D DepthTex;
\
        in vec2 vUv;
\
        out vec4 fragColor;
\
        void main() { fragColor = vec4(texture(DepthTex, vUv).r, 0.0, 0.0, 1.0); }
";

    fn sampled_quad_draw(
        entries: Vec<ResolvedSet1Entry>,
        sampler_layout: SamplerLayout,
        stage: usize,
        vbytes: &[u8],
    ) -> PipelineDraw {
        PipelineDraw {
            slot: 0,
            stage: stage as u32,
            modifier_stack: 0,
            draw: MeshDraw::vertices(0..4),
            source: PipelineDrawSource::Transient {
                vertex_ranges: vec![(0, 0..vbytes.len() as u64)],
                index_range: None,
            },
            bindings: Some(ResolvedDrawBindings {
                sampler_layout,
                entries,
                push_constants: None,
                reads_world_depth: false,
            }),
        }
    }

    fn draw_sampling_depth(
        gpu: &(wgpu::Device, wgpu::Queue),
        view: wgpu::TextureView,
        view_id: u64,
    ) {
        let (set, quad, sampler) = sampled_quad_setup(SAMPLE_DEPTH, false);
        let vbytes = bytemuck::cast_slice(&quad);
        let mut sampler_layout = SamplerLayout::default();
        sampler_layout.record(0, &sampler, view.texture().format(), true);
        let draw = sampled_quad_draw(
            vec![ResolvedSet1Entry::Texture {
                view_id,
                view,
                mips: None,
            }],
            sampler_layout,
            4,
            vbytes,
        );
        let format = wgpu::TextureFormat::Rgba8Unorm;
        let mut primitives = WorldPrimitives::new(&gpu.0, (format, false));
        let batch = Batch {
            set: Some(&set),
            draws: std::slice::from_ref(&draw),
            transient_vertices: vbytes,
            ..Default::default()
        };
        let at = render(gpu, &mut primitives, format, batch, STAGES - 1);
        for (x, y) in [(8, 8), (32, 32), (56, 56)] {
            assert_near(at(x, y), [63.75, 0.0, 0.0, 255.0], 1.5, "the depth, read");
        }
    }

    #[test]
    #[ignore = "daily"]
    fn a_deferred_direct_pack_draw_reads_depth_with_default_sampler() {
        let _guard = crate::gfx::test_gpu::lock();
        let Some(gpu) = device() else { return };
        let (device, queue) = &gpu;
        let mut targets = super::super::mod_render_targets::ModRenderTargets::default();
        let handle = targets
            .create(
                device,
                None,
                &mod_api::RenderTargetDescriptor::rgba8(
                    [8, 8],
                    mod_api::RenderTargetDepth::Depth32Float,
                ),
            )
            .unwrap();
        let (view_id, view) = targets.resolve_depth(None, handle).unwrap();
        clear_depth_to_a_quarter(device, queue, &view);
        let (set, quad, sampler) = sampled_quad_setup(SAMPLE_DEPTH, false);
        let vbytes = bytemuck::cast_slice(&quad);
        let mut sampler_layout = SamplerLayout::default();
        sampler_layout.record(0, &sampler, view.texture().format(), true);
        let draw = sampled_quad_draw(
            vec![ResolvedSet1Entry::Texture {
                view_id,
                view,
                mips: None,
            }],
            sampler_layout,
            world_primitives::STAGE_CUSTOM_GEOMETRY,
            vbytes,
        );
        let format = wgpu::TextureFormat::Rgba8Unorm;
        let mut primitives = WorldPrimitives::new(device, (format, false));
        let batch = Batch {
            set: Some(&set),
            draws: &[draw],
            transient_vertices: vbytes,
            ..Default::default()
        };
        let at = render_target(&gpu, &mut primitives, format, batch, STAGES - 1, true);
        assert_eq!(at(32, 32), [64.0, 0.0, 0.0, 255.0]);
    }

    #[test]
    #[ignore = "daily"]
    fn a_default_sampler_reads_transient_depth_after_the_frame_graph_swaps_the_entry() {
        use super::super::mod_frame_graph::{
            FrameGraphRunner, FrameGraphSlot, RecordedFrameGraph, RecordedFramePass,
            TransientTargetPool,
        };
        use super::super::mod_render_targets::ModRenderTargets;
        use mod_api::{
            FramePassDescriptor, FrameTarget, RenderTargetDepth, RenderTargetDescriptor,
        };

        let _guard = crate::gfx::test_gpu::lock();
        let Some((device, queue)) = device() else {
            return;
        };
        let mut targets = ModRenderTargets::default();
        let out = targets
            .create(
                &device,
                None,
                &RenderTargetDescriptor::rgba8([SIZE, SIZE], RenderTargetDepth::None),
            )
            .unwrap();
        let mut recorded = RecordedFrameGraph::default();
        let desc = RenderTargetDescriptor::rgba8([SIZE, SIZE], RenderTargetDepth::Depth32Float);
        let depth = recorded.add_transient(desc.clone());
        let write =
            FramePassDescriptor::new("store transient depth", FrameTarget::Transient(depth))
                .clear_depth(0.25);
        recorded.validate_new_pass(&write, false).unwrap();
        recorded.push_pass(RecordedFramePass::from_descriptor(write, None));
        let read = FramePassDescriptor::new("sample transient depth", FrameTarget::Mod(out))
            .reads(FrameTarget::Transient(depth))
            .clear_color_0([0.0, 1.0, 0.0, 1.0]);
        recorded.validate_new_pass(&read, false).unwrap();
        let mut pass = RecordedFramePass::from_descriptor(read, None);
        let (set, quad, sampler) = sampled_quad_setup(SAMPLE_DEPTH, false);
        let mut sampler_layout = SamplerLayout::default();
        sampler_layout.record(
            0,
            &sampler,
            super::super::mod_render_targets::wgpu_depth_format(desc.depth).unwrap(),
            true,
        );
        pass.transient_vertices
            .extend_from_slice(bytemuck::cast_slice(&quad));
        pass.draws.push(sampled_quad_draw(
            vec![ResolvedSet1Entry::TransientDepth { target: depth }],
            sampler_layout,
            0,
            &pass.transient_vertices,
        ));
        recorded.push_pass(pass);
        let mut runner =
            FrameGraphRunner::prepare(recorded, false, &mut Default::default()).unwrap();
        let mut primitives =
            WorldPrimitives::new(&device, (wgpu::TextureFormat::Rgba8Unorm, false));
        primitives.pipelines.build(&device, &set, None).unwrap();
        let mut pool = TransientTargetPool::default();
        let mut encoder = device.create_command_encoder(&Default::default());
        runner.execute_slot(
            FrameGraphSlot::BeforeWorld,
            &device,
            &queue,
            &mut encoder,
            &mut primitives,
            &targets,
            &mut pool,
        );
        runner.finish(&mut pool);
        let result = read_sample::<4>(
            &device,
            &queue,
            encoder,
            targets.color_texture(out, 0).unwrap(),
        );
        assert_eq!(result, [64, 0, 0, 255]);
    }

    #[test]
    #[ignore = "daily"]
    fn a_late_masked_oit_draw_gets_groups_for_the_existing_targets_and_draws() {
        use super::super::oit::{CloudSamples, Stage};
        let _guard = crate::gfx::test_gpu::lock();
        let Some((device, queue)) = crate::gfx::test_gpu::oit_device() else {
            return;
        };
        let mut targets = super::super::mod_render_targets::ModRenderTargets::default();
        let handle = targets
            .create(
                &device,
                None,
                &mod_api::RenderTargetDescriptor::rgba8(
                    [SIZE, SIZE],
                    mod_api::RenderTargetDepth::Depth32Float,
                ),
            )
            .unwrap();
        let (id, view) = targets.resolve_depth(None, handle).unwrap();
        clear_depth_to_a_quarter(&device, &queue, &view);
        const FRAGMENT: &str = "#version 330
#include <minecraft:oit.glsl>
uniform sampler2D DepthTex;
in vec2 vUv;
#ifndef OIT_ALPHA_ONLY
out vec4 fragColor;
#endif
void main() {
    vec4 color = vec4(0.8, 0.0, 0.0, texture(DepthTex, vUv).r);
#ifdef OIT_ALPHA_ONLY
    executeAlphaOnlyPhase(gl_FragCoord.z, color.a);
#else
#ifdef OIT_ACCUMULATE
    color = sampleColorForAccumulation(color);
#endif
    fragColor = color;
#endif
}";
        let (set, quad, sampler) = sampled_quad_setup(FRAGMENT, true);
        let vbytes = bytemuck::cast_slice(&quad);
        let mut sampler_layout = SamplerLayout::default();
        sampler_layout.record(0, &sampler, view.texture().format(), true);
        let draw = sampled_quad_draw(
            vec![ResolvedSet1Entry::Texture {
                view_id: id,
                view,
                mips: None,
            }],
            sampler_layout,
            Stage::DepthBounds as usize,
            vbytes,
        );
        let mut primitives =
            WorldPrimitives::new(&device, (wgpu::TextureFormat::Rgba8Unorm, false));
        primitives.pipelines.build(&device, &set, None).unwrap();
        let texture = |format, usage| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: None,
                size: wgpu::Extent3d {
                    width: SIZE,
                    height: SIZE,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage,
                view_formats: &[],
            })
        };
        let aux = texture(
            wgpu::TextureFormat::Rgba16Float,
            wgpu::TextureUsages::TEXTURE_BINDING,
        )
        .create_view(&Default::default());
        let samples = || {
            Some(CloudSamples {
                generation: 1,
                views: [&aux; 3],
            })
        };
        primitives.prepare_oit_mod_pipelines(&device, samples());
        let batch = Batch {
            set: Some(&set),
            oit_draws: &[draw],
            oit_transient_vertices: vbytes,
            ..Default::default()
        };
        primitives.prepare(
            &device,
            &mut |buffer, bytes| queue.write_buffer(buffer, 0, bytes),
            &world_primitives::Batch {
                pipelines: batch,
                ..Default::default()
            },
            view_proj(),
            [SIZE, SIZE],
            &aux,
            None,
        );
        primitives.prepare_oit_mod_pipelines(&device, samples());
        let color = texture(
            wgpu::TextureFormat::Rgba32Float,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        );
        let depth = texture(
            wgpu::TextureFormat::Depth32Float,
            wgpu::TextureUsages::RENDER_ATTACHMENT,
        );
        let depth_view = depth.create_view(&Default::default());
        clear_depth_to_a_quarter(&device, &queue, &depth_view);
        let mut encoder = device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &color.create_view(&Default::default()),
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &depth_view,
                    depth_ops: None,
                    stencil_ops: None,
                }),
                ..Default::default()
            });
            primitives.draw_oit(&mut pass, Stage::DepthBounds);
        }
        let result = read_sample::<16>(&device, &queue, encoder, &color);
        assert_eq!(
            f32::from_le_bytes(result[8..12].try_into().unwrap()),
            0.5,
            "masked OIT geometry wrote depth bounds"
        );
    }

    #[test]
    #[ignore = "daily"]
    fn r32float_sampler_blends_mips_only_when_the_device_enables_filtering() {
        let _guard = crate::gfx::test_gpu::lock();
        let Some(adapter) = crate::gfx::test_gpu::adapter() else {
            return;
        };
        let feature = wgpu::Features::FLOAT32_FILTERABLE;
        eprintln!(
            "R32Float adapter {:?}: FLOAT32_FILTERABLE={}",
            adapter.get_info(),
            adapter.features().contains(feature)
        );
        for features in [wgpu::Features::empty(), feature] {
            if !adapter.features().contains(features) {
                continue;
            }
            let gpu = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
                required_features: features,
                ..Default::default()
            }))
            .unwrap();
            let (device, queue) = &gpu;
            let filterable = device.features().contains(feature);
            assert_eq!(filterable, features == feature);
            eprintln!("R32Float device enabled FLOAT32_FILTERABLE={filterable}");
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: None,
                size: wgpu::Extent3d {
                    width: 2,
                    height: 2,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 2,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::R32Float,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            for (mip, width, values) in [(0, 2, &[0.25f32; 4][..]), (1, 1, &[0.75f32][..])] {
                queue.write_texture(
                    wgpu::TexelCopyTextureInfo {
                        mip_level: mip,
                        ..texture.as_image_copy()
                    },
                    bytemuck::cast_slice(values),
                    wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(4 * width),
                        rows_per_image: Some(width),
                    },
                    wgpu::Extent3d {
                        width,
                        height: width,
                        depth_or_array_layers: 1,
                    },
                );
            }
            let fragment =
                SAMPLE_DEPTH.replace("texture(DepthTex, vUv)", "textureLod(DepthTex, vUv, 0.25)");
            let (set, quad, sampler) = sampled_quad_setup(&fragment, false);
            let vbytes = bytemuck::cast_slice(&quad);
            let mut sampler_layout = SamplerLayout::default();
            sampler_layout.record(0, &sampler, texture.format(), filterable);
            let draw = sampled_quad_draw(
                vec![ResolvedSet1Entry::Texture {
                    view_id: 1,
                    view: texture.create_view(&Default::default()),
                    mips: None,
                }],
                sampler_layout,
                4,
                vbytes,
            );
            let format = wgpu::TextureFormat::Rgba8Unorm;
            let mut primitives = WorldPrimitives::new(device, (format, false));
            let at = render(
                &gpu,
                &mut primitives,
                format,
                Batch {
                    set: Some(&set),
                    draws: &[draw],
                    transient_vertices: vbytes,
                    ..Default::default()
                },
                STAGES - 1,
            );
            assert_eq!(
                at(32, 32),
                [if filterable { 96.0 } else { 64.0 }, 0.0, 0.0, 255.0]
            );
        }
    }

    fn read_sample<const N: usize>(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        mut encoder: wgpu::CommandEncoder,
        texture: &wgpu::Texture,
    ) -> [u8; N] {
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("sampled variant readback"),
            size: 256,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                origin: wgpu::Origin3d {
                    x: SIZE / 2,
                    y: SIZE / 2,
                    z: 0,
                },
                ..texture.as_image_copy()
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(256),
                    rows_per_image: None,
                },
            },
            wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
        );
        queue.submit([encoder.finish()]);
        let slice = readback.slice(..);
        slice.map_async(wgpu::MapMode::Read, |result| result.unwrap());
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        slice.get_mapped_range().unwrap()[..N].try_into().unwrap()
    }

    #[test]
    #[ignore = "daily"]
    fn sampler_variants_above_63_draw_and_dispatch_on_a_capable_device() {
        use super::super::mod_compute::{DispatchKind, RecordedDispatch};
        use crate::gfx::test_gpu;
        use crate::mod_compute::ComputeSet;
        use mod_api::{BindEntry, BindGroupLayout, ComputePipeline, StorageImageAccess};
        use std::fmt::Write;

        const PAIRS: u32 = 65;
        const COLORS: u32 = 60;
        // Compute has an additional output binding before the sampled pairs.
        const BINDINGS: u32 = 2 * PAIRS + 1;
        let _guard = test_gpu::lock();
        let Some(adapter) = test_gpu::adapter() else {
            return;
        };
        let capacity = |limits: wgpu::Limits| {
            (
                limits.max_sampled_textures_per_shader_stage,
                limits.max_samplers_per_shader_stage,
                limits.max_bindings_per_bind_group,
                limits.max_storage_textures_per_shader_stage,
            )
        };
        let (textures, samplers, bindings, images) = capacity(adapter.limits());
        let compute_supported = adapter
            .get_downlevel_capabilities()
            .flags
            .contains(wgpu::DownlevelFlags::COMPUTE_SHADERS);
        let info = adapter.get_info();
        let reported = format!(
            "{info:?}: sampled textures={textures}, samplers={samplers}, group bindings={bindings}, storage images={images}, compute shaders={compute_supported}"
        );
        if textures < PAIRS
            || samplers < PAIRS
            || bindings < BINDINGS
            || images == 0
            || !compute_supported
        {
            let reason = format!(
                "wide sampler fixture needs {PAIRS} pairs, compute shaders and {BINDINGS} compute bindings including one storage image; adapter reported {reported}"
            );
            assert!(!test_gpu::require_adapter(), "{reason}");
            eprintln!("skipped: {reason}");
            return;
        }
        eprintln!("wide sampler adapter {reported}");
        let color_format = wgpu::TextureFormat::Rgba8Unorm;
        let depth_format = wgpu::TextureFormat::Depth32Float;
        let color_usage = wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::TEXTURE_BINDING;
        let depth_usage =
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING;
        let output_usage = wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::COPY_SRC;
        for (format, usage) in [
            (depth_format, depth_usage),
            (color_format, color_usage | output_usage),
        ] {
            let allowed = adapter.get_texture_format_features(format).allowed_usages;
            let missing = usage.difference(allowed);
            if !missing.is_empty() {
                let reason = format!(
                    "wide sampler fixture needs {format:?} {usage:?}; {} / {:?} allows {allowed:?}, missing {missing:?}",
                    info.name, info.backend
                );
                assert!(!test_gpu::require_adapter(), "{reason}");
                eprintln!("skipped: {reason}");
                return;
            }
        }
        let gpu = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            required_limits: wgpu::Limits {
                max_sampled_textures_per_shader_stage: PAIRS,
                max_samplers_per_shader_stage: PAIRS,
                max_bindings_per_bind_group: BINDINGS,
                max_storage_textures_per_shader_stage: 1,
                ..Default::default()
            },
            ..Default::default()
        }))
        .expect("request the eligible wide sampler device");
        let (device, queue) = &gpu;
        let actual = capacity(device.limits());
        eprintln!(
            "wide sampler device (textures, samplers, bindings, storage images) = {actual:?}; features = {:?}",
            device.features()
        );
        assert_eq!(actual, (PAIRS, PAIRS, BINDINGS, 1));

        let texture = |size, mip_level_count, format, usage| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: None,
                size: wgpu::Extent3d {
                    width: size,
                    height: size,
                    depth_or_array_layers: 1,
                },
                mip_level_count,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage,
                view_formats: &[],
            })
        };
        let color = texture(2, 2, color_format, color_usage);
        for (mip, size, green) in [(0, 2, 0), (1, 1, 255)] {
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    mip_level: mip,
                    ..color.as_image_copy()
                },
                &[0, green, 0, 255].repeat((size * size) as usize),
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(size * 4),
                    rows_per_image: Some(size),
                },
                wgpu::Extent3d {
                    width: size,
                    height: size,
                    depth_or_array_layers: 1,
                },
            );
        }
        let color = color.create_view(&Default::default());
        let depth = texture(2, 1, depth_format, depth_usage).create_view(&Default::default());
        clear_depth_to_a_quarter(device, queue, &depth);

        let sampler = Sampler::new(SamplerFilter::Nearest, SamplerAddress::ClampToEdge);
        let mut layout = BindGroupLayout::builder();
        let mut sampler_layout = SamplerLayout::default();
        let mut entries = Vec::new();
        let mut declarations = String::new();
        let mut samples = String::from("float red = 0.0; float green = 0.0;\n");
        for ordinal in 0..PAIRS {
            let name = format!("Sample{ordinal}");
            layout = layout.sampler(&name, sampler);
            writeln!(declarations, "uniform sampler2D {name};").unwrap();
            let (view, id, sum, channel) = if ordinal < COLORS {
                (&color, 1, "green", "g")
            } else {
                (&depth, 2, "red", "r")
            };
            writeln!(samples, "{sum} += textureLod({name}, uv, 0.25).{channel};").unwrap();
            sampler_layout.record(ordinal, &sampler, view.texture().format(), false);
            entries.push(ResolvedSet1Entry::Texture {
                view_id: id,
                view: view.clone(),
                mips: None,
            });
        }
        // All 65 pairs are read, including depth at ordinal 64. The five depth pairs spill
        // the key; the sixty filterable pairs must keep their vanilla mip blend.
        let value = format!(
            "vec4(red / {}.0, green / {COLORS}.0, 0.0, 1.0)",
            PAIRS - COLORS
        );
        let layout = layout.build().unwrap();
        let fragment = format!(
            "#version 330\n{declarations}in vec2 vUv;\nout vec4 fragColor;\nvoid main() {{ vec2 uv = vUv; {samples} fragColor = {value}; }}\n"
        );
        let (set, quad) = sampled_quad_with_layout(&fragment, false, layout.clone());
        let vbytes = bytemuck::cast_slice(&quad);
        let draw = sampled_quad_draw(entries.clone(), sampler_layout.clone(), 4, vbytes);
        let mut primitives = WorldPrimitives::new(device, (color_format, false));
        let batch = Batch {
            set: Some(&set),
            draws: &[draw],
            transient_vertices: vbytes,
            ..Default::default()
        };
        for _ in 0..2 {
            let pixel =
                render(&gpu, &mut primitives, color_format, batch, STAGES - 1)(SIZE / 2, SIZE / 2);
            eprintln!("wide sampler raster readback: {pixel:?}");
            assert_eq!(pixel, [64.0, 64.0, 0.0, 255.0]);
        }

        let access = StorageImageAccess::WriteOnly;
        let mut compute_layout = layout;
        compute_layout.entries.insert(
            0,
            BindEntry::new("Out", BindingKind::StorageImage { access }),
        );
        let id = Identifier::parse("mod_c3:wide_sampler").unwrap();
        let compute_shader = format!(
            "#version 450\nlayout(local_size_x = 1) in;\nlayout(rgba8) writeonly uniform image2D Out;\n{declarations}void main() {{ vec2 uv = vec2(0.5); {samples} imageStore(Out, ivec2({center}), {value}); }}\n",
            center = SIZE / 2
        );
        let set = ComputeSet::build(
            &[ComputePipeline::builder(id.clone(), id.clone())
                .bind_group_layout(compute_layout)
                .build()],
            2,
            |_| Ok(Some(compute_shader.clone())),
        );
        let compute = &mut primitives.pipelines.compute;
        compute.build(device, &set);
        let slot = set
            .slot(set.resolve(&id).expect("wide compute pipeline is admitted"))
            .unwrap();
        let output = texture(SIZE, 1, color_format, output_usage);
        entries.insert(
            0,
            ResolvedSet1Entry::StorageImage {
                view_id: 3,
                view: output.create_view(&Default::default()),
                format: color_format,
                mips: TargetMips {
                    texture: 3,
                    base_mip: 0,
                    mip_levels: 1,
                    usage: TargetUse::StorageWriteOnly,
                },
            },
        );
        let dispatch = RecordedDispatch {
            slot,
            sampler_layout,
            entries,
            inline: Vec::new(),
            texel: Vec::new(),
            kind: DispatchKind::Direct([1, 1, 1]),
        };
        let mut encoder = device.create_command_encoder(&Default::default());
        compute.record(device, &mut encoder, [&dispatch]);
        let pixel = read_sample::<4>(device, queue, encoder, &output);
        eprintln!("wide sampler compute readback: {pixel:?}");
        assert_eq!(pixel, [64, 64, 0, 255]);
    }

    /// A depth texture bound through `TextureSource::WorldDepth`: the view the renderer makes
    /// for its own depth buffer.
    #[test]
    #[ignore = "daily"]
    fn the_world_depth_binds_to_a_default_sampler_and_reads_nearest() {
        let _guard = crate::gfx::test_gpu::lock();
        let Some((device, queue)) = device() else {
            return;
        };
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: wgpu::TextureFormat::Rgba8Unorm,
            width: 4,
            height: 4,
            present_mode: wgpu::PresentMode::Fifo,
            desired_maximum_frame_latency: 2,
            alpha_mode: wgpu::CompositeAlphaMode::Auto,
            view_formats: vec![],
            color_space: wgpu::SurfaceColorSpace::default(),
        };
        let view = crate::gfx::Renderer::depth(&device, &config);
        clear_depth_to_a_quarter(&device, &queue, &view);
        draw_sampling_depth(
            &(device, queue),
            view,
            crate::gfx::mod_uniforms::DYNAMIC_VIEW_ID_BIT | 3,
        );
    }

    /// The world depth after the renderer replaced it, as a resize does (#6175): a draw whose
    /// inputs did not otherwise change keeps its set-1 bind group between frames, so the
    /// replacement needs a view ID of its own for the second frame to sample it.
    #[test]
    #[ignore = "daily"]
    fn a_replaced_world_depth_is_sampled_through_a_retained_bind_group() {
        let _guard = crate::gfx::test_gpu::lock();
        let Some((adapter, device, queue)) = crate::gfx::test_gpu::optional_device() else {
            return;
        };
        let depth_of = |size: u32, value: f32| {
            let config = wgpu::SurfaceConfiguration {
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                format: wgpu::TextureFormat::Rgba8Unorm,
                width: size,
                height: size,
                present_mode: wgpu::PresentMode::Fifo,
                desired_maximum_frame_latency: 2,
                alpha_mode: wgpu::CompositeAlphaMode::Auto,
                view_formats: vec![],
                color_space: wgpu::SurfaceColorSpace::default(),
            };
            let view = crate::gfx::Renderer::depth(&device, &config);
            clear_depth(&device, &queue, &view, value);
            view
        };
        let unused_view = || {
            device
                .create_texture(&wgpu::TextureDescriptor {
                    label: None,
                    size: wgpu::Extent3d {
                        width: 1,
                        height: 1,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    usage: wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                })
                .create_view(&Default::default())
        };
        let resources = crate::gfx::mod_gpu::ModGpuResources::new(
            adapter,
            device.clone(),
            queue.clone(),
            Default::default(),
            crate::gfx::mod_gpu::ModFrameState {
                entity_atlas: unused_view(),
                lightmap: unused_view(),
                entity_quad_capacity: crate::gfx::EntityQuadCapacity::from_limits(
                    &device.limits(),
                    false,
                ),
                pack_iris: false,
            },
            // Depth 0.25 is 63.75 of 255, which the frame stores as 64.
            depth_of(4, 0.25),
            None,
        );

        let (set, quad, sampler) = sampled_quad_setup(SAMPLE_DEPTH, false);
        let vbytes = bytemuck::cast_slice(&quad);
        let format = wgpu::TextureFormat::Rgba8Unorm;
        let gpu = (device.clone(), queue.clone());
        let mut primitives = WorldPrimitives::new(&device, (format, false));
        // One frame: the quad over the world depth as the mod handles hand it out now.
        let mut frame = || {
            let (view_id, view) = resources.mod_world_depth_texture_view();
            let mut sampler_layout = SamplerLayout::default();
            sampler_layout.record(0, &sampler, view.texture().format(), true);
            let draw = sampled_quad_draw(
                vec![ResolvedSet1Entry::Texture {
                    view_id,
                    view,
                    mips: None,
                }],
                sampler_layout,
                4,
                vbytes,
            );
            let batch = Batch {
                set: Some(&set),
                draws: std::slice::from_ref(&draw),
                transient_vertices: vbytes,
                ..Default::default()
            };
            let at = render(&gpu, &mut primitives, format, batch, STAGES - 1);
            [(8, 8), (32, 32), (56, 56)].map(|(x, y)| at(x, y))
        };
        assert_eq!(
            frame(),
            [[64.0, 0.0, 0.0, 255.0]; 3],
            "the first frame reads the first depth"
        );

        // The renderer's resize makes a larger depth and hands it to the mod handles. Depth
        // 0.75 is 191.25 of 255, which the frame stores as 191.
        resources.replace_world_depth(depth_of(8, 0.75));
        assert_eq!(
            frame(),
            [[191.0, 0.0, 0.0, 255.0]; 3],
            "the second frame reads the replacement depth, not the retired one"
        );
    }

    /// A depth texture bound through `TextureSource::RenderTargetDepth`: the depth-only view a
    /// mod's render target resolves for sampling.
    #[test]
    #[ignore = "daily"]
    fn a_render_target_depth_binds_to_a_default_sampler_and_reads_nearest() {
        let _guard = crate::gfx::test_gpu::lock();
        let Some((device, queue)) = device() else {
            return;
        };
        let mut targets = crate::gfx::mod_render_targets::ModRenderTargets::default();
        let rt = targets
            .create(
                &device,
                Some("test_mod"),
                &mod_api::RenderTargetDescriptor::rgba8(
                    [4, 4],
                    mod_api::RenderTargetDepth::Depth32Float,
                ),
            )
            .unwrap();
        let attachment = targets
            .resolve_target(Some("test_mod"), rt, 0)
            .unwrap()
            .depth_view
            .unwrap();
        clear_depth_to_a_quarter(&device, &queue, &attachment);
        let (view_id, view) = targets.resolve_depth(Some("test_mod"), rt).unwrap();
        draw_sampling_depth(&(device, queue), view, view_id);
    }

    /// A depth texture bound through `TextureSource::TransientTargetDepth`: the depth view the
    /// frame graph's pool makes for a transient target.
    #[test]
    #[ignore = "daily"]
    fn a_transient_depth_binds_to_a_default_sampler_and_reads_nearest() {
        let _guard = crate::gfx::test_gpu::lock();
        let Some((device, queue)) = device() else {
            return;
        };
        let mut pool = crate::gfx::mod_frame_graph::TransientTargetPool::default();
        let transient = pool.acquire(
            &device,
            &mod_api::RenderTargetDescriptor::rgba8(
                [4, 4],
                mod_api::RenderTargetDepth::Depth32Float,
            ),
        );
        let attachment = transient.as_resolved().depth_view.unwrap();
        clear_depth_to_a_quarter(&device, &queue, &attachment);
        let (view_id, view) = transient.depth_view().unwrap();
        draw_sampling_depth(&(device, queue), view, view_id);
    }

    /// A depth texture a test creates itself, with no engine path in between.
    #[test]
    #[ignore = "daily"]
    fn a_depth_texture_binds_to_a_default_sampler_and_reads_nearest() {
        let _guard = crate::gfx::test_gpu::lock();
        let Some((device, queue)) = device() else {
            return;
        };
        let depth = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Depth to sample"),
            size: wgpu::Extent3d {
                width: 4,
                height: 4,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Depth32Float,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let view = depth.create_view(&Default::default());
        clear_depth_to_a_quarter(&device, &queue, &view);
        draw_sampling_depth(
            &(device, queue),
            view,
            crate::gfx::mod_uniforms::DYNAMIC_VIEW_ID_BIT | 2,
        );
    }

    #[test]
    #[ignore = "daily"]
    fn a_mipmapped_dynamic_texture_sampled_at_a_distance_shows_level_two() {
        let _guard = crate::gfx::test_gpu::lock();
        let Some((device, queue)) = device() else {
            return;
        };
        let id = Identifier::parse("mod_c3:mip_tex").unwrap();
        let vsh = "#version 330\n\
            #include <minecraft:projection.glsl>\n\
            #include <minecraft:dynamictransforms.glsl>\n\
            in vec3 Position;\n\
            in vec2 UV0;\n\
            in vec4 Color;\n\
            out vec2 vUv;\n\
            void main() {\n\
                vec2 ndc = vec2(Position.x / 32.0 - 1.0, 1.0 - Position.y / 32.0);\n\
                gl_Position = vec4(ndc, 0.5, 1.0);\n\
                vUv = UV0;\n\
            }\n";
        let fsh = "#version 330\n\
            uniform sampler2D CustomTex;\n\
            in vec2 vUv;\n\
            out vec4 fragColor;\n\
            void main() {\n\
                fragColor = texture(CustomTex, vUv);\n\
            }\n";
        let p = RenderPipeline::new(
            id.clone(),
            id,
            PipelineBlend::Opaque,
            mod_api::WorldDepth::ThroughWalls,
        )
        .sampler("CustomTex", {
            let mut sampler = Sampler::new(SamplerFilter::Nearest, SamplerAddress::ClampToEdge);
            sampler.max_lod = Some(3.0);
            sampler
        });
        let set = Pipelines::build(&[p], 1, |file| {
            if file.ends_with(".vsh") {
                Ok(Some(vsh.to_owned()))
            } else if file.ends_with(".fsh") {
                Ok(Some(fsh.to_owned()))
            } else {
                Ok(sunrise_mod_pipeline::stubs::read(
                    file.strip_prefix("assets/")
                        .and_then(|r| r.split_once('/'))
                        .map(|(ns, p)| format!("{ns}:{p}"))
                        .as_deref()
                        .unwrap_or(""),
                )
                .map(str::to_owned))
            }
        })
        .expect("textured set-1 pipeline must translate");

        // A 256 x 256 texture of 4 levels (256, 128, 64, 32 wide), each one solid colour, made
        // and filled the way a mod does: a create, then one region write per level.
        let colors: [[u8; 4]; 4] = [
            [255, 0, 0, 255],
            [0, 255, 0, 255],
            [0, 0, 255, 255],
            [255, 255, 0, 255],
        ];
        let level_pixels =
            |level: u32| colors[level as usize].repeat(((256 >> level) as usize).pow(2));
        let mut commands = vec![crate::hud::dynamic_textures::Command::Create {
            handle: 1,
            size: [256, 256],
            mip_levels: 4,
            rgba: level_pixels(0),
        }];
        for level in 1..4 {
            commands.push(crate::hud::dynamic_textures::Command::Write {
                handle: 1,
                mip_level: level,
                origin: [0, 0],
                size: [256 >> level; 2],
                rgba: level_pixels(level),
            });
        }
        let store = crate::hud::dynamic_textures::Store::default();
        let mut uploads = crate::gfx::upload::Uploads::new(&device);
        let mut encoder = device.create_command_encoder(&Default::default());
        store.apply(&device, &mut uploads, &mut encoder, None, &commands);
        uploads.finish(&encoder);
        queue.submit([encoder.finish()]);
        let (view, _) = store.view(1).expect("the texture was created");
        let view_id = crate::gfx::mod_uniforms::DYNAMIC_VIEW_ID_BIT | 1;

        let quad = [
            PipelineVertex {
                pos: [0.0, 0.0, 0.0],
                uv: [0.0, 0.0],
                color: 0xFFFF_FFFF,
            },
            PipelineVertex {
                pos: [0.0, 64.0, 0.0],
                uv: [0.0, 1.0],
                color: 0xFFFF_FFFF,
            },
            PipelineVertex {
                pos: [64.0, 64.0, 0.0],
                uv: [1.0, 1.0],
                color: 0xFFFF_FFFF,
            },
            PipelineVertex {
                pos: [64.0, 0.0, 0.0],
                uv: [1.0, 0.0],
                color: 0xFFFF_FFFF,
            },
        ];
        let vbytes = bytemuck::cast_slice(&quad);
        let draw_cmd = PipelineDraw {
            slot: 0,
            stage: 4,
            modifier_stack: 0,
            draw: MeshDraw::vertices(0..4),
            source: PipelineDrawSource::Transient {
                vertex_ranges: vec![(0, 0..vbytes.len() as u64)],
                index_range: None,
            },
            bindings: Some(ResolvedDrawBindings {
                sampler_layout: SamplerLayout::EMPTY,
                entries: vec![ResolvedSet1Entry::Texture {
                    view_id,
                    view,
                    mips: None,
                }],
                push_constants: None,
                reads_world_depth: false,
            }),
        };

        let format = wgpu::TextureFormat::Rgba8Unorm;
        let mut primitives = WorldPrimitives::new(&device, (format, false));
        let gpu = (device, queue);
        let batch = Batch {
            set: Some(&set),
            draws: std::slice::from_ref(&draw_cmd),
            transient_vertices: vbytes,
            time: 0.0,
            ..Default::default()
        };
        let at = render(&gpu, &mut primitives, format, batch, STAGES - 1);
        // The quad is 64 pixels wide for 256 texels: four texels a pixel, level of detail 2.
        for (x, y) in [(8, 8), (32, 32), (56, 56)] {
            assert_near(at(x, y), [0.0, 0.0, 255.0, 255.0], 1.0, "level 2");
        }
    }

    #[test]
    #[ignore = "daily"]
    fn offscreen_render_target_draw_sample_persist_and_resize() {
        let _gpu_lock = crate::gfx::test_gpu::lock();
        let (device, queue) = device().expect("a GPU adapter");

        // Pipeline 0: solid red quad into offscreen target (no set-1 bindings).
        let write_id = Identifier::parse("test:offscreen_write").unwrap();
        let write_vsh = "#version 330\n\
            in vec3 Position;\n\
            in vec2 UV0;\n\
            in vec4 Color;\n\
            out vec4 vColor;\n\
            layout(std140) uniform Projection { mat4 ProjMat; };\n\
            layout(std140) uniform DynamicTransforms {\n\
                mat4 ModelViewMat; mat4 TextureMat; vec4 ColorModulator; vec3 ModelOffset;\n\
            };\n\
            void main() {\n\
                gl_Position = ProjMat * ModelViewMat * vec4(Position, 1.0);\n\
                vColor = Color;\n\
            }\n";
        let write_fsh = "#version 330\n\
            in vec4 vColor;\n\
            out vec4 fragColor;\n\
            void main() { fragColor = vColor; }\n";

        // Pipeline 1: samples the offscreen target's colour attachment via set-1 sampler.
        let read_id = Identifier::parse("test:offscreen_read").unwrap();
        let read_vsh = "#version 330\n\
            in vec3 Position;\n\
            in vec2 UV0;\n\
            in vec4 Color;\n\
            out vec2 vUv;\n\
            void main() {\n\
                vec2 ndc = vec2(Position.x / 32.0 - 1.0, 1.0 - Position.y / 32.0);\n\
                gl_Position = vec4(ndc, 0.5, 1.0);\n\
                vUv = UV0;\n\
            }\n";
        let read_fsh = "#version 330\n\
            in vec2 vUv;\n\
            uniform sampler2D OffscreenTex;\n\
            out vec4 fragColor;\n\
            void main() { fragColor = texture(OffscreenTex, vUv); }\n";

        let p_write = RenderPipeline::new(
            write_id.clone(),
            write_id,
            PipelineBlend::Opaque,
            mod_api::WorldDepth::ThroughWalls,
        );
        let p_read = RenderPipeline::new(
            read_id.clone(),
            read_id,
            PipelineBlend::Opaque,
            mod_api::WorldDepth::ThroughWalls,
        )
        .sampler(
            "OffscreenTex",
            mod_api::Sampler::new(
                mod_api::SamplerFilter::Nearest,
                mod_api::SamplerAddress::ClampToEdge,
            ),
        );

        let set = Pipelines::build(&[p_write, p_read], 1, |file| {
            if file.contains("offscreen_write") && file.ends_with(".vsh") {
                Ok(Some(write_vsh.to_owned()))
            } else if file.contains("offscreen_write") && file.ends_with(".fsh") {
                Ok(Some(write_fsh.to_owned()))
            } else if file.contains("offscreen_read") && file.ends_with(".vsh") {
                Ok(Some(read_vsh.to_owned()))
            } else if file.contains("offscreen_read") && file.ends_with(".fsh") {
                Ok(Some(read_fsh.to_owned()))
            } else {
                Ok(None)
            }
        })
        .expect("offscreen write/read pipelines must translate");

        let mut targets = crate::gfx::mod_render_targets::ModRenderTargets::default();
        let rt = targets
            .create(
                &device,
                Some("test_mod"),
                &mod_api::RenderTargetDescriptor::rgba8(
                    [32, 32],
                    mod_api::RenderTargetDepth::Depth32Float,
                ),
            )
            .expect("create 32x32 RGBA8+D32F target");
        assert_eq!(targets.size(Some("test_mod"), rt), Ok([32, 32]));
        assert_eq!(
            targets.size(Some("other_mod"), rt),
            Err(mod_api::RenderTargetError::NotOwned)
        );

        assert!(targets.resolve_depth(Some("test_mod"), rt).is_ok());

        let format = wgpu::TextureFormat::Rgba8Unorm;
        let mut primitives = WorldPrimitives::new(&device, (format, false));
        primitives
            .pipelines
            .build(&device, &set, None)
            .expect("build offscreen pipelines");

        // Orthographic projection mapping [0..32, 0..32] into clip space [-1..1].
        let ortho32 = Mat4::orthographic_rh(0.0, 32.0, 32.0, 0.0, -1.0, 1.0).to_cols_array();
        // `PipelineVertex.color` packs red in the low byte (0xAABBGGRR).
        let red_quad = [
            PipelineVertex {
                pos: [0.0, 0.0, 0.0],
                uv: [0.0, 0.0],
                color: 0xFF00_00FF,
            },
            PipelineVertex {
                pos: [0.0, 32.0, 0.0],
                uv: [0.0, 1.0],
                color: 0xFF00_00FF,
            },
            PipelineVertex {
                pos: [32.0, 32.0, 0.0],
                uv: [1.0, 1.0],
                color: 0xFF00_00FF,
            },
            PipelineVertex {
                pos: [32.0, 0.0, 0.0],
                uv: [1.0, 0.0],
                color: 0xFF00_00FF,
            },
        ];
        let red_vbytes = bytemuck::cast_slice(&red_quad);
        let write_draw = PipelineDraw {
            slot: 0,
            stage: 0,
            modifier_stack: 0,
            draw: MeshDraw::vertices(0..4),
            source: PipelineDrawSource::Transient {
                vertex_ranges: vec![(0, 0..red_vbytes.len() as u64)],
                index_range: None,
            },
            bindings: None,
        };

        // Pass 1: clear to black, draw red quad into offscreen target.
        let resolved_rt = targets.resolve_target(Some("test_mod"), rt, 0).unwrap();
        let pass1 = mod_api::OffscreenPass::new(rt)
            .clear_color_0([0.0, 0.0, 0.0, 1.0])
            .clear_depth(0.0)
            .projection(ortho32);
        let mut enc = device.create_command_encoder(&Default::default());
        primitives.execute_offscreen_pass(
            &device,
            &queue,
            &mut enc,
            &OffscreenPassData {
                pass: &pass1,
                target: &resolved_rt,
                draws: std::slice::from_ref(&write_draw),
                transient_vertices: red_vbytes,
                transient_indices: &[],
                inline_uniforms: &[],
                texel_data: &[],
                modifiers: None,
                modifier_defs: None,
            },
        );
        // Pass 2: LoadOp::Load (no clear) with zero draws — pixels must persist!
        let pass2 = mod_api::OffscreenPass::new(rt);
        primitives.execute_offscreen_pass(
            &device,
            &queue,
            &mut enc,
            &OffscreenPassData {
                pass: &pass2,
                target: &resolved_rt,
                draws: &[],
                transient_vertices: &[],
                transient_indices: &[],
                inline_uniforms: &[],
                texel_data: &[],
                modifiers: None,
                modifier_defs: None,
            },
        );
        queue.submit([enc.finish()]);

        // Same-size resize is a no-op that preserves existing contents and view_id.
        let (view_id_before, _, _) = targets.resolve_color(Some("test_mod"), rt, 0).unwrap();
        targets
            .resize(&device, Some("test_mod"), rt, [32, 32])
            .unwrap();
        let (view_id_after_noop, rt_view, _) =
            targets.resolve_color(Some("test_mod"), rt, 0).unwrap();
        assert_eq!(view_id_before, view_id_after_noop);

        // Sample the offscreen target in a world pass and verify the red pixels survived!
        let sample_quad = [
            PipelineVertex {
                pos: [0.0, 0.0, 0.0],
                uv: [0.0, 0.0],
                color: 0xFFFF_FFFF,
            },
            PipelineVertex {
                pos: [0.0, 64.0, 0.0],
                uv: [0.0, 1.0],
                color: 0xFFFF_FFFF,
            },
            PipelineVertex {
                pos: [64.0, 64.0, 0.0],
                uv: [1.0, 1.0],
                color: 0xFFFF_FFFF,
            },
            PipelineVertex {
                pos: [64.0, 0.0, 0.0],
                uv: [1.0, 0.0],
                color: 0xFFFF_FFFF,
            },
        ];
        let sample_vbytes = bytemuck::cast_slice(&sample_quad);
        let read_draw = PipelineDraw {
            slot: 1,
            stage: 4,
            modifier_stack: 0,
            draw: MeshDraw::vertices(0..4),
            source: PipelineDrawSource::Transient {
                vertex_ranges: vec![(0, 0..sample_vbytes.len() as u64)],
                index_range: None,
            },
            bindings: Some(ResolvedDrawBindings {
                sampler_layout: SamplerLayout::EMPTY,
                entries: vec![ResolvedSet1Entry::Texture {
                    view_id: view_id_after_noop,
                    view: rt_view,
                    mips: None,
                }],
                push_constants: None,
                reads_world_depth: false,
            }),
        };

        // Resize to 64x64 changes dimensions and view_id; destroy retires handle.
        let gpu = (device, queue);
        let batch = Batch {
            set: Some(&set),
            draws: std::slice::from_ref(&read_draw),
            transient_vertices: sample_vbytes,
            time: 0.0,
            ..Default::default()
        };
        let at = render(&gpu, &mut primitives, format, batch, STAGES - 1);
        assert_near(
            at(32, 32),
            [255.0, 0.0, 0.0, 255.0],
            1.0,
            "sampled offscreen red pixel",
        );

        targets
            .resize(&gpu.0, Some("test_mod"), rt, [64, 64])
            .unwrap();
        assert_eq!(targets.size(Some("test_mod"), rt), Ok([64, 64]));
        let (view_id_resized, _, _) = targets.resolve_color(Some("test_mod"), rt, 0).unwrap();
        assert_ne!(view_id_before, view_id_resized);
        targets.release(Some("test_mod"), rt).unwrap();
        assert_eq!(
            targets.size(Some("test_mod"), rt),
            Err(mod_api::RenderTargetError::Stale)
        );
    }

    #[test]
    #[ignore = "daily"]
    fn a_glsl_image_store_writes_one_mip_level_of_a_render_target() {
        let _gpu = crate::gfx::test_gpu::lock();
        let Some((device, queue)) = device() else {
            return;
        };
        let id = Identifier::parse("mod_mips:store").unwrap();
        let vsh = "#version 330\n\
            in vec3 Position;\n\
            in vec2 UV0;\n\
            in vec4 Color;\n\
            void main() {\n\
                vec2 ndc = vec2(Position.x / 32.0 - 1.0, 1.0 - Position.y / 32.0);\n\
                gl_Position = vec4(ndc, 0.5, 1.0);\n\
            }\n";
        let fsh = "#version 330\n\
            layout(r32f) writeonly uniform image2D Next;\n\
            out vec4 fragColor;\n\
            void main() {\n\
                imageStore(Next, ivec2(0), vec4(0.25));\n\
                imageStore(Next, ivec2(1, 0), vec4(0.5));\n\
                fragColor = vec4(1.0);\n\
            }\n";
        let pipeline = RenderPipeline::new(
            id.clone(),
            id,
            PipelineBlend::Opaque,
            mod_api::WorldDepth::ThroughWalls,
        )
        .bind_group_layout(
            mod_api::BindGroupLayout::builder()
                .entry(mod_api::BindEntry::new(
                    "Next",
                    mod_api::BindingKind::StorageImage {
                        access: mod_api::StorageImageAccess::WriteOnly,
                    },
                ))
                .build()
                .unwrap(),
        );
        let set = Pipelines::build(&[pipeline], 1, |file| {
            if file.ends_with(".vsh") {
                Ok(Some(vsh.to_owned()))
            } else if file.ends_with(".fsh") {
                Ok(Some(fsh.to_owned()))
            } else {
                Ok(stub_asset(file))
            }
        })
        .expect("a write-only storage image pipeline must translate");

        // A 4 x 4 pyramid of three levels: level 1 is 2 x 2.
        let mut targets = crate::gfx::mod_render_targets::ModRenderTargets::default();
        let target = targets
            .create(
                &device,
                None,
                &mod_api::RenderTargetDescriptor::new([4, 4])
                    .color_target(mod_api::RenderTargetFormat::R32Float)
                    .mip_levels(3)
                    .storage(mod_api::StorageImageAccess::WriteOnly),
            )
            .expect("a mip-chained R32Float write-only storage target");
        let (view_id, view, format, mips) = targets
            .resolve_storage_image(
                None,
                mod_api::StorageImageAccess::WriteOnly,
                mod_api::StorageImageBinding::new(target, 0, 1),
            )
            .unwrap();

        let quad =
            [[0.0, 0.0], [0.0, 64.0], [64.0, 64.0], [64.0, 0.0]].map(|[x, y]| PipelineVertex {
                pos: [x, y, 0.0],
                uv: [0.0, 0.0],
                color: 0xFFFF_FFFF,
            });
        let vbytes = bytemuck::cast_slice(&quad);
        let draw = PipelineDraw {
            slot: 0,
            stage: 4,
            modifier_stack: 0,
            draw: MeshDraw::vertices(0..4),
            source: PipelineDrawSource::Transient {
                vertex_ranges: vec![(0, 0..vbytes.len() as u64)],
                index_range: None,
            },
            bindings: Some(ResolvedDrawBindings {
                sampler_layout: SamplerLayout::EMPTY,
                entries: vec![ResolvedSet1Entry::StorageImage {
                    view_id,
                    view,
                    format,
                    mips,
                }],
                push_constants: None,
                reads_world_depth: false,
            }),
        };
        let frame_format = wgpu::TextureFormat::Rgba8Unorm;
        let mut primitives = WorldPrimitives::new(&device, (frame_format, false));
        let gpu = (device, queue);
        let batch = Batch {
            set: Some(&set),
            draws: std::slice::from_ref(&draw),
            transient_vertices: vbytes,
            time: 0.0,
            ..Default::default()
        };
        let at = render(&gpu, &mut primitives, frame_format, batch, STAGES - 1);
        assert_near(
            at(32, 32),
            [255.0, 255.0, 255.0, 255.0],
            1.0,
            "the draw ran",
        );

        // Read level 1 (2 x 2) and the single texel of level 0 back.
        let (device, queue) = &gpu;
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: 1024,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let texture = targets.color_texture(target, 0).unwrap();
        let mut encoder = device.create_command_encoder(&Default::default());
        for (mip, offset, size) in [(1, 0, 2), (0, 512, 1)] {
            encoder.copy_texture_to_buffer(
                wgpu::TexelCopyTextureInfo {
                    texture,
                    mip_level: mip,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                wgpu::TexelCopyBufferInfo {
                    buffer: &readback,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset,
                        bytes_per_row: Some(256),
                        rows_per_image: None,
                    },
                },
                wgpu::Extent3d {
                    width: size,
                    height: size,
                    depth_or_array_layers: 1,
                },
            );
        }
        queue.submit([encoder.finish()]);
        readback.slice(..).map_async(wgpu::MapMode::Read, |_| {});
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        let bytes = readback.slice(..).get_mapped_range().unwrap().to_vec();
        let texel =
            |offset: usize| f32::from_ne_bytes(bytes[offset..offset + 4].try_into().unwrap());
        // The shader wrote two texels of level 1; the others and level 0 keep their zeroes.
        assert_eq!(texel(0), 0.25, "level 1 texel (0, 0)");
        assert_eq!(texel(4), 0.5, "level 1 texel (1, 0)");
        assert_eq!(texel(256), 0.0, "level 1 texel (0, 1)");
        assert_eq!(texel(260), 0.0, "level 1 texel (1, 1)");
        assert_eq!(texel(512), 0.0, "level 0 texel (0, 0)");
    }

    fn offscreen_quad_4x4(x0: f32, x1: f32) -> [PipelineVertex; 4] {
        [[x0, 0.0], [x0, 4.0], [x1, 4.0], [x1, 0.0]].map(|[x, y]| PipelineVertex {
            pos: [x, y, 0.0],
            uv: [0.0, 0.0],
            color: 0xFFFF_FFFF,
        })
    }

    fn execute_offscreen_4x4_passes(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        primitives: &mut WorldPrimitives,
        targets: &crate::gfx::mod_render_targets::ModRenderTargets,
        passes: &[(mod_api::RenderTargetRef, &[PipelineDraw])],
        transient_vertices: &[u8],
    ) -> Vec<u8> {
        let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let ortho4 = Mat4::orthographic_rh(0.0, 4.0, 4.0, 0.0, -1.0, 1.0).to_cols_array();
        let identity = Mat4::IDENTITY.to_cols_array();
        let mut encoder = device.create_command_encoder(&Default::default());
        for &(rt, draws) in passes {
            let resolved = targets.resolve_target(Some("test_mod"), rt, 0).unwrap();
            let pass = mod_api::OffscreenPass::new(rt)
                .clear_color_0([0.0, 0.0, 0.0, 1.0])
                .projection(ortho4)
                .view(identity);
            primitives.execute_offscreen_pass(
                device,
                queue,
                &mut encoder,
                &OffscreenPassData {
                    pass: &pass,
                    target: &resolved,
                    draws,
                    transient_vertices,
                    transient_indices: &[],
                    inline_uniforms: &[],
                    texel_data: &[],
                    modifiers: None,
                    modifier_defs: None,
                },
            );
        }
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: (passes.len() as u64) * 1024,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        for (i, &(rt, _)) in passes.iter().enumerate() {
            encoder.copy_texture_to_buffer(
                wgpu::TexelCopyTextureInfo {
                    texture: targets.color_texture(rt, 0).unwrap(),
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                wgpu::TexelCopyBufferInfo {
                    buffer: &readback,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: (i as u64) * 1024,
                        bytes_per_row: Some(256),
                        rows_per_image: None,
                    },
                },
                wgpu::Extent3d {
                    width: 4,
                    height: 4,
                    depth_or_array_layers: 1,
                },
            );
        }
        queue.submit([encoder.finish()]);
        assert!(
            pollster::block_on(validation.pop()).is_none(),
            "offscreen pass must not raise a wgpu validation error"
        );
        readback.slice(..).map_async(wgpu::MapMode::Read, |_| {});
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        let bytes = readback.slice(..).get_mapped_range().unwrap().to_vec();
        readback.unmap();
        bytes
    }

    #[test]
    #[ignore = "daily"]
    fn offscreen_pass_binds_storage_buffer_ranges_and_skips_rw_conflicts() {
        let _gpu = crate::gfx::test_gpu::lock();
        let Some((device, queue)) = device() else {
            return;
        };

        let read_id = Identifier::parse("test:offscreen_ssbo_read").unwrap();
        let write_id = Identifier::parse("test:offscreen_ssbo_write").unwrap();
        let read_fsh = "#version 330\n\
            layout(std430) readonly buffer Tints { vec4 tint[]; };\n\
            out vec4 fragColor;\n\
            void main() {\n\
                fragColor = tint[0];\n\
            }\n";
        let write_fsh = "#version 330\n\
            layout(std430) buffer Tints { vec4 tint[]; };\n\
            out vec4 fragColor;\n\
            void main() {\n\
                tint[0] = vec4(0.0, 0.0, 1.0, 1.0);\n\
                fragColor = vec4(0.0, 0.0, 1.0, 1.0);\n\
            }\n";
        let make_pipeline = |id: Identifier, read_only: bool| {
            RenderPipeline::new(
                id.clone(),
                id,
                PipelineBlend::Opaque,
                mod_api::WorldDepth::ThroughWalls,
            )
            .bind_group_layout(
                mod_api::BindGroupLayout::builder()
                    .entry(mod_api::BindEntry::new(
                        "Tints",
                        mod_api::BindingKind::StorageBuffer {
                            read_only,
                            min_size: 16,
                        },
                    ))
                    .build()
                    .unwrap(),
            )
        };
        let set = Pipelines::build(
            &[make_pipeline(read_id, true), make_pipeline(write_id, false)],
            1,
            |file| {
                if file.ends_with(".vsh") {
                    Ok(Some(VSH.to_owned()))
                } else if file.contains("offscreen_ssbo_read") && file.ends_with(".fsh") {
                    Ok(Some(read_fsh.to_owned()))
                } else if file.contains("offscreen_ssbo_write") && file.ends_with(".fsh") {
                    Ok(Some(write_fsh.to_owned()))
                } else {
                    Ok(stub_asset(file))
                }
            },
        )
        .expect("offscreen storage buffer pipelines must translate");

        let mut storage = crate::gfx::mod_storage::ModStorageBuffers::default();
        let buffer = storage
            .create(&device, None, &mod_api::StorageBufferDescriptor::new(512))
            .unwrap();
        storage
            .write_range(
                None,
                buffer,
                0,
                bytemuck::cast_slice(&[1.0f32, 0.0, 0.0, 1.0]),
            )
            .unwrap();
        storage
            .write_range(
                None,
                buffer,
                256,
                bytemuck::cast_slice(&[0.0f32, 1.0, 0.0, 1.0]),
            )
            .unwrap();
        storage.flush(&device, &queue);
        let limits = device.limits();
        let resolve_range = |offset| {
            storage
                .resolve(&limits, None, buffer.slice(offset, 16), 16)
                .unwrap()
        };

        let mut targets = crate::gfx::mod_render_targets::ModRenderTargets::default();
        let rt = targets
            .create(
                &device,
                Some("test_mod"),
                &mod_api::RenderTargetDescriptor::rgba8([4, 4], mod_api::RenderTargetDepth::None),
            )
            .expect("create 4x4 RGBA8 offscreen target");

        let format = wgpu::TextureFormat::Rgba8Unorm;
        let mut primitives = WorldPrimitives::new(&device, (format, false));
        primitives
            .pipelines
            .build(&device, &set, None)
            .expect("build offscreen storage buffer pipelines");

        let vertices: Vec<PipelineVertex> =
            [offscreen_quad_4x4(0.0, 2.0), offscreen_quad_4x4(2.0, 4.0)].concat();
        let vbytes = bytemuck::cast_slice(&vertices);
        let draw_cmd = |slot: u32, offset: u64, first_quad: u64| {
            let r = resolve_range(offset);
            let stride = 4 * std::mem::size_of::<PipelineVertex>() as u64;
            PipelineDraw {
                slot,
                stage: 0,
                modifier_stack: 0,
                draw: MeshDraw::vertices(0..4),
                source: PipelineDrawSource::Transient {
                    vertex_ranges: vec![(0, first_quad * stride..(first_quad + 1) * stride)],
                    index_range: None,
                },
                bindings: Some(ResolvedDrawBindings {
                    sampler_layout: SamplerLayout::EMPTY,
                    entries: vec![ResolvedSet1Entry::StorageBuffer {
                        id: r.id,
                        buffer: r.buffer,
                        offset: r.offset,
                        size: r.length,
                    }],
                    push_constants: None,
                    reads_world_depth: false,
                }),
            }
        };

        let read_pass_pixels = |primitives: &mut WorldPrimitives, draws: &[PipelineDraw]| {
            let bytes = execute_offscreen_4x4_passes(
                &device,
                &queue,
                primitives,
                &targets,
                &[(rt, draws)],
                vbytes,
            );
            move |x: usize, y: usize| -> [u8; 4] {
                let i = y * 256 + x * 4;
                [bytes[i], bytes[i + 1], bytes[i + 2], bytes[i + 3]]
            }
        };

        // Pass 1: two read-only sub-ranges of the storage buffer render red (left) and green (right).
        let read_draws = [draw_cmd(0, 0, 0), draw_cmd(0, 256, 1)];
        let at = read_pass_pixels(&mut primitives, &read_draws);
        assert_eq!(at(0, 1), [255, 0, 0, 255], "left range red");
        assert_eq!(at(3, 1), [0, 255, 0, 255], "right range green");

        // Pass 2: binding the same storage buffer both read-only and read-write in one offscreen
        // pass skips conflicting draws cleanly without raising a wgpu validation error.
        let conflict_draws = [draw_cmd(0, 0, 0), draw_cmd(1, 256, 1)];
        let at = read_pass_pixels(&mut primitives, &conflict_draws);
        assert_eq!(
            at(0, 1),
            [0, 0, 0, 255],
            "conflicting read-only draw skipped"
        );
        assert_eq!(
            at(3, 1),
            [0, 0, 0, 255],
            "conflicting read-write draw skipped"
        );
    }

    #[test]
    #[ignore = "daily"]
    fn offscreen_targets_skip_linear_light_decode_while_world_keeps_it() {
        let _gpu = crate::gfx::test_gpu::lock();
        let Some((device, queue)) = device() else {
            return;
        };

        let color_id = Identifier::parse("test:hdr_offscreen_color").unwrap();
        let r32f_id = Identifier::parse("test:hdr_offscreen_r32f").unwrap();
        let pow_id = Identifier::parse("test:hdr_explicit_pow").unwrap();
        // Exact dyadic f16/f32 values so offscreen readback is bit-exact and clearly differs from
        // `pow(rgb, 2.2)`.
        let color_fsh = "#version 330\n\
            in vec2 texCoord0;\n\
            in vec4 vertexColor;\n\
            out vec4 fragColor;\n\
            void main() {\n\
                fragColor = vec4(0.75, 0.5, 0.25, 1.0);\n\
            }\n";
        let r32f_fsh = "#version 330\n\
            in vec2 texCoord0;\n\
            in vec4 vertexColor;\n\
            out float fragDepthCopy;\n\
            void main() {\n\
                fragDepthCopy = 0.5;\n\
            }\n";
        // Reference decode evaluated on the same GPU into the same Rgba16Float format so the world
        // HDR readback can be compared bit-exactly without an epsilon tolerance.
        let pow_fsh = "#version 330\n\
            in vec2 texCoord0;\n\
            in vec4 vertexColor;\n\
            out vec4 fragColor;\n\
            void main() {\n\
                fragColor = vec4(pow(vec3(0.75, 0.5, 0.25), vec3(2.2)), 1.0);\n\
            }\n";

        let make = |id: Identifier| {
            RenderPipeline::new(
                id.clone(),
                id,
                PipelineBlend::Opaque,
                mod_api::WorldDepth::ThroughWalls,
            )
        };
        let set = Pipelines::build(&[make(color_id), make(r32f_id), make(pow_id)], 1, |file| {
            if file.ends_with(".vsh") {
                Ok(Some(VSH.to_owned()))
            } else if file.contains("hdr_offscreen_color") && file.ends_with(".fsh") {
                Ok(Some(color_fsh.to_owned()))
            } else if file.contains("hdr_offscreen_r32f") && file.ends_with(".fsh") {
                Ok(Some(r32f_fsh.to_owned()))
            } else if file.contains("hdr_explicit_pow") && file.ends_with(".fsh") {
                Ok(Some(pow_fsh.to_owned()))
            } else {
                Ok(stub_asset(file))
            }
        })
        .expect("HDR offscreen color, R32Float and reference pow pipelines must translate");

        let world_format = wgpu::TextureFormat::Rgba16Float;
        let mut primitives = WorldPrimitives::new(&device, (world_format, true));
        primitives
            .pipelines
            .build(&device, &set, None)
            .expect("HDR linear_light build must accept both vec4 and out float shaders");

        let mut targets = crate::gfx::mod_render_targets::ModRenderTargets::default();
        let color_rt = targets
            .create(
                &device,
                Some("test_mod"),
                &mod_api::RenderTargetDescriptor::new([4, 4])
                    .color_target(mod_api::RenderTargetFormat::Rgba16Float),
            )
            .expect("create 4x4 Rgba16Float offscreen target");
        let r32f_rt = targets
            .create(
                &device,
                Some("test_mod"),
                &mod_api::RenderTargetDescriptor::new([4, 4])
                    .color_target(mod_api::RenderTargetFormat::R32Float),
            )
            .expect("create 4x4 R32Float offscreen target");
        let ref_pow_rt = targets
            .create(
                &device,
                Some("test_mod"),
                &mod_api::RenderTargetDescriptor::new([4, 4])
                    .color_target(mod_api::RenderTargetFormat::Rgba16Float),
            )
            .expect("create 4x4 Rgba16Float reference pow target");

        let full_quad = offscreen_quad_4x4(0.0, 4.0);
        let vbytes = bytemuck::cast_slice(&full_quad);
        let offscreen_draw = |slot: u32| PipelineDraw {
            slot,
            stage: 0,
            modifier_stack: 0,
            draw: MeshDraw::vertices(0..4),
            source: PipelineDrawSource::Transient {
                vertex_ranges: vec![(0, 0..vbytes.len() as u64)],
                index_range: None,
            },
            bindings: None,
        };
        let color_draw = [offscreen_draw(0)];
        let r32f_draw = [offscreen_draw(1)];
        let ref_pow_draw = [offscreen_draw(2)];
        let bytes = execute_offscreen_4x4_passes(
            &device,
            &queue,
            &mut primitives,
            &targets,
            &[
                (color_rt, &color_draw),
                (r32f_rt, &r32f_draw),
                (ref_pow_rt, &ref_pow_draw),
            ],
            vbytes,
        );

        let read_rgba16f = |base: usize| -> [f32; 4] {
            let px = base + 256 + 8;
            std::array::from_fn(|c| {
                let at = px + c * 2;
                half::f16::from_le_bytes([bytes[at], bytes[at + 1]]).to_f32()
            })
        };

        // Offscreen Rgba16Float keeps exact raw [0.75, 0.5, 0.25, 1.0] (not linear-decoded).
        let offscreen_rgba = read_rgba16f(0);
        assert_eq!(
            offscreen_rgba,
            [0.75, 0.5, 0.25, 1.0],
            "offscreen Rgba16Float must skip linear_fragment_finish"
        );

        // Offscreen R32Float keeps exact 0.5 (not pow(0.5, 2.2)).
        let r32_px = 1024 + 256 + 4;
        let r32_val = f32::from_ne_bytes(bytes[r32_px..r32_px + 4].try_into().unwrap());
        assert_eq!(
            r32_val, 0.5,
            "offscreen R32Float must preserve exact raw 0.5"
        );

        // Reference explicit GPU `pow(vec3(0.75, 0.5, 0.25), vec3(2.2))` in Rgba16Float.
        let expected_linear_rgba = read_rgba16f(2048);
        assert_ne!(
            expected_linear_rgba,
            [0.75, 0.5, 0.25, 1.0],
            "explicit pow(rgb, 2.2) reference must differ from raw gamma values"
        );

        // Drawing the same vec4 pipeline into the HDR world target matches the explicit GPU
        // `pow(rgb, 2.2)` decode bit-exactly.
        let world_quad = quad([0.0, 0.0, 64.0, 64.0], 2.0, [255; 4]);
        let world_vbytes = bytemuck::cast_slice(&world_quad);
        let world_draw = PipelineDraw {
            slot: 0,
            stage: 0,
            modifier_stack: 0,
            draw: MeshDraw::vertices(0..4),
            source: PipelineDrawSource::Transient {
                vertex_ranges: vec![(0, 0..world_vbytes.len() as u64)],
                index_range: None,
            },
            bindings: None,
        };
        let gpu = (device, queue);
        let batch = Batch {
            set: Some(&set),
            draws: std::slice::from_ref(&world_draw),
            transient_vertices: world_vbytes,
            time: 0.0,
            ..Default::default()
        };
        let at = render(&gpu, &mut primitives, world_format, batch, STAGES - 1);
        assert_eq!(
            at(32, 32),
            expected_linear_rgba,
            "world HDR target must match explicit GPU linear_fragment_finish decode bit-exactly"
        );
    }

    #[test]
    #[ignore = "daily"]
    fn stencil_test_masks_offscreen_target_ignores_non_stencil_target_and_modifier_disables_it() {
        use mod_api::{
            DepthState, PipelineModifier, StencilOperation, StencilPerFaceTest, StencilTest,
        };

        let _gpu = crate::gfx::test_gpu::lock();
        let Some((device, queue)) = device() else {
            return;
        };

        let stamp_id = Identifier::parse("test:stencil_stamp_zero").unwrap();
        let eq_one_id = Identifier::parse("test:stencil_draw_eq_one").unwrap();
        let plain_id = Identifier::parse("test:stencil_plain_green").unwrap();
        let red_fsh = "#version 330\n\
            out vec4 fragColor;\n\
            void main() {\n\
                fragColor = vec4(1.0, 0.0, 0.0, 1.0);\n\
            }\n";
        let green_fsh = "#version 330\n\
            out vec4 fragColor;\n\
            void main() {\n\
                fragColor = vec4(0.0, 1.0, 0.0, 1.0);\n\
            }\n";

        let stamp_stencil = StencilTest::both(
            StencilPerFaceTest::new(
                StencilOperation::Keep,
                StencilOperation::Keep,
                StencilOperation::Replace,
                DepthCompare::Always,
            ),
            u32::MAX,
            u32::MAX,
            0,
        );
        let eq_one_stencil = StencilTest::both(
            StencilPerFaceTest::new(
                StencilOperation::Keep,
                StencilOperation::Keep,
                StencilOperation::Keep,
                DepthCompare::Equal,
            ),
            u32::MAX,
            0,
            1,
        );

        let pipelines = [
            RenderPipeline::new(
                stamp_id.clone(),
                stamp_id,
                PipelineBlend::Opaque,
                mod_api::WorldDepth::ThroughWalls,
            )
            .depth_stencil(Some(
                DepthState::new(DepthCompare::Always, false).stencil_test(Some(stamp_stencil)),
            )),
            RenderPipeline::new(
                eq_one_id.clone(),
                eq_one_id,
                PipelineBlend::Opaque,
                mod_api::WorldDepth::ThroughWalls,
            )
            .depth_stencil(Some(
                DepthState::new(DepthCompare::Always, false).stencil_test(Some(eq_one_stencil)),
            )),
            RenderPipeline::new(
                plain_id.clone(),
                plain_id,
                PipelineBlend::Opaque,
                mod_api::WorldDepth::ThroughWalls,
            )
            .depth_stencil(Some(DepthState::new(DepthCompare::Always, false))),
        ];
        let set = Pipelines::build(&pipelines, 1, |file| {
            if file.ends_with(".vsh") {
                Ok(Some(VSH.to_owned()))
            } else if file.contains("stencil_stamp_zero") && file.ends_with(".fsh") {
                Ok(Some(red_fsh.to_owned()))
            } else if file.ends_with(".fsh") {
                Ok(Some(green_fsh.to_owned()))
            } else {
                Ok(stub_asset(file))
            }
        })
        .expect("stencil test pipelines must translate");

        let mut targets = crate::gfx::mod_render_targets::ModRenderTargets::default();
        let stencil_rt = targets
            .create(
                &device,
                Some("test_mod"),
                &mod_api::RenderTargetDescriptor::rgba8(
                    [4, 4],
                    mod_api::RenderTargetDepth::Depth24PlusStencil8,
                ),
            )
            .expect("create 4x4 Depth24PlusStencil8 offscreen target");
        let depth_only_rt = targets
            .create(
                &device,
                Some("test_mod"),
                &mod_api::RenderTargetDescriptor::rgba8(
                    [4, 4],
                    mod_api::RenderTargetDepth::Depth32Float,
                ),
            )
            .expect("create 4x4 Depth32Float offscreen target");

        let format = wgpu::TextureFormat::Rgba8Unorm;
        let mut primitives = WorldPrimitives::new(&device, (format, false));
        primitives
            .pipelines
            .build(&device, &set, None)
            .expect("build stencil test pipelines");

        let vertices: Vec<PipelineVertex> =
            [offscreen_quad_4x4(0.0, 2.0), offscreen_quad_4x4(0.0, 4.0)].concat();
        let vbytes = bytemuck::cast_slice(&vertices);
        let stride = 4 * std::mem::size_of::<PipelineVertex>() as u64;
        let make_draw = |slot: u32, quad_idx: u64, modifier_stack: u32| PipelineDraw {
            slot,
            stage: 0,
            modifier_stack,
            draw: MeshDraw::vertices(0..4),
            source: PipelineDrawSource::Transient {
                vertex_ranges: vec![(0, quad_idx * stride..(quad_idx + 1) * stride)],
                index_range: None,
            },
            bindings: None,
        };

        let ortho4 = Mat4::orthographic_rh(0.0, 4.0, 4.0, 0.0, -1.0, 1.0).to_cols_array();
        let identity = Mat4::IDENTITY.to_cols_array();
        let run_pass = |primitives: &mut WorldPrimitives,
                        rt: mod_api::RenderTargetRef,
                        clear_stencil: Option<u32>,
                        draws: &[PipelineDraw],
                        modifiers: Option<&ModifierStacks>,
                        modifier_defs: Option<&[PipelineModifier]>|
         -> Vec<u8> {
            let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);
            let resolved = targets.resolve_target(Some("test_mod"), rt, 0).unwrap();
            let mut pass = mod_api::OffscreenPass::new(rt)
                .clear_color_0([0.0, 0.0, 0.0, 1.0])
                .clear_depth(0.0)
                .projection(ortho4)
                .view(identity);
            if let Some(s) = clear_stencil {
                pass = pass.clear_stencil(s);
            }
            let mut encoder = device.create_command_encoder(&Default::default());
            primitives.execute_offscreen_pass(
                &device,
                &queue,
                &mut encoder,
                &OffscreenPassData {
                    pass: &pass,
                    target: &resolved,
                    draws,
                    transient_vertices: vbytes,
                    transient_indices: &[],
                    inline_uniforms: &[],
                    texel_data: &[],
                    modifiers,
                    modifier_defs,
                },
            );
            let readback = device.create_buffer(&wgpu::BufferDescriptor {
                label: None,
                size: 1024,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });
            encoder.copy_texture_to_buffer(
                wgpu::TexelCopyTextureInfo {
                    texture: targets.color_texture(rt, 0).unwrap(),
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                wgpu::TexelCopyBufferInfo {
                    buffer: &readback,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(256),
                        rows_per_image: None,
                    },
                },
                wgpu::Extent3d {
                    width: 4,
                    height: 4,
                    depth_or_array_layers: 1,
                },
            );
            queue.submit([encoder.finish()]);
            assert!(
                pollster::block_on(validation.pop()).is_none(),
                "stencil offscreen pass must not raise a wgpu validation error"
            );
            readback.slice(..).map_async(wgpu::MapMode::Read, |_| {});
            device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
            let bytes = readback.slice(..).get_mapped_range().unwrap().to_vec();
            readback.unmap();
            bytes
        };
        let pixel = |bytes: &[u8], x: usize, y: usize| -> [u8; 4] {
            let i = y * 256 + x * 4;
            [bytes[i], bytes[i + 1], bytes[i + 2], bytes[i + 3]]
        };

        // 1. On Depth24PlusStencil8: clear stencil to 1, stamp left half to 0 (red), then draw
        // green full-screen with stencil Equal 1 -> left stays red, right turns green.
        let masked = run_pass(
            &mut primitives,
            stencil_rt,
            Some(1),
            &[make_draw(0, 0, 0), make_draw(1, 1, 0)],
            None,
            None,
        );
        assert_eq!(
            pixel(&masked, 0, 1),
            [255, 0, 0, 255],
            "left half stamped to stencil 0 must reject Equal 1 draw"
        );
        assert_eq!(
            pixel(&masked, 3, 1),
            [0, 255, 0, 255],
            "right half kept at stencil 1 must pass Equal 1 draw"
        );

        // 2. A PipelineModifier with `stencil_test(None)` (`Some(None)`) disables stencil testing
        // on the same Depth24PlusStencil8 target, so green overwrites the stamped left half too.
        let modifier_defs =
            [
                PipelineModifier::new(Identifier::parse("test:disable_stencil").unwrap())
                    .stencil_test(None),
            ];
        let mut stacks = ModifierStacks::new();
        let disable_stencil = stacks.intern(&[0]);
        let unmasked = run_pass(
            &mut primitives,
            stencil_rt,
            Some(1),
            &[make_draw(0, 0, 0), make_draw(1, 1, disable_stencil)],
            Some(&stacks),
            Some(&modifier_defs),
        );
        assert_eq!(
            pixel(&unmasked, 0, 1),
            [0, 255, 0, 255],
            "modifier Some(None) must disable stencil test on left half"
        );
        assert_eq!(
            pixel(&unmasked, 3, 1),
            [0, 255, 0, 255],
            "modifier Some(None) must still draw right half"
        );

        // 3. On Depth32Float (no stencil plane): the stencil pipeline is not refused and renders
        // identically to the same pipeline with `stencil_test: None`.
        let with_stencil = run_pass(
            &mut primitives,
            depth_only_rt,
            None,
            &[make_draw(1, 1, 0)],
            None,
            None,
        );
        let without_stencil = run_pass(
            &mut primitives,
            depth_only_rt,
            None,
            &[make_draw(2, 1, 0)],
            None,
            None,
        );
        for y in 0..4 {
            for x in 0..4 {
                assert_eq!(
                    pixel(&with_stencil, x, y),
                    pixel(&without_stencil, x, y),
                    "Depth32Float pixel ({x}, {y}) must match pipeline without stencil"
                );
            }
        }
        assert_eq!(
            pixel(&with_stencil, 1, 1),
            [0, 255, 0, 255],
            "stencil pipeline on Depth32Float must draw green"
        );
    }

    #[test]
    #[ignore = "daily"]
    fn early_depth_test_gates_unsupported_devices_and_skips_storage_writes_when_occluded() {
        use mod_api::{
            DepthState, RenderCapabilities, RenderPipelineError, StorageBufferDescriptor,
        };

        let _gpu = crate::gfx::test_gpu::lock();
        let Some(adapter) = crate::gfx::test_gpu::adapter() else {
            return;
        };

        let blocker_id = Identifier::parse("test:early_z_blocker").unwrap();
        let early_id = Identifier::parse("test:early_z_raster").unwrap();
        let storage_id = Identifier::parse("test:frag_storage_only").unwrap();
        let ndc_vsh = "#version 330\n\
            in vec3 Position;\n\
            in vec2 UV0;\n\
            in vec4 Color;\n\
            void main() {\n\
                gl_Position = vec4(Position.x / 2.0 - 1.0, 1.0 - Position.y / 2.0, Position.z, 1.0);\n\
            }\n";
        let blocker_fsh = "#version 330\n\
            out vec4 fragColor;\n\
            void main() {\n\
                fragColor = vec4(1.0);\n\
            }\n";
        let early_fsh = "#version 330\n\
            layout(early_fragment_tests) in;\n\
            layout(std430) buffer Vis { uint visible[]; };\n\
            out vec4 fragColor;\n\
            void main() {\n\
                visible[0] = 1u;\n\
                fragColor = vec4(1.0);\n\
            }\n";
        let storage_fsh = "#version 330\n\
            layout(std430) buffer Vis { uint visible[]; };\n\
            out vec4 fragColor;\n\
            void main() {\n\
                visible[0] = 1u;\n\
                fragColor = vec4(1.0);\n\
            }\n";

        let storage_layout = || {
            mod_api::BindGroupLayout::builder()
                .entry(mod_api::BindEntry::new(
                    "Vis",
                    mod_api::BindingKind::StorageBuffer {
                        read_only: false,
                        min_size: 4,
                    },
                ))
                .build()
                .unwrap()
        };
        let make_set = |generation: u64| {
            let pipelines = [
                RenderPipeline::new(
                    blocker_id.clone(),
                    blocker_id.clone(),
                    PipelineBlend::Opaque,
                    mod_api::WorldDepth::Tested,
                )
                .depth_stencil(Some(DepthState::new(DepthCompare::GreaterEqual, true)))
                .optional(),
                RenderPipeline::new(
                    early_id.clone(),
                    early_id.clone(),
                    PipelineBlend::Opaque,
                    mod_api::WorldDepth::Tested,
                )
                .depth_stencil(Some(DepthState::new(DepthCompare::GreaterEqual, false)))
                .bind_group_layout(storage_layout())
                .optional(),
                RenderPipeline::new(
                    storage_id.clone(),
                    storage_id.clone(),
                    PipelineBlend::Opaque,
                    mod_api::WorldDepth::Tested,
                )
                .depth_stencil(Some(DepthState::new(DepthCompare::GreaterEqual, false)))
                .bind_group_layout(storage_layout())
                .optional(),
            ];
            Pipelines::build(&pipelines, generation, |file| {
                if file.ends_with(".vsh") {
                    Ok(Some(ndc_vsh.to_owned()))
                } else if file.contains("early_z_blocker") && file.ends_with(".fsh") {
                    Ok(Some(blocker_fsh.to_owned()))
                } else if file.contains("early_z_raster") && file.ends_with(".fsh") {
                    Ok(Some(early_fsh.to_owned()))
                } else if file.contains("frag_storage_only") && file.ends_with(".fsh") {
                    Ok(Some(storage_fsh.to_owned()))
                } else {
                    Ok(stub_asset(file))
                }
            })
            .expect("early-Z and fragment-storage pipelines must translate")
        };

        // 1. A device without SHADER_EARLY_DEPTH_TEST refuses the early_fragment_tests pipeline
        // with RenderPipelineError::UnsupportedFeature while keeping the other pipelines.
        {
            let Some((device_without, _queue_without)) =
                crate::gfx::test_gpu::optional_device_with(&adapter, &Default::default())
            else {
                return;
            };
            let set = make_set(1);
            let mut renderer =
                ModPipelines::new(&device_without, (wgpu::TextureFormat::Rgba8Unorm, false));
            renderer
                .build(&device_without, &set, None)
                .expect("optional unsupported feature pipeline must not fail load");
            assert!(set.resolve(&blocker_id).is_ok());
            assert_eq!(
                set.resolve(&early_id),
                Err(RenderPipelineError::UnsupportedFeature)
            );
            assert!(set.resolve(&storage_id).is_ok());

            // When fragment_writable_storage is also false (e.g. Vulkan without
            // fragmentStoresAndAtomics or OpenGL < 4.3), both storage-writing pipelines refuse
            // with UnsupportedFeature.
            let set_no_storage = make_set(2);
            let mut gated =
                ModPipelines::new(&device_without, (wgpu::TextureFormat::Rgba8Unorm, false));
            gated.set_capabilities(&RenderCapabilities::default());
            gated
                .build(&device_without, &set_no_storage, None)
                .expect("optional unsupported storage pipeline must not fail load");
            assert!(set_no_storage.resolve(&blocker_id).is_ok());
            assert_eq!(
                set_no_storage.resolve(&early_id),
                Err(RenderPipelineError::UnsupportedFeature)
            );
            assert_eq!(
                set_no_storage.resolve(&storage_id),
                Err(RenderPipelineError::UnsupportedFeature)
            );
        }

        // 2. On an adapter that supports forced early-Z (Vulkan/OpenGL, non-Adreno), occluded
        // fragments must skip their SSBO write while visible fragments execute it.
        if !adapter
            .features()
            .contains(wgpu::Features::SHADER_EARLY_DEPTH_TEST)
        {
            return;
        }
        let Some((device, queue)) = crate::gfx::test_gpu::optional_device_with(
            &adapter,
            &wgpu::DeviceDescriptor {
                required_features: wgpu::Features::SHADER_EARLY_DEPTH_TEST,
                ..Default::default()
            },
        ) else {
            return;
        };
        let caps = crate::gfx::mod_render_targets::probe_render_capabilities(&adapter, &device);
        if !caps.shader_early_depth_test || !caps.fragment_writable_storage {
            return;
        }

        let set = make_set(3);
        let format = wgpu::TextureFormat::Rgba8Unorm;
        let mut primitives = WorldPrimitives::new(&device, (format, false));
        primitives.pipelines.set_capabilities(&caps);
        primitives
            .pipelines
            .build(&device, &set, None)
            .expect("build early-Z pipelines on capable device");
        assert!(set.resolve(&early_id).is_ok());

        let mut targets = crate::gfx::mod_render_targets::ModRenderTargets::default();
        let rt = targets
            .create(
                &device,
                Some("test_mod"),
                &mod_api::RenderTargetDescriptor::rgba8(
                    [4, 4],
                    mod_api::RenderTargetDepth::Depth32Float,
                ),
            )
            .expect("create 4x4 Depth32Float target");
        let resolved_rt = targets.resolve_target(Some("test_mod"), rt, 0).unwrap();

        let mut storage = crate::gfx::mod_storage::ModStorageBuffers::default();
        let buffer = storage
            .create(&device, None, &StorageBufferDescriptor::new(512).copy_src())
            .unwrap();
        storage.write_range(None, buffer, 0, &[0u8; 4]).unwrap();
        storage.write_range(None, buffer, 256, &[0u8; 4]).unwrap();
        storage.flush(&device, &queue);
        let limits = device.limits();
        let resolve_slice = |offset: u64, len: u64| {
            storage
                .resolve(&limits, None, buffer.slice(offset, len), 4)
                .unwrap()
        };

        let ndc_quad = |x0: f32, x1: f32, z: f32| {
            [[x0, 0.0], [x0, 4.0], [x1, 4.0], [x1, 0.0]].map(|[x, y]| PipelineVertex {
                pos: [x, y, z],
                uv: [0.0, 0.0],
                color: 0xFFFF_FFFF,
            })
        };
        // Reversed-Z: depth clears to 0.0, GreaterEqual passes nearer (larger) Z.
        // Quad 0: blocker at z=0.8 on left half; Quad 1: occluded at z=0.2 on left half;
        // Quad 2: visible at z=0.2 on right half.
        let vertices: Vec<PipelineVertex> = [
            ndc_quad(0.0, 2.0, 0.8),
            ndc_quad(0.0, 2.0, 0.2),
            ndc_quad(2.0, 4.0, 0.2),
        ]
        .concat();
        let vbytes = bytemuck::cast_slice(&vertices);
        let stride = 4 * std::mem::size_of::<PipelineVertex>() as u64;
        let quad_draw = |slot: u32, quad_idx: u64, storage_offset: Option<u64>| PipelineDraw {
            slot,
            stage: 0,
            modifier_stack: 0,
            draw: MeshDraw::vertices(0..4),
            source: PipelineDrawSource::Transient {
                vertex_ranges: vec![(0, quad_idx * stride..(quad_idx + 1) * stride)],
                index_range: None,
            },
            bindings: storage_offset.map(|offset| {
                let r = resolve_slice(offset, 16);
                ResolvedDrawBindings {
                    sampler_layout: SamplerLayout::EMPTY,
                    entries: vec![ResolvedSet1Entry::StorageBuffer {
                        id: r.id,
                        buffer: r.buffer,
                        offset: r.offset,
                        size: r.length,
                    }],
                    push_constants: None,
                    reads_world_depth: false,
                }
            }),
        };

        let draws = [
            quad_draw(0, 0, None),
            quad_draw(1, 1, Some(0)),
            quad_draw(1, 2, Some(256)),
        ];
        let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let pass = mod_api::OffscreenPass::new(rt)
            .clear_color_0([0.0, 0.0, 0.0, 1.0])
            .clear_depth(0.0);
        let mut encoder = device.create_command_encoder(&Default::default());
        primitives.execute_offscreen_pass(
            &device,
            &queue,
            &mut encoder,
            &OffscreenPassData {
                pass: &pass,
                target: &resolved_rt,
                draws: &draws,
                transient_vertices: vbytes,
                transient_indices: &[],
                inline_uniforms: &[],
                texel_data: &[],
                modifiers: None,
                modifier_defs: None,
            },
        );
        let resolved_all = resolve_slice(0, 512);
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: 512,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        encoder.copy_buffer_to_buffer(&resolved_all.buffer, 0, &readback, 0, 512);
        queue.submit([encoder.finish()]);
        assert!(
            pollster::block_on(validation.pop()).is_none(),
            "early_fragment_tests offscreen pass must not raise a wgpu validation error"
        );
        readback.slice(..).map_async(wgpu::MapMode::Read, |_| {});
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        let bytes = readback.slice(..).get_mapped_range().unwrap().to_vec();
        readback.unmap();

        let occluded = u32::from_ne_bytes(bytes[0..4].try_into().unwrap());
        let visible = u32::from_ne_bytes(bytes[256..260].try_into().unwrap());
        assert_eq!(
            occluded, 0,
            "depth-occluded early_fragment_tests draw must skip SSBO write"
        );
        assert_eq!(
            visible, 1,
            "unoccluded early_fragment_tests draw must execute SSBO write"
        );
    }
}
