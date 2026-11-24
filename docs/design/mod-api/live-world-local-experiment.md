# Local experiment: live world projections

Refs #1022. This is local experimental work, not an approved upstream spec,
claim, or implementation of all #1022 acceptance criteria. Nothing in this
experiment asserts clean-room provenance. No public coordination was performed.

## Route and boundary

A mod-owned runtime Void dimension is the single authoritative local world.
Its ordinary chunks, block entities, inventories, attachments, block/fluid
schedules and server tick contexts remain the existing engine instances. A
projection owns only a bounded sparse membership index, rigid pose samples and
a reference to a destination level. Poses come from the caller; the engine does
not add another rigid-body integrator. Projection drawing and collision read the
same local cells and sample pair. No per-tick block reconstruction is permitted.

The alternative is extending the private carrier representation. It already has
durable source-to-carrier ownership transfer and conservative contact geometry,
but its persisted cells have i8 coordinates and no live level/ticker/menu context.
Merely exposing those snapshots would not make inventories live. The runtime
route preserves the actual level context and configurable checked i32 coordinates.
Its cost is that projected player actions and cross-level atomic transfer need
explicit integration; a dimension name alone proves neither.

## Experimental slice

Normal shutdown finishes a final common checkpoint after actual leaving-player snapshots.
A write made by a post-acknowledgment SaveToFile notice refuses the final cut and fences the
session. Independent SaveDir flush remains refused while common leases are active; a clean
fully closed level releases its leases. This is the ordinary server teardown route, not a
caller promise to checkpoint before quitting.

The first implementation adds catalog-generated native/wasm handles for binding a
real runtime world, inspecting its authoritative cells, ray targeting, publishing
revalidated external poses and retiring the projection. It uses the existing
carrier contact kernel and the existing moving-collider/carry and transformed
block drawing routes. All inputs and work are bounded, and refusals preserve the
previous projection. Source and destination residency are checked without loading
or generating chunks inside an API call. Source blocks retain their one ordinary
inventory/ticker owner regardless of motion. Geometry and prepared model caches invalidate on source block and resource revisions,
including while the source is parked. Inventory-only writes do not rebuild the static
physics/model caches. The Source slice refuses contextual and position-dependent shapes;
ordinary destination terrain uses the existing actual World/position collision query with
an empty entity context, preserving query errors and residency/work admission. Source special
rendering is currently restricted to real chest lid contexts.

A binding keeps its initial local pivot. Published poses with another logical pivot
are reanchored without changing any physical point; returned samples use the binding's
pivot. Linear pivot displacement and shortest quaternion motion are swept continuously
together with bounded conservative advancement. The explicit pivot is carried through
the existing frame/history path, so drawing, carry-age interpolation and collision
samples preserve the same pivot. An idle publisher collapses previous/current samples.

An upright carried body can need support-face clearance when its support turns around a
horizontal axis. The existing carry path requests that clearance only within the body's
ordinary step allowance, through the same collided movement and single movement gate. It
then inspects actual current/downward contact instead of forcing a ground flag. A cancelled
invocation preserves the complete previous body state, including through the player and
armor-stand movement delegates. Ordinary gravity may settle an admitted body through a real
remaining gap; that is distinct from embedding, falling through or replaying a carry.

Active membership derives ephemeral ordinary source streaming retention; it never changes
persistent command or mod-forced tickets. `LevelRef::projection_footprint` reports the real
terrain-ring charge before a binding, and `should_tick_blocks_at` observes each open parked
level's own tickets and pending entities. Initial source activation must come from a global
or running parent callback. Automatic source close releases binding caches and reservations.
The client advertises and enforces a bounded four-slot viewed-level capacity; older clients'
smaller negotiated caps and existing area/column/security limits remain authoritative.

`LevelRef::revisions` reads actual open-world block and block-entity content counters without
a binding or chunk load, with ordinary typed read refusals. The target-aware
`LevelRef::attachments` adapter borrows the actual owned level holder. An existing binding's
accepted pose and opaque caller metadata can join one ordinary Host transaction; cancellation
restores samples, revisions, per-tick admission and exact target attachment before-images.

The default limits are operating reservations, not coordinate representation limits.
Coordinates remain checked i32. Mods select bounded cell/chunk/box/contact/candidate
and motion-subdivision budgets subject to administrator-owned per-owner/aggregate
quotas. The service refuses work it cannot resolve within those budgets.

The negotiated metadata channel carries only binding membership, real-source state
witnesses and pose samples. Existing viewed-level packets own the client world's
blocks and block entity updates. A client exposes a new sample only after its real
viewed member states match the witness, and retains derived coherent caches while
updates are incomplete. Equal-frame divergent messages refuse.

A connected client consumes coherent contact motion once per local player tick, before its
ordinary position/support-age report. Models and the visual pick outline use that completed
client tick's private presentation pair, matching the camera even when network samples
coalesce; public accepted samples and authoritative current hit/action geometry are unchanged.
Late join and idle ticks settle presentation. Actual collision-equivalent state changes retain
contact through immutable local-box equality, while membership/resources or real shape changes
settle it. Viewed state/column mutations advance the existing World revision; duplicate state,
block-entity-only and light-only updates leave block geometry valid. See the separate
[client cost account](live-world-client-costs.md); cached shapes do not imply allocation-free
combined placement indices.

The explicit owner action methods re-clip from a real player's eye, check reach and
ordinary-world/other-projection occlusion, and lend the source context without moving
the player's physical body. Ordinary placement prepares exact canonical states/shapes,
checks membership/stream/collision limits and asks the typed live_world_admission hook
before mutation or stack consumption. Callback writes join the ordinary transaction;
stale decisions refuse without replay. Breaks retain the hit and source/pose/membership
and storage epochs through callbacks, revalidating immediately before removal.

Ordinary client picking and authenticated main-hand guest requests retain the transformed
server hit. The supported input slice includes ordinary block-item placement and stock
chest/furnace menu operations, slot/progress synchronization and continuing source-frame reach
and holder checks. Physical building blockers use the existing oriented kernel against actual
parent bodies; the physical player is never moved into local coordinates. Menus retain real
source holders, release ordinary openers and conserve the cursor on reach loss, retirement or
queued source close. Caller-held outer transactions refuse projected use before any screen,
slot, cursor or opener effect. Progressive survival mining, automatic off-hand fallthrough and
specialized recipe/button/toggle packets explicitly refuse in this slice.

The developer control query `simulation live-worlds [SOURCE]` uses the authoritative owner link
to report bounded real cells/states/shapes, live container slots and furnace progress, read
revisions and exact pose bits. It refuses unavailable or oversized snapshots instead of
returning partial success. It is an inspection route, not another inventory or save authority.

The implementation must report separately which runtime, save/reopen, action,
transfer, client tracking and graphical tests actually ran. Declared API shape,
geometry tests, and direct tests of the real Game are different evidence.
No #1022 completion claim follows from this slice.

## Common ownership and save frontier

`LevelRef::prepare_transfer` reads a bounded mapping of real source/destination cells across
open owned runtime levels and ordinary levels. Preparation checks every source witness and
empty destination before destruction; one occupied or unavailable later destination refuses
the entire mapping. Tokens retain optimistic read witnesses, not live inventory payloads.
`stage` belongs to the exact current root transaction; `commit` owns a root when none exists.
The root can include owned targeted Level attachments on a third unbound source. A planned
epoch and closure Ok are intentions. Only `receipt` proves the common durable decision.

Transfer admission is confined to a global server frontier, outside lifted guests, source
menu scopes, BE ticks and per-level scheduler passes. Pending real saved work, callbacks,
menus, alien serializers/Parts, unknown position-sensitive data and unsupported state/BE
contexts refuse before mutation. The supported payload slice includes conservative plain
stock blocks, dry state-invariant slabs and actual single stock chest/furnace models. An
empty saved-tick sentinel has no work to unpack; a nonempty saved list still waits for the
ordinary first-ticking frontier. Existing core queued/held occurrences keep multiplicity,
relative delay, priority and checked imported order. No second scheduler is introduced.

The common journal prepares actual ordinary chunk/entity snapshots, source/destination
payload afterimages, level attachments, projection/tombstone metadata, selected actual player
snapshots and one checkpoint epoch before publishing either ownership change. Existing
SaveDir writers publish these frozen records. Full region allocation headers and unselected
sector/external-stream witnesses protect unrelated contents. Recovery runs under the existing
SessionLock before migration, level/player readers or ordinary SaveDir creation. Ambiguous
publication and post-Prepared failure fence the actual Game session, including ticker,
actions, networking, autosave and close; a caller cannot ignore Storage and continue a
partially committed world. A fresh recovered process is required.

Once common player mode begins it lasts for that recovered session, including an aborted
first binding and final retirement. Ordinary autosave, explicit checkpoint, close and held
streaming retirements use the same group. Targeted metadata joins the actual root readset;
late callback changes refuse before Prepared without replaying the callback. Offline player
file moves and arrival effects that cannot join the supported group refuse before mutation.
The configured common record/byte/open-level ceilings are operational restrictions; no
unbounded snapshot is silently accepted.

Pure pose/binding/metadata roots commit coherently in memory and retain dirty gated coverage;
they do not serialize every world on every pose step. Exact root cancellation restores each
source's previous/current sample, revision, per-tick publication marker and targeted opaque
attachment beforeimage. The existing void transaction close cannot report every generic
error through its closure value. A caller must verify actual accepted samples and metadata
after close before advancing its own state, then use a completed checkpoint or transfer
receipt for durability.

New persistence encodes accepted pose components as IEEE bits. Restore preserves saved CURRENT
and revision exactly, settles previous=CURRENT, and does not replay historical motion. The
saved descriptor exposes authoritative bounded membership/source residency requirements before
binding restore; callers use ordinary scoped tickets and wait for actual readiness. Missing
alone is NotSaved. Earlier experimental disk records require an explicit developer recovery
migration that retains original bytes; this is data preservation authorized for this local
experiment, not a mod API alias or compatibility layer. Legacy JSON cannot recover precision
already lost by its earlier float encoding.

`CollisionChecked` is the default publication policy. `ExternallyResolved` requires the
existing server property `sunrise-live-world-external-pose-owners` to list the trusted owner.
The latter consumes externally resolved finite poses without solving, clamping or silently
correcting ordinary stabilization overlap. Identity, generation, residency, once-per-tick and
bounded motion/contact-consumer checks remain in force. The engine never feeds a canonical
host sample back into the caller's dynamics state. Nested projected destinations refuse.

## Common checkpoint budget and dirty terrain

A common save counts raw encoded WAL bytes, including metadata, before signatures,
region witnesses and checksum. The existing default common and per-record limits remain
unchanged. Clean ordinary terrain follows the existing World.modified policy instead of
being serialized again merely because it is resident. Live/raw/opaque block-entity data,
chunk attachments, current or dirty schedules, active membership and transfer columns
remain conservative terrain participants. Full column readsets, actual entities and
deletions, ordinary POI resolution, deferred/proto records, player data, level
attachments and projection metadata remain in the common decision. A never-POI column
follows ordinary no-record behavior; a last-POI removal still writes empty Sections.

The actual ChunkTicks owner tracks mutation since durable acknowledgment, including a
last stale scheduled occurrence being consumed without a block change. Checkpoint
readsets pin real queue revisions even when their packed list is empty. Only the exact
captured selected queue can become clean after all common acknowledgments succeed. A
post-capture transfer mutation remains conservatively dirty. Budget/refusal does not
clear modified terrain, queue revisions, receipts or held retirement plans. No new
queue, ticker or read authority is introduced.

The existing trusted simulation live-world inspection includes last_checkpoint_capture:
exact total/payload/witness bytes and record categories when the complete WAL was
measured, or an explicitly partial lower bound if frozen payload admission stopped
earlier. This is size admission, not a durable-success receipt. Oversized dirty
decisions still refuse before Prepared and retain live ownership; clean-terrain omission
is not a guarantee that arbitrary travel fits the common cap.

Internal Rust dependency surface changes: ChunkTicks adds two private revision fields
and doc-hidden persistence_revision, needs_persistent_save and conditional
acknowledge_persistent_save; LevelTicks delegates the conditional acknowledgment.
SaveDir's doc-hidden snapshot helper takes explicit selected terrain columns. The hidden
journal module adds scalar Usage, typed complete/partial budget errors, byte_budget_error
and Prepared::encoded_usage for the Game/lib boundary. No mod-vocabulary, catalog,
LevelRef receipt or WAL format changes. Exact-build SDK/native
consumers must rebuild; no binary compatibility promise.

Queue mutations add constant integer bookkeeping with no allocation or new whole-level
tick scan. Checkpoints still allocate full bounded readsets and entity/POI/frozen
metadata, and walk all participating resident columns. Exact envelope size accounting
visits borrowed byte lengths and witness metadata without cloning payloads to measure
them. No allocation-free, arbitrary-world or causal before/after performance claim
follows. Actual new tests, retained setup failures, source hashes and measurement
commands are bound in .local/live-world-evidence/checkpoint-budget.

## Remaining acceptance boundaries

The source tree retains an independent public native/wasm example under
`tools/fixtures/live_world_api`; generated SDK pins and binaries stay outside it. The normal
tool-generated projects must compile and load against the exact coherent build, and execute
in the one existing isolated review app/save. Genuine window rendering, physical crosshair/
mouse/menu input and full process moving-save/restart are separate gates from direct Game
and constructed-client routing tests. Late join and dedicated/LAN behavior are bounded by
the actual recorded routes; unsupported routes refuse instead of inventing a live owner.
The provenance/evidence records identify actual execution and failures. No #1022 completion,
upstream approval, complete block/BE support or latest-main compatibility follows from this
local experiment.

The preserved experimental base still contains legacy WASM catalog routes. Their actual
compile/load outcomes remain separate evidence for that base. The future proposal follows
the native direction identified by the coordinator's recorded official transport audit;
this continuation does not revive the subsequently removed transport.
