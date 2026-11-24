//! Read-only client session and local-player status.

use crate::{BlockPos, GameMode, Text, TextFormError};
use core::ops::ControlFlow;

/// Why a client-session read could not run.
// no wire: this refusal is returned by in-process client-session reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ClientSessionReadError {
    /// The current phase or backend does not allow this read.
    Unavailable,
}

impl crate::NoBackend for ClientSessionReadError {
    fn no_backend() -> Self {
        Self::Unavailable
    }
}

// no wire: borrows the current client connection and integrated world.
/// The connected server, not the reconnect target of an earlier session.
/// Its text borrows the host; copy it only to retain it beyond that borrow.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
// exhaustive: the game builds it with a literal; mods only read it
pub struct ClientServer<'a> {
    /// The integrated world's current display name. External connections do not retain
    /// a server display name; this is `None`, never the connecting player's name.
    pub name: Option<&'a str>,
    /// The joined address, with friend-invite secrets redacted. Memory
    /// connections have no address (`None`), not a made-up socket address.
    pub address: Option<&'a str>,
    /// This window owns the connected integrated server. A loopback address
    /// alone does not make this true.
    pub integrated_local: bool,
    /// Folder name relative to the instance's saves directory for the owned
    /// integrated world. Renaming the world in-game leaves this key unchanged.
    /// Connections to a server this window does not own return `None`.
    pub save_folder: Option<&'a str>,
}

/// The local player's item use as the client sees it (`LivingEntity.isUsingItem`,
/// `getUsedItemHand`, `getTicksUsingItem`, `getUseItemRemainingTicks`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub struct ItemUse {
    /// The hand holding the item in use.
    pub hand: crate::Hand,
    /// Ticks the item has been used so far.
    pub ticks_used: i32,
    /// Ticks left until the use completes; with `ticks_used`, the item's use duration.
    pub ticks_remaining: i32,
}

impl ItemUse {
    /// An item use of `hand`, `ticks_used` ticks old with `ticks_remaining` left.
    pub fn new(hand: crate::Hand, ticks_used: i32, ticks_remaining: i32) -> Self {
        Self {
            hand,
            ticks_used,
            ticks_remaining,
        }
    }
}

/// Existing local frame counters, sampled only when requested by a mod.
#[derive(Clone, Copy, Debug, PartialEq)]
// exhaustive: the game builds it with a literal; mods only read it
#[derive(sunrise_mod_api_macros::Wire)]
pub struct ClientFrameStats {
    /// The game's most recently computed frames per second (updated about once
    /// a second), not an inverse of a single frame time.
    pub fps: f32,
    /// Frames rendered by this window since it started, including menus.
    pub rendered_frames: u64,
    /// The most recent completed frame's wall interval in nanoseconds, including
    /// waits. `None` before a frame sample exists or after the chart was reset.
    pub frame_time_ns: Option<u64>,
}

/// The local player's current block-breaking operation. An unavailable player
/// returns `None` from the handle, distinct from [`Self::Idle`].
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub enum BlockBreakingState {
    /// No block is currently being broken.
    Idle,
    /// The client's current breaking target and predicted progress.
    Breaking {
        /// The target block.
        position: BlockPos,
        /// The accumulated breaking fraction, as the client holds it.
        progress: f32,
        /// The visible crack stage, 0 through 9; absent while progress is zero.
        stage: Option<u8>,
    },
}

/// The live player list as the Tab overlay draws it (`PlayerTabOverlay`), read with
/// [`crate::ClientSession::client_tab_list`]. It reflects what the server has sent.
// no wire: the view lends the current list and overlay text in-process.
pub trait ClientTabList {
    /// Visit the listed players in drawn order (`getPlayerInfos`: higher list order first,
    /// then non-spectators before spectators, team and profile name), at most the 80 it draws.
    /// Each row is valid only during its callback. Return `Break(())` to stop; an empty list
    /// returns `Continue(())` without calling the visitor.
    fn visit_entries(
        &self,
        visit: &mut dyn FnMut(&dyn ClientTabListEntry) -> ControlFlow<()>,
    ) -> ControlFlow<()>;

    /// Copy the header the server set. `Ok(None)` means absent or cleared; a conversion
    /// refusal affects only this copy.
    fn copy_header(&self) -> Result<Option<Text>, TextFormError>;

    /// Copy the footer the server set. `Ok(None)` means absent or cleared; a conversion
    /// refusal affects only this copy.
    fn copy_footer(&self) -> Result<Option<Text>, TextFormError>;
}

/// One listed player (`PlayerInfo`) as the Tab overlay shows it.
// no wire: a row lends the player's current information for one callback.
pub trait ClientTabListEntry {
    /// Borrow the profile name (`GameProfile.name`); copy it only if retaining it.
    fn name(&self) -> &str;

    /// The profile UUID.
    fn uuid(&self) -> u128;

    /// The game mode the server reported.
    fn game_mode(&self) -> GameMode;

    /// The latency in milliseconds the ping icon shows; negative when unknown.
    fn latency(&self) -> i32;

    /// Copy the name as drawn (`getNameForDisplay`): the display name the server sent, or
    /// the profile name with its team's prefix, suffix and colour; italic for a spectator.
    /// A conversion refusal affects only this copy; there is no plaintext fallback.
    fn copy_display(&self) -> Result<Text, TextFormError>;
}
