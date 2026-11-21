# Reading the level from a block screen

`MenuPart::screen_draw` exposes `event.pos: Option<[i32; 3]>`, alongside the menu type, the block identifier (`event.block`, `None` for a menu opened without a block), synchronized integer data slots and drawing origin. The position comes from the open menu's server-supplied extra data. For servers without that protocol, it is the clicked block position. A menu explicitly opened without a position reports `None`, not a fabricated origin.

Use this position with the existing both-sided level reads through the client host to inspect the client level view, for example the block beneath a machine. This view may lag the server and is not a substitute for authoritative menu data. The draw callback remains read-only and fires beneath items and labels.
