//! The `entity_tick_pre` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0154

pub trait Hooks {
    /// The entities `entities` are about to tick in the level `dimension`, and `kinds` holds the
    /// type of each (`minecraft:zombie`, `minecraft:item`), in the same order (NeoForge's
    /// `EntityTickEvent.Pre`, posted right before each entity's tick in `tickNonPassenger`). Told
    /// on the server for the mobs, players, item entities, minecarts, boats, general entity rows
    /// and projectiles that tick in an entity-ticking chunk, and the entities the server keeps
    /// without simulating them (displays, markers, interactions), whose tick is only told (not a
    /// player's own boat or cart, which ticks with the player); and on a client for the mobs, other
    /// players, projectiles, tridents and displays it ticks; `host` says which side. Minecarts,
    /// boats and general rows are each told alone. Eligible types named at registration with
    /// `entity_tick_each` (or requested through `entity_tick_each_entity`) are also told alone, to
    /// the mods that named them; the other mods hear them with their family, and when one of them
    /// cancels one there, it doesn't tick and the mods that named it hear nothing of it. Other
    /// observed entities are told once per family. On the server, player Pre runs before the
    /// observed parts of `ServerPlayer.tick`, not the connection body's `player_tick_pre`. Not told
    /// yet: on the server orbs, primed TNT, falling blocks, armor stands, paintings, item frames,
    /// leash knots, eyes of ender, cushions, tridents, fireworks, lightning, fishing hooks, ominous
    /// item spawners, end crystals, the dragon and its breath, evoker fangs, thrown potions and
    /// potion clouds; on a client, its other entities, the local player included. Push an ID from
    /// `entities` onto `canceled` to skip that entity's observed tick work and suppress its Post.
    /// IDs outside this call are ignored; repeated IDs count once. A mod using these hooks also
    /// opts in through `entity_tick_listened`. Version 3 expands the version 2 coverage without
    /// changing the event signature.
    #[hook(
        side = "both",
        version = 3,
        contract = "collect",
        out = "canceled",
        select = true,
        neoforge = "EntityTickEvent.Pre",
        cancellable = true
    )]
    fn entity_tick_pre();
}
