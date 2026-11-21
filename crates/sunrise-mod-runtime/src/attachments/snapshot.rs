//! Read-only generations of a holder's synced bindings, detached from the holder itself.
use super::{
    AttachmentEntry, AttachmentError, AttachmentKeyView, AttachmentType, Attachments, Compound,
    Entry, Tag, index::AttachmentKey,
};
use std::{fmt, sync::Arc};

type Binding = (AttachmentKey, Arc<AttachmentEntry>);

/// A whole generation in attachment key order. Clones share the same backing slice.
// no wire: in-process shared bindings; attachment sync owns their network codec.
#[derive(Clone)]
pub struct SyncedAttachmentSnapshot(Arc<[Binding]>);

impl SyncedAttachmentSnapshot {
    /// Charge the slice, keys and retained codec input of the whole generation before
    /// publishing it. `compound_heap_bytes` measures a compound's owned allocations; cached
    /// typed results are native mod values, not part of the codec input budget.
    pub fn capture(
        holder: &Attachments,
        limit: usize,
        compound_heap_bytes: impl Fn(&Compound) -> usize,
    ) -> Option<Self> {
        let arc_header = 2 * size_of::<usize>();
        let entry_overhead =
            arc_header + size_of::<AttachmentEntry>() + align_of::<AttachmentEntry>() - 1;
        let mut left = limit.checked_sub(arc_header + align_of::<Binding>() - 1)?;
        let mut entries = Vec::new();
        for (key, binding) in &holder.entries {
            if !binding.is_synced() {
                continue;
            }
            let Some(value) = binding.synced_value() else {
                continue;
            };
            let (namespace, name) = key;
            let key_bytes = (namespace.0.len() + name.0.len()) * size_of::<u16>();
            left = left.checked_sub(size_of::<Binding>() + entry_overhead + key_bytes)?;
            let frozen = match &binding.state {
                Entry::Raw { saved, .. } => {
                    left = left.checked_sub(compound_heap_bytes(saved))?;
                    Arc::clone(binding)
                }
                Entry::Decoded(_) => {
                    // A live read can promote the replica to a typed binding. Freeze its codec
                    // input, not that live Rust value: render reads must still use the key's codec.
                    let mut saved =
                        Compound::from([("value".into(), Tag::Compound(Compound::new()))]);
                    left = left.checked_sub(compound_heap_bytes(&saved))?;
                    left = left.checked_sub(compound_heap_bytes(value))?;
                    saved.insert("value".into(), Tag::Compound(value.clone()));
                    Arc::new(AttachmentEntry::raw(saved))
                }
            };
            entries.push((key.clone(), frozen));
        }
        Some(Self(entries.into()))
    }

    /// Read a captured binding, without inserting a default or consulting its old holder.
    pub fn get<T: Send + Sync + 'static>(
        &self,
        key: &AttachmentType<T>,
    ) -> Result<Option<&T>, AttachmentError> {
        let lookup = (key.namespace, key.name);
        self.0
            .binary_search_by(|(key, _)| {
                (key as &dyn AttachmentKeyView).cmp(&lookup as &dyn AttachmentKeyView)
            })
            .ok()
            .map(|i| self.0[i].1.read(key))
            .transpose()
    }
}

impl fmt::Debug for SyncedAttachmentSnapshot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SyncedAttachmentSnapshot")
            .field("fields", &self.0.len())
            .finish()
    }
}

impl PartialEq for SyncedAttachmentSnapshot {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
            || (self.0.len() == other.0.len()
                && self.0.iter().zip(other.0.iter()).all(
                    |((key, value), (other_key, other_value))| {
                        key == other_key
                            && (Arc::ptr_eq(value, other_value)
                                || value.synced_value() == other_value.synced_value())
                    },
                ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::attachments::{AttachmentCodec, AttachmentSync, Holder};

    static FUEL: AttachmentType<i32> = AttachmentType {
        namespace: "test",
        name: "fuel",
        default: || 99,
        codec: Some(AttachmentCodec {
            encode: |n| Ok([("n".into(), Tag::Int(*n))].into()),
            decode: |tag| match tag.get(&"n".into()) {
                Some(Tag::Int(n)) => Ok(*n),
                _ => Err(AttachmentError::Decode),
            },
        }),
        sync: true,
        copy_on_death: false,
    };

    fn sync(holder: &mut Attachments, key: &str, value: Option<Compound>) {
        holder.apply_sync(AttachmentSync {
            namespace: "test".into(),
            name: key.into(),
            value,
        });
    }

    #[test]
    fn a_retained_generation_reads_the_old_binding_after_replacement_and_removal() {
        let mut holder = Attachments::default();
        let value = |n| Some([("n".into(), Tag::Int(n))].into());
        sync(&mut holder, "fuel", value(7));
        let capture = |holder: &Attachments| {
            SyncedAttachmentSnapshot::capture(holder, usize::MAX, Compound::storage_bytes).unwrap()
        };
        let first = capture(&holder);
        assert!(Arc::ptr_eq(
            &first.0[0].1,
            holder.entry("test", "fuel").unwrap().unwrap()
        ));
        let frame = first.clone();
        assert!(Arc::ptr_eq(&frame.0, &first.0));

        sync(&mut holder, "fuel", value(11));
        let next = capture(&holder);
        sync(&mut holder, "fuel", None);
        assert_eq!(capture(&holder).get(&FUEL), Ok(None));
        // Read cold, after the live holder has lost both values.
        assert_eq!(frame.get(&FUEL), Ok(Some(&7)));
        assert_eq!(next.get(&FUEL), Ok(Some(&11)));
    }

    #[test]
    fn value_equality_changes_only_with_keys_or_canonical_nbt() {
        let mut a = Attachments::default();
        let mut b = Attachments::default();
        let value = |reverse: bool, bits| {
            // Aa and BB collide. The compounds need not traverse them in the same order.
            let mut tags = [
                ("Aa", Tag::Float(bits)),
                ("BB", Tag::Int(2)),
                ("n", Tag::Int(7)),
            ];
            if reverse {
                tags.swap(0, 1);
            }
            Some(tags.map(|(k, v)| (k.into(), v)).into())
        };
        sync(&mut a, "last", Some(Compound::new()));
        sync(&mut a, "fuel", value(false, 0x7fc0_0042));
        sync(&mut b, "fuel", value(true, 0x7fc0_0042));
        sync(&mut b, "last", Some(Compound::new()));
        let capture = |holder: &Attachments| {
            SyncedAttachmentSnapshot::capture(holder, usize::MAX, Compound::storage_bytes).unwrap()
        };
        let a = capture(&a);
        assert_eq!(a.get(&FUEL), Ok(Some(&7)));
        assert_eq!(
            a,
            capture(&b),
            "neither iteration order nor warm reads count"
        );
        sync(&mut b, "fuel", value(true, 0x7fc0_0043));
        assert_ne!(a, capture(&b), "even a changed floating-point bit counts");
        sync(&mut b, "fuel", value(true, 0x7fc0_0042));
        sync(&mut b, "last", None);
        sync(&mut b, "next", Some(Compound::new()));
        assert_ne!(
            a,
            capture(&b),
            "equal values under different keys do not count"
        );
    }

    #[test]
    fn a_budget_refuses_the_whole_generation_and_recovers_after_removal() {
        let mut holder = Attachments::default();
        // Each codec input fits individually; the pair cannot fit whichever key comes first.
        let capture =
            |holder: &Attachments| SyncedAttachmentSnapshot::capture(holder, 8_192, |_| 4_096);
        sync(
            &mut holder,
            "fuel",
            Some([("n".into(), Tag::Int(7))].into()),
        );
        let first = capture(&holder).unwrap();
        sync(&mut holder, "last", Some(Compound::new()));
        assert!(capture(&holder).is_none());
        sync(&mut holder, "fuel", None);
        assert_eq!(capture(&holder).unwrap().get(&FUEL), Ok(None));
        assert_eq!(first.get(&FUEL), Ok(Some(&7)));
    }

    #[test]
    fn snapshots_of_live_decoded_bindings_still_read_through_the_codec() {
        static CODEC: AttachmentType<i32> = AttachmentType {
            namespace: "test",
            name: "fuel",
            default: || 0,
            codec: Some(AttachmentCodec {
                encode: |n| Ok([("n".into(), Tag::Int(n / 10))].into()),
                decode: |c| match c.get(&"n".into()) {
                    Some(Tag::Int(n)) => Ok(*n * 10),
                    _ => Err(AttachmentError::Decode),
                },
            }),
            sync: true,
            copy_on_death: false,
        };
        let mut holder = Attachments::default();
        CODEC.set(&mut holder, Holder::Level, 27).unwrap();
        let frame =
            SyncedAttachmentSnapshot::capture(&holder, 8_192, Compound::storage_bytes).unwrap();
        assert_eq!(CODEC.get_existing(&mut holder, Holder::Level), Ok(Some(27)));
        assert_eq!(frame.get(&CODEC), Ok(Some(&20)));
    }
}
