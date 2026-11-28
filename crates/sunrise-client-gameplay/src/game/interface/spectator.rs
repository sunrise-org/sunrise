//! The spectator menu (`SpectatorGui`): its players and teams from the
//! server's player list and teams, and the teleport request.
use crate::game::ClientGame;
use sunrise_client::{
    hud::HudBatch,
    hud_layout, keybinds,
    spectator_menu::{Member, Outcome, PlayerInfo, SpectatorMenu, Team},
    tab_list::default_skin,
};
use sunrise_game_data::{
    scoreboard::{comp_text, team_color_rgb},
    text::Text,
};
use sunrise_level::profile_lookup::SPECTATOR;
use sunrise_protocol::packets::play::{self, Serverbound as S};

/// The `PlayerInfo` of a menu member, read from this connection when an item
/// asks for it: the roster is frozen when the menu opens, its values are not.
struct LivePlayerInfos<'g, 'a>(&'g ClientGame<'a>);

impl PlayerInfo for LivePlayerInfos<'_, '_> {
    fn skin(&self, uuid: u128) -> String {
        let game = self.0;
        let Some(remote) = game.client.remote.as_ref() else {
            return default_skin(uuid).to_owned();
        };
        game.face_skin(uuid, remote.player_info_skin(uuid))
    }
    fn enabled(&self, uuid: u128) -> bool {
        self.0
            .client
            .remote
            .as_ref()
            .and_then(|remote| remote.player_info(uuid))
            .is_some_and(|player| player.game_mode != SPECTATOR)
    }
}

impl ClientGame<'_> {
    /// `Util.getMillis` for the menu's fade.
    fn spectator_now(&self) -> i64 {
        self.client.started.elapsed().as_millis() as i64
    }
    /// `TeleportToPlayerMenuCategory` starts from listed players;
    /// `TeleportToTeamMenuCategory.TeamSelectionItem.create` looks up every team member.
    pub fn new_spectator_menu(&mut self) -> SpectatorMenu {
        let tab = self
            .remote_tab_players()
            .map_or_else(Vec::new, |(players, _, _)| players);
        let players = tab
            .iter()
            .map(|p| Member {
                uuid: p.uuid,
                name: p.name.clone(),
            })
            .collect();
        let teams = self.common.score.board.teams_streamed();
        let rosters: Vec<_> = self
            .client
            .remote
            .iter()
            .flat_map(|remote| teams.iter().copied().map(move |team| (remote, team)))
            .filter_map(|(remote, team)| {
                let members: Vec<_> = team
                    .players
                    .iter()
                    .filter_map(|name| remote.player_info_by_name(name))
                    .filter(|p| p.game_mode != SPECTATOR)
                    .map(|p| Member {
                        uuid: p.uuid,
                        name: p.name.clone(),
                    })
                    .collect();
                (!members.is_empty()).then(|| {
                    (
                        comp_text(&team.display_name),
                        team.color.as_deref().and_then(team_color_rgb),
                        members,
                    )
                })
            })
            .collect();
        let teams = rosters
            .into_iter()
            .map(|(name, color, players)| {
                // A random member's face is the team's icon; its skin is read
                // when the icon is drawn (`iconSkin`).
                let pick = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_or(0, |d| d.subsec_nanos() as usize)
                    % players.len();
                Team {
                    name,
                    color,
                    icon: players[pick].uuid,
                    players,
                }
            })
            .collect();
        SpectatorMenu::new(players, teams, &LivePlayerInfos(self))
    }
    /// The player list as the menu reads it: the lookup every press goes through.
    pub fn spectator_player_infos(&self) -> impl PlayerInfo + '_ {
        LivePlayerInfos(self)
    }
    pub fn spectator_outcome(&mut self, outcome: Outcome) {
        if let Outcome::Teleport(uuid) = outcome {
            // PlayerMenuItem.selectItem.
            self.send(S::TeleportToEntity(play::TeleportToEntity { uuid }));
        }
    }
    /// A hotbar key as a spectator (`SpectatorGui.onHotbarSelected`).
    pub(in crate::game::client_side) fn spectator_hotbar_key(&mut self, slot: i32) {
        let now = self.spectator_now();
        let menu = self.spectator_menu_if_needed();
        let mut gui = std::mem::take(&mut self.client.spectator_gui);
        let outcome = gui.on_hotbar_selected(slot, now, menu, &LivePlayerInfos(self));
        self.client.spectator_gui = gui;
        self.spectator_outcome(outcome);
    }
    /// `key.spectatorHotbar`: opens the menu, or uses the selected item.
    pub(in crate::game::client_side) fn spectator_action_key(&mut self) {
        let now = self.spectator_now();
        let menu = self.spectator_menu_if_needed();
        let mut gui = std::mem::take(&mut self.client.spectator_gui);
        let outcome = gui.on_action_key(now, menu, &LivePlayerInfos(self));
        self.client.spectator_gui = gui;
        self.spectator_outcome(outcome);
    }
    /// The menu a press would open, built only when none is up.
    fn spectator_menu_if_needed(&mut self) -> Option<SpectatorMenu> {
        (!self.client.spectator_gui.is_menu_active()).then(|| self.new_spectator_menu())
    }
    /// `MouseHandler.onScroll` in spectator mode with the menu up.
    pub(in crate::game::client_side) fn spectator_scroll(&mut self, wheel: i32) {
        let now = self.spectator_now();
        let mut gui = std::mem::take(&mut self.client.spectator_gui);
        gui.on_mouse_scrolled(wheel, now, &LivePlayerInfos(self));
        self.client.spectator_gui = gui;
    }
    /// `Hud.extractHotbar` for a spectator, and `extractAction`'s prompt.
    pub(in crate::game::client_side) fn draw_spectator_hotbar(
        &mut self,
        batch: &mut HudBatch,
        show_action: bool,
    ) {
        let (w, h) = (batch.width as i32, batch.height as i32);
        let now = self.spectator_now();
        let labels: Vec<Text> = (1..=9)
            .map(|n| {
                keybinds::display_name(self.client.settings.keys.get(&format!("key.hotbar.{n}")))
            })
            .collect();
        let mut gui = std::mem::take(&mut self.client.spectator_gui);
        let draws = {
            let font = &self.client.gui.atlas.font;
            let infos = LivePlayerInfos(self);
            let mut draws = gui.hotbar(w, h, now, &|slot| labels[slot].clone(), font, &infos);
            if show_action {
                draws.extend(gui.action(
                    w,
                    h,
                    now,
                    font,
                    self.client.settings.text_background(0.0),
                ));
            }
            draws
        };
        self.client.spectator_gui = gui;
        hud_layout::paint(batch, &self.client.gui.atlas, &draws);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use sunrise_assets::resource_source::ResourceSource;
    use sunrise_client::{
        font::Font,
        hud_layout::GuiDraw,
        spectator_menu::{Item, SpectatorGui},
    };

    fn member(uuid: u128, name: &str) -> Member {
        Member {
            uuid,
            name: name.into(),
        }
    }

    fn blit_textures(draws: &[GuiDraw]) -> Vec<String> {
        draws
            .iter()
            .filter_map(|draw| match draw {
                GuiDraw::Blit { texture, .. } => Some(texture.to_string()),
                _ => None,
            })
            .collect()
    }

    fn blit_color(draws: &[GuiDraw], texture: &str) -> u32 {
        draws
            .iter()
            .find_map(|draw| match draw {
                GuiDraw::Blit {
                    texture: t, color, ..
                } if t == texture => Some(*color),
                _ => None,
            })
            .unwrap_or_else(|| panic!("no face drawn for {texture}"))
    }

    fn players_page_of(menu: &mut SpectatorMenu, infos: &HashMap<u128, (String, bool)>) {
        menu.select_slot(0, infos);
        menu.select_slot(0, infos);
    }

    #[test]
    fn a_skin_that_arrives_after_opening_is_drawn_for_the_member_and_team_icon() {
        const PLAYER: u128 = 0xbeef;
        let old_skin = "entity/player/wide/steve";
        let new_skin = "skins/arrived";
        let mut infos = HashMap::from([(PLAYER, (old_skin.to_owned(), true))]);
        let player = member(PLAYER, "Player");
        let team = Team {
            name: Text::literal("Team"),
            color: None,
            players: vec![player.clone()],
            icon: PLAYER,
        };
        let mut member_page = SpectatorMenu::new(vec![player], vec![team.clone()], &infos);
        players_page_of(&mut member_page, &infos);
        let mut team_page = SpectatorMenu::new(Vec::new(), vec![team], &infos);
        team_page.select_slot(1, &infos);
        team_page.select_slot(1, &infos);

        // The texture finishes downloading after the menu was built.
        infos.insert(PLAYER, (new_skin.to_owned(), true));

        let font = Font::metrics(&ResourceSource::default());
        let mut member_gui = SpectatorGui::default();
        member_gui.on_hotbar_selected(0, 0, Some(member_page), &infos);
        let member_draws = member_gui.hotbar(100, 100, 0, &|_| Text::literal(""), &font, &infos);
        assert!(blit_textures(&member_draws).iter().any(|t| t == new_skin));

        let mut team_gui = SpectatorGui::default();
        team_gui.on_hotbar_selected(0, 0, Some(team_page), &infos);
        let team_draws = team_gui.hotbar(100, 100, 0, &|_| Text::literal(""), &font, &infos);
        assert!(blit_textures(&team_draws).iter().any(|t| t == new_skin));
    }

    #[test]
    fn a_member_who_becomes_a_spectator_is_no_longer_selectable() {
        const PLAYER: u128 = 0xbeef;
        let mut infos = HashMap::from([(PLAYER, ("skin".to_owned(), true))]);
        let mut menu = SpectatorMenu::new(vec![member(PLAYER, "Player")], vec![], &infos);
        players_page_of(&mut menu, &infos);
        assert!(menu.item(0).enabled(&infos));

        infos.insert(PLAYER, ("skin".to_owned(), false));
        assert!(!menu.item(0).enabled(&infos));
        assert_eq!(menu.select_slot(0, &infos), Outcome::None);
        assert_eq!(menu.select_slot(0, &infos), Outcome::None);
    }

    #[test]
    fn a_member_who_leaves_spectator_is_selectable_again() {
        const PLAYER: u128 = 0xbeef;
        let mut infos = HashMap::from([(PLAYER, ("skin".to_owned(), true))]);
        let mut menu = SpectatorMenu::new(vec![member(PLAYER, "Player")], vec![], &infos);
        players_page_of(&mut menu, &infos);

        infos.insert(PLAYER, ("skin".to_owned(), false));
        assert!(!menu.item(0).enabled(&infos));
        assert_eq!(menu.select_slot(0, &infos), Outcome::None);

        infos.insert(PLAYER, ("skin".to_owned(), true));
        assert!(menu.item(0).enabled(&infos));
        assert_eq!(menu.select_slot(0, &infos), Outcome::Teleport(PLAYER));
    }

    #[test]
    fn a_teammate_who_becomes_a_spectator_keeps_the_fade_tint() {
        const PLAYER: u128 = 0xbeef;
        let skin = "entity/player/wide/steve";
        let mut infos = HashMap::from([(PLAYER, (skin.to_owned(), true))]);
        let team = Team {
            name: Text::literal("Team"),
            color: None,
            players: vec![member(PLAYER, "Player")],
            icon: PLAYER,
        };
        // Open the root page, then Teleport to Team, then the team itself:
        // `Item::Team` -> `Category::TeamPlayers`.
        let mut menu = SpectatorMenu::new(Vec::new(), vec![team], &infos);
        menu.select_slot(1, &infos);
        menu.select_slot(1, &infos);
        assert!(matches!(menu.item(0), Item::Team(_)));
        menu.select_slot(0, &infos);
        menu.select_slot(0, &infos);
        assert!(matches!(menu.item(0), Item::Player(p) if p.uuid == PLAYER));

        // They switch to spectator while their team's page is open.
        infos.insert(PLAYER, (skin.to_owned(), false));
        assert!(!menu.item(0).enabled(&infos));
        let mut gui = SpectatorGui::default();
        gui.on_hotbar_selected(0, 0, Some(menu), &infos);
        let draws = gui.hotbar(
            100,
            100,
            0,
            &|_| Text::literal(""),
            &Font::metrics(&ResourceSource::default()),
            &infos,
        );
        let tint = blit_color(&draws, skin);
        assert_eq!(tint, hud_layout::white(1.0));
        assert_ne!(tint, hud_layout::color_from_float(1.0, 0.25, 0.25, 0.25));
        assert_eq!(gui.on_hotbar_selected(0, 0, None, &infos), Outcome::None);
        assert_eq!(gui.on_hotbar_selected(0, 0, None, &infos), Outcome::None);

        // Leaving spectator makes the same entry selectable again.
        infos.insert(PLAYER, (skin.to_owned(), true));
        assert_eq!(
            gui.on_hotbar_selected(0, 0, None, &infos),
            Outcome::Teleport(PLAYER)
        );
    }

    #[test]
    fn a_disabled_player_is_drawn_at_the_fade_alpha() {
        const PLAYER: u128 = 0xbeef;
        let skin = "entity/player/wide/steve";
        let mut infos = HashMap::from([(PLAYER, (skin.to_owned(), true))]);
        let mut menu = SpectatorMenu::new(vec![member(PLAYER, "Player")], vec![], &infos);
        players_page_of(&mut menu, &infos);
        infos.insert(PLAYER, (skin.to_owned(), false));
        assert!(!menu.item(0).enabled(&infos));

        let mut gui = SpectatorGui::default();
        gui.on_hotbar_selected(0, 0, Some(menu), &infos);
        let draws = gui.hotbar(
            100,
            100,
            0,
            &|_| Text::literal(""),
            &Font::metrics(&ResourceSource::default()),
            &infos,
        );
        assert_eq!(
            blit_color(&draws, skin),
            hud_layout::color_from_float(1.0, 1.0, 1.0, 1.0)
        );
    }

    #[test]
    fn a_member_without_player_info_draws_their_default_skin() {
        let infos = HashMap::new();
        assert_eq!(
            PlayerInfo::skin(&infos, 0xcaff),
            sunrise_client::tab_list::default_skin(0xcaff)
        );
        assert!(!infos.enabled(0xcaff));
    }

    #[test]
    fn a_player_who_joins_after_opening_is_not_in_the_menu() {
        const EARLY: u128 = 0xbeef;
        const LATE: u128 = 0xcafe;
        let mut infos = HashMap::from([(EARLY, ("skin".to_owned(), true))]);
        let mut menu = SpectatorMenu::new(vec![member(EARLY, "Early")], vec![], &infos);

        infos.insert(LATE, ("skin".to_owned(), true));
        players_page_of(&mut menu, &infos);
        assert!(matches!(menu.item(0), Item::Player(p) if p.uuid == EARLY));
        assert_eq!(menu.item(1), Item::Empty);
    }
}
