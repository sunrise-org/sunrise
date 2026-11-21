//! The `level_datapacks` area: data-driven lookups: datapack registry entries, recipe matching and loot tables. Host method declarations; access defaults and game forwarders are generated.
#![allow(unused_variables)]
use crate::*;
/// The handles' reads of the `level_datapacks` area, implemented by the game and reached through [`Backend`].
// hidden: a handle's backend, reached through `Backend`; mods call the handle, never this trait
#[doc(hidden)]
#[sunrise_mod_api_macros::host_area]
pub trait LevelDatapacksBackend {
    /// Side: both.
    /// The entry ids (`addon:fire`) of a datapack registry a mod declared
    /// (`declare_datapack_registry`), sorted, as this world session last loaded them. On a client
    /// they are the ones its server sent at the join and after each reload, and only for a
    /// registry declared `Required` or `Optional`. None for a stale or parked level and a
    /// registry no mod declared (or, on a client, one the server did not send). Reads nothing
    /// from disk.
    #[access(WorldOnly)]
    fn level_datapack_entries(&self, level: LevelRef, registry: &str) -> Option<Vec<String>>;
    /// Side: both.
    /// A handle to `entry` of `registry` at the generation now loaded (on a client, the one its
    /// server last sent). A handle kept across a reload fails its next read with
    /// `StaleGeneration`, one from an earlier world session with `WrongSession`. A stale or parked
    /// level and a registry no mod declared are `UnknownRegistry`; an entry no pack defines is
    /// `MissingEntry`.
    #[access(WorldOnly)]
    fn level_datapack_handle(
        &self,
        level: LevelRef,
        registry: &str,
        entry: &str,
    ) -> Result<DatapackHandle, DatapackReadError>;
    /// Side: both.
    /// The decoded fields of the entry a handle names, in the registry's schema order, as plain
    /// values: a copy, which carries no authority after the read. The handle is checked first
    /// (session, then generation), then the entry; the errors are those of
    /// [`level_datapack_handle`](Self::level_datapack_handle).
    #[access(WorldOnly)]
    fn level_datapack_read(
        &self,
        level: LevelRef,
        handle: &DatapackHandle,
    ) -> Result<Vec<(String, DatapackValue)>, DatapackReadError>;
    /// Side: server.
    /// Match and assemble through the engine's recipe matcher, without consuming inputs.
    /// Covers minecraft:crafting, smelting, blasting, smoking, campfire_cooking,
    /// stonecutting and smithing, including data-pack recipes of those types.
    /// Registered crafting serializers match through minecraft:crafting. Read-only and unregistered
    /// mod-owned types return RecipeLookupError::NotMatchable; cache recipes_of_type once
    /// per load/reload and match those in mod memory (no per-tick host crossing).
    /// A valid input without a match returns Ok(None); invalid shapes/items and unavailable
    /// levels are typed refusals. Crafting dimensions are 1 through 3, with row-major slots.

    #[access(WorldOnly)]
    fn level_recipe_for(
        &self,
        level: LevelRef,
        recipe_type: &Identifier,
        inputs: &RecipeInput,
    ) -> Result<Option<RecipeMatch>, RecipeLookupError>;
}
/// The handles' writes of the `level_datapacks` area, implemented by the game and reached through [`BackendMut`].
// hidden: a handle's backend, reached through `Backend`; mods call the handle, never this trait
#[doc(hidden)]
#[sunrise_mod_api_macros::host_area]
pub trait LevelDatapacksBackendMut {
    /// Side: server.
    /// One roll of the loot table `table` (`mymod:sifting/gravel`, the world's data packs
    /// merged), handed back instead of dropped: `LootTable.getRandomItems(LootParams)` with
    /// `origin` as ORIGIN, `tool` as TOOL when given and `entity` as THIS_ENTITY when given,
    /// the context `/loot … fish <table> <pos> <tool>` gives. The roll draws from the table's
    /// random sequence, else the level random, as vanilla's does, so it moves later draws: that
    /// is why this is a write. The stacks come split to their stack sizes, empties left out.
    /// Mods' loot kinds see `origin` and this level; no `loot_drops` event, since nothing
    /// drops. None for a table that isn't there, a stale or parked handle, a client, and a
    /// stale `entity`.
    #[access(WorldOnly)]
    fn level_roll_loot(
        &mut self,
        level: LevelRef,
        table: &str,
        origin: [f64; 3],
        tool: Option<&ItemStack>,
        entity: Option<EntityRef>,
    ) -> Result<Option<Vec<ItemStack>>, StackWriteError>;
}
