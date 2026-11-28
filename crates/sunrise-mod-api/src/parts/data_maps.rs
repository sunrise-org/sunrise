//! Startup authoring, validation, and freeze for mod-declared data maps (NeoForge's
//! `RegisterDataMapTypesEvent`).
use super::*;
use crate::{DataMapSync, DatapackRegistrySync, Identifier, RawDataMap};

/// Engine built-in NeoForge data maps (`NeoForgeDataMaps.java`): mods may read them,
/// never re-register them.
const RESERVED_BUILTIN_DATA_MAPS: &[(&str, &str)] = &[
    ("minecraft:block", "neoforge:flammable"),
    ("minecraft:block", "neoforge:oxidizables"),
    ("minecraft:block", "neoforge:waxables"),
    ("minecraft:block", "neoforge:transformables"),
    (
        "minecraft:block_transformer",
        "neoforge:block_transform_appenders",
    ),
    (
        "minecraft:entity_type",
        "neoforge:acceptable_villager_distances",
    ),
    ("minecraft:entity_type", "neoforge:monster_room_mobs"),
    ("minecraft:entity_type", "neoforge:parrot_imitations"),
    ("minecraft:worldgen/biome", "neoforge:villager_types"),
    ("minecraft:villager_profession", "neoforge:raid_hero_gifts"),
    ("minecraft:game_event", "neoforge:vibration_frequencies"),
    ("minecraft:item", "neoforge:villager_compostables"),
];

/// Vanilla 26.3 `RegistryDataLoader.RELOADABLE_REGISTRIES`: reloadable datapack registries
/// do not support synced data maps (`RegisterDataMapTypesEvent.java:40-41`).
const RELOADABLE_DATAPACK_REGISTRIES: &[&str] = &[
    "minecraft:loot_table",
    "minecraft:predicate",
    "minecraft:context_float_provider",
    "minecraft:context_int_provider",
    "minecraft:item_modifier",
    "minecraft:slot_source",
    "minecraft:advancement",
    "minecraft:recipe",
];

/// Vanilla 26.3 `RegistryDataLoader` world datapack registries not in
/// `SYNCHRONIZED_REGISTRIES`: non-synced world datapack registries do not support synced
/// data maps (`RegisterDataMapTypesEvent.java:37-39`).
const NON_SYNCED_WORLD_DATAPACK_REGISTRIES: &[&str] = &[
    "minecraft:worldgen/carver",
    "minecraft:worldgen/feature",
    "minecraft:worldgen/placed_feature",
    "minecraft:worldgen/structure",
    "minecraft:worldgen/structure_set",
    "minecraft:worldgen/processor_list",
    "minecraft:worldgen/template_pool",
    "minecraft:worldgen/noise_settings",
    "minecraft:worldgen/noise",
    "minecraft:worldgen/density_function",
    "minecraft:worldgen/material_rule",
    "minecraft:worldgen/material_condition",
    "minecraft:worldgen/world_preset",
    "minecraft:worldgen/flat_level_generator_preset",
    "minecraft:trial_spawner",
    "minecraft:worldgen/multi_noise_biome_source_parameter_list",
    "minecraft:enchantment_provider",
    "minecraft:villager_trade",
    "minecraft:trade_set",
    "minecraft:dimension",
];

impl DataMapsRegistrar for PartRegistrar {
    fn register_data_map_erased(&mut self, map: RawDataMap) {
        let raw_registry = map.raw_registry().to_owned();
        let raw_id = map.raw_id().to_owned();
        let sync = map.raw_sync();

        let Some(registry) = Identifier::parse_namespaced(&raw_registry) else {
            self.refused.push(format!(
                "mod {} registered data map {raw_id:?} on invalid registry {raw_registry:?}",
                self.mod_id
            ));
            return;
        };
        let Some(id) = Identifier::parse_namespaced(&raw_id) else {
            self.refused.push(format!(
                "mod {} registered data map {raw_id:?} on {registry}, which is not a valid namespaced identifier",
                self.mod_id
            ));
            return;
        };
        let registry_str = registry.to_string();
        let id_str = id.to_string();

        if RESERVED_BUILTIN_DATA_MAPS.contains(&(registry_str.as_str(), id_str.as_str())) {
            self.refused.push(format!(
                "mod {} cannot register reserved built-in data map {id_str} on {registry_str}",
                self.mod_id
            ));
            return;
        }

        // `side = "server"` refuses only `DataMapSync::Required`
        // (`RegistryDataMapNegotiation.run:37-50` only refuses clients lacking mandatory maps).
        if sync == DataMapSync::Required
            && mod_vocab::__private::parts_installation::loaded_mods()
                .get(self.mod_id)
                .is_some_and(|package| package.side() == crate::Side::Server)
        {
            self.refused.push(format!(
                "mod {} cannot register required synced data map {id_str} on {registry_str} from a server-only package",
                self.mod_id
            ));
            return;
        }

        // `RegisterDataMapTypesEvent.java:36-43`: synced maps are refused on reloadable
        // datapack registries and on non-synced world datapack registries.
        if sync != DataMapSync::ServerOnly {
            if RELOADABLE_DATAPACK_REGISTRIES.contains(&registry_str.as_str()) {
                self.refused.push(format!(
                    "mod {} cannot register synced data map {id_str} for reloadable datapack registry {registry_str}",
                    self.mod_id
                ));
                return;
            }
            if NON_SYNCED_WORLD_DATAPACK_REGISTRIES.contains(&registry_str.as_str()) {
                self.refused.push(format!(
                    "mod {} cannot register synced data map {id_str} for world datapack registry {registry_str} that is not synced",
                    self.mod_id
                ));
                return;
            }
        }

        if let Some(existing) = self
            .data_maps
            .iter()
            .find(|existing| existing.registry() == registry_str && existing.id_str() == id_str)
        {
            self.refused.push(format!(
                "data map {id_str} on {registry_str} is registered twice, by {} and {}",
                existing.owner_mod(),
                self.mod_id
            ));
            return;
        }

        self.data_maps
            .push(Arc::new(mod_vocab::__private::RegisteredDataMap::from_raw(
                self.mod_id,
                registry,
                id,
                map,
            )));
    }
}

impl PartRegistrar {
    /// Validate cross-registration constraints that depend on all mods having completed
    /// `Hooks::register` (such as synced data maps on mod-declared non-synced datapack
    /// registries).
    pub(crate) fn validate_data_maps(&mut self) {
        for map in &self.data_maps {
            if map.sync() == DataMapSync::ServerOnly {
                continue;
            }
            if let Some(datapack) = self
                .datapack_registries
                .iter()
                .find(|declared| declared.key == map.registry())
                && datapack.sync == DatapackRegistrySync::ServerOnly
            {
                self.refused.push(format!(
                    "mod {} cannot register synced data map {} for world datapack registry {} that is not synced",
                    map.owner_mod(),
                    map.id_str(),
                    map.registry()
                ));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        DataMap, DataMapSync, DatapackField, DatapackFieldKind, DatapackRegistrySync, HookId,
        Hooks, ItemKey, ModEntry, PackageInfo, PackageKind, RawDataMap, Registrar,
        RegistrationContext, Side, Tag,
    };
    use mod_vocab::__private::game::BackendRefOf;

    struct FuelMap {
        id: &'static str,
        sync: DataMapSync,
    }

    impl DataMap for FuelMap {
        type Entry = ItemKey;
        type Value = i32;
        fn id(&self) -> &str {
            self.id
        }
        fn decode(&self, value: &Tag) -> Option<i32> {
            match value {
                Tag::Int(n) => Some(*n),
                _ => None,
            }
        }
        fn sync(&self) -> DataMapSync {
            self.sync
        }
    }

    fn install_with(mods: Vec<ModEntry>) -> Result<(), Vec<String>> {
        crate::parts::install_parts(&mods, &crate::parts::tests::rule_schema(), |_| false)
    }

    #[test]
    fn duplicate_data_map_refused_naming_both_mods() {
        struct ModA;
        struct ModB;
        impl Hooks for ModA {
            fn subscribed(&self) -> &'static [Option<HookId>] {
                &[]
            }
            fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
                r.register_data_map(FuelMap {
                    id: "shared:fuel",
                    sync: DataMapSync::ServerOnly,
                });
            }
        }
        impl Hooks for ModB {
            fn subscribed(&self) -> &'static [Option<HookId>] {
                &[]
            }
            fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
                r.register_data_map(FuelMap {
                    id: "shared:fuel",
                    sync: DataMapSync::Optional,
                });
            }
        }
        let err = install_with(vec![
            ModEntry::new("mod_a", Box::new(ModA)),
            ModEntry::new("mod_b", Box::new(ModB)),
        ])
        .unwrap_err();
        assert!(
            err.iter().any(|msg| msg.contains("shared:fuel")
                && msg.contains("mod_a")
                && msg.contains("mod_b")),
            "expected duplicate refusal naming both mods, got: {err:?}"
        );
    }

    #[test]
    fn reserved_builtin_data_map_refused_naming_mod() {
        struct Intruder;
        impl Hooks for Intruder {
            fn subscribed(&self) -> &'static [Option<HookId>] {
                &[]
            }
            fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
                r.register_data_map(FuelMap {
                    id: "neoforge:villager_compostables",
                    sync: DataMapSync::ServerOnly,
                });
            }
        }
        let err = install_with(vec![ModEntry::new("intruder", Box::new(Intruder))]).unwrap_err();
        assert!(
            err.iter()
                .any(|msg| msg.contains("intruder")
                    && msg.contains("neoforge:villager_compostables")),
            "expected reserved builtin refusal, got: {err:?}"
        );
    }

    #[test]
    fn minecraft_namespace_data_map_is_accepted() {
        struct VanillaNs;
        impl Hooks for VanillaNs {
            fn subscribed(&self) -> &'static [Option<HookId>] {
                &[]
            }
            fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
                r.register_data_map(FuelMap {
                    id: "minecraft:custom_fuel",
                    sync: DataMapSync::Optional,
                });
            }
        }
        assert!(install_with(vec![ModEntry::new("vanilla_ns", Box::new(VanillaNs))]).is_ok());
    }

    #[test]
    fn server_side_package_refuses_required_but_allows_optional_data_map() {
        struct ServerOptional;
        impl Hooks for ServerOptional {
            fn subscribed(&self) -> &'static [Option<HookId>] {
                &[]
            }
            fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
                r.register_data_map(FuelMap {
                    id: "srv:opt_fuel",
                    sync: DataMapSync::Optional,
                });
            }
        }
        struct ServerRequired;
        impl Hooks for ServerRequired {
            fn subscribed(&self) -> &'static [Option<HookId>] {
                &[]
            }
            fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
                r.register_data_map(FuelMap {
                    id: "srv:req_fuel",
                    sync: DataMapSync::Required,
                });
            }
        }
        crate::load_state::with_test_package_inventory(
            vec![PackageInfo::new(
                "srv".into(),
                "1.0.0".into(),
                PackageKind::Code,
                Side::Server,
                vec!["srv".into()],
                Vec::new(),
            )],
            || {
                assert!(
                    install_with(vec![ModEntry::new("srv", Box::new(ServerOptional))]).is_ok(),
                    "side = server must allow DataMapSync::Optional"
                );
                let err =
                    install_with(vec![ModEntry::new("srv", Box::new(ServerRequired))]).unwrap_err();
                assert!(
                    err.iter()
                        .any(|msg| msg.contains("srv") && msg.contains("srv:req_fuel")),
                    "side = server must refuse DataMapSync::Required naming mod, got: {err:?}"
                );
            },
        );
    }

    #[test]
    fn synced_data_map_on_non_synced_world_datapack_registry_is_refused() {
        struct VanillaNonSynced;
        impl Hooks for VanillaNonSynced {
            fn subscribed(&self) -> &'static [Option<HookId>] {
                &[]
            }
            fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
                r.register_data_map_by_name(
                    RawDataMap::new("minecraft:worldgen/structure", "test:weights")
                        .sync(DataMapSync::Optional),
                );
            }
        }
        let err = install_with(vec![ModEntry::new(
            "bad_world_sync",
            Box::new(VanillaNonSynced),
        )])
        .unwrap_err();
        assert!(
            err.iter().any(|msg| msg.contains("bad_world_sync")
                && msg.contains("minecraft:worldgen/structure")),
            "expected refusal for non-synced vanilla world datapack registry, got: {err:?}"
        );

        struct ModNonSynced;
        impl Hooks for ModNonSynced {
            fn subscribed(&self) -> &'static [Option<HookId>] {
                &[]
            }
            fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
                r.declare_datapack_registry(
                    "spell",
                    DatapackRegistrySync::ServerOnly,
                    vec![DatapackField::required(
                        "power",
                        DatapackFieldKind::Int { min: 0, max: 10 },
                    )],
                )
                .unwrap();
                r.register_data_map_by_name(
                    RawDataMap::new("spell_mod:spell", "spell_mod:weights")
                        .sync(DataMapSync::Optional),
                );
            }
        }
        let err =
            install_with(vec![ModEntry::new("spell_mod", Box::new(ModNonSynced))]).unwrap_err();
        assert!(
            err.iter()
                .any(|msg| msg.contains("spell_mod") && msg.contains("spell_mod:spell")),
            "expected refusal for non-synced mod datapack registry, got: {err:?}"
        );
    }

    #[test]
    fn synced_data_map_on_reloadable_datapack_registry_is_refused() {
        struct ReloadableSync;
        impl Hooks for ReloadableSync {
            fn subscribed(&self) -> &'static [Option<HookId>] {
                &[]
            }
            fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
                r.register_data_map_by_name(
                    RawDataMap::new("minecraft:recipe", "test:recipe_meta")
                        .sync(DataMapSync::Optional),
                );
            }
        }
        let err = install_with(vec![ModEntry::new(
            "bad_reload_sync",
            Box::new(ReloadableSync),
        )])
        .unwrap_err();
        assert!(
            err.iter()
                .any(|msg| msg.contains("bad_reload_sync") && msg.contains("minecraft:recipe")),
            "expected refusal for reloadable datapack registry, got: {err:?}"
        );
    }

    #[test]
    fn entry_key_data_map_and_reload_context_decode_and_refuse_invalid_keys() {
        use crate::{
            CustomRegistryEntry, DataMapError, EntryKey, Host, RegistryKey, RegistryOptions,
        };
        use std::sync::OnceLock;

        struct Gas;
        const GASES: RegistryKey<Gas> = RegistryKey::new("chemlib:gas");
        impl CustomRegistryEntry for Gas {
            const REGISTRY: RegistryKey<Self> = GASES;
        }

        struct OtherFluid;
        const OTHER: RegistryKey<OtherFluid> = RegistryKey::new("other:fluid");
        impl CustomRegistryEntry for OtherFluid {
            const REGISTRY: RegistryKey<Self> = OTHER;
        }

        struct GasHeatMap;
        impl DataMap for GasHeatMap {
            type Entry = EntryKey<Gas>;
            type Value = i32;
            fn id(&self) -> &str {
                "chemlib:gas_heat"
            }
            fn decode(&self, value: &Tag) -> Option<i32> {
                match value {
                    Tag::Int(n) => Some(*n),
                    _ => None,
                }
            }
        }

        static STEAM: OnceLock<EntryKey<Gas>> = OnceLock::new();
        static OXYGEN: OnceLock<EntryKey<Gas>> = OnceLock::new();

        // Each mod registers only in the namespace it owns (the custom registries'
        // namespace rule): chemlib declares `chemlib:gas`, boiler adds its own entry to it,
        // and `other` declares and fills `other:fluid`.
        struct Chemlib;
        impl Hooks for Chemlib {
            fn subscribed(&self) -> &'static [Option<HookId>] {
                &[]
            }
            fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
                r.declare_registry(GASES, RegistryOptions::new());
                let _ = OXYGEN.set(r.register_entry(GASES, "chemlib:oxygen", Gas));
                r.register_data_map(GasHeatMap);
            }
        }
        struct Boiler;
        impl Hooks for Boiler {
            fn subscribed(&self) -> &'static [Option<HookId>] {
                &[]
            }
            fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
                let _ = STEAM.set(r.register_entry(GASES, "boiler:steam", Gas));
            }
        }
        struct Other;
        impl Hooks for Other {
            fn subscribed(&self) -> &'static [Option<HookId>] {
                &[]
            }
            fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
                r.declare_registry(OTHER, RegistryOptions::new());
                let _ = r.register_entry(OTHER, "other:water", OtherFluid);
            }
        }

        install_with(vec![
            ModEntry::new("chemlib", Box::new(Chemlib)),
            ModEntry::new("boiler", Box::new(Boiler)),
            ModEntry::new("other", Box::new(Other)),
        ])
        .unwrap();
        let steam = *STEAM.get().unwrap();
        let oxygen = *OXYGEN.get().unwrap();

        struct GasMapsHost {
            rows: [(String, Tag); 1],
        }
        impl crate::prelude::DataMapsBackend for GasMapsHost {
            fn data_map_value(
                &self,
                registry: &str,
                entry: &str,
                map: &str,
            ) -> Result<Option<&Tag>, DataMapError> {
                Ok(self
                    .data_map_entries(registry, map)?
                    .iter()
                    .find_map(|(id, tag)| (id == entry).then_some(tag)))
            }
            fn data_map_entries(
                &self,
                registry: &str,
                map: &str,
            ) -> Result<&[(String, Tag)], DataMapError> {
                match (registry, map) {
                    ("chemlib:gas", "chemlib:gas_heat") => Ok(&self.rows),
                    _ => Err(DataMapError::UnknownType),
                }
            }
        }
        impl Host for GasMapsHost {
            fn backend(&self) -> Option<crate::BackendRef<'_>> {
                Some(crate::BackendRef::of(self))
            }
        }
        crate::impl_host_areas!(GasMapsHost);
        crate::impl_backend_areas!(GasMapsHost, except(data_maps));
        impl crate::Backend for GasMapsHost {}

        struct BareHost;
        impl Host for BareHost {}
        crate::impl_host_areas!(BareHost);

        let host = GasMapsHost {
            rows: [("boiler:steam".to_owned(), Tag::Int(373))],
        };
        assert_eq!(steam.data(&host, &GasHeatMap), Ok(Some(373)));
        assert_eq!(oxygen.data(&host, &GasHeatMap), Ok(None));
        assert_eq!(
            steam.data(&BareHost, &GasHeatMap),
            Err(DataMapError::Unavailable)
        );
        // Reinstalling a fresh epoch without chemlib:gas makes the old EntryKey invalid.
        install_with(vec![]).unwrap();
        assert_eq!(
            steam.data(&host, &GasHeatMap),
            Err(DataMapError::InvalidKey)
        );
    }

    #[test]
    fn two_threads_with_separate_test_parts_do_not_see_each_others_registries() {
        use crate::{Host, HostRegistries, RegistryKey, RegistryOptions};
        use std::sync::{Arc, Barrier};

        struct RegA;
        struct RegB;
        const KEY_A: RegistryKey<RegA> = RegistryKey::new("thread_a:reg");
        const KEY_B: RegistryKey<RegB> = RegistryKey::new("thread_b:reg");

        struct ModA;
        impl Hooks for ModA {
            fn subscribed(&self) -> &'static [Option<HookId>] {
                &[]
            }
            fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
                r.declare_registry(KEY_A, RegistryOptions::new());
                r.register_entry(KEY_A, "thread_a:item", RegA);
            }
        }
        struct ModB;
        impl Hooks for ModB {
            fn subscribed(&self) -> &'static [Option<HookId>] {
                &[]
            }
            fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
                r.declare_registry(KEY_B, RegistryOptions::new());
                r.register_entry(KEY_B, "thread_b:item", RegB);
            }
        }

        struct DummyHost;
        impl Host for DummyHost {}
        crate::impl_host_areas!(DummyHost);

        let barrier = Arc::new(Barrier::new(2));
        let b1 = Arc::clone(&barrier);
        let t1 = std::thread::spawn(move || {
            install_with(vec![ModEntry::new("thread_a", Box::new(ModA))]).unwrap();
            b1.wait();
            assert!(DummyHost.registry(KEY_A).is_some());
            assert!(DummyHost.registry(KEY_B).is_none());
        });
        let b2 = Arc::clone(&barrier);
        let t2 = std::thread::spawn(move || {
            install_with(vec![ModEntry::new("thread_b", Box::new(ModB))]).unwrap();
            b2.wait();
            assert!(DummyHost.registry(KEY_B).is_some());
            assert!(DummyHost.registry(KEY_A).is_none());
        });
        t1.join().unwrap();
        t2.join().unwrap();
    }
}
