//! The typed submission surface; renderer storage is never exposed to mods.
use crate::game::ClientGame;
use crate::game::client_side::mods_client;
use crate::game::host::draw::GameWorldDrawContext;
use glam::{DVec3, Mat4, Vec3};
use mod_api::{
    DrawError, InstanceError, ModelInstance, ModelRef, NameTagOptions, PipelineDrawError,
    PipelineQuad, RenderLevelStage, RenderPipelineRef, WorldLine, WorldQuad, WorldTextOptions,
};
use sunrise_client::mesher::Vertex;
use sunrise_client::screen_effects::{CameraQuads, sampling};
use sunrise_client::terrain_model;
use sunrise_client::world_geometry::{self, ModelBatch, WorldGeometry};

/// Drives mod world rendering hooks from `Renderer::draw_frame`'s real visibility, sky, prepare,
/// pass-stage, and post-world boundaries.
pub(in crate::game) struct GameWorldStageDriver<'a, 'g> {
    game: &'a ClientGame<'g>,
    frame_camera: &'a mod_api::Camera,
    alpha: f64,
    draw: &'a mut WorldGeometry,
}

impl<'a, 'g> GameWorldStageDriver<'a, 'g> {
    pub(in crate::game) fn new(
        game: &'a ClientGame<'g>,
        frame_camera: &'a mod_api::Camera,
        alpha: f64,
        draw: &'a mut WorldGeometry,
    ) -> Self {
        Self {
            game,
            frame_camera,
            alpha,
            draw,
        }
    }

    fn view<'v>(
        game: &ClientGame<'_>,
        camera: &'v mod_api::Camera,
        alpha: f64,
        sections: &'v [mod_api::RenderableSection],
        oit_on: bool,
    ) -> mod_api::WorldRenderView<'v> {
        mod_api::WorldRenderView::new(
            camera,
            game.world_camera_entity_eye(),
            alpha as f32,
            game.common.world.game_time,
            game.animation_time(),
            oit_on,
            sections,
        )
    }
}

impl Drop for GameWorldStageDriver<'_, '_> {
    fn drop(&mut self) {
        self.game.discard_submitted_groups(self.draw);
    }
}

impl sunrise_client::gfx::world_primitives::FrameHooks for GameWorldStageDriver<'_, '_> {
    fn wants_after_sky(&self) -> bool {
        mod_api::__private::listened(mod_api::HookId::RenderLevelStage)
    }

    fn on_custom_geometry(&mut self, sections: &[mod_api::RenderableSection], oit_on: bool) {
        let view = Self::view(self.game, self.frame_camera, self.alpha, sections, oit_on);
        self.draw.begin_custom_geometry();
        self.game.native_submit_custom_geometry(&view, self.draw);
        self.game.submit_debug_geometry(&view, self.draw);
    }

    fn on_frame_graph_setup(&mut self, sections: &[mod_api::RenderableSection], oit_on: bool) {
        if !mod_api::__private::listened(mod_api::HookId::FrameGraphSetup) {
            return;
        }
        let view = Self::view(self.game, self.frame_camera, self.alpha, sections, oit_on);
        let surface = self.draw.frame_graph_surface(oit_on);
        self.game.native_frame_graph_setup(&view, surface);
    }

    fn take_frame_graph(
        &mut self,
    ) -> Option<sunrise_client::gfx::mod_frame_graph::RecordedFrameGraph> {
        self.draw.take_frame_graph()
    }

    fn on_after_sky(&mut self, sections: &[mod_api::RenderableSection], oit_on: bool) {
        let view = Self::view(self.game, self.frame_camera, self.alpha, sections, oit_on);
        let mut surface = self.draw.stage_surface(RenderLevelStage::AfterSky);
        self.game
            .native_render_level_stage(RenderLevelStage::AfterSky, &view, &mut surface);
    }

    fn on_prepare_render_buffers(
        &mut self,
        sections: &[mod_api::RenderableSection],
        oit_on: bool,
        out: &mut sunrise_client::gfx::world_primitives::PreparedHookGeometry,
    ) {
        let view = Self::view(self.game, self.frame_camera, self.alpha, sections, oit_on);
        self.game.native_prepare_render_buffers(&view, self.draw);
        self.draw.finalize();
        self.game.resolve_submitted_groups(self.draw);
        let camera = DVec3::from(self.draw.camera());
        self.game.bake_model_instances(
            self.draw,
            camera,
            &mut out.entities,
            &mut out.entity_spans,
            Some(&mut out.see_through),
        );
        self.draw.sort_labels();
        let [world, see_through] = self.draw.labels();
        if !world.is_empty()
            && let Some(span) =
                self.game
                    .entity_part_span(out.entities.len(), "minecraft:name_tag", true)
        {
            out.entity_spans.push(span);
        }
        out.entities.extend_from_slice(world);
        out.see_through.extend_from_slice(see_through);
        out.prepared_draws
            .append(&mut self.game.client.prepared_model_draws.borrow_mut());
        out.text_patches
            .append(&mut self.game.client.text_patches.borrow_mut());
    }

    fn on_render_stage(
        &mut self,
        stage: RenderLevelStage,
        sections: &[mod_api::RenderableSection],
        oit_on: bool,
    ) -> &[sunrise_client::gfx::world_primitives::DrawRanges] {
        let view = Self::view(self.game, self.frame_camera, self.alpha, sections, oit_on);
        let mut surface = self.draw.stage_surface(stage);
        self.game
            .native_render_level_stage(stage, &view, &mut surface);
        self.draw.stage_draws()
    }

    fn prepared_primitives(&self) -> sunrise_client::gfx::world_primitives::Batch<'_> {
        self.draw
            .batch_with(&self.game.client.client_pack.render.mod_pipelines)
    }

    fn on_after_level(&mut self, sections: &[mod_api::RenderableSection], oit_on: bool) {
        let view = Self::view(self.game, self.frame_camera, self.alpha, sections, oit_on);
        let mut surface = self.draw.stage_surface(RenderLevelStage::AfterLevel);
        self.game
            .native_render_level_stage(RenderLevelStage::AfterLevel, &view, &mut surface);
    }
}

/// Validate binding [`mod_api::TextureSource::WorldDepth`]: a depth-writing pipeline cannot sample
/// the depth target it writes (`InvalidBinding`), and an in-world raster pass needs adapter
/// read-only depth-stencil support (`TargetReadWriteHazard`).
fn check_world_depth_binding(
    writes_depth: bool,
    in_world_raster: bool,
    read_only_depth_stencil: bool,
) -> Result<(), PipelineDrawError> {
    if writes_depth {
        return Err(PipelineDrawError::InvalidBinding);
    }
    if in_world_raster && !read_only_depth_stencil {
        return Err(PipelineDrawError::TargetReadWriteHazard);
    }
    Ok(())
}

impl ClientGame<'_> {
    /// A camera-space quad in solid `argb`, of the atlas's white texel.
    pub(in crate::game) fn native_camera_quad(
        &self,
        draw: &mut CameraQuads,
        corners: [[f32; 3]; 4],
        argb: u32,
    ) -> bool {
        self.native_camera_sprite(draw, "sunrise/white", corners, [[0.5; 2]; 4], argb)
    }
    /// A camera-space quad of sprite `name`, sampled as `ScreenEffects::build` samples it; false
    /// for an unknown sprite or a coordinate that is not finite.
    pub(in crate::game) fn native_camera_sprite(
        &self,
        draw: &mut CameraQuads,
        name: &str,
        corners: [[f32; 3]; 4],
        uv: [[f32; 2]; 4],
        argb: u32,
    ) -> bool {
        let Some(r) = self.client.gui.atlas.texture_rect(name) else {
            return false;
        };
        let flags = self.client.gui.atlas.texture_flags(name);
        let sampling = flags.map_or(0, sampling);
        draw.push_on_atlas(
            corners,
            uv,
            [r.u0, r.v0, r.u1, r.v1],
            argb,
            sampling,
            self.client.gui.atlas.texture_atlas_page(name),
        )
    }
    /// `DebugRenderer.refreshRendererList`: replace the active callbacks with newly collected
    /// ones whose entries are on. Runs at start, at login and when the selection changes.
    pub(in crate::game) fn refresh_debug_renderers(&mut self) {
        struct EntryStates<'a> {
            selection: &'a sunrise_client::debug_entries::DebugEntries,
            entries: &'a [sunrise_client::debug_entries::RegisteredDebugEntry],
            overlay: bool,
            reduced: bool,
        }
        impl mod_api::__private::DebugEntryStates for EntryStates<'_> {
            fn is_enabled(&self, id: &str) -> bool {
                self.selection
                    .enabled(self.entries, id, self.overlay, self.reduced)
            }
        }
        let states = EntryStates {
            selection: &self.client.debug_entries.0,
            entries: &self.client.registered_debug_entries.0,
            overlay: self.client.debug,
            reduced: self.reduced_debug_info(),
        };
        let renderers = mods_client::debug_renderers(self.common.options.execution_role, &states);
        self.client.debug_renderers.0 = self.client.debug_entries.0.active_renderers(renderers);
    }
    fn submit_debug_geometry(&self, view: &mod_api::WorldRenderView<'_>, draw: &mut WorldGeometry) {
        let mut context = GameWorldDrawContext::new(self, draw);
        for renderer in &self.client.debug_renderers.0 {
            renderer.draw(self, view, &mut context);
        }
    }
    /// Whether any mod subscribes to world rendering hooks or active debug renderers.
    pub(in crate::game) fn has_world_render_hooks(&self) -> bool {
        !self.client.debug_renderers.0.is_empty()
            || mod_api::__private::listened(mod_api::HookId::SubmitCustomGeometry)
            || mod_api::__private::listened(mod_api::HookId::PrepareRenderBuffers)
            || mod_api::__private::listened(mod_api::HookId::RenderLevelStage)
            || mod_api::__private::listened(mod_api::HookId::FrameGraphSetup)
    }
    /// `DebugRenderer.emitGizmos`'s first half: a selection the list has not seen refreshes it.
    pub(in crate::game) fn follow_debug_selection(&mut self) {
        let version = self.client.debug_entries.0.enabled_version();
        if self.client.debug_renderers_version != Some(version) {
            self.client.debug_renderers_version = Some(version);
            self.refresh_debug_renderers();
        }
    }
    /// One view's geometry, the last view's gone first. `title` views draw no world: the
    /// title and connect screens have no mod geometry, so none outlives leaving a world.
    #[cfg(test)]
    pub(in crate::game) fn fill_world_geometry(
        &mut self,
        title: bool,
        frame_camera: Option<&mod_api::Camera>,
        camera: [f64; 3],
        view: Mat4,
        alpha: f64,
        oit_on: bool,
        sections: &[mod_api::RenderableSection],
        draw: &mut WorldGeometry,
        entities: &mut Vec<Vertex>,
    ) {
        let white = self.client.gui.atlas.sprites["sunrise/white"];
        draw.reset(
            camera,
            view,
            [(white.u0 + white.u1) * 0.5, (white.v0 + white.v1) * 0.5],
        );
        if title {
            return;
        }
        let Some(frame_camera) = frame_camera else {
            return;
        };
        draw.set_model_cut(entities.len() as u32);
        // Seconds of game time, wrapping every hour (72,000 ticks) so f32 keeps sub-tick steps.
        draw.set_time(
            ((self.common.world.game_time.rem_euclid(72_000) as f64 + alpha) / 20.0) as f32,
        );
        let view = mod_api::WorldRenderView::new(
            frame_camera,
            self.world_camera_entity_eye(),
            alpha as f32,
            self.common.world.game_time,
            self.animation_time(),
            oit_on,
            sections,
        );
        // Exact NeoForge 26.3 / vanilla 26.3 frame sequence:
        // 1. Custom geometry submission + debug renderers (deferred to STAGE_CUSTOM_GEOMETRY).
        draw.begin_custom_geometry();
        self.native_submit_custom_geometry(&view, draw);
        self.follow_debug_selection();
        self.submit_debug_geometry(&view, draw);
        // 2. AfterSky (no active render pass).
        {
            let mut surface = draw.stage_surface(RenderLevelStage::AfterSky);
            self.native_render_level_stage(RenderLevelStage::AfterSky, &view, &mut surface);
        }
        // 3. Prepare render buffers before pass-bearing stages run, then resolve queued groups
        // using their prepared transforms, lengths, and records.
        self.native_prepare_render_buffers(&view, draw);
        self.resolve_submitted_groups(draw);
        // 4. Pass-bearing stages in exact frame order (classic or OIT).
        for &stage in RenderLevelStage::frame_order(oit_on) {
            if !stage.has_render_pass() {
                continue;
            }
            let mut surface = draw.stage_surface(stage);
            self.native_render_level_stage(stage, &view, &mut surface);
        }
        // 5. AfterLevel (no active render pass).
        {
            let mut surface = draw.stage_surface(RenderLevelStage::AfterLevel);
            self.native_render_level_stage(RenderLevelStage::AfterLevel, &view, &mut surface);
        }
        // 6. Close pass stages, append deferred custom geometry, commit model instances and labels.
        draw.finalize();
        self.commit_model_instances(draw, DVec3::from(camera), entities);
        // Labels (`name_tag`) draw with the world's translucent name tags, far to near, their
        // see-through copies over everything.
        draw.sort_labels();
        let [world, see_through] = draw.labels();
        if !world.is_empty() {
            self.mark_translucent_entity(entities.len(), "minecraft:name_tag");
            entities.extend_from_slice(world);
        }
        self.client
            .see_through_vertices
            .extend_from_slice(see_through);
    }
    /// A mod's label as `SubmitNodeCollection.submitNameTag` draws a name: 0.5 above `at`,
    /// facing the camera, `options.scale` blocks a font pixel (a name tag's 0.025 by default).
    /// See-through, twice: opaque white text in the world, its light raised to at least 2
    /// unless `exact_light` (no backdrop), and the faded text on the backdrop over everything,
    /// unlit. Otherwise once, depth tested and
    /// translucent: the faded text on the backdrop in the caller's light. The faded text's alpha
    /// (unless `faded_alpha`) and the backdrop's follow the player's text background opacity.
    /// Refused whole, before layout, for a position that is not finite
    /// ([`DrawError::NonFinite`]) or a scale that is not finite and positive
    /// ([`DrawError::InvalidScale`]); after layout, for a finite scale whose text positions
    /// overflow f32 ([`DrawError::NonFinite`]). Any newly rasterized glyphs are uploaded with
    /// the label, and also when it is refused after layout, as the atlas already holds them.
    pub(in crate::game) fn native_world_name_tag(
        &self,
        draw: &mut WorldGeometry,
        at: [f64; 3],
        text: &mod_api::Text,
        options: NameTagOptions,
    ) -> Result<(), DrawError> {
        draw.check_label(at)?;
        if !options.scale.is_finite() || options.scale <= 0.0 {
            return Err(DrawError::InvalidScale);
        }
        let text = sunrise_client_lib::api_text::to_engine_inprocess(text);
        let from_camera = DVec3::from(at) - DVec3::from(draw.camera());
        let o = from_camera + DVec3::new(0.0, 0.5, 0.0);
        let lines = [(text, 0.0)];
        // Options.getBackgroundOpacity(0.25), and the faded text alpha that goes with it.
        let backdrop = if options.background {
            self.client.settings.text_background(0.25)
        } else {
            0
        };
        let faded = options
            .faded_alpha
            .map_or_else(|| self.client.settings.faded_text_alpha(0.25), u32::from);
        let light = options.light;
        // PoseStack.scale(s, -s, s) in place of the name tag's 0.025.
        let unit = options.scale;
        let (mut world, mut see_through) = (Vec::new(), Vec::new());
        let quads = |alpha, backdrop, layer, light, out: &mut Vec<Vertex>| {
            self.styled_name_tag_quads(o, &lines, alpha, backdrop, layer, light, unit, out, None)
        };
        let mut patches = if options.see_through {
            // LightCoordsUtil.lightCoordsWithEmission(light, 2): sky and block at least 2.
            let lit = sunrise_client::block_entity_render::vertex_light(if options.exact_light {
                light
            } else {
                (light >> 4).max(2) << 4 | (light & 15).max(2)
            });
            let mut patches = quads(0xFF, 0, 0, lit, &mut world);
            // RenderPipelines.TEXT_SEE_THROUGH: no depth test, no lightmap.
            patches.extend(quads(faded, backdrop, 2, 0, &mut see_through));
            patches
        } else {
            // One translucent copy in the light as given.
            let lit = sunrise_client::block_entity_render::vertex_light(light);
            quads(faded, backdrop, 0, lit, &mut world)
        };
        // A finite scale can still overflow the camera-relative positions (f32::MAX): refused
        // as any draw whose coordinates overflow f32, with nothing drawn.
        let finite = |v: &Vertex| v.pos.iter().all(|c| c.is_finite());
        if !world.iter().chain(&see_through).all(finite) {
            self.client.text_patches.borrow_mut().append(&mut patches);
            return Err(DrawError::NonFinite);
        }
        draw.label(from_camera.length_squared(), &world, &see_through);
        self.client.text_patches.borrow_mut().append(&mut patches);
        Ok(())
    }
    /// A mod's posed text, as a block entity's text is drawn (`world_text_quads`), under
    /// `pose` (row-major 3×4) at `origin`, the camera subtracted in `f64`. Depth tested, it
    /// draws with the labels the world's name tags draw with, far to near by `origin`, full
    /// bright when `glowing` (`LightCoordsUtil.FULL_BRIGHT`, as glowing sign text) and otherwise
    /// in `light`. See-through (`Font.DisplayMode.SEE_THROUGH`, `RenderPipelines.TEXT_SEE_THROUGH`:
    /// no depth test, no lightmap), with the labels' copies over everything. Refused whole,
    /// before layout, for an `origin`, `pose` or `at` that is not finite, and after it when a
    /// finite pose or origin still overflows a vertex to infinity or NaN.
    pub(in crate::game) fn native_world_text(
        &self,
        draw: &mut WorldGeometry,
        origin: [f64; 3],
        pose: [f32; 12],
        text: &mod_api::Text,
        options: WorldTextOptions,
    ) -> Result<f32, DrawError> {
        draw.check_label(origin)?;
        if !pose.iter().chain(&options.at).all(|v| v.is_finite()) {
            return Err(DrawError::NonFinite);
        }
        let text = sunrise_client_lib::api_text::to_engine_inprocess(text);
        let from_camera = DVec3::from(origin) - DVec3::from(draw.camera());
        let pose =
            Mat4::from_translation(from_camera.as_vec3()) * world_geometry::row_major_3x4(pose);
        let hud = |c: u32| {
            sunrise_client::hud::rgba((c >> 16) as u8, (c >> 8) as u8, c as u8, (c >> 24) as u8)
        };
        let look = super::native_block_entity_draw::TextLook {
            color: hud(options.color),
            shadow: options.shadow,
            outline: options.outline.map(hud),
            background: hud(options.background),
        };
        let (layer, light) = if options.see_through {
            (2, 0)
        } else {
            let light = if options.glowing { 0xFF } else { options.light };
            (0, sunrise_client::block_entity_render::vertex_light(light))
        };
        let mut quads = Vec::new();
        let width = self.world_text_quads(&text, options.at, look, pose, |quad| {
            quads.extend(quad.map(|v| Vertex { layer, light, ..v }));
            true
        });
        // A huge pose entry or origin can overflow the camera-relative corners: draw nothing.
        if !quads.iter().flat_map(|v| v.pos).all(f32::is_finite) {
            return Err(DrawError::NonFinite);
        }
        let (world, see_through): (&[Vertex], &[Vertex]) = if options.see_through {
            (&[], &quads)
        } else {
            (&quads, &[])
        };
        draw.label(from_camera.length_squared(), world, see_through);
        Ok(width)
    }
    /// Submit unlit ARGB world lines atomically. Invalid coordinates or widths leave the batch
    /// unchanged.
    pub(in crate::game) fn native_world_draw_lines(
        &self,
        draw: &mut WorldGeometry,
        lines: &[WorldLine],
    ) -> Result<(), DrawError> {
        draw.lines(lines)
    }
    /// Submit unlit, alpha-blended world quads atomically, using the white atlas texel.
    /// Refuses non-finite coordinates; quad storage grows dynamically into the GPU buffers.
    pub(in crate::game) fn native_world_draw_quads(
        &self,
        draw: &mut WorldGeometry,
        quads: &[WorldQuad],
    ) -> Result<(), DrawError> {
        draw.quads(quads)
    }
    /// Allocate a retained GPU [`mod_api::MeshRef`] on the active renderer.
    pub(in crate::game) fn create_mesh(
        &self,
        desc: &mod_api::MeshDescriptor<'_>,
    ) -> Result<mod_api::MeshRef, mod_api::MeshError> {
        let gpu = self.mod_gpu().ok_or(mod_api::MeshError::Unavailable)?;
        gpu.create_mesh(mod_api::content::crash::current_native_mod(), desc)
    }
    /// Update vertex buffer `binding` of retained `mesh`.
    pub(in crate::game) fn mesh_write_vertices(
        &self,
        mesh: mod_api::MeshRef,
        binding: u8,
        byte_offset: u64,
        data: &[u8],
    ) -> Result<(), mod_api::MeshError> {
        let gpu = self.mod_gpu().ok_or(mod_api::MeshError::Unavailable)?;
        gpu.mesh_write_vertices(
            mod_api::content::crash::current_native_mod(),
            mesh,
            binding,
            byte_offset,
            data,
        )
    }
    /// Update the index buffer of retained `mesh`.
    pub(in crate::game) fn mesh_write_indices(
        &self,
        mesh: mod_api::MeshRef,
        byte_offset: u64,
        data: &[u8],
    ) -> Result<(), mod_api::MeshError> {
        let gpu = self.mod_gpu().ok_or(mod_api::MeshError::Unavailable)?;
        gpu.mesh_write_indices(
            mod_api::content::crash::current_native_mod(),
            mesh,
            byte_offset,
            data,
        )
    }
    /// Reallocate backing GPU buffers of retained `mesh` in place.
    pub(in crate::game) fn mesh_replace(
        &self,
        mesh: mod_api::MeshRef,
        desc: &mod_api::MeshDescriptor<'_>,
    ) -> Result<(), mod_api::MeshError> {
        let gpu = self.mod_gpu().ok_or(mod_api::MeshError::Unavailable)?;
        gpu.mesh_replace(mod_api::content::crash::current_native_mod(), mesh, desc)
    }
    /// Free retained `mesh`'s GPU buffers and retire the handle.
    pub(in crate::game) fn mesh_release(
        &self,
        mesh: mod_api::MeshRef,
    ) -> Result<(), mod_api::MeshError> {
        let gpu = self.mod_gpu().ok_or(mod_api::MeshError::Unavailable)?;
        gpu.mesh_release(mod_api::content::crash::current_native_mod(), mesh)
    }
    /// Allocate a retained GPU [`mod_api::UniformBufferRef`] on the active renderer's device.
    pub(in crate::game) fn create_uniform_buffer(
        &self,
        bytes: &[u8],
    ) -> Result<mod_api::UniformBufferRef, mod_api::BufferError> {
        let gpu = self.mod_gpu().ok_or(mod_api::BufferError::Unavailable)?;
        gpu.create_uniform_buffer(mod_api::content::crash::current_native_mod(), bytes)
    }
    /// Partially or fully update retained uniform `buffer` starting at `byte_offset`.
    pub(in crate::game) fn uniform_buffer_write(
        &self,
        buffer: mod_api::UniformBufferRef,
        byte_offset: u32,
        bytes: &[u8],
    ) -> Result<(), mod_api::BufferError> {
        let gpu = self.mod_gpu().ok_or(mod_api::BufferError::Unavailable)?;
        gpu.uniform_buffer_write(
            mod_api::content::crash::current_native_mod(),
            buffer,
            byte_offset,
            bytes,
        )
    }
    /// Free retained uniform `buffer` and retire the handle.
    pub(in crate::game) fn uniform_buffer_release(
        &self,
        buffer: mod_api::UniformBufferRef,
    ) -> Result<(), mod_api::BufferError> {
        let gpu = self.mod_gpu().ok_or(mod_api::BufferError::Unavailable)?;
        gpu.uniform_buffer_release(mod_api::content::crash::current_native_mod(), buffer)
    }
    /// Allocate a retained GPU [`mod_api::StorageBufferRef`] on the active renderer's device.
    pub(in crate::game) fn create_storage_buffer(
        &self,
        desc: &mod_api::StorageBufferDescriptor,
    ) -> Result<mod_api::StorageBufferRef, mod_api::BufferError> {
        let gpu = self.mod_gpu().ok_or(mod_api::BufferError::Unavailable)?;
        gpu.create_storage_buffer(mod_api::content::crash::current_native_mod(), desc)
    }
    /// Queue `data` for `offset` of retained storage `buffer`.
    pub(in crate::game) fn storage_buffer_write_range(
        &self,
        buffer: mod_api::StorageBufferRef,
        offset: u64,
        data: &[u8],
    ) -> Result<(), mod_api::BufferError> {
        let gpu = self.mod_gpu().ok_or(mod_api::BufferError::Unavailable)?;
        gpu.storage_buffer_write_range(
            mod_api::content::crash::current_native_mod(),
            buffer,
            offset,
            data,
        )
    }
    /// The size retained storage `buffer` was created with.
    pub(in crate::game) fn storage_buffer_size(
        &self,
        buffer: mod_api::StorageBufferRef,
    ) -> Result<u64, mod_api::BufferError> {
        let gpu = self.mod_gpu().ok_or(mod_api::BufferError::Unavailable)?;
        gpu.storage_buffer_size(mod_api::content::crash::current_native_mod(), buffer)
    }
    /// Free retained storage `buffer` and retire the handle.
    pub(in crate::game) fn storage_buffer_release(
        &self,
        buffer: mod_api::StorageBufferRef,
    ) -> Result<(), mod_api::BufferError> {
        let gpu = self.mod_gpu().ok_or(mod_api::BufferError::Unavailable)?;
        gpu.storage_buffer_release(mod_api::content::crash::current_native_mod(), buffer)
    }
    /// Record a copy of retained storage `buffer`'s range at this point of the compute node being
    /// recorded.
    pub(in crate::game) fn storage_buffer_request_readback(
        &self,
        buffer: mod_api::StorageBufferRef,
        offset: u64,
        len: u64,
    ) -> Result<mod_api::ReadbackTicket, mod_api::BufferError> {
        let gpu = self.mod_gpu().ok_or(mod_api::BufferError::Unavailable)?;
        gpu.storage_buffer_request_readback(
            mod_api::content::crash::current_native_mod(),
            buffer,
            offset,
            len,
        )
    }
    /// The bytes of a readback, none while it is in flight.
    pub(in crate::game) fn readback_poll(
        &self,
        ticket: mod_api::ReadbackTicket,
    ) -> Result<Option<Vec<u8>>, mod_api::BufferError> {
        let gpu = self.mod_gpu().ok_or(mod_api::BufferError::Unavailable)?;
        gpu.readback_poll(mod_api::content::crash::current_native_mod(), ticket)
    }
    /// Drop a pending readback.
    pub(in crate::game) fn readback_release(
        &self,
        ticket: mod_api::ReadbackTicket,
    ) -> Result<(), mod_api::BufferError> {
        let gpu = self.mod_gpu().ok_or(mod_api::BufferError::Unavailable)?;
        gpu.readback_release(mod_api::content::crash::current_native_mod(), ticket)
    }
    /// Allocate a mod-owned offscreen [`mod_api::RenderTargetRef`] on the active renderer's device.
    pub(in crate::game) fn create_render_target(
        &self,
        desc: &mod_api::RenderTargetDescriptor,
    ) -> Result<mod_api::RenderTargetRef, mod_api::RenderTargetError> {
        let gpu = self
            .mod_gpu()
            .ok_or(mod_api::RenderTargetError::Unavailable)?;
        gpu.create_render_target(mod_api::content::crash::current_native_mod(), desc)
    }
    /// Resize mod-owned offscreen `target` (no-op if unchanged).
    pub(in crate::game) fn render_target_resize(
        &self,
        target: mod_api::RenderTargetRef,
        size: [u32; 2],
    ) -> Result<(), mod_api::RenderTargetError> {
        let gpu = self
            .mod_gpu()
            .ok_or(mod_api::RenderTargetError::Unavailable)?;
        gpu.render_target_resize(mod_api::content::crash::current_native_mod(), target, size)
    }
    /// Query pixel dimensions of mod-owned offscreen `target`.
    pub(in crate::game) fn render_target_size(
        &self,
        target: mod_api::RenderTargetRef,
    ) -> Result<[u32; 2], mod_api::RenderTargetError> {
        let gpu = self
            .mod_gpu()
            .ok_or(mod_api::RenderTargetError::Unavailable)?;
        gpu.render_target_size(mod_api::content::crash::current_native_mod(), target)
    }
    /// The optional device features and limits of the active device; all off (and zero)
    /// without one.
    pub(in crate::game) fn render_capabilities(&self) -> mod_api::RenderCapabilities {
        self.mod_gpu().map_or_else(
            || {
                mod_api::__private::built::render_capabilities(
                    false,
                    Vec::new(),
                    false,
                    false,
                    false,
                    false,
                    false,
                    mod_api::RenderLimits::default(),
                )
            },
            |gpu| gpu.render_capabilities(),
        )
    }
    /// Free mod-owned offscreen `target` and retire the handle.
    pub(in crate::game) fn render_target_release(
        &self,
        target: mod_api::RenderTargetRef,
    ) -> Result<(), mod_api::RenderTargetError> {
        let gpu = self
            .mod_gpu()
            .ok_or(mod_api::RenderTargetError::Unavailable)?;
        gpu.render_target_release(mod_api::content::crash::current_native_mod(), target)
    }
    /// Validate and resolve per-draw set-1 [`mod_api::DrawBindings`] against pipeline `slot`'s
    /// declared `BindGroupLayout` and `push_constant_size`, refusing a mip level bound in ways one
    /// pass cannot combine with [`PipelineDrawError::TargetReadWriteHazard`].
    fn resolve_draw_bindings(
        &self,
        draw: &mut WorldGeometry,
        slot: u32,
        bindings: &mod_api::DrawBindings<'_>,
    ) -> Result<Option<sunrise_client::gfx::mod_pipelines::ResolvedDrawBindings>, PipelineDrawError>
    {
        let pack_linked = !draw.is_oit_pass() && self.pack_slot_bound(slot as usize);
        let draw = std::cell::RefCell::new(draw);
        self.resolve_draw_bindings_into(
            slot,
            bindings,
            None,
            &[],
            "",
            &[],
            pack_linked,
            true,
            &mut |bytes| draw.borrow_mut().push_inline_uniform(bytes),
            &mut |bytes| draw.borrow_mut().push_texel_data(bytes),
        )
    }

    /// Validate and resolve per-draw set-1 [`mod_api::DrawBindings`] for either the main world
    /// pass (`active_target = None`, `in_world_raster = true`) or an offscreen pass
    /// (`active_target = Some((target, mip_level))`, `in_world_raster = false`: a mod render
    /// target's mip level or a transient frame-graph target at level 0), refusing self-sampling
    /// with [`PipelineDrawError::TargetReadWriteHazard`].
    #[allow(clippy::too_many_arguments)]
    fn resolve_draw_bindings_into(
        &self,
        slot: u32,
        bindings: &mod_api::DrawBindings<'_>,
        active_target: Option<(mod_api::FrameTarget, u32)>,
        transients: &[mod_api::RenderTargetDescriptor],
        pass_label: &str,
        reads: &[mod_api::FrameTarget],
        pack_linked: bool,
        in_world_raster: bool,
        push_inline: &mut dyn FnMut(&[u8]) -> std::ops::Range<usize>,
        push_texel: &mut dyn FnMut(&[u8]) -> std::ops::Range<usize>,
    ) -> Result<Option<sunrise_client::gfx::mod_pipelines::ResolvedDrawBindings>, PipelineDrawError>
    {
        let (layout, pc_size) = self
            .client
            .client_pack
            .render
            .mod_pipelines
            .bind_group_layout(slot)
            .ok_or(PipelineDrawError::Pipeline(
                mod_api::RenderPipelineError::Invalid,
            ))?;
        let writes_depth = self
            .client
            .client_pack
            .render
            .mod_pipelines
            .writes_depth(slot);
        self.resolve_bindings_against(
            layout,
            pc_size,
            writes_depth,
            bindings,
            active_target,
            transients,
            pass_label,
            reads,
            pack_linked,
            in_world_raster,
            push_inline,
            push_texel,
        )
    }

    /// [`Self::resolve_draw_bindings_into`] for any pipeline kind: `layout` is the pipeline's
    /// declared bind group, `pc_size` its push-constant bytes (0 for compute) and `writes_depth`
    /// whether it writes depth (a depth-sampling hazard). Transient samples must be among
    /// `reads`, the declared reads of `pass_label`; see [`PipelineDrawError::InvalidBinding`].
    #[allow(clippy::too_many_arguments)]
    pub(in crate::game) fn resolve_bindings_against(
        &self,
        layout: &mod_api::BindGroupLayout,
        pc_size: u32,
        writes_depth: bool,
        bindings: &mod_api::DrawBindings<'_>,
        active_target: Option<(mod_api::FrameTarget, u32)>,
        transients: &[mod_api::RenderTargetDescriptor],
        pass_label: &str,
        reads: &[mod_api::FrameTarget],
        pack_linked: bool,
        in_world_raster: bool,
        push_inline: &mut dyn FnMut(&[u8]) -> std::ops::Range<usize>,
        push_texel: &mut dyn FnMut(&[u8]) -> std::ops::Range<usize>,
    ) -> Result<Option<sunrise_client::gfx::mod_pipelines::ResolvedDrawBindings>, PipelineDrawError>
    {
        use sunrise_client::gfx::mod_pipelines::SamplerLayout;
        use sunrise_client::gfx::mod_render_targets::{wgpu_color_format, wgpu_depth_format};

        if layout.is_empty() && pc_size == 0 {
            if !bindings.entries.is_empty() || bindings.push_constants.is_some() {
                return Err(PipelineDrawError::InvalidBinding);
            }
            return Ok(None);
        }
        // Reject duplicate or unknown entry names in `bindings`.
        if bindings.entries.len() != layout.entries.len() {
            return Err(PipelineDrawError::InvalidBinding);
        }
        let gpu = self
            .mod_gpu()
            .ok_or(PipelineDrawError::Buffer(mod_api::BufferError::Unavailable))?;
        let owner = mod_api::content::crash::current_native_mod();
        let (max_uniform, max_texel) = gpu.mod_binding_limits();
        // A draw of an offscreen pass binds nothing of the level that pass renders into.
        if let Some((mod_api::FrameTarget::Mod(rt), mip)) = active_target
            && bindings.entries.iter().any(|(_, value)| match value {
                mod_api::BindingValue::Texture(mod_api::TextureSource::RenderTargetColor(t, _))
                | mod_api::BindingValue::Texture(mod_api::TextureSource::RenderTargetDepth(t)) => {
                    *t == rt
                }
                mod_api::BindingValue::Texture(mod_api::TextureSource::RenderTargetColorMip(
                    view,
                )) => {
                    view.target == rt
                        && (view.base_mip..view.base_mip.saturating_add(view.mip_levels))
                            .contains(&mip)
                }
                mod_api::BindingValue::StorageImage(image) => {
                    image.target == rt && image.mip_level == mip
                }
                _ => false,
            })
        {
            return Err(PipelineDrawError::TargetReadWriteHazard);
        }
        let resolve_transient = |id: mod_api::TransientTargetId| {
            let target = mod_api::FrameTarget::Transient(id);
            if active_target.is_some_and(|(active, _)| active == target) {
                return Err(PipelineDrawError::TargetReadWriteHazard);
            }
            let desc = transients
                .get(id.raw() as usize)
                .ok_or(PipelineDrawError::InvalidBinding)?;
            if !reads.contains(&target) {
                gpu.warn_undeclared_transient_read_once(pass_label, id);
                return Err(PipelineDrawError::InvalidBinding);
            }
            Ok(desc)
        };

        // Validate every declared entry before appending to byte pools so a refused draw appends
        // nothing.
        enum StagedEntry<'a> {
            UniformBuffer {
                id: u64,
                buffer: wgpu::Buffer,
                byte_len: u64,
            },
            InlineUniform(&'a [u8]),
            Texture {
                view_id: u64,
                view: wgpu::TextureView,
                mips: Option<sunrise_client::gfx::mod_render_targets::TargetMips>,
            },
            TransientColor {
                target: mod_api::TransientTargetId,
                attachment: u8,
            },
            TransientDepth {
                target: mod_api::TransientTargetId,
            },
            TexelBytes(Vec<u8>),
            Storage(sunrise_client::gfx::mod_storage::ResolvedStorageRange),
            StorageImage {
                view_id: u64,
                view: wgpu::TextureView,
                format: wgpu::TextureFormat,
                mips: sunrise_client::gfx::mod_render_targets::TargetMips,
            },
        }
        let float32_filterable = gpu.float32_filterable();
        let read_only_depth_stencil = gpu.read_only_depth_stencil();
        let mut reads_world_depth = false;
        let mut sampler_layout = SamplerLayout::default();
        let mut sampled = 0;
        let mut staged = Vec::with_capacity(layout.entries.len());
        for decl in &layout.entries {
            let mut matches = bindings
                .entries
                .iter()
                .filter(|(name, _)| *name == decl.name.as_str());
            let Some((_, value)) = matches.next() else {
                return Err(PipelineDrawError::InvalidBinding);
            };
            if matches.next().is_some() {
                return Err(PipelineDrawError::InvalidBinding);
            }
            match (decl.kind, value) {
                (
                    mod_api::BindingKind::UniformBuffer { min_size },
                    mod_api::BindingValue::Uniform(mod_api::UniformValue::Buffer(handle)),
                ) => {
                    let (id, buffer, byte_len) =
                        gpu.resolve_mod_uniform_buffer(owner, *handle, min_size)?;
                    staged.push(StagedEntry::UniformBuffer {
                        id,
                        buffer,
                        byte_len,
                    });
                }
                (
                    mod_api::BindingKind::UniformBuffer { min_size },
                    mod_api::BindingValue::Uniform(mod_api::UniformValue::Inline(bytes)),
                ) => {
                    if bytes.len() < min_size as usize
                        || !bytes.len().is_multiple_of(4)
                        || bytes.len() as u64 > max_uniform
                    {
                        return Err(PipelineDrawError::InvalidBinding);
                    }
                    staged.push(StagedEntry::InlineUniform(bytes));
                }
                (
                    mod_api::BindingKind::CombinedImageSampler { sampler },
                    mod_api::BindingValue::Texture(source),
                ) => {
                    let (entry, format) = match source {
                        mod_api::TextureSource::TransientTargetColor(target, attachment) => {
                            let format = resolve_transient(*target)?
                                .color_targets
                                .get(*attachment as usize)
                                .ok_or(PipelineDrawError::InvalidBinding)?;
                            (
                                StagedEntry::TransientColor {
                                    target: *target,
                                    attachment: *attachment,
                                },
                                wgpu_color_format(*format),
                            )
                        }
                        mod_api::TextureSource::TransientTargetDepth(target) => {
                            let format = wgpu_depth_format(resolve_transient(*target)?.depth)
                                .ok_or(PipelineDrawError::InvalidBinding)?;
                            (StagedEntry::TransientDepth { target: *target }, format)
                        }
                        other => {
                            let mut mips = None;
                            let (view_id, view) = match other {
                                mod_api::TextureSource::AtlasSprite(id) => {
                                    let sprite_key = id
                                        .to_string()
                                        .strip_prefix("minecraft:")
                                        .map(str::to_owned)
                                        .unwrap_or_else(|| id.to_string());
                                    if !self
                                        .client
                                        .client_pack
                                        .render
                                        .sprite_layers
                                        .contains_key(&sprite_key)
                                        && !self.client.gui.atlas.sprites.contains_key(&sprite_key)
                                    {
                                        return Err(PipelineDrawError::InvalidBinding);
                                    }
                                    gpu.mod_atlas_texture_view()
                                }
                                mod_api::TextureSource::DynamicTexture(id) => {
                                    self.resolve_dynamic_texture_view(gpu, id)?
                                }
                                mod_api::TextureSource::StandaloneTexture(id) => {
                                    let asset_path = if id.path().ends_with(".png") {
                                        format!("assets/{}/{}", id.namespace(), id.path())
                                    } else {
                                        format!(
                                            "assets/{}/textures/{}.png",
                                            id.namespace(),
                                            id.path()
                                        )
                                    };
                                    gpu.resolve_mod_standalone_texture(id, || {
                                        self.common.pack.source.read(&asset_path).ok()
                                    })?
                                }
                                mod_api::TextureSource::WorldDepth => {
                                    check_world_depth_binding(
                                        writes_depth,
                                        in_world_raster,
                                        read_only_depth_stencil,
                                    )?;
                                    reads_world_depth = true;
                                    gpu.mod_world_depth_texture_view()
                                }
                                mod_api::TextureSource::Lightmap => gpu.mod_lightmap_texture_view(),
                                mod_api::TextureSource::RenderTargetColor(target, attachment) => {
                                    let (id, view, target_mips) = gpu
                                        .resolve_mod_render_target_color(
                                            owner,
                                            *target,
                                            *attachment,
                                        )?;
                                    mips = Some(target_mips);
                                    (id, view)
                                }
                                mod_api::TextureSource::RenderTargetDepth(target) => {
                                    gpu.resolve_mod_render_target_depth(owner, *target)?
                                }
                                mod_api::TextureSource::RenderTargetColorMip(view) => {
                                    let (id, view, target_mips) =
                                        gpu.resolve_mod_render_target_color_mips(owner, *view)?;
                                    mips = Some(target_mips);
                                    (id, view)
                                }
                                _ => return Err(PipelineDrawError::InvalidBinding),
                            };
                            let format = view.texture().format();
                            (
                                StagedEntry::Texture {
                                    view_id,
                                    view,
                                    mips,
                                },
                                format,
                            )
                        }
                    };
                    sampler_layout.record(sampled, &sampler, format, float32_filterable);
                    sampled += 1;
                    staged.push(entry);
                }
                (
                    mod_api::BindingKind::TexelBuffer { format },
                    mod_api::BindingValue::TexelBuffer(bytes),
                ) => {
                    let expanded_len = bytes.len() as u64 * (4 / format.element_bytes() as u64);
                    if bytes.is_empty()
                        || !bytes.len().is_multiple_of(format.element_bytes())
                        || expanded_len > max_texel
                    {
                        return Err(PipelineDrawError::InvalidBinding);
                    }
                    let expanded = match format {
                        mod_api::TexelBufferFormat::R8Sint => {
                            let mut out = Vec::with_capacity(bytes.len() * 4);
                            for &b in *bytes {
                                out.extend_from_slice(&i32::from(b as i8).to_ne_bytes());
                            }
                            out
                        }
                        mod_api::TexelBufferFormat::R8Uint => {
                            let mut out = Vec::with_capacity(bytes.len() * 4);
                            for &b in *bytes {
                                out.extend_from_slice(&u32::from(b).to_ne_bytes());
                            }
                            out
                        }
                        mod_api::TexelBufferFormat::R32Sint
                        | mod_api::TexelBufferFormat::R32Uint => bytes.to_vec(),
                        _ => return Err(PipelineDrawError::InvalidBinding),
                    };
                    staged.push(StagedEntry::TexelBytes(expanded));
                }
                (
                    mod_api::BindingKind::StorageBuffer { min_size, .. },
                    mod_api::BindingValue::StorageBuffer(slice),
                ) => {
                    let range = gpu
                        .resolve_mod_storage_range(owner, *slice, min_size)
                        .map_err(PipelineDrawError::Buffer)?;
                    staged.push(StagedEntry::Storage(range));
                }
                (
                    mod_api::BindingKind::StorageImage { access },
                    mod_api::BindingValue::StorageImage(image),
                ) => {
                    let (view_id, view, format, mips) =
                        gpu.resolve_mod_storage_image(owner, access, *image)?;
                    staged.push(StagedEntry::StorageImage {
                        view_id,
                        view,
                        format,
                        mips,
                    });
                }
                _ => return Err(PipelineDrawError::InvalidBinding),
            }
        }
        // A translated pack program owns its pipeline layout, unlike direct mod draws into
        // pack targets. Refuse only that route and only if the bindings require another layout.
        if pack_linked && !sampler_layout.is_empty() {
            return Err(PipelineDrawError::PackSamplerLayout);
        }
        // One render pass gives a mip level one usage: a draw binds no level both as a storage
        // image it writes and as anything else (a sampled view, or another storage access).
        let mips_of = |entry: &StagedEntry<'_>| match entry {
            StagedEntry::Texture { mips, .. } => *mips,
            StagedEntry::StorageImage { mips, .. } => Some(*mips),
            _ => None,
        };
        if staged.iter().enumerate().any(|(i, a)| {
            mips_of(a).is_some_and(|a| {
                staged[i + 1..]
                    .iter()
                    .filter_map(mips_of)
                    .any(|b| a.conflicts_with(&b))
            })
        }) {
            return Err(PipelineDrawError::TargetReadWriteHazard);
        }
        let pc_slice = if pc_size > 0 {
            let Some(bytes) = bindings.push_constants else {
                return Err(PipelineDrawError::InvalidBinding);
            };
            if bytes.is_empty() || bytes.len() > pc_size as usize || !bytes.len().is_multiple_of(4)
            {
                return Err(PipelineDrawError::InvalidBinding);
            }
            Some(bytes)
        } else {
            if bindings.push_constants.is_some() {
                return Err(PipelineDrawError::InvalidBinding);
            }
            None
        };

        let mut resolved_entries = Vec::with_capacity(staged.len());
        for entry in staged {
            match entry {
                StagedEntry::UniformBuffer {
                    id,
                    buffer,
                    byte_len,
                } => {
                    resolved_entries.push(
                        sunrise_client::gfx::mod_pipelines::ResolvedSet1Entry::UniformBuffer {
                            id,
                            buffer,
                            byte_len,
                        },
                    );
                }
                StagedEntry::InlineUniform(bytes) => {
                    let byte_range = push_inline(bytes);
                    resolved_entries.push(
                        sunrise_client::gfx::mod_pipelines::ResolvedSet1Entry::InlineUniform {
                            byte_range,
                        },
                    );
                }
                StagedEntry::Texture {
                    view_id,
                    view,
                    mips,
                } => {
                    resolved_entries.push(
                        sunrise_client::gfx::mod_pipelines::ResolvedSet1Entry::Texture {
                            view_id,
                            view,
                            mips,
                        },
                    );
                }
                StagedEntry::Storage(range) => {
                    resolved_entries.push(
                        sunrise_client::gfx::mod_pipelines::ResolvedSet1Entry::StorageBuffer {
                            id: range.id,
                            buffer: range.buffer,
                            offset: range.offset,
                            size: range.length,
                        },
                    );
                }
                StagedEntry::StorageImage {
                    view_id,
                    view,
                    format,
                    mips,
                } => {
                    resolved_entries.push(
                        sunrise_client::gfx::mod_pipelines::ResolvedSet1Entry::StorageImage {
                            view_id,
                            view,
                            format,
                            mips,
                        },
                    );
                }
                StagedEntry::TransientColor { target, attachment } => {
                    resolved_entries.push(
                        sunrise_client::gfx::mod_pipelines::ResolvedSet1Entry::TransientColor {
                            target,
                            attachment,
                        },
                    );
                }
                StagedEntry::TransientDepth { target } => {
                    resolved_entries.push(
                        sunrise_client::gfx::mod_pipelines::ResolvedSet1Entry::TransientDepth {
                            target,
                        },
                    );
                }
                StagedEntry::TexelBytes(bytes) => {
                    let byte_range = push_texel(&bytes);
                    resolved_entries.push(
                        sunrise_client::gfx::mod_pipelines::ResolvedSet1Entry::TexelBuffer {
                            byte_range,
                        },
                    );
                }
            }
        }
        let push_constants = pc_slice.map(push_inline);
        Ok(Some(
            sunrise_client::gfx::mod_pipelines::ResolvedDrawBindings {
                sampler_layout,
                entries: resolved_entries,
                push_constants,
                reads_world_depth,
            },
        ))
    }

    fn resolve_dynamic_texture_view(
        &self,
        gpu: &sunrise_client::gfx::ModGpuResources,
        id: &mod_api::Identifier,
    ) -> Result<(u64, wgpu::TextureView), mod_api::PipelineDrawError> {
        let Some((handle, _)) = self.client.dynamic_textures.get(id) else {
            return Err(mod_api::PipelineDrawError::InvalidBinding);
        };
        gpu.dynamic_texture_view(handle)
            .ok_or(mod_api::PipelineDrawError::InvalidBinding)
    }

    /// Submit quads drawn by a mod pipeline: shorthand over [`Self::native_world_draw_vertices`]
    /// for pipelines using the default `PositionTexColor` vertex format and
    /// [`mod_api::PrimitiveTopology::Quads`]. Non-finite quads are skipped individually without
    /// rejecting valid quads in the same slice.
    pub(in crate::game) fn native_world_draw_pipeline_quads(
        &self,
        draw: &mut WorldGeometry,
        pipeline: RenderPipelineRef,
        quads: &[PipelineQuad],
    ) -> Result<(), PipelineDrawError> {
        let slot = self
            .client
            .client_pack
            .render
            .mod_pipelines
            .slot(pipeline)
            .map_err(PipelineDrawError::Pipeline)?;
        let base_topology = self
            .client
            .client_pack
            .render
            .mod_pipelines
            .topology(slot)
            .unwrap_or(mod_api::PrimitiveTopology::Quads);
        if draw.effective_topology(base_topology) != mod_api::PrimitiveTopology::Quads {
            return Err(PipelineDrawError::TopologyMismatch);
        }
        if !self
            .client
            .client_pack
            .render
            .mod_pipelines
            .uses_default_vertex_format(slot)
        {
            return Err(PipelineDrawError::VertexFormatMismatch);
        }
        let camera = draw.camera();
        let mut vertices = Vec::with_capacity(quads.len() * 4);
        for q in quads {
            let corners_finite = q.corners.iter().all(|corner| {
                corner
                    .iter()
                    .zip(camera)
                    .all(|(&p, cam)| p.is_finite() && ((p - cam) as f32).is_finite())
            });
            if !corners_finite || q.uv.iter().flatten().any(|c| !c.is_finite()) {
                continue;
            }
            let color = sunrise_client::world_primitives::vertex_color(q.color, 255);
            vertices.extend((0..4).map(|corner| {
                sunrise_client::gfx::mod_pipelines::PipelineVertex {
                    pos: std::array::from_fn(|i| (q.corners[corner][i] - camera[i]) as f32),
                    uv: q.uv[corner],
                    color,
                }
            }));
        }
        if vertices.is_empty() {
            return Ok(());
        }
        self.native_world_draw_vertices(
            draw,
            pipeline,
            &[bytemuck::cast_slice(&vertices)],
            None,
            mod_api::MeshDraw::vertices(0..vertices.len() as u32),
            &mod_api::DrawBindings::EMPTY,
        )
    }
    /// Record a zero-copy retained GPU [`mod_api::MeshRef`] draw call after validating pipeline
    /// liveness, mesh ownership/liveness, vertex-binding strides, and draw/instance bounds.
    pub(in crate::game) fn native_world_draw_mesh(
        &self,
        draw: &mut WorldGeometry,
        pipeline: RenderPipelineRef,
        mesh: mod_api::MeshRef,
        mesh_draw: mod_api::MeshDraw,
        bindings: &mod_api::DrawBindings<'_>,
    ) -> Result<(), PipelineDrawError> {
        let slot = self
            .client
            .client_pack
            .render
            .mod_pipelines
            .slot(pipeline)
            .map_err(PipelineDrawError::Pipeline)?;
        let vertex_bindings = self
            .client
            .client_pack
            .render
            .mod_pipelines
            .vertex_bindings(slot)
            .ok_or(PipelineDrawError::Pipeline(
                mod_api::RenderPipelineError::Invalid,
            ))?;
        let base_topology = self
            .client
            .client_pack
            .render
            .mod_pipelines
            .topology(slot)
            .unwrap_or(mod_api::PrimitiveTopology::Quads);
        let topology = draw.effective_topology(base_topology);
        let gpu = self
            .mod_gpu()
            .ok_or(PipelineDrawError::Mesh(mod_api::MeshError::Unavailable))?;
        check_generated_indices(topology, &mesh_draw, gpu.max_buffer_size())?;
        let resolved = gpu.resolve_mod_mesh_draw(
            mod_api::content::crash::current_native_mod(),
            mesh,
            vertex_bindings,
            topology,
            &mesh_draw,
        )?;
        let resolved_bindings = self.resolve_draw_bindings(draw, slot, bindings)?;
        let empty = mesh_draw.instances.is_empty()
            || match &mesh_draw.range {
                mod_api::DrawRange::Vertices(r) => r.is_empty(),
                mod_api::DrawRange::Indexed { indices, .. } => indices.is_empty(),
                _ => true,
            };
        if !empty {
            draw.draw_mesh_slot(slot, resolved, mesh_draw, resolved_bindings);
        }
        Ok(())
    }
    /// Check a GPU-driven [`mod_api::IndirectDraw`] against its pipeline, mesh and command
    /// buffer, before anything is recorded: the pipeline's vertex bindings against the mesh (a
    /// pipeline with bindings needs a mesh), the command buffer's handle, and the rest in
    /// `IndirectSource::new`.
    fn resolve_indirect_draw(
        &self,
        slot: u32,
        topology: mod_api::PrimitiveTopology,
        draw: &mod_api::IndirectDraw<'_>,
    ) -> Result<sunrise_client::gfx::mod_pipelines::IndirectSource, PipelineDrawError> {
        let vertex_bindings = self
            .client
            .client_pack
            .render
            .mod_pipelines
            .vertex_bindings(slot)
            .ok_or(PipelineDrawError::Pipeline(
                mod_api::RenderPipelineError::Invalid,
            ))?;
        let gpu = self
            .mod_gpu()
            .ok_or(PipelineDrawError::Mesh(mod_api::MeshError::Unavailable))?;
        let owner = mod_api::content::crash::current_native_mod();
        let mesh = match draw.mesh {
            Some(handle) => Some(gpu.resolve_mod_mesh_indirect(owner, handle, vertex_bindings)?),
            None if vertex_bindings.is_empty() => None,
            None => return Err(PipelineDrawError::VertexFormatMismatch),
        };
        let buffer = gpu
            .resolve_mod_storage_buffer(owner, draw.indirect_buffer)
            .map_err(PipelineDrawError::Buffer)?;
        let count = match draw.count_buffer {
            Some((handle, offset)) => Some((
                gpu.resolve_mod_storage_buffer(owner, handle)
                    .map_err(PipelineDrawError::Buffer)?,
                offset,
            )),
            None => None,
        };
        sunrise_client::gfx::mod_pipelines::IndirectSource::new(
            topology,
            mesh,
            buffer,
            draw.indirect_offset,
            draw.max_count,
            count,
            gpu.multi_draw_indirect_count(),
        )
    }
    /// Record a GPU-driven [`mod_api::IndirectDraw`] for the world pass, after checking it.
    pub(in crate::game) fn native_world_draw_indirect(
        &self,
        draw: &mut WorldGeometry,
        indirect: &mod_api::IndirectDraw<'_>,
    ) -> Result<(), PipelineDrawError> {
        let slot = self
            .client
            .client_pack
            .render
            .mod_pipelines
            .slot(indirect.pipeline)
            .map_err(PipelineDrawError::Pipeline)?;
        let base_topology = self
            .client
            .client_pack
            .render
            .mod_pipelines
            .topology(slot)
            .unwrap_or(mod_api::PrimitiveTopology::Quads);
        let source =
            self.resolve_indirect_draw(slot, draw.effective_topology(base_topology), indirect)?;
        let resolved_bindings = self.resolve_draw_bindings(draw, slot, indirect.bindings)?;
        if indirect.max_count > 0 {
            draw.draw_indirect_slot(slot, source, resolved_bindings);
        }
        Ok(())
    }
    /// Validate and queue a transient per-frame vertex/index draw call.
    pub(in crate::game) fn native_world_draw_vertices(
        &self,
        draw: &mut WorldGeometry,
        pipeline: RenderPipelineRef,
        vertices: &[&[u8]],
        indices: Option<(mod_api::IndexFormat, &[u8])>,
        mesh_draw: mod_api::MeshDraw,
        bindings: &mod_api::DrawBindings<'_>,
    ) -> Result<(), PipelineDrawError> {
        let slot = self
            .client
            .client_pack
            .render
            .mod_pipelines
            .slot(pipeline)
            .map_err(PipelineDrawError::Pipeline)?;
        let vertex_bindings = self
            .client
            .client_pack
            .render
            .mod_pipelines
            .vertex_bindings(slot)
            .ok_or(PipelineDrawError::Pipeline(
                mod_api::RenderPipelineError::Invalid,
            ))?;
        let base_topology = self
            .client
            .client_pack
            .render
            .mod_pipelines
            .topology(slot)
            .unwrap_or(mod_api::PrimitiveTopology::Quads);
        let topology = draw.effective_topology(base_topology);
        if mesh_draw.instances.start > mesh_draw.instances.end {
            return Err(PipelineDrawError::InvalidRange);
        }
        if vertices.len() != vertex_bindings.len() {
            return Err(PipelineDrawError::VertexFormatMismatch);
        }
        let mut bound_slices = Vec::with_capacity(vertices.len());
        let mut vertex_records = Vec::with_capacity(vertices.len());
        for ((binding_idx, fmt), &slice) in vertex_bindings.iter().zip(vertices.iter()) {
            let stride = fmt.stride as usize;
            if slice.len() % 4 != 0 || slice.len() % stride != 0 {
                return Err(PipelineDrawError::InvalidAlignment);
            }
            let record_count = slice.len() / stride;
            match fmt.step_mode {
                mod_api::VertexStepMode::Vertex => {
                    if let mod_api::DrawRange::Vertices(ref range) = mesh_draw.range
                        && (range.start > range.end || (range.end as usize) > record_count)
                    {
                        return Err(PipelineDrawError::InvalidRange);
                    }
                }
                mod_api::VertexStepMode::Instance => {
                    if (mesh_draw.instances.end as usize) > record_count {
                        return Err(PipelineDrawError::InvalidRange);
                    }
                }
                _ => return Err(PipelineDrawError::VertexFormatMismatch),
            }
            vertex_records.push((fmt.step_mode, record_count));
            bound_slices.push((u32::from(*binding_idx), slice));
        }
        check_generated_indices(
            topology,
            &mesh_draw,
            self.mod_gpu().map_or(u64::MAX, |gpu| gpu.max_buffer_size()),
        )?;
        match &mesh_draw.range {
            mod_api::DrawRange::Vertices(range) => {
                if range.start > range.end {
                    return Err(PipelineDrawError::InvalidRange);
                }
                let count = range.end - range.start;
                if matches!(
                    topology,
                    mod_api::PrimitiveTopology::Quads | mod_api::PrimitiveTopology::Lines
                ) && count % 4 != 0
                {
                    return Err(PipelineDrawError::InvalidRange);
                }
            }
            mod_api::DrawRange::Indexed {
                indices: idx_range,
                base_vertex,
            } => {
                let Some((fmt, idx_bytes)) = indices else {
                    return Err(PipelineDrawError::MissingIndexBuffer);
                };
                let elem = fmt.byte_size() as usize;
                if idx_bytes.len() % elem != 0 || idx_bytes.len() % 4 != 0 {
                    return Err(PipelineDrawError::InvalidAlignment);
                }
                let idx_count = idx_bytes.len() / elem;
                if idx_range.start > idx_range.end || (idx_range.end as usize) > idx_count {
                    return Err(PipelineDrawError::InvalidRange);
                }
                check_indexed_vertices(&vertex_records, fmt, idx_bytes, idx_range, *base_vertex)?;
            }
            _ => return Err(PipelineDrawError::InvalidRange),
        }
        let resolved_bindings = self.resolve_draw_bindings(draw, slot, bindings)?;
        let empty = mesh_draw.instances.is_empty()
            || match &mesh_draw.range {
                mod_api::DrawRange::Vertices(r) => r.is_empty(),
                mod_api::DrawRange::Indexed { indices, .. } => indices.is_empty(),
                _ => true,
            };
        if !empty {
            draw.draw_vertices_slot(slot, &bound_slices, indices, mesh_draw, resolved_bindings);
        }
        Ok(())
    }
    /// Record one compute dispatch into a frame-graph compute node: the handle, the bindings
    /// against the pipeline's declared bind group and the workgroup counts (or the indirect
    /// buffer range) are all checked here, before any GPU call, so a bad dispatch is an `Err`.
    pub(in crate::game) fn native_compute_dispatch(
        &self,
        pass: &mut sunrise_client::gfx::mod_frame_graph::RecordedFramePass,
        transients: &[mod_api::RenderTargetDescriptor],
        pipeline: mod_api::ComputePipelineRef,
        bindings: &mod_api::DrawBindings<'_>,
        workgroups: Result<[u32; 3], (mod_api::StorageBufferRef, u64)>,
    ) -> Result<(), mod_api::ComputeDispatchError> {
        use mod_api::ComputeDispatchError as Refused;
        use sunrise_client::gfx::mod_compute::{DispatchKind, RecordedDispatch};
        use sunrise_client::gfx::mod_pipelines::ResolvedDrawBindings;
        let dispatches = pass.dispatches.as_mut().ok_or(Refused::Unavailable)?;
        let gpu = self.mod_gpu().ok_or(Refused::Unavailable)?;
        let compute = self.client.client_pack.render.mod_pipelines.compute();
        let slot = compute.slot(pipeline).map_err(Refused::Pipeline)?;
        let built = compute.built()[slot as usize]
            .as_ref()
            .ok_or(Refused::Pipeline(mod_api::ComputePipelineError::Invalid))?;
        let kind = match workgroups {
            Ok(counts) => {
                let max = gpu.max_compute_workgroups_per_dimension();
                if counts.iter().any(|&n| n > max) {
                    return Err(Refused::TooManyWorkgroups);
                }
                DispatchKind::Direct(counts)
            }
            Err((indirect, offset)) => {
                let buffer = gpu
                    .resolve_mod_storage_buffer(
                        mod_api::content::crash::current_native_mod(),
                        indirect,
                    )
                    .map_err(Refused::Buffer)?;
                if !buffer.usage().contains(wgpu::BufferUsages::INDIRECT)
                    || !offset.is_multiple_of(4)
                    || offset.checked_add(12).is_none_or(|end| end > buffer.size())
                {
                    return Err(Refused::InvalidIndirect);
                }
                DispatchKind::Indirect { buffer, offset }
            }
        };
        let mut inline = Vec::new();
        let mut texel = Vec::new();
        let resolved = self
            .resolve_bindings_against(
                &built.bind_group_layout,
                0,
                false,
                bindings,
                None,
                transients,
                &pass.label,
                &pass.reads,
                false,
                false,
                &mut |bytes| {
                    let start = inline.len();
                    inline.extend_from_slice(bytes);
                    start..inline.len()
                },
                &mut |bytes| {
                    let start = texel.len();
                    texel.extend_from_slice(bytes);
                    start..texel.len()
                },
            )
            .map_err(|error| match error {
                PipelineDrawError::Buffer(error) => Refused::Buffer(error),
                _ => Refused::InvalidBinding,
            })?;
        let indirect = workgroups.err().map(|(buffer, _)| buffer);
        if compute_bindings_alias(&built.bind_group_layout, bindings, indirect) {
            return Err(Refused::InvalidBinding);
        }
        // A storage image's texture format is the one its shader declares.
        let ResolvedDrawBindings {
            sampler_layout,
            entries,
            ..
        } = resolved.unwrap_or_default();
        for (index, entry) in entries.iter().enumerate() {
            if let sunrise_client::gfx::mod_pipelines::ResolvedSet1Entry::StorageImage {
                format,
                ..
            } = entry
                && sunrise_client::gfx::mod_compute::declared_storage_format(built, index)
                    != Some(*format)
            {
                return Err(Refused::InvalidBinding);
            }
        }
        dispatches.push(RecordedDispatch {
            slot,
            sampler_layout,
            entries,
            inline,
            texel,
            kind,
        });
        gpu.note_compute_dispatch();
        Ok(())
    }

    /// Record a retained GPU [`mod_api::MeshRef`] draw into an offscreen frame-graph pass.
    pub(in crate::game) fn native_offscreen_draw_mesh(
        &self,
        pass: &mut sunrise_client::gfx::mod_frame_graph::RecordedFramePass,
        transients: &[mod_api::RenderTargetDescriptor],
        pipeline: RenderPipelineRef,
        mesh: mod_api::MeshRef,
        mesh_draw: mod_api::MeshDraw,
        bindings: &mod_api::DrawBindings<'_>,
    ) -> Result<(), PipelineDrawError> {
        let slot = self
            .client
            .client_pack
            .render
            .mod_pipelines
            .slot(pipeline)
            .map_err(PipelineDrawError::Pipeline)?;
        let vertex_bindings = self
            .client
            .client_pack
            .render
            .mod_pipelines
            .vertex_bindings(slot)
            .ok_or(PipelineDrawError::Pipeline(
                mod_api::RenderPipelineError::Invalid,
            ))?;
        let topology = self
            .client
            .client_pack
            .render
            .mod_pipelines
            .topology(slot)
            .unwrap_or(mod_api::PrimitiveTopology::Quads);
        let gpu = self
            .mod_gpu()
            .ok_or(PipelineDrawError::Mesh(mod_api::MeshError::Unavailable))?;
        let resolved = gpu.resolve_mod_mesh_draw(
            mod_api::content::crash::current_native_mod(),
            mesh,
            vertex_bindings,
            topology,
            &mesh_draw,
        )?;
        let active_target = pass.target.map(|target| (target, 0));
        let resolved_bindings = self.resolve_draw_bindings_into(
            slot,
            bindings,
            active_target,
            transients,
            &pass.label,
            &pass.reads,
            false,
            false,
            &mut |bytes| {
                let start = pass.inline_uniforms.len();
                pass.inline_uniforms.extend_from_slice(bytes);
                start..pass.inline_uniforms.len()
            },
            &mut |bytes| {
                let start = pass.texel_data.len();
                pass.texel_data.extend_from_slice(bytes);
                start..pass.texel_data.len()
            },
        )?;
        let empty = mesh_draw.instances.is_empty()
            || match &mesh_draw.range {
                mod_api::DrawRange::Vertices(r) => r.is_empty(),
                mod_api::DrawRange::Indexed { indices, .. } => indices.is_empty(),
                _ => true,
            };
        if !empty {
            pass.draws
                .push(sunrise_client::gfx::mod_pipelines::PipelineDraw {
                    slot,
                    stage: 0,
                    modifier_stack: 0,
                    draw: mesh_draw,
                    source: sunrise_client::gfx::mod_pipelines::PipelineDrawSource::Retained(
                        resolved,
                    ),
                    bindings: resolved_bindings,
                });
        }
        Ok(())
    }
    /// Record a GPU-driven [`mod_api::IndirectDraw`] into an offscreen frame-graph pass, after
    /// checking it.
    pub(in crate::game) fn native_offscreen_draw_indirect(
        &self,
        pass: &mut sunrise_client::gfx::mod_frame_graph::RecordedFramePass,
        transients: &[mod_api::RenderTargetDescriptor],
        indirect: &mod_api::IndirectDraw<'_>,
    ) -> Result<(), PipelineDrawError> {
        let slot = self
            .client
            .client_pack
            .render
            .mod_pipelines
            .slot(indirect.pipeline)
            .map_err(PipelineDrawError::Pipeline)?;
        let topology = self
            .client
            .client_pack
            .render
            .mod_pipelines
            .topology(slot)
            .unwrap_or(mod_api::PrimitiveTopology::Quads);
        let source = self.resolve_indirect_draw(slot, topology, indirect)?;
        let active_target = pass.target.map(|target| (target, 0));
        let resolved_bindings = self.resolve_draw_bindings_into(
            slot,
            indirect.bindings,
            active_target,
            transients,
            &pass.label,
            &pass.reads,
            false,
            false,
            &mut |bytes| {
                let start = pass.inline_uniforms.len();
                pass.inline_uniforms.extend_from_slice(bytes);
                start..pass.inline_uniforms.len()
            },
            &mut |bytes| {
                let start = pass.texel_data.len();
                pass.texel_data.extend_from_slice(bytes);
                start..pass.texel_data.len()
            },
        )?;
        if indirect.max_count > 0 {
            pass.draws
                .push(sunrise_client::gfx::mod_pipelines::PipelineDraw {
                    slot,
                    stage: 0,
                    modifier_stack: 0,
                    draw: mod_api::MeshDraw::vertices(0..0),
                    source: sunrise_client::gfx::mod_pipelines::PipelineDrawSource::Indirect(
                        source,
                    ),
                    bindings: resolved_bindings,
                });
        }
        Ok(())
    }
    /// Record a transient per-frame vertex/index draw into an offscreen frame-graph pass.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::game) fn native_offscreen_draw_vertices(
        &self,
        pass: &mut sunrise_client::gfx::mod_frame_graph::RecordedFramePass,
        transients: &[mod_api::RenderTargetDescriptor],
        pipeline: RenderPipelineRef,
        vertices: &[&[u8]],
        indices: Option<(mod_api::IndexFormat, &[u8])>,
        mesh_draw: mod_api::MeshDraw,
        bindings: &mod_api::DrawBindings<'_>,
    ) -> Result<(), PipelineDrawError> {
        let slot = self
            .client
            .client_pack
            .render
            .mod_pipelines
            .slot(pipeline)
            .map_err(PipelineDrawError::Pipeline)?;
        let vertex_bindings = self
            .client
            .client_pack
            .render
            .mod_pipelines
            .vertex_bindings(slot)
            .ok_or(PipelineDrawError::Pipeline(
                mod_api::RenderPipelineError::Invalid,
            ))?;
        let topology = self
            .client
            .client_pack
            .render
            .mod_pipelines
            .topology(slot)
            .unwrap_or(mod_api::PrimitiveTopology::Quads);
        if mesh_draw.instances.start > mesh_draw.instances.end {
            return Err(PipelineDrawError::InvalidRange);
        }
        if vertices.len() != vertex_bindings.len() {
            return Err(PipelineDrawError::VertexFormatMismatch);
        }
        let mut bound_slices = Vec::with_capacity(vertices.len());
        for ((binding_idx, fmt), &slice) in vertex_bindings.iter().zip(vertices.iter()) {
            let stride = fmt.stride as usize;
            if slice.len() % 4 != 0 || slice.len() % stride != 0 {
                return Err(PipelineDrawError::InvalidAlignment);
            }
            let record_count = slice.len() / stride;
            match fmt.step_mode {
                mod_api::VertexStepMode::Vertex => {
                    if let mod_api::DrawRange::Vertices(ref range) = mesh_draw.range
                        && (range.start > range.end || (range.end as usize) > record_count)
                    {
                        return Err(PipelineDrawError::InvalidRange);
                    }
                }
                mod_api::VertexStepMode::Instance => {
                    if (mesh_draw.instances.end as usize) > record_count {
                        return Err(PipelineDrawError::InvalidRange);
                    }
                }
                _ => return Err(PipelineDrawError::VertexFormatMismatch),
            }
            bound_slices.push((u32::from(*binding_idx), slice));
        }
        match &mesh_draw.range {
            mod_api::DrawRange::Vertices(range) => {
                if range.start > range.end {
                    return Err(PipelineDrawError::InvalidRange);
                }
                let count = range.end - range.start;
                if matches!(
                    topology,
                    mod_api::PrimitiveTopology::Quads | mod_api::PrimitiveTopology::Lines
                ) && count % 4 != 0
                {
                    return Err(PipelineDrawError::InvalidRange);
                }
            }
            mod_api::DrawRange::Indexed {
                indices: idx_range, ..
            } => {
                let Some((fmt, idx_bytes)) = indices else {
                    return Err(PipelineDrawError::MissingIndexBuffer);
                };
                let elem = fmt.byte_size() as usize;
                if idx_bytes.len() % elem != 0 || idx_bytes.len() % 4 != 0 {
                    return Err(PipelineDrawError::InvalidAlignment);
                }
                let idx_count = idx_bytes.len() / elem;
                if idx_range.start > idx_range.end || (idx_range.end as usize) > idx_count {
                    return Err(PipelineDrawError::InvalidRange);
                }
            }
            _ => return Err(PipelineDrawError::InvalidRange),
        }
        let active_target = pass.target.map(|target| (target, 0));
        let resolved_bindings = self.resolve_draw_bindings_into(
            slot,
            bindings,
            active_target,
            transients,
            &pass.label,
            &pass.reads,
            false,
            false,
            &mut |bytes| {
                let start = pass.inline_uniforms.len();
                pass.inline_uniforms.extend_from_slice(bytes);
                start..pass.inline_uniforms.len()
            },
            &mut |bytes| {
                let start = pass.texel_data.len();
                pass.texel_data.extend_from_slice(bytes);
                start..pass.texel_data.len()
            },
        )?;
        let empty = mesh_draw.instances.is_empty()
            || match &mesh_draw.range {
                mod_api::DrawRange::Vertices(r) => r.is_empty(),
                mod_api::DrawRange::Indexed { indices, .. } => indices.is_empty(),
                _ => true,
            };
        if !empty {
            let mut vertex_ranges = Vec::with_capacity(bound_slices.len());
            for (binding, bytes) in bound_slices {
                let start = pass.transient_vertices.len() as u64;
                pass.transient_vertices.extend_from_slice(bytes);
                let end = pass.transient_vertices.len() as u64;
                vertex_ranges.push((binding, start..end));
            }
            let index_range = indices.map(|(fmt, bytes)| {
                let start = pass.transient_indices.len() as u64;
                pass.transient_indices.extend_from_slice(bytes);
                let end = pass.transient_indices.len() as u64;
                (fmt, start..end)
            });
            pass.draws
                .push(sunrise_client::gfx::mod_pipelines::PipelineDraw {
                    slot,
                    stage: 0,
                    modifier_stack: 0,
                    draw: mesh_draw,
                    source: sunrise_client::gfx::mod_pipelines::PipelineDrawSource::Transient {
                        vertex_ranges,
                        index_range,
                    },
                    bindings: resolved_bindings,
                });
        }
        Ok(())
    }
    /// Shorthand for drawing [`PipelineQuad`]s into an offscreen frame-graph pass using the
    /// default `PositionTexColor` vertex format.
    pub(in crate::game) fn native_offscreen_draw_pipeline_quads(
        &self,
        pass: &mut sunrise_client::gfx::mod_frame_graph::RecordedFramePass,
        transients: &[mod_api::RenderTargetDescriptor],
        camera: [f64; 3],
        pipeline: RenderPipelineRef,
        quads: &[PipelineQuad],
    ) -> Result<(), PipelineDrawError> {
        let slot = self
            .client
            .client_pack
            .render
            .mod_pipelines
            .slot(pipeline)
            .map_err(PipelineDrawError::Pipeline)?;
        let topology = self
            .client
            .client_pack
            .render
            .mod_pipelines
            .topology(slot)
            .unwrap_or(mod_api::PrimitiveTopology::Quads);
        if topology != mod_api::PrimitiveTopology::Quads {
            return Err(PipelineDrawError::TopologyMismatch);
        }
        if !self
            .client
            .client_pack
            .render
            .mod_pipelines
            .uses_default_vertex_format(slot)
        {
            return Err(PipelineDrawError::VertexFormatMismatch);
        }
        let mut vertices = Vec::with_capacity(quads.len() * 4);
        for q in quads {
            let corners_finite = q.corners.iter().all(|corner| {
                corner
                    .iter()
                    .zip(camera)
                    .all(|(&p, cam)| p.is_finite() && ((p - cam) as f32).is_finite())
            });
            if !corners_finite || q.uv.iter().flatten().any(|c| !c.is_finite()) {
                continue;
            }
            let color = sunrise_client::world_primitives::vertex_color(q.color, 255);
            vertices.extend((0..4).map(|corner| {
                sunrise_client::gfx::mod_pipelines::PipelineVertex {
                    pos: std::array::from_fn(|i| (q.corners[corner][i] - camera[i]) as f32),
                    uv: q.uv[corner],
                    color,
                }
            }));
        }
        if vertices.is_empty() {
            return Ok(());
        }
        self.native_offscreen_draw_vertices(
            pass,
            transients,
            pipeline,
            &[bytemuck::cast_slice(&vertices)],
            None,
            mod_api::MeshDraw::vertices(0..vertices.len() as u32),
            &mod_api::DrawBindings::EMPTY,
        )
    }
    /// Resolve `model` once, cull copies against the frustum first, and skip any individual
    /// invalid visible copy without rolling back valid copies.
    pub(in crate::game) fn native_world_draw_model_instances(
        &self,
        draw: &mut WorldGeometry,
        model: ModelRef,
        instances: &[ModelInstance],
    ) -> Result<(), InstanceError> {
        self.draw_cached_model_instances(draw, model, instances, None)
    }
    pub(in crate::game) fn draw_cached_model_instances(
        &self,
        draw: &mut WorldGeometry,
        model: ModelRef,
        instances: &[ModelInstance],
        cached: Option<&[sunrise_client::gfx::PreparedModelInstance]>,
    ) -> Result<(), InstanceError> {
        use mod_api::__private::game::Fields;
        let fields = model.fields();
        if fields.generation != self.client.standalone_geometry.generation() {
            return Err(InstanceError::Model(mod_api::ModelError::StaleModel));
        }
        let slot = self
            .client
            .standalone_geometry
            .instance_slot(fields.slot)
            .ok_or(InstanceError::Model(mod_api::ModelError::UnknownModel))?;
        let uploaded = self.renderer_prepared_generation();
        #[cfg(test)]
        let uploaded = uploaded.or_else(sunrise_client::gfx::test_prepared_generation);
        let gpu = super::native_block_entity_draw::prepared_route(
            self.client.pack_ids.is_some(),
            !self.client.native_shaders.picked.is_empty(),
            uploaded,
            fields.generation,
        );
        let mut staged = self.client.prepared_model_draws.borrow_mut();
        let mut gpu_left = if gpu {
            let room = self
                .prepared_copy_room(staged.len() as u64, instances.len())
                .unwrap_or(0);
            staged.reserve(room);
            room
        } else {
            0
        };
        let cpu_only = slot.translucent || slot.state;
        let cut = draw.model_cut();
        let camera = DVec3::from(draw.camera());
        let mut builder = sunrise_client::gfx::PreparedInstanceBuilder::default();
        let batch = ModelBatch {
            slot: fields.slot,
            generation: fields.generation,
            bounds: slot.bounds,
        };
        let mut stage_copy = |index: usize, copy: &world_geometry::ModelCopy| {
            // The instance route draws one fixed index range per model: a copy that needs
            // other faces, another colour, translucency, a ThroughWalls modifier, or a block state
            // takes the CPU route.
            if copy.through_walls
                || copy.overlay != 0
                || copy.color != u32::MAX
                || copy.hidden_faces != 0
                || cpu_only
                || gpu_left == 0
            {
                return false;
            }
            gpu_left -= 1;
            let instance = if let Some(records) = cached {
                let mut record = records[index];
                let delta = (DVec3::from(copy.origin) - camera).as_vec3();
                for axis in 0..3 {
                    record.model[3][axis] += delta[axis];
                }
                record
            } else {
                let mut instance = builder.instance(
                    world_geometry::row_major_3x4(copy.transform),
                    DVec3::from(copy.origin),
                    camera,
                    super::native_block_entity_draw::prepared_tint(copy.tint),
                    copy.light,
                );
                instance.tint[3] = (copy.tint >> 24) as f32 / 255.0;
                instance
            };
            staged.push(sunrise_client::gfx::PreparedModelDraw {
                cpu_vertices: cut,
                generation: copy.generation,
                slot: copy.slot,
                instance,
            });
            true
        };
        if cached.is_some() {
            draw.submit_validated_model(
                batch,
                instances,
                self.client.view_frustum.as_ref(),
                stage_copy,
            );
        } else {
            draw.submit_model(
                batch,
                instances,
                self.client.view_frustum.as_ref(),
                |copy| stage_copy(0, copy),
            );
        }
        Ok(())
    }
    #[cfg(test)]
    pub(in crate::game) fn commit_model_instances(
        &self,
        draw: &mut WorldGeometry,
        camera: DVec3,
        entities: &mut Vec<Vertex>,
    ) {
        self.bake_model_instances(
            draw,
            camera,
            entities,
            &mut self.client.entity_spans.borrow_mut(),
            None,
        );
    }

    /// Bake CPU copies onto the entity stream with their pack spans. Opaque standalone copies
    /// go as block entities, one run per overlay. Material copies follow and the renderer sorts
    /// their translucent quads far to near. Copies carrying a `ThroughWalls` pipeline modifier
    /// bake into `see_through` when provided.
    fn bake_model_instances(
        &self,
        draw: &mut WorldGeometry,
        camera: DVec3,
        entities: &mut Vec<Vertex>,
        spans: &mut Vec<sunrise_client::gfx::pack_draw::EntitySpan>,
        mut see_through: Option<&mut Vec<Vertex>>,
    ) {
        let capacity = self.entity_quad_capacity();
        let reserved_quads = self.entity_batch_reserved_quads();
        let mut run_overlay = None;
        let mut materials = self.client.model_copy_order.borrow_mut();
        let mut mark_span = |start| {
            if let Some(span) = self.model_copy_span(start) {
                match spans.last_mut() {
                    Some(last) if last.start == span.start => *last = span,
                    _ => spans.push(span),
                }
            }
        };
        materials.clear();
        for index in 0..draw.copy_count() {
            let copy = draw.copy(index);
            let Some(slot) = self.client.standalone_geometry.instance_slot(copy.slot) else {
                continue;
            };
            if copy.through_walls {
                if let (Some(out), Some(faces)) = (
                    see_through.as_deref_mut(),
                    self.client.standalone_geometry.faces(copy.slot),
                ) {
                    submit_opaque_model_copy(
                        out,
                        capacity,
                        0,
                        faces,
                        copy.transform,
                        (DVec3::from(copy.origin) - camera).as_vec3(),
                        copy.light,
                        copy.tint,
                        copy.color,
                        copy.overlay,
                        copy.hidden_faces,
                    );
                }
                continue;
            }
            if slot.translucent || slot.state || copy.color >> 24 != 0xFF {
                materials.push(index as u32);
                continue;
            }
            let Some(faces) = self.client.standalone_geometry.faces(copy.slot) else {
                continue;
            };
            let room =
                sunrise_client::gfx::EntityQuadScope::new(entities, capacity, reserved_quads)
                    .remaining_quads();
            if room == 0 {
                break;
            }
            let start = entities.len();
            if run_overlay != Some(copy.overlay) {
                run_overlay = Some(copy.overlay);
                mark_span(start);
            }
            submit_opaque_model_copy(
                entities,
                capacity,
                reserved_quads,
                faces,
                copy.transform,
                (DVec3::from(copy.origin) - camera).as_vec3(),
                copy.light,
                copy.tint,
                copy.color,
                copy.overlay,
                copy.hidden_faces,
            );
        }
        if !materials.is_empty() {
            mark_span(entities.len());
            let layers = &self.client.client_pack.render.sprite_layers;
            for &index in materials.iter() {
                if !sunrise_client::gfx::EntityQuadScope::new(entities, capacity, reserved_quads)
                    .has_room()
                {
                    break;
                }
                let copy = draw.copy(index as usize);
                let Some(faces) = self.model_faces(copy.slot) else {
                    continue;
                };
                let (state, special) = self
                    .client
                    .standalone_geometry
                    .instance_slot(copy.slot)
                    .map_or((false, None), |slot| (slot.state, slot.special));
                let shift = (DVec3::from(copy.origin) - camera).as_vec3();
                let pose =
                    Mat4::from_translation(shift) * world_geometry::row_major_3x4(copy.transform);
                let options = terrain_model::NativeModelRenderOptions {
                    shade: if state {
                        terrain_model::NativeModelShade::Model
                    } else {
                        terrain_model::NativeModelShade::Entity
                    },
                    tint: copy.tint & 0xFF_FFFF,
                    tint_alpha: (copy.tint >> 24) as u8,
                    color: copy.color,
                    hidden_faces: copy.hidden_faces,
                    ..Default::default()
                };
                {
                    let mut scope = sunrise_client::gfx::EntityQuadScope::new(
                        entities,
                        capacity,
                        reserved_quads,
                    );
                    submit_material_model_copy(
                        &mut scope,
                        faces,
                        pose,
                        copy.light,
                        options,
                        |name| {
                            layers
                                .get(name)
                                .or_else(|| layers.get(name.strip_prefix("minecraft:")?))
                                .copied()
                        },
                    );
                    if let Some(state) = special {
                        self.special_model_part(state, pose, copy.light, copy.color, &mut scope);
                    }
                }
            }
        }
        draw.clear_copies();
    }
}

#[allow(clippy::too_many_arguments)]
fn submit_opaque_model_copy(
    entities: &mut Vec<Vertex>,
    capacity: sunrise_client::gfx::EntityQuadCapacity,
    reserved_quads: usize,
    faces: &[crate::game::client_side::minecarts_client::PreparedModelFace],
    transform: [f32; 12],
    shift: Vec3,
    light: u32,
    tint: u32,
    color: u32,
    overlay: u32,
    hidden_faces: u8,
) {
    let room = sunrise_client::gfx::EntityQuadScope::new(entities, capacity, reserved_quads)
        .remaining_quads();
    if room == 0 {
        return;
    }
    let start = entities.len();
    let shown = || {
        faces
            .iter()
            .filter(|face| !face.hidden_by(hidden_faces))
            .take(room)
    };
    crate::game::client_side::minecarts_client::emit_model_faces(
        shown().copied().map(|face| (face, light)),
        world_geometry::row_major_3x4(transform),
        true,
        |index| (index >= 0).then_some(tint & 0xFF_FFFF),
        entities,
    );
    let (quads, _) = entities[start..].as_chunks_mut::<4>();
    for quad in quads {
        for vertex in quad.iter_mut() {
            vertex.pos = (Vec3::from(vertex.pos) + shift).to_array();
            vertex.layer = overlay;
            if color != u32::MAX {
                vertex.color = terrain_model::multiply_draw_color(vertex.color, color);
            }
        }
    }
    crate::game::client_side::minecarts_client::scale_tint_alpha(
        &mut entities[start..],
        shown(),
        tint,
    );
    sunrise_client::gfx::EntityQuadScope::new(entities, capacity, reserved_quads)
        .retain_valid_since(start);
}

fn submit_material_model_copy(
    scope: &mut sunrise_client::gfx::EntityQuadScope<'_>,
    faces: &[sunrise_client::models::catalog::BakedFace],
    pose: Mat4,
    light: u32,
    options: terrain_model::NativeModelRenderOptions<'_>,
    sprite_layer: impl Fn(&str) -> Option<u32>,
) {
    terrain_model::draw_model(faces, pose, light, options, sprite_layer, |quad| {
        scope.submit_quad(quad)
    });
}
/// Refuse a `Vertices` draw whose engine-generated indices (`Quads`, `Lines`, `TriangleFan`)
/// would need a larger index buffer than the device allows, as a draw with no vertex-step
/// buffer to bound it would otherwise ask for.
fn check_generated_indices(
    topology: mod_api::PrimitiveTopology,
    draw: &mod_api::MeshDraw,
    max_buffer_size: u64,
) -> Result<(), PipelineDrawError> {
    if let mod_api::DrawRange::Vertices(range) = &draw.range
        && matches!(
            topology,
            mod_api::PrimitiveTopology::Quads
                | mod_api::PrimitiveTopology::Lines
                | mod_api::PrimitiveTopology::TriangleFan
        )
        && range.start <= range.end
        && u64::from(topology.index_count(range.end - range.start)) * 4 > max_buffer_size
    {
        return Err(PipelineDrawError::InvalidRange);
    }
    Ok(())
}

/// Check that an indexed draw only reaches records every per-vertex buffer holds: each
/// vertex-step buffer is non-empty, and the largest index in `range` plus `base_vertex` falls
/// inside all of them. `records` lists each bound buffer's step mode and record count.
fn check_indexed_vertices(
    records: &[(mod_api::VertexStepMode, usize)],
    format: mod_api::IndexFormat,
    index_bytes: &[u8],
    range: &std::ops::Range<u32>,
    base_vertex: i32,
) -> Result<(), PipelineDrawError> {
    if range.is_empty() {
        return Ok(());
    }
    let size = format.byte_size() as usize;
    let used = &index_bytes[range.start as usize * size..range.end as usize * size];
    let read = |chunk: &[u8]| match format {
        mod_api::IndexFormat::Uint16 => u32::from(u16::from_le_bytes([chunk[0], chunk[1]])),
        _ => u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]),
    };
    let (lowest, highest) = used
        .chunks_exact(size)
        .map(read)
        .fold((u32::MAX, 0), |(lo, hi), i| (lo.min(i), hi.max(i)));
    let first = i64::from(lowest) + i64::from(base_vertex);
    let last = i64::from(highest) + i64::from(base_vertex);
    for &(step, count) in records {
        if step == mod_api::VertexStepMode::Vertex && (first < 0 || last >= count as i64) {
            return Err(PipelineDrawError::InvalidRange);
        }
    }
    Ok(())
}

/// Whether a compute dispatch uses one storage buffer or one storage-image mip level twice with
/// a write among the uses (or reads its indirect buffer while writing it): wgpu rejects such a
/// usage scope when the encoder finishes, so the dispatch is refused here instead.
fn compute_bindings_alias(
    layout: &mod_api::BindGroupLayout,
    bindings: &mod_api::DrawBindings<'_>,
    indirect: Option<mod_api::StorageBufferRef>,
) -> bool {
    let mut buffers: Vec<(mod_api::StorageBufferRef, bool)> =
        indirect.map(|buffer| (buffer, false)).into_iter().collect();
    let mut images: Vec<(mod_api::StorageImageBinding, bool)> = Vec::new();
    for decl in &layout.entries {
        let Some((_, value)) = bindings
            .entries
            .iter()
            .find(|(name, _)| *name == decl.name.as_str())
        else {
            continue;
        };
        match (decl.kind, value) {
            (
                mod_api::BindingKind::StorageBuffer { read_only, .. },
                mod_api::BindingValue::StorageBuffer(slice),
            ) => buffers.push((slice.buffer, !read_only)),
            (
                mod_api::BindingKind::StorageImage { access },
                mod_api::BindingValue::StorageImage(image),
            ) => images.push((*image, access != mod_api::StorageImageAccess::ReadOnly)),
            _ => {}
        }
    }
    let clashes = |same: bool, a_writes: bool, b_writes: bool| same && (a_writes || b_writes);
    buffers.iter().enumerate().any(|(i, (a, a_writes))| {
        buffers[i + 1..]
            .iter()
            .any(|(b, b_writes)| clashes(a == b, *a_writes, *b_writes))
    }) || images.iter().enumerate().any(|(i, (a, a_writes))| {
        images[i + 1..]
            .iter()
            .any(|(b, b_writes)| clashes(a == b, *a_writes, *b_writes))
    })
}

#[cfg(test)]
use sunrise_gameplay_common::game::mods::ModGameExt1 as _;

#[cfg(test)]
use sunrise_gameplay_common::game::native_lifecycle_common::NativeLifecycleCommonGameExt1 as _;

#[cfg(test)]
mod tests {
    use super::*;
    use glam::{DVec3, Mat4, Quat, Vec3};
    use mod_api::{
        ClientHost, Hooks, Identifier, InstanceGroups, LightCoords, ModEntry, ModelInstance,
        ModelRef, Overlay, SubmitCustomGeometryEvent, WorldDrawContext,
    };
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use sunrise_client::mesher::Vertex;

    static FILL_MODEL: Mutex<Option<ModelRef>> = Mutex::new(None);
    static FILL_ARMED: AtomicBool = AtomicBool::new(false);

    struct FillBatch;
    #[mod_api::__subscriptions]
    impl Hooks for FillBatch {
        fn submit_custom_geometry(
            &self,
            _: &dyn ClientHost,
            event: &mut SubmitCustomGeometryEvent<'_>,
        ) {
            if !FILL_ARMED.load(Ordering::Relaxed) {
                return;
            }
            let model = FILL_MODEL.lock().unwrap().unwrap();
            event
                .draw
                .draw_model_instances(model, &[ModelInstance::at([1.0, 0.0, 0.0], 0xFFFF_FFFF)])
                .unwrap();
        }
    }

    fn rows(local: Mat4) -> [f32; 12] {
        let c = local.to_cols_array_2d();
        [
            c[0][0], c[1][0], c[2][0], c[3][0], c[0][1], c[1][1], c[2][1], c[3][1], c[0][2],
            c[1][2], c[2][2], c[3][2],
        ]
    }

    fn context<'a, 'g>(
        game: &'a ClientGame<'g>,
        draw: &'a mut WorldGeometry,
    ) -> GameWorldDrawContext<'a, 'g> {
        GameWorldDrawContext {
            game,
            surface: draw,
        }
    }

    struct Restore;
    impl Drop for Restore {
        fn drop(&mut self) {
            sunrise_client::gfx::cap_prepared_instance_slots(u64::MAX);
            sunrise_client::gfx::set_test_prepared_generation(None);
        }
    }

    fn first_colour(pixels: &[u8]) -> [u8; 4] {
        pixels
            .as_chunks::<4>()
            .0
            .iter()
            .copied()
            .find(|pixel| pixel[0] != 0 || pixel[1] != 0 || pixel[2] != 0)
            .unwrap_or([0; 4])
    }

    fn worst_pixels(
        game: &ClientGame<'_>,
        cpu_vertices: &[Vertex],
        instance: &sunrise_client::gfx::PreparedModelInstance,
        slot: u32,
        uploaded: u64,
        view: Mat4,
    ) -> (u8, [u8; 4], [u8; 4]) {
        let pixels = game.client.gui.atlas.pixels().to_vec();
        let size = [game.client.gui.atlas.width, game.client.gui.atlas.height];
        let cpu = sunrise_client::gfx::entity_pixel_test::render(
            cpu_vertices,
            size,
            &pixels,
            view,
            false,
        )
        .expect("software Vulkan entity pixels");
        let (vertices, ranges) = game.client.standalone_geometry.prepared_upload();
        let gpu = sunrise_client::gfx::entity_pixel_test::render_instance(
            sunrise_client::gfx::entity_pixel_test::PreparedInstanceDraw {
                vertices: &vertices,
                ranges,
                uploaded,
                requested: uploaded,
                slot,
                instances: std::slice::from_ref(instance),
                per_copy: false,
                hdr: false,
                atlas_size: size,
                atlas: &pixels,
                view_proj: view,
            },
        )
        .expect("software Vulkan prepared instance");
        let worst = cpu
            .iter()
            .zip(gpu.iter())
            .map(|(left, right)| left.abs_diff(*right))
            .max()
            .unwrap_or(0);
        (worst, first_colour(&cpu), first_colour(&gpu))
    }

    fn grass(game: &ClientGame<'_>) -> ModelRef {
        let model = game
            .native_model(&Identifier::parse("minecraft:block/grass_block").unwrap())
            .unwrap();
        let generation = mod_api::__private::game::Fields::fields(&model).generation;
        sunrise_client::gfx::set_test_prepared_generation(Some(generation));
        model
    }

    #[test]
    fn indexed_draws_need_every_vertex_buffer_to_hold_the_indexed_records() {
        use mod_api::{IndexFormat, VertexStepMode};
        let indices: Vec<u8> = [0u16, 1, 2, 0]
            .iter()
            .flat_map(|i| i.to_le_bytes())
            .collect();
        let check = |records: &[(VertexStepMode, usize)], base: i32| {
            check_indexed_vertices(records, IndexFormat::Uint16, &indices, &(0..4), base)
        };
        assert_eq!(check(&[(VertexStepMode::Vertex, 3)], 0), Ok(()));
        // An empty per-vertex buffer with a non-empty indexed range is refused, not drawn.
        assert_eq!(
            check(&[(VertexStepMode::Vertex, 0)], 0),
            Err(PipelineDrawError::InvalidRange)
        );
        // The largest index (2) must fit, after `base_vertex`.
        assert_eq!(
            check(&[(VertexStepMode::Vertex, 2)], 0),
            Err(PipelineDrawError::InvalidRange)
        );
        // `base_vertex` 2 moves the largest index to record 4.
        assert_eq!(check(&[(VertexStepMode::Vertex, 5)], 2), Ok(()));
        assert_eq!(
            check(&[(VertexStepMode::Vertex, 4)], 2),
            Err(PipelineDrawError::InvalidRange)
        );
        // A negative `base_vertex` that moves the lowest index below record 0 is refused.
        assert_eq!(
            check(&[(VertexStepMode::Vertex, 4)], -1),
            Err(PipelineDrawError::InvalidRange)
        );
        // Instance-step buffers are bounded by the instance range, not by the indices.
        assert_eq!(check(&[(VertexStepMode::Instance, 0)], 0), Ok(()));
        // An empty range reads nothing.
        assert_eq!(
            check_indexed_vertices(
                &[(VertexStepMode::Vertex, 0)],
                IndexFormat::Uint16,
                &indices,
                &(2..2),
                0
            ),
            Ok(())
        );
    }

    #[test]
    fn generated_indices_are_bounded_by_the_device_buffer_size() {
        use mod_api::{MeshDraw, PrimitiveTopology};
        let draw = MeshDraw::vertices(0..4_000_000_000).with_instances(0..4);
        // Four million quads would need 24 bytes each of generated indices.
        assert_eq!(
            check_generated_indices(PrimitiveTopology::Quads, &draw, 1 << 30),
            Err(PipelineDrawError::InvalidRange)
        );
        assert_eq!(
            check_generated_indices(PrimitiveTopology::Triangles, &draw, 1 << 30),
            Ok(())
        );
        let small = MeshDraw::vertices(0..4);
        assert_eq!(
            check_generated_indices(PrimitiveTopology::Quads, &small, 24),
            Ok(())
        );
        assert_eq!(
            check_generated_indices(PrimitiveTopology::Quads, &small, 23),
            Err(PipelineDrawError::InvalidRange)
        );
    }

    #[test]
    #[ignore = "daily"]
    fn an_opaque_copy_with_a_colour_or_hidden_faces_is_baked_on_the_cpu() {
        let _fixture =
            sunrise_client::models::standalone_fixture(vec!["minecraft:block/grass_block".into()]);
        super::super::native_entity_refs::tests::with_client_game(|game| {
            let _restore = Restore;
            let model = grass(game);
            let camera = DVec3::new(0.0, 70.0, 0.0);
            let half_red = 0xFF80_FFFF;
            let mut draw = WorldGeometry::default();
            draw.reset(camera.to_array(), Mat4::IDENTITY, [0.0, 0.0]);
            // One whole copy, and one with its bottom faces hidden (direction bit 0).
            context(game, &mut draw)
                .draw_model_instances(
                    model,
                    &[
                        ModelInstance::at([3.0, 70.0, 0.0], u32::MAX).with_color(half_red),
                        ModelInstance::at([9.0, 70.0, 0.0], u32::MAX)
                            .with_color(half_red)
                            .with_hidden_faces(0b1),
                    ],
                )
                .unwrap();
            let mut entities = Vec::new();
            game.commit_model_instances(&mut draw, camera, &mut entities);
            assert!(
                game.client.prepared_model_draws.borrow().is_empty(),
                "no GPU copy"
            );
            let slot = mod_api::__private::game::Fields::fields(&model).slot;
            let faces = game
                .client
                .standalone_geometry
                .faces(slot)
                .expect("prepared faces")
                .len();
            // The first copy holds every face; the second lacks the hidden ones.
            assert!(entities.len() > faces * 4, "the whole copy is there");
            assert!(entities.len() < faces * 8, "the hidden faces are absent");
            // Vertex colours are ABGR: white times `half_red` (`0xFF80FFFF`) on every face.
            assert!(entities.iter().all(|v| v.color == 0xFFFF_FF80));
        });
    }

    #[test]
    #[ignore = "daily"]
    fn a_block_state_resolves_once_and_an_unknown_state_is_refused() {
        let _fixture = sunrise_client::models::standalone_fixture(Vec::new());
        super::super::native_entity_refs::tests::with_client_game(|game| {
            let slot = |state: u32| {
                game.native_block_state_model(sunrise_world::registry::BlockStateId(state))
                    .map(|model| mod_api::__private::game::Fields::fields(&model).slot)
            };
            // Air has no model, and a state past the registry's table is not a panic.
            assert_eq!(
                slot(game.common.world.air.0),
                Err(mod_api::ModelError::UnknownModel)
            );
            assert_eq!(slot(u32::MAX), Err(mod_api::ModelError::UnknownModel));
            // The first state with a model resolves to the same slot every time.
            let (state, first) = (1..)
                .find_map(|state| slot(state).ok().map(|slot| (state, slot)))
                .expect("some block state has a model");
            assert_eq!(slot(state), Ok(first));
        });
    }

    #[test]
    #[ignore = "daily"]
    fn a_special_block_state_draws_its_renderer_and_a_faded_copy_blends() {
        sunrise_gameplay_common::game::in_own_process(
            "game::client_side::native_custom_geometry::tests::a_special_block_state_draws_its_renderer_and_a_faded_copy_blends",
            || {
                let _fixture = sunrise_client::models::standalone_fixture(Vec::new());
                super::super::native_entity_refs::tests::with_client_game(|game| {
                    let chest = game
                        .pack
                        .assets
                        .registry
                        .default_state("minecraft:chest")
                        .unwrap();
                    // `BuiltInBlockModels`: a chest is its block-state model plus its special renderer.
                    let model = game
                        .native_block_state_model(chest)
                        .expect("a chest resolves to its composite model");
                    let slot = mod_api::__private::game::Fields::fields(&model).slot;
                    let instance = game.client.standalone_geometry.instance_slot(slot).unwrap();
                    assert_eq!(instance.special, Some(chest));
                    // The chest's box reaches 14/16 up, so copies cull by the renderer's geometry.
                    assert!(instance.bounds[1][1] > 0.8, "{:?}", instance.bounds);
                    let camera = DVec3::new(0.0, 70.0, 0.0);
                    let mut draw = WorldGeometry::default();
                    draw.reset(camera.to_array(), Mat4::IDENTITY, [0.0, 0.0]);
                    context(game, &mut draw)
                        .draw_model_instances(
                            model,
                            &[
                                ModelInstance::at([3.0, 70.0, 0.0], u32::MAX),
                                ModelInstance::at([6.0, 70.0, 0.0], u32::MAX)
                                    .with_color(0x80FF_FFFF),
                            ],
                        )
                        .unwrap();
                    let mut entities = Vec::new();
                    game.commit_model_instances(&mut draw, camera, &mut entities);
                    let (opaque, faded): (Vec<&Vertex>, Vec<&Vertex>) =
                        entities.iter().partition(|vertex| vertex.pos[0] < 5.0);
                    assert!(!opaque.is_empty() && opaque.len() == faded.len());
                    assert!(
                        opaque
                            .iter()
                            .all(|v| v.layer & sunrise_client::gfx::OIT_ENTITY == 0
                                && v.color >> 24 == 0xFF)
                    );
                    assert!(
                        faded
                            .iter()
                            .all(|v| v.layer & sunrise_client::gfx::OIT_ENTITY != 0
                                && v.color >> 24 < 0xFF)
                    );
                    let registry = &game.common.pack.assets.registry;
                    let parse =
                        |text| sunrise_client::block_sprites::parse_state(registry, text).unwrap();
                    let (left, gateway, table) = (
                        parse("chest[type=left]"),
                        parse("end_gateway"),
                        parse("enchanting_table"),
                    );
                    let special = |state, pose, color| {
                        let mut out = Vec::new();
                        game.special_model_part(
                            state,
                            pose,
                            0xF0F0,
                            color,
                            &mut sunrise_client::gfx::EntityQuadScope::new(
                                &mut out,
                                sunrise_client::gfx::EntityQuadCapacity::default_device(),
                                0,
                            ),
                        );
                        out
                    };
                    // `ChestModel.createDoubleBodyLeftLayer`: bottom, lid and lock, each a cuboid with every
                    // face but WEST (`allOfEnumExcept`), so 3 * 5 faces of 4 vertices; the half draws
                    // alone, with no neighbour to pair with.
                    let out = special(left, Mat4::IDENTITY, u32::MAX);
                    assert_eq!(out.len(), 3 * 5 * 4);
                    // `ChestModel.createSingleBodyLayer`: bottom, lid and lock, 3 * 6 faces of 4 vertices.
                    let finite = special(chest, Mat4::IDENTITY, u32::MAX);
                    assert_eq!(finite.len(), 3 * 6 * 4);
                    // Every quad of a pose past the finite range is skipped, not the draw refused.
                    let out = special(
                        chest,
                        Mat4::from_scale(Vec3::splat(f32::INFINITY)),
                        u32::MAX,
                    );
                    assert_eq!(out.len(), 0);
                    // EndCubeSpecialRenderer: all six faces, and the starfield stays opaque when faded.
                    let out = special(gateway, Mat4::IDENTITY, 0x80FF_FFFF);
                    assert_eq!(out.len(), 24);
                    assert!(
                        out.iter()
                            .all(|v| v.layer & sunrise_client::gfx::OIT_ENTITY == 0)
                    );
                    // The enchanting table's book is `BookModel.createBodyLayer`: left_lid, right_lid,
                    // seam, left_pages, right_pages and two flip pages, 7 cuboids of 6 faces of 4 vertices.
                    let out = special(table, Mat4::IDENTITY, u32::MAX);
                    assert_eq!(out.len(), 7 * 6 * 4);
                    let model = game.native_block_state_model(table).unwrap();
                    let slot = mod_api::__private::game::Fields::fields(&model).slot;
                    assert!(!game.model_faces(slot).unwrap().is_empty());
                    assert_eq!(
                        game.client
                            .standalone_geometry
                            .instance_slot(slot)
                            .unwrap()
                            .special,
                        Some(table)
                    );
                });
            },
        );
    }

    #[test]
    #[ignore = "daily"]
    fn a_tinted_block_state_carries_its_default_tint_in_the_baked_colour() {
        sunrise_gameplay_common::game::in_own_process(
            "game::client_side::native_custom_geometry::tests::a_tinted_block_state_carries_its_default_tint_in_the_baked_colour",
            || {
                super::super::native_entity_refs::tests::with_client_game(|game| {
                    let colours = |name: &str| {
                        let state = game
                            .common
                            .pack
                            .assets
                            .registry
                            .default_state(name)
                            .unwrap();
                        let model = game.native_block_state_model(state).unwrap();
                        let slot = mod_api::__private::game::Fields::fields(&model).slot;
                        let faces = game.model_faces(slot).unwrap();
                        faces
                            .iter()
                            .map(|face| (face.quad.tint_index, face.quad.color))
                            .collect::<Vec<_>>()
                    };
                    let tinted = |faces: &[(i32, Option<i32>)]| {
                        faces
                            .iter()
                            .filter(|(index, _)| *index >= 0)
                            .map(|(_, colour)| *colour)
                            .collect::<Vec<_>>()
                    };
                    let grass = tinted(&colours("minecraft:grass_block"));
                    assert!(!grass.is_empty(), "the grass block has a tinted face");
                    // `GrassColor.getDefaultColor()` is `get(0.5, 1.0)`: pixel (127, 127) of
                    // `colormap/grass.png`, 0x7CBD6B, times white.
                    assert!(
                        grass.iter().all(|c| *c == Some(0xFF7C_BD6Bu32 as i32)),
                        "{grass:x?}"
                    );
                    // Stone has faces and none is tinted, so none carries a colour.
                    let stone = colours("minecraft:stone");
                    assert!(!stone.is_empty());
                    assert!(
                        stone
                            .iter()
                            .all(|&(index, colour)| index < 0 && colour.is_none())
                    );
                });
            },
        );
    }

    #[test]
    #[ignore = "daily"]
    fn persistent_groups_match_batch() {
        use mod_api::{GroupError, InstanceGroups};
        let _fixture = sunrise_client::models::standalone_fixture(vec![
            "minecraft:block/grass_block".into(),
            "minecraft:block/glass".into(),
        ]);
        super::super::native_entity_refs::tests::with_client_game(|game| {
            let _restore = Restore;
            let model = grass(game);
            sunrise_client::gfx::cap_prepared_instance_slots(16_384);
            let _owner =
                mod_platform::content::crash::enter_native_mod(mod_api::content::crash::ModCall {
                    id: "a",
                    hook: "test",
                    part: None,
                });
            let large_group = game.create_group(model, 16_385).unwrap();
            large_group.destroy(game).unwrap();
            let glass = game
                .native_model(&Identifier::parse("minecraft:block/glass").unwrap())
                .unwrap();
            assert!(game.create_group(glass, 1).is_ok());
            let group = game.create_group(model, 8).unwrap();
            let near = ModelInstance::at([0.0; 3], 0xFF80_C040);
            let far = ModelInstance::at([40.0, 0.0, 0.0], 0xFFFF_FFFF);
            let setup: &dyn mod_api::ClientSetupContext = game;
            group.write(setup, 0, &[far, near]).unwrap();
            assert_eq!(group.write(game, 8, &[near]), Err(GroupError::OutOfRange));
            let mut bad = near;
            bad.light.block = 16;
            assert_eq!(
                group.write(game, 1, &[far, bad]),
                Err(GroupError::InvalidLight)
            );
            assert_eq!(group.set_len(game, 9), Err(GroupError::OutOfRange));
            game.client.view_frustum = Some(sunrise_client::gfx::test_frustum([
                glam::Vec4::new(-1.0, 0.0, 0.0, 4.0),
                glam::Vec4::W,
                glam::Vec4::W,
                glam::Vec4::W,
                glam::Vec4::W,
                glam::Vec4::W,
            ]));
            let mut draw = WorldGeometry::default();
            {
                let _other = mod_platform::content::crash::enter_native_mod(
                    mod_api::content::crash::ModCall {
                        id: "b",
                        hook: "test",
                        part: None,
                    },
                );
                // Handles are not gated on the TLS caller mod name: another mod holding the
                // live handle can update, validate, and submit it.
                group.write(game, 0, &[far, near]).unwrap();
                group.set_len(game, 2).unwrap();
                assert_eq!(
                    group.set_group_transform(game, [f32::NAN; 12]),
                    Err(GroupError::NonFinite)
                );
                draw.reset([0.0; 3], Mat4::IDENTITY, [0.0; 2]);
                context(game, &mut draw).draw_instance_group(group).unwrap();
                game.resolve_submitted_groups(&mut draw);
                assert_eq!(game.client.prepared_model_draws.borrow().len(), 1);
                game.client.prepared_model_draws.borrow_mut().clear();
            }
            let invalid_only = vec![ModelInstance::at([f64::NAN; 3], 0); 4];
            context(game, &mut draw)
                .draw_model_instances(model, &invalid_only)
                .unwrap();
            assert!(game.client.prepared_model_draws.borrow().is_empty());
            assert_eq!(draw.copy_count(), 0);
            let render = |game: &ClientGame<'_>| {
                let staged = game.client.prepared_model_draws.borrow();
                assert_eq!(staged.len(), 1, "the far copy must be culled");
                let (vertices, ranges) = game.client.standalone_geometry.prepared_upload();
                sunrise_client::gfx::entity_pixel_test::render_instance(
                    sunrise_client::gfx::entity_pixel_test::PreparedInstanceDraw {
                        vertices: &vertices,
                        ranges,
                        uploaded: model.fields().generation,
                        requested: model.fields().generation,
                        slot: model.fields().slot,
                        instances: &[staged[0].instance],
                        per_copy: false,
                        hdr: false,
                        atlas_size: [game.client.gui.atlas.width, game.client.gui.atlas.height],
                        atlas: &game.client.gui.atlas.pixels(),
                        view_proj: Mat4::IDENTITY,
                    },
                )
                .expect("software Vulkan group pixels")
            };
            use mod_api::__private::game::Fields;
            draw.reset([0.0; 3], Mat4::IDENTITY, [0.0; 2]);
            context(game, &mut draw)
                .draw_model_instances(model, &[far, near])
                .unwrap();
            let expected = render(game);
            assert!(expected.as_chunks::<4>().0.iter().any(|p| p[..3] != [0; 3]));
            game.client.prepared_model_draws.borrow_mut().clear();
            draw.reset([0.0; 3], Mat4::IDENTITY, [0.0; 2]);
            context(game, &mut draw).draw_instance_group(group).unwrap();
            context(game, &mut draw).draw_instance_group(group).unwrap();
            game.resolve_submitted_groups(&mut draw);
            assert_eq!(game.client.prepared_model_draws.borrow().len(), 2);
            draw.reset([0.0; 3], Mat4::IDENTITY, [0.0; 2]);
            game.client.prepared_model_draws.borrow_mut().clear();
            context(game, &mut draw).draw_instance_group(group).unwrap();
            game.resolve_submitted_groups(&mut draw);
            assert_eq!(expected, render(game));
            let bytes = || game.client.instance_groups.borrow().converted_bytes();
            let stride = std::mem::size_of::<sunrise_client::gfx::PreparedModelInstance>();
            let before = bytes();
            // An unchanged frame stages all visible records, but converts none.
            draw.reset([0.0; 3], Mat4::IDENTITY, [0.0; 2]);
            game.client.prepared_model_draws.borrow_mut().clear();
            context(game, &mut draw).draw_instance_group(group).unwrap();
            game.resolve_submitted_groups(&mut draw);
            assert_eq!(bytes(), before);
            assert_eq!(
                game.client.prepared_model_draws.borrow().len() * stride,
                stride
            );
            assert_eq!(expected, render(game));
            let mut changed = near;
            changed.tint = 0xFFFF_2040;
            changed.transform = [2.0, 0.0, 0.0, 0.2, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0];
            group.write(game, 1, &[changed]).unwrap();
            assert_eq!(bytes() - before, stride);
            draw.reset([0.0; 3], Mat4::IDENTITY, [0.0; 2]);
            game.client.prepared_model_draws.borrow_mut().clear();
            context(game, &mut draw)
                .draw_model_instances(model, &[far, changed])
                .unwrap();
            let rewritten = render(game);
            assert_ne!(expected, rewritten);
            draw.reset([0.0; 3], Mat4::IDENTITY, [0.0; 2]);
            game.client.prepared_model_draws.borrow_mut().clear();
            context(game, &mut draw).draw_instance_group(group).unwrap();
            game.resolve_submitted_groups(&mut draw);
            assert_eq!(rewritten, render(game));
            assert_eq!(
                game.client.prepared_model_draws.borrow().len() * stride,
                stride
            );
            let pose = [
                0.5, 0.0, 0.0, 0.125, 0.0, 0.5, 0.0, 0.25, 0.0, 0.0, 0.5, 0.0,
            ];
            group.set_group_transform(game, pose).unwrap();
            assert_eq!(bytes() - before, 3 * stride); // one copy, then two live copies
            let mut precomposed = changed;
            // Independent result of scale(0.5) * (scale_x(2) + translate_x(0.2)).
            precomposed.transform = [
                1.0, 0.0, 0.0, 0.225, 0.0, 0.5, 0.0, 0.25, 0.0, 0.0, 0.5, 0.0,
            ];
            draw.reset([0.0; 3], Mat4::IDENTITY, [0.0; 2]);
            game.client.prepared_model_draws.borrow_mut().clear();
            context(game, &mut draw)
                .draw_model_instances(model, &[precomposed])
                .unwrap();
            let transformed = render(game);
            assert_ne!(expected, transformed);
            let count = bytes();
            assert_eq!(
                group.set_group_transform(game, [f32::INFINITY; 12]),
                Err(GroupError::NonFinite)
            );
            assert_eq!(bytes(), count);
            draw.reset([0.0; 3], Mat4::IDENTITY, [0.0; 2]);
            game.client.prepared_model_draws.borrow_mut().clear();
            context(game, &mut draw).draw_instance_group(group).unwrap();
            game.resolve_submitted_groups(&mut draw);
            assert_eq!(transformed, render(game));
            assert_eq!(
                game.client.prepared_model_draws.borrow().len() * stride,
                stride
            );
            let mut outside = pose;
            outside[3] = 40.0;
            group.set_len(game, 0).unwrap();
            let count = bytes();
            group.set_group_transform(game, outside).unwrap();
            assert_eq!(bytes(), count, "no live records to rebuild");
            group.set_len(game, 2).unwrap();
            assert_eq!(
                bytes() - count,
                2 * stride,
                "newly exposed records use the current pose"
            );
            draw.reset([0.0; 3], Mat4::IDENTITY, [0.0; 2]);
            game.client.prepared_model_draws.borrow_mut().clear();
            context(game, &mut draw).draw_instance_group(group).unwrap();
            game.resolve_submitted_groups(&mut draw);
            assert!(
                game.client.prepared_model_draws.borrow().is_empty(),
                "cull group-transformed bounds"
            );
            group
                .set_group_transform(game, mod_api::IDENTITY_TRANSFORM)
                .unwrap();
            group
                .write(
                    game,
                    0,
                    &[ModelInstance::at(
                        [30_000_000.25, 30_000_000.5, 30_000_000.75],
                        0xFFFF_FFFF,
                    )],
                )
                .unwrap();
            group.set_len(game, 1).unwrap();
            draw.reset([30_000_000.0; 3], Mat4::IDENTITY, [0.0; 2]);
            game.client.prepared_model_draws.borrow_mut().clear();
            context(game, &mut draw).draw_instance_group(group).unwrap();
            game.resolve_submitted_groups(&mut draw);
            {
                let staged = game.client.prepared_model_draws.borrow();
                assert_eq!(staged.len(), 1);
                // Subtract world coordinates in f64: each fractional block survives.
                assert_eq!(staged[0].instance.model[3], [0.25, 0.5, 0.75, 1.0]);
            }
            // Groups grow past the old 65,536 per-mod cap.
            let sibling = game.create_group(model, 16_384).unwrap();
            let third = game.create_group(model, 16_384).unwrap();
            let fourth = game.create_group(model, 16_384).unwrap();
            let fifth = game.create_group(model, 16_384).unwrap();
            let sixth = game.create_group(model, 1).unwrap();
            for handle in [sibling, third, fourth, fifth, sixth] {
                handle.destroy(game).unwrap();
            }
            group.destroy(game).unwrap();
            assert_eq!(group.set_len(game, 1), Err(GroupError::Stale));
            assert_eq!(
                group.set_group_transform(game, pose),
                Err(GroupError::Stale)
            );
            assert_eq!(
                context(game, &mut draw).draw_instance_group(group),
                Err(GroupError::Stale)
            );
            let fresh = game.create_group(model, 8).unwrap();
            assert_ne!(fresh, group);
            game.spawn_pack_reload(super::super::packs::ReloadKind::Selection)
                .unwrap();
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
            while game.client.pack_reload.0.is_some() && std::time::Instant::now() < deadline {
                game.drive_pack_reload();
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
            assert_ne!(
                game.client.standalone_geometry.generation(),
                model.fields().generation,
                "reload error: {:?}",
                game.client.chat_ui.options.resource_error
            );
            let reload: &dyn mod_api::ClientReloadHost = game;
            assert_eq!(fresh.set_len(reload, 1), Err(GroupError::Stale));
            let after_reload: Vec<_> = (0..4)
                .map(|_| {
                    reload
                        .instance_groups()
                        .create_group(grass(game), 16_384)
                        .unwrap()
                })
                .collect();
            assert_ne!(fresh, after_reload[0]);
            after_reload[0].write(reload, 0, &[near]).unwrap();
            game.native_lifecycle_world_closed();
            assert_eq!(after_reload[0].destroy(game), Err(GroupError::Stale));
            assert_ne!(after_reload[0], game.create_group(grass(game), 1).unwrap());
        });
    }

    #[test]
    #[ignore = "daily"]
    fn persistent_groups_cpu_routes_and_lifetime() {
        use mod_api::__private::game::Fields;
        use mod_api::{GroupError, InstanceGroups};
        let _fixture =
            sunrise_client::models::standalone_fixture(vec!["minecraft:block/grass_block".into()]);
        super::super::native_entity_refs::tests::with_client_game(|game| {
            let _restore = Restore;
            let model = grass(game);
            let _owner =
                mod_platform::content::crash::enter_native_mod(mod_api::content::crash::ModCall {
                    id: "group-owner",
                    hook: "test",
                    part: None,
                });
            let camera = DVec3::new(8_000_000.0, 70.0, -4_000_000.0);
            let mut copy = ModelInstance::at(camera.to_array(), 0xFF80_C040);
            copy.light = LightCoords::new(3, 12).unwrap();
            copy.transform = rows(Mat4::from_scale(Vec3::splat(0.6)));
            let pose = Mat4::from_translation(Vec3::new(-0.3, -0.3, 0.2));
            let mut composed = copy;
            // Independent expected affine transform: scale stays 0.6; translation is the pose.
            composed.transform = [0.6, 0.0, 0.0, -0.3, 0.0, 0.6, 0.0, -0.3, 0.0, 0.0, 0.6, 0.2];
            let pixels = |game: &ClientGame<'_>, vertices: &[Vertex]| {
                sunrise_client::gfx::entity_pixel_test::render(
                    vertices,
                    [game.client.gui.atlas.width, game.client.gui.atlas.height],
                    &game.client.gui.atlas.pixels(),
                    Mat4::IDENTITY,
                    false,
                )
                .expect("software Vulkan CPU group pixels")
            };
            // Both shader selection and unavailable GPU records force the shared CPU route.
            // Overlay copies also use it even when GPU records are available.
            for route in 0..3 {
                game.client.native_shaders.picked = if route == 0 {
                    "test-pack".into()
                } else {
                    String::new()
                };
                sunrise_client::gfx::set_test_prepared_generation(if route == 1 {
                    None
                } else {
                    Some(model.fields().generation)
                });
                copy.overlay = if route == 2 {
                    Overlay::new(7, true).unwrap()
                } else {
                    Overlay::NONE
                };
                composed.overlay = copy.overlay;
                let group = game.create_group(model, 1).unwrap();
                group.write(game, 0, &[copy]).unwrap();
                group.set_group_transform(game, rows(pose)).unwrap();
                let mut batch = WorldGeometry::default();
                batch.reset(camera.to_array(), Mat4::IDENTITY, [0.0; 2]);
                context(game, &mut batch)
                    .draw_model_instances(model, &[composed])
                    .unwrap();
                let mut expected = Vec::new();
                game.commit_model_instances(&mut batch, camera, &mut expected);
                assert!(game.client.prepared_model_draws.borrow().is_empty());
                let expected_pixels = pixels(game, &expected);
                assert!(
                    expected_pixels
                        .as_chunks::<4>()
                        .0
                        .iter()
                        .any(|p| p[..3] != [0; 3])
                );
                let mut draw = WorldGeometry::default();
                draw.reset(camera.to_array(), Mat4::IDENTITY, [0.0; 2]);
                context(game, &mut draw).draw_instance_group(group).unwrap();
                context(game, &mut draw).draw_instance_group(group).unwrap();
                // Destruction before prepare invalidates the handle, not the accepted copies.
                group.destroy(game).unwrap();
                assert_eq!(group.write(game, 0, &[copy]), Err(GroupError::Stale));
                assert_eq!(group.destroy(game), Err(GroupError::Stale));
                assert_eq!(
                    context(game, &mut draw).draw_instance_group(group),
                    Err(GroupError::Stale)
                );
                let replacement = game.create_group(model, 1).unwrap();
                assert_eq!(replacement.fields().slot, group.fields().slot);
                assert_ne!(replacement.fields().generation, group.fields().generation);
                assert_eq!(group.set_len(game, 1), Err(GroupError::Stale));
                assert_eq!(
                    group.set_group_transform(game, rows(pose)),
                    Err(GroupError::Stale)
                );
                game.resolve_submitted_groups(&mut draw);
                let mut actual = Vec::new();
                game.commit_model_instances(&mut draw, camera, &mut actual);
                assert_eq!(
                    actual.len(),
                    expected.len() * 2,
                    "repeated group draws in one view submit explicitly"
                );
                assert_eq!(
                    pixels(game, &actual[..expected.len()]),
                    expected_pixels,
                    "CPU route {route}"
                );
                assert_eq!(
                    pixels(game, &actual[expected.len()..]),
                    expected_pixels,
                    "CPU route {route} second submission"
                );
                replacement.write(game, 0, &[copy]).unwrap();
                replacement.set_len(game, 0).unwrap();
                draw.reset(camera.to_array(), Mat4::IDENTITY, [0.0; 2]);
                context(game, &mut draw)
                    .draw_instance_group(replacement)
                    .unwrap();
                game.resolve_submitted_groups(&mut draw);
                assert_eq!(draw.copy_count(), 0);
                replacement.destroy(game).unwrap();
            }
            let extra_group = game.create_group(model, 2).unwrap();
            extra_group.write(game, 0, &[copy, copy]).unwrap();
            let mut large = WorldGeometry::default();
            large.reset(camera.to_array(), Mat4::IDENTITY, [0.0; 2]);
            context(game, &mut large)
                .draw_model_instances(model, &vec![copy; 16_383])
                .unwrap();
            context(game, &mut large)
                .draw_instance_group(extra_group)
                .unwrap();
            game.resolve_submitted_groups(&mut large);
            assert_eq!(
                large.copy_count(),
                16_385,
                "groups and batches grow past the initial 16,384 hint"
            );
            extra_group.destroy(game).unwrap();
            let released = game.create_group(model, 16_384).unwrap();
            let other_group = {
                let _other = mod_platform::content::crash::enter_native_mod(
                    mod_api::content::crash::ModCall {
                        id: "unaffected-owner",
                        hook: "test",
                        part: None,
                    },
                );
                // Another mod holding the handle can access it while the owner stays enabled;
                // disabling the owner retires it below.
                released.set_len(game, 1).unwrap();
                game.create_group(model, 1).unwrap()
            };
            game.client
                .settings
                .mods
                .disabled
                .insert("group-owner".into());
            game.sync_native_switches();
            let mut draw = WorldGeometry::default();
            assert_eq!(
                context(game, &mut draw).draw_instance_group(released),
                Err(GroupError::Stale)
            );
            assert_eq!(draw.copy_count(), 0);
            {
                let _other = mod_platform::content::crash::enter_native_mod(
                    mod_api::content::crash::ModCall {
                        id: "unaffected-owner",
                        hook: "test",
                        part: None,
                    },
                );
                other_group.set_len(game, 1).unwrap();
                other_group.destroy(game).unwrap();
            }
            game.client.settings.mods.disabled.remove("group-owner");
            game.sync_native_switches();
            assert_eq!(released.set_len(game, 1), Err(GroupError::Stale));
            let reenabled: Vec<_> = (0..4)
                .map(|_| game.create_group(model, 16_384).unwrap())
                .collect();
            assert_ne!(reenabled[0], released);
            // The same owner id after retirement cannot resurrect old generations.
            game.native_lifecycle_world_closed();
            assert_eq!(released.set_len(game, 1), Err(GroupError::Stale));
            let renewed = game.create_group(model, 16_384).unwrap();
            assert_eq!(released.fields().slot, renewed.fields().slot);
            assert_eq!(released.destroy(game), Err(GroupError::Stale));
            assert_ne!(renewed, released);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn draw_model_instances_matches_cpu_pixels() {
        let _fixture =
            sunrise_client::models::standalone_fixture(vec!["minecraft:block/grass_block".into()]);
        super::super::native_entity_refs::tests::with_client_game(|game| {
            let _restore = Restore;
            let model = grass(game);
            sunrise_client::gfx::cap_prepared_instance_slots(8);
            let local = Mat4::from_scale_rotation_translation(
                Vec3::new(0.85, 1.15, 0.7),
                Quat::from_rotation_y(0.6),
                Vec3::new(0.12, -0.05, 0.08),
            );
            let origin = DVec3::new(8_000_000.5, 70.0, -4_000_000.25);
            let camera = DVec3::new(8_000_000.0, 70.2, -4_000_000.0);
            let lit = ModelInstance::new(
                origin.to_array(),
                rows(local),
                0xFF80_C040,
                LightCoords::new(3, 15).unwrap(),
                Overlay::NONE,
            );
            let other = ModelInstance::new(
                origin.to_array(),
                rows(local),
                0xFFFF_FFFF,
                LightCoords::new(15, 3).unwrap(),
                Overlay::NONE,
            );
            let hurt = ModelInstance::new(
                origin.to_array(),
                rows(local),
                0xFFFF_FFFF,
                LightCoords::FULL_BRIGHT,
                Overlay::new(0, true).unwrap(),
            );
            let mut draw = WorldGeometry::default();
            draw.reset(camera.to_array(), Mat4::IDENTITY, [0.0, 0.0]);
            context(game, &mut draw)
                .draw_model_instances(model, &[lit, other, hurt])
                .unwrap();
            let mut entities = Vec::new();
            game.commit_model_instances(&mut draw, camera, &mut entities);
            let staged = game.client.prepared_model_draws.borrow();
            assert_eq!(staged.len(), 2, "the lit copies were not instanced");
            assert_eq!(staged[0].instance.light, 0x30F0);
            assert_eq!(staged[1].instance.light, 0xF030);
            assert_eq!(
                staged[0].cpu_vertices, 0,
                "cpu overlay vertices must follow the cut"
            );
            let instance = staged[0].instance;
            let slot = staged[0].slot;
            let uploaded = staged[0].generation;
            drop(staged);
            assert!(!entities.is_empty(), "the overlay copy was not baked");
            assert!(
                entities.iter().all(|vertex| vertex.layer == 1),
                "overlay copies must follow the instanced prefix"
            );
            let view = Mat4::from_scale_rotation_translation(
                Vec3::new(1.4, 1.4, 0.2),
                Quat::IDENTITY,
                Vec3::new(-0.55, -0.85, 0.5),
            );
            let faces = game
                .client
                .standalone_geometry
                .faces(slot)
                .expect("prepared faces");
            let mut cpu_vertices = Vec::new();
            crate::game::client_side::minecarts_client::emit_model_faces(
                faces
                    .iter()
                    .copied()
                    .map(|face| (face, world_geometry::pack_light(lit.light))),
                world_geometry::row_major_3x4(lit.transform),
                true,
                |index| (index >= 0).then_some(lit.tint & 0xFF_FFFF),
                &mut cpu_vertices,
            );
            let shift = (origin - camera).as_vec3();
            let alpha = lit.tint >> 24;
            for vertex in &mut cpu_vertices {
                vertex.pos = (Vec3::from(vertex.pos) + shift).to_array();
                if alpha != 255 {
                    let scaled = (vertex.color >> 24) * alpha / 255;
                    vertex.color = vertex.color & 0x00FF_FFFF | scaled << 24;
                }
            }
            let (worst, cpu_px, gpu_px) =
                worst_pixels(game, &cpu_vertices, &instance, slot, uploaded, view);
            eprintln!("worst {worst} cpu {cpu_px:?} gpu {gpu_px:?}");
            assert_eq!(worst, 0, "instanced and cpu pixels differed by {worst}");
            assert!(cpu_px != [0; 4], "the model drew no colour");
        });
    }

    #[test]
    #[ignore = "daily"]
    fn a_partial_tint_alpha_matches_on_tinted_faces_only() {
        let _fixture =
            sunrise_client::models::standalone_fixture(vec!["minecraft:block/grass_block".into()]);
        super::super::native_entity_refs::tests::with_client_game(|game| {
            let _restore = Restore;
            let model = grass(game);
            let local = Mat4::from_scale(Vec3::splat(1.0));
            let origin = DVec3::new(8_000_000.5, 70.0, -4_000_000.25);
            let camera = DVec3::new(8_000_000.0, 70.2, -4_000_000.0);
            let lit = ModelInstance::new(
                origin.to_array(),
                rows(local),
                0x40FF_3040,
                LightCoords::new(3, 15).unwrap(),
                Overlay::NONE,
            );
            let view = Mat4::from_scale_rotation_translation(
                Vec3::new(1.4, 1.4, 0.2),
                Quat::IDENTITY,
                Vec3::new(-0.55, -0.85, 0.5),
            );
            sunrise_client::gfx::cap_prepared_instance_slots(8);
            let mut draw = WorldGeometry::default();
            draw.reset(camera.to_array(), Mat4::IDENTITY, [0.0, 0.0]);
            context(game, &mut draw)
                .draw_model_instances(model, &[lit])
                .unwrap();
            let mut spare = Vec::new();
            game.commit_model_instances(&mut draw, camera, &mut spare);
            let staged = game.client.prepared_model_draws.borrow();
            let instance = staged[0].instance;
            let slot = staged[0].slot;
            let uploaded = staged[0].generation;
            drop(staged);
            game.client.prepared_model_draws.borrow_mut().clear();
            sunrise_client::gfx::set_test_prepared_generation(None);
            let mut cpu_draw = WorldGeometry::default();
            cpu_draw.reset(camera.to_array(), Mat4::IDENTITY, [0.0, 0.0]);
            context(game, &mut cpu_draw)
                .draw_model_instances(model, &[lit])
                .unwrap();
            let mut entities = Vec::new();
            game.commit_model_instances(&mut cpu_draw, camera, &mut entities);
            assert!(
                entities.iter().any(|vertex| vertex.color >> 24 == 255),
                "an untinted face stays opaque"
            );
            assert!(
                entities.iter().any(|vertex| vertex.color >> 24 == 0x40),
                "a tinted face takes the tint alpha"
            );
            let (worst, cpu_px, gpu_px) =
                worst_pixels(game, &entities, &instance, slot, uploaded, view);
            eprintln!("partial worst {worst} cpu {cpu_px:?} gpu {gpu_px:?}");
            assert_eq!(worst, 0, "partial tint alpha differed by {worst}");
        });
    }

    #[test]
    #[ignore = "daily"]
    fn translucent_and_hidden_copies_take_the_cpu_routes() {
        let _fixture =
            sunrise_client::models::standalone_fixture(vec!["minecraft:block/grass_block".into()]);
        super::super::native_entity_refs::tests::with_client_game(|game| {
            let _restore = Restore;
            let model = grass(game);
            let camera = DVec3::new(0.0, 70.0, 0.0);
            let at = |x: f64| ModelInstance::at([x, 70.0, 0.0], 0xFFFF_FFFF);
            let mut draw = WorldGeometry::default();
            draw.reset(camera.to_array(), Mat4::IDENTITY, [0.0, 0.0]);
            // A near and a far translucent copy and an opaque copy with
            // every face hidden that draws nothing.
            context(game, &mut draw)
                .draw_model_instances(
                    model,
                    &[
                        at(3.0).with_color(0x80FF_FFFF),
                        at(9.0).with_color(0x80FF_FFFF),
                        at(6.0).with_hidden_faces(0b11_1111),
                    ],
                )
                .unwrap();
            let mut entities = Vec::new();
            game.commit_model_instances(&mut draw, camera, &mut entities);
            assert!(
                game.client.prepared_model_draws.borrow().is_empty(),
                "no GPU copy"
            );
            assert!(!entities.is_empty());
            assert!(entities.iter().all(
                |v| v.layer & 0xFFFF == sunrise_client::gfx::TERRAIN_MODEL_TRANSLUCENT_OVERLAY
            ));
            let mean_x = |verts: &[Vertex]| {
                verts.iter().map(|vertex| vertex.pos[0]).sum::<f32>() / verts.len() as f32
            };
            // The two translucent copies' quads and none of the fully hidden copy's.
            let slot = mod_api::__private::game::Fields::fields(&model).slot;
            let faces = game
                .client
                .standalone_geometry
                .faces(slot)
                .expect("prepared faces")
                .len();
            assert_eq!(entities.len(), faces * 2 * 4);
            // In submission order (the far-to-near sort is the renderer's): the cube at x 3
            // spans 3..4, mean 3.5, and the one at x 9 spans 9..10, mean 9.5.
            let half = entities.len() / 2;
            assert_eq!(mean_x(&entities[..half]), 3.5);
            assert_eq!(mean_x(&entities[half..]), 9.5);
            assert!(entities.iter().all(|v| v.color >> 24 == 0x80));
        });
    }

    #[test]
    #[ignore = "daily"]
    fn draw_model_instances_bakes_overflow_after_the_instanced_prefix() {
        let _fixture =
            sunrise_client::models::standalone_fixture(vec!["minecraft:block/grass_block".into()]);
        super::super::native_entity_refs::tests::with_client_game(|game| {
            let _restore = Restore;
            let model = grass(game);
            sunrise_client::gfx::cap_prepared_instance_slots(1);
            let camera = DVec3::new(8_000_000.0, 70.0, -4_000_000.0);
            let copy = |x: f64| {
                ModelInstance::new(
                    [camera.x + x, camera.y, camera.z],
                    rows(Mat4::IDENTITY),
                    0xFFFF_FFFF,
                    LightCoords::FULL_BRIGHT,
                    Overlay::NONE,
                )
            };
            let mut draw = WorldGeometry::default();
            draw.reset(camera.to_array(), Mat4::IDENTITY, [0.0, 0.0]);
            context(game, &mut draw)
                .draw_model_instances(model, &[copy(0.0), copy(3.0), copy(6.0)])
                .unwrap();
            let mut entities = Vec::new();
            game.commit_model_instances(&mut draw, camera, &mut entities);
            let staged = game.client.prepared_model_draws.borrow();
            assert_eq!(staged.len(), 1, "only the first copy fits the room");
            assert_eq!(staged[0].cpu_vertices, 0, "baked copies follow the prefix");
            let placed = staged[0].instance.model[3][0];
            assert!(
                placed.abs() < 1.5,
                "the instanced copy must be the first one, got {placed}"
            );
            let slot = staged[0].slot;
            drop(staged);
            let faces = game
                .client
                .standalone_geometry
                .faces(slot)
                .expect("prepared faces")
                .len();
            assert_eq!(
                entities.len(),
                faces * 8,
                "both overflow copies must be baked"
            );
            let mean_x = |verts: &[Vertex]| {
                verts.iter().map(|vertex| vertex.pos[0]).sum::<f32>() / verts.len() as f32
            };
            let half = entities.len() / 2;
            let first = mean_x(&entities[..half]);
            let second = mean_x(&entities[half..]);
            assert!(
                (first - 3.0).abs() < 1.5,
                "first overflow copy, got {first}"
            );
            assert!(
                (second - 6.0).abs() < 1.5,
                "second overflow copy, got {second}"
            );
        });
    }

    #[test]
    #[ignore = "daily"]
    fn draw_model_instances_culls_skips_invalid_and_grows_past_initial_hint() {
        let _fixture =
            sunrise_client::models::standalone_fixture(vec!["minecraft:block/grass_block".into()]);
        super::super::native_entity_refs::tests::with_client_game(|game| {
            let _restore = Restore;
            let model = grass(game);
            sunrise_client::gfx::cap_prepared_instance_slots(16_384);
            game.client.view_frustum = Some(sunrise_client::gfx::test_frustum([
                glam::Vec4::new(-1.0, 0.0, 0.0, 4.0),
                glam::Vec4::W,
                glam::Vec4::W,
                glam::Vec4::W,
                glam::Vec4::W,
                glam::Vec4::W,
            ]));
            let mut draw = WorldGeometry::default();
            draw.reset([0.0; 3], Mat4::IDENTITY, [0.0, 0.0]);
            let near = ModelInstance::at([1.0, 0.0, 0.0], 0xFFFF_FFFF);
            let far = ModelInstance::at([40.0, 0.0, 0.0], 0xFFFF_FFFF);
            context(game, &mut draw)
                .draw_model_instances(model, &[far, near])
                .unwrap();
            assert_eq!(
                game.client.prepared_model_draws.borrow().len(),
                1,
                "only the visible copy is staged"
            );
            let mut committed = Vec::new();
            game.commit_model_instances(&mut draw, DVec3::ZERO, &mut committed);
            assert_eq!(draw.copy_count(), 0, "commit takes the copies");
            assert!(committed.is_empty(), "the staged copy is not also baked");
            game.client.prepared_model_draws.borrow_mut().clear();
            draw.reset([0.0; 3], Mat4::IDENTITY, [0.0, 0.0]);
            {
                let mut ctx = context(game, &mut draw);
                ctx.draw_model_instances(model, &[near]).unwrap();
                let mut bad = near;
                bad.light.block = 16;
                // Invalid visible copies are skipped individually; valid copies stay staged.
                ctx.draw_model_instances(model, &[near, bad]).unwrap();
                assert_eq!(game.client.prepared_model_draws.borrow().len(), 2);
                // Copies past the GPU slot cap fall back to CPU baking instead of failing.
                ctx.draw_model_instances(model, &vec![near; 16_384])
                    .unwrap();
                assert_eq!(game.client.prepared_model_draws.borrow().len(), 16_384);
                assert_eq!(ctx.surface.copy_count(), 2);
                game.client.prepared_model_draws.borrow_mut().clear();
                ctx.surface.reset([0.0; 3], Mat4::IDENTITY, [0.0, 0.0]);
                // Large off-screen slices cull cleanly without error.
                ctx.draw_model_instances(model, &vec![far; 16_385]).unwrap();
                assert_eq!(game.client.prepared_model_draws.borrow().len(), 0);
                assert_eq!(ctx.surface.copy_count(), 0);
            }
        });
    }

    static STAGES_ARMED: AtomicBool = AtomicBool::new(false);
    static STAGE_ORDER: Mutex<Vec<&'static str>> = Mutex::new(Vec::new());

    fn test_camera(position: [f64; 3], view: Mat4) -> mod_api::Camera {
        mod_api::Camera {
            position,
            eye_height: 1.62,
            partial_tick: 0.0,
            yaw: 0.0,
            pitch: 0.0,
            roll: 0.0,
            fov_degrees: 70.0,
            near: 0.05,
            far: 128.0,
            view: view.to_cols_array(),
            projection: Mat4::IDENTITY.to_cols_array(),
            dimension: "minecraft:overworld".into(),
        }
    }

    /// A line one block long at `x`, so each listener's output can be told apart.
    fn marker(x: f64) -> mod_api::WorldLine {
        mod_api::WorldLine::new(
            [x, 0.0, -1.0],
            [x + 1.0, 0.0, -1.0],
            0xFF00_FF00,
            1.0,
            mod_api::WorldDepth::Tested,
        )
    }

    /// Draws in `submit_custom_geometry`, `DebugRenderer`, standard pass stages, and OIT
    /// sub-passes while checking that `AfterSky` and `AfterLevel` provide `StagePass::None`.
    struct Staged(std::sync::Mutex<Vec<(RenderLevelStage, mod_api::PreparedWorldDraw)>>);
    #[mod_api::__subscriptions]
    impl Hooks for Staged {
        fn register_debug_renderers(
            &self,
            _: &mod_api::RegistrationContext,
            event: &mut mod_api::RegisterDebugRenderersEvent<'_>,
        ) {
            event
                .renderers
                .push(mod_api::DebugRenderer::new(|_, _, draw| {
                    if STAGES_ARMED.load(Ordering::Relaxed) {
                        STAGE_ORDER.lock().unwrap().push("debug_renderer");
                        draw.lines(&[marker(35.0)]).unwrap();
                    }
                }));
        }
        fn prepare_render_buffers(
            &self,
            _: &dyn ClientHost,
            event: &mut mod_api::PrepareRenderBuffersEvent<'_>,
        ) {
            if STAGES_ARMED.load(Ordering::Relaxed) {
                assert_eq!(event.view.sections.len(), 1);
                assert_eq!(event.view.sections[0].origin, [0, 64, 0]);
                STAGE_ORDER.lock().unwrap().push("prepare");
                let mut batches = self.0.lock().unwrap();
                batches.clear();
                use mod_api::{OitStage, RenderLevelStage::*};
                for (stage, x) in [(AfterOpaqueBlocks, 10.0), (AfterOpaqueFeatures, 15.0)]
                    .into_iter()
                    .chain(
                        if event.view.improved_transparency {
                            [
                                (OitTranslucent(OitStage::DepthBounds), 100.0),
                                (OitTranslucent(OitStage::Accumulate), 200.0),
                            ]
                            .as_slice()
                        } else {
                            [(AfterWeather, 20.0)].as_slice()
                        }
                        .iter()
                        .copied(),
                    )
                {
                    batches.push((stage, event.context.lines(stage, &[marker(x)]).unwrap()));
                }
            }
        }
        fn submit_custom_geometry(
            &self,
            _: &dyn ClientHost,
            event: &mut SubmitCustomGeometryEvent<'_>,
        ) {
            if STAGES_ARMED.load(Ordering::Relaxed) {
                STAGE_ORDER.lock().unwrap().push("submit");
                event.draw.lines(&[marker(30.0)]).unwrap();
            }
        }
        fn render_level_stage(
            &self,
            _: &dyn ClientHost,
            event: &mut mod_api::RenderLevelStageEvent<'_>,
        ) {
            if !STAGES_ARMED.load(Ordering::Relaxed) {
                return;
            }
            let label = match event.stage {
                RenderLevelStage::AfterSky => "after_sky",
                RenderLevelStage::AfterOpaqueBlocks => "after_opaque_blocks",
                RenderLevelStage::AfterOpaqueFeatures => "after_opaque_features",
                RenderLevelStage::AfterTranslucentFeatures => "after_translucent_features",
                RenderLevelStage::AfterTranslucentBlocks => "after_translucent_blocks",
                RenderLevelStage::AfterTranslucentParticles => "after_translucent_particles",
                RenderLevelStage::AfterWeather => "after_weather",
                RenderLevelStage::OitTranslucent(mod_api::OitStage::DepthBounds) => {
                    "oit_depth_bounds"
                }
                RenderLevelStage::OitTranslucent(mod_api::OitStage::Transmittance) => {
                    "oit_transmittance"
                }
                RenderLevelStage::OitTranslucent(mod_api::OitStage::Accumulate) => "oit_accumulate",
                RenderLevelStage::AfterLevel => "after_level",
                _ => "other",
            };
            STAGE_ORDER.lock().unwrap().push(label);
            match event.stage {
                RenderLevelStage::AfterSky | RenderLevelStage::AfterLevel => {
                    assert!(!event.context.has_render_pass());
                    assert!(matches!(event.context.pass(), mod_api::StagePass::None));
                    assert!(event.context.world_pass().is_none());
                    assert!(event.context.oit_pass().is_none());
                }
                RenderLevelStage::AfterOpaqueBlocks => {
                    assert!(event.context.has_render_pass());
                    assert!(event.context.oit_pass().is_none());
                    event
                        .context
                        .world_pass()
                        .unwrap()
                        .draw(
                            self.0
                                .lock()
                                .unwrap()
                                .iter()
                                .find(|(stage, _)| *stage == event.stage)
                                .unwrap()
                                .1,
                        )
                        .unwrap();
                }
                RenderLevelStage::AfterOpaqueFeatures => {
                    assert!(event.context.has_render_pass());
                    event
                        .context
                        .world_pass()
                        .unwrap()
                        .draw(
                            self.0
                                .lock()
                                .unwrap()
                                .iter()
                                .find(|(stage, _)| *stage == event.stage)
                                .unwrap()
                                .1,
                        )
                        .unwrap();
                }
                RenderLevelStage::AfterWeather => {
                    assert!(event.context.has_render_pass());
                    event
                        .context
                        .world_pass()
                        .unwrap()
                        .draw(
                            self.0
                                .lock()
                                .unwrap()
                                .iter()
                                .find(|(stage, _)| *stage == event.stage)
                                .unwrap()
                                .1,
                        )
                        .unwrap();
                }
                RenderLevelStage::OitTranslucent(expected_oit) => {
                    assert!(event.context.has_render_pass());
                    assert!(event.context.world_pass().is_none());
                    let (oit, pass) = event.context.oit_pass().unwrap();
                    assert_eq!(oit, expected_oit);
                    assert_eq!(pass.oit_stage(), expected_oit);
                    if let Some((_, batch)) = self
                        .0
                        .lock()
                        .unwrap()
                        .iter()
                        .find(|(stage, _)| *stage == event.stage)
                    {
                        pass.draw(*batch).unwrap();
                    }
                }
                _ => {
                    assert!(event.context.has_render_pass());
                    assert!(event.context.world_pass().is_some());
                }
            }
        }
    }

    #[test]
    #[ignore = "daily"]
    fn each_stage_is_asked_in_frame_order_and_its_lines_draw_there() {
        super::super::native_entity_refs::tests::with_client_game(|game| {
            mod_platform::__private::install(
                vec![ModEntry::new(
                    "staged",
                    Box::new(Staged(Default::default())),
                )]
                .into(),
            );
            game.refresh_debug_renderers();
            assert_eq!(game.client.debug_renderers.0.len(), 1);
            game.client.debug_renderers_version =
                Some(game.client.debug_entries.0.enabled_version());
            let mut draw = WorldGeometry::default();
            let frame_camera = test_camera([0.0; 3], Mat4::IDENTITY);
            let sections = [mod_api::RenderableSection::new(
                [0, 64, 0],
                [[0.0, 64.0, 0.0], [16.0, 80.0, 16.0]],
                false,
            )];

            // Classic transparency mode: submit -> DebugRenderer -> AfterSky -> prepare ->
            // 6 classic pass stages -> AfterLevel, with zero OIT calls.
            STAGE_ORDER.lock().unwrap().clear();
            STAGES_ARMED.store(true, Ordering::Relaxed);
            game.fill_world_geometry(
                false,
                Some(&frame_camera),
                [0.0; 3],
                Mat4::IDENTITY,
                0.0,
                false,
                &sections,
                &mut draw,
                &mut Vec::new(),
            );
            STAGES_ARMED.store(false, Ordering::Relaxed);
            assert_eq!(
                *STAGE_ORDER.lock().unwrap(),
                [
                    "submit",
                    "debug_renderer",
                    "after_sky",
                    "prepare",
                    "after_opaque_blocks",
                    "after_opaque_features",
                    "after_translucent_features",
                    "after_translucent_blocks",
                    "after_translucent_particles",
                    "after_weather",
                    "after_level",
                ],
                "classic frame order matches NeoForge 26.3"
            );
            let batch = draw.batch();
            let starts: Vec<f32> = batch.lines.iter().map(|line| line[0]).collect();
            assert_eq!(
                starts,
                [10.0, 15.0, 20.0, 30.0, 35.0],
                "pass stages in frame order, custom geometry and debug renderers appended at STAGE_CUSTOM_GEOMETRY"
            );
            assert_eq!(batch.stage_ends, [[0, 0, 3, 0]; 6]);
            assert!(batch.oit.is_empty());

            // OIT (improved transparency) mode: OIT sub-passes replace classic translucent/weather.
            STAGE_ORDER.lock().unwrap().clear();
            STAGES_ARMED.store(true, Ordering::Relaxed);
            game.fill_world_geometry(
                false,
                Some(&frame_camera),
                [0.0; 3],
                Mat4::IDENTITY,
                0.0,
                true,
                &sections,
                &mut draw,
                &mut Vec::new(),
            );
            STAGES_ARMED.store(false, Ordering::Relaxed);
            assert_eq!(
                *STAGE_ORDER.lock().unwrap(),
                [
                    "submit",
                    "debug_renderer",
                    "after_sky",
                    "prepare",
                    "after_opaque_blocks",
                    "after_opaque_features",
                    "oit_depth_bounds",
                    "oit_transmittance",
                    "oit_accumulate",
                    "after_level",
                ],
                "OIT frame order matches NeoForge 26.3"
            );
            let batch = draw.batch();
            let starts: Vec<f32> = batch.lines.iter().map(|line| line[0]).collect();
            assert_eq!(starts, [10.0, 15.0, 30.0, 35.0]);
            assert_eq!(batch.stage_ends, [[0, 0, 2, 0]; 6]);
            let oit_starts: Vec<f32> = batch.oit.lines.iter().map(|line| line[0]).collect();
            assert_eq!(oit_starts, [100.0, 200.0]);
            assert_eq!(batch.oit.stage_ends, [[0, 0, 2, 0]; 2]);

            // A title view draws no world: nothing, and every stage empty.
            game.fill_world_geometry(
                true,
                None,
                [0.0; 3],
                Mat4::IDENTITY,
                0.0,
                false,
                &[],
                &mut draw,
                &mut Vec::new(),
            );
            assert!(draw.batch().is_empty());
            assert_eq!(draw.batch().stage_ends, [[0; 4]; 6]);
            assert_eq!(draw.batch().oit.stage_ends, [[0; 4]; 2]);
            game.client.debug_renderers.0.clear();
            game.client.debug_renderers_version = None;
        });
    }

    static VIEW_CHECK_ARMED: AtomicBool = AtomicBool::new(false);
    static VIEW_EXPECT_OIT: AtomicBool = AtomicBool::new(false);
    static VIEW_CHECK_HITS: AtomicUsize = AtomicUsize::new(0);
    static PREPARE_MODEL: Mutex<Option<ModelRef>> = Mutex::new(None);
    static PREPARE_GROUP: Mutex<Option<mod_api::InstanceGroupRef>> = Mutex::new(None);
    static PREPARE_STEP: AtomicUsize = AtomicUsize::new(0);
    static PREPARE_EVENTS: Mutex<Vec<&'static str>> = Mutex::new(Vec::new());

    fn assert_expected_world_view(view: &mod_api::WorldRenderView<'_>) {
        let expected_view = Mat4::from_translation(Vec3::new(1.0, -2.0, 3.0)).to_cols_array();
        let expected_proj = Mat4::from_scale(Vec3::new(0.5, 0.75, 1.25)).to_cols_array();
        assert_eq!(view.camera.position, [12.5, 71.25, -44.75]);
        assert_eq!(view.camera_position(), [12.5, 71.25, -44.75]);
        assert_eq!(view.camera.eye_height, 1.62);
        assert_eq!(view.camera.partial_tick, 0.25);
        assert_eq!(view.camera.yaw, 33.5);
        assert_eq!(view.camera.pitch, -12.0);
        assert_eq!(view.camera.fov_degrees, 85.0);
        assert_eq!(view.camera.near, 0.05);
        assert_eq!(view.camera.far, 256.0);
        assert_eq!(view.camera.view, expected_view);
        assert_eq!(view.model_view_matrix(), expected_view);
        assert_eq!(view.camera.projection, expected_proj);
        assert_eq!(view.projection_matrix(), expected_proj);
        assert_eq!(view.camera.dimension, "minecraft:the_nether");
        assert_eq!(view.partial_tick, 0.75);
        assert_eq!(view.game_time, 12_345);
        assert_eq!(
            view.improved_transparency,
            VIEW_EXPECT_OIT.load(Ordering::Relaxed)
        );
        assert_eq!(
            view.sections,
            [
                mod_api::RenderableSection::new(
                    [16, 64, -32],
                    [[16.0, 64.0, -32.0], [32.0, 80.0, -16.0]],
                    false,
                ),
                mod_api::RenderableSection::new(
                    [32, 64, -32],
                    [[32.0, 64.0, -32.0], [48.0, 80.0, -16.0]],
                    true,
                ),
            ]
        );
        VIEW_CHECK_HITS.fetch_add(1, Ordering::Relaxed);
    }

    struct PrepareViewMod;
    #[mod_api::__subscriptions]
    impl Hooks for PrepareViewMod {
        fn register_debug_renderers(
            &self,
            _: &mod_api::RegistrationContext,
            event: &mut mod_api::RegisterDebugRenderersEvent<'_>,
        ) {
            event
                .renderers
                .push(mod_api::DebugRenderer::new(|_, view, _| {
                    if VIEW_CHECK_ARMED.load(Ordering::Relaxed) {
                        assert_expected_world_view(view);
                    }
                }));
        }
        fn submit_custom_geometry(
            &self,
            _: &dyn ClientHost,
            event: &mut SubmitCustomGeometryEvent<'_>,
        ) {
            if !VIEW_CHECK_ARMED.load(Ordering::Relaxed) {
                return;
            }
            assert_expected_world_view(event.view);
            if let Some(group) = *PREPARE_GROUP.lock().unwrap() {
                event.draw.draw_instance_group(group).unwrap();
            }
        }
        fn prepare_render_buffers(
            &self,
            host: &dyn ClientHost,
            event: &mut mod_api::PrepareRenderBuffersEvent<'_>,
        ) {
            if !VIEW_CHECK_ARMED.load(Ordering::Relaxed) {
                return;
            }
            assert_expected_world_view(event.view);
            PREPARE_EVENTS.lock().unwrap().push("prepare");
            let mut slot = PREPARE_GROUP.lock().unwrap();
            let group = match *slot {
                Some(group) => group,
                None => {
                    let model = PREPARE_MODEL.lock().unwrap().unwrap();
                    let created = host.instance_groups().create_group(model, 4).unwrap();
                    *slot = Some(created);
                    created
                }
            };
            let step = PREPARE_STEP.fetch_add(1, Ordering::Relaxed);
            let copy0 = ModelInstance::at([13.0, 71.0, -45.0], 0xFF80_C040);
            let copy1 = ModelInstance::at([14.0, 71.0, -45.0], 0xFF40_80C0);
            if step == 0 {
                // Frame 1: group was submitted in `submit_custom_geometry` with len 0 and identity
                // transform; updating records, group transform, and len here must take effect on
                // Frame 1 itself (not one frame late).
                group.write(host, 0, &[copy0]).unwrap();
                group
                    .set_group_transform(
                        host,
                        [1.0, 0.0, 0.0, 0.5, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0],
                    )
                    .unwrap();
                group.set_len(host, 1).unwrap();
            } else {
                // Frame 2: grow len to 2 and shift group transform again; Frame 2 must see both.
                group.write(host, 0, &[copy0, copy1]).unwrap();
                group
                    .set_group_transform(
                        host,
                        [1.0, 0.0, 0.0, 1.25, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0],
                    )
                    .unwrap();
                group.set_len(host, 2).unwrap();
            }
        }
        fn render_level_stage(
            &self,
            _: &dyn ClientHost,
            event: &mut mod_api::RenderLevelStageEvent<'_>,
        ) {
            if !VIEW_CHECK_ARMED.load(Ordering::Relaxed) {
                return;
            }
            assert_expected_world_view(event.view);
            if event.stage == RenderLevelStage::AfterOpaqueBlocks {
                PREPARE_EVENTS.lock().unwrap().push("after_opaque_blocks");
            }
        }
    }

    #[test]
    #[ignore = "daily"]
    fn world_render_view_lends_frame_state_and_prepare_updates_instance_groups() {
        if super::super::native_entity_refs::tests::run_fresh_cases(
            "game::client_side::native_custom_geometry::tests::world_render_view_lends_frame_state_and_prepare_updates_instance_groups",
            "SUNRISE_PREPARE_WORLD_VIEW_CHILD",
            &["same-frame-groups"],
        ) {
            return;
        }
        let _fixture =
            sunrise_client::models::standalone_fixture(vec!["minecraft:block/grass_block".into()]);
        super::super::native_entity_refs::tests::with_client_game(|game| {
            let _restore = Restore;
            let model = grass(game);
            sunrise_client::gfx::cap_prepared_instance_slots(16);
            game.client.view_frustum = None;
            game.common.world.game_time = 12_345;
            *PREPARE_MODEL.lock().unwrap() = Some(model);
            *PREPARE_GROUP.lock().unwrap() = None;
            PREPARE_EVENTS.lock().unwrap().clear();

            mod_platform::__private::install(
                vec![ModEntry::new("prepare_view", Box::new(PrepareViewMod))].into(),
            );
            game.refresh_debug_renderers();
            assert_eq!(game.client.debug_renderers.0.len(), 1);
            game.client.debug_renderers_version =
                Some(game.client.debug_entries.0.enabled_version());

            let camera_view = Mat4::from_translation(Vec3::new(1.0, -2.0, 3.0));
            let camera_proj = Mat4::from_scale(Vec3::new(0.5, 0.75, 1.25));
            let frame_camera = mod_api::Camera {
                position: [12.5, 71.25, -44.75],
                eye_height: 1.62,
                partial_tick: 0.25,
                yaw: 33.5,
                pitch: -12.0,
                roll: 0.0,
                fov_degrees: 85.0,
                near: 0.05,
                far: 256.0,
                view: camera_view.to_cols_array(),
                projection: camera_proj.to_cols_array(),
                dimension: "minecraft:the_nether".into(),
            };
            let sections = [
                mod_api::RenderableSection::new(
                    [16, 64, -32],
                    [[16.0, 64.0, -32.0], [32.0, 80.0, -16.0]],
                    false,
                ),
                mod_api::RenderableSection::new(
                    [32, 64, -32],
                    [[32.0, 64.0, -32.0], [48.0, 80.0, -16.0]],
                    true,
                ),
            ];
            let mut draw = WorldGeometry::default();
            PREPARE_STEP.store(0, Ordering::Relaxed);
            // Pre-create an empty group before Frame 1 so Frame 1 submits it in
            // `submit_custom_geometry`, then mutates its records/transform/len in
            // `prepare_render_buffers` and immediately sees the updated instances on Frame 1.
            {
                let _owner = mod_platform::content::crash::enter_native_mod(
                    mod_api::content::crash::ModCall {
                        id: "prepare_view",
                        hook: "test",
                        part: None,
                    },
                );
                let created = game.create_group(model, 4).unwrap();
                created.set_len(game, 0).unwrap();
                *PREPARE_GROUP.lock().unwrap() = Some(created);
            }

            // Frame 1 (classic): verifies every WorldRenderView field across all hooks and confirms
            // same-frame visibility of transform/len updates made in `prepare_render_buffers`.
            game.client.prepared_model_draws.borrow_mut().clear();
            VIEW_EXPECT_OIT.store(false, Ordering::Relaxed);
            VIEW_CHECK_HITS.store(0, Ordering::Relaxed);
            VIEW_CHECK_ARMED.store(true, Ordering::Relaxed);
            game.fill_world_geometry(
                false,
                Some(&frame_camera),
                frame_camera.position,
                camera_view,
                0.75,
                false,
                &sections,
                &mut draw,
                &mut Vec::new(),
            );
            VIEW_CHECK_ARMED.store(false, Ordering::Relaxed);
            assert_eq!(
                *PREPARE_EVENTS.lock().unwrap(),
                ["prepare", "after_opaque_blocks"],
                "prepare_render_buffers must run before pass-bearing stages"
            );
            // 1 submit + 1 debug_renderer + 1 prepare + 8 classic render_level_stage callbacks.
            assert_eq!(VIEW_CHECK_HITS.load(Ordering::Relaxed), 11);
            {
                let staged = game.client.prepared_model_draws.borrow();
                assert_eq!(
                    staged.len(),
                    1,
                    "Frame 1 must render the instance written and enabled in prepare_render_buffers"
                );
                // (origin [13.0, 71.0, -45.0] - camera [12.5, 71.25, -44.75]) + group translate [0.5, 0.0, 0.0]
                assert_eq!(staged[0].instance.model[3], [1.0, -0.25, -0.25, 1.0]);
            }

            // Frame 2 (OIT): updates len to 2 and group transform to +1.25 X in
            // `prepare_render_buffers` and verifies both instances stage on Frame 2 itself.
            game.client.prepared_model_draws.borrow_mut().clear();
            PREPARE_EVENTS.lock().unwrap().clear();
            VIEW_EXPECT_OIT.store(true, Ordering::Relaxed);
            VIEW_CHECK_HITS.store(0, Ordering::Relaxed);
            VIEW_CHECK_ARMED.store(true, Ordering::Relaxed);
            game.fill_world_geometry(
                false,
                Some(&frame_camera),
                frame_camera.position,
                camera_view,
                0.75,
                true,
                &sections,
                &mut draw,
                &mut Vec::new(),
            );
            VIEW_CHECK_ARMED.store(false, Ordering::Relaxed);
            assert_eq!(
                *PREPARE_EVENTS.lock().unwrap(),
                ["prepare", "after_opaque_blocks"]
            );
            // 1 submit + 1 debug_renderer + 1 prepare + 7 OIT render_level_stage callbacks.
            assert_eq!(VIEW_CHECK_HITS.load(Ordering::Relaxed), 10);
            {
                let staged = game.client.prepared_model_draws.borrow();
                assert_eq!(
                    staged.len(),
                    2,
                    "Frame 2 must render both updated instances from prepare_render_buffers"
                );
                // (origin [13.0, 71.0, -45.0] - camera [12.5, 71.25, -44.75]) + group translate [1.25, 0.0, 0.0]
                assert_eq!(staged[0].instance.model[3], [1.75, -0.25, -0.25, 1.0]);
                // (origin [14.0, 71.0, -45.0] - camera [12.5, 71.25, -44.75]) + group translate [1.25, 0.0, 0.0]
                assert_eq!(staged[1].instance.model[3], [2.75, -0.25, -0.25, 1.0]);
            }

            if let Some(group) = PREPARE_GROUP.lock().unwrap().take() {
                let _owner = mod_platform::content::crash::enter_native_mod(
                    mod_api::content::crash::ModCall {
                        id: "prepare_view",
                        hook: "test",
                        part: None,
                    },
                );
                group.destroy(game).unwrap();
            }
            *PREPARE_MODEL.lock().unwrap() = None;
            game.client.debug_renderers.0.clear();
            game.client.debug_renderers_version = None;
        });
    }

    #[test]
    #[ignore = "daily"]
    fn fill_world_geometry_commits_an_accepted_batch() {
        let _fixture =
            sunrise_client::models::standalone_fixture(vec!["minecraft:block/grass_block".into()]);
        super::super::native_entity_refs::tests::with_client_game(|game| {
            let _restore = Restore;
            let model = grass(game);
            sunrise_client::gfx::cap_prepared_instance_slots(8);
            *FILL_MODEL.lock().unwrap() = Some(model);
            mod_platform::__private::install(
                vec![ModEntry::new("batch", Box::new(FillBatch))].into(),
            );
            FILL_ARMED.store(true, Ordering::Relaxed);
            let mut draw = WorldGeometry::default();
            let mut entities = vec![<Vertex as bytemuck::Zeroable>::zeroed(); 4];
            let frame_camera = test_camera([0.0; 3], Mat4::IDENTITY);
            game.fill_world_geometry(
                false,
                Some(&frame_camera),
                [0.0; 3],
                Mat4::IDENTITY,
                0.0,
                false,
                &[],
                &mut draw,
                &mut entities,
            );
            FILL_ARMED.store(false, Ordering::Relaxed);
            assert_eq!(
                game.client.prepared_model_draws.borrow().len(),
                1,
                "fill_world_geometry must commit the accepted copy"
            );
            assert_eq!(draw.copy_count(), 0);
            assert_eq!(entities.len(), 4);
            assert_eq!(game.client.prepared_model_draws.borrow()[0].cpu_vertices, 4);
        });
    }

    #[test]
    fn shared_device_quad_capacity_bounds_fluid_terrain_models_and_cpu_model_copies() {
        use mod_api::{FluidBoxOptions, FluidStack, TerrainModelOptions};
        use sunrise_assets::{
            bake::{RenderType, bake_flat_face},
            model::{Direction, Material},
        };
        use sunrise_client::{
            block_entity_render::diffuse_lights,
            gfx::{EntityQuadCapacity, EntityQuadScope},
            models::catalog::BakedFace,
        };

        let make_baked = || {
            let mut quad = bake_flat_face(
                [0.0; 3],
                [16.0; 3],
                [0.0, 0.0, 16.0, 16.0],
                Direction::South,
                Material {
                    sprite: "test:casing".into(),
                    force_translucent: false,
                },
                -1,
            )
            .unwrap();
            quad.render_type = Some(RenderType::Cutout);
            BakedFace::new(quad, 7, true, false, None)
        };
        let sample = make_baked();
        let prepared_face = crate::game::client_side::minecarts_client::PreparedModelFace::new(
            &sample.quad,
            sunrise_client::hud::Rect {
                u0: 0.0,
                v0: 0.0,
                u1: 1.0,
                v1: 1.0,
                w: 16,
                h: 16,
            },
        );

        // Small device-derived capacity: 7 total quads, with 2 already reserved by other streams
        // in the same combined batch => 5 quads of budget shared across all draw paths.
        let capacity =
            EntityQuadCapacity::from_max_buffer_size(EntityQuadCapacity::VERTEX_QUAD_BYTES * 7);
        let reserved = 2usize;

        // 1) Terrain model submits 1 quad into `entities`.
        let mut draw = super::super::native_block_entity_draw::BlockEntityDraw::posed(
            0.0,
            0,
            0xF0F0,
            [0; 3],
            Mat4::IDENTITY,
            diffuse_lights(false),
            Vec::new(),
        )
        .with_quad_budget(capacity, reserved);
        super::super::native_terrain_model::draw_terrain_model(
            &mut draw,
            &[make_baked()],
            |_| Some(7),
            TerrainModelOptions::default(),
            |_, _, _| {},
        );
        let mut entities = draw.into_vertices();
        assert_eq!(entities.len(), 4, "1 terrain model quad admitted");

        // 2) Opaque CPU model copy adds 1 quad through the same device-derived capacity.
        submit_opaque_model_copy(
            &mut entities,
            capacity,
            reserved,
            &[prepared_face],
            rows(Mat4::IDENTITY),
            Vec3::ZERO,
            0xF0F0,
            0xFFFF_FFFF,
            u32::MAX,
            0,
            0,
        );
        assert_eq!(entities.len(), 8, "1 opaque CPU model copy quad admitted");

        // 3) Material CPU model copy adds 2 quads through EntityQuadScope.
        {
            let mut scope = EntityQuadScope::new(&mut entities, capacity, reserved);
            submit_material_model_copy(
                &mut scope,
                &[make_baked(), make_baked()],
                Mat4::IDENTITY,
                0xF0F0,
                terrain_model::NativeModelRenderOptions::default(),
                |_| Some(7),
            );
        }
        assert_eq!(
            entities.len(),
            16,
            "2 material CPU model copy quads admitted"
        );

        // 4) Fluid box receives the final 1 quad of remaining budget and stops at whole quads.
        let water = FluidStack::new(mod_api::__private::parts_installation::fluid_key(0), 1000);
        let look =
            super::super::native_fluid_render::fluid_box_look("minecraft:water", None, 0, |_| {
                Some(7)
            })
            .unwrap();
        let mut draw = super::super::native_block_entity_draw::BlockEntityDraw::posed(
            0.0,
            0,
            0xF0F0,
            [0; 3],
            Mat4::IDENTITY,
            diffuse_lights(false),
            entities,
        )
        .with_quad_budget(capacity, reserved);
        super::super::native_fluid_render::draw_fluid_box(
            &mut draw,
            look,
            &water,
            [0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
            FluidBoxOptions::new(true, 0.0),
        );
        let mut entities = draw.into_vertices();
        assert_eq!(
            entities.len(),
            20,
            "fluid fills the 5th and final quad of budget (7 max - 2 reserved)"
        );

        // 5) Once at capacity, additional opaque/material CPU model copies add zero vertices.
        submit_opaque_model_copy(
            &mut entities,
            capacity,
            reserved,
            &[prepared_face],
            rows(Mat4::IDENTITY),
            Vec3::ZERO,
            0xF0F0,
            0xFFFF_FFFF,
            u32::MAX,
            0,
            0,
        );
        {
            let mut scope = EntityQuadScope::new(&mut entities, capacity, reserved);
            submit_material_model_copy(
                &mut scope,
                &[make_baked()],
                Mat4::IDENTITY,
                0xF0F0,
                terrain_model::NativeModelRenderOptions::default(),
                |_| Some(7),
            );
        }
        assert_eq!(entities.len(), 20);
    }

    struct RenderFault {
        panics: bool,
        batch: Mutex<Option<mod_api::PreparedWorldDraw>>,
    }

    #[mod_api::__subscriptions]
    impl Hooks for RenderFault {
        fn register_debug_renderers(
            &self,
            _: &mod_api::RegistrationContext,
            event: &mut mod_api::RegisterDebugRenderersEvent<'_>,
        ) {
            if self.panics {
                event
                    .renderers
                    .push(mod_api::DebugRenderer::new(|_, _, draw| {
                        draw.lines(&[marker(50.0)]).unwrap();
                        panic!("debug renderer failed");
                    }));
                event
                    .renderers
                    .push(mod_api::DebugRenderer::new(|_, _, draw| {
                        draw.lines(&[marker(60.0)]).unwrap()
                    }));
            }
        }

        fn submit_custom_geometry(
            &self,
            _: &dyn ClientHost,
            event: &mut SubmitCustomGeometryEvent<'_>,
        ) {
            let x = if self.panics { 30.0 } else { 40.0 };
            event.draw.lines(&[marker(x)]).unwrap();
        }

        fn prepare_render_buffers(
            &self,
            _: &dyn ClientHost,
            event: &mut mod_api::PrepareRenderBuffersEvent<'_>,
        ) {
            let x = if self.panics { 10.0 } else { 20.0 };
            *self.batch.lock().unwrap() = Some(
                event
                    .context
                    .lines(RenderLevelStage::AfterOpaqueBlocks, &[marker(x)])
                    .unwrap(),
            );
        }

        fn render_level_stage(
            &self,
            _: &dyn ClientHost,
            event: &mut mod_api::RenderLevelStageEvent<'_>,
        ) {
            if event.stage == RenderLevelStage::AfterOpaqueBlocks {
                let batch = self.batch.lock().unwrap().unwrap();
                event.context.world_pass().unwrap().draw(batch).unwrap();
                assert!(!self.panics, "world stage listener failed");
            }
        }
    }

    #[test]
    #[ignore = "daily"]
    fn render_panics_use_the_shared_boundary_without_rolling_back_draws() {
        use mod_api::__private::{HookPanicReport, take_hook_panics};
        use sunrise_client::gfx::world_primitives::FrameHooks;

        super::super::native_entity_refs::tests::with_client_game(|game| {
            mod_platform::__private::install(
                [("render_fault", true), ("render_healthy", false)]
                    .into_iter()
                    .map(|(id, panics)| {
                        ModEntry::new(
                            id,
                            Box::new(RenderFault {
                                panics,
                                batch: Mutex::new(None),
                            }),
                        )
                    })
                    .collect::<Vec<_>>()
                    .into(),
            );
            let camera = test_camera([0.0; 3], Mat4::IDENTITY);
            let mut draw = WorldGeometry::default();
            for (expected_requests, expected_lines) in [
                (&[0..1, 1..2][..], &[10.0, 20.0, 30.0, 40.0, 50.0, 60.0][..]),
                (
                    std::slice::from_ref(&(1..2)),
                    &[10.0, 20.0, 30.0, 40.0, 60.0][..],
                ),
            ] {
                // Refresh with fresh callback objects: enablement belongs to the callback
                // identity and the mod for the session, not to this list's current boxed closure.
                game.refresh_debug_renderers();
                assert_eq!(game.client.debug_renderers.0.len(), 2);

                draw.reset([0.0; 3], Mat4::IDENTITY, [0.0; 2]);
                let mut driver = GameWorldStageDriver::new(game, &camera, 0.0, &mut draw);
                driver.on_custom_geometry(&[], false);
                driver.on_prepare_render_buffers(&[], false, &mut Default::default());
                let requests =
                    driver.on_render_stage(RenderLevelStage::AfterOpaqueBlocks, &[], false);
                assert_eq!(
                    requests
                        .iter()
                        .map(|r| r.primitives[2].clone())
                        .collect::<Vec<_>>(),
                    expected_requests,
                );
                assert_eq!(
                    driver
                        .prepared_primitives()
                        .lines
                        .iter()
                        .map(|line| line[0])
                        .collect::<Vec<_>>(),
                    expected_lines,
                );
            }
            assert_eq!(
                take_hook_panics(),
                [
                    HookPanicReport {
                        mod_id: "render_fault",
                        hook: "debug_renderer_draw"
                    },
                    HookPanicReport {
                        mod_id: "render_fault",
                        hook: "render_level_stage"
                    },
                ],
            );
        });
    }

    #[test]
    fn compute_bindings_that_alias_a_writable_use_are_refused() {
        use mod_api::{
            BindEntry, BindGroupLayout, BindingKind, DrawBindings, StorageImageAccess,
            StorageImageBinding,
        };
        let buffer = |slot| mod_api::__private::built::storage_buffer_ref(1, slot);
        let slice = |slot| buffer(slot).slice(0, 16);
        let target = mod_api::__private::built::render_target_ref(1, 0);
        let storage = |name: &str, read_only| {
            BindEntry::new(
                name,
                BindingKind::StorageBuffer {
                    read_only,
                    min_size: 4,
                },
            )
        };
        let image = |name: &str, access| BindEntry::new(name, BindingKind::StorageImage { access });
        let layout = |entries: Vec<BindEntry>| {
            entries
                .into_iter()
                .fold(BindGroupLayout::builder(), |b, e| b.entry(e))
                .build()
                .unwrap()
        };
        // (a) one buffer twice, one use writable; two read-only uses and two buffers are fine.
        let mixed = layout(vec![storage("A", false), storage("B", true)]);
        let same = DrawBindings::default()
            .storage_buffer("A", slice(1))
            .storage_buffer("B", slice(1));
        assert!(compute_bindings_alias(&mixed, &same, None));
        let apart = DrawBindings::default()
            .storage_buffer("A", slice(1))
            .storage_buffer("B", slice(2));
        assert!(!compute_bindings_alias(&mixed, &apart, None));
        let readers = layout(vec![storage("A", true), storage("B", true)]);
        assert!(!compute_bindings_alias(&readers, &same, None));
        // (b) the indirect buffer is also bound writable; bound read-only it is fine.
        let writer = layout(vec![storage("A", false)]);
        let one = DrawBindings::default().storage_buffer("A", slice(1));
        assert!(compute_bindings_alias(&writer, &one, Some(buffer(1))));
        assert!(!compute_bindings_alias(&writer, &one, Some(buffer(2))));
        let reader = layout(vec![storage("A", true)]);
        assert!(!compute_bindings_alias(&reader, &one, Some(buffer(1))));
        // (c) one mip level in two entries with differing access; another mip is fine.
        let images = layout(vec![
            image("R", StorageImageAccess::ReadOnly),
            image("W", StorageImageAccess::WriteOnly),
        ]);
        let mip = |level| StorageImageBinding::new(target, 0, level);
        let clash = DrawBindings::default()
            .storage_image("R", mip(1))
            .storage_image("W", mip(1));
        assert!(compute_bindings_alias(&images, &clash, None));
        let other = DrawBindings::default()
            .storage_image("R", mip(0))
            .storage_image("W", mip(1));
        assert!(!compute_bindings_alias(&images, &other, None));
    }

    /// The handles one mod makes in `frame_graph_setup` and `prepare_render_buffers`, and releases in
    /// `render_level_stage`.
    #[derive(Default)]
    struct GpuHandles {
        target: Option<mod_api::RenderTargetRef>,
        mesh: Option<mod_api::MeshRef>,
        uniform: Option<mod_api::UniformBufferRef>,
        storage: Option<mod_api::StorageBufferRef>,
        pipeline: Option<RenderPipelineRef>,
        draw: Option<mod_api::PreparedWorldDraw>,
    }

    /// Reaches the GPU through the host in the three frame hooks, each call unwrapped: a refusal
    /// panics the hook, and the shared boundary reports it.
    struct GpuProbe(std::sync::Arc<Mutex<GpuHandles>>);

    #[mod_api::__subscriptions]
    impl Hooks for GpuProbe {
        fn register_render_pipelines(
            &self,
            _: &mod_api::RegistrationContext,
            event: &mut mod_api::RegisterRenderPipelinesEvent<'_>,
        ) {
            event.register(
                mod_api::RenderPipeline::new(
                    Identifier::parse("gpu_probe:sampled").unwrap(),
                    Identifier::parse("gpu_probe:core/sampled").unwrap(),
                    mod_api::PipelineBlend::Opaque,
                    mod_api::WorldDepth::ThroughWalls,
                )
                .sampler(
                    "ColorTex",
                    mod_api::Sampler::new(
                        mod_api::SamplerFilter::Nearest,
                        mod_api::SamplerAddress::ClampToEdge,
                    ),
                ),
            );
        }

        fn frame_graph_setup(
            &self,
            host: &dyn ClientHost,
            event: &mut mod_api::FrameGraphSetupEvent<'_>,
        ) {
            let desc = mod_api::RenderTargetDescriptor::new([8, 8])
                .color_target(mod_api::RenderTargetFormat::Rgba8Unorm);
            let target = host.create_render_target(&desc).unwrap();
            target.resize(host, [16, 16]).unwrap();
            assert_eq!(target.size(host), Ok([16, 16]));
            event
                .graph
                .add_pass(
                    mod_api::FramePassDescriptor::new(
                        "gpu_probe",
                        mod_api::FrameTarget::Mod(target),
                    ),
                    &mut |_| {},
                )
                .unwrap();
            self.0.lock().unwrap().target = Some(target);
        }

        fn prepare_render_buffers(
            &self,
            host: &dyn ClientHost,
            event: &mut mod_api::PrepareRenderBuffersEvent<'_>,
        ) {
            // One quad of the default `PositionTexColor` format: 24 bytes a vertex.
            let init = [mod_api::MeshBufferInit::from_bytes(24, &[0; 96])];
            let mesh = host
                .create_mesh(&mod_api::MeshDescriptor::new(&init))
                .unwrap();
            let uniform = host.create_uniform_buffer(&[0; 16]).unwrap();
            uniform.write(host, 0, &[1; 16]).unwrap();
            let storage = host
                .create_storage_buffer(&mod_api::StorageBufferDescriptor::new(64))
                .unwrap();
            storage.write_range(host, 0, &[1; 16]).unwrap();
            let pipeline = host
                .render_assets()
                .render_pipeline(&Identifier::parse("gpu_probe:sampled").unwrap())
                .unwrap();
            let target = self.0.lock().unwrap().target.unwrap();
            let draw = event
                .context
                .draw_mesh(
                    RenderLevelStage::AfterOpaqueFeatures,
                    pipeline,
                    mesh,
                    mod_api::MeshDraw::vertices(0..4),
                    &mod_api::DrawBindings::default().texture(
                        "ColorTex",
                        mod_api::TextureSource::RenderTargetColor(target, 0),
                    ),
                )
                .unwrap();
            let mut handles = self.0.lock().unwrap();
            handles.pipeline = Some(pipeline);
            handles.draw = Some(draw);
            handles.mesh = Some(mesh);
            handles.uniform = Some(uniform);
            handles.storage = Some(storage);
        }

        fn render_level_stage(
            &self,
            host: &dyn ClientHost,
            event: &mut mod_api::RenderLevelStageEvent<'_>,
        ) {
            if event.stage != RenderLevelStage::AfterOpaqueFeatures {
                return;
            }
            let handles = self.0.lock().unwrap();
            event
                .context
                .world_pass()
                .unwrap()
                .draw(handles.draw.unwrap())
                .unwrap();
            handles.mesh.unwrap().release(host).unwrap();
            handles.uniform.unwrap().release(host).unwrap();
            handles.storage.unwrap().release(host).unwrap();
            handles.target.unwrap().release(host).unwrap();
        }
    }

    /// `Game::render` takes the renderer out of the game for `Renderer::render`, so every frame hook
    /// runs with `game.client.renderer.0` empty: the mod GPU handles must keep working then, and refuse only
    /// once the game holds none.
    #[test]
    #[ignore = "daily"]
    fn mod_gpu_handles_work_while_the_renderer_is_out_of_the_game() {
        use mod_api::__private::take_hook_panics;
        use mod_api::{BufferError, MeshError, RenderTargetError};
        use sunrise_client::gfx::mod_pipelines::{PipelineDrawSource, ResolvedSet1Entry};
        use sunrise_client::gfx::world_primitives::FrameHooks;

        // The mod's pipeline registration is process-global and published once: run in a child.
        if super::super::native_entity_refs::tests::run_fresh_cases(
            "game::client_side::native_custom_geometry::tests::mod_gpu_handles_work_while_the_renderer_is_out_of_the_game",
            "SUNRISE_MOD_GPU_FRAME_CHILD",
            &["run"],
        ) {
            return;
        }
        super::super::native_entity_refs::tests::with_client_game(|game| {
            let (_event_loop, window) =
                crate::game::native_hooks::particle_count_tests::hidden_window();
            game.common.options.offscreen = true;
            let renderer = game.new_renderer(window.clone()).unwrap();
            let device_capacity = renderer.entity_quad_capacity();
            game.set_renderer(renderer);
            game.client.window.0 = Some(window);

            let handles = std::sync::Arc::new(Mutex::new(GpuHandles::default()));
            mod_platform::__private::install(
                vec![ModEntry::new(
                    "gpu_probe",
                    Box::new(GpuProbe(handles.clone())),
                )]
                .into(),
            );
            let _ = take_hook_panics();

            // The mod's sampler pipeline, registered and built from its GLSL as a resource load does.
            let mut pipelines = Vec::new();
            use mod_api::__private::game::RegistrationContextNew;
            sunrise_gameplay_common::game::native_hooks::native_register_render_pipelines(
                &mod_api::RegistrationContext::new(),
                &mut pipelines,
                &mut Vec::new(),
                &mut Vec::new(),
            );
            assert_eq!(pipelines.len(), 1);
            let vanilla = tempfile::tempdir().unwrap();
            std::fs::write(
                vanilla.path().join("render-pipelines.json"),
                r#"{"required":["minecraft:pipeline/solid_block"],"optional":["minecraft:pipeline/wireframe"]}"#,
            )
            .unwrap();
            sunrise_client_render::mod_pipelines::register(pipelines, vanilla.path()).unwrap();
            let shaders = tempfile::tempdir().unwrap();
            let core = shaders.path().join("assets/gpu_probe/shaders/core");
            std::fs::create_dir_all(&core).unwrap();
            std::fs::write(
                core.join("sampled.vsh"),
                "#version 330\n\
                 in vec3 Position;\n\
                 in vec2 UV0;\n\
                 in vec4 Color;\n\
                 out vec2 vUv;\n\
                 out vec4 vColor;\n\
                 void main() {\n\
                     gl_Position = vec4(Position.xy, 0.5, 1.0);\n\
                     vUv = UV0;\n\
                     vColor = Color;\n\
                 }\n",
            )
            .unwrap();
            std::fs::write(
                core.join("sampled.fsh"),
                "#version 330\n\
                 uniform sampler2D ColorTex;\n\
                 in vec2 vUv;\n\
                 in vec4 vColor;\n\
                 out vec4 fragColor;\n\
                 void main() {\n\
                     fragColor = texture(ColorTex, vUv) * vColor;\n\
                 }\n",
            )
            .unwrap();
            let mut render = sunrise_client_render::assets::RenderAssets::unpublished();
            render.mod_pipelines = sunrise_client_render::mod_pipelines::Pipelines::load(
                &sunrise_assets::resource_source::ResourceSource::directory(
                    shaders.path().to_owned(),
                ),
                7,
            )
            .unwrap();
            game.client.client_pack.render = std::sync::Arc::new(render);

            // `game.rs`'s `Game::render`: the renderer is out while its frame is drawn, the mod
            // GPU handles stay.
            let renderer_slot = game.client.renderer.0.take();
            assert!(game.client.renderer.0.is_none());
            assert!(game.mod_gpu().is_some());
            assert_eq!(game.entity_quad_capacity(), device_capacity);

            let camera = test_camera([0.0; 3], Mat4::IDENTITY);
            let mut draw = WorldGeometry::default();
            draw.reset([0.0; 3], Mat4::IDENTITY, [0.0; 2]);
            {
                let mut driver = GameWorldStageDriver::new(game, &camera, 0.0, &mut draw);
                driver.on_frame_graph_setup(&[], false);
                // The recorded pass draws into the mod render target the hook created.
                let graph = driver.take_frame_graph().unwrap();
                let created = handles.lock().unwrap().target.unwrap();
                assert_eq!(graph.passes.len(), 1);
                assert_eq!(graph.passes[0].label, "gpu_probe");
                assert!(matches!(
                    graph.passes[0].target,
                    Some(mod_api::FrameTarget::Mod(t)) if t == created
                ));
                driver.on_prepare_render_buffers(&[], false, &mut Default::default());
                let stage_draws: Vec<_> = driver
                    .on_render_stage(RenderLevelStage::AfterOpaqueFeatures, &[], false)
                    .iter()
                    .map(|ranges| ranges.pipeline_draws.clone())
                    .collect();
                // The mesh draw sampling the mod target's colour is in the stage batch.
                assert_eq!(stage_draws, vec![0..1]);
                let batch = driver.prepared_primitives();
                assert_eq!(batch.pipelines.draws.len(), 1);
                let recorded = &batch.pipelines.draws[0];
                assert_eq!(recorded.slot, 0);
                assert!(matches!(recorded.source, PipelineDrawSource::Retained(_)));
                assert!(matches!(
                    recorded.bindings.as_ref().map(|b| b.entries.as_slice()),
                    Some([ResolvedSet1Entry::Texture { mips: Some(_), .. }])
                ));
            }
            let panics = take_hook_panics();
            assert!(panics.is_empty(), "{panics:?}");
            let made = handles.lock().unwrap();
            assert!(made.target.is_some() && made.mesh.is_some());
            assert!(made.uniform.is_some() && made.storage.is_some());
            let (mesh, uniform, storage, target, pipeline) = (
                made.mesh.unwrap(),
                made.uniform.unwrap(),
                made.storage.unwrap(),
                made.target.unwrap(),
                made.pipeline.unwrap(),
            );
            drop(made);

            // The stage released every handle: a second release is refused.
            assert!(game.mesh_release(mesh).is_err());
            assert!(game.render_target_resize(target, [8, 8]).is_err());

            // The renderer's own textures that `TextureSource` binds resolve from the handles.
            let gpu = game.mod_gpu().unwrap();
            assert_eq!(
                gpu.mod_lightmap_texture_view().0,
                sunrise_client::gfx::mod_uniforms::LIGHTMAP_VIEW_ID
            );
            assert_eq!(
                gpu.mod_atlas_texture_view().0,
                sunrise_client::gfx::mod_uniforms::ATLAS_VIEW_ID
            );
            assert!(gpu.dynamic_texture_view(u64::MAX).is_none());

            // An active pack alone must not reject a direct draw; it is the translated
            // program's fixed sampler layout for this slot that makes the binding invalid.
            game.client.title.0 = None;
            game.client.screen = sunrise_gameplay_common::game::Screen::Playing;
            game.client
                .pack_frame_state
                .set(Some(mod_api::ShaderPackState::new(false, None, 1)));
            let depth = mod_api::DrawBindings::default()
                .texture("ColorTex", mod_api::TextureSource::WorldDepth);
            assert!(game.resolve_draw_bindings(&mut draw, 0, &depth).is_ok());
            game.client.pack_frame_slots.borrow_mut().push(0);
            assert_eq!(
                game.resolve_draw_bindings(&mut draw, 0, &depth)
                    .unwrap_err(),
                PipelineDrawError::PackSamplerLayout
            );
            let filterable = mod_api::DrawBindings::default()
                .texture("ColorTex", mod_api::TextureSource::Lightmap);
            assert!(
                game.resolve_draw_bindings(&mut draw, 0, &filterable)
                    .is_ok()
            );
            game.client.pack_frame_slots.borrow_mut().clear();
            game.client.pack_frame_state.set(None);

            // Without the game's handles (the renderer still out), nothing resolves.
            game.client.mod_gpu = None;
            assert_eq!(
                game.uniform_buffer_write(uniform, 0, &[1; 16]),
                Err(BufferError::Unavailable)
            );
            assert_eq!(
                game.storage_buffer_write_range(storage, 0, &[1; 16]),
                Err(BufferError::Unavailable)
            );
            assert_eq!(
                game.render_target_resize(target, [8, 8]),
                Err(RenderTargetError::Unavailable)
            );
            assert_eq!(game.mesh_release(mesh), Err(MeshError::Unavailable));
            assert_eq!(
                game.native_world_draw_mesh(
                    &mut draw,
                    pipeline,
                    mesh,
                    mod_api::MeshDraw::vertices(0..4),
                    &mod_api::DrawBindings::default().texture(
                        "ColorTex",
                        mod_api::TextureSource::RenderTargetColor(target, 0)
                    ),
                ),
                Err(mod_api::PipelineDrawError::Mesh(MeshError::Unavailable))
            );
            game.client.renderer.0 = renderer_slot;
        });
    }

    struct WorldDepthCompute(std::sync::Arc<Mutex<Option<mod_api::RenderTargetRef>>>);

    #[mod_api::__subscriptions]
    impl Hooks for WorldDepthCompute {
        fn register_render_pipelines(
            &self,
            _: &mod_api::RegistrationContext,
            event: &mut mod_api::RegisterRenderPipelinesEvent<'_>,
        ) {
            let shader = Identifier::parse("depth_probe:sample").unwrap();
            let layout = mod_api::BindGroupLayout::builder()
                .entry(mod_api::BindEntry::new(
                    "OutImage",
                    mod_api::BindingKind::StorageImage {
                        access: mod_api::StorageImageAccess::WriteOnly,
                    },
                ))
                .sampler(
                    "DepthTex",
                    mod_api::Sampler::new(
                        mod_api::SamplerFilter::Nearest,
                        mod_api::SamplerAddress::ClampToEdge,
                    ),
                )
                .build()
                .unwrap();
            event.compute_pipeline(
                mod_api::ComputePipeline::builder(shader.clone(), shader)
                    .bind_group_layout(layout)
                    .build(),
            );
        }

        fn frame_graph_setup(
            &self,
            host: &dyn ClientHost,
            event: &mut mod_api::FrameGraphSetupEvent<'_>,
        ) {
            let target = *self.0.lock().unwrap().get_or_insert_with(|| {
                host.create_render_target(
                    &mod_api::RenderTargetDescriptor::new([1, 1])
                        .color_target(mod_api::RenderTargetFormat::Rgba8Unorm)
                        .storage(mod_api::StorageImageAccess::WriteOnly),
                )
                .unwrap()
            });
            let pipeline = host
                .render_assets()
                .compute_pipeline(&Identifier::parse("depth_probe:sample").unwrap())
                .unwrap();
            event
                .graph
                .add_compute_pass(
                    mod_api::ComputePassDescriptor::new("sample live world depth")
                        .reads(mod_api::FrameTarget::MainDepth)
                        .writes(mod_api::FrameTarget::Mod(target)),
                    &mut |node| {
                        node.dispatch(
                            pipeline,
                            &mod_api::DrawBindings::default()
                                .storage_image(
                                    "OutImage",
                                    mod_api::StorageImageBinding::new(target, 0, 0),
                                )
                                .texture("DepthTex", mod_api::TextureSource::WorldDepth),
                            [1, 1, 1],
                        )
                        .unwrap()
                    },
                )
                .unwrap();
        }
    }

    /// A frame hook that hands the renderer the frame graph a fixture recorded, to feed it
    /// through the renderer's ordinary `FrameGraphRunner` while the fixture's world assets and
    /// stages stay as they are. It also notes the world depth view `gpu` publishes (its view ID
    /// and size) each time the frame reaches its custom geometry boundary.
    struct Recorded<'a> {
        graph: Option<sunrise_client::gfx::mod_frame_graph::RecordedFrameGraph>,
        gpu: &'a sunrise_client::gfx::ModGpuResources,
        depth_seen: Vec<(u64, [u32; 2])>,
    }
    impl sunrise_client::gfx::world_primitives::FrameHooks for Recorded<'_> {
        fn wants_after_sky(&self) -> bool {
            false
        }
        fn on_custom_geometry(&mut self, _: &[mod_api::RenderableSection], _: bool) {
            let (id, depth) = self.gpu.mod_world_depth_texture_view();
            self.depth_seen
                .push((id, [depth.texture().width(), depth.texture().height()]));
        }
        fn take_frame_graph(
            &mut self,
        ) -> Option<sunrise_client::gfx::mod_frame_graph::RecordedFrameGraph> {
            self.graph.take()
        }
        fn on_after_sky(&mut self, _: &[mod_api::RenderableSection], _: bool) {}
        fn on_prepare_render_buffers(
            &mut self,
            _: &[mod_api::RenderableSection],
            _: bool,
            _: &mut sunrise_client::gfx::world_primitives::PreparedHookGeometry,
        ) {
        }
        fn on_render_stage(
            &mut self,
            _: RenderLevelStage,
            _: &[mod_api::RenderableSection],
            _: bool,
        ) -> &[sunrise_client::gfx::world_primitives::DrawRanges] {
            &[]
        }
        fn prepared_primitives(&self) -> sunrise_client::gfx::world_primitives::Batch<'_> {
            Default::default()
        }
        fn on_after_level(&mut self, _: &[mod_api::RenderableSection], _: bool) {}
    }

    /// Clear world depth, record and render the `WorldDepthCompute` frame graph, and read back
    /// the pixel the compute pass wrote. Returns what `render` returned with that pixel.
    fn world_depth_frame(
        game: &mut ClientGame<'_>,
        depth: &wgpu::TextureView,
        value: f32,
        render: &std::sync::Arc<sunrise_client_render::assets::RenderAssets>,
        probe: &Mutex<Option<mod_api::RenderTargetRef>>,
        gpu: &sunrise_client::gfx::ModGpuResources,
    ) -> (bool, [u8; 4]) {
        use mod_api::__private::take_hook_panics;
        use sunrise_client::gfx::world_primitives::FrameHooks;
        let renderer = game.client.renderer.0.as_ref().unwrap();
        let mut encoder = renderer.device.create_command_encoder(&Default::default());
        // Initialize/store the actual view that WorldDepth resolves. This node runs before
        // the normal world pass so its known value is independent of world geometry and
        // the engine's unrelated store policy for graph reads later in the frame.
        drop(encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: depth,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(value),
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            ..Default::default()
        }));
        renderer.queue.submit([encoder.finish()]);

        let camera = test_camera([0.0; 3], Mat4::IDENTITY);
        let mut draw = WorldGeometry::default();
        draw.reset([0.0; 3], Mat4::IDENTITY, [0.0; 2]);
        let original = std::mem::replace(&mut game.client.client_pack.render, render.clone());
        let graph = {
            let mut driver = GameWorldStageDriver::new(game, &camera, 0.0, &mut draw);
            driver.on_frame_graph_setup(&[], false);
            driver.take_frame_graph().expect("native compute graph")
        };
        game.client.client_pack.render = original;
        let panics = take_hook_panics();
        assert!(panics.is_empty(), "{panics:?}");
        let mut hooks = Recorded {
            graph: Some(graph),
            gpu,
            depth_seen: Vec::new(),
        };
        let drawn = game
            .render_with(|renderer, frame| renderer.render(frame, Some(&mut hooks)))
            .unwrap();
        let target = probe.lock().unwrap().expect("native output target");
        let (_, view, _) = gpu
            .resolve_mod_render_target_color(Some("depth_probe"), target, 0)
            .unwrap();
        let renderer = game.client.renderer.0.as_ref().unwrap();
        let readback = renderer.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("native compute output"),
            size: 256,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = renderer.device.create_command_encoder(&Default::default());
        encoder.copy_texture_to_buffer(
            view.texture().as_image_copy(),
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
        renderer.queue.submit([encoder.finish()]);
        let slice = readback.slice(..);
        slice.map_async(wgpu::MapMode::Read, |result| result.unwrap());
        renderer
            .device
            .poll(wgpu::PollType::wait_indefinitely())
            .unwrap();
        let pixel = <[u8; 4]>::try_from(&slice.get_mapped_range().unwrap()[..4]).unwrap();
        (drawn, pixel)
    }

    #[test]
    #[ignore = "daily"]
    fn native_frame_graph_compute_reads_world_depth_after_resize_with_the_default_sampler() {
        use mod_api::__private::{game::RegistrationContextNew, take_hook_panics};
        if super::super::native_entity_refs::tests::run_fresh_cases(
            "game::client_side::native_custom_geometry::tests::native_frame_graph_compute_reads_world_depth_after_resize_with_the_default_sampler",
            "SUNRISE_MOD_WORLD_DEPTH_COMPUTE_CHILD",
            &["run"],
        ) {
            return;
        }

        super::super::native_entity_refs::tests::with_client_game(|game| {
            let (event_loop, window) =
                crate::game::native_hooks::particle_count_tests::hidden_window();
            game.common.options.offscreen = true;
            game.client.title.0 = None;
            game.client.screen = sunrise_gameplay_common::game::Screen::Playing;
            let renderer = game.new_renderer(window.clone()).unwrap();
            game.set_renderer(renderer);
            game.client.window.0 = Some(window);
            // Keep the window/event loop alive through GPU teardown.
            let _event_loop = event_loop;

            let probe = std::sync::Arc::new(Mutex::new(None));
            mod_platform::__private::install(
                vec![ModEntry::new(
                    "depth_probe",
                    Box::new(WorldDepthCompute(probe.clone())),
                )]
                .into(),
            );
            let _ = take_hook_panics();
            let mut compute = Vec::new();
            sunrise_gameplay_common::game::native_hooks::native_register_render_pipelines(
                &mod_api::RegistrationContext::new(),
                &mut Vec::new(),
                &mut Vec::new(),
                &mut compute,
            );
            sunrise_client_render::mod_compute::register(compute).unwrap();
            let shaders = tempfile::tempdir().unwrap();
            let path = shaders.path().join("assets/depth_probe/shaders/compute");
            std::fs::create_dir_all(&path).unwrap();
            std::fs::write(
                path.join("sample.csh"),
                "#version 450
layout(local_size_x = 1) in;
layout(rgba8) writeonly uniform image2D OutImage;
uniform sampler2D DepthTex;
void main() {
    float depth = textureLod(DepthTex, vec2(0.5), 0.0).r;
    imageStore(OutImage, ivec2(0), vec4(depth, 0.4, 1.0 - depth, 1.0));
}
",
            )
            .unwrap();
            let mut render = sunrise_client_render::assets::RenderAssets::unpublished();
            render.mod_pipelines = sunrise_client_render::mod_pipelines::Pipelines::load(
                &sunrise_assets::resource_source::ResourceSource::directory(
                    shaders.path().to_owned(),
                ),
                7,
            )
            .unwrap();
            game.client
                .renderer
                .0
                .as_mut()
                .unwrap()
                .build_mod_pipelines(&render.mod_pipelines, &|_| None)
                .unwrap();
            let render = std::sync::Arc::new(render);
            let gpu = game.mod_gpu().unwrap().clone();
            // Depth 0.25 and 0.75 times 255 give 63.75 and 191.25: they round to 64 and 191;
            // green 0.4 times 255 rounds to 102.
            let (initial_id, depth) = gpu.mod_world_depth_texture_view();
            assert_eq!(
                world_depth_frame(game, &depth, 0.25, &render, &probe, &gpu),
                (true, [64, 102, 191, 255])
            );
            assert_eq!(gpu.mod_world_depth_texture_view().0, initial_id);

            let width = depth.texture().width() + 16;
            let height = depth.texture().height() + 16;
            game.client
                .renderer
                .0
                .as_mut()
                .unwrap()
                .resize(width, height);
            let (resized_id, depth) = gpu.mod_world_depth_texture_view();
            assert_eq!(
                [depth.texture().width(), depth.texture().height()],
                [width, height]
            );
            assert_ne!(resized_id, initial_id);
            assert_eq!(
                world_depth_frame(game, &depth, 0.75, &render, &probe, &gpu),
                (true, [191, 102, 64, 255])
            );
            drop(game.client.renderer.0.take());
            drop(game.client.mod_gpu.take());
        });
    }

    /// A screenshot twice the window's size draws one frame over a world depth of its own
    /// (`Renderer::draw_scaled_frame`), and the window's depth comes back under a view ID no
    /// earlier view had. Only view IDs and sizes are compared.
    #[test]
    #[ignore = "daily"]
    fn a_scaled_screenshot_reads_its_own_world_depth_then_a_new_window_id() {
        if super::super::native_entity_refs::tests::run_fresh_cases(
            "game::client_side::native_custom_geometry::tests::a_scaled_screenshot_reads_its_own_world_depth_then_a_new_window_id",
            "SUNRISE_MOD_SCALED_SCREENSHOT_DEPTH_CHILD",
            &["run"],
        ) {
            return;
        }

        super::super::native_entity_refs::tests::with_client_game(|game| {
            let (event_loop, window) =
                crate::game::native_hooks::particle_count_tests::hidden_window();
            game.common.options.offscreen = true;
            game.client.title.0 = None;
            game.client.screen = sunrise_gameplay_common::game::Screen::Playing;
            let renderer = game.new_renderer(window.clone()).unwrap();
            game.set_renderer(renderer);
            game.client.window.0 = Some(window);
            // Keep the window/event loop alive through GPU teardown.
            let _event_loop = event_loop;

            let gpu = game.mod_gpu().unwrap().clone();
            let (window_id, depth) = gpu.mod_world_depth_texture_view();
            let window_size = [depth.texture().width(), depth.texture().height()];
            {
                let renderer = game.client.renderer.0.as_mut().unwrap();
                renderer.screenshot_quality = 2;
                renderer.screenshot = Some(sunrise_client::gfx::ScreenshotRequest {
                    path: "unused.png".into(),
                    size: None,
                    scaled: true,
                    background: false,
                });
            }
            let mut hooks = Recorded {
                graph: None,
                gpu: &gpu,
                depth_seen: Vec::new(),
            };
            let drawn = game
                .render_with(|renderer, frame| renderer.render(frame, Some(&mut hooks)))
                .unwrap();
            assert!(!drawn, "the screenshot frame never reaches the window");

            // Inside the frame: a depth of another ID and another size than the window's.
            assert_eq!(hooks.depth_seen.len(), 1);
            let (scaled_id, scaled_size) = hooks.depth_seen[0];
            assert_ne!(scaled_id, window_id);
            assert_ne!(scaled_size, window_size);

            // After it: the window's size again, under an ID neither of the two views had.
            let (restored_id, depth) = gpu.mod_world_depth_texture_view();
            assert_ne!(restored_id, scaled_id);
            assert_ne!(restored_id, window_id);
            assert_eq!(
                [depth.texture().width(), depth.texture().height()],
                window_size
            );
            drop(game.client.renderer.0.take());
            drop(game.client.mod_gpu.take());
        });
    }

    struct DepthRetentionProbe(std::sync::Arc<Mutex<Option<mod_api::RenderTargetRef>>>);

    #[mod_api::__subscriptions]
    impl Hooks for DepthRetentionProbe {
        fn register_render_pipelines(
            &self,
            _: &mod_api::RegistrationContext,
            event: &mut mod_api::RegisterRenderPipelinesEvent<'_>,
        ) {
            let shader = Identifier::parse("depth_retention:sample").unwrap();
            let layout = mod_api::BindGroupLayout::builder()
                .entry(mod_api::BindEntry::new(
                    "OutImage",
                    mod_api::BindingKind::StorageImage {
                        access: mod_api::StorageImageAccess::WriteOnly,
                    },
                ))
                .sampler(
                    "DepthTex",
                    mod_api::Sampler::new(
                        mod_api::SamplerFilter::Nearest,
                        mod_api::SamplerAddress::ClampToEdge,
                    ),
                )
                .build()
                .unwrap();
            event.compute_pipeline(
                mod_api::ComputePipeline::builder(shader.clone(), shader)
                    .bind_group_layout(layout)
                    .build(),
            );
        }

        fn frame_graph_setup(
            &self,
            host: &dyn ClientHost,
            event: &mut mod_api::FrameGraphSetupEvent<'_>,
        ) {
            let target = *self.0.lock().unwrap().get_or_insert_with(|| {
                host.create_render_target(
                    &mod_api::RenderTargetDescriptor::rgba8(
                        [1, 1],
                        mod_api::RenderTargetDepth::None,
                    )
                    .storage(mod_api::StorageImageAccess::WriteOnly),
                )
                .unwrap()
            });
            let pipeline = host
                .render_assets()
                .compute_pipeline(&Identifier::parse("depth_retention:sample").unwrap())
                .unwrap();
            event
                .graph
                .add_compute_pass(
                    mod_api::ComputePassDescriptor::new("sample opaque terrain depth")
                        .after_stage(RenderLevelStage::AfterOpaqueBlocks)
                        .reads(mod_api::FrameTarget::MainDepth)
                        .writes(mod_api::FrameTarget::Mod(target)),
                    &mut |node| {
                        let bindings = mod_api::DrawBindings::default()
                            .storage_image(
                                "OutImage",
                                mod_api::StorageImageBinding::new(target, 0, 0),
                            )
                            .texture("DepthTex", mod_api::TextureSource::WorldDepth);
                        node.dispatch(pipeline, &bindings, [1, 1, 1]).unwrap();
                    },
                )
                .unwrap();
        }
    }

    #[test]
    #[ignore = "daily"]
    fn frame_graph_compute_reads_rendered_opaque_depth() {
        use super::super::native_entity_refs::tests::{test_window, with_client_game};
        use crate::game::loading_window::{loading_frame, loading_renderer};
        use mod_api::__private::{game::RegistrationContextNew, take_hook_panics};
        use std::sync::Arc;
        use sunrise_client::mesher::{SectionMesh, TerrainVertex};
        use sunrise_client_render::{assets::RenderAssets, graphics::GraphicsApi};
        use sunrise_gameplay_common::game::{Screen, test_fixtures::run_fresh_cases_with_env};

        // One fresh mod registry and no cull/visibility pass that could force a store first.
        if run_fresh_cases_with_env(
            "game::client_side::native_custom_geometry::tests::frame_graph_compute_reads_rendered_opaque_depth",
            "SUNRISE_FRAME_GRAPH_DEPTH_STORE_CHILD",
            &["run"],
            1,
            &[("SUNRISE_GPU_CULL", "0"), ("SUNRISE_VISBUFFER", "0")],
        ) {
            return;
        }
        with_client_game(|game| {
            let (_event_loop, window) = test_window([64, 64]);
            // Default graphics, a white terrain sprite, no pack, and native resolution.
            let mut renderer =
                loading_renderer(window, true, None, None, None, GraphicsApi::Vulkan).unwrap();
            renderer.set_section_cull(false).unwrap();
            eprintln!("adapter: {:?}", renderer.info);
            game.set_renderer(Box::new(renderer));
            game.client.title.0 = None;
            game.client.screen = Screen::Playing;
            assert!(!sunrise_client_render::render_slots::needs().world_depth);

            let probe = Arc::new(Mutex::new(None));
            mod_platform::__private::install(
                vec![ModEntry::new(
                    "depth_retention",
                    Box::new(DepthRetentionProbe(probe.clone())),
                )]
                .into(),
            );
            let _ = take_hook_panics();
            let mut compute = Vec::new();
            sunrise_gameplay_common::game::native_hooks::native_register_render_pipelines(
                &mod_api::RegistrationContext::new(),
                &mut Vec::new(),
                &mut Vec::new(),
                &mut compute,
            );
            sunrise_client_render::mod_compute::register(compute).unwrap();
            let shaders = tempfile::tempdir().unwrap();
            let folder = shaders
                .path()
                .join("assets/depth_retention/shaders/compute");
            std::fs::create_dir_all(&folder).unwrap();
            std::fs::write(
                folder.join("sample.csh"),
                "#version 450
layout(local_size_x = 1) in;
layout(rgba8) writeonly uniform image2D OutImage;
uniform sampler2D DepthTex;
void main() {
    float depth = textureLod(DepthTex, vec2(0.5), 0.0).r;
    imageStore(OutImage, ivec2(0), vec4(depth, 0.0, 0.0, 1.0));
}
",
            )
            .unwrap();
            let mut assets = RenderAssets::unpublished();
            assets.mod_pipelines = sunrise_client_render::mod_pipelines::Pipelines::load(
                &sunrise_assets::resource_source::ResourceSource::directory(
                    shaders.path().to_owned(),
                ),
                7,
            )
            .unwrap();
            let mut renderer = game.client.renderer.0.take().unwrap();
            renderer
                .build_mod_pipelines(&assets.mod_pipelines, &|_| None)
                .unwrap();
            game.client.client_pack.render = Arc::new(assets);

            // Face -Z toward the camera. A white opaque wall fills the center of the frame at
            // view-space z = -2 (distance 2 blocks ahead of the camera looking along +Z).
            let mut wall = SectionMesh::default();
            for (pos, uv) in [[0., 0., 4.], [0., 16., 4.], [16., 16., 4.], [16., 0., 4.]]
                .into_iter()
                .zip([[0., 1.], [0., 0.], [1., 0.], [1., 1.]])
            {
                wall.solid
                    .push(TerrainVertex::new(pos, uv, 0, u32::MAX, 0xf0f0));
            }
            wall.sort_solid();
            renderer.load_column(0, 0);
            renderer.upload((0, 10, 0), &wall); // Section starts at y = -64 + 10 * 16 = 96.
            let hud = sunrise_client::hud::HudBatch::scratch([
                renderer.config.width,
                renderer.config.height,
            ]);
            let mut frame = loading_frame(&hud);
            frame.title = false;
            frame.distance = 64.0;
            frame.detail_distance = 64.0;
            frame.camera = DVec3::new(8.0, 104.0, 2.0);
            frame.eye = frame.camera;
            frame.pitch = 0.0;
            frame.eye_rotation = [0.0, 0.0];
            let camera = test_camera(
                frame.camera.to_array(),
                sunrise_client::camera::view_rotation_matrix(0.0, 0.0).0,
            );
            let mut draw = WorldGeometry::default();
            draw.reset(
                camera.position,
                Mat4::from_cols_array(&camera.view),
                [0.0; 2],
            );
            let mut hooks = GameWorldStageDriver::new(game, &camera, 0.0, &mut draw);
            assert!(renderer.render(&frame, Some(&mut hooks)).unwrap());
            assert_eq!(
                renderer.drawn_sections, 1,
                "the opaque wall section must draw"
            );
            assert!(
                renderer.section_draws > 0,
                "the opaque wall must issue a draw"
            );
            drop(hooks);
            renderer.wait_submissions();
            let panics = take_hook_panics();
            assert!(panics.is_empty(), "{panics:?}");

            let target = probe.lock().unwrap().expect("the live frame hook ran");
            let (_, view, _) = renderer
                .mod_gpu()
                .resolve_mod_render_target_color(Some("depth_retention"), target, 0)
                .unwrap();
            let readback = renderer.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("sampled opaque depth readback"),
                size: 256,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });
            let mut encoder = renderer.device.create_command_encoder(&Default::default());
            encoder.copy_texture_to_buffer(
                view.texture().as_image_copy(),
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
            renderer.queue.submit([encoder.finish()]);
            let slice = readback.slice(..);
            slice.map_async(wgpu::MapMode::Read, |result| result.unwrap());
            renderer
                .device
                .poll(wgpu::PollType::wait_indefinitely())
                .unwrap();
            // Reversed-Z at 2 blocks with near 0.05 and far 1000 is ~0.02495: red = 6 in RGBA8.
            // Discard gives red = 0 even though the compute dispatch still writes alpha = 255.
            assert_eq!(&slice.get_mapped_range().unwrap()[..4], &[6, 0, 0, 255]);
            drop(game.client.mod_gpu.take());
            drop(renderer);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn frame_graph_compute_reads_scaled_world_depth() {
        use super::super::native_entity_refs::tests::{test_window, with_client_game};
        use crate::game::loading_window::{loading_frame, loading_renderer};
        use mod_api::__private::{game::RegistrationContextNew, take_hook_panics};
        use std::sync::Arc;
        use sunrise_client::mesher::{SectionMesh, TerrainVertex};
        use sunrise_client_render::{assets::RenderAssets, graphics::GraphicsApi};
        use sunrise_gameplay_common::game::{Screen, test_fixtures::run_fresh_cases_with_env};

        if run_fresh_cases_with_env(
            "game::client_side::native_custom_geometry::tests::frame_graph_compute_reads_scaled_world_depth",
            "SUNRISE_SCALED_WORLD_DEPTH_CHILD",
            &["run"],
            1,
            &[("SUNRISE_GPU_CULL", "0"), ("SUNRISE_VISBUFFER", "0")],
        ) {
            return;
        }
        with_client_game(|game| {
            let (_event_loop, window) = test_window([64, 64]);
            let mut renderer =
                loading_renderer(window, true, None, None, None, GraphicsApi::Vulkan).unwrap();
            renderer.set_section_cull(false).unwrap();
            game.set_renderer(Box::new(renderer));
            game.client.title.0 = None;
            game.client.screen = Screen::Playing;

            let probe = Arc::new(Mutex::new(None));
            mod_platform::__private::install(
                vec![ModEntry::new(
                    "depth_retention",
                    Box::new(DepthRetentionProbe(probe.clone())),
                )]
                .into(),
            );
            let _ = take_hook_panics();
            let mut compute = Vec::new();
            sunrise_gameplay_common::game::native_hooks::native_register_render_pipelines(
                &mod_api::RegistrationContext::new(),
                &mut Vec::new(),
                &mut Vec::new(),
                &mut compute,
            );
            sunrise_client_render::mod_compute::register(compute).unwrap();
            let shaders = tempfile::tempdir().unwrap();
            let folder = shaders
                .path()
                .join("assets/depth_retention/shaders/compute");
            std::fs::create_dir_all(&folder).unwrap();
            std::fs::write(
                folder.join("sample.csh"),
                "#version 450
layout(local_size_x = 1) in;
layout(rgba8) writeonly uniform image2D OutImage;
uniform sampler2D DepthTex;
void main() {
    float depth = textureLod(DepthTex, vec2(0.5), 0.0).r;
    imageStore(OutImage, ivec2(0), vec4(depth, 0.0, 0.0, 1.0));
}
",
            )
            .unwrap();
            let mut assets = RenderAssets::unpublished();
            assets.mod_pipelines = sunrise_client_render::mod_pipelines::Pipelines::load(
                &sunrise_assets::resource_source::ResourceSource::directory(
                    shaders.path().to_owned(),
                ),
                7,
            )
            .unwrap();
            let gpu = game.mod_gpu().unwrap().clone();
            let mut renderer = game.client.renderer.0.take().unwrap();
            renderer
                .build_mod_pipelines(&assets.mod_pipelines, &|_| None)
                .unwrap();
            game.client.client_pack.render = Arc::new(assets);

            // White opaque wall 2 blocks ahead of the camera (reversed-Z ~0.02495 -> red 6).
            let mut wall = SectionMesh::default();
            for (pos, uv) in [[0., 0., 4.], [0., 16., 4.], [16., 16., 4.], [16., 0., 4.]]
                .into_iter()
                .zip([[0., 1.], [0., 0.], [1., 0.], [1., 1.]])
            {
                wall.solid
                    .push(TerrainVertex::new(pos, uv, 0, u32::MAX, 0xf0f0));
            }
            wall.sort_solid();
            renderer.load_column(0, 0);
            renderer.upload((0, 10, 0), &wall);

            let render_and_read = |renderer: &mut sunrise_client::gfx::Renderer,
                                   game: &mut ClientGame<'_>|
             -> [u8; 4] {
                let hud = sunrise_client::hud::HudBatch::scratch([
                    renderer.config.width,
                    renderer.config.height,
                ]);
                let mut frame = loading_frame(&hud);
                frame.title = false;
                frame.distance = 64.0;
                frame.detail_distance = 64.0;
                frame.camera = DVec3::new(8.0, 104.0, 2.0);
                frame.eye = frame.camera;
                frame.pitch = 0.0;
                frame.eye_rotation = [0.0, 0.0];
                let camera = test_camera(
                    frame.camera.to_array(),
                    sunrise_client::camera::view_rotation_matrix(0.0, 0.0).0,
                );
                let mut draw = WorldGeometry::default();
                draw.reset(
                    camera.position,
                    Mat4::from_cols_array(&camera.view),
                    [0.0; 2],
                );
                let mut hooks = GameWorldStageDriver::new(game, &camera, 0.0, &mut draw);
                assert!(renderer.render(&frame, Some(&mut hooks)).unwrap());
                assert_eq!(
                    renderer.drawn_sections, 1,
                    "the opaque wall section must draw"
                );
                assert!(
                    renderer.section_draws > 0,
                    "the opaque wall must issue a draw"
                );
                drop(hooks);
                renderer.wait_submissions();
                let panics = take_hook_panics();
                assert!(panics.is_empty(), "{panics:?}");

                let target = probe.lock().unwrap().expect("the live frame hook ran");
                let (_, view, _) = renderer
                    .mod_gpu()
                    .resolve_mod_render_target_color(Some("depth_retention"), target, 0)
                    .unwrap();
                let readback = renderer.device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("scaled world depth readback"),
                    size: 256,
                    usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                    mapped_at_creation: false,
                });
                let mut encoder = renderer.device.create_command_encoder(&Default::default());
                encoder.copy_texture_to_buffer(
                    view.texture().as_image_copy(),
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
                renderer.queue.submit([encoder.finish()]);
                let slice = readback.slice(..);
                slice.map_async(wgpu::MapMode::Read, |result| result.unwrap());
                renderer
                    .device
                    .poll(wgpu::PollType::wait_indefinitely())
                    .unwrap();
                <[u8; 4]>::try_from(&slice.get_mapped_range().unwrap()[..4]).unwrap()
            };

            // Frame 1 (unscaled): samples window depth and keeps its initial view ID.
            let (window_id, window_depth) = gpu.mod_world_depth_texture_view();
            let window_size = [
                window_depth.texture().width(),
                window_depth.texture().height(),
            ];
            assert_eq!(render_and_read(&mut renderer, game), [6, 0, 0, 255]);
            assert_eq!(gpu.mod_world_depth_texture_view().0, window_id);

            // Poison the window depth with 0.75 (red = 191) and enable 0.5x world resolution
            // scale so the world pass draws into WorldPost's scaled depth target instead.
            let mut encoder = renderer.device.create_command_encoder(&Default::default());
            drop(encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &window_depth,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(0.75),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                ..Default::default()
            }));
            renderer.queue.submit([encoder.finish()]);
            renderer.set_world_post(0.5, false, false, 1.0, 0.85);

            // Frame 2 (scaled 0.5x): must publish the scaled depth under a new ID and read the
            // rendered wall (6), not the poisoned window depth (191).
            assert_eq!(render_and_read(&mut renderer, game), [6, 0, 0, 255]);
            let (scaled_id, scaled_depth) = gpu.mod_world_depth_texture_view();
            assert_ne!(scaled_id, window_id);
            assert_eq!(
                [
                    scaled_depth.texture().width(),
                    scaled_depth.texture().height(),
                ],
                [window_size[0] / 2, window_size[1] / 2]
            );

            // Frame 3 (same 0.5x scale): steady-state frame keeps the same view ID.
            assert_eq!(render_and_read(&mut renderer, game), [6, 0, 0, 255]);
            assert_eq!(gpu.mod_world_depth_texture_view().0, scaled_id);

            // Frame 4 (back to 1.0x unscaled): publishes window depth under a brand-new ID.
            renderer.set_world_post(1.0, false, false, 1.0, 0.85);
            assert_eq!(render_and_read(&mut renderer, game), [6, 0, 0, 255]);
            let (restored_id, restored_depth) = gpu.mod_world_depth_texture_view();
            assert_ne!(restored_id, scaled_id);
            assert_ne!(restored_id, window_id);
            assert_eq!(
                [
                    restored_depth.texture().width(),
                    restored_depth.texture().height(),
                ],
                window_size
            );

            drop(window_depth);
            drop(scaled_depth);
            drop(restored_depth);
            drop(gpu);
            drop(game.client.mod_gpu.take());
            drop(renderer);
        });
    }

    struct PackWorldDepthProbe(std::sync::Arc<Mutex<Option<mod_api::RenderTargetRef>>>);

    #[mod_api::__subscriptions]
    impl Hooks for PackWorldDepthProbe {
        fn register_render_pipelines(
            &self,
            _: &mod_api::RegistrationContext,
            event: &mut mod_api::RegisterRenderPipelinesEvent<'_>,
        ) {
            let shader = Identifier::parse("pack_world_depth:sample").unwrap();
            let layout = mod_api::BindGroupLayout::builder()
                .entry(mod_api::BindEntry::new(
                    "OutImage",
                    mod_api::BindingKind::StorageImage {
                        access: mod_api::StorageImageAccess::WriteOnly,
                    },
                ))
                .sampler(
                    "DepthTex",
                    mod_api::Sampler::new(
                        mod_api::SamplerFilter::Nearest,
                        mod_api::SamplerAddress::ClampToEdge,
                    ),
                )
                .build()
                .unwrap();
            event.compute_pipeline(
                mod_api::ComputePipeline::builder(shader.clone(), shader)
                    .bind_group_layout(layout)
                    .build(),
            );
        }

        fn frame_graph_setup(
            &self,
            host: &dyn ClientHost,
            event: &mut mod_api::FrameGraphSetupEvent<'_>,
        ) {
            let target = *self.0.lock().unwrap().get_or_insert_with(|| {
                host.create_render_target(
                    &mod_api::RenderTargetDescriptor::rgba8(
                        [2, 1],
                        mod_api::RenderTargetDepth::None,
                    )
                    .storage(mod_api::StorageImageAccess::WriteOnly),
                )
                .unwrap()
            });
            let pipeline = host
                .render_assets()
                .compute_pipeline(&Identifier::parse("pack_world_depth:sample").unwrap())
                .unwrap();
            event
                .graph
                .add_compute_pass(
                    mod_api::ComputePassDescriptor::new("sample world depth top and bottom")
                        .after_stage(RenderLevelStage::AfterOpaqueBlocks)
                        .reads(mod_api::FrameTarget::MainDepth)
                        .writes(mod_api::FrameTarget::Mod(target)),
                    &mut |node| {
                        let bindings = mod_api::DrawBindings::default()
                            .storage_image(
                                "OutImage",
                                mod_api::StorageImageBinding::new(target, 0, 0),
                            )
                            .texture("DepthTex", mod_api::TextureSource::WorldDepth);
                        node.dispatch(pipeline, &bindings, [1, 1, 1]).unwrap();
                    },
                )
                .unwrap();
        }
    }

    #[test]
    #[ignore = "daily"]
    fn frame_graph_compute_reads_identical_world_depth_with_and_without_terrain_pack() {
        use super::super::native_entity_refs::tests::{test_window, with_client_game};
        use crate::game::loading_window::{loading_frame, loading_renderer};
        use mod_api::__private::{game::RegistrationContextNew, take_hook_panics};
        use std::sync::Arc;
        use sunrise_client::mesher::{SectionMesh, TerrainVertex};
        use sunrise_client_render::{assets::RenderAssets, graphics::GraphicsApi};
        use sunrise_gameplay_common::game::{Screen, test_fixtures::run_fresh_cases_with_env};

        if run_fresh_cases_with_env(
            "game::client_side::native_custom_geometry::tests::frame_graph_compute_reads_identical_world_depth_with_and_without_terrain_pack",
            "SUNRISE_PACK_WORLD_DEPTH_COPY_CHILD",
            &["run"],
            1,
            &[("SUNRISE_GPU_CULL", "0"), ("SUNRISE_VISBUFFER", "0")],
        ) {
            return;
        }
        with_client_game(|game| {
            let (_event_loop, window) = test_window([64, 64]);
            let mut renderer =
                loading_renderer(window, true, None, None, None, GraphicsApi::Vulkan).unwrap();
            renderer.set_section_cull(false).unwrap();
            game.set_renderer(Box::new(renderer));
            game.client.title.0 = None;
            game.client.screen = Screen::Playing;

            let probe = Arc::new(Mutex::new(None));
            mod_platform::__private::install(
                vec![ModEntry::new(
                    "pack_world_depth",
                    Box::new(PackWorldDepthProbe(probe.clone())),
                )]
                .into(),
            );
            let _ = take_hook_panics();
            let mut compute = Vec::new();
            sunrise_gameplay_common::game::native_hooks::native_register_render_pipelines(
                &mod_api::RegistrationContext::new(),
                &mut Vec::new(),
                &mut Vec::new(),
                &mut compute,
            );
            sunrise_client_render::mod_compute::register(compute).unwrap();
            let shaders = tempfile::tempdir().unwrap();
            let folder = shaders
                .path()
                .join("assets/pack_world_depth/shaders/compute");
            std::fs::create_dir_all(&folder).unwrap();
            std::fs::write(
                folder.join("sample.csh"),
                "#version 450
layout(local_size_x = 1) in;
layout(rgba8) writeonly uniform image2D OutImage;
uniform sampler2D DepthTex;
void main() {
    float top_depth = textureLod(DepthTex, vec2(0.5, 0.25), 0.0).r;
    float bottom_depth = textureLod(DepthTex, vec2(0.5, 0.75), 0.0).r;
    imageStore(OutImage, ivec2(0, 0), vec4(top_depth, 0.0, 0.0, 1.0));
    imageStore(OutImage, ivec2(1, 0), vec4(bottom_depth, 0.0, 0.0, 1.0));
}
",
            )
            .unwrap();
            let mut assets = RenderAssets::unpublished();
            assets.mod_pipelines = sunrise_client_render::mod_pipelines::Pipelines::load(
                &sunrise_assets::resource_source::ResourceSource::directory(
                    shaders.path().to_owned(),
                ),
                7,
            )
            .unwrap();
            let mut renderer = game.client.renderer.0.take().unwrap();
            renderer
                .build_mod_pipelines(&assets.mod_pipelines, &|_| None)
                .unwrap();
            game.client.client_pack.render = Arc::new(assets);

            // Upper-half opaque wall (local y 8..16 -> world y 104..112, above the y=104 horizon)
            // at distance 2 blocks ahead of the camera so both Z conversion and Y orientation are
            // proven: top sample (0.5, 0.25) hits the wall (red 6), bottom (0.5, 0.75) hits sky (0).
            let mut wall = SectionMesh::default();
            for (pos, uv) in [[0., 8., 4.], [0., 16., 4.], [16., 16., 4.], [16., 8., 4.]]
                .into_iter()
                .zip([[0., 1.], [0., 0.], [1., 0.], [1., 1.]])
            {
                wall.solid
                    .push(TerrainVertex::new(pos, uv, 0, u32::MAX, 0xf0f0));
            }
            wall.sort_solid();
            renderer.load_column(0, 0);
            renderer.upload((0, 10, 0), &wall);

            let render_and_read = |renderer: &mut sunrise_client::gfx::Renderer,
                                   game: &mut ClientGame<'_>,
                                   want_screenshot: bool|
             -> ([[u8; 4]; 2], Option<[u8; 4]>) {
                if want_screenshot {
                    renderer.screenshot = Some(sunrise_client::gfx::ScreenshotRequest {
                        path: "unused.png".into(),
                        size: None,
                        scaled: false,
                        background: false,
                    });
                }
                let hud = sunrise_client::hud::HudBatch::scratch([
                    renderer.config.width,
                    renderer.config.height,
                ]);
                let mut frame = loading_frame(&hud);
                frame.title = false;
                frame.distance = 64.0;
                frame.detail_distance = 64.0;
                frame.camera = DVec3::new(8.0, 104.0, 2.0);
                frame.eye = frame.camera;
                frame.pitch = 0.0;
                frame.eye_rotation = [0.0, 0.0];
                let camera = test_camera(
                    frame.camera.to_array(),
                    sunrise_client::camera::view_rotation_matrix(0.0, 0.0).0,
                );
                let mut draw = WorldGeometry::default();
                draw.reset(
                    camera.position,
                    Mat4::from_cols_array(&camera.view),
                    [0.0; 2],
                );
                let mut hooks = GameWorldStageDriver::new(game, &camera, 0.0, &mut draw);
                assert!(renderer.render(&frame, Some(&mut hooks)).unwrap());
                assert_eq!(
                    renderer.drawn_sections, 1,
                    "the opaque wall section must draw"
                );
                assert!(
                    renderer.section_draws > 0,
                    "the opaque wall must issue a draw"
                );
                drop(hooks);
                renderer.wait_submissions();
                let panics = take_hook_panics();
                assert!(panics.is_empty(), "{panics:?}");

                let top_pixel = want_screenshot.then(|| {
                    let mut pending = renderer.screenshot_result.take().unwrap().unwrap();
                    let image = pending.image_mut();
                    let at = (16 * image.width as usize + 32) * 4;
                    image.rgba[at..at + 4].try_into().unwrap()
                });

                let target = probe.lock().unwrap().expect("the live frame hook ran");
                let (_, view, _) = renderer
                    .mod_gpu()
                    .resolve_mod_render_target_color(Some("pack_world_depth"), target, 0)
                    .unwrap();
                let readback = renderer.device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("pack world depth readback"),
                    size: 256,
                    usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                    mapped_at_creation: false,
                });
                let mut encoder = renderer.device.create_command_encoder(&Default::default());
                encoder.copy_texture_to_buffer(
                    view.texture().as_image_copy(),
                    wgpu::TexelCopyBufferInfo {
                        buffer: &readback,
                        layout: wgpu::TexelCopyBufferLayout {
                            offset: 0,
                            bytes_per_row: Some(256),
                            rows_per_image: None,
                        },
                    },
                    wgpu::Extent3d {
                        width: 2,
                        height: 1,
                        depth_or_array_layers: 1,
                    },
                );
                renderer.queue.submit([encoder.finish()]);
                let slice = readback.slice(..);
                slice.map_async(wgpu::MapMode::Read, |result| result.unwrap());
                renderer
                    .device
                    .poll(wgpu::PollType::wait_indefinitely())
                    .unwrap();
                let mapped = slice.get_mapped_range().unwrap();
                let depths = [
                    <[u8; 4]>::try_from(&mapped[0..4]).unwrap(),
                    <[u8; 4]>::try_from(&mapped[4..8]).unwrap(),
                ];
                (depths, top_pixel)
            };

            // Frame 1 (plain route, no shader pack): top hits wall (6), bottom hits sky (0).
            let (plain_depths, _) = render_and_read(&mut renderer, game, false);
            assert_eq!(plain_depths, [[6, 0, 0, 255], [0, 0, 0, 255]]);

            // Enable a minimal `gbuffers_terrain` shader pack that draws terrain pure blue into
            // `colortex0` + `depthtex0` (standard-Z, GL rows) so Sunrise's plain world pass draws
            // no terrain into `world_depth`.
            let pack_dir = tempfile::tempdir().unwrap();
            let pack_shaders = pack_dir.path().join("shaders");
            std::fs::create_dir_all(&pack_shaders).unwrap();
            std::fs::write(
                pack_shaders.join("gbuffers_terrain.vsh"),
                "#version 120\nvoid main() {\n    gl_Position = ftransform();\n}\n",
            )
            .unwrap();
            std::fs::write(
                pack_shaders.join("gbuffers_terrain.fsh"),
                "#version 120\nvoid main() {\n    gl_FragData[0] = vec4(0.0, 0.0, 1.0, 1.0);\n}\n",
            )
            .unwrap();
            let pack = sunrise_shaders::Pack::open(pack_dir.path()).unwrap();
            let properties = sunrise_shaders::Properties::default();
            let macros = sunrise_shaders::Macros::default();
            let inputs = renderer.pack_inputs();
            let post = sunrise_shaders::load_post(&pack, "", &properties, &macros, &inputs);
            assert!(post.terrain.is_some(), "gbuffers_terrain must translate");
            assert!(
                renderer
                    .set_shader_pack(0, Some(post), &[])
                    .unwrap()
                    .is_some(),
                "gbuffers_terrain shader pack must install"
            );
            assert_eq!(renderer.shader_pack_kind(), Some("iris"));

            // Frame 2 (shader pack draws terrain): the pack drew the blue wall into depthtex0 and
            // `PackPost::export_depth` copied it into `world_depth` before `AfterOpaqueBlocks`,
            // giving the exact same reversed-Z, screen-row values as the plain route.
            let (pack_depths, pack_top_pixel) = render_and_read(&mut renderer, game, true);
            assert_eq!(
                pack_top_pixel.unwrap(),
                [0, 0, 255, 255],
                "gbuffers_terrain must own the terrain draw"
            );
            assert_eq!(
                pack_depths, plain_depths,
                "WorldDepth must match between plain and terrain-drawing shader-pack routes"
            );

            drop(game.client.mod_gpu.take());
            drop(renderer);
        });
    }

    struct MidWorldSplitDepthProbe {
        target: Mutex<Option<mod_api::RenderTargetRef>>,
        draws: Mutex<Vec<mod_api::PreparedWorldDraw>>,
    }

    #[mod_api::__subscriptions]
    impl Hooks for MidWorldSplitDepthProbe {
        fn register_render_pipelines(
            &self,
            _: &mod_api::RegistrationContext,
            event: &mut mod_api::RegisterRenderPipelinesEvent<'_>,
        ) {
            let fmt = mod_api::VertexFormat::builder(mod_api::VertexStepMode::Vertex)
                .attribute("Pos", mod_api::VertexAttributeFormat::Float32x3)
                .attribute("Tint", mod_api::VertexAttributeFormat::Float32x4)
                .build()
                .unwrap();
            event.register(
                mod_api::RenderPipeline::new(
                    Identifier::parse("mid_world_split:solid").unwrap(),
                    Identifier::parse("mid_world_split:core/solid").unwrap(),
                    mod_api::PipelineBlend::Opaque,
                    mod_api::WorldDepth::Tested,
                )
                .vertex_binding(0, fmt),
            );
        }

        fn frame_graph_setup(
            &self,
            host: &dyn ClientHost,
            event: &mut mod_api::FrameGraphSetupEvent<'_>,
        ) {
            let desc = mod_api::RenderTargetDescriptor::new([4, 4])
                .color_target(mod_api::RenderTargetFormat::Rgba8Unorm);
            let target = *self
                .target
                .lock()
                .unwrap()
                .get_or_insert_with(|| host.create_render_target(&desc).unwrap());
            // Kept mid-world pass that does NOT declare `reads(FrameTarget::MainDepth)`.
            event
                .graph
                .add_pass(
                    mod_api::FramePassDescriptor::new(
                        "mid_world_noop",
                        mod_api::FrameTarget::Mod(target),
                    )
                    .after_stage(RenderLevelStage::AfterOpaqueBlocks),
                    &mut |_| {},
                )
                .unwrap();
        }

        fn prepare_render_buffers(
            &self,
            host: &dyn ClientHost,
            event: &mut mod_api::PrepareRenderBuffersEvent<'_>,
        ) {
            #[repr(C)]
            #[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
            struct PosTint([f32; 3], [f32; 4]);

            let pipeline = host
                .render_assets()
                .render_pipeline(&Identifier::parse("mid_world_split:solid").unwrap())
                .unwrap();
            // Right half behind the opaque wall (reversed-Z 0.01 < ~0.02495, > 0.0), so
            // discarded depth lets it overwrite pixel (32, 32) green while preserved depth keeps
            // pixel (32, 32) equal to its horizontally symmetric uncovered neighbour (31, 32).
            let behind = [
                PosTint([0.0, -1.0, 0.01], [0.0, 1.0, 0.0, 1.0]),
                PosTint([1.0, -1.0, 0.01], [0.0, 1.0, 0.0, 1.0]),
                PosTint([1.0, 1.0, 0.01], [0.0, 1.0, 0.0, 1.0]),
                PosTint([0.0, 1.0, 0.01], [0.0, 1.0, 0.0, 1.0]),
            ];
            // In front of the opaque wall in the top-left corner to prove the stage ran.
            let front_corner = [
                PosTint([-1.0, 0.5, 0.5], [1.0, 0.0, 0.0, 1.0]),
                PosTint([-0.5, 0.5, 0.5], [1.0, 0.0, 0.0, 1.0]),
                PosTint([-0.5, 1.0, 0.5], [1.0, 0.0, 0.0, 1.0]),
                PosTint([-1.0, 1.0, 0.5], [1.0, 0.0, 0.0, 1.0]),
            ];
            let draw_behind = event
                .context
                .draw_vertices(
                    RenderLevelStage::AfterOpaqueFeatures,
                    pipeline,
                    &[bytemuck::cast_slice(&behind)],
                    None,
                    mod_api::MeshDraw::vertices(0..4),
                    &mod_api::DrawBindings::default(),
                )
                .unwrap();
            let draw_front = event
                .context
                .draw_vertices(
                    RenderLevelStage::AfterOpaqueFeatures,
                    pipeline,
                    &[bytemuck::cast_slice(&front_corner)],
                    None,
                    mod_api::MeshDraw::vertices(0..4),
                    &mod_api::DrawBindings::default(),
                )
                .unwrap();
            *self.draws.lock().unwrap() = vec![draw_behind, draw_front];
        }

        fn render_level_stage(
            &self,
            _: &dyn ClientHost,
            event: &mut mod_api::RenderLevelStageEvent<'_>,
        ) {
            if event.stage != RenderLevelStage::AfterOpaqueFeatures {
                return;
            }
            let draws: Vec<_> = self.draws.lock().unwrap().drain(..).collect();
            for draw in draws {
                event.context.world_pass().unwrap().draw(draw).unwrap();
            }
        }
    }

    #[test]
    #[ignore = "daily"]
    fn mid_world_frame_graph_split_preserves_depth_without_depth_read() {
        use super::super::native_entity_refs::tests::{test_window, with_client_game};
        use crate::game::loading_window::{loading_frame, loading_renderer};
        use mod_api::__private::{game::RegistrationContextNew, take_hook_panics};
        use std::sync::Arc;
        use sunrise_client::mesher::{SectionMesh, TerrainVertex};
        use sunrise_client_render::{assets::RenderAssets, graphics::GraphicsApi};
        use sunrise_gameplay_common::game::{Screen, test_fixtures::run_fresh_cases_with_env};

        if run_fresh_cases_with_env(
            "game::client_side::native_custom_geometry::tests::mid_world_frame_graph_split_preserves_depth_without_depth_read",
            "SUNRISE_MID_WORLD_SPLIT_DEPTH_CHILD",
            &["run"],
            1,
            &[("SUNRISE_GPU_CULL", "0"), ("SUNRISE_VISBUFFER", "0")],
        ) {
            return;
        }
        with_client_game(|game| {
            let (_event_loop, window) = test_window([64, 64]);
            let mut renderer =
                loading_renderer(window, true, None, None, None, GraphicsApi::Vulkan).unwrap();
            renderer.set_section_cull(false).unwrap();
            game.set_renderer(Box::new(renderer));
            game.client.title.0 = None;
            game.client.screen = Screen::Playing;
            assert!(!sunrise_client_render::render_slots::needs().world_depth);

            mod_platform::__private::install(
                vec![ModEntry::new(
                    "mid_world_split",
                    Box::new(MidWorldSplitDepthProbe {
                        target: Mutex::new(None),
                        draws: Mutex::new(Vec::new()),
                    }),
                )]
                .into(),
            );
            let _ = take_hook_panics();
            let mut pipelines = Vec::new();
            sunrise_gameplay_common::game::native_hooks::native_register_render_pipelines(
                &mod_api::RegistrationContext::new(),
                &mut pipelines,
                &mut Vec::new(),
                &mut Vec::new(),
            );
            let vanilla = tempfile::tempdir().unwrap();
            std::fs::write(
                vanilla.path().join("render-pipelines.json"),
                r#"{"required":["minecraft:pipeline/solid_block"],"optional":["minecraft:pipeline/wireframe"]}"#,
            )
            .unwrap();
            sunrise_client_render::mod_pipelines::register(pipelines, vanilla.path()).unwrap();
            let shaders = tempfile::tempdir().unwrap();
            let core = shaders.path().join("assets/mid_world_split/shaders/core");
            std::fs::create_dir_all(&core).unwrap();
            std::fs::write(
                core.join("solid.vsh"),
                "#version 330\n\
                 in vec3 Pos;\n\
                 in vec4 Tint;\n\
                 out vec4 vTint;\n\
                 void main() {\n\
                     gl_Position = vec4(Pos, 1.0);\n\
                     vTint = Tint;\n\
                 }\n",
            )
            .unwrap();
            std::fs::write(
                core.join("solid.fsh"),
                "#version 330\n\
                 in vec4 vTint;\n\
                 out vec4 fragColor;\n\
                 void main() {\n\
                     fragColor = vTint;\n\
                 }\n",
            )
            .unwrap();
            let mut assets = RenderAssets::unpublished();
            assets.mod_pipelines = sunrise_client_render::mod_pipelines::Pipelines::load(
                &sunrise_assets::resource_source::ResourceSource::directory(
                    shaders.path().to_owned(),
                ),
                7,
            )
            .unwrap();
            let mut renderer = game.client.renderer.0.take().unwrap();
            renderer
                .build_mod_pipelines(&assets.mod_pipelines, &|_| None)
                .unwrap();
            game.client.client_pack.render = Arc::new(assets);

            // White opaque wall 2 blocks ahead of the camera (reversed-Z ~0.02495).
            let mut wall = SectionMesh::default();
            for (pos, uv) in [[0., 0., 4.], [0., 16., 4.], [16., 16., 4.], [16., 0., 4.]]
                .into_iter()
                .zip([[0., 1.], [0., 0.], [1., 0.], [1., 1.]])
            {
                wall.solid
                    .push(TerrainVertex::new(pos, uv, 0, u32::MAX, 0xf0f0));
            }
            wall.sort_solid();
            renderer.load_column(0, 0);
            renderer.upload((0, 10, 0), &wall);
            renderer.screenshot = Some(sunrise_client::gfx::ScreenshotRequest {
                path: "unused.png".into(),
                size: None,
                scaled: false,
                background: false,
            });
            let hud = sunrise_client::hud::HudBatch::scratch([
                renderer.config.width,
                renderer.config.height,
            ]);
            let mut frame = loading_frame(&hud);
            frame.title = false;
            frame.distance = 64.0;
            frame.detail_distance = 64.0;
            frame.camera = DVec3::new(8.0, 104.0, 2.0);
            frame.eye = frame.camera;
            frame.pitch = 0.0;
            frame.eye_rotation = [0.0, 0.0];
            let camera = test_camera(
                frame.camera.to_array(),
                sunrise_client::camera::view_rotation_matrix(0.0, 0.0).0,
            );
            let mut draw = WorldGeometry::default();
            draw.reset(
                camera.position,
                Mat4::from_cols_array(&camera.view),
                [0.0; 2],
            );
            let mut hooks = GameWorldStageDriver::new(game, &camera, 0.0, &mut draw);
            assert!(renderer.render(&frame, Some(&mut hooks)).unwrap());
            assert_eq!(
                renderer.drawn_sections, 1,
                "the opaque wall section must draw"
            );
            assert!(
                renderer.section_draws > 0,
                "the opaque wall must issue a draw"
            );
            drop(hooks);
            renderer.wait_submissions();
            let panics = take_hook_panics();
            assert!(panics.is_empty(), "{panics:?}");

            let mut pending = renderer.screenshot_result.take().unwrap().unwrap();
            let image = pending.image_mut();
            let pixel = |x: usize, y: usize| -> [u8; 4] {
                let at = (y * image.width as usize + x) * 4;
                image.rgba[at..at + 4].try_into().unwrap()
            };
            // Top-left corner: AfterOpaqueFeatures ran and drew the in-front red quad.
            assert_eq!(pixel(4, 4), [255, 0, 0, 255]);
            // Right-of-center pixel (32, 32) stays identical to its symmetric uncovered wall
            // neighbour (31, 32): the behind-wall green quad was occluded after the split.
            let wall_left = pixel(31, 32);
            let wall_right = pixel(32, 32);
            assert_ne!(wall_right, [0, 255, 0, 255]);
            assert!(
                wall_left[0] > 0 && wall_left[2] > 0,
                "expected white wall pixel, got {wall_left:?}"
            );
            assert_eq!(
                wall_right, wall_left,
                "opaque wall must occlude the behind-wall quad after a mid-world split"
            );

            drop(game.client.mod_gpu.take());
            drop(renderer);
        });
    }

    #[test]
    fn world_depth_binding_refuses_depth_writes_and_missing_read_only_depth_stencil() {
        assert_eq!(
            check_world_depth_binding(true, true, true),
            Err(PipelineDrawError::InvalidBinding)
        );
        assert_eq!(
            check_world_depth_binding(true, false, false),
            Err(PipelineDrawError::InvalidBinding)
        );
        assert_eq!(
            check_world_depth_binding(false, true, false),
            Err(PipelineDrawError::TargetReadWriteHazard)
        );
        assert_eq!(check_world_depth_binding(false, true, true), Ok(()));
        assert_eq!(check_world_depth_binding(false, false, false), Ok(()));
    }

    struct InWorldDepthSampleProbe {
        draws: Mutex<Vec<mod_api::PreparedWorldDraw>>,
    }

    #[mod_api::__subscriptions]
    impl Hooks for InWorldDepthSampleProbe {
        fn register_render_pipelines(
            &self,
            _: &mod_api::RegistrationContext,
            event: &mut mod_api::RegisterRenderPipelinesEvent<'_>,
        ) {
            let sample_fmt = mod_api::VertexFormat::builder(mod_api::VertexStepMode::Vertex)
                .attribute("Pos", mod_api::VertexAttributeFormat::Float32x3)
                .attribute("Tint", mod_api::VertexAttributeFormat::Float32x4)
                .build()
                .unwrap();
            event.register(
                mod_api::RenderPipeline::new(
                    Identifier::parse("world_depth_in_world:sample").unwrap(),
                    Identifier::parse("world_depth_in_world:core/sample").unwrap(),
                    mod_api::PipelineBlend::Opaque,
                    mod_api::WorldDepth::Tested,
                )
                .vertex_binding(0, sample_fmt)
                .sampler(
                    "DepthTex",
                    mod_api::Sampler::new(
                        mod_api::SamplerFilter::Nearest,
                        mod_api::SamplerAddress::ClampToEdge,
                    ),
                ),
            );
            let corner_fmt = mod_api::VertexFormat::builder(mod_api::VertexStepMode::Vertex)
                .attribute("Pos", mod_api::VertexAttributeFormat::Float32x3)
                .build()
                .unwrap();
            event.register(
                mod_api::RenderPipeline::new(
                    Identifier::parse("world_depth_in_world:corner").unwrap(),
                    Identifier::parse("world_depth_in_world:core/corner").unwrap(),
                    mod_api::PipelineBlend::Opaque,
                    mod_api::WorldDepth::Tested,
                )
                .depth_write(true)
                .vertex_binding(0, corner_fmt),
            );
        }

        fn prepare_render_buffers(
            &self,
            host: &dyn ClientHost,
            event: &mut mod_api::PrepareRenderBuffersEvent<'_>,
        ) {
            #[repr(C)]
            #[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
            struct PosTint([f32; 3], [f32; 4]);

            let sample = host
                .render_assets()
                .render_pipeline(&Identifier::parse("world_depth_in_world:sample").unwrap())
                .unwrap();
            let corner = host
                .render_assets()
                .render_pipeline(&Identifier::parse("world_depth_in_world:corner").unwrap())
                .unwrap();
            let depth_bindings = mod_api::DrawBindings::default()
                .texture("DepthTex", mod_api::TextureSource::WorldDepth);

            // In front of the opaque wall (reversed-Z 0.5 >= ~0.02495): passes read-only depth test.
            let front = [
                PosTint([-1.0, -1.0, 0.5], [0.0, 0.4, 0.0, 1.0]),
                PosTint([1.0, -1.0, 0.5], [0.0, 0.4, 0.0, 1.0]),
                PosTint([1.0, 1.0, 0.5], [0.0, 0.4, 0.0, 1.0]),
                PosTint([-1.0, 1.0, 0.5], [0.0, 0.4, 0.0, 1.0]),
            ];
            // Behind the opaque wall (reversed-Z 0.01 < ~0.02495): rejected by read-only depth test.
            let behind = [
                PosTint([-1.0, -1.0, 0.01], [0.0, 1.0, 0.0, 1.0]),
                PosTint([1.0, -1.0, 0.01], [0.0, 1.0, 0.0, 1.0]),
                PosTint([1.0, 1.0, 0.01], [0.0, 1.0, 0.0, 1.0]),
                PosTint([-1.0, 1.0, 0.01], [0.0, 1.0, 0.0, 1.0]),
            ];
            // Top-left NDC quad drawn with depth writes on after the read-only pass.
            let top_left: [[f32; 3]; 4] = [
                [-1.0, 0.5, 0.5],
                [-0.5, 0.5, 0.5],
                [-0.5, 1.0, 0.5],
                [-1.0, 1.0, 0.5],
            ];
            let draw_front = event
                .context
                .draw_vertices(
                    RenderLevelStage::AfterOpaqueBlocks,
                    sample,
                    &[bytemuck::cast_slice(&front)],
                    None,
                    mod_api::MeshDraw::vertices(0..4),
                    &depth_bindings,
                )
                .unwrap();
            let draw_behind = event
                .context
                .draw_vertices(
                    RenderLevelStage::AfterOpaqueBlocks,
                    sample,
                    &[bytemuck::cast_slice(&behind)],
                    None,
                    mod_api::MeshDraw::vertices(0..4),
                    &depth_bindings,
                )
                .unwrap();
            let draw_corner = event
                .context
                .draw_vertices(
                    RenderLevelStage::AfterOpaqueBlocks,
                    corner,
                    &[bytemuck::cast_slice(&top_left)],
                    None,
                    mod_api::MeshDraw::vertices(0..4),
                    &mod_api::DrawBindings::default(),
                )
                .unwrap();
            *self.draws.lock().unwrap() = vec![draw_front, draw_behind, draw_corner];
        }

        fn render_level_stage(
            &self,
            _: &dyn ClientHost,
            event: &mut mod_api::RenderLevelStageEvent<'_>,
        ) {
            if event.stage != RenderLevelStage::AfterOpaqueBlocks {
                return;
            }
            let draws: Vec<_> = self.draws.lock().unwrap().drain(..).collect();
            for draw in draws {
                event.context.world_pass().unwrap().draw(draw).unwrap();
            }
        }
    }

    #[test]
    #[ignore = "daily"]
    fn in_world_pipeline_samples_world_depth_in_read_only_depth_pass() {
        use super::super::native_entity_refs::tests::{test_window, with_client_game};
        use crate::game::loading_window::{loading_frame, loading_renderer};
        use mod_api::__private::{game::RegistrationContextNew, take_hook_panics};
        use std::sync::Arc;
        use sunrise_client::mesher::{SectionMesh, TerrainVertex};
        use sunrise_client_render::{assets::RenderAssets, graphics::GraphicsApi};
        use sunrise_gameplay_common::game::{Screen, test_fixtures::run_fresh_cases_with_env};

        if run_fresh_cases_with_env(
            "game::client_side::native_custom_geometry::tests::in_world_pipeline_samples_world_depth_in_read_only_depth_pass",
            "SUNRISE_IN_WORLD_DEPTH_SAMPLE_CHILD",
            &["run"],
            1,
            &[("SUNRISE_GPU_CULL", "0"), ("SUNRISE_VISBUFFER", "0")],
        ) {
            return;
        }
        with_client_game(|game| {
            let (_event_loop, window) = test_window([64, 64]);
            let mut renderer =
                loading_renderer(window, true, None, None, None, GraphicsApi::Vulkan).unwrap();
            renderer.set_section_cull(false).unwrap();
            assert!(
                renderer.mod_gpu().read_only_depth_stencil(),
                "Vulkan adapter must support READ_ONLY_DEPTH_STENCIL"
            );
            game.set_renderer(Box::new(renderer));
            game.client.title.0 = None;
            game.client.screen = Screen::Playing;
            assert!(!sunrise_client_render::render_slots::needs().world_depth);

            mod_platform::__private::install(
                vec![ModEntry::new(
                    "world_depth_in_world",
                    Box::new(InWorldDepthSampleProbe {
                        draws: Mutex::new(Vec::new()),
                    }),
                )]
                .into(),
            );
            let _ = take_hook_panics();
            let mut pipelines = Vec::new();
            sunrise_gameplay_common::game::native_hooks::native_register_render_pipelines(
                &mod_api::RegistrationContext::new(),
                &mut pipelines,
                &mut Vec::new(),
                &mut Vec::new(),
            );
            let vanilla = tempfile::tempdir().unwrap();
            std::fs::write(
                vanilla.path().join("render-pipelines.json"),
                r#"{"required":["minecraft:pipeline/solid_block"],"optional":["minecraft:pipeline/wireframe"]}"#,
            )
            .unwrap();
            sunrise_client_render::mod_pipelines::register(pipelines, vanilla.path()).unwrap();
            let shaders = tempfile::tempdir().unwrap();
            let core = shaders
                .path()
                .join("assets/world_depth_in_world/shaders/core");
            std::fs::create_dir_all(&core).unwrap();
            std::fs::write(
                core.join("sample.vsh"),
                "#version 330\n\
                 in vec3 Pos;\n\
                 in vec4 Tint;\n\
                 out vec4 vTint;\n\
                 void main() {\n\
                     gl_Position = vec4(Pos, 1.0);\n\
                     vTint = Tint;\n\
                 }\n",
            )
            .unwrap();
            std::fs::write(
                core.join("sample.fsh"),
                "#version 330\n\
                 uniform sampler2D DepthTex;\n\
                 in vec4 vTint;\n\
                 out vec4 fragColor;\n\
                 void main() {\n\
                     float d = textureLod(DepthTex, vec2(0.5), 0.0).r;\n\
                     fragColor = vec4(d, vTint.g, 1.0 - d, 1.0);\n\
                 }\n",
            )
            .unwrap();
            std::fs::write(
                core.join("corner.vsh"),
                "#version 330\n\
                 in vec3 Pos;\n\
                 void main() {\n\
                     gl_Position = vec4(Pos, 1.0);\n\
                 }\n",
            )
            .unwrap();
            std::fs::write(
                core.join("corner.fsh"),
                "#version 330\n\
                 out vec4 fragColor;\n\
                 void main() {\n\
                     fragColor = vec4(0.0, 1.0, 0.0, 1.0);\n\
                 }\n",
            )
            .unwrap();
            let mut assets = RenderAssets::unpublished();
            assets.mod_pipelines = sunrise_client_render::mod_pipelines::Pipelines::load(
                &sunrise_assets::resource_source::ResourceSource::directory(
                    shaders.path().to_owned(),
                ),
                7,
            )
            .unwrap();
            let mut renderer = game.client.renderer.0.take().unwrap();
            renderer
                .build_mod_pipelines(&assets.mod_pipelines, &|_| None)
                .unwrap();
            game.client.client_pack.render = Arc::new(assets);

            // White opaque wall 2 blocks ahead of the camera (reversed-Z ~0.02495 -> red 6).
            let mut wall = SectionMesh::default();
            for (pos, uv) in [[0., 0., 4.], [0., 16., 4.], [16., 16., 4.], [16., 0., 4.]]
                .into_iter()
                .zip([[0., 1.], [0., 0.], [1., 0.], [1., 1.]])
            {
                wall.solid
                    .push(TerrainVertex::new(pos, uv, 0, u32::MAX, 0xf0f0));
            }
            wall.sort_solid();
            renderer.load_column(0, 0);
            renderer.upload((0, 10, 0), &wall);
            renderer.screenshot = Some(sunrise_client::gfx::ScreenshotRequest {
                path: "unused.png".into(),
                size: None,
                scaled: false,
                background: false,
            });
            let hud = sunrise_client::hud::HudBatch::scratch([
                renderer.config.width,
                renderer.config.height,
            ]);
            let mut frame = loading_frame(&hud);
            frame.title = false;
            frame.distance = 64.0;
            frame.detail_distance = 64.0;
            frame.camera = DVec3::new(8.0, 104.0, 2.0);
            frame.eye = frame.camera;
            frame.pitch = 0.0;
            frame.eye_rotation = [0.0, 0.0];
            let camera = test_camera(
                frame.camera.to_array(),
                sunrise_client::camera::view_rotation_matrix(0.0, 0.0).0,
            );
            let mut draw = WorldGeometry::default();
            draw.reset(
                camera.position,
                Mat4::from_cols_array(&camera.view),
                [0.0; 2],
            );
            let mut hooks = GameWorldStageDriver::new(game, &camera, 0.0, &mut draw);
            assert!(renderer.render(&frame, Some(&mut hooks)).unwrap());
            assert_eq!(
                renderer.drawn_sections, 1,
                "the opaque wall section must draw"
            );
            assert!(
                renderer.section_draws > 0,
                "the opaque wall must issue a draw"
            );
            drop(hooks);
            renderer.wait_submissions();
            let panics = take_hook_panics();
            assert!(panics.is_empty(), "{panics:?}");

            let mut pending = renderer.screenshot_result.take().unwrap().unwrap();
            let image = pending.image_mut();
            let pixel = |x: usize, y: usize| -> [u8; 4] {
                let at = (y * image.width as usize + x) * 4;
                image.rgba[at..at + 4].try_into().unwrap()
            };
            // Center: sampled opaque wall depth (~0.02495 -> 6), front tint (0.4 -> 102),
            // 1 - depth (~0.975 -> 249); behind-wall draw (tint 1.0 -> 255) rejected by depth test.
            assert_eq!(pixel(32, 32), [6, 102, 249, 255]);
            // Top-left corner: writable depth pass resumed after the read-only pass.
            assert_eq!(pixel(4, 4), [0, 255, 0, 255]);

            drop(game.client.mod_gpu.take());
            drop(renderer);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn a_sampled_transient_the_pass_does_not_declare_in_its_reads_is_refused() {
        use crate::game::host::draw::GameFrameGraphContext;
        use mod_api::{ComputeDispatchError, DrawBindings, FrameGraphContext, FrameTarget};
        use sunrise_assets::resource_source::{Resource, ResourceSource};
        use sunrise_client::gfx::mod_pipelines::{PipelineVertex, ResolvedSet1Entry};
        use sunrise_client::world_geometry::FrameGraphSurface;

        // Pipeline registration is process-global; publish only in the fresh child.
        if super::super::native_entity_refs::tests::run_fresh_cases(
            "game::client_side::native_custom_geometry::tests::a_sampled_transient_the_pass_does_not_declare_in_its_reads_is_refused",
            "SUNRISE_UNDECLARED_TRANSIENT_READ_CHILD",
            &["run"],
        ) {
            return;
        }
        super::super::native_entity_refs::tests::with_client_game(|game| {
            let (_event_loop, window) =
                crate::game::native_hooks::particle_count_tests::hidden_window();
            game.common.options.offscreen = true;
            let renderer = game.new_renderer(window.clone()).unwrap();
            game.set_renderer(renderer);
            game.client.window.0 = Some(window);

            let draw_id = Identifier::parse("transient_probe:core/sample").unwrap();
            let compute_id = Identifier::parse("transient_probe:sample").unwrap();
            let layout = mod_api::BindGroupLayout::builder()
                .uniform_buffer("ProbeParams", 16)
                .sampler(
                    "InputTex",
                    mod_api::Sampler::new(
                        mod_api::SamplerFilter::Nearest,
                        mod_api::SamplerAddress::ClampToEdge,
                    ),
                );
            let draw_pipeline = mod_api::RenderPipeline::new(
                draw_id.clone(),
                draw_id.clone(),
                mod_api::PipelineBlend::Opaque,
                mod_api::WorldDepth::ThroughWalls,
            )
            .bind_group_layout(layout.clone().build().unwrap());
            let compute_pipeline =
                mod_api::ComputePipeline::builder(compute_id.clone(), compute_id.clone())
                    .bind_group_layout(
                        layout
                            .entry(mod_api::BindEntry::new(
                                "Out",
                                mod_api::BindingKind::StorageBuffer {
                                    read_only: false,
                                    min_size: 4,
                                },
                            ))
                            .build()
                            .unwrap(),
                    )
                    .build();
            let vanilla = tempfile::tempdir().unwrap();
            std::fs::write(
                vanilla.path().join("render-pipelines.json"),
                r#"{"required":["minecraft:pipeline/solid_block"],"optional":["minecraft:pipeline/wireframe"]}"#,
            )
            .unwrap();
            sunrise_client_render::mod_pipelines::register(vec![draw_pipeline], vanilla.path())
                .unwrap();
            sunrise_client_render::mod_compute::register(vec![compute_pipeline]).unwrap();

            let shaders = ResourceSource::indexed(
                [
                    (
                        "assets/transient_probe/shaders/core/sample.vsh",
                        "#version 330
in vec3 Position;
in vec2 UV0;
in vec4 Color;
out vec2 vUv;
out vec4 vColor;
void main() {
    gl_Position = vec4(Position.xy, 0.5, 1.0);
    vUv = UV0;
    vColor = Color;
}
",
                    ),
                    (
                        "assets/transient_probe/shaders/core/sample.fsh",
                        "#version 330
layout(std140) uniform ProbeParams { vec4 Bias; };
uniform sampler2D InputTex;
in vec2 vUv;
in vec4 vColor;
out vec4 fragColor;
void main() { fragColor = texture(InputTex, vUv) * vColor + Bias; }
",
                    ),
                    (
                        "assets/transient_probe/shaders/compute/sample.csh",
                        "#version 450
layout(local_size_x = 1) in;
layout(std140) uniform ProbeParams { vec4 Bias; };
uniform sampler2D InputTex;
layout(std430) buffer Out { uint values[]; };
void main() { values[0] = floatBitsToUint(textureLod(InputTex, vec2(0.5), 0.0).r + Bias.x); }
",
                    ),
                ]
                .into_iter()
                .map(|(path, shader)| (path.into(), Resource::bytes(shader.as_bytes().to_vec())))
                .collect(),
            );
            let mut render = sunrise_client_render::assets::RenderAssets::unpublished();
            render.mod_pipelines =
                sunrise_client_render::mod_pipelines::Pipelines::load(&shaders, 7).unwrap();
            game.client
                .renderer
                .0
                .as_mut()
                .unwrap()
                .build_mod_pipelines(&render.mod_pipelines, &|_| None)
                .unwrap();
            let pipeline = render.mod_pipelines.resolve(&draw_id).unwrap();
            let compute = render.mod_pipelines.compute().resolve(&compute_id).unwrap();
            game.client.client_pack.render = std::sync::Arc::new(render);

            let storage = game
                .create_storage_buffer(&mod_api::StorageBufferDescriptor::new(16))
                .unwrap();
            let vertices =
                [[-1.0, -1.0], [1.0, -1.0], [1.0, 1.0], [-1.0, 1.0]].map(|[x, y]| PipelineVertex {
                    pos: [x, y, 0.0],
                    uv: [(x + 1.0) / 2.0, (y + 1.0) / 2.0],
                    color: u32::MAX,
                });
            let inline = [0u8; 16];

            // The refusals below and the cycle after them log through the production warning
            // set; this test process is fresh, so installing its logger changes nothing shared.
            static WARNINGS: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());
            struct Capture;
            impl log::Log for Capture {
                fn enabled(&self, metadata: &log::Metadata<'_>) -> bool {
                    metadata.level() == log::Level::Warn
                }
                fn log(&self, record: &log::Record<'_>) {
                    let text = record.args().to_string();
                    if self.enabled(record.metadata()) && text.starts_with("Sunrise: frame graph") {
                        WARNINGS.lock().unwrap().push(text);
                    }
                }
                fn flush(&self) {}
            }
            log::set_logger(&Capture).unwrap();
            log::set_max_level(log::LevelFilter::Warn);

            // Per case: whether the draw declares the read, lists the transient in `.writes`,
            // or renders into it, and what the draw returns. The compute dispatch, which samples
            // the transient's depth, follows the first two cases only.
            for (declared, writes_only, own_attachment, expected) in [
                (true, false, false, Ok(())),
                (false, false, false, Err(PipelineDrawError::InvalidBinding)),
                (false, true, false, Err(PipelineDrawError::InvalidBinding)),
                (
                    false,
                    false,
                    true,
                    Err(PipelineDrawError::TargetReadWriteHazard),
                ),
            ] {
                let accepted = expected.is_ok();
                let with_compute = !writes_only && !own_attachment;
                let mut surface = FrameGraphSurface::default();
                let mut graph = GameFrameGraphContext::new(game, &mut surface);
                let input = graph
                    .create_transient(&mod_api::RenderTargetDescriptor::rgba8(
                        [8, 8],
                        mod_api::RenderTargetDepth::Depth32Float,
                    ))
                    .unwrap();
                let target = if own_attachment {
                    input
                } else {
                    graph
                        .create_transient(&mod_api::RenderTargetDescriptor::rgba8(
                            [8, 8],
                            mod_api::RenderTargetDepth::None,
                        ))
                        .unwrap()
                };
                let mut draw =
                    mod_api::FramePassDescriptor::new("draw", FrameTarget::Transient(target));
                if declared {
                    draw = draw.reads(FrameTarget::Transient(input));
                }
                if writes_only {
                    draw = draw.writes(FrameTarget::Transient(input));
                }

                // The valid inline uniform is first in both layouts, so a partial append by
                // the draw recorder would leave bytes even if it later refused the sampler.
                let draw_bindings = DrawBindings::new()
                    .inline_uniform("ProbeParams", &inline)
                    .texture(
                        "InputTex",
                        mod_api::TextureSource::TransientTargetColor(input, 0),
                    );
                graph
                    .add_pass(draw, &mut |pass| {
                        assert_eq!(
                            pass.draw_vertices(
                                pipeline,
                                &[bytemuck::cast_slice(&vertices)],
                                None,
                                mod_api::MeshDraw::vertices(0..4),
                                &draw_bindings,
                            ),
                            expected,
                        );
                    })
                    .unwrap();
                if with_compute {
                    // Declaring another read does not declare this transient.
                    let dispatch =
                        mod_api::ComputePassDescriptor::new("compute").reads(if declared {
                            FrameTarget::Transient(input)
                        } else {
                            FrameTarget::MainDepth
                        });
                    let compute_bindings = DrawBindings::new()
                        .inline_uniform("ProbeParams", &inline)
                        .texture(
                            "InputTex",
                            mod_api::TextureSource::TransientTargetDepth(input),
                        )
                        .storage_buffer("Out", storage.slice(0, 16));
                    graph
                        .add_compute_pass(dispatch, &mut |node| {
                            assert_eq!(
                                node.dispatch(compute, &compute_bindings, [1, 1, 1]),
                                if declared {
                                    Ok(())
                                } else {
                                    Err(ComputeDispatchError::InvalidBinding)
                                },
                            );
                        })
                        .unwrap();
                }

                let draw = &surface.graph.passes[0];
                if accepted {
                    assert_eq!(draw.draws.len(), 1);
                    assert_eq!(draw.inline_uniforms, inline);
                    assert!(matches!(
                        draw.draws[0].bindings.as_ref().unwrap().entries.as_slice(),
                        [ResolvedSet1Entry::InlineUniform { .. }, ResolvedSet1Entry::TransientColor { target, attachment: 0 }]
                            if *target == input
                    ));
                } else {
                    assert!(draw.draws.is_empty());
                    assert!(draw.transient_vertices.is_empty());
                    assert!(draw.inline_uniforms.is_empty());
                }
                if with_compute {
                    let dispatches = surface.graph.passes[1].dispatches.as_deref().unwrap();
                    if accepted {
                        assert_eq!(dispatches.len(), 1);
                        assert_eq!(dispatches[0].inline, inline);
                        assert!(matches!(
                            dispatches[0].entries.as_slice(),
                            [ResolvedSet1Entry::InlineUniform { .. }, ResolvedSet1Entry::TransientDepth { target }, ResolvedSet1Entry::StorageBuffer { .. }]
                                if *target == input
                        ));
                    } else {
                        assert!(dispatches.is_empty());
                    }
                }
            }

            // A cycle whose pass labels are exactly the old string-prefixed key of the refusal
            // above ("undeclared transient read", "draw", "0"), refused twice: it is admitted
            // once, and it neither suppresses nor is suppressed by the refusal of pass "draw"
            // sampling transient 0 that was logged during the cases.
            let mut surface = FrameGraphSurface::default();
            let mut graph = GameFrameGraphContext::new(game, &mut surface);
            let output = graph
                .create_transient(&mod_api::RenderTargetDescriptor::rgba8(
                    [8, 8],
                    mod_api::RenderTargetDepth::None,
                ))
                .unwrap();
            let first = graph
                .add_pass(
                    mod_api::FramePassDescriptor::new(
                        "undeclared transient read",
                        FrameTarget::Transient(output),
                    ),
                    &mut |_| {},
                )
                .unwrap();
            let second = graph
                .add_pass(
                    mod_api::FramePassDescriptor::new("draw", FrameTarget::Transient(output))
                        .after_pass(first),
                    &mut |_| {},
                )
                .unwrap();
            for _ in 0..2 {
                assert_eq!(
                    graph
                        .add_pass(
                            mod_api::FramePassDescriptor::new("0", FrameTarget::Transient(output))
                                .after_pass(second)
                                .before_pass(first),
                            &mut |_| {},
                        )
                        .unwrap_err(),
                    mod_api::FrameGraphError::Cycle {
                        passes: vec![
                            "undeclared transient read".to_owned(),
                            "draw".to_owned(),
                            "0".to_owned(),
                        ],
                    },
                );
            }

            // Each warning kind is logged once however often it recurs: the draw refusals of
            // the second and third cases share the first one's key, and the second cycle
            // attempt shares the first one's.
            assert_eq!(
                *WARNINGS.lock().unwrap(),
                [
                    "Sunrise: frame graph pass draw samples transient target 0 without declaring it in its reads; binding refused",
                    "Sunrise: frame graph pass compute samples transient target 0 without declaring it in its reads; binding refused",
                    "Sunrise: frame graph cycle detected between undeclared transient read, draw, 0",
                ]
            );
            drop(game.client.renderer.0.take());
            drop(game.client.mod_gpu.take());
        });
    }
}
