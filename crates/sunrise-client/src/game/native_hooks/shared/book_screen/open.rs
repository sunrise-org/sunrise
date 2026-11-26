use super::*;

impl Game<'_> {
    /// The mod block at `pos` when its container opens menu type `menu`, the
    /// `block` its screen factory is told. Only when this is the menu that
    /// block opens: a position outlives the use that set it, so another menu
    /// opened later (a command's, another mod's) is not the last clicked
    /// machine's. Read the declaration directly, as a client's world has no
    /// server-side mod container table.
    pub(in crate::game) fn menu_opener_block(&self, pos: Pos, menu: &str) -> Option<&str> {
        let block = &self
            .res
            .facts()
            .get(self.world.block(pos.0, pos.1, pos.2))
            .name;
        let container = mod_api::__private::parts_installation::block_container(block)?;
        let (opens, _) = mod_menu(container.menu.as_deref()?)?;
        (opens == menu).then_some(block.as_ref())
    }
    /// Construct a candidate using packet data, without publishing its backing or owner.
    /// `Ok(None)` is an ordinary stock menu with no factory; `Err(())` refuses it.
    pub(in crate::game) fn prepare_native_screen(
        &self,
        (window, name): (i32, &str),
        title: Option<&mod_api::Text>,
        key: Pos,
        target: Option<Pos>,
        backing_slots: usize,
        extra: Option<sunrise_protocol::menu_extra::Message>,
    ) -> Result<Option<OpenNativeScreen>, ()> {
        if self.is_logical_server() || self.lan_acting.is_some() {
            return Err(());
        }
        let mod_type = sunrise_client::containers::mod_menu_type(name);
        let block = target.and_then(|pos| self.menu_opener_block(pos, name));
        let binding = mod_api::RecipeBookLayout::registered(name);
        let of = block.unwrap_or(name);
        // A matching payload supplies the position, including an explicit absence.
        // Otherwise only a verified clicked opener supplies one, never the slot storage key.
        let pos = extra.as_ref().map_or_else(
            || {
                target
                    .filter(|_| block.is_some())
                    .map(|(x, y, z)| [x, y, z])
            },
            |extra| extra.pos,
        );
        let open = mod_api::MenuScreenOpen {
            window,
            menu: name,
            block,
            pos,
            title,
            backing_slots,
            player_slots: mod_type
                .map_or(36, sunrise_client::containers::ModMenuType::player_slots),
            data_slots: mod_type.map_or(0, |kind| kind.data_slots),
            extra: extra.as_ref().map_or(&[], |extra| extra.data.as_slice()),
        };
        let created = mod_api::__private::create_menu_screen(self, open);
        // Opening bytes are construction-only; the result owns its presentation.
        drop(extra);
        let Some((owner, created)) = created else {
            return Ok(None);
        };
        let mut instance = match created {
            Ok(Some(instance)) => instance,
            Ok(None) => {
                log::warn!("mod {owner} declined to create a screen for {of}");
                return Err(());
            }
            Err(reason) => {
                log::error!("mod {owner} screen for {of}: {reason}");
                return Err(());
            }
        };
        let screen = instance.layout_mut();
        if let Some((book, _, _)) = &binding
            && screen.recipe_book.as_ref() != Some(book)
        {
            log::error!("screen for {of}: expected the menu's bound recipe book {book:?}");
            return Err(());
        }
        if ![screen.width, screen.height]
            .into_iter()
            .all(|size| size.is_finite() && size > 0.0)
            || ![screen.inventory_x, screen.inventory_y]
                .into_iter()
                .chain(screen.title_label.into_iter().flatten())
                .chain(screen.inventory_label.into_iter().flatten())
                .chain(
                    screen
                        .recipe_book_toggle
                        .into_iter()
                        .flat_map(|toggle| [toggle.x, toggle.y]),
                )
                .all(f32::is_finite)
        {
            log::error!("screen for {of}: invalid dimensions or coordinates");
            return Err(());
        }
        if screen.slots.len() > backing_slots {
            log::error!("screen for {of}: layout is longer than its {backing_slots} backing slots");
            return Err(());
        }
        // Every slot uses its backing index; `None` (including omitted trailing
        // entries) marks an initially inactive slot (`Slot.isActive()`).
        for (index, slot) in screen.slots.iter().enumerate() {
            if let Some(slot) = slot
                && !slot.position.into_iter().all(f32::is_finite)
            {
                log::error!("screen for {of}: nonfinite slot {index} coordinates");
                return Err(());
            }
        }
        screen.slots.resize_with(backing_slots, || None);
        Ok(Some(OpenNativeScreen {
            menu: OpenNativeMenu {
                key,
                window,
                // Ownership is bound only in install_native_screen.
                generation: 0,
                name: name.to_owned(),
                title: title.cloned(),
                block: block.map(str::to_owned),
                pos,
                mod_type,
                data: Vec::new(),
            },
            slot_moves: Default::default(),
            layout_revision: 0,
            screen: instance,
            shown: false,
            detached: false,
        }))
    }

    pub(in crate::game) fn install_native_screen(
        &mut self,
        mut screen: OpenNativeScreen,
        generation: u64,
    ) {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        screen.layout_revision = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        screen.menu.generation = generation;
        self.mod_screen = Some(screen);
    }

    #[cfg(test)]
    pub(in crate::game) fn open_native_screen(
        &mut self,
        key: Pos,
        target: Option<Pos>,
        backing_slots: usize,
        extra: Option<sunrise_protocol::menu_extra::Message>,
    ) -> bool {
        let Some(remote) = self.remote.as_ref() else {
            return false;
        };
        let Some((window, menu)) = remote.menu.as_ref() else {
            return false;
        };
        let generation = remote.menu.generation;
        match self.prepare_native_screen(
            (*window, menu),
            remote.menu_title_snapshot(),
            key,
            target,
            backing_slots,
            extra,
        ) {
            Ok(Some(screen)) => self.install_native_screen(screen, generation),
            Ok(None) => (),
            Err(()) => return false,
        }
        true
    }
}
