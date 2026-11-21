//! Owner enablement, attribution and panic containment for retained resources.
use std::sync::atomic::{AtomicBool, Ordering};

/// Owner identity and poison state for one retained mod callback.
// no wire: the callback's enablement and poison state belong to this process.
// exhaustive: engine-only callback bookkeeping; private fields prevent downstream construction.
pub struct CallbackOwner {
    id: &'static str,
    index: usize,
    poisoned: AtomicBool,
}

impl CallbackOwner {
    /// Bind a dispatch-attributed registration to its installed mod slot once, at freeze time.
    pub fn new(id: &'static str) -> Option<Self> {
        Some(Self {
            id,
            index: crate::__private::mod_index(id)?,
            poisoned: AtomicBool::new(false),
        })
    }

    /// Invoke retained code with its owner's enablement, attribution and panic policy.
    pub fn call<T>(
        &self,
        kind: &'static str,
        operation: &'static str,
        invoke: impl FnOnce() -> T,
    ) -> Result<T, String> {
        call(self.id, self.index, &self.poisoned, kind, operation, invoke)
            .map_err(|why| format!("mod {}: {why}", self.id))
    }
}

pub(crate) fn enabled(index: usize, poisoned: &AtomicBool) -> bool {
    crate::mod_index_on(index) && !poisoned.load(Ordering::Acquire)
}

pub(crate) fn call<T>(
    owner: &'static str,
    index: usize,
    poisoned: &AtomicBool,
    kind: &'static str,
    operation: &'static str,
    invoke: impl FnOnce() -> T,
) -> Result<T, String> {
    if !enabled(index, poisoned) {
        return Err(format!("{kind} owner is disabled"));
    }
    let _guard = crate::mark_mod(owner, operation);
    // A refusal by the phase gate refuses this call and nothing more; a real panic disables the owner.
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        crate::world::phase_refusal::contain_refusal(owner, invoke)
    })) {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(refusal)) => {
            refusal.report();
            Err(format!("{kind} callback refused: {refusal}"))
        }
        Err(_) => {
            poisoned.store(true, Ordering::Release);
            Err(format!("{kind} callback panicked; it is disabled"))
        }
    }
}
