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

The first implementation adds catalog-generated native/wasm handles for binding a
real runtime world, inspecting its authoritative cells, ray targeting, publishing
revalidated external poses and retiring the projection. It uses the existing
carrier contact kernel and the existing moving-collider/carry and transformed
block drawing routes. All inputs and work are bounded, and refusals preserve the
previous projection. Source and destination residency are checked without loading
or generating chunks inside an API call. Source blocks retain their one ordinary
inventory/ticker owner regardless of motion. Geometry and prepared model caches invalidate on source block and resource revisions,
including while the source is parked. Inventory-only writes do not rebuild the static
physics/model caches. Contextual and position-dependent shapes explicitly refuse;
source special rendering is currently restricted to real chest lid contexts.

A binding keeps its initial local pivot. Published poses with another logical pivot
are reanchored without changing any physical point; returned samples use the binding's
pivot. Linear pivot displacement and shortest quaternion motion are swept continuously
together with bounded conservative advancement. The explicit pivot is carried through
the existing frame/history path, so drawing, carry-age interpolation and collision
samples preserve the same pivot. An idle publisher collapses previous/current samples.

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

## Follow-up contracts required for enablement

Prepared multi-dimension transfer must durably journal both source and destination
before either ownership change. Binding creation/removal/membership must join the
same durable metadata frontier before any broader transaction claim. The supported ordinary
action and menu route above still needs genuine window/mouse validation and an independently
packaged native/wasm fixture; API calls and a constructed client Game do not prove that scene.
A vanilla display or marker is not an ownership substitute. Late
join, paused/ticket-refused source contexts, parked save and recovery need tests
on integrated, LAN and dedicated runtime paths. Unsupported routes refuse instead
of pretending a copied block/inventory is live.

The initial explicit checkpoint is a real save barrier followed by projection metadata.
It is not an atomic cross-owner or autosave journal. Before enablement, the journal must
cover ordinary saves and their failure windows as well as explicit checkpoints, source
attachments and prepared transfer/return recovery. Missing metadata is typed NotSaved;
corrupt/storage refusals never silently initialize another scene.
