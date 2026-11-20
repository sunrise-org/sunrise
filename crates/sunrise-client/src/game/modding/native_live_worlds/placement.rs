//! Prepared membership derived from the real ordinary placement preview.
use super::*;
use sunrise_client::lan::viewed_levels::areas::ChunkArea;
use sunrise_world::level::PlacementPreview;

pub(in crate::game) struct PreparedMembership {
    handle: LiveWorldRef,
    source_revision: u64,
    pose_revision: u64,
    membership: u64,
    cells: Vec<[i32; 3]>,
    retained: Vec<(i32, i32)>,
    areas: Vec<ChunkArea>,
    changes: Vec<mod_api::LiveWorldPlacementCell>,
}
impl Game<'_> {
    pub(in crate::game) fn prepare_projection_placement(
        &self,
        preview: &PlacementPreview,
    ) -> Result<Option<PreparedMembership>, Error> {
        let table = self.live_worlds.borrow();
        let Some(entry) = table
            .entries
            .values()
            .find(|e| e.handle.level().dimension() == self.dim.key())
        else {
            return Ok(None);
        };
        let mut writes = BTreeMap::new();
        if let Some((p, s)) = preview.pre_clear {
            writes.insert([p.x, p.y, p.z], s);
        }
        if let Some((p, s)) = preview.companion {
            writes.insert([p.x, p.y, p.z], s);
        }
        writes.insert([preview.pos.x, preview.pos.y, preview.pos.z], preview.state);
        let mut members: BTreeSet<_> = entry.cells.iter().copied().collect();
        let mut changes = Vec::with_capacity(writes.len());
        for (&pos, &state) in &writes {
            if !self.live_world_tick_ready(entry.handle.level(), pos) {
                return Err(Error::NotResident);
            }
            let shape = state_shape(self, state)?;
            if self.res.behaviours().is_air(state) {
                members.remove(&pos);
            } else {
                members.insert(pos);
            }
            changes.push(mod_api::LiveWorldPlacementCell::new(
                pos,
                state.try_into_api().map_err(|_| Error::Unsupported)?,
                shape
                    .boxes()
                    .iter()
                    .map(|b| ColliderBox {
                        min: b.min_position().components(),
                        max: b.max_position().components(),
                    })
                    .collect(),
            ));
        }
        let cells = checked_cells(&members.into_iter().collect::<Vec<_>>(), entry.limits)?;
        let retained = retention::checked_retention(&cells, entry.limits)?;
        let areas = sync::stream_areas(entry.handle.level(), &cells)?;
        self.check_projection_views_replacing(entry.destination, &areas, Some(entry.handle))?;
        let mut local = Vec::with_capacity(cells.len());
        let mut boxes = Vec::new();
        for &pos in &cells {
            let shape = if let Some(&state) = writes.get(&pos) {
                state_shape(self, state)?
            } else {
                shape(self, &self.world, pos)?
            };
            boxes.extend(
                shape
                    .boxes()
                    .iter()
                    .map(|b| b.move_by(vector(pos.map(f64::from)))),
            );
            if boxes.len() > entry.limits.boxes as usize {
                return Err(Error::Budget);
            }
            local.push(CarrierCell::new(position(pos), shape));
        }
        let geometry = Geometry {
            source_revision: self.world.revision,
            membership: entry.membership,
            resources: self.standalone_geometry.generation(),
            state_hash: [0; 32],
            contact: CarrierGeometry::new(u128::from(entry.collider), local)
                .map_err(|_| Error::Invalid)?,
            boxes: Arc::new(ColliderBoxes::new(boxes)),
        };
        let pose = checked_pose(entry.sample.current)?;
        let obstacles = self.projection_obstacles(entry, &geometry, &pose, Vec3::ZERO, 0.0)?;
        let checked = geometry
            .contact
            .translate_step(
                &pose,
                Vec3::ZERO,
                &obstacles,
                &ContactBudget {
                    boxes: entry.limits.boxes as usize,
                    iterations: entry.limits.contacts as usize,
                },
            )
            .map_err(|_| Error::Budget)?;
        if checked.fraction < 1.0 {
            return Err(Error::Obstructed);
        }
        Ok(Some(PreparedMembership {
            handle: entry.handle,
            source_revision: self.world.revision,
            pose_revision: entry.sample.revision,
            membership: entry.membership,
            cells,
            retained,
            areas,
            changes,
        }))
    }

    /// The exact decision is lent to hooks inside the caller's storage transaction.
    pub(in crate::game) fn admit_projection_placement(
        &mut self,
        plan: &PreparedMembership,
    ) -> bool {
        let mut canceled = false;
        if mod_api::__private::listened(mod_api::HookId::LiveWorldAdmission) {
            let player = self.native_acting_player();
            self.native_live_world_admission(plan.handle, player, &plan.changes, &mut canceled);
        }
        !canceled && self.projection_placement_fresh(plan)
    }
    pub(in crate::game) fn projection_placement_fresh(&self, plan: &PreparedMembership) -> bool {
        self.projection_action_fresh()
            && self.world.revision == plan.source_revision
            && self
                .live_worlds
                .borrow()
                .entries
                .get(&plan.handle.generation())
                .is_some_and(|e| {
                    e.handle == plan.handle
                        && e.sample.revision == plan.pose_revision
                        && e.membership == plan.membership
                })
    }
    pub(in crate::game) fn finish_projection_placement(&mut self, plan: PreparedMembership) {
        let source = plan.handle.level();
        self.journal_projection_binding(source);
        self.dirty_live_world_level(source)
            .expect("an admitted projection retains its source save lease");
        let table = self.live_worlds.get_mut();
        if let Some(entry) = table.entries.get_mut(&plan.handle.generation()) {
            entry.cells = plan.cells;
            entry.retained = plan.retained;
            entry.stream_areas = plan.areas;
            entry.membership = entry
                .membership
                .checked_add(1)
                .expect("projection membership exhausted");
            // Membership is part of the cache key. Retain the last coherent caches until
            // the complete replacement succeeds, including temporary source residency loss.
        }
        self.refresh_live_world_retention(source);
    }
    pub(in crate::game) fn projection_block_removed(&mut self, pos: [i32; 3]) {
        let affected: Vec<_> = self
            .live_worlds
            .borrow()
            .entries
            .values()
            .filter(|entry| {
                entry.handle.level().dimension() == self.dim.key()
                    && entry.cells.binary_search(&pos).is_ok()
            })
            .map(|entry| (entry.handle, entry.cells.len()))
            .collect();
        for &(handle, count) in &affected {
            self.journal_projection_binding(handle.level());
            self.dirty_live_world_level(handle.level())
                .expect("an admitted projection retains its source save lease");
            if count == 1 {
                self.record_projection_retirement(handle, 0)
                    .expect("the real final member still has its source owner");
            }
        }
        let last: Vec<_> = self
            .live_worlds
            .borrow()
            .entries
            .values()
            .filter(|entry| {
                entry.handle.level().dimension() == self.dim.key()
                    && entry.cells.as_slice() == [pos]
            })
            .map(|entry| entry.handle)
            .collect();
        for handle in last {
            // Ordinary authoritative removal already holds the source and its player context;
            // its missing BE is an ordinary menu invalidation, not a reason to lose the cursor.
            self.close_projection_menus(handle)
                .expect("final member removal retains its ordinary source/menu context");
        }
        let mut sources = Vec::new();
        let mut empty = Vec::new();
        let table = self.live_worlds.get_mut();
        for entry in table
            .entries
            .values_mut()
            .filter(|e| e.handle.level().dimension() == self.dim.key())
        {
            if let Ok(index) = entry.cells.binary_search(&pos) {
                entry.cells.remove(index);
                sources.push(entry.handle.level());
                if entry.cells.is_empty() {
                    empty.push(entry.handle.generation());
                    continue;
                }
                entry.retained = retention::checked_retention(&entry.cells, entry.limits)
                    .expect("removing members cannot expand source retention");
                entry.membership = entry
                    .membership
                    .checked_add(1)
                    .expect("projection membership exhausted");
                // Keep the admitted cover. Re-tiling a smaller set can fragment one area
                // into many and exceed a guest's aggregate area budget after mutation.
            }
        }
        for generation in empty {
            table.entries.remove(&generation);
        }
        for source in sources {
            self.refresh_live_world_retention(source);
        }
    }
}
