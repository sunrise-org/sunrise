//! The light texture (26.3's `Lightmap.render`, `RenderPipelines.LIGHTMAP`): 16x16, drawn by
//! `core/screenquad` and `core/lightmap` from the frame's `LightmapInfo` into an 8-bit target, as
//! vanilla draws it. The shaders are the game's own, or a resource pack's that replace them, as a
//! pack that darkens caves or tints the night expects (wiki "Lightmaps", 1.21.11 and up): the
//! bytes come from the GPU's own arithmetic and its unorm write, as vanilla's do.
//! `Renderer::update_lightmap` is the one entry point for the frame's light texture.

use crate::polytone_post::ExpressionUniform;
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};
use sunrise_glsl::{Glue, Translated};

/// `core/screenquad`: no vertex input, three vertices from `gl_VertexIndex`. GL's framebuffer
/// rows run from the bottom and wgpu's from the top: the flip keeps row `n` at sky level `n`.
pub(crate) const GLUE: Glue = Glue {
    inputs: &[],
    prelude: "",
    setup: "",
    finish: "gl_Position.y = -gl_Position.y;",
    vertex_id: "int(gl_VertexIndex)",
    fragment_finish: None,
    interface: sunrise_shaders::DEVICELESS_INTERFACE_LIMITS,
};

/// The block `Lightmap.render` fills, beside the vanilla ones.
pub(crate) const INFO_BLOCK: &str = "LightmapInfo";

/// The pipeline's shaders, whose packs' expression uniforms (`shader_modifiers`) it is given.
pub const SHADERS: [&str; 2] = ["minecraft:core/screenquad", "minecraft:core/lightmap"];

/// `LightmapInfo`'s offset in the uniform buffer, after vanilla's blocks
/// (`expression_blocks::VANILLA_OFFSETS`), and its size.
const INFO_OFFSET: u64 = 1024;
const INFO_SIZE: u64 = 96;
/// The bytes of vanilla's blocks and `LightmapInfo` in the uniform buffer.
const UNIFORM_BYTES: usize = (INFO_OFFSET + INFO_SIZE) as usize;

/// `LightmapRenderState` as `Lightmap.render` writes it (std140: six floats, four vec3s).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct LightmapInfo {
    pub sky_factor: f32,
    pub block_factor: f32,
    pub night_vision: f32,
    pub darkness: f32,
    pub boss_darkening: f32,
    pub brightness: f32,
    pub block_light_tint: [f32; 3],
    pub sky_light_color: [f32; 3],
    pub ambient_color: [f32; 3],
    pub night_vision_color: [f32; 3],
}

impl LightmapInfo {
    /// The block's bytes, as `lightmap.fsh` reads them.
    pub(crate) fn std140(&self) -> [u8; INFO_SIZE as usize] {
        let mut floats = [0f32; 24];
        floats[..6].copy_from_slice(&[
            self.sky_factor,
            self.block_factor,
            self.night_vision,
            self.darkness,
            self.boss_darkening,
            self.brightness,
        ]);
        // vec3s start on 16 bytes: floats 8, 12, 16 and 20.
        for (i, v) in [
            self.block_light_tint,
            self.sky_light_color,
            self.ambient_color,
            self.night_vision_color,
        ]
        .into_iter()
        .enumerate()
        {
            floats[8 + 4 * i..11 + 4 * i].copy_from_slice(&v);
        }
        let mut bytes = [0u8; INFO_SIZE as usize];
        bytes.copy_from_slice(bytemuck::cast_slice(&floats));
        bytes
    }
}

/// A readback buffer's state.
const IDLE: u8 = 0;
/// The frame's commands copy the texture into it; mapped once they are submitted.
const COPIED: u8 = 1;
const MAPPING: u8 = 2;
const MAPPED: u8 = 3;
/// A readback's row: copies need 256 bytes a row, and the texture has 64.
const ROW: u64 = 256;

/// The pack-drawn texture read back for F3's lightmap chart (`Renderer::lightmap_sample`), one
/// frame late, through two buffers in turn. It exists only while the chart is shown
/// ([`CoreLightmap::set_readback`]): its buffers are made when the chart opens and dropped when
/// it closes, so a chart opened again never shows a copy from before.
struct Readback {
    buffers: [(wgpu::Buffer, Arc<AtomicU8>); 2],
    /// The buffer this frame copies into, to be mapped once it is submitted.
    copied: Option<usize>,
    /// Each buffer's copy, numbered in frame order, so the newest one wins.
    serials: [u64; 2],
    next: u64,
    /// Each buffer's copy's epoch, and the current one: a copy from an earlier epoch (made
    /// before fullbright stopped the pack's shader) is dropped, never shown.
    epochs: [u64; 2],
    epoch: u64,
}

impl Readback {
    fn new(device: &wgpu::Device) -> Self {
        let buffer = || {
            let buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("Resource pack lightmap readback"),
                size: ROW * 16,
                usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            (buffer, Arc::new(AtomicU8::new(IDLE)))
        };
        Self {
            buffers: [buffer(), buffer()],
            copied: None,
            serials: [0; 2],
            next: 0,
            epochs: [0; 2],
            epoch: 0,
        }
    }
}

/// Whose lightmap shader a pipeline draws: vanilla's own `core/screenquad` and `core/lightmap`, the
/// default light texture, or a resource pack's, which overrides it (`hud::pack_text`'s
/// `load_lightmap` decides). Only a pack's is an override: what replaces the default texture
/// stands in for vanilla's alone.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShaderOrigin {
    Pack,
    Vanilla,
}

/// The packs' lightmap pipeline, its uniforms and, while F3 charts it, its readback.
///
/// It holds nothing of the level: a level change keeps it, and a pack change or reload makes a
/// new renderer, which loads the packs' shader again (`set_lightmap_shader`).
pub(crate) struct CoreLightmap {
    /// Whose shader it draws.
    pub(crate) origin: ShaderOrigin,
    pipeline: wgpu::RenderPipeline,
    group: wgpu::BindGroup,
    uniform: wgpu::Buffer,
    /// The packs' expression uniforms its stages read (`shader_modifiers`), resolved when it
    /// loaded, after `LightmapInfo`.
    pub(super) extra: super::expression_blocks::ExpressionBlocks,
    /// The bytes of vanilla's blocks and `LightmapInfo` a frame writes, kept between frames.
    bytes: [u8; UNIFORM_BYTES],
    readback: Option<Readback>,
}

impl CoreLightmap {
    /// The pipeline for `program`, drawing into an `Rgba8Unorm` texture, with `expressions`
    /// (resolved when it loaded: the expression blocks its stages read); or the device's refusal,
    /// named by vanilla's `LIGHTMAP`, when the pack's shader does not compile for the backend or
    /// a stage reads more blocks than the device binds to one (each caught by the error scope,
    /// whose message names the limit). Each block is bound only to the stages that read it: one
    /// only declared takes no binding.
    pub(super) fn new(
        device: &wgpu::Device,
        program: &Translated,
        expressions: &[ExpressionUniform],
        origin: ShaderOrigin,
    ) -> Result<Self, String> {
        let pipeline = crate::hud::pack_text::LIGHTMAP[0].pipeline;
        crate::core_shader::scoped(device, pipeline, || {
            Self::made(device, program, expressions, origin)
        })
    }

    fn made(
        device: &wgpu::Device,
        program: &Translated,
        expressions: &[ExpressionUniform],
        origin: ShaderOrigin,
    ) -> Self {
        let info_binding = program
            .other
            .iter()
            .position(|n| n == INFO_BLOCK)
            .map(|i| program.other_binding(i));
        let module = |label, module: &naga::Module| {
            device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some(label),
                source: wgpu::ShaderSource::Naga(std::borrow::Cow::Owned(module.clone())),
            })
        };
        let vertex = module("Resource pack lightmap (vertex)", &program.vertex);
        let fragment = module("Resource pack lightmap (fragment)", &program.fragment);
        let extra = super::expression_blocks::ExpressionBlocks::new(
            program,
            expressions,
            INFO_OFFSET + 256,
        );
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Resource pack lightmap"),
            size: extra.size(),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        // Every block a stage reads, at its offset and size, visible to the stages reading it:
        // vanilla's four and `LightmapInfo`, then the expression uniforms.
        let info = info_binding.map(|b| {
            let size = super::expression_blocks::block_size(program, b, INFO_SIZE);
            (b, INFO_OFFSET, size, program.stages_reading(b))
        });
        let mut blocks = super::expression_blocks::vanilla_blocks(program);
        blocks.extend(info.filter(|(.., stages)| !stages.is_empty()));
        let (entries, resources) = super::expression_blocks::bindings(&blocks, &extra, &uniform);
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Resource pack lightmap"),
            entries: &entries,
        });
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Resource pack lightmap"),
            layout: &layout,
            entries: &resources,
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Resource pack lightmap"),
            bind_group_layouts: &[Some(&layout)],
            ..Default::default()
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Resource pack lightmap"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &vertex,
                entry_point: Some("main"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &fragment,
                entry_point: Some("main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        Self {
            origin,
            pipeline,
            group,
            uniform,
            bytes: [0; UNIFORM_BYTES],
            extra,
            readback: None,
        }
    }

    /// Whether the drawn texture is read back (`on`: F3's lightmap chart is shown). Off, the
    /// two buffers are dropped and a frame copies nothing.
    pub(super) fn set_readback(&mut self, device: &wgpu::Device, on: bool) {
        if on != self.readback.is_some() {
            self.readback = on.then(|| Readback::new(device));
        }
    }

    /// The frame's blocks in the uniform buffer's layout, in the bytes the pipeline keeps for
    /// them (no allocation a frame): `globals` at 0 and `info` at its own. With the buffer they
    /// go to.
    fn uniforms(&mut self, globals: [u8; 48], info: &LightmapInfo) -> (&wgpu::Buffer, &[u8]) {
        let data = &mut self.bytes;
        data[..48].copy_from_slice(&globals);
        data[INFO_OFFSET as usize..].copy_from_slice(&info.std140());
        (&self.uniform, data)
    }

    /// Uploads the frame's blocks: [`Self::uniforms`], then each expression uniform's value in
    /// its own block, its 16 bytes only.
    fn upload(
        &mut self,
        uploads: &mut super::upload::Uploads,
        encoder: &mut wgpu::CommandEncoder,
        globals: [u8; 48],
        info: &LightmapInfo,
    ) {
        let (buffer, data) = self.uniforms(globals, info);
        uploads.buffer(encoder, buffer, 0, data);
        self.extra.upload(uploads, encoder, &self.uniform);
    }

    /// `Lightmap.render`'s pass: the screen quad over the whole of `texture` (through `view`,
    /// the renderer's own), then, while F3 charts it, its copy for the readback when a buffer
    /// is free.
    fn draw(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        texture: &wgpu::Texture,
        view: &wgpu::TextureView,
    ) {
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Resource pack lightmap"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::WHITE),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &self.group, &[]);
            pass.draw(0..3, 0..1);
        }
        let Some(readback) = self.readback.as_mut() else {
            return;
        };
        // A copy no submission mapped (a frame submitted another way): its buffer is free.
        if let Some(i) = readback.copied.take() {
            readback.buffers[i].1.store(IDLE, Ordering::Release);
        }
        let Some(i) = (0..2).find(|&i| readback.buffers[i].1.load(Ordering::Acquire) == IDLE)
        else {
            return;
        };
        encoder.copy_texture_to_buffer(
            texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &readback.buffers[i].0,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(ROW as u32),
                    rows_per_image: Some(16),
                },
            },
            texture.size(),
        );
        readback.buffers[i].1.store(COPIED, Ordering::Release);
        readback.copied = Some(i);
        readback.next += 1;
        readback.serials[i] = readback.next;
        readback.epochs[i] = readback.epoch;
    }

    /// The readback to start once the frame is submitted: mapping the buffer it copied into.
    pub(super) fn submitted(&mut self) -> Option<super::submit::AfterSubmit> {
        let readback = self.readback.as_mut()?;
        let i = readback.copied.take()?;
        let (buffer, state) = readback.buffers[i].clone();
        state.store(MAPPING, Ordering::Release);
        Some(Box::new(move || {
            let mapped = state.clone();
            buffer
                .slice(..)
                .map_async(wgpu::MapMode::Read, move |result| {
                    mapped.store(
                        if result.is_ok() { MAPPED } else { IDLE },
                        Ordering::Release,
                    );
                });
        }))
    }

    /// Copies made so far, mapped or still in flight, are of a texture no longer drawn (fullbright
    /// took over): they are dropped as they land, and only later copies are read.
    fn forget_copies(&mut self) {
        if let Some(readback) = self.readback.as_mut() {
            readback.epoch += 1;
        }
    }

    /// The newest texture read back, into `pixels`; whether one was.
    fn collect(&self, pixels: &mut [u8; 16 * 16 * 4]) -> bool {
        let Some(readback) = &self.readback else {
            return false;
        };
        let mut any = false;
        let mut order = [0, 1];
        order.sort_by_key(|&i| readback.serials[i]);
        for i in order {
            let (buffer, state) = &readback.buffers[i];
            if state.load(Ordering::Acquire) != MAPPED {
                continue;
            }
            let current = readback.epochs[i] == readback.epoch;
            if current && let Ok(range) = buffer.slice(..).get_mapped_range() {
                for row in 0..16 {
                    let at = row * ROW as usize;
                    pixels[row * 64..][..64].copy_from_slice(&range[at..at + 64]);
                }
                any = true;
            }
            buffer.unmap();
            state.store(IDLE, Ordering::Release);
        }
        any
    }
}

/// What the light texture holds: what the lightmap shader drew, or white (fullbright, or no
/// lightmap pipeline).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Drawn {
    White,
    Shader,
}

impl Drawn {
    /// What the frame's light texture holds: the shader's, when its pipeline exists, unless
    /// fullbright keeps the texture white.
    pub(crate) fn now(fullbright: bool, shader: bool) -> Self {
        if shader && !fullbright {
            Self::Shader
        } else {
            Self::White
        }
    }
}

/// The light texture's pixels F3's lightmap chart paints (RGBA rows of sky level by block level),
/// with what drew them. White is current at once; a drawn texture's arrive through the readback a
/// frame later, and only while the chart is open. A sample from another source than the one
/// drawing now is never shown: after the chart closes and opens again, or fullbright turns on or
/// off, it paints nothing until the texture being drawn comes back.
pub(crate) struct LightmapChart {
    pub(crate) pixels: [u8; 16 * 16 * 4],
    drawn: Option<Drawn>,
}

impl LightmapChart {
    pub(crate) fn new() -> Self {
        Self {
            pixels: [0; 16 * 16 * 4],
            drawn: None,
        }
    }

    /// The white texture, current at once.
    fn white(&mut self) {
        self.pixels = [255; 16 * 16 * 4];
        self.drawn = Some(Drawn::White);
    }

    /// A drawn texture read back into `pixels`.
    fn copied(&mut self) {
        self.drawn = Some(Drawn::Shader);
    }

    /// The readback closed: a drawn sample is no longer kept current.
    fn forget_drawn(&mut self) {
        if self.drawn == Some(Drawn::Shader) {
            self.drawn = None;
        }
    }

    /// What the chart paints while `drawing` draws the texture: its pixels, once they are
    /// that texture's.
    pub(crate) fn shown(&self, drawing: Drawn) -> Option<&[u8; 16 * 16 * 4]> {
        (self.drawn == Some(drawing)).then_some(&self.pixels)
    }
}

/// The lightmap pipeline for `program`, none without one (resources without the game's own pair,
/// as the boot screen's, `HudAtlas::boot`), or why the device can't make `LIGHTMAP`: a program past
/// its limits (`sunrise_glsl::PastDeviceLimit`) or its refusal. Either fails the pack reload, as a
/// program the driver doesn't link fails vanilla's (`ShaderManager.apply`): nothing else draws the
/// light texture.
pub(super) fn lightmap_for(
    device: &wgpu::Device,
    program: Option<&Translated>,
    expressions: &[ExpressionUniform],
    origin: ShaderOrigin,
) -> Result<Option<CoreLightmap>, String> {
    let pipeline = crate::hud::pack_text::LIGHTMAP[0].pipeline;
    let Some(program) = program else {
        return Ok(None);
    };
    let targets = [wgpu::TextureFormat::Rgba8Unorm];
    if let Some(past) = program.past_device_limit(&device.limits(), &targets) {
        return Err(format!("{pipeline}: {past}"));
    }
    CoreLightmap::new(device, program, expressions, origin).map(Some)
}

/// The renderer's light texture: the lightmap shader draws it.
impl super::Renderer {
    /// The lightmap shader (`HudAtlas::lightmap_shader`): the game's own, or the resource packs',
    /// as `origin` says (`HudAtlas::lightmap_origin`).
    /// The pipeline is built here: a program past the device's limits, or the backend's refusal
    /// of it, a pipeline vanilla requires, is returned and fails the pack reload. Unlike the sky's and the block outline's, the lightmap
    /// is kept on a frame of linear light (docs/hdr-passes.md): it draws the 8-bit light texture,
    /// never the frame, and the world's shaders read that texture the same way under either
    /// encoding (they decode their result at the end, `linear_blend`).
    pub fn set_lightmap_shader(
        &mut self,
        program: Option<&Translated>,
        expressions: &[ExpressionUniform],
        origin: ShaderOrigin,
    ) -> Result<(), String> {
        self.core_lightmap = lightmap_for(&self.device, program, expressions, origin)?;
        match self.core_lightmap.as_ref().map(|core| core.origin) {
            Some(ShaderOrigin::Vanilla) => {
                log::info!("the light texture is drawn by vanilla's lightmap shader")
            }
            Some(ShaderOrigin::Pack) => {
                log::info!("the light texture is drawn by a resource pack's lightmap shader")
            }
            None => {
                log::error!("no lightmap shader in these resources: the light texture stays white")
            }
        }
        self.lightmap_white = false;
        Ok(())
    }

    /// Whether F3's lightmap chart is shown (the game's `debug_shows_lightmap`): only then is the
    /// drawn light texture read back for it (`lightmap_sample`).
    pub fn set_lightmap_readback(&mut self, on: bool) {
        if let Some(core) = self.core_lightmap.as_mut() {
            core.set_readback(&self.device, on);
            if !on {
                self.lightmap_chart.forget_drawn();
            }
        }
    }

    /// The light texture's pixels for F3's lightmap chart, RGBA rows of sky level by block
    /// level: none until the texture drawn now has been read (`LightmapChart`).
    pub fn lightmap_sample(&self) -> Option<&[u8; 16 * 16 * 4]> {
        let drawing = Drawn::now(self.fullbright, self.core_lightmap.is_some());
        self.lightmap_chart.shown(drawing)
    }

    /// The frame's light texture (`LightmapRenderStateExtractor.extract`, then
    /// `Lightmap.render`): the lightmap shader draws it from the frame's `LightmapInfo`.
    /// Fullbright keeps it white, and so does a missing pipeline (logged when it was set).
    pub(super) fn update_lightmap(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        frame: &super::Frame,
    ) {
        let sky = &frame.sky;
        // What lightmap.fsh reads (NightVisionColor's default is #999999).
        let info = LightmapInfo {
            sky_factor: sky.sky_light_factor,
            block_factor: 1.4 + frame.block_flicker,
            night_vision: frame.night_vision,
            darkness: frame.darkness.1,
            boss_darkening: frame.boss_darken,
            brightness: (self.gamma - frame.darkness.0).max(0.0),
            block_light_tint: sky.block_light_tint,
            sky_light_color: sky.sky_light_color,
            ambient_color: sky.ambient_light_color,
            night_vision_color: [0.6; 3],
        };
        // The shader's globals: none are made while it doesn't draw.
        let globals =
            (!self.fullbright && self.core_lightmap.is_some()).then(|| self.pack_globals(frame));
        if let Some(globals) = globals
            && let Some(core) = self.core_lightmap.as_mut()
        {
            core.upload(&mut self.uploads, encoder, globals, &info);
            if core.collect(&mut self.lightmap_chart.pixels) {
                self.lightmap_chart.copied();
            }
            core.draw(encoder, &self.lightmap, &self.lightmap_view);
            // White goes up again when the shader stops drawing.
            self.lightmap_white = false;
            return;
        }
        // Fullbright stops the shader: what it copied before must not be read as current when
        // it draws again.
        if let Some(core) = self.core_lightmap.as_mut() {
            core.forget_copies();
        }
        if self.lightmap_white {
            return;
        }
        let pixels = [255u8; 16 * 16 * 4];
        self.uploads.texture(
            encoder,
            wgpu::TexelCopyTextureInfo {
                texture: &self.lightmap,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            16,
            16,
            &pixels,
        );
        self.lightmap_chart.white();
        self.lightmap_white = true;
    }
}

#[cfg(test)]
#[path = "core_lightmap_tests.rs"]
mod tests;
