//! `LevelRef::play_sound` leaving one player out: the player handle is translated to the
//! connection inside the game.

use super::native_guest_turn_tests::with_guests;
use mod_api::SoundSource;

/// A sound queued with `except` names that player's connection; without it, no one.
#[test]
#[ignore = "daily"]
fn a_level_sound_leaves_out_the_player_asked() {
    with_guests(|game| {
        game.options.dedicated = Some(Vec::new());
        let player = game.player(13).unwrap();
        let level = game.level(game.dim.id()).unwrap();
        let at = [0.0, 64.0, 0.0];
        let sound = "minecraft:block.bell.use";
        let play = |game: &mut super::Game<'_>, except| {
            game.level_play_sound(level, sound, at, SoundSource::Blocks, 1.0, 1.0, except)
        };
        let queued = |game: &super::Game<'_>| game.creatures.lan_sounds.len();
        let before = queued(game);
        assert!(play(game, Some(player)));
        assert_eq!(game.creatures.lan_sounds.last().unwrap().except, Some(13));
        assert!(play(game, None));
        assert_eq!(game.creatures.lan_sounds.last().unwrap().except, None);
        assert_eq!(queued(game), before + 2);
        let gone = mod_api::__private::game::player_ref(
            mod_api::EntityRef::new(
                mod_api::WorldEpoch::new(player.entity().epoch().get() + 1),
                player.entity().id(),
            ),
            player.uuid(),
        );
        assert!(!play(game, Some(gone)), "a stale player queues nothing");
        assert_eq!(queued(game), before + 2);
    });
}
