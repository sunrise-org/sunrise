//! `ChunkHolder.broadcastChanges`: the light and blocks that changed in the level in place, to
//! the players sent those chunks, and, wrapped with the level, to the players whose client views
//! it (`viewed_levels`).
use super::{Guest, Hub};
use crate::{chunk::Chunk, lan::viewed_levels::send::Runs, world::World};
use sunrise_protocol::{
    packets::play::{self, Clientbound as C},
    viewed_levels::Level,
};

impl Hub {
    /// The players sent chunk `key` of the level in place: those in it (None), and those whose
    /// client views it, with the level to wrap with. The chunks each was sent come with it.
    fn chunk_holders(
        &self,
        key: (i32, i32),
    ) -> Vec<(
        usize,
        Option<Level>,
        &crate::fast_hash::FxHashSet<(i32, i32)>,
    )> {
        let viewed = self.viewed.active();
        (0..self.guests.len())
            .filter_map(|i| {
                let g = &self.guests[i];
                if self.watches(g) {
                    g.sent_chunks
                        .contains(&key)
                        .then_some((i, None, &g.sent_chunks))
                } else if viewed {
                    self.view_here(g)
                        .filter(|v| v.sent.contains(&key))
                        .map(|v| (i, Some(v.level), &v.sent))
                } else {
                    None
                }
            })
            .collect()
    }

    /// `body`, a play clientbound packet, to each of `holders`.
    fn send_to_holders(&self, holders: &[(usize, Option<Level>)], body: &[u8]) {
        for &(i, level) in holders {
            let g = &self.guests[i];
            match level {
                None => g.sender.send(body.to_vec()),
                Some(level) => {
                    let mut runs = Runs::new(level);
                    runs.push(body);
                    g.sender.send_many(runs.finish());
                }
            }
        }
    }

    /// `ChunkHolder.broadcastChanges`' light part: the light sections that
    /// changed in a column go to the players for whom it is on the border of
    /// what they track (`ChunkMap.isChunkOnTrackedBorder`); the others light
    /// the blocks they are sent themselves.
    pub(in crate::lan) fn broadcast_light_changes(&mut self, world: &mut World) {
        let Some(log) = world.light_log.as_mut() else {
            return;
        };
        if log.is_empty() {
            return;
        }
        let mut changes: Vec<_> = log.drain().collect();
        // One entry per column, so the order is the columns'.
        changes.sort_unstable_by_key(|&(column, _)| column);
        for ((x, z), sections) in changes {
            let viewers: Vec<(usize, Option<Level>)> = self
                .chunk_holders((x, z))
                .into_iter()
                .filter(|(_, _, sent)| {
                    (-1..=1).any(|dz| (-1..=1).any(|dx| !sent.contains(&(x + dx, z + dz))))
                })
                .map(|(i, level, _)| (i, level))
                .collect();
            if viewers.is_empty() {
                continue;
            }
            let Some(chunk) = world.chunks.get(&(x, z)) else {
                continue;
            };
            let body = self.encoder.light_update(chunk, sections);
            self.send_to_holders(&viewers, &body);
        }
    }

    pub(in crate::lan) fn broadcast_block_changes(&mut self, world: &mut World) {
        // Chunks changed in bulk (`World::edit_runs`) go whole to those who see them: one packet each, where a
        // change of each block would be thousands (the client lights and meshes them once, on its workers).
        let bulk: Vec<(i32, i32)> = if world.bulk_log.is_empty() {
            Vec::new()
        } else {
            let mut keys: Vec<_> = world.bulk_log.drain().collect();
            keys.sort_unstable();
            keys
        };
        if !bulk.is_empty() {
            // The single-player window's far terrain draws a column it isn't sent as the window last saw it: it is
            // handed the column to summarize again (`handoff::changed`).
            let unsent = |g: &Guest, key: &(i32, i32)| {
                self.here(g)
                    && g.address() == crate::lan::conn::MEMORY_ADDRESS
                    && !g.sent_chunks.contains(key)
            };
            for key in &bulk {
                if let Some(chunk) = world.chunks.get(key)
                    && self.guests.iter().any(|g| unsent(g, key))
                {
                    crate::lan::handoff::changed(chunk);
                }
            }
            let columns: Vec<&Chunk> = bulk
                .iter()
                .filter_map(|k| world.chunks.get(k).map(|c| &**c))
                .collect();
            self.encoder.warm(&columns);
            for chunk in columns {
                let key = (chunk.x, chunk.z);
                let holders = self.holders(key);
                if holders.is_empty() {
                    continue;
                }
                let body = self.encoder.body_in(chunk, world);
                self.send_to_holders(&holders, &body);
            }
        }
        let Some(log) = world.block_log.as_mut() else {
            return;
        };
        if !bulk.is_empty() {
            // (what changed in them one at a time went with them, but for one-shot fields: a chunk has none)
            log.retain(|p| {
                bulk.binary_search(&(p.0 >> 4, p.2 >> 4)).is_err()
                    || matches!(
                        world.block_entities.get(p),
                        Some(crate::containers::BlockEntity::Mod(m)) if m.has_client_update_fields()
                    )
            });
        }
        if log.is_empty() {
            return;
        }
        let mut changes = std::mem::take(log);
        changes.sort_unstable();
        changes.dedup();
        // Group by section.
        let mut sections: std::collections::BTreeMap<(i32, i32, i32), Vec<(i32, i32, i32)>> =
            Default::default();
        for p in changes {
            sections
                .entry((p.0 >> 4, p.1 >> 4, p.2 >> 4))
                .or_default()
                .push(p);
        }
        for ((sx, sy, sz), positions) in sections {
            let holders = self.holders((sx, sz));
            if holders.is_empty() {
                continue;
            }
            let packet = if positions.len() == 1 {
                let (x, y, z) = positions[0];
                C::BlockUpdate(play::BlockUpdate {
                    pos: [x, y, z],
                    state: world.block(x, y, z).0 as i32,
                })
            } else {
                C::SectionBlocksUpdate(play::SectionBlocksUpdate {
                    section: [sx, sy, sz],
                    changes: positions
                        .iter()
                        .map(|&(x, y, z)| {
                            let state = i64::from(world.block(x, y, z).0);
                            state << 12
                                | i64::from(x & 15) << 8
                                | i64::from(z & 15) << 4
                                | i64::from(y & 15)
                        })
                        .collect(),
                })
            };
            self.send_to_holders(&holders, &packet.encode());
            // Block entities with data the client shows.
            for &(x, y, z) in &positions {
                let state = world.block(x, y, z).0;
                if !self
                    .encoder
                    .entity_matches(state, world.block_entities.get(&(x, y, z)))
                {
                    continue;
                }
                // A decorated pot is no block entity: its tag is the chunk's saved one.
                let state = world.block(x, y, z).0;
                let tag = match world.block_entities.get(&(x, y, z)) {
                    Some(entity) => self.encoder.change_tag((x, y, z), state, entity),
                    None => world
                        .chunks
                        .get(&(x >> 4, z >> 4))
                        .and_then(|c| crate::lan::chunks::pot_tag(c, (x, y, z))),
                };
                let Some(tag) = tag else {
                    continue;
                };
                let Some(kind) = self.encoder.entity_type(state) else {
                    continue;
                };
                let body = C::BlockEntityData(play::BlockEntityData {
                    pos: [x, y, z],
                    kind,
                    tag,
                })
                .encode();
                self.send_to_holders(&holders, &body);
            }
        }
    }

    /// [`Self::chunk_holders`] without the chunks they were sent.
    fn holders(&self, key: (i32, i32)) -> Vec<(usize, Option<Level>)> {
        self.chunk_holders(key)
            .into_iter()
            .map(|(i, level, _)| (i, level))
            .collect()
    }
}
