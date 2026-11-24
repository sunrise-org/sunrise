//! A player's statistics and recipe book for native server mods, through a
//! `PlayerRef` (docs/modding/src/how-to/native-mods.md): `ServerStatsCounter` and
//! `ServerRecipeBook`. Scalar reads return None for a stale or absent player;
//! a book visit reports a typed refusal. Writes return false (or None) for
//! those players, for a dead player, and for anyone but the acting player
//! during a player turn.
use super::{
    Game, lan_actions::CombatPlayer, native_server::PlayerRead, stats::Stats,
    transactions::DeferredEffect,
};
use core::ops::ControlFlow;
use mod_api::{EntityReadError, PlayerRef, StatType};
use sunrise_gameplay::stats::StatValues;

use super::native_namespaced_id::namespaced;

impl Game<'_> {
    /// Whether `player` still names the same player on this server.
    pub(super) fn native_player_current(&self, player: PlayerRef) -> bool {
        self.player_handle_read(player).is_some()
    }
    /// The guest record `player` reads from outside their turn, or None
    /// when their state is in the game (they act, or they are the host).
    pub(super) fn native_player_resting(
        &self,
        player: PlayerRef,
    ) -> Option<&sunrise_client::lan::guest::Guest> {
        match self.player_handle_read(player)? {
            PlayerRead::Guest(guest) => Some(guest),
            PlayerRead::Acting { .. } => None,
        }
    }
    /// The player's statistics, including the counter lent to a nested turn.
    fn native_player_stats(&self, player: PlayerRef) -> Option<Option<&Stats>> {
        match self.player_handle_read(player) {
            Some(PlayerRead::Guest(g)) => {
                Some(self.lan.as_ref()?.play.get(&g.conn).map(|p| &p.stats))
            }
            Some(PlayerRead::Acting { .. }) => Some(Some(&self.stats)),
            None => {
                let entity = player.entity();
                if self.lan_displaced_players.is_empty()
                    || self.native_lifecycle.epoch()? != entity.epoch()
                {
                    return None;
                }
                let CombatPlayer::Displaced(depth, _) = self.combat_player(entity.id())? else {
                    return None;
                };
                let frame = &self.lan_displaced_players[depth];
                (frame.uuid == Some(player.uuid())).then_some(Some(&frame.stats))
            }
        }
    }

    /// The value of statistic `stat_type` for `key` (`minecraft:stone`,
    /// `minecraft:jump`...), as `ServerStatsCounter.getValue`: 0 when never
    /// counted.
    pub(super) fn player_stat(
        &self,
        player: PlayerRef,
        stat_type: StatType,
        key: &str,
    ) -> Option<i32> {
        let ty = stat_type.path();
        let stats = self.native_player_stats(player)?;
        Some(stats.map_or(0, |s| s.get(ty, &namespaced(key))))
    }
    /// `ServerStatsCounter.toJson` for this player's live counter.
    pub(super) fn player_statistics(&self, player: PlayerRef) -> Option<String> {
        if !self.is_logical_server() {
            return None;
        }
        let json = self
            .native_player_stats(player)?
            .map_or_else(|| Stats::default().to_json(), Stats::to_json);
        Some(json.to_string())
    }

    /// Decode with the server's registries before entering the player's turn. Inside a
    /// transaction the replacement waits for the root commit, as the game's own deferred
    /// statistics do, so counters and criterion scores never outlive an abort apart.
    pub(super) fn player_set_statistics(&mut self, player: PlayerRef, snapshot: &str) -> bool {
        if !self.is_logical_server() {
            return false;
        }
        let Some(values) = Stats::decode_snapshot(snapshot, |registry, id| {
            self.stat_registry_contains(registry, id)
        }) else {
            return false;
        };
        if self.transactions.last().is_some() {
            if !self.native_player_writable(player) {
                return false;
            }
            self.defer_effect(DeferredEffect::RestoreStatistics(player, Box::new(values)));
            return true;
        }
        self.restore_player_statistics(player, values)
    }
    /// The replacement itself, in the player's turn.
    pub(super) fn restore_player_statistics(
        &mut self,
        player: PlayerRef,
        values: StatValues,
    ) -> bool {
        self.native_as_player_handle(player, |game| {
            // A local counter loads lazily; guest turns already own their counter.
            if game.lan_acting.is_none() {
                game.load_stats();
            }
            game.stats.replace_values(values);
        })
    }
    /// Whether `native_as_player_handle` would run for `player` now.
    fn native_player_writable(&self, player: PlayerRef) -> bool {
        match self.player_handle_read(player) {
            None => false,
            Some(PlayerRead::Acting { .. }) => true,
            Some(PlayerRead::Guest(g)) => !g.dead,
        }
    }

    /// `Player.awardStat(stat, amount)`: adds `amount`, as the game does
    /// for its own counts, so `player_stat_award` hears it and may rewrite or cancel
    /// it, and scoreboard criteria follow. False for a player it can't
    /// reach.
    pub(super) fn player_award_stat(
        &mut self,
        player: PlayerRef,
        stat_type: StatType,
        key: &str,
        amount: i32,
    ) -> bool {
        let ty = stat_type.path();
        let key = namespaced(key);
        self.native_player_current(player)
            && self.native_as_player_handle(player, |game| game.award_stat(ty, &key, amount))
    }
    /// `Player.resetStat`: sets the statistic back to 0, which scoreboard
    /// criteria follow. False for a player it can't reach.
    pub(super) fn player_reset_stat(
        &mut self,
        player: PlayerRef,
        stat_type: StatType,
        key: &str,
    ) -> bool {
        let ty = stat_type.path();
        let key = namespaced(key);
        self.native_player_current(player)
            && self.native_as_player_handle(player, |game| game.stats.reset(ty, &key))
    }

    /// Whether the player's recipe book knows `recipe`
    /// (`ServerRecipeBook.contains`).
    pub(super) fn player_has_discovered_recipe(
        &self,
        player: PlayerRef,
        recipe: &str,
    ) -> Option<bool> {
        if !self.native_player_current(player) {
            return None;
        }
        let id = namespaced(recipe);
        Some(match self.native_player_resting(player) {
            Some(g) => g.recipe_book.recipes.contains(&id),
            None => self.level.recipe_book.recipes.contains(&id),
        })
    }
    /// Visits the player's qualified recipe ids in the resident book's order (`ServerRecipeBook.known`).
    pub(super) fn player_visit_discovered_recipes(
        &self,
        player: PlayerRef,
        visit: &mut dyn FnMut(&str) -> ControlFlow<()>,
    ) -> Result<ControlFlow<()>, EntityReadError> {
        if !self.is_logical_server() {
            return Err(EntityReadError::Unavailable);
        }
        let book = match self
            .player_handle_read(player)
            .ok_or(EntityReadError::Stale)?
        {
            PlayerRead::Guest(guest) => &guest.recipe_book.recipes,
            PlayerRead::Acting { .. } => &self.level.recipe_book.recipes,
        };
        Ok(book.iter().map(String::as_str).try_for_each(visit))
    }
    /// `ServerPlayer.awardRecipes`, as `/recipe give` does: the recipes
    /// unlock in the player's book, with their toasts and the
    /// `player_recipe_discovered` hook, and the client hears of them.
    /// Unknown ids are skipped. The number of recipe displays added, None
    /// for a player it can't reach.
    pub(super) fn player_award_recipes(
        &mut self,
        player: PlayerRef,
        recipes: &[&str],
    ) -> Option<i32> {
        self.native_player_change_recipes(player, true, recipes)
    }
    /// `ServerPlayer.resetRecipes`, as `/recipe take` does. The number of
    /// recipe displays taken out, None for a player it can't reach.
    pub(super) fn player_reset_recipes(
        &mut self,
        player: PlayerRef,
        recipes: &[&str],
    ) -> Option<i32> {
        self.native_player_change_recipes(player, false, recipes)
    }
    fn native_player_change_recipes(
        &mut self,
        player: PlayerRef,
        give: bool,
        recipes: &[&str],
    ) -> Option<i32> {
        if !self.native_player_current(player) {
            return None;
        }
        // `awardRecipes` takes recipes the server has: an id no recipe has
        // would otherwise go into the book.
        let index = &self.stations().recipe_index;
        let ids: Vec<String> = recipes
            .iter()
            .map(|r| namespaced(r))
            .filter(|id| index.contains(id))
            .collect();
        let mut count = 0;
        self.native_as_player_handle(player, |game| {
            count = game.change_recipes(give, &ids);
        })
        .then_some(count)
    }
}

#[cfg(test)]
mod tests {
    use super::super::{
        native_entity_refs::tests::{fixture_root, run_fresh_cases, with_game},
        native_guest_turn_tests::with_guests,
    };
    use super::*;
    use mod_api::{Flow, Hooks, PlayerStatAwardEvent, ServerHost, StatType};
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    struct ObserveAwards(Arc<AtomicUsize>);

    #[mod_api::__subscriptions]
    impl Hooks for ObserveAwards {
        fn player_stat_award(
            &self,
            _: &mut dyn ServerHost,
            _: &mut PlayerStatAwardEvent<'_>,
        ) -> Flow {
            self.0.fetch_add(1, Ordering::Relaxed);
            Flow::Continue
        }
    }

    #[test]
    #[ignore = "daily"]
    fn statistics_restore_updates_live_scores_and_next_request_without_awarding() {
        use serde_json::{Value, json};
        use sunrise_client::{
            lan::{Out, PlayerSender, playerdata::uuid_ints},
            scoreboard::{RenderType, literal},
        };
        use sunrise_protocol::{
            buf::Reader,
            ids::play::clientbound::AWARD_STATS,
            packets::play::{ClientCommand, Clientbound, Serverbound},
        };

        if run_fresh_cases(
            "game::native_player_stats::tests::statistics_restore_updates_live_scores_and_next_request_without_awarding",
            "SUNRISE_STATISTICS_SNAPSHOT_CHILD",
            &["restore"],
        ) {
            return;
        }
        sunrise_world::datafix::set_resources(&fixture_root());
        let awards = Arc::new(AtomicUsize::new(0));
        let mods = vec![mod_api::ModEntry::new(
            "observe_awards",
            Box::new(ObserveAwards(awards.clone())),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            with_guests(|game| {
                let world = tempfile::tempdir().unwrap();
                game.world_root = world.path().to_path_buf();
                game.load_scoreboard();
                assert!(mod_api::__private::listened(
                    mod_api::HookId::PlayerStatAward
                ));

                let version = sunrise_world::DATA_VERSION;
                let empty = json!({"DataVersion": version, "stats": {}});
                let player = game.player(12).unwrap();
                let other = game.player(13).unwrap();
                let stale = mod_api::__private::game::player_ref(player.entity(), 999);
                assert!(stale.statistics(game).is_none());
                assert!(!stale.set_statistics(game, &empty.to_string()));
                let before = player.statistics(game).unwrap();
                assert_eq!(serde_json::from_str::<Value>(&before).unwrap(), empty);

                // No DataVersion: vanilla's StatsCounterFix upgrades this flat counter.
                assert!(player.set_statistics(game, r#"{"stat.jump":6}"#));
                assert_eq!(player.stat(game, StatType::Custom, "jump"), Some(6));
                assert_eq!(serde_json::from_str::<Value>(&before).unwrap(), empty);

                let host = game.lan.as_mut().unwrap();
                let stats = &mut host.play.get_mut(&12).unwrap().stats;
                stats.add("mined", "minecraft:stone", 6);
                stats.custom("jump", 3);
                stats.custom("deaths", 2);
                stats.events.clear();
                stats.take_dirty();
                host.play.entry(13).or_default().stats.custom("jump", 17);
                let other_before = other.statistics(game).unwrap();
                let board = &mut game.score.board;
                for (name, criteria, score) in [
                    ("jump", "minecraft.custom:minecraft.jump", 50),
                    ("mined", "minecraft.mined:minecraft.stone", 40),
                    ("deaths", "minecraft.custom:minecraft.deaths", 11),
                    ("dummy", "dummy", 80),
                ] {
                    assert!(board.add_objective(
                        name,
                        criteria,
                        literal(name),
                        RenderType::Integer,
                        false,
                        None
                    ));
                    board.set("guest12", name, score);
                }
                board.set("guest13", "jump", 17);

                let before = player.statistics(game).unwrap();
                assert!(!player.set_statistics(game, r#"{"stats":{}"#));
                assert_eq!(player.statistics(game).unwrap(), before);
                let restored = json!({
                    "DataVersion": version,
                    "stats": {"minecraft:custom": {"minecraft:jump": -7, "minecraft:deaths": 2}}
                });
                assert!(player.set_statistics(game, &restored.to_string()));
                assert_eq!(player.stat(game, StatType::Mined, "stone"), Some(0));
                assert_eq!(other.statistics(game).unwrap(), other_before);
                let board = &game.score.board;
                for (holder, name, value) in [
                    ("guest12", "jump", -7),
                    ("guest12", "mined", 0),
                    ("guest12", "deaths", 11),
                    ("guest12", "dummy", 80),
                    ("guest13", "jump", 17),
                ] {
                    assert_eq!(
                        board.score(holder, name).unwrap().value,
                        value,
                        "{holder}/{name}"
                    );
                }

                let (tx, rx) = std::sync::mpsc::channel();
                game.lan
                    .as_mut()
                    .unwrap()
                    .hub
                    .update_guest(12, |guest| {
                        guest.sender = PlayerSender::new(tx, Arc::default());
                    })
                    .unwrap();
                game.lan_guest_packet(12, Serverbound::ClientCommand(ClientCommand { action: 1 }));
                let host = game.lan.as_ref().unwrap();
                let hub = &host.hub;
                let id = |registry, key| hub.registry_id(registry, key).unwrap();
                let mut responses = Vec::new();
                for out in rx.try_iter() {
                    let bodies = match out {
                        Out::Packet(body) => vec![body],
                        Out::Packets(bodies) => bodies,
                        Out::Disconnect(_) => panic!("statistics request disconnected the guest"),
                    };
                    for body in bodies {
                        let mut r = Reader::new(&body);
                        let id = r.varint().unwrap();
                        if id == AWARD_STATS {
                            let Clientbound::AwardStats(response) =
                                Clientbound::decode(id, r.rest()).unwrap()
                            else {
                                panic!("not an AwardStats response");
                            };
                            responses.push(response.stats);
                        }
                    }
                }
                assert_eq!(
                    responses,
                    [vec![
                        (
                            id("minecraft:stat_type", "minecraft:custom"),
                            id("minecraft:custom_stat", "minecraft:jump"),
                            -7
                        ),
                        (
                            id("minecraft:stat_type", "minecraft:mined"),
                            id("minecraft:block", "minecraft:stone"),
                            0
                        ),
                    ]]
                );
                game.save_connected_guest_statistics(host);
                let path = super::super::stats::player_json(world.path(), "stats", uuid_ints(12));
                let saved: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
                assert_eq!(saved, restored);

                // In a transaction the parked guest's restore waits for the commit: an abort
                // leaves the counter and its score as they were, a commit moves both.
                let jump9 = json!({
                    "DataVersion": version,
                    "stats": {"minecraft:custom": {"minecraft:jump": 9}}
                })
                .to_string();
                use mod_api::HostTransaction as _;
                for commit in [false, true] {
                    let server: &mut dyn ServerHost = game;
                    let result = server.transaction(|server, _| {
                        assert!(player.set_statistics(server, &jump9));
                        assert_eq!(player.stat(server, StatType::Custom, "jump"), Some(-7));
                        if commit { Ok(()) } else { Err(()) }
                    });
                    assert_eq!(result.is_ok(), commit);
                    let expected = if commit { 9 } else { -7 };
                    assert_eq!(player.stat(game, StatType::Custom, "jump"), Some(expected));
                    assert_eq!(
                        game.score.board.score("guest12", "jump").unwrap().value,
                        expected
                    );
                }

                assert!(player.set_statistics(game, &json!({"DataVersion": version}).to_string()));
                let cleared = player.statistics(game).unwrap();
                assert_eq!(serde_json::from_str::<Value>(&cleared).unwrap(), empty);
                assert_eq!(awards.load(Ordering::Relaxed), 0);
            });
        });
    }

    #[test]
    #[ignore = "daily"]
    fn a_resting_guest_reads_from_their_own_record() {
        use std::sync::{Arc, atomic::AtomicI32, mpsc};
        use sunrise_client::lan::{Joined, NetConfig, PlayerSender, guest::Guest, hub::HubConfig};
        use sunrise_protocol::packets::configuration::ClientInformation;

        with_game(|game| {
            let settings = super::super::lan::LanSettings {
                port: 0,
                guest_commands: false,
                force_game_mode: false,
            };
            let net = NetConfig {
                announce: false,
                ..NetConfig::lan(0, "StatsTest".into(), 2)
            };
            let config = HubConfig {
                default_game_mode: 0,
                guest_commands: false,
                force_game_mode: false,
                hardcore: false,
                view_distance: 10,
                simulation_distance: 10,
                dimension: "minecraft:overworld".into(),
                allow_flight: false,
                seed: 0,
                max_players: 2,
                integrated: false,
                levels: Default::default(),
                level_seeds: Default::default(),
            };
            game.lan_host(settings, net, config).unwrap();
            let (tx, _rx) = mpsc::channel();
            let mut guest = Guest::new(
                Joined {
                    conn: 1,
                    uuid: 123,
                    name: "guest".into(),
                    properties: Vec::new(),
                    info: ClientInformation::new(8),
                    brand: String::new(),
                    sender: PlayerSender::new(tx, Arc::default()),
                    latency: Arc::new(AtomicI32::new(0)),
                    address: "127.0.0.1:1".parse().unwrap(),
                    expected: false,
                },
                20,
                [0.0; 3],
                8,
            );
            guest.placed = true;
            guest.recipe_book.recipes.insert("minecraft:stick".into());
            let host = game.lan.as_mut().unwrap();
            host.hub.guests.push(guest);
            let mut play = super::super::lan_actions::GuestPlay::default();
            play.stats.add("custom", "minecraft:jump", 7);
            host.play.insert(1, play);
            game.stats.add("custom", "minecraft:jump", 3);

            let player = game.player(123).unwrap();
            let jump = |game: &super::Game<'_>| game.player_stat(player, StatType::Custom, "jump");
            assert_eq!(jump(game), Some(7));
            assert_eq!(
                game.player_has_discovered_recipe(player, "stick"),
                Some(true)
            );
            assert_eq!(
                game.player_has_discovered_recipe(player, "minecraft:diamond_pickaxe"),
                Some(false)
            );
            let mut recipes = Vec::new();
            assert_eq!(
                player.visit_discovered_recipes(game, &mut |id| {
                    recipes.push(id.to_owned());
                    ControlFlow::Continue(())
                }),
                Ok(ControlFlow::Continue(()))
            );
            assert_eq!(recipes, ["minecraft:stick"]);
            // In the guest's turn their state is in the game, not merged with the resting book.
            game.level.recipe_book.recipes.clear();
            game.lan_acting_uuid = Some(123);
            game.lan_acting_entity = Some(20);
            assert_eq!(jump(game), Some(3));
            assert_eq!(
                player.visit_discovered_recipes(game, &mut |_| panic!("empty acting book")),
                Ok(ControlFlow::Continue(()))
            );
            let book = &mut game.level.recipe_book.recipes;
            book.insert("minecraft:torch".into());
            book.insert("minecraft:oak_planks".into());
            let mut first = Vec::new();
            assert_eq!(
                player.visit_discovered_recipes(game, &mut |id| {
                    first.push(id.to_owned());
                    ControlFlow::Break(())
                }),
                Ok(ControlFlow::Break(()))
            );
            assert_eq!(first, ["minecraft:oak_planks"]);
            game.lan_acting_uuid = None;
            game.lan_acting_entity = None;
            // A guest without play state yet has counted nothing.
            game.lan.as_mut().unwrap().play.remove(&1);
            assert_eq!(jump(game), Some(0));
            game.lan.as_mut().unwrap().hub.guests.remove(0);
            assert_eq!(
                player.visit_discovered_recipes(game, &mut |_| panic!("stale player")),
                Err(EntityReadError::Stale)
            );
        });
    }
}
