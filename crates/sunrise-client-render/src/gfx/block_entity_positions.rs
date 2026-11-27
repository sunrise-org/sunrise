//! Membership and revisions of block entities attached to section meshes.
use super::*;
impl Renderer {
    /// Update the metadata for the new mesh and release the old one.
    pub(super) fn before_mesh(&mut self, key: SectionKey, mesh: &MeshFacts) {
        self.occlusion.load_column(key.0, key.2);
        self.occlusion.set(key, mesh.visibility);
        let changed = self.block_entities.update(key, mesh.block_entities);
        if changed {
            self.block_entity_revision = next_block_entity_revision();
            self.block_entity_membership_revisions
                .insert(key, self.block_entity_revision);
        }
        if !mesh.block_entities.is_empty()
            || self.block_entity_membership_revisions.contains_key(&key)
        {
            // Packed light can change with an unchanged BE list. Keep a
            // tombstone for formerly occupied sections until unload so a
            // deleted last entity invalidates only its own cached section.
            self.block_entity_section_revisions
                .insert(key, next_block_entity_revision());
        }
        if mesh.translucent && !self.sections.contains(key) {
            // New translucent terrain, not an edit's remesh: large pages wait for it.
            self.shrink_gate.wait(Instant::now(), SHRINK_SETTLE);
        }
        if let std::collections::hash_map::Entry::Vacant(entry) = self.section_uploaded.entry(key) {
            let now = std::time::Instant::now();
            entry.insert(now);
            self.newest_upload = Some(now);
        }
        self.release_section(key);
    }

    /// A moving payload must disappear immediately, before the asynchronous remesh arrives.
    pub fn invalidate_block_entity_position(&mut self, pos: [i32; 3]) {
        let Some(sy) = self.layout.section_index(pos[1]) else {
            return;
        };
        let key = (pos[0] >> 4, sy as i32, pos[2] >> 4);
        self.block_entities.remove_position(key, pos);
        let revision = next_block_entity_revision();
        self.block_entity_revision = revision;
        self.block_entity_membership_revisions.insert(key, revision);
        self.block_entity_section_revisions.insert(key, revision);
    }
}
