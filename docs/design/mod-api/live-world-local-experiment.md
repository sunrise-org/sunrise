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

The implementation must report separately which runtime, save/reopen, action,
transfer, client tracking and graphical tests actually ran. Declared API shape,
geometry tests, and direct tests of the real Game are different evidence.
No #1022 completion claim follows from this slice.

## Follow-up contracts required for enablement

Prepared multi-dimension transfer must durably journal both source and destination
before either ownership change. An action must carry owner/generation/cell/pose
revision and enter ordinary permissions, reach, use/mining and menu rules with a
projected interaction context. Continuing vanilla chest/furnace menus and ordinary client mining/use input must retain
the source owner/generation/cell identity and revalidate transformed reach on each click
and tick. Retirement/unload must close them through ordinary cursor/inventory cleanup.
A vanilla display or marker is not an ownership substitute. Late
join, paused/ticket-refused source contexts, parked save and recovery need tests
on integrated, LAN and dedicated runtime paths. Unsupported routes refuse instead
of pretending a copied block/inventory is live.

The initial explicit checkpoint is a real save barrier followed by projection metadata.
It is not an atomic cross-owner or autosave journal. Before enablement, the journal must
cover ordinary saves and their failure windows as well as explicit checkpoints, source
attachments and prepared transfer/return recovery. Missing metadata is typed NotSaved;
corrupt/storage refusals never silently initialize another scene.
