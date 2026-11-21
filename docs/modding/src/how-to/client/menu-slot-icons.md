# Menu slot icons

Declare each visible container slot in `Screen.slots` as `Some(ScreenSlot { position: [x, y], no_item_icon: Some("minecraft:container/slot/shield".into()) })`. Use `None` for a hidden slot or for no icon. Geometry and the optional icon belong to the same slot declaration, including registered recipe-book screens.

The icon is vanilla's `Slot.getNoItemIcon`: a GUI atlas sprite identifier, drawn at 16 × 16 when the displayed stack is empty. A quick-craft preview that displays an item suppresses it; a single-slot drag hides the entire slot's contents as usual. Filled slots draw their item instead. Custom backgrounds do not suppress the icon.

The client resolves icons against the current GUI atlas when opening the declared screen. An unknown identifier refuses the mod screen with a diagnostic before drawing; the menu itself remains open. Icon lookup does not allocate during drawing.
