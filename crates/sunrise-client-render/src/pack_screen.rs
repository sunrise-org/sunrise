//! The resource pack screen (`PackSelectionScreen` with its two
//! `TransferableSelectionList`s, search box and buttons) and the
//! `ConfirmScreen` an incompatible pack opens, as vanilla's state machine:
//! clicks, the mouse wheel and typing change the selection model, the lists'
//! entries, scroll, selection and focus as the vanilla widgets do, and
//! [`PackScreen::draw`] gives the screen's GUI calls. The game shows it;
//! tools/test_pack_screen_parity.py compares it with vanilla.
use crate::{
    font::Patch,
    hud::{HudAtlas, PACK_ICON_SIZE, PACK_ICON_SLOTS},
    packs::SelectionModel,
    screen_draw::{self as d, Draw, Measure, TextField},
    text::{Style, Text},
};
use std::{collections::HashMap, sync::Arc};

pub use sunrise_level::pack_meta::{Compat, PackInfo, PackSource as Source};

/// A `pack.png` as the pack screen draws it: RGBA, stretched to a
/// `PACK_ICON_SIZE` square by nearest texels, as vanilla's icon texture is
/// sampled into its 32x32 square. None when it does not decode, where
/// `loadPackIcon` falls back to `unknown_pack`.
pub fn load_icon(png: &[u8]) -> Option<Arc<[u8]>> {
    scale_icon(crate::hud::decode_bytes(png).ok()?)
}
fn scale_icon((w, h, rgba): (u32, u32, Vec<u8>)) -> Option<Arc<[u8]>> {
    if w == 0 || h == 0 {
        return None;
    }
    let size = u64::from(PACK_ICON_SIZE);
    let mut out = Vec::with_capacity((size * size * 4) as usize);
    for y in 0..size {
        let sy = ((2 * y + 1) * u64::from(h) / (2 * size)) as usize;
        for x in 0..size {
            let sx = ((2 * x + 1) * u64::from(w) / (2 * size)) as usize;
            let at = (sy * w as usize + sx) * 4;
            out.extend_from_slice(&rgba[at..at + 4]);
        }
    }
    Some(out.into())
}
/// Points the icon blits of `draws` at the atlas's pack icon slots and writes the `icons` they
/// name into them, one slot per blit (vanilla registers a texture per icon). Icons past the last
/// slot keep `unknown_pack`.
pub fn bind_icon_slots(
    icons: &HashMap<String, Arc<[u8]>>,
    draws: &mut [Draw],
    atlas: &HudAtlas,
    patches: &mut Vec<Patch>,
) {
    let mut slots = 0..PACK_ICON_SLOTS;
    for draw in draws {
        let Draw::Blit { texture, .. } = draw else {
            continue;
        };
        let Some(icon) = icons.get(texture.as_str()) else {
            continue;
        };
        let Some(slot) = slots.next() else {
            return;
        };
        let sprite = format!("sunrise/pack-icon{slot}");
        let Some(rect) = atlas.sprites.get(&sprite) else {
            return;
        };
        patches.push(Patch {
            x: (rect.u0 * atlas.width as f32).round() as u32,
            y: (rect.v0 * atlas.height as f32).round() as u32,
            width: PACK_ICON_SIZE,
            height: PACK_ICON_SIZE,
            rgba: icon.to_vec(),
        });
        *texture = sprite;
    }
}
/// `PackSelectionScreen.loadPackIcon`'s texture for a pack with an icon.
pub fn icon_texture(id: &str) -> String {
    let sanitized: String = id
        .to_lowercase()
        .chars()
        .map(|c| {
            if c.is_ascii_lowercase() || c.is_ascii_digit() || "_-./".contains(c) {
                c
            } else {
                '_'
            }
        })
        .collect();
    let bytes: Vec<u8> = id.encode_utf16().flat_map(u16::to_le_bytes).collect();
    let hash: String = crate::skins::sha1(&bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    format!("pack/{sanitized}/{hash}/icon")
}

#[derive(Clone, Debug, PartialEq)]
enum Row {
    Header,
    Pack(String),
}
#[derive(Clone, Debug)]
struct Entry {
    row: Row,
    y: i32,
    height: i32,
}
/// A `TransferableSelectionList`.
#[derive(Clone, Debug)]
struct List {
    selected_side: bool,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    scroll: f64,
    entries: Vec<Entry>,
    /// `getSelected`, an index into `entries`.
    selected: Option<usize>,
    /// The list has a focused child (possibly an entry it no longer holds).
    focused: bool,
    /// The pointer's last y while the scrollbar is held (`scrolling`).
    held: Option<f64>,
}
impl List {
    fn new(selected_side: bool, height: i32) -> Self {
        Self {
            selected_side,
            x: 0,
            y: 33,
            width: 200,
            height,
            scroll: 0.0,
            entries: Vec::new(),
            selected: None,
            focused: false,
            held: None,
        }
    }
    fn bottom(&self) -> i32 {
        self.y + self.height
    }
    fn row_left(&self) -> i32 {
        self.x + self.width / 2 - self.row_width() / 2
    }
    fn row_width(&self) -> i32 {
        self.width - 4
    }
    fn content_height(&self) -> i32 {
        self.entries.iter().map(|e| e.height).sum::<i32>() + 4
    }
    fn max_scroll(&self) -> i32 {
        (self.content_height() - self.height).max(0)
    }
    fn scrollable(&self) -> bool {
        self.max_scroll() > 0
    }
    /// `TransferableSelectionList.Entry.getWidth`.
    fn entry_width(&self) -> i32 {
        self.row_width() - if self.scrollable() { 6 } else { 0 }
    }
    fn next_y(&self) -> i32 {
        self.y + 2 - self.scroll as i32 + self.entries.iter().map(|e| e.height).sum::<i32>()
    }
    fn add(&mut self, row: Row, height: i32) {
        let y = self.next_y();
        self.entries.push(Entry { row, y, height });
    }
    fn reposition(&mut self) {
        let mut y = self.y + 2 - self.scroll as i32;
        for e in &mut self.entries {
            e.y = y;
            y += e.height;
        }
    }
    fn set_scroll(&mut self, v: f64) {
        self.scroll = v.clamp(0.0, self.max_scroll() as f64);
        self.reposition();
    }
    fn scroll_to(&mut self, i: usize) {
        let e = &self.entries[i];
        let top = e.y - self.y - 2;
        if top < 0 {
            self.set_scroll(self.scroll + top as f64);
        }
        let e = &self.entries[i];
        let bottom = self.bottom() - e.y - e.height - 2;
        if bottom < 0 {
            self.set_scroll(self.scroll - bottom as f64);
        }
    }
    fn set_selected(&mut self, i: Option<usize>) {
        self.selected = i;
        if let Some(i) = i {
            let e = &self.entries[i];
            let top = e.y + 2 < self.y;
            let bottom = e.y + 2 + e.height - 4 > self.bottom();
            if top || bottom {
                self.scroll_to(i);
            }
        }
    }
    fn update_size(&mut self, width: i32, height: i32, x: i32, y: i32) {
        self.width = width;
        self.height = height;
        self.x = x;
        self.y = y;
        self.reposition();
        if let Some(i) = self.selected {
            self.scroll_to(i);
        }
        self.set_scroll(self.scroll);
    }
    fn is_over(&self, x: f64, y: f64) -> bool {
        x >= self.x as f64
            && y >= self.y as f64
            && x < (self.x + self.width) as f64
            && y < self.bottom() as f64
    }
    /// `getEntryAtPosition`.
    fn entry_at(&self, x: i32, y: i32) -> Option<usize> {
        let (left, w) = (self.row_left(), self.entry_width());
        self.entries
            .iter()
            .position(|e| x >= left && x < left + w && y >= e.y && y < e.y + e.height)
    }
    fn scroll_bar_x(&self) -> i32 {
        self.x + self.width - 6
    }
    fn scroller_height(&self) -> i32 {
        ((self.height * self.height) as f32 / self.content_height() as f32) as i32
    }
    /// `AbstractScrollArea.mouseDragged` while the scrollbar is held.
    fn drag(&mut self, y: f64) -> bool {
        let Some(last) = self.held else { return false };
        if y < self.y as f64 {
            self.set_scroll(0.0);
        } else if y > self.bottom() as f64 {
            self.set_scroll(self.max_scroll() as f64);
        } else {
            let max = self.max_scroll().max(1) as f64;
            let bar = self.scroller_height().clamp(32, self.height - 8);
            let scale = (max / (self.height - bar) as f64).max(1.0);
            self.set_scroll(self.scroll + (y - last) * scale);
        }
        self.held = Some(y);
        true
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Focus {
    None,
    Search,
    List(usize),
    /// Open Pack Folder (0) or Done (1).
    Button(usize),
}

#[derive(Clone, Debug)]
enum ConfirmAction {
    Select(String),
    Import(Vec<std::path::PathBuf>),
}

/// The open `ConfirmScreen` for an incompatible pack or dropped files.
#[derive(Clone, Debug)]
struct Confirm {
    action: ConfirmAction,
    title: Text,
    message: Text,
}

/// What a click on the screen asks the game to do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    None,
    /// The player accepted `pack.dropConfirm`.
    Import(Vec<std::path::PathBuf>),
    /// The Open Pack Folder button.
    OpenFolder,
    /// The Done button (`onClose`: commit).
    Done,
    /// Sunrise's own: the options button on the card of this pack, which has Polytone options.
    Options(String),
}

/// The pack screen and its state.
#[derive(Clone, Debug)]
pub struct PackScreen {
    /// `resourcePack.title`, or `dataPack.title` from Create World.
    pub title: Text,
    pub width: i32,
    pub height: i32,
    pub in_world: bool,
    pub packs: Vec<PackInfo>,
    pub model: SelectionModel,
    search: TextField,
    focus: Focus,
    /// 0: available, 1: selected.
    lists: [List; 2],
    confirm: Option<Confirm>,
    /// The selection changed since the screen opened.
    pub changed: bool,
    /// The packs' icons by their [`icon_texture`].
    pub icons: HashMap<String, Arc<[u8]>>,
    /// The packs with Polytone options (#11486): their cards carry an options button.
    pub with_options: std::collections::BTreeSet<String>,
    /// Which atlas slot holds which icon, kept from one frame to the next
    /// ([`PackScreen::bind_icons`]).
    icon_slots: std::cell::RefCell<IconSlots>,
}

/// The atlas's pack icon slots and the icon each holds. The atlas keeps what is
/// written into a slot, so an icon is sent once, when it takes a slot, and not
/// again every frame it is drawn.
#[derive(Clone, Debug, Default)]
struct IconSlots {
    /// The atlas the pixels went into ([`HudAtlas::revision`]); a new atlas holds none of them.
    revision: u64,
    /// Frames bound so far: an icon drawn in this one keeps its slot.
    frame: u64,
    /// Least recently drawn first: the icon's texture key, its slot, its pixels, and the
    /// frame it was last drawn in.
    held: Vec<(String, usize, Arc<[u8]>, u64)>,
}

impl IconSlots {
    /// Starts a frame on `atlas`; an atlas that is not the one the slots were filled in empties them.
    fn begin(&mut self, atlas: &HudAtlas) {
        if self.revision != atlas.revision {
            self.held.clear();
            self.revision = atlas.revision;
        }
        self.frame += 1;
    }

    /// The slot for the icon drawn under `texture`, and whether its pixels have to be sent. A new
    /// icon takes a free slot, else the slot of the icon drawn longest ago; none when every slot
    /// holds an icon drawn in this frame, and that icon keeps `unknown_pack`.
    fn slot_for(&mut self, texture: &str, icon: &Arc<[u8]>) -> Option<(usize, bool)> {
        if let Some(at) = self.held.iter().position(|h| h.0 == texture) {
            let mut kept = self.held.remove(at);
            let same = Arc::ptr_eq(&kept.2, icon);
            kept.2 = icon.clone();
            kept.3 = self.frame;
            let slot = kept.1;
            self.held.push(kept);
            return Some((slot, !same));
        }
        let slot = match (0..PACK_ICON_SLOTS).find(|i| !self.held.iter().any(|h| h.1 == *i)) {
            Some(free) => free,
            None if self.held.first().is_some_and(|h| h.3 < self.frame) => self.held.remove(0).1,
            None => return None,
        };
        self.held
            .push((texture.to_owned(), slot, icon.clone(), self.frame));
        Some((slot, true))
    }
}

fn tr(key: &str) -> Text {
    Text::translate(key, &[])
}

/// The options button on a pack's card.
const OPTIONS_SIZE: i32 = 16;
impl PackScreen {
    /// `PackSelectionScreen.init`: the lists filled, the search box focused.
    pub fn new(
        m: &Measure,
        title: Text,
        width: i32,
        height: i32,
        in_world: bool,
        packs: Vec<PackInfo>,
        selected: &[String],
    ) -> Self {
        let model = SelectionModel::new(packs.iter().map(PackInfo::selectable).collect(), selected);
        let mut search = TextField::new(0, 0, 200, 15);
        search.hint = Some(tr("gui.packSelection.search").with_style(&Style {
            color: Some(d::GRAY),
            italic: Some(true),
            ..Style::default()
        }));
        let icons = packs
            .iter()
            .filter_map(|p| Some((icon_texture(&p.id), p.icon.clone()?)))
            .collect();
        let mut s = Self {
            title,
            width,
            height,
            in_world,
            packs,
            icons,
            model,
            search,
            focus: Focus::None,
            lists: [List::new(false, height - 66), List::new(true, height - 66)],
            confirm: None,
            with_options: Default::default(),
            icon_slots: Default::default(),
            changed: false,
        };
        s.reposition(m);
        s.populate(None);
        s.set_focus(Focus::Search);
        s
    }
    fn info(&self, id: &str) -> Option<&PackInfo> {
        self.packs.iter().find(|p| p.id == id)
    }
    /// Whether the lists leave `id` out ([`PackInfo::hidden`]).
    fn hidden(&self, id: &str) -> bool {
        self.info(id).is_some_and(|p| p.hidden)
    }
    /// The header layout: title, drop info and search box, centred.
    fn header(&self, m: &Measure) -> [(i32, i32); 3] {
        let tw = m.width(&self.title);
        let dw = m.width(&tr("pack.dropInfo"));
        let col = tw.max(dw).max(200);
        let frame = self.width.max(col);
        let lx = (0.5 * (frame - col) as f32) as i32;
        let cell = |w: i32| lx + (0.5 * (col - w) as f32) as i32;
        [(cell(tw), 4), (cell(dw), 17), (cell(200), 30)]
    }
    /// The footer's two buttons.
    fn footer(&self) -> [(i32, i32); 2] {
        let frame = self.width.max(308);
        let x = (0.5 * (frame - 308) as f32) as i32;
        let y = self.height - 33 + (0.5f32 * 13.0).round() as i32;
        [(x, y), (x + 158, y)]
    }
    /// `repositionElements`.
    fn reposition(&mut self, m: &Measure) {
        let (sx, sy) = self.header(m)[2];
        self.search.x = sx;
        self.search.y = sy;
        let content = self.height - 49 - 33;
        let w = self.width;
        self.lists[0].update_size(200, content, w / 2 - 15 - 200, 49);
        self.lists[1].update_size(200, content, w / 2 + 15, 49);
    }
    /// `AbstractContainerEventHandler.setFocused` on the screen.
    fn set_focus(&mut self, f: Focus) {
        if self.focus == f {
            return;
        }
        match self.focus {
            Focus::Search => self.search.focused = false,
            Focus::List(i) => self.lists[i].focused = false,
            Focus::None | Focus::Button(_) => {}
        }
        if f == Focus::Search {
            self.search.focused = true;
        }
        self.focus = f;
    }
    fn shown(&self, side: usize) -> Vec<String> {
        let value = self.search.value.to_lowercase();
        let blank = self.search.value.trim().is_empty()
            && self.search.value.chars().all(char::is_whitespace);
        let list = if side == 1 {
            &self.model.selected
        } else {
            &self.model.unselected
        };
        list.iter()
            .filter(|p| !self.hidden(&p.id))
            .filter(|p| {
                blank
                    || p.id.to_lowercase().contains(&value)
                    || self.info(&p.id).is_some_and(|i| {
                        i.title.plain().to_lowercase().contains(&value)
                            || i.description.plain().to_lowercase().contains(&value)
                    })
            })
            .map(|p| p.id.clone())
            .collect()
    }
    /// `TransferableSelectionList.updateList`.
    fn update_list(&mut self, side: usize, ids: Vec<String>, transferred: Option<&str>) {
        let l = &mut self.lists[side];
        l.entries.clear();
        l.selected = None;
        l.add(Row::Header, 13);
        l.set_selected(None);
        for id in ids {
            let hit = transferred == Some(id.as_str());
            self.lists[side].add(Row::Pack(id), 36);
            if hit {
                self.set_focus(Focus::List(side));
                let l = &mut self.lists[side];
                l.focused = true;
                let i = l.entries.len() - 1;
                l.set_selected(Some(i));
            }
        }
        let l = &mut self.lists[side];
        l.set_scroll(l.scroll);
    }
    /// `populateLists`: both lists, then again through the search filter.
    fn populate(&mut self, transferred: Option<&str>) {
        let all = |m: &SelectionModel, side: bool| -> Vec<String> {
            (if side { &m.selected } else { &m.unselected })
                .iter()
                .map(|p| p.id.clone())
                .collect()
        };
        let visible = |ids: Vec<String>| -> Vec<String> {
            ids.into_iter().filter(|id| !self.hidden(id)).collect()
        };
        let (selected, available) = (all(&self.model, true), all(&self.model, false));
        let (selected, available) = (visible(selected), visible(available));
        self.update_list(1, selected, transferred);
        self.update_list(0, available, transferred);
        self.filter(transferred);
    }
    /// `updateFilteredEntries`.
    fn filter(&mut self, transferred: Option<&str>) {
        let (sel, avail) = (self.shown(1), self.shown(0));
        self.update_list(1, sel, transferred);
        self.update_list(0, avail, transferred);
    }
    /// The pack's index in its model list.
    fn model_index(&self, side: usize, id: &str) -> Option<usize> {
        let list = if side == 1 {
            &self.model.selected
        } else {
            &self.model.unselected
        };
        list.iter().position(|p| p.id == id)
    }
    fn flags(&self, side: usize, id: &str) -> (bool, bool, bool, bool) {
        let Some(i) = self.model_index(side, id) else {
            return (false, false, false, false);
        };
        let s = side == 1;
        (
            self.model.can_select(s, i),
            self.model.can_unselect(s, i),
            self.model.can_move_up(s, i),
            self.model.can_move_down(s, i),
        )
    }
    fn overlay(&self, id: &str) -> bool {
        self.info(id).is_none_or(|p| !p.fixed || !p.required)
    }

    /// Whether a left click activates a footer or confirmation button.
    /// Pack entries and the search field do not play widget click sounds.
    pub fn button_at(&self, m: &Measure, x: f64, y: f64) -> bool {
        let buttons = if let Some(c) = &self.confirm {
            self.confirm_layout(m, c).2
        } else {
            self.footer()
        };
        buttons.into_iter().any(|(bx, by)| {
            x >= bx as f64 && y >= by as f64 && x < (bx + 150) as f64 && y < (by + 20) as f64
        })
    }
    /// A left click (`Screen.mouseClicked`).
    pub fn click(&mut self, m: &Measure, x: f64, y: f64) -> Action {
        self.click_with_outcome(m, x, y).0
    }

    pub(crate) fn click_with_outcome(&mut self, m: &Measure, x: f64, y: f64) -> (Action, bool) {
        if self.confirm.is_some() {
            return self.confirm_click(m, x, y);
        }
        if self.search.contains(x, y) {
            self.search.click(m, x, false, false);
            self.set_focus(Focus::Search);
            return (Action::None, true);
        }
        for side in 0..2 {
            if !self.lists[side].is_over(x, y) {
                continue;
            }
            let l = &self.lists[side];
            let scrolling = l.scrollable()
                && x >= l.scroll_bar_x() as f64
                && x <= (l.scroll_bar_x() + 6) as f64
                && y >= l.y as f64
                && y < l.bottom() as f64;
            // updateScrolling: a press on the scroller holds it.
            if scrolling {
                self.lists[side].held = Some(y);
                self.set_focus(Focus::List(side));
                return (Action::None, true);
            }
            let l = &self.lists[side];
            let hit = l.entry_at(x as i32, y as i32);
            if let Some(i) = hit
                && let Some((id, bx, by)) = self.options_button(side, i)
                && d::inside((x as i32, y as i32), bx, by, OPTIONS_SIZE, OPTIONS_SIZE)
            {
                return (Action::Options(id), true);
            }
            let mut took = false;
            if let Some(i) = hit {
                took = true;
                match l.entries[i].row.clone() {
                    Row::Header => {
                        let l = &mut self.lists[side];
                        l.focused = true;
                        l.set_selected(Some(i));
                    }
                    Row::Pack(id) => {
                        let current = self.entry_click(m, side, i, &id, x as i32, y as i32);
                        if self.lists[side]
                            .entries
                            .iter()
                            .any(|e| e.row == Row::Pack(id.clone()))
                        {
                            let l = &mut self.lists[side];
                            l.focused = true;
                            if current {
                                l.set_selected(Some(i));
                            }
                        }
                    }
                }
            }
            if took {
                self.set_focus(Focus::List(side));
            }
            return (Action::None, took);
        }
        for (i, (bx, by)) in self.footer().into_iter().enumerate() {
            if x >= bx as f64 && y >= by as f64 && x < (bx + 150) as f64 && y < (by + 20) as f64 {
                self.set_focus(Focus::Button(i));
                let action = if i == 0 {
                    Action::OpenFolder
                } else {
                    Action::Done
                };
                return (action, true);
            }
        }
        (Action::None, false)
    }
    /// `PackEntry.mouseClicked`; true when the entry clicked is still the
    /// list's own (nothing was repopulated).
    fn entry_click(
        &mut self,
        m: &Measure,
        side: usize,
        i: usize,
        id: &str,
        x: i32,
        y: i32,
    ) -> bool {
        if !self.overlay(id) {
            return true;
        }
        let e = &self.lists[side].entries[i];
        let (cx, cy) = (self.lists[side].row_left() + 2, e.y + 2);
        let (rx, ry) = (x - cx, y - cy);
        let (sel, unsel, up, down) = self.flags(side, id);
        let icon = (0..32).contains(&rx) && (0..32).contains(&ry);
        let left = (0..16).contains(&rx) && (0..32).contains(&ry);
        let top_right = (16..32).contains(&rx) && (0..16).contains(&ry);
        let bottom_right = (16..32).contains(&rx) && (16..32).contains(&ry);
        let index = self.model_index(side, id).unwrap_or(0);
        let s = side == 1;
        if sel && icon {
            if self
                .info(id)
                .is_some_and(|p| p.compat != Compat::Compatible)
            {
                self.open_confirm(id);
                return true;
            }
            self.model.activate(s, index, true);
        } else if unsel && left {
            self.model.activate(s, index, false);
        } else if up && top_right {
            self.model.move_by(s, index, true);
        } else if down && bottom_right {
            self.model.move_by(s, index, false);
        } else {
            return true;
        }
        self.changed = true;
        self.populate(Some(id));
        let _ = m;
        false
    }
    /// `handlePackSelection` of an incompatible pack: the ConfirmScreen
    /// (the pack screen loses its focus as `Gui.setScreen` clears it).
    fn open_confirm(&mut self, id: &str) {
        let compat = self.info(id).map_or(Compat::Unknown, |p| p.compat);
        self.set_focus(Focus::None);
        for l in &mut self.lists {
            l.focused = false;
        }
        self.confirm = Some(Confirm {
            action: ConfirmAction::Select(id.to_owned()),
            title: tr("pack.incompatible.confirm.title"),
            message: tr(&format!("pack.incompatible.confirm.{}", compat.key())),
        });
    }
    /// Winit delivers one event per path; collect a multi-file drop in one confirmation.
    pub fn drop_file(&mut self, path: std::path::PathBuf) {
        if matches!(
            self.confirm.as_ref().map(|c| &c.action),
            Some(ConfirmAction::Select(_))
        ) {
            return;
        }
        let mut paths = match self.confirm.take() {
            Some(Confirm {
                action: ConfirmAction::Import(paths),
                ..
            }) => paths,
            _ => Vec::new(),
        };
        if !paths.contains(&path) {
            paths.push(path);
        }
        let names = paths
            .iter()
            .filter_map(|p| p.file_name())
            .map(|n| n.to_string_lossy())
            .collect::<Vec<_>>()
            .join(", ");
        self.set_focus(Focus::None);
        for list in &mut self.lists {
            list.focused = false;
        }
        self.confirm = Some(Confirm {
            action: ConfirmAction::Import(paths),
            title: tr("pack.dropConfirm"),
            message: Text::literal(names),
        });
    }
    /// The ConfirmScreen's layout: title, message lines and the two buttons.
    fn confirm_layout(
        &self,
        m: &Measure,
        c: &Confirm,
    ) -> ((i32, i32), (Vec<(Text, i32)>, i32, i32), [(i32, i32); 2]) {
        let max = self.width - 50;
        let lines = d::label_lines(m, &c.message, max, 15, &Style::default());
        let mw = d::label_width(&lines, max);
        let mh = lines.len() as i32 * 9;
        let tw = m.width(&c.title);
        let col = tw.max(mw).max(304);
        let total = 9 + 8 + mh + 8 + 36;
        let lx = (0.5 * (self.width - col) as f32) as i32;
        let ly = (0.5 * (self.height - total) as f32) as i32;
        let cell = |w: i32| lx + (0.5 * (col - w) as f32) as i32;
        let by = ly + 9 + 8 + mh + 8 + 16;
        let bx = cell(304);
        (
            (cell(tw), ly),
            (lines, cell(mw), ly + 17),
            [(bx, by), (bx + 154, by)],
        )
    }
    fn confirm_click(&mut self, m: &Measure, x: f64, y: f64) -> (Action, bool) {
        let Some(c) = self.confirm.clone() else {
            return (Action::None, false);
        };
        let (_, _, buttons) = self.confirm_layout(m, &c);
        for (i, (bx, by)) in buttons.into_iter().enumerate() {
            if x >= bx as f64 && y >= by as f64 && x < (bx + 150) as f64 && y < (by + 20) as f64 {
                // The callback: back to the pack screen (repositioned), then select.
                self.confirm = None;
                self.reposition(m);
                if i == 0 {
                    match c.action {
                        ConfirmAction::Import(paths) => return (Action::Import(paths), true),
                        ConfirmAction::Select(id) => {
                            if let Some(index) = self.model_index(0, &id) {
                                self.model.activate(false, index, true);
                                self.changed = true;
                                self.populate(Some(&id));
                            }
                        }
                    }
                }
                return (Action::None, true);
            }
        }
        (Action::None, false)
    }
    /// The focus, the lists' rows, scroll and selection (for debugging).
    pub fn debug(&self) -> String {
        format!(
            "{:?} {:?}",
            self.focus,
            self.lists
                .iter()
                .map(|l| (
                    l.scroll,
                    l.selected,
                    l.focused,
                    l.entries.iter().map(|e| e.y).collect::<Vec<_>>()
                ))
                .collect::<Vec<_>>()
        )
    }
    /// Whether the confirmation screen is shown.
    pub fn confirming(&self) -> bool {
        self.confirm.is_some()
    }
    /// Escape on the confirmation screen answers no.
    pub fn cancel_confirm(&mut self, m: &Measure) -> bool {
        if self.confirm.take().is_some() {
            self.reposition(m);
            return true;
        }
        false
    }
    /// The mouse wheel (`AbstractScrollArea.mouseScrolled`).
    /// The pointer moved with the button held: a held scrollbar follows it.
    pub fn drag(&mut self, y: f64) -> bool {
        self.lists.iter_mut().any(|l| l.drag(y))
    }
    /// `onRelease`.
    pub fn release(&mut self) -> bool {
        let mut handled = false;
        for l in &mut self.lists {
            handled |= l.held.take().is_some();
        }
        handled
    }
    pub fn scroll(&mut self, x: f64, y: f64, amount: f64) -> bool {
        if self.confirm.is_some() || self.search.contains(x, y) {
            return false;
        }
        for l in &mut self.lists {
            if l.is_over(x, y) {
                l.set_scroll(l.scroll - amount * 18.0);
                return true;
            }
        }
        false
    }
    /// Typed text, when the search box has the focus.
    pub fn type_text(&mut self, m: &Measure, s: &str) {
        if self.confirm.is_none() && self.focus == Focus::Search && self.search.insert_text(m, s) {
            self.filter(None);
        }
    }
    /// Ctrl+A/C/V/X in the search box; false when it does not have the focus.
    pub fn shortcut(
        &mut self,
        m: &Measure,
        shortcut: crate::clipboard::EditShortcut,
        clipboard: &mut dyn crate::clipboard::Clipboard,
    ) -> bool {
        if !self.search_focused() {
            return false;
        }
        if self.search.shortcut(m, shortcut, clipboard) {
            self.filter(None);
        }
        true
    }
    /// Backspace in the search box.
    pub fn erase(&mut self, m: &Measure, count: isize) {
        if self.confirm.is_none()
            && self.focus == Focus::Search
            && self.search.delete_chars(m, -count)
        {
            self.filter(None);
        }
    }
    pub fn search_focused(&self) -> bool {
        self.confirm.is_none() && self.focus == Focus::Search
    }
    /// Resize (`Screen.init` on a new size: `repositionElements`).
    pub fn resize(&mut self, m: &Measure, width: i32, height: i32) {
        self.width = width;
        self.height = height;
        self.reposition(m);
    }
    /// `PackSelectionModel.commit`: the selection as options.txt stores it.
    pub fn commit(&self) -> Vec<String> {
        self.model.commit()
    }
    /// Points the icon blits of `draws` at the atlas's pack icon slots and
    /// writes the icons into them (vanilla registers a texture per pack). An
    /// icon keeps its slot from one frame to the next and is sent when it takes
    /// the slot, not every frame; the slot of the icon drawn longest ago is
    /// taken first. Icons past the last slot in one frame keep `unknown_pack`.
    pub fn bind_icons(&self, draws: &mut [Draw], atlas: &HudAtlas, patches: &mut Vec<Patch>) {
        let mut slots = self.icon_slots.borrow_mut();
        slots.begin(atlas);
        for draw in draws {
            let Draw::Blit { texture, .. } = draw else {
                continue;
            };
            let Some(icon) = self.icons.get(texture.as_str()) else {
                continue;
            };
            let Some((slot, send)) = slots.slot_for(texture, icon) else {
                continue;
            };
            let sprite = format!("sunrise/pack-icon{slot}");
            let Some(rect) = atlas.sprites.get(&sprite) else {
                slots.held.pop();
                return;
            };
            if send {
                patches.push(Patch {
                    x: (rect.u0 * atlas.width as f32).round() as u32,
                    y: (rect.v0 * atlas.height as f32).round() as u32,
                    width: PACK_ICON_SIZE,
                    height: PACK_ICON_SIZE,
                    rgba: icon.to_vec(),
                });
            }
            *texture = sprite;
        }
    }

    /// The shown screen's calls (`Screen.extractRenderState`); `blink_ms`
    /// is the time since the search box took the focus.
    pub fn draw(&self, m: &Measure, mouse: (i32, i32), blink_ms: u64, now_ms: i64) -> Vec<Draw> {
        let mut out = Vec::new();
        // PackSelectionScreen, or the ConfirmScreen over it (its title and
        // its message).
        crate::widgets::begin_screen(|| match &self.confirm {
            Some(c) => crate::widgets::ScreenInfo::new(
                "packs/confirm",
                crate::widgets::join_for_narration(&[&c.title.plain(), &c.message.plain()]),
            ),
            None => crate::widgets::ScreenInfo::new("packs", self.title.plain()),
        });
        if let Some(c) = &self.confirm {
            let ((tx, ty), (lines, mx, my), buttons) = self.confirm_layout(m, c);
            d::string_widget(&mut out, m, &c.title, tx, ty, 0);
            d::label(&mut out, &lines, mx, my, self.width - 50, true);
            for ((bx, by), key) in buttons.into_iter().zip(["gui.yes", "gui.no"]) {
                let hover = d::inside(mouse, bx, by, 150, 20);
                crate::widgets::widget_cursor(hover, true);
                d::button(&mut out, m, &tr(key), bx, by, 150, 20, true, hover, now_ms);
            }
            return out;
        }
        let [(tx, ty), (dx, dy), _] = self.header(m);
        d::string_widget(&mut out, m, &self.title, tx, ty, 0);
        d::string_widget(
            &mut out,
            m,
            &tr("pack.dropInfo").with_color(d::GRAY),
            dx,
            dy,
            0,
        );
        self.search.draw(&mut out, m, blink_ms, None);
        if self.search.contains(f64::from(mouse.0), f64::from(mouse.1)) {
            crate::widgets::text_cursor(true, true);
        }
        for side in 0..2 {
            self.draw_list(&mut out, m, side, mouse);
        }
        self.draw_footer(
            &mut out,
            m,
            mouse,
            now_ms,
            [&tr("pack.openFolder"), &tr("gui.done")],
        );
        out
    }
    /// The options button on the card of row `i` of list `side`, when its pack has
    /// Polytone options: the pack and the button's corner, at the card's right edge.
    fn options_button(&self, side: usize, i: usize) -> Option<(String, i32, i32)> {
        let l = &self.lists[side];
        let e = l.entries.get(i)?;
        let Row::Pack(id) = &e.row else { return None };
        if !self.with_options.contains(id) {
            return None;
        }
        let x = l.row_left() + l.entry_width() - OPTIONS_SIZE - 2;
        let y = e.y + (e.height - OPTIONS_SIZE) / 2;
        Some((id.clone(), x, y))
    }
    /// The options button of a card: a small button with a sliders glyph.
    fn draw_options_button(out: &mut Vec<Draw>, x: i32, y: i32, hover: bool) {
        crate::widgets::narratable(|| {
            crate::widgets::Narratable::button(
                tr("sunrise.pack.options").plain(),
                [x as f32, y as f32, OPTIONS_SIZE as f32, OPTIONS_SIZE as f32],
                true,
            )
        });
        let sprite = if hover {
            "widget/button_highlighted"
        } else {
            "widget/button"
        };
        d::sprite(out, sprite, x, y, OPTIONS_SIZE, OPTIONS_SIZE);
        let (line, knob) = if hover {
            (0xFF_C6_C6_C6u32 as i32, -1)
        } else {
            (0xFF_A0_A0_A0u32 as i32, 0xFF_E0_E0_E0u32 as i32)
        };
        // Three sliders, their knobs at different places.
        for (row, at) in [(4, 8), (8, 4), (12, 9)] {
            d::fill(out, x + 3, y + row, x + 13, y + row + 1, line);
            d::fill(out, x + at, y + row - 1, x + at + 2, y + row + 2, knob);
        }
    }
    /// The footer's two buttons, "Open Pack Folder" and "Done", over their
    /// labels; a label wider than its button scrolls with the clock.
    fn draw_footer(
        &self,
        out: &mut Vec<Draw>,
        m: &Measure,
        mouse: (i32, i32),
        now_ms: i64,
        labels: [&Text; 2],
    ) {
        for (i, ((bx, by), label)) in self.footer().into_iter().zip(labels).enumerate() {
            let hover = d::inside(mouse, bx, by, 150, 20);
            // The cursor follows the mouse; the highlight also keyboard focus.
            crate::widgets::widget_cursor(hover, true);
            if i == 1 {
                crate::widgets::declare_action(
                    crate::widgets::Action::Done,
                    [bx as f32, by as f32, 150.0, 20.0],
                    true,
                );
            }
            d::button(
                out,
                m,
                label,
                bx,
                by,
                150,
                20,
                true,
                hover || self.focus == Focus::Button(i),
                now_ms,
            );
            if i == 0 {
                crate::widgets::last_narratable(|n| {
                    n.hint = Some(tr("pack.folderInfo").plain());
                });
            }
            if i == 0 && hover {
                d::tooltip(out, m, &tr("pack.folderInfo"), mouse);
            }
        }
    }
    fn draw_list(&self, out: &mut Vec<Draw>, m: &Measure, side: usize, mouse: (i32, i32)) {
        let l = &self.lists[side];
        let hovered = if l.is_over(mouse.0 as f64, mouse.1 as f64) {
            l.entry_at(mouse.0, mouse.1)
        } else {
            None
        };
        let header = if l.selected_side {
            "pack.selected.title"
        } else {
            "pack.available.title"
        };
        // TransferableSelectionList: the row under the mouse, else the
        // selected one; the header narrates its title, a pack "Selected:
        // <title>".
        crate::widgets::narratable(|| {
            use crate::widgets::{ListKind, Narratable, Row as Narrated};
            let narrated = |i: usize| {
                let text = match &l.entries[i].row {
                    Row::Header => tr(header).plain(),
                    Row::Pack(id) => {
                        let title = self
                            .info(id)
                            .map_or_else(|| Text::literal(id.clone()), |p| p.title.clone());
                        Text::translate("narrator.select", &[title]).plain()
                    }
                };
                Narrated::new(i, text)
            };
            Narratable::list(
                ListKind::Object,
                l.entries.len(),
                hovered.map(narrated),
                l.selected.filter(|i| *i < l.entries.len()).map(narrated),
                [l.x as f32, l.y as f32, l.width as f32, l.height as f32],
            )
            .key(crate::widgets::key_of(&("packs", side)))
            .focused(l.focused)
        });
        let prefix = if self.in_world { "inworld_" } else { "" };
        let right = l.x + l.width;
        d::blit(
            out,
            &format!("textures/gui/{prefix}menu_list_background.png"),
            l.x,
            l.y,
            (right as f32, (l.bottom() + l.scroll as i32) as f32),
            l.width,
            l.height,
            [32, 32],
        );
        out.push(Draw::Scissor {
            x0: l.x.clamp(0, self.width),
            y0: l.y.clamp(0, self.height),
            x1: right.clamp(0, self.width),
            y1: l.bottom().clamp(0, self.height),
        });
        let (left, w) = (l.row_left(), l.entry_width());
        let narrow = if l.scrollable() { 6 } else { 0 };
        let mut tooltip = None;
        for (i, e) in l.entries.iter().enumerate() {
            if e.y + e.height < l.y || e.y > l.bottom() {
                continue;
            }
            if l.selected == Some(i) {
                let color = if l.focused { -1 } else { -8355712 };
                d::fill(out, left, e.y, left + w, e.y + e.height, color);
                d::fill(
                    out,
                    left + 1,
                    e.y + 1,
                    left + w - 1,
                    e.y + e.height - 1,
                    -16777216,
                );
            }
            let (cx, cy) = (left + 2, e.y + 2);
            match &e.row {
                Row::Header => {
                    let t = tr(header).with_style(&Style {
                        underlined: Some(true),
                        bold: Some(true),
                        ..Style::default()
                    });
                    let tw = m.width(&t);
                    d::text(
                        out,
                        t,
                        left + w / 2 - tw / 2,
                        cy + (e.height - 4) / 2 - 4,
                        -1,
                        true,
                    );
                }
                Row::Pack(id) => {
                    let Some(p) = self.info(id) else { continue };
                    if p.compat != Compat::Compatible {
                        d::fill(
                            out,
                            cx - 1,
                            cy - 1,
                            cx + w - 4 + 1,
                            cy + e.height - 4 + 1,
                            -8978432,
                        );
                    }
                    let icon = if p.icon.is_some() {
                        icon_texture(id)
                    } else {
                        "textures/misc/unknown_pack.png".into()
                    };
                    d::blit(out, &icon, cx, cy, (0.0, 0.0), 32, 32, [32, 32]);
                    let (mut name, (mut desc, mut desc_style)) =
                        (p.title.clone(), p.extended_description());
                    let focus_hover = l.selected == Some(i) && l.focused;
                    if self.overlay(id) && (hovered == Some(i) || focus_hover) {
                        d::fill(out, cx, cy, cx + 32, cy + 32, -1601138544);
                        let (rx, ry) = (mouse.0 - cx, mouse.1 - cy);
                        if p.compat != Compat::Compatible {
                            name = tr("pack.incompatible");
                            desc = tr(&format!("pack.incompatible.{}", p.compat.key()))
                                .with_color(d::GRAY);
                            desc_style = Style::color(d::GRAY);
                        }
                        let (sel, unsel, up, down) = self.flags(side, id);
                        let quarter = |x0: i32, x1: i32, y0: i32, y1: i32| {
                            (x0..x1).contains(&rx) && (y0..y1).contains(&ry)
                        };
                        let mut arrow = |name: &str, on: bool| {
                            let s = if on {
                                format!("transferable_list/{name}_highlighted")
                            } else {
                                format!("transferable_list/{name}")
                            };
                            d::sprite(out, &s, cx, cy, 32, 32);
                        };
                        if sel {
                            arrow("select", quarter(0, 32, 0, 32));
                        } else {
                            if unsel {
                                arrow("unselect", quarter(0, 16, 0, 32));
                            }
                            if up {
                                arrow("move_up", quarter(16, 32, 0, 16));
                            }
                            if down {
                                arrow("move_down", quarter(16, 32, 16, 32));
                            }
                        }
                    }
                    let mut max = 157 - narrow;
                    if let Some((_, bx, by)) = self.options_button(side, i) {
                        let hover = hovered == Some(i)
                            && d::inside(mouse, bx, by, OPTIONS_SIZE, OPTIONS_SIZE);
                        Self::draw_options_button(out, bx, by, hover);
                        // A control for a pad's crosshair to glide onto and its D-pad to hop to,
                        // while the list shows it whole.
                        let mark = crate::widgets::targets_mark();
                        let size = OPTIONS_SIZE as f32;
                        crate::widgets::target([bx as f32, by as f32, size, size]);
                        crate::widgets::clip_targets(mark, l.y as f32, l.bottom() as f32);
                        if hover {
                            tooltip = Some(tr("sunrise.pack.optionsInfo"));
                        }
                        max -= OPTIONS_SIZE + 2;
                    }
                    d::string_widget(out, m, &name, cx + 34, cy + 1, max);
                    let lines = d::label_lines(m, &desc, max, 2, &desc_style);
                    d::label(out, &lines, cx + 34, cy + 12, max, false);
                }
            }
        }
        out.push(Draw::NoScissor);
        if let Some(t) = tooltip {
            d::tooltip(out, m, &t, mouse);
        }
        d::blit(
            out,
            &format!("textures/gui/{prefix}header_separator.png"),
            l.x,
            l.y - 2,
            (0.0, 0.0),
            l.width,
            2,
            [32, 2],
        );
        d::blit(
            out,
            &format!("textures/gui/{prefix}footer_separator.png"),
            l.x,
            l.bottom(),
            (0.0, 0.0),
            l.width,
            2,
            [32, 2],
        );
        if l.scrollable() {
            // AbstractScrollArea.extractScrollbar: over the bar, the pointer
            // points, or resizes while the bar is held.
            let over_bar = mouse.0 >= l.scroll_bar_x()
                && mouse.0 < l.scroll_bar_x() + 6
                && mouse.1 >= l.y
                && mouse.1 < l.bottom();
            if over_bar {
                crate::widgets::request_cursor(if l.held.is_some() {
                    crate::widgets::Cursor::ResizeNs
                } else {
                    crate::widgets::Cursor::PointingHand
                });
            }
            let sh = l.scroller_height().clamp(32, l.height - 8);
            let sy = (l.scroll as i32 * (l.height - sh) / l.max_scroll() + l.y).max(l.y);
            d::sprite(
                out,
                "widget/scroller_background",
                l.scroll_bar_x(),
                l.y,
                6,
                l.height,
            );
            d::sprite(out, "widget/scroller", l.scroll_bar_x(), sy, 6, sh);
        }
    }
}

#[cfg(test)]
mod icon_tests {
    use super::*;
    use crate::packs::Position;

    #[test]
    fn dropped_files_require_yes_and_keep_the_pending_selection() {
        let atlas =
            crate::hud::HudAtlas::build(&crate::resource_source::ResourceSource::directory(
                "/nonexistent/sunrise-test-assets".into(),
            ))
            .unwrap();
        let m = Measure(&atlas.font);
        let mut screen = PackScreen::new(
            &m,
            tr("resourcePack.title"),
            480,
            270,
            false,
            vec![pack("file/Keep", None)],
            &[],
        );
        screen.model.activate(false, 0, true);
        screen.changed = true;
        let before = screen.commit();
        assert_eq!(before, ["file/Keep"]);
        screen.drop_file("One.zip".into());
        screen.drop_file("Two".into());
        screen.drop_file("One.zip".into());
        let no = screen
            .confirm_layout(&m, screen.confirm.as_ref().unwrap())
            .2[1];
        assert_eq!(
            screen.click(&m, f64::from(no.0 + 1), f64::from(no.1 + 1)),
            Action::None
        );
        assert!(!screen.confirming());
        assert_eq!(screen.commit(), before);
        screen.drop_file("One.zip".into());
        screen.drop_file("Two".into());
        screen.drop_file("One.zip".into());
        let yes = screen
            .confirm_layout(&m, screen.confirm.as_ref().unwrap())
            .2[0];
        assert_eq!(
            screen.click(&m, f64::from(yes.0 + 1), f64::from(yes.1 + 1)),
            Action::Import(vec!["One.zip".into(), "Two".into()])
        );
        assert_eq!(screen.commit(), before);
        screen.drop_file("Cancelled.zip".into());
        assert!(screen.cancel_confirm(&m));
        assert!(!screen.confirming());
        let mut options = crate::options_ui::OptionsUi::default();
        options.pack_screen = Some(screen);
        options.update_pack_infos(vec![pack("file/Keep", None), pack("file/Imported", None)]);
        assert_eq!(options.pack_selection, ["file/Keep"]);
        assert!(options.pack_screen.is_none());
    }

    fn png(w: u32, h: u32, rgba: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut encoder = png::Encoder::new(&mut out, w, h);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder
            .write_header()
            .unwrap()
            .write_image_data(rgba)
            .unwrap();
        out
    }
    fn pack(id: &str, icon: Option<Arc<[u8]>>) -> PackInfo {
        PackInfo {
            id: id.into(),
            title: Text::literal(id),
            description: Text::literal(""),
            description_style: Style::default(),
            source: Source::BuiltIn,
            compat: Compat::Compatible,
            icon,
            required: false,
            position: Position::Top,
            fixed: false,
            hidden: false,
        }
    }

    /// Any size stretches over the whole square, nearest texels, as the
    /// icon texture is sampled into its 32x32 blit.
    #[test]
    fn icons_stretch_to_the_slot_by_nearest_texels() {
        let (red, blue) = ([255, 0, 0, 255], [0, 0, 255, 128]);
        let icon = load_icon(&png(2, 1, &[red, blue].concat())).unwrap();
        let side = PACK_ICON_SIZE as usize;
        assert_eq!(icon.len(), side * side * 4);
        let at = |x: usize, y: usize| &icon[(y * side + x) * 4..][..4];
        assert_eq!((at(0, 0), at(side / 2 - 1, side - 1)), (&red[..], &red[..]));
        assert_eq!(
            (at(side / 2, 0), at(side - 1, side - 1)),
            (&blue[..], &blue[..])
        );
        assert!(load_icon(b"not a png").is_none());
    }

    /// A mod's hidden pack is in neither list, but the selection keeps it: a required hidden pack
    /// stays selected when the player commits.
    #[test]
    fn hidden_packs_stay_selected_but_out_of_both_lists() {
        let atlas = crate::hud::HudAtlas::build(
            &sunrise_assets::resource_source::ResourceSource::directory(
                "/nonexistent/sunrise-test-assets".into(),
            ),
        )
        .unwrap();
        let m = Measure(&atlas.font);
        let mut compat = pack("mod/example:compat", None);
        compat.hidden = true;
        compat.required = true;
        let mut optional = pack("mod/example:optional", None);
        optional.hidden = true;
        let packs = vec![compat, optional, pack("file/Plain", None)];
        let selected = ["mod/example:compat".to_owned()];
        let screen = PackScreen::new(
            &m,
            tr("resourcePack.title"),
            480,
            320,
            false,
            packs,
            &selected,
        );
        let listed: Vec<&Row> = screen
            .lists
            .iter()
            .flat_map(|l| &l.entries)
            .map(|e| &e.row)
            .collect();
        assert!(
            listed.contains(&&Row::Pack("file/Plain".into())),
            "{listed:?}"
        );
        for id in ["mod/example:compat", "mod/example:optional"] {
            assert!(!listed.contains(&&Row::Pack(id.into())), "{id} listed");
        }
        assert!(screen.commit().contains(&"mod/example:compat".to_owned()));
        assert!(!screen.commit().contains(&"mod/example:optional".to_owned()));
    }

    /// A pack with Polytone options carries an options button on its card, which opens
    /// that pack's options and nothing else; a pack without options has none.
    #[test]
    fn a_card_with_options_opens_its_own_pack_options() {
        let atlas = crate::hud::HudAtlas::build(
            &sunrise_assets::resource_source::ResourceSource::directory(
                "/nonexistent/sunrise-test-assets".into(),
            ),
        )
        .unwrap();
        let m = Measure(&atlas.font);
        let packs = vec![pack("file/Configurable", None), pack("file/Plain", None)];
        let mut screen = PackScreen::new(&m, tr("resourcePack.title"), 480, 320, false, packs, &[]);
        screen.with_options.insert("file/Configurable".into());
        let row = |screen: &PackScreen, id: &str| {
            screen.lists[0]
                .entries
                .iter()
                .position(|e| e.row == Row::Pack(id.into()))
                .unwrap()
        };
        assert!(
            screen
                .options_button(0, row(&screen, "file/Plain"))
                .is_none()
        );
        let (id, bx, by) = screen
            .options_button(0, row(&screen, "file/Configurable"))
            .unwrap();
        assert_eq!(id, "file/Configurable");
        let (cx, cy) = (bx + OPTIONS_SIZE / 2, by + OPTIONS_SIZE / 2);
        // Hovered, it explains itself; a pad's crosshair reaches it as a control.
        let draws = screen.draw(&m, (cx, cy), 0, 0);
        assert!(draws.iter().any(|d| matches!(d, Draw::Tooltip { .. })));
        crate::widgets::end_frame(crate::widgets::generation() + 1);
        let size = OPTIONS_SIZE as f32;
        let button = [bx as f32, by as f32, size, size];
        let targets = crate::widgets::last_targets();
        assert!(targets.iter().any(|t| t.rect == button), "{targets:?}");
        assert_eq!(
            screen.click(&m, cx as f64, cy as f64),
            Action::Options("file/Configurable".into())
        );
        // The press opened the options, it did not move the pack.
        assert!(screen.commit().is_empty());
    }

    /// Programmer Art and High Contrast draw their own `pack.png`; a pack
    /// without one keeps `unknown_pack`.
    #[test]
    fn drawn_icons_take_atlas_slots() {
        let atlas = crate::hud::HudAtlas::build(
            &sunrise_assets::resource_source::ResourceSource::directory(
                (std::path::Path::new("/nonexistent/sunrise-test-assets")).to_owned(),
            ),
        )
        .unwrap();
        let m = Measure(&atlas.font);
        let dirt = load_icon(&png(1, 1, &[134, 96, 67, 255])).unwrap();
        let contrast = load_icon(&png(1, 1, &[0, 0, 0, 255])).unwrap();
        let packs = vec![
            pack("programmer_art", Some(dirt.clone())),
            pack("high_contrast", Some(contrast.clone())),
            pack("file/Iconless", None),
        ];
        let screen = PackScreen::new(&m, tr("resourcePack.title"), 480, 320, false, packs, &[]);
        let mut draws = screen.draw(&m, (0, 0), 0, 0);
        let mut patches = Vec::new();
        screen.bind_icons(&mut draws, &atlas, &mut patches);
        let icons: Vec<&str> = draws
            .iter()
            .filter_map(|d| match d {
                Draw::Blit { texture, .. } if texture.contains("pack") => Some(texture.as_str()),
                _ => None,
            })
            .collect();
        // The list shows them sorted by id.
        assert_eq!(
            icons,
            [
                "textures/misc/unknown_pack.png",
                "sunrise/pack-icon0",
                "sunrise/pack-icon1"
            ]
        );
        assert_eq!(patches.len(), 2);
        for (i, (patch, icon)) in patches.iter().zip([&contrast, &dirt]).enumerate() {
            let rect = atlas.sprites[&format!("sunrise/pack-icon{i}")];
            assert_eq!(
                (patch.x, patch.y, patch.width, patch.height),
                (
                    (rect.u0 * atlas.width as f32).round() as u32,
                    (rect.v0 * atlas.height as f32).round() as u32,
                    PACK_ICON_SIZE,
                    PACK_ICON_SIZE
                )
            );
            assert_eq!(&patch.rgba[..], &icon[..]);
        }
    }
}

#[cfg(test)]
mod click_sound_tests {
    use super::*;

    #[test]
    fn only_footer_and_confirmation_buttons_request_a_click_sound() {
        let atlas = crate::hud::HudAtlas::build(
            &sunrise_assets::resource_source::ResourceSource::directory(
                (std::path::Path::new("/nonexistent/sunrise-test-assets")).to_owned(),
            ),
        )
        .unwrap();
        let m = Measure(&atlas.font);
        let title = tr("resourcePack.title");
        let mut screen = PackScreen::new(&m, title, 480, 320, false, Vec::new(), &[]);
        assert!(!screen.button_at(&m, 0.0, 0.0));
        assert!(!screen.button_at(
            &m,
            screen.search.x as f64 + 1.0,
            screen.search.y as f64 + 1.0
        ));
        let buttons = screen.footer();
        for (i, (x, y)) in buttons.into_iter().enumerate() {
            assert!(screen.button_at(&m, x as f64 + 1.0, y as f64 + 1.0));
            assert!(!screen.button_at(&m, (x + 150) as f64, (y + 20) as f64));
            assert_eq!(
                screen.click(&m, x as f64 + 1.0, y as f64 + 1.0),
                if i == 0 {
                    Action::OpenFolder
                } else {
                    Action::Done
                }
            );
        }
        screen.confirm = Some(Confirm {
            action: ConfirmAction::Select("incompatible".into()),
            title: Text::literal("Confirm"),
            message: Text::literal("Use this pack?"),
        });
        let buttons = screen
            .confirm_layout(&m, screen.confirm.as_ref().unwrap())
            .2;
        for (x, y) in buttons {
            assert!(screen.button_at(&m, x as f64 + 1.0, y as f64 + 1.0));
        }
        assert!(!screen.button_at(
            &m,
            screen.footer()[0].0 as f64 + 1.0,
            screen.footer()[0].1 as f64 + 1.0
        ));
        let (x, y) = buttons[1];
        assert_eq!(
            screen.click(&m, x as f64 + 1.0, y as f64 + 1.0),
            Action::None
        );
        assert!(!screen.confirming());
    }
}

#[cfg(test)]
mod footer_label_tests {
    use super::*;
    use crate::lang::{self, Language};

    /// The footer drawn with these labels (no global language involved) and
    /// the footer buttons' positions.
    fn footer(
        atlas: &HudAtlas,
        open_folder: &Text,
        done: &Text,
        now_ms: i64,
    ) -> (Vec<Draw>, [(i32, i32); 2]) {
        let m = Measure(&atlas.font);
        let screen = PackScreen::new(&m, Text::literal("Packs"), 480, 320, false, Vec::new(), &[]);
        let mut out = Vec::new();
        screen.draw_footer(&mut out, &m, (0, 0), now_ms, [open_folder, done]);
        (out, screen.footer())
    }
    /// Every label drawn in `[bx + 2, bx + 148] x [by, by + 20]` of its button:
    /// the scissor of a scrolling one is that box, and the text sits between
    /// the scissor's calls.
    fn assert_labels_stay_in_their_buttons(draws: &[Draw], buttons: &[(i32, i32)]) -> usize {
        let mut scissors = 0;
        let mut open = None;
        let mut button = 0;
        for d in draws {
            match d {
                Draw::Sprite { x, y, .. } => {
                    button = buttons.iter().position(|b| *b == (*x, *y)).unwrap();
                }
                Draw::Scissor { x0, y0, x1, y1 } => {
                    let (bx, by) = buttons[button];
                    assert_eq!([*x0, *y0, *x1, *y1], [bx + 2, by, bx + 148, by + 20]);
                    assert!(open.replace(()).is_none());
                    scissors += 1;
                }
                Draw::NoScissor => assert!(open.take().is_some()),
                _ => {}
            }
        }
        assert!(open.is_none());
        scissors
    }

    /// The Resource Packs footer with a label wider than its 150 px button
    /// does not panic and scrolls; the short "Done" keeps its centre.
    #[test]
    fn a_footer_label_wider_than_its_button_scrolls() {
        let atlas = crate::hud::HudAtlas::build(
            &sunrise_assets::resource_source::ResourceSource::directory(
                (std::path::Path::new("/nonexistent/sunrise-test-assets")).to_owned(),
            ),
        )
        .unwrap();
        let m = Measure(&atlas.font);
        let wide = Text::literal("Otvori papku s paketami resursov, i jos nesto");
        assert!(m.width(&wide) > 146);
        let (draws, buttons) = footer(&atlas, &wide, &Text::literal("Done"), 0);
        assert_eq!(assert_labels_stay_in_their_buttons(&draws, &buttons), 1);
        // The clock moves it.
        let x = |draws: &[Draw]| {
            draws
                .iter()
                .find_map(|d| match d {
                    Draw::Text { x, .. } => Some(*x),
                    _ => None,
                })
                .unwrap()
        };
        let (later, _) = footer(&atlas, &wide, &Text::literal("Done"), 2000);
        assert_ne!(x(&draws), x(&later));
        // Both too wide: both scroll.
        let (draws, buttons) = footer(&atlas, &wide, &wide, 0);
        assert_eq!(assert_labels_stay_in_their_buttons(&draws, &buttons), 2);
    }

    /// The real language files: the Resource Packs footer in each of the 143
    /// languages draws without a panic, and in the ones whose "Open Pack
    /// Folder" is wider than the button (Afrikaans and Macedonian among them,
    /// the crash of #6129) it scrolls. A local `Language` per file: the global
    /// language stays as it was.
    #[test]
    #[ignore = "daily"]
    fn the_footer_draws_in_every_language_file() {
        let root = std::env::var_os("SUNRISE_TEST_RESOURCES")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| {
                std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("../../.local/sunrise-client/26.3")
            });
        let atlas = crate::hud::HudAtlas::build(
            &sunrise_assets::resource_source::ResourceSource::directory(root.to_path_buf()),
        )
        .unwrap();
        let m = Measure(&atlas.font);
        let before = lang::code();
        let mut codes: Vec<String> = std::fs::read_dir(root.join("assets/minecraft/lang"))
            .unwrap()
            .filter_map(|e| {
                let name = e.ok()?.file_name().into_string().ok()?;
                name.strip_suffix(".json").map(str::to_owned)
            })
            .collect();
        codes.sort();
        assert!(codes.len() >= 143, "{}", codes.len());
        let mut scrolling = Vec::new();
        for code in &codes {
            let language = Language::load(&[&root], code);
            let label = |key| Text::literal(language.get(key).unwrap_or(key));
            let (open_folder, done) = (label("pack.openFolder"), label("gui.done"));
            for now_ms in [0, 700, 1300, 5000] {
                let (draws, buttons) = footer(&atlas, &open_folder, &done, now_ms);
                let n = assert_labels_stay_in_their_buttons(&draws, &buttons);
                assert_eq!(
                    n,
                    [&open_folder, &done]
                        .iter()
                        .filter(|t| m.width(t) > 146)
                        .count(),
                    "{code}"
                );
                if now_ms == 0 && n > 0 {
                    scrolling.push(code.as_str());
                }
            }
        }
        for code in ["af_za", "mk_mk"] {
            assert!(scrolling.contains(&code), "{code} not in {scrolling:?}");
        }
        assert_eq!(lang::code(), before, "the global language was left alone");
    }
}

#[cfg(test)]
mod icon_slot_tests {
    use super::*;
    use crate::packs::Position;

    fn atlas() -> HudAtlas {
        HudAtlas::build(&sunrise_assets::resource_source::ResourceSource::directory(
            "/nonexistent/sunrise-test-assets".into(),
        ))
        .unwrap()
    }

    /// `count` packs with an icon each, the pixels of icon `i` all `i`.
    fn packs(count: usize) -> Vec<PackInfo> {
        let side = PACK_ICON_SIZE as usize;
        (0..count)
            .map(|i| PackInfo {
                id: format!("file/pack{i:03}"),
                title: Text::literal(format!("pack{i:03}")),
                description: Text::literal(""),
                description_style: Style::default(),
                source: Source::BuiltIn,
                compat: Compat::Compatible,
                icon: Some(vec![i as u8; side * side * 4].into()),
                required: false,
                position: Position::Top,
                fixed: false,
                hidden: false,
            })
            .collect()
    }

    /// One frame as `options_ui` draws it: the calls, then the icons bound. The icons drawn
    /// (their texture keys before binding), the textures after, and the patches sent.
    fn frame(screen: &PackScreen, atlas: &HudAtlas) -> (Vec<String>, Vec<String>, Vec<Patch>) {
        let m = Measure(&atlas.font);
        let mut draws = screen.draw(&m, (0, 0), 0, 0);
        let drawn: Vec<String> = draws
            .iter()
            .filter_map(|d| match d {
                Draw::Blit { texture, .. } if screen.icons.contains_key(texture) => {
                    Some(texture.clone())
                }
                _ => None,
            })
            .collect();
        let mut patches = Vec::new();
        screen.bind_icons(&mut draws, atlas, &mut patches);
        let bound = draws
            .iter()
            .filter_map(|d| match d {
                Draw::Blit { texture, .. } if texture.contains("pack-icon") => {
                    Some(texture.clone())
                }
                _ => None,
            })
            .collect();
        (drawn, bound, patches)
    }

    /// Only the available list has packs: one row of 36 for each of `count` icons fits in a
    /// window `height` high.
    /// `AbstractScrollArea.extractScrollbar`: over a held pack list's bar the
    /// pointer asks to resize vertically, at rest it points.
    #[test]
    fn the_pack_scrollbar_asks_for_its_cursor() {
        let atlas = atlas();
        let m = Measure(&atlas.font);
        let mut screen = screen(&m, 30, 200);
        let l = &screen.lists[0];
        assert!(l.scrollable());
        let at = (l.scroll_bar_x() + 2, l.y + 8);
        let _ = crate::widgets::take_cursor();
        screen.draw(&m, at, 0, 0);
        assert_eq!(
            crate::widgets::take_cursor(),
            crate::widgets::Cursor::PointingHand
        );
        screen.lists[0].held = Some(f64::from(at.1));
        screen.draw(&m, at, 0, 0);
        assert_eq!(
            crate::widgets::take_cursor(),
            crate::widgets::Cursor::ResizeNs
        );
    }
    fn screen(m: &Measure, count: usize, height: i32) -> PackScreen {
        PackScreen::new(
            m,
            tr("resourcePack.title"),
            480,
            height,
            false,
            packs(count),
            &[],
        )
    }

    /// Every frame sent every drawn icon again (a 128 by 128 RGBA copy each) into the same
    /// atlas, which keeps what it is given: an icon is sent when it takes a slot, once.
    #[test]
    fn an_icon_is_sent_once_and_not_every_frame() {
        let atlas = atlas();
        let screen = screen(&Measure(&atlas.font), 5, 400);
        let (drawn, bound, patches) = frame(&screen, &atlas);
        assert_eq!((drawn.len(), bound.len(), patches.len()), (5, 5, 5));
        for again in 0..3 {
            let (_, rebound, resent) = frame(&screen, &atlas);
            assert_eq!(rebound, bound, "frame {again}");
            assert_eq!(resent.len(), 0, "frame {again}");
        }
    }

    /// More icons drawn in one frame than there are slots: the first 64 get one, the rest keep
    /// `unknown_pack`, and the next frame does not move them or send anything.
    #[test]
    fn past_the_slots_in_one_frame_the_rest_keep_unknown_pack() {
        let atlas = atlas();
        let screen = screen(&Measure(&atlas.font), PACK_ICON_SLOTS + 6, 4000);
        let (drawn, bound, patches) = frame(&screen, &atlas);
        assert_eq!(
            drawn.len(),
            PACK_ICON_SLOTS + 6,
            "the window shows every row"
        );
        assert_eq!(
            (bound.len(), patches.len()),
            (PACK_ICON_SLOTS, PACK_ICON_SLOTS)
        );
        let (_, rebound, resent) = frame(&screen, &atlas);
        assert_eq!((rebound, resent.len()), (bound, 0));
    }

    /// Scrolling through more packs than slots: every icon drawn gets a slot, each one is sent
    /// once on the way down, and a frame that draws what the last one drew sends nothing.
    #[test]
    fn scrolling_sends_only_the_icons_that_come_into_view() {
        let atlas = atlas();
        let m = Measure(&atlas.font);
        let mut screen = screen(&m, 70, 330);
        let mut seen = std::collections::HashSet::new();
        let mut sent = 0;
        // 150 wheel steps of 18 are more than the 2520 high list.
        for step in 0..150 {
            let (drawn, bound, patches) = frame(&screen, &atlas);
            assert_eq!(bound.len(), drawn.len(), "step {step}: none falls back");
            let new = drawn.iter().filter(|t| seen.insert((*t).clone())).count();
            assert_eq!(patches.len(), new, "step {step}");
            sent += patches.len();
            assert_eq!(frame(&screen, &atlas).2.len(), 0, "step {step}: same view");
            screen.scroll(100.0, 200.0, -1.0);
        }
        assert_eq!((seen.len(), sent), (70, 70));
    }

    /// The slot of the icon drawn longest ago is the one taken, and the icon that left it is
    /// sent again when it comes back.
    #[test]
    fn the_icon_drawn_longest_ago_gives_up_its_slot() {
        let icons: Vec<Arc<[u8]>> = packs(PACK_ICON_SLOTS + 1)
            .into_iter()
            .map(|p| p.icon.unwrap())
            .collect();
        let mut slots = IconSlots::default();
        let key = |i: usize| format!("k{i}");
        slots.begin(&atlas());
        for i in 0..PACK_ICON_SLOTS {
            assert_eq!(slots.slot_for(&key(i), &icons[i]), Some((i, true)));
        }
        // The next frame draws icon 0 first, then a new one: slot 1, the oldest left, is taken.
        slots.frame += 1;
        assert_eq!(slots.slot_for(&key(0), &icons[0]), Some((0, false)));
        assert_eq!(
            slots.slot_for("new", &icons[PACK_ICON_SLOTS]),
            Some((1, true))
        );
        // Icon 1 comes back: it is sent again, into the slot of the next oldest.
        assert_eq!(slots.slot_for(&key(1), &icons[1]), Some((2, true)));
    }

    /// New pixels under the same key take the same slot and are sent; a new atlas holds none
    /// of the icons, so all are sent again.
    #[test]
    fn changed_pixels_and_a_new_atlas_send_again() {
        let atlas = atlas();
        let mut screen = screen(&Measure(&atlas.font), 3, 400);
        let (_, bound, patches) = frame(&screen, &atlas);
        assert_eq!(patches.len(), 3);
        let key = icon_texture("file/pack001");
        screen
            .icons
            .insert(key, vec![200; patches[1].rgba.len()].into());
        let (_, rebound, resent) = frame(&screen, &atlas);
        assert_eq!(rebound, bound);
        assert_eq!(resent.len(), 1);
        assert_eq!((resent[0].x, resent[0].y), (patches[1].x, patches[1].y));
        assert_eq!(resent[0].rgba[0], 200);
        let other = self::atlas();
        assert_ne!(other.revision, atlas.revision);
        assert_eq!(frame(&screen, &other).2.len(), 3);
    }
}
