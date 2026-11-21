use super::*;

impl Game<'_> {
    /// The screen of menu type `menu`: its startup-validated registered book
    /// layout, else what the menu type's parts answer `native_screen`. `block`
    /// is the mod block whose menu it is, when the menu is block-backed.
    fn menu_screen(&self, menu: &str, block: Option<&str>) -> Option<mod_api::Screen> {
        let mut screen = mod_api::RecipeBookLayout::registered_screen(menu);
        if screen.is_some() {
            // A code-opened menu may provide its grid through native slot
            // backing instead of a block entity.
            return screen;
        }
        self.native_screen(menu, block, &mut screen);
        screen
    }
    /// The mod screen of a menu of type `menu` opened at `target`, with the
    /// mod block whose menu it is: a mod menu type's screen is its type's
    /// (`menu_screen`) whatever opened it, and a vanilla type gets a mod's
    /// screen only as the menu of the mod block at `target` that opens it.
    fn opened_screen(
        &self,
        menu: &str,
        target: Option<Pos>,
    ) -> Option<(Option<String>, mod_api::Screen)> {
        let block = target.and_then(|t| self.menu_opener_block(t, menu));
        if block.is_none() && sunrise_client::containers::mod_menu_type(menu).is_none() {
            return None;
        }
        let screen = self.menu_screen(menu, block.as_deref())?;
        Some((block, screen))
    }
    /// The container slots (of `size`) that the mod's screen for a menu of
    /// type `menu` opened at `target` does not show: the server's menu does
    /// not have them either, so a shift-click never moves an item where the
    /// player cannot see it.
    pub(in crate::game) fn native_hidden_slots(
        &self,
        menu: &str,
        target: Option<Pos>,
        size: usize,
    ) -> Vec<usize> {
        let Some((_, screen)) = self.opened_screen(menu, target) else {
            return Vec::new();
        };
        (0..size)
            .filter(|&i| screen.slots.get(i).is_none_or(Option::is_none))
            .collect()
    }
    /// The mod block at `pos` when its container opens menu type `menu`, the
    /// `block` its screen hooks are told. Only when this is the menu that
    /// block opens: a position outlives the use that set it, so another menu
    /// opened later (a command's, another mod's) is not the last clicked
    /// machine's. Asked of the block's container, as a client's world has no
    /// mod containers.
    fn menu_opener_block(&self, pos: Pos, menu: &str) -> Option<String> {
        let block = &self
            .res
            .facts()
            .get(self.world.block(pos.0, pos.1, pos.2))
            .name;
        if block.starts_with("minecraft:") {
            return None;
        }
        let mut container = None;
        self.native_block_entity_container(block, &mut container);
        let opens = container.and_then(|c| mod_menu(c.menu.as_deref()?).map(|(m, _)| m));
        (opens == Some(menu)).then(|| block.to_string())
    }
    /// A menu opened on this client with its slots under `key`, at `target`:
    /// its screen is `opened_screen`'s. `extra` is what the server sent for
    /// this window first.
    pub(in crate::game) fn open_native_screen(
        &mut self,
        key: Pos,
        target: Option<Pos>,
        window: i32,
        menu: &str,
        extra: Option<sunrise_protocol::menu_extra::Message>,
    ) {
        self.clear_mod_book_placement_screen();
        self.mod_screen = None;
        if let Some((block, mut screen)) = self.opened_screen(menu, target) {
            let mod_type = sunrise_client::containers::mod_menu_type(menu);
            if let Some(kind) = mod_type {
                screen.slots.truncate(kind.slots);
            }
            // Check sprite identifiers once at the client declaration boundary.
            for slot in screen.slots.iter_mut().flatten() {
                if let Some(icon) = &slot.no_item_icon
                    && self.gui.atlas.gui_rect(icon).is_none()
                {
                    let of = block.as_deref().unwrap_or(menu);
                    log::error!("screen for {of}: unknown empty-slot GUI sprite {icon:?}");
                    return;
                }
            }
            self.mod_screen = Some(OpenNativeScreen {
                slot_moves: Default::default(),
                layout_revision: {
                    static NEXT: std::sync::atomic::AtomicU64 =
                        std::sync::atomic::AtomicU64::new(1);
                    NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
                },
                key,
                window,
                menu: menu.to_owned(),
                block,
                screen,
                mod_type,
                data: Vec::new(),
                extra,
            });
            self.begin_mod_book_placement();
            self.ensure_mod_book_catalog();
        }
    }
}
