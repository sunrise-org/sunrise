> 🤖 Opened by OWNER_LOGIN's AI agent

OFFLINE DRAFT ONLY. No PR or claim was opened. Replace OWNER_LOGIN only before
an owner-authorized eventual submission.

Refs #1022

- [x] feature
- [ ] parity
- [ ] breaking

This local experimental foundation binds an actual mod-owned runtime Void level
to a sparse moving projection. Its ordinary block entities, inventories,
attachments, schedules and server contexts remain authoritative. External pose
samples use the existing bounded contact kernel and moving-collider/carry route;
local model faces are cached by source block/resource revisions.

Layer: engine extension point and generated platform handles. No new simulation
dependency, content identifier, dynamics integrator or game/launcher installation.
Source inputs and pending upstream coordination are recorded in
`live-world-provenance.md`. This is not an upstream spec-approval or clean-room
claim. The complete issue acceptance scene has not passed.

Validation: `cargo check --profile fast --locked --offline -p sunrise-client
--no-default-features` passed for the initial foundation. Actual Game fixture binary compiled and listed the three intended tests. Its first
isolated run passed both unit tests and reached real runtime ownership, sparse
clip and pose/hit checks, then failed an incorrectly server-routed drawing
assertion: server-only resource loading does not populate a client model catalog.
The server fixture now scopes itself to real inventory/ticking/save/reopen and
awaits its next run. Client tracking, window rendering, actions and durable
cross-owner transfer validation remain pending. Do not replace their status with
a compile-only claim.

Tried in game: pending actual local runtime evidence.

Performance: no projection means an empty-table branch. Stable local collision
geometry and prepared model faces are cached on source block/resource revisions;
normal inventory-only changes do not rebuild those caches. Per-owner and
aggregate reservations constrain active memory and contact work.
