//! The `menu_slot_take` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A player took a stack out of slot `slot` of the mod menu type `menu`
    /// open on the mod block entity `entity`, on the server (`Slot.onTake`):
    /// a notice after the item left the slot, which refuses nothing (the
    /// pickup rule, `menu_slot_rules`' `may_pickup`, refuses). `player` is the
    /// player's UUID; `slot` is the menu's index, the block's own slots first
    /// (linked containers and the player's inventory are never heard). `taken` is
    /// a copy of the stack that left the slot, components included, and `left`
    /// what the slot holds afterwards (`None` when it was emptied). Vanilla
    /// hands `onTake` the taken stack on a click but the stack left in the
    /// slot on a quick move, so both are carried: for a quick move `taken` is
    /// the amount moved (the slot's stack before, less `left`).
    ///
    /// Heard once per take, after it: a pickup to an empty cursor (all of the
    /// slot with the left button, half rounded up with the right); a pickup
    /// onto a matching cursor stack from a slot that refuses placement (a
    /// result slot, all of it or nothing); a hotbar or offhand swap that takes
    /// the slot's item (the whole stack, or the slot's former stack when both
    /// held items; a stack that did not fit goes to the inventory or drops
    /// after the notice); a throw (once per repeat of the control variant);
    /// pick-all (once per slot taken from, with the part taken); and each
    /// quick move that moved something. A shift-click on a crafting result
    /// repeats the move once per craft, so it is heard once per craft, each
    /// with that craft's stack. When a part answered the shift-click with
    /// `menu_quick_move`'s `handled`, it is heard after the move that part
    /// applied, if it moved something, with the stack and the count it moved.
    /// Not heard for an exchange of the cursor and slot stacks, an insertion,
    /// a drag spread, a creative clone, dropping the cursor, a take the pickup
    /// rule refused or a quick move that moved nothing; menu buttons, recipe
    /// placement and creative slot sets are not clicks and never reach it.
    ///
    /// The take and the listeners are one action: what a listener writes
    /// through the host stays when it returns, and if a listener fails
    /// (panics or writes an invalid block entity snapshot), all writes and
    /// takes from that click are undone before the menu is resent. `entity` is out of the world during the
    /// call, as in `menu_button`. Not `item_crafted` or `item_smelted`, which
    /// keep their meaning: a result slot that gives experience or unlocks a
    /// recipe does it here through the host. Nothing is asked when no mod
    /// listens.
    #[hook(
        side = "server",
        contract = "listen",
        scope = "block_entity",
        key = "entity",
        neoforge = "none (a Java menu's slot overrides Slot.onTake; NeoForge adds no event)"
    )]
    fn menu_slot_take();
}
