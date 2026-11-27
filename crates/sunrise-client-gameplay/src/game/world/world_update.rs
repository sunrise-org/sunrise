//! The frame's world update (`update_world`): what arrived from the server, the chunks that
//! streamed in and the meshes handed to the renderer, the columns that unload (their entities
//! and block entities saved out first).
use super::*;

impl crate::game::ClientGame<'_> {
    pub fn client_update_world(&mut self) {
        debug_assert!(
            !self.is_logical_server(),
            "client_update_world called on server"
        );
        if let Err(e) = sunrise_level::spans::time("poll_remote", || self.poll_remote()) {
            log::warn!("Sunrise: {e}");
        }
        // A disconnect during poll_remote ends the client session; stop before world work runs.
        if self.client.title.0.is_some() || self.client.remote.is_none() {
            return;
        }
        self.poll_remote_light();
        // The renderer laid translucent quads out again (a shader pack's flat terrain came
        // or went) and dropped every section: meshed again, as for a new one.
        if self
            .client
            .renderer
            .0
            .as_deref_mut()
            .is_some_and(Renderer::take_dropped_sections)
        {
            self.all_sections_changed();
        }
        self.ask_edited_light_emission();
        let camera_pos = self.visual_camera_position();
        if let Some(pos) = camera_pos {
            self.client.streaming.center =
                ((pos[0].floor() as i32) >> 4, (pos[2].floor() as i32) >> 4);
            self.client.streaming.center_y = pos[1].floor() as i32;
        }
        // A headless game has nothing to draw sections with: like a server's, they aren't
        // meshed while it has no renderer.
        if self.common.options.headless {
            self.client.streaming.mesh = self.client.renderer.0.is_some();
        }
        let areas = self.client.streaming.extra.len();
        // `Level.getChunk`'s tickets keep their columns until they expire.
        self.client.streaming.hold_unknown_tickets();
        self.client.streaming.upload_backlog = self.client.pending_uploads.0.len();
        let mut plan = sunrise_level::spans::time("streaming_plan", || {
            let mut step = self
                .client
                .streaming
                .begin_plan_update(&mut self.common.world);
            loop {
                match step {
                    sunrise_client_render::streaming::PlanStep::Ready(plan) => break plan,
                    sunrise_client_render::streaming::PlanStep::Save { chunk, resume } => {
                        self.client.streaming.save_finished(chunk);
                        step = self
                            .client
                            .streaming
                            .resume_plan_update(&mut self.common.world, resume);
                    }
                }
            }
        });
        let (commit, protos) = self
            .client
            .streaming
            .prepare_commit(&mut self.common.world, &mut plan);
        self.client.streaming.submit_protos(protos);
        let (mut uploads, unloaded) = sunrise_level::spans::time("streaming", || {
            self.client
                .streaming
                .complete_commit(&mut self.common.world, commit)
        });
        // Sections chosen to mesh wait for the mods' `section_visibility` (none without a mod).
        self.prepare_section_visibility();
        self.client.streaming.extra.truncate(areas);
        // The viewed levels' meshing, queued after the player's level's (nothing while no view
        // names one).
        if self.client.streaming.mesh
            && let Some(remote) = &mut self.client.remote
        {
            let streaming = &self.client.streaming;
            remote
                .viewed
                .mesh_frame(|| streaming.viewed_mesh_source(), std::time::Instant::now());
        }
        self.ask_loaded_light_emission();
        // Vanilla saves the chunk before its unload hook and drops its block entities afterwards.
        sunrise_level::spans::time("mods_chunks_changed", || {
            self.native_chunks_loaded();
            self.native_chunks_unloaded(&unloaded);
        });
        for chunk in &unloaded {
            self.common.world.modified.unloaded(&(chunk.x, chunk.z));
        }
        for chunk in &unloaded {
            self.common.world.modified.unloaded(&(chunk.x, chunk.z));
        }
        if !unloaded.is_empty() {
            let gone: sunrise_level::fast_hash::FxHashSet<(i32, i32)> =
                unloaded.iter().map(|c| (c.x, c.z)).collect();
            sunrise_level::spans::time("unload_block_entities", || {
                for &column in &gone {
                    self.common.world.block_entities.unload_column(column);
                }
            });
            self.common
                .world
                .block_names
                .retain(|p, _| !gone.contains(&(p.0 >> 4, p.2 >> 4)));
        }
        // Terrain still on its way from the server (single player's integrated one
        // too): the view's outermost columns not all in, or its events not all handled.
        let radius = match &self.client.remote {
            // As far as the server fills it: its view distance and one.
            Some(r) => self.client.streaming.radius.min(r.view_distance + 1),
            None => self.client.streaming.radius,
        };
        let view = (self.client.streaming.center, radius);
        let has = |column: (i32, i32)| self.common.world.chunks.contains_key(&column);
        let now = Instant::now();
        let arriving = {
            let _span = sunrise_level::spans::guard("view_arrival");
            self.client.view_arrival.arriving(view, now, has) || self.remote_backlog() > 0
        };
        if let Some(renderer) = self.client.renderer.0.as_mut() {
            let g = &self.client.settings.sunrise;
            let budgeted = g.chunk_updates > 0 || g.upload_budget_kib > 0;
            let mut raw_flight = self.client.flight.0.as_mut().filter(|flight| flight.raw);
            // The meshes, once copied, are freed by a worker (`MeshStreamer::free`).
            if !budgeted
                && self.client.pending_uploads.0.is_empty()
                && uploads.len() <= AUTO_UPLOADS_AT_ONCE
            {
                sunrise_level::spans::time("upload", || {
                    for upload in &uploads {
                        if self
                            .client
                            .streaming
                            .can_publish_mesh(&self.common.world, upload)
                        {
                            let bytes = renderer.mesh_upload_bytes(&upload.mesh);
                            renderer.upload(upload.key, &upload.mesh);
                            if let Some(flight) = raw_flight.as_deref_mut() {
                                flight.note_mesh_uploads(1, bytes);
                            }
                        }
                    }
                });
                self.client.streaming.free(uploads);
            } else {
                // Sunrise's chunk-update and upload budgets: the nearest
                // sections go first, the rest wait for later frames; a newer
                // mesh of a waiting section replaces it, and sections of
                // chunks no longer loaded are dropped.
                let pending = &mut self.client.pending_uploads.0;
                let order_span = sunrise_level::spans::guard("order_uploads");
                let fresh: sunrise_level::fast_hash::FxHashSet<(i32, i32, i32)> =
                    uploads.iter().map(|upload| upload.key).collect();
                let current = |upload: &sunrise_client_render::streaming::MeshUpload| {
                    self.common
                        .world
                        .chunks
                        .contains_key(&(upload.key.0, upload.key.2))
                        && self
                            .client
                            .streaming
                            .can_publish_mesh(&self.common.world, upload)
                };
                // Filtering leaves the surviving waiting run in the same order.
                let mut spent: Vec<_> = pending
                    .extract_if(.., |upload| fresh.contains(&upload.key) || !current(upload))
                    .collect();
                spent.extend(uploads.extract_if(.., |upload| !current(upload)));
                let centre = self.client.streaming.center;
                // Section keys count from the world's bottom (`origin`'s min_y + y * 16).
                let bottom = f64::from(self.common.world.bottom_y());
                let cam_y = camera_pos.map_or(4, |pos| ((pos[1] - bottom) / 16.0).floor() as i32);
                let far = |k: &(i32, i32, i32)| section_distance_sq(*k, centre, cam_y);
                let order = Some((centre, cam_y));
                let priority = |upload: &sunrise_client_render::streaming::MeshUpload| {
                    std::cmp::Reverse(far(&upload.key))
                };
                let work = order_uploads(
                    pending,
                    &mut uploads,
                    &mut self.client.pending_uploads_merge.0,
                    self.client.pending_uploads_order == order,
                    priority,
                );
                self.client.pending_uploads_order = order;
                drop(order_span);
                let (limit, budget) = if budgeted {
                    (
                        g.chunk_updates as usize,
                        u64::from(g.upload_budget_kib) * 1024,
                    )
                } else {
                    (0, 0)
                };
                let (mut count, mut bytes) = (0, 0u64);
                // Without the player's budgets, uploads stop once they took this
                // frame's share (`UploadPace::budget`: a quarter of the recent frame
                // time, more with a backlog, at most 2 ms; at least one goes): a burst
                // of finished meshes spreads over frames, nearest first.
                let auto =
                    (!budgeted).then(|| self.client.upload_pace.budget(pending.len(), arriving));
                let started = Instant::now();
                let upload_span = sunrise_level::spans::guard("upload_queue");
                while let Some(upload) = pending.last() {
                    let mesh = &upload.mesh;
                    if auto.is_some_and(|time| count > 0 && started.elapsed() >= time) {
                        break;
                    }
                    let size = renderer.mesh_upload_bytes(mesh);
                    if (limit > 0 && count >= limit)
                        || (budget > 0 && bytes > 0 && bytes + size > budget)
                    {
                        break;
                    }
                    let upload = pending.pop().expect("checked above");
                    renderer.upload(upload.key, &upload.mesh);
                    spent.push(upload);
                    count += 1;
                    bytes += size;
                }
                drop(upload_span);
                if let Some(flight) = raw_flight {
                    flight.note_upload_queue_work(work);
                    flight.note_mesh_uploads(count, bytes);
                }
                self.client.streaming.free(spent);
            }
            renderer.remove_columns(unloaded.iter().map(|c| (c.x, c.z)));
            // Meshes in the works are the view's terrain while it is new, then edits'.
            let meshing = self.client.streaming.busy() || !self.client.pending_uploads.0.is_empty();
            renderer
                .set_terrain_loading(arriving || (meshing && self.client.view_arrival.recent(now)));
        }
    }
}

/// Farthest first (the uploader pops from the back). Returns the number of
/// elements passed to a sort and to the merge, respectively.
pub(in crate::game::client_side) fn order_uploads<T, K: Ord>(
    waiting: &mut Vec<T>,
    fresh: &mut Vec<T>,
    scratch: &mut Vec<T>,
    same_section: bool,
    priority: impl Fn(&T) -> K,
) -> [usize; 2] {
    if !same_section {
        waiting.append(fresh);
        if waiting.len() < 2 {
            return [0, 0];
        }
        waiting.sort_unstable_by_key(priority);
        return [waiting.len(), 0];
    }
    let sorted = if fresh.len() > 1 {
        fresh.sort_unstable_by_key(&priority);
        fresh.len()
    } else {
        0
    };
    if waiting
        .last()
        .zip(fresh.first())
        .is_none_or(|(old, new)| priority(old) <= priority(new))
    {
        waiting.append(fresh);
        return [sorted, 0];
    }

    // At equal distance, write the waiting mesh first so the fresh mesh is
    // popped first.
    let merged = waiting.len() + fresh.len();
    scratch.reserve(merged);
    let mut old = waiting.drain(..).peekable();
    let mut new = fresh.drain(..).peekable();
    while let (Some(a), Some(b)) = (old.peek(), new.peek()) {
        scratch.extend(if priority(a) <= priority(b) {
            old.next()
        } else {
            new.next()
        });
    }
    scratch.extend(old);
    scratch.extend(new);
    std::mem::swap(waiting, scratch);
    [sorted, merged]
}

/// Squared section-grid distance in `i128` so extreme camera coordinates cannot overflow.
pub(in crate::game::client_side) fn section_distance_sq(
    key: (i32, i32, i32),
    centre: (i32, i32),
    cam_y: i32,
) -> i128 {
    let dx = i128::from(key.0) - i128::from(centre.0);
    let dy = i128::from(key.1) - i128::from(cam_y);
    let dz = i128::from(key.2) - i128::from(centre.1);
    dx * dx + dy * dy + dz * dz
}

use sunrise_gameplay_common::game::light_emission::LightEmissionGameExt1 as _;

use sunrise_gameplay_common::game::native_hooks::SharedGameExt1 as _;

#[cfg(test)]
pub(in crate::game) mod tests {
    use super::{order_uploads, section_distance_sq};
    use std::cmp::Reverse;

    #[test]
    fn section_distance_sq_handles_extreme_coordinates_without_overflow() {
        let far = section_distance_sq(
            (i32::MAX, i32::MAX, i32::MAX),
            (i32::MIN, i32::MIN),
            i32::MIN,
        );
        let near = section_distance_sq(
            (i32::MIN, i32::MIN, i32::MIN),
            (i32::MIN, i32::MIN),
            i32::MIN,
        );
        assert_eq!(near, 0);
        assert!(far > near);
    }

    #[test]
    fn stationary_upload_budget_gives_fresh_meshes_priority_on_ties() {
        let mut waiting = vec![(20, "far"), (4, "waiting")];
        let mut fresh = vec![(1, "nearest"), (9, "middle"), (4, "fresh")];
        let mut scratch = Vec::new();
        let priority = |&(distance, _): &(i32, &str)| Reverse(distance);
        order_uploads(&mut waiting, &mut fresh, &mut scratch, true, priority);
        // Pin the stationary queue's fresh-first tie policy: two uploads leave
        // the waiting mesh first in line next frame. The old sort_unstable_by_key
        // did not guarantee this tie order.
        let selected: Vec<_> = std::iter::from_fn(|| waiting.pop().map(|(_, name)| name))
            .take(2)
            .collect();
        assert_eq!(selected, ["nearest", "fresh"]);
        assert_eq!(waiting.pop(), Some((4, "waiting")));
        assert_eq!(waiting.pop(), Some((9, "middle")));
        assert_eq!(waiting.pop(), Some((20, "far")));
    }
}
