fn prepare_game_boot_with_platform(root: &std::path::Path) {
    sunrise_gameplay_common::game::test_fixtures::prepare_game_boot(
        root,
        mod_platform::__private::part_tests::register_game_keys,
    );
}

pub(crate) mod native_entity_refs {
    pub(crate) mod tests {
        pub(crate) fn with_game(test: impl FnOnce(&mut sunrise_gameplay::game::ServerGame<'_>)) {
            sunrise_client::native_item_places_test_support::with_game_with_registration(
                super::super::prepare_game_boot_with_platform,
                test,
            );
        }

        pub(crate) fn with_resident_game(
            test: impl FnOnce(&mut sunrise_gameplay::game::ServerGame<'_>),
        ) {
            sunrise_client::native_item_places_test_support::with_resident_game_with_registration(
                super::super::prepare_game_boot_with_platform,
                test,
            );
        }
    }
}

pub(crate) mod native_guest_turn_tests {
    pub(crate) fn with_guests(test: impl FnOnce(&mut sunrise_gameplay::game::ServerGame<'_>)) {
        sunrise_client::native_item_places_test_support::with_scoped_fixture_globals(|| {
            sunrise_gameplay::game::native_guest_turn_tests::with_guests(test);
        });
    }
}

pub(crate) mod native_item_places;
