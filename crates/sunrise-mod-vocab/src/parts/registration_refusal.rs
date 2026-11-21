// exports: none (reached inside the crate only)
//! Registration refusals and bounded loading messages.
use std::sync::{Arc, Mutex};

use crate::RegistrationContext;

/// Why a native declaration was not accepted.
#[derive(Clone, Debug, PartialEq, Eq, sunrise_mod_api_macros::Wire)]
#[non_exhaustive]
pub enum DeclarationError {
    /// The thread is no longer in the registration phase.
    RegistrationClosed {
        /// Identifier supplied by the caller.
        id: String,
    },
    /// The identifier is not a valid resource location.
    InvalidId {
        /// Rejected identifier, including any inferred namespace.
        id: String,
    },
    /// The mod's manifest does not own the identifier's namespace.
    UnownedNamespace {
        /// Rejected identifier.
        id: String,
        /// Namespace not available to the declaring mod.
        namespace: String,
    },
    /// A declaration failed another registry-specific check.
    Rejected {
        /// Rejected identifier.
        id: String,
        /// Registry-specific explanation.
        reason: String,
    },
}

impl DeclarationError {
    /// The identifier of the rejected declaration.
    pub fn id(&self) -> &str {
        match self {
            Self::RegistrationClosed { id }
            | Self::InvalidId { id }
            | Self::UnownedNamespace { id, .. }
            | Self::Rejected { id, .. } => id,
        }
    }
}

impl std::fmt::Display for DeclarationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::RegistrationClosed { id } => write!(f, "registration is closed for {id:?}"),
            Self::InvalidId { id } => write!(f, "invalid identifier {id:?}"),
            Self::UnownedNamespace { id, namespace } => {
                write!(f, "{id:?} in unowned namespace {namespace:?}")
            }
            Self::Rejected { id, reason } => write!(f, "{id:?}: {reason}"),
        }
    }
}
impl std::error::Error for DeclarationError {}

// no wire: engine-side admission record; only its typed error crosses to the mod.
// hidden: the game collects these records before opening any world.
#[doc(hidden)]
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct DeclarationRefusal {
    /// ID of the submitting package, not necessarily the declaration's namespace.
    pub mod_id: String,
    /// Registry or declaration kind.
    pub kind: DeclarationKind,
    /// Rejected identifier and reason.
    pub error: DeclarationError,
}

impl DeclarationRefusal {
    /// Attribute a refused declaration to the package that submitted it.
    pub fn new(mod_id: impl Into<String>, kind: DeclarationKind, error: DeclarationError) -> Self {
        Self {
            mod_id: mod_id.into(),
            kind,
            error,
        }
    }
}

// no wire: engine-side admission record discriminant.
// hidden: the game owns the admission report.
#[doc(hidden)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
// exhaustive: the engine translates every kind used in admission diagnostics.
pub enum DeclarationKind {
    /// A native block declaration.
    Block,
    /// A native item declaration.
    Item,
}

/// The most loading warnings one mod keeps; later ones are dropped.
const MAX_LOADING_WARNINGS: usize = 16;

/// A loading message as a translation key and its arguments, resolved against the
/// selected language when the game shows it (NeoForge's `ModLoadingIssue`).
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct LoadingMessage {
    pub(crate) key: String,
    pub(crate) args: Vec<String>,
}

impl LoadingMessage {
    /// A message from `key` and its `%s` arguments, each cut to 4096 bytes as `refuse` cuts.
    pub fn new(key: &str, args: &[&str]) -> Self {
        Self {
            key: bounded_text(key).to_owned(),
            args: args
                .iter()
                .map(|arg| bounded_text(arg).to_owned())
                .collect(),
        }
    }
    /// The translation key; also the text shown when no language has it.
    pub fn key(&self) -> &str {
        &self.key
    }
    /// The arguments, in `%s` order.
    pub fn args(&self) -> &[String] {
        &self.args
    }
}

/// `text` cut to at most 4096 UTF-8 bytes, at a character boundary.
fn bounded_text(text: &str) -> &str {
    let mut end = text.len().min(4096);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

/// One mod's diagnostic record, owned by the native registrar and written through its context.
// no wire: the native registrar retains this record for admission and warning publication.
// hidden: engine registration plumbing, not a mod authoring surface.
#[doc(hidden)]
#[derive(Default)]
#[non_exhaustive]
pub struct RegistrationDiagnostics {
    /// The first reason the mod gave for refusing its own registration, if any.
    pub refusal: Option<String>,
    /// Warnings recorded in call order up to the per-mod limit.
    pub warnings: Vec<LoadingMessage>,
}

/// Lend the native registrar's sink to this mod's `Hooks::register` callback.
// hidden: other registration hooks only receive the read view.
#[doc(hidden)]
pub fn registration_context_with_diagnostics(
    diagnostics: Arc<Mutex<RegistrationDiagnostics>>,
) -> RegistrationContext {
    RegistrationContext {
        diagnostics: Some(diagnostics),
        ..RegistrationContext::new()
    }
}

impl RegistrationContext {
    /// In [`Hooks::register`](crate::Hooks::register), refuse this mod's registration without
    /// panicking. The first reason wins, even if empty. Reasons over 4096 UTF-8 bytes are
    /// truncated at a character boundary; shorter reasons are preserved verbatim.
    /// All this mod's declarations from the callback are discarded, including later
    /// declarations, without adding their validation errors to the report. Other mods'
    /// declarations are unchanged. Return from the callback after calling this method.
    /// This method has no effect in other hooks.
    pub fn refuse(&self, reason: &str) {
        if let Some(diagnostics) = &self.diagnostics {
            diagnostics
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .refusal
                .get_or_insert_with(|| bounded_text(reason).to_owned());
        }
    }

    /// In [`Hooks::register`](crate::Hooks::register), record a loading problem without
    /// refusing this mod. `key` is a translation key in the mod's own `lang/` files and
    /// `args` its `%s` arguments. The log names the key and arguments; the Mods screen
    /// shows the first warning in the player's language. The first 16 warnings are kept,
    /// with each key and argument cut as [`refuse`](Self::refuse) cuts its reason.
    /// This method has no effect in other hooks.
    pub fn warn(&self, key: &str, args: &[&str]) {
        if let Some(diagnostics) = &self.diagnostics {
            let mut diagnostics = diagnostics.lock().unwrap_or_else(|p| p.into_inner());
            if diagnostics.warnings.len() < MAX_LOADING_WARNINGS {
                diagnostics.warnings.push(LoadingMessage::new(key, args));
            }
        }
    }
}

/// The loading warnings mod `id` gave during registration, in order, from the installed Parts.
// hidden: read by the game's Mods screen.
#[doc(hidden)]
pub fn registration_warnings(id: &str) -> Vec<LoadingMessage> {
    super::installed().map_or_else(Vec::new, |installed| {
        installed
            .warnings
            .iter()
            .filter(|(owner, _)| *owner == id)
            .flat_map(|(_, warnings)| warnings.iter().cloned())
            .collect()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounded_reasons_preserve_utf8_and_empty_is_a_refusal() {
        for (reason, expected) in [
            ("a".repeat(4095) + "é!", "a".repeat(4095)),
            ("é".repeat(2048), "é".repeat(2048)),
            (String::new(), String::new()),
        ] {
            let diagnostics = Arc::new(Mutex::new(RegistrationDiagnostics::default()));
            let cx = registration_context_with_diagnostics(Arc::clone(&diagnostics));
            cx.refuse(&reason);
            cx.refuse("not the first reason");
            assert_eq!(diagnostics.lock().unwrap().refusal, Some(expected));
        }
    }

    #[test]
    fn a_warning_cuts_key_and_arguments_at_a_character_boundary_and_keeps_every_argument() {
        let fits = "a".repeat(4096);
        let over = format!("{}é", "a".repeat(4095));
        let many: Vec<String> = (0..20).map(|i| i.to_string()).collect();
        let mut args: Vec<&str> = vec![&fits, &over];
        args.extend(many.iter().map(String::as_str));
        let warning = super::LoadingMessage::new(&format!("{fits}b"), &args);
        assert_eq!(warning.key(), fits);
        assert_eq!(warning.args()[0], fits);
        assert_eq!(warning.args()[1], "a".repeat(4095));
        assert_eq!(warning.args().len(), 22);
        assert_eq!(warning.args()[21], "19");
    }
}
