use super::*;

impl Renderer {
    /// Runs `post` over the world from now on (`None`: no pack), for the dimension `dimension`;
    /// `block_ids` gives each source state its model and fluid material ids (two values
    /// per state); emission and ownership come from the mesher's quad records.
    /// Returns the programs that run and the notes of loading them, or why the pack can't run:
    /// the dimension then runs without a pack.
    pub fn set_shader_pack(
        &mut self,
        dimension: i32,
        pack: Option<sunrise_shaders::Post>,
        block_ids: &[i32],
    ) -> std::result::Result<Option<(Vec<String>, Vec<String>)>, pack_post::Refused> {
        if pack.is_none() {
            self.forget_pack_atlas();
        }
        self.pack_served = pack.as_ref().and_then(|p| p.hdr);
        let draws = self.pack_terrain_draws();
        let buffers = pack_terrain_buffers();
        let buffers = &buffers[..terrain_buffer_count(draws)];
        let flat = self.pack_terrain_flat;
        let pages = draws.reads_pages();
        let sprite_rects = self
            .pack_atlas
            .as_ref()
            .map_or_else(Vec::new, |a| a.rects_table());
        let post = pack.map(|post| {
            let world = pack_post::World {
                atlas: self.pack_atlas.as_ref().map(|a| (a.view.clone(), a.size())),
                sprite_rects: &sprite_rects,
                material_maps: self
                    .pack_material_atlases
                    .as_ref()
                    .map(|a| [a[0].view.clone(), a[1].view.clone()]),
                lightmap: self.lightmap_view.clone(),
                gui_size: self
                    .hud
                    .atlas_texture
                    .as_ref()
                    .map_or([1, 1], |t| [t.width(), t.height()]),
                terrain: self.pack_atlas.as_ref().map(|_| pack_post::Geometry {
                    // Flat terrain programs pull their vertices (`pack_atlas::PULL_FLAT`).
                    buffers: if flat { &[] } else { buffers },
                    table: self.flat.as_ref().filter(|_| flat).map(|f| &f.draw_layout),
                    depth_write: true,
                    atlas: None,
                    material_maps: true,
                    ids: None,
                    cull: true,
                    pages,
                }),
                block_ids,
                celestial: self
                    .hud
                    .atlas_texture
                    .as_ref()
                    .map(|t| t.create_view(&Default::default())),
                clouds: Some(&self.cloud_bindings),
                weather: true,
                hand: true,
            };
            let mod_runtime = crate::mod_pipelines::pack_runtime_specs();
            let mod_vertex_buffers = [Some(mod_pipelines::PipelineVertex::buffer_layout())];
            let mod_set1_layouts: Vec<Option<&wgpu::BindGroupLayout>> = self
                .world_primitives
                .as_ref()
                .map(|wp| wp.pipelines.set1_bind_group_layouts())
                .unwrap_or_default();
            let mod_vertex_stages = self
                .world_primitives
                .as_ref()
                .map(|wp| wp.pipelines.pack_vertex_stages())
                .unwrap_or_default();
            pack_post::PackPost::new_with_mods(
                (&self.device, &self.queue, &self.adapter),
                post,
                self.output.frame.format,
                world,
                mod_runtime
                    .as_runtime_with_layouts(&mod_vertex_buffers, &mod_set1_layouts)
                    .with_vertex_stages(&mod_vertex_stages),
                &std::sync::atomic::AtomicBool::new(false),
            )
        });
        self.pack_loading = None;
        self.install_shader_pack(dimension, post.transpose(), block_ids.to_vec(), None)
    }
    /// Runs the pack `set_shader_pack` or `load_shader_pack` built (`None`: no pack), or none when
    /// it can't run, for the dimension `dimension`; the programs that run and their notes.
    /// `block_ids` are the pack's, as `set_shader_pack` takes them. The mod pipelines' pack
    /// state is published with a pack that runs (`mod_pack_state`, built with the pack, or
    /// built here), and dropped with one that doesn't.
    fn install_shader_pack(
        &mut self,
        dimension: i32,
        post: std::result::Result<Option<pack_post::PackPost>, pack_post::Refused>,
        block_ids: Vec<i32>,
        mod_pack_state: Option<mod_pipelines::BuiltPackState>,
    ) -> std::result::Result<Option<(Vec<String>, Vec<String>)>, pack_post::Refused> {
        self.pack_changed();
        let runs = post.as_ref().is_ok_and(Option::is_some);
        if let Some(world_primitives) = self.world_primitives.as_mut() {
            if runs {
                (world_primitives.pipelines).select_pack(&self.device, mod_pack_state);
            } else {
                world_primitives.pipelines.deselect_pack();
            }
        }
        self.replace_pack_block_ids(if runs { block_ids } else { Vec::new() }, runs);
        let mut post = match post {
            Ok(post) => post,
            Err(e) => {
                self.pack_post = Some((dimension, None));
                self.forget_pack_atlas();
                return Err(e);
            }
        };
        if let Some(post) = post.as_mut() {
            post.set_paged_maps(
                (self.shared_material_maps.each_ref())
                    .map(|t| t.as_ref().map(|t| t.create_view(&Default::default()))),
            );
        }
        let report = post.as_ref().map(|p| {
            let programs = p.programs().into_iter().map(str::to_owned).collect();
            (programs, p.notes.clone())
        });
        self.pack_post = Some((dimension, post));
        Ok(report)
    }
    /// Makes `block_ids` the running pack's (`runs`: a pack runs with them): when they differ,
    /// the normal stream, which holds each quad's id under the ids it was uploaded with, is
    /// dropped, and the game meshes every section again. Whether they differed.
    fn replace_pack_block_ids(&mut self, block_ids: Vec<i32>, runs: bool) -> bool {
        let old = self.pack_block_ids.as_deref().map_or(&[][..], |ids| &ids.0);
        if block_ids == old {
            return false;
        }
        if runs && self.arena.has_normals() {
            self.sections_dropped |= self.sections.len() > 0;
            self.clear_sections(self.layout);
        }
        self.pack_block_ids = (!block_ids.is_empty()).then(|| Box::new(PackBlockIds(block_ids)));
        true
    }
    /// The pack's flat terrain table (`pack_post::block_ids`) rewritten with the running ids.
    fn write_pack_terrain_ids(&mut self) {
        let ids = self.pack_block_ids.as_deref().map_or(&[][..], |ids| &ids.0);
        let rects = self
            .pack_atlas
            .as_ref()
            .map_or_else(Vec::new, |a| a.rects_table());
        if let Some((_, Some(post))) = &self.pack_post
            && !post.write_block_ids(&self.queue, ids, &rects)
        {
            log::warn!("shader pack: the block ids no longer fit its terrain table");
        }
    }
    /// Replaces the block ids (`set_shader_pack`'s `block_ids`) of the pack that runs, or of the
    /// one `load_shader_pack` builds, which then runs with them: the game resolves them again
    /// when the world's tags its `block.properties` names change. The terrain draws them from
    /// the next frame on (the normal stream once its sections are meshed again). Nothing without
    /// a pack.
    pub fn set_pack_block_ids(&mut self, block_ids: Vec<i32>) {
        if let Some(loading) = &mut self.pack_loading {
            loading.block_ids = Some(block_ids);
            return;
        }
        if !matches!(self.pack_post, Some((_, Some(_)))) {
            return;
        }
        if self.replace_pack_block_ids(block_ids, true) {
            self.write_pack_terrain_ids();
        }
    }
    /// Sets the external shader pipeline world frames call (`external`), or none.
    pub fn set_external_pipeline(&mut self, pipeline: Option<Box<dyn external::ExternalPipeline>>) {
        self.external = pipeline;
    }
    /// Makes the running shader pack forget what it kept from earlier frames:
    /// the next frame draws as a freshly loaded pack's first. Nothing to do without a pack.
    pub fn reset_shader_pack_history(&mut self) {
        // A frame still on the submit thread uses the targets and readbacks replaced
        // here.
        self.land_frame();
        if let Some(pipeline) = &mut self.external {
            pipeline.reset_history();
        }
        if let Some((_, Some(post))) = &mut self.pack_post {
            post.reset_history(&self.device);
        }
    }
    /// Counts a shader pack loaded, switched or turned off (`ShaderPackState::generation`).
    fn pack_changed(&mut self) {
        self.pack_generation = self.pack_generation.wrapping_add(1);
    }
    /// The running pack as a mod reads it this frame, or `None` while the panorama shows or no
    /// pack runs. `frame` gives the camera, so the shadow matrices are this frame's.
    pub fn shader_pack_state(&self, frame: &Frame<'_>) -> Option<mod_api::ShaderPackState> {
        if frame.title {
            return None;
        }
        match &self.pack_post {
            Some((_, Some(post))) => Some(mod_api::ShaderPackState::new(
                post.has_clouds(),
                post.mod_shadow_state(frame).map(
                    |(resolution, distance, model_view, projection)| {
                        mod_api::ShaderPackShadowView::new(
                            resolution, distance, model_view, projection,
                        )
                    },
                ),
                self.pack_generation,
            )),
            _ => None,
        }
    }
    /// The mod pipeline slots the running `shaders/` pack bound to its programs.
    pub fn pack_bound_mod_slots(&self) -> &[usize] {
        match &self.pack_post {
            Some((_, Some(post))) => post.bound_mod_slots(),
            _ => &[],
        }
    }
    /// The running shader pack's kind: `iris` (a `shaders/` pack).
    pub fn shader_pack_kind(&self) -> Option<&'static str> {
        match &self.pack_post {
            Some((_, Some(_))) => Some("iris"),
            _ => None,
        }
    }
    /// `screenshot-sequence`: waits (30 seconds at most) for the frame just drawn to finish on
    /// the GPU, and takes the centre depth a `shaders/` pack copied in it, which its next frame
    /// reads: that frame's, or an error, never an older one. The last frame's PNG has already
    /// waited for it, without a limit, as `screenshot` does.
    pub fn finish_sequence_frame(&mut self) -> Result<()> {
        super::in_flight::poll_live(
            &self.device,
            &self.device_lost,
            wgpu::PollType::Wait {
                submission_index: None,
                timeout: Some(std::time::Duration::from_secs(30)),
            },
        )
        .map_err(|e| format!("the frame didn't finish on the GPU: {e}"))?;
        if let Some((_, Some(post))) = &mut self.pack_post {
            post.take_center_depth()?;
        }
        Ok(())
    }
    /// As `set_shader_pack` with a pack, but its pipelines build on a thread of their own while
    /// the game keeps drawing without one: the driver compiles a big pack's programs for seconds.
    /// `poll_shader_pack` runs it once they are built. `label` and `started` come back with it.
    pub fn load_shader_pack(
        &mut self,
        dimension: i32,
        pack: sunrise_shaders::Post,
        block_ids: Vec<i32>,
        label: String,
        started: std::time::Instant,
    ) {
        let device = self.device.clone();
        let queue = self.queue.clone();
        let adapter = self.adapter.clone();
        let format = self.output.frame.format;
        let atlas = self.pack_atlas.as_ref().map(|a| (a.view.clone(), a.size()));
        let sprite_rects = self
            .pack_atlas
            .as_ref()
            .map_or_else(Vec::new, |a| a.rects_table());
        let material_maps = self
            .pack_material_atlases
            .as_ref()
            .map(|a| [a[0].view.clone(), a[1].view.clone()]);
        let lightmap = self.lightmap_view.clone();
        let gui_size = self
            .hud
            .atlas_texture
            .as_ref()
            .map_or([1, 1], |t| [t.width(), t.height()]);
        let celestial = self
            .hud
            .atlas_texture
            .as_ref()
            .map(|t| t.create_view(&Default::default()));
        let flat = self.pack_terrain_flat;
        let draws = self.pack_terrain_draws();
        let pages = draws.reads_pages();
        let table = (self.flat.as_ref())
            .filter(|_| flat)
            .map(|f| f.draw_layout.clone());
        let terrain = self.pack_atlas.is_some();
        let draws_sections = PackLoading::draws_of(&pack, terrain);
        let served = pack.hdr;
        #[cfg(feature = "test-support")]
        {
            self.test_hooks.pack_loads += 1;
        }
        let clouds = self.cloud_bindings.clone();
        let mod_runtime = crate::mod_pipelines::pack_runtime_specs();
        let mod_set1_owned: Vec<Option<wgpu::BindGroupLayout>> = self
            .world_primitives
            .as_ref()
            .map(|wp| {
                wp.pipelines
                    .set1_bind_group_layouts()
                    .into_iter()
                    .map(|l| l.cloned())
                    .collect()
            })
            .unwrap_or_default();
        let mod_vertex_stages = self
            .world_primitives
            .as_ref()
            .map(|wp| wp.pipelines.pack_vertex_stages())
            .unwrap_or_default();
        // Built with the pack, off the draw path: the mod pipelines' objects for its targets.
        let mod_pack_state = self
            .world_primitives
            .as_ref()
            .map(|wp| wp.pipelines.pack_state_builder());
        let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let cancelled = cancel.clone();
        let build = move || {
            let buffers = pack_terrain_buffers();
            let buffers = &buffers[..terrain_buffer_count(draws)];
            let mod_vertex_buffers = [Some(mod_pipelines::PipelineVertex::buffer_layout())];
            let mod_set1_refs: Vec<Option<&wgpu::BindGroupLayout>> =
                mod_set1_owned.iter().map(Option::as_ref).collect();
            let world = pack_post::World {
                atlas,
                sprite_rects: &sprite_rects,
                material_maps,
                lightmap,
                gui_size,
                terrain: terrain.then(|| pack_post::Geometry {
                    // Flat terrain programs pull their vertices (`pack_atlas::PULL_FLAT`).
                    buffers: if flat { &[] } else { buffers },
                    table: table.as_ref(),
                    depth_write: true,
                    atlas: None,
                    material_maps: true,
                    ids: None,
                    cull: true,
                    pages,
                }),
                block_ids: &block_ids,
                celestial,
                clouds: Some(&clouds),
                weather: true,
                hand: true,
            };
            let post = pack_post::PackPost::new_with_mods(
                (&device, &queue, &adapter),
                pack,
                format,
                world,
                mod_runtime
                    .as_runtime_with_layouts(&mod_vertex_buffers, &mod_set1_refs)
                    .with_vertex_stages(&mod_vertex_stages),
                &cancelled,
            );
            let mod_pack_state = (post.is_ok())
                .then_some(mod_pack_state)
                .flatten()
                .map(|builder| builder.build(&device));
            // The ids and the mod pipelines' state come back with it, for `install_shader_pack`.
            (post, block_ids, mod_pack_state)
        };
        #[cfg(feature = "test-support")]
        let (watch, running) = {
            let watch = self.retired_pack_builds.watch.clone();
            let running = watch.started();
            (watch, running)
        };
        // A pack is not the game's code: a panic building it turns the pack off (#3525).
        let thread = std::thread::Builder::new()
            .name("Shader pack".into())
            .spawn(move || {
                #[cfg(feature = "test-support")]
                let _running = running;
                #[cfg(feature = "test-support")]
                watch.wait_gate();
                crate::crash::contain(build)
            });
        match thread {
            Ok(thread) => {
                // The last pack doesn't draw this dimension meanwhile.
                self.pack_changed();
                self.pack_post = None;
                if let Some(world_primitives) = self.world_primitives.as_mut() {
                    world_primitives.pipelines.deselect_pack();
                }
                self.pack_loading = Some(PackLoading {
                    dimension,
                    cancel,
                    thread: Some(thread),
                    retired: self.retired_pack_builds.clone(),
                    origin: self.pack_build_origin(),
                    label,
                    started,
                    draws: draws_sections,
                    served,
                    block_ids: None,
                });
            }
            Err(e) => {
                self.pack_thread_failed(dimension, &label, &e);
                self.pack_served = served;
            }
        }
    }
    /// Whether the pack `load_shader_pack` builds is built, waiting for `poll_shader_pack`.
    pub fn shader_pack_built(&self) -> bool {
        self.pack_loading
            .as_ref()
            .and_then(|l| l.thread.as_ref())
            .is_some_and(|t| t.is_finished())
    }
    /// `poll_shader_pack`, unless the pack in the renderer is `stale`: loaded, or still
    /// building, from inputs the game no longer wants (another Iris compatibility). Then it is
    /// dropped first (`drop_shader_packs`), so a build already done is never installed, and
    /// nothing comes back.
    pub fn poll_shader_pack_unless(&mut self, stale: bool) -> Option<PackPoll> {
        if stale {
            self.drop_shader_packs();
            return None;
        }
        self.poll_shader_pack()
    }
    /// `load_shader_pack` found no thread to build its pack on: the pack is refused for
    /// `dimension`, as one that fails to build is (`install_shader_pack`), so it is not tried
    /// again every frame and its atlases go.
    pub(crate) fn pack_thread_failed(
        &mut self,
        dimension: i32,
        label: &str,
        error: &std::io::Error,
    ) {
        log::error!("shader pack {label}: no thread to build it on: {error}");
        self.pack_loading = None;
        let refused = pack_post::Refused {
            reason: format!("no thread to build it on: {error}"),
            notes: Vec::new(),
        };
        let _ = self.install_shader_pack(dimension, Err(refused), Vec::new(), None);
    }
    /// The pack `load_shader_pack` builds, once its thread is done: it runs from now on, as
    /// `set_shader_pack` would run it. Its dimension, label, start and what `set_shader_pack`
    /// returns.
    pub fn poll_shader_pack(&mut self) -> Option<PackPoll> {
        // Builds cancelled earlier that have ended since: their packs go now, also while no
        // pack is wanted.
        self.retired_pack_builds.reap();
        if !self.pack_loading.as_ref()?.thread.as_ref()?.is_finished() {
            return None;
        }
        let mut loading = self.pack_loading.take()?;
        let thread = loading.thread.take()?;
        let (built, block_ids, mod_pack_state) = match thread.join() {
            Ok(Ok(built)) => built,
            // A pack that doesn't run keeps no ids.
            Ok(Err(panic)) => (
                Err(pack_post::Refused {
                    reason: format!("building it panicked: {panic}"),
                    notes: Vec::new(),
                }),
                Vec::new(),
                None,
            ),
            Err(_) => (
                Err(pack_post::Refused {
                    reason: "its thread panicked".into(),
                    notes: Vec::new(),
                }),
                Vec::new(),
                None,
            ),
        };
        self.pack_served = loading.served;
        // Ids given while it built replace those it was built with, in its terrain table too
        // where they differ.
        let given = loading
            .block_ids
            .take()
            .filter(|ids| built.is_ok() && *ids != block_ids);
        let rewrite = given.is_some();
        let block_ids = given.unwrap_or(block_ids);
        let report = self.install_shader_pack(
            loading.dimension,
            built.map(Some),
            block_ids,
            mod_pack_state,
        );
        if rewrite {
            self.write_pack_terrain_ids();
        }
        let label = std::mem::take(&mut loading.label);
        Some((loading.dimension, label, loading.started, report))
    }
    /// Drops the pack still building for another served answer of Sunrise's HDR than `served`
    /// (`Post::hdr`): whatever it ends in, success or refusal, is for a load nobody wants any
    /// more, so `poll_shader_pack` never reports it against the pack wanted now. A build for
    /// another dimension with the same answer is kept, to be polled as before. Whether one was
    /// dropped; the loader then loads the wanted one.
    pub fn drop_superseded_pack_load(&mut self, served: Option<sunrise_shaders::Tonemap>) -> bool {
        if self
            .pack_loading
            .as_ref()
            .is_none_or(|l| l.served == served)
        {
            return false;
        }
        self.pack_loading = None;
        self.pack_changed();
        true
    }
    /// What this renderer's pack builds run on (`BuildOrigin`).
    fn pack_build_origin(&self) -> BuildOrigin {
        BuildOrigin {
            device_lost: self.device_lost.clone(),
            #[cfg(feature = "test-support")]
            watch: self.retired_pack_builds.watch.clone(),
        }
    }
    /// Joins the cancelled pack builds that have ended, as every `poll_shader_pack` does: for the
    /// frames that don't poll (the title screen's).
    pub fn reap_pack_builds(&self) {
        self.retired_pack_builds.reap();
    }
    /// The pack builds still running, for the renderer that replaces this one
    /// (`adopt_pack_builds`): the one loading is cancelled first. This renderer then drops
    /// without waiting for any (a renderer swap must not wait on the driver).
    pub fn take_pack_builds(&mut self) -> PackBuilds {
        self.pack_loading = None;
        PackBuilds(self.retired_pack_builds.take())
    }
    /// Keeps the pack builds another renderer handed over (`take_pack_builds`): joined as they
    /// end, and before this renderer drops unless their own device is lost.
    pub fn adopt_pack_builds(&mut self, builds: PackBuilds) {
        for build in builds.0 {
            self.retired_pack_builds.retire(build);
        }
    }
    /// Drops the running shader pack.
    pub fn drop_shader_packs(&mut self) {
        self.pack_loading = None;
        self.pack_changed();
        self.pack_post = None;
        self.pack_served = None;
        if let Some(world_primitives) = self.world_primitives.as_mut() {
            world_primitives.pipelines.deselect_pack();
        }
        self.forget_pack_atlas();
        self.separate_ao = false;
        self.pack_block_ids = None;
    }
    /// Frees the pack's block atlas and the material atlases made in its layout
    /// (`pack_terrain_inputs` makes them again, together).
    fn forget_pack_atlas(&mut self) {
        self.pack_atlas = None;
        self.pack_material_atlases = None;
    }
    /// Sets whether shader pack terrain inputs pass vertex alpha to `hz_Color.a` (`separateAo=true`).
    pub fn set_separate_ao(&mut self, separate_ao: bool) {
        if self.separate_ao != separate_ao {
            self.separate_ao = separate_ao;
            if let Some(atlas) = &mut self.pack_atlas {
                atlas.separate_ao = separate_ao;
            }
        }
    }
    /// Lava's sprite layers (still and flowing), whose faces a pack's solid terrain draws.
    pub fn set_pack_lava_layers(&mut self, layers: Vec<u16>) {
        self.pack_lava_layers = layers;
    }
    /// What a shader pack's gbuffers programs read from Sunrise's geometry: the sky's, and
    /// terrain's in the pack's block atlas (made here when missing; none when it does not fit).
    pub fn pack_inputs(&mut self) -> sunrise_shaders::GameInputs {
        let terrain = self.pack_terrain_inputs();
        sunrise_shaders::GameInputs {
            interface_limits: crate::core_shader::interface_limits(&self.device.limits()),
            mod_programs: crate::mod_pipelines::pack_program_requests(),
            ..pack_atlas::game_inputs(terrain, self.pack_terrain_flat, &self.pack_lava_layers)
        }
    }

    pub(super) fn pack_terrain_inputs(&mut self) -> Option<sunrise_shaders::GbuffersInputs> {
        // Flat draws whenever the device has them (`render` keeps them on with a pack).
        let flat = self.flat.is_some() && self.flat_on;
        // Flat draws read each sprite's place from a table: the atlas is stitched as the game's.
        // Other draws need the grid.
        if !flat && self.pack_atlas.as_ref().is_some_and(|a| a.is_stitched()) {
            self.forget_pack_atlas();
        }
        if self.pack_atlas.is_none() {
            let mut encoder = self.device.create_command_encoder(&Default::default());
            let layers = self
                .sprite_textures
                .iter()
                .map(|t| t.depth_or_array_layers())
                .sum();
            let sprite = self.sprite_textures[0].width();
            // Vanilla's padding counts the anisotropy bit only with anisotropic filtering.
            let anisotropy_bit = if self.texture_filtering == 2 {
                u32::from(self.max_anisotropy).trailing_zeros()
            } else {
                0
            };
            let stitched = flat
                .then(|| {
                    pack_atlas::PackAtlas::stitched(
                        &self.device,
                        &mut encoder,
                        &self.sprite_textures,
                        self.sprite_split,
                        sprite,
                        &self.sprite_native,
                        self.mipmap_option,
                        anisotropy_bit,
                    )
                })
                .flatten();
            if flat && stitched.is_none() {
                log::warn!(
                    "shader pack block atlas: the sprites don't stitch on this device, a grid instead"
                );
            }
            let mut atlas = stitched.or_else(|| {
                pack_atlas::PackAtlas::new(
                    &self.device,
                    &mut encoder,
                    &self.sprite_textures,
                    self.sprite_split,
                    layers,
                    sprite,
                )
            });
            if let Some(atlas) = &mut atlas {
                atlas.separate_ao = self.separate_ao;
            }
            self.pack_material_atlases = atlas.as_ref().and_then(|colour| {
                let maps = self.pbr_maps.as_ref()?;
                let make = |kind: usize, encoder: &mut wgpu::CommandEncoder| {
                    let arrays: Vec<_> = maps.iter().map(|pair| pair[kind].clone()).collect();
                    colour.material_for(&self.device, encoder, &arrays, layers)
                };
                Some([make(0, &mut encoder)?, make(1, &mut encoder)?])
            });
            self.pack_atlas = atlas;
            self.queue.submit([encoder.finish()]);
        }
        self.pack_terrain_flat = flat;
        // Per-section draws read the pages where the pack's terrain programs still fit the
        // vertex stage with them, and the normal stream where they don't but fit with it.
        let gpu = pack_post::Gpu::of(&self.adapter, self.device.features());
        let limits = self.device.limits();
        let fit = (self.pack_terrain_bindings).pages_fit(&limits, gpu);
        self.pack_terrain_pages = !flat && self.terrain_pages && fit;
        let stream = (self.pack_terrain_bindings).normals_fit(&limits, gpu);
        self.pack_terrain_normals = !flat && !self.pack_terrain_pages && stream;
        if !flat && !self.pack_terrain_pages && !stream {
            log::warn!(
                "shader pack terrain: {:?} fit the vertex stage's buffers with neither set 3 \
                 nor the normal stream; per-section draws give the constant normal and tangent, \
                 mc_Entity -1 and at_midBlock 0",
                self.pack_terrain_bindings
            );
        }
        let draws = self.pack_terrain_draws();
        Some(self.pack_atlas.as_ref()?.terrain_inputs_for(draws))
    }
    /// How the game's terrain draws give the pack's terrain programs their vertices, as
    /// `pack_terrain_inputs` last took them: flat, or per section, reading their quad's corners
    /// from the section arena's pages where vertex shaders read storage and the pack's programs
    /// fit with them (`pack_terrain_pages`), else their quad's normal and tangent from the
    /// normal stream where they fit with it (`pack_terrain_normals`), else as attributes alone.
    pub(crate) fn pack_terrain_draws(&self) -> pack_atlas::TerrainDraws {
        match (
            self.pack_terrain_flat,
            self.pack_terrain_pages,
            self.pack_terrain_normals,
        ) {
            (true, ..) => pack_atlas::TerrainDraws::Flat,
            (false, true, _) => pack_atlas::TerrainDraws::Pages,
            (false, false, true) => pack_atlas::TerrainDraws::Normals,
            (false, false, false) => pack_atlas::TerrainDraws::Attributes,
        }
    }
    /// What the pack about to be loaded binds in its terrain programs' vertex stage
    /// (`pack_atlas::TerrainBindings::of`), before `pack_inputs`: per-section draws read the
    /// arena's pages, or else the normal stream, only where those programs still fit with them.
    pub fn set_pack_terrain_bindings(&mut self, bindings: pack_atlas::TerrainBindings) {
        self.pack_terrain_bindings = bindings;
    }
    /// The macros this renderer gives shader packs: the GL extensions this device has
    /// (subgroups, when it has them), and the vendor, renderer and mipmap levels
    /// (`Macros::define_renderer`); with them, the feature flags this device decides
    /// (`define_device_features`).
    pub fn define_pack_macros(&self, macros: &mut sunrise_shaders::Macros) {
        macros.define_renderer(self.mipmaps);
        define_device_features(macros, self.adapter.get_downlevel_capabilities().flags);
        if self.device.features().contains(wgpu::Features::SUBGROUP) {
            macros.define_subgroups();
        }
        for (name, value) in crate::mod_pipelines::pack_defines() {
            macros.define(name, value);
        }
    }

    /// How the active translated shader pack binds mod pipeline `slot`.
    pub fn pack_override_state(&self, slot: usize) -> mod_api::PackOverrideState {
        match &self.pack_post {
            Some((_, Some(post))) => {
                if post.is_mod_pipeline_bound(slot) {
                    mod_api::PackOverrideState::Bound
                } else {
                    mod_api::PackOverrideState::UnboundPack
                }
            }
            _ => mod_api::PackOverrideState::NoPack,
        }
    }

    /// The dimension the shader pack building or running was set for and Sunrise's HDR as it
    /// was served then (`Post::hdr`): a pack set for another dimension or another answer is
    /// loaded again.
    pub fn shader_pack_loaded_for(&self) -> Option<(i32, Option<sunrise_shaders::Tonemap>)> {
        match &self.pack_loading {
            Some(loading) => Some((loading.dimension, loading.served)),
            None => self.pack_post.as_ref().map(|p| (p.0, self.pack_served)),
        }
    }
}

/// A built pack as `Renderer::poll_shader_pack` gives it: its dimension, label, start, and what
/// `set_shader_pack` returns.
pub type PackPoll = (
    i32,
    String,
    std::time::Instant,
    std::result::Result<Option<(Vec<String>, Vec<String>)>, pack_post::Refused>,
);

/// The section origins' vertex attribute (`section_instance`).
const INSTANCE_ATTRIBUTES: [wgpu::VertexAttribute; 1] = wgpu::vertex_attr_array![5=>Float32x4];

/// The vertex buffers a pack's per-section terrain programs read: the arena's vertices, the
/// sections' origins and the normal stream (`pack_atlas::NORMAL_ATTRIBUTES`), of which a
/// pipeline takes `terrain_buffer_count`.
fn pack_terrain_buffers() -> [Option<wgpu::VertexBufferLayout<'static>>; 3] {
    [
        Some(wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<TerrainVertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &TERRAIN_ATTRIBUTES,
        }),
        Some(wgpu::VertexBufferLayout {
            array_stride: 16,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &INSTANCE_ATTRIBUTES,
        }),
        Some(wgpu::VertexBufferLayout {
            array_stride: pack_atlas::NORMAL_BYTES,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &pack_atlas::NORMAL_ATTRIBUTES,
        }),
    ]
}

/// How many of `pack_terrain_buffers` a terrain program drawn as `draws` reads: the normal
/// stream only with `TerrainDraws::Normals`.
fn terrain_buffer_count(draws: pack_atlas::TerrainDraws) -> usize {
    if draws == pack_atlas::TerrainDraws::Normals {
        3
    } else {
        2
    }
}

/// The answers a device with `flags` gives to a pack's feature flags (spec 3.2) that depend on
/// it: `PER_BUFFER_BLENDING` where wgpu draws colour targets that differ in blend
/// (`DownlevelFlags::INDEPENDENT_BLEND`).
pub(crate) fn define_device_features(
    macros: &mut sunrise_shaders::Macros,
    flags: wgpu::DownlevelFlags,
) {
    macros.set_per_buffer_blending(flags.contains(wgpu::DownlevelFlags::INDEPENDENT_BLEND));
}
