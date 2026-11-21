use super::*;
use crate::game::{
    api_values::{TryIntoApi, TryIntoEngine},
    transactions::OpenTransactions,
};
use mod_api::{
    BlockEntityItemError, BlockEntityRef, BlockEntityTickAccess, BlockEntityTickScope,
    BlockEntityView, ResidentBlockEntity, ServerHost,
};
use std::collections::HashMap;
use sunrise_client::{block_entities::BlockEntityLoan, chunk_map::ChunkMap, fast_hash::FxHashMap};
use sunrise_core::nbt::{Compound, JavaString, Tag};
use sunrise_items::items::Items;

#[cfg(test)]
#[path = "block_entity_tick/tests.rs"]
mod tests;

impl Game<'_> {
    /// `LevelChunk.BoundTickingBlockEntity.tick` for a resident mod block entity.
    /// Returns whether the ticker was dispatched, so skipped rows have no after-slot.
    pub(in crate::game) fn tick_mod_block_entity_loaded(&mut self, pos: Pos) -> bool {
        let Some(BlockEntity::Mod(entity)) = self.world.block_entities.get(&pos) else {
            return false;
        };
        let Some(kind) = entity.tick_kind() else {
            return false;
        };
        let Some(generation) = self.world.block_entities.generation(&pos) else {
            return false;
        };
        let live_state = self.world.block(pos.0, pos.1, pos.2);
        let block = &self.res.facts().get(live_state).name;
        if !valid_tick_subject(block, entity) {
            if let Some(id) = self
                .world
                .block_entities
                .record_invalid_mod_tick(pos, generation)
            {
                log::warn!("Block entity {id} at {pos:?} cannot tick on {block}");
            }
            return false;
        }
        let Some(level) = self.level(self.dim.id()) else {
            return false;
        };
        let subject =
            mod_api::__private::game::block_entity_ref(level, [pos.0, pos.1, pos.2], generation);
        let mut access = TickAccess {
            game: self,
            subject,
        };
        Self::native_block_entity_tick(kind, BlockEntityTickScope::new(&mut access));
        self.world
            .block_entities
            .clear_ticker_warning(pos, generation);
        true
    }

    /// Dispatch the one resident server ticker; the scope owns access to the Host.
    pub(in crate::game) fn native_block_entity_tick(
        kind: mod_api::BlockEntityTypeKey,
        subject: mod_api::BlockEntityTickScope<'_>,
    ) {
        Self::fire_block_entity_tick(kind, subject);
    }
}

fn valid_tick_subject(block: &str, entity: &EngineBlockEntity) -> bool {
    sunrise_client::block_entity_types::valid(block, entity.id()) == Some(true)
}

struct TickAccess<'g, 'w> {
    game: &'g mut Game<'w>,
    subject: BlockEntityRef,
}

impl BlockEntityTickAccess for TickAccess<'_, '_> {
    fn with_view(&mut self, call: &mut dyn FnMut(BlockEntityView<'_>)) {
        use mod_api::__private::game::Fields;
        let target = self.subject.fields();
        let [x, y, z] = target.position;
        if !self.game.level_in_place(target.level) || !self.game.world.loaded(x, z) {
            return;
        }
        let live_state = self.game.world.block(x, y, z);
        let Ok(state) = live_state.try_into_api() else {
            return;
        };
        let slot_count = self
            .game
            .world
            .block_entities
            .mod_container(live_state.0)
            .map_or(0, |c| c.slots);
        let pos = (x, y, z);
        let Some(loan) = self
            .game
            .world
            .block_entities
            .loan_mod_entity(pos, target.generation)
        else {
            return;
        };
        if !valid_tick_subject(&self.game.res.facts().get(live_state).name, loan.entity) {
            return;
        }
        let mut data = TickData {
            pos,
            loan,
            slot_count,
            items: self.game.res.items(),
            journal: &mut self.game.transactions,
            chunks: &self.game.world.chunks,
            names: &self.game.world.block_names,
            hoppers: &self.game.hoppers,
        };
        call(BlockEntityView::new(self.subject, state, &mut data));
    }

    fn host(&mut self) -> &mut dyn ServerHost {
        self.game
    }
}

struct TickData<'a> {
    pos: Pos,
    loan: BlockEntityLoan<'a>,
    slot_count: usize,
    items: &'a Items,
    journal: &'a mut OpenTransactions,
    chunks: &'a ChunkMap,
    names: &'a FxHashMap<Pos, Tag>,
    hoppers: &'a HashMap<Pos, super::super::machines::HopperState>,
}

impl TickData<'_> {
    fn entity(&self) -> &EngineBlockEntity {
        self.loan.entity
    }

    fn edit(&mut self) -> &mut EngineBlockEntity {
        self.journal.journal_block_entity_loan(
            self.pos,
            &self.loan,
            self.chunks,
            self.names.get(&self.pos),
            self.hoppers.get(&self.pos).copied(),
        );
        self.loan.entity
    }
}

impl ResidentBlockEntity for TickData<'_> {
    fn id(&self) -> &str {
        self.entity().id()
    }
    fn data_slots(&self) -> &[i32] {
        &self.entity().data
    }

    fn set_data_slot(&mut self, slot: u32, value: i32) -> bool {
        let index = slot as usize;
        if self
            .entity()
            .data
            .get(index)
            .is_none_or(|old| *old == value)
        {
            return false;
        }
        self.edit().data[index] = value;
        true
    }

    fn set_data_slots(&mut self, data: &[i32]) -> bool {
        if self.entity().data == data {
            return false;
        }
        let dest = &mut self.edit().data;
        dest.clear();
        dest.extend_from_slice(data);
        true
    }

    fn nbt(&self) -> &Compound {
        &self.entity().nbt
    }

    fn set_nbt(&mut self, key: JavaString, value: Option<Tag>) -> bool {
        let is_items = key.0.iter().copied().eq("Items".encode_utf16());
        if ["id", "x", "y", "z", sunrise_client::containers::SHAPES_KEY]
            .iter()
            .any(|reserved| key.0.iter().copied().eq(reserved.encode_utf16()))
            || (is_items && self.slot_count > 0)
            || value.as_ref().is_some_and(|tag| {
                matches!(tag, Tag::End) || sunrise_client::nbt_path::too_deep(tag, 1)
            })
        {
            return false;
        }
        let unchanged = if is_items && !self.entity().items.is_empty() {
            sunrise_client::save::block_entity_encode::mod_items(self.entity(), self.items) == value
        } else {
            self.entity().nbt.get(&key) == value.as_ref()
        };
        if unchanged {
            return false;
        }
        let entity = self.edit();
        if is_items {
            entity.items.clear();
        }
        match value {
            Some(tag) => {
                entity.nbt.insert(key, tag);
            }
            None => {
                entity.nbt.remove(&key);
            }
        }
        true
    }

    fn item_slots(&self) -> usize {
        self.slot_count
    }

    fn item_slot(&self, slot: u32) -> Option<(&str, u32)> {
        let index = slot as usize;
        if index >= self.slot_count {
            return None;
        }
        let stack = self.entity().items.get(index)?.as_ref()?;
        Some((&stack.item, stack.count))
    }

    fn copy_item_slot(&self, slot: u32) -> Option<mod_api::ItemStack> {
        self.entity()
            .items
            .get(slot as usize)?
            .as_ref()?
            .try_into_api()
            .ok()
    }

    fn set_item_slot(
        &mut self,
        slot: u32,
        stack: Option<mod_api::ItemStack>,
    ) -> Result<bool, BlockEntityItemError> {
        let index = slot as usize;
        let slots = self.slot_count;
        if index >= slots {
            return Err(BlockEntityItemError::SlotOutOfBounds {
                slot,
                slots: slots as u32,
            });
        }
        let stack = stack
            .as_ref()
            .map(|stack| stack.try_into_engine(self.items))
            .transpose()?;
        let entity = self.entity();
        if entity.items.get(index).and_then(Option::as_ref) == stack.as_ref()
            && !entity.has_opaque_item_slot(index)
        {
            return Ok(false);
        }
        let entity = self.edit();
        if index >= entity.items.len() {
            entity.items.resize(slots, None);
        }
        if let Some(dest) = entity.item_slot_mut(index) {
            *dest = stack;
        }
        Ok(true)
    }

    fn set_client_update_fields(&mut self, fields: Compound) {
        if !fields.is_empty() || self.entity().has_client_update_fields() {
            self.edit().set_client_update_fields(fields);
        }
    }
}
