//! Refusals when a client mod rebinds or resets one of its own key mappings.

/// Why [`set_mod_key`](crate::prelude::ClientInputClientHost::set_mod_key) or
/// [`reset_mod_key`](crate::prelude::ClientInputClientHost::reset_mod_key) changed nothing.
/// Every refusal leaves the binding and `sunrise-mods.txt` as they were.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub enum ModKeyRefusal {
    /// The mod has no registered mapping of that name.
    NotRegistered,
    /// A native mod asked for another mod's mapping: a mod rebinds only its own.
    NotOwner,
    /// The key is not a key or mouse button name Controls knows (`key.keyboard.z`,
    /// `key.mouse.left`, `key.keyboard.unknown` for none).
    UnknownKey,
    /// The modifiers hold a bit other than shift (1), control (2) and alt (4).
    UnknownModifiers,
    /// Asked with no game backend: key bindings are the client's.
    Unavailable,
}

impl std::fmt::Display for ModKeyRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::NotRegistered => "the mod registered no key mapping of that name",
            Self::NotOwner => "a mod rebinds only its own key mappings",
            Self::UnknownKey => "the key is not a key or mouse button name",
            Self::UnknownModifiers => "modifiers are shift (1), control (2) and alt (4) only",
            Self::Unavailable => "key bindings are on the client",
        })
    }
}

impl std::error::Error for ModKeyRefusal {}

impl crate::NoBackend for ModKeyRefusal {
    fn no_backend() -> Self {
        Self::Unavailable
    }
}
