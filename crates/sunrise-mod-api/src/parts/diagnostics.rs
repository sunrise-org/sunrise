//! Platform-owned warning history, never part of an engine lookup table.
use std::{
    cell::RefCell,
    collections::HashSet,
    sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
    },
};
static BROKEN: Mutex<Option<HashSet<String>>> = Mutex::new(None);
static OFF_THREAD_WARNED: AtomicBool = AtomicBool::new(false);

thread_local! {
    // Travel's authoritative thread keeps only this tick's refusals. The set
    // needs no heap allocation until a listener produces malformed output.
    static OUTPUT_REFUSALS: RefCell<HashSet<(usize, &'static str)>> =
        RefCell::new(HashSet::new());
}

pub(crate) const DIAGNOSTICS: mod_vocab::dispatch_scope::Diagnostics =
    mod_vocab::dispatch_scope::Diagnostics {
        first_broken_warning: |owner| {
            BROKEN
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .get_or_insert_default()
                .insert(owner.to_owned())
        },
        first_off_thread_warning: || !OFF_THREAD_WARNED.swap(true, Ordering::Relaxed),
        first_output_refusal: |hook, id| {
            OUTPUT_REFUSALS.with(|refusals| refusals.borrow_mut().insert((hook as usize, id)))
        },
        reset_output_refusals: || OUTPUT_REFUSALS.with(|refusals| refusals.borrow_mut().clear()),
    };

#[test]
fn output_refusal_diagnostics_are_once_per_listener_per_tick() {
    let hook = mod_vocab::HookId::LivingTravel;
    let first = DIAGNOSTICS.first_output_refusal;
    (DIAGNOSTICS.reset_output_refusals)();
    assert!(first(hook, "first"));
    assert!(!first(hook, "first"));
    assert!(first(hook, "second"));
    (DIAGNOSTICS.reset_output_refusals)();
    assert!(first(hook, "first"));
}

#[cfg(test)]
pub(crate) fn warned_about(owner: &str) -> bool {
    BROKEN
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
        .is_some_and(|owners| owners.contains(owner))
}
