# Hook notes

`native_block_clone_item` supplies a block's clone stack from its actual state before ordinary Ctrl+Pick block-entity component copying. First response owns the result; no response retains vanilla. An empty or unregistered response suppresses picking.

## Block extra models

`ClientHost::set_block_extra_models(pos, models)` replaces the standalone models (`RenderAssets::model`) drawn as part of the block at `pos`; an empty list clears it. The lists are client-only and need no block entity. A resend of a resident column preserves attachments at unchanged cells; unloading the column clears them. A block state or level change also clears the affected lists.

The live client Host can set the list; reload and setup contexts do not expose the setter. `NotLoaded` means the position is outside a resident column or the level height; `Model` means the handle is stale or is not a registered standalone model. A refused call changes nothing. Resolve new handles in `client_resources_reloaded`, then set the lists again from a live client callback (`client_tick_pre`).

After the block geometry, extra faces are drawn in block and list order, with the block's light, neighbour culling and tints but their own render layers. Mod tint providers are asked before the mesh is scheduled; a provider that panics supplies white. Drawing stops before the next face whose worst-case expansion cannot fit the section's upload budget, so the last model may be partially drawn.

`ServerHost::create_boss_bar(id, name)` creates a custom boss event and returns `Result<Option<BossBarRef>, TextFormError>` (`None` when the id is taken or invalid, `Err` for text deeper than 512 nodes); `ServerHost::boss_bar(id)` finds an existing one. The handle sets its name, value, maximum, colour, overlay, flags and visibility, adds or removes `PlayerRef` viewers with `add_player`/`remove_player`, and deletes the event with `remove`. These are saved custom boss events, not a per-player bar snapshot.

## Camera entity

`ClientHost::camera_entity()` returns an `EntityRef` for the entity the view follows
(`Minecraft.getCameraEntity`): the local player, unless a server attached the camera to another
entity. It is `None` on the title screen and outside a world. Together with `camera()` and
`camera_type()` it lets a minimap or freecam mod tell its own view from a spectated one.

`ClientHost::set_camera_entity(entity)` is `Minecraft.setCameraEntity`: it moves the view onto a
mob (or back onto the player with the player's own ref) and returns whether it did. It selects
the entity only; the camera angle stays the entity's, and `compute_camera_angles` and
`camera_position` are the hooks that override angles and render-only camera position.

## Client heights

`LevelRef::visit_surface_heights(host, columns, heightmap, visit)` is
`LevelReader.getHeight(type, x, z)` on any open level (server active or parked, or the client's
current open level): each column's height from loaded chunks, `None` for a column whose chunk is
not held, streamed with no column-count cap and early exit via `ControlFlow::Break(())`. A minimap
or a tracker reads many columns in one call instead of scanning blocks; on a client it reads the
client's own view and follows server block changes as they arrive.

## Loaded chunks

`level.visit_loaded_chunks(host, &mut visit)` visits the chunks the level holds now, in x-then-z
order. It loads no chunk and does not copy the loaded set. Return `ControlFlow::Break(())` to
stop or `Continue(())` to visit the next chunk.

`ClientHost::create_fireworks(&FireworksRequest) -> bool` draws a firework's explosions at a point in this client's world through the same starter a bursting rocket uses: each `FireworkExplosion`'s shape, colours, fade colours, trail and twinkle, and the rocket's velocity carried by the sparks. `audible` gates the blast and twinkle sounds, as a rocket's silence does; the sparks and flash draw either way. No explosions puffs a little smoke, as vanilla's. The method returns false on servers, without a client player/level, or for non-finite input.

## Client window and screen state

`ClientHost::window_focused()` reports the last focus state from the window
system. `ClientHost::window_hovered()` is false until the pointer first enters
the window or the player moves it over the window; after that it follows the
window system's enter and leave. `ClientHost::window_occluded()` is the window
system's occlusion report, where the platform gives one.
`ClientHost::observed_screen_id()` returns the same ID screen hooks see,
including overlays. `None` means no view is available: there may be no open
screen or no renderer yet. It does not mean the client is outside a world.
Read again after a screen or focus change.

## Input preferences

`ClientHost::input_options()` returns a copy of the current mouse and movement
input preferences, even outside a world. Read again after settings change; an
older snapshot stays unchanged. The fields are documented in the API reference.

## Option snapshots

`ClientHost::graphics_options()`, `gui_options()`, `chat_options()` and `audio_options()` each
return a copy of one group of the player's preferences, even outside a world, like
`input_options()`. Read again after the settings change; an older snapshot stays unchanged.
A zoom, brightness, HUD or chat mod reads these instead of keeping its own copy of the player's
choices. `ClientHost::shader_pack_active()` reports whether a shader pack is currently loaded and
rendering the world (always `false` on the title screen, matching `IrisApi.isShaderPackInUse()`).
The fields are documented in the API reference.

`Host::add_particle(&ParticleOptions, position, velocity) -> bool` submits one particle to this client's loaded world. It preserves all option and velocity slots, including encoded color/size arguments, and uses the existing built-in and named-sprite providers with normal camera-distance, Particles-setting and population limits. True means submitted, not necessarily visible; missing providers/sprite sets draw nothing. It returns false on servers, without a client player/level, or for non-finite numeric input. A mod's own particle type gets code through `register_particle_providers`; new trigger hooks remain separate.

## Client session and local-player status

`ClientHost::client_session()` supplies read-only `client_server()`,
`client_language()` and `client_frame_stats()`. All return `None` outside a
logged-in client world. These reads use existing local state and send nothing.
`ClientServer<'_>` lends text from the host and keeps missing names and addresses
explicit: its `name` is the integrated world's current display name, or `None`
for an external connection (the game retains the player name, not a server
display name). Its `address` is absent for memory connections; friend-invite
secrets are redacted. `integrated_local` means this window actually owns that
integrated server, not that the address looks like localhost. `save_folder`
gives the owning window the integrated save's folder name as a key for local
data. It survives display-name changes and is scoped to the game instance. It
is absent for any other connection. All three text fields are `Option<&str>`;
use `.map(str::to_owned)` on a field only to keep it beyond the host borrow.

`client_language()` shares an `Arc<str>` snapshot of the applied language code,
not a pending options choice. Use `.as_deref()` to compare it with `Some("en_us")`.
A later language change leaves the snapshot intact.
Use the existing `ClientHost::is_paused()` for the applied pause state.
`ClientFrameStats` reports the game's most recently computed `fps`, lifetime
`rendered_frames` (menus included), and the newest completed frame's wall
interval as `frame_time_ns` (including waits). The interval is `None` until a
sample exists or after the chart is reset. Reading it starts no new recording.

`PlayerRef::block_breaking(&dyn ClientHost)` and `ambient_mood(&dyn ClientHost)`
only accept the current local player's handle. Stale, fake and nonlocal handles
return `None`, as does a disconnected client. `BlockBreakingState::Idle` is
instead a valid local player who is not breaking a block. `Breaking` gives the
target position, accumulated predicted progress and optional visible crack
stage (0 through 9; absent at zero progress). The stage uses the same
calculation as the world overlay. Mood is the existing cave ambience value,
normally 0 through 1, not a server-side measurement.

## Sending a map image

`PlayerRef::send_map` sends a complete 128-by-128 map image through a
`&mut dyn ServerHost`. It validates the live player handle and refuses fake
players; a connectionless actor never produces a protocol packet. The image
uses map colour bytes and is not saved in the world's map storage. Send it
again after a player reconnects if no saved map owns that id.

## Equipment slot

`ItemResource::equippable_slot(host)` answers which slot a stack goes in when worn
(`Equippable.slot`), from the item's `equippable` component after the stack's patch, so a
command's or a data pack's override counts. It does not say whether a given entity may wear it;
that depends on the wearer and is not answered here.

## Enchantment conflicts

`Host::enchantments_compatible(a, b)` answers `Enchantment.areCompatible` from the world's
enchantment definitions, so a data pack's `exclusive_set` counts. A mod that adds an enchantment
to a stack asks it for each enchantment already on it, as the enchanting table and the anvil do.

## Effective stack properties

`ItemResource` reads the stack's component overrides and removals before item defaults:
`damageable(host)`, `foil(host)`, `rarity(host)` and `enchantable(host)`.
`repairable(host, material)` takes another resource; only its item identity and tags matter,
not its count or component patch. `bar(host)` returns an optional `ItemBar`, with width out of
13 pixels and RGB colour, before client drawing hooks; bundles use their fullness bar.
`use_duration(host)` returns ticks, including the gameplay use-duration Part. Vanilla's
current item classes do not require a player argument; Parts see the host's current acting
player. These reads use a world-phase host; tag-based repair checks need its authoritative world tags.
Duration and bar reads need an engine backend and return `None` for an unknown item.

## Tooltip lines and random enchanting

`ItemResource::tooltip(client, advanced, creative)` returns styled text and line colours
for overlays that need the ordinary inventory tooltip. It uses the client's current viewer
and the requested flags. Queries include mod edits, except during item tooltip line construction,
when they return base lines. A query never gathers images or lays out or draws a tooltip.

`ItemResource::enchant_randomly(server, level, power, pool)` rolls from the specified
enchantment tag with the active server level's random source and the stack's effective
enchantability. It returns a detached result. Use the normal item-place or capability
transaction to replace a stored stack. Discarding the result does not rewind the random source.

## Stack combat queries

`ItemResource::entity_attack_range(host, attacker)` returns effective minimum, maximum and
hitbox margin; `contains(distance, buffer)` uses vanilla's inclusive float bounds.
`damage_source(host, attacker)` returns a typed owned source: component override, item-specific
source (mace smash), then the actor's ordinary melee source. Actors and targets are typed handles;
unavailable or stale handles return `None`. These reads never equip the queried stack.
`can_be_hurt_by(host, source)` asks whether the item itself can take that source's damage,
including damage-resistant component overrides, removals and damage type tags. It does not
calculate the wearer's protection.
`attack_damage_bonus(host, attacker, target, base_damage)` previews enchantment and item bonuses
with a cloned RNG snapshot. A read never consumes world RNG; a later attack may roll differently.
`hurt_and_break` remains the durability write.

## Bone meal at a position

`LevelRef::bone_meal(host, pos)` runs the whole bone meal sequence on one block for a mod that
grows things itself (a sprinkler, a growth totem), so it neither reimplements the checks nor
skips `bonemeal` listeners. It takes no stack: nothing is consumed, and a hook that edits the
stack it is shown edits a throwaway one.

## Setting a biome

`LevelRef::set_biome(host, pos, biome)` changes one biome cell, the 4x4x4 block volume a biome is
stored in, so every position in that cell changes with it. It shares `/fillbiome`'s save, mesh and
resend path, so the change survives a reload and reaches players as the command's does. Name a
biome the world knows (a data pack's included); anything else is refused.
Tentative reads see the new cells inside a transaction. Abort restores the cells without
publishing. Saving, remeshing and resending wait for the outermost commit, once per column;
outside a transaction they run immediately.

## World and spawnable bounds

`LevelRef::in_world_bounds(host, pos)` and `in_spawnable_bounds(host, pos)` answer
`Level.isInWorldBounds` and `Level.isInSpawnableBounds` without loading a chunk. The first also
needs the position inside the level's build height; the second is the wider limit `/summon` and
`/tp` use (30,000,000 on x and z, 20,000,000 on y, whatever the level's height). A mod checks them
before reading or writing a far position, rather than learning it from a failed write.

## Biomes, sky and floor height

`LevelRef::biome_at(host, pos)` reads an active or parked server level's biome;
`client_biome(host, pos)` reads the client's current copy. Both use the level's biome zoom,
including its hashed seed, and return `None` when the selected noise column is not loaded.
They do not generate or load chunks. A biome read may select a neighbouring column.

`find_closest_biome(host, biome, BiomeSearch::new(origin, radius, horizontal_step, vertical_step))`
uses the server generator's sampled search, as `/locate biome` does. `biome` is an id or `#tag`.
The result names the biome and its sample position. This searches generated biomes, not later
edits to resident biome cells, and does not load resident chunks. A negative radius or
nonpositive sampling step returns `None`.

`get_sky_darken(host)` shares gameplay's dimension, time and weather calculation.
`logical_height(host)` is the dimension's logical height, which can differ from build height.
`block_floor_height(host, pos)` uses normal collision shapes with an empty entity context;
the height is relative to the block, and negative infinity means no floor. An unloaded
column or an out-of-build-height position returns `None`. These three reads also work on
parked server levels and the current client level, without loading chunks or swapping dimensions.

## Respawn data, maps, ticks, sky and sight

`LevelRef::respawn_data(host)` is the world spawn, the level it is in and the way a player
appearing there faces (`LevelData.getRespawnData`), and `set_respawn_data(host, pos, yaw, pitch)`
is `/setworldspawn`'s own write in the handle's level: the yaw wraps and the pitch clamps as
`RespawnData.of` does, a player whose respawn point was the old world spawn follows it, and
`spawn_change` fires. The world has one world spawn, so any level's handle reads the same one,
and writing through a level moves it there.

`level.visit_map_data(host, id, &mut visit)` lends a map's saved data (`MapItemSavedData`):
centre, scale, dimension, locked, tracking and unlimited flags, and its 128 by 128 colour IDs.
It reads from memory or opens the saved data for the visit. Copy the dimension or pixels only
to retain them. A found map calls the visitor once and returns `Ok(Some(()))`; a missing map
returns `Ok(None)`. A stale level or unavailable world access gives `LevelReadError`. Maps are
world-wide: any current server level handle reaches them. `set_map_data(host, id, colors)`
repaints it pixel by pixel as `setColor` does, so the map saves, redraws and reaches players
carrying or framing it. Markers are not part of either call.

`block_tick(host, pos)` runs the block's scheduled tick now through the path a due tick from the
queue takes (the block's part first, then `BlockState.tick`), and `block_random_tick(host, pos)`
random-ticks the position as `tickChunk` does, lava's fluid tick included. Both draw from the
level random and send their level events exactly as the normal ticks do; neither reads or changes
the tick queue, which `schedule_block_tick` fills.

`can_see_sky(host, pos)` is `canSeeSky`: full sky light. `line_of_sight(host, from, to)` delegates
to `clip` with `ClipBlock::Collider` and `ClipFluid::None` (with no 128-block entity distance
check) and reports whether the segment misses.
`lightning_target(host, pos)` is `findLightningTargetAround`: a rod, else a living entity under
open sky near the column picked with the level random, else the column's top. It strikes nothing.
Environment attributes are not here: Sunrise has no general server-side attribute table to read.

## Drops, fluid shapes, collisions and entity rays

`LevelRef::block_drops(host, pos, tool, entity)` is `Block.getDrops`: what the block at `pos`
would drop if `entity` broke it with `tool`, with what its block entity copies in (a shulker
box's contents, a banner's patterns, a pot's sherds). It breaks nothing and spawns nothing, and
fires no `block_drops` event; `destroy_block` with `collect` does all of that. The roll draws
from the table's random sequence or the level random, as vanilla's does, so it takes a
`ServerHost` and moves later draws.

`LevelRef::roll_loot(host, table, origin, tool, entity)` is `LootTable.getRandomItems`: one roll
of any loot table, the world's data packs merged, handed back instead of dropped, for a sieve or a
machine that turns an input into a table's roll. The context is the one `/loot … fish <table>
<pos> <tool>` gives: `origin`, `tool` for `match_tool` and enchantment bonuses, and `entity` as
`this`. The stacks come split to their stack sizes. It draws as `block_drops` does and fires no
`loot_drops` event. None for a table that isn't there.

`fluid_shape(host, pos)` is `FluidState.getShape`: no box for no fluid, else one block-local box
up to the fluid's height, a full block under the same fluid. `has_collisions(host, aabb)` asks
whether any block collision shape is in a box, with `visit_block_collisions`'s walk and
without building the list; entities and the world border are not part of it.

Use `ray_trace_nearest_entity(host, from, to, margin, exclude, &mut accept)` if you
only need one hit. Matching `ProjectileUtil.getEntityHitResult`, it passes each
non-excluded candidate `EntityRef` in `visit_entities_in` order to `accept` before
reading its bounding box and clipping against the segment. The predicate can make shared
reads through the same host; no residency guard is held. Of the accepted candidates that
the segment clips, the one with the smallest squared distance wins; an exact tie keeps
the first visited hit. No accepted hit is `Ok(None)`. Non-finite geometry or unavailable
world access returns `Err(LevelReadError::Unavailable)`; a stale level gives
`Err(LevelReadError::Stale)`. A refused query never runs the predicate.

Keep `ray_trace_entities(host, from, to, margin, exclude)` if you need the full hit list
or must process hits nearest first. It preserves spatial traversal order for equal distances.

Both queries use the test `ProjectileUtil.getEntityHitResult` makes on each candidate:
clip against its box inflated by `margin` (`AABB.inflate`: positive grows, negative
shrinks; any finite value is accepted) and report the entry point and distance. A ray
starting inside the adjusted box does not hit it. `exclude` skips an entity such as the
shooter; `visit_entities_in` supplies the candidates, so spectator guests are not hit.
The filter (`canHitEntity`, `isPickable`) is up to the caller.

## Fluid properties and the explicit fluid tick

The fluid queries answer for a mod's world fluid as they do for water and lava, through the same
rules the flow reads, so a custom fluid behaves like one of them without a table of its own.

Position-dependent answers are `LevelRef` reads. `fluid_tick_delay(host, fluid, pos)` is
`Fluid.getTickDelay`: water 5, lava 30 and 10 in the Nether, the empty fluid 0, a flowing form as its
source; a mod's fluid answers what it declared as its `fluid_rules` parts change it at `pos`. It reads
the level in place on the server and is `None` for a position that isn't loaded. `fluid_height(host,
pos)` is `FluidState.getHeight` (a full block under the same fluid, else `amount / 9`),
`fluid_height_for_camera` is the surface the camera's immersion test reads (a source under a block
with a sturdy underside is 1), and `fluid_can_be_replaced_with(host, pos, fluid, direction)` is
`FluidState.canBeReplacedWith`: water yields to a non-water fluid flowing down, lava to water from 4/9
of a block high, the empty fluid to anything and a mod's fluid to nothing.

World-independent facts are in the fluid service (`host.fluid_states()`): `type_properties(key)` now
also carries `explosion_resistance` (100 for a fluid, 0 for the empty one), `randomly_ticks` (lava
only) and `drip_particle` (`minecraft:dripping_water`, `minecraft:dripping_lava`, else none), and
`legacy_block(state)` is `FluidState.createLegacyBlock`, the water, lava or mod fluid block that holds
a fluid state, air for the empty fluid.

`fluid_tick(host, pos)` takes a `ServerHost` and runs the flow step of the fluid at `pos` now, the
body of a due fluid tick: it levels, spreads and schedules its next step with the same random draws,
hooks and level events. Like `block_tick` it neither reads nor changes the tick queue; `schedule_fluid_tick`
queues one.

## Players, structures and chunk watchers

`LevelRef::visit_online_players(host, &mut visit)` visits `ServerLevel.players` without collecting
them. This works on active and parked server levels. In the active level, the local or acting
player comes first, followed by placed guests in server order. A parked level has only its
placed guests. Each resolvable player is visited once; fake players and players displaced by a
nested guest turn are skipped. Return `ControlFlow::Break(())` to stop, or `Continue(())` for the
next player. The walk returns `Ok(Break(()))` on an early stop and `Ok(Continue(()))` on completion,
including an empty walk. A stale level returns `LevelReadError::Stale`; a client or unavailable
world access returns `LevelReadError::Unavailable`. Neither calls the visitor. You can
make shared reads on the same host inside the visitor. If you need to write or send a packet,
collect the players you need and use the mutable host after the visit returns.

`visit_entities_in` is the spatial query: it visits placed non-spectator guests on a server,
or the tracked players and the window's own non-spectator player on a client.

`find_nearest_structure(host, structure, StructureSearch::new(origin, radius))` is
`ServerLevel.findNearestMapStructure` without skipping known starts, the search `/locate
structure` runs in the level's generator. `structure` is an id or `#tag`; the result names the
structure and the position `/locate` reports. `radius` counts rings of placement cells for
random-spread sets (nonnegative; `/locate` uses 100); strongholds' rings are all checked whatever
it is. The search builds structure starts but neither loads nor generates chunks; a world without
structures, a structure this level cannot generate, an origin past the world's 30,000,000-block
edge (`Level.isInWorldBoundsHorizontal`) and a negative radius give `None`.

`structures_with_piece_at(host, pos, structure)` and `structures_at(host, pos, structure)` say
which of the structures `structure` names lie at `pos`: inside one of a start's pieces
(`StructureManager.getStructureWithPieceAt`) or inside its whole bounding box (`getStructureAt`).
They read the loaded chunk's structure references and never load or generate a chunk.

`chunk.visit_watchers(host, &mut visit)` visits `ChunkMap.getPlayers(pos, false)`: the players
the chunk was sent to and is inside the view of, from their `chunk_watch` until they move away.
Fake players watch nothing. Return `ControlFlow::Break(())` to stop. An empty walk completes
without a callback; a stale level or unavailable side or phase returns `LevelReadError`.

`native_muffle_sounds(amount, lowpass, ticks, exempt)` (`host.muffle_sounds`) turns every sound down and low-passes it, fading back over `ticks` client ticks, except the sound events in `exempt`, which may list any number of namespaced sound IDs.

## Named hook catalog (draft)

The declarations map stable hook names and contract versions to source targets. A contract includes its signature,
timing and behavior: retain it or version incompatible changes.
Each listener's edited stack-list outputs are validated before leaving its scope. A refused output keeps the immediately
preceding accepted value; scalar outputs and cancellation still apply, and later listeners remain independent.
One warning per refused output names its owning mod, hook, output and conversion error.

### Neighbour observations (draft)

`neighbor_changed` observes each authoritative server `neighborChanged` before the block handles it. It supplies the dimension, position, current owned `BlockState`, changed `BlockKey` and moved flag. The changed block key comes from the source block's default state through the prepared state index, after the primary state's routing guard. No registry-name lookup or allocation occurs on this path. Use explicit immutable block metadata for property queries; the hook does not load chunks. Its host is the level writer vanilla's `neighborChanged` gets (`&mut dyn LevelWriteHost`): a `BlockPart::neighbor_changed` can read its inputs there with `level.signal(pos, direction)`, `direct_signal`, `direct_signal_to`, `best_neighbor_signal` and `has_neighbor_signal`, the same reads as `LevelRef`'s (mods' parts included), schedule its own tick with `level.schedule_block_tick`, as a diode does (`DiodeBlock.checkTickOnNeighbor`), and set blocks with `level.set_block`, as `RedstoneLampBlock.neighborChanged` lights itself at once (`level.setBlock(pos, state.cycle(LIT), 2)`) and only schedules its tick to turn off. A write with neighbour-update flags queues those updates to run next, depth first, before the rest of the update that delivered the call, as `Level.setBlock` from inside `neighborChanged` does through `CollectingNeighborUpdater`; the block's own handling then runs with the state it was called with. Contract version `3`: version `2` handed the hook a `&mut dyn LevelTicksHost`, which could not set blocks, and version `1` a plain `&dyn LevelHost`. Diverges from upstream.

### Server commands and players

For mods that run where the world is simulated: a dedicated server, a LAN host, single player's server thread
(`crates/sunrise-client/src/game/modding/native_server.rs`).

- **`register_commands(commands)`**: push a `CommandRegistration` per usage with
  `Command::new(usage).executes(executor)` or `Command::op(usage, level).executes(executor)`.
  The server asks when a world opens, reloads data packs, or refreshes its commands.
  A usage is words, then arguments `<name:kind>`, optional ones last in brackets:
  `pay <player:player> <amount:integer>` or `balance [player:player]`.
  Kinds: `word`, `text` (the rest of the line, last), `integer`,
  `number`, `boolean`, `player`, `item`, `block_pos`, `vec3`, `id`, and any command argument type a mod registered
  (`Registrar::register_command_argument`): its namespaced id is the kind, as in
  `<machine:mymod:machine>`. A custom argument reads one word, a vanilla identifier (`Id`), or the rest of the
  line (`Greedy`) and delivers its type's canonical value to the executor. Usages merge into one tree that
  each player receives alongside vanilla's, filtered by permission. Even a client that does not know the
  custom type can parse and complete it: the tree sends the matching vanilla parser and asks the server
  for suggestions. The server parses player and console invocations against that tree, including each
  usage's permission and argument types, before calling the executor as the registering mod.
  Each executable usage keeps its own permission: an operator-only `foo` remains unavailable to ordinary players
  even if `foo bar` is public. Players can traverse the shared `foo` prefix to reach `bar`, but cannot execute
  `foo` itself. Optional arguments make their preceding prefixes executable at that usage's level; if several
  accepted usages execute at the same node, each later accepted registration overwrites that node's executor
  (matching Brigadier's `CommandNode.addChild`) together with Sunrise's paired execution permission level
  for that node. A rejected usage does not change the permissions of accepted usages.
  Clients receive the accepted command tree, never its executor bindings. When argument alternatives consume the same input without errors, an
  executable alternative takes precedence over a denied one; equally valid alternatives keep registration order.
  A name vanilla has, or a usage that doesn't parse, is left out with a warning in the log.

- **Command executor**: the closure receives `(&mut dyn ServerHost, &CommandSource, &CommandInvocation)`.
  The invocation has its literal path, its `dimension`, `position` and `rotation` (`[yaw, pitch]` in degrees) after `execute in/at/rotated`,
  the `entity` it executes as after `execute as` (and `player()` when that is a player), and named `CommandArgumentValue` values, not raw
  command text. Integer, number and boolean values retain the dispatcher's types; word and greedy text are strings. Player selectors resolve
  to a single live `PlayerRef`, items become component-bearing stacks of one, and block positions resolve source-relative/local
  coordinates without requiring loaded chunks. `mod_id` is a loaded package id as a string; `resource:<registry>`
  is `CommandArgumentValue::Id`, an id that registry holds. A custom argument is `CommandArgumentValue::Custom
  { kind, value }`: its registered type's id and what its parser returned. Missing optional arguments are absent.
  Parsing, permission and resolution failures produce feedback without calling the executor. The console resolves from the world spawn's
  block corner, in the level the spawn is in, with zero rotation and no source entity (`@s` fails).
  Return `Ok(n)` for success, including `Ok(0)`, or `Err(message.into())` with a `Text`. Failed commands store zero. No other mod is asked.
  Failure text defaults to red and does not depend on `sendCommandFeedback`. It is not shown for forked commands, silent sources, or sources that do not accept failures.
- **`armor_stand_manipulate(player, name, stand_id, slot, cancel)`**: a player takes an item from an armor stand or
  puts one on it, heard after the stand's disabled-slot checks and before anything moves. `slot` is `mainhand`,
  `offhand`, `feet`, `legs`, `chest` or `head`. Set `cancel` and nothing changes (the click is spent). The player's
  state is swapped in; `player` is their UUID. Not told of a click that changes nothing; breaking or blasting a stand and
  a dispenser armouring one are other ways its equipment moves and are not covered.
- **`vehicle_moved(player, kind, from, to)`**: a boat or minecart that carries a server player (`player`, its UUID)
  moved since the last tick, heard once a tick after the move with its positions before and after. The rider's state is
  swapped in, so `host.players().acting_player`, `PlayerRef::tell` and the item helpers reach them. A vehicle a player rode and left is
  still told of, with no `player` and nobody swapped in, until it has stood still for ten seconds. Mounts (horses...) and
  vehicles nobody ever rode are not covered. It only hears and costs one position compare a tick for each watched vehicle.
- **`container_opened(player, name, kind, pos, cancel)`**: a player is about to open one of the game's own menus
  (a chest, furnace, crafting table, villager, anvil, lectern, a horse's inventory...). `kind` is the menu type
  (`minecraft:generic_9x3`, `minecraft:merchant`...), `pos` the block it belongs to, none for a villager, a mount or a
  vehicle's chest. Heard after the checks that refuse the menu (a locked chest), before the open menu closes. Set
  `cancel` and nothing opens. The player's state is swapped in, so `host.players().acting_player`, `PlayerRef::tell` and the item
  helpers reach them. Menus a mod opens (`PlayerRef::open_chest_menu`, `PlayerRef::open_mod_menu`) are not heard.
- **`creative_slot(player, name, slot, item, count, cancel)`**: a creative-mode player sets a slot from the creative
  screen (`slot` is the inventory menu's slot; below 0 is a stack thrown out of the screen), with the `item` (`minecraft:air`
  for an emptied slot) and `count`. Heard after vanilla's checks, before the slot changes. Set `cancel` and the packet is
  ignored. The player's state is swapped in. Stacks a creative player gets other ways (pick block, `/give`) are not heard.
- **`Hooks::player_game_mode_changed`**: a server player's game mode is about to change, from `event.from` to
  `event.to`, both `GameMode` (`Survival`, `Creative`, `Adventure`, `Spectator`): the `/gamemode` command, the
  F3+F4 switcher's packet and a hardcore world's respawn into spectator, once per change to a different mode. The mode a
  player joins with (`defaultgamemode`) is not heard. Write `*event.to` to send the player to another mode instead (each listener sees the previous one's choice); return `Flow::Cancel`, or leave `to` at the current mode, and the player keeps their mode. Inside the
  command and the switcher the player's state is swapped in; for the hardcore respawn it is not. The hook receives a
  `&mut dyn ServerHost`, and `event.player` identifies the player by UUID.
- **`player_changed_dimension(player, name, from, to)`**: a player changed dimension, once per move
  (`from` and `to` are ids such as `minecraft:overworld` and `minecraft:the_nether`), heard when the move has ended:
  the portal's exit is found and the player stands there, a command's teleport has landed, or a respawn in another
  level than the death's is placed for good. `to` is the level in place and the player's state is swapped in (as in
  a command executor), so `host.players().acting_player` and `PlayerRef::tell` reach them. Heard for Nether and End portals
  (also riding, and leaving the End as its winner), `/execute in ... run tp` and any command teleporting into another
  level, and death respawns, on dedicated and integrated servers (single player and LAN), where the world is simulated.
  Not heard by a remote client window, for a move within a level (`/tp`, an End gateway), for a player who leaves before the move ends, or for a move ending where it started (a respawn in the death's level).
- **`player_toggle_flight(player, name, flying, cancel)`**: a player's client started (`flying`) or stopped
  flying, once per change, for a player who may fly (creative or spectator), on a server. The player's state is swapped
  in, so `PlayerRef::tell` and the other helpers reach them. Set `cancel` to refuse it: the player keeps the state they
  had and gets their abilities back, so the client lands or flies again. Only what a client asks for is heard: not the
  flight a game-mode change starts or ends, and not a LAN host's own player.
- **`player_switch_hotbar_slot_pre(player, player_entity_id, old_slot, new_slot, cancel)`** and
  **`player_switch_hotbar_slot_post(player, player_entity_id, old_slot, new_slot)`**
  (`PlayerSwitchHotbarSlotEvent.Pre`/`.Post`): a server player switches to a different hotbar slot. Pre runs after
  the slot range check and before it changes; set `cancel` to keep the old slot, and the client is sent that slot
  again. Post runs after the change. The player's state is not swapped in.
- **`furnace_burn(pos, fuel)`**: a furnace consumed `fuel` and started burning at `pos`.
- **`furnace_smelt(pos, item, result)`**: a furnace finished smelting `item` into `result` at `pos`.
- **`player_item_cooldown(player, name, item, ticks, cancel)`**: an item cooldown is about to start for a
  server player, including a used item's cooldown and a shield disabled by a hit. `item` is the item id and `ticks`
  its duration. Set `cancel` to skip the cooldown; shield disable still stops the use and plays its configured sound.
  The player's state is not swapped in.
- **`player_recipe_discovered(player, name, recipe, cancel)`**: a recipe id is about to become known to a
  server player through an advancement reward, a knowledge book or `/recipe give`. Set `cancel` to keep it locked;
  the client is sent the matching removal. The player's state is not swapped in.
- **`native_player_set_spawn(player, player_entity_id, name, dimension, pos, forced, cancel)`**: a server player's
  respawn point is about to be set, at each `ServerPlayer.setRespawnPosition` call: every sleep in a bed, a new respawn
  anchor point, `/spawnpoint`, and the copy to a replacement after `player_clone` (`pos` none for a player with no point).
  Cancel to keep the replacement's current point, including one set by a clone listener.
- **`entity_travel_to_dimension(entity, dimension, cause, cancel)`**: a server entity is about to travel to
  the level `dimension`, before it moves (NeoForge's `EntityTravelToDimensionEvent`); `cause` is `nether_portal`,
  `end_portal`, `end_gateway` or `teleport` (a `/tp` or spectator teleport into another level). Set `cancel` to
  keep it where it is. Players and every other entity that crosses a portal or a command's teleport are asked;
  passengers are asked recursively, and a refused passenger stays behind, dismounted, with its own subtree.
  Discarding an entity prevents its travel. A player leaving the End is asked about the resolved destination, including the world spawn when the saved point is missing. Respawn after the End credits is not travel. The player's state is not swapped in.
- **`block_damage_abort(player, name, pos)`**: a server player stopped digging `pos` before it broke. It is
  heard after the server clears the dig and sends the block state back to the client. The player's state is not
  swapped in.
- **`player_pick_item(player, name, from_entity, pos, entity_id, cancel)`**: a server player's middle-click
  pick resolved a block or entity's pick stack and is about to put it in the hotbar. For a block, `pos` is its
  position and `entity_id` is zero; for an entity, `from_entity` is true, `pos` is `[0, 0, 0]` and `entity_id`
  is its network id. The player's state is swapped in. Set `cancel` to leave the inventory and client unchanged. A
  pick with no result is not heard.
- **`native_sign_changed(player, name, dimension, pos, front, lines, cancel)`**: a player finished a sign they were
  editing, heard once every check has passed (editing that sign, in reach, not waxed) and right before
  the text goes on it. `lines` holds the four lines as written, formatting codes still in; rewrite them and that is
  what the sign gets, or set `cancel` and it keeps its old text. The
  player's state is swapped in, so `PlayerRef::tell` reaches them. Not told: the host's own editing on a LAN or
  single-player world, and changes that are no text edit (dye, glow ink, wax, `/data`, `/setblock`).
- **`player_client_loaded_world(player, name)`**: the server heard that a guest's client finished loading
  the level around them after joining, respawning or changing dimension. The player's state is not swapped in.
- **`player_recipe_book_settings_change(player, name, book, open, filtering)`**: a server player's recipe
  book open or filter setting changed; `book` is the book type's protocol id. The player's state is not swapped in.
- **`player_command(player, name, command, canceled, replies)`**: any command a player typed (without the
  slash), vanilla's too and commands the player may not run, heard before anything looks at it, and each line of a
  quiet command batch. Set `canceled` and nothing runs; push lines to `replies` and the player gets them as system chat
  (a quiet batch drops them). The player's state is not swapped in, and `PlayerRef::tell` and the other helpers that take a
  UUID reach nobody inside this hook: answer through `replies`. Commands may carry secrets (`/msg`, a login plugin's
  password), so don't log them carelessly.
- **`player_respawned(player, name, end_conquered)`**: once after `PlayerList.respawn` places and synchronizes the replacement (NeoForge's `PlayerRespawnEvent`). The player's state is swapped in for the call, so `host.players().acting_player()` names them. `end_conquered` is true after the End credits (health, food and experience are kept) and false after death. An ordinary End teleport does not fire it.
- **`player_login(player, name, cancel, reason)`**: a player the server has accepted (their name, the whitelist,
  bans and the player limit are checked) and who holds a place, heard before their saved state is loaded and before they
  are placed in a level (NeoForge's `PlayerNegotiationEvent`). Set `cancel` and they are disconnected with `reason` (a
  plain "Disconnected" when it stays `None`); no join message and no `player_logged_in` follow. It runs after the
  client's configuration, so the client sees the refusal as an in-game disconnect screen. The player is not placed yet,
  so no player's state is swapped in and `PlayerRef::tell` reaches nobody: the refusal text is the answer. Players the
  server refuses itself (name, whitelist, bans, a full server, the proxy's forwarding) are not heard.
- **`player_teleported(player, name, from, to, dimension)`**: a server player a command (`/tp`, `/teleport`,
  also what a mod's `run_command` reaches) moved, inside their level or into another; `from` and `to` are their feet
  before and after and `dimension` the level they are in now. Heard after the move has reached the player's state and
  their client (into another level: when that level is entered, before it has loaded around them), so it cannot stop it
  (NeoForge's `EntityTeleportEvent`). The player's state is swapped in, so `PlayerRef::tell` and the acting-player helpers
  work. Ender pearls and chorus fruit have their own hooks below; not heard yet: a spectator's teleport menu; portals
  and respawns are not teleports here.
- **`native_ender_pearl_teleport(owner, pearl, from, target, damage, cancel, hit_entity, hit_block, hit_face, hit_at)` (version 3)**: a server player's or mob's
  pearl landed and is about to teleport its live `EntityRef` owner. `target` may be moved; cancellation spends
  the pearl without teleport, sound, damage or endermite roll. Players retain acting-player context and
  mutable fall damage (5). Mobs preserve rotation, reset fall distance, take no player fall damage and
  make no endermite draw. The pending move refuses an owner changed by the listener. The hit fields say what the pearl hit (`getHitResult`): an entity (`hit_entity`, with the cell of `hit_at` and face `[0, 0, 0]`) or a block and its face normal.
- **`item_consumption_teleport(consumer, consumed, from, target, cancel)` (version 2)**: a server player or fox whose `teleport_randomly`
  consume effect (chorus fruit) is about to try a spot (NeoForge's `EntityTeleportEvent.ItemConsumption`,
  `TeleportRandomlyConsumeEffect.apply`): once per attempt, up to 16, after they stopped riding. A mod may move
  `target` (its y is kept inside the level's height); the game still looks down from it for ground to land on, inside
  the border. Canceling ends the effect: no
  teleport and no more attempts, but the food is consumed. `consumer` is a `LivingRef` and `consumed` is the
  captured stack before consumption (count and components included). A fox's listener receives the full server
  host before its living movement and before the next mob ticks; writes to actors or the level survive the
  continuation. Changing the consumer's mouth item, position or eligibility refuses the captured effect.
- **`player_clone(player, name, was_death, original)`**: after death or the End credits, after old-player
  cleanup and data copy but before the replacement is placed. `OriginalPlayer<'_>` lends the retained old player,
  including its source level and independently journaled attachments, only for this call. With no subscriber,
  no old player is retained, though attachment copy still runs. See `Hooks::player_clone` for the full contract.
- **`player_load_from_file(player, name, directory)`**: a joining player's saved state was read and applied
  (NeoForge's `PlayerEvent.LoadFromFile`, `PlayerList.loadPlayerData`), before they are placed in a level: once for each
  join on a server, for an integrated server's owner (loaded from the file `level.dat` names, else their own) as for
  anyone else. `directory` is the world's `players/data` folder, created if missing; read your own file there, named `<uuid>.<suffix>`. It
  is heard whether or not the player was in the world before (a new player has no file). No player's state is swapped
  in. It cannot change or stop anything.
- **`player_save_to_file(player, name, directory)`**: a placed player's data was saved (NeoForge's
  `PlayerEvent.SaveToFile`, `PlayerList.save`): when they leave or the world closes, when the dedicated server saves
  everyone, and for an integrated server's owner when the world saves their state into `level.dat`. `directory` is the
  world's `players/data` folder, created if missing; write your own file there, named `<uuid>.<suffix>`, and never the player's `.dat`.
  The player may be leaving, so no player's state is swapped in. It cannot change or stop anything.
- **`player_commands_sent`**: before every outgoing command tree (join, permission refresh, respawn and
  level change), the listener gets the live `PlayerRef` and a view of the roots that passed the player's permissions.
  Return `event.tree.hide_where(...)` to omit roots in that send; aliases leading into a removed branch disappear too.
  The server dispatcher is unchanged (stop execution with `player_command`).
- **`player_sprint(player, name, sprinting)`**: a server player's client started or stopped sprinting (the sprint
  key, a double tap forward, letting go), once for each change, after the server's player took it. It only hears. It
  reports what the client asks, not what the server does: a sprint that ends by itself (a hit's knockback, hunger, a
  wall) is not heard. The player's state is swapped in, so `PlayerRef::tell` and the acting-player helpers work.
- **`resource_pack_status(player, name, status)`**: a server player's client answered a resource pack while
  playing (`ServerboundResourcePackPacket`): `status` is `Action`'s number (0 loaded, 1 declined, 2 failed download,
  3 accepted, 4 downloaded, 5 invalid URL, 6 failed reload, 7 discarded). It only hears. The pack a dedicated server
  offers during the configuration is answered before the player has joined and is not heard. The player's state is
  swapped in, so `PlayerRef::tell` works.
- **`tnt_primed(player, pos, cancel)`**: TNT at `pos` is about to be lit by a flint and steel or fire charge
  (`player` is whoever clicked, whose state is swapped in), fire, a redstone signal, an arrow or a dispenser (`player` is
  None). Set `cancel` and it is not primed: the block stays, or is put back if fire or a signal had already taken it
  out. TNT set off by a blast, a TNT minecart and a sulfur cube are not asked; `explosion_detonate` can spare
  the blocks of a blast.
- **`hanging_place(player, name, kind, pos, cancel)`**: a server player places an item frame, a glow item
  frame, a painting or a leash knot (tying a lead to a fence that has none; `pos` is the fence), once it fits and
  before it is placed or the item used. Set `cancel` and nothing happens; the item stays in the hand. The player's
  state is swapped in, so `PlayerRef::tell` answers them. `/summon` and a lead tied to a mob are not heard.
- **`hanging_break(player, kind, pos, cancel)`**: an item frame, a glow item frame or a painting (`kind`)
  hanging in `pos` is about to break: a server player's hit (`player`, whose state is swapped in), a blast, or the
  wall it hangs on gone (`player` None for both), before anything drops or sounds. Set `cancel` and it stays as it
  was. A hit that only takes a frame's item out, the game's own player and `/kill` are not asked.
- **`bucket_empty(player, name, pos, fluid, cancel)`**: a server player empties a bucket (`fluid` is
  `minecraft:water`, `minecraft:lava` or `minecraft:powder_snow`; fish and axolotl buckets pour water) into `pos`,
  right before the fluid is placed. Set `cancel` and nothing is poured. The player's state is swapped in, so
  `PlayerRef::tell` answers them. Filling a bucket is `fill_bucket`'s moment (version 2); dispensers
  and cauldrons are not heard. A fluid or mob bucket is also heard at the block beside the clicked face whenever that
  block is not `pos`, since a block that refuses the fluid passes it there; the fluid may still go into `pos`.
- **`player_bed_enter(player, name, pos, cancel)`**: a server player passed the sleep checks and is about to
  enter the bed at its head position. Set `cancel` to keep them out.
- **`player_bed_leave(player, name, pos, wake_immediately, update_level)`**: A server player is getting out of a bed (`PlayerWakeUpEvent`), heard before their sleeping state is cleared, so they still read as sleeping at `pos`, the head position. `update_level` is false for the morning wake of all players and true when the player got up; `wake_immediately` is true for generic stop-sleeping, missing-bed and death wakes, false for morning, explicit leave and bed-rule refusal; the hook owns the full caller table.
- **`player_edit_book(player, name, pages, signing, cancel)`**: A server player book-and-quill edit or signing packet is about to be applied. `pages` is its page count and `signing` says it has a title. Set `cancel` to drop the packet.
- **`player_shear_entity(player, name, entity_id, off_hand, cancel)`**: A server player is about to shear a sheep, mooshroom, snow golem or bogged. The hook gives the mob entity id and whether the shears are in the off hand. Set `cancel` to leave the mob and shears unchanged.
- **`player_advancement_done(player, name, advancement)`**: A server player completed an advancement after its progress was recorded. `advancement` is its identifier, such as `minecraft:story/mine_stone`.
- **`player_advancement_progress`**: A criterion of a server player's advancement was granted or revoked, after its progress changed and before any completion rewards, including the `minecraft:tick` criteria granted on a network player's first tick or a reload. The server-side event supplies `player` (UUID), `player_entity_id`, `name`, `advancement`, the changed `criterion`, and `progress_type` (`ProgressType::Grant` or `ProgressType::Revoke`). Its `progress` is an `AdvancementProgress` snapshot of this player's complete state at that mutation: all criteria in definition order with their original grant dates (or `None`), the requirement groups, the number of satisfied groups, and `done`. Each group is an OR of criteria; all groups must be satisfied, and an empty requirements list is not done. Clone the snapshot to retain it without borrowing the game; subsequent grants, revokes and reloads cannot change it. This is a read-only observation, not a way to write progress. A grant or revoke that changes nothing is not heard. The snapshot is built only when a listener subscribes.
- **`player_stat_award(player, name, stat_type, stat, value, cancel)`**: A statistic is about to be awarded to a server player, before its count and scoreboard criteria change. `stat_type` and `stat` are writable canonical namespaced strings, such as `minecraft:mined` and `minecraft:stone`, or `minecraft:custom` and a registered custom statistic. `value` is a writable `i32` amount. Unknown types or entries, noncanonical ids, and negative amounts reject the entire rewrite (warned once), keeping the original identifiers and amount. Set `cancel` to award nothing. Integrated and LAN servers apply the edits to counters, scoreboard criteria and dirty statistics packets. In a transaction, the hook and award wait for the outermost successful commit; aborts emit neither.
- **`player_riptide(player, name)`**: a server player launched with a riptide trident, after its velocity and spin are applied. The player's state is swapped in.
- **`player_fish(player, name, state, cancel)`**: a player's fishing rod, on a server: `state` 0 the cast, 1 a fish
  biting, 2 the reel-in of a bite (the catch), 3 the reel-in with nothing on the line, 4 the reel-in of a bobber hooked
  to an entity or item. Set `cancel` and nothing is thrown (0), the fish gets away and the wait starts over (1), or the
  bobber stays out with no wear on the rod (2 to 4). The bobber ending by itself (too far, rod put away, owner dead) is
  not heard, nor is the host of a LAN world's rod. The player's state is swapped in, so `host.players().acting_player`,
  `PlayerRef::tell` and the item helpers reach them.
- **`block_dispense_armor(dimension, pos, item, target)`**: a dispenser equipped `item` on the first eligible player, mob or armor stand in front of its block at `pos`. The equip, item consumption, sound and game event have completed; `target` names the kind of entity equipped.
- **`block_shear_entity(dimension, pos, entity, kind)`**: a dispenser sheared the entity of `kind` and id `entity` at the block in front of the dispenser at `pos`, after applying the shearing effects.
- **`block_receive_game(dimension, pos, block, event, cancel)`**: a sculk sensor, calibrated sensor or shrieker received the vibration `event`. Set `cancel` to suppress its response.
- **`brewing_start(pos, ingredient)`**: a brewing stand at `pos` starts its timer for `ingredient`, after it consumes fuel. Server-side.
- **`campfire_start(pos, item)`**: `item` was placed on the campfire at `pos` and began cooking. Server-side.
- **`block_cook(pos, item, result)`**: a campfire finishes cooking `item` into `result` at `pos`, before the result is dropped. Server-side.
- **`compost_item(pos, item)`**: a player adds a compostable item to the composter at `pos`, after its layer is applied. Server-side.
- **`crafter_craft(pos, recipe)`**: a crafter completed `recipe` at `pos`, after consuming its ingredients and handling its output. Server-side.
- **`player_attack_cooldown_reset(player, name)`**: a server player's punch was handled: the main-hand swing and attack-strength reset have both happened. The player's state is not swapped in.
- **`player_bucket_entity(player, name, entity_id, entity)`**: a server player caught a bucketable mob after the mob was removed and the filled bucket was put in the player's hand or inventory. `entity` is the mob species id.
- **`player_change_beacon_effect(player, name, pos, primary, secondary)`**: a server player's beacon selection was stored. Effect values are `Option<EffectKey>`, `None` when none is selected.
- **`player_deep_sleep(player, name)`**: a server player reached 100 ticks asleep in a bed and now counts as deeply asleep for the night-skip check.
- **`player_egg_throw(player, name)`**: a server player threw an egg after its projectile was launched and its use applied.
- **`player_elytra_boost(player, name)`**: a server player used a firework rocket to boost while gliding, after the rocket was launched and consumed.
- **`player_flower_pot_manipulate(player, name, pos, plant)`**: a server player put `plant` into the flower pot at `pos`, after its state changed and the item was consumed.
- **`player_insert_lectern_book(player, name, pos, book)`**: a server player put `book` on the lectern at `pos`, after the block entity and inventory changed.
- **`player_take_lectern_book(player, name, pos, book)`**: a server player removed `book` from the lectern at `pos`, after its block entity changed.
- **`player_lectern_page_change(player, name, pos, page)`**: a server player changed the lectern at `pos` to zero-based `page`, after its page and redstone pulse changed.
- **`player_item_frame_change(player, name, entity_id, action, item, rotation)`**: a server player changed a frame (`put`, `rotate`, `remove_item` or `break`), after its contents and comparator output changed. `item` is empty when none remains.
- **`player_item_mend(player, name, item, repair, experience)`**: an orb actually repaired `repair` durability on `item` for a server player, after the item changed. `experience` is the orb value.
- **`player_shield_disable(player, name, ticks)`**: a blocked melee hit disabled the server player’s shield for `ticks` ticks. It fires just before the cooldown is set.
- **`player_loom_pattern_select(player, name, pattern)`**: a server player selected loom pattern button `pattern`, after the menu accepted it.
- **`player_name_entity(player, name, entity_id, entity_name)`**: a server player used a named name tag on a mob, after it became persistent and was named.
- **`player_open_sign(player, name, pos, front)`**: a server player opened the sign editor at `pos` for the front or back side, after the editor state was made.
- **`player_pickup_arrow(player, name, entity_id, item)`**: a server player picked up an arrow after it was removed and its pickup was applied. `item` is the pickup item id.
- **`player_stonecutter_recipe_select(player, name, recipe)`**: a server player selected stonecutter recipe button `recipe`, after the menu accepted it.
- **`player_swap_with_equipment_slot(player, name, slot)`**: a server player swapped their main hand with `slot` (currently `offhand`), after both hands changed.
- **`player_toggle_entity_age_lock(player, name, entity_id, locked)`**: a server player used a golden dandelion on an ageable mob, after its age lock changed and the item was used.
- **`player_unleash_entity(player, name, entity_id)`**: a server player's action removed an entity's leash, after its leash state was cleared.
- **`player_velocity(player, name, velocity)`**: the server synced changed motion to a player, with `velocity` in blocks per tick.
- **`player_stop_spectating_entity(player, name, entity_id)`**: a spectator's camera returned to their own player after they stopped watching `entity_id`.
- **`player_map_filled(player, name, map_id)`**: a server player filled an empty map, after the filled map and its saved map data were created.
- **`player_teleport_end_gateway(player, name, gateway, cancel)`**: a player is about to travel through the End gateway at `gateway`. Set `cancel` to leave them at the entrance; the gateway does not start its cooldown. The player is not swapped in.
- **`player_recipe_book_click(player, name, recipe, all)`**: a server player requests placement of a known recipe through a vanilla or mod book, after menu and recipe checks and before placement or the mod menu's own decision. `all` is true for the Shift-click form. The player is not swapped in.
- **`player_exp_cooldown_change(player, name, ticks)`**: the player’s XP orb pickup cooldown changed, after its new value is stored. This includes the two tick delay after an orb is picked up and each tick it counts down. The player is not swapped in.
- **`entity_enter_block(entity, kind, pos, block)`**: a bee was stored in the beehive or nest at `pos`.
- **`entity_change_block(entity, kind, pos, from, to)`**: a mob changed a block, after the new state was applied.
- **`entity_break_door(entity, kind, pos)`**: a mob finished breaking a door at `pos`, after the block was removed.
- **`creeper_power(entity, lightning)`**: a lightning strike charged the creeper.
- **`living_visibility(entity, looking, visibility)`**: how visible `entity` is to `looking` (NeoForge's `LivingVisibilityEvent`, last in `LivingEntity.getVisibilityPercent`). `visibility` comes in as vanilla's percent: sneaking 0.8, invisibility scaled by armour cover, and 0.5 for a worn mob head whose default `mob_visibility` names the looking mob's type (a zombie head fools zombies, not husks). Multiply it to scale the range a targeting check sees the entity from. A negative answer counts as 0, and the result is clamped to 10, as vanilla clamps. It asks for a player, or for a mob (no sneaking factor: vanilla reads it through `isDiscrete`), seen by a mob whose targeting check scales by visibility (`TargetingConditions.testInvisible`): hostile targeting, the sensors, the happy ghast's tempt, a mob hunting a mob. It asks after the alive, spectator and selector checks, as `TargetingConditions.test` does. With no listener, `MobWorld::mob_hooks` is `None` and nothing is asked.
- **`entity_mob_griefing(entity, can_grief)`**: may `entity` change the world (NeoForge's `EntityMobGriefingEvent`, `EventHooks.canEntityGrief`)? `can_grief` comes in as the `mob_griefing` rule. Sunrise asks with the entity responsible for the change: the exploding entity for a mob explosion, the owner for a mob's arrow or small fireball, the wither for its rose, otherwise the mob itself. A zombie breaking a door, the dragon breaking its walls, a ravager breaking pitcher crops, a burning mob melting powder snow and a weaving mob's cobwebs ask with that mob. Whether an explosion reaches items and decorations asks with the exploding entity, and whether a mob-caused blast harms decorations asks with that mob. A blast's vehicles and the ravager's roar on armor stands read the rule. The game asks through `Game::can_entity_grief` and the mob AI through `MobWorld::can_grief`. With no listener both read the rule: one `listened` check, no dispatch. A large fireball's fire asks for its owner: the ghast that shot it, or the player who sent it back. A fireball whose owner is gone reads the rule. The blast's block damage still reads the rule, because Sunrise's mob explosion has no source entity yet.
- **`enderman_anger(enderman, player, cancel)`**: `player` stares at `enderman` (NeoForge's `EndermanAngerEvent`, last in `Enderman.isBeingStaredBy`, once the look cone, line of sight and carved pumpkin checks passed). It is asked each time the enderman's goals check the stare: to get angry, to freeze while looked at, and to teleport away up close. Set `cancel` and that stare doesn't count. The mob AI asks through `MobWorld::mob_hooks`, which is set only on the server while a mod listens: with no listener the stare check costs nothing more.

  Mob AI asks mods through `MobWorld::mob_hooks`, an `Option<&dyn MobHooks>`: one trait in the client lib, implemented once by the game, whose methods only build the hook's arguments and call its generated `native_*` entry. It is `Some` only on an authoritative server while a mod listens to one of its hooks, and each method is gated by its own hook's `listened`, so adding a method costs nothing to mods that don't listen to it. The hook gets a read `Host` over the game while the mob tick runs: the mobs being ticked and the level's random and attachments are lent to the tick, so through that host every mob's handle reads as gone and level attachments read as none. Use the event's own fields; anything that must write waits for a deferred hook.
- **`creeper_ignite(entity, player, item)`**: a player used a creeper igniter on the creeper, after it entered its ignited state.
- **`exp_bottle(pos, experience)`**: an experience bottle broke and its orbs were created.
- **`firework_explode(entity, pos, bursts)`**: a firework rocket completed its damage and explosion effects.
- **`potion_splash(item, pos)`**: a thrown splash or lingering potion finished applying its hit effects at `pos`. Experience bottles are excluded.
- **`entity_enter_love_mode(entity, kind, player)`**: a mob entered love mode after a player interaction; `player` is the feeder UUID when known.
- **`teleport_command(dimension, entity, from, to, cancel)`**: a server entity is about to be teleported by `/tp` or `/teleport`. `dimension` is the level it arrives in, `entity` its network id, `from` its feet now and `to` where the command sends it; move `to` to redirect it, or set `cancel` to keep it where it is (the command still reports success).
- **`entity_teleport(dimension, entity, from, to)`**: a server entity completed a same-level teleport command. `entity` is its network id; `from` and `to` are coordinates in `dimension`. This reports the move after it happens.
- **`entity_breed(parent, partner, child)`**: two mobs completed a breeding action. `child` is absent when the vanilla action lays an egg instead of creating a baby.
- **`entity_tame(entity, kind, owner)`**: a mob became tamed after its owner and persistent state were set. `owner` is the server player's UUID when known.
- **`item_spawn(entity, item, count, pos)`**: a dropped item entity was successfully added to the server level. The count is its stack size and `pos` is its spawn position.
- **`entity_pose_change(player, name, from, to)`**: a guest player's synchronized pose changed. Pose names are `STANDING`, `CROUCHING`, `SWIMMING`, `FALL_FLYING`, `SPIN_ATTACK` or `SLEEPING`.
- **`entity_toggle_glide(player, name, gliding)`**: a guest player's fall flying state changed after the server accepted or ended a glide.
- **`arrow_nock(player, name, bow, hand, has_ammo, cancel)`**: a player is about to start using a bow, where `BowItem.use` runs on the server (NeoForge's `ArrowNockEvent`). `has_ammo` is whether `getProjectile` found ammunition, always true in creative. Set `cancel` to fail the use: the bow is not drawn. Crossbows and tridents do not ask, and the client's own prediction does not.
- **`living_get_projectile(player, name, entity, kind, weapon, projectile)`**: the ammunition `LivingEntity.getProjectile(weapon)` returns (NeoForge's `LivingGetProjectileEvent`). Every mod gets the event with `projectile` set to the vanilla ammunition (the stack found in the hands or inventory, a mob's held projectile or a new arrow, a creative player's default arrow, or empty) and may change or empty it. It is asked on the bow's start and release, the crossbow's start and draw, the client's prediction of those starts, a skeleton's or illusioner's shot, and a pillager's or piglin's crossbow load (`entity` and `kind` name the mob, `player` is empty); a mob's arrow takes the replaced arrow's item and effect, a mob's crossbow also fires a replaced firework rocket, and an emptied stack leaves its crossbow uncharged and silent. A replaced stack is not taken from the inventory: the hook owns it.
- **`entity_portal(dimension, entity, pos)`**: a server entity finished its portal wait and is beginning to leave through the portal block at `pos`. The destination follows the existing portal rules; the hook reports the transfer without changing it.
- **`item_smelted(player, stack, amount_removed)`**: a player took furnace, blast furnace or smoker output with a nonzero `removeCount` (`FurnaceResultSlot.checkTakeAchievements`, NeoForge `PlayerEvent.ItemSmeltedEvent`). Fires on both logical sides (`HostSide::Server` on the server after the furnace awarded its used recipes and popped experience; `HostSide::Client` during client click prediction without awarding experience). `player` is the live player handle and `stack` lends the stack vanilla passed there (`ItemStackView<'_>`).
- **`furnace_start_smelt(pos, item)`**: a furnace at `pos` begins cooking `item`, when its cooking timer first advances. Server-side.
- **`brewing_fuel(pos, fuel)`**: a brewing stand at `pos` consumed `fuel` and gained brewing fuel. Server-side.
- **`enchant_item(player, name, pos, item, levels)`**: a player applied an enchantment at `pos`; `item` is the finished item and `levels` the levels charged. Guest state is not swapped in.
- **`prepare_item_enchant(player, name, pos, costs)`**: enchanting offers were calculated at `pos`; change the three offer costs before they are sent to the player's menu. Guest state is not swapped in.
- **`anvil_update(host, event)`** (`AnvilUpdateEvent`, both logical sides): fires live after `AnvilMenu.createResult` (with no answer cache), matching NeoForge `AnvilMenu.createResult` / `CommonHooks.onAnvilUpdate`. Read `event.inputs` and change `event.edits`'s output, XP cost or material cost. `StackResult::Keep` keeps vanilla's output and `StackResult::Set(None)` clears it. Cancellation leaves no output and zero costs; an invalid replacement output is logged and ignored (keeping vanilla's output while still applying the cost edits). `xp_cost` stays signed across listeners (`AnvilUpdateEvent.setXpCost(int)`) and clamps to `0` when applied (`AnvilMenu.setCost`); a zero cost prevents taking the output (`AnvilMenu.mayPickup` requires a positive cost). Negative material costs clamp to zero, which consumes the entire right stack when a positive-cost output is taken.
- **`anvil_craft_pre(host, event)`** (`AnvilCraftPreEvent`, server only): before an anvil take charges levels, uses the inputs or wears the anvil, `event.inputs` lends the pre-take left and right stacks and result. Set `event.cancelled` or return `Flow::Cancel` to return early before charging levels, consuming inputs, damaging the anvil or firing `anvil_craft_post`, without undoing the click's item move (matching NeoForge `AnvilMenu.onTake`). The client's predicted take does not fire it.
- **`anvil_craft_post(host, event)`** (`AnvilCraftPostEvent`, server only): after an accepted anvil take charges levels, uses the inputs and applies anvil wear, `event.inputs` lends the same pre-take snapshot as Pre. A broken anvil is already air. Post does not fire on a veto.
- **`prepare_smithing(player, name, pos, result)`**: a smithing result was prepared for a player at `pos`; change it before it is sent to their menu. Guest state is not swapped in.
- **`prepare_item_craft(player, name, pos, result)`**: a crafting-grid result was prepared; `pos` is the crafting table, or None for the player's inventory grid. Change it before it is sent to the menu. Guest state is not swapped in.
- **`trade_select(player, name, villager, index)`**: a player selected offer `index` in a villager's trading menu; `villager` is its entity id when known. The menu has updated and the player's state is not swapped in.
- **`hopper_pickup_item(dimension, pos, item, count, cancel)`**: a hopper at `pos` is about to pick up a dropped stack; set `cancel` to leave it in the world.
- **`weather_change(host, event)`** (`WeatherChangeEvent`, version 2): before `/weather`,
  `level.set_weather` or a GameTest weather environment writes the server's shared timers and flags. `event.dimension` names the origin,
  not the scope. `event.clear_ticks` is the proposed clear timer, and `event.weather_ticks` is
  the proposed rain and thunder timer. Inspect both flags (`event.raining` and `event.thundering`)
  in this one callback. Return `Flow::Cancel` or set `event.cancelled` to veto the whole write;
  the setter returns `false` on a final veto without undoing any listener's Host writes.
- **`lightning_strike(player, pos, cause, cancel)`**: a weather, Channeling-trident, or `/summon` lightning bolt
  is about to be created. Set `cancel` to prevent the bolt and its effects; `cause` names the creation path.
- **`entity_struck_by_lightning(entity, bolt, cancel)`**: a lightning bolt is about to hit `entity`
  (NeoForge's `EntityStruckByLightningEvent`), asked before vanilla's `Entity.thunderHit`, once per strike for each mob, player,
  item, vehicle or other entity whose box meets the strike box. Set `cancel` to spare it: no fire, damage or
  conversion, and `creeper_power` is not called. It still counts as struck for Channeling's advancement, as
  in NeoForge. A visual-only bolt asks nothing. With no listener the cost is one `listened` check per struck entity.
- **`time_skip(dimension, from, to, cause, cancel)`**: `/time` and level-clock setters may be vetoed
  before they change the clock. Sleeping uses `sleep_finished_time` instead.
- **`sleep_finished_time(dimension, current, new_time, adjustment)`**: enough players slept and `advance_time`
  is on (NeoForge's `SleepFinishedTimeEvent`). `adjustment`, a `ClockAdjustment`, starts as vanilla's
  `wake_up_from_sleep` marker; a mod may set `Absolute`, `Relative` or another `Marker`, read with
  `level_clock_marker_time`. `new_time`: the morning, read only (contract 2). Level's own clock, resolved after every mod;
  a time before `current` or past `i64::MAX`, or a marker that clock lacks (only the Overworld's has any), moves to the morning.
- **`world_border_bounds_changed(host, event)`** (version 2): the border at `event.level`
  was resized. `event.lerp_ticks` is `None` for a fixed write or `Some(requested_ticks)`
  for a timed request, even if the requested size equals the current size.
- **`world_border_center_changed(host, event)`** (version 2): the border at `event.level`
  was recentered, including equal-value direct writes. Read the named border with
  `event.level.world_border(host)` even if that level is not in place.
- **`spawn_change(dimension, pos, angle)`**: `/setworldspawn` moved the world's spawn point to `pos` in level `dimension`, after its level, position and rotation are applied.
- **`portal_create(dimension, pos)`**: a Nether portal built for dimension travel or the End arrival platform was completed in `dimension`; `pos` is its lower corner or platform origin.
- **`raid_spawn_wave(raid, pos, wave, raiders)`**: a raid wave is about to be spawned at `pos`; `raiders` is the count Sunrise will spawn.
- **`raid_state_change(raid, pos, state)`**: a raid at `pos` was triggered or changed to `victory`, `loss`, or `stopped`. Its `RaidRef` identifies the owning level, not necessarily the level currently in place.
- **`world_saved(wait)`**: the server completed its world save cycle; `wait` is true when chunk writes were flushed before returning.
- **`entities_load(dimension, chunk, count)`**: saved entity records for `chunk` finished loading into `dimension`.
- **`entities_unload(dimension, chunk, count)`**: entities in `chunk` left the active world and were queued for saving.
- **`tab_complete(player, name, command, range, suggestions)`**: a guest's server completion request was evaluated; mods may add or filter suggestions. `range` is the replaced byte range. Guest state is not swapped in.
- **`broadcast_message(line, cancel)`**: a system broadcast line is about to be logged and sent to every player. A mod may change `line` or set `cancel` to drop the broadcast.
- **`whitelist_state_update(enabled)`**: the dedicated server's player whitelist was enabled or disabled.
- **`player_bundle_item_select(player, name, slot, index)`**: a server player selected an item in a bundle;
  `slot` is the packet's menu slot and `index` is the chosen item index.
- **`prepare_hopper_transfer(pos)`**: a hopper at `pos` begins a transfer attempt, after its cooldown and enabled checks and before pushing or pulling items.
- **`inventory_move_item(source, destination, item, cancel)`**: a hopper is about to move one item between containers (a composter's bone meal too). Set `cancel` to leave it in the source slot. A chest boat or minecart is reported at the block it is in.
- **`hopper_inventory_search(hopper, container, entity)`**: a hopper selects a block container or container entity while searching for items.
- **`vault_loot(pos, item)`**: a vault is about to eject a loot item, before the item entity spawns.
- **`world_border_bounds_change_finish(host, event)`** (version 2): when an actual moving extent
  reaches its target size at the start of `event.level`'s normal tick, even when the level is idle and
  parked. Starting or replacing an extent is not a finish.
- **`entity_unleash(entity, kind)`**: a mob's leash was removed, before any lead drop is handled.
- **`entity_drop_item(entity, item, count, pos)`**: a mob is about to drop `count` of `item` at `pos`.
- **`explosion_prime(center, radius)`**: a server explosion is about to begin at `center` with `radius`.
  It is listen-only, so the explosion proceeds as usual; it also fires on the single-player server.
- **`vehicle_damage(kind, damage, cancel)`**: a player is about to damage a boat or minecart. Change `damage`
  or set `cancel` to leave its damage and wobble unchanged; the attack still completes. `kind` is `boat` or the
  minecart type. Server-side.
- **`vehicle_destroy(kind, pos)`**: a boat or minecart is about to be destroyed at `pos`. TNT minecarts that
  prime instead of breaking are not heard. Server-side.
- **`vehicle_create(kind, pos)`**: a boat or minecart joined the level at `pos`, after being placed by a
  player or dispenser. Server-side.
- **`player_toggle_sneak(player, name, sneaking)`**: a guest's player input changed sneaking, after the hub applied the state. The guest's state is swapped in; only actual changes are reported.
- **`player_input(player, name, flags)`**: a loaded guest sent a `PlayerInput` packet, after its flags were applied to the hub. `flags` is the packet's bit field, and the guest's state is swapped in.
- **`player_arm_swing(player, name, hand)`**: a guest's arm swing was handled and their attack strength reset. The packet currently swings `mainhand`; the guest's state is swapped in.
- **`player_kicked(player, name, reason)` (version 2)**: a network or fake guest was kicked after the hub recorded the rendered disconnect reason; a network guest also receives the disconnect packet. The guest's state is swapped in, unless the guest was already removed in the same poll: the hook still fires, without it.
- **`player_level_change(player, name, from, to)`**: a server player's experience change altered their level, after the new level was applied. The guest's state is swapped in when the player is a guest.
- **`player_swap_hands(player, name, to_main_hand, to_off_hand, cancel)` (version 3)**: a player pressed the swap-hands key (not a spectator), heard
  before the main and off hand items swap. `to_main_hand` starts as the off hand's stack and `to_off_hand` as the main hand's; the hands get what the listeners leave, and the client gets both slots when they changed. Set `cancel` and they stay where they are; the player's client
  is sent both slots again. The player's state is swapped in, so `host.players().acting_player`, `PlayerRef::tell` and the item
  helpers reach them. Heard for a server's players, the single-player owner and a LAN host. Not heard: the same swap from an open inventory (hovering a slot and pressing the key).
- Services: `host.players().server_players()` returns online player handles in connection order;
  `player.tell(host, text)` sends a system line to one player, also the acting player inside their hook;
  `host.messages().broadcast(text)`, to every player and the server's
  log; `native_server_event(name, uuid, message)`, an `announce` event on a dedicated server's event stream, which the Discord
  bridge relays when `[discord] events` lists `announce`; `host.commands().feedback(text, broadcast_to_admins)`, the command's answer to the player whose command it is (and, broadcast, the other operators' `[name: text]` line); `native_world_dir()`, the
  open world's folder, where a server mod keeps its data in a folder of its own so the world's backups carry it.
- Items: a player's inventory is their [`ENTITY_ITEMS`](../concepts/capabilities/entity-items.md) handler, from any server hook.
- Experience: `level.visit_experience_orbs(host, min, max, visit)`, lending each orb as an `ExperienceOrbRef`
  (`entity`, `value`, `count`, and `age`), read before a mod cancels an orb in `entity_join_level`; and a player's `give_experience_points(host,
  points)` and `give_levels(host, levels)` through their `PlayerRef`: signed points or levels are added or, when
  negative, taken off. Only actual orb pickup repairs items with Mending; canceling an orb and giving points instead
  does not run Mending. Both writes return false for an unavailable, dead or displaced player. A merged orb is
  worth `value` × `count`.
- `host.players().acting_player()`, the player whose state is swapped in (inside a command executor,
  `right_click_block`, `native_use_on`, `break_block`...; None elsewhere);
  `host.commands().run_command(line)`, a command run as the server's console (level 4, no output), including during a guest's
  turn. The acting player and other live guests are reachable; displaced outer players are excluded only on this
  native path, without changing ordinary player or console command resolution. A teleport to another level
  (`execute in minecraft:the_nether run tp Alex 0 70 0`) lands at the next tick's start. The returned player handles read health through their living handle (0 once dead).
  `player.action_bar(host, text)`, a line over a player's hotbar, which also reaches the acting player inside their
  hook.
- **`food_level_changed(player, name, from, to, cancel)`**: the food level of a player is about to change, on a
  server (eating, hunger and saturation effects, exhaustion, starving), before the new level reaches them. Change `to`
  (0 to 20) or set `cancel` to leave it at `from`, saturation included. It is heard once for what one turn of the
  player changed, not for the full bar a respawn gives, the food a player joins with, or the host of a LAN world. The
  player's state is swapped in, so `host.players().acting_player`, `PlayerRef::tell` and the item helpers reach them.
- **`right_click_block(click, use_block, use_item, cancellation_result, canceled)`** : NeoForge's `PlayerInteractEvent.RightClickBlock`, a player's right click on a
  block, once per hand where `useItemOn` starts, on the clicking client and on the server (`click.side`). `click`
  (`BlockClick`) names the level, block, hit, hand, held stack and the player's UUID and entity id. `use_block`
  and `use_item` (`TriState`) force the block's use or the item's `useOn` off, or on (the block even while
  sneaking with something in either hand, the item even with an empty hand or while it cools down).   Default skips sneak with either hand full (both-stacks bypass remains open in #2934) and requires a held item off cooldown; forced empty use reaches `ItemAfterBlock` with no stack or Part.
  Adventure restrictions still apply. Cancelling skips both and answers `cancellation_result` (`InteractionResult`, default `Pass`, which goes on to the item's own use and the other
  hand); with `break_block`, a cancel with `Fail` keeps players out of claimed land, and the player's client is
  sent the block back.
- **`use_item_on_block(click, phase, cancellation_result, canceled)`**: NeoForge's
  `UseItemOnBlockEvent`, on both sides, immediately before the selected block or item-on-block phase.
  Canceling replaces that phase with `cancellation_result` (default `TryWithEmptyHand`), skipping its
  Part and template. `UsePhase::ItemBeforeBlock`, `Block` and `ItemAfterBlock` all fire; `ItemBeforeBlock` precedes the item's
  `on_item_use_first` Part.
- **`right_click_item(click, cancellation_result, canceled)`**: NeoForge's
  `PlayerInteractEvent.RightClickItem`, on both sides, before `Item.use` and its Part after target
  fallthrough. Canceling answers with `cancellation_result` (default `Pass`). `Success`,
  `SuccessServer` and `Consume` end the client's hand loop. Only `Success` predicts a swing.
- **`block_spread(dimension, kind, from, to, canceled)`** and **`native_piston_pre(dimension, piston,
  direction, extending, push, destroy, canceled)`** and **`piston_post(dimension, piston, direction, extending, push, destroy)`** (on `GameLevel`, like the block observers): a fluid flowing or fire
  spreading (`kind` `fluid` or `fire`) from `from` into `to`, and a piston moving blocks once its structure resolved (`native_piston_pre`, NeoForge's `PistonEvent.Pre`; `piston_post`, `PistonEvent.Post`, hears the move done).
  Canceled, the block at `to` stays as it is, and the piston moves nothing. Fluids ask on every flow: answer from
  memory, fast.
- **`block_ignite(dimension, player, pos, cause, cancel)`** (on `GameLevel`): fire is about to be placed.
  It covers fire spread, lava's random fire placement, lightning strikes, and dispenser flint-and-steel. Set `cancel`
  to leave the position unchanged. Direct player flint-and-steel/fire-charge placement is not covered.
- **`block_burn(dimension, pos, cancel)`** (on `GameLevel`): fire's burnout check is about to remove the block at
  `pos`. Set `cancel` to keep it.
- **`crop_grow_pre(dimension, pos, state, result)`** and **`crop_grow_post(dimension, pos,
  original_state, state)`** (on `GameLevel`): NeoForge's `CropGrowEvent.Pre` and `.Post`, at the random-tick growth of
  crops (beetroot and torchflowers after their own 1 in 3 skip), stems, nether wart, cocoa, sweet berries, bamboo,
  cactus, chorus flowers, sugar cane, kelp and the vines. `result` is a `TriState`: `Default` keeps vanilla's
  chance, `True` grows, `False` doesn't. Pre fires at the position that grows (the block above for cactus and chorus,
  the block grown into for kelp and vines). Vanilla's random roll is drawn first, in vanilla's order, and passed as
  the default, so crops keep vanilla's random sequence. Pitcher crops, bamboo saplings and mangrove propagules
  don't fire.
- **`fluid_level_change(dimension, pos, fluid, from, to, cancel)`** (on `GameLevel`): flowing `water` or `lava`
  at `pos` is about to change level. Levels run from 0 through 8. Set `cancel` to keep the old level.
- **`neighbor_notify(dimension, pos, state, skipped_side, cancel)`** (on `GameLevel`): NeoForge's
  `NeighborNotifyEvent`. The block at `pos` is about to tell its neighbours it changed: `ServerLevel.updateNeighborsAt`
  (all six sides, `skipped_side` is `None`) or `updateNeighborsAtExceptFromFacing` (all but `skipped_side`). Set
  `cancel` and no neighbour hears, on both paths. Asked for every such update, redstone
  included: answer fast.
- **`create_fluid_source(dimension, pos, state, can_convert)`** (on `GameLevel`): NeoForge's
  `CreateFluidSourceEvent`. When the game works out a block's new fluid (`FlowingFluid.getNewLiquid`), it asks once
  for each horizontal neighbour that is a source of the same fluid, with that neighbour's `pos` and `state`; two
  counted sources over a solid block or a source make a new one. `can_convert` starts as the fluid's rule
  (`water_source_conversion` or `lava_source_conversion`). Spread checks work out new fluids too, so this is asked
  often: answer fast.
- **`leaves_decay(dimension, pos, cancel)`** (on `GameLevel`): a non-persistent leaf block at distance 7 is
  about to decay. Set `cancel` to keep it.
- **`inventory_block_start(player, name, pos, cancel)`**: a guest is about to open an inventory supplied by a
  block at `pos`. Set `cancel` to leave it closed. The guest's state is not swapped in.
- **`moisture_change(dimension, pos, from, to, cancel)`** (on `GameLevel`): farmland at `pos` is about to
  change moisture. Set `cancel` to keep its old moisture.
- **`sculk_bloom(dimension, pos)`**: after a sculk catalyst at `pos` takes experience, its block has entered
  the blooming state.
- **`sponge_absorb(dimension, pos)`** (on `GameLevel`): a sponge at `pos` has absorbed water and become wet.
- **`target_hit(dimension, pos, strength)`**: a projectile hit the target block at `pos`, which now has the
  reported redstone output `strength`.
- **`vault_change_state(dimension, pos, state)`**: the vault block at `pos` changed its state to `state`
  (`inactive`, `active`, `unlocking`, or `ejecting`).
- **`block_failed_dispense(dimension, pos, item)`**: a dispenser at `pos` failed to dispense `item`.
- **`beacon_state_change(dimension, pos, active)`**: the beacon at `pos` gained or lost its complete beam
  pyramid; `active` says whether it is active now.
- **`anvil_damaged(dimension, pos, from, to)`**: the anvil at `pos` took damage, changing from `from` to
  `to`; `to` is air when it broke.
- **`block_break_progress_update(player, name, pos, progress)`**: a guest's crack animation at `pos` reached
  progress stage 0 through 9.
- **`block_lock_check(player, name, pos, locked)`**: a guest checks the lock on the container at `pos`;
  `locked` starts with vanilla's result and may be changed. The guest's state is not swapped in.
- **`block_fade(dimension, pos, from, to, cancel)`** (on `GameLevel`): ice or snow melts, or coral dies. `from`
  and `to` are block state IDs. Set `cancel` to keep the old block.
- **`block_form(dimension, pos, from, to, cancel)`** (on `GameLevel`): precipitation is about to place ice or
  snow, or add a snow layer at `pos`. `from` and `to` are block state IDs. Set `cancel` to keep the old state.
- **`bell_ring(dimension, player, pos, cancel)`** (on `GameLevel`): a bell is about to ring. Set `cancel` to
  stop the ring and its raider reveal.
- **`native_cauldron_level_changed(dimension, player, pos, from_level, to_level, reason, cancel)`** (on `GameLevel`):
  precipitation, dripstone, or a burning player being extinguished is about to change a cauldron's level. Set `cancel`
  to leave it as it was.
- **`random_tick(ticks, dimension, pos, state, handled)`** (on `GameLevel`): a block `tickChunk`
  random-ticks, before its own `randomTick`. Change the block with `LevelWriteHost::set_block_property(pos, key,
  value, flags)`, using `BlockUpdateFlags`; set `handled` to skip the block's own random tick.
- **`tick_block(ticks, dimension, pos, state, handled)`** (on `GameLevel`): a due scheduled tick of a block
  that is still there (`ServerLevel.tickBlock`), before its own `tick`; the same contract as `random_tick`.
- **`explosion_detonate(dimension, center, blocks, entities)`**: the positions an explosion is about to
  break and the entities in its box, before any is hit; listeners may remove, add or reorder positions
  and entities (`ExplosionEvent.Detonate`).
- **`explosion_knockback(dimension, center, blocks, entity, knockback_velocity)`**: a server explosion's
  calculated velocity, before it is added to the entity. Replace `knockback_velocity` to change the push;
  `entity` is None for a player, whom `host.players().acting_player` names. The player packet uses the same replacement. Wind bursts are heard too.

- **`fill_bucket(stack, pos, canceled)`** and **`attack_entity(entity, kind, pos, canceled)`**:
  a player filling an empty bucket at a block (version 2: before `BucketPickup.pickupBlock`; emptying is
  `bucket_empty`), and their blow on an entity, with the acting player swapped in. Canceled, the bucket does nothing
  and the blow does no harm.

### Mobs

- `LevelRef::spawn_mob(host, kind, pos, reason)` spawns a mob and returns an `EntityRef`, or `None` when refused.
  `SpawnReason` controls finalization; the new mob is not persistent, as for a natural spawn. Spawn limits and natural
  spawn rules do not apply, as for `/summon`. A non-mob kind, invalid or out-of-bounds position, unloaded chunk,
  stale or unavailable level, or cancellation during finalization or level entry refuses the spawn. Configure the
  resulting entity through its handle rather than a spawn-options record.
- Visit the level with `level.visit_all_entities(host, &mut visit)` and narrow the handles with `host.mob(entity)` or `host.living(entity)`.
- A mob's health is set with `living.set_health(host, health)` on its `LivingRef`.
- `EntityRef::kill(host)` kills an entity as `/kill` does (living entities fall and drop loot, other entities are removed with `minecraft:entity_die`), routing parked dimensions and returning `Result<bool, LevelWriteError>`. `EntityRef::discard` removes an entity without death or loot: from the call on selectors and handles no longer find it, its passengers are off and `entity_leave_level` has fired, in the same hook.
- While the game ticks its mobs they are out of the level: from a hook inside that tick, a spawned mob joins after the tick, and the others
  read as gone.

### Loot tables

Changing loot a mod didn't make, as NeoForge's `LootTableLoadEvent` and global loot modifiers and Fabric's
`LootTableEvents.MODIFY` and `MODIFY_DROPS` do. A mod's own loot function and condition kinds are #5451.
- **`living_death(entity, kind, source, weapon, cancel)` (version 2)**: server-only, before loot, experience and death messages. `source: &HurtSource` retains the accepted killing blow's damage type, real causing entity (including players), direct entity (including projectiles) and original optional position; `weapon: Option<&ItemStack>` retains its full killing stack and components. `LivingDeathEvent` exposes `entity`, `kind`, `source`, `weapon`, `cancel`; the old top-level `damage_type`/`attacker` fields and callback signature are removed, with no shim. Cancel suppresses death loot and leaves health exactly where the listener left it: health 5 stays 5; an unhealed body at health <= 0 is removed after 20 ticks without redispatch or loot.
- **`sunrise_world::loot::loot_table_load(context, id, table)`**: each table's JSON as it loads, after the data packs'. `Hooks::loot_table_load` gets the owning load's `&dyn DataLoadContext` first (see "Items' default components"); its `origin()` is always `ServerDataLoad`, since only the server loads loot tables. Push a pool
  onto `table["pools"]` or an entry onto a pool's `entries`; chests, block and mob drops, fishing and the rest then roll the
  changed table. Keep a replaced table's `random_sequence`: a mob's table without one drops nothing. It runs once per table on
  each server data load and `/reload`, not on client preloads. It is a free function in
  `crates/sunrise-world/src/loot.rs`, so a helper injected there compiles inside `sunrise-world`, without `crate::game`.
- **`loot_drops(context, drops)`**: the items a broken block or a dead mob is about to drop, after its tables rolled (a
  mob's worn, held and carried items and inventory too, not an enderman's carried block or a copper golem's preserved equipment).
  Add, change or take out stacks; empty ones don't drop. `LootDrops` says what dropped them: its own loot table (empty when
  none rolled), the dimension, the origin, the block's position and state or the mob's id and type, the player who broke or killed
  it (100-tick player kill credit), the tool and explosion radius. Deaths carry `damage_source` (type, causing/direct IDs), `tool` is None, and `weapon_item(host)` reads the recorded projectile weapon or live melee weapon. It hears blocks
  that a player, an explosion or a mob breaks, not other breaks (leaves decaying, a torch losing its wall, a piston,
  `/setblock ... destroy`), nor a creative break or a tool that can't harvest the block. A block's stacks get its block entity's data (a
  shulker box's items) after the hook, and a container's contents spill on their own, not through it. A player's use drops
  through it too, with `used` set and that player named: shears on a sheep, a mooshroom, a snow golem or a bogged (its shearing
  table), a brush on an armadillo (`minecraft:brush/armadillo`) or a suspicious block (its item, no table), shears carving a
  pumpkin, picking sweet berries or cave vines (their harvest table), and a player's hit taking an item frame or its item (the
  frame's entity, no table). A dispenser's shears and an explosion's frames aren't told. Leads: untying one or cutting it with
  shears is a use (the leashed mob, no table), and a mob a player killed while leashed drops its lead on its next tick as a call
  of its own, naming the killer (`used` false, no table). A lead that snaps or loses its holder names no player and isn't told.
- **`native_living_drops(entity, kind, player, source, recently_hit, drops, cancel)`**: NeoForge's
  `LivingDropsEvent`, the whole of what a dying mob, server player or armor stand drops, after `loot_drops` and with the
  rest of `dropAllDeathLoot` in: an enderman's carried block, a copper golem's preserved equipment, and a player's inventory
  (unless `keepInventory`). Each `ItemDrop` holds the stack, position, motion and pickup delay its item entity gets, and
  its thrower, target (who alone may pick it up), no-gravity and starting age; an unedited drop spawns unchanged. `source` is a `HurtSource` (the damage type,
  `causing` and `direct` entity ids); the id of a projectile isn't kept, so `direct` is 0 for it. Edit the list or set `cancel`
  for no drops; experience orbs are separate (`living_experience_drop`). An armor stand fires it from `brokenByAnything`
  with an empty list, and its stand item and equipment pop outside the list, as in vanilla. `loot_drops` stays for loot that
  is not a death (blocks, shearing, leads); for a mob's death it is heard first (it edits the stacks a table rolled), and this hook
  then fires once on the whole collected list, with nothing spawned until it returns.
- **`native_get_equipment_drop_chance(entity, kind, slot, source, recently_hit, stack, original_chance, preserved, chance)`**:
  NeoForge's `GetEquipmentDropChanceEvent`. Change the chance a dying mob rolls for the item `stack` in `slot`; `original_chance`
  is the recorded chance before enchantments and `preserved` says the slot drops without a player kill. Each slot is asked at its
  own point of `Mob.dropCustomDeathLoot`, just before its roll, and the roll is drawn whatever the chance is.
- **`enchanted_entity_loot(entity, source, attacker, enchantment, level)`**:
  NeoForge's `EnchantedEntityLootEvent`. Change the level an entity's loot table uses of `enchantment`, in the middle of the roll:
  `EnchantedCountIncreaseFunction` and `LootItemRandomChanceWithEnchantedBonusCondition` ask each time they are evaluated, on a death, on
  `EntityRef::roll_death_loot` and on `/loot ... kill`, whose `source` is `minecraft:magic` and whose `attacker` is the command's source. `entity` and `attacker` are live handles;
  `attacker` is None without one and may not be living, and `level` then starts at 0, otherwise at the living attacker's level. It is unsigned, so
  the negative levels `setEnchantmentLevel` throws on can't be written, and above 1024 it counts as 1024, so a mod's value can't size a loop. Without a listener the roll asks nothing, and its random's order is vanilla's.

### A mod block entity's data on its item

A mod's block entity carries its saved data on the item as vanilla's do: Ctrl + Pick Block, a loot function that copies the block entity (`minecraft:copy_components`-style) or a `/give` with `minecraft:block_entity_data:{id:"mod:type",...}` puts the compound (without `x`, `y`, `z`) in the item's `minecraft:block_entity_data`, and placing the item merges it into the new block entity before `native_place_block` runs, the counterpart of NeoForge's `BlockItem.updateCustomBlockEntityTag` and `BlockEntity.saveToItem`. Data tagged with another type is ignored. `/give` and `item replace` accept a type id a mod declared (its content's block entity types) and refuse any other as vanilla refuses an unknown one. The data is the compound the block entity saves, without its position and its components (`saveCustomOnly`): a component patch, a custom name say, rides as the item's own components and comes back as the block entity's on placing. A mod reads the data back in its own load as it does for a chunk.

### A block's items

A server mod reaches a block's items through its `ITEMS` capability ([parts](../concepts/parts.md)): `level.capability(host, &ITEMS, pos, side)` gives the block entity's slots as that side reaches them, by the rules hoppers follow (a furnace's sides, a double chest's two halves, a mod's `ModContainer` with its `faces`, `inputs` and `outputs`), and `resource_util::move_resources(host, &from, &to, filter, max, Some(tx))` moves items between two of them in a transaction. `side` is the side of the block being reached, so a pipe above a chest uses `Some(Direction::Up)`. A level not in place, a stale `LevelRef` and a client's copy of the world give none. `native_dimension()` names the level in place (`minecraft:overworld`) for a hook that isn't told it, such as `block_entity_tick`.

During `block_entity_tick`, use `event.subject.with_view` to read or edit the machine's own item slots in place. Return the view before borrowing the Host to reach another block. A composter answers `ITEMS` by face, with one slot and no block entity (NeoForge's `ComposterWrapper`): its top takes a compostable item while its level is below 7, its bottom gives the bone meal at level 8, and its other sides and `None` give none. Container entities (minecarts, chest boats) aren't reached this way yet. A pipe keeps a neighbour's capability with `BlockCapabilityCache` instead of looking it up every tick ([keeping a block capability](../concepts/capabilities/keeping-a-block-capability.md)). A provider whose answer changes with no block change (a machine's side configuration) calls `level.invalidate_capabilities(host, pos)` (NeoForge's `invalidateCapabilities`): the capability generation there moves on, at the root commit inside a transaction and not at all when it aborts, and every cache there looks up again at its next `get`. `level.capability_generation(host, pos)` reads it; only its change means anything.

### Entities

`crates/sunrise-client/src/game/modding/native_entities.rs`: helpers for server mods on open server levels.

- `LevelRef::summon(host, kind, pos, nbt)` spawns an entity of `kind` from its compound in `level`, as `/summon <kind> <pos> <nbt>`
  does (passengers included), routing parked dimensions and returning `Result<Option<EntityRef>, LevelWriteError>`. Returns `Ok(None)` outside `/summon`'s bounds, for an
  unloaded chunk, an unloadable kind, or a mod's `entity_join_level` veto (in a transaction its handle goes stale), and `Err(LevelWriteError)` on a client, for a stale level, or when summoning into a parked level inside an open transaction.

A model that moves (a turning gun barrel, a thrown projectile) is an `item_display` showing an item whose model comes from
the mod's `.smod` assets: summon it once, then move it and pose it with `DisplayRef` from a tick hook. Easing between ticks happens on each
client, so a call a tick looks smooth.

### Entity and level handles

`crates/sunrise-client/src/game/modding/native_entity_refs.rs` (#7387): an `EntityRef` names one entity of the open world and a
`LevelRef` one level. They are small `Copy` values a mod keeps, and their methods take the host, so a mod can hold
several at once: `host.entity(id)` makes one from the id an event carries, `host.level("minecraft:the_nether")` one from
a dimension.

An entity held out of storage while it is changed refuses handle reads and writes: reads return `None` and writes return
`false`. This includes cart and ridden-cart ticks, boat ticks (including ridden-boat leash ticks), and a copper golem turning into a
statue. A block entity also refuses its handle while it is being ticked. Tick hooks before and after that work can use
the handle normally.

The client also owns a world epoch, separate from the server's. It begins when a play login creates the
client level, changes when a dimension change or later login replaces that level, and retires after the
client logout hook or when reconfiguration clears the level. A same-level respawn keeps the epoch.
Client epochs do not fire server world lifecycle or server tick hooks. This bookkeeping does not make
the server entity and level reads available on a client; the reads listed under "On a client" below are.

On a client (`host.local_player()`, a client hook's `host`), these reads answer from the client's own copy, which is
what the server sent it (vanilla's `LocalPlayer`, `ClientLevel` and `Entity` fields), and are `None` for a stale
handle, an entity the client doesn't track and a level it doesn't show:

- `host.entity(id)` and `host.level(dimension)`: the local player or an entity the server told the client of, and the
  one level the client shows. `host.living(entity)` is the local player only.
- `entity.position(host)`, `rotation(host)` and `velocity(host)`: the client's copy, interpolated as it draws it.
- `entity.name(host)`: the synced custom name or type name, and a player's profile name, from either side's host.
- `level.block_state(host, pos)`, `block`, `block_property` and `is_loaded`: blocks of the chunks the client holds.
  `block_property` borrows the property value from the host.
  A chunk it doesn't hold reads as unloaded; no read loads one.
- `level.day_time(host)`: the synchronized Overworld clock in ticks (counting past 24000), advanced at the
  server-supplied rate while the client level runs. This remains the Overworld clock in the End, even though
  the sky uses the End's own default clock. Only a handle for the level the client shows can read it.
- `living.with_equipment(host, slot, read)`, `with_hand_item`, `effect`, `effects`, `potion_affected` (`hasEffect`), `health` and `max_health`, for the local player.
  Another entity's equipment, effects and health stay server reads, and so does every write.
- `player.last_death_location(host)`, for the local player: the position the login and respawn packets carried.
- `player.with_inventory(host, read)` lends a read-only inventory view in vanilla slot order (0 to 35 main, 36 to 42 equipment).
  Use `view.get(n)` to read slot `n`, or `view.iter().take_while(|(slot, _)| *slot < 36)` for just the main inventory.
  Read item, count and damage inside `read`. Use `stack.project()` only for components or to retain an owned stack;
  a failed projection does not mean the slot was empty. An unavailable player returns `None` without calling `read`;
  on a client only the local player's inventory is available.
- For the local player, its own state as the server sent it (`LocalPlayer`): `player.game_mode(host)`, `is_creative`,
  `is_spectator` and `may_build` (the game mode of the login, respawn and `ClientboundGameEvent` packets); `food`
  (level and saturation from `ClientboundSetHealth`; the client keeps no exhaustion, so it reads 0), `needs_food`,
  `has_enough_food` and `can_eat`; `abilities` (what the game mode gives, then `ClientboundPlayerAbilities`' flags and
  speeds); `selected_slot` (the hotbar slot held); `experience` (`ClientboundSetExperience`) and
  `xp_needed_for_next_level`. Another player reads `None` there: the client is not sent their state.
- The local player's open menu: `host.local_player()` then `player.container_menu(host)` makes a `MenuRef` for the menu
  the client has open (the player's own inventory when none is), and `menu.slot_count(host)`, `slot(host, index)` and
  `carried(host)` read it as the server's `ClientboundContainerSetSlot` and `SetCursorItem` packets left it. Indexes are
  the menu's own slot order (the same `slot_backing` names), not network slot ids. Another player's menu is `None`, and
  so are `container_id` and `kind` on a client (server reads). A slot reads `Some(None)` when empty and the outer `None`
  when it doesn't exist or the menu is unavailable; a menu type the client has no slot map for reads `None` throughout
  rather than guessed contents. A handle goes stale, and reads `None`, when its menu is closed or replaced (a reused
  window id included), on a respawn, a dimension change or a logout, and after a reconnect. Nothing here sends a packet
  or changes the menu.

- `entity.position(host)`, `block_position`, `rotation` (yaw and pitch), `kind` (`minecraft:zombie`), `uuid` (`&mut`:
  an entity with no UUID yet gets its own) and `level` read the entity.
- `level.block_state(host, pos)` returns a typed `BlockState` without allocating a state string. For text such as
  `minecraft:oak_stairs[facing=east,…]`, pass the ID to `host.block_state_string(id)`. `block` (the name alone),
  `block_property(host, pos, key)`, `is_loaded`, `height` (lowest `y` and height), `game_time` and `day_time` read any
  level the server has loaded, not only the one in place. A block in a chunk that isn't loaded or a `y` outside
  the world's height reads as `None`. `level.brightness(host, LightLayer::Block | Sky, pos)` reads the stored light
  (`getBrightness`) the same way, as the game's own light query holds it, and is `None` where the block read is.
  `level.height(host)` (`LevelHeightAccessor.getMinY`, `getHeight`) also works with `&dyn Host` in client
  hooks, including render, screen and mouse hooks. Obtain the handle with `host.level(dimension)`; on a
  client only its current dimension is available, after login. It reads that client's synchronized level
  shape, without loading chunks or allocating. A stale handle, logout or a different dimension returns
  `None`, not guessed Overworld bounds. On a server it still reads parked loaded levels as well.
  `level.fluid_state(host, pos)` reads the fluid there the same way.
  `level.fluid_flow(host, pos)` (`FluidState.getFlow`) reads its current as a `Vec3`, through the function that pushes
  entities, so a `fluid_flow` listener's change is what it reads: zero where no fluid flows, `None` where the block read
  is. It reads on a client too, from the client's own copy of the level. It costs what one entity's push by that block
  does: up to twelve block reads around `pos` (the four neighbours and below them; falling fluid also checks
  their faces), with no allocation, plus the `fluid_flow` listeners.
- `level.weather(host)` reads rain and thunder levels on **both sides**, using the shared `Host` from server,
  render, screen or mouse hooks. It returns `Some((rain, thunder))`, each 0.0 to 1.0, at partial tick 1.0
  (`Level.getRainLevel` / `getThunderLevel`, with thunder scaled by rain). Both are 0 where the level cannot have
  weather. A server can read any open level, including parked levels; a client reads only its current synchronized
  level after login. Stale handles and unavailable levels return `None`; the read never loads a level.
- `level.seed(host)`, `difficulty` (`"normal"`, as `/difficulty` names it), `game_rule(host, "keep_inventory")` (a
  `GameRuleValue`, `Bool` or `Int`; `Ok(None)` for an unknown rule), and `visit_game_rules` (every available rule as a
  borrowed namespaced id and its value) read server settings. `level.is_raining(host)` and `level.is_thundering(host)` read
  the named server level's own strengths, not the shared flags, even for a parked level.
  `level.is_raining_at(host, pos)` (`Level.isRainingAt`) asks about one position of any open level: it rains there, the
  position has full sky light with nothing motion-blocking or fluid above it, and its biome's precipitation at that
  height is rain (not snow or none). It is `None` where the block read is, an unloaded chunk or a `y` outside the
  world. `level.enabled_features(host)` is the world's `FeatureFlags` (`LevelReader.enabledFeatures`:
  `contains("minecraft:redstone_experiments")`, `iter()`); every level of a world shares them. If the save
  omits its flag list, the world enables `minecraft:vanilla`.
- `level.chunk_ref(host, [x, z])` makes a `ChunkRef` for any chunk of an open level (None past 30 million blocks).
  `chunk.forced(host)` and `level.visit_forced_chunks(host, &mut visit)` query `/forceload` tickets.
  `chunk.set_forced(host, bool)` adds or removes one and returns whether the saved set changed. A successful add waits for the chunk to load, even in a
  parked level. If streaming is paused, the add records the ticket to take effect when its level starts streaming.
  Adds after generation stops return false; repeated adds and removals do not load chunks.
- Ticket controllers (NeoForge's `TicketController`, registered in `RegisterTicketControllersEvent`): a mod registers
  each one once at startup, from `Hooks::register`, with `r.register_ticket_controller("quarry", validator)` and keeps the
  `TicketController` it returns (it is `Copy`; `Hooks` methods take `&self`, so a `OnceLock` holds it). A bare name uses
  the calling mod's namespace (`quarry` becomes `mymod:quarry`); an explicit valid `namespace:path` keeps its
  namespace, with attribution to the calling mod. A malformed or duplicate ID refuses startup (naming the mod and ID).
  Pass `Some(Arc::new(callback))` to bind a `LoadingValidationCallback` to this controller, or `None` to restore its
  tickets without validation. The handle is only made by the registrar with an opaque registration identity, so a name
  or slot alone cannot forge one and a ticket call looks up no string. The registry-issued handle itself authorizes
  active chunk-ticket operations without consulting the current caller; an invalid handle from a refused registration
  or another registration table does nothing.
- `chunk.set_mod_forced(host, controller, owner, forced, natural_spawning)` (NeoForge's `TicketController.forceChunk`)
  holds or releases a chunk through `controller` and returns whether its tickets changed. `owner` is a
  `TicketOwner`, `Block([x, y, z])` or `Entity(uuid)`. A ticket is scoped by controller, owner, chunk and
  `natural_spawning`, so two owners in one chunk release separately, one owner holds a plain and a natural-spawning
  ticket on a chunk apart, and two controllers of one mod never touch each other's. The chunk stays loaded, entity
  ticking and saved, and its level keeps ticking, while any ticket or `/forceload` holds it. With `natural_spawning`, the
  chunk joins the spawning chunks (`ChunkMap.collectSpawningChunks`, within 8 chunks of
  a player): it gets inhabited time, thunder and a spawn attempt. The local mob cap then lets mobs spawn there only
  while a non-spectator player is within 128 blocks of the chunk's centre, as vanilla's
  `LocalMobCapCalculator` decides. `level.visit_mod_forced_chunks(host, controller, &mut visit)` visits the controller's tickets,
  passing `owner`, `[x, z]` and `natural_spawning` to the visitor. Tickets are saved in `data/sunrise/mod_chunk_tickets.dat`, never in vanilla's
  `chunk_tickets.dat`, under the controller's id. When a world loads, the tickets of an id no loaded mod registers are
  dropped with a warning in the log, as NeoForge documents for a controller nobody registers; so are tickets from a
  file written before controllers, which are keyed by mod id. For an invalid controller handle (such as one from a
  refused registration or another registration table), a stale level handle, a client, or a chunk past the world's
  bounds, a write returns `false`. A visit returns `LevelReadError::Stale` for a stale level or
  `LevelReadError::Unavailable` for an invalid controller, unavailable side or backend.
- A controller's `LoadingValidationCallback` (NeoForge's `LoadingValidationCallback.validateTickets`) runs once after
  `level_load` and before the level's first tick, only if that controller has saved tickets in the level. At startup all
  built-in levels are available before validation begins. It receives
  the server Host, the opened `LevelRef` (even if parked), and a `TicketValidation` containing only that controller's
  saved tickets (NeoForge's `TicketHelper`). Tickets added in `level_load` are not staged for validation. A panic logs
  and queues an error toast naming the mod, discards that callback's removals, and disables only that controller's
  validator for the session. After all validators finish, surviving tickets force their chunks, including tickets
  without an enabled validator.
  `block_tickets()` and `entity_tickets()` lend read-only `BTreeMap` views of the controller's original saved tickets
  (`&BTreeMap<[i32; 3], TicketSet>` and `&BTreeMap<u128, TicketSet>`), with each owner's chunks apart by kind
  (`non_natural_spawning` and `natural_spawning`, as NeoForge's `TicketSet`). As in NeoForge's
  `TicketHelper.getBlockTickets`/`getEntityTickets`, those maps stay unchanged after removals during the callback, while
  subsequent removals skip rows already selected. A plain and a natural-spawning ticket on one chunk are two
  tickets. `remove_block_tickets(pos)` and `remove_entity_tickets(uuid)` remove everything an owner holds, both kinds
  (and return how many); `remove_block_ticket(pos, chunk, natural_spawning)` and
  `remove_entity_ticket(uuid, chunk, natural_spawning)` remove the one kind they name and leave the other. A removed
  ticket never forces its chunk and is left out of the next save.
- `host.living(entity)` gives a `LivingRef` for a mob, a player, an armor stand or the dragon (vanilla's
  `LivingEntity`); `living.entity()` goes back. `living.with_equipment(host, EquipmentSlot::Head, read)` and
  `living.with_hand_item(host, Hand::Main, read)` lend a single stack. An empty or unavailable slot returns `None`
  without running `read`. Use `stack.project()` in the callback only for components or to keep an owned copy.
  `living.with_weapon_item(host, read)` lends the living attacker's weapon on the server (the active Riptide stack
  when present, otherwise its main hand). `set_equipment(host, slot, stack)` puts a stack in a slot as
  `LivingEntity.setItemSlot` does (None or air empties it), and `set_hand_item(host, hand, stack)` puts one in a hand
  as `LivingEntity.setItemInHand` does (the main hand's or the off hand's slot, written as `set_equipment` writes it).
  Mobs store all eight slots; players (the host and guests) keep body and saddle beside their hand and armor fields. A player's
  `body` and `saddle` are kept and saved in the `equipment` compound as vanilla does, and dropped on death, but nothing
  renders or sends them. Armor stands and the dragon hold all eight slots in the same `Equipment`, saved in `equipment`
  (omitted when empty). A stand keeps a body or saddle a mod writes (`setItemSlot` stores it; `ArmorStand.canUseSlot`
  only stops a player equipping it), and pops it with the rest, after the other six slots, when it breaks. A write to a
  stand or the dragon runs `LivingEntity.onEquipItem` like a mob's: the equip sound and the `equip` or `unequip` game
  event, unless the entity has not ticked yet or the item did not change. `living_equipment_change` hears all
  eight slots of mobs, armor stands and connected players. `/item replace` keeps vanilla's predicate for a stand's or the dragon's chest, legs, feet, body
  and saddle (an empty stack, or an item made for that slot; a stand's `canUseSlot` refuses body and saddle), while
  `set_equipment` stays permissive. This is not full dragon equipment parity: the dragon keeps its equipment for mods and
  saves, and does not drop it on death (the death drops are built on `Mob`, which the dragon is not).
- `living.swing(host, hand, send_to_self)` (`LivingEntity.swing`) starts an arm swing, shortened by haste and lengthened
  by mining fatigue, unless one is under way and past its first half (it returns whether one started). The players around
  see it at the end of the tick, the swinger too with `send_to_self`; clients see a mob swing the hand asked for. Mobs and
  players work; armor stands and the dragon return false, and so does a player other than the acting one while another
  player acts.
- `living.arrow_count(host)` / `set_arrow_count` and `stinger_count` / `set_stinger_count` (`DATA_ARROW_COUNT_ID`,
  `DATA_STINGER_COUNT_ID`) read and set what is stuck in a player or a mob; the entity data carries it to the players
  around, so they see it. Armor stands and the dragon keep no such count (None, false). `set_arrow_removal_ticks` and
  `set_stinger_removal_ticks` set the ticks until the next one falls out (`removeArrowTime`, `removeStingerTime`; at 0 or
  less the timer restarts at `20 * (30 - count)`); a mob's timers run in its tick as a player's do.
- `living.item_use_remaining(host)` / `set_item_use_remaining(host, ticks)` (`useItemRemaining`) read and move the end of
  the item a player uses: at 1 the use completes on the next tick, at 0 or less it lasts until the player lets go, as in
  vanilla. It moves the deadline the use-item Tick hook moves, so shield, bow, trident, spear, instrument and bundle uses,
  which don't count it, ignore it, and a call inside that Tick hook is overwritten by the hook's result. Nothing in use reads 0 and refuses a write. A mob's use never runs out: its read is the item's use duration less the ticks its use counter has run, and a write moves that counter to where `ticks` are left, which the stabs of a spear and the draw of a bow follow. A piglin admiring gold and a drinking witch use no item, so they read 0 and refuse a write.
- `living.set_riptiding(host, ticks, damage)` (`startAutoSpinAttack`) starts a player's Riptide spin: for `ticks` ticks
  the player turns, the first mob its box touches takes `damage` and ends the spin, and the players around see it
  (living flag 4). It adds no velocity (a Riptide launch does that itself), and a `ticks` of 0 or less ends the spin.
  Mobs, armor stands and the dragon return false: Sunrise runs no spin for them.
- `living.set_waypoint_style(host, &style)` and `set_waypoint_color(host, rgb)` (`Waypoint.Icon.style`, `.color`) change
  the locator bar icon of a living entity as `/waypoint modify` does, untracking and tracking its waypoint again, so the
  players it transmits to get the new icon. A colour of `None` shows a player in their team's colour. Only a player's
  waypoint reaches other players (`waypoint_transmit_range` is 0 for every other entity); a mob's or an armor stand's
  icon is kept for its waypoint and `/waypoint list`. The dragon returns false.
- `living.used_item_hand(host)` (`getUsedItemHand` while `isUsingItem`) reads the hand of the item a player or a mob uses,
  None when it uses none; on a client only the local player answers. `living.start_using_item(host, hand)` is
  `LivingEntity.startUsingItem`: the item's use timer and the use-item Start hook, not the item's own `use` (no ammo check,
  no sound). A player's client lets go of a use whose button isn't down, as in vanilla, so that use is released on its next
  tick. `living.release_using_item(host)` is `releaseUsingItem`, what letting go of the button runs: the use-item Stop
  hook, the item's own release (a player's bow or trident shoots), a mod's `release_using`, once. A mob's use is its goal's
  counter: it shows and its goals see it, but it never runs out, so only a release or a goal ends it, and vanilla's bow and
  trident have no release for a mob. A piglin with something in its off hand and a drinking witch refuse a start and have no
  use to read or release: the counter is their own timer.
- `living.visit_effects(host, &mut visit)` visits a living entity's active effects (vanilla's `activeEffects`) as `ActiveEffect`s — the
  load-owned `EffectKey` (borrow its canonical name, if available, with `key.id(host)`), the ticks left (-1 infinite),
  the amplifier, and the `ambient`, `visible` and `show_icon` flags (`show_icon` is vanilla's `showIcon`, apart from the particles). `living.effect(host, key)` reads one (`getEffect`) and `living.potion_affected(host,
  key)` answers `hasEffect`; request the key once through `Registrar::effect_key`. `living.add_effect(host, &effect)` is
  `LivingEntity.addEffect` by the game's own path: a player's or a guest's goes through the potion machinery (its attribute
  modifiers, absorption, and the added-effect hooks for a content mod's effect), a mob's past `canBeAffected` (a spider
  shrugs off poison) and with the same hooks. It returns true when accepted — whether it replaced one already there or only
  extended it is `MobEffectInstance.update`'s business (a stronger one replaces with its own duration, an equal one extends,
  a weaker one changes nothing). `living.remove_effect(host, key)` and `living.remove_all_effects(host)` take effects away
  the way their running out does: the attribute modifiers they gave recompute and absorption's hearts go with theirs. The
  added-effect hooks hear a player's add without an entity id, so a guest's reaches them as the acting player (`None`) — a
  limitation of the hooks themselves, unchanged here. Armor stands and the dragon keep no effects, as `/effect` skips them:
  `visit_effects` returns `Err(EntityReadError::Unavailable)`, `effect` returns `None`, and their writes return false.
  The visitor may stop early by returning `ControlFlow::Break(())`. On a client,
  `living.effect_blend_factor(host, key, partial)` (`getEffectBlendFactor`) reads how far the local player's effect has
  faded in at the partial tick: darkness fades over 22 ticks and nausea in over 150 and out over 20. Other effects start
  at 0 after a blended addition and interpolate to their post-countdown target each client tick (1 while active, 0 once
  expired). An unblended addition starts at its target immediately; a replacement packet preserves the old blend.
  Partial ticks are not clamped. A missing effect reads 0. The client keeps no other entity's effects, so they read `None`.
- `host.fishing_hook(entity)` gives a `FishingHookRef` for a cast hook (`FishingHook`); `hook.entity()` goes back.
  `player_owner(host)` is the player who cast it, `hooked_in(host)` what it holds (None when nothing, or when that is
  gone) and `open_water(host)` is `isOpenWaterFishing`. `item_fished` exposes mutable catch drops, cancel and absolute
  rod damage, and names a hook by id. A server box query with `LevelRef::visit_entities_in` can find fishing hooks.
  Discarding one takes it out. It is not a `ProjectileRef`, and selectors do not find it yet.
- `host.projectile(entity)` gives a `ProjectileRef` for a projectile in the level's projectile list (arrows, a mob's
  trident, thrown items, fireballs, wind charges, skulls, shulker bullets, mods' thrown types), a trident a player
  threw or a firework rocket; `projectile.entity()` goes back. A player's trident and a rocket have an owner and a
  deflection only: `pickup`, `base_damage` and `critical` read None and their setters return false, and a player's
  trident always has a player owner (`set_owner` or `deflect` with none or a mob returns false). `owner(host)` resolves the owner to a current `EntityRef` (None when gone
  or in another level) and `set_owner(host, owner)` takes any entity but a fishing hook (`Projectile.setOwner`). An owner that is neither a
  player nor a mob is kept by its UUID: `owner` resolves it and a save keeps it, but a hit credits no one for it. `deflect(host, deflection, by, new_owner, by_attack, power)` is `Projectile.deflect`: `Deflection::Reverse`
  draws one float for the turn from the projectile's own random (`Entity.random`, never the level's), `AimDeflect`
  and `MomentumDeflect` use the look or the known speed (last tick's move, `Entity.getKnownSpeed`) of `by` and do
  nothing without it; the new owner is set and a fireball's or wind charge's acceleration power becomes 0.1 by an
  attack, else halves. A wind charge refuses for its first five ticks (`WindCharge.noDeflectTicks`): false, with no
  change and no draw. `set_owner` on an arrow also allows pickup for a player owner (`AbstractArrow.setOwner`);
  `deflect` does not (vanilla's `EntityReference` overload). Arrows (and a mob's trident for
  `pickup`) also have `pickup`/`set_pickup` (`ArrowPickup`), `base_damage`/`set_base_damage` and
  `critical`/`set_critical` and `pierce_level`/`set_pierce_level`. The pierce level is a signed byte:
  vanilla stores all values from -128 through 127 unchanged, and nonpositive values do not pierce. The
  typed result distinguishes a stale handle, a non-arrow projectile and a wrong-side host. The critical
  flag reaches clients through entity data; these handle methods operate on the server.
- `living.drop(host, stack, throw_randomly, include_thrower_name)` throws a stack the mod holds from the eyes
  (`LivingEntity.drop`) and gives the item entity; it never touches a slot. `is_tamed(host)` and `owner(host)` read a
  tamable mob or a horse (`TamableAnimal.isTame`, its owner's UUID).
- `entity.velocity(host)` (a player's is the move its client last reported), `set_velocity(host, v)` (for a player it
  replaces the server's motion and sends that player's own client the motion packet), `on_ground`, `bounding_box` (lowest and highest corner), `eye_height`, `eye_position`,
  `view_vector` and `distance_to_sqr(host, pos)` read and move it.
- `entity.up_vector(host)` is `Entity.getUpVector`, the view vector turned toward the zenith. `on_block(host)` is
  `Entity.getOnPos`: the supporting block of an entity on the ground (the solid block under its box nearest it),
  else the block under its feet. `pick(host, distance, include_fluids)` clips `Entity.pick`'s ray from the eyes
  along the view against block outlines and optional fluids (`ClipFluid::Any`/`None`) and returns the `ClipHit`
  (`miss` set for a miss); `LevelRef::ray_trace_nearest_entity` and `ray_trace_entities` clip entities.
  `is_multipart(host)` and `visit_parts(host, &mut visitor)` follow NeoForge's `IEntityExtension.isMultipartEntity`
  and `getParts`. The visitor can break early. It visits the ender dragon's eight `EnderDragonPart`s (not the Parts
  of this API), head, neck, body, three tail segments and
  two wings, each an `EntityRef` of its own entity id, resolved as any handle is. A part's `kind` is the dragon's, its position, rotation (none: zero), velocity (zero)
  and `bounding_box` are its own, `level` is the dragon's, and it is no living entity, mob or dragon handle; it
  stops resolving when its dragon does, and removing one is refused.
- `entity.nbt(host)` and `merge_nbt(host, compound)` read and change its data as `/data get` and `/data merge entity`
  do (merging reloads mobs, dropped items, armor stands, item frames, arrows, spectral arrows and tridents; never players);
  `tags`, `add_tag`, `remove_tag` work as `/tag` does, and `name` is its name as plain text.
  `set_custom_name(host, Some(text))` names it and `None` takes the name away (`Entity.setCustomName`);
  `custom_name_visible` and `set_custom_name_visible` read and change whether the name shows at any distance. Both
  writes are saved and reach the players tracking a mob or an armor stand; they return false for a player and for the
  kinds whose name Sunrise does not retain (projectiles, primed TNT, falling blocks, experience orbs, ominous item
  spawners, the dragon and crystals).
  `classification` is the type's `MobCategory`, and `added_to_level` is true from when `entity_join_level` let the
  entity in until it leaves or is removed.
- `entity.play_sound(host, sound, volume, pitch)` (`Entity.playSound`: at the entity, under its `sound_source`, for the
  players in range; a silent mob plays nothing and a player doesn't hear their own), `game_event(host, event)`
  (`Entity.gameEvent`: the entity is the source of a vibration for sculk sensors, wardens and allays),
  `broadcast_event(host, id)` (`Level.broadcastEntityEvent`: entity event `id` to the players tracking it and to
  itself, for `handleEntityEvent` to act on), `sound_source(host)` (`Entity.getSoundSource`, a `SoundSource`) and
  `spectator(host)` (`Entity.isSpectator`) work for every kind. `game_event` and `broadcast_event` act on the level in
  place and return false for a player in a parked level. There is no `sprint_particles`: vanilla spawns those on the
  client only.
- `player.stop_sound(host, sound, source)` sends `StopSoundCommand`'s packet to that player (`None` for any sound or
  source), `player.play_sound(host, sound, source, volume, pitch)` plays a sound for them only at their position, and
  `entity.play_bound_sound(host, sound, source, volume, pitch)` plays one that follows the entity for the players in
  range, as `Level.playSound(null, entity, ...)` does. `/stopsound` and `/playsound` send through the same code. They
  return false for a stale handle or a player who isn't online. Looping, fading and a handle to a playing sound need
  client handles.
- `living.health(host)` is `LivingEntity.getHealth` (Paper's `Damageable#getHealth`), `max_health` is
  `getMaxHealth` (players and guests read their attribute map; an armor stand the fixed 20 the game seeds it
  with — lava does not hurt it, and splash water reads the very field — and the dragon its 200, from their own
  fields until live attribute maps cover them) and `alive` is `isAlive`
  (Paper's `Entity#isDead` is its inverse; a dying mob, mid death animation, is not alive). During a nested
  player swap an outer player's displaced record is unavailable, and during a player's turn the other
  players' handles read `None` until the hub is back.
- `living.attribute_value(host, id)` is `AttributeMap.getValue` and `attribute_base` is `getBaseValue`, for a
  player, a guest, a mob, an armor stand and the dragon (`None` for an attribute the entity type lacks or a stale
  handle). A mob, a stand and the dragon answer from a live attribute map built from the type's
  defaults, the saved `attributes` list (bases and permanent modifiers, with their ids), and the attribute modifiers
  of the items they hold and wear, put on and taken off slot by slot in `EquipmentSlot` order as
  `LivingEntity.collectEquipmentChanges` does (transient: never saved). The map is built when first needed. Mob
  behavior still reads the type's defaults, so a modifier does not yet change how a mob moves or hits. An item's
  modifiers are the stack's own `attribute_modifiers` component or its item type's, as `item_attribute_modifier`
  leaves them. On a client the reads answer for the local player only, from the client's own copy of the synced
  instances (`ClientboundUpdateAttributes`, the type's defaults before the server sends any); every other entity
  reads `None` there, the client keeping no attribute map of another entity's, only the few fields it draws
  (`handleUpdateAttributes` for a mob extracts its maximum health and scale for the renderer).
- `living.visit_attributes(host, &mut visit)` lends ids from the holder's attribute set, in registry order.
  `living.visit_attribute_modifiers(host, id, &mut visit)` is `AttributeInstance.getModifiers`: it lends an
  `AttributeModifierView` for each modifier in addition order, including saved, transient, permanent and equipment
  modifiers. Return `ControlFlow::Break(())` to stop a visit; copy an id in the visitor if it must outlive the call.
  An attribute with no modifiers completes without any calls. Unknown or unsupported attributes are unavailable,
  as are reads of other entities on a client; stale handles return `EntityReadError::Stale`.
- The writes take players, guests (in their own turn), mobs and armor stands, and answer false for an attribute the
  type lacks, the dragon (`/attribute` refuses it, and nothing reads its attributes yet), a stale handle, or on a
  client. `living.add_attribute_modifier(host, id, modifier, lifetime)` is `AttributeInstance.addTransientModifier`
  or `addPermanentModifier` (`ModifierLifetime`), refused when the attribute already has a modifier with that id
  (vanilla throws) or the amount is not finite. `replace_attribute_modifier` puts the new one in place of the old
  (`addOrUpdateTransientModifier`, `addOrReplacePermanentModifier`), and the new lifetime wins.
  `remove_attribute_modifier(host, id, modifier_id)` is `removeModifier` (false when there was none), and
  `set_attribute_base(host, id, base)` is `setBaseValue`. `AttributeModifier` holds the modifier's id, amount and
  `AttributeOperation`. A permanent modifier and a base are saved with the entity; a transient modifier lasts until it
  unloads. The writes take the one path `/attribute` takes, so they go where the game and the attribute sync read:
  - a player's own attribute map;
  - for a mob, the store of the nine attributes its behavior uses (max health, movement and flying speed, follow
    range, attack damage, knockback resistance, jump strength, burning time, reinforcements), so it moves and hits by
    them at once; its armor map for armor and toughness, which its damage reads; and for its other attributes, as
    for an armor stand, the generic map `/attribute` reads and the sync sends.

  The live map behind `attribute_value` takes the same write, so the reads agree with `/attribute`, and the entity's
  save keeps it. A lower max health cuts a mob's health (`LivingEntity.onAttributeUpdated`).
- `item_attribute_modifier` (`ItemAttributeModifierEvent`) edits what one stack grants: `ItemAttributeModifiers`
  holds `ItemAttributeModifier` entries (the attribute's id, an `AttributeModifier`, an `EquipmentSlotGroup`) and
  edits them as NeoForge's event does: `add` (`addModifier`, refused when the attribute has that id already),
  `replace`, `remove`, `retain` (`removeIf`), `remove_all_for` and `clear`. Every reader of a stack's modifiers asks
  through one place (`sunrise_client::item_attributes`):
  - the equipment of a player, a mob, an armor stand and the dragon, when it changes;
  - mobs' gear knockback resistance and their pickup choices;
  - the attribute sync;
  - both tooltips.

  A worn stack is asked again only when it changes (`ItemStack.matches`), so the answer must be the same for the
  same stack. With no mod listening, or none changing a stack's list, nothing is copied. A player's attack speed
  still reads its held item type's `attack_speed`, not the stack's (see #549).
- `get_enchantment_level` (`GetEnchantmentLevelEvent`) edits the enchantments gameplay sees on a stack:
  `ItemEnchantments` is NeoForge's `ItemEnchantments.Mutable` (`level`, `set`, `upgrade`, `retain` for `removeIf`,
  levels at most 255), and `target` is the one enchantment a read asks for, `None` when every one is read. The
  game's reads follow NeoForge's split:
  - through the hook: `ItemStack::enchantment_level` (`getEnchantmentLevel`, every gameplay level read: Looting,
    Fortune and the other loot functions, Respiration, Silk Touch, Mending, Thorns, the trident's and crossbow's
    enchantments, fishing), and `ItemStack::all_enchantments` (`getAllEnchantments`: every enchantment effect
    component, the enchantments' attribute modifiers, the decorated pot's `hasTag` check, the killer's gear for
    entity loot);
  - the stack's own component: `ItemStack::enchantments` and `tag_enchantment_level` (`getTagEnchantmentLevel`)
    for tooltips, the anvil, grindstone and enchanting table, item predicates, saves, the network, and
    `EnchantmentHelper.has` (Curse of Binding), which this hook doesn't change.

  It is asked on every such read, many times a tick, so keep a listener cheap. With no mod listening nothing runs;
  with one, ids are parsed once and a read the mods leave alone allocates nothing.
- `living.set_health(host, health)` is `LivingEntity.setHealth` (Paper's `Damageable#setHealth`), cut to the
  range from zero to the maximum, with no hook; a non-finite value is refused. This raw write can set zero
  or change a dead entity's health without running the death pipeline.
  `living.heal(host, amount)` is `LivingEntity.heal`, through the `living_heal` hook: a signed amount changes
  health only when current health is above zero. Both write players, guests
  (in their own turn) and mobs, and answer false for armor stands and the dragon.
  `living.last_attacker(host)` is `LivingEntity.getLastHurtByMob`: who last hurt the entity, within a hundred
  ticks of the hurt, as an `EntityRef` of the current open; `None` once the window has passed or the attacker
  is gone, and for armor stands, the dragon and clients. A mob attacker that changed dimension is found in its
  loaded level under the id it arrived with, never another entity holding its UUID.
  `living.kill_credit(host)` is `LivingEntity.getKillCredit`: it returns the remembered player first, or the
  last hurting mob if there is no remembered player. A remembered player that cannot be resolved gives `None`,
  not the mob. Both resolve across loaded dimensions; a credited mob there answers only `kind`, `uuid`, `level`.
- An `EntityRef` holds the world's open (`WorldEpoch`) and the entity's id, which no other entity of that open
  gets. A handle of an earlier open, of an entity that is gone or moved to another level (it gets a new id there, as in
  vanilla), or used on a client reads as `None`.
- The entities are those of the level in place (the one a hook runs in) and the server's players, whatever their level.
- `host.block_entity(level, pos)` finds a block entity in a loaded chunk, including in parked levels;
  `kind`, `position`, `level` and `data` read it. A removed or temporarily taken-out entity reads `None`;
  a replacement at the same position resolves to the new entity, unlike vanilla's object identity.
- `player.open_menu(host, block_entity)` opens its menu on the server and returns true only when opened. It returns
  false for stale or wrong-level handles, non-providers, cancelled opens, and players without a connection (including
  fake players), since no connection can receive the menu.
- `block_entity.viewers(host)` lists the players with its container open on the server, read from their open menus
  when called: players in its level, not spectators or fake players, a double chest's menu on both halves. `None` for
  a stale handle or a client.
- `level.visit_block_shape(host, state, pos, kind, &mut visit)` visits a supplied state's block-local outline or collision boxes with an empty context, without changing the world's state; pass `level.block_state(host, pos)` for the stored state.
  `None` for a client, unknown state, stale handle or closed level; parked levels can be read.
- `level.visit_entities_in(host, min, max, &mut visit)` visits readable entities whose boxes intersect the given box
  in the active server level or the client's displayed level. The visitor can read them through the same host.
  `ControlFlow::Break(())` stops the visit; `Continue(())` keeps it going. An empty query completes successfully,
  whereas a stale or unavailable level returns `LevelReadError` before the visitor runs.
- `level.visit_all_entities(host, &mut visit)` visits the active server level's lookup lazily in admission order
  (`ServerLevel.getAllEntities`), with no box filter, skipping any member that cannot resolve and stopping
  immediately on `ControlFlow::Break(())`.
- `level.pop_resource(host, pos, stack)` drops a stack as a broken block does (`Block.popResource`), `spawn_item(host, x, y, z,
  stack, pickup_delay)` and `spawn_item_with_motion` spawn a plain item entity and give its handle, with the signed
  pickup delay vanilla's `ItemEntity.setPickUpDelay` sets at construction (10 a block's drop, 40 a player's throw, 32767 never), and
  `award_experience(host, x, y, z,
  amount)` splits experience into orbs (`ExperienceOrb.award`), all in the level in place.

`level.set_block(host, pos, state, flags)` and `level.remove_block(host, pos, moved)` change the level in place. They return false for a stale handle, client, parked level, unloaded chunk or out-of-height position; parked writes wait for a safe level swap. Removal preserves the fluid's legacy block; `moved` marks a piston move. `BlockUpdateFlags` combines supported vanilla update flags with `|`. `CLIENTS` sends the update, while `INVISIBLE` suppresses notification only on the client, as in `Level.setBlock`. Skipping block-entity side effects suppresses container-content drops when replacing a chest; without it, the contents spill.

`level.destroy_block(host, pos, drop, tool, breaker, collect)` uses that same level's destruction and loot path
(`Level.destroyBlock(pos, drop, breaker, maxUpdates)`). `tool: None` means an empty hand, and `breaker` is
passed to loot (`this` entity) and the `minecraft:block_destroy` game event. With `collect`, the returned stacks
are not spawned as item entities; without it, a successful break returns an empty list after spawning them. The
normal block-drop hook still runs once before either outcome; tool enchantments and vanilla random ordering are
unchanged. In the block's level transaction the replacement is readable immediately; entities, experience and
break effects wait for commit. Aborting restores the block. Another level's transaction alone does
not defer destruction.
`drop: false` suppresses resources. `None` means nothing broke, including an invalid write, a stale/foreign
`breaker`, or air.
`level.set_break_progress(host, breaker, pos, stage)` shows stages 0..9 or clears the crack otherwise;
a machine uses a stable negative breaker id.

A helper whose catalog entry names a `receiver` (`entity`, `level`, `block_entity`) is called on that handle (for example `entity.position(host)`), rather than as an ID-based `Host` method.

### Client menu input

Client input uses `menu.click(host, &[mod_api::MenuClick::QuickMove { slot }])` on a mutable `ClientHost`.
Actions use menu-order slots; only `Pickup` accepts `None` for outside. Typed `MouseButton`,
`SwapWith` and `DragKind` values replace vanilla's wire codes, and `Drag { kind, slots }` expands
to a complete gesture. All actions are validated before the first is predicted or sent through the
normal `ContainerClick` path. A stale handle, invalid slot/hotbar index, creative-only action in survival,
spectator, non-client host or creative inventory screen returns `false` without side effects.
`true` is submission, not server acceptance; authoritative slot/cursor corrections still win.
`menu.close(host)` closes the menu's open screen as Escape does (`AbstractContainerScreen.onClose`,
`LocalPlayer.closeContainer`, which sends `ServerboundContainerClosePacket`), and `menu.click_button(host, button)`
sends `ServerboundContainerButtonClickPacket`. Both return `false` for a stale handle or a non-client host; `close` also
while the menu's screen is not open, and `click_button` for the inventory menu.
See [A menu's slots](../concepts/capabilities/menu-slots.md) for actions and an example.
Diverges from upstream: new helpers.

### Mob AI goals

`crates/sunrise-client/src/game/modding/native_mob_goals.rs` (#29's "movement and goals"): a mob's AI is vanilla's two goal
selectors, and a `MobRef` reads and changes them. `mob.goals(host)` and `mob.target_goals(host)` list each selector's
goals as a `MobGoalEntry` — the priority (a lower number preempts a higher one), the kind id (`melee_attack`) and
whether the goal runs now. `mob.add_goal(host, priority, &goal)` is `GoalSelector.addGoal`: the `GoalSpec` names one of
the vanilla goal kinds Sunrise itself runs, with the vanilla constructor's parameters
(`MeleeAttackGoal(speed, followUnseen)`), and `mob.add_target_goal(host, priority, &goal)` adds to the target selector.
`mob.remove_goal(host, id)` and `mob.remove_target_goal(host, id)` are `removeGoal`: every goal of that kind goes, a
running one stopped first, and the count comes back; as vanilla's `removeAllGoals` with a `getClass()` predicate, a
species variant of a vanilla goal (a bee's `BeeHurtByOtherGoal`, a labeled stroll) keeps its own name, is read under it
and is removed by it.

The kinds, by their id: `float`, `climb_on_top_of_powder_snow`, `panic` (with `water_range` for the turtle's),
`stroll` (`water_avoiding`, `swim` and `flying` pick RandomStroll's variants), `look_at_player`, `random_look_around`,
`melee_attack`, `leap_at_target`, `avoid_entity`, `tempt` (item ids, `minecraft:wheat`), `breed`, `follow_parent`,
`follow_mob`, `restrict_sun`, `flee_sun`, `eat_block`, `breath_air`, `try_find_water`, and the target selector's
`nearest_target` (with `prey` and `interval`) and `hurt_by_target` (with `alert`). Species ids in `prey`, `what` and
`only` are namespaced (`minecraft:zombie`); a bare id reads as `minecraft:`'s.

An add refuses what the mob's vanilla class does not hold: a `PathfinderMob` goal for a bat, ghast or phantom,
and `breed` / `follow_parent` for a mob that is no animal. Slimes, magma cubes and shulkers are `PathfinderMob`s.
`follow_mob` accepts ground or flying navigation, but not water-bound or amphibious navigation. It also refuses
non-finite numbers, negative speeds and distances, a probability outside 0..=1, a
negative interval, an empty `items` list, and a species id no mob type carries. Reads and removes answer `None`/`false`
for a stale handle, a client, the dragon, and a mob whose copy is held out — nothing in a mob's own tick resolves, as
`Host` is not there yet: change goals in `entity_join`, which fires for a loaded chunk's mobs too. Goals are not saved,
as in vanilla: after a reload the mob has its species' set again, and a mod's `entity_join` adds its own back.

These methods reuse built-in goal kinds; they do not define arbitrary goal callbacks or run at construction's
`Mob.registerGoals` moment. A typed goal registrar and callbacks on `MobRef` and Parts are not part
of this API yet. NeoForge has no bus event at this moment. The upstream raw-`Mob`
`register_goals` interface is not part of this fork's API.

### Mob AI controls

`crates/sunrise-client/src/game/modding/native_mob_refs.rs` and `native_mob_navigation.rs` (#938's mob AI controls): a `MobRef`
steers a mob through the engine's own controls, each a momentary wish the mob's own AI may replace next tick, as
`move_to`'s path is. Reads take the host by `&dyn Host` and writes by `&mut dyn ServerHost`.

- `mob.look_at(host, [x, y, z])` is `LookControl.setLookAt`: the mob's look control rotates its head toward the position
  over the next ticks. Non-finite coordinates are refused. `mob.jump(host)` is `JumpControl.jump`: the mob jumps on its
  next own tick by its own jump path — no velocity is written. No species refuses either: vanilla's brains themselves
  write through these controls (a brain species' `LookAtTargetSink` calls the same `lookAt`), so a mod's wish and the
  mob's own are one flag, not a brain memory a write could miss (which is why `set_target` refuses brain species, and `set_brain_target` writes their `ATTACK_TARGET` memory instead). A
  creaking being looked at, an angry bee and an axolotl playing dead hold their look control, and a held creaking its
  jump control, as in vanilla: the wish waits until the control runs again.
- `mob.navigation_speed(host)` / `set_navigation_speed(host, speed)` read and set `PathNavigation.speedModifier`, the
  multiplier the current path is followed at (1.0 is the mob's normal speed; 0 until a path sets one) — not the mob's
  movement-speed attribute. `move_to`'s speed argument sets the same modifier, and the mob's own next `moveTo` (a goal's
  or a brain species') sets its own again, as in vanilla. Non-finite speeds are refused.
- `mob.restrict_to(host, [x, y, z], radius)` is `Mob.restrictTo(pos, radius)`: the home area the mob keeps to — its own
  stroll and wander goals pick their positions inside it, as a lead's and a guardian's home do. It is saved with the mob
  (`home_pos`, `home_radius`). `restrict_center(host)` (`getRestrictCenter`; the last live one after a clear, `[0, 0, 0]`
  for a mob that never had one), `restrict_radius(host)` (`getRestrictRadius`; `-1` without a restriction),
  `has_restriction(host)` (`hasRestriction`), `clear_restriction(host)` (`clearRestriction`; the center stays) and
  `is_within_restriction(host, pos)` (`isWithinRestriction`; true everywhere without a restriction) read and change it.
  Radius `-1` in `restrict_to` clears the restriction while keeping the position as the center.
- `mob.left_handed(host)` / `set_left_handed` (`Mob.setLeftHanded`, the mob flags in its entity data, saved as
  `LeftHanded`) and `can_pick_up_loot` / `set_can_pick_up_loot` (`Mob.setCanPickUpLoot`, saved as `CanPickUpLoot`;
  an allay and a sulfur cube decide for themselves) moved here from the flat `living_*` helpers of #938's first round:
  vanilla keeps both on `Mob`, so `MobRef` is their home. The flat helpers are gone, with no aliases.
- All of these answer `None`/`false` for a stale handle, a client, the dragon (no `Mob` state), and a mob whose copy is
  held out of the level.

### Mobs drawn by a mod

`crates/sunrise-client/src/game/modding/native_mob_draw.rs`: `render_living_pre(entity, draw, model, canceled)` (client) is asked
once a frame for every mob and player in view, before its model (NeoForge's `RenderLivingEvent.Pre`). Set `canceled` and nothing
more of its own drawing happens this frame: model, layers, name tag, leash, outline and `render_living_post`. Its flames and shadow still draw (#1285).

- `event.model` borrows the selected body's hierarchy in bind pose. To leave out a part and
  its descendants after animation, hide its model-local index for this draw only:

  ```rust,ignore
  if let Some(index) = event.model.part("nose").map(|part| part.index) {
      event.model.hide(index);
  }
  ```

  A missing name doesn't refuse the draw. Layers using the same body respect the hide;
  independent models (armour, held items, the cape) are unaffected.
- `entity` (`MobRender`) is the frame's `LivingEntityRenderState`: the id, kind, custom name and `baby`, the partial tick,
  `age` (ticks lived plus the partial tick), `body_yaw`, `head_yaw` (from the body) and `pitch` in degrees, the walk cycle
  (`walk_position`, `walk_speed`: a vanilla quadruped's legs swing `cos(walk_position * 0.6662) * 1.4 * walk_speed` radians), `hurt_time`,
  `death_time` and `on_ground`.
- `draw` is a block entity's frame (`block_entity_submit`): draw with `BlockEntityDrawContext::cube`, `quad`,
  `block` (taking a resolved `BlockState`) and `item` under its pose stack, which starts at the mob's feet turned with its body: +z
  is the way it faces, +x its left, +y up, in blocks. Turn a head or swing a leg with `rotate_about` and a pivot. It holds
  the mob's light.
- A mod's own creature is an ordinary mob (`LevelRef::spawn_mob`) drawn this way: a quiet villager, an allay or a bat
  gives it movement, sounds and saving, and the mod gives it a model and animations of its own. Tell yours apart by the
  ids the server half sends with mod messages, or by its custom name.
- Players wrap the living hooks with `render_player_pre(player, who, draw, model, canceled)` (`RenderPlayerEvent.Pre`)
  before the living pre, and `render_player_post(player, who, model, draw)` after the living post, once a frame for each
  other player in view and for this window's player in third person when it holds the camera, with the same frame and
  `who` (the profile name, and `local`). The pair stays at the player's join slot, interleaved with general entity
  renderer submissions. Living Pre sees edits already made in Player Pre. Canceling Player Pre skips Living Pre and both
  Post hooks; canceling only Living Pre still calls `render_player_post` with `model: None`.
- `render_living_post(entity, model, draw)` (`RenderLivingEvent.Post`; a type's own layers are `register_entity_layers`)
  adds after the entity's model and layers, in the same frame. Use the optional final `event.model`
  to find a part by name or follow the parent, child and sibling links with `part_at(index)`. Read
  `part.pose()` and check `part.visible`, which includes its ancestors. A successful replacement
  renderer supplies no body pose to the layers or Post. Don't keep a model view or index after the callback.

### Advancement triggers

`crates/sunrise-client/src/game/modding/native_criteria.rs`: a mod's own criteria triggers, as a NeoForge mod registers a
`CriterionTrigger` and calls its `SimpleCriterionTrigger.trigger`. The advancement is an ordinary file of a content mod
or a world's data pack (`data/mymod/advancement/connect_a_pipe.json`) whose criterion names the mod's trigger and
whatever conditions the mod reads:

```json
{
  "criteria": {
    "copper_pipe": {
      "trigger": "mymod:pipe_connected",
      "conditions": {"pipe": {"items": "#c:ingots/copper"}}
    }
  }
}
```

- `player.trigger_criterion(host, &trigger, &mut test)` lends a `CriterionTest` to the server for one synchronous
  call. For each open criterion of that exact trigger, the matcher runs first; then the game tests `player` in
  the target player's own level. Matches receive the usual progress, rewards, chat line and toast.
- `CriterionTest::test(&mut self, criterion: &dyn CriterionContext)` returns `Result<bool, CriterionTestError>`.
  The test may borrow from the triggering event. `criterion.conditions()` borrows the parsed `Value` bound at
  advancement load (`Value::Null` when absent). `criterion.item(key, &stack)` tests an item predicate using the
  target level's tags. A missing key matches; an invalid stack yields `CriterionTestError::Stack`.
- `Ok(())` means the player was reached, not that anything matched. An exact built-in key uses its built-in
  trigger path; a custom key does not need to be in the calling mod's namespace. An unavailable player or a
  client is refused, as is a player whose level cannot be entered. An inline `player` predicate uses the vanilla
  loot condition codec's `type` field; a malformed condition rejects the advancement at load. A callback refusal
  or panic ends the current invocation, not later calls with the same key. A panic logs and shows an error toast
  naming the mod.

### Explosions

`crates/sunrise-client/src/game/modding/native_explosions.rs`: for server mods, in the level in place; the full contract,
with the causing-entity table, is the book's "Setting off an explosion" (how-to/world/block-experience-and-explosions.md).

- `LevelRef::explode(host, source, center, radius, fire, interaction, settings)` is vanilla's full `Level.explode`. The
  causing entity comes from `source`, never the caller (a primed TNT's owner, a living entity itself, a projectile's
  living owner); with both the damage is `minecraft:player_explosion`. `interaction` is `None`, `Block`, `Mob`, `Tnt` or
  `Trigger`; `fire` works with any. `ExplosionSettings` names a damage type, an `ExplosionCalculator` (blocks, entities,
  knockback, an immune tag that replaces the resistance rule), particles and sound, each `None` vanilla's. No radius
  limit. Refused with an `ExplodeError`, doing nothing; the book lists when.
- A blast set off inside `explosion_detonate` runs that hook again, so a mod that chains blasts keeps its own guard.

### Presentation view transforms

`render_view_transform(partial, transform)` (client) runs each frame at vanilla's `GameRenderer.bobHurt` /
`bobView` stage, after hurt tilt and view bobbing, including when a screen takes input while the world
still draws. `transform` starts at identity; compose rotations, translations, or scales
onto it in dependency order (for example, camera shake scaled by `ClientGuiOptions::screen_effect_scale`).
The transform and its inverse must both be finite. The camera angles and position, audio listener, player
position and aim are unchanged.

### A player's motion

`crates/sunrise-client/src/game/modding/native_motion.rs`: a push on a client.

- `native_push_player([x, y, z])` (client) adds a push to the player's own velocity, in blocks a tick, as an
  explosion's knockback does: the player's own game moves them. The server didn't send it, so a server's movement checks
  may see a move they can't explain; for a push the server should stand behind, have the server send it. Each component is
  cut to 4 either way; an upward push lifts the player off the ground. A spectator, no player, or a value that isn't
  a number is left alone. A server mod that wants a client pushed sends its client half a message.

### Biome colours

`crates/sunrise-client/src/game/modding/biome_colors.rs`: the grass, leaf and water colours a client mod changes while the
game runs, for seasons (a golden autumn, a pale winter) or a world that greens as it heals.

- `biome_colors(biome, colors)` (client) changes a biome's `BiomeColors`: `grass`, `foliage` (leaves, vines),
  `dry_foliage` (leaf litter) and `water`, 0xRRGGBB, starting from the biome file and the colormaps. The dark forest's
  modifier still darkens the grass set here; the swamp's noise colours replace it.
- It is asked for every biome when a mod calls `native_refresh_biome_colors()`. When the colours differ from those in use,
  every loaded section and the far terrain are rebuilt with them, as changing Biome Blend does. Call it when your colours
  change and once when the player joins, not every tick: a rebuild takes the workers a few seconds. A slow fade is a
  call a day, not a frame.
- Block-breaking and falling-leaf particles keep the biome's own colours.

### Sounds a client mod plays

`ClientHost::play_local_sound(sound, after)` (client) plays `sound`, a `SoundPlay`, for this player only. Nothing is sent
to the server, and other players don't hear it. It takes `&self`, so the screen, key and mouse hooks, where interface
sounds belong, can call it: the sound is queued and starts at the next audio update (once a client tick), `after` ticks
counted from then on the unpaused sound-engine clock (`SoundEngine.playDelayed`).
- `SoundPlay::ui(id)` is an interface sound (`SimpleSoundInstance.forUI`): at the listener, on the `Ui` source, not
  fading with distance, as the game's own button clicks play.
- `SoundPlay::at(id, source, position)` plays at a place in the client's level, fading with distance. A volume past 1
  carries farther, as vanilla's attenuation does.
- `with_volume` and `with_pitch` supply base values to `SoundEngine.calculateVolume` and `calculatePitch`.
  `looping(delay)` repeats after `delay` ticks if positive or loops automatically otherwise.

It's the same record the `play_sound` hook edits, and it goes through the same mapping into the engine. The event is
vanilla's (`minecraft:entity.generic.explode`) or the mod's own: a content mod's `assets/<id>/sounds.json` and
`assets/<id>/sounds/*.ogg` load with the rest of its assets. A client half that hears from its server half (mod
messages) where something happened can time the sound to the distance, such as a blast heard when its shock wave reaches
the player: `after = distance / speed`. It answers `None` on a server, for a malformed namespaced event ID, a position
that cannot be represented as finite mixer coordinates, non-finite volume or pitch, or invalid `SoundEffects`. An event
the resources lack plays nothing. Diverges from upstream: horizon-rust/horizon's `native_play_local_sound` takes
`(sound, at, volume, pitch, delay)`.

Otherwise it answers a `SoundRef`, a handle on this one play, never reused. Its methods take the `&dyn ClientHost`, so
any client hook can call them. Like the play, the writes are queued for the next audio update, while the answers are
computed at the call:
- `sound.state(host)` is `Some(SoundState)`: `Waiting` (queued, waiting out `after`, or between a looping sound's
  repeats), `Playing` (`SoundEngine.isActive`) or `Stopped`. It is `None` on a server.
- `sound.stop(host)` stops it for good: a waiting sound never starts and a looping one never repeats. A stop asked
  since the last update already reads `Stopped`.
- `sound.fade(host, volume, ticks)` moves its volume to a finite target in equal steps over `ticks` unpaused ticks
  it plays, or at once for 0. A nonpositive target stops the sound when the fade ends. A later fade replaces one in
  progress. While paused, the fade waits and any interface or music sound keeps playing at its current volume.
- `sound.move_to(host, position)` moves the same instance without restarting it, in world blocks or relative to the
  listener. Each coordinate must be finite in the mixer's float representation (`Channel.setSelfPosition`).
- `sound.set_pitch(host, pitch)` sets a finite base pitch on the same instance, leaving the final pitch to the engine.

These writes answer false, changing nothing, once the sound is over. Once it ends, is stopped or faded out, or is
cleared (leaving or changing levels stops every sound), the handle reads `Stopped` and its writes do nothing. It stays
valid across a change of output device, which reopens the output on the same mixer and keeps the sounds playing.
Dropping or overwriting the handle doesn't stop the sound: keep a looping sound's handle while it plays (stop it before
starting another in its place), or nothing can stop it until the player leaves the level.

`ClientHost::sound_events()` lists every sound event the client's resources have (`SoundManager.getAvailableSounds`).
`audio_devices()` lists the output devices by name (`SoundEngine.getAvailableSoundDevices`), and `audio_device()` names
the one the options ask for, empty for the system's default. Both lists are built anew at each call, and listing the
devices probes each one: ask from setup or a settings screen, not every frame.

### Column reads

`crates/sunrise-client/src/game/modding/native_world_columns.rs`: streaming column queries on a
typed `LevelRef`, working on active and parked server levels (and, for heights, the client's open
level) without loading chunks or imposing guest column/block caps. Refusals return
`LevelReadError` without invoking `visit`, and returning `ControlFlow::Break(())` stops early.

- `LevelRef::visit_surface_heights(host, columns, heightmap, visit)` (both sides) visits each
  column's height on `heightmap` (`Heightmap`, one above the highest block that counts), `None`
  for a column whose chunk isn't loaded.
- `LevelRef::visit_columns(host, columns, heightmap, (below, above), visit)` (server) visits each
  loaded column as a borrowed `ColumnSlice<'a>` carrying its heightmap `base`, complete
  `BlockState` slice (`blocks`, bottom first, air outside the chunk's vertical range), and
  server-side biome JSON water colour (`water_color`, `0xRRGGBB`, at `base - 1`); unloaded columns
  yield `None`. `below + above` must be `1..=level_height` (`LevelHeightAccessor.getHeight`). Read
  blended grass and foliage tints on the client with `BlockAndTintGetter::block_tint` inside tint
  callbacks or `ClientHost::biome_tint` in other hooks.

### Chat and commands a client mod sends

`ClientHost::send_chat(message)` calls the play connection's `sendChat` or `sendCommand`. A leading `/` sends
its remainder as a command unless a client command listener handles it. `client_chat` runs for plain chat before
signing and may edit or cancel the send. The play listener owns signing and last-seen updates; a refused packet
leaves those pending. Normalization, the local Chat option and history apply to screen input, not a direct call.
`Ok(())` includes a local command, a canceled send or an empty chat result; `Err` reports an unavailable play
listener, wire encoding failure or local publication failure. Server policy still applies.

### Server menus

`crates/sunrise-client/src/game/modding/native_menus.rs`: menus a server mod builds from ordinary items, which a
stock client shows as a chest (`generic_9x1` to `generic_9x6`); nothing is installed on the player's side.

- `player.open_chest_menu(host, rows, title, items, replace)` opens one in place of the player's open menu (returning its `MenuRef`, with a new window id each time, as `ServerPlayer.openMenu` counts them); the old menu closes on the server as a close does, its cursor stack back to the inventory, and its window's packets are ignored. `MenuReplace::CloseFirst` first sends the client a close, vanilla's default; `MenuReplace::InPlace` doesn't (`MenuProvider.shouldTriggerClientSideContainerClosingOnOpen`), so the title and rows change without the mouse centring again. A title changes only this way. `menu.close_container(host)` closes it.
  The returned `MenuRef` (or `player.container_menu(host)` for the currently open one) exposes its slots as [menu slots](../concepts/capabilities/menu-slots.md), written in a transaction, and the tick's container sync sends each changed one (`ContainerSetSlot`), or `menu.set_items(host, items)` replaces the whole menu in one packet. Change its items that way, not by opening it again.
- A display menu's default click moves nothing. `MenuPart::menu_clicked` v4 runs directly on parts registered for
  the menu type before default click execution, with no synthetic decided-action wrapper or whole-inventory rollback.
  Returning `Flow::Cancel` skips only the default click and resyncs the menu, cursor and any omitted swap target;
  returning `Flow::Continue` rechecks the live menu, including `stillValid`, before running its default. An open or
  close in a transaction takes effect at the outermost commit; an abort leaves the old menu and both cursors intact.
  A panic disables only that mod's `menu_clicked` callback for the session with an error log and toast. It skips the
  default for that click, keeping committed edits and resyncing the still-live menu.
- `MenuPart::menu_removed` runs the owning menu's teardown (`AbstractContainerMenu.removed`) with the menu type, opener block, optional live `PlayerRef`, player UUID, window id, any opening hand and detached owned slot values.
- A trade on these menus leaves the offered stacks in their owners' inventories until both sides are settled. Then,
  in one `host.transaction`, each offered inventory index is checked to still hold exactly what was offered, taken
  out of the player's [`ENTITY_ITEMS`](../concepts/capabilities/entity-items.md) handler and put into the other's with `insert`
  ([the recipe](../concepts/capabilities/entity-items.md)). A changed offer or no room returns an error, and every move is put back.

### Render functions

`crates/sunrise-mod-render/src/render.rs`: native-only, unstable raw frame places, outside `mod_api`.
A native mod replaces one with a `HEAD` mixin at `sunrise_mod_render::render::<name>`. They run on the render thread with the frame's wgpu objects;
what a mod draws there shows in that frame. Draw contexts remain the API-typed drawing route.

- `world_drawn(world)`: the world is drawn (sky, terrain, entities, particles, weather and the world post pass),
  the first-person hand, the HUD and screens not yet. `WorldDrawn` holds the device, queue and encoder, the frame's
  colour target (copy it out before sampling it), the world's reversed depth and its size, the camera's position and
  its matrices with and without TAA's jitter, the frame's camera-relative entity quads with their runs by entity, the
  entity atlas, the shared quad index buffer and the block sprite arrays ([block textures](#block-textures)). Not
  called on the title screen's panorama. `motion` says which mob, player or dropped item each run of the entity quads
  is, by its entity id, so a mod can match an entity's vertices with last frame's (a motion blur's own motion
  vectors); `spans` only name the type.
- `hand_drawn(hand)`: the first-person hand and held item are drawn, after `world_drawn` and before
  entity outlines, post effects, screen effects and the HUD, on every frame `world_drawn` runs. `HandDrawn`
  holds the device, queue and encoder, the frame's colour target with the hand in it, the hand's camera-relative
  quads (empty when no hand drew: third person, F1, a shader pack's own hand), the hand's clip matrix (its own field
  of view and the view bob, no jitter), the camera, the entity atlas and the quad index buffer. The hand's depth isn't
  kept: draw its quads into a depth of your own. A motion blur can match the hand's quads with last frame's for the
  swing, attack, equip and bob motion, and blur the picture with the hand in it.
- `render_needs(needs)`: set `entity_spans` to have the entity geometry split into runs by entity without a
  shader pack (each run's id is stable from frame to frame: a hash of the entity type's name, a block entity's state),
  and `world_depth` to keep the world pass's depth when nothing else needs it. Set what you need and leave the rest:
  several mods can ask. It's asked as each frame is built, so keep it cheap.

### Block textures

`crates/sunrise-client/src/game/modding/native_block_sprites.rs` and `WorldDrawn::block_sprites`: for a client mod drawing
blocks in its own pass (flying debris as voxels, say) with the player's own block textures, resource packs included,
as the terrain samples them.

- `WorldDrawn::block_sprites` (`BlockSpriteArrays`) holds the block sprite arrays as `D2Array` views: the terrain's
  eight, a 1-layer stand-in past the ones in use, each with the mip levels the Mipmap Levels option leaves, animated
  layers at the frame shown. Each texture is a whole square layer of `size` texels, its UVs 0 to 1 across it,
  gamma-encoded as the world uses it. `split` layers go to an array (2048 on most GPUs, fewer where the GPU holds
  fewer): every vanilla texture is usually in array 0. `nearest` samples texels sharp (nearest, repeating, the
  nearest mip level), `filtered` is the terrain's own sampler.
- `native_block_sprites(blocks)` says, for each block (`minecraft:grass_block`, `oak_log[axis=x]`; properties left
  out take the default), what its model draws on each side, in `Direction` order (down, up, north, south, west,
  east): its faces in the order drawn, each with its `sprite` (`array`, `layer`, `animated`, `cutout`), its `rect`
  on the side and `inset` from the side's plane (a whole face is 0, 0, 1, 1 and 0), its `uv` map, its `tint` source
  and whether it is `translucent`; the model's `particle` sprite; whether it is a plain `cube`. None for a block the
  game doesn't know. A side's axes run along x then z for down and up, x then y for north and south, z then y for
  west and east, from the block's corner nearest 0, 0, 0 (`block_sprites::side_axes`); the point (a, b) of a side
  shows the sprite at `(uv[0], uv[1]) + a * (uv[2], uv[3]) + b * (uv[4], uv[5])`, rotations and mirrors included
  (for a plain block's north side, `u = 1 - x, v = 1 - y`). A tinted face is multiplied by its source's colour:
  `Grass`, `Foliage`, `DryFoliage` and `Water` are the biome's where the block is, `Fixed` a constant (spruce
  leaves, redstone at its power). A grass block's side is two faces, the side and then its grey overlay tinted with
  grass. The model is the state's first random variant; faces that don't lie flat on a side (a flower's crossed
  planes) are left out, and the `particle` sprite stands for them. Fluids give their still texture on every side.
- `native_texture_sprites(textures)`: a texture's sprite by name (`block/stone`), none for one no model uses.
- `native_gui_sprite(sprite)`: a GUI atlas sprite's `AtlasRegion` by `Identifier` (`minecraft:container/slot`,
  `minecraft:particle/flame`, `mymod:panel`), its UV rect and its size in its own pixels, none for one the atlas
  lacks. Plain data, right until the next `texture_atlas_stitched` for `TextureAtlas::Gui`.
- `native_block_sprites_version()` changes each time the resources load (a resource pack turned on or off moves the
  layers, and the arrays are new): ask again then.

Look blocks up once and keep what you need (a few words a block for your own shader), not every frame.

### Menu slot rules

`menu_slot_rules` asks the registered vanilla or mod menu's scoped `MenuPart` whether a slot takes a stack, whether it gives up what it holds and how many it holds, on both client prediction and server. Mount inventories use `sunrise:mount_inventory`; unregistered player inventory and creative picker menus do not dispatch. `slot` is the backing container index. The slot's own answers arrive first; the final maximum is bounded to 0–99 and the item's stack limit (matching vanilla `Slot.getMaxStackSize` and `Container.getMaxStackSize`). Rules also apply to quick-move top-ups. The hook takes the read-only `Host` (it is a query: it cannot write or cancel). The part can read the menu's current slots and the player's inventory (`Player`: `0..36` inventory, `36..40` armor feet-to-head, `40` offhand, matching vanilla `Inventory`) with `context.slot(container, index)`. Each live `mayPlace`, `mayPickup` and `getMaxStackSize` check during a click asks the parts directly, matching vanilla `AbstractContainerMenu.doClick` and `Slot`. On a LAN host's own click the game is lent to the click with the LAN host lifted out of it, so what the `Host` reads of LAN players answers as if no LAN were open. Inside a slot-rule Part, nested `MenuRef::slot_holds` and `slot_gives` reads keep the slot's own answers without dispatching menu Parts again, even on another slot or menu. `may_pickup` (`Slot.mayPickup`) arrives true and is read only on a registered mod menu type, for its own slots and the player's inventory slots in it, asked with the stack the slot holds; a vanilla menu type keeps its slots' own answer. The slot's own rule comes first (a slot it refuses is not asked), and one ask answers `may_place` and `may_pickup` together. It is asked by a pickup click on a slot that holds an item (after `item_stacked_on_other`, before any take, merge or exchange), by quick move once on the clicked slot before the move (moving into a slot asks only `may_place`), by a hotbar or offhand swap that takes the slot's item (with both stacks present `may_place` must hold too), by throw inside the take (a refusal drops nothing and stops the repeat), by pick-all (which gathers when the clicked slot is empty or refuses, and skips each source that refuses) and by every take the menu makes. A drag spread, a creative clone, an insertion into an empty slot and a click outside the window never ask it; a drag that ends on one slot is a pickup click on it. A refusal leaves the slot and the cursor as they were, and the server corrects the slots and cursor the client predicted otherwise. A slot that refuses placement gives all of its stack or none. A display-only slot (a recipe preview) answers both `may_pickup` and `may_place` false. A shift-click a part answers with `menu_quick_move` is asked the pickup rule first, as vanilla asks it before the menu's `quickMoveStack`.

The hook runs during clicks, drags and shift-clicks with the read-only `Host` on server and client prediction; answer from the stack and host alone, and fast. If a part panics, its answers are discarded and its callback is disabled for the session; the shared log and error toast name the mod. Other parts still answer the check. Parts that read `context.setting(..)` or `side()` have no counterpart on the Host.

### The loop context

`dyn LoopContext` is the read-only view for a hook fired inside a menu, recipe or
player-list loop, where no game is at hand (a catalog entry names it with
`"context": "loop"`; `player_name_format`,
`player_tab_list_name_format`, `living_jump_power`, `living_speed`, `living_flying_speed`, `living_heal`, `living_drown`, `living_fall`, `living_jump`, `ender_entity_teleport`, `fluid_flow`, `player_stays_on_ground_surface`, `block_jump_factor`, `block_speed_factor`, `friction` and the sound hooks below take it). `side()` is `LoopSide::Server` or
`LoopSide::Client`, `registry(name)` borrows names in registry ID order,
`setting(mod_id, key)` returns a loaded mod's setting captured when the context was
built, and `mods()` is the `LoadedMods` view of registration. It has no game Host,
handle construction or transaction, and its answers never change while it is held.
The side that owns the loop builds it, never a hook: the client builds one at window
start and again after each resource reload, and a server builds one when it starts its
mods and again on each `/reload`, so a reload captures fresh settings. A LAN or dedicated server's hub holds the server's
current one, so its Tab list and chat names, and the clicks of its players, see it too.

`item_stacked_on_other(host, event, canceled)` is NeoForge's `ItemStackedOnOtherEvent` plus vanilla's `Item.overrideStackedOnOther` / `Item.overrideOtherStackedOnMe` on `ItemPart` (version 7), fired inside a pickup click with the game's `Host` and the cursor and clicked slot lent in `event.click` (`click.direction` distinguishes `Event`, `CarriedOnOther`, and `OtherStackedOnMe` in vanilla `AbstractContainerMenu.tryItemClickBehaviourOverride` order); see its hook documentation. `item_destroyed(host, stack, item, damage_type, drops)` is `IItemExtension.onDestroyed` (`ItemStack.onDestroyed`) on `ItemPart`, fired on the server inside a transaction with the original intact stack before built-in components are cleared and before the destroyed dropped item entity is discarded, returning validated drop stacks to spawn at the entity's position. `player_name_format(context, username,
displayname)` and `player_tab_list_name_format(context, player, name, display_name)` format a player's name for chat and
commands (both sides) and for the Tab list (server). The name hooks are contract version `2`: a mod written for `1` takes the
context as a new first argument of the `Hooks` method, and reads the side from it instead of `event.side`.

The predicate supplied by `IngredientDefinition` takes the matching side's read-only loop context and an `IngredientStack` borrowed for that call. `Ingredient::test(host, stack)` uses the host's loop context. There is no hook dispatch during matching: the predicate runs as the mod that defined it, or rejects the stack without running if that mod is disabled.

### Sound hooks

Four client hooks take the `LoopContext` from the sound engine, which has no game at hand and plays sounds from its own loops:

- `play_sound` is `PlaySoundEvent`. It fires at the start of vanilla's `SoundEngine.play`, before the engine's own checks, for every sound the client plays. `original` is the sound as asked for and `sound` is `&mut Option<SoundPlay>`, a copy of it that later mods see edited: change its volume, pitch, source, position, identifier, looping, delay, attenuation, `relative` or `effects`, or assign `*event.sound = None` to cancel or stop playback. A ticking sound's live refresh (`starting` false) takes new volume, pitch and effects; a category volume refresh takes new volume and pitch. At start and on refresh, base volume and pitch, including zero, negative and infinite values, reach the engine's final calculations. Invalid effects (`SoundEffects::valid`) refuse playback. `effects` shapes the sound in the mixer: gain, a low-pass and a high-pass on the direct sound and sends to the shared `room`, `hall` and `cavern` reverbs. Every effect change glides over one output buffer. A changed identifier is resolved again at start, with its own weighted pick. Only the fields of `SoundPlay` carry over; the engine's random, ticking and resolved sound never leave it. A sound nobody edits plays as before.
- `play_sound_source` is `PlaySoundSourceEvent` and `play_streaming_source` is `PlayStreamingSourceEvent`. A sound fires exactly one of them, after the engine started it on a channel: streamed sounds (music, records) the second, every other sound the first. They are notices, and `ChannelRef` is an id only: a mod shapes a sound through `play_sound`'s `effects`, not through the channel.
- `sound_engine_load` is `SoundEngineLoadEvent`. It fires when the output is open: at window start, after each resource reload (with the new generation's context) and when the output device changes. With no audio (no device, or `SUNRISE_NO_AUDIO`) the engine never loads: this hook and the two source hooks do not fire, while `play_sound` still does, since it comes before the engine's check that it loaded.

All four early-out on the mod's subscription before building a `SoundPlay`, so a game with no listener pays one branch per play. They are client-side only: a server has no sound engine and builds no context.

### Items' default components

`Hooks::modify_default_components` receives `&dyn DataLoadContext` before its
`ModifyDefaultComponentsEvent`. The engine supplies this read-only phase context:
`origin()` is `DataLoadOrigin::ServerDataLoad` or `ClientPreload`, `registry(name)`
gives the names in registry ID order as a shared `Arc<[String]>` (no copy), `setting(mod_id, key)` returns a loaded mod's
setting captured when the owning context is built (including missing keys), and
`mods()` gives the same `LoadedMods` view used during registration (shared; cloning it
copies no package). A later owning
load captures fresh settings even when it reuses registry inputs; runtime Host
settings still read live storage. There is no game Host, handle construction or
transaction. The origin describes the actual owning load, not its thread name or
the side of a later reader.

The lower firing target is `modify_default_components(context, item, components)`
in `crates/sunrise-world/src/item_defaults.rs` (NeoForge's `ModifyDefaultComponentsEvent`).
`components` is the item's entry in `items.json`, vanilla's default-component report:
insert or replace a key to set a component (`"minecraft:max_stack_size": 16`), remove it to take the component away.
Any item can be changed, vanilla's included. Write values as vanilla's codec writes them: nothing decodes them with it.
Each table reads the fields it understands and takes a default for a field it can't read, but keeps the component, so a
malformed `minecraft:food` still makes the item edible, with no nutrition. A stack size above vanilla's 99 is kept: the
data pack's own is held to `ExtraCodecs.intRange(1, 99)`, a mod's to 1 to `i32::MAX` (the count the wire and the save
carry), and a value that is not a number leaves the data pack's own. Keep a stack above 99 to a server whose clients run
the mod: a vanilla client decodes the count with vanilla's 1 to 99 and cannot show it.

- One owning load decodes the defaults, asks once per item, then publishes one immutable snapshot. The item
  table, loot, enchanting, item models, banners, pot sherds and rarity readers borrow that snapshot; workers
  retain shared ownership. The renderers also share its canonical component prototypes. Stacks are saved and
  sent as their difference from the changed defaults, as from vanilla's.
- Reusing client-preloaded item data on the integrated server is not another load. Opening a world or running
  the data-pack `/reload` does not replay this hook. A dedicated resource load supplies `ServerDataLoad`;
  the client preload supplies `ClientPreload`. The hook may run on a load worker, before a game Host exists.
- Client and server each apply their own mods, so both need the mod. A client without it (a vanilla Java client, or
  Sunrise without the mod) fills in vanilla's defaults for the same stacks: a stone stack the server caps at 16 still
  looks like it stacks to 64 there.
- The creative search tab matches vanilla's exported tooltip text, so a changed `item_name` is not searchable yet.

## Counterparts

A hook's optional `neoforge` catalog key names its NeoForge 26.3 counterpart;
that counterpart sets the hook's name, side and cancellation contract. The optional
`fabric` and `paper` keys name Fabric and Paper events at the same moment with the
same contract. Each key is a string or a list of strings; omission is valid. One
hook serves each matching moment; near matches with different timing, side or
cancellation are gaps, not counterparts. A hook with none of these keys names the
mod mixin or use it replaces in its design or PR.

## Left to do

- A list of every native build, to delete the ones no instance needs.
- A prebuilt horizon-mixins from its CI instead of building it on the player's computer.

## More hook capabilities

- `entity_join_level(dimension, kind, pos, loaded_from_disk, subject, canceled)`, an entity joining a
  server level (NeoForge's `EntityJoinLevelEvent`, vanilla's `PersistentEntitySectionManager.addEntity`):
  `loaded_from_disk` is true when it comes back with its chunk, false for a fresh add (a drop, an orb, `/summon`, an
  arrival through a portal into another level). The entity is already in the level's lists when the hook runs; `canceled` takes it back
  out, and a `/summon` of it fails with vanilla's "duplicate UUIDs" message. It tells every kind in the game's entity
  lists (items, orbs, mobs, boats, minecarts, armor stands, primed TNT, falling blocks, projectiles, rockets, tridents),
  once each, in creation order, so check `kind`. A drop, an orb, a mob, a `/summon` and a chunk's entities are told at
  once. Arrow and throwable spawns, and fresh falling blocks, are also told at insertion. Other gameplay spawns are
  offered between tick phases, before they tick again.

- [A block's items](#a-blocks-items) are available to server mods through the item capability and
  `move_resources`, so a pipe or machine can move items transactionally.

- `host.set_display_slot(DisplaySlot::Sidebar, Some(objective))` shows an objective from the server scoreboard.
- What a server mod needs for a public server's screens: `player.set_sidebar(host, sidebar)` shows one
  player a sidebar of up to 15 lines (a text, and one where the score would be) that only their client holds (the
  server's scoreboard never sees it; a mod sends it again when they rejoin), or takes it away with `None`;
  `player.tab_list(host, header, footer)` sets the lines over and under their Tab list; `native_tick_stats()` reads the
  server's tick rate and average tick time in ms (`/tick query`'s); and `player.set_sleep_ignored(host, ignored)` leaves a
  player out of `players_sleeping_percentage`'s count until they are in bed (players away from the keyboard). It refuses
  when the server list is unavailable. Read where they look through each handle's entity.

- Damage a mod can change or cancel through `native_living_incoming_damage` (NeoForge's
  `LivingIncomingDamageEvent`): where `LivingEntity.hurtServer` starts, once its guards passed (invulnerable, dead or
  dying, fire with fire resistance; Sunrise's mobs don't check the fire resistance effect there yet), on every damage
  route, with the hit's level host (its blocks, random and own attachments), the hurt entity's id and type
  (`minecraft:player` for a player), the hit's source (`HurtSource`: the damage type, `minecraft:fall`; the causing
  entity, `causing`, vanilla's `DamageSource.getEntity`: a skeleton for its arrow; the direct entity, `direct`,
  `getDirectEntity`: the projectile's entity id when the damage type names one, for hits on players, mobs and
  the ender dragon, including an ownerless projectile, or the attacker itself in melee; `magic()` and any other
  source with no direct entity stay 0 even during that projectile's impact; and `position`, `getSourcePosition`), the `victim`'s
  position and view vector, the direct entity's `weapon` (`getWeaponItem`) when Sunrise knows it (a player's or mob's
  melee hit or projectile's saved weapon), `causing_weapon` (the causing living entity's current main hand; None for absent,
  unknown, non-living or gone owners), and a live mutable borrow of the engine's `DamageContainer` before reductions
  (no snapshot conversion). Entity ids are 0 for none and for the untracked sources listed below. A mod may change the
  container's new damage or the invulnerability ticks a fresh hit leaves (vanilla's 20), or set `canceled`: nothing of
  `hurtServer` itself happens then (no damage, hurt sound, knockback, invulnerable time, statistics or criteria), and
  the caller acts as on a refused hit, so a blast still pushes a mob whose hit was canceled, as vanilla pushes one that
  refused it. A canceled hit on the ender dragon still counts as landed, as vanilla's `EnderDragon.hurt` returns true
  whatever `reallyHurt` did: the arrow is spent and the melee effects run. A ghast's canceled hit by a fireball it was
  sent back counts as landed (`Ghast.hurtServer`). A negative amount hurts by 0 and a NaN or infinite one by
  `Float.MAX_VALUE`, as vanilla clamps them. It fires for the players, mobs and the ender dragon the game hurts, and
  runs on the thread that ticks the world (the server thread), so state a mod shares with its render thread hooks needs
  synchronising. `/kill` and `EntityRef::kill` don't reach it yet: they set the health to 0 instead of hurting with
  `minecraft:generic_kill` as vanilla does. A mod may also install owned reduction modifier closures on the live
  container (`container.add_reduction_modifier(Reduction::Armor, |view, amount| amount * 0.5)`, NeoForge's
  `addModifier` / `IReductionFunction`): each runs in registration order when its reduction stage executes
  (the damage cooldown, armor, Resistance, protection enchantments, a witch's innate resistance, the absorption hearts),
  receiving a read-only `DamageContainerView` of the live container at that stage and the current reduction value
  (vanilla's or the preceding modifier's output). Matching NeoForge's `DamageContainer` semantics, the modified
  reduction is recorded verbatim without clamping to `[0, before]` or substituting for `NaN`, and only the resulting
  damage is floored at `0.0` (`Math.max(new_damage - modified, 0.0F)`). For `Reduction::Absorption`, modifiers receive
  `Math.min(hearts, damage)`, the modified value is recorded as `Reduction::Absorption`, remaining damage is
  `Math.max(damage - modified, 0.0F)`, and the absorption pool deducts `Math.min(damage, modified)` floored at `0.0`
  (`Math.max(0.0F, hearts - absorbed)`). Guarded stages the hit does not enter (for example, `#bypasses_effects`, zero
  Resistance reduction, no Protection enchantments, a non-matching Witch hit, or a Wolf hit absorbed by wolf armor) run
  no modifier; `Reduction::Armor` and `Reduction::Absorption` run at their enclosing `actuallyHurt` sites even with a
  `0.0` starting reduction. Modifiers last for the hit. A panic inside a reduction callback is caught through the
  shared hook panic boundary (without writing a crash report), disables that mod's reduction
  callbacks for the rest of the session, reports loudly via `log::error!` and an error toast naming the mod, and leaves
  the incoming reduction value unchanged so other mods and the hit continue.

- `living_damage_pre` (NeoForge's `LivingDamageEvent.Pre`) runs in `LivingEntity.actuallyHurt` after armor and magic
  reductions and before absorption, lending the same live mutable `DamageContainer` (no snapshot conversion). A listener
  may read earlier reductions, edit `new_damage` or `post_attack_invulnerability_ticks`, or call
  `add_reduction_modifier` for subsequent stages (such as `Reduction::Absorption`, following the same unclamped
  recording, `max(damage - modified, 0)` remaining damage, `max(hearts - damage.min(modified), 0)` absorption pool
  deduction, and per-mod panic-disabling semantics as `living_incoming_damage`).

- `living_damage_post` (NeoForge's `LivingDamageEvent.Post`) observes every hit that passed `living_incoming_damage`
  and the cooldown, at the end of `hurtServer` (after the sounds and that hit's death or totem protection),
  before the hurt caller resumes, even for a hit that took no health (`container.health_damage()` 0).
  The next hazard, poison application or rider's hit sees the preceding hit completed; no callback waits for
  the next tick or turn exit. A bee's drowning hurt finishes before stinger expiry is checked,
  and an expiry hurt finishes before nectar and anger bookkeeping and the server AI controls.
  Post lends a read-only live `DamageContainerView` (no snapshot conversion). Its `container.should_cause_side_effects()` is `hurtServer`'s
  fresh-hit decision (`shouldCauseSideEffects`), taken before the damage: true for a fresh hit, false for a stronger hit
  inside the invulnerability window, and true for a source that bypasses the cooldown. The flag is not inferred from
  final damage or invulnerability ticks, so a damage listener changing those leaves it. It accompanies the reductions
  and damages and describes the decision to cause fresh-hit effects, not whether every effect occurred.

A player's damage type is the source's own. A mob's and the dragon's come from the kind of damage Sunrise keeps for
the hit, which doesn't tell every vanilla type apart yet:

- A trident's hit on a mob is `minecraft:arrow`, not `minecraft:trident`
- A wither skull's hit on a mob is `minecraft:magic`, not `minecraft:wither_skull`
- Evoker fangs on a mob are `minecraft:magic`, not `minecraft:indirect_magic`
- A fireball nobody shot is `minecraft:fireball`, not `minecraft:unattributed_fireball`
- A shulker bullet's hit has no `minecraft:mob_projectile`
- A bed or respawn anchor blast on a mob is `minecraft:explosion`, not `minecraft:bad_respawn_point`
- A blast from TNT a player lit is `minecraft:explosion` on a mob, not `minecraft:player_explosion`
- A bed or respawn anchor blast on the dragon is `minecraft:explosion`, not `minecraft:bad_respawn_point`
- A blast a mob caused (an end crystal a skeleton shot, a creeper's) is `minecraft:explosion` on the dragon, not
  `minecraft:player_explosion`

A registered damage type Sunrise keeps no kind of its own for keeps its id and its tags on a mob and the dragon: a
datapack's `demo:heat` hurt through `ApplyEffectTickEvent::hurt` or `entity_hurt` reaches the hurt hooks as
`demo:heat`.

The causing and direct entities are 0 for now in these cases:

- `magic()` dealt to a player during a projectile hit (an ownerless wither skull, instant harm from a
  tipped arrow): vanilla's source names no entity
- A player's causing entity, except for a mob that shot the projectile, the mob `/damage` names, an evoker that summoned
  the fangs, a slime or magma cube that touched the player, and a mob that exploded (a sulfur cube's is guessed until
  its blast names its source)
- A mob's causing and direct entities when a player is behind a hit outside a scoped attack (for example TNT
  a player lit); a projectile's hit does run inside its owner's attack
- A mob's direct entity when it isn't the causing one outside a projectile hit (for example a TNT blast)
- A falling block on a mob, whose causing and direct entity is the block in vanilla
- The dragon's direct entity for a blast
- The dragon's causing player when it isn't the acting one on a LAN guest's turn
- `/damage` on a mob `by` an entity with a damage type that isn't melee: the direct entity is 0, not that entity

And `/damage` on a mob `by` one entity `from` another with a melee damage type gives the `from` entity as the direct
one, where vanilla gives the `by` one.


- `BlockEntityRef::sync(host)`, which sends the holder's block and its update tag to clients again
  (`ServerLevel.sendBlockUpdated`): with the level's block changes, to the players who see its chunk, once per position
  however often it is called in a tick. Works during the holder's own tick and in parked levels; returns
  `LevelWriteError::Stale` after replacement or unload. A change made in a block entity's tick reaches clients at the
  start of the next tick, as in vanilla. Call it when what clients show changes, not for every tick's progress: a menu's
  data slots carry that.

- `entity_leave_level(dimension, entity, kind)`, an entity leaving a server level (NeoForge's
  `EntityLeaveLevelEvent`, vanilla's `onTrackingEnd`): removed for any reason or its chunk unloading, not the entities
  still loaded when the server stops. It only hears. It tells item entities, experience orbs and mobs (`kind` is the mob type) so far; check `kind`.

- Fluid tanks on a mod block's block entity, in millibuckets (a bucket is a thousand). `r.block_fluids(&block, tanks)` registers `ModFluidTanks`, as `r.block_container(&block, container)` registers slots: each tank's capacity, the
  tanks each face reaches, and which a fill may reach and a drain may take; the tanks' contents live in the entity's saved compound under
  `fluids`, so they survive restarts, and the block's own tick reads them there. Other blocks and mods reach them, and a
  cauldron's, through the [`FLUIDS` capability](../concepts/capabilities/block-fluids.md).

- A content effect's own behaviour belongs to its [`MobEffectPart`](../how-to/entities/effect-ticks.md), not global listeners:
  `MobEffectPart::on_effect_added` (`MobEffect.onEffectAdded`, version 2) runs after `LivingEntity.addEffect` inserts an instance of that effect the entity did not already have (never on reapplications, `forceAddEffect`, load or sync), and `should_apply_effect_tick_this_tick` / `apply_effect_tick` own its tick cadence and action.
  The four global `MobEffectEvent` hooks cover every effect, vanilla's included, and all name their entity as a `LivingRef` (the player's own included): `mob_effect_applicable`
  (NeoForge's `MobEffectEvent.Applicable`, vanilla's `canBeAffected`) answers whether an effect can land — the first
  mod to answer `Allow` or `Deny` overrides the vanilla tag check, and an unanswered hook falls
  back to it; `mob_effect_added` (NeoForge's `MobEffectEvent.Added`, version 2) hears every accepted
  addition pre-mutation with `old_effect`, `new_effect` and optional `source` (including reapplications
  and unchanged updates, non-cancellable); `mob_effect_expired` (`.Expired`) hears one count down to nothing — cancelling
  refuses the expiry, and the effect stays, kept at no ticks and asked again next tick;
  `mob_effect_remove` (`.Remove`) hears one about to be removed by any path — a command, milk or a
  mod's removal helper — and cancelling keeps it.

- Chunks and players coming and going, on the server: `chunk_load` (with whether the chunk is new), `chunk_unload`, and
  `player_logged_in` and `player_logged_out` (version 2), which come in pairs for network players however they leave, the server stopping included. A kicked fake also produces logout without a preceding login; silent `remove_fake` produces neither. Changes made to the departing chunk in `chunk_unload` aren't saved: as in vanilla, the chunk is saved before it unloads.

  One tick's streaming step (`update_world`) runs in this order:

  1. The chunks that come and go are worked out. A chunk saved while that is planned is copied for its save there.
  2. The arriving chunks join the level and the departing ones leave it.
  3. Each departing chunk that changed since its last save is copied, with its block entities and save data, and the
     copy is queued to save.
  4. `chunk_load` runs for each arriving chunk (`native_chunks_loaded`).
  5. `chunk_unload` runs for each departing chunk.
  6. The departing chunks' entities, block entities and block names are dropped.

  So an edit a `chunk_load` listener makes to a chunk departing in the same tick (a neighbour's block entity,
  say) comes after that chunk's save copy (step 3): it is not saved, and step 6 drops it. That is the difference from an
  ordinary load, where the edit lands in a chunk that stays and is saved with its next save, and it is the same rule as
  `chunk_unload`, whose edits also come after the copy. Change a chunk you may lose from a later tick instead,
  after checking it is still loaded.
- Sounds the server plays to its players: `native_play_level_sound_at_position` and `native_play_level_sound_at_entity`
  (NeoForge's `PlayLevelSoundEvent.AtPosition` and `.AtEntity`, in `ServerLevel.playSeededSound`). As in vanilla, a mob's own
  sounds (`Entity.playSound`) are at its position; only `playSound(except, sourceEntity, ...)` sounds are at an entity: vanilla's
  sites go through `Game::play_bound_sound` (the mobs' through `MobEvent::BoundSound`), which queues them for the end of the
  tick, after the tick's other sounds; the goat horn and a mod's bound sound ask directly. Two sites keep a difference: dye on a
  sheep and an allay's sounds reach the player who caused them too, since Sunrise's client plays them from the packet rather
  than predicting them. The packet stays positional (horizon-rust/horizon#9284). A mod may change the sound, its source, volume and pitch, or cancel it. Every
  server sound packet but `/playsound`'s goes through `Game::send_level_sound` (`game/interface/level_sounds.rs`), or the native-sound
  flush for a mod's own, so a new sound site asks the mods by calling it. The tick's sounds are asked as they go out, at the end
  of the tick. With no listener, the cost is one `listened` check per sound and nothing is allocated.
- Chunks a player watches, on the server: `chunk_watch(player, chunk)`, `chunk_sent(player, chunk)` and
  `chunk_unwatch(player, chunk)` (NeoForge's `ChunkWatchEvent.Watch`, `Sent` and `UnWatch`). Each chunk that comes into a
  player's view is watched, then sent; it's unwatched when it leaves the view, when the player leaves the level (a portal, a
  teleport, a respawn) and when they disconnect, in the level it was watched in. Vanilla watches a chunk when it queues it; Sunrise
  picks a chunk only as it sends it, so the watch comes just before the send. The hub records them only while a mod listens and
  the game tells them after the hub's network step for that level, so a handler sees the state after that step; the order per
  player and chunk is vanilla's. The server stopping doesn't unwatch.
- A chunk's ticket level changing, on the server: `chunk_ticket_level_updated(chunk, old, new)` (NeoForge's
  `ChunkTicketLevelUpdatedEvent`). The level is the chunk's loading level from its tickets (a player, a forced chunk, a portal
  ticket, the dragon fight's arena), on `ChunkLevel`'s scale: 31 and lower entity ticking, 32 block ticking, 33 loaded without
  ticking, and `ChunkTicketLevel::Unloaded` past it, so a non-ticking or unloaded chunk is never clamped to the ticking range.
  Sunrise keeps no level for a chunk partway through generating, and no chunk holder to hand over. The level's tickets keep each
  change, in the order they were made, only while a mod listens: with no listener an update of the tickets looks at no chunk
  and allocates nothing, so the tick costs what it did. A level is told in its own tick, after the tickets were updated for its players.

- [the renderer's functions](#render-functions): `world_drawn`, called once the world is drawn with its colour, depth, matrices and entity geometry, and
  `render_needs`, where a mod asks for the entity runs and the world's depth without a shader pack.

- [entities](#entities) for server mods: `LevelRef::summon(host, kind, pos, nbt)` spawns any entity from its compound
  as `/summon` does.

- `item_bar(stack, bar)`: the small bar `GuiGraphicsExtractor.itemDecorations` draws under an item,
  wherever the game shows one. `bar` arrives holding
  vanilla's choice (the damage bar, or a bundle's fullness, already applied); set it to a signed pixel width and an `rgb`
  colour, or `None` to hide it; the endpoint wraps as a Java `int` (version 2). Asked each frame a stack is drawn. Client-side (NeoForge has no event for
  this; vanilla's `Item.isBarVisible`/`getBarWidth`/`getBarColor`).

- Mods' own sound events in `play_sound`: an event of another namespace than `minecraft` (`mymod:machine_hum`) is sent to players by
  name, as vanilla sends a sound that isn't in its registry, and they hear it when their resources define it (a content mod's `assets/mymod/sounds.json`
  and its `.ogg` files). `minecraft:` events must still be registered.

- Nicknames for server mods: `player.set_nickname(host, Option<&str>)` shows a name in place of a player's in chat (their messages, `/me`, `/msg`) and the Tab list, the real name on hover and everywhere else. Obtain a `PlayerRef` from `host.players().server_players()` or `.acting_player()` and retain that value, not the borrowed service. The handle is tied to its world epoch and entity identity: nickname writes refuse stale or offline players, but can reach a connected player displaced by another player's turn. A nickname already set persists across reconnects until changed or the server stops.

- Tag membership uses `host.tags()` or `level.tags()` in a restricted callback. Resolve a `TagKey` once through `Registrar::tag_key` (or `tags.key(&id)`), then use `oak_log.is_in_tag(tags, logs)` (`BlockState.is(TagKey)`), `fluid.is_in_tag(tags, water)` (`FluidState.is(TagKey)`), `item.is_in_tag(tags, tag)` or `host.item_stack_in_tag(&stack, tag)` (`ItemStack.is(TagKey)`). The world's merged tags include data packs and content mods; an unknown tag is `Some(false)` and an empty stack tests `minecraft:air`. Clients read the tags their server sent. Until the authoritative tags are loaded, membership reads return `None`. `Tags::items`, `blocks` and `fluids` list members in vanilla's order; see the [registry-key contract](../concepts/registry-keys.md#item-values-and-tag-names) for missing, empty and unavailable tags.
- State and fluid facts use `let states = host.block_states();` (with `HostBlockStates` in scope) or `level.block_states()` in a restricted callback. `state.outline(&states)` borrows the local outline boxes (`BlockState.getShape` with an empty getter at the origin), empty for air. `state.fluid_state(&states)` reads the state's fluid (`BlockState.getFluidState`), including water for a waterlogged state. Both return `None` for an unknown state or unavailable tables. `level.fluid_state(host, pos)` on `LevelRef`, or `level.fluid_state(pos)` on `LevelHost`, keeps the position read (`BlockGetter.getFluidState`): `None` in an unloaded chunk or outside the build height.
- Retain fluid keys from `Registrar::fluid_key` (or look up a runtime name with `host.fluid_key(id)`). `water.is_same(&states, flowing_water)` (`Fluid.isSame`) equates a source and its flowing form and returns false if either key has no world form. `fluid.bucket(&states)` (`Fluid.getBucket`) returns its bucket item key (for a mod fluid, tank-only or world, the `bucket_item` it names unless missing or another fluid's game bucket); `item.bucket_fluid(&states)` (`BucketItem.content`) is its inverse for the buckets the game registers (water, lava, and a mod world fluid's generated bucket). `fluid.pickup_sound(&states)` (`Fluid.getPickupSound`) and `fluid.empty_sound(&states)` (NeoForge `FluidType.getSound(SoundActions.BUCKET_EMPTY)`) read bucket sound events. The bucket and sounds are `None` for `minecraft:empty` (whose vanilla bucket item is air) and for a tank-only fluid that names no bucket.

- A block part's `is_pathfindable` changes whether a path of a `PathComputationType` (`Land`, `Water`, `Air`) may go through the block's states (`BlockBehaviour.isPathfindable`); `event.pathfindable` starts with the block's own answer. The answer depends on the state and the type alone: the game asks once per state and type when parts resolve (at world load and on every `/reload`) and path finding, spawning and the goals read the stored answer, so a part cannot answer from a position, a level or the time. Server-side.
- A block part's `should_check_weak_power` adjusts whether a signal query reads indirect power through that block, for its position and input direction. Block entity neighbour notices use `on_neighbor_change`; `weak_changes` opts the farther block into notices through a conductor (see the block entity neighbours concept page).

- A block part's `should_redstone_wire_connect_to` adjusts whether redstone wire connects to that block for the given position and optional direction.

- A block part's `connects_to` says whether a fence, a wall, or iron bars or a glass pane of that block connects a side toward a neighbour state. `event.kind` names vanilla's rule and `event.connected` starts with its answer: `ConnectionKind::Fence` is `FenceBlock.connectsTo` (a solid face, a fence of the same kind, a gate across the arm), `Wall` is `WallBlock.connectsTo` (a wall, a solid face, bars or a pane, a gate across the side; a connected side is still low or tall from the block above) and `Bars` is `IronBarsBlock.attachsTo`, for iron bars and panes, which are one block class (a solid face, bars or a pane, a wall). Set it to false to deny a connection as well as to true to make one. The neighbour answers first, in its own `can_be_connected_to`, and `event.connected` starts from that answer: the connecting block has the last word. `pos` and `state` are the block being shaped, `neighbour` the state beside it, `direction` the way from the block to the neighbour (vanilla's bars rule takes none) and `face_solid` whether the neighbour's face toward it is sturdy. It is asked on placement and on every shape update toward a side, on authoritative servers only, and only the block's own part answers, so vanilla blocks cost nothing. The three rules stay separate in the library; only the question a mod is asked is shared, and `kind` leaves room for fields one kind alone needs. Diverges from upstream: new hook.

  **Migration (`fence_connects_to` → `connects_to`, contract version 2):** a part that implemented `fence_connects_to(&self, level, event: &mut FenceConnectsToEvent)` implements `connects_to(&self, level, event: &mut ConnectsToEvent)` instead. The event's fields are the same plus `kind`; a part that only answered for fences returns early unless `event.kind == ConnectionKind::Fence`. The answer, its timing and the fence's own rule are unchanged.

- A mod block's comparator output, as vanilla's `BlockBehaviour.hasAnalogOutputSignal` and `getAnalogOutputSignal`: `has_analog_output` says a block of yours has one,
  and `analog_output` gives it (0 to 15) for its state and position. A state change with neighbour updates tells the comparators itself; when the output
  changes otherwise, `level.update_neighbors(host, pos, state, NeighbourUpdate::OutputSignal)` makes them read it again (`Level.updateNeighbourForOutputSignal`).

`Hooks::component_types`, `blocks`, `items` and `standalone_models` take
`&RegistrationContext` before their event; `Hooks::register` takes it after
its `Registrar`. `context.mods()` returns a read-only `LoadedMods` view whose
`ids()` lists the packages enabled and admitted at startup in dispatch order; reloads keep it.
`context.registry("minecraft:item")` borrows a built-in registry's names in ID order as
`Option<&[String]>`, `context.setting(mod_id, key)` borrows a captured setting as `Option<&str>`,
and `context.instance_dir()` borrows the startup instance directory as `Option<&InstanceDirectory>`,
so a mod can validate the keys it declares before its Parts are finalized. All three read one view
the engine captures before the first registration hook runs: every registration step sees the same
registries (as `registries.json` lists them, without what mods declare during registration) and the
settings on disk at that moment, whatever the dispatch order. Missing registries, unset keys, and a
context with no engine snapshot return `None`. These contexts are supplied by the engine and have no
game `Host`, handle constructors or transactions. Declarations still go on each event or registrar.

- `component_types(context, event)` runs once before content loads. Call `event.register(id)` with each explicit `namespace:path`. Duplicates, invalid IDs and collisions with built-in component keys stop startup. `ItemResource::value` reads effective components, including defaults; `ItemResource::with_component` and `ItemResource::without_component` edit detached resources, and `ItemPlace::edit` edits a resident stack. Values use NBT and must fit the network reader limits.

- `create_skull_models`, `emissive`, `map_decoration_types`, `register_entity_spectator_shaders`, `register_fluid_models`, `register_render_pipelines` and `register_texture_atlases` take one entry per `event.register(entry)` call. `block_entity_type_add_blocks` adds memberships with `event.modify(kind, blocks)` or `event.modify_matching(predicate, blocks)`; `event.types` lists registered types, not additions. These registration methods stage entries in listener order. A listener cannot read, edit or remove earlier registrations; a phase refusal discards only that listener's additions. `register_render_pipelines` (version 4) accepts any valid pipeline `Identifier` without a namespace ownership gate. Mod pipeline IDs are unique across required and optional registrations; collisions with vanilla are checked in the matching registry as in NeoForge `RegisterRenderPipelinesEvent`. Fluid models, texture atlases and emissive sprite pairs are collected again on resource reload. Texture atlases reject collisions with vanilla 26.3's built-in engine atlases and duplicate `definition` or `atlas` IDs (first registration stands), without namespace bans or private atlas/sprite count quotas, and isolate overflowing mod atlases against the renderer's `max_texture_dimension_2d` limit.

- `register_layer_definitions(context, event)` (version 5, matching NeoForge `ClientHooks.registerLayerDefinition` and vanilla `LayerDefinitions.createRoots`) binds each layer ID to an owned `Send + Sync + 'static` supplier with `event.register(id, supplier)`. Any `<namespace>:<path>#<layer>` ID with a valid namespaced model `Identifier` before the first `#` and any layer suffix is accepted when it does not collide with an existing vanilla layer definition; duplicate mod registrations overwrite earlier suppliers (last-writer-wins) before freeze, while malformed IDs, vanilla collisions, or an unavailable vanilla layer table fail startup freeze before publishing. Winning suppliers run once at client startup when the layer table freezes. A panicking supplier leaves no definition and is reported in the error log and client error toast naming its mod; other suppliers still run. The game keeps their definitions, not the suppliers. Baking through `ClientHost::bake_layer` and reloading resources reuse the definitions. `register_debug_renderers(context, event)` (version 4, NeoForge `RegisterDebugRenderersEvent`) collects retained callbacks on renderer-list refresh; they run after custom geometry submission and share its deferred context, borrowing `WorldRenderView<'_>` with the frame camera and visible sections. `submit_custom_geometry` (version 2) collects deferred world draws. `prepare_render_buffers` updates same-frame instance groups and prepares lines and quads for active passes. `render_level_stage` (version 4) runs at the actual engine or compatible classic pack boundary and draws only prepared geometry. Sky and level completion lend a view but no pass.

- [explosions](#explosions) for server mods: `LevelRef::explode(host, source, center, radius, fire, interaction, settings)`
  sets one off as `Level.explode` does, with an optional source entity, `ExplosionInteraction::None`, `Block`, `Mob`, `Tnt`
  or `Trigger` for what it does to blocks, and `ExplosionSettings` for its damage type, calculator, particles and sound.

- Dispenser behaviours are chosen at `DispenserBlock.getDispenseMethod`, before vanilla behaviours, on the authoritative server for mod and vanilla items (not droppers). `dispense_method` asks the facing block's Part first, with the dimension, dispenser position, facing, selected stack, and that block's `state` (air included). If it declines, `dispense_item` asks the item's own Part with the same dispenser inputs. An ordinary `None` answer declines; if neither Part answers, vanilla decides. An inert dispatch instead skips the action: there is no slot write, slot notice, or reply feedback. An accepted `Some((remainder, event))` empties the slot for a `None` remainder or replaces it with a valid stack without consuming again. Event 0 is a silent replacement, including the usual slot notice; it is not inert. Events such as 1000 (click) and 1001 (fail) play with smoke. In v4, if a transaction is open in the dispenser's level, reply sound and smoke follow the behaviour's effects at the outermost commit and are discarded on abort. A refused remainder does not replace or notify the active slot, play reply feedback, or run another behaviour. Effects the chosen Part already committed are not undone. Resolve `host.level(dimension)`, then `level.dispense_default(host, pos, facing, stack)` drops one item normally; return its remainder with 1000 to use that behaviour even for items that normally have a special dispense.

- `ClientHost::modifiers()` returns a `ClientModifiers` snapshot of the window's shift, control, alt and super keys, using the same GLFW bits as the input hooks. It is available outside a world and clears on focus loss. Reading it copies the current modifier state; it does not allocate or dispatch hooks.

- `Hooks::render_tooltip_gather_components` (version 4, NeoForge's `RenderTooltipEvent.GatherComponents`, client-side, cancellable): an item's tooltip as one list of `TooltipElement`s — `TooltipElement::Text(Text, u32)` lines and `TooltipElement::Component(TooltipComponent)` entries (the built-in bundle tooltip or `ItemPart::tooltip_image`) — after `item_tooltip` and before wrapping, factory preparation, or measuring. Add, change, reorder or remove elements. Wrapping follows NeoForge's `ClientHooks.gatherTooltipComponentsFromElements` using the item's `ClientItemExtensions::tooltip_font` (default `minecraft:default`): text wraps automatically when the tooltip overflows both sides of the screen, and setting `max_width > 0` (`-1` by default) tightens the wrap width further. Each element is then prepared into a `PreparedTooltipComponent` (custom components via factories registered in `register_client_tooltip_component_factories`; an unregistered custom type aborts that tooltip before `render_tooltip_pre`, `render_tooltip_texture`, or painting with an error log) before `render_tooltip_pre` runs. Cancel (`Flow::Cancel`) and nothing is drawn, and neither `render_tooltip_pre` nor `render_tooltip_texture` is asked.

- `Hooks::render_tooltip_pre` (version 3, NeoForge's `RenderTooltipEvent.Pre`, client-side, cancellable): asked after component preparation and before measurement, positioning and drawing for item tooltips (`Some(&ItemStack)`) and non-item tooltips (`None`, such as a button's, a book's or creative tab names, not the selected-item name a bundle draws beside its grid). `components` lends `&[PreparedTooltipComponent]` in display order, read-only (inspect via `as_text`, `as_bundle` or `downcast_ref`, and measure via `draw.with_tooltip_font(font, ...)`). `place` is the anchor point in GUI pixels before `positioner` (`&dyn ClientTooltipPositioner`, `DefaultTooltipPositioner`) adds its offset and keeps the box on screen; set its `x` and `y` to move it. `screen` is `[screen_width, screen_height]` in GUI pixels. `font` (`&mut Identifier`, starting at `ClientItemExtensions::tooltip_font` for item tooltips or `minecraft:default`) selects the font used to measure and draw every component; replace it to measure and draw with another font without rewrapping already-split text lines. `draw` (`&mut dyn DrawContext`) lets a listener draw custom graphics that stay visible even when the tooltip is cancelled, and provides `text_with_font` and `with_tooltip_font`. Zero-width and zero-height components remain in the list and still run both draw passes. Set `cancel` (`Flow::Cancel`) and the tooltip box and components are not drawn and `render_tooltip_texture` is not asked. Runs each frame a tooltip shows, only while a mod listens.

- `Hooks::render_tooltip_texture` (NeoForge's `RenderTooltipEvent.Texture`, client-side): asked when drawing the tooltip box unless cancelled in `render_tooltip_gather_components` or `render_tooltip_pre`. Set `texture` (`Option<Identifier>`) to draw nine-sliced `<namespace>:tooltip/<name>_background` and `_frame` GUI sprites, or leave `None` to draw solid fills with `background`, `border_top` and `border_bottom` ARGB colours. Runs each frame a tooltip shows, only while a mod listens.

- `Hooks::component_hover` (vanilla's `GuiGraphicsExtractor.componentHoverEffect`; NeoForge has no event for it) is asked for the hover of the text component under the cursor in the chat, a book or a dialog, before the game draws it. `hover` is the typed hover action (`TextHover`: its text, an item or an entity), `x` and `y` the cursor. Set `cancel` and the game draws nothing for it, and a mod can draw its own with `draw`. Left alone, a text hover is split at half the screen's width (at least 200), an item hover draws its stack's tooltip, which asks `item_tooltip` and the `render_tooltip_*` hooks as any item tooltip does, and an entity hover shows its name, type and UUID while advanced tooltips are on. It runs each frame a hovered component shows, only while a mod listens. Diverges from upstream: new hook.

- [advancement triggers](#advancement-triggers) of a mod's own: `player.trigger_criterion(host, &trigger, &mut test)` fires a trigger
  an advancement of a content mod or a data pack names (`mymod:pipe_connected`) for one player, as a NeoForge mod's
  `SimpleCriterionTrigger.trigger`.

- Where a mod block can stay. `can_survive(dimension, pos, state, survives)` answers
  `BlockState.canSurvive` before the block does: as it is placed (`BlockItem.canPlace`: `Some(false)` refuses the spot),
  from the shape update of a block whose template checks it (a plant, a torch, a sign: `Some(false)` breaks it with its
  drops when a neighbour changes), and wherever else vanilla asks. `state` may not be in the world yet, so decide from
  `state`. `may_place_on(dimension, state, ground, ground_pos, may)` answers `VegetationBlock.mayPlaceOn` for a
  mod plant, whose block declaration opts into vegetation or whose template inherits it. The soil's `can_sustain_plant`
  decides first, then the plant; an empty answer keeps the plant's rule. Placement refuses unsupported ground and
  neighbour updates pop the plant off. Answer only for your own blocks. The level is read-only; both run on shape
  updates, so keep them short. With `can_sustain_plant` and `state_for_placement` they are asked on both sides (their docs).

- Drawing mod block entities on the client. `block_entity_submit(pos, block, state, entity, draw)` runs once a frame
  for each mod block entity in view, as vanilla's `BlockEntityRenderer.submit` does (within its view distance, 64 blocks
  unless `block_entity_view_distance` answers otherwise, asked once per mod block and kept, again after a resource reload or a world change; Diverges from upstream: new hook; in the frustum,
  its chunk faded in), borrowing the actual client copy populated by `block_entity_update_tag` (or the screen
  scene's own data). It draws into the entity batch
  under `draw`'s pose stack (`push`, `pop`, `translate`, `rotate_about`, `scale`, in blocks from the block's corner, as
  vanilla's `PoseStack`): `native_draw_quad` (four corners and a sprite's UVs), `native_draw_cube` (a model element's box,
  its faces baked as a model's are, each a `CubeFace::new(sprite)`, `.with_uv(uv)` for UVs of its own), `BlockEntityDrawContext::block(state)` (a resolved `BlockState`'s model, as `submitBlockModel`: a moving part is often a block of the mod's own; resolve static states once through `host.block_states()` rather than parsing property slices each frame) and `native_draw_text`
  (`BlockEntityDrawContext::text(text, at, rgb, glowing, outline)`: a component in the game's font, as a sign's line or a
  text display's, in font pixels with y down as `Font.drawInBatch` lays it out under the pose, with its styles   (bold, italic, underline, strikethrough, obfuscated, fonts, `§` codes), opaque `0xRRGGBB`, in the frame's light or full bright when
  `glowing` (a nixie tube), ringed with `outline` when given; `text_width` measures it, styles included, to centre it).   Parts drawn after `spin` turn on the client's animation clock (`animation_time`) and are lit where they turn to. Sprites are the atlas's:
  `<ns>:block/...` and `block/...`, and a content mod's `assets/<ns>/textures/entity/` as `<ns>:entity/...`. Culling uses the bounds
  `block_entity_render_bounding_box` gives (NeoForge's `getRenderBoundingBox`): the block with a block of margin
  unless the mod widens them, or never culled when a bound is infinite. Both run every frame for every one in view: keep
  them cheap.

- Bone meal on a mod block, the `BonemealableBlock` three questions a block nothing in Sunrise knows one for
  answers itself: `is_valid_bonemeal_target(state, valid)` says whether the block takes bone meal (`Some(false)` refuses it even where its template takes it, `None` leaves the
  template's; asked on both sides, as vanilla asks it, so a client predicts the use); `is_bonemeal_success(state, success)` is
  `isBonemealSuccess`, drawn through the level host's `with_random` to keep vanilla's order; and `perform_bonemeal(state)`
  is the growth itself, with `LevelWriteHost::set_block`/`LevelWriteHost::set_block_property`. The meal is used up and the growth
  particles and sound follow as vanilla's do, whether or not the roll grows the block, from a player's bone meal, a
  dispenser's and, on a crop, a villager's. A block that answers nothing keeps the `BonemealableBlock` its template gives it.
  Authoritative servers only, except the first question.

- Bone meal on any block: `Hooks::bonemeal` (NeoForge's `BonemealEvent`) hears every `BoneMealItem.growCrop`, before
  the block is asked anything: a player's use on both sides, a dispenser's and a villager's. It names the player (none for a
  dispenser or a villager), the block, the `source`, the stack (what the mod changes stays) and `valid_bonemeal_target`, the
  block's own answer. Cancel it and the growth, the block's bone meal Parts and the meal's use are skipped: `growCrop`
  answers `successful` (`setSuccessful`), false unless the mod sets it, so a player's use still tries the water plant
  beside the clicked face. A mod growing only its own blocks answers the three questions above instead.

- Mod-block landing is a block Part method, not a global hook (`block_fall_on` is removed; it diverges from upstream): `BlockPart::fall_on` (`Block.fallOn`) receives writable fall distance and damage multiplier before calculation; a zero multiplier prevents fall hurt and generic fall sounds. Creative and fall-immune entities notify with zero multiplier and retain immunity. Non-finite inputs are ignored; negative multipliers are clamped to zero. `BlockPart::bounce_on` (`Block.bounceOn`) fires only when the entity bounced, just before vanilla sets its movement, and may change the movement the bounce leaves. Both run synchronously inside the entity's move on the level random host (reads blocks, draws the level's random in vanilla's order, attachments; no writes); players and mobs fire them, and dropped items fire `fall_on` (kind `minecraft:item`, zero multiplier, read and act on the item through `ItemEntityRef`; an item's `bounce_on` is not fired yet). An item's fall distance grows by its moves down out of water and resets in water and on landing, as `Entity.checkFallDamage` has it; with no Part on the block it lands on, the landing costs one `fall_listens` lookup. `fall_on` asks for writes with `event.request(LandingWrite::SetBlock(state))` or `LandingWrite::DestroyBlock { drop }` (farmland to dirt after its random draw, a turtle egg breaking): they apply at the batch end, in the order asked, if the block is still the state the entity landed on.
- `Block.entityInside` is `BlockPart::entity_inside(host, event)` (v4) (`EntityInsideEvent { state, pos, entity, precise }`; `precise` is `Entity.checkInsideBlocks`' flag), the Part form of what `block_entity_inside` was; there is no global hook (NeoForge adds no event or extension for `Block.entityInside`). It fires mid-sweep, inside the entity's move, on the level random host (`&mut dyn LevelRandomHost`, like `fall_on`): a Part reads blocks and draws the level's random in vanilla's order, interleaved with the vanilla blocks of the same walk. The Part queues effects and owned callbacks matching vanilla's `InsideBlockEffectApplier.StepBasedCollector`: `event.apply(InsideEffect::…)` queues one of the five `InsideBlockEffectType`s (deduplicated per step), and `event.run_before(effect, |view| …)` / `event.run_after(effect, |view| …)` queue owned callbacks around an effect slot (running even if `apply` was not called for that slot). At sweep flush (`applyAndClear`), queued effects and callbacks run in step and `InsideEffect` order while the swept entity remains alive, each callback lent `&mut InsideEntityView<'_>` for direct entity and live-level reads and writes (`set_block`, `destroy_block`, `hurt`, `hurt_with_source`, `push`, `set_velocity`, `make_stuck_in_block`, `reset_fall_distance`, `set_remaining_fire_ticks`, `add_effect`). There are no encoded write enums, no touched-block state-unchanged guard, and no sibling-write rollback: level writes and source lookups act directly against the live level without exposing a taken-entity refusal, and non-panic refusals (such as an invalid `BlockState`) return `false` every time without latching. A panic in the part call discards its staged effects and closures; a panic during flush leaves completed live writes in place unless an existing transaction journals them. Both use the shared panic boundary: disable only that mod's `entity_inside` callback for the session, log the mod's ID, and queue a client error toast. Other mods' callbacks and built-in effects continue while the entity is alive. Only a block state a Part listens for fires it. A sweep with no queued work takes no action storage; built-ins and listening Parts reuse sweep scratch, growing it only when the retained action or capture storage is insufficient. Players, mobs, items and projectiles all fire it on the shared walk inside their own tick or batch. The block's `getEntityInsideCollisionShape`/`insideBlock` shape that decides "inside" is not exposed (tracked on #44).

- [loot table hooks](#loot-tables), for changing loot a mod didn't make. `loot_table_load(context, id, table)`, in `sunrise-world`'s `loot.rs`, gets each loot table's JSON as it loads, after the data packs' (NeoForge's `LootTableLoadEvent`, Fabric's `LootTableEvents.MODIFY`): add pools or entries, change or replace it, and chests, block and mob drops, fishing and the rest then roll the changed table. It runs on the server for each data load and reload, not client preloads. `loot_drops(context, drops)` gets the items a broken block or a dead mob is about to drop, to add, change or take out (global loot modifiers, `MODIFY_DROPS`), with `LootDrops`: the table rolled, the dimension, the origin, the block or the mob, the player who broke or killed it, the tool and an explosion's radius. It hears blocks a player, an explosion or a mob breaks, not other breaks such as leaves decaying.

- `register_enchantment_effects` binds value and entity effect codecs to the `type` used in a selected enchantment definition. See [A mod's own enchantment effect kinds](../how-to/items.md#custom-enchantment-effect-kinds).

- `screen_render_post`, the point of `ScreenEvent.Render.Post`: a native mod draws over the open screen after it and its tooltips, given its id, the server-opened menu's vanilla type (`menu`) and title, its container panel, its slot count, the mouse in GUI pixels and the partial tick (`ScreenView`). Every container, station and mod window draws on the inventory screen and so reports `inventory`: tell them apart by `menu` and `title` (None on a window the client opened) and by `slots`. It does not fire on the title screen, nor for the prompts, books and sign editors drawn over gameplay after it.

- `native_screen_exclusion()` answers the rectangles (`[x, y, width, height]`, GUI pixels) the open screen keeps clear for the things drawn beside its window: the recipe book's panel, the creative tab rows above and below the window, and the active effects right of the inventory, then every rectangle a mod added in `screen_exclusion_areas(screen, areas)` (a side panel, a button column; a mod that draws over a screen reports where). Compute it once per screen layout and cache it, not per frame: the rectangles move only when the window, the recipe book or the effects change. A mod that lays out panels of its own, as a recipe viewer does, shrinks its rows and columns around them. Empty and non-finite rectangles are dropped. Empty with no screen up. Vanilla has no counterpart; NeoForge recipe viewers take the same from their own API (EMI's `EmiExclusionArea`, JEI's `IGuiContainerHandler.getGuiExtraAreas`). Client-side.

- `native_item_burn_time(item)` answers the item's cooking-fuel burn time in ticks (`FuelValues.burnDuration`: its `cooking_fuel` component, the data pack's context int provider resolved), None where it is no fuel. `native_item_compost_chance(item)` answers the chance the item raises a composter's level (its `compostable` component's layers provider resolved; the `compostables` data map), None where it is not compostable. A mod cannot resolve either from the item's default component, which holds only the provider's name. Client-side, API next.
- `host.screen_hovered_stack()` answers the full stack in the slot under the pointer on the open container or creative screen (`AbstractContainerScreen.getHoveredSlot()`'s stack, its components included), the creative item and search tabs' catalog slots too; None over no slot, an empty one, or with no such screen. A recipe viewer reads it for its look-up keys. Client-side.
- `menu.slot_boxes(host)` (`menu_slot_boxes`) answers the open inventory or creative screen's slot rectangles (`[x, y, width, height]`, GUI pixels) in menu slot order: `AbstractContainerScreen.leftPos`/`topPos` plus each `Slot.x`/`Slot.y`, 16 by 16, so a mod that highlights or searches slots in `screen_render_post` draws where vanilla's own slot highlight does. `None` entries are slots the screen does not show; the whole answer is `None` for a stale menu, another screen, or an inventory covered by the recipe book. Client-side.

- `modify_default_components(context, item, components)`, in `sunrise-world`'s `item_defaults.rs`: an item's default
  components in `items.json`'s form, for any item, vanilla's included (NeoForge's `ModifyDefaultComponentsEvent`). Insert
  or replace a component to set it (`"minecraft:max_stack_size": 16`), remove it to take it away. The item table,
  loot, enchanting and item models all read the changed defaults, and stacks are saved and sent against them as
  against vanilla's. A `minecraft:max_stack_size` above vanilla's 99 is kept, up to `i32::MAX`. Asked for every item
  each time the game reads the defaults: answer the same each time. Client and
  server each ask their own mods, so both need the mod.

- The open screen hears of a mouse button going down (`screen_mouse_button_pressed_pre`) or up (`screen_mouse_button_released_pre`) before it acts, as NeoForge's `ScreenEvent.MouseButtonPressed.Pre` and `.MouseButtonReleased.Pre`: the screen, the GLFW button (0 left, 1 right, 2 middle, 3 back, 4 forward; a button past those is the platform's own number, which differs between systems) and GLFW's modifier bits (shift 1, control 2, alt 4, super 8), with the pointer in `screen.mouse`. A real press also carries `double_click`, true when the last handled press was on this screen instance with this button less than 250 ms ago. A cancelled press skips the screen handler, Post and gameplay mappings, but counts as handled for double-click detection. Cancel a release only when its press was cancelled, or a drag it ended stays held. They cover the game's and title's screens, including dialogs, prompts, books, lecterns, and sign editors drawn over gameplay. They do not fire on the connect and loading screens themselves or for a gamepad or keyboard-focus click.

- [falling blocks](#falling-blocks) for server mods: `level.spawn_falling_block(host, state, pos, velocity, falling)`
  spawns a falling block that flies, lands and places itself as sand does, for debris and collapsing structures.

- Presentation-only view transforms for client mods: `render_view_transform(partial, transform)` runs each
  frame at vanilla's `GameRenderer.bobHurt`/`bobView` stage so mods can compose view matrices (such as camera
  shake scaled by `ClientGuiOptions::screen_effect_scale`) without moving the camera or player.

- Client sounds: `ClientHost::play_local_sound(sound, after)` plays a `SoundPlay`, vanilla's event or the mod's own
  from its `assets/<id>/sounds.json`, for this player only: an interface sound (`SoundPlay::ui`) or one at a place
  (`SoundPlay::at`), immediately at the audio update or after the requested delay, so a client mod can click its own
  buttons or time a blast's sound to its shock wave. Its `SoundRef` follows and controls the individual play.

- Menus of content mods' own menu types (vanilla's `MenuType`): a mod container's `menu` may name a menu type its content mod declares in `data/<id>/sunrise/menu/` with as many slots ([making a content mod](../how-to/content-mods.md#menu-types)). It opens when the block is used, its data slots are the block entity's `data`, and a Sunrise client shows it through the mod's registered screen factory. Server collect hooks `recipe_sources` and `menu_links` name lent ingredient and linked-slot containers; see the menu-types guide for ordering, skips, write-back, uncapped resolution in both, and `menu_links`'s no-listener tick path.

- `block_highlight(pos, state, partial, canceled)` runs before building the eligible targeted block's selection outline, after hidden-HUD and game-mode restrictions. Set `canceled` to suppress only that outline; default behavior is unchanged otherwise. Position is block coordinates and state is the current block state ID. No entity outlines are affected.

- `render_hand(render, draw, canceled)` before each visible first-person hand, including maps, the empty main-hand arm and the empty off hand, where vanilla draws nothing. `render` carries the player UUID and entity id, hand, displayed stack, partial tick, interpolated pitch, light, swing and inverse equip height including swap animation scale. Adjust its translation (blocks), rotation (pitch, yaw, roll in degrees) and scale; the pose is applied in that order before the ordinary hand and item poses. Non-finite or out-of-range components are ignored individually: offsets must be within -16..=16 blocks, angles within -3600..=3600 degrees and scale within 0.01..=100. Set `canceled` to skip default geometry. `draw` draws in the hand, as the submit hooks' draws do: its pose stack starts in the hands' view space (blocks, +x right, +y up, -z into the screen) under the hand sway, before `render`'s pose and vanilla's per-item transforms, and both hands share it (`render.hand` says which one is asked). What a mod drew joins the hand's own vertices ahead of vanilla's, in a run of its own with no rendered item id, as the arm's: it gets the hand's field of view and the view bob, a shader pack's hand pass draws it, `hand_drawn` sees it, and it stays when the hand is cancelled. `HandDrawContext::render_arm` submits either physical arm with the local avatar’s effective skin, model width and that side’s sleeve, under the current draw pose and fixed light. Explicit submission does not fire `render_arm`. So cancel and draw to replace a hand, or draw without cancelling to add to it; it covers a hand that holds an item, where `render_arm` (vanilla's arm only) never fires, and the empty off hand, for a two-handed pose's second arm. Unlike NeoForge's `RenderHandEvent`, the pose stack is `draw`'s own: turning it moves what the mod draws, `render`'s pose moves the default geometry, and player render state is not exposed.

`screen_opening(current, new, outcome)` (`ScreenEvent.Opening`), `screen_closing(screen)` (`ScreenEvent.Closing`) and `screen_init_post(screen, widgets)` (`ScreenEvent.Init.Post`, version 4) take a `ScreenView`, and `screen_init_post` also receives `widgets: &mut dyn ScreenWidgets` (`Screen.addRenderableWidget`) to mount `Button`, `TextField`, and custom `ScreenWidget` children on the screen. Mounted widgets are cleared and rebuilt on every init or resize, drawn in the receiving screen's Pre-guarded contents pass before Foreground and tooltips (and on overlays in their own draw pass), and handled by the engine for hover, focus, unified Tab/arrow traversal, synthetic activation, narration, IME, double-click, and synchronous screen transitions from widget callbacks. Every change of the game's screen (all of the game's screen assignments) goes through `Game::set_screen`, which mirrors `Gui.setScreen`: when a different screen would open, `screen_opening` fires first with the screen that is open (`current`) and the one about to open (`new`; the id `playing` is vanilla's null, no screen), and answers `ScreenOpening::Cancel` (the current screen stays), `ScreenOpening::ModScreen` (the answering mod's own screen opens instead, if it can open there: over the world or the title's screens, including in place of a book or prompt a chat opening would replace, not over a loading world) or `ScreenOpening::Vanilla(ScreenTarget)` (another vanilla screen that takes no arguments opens instead, as it opens on its own: `Pause`, `Options`, `Inventory`, `Advancements`, `Stats` and `Chat` in a world, `Title`, `WorldSelect`, `WorldCreate`, `Multiplayer`, `AddServer` and `DirectConnect` on the title; a target that can't open there, a title screen in a world or a world's screen on the title, keeps the screen asked for and the game logs it); the replacement fires nothing again; the first mod to answer wins, and the dispatch records its id, so a mod can open only its own screen. Then `screen_closing` fires for the screen being left (never for `playing`), and the new screen opens. A repeated assignment of the same screen does not open it again, but a new chat instance does, even with the same screen ID; its Opening and Closing use the actual foreground screen, including a prompt, and Init.Post fires once after acceptance. A veto preserves the foreground and fires no Closing or Init.Post for that request. A game where no mod listens to `screen_opening` or `screen_closing` assigns the screen with no event. `screen_init_post` runs when a screen opens or resizes. The screens of the game's `Screen` list (inventory, containers, pause, options, chat, creative, stats, advancements, mod screens, dead, win…) announce themselves in `set_screen`, before they change, and honour every answer. The overlays drawn over gameplay (sign editors, book edit/sign/view and lectern screens, dialogs, and the command, link, code-of-conduct and server-pack prompts) open through `Game::open_screen_overlay`, which fires `screen_opening` before the overlay is set and honours the answers in the same way (a canceled server-pack prompt leaves its packs unanswered, a canceled code of conduct leaves the server waiting, as in NeoForge), then `screen_closing` for an overlay it replaces. The title's own screens (`world_select`, `world_create`, `world_edit`, `multiplayer`, `onboarding`…) open in the same way: `Title::set_mode` asks for the screen while a mod listens to `screen_opening` or `screen_closing`, and the game fires `screen_opening` (from the title screen shown) and `screen_closing` for the one left before it applies it, after the window event or control command that asked; `Cancel` keeps the screen shown, `ModScreen` opens the mod's own screen over the title (drawn and fed input by the screen hooks as over the world; closing it returns to the title screen), and `Vanilla` opens another title screen. The multiplayer list's own steps (`add_server`, `edit_server`, `direct_connect`, `server_delete`, `friends_join`, and back to `multiplayer`) are title screens too and go the same way. The title appearing as a whole (at startup, when a world closes, after a disconnect or a failed join) and going (when a world opens) goes through `Game::show_title` and `Game::hide_title`: `screen_opening` (from no screen to the title's screen, or back) and `screen_closing` for what it leaves fire before the change, and `screen_init_post` follows an appearance; a `Cancel` doesn't keep the title shut when there is no world to show instead, nor keep it open over a world that opens; an appearing title honours `ModScreen` and `Vanilla`. Still announced by the per-frame observer, through the same function and in the same order but after the change, so a mod's answer can't keep it shut: an overlay's closing. Overlay views use the full window as their rectangle and have no menu or title; anvil rename remains part of its inventory screen. Dialog states share the `dialog` id. A menu swapped inside the open inventory screen is not a change of screen. A server's container close (`ClientboundContainerClosePacket`) that leaves no screen fires no `screen_opening` and can't be cancelled; `screen_closing` still receives the container screen, and a close that opens the title or the death screen fires `screen_opening` for it.

- `client_resources_reloaded()` fires on the render thread after successful client resource-pack application: the atlas, models, renderer and client caches already use the new resources. It covers selection/F3+T and server-pack reloads. Failed staging or application and initial startup do not fire it. This is a completion notification, not a reload-listener registration or staging API.
- `texture_atlas_stitched(atlas)` (NeoForge's `TextureAtlasStitchedEvent`) fires on the render thread once per atlas the resources publish, `TextureAtlas::Blocks` and then `TextureAtlas::Gui`: on the first load before `client_setup`, and before `client_resources_reloaded` on later reloads. The atlas notifications run at the model manager's apply step, before listeners ordered after that anchor. A refused prepare fires nothing, and the old sprites stay right. Sunrise packs vanilla's many atlases into these two: the block sprite arrays (`texture_sprites` answers a `SpriteRef`) and the GUI atlas of GUI sprites, particles and flat item and block textures (`gui_sprite` answers an `AtlasRegion`). Look up the sprites a mod draws here, once, and keep them until that atlas is stitched again; never by name every frame. `TextureAtlas` is `#[non_exhaustive]`. A dedicated server never fires it.

- `client_stopped` fires after the client shutdown finishes. `game_shutting_down` fires after that on clients and after the dedicated server stops, with a side-typed host. Both fire once on orderly process exit. There is no separate `client_started`: `client_setup` covers that moment.

- `client_stopping()` (Fabric's `ClientLifecycleEvents.CLIENT_STOPPING`; NeoForge has no event for it) fires once per game process, on the render thread, when the game shuts down: after the world the player was in is left and saved, with the settings and resources still held, so a listener can read them through the `ClientHost` and save its own client state. Closing the window, a quit shortcut and the stop signals reach it; a crash does not. A dedicated server has no client and never fires it. Listeners are pure functions of the `ClientHost` reads and the mod's own state. Tick-time effect: none, it fires once at exit. Diverges from upstream: new hook.

- `client_setup` (NeoForge's `FMLClientSetupEvent`) fires once per game process on the render thread, after the first client load has applied and before the title screen's first frame. It receives `&mut dyn ClientSetupContext` and `&mut ClientSetupEvent`, not a `ClientHost`; setup can use published resources and client setup operations, but it cannot open worlds or transactions. `client_resources_reloaded` skips initial startup and runs after later reloads; returning to the title or opening another world does not repeat setup. NeoForge's `DeferredWorkQueue` has no counterpart (the listener already runs on the thread that would drain it). A dedicated server has no client and never fires this hook; `server_about_to_start` covers its setup.

- `vanilla_game_event(dimension, event, pos, cause, affected_state, cancel)` (NeoForge's `VanillaGameEvent`) fires on the server for every vanilla game event before the sculk sensors, wardens and allays hear it; canceling keeps them from hearing it. `cause` is the network id of the entity that caused it (`GameEvent.Context.sourceEntity`, None for none) and `affected_state` is the block state the event touched (`GameEvent.Context.affectedState`): the block a player broke or placed, the block a mob stepped on, and None where vanilla passes none (`Context.of(entity)`). Contract version `3`: version `1` had no `affected_state`, and version `3` publishes dispenser fluid-container transfers before inserting or preparing an overflow remainder when no caller-owned source-level transaction is open. With no listener the cost is one `listened` branch per event. Diverges from upstream.

- `get_arm_pose(entity_id, hand, stack, humanoid, pose)` (NeoForge's `IClientItemExtensions.getArmPose`) sets the pose of the arm that holds `stack` in `hand`, as a humanoid model draws it: the window's own player, remote players and humanoid mobs. `humanoid` is a borrowed `HumanoidRenderState` exposing both held stacks, the main arm, use hand, use ticks, use animation and the selected main/off-hand poses before listeners answer (including item declarations). It replaces what the item would give, whatever the use animation, a loaded crossbow or an `arm_pose` declared on the item say (a content mod's data key, see the content-mods how-to, is the answer when no mod listens or none sets one). Return one of vanilla's `ArmPose` values (`empty`, `item`, `block`, `bow_and_arrow`, `crossbow_hold`, `crossbow_charge`, `throw_trident`, `toot_horn`, `spyglass`, `spear`, `brush`); the first mod to answer wins. It is asked once a tick for each humanoid model and non-empty hand, only while a mod listens, and a two-handed pose still moves the other hand as vanilla's does. Custom poses are registered through `Registrar::register_arm_pose` and returned as `ArmPose::Custom`; `arm_pose_transform` writes their API-owned `ArmAngles` in radians. Unknown slots and non-finite angle components are ignored. Mob-specific animations run after the humanoid base pose, as in vanilla. API stack snapshots are converted only with subscribers; there is no allocation when no mod listens. Client-side, with the `ClientHost`. Diverges from upstream: new hook.

- `item_decorations(stack, x, y, draw)` appends custom GUI draws after the default bar, cooldown shade and count. `draw` lends a `GuiCanvas` with the caller's decoration pose and scissor; `x` and `y` are the icon's top-left in that scope, not in the model-only squeeze. Use `draw.scope` to group your decoration draws: returning an error discards that group. On the decorator canvas and its children, `item_decorations` draws the standard bar, cooldown and count without re-entering decorators (hook version 3). Widgets, live scenes, focus and narration belong in ordinary HUD or screen hooks. The shared path covers the hotbar, carried stacks, menu slots, recipe book and item tooltip icons, and the item icons of the advancement, beacon, stats and trading screens. `item_bar` still controls the bar separately; this hook does not suppress vanilla decorations.

- Using an item over time, as vanilla's `Item.getUseDuration`, `getUseAnimation` and `finishUsingItem` and NeoForge's `LivingEntityUseItemEvent.Start` and `.Finish`. `use_duration` answers a stack's use time: a mod's own item with no use of vanilla's that answers above 0 is used over time on a right click, until it is let go or the time runs out, on single player and LAN alike. `use_animation` changes the item's vanilla use animation, including specialised item poses, each first-person frame and each local or remote player model tick; a spear override without kinetic data is refused. `can_eat` decides whether a food can be eaten now. `living_entity_use_item_start` may change a use's duration or refuse it, on the server and where the window's client predicts the use, and `living_entity_use_item_finish` may replace what the hand holds once the time ran out (consumables, potions and mods' items), on the server and on the client that predicts it. Both take the using entity's id, a player's or a mob's, and the hand in use. Change the world only where this game runs the world, not on a server's client.

- Items and block states for drawing mod block entities: in `block_entity_submit`, `BlockEntityDrawContext::block(state)`
  draws an already-resolved `BlockState` (`event.state` or a state resolved once through `host.block_states()`), and
  `native_draw_item(draw,
  stack, display, owner_yaw, seed)` draws an item stack under `draw`'s pose as a campfire's or a shelf's
  (`ItemModelResolver.updateForTopItem` and `ItemStackRenderState.submit`, in the model's `display` transform; with `owner_yaw`, the block
  entity is the item's owner, as a shelf is, so a compass on it points from there). The client's copy of a block entity has
  no item slots: send the stacks you draw in `block_entity_update_tag` and read them from `entity.nbt()`.
  `BlockState::property(metadata, "facing")` reads an owned state's property without a world lookup.
  Host callbacks supply `host.block_states()` as metadata; tint workers receive their explicit
  immutable metadata resource and never need a game host.

- The world's clock for client mods: `native_clock()` is the shown world's game time and time of day in ticks (`i64`; the day time counts on past 24000), none on the title screen.

- The open screen hears of the wheel (`screen_mouse_scrolled_pre`) before it scrolls, as vanilla's `ScreenEvent.MouseScrolled.Pre`: the screen, the scroll's x and y as the game's scroll handler scales them (a line times the wheel sensitivity: with Discrete Scrolling on, a notch is ±1, its sign only; a touchpad's pixels are divided by 30), GLFW's modifier bits (shift 1, control 2, alt 4, super 8) and the pointer in `screen.mouse`. A cancelled turn scrolls nothing: no bundle under the pointer, no list, slider or scrollbar, and no hotbar slot. It fires for the window's wheel (including sunrise-ctl `scroll-at`) on the receiving screen, including the title and overlays, not on the connection or world-loading screen itself, during a closing save, or on controller/focus wheel input.

- Keeping a mod block entity's geometry: `draw.keep()` in `block_entity_submit` keeps what that call drew, and the
  block is drawn the same again without calling `block_entity_submit` or `block_entity_render_bounding_box`
  until its state, its block entity (what the server sent), its light or the resource packs change, so idle machines cost
  no call into the mod each frame. Keep only geometry that reads none of `partial`, `game_time`, the blocks around it or other state of yours:
  kept geometry stands still (an enchanted item's glint, a clock or compass drawn with `native_draw_item`).

- Any command a player types: `player_command(player, name, command, cancel, replies)` is heard before the command is looked at (NeoForge's `CommandEvent`), without its slash, for vanilla's commands, the mods' own and commands the player may not run, once for each command a player sends, on a server. Also each line of a quiet command batch. A mod sets `canceled` to stop it: nothing runs and nothing is answered but the lines it pushes to `replies`, which reach the player as system chat.

- Redstone signals as a block reads them, on a level handle (`LevelRef`) on a server, through the game's own `SignalGetter`, mods' `block_signal` and `should_check_weak_power` parts included: `level.signal(host, pos, direction)` (`getSignal`, how strongly the block at `pos` powers its neighbour on the `direction` side, conducted power included), `level.direct_signal(host, pos, direction)` (`getDirectSignal`, the strong power alone), `level.direct_signal_to(host, pos)` (`getDirectSignalTo`), `level.best_neighbor_signal(host, pos)` (`getBestNeighborSignal`, the strongest power reaching `pos`, as a dispenser or a piston reads it) and `level.has_neighbor_signal(host, pos)` (`hasNeighborSignal`, stopping at the first powered side). Each costs what the game's own call does. They take `&dyn Host` and read the live signal state without a transaction or mutable level borrow. Block signal and weak-power Part callbacks receive the restricted `LevelHost`, so they cannot recursively invoke these signal helpers, just as on the write path. They read the level in place: None (or false) for another level, a stale handle or a client.

- `level.weather(host)` (rain and thunder levels, 0 in the Nether and the End), `level.day_time(host)` (the time of day), `level.height(host)` (min y and height) and `native_max_stack_size(item)`, with `level.brightness(host, layer, pos)` for light and `level.is_raining_at(host, pos)` for rain at a position: what a mod's solar panel, wind turbine, quarry or own container needs without reaching into Sunrise's internals.

- Giving a whole stack (damage, name, enchantments kept) to any player from a server hook: `insert` into
  the player's [`ENTITY_ITEMS`](../concepts/capabilities/entity-items.md) handler places it as `Inventory.add` does, and answers how many went.

- A mod's command executor handles every source of its registered usage. Players retain their swapped-in state; the console has no acting player. Functions are silent, and block/cart feedback follows track-output. Returning `None` fails as an unknown command.

- Player-use drops to `loot_drops`: shears on a sheep, a mooshroom, a snow golem or a bogged, a brush on an
  armadillo or a suspicious block, carving a pumpkin, picking sweet berries or cave vines, and a player taking an item frame or
  its item pass through it, with `LootDrops::used` set and the player named.

- Server experience: `level.visit_experience_orbs(host, min, max, visit)` visits the experience orb handles in a box; `PlayerRef::give_experience_points` adds or removes signed points without Mending, and `PlayerRef::give_levels` gives or, for a negative amount, takes levels.
- Server player death location: `PlayerRef::last_death_location(host)` reads where the player last died as a `GlobalPos`, the dimension's key and the block (`Player.getLastDeathLocation`, saved as `LastDeathLocation`, which the recovery compass follows). The dimension is data: a death in a dimension that is not open still reads, and `host.level(&pos.dimension().to_string())` gives the level when it is. `GlobalPos::new` takes an `Identifier` and `dimension()` returns `&Identifier`, so dimension keys are validated before they enter the typed API. Players resolve as in the other `PlayerRef` reads, so the acting player works too. It is `None` for a player who never died, a stale handle, and a player who is not online or whose record is swapped out. `set_last_death_location(host, location)` stores a location or, for `None`, forgets it (`Player.setLastDeathLocation`); the client hears of it with the player's next respawn, as in vanilla. It returns false in the same cases and for a dead player (so inside `living_death_post` the read works and the write does not).
- Server player rules: `PlayerRef::may_use_item_at(host, pos, face, stack)` is `Player.mayUseItemAt`: a player who may build always may, anyone else only with a stack whose `can_place_on` matches the block `pos` lies against (the check the game's own item use asks), `None` for a player in another level than the one in place. `can_harm(host, other)` is `ServerPlayer.canHarmPlayer`: the `pvp` rule and the two teams (attacks, arrows and tridents ask the same). `respawn_config(host)` reads the respawn point as `Some(Some(PlayerRespawnConfig { position, yaw, forced }))`, `Some(None)` for a player with none (the world spawn), and `set_respawn_config(host, config)` is `/spawnpoint`'s write, `None` forgetting it: the yaw wraps, `player_set_spawn` may cancel (false), and a dead player (except the removed original during `player_respawn_position`), a dimension the game doesn't have and a yaw that is not finite are refused. `text_filtering(host)` is the client's `ClientInformation.textFilteringEnabled` (false for the player running the server); it is the client's own option, so there is no write. The sleep reads are `sleep_ignored(host)` (what `set_sleep_ignored` left), `counts_for_sleep(host)` (the night's count: not a spectator, not a fake, not ignored while out of bed) and `sleep_timer(host)` (`Player.sleepCounter`: 100 once asleep long enough, 101 to 109 on the way up); `set_sleep_timer(host, ticks)` writes the signed counter without entering or leaving a bed. Writes take a `ServerHost`.

- Lead drops to `loot_drops`: untying a lead or cutting it with shears (`used`), and the lead a mob a
  player killed while leashed drops on its next tick, naming the killer.

- Native mod particle types to `native_send_particles`: `ParticleOptions::simple(spark_key)` with a load-resolved `ParticleKey` for `mymod:spark` sends a particle of the mod's own type, drawn by Sunrise clients from the sprite set in the mod's `assets/<namespace>/particles/<path>.json` (`"textures": ["mymod:spark/1", ...]`, resolved as `particle/<texture>` in the atlas). The name must be namespaced; a client without that file draws nothing for it, vanilla clients (whose brand isn't Sunrise's) aren't sent it, and the call still returns true. The type behaves as vanilla's `SimpleParticleType` with a sprite set: a plain billboard, no tint, 20 to 40 ticks, its motion the game's default spread (the speed sets its direction, not how fast it goes).

A mod particle may opt into declarative behaviour in that same particle JSON (no new hook API):

```json
{
  "textures": ["mymod:spark"],
  "sunrise:behaviour": {
    "velocity": [0.0, 0.15, 0.0],
    "velocity_jitter": [0.02, 0.0, 0.02],
    "animation": "loop",
    "frame_ticks": 3,
    "gravity": 0.5,
    "friction": 0.96,
    "lifetime": 40,
    "size": [0.1, 0.0],
    "alpha": [1.0, 0.0]
  }
}
```

`animation` chooses how the particle walks through its sprites: `age` (first to last over its
lifetime), `loop` (cycles every `frame_ticks` ticks, 1..200, only valid with `loop`), `random` (one random
sprite, kept) or `first` (the first sprite, kept). Without `animation` the particle keeps the sprite it was
spawned with.

Every field is optional. Omit `sunrise:behaviour` (or use `{}`) to keep the existing behaviour,
including its random draws, unchanged. `velocity` replaces the initial randomized velocity with
an exact vector in blocks/tick, plus the velocity the spawn call passed; omit it to retain the spawn's default spread. `velocity_jitter` adds a random
offset per axis, uniform in `-j..=j` (0..16 each, drawn from the particle's seeded generator, so the same
seed gives the same particle); declaring it alone starts from zero plus the spawn velocity. Each component must
be finite and within -16..16. `gravity` is the vanilla multiplier: each tick subtracts
`0.04 * gravity` from vertical velocity before moving (negative rises; range -100..100).
After movement, `friction` multiplies all velocity components (0..1); normal block collisions
and the vanilla on-ground horizontal multiplier of 0.7 still apply. Omitted gravity remains 0,
omitted friction remains the engine default (0.98), and omitted lifetime remains 20..39 ticks.
`lifetime` is an integer from 1 to 12000; removal uses the existing base tick: it moves that many
times and is removed on the following tick.

`size` and `alpha` are `[start, end]` linear curves over `age / lifetime`, clamped to their
endpoints. Size is billboard half-width/half-height in blocks (0..16), not a multiplier;
alpha is opacity (0..1). The start value applies immediately at spawn, then the curves update
once per tick, reaching the end after `lifetime` motion ticks. Omitted size keeps the random
spawn size; omitted alpha stays 1. Fading particles use the blended particle layer.
Malformed declarations, unknown behaviour fields, non-finite or out-of-range numbers reject the
whole behaviour object with a log warning, falling back to the existing particle behaviour;
the sprite set still loads. Vanilla particle JSONs are unaffected. Resource reloads apply new
declarations to newly spawned particles; live particles keep their spawn-time values.
The engine copies fixed-size numeric data at spawn and performs no new allocations per particle
tick. Declarations support linear size/opacity curves and constant gravity/friction; for per-particle
code, register a provider with `register_particle_providers` ("Particle providers"). A provider replaces the
declared motion (its tick moves the particle) but the declared spawn values still seed it.

`register_particle_groups(context, event)` binds a namespaced `Identifier` (any nonempty
namespace and path, including `minecraft`) to a factory with `event.register(id, factory)` once
at client startup. Duplicate IDs fail client startup without invoking any factory (matching
NeoForge's `RegisterParticleGroupsEvent`, version 3). The factory captures owned data
(`Send + Sync + 'static`) and returns a `ParticleGroup` with its blend and particle types,
not its ID. The engine calls an accepted factory once per particle engine construction.
A sprite reload resolves particle types against those groups without running factories again.


- the open screen hears of a key press (`screen_key_pressed_pre`) and of each character it types (`screen_char_typed_pre`), as vanilla's `ScreenEvent.KeyPressed.Pre` and `.CharacterTyped.Pre`: the screen, vanilla's key name (`key.keyboard.e`; a key with no name fires nothing), whether the press is an auto-repeat, the character, GLFW's modifier bits (shift 1, control 2, alt 4, super 8), and `screen.mouse` for the pointer. A cancelled press goes no further — no focus move, no Escape-close, no inventory or hotbar key, no typing by the key itself — while its release still arrives and the characters no mod cancelled still reach the text box, as vanilla's separate `charTyped` comes for a cancelled `keyPressed` too. A cancelled character drops only that character and never reaches a text box, and the character hook's `field` is a copy of the focused box whose edits the box takes through its own filter, length limit and responders; control characters (Enter, Backspace, Tab, Delete, Escape, a Control letter) are keys and fire nothing. A key's characters fire after the key has acted, as GLFW's char callback comes after its key callback, so a key that closes the screen (the inventory key) reports no character (the closed screen hears none) and one that moves the focus types into the box it moved to. The input method's committed text fires the character hook too. They fire for the window and the control socket's `key` and `type` (not its `\b` backspace), after the fullscreen, screenshot, debug and narrator keys, on the receiving screen, including prompts, books and sign editors, not on the loading overlay or while a world saves.

- `native_push_player([x, y, z])` on a client, added to the local player's velocity as an explosion's knockback is.

- `data_pack_files(files)`, a native mod's own pack files in the data the server loads: while the tables are assembled (a world's first open, and again on every `/reload`), push `(path, bytes)` and they join that load. `path` is the overlay's key, `<kind>/<namespace>:<path>` without `.json`, over the kinds the tables read: `recipe/mymod:thing`, `tags/mymod:block/thing`, `loot_table/…`, `advancement/…`, `trade_set/…`, `villager_trade/…`, `dialog/…`; a file that isn't JSON is skipped with a warning. Asked once per load on both paths, so it is where a mod recomputes them from its config or the world. Contributed files sit below the world's own data packs: a pack's file for the same path still overrides a mod's, and a later contribution overrides an earlier one.

- Biome colours for client mods, for seasons: `biome_colors(biome, colors)` changes a biome's grass, foliage, dry
  foliage and water tints, asked for every biome when a mod calls `native_refresh_biome_colors()`, which rebuilds the
  loaded terrain with them.
- Hidden blocks for client mods (ore scanners, cutaways): the `block_render_visibility` Part method (`state`, `hidden`)
  makes a state draw nothing: the mesher sees it as air, so its neighbours' faces toward it show and it stops hiding what
  is behind it. Its fluid layer and block entity are hidden too; light, collision and the server are untouched. It is
  asked once per state when the client joins a world and again when a mod calls `refresh_render_visibility()`, which
  remeshes the loaded terrain. Attach it to vanilla blocks with `block_part_tag`. With no part answering, meshing is unchanged.

### Screenshots

`Hooks::screenshot` is the client-thread counterpart of NeoForge 26.3's
`ScreenshotEvent`. It runs with a `ClientHost` after F2's GPU readback and before
any file is written. `event.image` is the in-memory frame (`getImage`): RGBA8,
top to bottom, `width * height * 4` bytes. Change the pixels, or replace the
image, and the normal PNG is that image. A buffer whose length is not
`width * height * 4` writes neither file and shows the translated screenshot
failure line. The HDR copy is not this image; it stays the read-back frame.

Return `Flow::Cancel` to write neither the normal PNG nor its HDR companion.
The chat then shows `event.message` when it is set, and otherwise the translated
cancellation line (`screenshot.failure` with `sunrise.screenshot.canceled`).

Change `event.file_name` to a relative path under the player's `screenshots/`
folder (for example `album/view.png`). Missing parent folders are created.
Absolute paths, empty, `.` or `..` components, backslashes, drive prefixes, NUL
and symbolic links, including the HDR destination, are refused with the normal
translated screenshot failure line. The player may make the screenshots folder
itself a symlink. As with other local file operations, this does not isolate a
native mod or defend against a concurrent external filesystem replacement.

`event.message` starts as `None`: after a successful save the game builds the
normal translated, clickable success line using the final file name. Set it to
`Some(Text)` to replace that success line. Save errors always show the failure,
not a listener's success message. A no-op listener preserves ordinary saving.
With HDR enabled the existing tagged 16-bit copy follows the chosen name as
`<stem>-hdr.png`; its encoding and best-effort error handling are unchanged.
The hook does not intercept control-socket captures or automatic world icons.
It adds no tick work; dispatch runs only when an F2 image has been read back.

### Server data reload listeners

Trait mods implement `Hooks::add_server_reload_listeners` (server only), the counterpart of
NeoForge 26.3's `AddServerReloadListenersEvent`. Registration runs on world open, `/reload`
and `/datapack` changes, after pack selection. The `&dyn Host` beside the event gives registry
access at registration, as `AddServerReloadListenersEvent.getRegistryAccess` does. Register an
`Arc<impl ServerReloadListener>`
with a unique `Identifier` and `FileToIdConverter::json("machines")`, or `::new("machines", ".bin")`
for another suffix. The listener's `ServerReloadContext` lends converted IDs and bytes from
`data/*/machines/`, including a mod's packaged files and later selected datapack
overrides. A file `data/other/machines/heavy/press.json` becomes `other:heavy/press`: the directory
and extension are removed, subdirectories preserved, and all namespaces included. It cannot
read assets, other directories, or filesystem paths. Converter directories have no leading or
trailing `/`, or empty, `.` or `..` components. Converter fields and reload-view fields are private.

The prepare context is the pack view only, as vanilla's `ResourceManager`. Like vanilla's
`SimplePreparableReloadListener`, `ServerReloadListener::prepare` parses and validates without
changing live state and returns a `Box<dyn FnOnce() + Send + 'static>` using
`Box::new(move || { /* publish parsed state */ })`. The action owns its prepared state and the
runner consumes it once on the server's owning thread. All listeners prepare before **any** apply
runs. Do not read another listener's new published state in prepare: dependencies order both
phases but do not remove this barrier. Listener actions cannot report an error. A refusal
while preparing world tables comes before any listener applies. As vanilla's
`SimpleJsonResourceReloadListener.scanDirectory`, a listener that parses its files logs and
skips one that will not parse, carrying the rest through. Returning `Err` from `prepare` — or
introducing a dependency cycle — aborts the whole reload: all pending actions are dropped
without running and the old mod data and vanilla function library are kept. A handled
registration error from `add_listener`, `add_retained_listener` or `add_dependency` returns
`Err(ReloadOrderError)` without mutating or poisoning the registry and is logged each time.
A panicking or phase-refused registration contributes no listener declarations or edges.
A panic in prepare or apply logs the mod and listener, shows the mod error toast on the local
client, and skips that listener for the session without stopping the other applies. The game does
not admit that listener's new retained object. An apply action must publish any private state
it owns atomically: the game cannot undo writes to native mod memory.
Wrap the original error in `ReloadResourceError::new(id, cause)` to keep its source chain;
`resource()` borrows the resource ID and `std::error::Error::source` exposes the cause.
`Display` gives just the resource ID. Reload diagnostics walk the source chain once, marking
a circular reference and stopping there. The log and `/reload` reply name the listener, the
resource and the causes. A
failed `/datapack` change also preserves the selected packs on disk.

`event.listeners.add_dependency(first, second)` orders named listeners. `ReloadOrderError` retains the
IDs as typed variants: unknown names fail immediately without latching a failure, and cycles fail
after registration. `VanillaServerListeners::functions()` names Sunrise's function-library
listener, with no synthetic `minecraft:first` or `minecraft:last` boundary nodes or automatic
boundary edges. This anchor and dependency ends use `Identifier`. Mod listeners without a path to
or from a vanilla listener run after vanilla by default, in registration order. Dependencies
before or after `VanillaServerListeners::functions()` order both the real function-file read and
the function-library admission. Use the named anchor rather than its string value. `event.listeners.get_registry().iter()` borrows the registered names in registration
order, starting with the vanilla anchor. `event.listeners.get_graph().iter()` borrows
`(first, second)` name pairs in edge insertion order. These views borrow the registration in
progress: copy only the IDs you need to keep or use after registering another listener or
dependency. `event.listeners.get_last_vanilla_listener()` always borrows
`VanillaServerListeners::functions()` (matching NeoForge's
`SortedReloadListenerEvent.getLastVanillaListener`); the built-in global loot modifier listener
(`neoforge:loot_modifiers`) is registered before mod hooks without replacing it or adding a
pre-wired edge.
The reads give names, not the listener objects NeoForge exposes (a mod keeps its own listener's
`Arc`, and its name is the one it registered it under). `prepare` returns a boxed `FnOnce() + Send + 'static` action. Sunrise does not expose
NeoForge's condition context here.

`event.listeners.add_retained_listener(&key, converter, listener)` also keeps the listener
object for other hooks, the counterpart of NeoForge's `addRetainedListener(key, listener)`: the
caller builds the typed key — `let key: ListenerKey<MyListener> = ListenerKey::new(id)` — and
reads the object back from any server hook through
`host.reload_resources().listener(&key)` — an `Arc<T>` of the live object the last successful
reload installed, as NeoForge's `ReloadableServerResources.getListener`. Registration rules and
prepare/apply semantics are `add_listener`'s. Retention follows the reload: a new reload
registers fresh objects and the store is replaced whole by each successful reload, so a key
held across a reload addresses the new object unless the owning listener was skipped. A skipped
listener keeps its previous object, when one exists. Before the first successful reload, or
for a key whose id or type does not match a registration, `listener` gives `None`.

```rust,ignore
use std::sync::{Arc, RwLock};
use mod_api::{
    self, AddServerReloadListenersEvent, FileToIdConverter, Host, Hooks, Identifier,
    ReloadResourceError, ServerReloadContext, ServerReloadListener,
};

#[derive(Default)]
struct MachineData(Arc<RwLock<serde_json::Value>>);
impl ServerReloadListener for MachineData {
    fn prepare(
        &self,
        context: &ServerReloadContext,
    ) -> Result<Box<dyn FnOnce() + Send + 'static>, ReloadResourceError> {
        let id = Identifier::parse("my_mod:press").unwrap();
        let next = match context.read(&id) {
            Some(bytes) => serde_json::from_slice(bytes)
                .map_err(|error| ReloadResourceError::new(id, error))?,
            None => serde_json::Value::Null,
        };
        let live = self.0.clone();
        Ok(Box::new(move || *live.write().unwrap() = next))
    }
}
#[derive(Default)]
struct MyMod(Arc<MachineData>);
#[mod_api::sunrise_mod]
impl Hooks for MyMod {
    fn add_server_reload_listeners(
        &self,
        _context: &dyn ReloadContext,
        event: &mut AddServerReloadListenersEvent<'_>,
    ) {
        event.listeners.add_listener(
            Identifier::parse("my_mod:machines").unwrap(),
            FileToIdConverter::json("machines").unwrap(),
            self.0.clone(),
        ).unwrap();
    }
}
```

Reload work only: graph ordering, listing selected files, reading the winning bytes and one
boxed apply closure per listener. No new work or filesystem reads run on the tick path.
The existing post-success server-data notice below is a different moment, not a second
reload registration path.

- `tags_updated(cause)` fires once tags changed (NeoForge's `TagsUpdatedEvent`), on both sides. `TagsUpdateCause::ServerDataLoad { reload }` fires once the server's data tables are built: the recipes, tags, loot and advancements of the world's data packs and of `data_pack_files` are what the helpers read from then on. `reload` is false on a world's first open, before its chunks and players are loaded, and true after `/reload`, once the players were sent the new data. `TagsUpdateCause::ClientPacketReceived { integrated_server }` fires on a client once per set of tags from its server: at login, and for every update in play, on the game thread, with the new tags in the tag helpers. It is where a mod drops caches derived from the data and rebinds what depends on it. In single player both fire, each on its own thread; `cause.should_update_static_data()` is false only for an integrated server's client packet, so data the two share is updated once, on the server. A remote client's packet returns true.

### Client asset reload listeners

Trait mods implement `Hooks::add_client_reload_listeners` (client only), the counterpart of
NeoForge 26.3's `AddClientReloadListenersEvent`. Registration runs when a resource reload is
assembled — the player changed resource packs, pressed F3+T, or a server sent its packs —
after the selection is known and before the reload's assets are staged. The `&dyn ClientHost`
beside the event is the client read host. Register an `Arc<impl ClientReloadListener>` with a
unique `Identifier` and the same converters as the server side
(`FileToIdConverter::json("ui")`). The listener's `ClientReloadContext` lends converted IDs and
bytes from `assets/*/ui/` with resource-pack overrides already resolved: a selected
pack's file wins over the mod's packaged one, and a pack added later wins over both. A file
`assets/my_mod/ui/theme.json` becomes `my_mod:theme`. It cannot read data files, other
directories, or filesystem paths. Files resolve like vanilla resource packs: every file
except `lang/*.json` is delivered as the exact bytes of the winning pack's file, while
language files arrive key-wise merged across the packs, as vanilla loads them.

Prepare and apply semantics, error handling, crash attribution, dependency ordering and the
borrowed registry views match those of [server listeners](#server-data-reload-listeners):
`event.listeners.get_registry().iter()` borrows registered names in registration order,
starting with the vanilla anchor. `event.listeners.get_graph().iter()` borrows explicit
`(first, second)` edges in insertion order, and `event.listeners.get_last_vanilla_listener()`
borrows the vanilla anchor (`VanillaClientListeners::model_manager()`). Copy only the IDs you need
after the borrow ends.
`ClientReloadListener::prepare` parses without changing live state and returns a boxed
`FnOnce() + Send + 'static` action. All prepares finish before any apply; a prepare error or
dependency cycle aborts the reload and drops pending actions without running them, while a
handled registration error leaves the registry unmutated and does not poison the reload.
A panic follows the same named log, toast, and session-disablement path as on the server.
`VanillaClientListeners::model_manager()` names Sunrise's single modeled vanilla client
reload anchor, with no synthetic boundary nodes or automatic boundary edges. Only dangling
non-`minecraft` listeners are linked after the anchor during topological sorting (matching
NeoForge's `ReloadListenerSort`). Dependencies on that anchor run mod
prepares before or after the actual build, and applies on the corresponding side of
model publication on the game thread. First, the atlas uploads fire `texture_atlas_stitched`
with the previous model store (none on startup). `model_baking_completed` fires in the model
manager's apply once the new store is installed. All applies precede `client_resources_reloaded`.
On the initial load, registration uses an empty host because no game exists yet; a host
read not admitted by the phase gate is a refusal. The first load, F3+T, pack changes and server packs all
respect the same prepare/apply ordering. A failed initial prepare refuses the load,
rather than publishing a generation with its listener state missing. NeoForge's client event offers no
retained listeners, and Sunrise has none: the `Arc` the mod registers with is the listener's
own live state.

```rust,ignore
use std::sync::{Arc, RwLock};
use mod_api::{
    self, AddClientReloadListenersEvent, ClientReloadContext, ClientReloadListener,
    ClientReloadListenerContext, FileToIdConverter, Hooks, Identifier, ReloadResourceError,
};

#[derive(Default)]
struct Theme(Arc<RwLock<serde_json::Value>>);
impl ClientReloadListener for Theme {
    fn prepare(
        &self,
        context: &ClientReloadContext,
    ) -> Result<Box<dyn FnOnce() + Send + 'static>, ReloadResourceError> {
        let id = Identifier::parse("my_mod:theme").unwrap();
        let next = match context.read(&id) {
            Some(bytes) => serde_json::from_slice(bytes)
                .map_err(|error| ReloadResourceError::new(id, error))?,
            None => serde_json::Value::Null,
        };
        let live = self.0.clone();
        Ok(Box::new(move || *live.write().unwrap() = next))
    }
}
#[derive(Default)]
struct MyMod(Arc<Theme>);
#[mod_api::sunrise_mod]
impl Hooks for MyMod {
    fn add_client_reload_listeners(
        &self,
        _context: &dyn ClientReloadListenerContext,
        event: &mut AddClientReloadListenersEvent<'_>,
    ) {
        event.listeners.add_listener(
            Identifier::parse("my_mod:theme").unwrap(),
            FileToIdConverter::json("ui").unwrap(),
            self.0.clone(),
        ).unwrap();
    }
}
```

Reload work only: graph ordering, listing selected files, reading the winning bytes and one
boxed apply closure per listener. No new work or filesystem reads run on the tick path. The
existing post-success client-resources notice is a different moment, not a second reload
registration path.

- `compute_fov(partial, camera, degrees)` receives the computed world FOV after settings and ordinary FOV effects, before projection/Nausea. `camera` is its world position and `partial` the interpolation fraction. Mods can narrow or widen the angle; only finite values in 1..=179 degrees are accepted, otherwise the original computed angle is used. It runs for world rendering and near-plane overlays, not just once per frame. Hand/HUD FOV and the title panorama keep their existing paths.

`movement_input_update` (NeoForge's `MovementInputUpdateEvent`) lends the local player's input after sampling, before auto-jump and sprint. `event.input.move_vector` is `[left, forward]`; `event.input.key_presses` holds all seven independent flags, including opposing directions. Vector edits do not rewrite flags, and flag edits do not recalculate the vector. The sprint flag is a key press, so clearing it does not stop latched sprint or veto a vanilla double tap. The server receives the final key flags after auto-jump, not the move vector, and can correct modified movement.

- Mobs drawn by client mods: `render_living_pre(entity, draw, model, canceled)` is asked each frame for every mob and player in
  view, with its id, kind, custom name and the frame's body and head turns and walk cycle; a mod draws a model of its own with the
  block entity drawing helpers under `draw`'s pose (at the feet, turned with the body) and sets `canceled` to skip the rest of its own drawing (its flames and
  shadow still draw).

- A screen of a client mod's own: `ClientHost::mod_screen_open(mod_id)` opens one through the normal `Gui.setScreen` transition path (`screen_opening` and `screen_closing` fire for the transition, and `screen_init_post` runs once open; every mod's own screen has `ScreenView.id` `mod` and `ScreenView.mod_screen == Some(mod_id)`: check `event.screen.mod_screen` to identify the receiving screen in hooks, or `host.mod_screen()` for the currently open screen; it doesn't pause a single-player game; it can replace any open vanilla screen, title screen, overlay such as a book, sign editor, dialog, or prompt, or another mod's screen, running the replaced screen's close cleanup once hooks allow the transition; it returns `false` when cancelled or redirected by `screen_opening`, while a world or connection is loading, or without a client window; a gamepad can close it but not click on it). The screen hooks fire over it as over any other, so the mod mounts widgets in `screen_init_post`, draws in `screen_render_post`, and handles any extra clicks, scroll, keys, or typing in screen hooks, and Escape closes it (`mod_screen()` says whose is open, `mod_screen_close(mod_id)` closes it); and two drawing helpers beside `native_fill`: `native_quad(batch, points, argb)` and `native_item_model(batch, item, model, at, size)`, an icon drawn with another `item_model`.

- `camera_distance(partial, mirrored, entity, distance)` receives the final scaled maximum distance for a third-person camera, before the existing eight visual-shape collision rays shorten it. `entity` is the camera entity view, including optional living scale/distance and mount scale/distance; this covers player and spectator-mob cameras. `mirrored` identifies front-facing third person. Only finite values in 0..=1024 blocks are accepted, keeping mod requests within the collision-ray budget; other values retain the original distance. First-person views and title panoramas do not fire it.

- `compute_fog_color(partial, camera, fog_type, rgb)` adjusts the final world fog RGB after normal biome, weather and effect calculation and before GPU upload. Camera is its world position; fog_type distinguishes atmosphere, water, lava and powder snow. Only finite components in 0..=1 are accepted; invalid requests retain the original RGB. Alpha and all distance fields are unchanged. Title panoramas skip this hook.

- `mouse_button_pre(button, pressed, modifiers, canceled)` sees real button events on any screen, before normal dispatch. Button and modifiers use the screen hooks' GLFW encoding. Physical button bookkeeping already ran. Cancellation skips normal dispatch and Post. Accepted events call `mouse_button_post(button, pressed, modifiers)` after dispatch, including screen handlers' early returns. Synthetic pad/focus clicks skip both hooks. Existing screen-specific hooks remain.

- A player respawning: `player_respawned(player, name, end_conquered)` is heard once at the end of `PlayerList.respawn`, with the replacement placed (`end_conquered` is true after the credits, when health, food and experience are kept; false after death) (NeoForge's `PlayerRespawnEvent`), on a server, with their state swapped in so the helpers reach them.
- Where a player respawns (`PlayerRespawnPositionEvent`, version 3): `native_player_respawn_position(player, original_level, original_position, missing_respawn_block, from_end_fight, level, position, yaw, pitch, copy_original_spawn_position)`, on a server, once the respawn point is found and before `player_clone` or placement. The listener receives the original player and can change the replacement's destination level, position and facing, or whether its respawn point is copied (default: yes, unless the bed or anchor was missing). The destination the listener chooses is not block-checked and the hook does not fire again. Runs for death and after the End credits (`from_end_fight`), never for ordinary End teleports.

- A client mod's view: `native_raycast(distance)` and `native_view_hit()` (the first block along the view, across the loaded chunks).
- A client mod's main-hand input: `register_client_item_use` binds a single `ClientItemUseBehavior` to an item key at startup. It consumes the input before the crosshair target, without a use or release packet or a change to living-item use or blocking. Each callback receives the live typed `ItemStack` from its hotbar slot while the same item kind remains there (rebound on tick and release like `LivingEntity.useItem`, or the last valid stack of that kind if emptied or replaced) and the hotbar slot; `release_using` and `on_stop_using` also receive elapsed held ticks. Releasing the use input (including keybinds that open a screen and release key mappings, as in `Minecraft.handleKeybinds`) calls `release_using`, then `on_stop_using`; a reset (switching slots, replacing the item kind, dying or disconnecting) calls `on_stop_using` alone. Disabled owners are not called. Ordinary item use still goes through `ItemPart::use_item` and `ItemPart::release_using` on both sides.

- `render_fog(partial, camera, fog_type, fog)` adjusts final FogData after normal environment/effect/render-distance calculation and Sunrise's fog option, before GPU upload. Title panoramas skip it. The whole request is accepted only when color components are finite in 0..=1, all distances are finite, environmental and render-distance ends exceed their starts, and sky/cloud ends are nonnegative. Negative near planes remain valid (water/lava). Invalid requests retain the original whole fog.

- `screen_key_released_pre(screen, key, modifiers, canceled)` runs before the receiving screen handles a named window release, a `key ... up`, or the release in a `key ... tap`. Cancelling a physical Pre skips screen handling and Post but still releases the device key and runs mapped keyboard release cleanup. Cancelling a socket Pre skips screen handling, Post and `control_key_up`, so a held mapping can stay down. Cancelling an unhandled Post skips ordinary mapped release on either route, after the screen declined; it does not undo device tracking.

- Music for client mods: `music(music, volume)` changes the track the music manager plays and its volume each
  tick, and `music_layers(layers)` lists looping tracks played over it, each easing to its own volume (stems that
  grow fuller).

- A server's recipes reach its Sunrise clients, as NeoForge's `OnDatapackSyncEvent.sendRecipes` and `RecipesReceivedEvent`: on the server, `native_datapack_sync(player, recipe_types)` takes the recipe types (`minecraft:smelting`, `mymod:crusher`) a mod's client half needs, asked once the joining player exists, after admission and before their recipes are sent; `player` is a required `PlayerRef`. Reload asks once per online player, at most once per player per reload, before re-sending their recipes; with nobody online it is not asked. Configuration tags precede admission; Sunrise sends their final recipes (data packs merged, load conditions held) to that Sunrise client when it joins and after each `/reload`, in payloads within the clientbound cap, and nothing to a vanilla client. On the client `host.recipes_of_type` then gives the server's recipes of those types in place of the window's own, single player included, and `recipes_received(recipe_types)` says they came. Vanilla recipe serializers send resolved item candidates, not native ingredient predicates; mod serializers send their own writer's output. Unsent types retain their already-loaded local recipes. No type asked: nothing is sent and nothing changes; none asked after a load that asked some: the clients go back to their own recipes and `recipes_received` comes with no types.
- `level.recipe_for(host, &recipe_type, &inputs)` matches through the game's crafting, cooking, stonecutting and smithing matchers, including data-pack recipes of those vanilla types. `RecipeInput` has Crafting, Single and Smithing variants. A `RecipeMatch` carries recipe id, typed output, ordered remainders, experience and processing time. Valid non-matching input is `Ok(None)`; malformed input, invalid items and unavailable levels are typed errors. Mod-owned types return `RecipeLookupError::NotMatchable`: cache `recipes_of_type` once per load/reload and match in mod memory, with no per-tick host crossing. Dynamic custom outputs/contextual predicates stay mod-owned. Lookup does not consume inputs; cache the result until the input changes.

- `input_key(key, layout_key, raw_code, action, modifiers)` (version 2) runs after a real window keyboard event finishes handling, including shortcuts, screen consumption and repeats (`InputEvent.Key`). key is vanilla's name for the physical key (`key.keyboard.a`; `key.keyboard.unknown` for one it doesn't name; `getKey`), layout_key its name for the key the layout produced (`getKeycode`: a letter or digit names itself, so AZERTY's key at QWERTY's W is `key.keyboard.z`; a named key keeps the physical name; `None` otherwise), raw_code the platform scan code, or winit's native code for a key it doesn't identify, so unnamed keys stay apart (`getScanCode`; stable within a platform only), action is `KeyAction::Press`, `Repeat` or `Release`, and modifiers use the screen hooks' GLFW bits, captured before handling. Not cancellable. Foreign-window and OS synthetic focus events skip it. Window callback only: pad, focus and control-socket input are not window key events.

- `mouse_scrolling(scroll, accumulated, buttons, cursor, modifiers, canceled)` fires with a player in gameplay and no open screen/prompt/sleeping UI, after accumulation and before hotbar/spectator handling, including when no whole step has accumulated. scroll is x/y after discrete/sensitivity scaling (touchpad pixels normalized by 30), accumulated is the whole x/y pair, buttons are physical left/middle/right, cursor is the GUI position, and modifiers use GLFW bits. Cancel skips gameplay scrolling; accumulated whole steps remain consumed. Synthetic input follows the same gameplay scroll path; screen-scroll hooks are unchanged.

- `screen_mouse_dragged_pre(screen, button, delta, modifiers, canceled)` runs before screen drag handlers for real nonzero pointer movements with a held button. The most recently pressed button is active until it releases; focus loss clears it. Same screen exclusions and GLFW button/modifier encoding as the other screen input hooks. screen.mouse is the new GUI position; delta is movement in GUI pixels. Cancel skips screen drag handlers while retaining the new cursor position. Synthetic input and hover-only moves skip it.

- The screen render stages (`screen_render_pre`, `screen_render_background`, `screen_render_foreground`, then `screen_render_post`) are NeoForge's `ScreenEvent.Render.*`, on the screens `screen_render_post` covers. `Render.Pre` cancel skips the screen's own drawing and with it Background and Foreground; Post still fires. Background fires after the screen's background panel and before its contents; screens with no panel fire it before their contents. Foreground fires after the contents, before tooltips, the dragged stack and the preedit overlay. Lower client renderers report these boundaries to the game, which dispatches with its client host and draw context. `ScreenEvent.Init.Pre` has no hook: Sunrise has no screen init method to veto or skip; `screen_opening` (`ScreenEvent.Opening`) is the veto point. With no mod subscribed to any of the four, nothing is built per frame.
- Screen input Post hooks run on the receiver captured before handling, even if handling closed or switched screens, and never when matching Pre cancelled. Button Posts (`screen_mouse_button_pressed_post` and `screen_mouse_button_released_post`, version 3) run after the mounted widget or screen handler and expose its original answer (`was_click_handled` or `was_release_handled`). Press Post carries the same `double_click` as Pre. Listeners share `result`: `TriState::Default` keeps the original answer, `TriState::True` forces handled and `TriState::False` forces unhandled. The final answer decides double-click history for a press and whether an edge can reach gameplay mappings if no screen remains open. Drag, scroll, `screen_char_typed_post` (v3) and preedit Posts are notices for unhandled input, with no cancellation. Key Posts run for unhandled keys and can cancel delivery to gameplay bindings. `screen_key_pressed_post` (v3) identifies the captured receiver rather than whichever screen is open after the press. The control socket delivers both key phases on `tap`, the release on `up`, and unhandled character notices on `type`.

- The preedit hooks (`screen_preedit_pre`, `screen_preedit_post`) are NeoForge's `ScreenEvent.Preedit.Pre` and `.Post`, fired from the window's input method preedit event on the captured receiving screen while a screen is open. Pre cancel bypasses the screen's handler and Post. A focused vanilla field or editable mounted `TextField` takes the composition and suppresses Post. Custom `ScreenWidget`s have no preedit callback; `captures_input()` sets shortcut priority and does not consume preedit. Post fires for that unhandled composition unless Pre cancels. Committed text goes through `char_typed`. An empty `full_text` is a composition that ended. Nothing fires with no subscriber.
- `screen_render_inventory_mob_effects` is `ScreenEvent.RenderInventoryMobEffects`: once a frame in the survival and creative inventories' effect panels, after vanilla's early-outs (no effects, or under 32 GUI pixels free), with `available_space`, `compact` (vanilla's default is under 120) and `horizontal_offset` to change, and cancel to draw nothing. The answer is a screen-build input: the exclusion areas (`screen_exclusion_areas`) follow it in the same frame, with no lag behind a change, and a second build in one frame reuses the frame's answer rather than asking again; no subscriber leaves vanilla's layout.
- `gather_effect_screen_tooltips` is `GatherEffectScreenTooltipsEvent`: fires for every hovered rendered effect panel beside the survival and creative inventories. `tooltip` starts empty when the panel's information is fully visible, or as the full name then duration for a compact panel or a name cut short. A listener can add a tooltip where vanilla has none, or change, reorder or remove lines; an empty list draws nothing. `effect` is the panel's effect (a `ActiveEffect`). It is separate from `screen_render_inventory_mob_effects`, which picks the layout; hidden panels never dispatch. Screen and effect metadata are built only with a subscriber.


- Knockback a mod can change or cancel through `native_living_knockback` (NeoForge's `LivingKnockBackEvent`): at the
  start of `LivingEntity.knockback`, on the raw strength, before the entity's knockback resistance takes its share and
  its motion changes, with the hit's level host (its blocks, random and own attachments), the pushed entity's id and
  type (`minecraft:player` for a player), the push's `source` (`HurtSource`, named as `native_living_incoming_damage`
  names a hit's, with the same entity ids left at 0; a goat's ram pushes with `minecraft:mob_attack` and hurts with
  `minecraft:mob_attack_no_aggro`, as in vanilla), the `attacker` (the attacking entity's network id, falling back to a
  known last hurt-by mob, or 0 when unknown) and what it holds (`attacker_main` and `attacker_off`: lazily borrowed
  `ItemStackView`s of a player's selected and off-hand stacks or a mob's main hand; none when unknown, empty or
  unrepresentable, recorded only while a native mod is loaded), the pushed entity's `kill_credit` and lazily borrowed
  `kill_credit_main` (`ItemStackView`; unrelated component projection failures never skip the event), the `strength`,
  the direction (`x`, `z`; vanilla points it at the source of a hit, the attacker's side, and the entity is pushed the
  other way) and a `canceled` flag. A mod may change
  the three numbers or set `canceled`, and the push follows: canceled, the entity's motion is left alone. A `strength`
  of 0 or less after the hook is no push, and a direction too short to tell draws a random one after it, as in vanilla;
  the resistance scales the strength the hook leaves, so a full knockback resistance still stops the push. A value a mod
  leaves that is not a finite number is put back as it was. It fires for the players, mobs and armor stands the game
  knocks back: a hit's own push, a sprinting or Knockback hit, a sweep, a spear, a shield's push on its holder and a
  goat's ram. The ender dragon can also receive a default knockback when it is not sitting. The event does not fire
  when `SulfurCube.knockback` launches a cube carrying a block and the source has a causing entity, because it does
  not call the living implementation. Pushes that do not use `LivingEntity.knockback` also do not fire it: the
  dragon's wing push, Punch arrows, explosions, wind charges, a mace smash's area and a hoglin's throw. The hit is in
  the middle of its work: the level host reads and draws random but can't write the entity or the world. It runs on
  the thread that ticks the world (the server thread), so state a mod shares with its render thread hooks needs
  synchronising.


- `LevelRef::edit_blocks(host, edit)` for server mods: runs up columns, from a height or a heightmap's, of typed block states and filters, made at once straight into the chunks of an open server level (active or parked) with what `fill ... strict` does for block entities; the chunks changed go to the players whole.

- Streaming column reads on `LevelRef` (active and parked open levels): `LevelRef::visit_surface_heights` (both sides) and `LevelRef::visit_columns` (server: borrowed `ColumnSlice<'a>` with complete `BlockState` values and biome JSON water colour `water_color`). Client-side blended biome tints are read with `BlockAndTintGetter::block_tint` in block tint callbacks or `ClientHost::biome_tint` in other client hooks.

- `experience_orb_can_merge(dimension, into, from, into_value, from_value, can_merge)` is asked for each pair of orbs whose boxes meet, when an orb spawns (`ExperienceOrb.tryMergeToExisting`; `from` is 0, the new orb is no entity yet) and every 20 ticks as an orb scans (`scanForMerges`); `can_merge` holds vanilla's `canMerge` answer, which a mod may flip either way. `experience_orb_merge` follows a yes and may cancel it (its own notes). A merged orb counts as `count` orbs of the keeper's value. Narrow an `EntityRef` with `host.experience_orb(entity)` to an `ExperienceOrbRef` to read (`value`, `count`, `age`) or write (`set_value`, `set_count`, `set_age`; `set_count` refuses `0` and values above `i32::MAX` with `ExperienceOrbWriteError::InvalidCount`, while `set_value` and `set_age` preserve full live signed `i32` values).

- `block_entity` gives a block a block entity whatever its template: a mod's stone casing or thin part can use any template. The engine removes it when its block is broken or replaced, no piston moves it, and a mod that draws it (`block_entity_submit`) is asked for it.

- Slot rules for registered vanilla and mod menu types: `menu_slot_rules` starts with the slot's own answers and may change both, as a NeoForge mod overrides `Slot.mayPlace` and `Slot.getMaxStackSize`.

- An item's use tick and release: `living_entity_use_item_tick` replaces the ticks left after the may-still-use check. Effects run only above 0, then the counter decrements. On the server, canceling or setting 0 or below finishes the use without release-using; setting 1 finishes in that same tick after effects. A use-on-release item's counter goes negative until release instead. On the client, canceling sets the counter to -1 before decrement and skips effects; the negative countdown continues. `on_use_tick` is what a mod's own item does then (cook, charge). Canceling Start prevents beginning (duration 0 still begins); canceling Stop skips only release-using, while the use still clears. Finish cannot be canceled, but its result stack can be replaced. `release_using` acts with the ticks left; setting `used` starts the item's cooldown. Change the world only where this game runs the world, not on a server's client.

- `player.with_inventory(host, read)` and `player.with_ender_chest(host, read)` lend a live player's inventories.
  Use the matching slot setters on `PlayerRef` for writes. `player.send_map(host, map_id, colors)` sends a picture for
  a filled map with that `map_id`.

- A loot table's custom function or condition uses a load-time codec (`ModLootFunctionCodec` / `ModLootConditionCodec`) registered by id through `Registrar::register_loot_function_type` or `Registrar::register_loot_condition_type`. Each entry decodes once at load/preparation time into a bound `ModLootFunction` or `ModLootCondition`; unknown types or decode errors refuse the load. Bound rules receive a scoped `&mut LootStackEditor<'_>` (functions), borrowed `LootParamsView` (`LootTypeContext::params()`), authoritative tags (`tags()`) and the actual evaluation random (`random()`) without re-reading JSON at roll time. Entry, pool and table order is preserved in both loot evaluators.

- A server's levels opening and closing, on the single player server thread and on a dedicated server: `level_load(dimension)` (v4) when a level opens, before any of its chunks (NeoForge's `LevelEvent.Load`), and `level_unload(dimension)` once it has closed, after its own close completes (`LevelEvent.Unload`). The Overworld fires `level_load` first, then initializes its world spawn if needed. The remaining configured levels follow the `LEVEL_STEM` registry traversal; their relative order is not a promise. All of these loads precede `server_started` (v2), which also observes the completed initial save on an integrated server. That save is outside its listeners’ transaction journals. Unticketed parked non-Overworld levels can evict chunks and entities without firing `level_unload` or a second `level_load` on revisit. Open levels close in creation order during `MinecraftServer.stopServer` after network shutdown, player save/disconnect (each player-file notice requires a successful replacement), and a successful final world save, before `server_stopped` (skipping `level_unload` when the world save fails/panics or that level's own close fails).

- The client's level made and dropped, on the window: `client_level_load(dimension)` when the client makes a level, on joining a server (single player's own included) and on changing dimension (NeoForge's `LevelEvent.Load` from the `ClientLevel` constructor), and `client_level_unload(dimension)` when it drops one, replaced by another dimension's (told after the new one's load, as `Minecraft.setLevel` does) or on reconfiguration, leaving the server, or quitting (`LevelEvent.Unload`). Each load has one unload, before the integrated server is halted on quit. Resolve `host.level(event.dimension)` during either callback; the entering and retiring handles read their own synchronized datapack configurations.

- A projectile's impact, which a mod can cancel, through `native_projectile_impact` (NeoForge's `ProjectileImpactEvent`): it fires when a projectile has found what it meets, before the hit acts, with the projectile's id, type (`minecraft:arrow`) and owner, the level, and what it hit: an entity (its id and type) or a block (its position and the face's normal) and the point. Set `canceled` and the impact does not happen: no damage, knockback, sound or block reaction, an arrow does not stick, a pearl does not teleport, a potion or a fireball does not burst. The projectile flies on if the mod only cancels. Resolve its live handle with `host.entity(event.projectile)` and call `EntityRef::discard` to consume it. A removed projectile cannot apply or replay the hit, even if another listener clears cancellation. It covers arrows, tridents, thrown items, fireballs, wither skulls, wind charges, llama spit and shulker bullets, on the single player server thread and on a dedicated server. Version 2 includes flying fishing bobbers and fireworks, never misses or landing on water. Listeners keeping the earlier family set filter out `minecraft:fishing_bobber` and `minecraft:firework_rocket`. A canceled bobber skips hooking and hit-point redirection but its own move still stops on solid blocks; it notifies before vanilla's entity deflection and world-border bounce. Rockets notify after the deflection decision; cancellation skips their impact explosion, without stopping their normal lifetime. Other projectile families still omit breeze deflections and hits on wind charges. Version 3 checks `projectile_impact` subscriptions directly; with no enabled subscriber the server skips impact preparation and dispatch.

- An entity getting on or off a vehicle, which a mod can cancel, through `native_entity_mount` (NeoForge's `EntityMountEvent`): the rider's id and type, the vehicle's id and type, whether it is mounting, and a `canceled` flag. Destructive removal asks the departing rider to get off first, then asks its direct passengers in passenger-list order. The removed entity no longer has a valid handle or selector result during those calls. Cancellation keeps an edge to a removed boat, cart or mob. A resident Mob retries before its next eligible entity tick, after the tick's cancellable Pre event; a server player retries on its next vehicle tick. A rejected, nonresident passenger does not autonomously retry. Explicit later dismounts ask again. A mob riding a player is compulsorily ejected on save, logout, death or dimension change; the dismount notifies the hook but ignores cancellation. Loading a save does not notify. On the single player server thread and on a dedicated server.

- `interaction_key_mapping_triggered(key, hand, attempt, swing_hand, canceled)` fires while playing when an interaction key is about to act on the crosshair (NeoForge's `InputEvent.InteractionKeyMappingTriggered`): `key` is `InteractionKey::Attack` (when an attack is allowed, and each tick a held key keeps digging a block), `Use` (once for each hand tried, main first, before the hand's target and item) or `PickBlock` (only with a block or entity aimed at). `attempt` distinguishes a queued press from a held mapping. Set `canceled` to skip the action; a cancelled use ends the click, so the other hand gets no turn. Attack and use honour `swing_hand` (true on entry) as a swing request independently of cancellation: false suppresses natural swing requests, cancellation with true requests a swing without acting, and uncancelled true preserves natural no-swing outcomes. A request does not guarantee a new animation: an item can veto it, and an animation already in progress can prevent a new swing from starting during held mining. Pick only vetoes its packet and never requests a swing. The window client on any server, including single player.

- `key_mapping_pre(mapping, pressed, canceled)` is asked while playing, before the game acts on a physical key or mouse button bound to a gameplay mapping (`key.jump`, `key.attack`...), once for each mapping bound to it. `pressed` is true for a press (a key's repeats too) and false for its release. Set `canceled` on a press to drop it for every mapping bound to that input: the mapping is not held, nothing acts on it, and a mouse press is not counted as a click. A cancel on a release is ignored, so what the game holds is always let go. A release is asked too, while playing: a screen that opens lets go of what it holds without asking. A key bound to two mappings (forward and sprint) is canceled as one. It is asked for the window's keys and mouse buttons, agent control's included, after the window's own shortcuts (fullscreen, screenshot, debug keys) and with no screen open. A controller's presses are not asked, nor are `VanillaKeyMappingRef::set_down` and `VanillaKeyMappingRef::click`: they change a mapping's held state or clicks, not a physical key or button.

- `gui_layer_pre` and `gui_layer_post` fire around each dispatched vanilla or mod HUD slot. Paint below the slot in Pre or above it in Post. Returning `Flow::Cancel` from Pre skips that slot's body and Post, including any vanilla bookkeeping and audio in that body: `minecraft:player_health` owns the heart blink and `minecraft:air_level` owns the bubble state and pop sound. Vetoing one slot does not veto its neighbours. `register_gui_layers` installs, wraps or replaces bodies at client startup. A wrapper's `draw_previous()` paints into the same batch and doesn't fire these hooks again; omit the call to skip the previous body. Replacing or wrapping a body keeps the slot's ID, order and dispatch. The status slots fire even under F1, in creative or off a mount; the vanilla visibility checks apply to the original body only, including when a wrapper delegates to it.

- `map_decorations(map, decorations)` is asked each time a map is drawn (held, in an item frame, or on the cartography table's screen) with `MapInfo` (id, centre block, scale, dimension, locked, whether it is in a frame); push `MapDecoration { sprite, x, y, rot, shown_on_frame }` (an atlas sprite, a position in half map pixels from -128 to 127, a rotation 0 to 15, and whether it shows in frames and on the screen like vanilla's markers) and it is drawn after the vanilla decorations with the same pose; a sprite the atlas lacks is left out. Nothing is saved in the map and vanilla's decorations are not passed in. This is the client half of NeoForge's `MapDecorationType`; types registered on the server and saved in map data need code-side registration.

- A player's client starting and stopping tracking an entity, on the single player server thread and on a dedicated server: `player_start_tracking(player, name, entity, uuid, kind)` when the entity is sent to the player (NeoForge's `PlayerEvent.StartTracking`, vanilla's `ServerEntity.addPairing`) and `player_stop_tracking(...)` when it stops (`PlayerEvent.StopTracking`, `removePairing`): out of range, gone from the level, or the player left, changed level or respawned. They name the entity by ID, UUID and type and run with no player's state swapped in, so a mod sends what the player needs with `PlayerRef::send_payload`, and it reaches the player right after the spawn bundle or the removal packet. Each start has one stop. Both are heard in the tick's tracking pass (version 2), once the pass has built every player's packets, because the pass builds them on several threads: a listener sees the player tracking every entity the pass paired, not only those paired before it. A leaving player's stops follow `player_logged_out`; a respawn's or a level change's come at the next pass. While they run the engine is walking its tracker, so a listener reads the game and sends payloads but doesn't spawn, remove or teleport entities.

- An energy buffer on a mod block's block entity. `r.block_energy(&block, buffer)` registers a `ModEnergyBuffer` as `SimpleEnergyHandler` is built: its capacity, the most one insert and one extract moves, and the faces a fill may come through and a drain may leave by. The amount lives in the entity's saved compound as the int `energy`, so it survives restarts and the block's own tick reads it there. The game's own energy capability and registration-time type coverage share that declaration (see "A block's energy").

- Data maps (`RegisterDataMapTypesEvent`). `native_data_map_types(types)` pushes the types a mod registers: a namespaced `id` (`namespace:path`) for a `registry`, any the game names; asked once per load of the server's data (a world's first open and every `/reload`). The values come from `data/<namespace>/data_maps/<registry folder>/<path>.json` (the registry's path for a `minecraft` registry, `<namespace>/<path>` otherwise) in the world's data packs and from files a mod pushes through `native_data_pack_files` under the key `data_maps/<namespace>:<registry folder>/<path>`: `{"replace": false, "values": {"minecraft:coal": ..., "#minecraft:logs": ...}, "remove": [...]}`, an entry's value being the data itself or `{"value": ..., "replace": false}`, optionally with its own `neoforge:conditions`; every entry value is converted to NBT before applying `replace`, inserts or `remove`, so a file with an unconvertible value is skipped atomically with an error log; an id the registry doesn't hold is skipped with one warning per file; `replace` clears what earlier files set, a tag gives every entry in it the value, `remove` applies after the file's own values, and a newer value overrides an older one. Push a typed map (`DataMap`: its id, its entries' registry and how a value decodes) with `DataMap::ty`. A key's `data(host, &map)` returns an owned decoded value. `data_map_value(host, registry, entry, map)` borrows a raw tag, and `data_map_entries(host, registry, map)` borrows the whole slice in entry ID order. Both raw reads borrow from the host; copy only what must outlive that borrow. A point read returns `Ok(None)` when the map gives the entry no value; a known empty map yields an empty slice. Missing publication and an unknown map type are errors, not absence; `DataMapError::Undecodable` is returned when a typed `DataMap::decode` refuses a tag. Server side only and not synced to clients; advanced mergers are not here yet. The six builtin mob and item maps are registered by the engine in the same store with canonical codec NBT values, including Float distances and Int frequencies. This adds no query key families; remaining query coverage is deferred to #4199. `native_data_maps_updated(registry, cause)` (NeoForge's `DataMapsUpdatedEvent`) fires once for each registry that has a type, in registry id order, right after `native_tags_updated`'s `ServerDataLoad`, on a world's first open and after every `/reload`, with `DataMapsUpdateCause::ServerReload`: the place to build caches from the new values. `DataMapsUpdateCause::ClientSync` is for a client applying synced data maps, which does not exist yet.

`living_fall(context, entity, kind, distance, multiplier, cancel)` corresponds to NeoForge's `LivingFallEvent` at `LivingEntity.causeFallDamage`. Mods may change the distance and damage multiplier or cancel fall handling; cancellation does not reverse physical movement. This free function receives the entity ID and type and cannot call game helpers. Player landings, mounted player falls and non-equine mob falls are wired. Equines use their own fall-damage override rather than `LivingEntity.causeFallDamage`. `context` is the loop context of the side that ticks the entity; the hook fires inside the landing, so it takes no `Host` (a deferred call would reorder the fall damage and block effects). `ender_entity_teleport` takes the context the same way, and so does `living_jump`, which fires inside the jump. `farmland_trample` takes the server host in v3 (see its own entry). Contract version `2` for `living_fall` and `ender_entity_teleport`: a mod written for `1` takes the context as a new first argument of the `Hooks` method.

`living_jump(context, entity, kind, velocity)` listens for NeoForge's `LivingEvent.LivingJumpEvent` at the end of `LivingEntity.jumpFromGround`, once per jump of a player or a mob, when the jump power is above vanilla's `1.0E-5` threshold. It receives the entity ID and type and `velocity`, the entity's motion right after the jump (`[x, y, z]`, blocks per tick). Like NeoForge's event it cannot be cancelled; the velocity is read-only (a write to `event.velocity` is not applied), because a NeoForge listener changes the jump by setting the entity's motion, which needs a `Host` this hook does not have yet. Jump height is edited with `living_jump_power`. The player's jump fires before its jump statistic and exhaustion, as `Player.jumpFromGround` calls its parent first; a rabbit's and a sniffer's fire before their own push from standing. The player motion and mob ground-jump paths, including rabbits, cubes and ridden horses and camels, are wired; a mod's simulation of a server player's tick (`native_simulate_player_move`) does not fire it, the server's own jump of a guest does. Contract version `3`: `LivingJumpEvent` gains `velocity`; mods rebuild.

`living_heal(context, entity, amount, cancel)` corresponds to NeoForge's `LivingHealEvent` at `LivingEntity.heal`. The amount can be changed and cancellation prevents the heal. It covers player effect, food and peaceful regeneration healing and the wired mob healing paths. Other healing paths and client-side coverage remain unverified. `entity` is the affected player or mob ID. `context` is the loop context of the side that ticks the entity; the hook fires inside the health update, so it takes no `Host` (a deferred call would reorder vanilla's regeneration timers). Contract version `2`: a mod written for `1` takes the context as a new first argument of the `Hooks` method.

`living_drown(context, entity, kind, is_drowning, damage_amount, bubble_count, cancel)` asks on the server when a player or mob cannot breathe and has no air (NeoForge's `LivingDrownEvent`, `LivingEntity.baseTick`); water animals, axolotls and nautiluses are asked too, from their own air logic, and show no bubbles. Mods may change the drowning decision, damage and bubble count, or cancel. Damage at or below zero does not hurt. The normal eight bubbles use the entity event; other positive counts are sent in a particle packet, and nonpositive counts suppress bubbles.

- A player's or mob's breathing, on the single player server thread and on a dedicated server: `living_breathe(host, event)` every tick for a player and for a mob (NeoForge's `LivingBreatheEvent`, vanilla's `LivingEntity.baseTick`), with Sunrise's answers as the defaults: set `can_breathe` and how much air `consume` takes or `refill` gives (up to `entity_max_air_supply`); at -20 the entity is set to 0 air and takes 2 drowning damage. Version 3 gives the listener the server's host and the entity as a `LivingRef`, so it can read the air (`entity_air_supply`), equipment and blocks, edit items through `ItemPlace::edit` and grant criteria, in one transaction with the air update: if the entity is gone, dead or has other air when the listener returns, the update is refused (the air is never recomputed) and the transactional writes are rolled back; a granted criterion stays granted. Axolotls, nautiluses and `WaterAnimal`s are asked with their own air logic's answers (full air where they breathe, 1 lost elsewhere) and dry out or drown at -20 by it.

- Blocks and items a native mod declares in code, as NeoForge's `DeferredRegister.Blocks` and `.Items` drained by `RegisterEvent`, instead of one content file each: `blocks(context, blocks)` and `items(context, items)`, asked once at start on every side, take `mod_api::Block` and `Item`, the declarations a content mod's `data/<id>/sunrise/block/` and `item/` files hold (a vanilla template and what differs; a block has its item unless `item` is false, in code as in a file). Bare IDs use the declaring mod's namespace; explicit valid keys keep their namespace and cannot replace an existing registry key. They join the content mods' blocks and items, inside the content mod whose id is that namespace when it is on (its models, textures, loot and recipes), else on their own with the template's look, so their network ids, saves and the joining client's content check are the files' (docs/modding/src/how-to/content-mods.md). They pass the files' checks; a namespace whose declarations don't is logged and left out.

- `entity_tick_pre` and `entity_tick_post` observe the entity families described in the hook catalog on both logical sides. Every observed entity is bracketed individually around its own tick (`Pre(A)`, tick `A`, `Post(A)`, `Pre(B)`, ...), carrying one `entity: u32` and `kind: &str`; `entity_tick_pre` returns `Flow::Cancel` to skip that entity's tick work and Post. Family batching and the `entity_tick_each` / `entity_tick_each_entity` opt-in flags are removed. The server's player entity tick is separate from the player connection body. The local client avatar is not observed by these hooks. Version 4 checks `entity_tick_pre` and `entity_tick_post` subscriptions directly with zero preparation when neither is subscribed.

- A mob's, an armor stand's or a player's equipment changing, on the single player server thread and on a dedicated server: `living_equipment_change(entity, slot, from, to)` (version 3) once for each `EquipmentSlot` (`MainHand`, `OffHand`, `Feet`, `Legs`, `Chest`, `Head`, `Body`, `Saddle`) that holds another stack than `LivingEntity.detectEquipmentUpdates` / `collectEquipmentChanges` last saw (NeoForge's `LivingEquipmentChangeEvent`, vanilla's `LivingEntity.detectEquipmentUpdates` / `collectEquipmentChanges` and `PlayerEquipment`). `entity` is a `LivingRef` and `from`/`to` lend `Option<ItemStackView<'_>>` lazily (call `project()` on demand for components or an owned copy). Unchanged entities are compared by borrow without cloning their equipment, only changed slots retain history, and each entity's check runs at its authoritative `detectEquipmentUpdates` site before attribute modifier updates (for connected players inside the server body tick after item use, skipping spectators touching unloaded chunks; for mobs during their body tick after despawn, base tick and item use, before `aiStep` and Post; for armor stands inside `armor_stand_location_effects` with live per-slot re-checks). A mob or connected player new to the level counts from empty on its first body tick. An enabled `living_equipment_change` subscription is sufficient for delivery; changes observed with no subscriber are not replayed. Players connected to the server (a dedicated server's, a published world's guests) are told across all eight slots, with `MainHand` reading the selected hotbar slot (so a hotbar selection or finished item use counts). The integrated server's owner is a connected player too. It only hears.

- `render_player_pre(player, who, draw, model, canceled)` fires once a frame for each other player in view and for this window's player in third person, before its `render_living_pre`, with the same frame (`MobRender`, a `BlockEntityDraw` at the feet turned with the body) and `who` (profile name, whether it is the local player); set `canceled` to skip the player's own drawing this frame (its flames and shadow still draw) (`RenderPlayerEvent.Pre`). `render_living_post(entity, model, draw)` and then, for a player, `render_player_post(player, who, model, draw)` fire once a frame after the vanilla model and layers of every mob and player in view whose pre hooks weren't canceled (but a player whose `render_living_pre` alone was canceled still gets `render_player_post`, with no model), and what the mod adds in that frame is drawn with it (`RenderLivingEvent.Post`, `RenderPlayerEvent.Post`; layers of one type's renderer are `register_entity_layers`, `EntityRenderersEvent.AddLayers`). The draw starts in the entity's frame; the optional model lends the final poses and hierarchy to anyone attaching geometry to bones. The game asks once a frame whether any mod listens to the four, and with none builds nothing for them.

- A player finishing a sign: `native_sign_changed(player, name, dimension, pos, front, lines, cancel)` is heard when a player sends the text they wrote on a sign they were editing, right before it goes on the sign, once the checks have passed, on a server, with the player's state swapped in (`PlayerRef::tell` reaches them). A mod may rewrite `lines` (four lines of plain text) or set `cancel` so the sign keeps its old text; a waxed sign is not told.

- Models no blockstate names, as in NeoForge's `ModelEvent.RegisterStandalone`: `standalone_models(context, event)` runs once at client startup (not on integrated or dedicated servers). Call `event.register_model(id)` for a plain file, or `event.register(id, dependencies, baker)` to assemble declared model files during resource loads. The baker owns its captures (`Send + Sync + 'static`) and may run on a resource worker; the `StandaloneModelBakeContext` is borrowed only for that invocation. Use its `append_dependency(id, quarter_turns, uvlock)` to append quads in call order with the blockstate baker's centered negative XYZ quarter-turns and inverse-face UV locking. A failed `append_dependency` leaves the assembly unchanged and returns `Err`; the baker's returned `Result` controls whether the model is published (`Ok(())`) or discarded (`Err(_)`). A missing dependency or `Err` return drops the result for that load and logs the registering mod, not the source namespace, and the baker runs again on later resource reloads; only a panic disables the callback for the session. A successfully baked file wins over a baker with the same ID; otherwise the first successful baker wins. Resolve each result's `Identifier` through `host.render_assets().model(&id)` in `model_baking_completed`, retaining its opaque `ModelRef` between frames. `BlockEntityDrawContext::model(model, tint)` consumes that handle under the draw pose in `block_entity_submit` or `render_living_pre`, with `tint` (`0xAARRGGBB`) on tinted faces, alpha included; use `0xFFFFFFFF` for no tint. Missing or unbaked names return `ModelError::UnknownModel`. Successful resource replacement invalidates old handles with `ModelError::StaleModel`; failed reloads keep them usable. Drawing errors append no geometry. Lookup never reads or bakes files, and results are quads rather than arbitrary mod-defined baked types.

- A player opening and closing any menu on a server, on the single player server thread and on a dedicated server: `player_container_open(player, name, dimension, menu_type, pos, menu)` and `player_container_close(player, menu_type, pos, menu)` (NeoForge's `PlayerContainerEvent.Open` and `.Close`), with the menu type's name and, for a block's menu, its position. They run with the player's state swapped in. Unlike the viewer count of a mod container (`mod_container_viewers`) they name the player and cover every menu: chests, furnaces, crafting tables, trades, a mod's own. The open is heard synchronously after construction, the open packet is queued and full contents sent, with the new menu already current; failed construction and the player's own inventory are excluded. Even inside one tick, open precedes close; replacement closes the old menu before the new open. Each open has one close, also when the player leaves. `player_container_close` v2 fires after the menu's `removed` cleanup (`MenuPart::menu_removed` for owning mod menus) and before `containerMenu` resets to the inventory menu, passing `player: PlayerRef` and `menu: Option<MenuRef>` with the same support boundary as open. Crafting tables, merchants, workstations, lecterns, and mount inventories have no handle yet.

- A player starting or stopping flight: `player_toggle_flight(player, name, flying, cancel)` is heard once for each change a player's client makes to its flying (vanilla honours it only for a player who may fly), on a server, with the player's state swapped in. A mod sets `cancel` to refuse it: the player keeps the state they had and the server sends their abilities back, so the client lands or flies again. Only what a client asks for is heard, not the flight a game-mode change starts or ends, and not a LAN host's own player.

- `ClientHost::window_size()`, the window's width and height in framebuffer pixels (`Window.getWidth`, `getHeight`), independent of GUI scale: the size `window_resize` reports. It returns no size on a server or when no window exists.


- A player changing dimension: `player_changed_dimension(player, name, from, to)` is heard once after a cross-dimension portal or command teleport lands, with `from` and `to` as dimension ids and the player's state swapped in. It is server-only. Same-level moves and replacement by `PlayerList.respawn` are not heard; use `player_respawned` for death or the return after the End credits.

- `pickup_xp(player, dimension, orb, canceled)` runs before a server player takes an orb (`ExperienceOrb.playerTouch`, NeoForge's `PlayerXpEvent.PickupXp`), only when `takeXpDelay` is 0; with no listener the server skips swapping a guest in for it (the shared `listened` probe). `orb` is the live `ExperienceOrbRef` handle: reads (`value`, `count`, `age`) and writes (`set_value`, `set_count`, `set_age`, or `orb.entity().kill(host)`) act on the live orb immediately so later listeners see earlier edits. `set_value` preserves signed `i32` values (zero and negative values repair nothing and award no XP; a surviving orb saves `Value` as a signed short); `set_count` enforces `ExtraCodecs.POSITIVE_INT` (`1..=i32::MAX`) and refuses `0` or values above `i32::MAX` with `ExperienceOrbWriteError::InvalidCount`. Canceling skips the pickup continuation (`takeXpDelay` stays 0, no take/sound, no Mending/XP award, and no count decrement), leaving any live writes already made to `orb` in place. Otherwise, if the orb is still resident, pickup sets `takeXpDelay` to 2, sends the take packet/sound, repairs Mending and awards remaining positive XP, then decrements `count` and discards at 0. It runs on the integrated and the dedicated server, guests included.

- Whom an experience orb follows (`XpOrbTargetingEvent`): `xp_orb_targeting(orb, scan_distance, following)`, on a server, for each orb as it ticks. `following` starts as the nearest living player within `scan_distance` (8) blocks who isn't a spectator, or `None`; set another player, or `None` to follow no one. The orb goes only for a living player who isn't a spectator and is in its level, and moves only once they are within 8 blocks. An orb that follows no one gets vanilla's push out of a block it is stuck in. Sunrise looks for the nearest player on every orb tick, where vanilla keeps the player it follows while they stay within 8 blocks, so the hook is asked every tick for each orb. Clients still predict the orb's flight towards the nearest player; the server's position wins.

- Where a loot roll happens, for custom loot types and global loot modifiers: `LootParamsView`
  (borrowed from `LootTypeContext::params()` or `LootModifierContext::params()`) exposes `origin()`
  (`LootContextParams.ORIGIN`: a broken block's centre, a dead mob's position), `block_entity()`
  (`BLOCK_ENTITY`: the broken block's entity as it was, saved with its id, position and `Items`),
  and `level()` (a `LootLevelView` lending its dimension, game time, time of day, rain and thunder)
  on a block broken by a player or an explosion, on a mob's death loot, and on rolls that supply a
  level place; `None` when not supplied. The game sets it around a roll with `with_native_loot_place`.

- How a mod block ignites on either side and burns on the server. `fire_odds(dimension, state, pos, face, odds)` answers
  `FireBlock`'s flammability table (`FireBlock.setFlammable(block, igniteOdds, burnOdds)`) before vanilla's table, for the block at `pos` seen from its side `face` (the one facing the fire, or the air fire may take): put
  `(ignite, burn)` in `odds` for your own blocks. Ignite odds above zero let fire stand beside the block and spread
  to the air around it (planks 5, wool 30), so a face with ignite odds of zero isn't flammable; burn odds are how fast fire destroys it (planks 20, wool 60); `(0, 0)` never
  burns; negative answers become zero, burn odds cap at 300 and spread arithmetic saturates. A waterlogged state never burns and isn't asked. It runs hundreds of times per fire tick: keep it short. `is_fire_source`, `ignited_by_lava` and `on_caught_fire` are the block's other fire questions.
  `smoke_source(dimension, state, smoke)` answers `CampfireBlock.isSmokeSource` for the block under a campfire,
  as the campfire is placed or that block changes: `Some(true)` makes the campfire a signal fire, as a hay bale does.
  Answer only for your own blocks and leave the answer empty otherwise; the level is read-only. It is asked on both sides,
  including the client's placement prediction, with a typed `LevelSide`: answer from the state and level alone.

- A server player's game mode about to change: `Hooks::player_game_mode_changed` receives a `PlayerGameModeChangedEvent` before the change (NeoForge's `PlayerChangeGameModeEvent`) for the `/gamemode` command, the F3+F4 switcher and a hardcore world's respawn into spectator, once for each change to a different mode, on a server; not for the default mode a player joins with. Its `from` and `to` are `GameMode` values, the same type used by command parsing and `PlayerRef::game_mode`, `set_game_mode` and `previous_game_mode`. Write `*event.to` to redirect the change, or return `Flow::Cancel` to keep the old mode; numeric ids are only for network and save boundaries.

- The server's difficulty changed, from one of `peaceful`, `easy`, `normal`, `hard` to another, through `difficulty_changed` (NeoForge's `DifficultyChangeEvent`). It is vanilla's `MinecraftServer.setDifficulty`, which `/difficulty`, a dedicated server's management and a test environment's difficulty all go through, and it is told only when the difficulty is another one. It only hears: the change has already happened.

- A player's fishing rod: `player_fish(player, name, state, cancel)` is heard for the cast (`state` 0), a fish biting (1), the reel-in with a catch (2), the reel-in with nothing on the line (3) and the reel-in of a bobber hooked to an entity or item (4), for a player of a server (Bukkit's `PlayerFishEvent`; not NeoForge's `ItemFishedEvent`). A mod sets `cancel` to stop a cast, make the fish get away, or leave the bobber out on a reel-in. The player's state is swapped in.

- A player took a crafting grid's result, through `item_crafted` (NeoForge's `PlayerEvent.ItemCraftedEvent`): the player's uuid, the recipe the grid matched (`minecraft:oak_planks`), the result stack it took, with its components, and the grid's cells row by row before the take used them. It only hears; the take has already happened. It fires for the crafting grid of an inventory or a crafting table, on the integrated server and on a dedicated server, guests included, and not for a stonecutter's, a smithing table's or a furnace's result.

- `native_server_event(name, uuid, message)` posts an `announce` event to a dedicated server's event stream (`GET /api/v1/events`); sunrise-server's Discord bridge relays it, in gold, when `[discord] events` lists `announce` (off by default). False elsewhere and for an empty message.

- `native_state_block(state)` names the block a state belongs to (`BlockState.getBlock()`); a `state_for_placement` part compares it with its own block. On the server, `LevelRef::state_for_placement(host, block, &context)` asks those parts with an already-resolved context, without placing the candidate.

- A recipe ingredient of a mod's own kind asks `ingredient(context, event)` once per occurrence in the owning recipe load, including nested occurrences and slots outside `ingredients`. The first `Some(IngredientDefinition::new(items, test))` wins. The candidate list is deduplicated in first-seen order and kept with the predicate, apart from recipe JSON. No answer refuses the recipe with its file and kind; an answered empty list keeps manual matching without supplying placement candidates. A reload asks again; a remote client does not perform `ServerDataLoad`.

- A frame's two moments, as NeoForge's `RenderFrameEvent.Pre` and `.Post`: `render_frame_pre(host, event)` runs once a frame (in a world or not) after Sunrise's extras and just before the frame is rendered, and `render_frame_post` right after it was rendered; both carry the frame's time as `event.elapsed`, `world_partial_tick` (captured world interpolation, held while paused or frozen), and `partial_tick` (advancing window interpolation). Outside a world both use the window clock. Pre reports the value about to be drawn; post retains the same snapshot even if a listener changes the clock or screen. Client side; a frame with no listener costs one branch.
- Items a mod's own recipe type takes in a station slot. `recipe_property_sets(context, event)` is asked once per load, after the recipes are resolved, with the `DataLoadContext` of the load that owns them (`origin()` `ServerDataLoad` on a server, `ClientPreload` on a client's own load), and again after `/reload`. Call `event.register(RecipePropertyAddition::new(set, items))` once per set to add to (`minecraft:furnace_input`, `blast_furnace_input`, `smoker_input`, `campfire_input`, `smithing_template`, `smithing_base`, `smithing_addition`, `brewing_input`, `brewing_reagent`); the vanilla recipes' items stay in. The furnace shift-click, the smithing and brewing slots and the set a client is sent read the one answer, so the client predicts what the server accepts. Any valid namespaced key creates a custom set (including an unused `minecraft:` key), synchronized to clients alongside vanilla sets. Bare names refer to built-in sets. Later listeners may extend an existing set without changing who created it. `Host::mod_recipe_property_contains` and the same `LoopContext` query take the set key and an `ItemKey`; a mod menu's `menu_slot_rules` reads that generation for both server acceptance and client prediction. These queries cover mod-created sets only, returning false for unknown or vanilla keys. Invalid keys, unknown bare names and air are logged with the contributing mod and skipped; an item the registry does not know is ignored. The answer must be the same every time.

- The in-world HUD's two moments, as NeoForge's `RenderGuiEvent.Pre` and `.Post`: `render_gui_pre(host, event)` runs once a frame in a world, before any vanilla HUD layer, and a mod that returns `Flow::Cancel` vetoes the HUD pass, including the status slots. The HUD chat layer is vetoed, but focused chat draws with its screen. Toasts draw after the active screen or overlay, outside the veto. F1 hides ordinary toasts and audio stream failure notices, but not other callback error toasts. Unlike F1, that veto skips those slots' installed bodies and layer events. Pre is heard under F1 and before Options over a world, but not in replay or while a server dialog or the connection or loading screen covers the window. `render_gui_post(host, event)` runs immediately after the complete slot walk and before screens, toasts, debug overlays and IME; draw through `event.draw` and read earlier layers' clipped extents through `ClientHost::hud_layer_bounds`. Client side; a frame with no listener costs one branch each.
- Lowering the frame rate limit: `framerate_limit(focused, minimized, in_world, limit)` is asked each frame with the limit Sunrise is about to use (Max Framerate after the inactivity throttle and Battery Mode, 260 for unlimited), whether the window has focus, whether it is minimised and whether a world is open. A mod may lower `limit`, for example to throttle a window in the background as Dynamic FPS does; Sunrise keeps the lower of its own limit and the mod's and ignores values below 1.
- The camera type F5 cycles to: `camera_type_cycle(host, event)` runs when `key.togglePerspective` cycles the window's
  camera type (`Minecraft.handleKeybinds`), with `previous` and vanilla's `next` (first person, third person behind, third
  person in front); a mod may set `next` to skip or keep a type, and the camera entity's post effect follows the answer.
  `camera_type()` reads the type and `set_camera_type(type)` sets it as F5 does, without a cycle, for a freelook mod that
  switches to third person and back; with `compute_camera_angles` and `camera_position` overriding the drawn camera, that
  is a camera independent of the player. The game's two copies of `CameraType` are now one, `mod_api::CameraType`. Client side.

- A client tick's two moments, as NeoForge's `ClientTickEvent.Pre` and `.Post`: `client_tick_pre(host, event)` runs at the start of every client tick (`Minecraft.tick`), paused or not and on the title screen too, and `client_tick_post` at its end. They replace the single `tick` hook, which also fired on a server's own `Game`: the server's tick is `server_tick` (`ServerTickEvent`), so a mod that implemented `tick` implements `client_tick_pre` and, for a server, `server_tick`. Window only, client side; a tick with no listener costs one branch.
- How long a client tick lasts, a "timer": `tick_target_millis(host, event)` gets the milliseconds the window's
  `DeltaTracker.Timer` divides each frame's time by (`Minecraft.getTickTargetMillis`), vanilla's 50 at 20 ticks a
  second or the server's slower `/tick rate`, and may set another, once a frame before that frame's client ticks run.
  25 runs the client twice as fast, 100 half as fast, still at most ten ticks a frame; an answer that is not a finite
  number above zero keeps vanilla's. Only the window's clock changes; a server keeps its own rate. Client side; with no
  listener, one branch a frame.

- A player swapping their main and off hand items: `player_swap_hands(player, name, to_main_hand, to_off_hand, cancel)` is heard before the swap (a swap from an open inventory is not heard), for a player of a server who is not a spectator, with their state swapped in. A mod may replace the stack each hand gets, or set `cancel` to keep the items where they are; the player's client is sent both slots again.

- A player's food level changing: `food_level_changed(player, name, from, to, cancel)` is heard for a player of a server when a turn of theirs changed it (eating, hunger and saturation effects, exhaustion, starving; Bukkit's `FoodLevelChangeEvent`), before the new level reaches them. A mod changes `to` or sets `cancel` to leave the level at `from`.

- A server player teleported: `player_teleported(player, name, from, to, dimension)` is heard after `/tp` or `/teleport` has moved a player inside their level or into another one (NeoForge's `EntityTeleportEvent`), with their state swapped in. `from` and `to` are their feet before and after, `dimension` the level they are in now. It cannot stop the move; ender pearls, chorus fruit and a spectator's teleport menu are not heard yet, portals and respawns are not teleports here.

- The command tree a server sends a player: `player_commands_sent` lends the live `PlayerRef` and
  permission-filtered outgoing roots. Use `event.tree.hide_where(...)` to select roots to omit for this send.
  The player's client neither completes nor highlights the hidden commands, but execution is unchanged.

- A joining player the mods may refuse: `player_login(player, name, cancel, reason)` is heard once for each player a server has accepted (name, whitelist, bans and player limit checked), before their saved state is loaded and before they are placed in a level (NeoForge's `PlayerNegotiationEvent`). A mod sets `cancel` to disconnect them with `reason` ("Disconnected" when it stays unset); no join message and no `player_logged_in` follow.

- A server player's sprint: `player_sprint(player, name, sprinting)` is heard once each time a player's client starts or stops sprinting (the sprint key, a double tap forward, letting go) and the server's player has taken it, with their state swapped in. It only hears what the client asks: a sprint that ends by itself (knockback, hunger, a wall) is not reported.

- A resource pack answer: `resource_pack_status(player, name, status)` is heard when a server player's client answers a resource pack in play (`status` is `ServerboundResourcePackPacket.Action`'s number: 0 loaded, 1 declined, 2 failed download, 3 accepted, 4 downloaded, 5 invalid URL, 6 failed reload, 7 discarded), with their state swapped in. The pack offered during the configuration is answered before the player has joined and is not heard.

- A player taking an item from or putting one on an armor stand: `armor_stand_manipulate(player, name, stand_id, slot, cancel)` is heard after the stand's disabled-slot checks and before anything moves, with the stand's entity id and the slot the click reached (`mainhand`, `offhand`, `feet`, `legs`, `chest`, `head`), on a server, with the player's state swapped in. A mod sets `cancel` and neither the stand nor the player's hand changes.

- A player opening one of the game's own menus: `container_opened(player, name, kind, pos, cancel)` is heard when a chest, furnace, crafting table, villager, anvil, lectern, horse inventory... is about to open (the job of NeoForge's `PlayerContainerEvent.Open`, but before the menu opens), after the checks that refuse it, with the menu type and the block's position, on a server, with the player's state swapped in. A mod sets `cancel` and nothing opens. Menus a mod opens (`PlayerRef::open_chest_menu`, `PlayerRef::open_mod_menu`) are not heard.

- A creative player setting a slot: `creative_slot(player, name, slot, item, count, cancel)` is heard when the creative screen sets an inventory slot, or throws a stack out (a slot below 0), after the packet passed vanilla's checks and before the slot changes, on a server, with the player's state swapped in. A mod sets `cancel` and the packet is ignored.

- A boat or minecart moving: `vehicle_moved(player, kind, from, to)` is heard once a tick for each boat or minecart that carries a server player (and for one a server player left, for ten seconds after it stood still), with the vehicle's type and its position before and after (`VehicleMoveEvent`), on a server, with the rider's state swapped in. `player` is the rider's UUID, none for a vehicle that carries nobody now. It only hears.

- A server player emptying a bucket: `bucket_empty(player, name, pos, fluid, cancel)` is heard right before the fluid is placed, once the player may build there. A mod sets `cancel` and nothing is poured. The player's state is swapped in, so the helpers reach them. Filling a bucket is `fill_bucket`'s moment; dispensers and cauldrons are not heard.

- A server player placing a hanging entity: `hanging_place(player, name, kind, pos, cancel)` is heard for an item frame, a glow item frame, a painting and a leash knot (a lead tied to a fence that has none), once it fits and before it is placed or the item used. A mod sets `cancel` and nothing is placed. The player's state is swapped in, so `PlayerRef::tell` answers them.

- TNT about to be primed: `tnt_primed(player, pos, cancel)` is heard before a flint and steel, a fire charge, fire, a redstone signal, an arrow or a dispenser lights the TNT at `pos`, with the player whose click lit it (None when no player did). A mod sets `cancel` and it is not primed: the block stays, or is put back when fire or a signal had already taken it out. The clicking player's state is swapped in. TNT set off by a blast, a TNT minecart and a sulfur cube are not asked.

- `anvil_damaged(dimension, pos, from, to)` runs after an anvil at `pos` changes from `from` to `to`; `to` is air when it broke.

- `beacon_state_change(dimension, pos, active)` is called after the beacon at `pos` gains or loses its complete beam pyramid; `active` says whether it is active now.

- A bell is about to ring: `bell_ring(dimension, player, pos, cancel)` is asked before the bell event and sound. Set `cancel` to stop the ring and its raider reveal. `player` is the acting server player when available.

- `block_break_progress_update(player, name, pos, progress)` is called when a guest's crack animation at `pos` reaches a new stage from 0 through 9.

- Fire about to burn a block: `block_burn(dimension, pos, cancel)` is heard when fire's burnout check is about to remove the block at `pos`. Set `cancel` to keep it.

- A campfire finishes cooking `item` into `result` at `pos`, before dropping the result.

- A dispenser equips `item` on the first eligible player, mob or armor stand in front of its block at `pos`, after the item is consumed and the equip sound and game event are applied. `target` identifies the kind of entity equipped.

- A block with a vanilla experience drop is about to award its points at `pos`; `block` is its identifier. Change `experience` to adjust the amount before the orbs are awarded.

- `block_fade(dimension, pos, from, to, cancel)` runs when ice or snow melts, or coral dies. `from` and `to` are block state IDs. Set `cancel` to keep the old block.

- `block_failed_dispense(dimension, pos, item)` is called when a dispenser at `pos` fails to dispense `item`.

- `block_form(dimension, pos, from, to, cancel)` runs when precipitation is about to place ice or snow, or add a snow layer at `pos`. `from` and `to` are block state IDs. Set `cancel` to keep the old state.

- Fire about to be placed: `block_ignite(dimension, player, pos, cause, cancel)` is heard for fire spread, lava, lightning and dispenser flint-and-steel. Set `cancel` to leave the position unchanged. Direct player flint-and-steel/fire-charge placement is not covered.

- `block_lock_check(player, name, pos, locked)` runs when a guest checks the container at `pos`; `locked` starts with vanilla's result and may be changed. The guest's state is not swapped in.

- A sculk sensor, calibrated sensor or shrieker received vibration event `event` at `pos`; `block` is its identifier. Set `cancel` to suppress the block's response.

- A dispenser sheared entity `entity` of kind `kind` at the block in front of the dispenser at `pos`, after the shearing effects.

- A brewing stand consumed fuel: `brewing_fuel(pos, fuel)` is heard after the stand at `pos` consumed the item `fuel` and gained brewing fuel. Server-side.

- A brewing stand starts brewing after it consumes fuel and sets its timer.

- A system broadcast line is about to be logged and sent to every player: `broadcast_message(line, cancel)` lets a mod rewrite it or set `cancel` to drop it.

- An item was placed on a campfire and began cooking.

- A cauldron's fill level is about to change: `native_cauldron_level_changed(dimension, player, pos, from_level, to_level, reason, cancel)` is asked for rain, snow, dripstone and extinguishing. Set `cancel` to leave it as it was. `from_level` and `to_level` use 0 for empty and 1 to 3 for filled levels. `player` is the acting server player when available. Bucket, bottle and dyeing interactions are not covered yet.

- A player adds a compostable item to a composter at `pos`; the layer has been applied.

- A crafter at `pos` completed `recipe`; item consumption and output handling have finished.

- A player used an item in `minecraft:creeper_igniters` on a creeper, after it entered its ignited state.

- Lightning struck a creeper and set its charged state; the hook reports the creeper id and strike position.

- A player applied an enchantment: `enchant_item(player, name, pos, item, levels)` is heard after the player at `pos` applied it, with the finished `item` and levels charged. The guest's state is not swapped in.

- Saved entity records for a chunk finished loading into the server world: `entities_load(dimension, chunk, count)` reports the dimension, chunk coordinates and number of records read.

- Entities in a chunk left the active world and were queued for saving: `entities_unload(dimension, chunk, count)` reports the dimension, chunk coordinates and number of records.

- A zombie or raider finished breaking the door at `pos`; the mob id and entity type are reported after the door is removed.

- Two mobs completed a breeding action: `entity_breed(parent, partner, child)` reports both parents and the baby id when one was created; egg laying has no child id.

- A mob changed the block at `pos`; the hook reports the mob id and kind, and the old and new block names after the new state was applied.

- A bee was stored in a beehive or bee nest at `pos`; the hook reports the bee id and kind and the block name, after the bee was removed from the level.

- A mob entered love mode after a player interaction; the hook reports its entity id, kind and feeder UUID when known.

- A server entity finished its portal wait and is beginning to leave through the portal block at `pos`: `entity_portal(dimension, entity, pos)` reports the entity's network id, source dimension and entry block. The transfer destination follows the existing portal rules.

- A guest player's synchronized pose changed: `entity_pose_change(player, name, from, to)` reports the old and new pose names.

- A mob became tamed after its owner and persistence were set: `entity_tame(entity, kind, owner)` reports the entity type and owner UUID when known.

- A server entity is about to be teleported by `/tp` or `/teleport`: `teleport_command(dimension, entity, from, to, cancel)` (NeoForge's `EntityTeleportEvent.TeleportCommand`) names the level it arrives in, its network id, its feet now and where the command sends it. A mod may move `to` (a value that is not a number is ignored); `cancel` leaves the entity where it is. It fires once per teleported entity, before `entity_teleport` and `player_teleported`, which hear the move after.
- A server entity completed a same-level teleport command: `entity_teleport(dimension, entity, from, to)` reports its network id and the coordinates before and after the move.

- A guest player's fall flying state changed after the server accepted or ended a glide: `entity_toggle_glide(player, name, gliding)`.

- A mob conversion completed through Sunrise’s shared `Mob.convertTo` path; the hook reports the removed entity and its replacement ids and kinds.

- An experience bottle broke, after its water splash and experience orbs were created; the hook reports the impact position and awarded experience.

- A firework rocket completed its damage, particles and explosion effects; the hook reports its entity id, position and number of burst shapes.

- `fluid_level_change(dimension, pos, fluid, from, to, cancel)` is called when flowing water or lava at `pos` is about to change level; levels run from 0 through 8. Set `cancel` to keep the old level.

- A furnace consumes `fuel` and begins burning at `pos` (`furnace_burn`).

- A player takes smelted output from a furnace, blast furnace or smoker with a nonzero `removeCount` (`item_smelted`), on both logical sides (on the server after the furnace awards its used recipes and pops experience, and on the client during click prediction without awarding experience).

- A furnace finishes smelting `item` into `result` at `pos` (`furnace_smelt`).

- A furnace begins cooking `item` at `pos` when its cooking timer first advances (`furnace_start_smelt`).

- A hopper is about to pick up a dropped stack: `hopper_pickup_item(dimension, pos, item, count, cancel)` names its position and stack. Set `cancel` to leave the item in the world.

- `inventory_block_start(player, name, pos, cancel)` runs before a guest opens an inventory supplied by a block at `pos`. Set `cancel` to leave it closed; the guest's state is not swapped in.

- A dropped item entity was successfully added to the server level: `item_spawn(entity, item, count, pos)` reports its entity id, item type, stack count and spawn position.

- `leaves_decay(dimension, pos, cancel)` is called when a non-persistent leaf block at distance 7 is about to decay. Set `cancel` to keep it.

- A weather, Channeling-trident or `/summon` lightning bolt is about to be created: `lightning_strike(player, pos, cause, cancel)`. Set `cancel` to prevent the bolt and its effects.

- `moisture_change(dimension, pos, from, to, cancel)` runs when farmland at `pos` is about to change moisture from `from` to `to`. Set `cancel` to keep the old moisture.

- A server player put a plant into a flower pot: `player_flower_pot_manipulate(player, name, pos, plant)` is heard after the potted state is stored and the item consumed.

- A server player inserted a book into a lectern: `player_insert_lectern_book(player, name, pos, book)` is heard after the block entity and inventory changed.

- A server player changed an item frame: `player_item_frame_change(player, name, entity_id, action, item, rotation)` is heard after its contents and comparator output changed.

- An orb repaired an item through Mending: `player_item_mend(player, name, item, repair, experience)` is heard after its damage changed.

- A server player changed a lectern page: `player_lectern_page_change(player, name, pos, page)` is heard after the page and redstone pulse changed.

- A server player took a book from a lectern: `player_take_lectern_book(player, name, pos, book)` is heard after its block entity changed.

- `spawn_change(dimension, pos, angle)` is told after `/setworldspawn` moves the world's spawn point, with its level, block position and rotation.

- `world_border_bounds_changed` hears successful command and direct size writes. Read the
  affected border through `event.level.world_border(host)`; `event.lerp_ticks` distinguishes
  the fixed and timed setters, including a timed write to the current size.

- `world_border_center_changed` hears successful command and direct center writes, even an
  equal-value direct write, and gives the affected `event.level`. Neither notification needs
  the level to be in place or fires for loads or client packets. A nested border setter applies
  normally; it skips same-hook callbacks already running on this thread. Panics are logged and
  shown in an error toast and disable only the panicking callback for the session. Other
  refusals are logged when they happen and do not disable the listener.

- A server player completed an advancement after progress was recorded; `advancement` is its identifier, such as `minecraft:story/mine_stone`.

- A server player's main-hand punch swing and attack-strength reset have happened: `player_attack_cooldown_reset(player, name)`.

- A server player about to enter a bed after its sleep checks pass: `player_bed_enter(player, name, pos, cancel)` hears the bed head position. Set `cancel` to keep the player out. The player's state is swapped in.

- A server player is waking or standing up from the bed at `pos`; `player_bed_leave` is heard before their sleeping state is cleared, so a listener still reads them as sleeping.

- A server player caught a bucketable mob: `player_bucket_entity(player, name, entity_id, entity)` runs after the mob was removed and the filled bucket was put in the player's hand or inventory. `entity` is the mob species id.

- A server player's selected beacon effects were stored: `player_change_beacon_effect(player, name, pos, primary, secondary)`. The primary and secondary values are `Option<EffectKey>`; `None` means no effect is selected.


- A server player counts as deeply asleep after 100 ticks in a bed: `player_deep_sleep(player, name)` fires once at that point.

- A server player book-and-quill packet is about to be applied. `pages` is its page count and `signing` says it has a title; set `cancel` to drop it.

- A server player threw an egg after its projectile was launched and its use applied.

- A server player used a firework rocket to boost while gliding, after the rocket was launched and consumed.


- A server player's XP orb pickup cooldown changed, after its new value is stored (`player_exp_cooldown_change(player, name, ticks)`).

- An item cooldown about to start for a server player: `player_item_cooldown(player, name, item, ticks, cancel)` is called for use cooldowns and for a shield disabled by a hit. `item` is the item id and `ticks` its duration. Set `cancel` to skip the cooldown; shield disable still stops the use and plays its configured sound. The player's state is not swapped in.

- A server player switches hotbar slot (`PlayerSwitchHotbarSlotEvent`), only when the slot differs: `player_switch_hotbar_slot_pre(player, player_entity_id, old_slot, new_slot, cancel)` is called after the slot range check and before the selected slot changes. Set `cancel` to keep the old slot; the client is sent that slot again. Otherwise `player_switch_hotbar_slot_post(player, player_entity_id, old_slot, new_slot)` follows once the slot changed and a main-hand item use stopped. The player's state is not swapped in.


- A server player selected loom pattern button `pattern`, after the menu accepted it: `player_loom_pattern_select(player, name, pattern)`.

- A server player filled an empty map: `player_map_filled(player, name, map_id)` fires after the filled map and its saved map data were created.

- A server player named a mob with a name tag, after its name and persistence changed: `player_name_entity(player, name, entity_id, entity_name)`.

- A server player opened a sign editor at `pos`, for the front or back side, after the editor state was made: `player_open_sign(player, name, pos, front)`.

- A server player's middle-click pick resolved a block or entity's pick stack and is about to put it in the hotbar: `player_pick_item(player, name, from_entity, pos, entity_id, cancel)` receives block coordinates and entity_id zero for a block, or zero coordinates and the entity network id for an entity. A pick with no result is not heard. Set `cancel` to leave the inventory and client unchanged. The player's state is swapped in.

- A server player picked up an arrow after it was removed and its pickup was applied: `player_pickup_arrow(player, name, entity_id, item)` gives the pickup item id.

- A server entity about to travel to another level, a player or not: `entity_travel_to_dimension(entity, dimension, cause, cancel)` (NeoForge's `EntityTravelToDimensionEvent`) names the target level; `cause` is `nether_portal`, `end_portal`, `end_gateway` or `teleport`. Cancel to keep the entity where it is. The player's state is not swapped in. Ordinary End exits are `end_portal`; respawn after the End credits is not travel.

- A server player requests placement of a known recipe through a vanilla or mod recipe book (`player_recipe_book_click(player, name, recipe, all)`).

- A recipe about to become known to a server player: `player_recipe_discovered(player, name, recipe, cancel)` is heard for advancement rewards, knowledge books and `/recipe give`. Set `cancel` to keep it locked; the client is sent the matching removal. The player's state is not swapped in.

- A server player launches with a riptide trident: `player_riptide(player, name)` fires after the velocity and spin are applied. The player's state is swapped in.

- A server player about to shear a sheep, mooshroom, snow golem or bogged. `off_hand` says which hand holds the shears; set `cancel` to leave the mob and tool unchanged.

- A server player's shield was disabled by a blocked melee hit: `player_shield_disable(player, name, ticks)` fires immediately before the shield cooldown is set.

- A server player selected stonecutter recipe button `recipe`, after the menu accepted it: `player_stonecutter_recipe_select(player, name, recipe)`.

- A spectator's camera returned to their own player after they stopped watching an entity: `player_stop_spectating_entity(player, name, entity_id)` gives the former target.

- A server player swapped their main hand with an equipment slot: `player_swap_with_equipment_slot(player, name, slot)` fires after both hands changed; `slot` is currently `offhand`.

- A player is about to travel through an End gateway (`player_teleport_end_gateway(player, name, gateway, cancel)`). Set `cancel` to leave them at the entrance without starting the gateway cooldown.

- A server player used a golden dandelion on an ageable mob: `player_toggle_entity_age_lock(player, name, entity_id, locked)` fires after the lock and item use changed.

- A server player's action removed an entity's leash: `player_unleash_entity(player, name, entity_id)` fires after the leash state is cleared.

- The server synced changed motion to a player: `player_velocity(player, name, velocity)` fires with velocity in blocks per tick.

- `portal_create(dimension, pos)` is told after a Nether portal for dimension travel or the End arrival platform has been built, with its dimension and lower corner or platform origin.

- A thrown splash or lingering potion finished applying its hit effects; the hook reports the potion item and impact position. Experience bottles are excluded.

- A crafting-grid result was prepared: `prepare_item_craft(player, name, pos, result)` is called for the inventory grid (None) or a table at `pos`; change the result before it is sent to the menu. Guest state is not swapped in.

- Enchanting offers were calculated: `prepare_item_enchant(player, name, pos, costs)` is called for a server player at `pos`; change the three costs before they are sent to the menu. Guest state is not swapped in.

- A smithing result was prepared: `prepare_smithing(player, name, pos, result)` is called for a server player at `pos`; change the result before it is sent to the menu. Guest state is not swapped in.

- A raid wave has spawned at its position, with its raid id, wave number and raider count (riders' mounts not counted).

- A raid is triggered or reaches `victory`, `loss` or `stopped`: `raid_state_change(raid, pos, state)` reports its owning `RaidRef`, center and new state. The handle retains its level even when that level is parked; an already removed raid cannot be read.

- `sculk_bloom(dimension, pos)` is called after a sculk catalyst at `pos` takes experience and enters the blooming state.

- `sponge_absorb(dimension, pos)` is called after a sponge at `pos` has absorbed water and become wet.

- A guest's server command completion request was evaluated: `tab_complete(player, name, command, range, suggestions)` lets a mod add or filter the suggestions; `range` is the replaced byte range. The guest's state is not swapped in.

- `target_hit(dimension, pos, strength)` is called after a projectile hits the target block at `pos` and it takes the reported redstone output strength.

- A `/time` command or mod clock setter is about to change the clock: `time_skip(dimension, from, to, cause, cancel)`. Set `cancel` to keep the old time.

- A player selected a trade: `trade_select(player, name, villager, index)` is heard after the trading menu updates; `villager` is the entity id when known. The player's state is not swapped in.

- `vault_change_state(dimension, pos, state)` is called after a vault at `pos` changes to `state` (`inactive`, `active`, `unlocking`, or `ejecting`).

- `weather_change` hears explicit server-wide weather parameter writes, not natural weather ticks
  or strength changes. Each listener sees the proposed signed timers and flags. Return `Flow::Cancel`
  or set `event.cancelled` to veto all the parameters; a veto is logged with the mod's ID and does
  not undo any Host writes. On panic, the game ignores that listener's edits to cancellation,
  logs the panic, shows an error toast and disables only that mod's callback for the session.
  Phase and reentry refusals are logged each time without disabling the listener. A nested write
  skips callbacks for this hook already running on the thread; other eligible listeners still run.

- The dedicated server's whitelist changed state: `whitelist_state_update(enabled)` reports its final on/off state, after `/whitelist on` or `/whitelist off` from a player or console.

- The server completed a world save cycle: `world_saved(wait)` reports whether chunk writes were flushed before returning.

- The time players wake at after sleeping, as NeoForge's `SleepFinishedTimeEvent`: `sleep_finished_time(dimension, current, new_time, adjustment)` runs on the server once enough players have slept and the `advance_time` rule is on, before the clock moves. `adjustment` (a `ClockAdjustment`: `Absolute`, `Relative` or `Marker`, starting as vanilla's `wake_up_from_sleep` marker) says where the clock goes and a mod may change it; `new_time` is the vanilla morning, read only. A time before `current`, or an undefined marker, is refused. Set `canceled` to keep the old time without stopping waking or the weather reset.

- A shift-click in a mod container's menu routed by the mod, `AbstractContainerMenu.quickMoveStack`: `menu_quick_move(kind, subject, container_slots, slot, stack, handled)` (version 6) runs on the server before the game's own routing, for a mod container on a vanilla menu type or its own menu type, lending the live resident block entity and server `Host` through `subject: BlockEntityScope<'_>`. The mod takes what it moves out of `stack` and sets `handled`; the slot keeps the rest and the client gets the menu again.


- A server player's or fox's `teleport_randomly` consume effect (chorus fruit) is about to try a spot: version 2 of `item_consumption_teleport(consumer, consumed, from, target, cancel)`, once per attempt (up to 16). `consumer` is the living actor's handle and `consumed` is the public API stack captured before the use, with its count, damage and component patch, rather than the current contents of the hand. The living handle replaces version 1's player-only UUID/name fields; players retain their acting-player context. A mod may move `target`; the game still looks down from it for ground to land on. `cancel` ends the effect with no teleport and no more attempts, not consumption. A fox's callback runs with the full server host before its living AI/movement and before the next mob ticks; listener changes to its mouth item, position or eligibility invalidate the captured decision rather than being overwritten.

- A server player's or mob's ender pearl landed: version 3 of `native_ender_pearl_teleport(owner, pearl, from, target, damage, cancel, hit_entity, hit_block, hit_face, hit_at)` runs before teleporting the live `EntityRef` owner. A mod may move `target`; cancellation spends the pearl without a teleport, sound, player damage or endermite roll. A player retains acting-player context and mutable fall damage (5). A mob preserves rotation, resets fall distance, takes no player fall damage and makes no endermite draw. A listener's move or an owner that is no longer eligible invalidates the pending move; it is refused rather than overwriting that write. `owner` replaces version 1's player-only UUID/name fields; use its handle and the host services. Version 3 adds what the pearl hit (`hit_entity`, `hit_block`, `hit_face`, `hit_at`). No compatibility fields or older alias are retained.

- Drawing an item stack on a screen: `native_item_stack(batch, stack, at, size)` draws its model, glint, damage bar, cooldown and mod decorations, but not its count. It draws nothing and returns false for an unknown item or an empty stack (air, or a count of 0).

- Hiding a player (a vanish): `player.hide(host, level)` hides that player, from the next tick, from the players below permission `level` (None shows them again): they leave those players' Tab list and world (their clients stop tracking them, as they do a spectator) and those players don't read them joining or leaving. Players of `level` or more still see them; a player made an operator sees them again. Retain a `PlayerRef` obtained while the player is available: unlike live-player writes, hide can still change that UUID's visibility after disconnect, within the handle's world epoch. It cannot target an arbitrary offline UUID, and a handle from a closed or reopened world refuses the write. The visibility setting is kept by UUID across reconnects until changed or the server stops. It requires `&mut dyn ServerHost` and an available server player list.

- A player's item used up or broken: `player_destroy_item(player, player_entity_id, original, hand)` reports the complete stack before use or durability loss, including its count, damage and components. It covers hand use, entity consumption, completed eating/drinking and durability breaks on each logical side, including the local host. `hand` is `Some(Main)` or `Some(Off)` for hands, `None` for armour or inventory ammunition. A partial consumption and creative use preserving the stack are silent. Listen only, after the consumption or break has applied and before a use remainder is installed. The callback receives the typed `HostSide`. `ItemPart::damage_item` decides wear; this notice cannot veto it (NeoForge's `PlayerDestroyItemEvent`).

- A player's damaging melee hit on a living target: `critical_hit(target_kind, vanilla_critical, critical, damage_multiplier)` may force or deny the critical hit and change its multiplier (1.5 for a vanilla crit, else 1.0); the sweep, sound and particles follow the decided result. Not fired for a dragon part or a hit that does no damage.

- Attachment reads that never insert the default: `AttachmentKey::has` says whether a holder carries a value, saved or set,
  even before it decodes (NeoForge's `hasData`), and `AttachmentKey::get_existing` returns `None` instead of the default
  (NeoForge's `getExistingData`). Neither adds anything to the holder's saved data.

- A player releases a bow or fires a charged crossbow: `arrow_loose(stack, has_ammo, charge, canceled)` may change the held-tick `charge` for a bow or prevent either shot. The crossbow asks with charge 1 before spending its loaded projectiles. A negative charge refuses the shot. Projectile admission is a separate `entity_join_level` decision.

- An item frame or painting about to break: `hanging_break(player, kind, pos, cancel)` is heard before a server player's hit, a blast or the loss of its wall breaks an item frame, a glow item frame or a painting, with the hitting player (None for a blast or a lost wall). A mod sets `cancel` and it stays. The hitting player's state is swapped in. A hit that only takes a frame's item out, the game's own player and `/kill` are not asked.

- A criterion of a server player's advancement was granted or revoked after its progress changed and before any completion rewards, except the `minecraft:tick` criteria granted on a network player's first tick or a reload, which are heard after that tick's rewards and packets; `advancement` is its identifier, such as `minecraft:story/mine_stone`, `criterion` the criterion name and `progress_type` is `ProgressType::Grant` or `ProgressType::Revoke`. A grant or revoke that changes nothing is not heard.

- A statistic about to be awarded to a server player: `player_stat_award(player, name, stat_type, stat, value, cancel)`. The canonical namespaced `stat_type` and `stat` strings and the `i32` amount `value` are writable. Unknown types or entries, noncanonical ids and negative amounts keep the entire original award, with a warning once. Registered custom stats are valid redirect targets. Set `cancel` to award nothing: the count and its scoreboard criteria stay as they were. Transaction-deferred awards fire only after the outermost successful commit.

- A player's data copied to their replacement: `player_clone(player, name, was_death, original)` (version 2) fires after death (`was_death` true) and the credits-showing End return (`was_death` false), after old-player cleanup (carried drop and menu close), destination level open, and copying vanilla state and serializable attachments (`copy_on_death` on death, all serializable entries on End return), and before placement (`ServerPlayer.restoreFrom`, NeoForge's `PlayerEvent.Clone`). `player` is the replacement's UUID, already carrying its destination level/dimension; `original: OriginalPlayer<'_>` directly lends the retained pre-copy player (preserving its source level/dimension) for the callback's lifetime (compile-time scoped, with no token handle or separate original-player holder variant). Its attachment methods follow normal `AttachmentHolder` semantics on the old player without touching the replacement and participate in `host.transaction`: `attachment` returns defaults without inserting them, `existing_attachment` decodes and caches saved entries once, and `has_attachment`, `with_decoded_attachment`, `set_attachment`, and `remove_attachment` act on the old holder only, rolling back with replacement writes on abort. It also exposes `with_inventory` (vanilla 0–42 slots, after death drops and menu close or intact with keep-inventory and on End return), `persistent_data`, `uuid`, `position`, `dimension`, `level`, `health`, `experience`, and `effects`. No subscriber means no retained copy (attachment copy still runs). Nothing swaps a player's state in.

- A player loaded from the world's save: `player_load_from_file(player, name, directory)` is heard once for each join (the owner of an integrated server too), after the player's `.dat` is read and applied and before they are placed in a level (NeoForge's `PlayerEvent.LoadFromFile`, `PlayerList.loadPlayerData`). `directory` is the world's player directory (`players/data`); a mod reads its own file there, named `<uuid>.<suffix>`. It cannot change or stop anything; no player's state is swapped in.

- A player saved to the world's store: `player_save_to_file(player, name, directory)` is heard once for each save of a placed player (when they leave or the world closes, when the dedicated server saves everyone, and for an integrated server's owner when the world saves them; LAN guests are not heard on integrated autosaves) after their data is written (NeoForge's `PlayerEvent.SaveToFile`, `PlayerList.save`). `directory` is the world's player directory (`players/data`); write your own file there, named `<uuid>.<suffix>`, never the player's `.dat`. The player may be leaving, so no player's state is swapped in. It cannot change or stop anything.

- `item_expire(item, extra_life)` (version 2), a dropped item reaching its lifespan on the server (NeoForge's `ItemExpireEvent` / `EventHooks.onItemExpire`, `ItemEntity.tick`): every mod may set or add to `extra_life`, in ticks from zero, and query the live item entity through `event.item` (including lending its stack with `event.item.with_stack(host, ...)`). It runs even if a listener extends the item's life. Afterward `lifespan` (default 6000) gets `extra_life` by signed 32-bit wrapping addition, then is clamped to `0..=32766` (`Short.MAX_VALUE - 1`) without rewinding `age`. If `age >= lifespan`, the item is discarded and `entity_leave_level` fires before the next drop ticks.

- The title's own screens to the screen hooks. `ScreenView.id` has one id per title state (`title`, `world_select`, `world_create`, `world_create_flat`, `world_create_buffet`, `world_preset_editor`, `world_edit`, `world_backup`, `world_backup_confirm`, `world_optimize_confirm`, `world_optimize`, `low_disk_space`, `world_delete`, `world_recover`, `world_recovered`, `error`, `multiplayer`, `add_server`, `edit_server`, `direct_connect`, `server_delete`, `friends_join`, `disconnected`, `multiplayer_warning`, `onboarding`, `ban_notice`), with no `menu`, no `title` and no `slots`, and the whole window as `rect`. `screen_render_post` fires after the title draws its screen (before its toasts), and the `_pre` input hooks — mouse buttons, drag, wheel, key press and release, character typed — fire on it and can cancel it as on any screen, so a mod draws its own button or browser over Create New World and takes the clicks. A dialog or prompt over the title is itself the receiver. Characters go to the focused child after the key acts; with no child handling them, `screen_char_typed_post` still reports them. `native_instance_dir()` is the instance folder, where `options.txt`, `saves/`, `resourcepacks/`, `shaderpacks/` and `sunrise-mods/` are; a world's `datapacks/` is in that world's folder.
- On the server, `anvil_craft_post(host, event)` reports a completed take after the level charge, input consumption and anvil wear. `event.inputs` lends the same pre-take stacks as Pre, even if the anvil broke. Vetoed takes do not fire Post.

- On the server, `anvil_craft_pre(host, event)` lends the left and right inputs and the result through `event.inputs` before those effects. Set `event.cancelled` or return `Flow::Cancel` to return early before charging levels, consuming inputs, damaging the anvil or firing `anvil_craft_post`, without undoing the click's item move (matching NeoForge `AnvilMenu.onTake`).

- A server player stopped digging `pos` before the block broke: `block_damage_abort(player, name, pos)` is heard after the server clears the dig and sends the block state back to the client. The player's state is not swapped in.

- A mob is about to drop `count` of `item` at `pos`: `entity_drop_item(entity, item, count, pos)`.

- A mob's leash was removed, before any lead drop is handled: `entity_unleash(entity, kind)`.

- A server explosion is about to begin at `center` with `radius` (`explosion_prime`); the hook is listen-only and the explosion proceeds as usual.

- A hopper selects a container while searching for items: `hopper_inventory_search(hopper, container, entity)` identifies the hopper and the selected block container or container entity.

- A hopper is about to move one item between containers: `inventory_move_item(source, destination, item, cancel)` runs before the source slot is changed, a composter's bone meal included; a chest boat or minecart is reported at the block it is in. Set `cancel` to keep the item in its source slot.

- A boat or minecart joined the level at `pos`, after a player or dispenser placed it: `vehicle_create(kind, pos)` is listen-only.

- A player is about to damage a boat or minecart: `vehicle_damage(kind, damage, cancel)` may change the damage or cancel it before the vehicle's damage and wobble change; the attack still completes.

- A boat or minecart is about to be destroyed at `pos`: `vehicle_destroy(kind, pos)` is heard just before it is removed; TNT minecarts that prime instead are not heard.

- A guest's arm swing: `player_arm_swing(player, name, hand)` is called after the server handles the guest's `Punch` packet and resets attack strength. The current packet swings `mainhand`; the guest's state is swapped in.

- A server player selected the item at `index` in a bundle in menu `slot`: `player_bundle_item_select(player, name, slot, index)`.

- A server player's client finished loading the level around them after joining, respawning or changing dimension: `player_client_loaded_world(player, name)`.

- A guest's player input flags: `player_input(player, name, flags)` is called after a loaded guest's `PlayerInput` packet is applied to the hub. `flags` is the packet's bit field; the guest's state is swapped in.

- A network or fake guest was kicked: `player_kicked(player, name, reason)` (version 2) is called after the hub records the rendered reason; a network guest also receives the disconnect. The guest's state is swapped in unless already removed in the same poll.

- A server player's experience level changed: `player_level_change(player, name, from, to)` is called after experience points or levels change their level, including orb points. The player's state is swapped in when they are a guest.

- A server player's recipe book open or filter setting changed: `player_recipe_book_settings_change(player, name, book, open, filtering)`, where `book` is its typed `RecipeBookIdentity`.

- A guest's player input changed sneaking: `player_toggle_sneak(player, name, sneaking)` is called after the hub applies the new state, only when it changes. The guest's state is swapped in.

- A hopper begins a transfer attempt: `prepare_hopper_transfer(pos)` runs after its cooldown and enabled checks, before it pushes or pulls items.

- A vault is about to eject a loot item: `vault_loot(pos, item)` supplies the vault position and item identifier before it spawns.

- `world_border_bounds_change_finish` gives `event.level` when its moving border reaches the target
  at the start of its normal tick. Starting or replacing an extent is not a finish. Panics isolate just
  that callback for the session, with a log and error toast, as for the center and size notices.

- `ClientHost::create_dynamic_texture(id, width, height, rgba)`, `update_dynamic_texture`, `update_dynamic_texture_region(id, origin, size, rgba)` and `release_dynamic_texture(id)`: vanilla's `DynamicTexture` registered in the `TextureManager` (`register`, `upload`, `release`), the region write after the region overload of `CommandEncoder.writeToTexture`. Each texture is a GPU texture of its own, `RGBA8_UNORM`, one mip level, sampled nearest with repeat, under an id in a namespace the calling mod owns; `GuiCanvas::texture` draws a live id of any mod, else the resource stack. Calls are checked on the client thread and applied in order on the render thread before the frame, with no merging and no budget; the game keeps no CPU copy. The only size limit is the device's `max_texture_dimension_2d` (`TooLarge`). Textures survive resource reloads and world close and are freed by `release_dynamic_texture` or the mod's unload; a lost graphics device loses them (the next update reads `Unknown`). An id released after its draw was submitted draws the missing texture. Refusals are named (`DynamicTextureError`) and change nothing. Replaces `native_mod_image`. Diverges from upstream: horizon-rust/horizon keeps `native_mod_image`.
- A living entity drowning (`living_drown`), with changeable drowning, damage and bubble count, and cancellation.
- `Game::native_translate(key)`: the selected language's text for a translation key, or `None` when no language layer defines it (`I18n.exists`), so a mod can fall back its own way (a recipe viewer names an item tag by `tag.item.<namespace>.<path>` and falls back to the id). NeoForge: `I18n.exists` and `I18n.get`. Client-side.
- Every `Host`, on either side, and `ClientSetupContext` expose `enchantments()`, `enchantment_supports(id, item)` and `enchantment_min_cost(id, level)`. A server reads its own enchantment registry, data packs included; a client reads the table built from the server's synced enchantment entries when available, and the local table otherwise. `enchantments()` lists `EnchantmentInfo { id, max_level, weight, anvil_cost, curse, description }` in registry order: `weight` is the selection weight (`Enchantment.getWeight`), `anvil_cost` is the base anvil fee per enchantment level (`Enchantment.getAnvilCost`) and `description` is the definition's name component (`Enchantment.description`). `enchantment_supports` checks the entry's `supported_items` (`Enchantment.isSupportedItem`), not Item Part overrides. `enchantment_min_cost` reads the power a level needs (`Enchantment.getMinCost`), or `None` for an unknown enchantment.
- `Game::native_fluid(batch, fluid, [x, y], size)` and `Game::native_fluid_name(fluid)`: a fluid (`minecraft:water`, `mymod:oil`) as a square of `size` GUI pixels, the GUI sprite its look names (`data/<mod>/sunrise/fluid/<name>.json`) tinted with its colour, else a flat square of the colour (vanilla water and lava have theirs), and its name as the lang gives it. What a recipe viewer draws for a fluid stack; the slot, the amount and the tooltip are the mod's. NeoForge: `IClientFluidTypeExtensions.getStillTexture` and `getTintColor`. Client-side.

- A living entity's fall (`living_fall`), with changeable distance and multiplier and cancellation.

- A living entity healing (`living_heal`), with a changeable amount and cancellation.

- A living entity jumping (`living_jump`).

## Hook and helper capabilities

- `ClientHost::panel_items` shows an item panel beside container screens; `panel_event` supplies the player's
  clicks, searches and keys over its items. `Host::recipes_of_type` returns `Recipe` values with parsed ingredients,
  results and rules for supported vanilla types, and the original JSON for mod-defined types. `Recipe` does not
  define display pages, per-result chances or fluid input/output slots: a viewer interprets its own machine-recipe
  JSON and draws those details itself. `known_recipes`, `place_recipe` and `transfer_recipe` provide recipe-book
  integration, `transfer_recipe_check` its dry run on the server; they do not create a machine-recipe display record.
  `native_sprite` takes an optional `tint` (`0xAARRGGBB`) that multiplies every texel, as a fluid's colour does.
  `DrawContext::text(text, at, argb, shadow)` takes its colour as vanilla's `GuiGraphics` does, `0xAARRGGBB`
  (`0xFFFFFFFF` is opaque white), and draws nothing for a fully transparent one, as vanilla's does.
  `DrawContext::text_width(text)` measures what `native_text` draws, in GUI pixels, to lay text out before drawing
  it (`Font.width`). A sprite's own size is `gui_sprite(&id)`'s `size`, read once in `texture_atlas_stitched`.
  Diverges from upstream: there `native_text` takes an opaque `0xRRGGBB`.
  Slot clicks keep their `SlotClicked` numbering: inputs, then results.

- `GameLevel::native_send_particles` on authoritative servers. It sends supported
  vanilla particles to nearby players. Block particles require a valid block state; item particles
  require a validated stack. Trail and vibration remain unsupported. Count zero sends one directional
  particle. The return value says whether the call was accepted, not whether any player received it.

- Authoritative server tick events through `server_tick`: start (before any level ticks, like NeoForge's `ServerTickEvent.Pre`) and end of each non-paused
  server tick, with `world_running`. A frozen world reports false except during a requested step. Like `world_lifecycle`,
  it only fires for `Authority::IntegratedServer` (single player or LAN) and `Authority::DedicatedServer`.

- Server commands and players: `register_commands` adds usages to the server tree with
  `Command::executes` to bind their executors, with helpers to answer
  ([Server commands and players](#server-commands-and-players)).

- A player's items for server mods: counting, taking and giving go through the player's
  [`ENTITY_ITEMS`](../concepts/capabilities/entity-items.md) handler.

- Server menus: a vanilla chest menu a server mod opens and fills, whose clicks and teardown reach the mod
  (`menu_clicked`, `menu_removed`, `player_container_close`), and trades on them between the players'
  [`ENTITY_ITEMS`](../concepts/capabilities/entity-items.md) handlers in one transaction ([Server menus](#server-menus)).

- Item tooltips: unless the stack hides them, `ItemPart::tooltip` adds lines where `Item.appendHoverText` does,
  after the name and before the components, with the same `TooltipAppendContext` as appenders in `event.context`.
  `Hooks::item_tooltip` (NeoForge's `ItemTooltipEvent`, version 3) then gets every item's whole tooltip once it is
  complete, lending `stack: ItemStackView<'_>` (call `project()` to copy components on demand),
  `display: &TooltipDisplay<'_>`, live `lines: TooltipLines<'_>` validated per edit with `TextFormError`
  (one invalid edit leaves all other lines intact), the local player, and its `TooltipFlag` (`advanced`,
  `creative`), and may add, change or remove any line. It also fires for `ItemResource::tooltip`
  queries; reentrant queries return base lines. Creative search uses exported lines and does not fire it.

- Data component tooltip lines: `Registrar::register_tooltip_appender(component, placement, appender)` (NeoForge's
  `RegisterTooltipAppendersEvent`) gives one component's `TooltipAppender` a `TooltipPlacement`: `BeforeAll`, `AfterAll`,
  or `Before(id)` and `After(id)` for a vanilla tooltip component or one another appender is registered for, sorted as
  `ItemTooltipHandler` sorts them. The game asks only that appender, only for a stack that carries the
  component (live, transient ones included) and whose `tooltip_display` shows it, and only while its mod is enabled. It gets
  the stack and a `TooltipAppendContext` (local player, `advanced`/`creative`/`nested` flags, and the client level's
  `tick_rate`, or 20 without a level). Its lines go after the item's own and before the advanced lines.
  Creative search doesn't read these lines or `item_tooltip`; mods reacting to every item use `item_tooltip`.
  `Before`/`After` accept only the 23 vanilla anchors listed in `tooltip_appender.rs`, not other registered
  components. The `registerAppender(TooltipLocation, ...)` overload is not represented. Several appenders may register
  for one component; all run in registration order at an equal placement. The player is always the local player,
  including for nested crossbow projectiles where vanilla passes no player; use `nested` to distinguish those calls.

- `ItemPart::use_on` and `use_item` run on both sides (`HostSide`), version 2, with
  `Option<InteractionResult>` answers. `None` defers; `Some` replaces the template.
  Consuming answers end the client's hand loop: `Success` swings client-side, `SuccessServer`
  server-side, `Consume` never. Client hosts permit reads/effects, not authoritative writes.

- A block's own item-use behaviour on both sides: attach a `BlockPart` and implement
  `use_item_on(host: HostSide, event) -> Option<InteractionResult>`. `event.state` keys the per-state
  table; `event.click` carries the level, position, hit, hand, stack and player. `Some` replaces
  the template's `Block.useItemOn`: `Success` predicts a client swing, `SuccessServer` and `Consume`
  consume without a client swing, `Pass`/`Fail` let the item try its `useOn`, and `TryWithEmptyHand`
  tries the block's `useWithoutItem` on the main hand only. `None` leaves the template in charge.
  This is the block's behaviour, not the `UseItemOnBlockEvent` interceptor. `right_click_block`
  still decides whether to use the block. Only the clicked state's parts run.

- Global interaction interception on both sides: `Hooks::use_item_on_block(host, event)` is NeoForge's
  `UseItemOnBlockEvent`. `event.phase` is `UsePhase::Block` just before the block's item method/Part,
  or `ItemAfterBlock` just before the item's `use_on` method/Part. It fires only after that phase's
  sneak, `use_block`/`use_item`, cooldown and other eligibility gates select it. `event.click`
  (`BlockClick`) supplies the dimension, hit, hand, stack and player. Return `Flow::Cancel` to
  skip that phase's Part and template, using `*event.cancellation_result` as the phase's result
  (default `TryWithEmptyHand`); a main-hand block-phase TRY can still run the empty method.
  `UsePhase::ItemBeforeBlock` fires first, before the item's `on_item_use_first` Part and before
  the block is asked anything (ahead of the sneak gate, unless `right_click_block` denied the item);
  canceling skips that item's Part, but `Pass` continues to the block. Any non-`Pass` answer ends the block interaction.
  Client Parts predict hands only; server-only answers cannot retroactively suppress client offhand use.
  `Fail` ends only block interaction; `Pass`/`TryWithEmptyHand` fall through.
- `ItemPart::on_item_use_first(host, event)` is `IItemExtension.onItemUseFirst` on both sides, for
  the held item (`event.click` is the same `BlockClick`). Any non-`Pass` answer skips the block
  and the item's `use_on` for this hand; only `Success` predicts a swing. `None` or `Pass` goes on
  to the block. NeoForge's bus event for this moment is `use_item_on_block`'s `ItemBeforeBlock` phase.

- `Hooks::right_click_item(host, event)` is NeoForge's `PlayerInteractEvent.RightClickItem`, before
  `Item.use` and the item's `use_item` Part on both sides, after the same hand's target interaction falls through.
  `event.click` (`ItemClick`) contains the dimension, player's feet block position, player,
  hand and stack. `Flow::Cancel` skips the Part/template and returns `*event.cancellation_result`
  (default `Pass`). `Success`, `SuccessServer` and `Consume` end the client's hand loop. `Success`
  predicts a swing, `SuccessServer` swings on the server, and `Consume` has no swing. A block
  success, block FAIL, or entity success prevents item fallback; the server fires this on the separate UseItem packet
  rather than on UseItemOn.

- A block's empty-hand behaviour: `BlockPart::use_without_item(host, event)` replaces
  `Block.useWithoutItem` on either side, on the main hand only, when the block result is
  `TryWithEmptyHand`. This includes a canceled `Block` event with that result. There is no
  fourth event phase. The Part sees the captured state's key and the hand's optional stack.
  `Some` replaces the template; `None` lets it run. Consuming results trigger DEFAULT_BLOCK_USE
  on the server. `Pass`, `Fail` and `TryWithEmptyHand` continue to the item without retrying.
  A client Part has reads and local effects, and cannot suppress the ordinary server request.
  Prediction keeps the ordinary path for a refused payload and discards a stale result.

- Breaking and placing a block, on the server for every player: `break_block` hears a player break a
  block once the game allowed it (`ServerPlayerGameMode.destroyBlock`) and may cancel it, which keeps the block and its
  drops. Set `exp_to_drop` to `Some(amount)` to replace the vanilla XP sample later, after item drops are prepared;
  `None` retains vanilla sampling. The subsequent `block_drops` event can edit the resulting experience or cancel
  the drops, and disabled block drops suppress both items and XP. `entity_place` (`EntityPlaceEvent`, `EntityMultiPlaceEvent`) hears a block placed from an item
  (`BlockItem.place`), an enderman putting its block down or a snow golem's trail, before it is written, and may cancel it:
  nothing is placed and the stack is kept. For a player, `placed_against` is the state next to the primary placed
  block opposite the clicked face, even when the clicked block is replaceable; for mobs it is the state below.
  A bed, door or tall plant arrives as one `Placement::Multi`. Silverfish
  infesting, turtle eggs and falling blocks are not placements yet.

- Block entities for a mod's blocks (`block_entity`, `block_entity_tick`): made when the block is
  placed and saved with the chunk. Attaching a `BlockEntityPart` with a tick method binds the ticker
  for its registered type on that side; `BlockEntityTypeBuilder::ticker_when` can restrict its states.
  `block_entity_tick` receives `&mut BlockEntityTickEvent<'_>` without a separate Host.
  `event.subject.with_view` lends the resident type ID, saved data, item slots, and transient
  data slots for the closure; it returns `None` if the subject is no longer available. To edit
  Part state, return the handle from `event.subject.with_view(|view| view.handle())`. After the
  view borrow ends, borrow `event.subject.host()` separately for `edit_state`.
  Use `event.subject.transaction` to commit edits to the subject and other objects together.

- Item slots for those block entities (`r.block_container(&block, container)`): hoppers, comparators and breaking the
  block use them as they use vanilla's containers, and the tick's resident view lends the slots.

- Menus for mod containers: the container names one of vanilla's slot-grid or furnace menus, which opens when the block is used.
  The tick's resident view lends the data slots the menu shows.

- A machine reaches neighbouring storage through the block's Parts and capabilities, not a raw mutable
  block-entity callback. `block_entity.data_slots(host)` reads a mod entity's menu data on the server or
  the client's own copy; `set_data_slot(host, slot, value)` writes a declared slot on the server under
  the current transaction, rolls back on abort, and uses the normal menu broadcast to viewers.
  `level.set_block` uses the level's normal update path.

- Screens a mod draws for menu types: `r.register_menu_screen(menu, factory)` registers the constructor directly by menu type, matching NeoForge's `RegisterMenuScreensEvent` and vanilla's `MenuScreens.SCREENS`. It returns a fresh `MenuScreen::new(layout)` for each client opening. Decode `MenuScreenOpen::extra` during this call; the bytes are not available afterwards. Draw GUI textures (`assets/<namespace>/textures/gui`) with `MenuScreen::with_draw` and read live values from `MenuScreenFrame::data`.
  The factory goes with the menu type whatever opened the menu: a block's use or `PlayerRef::open_mod_menu` from an item, an entity or a command. Built-in vanilla menu types that already have a screen in `MenuScreens.SCREENS` and duplicate registrations are refused; any mod may register a factory for a custom menu type. `open.block` is `None` without a verified opener.
  `None` entries in `MenuScreenLayout::slots` (including omitted trailing entries) start inactive/hidden (`Slot.isActive()`) and can be activated or deactivated locally with `set_screen_slot` by any mod; local slot changes neither reach the server nor survive a replacement. A bound book needs a registered factory for its menu: set the book in the layout.

- Buttons and tooltips on mod screens: install client callbacks with `MenuScreen::with_click` and `MenuScreen::with_tooltip`. A button returned by the client is a request: the server still checks its window, menu validity and the owning part's `menu_button_valid` before calling `menu_button`, with the backing block entity if present. A mod inserts items through the block's item capability, `level.capability(host, &ITEMS, pos, side)`.
- `menu.click_button(host, button)`, on a client `MenuRef`, presses button `button` of the open menu as a click on one of its buttons does
  (`MultiPlayerGameMode.handleInventoryButtonClick`, `ServerboundContainerButtonClickPacket`); the server's menu decides what it does
  (`clickMenuButton`, and `menu_button` for a mod's menu). It is for a client mod's own input on a menu's screen other than a press, such as a key or the wheel (`screen_key_pressed_pre`, `screen_mouse_scrolled_pre`). It returns `false` for a stale
  handle and for the inventory menu, which no server opened.

- What a server mod needs to guard land and move players. `right_click_block` hears a player's right
  click on a block before the block's own use (a chest, a door, a lever) and the item's, and may cancel it
  with NeoForge's answer: `Fail` stops the click, and the player sees the block as it
  is. `host.players().acting_player()` returns the acting player handle;
  `host.commands().run_command` runs a command as the server's console (level 4, no output), so `execute in <dimension> run tp
  <player> <x> <y> <z>` moves a player across levels; `PlayerRef::action_bar` tells a player why, even inside their own
  hook; and the handles `host.players().server_players()` lists expose health through `player.entity().living(host)`.

- What a server mod needs to keep the world itself out of guarded land. `block_spread` hears a
  fluid flow or fire spread into a block, `native_piston_pre` a piston's push or pull, `explosion_detonate` the
  blocks an explosion is about to break (a mod takes out those it spares), `fill_bucket` a bucket filled,
  `bucket_empty` one emptied, and `attack_entity` a player's blow on an entity (an item frame, an armor stand, an animal, a
  player). Each may cancel, and nothing happens.

- `level_tick_pre` and `level_tick_post`, both-sided observations around each
  level tick pass. Each receives the dimension ID; neither can cancel the tick. Paused and held passes fire
  neither hook, and an error skips the post hook.

- `native_set_block`, a server-side `Game` helper that replaces a state in the current dimension through
  `Level.setBlock`. It accepts vanilla's neighbour and shape flags and returns whether a block changed; a block with a
  block entity gets one. `UPDATE_CLIENTS` sends the update; `UPDATE_INVISIBLE` suppresses notification only on the client, as in `Level.setBlock`. Hooks holding only `GameLevel` cannot call it yet.

- `LevelWriteHost::play_sound` queues a sound for nearby players in the level's dimension, including during a
  guest's turn. Pass a valid namespaced event key, a `SOUND_SOURCES` index, position, volume and pitch.
  Registered events are sent by ID and other valid keys by name, regardless of namespace. A malformed key
  is refused. A queued sound does not guarantee playback; the client must have the event's resources.

- `Registrar::register_config(ConfigSpec, Option<Arc<ConfigListener>>)` registers config files in startup
  declaration order (`ModContainer.registerConfig`) and delivers `ConfigEvent::{Loading, Reloading, Unloading}`
  (`ModConfigEvent`) with `&ConfigSnapshot` (`mod_id`, `kind`, registered `file`, actual on-disk `path`, `text`)
  only to the registered owner. `Host::config_dir()` lends `&std::path::Path` for the game's `config/` folder
  beside `options.txt` (beside `server.properties` on a dedicated server), matching NeoForge's
  `FMLPaths.CONFIGDIR`. Initial loading lends `ConfigContext` for owner-bound settings and `config_dir()`;
  reloading and unloading lend the polling game's `HostSide` and stopping game's `ServerHost`, respectively.
  Client setup also lends `config_dir()`. Registration can derive it from the captured `InstanceDirectory`.
  Native mods read and write their own config and bulk files directly with `std::fs` under `Host::config_dir()`,
  `Host::instance_dir()` (for instance-wide data such as `<instance>/sunrise-mod-data/<mod id>/`), or
  `ServerHost::world_dir()` (for world-scoped data backed up and deleted with the save).
  Settings (`sunrise-mods/.sunrise-mod-data/<mod id>/`, one file per key) and shipped verified package `[runtime]`
  assets (`RegistrationContext::runtime_files()`) remain separate
  ([config files and settings](../how-to/mod-settings.md)).

- To draw custom status rows, replace or wrap `minecraft:player_health` (hearts), `minecraft:armor_level`,
  `minecraft:food_level` or `minecraft:air_level` in `register_gui_layers`. Use the installed body's
  `DrawContext` (`draw_gui` lends a scoped canvas). `gui_layer_pre` can veto a slot without vetoing
  the other rows. Heart bookkeeping belongs to the vanilla heart body; bubble sounds belong to the
  vanilla air body.

- Which hearts the player's health bar draws through `player_heart_type` (NeoForge's `PlayerHeartTypeEvent`):
  when the vanilla player-health body draws its hearts, `kind` comes in as vanilla's pick (poisoned, withered,
  frozen or normal) and a mod may set another. Only the heart sprites change.

- Where each boss bar is drawn through `customize_boss_event_progress` (NeoForge's
  `CustomizeGuiOverlayEvent.BossEventProgress`): once a frame per bar, in the order the server added them, `x` and
  `y` come in as the bar's top-left corner (91 left of the centre; 12, then 19 lower for each bar) and `increment` as
  the spacing to the next (19). The bar and its title are drawn where a mod moved them, and the next bar comes in
  `increment` below where this one came in. A cancelled bar is not drawn and its increment still applies. Bars stop
  once the next would start a third of the way down the screen. With no listener nothing is built per bar.

- Where the in-game chat is drawn through `customize_chat_overlay` (NeoForge's `CustomizeGuiOverlayEvent.Chat`):
  each frame the chat layer draws, `x` and `y` come in as the chat's bottom-left corner (0 and the GUI height minus 40)
  with the GUI size and whether the chat screen is open, and the chat is drawn shifted to where a mod moved them. Not
  asked while the chat is hidden (F1 with the chat closed). Only the drawing moves; the input box and clicks on
  messages stay in vanilla's place, as with NeoForge.

- `entity_interact` and `ItemPart::item_interact_mob` run on both sides with native `HostSide`,
  version 2, answering `Option<InteractionResult>`; the latter includes the interaction hand.
  `None` defers; consuming results end the hand loop with the swing rules above; nonconsuming entity answers fall through.

- Left-clicking a block, on the server for every player: `left_click_block` hears the start of a dig
  (the block, the face hit) once the game allowed it, before the block is attacked (a note block plays) or broken in
  creative. A mod that cancels it keeps the block: on the server it isn't attacked, dug or broken, and the player's client
  gets it back. The clicking player's own client still predicts the crack until then.

- A player's tick: `player_tick_pre` and `player_tick_post` match NeoForge 26.3's
  non-cancellable `PlayerTickEvent.Pre`/`Post` on **both logical sides**, including
  ordinary single player's client window and server-thread players. Match the typed
  `HostSide`; the event's `player: PlayerRef` unambiguously names the ticked player
  (including other client-side players). On the server the helpers read that acting
  player. Pre precedes sleep/cooldown/body work; Post follows it, including item,
  arrow and XP pickups. Dead players still receive the pair without enabling dead
  actions. Players tick under `/tick freeze`, not while paused. With no listeners,
  the added per-player cost is only the indexed subscriber probe for each endpoint;
  listeners on the server pay an acting-guest enter/leave at each endpoint, reusing
  the already resolved guest index (no second connection lookup).

- Shape updates a mod can answer through `native_update_shape`: on an authoritative server, each time a
  block's state is worked out from a neighbour (`BlockBehaviour.updateShape`): neighbour shape updates after a
  `setBlock`, a block fitted to all its neighbours as a piston lands it or a chunk is post-processed, and the blocks at
  a feature's edge as it is placed in the live world, such as a tree grown from a sapling
  (`StructureTemplate.updateShapeAtEdge`). It gets the dimension, position, state, the neighbour's direction, position
  and state, before the block computes it. `state` may not be in the world yet (a piston landing it), so decide from
  `state`, not from the block at `pos`. Answer only for your own blocks and leave `new` empty otherwise: a state put
  there is the block's new state, applied as vanilla applies the result (air breaks the block, with its drops, on a
  neighbour update; post-processing and a feature's edge write air without drops), and the block's own `updateShape`
  doesn't run, nor the ticks it would schedule. The level is read-only, as vanilla's `LevelReader` is there; the hook
  schedules ticks with its `ticks` through the level's `native_schedule_block_tick(ticks, pos, block, delay)` and
  `native_schedule_fluid_tick(ticks, pos, fluid, delay)` (vanilla's `ScheduledTickAccess.scheduleTick`; false for an
  unknown name or a position that isn't loaded). It runs often, at least six times per `setBlock` that updates shapes
  (more for redstone wire's diagonals and knock-on updates): keep it short. A LAN guest shows vanilla's shape for a
  moment, until the server's update arrives. Chunk generation fires none.

- Random ticks a mod can handle through `random_tick`: on an authoritative server, each block
  `ServerLevel.tickChunk` random-ticks (`BlockState.randomTick`), with its dimension, position and state, before the
  block handles it. Only blocks that tick randomly fire it (a mod block as its vanilla template block does), once per
  draw that lands on one: it runs often, so keep it short. Bone meal's random ticks (`StemBlock.performBonemeal` calls
  `randomTick` itself) don't fire it. The hook may change the block with the level's
  `LevelWriteHost::set_block_property(pos, key, value, flags)` and set `handled` to skip the block's own random tick, so a
  mod crop grows its own way. Without `handled`, the block then takes its own random tick as it is now (if it still
  ticks randomly), and lava's fluid random tick runs either way. It needs the level mutably, which only `random_tick` and `tick_block` give. The tick's level writer uses vanilla's `Level.setBlock` flags via `BlockUpdateFlags`: a
  vanilla crop grows with `BlockUpdateFlags::CLIENTS` (neighbours' shapes update, so an observer fires, but no neighbour
  gets `neighborChanged` and comparators aren't told), a furnace sets its `lit` with `BlockUpdateFlags::NEIGHBORS | BlockUpdateFlags::CLIENTS` (both). Sunrise
  sends updates when `UPDATE_CLIENTS` (2) is set; `UPDATE_INVISIBLE` (4) suppresses notification only on the client, as in `Level.setBlock`. `UPDATE_IMMEDIATE` (8) has no effect yet. It writes at any loaded position, returns false off an authoritative server (the `Game` one too: a window that isn't a server), for a property or
  value the block doesn't have, a position that isn't loaded, or a property that already has the value (nothing is written:
  vanilla's `Level.setBlock` returns false for an unchanged state, as `LevelWriteHost::set_block` does), and true when the block changed. From a server Host, use `level.set_block_property(host, pos, key, value, flags)` with typed
  `BlockUpdateFlags` and handle its `LevelWriteError`; for a brewing stand's bottles, the flag is `BlockUpdateFlags::CLIENTS`.

- Scheduled block ticks a mod can handle through `tick_block`: on an authoritative server, each due
  scheduled tick of a block that is still there (`ServerLevel.tickBlock` → `BlockState.tick`), with its dimension,
  position and state, before the block handles it. The hook may change the block with
  `LevelWriteHost::set_block_property` and the flags the block's own tick would write with (an observer's tick
  uses `BlockUpdateFlags::CLIENTS`) and set `handled` to skip the block's own tick. Without `handled`, the block then takes
  its own tick as it is now, if it is still the tick's block.

- The commands a mod owns on the client, as NeoForge's `RegisterClientCommandsEvent`: push a `ClientCommandRegistration` made by `Command::new("x <mode:word>").executes_client(...)`. The chat display combines the two trees, while client and server execute in separate dispatchers. Only an unknown client command or argument is sent to the server. The client refuses nonzero operator levels, server-only argument kinds and custom argument types with a server suggestion provider. Asked at connect and when the server sends its tree, never per frame.
- The keys a mod registers on Options > Controls, as NeoForge's `RegisterKeyMappingsEvent`: call
  `Registrar::register_key_mapping(KeyRegistration::new("zoom", "key.keyboard.z").label("key.mymod.zoom"))`
  from `Hooks::register`. Ownership comes from the registrar; invalid or duplicate local ids refuse startup.
  `.category(...)` takes a translation key, `.modifiers(...)` sets shift/control/alt, and `.context(...)` takes
  `KeyConflictContext::{Universal, InGame, Gui}`. Only client startup collects the retained declarations for Controls;
  dedicated and integrated servers do not. One editor persists keys and modifiers in `sunrise-mods.txt`.
  Conflicts compare vanilla and mod bindings, context and modifiers, excluding unbound and both-default pairs.
  Resolve a `ModKeyMappingRef` once with `host.mod_key_mapping(owner, id)` during `client_setup`. Retain
  it for `binding(host)` (current name borrowed from the host, plus modifiers), `translated_key_message(host)`
  (current Controls label on demand), `is_down(host)` and `consume_click(host)` (`KeyMapping.isDown`,
  `consumeClick`). Any mod can `set_binding(host, key, modifiers)` or `reset_binding(host)` on a
  resolved mapping, which take effect from the next input and save by owner and id. `NotRegistered` is a missing owner/id at
  resolution; `Unavailable` means this host or phase has no client input access. `Stale` means a retained ref
  belongs to a replaced client or registration roster: resolve again there. Binding edits alone do not
  invalidate the ref. Persist names, not refs.

- Recipes of any type (`RecipeManager.getAllRecipesFor`), vanilla's (`minecraft:smelting`, `minecraft:campfire_cooking`) and a mod's own (`mymod:crusher`, for a machine or an altar) through one read: `host.recipes_of_type("minecraft:smelting")` gives each recipe in force, in file order, as a `Recipe`: its `id`, `kind`, `group`, typed `ingredients` and `results` (for shaped, shapeless and cooking recipes, as the game parsed them; a recipe the game cannot parse is not in force and not listed), `cooking_time` and `experience` for the cooking types, and its file's `json`, which is all a mod's own type has. The recipes are those of `data/<namespace>/recipe/` in vanilla, content mods (`.smod`) and the world's data packs, with data packs merged and load conditions held, so a pack's override or removal shows; a type nobody uses gives nothing. `Host::recipe_ingredient` and `Host::recipe_result` read the usual fields as JSON text, tags included (a bare id is read as `minecraft:`; items the game does not know are left out, and the helper returns `Ok(None)` if none is left). Invalid JSON returns `RecipeJsonError::InvalidJson` rather than panicking. The recipes are the server's table: on the server thread, a dedicated server or a LAN host; a client, the single-player window included, sees vanilla's and the content mods' recipes, not the world's data packs', unless a mod had the server send it the type (`native_datapack_sync`). All three give nothing outside a world. The game crafts the recipes of its own types itself, and shows none of a mod's own in the recipe book; `/reload` reads a data pack's again, so look them up again when you need them. The lookup borrows the table, with no copy.

- [mobs](#mobs) for server mods: spawn one, read the mobs of a level in place, and move, hurt or remove one by its id.

- Redstone from a mod's block, on authoritative servers. `block_signal` may change the signal a block gives on
  a face, weak or direct, and `is_signal_source` lets redstone wire connect to it. Both are asked very often, so check the
  block first and answer from the mod's own state. When the output changes, and once the block is placed (a direct signal past a conductor isn't read before),
  `level.update_neighbors(host, pos, state, NeighbourUpdate::Neighbours)` makes the blocks around it read it again
  (`Level.updateNeighborsAt`; a block that powers what it is attached to, as a lever does, also updates there). A
  diode's front takes `level.neighbor_changed(host, front, state)` then `NeighbourUpdate::NeighboursExcept(facing)`
  (`DiodeBlock.updateNeighborsInFront`). Both act on the level in place and are refused elsewhere.

- Item behaviour, on the server for every player: `hurt_enemy` hears a player's hit land on a mob with the
  held stack, `inventory_tick` runs each tick, on the `ItemPart`s of every stack a player carries (armour and off hand included), and
  the stack is the place `ItemPlace::PlayerSlot { player, slot }`, read and written as any item place, and
  `living_equipment_change` hears an equipment slot take a different stack.

- Hearing players and mobs on the server: `entity_join_level` for admissions, `living_death` before
  death and `living_death_post` after death effects, including player deaths and player kill credit;
  `item_toss` (`ItemTossEvent`: a throw, with the item entity made but not yet in the level, which the `item_entity_*` methods read and change; not the items scattered when a player dies; `canceled` keeps the entity out of the level and the items are lost) and `item_picked_up` (the ground item entity; `can_pickup`: `False` leaves the stack on the ground, `True` takes it despite its pickup delay or another player's target), then `item_picked_up_post` (`ItemEntityPickupEvent.Post`: the same item entity, a copy of the stack before the pickup and the live remainder on it, which a listener may set; heard on a whole or partial pickup, never when nothing went in). They name a player by UUID.

- [Server data reload listeners](#server-data-reload-listeners) read a mod's pack resources with datapack overrides, once per reload.
- [Client asset reload listeners](#client-asset-reload-listeners) read a mod's pack resources with resource-pack overrides, once per reload.

- For item entities near a block, visit `level.visit_entities_in(host, min, max, &mut visit)` and
  narrow each handle with `host.item_entity(entity)`. Read the stack with `item.with_stack(host, read)`
  and the delay with `item.pickup_delay(host)`. Retain the handle for later writes; `item.set_stack`
  changes its nonempty stack, and discarding the entity is a separate operation.

- `level.with_random(host, |rng| rng.next_int_bound(bound))` borrows the named server level's live
  stream (`Level.getRandom()`), including levels parked while the server ticks another. In a restricted
  level hook use `host.with_random(|rng| rng.next_int_bound(bound))` to borrow the hook's level stream.
  The borrow ends with the closure. Without an authoritative, resident stream to lend, including for a
  stale level handle, the call returns `None` and skips the closure. Draws advance the live stream
  directly and are not rolled back on panic/unwind or transaction abort, matching vanilla
  `Level.getRandom()`. A mod that sets its own seed uses a separate `LegacyRandomSource`.

- Server lifecycle hooks run once per start, on both the single-player server thread and a dedicated server,
  matching NeoForge 26.3 and vanilla's `MinecraftServer` order.
  `server_about_to_start(dedicated)` runs before worlds load (`ServerAboutToStartEvent`), after a successful dedicated bind.
  After the Overworld loads and initializes spawn if needed, other configured levels load in `LEVEL_STEM` traversal order.
  After level loading, adopted datapack/text setup,
  spawn preparation, and the initial world save, `server_starting(dedicated)` (`ServerStartingEvent`) runs on both
  dedicated and integrated servers, before accepting play.
  `server_started(dedicated)` runs after initialization completes and before the first tick (`ServerStartedEvent`),
  and before the integrated client receives its ready connection.
  On an orderly shutdown (when the tick loop exits normally), `server_stopping(dedicated)` (`ServerStoppingEvent`)
  runs before `stopServer` (network admission stop, player save/disconnect, final world save in level creation order,
  per-level close and `level_unload`, and storage release) and `server_stopped(dedicated)` (`ServerStoppedEvent`).
  On a failed open (including a dedicated TCP port bind failure before `server_about_to_start`) or a tick
  error/panic, `server_stopping` is skipped; `stopServer` still runs (saving adopted worlds only, closing levels
  with `level_unload` only after a successful save and per-level close, logging stop errors, and releasing storage)
  and `server_stopped` still fires unconditionally, even if the final save failed or a shutdown listener panicked.
  A failed open never overwrites a world the server did not successfully adopt: the session lock is taken before the
  prepared level is installed, and only that installation allows the final save. The first failure is retained if
  cleanup also fails. These are server-side, mutable `ServerHost` notifications, not cancellable. No tick-time cost:
  these hooks dispatch only at server startup and shutdown.

- Mod block entities are sent to clients. Only declared types receive wire IDs after vanilla's, and several valid
  blocks can share a type. Server and client use its declared identity; `block_entity` supplies only initial data.
  `block_entity_update_tag` gives what clients get of one (`BlockEntity.getUpdateTag`, read from the block entity
  alone): it is asked when a chunk holding it is sent and when its block changes, and the client merges it into the
  copy's `nbt`. A saved chunk sent past the view distance asks it on its own thread. Read its borrowed `entity.nbt()`
  or project individual slots with `entity.copy_item_slot(slot)`; a refused stack does not suppress the tag callback. Keep it to what clients show (a machine running, its facing, a tank's level), not a menu's progress:
  menus have their own data slots. `block_entity_data_received` notices merged client data with a `Chunk` or `UpdatePacket` source; empty chunk tags and absent packet tags are silent, while empty packet compounds notify. Related `IBlockEntityExtension.onDataPacket` and `handleUpdateTag` perform loading, not this post-merge notice; they are not counterparts.

- One-shot fields ride that tag. `view.set_client_update_fields(fields)` on a `BlockEntityView` queues them in
  the runtime under the entity's identity; they are not part of the plain-data entity,
  so saves, clones and comparisons never see them and its revision stays. Once a block change (queued by `BlockEntityRef::sync`)
  reaches viewers, the game builds one tag, puts the fields over `block_entity_update_tag`'s and sends it to
  them all. A change nobody sees, a chunk send and a single player's resend leave them waiting; removal or unload drops them.

- `affect_neighbors_after_removal` is a mod block's `Block.affectNeighborsAfterRemoval`: the block was
  removed from `pos` (`moved`: a piston pushed it) with `UPDATE_NEIGHBORS` or the piston flag. There is no bus
  event for it, so it is a `BlockPart` method only. The block's own class has already run its updates, as an
  override's `super` call; the part then pushes onto `updates` what it wants, in order: `NeighbourUpdate::OutputSignal`
  (`updateNeighbourForOutputSignal`), `Neighbours` (`updateNeighborsAt`) and `NeighboursExcept(direction)`
  (`updateNeighborsAtExceptFromFacing`), and the level runs each at once with vanilla's own functions. It runs
  inside the level's write with the level host, so it reads but cannot write blocks or spawn entities; a mod that
  needs either reports the gap on #44. The cost with no part on the block is one `has_part_hook` read.

- `visuals`: a world-frame hook that appends block-rim and
  colored-light requests (`BlockRim::new(pos, argb, width)`,
  `ColoredLight::new(at, rgb, radius, intensity)`). Rims trace the client's existing block outline edges,
  depth-tested against terrain. Lights add camera-local RGB illumination to the
  normal terrain/entity shaders, not gameplay light. Only the first eight rims
  and eight lights are examined; positions must be within 32 blocks of the player,
  values must be finite and bounded, and source chunks must be loaded. At most
  256 rim edges and 16 colored lights are rendered. Requests are frame-local,
  so disabling a native mod or leaving the world clears them without a retained
  state. It also adds `emissive`: a registration-context hook called at client
  startup and on every resource reload to register typed `EmissiveSpriteRegistration`
  pairs (`base` and `overlay` `Identifier`s). Transparent pixels in an overlay leave its
  base unchanged; opaque pixels are drawn full-bright without changing gameplay
  light or the surrounding rock. Overlay textures resolve from
  `assets/<namespace>/textures/<path>.png` during bake/reload without a per-mod
  quota, stay bound to the loaded render-asset generation, and share the GPU
  emissive uniform table with OptiFine overlays. When no overlays are active,
  the terrain shader skips the lookup.
  When a native mod bundles a Sunrise shader pack, naming its folder
  `native-<mod-id>` makes the Mods On/Off control pause that pack too (without
  changing the player's saved shader selection); other packs are unaffected.

### Custom ingredient metadata
`ingredient` receives its recipe metadata as an API `Compound`. Recipe JSON is
converted at load by the shared JsonOps-to-NbtOps boundary; mod code matches
`Tag` variants rather than JSON fields. Nested null refuses the recipe as an
unclaimed kind does; numeric and heterogeneous arrays remain lists. Read `event.data`
while claiming the kind and capture the parsed configuration in the predicate. Matching
borrows the live stack, not the recipe's JSON or its converted compound.

### Falling blocks

`crates/sunrise-client/src/game/modding/native_falling.rs`: falling-block spawning for server mods through a
`LevelRef`, sharing `Game::spawn_falling_block_entity` with `LevelEvent::FallingBlock`.

- `level.spawn_falling_block(host, state, pos, velocity, falling)` spawns a falling block of `state` at `pos` (its
  bottom centre) with initial `velocity` in blocks a tick (`Entity.setDeltaMovement`, kept exact on live state while
  network packets quantize through `LpVec3` on send). It flies, lands and places itself as sand does, or drops as an
  item where it can't. `Falling` holds `drop_item` (default true) and `hurts` (hurts what it lands on as an anvil
  does, 2 a block fallen and 40 at most; default false). Any valid block state is accepted, including air (which
  vanilla discards on its first tick in `FallingBlockEntity.tick`), with no per-level count cap.
- The block at `pos` stays: take it away first with `level.set_block` to make a block fall out of the world, as an
  explosion's debris or a collapsing tower would.
- The named open level can be parked. The call neither requires nor loads the destination chunk;
  section visibility and ticking govern when the admitted entity is active.
- It runs `entity_constructing` and `entity_join_level` at the call. A transaction in the destination level
  delays the join until root commit and makes the returned handle stale on abort or a veto at commit.
  `Ok(None)` means non-finite coordinates or velocity, exhausted entity ids, or an immediate join veto.
  `Err(LevelWriteError)` means a stale or closed level, a client, or an unknown block state.

### Bulk block edits

`crates/sunrise-client/src/game/modding/native_world_edit.rs`: `LevelRef::edit_blocks(host, edit)` for server mods,
on any open server level (active or parked).

- `level.edit_blocks(host, edit)` makes many blocks at once: `BlockEdit` holds a `palette` of block states and
  a list of typed `StatePredicate` filters (exact blocks or tags), and `runs`, each
  `height` blocks up column (`x`, `z`) from `y`, set to palette entry `block`, only where filter `only` passes the block
  there. With `surface` (`Heightmap`: `WorldSurface`, `MotionBlocking`, `MotionBlockingNoLeaves` or `OceanFloor`),
  `y` counts from that height of the column, read once a column before anything changes. Runs are made in order, each
  seeing the changes of those before it.
- The blocks go straight into the chunks, a section packed once however many of its blocks change, as `fill ... strict`
  does: no neighbour is told, nothing drops, and no block updates or ticks; light is worked out again for the chunks
  changed, and they go to the players whole. A block replaced by another loses its block entity, and a new block gets
  the one `set_block` would give it (a mod block's own, included).
- It answers the number of blocks changed. Runs in chunks that aren't loaded, and the parts of runs outside the level's
  height, are left out. It answers `Err`, changing nothing, on a client, for another dimension, when a palette state
  or filter name cannot be resolved, or when a run names a palette entry or filter past the end. Filter properties are
  checked when the mod makes the predicate; NBT is not representable.

`ClientHost::open_chat(text)` asks to open a chat screen containing `text`. It can replace the current screen, including another chat screen, and normal screen-opening hooks can cancel or replace it. Nothing is sent. Without a local player it returns an error.

`native_simulate_player_move(uuid, input)` plays one tick of a server player's movement from a bot's keys (forward, strafe, jump, sneak, sprint, yaw, pitch, the last velocity) with the client's own movement code, as `LocalPlayer.aiStep` and `Player.travel` do, and returns the position, velocity and ground it reaches without moving the player; `LevelRef::visit_block_collisions(host, aabb, &mut visit)` lends each intersecting shape, as `Level.getBlockCollisions(null, aabb)`.

## Client brand

`PlayerRef::client_brand(host)` answers which client a player joined with, from the brand string
the connection recorded (`ServerPlayer.clientBrand`). Sunrise's own clients say `sunrise`, which
is what the game itself checks before it sends them Sunrise-only packets, so a mod that does the
same asks here rather than guessing from the name.

## Selected slot, dropping a stack and closing a menu

`PlayerRef::set_selected_slot(host, slot)` makes `slot` (0 to 8) the hotbar slot a player holds
and tells their client with `SetHeldSlot`; a new slot ends a main-hand use in progress, as the
client's own switch does, and `player_switch_hotbar_slot_pre` is not asked, since it is for the
player's own input; it reaches the client of a guest acting in their own turn too.
`PlayerRef::drop_stack(host, stack, thrown_by_player)` is `Player.drop(stack, false,
thrown_by_player)`: the stack is thrown from the eyes as an item entity and `item_toss` may
cancel it. It takes nothing from an inventory, so take the stack out through the item places
first; that is what separates it from giving a stack that falls back to a drop when the
inventory is full. It returns `Ok(None)` once the item is in the world, and otherwise hands the
stack back untouched as `Ok(Some(stack))` (empty, a stale or dead player, a vetoed toss) for the caller to put back,
so a refused drop never loses items. As with other checked stack writes, a failure to convert
the API-owned stack returns `StackWriteError` before any game operation. API stacks are nonempty;
the engine empty-stack refusal remains internal.

`MenuRef::close_container(host)` closes a player's open menu on a server as
`ServerPlayer.closeContainer` does. A live handle close queues `ContainerClose` before tearing
down the menu and returning the carried stack, and fires `player_container_close` once. It
returns `false` when a disconnect is pending. The close runs immediately outside a transaction.
Inside one, the request closes that same window at outermost commit; an abort leaves it open.
The player's own inventory menu is never closed. A client closes its own screen with `MenuRef::close`.

## Recipe book view

`player.visit_discovered_recipes(host, &mut visit)` visits every recipe the player's book knows;
`has_discovered_recipe` asks about one at a time. The visitor borrows each qualified recipe id as
`&str`; copy it inside the callback if you need to keep it. Return `ControlFlow::Continue(())` to
keep visiting or `ControlFlow::Break(())` to stop. An empty book completes without a callback;
an unavailable or stale player gives `EntityReadError` instead. The visit reads the resting
guest's book outside their turn and their acting book during it. Use `award_recipes` and
`reset_recipes` to change the book.

`host.commands().refresh_commands()`, on the server's `Commands` service, asks every mod's `register_commands` again at the start of the next server tick, rebuilds the commands players may run, and sends every player on the server their tree again after `player_commands_sent`, as vanilla's `PlayerList.sendPlayerPermissionLevel` (`Commands.sendCommands`) does after `/reload`. Nothing happens where no world is open to players.

The new server dispatcher binds fresh executors to its accepted usages. A parse from the old dispatcher cannot execute after the rebuild. Players receive the rebuilt tree without its executors.

`sound_play` intercepts actual client sound starts and live parameter updates. `SoundPlayback` identifies the instance, event, source, position, relative flag and whether it is starting. Edit base volume/pitch transiently, or cancel before subtitles/backend writes; invalid edits cancel. Use `play_sound` and `SoundPlay::effects` to edit the gain, filters, and reverb sends. No Host/world borrow is supplied. Runs on the game thread; use existing client hooks for world/acoustics state.

`level.world_border(host)` reads the bounds of the resident server level named by `level`, without opening or switching levels.

`native_world_seed()`, the world's seed as `/seed` prints it (Bukkit's `World.getSeed`), the one every level generates from; None off a server.

## Discoverable bundled packs

`Hooks::add_pack_finders` receives a `&mut dyn PackDiscoveryContext` and an
`AddPackFindersEvent`. It provides local text operations before a game exists.
On live rediscovery, `context.live_host()` lends the game's `HostSide`; whether
that host has an open world is separate. Use `event.packs.pack_type()` to distinguish
`PackType::ClientResources` from `PackType::ServerData`. Register a
`PackRegistration` with `event.packs.add_pack_finders(...)`; registrations for
the other repository type are ignored. This hook contributes whole packs, while
resource-file hooks and reload listeners retain their separate roles. Native
mods disabled in `sunrise-mods.txt` are skipped before their listener is
entered, from the first startup discovery onwards.

A location such as `example:assets/example/resourcepacks/high_contrast` names a
loaded mod and a directory inside its package, containing `pack.mcmeta` and the
usual `assets/` or `data/` tree. Folder packages and `.smod` archives use the same
logical paths, without extraction. Invalid locations, missing metadata and
duplicate repository ids return and log a `PackFinderError` without adding a partial pack.
A panic discards only the current listener's packs.

The repository id is `mod/<location>`. Packs are optional by default: resource
packs appear in Resource Packs, and data packs appear in the world's Data Packs
screen and `/datapack` repository. Selecting a resource pack applies it through
client reload; data-pack selection applies through the world's data reload.
Discovery runs again for reload, so replacing an archive opens its new contents.
Use translated `Text` for the registration's title and pack metadata description.

`always_active` makes a pack required and prevents deselection. `position`
chooses its default insertion priority (`Top` or `Bottom`); later resource
layers win. Required data packs also enter a new world's initial selection when
the player does not open the Data Packs screen. `PackSource::Feature` exposes
packs with declared feature flags in Experiments. Their flags must be enabled
for the world before `/datapack enable` accepts them, and feature packs that
request flags cannot be disabled through that command.

`hidden` keeps a pack out of the Resource Packs, Data Packs and Experiments
screens, for a compatibility pack or texture override players should not see.
A hidden pack still loads when selected, so pair it with `always_active` for one
that is always on. `/datapack list` and `/datapack enable|disable` still name
hidden data packs for operators.

## Section geometry

Section geometry (NeoForge's `AddSectionGeometryEvent`) is quads a mod adds to a chunk section's own mesh, for
geometry that belongs to the terrain but no block model draws: a mod's wires between blocks, markings on the ground.
`Hooks::add_section_geometry` registers the mod's `SectionGeometry` once at start, and each time a section meshes
the mesher asks its `add(origin, context)` to add its quads, after the section's blocks:

```rust,ignore
struct Marker;
impl SectionGeometry for Marker {
    fn add(&self, origin: [i32; 3], context: &mut dyn SectionGeometryContext) {
        if origin != [0, 64, 0] {
            return;
        }
        let light = [context.level().brightness(LightLayer::Sky, [8, 65, 8]) as u8, 0];
        context.add("minecraft:block/gold_block", SectionQuad {
            positions: [[7.0, 1.01, 7.0], [7.0, 1.01, 9.0], [9.0, 1.01, 9.0], [9.0, 1.01, 7.0]],
            uvs: [[0.0, 0.0], [0.0, 1.0], [1.0, 1.0], [1.0, 0.0]],
            color: 0xFFFF_FFFF,
            light,
            translucent: false,
        });
    }
}

fn add_section_geometry(&self, _context: &RegistrationContext, event: &mut AddSectionGeometryEvent<'_>) {
    event.renderers.register(Arc::new(Marker));
}
```

Positions are in blocks from the section's lowest corner, each 0 to 16, UVs within the sprite (0 to 1), and the
sprite a block atlas sprite's name. What a renderer adds to a section is taken whole or refused whole, with one log
line: a corner or UV out of range or not a number, a sprite the atlas lacks, or a quad past a budget refuses all of
it, and `context.add` answers false from then on; the section keeps its own blocks. A section's blocks and every
renderer's quads share one budget, 262144 quads and 32 MiB of mesh data (`context.remaining()` says what is
left), and the sections meshing at once share 128 MiB of added quads not yet in their meshes. A renderer that panics
is asked no more. `context.level()` reads the section's chunk and its eight neighbours, as a block tint source's
`level` does. The renderer runs on the meshing workers, so it keeps no mutable state and gets no `Host`; it is asked for every
section the client meshes, also one that holds only air, and returns quickly when it has nothing there. Its quads
are shaded only by the colour and light it gives them. A section meshes again when its blocks change, when the
resources reload, and when a mod calls `ClientHost::refresh_render_visibility` and the hidden blocks change.

## Tree ground

- **`alter_ground(context, event)`**: NeoForge's `AlterGroundEvent`, fired where `AlterGroundDecorator.place` has found
  the positions it places around and before it places any ground. `event.provider` starts as
  `GroundProvider::Decorator`, the decorator's own provider; a mod sets it to `GroundProvider::Mod` with its own
  `GroundStateProvider`. The decorator then asks that provider once per position it tries, top down in each column
  (`state_at(level, pos)`): a canonical `BlockState` is placed there and the column stops; `None` tries the next
  position down. A provider reads canonical states through `level.block_state(pos)`, draws from the decorator's
  own random through `level.random()`, and asks the provider it wrapped with `GroundProvider::state_at`
  (`level.decorator_state(pos)`
  for the decorator's own). With no listener, or with `provider` left as `Decorator`, the decorator runs exactly as
  vanilla's, blocks and draws alike; a provider that only wraps the decorator's own keeps them too.
- **Where it runs.** On the worldgen worker threads while chunks decorate, and on the server thread when a sapling
  grows into a tree with this decorator. There is no `Host`: `context` is a `WorldgenHost`, the server's registries,
  settings and mod list as they were at its last resource load (start or `/reload`). A provider is shared by the
  workers, so it keeps no mutable state; the same seed must give the same ground.
- **Answers.** A state the game's registry doesn't have stops the tree feature, as an invalid state in a data pack
  does.
- **Cost.** With no listener the decorator pays one subscription check per decorated tree and allocates nothing.
  With a listener, one event per tree; a mod's provider is asked per position instead of the decorator's.

### Random display ticks: sparks, smoke and dust

`animate_tick` is vanilla's `Block.animateTick(BlockState, Level, BlockPos, RandomSource)` for a mod's block:
the client's ambient block effects, without a block entity. Each client tick `ClientLevel.animateTick` picks
667 random positions within 16 blocks of the player and 667 within 32, and each picked position's block draws
its effects there; a block near the player is picked more often than one far away. A part on the picked
state's block is called once for that pick, inside it as vanilla calls a block's `animateTick`: after the
block's own vanilla effects, before its fluid's effects and the next pick. It gets the client's level, the
position, the state and the level's random.

Draw from `event.random` to keep a port's odds and order (`random.nextFloat()` is `next_float()`,
`nextInt(bound)` `next_int_bound(bound)`, `nextDouble()` `next_double()`, `nextBoolean()` `next_boolean()`).
Add particles with `host.add_particle(&options, at, velocity)`, which applies the camera distance and the
Particles option as `ClientLevel.addParticle` does, and play sounds with `host.play_local_sound`
(`ClientLevel.playLocalSound`). Read the block's neighbours through `event.level`. A block that should
always show its effects, a machine's working smoke, belongs in `block_entity_client_tick` instead: a picked
position is a matter of chance.

The sampler asks the per-block table whether the picked state's block has a part answering
`animate_tick`, so a block without one costs one lookup and no allocation, and with no part answering it
nowhere the sampler asks nothing. `random` is the level's random itself: the part's draws and the game's own
during the call (`host.add_particle`'s roll under the Decreased Particles option) move the one stream in call
order. It never runs on a server, and a screen's block scene (`animate_scene`) does not call it.

## Baby entity spawn

- **`baby_entity_spawn(dimension, parent, partner, player, player_entity_id, child, cancel)`** (NeoForge's `BabyEntitySpawnEvent`, version 4): fires after breeding has attempted to construct the baby (`Animal.spawnChildFromBreeding` / `Fox.FoxBreedGoal.breed`) and before breeding snaps the baby, finalizes breeding, and inserts it. Even when no initial baby was constructed, the hook fires with an empty slot. `child` (`BabyEntitySpawnChild<'_>`) lends the single optional breeding slot directly on the stack: inspect the current baby with `event.child.get()`, edit it in place with `event.child.get_mut()`, construct a new baby with `event.child.construct(host, PendingMobSpec::new(kind, dimension, position, yaw, true))?` and swap it in with `event.child.replace(baby)?`, or clear the slot with `event.child.take()` / `event.child.clear()`. Creating a detached child fires `entity_constructing` immediately; discarding it cleans up its unpublished state on drop. A replacement must be an `AgeableMob` (`minecraft:fox` in fox breeding), but may start as an adult. Invalid replacements answer `Invalid`. The event runs before either parent's breeding cooldown.
- A panicking listener is disabled for this hook by the shared dispatcher. Its cancellation flag is restored, but direct writes to the child and slot are not journaled and remain visible to later listeners.
- Cancelling (`Flow::Cancel`) resets both `parent` and `partner` to age 6000 and love 0, clears breeding credit, and spawns no baby, experience, `animals_bred` statistic, advancement or hearts. Returning `Flow::Continue` with an empty slot spawns no baby and skips breeding finalization without resetting the parents or consuming credit (matching NeoForge's `Animal.spawnChildFromBreeding` and `Fox.FoxBreedGoal.breed`). The inheritance draws already made stay spent.
- On acceptance, ordinary animals set the child's baby age and snap it to the initiating parent's live position with zero yaw and pitch before awarding credit. Foxes grant breeder trust and award credit using the child's pre-snap age and position, then reset the parents before setting baby age and snapping the child. Both paths insert the selected child once.
- Turtle and frog breeding, sniffer egg laying and turtle hatching make eggs, not babies, and do not fire it. Armadillos breed through the ordinary animal path and do. Villagers breed through their own behavior and do not (a separate moment).

### Server block changes

`block_changed` observes a committed authoritative server `Level.setBlock` change before block side effects and
neighbour updates. It supplies the dimension, position, old and new states, and the `BlockUpdateFlags` applied to the
write. `KNOWN_SHAPE` skips the shape passes but does not suppress this notice. The hook does not observe equal-state
writes, chunk loading or direct storage writes that bypass `Level.setBlock`. Each listener gets its own event.

## Block drops

- **`block_drops(level, dimension, pos, state, breaker, breaker_entity, block_entity, block_entity_data, tool, drops, experience, cancel)`** (NeoForge's `BlockDropsEvent`): `Block.dropResources` prepared the item entities but has not joined them to the level. Fires for normal player, mob and level-side breaks, not for explosions, dispensers or other loot-table drops. Use `can_drop_from_explosion` to gate blast drops or `loot_drops` to change blast stacks before merging. `breaker` is the player's UUID; `breaker_entity` is the player or mob whose break runs, if known. The original block entity is lent as a resident `block_entity` or, if removed or stored only in the chunk, as `block_entity_data` (read-only, including `CustomName`, without identity or position). `tool` lends an `ItemStackView`; call `project()` to copy components. `drops` is a live `PreparedBlockDrops` view with positions and motions already drawn. Inspect fields without projecting components, edit in place with `PreparedDropMut` to keep the original RNG, or add/replace/copy with owned `ItemDrop` captures. Stack writes validate individually with `StackWriteError`. Items join before the experience award. Cancel suppresses items, XP and `spawnAfterBreak` effects, not bees released by `playerDestroy`. With the `block_drops` gamerule off, positions are still drawn but the list starts empty and neither items nor XP spawn. Replaces `block_exp` with the event's `experience`.

### Client ticks: hums, smoke and moving parts

`block_entity_client_tick` is vanilla's client `BlockEntityTicker` (`EntityBlock.getTicker` asked with the
client level): what a machine does on the client alone, a looping hum, smoke from its chimney, a part eased
between two server updates. Implement the method on a `BlockEntityPart` attached to the type. It binds the
client ticker for that type's accepted states. Use `BlockEntityTypeBuilder::ticker_when` to select only some
states or sides; with no interested Part, nothing is bound.

It runs on each client (the window's own in single player included) once per client tick while its level runs
(not paused or frozen), for each loaded copy of the type, in vanilla's `Level.blockEntityTickers` order.
A chunk's initial tickers join in packet order; a state change that adds a ticker joins it when enabled.
A loaded copy ticks whether or not it is drawn (`block_entity_submit` runs only for those in view). `event.subject` is a
`BlockEntityClientScope`: call `event.subject.with_view(|view| ...)` to read the resident copy's handle,
position, level, block state, data slots, item slots and merged NBT directly with no per-tick snapshot copy,
then call `event.subject.host()` for the `ClientHost` to play a sound with `play_local_sound` (keeping its
`SoundRef` to stop or fade it) or add particles with `add_particle`. The resident data is read only and holds
what the frame's handling of the server's packets merged into the copy (`block_entity_data_received` told
first): a frame handles what has arrived and then runs its client ticks, so all the ticks of one frame see
the same data and an update arriving during a frame is seen from the next frame's first tick. A copy that is
removed, replaced by another type, changed to a state without a client ticker or whose chunk unloads is not
called again from that tick on, and its handle no longer reads once it is removed, replaced or unloaded: stop
what you keep for it then. It never runs on a server.

`block_entity_on_load` is the per-instance attachment notice, after saved data is applied and before ticking. It also runs for fresh placement and for client chunk instances. The factory remains a separate creation step. Its counterpart is `IBlockEntityExtension.onLoad`.

## Add blocks to block entity types

`Hooks::block_entity_type_add_blocks` runs once after content declarations and before
worlds load, with a `RegistrationContext`. Retain block and type keys from `register`.
Call `event.modify(kind, &[block])`, or use `event.modify_matching(|kind| ..., &[block])`
to extend every matching type. Inspect the registered keys through `event.types`; the predicate
sees those keys in order. These methods stage additions; they cannot remove earlier memberships.

Validity is many-to-many. An addition allows an existing entity of that type to remain
on the block; it does not select the block's factory. A declared native type still
owns creation, and a content block otherwise inherits its original vanilla template.
A vault template associated with `minecraft:vault` creates, ticks and saves a vanilla
vault, without declaring another entity type. Signs retain their template defaults.

As in NeoForge's public `BlockEntityTypeAddBlocksEvent` javadoc, a block must inherit
the common superclass of the targeted type's original valid blocks. Invalid keys or
classes fail startup, duplicate additions are idempotent, and a resource reload cannot
change the frozen validity sets or wire IDs. No listener means no added memberships.

## Block jump and speed factors

- **Block Part methods `block_jump_factor(context, event)` and `block_speed_factor(context, event)`**: edit `event.factor`, the factor of the block state `event.state` the part is attached to (NeoForge's `Block.getJumpFactor()` and `Block.getSpeedFactor()`; vanilla's values are 0.5 jump and 0.4 speed for honey, 0.4 speed for soul sand, 1.0 elsewhere) where an entity reads it. `factor` starts at the block's own value. The edit is made on each read of that block, before the entity combines the blocks as vanilla does: the jump factor of the block at the feet unless it is 1.0, then the block below; the speed factor of the block at the feet unless that is water, a bubble column or not 1.0, then the block below. So a part that gives its block 0.5 slows an entity on it just as honey does, and one that gives 1.0 to honey lets the block below decide. Read for the player, every mob's ground and jump path (the rabbit's hop included), boats and minecarts where vanilla reads the speed factor (`Entity.move`'s last step). They fire on both sides and on the client's replayed ticks, so a mod registers on both and keeps each listener a pure function of its inputs. `context` is the loop context of the side that ticks the entity, as for the motion value hooks above. The player's `movement_efficiency` still applies after the part's factor. There is no global method: NeoForge has no bus event for these, so a mod reacting to its own blocks attaches a part and one reacting to every block listens to `living_jump_power` and `living_speed`. With no part attached to the state, the owning game checks the hook subscription and prepared API block key before any event conversion; with none installed the movement result is bit-identical to vanilla's. Beyond NeoForge (no event); Diverges from upstream: new hooks.

## Villager farming

Farmers recognize mod blocks made from farmland and `CropBlock` templates, including declarations from
`blocks`. A crop block's generated item is its seed: add that item to
`minecraft:villager_plantable_seeds` to let farmers plant its default block state from their inventory.
Untagged items and tagged items without a block are skipped. Crop maturity follows the template's class:
beetroots mature at age 3, ordinary crops at 7; torchflower crops never reach their maximum age 2 while
still crops. Pitcher crops are not `CropBlock`s and are not harvested by this behavior. Farming still
requires a farmer at work and `mob_griefing` enabled.

## Blocks fences, walls and panes connect to

- **`BlockPart::can_be_connected_to(level, event)`** (`CanBeConnectedToEvent { dimension, pos, state, connector, kind, direction, face_solid }`): the block beside a fence, a wall, or iron bars or a glass pane answers whether that connector connects to it (NeoForge's `IBlockExtension.canBeConnectedTo`). `event.state` at `event.pos` is your block, the target; `event.connector` is the connecting block's state, `event.kind` its vanilla rule (`ConnectionKind::Fence`, `Wall` or `Bars`), `event.direction` the way from your block to the connector and `event.face_solid` whether your block's face toward it is sturdy. `Some(true)` connects, even where vanilla would not (a lantern beside a fence); `Some(false)` refuses, even beside a sturdy face; `None` leaves it to another part and then vanilla's rule: a fence takes a sturdy face that is not an exception, a fence of the same kind or a gate across its arm; a wall a wall, a sturdy face, bars or a pane, or a gate across its side; bars and panes a sturdy face, bars or a pane, or a wall. A wall's connected side is still low or tall from the block above it. The connector's own part is asked after, in `connects_to`, with `event.connected` starting at this answer: the connecting block has the last word (Sunrise's order). Asked when the connector is placed and each time it settles its shape toward your block. A panicking part is contained with a warning and vanilla's answer is used. Authoritative servers only; the level is read-only. With no part attached to the state, one table lookup per side asked. Diverges from upstream: new hook.

## Block drops from explosions

- **`BlockPart::can_drop_from_explosion(level, event)`** (`CanDropFromExplosionEvent { pos, state, explosion }`): whether the block an explosion destroys drops (NeoForge's `IBlockExtension.canDropFromExplosion`), asked once per destroyed block where `BlockBehaviour.onExplosionHit` asks `Block.dropFromExplosion`, before anything drops. `Some(false)` skips the drop: no loot, and none of the block's `spawnAfterBreak` (an infested block's silverfish, an ore's or a spawner's experience). `Some(true)` drops, even for TNT, which vanilla keeps from dropping. `None` leaves it to another part and then vanilla. The block is removed either way, `on_block_exploded` still hears it, and a TNT still primes: `TntBlock.wasExploded` is a separate moment. A panicking part is contained with a warning and vanilla's answer is used. Authoritative servers only; the level is read-only. With no part attached to the state, one table lookup per destroyed block. Diverges from upstream: new hook.

## Blocks an entity may destroy

- **`BlockPart::can_entity_destroy(host, event)`** (`CanEntityDestroyEvent { pos, state, entity }`): whether the entity `event.entity` may destroy the block (NeoForge's `IBlockExtension.canEntityDestroy`), asked where vanilla asks its per-block rule: the wither's block break (`WitherBoss.canDestroy`: any block but air and `#wither_immune`), a dangerous wither skull's blast, where the entity is the skull (kept in the level and readable through the blast) and only a block it may destroy has its explosion resistance capped at 0.8 (`WitherSkull.getBlockExplosionResistance`), the dragon's `checkWalls`, a zombie's door breaking (`BreakDoorGoal.canUse`) and the turtle-egg `RemoveBlockGoal`'s target choice (`isValidTarget`). `Some(true)` makes a block vanilla spares destroyable for that entity, such as `#wither_immune` bedrock for the wither; `Some(false)` spares one it would break, and a spared block in the dragon's way puts it in a wall. `None` leaves it to another part and then vanilla's rule. The order is: the griefing gate first (`entity_mob_griefing`, the `mob_griefing` rule), so with griefing off nothing breaks whatever the part says, then this answer, then `living_destroy_block`, which can still keep a block the part allowed (fired at the dragon's check and at the door's break; the egg goal's target choice fires none). For the dragon, `#dragon_immune` still holds after all of them: a yes does not bypass it, and a `#dragon_transparent` block is skipped before the part is asked. Ravager: griefing event only. A panicking part is contained with a warning and vanilla's rule is used. Authoritative servers only; the host is read-only. The dragon asks every tick for each block in its head, neck and body, so answer fast. With no part attached to the state, one table lookup per block. Diverges from upstream: new hook.

### Client attack targeting

`client_attack_target` runs with a ClientHost from the remote client's start-attack input path after interaction cancellation and before ordinary melee dispatch. Spear jabs bypass it. Cancellation suppresses attacks; an explicit target batch is deduplicated in first-occurrence order and the unchanged crosshair entry preserves its committed tick-pick point and occlusion policy without re-hitting it. Additional targets are checked for lifetime, reach, held-item range and occlusion. The server remains authoritative.

## Client block changes

`client_block_changed(dimension, pos, old, new, predicted)` runs on a client of a server
after each block of the client level changes state, with the level already reading `new`. It
is the per-block counterpart of `client_chunk_load` and `client_chunk_unload`, so a client map
can invalidate just the chunk that holds `pos` instead of rescanning the loaded terrain. Every
route a client writes a block by tells it: the server's `BlockUpdate`, each block of a
`SectionBlocksUpdate`, the client's own predictions (`predicted` true) and, when the server
acknowledges them (`BlockChangedAck`), each block that takes the server's state back
(`predicted` false). A server update for a block still under a pending prediction is held, as in
vanilla, and told when the acknowledgement applies it. A write that keeps the state is not told.
An integrated world's own blocks are the server's `block_changed`; a level the server opens for
viewing is not told.

## Client break progress

`client_break_progress(dimension, breaker, pos, progress)` runs on a client of a server each time
the game tells its client level of a crack (`ClientLevel.destroyBlockProgress`). `breaker` is the
entity id of whoever cracks the block; `progress` 0 to 9 is the crack stage, and any other value
takes that breaker's crack away. Two routes tell it, as in vanilla: the server's
`BlockDestruction` packet (`ClientPacketListener.handleBlockDestruction`), and the local player's
own dig, with `breaker` the player's own entity id (`MultiPlayerGameMode.startDestroyBlock` and
`continueDestroyBlock` with the dig's stage, -1 before it has progress; `stopDestroyBlock` and a
finished break with -1). A server that runs mining itself shows the player's own crack only
through the packet, so this is how a client mod hears how far that block has cracked. A mod's own
`set_client_break_progress` is not told back to it. The hook only reports: the crack is shown as
vanilla shows it whatever the listener does.

## Client chunk load

`client_chunk_load(dimension, chunk)` runs on a client of a server, a LAN
guest included, once for each `LevelChunkWithLight` packet the client adopts
(vanilla's `ClientChunkCache.replaceWithPacketData`). It runs after the column
and its block entities are in the client level, so a listener can read the
column's blocks through `ClientHost` (`host.level(dimension)`, then
`level.block_state(host, pos)` or `level.chunk_ref(host, chunk)` -> `ChunkRef::snapshot(host)`
for an immutable off-thread snapshot). A column the server sends again while it is
resident is told again, without an unload between. A biome packet
(`ChunksBiomes`) tells nothing. `dimension` is the client level's dimension, as
the server named it. An integrated world's own chunks have the server's
`chunk_load` instead, and that hook is not called on the client.

## Client chunk unload

`client_chunk_unload(dimension, chunk)` runs on a client of a server once
for each column whose data leaves the client level, while its blocks and block
entities are still readable, just before they are freed. Every route that frees
a column tells it: the server's `ForgetLevelChunk` packet, a newer column
taking the slot of one that is out of view, the view radius shrinking, and the
client level going (a dimension change, a respawn into another level, leaving
the server). The view centre moving away tells nothing: as in
`ClientChunkCache`, the column stays in its slot out of view, and it is shown
again untold when it is back in view, with no `client_chunk_load`. A column
that is already gone is not told again. NeoForge tells only the forget packet
(`ClientChunkCache.drop`); Sunrise tells every route that frees a column, so a
listener never keeps state for a column the client no longer holds. `dimension`
is the level the column belonged to: on a dimension change, the old one.

## Decoded client packets

`Hooks::client_packet` receives `&mut dyn ClientHost` and a `ClientPacketEvent<'_>`
before the window handles a decoded server play packet. `event.packet` borrows
an immutable callback-lifetime `mod_api::ClientPacket<'a>`: `id: &'a Identifier`
identifies the packet type (for example, `minecraft:custom_payload`); `custom_payload`
is `Option<ClientPayload<'a>>` (`channel: &'a str`, `data: &'a [u8]`) for a custom
payload, or `None` otherwise. An id outside the protocol's packet table is reported as
`sunrise:unknown`. Other decoded fields and mutable packet bytes are not exposed. Copy
only what you retain after the callback returns (`id.clone()`, `channel.to_owned()`,
`data.to_vec()`). With no listener, the receive path checks one flag and builds nothing.

The hook cannot cancel or replace a packet. It also observes chunk packets
handled by the batching path, once each. Configuration and control packets (including keep-alives,
chunk batch starts and finishes, and resource pack messages) do not fire it. Standalone chunk batch
controls run on the receive thread; bundled controls run with their members on the main thread.
The chunks between a batch's start and finish still fire it.

`ClientHost::hold_packets` gives the calling mod its own hold. Overlapping holds share one
ordered queue; releasing one cannot send past another. Dropping one discards only packets it
covered, including overlaps, without ending any other hold. Held packets keep their ordinary
writer credit and are never flushed to make room. Disconnect, protocol incarnation changes and
aborted travel invalidate holds. Keep-alives, chunk batch acknowledgements, configuration,
cookies and pack answers bypass holds, standalone or bundled. A hold exposes no packet bodies.

`Hooks::client_packet_send` is the other direction: a `ClientPacketSendEvent<'_>` with the same
borrowed `ClientPacket<'_>` view for each play packet the window's game is about to send
(`Connection.send`), in order, including the selected slot sent before an attack or a use.
Returning `Flow::Cancel` drops the packet unless a later `#[listener(receive_cancelled)]`
listener sets `event.cancelled = false` and returns `Flow::Continue`. Play-phase pongs pass
through this hook. Control replies (keep-alives, chunk batch acknowledgements, configuration
pongs and cookies), configuration packets, the phase-dependent answers (resource pack status
and a dialog's custom click action), and far terrain requests do not fire it. With no listener
the send path checks one flag and builds nothing.

Both hooks give later listeners the original borrowed view. A panic logs and shows an error
toast naming the mod, disables only its callback for that hook, and leaves any accepted send
cancellation unchanged. Later eligible listeners still run.

## Client presentation and lifecycle

`Hooks::menu_background` can adjust the requested blur radius or set it to
`None` before the game validates it.

`Hooks::entity_outline` receives the entity id, kind, and mutable optional RGB
outline for the current client render. Leave the existing value unchanged to
preserve vanilla glowing; `None` disables it. These read-only client hooks
receive `&dyn ClientHost`. They do not grant server writes.

`Hooks::client_player_logging_out` runs before client disconnect teardown
(`Minecraft.disconnect`), with the live local `PlayerRef` and `GameMode` when
present and `None` on no-player transitions, not for a dimension change or
reconfiguration. Initialization and final shutdown still use the existing
`client_setup` and `client_stopping` hooks, with no parallel aliases.

## Pickups a client sees

`client_take_item_entity(collected, collector, amount)` runs on a client of a server when the
server tells it a nearby entity picked something up (`ClientboundTakeItemEntityPacket`), before the
client shrinks or removes what was taken. A client-only mod can list recent pickups, or tell the
local player's own (`collector` is the local player's entity) from other players' and mobs', on
any server; the server-side pickup hooks never run on a client of an unmodded server.

## Collision shape by the querying entity

- **Block Part method `collision_shape(context, event)`.** Set `*event.shape = Some(shape)` to override the shape for this query (`BlockBehaviour.BlockStateBase.getCollisionShape`; NeoForge adds no event here). Construct a block-local `VoxelShape` with `VoxelShape::try_from_boxes`; if construction fails, leave the answer unchanged. `None` selects the normal, contextual or block-entity-published shape. If a Part panics, only that invocation's answer is discarded; earlier answers stand and later Parts still run.
- **Who asks, and when.** Entity movement (`Entity.move` and its step-up retries), placement checks, projectile paths and the other reads that take a `CollisionContext`; never fluid passage or face support. A part is asked for every query that meets an opted-in state, as vanilla asks `getCollisionShape` for every block of every sweep: a step-up retry or a second sweep asks again, and a change to the mod's own state inside a tick is seen by the next query, on both sides. A collision query made inside the callback gets the empty-context answer (the block's normal shape), never a nested call. The per-state shape cache stays the empty-context answer, so a block still counts as a large shape where its normal shape is one. It runs on the thread that owns the level, the server tick and the client's own prediction; any other thread (the mesher, world generation) reads the normal shape.
- **Why there is no `Host`.** A move holds the level's collision data and cannot lend a `Host` to a mod in the middle of it, as for `entity_move`, `friction` and the other loop-context hooks. The part answers from the query and from state it keeps itself: a rule that reads more than the query (a conveyor that is solid only for the entities it carries; powder snow reads a fall distance) tracks that state in the mod, keyed by `query.entity()`, and syncs it so the client computes the same answer. If a real mod needs an entity read it cannot track itself, one captured field is added to `CollisionQuery`; the `Host` is not reopened.
- **The client's duty.** A client that disagrees with the server about the mod's own state predicts a different shape; the server then corrects the player's position (the usual "moved wrongly" correction, not a kick). Sync the state the rule reads.
- **Not the selection shape.** The outline a player targets is the block's declared shape, or the outline its block entity publishes; this hook is never asked for it, the interaction shape or the camera clip. A block can be solid and unselectable, or passable and selectable.
- **Cost.** A block with no part that answers it costs nothing. An opted-in block costs one call per query that meets it. Diverges from upstream: new hook.

## Console line

- **`console_line(host, event)`** observes operator log lines on dedicated and integrated servers. The server thread delivers them in order at the next tick's start. The queue keeps the oldest 4096 lines and drops newer ones until the next drain; the first event of that batch reports the count in `dropped`, with zero on every other event. It keeps 8192 bytes of message and 128 of thread name per line at character boundaries. Nothing is captured or queued while idle, and a listener's own log calls are not captured. Neither Paper nor NeoForge has a matching event; a Java mod or plugin can attach a Log4j appender.

`consume_effect` receives the custom kind, the canonical save-codec data for a registered kind (otherwise the retained definition data), the pre-shrink stack snapshot and the consuming entity (player or mob). Entries come from the effective consumable, including stack patches, and run in list order between the built-in entries. The callback is bound at decode; nested effects are not filtered by kind.

### What a mod's container item holds

`contained_fluid` is the client's read of NeoForge's `FluidUtil.getFirstStackContained` for a mod's own item: an
`ItemPart` attached to a container item (a tank, a canister, a modded bucket) says which registered fluid a stack
holds, and the item's `neoforge:fluid_container` model draws it as it draws a water bucket's: the fluid's still
sprite through the model's `fluid` mask, tinted as a stack holding that fluid, under the cover. Leaving
`event.fluid` at `None` draws the stack as holding none: the model's declared fluid untinted, or no fluid layer
when that is `minecraft:empty`.

The answer is a `FluidKey`, never a sprite or a colour: take it once in `register` (`r.fluid_key("minecraft:water")`,
or the key `declare_fluid` returned) and keep it in the part. The stack is an owned snapshot, so a part reads its
own component (an amount, the fluid it was filled with) from `event.stack`. Only items outside `minecraft:` ask:
a part attached to a vanilla item by key or tag is never asked, and vanilla's buckets keep their closed answer.

It is asked once per draw of a stack whose model is a fluid container, so at most once per stack per frame, and
only for an item whose parts subscribe: that check is a lookup by the item's key, and the stack snapshot is built
only after it. A registered fluid the item renderer has no look for (a tank-only fluid with no `sunrise/fluid`
data) draws the missing texture through the mask, so the mistake shows. A key no fluid is registered under draws
the stack as holding none and is logged once for that item. The hook is
client-only; the server keeps the fluid in the stack's own data, through `ITEM_FLUIDS` and the item's
components, as before.

## Container loot

- **`container_loot_filled(context, drops)`**: Sunrise addition. Each block or chest vehicle fill exposes its rolled list before shuffle, splitting and placement. Each double-chest half fires separately. Replace, remove or add stacks; empty stacks disappear, oversized counts are trimmed to the effective maximum, and excess stacks that cannot fit are discarded. Context names the table, dimension, block position or entity id and block centre or entity position. Only a player open that initiates the fill names an opener; other accesses, including an ItemPlace::EntitySlot read during a player action, give None. Filled containers and block or mob drops do not fire it. The read-only level Host permits reads and only the list is writable. No API snapshots are built without listeners, and the roll and placement draws are unchanged. Codec-valid damage above max_damage is kept. The `HookList` validates each edit that supplies a stack. A refused edit warns and leaves other edits intact.

### Per-stack crafting remainders
`crafting_remainder` is an item Part listen query on the authoritative server,
with a read-only `Host`. Its event starts with the item's data remainder; changing
it to `None` leaves nothing. Grid consumption (including repeated quick moves),
the crafter, furnace fuel, and brewing fuel/ingredients use the same stack-aware resolver. Book and banner
recipe copies remain recipe-owned. `Host::item_crafting_remainder` reads data only.
The resolver checks the prepared item subscription table before converting or
lending the input stack. Read its components with `get`, or use `project` to keep an owned copy. Invalid returned stacks are refused at `ItemBoundary`,
retaining the initial data remainder; native panics retain normal Part attribution.

## Skull models

`Hooks::create_skull_models` is the client registration-phase counterpart of NeoForge 26.3's
`EntityRenderersEvent.CreateSkullModels`. It receives a `RegistrationContext`, once at client
startup after `layer_definitions`. Call `event.register(model)` with a `SkullModel` per skull
type: its `kind` (`mymod:goblin`), the model layer the head is baked from, and the texture
used when the item model names none (vanilla's `SkullBlockRenderer.SKIN_BY_TYPE`).

The layer is one a mod registered in `layer_definitions` (`mymod:goblin_head#main`) or a vanilla
layer by its `entity-models.json` name (`minecraft:zombie_head#main`). It is baked once, at
registration. Its `head` part is posed as vanilla's skull heads are (no turn); a mod type has no
animation, as only vanilla's dragon and piglin heads animate.

This listener is compiled in the API tests:

```rust,ignore
{{#include ../../../../crates/sunrise-client/tests/it/mod_api_hooks.rs:create_skull_models}}
```

A registered kind is a name a `minecraft:head` item model can use, exactly as a vanilla one:

```json
{ "model": { "type": "minecraft:special", "base": "mymod:item/goblin_head",
  "model": { "type": "minecraft:head", "kind": "mymod:goblin" } } }
```

The head then draws wherever that item model draws: in hand, in a GUI, in an item frame and
dropped. Worn in a head slot it draws as other head items do, in the item's `head` display
transform; the skull's own placement on a head, like a placed skull, needs the item to be a mod
skull block. The model's optional `texture` replaces the default as it does for
vanilla's kinds. A kind nobody registered is refused when the item model loads, as an unknown
vanilla kind is.

Any valid canonical `<namespace>:<path>` identifier (including `minecraft:` for non-vanilla paths)
is accepted. An invalid identifier, one of the seven built-in vanilla skull types
(`minecraft:skeleton`, `minecraft:wither_skeleton`, `minecraft:player`, `minecraft:zombie`,
`minecraft:creeper`, `minecraft:piglin`, `minecraft:dragon`), or a kind registered already (by any
mod) is logged as an error and left out, as `registerSkullModel` refuses duplicate types. So is an
entry whose layer neither a mod nor vanilla has, or whose texture is empty. Skull types are frozen
once at startup. Placed skull blocks of a mod type are not drawn by this hook: a mod block
declaring that it is a skull is not covered yet.

## Custom spawners

`modify_custom_spawners` (NeoForge's `ModifyCustomSpawnersEvent`) fires once per server level when it opens, with the level's `CustomSpawners` list: vanilla's five built-ins (`WanderingTrader`, `Patrol`, `Cat`, `VillageSiege`, `Phantom`) in the order Sunrise ticks them for the Overworld, and an empty list for other dimensions (`MinecraftServer#createLevels`). The listener gets the level and an editor over that list: `entries`, `get` and `position` read every entry, built-in or a mod's; `remove` and `remove_at` take an entry out (a removed built-in never ticks, its random draws go with it); `insert` puts an owned `CustomSpawner` (a struct implementing `CustomSpawner` or a closure) at any place, `add` appends it, and `move_entry` reorders. The edits are staged: they apply after every listener ran, and a listener that panics has its edits undone, so another mod's instance it removed is still in the level. With no listener, or one that leaves the list alone, nothing changes: the same spawners tick in the same order.

Each registered `CustomSpawner` is owned by that level for its lifetime, ticked directly as its registering mod once per eligible server tick (outside debug worlds, with `spawn_mobs` enabled, matching `ServerChunkCache#tickChunks`) inside a server transaction, and dropped when the level unloads or the world closes. A panic rolls back that tick's writes, disables that spawner type for the registering mod for the rest of the session, and reports the mod and callback through the shared panic queue. Ordinary Host errors do not disable the spawner.

The callback's `spawn_enemies` is the chunk source's stored setting: `spawn_mobs && spawn_monsters` sampled at server startup, on difficulty updates, and when `spawn_monsters` is set (`MinecraftServer#updateMobSpawningFlags`). Changing only `spawn_mobs` changes whether the pass runs, not the stored setting. Peaceful is not a separate filter on that setting.

Sunrise ticks every entry in the custom spawner list in one ordered walk during the chunk-spawning phase (`ServerLevel#tickCustomSpawners`): the built-in spawners in list order (`WanderingTrader`, `Patrol`, `Cat`, `VillageSiege`, `Phantom`; vanilla's own built-in order differs, and moving it is a separate parity change), followed by any mod-appended `CustomSpawner` objects in list order so they see the effects of all preceding entries, including `Phantom`.

## When the notice fires, per side and cause

`ServerDataLoad { reload: false }` fires once the server's data tables are
built, on a world's first open, before its chunks and players load;
`ServerDataLoad { reload: true }` fires after `/reload`, once the players were
sent the new data. `ClientPacketReceived { integrated_server }` fires on a
client once per set of tags its server sends, at login and for each update in
play: the client's applied registries are what changed there, and the notice
marks them in place. In single player both causes fire, each on its own
thread; `cause.should_update_static_data()` is false only for an integrated
server's client packet, so the data the two threads share is updated once, on
the server.

Each notice fires right before the same-cause `tags_updated`, after the
published defaults and the tags are readable and before any entity, block or
player is ticked with them. The read helpers (`item_default_component`,
`max_stack_size`, `max_damage`) answer the published defaults from then on,
`modify_default_components`' edits included. `modify_default_components`
itself is not replayed by reloads: a reader that must follow a reload rebuilds
here, which is why the notice also fires again after `/reload`.

Unlike NeoForge's client, Sunrise's client does not rebind component defaults
on a tag packet: its defaults come from its own load. The client-side notice
therefore marks the applied tags' arrival rather than a rebinding; how often
it should fire, and its exact order against the client's component binding,
are the open questions this hook's review asks about.

## Effect particles

`effect_particle_modification` (NeoForge's `EffectParticleModificationEvent`) is asked once for each active
effect of a living entity, hidden ones included, and changes the swirl the server sends for it (`DATA_EFFECT_PARTICLES`):
the mobs and the players every client is sent. A listener replaces the effect's `particle`, sets it to `None` for the
original, or flips `visible`, which can show a hidden effect's swirl. A named particle goes only to Sunrise clients;
vanilla clients (whose brand isn't Sunrise's) get the list without it. `DATA_EFFECT_AMBIENCE_ID` is decided from the
result, as `areAllEffectsAmbient`: only effects visible after the mods count. As in vanilla it is asked when the
entity's effects change, not every tick, and the answers are sent until the next change; with no listener nothing is
kept and vanilla's list goes out. The window's own player draws its swirls from its own effects.

## Enchanted block loot

- **`enchanted_block_loot(level, dimension, pos, state, tool, enchantment, level, entity)`** (NeoForge's `EnchantedBlockLootEvent`): a block's loot table reads the level of `enchantment` on `tool`, at `ApplyBonusCount.run` or `BonusLevelTableCondition.test`. Change `level` and that one count or condition uses it: Fortune-style mods change it per block, breaker or dimension, without touching the stack. `level` starts after `get_enchantment_level`. `tool` is none for an empty hand or a toolless break, which is still asked at level 0. `entity` (`LootContextParams.THIS_ENTITY`) is the player mining, the breaking mob, the blast's direct source (primed TNT, TNT cart, exploding mob, fireball, wither skull, end crystal), the enderman dropping its carried block, the source of `/loot ... mine`, the projectile breaking a chorus flower or a decorated pot, the villager harvesting, the ravager in a crop, or none (a neighbour update, a piston, water, a trident through a speleothem, whose `destroyBlock` names nobody; also a boat through a lily pad, though vanilla names it). A `u32` level can't be negative, as `setEnchantmentLevel` refuses; above 1024 it counts as 1024, so a mod's value can't size a loop. A table reaches it once per read, in the roll's order, so a break can ask several times or never. Only block tables ask: chests, fishing, entity tables and `block_interact` tables do not. It runs in the roll with the read-only level host, so a listener can't move the table's random; the level random and the sequences are the roll's until it ends. The host can't read `entity`: a break can come from inside a level write, where the game is split up and there is no `Host`, so `getEntity`'s entity is a handle here, to keep or compare. Authoritative servers only. Tick-time effect: one `listened` test per roll with no listener; with one, each roll builds the level host.

## Entering a section

`Hooks::entity_entering_section` (NeoForge's `EntityEvent.EnteringSection`) tells a mod each time an entity's
position puts it in another 16x16x16 section, on the side that saw it move. It is the moment vanilla moves the
entity between its section lists (`Entity.setPosRaw`, then the level callback's `onMove`). Use it to keep a mod's own
per-section or per-chunk index of entities without scanning every entity each tick:

```rust,ignore
fn entity_entering_section(&self, host: HostSide<'_>, event: &mut EntityEnteringSectionEvent) {
    if !event.chunk_changed {
        return;
    }
    let [x, _, z] = event.new_section;
    if let HostSide::Server(_) = host {
        self.index.lock().unwrap().insert(event.entity, (x, z));
    }
}
```

`old_section` and `new_section` are section coordinates (each block coordinate shifted right by 4), and
`chunk_changed` says the x or z section differs. Each position update is one check against the section the entity
was in before it, so a teleport and the walk after it are two events, and a move out of a section and back is two.
One walk the collisions shape is one update, told once at where it ended. A mob that teleports twice within its own
tick and then walks is told three crossings.

An entity joining a level, changing level or a player respawning starts in its section untold, as in vanilla; one
that moves before its first tick is told. With no listener the game files nothing, so the hook costs a
subscription check per position update.

The hook runs at the position write: at once for a teleport (`/tp`, `teleport_to`, `random_teleport` from outside the tick,
an enderman's or a shulker's teleport when hurt), a piston or shulker push, a server player's sent move and, on a client,
a packet that places an entity. Mob writes are told in write order. A write told while the mob view is borrowed
gets a position-only Host: the crossing entity's position, `entity_residency` `Busy` for it and `Gone` for any other
entity, and nothing else of the world; a player move also answers that player's ground flag and velocity before the
move updates them, and `local_player` on the client. A write captured without a live observer is told after the
owning row is restored, before the next mob ticks, with the owning Host that reads resident entities. Crossings
queued while a listener runs are told after it returns, in filing order, with that Host; each keeps the placement of its
own move, which a read of the entity's position answers, and no later move or removal rewrites or deletes it. Listeners must not move or remove the crossing entity while its write in progress.

## Experience orb merge

- **`experience_orb_merge(dimension, into, from, into_value, into_count, from_value, from_count, canceled)`** follows a yes from `experience_orb_can_merge` and may cancel the merge: neither orb is absorbed, and a spawning orb (`from` 0, `from_count` 1) tries the next orb in reach, then spawns on its own. `into_value` is signed and reported as the orb holds it, so a negative orb is not clamped to the 0 points it pays.
- Left alone, `into` adds `from`'s count. A mod that narrows `into` via `host.entity(into)` + `host.experience_orb(entity)` and writes `into` with `ExperienceOrbRef::set_value` / `set_count` here makes that write the merged orb: `from` is taken in without adding its count, so `into_value = 11, into_count = 1` pays 11 once.
- The merge is refused, and `into` put back as it was (value, count and age), when the mod cancels, when `from` changed or left reach during the hook, or when the count would pass `i32::MAX` (`ExtraCodecs.POSITIVE_INT`). Neither orb is absorbed; a write the listener made to `from` itself is not undone.
- `ExperienceOrbRef::set_value` and `set_age` write full live `i32` values (saving truncates `Value` and `Age` to `(short)` as vanilla `ExperienceOrb.addAdditionalSaveData` does); `set_count` accepts `1..=i32::MAX` and returns `Err(ExperienceOrbWriteError::InvalidCount)` otherwise without mutating the orb.
- Version 2 adds `into_value`, `into_count` and `from_count`; rebuild listeners against this SDK.
- Version 3: a value written to `into` here is held live over the full signed `i32` range (a write of 40,000 stays 40,000 on the merged orb); only saving narrows `Value` to `(short)`, as `ExperienceOrb.addAdditionalSaveData` does. Version 2 refused a value outside a short during the merge.

## Block explosion resistance

- **`BlockPart::explosion_resistance(level, event)`** (`ExplosionResistanceEvent { pos, state, explosion }`): answer `Some(resistance)` to replace the block's own explosion resistance (NeoForge's `IBlockExtension.getExplosionResistance`) against the blast `event.explosion` describes, the same `ExplosionStart` the global `explosion_start` hears. A ray of the blast asks as it enters the block; its next steps in the same block reuse the answer, so keep it a pure function of the inputs. The answer replaces only the block's static value: vanilla combines it with the cell's fluid via `max` (`0.0` for empty fluid, `100.0` for water or lava), and the overrides vanilla applies after the block's value keep winning (a respawn anchor's water cell, a TNT minecart's rails and the blocks under them, which are never asked, and a dangerous wither skull's 0.8 cap on a block that is not `#wither_immune`). `None` leaves it to another part and then the block's value. Any finite answer is accepted; a non-finite answer is refused with a warning, and a panicking part is contained; the block's own value is used for both. Authoritative servers only; the level is read-only. With no part attached to the state, one table lookup. Diverges from upstream: new hook.

## Extend POI types

`extend_poi_types` runs once the block registry is resolved, before any world's point of
interest tables are built, and again when resources reload. Build a filter with
`StatePredicate::block(block, constraints)` against the resolved block definitions. The
`PoiTypeExtensions` registrar's `extend(poi_type, states)` adds the selected states to the
vanilla type named by `poi_type` (`minecraft:home`, `minecraft:armorer`, `minecraft:beehive`, ...).
A predicate with no constraints adds all of the block's states; constraints narrow it (`part=head`).
A panic or phase refusal discards that callback's pending additions before later mods run.

Added states are saved, located and found by villagers exactly as vanilla's own: the save's
`poi` records, `/locate poi`, and a villager's search for a bed, bell or job site see one table.
Bees are the exception: they find a hive by its block entity and the `minecraft:beehives` tag,
not by its POI type, so a mod's hive also needs that tag.
The engine refuses, with one logged warning naming both types, an unknown POI type and a state
vanilla or an earlier mod already gives to a different type; the other additions still apply,
and adding a state to the type it already has changes nothing. Tag predicates are refused.
There are no new POI types: a mod extends vanilla's, as NeoForge's event does.

The extension table is process-wide, like the loaded mod set: re-collecting replaces
classification for every simultaneously live resource set. Villagers cache only vanilla
classification and consult the current extension table, just as saves and locate do.

## Block screen effects

`extract_block_screen_effect` fires before block, water and fire overlays are drawn. Water and fire are first-person only. Water uses the player's eye fluid sampled through death tick 20 and kept after removal until respawn. `camera_submersion` can override that sample. The event carries the default water state at the player's eye block, including inside a waterlogged block. The event's `set_custom_renderer` is NeoForge's `setCustomRenderer`: the last eligible listener to set a renderer for an overlay wins. Cancellation skips listeners that did not opt in with `#[listener(receive_cancelled)]`, but it does not clear a renderer. A canceled overlay with no renderer draws nothing. For block and water, cancellation suppresses the vanilla quads even if a renderer was selected. For fire, cancellation uses a no-op only if no renderer was selected; otherwise a renderer that returns `false` still precedes vanilla fire.

After extraction, the game calls each selected renderer's `submit` in block, water, fire order. It submits just before its own overlay and can return `true` to suppress that overlay's vanilla quads. A failed submission contributes no quads and does not change extraction's vanilla drawing decision. A panic disables that mod's submission for that overlay kind for the session, not its extraction hook or its other overlay submissions. Panics name the mod in the log and client error toast. Earlier overlays, later overlays and the HUD's camera overlays are unaffected. `CameraDrawContext` takes camera-space quads under the first-person hand's projection, after the level and hand and before the GUI. The player and camera can be read on `ClientHost`; `partial_tick` is `getWorldPartialTick` at both stages, and `player_partial_tick` is `getPlayerPartialTick` at extraction. Nothing drawn outlives the frame, and no GPU handle is exposed.

## Finalize spawn

- **`finalize_spawn(dimension, kind, reason, pos, spawner, difficulty, spawn_cancelled, mob, group, cancel)`** (NeoForge's `FinalizeSpawnEvent`, version 3): fires before `Mob.finalizeSpawn`, and also for configured spawner NBT even though vanilla finalization will not run. There are two cancels: return `Flow::Cancel` to skip finalization (the mob still spawns, and only finalization’s rolls and setup are skipped), or set `spawn_cancelled` so no mob is added. `difficulty` (`Difficulty`) is settable and is what finalization reads. `spawner` is `Spawner::Block` for a monster spawner block, `Spawner::Trial` for a trial spawner block, `Spawner::Minecart` for a spawner minecart, or `None` otherwise (`FinalizeSpawnEvent.getSpawner`). A minecart handle reads the current cart state through the supplied read-only host. Outside spawners, fired only where vanilla 26.3 calls `finalizeSpawn`, with vanilla's reason at that call: natural spawning, chunk generation, spawners, trial spawners, patrols, raids and sieges, reinforcements, cat spawners, phantoms, jockeys, `/summon`, `/raid`, spawn eggs, dispensers, the nether portal's piglin (`Structure`), skeleton traps, evoker vexes, wandering traders and conversions. Not fired where vanilla creates the mob without it (breeding, an infested block's silverfish, a thrown egg's chick, a thrown pearl's endermite, hatching eggs, built golems, the oozing effect's slimes, a lightning trap's horse), for loads (`persist.rs` finalizes a placed entity only through its `sunrise:finalize` marker, which is a load) or for structure mobs placed by world generation. The mob being finalized is lent directly on the stack as `event.mob` (`UnspawnedMobEditor<'_>`): read it directly without `host` (`kind`, `position`, `equipment`, `attribute_base`, `attribute_value`, `tags`, `custom_name`) and edit it directly (`set_equipment`, `set_attribute_base`, `add_attribute_modifier`, `remove_attribute_modifier`, `add_tag`, `remove_tag`, `set_custom_name`). Each edit applies at once and reaches only that mob; the borrow cannot outlive the hook call, so there is no handle or `Expired` state. The editor is a scoped loan rather than a `&mut dyn ServerHost` because the hook fires inside spawning passes that hold the level shared (natural spawning, spawners), as `CriterionContext` is lent to a criterion test. Listeners run before finalization, so finalization's own equipment and attribute rolls overwrite a listener's edit to the same slot or attribute; `Flow::Cancel` keeps every edit. Mobs that finalization creates (jockeys, a zombie horse's rider) get a stack loan of their own. The ender dragon, kept apart from mobs, answers its `kind` and `position` and `Unsupported` for the rest. `group` (`&mut Option<SpawnGroup>`) is the pack's `SpawnGroupData`, converted at the boundary: the listener may read, change, set or clear it, and finalization reads what it leaves. Chunk generation passes group data to the next pack member; live spawning does not yet thread it through packs. The new types are plain data (`Difficulty`, `Spawner`, `SpawnGroup`, `AgeableGroup`, `TropicalVariant`, `#[non_exhaustive]`). Read-only host; listeners run on replayed ticks, so they are pure functions of their inputs and change only `event.mob` and `group`. Tick-time effect: one `listened` branch with no listeners; with one, a direct stack borrow with no copy or arena entry. Beyond NeoForge: none.

`window_resize` notifies client mods after a window resize or applying a changed GUI scale, fullscreen, or Unicode font setting. Its `width` and `height` are physical pixels, and `gui_width` and `gui_height` are scaled GUI pixels. It fires only when physical or scaled dimensions change.

### Structure template reads

`LevelRef::visit_structure_template(host, id, at, rotation, mirror, visit)` reads a template
without placing it. It searches the world's saved templates before data packs, lends each
transformed block as a `StructureTemplateBlockView<'_>` in vanilla placement order with its
transformed state and borrowed block-entity data (stopping early on `ControlFlow::Break(())`),
and returns the rotated size `[i32; 3]`. It uses the palette selected at `at`; processors and
entities are not included. Refusals return a typed `StructureTemplateError`. A builder can read
the template and place its blocks over time.

## Frame presentation

`render_frame_post` fires every frame once rendering returns, whatever became of
the frame. `flip_frame` fires after it, on the game's thread, and only when the
frame was presented. A frame presented on the game's thread (VSync, the default)
was already presented when `render_frame_post` fired: there the two differ only
on the frames that were skipped.

Nothing fires `flip_frame` for a frame that was skipped: the window is minimized
or has no drawable size, the swapchain gave no image (timeout, occluded, outdated
or lost), the device was lost, or the frame was a scaled screenshot that went to a
file. The closing save screen and the startup loading screens present frames of
their own, and those fire none of the frame hooks.

Where a frame goes decides what "presented" means when the hook runs:

- Presented on the game's thread: the present has returned.
- Presented on the submit thread: the game waits for that present before the hook
  runs. It waits only when a mod listens.
- Frame generation: the frame has been submitted and handed to the present thread,
  which shows it up to half a real frame later and can still fail to. The game does
  not wait, because that would hold back the next frame and undo frame generation.
  A made frame fires nothing of its own: the hook follows the real frames.
- Offscreen, with no swapchain (for example, agent control): the frame has
  been submitted to its offscreen target.

## Fluid flow

- **`fluid_flow(context, fluid, pos, flow)`**: edit the current (`FlowingFluid.getFlow`) of the fluid block at `pos` as it pushes an entity: the player, mobs, dropped items, experience orbs, armor stands and arrows all read it through the one shared helper. It runs for each fluid block the entity touches in a tick, before the vanilla scale for shallow fluid, so keep the listener short. The renderer's own flow vector (a fluid texture's rotation) is not changed. `level.fluid_flow(host, pos)` reads the edited current. It fires on both sides and on the client's replayed ticks, with the loop `context` of the side that ticks the entity, so a mod registers on both and keeps the listener a pure function of its inputs. With no listener the cost is one `listened` branch per fluid block. Beyond NeoForge (no counterpart); Diverges from upstream: new hook.

## Fluid tooltips

- **`native_fluid_tooltip(stack, lines)`** is `FluidTooltipEvent`: the tooltip of a fluid, with no item behind it. `lines` starts as the game's own (`sunrise_client::fluid_tooltip::lines`): the fluid's name (`fluid_type.<mod>.<name>`, vanilla's by its block), its amount as `sunrise.fluid.amount` ("%s mB") and, with F3+H, its id. Each line is a `Text` (translation-ready) with a colour as `rgba` packs it. Add, change, reorder or remove lines; an empty list draws nothing. The game asks it where it draws a fluid slot itself (the recipe viewer's wells under the cursor), before `render_tooltip_pre` lays the tooltip out. Nothing item-shaped fires for it: `item_tooltip` and `render_tooltip_gather_components` belong to stacks of items.

## Frame graph setup

`Hooks::frame_graph_setup` is the client counterpart of NeoForge 26.3's `FrameGraphSetupEvent`
and vanilla 26.3's `FrameGraphBuilder`. It fires once per world view at the start of level
rendering, before world passes run, and never fires on title or connect screens.

Mods record passes as data via `event.graph.add_pass(desc, &mut |ctx| { ... })`, reading and
writing:
- vanilla 26.3 `LevelTargetBundle` built-in targets (`FrameTarget::MainColor`,
  `FrameTarget::MainDepth`, `FrameTarget::EntityOutline`, and the OIT targets),
- imported mod-owned offscreen targets (`FrameTarget::Mod(RenderTargetRef)`, always kept and
  never culled), and
- transient per-frame targets allocated via `event.graph.create_transient(&desc)` (pooled across
  frames and released after their last reader pass; passes writing only to unread transient
  targets are culled unless `disable_culling()` is set).

Ordering anchors reuse the existing `RenderLevelStage` enum (`AfterSky`, `AfterOpaqueBlocks`,
`AfterOpaqueFeatures`, `AfterTranslucentFeatures`, `AfterTranslucentBlocks`,
`AfterTranslucentParticles`, `AfterWeather`, `OitTranslucent`, `AfterLevel`). Kept passes are
topologically sorted with deterministic registration-order tie-breaking; cycles are refused with
`FrameGraphError::Cycle` (logged once per distinct cycle, never panicking). Under an active
Iris/OptiFine shader pack, scheduled passes run at the pack's corresponding world boundaries
before `final` composition.

## Block friction

`BlockPart::friction` answers NeoForge's `IBlockExtension.getFriction` for a state at a position,
with an optional `EntityRef`; `level.entity_kind(entity)` reads its type key. It runs for ground travel, ground drag, boat ground friction and lead
angular friction, and for explicit `LevelRef::friction` queries. The read-only `LevelSide` host
can read the level or ask for another state's answer at the same position, for example a block
taking its material's friction. The queried state need not be what the level holds at that position.
The borrowed view reads block states, their properties, fluids and light, without loading chunks.
`block_state_id` parses a block name with properties as elsewhere.
It also reads weather, enabled features, tags and the side's resident block entities, but has no
container, shape or signal view (those answer `None`). A client's block entity is its own copy, not the
server's: answer from state both sides hold. Each listener gets the original `pos`, `state` and `entity`;
only `friction` carries from one listener to the next.

An active query on the same level, position, state and entity id (or no entity) cannot be repeated.
A recursive or over-deep query returns `None` (`Ok(None)` through `LevelRef`). Without a part,
or when the part leaves the answer alone or writes a non-finite value, the state's own
coefficient is used; any finite value is used as given. Physics still applies vanilla's friction modifier and drag at the sites that
used them before. Register the part on both sides and keep its answer pure so prediction and
replay agree.

## Conditional item model properties

A client mod supplies its own namespaced `minecraft:condition` property by implementing
`Hooks::item_model_property`. This hook is the registration: match your property name, answer
through `value`, and leave other names untouched. The resolver calls it when choosing a branch,
not when loading resources. It defaults to false without a provider and skips dispatch entirely
once no loaded mod overrides it. Vanilla properties do not enter the hook.

```rust,ignore
fn item_model_property(
    &self,
    _host: &dyn ClientHost,
    event: &mut ItemModelPropertyEvent<'_>,
) {
    if event.property == "example:charged" {
        event.value = event.stack.components().and_then(|patch| patch.as_nbt().get("example:charged"))
            .is_some_and(|value| matches!(value, mod_api::Tag::Byte(1)));
    }
}
```

`ClientHost`, `ItemModelPropertyEvent`, and `ItemStack` are exported by `mod_api`. The stack is an owned `ItemStack` snapshot,
including for nested bundle items: `item()`, `count()` and explicit `components()` (not merged
defaults). `bundle_selected` carries the runtime selection separately; -1 means none. Listeners run in mod order; later answers win.
Use `"property": "example:charged"` with both `on_true` and `on_false` in the item's
`minecraft:condition` node. This hook chooses the branch; it does not build a special renderer.
Register custom `minecraft:special` types with `register_special_model_renderer`. Their factories
decode and bake nodes at resource load, and the baked renderer reads a borrowed
`SpecialModelStack` and submits geometry when the node is drawn. An unknown special type is a
load error for the enclosing item definition.

## Combat hook ownership

The six combat hooks (`living_incoming_damage`, `damage_pre`, `damage_post`, `living_knockback`, `living_use_totem`, `living_shield_block`) are lent to each hit by the game that runs it, not installed once per process. A mod sees no difference: order, arguments and cancel rules are unchanged.

**Migration (`living_use_totem`, `living_shield_block`, now version 2):** both now take the hit's level host as their first argument, as `living_incoming_damage` does: its blocks, random and own attachments, with no writes beyond them. Add the parameter (`level: &mut dyn LevelRandomHost`); the event, its fields, its order and its cancel rules are unchanged. `living_shield_block` fires inside `hurtServer` before the armor and enchantment reductions and sees the hit's `damage` as it came in; `living_use_totem` fires after health reached 0 and before `die`. Tick-time effect: one extra pointer in the hit and the same `listened` early-outs. Diverges from upstream: the two hooks gain the level host that upstream's catalog rows did not have.

**Attacker and held stacks (`living_knockback`, the weapon of `living_incoming_damage`):** the attacker's id and the stacks it holds come from the hit that started the attack, not from process state, so a hit nested inside another (a thorns reply, a guest's attack during the host's) names its own attacker and nothing is left behind afterwards. No event field, order, random draw or cancel rule changes, so neither hook's version does. The game copies the held stacks only while a mod listens; without one the hit carries the attacker id and two empty slots, no allocation.

**Sounds and an entity's random (`living_incoming_damage`, `living_damage_pre`, `living_damage_post`, `living_shield_block`, `living_use_totem`, now one version up):** these five take the hit's host as `&mut dyn HitHost` (a `LevelRandomHost` that also plays level sounds and draws from an entity's own random), so a listener can play a hit sound with a pitch drawn from the attacker. `host.play_sound_at(sound, at, source, volume, pitch)` is `Level.playSound(null, x, y, z, ...)` and `host.play_sound_from(entity, sound, source, volume, pitch)` is `Level.playSound(null, entity, ...)`: every player in range hears it, the entity's own player too, and a cancelled hit still plays what was queued. The sound's seed is drawn at the call from the level's sound-seed stream, never from the level random. `entity.with_random(host, |rng| rng.next_float())` lends the entity's actual `RandomSource`, in listener order, and the stream keeps the draws. A hit lends the entity its source names as its causing entity where the game has it at hand (see the causers paragraph below for the sites); the attacking mob's own random is the one its AI draws from. Any other entity, a stale handle, and a client answer `None` (`false` for a sound). **Migration:** change the host parameter's type from `&mut dyn LevelRandomHost` to `&mut dyn HitHost`; `with_random`, attachments and the event are unchanged. `entity_invulnerability_check` is asked on both sides and as a query, so its host stays read-only and it gets none of this. Tick-time effect: one pointer in the hit; with no listener nothing is looked up, drawn or allocated.

**The victim and the causing entity (`living_incoming_damage` version 5, `living_damage_pre` and `living_damage_post` version 4):** `host.victim()` and `host.causing()` lend a read-only `LivingView` of the hit's living victim and of the causing entity the hit already lends, when it is the one the source names (`LivingIncomingDamageEvent.getEntity`, `DamageSource.getEntity`). Through the same host, the attachments of those two holders now read where they answered `MissingHolder` before (`has`, `get_existing`), and their writes answer `AttachmentError::ReadOnlyHolder` instead, writing nothing. **Migration:** a listener that relied on `MissingHolder` for the victim's or causer's holder handles the values or `ReadOnlyHolder`; nothing else changes. `living_shield_block` and `living_use_totem` lend no view and keep their version. Tick-time effect: a few references copied into the hit; with no listener nothing is looked up or built.

**Causers at every damage site (`living_incoming_damage` version 6, `living_damage_pre` and `living_damage_post` version 5, `living_shield_block` and `living_use_totem` version 4):** the hit lends its causing entity wherever the game has it at hand, not only at a melee hit: Thorns' wearer, the shooter or owner of a projectile, a firework rocket, evoker fangs, a wither skull, a thrown trident or a wind charge, an explosion's igniter or owner, and a pufferfish stinging a mob inside its own tick. A guest that is not in place is lent from its hub record, read-only, with its own attachment map. When the victim caused its own hit (a player's own TNT hurting that player), the source names the victim and `causing()` answers `None`: the listener reads it through `victim()`. So `host.causing()`, `entity.with_random(host, ...)` and `host.play_sound_from(entity, ...)` answer at those sites, and the causer's attachment holder is read-only there. **Migration:** a listener that relied on `None` from those calls at such a site handles the lent entity; nothing else changes. Tick-time effect: with no listener nothing is looked up; with one, a hit looks its causer up once.

Version 3 removes `entry` (`KnockbackEntry`) to fire one knockback moment matching NeoForge's `CommonHooks.onLivingKnockBack`, and lends `attacker_main`, `attacker_off`, and `kill_credit_main` lazily as `Option<ItemStackView<'_>>` so stack component projection failures never skip the event.

## Living motion values

- **`living_jump_power(context, entity, kind, power)`, `living_speed(context, entity, kind, speed)` and `living_flying_speed(context, entity, kind, speed)`**: edit a living entity's jump power (`LivingEntity.getJumpPower`), movement speed (`getSpeed`) and airborne acceleration (`getFlyingSpeed`) where its travel and jump read them. They cover the player and the mobs' ground, fluid and jump paths (the rabbit's hop included); a mob that ships its own flying travel (an allay, a guardian) still reads its own speed. They fire on both sides and on the client's replayed ticks, so a mod registers on both and keeps each listener a pure function of its inputs. `context` is the loop context of the side that ticks the entity (`context.side()`, registries, settings, mod list): the server's for a mob or a server player, the client's for its own prediction and replayed ticks. Contract version `2`: a mod written for `1` takes the context as a new first argument of the `Hooks` method. With no listener the cost is one `listened` branch per call. Beyond NeoForge (no counterpart); Diverges from upstream: new hooks. For per-block jump and speed factors see `block_jump_factor` and `block_speed_factor` below.

## Map decoration types

`Hooks::map_decoration_types` is the registration-phase counterpart of NeoForge 26.3's
`RegisterEvent` for `Registries.MAP_DECORATION_TYPE`. It receives a `RegistrationContext`, once at
startup on every side. Call `event.register(decoration)` with a `MapDecorationType` per type:
an id `<mod id>:<path>`, the GUI atlas sprite that draws it (`assetId`) and `show_on_item_frame`.

A registered type is a name a `minecraft:map_decorations` entry can use, exactly as a vanilla
one: an item's entry `{type: "mymod:treasure", x, z, rotation}` (what `/give` writes and
`MapItemSavedData.addTargetDecoration` adds) is drawn with the type's sprite on a held map, in an
item frame and on the cartography table's screen, and a type with `show_on_item_frame` false is
left out of the last two as vanilla's player arrows and markers are. The sprite is looked up in
the GUI atlas by name, so a pack's texture or a mod's sprite source's pixels
(`Hooks::register_sprite_sources` in the GUI atlas, `mymod:gui/treasure`) works; a sprite the atlas lacks is left
out of the drawing. A mod type never counts toward a map's decoration limit (`trackCount`).

An id in vanilla's namespace, not of that form, registered already, or with an empty sprite is
logged and left out. Types are frozen once at startup; a type a
server's map data names that a client did not register is not drawn. Map data a Sunrise server
sends to a client (`ClientboundMapItemData`) numbers vanilla's types only, so a mod type reaches a
client through the item's component, not through that packet.

### Menu buttons

`menu_button` handles a mod menu's button (`AbstractContainerMenu.clickMenuButton(Player, int)`) on the server, for
single player and LAN guests. It is routed to the active menu type's owner and exposes the player identity, button,
optional block position, a live `BlockEntityScope` (`subject`) and the mutable carried stack, so a widget outside the
slots can take or give what the player carries. Call `subject.with_view(...)` to read or edit the resident block entity
in place (each write is validated at the boundary; `with_view` returns `None` on a blockless menu or after the block
entity is removed or replaced), `subject.host()` to reach the server `Host`, and `subject.transaction(...)` when an
edit must span the subject and other world storage atomically. A panicking listener is contained (its carried-stack
edit is discarded and any open transaction rolls back) and disabled for `menu_button` for the rest of the session.
A panic in a different hook does not discard the cursor edit of a button callback that returns normally.

### Menu button IDs

`menu_button_valid` asks the owner of the active menu whether it accepts a button ID. The owner chooses its IDs;
there is no shared range. Only an accepted ID reaches `menu_button`.

## Shift-click in a mod container

`menu_quick_move(kind, subject, container_slots, slot, stack, handled)` (contract version 6) is a mod container's
`AbstractContainerMenu.quickMoveStack`, the method a Java menu overrides to say where a shift-clicked stack goes. It is
asked on the server before the game's own routing, for the mod block entity whose menu a player shift-clicked
with the left or right button, on a vanilla menu type (`minecraft:generic_9x1`...) or on the mod's own menu type.
`subject` (`BlockEntityScope<'_>`) lends sequential access to the live resident block entity (`subject.with_view(...)`,
which exposes `position()`, `item_slot(...)`, `set_item_slot(...)`, `nbt()`, `data_slots()` and returns `None` if the
block entity was removed or replaced during the call), the server `Host` (`subject.host()`), and atomic transactions
(`subject.transaction(...)`). `slot` is the menu's index: below `container_slots` a container slot (its own, linked,
or the crafting result), from there the player's inventory, its 27 main slots, then the hotbar. The live menu asks
`Slot.mayPickup` first, then passes a copy of what the slot holds as `stack`. Take what you move out of `stack` and
return `Flow::Cancel`: the actual clicked source keeps the remainder (`None` empties it), the game's routing is
skipped, and the player's client gets the whole menu again. For a linked slot or result this does not replace an
own storage slot. A result handler must consume its inputs: the remainder stays available for the inputs it leaves
until the grid changes. Left unhandled, the game's routing runs on the original clicked stack without asking pickup
again: the listener's journaled edits are rolled back first. A later Part sees the earlier healthy Parts' proposals.
Writes through `subject.with_view(...)` validate at the boundary and apply to the live resident block entity in place.
The journaled edits and clicked-slot settlement commit together, before publishing implicit `BlockEntity.setChanged`
notices and calling `menu_slot_take` where applicable. Deferred notices are delivered only to still-resident holders.
An explicit `BlockEntityRef::set_changed` call is immediate; a same-block write in that callback does not queue another
notice. A panicking Part loses its journaled edits and outputs and is disabled for the rest of the session. A later
Part can still handle the move; if none does after a quick-move Part panics, the menu is resynced instead of falling back.
A panic contained in a nested hook does not by itself reject the move. A refused move, including an invalid remainder,
a changed clicked stack, or a stale menu, rolls back its journaled writes and does not fall back.

Not asked for an empty slot, a slot the mod's screen hides, another click type, a spectator or a dead player, a menu
no longer valid, or a shift-click during a drag (it only ends the drag, as in vanilla). It is asked once per click, where vanilla asks `quickMoveStack` again while the same item keeps moving: move
all you want in one call. The clicking player's state is swapped in (`subject.host().players().acting_player()` names them), and the block entity stays resident in the level during the call. Only a server's players (single player's server
thread, LAN guests, a dedicated server) shift-click this way; a native mod's own menus (`native_open_menu`) get their
clicks already.

### Keeping a mod menu open

`menu_still_valid` is vanilla's `AbstractContainerMenu.stillValid(Player)`, the method a Java menu overrides to
say whether a player may keep it open. The server asks the scoped `MenuPart` of the open mod menu's type each
tick the menu is open and before processing a matching non-spectator container click. A surviving menu is
checked again after `menu_clicked` listeners if the default click will run. A click for a different window is
dropped; a matching spectator click resends the menu without asking the part. Vanilla menu types keep
their own check and never ask a part. A false answer closes the menu as walking away does: what the
cursor held goes back to the inventory and the client's window closes.

`event.player` is the player's UUID and `event.menu` is the menu type. `event.pos` is the backing block's
position, or `None` for a blockless menu even if its opener supplied a position for the client's screen.
An item opener can supply a hand independently of how the menu stores its slots. The part sees it as
`event.opening_item`; its `stack_matches` compares the hand's current item and components with its
contents at open, ignoring count. This compares values, not stack instance identity. A part can use the result
when setting `event.valid`; the game does not close the menu just because the answer is false.
`event.valid` starts true for blockless menus. For block menus it starts true only if the player's eyes
(`event.eye`) are within their block interaction range plus 4 of each backing block. A part may keep a larger
machine's menu open past that distance, or close a menu for any reason. A part that leaves `valid` alone, or
whose mod is switched off, keeps the game's answer. A menu whose block entity is gone closes without asking;
respawning, changing dimension or leaving also closes the menu.

The hook runs in the loop context (no level or game Host) on every open menu of the type each tick, so answer
from the event and the context alone, and fast. Whether the type has a part that answers is looked up once,
when the menu opens: a menu with none does no work for this hook.

### Recipe transfer into a mod menu

A viewer's "+" (`RecipesClientHost::transfer_recipe_id`, or `transfer_recipe` for a recipe-book recipe)
names a recipe and the open window. The game's own transfer fills crafting grids only: a machine menu (an
inscriber, an alloy smelter, a pattern encoder) has none. The server first asks the `MenuPart` of the open
menu's type: `menu_transfer` gets the player, the menu type and block, the recipe id as sent (look it up with
the recipe reads; an id the server doesn't know is the part's to refuse), `max` for a Shift-click, and
`dry_run` for the viewer's check, which must move nothing. The part moves the ingredients itself with the
level host it is lent (the player's inventory through `ENTITY_ITEMS`, the block entity's container through its
capability) and returns a `RecipeTransferResult`; the viewer shows it as it shows the game's. A part that
returns None hands the transfer on to the game's grid transfer, which answers `NotIntoMenu` for a
menu without a crafting grid. Vanilla menu types are never asked. The contract is the same in a solo world and
for a LAN host's own player: the game is the server there, and asks the part on the same server-side path
(the player's own block container menu, its UUID, `dry_run` for the viewer's check) before its grid transfer.

Version 2 adds the dry run's cells. `missing_cells` names the recipe's cells no item fills and
`craftable_cells` the ones the menu lacks but could craft (a storage network's autocrafting), both as indices in
`RecipeTransferCheck`'s cell order: width by height for a shaped recipe, else its ingredients in order. They reach
the viewer as `RecipeTransferCheck::missing_cells` and `craftable_cells`, with any answer, so a part can answer
`Moves` and still mark the craftable cells. The game keeps both lists below the recipe's cell count, from its
known layout (one for a stonecutting recipe, three for smithing, the placement's slots for crafting and cooking;
256 for a recipe it can't inspect, such as a mod's own type), each once, in ascending order; pushes outside a dry
run are dropped.

## Minecart pass

- **`BlockPart::minecart_pass(host, event)`** (`MinecartPassEvent { state, pos, entity }`): a minecart passed over a rail block state the Part is attached to, at the moment vanilla calls `activateMinecart` for an activator rail, here for every rail. It is the Part form of Forge's `IBaseRailExtension.onMinecartPass`, which NeoForge 26.3 no longer has, so there is no global hook. With the old minecart physics it fires once per tick the cart is on a rail; with `minecart_improvements` once per track iteration, so a fast cart can pass several times in a tick. It is recorded during the cart's step and delivered after the cart's batch has ticked, through the same drain as `entity_inside`, with a `&mut dyn ServerHost`, so a write the part makes (a velocity change through `EntityRef`) lands on the stored cart. A pass whose cart is gone or whose rail changed is dropped. Only a rail state a Part listens for is recorded: with no such Part, one `block_listens` test per rail step. Beyond NeoForge; Diverges from upstream: new hook.

## Mob despawn

- **`mob_despawn(dimension, mob, kind, pos, result)`** (NeoForge's `MobDespawnEvent`, server): fired once per tick for every mob on the entity tick list just before `Mob.checkDespawn`, including persistent mobs and mobs in peaceful. A mob is on the list when its chunk is entity-ticking, which player tickets give across the whole view distance; listed mobs beyond the simulation distance are checked without being ticked. `result` starts as `Despawn::Default`. `Allow` discards the mob now, `Deny` keeps it this tick (a mod can keep a hostile mob in peaceful, as NeoForge's event documents), and `Default` leaves vanilla's rules to decide. An answer other than `Default` skips vanilla's checks, so it also skips the `nextInt(800)` draw of the idle-despawn roll: the hook fires before that draw. A mob in a loaded chunk that is not entity-ticking is not checked and the hook does not fire for it; the wither and the ender dragon override `checkDespawn` without `super`, so it does not fire for them either. In vanilla the whole of `Mob.checkDespawn` follows the event, the peaceful-mode discard included, so a `Deny` keeps a hostile mob in peaceful.
- Version 2 makes the ticking mob and snapshot peers readable through the scoped tick host. The ticking mob is read directly; an already-ticked peer exposes post-turn state and a not-yet-ticked peer pre-turn state. A mob spawned during the pass does not resolve in this view until the next pass. Removed mobs do not resolve, unlike dead-but-present mobs.
- Version 3 narrows the hook to the entity tick list: mobs in loaded chunks that are not entity-ticking no longer fire it or check despawning, as in vanilla.
- Listeners run on replayed ticks, so they are pure functions of their inputs. Tick-time effect: one `listened` branch per mob per tick with no listeners; the answer sits in a `Mob` field that costs one byte and is reset after the mob's tick.
- Beyond NeoForge: none. `Despawn` is plain data, `#[non_exhaustive]`.

## Spawn checks

- **`mob_spawn_placement_check(dimension, kind, reason, pos, default_result, level, random, result)`** (NeoForge's `MobSpawnEvent.SpawnPlacementCheck`): at `SpawnPlacements.checkSpawnRules`, `default_result` is the effective predicate result, including predicates composed or replaced through `Registrar::register_spawn_placement`. Set `result` to `SpawnCheck::Succeed` or `Fail` to override it, or leave `Default` to keep it. Each listener borrows the same live `random` stream the predicates already drew from. Reassigning the reference is local to that listener; writes through `result` carry to the next listener. Neither the predicate draws nor the listener draws are undone if the result is overridden or the spawn is refused later. The hook runs for natural spawning, chunk-generation packs, reinforcements, and the type check at spawners and trial spawners, including non-mobs. Patrols and village sieges call vanilla predicates directly, so neither registered predicates nor this hook run there. An ordinary spawner with custom rules skips it; trial spawners check their custom rules afterward. Like NeoForge's `SpawnPlacementCheck`, it has no spawner field.
- **`mob_spawn_position_check(dimension, kind, reason, pos, level, spawner, mob, result)`** (NeoForge's `MobSpawnEvent.PositionCheck`, version 3): `Default` runs `Mob.checkSpawnRules` and `checkSpawnObstruction` as vanilla; `Succeed` and `Fail` skip them and their draws. This fires for natural mobs, chunk-generation packs and `BaseSpawner` ticks (monster spawner blocks and spawner minecarts), including spawners with configured entity data. Trial spawners run their obstruction check directly. `spawner` (`Option<Spawner>`) is `Spawner::Block { pos }` for a monster spawner block, `Spawner::Minecart { entity }` for a spawner minecart, and `None` otherwise (`MobSpawnEvent.PositionCheck.getSpawner`). `mob` (`UnspawnedMobView<'_>`) borrows the actual new or loaded candidate. Read it directly (`kind`, `position`, `equipment`, `attribute_base`, `attribute_value`, `tags`, `custom_name`); there is no editor or `Expired` handle. A refusal drops the candidate; on success, finalization and admission use the same mob. Creating it does not draw from the level's random.
- Both get the read-only host and `level` (`LevelRef`, the level the entity would join). Both run on replayed ticks, so listeners should be pure functions of their inputs. One `SpawnCheck` serves both (NeoForge's two enums have the same constants).

## Mob split

- **`mob_split(dimension, parent, kind, size, child_size, children, cancel)`** (NeoForge's `MobSplitEvent`, version 4): fires when a slime, magma cube or sulfur cube above size 1 dies and splits, after vanilla has constructed its children (`AbstractCubeMob.remove`: the count, then each half-size child and the parent's yaw draw for it; a primed sulfur cube above size 1 dispatches with an empty list). `children` (`MobSplitChildren<'_>`) lends an unbounded mutable child list directly on the stack, matching NeoForge's `MobSplitEvent.getChildren()`.
- The list editor offers `len`, `is_empty`, `get` (read-only `UnspawnedMobView<'_>`), `get_mut` (mutable `PendingChildEditor<'_>` with `set_position`, `set_yaw`, `set_baby`, `set_size` and all unspawned-mob edits), `construct` (taking a `PendingMobSpec` and returning an owned `UnspawnedMob`), `push`, `insert`, `replace`, `remove` and `clear`. Any detached `UnspawnedMob` not left in the list is cleanly discarded on normal Rust drop with no arena or sweep. There is no 64-child cap, no lease token and no `Expired`/`Ownership`/`Capacity`/`Budget` errors; an out-of-bounds index on `insert`/`replace`, an unknown kind, a wrong dimension, a non-finite position or yaw, or an invalid size answers `Invalid` without mutating the list.
- A panicking listener is disabled for this hook by the shared dispatcher. Its cancellation flag is restored, but direct writes to children and their list are not journaled and remain visible to later listeners.
- A detached child constructed by a listener fires `entity_constructing` immediately, even if it is never added to the list. Every original proposal fires `entity_constructing` before `mob_split`, including proposals later removed or canceled. Canceling (`Flow::Cancel`) or removing every child spawns none. Construction draws and unrelated Host writes stay, and the parent still dies. If accepted, the remaining children join once each in list order. Removal effects read the live parent after the split and child admissions, so removing an effect during either callback prevents its removal action.
- Not fired for a parent of size 1. With no listener the split runs exactly as vanilla and allocates nothing for the list.

Version 2 follows ContainerOpenersCounter: synchronous open/close, five-tick rechecks, and game events on zero crossings instead of a per-tick recount.

## Mod mismatch

A world records the code mods it was saved with at the root of `level.dat`, beside `Data`, under
`fml.LoadingModList`, the list NeoForge keeps: one `{ModId, ModVersion}`
compound per loaded mod, in load order, rewritten on every save of the world.
Its content mods go in the same form under `sunrise.ContentModList`, beside the
namespaces of the mod block states registered when it was saved
(`sunrise.BlockNamespaces`). The version is the mod's `sunrise-mod.toml` `version`,
or `""` when the game doesn't know it. Saves record only nonempty lists, and an empty
saved list counts as absent. A world without either list (vanilla, saved with no mods, or
saved before Sunrise wrote one) is no mismatch: it fires nothing and gets the lists on its
next save when mods are loaded.

`Hooks::mod_mismatch` runs whenever a nonempty saved list exists, changed or unchanged,
once per open. A saved mod that is no longer loaded (or a saved block namespace nothing
registers now) would lose its blocks, and what they hold, for good, so the world asks
before it opens: a blocking question on the client (Don't Open by default, Back Up and
Open, Open Without a Backup), and a refusal on a dedicated server unless
`--allowMissingMods` or `server.toml`'s `[missing_mods] allow = true` says to open it.
Then the hook fires before the world opens, on the side that asks (`HostSide::Client` in
singleplayer, the dedicated server's `HostSide::Server`), with no world loaded yet, and
each id a listener resolves is taken off the question. Otherwise it fires on the opening
server (the integrated one in singleplayer) before world-state restoration; preparation
has already upgraded the world at this boundary.
Versions compare using the loader's semantic version parser (omitted minor and patch
are zero); empty or invalid versions compare by exact text. `minecraft` is excluded.
Call `event.mark_resolved("your_mod")` to mark a migration, and inspect prior resolutions
with `event.was_resolved(id)`, `event.resolver(id)` or `event.resolved()`.
The first resolving listener is recorded in each marker's `resolver`, and earlier
resolutions are preserved against deletion or replacement. Entries are logged sorted by id:
resolved at debug (self or other resolver), unresolved at warn. Only a missing mod
holds the open back; a changed version never does.

## Model baking completed

`model_baking_completed` (NeoForge's `ModelEvent.BakingCompleted`) fires on the render thread once per successful model
load at the model manager's apply step, right after both atlases' `texture_atlas_stitched`. Listeners ordered after
the model manager apply after it. The store `render_assets().model(&id)` reads is already the new one, the standalone
models included. Resolve the models a mod draws there, once, and keep the `ModelRef`s until
the next `model_baking_completed`, when the old ones answer `ModelError::StaleModel`; a reload that fails fires nothing
and leaves them valid. Use `host.render_assets().baked_model(key)` during the callback to borrow the published geometry
without editing it. A dedicated server never fires it.

### After a block item places its block

`ItemPart::on_block_placed_by` is the server-side `BlockItemExtension.onBlockPlacedBy`
notice (hook version 3). It runs on the original item's Part, after the selected
block's final state, stack block-entity data and `Block.setPlacedBy` work exist,
including a bed's head, at the primary position
(the bed's foot). The event carries the pre-shrink `stack`, optional `placer`, placement `level`, `pos`,
live `state` after the callback, clicked `face` and world-space `click`. A dispenser has no placer.
Use the `ServerHost` and block-entity handles to copy stack data or link companions;
writes are not overwritten by the placement's stack-data application. The notice
precedes the item's placement sound, game event, trigger and consumption, and
cannot cancel the placement. The block callback can run its own effects first,
including clearing a completed wither pattern. A refusal, placement failure,
`entity_place` cancellation or primary write that leaves a different block before
`setPlacedBy` never fires it. Items without a
subscribed Part cost one `item_listens` lookup and no stack conversion.

## Block state changes

- **`BlockPart::on_block_state_change(level, event)`** (`OnBlockStateChangeEvent { dimension, pos, state, old, new, predicted }`): a write through `Level.setBlock` held and the block at `event.pos` went from `event.old` to `event.new` (NeoForge's `IBlockExtension.onBlockStateChange`). It is heard where vanilla runs `updatePOIOnBlockStateChange`, at the end of the write: after the old block entity is removed, `on_place`, the client update, the neighbour updates and the neighbours' shape updates, so a fence beside a new stone block already shows its connection. The parts of `old`'s block hear it first, then, if the block changed, the parts of `new`'s block; `event.state` is the state whose block's parts are asked. It is a notice: the level host is read-only (`LevelSide`, which says whether this is a server or a client). Separate from the global `block_changed`, which a server hears right after the state is written, before any of the write's side effects, and also for a write whose `on_place` replaced the block; this hook skips such a write and hears the write that replaced it. Heard on both sides: every write a server makes, a client's replay of a piston's move, and on a client each block the server sends (`BlockUpdate`, `SectionBlocksUpdate`). Applied local predictions, including mining breaks and placements, are heard immediately with `predicted: true`. Server writes, settled corrections and every server-side write carry `predicted: false`. A server update for a predicted position remains unheard until acknowledgement. A confirmed prediction is not heard again; a denial is heard once from the predicted state to the server's state. Equal-state writes, unapplied guesses made while deciding an action and world generation's writes are not heard. A panicking part is contained with a warning. With no listener, one flag check builds nothing. Diverges from upstream: new hook.

### When a block entity's chunk unloads

`on_chunk_unloaded` is NeoForge's `IBlockEntityExtension.onChunkUnloaded`: a `BlockEntityPart` attached to a mod
block entity type is told once for each block entity of that type in a chunk that unloads, so a machine with a
running job, an open handle or a place in a network it keeps can let go of it. `event.pos` is where it stands,
`event.block_entity` its handle, equal to the one `host.block_entity` gave while it was loaded (so a map keyed by
it finds the entry), and `event.entity` a borrow of its type id and data as they are at the call: on a server, what the chunk
was just saved with; on a client, its copy as the server last sent it. Read it directly during the
callback; unloading rows need not resolve again by handle.

It fires on each side that holds a copy of the block entity. On a server it fires for each chunk that leaves a
level, a level that closes for being empty included, after the chunk was saved and before the block entity is
dropped. A chunk the server streams out has already left the level by then, so the handle's reads and writes
find nothing there (`None`, `Stale`): read what you need from the event. Changes made here, through the handle or
otherwise, aren't saved, as for `chunk_unload`. On a client (the window's own in single player included)
it fires when the server forgets a chunk or the view moves away from it, while the chunk is still there.

It fires once per block entity per unload and never for a vanilla block entity. It does not fire when the world
closes (those moments have `level_unload`, `server_stopping`, and `server_stopped`), nor on a client when its whole level goes (a respawn, a
dimension change, leaving the server). The chunk-wide notice stays `chunk_unload`, a server hook: on a server
it is told after the chunk's block entities, and a client is told of them alone. A chunk unload with no part
subscribed costs one check.

## Blocks destroyed by pistons

- **`BlockPart::on_destroyed_by_push_reaction(level, event)`** (`OnDestroyedByPushReactionEvent { dimension, pos, state, direction, fluid }`): a piston's move destroys the block (a `Destroy` push reaction such as a torch's), heard in `PistonBaseBlock.moveBlocks` after the block dropped its resources and while it is still in its cell, before the engine sets air there (flags 18). `direction` is the way the blocks move: the piston's facing as it extends, the opposite as a sticky piston pulls. It is a notice (NeoForge's `IBlockExtension.onDestroyedByPushReaction`): the removal still happens. The level host writes and schedules ticks (`&mut dyn LevelWriteHost`), but a write to any cell of the move is lost: its own cell and the other destroyed cells turn to air, and the pushed blocks were read before the destroy loop, so each of them moves as it was and the cells they leave and enter end as air or a moving block, as does the arm. Writes outside the move stay. `piston_pre` is the veto for the whole move; a cancelled move and a client's replay of a move are not heard. A panicking part is contained with a warning and the move goes on. `fluid` is the fluid state the cell holds, as NeoForge passes it; the removal itself always happens, and a mod cannot replace it here. Authoritative servers only. With no part attached to the state, one table lookup per destroyed block. Diverges from upstream: new hook.

## Block Parts in worldgen

- **Which Part methods.** A `BlockPart` method declared with the worldgen context runs while a feature generates:
  today `on_tree_grow` and `rotate`. The game asks them on the worldgen worker threads, and on the server thread when a sapling
  grows into a tree, with the same tables in both places.
- **What it reads.** `context` is a `WorldgenHost`, the server's registries, settings and mod list as they were at
  its last resource load; there is no `Host` and nothing to write. The parts asked are those the server resolved
  against its own tags when its world opened or at its last `/reload`, attached by key or by tag alike; a client
  window's tables never reach the workers. A feature that started before a `/reload` finishes with the tables it
  started with.
- **What it may keep.** Nothing mutable: neighbouring chunks generate at the same time on other threads, so a part
  is a pure function of its inputs. Draw only from the feature's own random in the event: the part's draws fall
  between the feature's, in call order, and the same seed gives the same world.
- **Answers.** A state the game's registry does not have is refused: the game logs the mods whose parts answered and
  keeps the feature's own answer. A refusal never panics.
- **Cost.** With no part subscribed, a call costs one subscription check. A lookup reads this thread's handle on
  the published tables and takes no lock.

## Player bed leave callers

The notice precedes clearing the sleeping position and pose. The pairs below
are `(wake_immediately, update_level)`; an immediate wake resets the sleep
counter to 0, otherwise it becomes 100 for the fade-out.

| Existing server caller | Pair |
| --- | --- |
| Morning, acting player or stored guest | `(false, false)` |
| Leave-bed packet, button, close chat, or Sunrise's leave/sneak path | `(false, true)` |
| Player bed-rule refusal | `(false, true)` |
| Missing bed / living-entity continuation failure | `(true, true)` |
| Death or missing acting body | `(true, true)` |
| Mod `LivingRef::stop_sleeping`, acting player or guest | `(true, true)` |

A nested wake of the same player does not emit a second notice. Plain position
teleports and transition-based dimension changes do not acquire a new blanket
wake. A distinct level-and-position wake, if added, uses `(true, true)`; a
disconnect wake, if added, uses `(true, false)`. Direct metadata clearing is not
stop-sleeping, and client metadata resets do not emit this server hook.

## Player destroyed block

- **`player_destroyed_block(host, event)`**: a player's break has removed the block. `event.pos` and `event.state` are where it stood and what the removal took out (read just before it, so after `playerWillDestroy` and `break_block`: a cracked decorated pot, a disarmed tripwire where vanilla passes `Block.destroy` the armed one, or a block a `break_block` listener put there), `event.block_entity` is the saved data of that block's block entity (`BlockEntity.saveWithoutMetadata`) or none, and the host's helpers name the acting player. It fires at the `Block.destroy` call that vanilla's `ServerPlayerGameMode.destroyBlock` makes only once `Level.removeBlock` succeeded: after the removal and before a creative player's return and `playerDestroy`'s item drops, statistics and experience. A break `break_block` cancels, or one that removes nothing, is not heard; neither is a level-side break (a piston, water, an explosion): those never go through `destroyBlock`. The block entity is read before the removal only while a mod listens, because the entity is gone afterwards and a `BlockEntityRef` would be stale. On the breaking client it also fires, with a `HostSide::Client`, once the local player's predicted removal succeeded (`MultiPlayerGameMode.destroyBlock`'s `Block.destroy` call): only for the local player's own break, never for another player's or a refused prediction, with `event.state` the block read before `playerWillDestroy` (vanilla's `oldState`, not the server's adjusted state) and `event.block_entity` always none. Only local effects are meant there; match `HostSide::Server` to stay server-only. There is no NeoForge event for this moment: the name and contract are this fork's.

## Player move

- **`player_move(host, event)`**: Bukkit's `PlayerMoveEvent` for a guest's own movement, fired from `ServerGamePacketListenerImpl.handlePlayerPositionChange`'s place: after the server validated the report, before it is committed. Cancelling vetoes it; the client is sent back to the accepted position and rotation. The engine applies no threshold: a mod that wants Paper's 1/256 square block or ten degrees compares `from` and `to` itself.

### Passages within a level

`crates/sunrise-client/src/game/modding/native_passages.rs` and `native_passage_cross.rs`, with the hub's half in
`crates/sunrise-client/src/lan/hub/passage_cross.rs` (#3775, docs/modding/src/how-to/passages.md).

- `cross_passage(passage)` (client) moves the window's player through a passage its server told it of (feet, last
  tick's place, velocity, look, turned) and sends `sunrise:passage_cross` (the number, the feet before). Refused with
  nothing moved or sent as a `PassageCrossError`; a payload a mod's `client_packet_send` cancels is `Cancelled`. `last_passage_refusal()` (client) is the server's
  `PassageCrossRefusal` of the last crossing, until the next.
- On the server the move to where it crossed from is checked as any move; the crossing then holds the guest's
  other packets until the game has decided it from its own `from`. The game checks eligibility, the passage, the
  20-block reach and the opening's and destination's chunks, fires `player_passage_cross`, checks all of it again,
  and only then works out where the player comes out (that chunk must be held too). Accepted with an all-relative,
  zero position packet; refused by name (`sunrise:passage_sync`) and put back with an absolute one.


- **`player_passage_cross(player, passage, from, cancel)`**: `player`'s client crossed `passage` from `from` (its feet before), the move there passed and both ends are tracked; the server has not worked out where the player comes out yet. Set `cancel` and the crossing is refused (`PassageCrossRefusal::Vetoed`), the player put back at `from`. Everything checked before is checked again after the listeners, so one that removes the passage or moves the player refuses it too. A passage into another level (`destination_level`) never reaches the hook: the server refuses it `PassageCrossRefusal::FarLevel` first.

## Player start spectating entity

- **`player_start_spectating_entity(host, event)`**: a spectator's camera moved to `event.entity_id`, from a click (`handleSpectatorAction`) or `/spectate`. It is heard after vanilla's `ServerPlayer.setCamera` moved the view to the target and sent `ClientboundSetCameraPacket`, once per new target: switching straight from one entity to another is one start for the new one, and `player_stop_spectating_entity` is heard only when the camera returns to the player. Setting the camera to the target it already has, to the player's own entity, or to an entity that is gone (no camera is set then) is not heard.

## Player edge backoff

- **`player_stays_on_ground_surface(context, entity, staying)`**: edit whether the player counts as staying on the ground surface (`Player.isStayingOnGroundSurface`, vanilla: the shift key is down) where `maybeBackOffFromEdge` asks, so a mod can make the player hold to an edge without crouching, or walk off one while crouching. It fires on both sides and on the client's replayed ticks, with the loop `context` of the side that ticks the player (see [The loop context](#the-loop-context)), so a mod registers on both and keeps the listener a pure function of its inputs. With no listener the cost is one `listened` branch per movement. Beyond NeoForge (no counterpart); Diverges from upstream: new hook. Mobs never back off an edge, so there is no mob call site.

### Local player relative turn (`player_turn`)

- `player_turn(player_entity_id, player_uuid, x, y)` runs on the client immediately before the
  local player's `Entity.turn(xo, yo)` applies a relative look delta, whether it came from the
  mouse (after `calculate_player_turn` and sensitivity/cinematic smoothing), a controller stick,
  the keyboard turn keys, or `sunrise-ctl turn`. Freecam and direct `ClientHost::set_local_rotation`
  skip it.
- `x` (yaw) and `y` (pitch) are in vanilla `Entity.turn` units (`(delta as f32) * 0.15` degrees),
  matching `Entity.turn(double xo, double yo)`. Listeners run in hook order and chain changes;
  if an axis's `(delta as f32) * 0.15` is not finite after dispatch, Sunrise logs a warning and
  reverts that axis to its incoming value.
- Use `calculate_player_turn` when you specifically need mouse sensitivity or cinematic camera
  smoothing (`MouseHandler.turnPlayer`); use `player_turn` when a turn scaling or adjustment
  should apply across mouse, controller and keyboard look alike (for example, slowing look speed
  while zoomed).

- `pre_remove_side_effects` is the server's `BlockEntity.preRemoveSideEffects`: the block is being
  replaced or broken with side effects (not under flag 256). Read the outgoing row via `event.entity`
  and its attachments with `key.get_in(host, Holder::BlockEntity(pos), read)`. Submit effects with `event.effects`.
  Base contents scatter first unless suppressed; contributed slots, positioned items with default pickup delay,
  and experience follow in that order. Order within each group is preserved. `drop_slot(None)` draws an empty
  slot's scatter position without spawning. An invalid stack or nonfinite position refuses only that call.
  A Part that contributes nothing does not cause removal effects; contributions from a panicking Part are discarded.
  A ticking or menu-hook block entity can be removed through the Host; don't drop its contents separately in that callback.

## Preparing render buffers

`Hooks::prepare_render_buffers` is the client counterpart of NeoForge 26.3's
`PrepareRenderBuffersEvent`. It fires once per world view before any pass-bearing world render
stages execute, whether or not the frame has an `AfterSky` stage. When a classic shader pack
owns the sky or celestial work, `AfterSky` follows preparation and the engine world import. Preparation never
fires on title or connect screens.

Use `event.context.lines(stage, lines)` or `.quads(stage, quads)` to prepare a batch for a
pass-bearing stage. Keep the
returned `PreparedWorldDraw` until `render_level_stage`, then give it to the active pass's `draw`.
A handle is valid only for its own view and stage. Nothing is uploaded from a stage callback.

Changes made here to `InstanceGroup` records, active length, or group transform apply to groups
already submitted for this frame. Its `WorldRenderView`
lends the frame's `Camera`, world partial tick, game time, whether vanilla's improved transparency
(OIT) is active this frame, and the primary camera's canonical visible `RenderableSection` slice.

## Block-state definition types

A blockstate file (`assets/<namespace>/blockstates/<block>.json`) picks each state's model with vanilla's
`variants` or `multipart`. A mod registers a definition type of its own (NeoForge's
`RegisterBlockStateModels`) once at client startup via `Hooks::register_block_state_models`. Call
`event.register(id, provider)` to bind the type identifier directly to its provider callback; when a
blockstate file names it in `neoforge:definition_type`, the game invokes the bound provider on every load
and reload without scanning other mods.

```rust,ignore
fn register_block_state_models(
    &self,
    _context: &RegistrationContext,
    event: &mut RegisterBlockStateModelsEvent<'_>,
) {
    event.register(Identifier::parse("example:by_power").unwrap(), |input| {
        // {"neoforge:definition_type": "example:by_power", "on": "example:block/lamp_on", "off": "example:block/lamp_off"}
        let (Some(Tag::String(on)), Some(Tag::String(off))) =
            (input.config.get("on"), input.config.get("off"))
        else {
            return Err("needs \"on\" and \"off\" models".into());
        };
        Ok(input
            .states
            .iter()
            .enumerate()
            .map(|(state, properties)| {
                let lit = properties.get("powered").is_some_and(|value| value == "true");
                BlockStateModelPick::Model {
                    state: state as u32,
                    model: (if lit { on } else { off }).to_string_lossy(),
                    turns: [0; 3],
                    uvlock: false,
                }
            })
            .collect())
    });
}
```

`BlockStateDefinitionInput` hands the provider the block's identifier (`input.block`), the file's whole JSON
object converted to a `Tag` (`input.config`), and the block's states in registry order (`input.states`),
each as its property names and values. A pick names a state by its index there. Any valid
`<namespace>:<path>` identifier (including `minecraft:`) is accepted; a later registration for the same ID
replaces the earlier one (last-writer-wins) with a warning naming both mods.

The file is refused whole, with its name and a reason logged, when the type is not registered, the provider
returns `Err(reason)` or pushes `BlockStateModelPick::Refuse`, a pick names a state the block lacks, names
one state twice or a model id that doesn't read, or no state is picked. Returning `Err` reports the refusal
each time without disabling the provider; panicking disables that provider for the session and surfaces an
error toast. A file several packs provide binds each pack's layer on its own, as vanilla does: a refused
layer leaves the states to the layers below it.

## Block tint sources

A block tint source (NeoForge's `RegisterColorHandlersEvent.BlockTintSources`) colours a greyscale block model,
as vanilla colours grass and leaves: `Hooks::register_block_tint_sources` registers each block's list of layers
once at start, and a model face with `tintindex` i is multiplied by layer i. A layer is one of vanilla's sources
(`VanillaTint`: `Constant`, `ConstantSplit`, `Grass`, `Foliage`, `Water`, `Redstone`...) or the mod's own
`BlockTintSource`, with vanilla's four methods:

- `color(state, metadata)`: the colour away from a level (a block in a minecart, a mod's drawn block);
- `color_in_world(state, metadata, level, pos)`: the chunk mesher's colour, and a block a piston moves; it defaults to
  `color`;
- `color_as_terrain_particle(state, metadata, level, pos)`: the bits the block breaks into; it defaults to
  `color_in_world`;
- `relevant_properties()`: the properties the colour depends on. Vanilla groups states for shared baked models
  by them; Sunrise bakes every state and does not read it.

```rust,ignore
struct Dyed;
impl BlockTintSource for Dyed {
    fn color(&self, state: &BlockState, metadata: &dyn BlockMetadata) -> u32 {
        match state.property(metadata, "color").ok().flatten() {
            Some("red") => 0xFFB02E26,
            _ => 0xFFF9FFFE,
        }
    }
}

fn register_block_tint_sources(
    &self,
    _context: &RegistrationContext,
    event: &mut RegisterBlockTintSourcesEvent<'_>,
) {
    event.sources.register(
        vec![TintLayer::Source(Arc::new(Dyed)), VanillaTint::Grass.into()],
        vec!["example:dyed_lamp".into()],
    );
}
```

Colours are ARGB; the faces take their RGB, as vanilla's block renderer does. The mesher asks `color_in_world`
on its worker threads, once per block and tint index each time it meshes the section, so a source keeps no
mutable state and gets no `Host`. Each colour method receives an explicit immutable `BlockMetadata` resource, also used by `BlockState::property`, `with` and `of`. `level` is a `BlockAndTintGetter` over the section's chunk and its eight
neighbours: block and fluid states, light, face shading and `block_tint(pos, resolver)` for the biome colours.
Block entities are not part of it. A block with no mod layers meshes as before, and the registrations hold after
a pack change or F3+T. Fluids take their colour from their fluid model, not from these registrations.

## Cauldron fluid content

`register_cauldron_fluid_content` runs once the block registry is resolved, before any world
opens, and again when resources reload. It collects a `CauldronFluidContents` registrar:
`register(block, fluid, total, level)` says that `block` holds `total` millibuckets of `fluid`
when full, over the levels of its integer property `level` (`"level"` for a block built like
`LayeredCauldronBlock`), or only ever full when `level` is none, as a lava cauldron. The block
and the fluid are the keys `register` requested (`block_key`, `fluid_key`, `declare_fluid`).

Vanilla's water (`minecraft:water_cauldron`, 1000 mB over `level` 1..3) and lava
(`minecraft:lava_cauldron`, 1000 mB, always full) are built into the same table, so the fluid
capability answers every cauldron that has a content alike: each level holds `total` ÷ the
number of levels, a fill adds whole levels and turns an empty `minecraft:cauldron` into the
content's block at the level the amount reaches, and a drain takes whole levels and turns the
last one back into `minecraft:cauldron`. Powder snow is no fluid, and its cauldron has no
content.

The engine refuses, with one logged warning naming both sides, a block that already has a
content (vanilla's or an earlier mod's), a fluid that already has a cauldron (one content per
fluid, as NeoForge keeps), an unknown block or fluid, a level property the block doesn't have
or that isn't an integer property, and a total of 0 or one that doesn't divide evenly by the
number of levels. The rest of a mod's contents still apply. What a cauldron does when an item
is used on it is not part of this hook.

## Cauldron interactions

`register_cauldron_interaction` registers item and item-tag callbacks on both sides. See [Registering cauldron interactions](../how-to/world/cauldron-interactions.md) for the server-side `Interaction::interact` callback, overwrite-in-place precedence and server-corrected prediction contract.

## Colour resolvers

A colour resolver (vanilla's `ColorResolver`, NeoForge's `RegisterColorHandlersEvent.ColorResolvers`) is a colour
calculated from a biome and a block column, which the game blends over the biomes around a block as it blends
grass, foliage and water: each block averages the resolver's colour over the `(2r+1)^2` columns of the biome blend
radius, each column in the biome the game zooms it to. `Hooks::register_color_resolvers` registers the mod's own
`BiomeColorResolver` once at start and gets back the `ColorResolver` to pass to `host.biome_tint(pos, resolver)`
from a client hook or `level.block_tint(pos, resolver)` from a `BlockTintSource`:

```rust,ignore
static FROST: OnceLock<ColorResolver> = OnceLock::new();

struct Frost;
impl BiomeColorResolver for Frost {
    fn color(&self, _biome: &str, info: &ResolverBiome, _x: f64, _z: f64) -> u32 {
        if info.temperature < 0.15 { 0xE0F0FF } else { info.colors.foliage }
    }
}

struct FrostLeaves;
impl BlockTintSource for FrostLeaves {
    fn color(&self, _: &BlockState, _: &dyn BlockMetadata) -> u32 {
        0xFF48B518
    }
    fn color_in_world(
        &self,
        _: &BlockState,
        _: &dyn BlockMetadata,
        level: &dyn BlockAndTintGetter,
        pos: [i32; 3],
    ) -> u32 {
        level.block_tint(pos, *FROST.get().expect("registered at start"))
    }
}

fn register_color_resolvers(
    &self,
    _context: &RegistrationContext,
    event: &mut RegisterColorResolversEvent<'_>,
) {
    FROST.get_or_init(|| event.resolvers.register(Arc::new(Frost)));
}
```

`color(biome, info, x, z)` gets the biome's id, its temperature, downfall and colours (after `biome_colors`) and the
block column, and returns 0xRRGGBB. The mesher asks it on its worker threads, once per column the section's tinted
blocks blend over, so a resolver keeps no mutable state and gets no `Host`. Its colours are cached for one mesh of
one section only: a resource or pack reload, a biome colour refresh and a block change all mesh the section again
and ask the resolver again.

## Command argument kinds beyond the basics

- Besides `word`, `text`, `integer`, `number`, `boolean`, `player`, `item` and `block_pos`, an argument's kind may be `vec3`, `vec3_uncentered`, `vec2`, `vec2_uncentered`, `column_pos`, `rotation`, `dimension` or `nbt`. Each is sent to clients as vanilla's parser, so `~`, `^` and the dimension list complete as in a vanilla command. Neither `vec3`/`vec3_uncentered` nor `vec2`/`vec2_uncentered` can share a position in one command's usages, as they look the same to a client.

- `vec3` and `vec3_uncentered` deliver `CommandArgumentValue::Vec3`; `vec2` and `vec2_uncentered` deliver `CommandArgumentValue::Vec2` (`x` and `z` as `[f32; 2]`, `~` relative parts only); `column_pos` delivers `ColumnPos`, `rotation` delivers `Rotation` (yaw, pitch), `dimension` delivers `Dimension(LevelRef)` and `nbt` delivers `Nbt(Compound)`. `~` and `^` resolve against the source's position and rotation. A `dimension` the world has not loaded is refused with `argument.dimension.invalid` before the executor runs.

- `string` reads one word or one `"quoted"` / `'quoted'` phrase, as Brigadier's `StringArgumentType.string()` (`QUOTABLE_PHRASE`), and other arguments may follow it. A registered argument type reads the same with `CommandArgumentKind::Quotable` (otherwise one word, or the rest of the line with `Greedy`). Word, `string` and greedy text values are strings: a `string` arrives decoded as `CommandArgumentValue::String`, without its quotes and escapes, and a `Quotable` type's `parse` gets that decoded text. Clients receive both as `brigadier:string` mode 1.

## Debug screen entries

`Hooks::register_debug_entries` is the client registration-phase counterpart of NeoForge
26.3's `RegisterDebugEntriesEvent`. It receives a `RegistrationContext` once at client startup
and gives each mod an empty `event.entries` batch. Use any valid `<namespace>:<path>`
identifier (including custom `minecraft:<path>` IDs that do not collide with built-in vanilla
entries). A malformed or duplicate ID is logged and left out.

Declare text with `DebugEntry::text(id, group, lines)`. The `DebugGroup` chooses which
lines it joins: vanilla's `Position`, `Light`, `Memory`, `LookingAtBlock`, `LookingAtEntity`,
`LookingAtFluid` or `ChunkGeneration`, or `Named(id)`, a group of the mod's own that every
entry naming the same identifier joins. The client owns the callback for its lifetime. Only
while the entry is visible, the client calls the callback under the registering mod's identity,
with the current `&dyn ClientHost` and a `&mut Vec<String>` to append lines to. Legacy colour
codes (§) work. If the callback appends nothing, nothing joins the group. The lines are
drawn locally, never uploaded or recorded.

Declare a renderer switch with `DebugEntry::renderer(id)` (vanilla's `DebugEntryNoop`). It has
no text group or callback. Query its live state during
[`Hooks::register_debug_renderers`](#debug-renderers) with
`event.is_debug_entry_enabled(id)`.

Each entry appears in the corresponding text or renderer section of the debug options,
sorted by ID with vanilla's, and starts off. Its status (`alwaysOn`, `inOverlay`, `never`) is
saved in `debug-profile.json` under the full ID. By default, reduced debug info hides an entry
(matching vanilla's `DebugScreenEntry.isAllowed`); call `.with_reduced_debug_info(true)` on a
`DebugEntry::text` or `DebugEntry::renderer` to keep it allowed under reduced debug info.
Native text is appended after vanilla's entries in registration order.

## Debug renderers

`Hooks::register_debug_renderers` is the client counterpart of NeoForge 26.3's
`RegisterDebugRenderersEvent`, and like it fires during vanilla's
`DebugRenderer.refreshRendererList`: once as the client starts, on each login, on the first view
of a world since the start, and on the next view of a world after the debug selection changes.
That is a status set in the debug options or by a key, a profile loaded, the F3 overlay opened or
closed, or reduced debug info rebuilt. Respawn also invalidates the list for the next view.
Drawing the title or connect screen doesn't refresh anything. The hook receives a
`RegistrationContext`, so it reads no world. Each mod receives an empty `event.renderers`
batch and pushes `DebugRenderer::new(draw)` per active renderer.

Each refresh replaces the active list, so push every renderer that should draw until the next
refresh and capture its per-refresh state in the callback. To gate a renderer on a debug
options switch (either a built-in vanilla entry such as `"minecraft:chunk_borders"` or a
startup `DebugEntry::renderer(id)` from `Hooks::register_debug_entries`), check
`event.is_debug_entry_enabled(id)` before pushing `DebugRenderer::new(draw)`. Unconditional
renderers require no switch entry. Every submitted renderer from an enabled mod is kept in
registration order, with no ID-based filtering or deduplication.

For each world view, the client calls the installed callbacks in registration order,
under their registering mods' identities, after `submit_custom_geometry`. It passes the current
`ClientHost`, `WorldRenderView` (lending the frame's `Camera`, world partial tick, game time,
improved-transparency state, and visible `RenderableSection` slice) and the same
`WorldDrawContext`, so [its submission rules](#custom-world-geometry) apply and are shared.
Output clears every view, and callbacks don't run on the title or connect screens.

## Dimension transition screens

Push `DimensionTransitionScreen::new(transition, factory)` into `event.screens`.
Use `DimensionTransition::Incoming(to)`, `Outgoing(from)` or `Conditional { to, from }`
with namespaced dimension IDs. The client resolves those IDs at registration and logs and
rejects invalid or duplicate transition keys.

On a matching trip, the client calls the selected factory once as the registering mod and
keeps its presentation until the loading screen closes. An existing loading screen (including
vanilla's) keeps its presentation if another level load starts while it is open. Login and
death respawn do not select factories when they open a new loading screen. Return a boxed
`DimensionTransitionPresentation` (a draw closure or an object); each draw receives the current
reason and optional smoothed integrated-server progress. The existing tracker still governs
readiness, timeout and closing.

## Entity layers

An entity layer (NeoForge's `EntityRenderersEvent.AddLayers` and `RenderLayer`) draws over a living entity
renderer or replaces one of its stock layers: armour of a mod's own, a glow, something held or worn, or a
custom head block in place of a snow golem's pumpkin. `Hooks::register_entity_layers` is asked on every
resource load, after the entity renderers resolve, since a reload builds the renderers again and neither
added layers nor hidden stock layers carry over; the game resolves each target's layers and hides to the
slot its draw reads, so a frame hands an entity to its target's layers and hides only. In the ordered body
pass the layers submit in level join order, including the local player at its own slot when its body is
submitted, interleaved with submissions from general entity renderers.

```rust,ignore
struct Halo;
impl EntityRenderLayer for Halo {
    fn submit(
        &self,
        _entity: &EntityRenderState,
        model: Option<&dyn FrameModel>,
        draw: &mut dyn BlockEntityDrawContext,
        _stock: &mut dyn StockLayerDraw,
    ) -> Result<(), String> {
        // Skip the halo when the body has a head and that part or an ancestor was hidden.
        // A replacement renderer or a body without this part still gets a halo.
        if model.and_then(|body| body.part("head")).is_some_and(|head| !head.visible) {
            return Ok(());
        }
        // In the entity's frame: at its feet, turned with its body. Any atlas sprite will do:
        // a vanilla one, as here, or one of the mod's own assets.
        let face = Some(CubeFace::new("block/gold_block"));
        draw.cube([-4.0, 34.0, -4.0], [4.0, 35.0, 4.0], [face; 6], 0xFFFFFF);
        Ok(())
    }
}

fn register_entity_layers(
    &self,
    _context: &RegistrationContext,
    event: &mut RegisterEntityLayersEvent<'_>,
) {
    let halo = Arc::new(Halo);
    for target in [
        LayerTarget::Entity("minecraft:zombie".into()),
        LayerTarget::Player(PlayerModel::Wide),
        LayerTarget::Player(PlayerModel::Slim),
    ] {
        event.layers.add(target, halo.clone()).unwrap();
    }
}
```

The event tells what renderers this load built: `entity_types()` borrows the sorted slice of every entity
type with a renderer, `is_living(type)` says whether it is a living entity's, `player_models()` borrows
the `[Wide, Slim]` slice, `has_mod_renderer(type)` says whether a mod registered the type's renderer, and
`stock_layers(&target)` borrows the slice of `StockLayer` variants that target's renderer draws (empty for
a mod renderer or a target that takes no layers). No renderer object crosses the API.

`hide(target, stock_layer)` removes one stock layer from `target`'s renderer for this resource load
(mirroring removing a vanilla layer from a renderer's `layers` list in `AddLayers`): for example, hiding
`StockLayer::SnowGolemHead` on `minecraft:snow_golem` stops vanilla's carved pumpkin from drawing beneath a
mod's replacement head layer, and hiding `StockLayer::Armor(slot, pass)` on `PlayerModel::Wide` and
`PlayerModel::Slim` omits worn player armour everywhere that renderer draws (the world and screen previews).
`hide` refuses, by name, a target that can't take layers or a layer not in `stock_layers(&target)`. Hiding
the same layer twice is harmless.

For per-entity decisions (such as hiding armour only on selected players), hide the stock layer on the
target in `register_entity_layers`, add a mod layer to that target, and call
`stock.draw_stock_layer(layer)` inside `EntityRenderLayer::submit` for the entities that should still show
it. `draw_stock_layer` submits the stock layer with the entity's exact pose, lighting and visibility
conditions for that frame (an empty slot draws nothing and returns `Ok(())`). It returns `Err` for a layer
the renderer lacks or one that was not hidden on this target. Anything it drew rolls back with the rest of
`submit` on an `Err` return or a panic.

A layer attaches only to a living renderer, a mod's own included: `add` refuses a target that is no entity
type, a type whose renderer isn't a living one (a boat, a display, a projectile), `minecraft:player`,
whose layers go to a player model, and a living type that draws through a route of its own which doesn't
run layers yet (the armour stand, the ender dragon); the `Err` names the target, and the game logs it too. An
entity whose `render_living_pre` or `render_player_pre` a mod canceled draws none of its layers, as
NeoForge's canceled `RenderLivingEvent.Pre` skips them. Several mods may add to one target: their layers
draw in successful registration order. A panic or phase refusal discards only that listener's added
layers and stock-layer hides. Layers draw after the entity's model and its unhidden vanilla layers, each
from the entity's frame (at its feet, turned with its body, as `render_living_pre` draws). `entity.yaw` is
the body's turn and `entity.pitch` the head's. `model` lends the final body hierarchy for the callback,
including for nonhumanoids, or `None` if no body pose was produced (such as after a successful replacement
renderer). Read a named part or follow the hierarchy and compose its parent transforms to attach
geometry. A layer that returns an `Err` rolls back what it drew that frame, logs a warning, and stays
active for later draws while the other layers still run. A layer that panics rolls back what it drew that
frame and disables the mod's layer instance for the rest of the session, with an error log and an
in-game error toast naming the mod. Re-adding the same `Arc` on reload leaves it disabled regardless of
its target or position. A new instance starts enabled. The layers draw on the mobs (vanilla's and the
content mods' living types) and the players; the armour stand and the ender dragon take them once
their own draws run layers, in a follow-up of #860, which lifts their refusal.

## Entity renderers

An entity renderer (NeoForge's `EntityRenderersEvent.RegisterRenderers` and `EntityRenderer`) gives an
entity type code that draws it. `Hooks::register_entity_renderers` registers one renderer per entity type
once at start, before resources load, and the game resolves each type's renderer on every resource load
(the first one and every reload after it), so a frame hands an entity to the renderer of its type: no
lookup by name, no scan over the mods. In the ordered body pass, general, living, primed TNT and
experience orb renderers submit at each entity's slot in level join order.

```rust,ignore
struct Bolt;
impl EntityRenderer for Bolt {
    fn submit(
        &self,
        entity: &EntityRenderState,
        draw: &mut dyn BlockEntityDrawContext,
    ) -> Result<(), String> {
        // The pose starts at the entity, not turned: face the way it flies.
        draw.rotate_about([0.0, 1.0, 0.0], -entity.yaw, [0.0; 3]);
        draw.rotate_about([1.0, 0.0, 0.0], entity.pitch, [0.0; 3]);
        let face = Some(CubeFace::new("example:entity/bolt"));
        draw.cube([-1.0, -1.0, -4.0], [1.0, 1.0, 4.0], [face; 6], 0xFFFFFF);
        Ok(())
    }
}

fn register_entity_renderers(
    &self,
    _context: &RegistrationContext,
    event: &mut RegisterEntityRenderersEvent<'_>,
) {
    event.renderers.register("example:bolt".into(), Arc::new(Bolt));
}
```

The renderer draws with the helpers a block entity or mob submit hook draws with (`quad`, `cube`, `block`,
`item`, `model_part`), under a pose stack that starts at the entity's interpolated position, in blocks,
not turned; `EntityRenderState` has the frame's partial tick, the entity's age and its interpolated yaw
and pitch, and `draw.light()` the entity's light. A living entity's state also has `living`, its
`LivingRenderState`: body and head turns, walk cycle, death time, scale, pose, bed and flags, by the
reference's names. What it draws replaces the type's own drawing (a thrown type's flying item; a living
type's model and its vanilla layers, held items and worn blocks included); the glowing outline still
draws around it. Entities of a type share the one
renderer, whose `submit` has no `Host` and no level: a renderer that needs the world reads it in a
hook. A general entity type (`declare_general_entity`) draws only through its renderer. Its
`EntityRenderState::attachments` holds the immutable generation captured before submission, and is
`None` only for other entity families.

A later registration for a type replaces the earlier one, whichever mod made it, as in NeoForge. When
resources load, a registration for a type the game does not know is logged with the type's id and left
out; one for a known type whose family does not draw through registered renderers yet is kept silently,
and the type keeps its own drawing until its family does. Today these routes draw through registered
renderers: the content mods' thrown types (a `data/<id>/sunrise/entity_type/<name>.json` with the
`minecraft:snowball` template), the living types drawn as mobs, vanilla's and the content mods'
(not the player, whose model goes by its skin, and not the armour stand or the ender dragon, which draw
through routes of their own), primed TNT (`minecraft:tnt`, in place of the flashing cube, reading
`tnt_fuse`), experience orbs (`minecraft:experience_orb`, in place of the billboarded orb quad, reading
`orb_value`), paintings (`minecraft:painting`, in place of the painting's quads,
reading `painting`), and item frames and glow item frames (`minecraft:item_frame` and
`minecraft:glow_item_frame`, in place of the frame model and the item or map on its face, reading
`item_frame`). Primed TNT's `age` is its unsaved `tickCount` plus the partial tick; its yaw and pitch
are its current rotation, not interpolated (it has no previous rotation). A painting's `age` is its unsaved `tickCount` plus the partial tick; its yaw is
`Direction.get2DDataValue * 90` and its pitch is 0 (it does not interpolate rotation). An item frame's `age` is its unsaved `tickCount` plus the partial tick; its yaw and
pitch follow `ItemFrame.setDirection` and are not interpolated (it has no previous rotation). Around a living type's renderer everything else stays: `render_living_pre`
(whose cancel skips it), the mods' layers and `render_living_post`, in the order the type's own
model has, so it changes with #3915 and not before. When the replacement succeeds, the layers and
Post receive `model: None`. Boats, displays and the other
projectiles come in follow-ups of #860. A renderer that returns an `Err` rolls back what it drew that
frame, logs a warning, falls back for that draw to what the type would draw without a registered
renderer, and stays active for later draws. A renderer that panics rolls back what it drew that frame and
disables only that renderer for the rest of the session (including across reloads), with an error log and
an in-game error toast naming the mod; until the session ends its type draws as it would without one.

## Entity spectator shaders

`Hooks::register_entity_spectator_shaders` is the client registration-phase counterpart of NeoForge's
`RegisterEntitySpectatorShadersEvent`. Call `event.register(shader)` with an `EntitySpectatorShader`
record naming the spectated entity type and its post effect. The post effect is an ordinary
post chain, `assets/<namespace>/post_effect/<path>.json`, which the mod's assets or a resource pack
supply, so its passes follow the same pack rules as vanilla's `minecraft:creeper` and
`minecraft:invert`; no shader code or GPU object crosses the mod API.

When the first-person view follows an entity (spectator mode, or a server's camera), the game
applies the registered effect for that entity's type (`GameRenderer.checkEntityPostEffect`). The
third-person view and leaving the camera clear it. The first registration for each entity type wins:
vanilla's creeper, spider, cave spider and enderman count as first, so a mod can't change what the
vanilla mobs show. A later registration for the same type, and one whose ids aren't namespaced, is
logged and skipped. Registered chains compile in the background with vanilla's, from the first frame
in a world.

```rust,ignore
fn register_entity_spectator_shaders(
    &self,
    _context: &RegistrationContext,
    event: &mut RegisterEntitySpectatorShadersEvent<'_>,
) {
    event.register(EntitySpectatorShader {
        entity: "example:wisp".into(),
        shader: "example:wisp_vision".into(),
    });
}
```

## In-world game tests

`register_game_tests` runs once per world open before test-instance definitions resolve their functions.
Register an `Arc<dyn GameTestFunction>` through `event.functions.register("example:check", callback)`.
Add a test with `event.functions.register_test(id, GameTestDefinition::new(structure, function))`,
or use a data-pack `test_instance` with type `minecraft:function` and function `example:check` selects it.
The callback receives a `ServerHost` and `GameTestContext`: use `context.level` for level handles and
`context.absolute_pos(relative)` for rotated structure coordinates. Return `Pass`, `Fail(message)` or
`Pending`; pending attempts keep the ordinary timeout. Each retry invokes the function again.
Definitions retain resolved callbacks only for their world snapshot. Unknown functions fail normally;
`minecraft:always_pass` remains built in. Data-pack test definitions override code-registered defaults. Registration refuses duplicates, invalid names and the vanilla
namespace. A panic or phase refusal discards both the listener's functions and its definitions,
leaving their keys available to later listeners. This adds no sequence engine and no per-tick registry lookup.

## HUD layers

`Hooks::register_gui_layers` is the client registration-phase counterpart of NeoForge 26.3's
`RegisterGuiLayersEvent`. It receives a `RegistrationContext` and a `GuiLayerRegistrar` as
`event.layers`, once at client startup. Call `layers.register(id, placement, Box::new(draw))`
with a namespaced ID, a `GuiLayerPlacement` and an owned draw callback. The four placements
are `BelowAll`, `Below(id)`, `Above(id)` and `AboveAll`, as `registerBelowAll`, `registerBelow`,
`registerAbove` and `registerAboveAll`. `Below` and `Above` name a vanilla layer (the IDs
`Hooks::gui_layer_pre` lists) or a layer already registered, including in this callback's own
batch. The game publishes a completed batch in call order before invoking the next listener.
A failed listener publishes none of its pending insertions, replacements or wrappers. A panic
in that listener, including while dropping one of its superseded bodies, disables its registration
callback for the session and reports its mod in the log and client error toast. Invalid or duplicate
IDs (including built-in vanilla layer IDs) and unknown relative anchors are logged and refused at
the call. The client calls each installed callback as the mod that registered it.

Use `layers.replace_layer(id, Box::new(draw))` to discard an existing body, or
`layers.wrap_layer(id, Box::new(wrap))` to add an outer wrapper. Both keep the slot's ID,
order and dispatch. A replacement does not inherit eligibility checks in the body it replaces.
A wrapper uses `ctx.with_draw` to paint with the current scoped drawing context and
`ctx.draw_previous()` to run the previous body, including its own checks, at that point.
It can omit that call to skip the previous body. Both methods refuse an unknown ID.
Bodies from earlier listeners retire after publication in first-replacement order. A panic while
retiring a body is reported against its owner without undoing the replacement or stopping the next listener.

A layer fires `Hooks::gui_layer_pre` with its ID, runs its installed body, and fires
`Hooks::gui_layer_post` once. No body or Post runs if Pre cancels. Drawing a previous
body does not fire layer hooks again. A callback panic disables only that callback for the
session and reports its mod through the shared error log and toast. Completed paint stays in
the batch, and errors returned by a drawing method do not disable the callback.

The status slots run in order: `minecraft:player_health`, `minecraft:armor_level`,
`minecraft:food_level`, `minecraft:vehicle_health` and `minecraft:air_level`.
F1 hides their vanilla paint, not their installed bodies or layer events. The vanilla heart,
armor, food and air bodies need a player in survival or adventure, and vanilla vehicle health
needs a mount with hearts. Custom paint can check `host.hud_mode()` if it should respect F1.
`Above("minecraft:player_health".into())` puts a new layer between hearts and armor;
use `Above("minecraft:air_level".into())` to put it above all status rows. Use
`Below("minecraft:player_health".into())` to stay below the player rows or
`Above("minecraft:vehicle_health".into())` to stay above mount hearts. No single anchor
keeps a layer both above the mount hearts and below the player rows after the reorder.

The whole walk runs under F1: visibility checks belong to the original bodies, not to
inserted layers, replacements or wrappers. A canceled `Hooks::render_gui_pre` skips the whole walk and `Hooks::render_gui_post`. Use
`Hooks::render_gui_post` to draw after the layer walk, or `Hooks::screen_render_post` to draw
above an open screen.

## Item model types

An item definition (`assets/<namespace>/items/<path>.json`) is a tree of model nodes: vanilla's `minecraft:model`,
`condition`, `select`, `range_dispatch`, `composite`, `special` and the rest. A mod adds a node type of its own
(NeoForge's `RegisterItemModelsEvent`) once at client startup in `Hooks::register_item_models` via
`event.register(id, decode)`: `decode` owns the node's schema and reads its original JSON object through
`ItemModelConfig::field` or `ItemModelConfig::decode` once on each load and reload. It
declares nested child item models on `children: &mut dyn ItemModelChildren` (`add_named_models`, `add_field`,
`add_list`, `add_object`, or `add_model`), and returns the bound per-stack callback.

Child models registered through `children` are ordinary item model nodes, decoded and baked by the game.
Per stack, the bound callback pushes an `ItemModelLayer` for each child key to draw, in order:

```rust,ignore
fn register_item_models(
    &self,
    _context: &RegistrationContext,
    event: &mut RegisterItemModelsEvent<'_>,
) {
    event.register(
        Identifier::parse("example:charge").unwrap(),
        |_config, children| {
            children.add_named_models()?;
            Ok(|_host, input, layers| {
                let name = if input.stack.count() > 1 { "full" } else { "empty" };
                layers.push(ItemModelLayer::new(name));
            })
        },
    );
}
```

```json
{"model": {"type": "example:charge",
  "models": {"empty": {"type": "minecraft:model", "model": "example:item/cell_empty"},
             "full": {"type": "minecraft:model", "model": "example:item/cell_full"}}}}
```

Call `children.use_transformation()?` to decode the standard `transformation` field and apply it to every model the node draws. Otherwise no standard transform is read; your schema can use the name for something else. A key not declared on `children` draws nothing.
A layer also owns whose display transform its models draw under: `ItemModelLayer::new` keeps each model's
own transform for the display context (a hand, a screen, the ground), while
`.with_item_transform(ItemTransformMode::None)` draws its models, and every model nested under them, with no
display transform, only centred as every item is (use it where the node sits in a model that already placed
the item, so the transform is not applied twice).

Any valid `Identifier` (including `minecraft:`) is accepted; a later registration for the same id replaces
the earlier one (last-writer-wins, matching NeoForge's `LateBoundIdMapper`), and registering a built-in
vanilla or engine item model type id overrides that built-in when decoding item definitions (including
vanilla schemas whose child models live in arrays or arbitrary JSON fields rather than a `models` object).
Returning `Err(reason)` from the decoder rejects the enclosing item definition without falling back to a
built-in; panicking in the decoder or per-stack callback disables that mod's callback for the session and
surfaces an error toast. A node type no mod registered is an error naming it: the item definition holding it
isn't loaded, and the item draws the missing model.

## Item tint sources

An item tint source (NeoForge's `RegisterColorHandlersEvent.ItemTintSources`) colours a greyscale item per
stack. Register it once at client startup in `Hooks::register_item_tint_sources` via
`event.register(id, decode)`: `decode` parses the tint entry's JSON object (as a `Tag`) once when an item
definition is loaded or reloaded and returns the bound per-stack ARGB callback.

```rust,ignore
fn register_item_tint_sources(
    &self,
    _context: &RegistrationContext,
    event: &mut RegisterItemTintSourcesEvent<'_>,
) {
    event.register(Identifier::parse("example:charge").unwrap(), |config| {
        let default = match config.get("default") {
            Some(Tag::Int(value)) => *value as u32,
            _ => 0xFFFFFF,
        };
        Ok(move |_host, input| {
            let rgb = if input.stack.count() > 1 { 0x55FF55 } else { default };
            0xFF000000 | rgb
        })
    });
}
```

Use it as an entry of a `minecraft:model`'s `tints`, `{"type": "example:charge", "default": 65280}`: the
entry's place in the list is the `tintindex` it colours. Any valid `<namespace>:<path>` identifier (including
`minecraft:`) is accepted; a later registration for the same id replaces the earlier one (last-writer-wins,
matching NeoForge's `LateBoundIdMapper`), and registering a built-in vanilla tint source id overrides that
built-in when decoding item definitions. Returning `Err(reason)` from the decoder rejects the enclosing item
definition without falling back to a built-in; panicking in the decoder or per-stack callback disables that
mod's callback for the session and surfaces an error toast. A foreign source no mod registers draws white,
decided once at load.

## Animation types

A keyframe animation file (`assets/<namespace>/neoforge/animations/entity/<path>.json`) names a target per
entry and an interpolation per keyframe: vanilla's `minecraft:position`, `minecraft:rotation` and
`minecraft:scale`, and `minecraft:linear` and `minecraft:catmullrom`. A mod registers its own (NeoForge's
`RegisterJsonAnimationTypesEvent`) once at client startup via `Hooks::register_json_animation_types`,
binding each identifier directly to its callbacks. When animation files are parsed during resource load and
reload, each custom target's converter transforms the raw keyframe vectors into internal units (matching
NeoForge's `AnimationParser`) and binds the target and interpolation callbacks directly onto the parsed
`AnimationHolder`. Baking with `holder.bake(&root)` produces a `KeyframeAnimation` that calls those bound
callbacks directly during playback with no lookup per sample.

```rust,ignore
fn register_json_animation_types(
    &self,
    _context: &RegistrationContext,
    event: &mut RegisterJsonAnimationTypesEvent<'_>,
) {
    event.register_target(
        Identifier::parse("example:glow").unwrap(),
        |part, value| part.offset_scale(value),
        |raw| raw.map(|v| v * 0.5),
    );
    event.register_interpolation(
        Identifier::parse("example:step").unwrap(),
        |keyframes, _alpha, prev, _next, scale| keyframes[prev].post.map(|v| v * scale),
    );
}

// In client_setup and client_resources_reloaded, once:
let baked = holder.bake(&root);
// Each frame:
baked.apply(&mut root, &state, age_in_ticks);
```

Any valid `<namespace>:<path>` identifier (including `minecraft:`) is accepted; a target and an
interpolation with the same identifier are separate namespaces. Duplicate registrations and collisions with
the five built-ins (`minecraft:position`, `minecraft:rotation`, `minecraft:scale`, `minecraft:linear`,
`minecraft:catmullrom`) are logged as registration errors and the first registration stands. An animation
file naming an unregistered target or interpolation fails to load, naming it. If a custom target's
converter returns a non-finite vector, that animation file fails to load; if a callback panics during parse
or playback, that callback is disabled for the session with an error log and toast.

## Entity model layers

`Hooks::register_layer_definitions` is the client registration-phase counterpart of NeoForge 26.3's
`EntityRenderersEvent.RegisterLayerDefinitions`. It receives a `RegistrationContext` once at
client startup. Call `event.register(id, supplier)` to stage a supplier that builds a
`LayerDefinition`. Dedicated and integrated server roles do not fire this client event.
IDs have the form `<namespace>:<path>#<layer>`. Write the model `Identifier` in its full
`namespace:path` form even when a half is empty; `<layer>` is any suffix after the first `#`. Any valid ID that does not collide
with an existing vanilla layer definition is accepted; duplicate mod registrations for the same ID
overwrite earlier suppliers (last-writer-wins) before freeze. Malformed IDs, collisions with
existing vanilla layer definitions, or an unavailable/invalid vanilla `entity-models.json` key
table fail startup freeze before any supplier runs or the layer table publishes.

The game calls each winning supplier once when assembling the frozen layer table. Overwritten
suppliers never run. A supplier that is disabled, refuses or panics leaves no definition.
A panic names the mod and `build_layer_definition` in the shared error log and client error
toast; other suppliers, including that mod's other suppliers, still run. Bakes and resource
reloads do not call suppliers again.

A `LayerDefinition` holds a `MeshDefinition` of named parts whose `CubeListBuilder` boxes sit
at a `PartPose`, on a texture of the given size. Texture offsets, mirroring and
`CubeDeformation` match vanilla's model builders. This listener is compiled in the API tests:

```rust,ignore
{{#include ../../../../crates/sunrise-client/tests/it/mod_api_hooks.rs:register_layer_definitions}}
```

`ClientHost::bake_layer` bakes a registered layer or a vanilla layer by its
`entity-models.json` name (`minecraft:zombie#main`) into an owned `ModelPart` tree. All
layers are indexed once per client resource load and reload rather than read from disk on
each bake; callers should still bake once and reuse the owned `ModelPart`, calling
`reset_all_poses` and setting the parts' positions and rotations each frame as `setupAnim`
does.

Draw the tree with `BlockEntityDrawContext::model_part`, using the same draw context the mob
or block entity submit hook received. There is no separate Host drawing helper. Positions
are texels, +y down and the front towards -z; convert the mob's frame with scale
`[1.0, -1.0, -1.0]` then translation `[0.0, -1.501, 0.0]`, as `LivingEntityRenderer` does.
The sprite is the entity texture ID and the tint is `0xRRGGBB`.

## Model loaders

A model JSON may name a `loader`: NeoForge's `neoforge:composite`, `neoforge:obj` and `neoforge:empty` are built in, and a mod
registers one of its own (NeoForge's `ModelEvent.RegisterLoaders`) once at client startup via
`Hooks::register_model_loaders`. Call `event.register(id, schema, loader)` to bind a loader identifier to
its schema and bake callback. `ModelLoaderSchema::Custom` leaves every model field except `loader` to your
callback. Use `ModelLoaderSchema::ModelProperties` to decode standard fields such as `parent`, `textures`
and `display`, but supply your own geometry. Use `ModelLoaderSchema::Cuboids` to decode and bake the model's
cuboid elements alongside your quads.

```rust,ignore
fn register_model_loaders(
    &self,
    _context: &RegistrationContext,
    event: &mut RegisterModelLoadersEvent<'_>,
) {
    event.register(Identifier::parse("example:pillar").unwrap(), ModelLoaderSchema::ModelProperties, |_input| {
        // The north face of a 4-pixel post in the middle of the block.
        Ok(vec![ModelLoaderQuad {
            positions: [
                [0.625, 0.0, 0.375],
                [0.375, 0.0, 0.375],
                [0.375, 1.0, 0.375],
                [0.625, 1.0, 0.375],
            ],
            uvs: [[1.0, 1.0], [0.0, 1.0], [0.0, 0.0], [1.0, 0.0]],
            texture: ModelLoaderTexture::Slot("side".into()),
            cull: None,
            tint_index: -1,
            shade: true,
        }])
    });
}
```

`ModelLoaderInput` gives the callback the model identifier (`input.model`), the original JSON object as a
string (`input.config`), and the resolved standard texture slots (`input.textures`). A quad can name one of
those slots (`ModelLoaderTexture::Slot`) or directly name a sprite (`ModelLoaderTexture::Sprite`). Quads are
in block units, and the blockstate variant's rotation moves them whole. A model and effective slot set are
baked once per load, even when several variants reference them.

Any valid `<namespace>:<path>` identifier (including `minecraft:`) is accepted. Duplicate registrations and
collisions with `neoforge:composite`, `neoforge:obj`, `neoforge:empty` or `fusion:model` are registration errors; the first
registration stands. A loader ID written as a string, or as an object with `id` and no `optional` flag,
is required. When the loader is absent, only `"loader": {"id":"example:pillar","optional":true}` uses
ordinary cuboid geometry. An absent required loader rejects its declaring file and logs the error.
References to that file use the missing model at that link: a descendant can still supply its own
geometry. If a present loader returns `Err(reason)`, that bake uses the missing model and logs the
error. An empty successful quad list is not an error. Panicking also
disables that loader callback for the session and surfaces an error toast. A quad with a position or UV
that isn't finite, or a texture key the model doesn't define, is skipped by itself with a reason.

## Particle groups

`Hooks::register_particle_groups` is the client registration-phase counterpart of NeoForge
26.3's `RegisterParticleGroupsEvent`. It receives a `RegistrationContext` once at client
startup. Call `event.register` with a namespaced ID and a factory. The factory returns a
`ParticleGroup` with a `ParticleBlend` (`Opaque`, `Translucent`, or `Additive`) and the particle types
that draw in it. `Additive` draws after the translucent layer, depth-tested without depth writes,
blended additively.

```rust,ignore
fn register_particle_groups(
    &self,
    _context: &RegistrationContext,
    event: &mut RegisterParticleGroupsEvent<'_>,
) {
    event.register(Identifier::parse("example:glow").unwrap(), || ParticleGroup {
        blend: ParticleBlend::Translucent,
        particles: vec!["example:orbit".into(), "example:spark".into()],
    });
}
```

A group is vanilla's `ParticleRenderType` with the `ParticleGroup` the particle engine keeps for
it. Its particles are counted, admitted, ticked and removed apart from every other group's, so
each group has vanilla's own limit of 16384 live particles and a crowd in one cannot crowd out
another, and they are drawn after the built-in particles, groups in registration order. The group
decides the layer of its particles (`blend`), whatever their sprites hold. List each type by its
namespaced ID, such as `example:spark` for `assets/example/particles/spark.json`. This works with
or without a `register_particle_providers` provider, so a group adds no particle look or motion
of its own: that stays in the declaration and the provider. A type no group lists draws in the
built-in group as before, and the built-in particles are never moved. A type another group lists
already, or no sprite set matches, is logged and left out.

The game freezes factories in registration order. Any nonempty namespace and path is
allowed, including `minecraft`. Duplicate IDs fail client startup without invoking any
factory (matching NeoForge's `RegisterParticleGroupsEvent`). IDs with an empty namespace
or path are logged and skipped. Each particle engine calls the accepted factories once
when it is constructed. If a factory is disabled, refuses or panics, that engine omits
the group. When sprite sets load or reload, the game resolves each type's group from the
resulting engine-owned definitions. A spawn, a tick and a draw use the resolved group
handle, look nothing up by group name and scan no mod. Live provider-driven particles are
dropped when resources reload, in a group as in the built-in one.

## Particle providers

Register a common `ParticleType<T>` in `Hooks::register`, supplying its map and stream codecs.
`Hooks::register_particle_providers` registers the client factory with
`event.providers.register_sprite_set(ty, |sprites| Provider::new(sprites))`. The matching sprite
list is `assets/<namespace>/particles/<path>.json`. Spawn using `ty.options(value)`; a unit type
uses `ParticleMapCodec::unit()`, `ParticleStreamCodec::wire()` and `ty.options(())`.

The provider's `create_particle` receives a `ParticleSpawn<T>` containing a borrowed typed value,
client level, position, auxiliary inputs and provider RNG, plus a writable `ParticleTickView`.
It can edit the initialized particle and returns an owned `CustomParticle`, or `None` to decline.
The engine still owns the camera-distance, Particles-setting and group admission rules.
The instance's fields are its private state. It gets a live view on each tick; the default
runs `Particle.tick`. An override can advance age separately, move the particle and remove it.
Render output never decides whether a particle is alive. No instance or view crosses the network.

`CustomParticle::extract` borrows the particle and the current frame. The default submits the
normal quad with its sprite, lighting and interpolated pose. An override can emit zero or
more independently placed opaque or translucent quads. An invalid quad is omitted; a failed
callback does not commit that call's geometry. Resource reload does not replay the hook.

## Pipeline modifiers

`Hooks::register_pipeline_modifiers` is the client registration-phase counterpart of NeoForge's
`RegisterPipelineModifiersEvent`. A modifier is plain data (`PipelineModifier`): each field is an
optional override of `PipelineState` (`blend`, `depth_compare`, `depth_write`, `depth_bias`,
`cull`, `color_write`, `polygon`, `topology`). Because modifiers are data rather than closures,
applying them is idempotent by construction and the renderer caches each `(pipeline, modifier
stack)` variant lazily.

```rust,ignore
fn register_pipeline_modifiers(
    &self,
    _context: &RegistrationContext,
    event: &mut RegisterPipelineModifiersEvent<'_>,
) {
    event.register(
        PipelineModifier::new(Identifier::parse("example:xray").unwrap())
            .depth(WorldDepth::ThroughWalls)
            .depth_write(false),
    );
}
```

Resolve a modifier once in `client_setup` (or `client_resources_reloaded`) with
`host.render_assets().pipeline_modifier(&id)` and apply it around draws with
`WorldDrawContext::with_pipeline_modifier(modifier, &mut |draw| { ... })`. Scopes nest: the most
recently pushed modifier applies first, then older ones, so for any field set by two modifiers
the outermost wins (`PipelineModifierStack.apply`). Modifiers apply to `lines`, `quads`, `pipeline_quads`, and `draw_model_instances` (not
`name_tag` labels), but not all of a modifier applies everywhere. The built-in `lines`, `quads`
and `draw_model_instances` have no pipeline of their own to vary, so only `depth` applies to them
(it picks the through-walls or tested copy). The full `PipelineModifier` (`blend`, `cull`,
`color_write`, `depth_write`, `polygon`, `depth`) applies to pipelines a mod registers and draws
with `pipeline_quads`. A field a modifier leaves unset keeps the pipeline's own value, which
defaults to what a mod pipeline already had before `PipelineState` existed. On OIT variants, `blend`
and `depth_write` overrides are ignored because OIT owns target blending and read-only depth.
Variants the GPU refuses (such as `PolygonMode::Line` on a device without wireframe support) are
logged once with both IDs and skipped while the rest of the frame draws.

## World preset editors

`register_preset_editors` is the client registration-phase counterpart of NeoForge's
`RegisterPresetEditorsEvent`. Call `event.register(preset, factory)` to bind a world-preset
identifier directly to a factory that creates a [`PresetEditor`] when the player clicks
Customize on Create World. The preset itself comes from the enabled data packs'
`worldgen/world_preset` JSON; registering an editor alone does not create a preset. Add the
preset to `data/minecraft/tags/worldgen/world_preset/normal.json` to offer it in Create
World's type selector; unlisted presets remain hidden.

Put editor-specific input and drawing in the bound `PresetEditor`. Generic screen hooks can identify
its screen as `world_preset_editor`. Input first reaches the corresponding screen Pre hook; characters
from a key, the input method, or the control socket reach `PresetEditor::char_typed` independently of
key cancellation. Keys consumed by the input method during composition do not reach the editor or
its footer; committed text uses the character path. An unhandled character reaches CharacterTyped.Post;
an unhandled key reaches KeyPressed.Post after the editor and the normal title controls decline it.

```rust,ignore
struct IslandsEditor;

impl PresetEditor for IslandsEditor {
    fn title(&self) -> Option<String> {
        Some("Islands Customization".into())
    }

    fn render(
        &mut self,
        draw: &mut dyn DrawContext,
        rect: [f32; 4],
        _draft: &mut PresetEditorDraft<'_>,
    ) {
        draw.fill([rect[0] + 20.0, rect[1] + 40.0, 80.0, 20.0], 0xFF00FF00);
    }
}

fn register_preset_editors(
    &self,
    _: &RegistrationContext,
    event: &mut RegisterPresetEditorsEvent<'_>,
) {
    event.register(Identifier::parse("example:islands").unwrap(), |_ctx| {
        Ok(Box::new(IslandsEditor))
    });
}
```

Any valid `<namespace>:<path>` identifier (including `minecraft:`) is accepted. A later
registration for the same preset replaces the earlier one (last-writer-wins), and registering
`minecraft:flat` or `minecraft:single_biome_surface` overrides the built-in Superflat or
Single Biome editor.

Customize opens `world_preset_editor`, with the menu background, header title, and Done and
Cancel footer buttons supplied by the game. The bound [`PresetEditor`] draws its UI before
`screen_render_post`, handles mouse clicks, keys, and typed characters, and edits the working
dimension stems through `PresetEditorDraft::set_dimensions`. Screen hooks still fire on
`world_preset_editor`, and `host.preset_editor_dimensions()` /
`host.set_preset_editor_dimensions(...)` remain available while it is open. Done commits the
validated draft to world creation; Cancel and Escape discard it. Neither path changes an
existing world. Keep custom drawing above the bottom 33 GUI pixels reserved for Done and
Cancel.

Returning `Err(reason)` from a factory logs a warning and leaves Create World open without
disabling the factory. Panicking in a factory or editor callback disables that mod's preset
editor callbacks for the session and surfaces an error toast. Dimension ids, stem structure
and flat-layer height bounds are checked before accepting an edit; registry resolution and
generator-specific decoding still run when opening the world.

## Numeric item model properties

A client mod adds a numeric `minecraft:range_dispatch` property (NeoForge's
`RegisterRangeSelectItemModelPropertyEvent`) once at client startup in
`Hooks::register_range_select_item_model_property` via `event.register(id, decode)`: `decode` parses the
`range_dispatch` node's JSON object (as a `Tag`) once on each load and reload and returns the bound
per-stack numeric callback.

```rust,ignore
fn register_range_select_item_model_property(
    &self,
    _context: &RegistrationContext,
    event: &mut RegisterRangeSelectItemModelPropertyEvent<'_>,
) {
    event.register(Identifier::parse("example:fill").unwrap(), |_config| {
        Ok(|_host, input| input.stack.count() as f32 / 64.0)
    });
}
```

Use `"property": "example:fill"` in the item's `minecraft:range_dispatch`: the model's `scale` multiplies the
number, then the highest threshold at or below it wins, else `fallback`, as for vanilla's properties. Any
valid `<namespace>:<path>` identifier (including `minecraft:`) is accepted; a later registration for the same
id replaces the earlier one (last-writer-wins, matching NeoForge's `LateBoundIdMapper`), and registering a
built-in vanilla property id overrides that built-in when decoding item definitions. Returning `Err(reason)`
from the decoder rejects the enclosing item definition without falling back to a built-in; panicking in the
decoder or per-stack callback disables that mod's callback for the session and surfaces an error toast. A
foreign property no mod registers reads 0, decided once at load.

## Render pipelines

`Hooks::register_render_pipelines` is the client registration-phase counterpart of NeoForge's
`RegisterRenderPipelinesEvent` (and of 1.21.1's `RegisterShadersEvent`, which it replaced). A
pipeline is plain data: its id, a GLSL core shader program (`.vsh` + `.fsh`) in the mod's assets
or another namespace, optional preprocessor defines, optional sprite texture, blend mode and depth
mode. Pass it to `event.register(pipeline)`; registration takes no GPU object or shader handle.

```rust,ignore
fn register_render_pipelines(
    &self,
    _context: &RegistrationContext,
    event: &mut RegisterRenderPipelinesEvent<'_>,
) {
    event.register(
        RenderPipeline::new(
            Identifier::parse("example:glow").unwrap(),
            Identifier::parse("example:core/glow").unwrap(),
            PipelineBlend::Additive,
            WorldDepth::Tested,
        )
        .vertex_shader(Identifier::parse("minecraft:core/position_tex_color").unwrap())
        .define("PULSE"),
    );
}
```

The shader id `example:core/glow` names `assets/example/shaders/core/glow.vsh` and
`assets/example/shaders/core/glow.fsh` (matching vanilla's `withVertexShader`/`withFragmentShader`).
Call `.vertex_shader(id)` to reuse another program's `.vsh`, such as
`minecraft:core/position_tex_color` or a library mod's vertex stage. Both stages may use
`#include <ns:path>`, `#include "relative"`, and `#moj_import` resolved through the active
resource stack (vanilla assets, loaded mods, and resource packs), and both stages receive
`RENDERPEARL_DEPTH_IS_ZERO_TO_ONE` followed by any `.define(...)` / `.define_value(...)` entries
declared on the pipeline.

```glsl
#version 330
#include <minecraft:globals.glsl>
#include <minecraft:dynamictransforms.glsl>

in vec2 texCoord0;
in vec4 vertexColor;
out vec4 fragColor;

void main() {
    vec4 color = vertexColor * ColorModulator;
#ifdef PULSE
    color.rgb *= 1.0 + sin(GameTime * 24000.0 * 0.2);
#endif
    fragColor = vec4(color.rgb, color.a * (1.0 - texCoord0.y));
}
```

The engine binds vanilla's four uniform blocks (`Globals`, `Projection`, `DynamicTransforms` with
camera-relative `ModelViewMat`, `ColorModulator = 1`, `ModelOffset = 0`, `TextureMat = identity`,
and `Fog`), vertex attributes `Position`, `UV0` and `Color` by name, and — when `.textured(...)`
is set — `Sampler0` plus `<sunrise:pipeline_sprite.glsl>` providing `vec4 sampleSprite(vec2 uv)`.
Mod pipelines work in vanilla's gamma space, matching vanilla core shaders: write what vanilla
writes (`texture * vertexColor * ColorModulator`). On frames holding linear light, the engine
decodes the fragment's location-0 output after `main()` (sanitizing negative/NaN RGB to 0 and
clamping alpha coverage to `[0, 1]`). Expanded GLSL stages are guarded for crash/memory
containment (1 MiB expanded, 200 chained operators per expression, 200 bracket/unary/brace-less
nesting levels, include depth 16 / 1,024 pastes / 8 MiB scanned).

Resolve the pipeline with `host.render_assets().render_pipeline(&id)` in `client_setup`
and again in `client_resources_reloaded`, then draw with `WorldDrawContext::pipeline_quads` from
`submit_custom_geometry` (or a debug renderer). A resource reload builds every registered pipeline
again, so a resource pack can replace a shader or an include; a handle from before the reload is
`Stale`.

A pipeline is required unless it is registered with `.optional()` (NeoForge's
`registerPipeline` and `registerOptionalPipeline`). Any valid `Identifier` is accepted, and
a mod pipeline ID must be unique across both policies, because mod lookups are by ID. A
collision with vanilla is checked in the corresponding required or optional table. Invalid GLSL
define names or values fail registration at startup. When a load cannot build a required pipeline
because its `.vsh`, `.fsh`, or an include is missing, refused or turned down by the GPU, the whole
load fails and names it: the game does not start, and a reload keeps the previous resources and
handles, then drops the selected resource packs and tries again, as vanilla does with any failed
reload. An optional pipeline that cannot be built is logged once with its id and resolves to
`Invalid`, and the other pipelines draw. Pipeline quads grow dynamically into the GPU buffers
with no fixed per-view cap (skipping any quad with non-finite coordinates or UVs individually);
in the deferred world pass, pipeline quads draw after the plain quads and lines, in submission
order.

## Compute pipelines

`event.compute_pipeline(ComputePipeline::builder(id, shader_id).bind_group_layout(layout)
.define("NAME").build())` registers a GLSL 450 compute shader on the same registrar. Vanilla's
`renderpearl` API has no compute pass, so a compute pipeline follows what it has: the shader is
`assets/<ns>/shaders/compute/<path>.csh`, its
bindings are a [`BindGroupLayout`] of uniform blocks, samplers, texel buffers, std430 storage
buffers and `image2D` storage images in one bind group, and the dispatch binds them with the same
[`DrawBindings`] a draw takes. Resolve the handle with `host.render_assets().compute_pipeline(&id)`
in `client_setup` and again in `client_resources_reloaded`.

Dispatch from a frame-graph compute node (`event.graph.add_compute_pass(ComputePassDescriptor::new(..)
.reads(..).writes(..), |ctx| ctx.dispatch(..))` in `frame_graph_setup`), ordered by the targets it
reads and writes and never culled; the node runs outside any render pass. `dispatch` takes workgroup
counts (each at most the device's `max_compute_workgroups_per_dimension`; a zero is a no-op) and
`dispatch_indirect` reads three `u32` counts from a storage buffer created with `indirect: true`.
The workgroup size (`max_compute_workgroup_size_*`, `max_compute_invocations_per_workgroup`), the
workgroup storage (`max_compute_workgroup_storage_size`) and the binding counts are checked
against the device's limits when the pipeline builds; a shader needing subgroup operations is
refused with [`ComputePipelineError::UnsupportedFeature`] on an adapter without them
([`RenderCapabilities::subgroup`]). A required-limit violation or a bad binding is an `Err`, never
a panic or a lost device. A compute pipeline whose shader is missing or does not build is logged once
and resolves to `Invalid`; it never fails the load. Compute pipelines and the GPU objects they
bind outlive a resource reload and a world close, but a [`ComputePipelineRef`] goes stale on a
reload like every pipeline handle: resolve it again in `client_resources_reloaded`.

## Categorical item model properties

A categorical `minecraft:select` property (NeoForge's `RegisterSelectItemModelPropertyEvent`) is registered
once at client startup in `Hooks::register_select_item_model_property` via `event.register(id, decode)`:
`decode` parses the `select` node's JSON object (as a `Tag`) once on each load and reload and returns a pair
`(decode_case, eval)`. `decode_case` decodes each `when` entry from a `&Tag` into the property's key type `T`
once at load (rejecting invalid `when` entries with `Err(reason)`), and `eval` answers `Option<T>`
per stack.

```rust,ignore
fn register_select_item_model_property(
    &self,
    _context: &RegistrationContext,
    event: &mut RegisterSelectItemModelPropertyEvent<'_>,
) {
    event.register(Identifier::parse("example:mode").unwrap(), |_config| {
        Ok((
            |when: &Tag| match when {
                Tag::String(mode) => Ok(mode.clone()),
                _ => Err("expected string mode".into()),
            },
            |_host, _input| Some("dig".to_owned()),
        ))
    });
}
```

Each `when` value decodes as a nonempty list of keys or as one key. The list codec is tried first;
if decoding its entries mismatches, the original value is tried as a single key. This also supports
key types that decode from arrays. Duplicate keys are rejected. The case whose key equals the answer
is drawn, or `fallback` when there is no match, the evaluator answers `None`, or equality panics.
Any valid `Identifier` (including `minecraft:`) is accepted; a later registration for the same id replaces
the earlier one (last-writer-wins, matching NeoForge's `LateBoundIdMapper`), and registering a built-in
vanilla property id overrides that built-in when decoding item definitions. Returning `Err(reason)` from the
decoder or `decode_case` rejects the enclosing item definition without falling back to a built-in; panicking
in the decoder, `decode_case`, equality, or the per-stack callback disables that mod's callback for the session and surfaces
an error toast. A select on a foreign property no mod registers draws its fallback, decided once at load.

Use `event.renderers.register(kind, SpecialModelFactory::new(decode, bake))` with a
validated `Identifier` (bare paths default to `minecraft:`, matching vanilla and NeoForge). Use
`SpecialModelConfig::decode` to read the inline `model` fields other than `type` into your own
config type. The bake callback consumes that config and can keep geometry leases for the current
resource load. Reloading rebuilds those leases and renderers, without firing this hook again.
Only registrations from a completed listener count when choosing the last factory and reporting
replacements; a panic or phase refusal discards that listener's whole batch.

On each draw, `SpecialModelRenderer::extract_argument` reads a borrowed `SpecialModelStack`
and returns the argument for `submit`. Submit geometry or nested stacks through
`SpecialModelCollector::submit_model` and `submit_stack`, using `with_pose` to scope local
transforms. `SpecialModelSubmitOptions` lets each submission choose explicit `packed_light`
(vanilla `LightCoordsUtil` layout; `None` inherits draw light and baked `light_emission`),
`render_type` (`Baked`, `ItemCutout`, `ItemTranslucent`, or `Custom(RenderTypeRef)`), and
`lighting` (`Diffuse` or `LightmapOnly`). The game applies the `base` model's display
transforms, particle and GUI light, and handles foil, overlay and outline through the normal
item draw path.

An unknown type, a disabled winning factory, or a decode or bake error or panic refuses that
item definition. The game does not retry a built-in decoder or a lower pack. Decode, bake,
argument extraction and submit run as the mod that registered the type. A failed draw rolls
back the whole special layer. Ordinary refusals are reported on every failed invocation without
disabling the callback. A panic logs and shows an error toast naming the mod, and disables only
that registration's affected callback for the session, including after resource reloads.
Only the outermost refused layer draws the missing model.

## Sprite sources

An atlas definition (`assets/<namespace>/atlases/<path>.json`) lists the sources that give the atlas its
sprites: vanilla's `directory`, `single`, `filter`, `unstitch` and `paletted_permutations`. A mod registers a
source type of its own (NeoForge's `RegisterSpriteSourcesEvent`) once at client startup via
`Hooks::register_sprite_sources`. Call `event.register(id, provider)` to bind the type identifier directly to
its provider callback; when an atlas definition lists an entry of that type, the game invokes the bound
provider on every load and reload without scanning other mods.

```rust,ignore
fn register_sprite_sources(
    &self,
    _context: &RegistrationContext,
    event: &mut RegisterSpriteSourcesEvent<'_>,
) {
    event.register(Identifier::parse("example:checker").unwrap(), |input| {
        // A JSON number arrives as the narrowest tag that holds it: `8` is a `Tag::Byte`.
        let side = match input.config.get("size") {
            Some(Tag::Byte(size)) => (*size).clamp(1, 64) as u32,
            _ => 16,
        };
        let rgba = (0..side * side)
            .flat_map(|i| {
                if (i / side + i % side) % 2 == 0 {
                    [255, 0, 255, 255]
                } else {
                    [0, 0, 0, 255]
                }
            })
            .collect();
        Ok(vec![SpriteSourceOutput::Pixels {
            sprite: Identifier::parse("example:checker").unwrap(),
            width: side,
            height: side,
            rgba,
        }])
    });
}
```

Use it as an entry of a definition's `sources`, `{"type": "example:checker", "size": 8}`: `SpriteSourceInput`
hands the provider the atlas identifier (`input.atlas`), the entry's whole JSON object converted to a `Tag`
(`input.config`), and read-only access to the load's resources (`input.resources`). A provider returns
`Ok(outputs)` of `SpriteSourceOutput`, which apply in order as vanilla's `SpriteSource.Output` calls do:
`Texture` adds a texture of the resources as a sprite, `Pixels` adds pixels the mod made, and `Remove` drops
a sprite by id (vanilla's `removeAll`). A sprite of an id listed already replaces the earlier one, as a
built-in source's does. A source never sees what the entries before it listed: output is write-only.

Any valid `<namespace>:<path>` identifier (including `minecraft:`) is accepted; a later registration for the
same id replaces the earlier one (last-writer-wins, matching NeoForge's `LateBoundIdMapper`) with a warning
naming both mods. Registering a built-in vanilla type (`minecraft:directory`, `minecraft:single`,
`minecraft:filter`, `minecraft:unstitch`, `minecraft:paletted_permutations`) overrides that built-in when
decoding atlas definitions, with a warning naming the overriding mod. Returning `Err(reason)` logs the error
and lists no sprites for that entry without disabling the provider; panicking disables that provider for the
session and surfaces an error toast.

`input.resources` is the `ResourceManager` vanilla hands `SpriteSource.run`, read-only: `read(path)` gives
the winning copy of a resource (`assets/<namespace>/textures/<path>.png`), `read_stack(path)` every pack's
copy lowest first, and `list(prefix)` the paths under a prefix. A source can make a sprite from the pixels
of a texture any pack has, a player's retexture included. It is borrowed for the single callback invocation.

Each entry is asked once per atlas each time the client prepares its resources, and the answer is kept
for everything that reads the atlas, so a source that counts or varies its answers is still seen as one
answer. Sprite sources run in every client atlas that lists them (including blocks, items, GUI and
particles, and atlases declared by `register_texture_atlases`). Each atlas is asked under its own id, so
two atlases sharing a definition each get their own question. Each atlas keeps its own sprites, by its own id: when the blocks
and items atlases both list an id, a model's material is the items atlas's, and `texture_sprites` finds
the blocks atlas's sprite of that id (and every sprite a source made in the blocks atlas, though no model
names it) after `texture_atlas_stitched`. A sprite only the items atlas lists has no block array to be found
in. An entry whose type no mod registered fails its definition, as an unknown type fails it in vanilla, and
the log names the type and the entry's place. A `Texture` the resources lack, and `Pixels` whose length
isn't `width * height * 4` or whose side is over 32768 pixels, are logged and skipped one by one.

## Texture atlases

`register_texture_atlases` receives a registration context. Call `event.register(atlas)`
with a `TextureAtlasRegistration` giving an atlas id (`mymod:icons`) and its definition
(`mymod:icons` reads `assets/mymod/atlases/icons.json`). The definition is a `sources`
list like vanilla's atlas definitions, and resource packs change it the same way.

Each registered atlas has its own texture, stitched independently from the base
HUD and other registered atlases. Its sprites are exposed in the GUI lookup under
`TextureAtlasRegistration::sprite_name`:
sprite `mymod:heart` of `mymod:icons` is `mymod:gui/atlases/icons/mymod/heart`.
Look it up with `gui_sprite` in `texture_atlas_stitched` for `TextureAtlas::Gui`
and draw it with `DrawContext::sprite` by that name. Every reload lists the
sprites again from the definition as the packs now give it.

The hook runs at client startup and on the game-owning thread before reload
staging; re-submit the atlases on each call. Any namespace is allowed, including
`minecraft:` and `sunrise:`. Collisions with vanilla 26.3's built-in engine
atlases (`AtlasManager.KNOWN_ATLASES`, such as `minecraft:blocks` or
`minecraft:gui`) on either `definition` or `atlas`, and duplicate `definition`
or `atlas` ids, are logged as errors naming the registering mod and earlier
owner, keeping the first registration (matching NeoForge's
`ClientHooks.gatherTextureAtlases` and `RegisterTextureAtlasesEvent.register`).
There is no synthetic atlas or sprite count cap; only the GPU device's actual
`max_texture_dimension_2d` limit applies, with per-atlas isolation and
diagnostics so an overflowing or bad atlas never shrinks unrelated model
textures or discards other atlases or sprites. A registered atlas sprite
replaces a raw pack file at the same staged path with a warning (matching
vanilla's `TextureManager.register`); if two registered atlases stage the same
sprite path, the first stands and a warning names both.

## Item frame rendering

`render_item_in_frame(entity_id, pos, stack, canceled)` runs before a visible item frame draws its item or map, including glow and invisible frames. Set `canceled` to hide the item without hiding the frame.

## World render stages

`Hooks::render_level_stage` is the client counterpart of NeoForge 26.3's
`RenderLevelStageEvent`, covering all 9 stages as one hook with a `RenderLevelStage`:
- `AfterSky` (no active render pass, immediately after the logical sky pass's drawing work)
- `AfterOpaqueBlocks` (after solid and cutout chunk geometry, before opaque features)
- `AfterOpaqueFeatures` (after opaque entities, block entities and particles)
- When classic transparency is active (`!view.improved_transparency`):
  - `AfterTranslucentFeatures` (after translucent entities and block entities)
  - `AfterTranslucentBlocks` (after translucent chunk geometry)
  - `AfterTranslucentParticles` (after translucent particles)
  - `AfterWeather` (after rain and snow, before the world border)
- When improved transparency / OIT is active (`view.improved_transparency`):
  - `OitTranslucent(OitStage::DepthBounds)`
  - `OitTranslucent(OitStage::Transmittance)`
  - `OitTranslucent(OitStage::Accumulate)`
- `AfterLevel` (no active render pass, after the level finishes rendering)

Each frame asks `Hooks::submit_custom_geometry` and registered debug renderers first (with
`WorldDrawContext`, for `name_tag`, `draw_model_instances`, `draw_instance_group`, and custom
world primitives drawn at the end of the world pass). `Hooks::prepare_render_buffers` comes before
primitive, entity, prepared-instance, and glyph uploads so group updates apply to this frame.
`AfterSky` runs when `LevelRenderer.addSkyPass` adds a pass: the dimension has a skybox,
there is no lava or powder-snow camera fog, and no camera mob effect blocks the sky. It fires
once at the end of that pass's work even when no sky geometry is visible. Sunrise's sky setting
and dragon boss fog affect drawing, not admission. A classic pack owning either the sky disc
or the celestial work completes the logical sky after its world import, with the engine still
drawing the unclaimed portions. Each active pass-bearing stage fires at its own render-pass
boundary. `AfterLevel` follows the world and precedes the engine's first-person hand and other
overlays. A classic pack's own hand remains inside its world passes for the pack's depth
snapshots; `AfterLevel` runs before pack composition, not before that hand. These hooks never
fire on title or connect screens.

`event.context` is a `RenderLevelStageContext`: `AfterSky` and `AfterLevel` return
`StagePass::None` (`has_render_pass() == false`); standard pass stages return
`StagePass::World(&mut dyn WorldPassDrawContext)`, and `OitTranslucent(oit)` returns
`StagePass::Oit(oit, &mut dyn WorldOitPassDrawContext)`. Call `pass.draw(batch)` with a
`PreparedWorldDraw` returned by `event.context` in `prepare_render_buffers` for this view and
stage. Both pass kinds draw prepared engine lines and quads in call order. `RenderLevelStage`
and `OitStage` are process-local selectors, not `Wire` values.

Stages that do not run on the active route are not synthesized. When a classic shader pack owns
terrain, water, features, or weather and exposes a float-blendable world color buffer
(`colortex0`), `AfterOpaqueBlocks`, `AfterOpaqueFeatures`, `AfterTranslucentFeatures`,
`AfterTranslucentBlocks`, and `AfterWeather` fire at the pack's own passes (before or after
deferred according to the pack's feature schedule) against the pack's color and depth targets.
On an incompatible (integer or unblendable) `colortex0`, those pack-owned stages are omitted
rather than firing early on a mismatched target.

`PrepareRenderBuffersContext::lines`, `quads`, and `pipeline_quads` prepare batches for any
pass-bearing stage (including `OitTranslucent`). At `OitTranslucent`, a mod pipeline must be
registered with `.order_independent()` and a non-`Opaque` blend mode; others are refused with
`PipelineDrawError::Pipeline(RenderPipelineError::Invalid)` and logged once per mod and pipeline.

A panic disables that mod's listener for this hook for the session, with an error log and toast.
Draws already accepted are not rolled back.

## Rotated block states

- **`BlockPart::rotate(context, event)`** (`RotateEvent { state, pos, rotation }`, `event.rotated`): NeoForge's `IBlockExtension.rotate(state, level, pos, rotation)`, asked as a structure template places a block, where vanilla runs `state.mirror(mirror).rotate(rotation)`. `event.state` is already mirrored (mirroring has no NeoForge extension and stays the game's: vanilla's table, or the template's for a mod block), `event.pos` is where the block goes and `event.rotation` the template's turn. `event.rotated` starts at the game's own turned state; set it to another state of the same block, typically built with `context.state_with_property(event.state, "facing", "north")`, to place that instead; `context.state_property(event.state, "facing")` reads the value to turn, as vanilla's `state.getValue(FACING)`. A state of another block, or one this game does not have, is refused with a warning and the game's answer kept.
- **Where it is asked.** Every Sunrise path that places a template's blocks: world generation's structures (`StructureTemplate.placeInWorld`, the legacy pieces' `placeBlock` and their template filters), `/place template` and the structure block, the jigsaw blocks a jigsaw structure joins its pieces by, and `LevelRef::visit_structure_template`. A block entity's data on rotation is a separate step after this one.
- **What it reads.** Like every worldgen Part method, it runs on the worldgen worker threads and on the server thread, with the server's worldgen `context` (a `WorldgenHost`, no `Host` and nothing to write), so it is a pure function of its inputs: the same seed gives the same world.
- **Panics.** A panicking part is contained with a warning and the game's answer kept.
- **Cost.** With no part subscribed, one subscription check per placed block; with one, a table lookup and the state conversions per placed block.

## Dropped files

`screen_files_dropped(screen, paths)` is vanilla's `Screen.onFilesDrop`: it fires once for each drop on the window while a screen is open, with every dropped path in the order the operating system gave them. A path that is not UTF-8 is passed through as it is. The game reads no file, so open the paths yourself. The hook only observes: the game's own handling of the drop (a dropped resource pack) always runs, whatever a mod does.

## Screen hover moves

`screen_mouse_moved_pre(screen, delta, modifiers, canceled)` is vanilla's `Screen.mouseMoved`: it fires for every real nonzero pointer movement over the open screen, whether or not a button is held, before `screen_mouse_dragged_pre` of the same move. screen.mouse is the new GUI position; delta is movement in GUI pixels. Cancel keeps the new cursor position and skips the rest of the move (the drag hook and drag handlers). A controller's pointer and a screen holding the pointer grab fire nothing; with no listener nothing is built.

## Screen pointer grab

`ScreensClientHost::set_pointer_grab(grab)` takes or gives back the pointer for the calling mod's own open screen, and `ScreensClientHost::pointer_grabbed()` reports it; `screen_pointer_motion(screen, delta)` then reports the device's raw relative motion, unscaled, in the device's units, instead of the camera or `screen_mouse_moved_pre`. The grab ends when the screen closes by any route, when the window loses focus (it is not taken back on refocus), or when the mod unloads; Escape reaches the screen like any key. `set_pointer_grab` returns false and changes nothing for another mod's screen, with no screen open, and (for a grab) with an unfocused window.

## Transparent screen backgrounds and landing changes

`Hooks::screen_transparent_background` runs immediately before an in-game screen's
translucent gradient. Returning `Flow::Cancel` suppresses that gradient, not the
screen's contents. Its `batch` is a `DrawContext`: custom drawing survives
cancellation. `ClientHost::apply_screen_blur()` requests the normal menu blur
at most once per frame; `menu_blur_radius()` reads the configured radius.
`DrawContext::draw_menu_background` and `tiled_sprite` draw menu textures through
the same atlas. The whole-screen `screen_render_pre` remains a separate moment.

## Section visibility

`section_visibility` decides, per chunk section, what the terrain mesh of that section draws: the whole section
(`SectionVisibility::hide_terrain()`) or, per cell, the block model, the fluid and a six-bit face mask
(`SectionVisibility::keep` with `CellVisibility` records). It extends `block_render_visibility`, which hides a
state everywhere, for decisions that depend on where a block is: a cutaway above the player, an x-ray of one
area, a structure preview that hides the terrain under it. No NeoForge counterpart has been verified.

```rust,ignore
fn section_visibility(&self, _: &dyn ClientHost, event: &mut SectionVisibilityEvent<'_>)
    -> Option<SectionVisibility>
{
    if event.input.origin[1] >= 64 {
        return Some(SectionVisibility::hide_terrain());
    }
    // Hide every cell of the mod's marker block, wherever it is in the section.
    let mut cells = Vec::new();
    for y in 0..16 {
        for z in 0..16 {
            for x in 0..16 {
                if event.input.cell([x, y, z]) == Ok(SectionCell::State(self.marker)) {
                    cells.push(CellVisibility::hidden([x as u8, y as u8, z as u8]));
                }
            }
        }
    }
    SectionVisibility::keep(cells).ok()
}
```

The input is the 16-cubed section and a one-block border on all six sides, edges and corners included (5832
cells), with each cell's light, stored biome and deterministic model seed. A cell is the state the world holds,
before any visibility answer: an ore a `block_render_visibility` part hides still reads as the ore. A neighbouring
column not loaded reads `Missing`: not air, but neither occluding nor matching; the section is asked again when
the column arrives or unloads. `SectionVisibilityInput::biome` returns the stored `BiomeKey` before biome zoom
and blending or `None` for a missing column or an unresolved slot. It never returns the rendering default in
place of a missing registered key. Reads beyond the border are refused.

Answers only take away. A face mask removes faces after normal culling and never forces a culled face to show. A
cell with neither model nor fluid no longer occludes, so its neighbours' faces toward it draw (also across a
section border); a cell with faces removed occludes only through the faces it keeps, and the section's
see-through graph is worked out again from what still occludes, erring toward drawing what is behind. Unculled
quads go with the model. `hide_terrain()` drops the section's block models, fluids and section geometry quads and
hides nothing behind it; block entities that draw themselves still draw. All mods' answers are combined by
intersection. Fluid heights and flow still use the captured states; visibility only affects which faces draw.

`keep` refuses the first record beyond one section's worth. A returned answer is checked whole: a cell outside
0 to 15, a face mask above 63 or a cell named twice refuses the answer with one log line
(`E_SECTION_VISIBILITY_...`), and the section draws as if the mod had not answered. A listener that panics is switched off
for this hook for the session, alone: the other listeners are asked as before, and every section
is asked again without the one that failed, so no mask it gave earlier stays. The decision is asked on the client world thread, so
it may read the `ClientHost`; it changes when the section meshes again, which a mod asks for with
`ClientHost::refresh_render_visibility`, never by polling.

## Block sound type

- **`BlockPart::sound_type(context, event)`** edits the contextual `BlockSoundType` for the attached block, as in NeoForge's `IBlockExtension.getSoundType`. Use `SoundEventKey::parse` at load time and retain the keys on the part. A bare name or `:name` uses `minecraft`, and a syntactically valid event need not already be loaded. A mod's event can be provided by `assets/<namespace>/sounds.json`; an absent event plays nothing rather than reverting to vanilla. Playback reads the selected slot: step, fall, place, break, or the hit sound for a sniffer digging. A solid bucket chooses its own placement event but uses the returned sound type's levels. A horse chooses its hoof sound from the returned type's identity, not its event keys. There is no block-specific volume or pitch admission check: the usual action factors and the sound engine still apply. The hook runs on the ordinary and combination step, horse step (asked at the position below even when a snow layer answers), mob fall, block item placement, client break and sniffer dig paths; it is not the mining hit or sculk sound hook.

### Choosing a block placement state

`BlockPart::state_for_placement` is vanilla's `Block.getStateForPlacement` for
player and dispenser placement, including client prediction for a player.
`event.context.source` distinguishes a player placement
(`PlacementSource::Player { yaw, pitch, sneaking }`) from a non-player
`DirectionalPlaceContext` (`PlacementSource::Directional { direction }`, such as
a dispenser placing a shulker box). A directional context's `replace_clicked` is
read from the clicked block. Where vanilla's directional context throws, as for a
pointed dripstone's vertical look, ordinary placement refuses: answer for your block
to place it anyway.
Code with a `ServerHost` can also call `LevelRef::state_for_placement` to ask
for a candidate without placing anything. The Part still receives only its
`LevelHost` through `LevelSide`; the context carries an item key, not a stack.
Answer only for your own blocks: `Some(Some(state))` selects a registered
candidate, `Some(None)` refuses placement without spending the stack, and
`None` delegates to the next Part or ordinary placement. Use
`native_state_with_property(state, key, value)` to set a property, including
one your block declares.

A candidate can belong to another block. The selected block does not run its
placement-state method again. If actually placed, it still gets the primary
cell's survival and obstruction checks as applicable, and a game-master
block requires creative operator permission. If the selected block spans cells, check its footprint
before returning it: `setPlacedBy` does not recheck its companion cell.

`stateful_block_entity` declares static factory ownership; pure `native_block_entity_from_state` receives the borrowed actual block/property map. An empty result creates no new entity and never falls through to API15. LevelChunk mutation creates inline after onPlace; existing same-block entities survive state property changes.

## Custom world geometry

`Hooks::submit_custom_geometry` runs on the client after particles, before rendering. Its
`WorldRenderView` lends the frame's `Camera`, world partial tick, game time, whether vanilla's
improved transparency (OIT) is active, and the visible `RenderableSection` slice;
`WorldDrawContext::lines` and `quads` accept unlit world-space ARGB geometry with depth testing or
through walls. `lines` and `quads` fail atomically for invalid inputs; `pipeline_quads` checks
`RenderPipelineRef` validity and staleness (`PipelineDrawError::Pipeline`) and skips any quad with
non-finite coordinates or UVs individually without rejecting valid quads in the same slice.
Backing storage grows dynamically and output clears every view. Submit extracted state, without
world writes or resource loading.

`WorldDrawContext::draw_model_instances` draws one batch of a resolved `ModelRef`. Each
`ModelInstance` carries an f64 world origin, a row-major 3×4 local transform, an `0xAARRGGBB`
tint (the same convention as `WorldQuad::color`) and `LightCoords` block and sky light, each
0 to 15, in vanilla's own convention. The engine converts tint and light to the frame format;
mods never assume that format or supply a packed light word. Copies outside the frustum are
culled first, and any individual visible copy with non-finite coordinates/transforms or light
outside `0..=15` is skipped without rolling back earlier valid copies. Backing storage grows
dynamically, with copies past GPU buffer room falling back to CPU baking.
`ModelInstance::color` (`0xAARRGGBB`, white by default) multiplies every face's colour; with alpha below `0xFF`, or for a model with a translucent face, the copy is drawn in the sorted translucent material (vanilla's `translucent_moving_block`), its quads far to near. `ModelInstance::hidden_faces` leaves out faces whose `cull` side is set.
Shader packs and any route that cannot
take GPU instances bake the same prepared geometry on the CPU instead of dropping the model; a
`shaders/` pack shades those copies as block entities, as it shades `BlockEntityDrawContext::model`.

`WorldDrawContext::with_pipeline_modifier` pushes a registered `PipelineModifierRef` for the
duration of its callback (nesting outermost-wins, matching `PipelineModifierStack.apply`),
applying to `lines`, `quads`, `pipeline_quads`, and `draw_model_instances` (not `name_tag`). Only the
modifier's `depth` applies to the built-in `lines`, `quads` and `draw_model_instances`; the whole
modifier applies to a registered pipeline drawn with `pipeline_quads`.

`WorldDrawContext::name_tag(at, &text, NameTagOptions::new(see_through, background, light))` draws
a label as vanilla draws a name tag (`submitNameTag`): 0.5 above `at`, facing the camera, 0.025
blocks a font pixel, so it shrinks with distance (scale by distance yourself for a constant size),
centred, in the text's styles. `background` puts the black backdrop behind it at the player's
text background opacity; `light` is the light at the label, sky light in the high four bits and
block light in the low four. Without `see_through` it is drawn once, depth tested and translucent:
the faded text on the backdrop, in `light` as given, hidden behind what is in front. With
`see_through` it is drawn twice: opaque in the world with no backdrop, its sky and block light each
raised to at least 2, and faded on the backdrop over everything, walls included, unlit. The
backdrop's alpha is 255 times the text background opacity (0.25 when it is set for chat only), the
faded text's 255 times the larger of (opacity + 0.75) / 2 and 0.5. Labels draw with the world's
name tags whatever the stage, sorted far to near. A position that is not
finite (`DrawError::NonFinite`) is refused whole: nothing is drawn. Only visible text and styles
are used, not click, hover or opaque metadata. `see_through` can show a
label through any wall, which vanilla does only for the names its own renderers choose; the call
sends nothing and reads nothing the client does not already hold.

NeoForge's TradeWithVillagerEvent at MerchantResultSlot.onTake. This non-cancellable collect receives the post-take offer for each trade, with the acting player swapped in. Accepted offer edits affect the next iteration and reach the client's menu snapshot on commit; observing alone keeps the usual offer-refresh timing. MerchantRef edits and menu replacements that conflict with the active click are refused.

## World views

A client mod can have the world drawn again from cameras of its own, into targets of its own: a camera monitor, a
second camera's preview beside the player's view, stills for a film strip. Once as each frame is built (not on the
title screen's panorama), before its entities are, `world_views` asks for them, so keep it cheap: push a
`WorldView::new(id, camera, yaw, pitch, fov_degrees, size)` (`with_roll(degrees)` turns it about the view
direction) to the `WorldViews` it hands you for each one wanted this frame. `push` returns `Ok(())` for a view
taken, or a `WorldViewRefusal` naming the view (`id`) and why (`reason`). For a view through a portal or a
mirror:
- `with_clip_plane([x, y, z, d])` draws only what lies in front of that world-space plane (`x * p.x + y * p.y +
  z * p.z + d >= 0`): terrain, entities and block entities behind it are left out, as a wall behind a portal's far
  side is. It becomes the view's near plane, so it needs no device feature, and the camera must be behind it.
  Entities and terrain are cut at the same plane; past it, an entity wins the depth test against terrain up to
  1/1024 of its distance in front of it (about 1 block at 1024 blocks).
- `with_mirror()` mirrors the picture left to right, as a mirror shows it: give the camera reflected through the
  mirror. A mirrored picture turns every triangle's winding round, so the view draws with face culling flipped to
  make up for it: the faces culled are the ones the unmirrored view culls, and back faces stay hidden.
- `with_parent(id)` says the view is seen in the view `id`, a portal's far side seen through another portal.
  Push the parent first, so a cycle cannot be asked for. A view with no parent is at depth 1, one seen in it at
  2, up to `MAX_WORLD_VIEW_DEPTH` (5, Immersive Portals' default `maxPortalLayer`). Views draw deepest first and
  in the order pushed within a depth, so a child's `world_view_drawn` runs before its parent is drawn: in the
  parent's `world_view_drawn`, the child's `color` and `depth` hold this frame's picture, to draw into the
  parent's target, as a view with no parent is drawn into the frame in `world_drawn`. Nothing more is bound: the
  frame encoder's order is the guarantee. Otherwise a child is drawn from its own camera like any view.

Up to `MAX_WORLD_VIEWS` (8) are taken, each at most `MAX_WORLD_VIEW_SIDE` (4096) pixels a side and the device's
largest texture side, and all together at most `WORLD_VIEW_PIXEL_BUDGET` pixels (4 x 4096 x 4096, as much as four
views took before views had parents). Nothing is left out without a word: `push` refuses, by its
`WorldViewRefusalReason`, a camera, yaw, pitch or roll not finite (`NotFinite`), a field of view outside 1 to 170
(`FieldOfView`), a side of 0 or over 4096 (`Side`), a side over the device's (`DeviceLimit`), a clip plane out of
range (`ClipPlane`: a normal `[x, y, z]` of zero, a value or its scaled or camera-relative form not finite, as f32
too, or the camera not behind it), an `id` already pushed this frame (`DuplicateId`), a parent not pushed before
it, refused, or the view itself (`MissingParent`, so a refused view's children are refused too), depth 6
(`TooDeep`; never clamped), a ninth view (`TooMany`) and a view past the pixel budget (`PixelBudget`, with its
pixels and those left) and a view of a level other than the player's own (`Level`: `with_level` names a level
by a handle; `None` and the player's own level draw, an open viewed level, a stale handle, a closed or never open
level are refused, as viewed levels are not drawn yet). Views taken for a frame that turns out to be a title or loading screen's are refused
afterwards (`TitleScreen`). Each refusal is logged once: the first time a view of that `id` is refused for that
reason.

Each view is drawn after the frame's world with the terrain it sees (solid and translucent), the frame's sky colour,
fog and entities (block entities too), from its own camera and with its own frustum; then `world_view_drawn` hands
it back as a `WorldViewDrawn`: its `id`, `parent` and `nesting_depth` (1 for a view with no parent), `color` (`color_format`, the world's light as `WorldDrawn::color` holds it,
before the tone curve, without the hand, the HUD or the frame's post passes), `depth` (Depth32Float, reversed),
`view_proj` (the world relative to the view's `camera`, to its clip space, as the picture and `depth` were drawn:
the clip plane as its near plane, x flipped for a mirror), and the device, queue and the frame's encoder. Draw your
own geometry into it there, sample it (both textures can be bound) or copy it out: `copy_texture_to_buffer` into a
buffer of your own and map that after the frame, for a still. The target stays the view's while each frame asks for
it, so a texture view of it can be kept and shown, and is drawn over every frame; one no frame asks for any more is
freed. Views draw before `world_drawn`, which can put them into the frame's picture.

A view sees what lies in its own frustum within the render distance of its camera: it lists the loaded sections its
frustum takes in itself (the clip plane is its near plane), without the caves, occlusion or faces turned away, and
its translucent sections go back to front from its camera (within one, the quads keep the player camera's order).
The player's camera keeps all of its culling, GPU culling too, so the frame's own picture and draws stay as they
were; each view costs one more pass over the loaded columns and its own draws. The entities and block entities the
frame builds are those the player's camera or a view sees. A view draws no sun, moon, stars, clouds, particles,
weather, block outline or hand, and none of a shader pack's passes. A frame without views costs nothing more: while
no mod listens to `world_views` it is not called.

`python3 tools/run_world_views.py <dir>` (throwaway homes) draws views through the renderer offscreen and checks
that the frame's picture is unchanged, with eight views too, that mirrored views cull the faces unmirrored ones do,
that a clip plane leaves out what is behind it and that a child's picture is this frame's in its parent's
`world_view_drawn`; `--bench` times a frame with no view, one and four.
