//! Registration and lookup plumbing for mod-declared load conditions
//! (`neoforge:conditions` and `fabric:load_conditions` custom handlers).
use super::*;
use crate::content::load_conditions::{LoadCondition, RegisteredLoadCondition};

/// Object-safe low-level registrar trait for custom load conditions.
/// Mods call [`Registrar::register_load_condition`] instead.
// hidden: typed helpers live on `dyn Registrar`; this object-safe layer is internal plumbing.
#[doc(hidden)]
pub trait LoadConditionsRegistrar {
    /// Register one boxed custom load condition handler under `id`.
    fn register_load_condition_erased(&mut self, id: &str, condition: Box<dyn LoadCondition>);
}

impl dyn Registrar + '_ {
    /// Register a custom load condition handler under canonical namespaced `id`
    /// during [`Hooks::register`](crate::Hooks::register).
    ///
    /// IDs may use any valid namespace, except built-in `neoforge:*` and `fabric:*`
    /// condition IDs. Invalid IDs, built-in IDs, and duplicate IDs across mods are
    /// refused at startup naming the mod(s).
    pub fn register_load_condition<C: LoadCondition + 'static>(&mut self, id: &str, condition: C) {
        self.register_load_condition_erased(id, Box::new(condition));
    }
}

/// All startup-frozen custom load conditions in the current installed epoch (`TEST_PARTS`
/// first, then the process-wide `INSTALLED`).
pub fn installed_load_conditions() -> &'static [RegisteredLoadCondition] {
    installed().map_or(&[], |installed| &installed.load_conditions)
}
