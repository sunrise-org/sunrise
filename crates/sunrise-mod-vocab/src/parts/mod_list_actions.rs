//! Owner-scoped actions for the installed Mods screen.
use super::*;

/// What the Mods list should do after an action callback returns.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
// exhaustive: the callback can stay on its parent or request its own screen
pub enum ModListActionResult {
    /// Leave the Mods list in place.
    Stay,
    /// Request the registering mod's own screen, subject to `screen_opening`.
    OpenScreen,
}

/// Invoked only for the registering mod, with its local action key and a read-only client host.
/// There need not be a world. Return a request rather than switching screens inside the callback.
pub type ModListActionCallback = fn(&dyn crate::ClientHost, &str) -> ModListActionResult;

/// Why a Mods-list action was not registered. A refusal does not change existing actions.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ModListActionError {
    /// The owner-local key is empty, starts with `.` or contains a disallowed character.
    InvalidKey(String),
    /// The label is not a nonempty translation key without whitespace or control characters.
    InvalidLabel(String),
    /// This package already registered an action under this key.
    Duplicate(String),
}

impl std::fmt::Display for ModListActionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidKey(key) => write!(f, "invalid Mods-list action key {key:?}"),
            Self::InvalidLabel(label) => write!(f, "invalid Mods-list action label {label:?}"),
            Self::Duplicate(key) => write!(f, "Mods-list action {key:?} is registered twice"),
        }
    }
}

impl std::error::Error for ModListActionError {}

/// The current lifecycle phase does not allow reading the native Mods-list opening context.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct ModListActionUnavailable;

impl crate::AccessRefusal for ModListActionUnavailable {
    fn unavailable() -> Self {
        Self
    }
}

impl std::fmt::Display for ModListActionUnavailable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("the Mods-list action context is not readable in this phase")
    }
}

impl std::error::Error for ModListActionUnavailable {}

/// Client Mods-list registration, reached through [`Registrar`].
pub trait ModListActionsRegistrar {
    /// Add a button after the stock controls of your own installed Mods entry. Call this from
    /// [`crate::Hooks::register`]. The registrar supplies the package ID and keeps declaration
    /// order. A local key uses `a-z`, `0-9`, `_`, `-` and `.`, and cannot start with `.`. `label`
    /// is a translation key, looked up in the active language when drawn. It is not a Config page.
    ///
    /// If `activate` requests an owned screen and the request opens, the action key is readable
    /// through [`crate::prelude::ScreensClientHost::mod_list_action_key`]. Closing returns to the
    /// same entry. Refusing the screen request does not undo native side effects of the callback.
    /// A `side = "server"` package cannot declare these client controls.
    fn register_mod_list_action(
        &mut self,
        key: &'static str,
        label: &'static str,
        activate: ModListActionCallback,
    ) -> Result<(), ModListActionError>;
}

// hidden: resolved identity passed between the Mods screen and the game.
#[doc(hidden)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
// exhaustive: an engine action identifies its package and local key
pub struct ModListActionId {
    pub owner: &'static str,
    pub key: &'static str,
}

impl ModListActionId {
    pub fn new(owner: &'static str, key: &'static str) -> Self {
        Self { owner, key }
    }
}

// hidden: frozen action table entry for the engine.
#[doc(hidden)]
#[derive(Clone, Debug)]
// exhaustive: engine registration constructs this record
pub struct ModListAction {
    pub id: ModListActionId,
    pub label: &'static str,
    pub activate: ModListActionCallback,
}

impl ModListAction {
    pub fn new(id: ModListActionId, label: &'static str, activate: ModListActionCallback) -> Self {
        Self {
            id,
            label,
            activate,
        }
    }
}

// hidden: indexed engine lookup; not a mod-facing registry.
#[doc(hidden)]
pub fn mod_list_actions(owner: &str) -> &'static [ModListAction] {
    installed()
        .and_then(|parts| parts.keys.mod_list_actions.get(owner))
        .filter(|(slot, _)| crate::mod_index_on(*slot))
        .map_or(&[], |(_, actions)| actions)
}
