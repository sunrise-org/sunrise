use crate::game::native_entity_refs::tests::{
    bind_local_player, prepare_entity_ticking_neighborhood, with_game,
};
use sunrise_client::motion::PlayerMotion;
use sunrise_core::geometry::Vec3;
use sunrise_core::position::ChunkPos;

/// #2690: `/summon lightning_bolt` with an NBT argument makes an ordinary
/// bolt, which strikes and is gone after its life, and is never saved
/// (`EntityType.noSave`); the reply names its `CustomName`.
#[test]
#[ignore = "daily"]
fn a_bolt_summoned_with_nbt_strikes_and_goes() {
    with_game(|game| {
        prepare_entity_ticking_neighborhood(game, ChunkPos::new(0, 0));
        game.player = Some(PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
        bind_local_player(game, 1, 99);
        game.set_allow_commands(true);
        game.build_registries();
        assert!(game.run_command("forceload add 0 0"));
        let others = game.others.len();
        assert!(
            game.run_command("summon lightning_bolt 8.5 64 8.5 {CustomName:\"fz\"}"),
            "{:?}",
            game.ui.chat.recent_lines(3)
        );
        assert_eq!(game.ui.chat.recent_lines(1), ["Summoned new fz"]);
        assert_eq!(game.bolts.len(), 1, "a real bolt");
        assert_eq!(game.others.len(), others, "no stored entity");
        // LightningBolt.tick: each of its up to three flashes lasts at most a
        // dozen ticks before `discard`.
        for _ in 0..60 {
            game.tick_bolts();
        }
        assert!(game.bolts.is_empty(), "gone after its life");
    });
}
