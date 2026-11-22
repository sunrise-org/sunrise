//! #1278: the guest a mob targets leaves its level while a bystander guest
//! stays. On the hosted tick the mob keeps that departed player, never the
//! bystander, until its own target goal lets go: at the goal selector's first
//! full pass for a guest who disconnected (`Mob.serverAiStep` runs it in full
//! when `(tickCount + id) % 2 == 0`), and only once its lost-sight memory runs
//! out for one who went to another dimension, as the same live entity.
use crate::game::*;
use std::sync::Arc;
use sunrise_client::{chunk::Chunk, mobs::Who};
use sunrise_core::position::ChunkPos;

/// The guest the zombie targets, and the bystander nearer to it.
pub(in crate::game) const LEAVER: u32 = 12;
pub(in crate::game) const BYSTANDER: u32 = 13;
const ZOMBIE_UUID: [i32; 4] = [24733, 0, 0, 1];

/// A served level open to the network, with no host player: a zombie that
/// targets guest `LEAVER`, and guest `BYSTANDER` in survival, nearer to it.
/// The ticks are frozen; [`step`] runs one.
pub(in crate::game) fn scene<'a>(
    resources: &'a mut Option<Arc<crate::game::Resources>>,
) -> (Game<'a>, u32) {
    use crate::game::native_item_places::tests::{guest, open_to_lan};
    let mut game = crate::game::game_tests::tests::server_game(resources);
    game.build_registries();
    game.set_allow_commands(true);
    for x in 0..2 {
        game.world
            .chunks
            .insert((x, 0), Arc::new(Chunk::empty(x, 0)));
        game.block_ticks
            .add_empty_chunk(ChunkPos::new(x, 0))
            .unwrap();
    }
    for x in 0..2 {
        game.load_ticking_neighbourhood(x, 0);
    }
    open_to_lan(&mut game);
    for (conn, x) in [(LEAVER, 10.5), (BYSTANDER, 6.5)] {
        let mut placed = guest(conn);
        placed.loaded = true;
        placed.position = [x, 100.0, 8.5];
        placed.previous_position = placed.position;
        game.lan.as_mut().unwrap().hub.guests.push(placed);
    }
    for command in [
        "forceload add 0 0 31 15".to_owned(),
        "tick freeze".to_owned(),
        "difficulty easy".to_owned(),
        "time set midnight".to_owned(),
        "gamerule advance_time false".to_owned(),
        "gamerule spawn_mobs false".to_owned(),
        "fill 0 99 0 31 99 15 stone".to_owned(),
        format!("gamemode creative guest{BYSTANDER}"),
        format!("gamemode survival guest{LEAVER}"),
        format!(
            "summon zombie 4.5 100 8.5 {{PersistenceRequired:1b,CanPickUpLoot:0b,UUID:[I;{},{},{},{}]}}",
            ZOMBIE_UUID[0], ZOMBIE_UUID[1], ZOMBIE_UUID[2], ZOMBIE_UUID[3]
        ),
    ] {
        assert!(game.run_command(&command), "{command}");
    }
    let id = match game.target_with_uuid(ZOMBIE_UUID) {
        Some(crate::game::commands::Target::Mob(id)) => id,
        other => panic!("{other:?}"),
    };
    // The zombie takes the only survival player it sees.
    for _ in 0..200 {
        step(&mut game);
        if targets(&game, id) == Some(LEAVER) {
            break;
        }
    }
    assert_eq!(
        targets(&game, id),
        Some(LEAVER),
        "the zombie takes the leaver"
    );
    // Now the bystander, nearer, could be taken too.
    assert!(game.run_command(&format!("gamemode survival guest{BYSTANDER}")));
    (game, id)
}

pub(in crate::game) fn step(game: &mut Game<'_>) {
    assert!(game.run_command("tick step 1"));
    game.tick_server(false).unwrap();
}

pub(in crate::game) fn zombie<'g>(game: &'g Game<'_>, id: u32) -> &'g sunrise_client::mobs::Mob {
    game.mobs.iter().find(|m| m.id == id).expect("the zombie")
}

/// The guest whose player the mob targets, if it targets a player.
pub(in crate::game) fn targets(game: &Game<'_>, id: u32) -> Option<u32> {
    (zombie(game, id).ai.target == Some(Who::Player))
        .then(|| game.lan.as_ref()?.mob_players.get(&id).copied())
        .flatten()
}

/// Whether the mob's last tick ran its goal selectors in full
/// (`Mob.serverAiStep`; the tick counted its age up first).
pub(in crate::game) fn full_pass(game: &Game<'_>, id: u32) -> bool {
    let m = zombie(game, id);
    let ticks = i64::from(m.age);
    !((ticks + i64::from(m.id)) % 2 != 0 && ticks > 1)
}

pub(in crate::game) fn disconnect(game: &mut Game<'_>, conn: u32) {
    game.lan
        .as_mut()
        .unwrap()
        .hub
        .guests
        .retain(|g| g.conn != conn);
}

fn travel(game: &mut Game<'_>, conn: u32) {
    let hub = &mut game.lan.as_mut().unwrap().hub;
    let gi = hub.guests.connection_index(conn).unwrap();
    hub.guests
        .update(gi, |g| g.dimension = "minecraft:the_nether".into());
}

/// Ticks until the first full pass: the target holds on the running-only
/// tick before it, and that pass drops it.
pub(in crate::game) fn until_full_pass(game: &mut Game<'_>, id: u32) {
    for _ in 0..2 {
        step(game);
        if full_pass(game, id) {
            return;
        }
        assert_eq!(
            targets(game, id),
            Some(LEAVER),
            "a running-only tick keeps the departed target"
        );
    }
    panic!("no full pass in two ticks");
}

#[test]
#[ignore = "daily"]
fn a_disconnected_target_drops_at_the_first_full_selector_pass() {
    let mut resources = None;
    let (mut game, id) = scene(&mut resources);
    disconnect(&mut game, LEAVER);
    until_full_pass(&mut game, id);
    assert_eq!(zombie(&game, id).ai.target, None, "the full pass drops it");
    assert_ne!(
        game.lan.as_ref().unwrap().mob_players.get(&id),
        Some(&BYSTANDER)
    );
}

#[test]
#[ignore = "daily"]
fn a_target_in_another_dimension_is_kept_until_the_zombie_loses_it() {
    let mut resources = None;
    let (mut game, id) = scene(&mut resources);
    travel(&mut game, LEAVER);
    // Out of sight but alive and in follow range: several full passes keep it.
    for tick in 0..20 {
        step(&mut game);
        assert_eq!(targets(&game, id), Some(LEAVER), "tick {tick}");
    }
    // `TargetGoal.unseenMemoryTicks` runs out within a few seconds.
    let mut dropped = false;
    for _ in 0..200 {
        step(&mut game);
        match zombie(&game, id).ai.target {
            None => {
                dropped = true;
                break;
            }
            Some(_) => assert_eq!(targets(&game, id), Some(LEAVER), "never the bystander"),
        }
    }
    assert!(dropped, "lost sight ends it");
}

#[test]
#[ignore = "daily"]
fn a_frozen_turn_keeps_the_departed_target_and_drops_it_on_resume() {
    let mut resources = None;
    let (mut game, id) = scene(&mut resources);
    disconnect(&mut game, LEAVER);
    // Frozen ticks: no mob turn runs at all.
    for _ in 0..5 {
        game.tick_server(false).unwrap();
        assert_eq!(targets(&game, id), Some(LEAVER), "frozen");
    }
    // A mob whose AI is off ticks, but no goal runs: the target stays, and
    // the binding stays on the departed guest.
    game.mobs.iter_mut().find(|m| m.id == id).unwrap().no_ai = true;
    for _ in 0..6 {
        step(&mut game);
        assert_eq!(targets(&game, id), Some(LEAVER), "no AI");
    }
    game.mobs.iter_mut().find(|m| m.id == id).unwrap().no_ai = false;
    until_full_pass(&mut game, id);
    assert_eq!(
        zombie(&game, id).ai.target,
        None,
        "resumed, the full pass drops it"
    );
}

/// The guest a golem is angry at, or a mob was just hurt by, leaves: the
/// bystander never inherits the anger or the retaliation.
#[test]
#[ignore = "daily"]
fn a_departed_attacker_hands_nothing_to_the_bystander() {
    let mut resources = None;
    let (mut game, id) = scene(&mut resources);
    // The zombie, hurt by the leaver and no longer targeting anyone.
    {
        let m = game.mobs.iter_mut().find(|m| m.id == id).unwrap();
        m.ai.target = None;
        m.ai.last_hurt_by = Some(Who::Player);
        m.ai.last_hurt_by_time = m.age as i32;
    }
    disconnect(&mut game, LEAVER);
    for tick in 0..20 {
        step(&mut game);
        assert_ne!(targets(&game, id), Some(BYSTANDER), "tick {tick}");
    }
    // An iron golem angry at the leaver.
    let mut resources = None;
    let (mut game, _) = scene(&mut resources);
    assert!(game.run_command(
        "summon iron_golem 2.5 100 8.5 {PersistenceRequired:1b,UUID:[I;24733,0,0,2]}"
    ));
    let golem = match game.target_with_uuid([24733, 0, 0, 2]) {
        Some(crate::game::commands::Target::Mob(id)) => id,
        other => panic!("{other:?}"),
    };
    {
        let m = game.mobs.iter_mut().find(|m| m.id == golem).unwrap();
        m.extra.anger = 300;
        m.extra.angry_at = Some(Who::Player);
    }
    game.lan.as_mut().unwrap().mob_players.insert(golem, LEAVER);
    disconnect(&mut game, LEAVER);
    for tick in 0..20 {
        step(&mut game);
        let m = game.mobs.iter().find(|m| m.id == golem).unwrap();
        let bound = game.lan.as_ref().unwrap().mob_players.get(&golem);
        assert!(
            m.ai.target != Some(Who::Player) || bound != Some(&BYSTANDER),
            "tick {tick}: the golem turned on the bystander"
        );
    }
}
