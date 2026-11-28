//! Native compiler output choices and guidance for declared internals.
use std::process::Command;

const TIMINGS_ENV: &str = "SUNRISE_NATIVE_CARGO_TIMINGS";
const VERBOSE_ENV: &str = "SUNRISE_NATIVE_BUILD_VERBOSE";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Timings {
    #[default]
    Off,
    Html,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CompilerStderr {
    #[default]
    LogOnly,
    Mirror,
}

/// Diagnostics never change the compile inputs or the native-build JSON contract.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BuildDiagnostics {
    pub timings: Timings,
    pub stderr: CompilerStderr,
}

impl BuildDiagnostics {
    /// Read machine-tool switches, or add them to the CLI's explicit choices.
    pub fn with_env(mut self) -> Self {
        if std::env::var_os(TIMINGS_ENV).is_some() {
            self.timings = Timings::Html;
        }
        if std::env::var_os(VERBOSE_ENV).is_some() {
            self.stderr = CompilerStderr::Mirror;
        }
        self
    }

    /// Send both choices through the pinned tool's environment, not its plan or arguments.
    pub fn configure(self, command: &mut Command) {
        for (name, enabled) in [
            (TIMINGS_ENV, self.timings == Timings::Html),
            (VERBOSE_ENV, self.stderr == CompilerStderr::Mirror),
        ] {
            if enabled {
                command.env(name, "1");
            } else {
                command.env_remove(name);
            }
        }
    }
}
pub fn with_internals_hint(error: String) -> String {
    if error.contains("internals")
        && ["E0432", "E0433", "E0603"]
            .iter()
            .any(|code| error.contains(code))
    {
        format!(
            "{error}\n{}",
            crate::host::tr(
                "Game internals must be declared: run cargo sunrise-mod internals sync in the mod's folder, then build again."
            )
        )
    } else {
        error
    }
}
