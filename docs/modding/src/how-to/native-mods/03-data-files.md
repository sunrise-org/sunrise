# Data files

A mod has one top-level `data/` tree and one `assets/` tree, with the same namespace and
content checks as a content mod. They are normal pack resources, not a second native-only data folder.
[Server data reload listeners](https://github.com/sunrise-org/sunrise/blob/main/crates/sunrise-mod-vocab/src/world/server_reload.rs) register through
`Hooks::add_server_reload_listeners`. During registration, use `event.listeners.get_registry().iter()`
to read names in registration order and `event.listeners.get_graph().iter()` to read dependency pairs
`(first, second)`. Both lend IDs from the registry, so clone only the names you need after the borrow ends.
`event.listeners.get_last_vanilla_listener()` borrows the effective ordering anchor (the built-in global
loot modifier listener). Use `VanillaServerListeners::functions()` to order against the function library
itself.

A listener's `ServerReloadContext` lists namespaced IDs and reads the
winning bytes from selected datapacks and the mod's own `data/`. Parse once in `prepare`, then return
`Ok(Box::new(move || { /* publish prepared data */ }))` from the method, whose return type is
`Result<Box<dyn FnOnce() + Send + 'static>, ReloadResourceError>`. Capture owned data: the runner
consumes the action once on the owning thread
only after all listeners prepare successfully. A prepare error drops pending actions without
running them. This work does not run on the tick. Server listener actions run before `data_pack_files`.
A later refusal keeps the old engine tables and Parts, but cannot undo effects of listeners or hooks
that already ran. Contributions share a list, so a failed file cannot be attributed to a particular mod.
The original pack trees remain separate from the launcher's code-only horizon-mixins input.

Client-side asset reads use [client asset reload listeners](https://github.com/sunrise-org/sunrise/blob/main/crates/sunrise-mod-vocab/src/client/client_reload.rs); server
listeners cannot read `assets/`. Client listeners return the same once-only action, which runs on
the game thread after the new packs are published. Packaged assets also participate in the game's
normal resource packs.

When registering a listener on either side, construct its converter with
`FileToIdConverter::json("machines")` for JSON or `FileToIdConverter::new("machines", ".bin")`
for another suffix. Invalid arguments return a `FileToIdConverterError`. Use the IDs returned by
`VanillaServerListeners` and `VanillaClientListeners` methods to declare dependencies on the
corresponding side. The converter, ordering helpers, prepare contexts and reload errors all stay
in-process; they are not `Wire` payloads.

Both prepare contexts lend converted IDs through `context.list()` and bytes through
`context.read(&id)`. To abort on an invalid file, return `Err(ReloadResourceError::new(id, cause))`
with the original error. For a validation message without a cause, pass a typed validation error
or `std::io::Error::other(message)`. `error.resource()` borrows the failing resource ID, and
`std::error::Error::source(&error)` exposes its cause. Registration returns typed
`ReloadOrderError` values for dependency and listener failures. No listener applies until every
prepare succeeds.

Outside a listener, `host.resource(pack_type, &identifier)` reads one file from the selected
stack: `PackType::ServerData` takes `mymod:machines/furnace.json` from `data/`, and
`PackType::ClientResources` takes `mymod:models/block/furnace.json` from `assets/`. As with
`ResourceManager.getResource`, the highest selected pack that supplies the file wins; filters,
a world's datapacks and a mod's own root data all apply. The answer is the winning bytes, or
`None` for a missing, unreadable or over-64-MiB file. Use a reload listener for a whole folder
or for parsing at reload time; a single read does not reload packs. Directory-pack
symlinks follow vanilla pack behavior: linked resource files are read through the pack.

## Embedded data

Generate code-only tables as Rust source before packaging and include them under `src/`.
The launcher refuses mod build scripts with `E_NATIVE_BUILD_SCRIPT`. Embedded tables do not
participate in resource reloads.

## Package contents

A native bundle accepts the manifest, optional icon, root `Cargo.toml`, `Cargo.lock` and `build.rs`,
`.rs` and `.toml` files under `src/`, `assets/` and `data/`, a `runtime/` folder listed in `[runtime]`, and root text files (`README*`, `LICENSE*`,
`LICENCE*`, `COPYING*`, `NOTICE*`). Source and resource size limits are checked separately;
any other file causes the archive to be refused ([the manifest](../../reference/manifest.md)).

`runtime/` holds the data files your code opens beside itself: each is listed in `[runtime]` with its SHA-512, any
file type, up to 64 MiB a file, 256 MiB and 256 files in all. They are not pack data and no reload touches them;
a missing, extra or mismatched file refuses the package by name ([`[runtime]`](../../reference/manifest.md)).

The game hashes each file once, when the package opens, and retains the verified bytes. Get the handle with
`cx.runtime_files()` in registration; you can keep it and use it from any thread. A mod can read only its own
package's files. A package with no `[runtime]` returns `RuntimeFilesError::NoRuntimeFiles`.

- `files.names()` borrows the listed names in name order as `&[String]`.
- `files.file("nav.bin")` returns `Option<&[u8]>`: parse the borrowed bytes directly, or use `.to_vec()` only
  when the bytes must outlive the handle. An unlisted name returns `None`.
- `files.file_path("nav.bin")` borrows the exact disk path as `&Path`; use `.to_path_buf()` if you must keep
  the path after dropping the handle. An unlisted name returns `RuntimePathError::NotListed`; a listed
  file in a `.smod` archive returns `RuntimePathError::NotAFolder`. This is a live path: opening it after
  a disk edit may read different bytes. Use `files.file()` when you need the verified snapshot.

Although the archive accepts `build.rs`, a native build refuses it unless you disable it with
`build = false` in Cargo's `[package]` table and remove build dependencies.

A native `.smod` carries both source and content: the launcher compiles the source, and the game
loads the package's `assets/` and `data/`. All packages take their identity from `sunrise-mod.toml`.
