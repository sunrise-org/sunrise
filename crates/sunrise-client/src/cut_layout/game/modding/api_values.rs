#[path = "api_values/small_values.rs"]
pub(crate) mod small_values;

#[cfg(test)]
pub(crate) fn install_test_platform() {
    mod_platform::__private::test_platform::install();
}

/// The common crate's `install_parts` for this crate's tests, with the test platform installed
/// first: the common crate installs it only under its own `cfg(test)`, which this crate's tests
/// don't set, so a test calling the common one directly passes only after another test installed
/// the platform in the same process.
#[cfg(test)]
pub(crate) fn install_parts(mods: &[mod_api::ModEntry]) -> Result<(), Vec<String>> {
    install_test_platform();
    sunrise_gameplay_common::game::api_values::install_parts(mods)
}

#[cfg(test)]
pub(crate) fn register_game_keys(ids: impl IntoIterator<Item = String>) {
    sunrise_gameplay_common::game::test_fixtures::register_game_keys(
        ids,
        mod_platform::__private::part_tests::register_game_keys,
    );
}
