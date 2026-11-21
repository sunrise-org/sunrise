//! `ServerLinksDialogScreen`: the pause menu's "Server Links..." button opens
//! the `minecraft:server_links` dialog (the `pause_screen_additions` tag's
//! fallback when a server sent ClientboundServerLinksPacket): a scrollable
//! list of the server's links, each opened through `ConfirmLinkScreen`
//! (Screen.clickUrlAction), and the `dialog/warning_button`'s
//! DialogScreen.WarningScreen ("Note about custom screens").
//! https://mcsrc.dev/2/26.3/net/minecraft/client/gui/screens/dialog/ServerLinksDialogScreen
use super::{Game, Screen, packs::open_uri};
use sunrise_client::{
    Result,
    clipboard::Clipboard,
    confirm_screen::{Layout as ConfirmLayout, layout as confirm_layout, link_labels, link_layout},
    hud::HudBatch,
    text::Text,
    ui,
};
use sunrise_protocol::packets::configuration::{ServerLink, ServerLinkKind};

/// HeaderAndFooterLayout's header and footer heights; the content wants to
/// start 30 pixels under the header (its `CONTENTS_MARGIN_TOP`).
const HEADER: f32 = 33.0;
const FOOTER: f32 = 33.0;
const CONTENT_TOP: f32 = HEADER + 30.0;
/// `server_links.json`: `columns` 1, `button_width` 310, `exit_action`'s 200.
const LINK_WIDTH: f32 = 310.0;
/// packControlsIntoColumns: rowSpacing 2 under the 20-pixel buttons.
const ROW: f32 = 22.0;
/// ScrollableLayout's BOTH reserve (scrollbarSpacing 4 + SCROLLBAR_WIDTH 6).
const SCROLL_RESERVE: f32 = 10.0;
/// DialogScreen's `dialog/warning_button` ImageButton.
const WARNING_BUTTON: f32 = 20.0;

/// The ServerLinks screen's state: ScrollableLayout's scroll and the link
/// ConfirmLinkScreen is asking about (`Screen::ConfirmLink`).
#[derive(Default)]
pub(super) struct LinksUi {
    pub scroll: f32,
    pub confirm: Option<String>,
    /// `ConfirmLinkScreen.trusted`: the pause menu's buttons ask with the
    /// trusted text and no warning; a server's link is untrusted.
    pub trusted: bool,
    /// The screen the confirmation returns to (its parent); Server Links when unset.
    pub back: Option<Screen>,
}

/// `ClientCommonPacketListenerImpl.handleServerLinks`: keep only the links
/// that validate, warning about the dropped ones.
pub(super) fn trusted(links: Vec<ServerLink>) -> Vec<ServerLink> {
    links
        .into_iter()
        .filter(|l| {
            // Util.parseAndValidateUntrustedUri: java.net.URI syntax, http or https.
            let ok = sunrise_client::component::untrusted_http_url(&l.link);
            if !ok {
                log::warn!("Sunrise: server sent an invalid link: {}", l.link);
            }
            ok
        })
        .collect()
}
/// `ServerLinks.KnownLinkType` by id.
const KNOWN_LINKS: [&str; 10] = [
    "report_bug",
    "community_guidelines",
    "support",
    "status",
    "feedback",
    "community",
    "website",
    "forums",
    "news",
    "announcements",
];
/// A known link type's name: ByIdMap.ZERO reads an id it doesn't know as `report_bug`.
fn known_link(id: i32) -> &'static str {
    usize::try_from(id)
        .ok()
        .and_then(|i| KNOWN_LINKS.get(i))
        .copied()
        .unwrap_or(KNOWN_LINKS[0])
}
/// `ServerLinks.findKnownType(KnownLinkType.BUG_REPORT)`: the first `report_bug` link.
pub(super) fn bug_report_link(links: &[ServerLink]) -> Option<&str> {
    links
        .iter()
        .find(|l| matches!(l.kind, ServerLinkKind::Known(id) if known_link(id) == KNOWN_LINKS[0]))
        .map(|l| l.link.as_str())
}
/// `ServerLinks.Entry.displayName`: `known_server_link.<name>` for a known
/// type (ByIdMap.ZERO reads unknown ids as `report_bug`), the component else.
fn link_label(link: &ServerLink) -> Text {
    match &link.kind {
        ServerLinkKind::Known(id) => {
            Text::translate(&format!("known_server_link.{}", known_link(*id)), &[])
        }
        ServerLinkKind::Custom(tag) => {
            sunrise_client::scoreboard::comp_text(&sunrise_client::scoreboard::tag_to_json(tag))
        }
    }
}

/// The scrolled column of link buttons: content height and, for a list that
/// fits, the `HeaderAndFooterLayout` contents frame's top.
fn list_height(n: usize) -> f32 {
    n as f32 * ROW - 2.0
}
fn list_top(h: f32, n: usize) -> f32 {
    CONTENT_TOP.min(h - FOOTER - list_height(n).min(h - HEADER - FOOTER))
}
fn scroll_limit(h: f32, n: usize) -> f32 {
    (list_height(n) - (h - HEADER - FOOTER)).max(0.0)
}

/// `ServerLinksScreen`'s buttons as (x, y) over the scrolled content, and the
/// footer `Back` button's corner. `top` includes the scroll.
fn link_buttons(w: f32, top: f32, n: usize) -> Vec<(f32, f32)> {
    let x = ((w - LINK_WIDTH - 2.0 * SCROLL_RESERVE) / 2.0 + SCROLL_RESERVE).floor();
    (0..n).map(|i| (x, top + i as f32 * ROW)).collect()
}
/// The warning button beside the title (DialogScreen's title row: 10 pixels
/// apart), clamped onto the screen (makeSureWarningButtonIsInBounds).
fn warning_button(w: f32, h: f32, title_width: f32) -> (f32, f32) {
    let x = ((w - title_width - 30.0) / 2.0).floor() + title_width + 10.0;
    let y = ((HEADER - WARNING_BUTTON) / 2.0).floor();
    if x < 0.0 || y < 0.0 || x > w - WARNING_BUTTON || y > h - WARNING_BUTTON {
        (0.0f32.max(w - 40.0), 5.0f32.min(h))
    } else {
        (x, y)
    }
}
fn back_button(w: f32, h: f32) -> (f32, f32) {
    (
        ((w - 200.0) / 2.0).floor(),
        h - FOOTER + ((FOOTER - 20.0) / 2.0).floor(),
    )
}

impl Game<'_> {
    /// `connection.serverLinks()`: the links the open server sent (empty for
    /// a world without a connection or one that sent none).
    pub(super) fn server_links(&self) -> &[ServerLink] {
        self.remote
            .as_ref()
            .map_or(&[], |r| r.server_links.as_slice())
    }
    /// DialogScreen over the pause menu: `menu.server_links.title`, the
    /// scrollable link buttons and `gui.back` in the footer.
    pub(super) fn draw_server_links(&mut self, batch: &mut HudBatch) {
        // ServerLinksDialogScreen: the dialog's title.
        sunrise_client::widgets::begin_screen(|| {
            sunrise_client::widgets::ScreenInfo::new(
                "server_links",
                super::screen_info::tr("menu.server_links.title", &[]),
            )
        });
        let atlas = &self.gui.atlas;
        let (w, h) = (batch.width, batch.height);
        ui::menu_background(batch, atlas, 0.0, 0.0, w, h, true);
        self.draw_screen_phase(batch, sunrise_client::screen_draw::RenderPhase::Background);
        let links = self.server_links().to_vec();
        self.links_ui.scroll = self
            .links_ui
            .scroll
            .clamp(0.0, scroll_limit(h, links.len()));
        let top = list_top(h, links.len()) - self.links_ui.scroll.trunc();
        // The title row: the title, then 10 pixels on the warning button.
        let title = Text::translate("menu.server_links.title", &[]);
        let tw = atlas.font.width_text(&title);
        let tx = ((w - tw - 30.0) / 2.0).floor();
        batch.text_styled(atlas, &title, tx, 12.0, u32::MAX, true);
        let (wx, wy) = warning_button(w, h, tw);
        let wh = ui::hovered(self.cursor, wx, wy, WARNING_BUTTON, WARNING_BUTTON);
        // DialogScreen.createWarningButton: an ImageButton with its own
        // message and its tooltip.
        sunrise_client::widgets::narratable(|| {
            sunrise_client::widgets::Narratable::button(
                super::screen_info::tr("menu.custom_screen_info.button_narration", &[]),
                [wx, wy, WARNING_BUTTON, WARNING_BUTTON],
                true,
            )
            .hovered(wh)
            .hint(super::screen_info::tr(
                "menu.custom_screen_info.tooltip",
                &[],
            ))
        });
        sunrise_client::widgets::record(
            "menu.custom_screen_info.tooltip",
            [wx, wy, WARNING_BUTTON, WARNING_BUTTON],
            true,
        );
        batch.sprite_scaled(
            atlas,
            if wh {
                "dialog/warning_button_highlighted"
            } else {
                "dialog/warning_button"
            },
            wx,
            wy,
            WARNING_BUTTON,
            WARNING_BUTTON,
            u32::MAX,
        );
        let first = batch.vertices.len();
        let inside = self.cursor.1 >= HEADER && self.cursor.1 < h - FOOTER;
        for (link, (x, y)) in links.iter().zip(link_buttons(w, top, links.len())) {
            let hover = inside && ui::hovered(self.cursor, x, y, LINK_WIDTH, 20.0);
            ui::button(
                batch,
                atlas,
                x,
                y,
                LINK_WIDTH,
                20.0,
                &link_label(link),
                true,
                hover,
            );
        }
        batch.clip_since(first, 0.0, HEADER, w, h - FOOTER);
        ui::scrollbar(
            batch,
            atlas,
            (w / 2.0).floor() + 155.0 + 4.0,
            HEADER,
            h - FOOTER,
            list_height(links.len()),
            self.links_ui.scroll.trunc(),
        );
        let (bx, by) = back_button(w, h);
        let hover = ui::hovered(self.cursor, bx, by, 200.0, 20.0);
        ui::action_button(
            batch,
            atlas,
            sunrise_client::widgets::Action::Back,
            bx,
            by,
            200.0,
            20.0,
            &Text::translate("gui.back", &[]),
            true,
            hover,
        );
        self.draw_screen_phase(batch, sunrise_client::screen_draw::RenderPhase::Foreground);
        if wh {
            ui::tooltip(
                batch,
                atlas,
                &[Text::translate("menu.custom_screen_info.tooltip", &[])],
                self.cursor,
            );
        }
    }
    /// A click on `Screen::ServerLinks`: a link, the warning button or Back.
    pub(super) fn server_links_click(&mut self) -> Result<()> {
        let (w, h) = self.gui_size();
        let links = self.server_links().to_vec();
        let top = list_top(h, links.len()) - self.links_ui.scroll.trunc();
        let inside = self.cursor.1 >= HEADER && self.cursor.1 < h - FOOTER;
        if inside
            && let Some(i) = link_buttons(w, top, links.len())
                .iter()
                .position(|(x, y)| ui::hovered(self.cursor, *x, *y, LINK_WIDTH, 20.0))
        {
            self.audio.play_button_click();
            self.click_link(&links[i].link);
            return Ok(());
        }
        let title = Text::translate("menu.server_links.title", &[]);
        let tw = self.gui.atlas.font.width_text(&title);
        let (wx, wy) = warning_button(w, h, tw);
        if ui::hovered(self.cursor, wx, wy, WARNING_BUTTON, WARNING_BUTTON) {
            self.audio.play_button_click();
            self.set_screen(Screen::DialogWarning);
            return Ok(());
        }
        let (bx, by) = back_button(w, h);
        if ui::hovered(self.cursor, bx, by, 200.0, 20.0) {
            self.audio.play_button_click();
            self.set_screen(Screen::Paused);
        }
        Ok(())
    }
    /// `Screen.clickUrlAction`: "Web Links" off ignores the click; "Prompt on
    /// Links" asks with ConfirmLinkScreen, else the browser opens directly.
    fn click_link(&mut self, url: &str) {
        if !self.settings.chat_links {
            return;
        }
        if self.settings.chat_links_prompt {
            self.links_ui.confirm = Some(url.to_owned());
            self.links_ui.trusted = false;
            self.links_ui.back = None;
            self.set_screen(Screen::ConfirmLink);
        } else {
            open_uri(url);
        }
    }
    /// `ConfirmLinkScreen.confirmLinkNow(parent, uri, true)`: ask about a link
    /// Mojang's own button names, whatever the "Web Links" options say.
    pub(super) fn confirm_trusted_link(&mut self, url: &str, parent: Screen) {
        self.links_ui.confirm = Some(url.to_owned());
        self.links_ui.trusted = true;
        self.links_ui.back = Some(parent);
        self.set_screen(Screen::ConfirmLink);
    }
    /// `ConfirmLinkScreen.confirmLinkNow(parent, uri, false)`: ask about a link that is not
    /// Mojang's, with the warning, whatever the "Web Links" options say.
    pub(super) fn confirm_untrusted_link(&mut self, url: &str, parent: Screen) {
        self.links_ui.confirm = Some(url.to_owned());
        self.links_ui.trusted = false;
        self.links_ui.back = Some(parent);
        self.set_screen(Screen::ConfirmLink);
    }
    /// The screen `ConfirmLinkScreen`'s callback returns to (`setScreen(parent)`).
    pub(super) fn confirm_link_back(&self) -> Screen {
        self.links_ui.back.unwrap_or(Screen::ServerLinks)
    }
    /// `ScrollableLayout`'s mouse wheel over the links.
    pub(super) fn server_links_scroll(&mut self, delta: f32) {
        let (_, h) = self.gui_size();
        self.links_ui.scroll = (self.links_ui.scroll - delta * 10.0)
            .clamp(0.0, scroll_limit(h, self.server_links().len()));
    }
    /// `ConfirmLinkScreen` (untrusted): "Are you sure you want to open the
    /// following website?", the link, the red warning, and Yes / Copy Link to
    /// Clipboard / No.
    pub(super) fn draw_confirm_link(&mut self, batch: &mut HudBatch) {
        let atlas = &self.gui.atlas;
        let (w, h) = (batch.width, batch.height);
        ui::menu_background(batch, atlas, 0.0, 0.0, w, h, true);
        self.draw_screen_phase(batch, sunrise_client::screen_draw::RenderPhase::Background);
        let url = self.links_ui.confirm.clone().unwrap_or_default();
        let trusted = self.links_ui.trusted;
        sunrise_client::widgets::begin_screen(|| {
            let title = if trusted {
                "chat.link.confirmTrusted"
            } else {
                "chat.link.confirm"
            };
            super::screen_info::confirm(
                "server_links/confirm_link",
                &super::screen_info::tr(title, &[]),
                &url,
            )
        });
        let layout = link_layout(&atlas.font, (w, h), &url, trusted);
        for (line, x, y) in &layout.lines {
            batch.text_styled(atlas, line, *x, *y, u32::MAX, true);
        }
        let labels = link_labels(self.links_ui.trusted);
        // Its third button answers no, as Escape does.
        if let Some(&(x, y)) = layout.buttons.get(2) {
            sunrise_client::widgets::declare_action(
                sunrise_client::widgets::Action::Cancel,
                [x, y, 100.0, 20.0],
                true,
            );
        }
        for ((x, y), key) in layout.buttons.iter().zip(labels) {
            let hover = ui::hovered(self.cursor, *x, *y, 100.0, 20.0);
            ui::button(
                batch,
                atlas,
                *x,
                *y,
                100.0,
                20.0,
                &Text::translate(key, &[]),
                true,
                hover,
            );
        }
        self.draw_screen_phase(batch, sunrise_client::screen_draw::RenderPhase::Foreground);
    }
    /// ConfirmLinkScreen's answer: yes opens the browser, the middle button
    /// copies the link, no closes; all return to Server Links.
    pub(super) fn confirm_link_click(&mut self) {
        let (w, h) = self.gui_size();
        let url = self.links_ui.confirm.clone().unwrap_or_default();
        let layout = link_layout(&self.gui.atlas.font, (w, h), &url, self.links_ui.trusted);
        for (i, (x, y)) in layout.buttons.iter().enumerate() {
            if !ui::hovered(self.cursor, *x, *y, 100.0, 20.0) {
                continue;
            }
            self.audio.play_button_click();
            match i {
                0 => open_uri(&url),
                1 => {
                    self.clipboard.set(&url);
                }
                _ => {}
            }
            self.set_screen(self.confirm_link_back());
            return;
        }
    }
    /// `DialogScreen.WarningScreen`: the custom-screens note, a disconnect
    /// button ("Custom screen rejected" is the shown reason) and Back.
    pub(super) fn draw_dialog_warning(&mut self, batch: &mut HudBatch) {
        sunrise_client::widgets::begin_screen(|| {
            super::screen_info::confirm(
                "dialog_warning",
                &super::screen_info::tr("menu.custom_screen_info.title", &[]),
                &super::screen_info::tr("menu.custom_screen_info.contents", &[]),
            )
        });
        let atlas = &self.gui.atlas;
        let (w, h) = (batch.width, batch.height);
        ui::menu_background(batch, atlas, 0.0, 0.0, w, h, true);
        self.draw_screen_phase(batch, sunrise_client::screen_draw::RenderPhase::Background);
        let layout = dialog_warning_layout(atlas, (w, h));
        for (line, x, y) in &layout.lines {
            batch.text_styled(atlas, line, *x, *y, u32::MAX, true);
        }
        let quit = if self.external_server() {
            "menu.disconnect"
        } else {
            "menu.returnToMenu"
        };
        for ((x, y), key) in layout.buttons.into_iter().zip([quit, "gui.back"]) {
            let hover = ui::hovered(self.cursor, x, y, 150.0, 20.0);
            if key == "gui.back" {
                sunrise_client::widgets::declare_action(
                    sunrise_client::widgets::Action::Back,
                    [x, y, 150.0, 20.0],
                    true,
                );
            }
            ui::button(
                batch,
                atlas,
                x,
                y,
                150.0,
                20.0,
                &Text::translate(key, &[]),
                true,
                hover,
            );
        }
        self.draw_screen_phase(batch, sunrise_client::screen_draw::RenderPhase::Foreground);
    }
    /// WarningScreen's answer: disconnect, or back to Server Links.
    pub(super) fn dialog_warning_click(&mut self) -> Result<()> {
        let (w, h) = self.gui_size();
        let layout = dialog_warning_layout(&self.gui.atlas, (w, h));
        for (i, (x, y)) in layout.buttons.iter().enumerate() {
            if !ui::hovered(self.cursor, *x, *y, 150.0, 20.0) {
                continue;
            }
            self.audio.play_button_click();
            if i == 0 {
                // DialogConnectionAccess.disconnect: leave with the custom
                // screens' reason.
                self.leave_server(Some(Text::translate_with_fallback(
                    "menu.custom_screen_info.disconnect",
                    "Custom screen rejected",
                    &[],
                )))?;
            } else {
                self.set_screen(Screen::ServerLinks);
            }
            return Ok(());
        }
        Ok(())
    }
}

fn dialog_warning_layout(atlas: &sunrise_client::hud::HudAtlas, size: (f32, f32)) -> ConfirmLayout {
    confirm_layout(
        &atlas.font,
        size,
        &Text::translate("menu.custom_screen_info.title", &[]),
        &Text::translate("menu.custom_screen_info.contents", &[]),
        &[],
        &[150.0; 2],
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bug_report_link_is_the_first_report_bug_link() {
        // ServerLinks.findKnownType(BUG_REPORT): a custom label never counts, and ByIdMap.ZERO
        // reads an id outside KnownLinkType as report_bug.
        let link = |kind, link: &str| ServerLink {
            kind,
            link: link.into(),
        };
        let links = [
            link(ServerLinkKind::Known(2), "https://example.net/support"),
            link(
                ServerLinkKind::Custom(sunrise_core::nbt::Tag::String("Bugs".into())),
                "https://example.net/custom",
            ),
            link(ServerLinkKind::Known(42), "https://example.net/unknown"),
            link(ServerLinkKind::Known(0), "https://example.net/bugs"),
        ];
        assert_eq!(bug_report_link(&links), Some("https://example.net/unknown"));
        assert_eq!(bug_report_link(&links[..2]), None);
    }

    #[test]
    fn a_malformed_bug_link_is_dropped_before_the_valid_one_is_picked() {
        // handleServerLinks validates each link (java.net.URI syntax, http or https) before
        // findKnownType(BUG_REPORT) picks the first: a broken entry can't hide the good one.
        let bug = |link: &str| ServerLink {
            kind: ServerLinkKind::Known(0),
            link: link.into(),
        };
        let links = trusted(vec![
            bug("https://exa mple.net/bugs"),
            bug("https://example.net/%zz"),
            bug("ftp://example.net/bugs"),
            bug("https://example.net/bugs"),
        ]);
        assert_eq!(bug_report_link(&links), Some("https://example.net/bugs"));
    }
}
