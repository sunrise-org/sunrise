pub use crate::parts::{DeclarationError, LoadingMessage};
mod __private {
    pub use crate::parts::{
        DeclarationKind, DeclarationRefusal, MAX_LOADING_WARNINGS, registration_warnings,
        take_declaration_refusals, take_registration_refusal, take_registration_warnings,
    };
}
