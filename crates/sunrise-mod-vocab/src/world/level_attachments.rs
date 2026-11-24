//! A borrowed view of an existing level's attachment holder, not another value store.
use crate::*;

/// A typed attachment store bound to one explicit server level.
///
/// Use existing `AttachmentType` get/set methods with `Holder::Level`. Other holder kinds
/// refuse. Current-level access keeps the ordinary rules; a parked target must be an open
/// runtime level owned by the calling enabled mod. Reads/decodes/defaults and writes share
/// the existing Host transaction and exact target-level undo, without switching context.
// no wire: caller-side borrowed adapter; each operation borrows the server backend.
#[non_exhaustive]
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
impl LevelAttachments<'_> {
    fn access<T>(
        &mut self,
        holder: Holder,
        f: impl FnOnce(&mut dyn BackendMut, LevelRef) -> Result<AttachmentResult<T>, LevelWriteError>,
    ) -> AttachmentResult<T> {
        if holder != Holder::Level {
            return Err(AttachmentError::MissingHolder);
        }
        let backend = backend_mut_of(self.host).ok_or(AttachmentError::MissingHolder)?;
        f(backend, self.level).map_err(|_| AttachmentError::MissingHolder)?
    }
}

impl AttachmentStore for LevelAttachments<'_> {
    fn holds(&mut self, holder: Holder, key: &AttachmentStoreKey<'_>) -> AttachmentResult<bool> {
        self.access(holder, |backend, level| {
            backend.level_attachment_holds(level, key)
        })
    }

    fn lend_decoded(
        &mut self,
        holder: Holder,
        key: &AttachmentStoreKey<'_>,
        read: AttachmentLoan<'_>,
    ) -> AttachmentResult<()> {
        self.access(holder, |backend, level| {
            backend.level_attachment_lend_decoded(level, key, read)
        })
    }

    fn binding(
        &mut self,
        holder: Holder,
        key: &AttachmentStoreKey<'_>,
    ) -> AttachmentResult<Option<AttachmentBinding>> {
        self.access(holder, |backend, level| {
            backend.level_attachment_binding(level, key)
        })
    }

    fn store(&mut self, holder: Holder, write: AttachmentStoreWrite<'_>) -> AttachmentResult<()> {
        self.access(holder, |backend, level| {
            backend.level_attachment_store(level, write)
        })
    }

    fn discard(&mut self, holder: Holder, key: &AttachmentStoreKey<'_>) -> AttachmentResult<()> {
        self.access(holder, |backend, level| {
            backend.level_attachment_discard(level, key)
        })
    }

    fn queue_sync(&mut self, holder: Holder, key: &AttachmentStoreKey<'_>) -> AttachmentResult<()> {
        self.access(holder, |backend, level| {
            backend.level_attachment_queue_sync(level, key)
        })
    }
}
