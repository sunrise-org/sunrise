# Controlling spawns

Use spawn hooks to accept or reject a spawn, edit a mob before finalization, or control despawning. All four hooks run on the server with a read-only host and fire on replayed ticks. Your listeners must be pure functions of their inputs. `finalize_spawn` can also change the mob it is about to finalize, through the editor it is handed, and the pack's group data; it changes nothing else.

| Moment | Hook | Answer |
|---|---|---|
| The spawn predicate for a type and position | `mob_spawn_placement_check` | `SpawnCheck::{Default, Succeed, Fail}` |
| The mob's own position and obstruction checks | `mob_spawn_position_check` | `SpawnCheck::{Default, Succeed, Fail}` |
| Just before `finalizeSpawn` | `finalize_spawn` | `Flow::Cancel`, `spawn_cancelled`, a new `difficulty`, edits to `mob`, a new `group` |
| Each tick, before a mob's `checkDespawn` | `mob_despawn` | `Despawn::{Default, Allow, Deny}` |

## Saying where a mod mob spawns

`Registrar::declare_spawn_placement(path, rule)` is NeoForge's `RegisterSpawnPlacementsEvent`: during registration a mod gives its entity type a `SpawnPlacementRule`, and the game's own spawn checks answer from it for that type, wherever they run (natural spawning, chunk generation, spawners, patrols, sieges, reinforcements).

```rust,ignore
r.declare_spawn_placement("ghoul", mod_api::SpawnPlacementRule {
    placement: mod_api::SpawnPlacementType::OnGround,
    heightmap: mod_api::Heightmap::MotionBlockingNoLeaves,
    spawnable_on: Some("mymod:ghoul_ground".to_owned()), // a block tag the block below is in
    light: mod_api::SpawnLight::Dark,              // Any, Dark (a monster's) or Bright (an animal's)
    biome_tag: Some("mymod:haunted".to_owned()),         // a biome tag the spot's biome is in
}).unwrap();
```

The placement type is `SpawnPlacements.isSpawnPositionOk`'s first check, and the rule stands in for the type's spawn predicate: every field that is set must hold, and a trial spawner ignores `light`. A type with no declaration keeps its template's rules. The game resolves the declarations once at load, so a check reads one table and never walks the mods. A tag is a namespaced name written without `#`; an id outside the mod's namespace, a bad tag or a second declaration of the same type refuses the mods at startup, and a type no content file defines refuses the resource load.

`mob_spawn_placement_check` is the separate per-check moment: it runs after the rule and is told its answer as `default_result`, so you can override it for one position.

## Saved display and utility entities

`entity_join_level` (hook version 3) additionally notifies for live fallback-store
entities: block, item and text displays, markers, interactions, and summoned or
chunk-restored eyes of ender and explicit leash knots. Gameplay-thrown eyes and
naturally tied leash knots live in separate lists and do not notify yet. Saved data
kept for a missing mod is not a live entity and does not notify. A fresh fallback
entity notifies once with `loaded_from_disk = false`; restoring it from a saved chunk
notifies once with `loaded_from_disk = true`. The normal join cancellation and
transaction rules apply to these entities too.

`entity_leave_level` (hook version 3) reports the same additional kinds when they leave.
On the server, `level.entities_in(host, min, max)` additionally returns readable `EntityRef`
handles for block, item and text displays, markers and interactions. Eyes of ender,
leash knots and other unmodeled kinds are not returned yet. The returned entities'
kind, position, saved data and tags can be inspected without command selectors. The query uses physical boxes, not rendering/culling boxes:
displays and markers have zero-sized boxes at their positions; interactions use their
saved width and height. Face-only contact does not intersect. A handle expires when its
entity unloads, so find it again when the saved entity rejoins.

`level_entities_in(level, min, max)` lists item frames and paintings whose boxes meet the box too, with
handles that resolve, after the ordinary entities and before the players.

## Walking every entity in a level

`host.level_all_entities(level)` returns the authoritative level's available
`EntityRef`s without an AABB filter, including spectator players. Unlike
`level_entities_in`, it does not append the dragon's parts: vanilla's
`ServerLevel.getAllEntities` iterates the entity lookup, not multipart hit boxes.
A remote client, stale handle or parked level returns `None`, never a partial
client-side list. Outside a scoped hook query, an unavailable lookup member makes the query return `None`.
Inside a scoped hook query, unavailable members are omitted.

The walk preserves lookup admission order across storage families, including
removal and re-admission at the end. An entity spawned during play, such as a
fired arrow or primed TNT, joins the lookup as it spawns, as vanilla's
`addFreshEntity` adds it, even when the engine gives it its id later. A chunk's
entities join in the order the chunk was stored: vanilla's `storeChunkSections`
order, section by section from section 0 upward and then the negative sections,
whatever storage family holds each entity. Each section keeps its entities in
the order they entered it: an entity that crosses into another section goes to
the end of that section, as vanilla's `onMove` puts it there, while its place in
the lookup stays the same. Each crossing is recorded as the entity moves, in
the order the moves happen: an entity that steps out of a section and back is
at the end of it again, and a passenger enters after its vehicle. A vehicle
comes before its passengers.

Section entry order does not depend on a listener of `entity_entering_section`:
the engine records each server crossing whether or not a mod hears it.

Some differences remain. Vanilla notes a section change at every position write;
here minecarts, boats, projectiles, dropped items, primed TNT, falling blocks,
fireworks, experience orbs and the other server entities that report after their
own update count several moves within one tick as one crossing, from where the
entity was to where it ended (mobs and players are told per write). A block
piston's push is recorded only while a mod listens to `entity_entering_section`.
Entities that the engine keeps outside the residency families, such as lingering
potion clouds, lightning bolts and eyes of ender, are not lookup members at all.
The call allocates buffers for the result and sorts admission keys; it does not
allocate an object per entity. Cache the result when doing several passes in one
hook rather than repeatedly crossing the host boundary.

## Checking a spawn

`mob_spawn_placement_check` and `mob_spawn_position_check` both get the `level` the mob would join (a `LevelRef`, so a
listener can read the blocks around `pos` through the host) and the `spawner` that asked: `Spawner::Block { pos }` for a
spawner block, `Spawner::Trial { pos }` for a trial spawner, and `None` for natural spawning, chunk generation, patrols,
sieges, reinforcements and a spawner minecart. Set `*event.result` to `SpawnCheck::Succeed` or `Fail` to force the
check; `Default` keeps vanilla's answer.

```rust,ignore
fn mob_spawn_placement_check(&self, _: &dyn mod_api::Host, event: &mut mod_api::MobSpawnPlacementCheckEvent<'_>) {
    // spawner blocks work as usual; nothing else spawns zombies here
    if event.kind == "minecraft:zombie" && event.spawner.is_none() {
        *event.result = mod_api::SpawnCheck::Fail;
    }
}
```

The position check also gets `event.mob`, the mob about to spawn, as an `UnspawnedMobRef` with the same reads as
`finalize_spawn`'s (no editor: the hook only reads). Vanilla creates the mob before this check; Sunrise creates it there
only when the hook has a listener, the same way the spawn would, and the spawn then goes on with that very mob, so what
the listener read is what joins the level. A refused check drops the mob: it never joins the level. Creating it draws
nothing from the level's random, so a listener that answers `Default` changes no draw.

```rust,ignore
fn mob_spawn_position_check(&self, host: &dyn mod_api::Host, event: &mut mod_api::MobSpawnPositionCheckEvent<'_>) {
    if event.mob.custom_name(host).is_ok_and(|name| name.is_some()) {
        *event.result = mod_api::SpawnCheck::Succeed;
    }
}
```

Where the game does not create the mob fresh, the handle answers only `kind` and `position` (the rest is
`UnspawnedMobError::Unsupported`): chunk generation's packs, which the world generator creates and finalizes after the
check, and a spawner or trial spawner whose spawn data carries entity data, which the game loads after the tick.

## Stopping a spawn or leaving a mob plain

`finalize_spawn` has two cancels. Returning `Flow::Cancel` skips finalization: the mob spawns, but with no equipment, attributes or class setup. Setting `event.spawn_cancelled = true` adds no mob at all.

```rust,ignore
fn finalize_spawn(&self, _: &dyn mod_api::Host, event: &mut mod_api::FinalizeSpawnEvent<'_>) -> mod_api::Flow {
    if event.kind == "minecraft:zombie" && event.reason == mod_api::SpawnReason::Natural {
        event.spawn_cancelled = true;
    }
    mod_api::Flow::Continue
}
```

`finalize_spawn` fires only where vanilla calls `finalizeSpawn`, so a breeding, an infested block's silverfish or a thrown egg's chick does not tell it.

For `/summon minecraft:ender_dragon` without NBT, command placement starts at zero
rotation and consumes no random yaw draw. Skipping finalization also skips its
follow-range and handedness draws. Spawn eggs retain their random placement yaw;
an explicit NBT compound follows the load route without finalization.

## Changing the mob before it is finalized

`event.mob` is the mob itself, an `UnspawnedMobRef`: it is not in a level, so it is not an `EntityRef`, and only this
hook call can use it. Read it through the host (`kind`, `position`, `equipment`, `attribute_base`, `attribute_value`,
`tags`, `custom_name`) and change it through `event.edits`, which reaches only this mob: equipment (an API-owned `ItemStack`), an attribute's base or a permanent modifier, tags and the custom name. An edit applies at once, so the
reads and the next listener see it. Once the hook returns, the handle answers `UnspawnedMobError::Expired`, as does a
handle from another call passed to this call's editor.

```rust,ignore
fn finalize_spawn(&self, host: &dyn mod_api::Host, event: &mut mod_api::FinalizeSpawnEvent<'_>) -> mod_api::Flow {
    if event.mob.kind(host).is_ok_and(|kind| kind.path() == "zombie") {
        let health = mod_api::Identifier::parse("minecraft:max_health").unwrap();
        let _ = event.edits.set_attribute_base(event.mob, &health, 40.0);
        let _ = event.edits.add_tag(event.mob, "mymod:tough");
    }
    mod_api::Flow::Continue
}
```

Listeners run before finalization. Finalization's own rolls follow them: equipment it rolls for a
slot replaces what a listener put there, and the attribute bonuses it adds apply on top of a listener's base. Return
`Flow::Cancel` to keep exactly what the listener left, since no finalization runs. Attribute edits reach the
attributes a mob keeps itself (max health, movement speed, follow range, attack damage, knockback resistance,
spawn reinforcements, jump strength, burning time, flying speed); the others answer `Unsupported` until the mob has
joined, when a `LivingRef` writes them. The ender dragon's handle answers its kind and position, and `Unsupported`
for the rest. A mob that finalization creates (a chicken jockey's chicken, a zombie horse's rider) fires the hook
again with a handle of its own.

The hook fires on replayed ticks with the read-only host, so the editor and `group` are the only things a
listener changes, and it must change them the same way every time: a pure function of the event.

`event.group` is the pack's `SpawnGroupData` as plain data (`SpawnGroup`, variants named as vanilla saves them):
read it, change it, set it or clear it, and finalization reads what is left. Each hook call receives `None`; group data is not passed between natural pack members.
Chunk generation shares group data inside the generator, outside this hook. A group you set affects
only this mob's finalization.

See [the `finalize_spawn` contract](https://github.com/sunrise-org/sunrise/blob/main/crates/sunrise-mod-vocab/src/hooks/finalize_spawn.rs) for the spawn sites that fire the hook.

## Changing how large a spawn group grows

A natural spawn adds mobs of one type to a group until the group reaches the type's limit (`Mob.getMaxSpawnClusterSize`: 4 by default, more for horses, wolves and schooling fish, 1 for ghasts and pillagers). The entity part of the type changes that limit with `EntityPart::spawn_cluster_size`, which gets the read-only host, the mob just spawned, why and from what it spawned, and the limit vanilla would use. Return `Some(limit)` to replace it, or `None` to leave it to the next part and then to vanilla. The first part to answer wins, and the group stops growing once its size reaches the limit.

```rust,ignore
struct ZombiePacks;
#[mod_api::sunrise_part]
impl mod_api::EntityPart for ZombiePacks {
    fn spawn_cluster_size(
        &self,
        _: &dyn mod_api::Host,
        _: mod_api::UnspawnedMobRef,
        _: mod_api::SpawnReason,
        _: Option<mod_api::SpawnSource>,
        vanilla: u32,
    ) -> Option<u32> {
        Some(vanilla * 2)
    }
}
```

Only natural spawning's packs ask it: vanilla has no group limit anywhere else. A type with no part keeps vanilla's limit and costs nothing. The mob is the same `UnspawnedMobRef` as `finalize_spawn`'s, finalized and not in a level, and the call runs on replayed ticks like `finalize_spawn`, so an answer must be a pure function of its inputs. Attach the part with `r.entity_part("minecraft:zombie", ZombiePacks)` as [entity parts](../../concepts/parts/entity-parts-and-mob-goals.md) describe.

## Keeping or removing a mob

`mob_despawn` fires once per tick for every mob whose chunk is entity-ticking, just before vanilla's `Mob.checkDespawn`, persistent mobs and mobs in peaceful mode included. Player tickets make chunks entity-ticking across the whole view distance, so a mob beyond the simulation distance is still checked, though it isn't ticked. A mob in a loaded chunk that is not entity-ticking is neither checked nor told. `Allow` discards the mob, `Deny` keeps it, `Default` leaves vanilla's rules to decide. An answer other than `Default` skips all of vanilla's checks, so a `Deny` keeps a hostile mob in peaceful mode and neither answer takes the idle-despawn `nextInt(800)` draw.

```rust,ignore
fn mob_despawn(&self, _: &dyn mod_api::Host, event: &mut mod_api::MobDespawnEvent<'_>) {
    if event.kind == "minecraft:zombie" {
        *event.result = mod_api::Despawn::Deny;
    }
}
```

The hook's scoped read host resolves the ticking mob directly and peers through the tick snapshot. An already-ticked peer exposes its post-turn state; a not-yet-ticked peer exposes its pre-turn state. A mob spawned during the pass does not resolve through this scoped view until the next pass. Removed mobs do not resolve, but a dead mob still present in the level is not removed and can still resolve. `living_change_target` uses the same view when fired inside this tick pass; outside it, that hook reads the entities available in the game host, not this snapshot.

This is the accepted read-host scoped-view first step of #795, not the completed in-place ownership design. Write-host callbacks and the remaining ownership work are outside this contract.

The wither and the ender dragon override `checkDespawn` in vanilla, so the event is not fired for them.

## Adding or removing a custom spawner

The level's custom spawners (phantoms, patrols, cats, village sieges, the wandering trader) are a list a mod edits once, in `modify_custom_spawners`. A mod's own spawner is a key plus `custom_spawner_tick`, which runs every tick while mob spawning is on:

```rust,ignore
fn modify_custom_spawners(&self, _: &dyn mod_api::Host, event: &mut mod_api::ModifyCustomSpawnersEvent<'_>) {
    // no phantoms in this world
    event.spawners.remove(&mod_api::SpawnerEntry::Builtin(mod_api::SpawnerKind::Phantom));
    event.spawners.add(mod_api::SpawnerKey::new("mymod:rat_swarm").unwrap());
}

fn custom_spawner_tick(&self, host: &mut dyn mod_api::ServerHost, event: &mut mod_api::CustomSpawnerTickEvent<'_>) {
    if event.key.as_str() == "mymod:rat_swarm" && event.spawn_enemies {
        // spawn through `host`
    }
}
```

Every mod is asked about every key, so a mod ignores keys it does not own. See [the custom spawner contracts](https://github.com/sunrise-org/sunrise/blob/main/crates/sunrise-mod-vocab/src/hooks/custom_spawners.rs).

## Spawning from a level

`LevelRef::spawn_mob(host, kind, pos, reason)` is `EntityType.spawn` for a mob at rest, the same living route `spawn_entity` takes with `SpawnBody::Living`: the mob is made as any of its type is, Parts, attachments and `entity_constructing` included, then `finalize_spawn` and `finalizeSpawn` run with `reason`, and it joins the level (`entity_join_level`; inside a transaction, at its root commit), not persistent. It returns the mob's handle, or `None` with nothing spawned: a kind that isn't a mob, a position that isn't finite, outside `Level.isInSpawnableBounds` or in an unloaded chunk, a level not in place, a client, or a mod that cancelled the spawn or kept the mob out.

`LevelRef::spawn_entity(host, kind, pos, &spawn)` spawns one entity of `kind` at `pos` and returns its handle, or `None` with nothing spawned. `EntitySpawn::new(body)` starts the spawn at rest, unturned and with no owner; set its `velocity` (blocks per tick, `Entity.setDeltaMovement`, set before it joins), `rotation` (`[yaw, pitch]`) and `owner` (the entity that fired it) as needed. Its `SpawnBody` names the family, and the call goes the way that family's own spawns go:

- `SpawnBody::Thrown { item }`: a thrown item projectile (vanilla's snowball, egg, ender pearl, experience bottle, wind charge, splash or lingering potion, or a content mod's thrown type) through `Projectile.spawnProjectile`, the route a thrown snowball takes. Its owner is set before it joins (`Projectile.setOwner`: hit credit, saved as `Owner`, and the shot keeps clear of its owner until it has left the owner's box), `projectile_launch` fires, and it carries one of `item`. With no `rotation` it faces along its velocity, as `Projectile.shoot` leaves it. Its owner is a player, a mob, a boat or a minecart: the shot needs the owner's collision identity to keep clear of it, so any other owner refuses the spawn.
- `SpawnBody::Living { reason }`: a mob (vanilla's or a content living type) through `EntityType.spawn`, the route `/summon` takes: made as any of its type is, Parts, attachments and `entity_constructing` included, then `finalize_spawn` and `finalizeSpawn` run with `reason`, and it joins the level (`entity_join_level`), not persistent. With no `rotation` it takes `EntityType.spawn`'s yaw. Both calls refuse a type that is disallowed in Peaceful when the level is Peaceful, before the mob is built or any spawn hooks run. A mob takes no owner. `kind` resolves as `/summon`'s does (`cow` is `minecraft:cow`). The ender dragon takes its own creation, facing `rotation`'s yaw; it has no velocity or pitch here, so a dragon given either is refused.
- `SpawnBody::General`: a row of a general type a mod declared (`declare_general_entity`), with a fresh UUID and the type's declared payload version, through the insertion `/summon` and chunk loading share. It is admitted as one record (no `entities_load`: no save was read) and joins as a fresh spawn. Its owner is kept on the row and saved as `Owner`. Its state is its attachments: write them on the returned handle inside the same transaction, and `entity_join_level` and the row's first tracking see them.

A general row can move to another position in its level through `EntityRef::teleport_to`. A row teleported on its own dismounts; a teleported vehicle takes its riders with it. Removing the vehicle dismounts its riders and leaves them in the level. `EntityRef::set_rotation` turns a general row without moving or dismounting it; the new angles reach clients through ordinary tracking.

In `declare_general_entity`, pass `pickable` after `summonable`. Pass `false` for
vanilla's plain `Entity.isPickable` default. With `true`, the crosshair can hit
the declared box once the row is shown. The ordinary use and attack packets name
that row, and the existing `entity_interact` and `attack_entity` hooks receive
its id and declared kind. Without a listener's response, use on the row passes
and the row declines damage. This does not make the box solid or give it a
middle-click item. The next argument declares how the row reacts to pistons:
pass `None` for `PushReaction::PushPull`, or `Some(PushReaction::IgnoreEntity)`
to leave the row in place. The declared reaction is readable on either side.

`EntityRef::owner(host)` reads the owner back (`TraceableEntity.getOwner`) for a projectile or a general row (`ProjectileRef::owner` answers the same for a projectile): a general type's own tick uses it to skip its shooter and to name the attacker of the damage it deals. A spawn is refused for a kind of another family than its body, a position, velocity or rotation that isn't finite, a position outside `Level.isInSpawnableBounds` or in an unloaded chunk, an owner that is stale, in another level, or without a collision identity for a thrown spawn, a level not in place, a client, or a mod that cancelled the spawn or kept the entity out.
Inside a transaction the join waits for the root commit, so a spawn a mod then keeps out still returns a handle, which reads stale from the commit on. The level's spawn limits, flags and spawn rules don't apply, as they don't to `/summon`; nor does a general type's `summonable`, which is `/summon`'s alone. Items, experience and block drops have their own calls (`spawn_item`, `award_experience`, `pop_resource`), and any other entity `summon`.

A level's natural spawning (`NaturalSpawner.spawnForChunk`) also has a policy a mod can change:

- `spawn_limits(host)` and `set_spawn_limits(host, limits)`: a `SpawnLimits` with each spawning category's `getMaxInstancesPerChunk`, `SpawnLimits::VANILLA` (70 monsters, 10 creatures, 15 ambient, 5 axolotls, underground water and water creatures each, 20 water ambient) until a mod sets others. A limit caps the category twice, as vanilla's does: over the level, under `limit * spawnable chunks / 289`, and around each player near the chunk, under `limit`. A limit of 0 stops the category.
- `spawn_flags(host)` and `set_spawn_flags(host, flags)`: a `SpawnFlags` with `monsters` and `friendlies`, both on until a mod turns one off. Off keeps that side out of the level's natural spawning; neither turns monsters on against the `spawn_monsters` rule or anything on with `spawn_mobs` off.
- `can_spawn_entities_in_chunk(host, [x, z])`: `ServerLevel.canSpawnEntitiesInChunk`, the question natural spawning asks of a spawn position outside the chunk it started in. Sunrise's spawner answers it as "the chunk is loaded", and so does this call.

```rust,ignore
fn level_load(&self, host: &mut dyn mod_api::ServerHost, event: &mut mod_api::LevelLoadEvent<'_>) {
    let Some(level) = host.level(event.dimension) else { return };
    // twice the monsters, no bats
    level.set_spawn_limits(host, mod_api::SpawnLimits { monster: 140, ambient: 0, ..mod_api::SpawnLimits::VANILLA });
}
```

Only natural spawning reads the policy: spawners, trial spawners, patrols, sieges, phantoms, a new chunk's creatures and custom spawners keep their own rules. A change counts from the level's next spawn pass, and mobs already there stay. The policy is not saved. It survives parking and reopening a level within the same world session, but closing the world resets it. Set it in `level_load`. Reads work on any host and answer `None` on a client or for a stale handle; the writes need a `ServerHost` and return `false` there.
