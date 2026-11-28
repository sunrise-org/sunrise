//! `handlePickItemFromBlock` and the container/crafting-table validity checks
//! read `minecraft:block_interaction_range` live and measure the block AABB
//! through the shared `within_block_range` helper (#6828).

use std::sync::Arc;
use sunrise_core::geometry::Vec3;
use sunrise_level::chunk::Chunk;
use sunrise_protocol::packets::play::{self, Serverbound as S};
use sunrise_sim::lan::menu::Slots;

const MENUS_CASE: &str = "SUNRISE_LIVE_BLOCK_REACH_MENUS_CASE";
const MENUS_TEST: &str = "game::server_side::lan_actions_server::packets::live_block_reach_tests::issue_6828_menus_follow_the_live_block_range";
const PICK_CASE: &str = "SUNRISE_LIVE_BLOCK_REACH_PICK_CASE";
const PICK_TEST: &str = "game::server_side::lan_actions_server::packets::live_block_reach_tests::issue_6828_pick_accepts_the_strict_boundary";

/// An attribute edit after the tick's cached range changes whether an open chest
/// and crafting-table menu stay open.
#[test]
#[ignore = "daily"]
fn issue_6828_menus_follow_the_live_block_range() {
    if sunrise_gameplay_common::game::test_fixtures::run_fresh_cases(
        MENUS_TEST,
        MENUS_CASE,
        &["menus"],
    ) {
        return;
    }
    let mut resources = None;
    let mut game = crate::game::game_tests::tests::server_game(&mut resources);
    let _rx = crate::game::lan::world_spawn_sync_tests::host_for(&mut game);
    game.world
        .chunks
        .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
    let chest = (8, 64, 0);
    game.world.block_entities.insert(
        chest,
        sunrise_level::containers::BlockEntity::Chest(vec![None; 27]),
    );
    let table = (0, 64, 8);
    let table_state = game
        .pack
        .assets
        .registry
        .default_state("minecraft:crafting_table")
        .unwrap();
    game.world
        .set(table.0, table.1, table.2, table_state)
        .unwrap();
    let motion =
        sunrise_sim::motion::PlayerMotion::new(game.motion, Vec3::new(0.5, 64.0, 0.5)).unwrap();
    game.server.lan.as_mut().unwrap().hub.guests.update(0, |g| {
        g.dimension = "minecraft:overworld".into();
        g.position = [0.5, 64.0, 0.5];
        g.motion = Some(Box::new(motion));
    });
    let set_range = |game: &mut crate::game::ServerGame<'_>, amount: f64| {
        game.server.lan.as_mut().unwrap().hub.guests.update(0, |g| {
            g.motion
                .as_mut()
                .unwrap()
                .set_modifier(
                    "minecraft:block_interaction_range",
                    "sunrise:test",
                    amount,
                    "add_value",
                    true,
                )
                .unwrap();
        });
    };
    let open_chest = |game: &mut crate::game::ServerGame<'_>| {
        let host = game.server.lan.as_mut().unwrap();
        host.hub.open_container(
            1,
            "minecraft:generic_9x3",
            sunrise_core::nbt::Tag::String("test".into()),
            chest,
            Slots::Block(chest),
            vec![None; 27],
            None,
        );
    };
    let menu_open = |game: &crate::game::ServerGame<'_>| {
        game.server.lan.as_ref().unwrap().hub.guests[0]
            .menu
            .is_some()
    };

    // The chest is 8 blocks away: inside 4.5 + 4, so the tick keeps it open.
    set_range(&mut game, 0.0);
    open_chest(&mut game);
    game.lan_containers_tick();
    assert!(menu_open(&game), "the chest is in default reach");
    // -2 leaves 2.5 + 4 = 6.5, so the live read closes it without waiting for a refresh.
    set_range(&mut game, -2.0);
    game.lan_containers_tick();
    assert!(!menu_open(&game), "the lowered live range closes the chest");
    // +1 makes 5.5 + 4 = 9.5: the same menu reopens and stays open.
    set_range(&mut game, 1.0);
    open_chest(&mut game);
    game.lan_containers_tick();
    assert!(
        menu_open(&game),
        "the raised live range keeps the chest open"
    );

    // The crafting table follows the same live range.
    game.guest_close_all(1);
    assert_eq!(
        game.guest_may_open(1, "minecraft:crafting", Some([table.0, table.1, table.2])),
        Some(true),
        "the guest may open the crafting table"
    );
    game.lan_open_crafting(1, [table.0, table.1, table.2]);
    let table_open = |game: &crate::game::ServerGame<'_>| {
        game.server
            .lan
            .as_ref()
            .unwrap()
            .menus
            .get(&1)
            .is_some_and(|m| m.crafting.table.is_some())
    };
    assert!(table_open(&game), "the crafting table menu is recorded");
    assert!(
        game.guest_menu_still_valid(1),
        "the table is in the raised reach"
    );
    set_range(&mut game, -2.0);
    assert!(
        !game.guest_menu_still_valid(1),
        "the lowered live range invalidates the table"
    );
}

/// A block one unit below x = 7 whose distance is just under the 7-block range:
/// vanilla accepts it; subtracting `lo + 1` from `lo` must not round it up.
#[test]
#[ignore = "daily"]
fn issue_6828_pick_accepts_the_strict_boundary() {
    if sunrise_gameplay_common::game::test_fixtures::run_fresh_cases(
        PICK_TEST,
        PICK_CASE,
        &["pick"],
    ) {
        return;
    }
    let mut resources = None;
    let mut game = crate::game::game_tests::tests::server_game(&mut resources);
    let _rx = crate::game::lan::world_spawn_sync_tests::host_for(&mut game);
    for chunk in [(-1, 0), (0, 0)] {
        game.world
            .chunks
            .insert(chunk, Arc::new(Chunk::empty(chunk.0, chunk.1)));
    }
    let pos = [-1, 65, 0];
    let stone = game
        .pack
        .assets
        .registry
        .default_state("minecraft:stone")
        .unwrap();
    game.world.set(pos[0], pos[1], pos[2], stone).unwrap();
    assert_eq!(
        game.world.block(pos[0], pos[1], pos[2]),
        stone,
        "the pick block is placed"
    );
    // Eyes at the largest f64 below x = 7: the box's max face is x = 0, so the
    // distance is 7 - one ulp, strictly inside a total reach of 7.
    let eye_x = f64::from_bits(7.0f64.to_bits() - 1);
    let mut motion =
        sunrise_sim::motion::PlayerMotion::new(game.motion, Vec3::new(eye_x, 64.0, 0.5)).unwrap();
    // The live range is 6.0; handlePickItemFromBlock adds its 1.0 buffer.
    let base = motion
        .attribute("minecraft:block_interaction_range")
        .unwrap();
    motion
        .set_modifier(
            "minecraft:block_interaction_range",
            "sunrise:test",
            6.0 - base,
            "add_value",
            true,
        )
        .unwrap();
    game.server.lan.as_mut().unwrap().hub.guests.update(0, |g| {
        g.dimension = "minecraft:overworld".into();
        g.position = [eye_x, 64.0, 0.5];
        g.motion = Some(Box::new(motion));
        g.game_mode = sunrise_sim::lan::guest::CREATIVE;
    });
    game.lan_guest_packet(
        1,
        S::PickItemFromBlock(play::PickItemFromBlock {
            pos,
            include_data: false,
        }),
    );
    let guest = &game.server.lan.as_ref().unwrap().hub.guests[0];
    assert!(
        guest
            .inventory
            .iter()
            .flatten()
            .any(|stack| stack.item.as_ref() == "minecraft:stone"),
        "the strict boundary pick reaches the block: {:?}",
        guest.inventory
    );
}
