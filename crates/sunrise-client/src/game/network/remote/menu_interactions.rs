use super::*;

impl Game<'_> {
    /// `AbstractContainerMenu.setItem`/`initializeContents`: the state id a
    /// click answers with is the open menu's, not another window's.
    pub(in crate::game) fn remote_menu_state(&mut self, container: i32, state: i32) {
        if let Some(r) = self.remote.as_mut() {
            let open = r.menu.as_ref().map_or(0, |m| m.0);
            if container == open {
                r.state_id = state;
            } else if container == 0 {
                r.inventory_state_id = state;
            }
        }
    }

    /// Change the active menu without losing window 0's cursor or state counter.
    pub(in crate::game) fn remote_set_menu(
        &mut self,
        menu: Option<(i32, String)>,
        title: Option<sunrise_client::text::Text>,
    ) {
        let Some(r) = self.remote.as_mut() else {
            return;
        };
        match (r.menu.as_ref().is_some(), menu.is_some()) {
            (false, true) => {
                r.inventory_carried = self.carried.take();
                r.inventory_state_id = std::mem::take(&mut r.state_id);
                for (saved, active) in r.inventory_craft_grid.iter_mut().zip(&mut self.craft_grid) {
                    *saved = active.take();
                }
                r.inventory_craft_result = r.craft_result.take();
            }
            (true, false) => {
                self.carried = r.inventory_carried.take();
                r.state_id = std::mem::take(&mut r.inventory_state_id);
                self.craft_grid.fill(None);
                for (saved, active) in r.inventory_craft_grid.iter_mut().zip(&mut self.craft_grid) {
                    *active = saved.take();
                }
                r.craft_result = r.inventory_craft_result.take();
            }
            (true, true) => {
                self.carried = None;
                r.state_id = 0;
                self.craft_grid.fill(None);
                r.craft_result = None;
            }
            (false, false) => {}
        }
        r.menu.set(menu);
        r.set_menu_title(title);
    }
    /// The server's open menu: its window id and menu type (0, "inventory"
    /// when none is open).
    pub(in crate::game) fn remote_menu(&self) -> (i32, String) {
        match self.remote.as_ref().and_then(|r| r.menu.as_ref().cloned()) {
            Some((id, name)) => (id, name),
            None => (0, "inventory".to_owned()),
        }
    }
    /// `MultiPlayerGameMode.handleContainerInput` for each click of `ops` on
    /// the server's open menu (a quick craft drag is several): the verified
    /// menu engine predicts them on this client's copy, then each goes out as
    /// a `ContainerClick` with the slots it changed and the carried stack
    /// after it; the server corrects anything it sees differently. A menu the
    /// engine doesn't model sends the clicks unpredicted. False when the batch
    /// cannot be submitted.
    pub(in crate::game) fn remote_menu_ops(&mut self, ops: &[(Option<usize>, i32, Input)]) -> bool {
        // CrafterScreen.slotClicked: a slot toggle goes out before the click.
        if let [(Some(slot), button, input)] = ops {
            self.crafter_slot_clicked(*slot, *button, *input);
        }
        let (container, menu) = self.remote_menu();
        let mut clicks = Vec::with_capacity(ops.len());
        for &(slot, button, input) in ops {
            let slot = match slot {
                None => -999,
                Some(internal) => match Self::menu_slot_index(&menu, internal) {
                    Some(s) => s as i16,
                    None => return false,
                },
            };
            clicks.push((slot, button, input));
        }
        // Unpredicted: nothing changed, the carried stack as it is.
        let mut prepared = None;
        let mut threw = false;
        self.menu_session_with_game(|game, m, env, _| {
            let Some((predicted, dropped)) = predict_clicks(m, env, &clicks) else {
                // The server throws on it too: nothing is predicted.
                return Some(false);
            };
            let packets = game.prepare_clicks(container, &predicted);
            let commit = packets.is_ok();
            prepared = Some(packets.map(|packets| (predicted, packets)));
            threw = dropped;
            Some(commit)
        });
        let result = prepared.unwrap_or_else(|| {
            let unpredicted: Vec<Predicted> = clicks
                .iter()
                .map(|&(slot, button, input)| {
                    (slot, button, input, Vec::new(), self.carried.clone())
                })
                .collect();
            self.prepare_clicks(container, &unpredicted)
                .map(|packets| (unpredicted, packets))
        });
        let (predicted, packets) = match result {
            Ok(result) => result,
            Err(error) => {
                log::warn!("Refused menu click: {error}");
                return false;
            }
        };
        self.send_clicks(&predicted, packets);
        // LivingEntity.drop(.., Prediction.PREDICTED): the client swings its own
        // main hand; the server shows the swing only to the other players.
        if threw {
            self.swing_hand(Hand::Main, SwingAnimation::DEFAULT);
        }
        true
    }
    /// A QUICK_MOVE on the server's open menu with nothing predicted: the
    /// client's slots stay as they are and every slot it moved comes back from
    /// the server (`broadcastChanges`), the answer the next step waits for.
    /// A prediction the server agrees with gets no answer at all.
    pub(in crate::game) fn remote_quick_move_unpredicted(&mut self, internal: usize) -> bool {
        let (container, menu) = self.remote_menu();
        let Some(slot) = Self::menu_slot_index(&menu, internal) else {
            return false;
        };
        let carried = self.carried.clone();
        let clicks = [(slot as i16, 0, Input::QuickMove, Vec::new(), carried)];
        match self.prepare_clicks(container, &clicks) {
            Ok(packets) => self.send_clicks(&clicks, packets),
            Err(error) => log::warn!("Refused quick move: {error}"),
        }
        true
    }
    /// A Shift double click on a server (`AbstractContainerScreen.mouseReleased`):
    /// a predicted QUICK_MOVE for every slot the engine moves, each sent.
    pub(in crate::game) fn remote_quick_move_all(&mut self, slot: usize, moved: &ItemStack) {
        let (container, _) = self.remote_menu();
        let mut sent: Vec<Predicted> = Vec::new();
        let mut prepared = None;
        self.menu_session_with_game(|game, m, env, to_menu| {
            let clicked = to_menu(slot)?;
            let mut before = menu_items(m);
            let mut each = |m: &sunrise_client::menu::Menu, t: usize| {
                let changed = changed_slots(&before, m);
                sent.push((t as i16, 0, Input::QuickMove, changed, m.carried.clone()));
                before = menu_items(m);
            };
            let modeled = m
                .quick_move_all_each(env, clicked, moved, &mut each)
                .is_ok();
            let packets = game.prepare_clicks(container, &sent);
            let commit = modeled && packets.is_ok();
            prepared = Some(packets);
            Some(commit)
        });
        match prepared {
            Some(Ok(packets)) => self.send_clicks(&sent, packets),
            Some(Err(error)) => log::warn!("Refused quick move: {error}"),
            None => (),
        }
    }
    fn prepare_clicks(
        &self,
        container: i32,
        clicks: &[Predicted],
    ) -> sunrise_protocol::Result<Vec<play::ContainerClick>> {
        let state = self.remote.as_ref().map_or(0, |r| r.state_id);
        clicks
            .iter()
            .map(|(slot, button, input, changed, carried)| {
                Ok(play::ContainerClick {
                    container,
                    state,
                    slot: *slot,
                    button: *button as i8,
                    input: input_ordinal(*input),
                    changed: changed
                        .iter()
                        .map(|(i, stack)| Ok((*i, self.hashed_stack(stack)?)))
                        .collect::<sunrise_protocol::Result<_>>()?,
                    carried: self.hashed_stack(carried)?,
                })
            })
            .collect()
    }

    fn send_clicks(&mut self, clicks: &[Predicted], packets: Vec<play::ContainerClick>) {
        // A result the engine claims to have changed is the one the server
        // agrees to (`click_agreed`), so it sends none: keep it. A result the
        // engine doesn't claim (a recipe it doesn't know) the server sends.
        if matches!(
            self.remote_menu().1.as_str(),
            "inventory" | "minecraft:crafting"
        ) && let Some(r) = self.remote.as_mut()
        {
            for (.., changed, _) in clicks {
                if let Some((_, result)) = changed.iter().find(|(slot, _)| *slot == 0) {
                    r.craft_result = result.clone();
                }
            }
        }
        for packet in packets {
            self.send(S::ContainerClick(packet));
        }
    }
    /// The server's open menu is the inventory or a chest-like container
    /// (chests, barrels, ender chests: `generic_9xN`; shulker boxes).
    pub(in crate::game) fn remote_storage_open(&self) -> bool {
        match self.remote.as_ref().and_then(|r| r.menu.as_ref()) {
            None => true,
            Some((_, name)) => {
                name == "inventory"
                    || name.starts_with("minecraft:generic_9x")
                    || name == "minecraft:shulker_box"
            }
        }
    }
    /// A click outside a server's window (slot -999): predicted like
    /// `MultiPlayerGameMode.handleContainerInput` (the carried stack after
    /// `doClick`; the server drops the stack and swings the hand).
    pub(in crate::game) fn remote_throw(
        &mut self,
        right: bool,
        input: sunrise_client::menu::Input,
    ) {
        use sunrise_client::menu::Input;
        let container = self
            .remote
            .as_ref()
            .and_then(|r| r.menu.as_ref())
            .map_or(0, |m| m.0);
        let mut predicted = self.carried.clone();
        let threw = input == Input::Pickup && predicted.is_some();
        if input == Input::Pickup
            && let Some(c) = predicted.as_mut()
        {
            if right && c.count > 1 {
                c.count -= 1;
            } else {
                predicted = None;
            }
        }
        let carried = match self.hashed_stack(&predicted) {
            Ok(carried) => carried,
            Err(error) => {
                log::warn!("Refused drop click: {error}");
                return;
            }
        };
        if threw {
            self.carried = predicted;
            self.swing_arm();
        }
        let state = self.remote.as_ref().map_or(0, |r| r.state_id);
        self.send(S::ContainerClick(play::ContainerClick {
            container,
            state,
            slot: -999,
            button: i8::from(right),
            input: if input == Input::Throw { 4 } else { 0 },
            changed: Vec::new(),
            carried,
        }));
    }
    /// `CreativeModeInventoryScreen.slotClicked` outside the window: the server knows nothing of
    /// a stack picked from the creative screen, so the carried stack (an item tab's right click:
    /// one of it) is dropped through `handleCreativeModeItemDrop`, a SetCreativeModeSlot to slot
    /// -1, and the hand swings as `Player.drop` predicts.
    pub(in crate::game) fn remote_creative_drop(&mut self, right: bool) {
        let one = right && !self.creative_inventory_open();
        let Some(carried) = self.carried.as_mut() else {
            return;
        };
        let thrown = if one && carried.count > 1 {
            carried.count -= 1;
            ItemStack {
                count: 1,
                ..carried.clone()
            }
        } else {
            self.carried.take().expect("carried")
        };
        self.remote_creative_throw(thrown);
    }
    /// `MultiPlayerGameMode.handleCreativeModeItemDrop`: a stack thrown out of the creative
    /// screen goes to the server as a SetCreativeModeSlot to slot -1, which drops it from the
    /// player's hand; the client only swings (`Player.drop` on a client).
    pub(in crate::game) fn remote_creative_throw(&mut self, thrown: ItemStack) {
        if !self.creative {
            return;
        }
        self.swing_arm();
        let item = self.to_network(&Some(thrown));
        self.send(S::SetCreativeModeSlot(play::SetCreativeModeSlot {
            slot: -1,
            item,
        }));
    }
    pub(in crate::game) fn remote_close_container(&mut self) {
        let id = self
            .remote
            .as_ref()
            .and_then(|r| r.menu.as_ref().map(|m| m.0))
            .unwrap_or(0);
        self.send(S::ContainerClose(play::ContainerClose { container: id }));
        self.remote_close_local();
    }
    /// `LocalPlayer.closeContainer` as the creative inventory closes: its
    /// `ItemPickerMenu` is window 0, and a server hears the close as it does
    /// any container screen's (`AbstractContainerScreen.onClose`).
    pub(in crate::game) fn close_creative_menu(&self) {
        self.send(S::ContainerClose(play::ContainerClose { container: 0 }));
    }
    /// `MultiPlayerGameMode.handleInventoryButtonClick`: button `button` of
    /// the menu the server opened goes to it (`ContainerButtonClick`), whose
    /// menu decides what it does (`clickMenuButton`). No sound: that is the
    /// screen's. False while no such menu is open.
    pub(in crate::game) fn remote_button_click(&self, button: i32) -> bool {
        let Some(container) = self
            .remote
            .as_ref()
            .and_then(|r| r.menu.as_ref())
            .map(|(w, _)| *w)
        else {
            return false;
        };
        self.send(S::ContainerButtonClick(play::ContainerButtonClick {
            container,
            button,
        }));
        true
    }
    pub(in crate::game) fn remote_reject_container(&mut self, container: i32) {
        self.remote_close_local();
        if self.screen == Screen::Inventory {
            self.set_screen(Screen::Playing);
            self.capture(true);
        }
        self.send(S::ContainerClose(play::ContainerClose { container }));
    }
    pub(in crate::game) fn remote_close_local(&mut self) {
        // `Gui.setScreen` clears the focus of the screen it takes down: the
        // container's keyboard focus must not outlive it (a Tab bound to the
        // inventory moves the focus and closes the container in one press).
        self.pad_clear_focus();
        self.recipe_ui.reset_screen();
        if self.mod_screen.take().is_some() {
            self.clear_mod_book_placement_screen();
            self.screen_state.build.reopened();
        }
        self.remote_set_menu(None, None);
        self.mount_menu_reset();
        // The client's menu has no level access: what it holds is the server's.
        self.station = None;
        self.craft_size = 2;
        if let Some(key) = self.container.take() {
            // Server containers stay renderable but lose their copied slots.
            if let Some(entity) = self.world.block_entities.get_mut(&key) {
                for s in entity.slots_mut() {
                    *s = None;
                }
            }
        }
    }
    /// A server's trading window (MerchantScreen over a ClientSideMerchant).
    pub(in crate::game) fn remote_open_merchant(
        &mut self,
        container: i32,
        title: String,
        text: sunrise_client::text::Text,
    ) {
        self.remote_set_menu(
            Some((container, "minecraft:merchant".to_owned())),
            Some(text.clone()),
        );
        let mut session = sunrise_client::merchant::Session::new(Vec::new(), true);
        session.client_side = true;
        self.open_remote_merchant(session, (!title.is_empty()).then_some(text));
    }
    /// ClientboundMerchantOffersPacket.
    pub(in crate::game) fn remote_merchant_offers(&mut self, p: play::MerchantOffers) {
        if self.open_menu_name(p.container) != "minecraft:merchant" {
            return;
        }
        let cost = |g: &Self, c: &play::NetCost| {
            g.to_stack(&Some(c.clone().into()))
                .map(|stack| sunrise_client::merchant::ItemCost { stack })
        };
        let offers: Vec<_> = p
            .offers
            .iter()
            .filter_map(|o| {
                let mut offer = sunrise_client::merchant::MerchantOffer {
                    base_cost_a: cost(self, &o.cost_a)?,
                    cost_b: match &o.cost_b {
                        Some(b) => Some(cost(self, b)?),
                        None => None,
                    },
                    result: self.to_stack(&Some(o.result.clone()))?,
                    uses: o.uses,
                    max_uses: o.max_uses,
                    reward_exp: true,
                    special_price_diff: o.special_price,
                    demand: o.demand,
                    price_multiplier: o.price_multiplier,
                    xp: o.xp,
                };
                // setToOutOfStock.
                if o.out_of_stock {
                    offer.uses = offer.max_uses;
                }
                Some(offer)
            })
            .collect();
        if let Some(m) = self.station.as_mut().and_then(|s| s.merchant.as_mut()) {
            m.session.offers = offers;
            m.session.level = p.level;
            m.session.xp = p.xp;
            m.session.villager = p.show_progress;
        }
    }
    /// ClientboundOpenScreenPacket for the menus Sunrise draws.
    pub(in crate::game) fn remote_open_screen(&mut self, p: play::OpenScreen) {
        let menu = self
            .remote
            .as_ref()
            .and_then(|r| {
                r.builtin
                    .get("minecraft:menu")?
                    .name(p.menu)
                    .map(str::to_owned)
            })
            .unwrap_or_default();
        let target = self
            .remote
            .as_mut()
            .and_then(|r| r.use_target.take())
            .map(|t| (t[0], t[1], t[2]));
        // The server names a mod menu's block itself: it, not the last
        // clicked block, is the one the menu is of. Only for this window:
        // any other's is stale.
        let extra = self
            .remote
            .as_mut()
            .and_then(|r| r.menu_extra.take())
            .filter(|e| e.window == p.container);
        let target = extra
            .as_ref()
            .map_or(target, |e| e.pos.map(|p| (p[0], p[1], p[2])));
        let mod_type = sunrise_client::containers::mod_menu_type(&menu);
        let block = target.and_then(|pos| self.menu_opener_block(pos, &menu));
        let owner = mod_type.map(|kind| kind.owner);
        // MenuScreens.create looks up the constructor before touching the active window.
        // A stock menu only needs a native screen if its opener masks backing slots.
        if (owner.is_some() || !mod_api::__private::menu_slot_exclusions(&menu, block).is_empty())
            && !mod_api::__private::has_menu_screen_factory(&menu, owner, block)
        {
            log::warn!("no screen factory for menu {menu}, owner {owner:?}, opener {block:?}");
            return;
        }
        if self.remote.as_ref().and_then(|r| r.menu.as_ref()).is_some() || self.mod_screen.is_some()
        {
            // An in-place open has no intervening ContainerClose packet. Drop
            // the replaced menu's local state, not the screen or pointer grab.
            self.remote_close_local();
        } else {
            self.recipe_ui.reset_screen();
        }
        if menu == "minecraft:lectern" {
            // LecternScreen: the book and page come in the menu's content and data.
            self.remote_set_menu(Some((p.container, menu)), None);
            self.remote_open_lectern(target.unwrap_or_default());
            return;
        }
        if let Some(kind) = Self::remote_station_kind(&menu) {
            let title = menu_title(&p.title);
            let pos = target.map_or([0, 0, 0], |t| [t.0, t.1, t.2]);
            self.remote_open_station(p.container, menu, kind, title, pos);
            return;
        }
        if menu == "minecraft:merchant" {
            // A trading window: its offers follow in a MerchantOffers packet.
            let title = plain(
                &p.title,
                &self
                    .remote
                    .as_ref()
                    .map(|r| r.lang.clone())
                    .unwrap_or_default(),
            );
            self.remote_open_merchant(p.container, title, menu_title(&p.title));
            return;
        }
        let entity = match menu.as_str() {
            "minecraft:crafting" => None,
            "minecraft:furnace" | "minecraft:blast_furnace" | "minecraft:smoker" => {
                let kind = match menu.as_str() {
                    "minecraft:blast_furnace" => FurnaceKind::BlastFurnace,
                    "minecraft:smoker" => FurnaceKind::Smoker,
                    _ => FurnaceKind::Furnace,
                };
                Some(BlockEntity::Furnace(Furnace {
                    kind,
                    items: [None, None, None],
                    lit_remaining: 0,
                    lit_total: 0,
                    cooking_timer: 0,
                    cooking_total: 0,
                    speed: 1.0,
                    recipes_used: std::collections::HashMap::new(),
                }))
            }
            // HopperMenu, and DispenserMenu (MenuType.GENERIC_3x3: dispensers
            // and droppers).
            "minecraft:hopper" => Some(BlockEntity::Chest(vec![None; 5])),
            // BrewingStandMenu: its times come in ContainerSetData.
            "minecraft:brewing_stand" => Some(BlockEntity::Brewing(Box::new(
                sunrise_client::brewing::BrewingStand::new(),
            ))),
            "minecraft:generic_3x3" => Some(BlockEntity::Chest(vec![None; 9])),
            // CrafterMenu: the disabled slots and triggered come in ContainerSetData.
            "minecraft:crafter_3x3" => Some(BlockEntity::Crafter(Default::default())),
            m if m.starts_with("minecraft:generic_9x") || m == "minecraft:shulker_box" => {
                let rows = m
                    .strip_prefix("minecraft:generic_9x")
                    .and_then(|r| r.parse::<usize>().ok())
                    .unwrap_or(3);
                Some(BlockEntity::Chest(vec![None; rows * 9]))
            }
            // A content mod's menu type: its container's slots, which its
            // mod's screen places.
            _ if mod_type.is_some() => Some(BlockEntity::Mod(Box::new(
                sunrise_client::containers::ModBlockEntity::new(
                    String::new(),
                    sunrise_client::containers::ModBlockEntityFields {
                        items: vec![None; mod_type.map_or(0, |t| t.slots)],
                        ..Default::default()
                    },
                ),
            ))),
            _ => {
                // Unsupported menu: tell the server we closed it.
                self.remote_reject_container(p.container);
                self.add_chat_line(
                    sunrise_client::text::Text::literal(format!(
                        "(Sunrise cannot show {menu} screens yet)"
                    )),
                    sunrise_client::chat::ChatSource::Client,
                );
                return;
            }
        };
        let title = menu_title(&p.title);
        self.remote_set_menu(Some((p.container, menu.clone())), Some(title));
        self.craft_size = if menu == "minecraft:crafting" { 3 } else { 2 };
        self.container = None;
        if let Some(entity) = entity {
            // The clicked block shows the menu when it holds that container;
            // an ender chest's or a double chest's slots are no one block's,
            // so they go under a key no block has.
            let holds = |k: &(i32, i32, i32)| {
                let name = &self.res.facts().get(self.world.block(k.0, k.1, k.2)).name;
                BlockEntity::for_block(name).is_some_and(|e| {
                    e.id() == entity.id() && e.slots().len() == entity.slots().len()
                })
            };
            let key = target.filter(holds).unwrap_or((i32::MIN, 0, 0));
            let backing_slots = entity.slots().len();
            self.world.block_entities.insert(key, entity);
            self.container = Some(key);
            if !self.open_native_screen(key, target, backing_slots, extra) {
                if mod_type.is_some() || key.0 == i32::MIN {
                    self.world.block_entities.remove(&key);
                }
                self.remote_reject_container(p.container);
                return;
            }
        }
        let native = self.mod_screen.is_some();
        self.set_screen(Screen::Inventory);
        if !native || self.native_screen_shown() {
            self.capture(false);
        }
    }
    /// `ClientPacketListener.handleMountScreenOpen`: a horse's or llama's
    /// inventory; for another entity the window is closed again.
    pub(in crate::game) fn remote_open_mount(&mut self, p: play::MountScreenOpen) {
        if self.mod_screen.is_some() {
            self.remote_close_local();
        }
        let columns = usize::try_from(p.columns).unwrap_or(0).min(5);
        let record = |game: &mut Self| {
            game.remote_set_menu(Some((p.container, format!("{MOUNT_MENU}{columns}"))), None);
        };
        // With a screen hook listening, the window is recorded before the
        // screen opens, so a mod's answer that replaces it closes this window
        // (a rejection clears it again); otherwise after, as vanilla's order.
        let staged = self.screen_hooks_heard();
        if staged {
            record(self);
        }
        if !self.open_remote_mount_menu(p.entity as u32, columns) {
            self.remote_reject_container(p.container);
            return;
        }
        if !staged {
            record(self);
        }
    }
    /// The type of the menu a server opened (`MenuType`'s id).
    pub(in crate::game) fn remote_menu_type(&self) -> Option<&str> {
        Some(self.remote.as_ref()?.menu.as_ref()?.1.as_str())
    }
    /// The title the server gave the menu it opened.
    pub(in crate::game) fn remote_menu_title(&self) -> Option<sunrise_client::text::Text> {
        self.remote.as_ref()?.menu_title.clone()
    }
    /// A language change reaches the server-sent components too: their titles
    /// are resolved when the packet arrives, with the session's language.
    pub(in crate::game) fn refresh_remote_language(&mut self) {
        if let Some(lang) = sunrise_client::lang::current()
            && let Some(r) = self.remote.as_mut()
        {
            r.lang = Arc::new(lang.map().clone());
        }
    }
    /// A server's chest-like menu (`MenuType.GENERIC_9xN`): its rows and title.
    pub(in crate::game) fn remote_chest_menu(&self) -> Option<(usize, sunrise_client::text::Text)> {
        let r = self.remote.as_ref()?;
        let rows = r
            .menu
            .as_ref()?
            .1
            .strip_prefix("minecraft:generic_9x")?
            .parse()
            .ok()?;
        Some((
            rows,
            r.menu_title
                .clone()
                .unwrap_or_else(sunrise_client::text::Text::empty),
        ))
    }
}
