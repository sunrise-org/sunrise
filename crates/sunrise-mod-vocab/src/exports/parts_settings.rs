//! Exports of `crate::parts` items defined in `parts/settings.rs`.
pub use crate::parts::{
    ConfigScreenContext, ConfigScreenFactory, ConfigScreenParent, ConfigScreenRefusal,
    ConfigScreenRegistrationError, ConfigScreenSession,
};
mod __private {
    pub use crate::parts::{client_registration, config_screen_factory, setting, settings};
}
