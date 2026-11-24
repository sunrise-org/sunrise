# Live-world public API fixture

This independent synthetic example uses Sunrise's public mod API and `std::sync` on both native
and wasm. It has no external physics library or other mod dependency. Its input is a caller-owned
kinematic pose sequence: at most 120 accepted steps, each requesting 0.01 blocks along X and
0.25 degrees of yaw relative to the run's starting sample. It uses the default
`CollisionChecked` publication policy. A refused sweep remains refused; no physical dynamics,
contact settling, gravity or collision resolution is claimed.

These are canonical authored fixture sources, without generated Cargo manifests, SDK pins or
binaries. Generate an ordinary native or wasm mod project using the tool for the exact frozen
experimental build, then copy this directory's `src/` into it. Build and runtime receipts belong
beside the local experiment's validation evidence; source presence is not a success claim.

## Build after the coherent SDK exists

Use the ordinary tool supplied with the exact frozen build. `<FROZEN_BUILD>` and `<RESOURCES>`
are placeholders, not available build or resource claims. Use normal `new` for a new project (or `update` for the existing generated workspace), then
copy this canonical source and check/build; do not patch generated SDK source or pins by hand:

```sh
cd /private/tmp/sunrise-live-world-api-example/live_world_fixture
cargo sunrise-mod update --sunrise <FROZEN_BUILD>
cargo sunrise-mod check --native
cargo sunrise-mod build
```

The integrator loads the resulting package into the **one existing owned isolated development
application and test save**, using its serialized launch procedure and existing read-only
`<RESOURCES>`. Inspect an unused fixture namespace and the empty reserved parent destination/sweep
area inside that save before enabling initialization. Reuse that same application, save, scratch
homes and resource location for restart checks. Keep its isolation settings, including
`SUNRISE_LAUNCHER_NO_SELF_UPDATE=1` and `SUNRISE_NO_AUDIO=1`. These project commands do not authorize
another staged application, installation, fixture game folder, game/world copy or compilation cache.
This author has not executed project/runtime commands or created any of those artifacts.

For wasm, use the identical `src/` in the separately generated ordinary project at
`/private/tmp/sunrise-live-world-api-example/wasm/live_world_fixture`. Its normal generation command is
`cargo sunrise-mod new wasm/live_world_fixture --template wasm --sunrise <FROZEN_BUILD>` from the
example root. Keep the native manifest unchanged. In that wasm project use
`cargo sunrise-mod check --wasm` and `cargo sunrise-mod build --wasm`. Let the tool generate its
manifest and wasm fields. This is a documented second-target route, not an executed second
project created by this author or a wasm success claim. The integrator owns generation and validation
of that tiny second project and reuses the shared SDK target/cache and read-only resources.

## Exercise the fixture

Commands require operator level 2. Each mutating command queues work for `ServerTickStart`;
the queue feedback confirms admission only. Read `status` after each step and wait for an actual
receipt before proceeding. `read` is an alias of the same read-only diagnostic operation.

| Command | Effect |
| --- | --- |
| `/live_world_fixture initialize` | Explicit first-use opt-in. Persist a same-source `Initializing` marker, seed only empty runtime-source cells, checkpoint `LocalOwned`, then create/checkpoint a projection. |
| `/live_world_fixture run` | Start the finite kinematic sequence from the actual accepted sample. |
| `/live_world_fixture pause` | Stop further requested pose steps; ordinary furnace ticking continues. |
| `/live_world_fixture checkpoint` | Request the source's common barrier and report its actual receipt. |
| `/live_world_fixture return` | Prepare and stage one batch from the runtime source to fixed ordinary-world cells; save `WorldOwned` in the same root transaction. |
| `/live_world_fixture refuse` | After return, map one cell to an empty runtime destination and a second to an occupied, unselected ordinary-world cell. Require `Obstructed` at preparation and unchanged actual revisions. Unexpected preparation is canceled and reported as unproven. |
| `/live_world_fixture capture` | Prepare/stage the reverse ordinary-world-to-source batch. Wait for the actual receipt, checkpoint `LocalOwned`, then project the existing payload at a separate binding frontier. |
| `/live_world_fixture project` | Retry binding creation from an inspected `LocalOwned` state; it never seeds cells. |
| `/live_world_fixture status` | Read the actual source marker, sample, members, both levels' revisions and both levels' chest/furnace data. Unavailable/stale data is shown as such. |

Explicit initialization checks every reserved parent return cell is resident, block-tick ready and
actual Air before its first source marker or seed write, including on a later initialization retry.
The source is `live_world_fixture:local`, a mod-owned Void runtime level. The sparse stone platform
occupies two source chunks, has a central hole and a detached cell, and carries an ordinary chest
and furnace. Its first pose translates local Y by 96. Return materializes the explicit original
cell mapping at ordinary-world Y 160/161, irrespective of the last visual pose. It refuses occupied
destinations; it never clears the destination to make the operation succeed. Inspect the unused
fixture namespace and empty reserved destination/sweep area inside the existing owned test save.
Do not edit fixture membership while testing
the transfer sequence; changed projected membership is refused rather than partially transferred.

`status` reports current data, not expected inventory values or fabricated counters. The marker
encodes the returned canonical previous/current poses as exact IEEE bits and keeps the saved phase,
last motion-step count and planned/actual transfer epochs. `durable_epoch` means the last actual
receipt observed by this process; transaction closure `Ok` and stage's planned epoch are not durable
receipts. Full receipts include actual level revisions and binding/tombstone identities.

An ordinary registered `TicketController` holds only finite bootstrap/transfer columns from a global
callback, then waits for actual `should_tick_blocks_at`. Successful binding hands retention to the
binding and releases those tickets. Restore reads the saved descriptor's real `source_chunks`,
instead of assuming the initial selection's columns. Saved bootstrap tickets are reconciled only
for this fixture's controller/owner. No world's `/forceload` state is used.

Restart paused. Existing data is restored from its saved phase; there is no automatic creation on
`NotSaved`. A missing marker beside a saved binding, corrupt attachment, `Storage`, migration-required
record or incomplete `Initializing` phase stops the fixture for inspection. The fixture stops issuing
mutations after `Storage`; fencing all game ticks/actions/network/save routes belongs to the engine.
An acknowledged transfer and its source phase can remain durably owned but unprojected across a
restart. Restoration or later binding creation consumes those existing contents and never reseeds.

## Evidence to retain when the integrator runs it

Retain the exact frozen build/SDK/toolchain/package hashes, command results, source descriptor,
actual transfer/checkpoint receipts, and before/after `status` reads. Check chest inventory and
furnace progress in both directions and after a fresh-process reopen; verify exactly one owner,
no duplicate ticking across the global frontier, retired binding generations, and occupied-batch
refusal before any first-source mutation. Native and wasm need separate real loader evidence.
`sunrise-ctl simulation live-worlds [SOURCE]`, when provided by that frozen build, can supply an
additional authoritative engine-side inspection; it is not an output generated by this author.

Provenance, exact inputs, formatting results and remaining blockers are recorded in
[AUTHORING-PROVENANCE.md](AUTHORING-PROVENANCE.md).

On restore, the fixture compares the exact saved CURRENT pose component bits and revision with
the actual host sample, and requires previous=CURRENT settlement. It retains the historical
publish receipt's previous pose in its persistent audit record; that old movement is not replayed.

The example explicitly budgets 64 MiB for a transfer's complete common checkpoint afterimages,
including already open covered worlds and player data. This is the host's bounded default
checkpoint ceiling, not a promise that every existing save fits. Actual oversized snapshots
refuse before Prepared; the example does not clear content or treat a planned epoch as success.
