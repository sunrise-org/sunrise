//! The compatibility check before a supervisor starts a dedicated game.

use serde::{Deserialize, Deserializer, Serialize};
use std::{
    path::Path,
    process::{Command, Stdio},
};
use sunrise_platform::server_files::dedicated_launch;

/// An inclusive range of dedicated launch contracts, also carried in `server-tool.json`.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct ContractRange {
    pub min: u32,
    pub max: u32,
}

impl ContractRange {
    pub fn contains(self, contract: u32) -> bool {
        (self.min..=self.max).contains(&contract)
    }
}

/// Dedicated launch contracts this manager can run.
pub const SUPPORTED_CONTRACTS: ContractRange = ContractRange { min: 1, max: 2 };

#[derive(Debug, Deserialize)]
pub struct GameContract {
    pub sunrise: String,
    // Only an absent field is the legacy contract; null or an invalid object is a bad reply.
    #[serde(default, deserialize_with = "read_dedicated")]
    dedicated: Option<Dedicated>,
}

#[derive(Debug, Deserialize)]
struct Dedicated {
    contract: u32,
    options: Vec<String>,
    control: u32,
    log: u32,
}

fn read_dedicated<'de, D: Deserializer<'de>>(input: D) -> Result<Option<Dedicated>, D::Error> {
    Dedicated::deserialize(input).map(Some)
}

impl GameContract {
    /// Builds that predate the `dedicated` object speak contract 1.
    pub fn contract(&self) -> u32 {
        self.dedicated.as_ref().map_or(1, |d| d.contract)
    }

    fn check(&self) -> Result<(), String> {
        let contract = self.contract();
        let supported = SUPPORTED_CONTRACTS.contains(contract);
        let Some(d) = &self.dedicated else {
            return if supported {
                Ok(())
            } else {
                Err(format!(
                    "game build {} speaks dedicated contract {contract}; this sunrise-server supports contracts {}–{}. \
                     Run `sunrise-server self-update` or `sunrise-server update` to get compatible versions.",
                    self.sunrise, SUPPORTED_CONTRACTS.min, SUPPORTED_CONTRACTS.max,
                ))
            };
        };
        let missing: Vec<_> = dedicated_launch::OPTIONS
            .iter()
            .copied()
            .filter(|option| !d.options.iter().any(|offered| offered == option))
            .collect();
        if supported
            && d.control == dedicated_launch::CONTROL_PROTOCOL
            && d.log == dedicated_launch::LOG_FORMAT
            && missing.is_empty()
        {
            return Ok(());
        }
        let missing = if missing.is_empty() {
            String::new()
        } else {
            format!("; missing {}", missing.join(", "))
        };
        Err(format!(
            "game build {} offers dedicated contract {}, control protocol {}, log format {}, options [{}]{missing}; \
             this sunrise-server supports contracts {}–{}, control protocol {}, log format {}, and needs [{}]. \
             Run `sunrise-server self-update` or `sunrise-server update` to get compatible versions.",
            self.sunrise,
            d.contract,
            d.control,
            d.log,
            d.options.join(", "),
            SUPPORTED_CONTRACTS.min,
            SUPPORTED_CONTRACTS.max,
            dedicated_launch::CONTROL_PROTOCOL,
            dedicated_launch::LOG_FORMAT,
            dedicated_launch::OPTIONS.join(", "),
        ))
    }
}

/// Ask the selected engine what it offers, without starting the server or requiring game files.
pub fn probe_contract(program: &Path) -> Result<GameContract, String> {
    let mut command = Command::new(program);
    // The probe only returns captured output; don't flash a Windows console for it.
    #[cfg(windows)]
    std::os::windows::process::CommandExt::creation_flags(
        &mut command,
        windows_sys::Win32::System::Threading::CREATE_NO_WINDOW,
    );
    let output = command
        .arg("--launch-api")
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("the game ({}) can't start: {e}", program.display()))?;
    if !output.status.success() {
        return Err(start_failure(
            program,
            output.status.code(),
            &String::from_utf8_lossy(&output.stderr),
        ));
    }
    serde_json::from_slice(&output.stdout).map_err(|e| {
        format!(
            "the game ({}) returned an invalid --launch-api reply: {e}",
            program.display()
        )
    })
}

/// Refuse an incompatible game before launching it in dedicated mode.
pub fn check_contract(program: &Path) -> Result<(), String> {
    probe_contract(program)?.check()
}

/// What to tell the owner when the game program exits at once.
fn start_failure(program: &Path, code: Option<i32>, stderr: &str) -> String {
    let reason = stderr
        .lines()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("")
        .trim();
    let mut text = format!(
        "the game ({}) can't start{}{}",
        program.display(),
        code.map_or(String::new(), |c| format!(" (exit status {c})")),
        if reason.is_empty() {
            String::new()
        } else {
            format!(": {reason}")
        }
    );
    if reason.contains("error while loading shared libraries")
        && (reason.contains("libasound") || reason.contains("libudev"))
    {
        text.push_str(
            "\nThe full game needs ALSA's and udev's libraries (libasound.so.2, libudev.so.1). Install the \
             server build, which needs neither (packaging/server/install.sh picks it on Linux), or the \
             libraries:\n  Debian, Ubuntu: sudo apt install libasound2 libudev1   (Ubuntu 24.04 and later: \
             libasound2t64)\n  Fedora, RHEL: sudo dnf install alsa-lib systemd-libs\n  Arch: sudo pacman -S \
             alsa-lib systemd-libs\n  NixOS: enable programs.nix-ld with alsa-lib and udev in its libraries",
        );
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

    #[test]
    fn a_missing_library_says_what_to_install() {
        let program = Path::new("/opt/sunrise/sunrise-client");
        let text = start_failure(
            program,
            Some(127),
            "/opt/sunrise/sunrise-client: error while loading shared libraries: libasound.so.2: cannot open \
             shared object file: No such file or directory\n",
        );
        assert!(text.starts_with(
            "the game (/opt/sunrise/sunrise-client) can't start (exit status 127): /opt/sunrise/sunrise-client: \
             error while loading shared libraries: libasound.so.2"
        ));
        assert!(text.contains("sudo apt install libasound2 libudev1"));
        // Another library: no ALSA advice (the server build, sunrise-engine, links neither).
        let engine = start_failure(
            Path::new("/opt/sunrise/sunrise-engine"),
            Some(127),
            "/opt/sunrise/sunrise-engine: error while loading shared libraries: libgcc_s.so.1: cannot open \
             shared object file: No such file or directory\n",
        );
        assert!(!engine.contains("libasound"), "{engine}");
        let other = start_failure(program, Some(1), "something else\n");
        assert_eq!(
            other,
            "the game (/opt/sunrise/sunrise-client) can't start (exit status 1): something else"
        );
    }

    fn current() -> Value {
        json!({
            "sunrise": "26.3-0.2.0",
            "launch_api": "0.1.2",
            "dedicated": {
                "contract": 2,
                "options": ["--resources", "--control-socket", "--transfers", "--dedicated", "--nogui", "--extra"],
                "control": 1,
                "log": 1,
            },
        })
    }

    #[test]
    fn a_reply_from_before_the_dedicated_object_is_contract_one() {
        let game: GameContract =
            serde_json::from_str(r#"{"sunrise":"26.3-0.1.0","launch_api":"0.1.1"}"#).unwrap();
        assert_eq!(game.contract(), 1);
        assert!(SUPPORTED_CONTRACTS.contains(game.contract()));
        assert_eq!(game.check(), Ok(()));
    }

    #[test]
    fn an_advertised_compatible_contract_is_accepted_with_extra_options() {
        let game: GameContract = serde_json::from_value(current()).unwrap();
        assert_eq!(game.contract(), 2);
        assert_eq!(game.check(), Ok(()));
    }

    #[test]
    fn unknown_contract_or_protocol_and_missing_option_are_refused() {
        for (key, value, detail) in [
            ("contract", json!(3), "contract 3"),
            ("control", json!(2), "control protocol 2"),
            ("log", json!(2), "log format 2"),
            (
                "options",
                json!([
                    "--resources",
                    "--control-socket",
                    "--transfers",
                    "--dedicated"
                ]),
                "missing --nogui",
            ),
        ] {
            let mut reply = current();
            reply["dedicated"][key] = value;
            let game: GameContract = serde_json::from_value(reply).unwrap();
            let error = game.check().unwrap_err();
            for expected in [
                "26.3-0.2.0",
                detail,
                "supports contracts 1–2",
                "sunrise-server self-update",
                "sunrise-server update",
            ] {
                assert!(
                    error.contains(expected),
                    "expected {expected:?} in {error:?}"
                );
            }
        }
    }

    #[test]
    fn a_bad_dedicated_field_does_not_fall_back_to_contract_one() {
        for bad in [Value::Null, json!({"contract": 2})] {
            let mut reply = current();
            reply["dedicated"] = bad;
            assert!(serde_json::from_value::<GameContract>(reply).is_err());
        }
    }
}
