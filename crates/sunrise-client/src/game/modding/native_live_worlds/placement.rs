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
        self.world.revision == plan.source_revision
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
        let table = self.live_worlds.get_mut();
        if let Some(entry) = table.entries.get_mut(&plan.handle.generation()) {
            entry.cells = plan.cells;
            entry.stream_areas = plan.areas;
            entry.membership = entry
                .membership
                .checked_add(1)
                .expect("projection membership exhausted");
            *entry.geometry.get_mut() = None;
            *entry.mesh.get_mut() = None;
        }
    }
    pub(in crate::game) fn projection_block_removed(&mut self, pos: [i32; 3]) {
        let table = self.live_worlds.get_mut();
        for entry in table
            .entries
            .values_mut()
            .filter(|e| e.handle.level().dimension() == self.dim.key())
        {
            if let Ok(index) = entry.cells.binary_search(&pos) {
                entry.cells.remove(index);
                entry.membership = entry
                    .membership
                    .checked_add(1)
                    .expect("projection membership exhausted");
                entry.stream_areas = sync::stream_areas(entry.handle.level(), &entry.cells)
                    .expect("removing a member cannot grow streaming budgets");
                *entry.geometry.get_mut() = None;
                *entry.mesh.get_mut() = None;
            }
        }
    }
}
