//! A borrowed view of an existing level's attachment holder, not another value store.
use crate::*;

/// A typed attachment store bound to one explicit server level.
///
/// Use existing `AttachmentType` get/set methods with `Holder::Level`. Other holder kinds
/// refuse. Current-level access keeps the ordinary rules; a parked target must be an open
/// runtime level owned by the calling enabled mod. Reads/decodes/defaults and writes share
/// the existing Host transaction and exact target-level undo, without switching context.
// no wire: caller-side borrowed adapter; its generated backend primitives cross the same native/wasm boundary.
pub struct LevelAttachments<'a> {
    level: LevelRef,
    host: &'a mut dyn ServerHost,
}
impl LevelRef {
    /// Borrow this actual level's attachment holder. Does not load or switch levels.
    /// A stale/closing, client, foreign parked or unavailable target refuses on access.
    /// Pair an accepted pose and its opaque caller metadata in one `host.transaction`:
    /// publish, encode the returned canonical sample, store it, then commit.
    pub fn attachments<'a>(&self, host: &'a mut dyn ServerHost) -> LevelAttachments<'a> {
        LevelAttachments { level: *self, host }
    }
}
fn level_holder(holder: Holder) -> AttachmentResult<()> {
    if holder == Holder::Level {
        Ok(())
    } else {
        Err(AttachmentError::MissingHolder)
    }
}
impl AttachmentStore for LevelAttachments<'_> {
    fn validate_attachment(
        &mut self,
        namespace: &str,
        name: &str,
        sync: bool,
    ) -> AttachmentResult<()> {
        crate::__private::validate_attachment_use(namespace, name, sync)
    }
    fn holds(&mut self, holder: Holder, namespace: &str, name: &str) -> AttachmentResult<bool> {
        level_holder(holder)?;
        self.level
            .attachment_holds(self.host, namespace, name)
            .map_err(|_| AttachmentError::MissingHolder)?
    }
    fn decoded(
        &mut self,
        holder: Holder,
        namespace: &str,
        name: &str,
    ) -> AttachmentResult<Option<AttachmentValue>> {
        level_holder(holder)?;
        self.level
            .attachment_decoded(self.host, namespace, name)
            .map_err(|_| AttachmentError::MissingHolder)?
    }
    fn saved(
        &mut self,
        holder: Holder,
        namespace: &str,
        name: &str,
    ) -> AttachmentResult<Option<Compound>> {
        level_holder(holder)?;
        self.level
            .attachment_saved(self.host, namespace, name)
            .map_err(|_| AttachmentError::MissingHolder)?
    }
    fn store(
        &mut self,
        holder: Holder,
        namespace: &str,
        name: &str,
        value: AttachmentValue,
        form: AttachmentForm,
    ) -> AttachmentResult<()> {
        level_holder(holder)?;
        self.level
            .attachment_store_value(self.host, namespace, name, value, form)
            .map_err(|_| AttachmentError::MissingHolder)?
    }
    fn discard(
        &mut self,
        holder: Holder,
        namespace: &str,
        name: &str,
        sync: bool,
    ) -> AttachmentResult<()> {
        level_holder(holder)?;
        self.level
            .attachment_discard(self.host, namespace, name, sync)
            .map_err(|_| AttachmentError::MissingHolder)?
    }
    fn queue_sync(&mut self, holder: Holder, namespace: &str, name: &str) -> AttachmentResult<()> {
        level_holder(holder)?;
        self.level
            .attachment_queue_sync(self.host, namespace, name)
            .map_err(|_| AttachmentError::MissingHolder)?
    }
}
