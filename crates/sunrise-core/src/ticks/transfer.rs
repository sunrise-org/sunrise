//! Prepared ownership handoff of occurrences from the one existing scheduler.
use super::*;
use std::collections::BTreeSet;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransferTickError {
    Busy,
    MissingChunk,
    Changed,
    Occupied,
    Budget,
}

/// A read witness or a transient removed occurrence, never another ticking scheduler.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TransferOccurrence {
    pub tick: ScheduledTick,
    pub held: bool,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TransferTicks {
    cells: BTreeSet<[i32; 3]>,
    occurrences: Vec<TransferOccurrence>,
}
impl TransferTicks {
    pub fn occurrences(&self) -> &[TransferOccurrence] {
        &self.occurrences
    }
}
fn xyz(p: BlockPos) -> [i32; 3] {
    [p.x, p.y, p.z]
}
fn columns(cells: &BTreeSet<[i32; 3]>) -> BTreeSet<(i32, i32)> {
    cells.iter().map(|p| (p[0] >> 4, p[2] >> 4)).collect()
}
impl LevelTicks {
    /// Bounded diagnostic bits: collecting, runnable, already-run, run-set, container heap.
    #[doc(hidden)]
    pub fn transfer_busy_bits(&self) -> u8 {
        u8::from(self.collecting)
            | (u8::from(!self.to_run.is_empty()) << 1)
            | (u8::from(!self.already_run.is_empty()) << 2)
            | (u8::from(!self.to_run_set.is_empty()) << 3)
            | (u8::from(!self.containers_to_tick.values.is_empty()) << 4)
    }
    pub fn transfer_settled(&self) -> bool {
        !self.collecting
            && self.to_run.is_empty()
            && self.already_run.is_empty()
            && self.to_run_set.is_empty()
            && self.containers_to_tick.values.is_empty()
    }
    pub fn capture_transfer_cells(
        &self,
        cells: &BTreeSet<[i32; 3]>,
        maximum: usize,
    ) -> Result<TransferTicks, TransferTickError> {
        if !self.transfer_settled() {
            return Err(TransferTickError::Busy);
        }
        let mut occurrences = Vec::new();
        for (x, z) in columns(cells) {
            let chunk = self
                .chunk(ChunkPos::new(x, z))
                .ok_or(TransferTickError::MissingChunk)?;
            // Ordinary chunk registration leaves Some(empty) on chunks with no saved work:
            // only a nonempty list needs the source level's first-ticking unpack frontier.
            if chunk
                .pending
                .as_ref()
                .is_some_and(|pending| !pending.is_empty())
            {
                return Err(TransferTickError::Busy);
            }
            let mut add = |tick: ScheduledTick, held| {
                if !cells.contains(&xyz(tick.pos)) {
                    return Ok(());
                }
                if occurrences.len() == maximum {
                    return Err(TransferTickError::Budget);
                }
                occurrences.push(TransferOccurrence { tick, held });
                Ok(())
            };
            for &tick in chunk.queued() {
                add(tick, false)?;
            }
            // A hash-map iteration must not become imported tie order.
            let mut held: Vec<_> = chunk.held.iter().collect();
            held.sort_by_key(|((kind, p), _)| (kind.0, p.x, p.y, p.z));
            for (_, ticks) in held {
                for &tick in ticks {
                    add(tick, true)?;
                }
            }
        }
        // Stable sorting preserves the existing occurrence order when every comparison key ties.
        occurrences.sort_by_key(|o| {
            (
                o.tick.sub_tick_order,
                o.tick.trigger_tick,
                o.tick.priority as i8,
                o.tick.kind.0,
                o.tick.pos.x,
                o.tick.pos.y,
                o.tick.pos.z,
                o.held,
            )
        });
        Ok(TransferTicks {
            cells: cells.clone(),
            occurrences,
        })
    }
    pub fn validate_transfer_cells(
        &self,
        witness: &TransferTicks,
    ) -> Result<(), TransferTickError> {
        let current = self
            .capture_transfer_cells(&witness.cells, witness.occurrences.len())
            .map_err(|error| {
                if error == TransferTickError::Budget {
                    TransferTickError::Changed
                } else {
                    error
                }
            })?;
        if current != *witness {
            return Err(TransferTickError::Changed);
        }
        Ok(())
    }
    /// Validate the entire selector before removing its first actual occurrence.
    pub fn extract_transfer_cells(
        &mut self,
        witness: &TransferTicks,
    ) -> Result<Vec<TransferOccurrence>, TransferTickError> {
        let current = self
            .capture_transfer_cells(&witness.cells, witness.occurrences.len())
            .map_err(|error| {
                if error == TransferTickError::Budget {
                    TransferTickError::Changed
                } else {
                    error
                }
            })?;
        if current != *witness {
            return Err(TransferTickError::Changed);
        }
        for (x, z) in columns(&witness.cells) {
            let key = ChunkPos::new(x, z).pack();
            let chunk = self
                .chunks
                .get_mut(&key)
                .ok_or(TransferTickError::MissingChunk)?;
            let before_count = chunk.count();
            // Remove through the ordinary heap operation, preserving all unrelated heap behavior.
            chunk.queue.remove_if(
                |tick| witness.cells.contains(&xyz(tick.pos)),
                ScheduledTick::drain_cmp,
            );
            let mut keys: Vec<_> = chunk.held.keys().copied().collect();
            keys.sort_by_key(|(kind, p)| (kind.0, p.x, p.y, p.z));
            for held_key in keys {
                if witness.cells.contains(&xyz(held_key.1)) {
                    chunk.held.remove(&held_key);
                }
            }
            chunk
                .positions
                .retain(|(_, p)| !witness.cells.contains(&xyz(*p)));
            if chunk.count() != before_count {
                chunk.changed_for_save();
            }
            if chunk.held.is_empty() {
                self.held_chunks.remove(&key);
            }
            if let Some(next) = chunk.peek() {
                self.deadlines.put(key, next.trigger_tick);
            } else {
                self.deadlines.remove(key);
            }
            chunk.trim();
        }
        // ScheduledTick is Copy plain data. These exact actual occurrences become the moved
        // values only after the original containers have all relinquished their selected work.
        Ok(current.occurrences)
    }
    pub fn preflight_transfer_install(
        &self,
        occurrences: &[TransferOccurrence],
    ) -> Result<(), TransferTickError> {
        if !self.transfer_settled() {
            return Err(TransferTickError::Busy);
        }
        for occurrence in occurrences {
            let p = occurrence.tick.pos;
            let chunk = self
                .chunk(p.chunk())
                .ok_or(TransferTickError::MissingChunk)?;
            if chunk
                .pending
                .as_ref()
                .is_some_and(|pending| !pending.is_empty())
            {
                return Err(TransferTickError::Busy);
            }
            if chunk.queued().iter().any(|t| t.pos == p)
                || chunk.held.keys().any(|(_, at)| *at == p)
            {
                return Err(TransferTickError::Occupied);
            }
        }
        Ok(())
    }
    /// Installs owned occurrences without first-schedule-wins deduplication. Duplicates loaded
    /// from an ordinary saved list remain distinct. Call only after whole-batch live preflight.
    pub fn install_transfer_occurrences(
        &mut self,
        owned: Vec<TransferOccurrence>,
    ) -> Result<(), TransferTickError> {
        self.preflight_transfer_install(&owned)?;
        for occurrence in owned {
            let tick = occurrence.tick;
            let key = tick.pos.chunk().pack();
            let chunk = self
                .chunks
                .get_mut(&key)
                .ok_or(TransferTickError::MissingChunk)?;
            if occurrence.held {
                chunk.hold(tick);
                self.held_chunks.insert(key);
            } else {
                chunk.positions.insert(tick.key());
                chunk.queue.push(tick, ScheduledTick::drain_cmp);
                chunk.changed_for_save();
            }
            if let Some(next) = chunk.peek() {
                self.deadlines.put(key, next.trigger_tick);
            }
        }
        Ok(())
    }
}
