// exports: none (reached inside the crate only)
//! Registration refusals and bounded loading messages.

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
// hidden: registration contexts enforce the cap.
#[doc(hidden)]
pub const MAX_LOADING_WARNINGS: usize = 16;

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
pub(crate) fn bounded_text(text: &str) -> &str {
    let mut end = text.len().min(4096);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

/// Drains the context's warnings after the registration callback.
// hidden: the loader collects warnings before installing Parts.
#[doc(hidden)]
pub fn take_registration_warnings(cx: &crate::RegistrationContext) -> Vec<LoadingMessage> {
    std::mem::take(&mut *cx.warnings.lock().unwrap_or_else(|p| p.into_inner()))
}

/// The loading warnings mod `id` gave during registration, in order, from the installed Parts.
// hidden: read by the game's Mods screen.
#[doc(hidden)]
pub fn registration_warnings(id: &str) -> Vec<LoadingMessage> {
    super::installed().map_or_else(Vec::new, |installed| {
        installed
            .warnings
            .iter()
            .filter(|(owner, _)| owner == id)
            .map(|(_, warning)| warning.clone())
            .collect()
    })
}

/// Drains the context's refusal after the registration callback.
// hidden: the loader decides whether to admit the registered mod.
#[doc(hidden)]
pub fn take_registration_refusal(cx: &crate::RegistrationContext) -> Option<String> {
    cx.self_refusal
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .take()
}

#[cfg(test)]
mod tests {
    use crate::RegistrationContext;
    #[test]
    fn bounded_reasons_preserve_utf8_and_empty_is_a_refusal() {
        for (reason, expected) in [
            ("a".repeat(4095) + "é!", "a".repeat(4095)),
            ("é".repeat(2048), "é".repeat(2048)),
            (String::new(), String::new()),
        ] {
            let cx = RegistrationContext::new();
            cx.refuse(&reason);
            cx.refuse("not the first reason");
            assert_eq!(*cx.self_refusal.lock().unwrap(), Some(expected));
        }
    }

    #[test]
    fn warnings_keep_key_and_arguments_up_to_the_cap() {
        let cx = RegistrationContext::new();
        for i in 0..=super::MAX_LOADING_WARNINGS {
            cx.warn("test.warn.optional", &["jei", &i.to_string()]);
        }
        let warnings = super::take_registration_warnings(&cx);
        assert_eq!(warnings.len(), super::MAX_LOADING_WARNINGS);
        assert_eq!(warnings[0].key(), "test.warn.optional");
        assert_eq!(warnings[0].args(), ["jei", "0"]);
        assert_eq!(warnings[15].args(), ["jei", "15"]);
        assert_eq!(*cx.self_refusal.lock().unwrap(), None);
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
