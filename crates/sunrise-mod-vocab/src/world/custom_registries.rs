//! Code-registered, startup-frozen custom static registries (NeoForge's `NewRegistryEvent` /
//! `RegistryBuilder<T>`).
//!
//! A library mod defines a [`RegistryKey<T>`], declares the registry during [`Hooks::register`](crate::Hooks::register)
//! via [`Registrar::declare_registry`](crate::Registrar::declare_registry), and dependent mods
//! register typed entries with [`Registrar::register_entry`](crate::Registrar::register_entry).
//! Once startup registration finishes, every registry freezes into an immutable [`CustomRegistry<T>`]
//! readable lock-free through [`HostRegistries::registry`].
use crate::Identifier;
use std::{
    any::{Any, TypeId, type_name},
    cmp::Ordering,
    collections::HashMap,
    fmt,
    hash::{Hash, Hasher},
    marker::PhantomData,
    sync::Arc,
};

/// Sentinel slot used for [`EntryKey`] when registration was refused.
#[cfg(test)]
pub(crate) const INVALID_SLOT: u32 = u32::MAX;

/// Compile-time typed key naming a mod-declared static registry (e.g. `chemlib:gas`).
///
/// Publish a `pub const` of this type from a library mod so dependent mods share one
/// canonical registry ID and Rust value type `T`.
// no wire: process-local typed handle naming a static registry.
// exhaustive: opaque key constructed via `RegistryKey::new`.
pub struct RegistryKey<T: 'static> {
    id: &'static str,
    _marker: PhantomData<fn() -> T>,
}

impl<T: 'static> RegistryKey<T> {
    /// Construct a typed key for the static registry with canonical namespaced `id`.
    pub const fn new(id: &'static str) -> Self {
        Self {
            id,
            _marker: PhantomData,
        }
    }

    /// The canonical namespaced registry identifier (e.g. `"chemlib:gas"`).
    pub const fn id(&self) -> &'static str {
        self.id
    }
}

impl<T: 'static> Copy for RegistryKey<T> {}

impl<T: 'static> Clone for RegistryKey<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T: 'static> fmt::Debug for RegistryKey<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("RegistryKey").field(&self.id).finish()
    }
}

impl<T: 'static> PartialEq for RegistryKey<T> {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

impl<T: 'static> Eq for RegistryKey<T> {}

impl<T: 'static> Hash for RegistryKey<T> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.id.hash(state);
    }
}

/// Dense, `Copy` handle to one registered entry in a [`CustomRegistry<T>`].
///
/// Handles are process-local slots valid only for the current startup: never persist raw
/// keys to disk or send them across a network. Persist [`CustomRegistry::id_of`] (a canonical
/// namespaced [`Identifier`]) and resolve it with [`CustomRegistry::by_id`] on load.
// no wire: process-local slot handle; persist the namespaced id from `CustomRegistry::id_of`.
// exhaustive: opaque slot handle created only by registration and registry lookup.
pub struct EntryKey<T: 'static> {
    registry_slot: u32,
    entry_slot: u32,
    _marker: PhantomData<fn() -> T>,
}

impl<T: 'static> EntryKey<T> {
    pub(crate) const fn new(registry_slot: u32, entry_slot: u32) -> Self {
        Self {
            registry_slot,
            entry_slot,
            _marker: PhantomData,
        }
    }

    #[cfg(test)]
    pub(crate) const fn invalid() -> Self {
        Self::new(INVALID_SLOT, INVALID_SLOT)
    }

    pub(crate) const fn slots(self) -> (u32, u32) {
        (self.registry_slot, self.entry_slot)
    }
}

impl<T: 'static> Copy for EntryKey<T> {}

impl<T: 'static> Clone for EntryKey<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T: 'static> fmt::Debug for EntryKey<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EntryKey")
            .field("registry_slot", &self.registry_slot)
            .field("entry_slot", &self.entry_slot)
            .finish()
    }
}

impl<T: 'static> PartialEq for EntryKey<T> {
    fn eq(&self, other: &Self) -> bool {
        self.registry_slot == other.registry_slot && self.entry_slot == other.entry_slot
    }
}

impl<T: 'static> Eq for EntryKey<T> {}

impl<T: 'static> Hash for EntryKey<T> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.registry_slot.hash(state);
        self.entry_slot.hash(state);
    }
}

impl<T: 'static> PartialOrd for EntryKey<T> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl<T: 'static> Ord for EntryKey<T> {
    fn cmp(&self, other: &Self) -> Ordering {
        (self.registry_slot, self.entry_slot).cmp(&(other.registry_slot, other.entry_slot))
    }
}

/// Options passed to [`Registrar::declare_registry`](crate::Registrar::declare_registry),
/// matching NeoForge's `RegistryBuilder`.
// no wire: startup-only declaration options.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct RegistryOptions {
    default_key: Option<String>,
    sync: bool,
}

impl RegistryOptions {
    /// Default options: no default entry and not synced to clients (`sync(false)`).
    pub fn new() -> Self {
        Self::default()
    }

    /// Configure a fallback default entry ID (`RegistryBuilder.defaultKey`).
    ///
    /// The named entry must be registered in this registry before startup finishes;
    /// otherwise startup fails naming the declaring mod, registry ID, and missing default key.
    pub fn default_key(mut self, id: impl Into<String>) -> Self {
        self.default_key = Some(id.into());
        self
    }

    /// Whether the server syncs this registry's entry IDs and tags to Sunrise clients
    /// during configuration (`RegistryBuilder.sync`).
    pub fn sync(mut self, sync: bool) -> Self {
        self.sync = sync;
        self
    }

    /// Configured default entry ID, if any.
    pub fn configured_default_key(&self) -> Option<&str> {
        self.default_key.as_deref()
    }

    /// Whether network sync is enabled.
    pub fn is_synced(&self) -> bool {
        self.sync
    }
}

/// Immutable, lock-free snapshot of a frozen static custom registry.
// no wire: process-local frozen registry holding arbitrary typed mod values.
// exhaustive: constructed only when startup registration freezes.
pub struct CustomRegistry<T: Send + Sync + 'static> {
    slot: u32,
    id: Identifier,
    id_str: String,
    tag_directory: String,
    sync: bool,
    default_entry_slot: Option<u32>,
    entry_ids: Vec<Identifier>,
    values: Vec<T>,
    by_id: HashMap<String, u32>,
}

impl<T: Send + Sync + 'static> fmt::Debug for CustomRegistry<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CustomRegistry")
            .field("id", &self.id)
            .field("sync", &self.sync)
            .field("len", &self.values.len())
            .finish()
    }
}

impl<T: Send + Sync + 'static> CustomRegistry<T> {
    pub(crate) fn from_frozen(
        slot: u32,
        id: Identifier,
        sync: bool,
        default_entry_slot: Option<u32>,
        entry_ids: Vec<Identifier>,
        values: Vec<T>,
        by_id: HashMap<String, u32>,
    ) -> Self {
        let id_str = id.to_string();
        let tag_directory = format!("{}/{}", id.namespace(), id.path());
        Self {
            slot,
            id,
            id_str,
            tag_directory,
            sync,
            default_entry_slot,
            entry_ids,
            values,
            by_id,
        }
    }

    /// Canonical namespaced identifier of this registry.
    pub fn id(&self) -> &Identifier {
        &self.id
    }

    /// Whether this registry is synced to clients during configuration.
    pub fn is_synced(&self) -> bool {
        self.sync
    }

    /// Number of registered entries.
    pub fn len(&self) -> usize {
        self.values.len()
    }

    /// Whether this registry has no registered entries.
    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    /// O(1) dense slot lookup. Returns `None` if `key` came from another registry
    /// or a refused registration; never panics on mod input.
    pub fn get(&self, key: EntryKey<T>) -> Option<&T> {
        let (registry_slot, entry_slot) = key.slots();
        if registry_slot != self.slot {
            return None;
        }
        self.values.get(entry_slot as usize)
    }

    /// [`get`](Self::get), falling back to [`default_entry`](Self::default_entry) when a
    /// `default_key` was configured.
    pub fn get_or_default(&self, key: EntryKey<T>) -> Option<&T> {
        self.get(key)
            .or_else(|| self.default_entry().map(|(_, _, value)| value))
    }

    /// Look up an entry by canonical namespaced identifier (e.g. `"boiler:steam"`).
    pub fn by_id(&self, id: &str) -> Option<(EntryKey<T>, &T)> {
        let &entry_slot = self.by_id.get(id)?;
        let value = self.values.get(entry_slot as usize)?;
        Some((EntryKey::new(self.slot, entry_slot), value))
    }

    /// [`by_id`](Self::by_id), falling back to [`default_entry`](Self::default_entry) when a
    /// `default_key` was configured.
    pub fn by_id_or_default(&self, id: &str) -> Option<(EntryKey<T>, &T)> {
        self.by_id(id)
            .or_else(|| self.default_entry().map(|(key, _, value)| (key, value)))
    }

    /// The configured default entry, if [`RegistryOptions::default_key`] was set.
    pub fn default_entry(&self) -> Option<(EntryKey<T>, &Identifier, &T)> {
        let slot = self.default_entry_slot?;
        let id = self.entry_ids.get(slot as usize)?;
        let value = self.values.get(slot as usize)?;
        Some((EntryKey::new(self.slot, slot), id, value))
    }

    /// Canonical namespaced identifier for `key` (what saves persist), or `None` if `key`
    /// does not belong to this registry.
    pub fn id_of(&self, key: EntryKey<T>) -> Option<&Identifier> {
        let (registry_slot, entry_slot) = key.slots();
        if registry_slot != self.slot {
            return None;
        }
        self.entry_ids.get(entry_slot as usize)
    }

    /// Iterate `(EntryKey<T>, &Identifier, &T)` in deterministic registration order.
    pub fn iter(&self) -> impl ExactSizeIterator<Item = (EntryKey<T>, &Identifier, &T)> {
        let slot = self.slot;
        self.entry_ids
            .iter()
            .zip(self.values.iter())
            .enumerate()
            .map(move |(index, (id, value))| (EntryKey::new(slot, index as u32), id, value))
    }
}

/// Frozen, type-erased view of one static custom registry, plugged into engine lookups
/// (tags, data maps, load conditions, configuration sync).
///
/// Engine plumbing only: exposed via `__private`, never re-exported from `mod_api`.
// no wire: engine-internal trait object over frozen static custom registries.
pub trait RegistryDirectory: Send + Sync {
    /// Canonical namespaced registry identifier (`"chemlib:gas"`).
    fn id(&self) -> &str;
    /// Whether this registry syncs to Sunrise clients during configuration.
    fn sync(&self) -> bool;
    /// Canonical namespaced entry IDs in deterministic registration order (index = raw ID).
    fn entry_ids(&self) -> &[Identifier];
    /// Whether `id` is registered in this registry.
    fn contains_id(&self, id: &str) -> bool;
    /// Tag folder under `data/<ns>/tags/`: `<reg_ns>/<reg_path>`, computed once at freeze.
    fn tag_directory(&self) -> &str;
    /// Downcast handle to the concrete [`CustomRegistry<T>`].
    fn as_any(&self) -> &(dyn Any + Send + Sync);
}

struct FrozenCustomRegistryDir<T: Send + Sync + 'static>(CustomRegistry<T>);

impl<T: Send + Sync + 'static> RegistryDirectory for FrozenCustomRegistryDir<T> {
    fn id(&self) -> &str {
        &self.0.id_str
    }

    fn sync(&self) -> bool {
        self.0.sync
    }

    fn entry_ids(&self) -> &[Identifier] {
        &self.0.entry_ids
    }

    fn contains_id(&self, id: &str) -> bool {
        self.0.by_id.contains_key(id)
    }

    fn tag_directory(&self) -> &str {
        &self.0.tag_directory
    }

    fn as_any(&self) -> &(dyn Any + Send + Sync) {
        &self.0
    }
}

/// Sealed type identity and matching freezer captured together at
/// [`Registrar::declare_registry`](crate::Registrar::declare_registry), so a registry's
/// registered [`TypeId`] and freeze-time downcast type cannot disagree.
// no wire: startup-only typed freezer token.
// exhaustive: opaque token constructed only via `CustomRegistryFreezer::of::<T>()`.
pub struct CustomRegistryFreezer {
    type_id: TypeId,
    type_name: &'static str,
    freeze: fn(
        u32,
        Identifier,
        bool,
        Option<u32>,
        Vec<(Identifier, Box<dyn Any + Send + Sync>)>,
    ) -> Arc<dyn RegistryDirectory>,
}

impl CustomRegistryFreezer {
    pub(crate) fn of<T: Send + Sync + 'static>() -> Self {
        Self {
            type_id: TypeId::of::<T>(),
            type_name: type_name::<T>(),
            freeze: freeze_typed::<T>,
        }
    }

    /// Identity of the value type accepted by this registry's freezer.
    pub fn type_id(&self) -> TypeId {
        self.type_id
    }

    /// Diagnostic name of the value type accepted by this registry's freezer.
    pub fn type_name(&self) -> &'static str {
        self.type_name
    }

    /// Freeze the entries already checked against this registry's value type at registration.
    pub fn freeze(
        &self,
        slot: u32,
        id: Identifier,
        sync: bool,
        default_entry_slot: Option<u32>,
        entries: Vec<(Identifier, Box<dyn Any + Send + Sync>)>,
    ) -> Arc<dyn RegistryDirectory> {
        (self.freeze)(slot, id, sync, default_entry_slot, entries)
    }
}

pub(crate) fn freeze_typed<T: Send + Sync + 'static>(
    slot: u32,
    id: Identifier,
    sync: bool,
    default_entry_slot: Option<u32>,
    entries: Vec<(Identifier, Box<dyn Any + Send + Sync>)>,
) -> Arc<dyn RegistryDirectory> {
    let mut entry_ids = Vec::with_capacity(entries.len());
    let mut values = Vec::with_capacity(entries.len());
    let mut by_id = HashMap::with_capacity(entries.len());
    for (index, (entry_id, boxed)) in entries.into_iter().enumerate() {
        by_id.insert(entry_id.to_string(), index as u32);
        entry_ids.push(entry_id);
        let value = *boxed
            .downcast::<T>()
            .expect("custom registry entry type was validated at registration");
        values.push(value);
    }
    Arc::new(FrozenCustomRegistryDir(CustomRegistry::from_frozen(
        slot,
        id,
        sync,
        default_entry_slot,
        entry_ids,
        values,
        by_id,
    )))
}

/// All frozen custom static registries held by an [`Installed`](crate::parts::Installed) epoch.
// no wire: engine-internal frozen table.
// exhaustive: constructed only by startup freeze.
#[derive(Default)]
pub struct FrozenCustomRegistries {
    directories: Vec<Arc<dyn RegistryDirectory>>,
    by_id: HashMap<String, usize>,
}

impl FrozenCustomRegistries {
    /// Construct from frozen registry directories in deterministic declaration order.
    pub fn from_directories(directories: Vec<Arc<dyn RegistryDirectory>>) -> Self {
        let by_id = directories
            .iter()
            .enumerate()
            .map(|(index, dir)| (dir.id().to_owned(), index))
            .collect();
        Self { directories, by_id }
    }

    /// All frozen registry directories in declaration order.
    pub fn directories(&self) -> &[Arc<dyn RegistryDirectory>] {
        &self.directories
    }

    /// Look up one frozen typed registry by canonical namespaced ID.
    pub fn get<T: Send + Sync + 'static>(&self, id: &str) -> Option<&CustomRegistry<T>> {
        let &index = self.by_id.get(id)?;
        self.directories[index]
            .as_any()
            .downcast_ref::<CustomRegistry<T>>()
    }
}

/// Read mod-declared static custom registries frozen at startup.
///
/// Implemented for every [`Host`] (including `&dyn Host`, `&dyn ServerHost`, and
/// `&dyn ClientHost`) as well as [`ReloadContext`](crate::ReloadContext).
pub trait HostRegistries {
    /// Look up the frozen static custom registry identified by `key`.
    ///
    /// Returns `None` if the registry was not declared (for example, an optional library
    /// mod is absent) or if `T` does not match the declaring mod's type.
    fn registry<T: Send + Sync + 'static>(&self, key: RegistryKey<T>)
    -> Option<&CustomRegistry<T>>;
}

impl<H: crate::prelude::TagsHost + ?Sized> HostRegistries for H {
    fn registry<T: Send + Sync + 'static>(
        &self,
        key: RegistryKey<T>,
    ) -> Option<&CustomRegistry<T>> {
        crate::parts::custom_registry::<T>(key.id())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, PartialEq, Eq)]
    struct Gas {
        color: u32,
    }

    #[test]
    fn custom_registry_lookup_default_and_cross_registry_safety() {
        let id_a = Identifier::parse("chemlib:gas").unwrap();
        let empty_id = Identifier::parse("chemlib:empty").unwrap();
        let steam_id = Identifier::parse("boiler:steam").unwrap();
        let dir_a = freeze_typed::<Gas>(
            0,
            id_a,
            true,
            Some(0),
            vec![
                (empty_id.clone(), Box::new(Gas { color: 0 })),
                (steam_id.clone(), Box::new(Gas { color: 0xEE_EE_EE })),
            ],
        );
        assert_eq!(dir_a.id(), "chemlib:gas");
        assert!(dir_a.sync());
        assert_eq!(dir_a.tag_directory(), "chemlib/gas");
        assert!(dir_a.contains_id("boiler:steam"));
        assert!(!dir_a.contains_id("boiler:absent"));

        let reg_a = dir_a
            .as_any()
            .downcast_ref::<CustomRegistry<Gas>>()
            .unwrap();
        assert_eq!(reg_a.len(), 2);
        assert!(!reg_a.is_empty());

        let (steam_key, steam) = reg_a.by_id("boiler:steam").unwrap();
        assert_eq!(steam.color, 0xEE_EE_EE);
        assert_eq!(reg_a.get(steam_key), Some(steam));
        assert_eq!(reg_a.id_of(steam_key), Some(&steam_id));

        // Cross-registry key (slot 1) and invalid sentinel key never panic and return None.
        let foreign_key = EntryKey::<Gas>::new(1, 0);
        assert_eq!(reg_a.get(foreign_key), None);
        assert_eq!(reg_a.id_of(foreign_key), None);
        assert_eq!(reg_a.get(EntryKey::invalid()), None);
        assert_eq!(reg_a.get_or_default(foreign_key), Some(&Gas { color: 0 }));
        assert_eq!(
            reg_a.by_id_or_default("boiler:absent"),
            Some((EntryKey::new(0, 0), &Gas { color: 0 }))
        );
        assert_eq!(
            reg_a
                .iter()
                .map(|(_, id, _)| id.to_string())
                .collect::<Vec<_>>(),
            vec!["chemlib:empty", "boiler:steam"]
        );
    }
}
