use super::{DataRef, Target};
use crate::game::native_entity_refs::tests::with_game;
use sunrise_client::motion::PlayerMotion;
use sunrise_core::geometry::Vec3;
use sunrise_core::nbt::Tag;

fn cooldown(game: &mut crate::game::Game<'_>, target: Target) -> Option<Tag> {
    game.data_of(&DataRef::Entity(target))
        .unwrap()
        .get("PortalCooldown")
        .cloned()
}

/// #2387: `Entity.baseTick` -> `handlePortal` -> `processPortalCooldown`
/// counts a mob's cooldown down every tick, and `saveWithoutId` writes the
/// live value, in `/data get` and in the chunk save.
#[test]
#[ignore = "daily"]
fn a_mob_saves_its_live_portal_cooldown() {
    with_game(|game| {
        // A player's tickets make the chunks around the origin entity-ticking.
        game.player = Some(PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
        game.update_chunk_tickets();
        let pig = game
            .summon_with_nbt(
                "minecraft:pig",
                [0.5, 64.0, 0.5],
                "{NoAI:1b,NoGravity:1b,PortalCooldown:20}",
            )
            .unwrap()
            .unwrap();
        game.handle_entity_portals();
        assert_eq!(cooldown(game, pig), Some(Tag::Int(19)));

        // The chunk save writes 19; the reloaded pig counts on from there.
        let Target::Mob(id) = pig else {
            panic!("{pig:?}")
        };
        let i = game.mobs.iter().position(|m| m.id == id).unwrap();
        let saved = game.mob_tree_nbt(i);
        assert_eq!(saved.get("PortalCooldown"), Some(&Tag::Int(19)));
        game.mobs.retain(|m| m.id != id);
        game.forget_entity(id);
        let reloaded = Target::Mob(game.load_mob(saved, None).unwrap());
        game.handle_entity_portals();
        assert_eq!(cooldown(game, reloaded), Some(Tag::Int(18)));

        // `/data merge` is `Entity.load` on the live mob.
        let mut merged = game.data_of(&DataRef::Entity(reloaded)).unwrap();
        if let Tag::Compound(fields) = &mut merged {
            fields.insert("PortalCooldown".into(), Tag::Int(50));
        }
        game.apply_entity_nbt(reloaded, merged).unwrap();
        assert_eq!(cooldown(game, reloaded), Some(Tag::Int(50)));

        let plain = game
            .summon_with_nbt("minecraft:pig", [4.5, 64.0, 0.5], "{NoAI:1b,NoGravity:1b}")
            .unwrap()
            .unwrap();
        game.handle_entity_portals();
        assert_eq!(cooldown(game, plain), Some(Tag::Int(0)));
    });
}
