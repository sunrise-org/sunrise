//! Retained copies feed the same per-view submission route as transient batches.
use super::Game;
use mod_api::__private::game::Fields;
use mod_api::{GroupError, InstanceGroupRef, InstanceGroups, ModelInstance, ModelRef};

use glam::{DVec3, Mat4};
use sunrise_client::{gfx::PreparedModelInstance, world_geometry::row_major_3x4};

struct Group {
    generation: u64,
    owner: Option<&'static str>,
    submitted: bool,
    model: ModelRef,
    copies: Vec<ModelInstance>,
    len: usize,
    transform: Mat4,
    composed: Vec<ModelInstance>,
    records: Vec<PreparedModelInstance>,
    #[cfg(test)]
    converted_bytes: usize,
}

impl Group {
    fn rewrite(&mut self, range: std::ops::Range<usize>) {
        let mut builder = sunrise_client::gfx::PreparedInstanceBuilder::default();
        for i in range {
            let copy = self.copies[i];
            let local = self.transform * row_major_3x4(copy.transform);
            let m = local.to_cols_array_2d();
            self.composed[i] = copy;
            self.composed[i].transform = [
                m[0][0], m[1][0], m[2][0], m[3][0], m[0][1], m[1][1], m[2][1], m[3][1], m[0][2],
                m[1][2], m[2][2], m[3][2],
            ];
            let mut record = builder.instance(
                local,
                DVec3::ZERO,
                DVec3::ZERO,
                super::native_block_entity_draw::prepared_tint(copy.tint),
                sunrise_client::world_geometry::pack_light(copy.light),
            );
            record.tint[3] = (copy.tint >> 24) as f32 / 255.0;
            self.records[i] = record;
            #[cfg(test)]
            {
                self.converted_bytes += std::mem::size_of::<PreparedModelInstance>();
            }
        }
    }
}

#[derive(Default)]
pub(super) struct InstanceGroupStorage {
    next: u64,
    groups: Vec<Option<Group>>,
    retired: sunrise_client::fast_hash::FxHashMap<u64, Group>,
}
impl InstanceGroupStorage {
    #[cfg(test)]
    pub(super) fn converted_bytes(&self) -> usize {
        self.groups
            .iter()
            .flatten()
            .map(|g| g.converted_bytes)
            .sum()
    }
    pub(super) fn clear(&mut self) {
        self.groups.clear();
        self.retired.clear();
    }
    pub(super) fn retire_disabled(&mut self, disabled: &std::collections::BTreeSet<String>) {
        for slot in &mut self.groups {
            if slot
                .as_ref()
                .is_some_and(|g| g.owner.is_some_and(|id| disabled.contains(id)))
            {
                let group = slot.take().expect("disabled slot has a group");
                if group.submitted {
                    self.retired.insert(group.generation, group);
                }
            }
        }
    }
    fn get(
        &mut self,
        handle: InstanceGroupRef,
        model_generation: u64,
    ) -> Result<&mut Group, GroupError> {
        self.get_owned(
            mod_api::content::crash::current_native_mod(),
            handle,
            model_generation,
        )
    }
    fn get_owned(
        &mut self,
        _owner: Option<&'static str>,
        handle: InstanceGroupRef,
        model_generation: u64,
    ) -> Result<&mut Group, GroupError> {
        let fields = handle.fields();
        self.groups
            .get_mut(fields.slot as usize)
            .and_then(Option::as_mut)
            .filter(|g| {
                g.generation == fields.generation && g.model.fields().generation == model_generation
            })
            .ok_or(GroupError::Stale)
    }
}

impl InstanceGroups for Game<'_> {
    fn create_group(&self, model: ModelRef, capacity: u32) -> Result<InstanceGroupRef, GroupError> {
        let fields = model.fields();
        if fields.generation != self.standalone_geometry.generation() {
            return Err(GroupError::Model(mod_api::ModelError::StaleModel));
        }
        self.standalone_geometry
            .instance_slot(fields.slot)
            .ok_or(GroupError::Model(mod_api::ModelError::UnknownModel))?;
        let owner = mod_api::content::crash::current_native_mod();
        let mut storage = self.instance_groups.borrow_mut();
        self.prepared_model_draws
            .borrow_mut()
            .reserve((capacity as usize).min(16_384));
        storage.next = storage
            .next
            .checked_add(1)
            .expect("instance group epoch exhausted");
        let mut group = Group {
            generation: storage.next,
            owner,
            submitted: false,
            model,
            copies: vec![ModelInstance::at([0.0; 3], 0xFFFF_FFFF); capacity as usize],
            len: 0,
            transform: Mat4::IDENTITY,
            composed: vec![ModelInstance::at([0.0; 3], 0xFFFF_FFFF); capacity as usize],
            records: vec![
                PreparedModelInstance::new(
                    Mat4::IDENTITY,
                    DVec3::ZERO,
                    DVec3::ZERO,
                    [1.0; 4],
                    0
                );
                capacity as usize
            ],
            #[cfg(test)]
            converted_bytes: 0,
        };
        group.rewrite(0..capacity as usize);
        let index = storage
            .groups
            .iter()
            .position(Option::is_none)
            .unwrap_or(storage.groups.len());
        let handle = mod_api::__private::built::instance_group_ref(group.generation, index as u32);
        if index == storage.groups.len() {
            storage.groups.push(Some(group));
        } else {
            storage.groups[index] = Some(group);
        }
        Ok(handle)
    }
}

impl Game<'_> {
    pub(super) fn instance_group_write(
        &self,
        group: InstanceGroupRef,
        first: u32,
        copies: &[ModelInstance],
    ) -> Result<(), GroupError> {
        let mut storage = self.instance_groups.borrow_mut();
        let group = storage.get(group, self.standalone_geometry.generation())?;
        let end = (first as usize)
            .checked_add(copies.len())
            .filter(|end| *end <= group.copies.len())
            .ok_or(GroupError::OutOfRange)?;
        sunrise_client::world_geometry::validate_instances(copies)?;
        group.copies[first as usize..end].copy_from_slice(copies);
        group.rewrite((first as usize).min(group.len)..end);
        group.len = group.len.max(end);
        Ok(())
    }
    pub(super) fn instance_group_set_transform(
        &self,
        handle: InstanceGroupRef,
        transform: [f32; 12],
    ) -> Result<(), GroupError> {
        let mut storage = self.instance_groups.borrow_mut();
        let group = storage.get(handle, self.standalone_geometry.generation())?;
        if transform.iter().any(|v| !v.is_finite()) {
            return Err(GroupError::NonFinite);
        }
        group.transform = row_major_3x4(transform);
        group.rewrite(0..group.len);
        Ok(())
    }
    pub(super) fn instance_group_set_len(
        &self,
        group: InstanceGroupRef,
        len: u32,
    ) -> Result<(), GroupError> {
        let mut storage = self.instance_groups.borrow_mut();
        let group = storage.get(group, self.standalone_geometry.generation())?;
        if len as usize > group.copies.len() {
            return Err(GroupError::OutOfRange);
        }
        if len as usize > group.len {
            group.rewrite(group.len..len as usize);
        }
        group.len = len as usize;
        Ok(())
    }
    pub(super) fn instance_group_destroy(
        &self,
        handle: InstanceGroupRef,
    ) -> Result<(), GroupError> {
        let mut storage = self.instance_groups.borrow_mut();
        storage.get(handle, self.standalone_geometry.generation())?;
        let group = storage.groups[handle.fields().slot as usize]
            .take()
            .expect("validated group slot");
        if group.submitted {
            storage.retired.insert(group.generation, group);
        }
        Ok(())
    }
    pub(super) fn draw_instance_group(
        &self,
        draw: &mut sunrise_client::world_geometry::WorldGeometry,
        handle: InstanceGroupRef,
    ) -> Result<(), GroupError> {
        let mut storage = self.instance_groups.borrow_mut();
        let group = storage.get(handle, self.standalone_geometry.generation())?;
        group.submitted = true;
        draw.queue_group(handle);
        Ok(())
    }

    /// Resolve live data after the prepare hook, or the last data of an accepted group that
    /// was destroyed. The latter no longer occupies a public handle slot.
    pub(super) fn resolve_submitted_groups(
        &self,
        draw: &mut sunrise_client::world_geometry::WorldGeometry,
    ) {
        let mut storage = self.instance_groups.borrow_mut();
        let mut submitted = draw.take_submitted_groups();
        for &handle in &submitted {
            let fields = handle.fields();
            let group = if let Some(group) = storage.retired.get(&fields.generation) {
                group
            } else if let Some(group) = storage.groups[fields.slot as usize]
                .as_mut()
                .filter(|group| group.generation == fields.generation)
            {
                group.submitted = false;
                group
            } else {
                unreachable!("accepted group must be live or retained");
            };
            let _ = self.draw_cached_model_instances(
                draw,
                group.model,
                &group.composed[..group.len],
                Some(&group.records[..group.len]),
            );
        }
        for handle in submitted.drain(..) {
            storage.retired.remove(&handle.fields().generation);
        }
        draw.recycle_submitted_groups(submitted);
    }

    /// A frame can stop before prepare, for example on graphics device loss.
    pub(super) fn discard_submitted_groups(
        &self,
        draw: &mut sunrise_client::world_geometry::WorldGeometry,
    ) {
        let mut storage = self.instance_groups.borrow_mut();
        for handle in draw.drain_submitted_groups() {
            let fields = handle.fields();
            if storage.retired.remove(&fields.generation).is_none()
                && let Some(group) = storage.groups[fields.slot as usize]
                    .as_mut()
                    .filter(|group| group.generation == fields.generation)
            {
                group.submitted = false;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn range_cache_preserves_other_records_and_world_origins() {
        let copy = ModelInstance::at([10.0, 20.0, 30.0], 0xFFFF_FFFF);
        let mut group = Group {
            generation: 1,
            owner: None,
            submitted: false,
            model: mod_api::__private::built::model_ref(1, 0),
            copies: vec![copy; 2],
            len: 2,
            transform: row_major_3x4([0.0, -2.0, 0.0, 3.0, 2.0, 0.0, 0.0, 4.0, 0.0, 0.0, 2.0, 5.0]),
            composed: vec![copy; 2],
            records: vec![
                PreparedModelInstance::new(
                    Mat4::IDENTITY,
                    DVec3::ZERO,
                    DVec3::ZERO,
                    [1.0; 4],
                    0
                );
                2
            ],
            converted_bytes: 0,
        };
        group.rewrite(0..2);
        let untouched = group.records[0];
        group.copies[1].transform[3] = 1.0;
        group.copies[1].transform[7] = 2.0;
        group.copies[1].transform[11] = 3.0;
        group.rewrite(1..2);
        assert_eq!(group.records[0].model, untouched.model);
        assert_eq!(
            group.records[1]
                .matrix()
                .transform_point3(glam::Vec3::new(2.0, 3.0, 4.0)),
            glam::Vec3::new(-7.0, 10.0, 19.0)
        );
        assert_eq!(group.composed[1].origin, [10.0, 20.0, 30.0]);
        assert_eq!(
            group.converted_bytes,
            3 * std::mem::size_of::<PreparedModelInstance>()
        );
        // A quarter turn after scale(2, 4, 8): inverse-transpose columns
        // are (0, 1/2, 0), (-1/4, 0, 0), (0, 0, 1/8).
        group.transform =
            row_major_3x4([0.0, -4.0, 0.0, 3.0, 2.0, 0.0, 0.0, 4.0, 0.0, 0.0, 8.0, 5.0]);
        group.rewrite(0..1);
        assert_eq!(
            group.records[0].normal,
            [
                [0.0, 0.5, 0.0, 0.0],
                [-0.25, 0.0, 0.0, 0.0],
                [0.0, 0.0, 0.125, 0.0]
            ]
        );
        // Reflect the first scaled axis: only the first normal column changes sign.
        group.transform =
            row_major_3x4([0.0, -4.0, 0.0, 3.0, -2.0, 0.0, 0.0, 4.0, 0.0, 0.0, 8.0, 5.0]);
        group.rewrite(0..1);
        assert_eq!(
            group.records[0].normal,
            [
                [0.0, -0.5, 0.0, 0.0],
                [-0.25, 0.0, 0.0, 0.0],
                [0.0, 0.0, 0.125, 0.0]
            ]
        );
        let handle = mod_api::__private::built::instance_group_ref(1, 0);
        let owned_handle = mod_api::__private::built::instance_group_ref(2, 1);
        let owned = Group {
            generation: 2,
            owner: Some("group-owner"),
            submitted: false,
            model: mod_api::__private::built::model_ref(1, 0),
            copies: vec![copy],
            len: 1,
            transform: Mat4::IDENTITY,
            composed: vec![copy],
            records: vec![PreparedModelInstance::new(
                Mat4::IDENTITY,
                DVec3::ZERO,
                DVec3::ZERO,
                [1.0; 4],
                0,
            )],
            converted_bytes: 0,
        };
        let mut storage = InstanceGroupStorage {
            next: 2,
            groups: vec![Some(group), Some(owned)],
            retired: Default::default(),
        };
        assert!(storage.get(handle, 1).is_ok());
        assert!(storage.get(owned_handle, 1).is_ok());
        // Access checks generation and model lifetime, not the TLS caller mod name.
        {
            let _other =
                mod_platform::content::crash::enter_native_mod(mod_api::content::crash::ModCall {
                    id: "other",
                    hook: "test",
                    part: None,
                });
            assert!(storage.get(handle, 1).is_ok());
            assert!(storage.get(owned_handle, 1).is_ok());
            assert!(matches!(storage.get(handle, 2), Err(GroupError::Stale)));
        }
        // Disabling the owner still retires its groups without touching engine-owned ones.
        storage.retire_disabled(&std::collections::BTreeSet::from(["group-owner".into()]));
        assert!(storage.get(handle, 1).is_ok());
        assert!(matches!(
            storage.get(owned_handle, 1),
            Err(GroupError::Stale)
        ));
    }
}
