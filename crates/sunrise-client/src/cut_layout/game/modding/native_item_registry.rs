#![cfg(test)]

#[cfg(test)]
mod tests {
    use mod_api::{Hooks, Identifier, Item, ItemsEvent, ModEntry, RegistrationContext};
    use sunrise_protocol::registry::Builtin;
    use {
        sunrise_gameplay::game::native_entity_refs::tests as fixture,
        sunrise_gameplay_common::game::execution_role::ExecutionRole,
    };

    struct Probe;
    #[mod_api::__subscriptions]
    impl Hooks for Probe {
        fn items(&self, _: &RegistrationContext, event: &mut ItemsEvent<'_>) {
            for item in
                ["numbers:z_item", "numbers:a_item"].map(|id| Item::new(id, "minecraft:stick"))
            {
                event.register(item).unwrap();
            }
        }
    }

    #[test]
    #[ignore = "daily"]
    fn item_numbers_follow_the_prepared_content_plan_for_api_and_scatter() {
        if fixture::run_fresh_cases(
            "cut_layout::game::native_item_registry::tests::item_numbers_follow_the_prepared_content_plan_for_api_and_scatter",
            "SUNRISE_ITEM_NUMBERS_CHILD",
            &["valid"],
        ) {
            return;
        }
        let base = std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
        let vanilla = Builtin::load(&Builtin::find(&base).unwrap()).unwrap();
        let vanilla = vanilla.get("minecraft:item").unwrap();
        let first_mod_id = u32::try_from(*vanilla.ids.values().max().unwrap()).unwrap() + 1;
        let mods: mod_api::Mods = vec![ModEntry::new("numbers", Box::new(Probe))].into();
        mod_platform::testing::with_mods(mods.clone(), || {
            sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
            let root =
                sunrise_gameplay_common::game::mods::content_root(&base, &Default::default())
                    .unwrap();
            crate::game::native_entity_refs::tests::with_game_role_client(
                root.prepared().to_owned(),
                ExecutionRole::Client,
                |game| {
                    for (id, expected) in [
                        (
                            "minecraft:stone",
                            u32::try_from(vanilla.id("minecraft:stone").unwrap()).unwrap(),
                        ),
                        ("numbers:a_item", first_mod_id),
                        ("numbers:z_item", first_mod_id + 1),
                    ] {
                        let key = game.item_key(&Identifier::parse(id).unwrap()).unwrap();
                        assert_eq!(key.registry_id(game), Some(expected), "{id}: API");
                        assert_eq!(game.item_number(id), Some(expected), "{id}: scatter");
                    }
                    let missing = "numbers:missing";
                    assert!(
                        game.item_key(&Identifier::parse(missing).unwrap())
                            .is_none()
                    );
                    assert_eq!(game.item_number(missing), None);
                },
            );
        });
    }
}
