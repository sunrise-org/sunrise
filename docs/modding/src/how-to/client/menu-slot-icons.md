# Menu slot icons

In the registered screen factory, declare each visible container slot in `MenuScreenLayout.slots` as `Some(ScreenSlot { position: [x, y], no_item_icon: Some("minecraft:container/slot/shield".into()) })`. Use `no_item_icon: None` for a visible slot with no icon, and a `None` slot entry only for a [registered slot exclusion](../world/machine-menu-data.md#the-menus-screen). Keep each entry at its physical backing index. Geometry and the optional icon belong to the client screen, including a screen for a bound recipe book.

The icon is vanilla's `Slot.getNoItemIcon`: a GUI atlas sprite identifier, drawn at 16 × 16 when the displayed stack is empty. A quick-craft preview that displays an item suppresses it; a single-slot drag hides the entire slot's contents as usual. Filled slots draw their item instead. Custom backgrounds do not suppress the icon. A local slot move moves the icon with the slot; a local hide suppresses it without changing the server's mask.

The client resolves icons against the current GUI atlas when opening the declared screen. An unknown identifier refuses the client opening with a diagnostic before drawing. Icon lookup does not allocate during drawing.
