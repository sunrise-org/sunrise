//! A bounded owned snapshot of ordinary physical building blockers for one source action.
//! No physical body is moved; source entities are checked by the ordinary source query.
use super::*;
use sunrise_core::geometry::Aabb;

pub(in crate::game) struct Building {
    frame: Frame,
    boxes: Arc<[Aabb]>,
    contacts: usize,
}

impl Building {
    pub(in crate::game) fn unobstructed(&self, local_shape: &VoxelShape) -> bool {
        let local = local_shape.boxes();
        if local
            .len()
            .checked_mul(self.boxes.len())
            .is_none_or(|work| work > self.contacts)
        {
            return false;
        }
        if local.is_empty() || self.boxes.is_empty() {
            return true;
        }
        let collider =
            OrientedCollider::new(self.frame, Arc::new(ColliderBoxes::new(local.to_vec())));
        self.boxes
            .iter()
            .all(|&physical| !collider.intersects(physical))
    }
}

impl Game<'_> {
    /// Called after borrowing the authenticated actor, before entering the source World.
    pub(super) fn capture_projection_building(
        &mut self,
        handle: LiveWorldRef,
        pose: LiveWorldPose,
    ) -> Result<Rc<Building>, Error> {
        let contacts = self.resolved_projection_entry(handle)?.limits.contacts as usize;
        let boxes = self
            .with_level_access(false, |_, level, _| {
                level.building_blocker_boxes_limited(contacts)
            })
            .ok_or(Error::Budget)?;
        Ok(Rc::new(Building {
            frame: frame(pose),
            boxes: boxes.into(),
            contacts,
        }))
    }

    pub(in crate::game) fn projection_building_context(&self) -> Option<Rc<Building>> {
        let table = self.live_worlds.borrow();
        let action = table.action?;
        (self.dim.key() == action.handle.level().dimension()
            && self.native_acting_player() == Some(action.player))
        .then(|| table.building.clone())
        .flatten()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn translated_and_rotated_candidate_uses_actual_physical_boxes_and_contact_budget() {
        let shape = VoxelShape::from_box(Aabb::new(
            Vec3::new(1.0, 64.0, 0.0),
            Vec3::new(2.0, 65.0, 1.0),
        ))
        .unwrap();
        let angle = std::f64::consts::FRAC_PI_4;
        let mut pose = LiveWorldPose::at([10.0, 0.0, 10.0]);
        pose.rotation = [0.0, angle.sin(), 0.0, angle.cos()];
        // +90 degrees around Y maps the independent local centre (1.5,64.5,0.5)
        // to (10.5,64.5,8.5), so an actual body there must block placement.
        let actual = Aabb::new(Vec3::new(10.25, 64.0, 8.25), Vec3::new(10.75, 65.8, 8.75));
        let hit = Building {
            frame: frame(pose),
            boxes: vec![actual].into(),
            contacts: 1,
        };
        assert!(!hit.unobstructed(&shape));
        let away = Building {
            frame: frame(pose),
            boxes: vec![actual.move_by(Vec3::new(3.0, 0.0, 0.0))].into(),
            contacts: 1,
        };
        assert!(away.unobstructed(&shape));
        let over_budget = Building {
            contacts: 0,
            ..away
        };
        assert!(!over_budget.unobstructed(&shape));
    }
}
