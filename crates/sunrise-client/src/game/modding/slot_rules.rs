//! `menu_slot_rules` on the game's side. A menu click cannot reach the game, so the game lends
//! itself to the click as its [`SlotRules`] (`Env::slot_rules`), with the read-only `Host`
//! behind it: the click asks while it plans, once for each slot and stack it consults, and the
//! answers stay in what it planned. A menu type no part answers is never asked, and nothing is
//! converted for it.
use super::*;
use crate::game::api_values::TryIntoApi;
use sunrise_client::{
    containers::ModMenuType,
    menu::{Asked, Kind, Menu, SlotContext, SlotRules},
};

impl SlotRules for Game<'_> {
    fn listens(&self, menu: &str) -> bool {
        mod_api::__private::has_part_hook(mod_api::HookId::MenuSlotRules)
            && mod_api::__private::menu_listens(mod_api::HookId::MenuSlotRules, menu)
    }

    fn rule(
        &self,
        menu: &str,
        slot: usize,
        stack: &ItemStack,
        context: SlotContext<'_>,
        may_place: &mut bool,
        may_pickup: &mut bool,
        max_stack: &mut u32,
    ) {
        // A stack that cannot cross to the API is not asked about: the slot's answers stand.
        let Ok(stack) = stack.try_into_api() else {
            return;
        };
        // Lend the staged menu slots. Convert only what the part reads; a stack that cannot
        // cross the API boundary shows empty, as an absent slot does.
        let read = |container, index| context.slot(container, index)?.try_into_api().ok();
        let view = mod_api::MenuSlotContext::new(context.player, context.pos, &read);
        // A listener that panics is a failed ask, not a failed click: the slot's own answers
        // stand (the dispatch writes nothing back from a failed pass), the failure is logged by
        // name, and the click goes on. The next ask, and the next click, are asked as ever.
        let asked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            mod_api::__private::contain_refusal(
                mod_api::content::crash::current_native_mod().unwrap_or("unknown"),
                || {
                    let (mut place, mut pickup, mut maximum) =
                        (*may_place, *may_pickup, *max_stack);
                    self.native_menu_slot_rules(mod_api::__private::MenuSlotRulesCall {
                        menu,
                        slot,
                        container: context.container,
                        stack: &stack,
                        context: &view,
                        may_place: &mut place,
                        may_pickup: &mut pickup,
                        max_stack: &mut maximum,
                    });
                    (place, pickup, maximum)
                },
            )
        }));
        match asked {
            Ok(Ok(answers)) => (*may_place, *may_pickup, *max_stack) = answers,
            Ok(Err(refusal)) => {
                refusal.report();
                log::warn!(
                    "menu_slot_rules: a listener was refused for {menu} slot {slot}: {refusal}; the slot's own answers stand"
                );
            }
            Err(_) => log::error!(
                "menu_slot_rules: a listener failed for {menu} slot {slot}; the slot's own answers stand"
            ),
        }
    }
}

impl Game<'_> {
    /// `Slot.mayPickup` of slot `slot` (the menu's index) of mod menu type `menu` while it holds
    /// `stack`, as the click itself answers it: the slot's own answer, then the parts' `may_pickup`.
    /// For the server's own steps that run before the click, as `menu_quick_move` does; the
    /// answers go to the click (`Menu::click_asked`), which does not ask again.
    pub(super) fn mod_menu_slot_gives(
        &self,
        menu: &'static ModMenuType,
        slot: usize,
        stack: &ItemStack,
        pos: [i32; 3],
        items: &[Option<ItemStack>],
    ) -> (bool, Asked) {
        let mut view = Menu::new(Kind::Mod(menu));
        view.set_registry_id(Some(menu.id));
        view.set_block_pos(Some(pos));
        view.load_container(&self.menu_env(), items);
        for (i, held) in self.inventory.iter().enumerate() {
            view.set_player_item(i, held.clone());
        }
        view.load_slot(slot, Some(stack.clone()));
        view.ask_pickup(&self.menu_env(), slot)
    }
}
