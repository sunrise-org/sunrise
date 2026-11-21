use super::{DataRef, Target};
use crate::game::native_entity_refs::tests::with_game;
use sunrise_core::nbt::Tag;

fn rotation(tag: &Tag) -> Option<&Tag> {
    tag.get("Rotation")
}

/// #2680: `Entity.load` reads `Rotation` and neither `AreaEffectCloud` nor
/// `FallingBlockEntity` turns, so both save the loaded rotation.
#[test]
#[ignore = "daily"]
fn a_cloud_and_a_falling_block_save_the_rotation_they_were_loaded_with() {
    with_game(|game| {
        let loaded = Tag::List(vec![
            Tag::Float(45.0_f32.to_bits()),
            Tag::Float(10.0_f32.to_bits()),
        ]);
        game.summon_with_nbt(
            "minecraft:area_effect_cloud",
            [5.5, 64.0, 5.5],
            "{Rotation:[45.0f,10.0f]}",
        )
        .unwrap();
        let id = game.breath_clouds.last().unwrap().id;
        let live = game.data_of(&DataRef::Entity(Target::Cloud(id))).unwrap();
        assert_eq!(rotation(&live), Some(&loaded), "cloud, /data get");
        let cloud = game.breath_clouds.last().unwrap();
        let saved =
            crate::game::potions::potion_cloud_to_nbt(cloud, [1, 2, 3, 4], game.persist_raw(id));
        assert_eq!(rotation(&saved), Some(&loaded), "cloud, chunk save");
        // A cloud no compound loaded (a potion's) faces 0, 0.
        let saved = crate::game::potions::potion_cloud_to_nbt(cloud, [1, 2, 3, 4], None);
        assert_eq!(
            rotation(&saved),
            Some(&Tag::List(vec![Tag::Float(0), Tag::Float(0)]))
        );

        let falling = game
            .summon_with_nbt(
                "minecraft:falling_block",
                [9.5, 64.0, 5.5],
                "{BlockState:{Name:\"minecraft:sand\"},Time:1,NoGravity:1b,Rotation:[45.0f,10.0f]}",
            )
            .unwrap()
            .unwrap();
        let live = game.data_of(&DataRef::Entity(falling)).unwrap();
        assert_eq!(rotation(&live), Some(&loaded), "falling block");
    });
}
