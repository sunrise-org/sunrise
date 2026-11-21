//! `ConfirmScreen`: a title, a message wrapped to `width - 50` (15 lines at
//! most) and two 150-wide buttons 4 apart and 16 below it, the whole column
//! centred; Escape answers no.
use crate::{
    font::Font,
    hud::{HudAtlas, HudBatch},
    text::Text,
    ui,
};

pub struct Confirm {
    /// Each line and where it is drawn.
    pub lines: Vec<(Text, f32, f32)>,
    /// The yes and no buttons' corners.
    pub buttons: [(f32, f32); 2],
    pub focus: Option<usize>,
    /// `declare_no` was called.
    declares_no: bool,
}
impl Confirm {
    pub fn new(font: &Font, size: (f32, f32), title: &Text, message: &Text) -> Self {
        let Choice { lines, buttons, .. } = Choice::new(font, size, title, message, 2, 150.0);
        Self {
            lines,
            buttons: [buttons[0], buttons[1]],
            focus: None,
            declares_no: false,
        }
    }
    /// Tab cycles focus between yes (0) and no (1).
    pub fn tab(&mut self, reverse: bool) {
        self.focus = match self.focus {
            Some(0) => Some(1),
            Some(1) => Some(0),
            None => Some(if reverse { 1 } else { 0 }),
            _ => Some(0),
        };
    }
    /// Left or right arrow key focuses yes or no.
    pub fn arrow(&mut self, left: bool) {
        self.focus = Some(if left { 0 } else { 1 });
    }
    /// The answer of the currently focused button.
    pub fn answer(&self) -> Option<bool> {
        match self.focus {
            Some(0) => Some(true),
            Some(1) => Some(false),
            _ => None,
        }
    }
    /// The answer a click at `cursor` gives: true for yes, false for no.
    pub fn answer_at(&self, cursor: (f32, f32)) -> Option<bool> {
        let [yes, no] = self.buttons;
        let on = |(x, y): (f32, f32)| ui::hovered(cursor, x, y, 150.0, 20.0);
        if on(yes) {
            Some(true)
        } else if on(no) {
            Some(false)
        } else {
            None
        }
    }
    /// The lines and the buttons (yes first), over whatever is behind.
    pub fn draw(
        &self,
        batch: &mut HudBatch,
        atlas: &HudAtlas,
        cursor: (f32, f32),
        labels: [&Text; 2],
    ) {
        self.draw_active(batch, atlas, cursor, labels, true);
    }
    /// The second button answers no, as Escape does (`widgets::Action`), once drawn.
    pub fn declare_no(&mut self) {
        self.declares_no = true;
    }
    /// `draw` with the buttons greyed out while `setDelay` runs (`active` false).
    pub fn draw_active(
        &self,
        batch: &mut HudBatch,
        atlas: &HudAtlas,
        cursor: (f32, f32),
        labels: [&Text; 2],
        active: bool,
    ) {
        for (line, x, y) in &self.lines {
            batch.text_styled(atlas, line, *x, *y, u32::MAX, true);
        }
        for (i, ((bx, by), label)) in self.buttons.into_iter().zip(labels).enumerate() {
            let is_focused = self.focus == Some(i);
            let hover = active && (ui::hovered(cursor, bx, by, 150.0, 20.0) || is_focused);
            if self.declares_no && i == 1 {
                let action = crate::widgets::Action::Cancel;
                ui::action_button(
                    batch, atlas, action, bx, by, 150.0, 20.0, label, active, hover,
                );
            } else {
                ui::button(batch, atlas, bx, by, 150.0, 20.0, label, active, hover);
            }
        }
    }
}

/// `ConfirmScreen` with a row of `count` buttons `width` wide, 4 apart: `ConfirmLinkScreen`'s
/// yes, copy and no are three of 100. `Confirm` is its two of 150.
pub struct Choice {
    /// Each line and where it is drawn.
    pub lines: Vec<(Text, f32, f32)>,
    /// The buttons' corners, left to right.
    pub buttons: Vec<(f32, f32)>,
    pub width: f32,
    pub focus: Option<usize>,
    /// The button `declare_no` named.
    no_button: Option<usize>,
}
impl Choice {
    pub fn new(
        font: &Font,
        (w, h): (f32, f32),
        title: &Text,
        message: &Text,
        count: usize,
        width: f32,
    ) -> Self {
        let message: Vec<Text> = font
            .split_lines(message, w - 50.0)
            .into_iter()
            .map(|(line, _)| line)
            .take(15)
            .collect();
        let tw = font.width_text(title);
        let mw = message
            .iter()
            .map(|l| font.width_text(l))
            .fold(0.0, f32::max);
        let mh = 9.0 * message.len() as f32;
        let row = width * count as f32 + 4.0 * count.saturating_sub(1) as f32;
        let col = tw.max(mw).max(row);
        let total = 9.0 + 8.0 + mh + 8.0 + 36.0;
        let (lx, ly) = (((w - col) / 2.0).trunc(), ((h - total) / 2.0).trunc());
        let cell = |cw: f32| lx + ((col - cw) / 2.0).trunc();
        let mid = cell(mw) + (mw / 2.0).trunc();
        let mut lines = vec![(title.clone(), cell(tw), ly)];
        for (i, line) in message.into_iter().enumerate() {
            let x = mid - (font.width_text(&line) / 2.0).trunc();
            lines.push((line, x, ly + 17.0 + 9.0 * i as f32));
        }
        let (bx, by) = (cell(row), ly + 9.0 + 8.0 + mh + 8.0 + 16.0);
        Self {
            lines,
            buttons: (0..count)
                .map(|i| (bx + (width + 4.0) * i as f32, by))
                .collect(),
            width,
            focus: None,
            no_button: None,
        }
    }
    /// Tab cycles focus across the buttons.
    pub fn tab(&mut self, reverse: bool) {
        let count = self.buttons.len();
        if count == 0 {
            return;
        }
        self.focus = match self.focus {
            Some(cur) => {
                if reverse {
                    Some(if cur == 0 { count - 1 } else { cur - 1 })
                } else {
                    Some((cur + 1) % count)
                }
            }
            None => Some(if reverse { count - 1 } else { 0 }),
        };
    }
    /// Left or right arrow key moves focus.
    pub fn arrow(&mut self, left: bool) {
        let count = self.buttons.len();
        if count == 0 {
            return;
        }
        self.focus = match self.focus {
            Some(cur) => {
                if left {
                    Some(cur.saturating_sub(1))
                } else {
                    Some((cur + 1).min(count - 1))
                }
            }
            None => Some(if left { 0 } else { count - 1 }),
        };
    }
    /// The index of the currently focused button.
    pub fn pressed(&self) -> Option<usize> {
        self.focus
    }
    /// The button a click at `cursor` hits, counted from the left.
    pub fn pressed_at(&self, cursor: (f32, f32)) -> Option<usize> {
        self.buttons
            .iter()
            .position(|&(x, y)| ui::hovered(cursor, x, y, self.width, 20.0))
    }
    /// Button `i` answers no, as Escape does (`widgets::Action`), once drawn.
    pub fn declare_no(&mut self, i: usize) {
        self.no_button = Some(i);
    }
    /// The lines and the buttons, over whatever is behind.
    pub fn draw(
        &self,
        batch: &mut HudBatch,
        atlas: &HudAtlas,
        cursor: (f32, f32),
        labels: &[Text],
    ) {
        for (line, x, y) in &self.lines {
            batch.text_styled(atlas, line, *x, *y, u32::MAX, true);
        }
        for (i, (&(bx, by), label)) in self.buttons.iter().zip(labels).enumerate() {
            let is_focused = self.focus == Some(i);
            let hover = ui::hovered(cursor, bx, by, self.width, 20.0) || is_focused;
            if self.no_button == Some(i) {
                let action = crate::widgets::Action::Cancel;
                ui::action_button(
                    batch, atlas, action, bx, by, self.width, 20.0, label, true, hover,
                );
            } else {
                ui::button(batch, atlas, bx, by, self.width, 20.0, label, true, hover);
            }
        }
    }
}

/// A confirm screen's laid-out text (title, message, extra warning lines) and
/// button corners; the whole column centred (ConfirmScreen.init).
pub struct Layout {
    pub lines: Vec<(Text, f32, f32)>,
    pub buttons: Vec<(f32, f32)>,
}
/// `ConfirmScreen.init`: title, message wrapped to `width - 50` (15 rows at
/// most), the extra lines, then a button row `spacing` 4 with `paddingTop` 16.
/// `FrameLayout.centerInRectangle` centres the column; each cell is then centred
/// in the column's width, and the message's lines in the message widget's.
pub fn layout(
    font: &Font,
    (w, h): (f32, f32),
    title: &Text,
    message: &Text,
    extra: &[Text],
    button_widths: &[f32],
) -> Layout {
    let message: Vec<Text> = font
        .split_lines(message, w - 50.0)
        .into_iter()
        .take(15)
        .map(|(line, _)| line)
        .collect();
    let tw = font.width_text(title);
    let mw = message
        .iter()
        .map(|l| font.width_text(l))
        .fold(0.0, f32::max);
    let row_w: f32 =
        button_widths.iter().sum::<f32>() + 4.0 * button_widths.len().saturating_sub(1) as f32;
    let col = extra
        .iter()
        .map(|l| font.width_text(l))
        .fold(tw.max(mw).max(row_w), f32::max);
    // LinearLayout vertical spacing 8; the button row's children pad 16 on top.
    let extra_h = 17.0 * extra.len() as f32;
    let mh = 9.0 * message.len() as f32;
    let total = 9.0 + 8.0 + mh + extra_h + 8.0 + 16.0 + 20.0;
    let lx = ((w - col) / 2.0).trunc();
    let cell = |cw: f32| lx + ((col - cw) / 2.0).trunc();
    let mut y = ((h - total) / 2.0).trunc();
    let mut lines = vec![(title.clone(), cell(tw), y)];
    y += 17.0;
    let message_x = cell(mw);
    for line in &message {
        let x = message_x + ((mw - font.width_text(line)) / 2.0).trunc();
        lines.push((line.clone(), x, y));
        y += 9.0;
    }
    for line in extra {
        y += 8.0;
        lines.push((line.clone(), cell(font.width_text(line)), y));
        y += 9.0;
    }
    let by = y + 8.0 + 16.0;
    let mut bx = cell(row_w);
    let buttons = button_widths
        .iter()
        .map(|bw| {
            let p = (bx, by);
            bx += bw + 4.0;
            p
        })
        .collect();
    Layout { lines, buttons }
}
/// `ConfirmLinkScreen`: `chat.link.confirm` (`confirmTrusted` when trusted),
/// the link as the message, and for an untrusted link `chat.link.warning`
/// in its red (0xFFCCCC); yes, copy and no are 100 wide.
pub fn link_layout(font: &Font, size: (f32, f32), url: &str, trusted: bool) -> Layout {
    let warning = [Text::translate("chat.link.warning", &[]).with_color(0xFFCCCC)];
    layout(
        font,
        size,
        &Text::translate(
            if trusted {
                "chat.link.confirmTrusted"
            } else {
                "chat.link.confirm"
            },
            &[],
        ),
        &Text::literal(url.to_owned()),
        if trusted { &[] } else { &warning },
        &[100.0; 3],
    )
}
/// `ConfirmLinkScreen`'s yes, copy and no labels: Open in Browser and Cancel
/// when trusted.
pub fn link_labels(trusted: bool) -> [&'static str; 3] {
    if trusted {
        ["chat.link.open", "gui.copy_link_to_clipboard", "gui.cancel"]
    } else {
        ["gui.yes", "gui.copy_link_to_clipboard", "gui.no"]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hud::Rect;
    use std::collections::HashMap;

    fn font() -> Font {
        // The missing-glyph metrics let these tests run without game assets.
        let rect = Rect {
            u0: 0.0,
            v0: 0.0,
            u1: 1.0,
            v1: 1.0,
            w: 1,
            h: 1,
        };
        Font::load(
            &sunrise_assets::resource_source::ResourceSource::default(),
            &HashMap::new(),
            &[],
            (1, 1),
            (0, 0),
            rect,
        )
    }

    #[test]
    fn three_buttons_sit_four_apart_in_the_middle() {
        let (title, message) = (Text::literal("Open?"), Text::literal("https://x"));
        let c = Choice::new(&font(), (480.0, 270.0), &title, &message, 3, 100.0);
        let xs: Vec<f32> = c.buttons.iter().map(|b| b.0).collect();
        // 3 x 100 + 2 x 4 = 308 wide, centred on 480.
        assert_eq!(xs, vec![86.0, 190.0, 294.0]);
        assert_eq!(c.pressed_at((191.0, c.buttons[1].1 + 1.0)), Some(1));
        assert_eq!(c.pressed_at((188.0, c.buttons[1].1 + 1.0)), None);
        // Confirm keeps its two 150-wide buttons where they were.
        let two = Confirm::new(&font(), (480.0, 270.0), &title, &message);
        assert_eq!(two.buttons[0].0, 88.0);
        assert_eq!(two.buttons[1].0, 242.0);
    }

    #[test]
    fn only_the_button_declared_no_gets_its_job() {
        use crate::widgets::{self, Action, DeclaredAction};
        let atlas = HudAtlas::build(&crate::resource_source::ResourceSource::indexed(
            Default::default(),
        ))
        .unwrap();
        let (title, message) = (Text::literal("Title"), Text::literal("Message"));
        let labels = [
            Text::literal("Yes"),
            Text::literal("Copy"),
            Text::literal("No"),
        ];
        let drawn = |frame: u64, draw: &dyn Fn(&mut HudBatch)| {
            widgets::set_action_screen(7);
            draw(&mut HudBatch::scratch([480, 270]));
            widgets::end_frame(frame);
            widgets::last_actions(7)
        };
        let no = |rect| {
            vec![DeclaredAction {
                action: Action::Cancel,
                rect,
                enabled: true,
            }]
        };

        let mut confirm = Confirm::new(&font(), (480.0, 270.0), &title, &message);
        let draw_confirm = |confirm: &Confirm, batch: &mut HudBatch| {
            confirm.draw(batch, &atlas, (0.0, 0.0), [&labels[0], &labels[2]]);
        };
        assert!(drawn(1, &|b| draw_confirm(&confirm, b)).is_empty());
        confirm.declare_no();
        let (x, y) = confirm.buttons[1];
        assert_eq!(
            drawn(2, &|b| draw_confirm(&confirm, b)),
            no([x, y, 150.0, 20.0])
        );

        let mut choice = Choice::new(&font(), (480.0, 270.0), &title, &message, 3, 100.0);
        let draw_choice = |choice: &Choice, batch: &mut HudBatch| {
            choice.draw(batch, &atlas, (0.0, 0.0), &labels);
        };
        assert!(drawn(3, &|b| draw_choice(&choice, b)).is_empty());
        choice.declare_no(2);
        let (x, y) = choice.buttons[2];
        assert_eq!(
            drawn(4, &|b| draw_choice(&choice, b)),
            no([x, y, 100.0, 20.0])
        );
    }

    #[test]
    fn confirm_and_choice_keyboard_navigation() {
        let (title, message) = (Text::literal("Title"), Text::literal("Message"));
        let mut confirm = Confirm::new(&font(), (480.0, 270.0), &title, &message);
        assert_eq!(confirm.focus, None);
        assert_eq!(confirm.answer(), None);

        confirm.tab(false);
        assert_eq!(confirm.focus, Some(0));
        assert_eq!(confirm.answer(), Some(true));

        confirm.tab(false);
        assert_eq!(confirm.focus, Some(1));
        assert_eq!(confirm.answer(), Some(false));

        confirm.tab(true);
        assert_eq!(confirm.focus, Some(0));

        confirm.arrow(false);
        assert_eq!(confirm.focus, Some(1));
        confirm.arrow(true);
        assert_eq!(confirm.focus, Some(0));

        let mut choice = Choice::new(&font(), (480.0, 270.0), &title, &message, 3, 100.0);
        assert_eq!(choice.focus, None);
        assert_eq!(choice.pressed(), None);

        choice.tab(false);
        assert_eq!(choice.focus, Some(0));
        assert_eq!(choice.pressed(), Some(0));

        choice.tab(false);
        assert_eq!(choice.focus, Some(1));

        choice.tab(false);
        assert_eq!(choice.focus, Some(2));

        choice.tab(false);
        assert_eq!(choice.focus, Some(0));

        choice.arrow(false);
        assert_eq!(choice.focus, Some(1));
        choice.arrow(true);
        assert_eq!(choice.focus, Some(0));
    }

    #[test]
    fn a_trusted_link_asks_with_open_in_browser_and_cancel() {
        // ConfirmLinkScreen: trusted swaps yes/no for chat.link.open / gui.cancel.
        assert_eq!(
            link_labels(true),
            ["chat.link.open", "gui.copy_link_to_clipboard", "gui.cancel"]
        );
        assert_eq!(
            link_labels(false),
            ["gui.yes", "gui.copy_link_to_clipboard", "gui.no"]
        );
    }
}
