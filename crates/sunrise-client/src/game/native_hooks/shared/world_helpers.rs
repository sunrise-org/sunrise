use super::*;

#[allow(dead_code)]
impl Game<'_> {
    /// Sets a block in the current authoritative level through `Level.setBlock`.
    /// Returns false for another dimension, an invalid state, or no change.
    /// `flags` selects vanilla's neighbour, shape and client updates. A block
    /// entity block gets its block entity.
    pub(in crate::game) fn native_set_block(
        &mut self,
        dimension: &str,
        pos: [i32; 3],
        state: BlockStateId,
        flags: i32,
    ) -> bool {
        if (self.serving.is_none() && self.options.dedicated.is_none())
            || dimension != self.dim.id()
            || self.pack.assets.registry.state(state).is_none()
        {
            return false;
        }
        let changed =
            self.with_level(|_, level, ticks| level.native_set_block(ticks, pos, state, flags));
        let _ = self.process_level_events();
        changed
    }
    /// The item entities of the server level `dimension` whose box meets
    /// the box `min`..`max`, in the level's order: vanilla's
    /// `Level.getEntitiesOfClass(ItemEntity.class, box, ENTITY_STILL_ALIVE)`, as a
    /// hopper's `getItemsAtAndAbove` asks for its block and the one above
    /// (`[x, y, z]`..`[x + 1, y + 2, z + 1]`). Empty for a level not in place
    /// (as `native_set_block`) and on a client of a server.
    pub(in crate::game) fn native_item_entities(
        &self,
        dimension: &str,
        min: [f64; 3],
        max: [f64; 3],
    ) -> Vec<NativeItemEntity> {
        if (self.serving.is_none() && self.options.dedicated.is_none())
            || dimension != self.dim.id()
        {
            return Vec::new();
        }
        self.dropped
            .iter()
            .filter(|e| e.stack.count > 0 && box_meets(&e.bounds(), min, max))
            .map(|e| NativeItemEntity {
                id: e.id,
                position: [e.position.x, e.position.y, e.position.z],
                stack: e.stack.clone(),
                // The helper exposes an unsigned delay; keep its projection.
                pickup_delay: e.pickup_delay.clamp(0, i64::from(u32::MAX)) as u32,
            })
            .collect()
    }
    /// Runs `change` on the mod block entity at `pos` (a generator charging
    /// the machine beside it) and returns what it returns; none for any
    /// other block entity, and for the one ticking now, which is out of the
    /// world. Afterwards, as its `setChanged` would, its chunk is saved next
    /// time and, when its items changed, comparators read it again.
    pub(in crate::game) fn native_with_mod_block_entity<R>(
        &mut self,
        pos: [i32; 3],
        change: impl FnOnce(&mut EngineBlockEntity) -> R,
    ) -> Option<R> {
        let key = (pos[0], pos[1], pos[2]);
        let Some(BlockEntity::Mod(m)) = self.world.block_entities.get_mut(&key) else {
            return None;
        };
        if m.id().is_empty() {
            return None;
        }
        let items = m.items.clone();
        let out = change(m);
        let moved = m.items != items;
        self.world.modified.insert((key.0 >> 4, key.2 >> 4));
        if moved {
            self.container_changed(key);
        }
        Some(out)
    }
    /// This `Game` is the window's, not a server's.
    pub(in crate::game) fn native_window(&self) -> bool {
        self.options.execution_role == ExecutionRole::Client
    }
    /// `ParticleEngine.countParticles`: every live group. Sunrise keeps vanilla's groups
    /// apart: ordinary particles, firework starters and elder guardians in the engine,
    /// and ITEM_PICKUP in `pickups`.
    pub(in crate::game) fn client_particle_count(&self) -> usize {
        self.particles.group_count() + self.pickups.live.len()
    }
    /// `ParticleEngine.clearParticles` over every group Sunrise keeps apart: the engine's
    /// particles, starters, queue and emitters, the item pickups and the elder guardian
    /// apparition. Leaving a level and switching levels clear through it too.
    pub(in crate::game) fn clear_client_particles(&mut self) {
        self.mod_clear_particles.set(false);
        self.particles.clear();
        self.pickups.clear();
    }
    /// Applies a client mod's `clear_particles` if one is waiting. Called at the start of
    /// each particle tick, after `RenderFrameEvent.Pre`, and at particle extraction, after
    /// HUD, screen and entity callbacks. Extraction also discards its geometry if a model
    /// callback asks to clear while it is being built, even while paused or tick-frozen.
    pub(in crate::game) fn apply_mod_particle_clear(&mut self) {
        if self.mod_clear_particles.take() {
            self.clear_client_particles();
        }
    }
    /// Draws one vanilla HUD layer between the mods' hooks: first the mods' layers
    /// placed below it (`draw_mod_gui_layers`), then `native_gui_layer_pre` (which
    /// may hide it), the layer, then `native_gui_layer_post`. `None` when a mod hid
    /// the layer.
    pub(in crate::game) fn gui_layer<R>(
        &mut self,
        layer: &str,
        batch: &mut HudBatch,
        draw: impl FnOnce(&mut Self, &mut HudBatch) -> R,
    ) -> Option<R> {
        self.draw_mod_gui_layers(Some(layer), batch);
        let mut canceled = false;
        self.native_gui_layer_pre(layer, batch, &mut canceled);
        let drawn = (!canceled).then(|| draw(self, batch));
        self.native_gui_layer_post(layer, batch);
        drawn
    }
    /// `GuiLayerManager.renderInner` for the mods' layers (`native_register_gui_layers`) not
    /// drawn yet this frame and placed below vanilla layer `until`, or for all that are left
    /// with `None`, at the HUD's end. Each fires `native_gui_layer_pre` (which may hide it),
    /// `native_gui_layer_draw`, then `native_gui_layer_post`, as a vanilla layer does. So a
    /// vanilla layer that is not drawn this frame keeps its place: the mods' layers around it
    /// draw at the next layer that is. Nothing while the HUD is hidden, and nothing to look up
    /// when no mod registered a layer.
    pub(in crate::game) fn draw_mod_gui_layers(
        &mut self,
        until: Option<&str>,
        batch: &mut HudBatch,
    ) {
        let order = sunrise_client::gui_layers::order();
        let Some(next) = self.hud_layer_next.filter(|_| order.has_mod_layers()) else {
            return;
        };
        let end = match until {
            Some(layer) => match order.position(layer) {
                Some(at) => at,
                None => return,
            },
            None => order.len(),
        };
        for at in next..end {
            let Some(layer) = order.mod_layer(at) else {
                continue;
            };
            let mut canceled = false;
            self.native_gui_layer_pre(layer, batch, &mut canceled);
            if !canceled {
                self.native_gui_layer_draw(layer, batch);
            }
            self.native_gui_layer_post(layer, batch);
        }
        // Past `until` itself: it is drawn next, by `gui_layer`.
        self.hud_layer_next = Some(next.max(end + usize::from(until.is_some())));
    }
    /// Whether a box in `corner` shows: F3's debug screen takes both top
    /// corners (its left and right columns), and a bottom corner while one
    /// of its charts is drawn there.
    pub(in crate::game) fn native_corner_shown(&self, corner: Corner) -> bool {
        let charts = self.debug_charts.corners(!self.has_singleplayer_server());
        !debug_takes(corner, self.debug, charts)
    }
    /// A mod's own screen closes (Escape, or `mod_screen_close`).
    pub(in crate::game) fn close_mod_screen(&mut self) {
        if self.screen == Screen::Mod {
            if self.title.is_some() {
                self.close_mod_screen_to_title();
            } else {
                self.set_screen(Screen::Playing);
                self.capture(true);
            }
            if self.screen == Screen::Mod {
                // `screen_opening` kept it open, or opened a mod's own.
                return;
            }
        }
        self.native_mod_screen = None;
    }
    /// `Screen.onClose`'s `setScreen(null)` with no world: the title screen
    /// opens, asked for once as `mod` to `title`, and a mod's answer applies
    /// as on any title screen. Told to the frame's observer, so it doesn't
    /// announce it again.
    fn close_mod_screen_to_title(&mut self) {
        let decision = if self.screen_hooks_heard() {
            let current = self.screen_change_view(Screen::Mod);
            let none = self.screen_change_view(Screen::Playing);
            self.decide_screen(&current, &screen_view_with_id(&none, "title"), true)
        } else {
            ScreenDecision::Open
        };
        match decision {
            ScreenDecision::Keep => return,
            ScreenDecision::ModScreen(mod_id) => self.apply_screen(Screen::Mod, Some(mod_id)),
            ScreenDecision::Vanilla(target) => self.open_screen_target(target),
            ScreenDecision::Open => {
                if let Some(title) = self.title.as_mut() {
                    title.open_by_id("title");
                }
                self.apply_screen(Screen::Playing, None);
            }
        }
        self.screen_state.observed = self.observed_now();
        self.shown_init_post();
    }
}
