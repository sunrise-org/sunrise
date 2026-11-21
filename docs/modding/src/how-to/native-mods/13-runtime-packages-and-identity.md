# Runtime packages and identity

Move the executable, prepared resources and `sunrise-native-mods/` package directory
together when you distribute a native build. Keep each copied project's flat manifest,
crate, assets and data.

The launcher generates the game's embedded native inventory from your selected
`sunrise-mod.toml` manifests: mod id, version and side. Cargo package metadata and
hook factories do not supply identity.

| Rule | Behaviour |
| --- | --- |
| Package location | The game reads each compiled mod's package from `<executable directory>/sunrise-native-mods/<id>/`, not the working directory or a build record. |
| Identity | The package's id, version and side must match the embedded inventory. |
| Startup validation | Before installing hooks or starting a world, the game refuses missing, unreadable, content-only or identity-mismatched packages. The error begins `Compiled native mods cannot be loaded:` and lists the affected mods and paths. Restore the packages exported with that executable or rebuild it. |
| Content-only mods | These use the working directory, player toggles and explicit `--dev-mod` paths. A content package cannot shadow a compiled mod's id. |
| Display metadata | The mod list and crash details include code-only mods from the embedded inventory. Display names and descriptions come from validated manifests. |

Declare Controls bindings through `Registrar::register_key_mapping`, and read one on the client with
`ClientHost::is_down(mod_id, id)` (held) and `ClientHost::consume_click(mod_id, id)` (takes one press off its
count, as `KeyMapping.consumeClick`). Each press, auto-repeat and mouse press counts while no screen is open and
the mapping's conflict context and modifiers are active; a screen opening drops the count. A key or mouse
button sent through the control socket counts as the window's does, and a key fires `input_key` once per press,
repeat and release, as the window's.

A mod's own screen can change its bindings as the player does in Controls:
`ClientHost::set_mod_key(mod_id, id, key, modifiers)` binds the mapping to a key or mouse button name
(`key.keyboard.x`, `key.mouse.left`, `key.keyboard.unknown` for none) with shift (1), control (2) and alt (4),
and `ClientHost::reset_mod_key(mod_id, id)` gives it its declared default back. The mapping answers to the new
key from the next input and the binding is saved to `sunrise-mods.txt`; `ClientHost::mod_key` reads it. A
native mod changes only its own mappings. Each refusal is a `ModKeyRefusal` (`NotRegistered`, `NotOwner`,
`UnknownKey`, `UnknownModifiers`, `Unavailable`) and changes nothing.

`native-mods.json` is provenance and rebuild status, not game identity, authentication, announcements
or startup input. The launcher retains it for build/status displays. The separate `sunrise-server`
supervisor may report the adjacent record as a provenance snapshot, not a query of the running engine.
Its SHA-256 values and revision describe the built binary's inputs, not the running mod set.
The launcher prepares jar data from canonical manifests as non-authoritative candidates; the engine
validates identity against its embedded inventory. Original-jar pack order and
whole-package refusal rules are in [Original Java jar layers](../../reference/manifest.md#original-java-jar-layers).

## Resolve an entity by UUID

In a server hook, wrap a saved `u128` in `EntityUuid` and pass it to the host.
The lookup returns `None` if the entity is unloaded or belongs to a different
level than the hook. Try again from a hook running in the entity's level.

```rust,ignore
use mod_api::{EntityRef, EntityUuid, ServerHost};

fn find_saved_entity(host: &dyn ServerHost, saved_uuid: u128) -> Option<EntityRef> {
    host.entity_by_uuid(EntityUuid(saved_uuid))
}

fn entity_identity(host: &mut dyn ServerHost, entity: EntityRef) -> Option<EntityUuid> {
    entity.uuid(host).map(EntityUuid)
}
```

`EntityRef::uuid` returns the raw `u128`. Use `EntityUuid(raw_uuid)` to pass
that value back into the lookup.
