//! Server-side `menu_slot_take`: the slot has delivered its stack and the click is paused. The
//! listener borrows a detached backing entity while the click's savepoint keeps all tentative
//! storage and effects.
use super::api_values::{TryIntoApi, finish_block_entity_writeback};
use super::{Game, transactions::After};
use sunrise_client::{
    block_entities::Pos,
    containers::{BlockEntity, ModBlockEntity},
    items::ItemStack,
    menu::SlotTake,
};

impl Game<'_> {
    /// Deliver a take made by the current click, inside its savepoint.
    pub(super) fn mod_menu_slot_take(
        &mut self,
        pos: Pos,
        player: u128,
        menu: &str,
        take: &SlotTake,
    ) -> bool {
        let Some(BlockEntity::Mod(m)) = self.world.block_entities.get_mut(&pos) else {
            return true;
        };
        if m.id().is_empty() {
            return true;
        }
        let mut entity = std::mem::take(&mut **m);
        let items = entity.items.clone();
        if !self.api_menu_slot_take(&mut entity, player, menu, take) {
            // The enclosing click journal still holds the original entity, even if the listener
            // removed or replaced it. Refusal must not spill the detached entity's contents.
            return false;
        }
        self.put_back_menu_entity(pos, entity, &items);
        true
    }

    /// Give the edited entity back to the slot it came from. Defer saves, comparator/listener
    /// work and spill until the whole click commits, including any later take.
    pub(super) fn put_back_menu_entity(
        &mut self,
        pos: Pos,
        entity: ModBlockEntity,
        before: &[Option<ItemStack>],
    ) {
        if let Some(BlockEntity::Mod(m)) = self.world.block_entities.get_mut(&pos)
            && m.id().is_empty()
        {
            let changed = entity.items != before;
            **m = entity;
            self.after_commit(if changed {
                After::Changed(pos)
            } else {
                After::DirtyBlockEntity(pos)
            });
        } else {
            self.after_commit(After::LostMenuContents(pos, Box::new(entity)));
        }
    }

    /// Notify with the entity already detached. Validate its edited snapshot before committing
    /// the listener's Host writes. A take that cannot cross to the API is not heard.
    pub(super) fn api_menu_slot_take(
        &mut self,
        entity: &mut ModBlockEntity,
        player: u128,
        menu: &str,
        (slot, taken, left): &SlotTake,
    ) -> bool {
        let (Ok(mut api), Ok(taken), Ok(left)) = (
            entity.try_into_api(),
            taken.try_into_api(),
            left.try_into_api(),
        ) else {
            return true;
        };
        let revision = api.revision();
        let heard = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            mod_api::run_transaction(self, |game, _| {
                game.native_menu_slot_take(&mut api, player, menu, *slot, &taken, left.as_ref());
                finish_block_entity_writeback(
                    entity,
                    &api,
                    revision,
                    game.items(),
                    "menu_slot_take",
                    Ok(()),
                )
            })
        }));
        match heard {
            Ok(result) => result.is_ok(),
            Err(_) => {
                log::error!("menu_slot_take: a listener failed for {menu}; the take is undone");
                false
            }
        }
    }
}
