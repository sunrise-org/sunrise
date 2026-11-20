use crate::game::native_entity_refs::tests::with_game;
use sunrise_client::motion::PlayerMotion;
use sunrise_core::geometry::Vec3;
use sunrise_core::nbt::Tag;

/// #2692: only `ArmorStand.causeDamage` breaks a stand on low health. One at
/// 0.3 stays; one at 0 is dead and `LivingEntity.tickDeath` counts its
/// `DeathTime` up and removes it at 20, dropping nothing.
#[test]
#[ignore = "daily"]
fn a_stand_breaks_on_low_health_only_through_damage_and_a_dead_one_goes_at_twenty() {
    with_game(|game| {
        game.player = Some(PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
        game.update_chunk_tickets();
        for (x, nbt) in [
            (0.5, "{Health:0.3f,NoGravity:1b}"),
            (
                4.5,
                "{Health:0.0f,NoGravity:1b,equipment:{mainhand:{id:\"minecraft:stone\",count:1}}}",
            ),
        ] {
            game.summon_with_nbt("minecraft:armor_stand", [x, 64.0, 0.5], nbt)
                .unwrap();
        }
        let (weak, dead) = (game.armor_stands[0].id, game.armor_stands[1].id);
        let death_time = |game: &crate::game::Game<'_>| {
            let a = game.armor_stands.iter().find(|a| a.id == dead).unwrap();
            sunrise_client::entity_nbt::armor_stand_to_nbt(a, [1, 2, 3, 4], None, game.res.items())
                .get("DeathTime")
                .cloned()
        };
        game.tick_armor_stands().unwrap();
        assert_eq!(death_time(game), Some(Tag::Short(1)));
        for _ in 1..19 {
            game.tick_armor_stands().unwrap();
        }
        assert_eq!(death_time(game), Some(Tag::Short(19)));
        game.tick_armor_stands().unwrap();
        assert!(game.armor_stands.iter().all(|a| a.id != dead), "gone at 20");
        for _ in 20..40 {
            game.tick_armor_stands().unwrap();
        }
        let weak = game.armor_stands.iter().find(|a| a.id == weak);
        assert_eq!(weak.map(|a| a.health), Some(0.3), "0.3 is not broken");
        assert!(game.dropped.is_empty(), "nothing dropped");
    });
}
