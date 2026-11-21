use super::*;

impl Game<'_> {
    /// A player (`player`, their UUID) took `taken` out of slot `slot` (the menu's index) of
    /// the mod menu type `menu` open on the mod block entity `entity`, which holds `left` after
    /// it: `Slot.onTake`, on the server, after the take. `entity` is out of the world during the
    /// call; the caller undoes the take when a listener fails.
    pub(in crate::game) fn native_menu_slot_take(
        &mut self,
        entity: &mut ModBlockEntity,
        player: u128,
        menu: &str,
        slot: usize,
        taken: &mod_api::ItemStack,
        left: Option<&mod_api::ItemStack>,
    ) {
        self.fire_menu_slot_take(entity, player, menu, slot, taken, left);
    }
}
