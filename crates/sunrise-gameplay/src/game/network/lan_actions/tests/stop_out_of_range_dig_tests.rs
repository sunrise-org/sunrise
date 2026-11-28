//! `ServerPlayerGameMode.handleBlockBreakAction`: an out-of-range
//! `STOP_DESTROY_BLOCK` acknowledges the action but leaves the active dig and
//! its clock alone; only `ABORT_DESTROY_BLOCK` clears them. An instant break is
//! decided at START, so a later tick never breaks the block on its own (#6853).

use std::sync::Arc;
use sunrise_protocol::packets::play::{self, Serverbound as S};

const CASE: &str = "SUNRISE_STOP_OUT_OF_RANGE_DIG_CASE";
const TEST: &str = "game::server_side::lan_actions_server::packets::stop_out_of_range_dig_tests::issue_6853_out_of_range_stop_keeps_the_active_dig";

/// Start a dig on a real block in reach, make it instant, walk away, then STOP:
/// the guest keeps the dig and the `destroyPos` clock, and the next tick leaves
/// the block intact.
#[test]
#[ignore = "daily"]
fn issue_6853_out_of_range_stop_keeps_the_active_dig() {
    if sunrise_gameplay_common::game::test_fixtures::run_fresh_cases(TEST, CASE, &["keep"]) {
        return;
    }
    let mut resources = None;
    let mut game = crate::game::game_tests::tests::server_game(&mut resources);
    let _rx = crate::game::lan::world_spawn_sync_tests::host_for(&mut game);
    game.server.lan.as_mut().unwrap().hub.guests.update(0, |g| {
        g.dimension = "minecraft:overworld".into();
        g.position = [0.5, 64.0, 0.5];
    });

    let stone = game
        .pack
        .assets
        .registry
        .default_state("minecraft:stone")
        .unwrap();
    let torch = game
        .pack
        .assets
        .registry
        .default_state("minecraft:torch")
        .unwrap();
    game.world
        .chunks
        .insert((0, 0), Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)));
    let pos = [2, 64, 0];
    game.world.set(pos[0], pos[1], pos[2], stone).unwrap();

    game.lan_guest_packet(
        1,
        S::PlayerAction(play::PlayerAction {
            action: 0,
            pos,
            direction: 1,
            sequence: 1,
        }),
    );
    let start_tick = {
        let host = game.server.lan.as_ref().unwrap();
        let play = host.play.get(&1).expect("the START ran for the guest");
        assert!(play.digging.is_some(), "the in-reach START began a dig");
        assert_eq!(play.destroy_pos, Some(pos), "the START wrote destroyPos");
        assert!(play.mining.is_some(), "the guest's mining state is kept");
        play.dig_start
    };

    // The block becomes instant after START (a hardness-0 block here), which
    // vanilla's active-dig tick only advances feedback for.
    game.world.set(pos[0], pos[1], pos[2], torch).unwrap();
    // Walk well out of block reach, then STOP the same block.
    game.server.lan.as_mut().unwrap().hub.guests.update(0, |g| {
        g.position = [64.5, 64.0, 0.5];
    });
    game.lan_guest_packet(
        1,
        S::PlayerAction(play::PlayerAction {
            action: 3,
            pos,
            direction: 1,
            sequence: 2,
        }),
    );

    {
        let host = game.server.lan.as_ref().unwrap();
        let play = host.play.get(&1).expect("the STOP ran for the guest");
        assert!(
            play.digging.is_some(),
            "an out-of-range STOP must not end the dig"
        );
        assert_eq!(
            play.destroy_pos,
            Some(pos),
            "the dig's clock survives the refused STOP"
        );
        assert_eq!(play.dig_start, start_tick, "START's tick is unchanged");
        assert!(
            play.mining.is_some(),
            "the guest's mining record survives the refused STOP"
        );
    }
    assert_eq!(
        game.common.world.block(pos[0], pos[1], pos[2]),
        torch,
        "the refused STOP leaves the block alone"
    );

    // The guest's next tick advances feedback only; START's non-instant block
    // must not break just because it became instant afterwards.
    game.tick_guests();
    assert_eq!(
        game.common.world.block(pos[0], pos[1], pos[2]),
        torch,
        "an active dig's tick must not break the block"
    );
}
