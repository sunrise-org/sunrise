//! The slot rules one click was answered: `menu_slot_rules` is asked once for each slot, stack
//! and set of the slot's own answers while a click plans, and the click's later steps
//! (`tryRemove`, `allowModification`, a quick move a mod left to the game) reuse that answer, so
//! the server and the client's prediction ask the same number of times.
use super::*;
use std::cell::RefCell;

/// A question to the rules: the menu slot, the stack and the slot's own answers.
type Question = (usize, ItemStack, (bool, bool, u32));

/// What one click was answered by `menu_slot_rules`: `mayPlace`, `mayPickup` and the maximum
/// for each question it asked.
#[derive(Clone, Debug, Default)]
pub struct Asked(Vec<(Question, (bool, bool, u32))>);

impl Asked {
    /// Whether no question was asked.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// A menu's answers while a click runs; none outside one, where every question asks afresh.
#[derive(Debug, Default)]
pub(super) struct Planning(RefCell<Option<Asked>>);

impl Menu {
    /// Runs `f` as one click, starting from the answers in `asked`, and gives back what the
    /// click was answered. Inside a click already running, `f` is part of that click.
    pub(super) fn planned<R>(
        &mut self,
        asked: Asked,
        f: impl FnOnce(&mut Self) -> R,
    ) -> (R, Asked) {
        if self.planning.0.borrow().is_some() {
            return (f(self), Asked::default());
        }
        *self.planning.0.borrow_mut() = Some(asked);
        let ran = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| f(self)));
        let asked = self.planning.0.take().unwrap_or_default();
        match ran {
            Ok(ran) => (ran, asked),
            Err(panic) => std::panic::resume_unwind(panic),
        }
    }

    /// `click`, starting from the answers `asked` the server got for this click before it
    /// reached the menu (the pickup rule `menu_quick_move` is preceded by): not asked again.
    pub fn click_asked(
        &mut self,
        env: &Env,
        slot_index: i32,
        button: i32,
        input: Input,
        asked: Asked,
    ) -> Result<(), &'static str> {
        self.start_click(env, slot_index, button, input, asked)
            .0
            .map(|_| ())
    }

    /// Start a click that may suspend at the first take, preserving the rule answers.
    pub fn start_click(
        &mut self,
        env: &Env,
        slot_index: i32,
        button: i32,
        input: Input,
        asked: Asked,
    ) -> (Result<Option<ClickContinuation>, &'static str>, Asked) {
        self.planned(asked, |menu| {
            menu.click_planned(env, slot_index, button, input)
        })
    }

    /// `Slot.mayPickup` of slot `s` asked before a click reaches the menu, and the answers to
    /// hand that click (`click_asked`).
    pub fn ask_pickup(&mut self, env: &Env, s: usize) -> (bool, Asked) {
        self.planned(Asked::default(), |menu| menu.may_pickup(env, s))
    }

    /// `apply_menu_slot_rules` for menu slot `s` of menu type `id`: during a click, asked once
    /// for each stack and set of the slot's own answers (`own`), then answered from the click's
    /// record.
    pub(super) fn ask_slot_rules(
        &self,
        env: &Env,
        id: &str,
        s: usize,
        stack: &ItemStack,
        own: (bool, bool, u32),
        item_max: u32,
    ) -> (bool, bool, u32) {
        let index = self.state.slots[s].index;
        let ask = || {
            apply_menu_slot_rules(
                env.slot_rules,
                id,
                index,
                stack,
                self.slot_context(env, s),
                own,
                item_max,
            )
        };
        let known = self.planning.0.borrow().as_ref().map(|asked| {
            asked
                .0
                .iter()
                .find(|((slot, held, mine), _)| *slot == s && held == stack && *mine == own)
                .map(|(_, answer)| *answer)
        });
        match known {
            None => ask(),
            Some(Some(answer)) => answer,
            // Not held across the ask: the rules may read other menus.
            Some(None) => {
                let answer = ask();
                if let Some(asked) = self.planning.0.borrow_mut().as_mut() {
                    asked.0.push(((s, stack.clone(), own), answer));
                }
                answer
            }
        }
    }
}
