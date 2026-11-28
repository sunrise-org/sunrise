//! The Players tab's scrolling rows and their UUID-based actions.

use super::{ConfigureTab, GREY, HudAtlas, HudBatch, Outcome, PlayerInfo, Screen, SubScreen, Text};
use crate::{ui, widgets};
use std::cell::{Cell, RefCell};
use winit::keyboard::KeyCode;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PlayerAction {
    Op(u128),
    Deop(u128),
    Remove(u128),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PlayerColumn {
    Operator,
    Remove,
}

impl PlayerAction {
    pub(super) fn uuid(self) -> u128 {
        match self {
            Self::Op(id) | Self::Deop(id) | Self::Remove(id) => id,
        }
    }

    fn column(self) -> PlayerColumn {
        match self {
            Self::Op(_) | Self::Deop(_) => PlayerColumn::Operator,
            Self::Remove(_) => PlayerColumn::Remove,
        }
    }

    fn style(self) -> usize {
        match self {
            Self::Op(_) => 0,
            Self::Deop(_) => 1,
            Self::Remove(_) => 2,
        }
    }
}

impl PlayerColumn {
    fn action(self, id: u128, operator: bool) -> PlayerAction {
        match self {
            Self::Operator if operator => PlayerAction::Deop(id),
            Self::Operator => PlayerAction::Op(id),
            Self::Remove => PlayerAction::Remove(id),
        }
    }
}

pub(super) fn player_uuid(player: &PlayerInfo) -> Option<u128> {
    player
        .uuid
        .as_deref()
        .and_then(super::auth::parse_uuid)
        .filter(|&id| id != 0)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Focus {
    Header,
    Player(u128, PlayerColumn),
}

#[derive(Clone, Debug, Default)]
pub(super) struct PlayersList {
    frame: RefCell<Option<Frame>>,
    focus: Option<(i64, Focus)>,
    metrics: Cell<Option<([f32; 3], f32)>>,
}

impl PlayersList {
    pub(super) fn clear_frame(&self) {
        self.frame.borrow_mut().take();
    }
}

/// Mouse hits use the clipped rectangles and row order from the last draw.
#[derive(Clone, Debug)]
struct Frame {
    realm: i64,
    view: View,
    offset: f32,
    heading: Option<[f32; 4]>,
    controls: Vec<Control>,
}

#[derive(Clone, Copy, Debug)]
struct Control {
    rect: [f32; 4],
    row: usize,
    action: PlayerAction,
}

const TOP: f32 = 25.0;
const WIDTH: f32 = 300.0;
const HEADER: f32 = 17.0;
const ROW: f32 = 36.0;
const PADDING: f32 = 2.0;
const LABELS: [&str; 3] = [
    "mco.configure.world.invites.normal.tooltip",
    "mco.configure.world.invites.ops.tooltip",
    "mco.configure.world.invites.remove.tooltip",
];
const ICONS: [&str; 3] = [
    "gui/sprites/player_list/make_operator",
    "gui/sprites/player_list/remove_operator",
    "gui/sprites/player_list/remove_player",
];

#[derive(Clone, Copy, Debug, PartialEq)]
struct View {
    width: f32,
    bottom: f32,
    left: f32,
    content: f32,
}

impl View {
    fn new((w, h): (f32, f32), rows: usize) -> Option<Self> {
        let width = w.ceil();
        let bottom = h.ceil() - 68.0;
        (bottom > TOP).then_some(Self {
            width,
            bottom,
            left: (width / 2.0).floor() - WIDTH / 2.0,
            content: 2.0 * PADDING + HEADER + rows as f32 * ROW,
        })
    }

    fn offset(self, mut scroll: ui::Scroll) -> f32 {
        scroll.clamp(TOP, self.bottom, self.content);
        scroll.amount.trunc()
    }

    fn row_y(self, row: usize, offset: f32) -> f32 {
        TOP + PADDING + HEADER + row as f32 * ROW - offset
    }

    fn clip(self, [x, y, w, h]: [f32; 4]) -> Option<[f32; 4]> {
        let left = x.max(0.0);
        let top = y.max(TOP);
        let right = (x + w).min(self.width);
        let bottom = (y + h).min(self.bottom);
        (right > left && bottom > top).then_some([left, top, right - left, bottom - top])
    }

    fn scrollbar_x(self) -> f32 {
        self.left + WIDTH + 8.0
    }

    fn has_scrollbar_space(self) -> bool {
        self.bottom - TOP >= 40.0
    }
}

fn hits(point: (f32, f32), [x, y, w, h]: [f32; 4]) -> bool {
    ui::hovered(point, x, y, w, h)
}

impl Screen {
    /// The configuration screen attaches the tab's widgets after its initial fetches.
    pub(crate) fn players_ready(&self) -> bool {
        matches!(
            self.subscreen,
            SubScreen::Configure {
                tab: ConfigureTab::Players,
                detail: Some(Ok(_)),
                subscription: Some(Ok(_)),
                regions: Some(Ok(_)),
                tier: Some(Ok(_)),
                ..
            }
        )
    }

    pub(crate) fn players_tab_ready(&self) -> bool {
        self.players_ready()
    }

    pub(crate) fn players_owns_keyboard(&self) -> bool {
        self.players_ready()
    }

    fn player_rows(&self) -> Option<(i64, &[PlayerInfo], ui::Scroll, bool)> {
        if !self.players_ready() {
            return None;
        }
        let SubScreen::Configure {
            server,
            detail: Some(Ok(detail)),
            players_scroll,
            detail_job,
            member_job,
            switching_slot,
            ..
        } = &self.subscreen
        else {
            return None;
        };
        Some((
            server.id,
            &detail.players,
            *players_scroll,
            detail_job.is_none() && member_job.is_none() && switching_slot.is_none(),
        ))
    }

    pub(super) fn player_actions_enabled(&self) -> bool {
        self.player_rows().is_some_and(|(_, _, _, active)| active)
    }

    pub(super) fn invalidate_players_frame(&self) {
        self.players_list.clear_frame();
    }

    pub(crate) fn clear_players_focus(&mut self) {
        self.players_list.focus = None;
    }

    fn player_focus(&self) -> Option<Focus> {
        let (realm, players, _, _) = self.player_rows()?;
        let (_, focus) = self.players_list.focus.filter(|(id, _)| *id == realm)?;
        match focus {
            Focus::Player(uuid, _) if !players.iter().any(|p| player_uuid(p) == Some(uuid)) => None,
            _ => Some(focus),
        }
    }

    pub(crate) fn players_list_focused(&self) -> bool {
        self.player_focus().is_some()
    }

    pub(crate) fn focused_player_column(&self) -> Option<PlayerColumn> {
        match self.player_focus()? {
            Focus::Header => None,
            Focus::Player(_, column) => Some(column),
        }
    }

    pub(crate) fn player_action_focused(&self) -> bool {
        self.focused_player_column().is_some()
    }

    pub(crate) fn player_control_at(
        &self,
        point: (f32, f32),
        size: (f32, f32),
    ) -> Option<PlayerAction> {
        let (realm, players, scroll, active) = self.player_rows()?;
        let view = View::new(size, players.len())?;
        let frame = self.players_list.frame.borrow();
        let frame = frame.as_ref().filter(|f| {
            active && f.realm == realm && f.view == view && f.offset == view.offset(scroll)
        })?;
        let control = frame.controls.iter().find(|c| hits(point, c.rect))?;
        let player = players.get(control.row)?;
        let uuid = player_uuid(player)?;
        // A reply or reorder between draw and click can invalidate the visible control.
        (control.action.column().action(uuid, player.operator) == control.action)
            .then_some(control.action)
    }

    fn displayed_player_action(&self) -> Option<PlayerAction> {
        let (_, players, _, _) = self.player_rows()?;
        let Focus::Player(uuid, column) = self.player_focus()? else {
            return None;
        };
        let player = players.iter().find(|p| player_uuid(p) == Some(uuid))?;
        let action = column.action(uuid, player.operator);
        self.player_action_drawn(action).then_some(action)
    }

    pub(crate) fn activate_player_focus(&mut self) -> Option<Outcome> {
        let action = self.displayed_player_action()?;
        Some(self.click_player_action(action))
    }

    pub(crate) fn focus_player_at(
        &mut self,
        point: (f32, f32),
        size: (f32, f32),
    ) -> Option<[f32; 4]> {
        let (realm, players, scroll, _) = self.player_rows()?;
        let view = View::new(size, players.len())?;
        let action = self.player_control_at(point, size);
        let (focus, rect) = {
            let frame = self.players_list.frame.borrow();
            let frame = frame.as_ref().filter(|f| {
                f.realm == realm && f.view == view && f.offset == view.offset(scroll)
            })?;
            if let Some(action) = action {
                let control = frame
                    .controls
                    .iter()
                    .find(|c| c.action == action && hits(point, c.rect))?;
                (Focus::Player(action.uuid(), action.column()), control.rect)
            } else {
                let rect = frame.heading.filter(|&r| hits(point, r))?;
                (Focus::Header, rect)
            }
        };
        self.players_list.focus = Some((realm, focus));
        Some(rect)
    }

    pub(crate) fn focused_player_rect(&self, size: (f32, f32)) -> Option<[f32; 4]> {
        let (realm, players, scroll, _) = self.player_rows()?;
        let view = View::new(size, players.len())?;
        let frame = self.players_list.frame.borrow();
        let frame = frame
            .as_ref()
            .filter(|f| f.realm == realm && f.view == view && f.offset == view.offset(scroll))?;
        match self.player_focus()? {
            Focus::Header => frame.heading,
            Focus::Player(..) => {
                let action = self.displayed_player_action()?;
                frame
                    .controls
                    .iter()
                    .find(|c| c.action == action)
                    .map(|c| c.rect)
            }
        }
    }

    /// `ContainerObjectSelectionList`: the count is the first stop and vertical arrows
    /// stay in the same child column as they move through the invited rows.
    pub(crate) fn move_player_focus(&mut self, key: KeyCode, size: (f32, f32)) -> Option<[f32; 4]> {
        use KeyCode::{ArrowDown, ArrowLeft, ArrowRight, ArrowUp};
        use PlayerColumn::{Operator, Remove};
        self.players_list.metrics.get()?;
        self.navigate_players(size, |rows, focus| match (key, focus) {
            (ArrowDown, Some(None)) if rows > 0 => Some(Some((0, Operator))),
            (ArrowDown, None) if rows > 0 => Some(Some((0, Operator))),
            (ArrowUp, Some(Some((0, _)))) => Some(None),
            (ArrowUp, Some(Some((i, column)))) => Some(Some((i - 1, column))),
            (ArrowDown, Some(Some((i, column)))) if i + 1 < rows => Some(Some((i + 1, column))),
            (ArrowRight, Some(Some((i, Operator)))) => Some(Some((i, Remove))),
            (ArrowLeft, Some(Some((i, Remove)))) => Some(Some((i, Operator))),
            _ => None,
        })
        .and_then(|()| self.focus_rect_from_state(size))
    }

    pub(crate) fn tab_player_focus(&mut self, reverse: bool, size: (f32, f32)) -> bool {
        use PlayerColumn::{Operator, Remove};
        self.navigate_players(size, |rows, focus| match (reverse, focus) {
            (false, None) => Some(None),
            (false, Some(None)) if rows > 0 => Some(Some((0, Operator))),
            (false, Some(Some((row, Operator)))) => Some(Some((row, Remove))),
            (false, Some(Some((row, Remove)))) if row + 1 < rows => Some(Some((row + 1, Operator))),
            (true, Some(Some((row, Remove)))) => Some(Some((row, Operator))),
            (true, Some(Some((row, Operator)))) => {
                Some(row.checked_sub(1).map(|row| (row, Remove)))
            }
            _ => None,
        })
        .and_then(|()| self.focus_rect_from_state(size))
        .is_some()
    }

    pub(crate) fn enter_player_focus(
        &mut self,
        from_bottom: bool,
        column: PlayerColumn,
        size: (f32, f32),
    ) -> bool {
        self.navigate_players(size, |rows, _| {
            Some(
                rows.checked_sub(1)
                    .filter(|_| from_bottom)
                    .map(|row| (row, column)),
            )
        })
        .and_then(|()| self.focus_rect_from_state(size))
        .is_some()
    }

    pub(super) fn player_action_drawn(&self, action: PlayerAction) -> bool {
        let Some((realm, _, scroll, true)) = self.player_rows() else {
            return false;
        };
        self.players_list
            .frame
            .borrow()
            .as_ref()
            .is_some_and(|frame| {
                frame.realm == realm
                    && frame.offset == frame.view.offset(scroll)
                    && frame
                        .controls
                        .iter()
                        .any(|control| control.action == action)
            })
    }

    fn focus_rect_from_state(&self, size: (f32, f32)) -> Option<[f32; 4]> {
        let (_, players, scroll, _) = self.player_rows()?;
        let view = View::new(size, players.len())?;
        let offset = view.offset(scroll);
        let (widths, heading_width) = self.players_list.metrics.get()?;
        match self.player_focus()? {
            Focus::Header => view.clip([
                view.left + WIDTH / 2.0 - (heading_width / 2.0).floor(),
                TOP + PADDING - offset,
                heading_width,
                HEADER,
            ]),
            Focus::Player(uuid, column) => {
                let (row, player) = players
                    .iter()
                    .enumerate()
                    .find(|(_, p)| player_uuid(p) == Some(uuid))?;
                let action = column.action(uuid, player.operator);
                let style = action.style();
                let remove_x = view.left + WIDTH - PADDING - widths[2];
                let x = if column == PlayerColumn::Operator {
                    remove_x - 8.0 - widths[style]
                } else {
                    remove_x
                };
                view.clip([x, view.row_y(row, offset) + 8.0, widths[style], 20.0])
            }
        }
    }

    fn navigate_players(
        &mut self,
        size: (f32, f32),
        next: impl FnOnce(
            usize,
            Option<Option<(usize, PlayerColumn)>>,
        ) -> Option<Option<(usize, PlayerColumn)>>,
    ) -> Option<()> {
        let Some((realm, players, scroll, active)) = self.player_rows() else {
            return None;
        };
        let Some(view) = View::new(size, players.len()) else {
            return None;
        };
        let rows: Vec<_> = players
            .iter()
            .enumerate()
            .filter_map(|(i, p)| Some((i, player_uuid(p)?)))
            .filter(|_| active)
            .collect();
        let current = match self.player_focus() {
            Some(Focus::Header) => Some(None),
            Some(Focus::Player(uuid, column)) => rows
                .iter()
                .position(|&(_, id)| id == uuid)
                .map(|i| Some((i, column))),
            None => None,
        };
        let Some(next) = next(rows.len(), current) else {
            return None;
        };
        let (focus, top, height) = match next {
            None => (Focus::Header, TOP + PADDING, HEADER),
            Some((i, column)) => {
                let (row, id) = rows[i];
                (Focus::Player(id, column), view.row_y(row, 0.0), ROW)
            }
        };
        self.players_list.focus = Some((realm, focus));
        if let SubScreen::Configure { players_scroll, .. } = &mut self.subscreen {
            let mut offset = view.offset(scroll);
            let top = top - offset;
            if top < TOP + PADDING {
                offset += top - TOP - PADDING;
            } else if top + height > view.bottom - PADDING {
                offset += top + height - view.bottom + PADDING;
            }
            players_scroll.amount = offset;
            players_scroll.clamp(TOP, view.bottom, view.content);
        }
        Some(())
    }

    pub(super) fn press_players(
        &mut self,
        point: (f32, f32),
        size: (f32, f32),
    ) -> Option<(Outcome, bool)> {
        let (_, players, _, _) = self.player_rows()?;
        let view = View::new(size, players.len())?;
        if let SubScreen::Configure { players_scroll, .. } = &mut self.subscreen
            && view.has_scrollbar_space()
            && players_scroll.press(point, view.scrollbar_x(), TOP, view.bottom, view.content)
        {
            return Some((Outcome::None, false));
        }
        let action = self.player_control_at(point, size);
        if self.focus_player_at(point, size).is_some() {
            return Some((
                action.map_or(Outcome::None, |a| self.click_player_action(a)),
                true,
            ));
        }
        self.clear_players_focus();
        None
    }

    pub(super) fn drag_players(&mut self, y: f32, size: (f32, f32)) -> bool {
        let Some((_, players, scroll, _)) = self.player_rows() else {
            return false;
        };
        if !scroll.held() {
            return false;
        }
        if let Some(view) = View::new(size, players.len())
            && view.has_scrollbar_space()
            && let SubScreen::Configure { players_scroll, .. } = &mut self.subscreen
        {
            players_scroll.drag(y, TOP, view.bottom, view.content);
            self.invalidate_players_frame();
        }
        true
    }

    pub(super) fn wheel_players(
        &mut self,
        delta: f32,
        point: (f32, f32),
        size: (f32, f32),
    ) -> bool {
        let Some((_, players, _, _)) = self.player_rows() else {
            return false;
        };
        if let Some(view) = View::new(size, players.len())
            && (0.0..view.width).contains(&point.0)
            && (TOP..view.bottom).contains(&point.1)
            && let SubScreen::Configure { players_scroll, .. } = &mut self.subscreen
        {
            players_scroll.wheel(delta, ROW / 2.0, TOP, view.bottom, view.content);
            self.invalidate_players_frame();
            return true;
        }
        false
    }

    pub(super) fn draw_players(
        &self,
        batch: &mut HudBatch,
        atlas: &HudAtlas,
        cursor: (f32, f32),
        size: (f32, f32),
        show_focus: bool,
    ) {
        self.invalidate_players_frame();
        let Some((realm, players, scroll, active)) = self.player_rows() else {
            return;
        };
        let Some(view) = View::new(size, players.len()) else {
            return;
        };
        let offset = view.offset(scroll);
        let focus = show_focus.then(|| self.player_focus()).flatten();
        let labels = LABELS.map(crate::lang::get);
        let widths = labels
            .each_ref()
            .map(|label| 16.0 + HudBatch::text_width(atlas, label));
        let heading = Text::translate(
            "mco.configure.world.invited.number",
            &[Text::literal(players.len().to_string())],
        )
        .underlined();
        let heading_width = HudBatch::text_width(atlas, &heading.plain()) + 8.0;
        self.players_list.metrics.set(Some((widths, heading_width)));
        let heading_rect @ [hx, hy, hw, hh] = [
            view.left + WIDTH / 2.0 - (heading_width / 2.0).floor(),
            TOP + PADDING - offset,
            heading_width,
            HEADER,
        ];
        let mut frame = Frame {
            realm,
            view,
            offset,
            heading: view.clip(heading_rect),
            controls: Vec::new(),
        };
        let first = batch.vertices.len();
        if let Some(rect) = frame.heading {
            let focused = focus == Some(Focus::Header);
            widgets::record(&heading.plain(), rect, true);
            widgets::narratable(|| {
                widgets::Narratable::text(heading.plain(), rect)
                    .hovered(focused || hits(cursor, rect))
            });
            if focused {
                batch.fill(atlas, hx, hy, hw, hh, u32::MAX);
                batch.fill(atlas, hx + 1.0, hy + 1.0, hw - 2.0, hh - 2.0, 0xFF00_0000);
            }
            batch.text_styled(atlas, &heading, hx + 4.0, hy + 4.0, u32::MAX, true);
        }
        for (row, player) in players.iter().enumerate() {
            let y = view.row_y(row, offset);
            if y + ROW <= TOP {
                continue;
            }
            if y >= view.bottom {
                break;
            }
            let uuid = player_uuid(player);
            #[cfg(not(test))]
            let face = self.face_sprite(
                batch,
                atlas,
                player.uuid.as_deref().filter(|_| uuid.is_some()),
                row,
            );
            #[cfg(test)]
            let face: Option<String> = None;
            batch.sprite_scaled(
                atlas,
                face.as_deref().unwrap_or("misc/unknown_server"),
                view.left + PADDING,
                y + PADDING,
                32.0,
                32.0,
                u32::MAX,
            );
            let color = if !player.accepted {
                GREY
            } else if player.online {
                0xFF00_FF00
            } else {
                u32::MAX
            };
            batch.text(atlas, &player.name, view.left + 42.0, y + 14.0, color, true);
            let op = PlayerColumn::Operator.action(uuid.unwrap_or_default(), player.operator);
            let remove = PlayerAction::Remove(uuid.unwrap_or_default());
            let remove_x = view.left + WIDTH - PADDING - widths[remove.style()];
            let op_x = remove_x - 8.0 - widths[op.style()];
            for (action, x) in [(op, op_x), (remove, remove_x)] {
                let style = action.style();
                let w = widths[style];
                let y = y + 8.0;
                let Some(rect) = view.clip([x, y, w, 20.0]) else {
                    continue;
                };
                let enabled = active && uuid.is_some();
                if enabled {
                    frame.controls.push(Control { rect, row, action });
                }
                let hover = hits(cursor, rect);
                let focused =
                    uuid.is_some_and(|id| focus == Some(Focus::Player(id, action.column())));
                widgets::record(&labels[style], rect, enabled);
                widgets::widget_cursor(hover, enabled);
                widgets::narratable(|| {
                    let player = crate::lang::tr("mco.invited.player.narration", &[&player.name]);
                    widgets::Narratable::button(
                        widgets::join_for_narration(&[&player, &labels[style]]),
                        rect,
                        enabled,
                    )
                    .hovered(hover || focused)
                });
                draw_button(
                    batch,
                    atlas,
                    style,
                    [x, y, w, 20.0],
                    enabled,
                    hover || focused,
                );
            }
        }
        batch.clip_since(first, 0.0, TOP, view.width, view.bottom);
        self.players_list.frame.replace(Some(frame));
        if view.has_scrollbar_space() {
            ui::scrollbar(
                batch,
                atlas,
                view.scrollbar_x(),
                TOP,
                view.bottom,
                view.content,
                offset,
            );
        }
    }
}

/// `SpriteIconButton.TextAndIcon`: reserve the trailing space for the 8脳7 sprite.
fn draw_button(
    batch: &mut HudBatch,
    atlas: &HudAtlas,
    style: usize,
    [x, y, w, h]: [f32; 4],
    enabled: bool,
    highlighted: bool,
) {
    let sprite = if !enabled {
        "widget/button_disabled"
    } else if highlighted {
        "widget/button_highlighted"
    } else {
        "widget/button"
    };
    ui::nine_slice(batch, atlas, sprite, x, y, w, h, u32::MAX);
    ui::scrolling_text_around_at(
        batch,
        atlas,
        &Text::translate(LABELS[style], &[]),
        x + (w / 2.0).floor(),
        x + 2.0,
        x + w - 12.0,
        y,
        y + h,
        crate::hud_overlays::now_ms(),
        if enabled { u32::MAX } else { GREY },
    );
    batch.sprite_scaled(
        atlas,
        ICONS[style],
        x + w - 10.0,
        y + 7.0,
        8.0,
        7.0,
        u32::MAX,
    );
}

#[cfg(test)]
#[path = "players_list_tests.rs"]
mod tests;
