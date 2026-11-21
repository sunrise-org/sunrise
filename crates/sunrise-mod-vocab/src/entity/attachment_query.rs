//! Owned snapshot data of attachment queries.
use crate::*;
use sunrise_mod_runtime::attachments::{self as runtime, index};

/// A SNAPSHOT of membership and value bindings in the host's current dimension.
/// Later changes require a fresh query. This is a residency view: removed but
/// unswept entities may appear, and stale writes fail. Acquisition neither clones values nor
/// decodes saved payloads; iteration decodes lazily. Keep the batch in process; use the
/// attachment type's codec for saving or syncing individual values.
///
/// ```compile_fail
/// use mod_api::{AttachmentBatch, wire::Writer};
/// fn encode_batch(batch: &AttachmentBatch) {
///     Writer::new().put(batch); // snapshots have no Wire encoding
/// }
/// ```
// no wire: retains all captured bindings, including values that have no persistence codec.
#[derive(Clone)]
#[non_exhaustive]
pub struct AttachmentBatch(runtime::AttachmentBatch<AttachmentHolder>);
impl From<runtime::AttachmentBatch<AttachmentHolder>> for AttachmentBatch {
    fn from(batch: runtime::AttachmentBatch<AttachmentHolder>) -> Self {
        Self(batch)
    }
}
impl AttachmentBatch {
    /// Bind a type without eagerly decoding or cloning any value.
    /// This view borrows only the key, never the host.
    pub fn query<T: Send + Sync + 'static>(
        self,
        key: &AttachmentType<T>,
    ) -> Result<AttachmentQuery<'_, T>, AttachmentError> {
        crate::parts::validate_attachment_use(key.namespace, key.name, key.sync)?;
        self.0.query(key)
    }
}
/// A typed snapshot whose iterator borrows values from its captured bindings.
/// `get(holder)` returns an owned clone with an expected O(1) probe before decode.
/// Missing or not yet synced holders return `Ok(None)`; no default is created.
/// `None` does not mean the holder is alive: absent and not-live holders alike
/// return `Ok(None)` when absent from the snapshot, without a host liveness check.
/// Only scoped residency probes report `StaleHolder`.
// no wire: binds a borrowed key to an in-process snapshot.
pub type AttachmentQuery<'key, T> = runtime::AttachmentQuery<'key, AttachmentHolder, T>;

/// A scoped residency identity issued from a captured row, not an access authorization
/// token. It cannot be reconstructed from bytes. Mods typically use the row's
/// `set` or `remove` method rather than passing the token to the host themselves.
///
/// ```compile_fail
/// use mod_api::{AttachmentWriteToken, wire::Reader};
/// let _ = Reader::decode::<AttachmentWriteToken>(&[]);
/// ```
// no wire: names a residency issued by the live attachment index.
#[derive(Clone)]
#[non_exhaustive]
pub struct AttachmentWriteToken(index::AttachmentOwner<AttachmentHolder>);
impl From<index::AttachmentOwner<AttachmentHolder>> for AttachmentWriteToken {
    fn from(token: index::AttachmentOwner<AttachmentHolder>) -> Self {
        Self(token)
    }
}
impl AttachmentWriteToken {
    /// Check the current dimension's residency before the ordinary write path.
    // hidden: the owning engine validates tokens; mods use the row's set/remove methods.
    #[doc(hidden)]
    pub fn validate(
        &self,
        index: &index::AttachmentIndex<AttachmentHolder>,
    ) -> Result<(), AttachmentRowError> {
        index.validate_owner(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    static KEY: AttachmentType<u32> = AttachmentType {
        namespace: "test",
        name: "value",
        default: || 0,
        codec: None,
        copy_on_death: false,
        sync: false,
    };

    #[test]
    fn write_tokens_go_stale_with_their_residency() {
        let holder = AttachmentHolder::Entity(EntityRef::new(WorldEpoch::new(1), 42));
        let index = Arc::new(std::sync::Mutex::new(index::AttachmentIndex::default()));
        let mut values = runtime::Attachments::default();
        KEY.set(&mut values, runtime::Holder::Level, 7).unwrap();
        values.bind_index(&index, holder);
        let batch = AttachmentBatch::from(runtime::AttachmentBatch::new(
            index.lock().unwrap().snapshot_named("test", "value"),
        ));
        let query = batch.query(&KEY).unwrap();
        let row = query.iter().next().unwrap().unwrap();
        assert_eq!(row.holder, holder);
        assert_eq!(*row.value, 7);
        let token = AttachmentWriteToken::from(row.write_token());
        assert_eq!(token.validate(&index.lock().unwrap()), Ok(()));
        assert_eq!(
            token.validate(&index::AttachmentIndex::default()),
            Err(AttachmentRowError::StaleHolder)
        );
        index.lock().unwrap().unload(holder).into_result();
        assert_eq!(
            token.validate(&index.lock().unwrap()),
            Err(AttachmentRowError::StaleHolder)
        );
        let mut again = runtime::Attachments::default();
        KEY.set(&mut again, runtime::Holder::Level, 7).unwrap();
        again.bind_index(&index, holder);
        assert_eq!(
            token.validate(&index.lock().unwrap()),
            Err(AttachmentRowError::StaleHolder)
        );
        assert_eq!(*row.value, 7);
    }
}
