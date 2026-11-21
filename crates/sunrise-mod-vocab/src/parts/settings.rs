use super::*;
use std::sync::atomic::AtomicBool;

/// Owner-bound settings retained for the client's mod pages and typed reads.
pub fn settings() -> &'static [(&'static str, crate::SettingSpec)] {
    installed().map_or(&[], |installed| &installed.keys.settings)
}

/// The setting `key` mod `mod_id` registered, if any.
pub fn setting(mod_id: &str, key: &str) -> Option<&'static crate::SettingSpec> {
    settings()
        .iter()
        .find(|(owner, spec)| *owner == mod_id && spec.key == key)
        .map(|(_, spec)| spec)
}

/// A mod's Config button factory, analogous to NeoForge's `IConfigScreenFactory`.
pub trait ConfigScreenFactory: Send + Sync + 'static {
    /// Called once per Config activation, for the loader-resolved `owner` only.
    /// Return a fresh session for `parent` or refuse without changing screens.
    /// Build UI state here; draw and handle input in the usual screen hooks.
    fn create_screen(
        &self,
        owner: &str,
        context: ConfigScreenContext,
        parent: ConfigScreenParent,
    ) -> Result<ConfigScreenSession, ConfigScreenRefusal>;
}

/// The client UI location of a Config activation. It does not grant world access.
#[derive(Clone, Copy, Debug)]
pub struct ConfigScreenContext {
    in_world: bool,
}

impl ConfigScreenContext {
    /// Whether the Mods list is in a world instead of on the title screen.
    pub fn in_world(self) -> bool {
        self.in_world
    }

    #[doc(hidden)]
    pub fn new(in_world: bool) -> Self {
        Self { in_world }
    }
}

/// An opaque return target for this activation's Mods list.
///
/// Clone it if your Back control needs to request a close from a read-only
/// screen hook. A retained target has no effect after its session closes.
#[derive(Clone)]
pub struct ConfigScreenParent(Arc<AtomicBool>);

impl ConfigScreenParent {
    /// Build a new screen session over this Mods list.
    pub fn screen(self) -> ConfigScreenSession {
        ConfigScreenSession { parent: self }
    }

    /// Request the active session's normal close, as if Escape were pressed.
    /// This takes effect after the current input or frame, if the session opened.
    pub fn go_back(&self) {
        self.0.store(true, Ordering::Relaxed);
    }

    #[doc(hidden)]
    pub fn new_activation() -> Self {
        Self(Arc::new(AtomicBool::new(false)))
    }
}

/// One activation of a mod's Config screen. Build it from the supplied return target.
pub struct ConfigScreenSession {
    parent: ConfigScreenParent,
}

impl ConfigScreenSession {
    #[doc(hidden)]
    pub fn belongs_to(&self, parent: &ConfigScreenParent) -> bool {
        Arc::ptr_eq(&self.parent.0, &parent.0)
    }

    #[doc(hidden)]
    pub fn take_back_request(&self) -> bool {
        self.parent.0.swap(false, Ordering::Relaxed)
    }
}

/// A factory declined to create a screen. The Mods list stays open.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ConfigScreenRefusal {
    /// The mod cannot present its configuration screen right now.
    Unavailable,
}

/// Why a Config screen factory could not be registered.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ConfigScreenRegistrationError {
    /// The owner already registered a factory. The first one remains installed.
    Duplicate,
    /// This process does not provide the client Config UI.
    NotClient,
}

/// Resolve a registered factory and its owner, not package metadata. Its
/// dispatch index was captured when the loader invoked registration.
pub fn config_screen_factory(
    owner: &str,
) -> Option<(&'static str, &'static Arc<dyn ConfigScreenFactory>)> {
    let (owner, (index, factory)) = installed()?
        .keys
        .config_screen_factories
        .get_key_value(owner)?;
    let loaded = crate::mods().get(*index)?;
    (loaded.id == *owner && crate::mod_index_on(*index)).then_some((*owner, factory))
}

/// Registration fixtures without a process environment use the client side.
pub fn client_registration(context: &crate::RegistrationContext) -> bool {
    context
        .runtime
        .as_ref()
        .is_none_or(|runtime| runtime.dist() == crate::Dist::Client)
}

/// Registrar operations for settings.
pub trait SettingsRegistrar {
    /// Register a setting shown on the calling mod's page in Options > Mods (NeoForge's
    /// `ModConfigSpec` entries). Called from `Hooks::register`; client settings only. A setting
    /// [`SettingSpec::check`](crate::SettingSpec::check) refuses, or a key the mod registered
    /// already, is not registered and comes back as the error.
    fn register_setting(&mut self, setting: crate::SettingSpec) -> Result<(), crate::SettingError>;

    /// Register this mod's custom Config screen during `Hooks::register` on the client.
    /// It takes precedence over generated settings, even when there are no rows.
    /// A second registration returns `Duplicate` without replacing the first.
    fn register_config_screen_factory(
        &mut self,
        factory: Box<dyn ConfigScreenFactory>,
    ) -> Result<(), ConfigScreenRegistrationError>;
}
