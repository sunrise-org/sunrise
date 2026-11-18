//! The extras' switches and sort key: their `sunrise-extras.txt` lines and Extras rows.
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use sunrise_platform::hooks;
use sunrise_platform::pages::{Act, Label, Page, Place, Row};
use sunrise_platform::settings::Store;

const MOUSE_TWEAKS: &str = "mouseTweaks";
const WHEEL: &str = "mouseTweaksWheel";
const SORT: &str = "sort";
const SORT_KEY: &str = "sortKey";
/// An unbound key, as Key Binds names it.
const UNBOUND: &str = "key.keyboard.unknown";

static MOUSE_TWEAKS_ON: AtomicBool = AtomicBool::new(false);
static WHEEL_ON: AtomicBool = AtomicBool::new(true);
static SORT_ON: AtomicBool = AtomicBool::new(false);
/// The sort key's name; `None` is unbound.
static SORT_KEY_NAME: Mutex<Option<String>> = Mutex::new(None);

/// Only the first local player owns these process-wide settings.
fn owns() -> bool {
    hooks::seat() == 0
}

/// Mouse Tweaks-style controls are on.
pub(crate) fn mouse_tweaks() -> bool {
    MOUSE_TWEAKS_ON.load(Ordering::Relaxed)
}
/// The wheel moves items, with the controls on.
pub(crate) fn wheel() -> bool {
    mouse_tweaks() && WHEEL_ON.load(Ordering::Relaxed)
}
/// The sort buttons are shown.
pub(crate) fn sort() -> bool {
    SORT_ON.load(Ordering::Relaxed)
}
/// The sort key's name (`key.keyboard.r`, `key.mouse.middle`), `key.keyboard.unknown` unbound.
pub(crate) fn sort_key() -> String {
    let key = SORT_KEY_NAME.lock().unwrap_or_else(|e| e.into_inner());
    key.clone().unwrap_or_else(|| UNBOUND.to_owned())
}
fn set_sort_key(name: &str) {
    let name = (name != UNBOUND).then(|| name.to_owned());
    *SORT_KEY_NAME.lock().unwrap_or_else(|e| e.into_inner()) = name;
}
fn text(on: bool) -> &'static str {
    if on { "true" } else { "false" }
}

/// The lines of `sunrise-extras.txt`, for `sunrise_platform::settings::register`.
pub static SETTING: Setting = Setting;

pub struct Setting;

impl Store for Setting {
    fn file(&self) -> &'static str {
        "sunrise-extras.txt"
    }
    fn owns(&self, key: &str) -> bool {
        matches!(key, MOUSE_TWEAKS | WHEEL | SORT | SORT_KEY)
    }
    fn load(&self, key: &str, value: &str) {
        if !owns() {
            return;
        }
        let switch = match key {
            MOUSE_TWEAKS => &MOUSE_TWEAKS_ON,
            WHEEL => &WHEEL_ON,
            SORT => &SORT_ON,
            _ => {
                if value.starts_with("key.") {
                    set_sort_key(value);
                }
                return;
            }
        };
        match value {
            "true" => switch.store(true, Ordering::Relaxed),
            "false" => switch.store(false, Ordering::Relaxed),
            _ => {}
        }
    }
    fn save(&self, line: &mut dyn FnMut(&str, &str)) {
        if !owns() {
            return;
        }
        line(MOUSE_TWEAKS, text(mouse_tweaks()));
        line(WHEEL, text(WHEEL_ON.load(Ordering::Relaxed)));
        line(SORT, text(sort()));
        line(SORT_KEY, &sort_key());
    }
    fn is_default(&self) -> bool {
        !owns()
            || (!mouse_tweaks()
                && WHEEL_ON.load(Ordering::Relaxed)
                && !sort()
                && sort_key() == UNBOUND)
    }
}

/// The rows in the Extras page's Inventory group, for `sunrise_platform::pages::register`.
pub static SECTION: Section = Section;

pub struct Section;

/// The rows, in order: their captions and what they say on hover.
const ROWS: [(&str, &str); 4] = [
    (
        "sunrise.extras.mouse_tweaks",
        "sunrise.extras.mouse_tweaks.tooltip",
    ),
    (
        "sunrise.extras.mouse_tweaks_wheel",
        "sunrise.extras.mouse_tweaks_wheel.tooltip",
    ),
    ("sunrise.extras.sort", "sunrise.extras.sort.tooltip"),
    ("sunrise.extras.sort_key", "sunrise.extras.sort_key.tooltip"),
];

impl Page for Section {
    fn id(&self) -> &'static str {
        "sunrise-inventory.extras"
    }
    fn title(&self) -> &'static str {
        "sunrise.extras.header.inventory"
    }
    fn place(&self) -> Place {
        Place::Extras
    }
    fn rows(&self, rows: &mut Vec<Row>) {
        if !owns() {
            return;
        }
        let tweaks = mouse_tweaks();
        rows.push(Row::toggle(ROWS[0].0, tweaks));
        // A setting of a switch that is off does nothing, so it is greyed out.
        rows.push(Row::toggle(ROWS[1].0, WHEEL_ON.load(Ordering::Relaxed)).active(tweaks));
        rows.push(Row::toggle(ROWS[2].0, sort()));
        rows.push(Row::key(ROWS[3].0, sort_key()).active(sort()));
    }
    fn act(&self, row: usize, act: Act) {
        if !owns() {
            return;
        }
        let flip = |switch: &AtomicBool| {
            switch.fetch_xor(true, Ordering::Relaxed);
        };
        match (row, act) {
            (0, Act::Press | Act::PressBack) => flip(&MOUSE_TWEAKS_ON),
            (1, Act::Press | Act::PressBack) if mouse_tweaks() => flip(&WHEEL_ON),
            (2, Act::Press | Act::PressBack) => flip(&SORT_ON),
            (3, Act::Bind(name)) if sort() => set_sort_key(&name),
            _ => {}
        }
    }
    fn tooltip(&self, row: usize) -> Option<Label> {
        ROWS.get(row).map(|(_, about)| Label::Key(about))
    }
}

#[cfg(test)]
pub(crate) fn reset() {
    MOUSE_TWEAKS_ON.store(false, Ordering::Relaxed);
    WHEEL_ON.store(true, Ordering::Relaxed);
    SORT_ON.store(false, Ordering::Relaxed);
    set_sort_key(UNBOUND);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_lines_load_save_and_grey_as_the_extras_did() {
        reset();
        assert!(SETTING.is_default());
        for (key, value) in [
            ("mouseTweaks", "true"),
            ("mouseTweaksWheel", "false"),
            ("sort", "true"),
            ("sortKey", "key.keyboard.r"),
            ("sortKey", "nonsense"),
        ] {
            assert!(SETTING.owns(key));
            SETTING.load(key, value);
        }
        let mut lines = Vec::new();
        SETTING.save(&mut |k, v| lines.push(format!("{k}:{v}")));
        assert_eq!(
            lines,
            [
                "mouseTweaks:true",
                "mouseTweaksWheel:false",
                "sort:true",
                "sortKey:key.keyboard.r",
            ]
        );
        // With the controls off, the wheel row is greyed and does nothing.
        SECTION.act(0, Act::Press);
        let mut rows = Vec::new();
        SECTION.rows(&mut rows);
        assert_eq!(
            rows[1],
            Row::toggle("sunrise.extras.mouse_tweaks_wheel", false).active(false)
        );
        SECTION.act(1, Act::Press);
        assert!(!WHEEL_ON.load(Ordering::Relaxed));
        // The key binds while sorting is on, and Escape unbinds it.
        SECTION.act(3, Act::Bind("key.mouse.middle".into()));
        assert_eq!(sort_key(), "key.mouse.middle");
        SECTION.act(3, Act::Bind(UNBOUND.into()));
        assert_eq!(sort_key(), UNBOUND);
        reset();
    }
}
