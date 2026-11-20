//! GPU renderer: terrain, sky, block outline and HUD. Camera-relative section origins keep world coordinates precise.
use crate::{
    Result,
    block_entity_render::{StaticCacheFrameStats, StaticSectionRevision, StaticSectionSnapshot},
    environment::Sky,
    fast_hash::FxHashMap,
    graphics::GraphicsApi,
    hud::{HudBatch, HudRenderer},
    mesher::{Part as Quads, SectionMesh, Sides, TerrainVertex, Vertex, packed},
    section_mask::SectionMask,
    sprites::SpriteAnimation,
};
#[cfg(test)]
use glam::Vec4;
use glam::{DVec3, Mat4, Vec3};
use std::{collections::HashMap, path::PathBuf, sync::Arc, time::Instant};
use wgpu::util::DeviceExt;
use winit::window::Window;
// RenderPipelines.CRUMBLING: dst * src + src * dst, alpha replaced.
const CRUMBLING_BLEND: wgpu::BlendState = wgpu::BlendState {
    color: wgpu::BlendComponent {
        src_factor: wgpu::BlendFactor::Dst,
        dst_factor: wgpu::BlendFactor::Src,
        operation: wgpu::BlendOperation::Add,
    },
    alpha: wgpu::BlendComponent::REPLACE,
};
/// `TerrainVertex` for `vs_terrain`: position, UV, layer, light, colour.
const TERRAIN_ATTRIBUTES: [wgpu::VertexAttribute; 5] =
    wgpu::vertex_attr_array![0=>Float32x3,1=>Uint16x2,2=>Uint16,4=>Uint8x2,3=>Unorm8x4];

fn no_pack_hdr_gain(layered: bool, pack: bool, world: bool) -> bool {
    layered && !pack && world
}

fn light_marks(format: wgpu::TextureFormat, hdr: bool) -> bool {
    hdr && format != LINEAR_FRAME
}

fn terrain_shader() -> String {
    include_str!("hdr_light.wgsl").to_owned() + include_str!("terrain.wgsl")
}

pub(super) use border::border_shader;

pub mod border;
#[cfg(test)]
mod crumbling_tests;
mod gpu_checks;
mod section_gpu;
mod static_entity;

pub(crate) use section_gpu::collect_animated_layers;
use section_gpu::{
    Part, SectionGpu, begin_shrink, decide_fade, forget_records, hold_fading, mark_fades,
};
use static_entity::{GpuStaticEntitySection, StaticEntityUniform};
pub mod outline;
mod overlay;
pub use border::BorderDraw;
use border::BorderUniform;
use gpu_checks::GpuChecks;
use outline::{debug_line_opaque, debug_lines_opaque_first, outline_vertices_into};
pub use outline::{outline_params, outline_vertices, outline_width};
use overlay::Overlay;
mod entity_batch;
mod entity_outline;
#[cfg(any(test, feature = "test-support"))]
#[doc(hidden)]
pub mod entity_pixel_test;
mod fluid;
mod prepared_model;
#[cfg(feature = "test-support")]
pub use prepared_model::fail_next_prepared_upload;
pub use prepared_model::{
    PreparedInstanceBuilder, PreparedModelDraw, PreparedModelInstance, PreparedModelVertex,
    PreparedModels, camera_matrix,
};
#[cfg(feature = "test-support")]
pub use prepared_model::{
    cap_prepared_instance_slots, capped_prepared_room, set_test_prepared_generation,
    test_prepared_generation,
};
mod block_entity_positions;
mod culling_control;
mod draw_frame;
mod entity_draw;
mod hdr_config;
#[cfg(test)]
mod inter_stage_tests;
mod renderer_construction;
mod section_fade;
mod section_upload;
mod shader_pack;
mod surface_lifecycle;
use entity_draw::*;
#[cfg(test)]
mod sprite_arrays_tests;
mod terrain_model;
#[cfg(any(test, feature = "test-support"))]
pub(crate) mod test_gpu;
mod weather;
mod world_text;
use cull::occlusion_culling_requested;
use entity_batch::EntityBatch;
pub use entity_batch::{
    ADDITIVE_PARTICLE_OVERLAY, ARMOR_GLINT_ONLY_OVERLAY, DISSOLVE_OVERLAY, DRAGON_RAYS_OVERLAY,
    ENERGY_SWIRL_OVERLAY, FLUID_OVERLAY, OIT_ENTITY, SHADOW_OVERLAY,
    TERRAIN_MODEL_CUTOUT_MIPPED_OVERLAY, TERRAIN_MODEL_CUTOUT_OVERLAY, TERRAIN_MODEL_SOLID_OVERLAY,
    TERRAIN_MODEL_TRANSLUCENT_OVERLAY, TEXT_POLYGON_OFFSET_OVERLAY, TRANSLUCENT_GLINT_ITEM_OVERLAY,
    TRANSLUCENT_ITEM_OVERLAY, TRANSLUCENT_PARTICLE_OVERLAY, WATER_MASK_OVERLAY, WORLD_TEXT_OVERLAY,
    WORLD_TEXT_POLYGON_OFFSET_OVERLAY, is_native_terrain, mark_oit,
};
use external::{shadow_box, shadow_runs};
pub use frustum::{Cull, Frusta, JoinedFrusta, ShadowCull};
use frustum::{block_entities_within_reach, frustum, shadow_casters, taa_frustum};
pub use section_fade::{CHUNK_VISIBILITY_THRESHOLD, chunk_visibility, fade_millis};
use section_fade::{FadeInputs, compiled_and_visible, section_instance, section_visibility};
pub use sunrise_render_core::SectionKey;
pub use view::{View, ViewSet};

/// Whether Translucent Batching is offered (`graphics::offer_translucent_batching`): where
/// indirect multi-draws (`multi_draw`) take its batches, but not OpenGL's, which loop over
/// their entries on the CPU (and without them every section is a call of its own anyway).
/// Frames drawn flat (Metal, Intel Gen11/Gen12's Vulkan driver) take it too: their
/// translucent calls then come one per page of its arena rather than about one per section.
fn translucent_batchable(info: &wgpu::AdapterInfo, multi_draw: bool) -> bool {
    multi_draw && info.backend != wgpu::Backend::Gl
}

/// The vertices flat draws bind for a call's page key: a page of the section arena, or of
/// the translucent arena up to its last range in use (`flat::TRANSLUCENT_KEYS`).
fn flat_vertices<'a>(
    arena: &'a arena::Arena,
    translucent: Option<&'a arena::Arena>,
) -> impl Fn(u32) -> wgpu::BufferSlice<'a> + Copy {
    move |key| match (translucent, key.checked_sub(flat::TRANSLUCENT_KEYS)) {
        (Some(translucent), Some(page)) => translucent.binding(page),
        _ => arena.buffer(key).slice(..),
    }
}

/// Bytes of `texture` with every mip level of every layer.
pub(crate) fn texture_bytes(texture: &wgpu::Texture) -> u64 {
    let size = texture.size();
    let block = u64::from(texture.format().block_copy_size(None).unwrap_or(4));
    (0..texture.mip_level_count())
        .map(|level| {
            let side = |v: u32| u64::from((v >> level).max(1));
            side(size.width) * side(size.height) * block
        })
        .sum::<u64>()
        * u64::from(size.depth_or_array_layers)
}

/// Whether translucent quads have an arena of their own: with Translucent Batching
/// (`batching`), unless a shader pack's frames can draw terrain flat (as for `flat_frame`),
/// which reads them in the section arena through its tables: where flat draws exist and
/// are on (`flat_draws`), for a `shaders/` pack's terrain programs (`terrain`) or a shadow
/// pass (`shadow`: a `shaders/` pack's, or the `[shadow]` of one of Sunrise's own).
fn translucent_split(batching: bool, flat_draws: bool, terrain: bool, shadow: bool) -> bool {
    batching && !(flat_draws && (terrain || shadow))
}

/// The compact layout of solid quads, unless `SUNRISE_PACKED_QUADS=0` (A/B checks).
fn packed_wanted() -> bool {
    std::env::var("SUNRISE_PACKED_QUADS").as_deref() != Ok("0")
}

pub(crate) mod animate;
mod arena;
pub mod capture;
mod celestial;
mod census;
pub mod compositor;
pub(crate) mod core_clouds;
pub(crate) mod core_lightmap;
pub use core_lightmap::SHADERS as LIGHTMAP_SHADERS;
pub(crate) mod core_lines;
mod core_particles;
pub use core_particles::HEIGHTMAP_SIZE as POLYTONE_HEIGHTMAP_SIZE;
pub(crate) mod core_sky;
pub(crate) mod core_text;
mod cull;
pub(crate) mod display_watch;
mod dx12_compiler;
mod end_sky;
pub mod entity_skin;
pub mod expression_blocks;
pub mod external;
mod flat;
mod frame_gen;
mod frustum;
mod gpu_culling;
mod hdr;
mod hdr_icc;
mod hdr_start;
mod hdr_stats;
pub(crate) mod hud_coverage;
mod in_flight;
use display_watch::DisplayWatch;
#[cfg(test)]
use display_watch::{macos_hdr, os_reported};
pub use hdr::{Calibration as HdrCalibration, Output as HdrOutput};
pub use output::{DisplayReading, EffectiveOutput, FrameConfiguration, FrameRequest};

pub mod block_light;
mod custom_sky;
mod displace;
mod frame_policy;
pub mod lightning;
mod linear_blend;
#[cfg(test)]
use frame_policy::frame_format;
use frame_policy::{frame_configuration, overlay_format, sunrise_tone_curve};
mod lod;
pub mod mod_pipelines;
mod native_material;
mod native_pack;
mod native_post;
use crate::render_slots as native_render;
mod native_shadow;
mod native_sky;
mod occlusion;
mod oit;
mod oit_features;
pub(crate) mod output;
mod output_apply;
mod pack_atlas;
mod pack_center_depth;
pub mod pack_draw;
mod pack_entities;
mod pack_hand;
mod pack_images;
mod pack_lines;
mod pack_post;
mod pack_shadow;
mod pack_sky;
mod pack_textures;
pub mod pack_timing;
pub mod pack_uniforms;
mod pack_weather;
mod pass_timing_api;
use pass_timing_api::{after_culling_pass, pixel_count, world_pass_after_culling};
mod page_runs;
pub mod panorama;
mod readback;
mod resident;
mod reuse;
mod screenshot;
pub use screenshot::{PendingScreenshot, ScreenshotSave};
mod screenshot_readback;
mod stars;
mod surface_setup;
use surface_setup::{AdapterChoice, AdapterFailure, open_adapter, pick_adapter, set_up_surface};
mod off_page;
use off_page::move_off_page;
pub mod submit;
mod sunrise_fan;
mod timing;
pub mod translucency;
pub(crate) mod upload;
mod view;
mod view_sections;
mod visbuffer;
mod wgsl_prefix;
mod world_handover;
mod world_post;
pub mod world_primitives;
mod world_views;

/// A section's instance: its camera-relative origin and, in `w`, the step of
/// its fixed-point UVs (`terrain.wgsl`'s `terrain_uv`).
/// The world pass again after occlusion culling's test: it loads both
/// targets (phase 2 draws over phase 1, reading the test's results).
/// `LevelRenderer.addSkyPass`: fluids and camera effects block the sky pass.
fn sky_open(frame: &Frame) -> bool {
    !frame.sky_blocked
}

/// The Overworld-style sky (disc, sunset, stars, void) draws this frame.
fn sky_visible(frame: &Frame) -> bool {
    !frame.title && frame.skybox && sky_open(frame)
}

/// The End during the dragon fight, whose world fog leaves out vanilla's sky: a shader pack's
/// `gbuffers_skybasic` still draws a fan around the horizon (`pack_sky`), as our probe packs read
/// it in Iris 1.11.7.
fn end_fog_sky(frame: &Frame) -> bool {
    !frame.title && frame.end_sky && frame.boss_world_fog
}

/// The point a `shaders/` pack's terrain is drawn relative to: the camera with the fraction of
/// each coordinate rounded to the nearest 1/65536 block. Read back with a probe pack of ours,
/// Iris 1.11.7's terrain `gl_Vertex` is the block's position minus this, for cameras from 13
/// to 40000 blocks out (#8464); `cameraPosition` and the matrices keep the exact camera. Off by
/// up to 2^-17 blocks, a fragment on a texel boundary of the shadow map reads the other texel.
fn pack_terrain_camera(camera: DVec3) -> DVec3 {
    let whole = camera.floor();
    whole + ((camera - whole) * 65536.0).round() / 65536.0
}
/// A new value for `Renderer::block_entity_revision`.
fn next_block_entity_revision() -> u64 {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

const MAX_OUTLINE_EDGES: u64 = 256;
/// Staging buffer size for section meshes: a frame of uploads while
/// exploring fits in a few.
const SECTION_UPLOAD_CHUNK: u64 = 4 << 20;
/// Section copies are submitted early past this many staged bytes, which
/// bounds the staging memory frames that are not drawn can hold.
const SECTION_COPY_LIMIT: u64 = 64 << 20;
/// What compaction copies in a frame, `compact_arena` first and
/// `shrink_translucent` with what it left, and a range more where a call's
/// first is larger (`move_off_page`): a page empties over a few frames, with
/// no hitch.
const COMPACT_BUDGET: u64 = 32 << 20;
/// A large translucent page starts to shrink (`Renderer::shrink_translucent`)
/// this long after terrain last arrived, and `SHRINK_RETRY` after a try found
/// no room.
/// `Renderer::remove_columns` filters each store once rather than looking up every
/// section key from this many keys, or from as many as there are resident sections.
const BULK_REMOVAL_KEYS: usize = 256;
const SHRINK_SETTLE: std::time::Duration = std::time::Duration::from_secs(1);
const SHRINK_RETRY: std::time::Duration = std::time::Duration::from_secs(5);
pub struct Renderer {
    instance: wgpu::Instance,
    adapter: wgpu::Adapter,
    /// The window's swapchain; `None` when rendering offscreen (`--offscreen`). Shared
    /// with frame generation's present thread while it runs.
    surface: Option<Arc<wgpu::Surface<'static>>>,
    /// The surface `release_surface` set aside for the renderer that replaces
    /// this one on the same device: reused, not created again, as a new
    /// surface for the window can come back lost (Wayland on Mesa).
    spare_surface: std::sync::Mutex<Option<wgpu::Surface<'static>>>,
    /// The offscreen colour target, sized like `config`.
    offscreen: Option<wgpu::Texture>,
    /// Offscreen submissions still on the GPU: at most two frames run ahead.
    in_flight: std::collections::VecDeque<wgpu::SubmissionIndex>,
    /// Which steps of the first frame on this device still go to the log
    /// (see `startup_step`).
    first_frame: FirstFrame,
    gpu_timer: Option<timing::GpuTimer>,
    /// GPU frame times (ms) read back since the caller last drained them.
    pub gpu_times: Vec<f32>,
    /// The light texture's pixels F3+4 shows, with what drew them (`lightmap_sample`).
    lightmap_chart: core_lightmap::LightmapChart,
    /// Time the last `render` spent blocked on the GPU or the display
    /// (acquiring, presenting, offscreen pacing, the submit thread holding the queue),
    /// excluded from CPU time.
    pub last_wait: std::time::Duration,
    /// CPU time of the last `render`, waits excluded.
    pub last_encode: std::time::Duration,
    /// Watched only while HDR is On or Auto, including Auto in SDR;
    /// before `window`, so it's dropped first.
    display: Option<DisplayWatch>,
    /// What this renderer's display last reported (`DisplayWatch`), its own: another
    /// renderer's watch never changes it.
    reading: output::DisplayReading,
    /// The watch was just made: the settings' Auto values take what it reported.
    display_fresh: bool,
    window: Arc<Window>,
    pub device: wgpu::Device,
    /// The shader modules, layouts and pipelines kept for the next renderer on
    /// this device ([`reuse`]).
    reuse: Arc<reuse::Reuse>,
    /// Frames are finished and submitted on its submit thread ([`submit`]).
    pub queue: submit::Queue,
    /// The last frame went to the submit thread; whether it had no window surface.
    /// The next frame takes it in ([`Self::land_frame`]).
    deferred: Option<bool>,
    /// Set by the device's lost callback. wgpu reports nothing else when an
    /// operation fails on a lost device (no error scope sees it).
    device_lost: Arc<std::sync::atomic::AtomicBool>,
    pub config: wgpu::SurfaceConfiguration,
    solid: SolidPipeline,
    /// `solid` split in two passes for the world pass (terrain.wgsl's `opaque_quad`).
    solid_opaque: SolidPipeline,
    solid_cutout: SolidPipeline,
    translucent: wgpu::RenderPipeline,
    translucent_mark: wgpu::RenderPipeline,
    /// Flat section draws (`flat`), where multi-draw is emulated.
    flat: Option<flat::Flat>,
    /// Sections upload their solid quads in the compact layout (`mesher::packed`): the solid
    /// passes read the arena themselves (`flat`) and no shader pack reads it as vertices.
    packed_quads: bool,
    /// The terrain's acceleration structures, with ray tracing on (`ray_scene`).
    ray_scene: Option<surface_setup::ray_query::ray_scene::RayScene>,
    /// The compact layout is allowed (`SUNRISE_PACKED_QUADS`, `set_packed_layout`).
    packed_on: bool,
    /// The visibility buffer for flat frames' solid terrain (`visbuffer`,
    /// `SUNRISE_VISBUFFER=1`).
    visbuf: Option<visbuffer::Visbuffer>,
    /// `flat` draws this frame's sections (off: the per-section draws).
    flat_on: bool,
    /// Occlusion culling of solid section draws, where the GPU's instance
    /// counts reach a multi-draw's entries (Vulkan, Metal).
    cull: Option<cull::Cull>,
    cull_on: bool,
    /// Solid quads facing away from the eye are left out (`mesher::Sides`).
    sides_on: bool,
    /// The near solid sections culled and drawn on the GPU (`gpu_culling`): its pass,
    /// records and fades while it is on, where it is offered (`gpu_culling_offer`, mirrored
    /// for the settings in `graphics::gpu_culling_offer`), and whether the last frame drew
    /// them so (`gpu_frame`).
    gpu_culling: Option<gpu_culling::GpuCulling>,
    gpu_culling_offer: crate::graphics::GpuCullingOffer,
    gpu_frame: bool,
    /// GPU frames since GPU culling was switched on: the first and every 64th after it
    /// check its records (`check_gpu_records`).
    #[cfg(debug_assertions)]
    gpu_checks: u32,
    sky: wgpu::RenderPipeline,
    /// The main sky without the sunrise fan, which `sunrise_pipeline` draws as
    /// geometry over it. The mirror keeps `sky`: its per-pixel fan is cheap at
    /// the reflection's size and needs no second projection.
    sky_base: wgpu::RenderPipeline,
    sunrise_pipeline: wgpu::RenderPipeline,
    sunrise_buffer: wgpu::Buffer,
    dark_disc: wgpu::RenderPipeline,
    /// Clears the world pass's depth to the far plane so the first-person hand draws in the
    /// same pass (`hand_in_world`), as the hand's own pass would clear it.
    depth_reset: wgpu::RenderPipeline,
    lines: wgpu::RenderPipeline,
    lines_secondary: wgpu::RenderPipeline,
    /// A resource pack's `core/rendertype_lines`, which draws the block outline in their place.
    core_lines: Option<core_lines::CoreLines>,
    core_text: Option<core_text::CoreText>,
    /// The packs' `core/sky` and `core/stars`, drawn in place of the native sky and stars.
    core_sky: Option<core_sky::CoreSky>,
    /// A resource pack's `core/lightmap`, which draws the light texture in place of the native
    /// one.
    core_lightmap: Option<core_lightmap::CoreLightmap>,
    /// A resource pack's `core/clouds`, which draws the clouds in place of the native ones.
    core_clouds: Option<core_clouds::CoreClouds>,
    /// Polytone GPU particle types with their own shader, and this frame's batches of them.
    core_particles: core_particles::CoreParticles,
    /// Gizmo lines: opaque ones (`RenderTypes.lines`, which writes depth),
    /// translucent ones (`RenderTypes.linesTranslucentNoDepthWrite`) and the
    /// always-on-top ones.
    debug_lines_opaque: wgpu::RenderPipeline,
    debug_lines: wgpu::RenderPipeline,
    debug_lines_on_top: wgpu::RenderPipeline,
    debug_crosshair: Option<wgpu::RenderPipeline>,
    /// The debug crosshair and the first-person hand in `overlay_format`, where it isn't the
    /// frame's.
    overlay_crosshair: Option<wgpu::RenderPipeline>,
    overlay_hand: Option<wgpu::RenderPipeline>,
    debug_line_buffer: wgpu::Buffer,
    debug_line_capacity: u64,
    /// The terrain resources with the debug crosshair's projection
    /// (`crosshair_globals`).
    crosshair_group: wgpu::BindGroup,
    crosshair_globals: wgpu::Buffer,
    /// The water mirror: its solid terrain pipeline, its camera's globals and their group,
    /// and its targets once drawn.
    solid_mirror: wgpu::RenderPipeline,
    mirror_globals: wgpu::Buffer,
    mirror_group: wgpu::BindGroup,
    mirror: Option<MirrorTarget>,
    /// Native mods' world views' targets and pipelines, and the largest side the device gives one.
    world_views: world_views::Kept,
    world_view_side: u32,
    entity_pipeline: wgpu::RenderPipeline,
    static_entity_pipeline: wgpu::RenderPipeline,
    static_entity_group: wgpu::BindGroup,
    static_entity_transform: wgpu::Buffer,
    static_entity_camera: Option<DVec3>,
    static_entity_catalog_revision: u64,
    static_entity_frame_stats: StaticCacheFrameStats,
    hdr_mark_pipeline: wgpu::RenderPipeline,
    /// `hdr_mark_pipeline` at `entity_pipeline`'s depth bias, for what that drew.
    hdr_entity_mark_pipeline: wgpu::RenderPipeline,
    hdr_hand_unmark_pipeline: wgpu::RenderPipeline,
    hdr_mark_lightning_pipeline: wgpu::RenderPipeline,
    weather: weather::Weather,
    crumbling_pipeline: wgpu::RenderPipeline,
    shadow_blob_pipeline: wgpu::RenderPipeline,
    translucent_item_pipeline: wgpu::RenderPipeline,
    /// `RenderPipelines.WATER_MASK`: depth writes only, no colour.
    water_mask_pipeline: wgpu::RenderPipeline,
    cloud_pipeline: wgpu::RenderPipeline,
    hdr_cloud_mark_pipeline: wgpu::RenderPipeline,
    cloud_mesh: crate::clouds::Mesh,
    cloud_buffer: wgpu::Buffer,
    cloud_capacity: u64,
    cloud_uniform: wgpu::Buffer,
    cloud_group: wgpu::BindGroup,
    additive_pipeline: wgpu::RenderPipeline,
    dragon_rays_pipeline: wgpu::RenderPipeline,
    /// `RenderPipelines.ENERGY_SWIRL`: additive, unlit, the sprite repeated within itself.
    energy_swirl_pipeline: wgpu::RenderPipeline,
    lightning_pipeline: wgpu::RenderPipeline,
    lightning: lightning::Renderer,
    see_through_pipeline: wgpu::RenderPipeline,
    /// Native mods' world primitives (`Frame::world_primitives`).
    world_primitives: world_primitives::WorldPrimitives,
    text_polygon_offset_pipeline: wgpu::RenderPipeline,
    world_text_pipeline: wgpu::RenderPipeline,
    fluid_pipeline: wgpu::RenderPipeline,
    terrain_model_pipelines: [wgpu::RenderPipeline; 4],
    celestial: celestial::Celestial,
    celestial_buffer: wgpu::Buffer,
    end_sky: end_sky::EndSky,
    stars: stars::Stars,
    /// Custom sky layers (`crate::custom_sky`), and the frame's from the game.
    custom_sky: custom_sky::CustomSky,
    custom_sky_draws: Vec<crate::custom_sky::Draw>,
    /// The glowing entities' outline (`LevelRenderer.entityOutlineTarget`).
    entity_outline: entity_outline::EntityOutline,
    panorama: panorama::Panorama,
    /// `RenderPipelines.WORLD_BORDER` and its buffers.
    border_pipeline: wgpu::RenderPipeline,
    border_group: wgpu::BindGroup,
    border_uniform: wgpu::Buffer,
    border_vertices: wgpu::Buffer,
    /// Screen effects: alpha-blended, then vignette (`dst * (1 - src)`)
    /// pipelines, each for every sampling (index `blend * 4 + sampling`).
    screen_pipelines: [wgpu::RenderPipeline; 16],
    screen_buffer: wgpu::Buffer,
    screen_capacity: u64,
    entity_group: wgpu::BindGroup,
    /// The layout of `entity_group`, for the native shadow pass.
    entity_bindings: wgpu::BindGroupLayout,
    /// Generation-owned prepared model geometry. World block entities instance it
    /// when the draw is not kept, spinning, or claimed by a shader pack.
    prepared_models: Option<PreparedModels>,
    prepared_draws: Vec<PreparedModelDraw>,
    /// Ordinary-stream cuts for `prepared_draws`, filled once per frame.
    prepared_cuts: Vec<u32>,
    prepared_ranges: Vec<prepared_model::OrderedSpan>,
    prepared_uploaded: Vec<PreparedModelInstance>,
    prepared_instance_buffer: wgpu::Buffer,
    prepared_instance_capacity: u64,
    /// The entity resources with the hand's projection (`hand_globals`).
    hand_group: wgpu::BindGroup,
    hand_globals: wgpu::Buffer,
    entity_buffer: wgpu::Buffer,
    entity_capacity: u64,
    static_entity_sections: HashMap<[i32; 3], GpuStaticEntitySection>,
    static_entity_indices: wgpu::Buffer,
    static_entity_index_quads: u32,
    entity_batch: EntityBatch,
    /// Ordinary entity models posed on the GPU; `None` without compute shaders.
    entity_skin: Option<entity_skin::EntitySkin>,
    group: wgpu::BindGroup,
    fluid_group: wgpu::BindGroup,
    fluid_sampler: wgpu::Sampler,
    terrain_model_group: wgpu::BindGroup,
    terrain_model_sampler: wgpu::Sampler,
    globals: wgpu::Buffer,
    /// Resource-pack normal/specular maps, split like the colour sprite arrays.
    pbr_maps: Option<Vec<[wgpu::Texture; 2]>>,
    lightmap: wgpu::Texture,
    /// The block sprites, `sprite_split` layers per array (the first is
    /// `sprites` in terrain.wgsl, the others `sprites_1`...).
    sprite_textures: Vec<wgpu::Texture>,
    /// Block sprite layers per array on this device ([`layer_split`]).
    sprite_split: u32,
    /// A 1-layer array bound where a pack needs fewer arrays.
    sprite_unused: wgpu::TextureView,
    /// The sprite arrays as the terrain samples them (`SPRITE_ARRAYS` views, `sprite_unused`
    /// past the ones in use, the Mipmap Levels option's levels), for native mods
    /// (`WorldDrawn::block_sprites`).
    sprite_views: Vec<wgpu::TextureView>,
    /// Nearest, repeating, nearest mip level: for drawing a sprite's texels sharp elsewhere.
    sprite_nearest: wgpu::Sampler,
    mip_levels: u32,
    /// The mipmap levels sampled (the Mipmap Levels option), at most `mip_levels - 1`.
    mipmaps: u32,
    terrain_layout: wgpu::BindGroupLayout,
    /// terrain.wgsl's `SPRITE_ARRAYS` and `SPRITE_SPLIT`, for a shader pack's terrain.
    terrain_sprites: [f64; 2],
    sprite_sampler: wgpu::Sampler,
    /// `Options.textureFiltering`: 0 none, 1 RGSS, 2 anisotropic.
    texture_filtering: u32,
    /// The anisotropic sampler's clamp (`Options.maxAnisotropyValue`); 1 unless anisotropic.
    max_anisotropy: u16,
    lightmap_view: wgpu::TextureView,
    lightmap_sampler: wgpu::Sampler,
    animations: animate::SpriteAnimator,
    material_animations: Vec<SpriteAnimation>,
    active_animated_layers: Vec<u64>,
    uploads: upload::Uploads,
    /// Section meshes are staged through their own belt into
    /// `section_copies`, which the next frame records its commands after:
    /// `Queue::write_buffer` would make a staging buffer per write.
    section_uploads: upload::Uploads,
    section_copies: Option<wgpu::CommandEncoder>,
    /// Bytes staged in `section_copies`.
    section_copy_bytes: u64,
    /// When a large translucent page may start to shrink (`shrink_translucent`).
    shrink_gate: ShrinkGate,
    /// Whether `lightmap` holds `lightmap_chart`'s native pixels yet.
    lightmap_uploaded: bool,
    sprite_minimum_size: u32,
    /// Each sprite layer's own side, before the arrays scaled it up (`mipmap::Settings::native_size`).
    sprite_native: Vec<u32>,
    /// The Mipmap Levels option (`set_mipmap_levels`), which sets a shader pack atlas's padding.
    mipmap_option: u32,
    indices: wgpu::Buffer,
    index_quads: u32,
    instances: wgpu::Buffer,
    instance_capacity: u64,
    outline: wgpu::Buffer,
    depth: wgpu::TextureView,
    sections: resident::Resident<SectionGpu>,
    lods: FxHashMap<crate::lod::TileKey, lod::Tile>,
    /// The same tiles by cell (`lod::TileGrid`), to test them a handful of
    /// boxes at a time (`draw_frame`).
    lod_cells: lod::TileGrid,
    /// Far tiles' occlusion culling slots (`lod::Tile::cull_slot`): those
    /// free, and the next never used.
    lod_cull_free: Vec<u32>,
    lod_cull_next: u32,
    /// The chunk columns the resident tiles leave out (`Lod::take_cutout`):
    /// their sections are drawn even past the render distance, so terrain
    /// leaving it never opens a hole before the tiles fill it in.
    lod_cutout: crate::fast_hash::FxHashSet<(i32, i32)>,
    /// Removed sections of `lod_cutout` columns, released once tiles cover them.
    lod_retiring: crate::fast_hash::FxHashSet<SectionKey>,
    /// Removed sections fading out over far terrain (`leave_section`).
    lod_leaving: Vec<(SectionKey, std::time::Instant)>,
    /// Finished tile meshes `upload_lods` has yet to upload.
    lod_pending: Option<lod::Pending>,
    /// When the fades the last tile job began are over (`upload_lods`): until
    /// then tiles may be dying or standing in for their replacements.
    lod_settled: Option<std::time::Instant>,
    detail_columns: FxHashMap<(i32, i32), SectionMask>,
    /// Bumped whenever a column becomes or stops being complete.
    detail_revision: u64,
    arena: arena::Arena,
    /// Translucent quads' own arena, with large pages, while the layout is split
    /// (`translucent_split`): the back-to-front list then stays in one page (and
    /// one sorted-index page) and draws in a few multi-draws rather than one per
    /// few sections.
    translucent_arena: Option<arena::Arena>,
    /// Translucent Batching (`Graphics::translucent_batching`, read when a renderer
    /// is made), where it is offered (`graphics::offer_translucent_batching`).
    translucent_batching: bool,
    /// Every section and far tile was dropped to change the layout since the game
    /// last asked (`take_dropped_sections`).
    sections_dropped: bool,
    sorted_indices: arena::Arena,
    /// Buffers of released arena pages, freed a few frames later on a thread of their own.
    graveyard: arena::Graveyard<wgpu::Buffer>,
    translucent_sorter: translucency::Sorter,
    translucency_cursor: usize,
    /// `DrawIndexedIndirectArgs` for this frame's solid then translucent draws.
    indirect: wgpu::Buffer,
    indirect_capacity: u64,
    /// Indirect draws may set `first_instance` (the section origin).
    pub multi_draw: bool,
    /// Per-frame scratch, kept to avoid allocating every frame.
    visible: Vec<resident::Visible>,
    visible_lods: Vec<lod::Visible>,
    /// The sections a shader pack's shadow camera sees (`external::ShadowView`).
    shadow_visible: Vec<resident::Visible>,
    /// The external shader pipeline world frames call (`external`); `None` draws vanilla's way.
    external: Option<Box<dyn external::ExternalPipeline>>,
    translucent_draws: Vec<(u64, usize, [u32; 5], (u32, Option<u32>))>,
    /// Per visible section: the occlusion test that also sees its translucent quads
    /// (`NO_TEST` without one): the section is left to phase 2.
    translucent_tests: Vec<u32>,
    /// This frame's translucent draws that a test sees: (index among them, test).
    translucent_marks: Vec<(u32, u32)>,
    /// Phase 2's draws and the translucent ones after them, with the instance counts the
    /// tests give (`Cull::upload`).
    phase2_upload: Vec<[u32; 5]>,
    origins: Vec<[f32; 4]>,
    /// `DrawIndexedIndirectArgs` words: index count, instance count, first
    /// index, base vertex, first instance.
    draws: Vec<[u32; 5]>,
    /// Vertex arena page and optional sorted index arena page of each draw:
    /// with sorted indices (a translucent draw), a page of `translucent_arena`
    /// when there is one.
    draw_pages: Vec<(u32, Option<u32>)>,
    /// Solid draws left to occlusion culling's phase 2, while the list is
    /// built, and their pages.
    phase2_draws: Vec<[u32; 5]>,
    phase2_pages: Vec<(u32, Option<u32>)>,
    /// Each section's solid draws, to group them by arena page.
    solid_runs: page_runs::PageRuns,
    phase2_runs: page_runs::PageRuns,
    /// The shadow camera's draws by page, in bands of distance from the light.
    shadow_runs: page_runs::PageRuns,
    /// Block-entity blocks per meshed section (world position, state id).
    block_entities: FxHashMap<SectionKey, Vec<([i32; 3], u32)>>,
    /// Changes on every mesh upload for that section; block-entity static
    /// geometry also depends on the section's packed light.
    block_entity_section_revisions: FxHashMap<SectionKey, u64>,
    /// Changes only when the block-entity positions/states in a section
    /// change. Empty-section tombstones remain until that section unloads so
    /// the static cache can remove deleted models without a global flush.
    block_entity_membership_revisions: FxHashMap<SectionKey, u64>,
    /// When each section was first uploaded (`RenderSection.uploadedTime`): kept across meshing
    /// it again, gone with its column. Sections that fade in over far terrain are not in it.
    section_uploaded: FxHashMap<SectionKey, std::time::Instant>,
    /// When `section_uploaded` last gained a section: once the fade time has passed since,
    /// every section is whole and frames skip the per-section lookups.
    newest_upload: Option<std::time::Instant>,
    /// When a section last began to fade in over far terrain (`SectionGpu::fading`), for the
    /// same shortcut.
    newest_lod_fade: Option<std::time::Instant>,
    /// Changes whenever `block_entities` does (unique across renderers).
    block_entity_revision: u64,
    /// The light sources of each section and their revision, for `block_light`.
    light_blocks: FxHashMap<SectionKey, Vec<([i32; 3], u32)>>,
    light_revision: u64,
    /// VisGraph face connectivity per meshed section (cave culling).
    occlusion: occlusion::Occlusion,
    /// The column `occlusion`, `sections` and `detail_columns` count in: the
    /// level's (`clear_sections`).
    layout: crate::chunk::ColumnLayout,
    /// Resolution scale, FXAA and TAA (Sunrise settings).
    world_post: world_post::Post,
    /// Frame generation (Sunrise setting).
    frame_gen: frame_gen::FrameGen,
    /// The entities' texture atlas (`entity_group`'s), for frame generation's motion.
    entity_atlas: wgpu::TextureView,
    /// HDR output (Sunrise setting): `Some` while the swapchain is HDR.
    hdr: Option<hdr::Hdr>,
    /// Which pixels of each block sprite may shine in HDR output (terrain.wgsl's
    /// `emission_thresholds`), measured when HDR output turns on.
    emission_stats: hdr_stats::EmissionStats,
    /// What this renderer's destination offers and its device draws.
    output_caps: output::OutputCapabilities,
    /// An output that failed to apply, not tried again until something it needs changes.
    output_rejected: Option<output_apply::Rejected>,
    /// Since when Auto keeps HDR on through the display's loss of its report.
    auto_off_since: Option<std::time::Instant>,
    /// The output applied (`update_output`), with the frame every pipeline that draws it
    /// targets: the swapchain's 8-bit format, or `Rgba16Unorm` when the renderer was made with
    /// HDR output on and Smooth Gradients and the device draws and blends into it, or
    /// `LINEAR_FRAME` (`frame_format`). A 16-bit frame with HDR off since reaches the
    /// swapchain through `readback`.
    output: output::EffectiveOutput,
    /// Whether a pack picked or dropped since the renderer was made has been told it runs on
    /// the frame that exists (`set_native_shader_pack`).
    frame_kept_logged: bool,
    /// The marker file of an HDR output that has not presented a frame yet
    /// (`graphics::HDR_MARKER`).
    hdr_marker: Option<std::path::PathBuf>,
    /// The window's graphics driver (`hdr_start`); `None` offscreen.
    hdr_driver: Option<String>,
    /// The HDR Lighting setting, kept for the HDR output made when HDR turns on.
    hdr_lighting_setting: bool,
    /// The shader pack's post-processing, and the dimension (legacy id) it was loaded for.
    pack_post: Option<(i32, Option<pack_post::PackPost>)>,
    /// A `shaders/` pack whose pipelines build on a thread of their own (`load_shader_pack`).
    pack_loading: Option<PackLoading>,
    /// The post passes of one of Sunrise's own shader packs; never beside `pack_post`'s.
    native: Option<native_pack::NativePack>,
    /// Time the shader pack's passes on the GPU (F3, or a capture made with `--gpu`).
    pass_timing: bool,
    /// Times the parts of passes too (`set_pass_detail`).
    pass_detail: bool,
    /// Least time between timed frames (`set_pass_timing_gap`).
    pass_timing_gap: std::time::Duration,
    /// Whether the timers keep every timed frame for `take_timed_frames`.
    keep_timed_frames: bool,
    /// Times Sunrise's own passes on the GPU, as `pass_timing` does (`own_pass_times`).
    own_timing: Option<pack_timing::PassTiming>,
    /// The entity vertices Sunrise draws itself while a shader pack draws the others.
    pack_entity_rest: Vec<Vertex>,
    /// Set 1 of the cloud pipeline, for a shader pack's `gbuffers_clouds`.
    cloud_bindings: wgpu::BindGroupLayout,
    /// Rain and snow handed to a shader pack's `gbuffers_weather`, once a pack draws them.
    pack_weather: Option<pack_weather::PackWeather>,
    /// The block sprites as one atlas, while a shader pack is loaded.
    pack_atlas: Option<pack_atlas::PackAtlas>,
    pack_material_atlases: Option<[pack_atlas::PackAtlas; 2]>,
    /// The shared atlas's LabPBR normal and specular paged maps (`set_shared_material_maps`),
    /// which shader packs read for entities, items and the hand.
    shared_material_maps: [Option<wgpu::Texture>; 2],
    /// The pack's terrain program reads flat draws' vertices (`pack_terrain_inputs`).
    pack_terrain_flat: bool,
    /// OptiFine/Iris separate ambient occlusion: pass vertex alpha to hz_Color.a.
    pub separate_ao: bool,
    /// Shader-pack terrain pages used by post processing, retained between frames.
    pack_pages: Vec<pack_post::TerrainPage>,
    /// Camera-relative selection outline vertices, retained between frames.
    outline_edges_scratch: Vec<[f32; 6]>,
    /// Selection outline line vertices, retained between frames.
    outline_lines_scratch: Vec<[f32; 8]>,
    /// Debug line vertices, retained between frames.
    debug_lines_scratch: Vec<[f32; 8]>,
    /// Screen effect vertices, retained between frames.
    screen_vertices_scratch: Vec<crate::screen_effects::ScreenVertex>,
    /// LOD leaving chunk columns, retained between frames.
    lod_leaving_scratch: crate::fast_hash::FxHashSet<(i32, i32)>,
    /// Shadow visible boxed sections, retained between frames.
    shadow_boxed_scratch: Vec<resident::Visible>,
    /// Shadow visible seen section keys, retained between frames.
    shadow_seen_scratch: crate::fast_hash::FxHashSet<SectionKey>,
    /// Shadow translucent section draws, retained between frames.
    shadow_translucent_scratch: Vec<(u64, [u32; 5], (u32, Option<u32>))>,
    /// Lava's sprite layers, which a pack's solid terrain draws (`pack_atlas::game_terrain`).
    pack_lava_layers: Vec<u16>,
    pub hud: HudRenderer,
    /// Vanilla's menu background blur, made on first use.
    menu_blur: Option<crate::menu_blur::MenuBlur>,
    /// `post_effect` chains, made on first use.
    post_effects: Option<crate::post_chain::PostEffects>,
    /// Where a screenshot's frame is drawn when the swapchain cannot be read back.
    readback: readback::Readback,
    pub info: wgpu::AdapterInfo,
    pub suspended: bool,
    /// Consecutive frames whose surface acquisition failed validation. The
    /// first gets the swapchain a fresh configuration; later ones reattach
    /// the window, as a lost surface does.
    acquire_validation_failures: u32,
    /// A surface from `recreate_surface` whose configuration failed. wgpu
    /// panics when such a surface is acquired from (it never had one that
    /// worked), so frames configure it again first and skip until it takes.
    surface_unconfigured: bool,
    /// The last frame's statistics, all of one frame (`show_stats`): its sections drawn
    /// (F3's `C:`, the world icon's check), section draws (face ranges and translucent
    /// sections) and the draw calls the backend issued for them. A CPU frame's are its own;
    /// in GPU frames (`gpu_culling`) they are the newest GPU frame's whose counts came back,
    /// a frame or a few earlier (`stats_lag`).
    pub drawn_sections: usize,
    pub section_draws: usize,
    pub section_draw_calls: usize,
    /// `draw_frame`'s frames past its early returns, and the one whose statistics are shown.
    frame_serial: u64,
    stats_frame: u64,
    pub uploaded_bytes: u64,
    started: Instant,
    /// The next frame to write to a PNG path (control socket, F2, world icon).
    pub screenshot: Option<ScreenshotRequest>,
    /// Readback outcome: CPU pixels for the game to approve before writing, or an error.
    pub screenshot_result: Option<std::result::Result<PendingScreenshot, String>>,
    screenshot_readbacks: screenshot_readback::Readbacks,
    /// Screenshot Quality (Sunrise setting): a `scaled` screenshot's frame is this many
    /// times the game's resolution (1, 2, 4 or 8).
    pub screenshot_quality: u32,
    /// How many times the pending scaled screenshot ran out of video memory: each
    /// halves its size, down to the window's.
    screenshot_halvings: u32,
    /// Brightness option (lightmap gamma), 0..1.
    pub gamma: f32,
    /// Optional full-light lightmap (Sunrise Extras).
    pub fullbright: bool,
    /// The Improved transparency option (`GameRenderer.useImprovedTransparency`).
    pub improved_transparency: bool,
    /// Its passes and targets, made when first drawn with.
    oit: Option<oit::Oit>,
    /// Advance animated block textures (Sunrise setting; vanilla always does).
    pub animated_textures: bool,
    present_modes: Vec<wgpu::PresentMode>,
    /// CPU copies of the section and far tile meshes uploaded while `render-capture` is on
    /// (`keep_capture_copies`, or `SUNRISE_RENDER_CAPTURE` at creation); `None` otherwise.
    capture_copies: Option<capture::Copies>,
    /// A scene to write from the next frame (`sunrise-ctl render-capture DIR`).
    pub capture: Option<capture::Request>,
    /// Outcome of the last capture: its summary, or the error.
    pub capture_result: Option<std::result::Result<serde_json::Value, String>>,
}
/// Every layer's mip levels, layer major (`first` is the first layer's
/// index into `settings`), made by `MipmapGenerator`.
/// Block sprite layers per texture array at most: the
/// `max_texture_array_layers` of most GPUs. Mods add sprites past it into more
/// arrays (terrain.wgsl's `sprite`); a GPU with fewer layers per array gets a
/// smaller split ([`layer_split`]).
pub(crate) const SPRITE_SPLIT: u32 = 2048;
/// A smaller split from `SUNRISE_SPRITE_SPLIT` (dev only: vanilla's sprites
/// drawn from several arrays must look the same).
fn requested_split() -> Option<u32> {
    std::env::var("SUNRISE_SPRITE_SPLIT")
        .ok()
        .and_then(|v| v.parse().ok())
        .filter(|&n| (1..=SPRITE_SPLIT).contains(&n))
}
fn adapter_requested_split(info: &wgpu::AdapterInfo, asked: Option<u32>, n: u32) -> Option<u32> {
    // Metal can fail on big mipmapped arrays (#8471): 512 layers each, more only for n > 8 x 512.
    asked.or((info.backend == wgpu::Backend::Metal).then(|| n.div_ceil(SPRITE_ARRAYS).max(512)))
}
/// The split the CPU mip chains are cut by before the adapter is known.
fn sprite_split() -> u32 {
    requested_split().unwrap_or(SPRITE_SPLIT)
}
/// The block sprite layers per texture array on an adapter that holds
/// `max_layers` layers per array: [`SPRITE_SPLIT`], `requested`
/// (`SUNRISE_SPRITE_SPLIT`) or `max_layers`, whichever is smallest, with a full
/// array still fitting once [`array_layers`] pads it for OpenGL. Errors when
/// `layers` sprites need more than the [`SPRITE_ARRAYS`] arrays terrain.wgsl binds.
fn layer_split(
    max_layers: u32,
    gl: bool,
    requested: Option<u32>,
    layers: u32,
) -> std::result::Result<u32, String> {
    let mut split = requested.unwrap_or(SPRITE_SPLIT).min(max_layers).max(1);
    if split > 1 && array_layers(split, gl) > max_layers {
        split -= 1;
    }
    if layers > split * SPRITE_ARRAYS {
        return Err(format!(
            "GPU supports {max_layers} texture layers per array; {layers} block textures need \
             more than the {SPRITE_ARRAYS} arrays of {split} Sunrise draws from"
        ));
    }
    Ok(split)
}
/// Sprite arrays terrain.wgsl binds (`sprites`, `sprites_1`...).
const SPRITE_ARRAYS: u32 = 8;
/// The layers to give a square texture of `count` layers viewed as a 2D
/// array. wgpu's OpenGL backend picks a texture's target from its layer
/// count when it is created: one layer makes a 2D texture and a multiple of
/// six a cube map, neither of which can be viewed as an array.
pub(crate) fn array_layers(count: u32, gl: bool) -> u32 {
    match count {
        _ if !gl => count,
        0 | 1 => 2,
        n if n % 6 == 0 => n + 1,
        n => n,
    }
}
/// The views of terrain.wgsl's sprite arrays, the unused ones a 1-layer
/// stand-in.
fn sprite_views(
    textures: &[wgpu::Texture],
    unused: &wgpu::TextureView,
    mips: Option<u32>,
) -> Vec<wgpu::TextureView> {
    (0..SPRITE_ARRAYS as usize)
        .map(|i| match textures.get(i) {
            Some(t) => t.create_view(&wgpu::TextureViewDescriptor {
                dimension: Some(wgpu::TextureViewDimension::D2Array),
                mip_level_count: mips,
                ..Default::default()
            }),
            None => unused.clone(),
        })
        .collect()
}
/// The block sprites sampled sharp, for native mods (`WorldDrawn::block_sprites`): nearest,
/// repeating, the nearest mip level.
const SPRITE_NEAREST: wgpu::SamplerDescriptor<'static> = wgpu::SamplerDescriptor {
    label: Some("Block sprites (nearest)"),
    address_mode_u: wgpu::AddressMode::Repeat,
    address_mode_v: wgpu::AddressMode::Repeat,
    address_mode_w: wgpu::AddressMode::ClampToEdge,
    mag_filter: wgpu::FilterMode::Nearest,
    min_filter: wgpu::FilterMode::Nearest,
    mipmap_filter: wgpu::MipmapFilterMode::Nearest,
    lod_min_clamp: 0.0,
    lod_max_clamp: 32.0,
    compare: None,
    anisotropy_clamp: 1,
    border_color: None,
};
/// The terrain sprites' sampler: LevelRenderer.chunkLayerSampler's linear one (clamped to the
/// edge) in every Texture Filtering mode, with `anisotropy` samples when it is above 1 (wgpu
/// then needs a linear mipmap filter too).
fn sprite_sampler_descriptor(anisotropy: u16) -> wgpu::SamplerDescriptor<'static> {
    wgpu::SamplerDescriptor {
        label: Some("Block sprites (filtered)"),
        address_mode_u: wgpu::AddressMode::ClampToEdge,
        address_mode_v: wgpu::AddressMode::ClampToEdge,
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        mipmap_filter: wgpu::MipmapFilterMode::Linear,
        anisotropy_clamp: anisotropy,
        ..Default::default()
    }
}
/// `Options.maxAnisotropyValue` for a `maxAnisotropyBit`: `1 << bit`, at most wgpu's 16.
fn sampler_anisotropy(bit: i32) -> u16 {
    1u16 << bit.clamp(0, 4)
}
/// terrain.wgsl's group 0.
#[allow(clippy::too_many_arguments)]
fn terrain_group(
    device: &wgpu::Device,
    label: &str,
    layout: &wgpu::BindGroupLayout,
    globals: &wgpu::Buffer,
    sprites: &[wgpu::TextureView],
    sprite_sampler: &wgpu::Sampler,
    lightmap: &wgpu::TextureView,
    lightmap_sampler: &wgpu::Sampler,
    emission_thresholds: &wgpu::TextureView,
) -> wgpu::BindGroup {
    let view = |binding, view| wgpu::BindGroupEntry {
        binding,
        resource: wgpu::BindingResource::TextureView(view),
    };
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some(label),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: globals.as_entire_binding(),
            },
            view(1, &sprites[0]),
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::Sampler(sprite_sampler),
            },
            view(3, lightmap),
            wgpu::BindGroupEntry {
                binding: 4,
                resource: wgpu::BindingResource::Sampler(lightmap_sampler),
            },
            view(5, &sprites[1]),
            view(6, &sprites[2]),
            view(7, &sprites[3]),
            view(8, &sprites[4]),
            view(9, &sprites[5]),
            view(10, &sprites[6]),
            view(11, &sprites[7]),
            view(12, emission_thresholds),
        ],
    })
}
mod prepared_textures;
pub use prepared_textures::*;
mod solid_pipeline;
pub use solid_pipeline::*;
mod frame;
pub use frame::*;
mod device;
pub use device::*;
/// Logs a step of the renderer's start, for a native crash in the graphics
/// driver: it leaves no panic or crash report, so the log's last line is the
/// only sign of where the game stopped. The log file is written unbuffered;
/// the flush is for loggers that buffer.
fn startup_step(step: std::fmt::Arguments) {
    log::info!("Sunrise renderer: {step}");
    log::logger().flush();
}
/// How far the first frame on a new device got, for `startup_step`.
#[derive(Clone, Copy, PartialEq, Eq)]
enum FirstFrame {
    Acquire,
    Submit,
    Drawn,
}
fn on_off(on: bool) -> &'static str {
    if on { "on" } else { "off" }
}
impl Renderer {
    /// The first-person hand's quads, which follow `first` quads of the entity buffer.
    /// Unmarks the clouds just drawn (`fs_hdr_cloud_mark`).
    fn mark_clouds(&self, pass: &mut wgpu::RenderPass<'_>, quads: u32) {
        if !light_marks(self.output.frame.format, self.hdr.is_some())
            || self.hdr.as_ref().is_none_or(|h| !h.lighting)
        {
            return;
        }
        pass.set_pipeline(&self.hdr_cloud_mark_pipeline);
        pass.set_bind_group(0, &self.entity_group, &[]);
        pass.set_bind_group(1, &self.cloud_group, &[]);
        pass.set_vertex_buffer(0, self.cloud_buffer.slice(..));
        pass.set_index_buffer(self.indices.slice(..), wgpu::IndexFormat::Uint32);
        pass.draw_indexed(0..quads * 6, 0, 0..1);
        pass.set_bind_group(0, &self.group, &[]);
    }
    /// Unmarks translucent terrain, over the draws that just blended it (`fs_translucent_mark`).
    fn mark_translucent_terrain(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        flat: Option<&flat::Flat>,
        (indirect, base): (&wgpu::Buffer, u32),
        solid_draws: u32,
        translucent_draws: u32,
    ) {
        if !light_marks(self.output.frame.format, self.hdr.is_some())
            || self.hdr.as_ref().is_none_or(|h| !h.lighting)
        {
            return;
        }
        match flat {
            Some(flat) => {
                pass.set_pipeline(&flat.translucent_mark);
                flat.draw(
                    pass,
                    flat.translucent_calls(),
                    flat_vertices(&self.arena, self.translucent_arena.as_ref()),
                );
                pass.set_bind_group(0, &self.group, &[]);
            }
            None => {
                pass.set_pipeline(&self.translucent_mark);
                pass.set_vertex_buffer(1, self.instances.slice(..));
                self.draw_sections(pass, indirect, base, solid_draws, translucent_draws);
            }
        }
        pass.set_index_buffer(self.indices.slice(..), wgpu::IndexFormat::Uint32);
    }
    /// The shared atlas's LabPBR maps (`HudAtlas::material_maps`), uploaded for the shader packs
    /// loaded from now on; a map too large for this GPU is left out.
    pub fn set_shared_material_maps(&mut self, maps: &[Option<crate::paged_maps::PagedMap>; 2]) {
        let limit = self.device.limits().max_texture_dimension_2d;
        let sizes: Vec<String> = (maps.iter().zip(["normals", "specular"]))
            .filter_map(|(m, name)| {
                m.as_ref()
                    .map(|m| format!("{name} {}x{}", m.width, m.height))
            })
            .collect();
        if !sizes.is_empty() {
            let bytes: usize = maps.iter().flatten().map(|m| m.pixels.len()).sum();
            log::info!(
                "shared atlas LabPBR maps: {} ({:.1} MiB)",
                sizes.join(", "),
                bytes as f64 / f64::from(1 << 20)
            );
        }
        self.shared_material_maps = maps.each_ref().map(|map| {
            let map = map.as_ref().filter(|m| m.width.max(m.height) <= limit)?;
            Some(self.device.create_texture_with_data(
                &self.queue,
                &wgpu::TextureDescriptor {
                    label: Some("Shared atlas material map"),
                    size: wgpu::Extent3d {
                        width: map.width,
                        height: map.height,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    usage: wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                },
                wgpu::util::TextureDataOrder::LayerMajor,
                &map.pixels,
            ))
        });
    }
    /// The pack's custom sky layer textures (`crate::custom_sky::load`).
    pub fn set_custom_sky(&mut self, pack: &dyn crate::custom_sky::SkyLayers) {
        self.custom_sky.set_pack(&self.device, &self.queue, pack);
    }
    /// The custom sky layers the next frame draws (`SkyLayers::draws`): the game refills it each
    /// frame, empty when the world's sky has none.
    pub fn custom_sky_draws(&mut self) -> &mut Vec<crate::custom_sky::Draw> {
        &mut self.custom_sky_draws
    }
    fn quad_indices(device: &wgpu::Device, quads: u32) -> wgpu::Buffer {
        surface_setup::ray_query::quad_indices(device, quads)
    }
    fn offscreen_target(
        device: &wgpu::Device,
        config: &wgpu::SurfaceConfiguration,
    ) -> wgpu::Texture {
        device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Offscreen frame"),
            size: wgpu::Extent3d {
                width: config.width,
                height: config.height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: config.format,
            usage: config.usage,
            view_formats: &config.view_formats,
        })
    }
    /// The share of real frames frame generation shows a made frame before (1 while it
    /// keeps up), `None` while it isn't running.
    /// The posed models' store size (`SkinFrame::base`), with entity skinning.
    pub fn skinned_models(&self) -> Option<u32> {
        self.entity_skin
            .as_ref()
            .map(entity_skin::EntitySkin::stored)
    }

    /// Whether ordinary entity models may be posed on the GPU (`Frame::skin`, `entity_skin::poses`).
    pub fn skins_entities(&self) -> bool {
        self.entity_skin.is_some()
            && entity_skin::poses(
                self.frame_gen.on,
                (self.pack_post.as_ref())
                    .map(|(_, p)| p.as_ref().is_some_and(|p| p.skins_entities())),
                self.native.is_some(),
                || native_render::needs().entity_spans,
            )
    }

    /// Whether the layers tagged `OIT_ENTITY` (the eyes render type's, horse
    /// markings, a slime's outer cube) may be posed on the GPU too
    /// (`mobs::skin::SkinModels::tagged`): only with improved transparency off,
    /// where they are ordinary entity quads. On, `EntityBatch` sorts them into
    /// its stages by the CPU vertices' `OIT_ENTITY` tag.
    pub fn skins_tagged(&self) -> bool {
        self.skins_entities() && !self.improved_transparency && self.pack_post.is_none()
    }

    pub fn frame_generation_share(&self) -> Option<f32> {
        self.frame_gen.made_share
    }
    /// Frame generation on or off (FSR 3.1's frame interpolation).
    pub fn set_frame_generation(&mut self, on: bool) {
        if !on {
            self.frame_gen.stop();
        }
        self.frame_gen.on = on;
    }
    /// `upscale`: output pixels per rendered pixel, for TAA to rebuild (1: native).
    /// `sharpness`: how much TAA's upscaled picture is sharpened, 0..=1.
    pub fn set_world_post(
        &mut self,
        scale: f32,
        antialias: bool,
        taa: bool,
        upscale: f32,
        sharpness: f32,
    ) {
        self.world_post.set_sharpness(&self.device, sharpness);
        let taa = taa && self.world_post.supports_taa();
        self.world_post.scale = scale.clamp(0.25, 2.0);
        self.world_post.antialias = antialias && !taa;
        self.world_post.taa = taa;
        self.world_post.upscale = if taa { upscale.clamp(1.0, 4.0) } else { 1.0 };
    }
    /// The Texture Filtering and Max Anisotropy options (`filtering`: 0 none, 1 RGSS, 2
    /// anisotropic; `bit`: `maxAnisotropyBit`). All take LevelRenderer's chunk layer sampler
    /// (linear, clamped, `bit` anisotropy when anisotropic) and terrain.wgsl's sampleNearest,
    /// or sampleRGSS for RGSS.
    pub fn set_texture_filtering(&mut self, filtering: i32, bit: i32) {
        let filtering = filtering.clamp(0, 2) as u32;
        let anisotropy = if filtering == 2 {
            sampler_anisotropy(bit)
        } else {
            1
        };
        if filtering == self.texture_filtering && anisotropy == self.max_anisotropy {
            return;
        }
        self.texture_filtering = filtering;
        self.max_anisotropy = anisotropy;
        self.sprite_sampler = self
            .device
            .create_sampler(&sprite_sampler_descriptor(anisotropy));
        self.rebuild_terrain_group();
    }
    fn rebuild_terrain_group(&mut self) {
        self.sprite_views = sprite_views(
            &self.sprite_textures,
            &self.sprite_unused,
            Some(self.mipmaps + 1),
        );
        self.group = terrain_group(
            &self.device,
            "Sunrise terrain resources",
            &self.terrain_layout,
            &self.globals,
            &self.sprite_views,
            &self.sprite_sampler,
            &self.lightmap_view,
            &self.lightmap_sampler,
            &self.emission_stats.view,
        );
        self.fluid_group = terrain_group(
            &self.device,
            "Still fluid resources",
            &self.terrain_layout,
            &self.globals,
            &self.sprite_views,
            &self.fluid_sampler,
            &self.lightmap_view,
            &self.lightmap_sampler,
            &self.emission_stats.view,
        );
        self.terrain_model_group = terrain_group(
            &self.device,
            "Native model resources",
            &self.terrain_layout,
            &self.globals,
            &self.sprite_views,
            &self.terrain_model_sampler,
            &self.lightmap_view,
            &self.lightmap_sampler,
            &self.emission_stats.view,
        );
    }
    /// Block sprite layers per array on this device: a sprite's array is its layer over it, its
    /// layer in that array the rest (`block_sprites::Lookup`).
    pub fn sprite_split(&self) -> u32 {
        self.sprite_split
    }
    /// The Mipmap Levels option: block textures sample at most `levels`
    /// reduced copies (0: none).
    pub fn set_mipmap_levels(&mut self, levels: u32) {
        self.mipmap_option = levels;
        let levels = crate::sprites::sprite_mipmap_limit(
            levels,
            self.sprite_textures[0].width(),
            self.sprite_minimum_size,
            self.mip_levels,
        );
        if levels == self.mipmaps {
            return;
        }
        self.mipmaps = levels;
        self.rebuild_terrain_group();
    }
    fn animate(&mut self, encoder: &mut wgpu::CommandEncoder, tick: u64, cull_inactive: bool) {
        let active = cull_inactive.then_some(&self.active_animated_layers[..]);
        self.animations.animate_modes(
            encoder,
            &mut self.uploads,
            tick,
            active,
            self.own_timing.as_mut(),
        );
        if let Some(arrays) = &self.pbr_maps {
            for animation in &self.material_animations {
                if !self.animations.changed().contains(&animation.layer) {
                    continue;
                }
                let Some(pair) = arrays.get((animation.layer / self.sprite_split) as usize) else {
                    continue;
                };
                for kind in 0..2 {
                    let Some(pixels) = animation.material_pixels(kind, tick) else {
                        continue;
                    };
                    let texture = &pair[kind];
                    let from = (pixels.len() / 4).isqrt() as u32;
                    for level in 0..texture.mip_level_count() {
                        let size = texture.width() >> level;
                        let pixels = crate::sprites::resample(pixels, from, size);
                        self.uploads.texture(
                            encoder,
                            wgpu::TexelCopyTextureInfo {
                                texture,
                                mip_level: level,
                                origin: wgpu::Origin3d {
                                    x: 0,
                                    y: 0,
                                    z: animation.layer % self.sprite_split,
                                },
                                aspect: wgpu::TextureAspect::All,
                            },
                            size,
                            size,
                            &pixels,
                        );
                    }
                }
            }
            if let Some(atlases) = &self.pack_material_atlases {
                for kind in 0..2 {
                    let arrays: Vec<_> = arrays.iter().map(|pair| pair[kind].clone()).collect();
                    atlases[kind].copy(
                        &self.device,
                        encoder,
                        &arrays,
                        self.animations.changed().iter().copied(),
                    );
                }
            }
        }
        if let Some(atlas) = &self.pack_atlas {
            atlas.copy(
                &self.device,
                encoder,
                &self.sprite_textures,
                self.animations.changed().iter().copied(),
            );
        }
    }
    #[sunrise_mod_api_macros::replaceable]
    pub fn render(&mut self, frame: &Frame) -> Result<bool> {
        // A lost device cannot accept uploads either. Leave recovery to the
        // owner instead of submitting atlas patches to the invalid device.
        if self.device_lost() {
            return Err("the graphics device was lost".into());
        }
        if let Some(request) = self.capture.take() {
            let result = self.write_capture(&request, frame);
            self.capture_result = Some(result.map_err(|e| e.to_string()));
        }
        let scaled = self.screenshot_size();
        let rendered = match scaled {
            Some(size) => self.draw_scaled_frame(frame, size),
            None => self.draw_frame(frame, None, self.output),
        };
        self.finish_prepared_frame();
        // A frame that was not drawn still submits the section copies and
        // atlas patches (glyphs, skins, maps, mods' dynamic textures), which the caller
        // already took.
        if self.device_lost() {
            return Err("the graphics device was lost".into());
        }
        let patched = !frame.hud.patches.is_empty() || !frame.hud.image_patches.is_empty();
        if !matches!(rendered, Ok(true)) && patched {
            let section_copies = self.section_copies.take();
            let has_section_copies = section_copies.is_some();
            let mut encoder = section_copies.unwrap_or_else(|| {
                self.device
                    .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                        label: Some("Sunrise atlas patches"),
                    })
            });
            self.hud
                .upload_patches(&mut self.uploads, &mut encoder, frame.hud);
            self.uploads.finish(&encoder);
            if has_section_copies {
                self.section_copy_bytes = 0;
                self.section_uploads.finish(&encoder);
            }
            self.queue.submit([encoder.finish()]);
        } else if let Some(copies) = self.take_section_copies() {
            self.queue.submit([copies]);
        }
        // A scaled screenshot's frame never reached the window.
        rendered.map(|drawn| drawn && scaled.is_none())
    }
    /// Whether the device's loss callback has fired. A surface reconfigure
    /// alone cannot restore resources belonging to a lost device.
    pub fn device_lost(&self) -> bool {
        self.device_lost.load(std::sync::atomic::Ordering::Relaxed)
    }
}
/// Encodes the copy of `texture`'s `size` pixels into a mappable buffer.
pub(crate) fn copy_frame(
    device: &wgpu::Device,
    encoder: &mut wgpu::CommandEncoder,
    texture: &wgpu::Texture,
    [width, height]: [u32; 2],
) -> wgpu::Buffer {
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("Sunrise screenshot"),
        size: u64::from(screenshot::padded_row(width)) * u64::from(height),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    encoder.copy_texture_to_buffer(
        texture.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(screenshot::padded_row(width)),
                rows_per_image: None,
            },
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
    buffer
}
/// Centre-crop the frame to a square and average it down to `size`.
pub(crate) fn square_downscale(pixels: &[u8], width: u32, height: u32, size: [u32; 2]) -> Vec<u8> {
    let (side, cx, cy) = if width > height {
        (height, (width - height) / 2, 0)
    } else {
        (width, 0, (height - width) / 2)
    };
    let [tw, th] = size;
    let mut out = vec![0u8; (tw * th * 4) as usize];
    for ty in 0..th {
        let y0 = cy + ty * side / th;
        let y1 = cy + (ty + 1) * side / th;
        for tx in 0..tw {
            let x0 = cx + tx * side / tw;
            let x1 = cx + (tx + 1) * side / tw;
            let (mut r, mut g, mut b) = (0u32, 0u32, 0u32);
            for y in y0..y1 {
                for x in x0..x1 {
                    let p = ((y * width + x) * 4) as usize;
                    r += u32::from(pixels[p]);
                    g += u32::from(pixels[p + 1]);
                    b += u32::from(pixels[p + 2]);
                }
            }
            let count = (x1 - x0) * (y1 - y0);
            let o = ((ty * tw + tx) * 4) as usize;
            out[o..o + 4].copy_from_slice(&[
                (r / count) as u8,
                (g / count) as u8,
                (b / count) as u8,
                255,
            ]);
        }
    }
    out
}
mod view_frustum;
pub use view_frustum::Frustum;

#[cfg(feature = "test-support")]
pub use view::test_frustum;
/// Vanilla's `Projection`: reversed depth (near maps to 1, far to 0, tests
/// are GREATER_THAN_OR_EQUAL), which keeps far faces a hair apart, like
/// fire_side's 0.01 pixel in front of its block, from z-fighting.
/// The section the camera is in.
fn camera_section(camera: DVec3, min_y: i32) -> SectionKey {
    (
        (camera.x.floor() as i32) >> 4,
        (camera.y.floor() as i32 - min_y) >> 4,
        (camera.z.floor() as i32) >> 4,
    )
}

/// The near plane of the world's projection, in blocks.
const NEAR_PLANE: f32 = 0.05;

pub(crate) fn world_projection(fov_degrees: f32, aspect: f32, far: f32) -> Mat4 {
    Mat4::perspective_rh(fov_degrees.to_radians(), aspect, far, NEAR_PLANE)
}

/// Calls `draw(first, quads)` for each run of a mesh's solid quads that can
/// face `eye` (`view::projection_center`), the mesh at the camera-relative
/// `origin`; all of them when there is no eye.
fn facing_runs(sides: &Sides, eye: Option<Vec3>, origin: [f32; 3], draw: impl FnMut(u32, u32)) {
    facing_runs_from(sides, eye, None, origin, Quads::All, draw)
}
/// `facing_runs` for the eye and, with `low`, every eye straight below it down to that
/// camera-relative height (the water mirror's eye).
fn facing_runs_from(
    sides: &Sides,
    eye: Option<Vec3>,
    low: Option<f32>,
    origin: [f32; 3],
    part: Quads,
    draw: impl FnMut(u32, u32),
) {
    match eye {
        Some(eye) => {
            let eye = eye - Vec3::from(origin);
            let low = low.map_or(eye.y, |y| (y - origin[1]).min(eye.y));
            sides.visible_from(eye.to_array(), low, part, draw)
        }
        None => sides.every(part, draw),
    }
}

#[cfg(test)]
mod frame_policy_tests;

#[cfg(test)]
mod tests;

#[cfg(test)]
mod sprite_upload_tests;

#[cfg(test)]
mod shadow_tests;

/// A `shaders/` pack building on its own thread (`Renderer::load_shader_pack`): the dimension it
/// is for, the thread (a panic in it contained), and what the game named it with and when.
/// Dropping it (another pack, none, or another dimension) cancels the build: the thread stops at
/// the next program instead of compiling the rest for nothing.
struct PackLoading {
    dimension: i32,
    /// Held to be dropped with it: that cancels the build.
    _cancel: CancelOnDrop,
    #[allow(clippy::type_complexity)]
    thread: std::thread::JoinHandle<
        std::result::Result<std::result::Result<pack_post::PackPost, pack_post::Refused>, String>,
    >,
    label: String,
    started: std::time::Instant,
}

/// Sets its flag when dropped: `PackLoading`'s cancel, which `PackPost::new` checks per program.
struct CancelOnDrop(std::sync::Arc<std::sync::atomic::AtomicBool>);

impl Drop for Renderer {
    /// Once the device is lost, the window's surfaces are left alive: wgpu refuses to present
    /// or discard a frame on a lost device and keeps that frame acquired, and destroying the
    /// swapchain under an acquired frame panics (while a panic unwinds, an abort).
    fn drop(&mut self) {
        if !self.device_lost() {
            return;
        }
        std::mem::forget(self.surface.take());
        std::mem::forget(
            self.spare_surface
                .get_mut()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .take(),
        );
    }
}

impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.store(true, std::sync::atomic::Ordering::Relaxed);
    }
}
