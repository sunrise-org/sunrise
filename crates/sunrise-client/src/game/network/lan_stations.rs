//! Workstation menus for LAN guests: the screen a guest's use opens runs on
//! the host's menu engine (`sunrise_client::menu`), the one single player
//! uses, with vanilla's packets: OpenScreen with the station's `MenuType`,
//! container clicks, `ServerboundContainerButtonClickPacket`, the content and
//! data slot sync after each (`broadcastChanges`), and the inputs back into
//! the inventory when the menu closes (`AbstractContainerMenu.clearContainer`).
use super::{
    Game,
    stations::{Kind as StationKind, Station},
};
use crate::game::{
    api_values::TryIntoApi,
    decided_action::Epoch,
    transactions::{After, decision::Reply},
};
use mod_api::item::menu::AnvilCraftInputs;
use std::cell::{Cell, RefCell};
use sunrise_client::{
    containers::BlockEntity,
    items::ItemStack,
    menu::{GrindstonePlaceEdits, GrindstoneTakeEdits, Kind, Menu},
};
use sunrise_core::random::RandomSource;
use sunrise_protocol::packets::play::{self, Clientbound as C};

/// A guest's open workstation menu.
#[derive(Clone)]
pub(super) struct GuestStation {
    pub(super) window: i32,
    state: i32,
    pub(super) kind: StationKind,
    pub(super) pos: [i32; 3],
    pub(super) menu: Box<Menu>,
    /// The data slots the client was last sent. `None` is not sent yet,
    /// which is not a cost: an anvil's signed cost may be `i32::MIN`.
    sent_data: Vec<Option<i32>>,
    /// The game time of the menu's last sound (`lastSoundTime`).
    last_sound: u64,
}

/// A guest's open lectern: `LecternMenu` over the block entity, made on the
/// menu engine for each click (`Game::lectern_menu`).
pub(super) struct GuestLectern {
    pub(super) window: i32,
    state: i32,
    pub(super) key: (i32, i32, i32),
    /// The book and page the client was last sent (`broadcastChanges`).
    sent: Option<(Option<ItemStack>, i32)>,
}

/// The `MenuType`, the engine's menu and the input slots
/// (`clearContainer`'s container) of a station a guest can use.
pub(super) fn menu_of(kind: StationKind) -> Option<(&'static str, Kind, std::ops::Range<usize>)> {
    Some(match kind {
        StationKind::Loom => ("minecraft:loom", Kind::Loom, 0..3),
        StationKind::Stonecutter => ("minecraft:stonecutter", Kind::Stonecutter, 0..1),
        StationKind::Grindstone => ("minecraft:grindstone", Kind::Grindstone, 0..2),
        StationKind::Anvil => ("minecraft:anvil", Kind::Anvil, 0..2),
        StationKind::Smithing => ("minecraft:smithing", Kind::Smithing, 0..3),
        StationKind::Enchanting => ("minecraft:enchantment", Kind::Enchanting, 0..2),
        StationKind::Beacon => ("minecraft:beacon", Kind::Beacon, 0..1),
        StationKind::Cartography => ("minecraft:cartography_table", Kind::Cartography, 0..2),
        _ => return None,
    })
}

impl Game<'_> {
    /// A reload changes what a stonecutter button number means. Close that window
    /// before publishing the new recipe generation so queued old buttons cannot act on it.
    pub(super) fn close_stonecutters_for_reload(&mut self) {
        let Some(host) = self.lan.as_ref() else {
            return;
        };
        let open: Vec<_> = host
            .menus
            .iter()
            .filter_map(|(&conn, menus)| {
                menus
                    .station
                    .as_ref()
                    .filter(|menu| menu.kind == StationKind::Stonecutter)
                    .map(|_| conn)
            })
            .collect();
        for conn in open {
            self.guest_close_menus_and_tell(conn);
        }
    }

    /// A workstation a guest's use opened (`ServerPlayer.openMenu`); false
    /// when guests can't use that one yet.
    pub(super) fn lan_open_station(&mut self, conn: u32, station: &Station) -> bool {
        let Some((menu_type, kind, _)) = menu_of(station.kind) else {
            return false;
        };
        // The hub may already have the new button map while native advancement
        // hooks are still ahead of UpdateRecipes. Do not open on an unpublished map.
        if kind == Kind::Stonecutter
            && self
                .lan
                .as_ref()
                .is_some_and(|host| host.hub.native_reload_pending)
        {
            return true;
        }
        // The mods may stop the menu; the click is spent.
        if self.guest_may_open(conn, menu_type, Some(station.pos)) != Some(true) {
            return true;
        }
        // ServerPlayer.openMenu closes the menu that was open.
        self.guest_close_all(conn);
        let title = sunrise_protocol::text::translatable(station.kind.title_key(), &[]);
        let Some(window) = self
            .lan
            .as_mut()
            .and_then(|host| host.hub.open_window(conn, menu_type, title))
        else {
            return true;
        };
        let mut menu = Box::new(Menu::new(kind));
        menu.set_registry_id(Some(menu_type));
        if kind == Kind::Stonecutter {
            menu.stone.server_buttons = self
                .lan
                .as_ref()
                .map(|host| host.hub.sync_data().stonecutter_buttons.clone());
        }
        // Without a mod the take runs AnvilMenu.onTake's order in the click itself.
        let hooked = !mod_api::__private::mods().is_empty();
        menu.anvil_hook = kind == Kind::Anvil && hooked;
        // Likewise GrindstoneMenu's result onTake.
        menu.grind_hook = kind == Kind::Grindstone && hooked;
        menu.item_smithed_hook = kind == Kind::Smithing && hooked;
        if kind == Kind::Enchanting {
            // Player.enchantmentSeed, made when first needed.
            let needs_seed = self
                .lan
                .as_ref()
                .and_then(|host| host.hub.guests.iter().find(|g| g.conn == conn))
                .is_some_and(|g| g.xp_seed == 0);
            let drawn = needs_seed.then(|| self.world.random.next_int());
            if let Some(host) = self.lan.as_mut() {
                let _ = host.hub.update_guest(conn, |g| {
                    if let Some(drawn) = drawn {
                        g.xp_seed = drawn;
                    }
                    menu.data[3] = g.xp_seed;
                });
            }
        }
        if kind == Kind::Beacon {
            menu.data = self.beacon_data(station.pos).to_vec();
        }
        if let Some(host) = self.lan.as_mut() {
            *host.menus.entry(conn).or_default().station = Some(GuestStation {
                window,
                state: 0,
                kind: station.kind,
                pos: station.pos,
                menu,
                sent_data: Vec::new(),
                last_sound: 0,
            });
        }
        self.lan_send_station(conn);
        self.player_menu_opening(conn);
        true
    }

    /// Runs `op` on the guest's open station (window `window`, any when
    /// None) with their inventory in it, then sends what changed. False when
    /// no such window is open.
    fn lan_station_op(
        &mut self,
        conn: u32,
        window: Option<i32>,
        op: impl FnOnce(&mut Menu, &sunrise_client::menu::Env),
    ) -> bool {
        self.lan_station_op_clicked(conn, window, None, op)
    }

    /// `lan_station_op` for `op` that makes `click`, which the mods decide first when it is a
    /// pickup they listen to (`guest_menu_op_decided`).
    fn lan_station_op_clicked(
        &mut self,
        conn: u32,
        window: Option<i32>,
        click: Option<super::lan_pickup::Click>,
        op: impl FnOnce(&mut Menu, &sunrise_client::menu::Env),
    ) -> bool {
        let Some((owner, mut gs)) = self
            .lan
            .as_mut()
            .and_then(|host| host.menus.get_mut(&conn))
            .and_then(|p| {
                let owner = p.station.id();
                Some((
                    owner,
                    p.station
                        .take_if(|s| window.is_none_or(|w| s.window == w))?,
                ))
            })
        else {
            return false;
        };
        let guest = self
            .lan
            .as_ref()
            .and_then(|host| host.hub.guests.iter().position(|g| g.conn == conn))
            .filter(|&gi| {
                !self
                    .lan
                    .as_ref()
                    .is_some_and(|h| h.hub.guests[gi].spectator())
            });
        let mut click_seal = None;
        let mut commit_click = true;
        if let Some(gi) = guest {
            // createResult runs when the inputs change (or a rename): a click
            // elsewhere must not hand the hooks their own result again.
            let result_slot = match gs.kind {
                StationKind::Anvil | StationKind::Grindstone => Some(2),
                StationKind::Smithing => Some(3),
                _ => None,
            };
            let before = result_slot.map(|r| {
                let slots: Vec<_> = (0..=r).map(|s| gs.menu.slot(s).cloned()).collect();
                // Only the anvil has a data slot (its cost) and a name; grindstone and
                // smithing menus have neither. A rename runs createResult even when the
                // result comes out the same (AnvilMenu.setItemName).
                (
                    slots,
                    gs.menu.data.first().copied(),
                    gs.menu.anvil.name.clone(),
                )
            });
            let mut undo = None;
            let hook = gs.menu.anvil_hook;
            // A take the mods listen to is one action with its click: the click, the hook and the
            // take's own effects share one transaction, so a refusal puts the guest's inventory,
            // cursor and drops back with the menu as it was (`before`).
            let take_listened =
                gs.menu.grind_hook && mod_api::__private::listened(mod_api::HookId::GrindstoneTake);
            let place_listened = gs.menu.grind_hook
                && mod_api::__private::listened(mod_api::HookId::GrindstonePlace);
            let pos = gs.pos;
            // Bind the owner and backing before running any part of the click. The grindstone
            // has no block entity, so use the level's committed and tentative block epochs.
            let admission = (take_listened || place_listened)
                .then(|| {
                    Some((
                        self.guest_actor(conn)?,
                        self.guest_station(conn)?,
                        self.grindstone_backing(pos),
                    ))
                })
                .flatten();
            let fresh = |game: &Game<'_>| {
                admission.is_some_and(|(_, (_, stamp), backing)| {
                    game.guest_station(conn)
                        .is_some_and(|(_, now)| now == stamp)
                        && game.grindstone_backing(pos) == backing
                })
            };
            if gs.kind == StationKind::Grindstone
                && (take_listened || !self.transactions.is_empty())
            {
                click_seal = Some(self.transaction_open());
                self.journal_guest_click(conn);
                self.journal_station_click(conn, owner, &gs);
            }
            self.guest_menu_op_decided(gi, &mut gs.menu, click, None, |menu, env| {
                undo = hook.then(|| menu.anvil_undo());
                op(menu, env);
            });
            // Both anvil hooks borrow the same converted snapshot, even after the
            // take changes the slots. The deferred take must run even if capture fails.
            let pending = gs.menu.anvil_craft.take();
            let deferred_take = pending.is_some();
            let pre_listened =
                deferred_take && mod_api::__private::listened(mod_api::HookId::AnvilCraftPre);
            let post_listened =
                deferred_take && mod_api::__private::listened(mod_api::HookId::AnvilCraftPost);
            let snapshot = pending
                .filter(|_| pre_listened || post_listened)
                .and_then(|craft| craft.try_into_api().ok());
            let inputs = snapshot
                .as_ref()
                .map(|(left, right, output)| AnvilCraftInputs {
                    left: left.as_ref(),
                    right: right.as_ref(),
                    output,
                });
            let mut cancelled = false;
            if deferred_take {
                let mut cancel = false;
                if pre_listened && let Some(inputs) = &inputs {
                    let (uuid, name) = self
                        .lan
                        .as_ref()
                        .map(|host| {
                            let guest = &host.hub.guests[gi];
                            (guest.uuid, guest.name.clone())
                        })
                        .unwrap_or((0, String::new()));
                    self.native_anvil_craft_pre(uuid, &name, gs.pos, inputs, &mut cancel);
                }
                if cancel && let Some(undo) = undo {
                    // Restore the result, inputs, cost, inventory and cursor.
                    // The cancelled take drops nothing and is resynced below.
                    cancelled = true;
                    self.guest_menu_op(gi, &mut gs.menu, |menu, _| menu.anvil_cancel(undo));
                } else {
                    gs.menu.anvil_take(&self.menu_env());
                    let dropped = std::mem::take(&mut gs.menu.dropped);
                    self.drop_guest_items(gi, dropped);
                }
            }
            // GrindstoneEvent.OnTakeItem: the taken result's onTake waits for the mods.
            let mut ground_xp = None;
            let taken = gs.menu.grind_take.take();
            let (uuid, entity_id) = self
                .lan
                .as_ref()
                .map(|host| (host.hub.guests[gi].uuid, host.hub.guests[gi].entity_id))
                .unwrap_or((0, 0));
            if let Some(take) = taken {
                if take_listened && let Some((actor, (route, _), _)) = admission {
                    let read = gs.menu.mutation_stamp();
                    let answer = RefCell::new(None::<GrindstoneTakeEdits>);
                    let listener_cancelled = Cell::new(false);
                    let applied = {
                        let menu = RefCell::new(&mut *gs.menu);
                        self.decided_action(
                            "grindstone_take",
                            route,
                            actor,
                            Epoch::Player,
                            // The station is the one the click ran on, its block is as it was and
                            // nothing the take read has changed.
                            |game| fresh(game) && menu.borrow().mutation_stamp() == read,
                            |game| {
                                let mut cancel = false;
                                let edits = match game.api_grindstone_take(
                                    uuid,
                                    entity_id,
                                    pos,
                                    &take,
                                    &mut cancel,
                                ) {
                                    Ok(edits) => edits,
                                    Err(reason) => return Reply::Invalid(reason),
                                };
                                if cancel {
                                    listener_cancelled.set(true);
                                    return Reply::Cancel;
                                }
                                *answer.borrow_mut() = Some(edits);
                                Reply::Allow
                            },
                            |game| {
                                let Some(edits) = answer.take() else {
                                    return;
                                };
                                if edits.xp < 0 {
                                    let amount = take.vanilla_amount(&game.menu_env());
                                    game.grindstone_taken(pos, amount);
                                } else {
                                    game.grindstone_award(pos, edits.xp.unsigned_abs());
                                }
                                menu.borrow_mut()
                                    .grindstone_take_inputs(&game.menu_env(), edits);
                            },
                        )
                    };
                    // A listener's cancel keeps the click (the result is still taken) and skips
                    // the rest of onTake; any other refusal undoes the click as well.
                    cancelled |= !applied;
                    commit_click = applied || listener_cancelled.get();
                } else if take_listened {
                    // An unidentifiable actor must not dispatch a listened hook outside an action.
                    cancelled = true;
                    commit_click = false;
                } else {
                    ground_xp = gs.menu.grindstone_taken(
                        &self.menu_env(),
                        &take,
                        GrindstoneTakeEdits::initial(take.xp),
                    );
                }
            }
            // A cancelled take runs no createResult: the inputs are untouched.
            let recomputed = !cancelled
                && before.is_some_and(|(slots, cost, item_name)| {
                    cost != gs.menu.data.first().copied()
                        || item_name != gs.menu.anvil.name
                        || slots
                            .iter()
                            .enumerate()
                            .any(|(s, b)| gs.menu.slot(s) != b.as_ref())
                });
            // AnvilUpdateEvent after createResult; a cancelled take's undo
            // is no createResult.
            let anvil_created =
                gs.kind == StationKind::Anvil && gs.menu.anvil_created() && !cancelled;
            if anvil_created
                && mod_api::__private::listened(mod_api::HookId::AnvilUpdate)
                && let Some((inputs, mut result)) = gs.menu.anvil_update()
            {
                let (uuid, entity_id) = self
                    .lan
                    .as_ref()
                    .map(|host| (host.hub.guests[gi].uuid, host.hub.guests[gi].entity_id))
                    .unwrap_or((0, 0));
                self.api_anvil_update(uuid, entity_id, gs.pos, inputs, &mut result);
                gs.menu.anvil_updated(result);
            }
            if recomputed && gs.kind == StationKind::Grindstone && gs.menu.grind_hook {
                let (uuid, entity_id) = self
                    .lan
                    .as_ref()
                    .map(|host| (host.hub.guests[gi].uuid, host.hub.guests[gi].entity_id))
                    .unwrap_or((0, 0));
                if place_listened && let Some((actor, (route, _), _)) = admission {
                    let read = gs.menu.mutation_stamp();
                    let answer = RefCell::new(None::<GrindstonePlaceEdits>);
                    let applied = {
                        let menu = RefCell::new(&mut *gs.menu);
                        self.decided_action(
                            "grindstone_place",
                            route,
                            actor,
                            Epoch::Player,
                            |game| fresh(game) && menu.borrow().mutation_stamp() == read,
                            |game| {
                                let mut cancel = false;
                                let edits = match game.api_grindstone_place(
                                    uuid,
                                    entity_id,
                                    pos,
                                    menu.borrow().grindstone_place(),
                                    &mut cancel,
                                ) {
                                    Ok(edits) => edits,
                                    Err(reason) => return Reply::Invalid(reason),
                                };
                                if cancel {
                                    return Reply::Cancel;
                                }
                                *answer.borrow_mut() = Some(edits);
                                Reply::Allow
                            },
                            |_| {
                                if let Some(edits) = answer.take() {
                                    menu.borrow_mut().grindstone_placed(edits, false);
                                }
                            },
                        )
                    };
                    // A cancel, a refusal and a stale read all leave no result.
                    if !applied {
                        gs.menu.grindstone_placed(Default::default(), true);
                    }
                } else {
                    // Without a subscriber use the vanilla result. A missing listener identity
                    // on the subscribed route instead refuses it, just like a stale read.
                    gs.menu
                        .grindstone_placed(Default::default(), place_listened);
                }
            }
            if recomputed && gs.kind == StationKind::Smithing {
                let (uuid, name) = self
                    .lan
                    .as_ref()
                    .map(|host| {
                        let guest = &host.hub.guests[gi];
                        (guest.uuid, guest.name.clone())
                    })
                    .unwrap_or((0, String::new()));
                let mut result = gs.menu.slot(3).cloned();
                self.native_prepare_smithing(uuid, &name, gs.pos, &mut result);
                let env = self.menu_env();
                gs.menu.set_slot(&env, 3, result);
            }
            self.lan_station_sounds(&mut gs);
            if let Some(amount) = gs.menu.ground.take() {
                self.grindstone_taken(gs.pos, amount);
            }
            if let Some(xp) = ground_xp {
                self.grindstone_award(gs.pos, xp.unsigned_abs());
            }
            // ItemSmithedEvent comes before the rest of SmithingMenu.onTake.
            let smithed = std::mem::take(&mut gs.menu.item_smithed);
            if let Some((uuid, entity_id)) = self
                .lan
                .as_ref()
                .map(|host| (host.hub.guests[gi].uuid, host.hub.guests[gi].entity_id))
            {
                for (inputs, result) in &smithed {
                    self.native_player_item_smithed(uuid, entity_id, inputs, result);
                }
            }
            let post_inputs = inputs.as_ref().filter(|_| post_listened && !cancelled);
            self.lan_station_combined(conn, &mut gs, post_inputs);
            if let Some(button) = gs.menu.enchant_button.take() {
                self.lan_enchant(gi, &mut gs, button);
            }
            if std::mem::take(&mut gs.menu.enchant_changed) {
                self.lan_enchant_offers(gi, &mut gs);
            }
        }
        let window = gs.window;
        let grindstone = gs.kind == StationKind::Grindstone;
        if let Some(menus) = self.lan.as_mut().and_then(|host| host.menus.get_mut(&conn))
            && menus.station.id() == owner
            && menus.station.is_none()
        {
            *menus.station = Some(gs);
        }
        if let Some(seal) = click_seal {
            self.transaction_close(&seal, commit_click);
        }
        if grindstone {
            self.after_commit(After::StationContent(conn, owner, window));
        } else {
            self.lan_send_station(conn);
        }
        true
    }

    fn grindstone_backing(
        &self,
        [x, y, z]: [i32; 3],
    ) -> (u64, u64, sunrise_world::registry::BlockStateId) {
        (
            self.world.revision,
            self.transactions.block_writes(),
            self.world.block(x, y, z),
        )
    }

    /// How many slots of its own the guest's open station menu has before the
    /// player's inventory (`slot_count - 36`), if one is open.
    pub(super) fn lan_station_own_slots(&self, conn: u32) -> Option<usize> {
        let gs = self.lan.as_ref()?.menus.get(&conn)?.station.as_ref()?;
        Some(gs.menu.slot_count() - 36)
    }

    /// `LecternBlock.openScreen` for a guest: `minecraft:lectern` over the
    /// block entity, with its book and page.
    pub(super) fn lan_open_lectern(&mut self, conn: u32, key: (i32, i32, i32)) {
        if self.guest_may_open(conn, "minecraft:lectern", Some([key.0, key.1, key.2])) != Some(true)
        {
            return;
        }
        // ServerPlayer.openMenu closes the menu that was open.
        self.guest_close_all(conn);
        let title = sunrise_protocol::text::translatable("container.lectern", &[]);
        let Some(window) = self
            .lan
            .as_mut()
            .and_then(|host| host.hub.open_window(conn, "minecraft:lectern", title))
        else {
            return;
        };
        if let Some(host) = self.lan.as_mut() {
            host.play.entry(conn).or_default().lectern = Some(GuestLectern {
                window,
                state: 0,
                key,
                sent: None,
            });
        }
        self.lan_send_lectern(conn, true);
        self.player_menu_opening(conn);
    }

    /// The lectern's book and page for the client: everything (`all`), or
    /// what changed since it was last sent.
    fn lan_send_lectern(&mut self, conn: u32, all: bool) {
        let Some((key, window, sent)) = self
            .lan
            .as_ref()
            .and_then(|host| host.play.get(&conn))
            .and_then(|p| p.lectern.as_ref())
            .map(|gl| (gl.key, gl.window, &gl.sent))
        else {
            return;
        };
        let (book, page) = match self.world.block_entities.get(&key) {
            Some(BlockEntity::Lectern(l)) => (&l.book, l.page),
            _ => (&None, 0),
        };
        if !all
            && sent
                .as_ref()
                .is_some_and(|(old, p)| old == book && *p == page)
        {
            return;
        }
        let page_changed = sent.as_ref().is_none_or(|(_, p)| *p != page);
        let book = book.clone();
        let now = Some((book.clone(), page));
        if let Some(host) = self.lan.as_mut()
            && let Some(gi) = host.hub.guests.connection_index(conn)
            && let Some(gl) = host.play.get_mut(&conn).and_then(|p| p.lectern.as_mut())
        {
            gl.sent = now;
            gl.state = (gl.state + 1) & 0x7FFF;
            host.hub.send_menu_snapshot(gi, window, gl.state, &[book]);
            if all || page_changed {
                host.hub.send_to(
                    conn,
                    &C::ContainerSetData(play::ContainerSetData {
                        container: window,
                        id: 0,
                        value: page as i16,
                    }),
                );
            }
        }
    }

    /// Runs `op` on the guest's lectern menu (window `window`) with their
    /// inventory, the block entity takes the book and page back, and the
    /// client gets what changed. False when no such window is open.
    fn lan_lectern_op(
        &mut self,
        conn: u32,
        window: i32,
        op: impl FnOnce(&mut Menu, &sunrise_client::menu::Env),
    ) -> bool {
        self.lan_lectern_op_clicked(conn, window, None, op)
    }

    /// `lan_lectern_op` for `op` that makes `click` (see `lan_station_op_clicked`).
    fn lan_lectern_op_clicked(
        &mut self,
        conn: u32,
        window: i32,
        click: Option<super::lan_pickup::Click>,
        op: impl FnOnce(&mut Menu, &sunrise_client::menu::Env),
    ) -> bool {
        let Some(key) = self
            .lan
            .as_ref()
            .and_then(|host| host.play.get(&conn))
            .and_then(|p| p.lectern.as_ref())
            .filter(|l| l.window == window)
            .map(|l| l.key)
        else {
            return false;
        };
        let guest = self
            .lan
            .as_ref()
            .and_then(|host| host.hub.guests.iter().position(|g| g.conn == conn))
            .filter(|&gi| {
                !self
                    .lan
                    .as_ref()
                    .is_some_and(|h| h.hub.guests[gi].spectator())
            });
        if let Some(gi) = guest
            && let Some(mut menu) = self.lectern_menu(key)
        {
            // Player.mayBuild: survival and creative.
            let may_build = self
                .lan
                .as_ref()
                .is_some_and(|host| matches!(host.hub.guests[gi].game_mode, 0 | 1));
            self.guest_menu_op_decided(gi, &mut menu, click, None, |menu, env| {
                menu.may_build = may_build;
                op(menu, env);
            });
            self.lectern_menu_done(key, &menu);
        }
        self.lan_send_lectern(conn, true);
        true
    }

    /// A click on the guest's lectern window; false for other windows.
    pub(super) fn lan_lectern_click(&mut self, conn: u32, p: &play::ContainerClick) -> bool {
        let click = super::lan_pickup::click_of(p);
        self.lan_lectern_op_clicked(conn, p.container, Some(click), |menu, env| {
            let _ = menu.click(env, click.0, click.1, click.2);
        })
    }

    /// The rest of a taken anvil or smithing result's `onTake`: the guest
    /// pays the levels, the anvil wears (level event 1030 or 1029), the
    /// smithing table sounds (1044). An anvil's pre-take inputs and result are
    /// then reported to the mods (`AnvilCraftEvent.Post`).
    fn lan_station_combined(
        &mut self,
        conn: u32,
        gs: &mut GuestStation,
        inputs: Option<&AnvilCraftInputs<'_>>,
    ) {
        let change = std::mem::take(&mut gs.menu.level_change);
        if change != 0 {
            self.as_guest(conn, |g| g.give_levels(change));
        }
        if !std::mem::take(&mut gs.menu.combined) {
            return;
        }
        if gs.kind == StationKind::Smithing {
            self.level_event_at(1044, gs.pos, 0);
        } else {
            let creative = gs.menu.creative;
            // A broken anvil closes the menu on the next stillValid check.
            self.anvil_used(gs.pos, creative);
            if let Some(inputs) = inputs
                && let Some((uuid, name)) = self
                    .lan
                    .as_ref()
                    .and_then(|host| host.hub.guests.iter().find(|g| g.conn == conn))
                    .map(|guest| (guest.uuid, guest.name.clone()))
            {
                self.native_anvil_craft_post(uuid, &name, gs.pos, inputs);
            }
        }
    }

    /// `EnchantmentMenu.slotsChanged` with a level: the costs and clues for
    /// the guest's seed and the bookshelves around the table.
    fn lan_enchant_offers(&mut self, gi: usize, gs: &mut GuestStation) {
        let (seed, guest_uuid, guest_entity_id, guest_name) = self
            .lan
            .as_ref()
            .map(|host| {
                let guest = &host.hub.guests[gi];
                (
                    guest.xp_seed,
                    guest.uuid,
                    guest.entity_id,
                    guest.name.clone(),
                )
            })
            .unwrap_or((0, 0, 0, String::new()));
        let bookcases = self.count_bookshelves(gs.pos);
        let offers = if mod_api::__private::mods().is_empty() {
            sunrise_client::stations::enchant_offers(
                &self.stations().enchantments,
                self.items(),
                seed,
                bookcases,
                gs.menu.slot(0),
            )
        } else {
            // EnchantmentLevelSetEvent for each row as getEnchantmentCost drew
            // it; the offers are then made with the levels the mods left.
            let mut drawn = [None; 3];
            sunrise_client::stations::enchant_offers_leveled(
                &self.stations().enchantments,
                self.items(),
                seed,
                bookcases,
                gs.menu.slot(0),
                |row, cost| *drawn[row].insert(cost),
            );
            let mut levels = drawn.map(|cost| cost.unwrap_or(0));
            if let Some(item) = gs
                .menu
                .slot(0)
                .and_then(|item| super::api_values::TryIntoApi::try_into_api(item).ok())
            {
                for (row, original) in drawn.iter().enumerate() {
                    if let Some(original_level) = *original {
                        let offer = mod_api::item::stations::EnchantmentRow {
                            player: guest_uuid,
                            player_entity_id: guest_entity_id,
                            row: row as i32,
                            power: bookcases,
                            item: &item,
                            original_level,
                        };
                        self.native_enchantment_level_set(gs.pos, &offer, &mut levels[row]);
                    }
                }
            }
            sunrise_client::stations::enchant_offers_leveled(
                &self.stations().enchantments,
                self.items(),
                seed,
                bookcases,
                gs.menu.slot(0),
                |row, _| levels[row],
            )
        };
        let mut costs = offers.costs;
        self.native_prepare_item_enchant(guest_uuid, &guest_name, gs.pos, &mut costs);
        let mut clues = offers.clues;
        for i in 0..3 {
            // A cost a mod changed shows the clue of the list it will give.
            if costs[i] != offers.costs[i]
                && let Some(stack) = gs.menu.slot(0)
            {
                clues[i] = sunrise_client::stations::enchant_clue(
                    &self.stations().enchantments,
                    self.items(),
                    seed,
                    i,
                    costs[i],
                    stack,
                );
            }
        }
        for i in 0..3 {
            gs.menu.data[i] = costs[i];
            let clue = clues[i].as_ref();
            gs.menu.data[4 + i] = clue
                .and_then(|(e, _)| {
                    self.lan
                        .as_ref()
                        .and_then(|host| host.hub.registry_id("minecraft:enchantment", e))
                })
                .unwrap_or(-1);
            gs.menu.data[7 + i] = clue.map_or(-1, |(_, l)| *l as i32);
        }
        gs.menu.data[3] = seed;
    }

    /// `EnchantmentMenu.clickMenuButton`'s `access.execute`: the item gets
    /// the offer's enchantments, the guest pays its levels and lapis and
    /// gets a new seed, the table sounds, and the offers are made again.
    fn lan_enchant(&mut self, gi: usize, gs: &mut GuestStation, button: usize) {
        let Some(item) = gs.menu.slot(0).cloned() else {
            return;
        };
        let Some((seed, creative, conn, uuid, entity_id, name)) = self.lan.as_ref().map(|host| {
            let g = &host.hub.guests[gi];
            (
                g.xp_seed,
                g.creative(),
                g.conn,
                g.uuid,
                g.entity_id,
                g.name.clone(),
            )
        }) else {
            return;
        };
        let Some(enchanted) = self.table_enchant(seed, button, gs.menu.data[button], &item) else {
            return;
        };
        let enchanted_for_hook = enchanted.clone();
        let need = button as i32 + 1;
        // The enchantments the item got, for PlayerEnchantItemEvent.
        let given = mod_api::__private::listened(mod_api::HookId::PlayerEnchantItem).then(|| {
            sunrise_client::stations::enchantment_list(
                &self.stations().enchantments,
                self.items(),
                seed,
                button,
                gs.menu.data[button],
                &item,
            )
        });
        // Player.onEnchantmentPerformed, and EnchantmentMenu's awardStat(ENCHANT_ITEM).
        self.as_guest(conn, |game| {
            game.enchantment_performed(need);
            game.award_custom_stat("enchant_item", 1);
            if let Some(given) = &given {
                game.native_player_enchant_item(uuid, entity_id, &enchanted, given);
            }
            // CriteriaTriggers.ENCHANTED_ITEM, with the levels it cost.
            game.trigger_enchanted_item(&enchanted, need);
        });
        let new_seed = self.world.random.next_int();
        if let Some(host) = self.lan.as_mut() {
            {
                let value = new_seed;
                host.hub.guests.update(gi, |guest| guest.xp_seed = value);
            }
        }
        let env = self.menu_env();
        let lapis = gs.menu.slot(1).cloned().filter(|_| !creative).map(|mut l| {
            l.count = l.count.saturating_sub(need as u32);
            l
        });
        gs.menu.set_slot(&env, 0, Some(enchanted));
        if !creative {
            gs.menu.set_slot(&env, 1, lapis.filter(|l| l.count > 0));
        }
        gs.menu.enchant_changed = false;
        self.lan_enchant_offers(gi, gs);
        let pitch = self.world.random.next_float() * 0.1 + 0.9;
        let at = gs.pos.map(|v| f64::from(v) + 0.5);
        self.audio.play(
            "minecraft:block.enchantment_table.use",
            Some(at),
            1.0,
            pitch,
        );
        self.native_enchant_item(uuid, &name, gs.pos, &enchanted_for_hook, need);
    }

    /// `ServerGamePacketListenerImpl.handleRenameItem`: the guest's anvil
    /// takes the name (`AnvilMenu.setItemName`).
    pub(super) fn lan_station_rename(&mut self, conn: u32, name: &str) {
        let anvil = self
            .lan
            .as_ref()
            .and_then(|host| host.menus.get(&conn))
            .and_then(|p| p.station.as_ref())
            .is_some_and(|s| s.kind == StationKind::Anvil);
        if anvil {
            self.lan_station_op(conn, None, |menu, env| {
                menu.rename(env, name);
            });
        }
    }

    /// The sounds the menu played, at the station (heard by everyone around,
    /// as `level.playSound(null, pos, ...)`), once per tick.
    fn lan_station_sounds(&mut self, gs: &mut GuestStation) {
        let sounds = std::mem::take(&mut gs.menu.sounds);
        if sounds.is_empty() || gs.last_sound == self.ticks {
            return;
        }
        gs.last_sound = self.ticks;
        let at = gs.pos.map(|v| f64::from(v) + 0.5);
        // The menu's `lastSoundTime` lets one sound through per tick, however
        // many results a shift-click took.
        self.audio.play(sounds[0], Some(at), 1.0, 1.0);
    }

    /// A click on the guest's station window; false for other windows.
    pub(super) fn lan_station_click(&mut self, conn: u32, p: &play::ContainerClick) -> bool {
        let click = super::lan_pickup::click_of(p);
        self.lan_station_op_clicked(conn, Some(p.container), Some(click), |menu, env| {
            let _ = menu.click(env, click.0, click.1, click.2);
        })
    }

    /// `ServerGamePacketListenerImpl.handleContainerButtonClick`:
    /// `clickMenuButton`, then `broadcastChanges` when the menu took it.
    pub(super) fn lan_station_button(&mut self, conn: u32, window: i32, id: i32) {
        // Route to the active menu type's listeners. Only a validated `Slots::Block`
        // supplies the optional position and entity backing.
        let mod_menu = self.lan.as_ref().and_then(|host| {
            let g = host.hub.guest(conn)?;
            let menu = g.menu.as_ref().filter(|m| m.window == window)?;
            if !mod_api::__private::menu_listens(mod_api::HookId::MenuButton, &menu.registry_id) {
                return None;
            }
            Some((
                menu.registry_id.clone(),
                g.menu_generation,
                match menu.slots {
                    sunrise_client::lan::menu::Slots::Block(pos) => Some(pos),
                    _ => None,
                },
                !g.spectator() && menu.still_valid(&host.hub.loop_context, &self.world, g),
                g.carried.clone(),
                g.uuid,
                g.entity_id,
            ))
        });
        if let Some((menu, menu_generation, backing, valid, mut carried, uuid, entity_id)) =
            mod_menu
        {
            if valid {
                // The clicking player's cursor stack (`clickMenuButton`'s
                // other half). Single player's own player is a guest of
                // its own host too (`lan.rs:165`), so this one path covers
                // both single player and a LAN guest.
                let original_carried = carried.clone();
                let backing = backing.filter(|&pos| self.mod_menu_at(pos) == Some(menu.as_ref()));
                let block = backing.map(|(x, y, z)| self.data_block_name([x, y, z]));
                let mut accepted = false;
                self.native_menu_button_valid(&menu, block.as_deref(), id, &mut accepted);
                if !accepted {
                    return;
                }
                let context = mod_api::item::menu::MenuButtonContext {
                    player: uuid,
                    player_entity_id: entity_id,
                    pos: backing.map(|(x, y, z)| [x, y, z]),
                    id,
                };
                // As that player: `native_acting_player` and `native_acting_id`
                // answer for the one who pressed.
                self.as_guest(conn, |game| {
                    game.mod_menu_button(&menu, context, block.as_deref(), backing, &mut carried)
                });
                if carried != original_carried
                    && let Some(host) = self.lan.as_mut()
                    && let Some(gi) = host.hub.guests.connection_index(conn)
                {
                    let changed = host.hub.guests.update(gi, |g| {
                        if g.menu_generation != menu_generation
                            || !g
                                .menu
                                .as_ref()
                                .is_some_and(|current| current.window == window)
                            || g.carried != original_carried
                        {
                            return false;
                        }
                        g.carried = carried;
                        true
                    });
                    if changed {
                        host.hub.sync_carried(gi, None);
                    }
                }
            }
            return;
        }
        // LecternMenu's page buttons and Take Book.
        if self.lan_lectern_op(conn, window, |menu, env| {
            menu.button(env, id);
        }) {
            return;
        }
        let identity = self
            .lan
            .as_ref()
            .and_then(|host| host.hub.guest(conn))
            .map(|g| (g.uuid, g.name.clone()));
        let mut selected = false;
        let kind = self
            .lan
            .as_mut()
            .and_then(|host| host.menus.get_mut(&conn))
            .and_then(|p| p.station.as_mut())
            .filter(|s| s.window == window)
            .map(|s| {
                if s.kind == StationKind::Stonecutter
                    && let Some(sent) = s.sent_data.first_mut()
                {
                    // The guest may have predicted a candidate the real stack cannot use.
                    *sent = None;
                }
                s.kind
            });
        self.lan_station_op(conn, Some(window), |menu, env| {
            selected = menu.button(env, id);
        });
        if selected && let Some((player, name)) = identity {
            match kind {
                Some(StationKind::Loom) => {
                    self.native_player_loom_pattern_select(player, &name, id);
                }
                Some(StationKind::Stonecutter) => {
                    self.native_player_stonecutter_recipe_select(player, &name, id);
                }
                _ => {}
            }
        }
    }

    /// The whole station menu and its changed data slots for the client
    /// (`sendAllDataToRemote` / `broadcastChanges`).
    pub(super) fn lan_send_station(&mut self, conn: u32) {
        let Some(host) = self.lan.as_mut() else {
            return;
        };
        let Some(gi) = host.hub.guests.iter().position(|g| g.conn == conn) else {
            return;
        };
        let Some(gs) = host.menus.get_mut(&conn).and_then(|p| p.station.as_mut()) else {
            return;
        };
        gs.state = (gs.state + 1) & 0x7FFF;
        let own = gs.menu.slot_count() - 36;
        let mut items: Vec<Option<ItemStack>> =
            (0..own).map(|s| gs.menu.slot(s).cloned()).collect();
        let g = &host.hub.guests[gi];
        items.extend((9..36).chain(0..9).map(|i| g.inventory[i].clone()));
        let state = gs.state;
        // ContainerSetData's value is a short. An anvil cost outside it
        // would change sign (`-65535` as `+1`), so the signed cost follows
        // on `sunrise:anvil_cost` after that short.
        gs.sent_data.resize(gs.menu.data.len(), None);
        let mut data = Vec::new();
        let mut wide_cost = None;
        let anvil = gs.kind == StationKind::Anvil;
        let window = gs.window;
        for (k, &v) in gs.menu.data.iter().enumerate() {
            if gs.sent_data[k] == Some(v) {
                continue;
            }
            gs.sent_data[k] = Some(v);
            let mut value = v as i16;
            if anvil && k == 0 {
                let synced = sunrise_protocol::anvil_cost::sync(v);
                value = synced.short;
                wide_cost = synced.wide;
            }
            data.push(C::ContainerSetData(play::ContainerSetData {
                container: window,
                id: k as i16,
                value,
            }));
        }
        host.hub.send_menu_snapshot(gi, window, state, &items);
        host.hub.guests.update(gi, |g| g.inventory_told());
        for packet in data {
            host.hub.send_to(conn, &packet);
        }
        if let Some(cost) = wide_cost {
            host.hub.send_to(
                conn,
                &C::CustomPayload(sunrise_protocol::packets::configuration::CustomPayload {
                    channel: sunrise_protocol::anvil_cost::CHANNEL.into(),
                    data: sunrise_protocol::anvil_cost::encode(
                        sunrise_protocol::anvil_cost::Cost { window, cost },
                    ),
                }),
            );
        }
    }

    /// `AbstractContainerMenu.removed` of the guest's station (`window`;
    /// None closes it whichever it is): the carried stack and the inputs go
    /// back to the inventory, what doesn't fit is dropped. False when no such
    /// window is open.
    pub(super) fn lan_station_closed(&mut self, conn: u32, window: Option<i32>) -> bool {
        // A lectern's menu keeps nothing of its own (the book is the block entity's).
        let Some(host) = self.lan.as_mut() else {
            return false;
        };
        let lectern = host
            .play
            .get_mut(&conn)
            .and_then(|p| p.lectern.take_if(|l| window.is_none_or(|w| l.window == w)));
        if let Some(lectern) = lectern {
            // AbstractContainerMenu.removed: the carried stack goes back.
            if let Some(gi) = host.hub.guests.connection_index(conn)
                && let Some(carried) = self.close_guest_menu_cursor(gi, lectern.window)
            {
                self.lan_place_back(gi, vec![carried]);
            }
            if window.is_some() {
                self.player_menu_closing(conn);
                return true;
            }
        }
        let Some(gs) = self
            .lan
            .as_mut()
            .and_then(|host| host.menus.get_mut(&conn))
            .and_then(|p| p.station.take_if(|s| window.is_none_or(|w| s.window == w)))
        else {
            return false;
        };
        let Some(host) = self.lan.as_mut() else {
            return true;
        };
        let Some(gi) = host.hub.guests.connection_index(conn) else {
            return true;
        };
        let carried = self.close_guest_menu_cursor(gi, gs.window);
        let inputs = station_inputs(&gs);
        if gs.kind == StationKind::Beacon {
            // BeaconMenu.removed drops the payment (player.drop).
            self.lan_place_back(gi, carried.into_iter().collect());
            self.drop_guest_items(gi, inputs);
        } else {
            self.lan_place_back(gi, carried.into_iter().chain(inputs).collect());
        }
        if window.is_some() {
            self.player_menu_closing(conn);
        }
        true
    }

    /// `BeaconBlockEntity.dataAccess`: the pyramid's levels and the powers
    /// (`BeaconMenu.encodeEffect`: the `mob_effect` id + 1, 0 for none).
    fn beacon_data(&self, pos: [i32; 3]) -> [i32; 3] {
        let Some(BlockEntity::Beacon(b)) = self.world.block_entities.get(&(pos[0], pos[1], pos[2]))
        else {
            return [0; 3];
        };
        let encode = |e: &Option<String>| {
            e.as_deref()
                .and_then(|n| {
                    self.lan
                        .as_ref()
                        .and_then(|host| host.hub.registry_id("minecraft:mob_effect", n))
                })
                .map_or(0, |id| id + 1)
        };
        [b.levels, encode(&b.primary), encode(&b.secondary)]
    }

    /// `ServerGamePacketListenerImpl.handleSetBeaconPacket`: the guest's
    /// beacon menu takes the powers (`BeaconMenu.updateEffects`), and they
    /// become the block entity's (`dataAccess.set`: the power_select sound
    /// while the beam is up).
    pub(super) fn lan_station_beacon(
        &mut self,
        conn: u32,
        primary: Option<i32>,
        secondary: Option<i32>,
    ) {
        let Some(pos) = self
            .lan
            .as_ref()
            .and_then(|host| host.menus.get(&conn))
            .and_then(|p| p.station.as_ref())
            .filter(|s| s.kind == StationKind::Beacon)
            .map(|s| s.pos)
        else {
            return;
        };
        let Some((player, name)) = self
            .lan
            .as_ref()
            .and_then(|host| host.hub.guests.iter().find(|g| g.conn == conn))
            .map(|g| (g.uuid, g.name.clone()))
        else {
            return;
        };
        // An id the registry doesn't have fails the packet's codec.
        let mut powers: [Option<(i32, String)>; 2] = [None, None];
        for (k, id) in [primary, secondary].into_iter().enumerate() {
            if let Some(id) = id {
                let Some(name) = self
                    .lan
                    .as_ref()
                    .and_then(|host| host.hub.registry_name("minecraft:mob_effect", id))
                else {
                    return;
                };
                powers[k] = Some((id, name.to_owned()));
            }
        }
        let data = self.beacon_data(pos);
        let mut updated = None;
        self.lan_station_op(conn, None, |menu, env| {
            menu.data.copy_from_slice(&data);
            let power = |k: usize| powers[k].as_ref().map(|(id, n)| (*id, n.as_str()));
            updated = Some(menu.beacon_update(env, power(0), power(1)));
        });
        // handleSetBeaconPacket: effects the beacon can't give, or no payment, disconnect.
        if updated == Some(false)
            && let Some(host) = self.lan.as_mut()
        {
            let name = host
                .hub
                .guests
                .iter()
                .find(|g| g.conn == conn)
                .map(|g| g.name.clone());
            eprintln!(
                "Player {} tried to set invalid beacon effects",
                name.unwrap_or_default()
            );
            host.hub.kick(
                conn,
                sunrise_protocol::text::translatable("multiplayer.disconnect.generic", &[]),
            );
        }
        if updated != Some(true) {
            return;
        }
        // Menu validation accepted these registry ids; convert once at the packet boundary.
        let [primary_key, secondary_key] = powers.each_ref().map(|power| {
            power.as_ref().map(|(_, id)| {
                let def = sunrise_client::brewing::effect(id).expect("validated beacon effect");
                mod_api::__private::effect_key_by_index(def.index)
                    .expect("effects resolved at load")
            })
        });
        let [primary, secondary] = powers.map(|p| p.map(|(_, n)| n));
        let mut select_sound = false;
        if let Some(BlockEntity::Beacon(b)) =
            self.world.block_entities.get_mut(&(pos[0], pos[1], pos[2]))
        {
            b.primary = primary.clone();
            b.secondary = secondary.clone();
            select_sound = !b.sections.is_empty();
        }
        self.native_player_change_beacon_effect(player, &name, pos, primary_key, secondary_key);
        if select_sound {
            self.beacon_sound(pos, "minecraft:block.beacon.power_select");
        }
        self.world.modified.insert((pos[0] >> 4, pos[2] >> 4));
    }

    /// `ServerPlayer.doTick`'s `stillValid`: a guest's station closes once its
    /// block is gone or the guest is out of reach (`isWithinBlockInteractionRange(pos, 4)`).
    pub(super) fn lan_stations_tick(&mut self) {
        self.lan_lecterns_tick();
        let conns: Vec<u32> = self
            .lan
            .as_ref()
            .map(|host| {
                host.menus
                    .iter()
                    .filter(|(_, p)| p.station.is_some())
                    .map(|(c, _)| *c)
                    .collect()
            })
            .unwrap_or_default();
        for conn in conns {
            let Some(host) = self.lan.as_ref() else {
                continue;
            };
            let Some(g) = host
                .hub
                .guests
                .iter()
                .find(|g| g.conn == conn && host.hub.here(g))
                .filter(|_| !self.player_tick_canceled(conn))
            else {
                continue;
            };
            let Some(gs) = host.menus.get(&conn).and_then(|p| p.station.as_ref()) else {
                continue;
            };
            let (window, beacon, pos) = (gs.window, gs.kind == StationKind::Beacon, gs.pos);
            let invalid = !self.lan_station_valid(g, gs);
            if invalid {
                self.lan_station_closed(conn, Some(window));
                if let Some(host) = self.lan.as_mut() {
                    host.hub.send_to(
                        conn,
                        &C::ContainerClose(play::ContainerClose { container: window }),
                    );
                }
            } else if beacon {
                // The beacon's data slots follow its block entity.
                let data = self.beacon_data(pos);
                if let Some(gs) = self
                    .lan
                    .as_mut()
                    .and_then(|host| host.menus.get_mut(&conn))
                    .and_then(|p| p.station.as_mut())
                    && gs.kind == StationKind::Beacon
                    && gs.menu.data[..] != data[..]
                {
                    gs.menu.data.copy_from_slice(&data);
                    self.lan_send_station(conn);
                }
            }
        }
    }
}

impl Game<'_> {
    /// A station menu's `stillValid`: its block is still that station and the
    /// guest is within `isWithinBlockInteractionRange(pos, 4)`.
    pub(super) fn lan_station_valid(
        &self,
        g: &sunrise_client::lan::guest::Guest,
        gs: &GuestStation,
    ) -> bool {
        let [x, y, z] = gs.pos;
        let name = &self.res.facts().get(self.world.block(x, y, z)).name;
        StationKind::from_block(name) == Some(gs.kind) && g.within_block_range(gs.pos, 4.0)
    }

    /// `LecternMenu.stillValid` (`Container.stillValidBlockEntity`): the block
    /// entity still holds a book and the guest is within the interaction
    /// range and its buffer.
    pub(super) fn lan_lectern_valid(
        &self,
        g: &sunrise_client::lan::guest::Guest,
        key: (i32, i32, i32),
    ) -> bool {
        matches!(
            self.world.block_entities.get(&key),
            Some(BlockEntity::Lectern(l)) if l.book.is_some()
        ) && g.within_block_range([key.0, key.1, key.2], 4.0)
    }

    /// The guests' lecterns each tick: `stillValid` (the block entity, its
    /// book, the reach) closes them, and `broadcastChanges` sends a book or
    /// page another player changed.
    fn lan_lecterns_tick(&mut self) {
        let Some(host) = self.lan.as_ref() else {
            return;
        };
        let open: Vec<(u32, i32, (i32, i32, i32))> = host
            .play
            .iter()
            .filter_map(|(c, p)| p.lectern.as_ref().map(|l| (*c, l.window, l.key)))
            .collect();
        for (conn, window, key) in open {
            let Some(host) = self.lan.as_ref() else {
                return;
            };
            let Some(g) = host.hub.guests.iter().find(|g| g.conn == conn) else {
                continue;
            };
            if !host.hub.here(g) || self.player_tick_canceled(conn) {
                continue;
            }
            if self.lan_lectern_valid(g, key) {
                self.lan_send_lectern(conn, false);
            } else {
                self.lan_station_closed(conn, Some(window));
                if let Some(host) = self.lan.as_mut() {
                    host.hub.send_to(
                        conn,
                        &C::ContainerClose(play::ContainerClose { container: window }),
                    );
                }
            }
        }
    }
}

/// What `clearContainer` gives back from a station's menu.
pub(super) fn station_inputs(gs: &GuestStation) -> Vec<ItemStack> {
    menu_of(gs.kind)
        .map(|(_, _, inputs)| inputs.filter_map(|s| gs.menu.slot(s).cloned()).collect())
        .unwrap_or_default()
}

#[cfg(test)]
mod anvil_tests {
    use super::*;
    use crate::game::native_guest_turn_tests::with_guests;
    use mod_api::{AnvilCraftPostEvent, AnvilCraftPreEvent, Flow, Hooks, ModEntry, ServerHost};
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    struct Observe(Arc<AtomicUsize>);

    impl Observe {
        fn check_inputs(&self, phase: usize, inputs: &AnvilCraftInputs<'_>) {
            let stack = |s: &mod_api::ItemStack| {
                (
                    mod_api::__private::item_key_id(s.item()),
                    s.count(),
                    s.damage(),
                )
            };
            assert_eq!(
                inputs.left.map(stack),
                Some((Some("minecraft:iron_sword"), 1, 80))
            );
            assert_eq!(
                inputs.right.map(stack),
                Some((Some("minecraft:iron_ingot"), 2, 0))
            );
            assert_eq!(stack(inputs.output), (Some("minecraft:iron_sword"), 1, 0));
            assert_eq!(self.0.load(Ordering::Relaxed), phase);
            self.0.fetch_add(1, Ordering::Relaxed);
        }
    }

    #[mod_api::__subscriptions]
    impl Hooks for Observe {
        fn anvil_craft_pre(
            &self,
            _: &mut dyn ServerHost,
            event: &mut AnvilCraftPreEvent<'_>,
        ) -> Flow {
            self.check_inputs(0, event.inputs);
            Flow::Continue
        }

        fn anvil_craft_post(&self, _: &mut dyn ServerHost, event: &mut AnvilCraftPostEvent<'_>) {
            self.check_inputs(1, event.inputs);
        }
    }

    #[test]
    #[ignore = "daily"]
    fn anvil_pre_and_post_read_pre_take_inputs() {
        const CONN: u32 = 12;
        const POS: [i32; 3] = [1, 64, 1];
        let calls = Arc::new(AtomicUsize::new(0));
        let mods = vec![ModEntry::new(
            "anvil_observer",
            Box::new(Observe(calls.clone())),
        )];
        mod_platform::testing::with_mods(mods.into(), || {
            with_guests(|game| {
                game.world
                    .chunks
                    .insert((0, 0), Arc::new(sunrise_client::chunk::Chunk::empty(0, 0)));
                let [x, y, z] = POS;
                let state = game
                    .pack
                    .assets
                    .registry
                    .default_state("minecraft:anvil")
                    .unwrap();
                game.world.set(x, y, z, state).unwrap();
                game.lan
                    .as_mut()
                    .unwrap()
                    .hub
                    .update_guest(CONN, |guest| guest.xp_level = 10)
                    .unwrap();
                assert!(game.lan_open_station(CONN, &Station::new(StationKind::Anvil, POS)));
                assert!(game.lan_station_op(CONN, None, |menu, env| {
                    let mut sword = ItemStack::new("minecraft:iron_sword", 1);
                    sword.damage = 80;
                    menu.set_slot(env, 0, Some(sword));
                    menu.set_slot(env, 1, Some(ItemStack::new("minecraft:iron_ingot", 2)));
                }));
                let window = game.lan.as_ref().unwrap().menus[&CONN]
                    .station
                    .as_ref()
                    .unwrap()
                    .window;
                game.lan_guest_packet(
                    CONN,
                    play::Serverbound::ContainerClick(play::ContainerClick {
                        container: window,
                        state: 0,
                        slot: 2,
                        button: 0,
                        input: 0,
                        changed: Vec::new(),
                        carried: None,
                    }),
                );

                let host = game.lan.as_ref().unwrap();
                let menu = &host.menus[&CONN].station.as_ref().unwrap().menu;
                assert!((0..=2).all(|slot| menu.slot(slot).is_none()));
                let guest = &host.hub.guests[0];
                let carried = guest.carried.as_ref().expect("the guest took the output");
                assert_eq!(
                    (&*carried.item, carried.count, carried.damage),
                    ("minecraft:iron_sword", 1, 0)
                );
                assert_eq!(guest.xp_level, 8);
                assert_eq!(
                    calls.load(Ordering::Relaxed),
                    2,
                    "Pre and Post both read the pre-take inputs"
                );
            });
        });
    }
}
