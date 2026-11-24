//! Target-aware primitives behind the borrowed LevelRef attachment store.
#![allow(unused_variables)]
use crate::*;

/// Generated backend for typed level attachment storage; use `LevelRef::attachments` with a typed key.
// hidden: a generated backend reserved for the typed attachment adapter.
#[doc(hidden)]
#[sunrise_mod_api_macros::host_area]
pub trait LevelAttachmentsBackendMut {
    /// Side: server.
    /// Whether an existing value is present on this level's holder.
    #[manual_handle]
    fn level_attachment_holds(
        &mut self,
        level: LevelRef,
        key: &AttachmentStoreKey<'_>,
    ) -> Result<AttachmentResult<bool>, LevelWriteError>;
    /// Side: server.
    /// Lend the existing decoded value from this level's real holder.
    #[manual_handle]
    fn level_attachment_lend_decoded(
        &mut self,
        level: LevelRef,
        key: &AttachmentStoreKey<'_>,
        read: AttachmentLoan<'_>,
    ) -> Result<AttachmentResult<()>, LevelWriteError>;
    /// Side: server.
    /// Return the unread persisted binding from this level's real holder.
    #[manual_handle]
    fn level_attachment_binding(
        &mut self,
        level: LevelRef,
        key: &AttachmentStoreKey<'_>,
    ) -> Result<AttachmentResult<Option<AttachmentBinding>>, LevelWriteError>;
    /// Side: server.
    /// Store through the ordinary attachment codec/index and Host transaction journal.
    #[manual_handle]
    fn level_attachment_store(
        &mut self,
        level: LevelRef,
        write: AttachmentStoreWrite<'_>,
    ) -> Result<AttachmentResult<()>, LevelWriteError>;
    /// Side: server.
    /// Discard through the same ordinary target-aware attachment store.
    #[manual_handle]
    fn level_attachment_discard(
        &mut self,
        level: LevelRef,
        key: &AttachmentStoreKey<'_>,
    ) -> Result<AttachmentResult<()>, LevelWriteError>;
    /// Side: server.
    /// Queue this level's ordinary attachment synchronization.
    #[manual_handle]
    fn level_attachment_queue_sync(
        &mut self,
        level: LevelRef,
        key: &AttachmentStoreKey<'_>,
    ) -> Result<AttachmentResult<()>, LevelWriteError>;
}
