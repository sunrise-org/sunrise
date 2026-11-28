use super::*;

impl crate::game::ServerGame<'_> {
    pub fn server_update_world(&mut self) {
        if let Err(error) = self.server_update_world_with_eviction(false) {
            log::warn!("Sunrise: failed to save unloaded chunks or entities: {error}");
        }
    }

    /// Forced idle eviction waits for writes before retiring columns.
    pub(in crate::game) fn server_update_world_with_eviction(&mut self, wait: bool) -> Result<()> {
        let dim = self.common.dim;
        self.ask_edited_light_emission(dim);
        sunrise_level::spans::time("tick_pregen", || self.tick_pregen());
        if self.defer_chunk_disk_save() {
            return if wait {
                self.admit_level_save()
            } else {
                Ok(())
            };
        }
        self.prune_disabled_async_chunk_requests();
        let areas = self.server.streaming.extra.len();
        self.update_world_server(dim);
        self.server.streaming.hold_unknown_tickets();
        let mut plan = sunrise_level::spans::time("streaming_plan", || {
            let mut step = self
                .server
                .streaming
                .begin_plan_update(&mut self.common.world);
            loop {
                match step {
                    sunrise_level::chunk_streamer::ChunkPlanStep::Ready(plan) => break plan,
                    sunrise_level::chunk_streamer::ChunkPlanStep::Save { chunk, resume } => {
                        self.save_late_full_chunk(chunk);
                        step = self
                            .server
                            .streaming
                            .resume_plan_update(&mut self.common.world, resume);
                    }
                }
            }
        });
        if wait {
            // Do not replace stored entities while their load is pending.
            plan.retain_unloading_columns(|key| !self.common.world.entities_pending.contains(key));
        }
        let saved = (|| -> Result<_> {
            let keys = plan.unloading_columns();
            if keys.is_empty() {
                return Ok((Vec::new(), HashSet::new()));
            }
            self.admit_level_save()?;
            let chunks = self.with_entities(
                keys.iter()
                    .filter(|key| self.common.world.modified.contains(key))
                    .filter_map(|key| self.common.world.chunks.get(key)),
            );
            // `SaveDir::entity_records` is the single filter for failed entity
            // reads: capture the unloading snapshots, then remove and notify
            // only for the chunks that were actually queued/saved.
            let (entities, refused_entities) = self.unloading_entity_snapshots(keys);
            let mut counts = Vec::new();
            if mod_api::__private::listened(mod_api::HookId::EntitiesUnload) {
                counts.extend(entities.iter().map(|(key, rows)| (*key, rows.len())));
            }
            let saved_entities = if wait {
                let mut saved_entities: HashSet<_> = entities.iter().map(|(key, _)| *key).collect();
                let refused = self.server.save.0.save_eviction(
                    &chunks,
                    self.common.pack.assets.registry,
                    world_biomes!(self),
                    entities,
                )?;
                saved_entities.retain(|key| !refused.contains(key));
                plan.retain_unloading_columns(|key| {
                    !refused.contains(key) && !refused_entities.contains(key)
                });
                saved_entities
            } else {
                if !chunks.is_empty() {
                    self.server.save.0.save_later(
                        &chunks,
                        self.common.pack.assets.registry,
                        world_biomes!(self),
                    )?;
                }
                let saved_entities = self.server.save.0.save_entities_later(entities)?;
                plan.retain_unloading_columns(|key| !refused_entities.contains(key));
                saved_entities
            };
            Ok((counts, saved_entities))
        })();
        if saved.is_err() {
            plan.keep_columns();
            self.server.streaming.force_unload_sweep();
        }
        let (commit, protos) = self
            .server
            .streaming
            .prepare_commit(&mut self.common.world, &mut plan);
        let unloaded_columns = plan.unloading_columns();
        self.server.streaming.submit_protos(protos);
        let unloaded = sunrise_level::spans::time("streaming", || {
            self.server
                .streaming
                .complete_commit(&mut self.common.world, commit)
        });
        self.server.streaming.extra.truncate(areas);
        sunrise_level::spans::time("spawn_fresh_chunks", || self.spawn_fresh_chunks());
        self.ask_loaded_light_emission(0);
        sunrise_level::spans::time("mods_chunks_changed", || {
            self.native_chunks_loaded();
            self.native_chunks_unloaded(&unloaded);
        });
        self.process_async_chunk_requests();
        for key in unloaded_columns {
            self.common.world.modified.remove(key);
            self.common.world.modified.unloaded(key);
        }
        if !unloaded_columns.is_empty() {
            if let Ok((counts, saved_entities)) = &saved {
                let removed_entities: HashSet<_> = saved_entities
                    .intersection(unloaded_columns)
                    .copied()
                    .collect();
                self.remove_unloading_entities(&removed_entities);
                for &(chunk, count) in counts {
                    if removed_entities.contains(&chunk) {
                        self.native_entities_unload(dim.id(), [chunk.0, chunk.1], count);
                    }
                }
            }
            sunrise_level::spans::time("unload_block_entities", || {
                for &column in unloaded_columns {
                    self.common.world.block_entities.unload_column(column);
                }
            });
            self.common
                .world
                .block_names
                .retain(|p, _| !unloaded_columns.contains(&(p.0 >> 4, p.2 >> 4)));
        }
        sunrise_level::spans::time("sync_tick_chunks", || self.sync_tick_chunks());
        saved.map(|_| ())
    }
    #[inline(always)]
    pub(in crate::game::server_side) fn update_world_server(&mut self, dim: Dim) {
        // The configuration connection is not a guest and owns no player area.
        // `extra` finishes chunks through radius + 1, even with no players.
        self.server.streaming.extra.extend(
            self.server
                .chunk_ticking
                .tickets
                .spawn_areas()
                .map(|(center, radius)| (center, radius - 1)),
        );
        if let Some(host) = &self.server.lan {
            self.server.streaming.extra.extend(
                host.hub
                    .lan
                    .configuration_tasks
                    .loading_spawns(dim.id())
                    .filter(|&center| !self.server.chunk_ticking.tickets.holds_player_spawn(center))
                    .map(|center| (center, sunrise_sim::ticket_levels::PLAYER_SPAWN_RADIUS - 1)),
            );
        }
        self.server.streaming.extra.extend(
            self.server
                .server_trips
                .iter()
                .flat_map(|trip| trip.spawn_destination_areas(dim)),
        );
        let waiting = std::mem::take(&mut self.server.spawn_wait);
        self.server
            .streaming
            .extra
            .extend(waiting.into_iter().map(|chunk| (chunk, 0)));
    }
}

use sunrise_gameplay_common::game::native_hooks::SharedGameExt1 as _;
