use super::*;
/// Owner-bound declarations retained for client mod-list rebuilds.
pub fn key_mappings() -> &'static [(&'static str, crate::KeyRegistration)] {
    installed().map_or(&[], |installed| &installed.keys.key_mappings)
}

/// Registrar operations for key mappings.
pub trait KeyMappingsRegistrar {
    /// Register a Controls binding owned by the calling mod (`RegisterKeyMappingsEvent`).
    /// Called from `Hooks::register`; invalid or duplicate local ids refuse startup. The mapping
    /// appears in Options > Controls under the mod's title and, unless the mod registered its own
    /// config screen (`SettingsRegistrar::register_config_screen`), as a Key Binds row on its
    /// generated Config page; both places rebind and reset the same player-chosen key.
    fn register_key_mapping(&mut self, mapping: crate::KeyRegistration);
}
