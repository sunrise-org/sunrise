//! Startup authoring, validation, and freeze for mod-declared static custom registries.
use super::*;
use crate::Identifier;
use std::any::Any;

pub(crate) struct PendingCustomRegistry {
    pub(crate) id: Identifier,
    pub(crate) owner_mod: &'static str,
    pub(crate) options: crate::RegistryOptions,
    pub(crate) freezer: mod_vocab::__private::CustomRegistryFreezer,
    pub(crate) entries: Vec<(Identifier, Box<dyn Any + Send + Sync>)>,
    pub(crate) entry_owners: HashMap<String, &'static str>,
}

impl CustomRegistriesRegistrar for PartRegistrar {
    fn declare_custom_registry_erased(
        &mut self,
        id: &'static str,
        options: crate::RegistryOptions,
        freezer: mod_vocab::__private::CustomRegistryFreezer,
    ) {
        let Some(parsed) = Identifier::parse_namespaced(id) else {
            self.refused.push(format!(
                "mod {} declared custom registry {id:?}, which is not a valid namespaced identifier",
                self.mod_id
            ));
            return;
        };
        if parsed.namespace() == "minecraft" {
            self.refused.push(format!(
                "mod {} declared custom registry {id:?} in the reserved minecraft namespace",
                self.mod_id
            ));
            return;
        }
        if !crate::load_state::owns_namespace(self.mod_id, parsed.namespace()) {
            self.refused.push(format!(
                "mod {} declared custom registry {id:?} in foreign namespace {:?}",
                self.mod_id,
                parsed.namespace()
            ));
            return;
        }
        let canonical = parsed.to_string();
        if self.has_builtin_registry(&canonical) {
            self.refused.push(format!(
                "mod {} declared custom registry {id:?}, which collides with a built-in registry",
                self.mod_id
            ));
            return;
        }
        if options.is_synced() && self.declaration_side_not_both(id) {
            self.refused.push(format!(
                "mod {} cannot declare synced custom registry {id:?} from a non-both-side package",
                self.mod_id
            ));
            return;
        }
        if let Some(existing) = self
            .custom_registries
            .iter()
            .find(|existing| existing.id == parsed)
        {
            self.refused.push(format!(
                "custom registry {id} is declared twice, by {} and {}",
                existing.owner_mod, self.mod_id
            ));
            return;
        }
        if let Some(datapack) = self
            .datapack_registries
            .iter()
            .find(|declared| declared.key == canonical)
        {
            self.refused.push(format!(
                "custom registry {id} declared by {} collides with datapack registry declared by {}",
                self.mod_id, datapack.mod_id
            ));
            return;
        }
        if let Err(error) = self.type_identities.claim(
            TypeIdentityKind::CustomRegistry,
            &canonical,
            self.mod_id,
            TypeIdentity::typed(freezer.type_id(), freezer.type_name()),
        ) {
            self.refused.push(error);
            return;
        }
        self.custom_registries.push(PendingCustomRegistry {
            id: parsed,
            owner_mod: self.mod_id,
            options,
            freezer,
            entries: Vec::new(),
            entry_owners: HashMap::new(),
        });
    }

    fn register_custom_entry_erased(
        &mut self,
        registry_id: &'static str,
        type_name: &'static str,
        entry_id: &str,
        value: Box<dyn Any + Send + Sync>,
    ) -> (u32, u32) {
        let invalid = (u32::MAX, u32::MAX);
        let Some(registry_slot) = self
            .custom_registries
            .iter()
            .position(|registry| registry.id.as_str() == registry_id)
        else {
            self.refused.push(format!(
                "mod {} registered entry {entry_id:?} into undeclared custom registry {registry_id:?}",
                self.mod_id
            ));
            return invalid;
        };
        let actual_type_id = (*value).type_id();
        if let Err(error) = self.type_identities.claim(
            TypeIdentityKind::CustomRegistry,
            registry_id,
            self.mod_id,
            TypeIdentity::typed(actual_type_id, type_name),
        ) {
            self.refused.push(error);
            return invalid;
        }
        let Some(parsed_entry) = Identifier::parse_namespaced(entry_id) else {
            self.refused.push(format!(
                "mod {} registered invalid entry id {entry_id:?} in custom registry {registry_id:?}",
                self.mod_id
            ));
            return invalid;
        };
        if parsed_entry.namespace() == "minecraft" {
            self.refused.push(format!(
                "mod {} registered entry {entry_id:?} in the reserved minecraft namespace of custom registry {registry_id:?}",
                self.mod_id
            ));
            return invalid;
        }
        if !crate::load_state::owns_namespace(self.mod_id, parsed_entry.namespace()) {
            self.refused.push(format!(
                "mod {} registered entry {entry_id:?} in foreign namespace {:?} of custom registry {registry_id:?}",
                self.mod_id,
                parsed_entry.namespace()
            ));
            return invalid;
        }
        let synced = self.custom_registries[registry_slot].options.is_synced();
        let entry_canonical = parsed_entry.to_string();
        if synced && self.declaration_side_not_both(&entry_canonical) {
            self.refused.push(format!(
                "mod {} cannot register entry {entry_id:?} into synced custom registry {registry_id:?} from a non-both-side package",
                self.mod_id
            ));
            return invalid;
        }
        let registry = &mut self.custom_registries[registry_slot];
        if let Some(&previous_owner) = registry.entry_owners.get(&entry_canonical) {
            self.refused.push(format!(
                "custom registry {registry_id} entry {entry_id} is registered twice, by {previous_owner} and {}",
                self.mod_id
            ));
            return invalid;
        }
        let entry_slot = registry.entries.len() as u32;
        registry.entry_owners.insert(entry_canonical, self.mod_id);
        registry.entries.push((parsed_entry, value));
        (registry_slot as u32, entry_slot)
    }
}

impl PartRegistrar {
    /// Validate cross-registry collisions and configured `default_key`s, then freeze every
    /// custom static registry into its immutable typed `CustomRegistry<T>` directory.
    pub(crate) fn validate_and_freeze_custom_registries(
        &mut self,
    ) -> mod_vocab::__private::FrozenCustomRegistries {
        for registry in &self.custom_registries {
            let registry_key = registry.id.as_str();
            if let Some(datapack) = self
                .datapack_registries
                .iter()
                .find(|declared| declared.key == registry_key)
            {
                self.refused.push(format!(
                    "custom registry {} declared by {} collides with datapack registry declared by {}",
                    registry.id, registry.owner_mod, datapack.mod_id
                ));
            }
            if let Some(default_key) = registry.options.configured_default_key() {
                match Identifier::parse_namespaced(default_key) {
                    None => self.refused.push(format!(
                        "mod {} configured invalid default_key {default_key:?} on custom registry {}",
                        registry.owner_mod, registry.id
                    )),
                    Some(parsed)
                        if !registry
                            .entries
                            .iter()
                            .any(|(entry_id, _)| entry_id == &parsed) =>
                    {
                        self.refused.push(format!(
                            "mod {} configured default_key {default_key:?} on custom registry {}, but that entry was never registered",
                            registry.owner_mod, registry.id
                        ));
                    }
                    Some(_) => {}
                }
            }
        }
        if !self.refused.is_empty() {
            return mod_vocab::__private::FrozenCustomRegistries::default();
        }
        let pending = std::mem::take(&mut self.custom_registries);
        let directories = pending
            .into_iter()
            .enumerate()
            .map(|(slot, registry)| {
                let default_slot = registry
                    .options
                    .configured_default_key()
                    .and_then(|key| {
                        registry
                            .entries
                            .iter()
                            .position(|(entry_id, _)| entry_id.as_str() == key)
                    })
                    .map(|index| index as u32);
                registry.freezer.freeze(
                    slot as u32,
                    registry.id,
                    registry.options.is_synced(),
                    default_slot,
                    registry.entries,
                )
            })
            .collect();
        mod_vocab::__private::FrozenCustomRegistries::from_directories(directories)
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        EntryKey, Hooks, Host, HostRegistries, ModEntry, PackageInfo, PackageKind, Registrar,
        RegistrationContext, RegistryKey, RegistryOptions, Side,
    };
    use std::sync::{Arc, Barrier, OnceLock};

    #[derive(Debug, PartialEq, Eq)]
    struct Gas {
        color: u32,
        toxic: bool,
    }

    #[derive(Debug, PartialEq, Eq)]
    struct FakeGas {
        density: u32,
    }

    const GASES: RegistryKey<Gas> = RegistryKey::new("chemlib:gas");
    const WRONG_GASES: RegistryKey<FakeGas> = RegistryKey::new("chemlib:gas");

    struct TestHost;
    impl Host for TestHost {}
    crate::impl_host_areas!(TestHost);

    fn install(mods: &[ModEntry]) -> Result<(), Vec<String>> {
        crate::parts::install_parts(mods, &crate::parts::tests::rule_schema(), |_| false)
    }

    struct ChemLib {
        sync: bool,
        with_default: bool,
        register_default: bool,
        oxygen: Arc<OnceLock<EntryKey<Gas>>>,
    }

    #[crate::__subscriptions]
    impl Hooks for ChemLib {
        fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
            let mut opts = RegistryOptions::new().sync(self.sync);
            if self.with_default {
                opts = opts.default_key("chemlib:empty");
            }
            r.declare_registry(GASES, opts);
            if self.register_default {
                r.register_entry(
                    GASES,
                    "chemlib:empty",
                    Gas {
                        color: 0,
                        toxic: false,
                    },
                );
            }
            let key = r.register_entry(
                GASES,
                "chemlib:oxygen",
                Gas {
                    color: 0xAA_DD_FF,
                    toxic: false,
                },
            );
            let _ = self.oxygen.set(key);
        }
    }

    struct Boiler {
        steam: Arc<OnceLock<EntryKey<Gas>>>,
    }

    #[crate::__subscriptions]
    impl Hooks for Boiler {
        fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
            let key = r.register_entry(
                GASES,
                "boiler:steam",
                Gas {
                    color: 0xEE_EE_EE,
                    toxic: false,
                },
            );
            let _ = self.steam.set(key);
        }
    }

    #[test]
    fn two_mods_register_and_read_frozen_custom_registry_and_roundtrip_by_id() {
        let oxygen = Arc::new(OnceLock::new());
        let steam = Arc::new(OnceLock::new());
        install(&[
            ModEntry::new(
                "chemlib",
                Box::new(ChemLib {
                    sync: true,
                    with_default: true,
                    register_default: true,
                    oxygen: oxygen.clone(),
                }),
            ),
            ModEntry::new(
                "boiler",
                Box::new(Boiler {
                    steam: steam.clone(),
                }),
            ),
        ])
        .unwrap();

        let gases = TestHost.registry(GASES).expect("chemlib:gas installed");
        assert!(gases.is_synced());
        assert_eq!(gases.len(), 3);
        let oxygen_key = *oxygen.get().unwrap();
        let steam_key = *steam.get().unwrap();
        assert_eq!(gases.get(oxygen_key).unwrap().color, 0xAA_DD_FF);
        assert_eq!(gases.get(steam_key).unwrap().color, 0xEE_EE_EE);
        assert_eq!(gases.id_of(steam_key).unwrap().to_string(), "boiler:steam");
        assert!(TestHost.registry(WRONG_GASES).is_none());

        // Save round-trip: persist canonical id, re-register in different order in session 2.
        let saved_id = gases.id_of(steam_key).unwrap().to_string();

        struct ReorderedChem;
        #[crate::__subscriptions]
        impl Hooks for ReorderedChem {
            fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
                r.declare_registry(GASES, RegistryOptions::new());
            }
        }
        struct ReorderedBoiler;
        #[crate::__subscriptions]
        impl Hooks for ReorderedBoiler {
            fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
                r.register_entry(
                    GASES,
                    "boiler:argon",
                    Gas {
                        color: 1,
                        toxic: false,
                    },
                );
                r.register_entry(
                    GASES,
                    "boiler:steam",
                    Gas {
                        color: 0xEE_EE_EE,
                        toxic: false,
                    },
                );
            }
        }
        install(&[
            ModEntry::new("chemlib", Box::new(ReorderedChem)),
            ModEntry::new("boiler", Box::new(ReorderedBoiler)),
        ])
        .unwrap();
        let gases2 = TestHost.registry(GASES).unwrap();
        let (resolved_key, resolved_val) = gases2.by_id(&saved_id).unwrap();
        assert_eq!(resolved_val.color, 0xEE_EE_EE);
        assert_eq!(
            gases2.id_of(resolved_key).unwrap().to_string(),
            "boiler:steam"
        );
    }

    #[test]
    fn startup_refusals_cover_duplicates_type_mismatch_default_key_and_namespaces() {
        // 1. Undeclared registry
        let err = install(&[ModEntry::new(
            "boiler",
            Box::new(Boiler {
                steam: Arc::new(OnceLock::new()),
            }),
        )])
        .unwrap_err();
        assert!(err.iter().any(|m| m.contains("undeclared custom registry")));

        // 2. Duplicate registry declaration
        let err = install(&[
            ModEntry::new(
                "chemlib",
                Box::new(ChemLib {
                    sync: false,
                    with_default: false,
                    register_default: false,
                    oxygen: Arc::new(OnceLock::new()),
                }),
            ),
            ModEntry::new(
                "chemlib",
                Box::new(ChemLib {
                    sync: false,
                    with_default: false,
                    register_default: false,
                    oxygen: Arc::new(OnceLock::new()),
                }),
            ),
        ])
        .unwrap_err();
        assert!(err.iter().any(|m| m.contains("declared twice")));

        // 3. Duplicate entry id
        let err = install(&[
            ModEntry::new(
                "chemlib",
                Box::new(ChemLib {
                    sync: false,
                    with_default: false,
                    register_default: false,
                    oxygen: Arc::new(OnceLock::new()),
                }),
            ),
            ModEntry::new(
                "boiler",
                Box::new(Boiler {
                    steam: Arc::new(OnceLock::new()),
                }),
            ),
            ModEntry::new(
                "boiler",
                Box::new(Boiler {
                    steam: Arc::new(OnceLock::new()),
                }),
            ),
        ])
        .unwrap_err();
        assert!(err.iter().any(|m| m.contains("registered twice")));

        // 4. Type mismatch between declaring mod and registering mod
        struct WrongTypeMod;
        #[crate::__subscriptions]
        impl Hooks for WrongTypeMod {
            fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
                r.register_entry(WRONG_GASES, "wrong:helium", FakeGas { density: 1 });
            }
        }
        let err = install(&[
            ModEntry::new(
                "chemlib",
                Box::new(ChemLib {
                    sync: false,
                    with_default: false,
                    register_default: false,
                    oxygen: Arc::new(OnceLock::new()),
                }),
            ),
            ModEntry::new("wrong", Box::new(WrongTypeMod)),
        ])
        .unwrap_err();
        assert!(
            err.iter().any(|m| m.contains("chemlib:gas")
                && m.contains("chemlib")
                && m.contains("wrong")
                && m.contains("Gas")
                && m.contains("FakeGas")),
            "expected type mismatch naming both mods and both types, got {err:?}"
        );

        // 5. Missing default_key refused at freeze
        let err = install(&[ModEntry::new(
            "chemlib",
            Box::new(ChemLib {
                sync: false,
                with_default: true,
                register_default: false,
                oxygen: Arc::new(OnceLock::new()),
            }),
        )])
        .unwrap_err();
        assert!(
            err.iter()
                .any(|m| m.contains("default_key") && m.contains("chemlib:empty")),
            "expected missing default_key refusal, got {err:?}"
        );

        // 6. Reserved minecraft: namespace and invalid identifiers refused
        struct BadNamespaces;
        #[crate::__subscriptions]
        impl Hooks for BadNamespaces {
            fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
                r.declare_registry(
                    RegistryKey::<Gas>::new("minecraft:gas"),
                    RegistryOptions::new(),
                );
                r.declare_registry(
                    RegistryKey::<Gas>::new("not_namespaced"),
                    RegistryOptions::new(),
                );
            }
        }
        let err = install(&[ModEntry::new("chemlib", Box::new(BadNamespaces))]).unwrap_err();
        assert_eq!(err.len(), 2, "got {err:?}");
        assert!(err[0].contains("minecraft"));
        assert!(err[1].contains("valid namespaced identifier"));

        // 7. Foreign namespace on declare_registry refused
        struct ForeignRegistryDecl;
        #[crate::__subscriptions]
        impl Hooks for ForeignRegistryDecl {
            fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
                r.declare_registry(GASES, RegistryOptions::new());
            }
        }
        let err = install(&[ModEntry::new("other", Box::new(ForeignRegistryDecl))]).unwrap_err();
        assert!(
            err.iter()
                .any(|m| m.contains("other") && m.contains("chemlib")),
            "expected foreign namespace refusal on declare_registry, got {err:?}"
        );

        // 8. Foreign namespace on register_entry refused
        struct ForeignEntryReg;
        #[crate::__subscriptions]
        impl Hooks for ForeignEntryReg {
            fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
                r.register_entry(
                    GASES,
                    "chemlib:nitrogen",
                    Gas {
                        color: 2,
                        toxic: false,
                    },
                );
            }
        }
        let err = install(&[
            ModEntry::new(
                "chemlib",
                Box::new(ChemLib {
                    sync: false,
                    with_default: false,
                    register_default: false,
                    oxygen: Arc::new(OnceLock::new()),
                }),
            ),
            ModEntry::new("other", Box::new(ForeignEntryReg)),
        ])
        .unwrap_err();
        assert!(
            err.iter()
                .any(|m| m.contains("other") && m.contains("chemlib")),
            "expected foreign namespace refusal on register_entry, got {err:?}"
        );
    }

    #[test]
    fn server_side_package_refused_for_synced_registry_but_allowed_for_unsynced() {
        let server_packages = vec![
            PackageInfo::new(
                "chemlib".into(),
                "1.0.0".into(),
                PackageKind::Code,
                Side::Both,
                vec!["chemlib".into()],
                vec![],
            ),
            PackageInfo::new(
                "boiler".into(),
                "1.0.0".into(),
                PackageKind::Code,
                Side::Server,
                vec!["boiler".into()],
                vec![],
            ),
        ];

        // Server package registering into synced registry -> refused.
        crate::load_state::with_test_package_inventory(server_packages.clone(), || {
            let err = install(&[
                ModEntry::new(
                    "chemlib",
                    Box::new(ChemLib {
                        sync: true,
                        with_default: false,
                        register_default: false,
                        oxygen: Arc::new(OnceLock::new()),
                    }),
                ),
                ModEntry::new(
                    "boiler",
                    Box::new(Boiler {
                        steam: Arc::new(OnceLock::new()),
                    }),
                ),
            ])
            .unwrap_err();
            assert!(
                err.iter()
                    .any(|m| m.contains("boiler") && m.contains("synced")),
                "expected side=server refusal, got {err:?}"
            );
        });

        // Server package declaring synced registry -> refused.
        crate::load_state::with_test_package_inventory(server_packages.clone(), || {
            struct ServerSynced;
            #[crate::__subscriptions]
            impl Hooks for ServerSynced {
                fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
                    r.declare_registry(
                        RegistryKey::<Gas>::new("boiler:gas"),
                        RegistryOptions::new().sync(true),
                    );
                }
            }
            let err = install(&[ModEntry::new("boiler", Box::new(ServerSynced))]).unwrap_err();
            assert!(
                err.iter()
                    .any(|m| m.contains("boiler") && m.contains("synced"))
            );
        });

        // Server package registering into unsynced registry -> allowed (ruling point 6).
        crate::load_state::with_test_package_inventory(server_packages, || {
            install(&[
                ModEntry::new(
                    "chemlib",
                    Box::new(ChemLib {
                        sync: false,
                        with_default: false,
                        register_default: false,
                        oxygen: Arc::new(OnceLock::new()),
                    }),
                ),
                ModEntry::new(
                    "boiler",
                    Box::new(Boiler {
                        steam: Arc::new(OnceLock::new()),
                    }),
                ),
            ])
            .unwrap();
            let gases = TestHost.registry(GASES).unwrap();
            assert!(gases.by_id("boiler:steam").is_some());
        });
    }

    /// Ruling point 19: two threads each install their own parts and never see each other's
    /// custom registries or entry IDs.
    #[test]
    fn two_threads_with_independent_installations_do_not_see_each_others_registries() {
        const ALPHA: RegistryKey<Gas> = RegistryKey::new("alpha:gas");
        const BETA: RegistryKey<Gas> = RegistryKey::new("beta:gas");

        struct AlphaMod;
        #[crate::__subscriptions]
        impl Hooks for AlphaMod {
            fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
                r.declare_registry(ALPHA, RegistryOptions::new());
                r.register_entry(
                    ALPHA,
                    "alpha:oxygen",
                    Gas {
                        color: 1,
                        toxic: false,
                    },
                );
            }
        }

        struct BetaMod;
        #[crate::__subscriptions]
        impl Hooks for BetaMod {
            fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
                r.declare_registry(BETA, RegistryOptions::new());
                r.register_entry(
                    BETA,
                    "beta:steam",
                    Gas {
                        color: 2,
                        toxic: false,
                    },
                );
            }
        }

        let barrier = Arc::new(Barrier::new(2));
        let b1 = barrier.clone();
        let t1 = std::thread::spawn(move || {
            install(&[ModEntry::new("alpha", Box::new(AlphaMod))]).unwrap();
            b1.wait();
            assert!(TestHost.registry(ALPHA).is_some());
            assert!(TestHost.registry(BETA).is_none());
            let dir_ids: Vec<&str> = mod_vocab::__private::custom_registry_directories()
                .iter()
                .map(|dir| dir.id())
                .collect();
            assert_eq!(dir_ids, vec!["alpha:gas"]);
            b1.wait();
        });
        let b2 = barrier;
        let t2 = std::thread::spawn(move || {
            install(&[ModEntry::new("beta", Box::new(BetaMod))]).unwrap();
            b2.wait();
            assert!(TestHost.registry(BETA).is_some());
            assert!(TestHost.registry(ALPHA).is_none());
            let dir_ids: Vec<&str> = mod_vocab::__private::custom_registry_directories()
                .iter()
                .map(|dir| dir.id())
                .collect();
            assert_eq!(dir_ids, vec!["beta:gas"]);
            b2.wait();
        });
        t1.join().unwrap();
        t2.join().unwrap();
    }

    #[test]
    fn mismatched_erased_entry_returns_registration_error_without_freeze_panic() {
        struct SpoofedErasedMod;
        #[crate::__subscriptions]
        impl Hooks for SpoofedErasedMod {
            fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
                r.register_custom_entry_erased(
                    GASES.id(),
                    std::any::type_name::<Gas>(),
                    "spoof:helium",
                    Box::new(FakeGas { density: 1 }),
                );
            }
        }
        let err = install(&[
            ModEntry::new(
                "chemlib",
                Box::new(ChemLib {
                    sync: false,
                    with_default: false,
                    register_default: false,
                    oxygen: Arc::new(OnceLock::new()),
                }),
            ),
            ModEntry::new("spoof", Box::new(SpoofedErasedMod)),
        ])
        .unwrap_err();
        assert!(
            err.iter()
                .any(|m| m.contains("chemlib:gas") && m.contains("chemlib") && m.contains("spoof")),
            "expected named type mismatch refusal without freeze panic, got {err:?}"
        );
    }
}
