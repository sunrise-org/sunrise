//! `ServerExplosion.hurtEntities`: a blast takes over the redirectable
//! projectiles in its reach, giving them its causing entity as owner.
use super::*;
use crate::game::game_tests::tests::server_game;
use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1 as _;
use sunrise_sim::{
    motion::PlayerMotion,
    projectiles::{Projectile, ProjectileKind},
};

/// The host body, id 1 and UUID `[0, 0, 0, 7]`, six blocks above a blast at
/// `[8.5, 64, 8.5]`: near enough for no player, far enough for no hurt.
fn host_above_the_blast(game: &mut crate::game::ServerGame<'_>) {
    game.player = Some(PlayerMotion::new(game.motion, Vec3::new(8.5, 70.0, 8.5)));
    crate::game::native_entity_refs::tests::bind_local_player(game, 1, 7);
}

/// A fireball of `kind` in the blast at `[8.5, 64, 8.5]`, owned by `ghast`.
fn ghast_fireball(
    game: &mut crate::game::ServerGame<'_>,
    kind: ProjectileKind,
    ghast: u32,
    at: Vec3,
) -> usize {
    let mut fireball = Projectile::fireball(kind, at, Some(Who::Mob(ghast)), [0.0, 0.0, 1.0]);
    fireball.owner_uuid = Some(game.uuid_for(ghast));
    game.creatures.projectiles.push(fireball);
    game.creatures.projectiles.len() - 1
}

#[test]
#[ignore = "daily"]
fn a_blast_gives_its_causing_entity_to_the_redirectable_projectiles_in_reach() {
    let mut resources = None;
    let mut game = server_game(&mut resources);
    game.build_registries();
    host_above_the_blast(&mut game);
    let ghast = 91;
    // A ghast's large fireball is in `#minecraft:redirectable_projectile`; a
    // blaze's small one is not.
    let large = ghast_fireball(
        &mut game,
        ProjectileKind::LargeFireball { power: 1 },
        ghast,
        Vec3::new(9.5, 64.5, 8.5),
    );
    let small = ghast_fireball(
        &mut game,
        ProjectileKind::SmallFireball,
        ghast,
        Vec3::new(7.5, 64.5, 8.5),
    );
    let ghast_uuid = game.creatures.projectiles[large].owner_uuid;
    // A player-lit TNT names the player as the blast's indirect source.
    game.server.combat.0.blast_owner = Some((1, Text::literal("Host")));
    game.server.combat.0.blast_owner_player = true;
    game.server.combat.0.blast_guest = None;
    game.server_explode([8.5, 64.0, 8.5], 2.0, BlockInteraction::Keep, false)
        .unwrap();
    let taken = &game.creatures.projectiles[large];
    assert_eq!(
        taken.owner,
        Some(Who::Player),
        "the blast took the fireball in the tag"
    );
    assert_eq!(
        taken.owner_uuid,
        Some([0, 0, 0, 7]),
        "the host's UUID, not the ghast's"
    );
    assert_eq!(taken.guest, None, "the host is the player in place");
    let kept = &game.creatures.projectiles[small];
    assert_eq!(
        kept.owner,
        Some(Who::Mob(ghast)),
        "a projectile outside the tag keeps its owner"
    );
    assert_eq!(kept.owner_uuid, ghast_uuid);
    assert_eq!(kept.guest, None);
    // A blast whose source has no causing entity leaves the projectile unowned.
    game.server.combat.0.blast_owner = None;
    game.server.combat.0.blast_owner_player = false;
    game.server_explode([8.5, 64.0, 8.5], 2.0, BlockInteraction::Keep, false)
        .unwrap();
    let cleared = &game.creatures.projectiles[large];
    assert_eq!(cleared.owner, None, "no causing entity, no owner");
    assert_eq!(cleared.owner_uuid, None, "and no UUID left over");
    assert_eq!(cleared.guest, None);
}

#[test]
#[ignore = "daily"]
fn an_acting_guests_blast_gives_the_projectile_the_guest() {
    let mut resources = None;
    let mut game = server_game(&mut resources);
    game.build_registries();
    host_above_the_blast(&mut game);
    // The guest's turn lends them the player body: the blast's cause is that
    // body's entity id, and the guest's connection is the one acting.
    game.server.lan_acting = Some(9);
    game.server.lan_acting_name = Some("Guest".into());
    game.server.lan_acting_uuid = Some(9);
    game.server.lan_acting_entity = Some(1);
    let ghast = 91;
    let fireball = ghast_fireball(
        &mut game,
        ProjectileKind::LargeFireball { power: 1 },
        ghast,
        Vec3::new(9.5, 64.5, 8.5),
    );
    game.server.combat.0.blast_owner = Some((1, Text::literal("Guest")));
    game.server.combat.0.blast_owner_player = true;
    game.server.combat.0.blast_guest = Some(9);
    game.server_explode([8.5, 64.0, 8.5], 2.0, BlockInteraction::Keep, false)
        .unwrap();
    let taken = &game.creatures.projectiles[fireball];
    assert_eq!(taken.owner, Some(Who::Player));
    assert_eq!(taken.guest, Some(9), "credited to the acting guest");
    assert_eq!(
        taken.owner_uuid,
        Some([0, 0, 0, 9]),
        "the acting guest's UUID, not the host's"
    );
}
