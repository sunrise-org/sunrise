# Local live-world client cost account

This is a source cost account for the local experiment, not an allocation or timing
measurement. The actual client regression verifies movement, packet ordering and emitted
model/outline positions; its elapsed test duration includes resource loading and is not
client throughput. Earlier empty/stable/inventory-only Game measurements exercise the server
route and do not measure this client change.

The connected client prepares the combined current collider index for its outer tick scope,
contact start and current support-age observation. A supported carried move requests the
current placement for its existing carry wave, and changed contact poses require an index
with the consumed start frames. These calls currently rebuild the combined projection index;
the ordinary entity base cache does not eliminate that work. Active projection ticks are not
claimed allocation-free. Empty projection handling returns the existing input index and
keeps no projection heap container, but adds scope/refcount operations.

Actual local shapes and prepared block models remain cached by existing block, membership
and resource revisions. Inventory-only and light-only updates leave those geometry caches
valid. Raw viewed state and full-column changes advance the existing World revision so lazy
coherence checks cannot reuse a stale state witness. Collision-equivalent state updates can
retain support by comparing the real ordered local box slices when immutable box identities
differ. Stable storage takes the Arc identity path. Real shape, membership or resource changes
settle the contact/presentation instead of borrowing a vanished support.

Per-replica contact and presentation records retain immutable box references, not another
cell/inventory store or an unbounded pose history. Shape rebuilds, box equality, replica/frame
lookup and combined-index work depend on the admitted cells, boxes, replicas and ordinary
entity colliders. Those count/work quotas do not constitute a complete process RAM or
wall-time bound; box-index expansion, retained generations, entity enumeration and allocator
capacity also matter.

The implementation seams are `Game::tick`, `tick_remote_player`,
`client_projection_contact_start`, `client_projection_contact_complete`,
`client_projection_draw_poses`, `projection_geometry`, `projection_mesh` and the existing
`moving_colliders::Prepared`/carry implementation. The ignored exact regression
`game::native_live_worlds::sync::carry_tests::client_tick_consumes_coalesced_projection_contact_before_ordinary_position_report`
uses the actual client tick, viewed-world packets, outgoing connection and emitted geometry.
The existing `native_live_worlds::perf_tests` and ordinary collider timing fixture are separate
observational controls. Use the same final artifact, resources and isolated application/save;
creating another benchmark world or cache is unnecessary.

A client performance claim needs an actual client capture with the ordinary controller's
profiling tools, paired with canonical fixture status and `simulation live-worlds` correctness
snapshots. Retain build/profile/hardware, resources, active geometry, residency, capture
settings and rejected publications. Sampled CPU time is not an allocation count. A causal
before/after claim additionally needs a matching baseline workload/profile; none is asserted
by this source account or by successful correctness tests.

See [the existing profiling tools](../../profiling.md) for capture and inspection commands.

The upright-footprint carry correction lives in `piston_push::carrying`. On a changed
support it may enumerate and sort actual support-face heights into reused thread-local
scratch, bounded by the body's existing step height. It shares the prepared current index
by Arc clone rather than constructing another placement index. Admitted movement then
uses the ordinary collision route and actual current/downward inspection; that inspection
still creates ordinary entity-obstacle storage and is not allocation-free. The unchanged
no-moved-collider return avoids this per-rider work. A skipped gate avoids all later contact
flag writes. Zero-step bodies do not acquire an invented step allowance.

The matching ordinary static-floor controls distinguish zero-Y tiny-gap accumulation and
once-only landing with existing downward velocity. The actual signed/pivot/client and
authoritative ItemEntity/ArmorStand cancellation rows are correctness regressions; their
wall-clock duration is not a client frame-time or allocation benchmark.

The retained source8 timing JSON includes an older hardcoded fast-profile label. Its
actual Cargo compiler-artifact profile and command/environment are authoritative; the
separate `source8-timing-profile-erratum.json` binds that discrepancy to the raw output.
Do not infer a fast-profile measurement from that literal or rewrite the original log.
