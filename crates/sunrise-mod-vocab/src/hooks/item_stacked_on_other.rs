//! The `item_stacked_on_other` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// NeoForge's `ItemStackedOnOtherEvent`, before either item override. It runs inside a pickup
    /// click on the authoritative side only (a client predicts vanilla and the server corrects it), with the cursor and both stacks frozen in `event`. Answer in `result`: `canceled`
    /// skips the bundle override, and a cancelled click that is also `handled` is done, with the
    /// stacks `result` sets (a cancelled click not `handled` goes on to the plain pickup; setting
    /// a stack on any other answer refuses the click). A stack set for the clicked slot goes
    /// through its pickup, placement and size rules using the actual replacement stack.
    /// A replacement cannot change a result slot or take a stack from a brewing potion slot:
    /// it would skip the slot's take effects. A write through the `Host` to what the click read
    /// makes it stale: the click is refused and every write is undone. An invalid answer or
    /// a panic also refuses the click without falling back to the plain pickup.
    #[hook(
        side = "both",
        contract = "cancel",
        out = "canceled",
        neoforge = "ItemStackedOnOtherEvent",
        cancellable = true,
        version = 4
    )]
    fn item_stacked_on_other();
}
