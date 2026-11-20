//! Target-aware primitives behind the borrowed LevelRef attachment store.
#![allow(unused_variables)]
use crate::*;
/// Generated backend for typed level attachment storage; use `LevelRef::attachments` with a typed key.
// hidden: generated handle backend; mods use the natural receiver methods.
#[doc(hidden)]
#[sunrise_mod_api_macros::host_area]
pub trait LevelAttachmentsBackendMut {
    /// Side: server.
    /// Whether an existing value is present on this level's holder. No other holder is addressed.
    #[access(WorldOnly)]
    fn level_attachment_holds(
        &mut self,
        level: LevelRef,
        namespace: &str,
        name: &str,
    ) -> Result<Result<bool, AttachmentError>, LevelWriteError>;
    /// Side: server.
    /// Existing decoded value from this level's real holder.
    #[access(WorldOnly)]
    fn level_attachment_decoded(
        &mut self,
        level: LevelRef,
        namespace: &str,
        name: &str,
    ) -> Result<Result<Option<AttachmentValue>, AttachmentError>, LevelWriteError>;
    /// Side: server.
    /// Existing persisted compound from this level's real holder.
    #[access(WorldOnly)]
    fn level_attachment_saved(
        &mut self,
        level: LevelRef,
        namespace: &str,
        name: &str,
    ) -> Result<Result<Option<Compound>, AttachmentError>, LevelWriteError>;
    /// Side: server.
    /// Store through the ordinary attachment codec/index and Host transaction journal.
    #[access(WorldOnly)]
    fn level_attachment_store_value(
        &mut self,
        level: LevelRef,
        namespace: &str,
        name: &str,
        value: AttachmentValue,
        form: AttachmentForm,
    ) -> Result<Result<(), AttachmentError>, LevelWriteError>;
    /// Side: server.
    /// Discard through the same ordinary target-aware attachment store.
    #[access(WorldOnly)]
    fn level_attachment_discard(
        &mut self,
        level: LevelRef,
        namespace: &str,
        name: &str,
        sync: bool,
    ) -> Result<Result<(), AttachmentError>, LevelWriteError>;
    /// Side: server.
    /// Queue this level's ordinary attachment synchronization.
    #[access(WorldOnly)]
    fn level_attachment_queue_sync(
        &mut self,
        level: LevelRef,
        namespace: &str,
        name: &str,
    ) -> Result<Result<(), AttachmentError>, LevelWriteError>;
}
