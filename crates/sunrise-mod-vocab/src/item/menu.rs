//! Moved here from the game so mods can name it.
#[allow(unused_imports)]
use crate::*;
/// One click on the local player's open menu, submitted with `MenuRef::click`.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq, sunrise_mod_api_macros::Wire)]
pub enum MenuClick {
    /// Pick up, place or drop the carried stack; `None` means outside.
    Pickup {
        slot: Option<usize>,
        button: MouseButton,
    },
    /// Shift-click a menu-order slot into the opposite inventory.
    QuickMove { slot: usize },
    /// Exchange a menu-order slot with a hotbar slot or the offhand.
    Swap { slot: usize, with: SwapWith },
    /// Clone a menu-order slot in creative mode.
    Clone { slot: usize },
    /// Drop one item or the whole stack from a menu-order slot.
    Throw { slot: usize, whole_stack: bool },
    /// Distribute the carried stack across menu-order slots as one complete gesture.
    Drag { kind: DragKind, slots: Vec<usize> },
    /// Collect matching stacks into the carried stack.
    PickupAll { slot: usize },
}

/// The mouse button used to pick up or place a stack.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
// exhaustive: the two vanilla pickup buttons
#[derive(sunrise_mod_api_macros::Wire)]
pub enum MouseButton {
    Left,
    Right,
}

/// The inventory location exchanged with a menu slot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
// exhaustive: vanilla swaps target the hotbar or the offhand
#[derive(sunrise_mod_api_macros::Wire)]
pub enum SwapWith {
    /// A hotbar index in 0..=8; other indices refuse the entire batch.
    Hotbar(u8),
    Offhand,
}

/// Whether `open_menu` or `PlayerRef::open_mod_menu` first sends the client a close for the menu
/// it replaces, as a Java `MenuProvider` chooses (`shouldTriggerClientSideContainerClosingOnOpen`).
/// Either way the old menu closes on the server, its cursor stack going back to the player's
/// inventory and `player_container_close` firing once.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
// exhaustive: vanilla's choice is a boolean
#[derive(sunrise_mod_api_macros::Wire)]
pub enum MenuReplace {
    /// `ServerPlayer.closeContainer`, vanilla's default: the client closes the old screen first,
    /// which centres the mouse again.
    CloseFirst,
    /// `ServerPlayer.doCloseContainer`: the new screen takes the old one's place, title and rows
    /// included, and the mouse stays where it is.
    InPlace,
}

/// How a complete drag distributes the carried stack.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
// exhaustive: vanilla has split, one-per-slot and creative-fill drags
#[derive(sunrise_mod_api_macros::Wire)]
pub enum DragKind {
    Split,
    One,
    /// Fill each slot in creative mode only.
    Clone,
}

/// Identity and location of a player pressing a mod container button.
// exhaustive: the game builds it with a literal; mods only read it
#[derive(sunrise_mod_api_macros::Wire)]
pub struct MenuButtonContext {
    pub player: u128,
    pub player_entity_id: i32,
    /// The block backing the menu, or None when the menu was opened without a block.
    pub pos: Option<[i32; 3]>,
    pub id: i32,
}

#[cfg(test)]
mod menu_button_context_tests {
    use super::MenuButtonContext;
    use crate::wire::{Reader, Writer};

    #[test]
    fn menu_button_context_round_trips_with_and_without_a_block_position() {
        for pos in [None, Some([12, -4, 29])] {
            let context = MenuButtonContext {
                player: 0x1234,
                player_entity_id: 57,
                pos,
                id: 8,
            };
            let mut writer = Writer::new();
            writer.put(&context);
            let decoded = Reader::decode::<MenuButtonContext>(writer.bytes()).unwrap();
            assert_eq!(decoded.player, context.player);
            assert_eq!(decoded.player_entity_id, context.player_entity_id);
            assert_eq!(decoded.pos, context.pos);
            assert_eq!(decoded.id, context.id);
        }
    }
}

/// The logical side on which a menu click runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
// exhaustive: a closed set the game matches without a wildcard
#[derive(sunrise_mod_api_macros::Wire)]
pub enum LogicalSide {
    Client,
    Server,
}
/// A pickup click on a menu slot, as `item_stacked_on_other` is asked about it: the stacks are the
/// ones the click found, frozen before any listener ran. A listener answers through
/// [`StackedOnOtherResult`]; it does not write these.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub struct StackedOnOther {
    /// The clicking player's UUID, when a player is clicking.
    pub player: Option<u128>,
    /// The clicking player's entity id, when a player is clicking.
    pub player_entity_id: Option<i32>,
    /// The clicked slot's menu index.
    pub slot: usize,
    /// Which container the slot is in, as the menu numbers them.
    pub container: usize,
    /// The slot's index in that container.
    pub container_slot: usize,
    /// A left-button click (a right-button click is not primary).
    pub primary: bool,
    /// The stack on the cursor, `None` when it is empty.
    pub carried: Option<ItemStack>,
    /// The stack in the clicked slot, `None` when it is empty.
    pub clicked: Option<ItemStack>,
}
impl StackedOnOther {
    /// The click `slot` (menu index) of `container`'s slot `container_slot`, with the stacks it found.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        player: Option<u128>,
        player_entity_id: Option<i32>,
        slot: usize,
        container: usize,
        container_slot: usize,
        primary: bool,
        carried: Option<ItemStack>,
        clicked: Option<ItemStack>,
    ) -> Self {
        Self {
            player,
            player_entity_id,
            slot,
            container,
            container_slot,
            primary,
            carried,
            clicked,
        }
    }
}
/// Whether a listener keeps the original stack or replaces it.
#[derive(Clone, Debug, Default, PartialEq)]
// exhaustive: a stack is left alone or replaced, nothing else
#[derive(sunrise_mod_api_macros::Wire)]
pub enum StackResult {
    /// Leave the original stack unchanged.
    #[default]
    Keep,
    /// Replace the stack: `None` empties it.
    Set(Option<ItemStack>),
}
/// A listener's answer to [`StackedOnOther`], validated once and applied once. Setting a stack is
/// allowed only on a cancelled click that is also `handled` (the listener did the click's work
/// itself); on any other the click is refused and nothing changes. A replaced stack must be a
/// valid stack (a known item, a count of at least one within the item's maximum).
#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub struct StackedOnOtherResult {
    /// `ItemStackedOnOtherEvent.setCancellationResult`: on a cancelled click, whether the click
    /// is done (`true`: the listener's work is all of it) or the plain pickup still runs.
    pub handled: bool,
    /// The cursor after the click.
    pub carried: StackResult,
    /// The clicked slot after the click.
    pub clicked: StackResult,
}
/// The inputs and vanilla result of `AnvilMenu.createResult`, borrowed for
/// the `anvil_update` callback. Edits by earlier listeners do not change these values.
#[derive(Clone, Debug, PartialEq)]
// exhaustive: the game builds it with a literal; mods only read it
// no wire: borrows the converted stacks and rename text for the hook call.
pub struct AnvilUpdateInputs<'a> {
    /// The stack in the left input slot. The hook is not asked without one.
    pub left: &'a ItemStack,
    /// The stack in the right input slot, if present.
    pub right: Option<&'a ItemStack>,
    /// The rename box's text, `None` if the client sent none.
    pub name: Option<&'a str>,
    /// The result calculated by vanilla, if any.
    pub vanilla_output: Option<&'a ItemStack>,
    /// The level cost calculated by vanilla.
    pub vanilla_xp_cost: i32,
    /// The number of material items vanilla would charge from the right slot.
    pub vanilla_material_cost: i32,
}

/// The shared answer to `anvil_update`. It starts at `Keep` and vanilla's costs.
/// The game applies the final edits after dispatch; cancellation or an invalid
/// replacement leaves no output and zero costs.
#[derive(Clone, Debug, PartialEq)]
// exhaustive: the game builds it with a literal
// no wire: staged edits are applied in-process at the end of the hook call.
pub struct AnvilUpdateEdits {
    /// Keep vanilla's output, or set a replacement. `Set(None)` clears it.
    /// A replacement differing from vanilla must be a valid stack.
    pub output: StackResult,
    /// Signed level cost. Zero or a negative value prevents taking the result,
    /// even in creative, and never refunds levels.
    pub xp_cost: i32,
    /// Units consumed from the right slot. Negative values clamp to zero;
    /// zero or a value exceeding the stack consumes all of it on take.
    pub material_cost: i32,
}
/// The inputs and vanilla result of `GrindstoneMenu.createResult`, borrowed for
/// the `grindstone_place` callback. Edits by earlier listeners do not change these values.
#[derive(Clone, Debug, PartialEq)]
// exhaustive: the game builds it with a literal; mods only read it
// no wire: borrows the converted stacks for the hook call.
pub struct GrindstonePlaceInputs<'a> {
    /// The stack in the top input slot, if present.
    pub top: Option<&'a ItemStack>,
    /// The stack in the bottom input slot, if present.
    pub bottom: Option<&'a ItemStack>,
    /// The result calculated by vanilla, if any.
    pub vanilla_output: Option<&'a ItemStack>,
}

/// The shared answer to `grindstone_place`, initially no override and `-1` XP.
/// The game validates the final answer before applying it.
#[derive(Clone, Debug, PartialEq)]
// exhaustive: the game builds it with a literal
// no wire: staged edits are applied in-process at the end of the hook call.
pub struct GrindstonePlaceEdits {
    /// Leave vanilla's result and experience with `None`, or replace the result with a
    /// valid stack. To clear the result, cancel the hook instead.
    pub output: Option<ItemStack>,
    /// Experience to award when the replacement is taken. `-1` uses vanilla's award;
    /// values below `-1` are refused unless cancelled.
    pub xp: i32,
}
/// The input stacks before the grindstone result was taken, borrowed for the
/// `grindstone_take` callback. Edits by earlier listeners do not change this snapshot.
#[derive(Clone, Debug, PartialEq)]
// exhaustive: the game builds it with a literal; mods only read it
// no wire: borrows the converted pre-take stacks for the hook call.
pub struct GrindstoneTakeInputs<'a> {
    /// The stack in the top input slot before the take, if present.
    pub top: Option<&'a ItemStack>,
    /// The stack in the bottom input slot before the take, if present.
    pub bottom: Option<&'a ItemStack>,
}

/// The shared answer to `grindstone_take`. Both remainders start at `None`
/// and the game validates the final answer before applying any of it.
#[derive(Clone, Debug, PartialEq)]
// exhaustive: the game builds it with a literal
// no wire: staged edits are applied in-process at the end of the hook call.
pub struct GrindstoneTakeEdits {
    /// Experience to award, initially what `grindstone_place` set or `-1` for
    /// vanilla's award. Set a nonnegative amount to override it. Values below
    /// `-1` are refused unless the take is cancelled.
    pub xp: i32,
    /// A valid replacement for the top input, or `None` to empty the slot.
    pub new_top: Option<ItemStack>,
    /// A valid replacement for the bottom input, or `None` to empty the slot.
    pub new_bottom: Option<ItemStack>,
}
/// The anvil inputs and result before the take. Both `anvil_craft_pre` and
/// `anvil_craft_post` borrow this same snapshot for their callback.
#[derive(Clone, Debug, PartialEq)]
// exhaustive: the game builds it with a literal; mods only read it
// no wire: borrows the pre-take snapshot for this hook call.
pub struct AnvilCraftInputs<'a> {
    /// The stack in the left slot before the take, if present.
    pub left: Option<&'a ItemStack>,
    /// The stack in the right slot before the take, if present.
    pub right: Option<&'a ItemStack>,
    /// The result stack that was taken.
    pub output: &'a ItemStack,
}

/// One of a menu's own (non-player) containers: the key a `menu_slot_rules` ask names its slot's
/// container by (`event.container`, with `event.slot` indexing inside it) and
/// [`MenuSlotContext::slot`] reads by. A plain menu has `Main` only; a mod menu type with a
/// crafting grid keeps its grid slots in `Grid` and its crafted result in `Result` (slot 0);
/// a slot of the player's inventory is `Player`'s.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub enum MenuContainer {
    /// The menu's own slots (a chest's, a machine's).
    #[default]
    Main,
    /// A crafting grid's slots, row by row.
    Grid,
    /// A crafting grid's result slot.
    Result,
    /// The player's inventory. A rule can be asked about one of its slots, but
    /// [`MenuSlotContext::slot`] does not read them.
    Player,
}
/// The menu's player, its block entity and a read-only view of its staged slots for one
/// `menu_slot_rules` ask, on either the server or the predicting client. Read only the slots
/// needed to answer the rule; the view cannot outlive the ask.
// no wire: this view borrows the click planner's staged slots.
#[non_exhaustive]
pub struct MenuSlotContext<'a> {
    /// The UUID of the player whose menu it is; None where no player is known.
    pub player: Option<u128>,
    /// The block entity the menu belongs to; None when the menu is not a block's
    /// (opened by a mod with no block, a vehicle's, the player's own inventory).
    pub pos: Option<[i32; 3]>,
    read: &'a dyn Fn(MenuContainer, usize) -> Option<ItemStack>,
}
impl<'a> MenuSlotContext<'a> {
    /// Lend a read of the menu's staged slots, with the player and block position of this ask.
    /// The reader returns `None` for empty, absent or unrepresentable slots, including every
    /// player inventory slot.
    pub fn new(
        player: Option<u128>,
        pos: Option<[i32; 3]>,
        read: &'a dyn Fn(MenuContainer, usize) -> Option<ItemStack>,
    ) -> Self {
        Self { player, pos, read }
    }

    /// Read one slot at its backing index, converting just that stack to an API value.
    /// Returns `None` if it is empty, absent, cannot be represented, or belongs to `Player`.
    pub fn slot(&self, container: MenuContainer, index: usize) -> Option<ItemStack> {
        (self.read)(container, index)
    }
}
