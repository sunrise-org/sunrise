//! Registration and lookup plumbing for mod-declared data maps (NeoForge's
//! `RegisterDataMapTypesEvent`).
use super::*;
use crate::world::data_maps::{DataMap, RawDataMap, RegisteredDataMap};

/// Object-safe low-level registrar trait for data maps.
/// Mods call [`Registrar::register_data_map`] or [`Registrar::register_data_map_by_name`] instead.
// hidden: typed helpers live on `dyn Registrar`; this object-safe layer is internal plumbing.
#[doc(hidden)]
pub trait DataMapsRegistrar {
    /// Register a data map descriptor lowered from [`DataMap`] or [`RawDataMap`].
    fn register_data_map_erased(&mut self, map: RawDataMap);
}

impl dyn Registrar + '_ {
    /// Register a typed data map for a built-in registry (`ItemKey`, `BlockKey`, ...) or a
    /// mod-declared custom static registry (`EntryKey<T>` where `T: CustomRegistryEntry`)
    /// (NeoForge's `RegisterDataMapTypesEvent#register`).
    ///
    /// Both `id` and the target registry must be valid namespaced identifiers; reserved
    /// built-in `(registry, id)` pairs, duplicate `(registry, id)` registrations, synced
    /// maps on non-synced or reloadable datapack registries, and `DataMapSync::Required`
    /// maps from a `side = "server"` package are refused at startup naming the mod.
    pub fn register_data_map<M: DataMap>(&mut self, map: M) {
        self.register_data_map_erased(RawDataMap::from_typed(map));
    }

    /// Register a by-name data map for a registry that has no typed key (such as a
    /// mod-declared datapack registry), paired with [`data_map_value`](crate::data_map_value)
    /// and [`data_map_entries`](crate::data_map_entries).
    ///
    /// Registries with a typed key use [`Registrar::register_data_map`].
    pub fn register_data_map_by_name(&mut self, map: RawDataMap) {
        self.register_data_map_erased(map);
    }
}

/// All startup-frozen data maps in the current installed epoch (`TEST_PARTS` first, then
/// the process-wide `INSTALLED`).
pub fn installed_data_maps() -> &'static [Arc<RegisteredDataMap>] {
    installed().map_or(&[], |installed| &installed.data_maps)
}
