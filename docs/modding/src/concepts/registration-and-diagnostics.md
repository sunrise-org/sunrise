# Registration and diagnostics

Declare keys, Parts, and crash-report callbacks in `Hooks::register`; submit blocks and items
with `event.register(declaration)` in `Hooks::blocks` and `Hooks::items`. Sunrise collects these
declarations once at startup, before resources load. The registrar and events are borrowed for
their respective hooks and cannot be kept to submit a declaration after the hook returns.

If you expose registration to scripts, require the matching registration hook before
queuing a declaration. `mod_api::registration_open()` can reject calls outside the phase,
but it does not tell you which hook is running. A late call (for example, from
`player_logged_in`) must not enter the queue: return `DeclarationError::RegistrationClosed`
and log the submitting mod and requested ID in the binding. The phase query itself does not log.

The submitting package owns its block and item declarations. `BlocksEvent::register` and
`ItemsEvent::register` return a typed `DeclarationError` for invalid IDs or fields. Checks that
need other declarations or loaded resources run after collection: for example, a submitted item
may conflict with another declaration's generated block item. A refusal is fatal at startup,
even if the caller ignores the error. The startup error and log identify the submitting mod,
declaration kind, ID, and reason.

If your `Hooks::register` bootstrap cannot complete, call `cx.refuse("reason")` and return.
Sunrise records the refusal directly against the mod whose `register` hook is running. It does
not panic or exit the hook for you. Like `warn`, `refuse` only records diagnostics in
`Hooks::register`; it has no effect in other hooks. These reports are separate from the native
block and item declaration errors described above.

## Registration refusal

- `refuse` applies only to the calling mod; it takes no mod ID.
- All declarations from that registration are discarded, including declarations made before
  or after the call. Later declarations do not add secondary validation errors.
- Other mods' declarations are unchanged.
- The first reason wins, including an empty reason. Reasons are preserved verbatim up to
  4096 UTF-8 bytes; longer reasons are truncated at a character boundary.

## Loading warnings

For log locations and filtering, see [Find your mod's diagnostics](../how-to/read-the-game-log.md).

A problem that should not stop your mod, such as a missing optional integration or a setting it
corrected, is a warning: `cx.warn("mymod.warn.no_jei", &["jei"])` inside `Hooks::register`.
Like NeoForge's `ModLoader.addLoadingIssue` with `ModLoadingIssue.withAffectedMod`, the
registration records the warning against the mod that reported it.

- The key is a translation key in your mod's own `lang/` files; the arguments fill its `%s` in
  order.
- Like `refuse`, it takes no mod ID: the warning belongs to the calling mod. The mod keeps
  loading and its declarations are kept.
- The log names the mod, the key and the arguments at warn level. The Mods screen shows the mod's
  first warning under its row in the player's language, or the key when no language has it; the
  row is not marked failed.
- A mod keeps its first 16 warnings; later ones are dropped. The key and each argument are cut as
  refusal reasons are, to 4096 UTF-8 bytes at a character boundary.

## Native startup and hook panics

The launcher supplies ordered mod IDs and hook factories to the installed game's
[native entry template](https://github.com/sunrise-org/sunrise/blob/main/crates/sunrise-client/src/native_main.rs.template).
The template constructs `ModEntry::new("example", example::hooks())` entries and passes them to
`sunrise_mod_host::main_with`. `Mods` is `Arc<[ModEntry]>`: each entry binds its ID to its hooks.
Before installation, `mod_api::mods()` returns an empty list.

Game source without the required template is refused before mod staging with:
“This game source has no native inventory entry template. Update Sunrise and rebuild the mods
against this game.”

A native hook panic stops the game. Its crash report and server-thread log identify
`in native mod <id>, hook <hook>`, using the `sunrise-mod.toml` ID. Parts retain their registering
mod's ID for startup errors and hook panics.

## Crash-report callbacks

Register callbacks inside `Hooks::register` (the counterpart to NeoForge's `CrashReportCallables`):

| Registration | Report output |
| --- | --- |
| `r.crash_section("caches", Arc::new(Caches))` | `<mod id>: caches: <value>` after Sunrise's system details |
| `r.crash_header(Arc::new(Banner))` | `<mod id>: <value>` under the report title |

Both callbacks implement `CrashSection`. The report calls `value()` once when it is written,
not during registration. Return `None` to omit a line.

- Lines follow registration order on clients and dedicated servers.
- Callbacks run on a separate thread. The report waits at most two seconds for headers and
  another two seconds for system-detail sections.
- A callback that panics or times out produces `~~ERROR~~`; the report is written.
- Callbacks belonging to the mod blamed for the crash are not called.
- Control characters become spaces, and each value is limited to 4096 bytes on one line.

## Converted-package attribution

If your native runtime hosts other packages, declare their IDs with
`mod_api::content::crash::ConvertedPackages::new(runtime_id, ids)`. Obtain a handle with
`packages.package(id)`, then call its `enter(export, site)` method around the converted package’s call.

The scope adds `in converted package <id>, export <export>, call site <site>` below the runtime's
`in native mod` line. Nested scopes restore the previous attribution on return and panic unwind.

- `package(id)` returns `None` for an undeclared ID.
- `enter` returns `None` unless the thread is executing a native call owned by that runtime.
- The guard is neither `Send` nor `Sync`. Attribution ends when the native frame that entered
  it exits, even if you retain the converted-package guard.
- A nested native frame hides the outer frame's package until it returns.
- Worker threads need their own native call and package scope.
- Entering a package clones its stored ID on each call; borrowed static IDs do not allocate.
