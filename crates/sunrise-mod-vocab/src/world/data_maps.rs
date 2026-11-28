//! Typed reads of data maps (NeoForge's `DataMapType` and `IWithData.getData`): the value the
//! server's data packs and mods' files gave a registry entry, after every pack was applied.
use crate::{
    Backend, BiomeKey, BiomeView, BlockEntityTypeKey, BlockKey, CustomRegistryEntry, EffectKey,
    EnchantmentKey, EntityTypeKey, EntryKey, FluidKey, Host, Identifier, ItemKey, NoBackend,
    ProfessionKey, RegistryName, Tag,
};
use entry::EntryName;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

/// Whether and how a data map is synchronized to joining clients (NeoForge's
/// `DataMapType.Builder.synced(networkCodec, mandatory)`).
// no wire: startup-only registration policy.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum DataMapSync {
    /// Server-only; never sent over the network (default).
    #[default]
    ServerOnly,
    /// Synced to Sunrise clients that declare this map; vanilla clients and clients
    /// without the map are allowed to join (`mandatory = false`).
    Optional,
    /// Synced to clients; clients (including vanilla clients) that do not declare this
    /// map as required are disconnected during configuration (`mandatory = true`).
    Required,
}

/// The one typed data map trait for both built-in registries ([`ItemKey`], [`BlockKey`], ...)
/// and mod-declared custom static registries ([`EntryKey<T>`] where `T: CustomRegistryEntry`)
/// (NeoForge's `DataMapType<R, T>` / `AdvancedDataMapType<R, T, V>`).
///
/// Builtin maps are already registered. They return canonical codec NBT compounds,
/// including Float distances (with infinity preserved) and Int frequencies.
/// Mod-defined maps retain the JSON-to-NBT mapping below.
///
/// Register a mod-defined map during [`Hooks::register`](crate::Hooks::register) with
/// [`Registrar::register_data_map`](crate::Registrar::register_data_map), then read an
/// entry's value with its key's `data` ([`ItemKey::data`], [`EntityTypeKey::data`],
/// [`EntryKey::data`]...), or by name with [`data_map_value`] and [`data_map_entries`].
/// Typed key reads return owned decoded values; reads by name borrow raw NBT from the host.
///
/// A value arrives as the NBT form of the JSON a file held: a whole number is an `Int`, or
/// a `Long` past `i32`, a fraction a `Double`, a boolean a `Byte`, an object a `Compound`
/// and an array a `List`. Files containing integers outside signed 64-bit range or `null`
/// at any nesting depth cannot be converted to NBT and are skipped atomically at load time
/// with an error log, preserving earlier layers.
/// Fractional/exponent JSON tokens use the parser's
/// `f64` value; empty and heterogeneous arrays retain their elements.
///
/// ```
/// use mod_api::{DataMap, DataMapSync, ItemKey, Tag};
/// /// `projecte:emc`: an item's EMC value.
/// struct Emc;
/// impl DataMap for Emc {
///     type Entry = ItemKey;
///     type Value = i64;
///     fn id(&self) -> &str {
///         "projecte:emc"
///     }
///     fn decode(&self, value: &Tag) -> Option<i64> {
///         match value {
///             Tag::Int(value) => Some(i64::from(*value)),
///             Tag::Long(value) => Some(*value),
///             _ => None,
///         }
///     }
///     fn sync(&self) -> DataMapSync {
///         DataMapSync::Optional
///     }
/// }
/// assert_eq!(Emc.id(), "projecte:emc");
/// assert_eq!(Emc.sync(), DataMapSync::Optional);
/// ```
pub trait DataMap: Send + Sync + 'static {
    /// The registry entries the map attaches values to: one of the [`DataMapEntry`] keys.
    type Entry: DataMapEntry;
    /// The owned result of decoding an entry's borrowed tag.
    type Value;
    /// Canonical namespaced map ID (`"chemlib:fuel"`).
    fn id(&self) -> &str;
    /// Decode an entry's merged NBT value, refusing another shape.
    fn decode(&self, value: &Tag) -> Option<Self::Value>;
    /// Sync policy ([`DataMapSync::ServerOnly`] by default).
    fn sync(&self) -> DataMapSync {
        DataMapSync::ServerOnly
    }
    /// Project a server-merged NBT value before sending it to clients
    /// (`DataMapType.networkCodec`).
    ///
    /// Returning `None` is a containment rule: projection failed, the engine logs once
    /// and leaves the entry out of the client snapshot. Default sends the tag unchanged.
    fn network_value(&self, value: &Tag) -> Option<Tag> {
        Some(value.clone())
    }
    /// Combine an older layer's value with a newer layer's value for the same entry when
    /// the newer entry did not set `"replace": true` (`DataMapValueMerger`).
    /// Default is last-wins (`Ok(incoming.clone())`).
    fn merge(&self, _entry: &Identifier, _existing: &Tag, incoming: &Tag) -> Result<Tag, String> {
        Ok(incoming.clone())
    }
    /// Partially remove data from an entry when a file uses NeoForge's object removal form
    /// `"remove": { "<id|#tag>": <remover> }` or array-of-objects form
    /// `[{"key": "<id|#tag>", "remover": <remover>}]` (`DataMapValueRemover`).
    /// Returning `Ok(None)` removes the entry completely; `Ok(Some(tag))` keeps the reduced
    /// value. Default rejects partial removers with an error (maps that do not support
    /// partial removal only accept full removals).
    fn remove_partial(
        &self,
        _entry: &Identifier,
        _existing: &Tag,
        _remover: &Tag,
    ) -> Result<Option<Tag>, String> {
        Err("data map does not support partial removers".into())
    }
    /// Whether `#tag` keys are allowed in `values` (`AdvancedDataMapType.supportsTags`).
    /// Default `true`.
    fn supports_tags(&self) -> bool {
        true
    }
}

type NetworkValueFn = Box<dyn Fn(&Tag) -> Option<Tag> + Send + Sync>;
type MergeFn = Box<dyn Fn(&Identifier, &Tag, &Tag) -> Result<Tag, String> + Send + Sync>;
type RemovePartialFn =
    Box<dyn Fn(&Identifier, &Tag, &Tag) -> Result<Option<Tag>, String> + Send + Sync>;

/// Descriptor for a by-name data map on a registry that has no typed key
/// (such as a datapack registry), paired with [`data_map_value`] and [`data_map_entries`].
///
/// Registries with a typed key (built-in keys and custom static registries whose payload
/// implements [`CustomRegistryEntry`]) use [`DataMap`] +
/// [`Registrar::register_data_map`](crate::Registrar::register_data_map).
// no wire: startup-only registration descriptor holding mod callbacks.
// exhaustive: builder constructed via `RawDataMap::new`.
pub struct RawDataMap {
    pub(crate) registry: String,
    pub(crate) id: String,
    pub(crate) sync: DataMapSync,
    pub(crate) supports_tags: bool,
    pub(crate) network_value: NetworkValueFn,
    pub(crate) merge: MergeFn,
    pub(crate) remove_partial: RemovePartialFn,
}

impl std::fmt::Debug for RawDataMap {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RawDataMap")
            .field("registry", &self.registry)
            .field("id", &self.id)
            .field("sync", &self.sync)
            .field("supports_tags", &self.supports_tags)
            .finish()
    }
}

impl RawDataMap {
    /// Start a by-name data map descriptor with the same defaults as [`DataMap`]
    /// ([`DataMapSync::ServerOnly`], `supports_tags = true`, identity `network_value`,
    /// last-wins `merge`, error `remove_partial`).
    pub fn new(registry: impl Into<String>, id: impl Into<String>) -> Self {
        Self {
            registry: registry.into(),
            id: id.into(),
            sync: DataMapSync::ServerOnly,
            supports_tags: true,
            network_value: Box::new(|value| Some(value.clone())),
            merge: Box::new(|_, _, incoming| Ok(incoming.clone())),
            remove_partial: Box::new(|_, _, _| {
                Err("data map does not support partial removers".into())
            }),
        }
    }

    /// Set the client synchronization policy ([`DataMap::sync`]).
    pub fn sync(mut self, sync: DataMapSync) -> Self {
        self.sync = sync;
        self
    }

    /// Set whether `#tag` keys are allowed in `values` ([`DataMap::supports_tags`]).
    pub fn supports_tags(mut self, supports_tags: bool) -> Self {
        self.supports_tags = supports_tags;
        self
    }

    /// Set the client projection callback ([`DataMap::network_value`]).
    ///
    /// Returning `None` is a containment rule: projection failed, the engine logs once
    /// and leaves the entry out of the client snapshot.
    pub fn network_value(
        mut self,
        f: impl Fn(&Tag) -> Option<Tag> + Send + Sync + 'static,
    ) -> Self {
        self.network_value = Box::new(f);
        self
    }

    /// Set the multi-layer merge callback ([`DataMap::merge`]).
    pub fn merge(
        mut self,
        f: impl Fn(&Identifier, &Tag, &Tag) -> Result<Tag, String> + Send + Sync + 'static,
    ) -> Self {
        self.merge = Box::new(f);
        self
    }

    /// Set the partial removal callback ([`DataMap::remove_partial`]).
    pub fn remove_partial(
        mut self,
        f: impl Fn(&Identifier, &Tag, &Tag) -> Result<Option<Tag>, String> + Send + Sync + 'static,
    ) -> Self {
        self.remove_partial = Box::new(f);
        self
    }

    /// The target registry ID as supplied by the caller.
    // hidden: the registrar lowers a RawDataMap by reading the registry as written; mods never read it back.
    #[doc(hidden)]
    pub fn raw_registry(&self) -> &str {
        &self.registry
    }

    /// The map ID as supplied by the caller.
    // hidden: the registrar lowers a RawDataMap by reading the id as written; mods never read it back.
    #[doc(hidden)]
    pub fn raw_id(&self) -> &str {
        &self.id
    }

    /// The configured sync policy.
    // hidden: the registrar lowers a RawDataMap by reading the sync policy as written; mods never read it back.
    #[doc(hidden)]
    pub fn raw_sync(&self) -> DataMapSync {
        self.sync
    }

    /// Lower a typed [`DataMap`] into a [`RawDataMap`] so both registration paths share
    /// one internal registration and panic-containment implementation.
    pub(crate) fn from_typed<M: DataMap>(map: M) -> Self {
        let registry = <M::Entry as entry::Entry>::REGISTRY.to_owned();
        let id = map.id().to_owned();
        let sync = map.sync();
        let supports_tags = map.supports_tags();
        let map = Arc::new(map);
        let net_map = Arc::clone(&map);
        let merge_map = Arc::clone(&map);
        let remove_map = map;
        Self {
            registry,
            id,
            sync,
            supports_tags,
            network_value: Box::new(move |value| net_map.network_value(value)),
            merge: Box::new(move |entry, existing, incoming| {
                merge_map.merge(entry, existing, incoming)
            }),
            remove_partial: Box::new(move |entry, existing, remover| {
                remove_map.remove_partial(entry, existing, remover)
            }),
        }
    }
}

/// Startup-validated, frozen data map registration held by an [`Installed`](crate::parts::Installed) epoch.
///
/// Engine plumbing only: exposed via `__private`, never re-exported from `mod_api`.
// no wire: engine-internal frozen registration holding mod callbacks.
// exhaustive: constructed only after startup validation.
pub struct RegisteredDataMap {
    owner_mod: &'static str,
    registry: Identifier,
    registry_str: String,
    id: Identifier,
    id_str: String,
    sync: DataMapSync,
    supports_tags: bool,
    network_value: NetworkValueFn,
    network_value_warned: AtomicBool,
    merge: MergeFn,
    remove_partial: RemovePartialFn,
    merge_poisoned: AtomicBool,
    remove_poisoned: AtomicBool,
}

impl std::fmt::Debug for RegisteredDataMap {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RegisteredDataMap")
            .field("owner_mod", &self.owner_mod)
            .field("registry", &self.registry_str)
            .field("id", &self.id_str)
            .field("sync", &self.sync)
            .field("supports_tags", &self.supports_tags)
            .finish()
    }
}

impl RegisteredDataMap {
    /// Construct a frozen data map registration after startup validation.
    pub fn from_raw(
        owner_mod: &'static str,
        registry: Identifier,
        id: Identifier,
        raw: RawDataMap,
    ) -> Self {
        let registry_str = registry.to_string();
        let id_str = id.to_string();
        Self {
            owner_mod,
            registry,
            registry_str,
            id,
            id_str,
            sync: raw.sync,
            supports_tags: raw.supports_tags,
            network_value: raw.network_value,
            network_value_warned: AtomicBool::new(false),
            merge: raw.merge,
            remove_partial: raw.remove_partial,
            merge_poisoned: AtomicBool::new(false),
            remove_poisoned: AtomicBool::new(false),
        }
    }

    /// Owning mod ID.
    pub fn owner_mod(&self) -> &'static str {
        self.owner_mod
    }

    /// Canonical namespaced target registry ID.
    pub fn registry(&self) -> &str {
        &self.registry_str
    }

    /// Parsed target registry identifier.
    pub fn registry_id(&self) -> &Identifier {
        &self.registry
    }

    /// Parsed canonical map identifier.
    pub fn id(&self) -> &Identifier {
        &self.id
    }

    /// Canonical namespaced map ID.
    pub fn id_str(&self) -> &str {
        &self.id_str
    }

    /// Synchronization policy.
    pub fn sync(&self) -> DataMapSync {
        self.sync
    }

    /// Whether `#tag` keys are allowed in `values`.
    pub fn supports_tags(&self) -> bool {
        self.supports_tags
    }

    /// Project a merged NBT value for client sync. If projection returns `None` or panics,
    /// logs once and omits the entry from the client snapshot (containment rule).
    pub fn project_network_value(&self, entry: &str, value: &Tag) -> Option<Tag> {
        let projected = crate::runtime::contain_callback(
            || (self.network_value)(value),
            |payload| {
                if !self.network_value_warned.swap(true, Ordering::AcqRel) {
                    crate::runtime::log_and_queue_named_panic(
                        crate::runtime::installed(),
                        "data_map_network_value",
                        self.owner_mod,
                        Some(&self.id),
                        payload,
                    );
                }
            },
        );
        match projected {
            Some(Some(tag)) => Some(tag),
            Some(None) => {
                if !self.network_value_warned.swap(true, Ordering::AcqRel) {
                    log::warn!(
                        "Sunrise: data map {} on {} (mod {}): network_value projection failed for entry {entry}; omitting entry from client sync",
                        self.id_str,
                        self.registry_str,
                        self.owner_mod
                    );
                }
                None
            }
            None => None,
        }
    }

    /// Run this map's merger inside `runtime.rs`'s `Callback::DataMapMerge` panic boundary.
    pub fn merge(&self, entry: &Identifier, existing: &Tag, incoming: &Tag) -> Result<Tag, String> {
        if self.merge_poisoned.load(Ordering::Acquire) {
            return Err(format!(
                "mod {}: data_map_merge ({}) is disabled after an earlier panic",
                self.owner_mod, self.id_str
            ));
        }
        if crate::runtime::installed().is_some()
            && !crate::runtime::session_named_callback_enabled(
                crate::runtime::Callback::DataMapMerge,
                self.owner_mod,
                &self.id,
            )
        {
            self.merge_poisoned.store(true, Ordering::Release);
            return Err(format!(
                "mod {}: data_map_merge ({}) is disabled after an earlier panic",
                self.owner_mod, self.id_str
            ));
        }
        match crate::runtime::call_session_named_callback(
            crate::runtime::Callback::DataMapMerge,
            self.owner_mod,
            &self.id,
            "data_map_merge",
            || (self.merge)(entry, existing, incoming),
        ) {
            Ok(result) => result,
            Err(panic_msg) => {
                self.merge_poisoned.store(true, Ordering::Release);
                Err(panic_msg)
            }
        }
    }

    /// Run this map's partial remover inside `runtime.rs`'s `Callback::DataMapRemove` panic boundary.
    pub fn remove_partial(
        &self,
        entry: &Identifier,
        existing: &Tag,
        remover: &Tag,
    ) -> Result<Option<Tag>, String> {
        if self.remove_poisoned.load(Ordering::Acquire) {
            return Err(format!(
                "mod {}: data_map_remove ({}) is disabled after an earlier panic",
                self.owner_mod, self.id_str
            ));
        }
        if crate::runtime::installed().is_some()
            && !crate::runtime::session_named_callback_enabled(
                crate::runtime::Callback::DataMapRemove,
                self.owner_mod,
                &self.id,
            )
        {
            self.remove_poisoned.store(true, Ordering::Release);
            return Err(format!(
                "mod {}: data_map_remove ({}) is disabled after an earlier panic",
                self.owner_mod, self.id_str
            ));
        }
        match crate::runtime::call_session_named_callback(
            crate::runtime::Callback::DataMapRemove,
            self.owner_mod,
            &self.id,
            "data_map_remove",
            || (self.remove_partial)(entry, existing, remover),
        ) {
            Ok(result) => result,
            Err(panic_msg) => {
                self.remove_poisoned.store(true, Ordering::Release);
                Err(panic_msg)
            }
        }
    }
}

/// A registry entry a data map can attach a value to through a typed key. Sealed: a registry
/// without one is read by name with [`data_map_value`].
pub trait DataMapEntry: entry::Entry {}

mod entry {
    use super::*;

    // exhaustive: the sealed lookup represents only resident, pinned or owned registry names.
    pub enum EntryName<'a> {
        Borrowed(&'a str),
        Pinned(RegistryName),
        Owned(String),
    }

    impl EntryName<'_> {
        pub(super) fn as_str(&self) -> &str {
            match self {
                Self::Borrowed(id) => id,
                Self::Pinned(id) => id.as_str(),
                Self::Owned(id) => id,
            }
        }
    }

    /// Common read-only lookup surface shared by [`Backend`] and [`crate::ReloadContext`].
    pub trait DataMapReader {
        fn data_map_value(
            &self,
            registry: &str,
            entry: &str,
            map: &str,
        ) -> Result<Option<&crate::Tag>, DataMapError>;
        fn game_event_key_id(
            &self,
            key: crate::GameEventKey,
        ) -> Result<RegistryName, crate::GameEventKeyError>;
        fn item_key_id(&self, key: ItemKey) -> Option<&str>;
        fn block_key_id(&self, key: BlockKey) -> Option<&str>;
        fn fluid_key_id(&self, key: FluidKey) -> Option<&str>;
        fn effect_key_id(&self, key: EffectKey) -> Option<&str>;
        fn entity_type_key_id(&self, key: EntityTypeKey) -> Option<RegistryName>;
        fn enchantment_key_id(&self, key: EnchantmentKey) -> Option<RegistryName>;
        fn block_entity_type_key_id(&self, key: BlockEntityTypeKey) -> Option<&str>;
        fn biome_key_id(&self, key: &BiomeKey) -> Result<&str, crate::BiomeKeyError>;
    }

    impl DataMapReader for &(dyn Backend + '_) {
        fn data_map_value(
            &self,
            registry: &str,
            entry: &str,
            map: &str,
        ) -> Result<Option<&crate::Tag>, DataMapError> {
            crate::prelude::DataMapsBackend::data_map_value(*self, registry, entry, map)
        }
        fn game_event_key_id(
            &self,
            key: crate::GameEventKey,
        ) -> Result<RegistryName, crate::GameEventKeyError> {
            crate::prelude::KeysBackend::game_event_key_id(*self, key)
        }
        fn item_key_id(&self, key: ItemKey) -> Option<&str> {
            crate::prelude::KeysBackend::item_key_id(*self, key)
        }
        fn block_key_id(&self, key: BlockKey) -> Option<&str> {
            crate::prelude::KeysBackend::block_key_id(*self, key)
        }
        fn fluid_key_id(&self, key: FluidKey) -> Option<&str> {
            crate::prelude::KeysBackend::fluid_key_id(*self, key)
        }
        fn effect_key_id(&self, key: EffectKey) -> Option<&str> {
            crate::prelude::KeysBackend::effect_key_id(*self, key)
        }
        fn entity_type_key_id(&self, key: EntityTypeKey) -> Option<RegistryName> {
            crate::prelude::KeysBackend::entity_type_key_id(*self, key)
        }
        fn enchantment_key_id(&self, key: EnchantmentKey) -> Option<RegistryName> {
            crate::prelude::KeysBackend::enchantment_key_id(*self, key)
        }
        fn block_entity_type_key_id(&self, key: BlockEntityTypeKey) -> Option<&str> {
            crate::prelude::KeysBackend::block_entity_type_key_id(*self, key)
        }
        fn biome_key_id(&self, key: &BiomeKey) -> Result<&str, crate::BiomeKeyError> {
            crate::prelude::KeysBackend::biome_view(*self, key).map(|view| view.id())
        }
    }

    impl DataMapReader for &(dyn crate::ReloadContext + '_) {
        fn data_map_value(
            &self,
            registry: &str,
            entry: &str,
            map: &str,
        ) -> Result<Option<&crate::Tag>, DataMapError> {
            crate::ReloadContext::data_map_value(*self, registry, entry, map)
        }
        fn game_event_key_id(
            &self,
            key: crate::GameEventKey,
        ) -> Result<RegistryName, crate::GameEventKeyError> {
            crate::ReloadContext::game_event_key_id(*self, key)
        }
        fn item_key_id(&self, key: ItemKey) -> Option<&str> {
            crate::ReloadContext::item_key_id(*self, key)
        }
        fn block_key_id(&self, key: BlockKey) -> Option<&str> {
            crate::ReloadContext::block_key_id(*self, key)
        }
        fn fluid_key_id(&self, key: FluidKey) -> Option<&str> {
            crate::ReloadContext::fluid_key_id(*self, key)
        }
        fn effect_key_id(&self, key: EffectKey) -> Option<&str> {
            crate::ReloadContext::effect_key_id(*self, key)
        }
        fn entity_type_key_id(&self, key: EntityTypeKey) -> Option<RegistryName> {
            crate::ReloadContext::entity_type_key_id(*self, key)
        }
        fn enchantment_key_id(&self, key: EnchantmentKey) -> Option<RegistryName> {
            crate::ReloadContext::enchantment_key_id(*self, key)
        }
        fn block_entity_type_key_id(&self, key: BlockEntityTypeKey) -> Option<&str> {
            crate::ReloadContext::block_entity_type_key_id(*self, key)
        }
        fn biome_key_id(&self, key: &BiomeKey) -> Result<&str, crate::BiomeKeyError> {
            crate::ReloadContext::biome_key_id(*self, key)
        }
    }

    /// The registry an entry is in, and its id there through a read-only data-map source.
    pub trait Entry {
        /// The registry's id.
        const REGISTRY: &'static str;
        /// The entry's full registry id.
        fn id<'a>(&self, reader: &'a dyn DataMapReader) -> Result<EntryName<'a>, DataMapError>;
    }
}

/// `DataMapEntry` and `data` for a key whose id the keys backend answers.
macro_rules! typed_entries {
    ($($key:ty => $registry:literal, $id:ident;)+) => {$(
        impl DataMapEntry for $key {}
        impl entry::Entry for $key {
            const REGISTRY: &'static str = $registry;
            fn id<'a>(
                &self,
                reader: &'a dyn entry::DataMapReader,
            ) -> Result<EntryName<'a>, DataMapError> {
                reader.$id(*self).into_result()
            }
        }
        impl $key {
            /// This entry's value in data map `map` (`Holder.getData`), as the server's data
            /// packs and mods' files left it: the newest file wins, a `#tag` entry gives every
            /// entry in the tag its value, `replace` and `remove` apply, and conditional files
            /// and values count only when their conditions hold. `Ok(None)` where the map
            /// gives this entry no value. A present value is decoded into an owned `M::Value`
            /// that can outlive the host borrow. A refused decode is [`DataMapError::Undecodable`],
            /// not an absent value.
            /// A key that this host cannot name returns `DataMapError::InvalidKey`
            /// when its registry is available to the host.
            ///
            /// Server side: the server merges the maps when its data loads, on a world's first
            /// open and after every `/reload`, and a read sees the last load whole, never a mix
            /// of two. Readable from `data_maps_updated` on, in the world and while data
            /// reloads. A reload that drops a value drops it here too.
            pub fn data<M: DataMap<Entry = $key>>(
                self,
                host: &dyn Host,
                map: &M,
            ) -> Result<Option<M::Value>, DataMapError> {
                data(&self, host, map)
            }
        }
    )+};
}
trait KeyIdResult<'a> {
    fn into_result(self) -> Result<EntryName<'a>, DataMapError>;
}
impl<'a> KeyIdResult<'a> for Option<&'a str> {
    fn into_result(self) -> Result<EntryName<'a>, DataMapError> {
        self.map(EntryName::Borrowed)
            .ok_or(DataMapError::InvalidKey)
    }
}
impl<'a> KeyIdResult<'a> for Option<RegistryName> {
    fn into_result(self) -> Result<EntryName<'a>, DataMapError> {
        self.map(EntryName::Pinned).ok_or(DataMapError::InvalidKey)
    }
}
impl<'a> KeyIdResult<'a> for Result<RegistryName, crate::GameEventKeyError> {
    fn into_result(self) -> Result<EntryName<'a>, DataMapError> {
        self.map(EntryName::Pinned).map_err(|error| match error {
            crate::GameEventKeyError::Unavailable => DataMapError::Unavailable,
            crate::GameEventKeyError::InvalidKey => DataMapError::InvalidKey,
        })
    }
}
typed_entries! {
    crate::GameEventKey => "minecraft:game_event", game_event_key_id;
    ItemKey => "minecraft:item", item_key_id;
    BlockKey => "minecraft:block", block_key_id;
    FluidKey => "minecraft:fluid", fluid_key_id;
    EffectKey => "minecraft:mob_effect", effect_key_id;
    EntityTypeKey => "minecraft:entity_type", entity_type_key_id;
    EnchantmentKey => "minecraft:enchantment", enchantment_key_id;
    BlockEntityTypeKey => "minecraft:block_entity_type", block_entity_type_key_id;
}
impl DataMapEntry for BiomeKey {}
impl entry::Entry for BiomeKey {
    const REGISTRY: &'static str = "minecraft:worldgen/biome";
    fn id<'a>(&self, reader: &'a dyn entry::DataMapReader) -> Result<EntryName<'a>, DataMapError> {
        reader
            .biome_key_id(self)
            .map(EntryName::Borrowed)
            .map_err(|error| match error {
                crate::BiomeKeyError::Unavailable => DataMapError::Unavailable,
                crate::BiomeKeyError::InvalidIdentifier
                | crate::BiomeKeyError::UnknownBiome
                | crate::BiomeKeyError::WrongGeneration => DataMapError::InvalidKey,
            })
    }
}
impl BiomeKey {
    /// Read this biome's data as for [`ItemKey::data`], validating the key against the
    /// currently installed server world biome registry. Unknown biomes return
    /// [`DataMapError::InvalidKey`]; no installed registry returns `Unavailable`.
    pub fn data<M: DataMap<Entry = BiomeKey>>(
        &self,
        host: &dyn Host,
        map: &M,
    ) -> Result<Option<M::Value>, DataMapError> {
        data(self, host, map)
    }
}
impl BiomeView<'_> {
    /// Read this scoped biome view's data as for [`ItemKey::data`]. A view from another world
    /// or world-data generation returns [`DataMapError::InvalidKey`]; no installed registry
    /// returns `Unavailable`.
    pub fn data<M: DataMap<Entry = BiomeKey>>(
        self,
        host: &dyn Host,
        map: &M,
    ) -> Result<Option<M::Value>, DataMapError> {
        let backend = crate::backend_of(host).ok_or_else(DataMapError::no_backend)?;
        let id = backend.biome_view_id(self).map_err(|error| match error {
            crate::BiomeKeyError::Unavailable => DataMapError::Unavailable,
            crate::BiomeKeyError::InvalidIdentifier
            | crate::BiomeKeyError::UnknownBiome
            | crate::BiomeKeyError::WrongGeneration => DataMapError::InvalidKey,
        })?;
        match backend.data_map_value(<BiomeKey as entry::Entry>::REGISTRY, id, map.id())? {
            Some(value) => map.decode(value).map(Some).ok_or(DataMapError::Undecodable),
            None => Ok(None),
        }
    }
}
impl DataMapEntry for ProfessionKey {}
impl entry::Entry for ProfessionKey {
    const REGISTRY: &'static str = "minecraft:villager_profession";
    fn id<'a>(&self, _: &'a dyn entry::DataMapReader) -> Result<EntryName<'a>, DataMapError> {
        Ok(EntryName::Owned(self.0.to_string()))
    }
}
impl ProfessionKey {
    /// This profession's value in data map `map`, as for [`ItemKey::data`].
    pub fn data<M: DataMap<Entry = ProfessionKey>>(
        &self,
        host: &dyn Host,
        map: &M,
    ) -> Result<Option<M::Value>, DataMapError> {
        data(self, host, map)
    }
}

impl<T: CustomRegistryEntry> DataMapEntry for EntryKey<T> {}
impl<T: CustomRegistryEntry> entry::Entry for EntryKey<T> {
    const REGISTRY: &'static str = T::REGISTRY.id();
    fn id<'a>(&self, _reader: &'a dyn entry::DataMapReader) -> Result<EntryName<'a>, DataMapError> {
        crate::parts::custom_registry::<T>(T::REGISTRY.id())
            .and_then(|registry| registry.id_str_of(*self))
            .map(EntryName::Borrowed)
            .ok_or(DataMapError::InvalidKey)
    }
}
impl<T: CustomRegistryEntry> EntryKey<T> {
    /// Read this custom-registry entry's decoded value in data map `map`,
    /// identically to [`ItemKey::data`].
    pub fn data<M: DataMap<Entry = EntryKey<T>>>(
        self,
        host: &dyn Host,
        map: &M,
    ) -> Result<Option<M::Value>, DataMapError> {
        data(&self, host, map)
    }
}

/// Why a data map read has no answer. An entry the map gives no value is `Ok(None)`, not one of
/// these. An empty map yields an empty slice from [`data_map_entries`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub enum DataMapError {
    /// No data maps are loaded here: a client that has not received synced data maps, a server
    /// before its world's data first loads, or a host with no game behind it.
    Unavailable,
    /// Neither the engine nor a mod registered the map for this registry during startup
    /// (`Registrar::register_data_map` / `Registrar::register_data_map_by_name`).
    UnknownType,
    /// A typed read's [`DataMap::decode`] refused the entry's NBT value.
    Undecodable,
    /// The key has no registry name in this host, or belongs to another world or generation.
    InvalidKey,
}
impl NoBackend for DataMapError {
    fn no_backend() -> Self {
        Self::Unavailable
    }
}

/// `entry`'s value in `map`, decoded through any read-only data-map source.
fn read_data<M: DataMap>(
    entry: &M::Entry,
    reader: &dyn entry::DataMapReader,
    map: &M,
) -> Result<Option<M::Value>, DataMapError> {
    let id = entry::Entry::id(entry, reader)?;
    let registry = <M::Entry as entry::Entry>::REGISTRY;
    match reader.data_map_value(registry, id.as_str(), map.id())? {
        Some(value) => map.decode(value).map(Some).ok_or(DataMapError::Undecodable),
        None => Ok(None),
    }
}

/// `entry`'s value in `map`, decoded.
fn data<M: DataMap>(
    entry: &M::Entry,
    host: &dyn Host,
    map: &M,
) -> Result<Option<M::Value>, DataMapError> {
    let backend = crate::backend_of(host).ok_or_else(DataMapError::no_backend)?;
    read_data(entry, &backend, map)
}

/// Entry `entry` of registry `registry`: its raw NBT value in data map `map`
/// (`IRegistryExtension.getData`). A bare entry id uses the `minecraft` namespace.
/// `Ok(None)` means the map has no value for this entry. A host without loaded maps or an
/// unknown map type returns a [`DataMapError`].
///
/// The tag borrows from the host's published data, not the lookup IDs. Use `Option::cloned`
/// only if you need the raw value after the host borrow ends.
pub fn data_map_value<'host>(
    host: &'host dyn Host,
    registry: &str,
    entry: &str,
    map: &str,
) -> Result<Option<&'host Tag>, DataMapError> {
    let backend = crate::backend_of(host).ok_or_else(DataMapError::no_backend)?;
    backend.data_map_value(registry, entry, map)
}

/// The whole published data map `map` of `registry`, sorted by entry ID
/// (`IRegistryExtension.getDataMap`). A known empty map yields an empty slice; unavailable
/// maps or an unknown type return a [`DataMapError`], not an empty slice.
///
/// The slice borrows from the host's published data, not the lookup IDs. Iterate by
/// reference, or call `to_vec` only if you need all the rows after that borrow ends.
pub fn data_map_entries<'host>(
    host: &'host dyn Host,
    registry: &str,
    map: &str,
) -> Result<&'host [(String, Tag)], DataMapError> {
    let backend = crate::backend_of(host).ok_or_else(DataMapError::no_backend)?;
    backend.data_map_entries(registry, map)
}

impl dyn crate::ReloadContext + '_ {
    /// Entry `entry`'s decoded value in data map `map`, for reload hooks such as
    /// `data_maps_updated`.
    pub fn data_map<M: DataMap>(
        &self,
        entry: &M::Entry,
        map: &M,
    ) -> Result<Option<M::Value>, DataMapError> {
        read_data(entry, &self, map)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A host with two registered maps. The stick's EMC has the wrong NBT type.
    struct Maps {
        emc: [(String, Tag); 2],
        hardness: [(String, Tag); 1],
    }
    impl Maps {
        fn new() -> Self {
            Self {
                emc: [
                    ("test:coal".into(), Tag::Int(10)),
                    ("test:stick".into(), Tag::String("ten".into())),
                ],
                hardness: [("test:stone".into(), Tag::Long(3))],
            }
        }
    }
    impl crate::host::data_maps::DataMapsBackend for Maps {
        fn data_map_value(
            &self,
            registry: &str,
            entry: &str,
            map: &str,
        ) -> Result<Option<&Tag>, DataMapError> {
            Ok(self
                .data_map_entries(registry, map)?
                .iter()
                .find_map(|(id, value)| (id.as_str() == entry).then_some(value)))
        }
        fn data_map_entries(
            &self,
            registry: &str,
            map: &str,
        ) -> Result<&[(String, Tag)], DataMapError> {
            match (registry, map) {
                ("minecraft:item", "test:emc") => Ok(&self.emc),
                ("minecraft:block", "test:hardness") => Ok(&self.hardness),
                _ => Err(DataMapError::UnknownType),
            }
        }
    }
    impl crate::host::keys::KeysBackend for Maps {
        fn item_key_id(&self, key: ItemKey) -> Option<&str> {
            ["test:coal", "test:stick", "test:air"]
                .get(key.0 as usize)
                .copied()
        }
        fn block_key_id(&self, key: BlockKey) -> Option<&str> {
            ["test:stone", "test:dirt"].get(key.0 as usize).copied()
        }
    }
    impl Host for Maps {
        fn backend(&self) -> Option<crate::BackendRef<'_>> {
            Some(crate::BackendRef::of(self))
        }
    }
    crate::impl_host_areas!(Maps);
    crate::impl_backend_areas!(Maps, except(data_maps, keys));
    impl Backend for Maps {}

    /// No game behind it.
    struct Bare;
    impl Host for Bare {}
    crate::impl_host_areas!(Bare);

    fn number(value: &Tag) -> Option<i64> {
        match value {
            Tag::Int(value) => Some(i64::from(*value)),
            Tag::Long(value) => Some(*value),
            _ => None,
        }
    }
    struct Emc;
    impl DataMap for Emc {
        type Entry = ItemKey;
        type Value = i64;
        fn id(&self) -> &str {
            "test:emc"
        }
        fn decode(&self, value: &Tag) -> Option<i64> {
            number(value)
        }
    }
    struct Hardness;
    impl DataMap for Hardness {
        type Entry = BlockKey;
        type Value = i64;
        fn id(&self) -> &str {
            "test:hardness"
        }
        fn decode(&self, value: &Tag) -> Option<i64> {
            number(value)
        }
    }
    /// A map no mod registered.
    struct Unregistered;
    impl DataMap for Unregistered {
        type Entry = ItemKey;
        type Value = i64;
        fn id(&self) -> &str {
            "test:unregistered"
        }
        fn decode(&self, value: &Tag) -> Option<i64> {
            number(value)
        }
    }

    #[test]
    fn a_read_decodes_the_value_and_tells_no_value_from_a_refusal() {
        let maps = Maps::new();
        assert_eq!(ItemKey(0).data(&maps, &Emc), Ok(Some(10)));
        assert_eq!(BlockKey(0).data(&maps, &Hardness), Ok(Some(3)));
        assert_eq!(ItemKey(2).data(&maps, &Emc), Ok(None));
        assert_eq!(BlockKey(1).data(&maps, &Hardness), Ok(None));
        assert_eq!(ItemKey(1).data(&maps, &Emc), Err(DataMapError::Undecodable));
        assert_eq!(ItemKey(3).data(&maps, &Emc), Err(DataMapError::InvalidKey));
        assert_eq!(
            ItemKey(0).data(&maps, &Unregistered),
            Err(DataMapError::UnknownType)
        );
        assert_eq!(ItemKey(0).data(&Bare, &Emc), Err(DataMapError::Unavailable));
        assert_eq!(
            BlockKey(0).data(&Bare, &Hardness),
            Err(DataMapError::Unavailable)
        );
    }

    #[test]
    fn a_read_by_name_reaches_the_whole_map() {
        let maps = Maps::new();
        let (value, entries) = {
            let registry = String::from("minecraft:block");
            let entry = String::from("test:stone");
            let map = String::from("test:hardness");
            (
                data_map_value(&maps, &registry, &entry, &map),
                data_map_entries(&maps, &registry, &map).unwrap(),
            )
        };
        assert_eq!(value, Ok(Some(&Tag::Long(3))));
        assert_eq!(entries, &[("test:stone".to_owned(), Tag::Long(3))]);
        assert_eq!(
            data_map_entries(&Bare, "minecraft:block", "test:hardness"),
            Err(DataMapError::Unavailable)
        );
    }
}
