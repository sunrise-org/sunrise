# Recipe serializers and recipe types

A recipe file's `type` is a **serializer ID**, not its recipe type. These are two
independent registries. All crafting serializers produce `minecraft:crafting`,
which the engine registers. Each mod registers its own serializer IDs and any new
recipe type IDs in namespaces it owns. Duplicate serializer IDs and duplicate
recipe type IDs refuse startup, naming the ID.

A `type` written without a namespace (`crafting_shaped`, or `:crafting_shaped`)
means `minecraft`, as everywhere an identifier is read; it is never the recipe
file's own namespace. The id is decoded once, when the file is read, so filters,
brewing and clients all see `minecraft:crafting_shaped`. An id that is not a
valid identifier (`Crafting_shaped`), a missing or non-string `type` and a recipe
that is not a JSON object are each refused by name; a valid id no mod registered
is refused separately as an unregistered serializer. Every refusal is listed in
the one report of recipes that failed to load.

Register crafting code in `Hooks::register`, which receives `RegistrationContext`:

```rust,ignore
r.register_crafting_recipe_serializer("my_mod:diagonal", Diagonal);
```

Implement `CraftingRecipeKind`. Its mod-owned `Value` must supply
`Some(CraftingRecipeCodec::wire())` from `codec` and implement `Wire`; a missing
codec refuses startup. `decode` reads the recipe JSON
object with `type` removed. `write` writes an equivalent JSON object for Sunrise
client synchronization. Known serializers that refuse malformed data skip that
recipe alone, with the file and serializer in the diagnostic.

`matches` and `assemble` take `&dyn LoopContext` before the decoded value and an
owned `CraftingGrid`: width, height and row-major optional stacks, including empty
cells. `remaining_items` instead takes `&dyn CraftingRemainders`, which extends
`LoopContext` with `default_remainder(cell: u32) -> Option<ItemStack>`.
Callbacks must be pure and deterministic. The engine retains the owning
handler with its value. Counts must be 1 through 99; encoded values are limited
to 1 MiB. Invalid result keys/counts refuse the result. Invalid remainder keys,
counts or vector lengths use default item remainders.

## Frozen tag reads and default remainders

All three callbacks read the same owning resource generation's authoritative item
tags with `is_in_tag(item, tag) -> Option<bool>`, `item_count(tag) -> Option<usize>`
and `item_at(tag, index) -> Option<ItemKey>`. Count and index preserve the server's
member order without allocating an iterator or member list. Reads allocate
nothing and do no file I/O. `None` means tags or item keys are not ready (including
client tags not yet synced); a matcher should then not match. Once loaded, an
unknown tag returns `Some(false)` membership and `Some(0)` members; an out-of-range
index returns `None`. Reload/sync publishes a new snapshot, not a mutable live tag
service, so one callback cannot mix generations.

`RegistrationContext` does **not** resolve `TagKey`s. Resolve the name with
`r.tag_key("minecraft:logs")` on the `Registrar` in `Hooks::register` and build the
registered kind with the key it returns. The key is a process-lifetime identity that
does not need the tag to have members yet, and it stays valid across reloads; only
the generation's membership changes. A hook that supplies a `Host` (for example
`tags_updated`) can resolve the same key with `host.tags().key(&id)`.

`default_remainder(cell)` reads that cell's original, **pre-take** stack and runs
the caller's existing default rule, including `ItemPart::crafting_remainder` on
the server. Cell indices are zero-based row-major indices within the actual
rectangular grid, not the engine's padded square. Empty and out-of-range cells
return `None`. The result is an owned, component-bearing stack: unlike tag reads,
this operation may allocate. It does not call the recipe kind recursively.

A kind can mix defaults, custom stacks and no remainder in one result:

```rust,ignore
let mut result = vec![None; grid.cells.len()];
result[0] = context.default_remainder(0);
result[1] = Some(custom_stack);
Some(result)
```

Returning `None` for the whole vector still requests the existing all-default
behavior. Clients retain their data-only default prediction; the server remains
authoritative for ItemPart behavior.

The server rechecks the current grid before a take. It asks remainders once per
take, including each shift-click iteration, before consuming inputs. A remainder
returns to an empty cell, merges with the same item and components, otherwise
enters inventory, then drops through `item_toss` if it cannot fit. A crafter ejects
remainders through its result path. Clients predict default remainders for mod
kinds and accept the server's correction. Mod crafting has no recipe-book display;
`/recipe give` and `take` track hidden known IDs. Query crafting through
`LevelRef::recipe_for` with `minecraft:crafting`, regardless of serializer ID.

For a machine whose recipes the mod reads itself, declare a recipe type and an
independent serializer:

```rust,ignore
r.register_recipe_type("my_mod:grinding");
r.register_raw_recipe_serializer("my_mod:grinder_json", "my_mod:grinding");
```

`{"type":"my_mod:grinder_json","duration":42}` is then published through
`recipes_of_type("my_mod:grinding")`, with its source JSON intact. It never joins
the crafting grid or a built-in station. The machine owns its matching and menu.
For custom decode/write logic, implement `RecipeSerializer` and use
`register_read_only_recipe_serializer(serializer_id, recipe_type_id, serializer)`.
Its value follows the same explicit Wire codec rule. Type declarations may be
forward references; startup validates them after all mods register.

Every engine serializer is registered too, including serializers in the
`minecraft:` namespace. An **unregistered serializer refuses the entire recipe
load**, naming the file and ID. Dedicated startup stops; a failed `/reload`
retains the old recipe table. There is no raw fallback for unknown IDs.
Content mods (`.smod`) shipping mod recipe JSON need the corresponding registered
serializer. Optional-mod recipes must use load conditions: a false condition
skips a file quietly **before** serializer lookup. Unknown condition types retain
the existing fatal load behavior. Malformed recipes of a known serializer retain
the existing skip-one-file behavior; changing that is a separate task.

Reload re-reads values and preserves startup registration. Joining Sunrise clients
receive serializer-written content; vanilla clients receive no custom kind.
`datapack_sync.recipe_types` selects actual recipe type IDs; the engine resolves
each file's serializer to its reported type for filtering and replacement, including
all serializers producing the same type. An empty registered crafting serializer still participates in sync
so removing its last recipe clears stale client data.
