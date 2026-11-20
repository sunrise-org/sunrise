Two native mods whose `[[native.mixin]]` rows the launcher builds against the prebuilt game by function
replacement (no game rebuild): `tools/ci/fn_replace_check.sh` builds them and runs a dedicated server.
Every helper prints a line starting `fnr:` so the run shows what ran, in what order.

`loop_generic_head` is built on its own, never beside `loop`: a HEAD mixin on the published loop's generic
function with no `[[native.loop]]` row, so the copy is the game's own loop with the empty slot.

`loop_second` is built beside `loop`, in the workflow's main set: a second mod's ticking block entities in the same world, and a slot of
its own that counts only those.

`api_only` has a typed hook and no native patches. Alone (`FNR_ROUTE=prebuilt-api`) it
takes the API route: the package's prebuilt API executable with the launcher's `sunrise_native_mods` library. In the
main set, beside mods with `[[native.mixin]]` rows (`FNR_ROUTE=prebuilt`), the whole set takes the engine route.

`mod_logging` is an API-only fixture with no `[[native.*]]` rows. On Linux, run
`python3 tools/ci/native_mod_logging.py LAUNCHER SDK_BASE RESOURCES` with a matching client and
dedicated-server SDK and a private copy of prepared resources. It builds two copies with different
manifest IDs and crate names, starts both games in private scratch folders, and checks their actual
log records in order. The server callback logs inside an aborted transaction; its record must survive with no
broadcast or loading warning.

`shared_left`, `shared_right` and `shared_tip` are their own set: the tip depends on the other two, and all three use
`itoa`. Each mod is a dylib, so the tip links only because the launcher builds the mods' registry crates once, into a
library they all depend on (sunrise-org/sunrise#1379). The check finds that library in the export, recorded as no
mod's, and with resources the server prints `fnr: shared_tip left 1 right 2 own 3`.
