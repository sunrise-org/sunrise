//! Settled level-work validation and checked order allocation for prepared handoffs.
use super::*;
use std::collections::BTreeSet;
use sunrise_core::ticks::TransferTickError;

impl Ticks {
    /// Bounded diagnostic only: level handling/capture/updater/events/pistons/comparator/torch,
    /// followed by the actual block/fluid scheduler bits. Never drains or clears work.
    #[doc(hidden)]
    pub fn transfer_busy_bits(&self, selected: &BTreeSet<[i32; 3]>) -> [u8; 3] {
        let selected_pos = |p: BlockPos| selected.contains(&[p.x, p.y, p.z]);
        let level = u8::from(self.handling_tick)
            | (u8::from(self.capture.is_some()) << 1)
            | (u8::from(!self.updater.transfer_settled()) << 2)
            | (u8::from(self.block_events.iter().any(|e| selected_pos(e.pos))) << 3)
            | (u8::from(
                self.moving_pistons.iter().any(|(p, _)| selected_pos(*p))
                    || self.piston_pushes.iter().any(|(p, _, _)| selected_pos(*p)),
            ) << 4)
            | (u8::from(self.comparator_outputs.keys().any(|p| selected_pos(*p))) << 5)
            | (u8::from(self.torch_toggles.iter().any(|(p, _)| selected_pos(*p))) << 6);
        [
            level,
            self.blocks.transfer_busy_bits(),
            self.fluids.transfer_busy_bits(),
        ]
    }
    pub fn transfer_settled(&self, selected: &BTreeSet<[i32; 3]>) -> bool {
        !self.handling_tick
            && self.capture.is_none()
            && self.updater.transfer_settled()
            && self.blocks.transfer_settled()
            && self.fluids.transfer_settled()
            && !self
                .block_events
                .iter()
                .any(|e| selected.contains(&[e.pos.x, e.pos.y, e.pos.z]))
            && !self
                .moving_pistons
                .iter()
                .any(|(p, _)| selected.contains(&[p.x, p.y, p.z]))
            && !self
                .piston_pushes
                .iter()
                .any(|(p, _, _)| selected.contains(&[p.x, p.y, p.z]))
            && !self
                .comparator_outputs
                .keys()
                .any(|p| selected.contains(&[p.x, p.y, p.z]))
            && !self
                .torch_toggles
                .iter()
                .any(|(p, _)| selected.contains(&[p.x, p.y, p.z]))
    }
    pub fn transfer_order_cursor(&self) -> i64 {
        self.sub_tick
    }
    /// Rebase imported work after all existing live orders, including loaded negative orders.
    /// Check before Prepared; do not rely on next_sub_tick's ordinary wrapping behavior here.
    pub fn transfer_order_range(
        &self,
        count: usize,
    ) -> Result<std::ops::Range<i64>, TransferTickError> {
        if count == 0 {
            return Ok(self.sub_tick..self.sub_tick);
        }
        let queued = self.blocks.all_queued().chain(self.fluids.all_queued());
        // Unpacked saved occurrences use negative orders; new imported orders stay above them,
        // including when an unrelated resident container has not unpacked its saved list yet.
        let mut base = self.sub_tick.max(0);
        for tick in queued {
            if tick.sub_tick_order >= base {
                base = tick
                    .sub_tick_order
                    .checked_add(1)
                    .ok_or(TransferTickError::Budget)?;
            }
        }
        let end = base
            .checked_add(i64::try_from(count).map_err(|_| TransferTickError::Budget)?)
            .ok_or(TransferTickError::Budget)?;
        Ok(base..end)
    }
    pub fn reserve_prepared_transfer_orders(
        &mut self,
        expected_cursor: i64,
        range: std::ops::Range<i64>,
    ) -> Result<(), TransferTickError> {
        if self.sub_tick != expected_cursor {
            return Err(TransferTickError::Changed);
        }
        if range.start < expected_cursor || range.end < range.start {
            return Err(TransferTickError::Changed);
        }
        if range.is_empty() {
            return Ok(());
        }
        if self
            .blocks
            .all_queued()
            .chain(self.fluids.all_queued())
            .any(|tick| tick.sub_tick_order >= range.start)
        {
            return Err(TransferTickError::Changed);
        }
        self.sub_tick = range.end;
        Ok(())
    }
}
