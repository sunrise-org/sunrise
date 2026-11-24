//! The typed submission surface; renderer storage is never exposed to mods.
use super::Game;
use super::mods;
use crate::game::host::draw::GameWorldDrawContext;
use glam::{DVec3, Mat4, Vec3};
use mod_api::{
    DrawError, InstanceError, ModelInstance, ModelRef, NameTagOptions, PipelineDrawError,
    PipelineQuad, RenderLevelStage, RenderPipelineRef, WorldLine, WorldQuad,
};
use sunrise_client::mesher::Vertex;
use sunrise_client::screen_effects::{CameraQuads, sampling};
use sunrise_client::terrain_model;
use sunrise_client::world_geometry::{self, ModelBatch, WorldGeometry};

/// Drives mod world rendering hooks from `Renderer::draw_frame`'s real visibility, sky, prepare,
/// pass-stage, and post-world boundaries.
pub(super) struct GameWorldStageDriver<'a, 'g> {
    game: &'a Game<'g>,
    frame_camera: &'a mod_api::Camera,
    alpha: f64,
    draw: &'a mut WorldGeometry,
}

impl<'a, 'g> GameWorldStageDriver<'a, 'g> {
    pub(super) fn new(
        game: &'a Game<'g>,
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
        camera: &'v mod_api::Camera,
        alpha: f64,
        game_time: i64,
        sections: &'v [mod_api::RenderableSection],
        oit_on: bool,
    ) -> mod_api::WorldRenderView<'v> {
        mod_api::WorldRenderView::new(camera, alpha as f32, game_time, oit_on, sections)
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
        let view = Self::view(
            self.frame_camera,
            self.alpha,
            self.game.world.game_time,
            sections,
            oit_on,
        );
        self.draw.begin_custom_geometry();
        self.game.native_submit_custom_geometry(&view, self.draw);
        self.game.submit_debug_geometry(&view, self.draw);
    }

    fn on_after_sky(&mut self, sections: &[mod_api::RenderableSection], oit_on: bool) {
        let view = Self::view(
            self.frame_camera,
            self.alpha,
            self.game.world.game_time,
            sections,
            oit_on,
        );
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
        let view = Self::view(
            self.frame_camera,
            self.alpha,
            self.game.world.game_time,
            sections,
            oit_on,
        );
        self.game.native_prepare_render_buffers(&view, self.draw);
        self.draw.finalize();
        self.game.resolve_submitted_groups(self.draw);
        let camera = DVec3::from(self.draw.camera());
        self.game
            .bake_model_instances(self.draw, camera, &mut out.entities, &mut out.entity_spans);
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
            .append(&mut self.game.prepared_model_draws.borrow_mut());
        out.text_patches
            .append(&mut self.game.text_patches.borrow_mut());
    }

    fn on_render_stage(
        &mut self,
        stage: RenderLevelStage,
        sections: &[mod_api::RenderableSection],
        oit_on: bool,
    ) -> &[sunrise_client::gfx::world_primitives::DrawRanges] {
        let view = Self::view(
            self.frame_camera,
            self.alpha,
            self.game.world.game_time,
            sections,
            oit_on,
        );
        let mut surface = self.draw.stage_surface(stage);
        self.game
            .native_render_level_stage(stage, &view, &mut surface);
        self.draw.stage_draws()
    }

    fn prepared_primitives(&self) -> sunrise_client::gfx::world_primitives::Batch<'_> {
        self.draw.batch_with(&self.game.pack.render.mod_pipelines)
    }

    fn on_after_level(&mut self, sections: &[mod_api::RenderableSection], oit_on: bool) {
        let view = Self::view(
            self.frame_camera,
            self.alpha,
            self.game.world.game_time,
            sections,
            oit_on,
        );
        let mut surface = self.draw.stage_surface(RenderLevelStage::AfterLevel);
        self.game
            .native_render_level_stage(RenderLevelStage::AfterLevel, &view, &mut surface);
    }
}

impl Game<'_> {
    /// A camera-space quad in solid `argb`, of the atlas's white texel.
    pub(super) fn native_camera_quad(
        &self,
        draw: &mut CameraQuads,
        corners: [[f32; 3]; 4],
        argb: u32,
    ) -> bool {
        self.native_camera_sprite(draw, "sunrise/white", corners, [[0.5; 2]; 4], argb)
    }
    /// A camera-space quad of sprite `name`, sampled as `ScreenEffects::build` samples it; false
    /// for an unknown sprite or a coordinate that is not finite.
    pub(super) fn native_camera_sprite(
        &self,
        draw: &mut CameraQuads,
        name: &str,
        corners: [[f32; 3]; 4],
        uv: [[f32; 2]; 4],
        argb: u32,
    ) -> bool {
        let Some(r) = self.gui.atlas.texture_rect(name) else {
            return false;
        };
        let flags = self.gui.atlas.flags.get(name).copied();
        let sampling = flags.map_or(0, sampling);
        draw.push_on_atlas(
            corners,
            uv,
            [r.u0, r.v0, r.u1, r.v1],
            argb,
            sampling,
            self.gui.atlas.texture_atlas_page(name),
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
            selection: &self.debug_entries,
            entries: &self.registered_debug_entries,
            overlay: self.debug,
            reduced: self.reduced_debug_info(),
        };
        let renderers = mods::debug_renderers(self.options.execution_role, &states);
        self.debug_renderers = self.debug_entries.active_renderers(renderers);
    }
    fn submit_debug_geometry(&self, view: &mod_api::WorldRenderView<'_>, draw: &mut WorldGeometry) {
        let mut context = GameWorldDrawContext::new(self, draw);
        for renderer in &self.debug_renderers {
            renderer.draw(self, view, &mut context);
        }
    }
    /// Whether any mod subscribes to world rendering hooks or active debug renderers.
    pub(super) fn has_world_render_hooks(&self) -> bool {
        !self.debug_renderers.is_empty()
            || mod_api::__private::listened(mod_api::HookId::SubmitCustomGeometry)
            || mod_api::__private::listened(mod_api::HookId::PrepareRenderBuffers)
            || mod_api::__private::listened(mod_api::HookId::RenderLevelStage)
    }
    /// `DebugRenderer.emitGizmos`'s first half: a selection the list has not seen refreshes it.
    pub(super) fn follow_debug_selection(&mut self) {
        let version = self.debug_entries.enabled_version();
        if self.debug_renderers_version != Some(version) {
            self.debug_renderers_version = Some(version);
            self.refresh_debug_renderers();
        }
    }
    /// One view's geometry, the last view's gone first. `title` views draw no world: the
    /// title and connect screens have no mod geometry, so none outlives leaving a world.
    #[cfg(test)]
    pub(super) fn fill_world_geometry(
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
        let white = self.gui.atlas.sprites["sunrise/white"];
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
        draw.set_time(((self.world.game_time.rem_euclid(72_000) as f64 + alpha) / 20.0) as f32);
        let view = mod_api::WorldRenderView::new(
            frame_camera,
            alpha as f32,
            self.world.game_time,
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
        self.see_through_vertices.extend_from_slice(see_through);
    }
    /// A mod's label as `SubmitNodeCollection.submitNameTag` draws a name: 0.5 above `at`,
    /// facing the camera, 0.025 blocks a font pixel. See-through, twice: opaque white text in
    /// the world, its light raised to at least 2 unless `exact_light` (no backdrop), and the
    /// faded text on the backdrop over everything, unlit. Otherwise once, depth tested and
    /// translucent: the faded text on the backdrop in the caller's light. The faded text's alpha
    /// (unless `faded_alpha`) and the backdrop's follow the player's text background opacity.
    /// Refused whole, before layout, for a position that is not finite; any newly
    /// rasterized glyphs are uploaded with the label.
    pub(super) fn native_world_name_tag(
        &self,
        draw: &mut WorldGeometry,
        at: [f64; 3],
        text: &mod_api::Text,
        options: NameTagOptions,
    ) -> Result<(), DrawError> {
        draw.check_label(at)?;
        let text = sunrise_client_lib::api_text::to_engine_inprocess(text);
        let from_camera = DVec3::from(at) - DVec3::from(draw.camera());
        let o = from_camera + DVec3::new(0.0, 0.5, 0.0);
        let lines = [(text, 0.0)];
        // Options.getBackgroundOpacity(0.25), and the faded text alpha that goes with it.
        let backdrop = if options.background {
            self.settings.text_background(0.25)
        } else {
            0
        };
        let faded = options
            .faded_alpha
            .map_or_else(|| self.settings.faded_text_alpha(0.25), u32::from);
        let light = options.light;
        let unit = super::names::NAME_TAG_UNIT;
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
        draw.label(from_camera.length_squared(), &world, &see_through);
        self.text_patches.borrow_mut().append(&mut patches);
        Ok(())
    }
    /// Submit unlit ARGB world lines atomically. Invalid coordinates or widths leave the batch
    /// unchanged.
    pub(super) fn native_world_draw_lines(
        &self,
        draw: &mut WorldGeometry,
        lines: &[WorldLine],
    ) -> Result<(), DrawError> {
        draw.lines(lines)
    }
    /// Submit unlit, alpha-blended world quads atomically, using the white atlas texel.
    /// Refuses non-finite coordinates; quad storage grows dynamically into the GPU buffers.
    pub(super) fn native_world_draw_quads(
        &self,
        draw: &mut WorldGeometry,
        quads: &[WorldQuad],
    ) -> Result<(), DrawError> {
        draw.quads(quads)
    }
    /// Submit quads drawn by a mod pipeline: the handle is checked against the published
    /// pipelines once, non-finite quads are skipped individually without rejecting valid quads
    /// in the same slice, and quad storage grows dynamically into the GPU buffers.
    pub(super) fn native_world_draw_pipeline_quads(
        &self,
        draw: &mut WorldGeometry,
        pipeline: RenderPipelineRef,
        quads: &[PipelineQuad],
    ) -> Result<(), PipelineDrawError> {
        let slot = self
            .pack
            .render
            .mod_pipelines
            .slot(pipeline)
            .map_err(PipelineDrawError::Pipeline)?;
        draw.pipeline_quads(slot, quads);
        Ok(())
    }
    /// Resolve `model` once, cull copies against the frustum first, and skip any individual
    /// invalid visible copy without rolling back valid copies.
    pub(super) fn native_world_draw_model_instances(
        &self,
        draw: &mut WorldGeometry,
        model: ModelRef,
        instances: &[ModelInstance],
    ) -> Result<(), InstanceError> {
        self.draw_cached_model_instances(draw, model, instances, None)
    }
    pub(super) fn draw_cached_model_instances(
        &self,
        draw: &mut WorldGeometry,
        model: ModelRef,
        instances: &[ModelInstance],
        cached: Option<&[sunrise_client::gfx::PreparedModelInstance]>,
    ) -> Result<(), InstanceError> {
        use mod_api::__private::game::Fields;
        let fields = model.fields();
        if fields.generation != self.standalone_geometry.generation() {
            return Err(InstanceError::Model(mod_api::ModelError::StaleModel));
        }
        let slot = self
            .standalone_geometry
            .instance_slot(fields.slot)
            .ok_or(InstanceError::Model(mod_api::ModelError::UnknownModel))?;
        let uploaded = self.renderer_prepared_generation();
        #[cfg(test)]
        let uploaded = uploaded.or_else(sunrise_client::gfx::test_prepared_generation);
        let gpu = super::native_block_entity_draw::prepared_route(
            self.pack_ids.is_some(),
            !self.native_shaders.picked.is_empty(),
            uploaded,
            fields.generation,
        );
        let mut staged = self.prepared_model_draws.borrow_mut();
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
            // other faces, another colour, translucency or a block state takes the CPU route.
            if copy.overlay != 0
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
            draw.submit_validated_model(batch, instances, self.view_frustum.as_ref(), stage_copy);
        } else {
            draw.submit_model(batch, instances, self.view_frustum.as_ref(), |copy| {
                stage_copy(0, copy)
            });
        }
        Ok(())
    }
    #[cfg(test)]
    pub(super) fn commit_model_instances(
        &self,
        draw: &mut WorldGeometry,
        camera: DVec3,
        entities: &mut Vec<Vertex>,
    ) {
        self.bake_model_instances(draw, camera, entities, &mut self.entity_spans.borrow_mut());
    }

    /// Bake CPU copies onto the entity stream with their pack spans. Opaque standalone copies
    /// go as block entities, one run per overlay. Material copies follow and the renderer sorts
    /// their translucent quads far to near.
    fn bake_model_instances(
        &self,
        draw: &mut WorldGeometry,
        camera: DVec3,
        entities: &mut Vec<Vertex>,
        spans: &mut Vec<sunrise_client::gfx::pack_draw::EntitySpan>,
    ) {
        let capacity = self.entity_quad_capacity();
        let reserved_quads = self.entity_batch_reserved_quads();
        let mut run_overlay = None;
        let mut materials = self.model_copy_order.borrow_mut();
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
            let Some(slot) = self.standalone_geometry.instance_slot(copy.slot) else {
                continue;
            };
            if slot.translucent || slot.state || copy.color >> 24 != 0xFF {
                materials.push(index as u32);
                continue;
            }
            let Some(faces) = self.standalone_geometry.faces(copy.slot) else {
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
            let layers = &self.pack.render.sprite_layers;
            let mut scope =
                sunrise_client::gfx::EntityQuadScope::new(entities, capacity, reserved_quads);
            for &index in materials.iter() {
                if !scope.has_room() {
                    break;
                }
                let copy = draw.copy(index as usize);
                let Some(faces) = self.model_faces(copy.slot) else {
                    continue;
                };
                let state = self
                    .standalone_geometry
                    .instance_slot(copy.slot)
                    .is_some_and(|slot| slot.state);
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
                submit_material_model_copy(&mut scope, faces, pose, copy.light, options, |name| {
                    layers
                        .get(name)
                        .or_else(|| layers.get(name.strip_prefix("minecraft:")?))
                        .copied()
                });
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
    faces: &[super::minecarts::PreparedModelFace],
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
    super::minecarts::emit_model_faces(
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
    super::minecarts::scale_tint_alpha(&mut entities[start..], shown(), tint);
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
        game: &'a Game<'g>,
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
        game: &Game<'_>,
        cpu_vertices: &[Vertex],
        instance: &sunrise_client::gfx::PreparedModelInstance,
        slot: u32,
        uploaded: u64,
        view: Mat4,
    ) -> (u8, [u8; 4], [u8; 4]) {
        let pixels = game.gui.atlas.pixels().to_vec();
        let size = [game.gui.atlas.width, game.gui.atlas.height];
        let cpu = sunrise_client::gfx::entity_pixel_test::render(
            cpu_vertices,
            size,
            &pixels,
            view,
            false,
        )
        .expect("software Vulkan entity pixels");
        let (vertices, ranges) = game.standalone_geometry.prepared_upload();
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

    fn grass(game: &Game<'_>) -> ModelRef {
        let model = game
            .native_model(&Identifier::parse("minecraft:block/grass_block").unwrap())
            .unwrap();
        let generation = mod_api::__private::game::Fields::fields(&model).generation;
        sunrise_client::gfx::set_test_prepared_generation(Some(generation));
        model
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
            assert!(game.prepared_model_draws.borrow().is_empty(), "no GPU copy");
            let slot = mod_api::__private::game::Fields::fields(&model).slot;
            let faces = game
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
                slot(game.world.air.0),
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
    fn a_tinted_block_state_carries_its_default_tint_in_the_baked_colour() {
        crate::game::in_own_process(
            "game::native_custom_geometry::tests::a_tinted_block_state_carries_its_default_tint_in_the_baked_colour",
            || {
                super::super::native_entity_refs::tests::with_client_game(|game| {
                    let colours = |name: &str| {
                        let state = game.pack.assets.registry.default_state(name).unwrap();
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
            game.view_frustum = Some(sunrise_client::gfx::test_frustum([
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
                assert_eq!(game.prepared_model_draws.borrow().len(), 1);
                game.prepared_model_draws.borrow_mut().clear();
            }
            let invalid_only = vec![ModelInstance::at([f64::NAN; 3], 0); 4];
            context(game, &mut draw)
                .draw_model_instances(model, &invalid_only)
                .unwrap();
            assert!(game.prepared_model_draws.borrow().is_empty());
            assert_eq!(draw.copy_count(), 0);
            let render = |game: &Game<'_>| {
                let staged = game.prepared_model_draws.borrow();
                assert_eq!(staged.len(), 1, "the far copy must be culled");
                let (vertices, ranges) = game.standalone_geometry.prepared_upload();
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
                        atlas_size: [game.gui.atlas.width, game.gui.atlas.height],
                        atlas: &game.gui.atlas.pixels(),
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
            game.prepared_model_draws.borrow_mut().clear();
            draw.reset([0.0; 3], Mat4::IDENTITY, [0.0; 2]);
            context(game, &mut draw).draw_instance_group(group).unwrap();
            context(game, &mut draw).draw_instance_group(group).unwrap();
            game.resolve_submitted_groups(&mut draw);
            assert_eq!(game.prepared_model_draws.borrow().len(), 2);
            draw.reset([0.0; 3], Mat4::IDENTITY, [0.0; 2]);
            game.prepared_model_draws.borrow_mut().clear();
            context(game, &mut draw).draw_instance_group(group).unwrap();
            game.resolve_submitted_groups(&mut draw);
            assert_eq!(expected, render(game));
            let bytes = || game.instance_groups.borrow().converted_bytes();
            let stride = std::mem::size_of::<sunrise_client::gfx::PreparedModelInstance>();
            let before = bytes();
            // An unchanged frame stages all visible records, but converts none.
            draw.reset([0.0; 3], Mat4::IDENTITY, [0.0; 2]);
            game.prepared_model_draws.borrow_mut().clear();
            context(game, &mut draw).draw_instance_group(group).unwrap();
            game.resolve_submitted_groups(&mut draw);
            assert_eq!(bytes(), before);
            assert_eq!(game.prepared_model_draws.borrow().len() * stride, stride);
            assert_eq!(expected, render(game));
            let mut changed = near;
            changed.tint = 0xFFFF_2040;
            changed.transform = [2.0, 0.0, 0.0, 0.2, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0];
            group.write(game, 1, &[changed]).unwrap();
            assert_eq!(bytes() - before, stride);
            draw.reset([0.0; 3], Mat4::IDENTITY, [0.0; 2]);
            game.prepared_model_draws.borrow_mut().clear();
            context(game, &mut draw)
                .draw_model_instances(model, &[far, changed])
                .unwrap();
            let rewritten = render(game);
            assert_ne!(expected, rewritten);
            draw.reset([0.0; 3], Mat4::IDENTITY, [0.0; 2]);
            game.prepared_model_draws.borrow_mut().clear();
            context(game, &mut draw).draw_instance_group(group).unwrap();
            game.resolve_submitted_groups(&mut draw);
            assert_eq!(rewritten, render(game));
            assert_eq!(game.prepared_model_draws.borrow().len() * stride, stride);
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
            game.prepared_model_draws.borrow_mut().clear();
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
            game.prepared_model_draws.borrow_mut().clear();
            context(game, &mut draw).draw_instance_group(group).unwrap();
            game.resolve_submitted_groups(&mut draw);
            assert_eq!(transformed, render(game));
            assert_eq!(game.prepared_model_draws.borrow().len() * stride, stride);
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
            game.prepared_model_draws.borrow_mut().clear();
            context(game, &mut draw).draw_instance_group(group).unwrap();
            game.resolve_submitted_groups(&mut draw);
            assert!(
                game.prepared_model_draws.borrow().is_empty(),
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
            game.prepared_model_draws.borrow_mut().clear();
            context(game, &mut draw).draw_instance_group(group).unwrap();
            game.resolve_submitted_groups(&mut draw);
            {
                let staged = game.prepared_model_draws.borrow();
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
            while game.pack_reload.is_some() && std::time::Instant::now() < deadline {
                game.drive_pack_reload();
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
            assert_ne!(
                game.standalone_geometry.generation(),
                model.fields().generation,
                "reload error: {:?}",
                game.ui.options.resource_error
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
            let pixels = |game: &Game<'_>, vertices: &[Vertex]| {
                sunrise_client::gfx::entity_pixel_test::render(
                    vertices,
                    [game.gui.atlas.width, game.gui.atlas.height],
                    &game.gui.atlas.pixels(),
                    Mat4::IDENTITY,
                    false,
                )
                .expect("software Vulkan CPU group pixels")
            };
            // Both shader selection and unavailable GPU records force the shared CPU route.
            // Overlay copies also use it even when GPU records are available.
            for route in 0..3 {
                game.native_shaders.picked = if route == 0 {
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
                assert!(game.prepared_model_draws.borrow().is_empty());
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
            game.settings.mods.disabled.insert("group-owner".into());
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
            game.settings.mods.disabled.remove("group-owner");
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
            let staged = game.prepared_model_draws.borrow();
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
                .standalone_geometry
                .faces(slot)
                .expect("prepared faces");
            let mut cpu_vertices = Vec::new();
            super::super::minecarts::emit_model_faces(
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
            let staged = game.prepared_model_draws.borrow();
            let instance = staged[0].instance;
            let slot = staged[0].slot;
            let uploaded = staged[0].generation;
            drop(staged);
            game.prepared_model_draws.borrow_mut().clear();
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
            assert!(game.prepared_model_draws.borrow().is_empty(), "no GPU copy");
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
            let staged = game.prepared_model_draws.borrow();
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
            game.view_frustum = Some(sunrise_client::gfx::test_frustum([
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
                game.prepared_model_draws.borrow().len(),
                1,
                "only the visible copy is staged"
            );
            let mut committed = Vec::new();
            game.commit_model_instances(&mut draw, DVec3::ZERO, &mut committed);
            assert_eq!(draw.copy_count(), 0, "commit takes the copies");
            assert!(committed.is_empty(), "the staged copy is not also baked");
            game.prepared_model_draws.borrow_mut().clear();
            draw.reset([0.0; 3], Mat4::IDENTITY, [0.0, 0.0]);
            {
                let mut ctx = context(game, &mut draw);
                ctx.draw_model_instances(model, &[near]).unwrap();
                let mut bad = near;
                bad.light.block = 16;
                // Invalid visible copies are skipped individually; valid copies stay staged.
                ctx.draw_model_instances(model, &[near, bad]).unwrap();
                assert_eq!(game.prepared_model_draws.borrow().len(), 2);
                // Copies past the GPU slot cap fall back to CPU baking instead of failing.
                ctx.draw_model_instances(model, &vec![near; 16_384])
                    .unwrap();
                assert_eq!(game.prepared_model_draws.borrow().len(), 16_384);
                assert_eq!(ctx.surface.copy_count(), 2);
                game.prepared_model_draws.borrow_mut().clear();
                ctx.surface.reset([0.0; 3], Mat4::IDENTITY, [0.0, 0.0]);
                // Large off-screen slices cull cleanly without error.
                ctx.draw_model_instances(model, &vec![far; 16_385]).unwrap();
                assert_eq!(game.prepared_model_draws.borrow().len(), 0);
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
            assert_eq!(game.debug_renderers.len(), 1);
            game.debug_renderers_version = Some(game.debug_entries.enabled_version());
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
            game.debug_renderers.clear();
            game.debug_renderers_version = None;
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
            "game::native_custom_geometry::tests::world_render_view_lends_frame_state_and_prepare_updates_instance_groups",
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
            game.view_frustum = None;
            game.world.game_time = 12_345;
            *PREPARE_MODEL.lock().unwrap() = Some(model);
            *PREPARE_GROUP.lock().unwrap() = None;
            PREPARE_EVENTS.lock().unwrap().clear();

            mod_platform::__private::install(
                vec![ModEntry::new("prepare_view", Box::new(PrepareViewMod))].into(),
            );
            game.refresh_debug_renderers();
            assert_eq!(game.debug_renderers.len(), 1);
            game.debug_renderers_version = Some(game.debug_entries.enabled_version());

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
            game.prepared_model_draws.borrow_mut().clear();
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
                let staged = game.prepared_model_draws.borrow();
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
            game.prepared_model_draws.borrow_mut().clear();
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
                let staged = game.prepared_model_draws.borrow();
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
            game.debug_renderers.clear();
            game.debug_renderers_version = None;
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
                game.prepared_model_draws.borrow().len(),
                1,
                "fill_world_geometry must commit the accepted copy"
            );
            assert_eq!(draw.copy_count(), 0);
            assert_eq!(entities.len(), 4);
            assert_eq!(game.prepared_model_draws.borrow()[0].cpu_vertices, 4);
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
        let prepared_face = super::super::minecarts::PreparedModelFace::new(
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
                assert_eq!(game.debug_renderers.len(), 2);

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
}
