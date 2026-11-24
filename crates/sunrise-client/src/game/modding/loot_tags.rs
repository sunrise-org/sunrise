//! Adapt the current loot evaluator's tag reads to the existing API resource.
use mod_api::{BlockKey, FluidKey, Identifier, ItemKey, TagKey, TagMembers, Tags};

pub(crate) struct LootTags<'a>(pub &'a dyn sunrise_world::loot::LootTags);

impl Tags for LootTags<'_> {
    fn key(&self, id: &Identifier) -> Option<TagKey> {
        mod_api::__private::tag_key(id)
    }
    fn is_in_tag(&self, item: ItemKey, tag: TagKey) -> Option<bool> {
        let item = mod_api::__private::item_key_id(item)?;
        mod_api::__private::with_tag_name(tag, |tag| Some(self.0.contains("item", tag?, item)))
    }
    fn damage_type_in_tag(&self, damage_type: &str, tag: TagKey) -> Option<bool> {
        mod_api::__private::with_tag_name(tag, |tag| self.0.damage_type_in_tag(damage_type, tag?))
    }
    fn item_tags(&self, item: ItemKey) -> Option<Vec<Identifier>> {
        super::native_tags::item_tags(self.0, item)
    }
    fn item_tag_names(&self) -> Option<Vec<Identifier>> {
        super::native_tags::item_tag_names(self.0)
    }
    fn block_in_tag(&self, block: BlockKey, tag: TagKey) -> Option<bool> {
        let block = mod_api::__private::block_key_id(block)?;
        mod_api::__private::with_tag_name(tag, |tag| Some(self.0.contains("block", tag?, block)))
    }
    fn fluid_in_tag(&self, fluid: FluidKey, tag: TagKey) -> Option<bool> {
        let fluid = mod_api::__private::fluid_id(fluid)?;
        mod_api::__private::with_tag_name(tag, |tag| Some(self.0.contains("fluid", tag?, fluid)))
    }
    fn items(&self, tag: TagKey) -> TagMembers<ItemKey> {
        super::native_tags::members(
            tag,
            |name| self.0.members("item", name),
            mod_api::__private::item_key,
        )
    }
    fn blocks(&self, tag: TagKey) -> TagMembers<BlockKey> {
        super::native_tags::members(
            tag,
            |name| self.0.members("block", name),
            mod_api::__private::block_key,
        )
    }
    fn fluids(&self, tag: TagKey) -> TagMembers<FluidKey> {
        super::native_tags::members(
            tag,
            |name| self.0.members("fluid", name),
            mod_api::__private::fluid_key,
        )
    }
}
