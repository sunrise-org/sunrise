//! Load-phase condition handlers registered via
//! [`Registrar::register_load_condition`](crate::Registrar::register_load_condition).
use crate::Identifier;
use std::sync::Arc;

/// A condition's invalid arguments or a registration refusal.
// exhaustive: mods construct this error with a diagnostic message.
#[derive(Clone, Debug, PartialEq, Eq, sunrise_mod_api_macros::Wire)]
pub struct ConditionError {
    /// Explanation included in the owning data file's reload diagnostic.
    pub message: String,
}

impl std::fmt::Display for ConditionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}
impl std::error::Error for ConditionError {}

impl ConditionError {
    /// A refusal with `message` as its diagnostic.
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl From<(Identifier, String)> for ConditionError {
    fn from((id, message): (Identifier, String)) -> Self {
        Self {
            message: format!("{id}: {message}"),
        }
    }
}

/// A handler that reads the condition's JSON from a borrowed raw value.
/// Arguments are the JSON object without the key that named it: `type` in a
/// `neoforge:conditions` list, `condition` under Fabric API's `fabric:load_conditions`.
/// Validate their shape and return `ConditionError` for invalid mod input, never
/// panic while decoding it.
/// A loader first selects the winning file and decodes its entire condition list,
/// including children of `not`, `and` and `or`. Only built-in registry and tag reads
/// order the dependencies between registry files; custom arguments are just JSON.
/// An unknown ID, malformed condition list, returned error, or native panic refuses
/// registry-backed data before publication. A simple JSON listener or data map logs
/// the error and skips the offending file instead. A panic is logged and queues an
/// error toast naming the mod, and disables only that handler for the session, across
/// loads. Evaluations that reach it report an error without calling it again. A returned
/// `ConditionError` or an ordinary phase refusal is reported without disabling the
/// handler. Handlers must not mutate published data.
pub trait LoadCondition: Send + Sync + 'static {
    /// Read and validate the raw JSON arguments when testing reaches this condition.
    /// Return `Ok(false)` when the condition does not hold: the winner is omitted, not
    /// replaced by a lower pack. Return `Err` for invalid arguments. Testing stops as
    /// soon as the enclosing list, `and`, or `or` is decided.
    fn test(&self, args: &serde_json::Value) -> Result<bool, ConditionError>;
}

/// Startup-validated, frozen load condition handler held by an
/// [`Installed`](crate::parts::Installed) epoch.
///
/// Engine plumbing only: exposed via `__private`, never re-exported from `mod_api`.
// no wire: engine-internal frozen registration holding mod callback.
// exhaustive: constructed only after startup validation.
#[derive(Clone)]
pub struct RegisteredLoadCondition {
    /// Canonical namespaced condition identifier.
    pub id: Identifier,
    /// Owning mod ID.
    pub owner_mod: &'static str,
    /// Condition handler.
    pub condition: Arc<dyn LoadCondition>,
}

impl std::fmt::Debug for RegisteredLoadCondition {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RegisteredLoadCondition")
            .field("id", &self.id)
            .field("owner_mod", &self.owner_mod)
            .finish()
    }
}

#[cfg(test)]
mod error_tests {
    #[test]
    fn condition_error_supports_standard_error_propagation() {
        fn propagate() -> Result<(), Box<dyn std::error::Error>> {
            let value: Result<(), super::ConditionError> =
                Err(super::ConditionError::new("bad value"));
            value?;
            Ok(())
        }
        assert_eq!(propagate().unwrap_err().to_string(), "bad value");
    }
}
