//! The dragon's boss bar follows the fight's player scan (`updatePlayers`,
//! every 20 ticks of `EnderDragonFight.tick`), and the fight ticks while its
//! level's `emptyTime` is under 300 (`ServerLevel.tick`), from the level's
//! opening and whether or not a player is in it. A player who leaves the End
//! is taken off the bar by the next scan, and by nothing else: not the level
//! change, not the client's respawn.
//!
//! The tick counts are vanilla's: the fight's scan counter starts at 21, so
//! its first tick scans; a level holding a ticket that keeps a dimension
//! active resets `emptyTime` to 0 before the running tick counts it to 1, and
//! an idle level counts on (2, 3, ... 299: 298 more fight ticks).
use crate::game::lan::LanSettings;
use crate::game::native_entity_refs::tests::{run_fresh_cases, with_game};
use crate::game::server_travel::LevelTeleport;
use mod_api::{DragonFightTickEvent, Flow, Hooks, ServerHost};
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicI32, AtomicUsize, Ordering},
    mpsc,
};
use sunrise_gameplay_common::game::{
    Dim, chat_common::ChatCommonGameExt1 as _, commands_common::CommandsCommonGameExt2 as _,
};
use sunrise_protocol::{
    buf::Reader,
    packets::{
        configuration::ClientInformation,
        play::{BossAction, Clientbound as C},
    },
};
use sunrise_sim::lan::{
    Joined, NetConfig, Out, PlayerSender,
    guest::{Guest, SPECTATOR},
    hub::HubConfig,
};

/// The fight's tick hook: how often it fired, whether it cancels, and the
/// answers of `LevelRef::dragon_fight_reset_spike_crystals` while it writes.
#[derive(Default)]
struct Log {
    cancel: AtomicBool,
    write: AtomicBool,
    ticks: AtomicUsize,
    writes_done: AtomicUsize,
    writes_refused: AtomicUsize,
}

impl Log {
    fn ticks(&self) -> usize {
        self.ticks.load(Ordering::SeqCst)
    }
}

struct Probe(Arc<Log>);
#[mod_api::__subscriptions]
impl Hooks for Probe {
    fn dragon_fight_tick(
        &self,
        host: &mut dyn ServerHost,
        event: &mut DragonFightTickEvent,
    ) -> Flow {
        self.0.ticks.fetch_add(1, Ordering::SeqCst);
        if self.0.write.load(Ordering::SeqCst) {
            let counter = if event.level.dragon_fight_reset_spike_crystals(host) {
                &self.0.writes_done
            } else {
                &self.0.writes_refused
            };
            counter.fetch_add(1, Ordering::SeqCst);
        }
        if self.0.cancel.load(Ordering::SeqCst) {
            Flow::Cancel
        } else {
            Flow::Continue
        }
    }
}

/// Runs `test` in a fresh process (the game fixture installs process-wide
/// tables) with the probe installed.
fn with_probe(name: &str, test: impl FnOnce(&mut crate::game::ServerGame<'_>, &Log)) -> bool {
    if run_fresh_cases(
        &format!("game::server_side::dragon::idle_fight_tests::{name}"),
        "SUNRISE_IDLE_FIGHT_CHILD",
        &["fight"],
    ) {
        return false;
    }
    let log = Arc::new(Log::default());
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "probe",
        Box::new(Probe(log.clone())),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || with_game(|game| test(game, &log)));
    true
}

/// What a guest's client was sent for the dragon's bar since the last look:
/// `true` an add, `false` a removal.
fn dragon_bar_packets(rx: &mpsc::Receiver<Out>) -> Vec<bool> {
    let dragon = sunrise_game_data::bossbar::stable_uuid("minecraft:ender_dragon");
    rx.try_iter()
        .flat_map(|out| match out {
            Out::Packet(body) => vec![body],
            Out::Packets(bodies) => bodies,
            Out::Disconnect(_) => Vec::new(),
        })
        .filter_map(|body| {
            let mut reader = Reader::new(&body);
            match C::decode(reader.varint().ok()?, reader.rest()).ok()? {
                C::BossEvent(event) if event.id == dragon => match event.action {
                    BossAction::Add { .. } => Some(true),
                    BossAction::Remove => Some(false),
                    _ => None,
                },
                _ => None,
            }
        })
        .collect()
}

const NONE: [bool; 0] = [];
const NEAR: [f64; 3] = [0.5, 128.0, 0.5];
const OVERWORLD: [f64; 3] = [0.5, 100.0, 0.5];

/// A hosting server with no player yet.
fn host(game: &mut crate::game::ServerGame<'_>) {
    game.build_registries();
    game.set_allow_commands(true);
    game.common.player = None;
    game.lan_host(
        LanSettings {
            port: 0,
            guest_commands: true,
            force_game_mode: false,
        },
        NetConfig {
            listen: false,
            announce: false,
            ..NetConfig::loopback(0, "idle-fight-test".into(), 4)
        },
        HubConfig {
            default_game_mode: 0,
            guest_commands: true,
            force_game_mode: false,
            hardcore: false,
            view_distance: 2,
            simulation_distance: 2,
            dimension: "minecraft:overworld".into(),
            allow_flight: false,
            seed: 0,
            max_players: 4,
            integrated: false,
            levels: Default::default(),
            level_seeds: Default::default(),
        },
    )
    .unwrap();
}

/// A network guest placed in `dim` at `at`; its UUID is `330 + conn`.
fn add_guest(
    game: &mut crate::game::ServerGame<'_>,
    conn: u32,
    dim: Dim,
    at: [f64; 3],
) -> mpsc::Receiver<Out> {
    let (tx, rx) = mpsc::channel();
    let mut guest = Guest::new(
        Joined {
            conn,
            uuid: 330 + u128::from(conn),
            name: format!("Guest{conn}"),
            properties: Vec::new(),
            info: ClientInformation::new(2),
            brand: "test".into(),
            sender: PlayerSender::new(tx, Arc::default()),
            latency: Arc::new(AtomicI32::new(0)),
            address: "127.0.0.1:25565".parse().unwrap(),
            expected: false,
        },
        // Clear of the ids the level's own entities (the spike crystals) take.
        10_000 + conn as i32,
        at,
        2,
    );
    guest.dimension = dim.id().into();
    guest.placed = true;
    guest.loaded = true;
    guest.loading = false;
    guest.load_timeout = 0;
    game.server.lan.as_mut().unwrap().hub.guests.push(guest);
    rx
}

/// `TeleportCommand` sending guest `conn` to `dim`.
fn send(game: &mut crate::game::ServerGame<'_>, conn: u32, dim: Dim, at: [f64; 3]) {
    game.apply_direct_level_teleport(LevelTeleport {
        conn,
        dim,
        at,
        rotation: None,
        vehicle: None,
    });
    assert_eq!(game.command_guest(conn).unwrap().dimension, dim.id());
}

fn tick(game: &mut crate::game::ServerGame<'_>, ticks: usize) {
    for _ in 0..ticks {
        game.tick_server(false).unwrap();
    }
}

/// Opens the End (and with it the fight), then puts the Overworld back in
/// place.
fn open_end(game: &mut crate::game::ServerGame<'_>) {
    assert_eq!(game.enter_command_level(Dim::End), Some(Dim::Overworld));
    assert_eq!(game.enter_command_level(Dim::Overworld), Some(Dim::End));
}

/// Two players in the End: one leaves while the other keeps it active. The
/// one who left keeps the bar until the next scan, frozen ticks included, and
/// one who comes back gets it at a scan, not on arrival.
#[test]
#[ignore = "daily"]
fn the_dragon_bar_changes_hands_only_at_the_fights_scans() {
    with_probe(
        "the_dragon_bar_changes_hands_only_at_the_fights_scans",
        |game, log| {
            host(game);
            open_end(game);
            let stays = add_guest(game, 2, Dim::End, NEAR);
            let leaves = add_guest(game, 3, Dim::End, [10.5, 128.0, 0.5]);
            // Tick 1 scans: both join the audience.
            tick(game, 1);
            assert_eq!(dragon_bar_packets(&stays), [true]);
            assert_eq!(dragon_bar_packets(&leaves), [true]);
            // Scan counter 1 to 4.
            tick(game, 4);
            send(game, 3, Dim::Overworld, OVERWORLD);
            assert!(
                game.levels_to_tick().contains(&Dim::End),
                "one player stays"
            );
            // A frozen server runs no fight tick: nothing changes.
            assert!(game.run_command("tick freeze"));
            tick(game, 40);
            assert_eq!(dragon_bar_packets(&leaves), NONE);
            assert!(game.run_command("tick unfreeze"));
            // Counter 5 to 19.
            tick(game, 15);
            assert_eq!(dragon_bar_packets(&leaves), NONE);
            assert_eq!(log.ticks(), 20);
            // Counter 20: the scan leaves out the player who left, only.
            tick(game, 1);
            assert_eq!(dragon_bar_packets(&leaves), [false]);
            assert_eq!(dragon_bar_packets(&stays), NONE);
            // Back in the End: no bar before the next scan, 20 ticks on.
            send(game, 3, Dim::End, [10.5, 128.0, 0.5]);
            tick(game, 19);
            assert_eq!(dragon_bar_packets(&leaves), NONE);
            tick(game, 1);
            assert_eq!(dragon_bar_packets(&leaves), [true]);
            assert_eq!(dragon_bar_packets(&stays), NONE);
        },
    );
}

/// One player leaves the End, which goes idle: the fight keeps ticking (its
/// hook with the End in place, so the documented writes work) and its next
/// scan removes the bar. After 300 empty ticks it stops; a player coming
/// back starts it again.
#[test]
#[ignore = "daily"]
fn a_player_who_left_the_end_leaves_the_dragon_bar_at_the_fights_next_scan() {
    with_probe(
        "a_player_who_left_the_end_leaves_the_dragon_bar_at_the_fights_next_scan",
        |game, log| {
            host(game);
            open_end(game);
            let rx = add_guest(game, 2, Dim::End, NEAR);
            // Ticks 1 to 5: the first scans and adds the bar.
            tick(game, 5);
            assert_eq!(dragon_bar_packets(&rx), [true]);
            assert_eq!(log.ticks(), 5);

            // `/execute in minecraft:overworld run tp`: nothing about the level
            // change removes the bar.
            send(game, 2, Dim::Overworld, OVERWORLD);
            assert!(
                !game.levels_to_tick().contains(&Dim::End),
                "the End is idle"
            );
            assert!(game.run_command("tick freeze"));
            tick(game, 40);
            assert_eq!(dragon_bar_packets(&rx), NONE);
            assert_eq!(log.ticks(), 5, "no fight tick while frozen");
            assert!(game.run_command("tick unfreeze"));
            // Counter 5 to 19 in the idle End, the hook writing as it runs.
            log.write.store(true, Ordering::SeqCst);
            tick(game, 15);
            log.write.store(false, Ordering::SeqCst);
            assert_eq!(dragon_bar_packets(&rx), NONE);
            assert_eq!(log.ticks(), 20, "the idle End's fight ticks");
            assert_eq!(log.writes_done.load(Ordering::SeqCst), 15);
            assert_eq!(log.writes_refused.load(Ordering::SeqCst), 0);
            // Counter 20: the scan finds nobody in the End.
            tick(game, 1);
            assert_eq!(dragon_bar_packets(&rx), [false]);

            // Back, then away again with every fight tick canceled: the hook
            // fires for `emptyTime` 2 to 299 and the scan never runs.
            send(game, 2, Dim::End, NEAR);
            tick(game, 20);
            assert_eq!(dragon_bar_packets(&rx), [true], "the scan on counter 20");
            send(game, 2, Dim::Overworld, OVERWORLD);
            log.cancel.store(true, Ordering::SeqCst);
            let before = log.ticks();
            tick(game, 298);
            assert_eq!(log.ticks() - before, 298);
            // `emptyTime` 300 on: no fight tick, so no scan, and the bar stays.
            log.cancel.store(false, Ordering::SeqCst);
            tick(game, 40);
            assert_eq!(log.ticks() - before, 298);
            assert_eq!(dragon_bar_packets(&rx), NONE);

            // A player in the End resets `emptyTime`: the fight ticks again,
            // its counter going on from 0. Five ticks there, then the idle
            // End's scan on counter 20 removes the bar.
            send(game, 2, Dim::End, NEAR);
            tick(game, 5);
            assert_eq!(log.ticks() - before, 303);
            send(game, 2, Dim::Overworld, OVERWORLD);
            tick(game, 14);
            assert_eq!(dragon_bar_packets(&rx), NONE);
            tick(game, 1);
            assert_eq!(dragon_bar_packets(&rx), [false]);
        },
    );
}

/// The fight is set up as the End opens and ticks for its first 299 running
/// ticks with nobody there. A spectator the rule keeps from loading chunks
/// holds no ticket that keeps the End active, so although Sunrise gives the
/// End its pass, the fight stops 300 ticks after the last other player left.
#[test]
#[ignore = "daily"]
fn the_fight_ticks_from_its_levels_opening_and_not_for_a_lone_skipped_spectator() {
    with_probe(
        "the_fight_ticks_from_its_levels_opening_and_not_for_a_lone_skipped_spectator",
        |game, log| {
            host(game);
            open_end(game);
            tick(game, 320);
            assert_eq!(log.ticks(), 299, "emptyTime 1 to 299");

            assert!(game.run_command("gamerule spectators_generate_chunks false"));
            let rx = add_guest(game, 2, Dim::End, NEAR);
            let before = log.ticks();
            // The player's ticket resets `emptyTime`. The empty ticks scanned
            // on ticks 1, 21, ... 281 and left the counter at 18; this tick
            // makes it 19, so the next tick scans.
            tick(game, 1);
            assert_eq!(log.ticks() - before, 1, "the fight ticks again");
            game.server
                .lan
                .as_mut()
                .unwrap()
                .hub
                .guests
                .update(0, |g| g.game_mode = SPECTATOR);
            let before = log.ticks();
            tick(game, 340);
            assert!(game.levels_to_tick().contains(&Dim::End), "Sunrise's pass");
            assert_eq!(log.ticks() - before, 298, "emptyTime 2 to 299");
            // A spectator is a valid player: the scans that ran added it.
            assert_eq!(dragon_bar_packets(&rx), [true]);
        },
    );
}
