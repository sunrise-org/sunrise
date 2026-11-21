//! Owner enablement, attribution and panic containment for retained resources.
use std::sync::atomic::{AtomicBool, Ordering};

pub(crate) fn call<T>(
    owner: &'static str,
    index: usize,
    poisoned: &AtomicBool,
    kind: &'static str,
    operation: &'static str,
    invoke: impl FnOnce() -> T,
) -> Result<T, String> {
    if !crate::mod_index_on(index) || poisoned.load(Ordering::Acquire) {
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
