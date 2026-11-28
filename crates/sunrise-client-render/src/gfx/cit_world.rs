//! OptiFine CIT enchantment glint layers over items and worn armour in the world
//! (`type=enchantment`; `crate::cit_glint` says which layers a stack draws, written from
//! OptiFine's `cit.properties` doc and CIT Resewn's `TypeEnchantment`). Each layer is a copy of
//! the item's quads tagged `CIT_GLINT_OVERLAY`, whose colour carries the layer's texture slot
//! (red) and strength (alpha) and whose `glint_uv` is the layer's texture coordinate, worked out
//! on the CPU as the GUI's are (`hud::cit_glint`). They are stored with the glint-only quads and
//! drawn after the entity pass with equal depth, over the surface they copy, by one pipeline per
//! blend (`fs_cit_glint` of `entity.wgsl`). The textures are the HUD's: one array of every
//! rule's texture, whose bind group is the entity pass's group 1.
use super::*;
use crate::hud::cit_glint::{GlintTextures, blend_state};
use std::ops::Range;

/// The entity pass's constant depth bias (`entity_pipeline`'s): a layer drawn with equal depth
/// must carry it too, to meet the surface it covers.
const ENTITY_BIAS: wgpu::DepthBiasState = wgpu::DepthBiasState {
    constant: 128,
    slope_scale: 0.0,
    clamp: 0.0,
};

/// The pipelines of the CIT glint layers drawn in the world.
pub(super) struct CitWorldPass {
    /// The textures' bind group: the HUD pass's, bound as group 1.
    group: wgpu::BindGroup,
    /// Per slot of `GlintTextures::slots`, the index of its blend's pipeline.
    slot_blend: Vec<u8>,
    /// One pipeline per entry of `GlintTextures::blends`, drawing into the frame.
    pipelines: Vec<wgpu::RenderPipeline>,
    /// The same in the overlays' format, for the first-person hand where that isn't the
    /// frame's.
    hand_pipelines: Option<Vec<wgpu::RenderPipeline>>,
}

impl CitWorldPass {
    /// The pipelines for the blends of `textures`, over the glint bindings `(layout, group)` the
    /// HUD's pass made of them, drawing into `frame` and, when the first-person hand has a
    /// format of its own, `hand` (each a format and whether it holds linear light).
    pub(super) fn new(
        device: &wgpu::Device,
        entity_bindings: &wgpu::BindGroupLayout,
        (cit_layout, group): (&wgpu::BindGroupLayout, &wgpu::BindGroup),
        (frame, hand): (
            (wgpu::TextureFormat, bool),
            Option<(wgpu::TextureFormat, bool)>,
        ),
        textures: &GlintTextures,
    ) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Entity (CIT glints)"),
            source: wgpu::ShaderSource::Wgsl(entity_shader().into()),
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("CIT glint (world) layout"),
            bind_group_layouts: &[Some(entity_bindings), Some(cit_layout)],
            ..Default::default()
        });
        let make = |target| pipelines(device, &layout, &shader, target, textures);
        Self {
            group: group.clone(),
            slot_blend: textures.slots.iter().map(|slot| slot.blend).collect(),
            pipelines: make(frame),
            hand_pipelines: hand.map(make),
        }
    }

    /// Binds the textures and the pipeline of `blend` for the layers' draws (`hand`: the
    /// first-person hand's, in the overlays' format); false, with nothing set, when there is
    /// none.
    fn bind(&self, pass: &mut wgpu::RenderPass<'_>, blend: u8, hand: bool) -> bool {
        let hand = self.hand_pipelines.as_ref().filter(|_| hand);
        let Some(pipeline) = hand.unwrap_or(&self.pipelines).get(usize::from(blend)) else {
            return false;
        };
        pass.set_pipeline(pipeline);
        pass.set_bind_group(1, &self.group, &[]);
        true
    }

    /// Draws the glint-only quads `quads` (whole quads, the first being quad `at` of a draw
    /// whose vertices start at `base`): vanilla's (the shield's) with `vanilla`, each run of
    /// layers with the pipeline of its blend, which `cit` holds (`hand`: the first-person
    /// hand's). Without it, no layer is drawn.
    pub(super) fn draw_glint_only(
        cit: Option<&Self>,
        pass: &mut wgpu::RenderPass<'_>,
        vanilla: &wgpu::RenderPipeline,
        quads: &[Vertex],
        (at, base): (u32, i32),
        hand: bool,
    ) {
        let blends = cit.map_or(&[][..], |cit| &cit.slot_blend[..]);
        for (blend, run) in glint_runs(quads.as_chunks::<4>().0, blends) {
            match (blend, cit) {
                (None, _) => pass.set_pipeline(vanilla),
                (Some(blend), Some(cit)) => {
                    if !cit.bind(pass, blend, hand) {
                        continue;
                    }
                }
                (Some(_), None) => continue,
            }
            pass.draw_indexed((at + run.start) * 6..(at + run.end) * 6, base, 0..1);
        }
    }
}

/// One pipeline per blend of `textures` over `layout`, drawing into `target` (a format and
/// whether it holds linear light).
fn pipelines(
    device: &wgpu::Device,
    layout: &wgpu::PipelineLayout,
    shader: &wgpu::ShaderModule,
    (format, linear): (wgpu::TextureFormat, bool),
    textures: &GlintTextures,
) -> Vec<wgpu::RenderPipeline> {
    let vertex = [Some(wgpu::VertexBufferLayout {
        array_stride: std::mem::size_of::<Vertex>() as u64,
        step_mode: wgpu::VertexStepMode::Vertex,
        attributes: &ENTITY_ATTRIBUTES,
    })];
    textures
        .blends
        .iter()
        .map(|&blend| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("CIT glint (world)"),
                layout: Some(layout),
                vertex: wgpu::VertexState {
                    module: shader,
                    entry_point: Some("vs_entity"),
                    compilation_options: Default::default(),
                    buffers: &vertex,
                },
                primitive: wgpu::PrimitiveState {
                    cull_mode: None,
                    ..Default::default()
                },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: wgpu::TextureFormat::Depth32Float,
                    depth_write_enabled: Some(false),
                    depth_compare: Some(wgpu::CompareFunction::Equal),
                    stencil: Default::default(),
                    bias: ENTITY_BIAS,
                }),
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: shader,
                    entry_point: Some("fs_cit_glint"),
                    compilation_options: wgpu::PipelineCompilationOptions {
                        constants: &linear_blend::frame_constants(format, linear),
                        ..Default::default()
                    },
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: Some(blend_state(blend)),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
        })
        .collect()
}

/// The blend of the slot a CIT glint quad's vertex carries in its colour's red byte; 0 for a
/// slot the table lacks.
fn slot_blend(blends: &[u8], vertex: &Vertex) -> u8 {
    blends
        .get(usize::from(vertex.color as u8))
        .copied()
        .unwrap_or(0)
}

/// Whether a quad of this `layer` (the overlay in its low 16 bits) is a CIT glint layer's.
fn is_layer(layer: u32) -> bool {
    layer & 0xffff == CIT_GLINT_OVERLAY
}

/// `quads` (the glint-only material, in order) as the runs one pipeline draws each: consecutive
/// quads of vanilla's glint (`None`) or of the layers of one blend (`Some(blend)`, a pipeline's
/// index, from the slot each quad's colour names in `blends`), with their quad ranges.
fn glint_runs<'a>(
    quads: &'a [[Vertex; 4]],
    blends: &'a [u8],
) -> impl Iterator<Item = (Option<u8>, Range<u32>)> + 'a {
    let kind =
        move |quad: &[Vertex; 4]| is_layer(quad[0].layer).then(|| slot_blend(blends, &quad[0]));
    let mut at = 0;
    quads
        .chunk_by(move |a, b| kind(a) == kind(b))
        .map(move |run| {
            let range = at..at + run.len() as u32;
            at = range.end;
            (kind(&run[0]), range)
        })
}

impl Renderer {
    /// Draws the CIT glint layers of `textures` (the pack's enchantment rules) in the world, as
    /// well as in the GUI; `None` or no textures draws none.
    pub fn set_cit_glints(&mut self, textures: Option<&GlintTextures>) {
        self.hud.set_cit_glints(&self.device, &self.queue, textures);
        let frame = self.output.frame;
        // The hand has pipelines of its own where its format isn't the frame's.
        let hand = (frame.overlay_format != frame.format || frame.linear_light)
            .then_some((frame.overlay_format, false));
        self.cit_world = textures.and_then(|textures| {
            let bindings = self.hud.cit_glint_bindings()?;
            Some(CitWorldPass::new(
                &self.device,
                &self.entity_bindings,
                bindings,
                ((frame.format, frame.linear_light), hand),
                textures,
            ))
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn quad(layer: u32, color: u32) -> [Vertex; 4] {
        [Vertex {
            pos: [0.0; 3],
            uv: [0.0; 2],
            layer,
            color,
            light: 0,
            normal: [0; 4],
            glint_uv: [0.0; 2],
        }; 4]
    }

    #[test]
    fn layers_draw_in_runs_of_one_blend_between_vanilla_glints() {
        let layer = |slot: u32| quad(CIT_GLINT_OVERLAY, slot | 0x8000_0000);
        let vanilla = quad(GLINT_ONLY_OVERLAY, u32::MAX);
        // Slots 0 to 2 blend as pipelines 0, 1 and 1.
        let blends = [0, 1, 1];
        let quads = [
            vanilla,
            layer(0),
            layer(1),
            layer(2),
            vanilla,
            vanilla,
            layer(0),
        ];
        let runs: Vec<_> = glint_runs(&quads, &blends).collect();
        assert_eq!(
            runs,
            [
                (None, 0..1),
                (Some(0), 1..2),
                (Some(1), 2..4),
                (None, 4..6),
                (Some(0), 6..7),
            ]
        );
    }

    #[test]
    fn a_layer_is_known_by_its_overlay_whatever_the_bits_above() {
        assert!(is_layer(CIT_GLINT_OVERLAY));
        assert!(is_layer(CIT_GLINT_OVERLAY | 1 << 16 | OIT_ENTITY));
        assert!(!is_layer(GLINT_ONLY_OVERLAY));
        assert!(!is_layer(0));
    }

    #[test]
    fn a_slot_the_table_lacks_blends_as_the_first() {
        let vertex = quad(CIT_GLINT_OVERLAY, 9)[0];
        assert_eq!(slot_blend(&[3, 4], &vertex), 0);
        assert_eq!(slot_blend(&[3, 4], &quad(CIT_GLINT_OVERLAY, 1)[0]), 4);
    }

    #[test]
    fn without_a_table_a_layer_is_still_told_from_vanillas_glint() {
        let quads = [quad(GLINT_ONLY_OVERLAY, 0), quad(CIT_GLINT_OVERLAY, 7)];
        let runs: Vec<_> = glint_runs(&quads, &[]).collect();
        assert_eq!(runs, [(None, 0..1), (Some(0), 1..2)]);
    }
}
