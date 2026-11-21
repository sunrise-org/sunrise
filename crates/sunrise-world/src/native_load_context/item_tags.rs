//! Authoritative item tags captured once for a resource-generation reader.
use super::*;
use std::collections::HashSet;

pub(super) struct ItemTags {
    ordered: HashMap<String, Vec<String>>,
    membership: HashMap<String, HashSet<String>>,
}

impl LoopContext {
    /// Publish authoritative members in their source order; None means not loaded/synced yet.
    /// Call only at the owning load or synchronization boundary, never from a recipe read.
    pub fn set_item_tags(&mut self, tags: Option<HashMap<String, Vec<String>>>) {
        self.item_tags = tags.map(|ordered| {
            let membership = ordered
                .iter()
                .map(|(tag, members)| (tag.clone(), members.iter().cloned().collect()))
                .collect();
            Arc::new(ItemTags {
                ordered,
                membership,
            })
        });
    }

    /// Members in authoritative order; an unknown loaded tag has an empty member list.
    pub fn item_tag_members(&self, tag: &str) -> Option<&[String]> {
        let tags = self.item_tags.as_ref()?;
        Some(tags.ordered.get(tag).map_or(&[], Vec::as_slice))
    }

    /// Membership in this generation; None distinguishes unavailable tables from an empty tag.
    pub fn item_is_in_tag(&self, item: &str, tag: &str) -> Option<bool> {
        let tags = self.item_tags.as_ref()?;
        Some(
            tags.membership
                .get(tag)
                .is_some_and(|members| members.contains(item)),
        )
    }
}
