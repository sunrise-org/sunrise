//! Host method declarations; access defaults and game forwarders are generated.
#![allow(unused_variables)]
use crate::*;
/// The client host's methods of the `screens` area, implemented by the game and reached through [`crate::ClientHost`].
#[sunrise_mod_api_macros::host_area]
pub trait ScreensClientHost {
    /// The world-preset editor currently open over Create World, or none.
    #[access(ClientSetup)]
    fn preset_editor(&self) -> Option<PresetEditor>;
    /// A copy of the open preset editor's working dimension stems.
    #[access(ClientSetup)]
    fn preset_editor_dimensions(&self) -> Option<Compound>;
    /// Replace the open editor's working dimension stems. Requires an overworld,
    /// valid dimension ids and type/generator declarations. Returns false outside
    /// the editor or for malformed data; Done commits, Cancel discards the copy.
    /// Interior screen state permits use from the read-only screen input hooks.
    #[access(ClientSetup)]
    fn set_preset_editor_dimensions(&self, dimensions: Compound) -> bool;
    /// Side: client.
    /// The block of the mod screen that is open (`screen`), as the
    /// server named it when the block opened its menu type; None when no mod
    /// screen is open. 401.
    #[access(ClientSetup)]
    fn screen_pos(&self) -> Option<[i32; 3]>;
    /// Side: client.
    /// What the block's mod wrote for the mod screen that is open
    /// (`menu_extra_data` on the server), empty when it wrote nothing;
    /// None when no mod screen is open. The same for a menu `PlayerRef::open_mod_menu`
    /// opened with its `extra`. Returns a copy. @API:menu_extra_data@.
    #[access(ClientSetup)]
    fn screen_extra_data(&self) -> Option<Vec<u8>>;
    /// Side: client.
    /// Where the open screen draws outside its `rect`, and where mods draw
    /// over it: the recipe book's panel, the creative tab rows, the active
    /// effects beside the inventory, then whatever `screen_exclusion_areas`
    /// adds. Each is `[x, y, width, height]` in GUI pixels; an empty or
    /// non-finite one is dropped. A mod that lays out its own panels
    /// (a recipe viewer) keeps them clear of these. Client-side; empty with
    /// no screen up.
    #[access(ClientSetup)]
    fn screen_exclusion(&self) -> Vec<[f32; 4]>;
    /// Side: client.
    /// The id of the item in the slot under the pointer on the open container screen
    /// (`AbstractContainerScreen.getHoveredSlot().getItem()`), None over no slot or an empty one.
    #[access(ClientSetup)]
    fn screen_hovered_item(&self) -> Option<ItemKey>;
    /// Side: client.
    /// Where the open screen of mod menu type `menu` puts container slot `slot` from the next
    /// frame (`Slot.x`, `Slot.y` in GUI pixels from the screen's top left, the item's top left
    /// corner): where `set_screen_slot` last put it, else where the screen declared it. None
    /// for a hidden slot, another menu type's or another mod's screen, or none open.
    #[access(ClientSetup)]
    fn screen_slot(&self, menu: &str, slot: usize) -> Option<[f32; 2]>;
    /// Side: client.
    /// Moves container slot `slot` of the open screen of mod menu type `menu` to `pos`, or hides
    /// it with None, as a Java screen that lays its slots out again sets `Slot.x`/`Slot.y` or
    /// deactivates a slot (Tinkers' station layouts). `slot` indexes `Screen::slots`. From the
    /// next frame the slot is a vanilla slot at that position: the hovered slot (the 18 by 18
    /// box around it), its highlight, its contents, its empty-slot icon and the recipe book's
    /// ghost items; a hidden one is none of these. A position outside the image is allowed.
    /// The change lasts until the screen closes, and the next open starts from the declared
    /// layout. Only the mod that owns the open screen moves its slots: the mod that registered
    /// its recipe-book layout (`MenusRegistrar`), with or without a screen part, else the mod
    /// whose screen part lays it out. Nothing is sent:
    /// the server's menu keeps every slot, so a hidden slot keeps its
    /// item and a shift-click may still fill it, as a deactivated Java slot does, and a slot
    /// the screen hid when it opened stays hidden. It takes `&self`, so a screen hook may call
    /// it every frame; repeating a slot's current place changes nothing and costs no layout
    /// work. Each refusal changes nothing and is named by `ScreenSlotRefusal`: `NoScreen`,
    /// `NoSuchSlot`, `HiddenAtOpen`, `BadPosition` and `Unavailable`.
    #[access(ClientSetup)]
    fn set_screen_slot(
        &self,
        menu: &str,
        slot: usize,
        pos: Option<[f32; 2]>,
    ) -> Result<(), ScreenSlotRefusal>;
    /// Side: client.
    /// Requests vanilla's menu background blur for this frame, with a per-frame guard
    /// so blur is applied at most once per frame. Returns true if blur was newly
    /// requested this frame, or false if it was already applied/requested this frame.
    /// API @API:screen_transparent_background@.
    #[access(ClientSetup)]
    fn apply_screen_blur(&self) -> bool;
    /// Side: client.
    /// The user's configured menu background blur radius (`menuBackgroundBlurriness` in options.txt,
    /// usually 0.0 to 10.0; default 5.0). 0.0 means blur is turned off. API @API:screen_transparent_background@.
    #[access(ClientSetup)]
    fn menu_blur_radius(&self) -> f32;
    /// Side: client.
    /// Opens a screen of mod `mod_id`'s own over the world: the screen hooks
    /// fire over it as over any other, so the mod draws it in
    /// `screen_render_post` and takes its clicks, scroll, keys and
    /// typing there; the mouse is freed, and Escape closes it. Every mod's own
    /// screen has the `ScreenView.id` `mod`, so a mod's screen hooks
    /// check `native_mod_screen()` is its own before drawing. It doesn't pause
    /// a single-player game (as the inventory doesn't). True when it is open:
    /// false while another screen is (or another mod's), over the title
    /// screen, a book, a sign editor, a dialog or a prompt, while the world
    /// loads, and anywhere but the window's own client (the server thread, a
    /// dedicated server). A gamepad can close it (B, Start) but not click on
    /// it: its clicks don't reach the mouse hooks.
    #[access(WorldOnly)]
    fn mod_screen_open(&mut self, mod_id: &str) -> bool;
    /// Side: client.
    /// Closes mod `mod_id`'s own screen, if it is open.
    #[access(WorldOnly)]
    fn mod_screen_close(&mut self, mod_id: &str);
    /// Side: client.
    /// Closes the open screen as its Escape key does: a container screen
    /// closes as the player's container close (`ServerboundContainerClosePacket`
    /// with the menu's id, then the client menu closed), a pause menu closes
    /// to the game, and another screen runs its own close. The sign editor
    /// sends its sign update (`ServerboundSignUpdatePacket`), as Escape does.
    /// A book closes with nothing sent; a lectern's book closes its menu
    /// (`ServerboundContainerClosePacket`). A confirmation, a server's pack
    /// prompt or a dialog open over another screen is the one that closes.
    /// Declining a required pack disconnects rather than returning to the
    /// screen underneath. The screen hooks fire as for Escape.
    /// The death screen, the code of conduct and a dialog Escape cannot
    /// close do not close this way. False when no screen is open (or,
    /// outside a world, when it cannot act).
    #[access(WorldOnly)]
    fn close_screen(&mut self) -> bool;
    /// Side: client.
    /// Opens the player's inventory as the inventory key does: locally, with
    /// no packet (the player menu is always open on the server). Riding a
    /// vehicle with its own inventory sends the open-inventory player command
    /// (`ServerboundPlayerCommandPacket`), as the key does. False with a
    /// screen already open, for a spectator, in a portal transition, or with
    /// no world.
    #[access(WorldOnly)]
    fn open_inventory(&mut self) -> bool;
    /// Side: client.
    /// The sign the sign editor is on, its block position and side; None when
    /// no sign editor is open or outside a world.
    #[access(ClientSetup)]
    fn sign_editor(&self) -> Option<SignEditView>;
    /// Side: client.
    /// The mod whose own screen is open, if one is (Escape, or another screen
    /// replacing it, closes it).
    #[access(ClientSetup)]
    fn mod_screen(&self) -> Option<&str>;
    /// Side: client.
    /// The open menu's title (the block's name, or the one it was given),
    /// as plain text.
    #[access(ClientSetup)]
    fn screen_title(&self) -> Option<String>;
    /// Side: client.
    /// The last window focus state reported by the window system.
    #[access(ClientSetup)]
    fn window_focused(&self) -> bool;
    /// Side: client.
    /// The same screen identifier provided to screen hooks.
    #[access(ClientSetup)]
    fn observed_screen_id(&self) -> Option<ScreenId>;
    /// Side: client.
    /// Whether the client draws its debug overlay this frame. Always-on debug entries can
    /// make it visible with F3 off. F1 hides the overlay unless a screen is open.
    #[access(ClientSetup)]
    fn debug_overlay_visible(&self) -> bool;
    /// Side: client.
    /// Whether a specific key is currently held down.
    /// `key` is the vanilla physical key or mouse button name (`key.keyboard.left.shift`,
    /// `key.mouse.left`, `key.mouse.4`, for example).
    /// Unknown names are not down. Modifier combinations use `ClientHost::modifiers`.
    #[access(ClientSetup)]
    fn is_key_down(&self, key: &str) -> bool;
}
