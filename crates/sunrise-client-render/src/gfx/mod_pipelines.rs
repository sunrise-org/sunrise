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
use super::world_primitives::{Upload, VertexPages};
use crate::mod_pipelines::{Built, BuiltTexture, Pipelines};
use glam::Mat4;
use mod_api::{PipelineBlend, WorldDepth};
use sunrise_mod_pipeline::{ENTRY, Options as PipelineOptions, SPRITE_RECT_BINDING};

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
    /// The stage it draws at (`STAGE_AFTER_OPAQUE_BLOCKS..=STAGE_CUSTOM_GEOMETRY`).
    pub stage: u32,
    pub quads: u32,
}

/// A frame's pipeline quads, as the game queued them: 4 vertices a quad, in the runs' order.
#[derive(Clone, Copy, Debug, Default)]
pub struct Batch<'a> {
    /// The pipelines of the resources the frame draws with; `None` draws nothing.
    pub set: Option<&'a Pipelines>,
    pub vertices: &'a [PipelineVertex],
    pub runs: &'a [PipelineRun],
    /// Seconds of game time (fallback `Globals.GameTime` when no full vanilla block is supplied).
    pub time: f32,
}

/// A run as uploaded: where its quads start in the vertex buffer.
struct Run {
    slot: usize,
    stage: usize,
    quads: std::ops::Range<u32>,
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

/// Target attachment and depth-compare state for [`ModPipelines::build_render_pipeline`].
#[derive(Clone, Copy)]
struct TargetState {
    color_format: wgpu::TextureFormat,
    depth_compare: wgpu::CompareFunction,
    write_mask: wgpu::ColorWrites,
}

/// A built pipeline: its compiled render pipeline, its per-pipeline bind group (binding only
/// the set-0 blocks/samplers its stages read, backed by the shared vanilla uniform buffer plus
/// optional sprite texture/rect), and the shader modules and fixed state needed to lazily
/// compile a shader-pack target variant.
struct Compiled {
    pipeline: wgpu::RenderPipeline,
    pipeline_layout: wgpu::PipelineLayout,
    group: wgpu::BindGroup,
    pack_group: wgpu::BindGroup,
    vertex_shader: wgpu::ShaderModule,
    /// Fragment shader targeting the main frame (linear-decoded when `linear_light`).
    fragment_shader: wgpu::ShaderModule,
    /// Vanilla-gamma fragment shader for shader-pack `colortex0` targets (when `linear_light`
    /// differs from the pack's gamma encoding, `pack_fragment_shader` holds the undecoded stage).
    pack_fragment_shader: Option<wgpu::ShaderModule>,
    blend: PipelineBlend,
    depth: WorldDepth,
    #[cfg(test)]
    textured: bool,
}

/// The pipelines and the buffer that draw a [`Batch`].
pub struct ModPipelines {
    color_format: wgpu::TextureFormat,
    linear_light: bool,
    /// Shared nearest/default sampler for `Sampler0`.
    sampler: wgpu::Sampler,
    /// Shared 1024-byte vanilla uniform buffer (`Globals`, `Projection`, `DynamicTransforms`, `Fog`).
    vanilla: wgpu::Buffer,
    /// Same blocks with `Projection` adapted to shader-pack GL coordinates (`pack_clip_matrix`).
    pack_vanilla: wgpu::Buffer,
    vertices: VertexPages,
    /// The load `pipelines` was built from; 0 before any.
    generation: u64,
    /// By slot; `None` for a pipeline the load refused.
    pipelines: Vec<Option<Compiled>>,
    /// Lazily built for a shader pack's `(colortex0 format, rgb_only)` + GL `[0..1]` depth.
    pack_pipelines: Option<(
        (wgpu::TextureFormat, bool),
        Vec<Option<wgpu::RenderPipeline>>,
    )>,
    runs: Vec<Run>,
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
            sampler,
            vanilla,
            pack_vanilla,
            vertices: VertexPages::default(),
            generation: 0,
            pipelines: Vec::new(),
            pack_pipelines: None,
            runs: Vec::new(),
        }
    }

    fn build_render_pipeline(
        device: &wgpu::Device,
        pipeline_layout: &wgpu::PipelineLayout,
        vertex_shader: &wgpu::ShaderModule,
        fragment_shader: &wgpu::ShaderModule,
        blend: PipelineBlend,
        depth: WorldDepth,
        target: TargetState,
    ) -> wgpu::RenderPipeline {
        let blend_state = match blend {
            PipelineBlend::Additive => wgpu::BlendState {
                color: wgpu::BlendComponent {
                    src_factor: wgpu::BlendFactor::SrcAlpha,
                    dst_factor: wgpu::BlendFactor::One,
                    operation: wgpu::BlendOperation::Add,
                },
                alpha: wgpu::BlendComponent {
                    src_factor: wgpu::BlendFactor::Zero,
                    dst_factor: wgpu::BlendFactor::One,
                    operation: wgpu::BlendOperation::Add,
                },
            },
            _ => wgpu::BlendState::ALPHA_BLENDING,
        };
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Mod pipeline"),
            layout: Some(pipeline_layout),
            vertex: wgpu::VertexState {
                module: vertex_shader,
                entry_point: Some(ENTRY),
                compilation_options: Default::default(),
                buffers: &[Some(PipelineVertex::buffer_layout())],
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                // Two-sided, as world quads are.
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: Some(false),
                depth_compare: Some(match depth {
                    WorldDepth::ThroughWalls => wgpu::CompareFunction::Always,
                    _ => target.depth_compare,
                }),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: fragment_shader,
                entry_point: Some(ENTRY),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: target.color_format,
                    blend: Some(blend_state),
                    write_mask: target.write_mask,
                })],
            }),
            multiview_mask: None,
            cache: None,
        })
    }

    /// One load's pipeline: its translated GLSL naga modules with the fixed state it registered,
    /// its per-pipeline set-0 layout and bind groups (`vanilla` and `pack_vanilla`), and its
    /// sprite texture/rect when textured — or the GPU's error.
    fn pipeline(
        &self,
        device: &wgpu::Device,
        built: &Built,
        textures: Option<TextureSources<'_>>,
    ) -> Result<Compiled, String> {
        use wgpu::util::DeviceExt;
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
                    ));
                };
                Some((sources.atlas.clone(), rect))
            }
        };

        let linear_fragment_module = if self.linear_light {
            let define_refs: Vec<&str> = built.defines.iter().map(String::as_str).collect();
            let options = PipelineOptions {
                textured: built.texture.is_some(),
                defines: &define_refs,
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
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Mod pipeline"),
            bind_group_layouts: &[Some(&group_layout)],
            ..Default::default()
        });
        let pipeline = Self::build_render_pipeline(
            device,
            &pipeline_layout,
            &vertex_shader,
            &fragment_shader,
            built.blend,
            built.depth,
            TargetState {
                color_format: self.color_format,
                depth_compare: wgpu::CompareFunction::GreaterEqual,
                write_mask: wgpu::ColorWrites::ALL,
            },
        );
        let internal = pollster::block_on(internal.pop());
        let validation = pollster::block_on(validation.pop());
        match internal.or(validation) {
            None => Ok(Compiled {
                pipeline,
                pipeline_layout,
                group,
                pack_group,
                vertex_shader,
                fragment_shader,
                pack_fragment_shader,
                blend: built.blend,
                depth: built.depth,
                #[cfg(test)]
                textured: sprite_binding.is_some(),
            }),
            Some(error) => Err(error.to_string()),
        }
    }

    /// Ensure pack-target variants (`colortex0` format + GL `[0..1]` `LessEqual` depth) exist
    /// for all loaded mod pipelines.
    pub fn ensure_pack_pipelines(
        &mut self,
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        rgb: bool,
    ) {
        let key = (format, rgb);
        if self
            .pack_pipelines
            .as_ref()
            .is_some_and(|(prev, list)| *prev == key && list.len() == self.pipelines.len())
        {
            return;
        }
        let target = TargetState {
            color_format: format,
            depth_compare: wgpu::CompareFunction::LessEqual,
            write_mask: if rgb {
                wgpu::ColorWrites::COLOR
            } else {
                wgpu::ColorWrites::ALL
            },
        };
        let compiled = self
            .pipelines
            .iter()
            .enumerate()
            .map(|(slot, entry)| {
                let c = entry.as_ref()?;
                let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);
                let internal = device.push_error_scope(wgpu::ErrorFilter::Internal);
                let frag = c
                    .pack_fragment_shader
                    .as_ref()
                    .unwrap_or(&c.fragment_shader);
                let pipeline = Self::build_render_pipeline(
                    device,
                    &c.pipeline_layout,
                    &c.vertex_shader,
                    frag,
                    c.blend,
                    c.depth,
                    target,
                );
                let internal = pollster::block_on(internal.pop());
                let validation = pollster::block_on(validation.pop());
                match internal.or(validation) {
                    None => Some(pipeline),
                    Some(error) => {
                        log::warn!(
                            "Sunrise: mod pipeline slot {slot} refused for shader-pack target {format:?}: {error}"
                        );
                        None
                    }
                }
            })
            .collect();
        self.pack_pipelines = Some((key, compiled));
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
        self.build_with(set, |this, built| this.pipeline(device, built, textures))
    }

    fn build_with(
        &mut self,
        set: &Pipelines,
        compile: impl Fn(&Self, &Built) -> Result<Compiled, String>,
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
            .map(|(_, built, error)| format!("required {}", gpu_refusal(built, error)))
            .collect();
        if !required.is_empty() {
            return Err(required.join("; "));
        }
        for (slot, built, error) in refused {
            if set.invalidate(slot) {
                log::warn!("Sunrise: {}", gpu_refusal(built, &error));
            }
        }
        for (slot, pipeline) in pipelines.iter().enumerate() {
            if pipeline.is_some() {
                set.revalidate(slot);
            }
        }
        self.generation = set.generation();
        self.pipelines = pipelines;
        self.pack_pipelines = None;
        Ok(())
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
        if batch
            .set
            .is_none_or(|set| set.generation() != self.generation)
        {
            return 0;
        }
        let mut next = 0;
        for run in batch.runs {
            let quads = next..next + run.quads;
            next = quads.end;
            if quads.end as usize * 4 > batch.vertices.len()
                || !matches!(self.pipelines.get(run.slot as usize), Some(Some(_)))
            {
                continue;
            }
            self.runs.push(Run {
                slot: run.slot as usize,
                stage: run.stage as usize,
                quads,
            });
        }
        let Some(last) = self.runs.last() else {
            return 0;
        };
        let quads = last.quads.end;
        self.vertices.upload(
            device,
            upload,
            "Mod pipeline quads",
            bytemuck::cast_slice(&batch.vertices[..quads as usize * 4]),
            4 * std::mem::size_of::<PipelineVertex>() as u64,
            0,
        );
        let fallback;
        let blocks = match vanilla_blocks {
            Some(b) => b,
            None => {
                fallback = fallback_vanilla_blocks(view_proj, viewport, batch.time);
                &fallback
            }
        };
        upload(&self.vanilla, blocks);
        let pack_blocks = pack_vanilla_blocks(blocks);
        upload(&self.pack_vanilla, &pack_blocks);
        self.vertices.index_quads(quads)
    }

    /// Whether any prepared runs fall in stages `first..=last`.
    pub fn has_runs(&self, first: usize, last: usize) -> bool {
        self.runs
            .iter()
            .any(|run| first <= run.stage && run.stage <= last)
    }

    /// Whether any prepared runs fall in stages `first..=last` for slots not in `bound_slots`.
    pub fn has_unbound_runs(&self, first: usize, last: usize, bound_slots: &[usize]) -> bool {
        self.runs
            .iter()
            .any(|run| first <= run.stage && run.stage <= last && !bound_slots.contains(&run.slot))
    }

    /// Append uploaded vertex chunks for prepared runs whose slot is bound to `PackPost` into
    /// `out`, reusing `out`'s existing allocation.
    pub fn pack_bound_chunks_into(
        &self,
        indices: &wgpu::Buffer,
        bound_slots: &[usize],
        out: &mut Vec<super::pack_post::ModDrawChunk>,
    ) {
        if self.runs.is_empty() || bound_slots.is_empty() {
            return;
        }
        for run in &self.runs {
            if !bound_slots.contains(&run.slot) {
                continue;
            }
            for (buffer, quads) in self.vertices.ranges(0, run.quads.clone()) {
                out.push(super::pack_post::ModDrawChunk {
                    slot: run.slot,
                    vertices: buffer.clone(),
                    indices: indices.clone(),
                    quads,
                });
            }
        }
    }

    /// Draws the prepared runs of stages `first..=last` in the order queued, with `indices`
    /// (6 a quad, `u32`), skipping slots in `skip_slots` (e.g. slots bound to a shader pack's
    /// translated mod pass). Leaves the pass with its own pipeline, bind group 0 and buffers.
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
    }

    fn draw_runs<'a>(
        &'a self,
        pass: &mut wgpu::RenderPass<'_>,
        indices: &wgpu::Buffer,
        pack: bool,
        runs: impl Iterator<Item = (&'a Run, std::ops::Range<u32>)>,
    ) {
        let pack_list = if pack {
            let Some((_, list)) = &self.pack_pipelines else {
                return;
            };
            Some(list)
        } else {
            None
        };
        let mut index_bound = false;
        for (run, range) in runs {
            let Some(Some(compiled)) = self.pipelines.get(run.slot) else {
                continue;
            };
            let pipeline = match pack_list {
                Some(list) => match list.get(run.slot) {
                    Some(Some(p)) => p,
                    _ => continue,
                },
                None => &compiled.pipeline,
            };
            if !index_bound {
                index_bound = true;
                pass.set_index_buffer(indices.slice(..), wgpu::IndexFormat::Uint32);
            }
            pass.set_pipeline(pipeline);
            pass.set_bind_group(
                0,
                if pack {
                    &compiled.pack_group
                } else {
                    &compiled.group
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
        self.world_primitives
            .as_mut()
            .expect("world primitives")
            .pipelines
            .build(&self.device, set, Some(textures))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gfx::world_primitives::{self, STAGES, WorldPrimitives};
    use glam::Vec3;
    use mod_api::{Identifier, RenderPipeline};

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
        (device, queue): &(wgpu::Device, wgpu::Queue),
        primitives: &mut WorldPrimitives,
        format: wgpu::TextureFormat,
        batch: Batch<'_>,
        last: usize,
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
            if drawn {
                primitives.draw_through(&mut pass, &mut 0, last);
            }
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
        let mut broken = set.built()[0].clone().unwrap();
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
        assert!(renderer.pipeline(&device, &broken, None).is_err());
        assert!(set.invalidate(0));
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
        assert!(!set.invalidate(0));
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
        let run = |slot, stage, quads| PipelineRun { slot, stage, quads };
        let runs = [run(0, 0, 2), run(1, 2, 1), run(3, 0, 1), run(4, 0, 1)];
        let batch = Batch {
            set: Some(&set),
            vertices: &vertices,
            runs: &runs,
            time: 1.0,
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
            },
            PipelineRun {
                slot: 1,
                stage: 0,
                quads: 1,
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
        assert!(primitives.pipelines.pipelines[0].as_ref().unwrap().textured);
        assert!(!primitives.pipelines.pipelines[3].as_ref().unwrap().textured);
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
            },
            PipelineRun {
                slot: 0,
                stage: (STAGES - 1) as u32,
                quads: 1,
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
}
