use super::*;
use sunrise_client::creative::Creative;

/// Helpers for native mods, kept as stable as the hook points.
// Native mods use these; nothing in Sunrise does.
impl crate::game::ClientGame<'_> {
    /// `screen_hovered_stack` at `(x, y)` in GUI pixels. A creative item tab's display
    /// slot shows the tab's list (`ItemPickerMenu`), scrolled to its first visible row.
    #[allow(dead_code)]
    pub(crate) fn hovered_stack_at(&self, batch: &HudBatch, x: f32, y: f32) -> Option<ItemStack> {
        self.hovered_stack_at_in(batch.size(), x, y)
    }

    #[inline(always)]
    pub(crate) fn hovered_stack_at_in(&self, gui: (f32, f32), x: f32, y: f32) -> Option<ItemStack> {
        if !matches!(self.client.screen, Screen::Inventory | Screen::Creative) {
            return None;
        }
        let slot = self.slot_at_in(gui, x, y)?;
        if self.creative_tab_open() && slot >= Creative::DISPLAY {
            return self.creative_display_stack(slot);
        }
        self.slot_view(slot).filter(|stack| stack.count > 0)
    }
    /// The full stack in the slot under the pointer on the open container or creative screen
    /// (`AbstractContainerScreen.getHoveredSlot().getItem()`), None over no slot, an empty one,
    /// or with no such screen open.
    pub fn screen_hovered_stack(&self) -> Option<ItemStack> {
        if !self.screen_hooks_open() {
            return None;
        }
        let size = self
            .client
            .renderer
            .0
            .as_ref()
            .map(|r| [r.config.width, r.config.height])?;
        let frame = self.gui_frame_for(size);
        self.hovered_stack_at_in(frame.size(), self.client.cursor.0, self.client.cursor.1)
    }
}
