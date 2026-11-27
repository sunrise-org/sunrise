#![cfg(feature = "testing")]

use mod_api::prelude::BlockEntitiesHost;
use mod_api::testing::{FakeHost, with_mods};
use mod_api::{Hooks, ModEntry, Mods, Registrar, RegistrationContext};

#[test]
fn a_replaced_fake_block_entity_does_not_resolve_through_its_old_handle() {
    struct Fixture;
    #[mod_api::__subscriptions]
    impl Hooks for Fixture {
        fn register(&self, registrar: &mut dyn Registrar, _: &RegistrationContext) {
            registrar.block_entity_type_key("fixture:holder");
        }
    }

    let mods: Mods = vec![ModEntry::new("fixture", Box::new(Fixture))].into();
    with_mods(mods.clone(), || {
        mod_api::__private::part_tests::install_parts_fixture(&mods).unwrap();
        let mut host = FakeHost::new();
        let level = host.level.level();
        let pos = [0, 64, 0];
        let kind = *host
            .level
            .block_entity(pos)
            .of_type("fixture:holder")
            .kind()
            .unwrap();
        let original = host.block_entity(level, pos).unwrap();
        assert_eq!(original.kind(&host), Some(kind));

        host.level.block_entity(pos).with_items(1);
        assert_eq!(host.block_entity(level, pos), Some(original));
        assert_eq!(original.kind(&host), Some(kind));

        let removed = host.level.remove_block_entity(pos).unwrap();
        assert_eq!(host.block_entity(level, pos), None);
        assert_eq!(original.kind(&host), None);

        *host.level.block_entity(pos) = removed;
        let replacement = host.block_entity(level, pos).unwrap();
        assert_ne!(original, replacement);
        assert_eq!(original.kind(&host), None);
        assert_eq!(replacement.kind(&host), Some(kind));
    });
}
