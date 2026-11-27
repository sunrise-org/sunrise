//! `indirect.<pass>` (spec 3.2): a compute shader whose work-group counts are three `uint`s of
//! one of the pack's storage buffers, as earlier passes of the frame left them.
//!
//! The counts are not dispatched from the pack's buffer itself. A small pass of Sunrise's copies
//! them first into a buffer of its own, and that one is the dispatch's: the pack's buffer stays a
//! storage buffer only, so the shader that reads its counts may also write it, which wgpu refuses
//! for a buffer used as storage and as the indirect source of the same dispatch. The copy is also
//! the check wgpu's own indirect validation would be, so it holds whether that validation is on
//! or not: `gfx::device::instance_descriptor` turns it off in release builds unless the
//! backends include DirectX 12, and `WGPU_VALIDATION_INDIRECT_CALL=1` turns it back on. Counts
//! that end past the buffer, or with one over the device's
//! `max_compute_workgroups_per_dimension`, become zero groups, so nothing runs, as for a direct
//! dispatch over that limit (decision D-10).

use sunrise_shaders::Indirect;

use crate::pack_images::Buffers;

/// Copies the counts, checked, from the pack's buffer (binding 0) to `counts` (binding 1).
const GUARD: &str = "
struct Params { word: u32, most: u32 }
@group(0) @binding(0) var<storage, read> source: array<u32>;
@group(0) @binding(1) var<storage, read_write> counts: array<u32, 3>;
@group(0) @binding(2) var<uniform> params: Params;

@compute @workgroup_size(1)
fn main() {
    let words = arrayLength(&source);
    var c = vec3<u32>(0u);
    if params.word <= words && words - params.word >= 3u {
        c = vec3(source[params.word], source[params.word + 1u], source[params.word + 2u]);
        if any(c > vec3(params.most)) {
            c = vec3<u32>(0u);
        }
    }
    counts[0] = c.x;
    counts[1] = c.y;
    counts[2] = c.z;
}
";

/// What a compute pass read indirectly needs: the guard's pipeline and buffers, and its bind
/// group over the pack's buffer as it was last made.
#[derive(Clone)]
pub struct IndirectDispatch {
    binding: u32,
    layout: wgpu::BindGroupLayout,
    pipeline: wgpu::ComputePipeline,
    params: wgpu::Buffer,
    counts: wgpu::Buffer,
    /// The bind group over the pack's buffer as it is made now; dropped when that buffer is
    /// discarded or made again ([`Self::forget_group`]).
    group: Option<wgpu::BindGroup>,
}

impl IndirectDispatch {
    pub fn new(device: &wgpu::Device, name: &str, indirect: Indirect) -> Self {
        let storage = |binding, read_only| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some(name),
            entries: &[
                storage(0, true),
                storage(1, false),
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some(name),
            source: wgpu::ShaderSource::Wgsl(GUARD.into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some(name),
            bind_group_layouts: &[Some(&layout)],
            ..Default::default()
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some(name),
            layout: Some(&pipeline_layout),
            module: &module,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            cache: None,
        });
        let most = device.limits().max_compute_workgroups_per_dimension;
        let words = [indirect.offset / 4, most, 0, 0];
        let params = wgpu::util::DeviceExt::create_buffer_init(
            device,
            &wgpu::util::BufferInitDescriptor {
                label: Some(name),
                contents: bytemuck::cast_slice(&words),
                usage: wgpu::BufferUsages::UNIFORM,
            },
        );
        let counts = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(name),
            size: Indirect::BYTES,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::INDIRECT,
            mapped_at_creation: false,
        });
        Self {
            binding: indirect.buffer,
            layout,
            pipeline,
            params,
            counts,
            group: None,
        }
    }

    /// Drops the bind group, which keeps the pack's buffer it binds alive: that buffer was
    /// discarded ([`Buffers::reset_history`]) or made again.
    pub fn forget_group(&mut self) {
        self.group = None;
    }

    /// Records the copy of the counts and returns the buffer to dispatch from; `None` while the
    /// pack's buffer is not made.
    pub fn record(
        &mut self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        buffers: &Buffers,
    ) -> Option<wgpu::Buffer> {
        let source = buffers.get(self.binding)?;
        if self.group.is_none() {
            let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("Shader pack indirect counts"),
                layout: &self.layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: source.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: self.counts.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: self.params.as_entire_binding(),
                    },
                ],
            });
            self.group = Some(group);
        }
        let group = self.group.as_ref()?;
        let mut cp = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("Shader pack indirect counts"),
            timestamp_writes: None,
        });
        cp.set_pipeline(&self.pipeline);
        cp.set_bind_group(0, group, &[]);
        cp.dispatch_workgroups(1, 1, 1);
        drop(cp);
        Some(self.counts.clone())
    }
}
