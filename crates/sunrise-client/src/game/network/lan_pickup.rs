//! A click on a LAN guest's Game-owned menu (the inventory, crafting table, machine, station,
//! lectern and mount menus), with the pickup decided by the mods first when any listens to
//! `item_stacked_on_other` (`stacked_pickup`). The trading menu builds its own and does the same
//! in `lan_trading.rs`.
use super::{Game, lan_crafting::input};
use crate::game::stacked_on_other::{Pickup, plan_click};
use sunrise_client::menu::{Env, Input, Menu, RuleAnswers, StoredRules};
use sunrise_protocol::packets::play;

/// The click a menu op makes: slot, button and input mode.
pub(in crate::game) type Click = (i32, i32, Input);

/// The click a packet makes.
pub(in crate::game) fn click_of(p: &play::ContainerClick) -> Click {
    (i32::from(p.slot), i32::from(p.button), input(p.input))
}

impl Game<'_> {
    /// `guest_menu_op`, with `click` (the click `op` makes, if it makes one) decided by the mods
    /// first. With no listener, no click or a click that is not a pickup this is `guest_menu_op`.
    /// A pickup the mods listen to is planned (their slot rules asked once, on a copy of `menu`
    /// set up by `prepare`, as `op` sets it up), frozen, decided and then run, inside the
    /// decision's savepoint, with the plan's answers; a refused one changes nothing and gives
    /// `None`, as does a guest who is gone.
    pub(super) fn guest_menu_op_decided<R>(
        &mut self,
        gi: usize,
        menu: &mut Menu,
        click: Option<Click>,
        prepare: Option<&dyn Fn(&mut Menu, &Env)>,
        op: impl FnOnce(&mut Menu, &Env) -> R,
    ) -> Option<R> {
        self.guest_menu_op_settled(gi, menu, click, prepare, op, |_, _, _| {})
    }

    /// `guest_menu_op_decided` for a menu whose click is written back to storage outside the menu
    /// (a machine's block entity, linked containers): `write_back` gets the game, the menu and
    /// what `op` returned, and runs right after `op`, inside the decision's savepoint when there
    /// is one, so an outer abort restores the storage together with the cursor and the slots.
    pub(super) fn guest_menu_op_settled<R>(
        &mut self,
        gi: usize,
        menu: &mut Menu,
        click: Option<Click>,
        prepare: Option<&dyn Fn(&mut Menu, &Env)>,
        op: impl FnOnce(&mut Menu, &Env) -> R,
        write_back: impl FnOnce(&mut Game<'_>, &mut Menu, &R),
    ) -> Option<R> {
        self.guest_menu_action(gi, menu, click, prepare, |game, menu, rules, stacked| {
            let ran = game.guest_menu_op_with(gi, menu, rules, stacked, op)?;
            write_back(game, menu, &ran);
            Some(ran)
        })
    }

    /// Run an authoritative menu action with a pickup's rule answers, when it needed a decision.
    /// The action can release the menu's `Env` between takes and continue inside the savepoint.
    pub(super) fn guest_menu_action<R>(
        &mut self,
        gi: usize,
        menu: &mut Menu,
        click: Option<Click>,
        prepare: Option<&dyn Fn(&mut Menu, &Env)>,
        action: impl FnOnce(
            &mut Game<'_>,
            &mut Menu,
            Option<&dyn sunrise_client::menu::SlotRules>,
            Option<&sunrise_client::menu::StackedOutcome>,
        ) -> Option<R>,
    ) -> Option<R> {
        let Some((slot, button, mode)) = click.filter(|c| c.2 == Input::Pickup) else {
            return action(self, menu, None, None);
        };
        if !Self::stacked_listened() {
            return action(self, menu, None, None);
        }
        // Freeze what the click finds on a copy; the answer determines its slot rules.
        let answers = RuleAnswers::default();
        let mut found = None;
        let mut copy = menu.clone();
        let planned = self.guest_menu_op(gi, &mut copy, |menu, env| {
            if let Some(prepare) = prepare {
                prepare(menu, env);
            }
            found = menu.stacked_click(env, slot, button, mode);
        });
        let (Some(frozen), Some(())) = (found, planned) else {
            return action(self, menu, None, None);
        };
        let conn = self.lan.as_ref()?.hub.guests[gi].conn;
        let route = self.lan.as_mut()?.menus.entry(conn).or_default().hub.id();
        let mut ran = None;
        let (carried, same) = (frozen.carried.clone(), frozen.clone());
        let pickup = self.stacked_pickup(
            route,
            frozen,
            move |game| {
                let now = game.lan.as_ref()?.hub.guests.get(gi)?.carried.clone();
                (now == carried).then(|| same.clone())
            },
            |game, answer| {
                if game
                    .guest_menu_op(gi, &mut copy, |menu, env| {
                        plan_click(menu, env, slot, button, &answers, answer);
                    })
                    .is_none()
                {
                    answer.refused.set(true);
                }
            },
            |game, answer| {
                game.journal_guest_click(conn);
                let rules = StoredRules(&answers);
                ran = action(game, menu, Some(&rules), answer);
            },
        );
        match pickup {
            Pickup::Ran => ran,
            Pickup::Refused => None,
        }
    }

    /// Click `p` on the guest `gi`'s `menu`, after `prepare` has set the menu up as the click finds
    /// it, and say which menu slots the click changed (`None` when the guest is gone or the click
    /// was refused).
    pub(super) fn guest_menu_click(
        &mut self,
        gi: usize,
        menu: &mut Menu,
        p: &play::ContainerClick,
        prepare: impl Fn(&mut Menu, &Env),
    ) -> Option<Vec<usize>> {
        let (slot, button, mode) = click_of(p);
        let prepare: &dyn Fn(&mut Menu, &Env) = &prepare;
        self.guest_menu_op_decided(
            gi,
            menu,
            Some((slot, button, mode)),
            Some(prepare),
            |menu, env| {
                prepare(menu, env);
                let before: Vec<_> = (0..menu.slot_count())
                    .map(|s| menu.slot(s).cloned())
                    .collect();
                let _ = menu.click(env, slot, button, mode);
                (0..menu.slot_count())
                    .filter(|&s| menu.slot(s) != before[s].as_ref())
                    .collect()
            },
        )
    }
}
