//! The resource pack screen's Add button: a [`Source`] that searches Modrinth and installs a pack
//! into the packs folder ([`SOURCE`]), its `packBrowser` line of `sunrise-extras.txt`
//! ([`SETTING`]) and its switch in Extras' Menus group ([`SECTION`]). Each player has their own
//! switch and session, by `sunrise_platform::hooks::seat()`. Off, the game never contacts Modrinth.
#![forbid(unsafe_code)]

mod transport;

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::{LazyLock, Mutex, MutexGuard};
use sunrise_platform::hooks::seat;
use sunrise_platform::packs::{Labels, Message, Source};
use sunrise_platform::pages::{Act, Label, Page, Place, Row};
use sunrise_platform::settings::Store;
use transport::Browser;

const KEY: &str = "packBrowser";
const LABEL: &str = "sunrise.extras.pack_browser";
const ABOUT: &str = "sunrise.extras.pack_browser.tooltip";

/// One player's switch and open browse session.
#[derive(Default)]
struct Seat {
    on: bool,
    browser: Option<Browser>,
}

static SEATS: Mutex<BTreeMap<usize, Seat>> = Mutex::new(BTreeMap::new());

fn seats() -> MutexGuard<'static, BTreeMap<usize, Seat>> {
    SEATS.lock().unwrap_or_else(|e| e.into_inner())
}

fn on() -> bool {
    seats().get(&seat()).is_some_and(|s| s.on)
}

fn set_on(on: bool) {
    seats().entry(seat()).or_default().on = on;
}

/// Runs `f` on this player's open session, if any.
fn with_browser<R>(f: impl FnOnce(&mut Browser) -> R) -> Option<R> {
    seats().get_mut(&seat())?.browser.as_mut().map(f)
}

/// Modrinth, for `sunrise_platform::packs::register`.
pub static SOURCE: Modrinth = Modrinth;

pub struct Modrinth;

static FILTER_KEYS: LazyLock<Vec<&'static str>> =
    LazyLock::new(|| transport::FILTERS.iter().map(|f| f.0).collect());

impl Source for Modrinth {
    fn enabled(&self) -> bool {
        on()
    }
    fn filters(&self) -> &'static [&'static str] {
        &FILTER_KEYS
    }
    fn labels(&self) -> Labels {
        Labels {
            add: "sunrise.pack.addInfo",
            search_hint: "sunrise.pack.searchHint",
            working: "sunrise.pack.addWorking",
        }
    }
    fn open(&self) {
        seats().entry(seat()).or_default().browser = Some(Browser::new());
    }
    fn search(&self, query: &str, chosen: &[usize]) {
        with_browser(|b| b.search(query, chosen));
    }
    fn fetch_icons(&self) {
        with_browser(|b| b.fetch_icons());
    }
    fn download(&self, hit: usize, dir: &Path, game_version: &str) {
        with_browser(|b| b.download(hit, dir, game_version));
    }
    fn poll(&self) -> Option<Message> {
        with_browser(Browser::poll).flatten()
    }
    fn close(&self, seat: usize) {
        if let Some(player) = seats().get_mut(&seat) {
            player.browser = None;
        }
    }
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
            "true" => set_on(true),
            "false" => set_on(false),
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

/// The switch's row in the Extras page's Menus group, for `sunrise_platform::pages::register`.
pub static SECTION: Section = Section;

pub struct Section;

impl Page for Section {
    fn id(&self) -> &'static str {
        "sunrise-pack-browser.extras"
    }
    fn title(&self) -> &'static str {
        "sunrise.extras.header.menus"
    }
    fn place(&self) -> Place {
        Place::Extras
    }
    fn rows(&self, rows: &mut Vec<Row>) {
        rows.push(Row::Toggle {
            label: LABEL,
            on: on(),
        });
    }
    fn act(&self, row: usize, act: Act) {
        if row == 0 && matches!(act, Act::Press | Act::PressBack) {
            set_on(!on());
        }
    }
    fn tooltip(&self, _row: usize) -> Option<Label> {
        Some(Label::Key(ABOUT))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_source_is_on_only_for_the_players_who_turned_it_on() {
        set_on(false);
        assert!(!SOURCE.enabled());
        SECTION.act(0, Act::Press);
        assert!(SOURCE.enabled());
        let mut saved = Vec::new();
        SETTING.save(&mut |k, v| saved.push(format!("{k}:{v}")));
        assert_eq!(saved, ["packBrowser:true"]);
        SETTING.load(KEY, "false");
        assert!(!SOURCE.enabled() && SETTING.is_default());
    }

    #[test]
    fn a_session_is_one_seats_and_reopening_drops_a_busy_one() {
        let guest = seat() + 7;
        SOURCE.open();
        with_browser(|b| b.busy = true);
        SOURCE.open();
        assert_eq!(with_browser(|b| b.busy), Some(false));
        seats().entry(guest).or_default().browser = Some(Browser::new());
        SOURCE.close(guest);
        assert!(seats()[&guest].browser.is_none());
        assert!(seats()[&seat()].browser.is_some());
        SOURCE.close(seat());
    }
}
