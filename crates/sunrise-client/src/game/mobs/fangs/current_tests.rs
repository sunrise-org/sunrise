use crate::game::native_entity_refs::tests::{prepare_entity_ticking_neighborhood, with_game};
use sunrise_client::motion::PlayerMotion;
use sunrise_core::geometry::Vec3;
use sunrise_core::position::ChunkPos;

/// #2362: `EvokerFangs.tick` runs `Entity.baseTick`, whose
/// `updateFluidInteraction` adds a current to the fangs' motion (a push under
/// 0.0045 is raised to 0.0045), and `saveWithoutId` writes it as `Motion`.
#[test]
#[ignore = "daily"]
fn fangs_in_a_current_are_pushed_and_fangs_in_still_water_are_not() {
    with_game(|game| {
        prepare_entity_ticking_neighborhood(game, ChunkPos::new(0, 0));
        game.player = Some(PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
        game.update_chunk_tickets();
        crate::game::native_entity_refs::tests::bind_local_player(game, 1, 99);
        game.set_allow_commands(true);
        game.build_registries();
        assert!(game.run_command("forceload add 0 0 31 15"));
        for command in [
            "fill -4 63 -4 12 63 4 minecraft:stone",
            // Lava flowing north (-z) from its source through the fangs' cell.
            "setblock 0 64 1 minecraft:lava",
            "setblock 0 64 0 minecraft:lava[level=2]",
            // A still pool, walled in on every side.
            "fill 7 64 -1 9 64 1 minecraft:stone",
            "setblock 8 64 0 minecraft:water",
        ] {
            assert!(game.run_command(command), "{command}");
        }
        game.insert_fangs(900, [0.5, 64.0, 0.5], 0.0, 100, 0);
        game.insert_fangs(901, [8.5, 64.0, 0.5], 0.0, 100, 0);
        game.tick_fangs();
        let motion = |id| game.fangs.list.iter().find(|f| f.id == id).unwrap().motion;
        let pushed = motion(900);
        assert_eq!((pushed.x, pushed.y), (0.0, 0.0), "{pushed:?}");
        assert!(pushed.z < 0.0, "{pushed:?}");
        assert!((pushed.length() - 0.0045).abs() < 1e-12, "{pushed:?}");
        assert_eq!(motion(901), Vec3::ZERO, "still water has no current");
        let saved = game.fangs_to_nbt(
            game.fangs.list.iter().find(|f| f.id == 900).unwrap(),
            [1, 2, 3, 4],
        );
        assert_eq!(
            sunrise_client::entity_nbt::motion(&saved),
            pushed,
            "the save writes the live motion"
        );
    });
}
