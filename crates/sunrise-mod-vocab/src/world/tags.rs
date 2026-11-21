//! Resolved tag names and the read-only merged-tag service.
use crate::{BlockKey, FluidKey, Identifier, ItemKey};

/// A process-lifetime tag name resolved once through [`Registrar::tag_key`](crate::TagsRegistrar::tag_key)
/// or [`Tags::key`].
/// Membership belongs to the current authoritative data reload, not to the key.
// no wire: decoding validates the u32 slot with TagKey::from_slot on the host.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
// exhaustive: the runtime alone creates name slots
pub struct TagKey(pub(crate) u32);

impl TagKey {
    pub(crate) fn from_slot(slot: u32) -> Result<Self, &'static str> {
        sunrise_mod_runtime::registry_keys::with_tag_name(slot, |id| {
            id.map(|_| Self(slot)).ok_or("unknown TagKey slot")
        })
    }
}

/// The result of reading one registry tag from the current authoritative snapshot.
// no wire: this native lookup result carries process-local registry keys.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum TagMembers<K> {
    /// No tag by this name bound, including a definition with an unresolved required entry.
    Missing,
    /// The bound tag's members in vanilla's order. Missing optional entries are skipped; the list may be empty.
    Members(Vec<K>),
    /// The snapshot or a required key mapping is unavailable. No partial list is returned.
    Unavailable,
}

/// Read merged block, item and fluid tags without borrowing the game's registry structs.
/// A client reads the tags its server sent. The list methods distinguish absent tags
/// from loaded empty tags, and report unavailable until that registry's authoritative tags
/// are applied or if a member key cannot be resolved. A retained tag key reads the current
/// snapshot on each call, including after reload.
pub trait Tags {
    /// Resolve a validated name once. The name need not currently have members;
    /// retain its key across reloads. None if the runtime cannot allocate a name slot.
    fn key(&self, _id: &Identifier) -> Option<TagKey> {
        crate::world::phase::unprovided(
            "Tags::key",
            "tags_key",
            crate::world::phase::LoadAccess::Metadata,
        )
    }
    /// Whether `item` is in the current item tag. An unknown tag is false;
    /// unavailable authoritative tags or item mappings answer None.
    fn is_in_tag(&self, _item: ItemKey, _tag: TagKey) -> Option<bool> {
        crate::world::phase::unprovided(
            "Tags::is_in_tag",
            "tags_is_in_tag",
            crate::world::phase::LoadAccess::Metadata,
        )
    }
    /// Read the current item tag in vanilla's order, including unrequested vanilla items.
    fn items(&self, _tag: TagKey) -> TagMembers<ItemKey> {
        crate::world::phase::unprovided(
            "Tags::items",
            "tags_items",
            crate::world::phase::LoadAccess::Metadata,
        )
    }
    /// Read the current fluid tag in vanilla's order.
    fn fluids(&self, _tag: TagKey) -> TagMembers<FluidKey> {
        crate::world::phase::unprovided(
            "Tags::fluids",
            "tags_fluids",
            crate::world::phase::LoadAccess::Metadata,
        )
    }
    /// Read the current block tag in vanilla's order, including unrequested vanilla blocks.
    fn blocks(&self, _tag: TagKey) -> TagMembers<BlockKey> {
        crate::world::phase::unprovided(
            "Tags::blocks",
            "tags_blocks",
            crate::world::phase::LoadAccess::Metadata,
        )
    }
}

/// Resolve a tag name at the game boundary.
pub fn tag_key(id: &Identifier) -> Option<TagKey> {
    sunrise_mod_runtime::registry_keys::tag_slot(id).map(TagKey)
}

/// Borrow a tag's canonical name within the runtime's read lock, without formatting or cloning.
pub fn with_tag_name<R>(key: TagKey, read: impl FnOnce(Option<&str>) -> R) -> R {
    sunrise_mod_runtime::registry_keys::with_tag_name(key.0, read)
}
