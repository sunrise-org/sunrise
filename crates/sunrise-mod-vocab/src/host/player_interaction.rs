//! The `player_interaction` area: what a player does to blocks and entities: range, permission, breaking and placing. Host method declarations; access defaults and game forwarders are generated.
#![allow(unused_variables)]
use crate::*;
/// The handles' reads of the `player_interaction` area, implemented by the game and reached through [`Backend`].
// hidden: a handle's backend, reached through `Backend`; mods call the handle, never this trait
#[doc(hidden)]
#[sunrise_mod_api_macros::host_area]
pub trait PlayerInteractionBackend {
    /// Side: server.
    /// `Player.getAttackStrengthScale(partial)`: the charge the player's next
    /// `Player.attack` reads, from 0 (just swung) to 1 (full), where `partial`
    /// (vanilla reads 0.5) is how far between ticks the read is. It is
    /// `(attackStrengthTicker + partial) / getCurrentItemAttackStrengthDelay()`,
    /// the delay from the attack speed attribute and the main-hand item.
    /// Inside `attack_entity` it reads the charge that blow uses: the hook
    /// runs before `Player.attack` resets the ticker. None for a stale or unavailable player.
    /// Non-finite partial ticks return `None`; finite partials are not restricted.
    fn player_attack_strength_scale(&self, player: PlayerRef, partial: f32) -> Option<f32>;
    /// Side: server.
    /// `Player.mayUseItemAt(pos, face, stack)`: whether the player may use `stack` at `pos`, the
    /// block it would be placed in when clicked on its `face`. A player who may build (survival,
    /// creative) always may; an adventure or spectator player only with a stack whose
    /// `can_place_on` component matches the clicked block, the one `pos` lies against. The game's
    /// own use runs the same check (`ItemStack.useOn`, buckets). None for a stale or unavailable
    /// player, and for one in another level than the one in place.
    fn player_may_use_item_at(
        &self,
        player: PlayerRef,
        pos: [i32; 3],
        face: BlockFace,
        stack: &ItemStack,
    ) -> Option<bool>;
    /// Side: server.
    /// `ServerPlayer.canHarmPlayer(other)`: whether `player` may hurt `other`: the `pvp` game
    /// rule is on, and their teams allow it (allies on a team without friendly fire cannot,
    /// `Player.canHarmPlayer`). Attacks, arrows and tridents ask the same. None when either
    /// handle is stale or unavailable.
    fn player_can_harm(&self, player: PlayerRef, other: PlayerRef) -> Option<bool>;
    /// Side: both.
    /// Whether `entity` is within the player's reach by the server's rule
    /// `reach` ([`EntityReach`]): from the player's eyes to the closest point
    /// of the entity's box, with the rule's own buffer, the range from the
    /// player's `minecraft:entity_interaction_range` (3.0, 5.0 in creative)
    /// and, for an attack, the main hand's `attack_range` component. With the
    /// eyes inside the box the distance is 0; there is no ray and no other
    /// fallback. This is the server's check, not the client's pick. None for a
    /// stale player or entity, one whose box Sunrise doesn't keep, or one in
    /// another level. On a client, `player` must be the local player: the range and main hand
    /// are its synced copy, its eye position is predicted, and the target's box is its client copy.
    fn player_within_entity_reach(
        &self,
        player: PlayerRef,
        entity: EntityRef,
        reach: EntityReach,
    ) -> Option<bool>;
}
/// The handles' writes of the `player_interaction` area, implemented by the game and reached through [`BackendMut`].
// hidden: a handle's backend, reached through `Backend`; mods call the handle, never this trait
#[doc(hidden)]
#[sunrise_mod_api_macros::host_area]
pub trait PlayerInteractionBackendMut {
    /// Side: both.
    /// `BlockBehaviour.getDestroyProgress(state, player, level, pos)`: the share of `state`
    /// one tick of mining removes at `pos`, using the tool in `player`'s selected hotbar
    /// slot (never the offhand), the block's hardness, efficiency, effects, block-break
    /// speed, submerged mining and the on-ground penalty, then the `can_harvest_block`,
    /// `player_harvest_check` and `player_break_speed` hooks; or, in place of all that, a block
    /// Part's `destroy_progress` answer for `state`, except from inside such an answer, where it
    /// is that math. `0.0` for an unbreakable state. Nothing requires `pos` to hold `state`. A
    /// server asks a connected or fake player (their state swapped in); a client only its local
    /// player.
    /// None for a stale handle or player, an unknown block state, a dead server player, a
    /// level other than the one in place, and a call from inside one of those hooks, which
    /// would run it again without bound.
    fn player_destroy_progress(
        &mut self,
        player: PlayerRef,
        level: LevelRef,
        pos: [i32; 3],
        state: BlockState,
    ) -> Option<f32>;
    /// Side: server.
    /// `BlockState.getCloneItemStack(level, pos, include_data)` for `player`: the stack Pick
    /// Block gives for the block at `pos`, the answer `ServerGamePacketListenerImpl
    /// .handlePickItemFromBlock` takes, before it copies block-entity data onto it
    /// (`addBlockDataToItem`). It is the block's own item (a crop's seed, a piston head's
    /// piston...) unless a block Part's `clone_item_stack` answers in its place, so it **runs
    /// that hook** with this player. `include_data` is what the block is told, true for a
    /// creative pick with Ctrl. Neither reach nor game mode is checked, and nothing is given.
    /// None for an empty stack (air), a stale handle or player, a dead player, a level other
    /// than the one in place, an unloaded position and a client.
    fn player_clone_item_stack(
        &mut self,
        player: PlayerRef,
        level: LevelRef,
        pos: [i32; 3],
        include_data: bool,
    ) -> Option<ItemStack>;
    /// Side: both.
    /// `Player.hasCorrectToolForDrops(state, level, pos)` and `IBlockExtension.canHarvestBlock`
    /// in one answer: whether the tool in `player`'s selected hotbar slot (never the offhand)
    /// harvests `state` at `pos` in `level`, the answer the game itself uses for the dig's
    /// progress, the break's drops and a door's break. It is vanilla's rule (true when the block
    /// needs no tool, else the held stack's `minecraft:tool` rules), then the block's
    /// `can_harvest_block` parts, then the global `player_harvest_check`, so it **runs those
    /// hooks** with this player. Sunrise's sites all share this one answer, unlike NeoForge's
    /// break progress, which skips the block override. Nothing requires `pos` to hold `state`. On
    /// a server a connected or fake player can be asked; on a client only the local player. None
    /// for a stale handle or player, an unknown block state, a dead server player, a level other than the one in place,
    /// a player the client has no state for, and a call from inside either harvest hook
    /// (`can_harvest_block`, `player_harvest_check`), even when asking about a different state.
    fn player_has_correct_tool_for_drops(
        &mut self,
        player: PlayerRef,
        level: LevelRef,
        pos: [i32; 3],
        state: BlockState,
    ) -> Option<bool>;
    /// Side: server.
    /// `ServerPlayerGameMode.destroyBlock`: breaks the block at `pos` in the player's level as
    /// this player breaks it, as FTB Ultimine does for each block of a vein. On a live block,
    /// `break_block` posts in NeoForge `CommonHooks.fireBlockBreak` order: pre-canceled when
    /// the main-hand item, game mode, or game-master permissions refuse the break, and hard
    /// game-mode and game-master restrictions still prevent removal even if a
    /// `#[listener(receive_cancelled)]` listener clears the cancel. Unlike a dig packet
    /// (`handleBlockBreakAction`), this does not check spawn protection, reach, or whether
    /// normal mining could finish the block.
    /// The break runs the block's effects, `player_destroyed_block`, the loot rolled for the
    /// main-hand item and its enchantments, `block_drops` with this player as the breaker, item
    /// wear, the mined statistic and exhaustion. A vein miner can take the drops and experience
    /// in `block_drops` and place them with `LevelRef::pop_resource` and
    /// `LevelRef::award_experience`. Indirect drops (such as a chest's contents) still spawn in
    /// the world. With `into_inventory`, the block's remaining item drops go to the player's
    /// inventory; overflow drops from them as in `Inventory.placeItemBackInInventory`.
    /// Chained calls fire the break hooks without a fixed nesting limit. A break of the same
    /// block from inside its own `break_block` callback does not call it again: it removes the
    /// block as a nested break and the outer call then does nothing more. The position is read
    /// from the level (a column that is not resident is loaded to read it), and `break_block`
    /// fires for what it holds, air included.
    /// True only if this call removed a block. False off the server, for a stale, dead or
    /// out-of-level player, for air, on a restriction or veto, or
    /// if a listener already removed the block or completed a nested player break at that
    /// position in that level (even if a non-air block or fluid remains).
    fn player_destroy_block(
        &mut self,
        player: PlayerRef,
        pos: [i32; 3],
        into_inventory: bool,
    ) -> bool;
    /// Side: server.
    /// `CommonHooks.fireBlockBreak`: whether the player's break of the block at `pos` in their
    /// level would be refused or canceled, asked as `ServerPlayerGameMode.destroyBlock` asks it,
    /// without breaking anything, for a mod that breaks the block itself afterwards. True when
    /// the player's checks refuse it (a spectator, an adventure player whose tool can't break it,
    /// a held item that can't break blocks, a game master block without the permission) or a
    /// `break_block` listener cancels it; those listeners hear it as a break. As in vanilla, a
    /// held debug stick takes the break as a click and selects its next property. Asked from
    /// inside eight other breaks or asks, it is refused, as `player_destroy_block` is. None for
    /// a stale handle, a player who isn't here, is dead or is in another level than the one in
    /// place, an unloaded block, a refused nesting and a client.
    fn player_break_canceled(&mut self, player: PlayerRef, pos: [i32; 3]) -> Option<bool>;
    /// Side: server.
    /// `EventHooks.onBlockPlace`: whether a mod cancels the player placing `state` at `pos`, where
    /// the block goes, having clicked the `face` of the block `pos` lies against (the block one
    /// step from `pos` opposite `face`), without placing anything, for a mod that sets the block
    /// itself afterwards. `entity_place` hears it as a single placement by this player against
    /// the block at the clicked position. Nothing else is checked: ask `player_may_use_item_at`
    /// for the game's own rule. Asked from inside eight other breaks or asks, it is refused. None
    /// for a stale handle, a player who isn't here, is dead or is in another level than the one
    /// in place, an unknown state, an unloaded block, a refused nesting and a client.
    fn player_place_canceled(
        &mut self,
        player: PlayerRef,
        pos: [i32; 3],
        face: BlockFace,
        state: BlockState,
    ) -> Option<bool>;
    /// Side: server.
    /// Acknowledges the player's block actions up to `sequence`
    /// (`ServerGamePacketListenerImpl.ackBlockChangesUpTo`), for a mod that handled one itself,
    /// such as an editor's own message: the client's predicted blocks settle on the server's
    /// states once it gets `ClientboundBlockChangedAckPacket`, which goes after the next tick's
    /// block changes. Pending acknowledgements take the maximum: a lower sequence after a
    /// higher one has no effect. Any nonnegative sequence is accepted, so a value too high can
    /// end pending client predictions early. Refuses, queueing nothing, with
    /// `PlayerWorldRefusal`: `Client`, `Stale`, `NoConnection` and `NegativeSequence`.
    fn player_ack_block_changes(
        &mut self,
        player: PlayerRef,
        sequence: i32,
    ) -> Result<(), PlayerWorldRefusal>;
    /// Side: server.
    /// Shows that player alone `state` at `pos` in their level (`ClientboundBlockUpdatePacket`
    /// to their client; Paper's `Player.sendBlockChange`), at once. The world is not changed:
    /// the real block comes back when it or its chunk is next sent, as in vanilla, so a mod
    /// sends it again while it still wants it shown. The visual override is not remembered;
    /// it does not itself acknowledge or clear pending client predictions.
    /// Refuses, sending nothing, with `PlayerWorldRefusal`: `Client`, `Stale`, `NoConnection`, `UnknownState` and
    /// `PositionNotSendable` (y outside -2048..=2047, x or z outside -33554432..33554432).
    fn player_show_block(
        &mut self,
        player: PlayerRef,
        pos: [i32; 3],
        state: BlockState,
    ) -> Result<(), PlayerWorldRefusal>;
    /// Side: server.
    /// Writes `state` at `pos` in `level` for the player, for a state the mod already chose
    /// (an editor's or a schematic's). It is `level_set_block` with a placement's flags
    /// (`NEIGHBORS | CLIENTS`, plus `KNOWN_SHAPE` when `update_neighbour_shapes` is false; without
    /// it the neighbours' shapes update, so fences, walls and stairs connect), so the block entity
    /// follows that write's rules, and then `Block.setPlacedBy` of the block that stands there, run in
    /// the player's turn (a bed places its head, a door its upper half). Nothing else of vanilla's `BlockItem.place`
    /// or `useOn` runs: no check that the block may be placed there (survival, entity
    /// obstruction, build permission, spawn protection), no `placed_block` advancement trigger or
    /// statistic, generic item placement sound or game event, or item use or consumption.
    /// The block callback may produce its own effects.
    /// `setPlacedBy` is not given the player as placer. The player is only whose turn `setPlacedBy` runs in, so it is
    /// not "as player" in the sense of a use-on. The state is written as given. Refuses, writing
    /// nothing, with `PlayerWorldRefusal`: `Client`, `Stale`, `Dead`, `StaleLevel`, `OtherLevel`
    /// (that `level` is not the level in place: both placement steps, the write
    /// and `setPlacedBy`, use the supplied level), `UnknownState`, `OutOfWorld`, `NotLoaded`,
    /// `Fenced` (the origin or a direct companion or callback write is held by a carrier transfer),
    /// `InTransaction` (a completed wither pattern in an open Host transaction), and `Unchanged`
    /// (that state already stands there, or `level_set_block` returns false). Neighbour updates
    /// follow the level's normal rules.
    fn player_place_block(
        &mut self,
        player: PlayerRef,
        level: LevelRef,
        pos: [i32; 3],
        state: BlockState,
        update_neighbour_shapes: bool,
    ) -> Result<(), PlayerWorldRefusal>;
}
