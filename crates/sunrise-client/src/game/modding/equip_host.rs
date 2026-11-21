//! Read-only equipment host: prepared facts for the target, ordinary game reads elsewhere.
use super::Game;
use mod_api::__private::game::BackendRefOf;
use mod_api::{
    Backend, BackendRef, EntityRef, EntityTypeKey, EquipmentSlot, Host, ItemStack, ItemStackView,
};

/// One target's read view, captured before slot traversal and compared again before apply.
// no wire: borrowed by the native equipment host and never sent out of process.
#[derive(PartialEq)]
pub(in crate::game) struct EquipSnapshot {
    pub(in crate::game) entity: EntityRef,
    pub(in crate::game) kind: Option<EntityTypeKey>,
    pub(in crate::game) position: Option<[f64; 3]>,
    pub(in crate::game) health: Option<f32>,
    pub(in crate::game) max_health: Option<f32>,
    pub(in crate::game) alive: Option<bool>,
    pub(in crate::game) living: bool,
    // Slots in EquipmentSlot order; empty or unsupported slots are None.
    pub(in crate::game) equipment: [Option<ItemStack>; 8],
}

pub(in crate::game) struct EquipHost<'g, 'a> {
    pub(in crate::game) game: &'g Game<'a>,
    pub(in crate::game) row: &'g EquipSnapshot,
}
impl EquipHost<'_, '_> {
    pub(in crate::game) fn with_equipment(
        &self,
        slot: EquipmentSlot,
        read: &mut dyn for<'a> FnMut(ItemStackView<'a>),
    ) -> Option<()> {
        read(ItemStackView::from_stack(
            self.row.equipment[slot as usize].as_ref()?,
        ));
        Some(())
    }
}
impl Host for EquipHost<'_, '_> {
    fn backend(&self) -> Option<BackendRef<'_>> {
        Some(BackendRef::of(self))
    }
}
impl Backend for EquipHost<'_, '_> {}
