//! The `player_original` area: the player a clone hook copies from: what it was before the data copy. Host method declarations; access defaults and game forwarders are generated.
#![allow(unused_variables)]
use crate::*;
/// The handles' reads of the `player_original` area, implemented by the game and reached through [`Backend`].
// hidden: a handle's backend, reached through `Backend`; mods call the handle, never this trait
#[doc(hidden)]
#[sunrise_mod_api_macros::host_area]
pub trait PlayerOriginalBackend {
    /// Side: server.
    /// Original UUID while its clone hook is active, otherwise None.
    #[access(WorldOnly)]
    fn original_player_uuid(&self, original_player: OriginalPlayerRef) -> Option<u128>;
    /// Side: server.
    /// Original position before the data copy, otherwise None.
    #[access(WorldOnly)]
    fn original_player_position(&self, original_player: OriginalPlayerRef) -> Option<[f64; 3]>;
    /// Side: server.
    /// Original dimension before the data copy, otherwise None.
    #[access(WorldOnly)]
    fn original_player_dimension(&self, original_player: OriginalPlayerRef) -> Option<Identifier>;
    /// Side: server.
    /// Original level before the data copy, otherwise None.
    #[access(WorldOnly)]
    fn original_player_level(&self, original_player: OriginalPlayerRef) -> Option<LevelRef>;
    /// Side: server.
    /// Original inventory slot: 0..35 main, 100..103 feet through head, -106 offhand.
    /// None for an empty or invalid slot or an expired clone hook.
    #[access(WorldOnly)]
    fn original_player_inventory_slot(
        &self,
        original_player: OriginalPlayerRef,
        slot: i32,
    ) -> Option<ItemStack>;
    /// Side: server.
    /// Original health at death or End return; None after the clone hook.
    #[access(WorldOnly)]
    fn original_player_health(&self, original_player: OriginalPlayerRef) -> Option<f32>;
    /// Side: server.
    /// Original experience at death or End return; None after the clone hook.
    #[access(WorldOnly)]
    fn original_player_experience(
        &self,
        original_player: OriginalPlayerRef,
    ) -> Option<PlayerExperience>;
    /// Side: server.
    /// Original active effects at death or End return; None after the clone hook.
    #[access(WorldOnly)]
    fn original_player_effects(
        &self,
        original_player: OriginalPlayerRef,
    ) -> Option<Vec<ActiveEffect>>;
}
