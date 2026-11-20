//! Native runtime diagnostics bound to the installed mod's manifest ID.
use std::{fmt, panic::Location, sync::OnceLock};

/// Severity of a runtime diagnostic, independent of any logging crate.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum LogLevel {
    /// An error or failed operation.
    Error,
    /// A recoverable problem.
    Warn,
    /// An ordinary diagnostic.
    Info,
    /// Debugging detail.
    Debug,
    /// Fine-grained tracing detail.
    Trace,
}

/// The result of submitting a record to the game log.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum LogOutcome {
    /// The writer accepted the record. This does not acknowledge a disk write.
    Queued,
    /// The level filter or warning/error flood control suppressed this record.
    Filtered,
    /// The game log is not available. No record was queued.
    Unavailable,
}

impl LogLevel {
    fn facade(self) -> log::Level {
        match self {
            Self::Error => log::Level::Error,
            Self::Warn => log::Level::Warn,
            Self::Info => log::Level::Info,
            Self::Debug => log::Level::Debug,
            Self::Trace => log::Level::Trace,
        }
    }
}

type Enqueue =
    fn(&'static str, log::Level, fmt::Arguments<'_>, &'static Location<'static>) -> LogOutcome;

struct Sink {
    enabled: fn(log::Level) -> bool,
    enqueue: Enqueue,
}

static SINK: OnceLock<Sink> = OnceLock::new();

/// Install the game's log sink before loading mods.
// hidden: engine plumbing, re-exported through __private for the game.
#[doc(hidden)]
pub fn install_mod_log_sink(enabled: fn(log::Level) -> bool, enqueue: Enqueue) {
    let _ = SINK.set(Sink { enabled, enqueue });
}

/// A native mod's own logger, from [`RegistrationContext::logger`](crate::RegistrationContext::logger).
/// Keep or copy it into hooks, Parts or workers. It needs no world, Host or transaction;
/// a queued record remains queued if registration is refused or a transaction rolls back.
// exhaustive: its field is private, so mods cannot construct or retarget it.
#[derive(Clone, Copy)]
pub struct ModLogger {
    id: Option<&'static str>,
}

impl ModLogger {
    /// Whether the game's log is available and its level filter accepts `level`.
    /// INFO, WARN and ERROR are enabled by default. Check before computing expensive
    /// arguments. Warning/error flood control is applied
    /// separately at the call to [`log`](Self::log).
    pub fn enabled(&self, level: LogLevel) -> bool {
        self.id.is_some()
            && SINK
                .get()
                .is_some_and(|sink| (sink.enabled)(level.facade()))
    }

    /// Queue a record on the calling thread, with this mod's ID and `level`.
    /// Use `format_args!(...)` to borrow the message. Filtered records do not run their
    /// formatter, and neither do calls made after the writer has stopped. This writes a
    /// diagnostic, not chat or a Mods-screen loading warning.
    #[track_caller]
    pub fn log(&self, level: LogLevel, message: fmt::Arguments<'_>) -> LogOutcome {
        match (self.id, SINK.get()) {
            (Some(id), Some(sink)) => {
                (sink.enqueue)(id, level.facade(), message, Location::caller())
            }
            _ => LogOutcome::Unavailable,
        }
    }
}

impl crate::RegistrationContext {
    /// Get the native logger from the context passed to [`Hooks::register`](crate::Hooks::register).
    /// The manifest ID is already bound; no target or namespace is supplied by the mod.
    /// A context not bound to a native registration entry yields an unavailable logger.
    pub fn logger(&self) -> ModLogger {
        ModLogger {
            id: self.native_mod_id,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::crash::{self, ModCall};
    use crate::{HookId, Hooks, ModEntry};

    #[test]
    fn registration_logger_uses_the_entry_even_inside_another_mods_crash_scope() {
        struct Quiet;
        impl Hooks for Quiet {
            fn subscribed(&self) -> &'static [Option<HookId>] {
                &[]
            }
        }
        install_mod_log_sink(
            |_| true,
            |id, _, _, _| {
                assert_eq!(id, "actual_manifest_id");
                LogOutcome::Queued
            },
        );
        let mods = [ModEntry::new("actual_manifest_id", Box::new(Quiet))];
        let mut registration = crate::runtime::entered_registration(&mods, "register");
        let (_, context) = registration.next().unwrap();
        // The vocabulary unit binary has no platform instrumentation backend. Change the
        // same crash owner that enter_native_mod sets in an instrumented game.
        let (frame, outer) = crash::push(ModCall {
            id: "other_mod",
            hook: "register",
            part: None,
        });
        let owner = crash::current_native_mod();
        let logger = context.logger();
        crash::pop(frame, outer);
        assert_eq!(owner, Some("other_mod"));
        assert_eq!(
            logger.log(LogLevel::Info, format_args!("ready")),
            LogOutcome::Queued
        );
    }
}
