use crate::game::native_entity_refs::tests::{prepare_entity_ticking_neighborhood, with_game};
use sunrise_client::motion::PlayerMotion;
use sunrise_core::geometry::Vec3;
use sunrise_core::nbt::Tag;
use sunrise_core::position::ChunkPos;

/// #2359: `Entity.updateFluidInteraction` resets the fall distance of an
/// entity whose box touches water; a cloud's box is `radius * 2` wide.
#[test]
#[ignore = "daily"]
fn a_cloud_in_water_resets_its_fall_distance_and_a_dry_one_keeps_it() {
    with_game(|game| {
        prepare_entity_ticking_neighborhood(game, ChunkPos::new(0, 0));
        game.player = Some(PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
        game.update_chunk_tickets();
        crate::game::native_entity_refs::tests::bind_local_player(game, 1, 99);
        game.set_allow_commands(true);
        game.build_registries();
        assert!(game.run_command("forceload add 0 0 31 15"));
        assert!(game.run_command("setblock 1 64 0 minecraft:water"));
        let mut ids = Vec::new();
        for x in ["0.5", "20.5"] {
            assert!(game.run_command(&format!(
                "summon minecraft:area_effect_cloud {x} 64 0.5 {{fall_distance:20.0d,Duration:200}}"
            )));
            ids.push(game.breath_clouds.last().unwrap().id);
        }
        game.tick_potion_clouds();
        // The chunk writer, which `/data get` reads the live cloud through too.
        let fall = |game: &mut crate::game::Game<'_>, id: u32| {
            let cloud = game.breath_clouds.iter().find(|c| c.id == id).unwrap();
            super::potion_cloud_to_nbt(cloud, [1, 2, 3, 4], game.persist_raw(id))
                .get("fall_distance")
                .cloned()
        };
        assert_eq!(
            fall(game, ids[0]),
            Some(Tag::Double(0.0_f64.to_bits())),
            "beside water"
        );
        assert_eq!(
            fall(game, ids[1]),
            Some(Tag::Double(20.0_f64.to_bits())),
            "dry"
        );
    });
}
