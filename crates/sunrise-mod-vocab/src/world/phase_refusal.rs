//! A native Host call refused by the phase gate: the latch that remembers it for the rest of the
//! mod's call, and the one helper that turns it into a value at the entry point.
//!
//! The scope state is a thread-local of `mod_vocab`, reached only through the non-inline
//! functions below. The SDK ships `mod_vocab` as a dylib shared by the game and every native
//! mod, so inlined callers never hold a second copy of the state.
use super::phase::ModPhase;
use std::cell::Cell;
use std::collections::HashSet;
use std::sync::{Mutex, OnceLock};

// no wire: the payload of an unwind inside one process, not serialized data.
/// What the phase gate records and unwinds with when a native mod calls a Host method its load
/// phase does not admit. `contain_refusal` turns it into an `Err` at the entry point.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
// exhaustive: the gate builds it as a complete record
pub struct PhaseRefusal {
    /// The `[mod] id` of the mod whose call was refused, `unknown` outside instrumented dispatch.
    pub mod_id: &'static str,
    /// The refused Host method.
    pub operation: &'static str,
    /// The phase the thread was in.
    pub phase: ModPhase,
}
impl std::fmt::Display for PhaseRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "mod {}: mod API {} is unavailable during {:?}",
            self.mod_id, self.operation, self.phase
        )
    }
}

impl PhaseRefusal {
    /// Logs the first refusal of this operation for this mod.
    pub fn report(&self) {
        static LOGGED: OnceLock<Mutex<HashSet<(&'static str, &'static str)>>> = OnceLock::new();
        let first = LOGGED
            .get_or_init(Mutex::default)
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .insert((self.mod_id, self.operation));
        if first {
            log::warn!(
                "native refusal: {} hook {}: mod API {} is unavailable during {:?}",
                self.mod_id,
                crate::content::crash::current_native_call().map_or("unknown", |call| call.hook),
                self.operation,
                self.phase
            );
        }
    }
}

/// One containment scope: the mod it runs and the first refusal recorded in it.
#[derive(Clone, Copy)]
struct Scope {
    mod_id: &'static str,
    refusal: Option<PhaseRefusal>,
}
thread_local! {
    static SCOPE: Cell<Option<Scope>> = const { Cell::new(None) };
}

// hidden: the saved outer scope `contain_refusal` restores, not a mod authoring surface.
#[doc(hidden)]
// no wire: thread-local scope bookkeeping.
// exhaustive: only `enter_scope` builds it
pub struct OuterScope(Option<Scope>);

/// Opens a scope for `mod_id`'s call; nested scopes save and restore the outer one.
// hidden: called by `contain_refusal` only.
#[doc(hidden)]
#[inline(never)]
pub fn enter_scope(mod_id: &'static str) -> OuterScope {
    OuterScope(SCOPE.replace(Some(Scope {
        mod_id,
        refusal: None,
    })))
}

/// Closes the scope, restores the outer one, and returns the refusal the scope latched.
// hidden: called by `contain_refusal` only.
#[doc(hidden)]
#[inline(never)]
pub fn leave_scope(outer: OuterScope) -> Option<PhaseRefusal> {
    SCOPE.replace(outer.0).and_then(|scope| scope.refusal)
}

/// Whether this thread's scope already holds a refusal: every later gated call refuses too.
// hidden: read by `check_access` only.
#[doc(hidden)]
#[inline(never)]
pub fn latched() -> bool {
    SCOPE.get().is_some_and(|scope| scope.refusal.is_some())
}

/// Refuses a gated call: records it in the scope (the first refusal stays) and unwinds with a
/// [`PhaseRefusal`] and no panic hook, so no crash report is made.
///
/// While the thread is already unwinding, the line is logged and the gate panics: a panic
/// during an unwind aborts the process. With no scope on the thread it panics with the named
/// message, as the gate did before refusals were contained.
// hidden: the gate's cold path.
#[doc(hidden)]
#[cold]
#[inline(never)]
pub fn refuse(operation: &'static str, phase: ModPhase) -> ! {
    let named = format!("mod API {operation} is unavailable during {phase:?}");
    if std::thread::panicking() {
        let call = crate::content::crash::current_native_call();
        log::error!(
            "native refusal: {} hook {}: {named}, called while the thread was already unwinding; the process aborts",
            call.map_or("unknown", |call| call.id),
            call.map_or("unknown", |call| call.hook)
        );
        panic!("{named}");
    }
    let Some(mut scope) = SCOPE.get() else {
        panic!("{named}");
    };
    let refusal = *scope.refusal.get_or_insert(PhaseRefusal {
        mod_id: scope.mod_id,
        operation,
        phase,
    });
    SCOPE.set(Some(scope));
    std::panic::resume_unwind(Box::new(refusal))
}

/// Runs `f` as `mod_id`'s call. A gated call refused inside it comes back as `Err`; so does a
/// call that returned normally after its mod swallowed the unwind, because the latch stays set
/// in the scope. Any other panic goes on unwinding, so real faults crash as before.
pub fn contain_refusal<T>(mod_id: &'static str, f: impl FnOnce() -> T) -> Result<T, PhaseRefusal> {
    let outer = enter_scope(mod_id);
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f));
    let latched = leave_scope(outer);
    match (result, latched) {
        (Ok(value), None) => Ok(value),
        (Ok(_), Some(refusal)) => Err(refusal),
        (Err(payload), latched) => match payload.downcast::<PhaseRefusal>() {
            Ok(refusal) => Err(latched.unwrap_or(*refusal)),
            Err(other) => std::panic::resume_unwind(other),
        },
    }
}

/// [`contain_refusal`] for a hand-written call of mod code outside generated dispatch, on the
/// current native mod, and only outside `World`: in `World` it is the plain call. A call site
/// answers the refusal as "the mod did not answer" and reports it.
pub fn contain_outside_world<T>(f: impl FnOnce() -> T) -> Result<T, PhaseRefusal> {
    crate::runtime::refusable(crate::runtime::outside_world(), f)
}
