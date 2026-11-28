use super::*;

/// Loads saved chunks before falling back to generation.
pub struct Persistent<G> {
    pub inner: Arc<G>,
    pub save: Arc<SaveDir>,
    pub registry: Arc<crate::assets::Assets>,
    pub biomes: Arc<Biomes>,
}

impl<G: ChunkGenerator> ChunkGenerator for Persistent<G> {
    type Worker = G::Worker;
    fn spawn_height(&self) -> i32 {
        self.inner.spawn_height()
    }
    fn worker(&self) -> G::Worker {
        let save = self.save.clone();
        self.inner.set_saved_starts(Arc::new(move |pos| {
            let root = save.raw_chunk(pos.x, pos.z)?;
            match root.tag.get("structures")?.get("starts")? {
                Tag::Compound(starts) => Some(starts.clone()),
                _ => None,
            }
        }));
        self.inner.worker()
    }
    fn load(&self, x: i32, z: i32) -> Option<Chunk> {
        self.save
            .tables
            .get_or_init(|| Arc::new(SaveTables::new(&self.registry)));
        let mut chunk = self
            .save
            .try_load(x, z, self.registry.registry, &self.biomes)
            .ok()
            .flatten()?;
        for s in &mut chunk.sections {
            Arc::make_mut(s).compact();
        }
        chunk.disk_entities = self.save.load_entities(x, z).unwrap_or_default();
        Some(chunk)
    }
    fn accept_loaded_chunk(&self, chunk: &mut Chunk) {
        self.save
            .load_disk_attachments(chunk, mod_api::ChunkDiskKind::Full);
    }
    fn prepare_loaded(&self, worker: &mut G::Worker, chunk: &Chunk) {
        self.inner.prepare_loaded(worker, chunk);
    }
    fn all_idle(&self) {
        self.inner.all_idle();
    }
    fn idle(&self, worker: &mut G::Worker) {
        self.inner.idle(worker);
    }
    fn proto(
        &self,
        worker: &mut G::Worker,
        x: i32,
        z: i32,
    ) -> std::result::Result<sunrise_level::chunk_streamer::Proto, String> {
        let (mut proto, saved) = self.worker_proto(worker, x, z)?;
        if let Some(saved) = saved {
            self.accept_loaded_proto(&mut proto, *saved);
        }
        Ok(proto)
    }
    fn worker_proto(
        &self,
        worker: &mut G::Worker,
        x: i32,
        z: i32,
    ) -> std::result::Result<
        (
            sunrise_level::chunk_streamer::Proto,
            Option<Box<SavedProto>>,
        ),
        String,
    > {
        let mut proto = self.inner.proto(worker, x, z)?;
        let Some(saved) = self
            .save
            .decode_saved_proto(x, z, self.registry.registry, &self.biomes)
        else {
            return Ok((proto, None));
        };
        if self.save.has_chunk_disk_hook(mod_api::ChunkDiskPhase::Load) {
            Ok((proto, Some(Box::new(saved))))
        } else {
            self.inner.restore_proto(&mut proto, saved);
            Ok((proto, None))
        }
    }
    fn accept_loaded_proto(
        &self,
        proto: &mut sunrise_level::chunk_streamer::Proto,
        mut saved: SavedProto,
    ) -> bool {
        self.save
            .load_disk_attachments(&mut saved.column, mod_api::ChunkDiskKind::Proto);
        self.inner.restore_proto(proto, saved);
        self.inner.is_decorated(proto)
    }
    fn is_decorated(&self, proto: &sunrise_level::chunk_streamer::Proto) -> bool {
        self.inner.is_decorated(proto)
    }
    fn light_view(&self, proto: &sunrise_level::chunk_streamer::Proto) -> Option<Chunk> {
        self.inner.light_view(proto)
    }
    fn non_air_sections(
        &self,
        proto: &sunrise_level::chunk_streamer::Proto,
    ) -> Option<crate::section_mask::SectionMask> {
        self.inner.non_air_sections(proto)
    }
    fn cache_census(&self) -> Vec<(&'static str, usize, Option<usize>)> {
        self.inner.cache_census()
    }
    fn proto_heap_bytes(
        &self,
        proto: &sunrise_level::chunk_streamer::Proto,
    ) -> ([usize; 6], [usize; 16]) {
        self.inner.proto_heap_bytes(proto)
    }
    fn save_protos_on_owner(&self) -> bool {
        self.save.has_chunk_disk_hook(mod_api::ChunkDiskPhase::Save)
    }
    fn save_protos(&self, protos: &[&sunrise_level::chunk_streamer::Proto]) {
        self.save
            .tables
            .get_or_init(|| Arc::new(SaveTables::new(&self.registry)));
        let saved: Vec<SavedProto> = protos
            .iter()
            .filter_map(|p| self.inner.saved_proto(p))
            .collect();
        if let Err(e) = self
            .save
            .save_protos(saved, self.registry.registry, &self.biomes)
        {
            log::warn!("Sunrise: failed to save proto chunks: {e}");
        }
    }
    fn save_finished(&self, mut chunk: Chunk) {
        self.save
            .tables
            .get_or_init(|| Arc::new(SaveTables::new(&self.registry)));
        let key = (chunk.x, chunk.z);
        let entities = std::mem::take(&mut chunk.entities);
        chunk.disk_entities.clear();
        // This chunk did not enter the level. Add only the new generation
        // records; its separate entity region may predate the terrain.
        let saved = self
            .save
            .save_later(&[Arc::new(chunk)], self.registry.registry, &self.biomes)
            .and_then(|_| {
                if !entities.is_empty()
                    && let Ok(mut stored) = self.save.load_entities(key.0, key.1)
                {
                    stored.extend(entities);
                    self.save.save_entities_later(vec![(key, stored)])?;
                }
                Ok(())
            });
        if let Err(e) = saved {
            log::warn!("Sunrise: failed to save chunk {} {}: {e}", key.0, key.1);
        }
    }
    fn decorates(&self) -> bool {
        self.inner.decorates()
    }
    fn decorates_beside_final(&self) -> bool {
        self.inner.decorates_beside_final()
    }
    fn proto_beside_final(
        &self,
        worker: &mut G::Worker,
        snapshot: &Chunk,
    ) -> Option<sunrise_level::chunk_streamer::Proto> {
        self.inner.proto_beside_final(worker, snapshot)
    }
    fn final_writes(
        &self,
        snapshot: &Chunk,
        proto: &sunrise_level::chunk_streamer::Proto,
    ) -> Vec<([i32; 3], u32)> {
        self.inner.final_writes(snapshot, proto)
    }
    fn final_block_entities(
        &self,
        entries: &[(&sunrise_level::chunk_streamer::Proto, &[([i32; 3], u32)])],
    ) -> Vec<((i32, i32, i32), crate::chunk::block_entity::BlockEntity)> {
        self.inner.final_block_entities(entries)
    }
    fn decorate(
        &self,
        worker: &mut G::Worker,
        region: &mut [sunrise_level::chunk_streamer::Proto; 9],
    ) {
        self.inner.decorate(worker, region);
    }
    fn decorate_with_outer(
        &self,
        worker: &mut G::Worker,
        region: &mut [sunrise_level::chunk_streamer::Proto; 9],
        outer: &[Arc<Chunk>],
    ) {
        self.inner.decorate_with_outer(worker, region, outer);
    }
    fn finish(&self, proto: sunrise_level::chunk_streamer::Proto) -> Chunk {
        let mut chunk = self.inner.finish(proto);
        // EntityStorage is independent of terrain: a newly generated column
        // can already have a saved entity section.
        chunk.disk_entities = self
            .save
            .load_entities(chunk.x, chunk.z)
            .unwrap_or_default();
        chunk
    }
}
