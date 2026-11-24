//! Whole stacks for server mods: one given to a player as `/give` gives it
//! (`player_give_stack`), and one read from SNBT as a saved item is (`item_from_snbt`).
use super::{
    Game,
    api_values::{StackReadError, TryIntoApi},
};
use mod_api::{PlayerDataRefusal, PlayerRef};
use sunrise_client::{items::ItemStack, lan::playerdata};
use sunrise_items::item_nbt;

impl Game<'_> {
    /// `GiveCommand.giveItem` for one stack: `Inventory.add`, the rest thrown for the player
    /// alone to pick up in their level, even when the hook is running in another one.
    pub(super) fn player_give_stack(
        &mut self,
        player: PlayerRef,
        stack: ItemStack,
    ) -> Result<Option<ItemStack>, PlayerDataRefusal> {
        self.player_write_gate(player)?;
        let owner = Some(playerdata::uuid_ints(player.uuid()));
        let mut rest = None;
        let ran = self.native_as_player_handle(player, |game| {
            rest = game.give_acting_stack(stack, owner);
            // containerMenu.broadcastChanges: InventoryChangeTrigger.
            if game.player.is_some() {
                game.after_commit(super::transactions::After::InventoryTrigger(player.uuid()));
            }
        });
        if ran {
            Ok(rest)
        } else {
            Err(PlayerDataRefusal::Stale)
        }
    }

    /// `TagParser.parseCompoundFully` followed by `ItemStack.CODEC`. This is text, not
    /// binary input to `NbtAccounter`.
    pub(super) fn item_from_snbt(&self, snbt: &str) -> Option<mod_api::ItemStack> {
        let allow_custom = self
            .remote
            .as_ref()
            .is_none_or(|remote| remote.registries.custom_components);
        item_nbt::item_from_snbt_with(
            snbt,
            |id| allow_custom && sunrise_protocol::item::custom::id(id).is_some(),
            |id| self.items().get(id).is_some(),
        )
        .and_then(|stack| {
            self.items()
                .get(&stack.item)
                .ok_or_else(|| format!("unknown item {}", stack.item))?;
            stack.try_into_api().map_err(|error| match error {
                StackReadError::UnknownItem => format!("unresolved item key {}", stack.item),
                StackReadError::Empty => "empty item stack".into(),
                StackReadError::InvalidComponents(reason) => reason.to_string(),
            })
        })
        .inspect_err(|error| {
            let caller = mod_api::content::crash::current_native_mod().unwrap_or("game");
            log::warn!("{caller}: item_from_snbt: {error}");
        })
        .ok()
    }
}
