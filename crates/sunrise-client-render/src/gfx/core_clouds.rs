//! A resource pack's `core/clouds` shader (26.3's `CloudRenderer`, `RenderPipelines.CLOUDS` and
//! `FLAT_CLOUDS`): the clouds drawn by the pack's GLSL from the faces `CloudRenderer.buildMesh`
//! encodes (`CloudFaces`, an `R8_SINT` texel buffer, here a storage buffer of ints) and its
//! `CloudInfo` block, as a pack that shades or reshapes the clouds expects. Without one the
//! clouds draw natively (`clouds.rs`, `vs_cloud`).

use crate::core_shader::{CoreProgram, Glue};

/// `core/clouds`: no vertex input, the face and its corner from `gl_VertexIndex`.
pub(crate) const GLUE: Glue = Glue {
    inputs: &[],
    prelude: "",
    setup: "",
    // 26.3's ProjMat already has wgpu's reversed 0 to 1 depth.
    finish: "",
    vertex_id: "int(gl_VertexIndex)",
};

/// The pipeline's own block and texel buffer, beside the vanilla blocks.
pub(crate) const INFO_BLOCK: &str = "CloudInfo";
pub(crate) const FACES: &str = "CloudFaces";

/// Each vanilla block's offset in the one buffer (`core_shader::BLOCKS` order), then
/// `CloudInfo`'s.
const BLOCK_OFFSETS: [u64; 4] = [0, 256, 512, 768];
const BLOCK_SIZES: [u64; 4] = [48, 64, 160, 48];
const INFO_OFFSET: u64 = 1024;
const INFO_SIZE: u64 = 48;
/// The uniform bytes a frame writes: the vanilla blocks (`core_lines::uniforms`), `CloudInfo`.
const UNIFORM_BYTES: usize = (INFO_OFFSET + INFO_SIZE) as usize;
/// The faces buffer's first size, in ints (a thousand faces); it grows to what a frame needs.
const FIRST_CAPACITY: u64 = 3 * 1024;

/// Why a frame's faces don't fit: more bytes than the device binds to one storage buffer. The
/// frame's clouds then draw natively.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct TooManyFaces {
    pub bytes: u64,
    pub limit: u64,
}

/// The packs' cloud pipelines (with and without back faces culled) and their buffers.
pub(crate) struct CoreClouds {
    fancy: wgpu::RenderPipeline,
    flat: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    group: wgpu::BindGroup,
    uniform: wgpu::Buffer,
    faces: wgpu::Buffer,
    /// Ints the faces buffer holds.
    capacity: u64,
    /// Where the shader reads `CloudFaces`; none when it doesn't (its vertex stage may place
    /// the quads from `gl_VertexIndex` alone), and the faces then stay on the CPU, counted.
    faces_binding: Option<u32>,
    info_binding: Option<u32>,
    /// The faces uploaded last, in quads, and what they were built for: while it holds, they
    /// are not built again (`CloudRenderer.prepare`).
    quads: u32,
    key: Option<crate::clouds::pack::FacesKey>,
}

impl CoreClouds {
    /// The pipelines for `program`, drawing into the world's `format`; `None` when the pack's
    /// shader reads a block it is not given or does not compile for the backend. A shader that
    /// doesn't read `CloudFaces` is drawn too, as many quads as the faces.
    pub(super) fn new(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        program: &CoreProgram,
    ) -> Option<Self> {
        let binding = |name: &str| {
            program
                .other
                .iter()
                .position(|n| n == name)
                .map(crate::core_shader::other_binding)
        };
        let faces_binding = binding(FACES);
        let info_binding = binding(INFO_BLOCK);
        // A pack's shader is untrusted: the backend failing to compile it is an internal error.
        let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let internal = device.push_error_scope(wgpu::ErrorFilter::Internal);
        let module = |label, module: &naga::Module| {
            device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some(label),
                source: wgpu::ShaderSource::Naga(std::borrow::Cow::Owned(module.clone())),
            })
        };
        let vertex = module("Resource pack clouds (vertex)", &program.vertex);
        let fragment = module("Resource pack clouds (fragment)", &program.fragment);
        let block = |binding: u32, size: u64| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: wgpu::BufferSize::new(size),
            },
            count: None,
        };
        let mut entries: Vec<wgpu::BindGroupLayoutEntry> =
            (0..4).map(|b| block(b, BLOCK_SIZES[b as usize])).collect();
        entries.extend(info_binding.map(|b| block(b, INFO_SIZE)));
        entries.extend(faces_binding.map(|binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only: true },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        }));
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Resource pack clouds"),
            entries: &entries,
        });
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Resource pack clouds"),
            size: INFO_OFFSET + 256,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let faces = faces_buffer(device, FIRST_CAPACITY);
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Resource pack clouds"),
            bind_group_layouts: &[Some(&layout)],
            ..Default::default()
        });
        // RenderPipelines.CLOUDS: translucent, the default depth test and writes, back faces
        // culled; FLAT_CLOUDS without culling.
        let pipeline = |label, cull_mode| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &vertex,
                    entry_point: Some("main"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                primitive: wgpu::PrimitiveState {
                    cull_mode,
                    ..Default::default()
                },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: wgpu::TextureFormat::Depth32Float,
                    depth_write_enabled: Some(true),
                    depth_compare: Some(wgpu::CompareFunction::GreaterEqual),
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &fragment,
                    entry_point: Some("main"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let fancy = pipeline("Resource pack clouds", Some(wgpu::Face::Back));
        let flat = pipeline("Resource pack flat clouds", None);
        let group = bind(
            device,
            &layout,
            &uniform,
            &faces,
            faces_binding,
            info_binding,
        );
        let internal = pollster::block_on(internal.pop());
        if let Some(e) = internal.or(pollster::block_on(validation.pop())) {
            log::warn!("the resource pack's clouds shader is not used: {e}");
            return None;
        }
        Some(Self {
            fancy,
            flat,
            layout,
            group,
            uniform,
            faces,
            capacity: FIRST_CAPACITY,
            faces_binding,
            info_binding,
            quads: 0,
            key: None,
        })
    }

    /// The frame's blocks (`vanilla`, as `core_lines::uniforms` makes them, then `CloudInfo`)
    /// and the clouds' faces (`clouds::pack`), uploaded: the faces built and uploaded only when
    /// `CloudRenderer.prepare` would build them again. The quads to draw; or why the faces
    /// don't fit the device, which leaves the frame's clouds to the native mesh.
    pub(super) fn prepare(
        &mut self,
        device: &wgpu::Device,
        uploads: &mut super::upload::Uploads,
        encoder: &mut wgpu::CommandEncoder,
        vanilla: &[u8; INFO_OFFSET as usize],
        input: &crate::clouds::Input<'_>,
    ) -> Result<u32, TooManyFaces> {
        let key = crate::clouds::pack::faces_key(input);
        if self.key.as_ref() != Some(&key) {
            let faces = crate::clouds::pack::faces(input);
            let ints = faces.len() as u64;
            if self.faces_binding.is_some() && ints > self.capacity {
                // The faces' own bytes against what the device binds; the buffer grows to the
                // next power of two within that.
                let limit = device.limits().max_storage_buffer_binding_size;
                if ints * 4 > limit {
                    // The faces kept are still the last key's: the next frame tries again.
                    return Err(TooManyFaces {
                        bytes: ints * 4,
                        limit,
                    });
                }
                let capacity = ints.next_power_of_two().min(limit / 4);
                self.capacity = capacity;
                self.faces = faces_buffer(device, capacity);
                self.group = bind(
                    device,
                    &self.layout,
                    &self.uniform,
                    &self.faces,
                    self.faces_binding,
                    self.info_binding,
                );
            }
            if self.faces_binding.is_some() && !faces.is_empty() {
                uploads.buffer(encoder, &self.faces, 0, bytemuck::cast_slice(&faces));
            }
            self.quads = (faces.len() / 3) as u32;
            self.key = Some(key);
        }
        let mut data = [0u8; UNIFORM_BYTES];
        data[..INFO_OFFSET as usize].copy_from_slice(vanilla);
        data[INFO_OFFSET as usize..].copy_from_slice(&crate::clouds::pack::info(input));
        uploads.buffer(encoder, &self.uniform, 0, &data);
        Ok(self.quads)
    }

    /// The faces prepared, from `indices` (the renderer's quad indices, which hold them):
    /// `CLOUDS` for fancy clouds, `FLAT_CLOUDS` for fast ones. The caller sets its own pipeline
    /// and group 0 again after.
    pub(super) fn draw(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        fancy: bool,
        indices: &wgpu::Buffer,
    ) {
        if self.quads == 0 {
            return;
        }
        pass.set_pipeline(if fancy { &self.fancy } else { &self.flat });
        pass.set_bind_group(0, &self.group, &[]);
        pass.set_index_buffer(indices.slice(..), wgpu::IndexFormat::Uint32);
        pass.draw_indexed(0..self.quads * 6, 0, 0..1);
    }
}

/// The pipeline's group: the vanilla blocks and `CloudInfo` from `uniform`, the faces when the
/// shader reads them.
fn bind(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    uniform: &wgpu::Buffer,
    faces: &wgpu::Buffer,
    faces_binding: Option<u32>,
    info_binding: Option<u32>,
) -> wgpu::BindGroup {
    let range = |binding: u32, offset: u64, size: u64| wgpu::BindGroupEntry {
        binding,
        resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
            buffer: uniform,
            offset,
            size: wgpu::BufferSize::new(size),
        }),
    };
    let mut entries: Vec<wgpu::BindGroupEntry> = (0..4)
        .map(|i| range(i as u32, BLOCK_OFFSETS[i], BLOCK_SIZES[i]))
        .collect();
    entries.extend(info_binding.map(|b| range(b, INFO_OFFSET, INFO_SIZE)));
    entries.extend(faces_binding.map(|binding| wgpu::BindGroupEntry {
        binding,
        resource: faces.as_entire_binding(),
    }));
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("Resource pack clouds"),
        layout,
        entries: &entries,
    })
}

fn faces_buffer(device: &wgpu::Device, ints: u64) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("Resource pack cloud faces"),
        size: ints * 4,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

/// Whether a pack's clouds draw this frame in place of the native mesh: a pack's shader is
/// loaded, the clouds are on, and nothing else draws them its own way (improved transparency's
/// passes, a shader pack, a native mod's clouds, the title screen), nor needs the native mesh
/// (HDR's cloud marks follow it).
#[derive(Clone, Copy, Debug)]
pub(crate) struct CloudsFrame {
    pub pack_shader: bool,
    pub clouds: bool,
    pub native_clouds: bool,
    pub improved_transparency: bool,
    pub shader_pack: bool,
    pub title: bool,
    pub cloud_marks: bool,
}

impl CloudsFrame {
    pub(crate) fn pack_draws(self) -> bool {
        self.pack_shader
            && self.clouds
            && self.native_clouds
            && !self.improved_transparency
            && !self.shader_pack
            && !self.title
            && !self.cloud_marks
    }
}

/// The clouds on the renderer: a pack's `core/clouds`, or the native mesh.
impl super::Renderer {
    /// The resource packs' `core/clouds` (`HudAtlas::clouds_shader`), or the native clouds. As
    /// `set_lines_shader`: a pack's GLSL writes vanilla's gamma, so a frame of linear light keeps
    /// the native clouds.
    pub fn set_clouds_shader(&mut self, program: Option<&CoreProgram>) {
        let program = program.filter(|_| !self.output.frame.linear_light);
        self.core_clouds =
            program.and_then(|p| CoreClouds::new(&self.device, self.output.frame.format, p));
    }
}

#[cfg(test)]
#[path = "core_clouds_tests.rs"]
mod tests;
