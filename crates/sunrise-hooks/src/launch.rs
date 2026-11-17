//! Switches an executable above the engine takes on the game's command line (docs/layers.md).
use std::ffi::OsStr;

use crate::{TooLate, registry::Registry};

static FLAGS: Registry<str> = Registry::new();

/// Declares the switch `name` (`--sunrise-extras`) and says whether this process has it.
pub fn declare(name: &'static str) -> Result<bool, TooLate> {
    FLAGS.register(name)?;
    Ok(game_args(std::env::args_os().skip(1)).any(|arg| arg == name))
}

/// Everything after `--dedicated` is the dedicated server's, not the game's.
fn game_args<A: AsRef<OsStr>>(args: impl IntoIterator<Item = A>) -> impl Iterator<Item = A> {
    args.into_iter()
        .take_while(|arg| arg.as_ref() != "--dedicated")
}

/// The engine's side. Only the engine calls these.
pub mod engine {
    use super::*;

    /// Every declared switch, for `--launch-api`; seals the list.
    pub fn all() -> &'static [&'static str] {
        FLAGS.all()
    }

    /// `args` without the declared switches, for the engine's own option parser.
    pub fn without_declared<A: AsRef<OsStr>>(args: impl IntoIterator<Item = A>) -> Vec<A> {
        let mut game = true;
        args.into_iter()
            .filter(|arg| {
                game &= arg.as_ref() != "--dedicated";
                !game || !all().iter().any(|&name| arg.as_ref() == name)
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_switch_counts_only_before_the_dedicated_servers_arguments() {
        let has = |args: &[&str]| game_args(args).any(|arg| *arg == "--extras");
        assert!(has(&["--resources", "r", "--extras"]));
        assert!(!has(&["--resources", "r"]));
        assert!(!has(&["--dedicated", "--extras"]));
    }

    #[test]
    fn the_engine_drops_declared_switches_and_keeps_the_rest() {
        assert_eq!(declare("--extras"), Ok(false));
        let args = ["--extras", "--world", "W", "--dedicated", "--extras"];
        assert_eq!(
            engine::without_declared(args),
            ["--world", "W", "--dedicated", "--extras"]
        );
        assert_eq!(engine::all(), ["--extras"]);
        assert_eq!(declare("--extras"), Err(TooLate));
    }
}
