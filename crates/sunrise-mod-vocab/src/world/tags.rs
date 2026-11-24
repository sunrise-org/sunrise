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

/// Read merged block, item, fluid and damage type tags without borrowing the game's registry
/// structs. A client reads the tags its server sent. The list methods distinguish absent tags
/// from loaded empty tags, and report unavailable until that registry's authoritative tags
/// are applied or if a member key cannot be resolved. A retained tag key reads the current
/// snapshot on each call, including after reload.
pub trait Tags {
    /// Resolve a validated name once. The name need not currently have members;
    /// retain its key across reloads. None if the runtime cannot allocate a name slot.
    fn key(&self, _id: &Identifier) -> Option<TagKey> {
        unimplemented!("Tags::key")
    }
    /// Whether `item` is in the current item tag. An unknown tag is false;
    /// unavailable authoritative tags or item mappings answer None.
    fn is_in_tag(&self, _item: ItemKey, _tag: TagKey) -> Option<bool> {
        unimplemented!("Tags::is_in_tag")
    }
    /// `DamageSource.is(TagKey)`: whether damage type `damage_type`, a hit's
    /// [`HurtSource::damage_type`](crate::HurtSource::damage_type) (`minecraft:arrow`), is in the
    /// current damage type tag, as bound at the last data reload. An unknown tag is false;
    /// an unknown damage type, or unavailable authoritative damage type tags, answer None.
    /// A tag service with no damage type data, such as a crafting kind's, answers None.
    fn damage_type_in_tag(&self, _damage_type: &str, _tag: TagKey) -> Option<bool> {
        None
    }
    /// The names of the tags that currently contain `item`, each once, in no specified order.
    /// `None` means the item mapping or authoritative tag names are unavailable.
    /// A known item with no tags returns an empty list. This does not resolve tag keys.
    ///
    /// Each call tests the item against every bound item tag and builds a new list: keep it
    /// out of per-tick and per-entity hooks. Read it when the answer is needed, or after a
    /// data reload, and keep the list; for one known tag, [`is_in_tag`](Self::is_in_tag) is
    /// a single lookup.
    fn item_tags(&self, _item: ItemKey) -> Option<Vec<Identifier>> {
        unimplemented!("Tags::item_tags")
    }
    /// Every currently bound item tag's name, including empty tags, each once in no specified order.
    /// `None` means the authoritative tag names are unavailable.
    /// A loaded item registry with no bound tags returns an empty list.
    /// Call [`key`](Self::key) separately if you need a key for membership reads.
    ///
    /// Each call builds a new list of every bound item tag: keep it out of per-tick and
    /// per-entity hooks, and keep the list between data reloads.
    fn item_tag_names(&self) -> Option<Vec<Identifier>> {
        unimplemented!("Tags::item_tag_names")
    }
    /// Whether `block` is in the current block tag (`BlockState.is(TagKey)`). An unknown tag
    /// is false; unavailable authoritative tags or block mappings answer None.
    fn block_in_tag(&self, _block: BlockKey, _tag: TagKey) -> Option<bool> {
        unimplemented!("Tags::block_in_tag")
    }
    /// Whether `fluid` is in the current fluid tag (`FluidState.is(TagKey)`). An unknown tag
    /// is false; unavailable authoritative tags or fluid mappings answer None.
    fn fluid_in_tag(&self, _fluid: FluidKey, _tag: TagKey) -> Option<bool> {
        unimplemented!("Tags::fluid_in_tag")
    }
    /// Read the current item tag in vanilla's order, including unrequested vanilla items.
    fn items(&self, _tag: TagKey) -> TagMembers<ItemKey> {
        unimplemented!("Tags::items")
    }
    /// Read the current fluid tag in vanilla's order.
    fn fluids(&self, _tag: TagKey) -> TagMembers<FluidKey> {
        unimplemented!("Tags::fluids")
    }
    /// Read the current block tag in vanilla's order, including unrequested vanilla blocks.
    fn blocks(&self, _tag: TagKey) -> TagMembers<BlockKey> {
        unimplemented!("Tags::blocks")
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
