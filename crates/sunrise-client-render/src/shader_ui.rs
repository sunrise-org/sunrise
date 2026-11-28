//! Sunrise's shader settings screen, on the settings shell (`settings_shell`): the options of the
//! picked shader pack (Shader Options). Its pages are the pack's own screens (a `shaders/` pack's
//! `screen.NAME`s), read from the pack each time the rows are built; the pack list's rows and what
//! picking does are here too, for Sunrise Settings' Shaders page (`sunrise_ui`), which lists them;
//! the shell draws and drives them, searches every screen, collapses the screens into one row
//! when the page list has no room for them, and has Undo. A pack's options apply when Apply
//! is pressed or the screen closes, as the pack loads again with them.
use crate::{
    lang,
    options_ui::Event,
    settings::Settings,
    settings_shell::{self as shell, Group, Output, Row, Sheet, Value},
    text::Text,
};
use std::collections::BTreeMap;

/// A page of the screen: screen `.0` of the picked pack (`screens`, 0 its main one).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Tab {
    Screen(u16),
}

/// A row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Item {
    /// `s.shader_packs[.0]`: picks it, or turns it off when it is the pick.
    Pack(u16),
    /// Opens `shaderpacks/`.
    Folder,
    /// The picked pack's Iris compatibility: Auto, On or Off (`graphics::IrisCompat`).
    IrisCompat,
    /// Option `.0` of the picked pack: the `.0`th of its options by name.
    Option(u16),
    /// Opens screen `.0`.
    Link(u16),
    /// The next of the pack's profiles.
    Profile,
    /// Every option of the pack back to its default.
    Reset,
}

const TITLE: &str = "Shaders";
const GENERAL: &str = "General";
/// The groups: the packs and Sunrise's own settings beside them (on Sunrise Settings' Shaders
/// page), a pack's options.
pub(crate) const INSTALLED: &str = "Installed";
pub(crate) const BUILT_IN: &str = "Built-in settings";
const PACK_OPTIONS: &str = "Pack settings";
/// The line over the picked pack's screens in the page list.
const PACK_HEADING: &str = "%s settings";
/// What the strip says of a pack's own screens: they are the pack's, not Sunrise's.
pub(crate) const PACK_SETTINGS: &str =
    "Settings made by the pack's author: they change only this pack.";
const NO_PACKS: &str = "No shader packs yet: open the pack folder and put some in it.";
const PICK_A_PACK: &str = "Pick a shader pack. Picking the one in use turns shaders off.";
/// Why the picked pack's settings can't be opened.
const NO_PICK: &str = "sunrise.settings.shader_options.no_pack";
const NO_SCREENS: &str = "sunrise.settings.shader_options.none";
const PENDING: &str = "Changed options apply with Apply, or when you leave.";
/// The picked pack's Iris compatibility switch, its description and its values.
const IRIS_COMPAT: &str = "sunrise.shaders.iris_compat";
const IRIS_COMPAT_ABOUT: &str = "sunrise.shaders.iris_compat.about";
const IRIS_COMPAT_AUTO: &str = "sunrise.shaders.iris_compat.auto";
const IRIS_COMPAT_ON: &str = "sunrise.shaders.iris_compat.on";
const IRIS_COMPAT_OFF: &str = "sunrise.shaders.iris_compat.off";
/// The row of a pack that runs with Iris compatibility (`%s`: its name), and what its
/// description adds.
const IRIS_COMPAT_ROW: &str = "sunrise.shaders.iris_compat.row";
const IRIS_COMPAT_RUNS: &str = "sunrise.shaders.iris_compat.runs";
/// Pack descriptions run longer than Sunrise's own: the strip has room for a line more.
pub(crate) const NOTE_LINES: usize = 3;
/// Screens this many links from the main one, or fewer, are in the page list.
const LISTED_DEPTH: u8 = 1;

/// One of the picked pack's screens.
struct PackScreen {
    title: String,
    /// What the strip says while nothing on it is pointed at.
    about: String,
    items: Vec<Item>,
    /// How many links away from the main screen: the page list holds the first two levels.
    depth: u8,
}

/// The picked pack's options, when they are read (`s.shader_options`).
fn classic(s: &Settings) -> Option<&crate::shader_options::ShaderOptions> {
    s.shader_options
        .as_ref()
        .filter(|o| o.pack == s.sunrise.shader_pack)
}

/// Whether the picked pack `file` runs with Iris compatibility, as its options were last read
/// with the switch (`ShaderOptions::iris_compat`).
fn runs_iris_compat(s: &Settings, file: &str) -> bool {
    file == s.sunrise.shader_pack && classic(s).is_some_and(|o| o.iris_compat)
}

/// The `.0`th option name of the pack.
fn option_name(o: &crate::shader_options::ShaderOptions, i: u16) -> Option<&str> {
    o.options.keys().nth(usize::from(i)).map(String::as_str)
}

fn plain(t: Text) -> String {
    t.strip_legacy().plain()
}

/// The picked pack's screens, its main one first, then in the order their links are met.
fn screens(s: &Settings) -> Vec<PackScreen> {
    classic(s).map_or_else(Vec::new, classic_screens)
}

fn classic_screens(o: &crate::shader_options::ShaderOptions) -> Vec<PackScreen> {
    use sunrise_shaders::ScreenItem;
    let keys: Vec<&String> = o.options.keys().collect();
    let mut names = vec![(String::new(), 0u8)];
    let mut out = Vec::new();
    let mut at = 0;
    while let Some((name, depth)) = names.get(at).cloned() {
        at += 1;
        let mut items = Vec::new();
        for item in o.screens.screen(&name).map_or(&[][..], |(items, _)| items) {
            match item {
                ScreenItem::Option(n) => {
                    if let Some(k) = keys.iter().position(|k| *k == n) {
                        items.push(Item::Option(k as u16));
                    }
                }
                ScreenItem::Screen(n) if o.screens.screen(n).is_some() => {
                    let k = match names.iter().position(|(m, _)| m == n) {
                        Some(k) => k,
                        None => {
                            names.push((n.clone(), depth.saturating_add(1)));
                            names.len() - 1
                        }
                    };
                    items.push(Item::Link(k as u16));
                }
                ScreenItem::Profile if !o.screens.profiles.is_empty() => items.push(Item::Profile),
                _ => {}
            }
        }
        let comment = |key: &str| o.comment(key).map(plain);
        let (title, about) = if name.is_empty() {
            items.push(Item::Reset);
            (lang::get(GENERAL), lang::get(PACK_SETTINGS))
        } else {
            let title = plain(o.screen_label(&name));
            let title = title.strip_suffix("...").unwrap_or(&title).to_owned();
            let about = comment(&format!("screen.{name}")).unwrap_or_default();
            (title, about)
        };
        out.push(PackScreen {
            title,
            about,
            items,
            depth,
        });
    }
    out
}

/// A screen's name in the page list and over its rows.
fn screen_name(i: u16, screens: &[PackScreen]) -> String {
    let screen = screens.get(usize::from(i));
    screen.map_or_else(String::new, |s| s.title.clone())
}

/// A row as its name and its value; `None` makes it a button across the row.
fn parts(item: Item, s: &Settings, screens: &[PackScreen]) -> (String, Option<String>) {
    match item {
        Item::Pack(i) => {
            let pack = s.shader_packs.get(usize::from(i));
            let name = match pack {
                Some(p) if runs_iris_compat(s, &p.file) => lang::tr(IRIS_COMPAT_ROW, &[&p.file]),
                Some(p) => p.file.clone(),
                None => String::new(),
            };
            (name, Some(String::new()))
        }
        Item::IrisCompat => {
            let value = match s.sunrise.iris_compat_of(&s.sunrise.shader_pack) {
                crate::graphics::IrisCompat::Auto => IRIS_COMPAT_AUTO,
                crate::graphics::IrisCompat::On => IRIS_COMPAT_ON,
                crate::graphics::IrisCompat::Off => IRIS_COMPAT_OFF,
            };
            (lang::get(IRIS_COMPAT), Some(lang::get(value)))
        }
        Item::Folder => (lang::get("pack.openFolder"), None),
        Item::Link(i) => (format!("{}...", screen_name(i, screens)), None),
        Item::Reset => (lang::get("Reset all options"), None),
        Item::Profile => match classic(s) {
            Some(o) => shell::name_and_value(&plain(o.profile_label())),
            None => (String::new(), None),
        },
        Item::Option(i) => match classic(s).and_then(|o| Some((o, option_name(o, i)?))) {
            Some((o, name)) => (plain(o.option_name(name)), Some(plain(o.value_text(name)))),
            None => (String::new(), None),
        },
    }
}

/// A tick box's state, for a pack (picked) or an on/off option.
fn switch(item: Item, s: &Settings) -> Option<bool> {
    match item {
        Item::Pack(i) => Some(
            s.shader_packs
                .get(usize::from(i))
                .is_some_and(|p| p.file == s.sunrise.shader_pack),
        ),
        Item::Option(i) => {
            let o = classic(s)?;
            let name = option_name(o, i)?;
            o.options[name]
                .switch
                .then(|| o.value(name) == Some("true"))
        }
        _ => None,
    }
}

/// A slider's position, for an option the pack shows as one.
fn slider(item: Item, s: &Settings) -> Option<f32> {
    let Item::Option(i) = item else { return None };
    let o = classic(s)?;
    o.slider_position(option_name(o, i)?)
}

/// A slider's stops, which Left and Right step between: the option's values.
fn slider_steps(item: Item, s: &Settings) -> Option<u32> {
    let Item::Option(i) = item else { return None };
    let o = classic(s)?;
    let stops = o.options[option_name(o, i)?].values.len().saturating_sub(1);
    (stops >= 1).then_some(stops as u32)
}

/// What a row does, or says of itself, for the strip and the search.
fn description(item: Item, s: &Settings, screens: &[PackScreen]) -> String {
    match item {
        Item::Pack(i) => {
            let Some(pack) = s.shader_packs.get(usize::from(i)) else {
                return String::new();
            };
            let kind = lang::get("An OptiFine or Iris shader pack.");
            let what = lang::get(if pack.file == s.sunrise.shader_pack {
                "In use: press to turn shaders off."
            } else {
                "Press to use it."
            });
            let compat = if runs_iris_compat(s, &pack.file) {
                format!(" {}", lang::get(IRIS_COMPAT_RUNS))
            } else {
                String::new()
            };
            format!("{kind} {what}{compat} ({})", pack.file)
        }
        Item::IrisCompat => lang::get(IRIS_COMPAT_ABOUT),
        Item::Folder => lang::get("Put shader pack .zip files or folders in it."),
        Item::Reset => lang::get("Puts every option of this pack back to its default."),
        Item::Profile => classic(s)
            .and_then(|o| o.comment("profile"))
            .map(plain)
            .unwrap_or_else(|| lang::get("Sets many options at once, as the pack groups them.")),
        Item::Link(i) => screens
            .get(usize::from(i))
            .map(|s| s.about.clone())
            .unwrap_or_default(),
        Item::Option(i) => classic(s)
            .and_then(|o| o.comment(&format!("option.{}", option_name(o, i)?)))
            .map(plain)
            .unwrap_or_default(),
    }
}

/// A row as the shell draws it: a tick box for a pack and an on/off option, a slider for an
/// option the pack shows as one, a value between `<` `>` for the other options and the profile,
/// a button across the row for the folder, a link and Reset.
fn row(item: Item, s: &Settings, screens: &[PackScreen]) -> Row<Item> {
    let (name, text) = parts(item, s, screens);
    let value = match (item, switch(item, s), slider(item, s), text) {
        (_, Some(on), ..) => Value::Switch(on),
        (_, None, Some(position), Some(text)) => Value::Slider {
            position: f64::from(position),
            steps: slider_steps(item, s),
            text,
        },
        (Item::Option(_) | Item::Profile | Item::IrisCompat, None, None, Some(text)) => {
            Value::Choice(text)
        }
        _ => Value::Press,
    };
    Row::new(item, name, value).about(description(item, s, screens))
}

/// The installed packs, the picked one followed by its Iris compatibility switch, then the
/// folder's row: what Sunrise Settings' Shaders page lists (`sunrise_ui`).
pub(crate) fn pack_rows(s: &Settings) -> Vec<Row<Item>> {
    let mut items = Vec::new();
    for (i, pack) in s.shader_packs.iter().enumerate() {
        items.push(Item::Pack(i as u16));
        if pack.file == s.sunrise.shader_pack {
            items.push(Item::IrisCompat);
        }
    }
    items.push(Item::Folder);
    items.into_iter().map(|item| row(item, s, &[])).collect()
}

/// Why the picked pack's settings can't be opened, if they can't: no pack picked, or the pick
/// has no screens (its options weren't read: the pack was refused).
pub(crate) fn settings_blocked(s: &Settings) -> Option<&'static str> {
    let pick = &s.sunrise.shader_pack;
    if pick.is_empty() || !s.shader_packs.iter().any(|p| &p.file == pick) {
        Some(NO_PICK)
    } else if screens(s).is_empty() {
        Some(NO_SCREENS)
    } else {
        None
    }
}

/// What the pack list says while nothing on it is pointed at: how to get packs, or how to pick.
pub(crate) fn packs_about(s: &Settings) -> String {
    lang::get(if s.shader_packs.is_empty() {
        NO_PACKS
    } else {
        PICK_A_PACK
    })
}

/// The picked pack's name, over its screens in the page list.
fn pack_name(s: &Settings) -> &str {
    (s.sunrise.shader_pack)
        .strip_suffix(".zip")
        .unwrap_or(&s.sunrise.shader_pack)
}

/// The shader screen. Its rows read the settings; what it keeps is for Undo.
#[derive(Default)]
pub(crate) struct ShaderScreen {
    /// The values this screen saved to a `shaders/` pack's file (picking another pack, Apply),
    /// by pack: Undo puts back the ones the screen opened with over them.
    written: BTreeMap<String, BTreeMap<String, String>>,
}

impl ShaderScreen {
    /// The picked `shaders/` pack's options are saved: what Undo has to put back over.
    fn saving(&mut self, s: &Settings) {
        if let Some(o) = s.shader_options.as_ref() {
            self.written.insert(o.pack.clone(), o.chosen.clone());
        }
    }
    /// Undo of the pick alone: the pick and the remembered pick go back as the screen opened
    /// with them, the pick with its options as they were, and say so.
    pub(crate) fn undo_pick(&mut self, s: &mut Settings, before: &Settings) -> Vec<Output> {
        let mut outputs = Vec::new();
        if s.sunrise.shader_pack != before.sunrise.shader_pack {
            // The pick goes back with its options as they were; what this screen saved over
            // them is saved again when it closes. The other pack's changes are dropped.
            s.sunrise.shader_pack = before.sunrise.shader_pack.clone();
            s.shader_options = before.shader_options.clone();
            if let Some(o) = s.shader_options.as_mut()
                && let Some(written) = self.written.get(&o.pack)
            {
                o.saved_as(written.clone());
            }
            outputs.push(Output::Event(Event::ShaderPack));
            if s.shader_options.is_none() {
                outputs.push(Output::Event(Event::OpenShaderOptions));
            }
        }
        if s.sunrise.iris_compat != before.sunrise.iris_compat {
            // The packs' Iris compatibility goes back too, the pick's read again with it.
            s.sunrise.iris_compat = before.sunrise.iris_compat.clone();
            outputs.push(Output::Event(Event::ShaderPack));
            outputs.push(Output::Event(Event::OpenShaderOptions));
        }
        if s.extras.last_shader_pack != before.extras.last_shader_pack {
            // The toggle key (or a removed pack) changed the remembered pick while the screen
            // was open: it goes back too. Rendering needs no event for what isn't picked.
            s.extras.last_shader_pack = before.extras.last_shader_pack.clone();
        }
        outputs
    }
    /// Whether the pick, the remembered pick or a pack's Iris compatibility differs from
    /// `before`.
    pub(crate) fn pick_differs(s: &Settings, before: &Settings) -> bool {
        let pick = s.sunrise.shader_pack != before.sunrise.shader_pack;
        let compat = s.sunrise.iris_compat != before.sunrise.iris_compat;
        pick || compat || s.extras.last_shader_pack != before.extras.last_shader_pack
    }
}

impl shell::Screen for ShaderScreen {
    type Page = Tab;
    type Row = Item;
    fn title(&self) -> &'static str {
        TITLE
    }
    fn note_lines(&self) -> usize {
        NOTE_LINES
    }
    /// The rows read the packs, the pick, its options and its Iris compatibility.
    fn stamp(&self, s: &Settings) -> u64 {
        let packs: Vec<_> = s.shader_packs.iter().map(|p| &p.file).collect();
        let options =
            (s.shader_options.as_ref()).map(|o| (&o.pack, &o.chosen, o.changed, o.iris_compat));
        let h = &s.sunrise;
        let compat = h.iris_compat_of(&h.shader_pack);
        shell::stamp_of(&(&h.shader_pack, packs, options, compat))
    }
    /// The picked pack's screens under its name: those the main screen links to in the page
    /// list, the ones farther in opened by their links.
    fn sheets(&self, s: &Settings) -> Vec<Sheet<Tab, Item>> {
        let screens = screens(s);
        let pending = self.pending(s);
        let mut sheets = Vec::new();
        for (i, screen) in screens.iter().enumerate() {
            let about = if pending {
                lang::get(PENDING)
            } else {
                screen.about.clone()
            };
            let rows = (screen.items.iter())
                .map(|&item| row(item, s, &screens))
                .collect();
            let groups = vec![Group {
                name: PACK_OPTIONS,
                rows,
            }];
            let mut sheet = Sheet::new(Tab::Screen(i as u16), screen.title.clone(), about, groups);
            sheet.listed = screen.depth <= LISTED_DEPTH;
            sheet.heading = (i == 0).then(|| lang::tr(PACK_HEADING, &[&pack_name(s)]));
            sheets.push(sheet);
        }
        sheets
    }
    fn link(&self, item: Item) -> Option<Tab> {
        match item {
            Item::Link(i) => Some(Tab::Screen(i)),
            _ => None,
        }
    }
    /// A pack is picked (or turned off) and saved, the last pack's changed options first; an
    /// option steps (back with Shift); the folder opens.
    fn press(&mut self, s: &mut Settings, item: Item, back: bool) -> Vec<Output> {
        let changed = vec![Output::Event(Event::Changed)];
        let pack = s.sunrise.shader_pack.clone();
        match item {
            Item::Pack(i) => {
                let Some(p) = s.shader_packs.get(usize::from(i)) else {
                    return Vec::new();
                };
                let key = p.file.clone();
                if s.sunrise.shader_pack == key {
                    s.set_shader_pick("");
                } else {
                    s.set_shader_pick(&key);
                };
                let mut outputs = Vec::new();
                // The last pack's changed options are kept before the next one's are read.
                if s.shader_options.as_ref().is_some_and(|o| o.changed) {
                    self.saving(s);
                    outputs.push(Output::Event(Event::ShaderOptions));
                }
                outputs.push(Output::Event(Event::ShaderPack));
                outputs.push(Output::Event(Event::OpenShaderOptions));
                outputs
            }
            Item::Folder => vec![Output::Event(Event::OpenShaderPackFolder)],
            // The pick's switch steps, is saved, and the pick is read again with it, its
            // changed options kept first; a load started with the old mode is never installed
            // (the game's `drop_stale_identity_build`).
            Item::IrisCompat if !pack.is_empty() => {
                let mode = s.sunrise.iris_compat_of(&pack).step(back);
                s.sunrise.set_iris_compat(&pack, mode);
                let mut outputs = Vec::new();
                if s.shader_options.as_ref().is_some_and(|o| o.changed) {
                    self.saving(s);
                    outputs.push(Output::Event(Event::ShaderOptions));
                }
                outputs.push(Output::Event(Event::ShaderPack));
                outputs.push(Output::Event(Event::OpenShaderOptions));
                outputs
            }
            Item::IrisCompat => Vec::new(),
            // The shell follows a link (`link`).
            Item::Link(_) => Vec::new(),
            Item::Profile => match s.shader_options.as_mut().filter(|o| o.pack == pack) {
                Some(o) => {
                    o.next_profile();
                    changed
                }
                None => Vec::new(),
            },
            Item::Reset => match s.shader_options.as_mut().filter(|o| o.pack == pack) {
                Some(o) => {
                    o.reset();
                    changed
                }
                None => Vec::new(),
            },
            Item::Option(i) => {
                let options = s.shader_options.as_mut().filter(|o| o.pack == pack);
                match options.and_then(|o| Some((option_name(o, i)?.to_owned(), o))) {
                    Some((name, o)) => {
                        o.cycle(&name, back);
                        changed
                    }
                    None => Vec::new(),
                }
            }
        }
    }
    /// A slider moves an option to the value nearest it.
    fn slide(&mut self, s: &mut Settings, item: Item, position: f64) {
        let Item::Option(i) = item else {
            return;
        };
        let pack = s.sunrise.shader_pack.clone();
        if let Some(o) = s.shader_options.as_mut().filter(|o| o.pack == pack)
            && let Some(name) = option_name(o, i).map(str::to_owned)
        {
            o.set_position(&name, position as f32);
        }
    }
    /// With the "Scroll wheel changes settings" extra, the wheel steps an option between `<` `>`
    /// and the profile (up goes back); the other rows leave it to scroll the list.
    fn wheel_step(&mut self, s: &mut Settings, item: Item, back: bool) -> Option<Vec<Output>> {
        let steps = match item {
            Item::Profile | Item::IrisCompat => true,
            Item::Option(_) => switch(item, s).is_none() && slider(item, s).is_none(),
            _ => false,
        };
        if !steps {
            return None;
        }
        let mut outputs = vec![Output::Event(Event::ButtonClick)];
        outputs.extend(self.press(s, item, back));
        Some(outputs)
    }
    fn undoes(&self) -> bool {
        true
    }
    /// The pick, the remembered pick and the options of the pack picked when the screen opened
    /// (saved when it closes, or Apply), as the screen opened with them. Another pack keeps what
    /// was saved for it.
    fn undo(&mut self, s: &mut Settings, before: &Settings) -> Vec<Output> {
        // The same pick: its options go back as they were.
        if s.sunrise.shader_pack == before.sunrise.shader_pack
            && let Some(o) = s.shader_options.as_mut()
            && let Some(was) = before.shader_options.as_ref().filter(|w| w.pack == o.pack)
        {
            o.restore(was.chosen.clone());
        }
        self.undo_pick(s, before)
    }
    /// Undo shows exactly while the pick, the remembered pick or the pack's options differ from
    /// what the screen opened with: an option stepped back to its value isn't a change.
    fn differs(&self, s: &Settings, before: &Settings) -> Option<bool> {
        let options = match (classic(s), before.shader_options.as_ref()) {
            (Some(o), Some(was)) => o.pack == was.pack && o.chosen != was.chosen,
            _ => false,
        };
        Some(Self::pick_differs(s, before) || options)
    }
    /// The pack's options wait for Apply until the pack loads again with them.
    fn pending(&self, s: &Settings) -> bool {
        classic(s).is_some_and(|o| o.changed)
    }
    /// The options are saved, and the pack loads again with them.
    fn apply(&mut self, s: &mut Settings) -> Vec<Output> {
        self.saving(s);
        vec![
            Output::Event(Event::ShaderOptions),
            Output::Event(Event::OpenShaderOptions),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hud::{HudAtlas, HudBatch};
    use crate::settings_shell::{Hosted, Part, Screen as _, Shell, Target};
    use crate::shader_options::ShaderOptions;
    use crate::shader_pack_list::Entry;
    use sunrise_shaders::{Lang, Macros, PackOption, Properties, Screens};
    use winit::keyboard::KeyCode as K;

    const SIZE: (f32, f32) = (854.0, 480.0);
    const CLICK: Output = Output::Event(Event::ButtonClick);
    const CHANGED: Output = Output::Event(Event::Changed);

    fn atlas() -> HudAtlas {
        HudAtlas::build(&sunrise_assets::resource_source::ResourceSource::directory(
            "/nonexistent/sunrise-test-assets".into(),
        ))
        .unwrap()
    }

    fn entry(file: &str) -> Entry {
        Entry { file: file.into() }
    }

    fn option(name: &str, values: &[&str], switch: bool) -> (String, PackOption) {
        let o = PackOption {
            name: name.into(),
            default: values[0].into(),
            values: values.iter().map(|v| (*v).into()).collect(),
            switch,
            tested: true,
        };
        (name.to_owned(), o)
    }

    /// `A.zip` picked, with `options` laid out by `properties`.
    fn classic_pack(options: BTreeMap<String, PackOption>, properties: &str) -> Settings {
        let mut s = Settings {
            shader_packs: vec![entry("A.zip"), entry("B.zip")],
            ..Settings::default()
        };
        s.sunrise.shader_pack = "A.zip".into();
        let screens = Screens::parse(
            &Properties::parse(properties, &Macros::standard()),
            &options,
        );
        s.shader_options = Some(ShaderOptions::new(
            "A.zip".into(),
            options,
            screens,
            Lang::default(),
            BTreeMap::new(),
        ));
        s
    }

    /// Two packs, the first picked: a `shaders/` pack with a main screen, a Lighting screen and
    /// a Shadows screen inside it. Options by name: BLOOM 0, QUALITY 1, SHADOW_RES 2, SUN 3.
    fn settings() -> Settings {
        let options = BTreeMap::from([
            option("BLOOM", &["false", "true"], true),
            option("QUALITY", &["1", "2", "3"], false),
            option("SHADOW_RES", &["1024", "2048", "4096"], false),
            option("SUN", &["1", "2"], false),
        ]);
        classic_pack(
            options,
            "screen = <profile> [LIGHTING] BLOOM QUALITY\nscreen.LIGHTING = SUN [SHADOWS]\n\
             screen.SHADOWS = SHADOW_RES\nsliders = SHADOW_RES\n\
             profile.LOW = SHADOW_RES=1024\nprofile.HIGH = SHADOW_RES=4096\n",
        )
    }

    /// The picked pack with `n` screens, each linked from its main one: more than the page list
    /// of a short window holds.
    fn many_screens(n: usize) -> Settings {
        let names: Vec<String> = (0..n).map(|i| format!("S{i}")).collect();
        let options = names
            .iter()
            .map(|name| option(&format!("O_{name}"), &["false", "true"], true))
            .collect();
        let links: Vec<String> = names.iter().map(|n| format!("[{n}]")).collect();
        let mut text = format!("screen = {}\n", links.join(" "));
        for name in &names {
            text += &format!("screen.{name} = O_{name}\n");
        }
        classic_pack(options, &text)
    }

    fn shell_at(tab: Tab) -> Shell<ShaderScreen> {
        let mut shell = Shell::<ShaderScreen>::default();
        shell.show(tab);
        shell
    }

    fn press(shell: &mut Shell<ShaderScreen>, s: &mut Settings, item: Item) -> Vec<Output> {
        let at = shell.point_at(s, SIZE, item);
        shell.click(s, at, SIZE, false, false, false)
    }

    #[test]
    fn a_packs_screens_are_pages_its_main_one_first_under_the_packs_name() {
        let s = settings();
        let sheets = ShaderScreen::default().sheets(&s);
        let pages: Vec<(Tab, &str, bool)> = (sheets.iter())
            .map(|p| (p.id, p.name.as_str(), p.listed))
            .collect();
        // The page list holds the main screen and the ones it links to; Shadows opens from
        // Lighting.
        assert_eq!(
            pages,
            [
                (Tab::Screen(0), "General", true),
                (Tab::Screen(1), "LIGHTING", true),
                (Tab::Screen(2), "SHADOWS", false),
            ]
        );
        assert_eq!(sheets[0].heading.as_deref(), Some("A settings"));
        assert!(
            sheets
                .iter()
                .filter(|p| p.id != Tab::Screen(0))
                .all(|p| p.heading.is_none())
        );
        let rows: Vec<Item> = sheets[0].groups[0].rows.iter().map(|r| r.id).collect();
        assert_eq!(
            rows,
            [
                Item::Profile,
                Item::Link(1),
                Item::Option(0),
                Item::Option(1),
                Item::Reset
            ]
        );
        // A tick box, a value between arrows, a slider in its pack's steps, and links as buttons.
        let value = |sheet: usize, item| {
            let rows = &sheets[sheet].groups[0].rows;
            rows.iter().find(|r| r.id == item).unwrap().value.clone()
        };
        assert_eq!(value(0, Item::Option(0)), Value::Switch(false));
        assert_eq!(value(0, Item::Option(1)), Value::Choice("1".into()));
        assert_eq!(value(0, Item::Link(1)), Value::Press);
        assert!(matches!(
            value(2, Item::Option(2)),
            Value::Slider { steps: Some(2), .. }
        ));
        // Another pack's options are not this one's: no page.
        let mut other = s.clone();
        other.sunrise.shader_pack = "B.zip".into();
        assert_eq!(ShaderScreen::default().sheets(&other).len(), 0);
    }

    #[test]
    fn a_short_page_list_collapses_a_packs_screens_and_leaves_none_out() {
        let s = many_screens(12);
        let mut shell = shell_at(Tab::Screen(4));
        // 427x480 GUI px: tall enough for the pack's line and its 13 screens.
        let mut batch = HudBatch::scratch([854, 960]);
        shell.draw_test_frame(
            &s,
            &mut batch,
            &atlas(),
            (0.0, 0.0),
            false,
            None,
            &mut |_, _| {},
        );
        assert_eq!(shell.page_places().len(), 13);
        let mut batch = HudBatch::scratch([854, 480]);
        shell.draw_test_frame(
            &s,
            &mut batch,
            &atlas(),
            (0.0, 0.0),
            false,
            None,
            &mut |_, _| {},
        );
        let parts: Vec<(Tab, Part)> = shell.page_places().iter().map(|p| (p.0, p.2)).collect();
        assert_eq!(
            parts,
            [
                (Tab::Screen(3), Part::Prev),
                (Tab::Screen(4), Part::Shown),
                (Tab::Screen(5), Part::Next)
            ]
        );
    }

    /// The picked pack's row is followed by its Iris compatibility switch, Auto at first; a
    /// press steps it for that pack alone, saves it and reads the pick again (its changed
    /// options saved first), and Undo puts it back. The pack's row says it runs with Iris
    /// compatibility while its options were read with it on, and not otherwise.
    #[test]
    fn the_picked_pack_has_an_iris_compatibility_switch_beside_its_row() {
        let english = lang::Language::load(&[], "en_us");
        let ((), missed) = lang::scoped(english, || {
            use crate::graphics::IrisCompat;
            let mut s = settings();
            let before = s.clone();
            let rows = pack_rows(&s);
            let ids: Vec<Item> = rows.iter().map(|r| r.id).collect();
            assert_eq!(
                ids,
                [Item::Pack(0), Item::IrisCompat, Item::Pack(1), Item::Folder]
            );
            assert_eq!(rows[1].name, "Iris compatibility");
            assert_eq!(rows[1].value, Value::Choice("Auto".into()));
            assert_eq!(
                (rows[0].name.as_str(), rows[2].name.as_str()),
                ("A.zip", "B.zip")
            );
            let mut screen = ShaderScreen::default();
            let reads = vec![
                Output::Event(Event::ShaderPack),
                Output::Event(Event::OpenShaderOptions),
            ];
            assert_eq!(screen.press(&mut s, Item::IrisCompat, false), reads);
            assert_eq!(s.sunrise.iris_compat_of("A.zip"), IrisCompat::On);
            assert_eq!(s.sunrise.iris_compat_of("B.zip"), IrisCompat::Auto);
            assert!(ShaderScreen::pick_differs(&s, &before));
            s.shader_options.as_mut().unwrap().cycle("BLOOM", false);
            let saved_first = [vec![Output::Event(Event::ShaderOptions)], reads.clone()].concat();
            assert_eq!(screen.press(&mut s, Item::IrisCompat, false), saved_first);
            assert_eq!(s.sunrise.iris_compat_of("A.zip"), IrisCompat::Off);
            assert_eq!(pack_rows(&s)[1].value, Value::Choice("Off".into()));
            // The row follows what the pick was read with.
            assert_eq!(pack_rows(&s)[0].name, "A.zip");
            s.shader_options.as_mut().unwrap().iris_compat = true;
            let rows = pack_rows(&s);
            assert_eq!(rows[0].name, "A.zip (Iris compatibility)");
            assert!(rows[0].about.contains("It runs with Iris compatibility."));
            assert_eq!(rows[2].name, "B.zip");
            assert_eq!(screen.undo_pick(&mut s, &before), reads);
            assert_eq!(s.sunrise.iris_compat_of("A.zip"), IrisCompat::Auto);
            // No pick, no switch.
            s.set_shader_pick("");
            let ids: Vec<Item> = pack_rows(&s).iter().map(|r| r.id).collect();
            assert_eq!(ids, [Item::Pack(0), Item::Pack(1), Item::Folder]);
            assert!(screen.press(&mut s, Item::IrisCompat, false).is_empty());
        });
        let mut missed = missed;
        missed.retain(|key| !VANILLA_KEYS.contains(&key.as_str()));
        assert_eq!(missed, Vec::<String>::new());
    }

    #[test]
    fn picking_a_pack_and_picking_it_again_turns_shaders_off() {
        let key = "B.zip";
        let mut s = settings();
        // Sunrise Settings' Shaders page lists the packs and hands a press to this screen.
        let mut screen = ShaderScreen::default();
        let picks = vec![
            Output::Event(Event::ShaderPack),
            Output::Event(Event::OpenShaderOptions),
        ];
        assert_eq!(screen.press(&mut s, Item::Pack(1), false), picks);
        assert_eq!(s.sunrise.shader_pack, key);
        assert_eq!(s.extras.last_shader_pack, key);
        assert_eq!(switch(Item::Pack(1), &s), Some(true));
        screen.press(&mut s, Item::Pack(1), false);
        assert_eq!(s.sunrise.shader_pack, "");
        assert_eq!(s.extras.last_shader_pack, key);
        assert_eq!(switch(Item::Pack(0), &s), Some(false));
    }

    #[test]
    fn undo_puts_back_the_remembered_pick_alone() {
        let mut s = settings();
        s.set_shader_pick("A.zip");
        s.set_shader_pick("");
        let before = s.clone();
        // The key cleared the memory for a removed pack while the screen was open: the pick
        // stays empty, and Undo still offers the remembered pick back, without a reload.
        s.extras.last_shader_pack.clear();
        let mut screen = ShaderScreen::default();
        assert_eq!(screen.differs(&s, &before), Some(true));
        assert!(screen.undo(&mut s, &before).is_empty());
        assert_eq!(
            (
                s.sunrise.shader_pack.as_str(),
                s.extras.last_shader_pack.as_str()
            ),
            ("", "A.zip")
        );
    }

    #[test]
    fn options_wait_for_apply_and_links_go_in_and_escape_back_out() {
        let mut s = settings();
        let mut shell = shell_at(Tab::Screen(0));
        let is_pending = |shell: &mut Shell<ShaderScreen>, s: &Settings| {
            shell.foot_at(s, SIZE).iter().any(|f| f.0 == Target::Apply)
        };
        assert!(!is_pending(&mut shell, &s));
        assert_eq!(press(&mut shell, &mut s, Item::Option(0)), [CLICK, CHANGED]);
        assert_eq!(switch(Item::Option(0), &s), Some(true));
        assert!(is_pending(&mut shell, &s));
        // Stepped back, nothing waits.
        press(&mut shell, &mut s, Item::Option(0));
        assert!(!is_pending(&mut shell, &s));
        press(&mut shell, &mut s, Item::Option(0));
        // Apply saves them and reads the pack again.
        let apply = shell
            .foot_at(&s, SIZE)
            .into_iter()
            .find(|f| f.0 == Target::Apply);
        let [x, y, w, h] = apply.unwrap().1;
        let applied = shell.click(
            &mut s,
            (x + w / 2.0, y + h / 2.0),
            SIZE,
            false,
            false,
            false,
        );
        let reload = [Event::ShaderOptions, Event::OpenShaderOptions].map(Output::Event);
        assert_eq!(applied, [&[CLICK][..], &reload].concat());
        // Into Lighting, then Shadows, which isn't in the page list: Escape backs out of each.
        press(&mut shell, &mut s, Item::Link(1));
        press(&mut shell, &mut s, Item::Link(2));
        assert_eq!(shell.open_page(), Some(Tab::Screen(2)));
        let mut batch = HudBatch::scratch([1708, 960]);
        shell.draw_test_frame(
            &s,
            &mut batch,
            &atlas(),
            (0.0, 0.0),
            false,
            None,
            &mut |_, _| {},
        );
        assert_eq!(shell.lit_page(), Some(Tab::Screen(1)));
        assert!(shell.key(&mut s, K::Escape, None, false, SIZE).is_empty());
        assert_eq!(shell.open_page(), Some(Tab::Screen(1)));
        shell.key(&mut s, K::Escape, None, false, SIZE);
        let done = shell.key(&mut s, K::Escape, None, false, SIZE);
        assert_eq!(
            done,
            [Output::Done],
            "the main screen's Escape closes the screen"
        );
        // Reset puts the option back.
        press(&mut shell, &mut s, Item::Reset);
        assert_eq!(switch(Item::Option(0), &s), Some(false));
    }

    #[test]
    fn left_and_right_step_an_option_and_a_shift_click_steps_it_back() {
        let mut s = settings();
        let mut shell = shell_at(Tab::Screen(0));
        let value = |s: &Settings| classic(s).unwrap().value("QUALITY").map(str::to_owned);
        shell.set_focus(Some(Target::Row(Item::Option(1))));
        shell.key(&mut s, K::ArrowRight, None, false, SIZE);
        assert_eq!(value(&s).as_deref(), Some("2"));
        shell.key(&mut s, K::ArrowLeft, None, false, SIZE);
        shell.key(&mut s, K::ArrowLeft, None, false, SIZE);
        assert_eq!(value(&s).as_deref(), Some("3"));
        let at = shell.point_at(&s, SIZE, Item::Option(1));
        shell.click(&mut s, at, SIZE, false, true, false);
        assert_eq!(
            value(&s).as_deref(),
            Some("3"),
            "the right button presses nothing"
        );
        shell.click(&mut s, at, SIZE, false, false, true);
        assert_eq!(value(&s).as_deref(), Some("2"));
        assert_eq!(shell.focused_row(), Some(Item::Option(1)));
    }

    #[test]
    fn the_search_finds_an_option_on_a_screen_inside_another() {
        let s = settings();
        let mut shell = shell_at(Tab::Screen(0));
        shell.set_search("shadow_res");
        assert_eq!(shell.listed(&s, SIZE), [(Tab::Screen(2), Item::Option(2))]);
        assert_eq!(slider(Item::Option(2), &s), Some(0.0));
    }

    #[test]
    fn undo_puts_back_the_pick_and_the_options() {
        let mut s = settings();
        let mut shell = shell_at(Tab::Screen(0));
        let undo_shows = |shell: &mut Shell<ShaderScreen>, s: &Settings| {
            shell.foot_at(s, SIZE).iter().any(|f| f.0 == Target::Undo)
        };
        press(&mut shell, &mut s, Item::Option(0));
        assert!(undo_shows(&mut shell, &s));
        // Stepped back to how it opened, there is nothing to undo.
        press(&mut shell, &mut s, Item::Option(0));
        assert!(!undo_shows(&mut shell, &s));
        // A changed option, then another pack picked.
        press(&mut shell, &mut s, Item::Option(0));
        // Another pack picked on Sunrise Settings' Shaders page, by this screen's press.
        let picked = shell.screen.press(&mut s, Item::Pack(1), false);
        assert!(
            picked.contains(&Output::Event(Event::ShaderOptions)),
            "{picked:?}"
        );
        // The game reads the next pack's options: this one has none.
        s.shader_options = None;
        let undo = shell
            .foot_at(&s, SIZE)
            .into_iter()
            .find(|f| f.0 == Target::Undo);
        let [x, y, w, h] = undo.unwrap().1;
        let outputs = shell.click(
            &mut s,
            (x + w / 2.0, y + h / 2.0),
            SIZE,
            false,
            false,
            false,
        );
        // The pick is back and saved; the pack's options are back as it opened, to be saved
        // over what picking away saved.
        assert_eq!(s.sunrise.shader_pack, "A.zip");
        assert!(
            outputs.contains(&Output::Event(Event::ShaderPack)),
            "{outputs:?}"
        );
        assert_eq!(switch(Item::Option(0), &s), Some(false));
        assert!(classic(&s).unwrap().changed);
        assert!(!undo_shows(&mut shell, &s));
    }

    #[test]
    fn every_text_is_in_the_language_table() {
        let mut s = settings();
        let (_, mut missed) = lang::scoped(lang::Language::load(&[], "de_de"), || {
            let screen = ShaderScreen::default();
            screen.sheets(&s);
            s.shader_packs.clear();
            screen.sheets(&s);
            for text in [TITLE, PENDING, "Apply", "Undo", "Search options"] {
                lang::get(text);
            }
        });
        // The folder's name and how a value is joined to its name are vanilla's.
        missed.retain(|key| !VANILLA_KEYS.contains(&key.as_str()));
        assert_eq!(missed, Vec::<String>::new());
    }
    const VANILLA_KEYS: [&str; 3] = ["pack.openFolder", "options.generic_value", "options.off"];
}
