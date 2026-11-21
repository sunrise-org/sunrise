//! The staged slot view lent to a `menu_slot_rules` ask and the reads needed to replay its answer.
use super::{
    CONTAINER_MAX, Env, GRID, ItemStack, MAIN, Menu, PLAYER, RESULT, SlotContext, SlotRules,
};
use mod_api::MenuContainer;
use std::cell::RefCell;

/// A slot rule's dependency on the planner's view. Store only what the rule read, including
/// empty slots; replaying the rule does not rebuild or convert the menu's slot matrix.
#[derive(Clone, Debug)]
pub(super) struct SlotReads {
    player: Option<u128>,
    pos: Option<[i32; 3]>,
    container: MenuContainer,
    slots: Vec<(MenuContainer, usize, Option<ItemStack>)>,
}

impl SlotReads {
    /// Compare through the view so an enclosing record depends on this comparison too. Even
    /// a replay miss depends on the slot that caused it: a later stage could match this answer.
    pub(super) fn matches(&self, context: SlotContext<'_>) -> bool {
        (self.player, self.pos, self.container) == (context.player, context.pos, context.container)
            && self
                .slots
                .iter()
                .all(|(container, index, stack)| context.slot(*container, *index) == stack.as_ref())
    }
}

/// The API's key for one of a menu's containers.
fn key(container: usize) -> MenuContainer {
    match container {
        MAIN => MenuContainer::Main,
        GRID => MenuContainer::Grid,
        RESULT => MenuContainer::Result,
        _ => MenuContainer::Player,
    }
}

/// The container a LAN view's slot `i` is in: the block's container's slots are `Main`, the
/// rest the player's.
pub fn lan_container(i: usize, container: Option<&[Option<ItemStack>]>) -> MenuContainer {
    match container {
        Some(c) if i < c.len() => MenuContainer::Main,
        _ => MenuContainer::Player,
    }
}

impl SlotContext<'_> {
    pub(super) fn record<R>(self, ask: impl FnOnce(SlotContext<'_>) -> R) -> (R, SlotReads) {
        let reads = RefCell::new(Vec::new());
        let trace = |container, index, stack: Option<&ItemStack>| {
            if let Some(outer) = self.trace {
                outer(container, index, stack);
            }
            let mut reads = reads.borrow_mut();
            if !reads.iter().any(|&(c, i, _)| (c, i) == (container, index)) {
                reads.push((container, index, stack.cloned()));
            }
        };
        let answer = ask(SlotContext {
            trace: Some(&trace),
            ..self
        });
        let reads = SlotReads {
            player: self.player,
            pos: self.pos,
            container: self.container,
            slots: reads.into_inner(),
        };
        (answer, reads)
    }

    /// What slot `index` of `container` holds, keyed as `MenuSlotContext::slot`.
    pub fn slot(&self, container: MenuContainer, index: usize) -> Option<&ItemStack> {
        let slots = match container {
            MenuContainer::Main => {
                if self.main_crafting.is_some_and(|crafting| {
                    crafting.grid_slots().contains(&index) || index == crafting.result
                }) {
                    &[]
                } else {
                    self.slots
                }
            }
            MenuContainer::Grid => self.grid,
            MenuContainer::Result => self.result,
            _ => &[],
        };
        let stack = slots.get(index).and_then(Option::as_ref);
        if let Some(trace) = self.trace {
            trace(container, index, stack);
        }
        stack
    }
}

impl Menu {
    /// Who and where this menu's `menu_slot_rules` ask about menu slot `s` is: the env's
    /// player, the menu's block (`set_block_pos`), `s`'s container and the menu's own
    /// staged containers.
    pub(super) fn slot_context<'s>(&'s self, env: &Env, s: usize) -> SlotContext<'s> {
        let container = self
            .state
            .slots
            .get(s)
            .map_or(PLAYER, |slot| slot.container);
        SlotContext {
            player: env.player,
            pos: self.state.block_pos,
            container: key(container),
            slots: &self.state.containers[MAIN],
            main_crafting: None,
            grid: &self.state.containers[GRID],
            result: &self.state.containers[RESULT],
            trace: None,
        }
    }
}

/// Apply the scoped rule to a slot's own answers (`may_place`, `may_pickup` and the maximum, in
/// that order), then enforce its item cap. The rules are asked here, while the click plans, and
/// only for a menu type a part answers; with none lent the slot's answers stand.
pub fn apply_menu_slot_rules(
    rules: Option<&dyn SlotRules>,
    menu: &str,
    slot: usize,
    stack: &ItemStack,
    context: SlotContext<'_>,
    own: (bool, bool, u32),
    item_max: u32,
) -> (bool, bool, u32) {
    let (mut may_place, mut may_pickup, mut max_stack) = own;
    if let Some(rules) = rules.filter(|rules| rules.listens(menu)) {
        rules.rule(
            menu,
            slot,
            stack,
            context,
            &mut may_place,
            &mut may_pickup,
            &mut max_stack,
        );
    }
    let max_stack = max_stack.clamp(1, CONTAINER_MAX).min(item_max);
    (may_place, may_pickup, max_stack)
}
