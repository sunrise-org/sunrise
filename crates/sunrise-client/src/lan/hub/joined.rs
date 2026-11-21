//! Entities that join a level between tracking passes (`ChunkMap.addEntity`): paired as they
//! joined, before their first tick.
use super::{Hub, entity_type, take_tracking, tracking_pass};
use crate::lan::entity::EntityState;

impl Hub {
    /// Fresh entities that joined during the current tick before the tracking pass:
    /// paired from their state as they joined (ChunkMap.addEntity), before their first tick.
    pub fn note_fresh_joined(&mut self, entities: &[EntityState]) {
        let joined: Vec<_> = entities
            .iter()
            .filter(|e| !self.tracks(e.id))
            .cloned()
            .collect();
        if joined.is_empty() {
            return;
        }
        self.fresh_joined
            .entry(self.config.dimension.clone())
            .or_default()
            .extend(joined);
    }

    /// `ChunkMap.addEntity` for late arrivals, before their sounds are sent.
    /// Pairing does not run another tracking pass or advance entity clocks.
    pub fn note_joined(&mut self, entities: &[EntityState]) {
        if entities.is_empty() {
            return;
        }
        let dimension = &self.config.dimension;
        let mut outs: Vec<Vec<crate::lan::Body>> = self
            .guests
            .iter()
            .filter(|g| self.here(g))
            .map(|_| Vec::new())
            .collect();
        let mut viewers = self.guests.tracking_viewers(
            |g| g.placed && g.dimension == *dimension,
            outs.iter_mut(),
            false,
        );
        let level = self.entities.entry(dimension.clone()).or_default();
        level.range_percent = Some(self.entity_range_percent);
        let (types, sync) = (&mut self.entity_types, &self.sync);
        level.pair_after_tick(entities, &self.riding, &mut viewers, |kind| {
            entity_type(types, sync, kind)
        });
        drop(viewers);
        // As a tracking pass's: held for the game while a mod listens (`tracking_pass`).
        let hold = tracking_pass::tracking_listened();
        let here: Vec<usize> = (0..self.guests.len())
            .filter(|&i| self.here(&self.guests[i]))
            .collect();
        for (i, out) in here.into_iter().zip(&mut outs) {
            if hold && self.guests[i].tracker.cut() {
                self.guests.update(i, |g| {
                    take_tracking(
                        g,
                        &mut self.native_tracking,
                        &mut self.attachment_snapshots,
                        self.sync_attachments,
                        Some(std::mem::take(out)),
                    );
                });
            } else {
                self.guests[i].sender.send_many(out.drain(..));
            }
        }
    }
}
