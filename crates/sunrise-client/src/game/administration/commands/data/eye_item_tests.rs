use super::{DataRef, Target};
use crate::game::native_entity_refs::tests::with_game;
use sunrise_core::nbt::Tag;

fn saved_item(game: &mut crate::game::Game<'_>, id: u32) -> Tag {
    let tag = game.data_of(&DataRef::Entity(Target::Other(id))).unwrap();
    tag.get("Item")
        .cloned()
        .expect("an eye of ender always saves Item")
}

#[test]
#[ignore = "daily"]
fn a_summoned_eye_saves_its_normalised_item_live_and_after_a_reload() {
    with_game(|game| {
        for (nbt, id) in [
            ("{}", "minecraft:ender_eye"),
            ("{Item:{id:\"minecraft:stone\",count:5}}", "minecraft:stone"),
            (
                "{Item:{id:\"minecraft:air\",count:1}}",
                "minecraft:ender_eye",
            ),
        ] {
            let expected = Tag::compound([("id", Tag::String(id.into())), ("count", Tag::Int(1))]);
            game.summon_with_nbt("minecraft:eye_of_ender", [5.5, 64.0, 5.5], nbt)
                .unwrap();
            let eye = game.others.last().unwrap().id;
            assert_eq!(saved_item(game, eye), expected, "live, {nbt}");

            let other = game.others.last().unwrap().clone();
            let uuid = game.uuid_for(eye);
            let saved = game.other_to_nbt(&other, uuid);
            game.others.retain(|o| o.id != eye);
            let reloaded = game
                .load_other_transferred(saved, "minecraft:eye_of_ender", None)
                .unwrap();
            assert_eq!(saved_item(game, reloaded), expected, "reloaded, {nbt}");
            game.others.retain(|o| o.id != reloaded);
        }
    });
}
