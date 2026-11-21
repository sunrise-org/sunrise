//! The `player_start_tracking` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0147

pub trait Hooks {
    /// A client started tracking an entity (NeoForge's
    /// `PlayerEvent.StartTracking`, posted at the end of
    /// `ServerEntity.addPairing`): `player` (`name`) has just been sent the
    /// spawn bundle of `entity` (its ID, `uuid` its UUID, `kind` its type,
    /// `minecraft:zombie`) and counts as tracking it. It came into range, its
    /// client's chunks reached it, or it was put in the level; another player
    /// is an entity too, and a player never tracks their own. The entity is
    /// still in the level. What a listener sends the player
    /// (`PlayerRef::send_payload`) reaches it right after that bundle, before
    /// the entity's next changes. Heard in the tick's tracking pass, after the
    /// pass has built every player's packets: the player already counts as
    /// tracking the other entities the same pass paired. The engine is still
    /// walking its tracker: read the game and send payloads, but don't spawn,
    /// remove or teleport entities from here. No player's state is swapped in.
    /// Server-side.
    #[hook(
        side = "server",
        contract = "listen",
        version = 2,
        neoforge = "PlayerEvent.StartTracking",
        cancellable = false
    )]
    fn player_start_tracking();
}
