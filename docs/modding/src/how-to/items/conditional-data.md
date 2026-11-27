# Conditional data

Add a `neoforge:conditions` array to control whether a server JSON file loads.
Supported readers are
recipes, loot tables, item modifiers, predicates, enchantments, jukebox songs,
instruments, damage types, advancements, other supported synchronized vanilla registries, and
[mod-declared datapack registries](../content-mods.md), even if their entries are never sent to clients.
Every condition must hold; a false condition omits the file. Mods can also
[register custom conditions](#registering-custom-conditions) for this same evaluator.
Register one handler per id using the read-only `RegistrationContext`. A handler
reads the condition's JSON from a borrowed raw value and validates its own
arguments. Invalid mod input must return `ConditionError`, never panic.

Ordinary JSON resources select the winning pack file first, then evaluate its
conditions before decoding the data. If the winner is omitted, a lower pack's
version does not return. Tag files and data-map files ignore a top-level
`neoforge:conditions` key. A data-map value with false conditions is omitted
before merging. In a loot table, only the pools
in the `pools` list carry their own `neoforge:conditions`: a false pool is
dropped and a passing pool loses the key before decoding. Anywhere else in a
recipe, predicate, item modifier, or a pool's entries and functions, a nested
key is ordinary data and is never evaluated, even when it names an unknown
condition. A damage
registry is frozen at world open; reloading reevaluates its tags, not its type
definitions. Generated item default components retain their prototype
lifetime and are not conditional data files.

`neoforge:mod_loaded` asks the owning resource generation's captured enabled
server-mod IDs, including active code-only mods, plus
`minecraft` always. Client-only, disabled and refused packages are absent;
resource namespaces are not mod IDs. Other supported forms are `neoforge:always`
(true), `neoforge:never` (false), `neoforge:not` (`value`), `neoforge:and` / `neoforge:or`
(`values`), `neoforge:feature_flags_enabled`, `neoforge:registered` and
`neoforge:tag_empty`. `feature_flags_enabled` takes `flags`, a nonempty array of feature IDs that must
all be enabled. `registered` and `tag_empty` take a `registry` (default `minecraft:item`) and a `value` or
`tag`, respectively. They look up an admitted entry or test whether a tag has any resolved members.
Within a supplied view, an absent registry has no entries or tags: `registered` is false and `tag_empty`
is true, with no diagnostic. That is different from a loader that cannot supply the context at all:
a check that reaches an unavailable context reports a load error.

## Fabric's `fabric:load_conditions`

A Java jar's data files (the passive pack layers of [Java jars](../content-mods.md#java-jars-as-external-assets))
may be guarded by Fabric API's key instead. The same readers evaluate it, at the top of a file only: tags
ignore it, and a loot pool's or data-map value's own key is NeoForge's alone. Its value is one condition
object, or a list that must all hold, each named by `condition`:

- `fabric:true`, `fabric:false`; `fabric:not` (`value`); `fabric:and` / `fabric:or` (`values`).
- `fabric:all_mods_loaded` / `fabric:any_mods_loaded` (`values`, mod IDs read as `neoforge:mod_loaded` reads them).
- `fabric:registry_contains` and `fabric:tags_populated`: a `registry` (`minecraft:item` when it is missing or
  not a valid ID) and `values`. Every entry must be registered, read as `neoforge:registered` reads it, or
  every tag bound by this load. A bound tag counts even when it holds nothing, for example one whose entries
  are all optional and absent; `neoforge:tag_empty` calls that tag empty.
- `fabric:features_enabled` (`features`): every flag enabled; an unknown flag fails the test. Sunrise answers
  from the world's enabled flags, in worldgen files too. Fabric reads a flag set captured at its last data
  reload instead: none during the first world's registry load (the test fails there), an older one later.
- Any other ID goes to the handler registered under it, which reads the object without `condition`.

Sunrise loads a file that has both keys only when both hold. That is Sunrise's own rule: NeoForge reads
only its key and Fabric only its own, so neither loader defines the pair. Unlike NeoForge's key, a Fabric
key that does not decode (an unknown ID or a malformed node) is logged once per load candidate and the
file loads, as Fabric API does. Fabric's `fabric:overlays` pack metadata is not read.

One known difference: while vanilla loads the dynamic registries together (worldgen, damage types), each
co-loaded registry's tag lookup is a registration getter in which every tag is present, so on Fabric a
`fabric:tags_populated` naming a tag of one of those registries always holds there. Sunrise answers from the
tags that load has bound, so such a tag that no file defines does not count.

## Registry conditions

A registry file sees the current candidate's static holders and resolved static tags, including tags
staged for the load but not yet published. A mod-declared registry also sees the available frozen world
registries and completed dependencies in the candidate. Only `registered` and `tag_empty` tests that
actually run can make a registry wait; short-circuited branches and strings in custom condition arguments
are not dependencies. It finishes each
dependency's entries and tags before testing its consumers. The file's own registry and registries in the
same dependency cycle are still pending: their entries do not satisfy `registered` and their tags count
as empty. A consumer outside the cycle sees the whole batch after it binds. A rejected file never makes
its ID visible, and no condition reads the previous generation to fill in missing pending entries.

Registry entry codecs use the vanilla feature flag set. Downstream readers such as recipes use the
world's enabled flags and the completed candidate registry and tag view. If an entry condition needs
an experimental world flag or a tag in its own registry, put that test in a downstream consumer instead.
An unknown feature flag name is a decode error, distinct from a valid flag that is not enabled.
At world open, a biome's `neoforge:value` (or its fields beside `neoforge:conditions`) must decode as a
biome after its conditions pass. Its effects and positional attributes must use their value formats, and
its carver and placed-feature holder sets resolve against the world's generation registries. A false
condition omits the biome without decoding it; a passing condition on an invalid biome fails world open.
Only admitted biomes count for `registered` and bind as tag members. The server keeps the generation
settings and local attributes; joining clients receive only the admitted climate, effects and syncable
attributes, without evaluating conditions again.

## Errors by loader

The loader decodes the entire conditions array before testing any of it,
including all children of `not`, `and` and `or`. If the key is present, its
value must be an array. An unknown ID or malformed node is a decode error
even if a sibling would decide the result. A namespace or installed mod does
not make an unregistered condition ID valid. Once decoded, the top-level list
and `and` stop at the first false result; `or` stops at the first true result.
Handlers in an untested branch do not run.

| Loader | Condition decode or handler error |
| --- | --- |
| World-open registries that read conditions, including frozen vanilla definitions | Report the failing entries and refuse the world open. |
| Mod-declared datapack registries | Report the registry, entry, pack and file. Refuse world open or keep the previous generation on reload. |
| Reloadable registries (loot tables and their pools, predicates, item modifiers, advancements, recipes) | Report the failing entries and keep the previous data generation. At world open, refuse the open. |
| Simple JSON reload listeners, including global loot modifiers | Log the condition ID and file path and skip only that file. |
| Data maps | Log the condition ID, file path, map type and registry. Skip the entire file, so its values, removals and `replace` flag have no effect. |

A file without the conditions key, or with an empty array, loads normally. A handler error is not a false
condition. A handler panic is logged and queues an error toast naming the mod; only that handler is disabled
for the session. Returned errors and phase refusals follow the loader's error policy without disabling the
handler. Successful reloads publish changed enabled IDs with the new data generation; retained readers keep
their old snapshot.

## Registering custom conditions

Implement `LoadCondition` as a handler that reads the condition's JSON, then
register it from `Hooks::register_load_conditions` using its `RegistrationContext` and
`event.conditions.register(Identifier::parse("testmod:flag").unwrap(), Flag)`.
Registration happens once per mod load, with one handler per id and a read-only
`RegistrationContext`. Any mod may register an unused ID in any namespace:
mod B can register `a:x` while mod A registers `a:y`. Built-in IDs such as
`neoforge:always`, `neoforge:never` and `neoforge:mod_loaded` cannot be replaced.
A duplicate ID names both mods in the error. A completed listener's registration error
refuses admission even if the listener ignores it. A
[failed registration callback](../../concepts/load-phases.md#registration-and-setup) publishes
neither handlers nor preparation errors; later listeners can still register those IDs.

```rust,ignore
struct Flag;
impl mod_api::LoadCondition for Flag {
    fn test(&self, args: &mod_api::serde_json::Value)
        -> Result<bool, mod_api::ConditionError>
    {
        args["value"].as_bool().ok_or_else(|| mod_api::ConditionError {
            message: "value must be a boolean".into(),
        })
    }
}
```

A recipe can use the registered name alongside the built-in conditions:

```json
{
  "type": "minecraft:crafting_shapeless",
  "ingredients": ["minecraft:flint"],
  "result": {"id": "minecraft:stick"},
  "neoforge:conditions": [{"type": "testmod:flag", "value": true}]
}
```

The handler receives a borrowed raw JSON value: the condition's object without
`type`. It reads and validates its own arguments, retaining JSON booleans,
numbers, strings, lists and objects. Invalid mod input must return
`ConditionError`; handlers must not panic while reading it.
Startup data loaders and the server's candidate data reload both use registered
handlers before entries become visible. Results are retained for each entry and
condition payload in that load, so repeated readers do not call the handler
again. A new load reevaluates the entry. Handlers must not mutate live data.
A handler panic becomes a named load error ("handler panicked") and follows the
[loader error rules](#errors-by-loader) above.

The public [NeoForge data load condition documentation](https://docs.neoforged.net/docs/resources/server/conditions/)
describes `ICondition` and the top-level `neoforge:conditions` array.
