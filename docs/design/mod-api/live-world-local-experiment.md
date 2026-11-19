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
inventory/ticker owner regardless of motion. Geometry caches invalidate when the
source world revision changes, including while the source is parked.

The implementation must report separately which runtime, save/reopen, action,
transfer, client tracking and graphical tests actually ran. Declared API shape,
geometry tests, and direct tests of the real Game are different evidence.
No #1022 completion claim follows from this slice.

## Follow-up contracts required for enablement

Prepared multi-dimension transfer must durably journal both source and destination
before either ownership change. An action must carry owner/generation/cell/pose
revision and enter ordinary permissions, reach, use/mining and menu rules with a
projected interaction context. A new synchronized projection owner must negotiate
its protocol; a vanilla display or marker is not an ownership substitute. Late
join, paused/ticket-refused source contexts, parked save and recovery need tests
on integrated, LAN and dedicated runtime paths. Unsupported routes refuse instead
of pretending a copied block/inventory is live.
