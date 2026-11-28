/// The hook points. Each does nothing in Sunrise.
impl crate::game::ServerGame<'_> {
    /// Fires `enchantment_level_set` for one offer of the table at `pos`
    /// (`EnchantmentMenu.slotsChanged`); the contract is the hook's
    /// documentation in `sunrise-mod-vocab`. Server-side.
    pub(in crate::game) fn native_enchantment_level_set(
        &mut self,
        pos: [i32; 3],
        offer: &mod_api::item::stations::EnchantmentRow<'_>,
        level: &mut i32,
    ) {
        self.fire_enchantment_level_set(pos, offer, level);
    }
}
