# Data files

A mod has one top-level `data/` tree and one `assets/` tree, with the same namespace and
content checks as a content mod. They are normal pack resources, not a second native-only data folder.
[Server data reload listeners](https://github.com/sunrise-org/sunrise/blob/main/crates/sunrise-mod-vocab/src/hooks/notes.rs#L2008) register through
`Hooks::add_server_reload_listeners`. Their `ServerReloadContext` lists namespaced IDs and reads the
winning bytes from selected datapacks and the mod's own `data/`. Parse once in prepare and publish in
apply, not on the tick. Server listener applies run before `data_pack_files`. A later refusal
keeps the old engine tables and Parts, but cannot undo effects of listeners or hooks that already
ran. Contributions share a list, so a failed file cannot be attributed to a particular mod.
The original pack trees remain separate from the launcher's code-only horizon-mixins input.

Client-side asset reads use [client asset reload listeners](https://github.com/sunrise-org/sunrise/blob/main/crates/sunrise-mod-vocab/src/hooks/notes.rs#L2102); server
listeners cannot read `assets/`. Packaged assets also participate in the game's normal resource
packs.

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

The game hashes each file once, when the package opens, and keeps those bytes: your registration hook reads them with
`cx.runtime_files()`, a handle you may keep and use from any thread. `files.names()` lists them, `files.file("nav.bin")`
returns the checked bytes (or `None` for a name your manifest does not list: a mod reads only its own package's files),
and `files.file_path("nav.bin")`, for native code that wants to map or compile a large file without a copy, returns where
it sits on disk. A
package installed as a `.smod` archive has no path (`NotAFolder`), and a later edit of a folder is yours to guard
against. A mod whose package declares no `[runtime]` gets `RuntimeFilesError::NoRuntimeFiles`.

Although the archive accepts `build.rs`, a native build refuses it unless you disable it with
`build = false` in Cargo's `[package]` table and remove build dependencies.

A native `.smod` carries both source and content: the launcher compiles the source, and the game
loads the package's `assets/` and `data/`. All packages take their identity from `sunrise-mod.toml`.
