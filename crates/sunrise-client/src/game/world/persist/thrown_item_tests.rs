use super::*;
use crate::game::native_entity_refs::tests::with_game;

/// #2691: `ThrowableItemProjectile.addAdditionalSaveData` writes the `Item`
/// it flies as (count 1), also once it has moved from where it was loaded.
#[test]
#[ignore = "daily"]
fn a_moved_thrown_item_projectile_saves_its_loaded_item() {
    with_game(|game| {
        let expected = Tag::compound([
            ("id", Tag::String("minecraft:stone".into())),
            ("count", Tag::Int(1)),
        ]);
        for kind in ["egg", "snowball", "ender_pearl", "experience_bottle"] {
            game.summon_with_nbt(
                &format!("minecraft:{kind}"),
                [5.5, 64.0, 5.5],
                "{Item:{id:\"minecraft:stone\",count:5},Motion:[0.05d,0.0d,0.0d],NoGravity:1b}",
            )
            .unwrap();
            let i = game.creatures.projectiles.len() - 1;
            // One tick's move away from the summon position.
            game.creatures
                .projectiles
                .update(i, |p| p.position.x += 0.05);
            let at = game.creatures.projectiles[i].position;
            let keys: HashSet<ChunkKey> = [chunk_of(at)].into();
            let (_, saved) = game.save_projectile(i, &keys).unwrap();
            assert_eq!(saved.get("Item"), Some(&expected), "{kind}");
            game.creatures.projectiles.remove(i);
        }
    });
}
