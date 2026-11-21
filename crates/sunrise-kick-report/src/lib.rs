//! Kick reports (`kickReport`, docs/extras.md): after a server kicks or bans the player, the
//! Disconnected screen says Sunrise may be the cause, asks them to report it, and offers a button
//! that copies a report with the game's recent log, so the run can be recreated and compared with
//! vanilla. Nothing is sent: the player pastes the report where they choose.
//!
//! A Sunrise crate (docs/layers.md): the game tells it how the window's connection ended through
//! the platform's hooks ([`HOOKS`]: `connecting`, `left`, `ended`) and asks it whether the screen
//! offers a report (`report`); the engine draws the screen, writes the report and copies it. The
//! official executable (`sunrise-official`) registers it with its switch: its line of
//! `sunrise-extras.txt` ([`SETTING`]) and its row in Extras' Multiplayer group ([`SECTION`]).
#![forbid(unsafe_code)]

use std::sync::atomic::{AtomicBool, Ordering};

use sunrise_platform::hooks::Hooks;
use sunrise_platform::hooks::connection::{By, Ended, Left, Report, Server};
use sunrise_platform::pages::{Act, Label, Page, Place, Row};
use sunrise_platform::settings::Store;

/// The crate's localized texts, registered only with the crate.
pub static LANG: &sunrise_platform::lang::Files = include!(concat!(env!("OUT_DIR"), "/locales.rs"));

/// Its hooks, for `sunrise_platform::hooks::register`.
pub static HOOKS: KickReport = KickReport;

pub struct KickReport;

/// The switch.
static ON: AtomicBool = AtomicBool::new(false);
/// The last connection ended in a kick or ban the screen has not asked about yet.
static KICKED: AtomicBool = AtomicBool::new(false);

/// What the screen says and its button.
const OFFER: Report = Report::new(
    &[
        "sunrise.kick_report.about",
        "sunrise.kick_report.ask",
        "sunrise.kick_report.private",
    ],
    "sunrise.kick_report.copy",
    "sunrise.kick_report.copied",
);

impl Hooks for KickReport {
    fn connecting(&self, _server: &Server) {
        KICKED.store(false, Ordering::Relaxed);
    }

    fn left(&self, _how: Left) {
        KICKED.store(false, Ordering::Relaxed);
    }

    fn ended(&self, end: &Ended<'_>) {
        let kicked = ON.load(Ordering::Relaxed) && turned_away(end);
        KICKED.store(kicked, Ordering::Relaxed);
    }

    /// The report carries the recent packets while the switch is on.
    fn trace_packets(&self) -> bool {
        ON.load(Ordering::Relaxed)
    }

    /// Once for each kick: the screen asks right after the end.
    fn report(&self) -> Option<Report> {
        KICKED.swap(false, Ordering::Relaxed).then_some(OFFER)
    }
}

/// Reasons a server gives that say nothing about what the client did: it stopped or is full,
/// the player isn't on its list, the versions differ, the account logged in elsewhere or isn't
/// verified. Prefixes of translation keys, so a key's variants share the verdict.
const NOT_THE_CLIENT: [&str; 10] = [
    "multiplayer.disconnect.server_shutdown",
    "multiplayer.disconnect.server_full",
    "multiplayer.disconnect.not_whitelisted",
    "multiplayer.disconnect.incompatible",
    "multiplayer.disconnect.outdated_client",
    "multiplayer.disconnect.outdated_server",
    "multiplayer.disconnect.duplicate_login",
    "multiplayer.disconnect.unverified_username",
    "multiplayer.disconnect.authservers_down",
    "disconnect.loginFailedInfo",
];

/// The server turned the player away: a kick or a ban, vanilla's or a plugin's (an anticheat's
/// reasons are its own text, with no key), not a lost connection or a refusal that says nothing
/// about the client.
fn turned_away(end: &Ended<'_>) -> bool {
    end.by == By::Server
        && !end
            .key
            .is_some_and(|key| NOT_THE_CLIENT.iter().any(|known| key.starts_with(known)))
}

/// The switch's line in `sunrise-extras.txt`.
const KEY: &str = "kickReport";
/// The switch's row, and what it says on hover.
const LABEL: &str = "sunrise.extras.kick_report";
const ABOUT: &str = "sunrise.extras.kick_report.tooltip";

fn on() -> bool {
    ON.load(Ordering::Relaxed)
}

fn set(on: bool) {
    ON.store(on, Ordering::Relaxed);
}

/// The switch's line of `sunrise-extras.txt`, for `sunrise_platform::settings::register`.
pub static SETTING: Setting = Setting;

pub struct Setting;

impl Store for Setting {
    fn file(&self) -> &'static str {
        "sunrise-extras.txt"
    }
    fn owns(&self, key: &str) -> bool {
        key == KEY
    }
    fn load(&self, _key: &str, value: &str) {
        match value {
            "true" => set(true),
            "false" => set(false),
            _ => {}
        }
    }
    fn save(&self, line: &mut dyn FnMut(&str, &str)) {
        line(KEY, if on() { "true" } else { "false" });
    }
    fn is_default(&self) -> bool {
        !on()
    }
}

/// The switch's row in the Extras page's Multiplayer group, for
/// `sunrise_platform::pages::register`.
pub static SECTION: Section = Section;

pub struct Section;

impl Page for Section {
    fn id(&self) -> &'static str {
        "sunrise-kick-report.extras"
    }
    fn title(&self) -> &'static str {
        "sunrise.extras.header.multiplayer"
    }
    fn place(&self) -> Place {
        Place::Extras
    }
    fn rows(&self, rows: &mut Vec<Row>) {
        rows.push(Row::toggle(LABEL, on()));
    }
    fn act(&self, row: usize, act: Act) {
        if row == 0 && matches!(act, Act::Press | Act::PressBack) {
            set(!on());
        }
    }
    fn tooltip(&self, _row: usize) -> Option<Label> {
        Some(Label::Key(ABOUT))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The only test that touches the statics: off, a kick offers nothing; on, an anticheat's
    /// kick offers the report once, and a lost connection or a stopping server offers none.
    #[test]
    fn the_switch_and_the_reason_decide_the_offer() {
        assert!(SETTING.is_default());
        let server = Server::new("example.org", "Tester", false);
        let kick = Ended::new("Unfair advantage", None, By::Server);
        HOOKS.connecting(&server);
        HOOKS.ended(&kick);
        assert_eq!(HOOKS.report(), None);
        assert!(!HOOKS.trace_packets());
        SECTION.act(0, Act::Press);
        assert!(HOOKS.trace_packets());
        let mut lines = Vec::new();
        SETTING.save(&mut |key, value| lines.push(format!("{key}:{value}")));
        assert_eq!(lines, ["kickReport:true"]);
        HOOKS.connecting(&server);
        HOOKS.ended(&kick);
        assert_eq!(HOOKS.report(), Some(OFFER));
        assert_eq!(HOOKS.report(), None, "asked once for each kick");
        let ban = Ended::new(
            "You are banned",
            Some("multiplayer.disconnect.banned"),
            By::Server,
        );
        HOOKS.connecting(&server);
        HOOKS.ended(&ban);
        assert_eq!(HOOKS.report(), Some(OFFER));
        for not_a_kick in [
            Ended::new("Timed out", Some("disconnect.timeout"), By::Connection),
            Ended::new(
                "Server closed",
                Some("multiplayer.disconnect.server_shutdown"),
                By::Server,
            ),
            Ended::new("Declined", None, By::Client),
        ] {
            HOOKS.connecting(&server);
            HOOKS.ended(&not_a_kick);
            assert_eq!(HOOKS.report(), None, "{}", not_a_kick.reason);
        }
        HOOKS.connecting(&server);
        HOOKS.ended(&kick);
        HOOKS.left(Left::Disconnected);
        assert_eq!(HOOKS.report(), None);
        SETTING.load(KEY, "false");
        assert!(SETTING.is_default());
    }
}
