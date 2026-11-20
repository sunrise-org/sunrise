//! Target-aware owned level attachments, using the ordinary map, codecs, index and transaction journal.
use super::Game;
use mod_api::{
    AttachmentAccess, AttachmentError, AttachmentForm, AttachmentResult, AttachmentStore,
    AttachmentValue, Compound, Holder, LevelRef,
};

impl Game<'_> {
    fn owned_level_attachment_map(
        &mut self,
        level: LevelRef,
        access: AttachmentAccess,
    ) -> AttachmentResult<&mut mod_api::Attachments> {
        if !self.is_logical_server() || !self.live_world_lifecycle_current(level) {
            return Err(AttachmentError::MissingHolder);
        }
        let owner = self
            .runtime_caller()
            .map_err(|_| AttachmentError::MissingHolder)?;
        if level.dimension() == self.dim.key() {
            self.journal_owned_level_attachments(level);
            return self.attachment_map(Holder::Level, access);
        }
        if self.levels.runtime.owner_of(level.dimension_id()) != owner {
            return Err(AttachmentError::MissingHolder);
        }
        self.native_level_world(level)
            .ok_or(AttachmentError::MissingHolder)?;
        self.journal_owned_level_attachments(level);
        let parked = self
            .levels
            .parked_mut_of(level.dimension_id())
            .ok_or(AttachmentError::MissingHolder)?;
        parked.world.attachment_syncs.insert(Holder::Level);
        Ok(&mut parked.world.attachments)
    }
    pub(super) fn level_attachment_holds(
        &mut self,
        level: LevelRef,
        namespace: &str,
        name: &str,
    ) -> Result<AttachmentResult<bool>, mod_api::LevelWriteError> {
        Ok((|| {
            mod_api::__private::validate_attachment_use(namespace, name, false)?;
            AttachmentStore::holds(
                self.owned_level_attachment_map(level, AttachmentAccess::Existing)?,
                Holder::Level,
                namespace,
                name,
            )
        })())
    }
    pub(super) fn level_attachment_decoded(
        &mut self,
        level: LevelRef,
        namespace: &str,
        name: &str,
    ) -> Result<AttachmentResult<Option<AttachmentValue>>, mod_api::LevelWriteError> {
        Ok((|| {
            mod_api::__private::validate_attachment_use(namespace, name, false)?;
            AttachmentStore::decoded(
                self.owned_level_attachment_map(level, AttachmentAccess::Existing)?,
                Holder::Level,
                namespace,
                name,
            )
        })())
    }
    pub(super) fn level_attachment_saved(
        &mut self,
        level: LevelRef,
        namespace: &str,
        name: &str,
    ) -> Result<AttachmentResult<Option<Compound>>, mod_api::LevelWriteError> {
        Ok((|| {
            mod_api::__private::validate_attachment_use(namespace, name, false)?;
            AttachmentStore::saved(
                self.owned_level_attachment_map(level, AttachmentAccess::Existing)?,
                Holder::Level,
                namespace,
                name,
            )
        })())
    }
    pub(super) fn level_attachment_store_value(
        &mut self,
        level: LevelRef,
        namespace: &str,
        name: &str,
        value: AttachmentValue,
        form: AttachmentForm,
    ) -> Result<AttachmentResult<()>, mod_api::LevelWriteError> {
        Ok((|| {
            mod_api::__private::validate_attachment_use(namespace, name, form.policy.sync)?;
            AttachmentStore::store(
                self.owned_level_attachment_map(level, AttachmentAccess::Store)?,
                Holder::Level,
                namespace,
                name,
                value,
                form,
            )
        })())
    }
    pub(super) fn level_attachment_discard(
        &mut self,
        level: LevelRef,
        namespace: &str,
        name: &str,
        sync: bool,
    ) -> Result<AttachmentResult<()>, mod_api::LevelWriteError> {
        Ok((|| {
            mod_api::__private::validate_attachment_use(namespace, name, sync)?;
            AttachmentStore::discard(
                self.owned_level_attachment_map(level, AttachmentAccess::Existing)?,
                Holder::Level,
                namespace,
                name,
                sync,
            )
        })())
    }
    pub(super) fn level_attachment_queue_sync(
        &mut self,
        level: LevelRef,
        namespace: &str,
        name: &str,
    ) -> Result<AttachmentResult<()>, mod_api::LevelWriteError> {
        Ok((|| {
            mod_api::__private::validate_attachment_use(namespace, name, true)?;
            AttachmentStore::queue_sync(
                self.owned_level_attachment_map(level, AttachmentAccess::Existing)?,
                Holder::Level,
                namespace,
                name,
            )
        })())
    }
}
