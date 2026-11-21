# Client particles and movement queries

## Client particles

`ClientHost::add_particle` submits the API's `ParticleOptions`, a world position and
velocity to the ordinary client particle queue. The client must have a loaded player
and world. Non-finite numeric inputs are refused. A successful submission does not
promise a visible particle: provider availability, sprite sets, distance and the
player's particle settings apply. This method is not available on `ServerHost`.

Particle options carry a `ParticleKey`, not an engine particle enum or a provider name.
Request the key with `Registrar::particle_key` during registration and retain it. The game resolves
provider identities once when content loads. `ParticleOptions::simple(key)` handles both vanilla
and custom sprite providers; custom providers require an `assets/<namespace>/particles/<path>.json`
sprite set. All option constructors take the resolved key, including `dust`, `item`, `trail`,
`vibration`, `shriek`, `sculk_charge` and `dust_transition`. Use the corresponding vanilla key
for these parameter families. Sending an option performs no string parsing or registry scan.

For stack-derived breaking or eating particles, call
`ClientHost::item_particle_options(&stack)` and pass the returned options to
`add_particle`. This read resolves the ground model with seed 0, preserving all
layer particle sprites and resource-pack and stack-component overrides. It returns
`None` for invalid stacks or unavailable resources/provider. It is available in
Setup, Reloading and World. Resolve again after resource reloads.
It allocates a sprite list only when called; it adds no per-tick work.

### Tinting an item crumb

The item crumbs (`item` and its `item_*` variants) multiply their red, green and blue by
`ParticleOptions::color` when they spawn and ignore its alpha; the default white changes
nothing, and `item_particle_options` results and server `item_stack` sends stay untinted.
A mod sets it with `with_color`, and a block-atlas sprite such as a fluid's still sprite is a
valid layer, so a fluid crumb is the sprite tinted by the fluid:

```rust,ignore
let crumb = ParticleOptions::item(item_key, vec!["minecraft:block/water_still".into()])
    .with_color([0.8 * r, 0.8 * g, 0.8 * b, 1.0]);
host.add_particle(&crumb, at, velocity);
```

An item crumb's colour must be finite with every component in `0.0..=1.0`; `add_particle`
returns `false` otherwise and nothing is clamped. The crumb keeps its small random spread of
velocity and the level's light.

### Particle providers

A particle provider (`Hooks::register_particle_providers`) moves a mod's own particle type with code. The type is
named by its id (`mymod:spark`), its sprite set and spawn values are the declarative
`assets/<namespace>/particles/<path>.json`, and it is spawned with `ParticleOptions::simple` on the key
`Registrar::particle_key` issued for the id.

The game calls `ParticleProvider::tick(&self, particles: &mut ParticleBatch, level: &dyn ParticleLevelView)`
once per type per tick, after it has aged the particles, with every live particle of the type; it is not called
when none is alive, and a registered provider with no particle alive is not visited. Batch iteration
(`for particle in particles`) preserves length and order. Set `removed` to expire a particle. If a callback
returns a different length, the engine expires the batch's particles.

Each `ModParticle` carries the particle's pose and look: `position`, `previous`, `velocity`, `size`, `rgba`, `frame`,
`state` (four numbers kept from tick to tick), a `facing` and a `roll`. `facing` is `Camera` (vanilla's
`LOOKAT_XYZ`), `AroundY` (`LOOKAT_Y`, which uses the camera's rotation about Y alone) or `Fixed([x, y, z, w])`, a
world-fixed unit quaternion that turns the quad's default plane (corners in XY, facing +Z); a quad lying flat facing up is
`[-sin(pi/4), 0, 0, cos(pi/4)]`. `roll` turns the quad about its own normal in radians, interpolated from the previous
tick's value, and nothing is drawn rotated while it is zero. A number that is not finite, a position past the world's limit,
a quaternion of no length and a roll beyond half of `f32::MAX` expire the particle.

`ParticleLevelView` reads the loaded level around the particles, read-only: `block_state(pos)` and
`brightness(layer, pos)`. A missing chunk reads as air. Light follows the render world's stored light; a dimension
without skylight always reads zero on the sky layer. Currently, an absent chunk or a position above the build height
reads full sky where present and no block light; below a loaded column both layers read zero. The client light storage
for padding sections and unloaded borders is tracked in [#2761](https://github.com/sunrise-org/sunrise/issues/2761).

Dispatch reuses its batch storage once it has enough capacity.

## Client fireworks

`ClientHost::create_fireworks` draws a firework's explosions from a `FireworksRequest`: the
position, the rocket's velocity carried by the sparks, a list of `FireworkExplosion`s (shape,
colours, fade colours, trail, twinkle) and `audible`. It uses the starter a bursting rocket
uses, so the explosions follow each other every other tick as a rocket's do. With `audible`
false the sparks and flash draw but the blast and twinkle sounds don't play, as for a
silent rocket. Nothing reaches the server or other players: a mod's server half tells each
client half to draw it. The request is refused on a server, without a loaded client player
and world, for non-finite numbers and for more than 256 explosions.

## Bot movement and block collisions

`PlayerRef::simulate_move` predicts one server player's movement tick from
`PlayerInput`, using the game's movement implementation without replacing the
live player's body. It returns `SimulatedMove` with position, velocity, ground,
horizontal-collision and sprinting results. It refuses stale or inaccessible players,
dead players, spectators, riders and invalid numeric input. A supplied velocity is
input to the prediction, not a write to the player.

`LevelRef::collision_boxes` reads block collision boxes within a finite, ordered
axis-aligned box. It excludes entity and border collisions. A refused level or a
query covering more than 1,000,000 cells returns `None`; a valid query with no
collisions returns `Some` containing an empty vector. Keep queries bounded rather
than scanning a whole level each tick.

`LevelRef::block_shape(host, state, pos, kind)` reads an explicit `BlockState`'s
outline or collision boxes with an empty context at a position in that level.
The supplied state can differ from the world's block, without changing it;
level-dependent shape inputs come from the actual level. To query the
stored block, first read `level.block_state(host, pos)` and pass that state.
This server read returns block-local boxes, or `None` for a
stale handle, closed level or client. Parked levels are supported.
