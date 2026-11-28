pub(crate) use sunrise_client_gameplay::game::client_side::test_resource_cache::{
    load, load_uncached,
};

#[cfg(test)]
mod tests {
    use crate::game::resource_load::{load_resources, load_resources_uncached};
    use crate::settings::Settings;
    use std::{sync::Arc, time::Instant};
    use sunrise_assets::resource_source::ResourceRoot;
    use sunrise_client_gameplay::game::client_side::{
        resource_loader::Loaded, test_resource_cache,
    };
    use sunrise_client_gameplay::game::packs;
    use sunrise_gameplay::game::native_entity_refs::tests::{
        load_fixture_server_resources, load_fixture_server_resources_uncached,
    };
    use sunrise_gameplay_common::game::test_resource_cache::root_stamps;
    use sunrise_gameplay_common::game::{execution_role::ExecutionRole, resource_load::Progress};

    fn load_for(role: ExecutionRole) -> Loaded {
        let root = sunrise_gameplay_common::game::test_fixtures::fixture_root();
        let mut settings = Settings::default();
        settings.sunrise.workers = 1;
        load_resources(
            ResourceRoot::directory(root.clone()),
            Vec::new(),
            None,
            &root,
            &settings,
            Instant::now(),
            false,
            role,
            &Progress::new(),
            &mut |_, _| unreachable!(),
        )
        .unwrap()
    }

    #[test]
    #[ignore = "daily"]
    fn repeated_and_concurrent_loads_share_resources_while_uncached_and_mutating_hooks_stay_private()
     {
        let first = load_for(ExecutionRole::IntegratedServer);
        let second = load_for(ExecutionRole::IntegratedServer);
        assert!(first.resources.living_type_keys().player.is_some());
        assert!(Arc::ptr_eq(&first.resources, &second.resources));
        assert!(std::ptr::eq(
            first.pack.assets.registry,
            second.pack.assets.registry
        ));
        assert!(Arc::ptr_eq(
            &first.pack.item_models,
            &second.pack.item_models
        ));
        assert!(Arc::ptr_eq(&first.pack.item_ids, &second.pack.item_ids));
        assert!(first.client_pack.is_none());
        assert!(first.client_res.is_none());
        assert!(first.gui.is_none());

        let root = sunrise_gameplay_common::game::test_fixtures::fixture_root();
        let role = ExecutionRole::IntegratedServer;
        let (server, _) = load_fixture_server_resources(&root, role);
        let (server_again, _) = load_fixture_server_resources(&root, role);
        assert!(Arc::ptr_eq(&server, &first.resources));
        assert!(Arc::ptr_eq(&server, &server_again));
        let (mut private_server, _) = load_fixture_server_resources_uncached(&root, role);
        assert!(!Arc::ptr_eq(&server, &private_server));
        assert!(Arc::get_mut(&mut private_server).is_some());

        let settings = Settings::default();
        let client_args = || ResourceRoot::directory(root.clone()).into();
        let load_client = |cached: bool| {
            let load = if cached {
                test_resource_cache::load
            } else {
                test_resource_cache::load_uncached
            };
            load(
                client_args(),
                Vec::new(),
                None,
                &root,
                &settings,
                Instant::now(),
                false,
                false,
                ExecutionRole::Client.into(),
                &Progress::new(),
                &mut |_, _| unreachable!(),
            )
            .unwrap()
        };
        let c1 = load_client(true);
        let mut c2 = load_client(true);
        assert!(Arc::ptr_eq(&c1.resources, &c2.resources));
        assert!(Arc::ptr_eq(&c1.pack.item_models, &c2.pack.item_models));
        assert!(Arc::ptr_eq(&c1.pack.item_ids, &c2.pack.item_ids));
        let c1_pack = c1.client_pack.as_ref().unwrap();
        let c2_pack = c2.client_pack.as_mut().unwrap();
        assert!(!Arc::ptr_eq(&c1_pack.item_renderer, &c2_pack.item_renderer));
        assert!(Arc::get_mut(&mut c2_pack.item_renderer).is_some());
        let mut private_client = load_client(false);
        assert!(!Arc::ptr_eq(&c1.resources, &private_client.resources));
        assert!(Arc::get_mut(&mut private_client.resources).is_some());
        assert!(
            Arc::get_mut(&mut private_client.client_pack.as_mut().unwrap().item_renderer).is_some()
        );

        // Concurrent calls on separate threads all share the exact same `Arc<Resources>`.
        let others: Vec<_> = std::thread::scope(|scope| {
            (0..4)
                .map(|_| {
                    let root = &root;
                    scope.spawn(move || {
                        let (server, _) = load_fixture_server_resources(root, role);
                        let shell = load_for(role);
                        assert!(Arc::ptr_eq(&server, &shell.resources));
                        server
                    })
                })
                .collect::<Vec<_>>()
                .into_iter()
                .map(|h| h.join().unwrap())
                .collect()
        });
        for res in others {
            assert!(Arc::ptr_eq(&first.resources, &res));
        }

        // `load_resources_uncached` always returns a private `Arc<Resources>`.
        let mut settings = Settings::default();
        settings.sunrise.workers = 1;
        let uncached = load_resources_uncached(
            ResourceRoot::directory(root.clone()),
            Vec::new(),
            None,
            &root,
            &settings,
            Instant::now(),
            false,
            ExecutionRole::IntegratedServer,
            &Progress::new(),
            &mut |_, _| unreachable!(),
        )
        .unwrap();
        assert!(!Arc::ptr_eq(&first.resources, &uncached.resources));

        // A custom block key still shares `Resources`/`BlockRegistry::Data` with private worker slots.
        struct CustomBlockKey;
        #[mod_api::__subscriptions]
        impl mod_api::Hooks for CustomBlockKey {
            fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
                r.block_key("minecraft:oak_sign");
            }
        }
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
            "cache_custom_key",
            Box::new(CustomBlockKey),
        )]
        .into();
        mod_platform::testing::with_mods(mods.clone(), || {
            sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
            let modded = load_for(ExecutionRole::IntegratedServer);
            assert!(Arc::ptr_eq(&first.resources, &modded.resources));
            assert!(!std::ptr::eq(
                first.pack.assets.registry,
                modded.pack.assets.registry
            ));
            assert_eq!(
                first.pack.assets.registry.payload_identity(),
                modded.pack.assets.registry.payload_identity()
            );
        });

        // A mod subscribing to a `Resources`-mutating hook (`CollisionShape`) bypasses cache.
        struct MutatingPart;
        #[mod_api::sunrise_part]
        impl mod_api::BlockPart for MutatingPart {
            fn collision_shape(
                &self,
                _: &dyn mod_api::LoopContext,
                _: &mut mod_api::CollisionShapeEvent<'_>,
            ) {
            }
        }
        struct MutatingMod;
        #[mod_api::__subscriptions]
        impl mod_api::Hooks for MutatingMod {
            fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
                let key = r.block_key("minecraft:stone");
                r.block_part(key, MutatingPart);
            }
        }
        let mutating: mod_api::Mods = vec![mod_api::ModEntry::new(
            "cache_mutating",
            Box::new(MutatingMod),
        )]
        .into();
        mod_platform::testing::with_mods(mutating.clone(), || {
            sunrise_gameplay_common::game::api_values::install_parts(&mutating).unwrap();
            let private = load_for(ExecutionRole::IntegratedServer);
            assert!(!Arc::ptr_eq(&first.resources, &private.resources));
        });
    }

    #[test]
    fn root_stamps_change_when_prepared_files_change_and_overlaid_roots_bypass_cache() {
        let dir = tempfile::tempdir().unwrap();
        let before = root_stamps(dir.path());
        std::fs::write(dir.path().join("facts.json"), b"{}").unwrap();
        let after = root_stamps(dir.path());
        assert_ne!(before, after);

        let plain: packs::PackRoots = ResourceRoot::directory(dir.path().to_path_buf()).into();
        assert_eq!(
            test_resource_cache::plain_directory_root(&plain),
            Some(dir.path().to_path_buf())
        );

        let overlaid: packs::PackRoots = ResourceRoot::directory(dir.path().to_path_buf())
            .with_mod_layers(Arc::from([
                sunrise_assets::resource_source::ResourceSource::default(),
            ]))
            .into();
        assert_eq!(test_resource_cache::plain_directory_root(&overlaid), None);
    }
}
