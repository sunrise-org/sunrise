# Load phases and access

A mod's code runs in phases. What a hook may touch depends on the phase it runs in: there is no `Host` while
the mod registers, and no world until the World phase.

![Each game thread goes through Packages, Registration, Setup, Config and World, in order; a data or asset reload
enters Reloading and returns to the phase it came from.](../images/load-phases.svg)

Registration, config loading and reload-listener registration all use the installed dependency order.
The resolver keeps bundle-order ties, checks present optional dependencies and refuses cycles. Content-only
packages stay after code in the loaded-mod inventory; listener-specific edges use `ReloadOrder`.

Each owning thread progresses through `Packages → Registration → Setup → Config → World`.

| Phase | Available access |
| --- | --- |
| Packages | Package discovery; no game Host operations. |
| Registration | Declarations through `RegistrationContext`, with its mod-list, registry and settings reads; no Host. |
| Setup | Client setup reads, published asset/data queries and registry/settings/mod-list reads; no world handles or transactions. |
| Config | Settings reads and the mod-scoped `ConfigFiles` service; no world handles, transactions or other game operations. |
| Reloading | Published asset/data queries, registry facts, settings and mod-list reads, listener registration, and `DataLoadContext` callbacks; no world handles or transactions. |
| World | The owning game's side-typed `ServerHost` or `ClientHost`. |

`RegistrationContext::instance_dir()` returns `Result<&std::path::Path, RegistrationReadError>`.
The path is borrowed from the startup snapshot and preserves the host's native encoding,
including non-Unicode names. Use it directly for path operations, or call
`cx.instance_dir()?.to_path_buf()` if you need to keep it beyond registration. A context
with no startup snapshot returns `RegistrationReadError::Unavailable`.

A mod that does slow work while the game starts shows it on the startup loading screen
(NeoForge's `StartupNotificationManager` and `ProgressMeter`). `RegistrationContext::progress(label,
steps)` opens a bar under the game's stage bar and returns `Result<StartupProgress, StartupProgressError>`.
It returns `TooManyBars` when 64 bars are already open. The returned bar is owned and `Send + Sync`:
call `step()`, `set(n)` and `label(text)` to update it, and drop it to remove it. A mod can keep the bar
after registration and update it or call `bar.message(text)` from a worker thread or from `client_setup`.

`RegistrationContext::message(text)` shows a line for a few seconds without opening a bar. Both
message methods and `bar.label(text)` borrow `text`; the caller need not keep it alive after the call.
The newest four bars and the last three lines are drawn, each cut to one line. A dedicated server
logs each label, line and progress in 10% increments instead.

This is part of the startup progress feature, not live rendering of synchronous setup work.
Bars can be opened through `RegistrationContext`; `client_setup` has no creation context and
must retain a handle from registration. Updates become visible only while the loading screen
is actually drawing. A synchronous `client_setup` listener runs on the render thread, so its
updates cannot draw live and may appear only after it returns, if another loading frame is drawn.
Slow work can run in a mod-owned worker and step a retained bar, but neither that worker nor
the bar keeps the loading screen open. Live display of synchronous setup work and a creation
route from `client_setup` remain outside this change.

## Registration and setup

Startup hooks such as `register`, `component_types`, `blocks`, `items` and `standalone_models`
receive no `Host` and read startup facts through `RegistrationContext`.
`register` receives `&mut dyn Registrar` for writing declarations. In `component_types`, call
`event.register("mymod:charge".into())` with each owned, namespaced ID. The game checks IDs and
duplicates before freezing the component registry.

Scoped registration events are borrowed only for the listener call. Their registration methods
add entries in dispatch order without letting a listener read, edit or remove earlier registrations.
If the phase gate refuses a listener, its additions are discarded and earlier registrations stand.

A callback registered on the event cannot borrow the event or the context. Capture owned inputs
for its `Send + Sync + 'static` closure. A model layer supplier runs once at the startup freeze;
a particle group factory runs once per particle engine, not on sprite reload. A standalone model
baker can run on a resource worker during each load, and borrows its bake context only for that
invocation. See [model drawing](../how-to/client/model-drawing.md) and
[particle render groups](../how-to/client/particles-and-movement-queries.md#particle-render-groups).

Not every registration-context hook runs only at startup. `register_fluid_models` and
`register_texture_atlases` collect declarations again on client resource reload before baking or
stitching, respectively. Atlas admission, duplicate handling and dispatch timing are unchanged.

The context's `mods()`, `registry(name)` and `setting(mod_id, key)` read
one view captured before the first of them runs, so every registration step sees the same values
whatever the dispatch order: the loaded packages, the built-in registries as `registries.json`
lists them (not what mods declare during registration) and the settings on disk when the snapshot is captured. A
mod validates the keys it declares against them from `Hooks::register`; a name that is not a
namespaced identifier, an unknown registry, an empty or path-like setting key, a package that is
not loaded, or a context the engine gave no data returns a `RegistrationReadError`. Client setup
runs once after the initial asset apply. The window's Client/Common configs load once immediately after client setup and before title,
connection, input, render or ordinary tick callbacks. This includes pack-error recovery: the failed
selection is cleared and saved during initialization, but its options screen opens only after Config
completes. A title-screen callback is already in World: world-handle constructors can return `None`,
rather than being forbidden in Setup. A dedicated
server loads Common once before its first world; Server configs load and unload per world.
A second world does not repeat registration or client setup. Config loading can read settings and
use the scoped `ConfigFiles` service to read or save its own config files. World handles,
transactions and other game operations panic with `Config` in the diagnostic.
Config file polling is not a phase transition, and Server config unloading leaves the World phase.

Attribute bounds are registration data too. `Registrar::widen_attribute_bounds` asks, from a both-side package's
`register`, join the attribute catalog once when it grows: after the attributes mods declare with
`declare_attribute`, and before entity attribute rows resolve and anything reads a definition. The bounds become
the union of every mod's asks, so the result does not depend on the dispatch order, and the default never moves
([content mods](../how-to/content-mods.md) has the call, its refusals and what saves keep).

## Reloading

Every data or asset reload enters `Reloading`, including listener registration, prepare, apply and
the reload completion notice. Initial loads can nest in Setup; runtime reloads nest in World.
Returning, including after a failed prepare, restores the entering phase. Published asset/data
queries read the published generation, not pending prepare output: completion callbacks can
refresh sprite and recipe caches after apply, including on first world open, without gaining world
handles or transactions. Their rustdoc names their load-phase availability. Server listener
registration retains `&dyn Host`, restricted to these published facts, registry facts, settings and
mod-list queries. Callbacks that take `DataLoadContext` use its restricted reads. Neither permits
opening a world handle or transaction. The first client registration and prepare run on the loader
thread; later registration runs on the render thread, prepare on its worker, and apply on the owning
game thread.

## Load contexts

`DataLoadContext`, `LoopContext` and `WorldgenHost` lend frozen metadata for their resource
generation. `registry(name)` returns `Option<&[String]>` in registry ID order,
`setting(mod_id, key)` returns `Option<&str>`, and `mods()` returns `&LoadedMods` in dispatch order.
The getters do not read files. `None` means the registry or setting is unavailable, not that it is
present and empty. Use the references within the callback; take an owned value only if it must
outlive the context borrow. Use `DataLoadOrigin` to identify the load origin and `LoopSide` to identify
the owning side.

`DataLoadContext::fluid_tags()` borrows the recipe pass's fluid-tag table. It returns `None` when
there is no recipe overlay, `Some(&[])` when that overlay has no fluid tags, and otherwise a slice
of `(tag ID, members)` pairs with each tag's members in source order.

During registration, `RegistrationContext::mods()` already lends the startup inventory.
`RegistrationContext::registry` and `RegistrationContext::setting` still return their original
owned results and report `RegistrationReadError` as described above.
[Chunk disk callbacks](../how-to/chunk-data-load.md) also borrow registry names, but from the
resource generation retained by their worker.

### Ordinary Host reads

In World, both client and server hosts expose `registry_entry(registry, id)`,
`registry_entries(registry)` and `registry_entry_in_tag(&entry, tag)`. These read the
current side's live registry and merged tags, not pending reload data. Unknown or
unavailable registries and entries return `None`; an absent tag or a tag that does
not contain the entry returns `Some(false)`. Enumeration yields each identity once.

`ConfigHost::setting` still returns `Option<String>` on an ordinary `Host`; it is not the
captured `Option<&str>` read on a load, loop or worldgen context.

Server world-generation reads use the published decoded owners, including the
live feature data in worlds without a terrain locator. Invalid source files do
not establish entries. Prepared world-preset files have no published complete
registry owner, so `minecraft:worldgen/world_preset` is unavailable; selecting a
preset, sharing generator resources or reopening a saved stem does not change that.

`RegistryEntry` is a plain Wire value containing `registry: Identifier` and
`id: Identifier`. Its names may be retained across
reloads; constructing the value does not establish that the entry exists.
`as_block_key(host)`, `as_item_key(host)` and the corresponding narrowing methods
for block entity types, boolean game rules, dimensions, effects, enchantments,
entity types, fluids, menus and particles resolve a real host-issued typed key.
A different registry or unknown entry returns `None`. Typed keys remain
process-local; narrowing never treats a protocol numeric ID as a key slot.

Mod-declared datapack registries resolve tags against the accepted candidate
before publication; tags with missing required members are omitted. Synchronized
snapshots carry those exact tag names and members (including empty tags);
server-only and unnegotiated registries send neither entries nor tags. Clients
validate membership before activating a snapshot.
On clients, generic reads never fall back to prepared server-only datapack files:
built-ins and the connection's currently synchronized registries are the entire view.

The game alone advances phases. An illegal or wrong-thread advance is refused before dispatch;
a forbidden mod call panics naming the phase, rather than returning a `Result`.
There is no separate common-setup, load-complete or intermod event: registration and the first server
reload cover common setup, client setup is `client_setup`, dedicated setup is `server_about_to_start`,
and dependencies express intermod ordering. Load complete is the edge after initial Config and before
`server_about_to_start`, not a hook.

`DataLoadOrigin` and `LoopSide` describe the local load or loop, not data to encode. Read them
through `DataLoadContext::origin()` and `LoopContext::side()` as before; if a payload needs such
a fact, map it into a field of your own [Wire record](wire-types.md).

[FML loader APIs and their Sunrise routes](../reference/fml-loader-apis.md) compares the two loaders'
file and path APIs, including the game directory.

## Settings snapshot limits

Settings snapshots accept regular UTF-8 files only. File and package-directory symlinks,
including internal links, are not followed. Invalid UTF-8, special files and refused entries
are absent from the snapshot rather than reported as read errors.

| Limit | Behaviour |
| --- | --- |
| 64 KiB per file, before trimming | Oversized values are omitted. |
| 1,024 captured keys per mod | Additional keys are omitted. |
| 16 MiB for the snapshot, before trimming | Values exceeding the remaining budget are omitted. |
| 4,096 enumerated names per package | A package exceeding the budget is omitted entirely. |

Capture proceeds in sorted package-ID and key-name order. Missing settings directories and
empty settings produce no warning. A capture with refusals emits one warning naming each
known omitted key, its mod and its reason; an inaccessible or incompletely enumerated
directory is identified by scope instead of inventing a key. Enumeration-limit diagnostics
name the entries visited within the bounded scan; the entire package is still omitted.
Initial registration, item-data loading, table preload and loop publication share one
captured settings snapshot rather than rescanning it on each startup thread. The game,
audio and world-generation readers retain that generation's immutable snapshot without
recapturing or repeating diagnostics. A new capture reports new refusals,
even if an earlier generation already warned. These limits apply to scalar settings, not
mod data files.
