//! Metadata before-images for the existing Host transaction; no payload authority.
use super::*;

#[derive(Clone, Copy)]
pub(super) struct RetiredBinding {
    pub(super) generation: u64,
    pub(super) members: u32,
    pub(super) source_revision: u64,
}
#[derive(Clone)]
pub(in crate::game) struct KeptProjectionBinding {
    source: LevelRef,
    entry: Option<Entry>,
    retired: Option<RetiredBinding>,
    dirty: bool,
    held: bool,
}
impl Game<'_> {
    pub(in crate::game) fn capture_projection_binding(
        &self,
        source: LevelRef,
    ) -> KeptProjectionBinding {
        KeptProjectionBinding {
            source,
            entry: self
                .live_worlds
                .borrow()
                .entries
                .values()
                .find(|e| e.handle.level() == source)
                .cloned(),
            retired: self.live_world_durability.retired.get(&source).copied(),
            dirty: self.live_world_durability.dirty.contains(&source),
            held: self.live_world_durability.members.contains_key(&source),
        }
    }
    pub(in crate::game) fn restore_projection_binding(&mut self, kept: KeptProjectionBinding) {
        let source = kept.source;
        let table = self.live_worlds.get_mut();
        table
            .entries
            .retain(|_, entry| entry.handle.level() != source);
        if let Some(entry) = kept.entry {
            table.entries.insert(entry.handle.generation(), entry);
        }
        match kept.retired {
            Some(retired) => {
                self.live_world_durability.retired.insert(source, retired);
            }
            None => {
                self.live_world_durability.retired.remove(&source);
            }
        }
        if kept.dirty {
            self.live_world_durability.dirty.insert(source);
        } else {
            self.live_world_durability.dirty.remove(&source);
        }
        if !kept.held && !self.player_autosaves.common_required() {
            self.live_world_durability.members.remove(&source);
        }
        self.refresh_live_world_retention(source);
    }
    pub(super) fn record_projection_retirement(
        &mut self,
        handle: LiveWorldRef,
        members: u32,
    ) -> Result<(), Error> {
        let source = handle.level();
        let source_revision = self
            .native_level_world(source)
            .ok_or(Error::Stale)?
            .revision;
        self.dirty_live_world_level(source)?;
        self.live_world_durability.retired.insert(
            source,
            RetiredBinding {
                generation: handle.generation(),
                members,
                source_revision,
            },
        );
        Ok(())
    }
}
