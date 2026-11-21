//! Frozen, per-entity attachment data for the submission pass.
use crate::wire::{Reader, Wire, WireError, Writer};
use crate::{AttachmentError, AttachmentType};
use std::sync::Arc;
use sunrise_core::nbt::{self, Limits, Tag};
use sunrise_mod_runtime::attachments::{AttachmentEntry, Attachments};

// The render call carries a projection, not a replay of the sync packets.
const RENDER_FRAME_BYTES: usize = 64 * 1024;

// Namespace, name and NBT each have a Wire length prefix.
const FIELD_PREFIX_BYTES: usize = 3 * size_of::<u32>();

/// A general entity's synced fields as they were at render-state extraction.
/// Fields that could not be captured are unavailable through [`Self::get`];
/// they do not prevent submission.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, sunrise_mod_api_macros::Wire)]
pub struct EntityRenderAttachments {
    data: Arc<Snapshot>,
}

struct Snapshot {
    // Pre-encoded when sync arrives, not for every render call.
    bytes: Vec<u8>,
    entries: Vec<(String, String, AttachmentEntry)>,
}

impl std::fmt::Debug for Snapshot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Snapshot")
            .field("fields", &self.entries.len())
            .finish()
    }
}

impl PartialEq for Snapshot {
    fn eq(&self, other: &Self) -> bool {
        self.bytes == other.bytes
    }
}

impl EntityRenderAttachments {
    /// Projects the holder's synced fields into a bounded render input in key order.
    /// A field that doesn't fit doesn't use the budget or affect the synced holder.
    // hidden: engine-side only; mods consume the snapshots handed to their renderer.
    #[doc(hidden)]
    pub fn from_synced(attachments: &Attachments) -> Self {
        let mut values: Vec<_> = attachments
            .synced()
            .into_iter()
            .filter_map(|field| {
                Some((
                    field.namespace.to_string().ok()?,
                    field.name.to_string().ok()?,
                    field.value?,
                ))
            })
            .collect();
        values.sort_unstable_by(|a, b| (&a.0, &a.1).cmp(&(&b.0, &b.1)));
        let mut writer = Writer::new();
        writer.put_len(0); // Filled with the count of fields actually captured below.
        let mut entries = Vec::new();
        for (namespace, name, value) in values {
            let Some(budget) = (RENDER_FRAME_BYTES - writer.bytes().len())
                .checked_sub(FIELD_PREFIX_BYTES + namespace.len() + name.len())
            else {
                continue;
            };
            let tag = Tag::Compound(value);
            let Ok(encoded) = nbt::write_network(
                &tag,
                Limits {
                    max_bytes: budget,
                    ..Limits::default()
                },
            ) else {
                continue;
            };
            writer.put_str(&namespace);
            writer.put_str(&name);
            writer.put(&encoded);
            entries.push((
                namespace,
                name,
                AttachmentEntry::from_saved([("value".into(), tag)].into()),
            ));
        }
        let mut bytes = writer.into_bytes();
        bytes[..size_of::<u32>()].copy_from_slice(&(entries.len() as u32).to_le_bytes());
        Self {
            data: Arc::new(Snapshot { bytes, entries }),
        }
    }

    /// Borrows the synced value of `key`, or returns `Ok(None)` if absent or not captured.
    /// Never inserts a default and never reads the live world. A captured value uses the key's
    /// codec, just as on the synced client holder.
    pub fn get<T: Send + Sync + 'static>(
        &self,
        key: &AttachmentType<T>,
    ) -> Result<Option<&T>, AttachmentError> {
        crate::parts::validate_attachment_use(key.namespace, key.name, key.sync)?;
        self.data
            .entries
            .binary_search_by(|(namespace, name, _)| {
                (namespace.as_str(), name.as_str()).cmp(&(key.namespace, key.name))
            })
            .ok()
            .map(|i| self.data.entries[i].2.read(key))
            .transpose()
    }
}

impl Wire for Snapshot {
    crate::wire::layout_fields!("fields" => Vec<(String, String, Vec<u8>)>);
    const MIN: usize = 4;

    fn put(&self, out: &mut Writer) {
        out.put_bytes(&self.bytes);
    }

    fn take(input: &mut Reader<'_>) -> Result<Self, WireError> {
        input.bounded(RENDER_FRAME_BYTES, |payload| {
            let bytes = payload.rest();
            let count = payload.take_len(FIELD_PREFIX_BYTES)?;
            let mut entries: Vec<(String, String, AttachmentEntry)> = Vec::with_capacity(count);
            for _ in 0..count {
                let (namespace, name): (String, String) = (payload.take()?, payload.take()?);
                if entries
                    .last()
                    .is_some_and(|(ns, path, _)| (ns, path) >= (&namespace, &name))
                {
                    return Err(WireError::Invalid("unordered attachment keys"));
                }
                let n = payload.take_len(1)?;
                let (tag, consumed) = nbt::read_network(payload.take_bytes(n)?, Limits::default())
                    .map_err(|_| WireError::Invalid("invalid attachment NBT"))?;
                if !matches!(tag, Tag::Compound(_)) || consumed != n {
                    return Err(WireError::Invalid("invalid attachment compound"));
                }
                entries.push((
                    namespace,
                    name,
                    AttachmentEntry::from_saved([("value".into(), tag)].into()),
                ));
            }
            Ok(Self {
                bytes: bytes[..bytes.len() - payload.rest().len()].to_vec(),
                entries,
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attachment_snapshot_wire_decode_refuses_more_than_one_render_frame() {
        let encoded = nbt::write_network(
            &Tag::Compound([("v".into(), Tag::ByteArray(vec![42; 40 * 1024]))].into()),
            Limits::default(),
        )
        .unwrap();
        let fields = ["a", "b"].map(|name| ("test".to_owned(), name.to_owned(), encoded.clone()));
        let mut writer = Writer::new();
        writer.put(&Vec::from(fields));
        assert!(matches!(
            Snapshot::take(&mut Reader::new(writer.bytes())),
            Err(WireError::Length)
        ));
    }
}
