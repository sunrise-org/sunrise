use super::{DataRef, Target};
use crate::game::native_entity_refs::tests::with_game;
use sunrise_client::motion::PlayerMotion;
use sunrise_core::geometry::Vec3;
use sunrise_core::nbt::Tag;

fn fire(game: &mut crate::game::Game<'_>, target: Target) -> Option<Tag> {
    game.data_of(&DataRef::Entity(target))
        .ok()
        .and_then(|tag| tag.get("Fire").cloned())
}

/// #2683: `Entity.baseTick` counts `Fire` down for an eye of ender, evoker
/// fangs and an ominous item spawner, clears it for a fire-immune area
/// effect cloud, and hurts a burning shulker bullet every 20 ticks, which
/// destroys it; `applyEffectsFromBlocks` resets a bullet's unlit `-5` to 0.
#[test]
#[ignore = "daily"]
fn fire_changes_on_eyes_fangs_spawners_clouds_and_bullets() {
    with_game(|game| {
        game.player = Some(PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
        game.update_chunk_tickets();
        let summon = |game: &mut crate::game::Game<'_>, kind: &str, x: f64, nbt: &str| {
            game.summon_with_nbt(kind, [x, 64.0, 0.5], nbt).unwrap()
        };
        let eye = summon(game, "minecraft:eye_of_ender", 0.5, "{Fire:40s}").unwrap();
        summon(game, "minecraft:evoker_fangs", 2.5, "{Fire:40s,Warmup:100}");
        let fangs = Target::Fangs(game.fangs.list.last().unwrap().id);
        let spawner = summon(
            game,
            "minecraft:ominous_item_spawner",
            4.5,
            "{Fire:40s,spawn_item_after_ticks:100L,item:{id:\"minecraft:stone\",count:1}}",
        )
        .unwrap();
        summon(
            game,
            "minecraft:area_effect_cloud",
            6.5,
            "{Fire:40s,Duration:200}",
        );
        let cloud = Target::Cloud(game.breath_clouds.last().unwrap().id);
        let unlit = summon(
            game,
            "minecraft:shulker_bullet",
            8.5,
            "{Fire:-5s,NoGravity:1b}",
        )
        .unwrap();
        let burning = summon(
            game,
            "minecraft:shulker_bullet",
            10.5,
            "{Fire:40s,NoGravity:1b}",
        )
        .unwrap();
        game.ticked_entities_through = u32::MAX;
        game.tick_summoned_eyes();
        game.tick_fangs();
        game.tick_ominous_item_spawners();
        game.tick_potion_clouds();
        game.tick_projectiles_now().unwrap();
        assert_eq!(fire(game, eye), Some(Tag::Short(39)), "eye");
        assert_eq!(fire(game, fangs), Some(Tag::Short(39)), "fangs");
        assert_eq!(fire(game, spawner), Some(Tag::Short(39)), "spawner");
        assert_eq!(fire(game, cloud), Some(Tag::Short(0)), "cloud");
        assert_eq!(fire(game, unlit), Some(Tag::Short(0)), "unlit bullet");
        let Target::Projectile(id) = burning else {
            panic!("{burning:?}")
        };
        assert!(
            game.creatures.projectiles.iter().all(|p| p.id != id),
            "the burning bullet is destroyed"
        );
    });
}
