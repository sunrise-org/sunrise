//! Target-aware level attachments using the ordinary typed storage and transaction journal.
use super::Game;
use mod_api::{
    AttachmentAccess, AttachmentBinding, AttachmentError, AttachmentHost, AttachmentLoan,
    AttachmentResult, AttachmentStore, AttachmentStoreKey, AttachmentStoreWrite, Attachments,
    Holder, LevelRef, LevelWriteError,
};

/// Lend one explicit level to the shared attachment store, without swapping the active level.
struct LevelAttachments<'a, 'g> {
    game: &'a mut Game<'g>,
    level: LevelRef,
}

impl AttachmentHost for LevelAttachments<'_, '_> {
    fn validate_attachment(&self, namespace: &str, name: &str, sync: bool) -> AttachmentResult<()> {
        mod_api::__private::validate_attachment_use(namespace, name, sync)
    }

    fn validate_attachment_key(&self, key: &AttachmentStoreKey<'_>) -> AttachmentResult<()> {
        mod_api::__private::validate_attachment_key(key)
    }

    fn read_attachments(&mut self, holder: Holder) -> AttachmentResult<Option<&Attachments>> {
        if holder != Holder::Level {
            return Err(AttachmentError::MissingHolder);
        }
        self.game
            .native_level_world(self.level)
            .map(|world| Some(&world.attachments))
            .ok_or(AttachmentError::MissingHolder)
    }

    fn attachments(
        &mut self,
        holder: Holder,
        _: AttachmentAccess,
    ) -> AttachmentResult<&mut Attachments> {
        if holder != Holder::Level {
            return Err(AttachmentError::MissingHolder);
        }
        self.game.journal_owned_level_attachments(self.level);
        if self.level.dimension() == self.game.dim.key() {
            return Ok(&mut self.game.world.attachments);
        }
        self.game
            .levels
            .parked_world_mut_of(self.level.dimension_id())
            .map(|world| &mut world.attachments)
            .ok_or(AttachmentError::MissingHolder)
    }

    fn attachment_changed(&mut self, holder: Holder, _: bool, sync: bool) {
        if sync && holder == Holder::Level {
            if self.level.dimension() == self.game.dim.key() {
                self.game.world.attachment_syncs.insert(holder);
            } else if let Some(world) = self
                .game
                .levels
                .parked_world_mut_of(self.level.dimension_id())
            {
                world.attachment_syncs.insert(holder);
            }
        }
    }
}

impl<'g> Game<'g> {
    fn owned_level_attachments(
        &mut self,
        level: LevelRef,
    ) -> AttachmentResult<LevelAttachments<'_, 'g>> {
        if !self.is_logical_server() || !self.live_world_lifecycle_current(level) {
            return Err(AttachmentError::MissingHolder);
        }
        let owner = self
            .runtime_caller()
            .map_err(|_| AttachmentError::MissingHolder)?;
        if level.dimension() != self.dim.key()
            && self.levels.runtime.owner_of(level.dimension_id()) != owner
        {
            return Err(AttachmentError::MissingHolder);
        }
        Ok(LevelAttachments { game: self, level })
    }

    pub(super) fn level_attachment_holds(
        &mut self,
        level: LevelRef,
        key: &AttachmentStoreKey<'_>,
    ) -> Result<AttachmentResult<bool>, LevelWriteError> {
        Ok(self
            .owned_level_attachments(level)
            .and_then(|mut store| store.holds(Holder::Level, key)))
    }

    pub(super) fn level_attachment_lend_decoded(
        &mut self,
        level: LevelRef,
        key: &AttachmentStoreKey<'_>,
        read: AttachmentLoan<'_>,
    ) -> Result<AttachmentResult<()>, LevelWriteError> {
        Ok(self
            .owned_level_attachments(level)
            .and_then(|mut store| store.lend_decoded(Holder::Level, key, read)))
    }

    pub(super) fn level_attachment_binding(
        &mut self,
        level: LevelRef,
        key: &AttachmentStoreKey<'_>,
    ) -> Result<AttachmentResult<Option<AttachmentBinding>>, LevelWriteError> {
        Ok(self
            .owned_level_attachments(level)
            .and_then(|mut store| store.binding(Holder::Level, key)))
    }

    pub(super) fn level_attachment_store(
        &mut self,
        level: LevelRef,
        write: AttachmentStoreWrite<'_>,
    ) -> Result<AttachmentResult<()>, LevelWriteError> {
        Ok(self
            .owned_level_attachments(level)
            .and_then(|mut store| store.store(Holder::Level, write)))
    }

    pub(super) fn level_attachment_discard(
        &mut self,
        level: LevelRef,
        key: &AttachmentStoreKey<'_>,
    ) -> Result<AttachmentResult<()>, LevelWriteError> {
        Ok(self
            .owned_level_attachments(level)
            .and_then(|mut store| store.discard(Holder::Level, key)))
    }

    pub(super) fn level_attachment_queue_sync(
        &mut self,
        level: LevelRef,
        key: &AttachmentStoreKey<'_>,
    ) -> Result<AttachmentResult<()>, LevelWriteError> {
        Ok(self
            .owned_level_attachments(level)
            .and_then(|mut store| store.queue_sync(Holder::Level, key)))
    }
}
