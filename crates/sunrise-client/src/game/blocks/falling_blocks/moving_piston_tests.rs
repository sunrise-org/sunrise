use crate::game::native_entity_refs::tests::{prepare_entity_ticking_neighborhood, with_game};
use sunrise_client::motion::PlayerMotion;
use sunrise_core::geometry::Vec3;
use sunrise_core::position::ChunkPos;

/// #2363: `FallingBlockEntity.tick` skips its place, drop and discard branch
/// when it rests in a `moving_piston` cell: it stays, its motion scaled by
/// (0.7, -0.5, 0.7) and then by the 0.98F drag.
#[test]
#[ignore = "daily"]
fn a_falling_block_resting_in_a_moving_piston_cell_stays() {
    with_game(|game| {
        prepare_entity_ticking_neighborhood(game, ChunkPos::new(0, 0));
        game.player = Some(PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
        crate::game::native_entity_refs::tests::bind_local_player(game, 1, 99);
        game.set_allow_commands(true);
        game.build_registries();
        assert!(game.run_command("forceload add 0 0 31 15"));
        for command in [
            "fill 0 63 0 4 63 4 minecraft:stone",
            "setblock 2 64 2 minecraft:moving_piston",
        ] {
            assert!(game.run_command(command), "{command}");
        }
        let target = game
            .summon_with_nbt(
                "minecraft:falling_block",
                [2.5, 64.0, 2.5],
                "{BlockState:{Name:\"minecraft:sand\"},Time:1,Motion:[0.1d,0.0d,0.0d]}",
            )
            .unwrap()
            .unwrap();
        let crate::game::commands::Target::Falling(id) = target else {
            panic!("{target:?}")
        };
        let i = game.falling.iter().position(|f| f.id == id).unwrap();
        game.tick_one_falling(i, &[]).unwrap();
        let f = game
            .falling
            .iter()
            .find(|f| f.id == id)
            .expect("the block stays");
        assert_eq!(f.velocity.x, 0.1 * 0.7 * f64::from(0.98_f32));
        assert_eq!((f.velocity.y, f.velocity.z), (0.0, 0.0));
        assert_eq!(
            &*game.res.facts().get(game.world.block(2, 64, 2)).name,
            "minecraft:moving_piston",
            "nothing was placed"
        );
        assert!(game.dropped.is_empty(), "nothing dropped");
    });
}
