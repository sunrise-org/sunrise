use crate::game::native_entity_refs::tests::with_game;
use sunrise_client::items::ItemStack;
use sunrise_client::motion::PlayerMotion;
use sunrise_core::geometry::Vec3;

/// #2361: `ServerLevel.tick` ticks only the entities in entity-ticking
/// chunks, so fangs and an ominous item spawner outside them stand still.
#[test]
#[ignore = "daily"]
fn fangs_and_an_ominous_item_spawner_outside_the_ticking_chunks_do_not_tick() {
    with_game(|game| {
        // A player's tickets: chunks out of the simulation distance around
        // them do not tick entities.
        game.player = Some(PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
        game.update_chunk_tickets();
        let (near, far) = ([0.5, 64.0, 0.5], [100000.5, 64.0, 0.5]);
        game.add_fangs(near, 0.0, 5, 0);
        game.add_fangs(far, 0.0, 5, 0);
        let stone = || ItemStack {
            item: "minecraft:stone".into(),
            count: 1,
            damage: 0,
            extra: None,
        };
        game.add_ominous_item_spawner(stone(), near, 1);
        game.add_ominous_item_spawner(stone(), far, 1);
        game.tick_fangs();
        game.tick_ominous_item_spawners();
        let warmup = |at: [f64; 3]| {
            game.fangs
                .list
                .iter()
                .find(|f| f.pos == at)
                .map(|f| f.warmup)
        };
        assert_eq!(warmup(near), Some(4));
        assert_eq!(warmup(far), Some(5), "outside the ticking chunks");
        let spawners: Vec<_> = game.ominous_spawners.iter().map(|s| s.position).collect();
        assert_eq!(spawners, [far], "only the ticking spawner spawned its item");
    });
}
