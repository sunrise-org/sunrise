use super::{DataRef, Target};
use crate::game::native_entity_refs::tests::with_game;
use sunrise_client::motion::PlayerMotion;
use sunrise_core::geometry::Vec3;
use sunrise_core::nbt::Tag;

fn read(game: &mut crate::game::Game<'_>, stand: Target) -> [Option<Tag>; 5] {
    let tag = game.data_of(&DataRef::Entity(stand)).unwrap();
    [
        "HurtTime",
        "Air",
        "FallFlying",
        "current_impulse_context_reset_grace_time",
        "PortalCooldown",
    ]
    .map(|key| tag.get(key).cloned())
}

fn tick(game: &mut crate::game::Game<'_>) {
    game.handle_entity_portals();
    game.tick_armor_stands().unwrap();
}

/// #2678: an armor stand is a `LivingEntity`: its hurt time, portal cooldown
/// and impulse grace time count down, its air refills by 4 up to 300 out of
/// water, `updateFallFlying` clears the flag, and the save writes them live.
#[test]
#[ignore = "daily"]
fn an_armor_stand_ticks_its_living_counters() {
    with_game(|game| {
        game.player = Some(PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
        game.update_chunk_tickets();
        let stand = game
            .summon_with_nbt(
                "minecraft:armor_stand",
                [0.5, 64.0, 0.5],
                "{NoGravity:1b,HurtTime:40s,Air:100s,FallFlying:1b,\
                 current_impulse_context_reset_grace_time:20,PortalCooldown:20}",
            )
            .unwrap()
            .unwrap();
        let full = game
            .summon_with_nbt(
                "minecraft:armor_stand",
                [4.5, 64.0, 0.5],
                "{NoGravity:1b,Air:32767s}",
            )
            .unwrap()
            .unwrap();
        tick(game);
        assert_eq!(
            read(game, stand),
            [
                Some(Tag::Short(39)),
                Some(Tag::Short(104)),
                Some(Tag::Byte(0)),
                Some(Tag::Int(19)),
                Some(Tag::Int(19)),
            ]
        );
        for _ in 0..39 {
            tick(game);
        }
        assert_eq!(
            read(game, stand),
            [
                Some(Tag::Short(0)),
                Some(Tag::Short(260)),
                Some(Tag::Byte(0)),
                Some(Tag::Int(0)),
                Some(Tag::Int(0)),
            ]
        );
        assert_eq!(
            read(game, full)[1],
            Some(Tag::Short(32767)),
            "above 300 it stays"
        );
    });
}
