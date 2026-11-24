# Exact inputs: local live-world experiment

Status: local experimental, unapproved upstream. Base Sunrise commit:
`d4be68317b0c86f1409d8cce65d467ddd40f68ed`. Development branch:
`local/live-world-phase3-continuation`. This is not a clean-room claim.

## External inputs

- Assignment from the local owner/coordinator: generic live sparse worlds,
  external poses, authoritative actions, transfer, persistence, rendering and
  moving contacts; no mod-specific simulation, dependencies or identifiers.
- Public Sunrise issue [#1022](https://github.com/sunrise-org/sunrise/issues/1022),
  body and comments fetched 2026-11-19. The comments response was empty.
- Public Sunrise open-PR search for `1022`, then `moving structures collider runtime
  level`, fetched 2026-11-19. Both returned no results. This is a search result,
  not proof that overlapping public work does not exist.

No other engine/mod source, neighboring port source, archive, reference checkout,
foreign source fetch, or simulation dependency was read or used. The linked
historical issues and foreign source links in Sunrise's own documents were not
opened. Only the Sunrise development worktree's own source and documentation
were inspected. Existing Sunrise source remains under its existing licenses;
no foreign code was copied or adapted.

## Sunrise inputs at the base commit

The exact source inspection list is maintained in `live-world-inputs.txt` next to
this record. It includes instruction/design files and each source file whose
contents informed the experiment. Filename and symbol searches were limited to
this Sunrise worktree. The implementation reuses Sunrise's runtime levels,
world handles, carrier contact geometry, moving colliders/carry, transformed block
models and existing durable storage; reuse is not an original-provenance claim.

## Offline coordination draft

> Proposed partial work, Refs #1022: a runtime-dimension-backed live projection
> with checked membership/poses and existing collision/drawing. The local owner
> authorized an experimental prototype before claim/spec approval. No claim was
> posted. Before upstream submission, confirm overlapping work and ask maintainers
> to approve the route, especially projected actions and cross-level journal format.

This text is a draft only. No issue, comment, claim, pull request, push, message
or publication was made.

## Build inputs and environment

The owner separately authorized acquiring existing `Cargo.lock` packages into the
shared Cargo home for compilation. Cargo used its locked dependency source and
the worktree's already pinned vendored crates; those implementations were not
opened for research, copied or adapted. No dependency was added or repinned.
The single target is the owner-assigned `sdk-target`; the stable toolchain and
prepared resources are the owner-supplied build/runtime inputs. Offline checks
first refused missing cache entries; an authorized network escalation acquired
the locked packages. Public issue/tool results were read only.

## Local validation record (foundation)

- Initial vocabulary check passed; the initial gameplay check passed.
- The actual library-test target compiled with the command-local
  `profile.fast.package.sunrise-client.opt-level=0` override. Existing dependency
  compilation remains in the single `fast` target.
- `--list native_live_world` listed exactly the two geometry/budget unit tests and
  the real Game runtime fixture. The first isolated execution ran all three: both
  unit tests passed; the real Game reached runtime creation, holder/sparse clip and
  pose/hit assertions, then failed the server-only drawing assertion. The failed
  log is preserved in the temporary validation run, not discarded or called a pass.
- The real server fixture is being corrected to exercise actual source ticker,
  inventory and close/reopen persistence. Separate-client/window proof is required
  for drawing. Unintegrated sync drafts are excluded from the foundation commit.

## Continuation inputs and validation

The coordinator reported public main `4f80f416ae19fc60cc2125b54ec4ff0fde713a1e`
as observed on 2026-11-19. This experiment remains on its recorded `d4be683` base;
no rebase, newer source compatibility claim or inspection of the reported newer
implementation was made. The local base has no `.claude/skills/sunrise-mods/SKILL.md`;
that missing-path probe supplied no input.

The dependent contact-kernel commit adds continuous simultaneous linear/angular
sweeps using the existing conservative advancement machinery. Its actual
`carrier::contact` test run passed, including the prior pure-motion regressions,
closing/sliding floor contacts, a thin mid-interval obstacle and budget refusal.
This is geometry evidence, not a window, player-action or transfer milestone.

The public-tree pre-commit hook could not complete its whole-index fingerprint
scan: Git tried to fetch missing promisor blob
`e959a076e9fa4b8c1fcf990b2253a935da5bb5f4` and sandbox DNS failed. The dependent
local commit records a staged-path-only audit with the checker’s existing path
rules and disabled hook execution for that commit. No full fingerprint pass is
claimed. The coordinator offered an independent authorized network audit; its
outcome remains separate from this record until reported.

The coordinator subsequently completed the exact public-tree checker with authorized
network access to the existing promisor remote, then repeated it offline successfully
at `f687661`. No source contents from those missing blobs were supplied as inputs here.
An earlier entry described `9da9109` as running the ordinary pre-commit checker.
That description does not establish an installed automatic hook: the later `a944343`
receipt recorded `core.hooksPath` unset and an explicit full checker pass. Only retained
invocations establish execution; hook presence is not inferred from a successful commit.
The earlier failed-hook receipt remains part of the history.

Actual isolated Game action evidence is retained in the run named
`sunrise-live-world-proof-38nr4tfp`: owner admission cancellation, callback inventory
rollback, real placement/consumption/membership, another owner's nearer wall occlusion,
break callback mutation/retirement refusal and ordinary break passed. In that same run,
the runtime fixture passed live chest/furnace and saved close/reopen content assertions
but refused final projection restoration while the source awaited its real entity
residency tick. It was not recorded as a complete fixture pass.

The subsequent isolated run `sunrise-live-world-proof-yo231dzu` passed the complete
current Game fixture selection. The runtime child completed the ordinary frozen
entity residency pass and restored a new projection generation after reopening
its real source. Its structured evidence reports the actual chest and furnace
values; it explicitly labels client drawing as a separate pending route. The
run receipt records the test executable SHA-256, arguments and prepared resources.
This is not full process-restart, menu-lifetime, window or transfer-journal evidence.

## SDK exporter dependency inputs

The normal SDK export exposed an inactive-architecture dependency refusal in the
base's `tools/sdk_api_source.py`. The focused tool correction reads Cargo's own
host-filtered dependency graph and preserves each target table and manifest alias;
it adds no dependency, pin, source bypass or architecture-specific exception.
The unfiltered graph remains the source packing closure for all supported targets.

The diagnostic used this worktree's exporter, prebuilt helper, their Python tests
and Cargo manifests/lockfile. Cargo metadata and tree output are build inputs, not
foreign implementation research. The current official-head status was reported
by the coordinator only; this worker did not inspect that newer implementation.
The focused unit command is `python3 -m unittest tests.test_sdk_api_source` with
a Python supporting `tomllib`. A separate no-compilation replay normalizes the
actual exported manifests against Cargo's host graph and game feature tree.

The broader prebuilt test selection was also executed with the immutable prior
exporter bytes loaded into a temporary module. It reproduced the same failures
and environment refusals observed with the correction. These failed runs remain
failed receipts; no broad-suite success is claimed. Exact logs, commands and
source hashes live in `.local/live-world-evidence/sdk-target-fix/` for the
coordinator to preserve outside this worktree.

The continuation's earlier phrase about an ordinary pre-commit run does not
establish persistent hook configuration. The retained checker result is the
asserted evidence. At `a944343`, `core.hooksPath` was observed unset; the full
indexed public-tree checker was executed explicitly and passed. Later local
commits invoke `.githooks` explicitly rather than assuming that configuration.

A separate generic dependency-identity regression covers an unrenamed package
beside a renamed version of that same package, in both extern orders and with a
library name different from its package. The exporter must use Cargo's actual
library target and refuse ambiguous identities. The correction is a separate
tool-only continuation, preserving the tested `a944343` history.

The normal `a944343` SDK build, packing and index succeeded. Its exported-source
consumer gate completed the compile commands but failed the final strict API
closure comparison. Authorized generated SDK logs and Cargo metadata identify
transitive game feature unification absent from the exported API workspace;
this is not a fidelity pass. Investigation uses Sunrise's own release matrix,
manifests, tool code and Cargo's dependency metadata, without opening dependency
implementation source. Native target differences and wasm must remain separate
ordinary Cargo conditions in the one portable API-source archive.

All local Git signoffs remain local authorship records. Human DCO eligibility,
review and upstream claim/spec approval remain pending; the experiment and its
offline PR drafts do not assert those approvals.

The portable transitive-feature correction derives normal-library edges from
`cargo tree --no-dedupe`, excluding host proc-macro subtrees. Platform-filtered
metadata alone is insufficient: it can retain inactive dependency kinds on a
surviving edge. Metadata supplies exact package, version and library identities;
the actual target tree supplies active edges and feature unions. The release's
own `sdk-archives` listing selects native legs. Registry declarations already
inside each API root's closure are pinned per exact native target; existing
direct names are reused because Cargo refuses duplicate names for one package
version. The original wasm and other-target dependency declarations stay intact.
No dependency implementation is read or copied.

The no-compilation manifest probe uses the normalized manifests and empty Rust
target placeholders, not a copy of the game or a substitute runtime fixture. It
resolves each published native target through real Cargo and compares complete
package and feature sets for both API roots. Its wasm resolution checks that the
native feature pins do not activate there. Initial probe failures (an overbroad
platform assertion and duplicate dependency names) are retained alongside the
corrected run. These are graph/manifest checks; official generator, consumer
compilation and runtime evidence remain separately reported. The ordinary SDK
consumer gate now requires exact closure and feature equality for both roots.

## Fresh Sunrise-only helper inputs

The coordinator explicitly assigned two fresh-context AI helpers for bounded same-source
work. `/root/sunrise_input` supplied temporary input/menu drafts, their own-source list,
integration notes and hashes. `/root/sunrise_storage` supplied temporary WAL/gate/snapshot
and filesystem-test drafts, exact source hashes, integration order and limitations.
Both were limited to this Sunrise tree, its rules and Sunrise-authored scratch drafts;
neither edited the shared tree, compiled it, ran a game, contacted maintainers or supplied
foreign implementation inputs. The lead remains the integrator. Their parser/patch checks
are not runtime evidence. Original handoffs and subsequent actual integration receipts are
retained separately under `.local/live-world-evidence/`.

The coordinator also reported relevant newer official SDK work at main `94256510f74e6843754e0a93fd9eae4214c8a08737d273`:
merged #3472 uses a different transitive subset policy, open #3468 overlaps the inactive-target
fix, and open #3478 clarifies that policy. This is coordinator-reported public audit input,
not an inspection or incorporation of those newer implementations here. The retained local
strict target-specific SDK experiment requires reconciliation before any upstream proposal.
The pinned development base and earlier failed export receipts remain unchanged.

All implementation and draft text in this local series is AI-authored. Local signed-off
commits do not certify human DCO eligibility, completed human review, upstream spec/claim
approval or redistribution eligibility. Those decisions remain pending; nothing was posted.

## Input/menu, retention and publication checkpoint

The integrated same-source input/menu handoff was actually compiled and exercised through
resource-backed Game child processes. The failed first batch remains retained: it exposed an
incoming-versus-outgoing engine-channel admission error, a physical actor dimension lost in
source scope and an incompletely started scheduler fixture. The corrected snapshots also
exercise ordinary held-block packets, exact physical blockers, continuing menu cleanup,
source versus parent sentinels, actual source activation and targeted metadata/pose rollback.
A later snapshot adds pre-effect Busy refusal for caller-held projected use transactions and
genuine current/parked/empty/foreign/stale level revision reads. The actual standard generator
rejected an initial Option-shaped new read; it was corrected to existing LevelReadError with
no expansion of the shrinking unwinding allowlist. Native/Game and wasm checks and actual
listed tests are separately bound to their source snapshots under the local evidence folder.

The real viewed-level hub test initially could not bind a local loopback socket in the
sandbox. Its authorized loopback-capable rerun passed. That OS refusal was not an automatic
approval rejection, a logic failure or a graphical client proof. The client Game's independent
open/close routing test is a separate recorded result. The coordinator independently replayed
the exact test binaries and reported matching results; those are separate receipts, not an
inference from code compilation.

WAL/save-gate/transfer payload drafts and the next public contract remain unintegrated at this
checkpoint. Neither the existing metadata save nor the successful source transaction fixture
is represented as atomic durable cross-owner transfer. Multi-source publication rollback,
exact rotating persistence, actual prepared transfer/recovery, normal native/wasm packaging,
window input and measurement remain distinct required gates.


## Common durable ownership continuation

The next batch integrates the same-source WAL, SaveDir gate, immutable player Snapshot
queue, actual payload/schedule handoff and Game frontier/fence drafts. Their original
source-only parser results remain original handoff receipts; subsequent lead integration
and executed tests are separately bound. A later bounded helper exclusively integrated and
tested the three lower playerdata files; all Game orchestration stayed lead-owned. Its
first resource-less run skipped old resource branches, then the exact final binary reran
with the approved resources and empty stderr. The observed Game-discovery build-script
inputs were archived, so this is lower-layer evidence, not a full-tree SDK claim.

The coordinator supplied generic Sunrise-only source-navigation observations, including
that vanilla BE membership uses a validated template/factory rather than mod-declared
`type_for`, and that a retirement receipt must compare its actual post-handoff revision.
The coordinator has a broader prior context; these observations do not establish clean-room
or approved upstream provenance. No foreign implementation or personal simulation source
was supplied or consulted. The recorded development base remains unchanged.

Actual lower runs exercised journal recovery after torn selected region data/header, strict
unselected-sector and external-stream witnesses, pre-Prepared readset refusal, empty-level
metadata and the immutable player queue. Later Game attempts exposed a missing-region delete
acknowledgment bug: the ordinary writer correctly leaves a never-existing empty Entities
region absent, while the witness reader had demanded a file. The correction accepts absence
only when the original region never existed and every selected afterimage is deletion; any
expected payload or formerly existing region stays strict. A dedicated actual ordinary
SaveDir writer regression accompanies that fix. Newly added lower rows still require their
final focused execution; prior passing lower receipts do not cover later edits by inference.

The actual Game `durable-backend/check-7` batch failed all selected rows. Later batches
retained each failure and correction rather than changing prior status. `check-9` and
`check-10` each passed the other selected cases but failed the public transfer row, first at
vanilla factory admission, then at a saved-tick sentinel. Ordinary chunk registration
retains Some(empty) without registering an unpack wait; rejecting that empty sentinel made
valid empty destinations permanently Busy. Capture/install now distinguish no work from a
real nonempty pending list. The actual core ownership tests include empty source/destination
sentinels, import and exactly-once execution while preserving nonempty pending refusal and
queued/held duplicate behavior. An instrument-only fixture privacy error in `check-12`
remains a compile failure, not runtime evidence.

`check-13` reached the first real journaled three-cell payload handoff and third-owner
metadata, then failed its retirement-count assertion because receipt code compared to the
pre-transfer revision. `check-14` exercised forward/reverse payload handoff and the dry slab
lifecycle, then exposed an inconsistent synthetic hot-furnace seed (unlit block with a
burning BE). That fixture was corrected to the corresponding real stock LIT state; ordinary
furnace behavior was unchanged. The complete exact public transfer case passed in `check-15`:
occupied second destination full refusal, codec-root abort, actual shared receipt/metadata,
real inventory/counter preservation, both transfer directions, dry nonfull state and a hot
furnace's next-global-tick progress with one owner.

The full `check-15` family passed all but the older action row's stale-handle expectation;
client viewed-level capacity and parked readiness passed. The refusal already preserved
blocks/player state. The new required binding journal also correctly restored callback
retirement on that refused ordinary transaction, so the old expectation of permanent Stale
was inconsistent with the new atomic contract. The updated row requires exact sample,
binding and membership restoration. Its final execution, the narrow trusted-pose admission
row and the required explicit-base pr_ready run remain separate pending gates until their
actual receipts exist. No failed family is retrospectively marked green.

The actual OS write-refusal Game test demonstrated a durable Prepared record, immediate
session fencing, no following ticker/save publication, and another process recovering before
ordinary source World/BE loading. The rotating checkpoint/reopen row preserves saved CURRENT
component bits and revision across repeated cycles, with previous=CURRENT settlement. This
is distinct from preserving the historical previous sample in opaque caller audit metadata.
The cache account is observational on one recorded fast profile/aarch64 macOS run with
paused ordinary server turns and exact geometry-cache identity across BE-only changes. It
is not a matched before/after benchmark or a completed interactive-window test.

The independent authored native/wasm example is retained under tools/fixtures/live_world_api
without generated manifests, self-commit pins, SDK source or binaries. It reuses the one
existing isolated application/save and a fresh fixture namespace, preflights every reserved
parent cell before any marker/seed, verifies exact restore semantics and reads actual sample
and encoded metadata back after root close before advancing caller state. Normal SDK
compilation, loading, mouse/window and moving-save full restart remain owner-coordinated
separate gates; this source-only artifact is not their pass claim.

The owner supplied exact stable wasm std and rust-src components as ordinary build inputs;
coordinator receipts bind their official checksums and unchanged compiler/compiled-library
hashes. No std/dependency implementation was opened for algorithm research. One helper used
the existing owner-provided opaque environment launcher, recorded its target override before
Cargo, and corrected that inside its child; its source was not opened. Lead builds invoke the
preserved compiler/Cargo directly, sharing the one assigned cache and target.

Current experimental disk-v1 preservation is an explicit retained-original developer recovery
operation, authorized by the owner. It introduces no old mod API alias, shim or old-symbol
fallback. Earlier JSON cannot retroactively recover lost floating bits. AI authorship, local
signoff limits, pending human DCO/eligibility/review, claim/spec approval and no-public-action
status remain exactly as recorded above.


The final frozen lower snapshot `durable-backend/final-lower` compiled with no source changes
during the build. Its actual filesystem selection passed all rows, including the newly added
migration-original, expected-missing strictness and actual ordinary-writer missing-delete
regressions. Its full playerdata selection reran with real resources and empty stderr; the
explicitly ignored large aggregate row remains unexecuted. Native and wasm vocabulary/API
checks both completed successfully against that same frozen source. Exact commands, target,
compiler/resource and binary hashes live in that snapshot's structured receipts. These are
internal generator/lower-layer checks, not normal generated-project or graphical loader proof.

The pre-commit source checkpoint deliberately adds the corrected exact rollback assertion
and the trusted external-pose policy row before the required explicit-base pr_ready build.
Their actual later execution and pr_ready's complete output must be retained separately;
source presence or earlier lower/client checks do not establish those results. Normal SDK
and interactive app/save acceptance remains a subsequent, separately identified local stage.


## Explicit-base readiness attempt and bounded validation fixes

The required `tools/pr_ready.py --base d4be68317b0c86f1409d8cce65d467ddd40f68ed
--quick --verbose` was actually invoked on clean `ba3bc9`, with the shared target/home,
incremental disabled, bounded jobs and dev/test debug information disabled. It finished
nonzero with its source unchanged. Its static/tree/layer/format and doc selections passed;
its first Python run used this host's unversioned Xcode Python 3.9.6, below CONTRIBUTING's
prerequisite. That environment mistake is retained as a failed run, not a baseline or pass.
The preserved Rust compiler remains the owner-pinned 1.99.1; the tool's calendar-derived
warning about newer CI Rust does not explain or waive actual introduced diagnostics.

The real full-target run exposed missing new hook types in the dispatch-only compile
fixture, an internal export placed outside its required area fragment, and old no-projection
mining/outline snippet assumptions. Those existing fixtures now account for the new surface
while retaining their actual ordinary-path assertions. Runtime ownership/action claims still
come from real Game tests. Hidden engine constructors/backends now explain their purpose and
the borrowed level adapter is explicitly non-exhaustive. Clippy findings were corrected
without suppressing checks. The focused modern-Python boundary/mining/outline selection and
actual generated dispatch suite passed; their exact outputs remain separate receipts.

The first selected Game unit run passed its other cases but failed the existing
`a_non_unicode_package_folder_keeps_its_exact_path` at directory creation. The coordinator
replayed the same binary both in the sandbox (OS permission refusal) and with authorized
execution (host Illegal byte sequence), before package/API code. It verified that source
file against the immutable base. This is a concrete host filesystem limitation; the test
was neither changed nor called passed. Because Cargo stops that selection at its failure,
remaining selected test crates must be executed explicitly if the final run stops there.

A complete subsequent Python 3.14 sweep ran with fresh canonical temporary paths and homes.
It exposed a remaining outline source-marker assumption, subsequently corrected with its
real extracted-filter test passing. Other retained failures include missing Linux/GNU CI
prerequisites, sandbox process/socket restrictions and sparse roadmap inputs. The coordinator
expanded only the existing pinned worktree's roadmap data and supplied a real local ISC
Darwin flock CLI as a normal tool input; its implementation was not read by this worker or
incorporated in engine code. Standard system tool directories are restored in the final
PATH. Unknown or separately reproduced prebuilt/tooling failures remain explicitly
unclassified until supported; no broad Python or workspace pass follows from focused checks.

The final public transfer row is extended to two real payload destinations, queued-close
staleness before any first-destination mutation, common third-owner metadata and return.
The independent example chooses an explicit bounded common-afterimage byte budget rather
than assuming that the moved-cell payload alone determines checkpoint cost. These additions
are coverage/configuration of the existing contract, not another ownership framework.
Their final actual execution belongs to the next recorded Game artifact. All earlier
failed runs, unchanged-source bindings and coordinator replay receipts remain historical.

The subsequent clean `4e30476652` explicit-base readiness invocation finished nonzero with
its source unchanged. Workspace/server Clippy, static checks, doc tests and generated dispatch
tests passed. The selected Game unit failure remained the separately reproduced non-Unicode
directory-creation limitation; the complete Python selection remained nonzero. Its exact
output also exposed one tuple-only rustfmt difference, corrected without changing the test.
Actual ignored Game executions then passed the extended two-destination transfer and ordinary
action rollback rows. The new publication-policy row instead returned Unavailable before
reaching its first policy decision; the coordinator reproduced that same failure independently.
That fixture now uses the established opened-world scratch root, session lock, aligned SaveDir
and world-ready setup, with explicit owner/phase assertions before the public call. This is
a test-context correction pending its own execution, not evidence that the policy passed.
The full readiness failure is retained, and unchanged Linux/Python selections are not rerun
or relabelled green because of this bounded fixture/format correction.

The complementary selected-crate execution reached the real client-lib suite and exposed
an old configuration-wire assertion expecting the client's former two-level advertisement.
It now asserts the intentional four-level advertisement; separate old-peer two-level and
fifth-level refusal coverage is preserved. The unchanged Unix-socket path-limit test also
failed with the deep evidence TMPDIR and sandbox bind restrictions. Its exact existing binary
and test passed with fresh short temporary paths under authorized local execution outside
the sandbox, without changing the test. Both failed and successful environment receipts are
retained. Cargo's stop still requires executing the remaining package selection explicitly.

Those complementary selections were subsequently executed through every remaining package
and integration target, using no-fail-fast for the last selection. The unchanged renderer
crash-report assertions failed because their captured stacks lacked the required source-file
details under the recorded debug-zero profile; exact source equality with the base is retained,
but it is not a baseline behavior pass. A package-only debug-information rerun is separate
evidence. The world selection also reached an existing invalid-UTF-8 directory creation and
received the same concrete host Illegal byte sequence error. The complete protocol unit target
aborted with a stack overflow before the live-world test names; that remains a nonpass without
an invented cause. Both new live-world protocol selectors were then listed and executed on
that exact binary successfully. The full and complementary readiness failures remain intact;
none is replaced by the focused runtime or protocol results.

## Post-SDK generic lifecycle and destination-context follow-up

The coordinator reported that normal SDK generation, strict consumers and packaging passed
for clean 860fd62195. That is separate from actual play. A later ordinary quit reported the
common-flush refusal despite prior acknowledged checkpoints. No personal source, outputs,
coordinates, poses or algorithms were supplied to this worker. The own-source trace found
finish_server_save's unconditional independent flush after guest teardown while common
SaveDir leases intentionally remain required. The candidate uses a final common decision
after leaving-player snapshots and checks its post-notice cut; it never weakens direct flush
admission or post-Prepared fencing. A real Game shutdown regression is authored for projected
container/furnace preservation, a post-initial-save guest inventory update, late SaveToFile
mutation refusal/fencing and the unchanged ordinary-world route. This is uncompiled candidate
work at the time of this entry, not a successful shutdown or recovery claim.

A separate generic source audit found ordinary destination terrain queried through the
source-admission helper. The candidate calls Sunrise's existing actual World/position collision
query with an empty entity context, preserving its Result instead of defaulting errors to air.
Source admission remains conservative; residency and candidate/chunk/box admission stay in
place. The actual Game candidates exercise water versus stone, positioned bamboo, unsupported
water in Source, and budget/residency refusal with retry. Syntax/applicability checks alone do
not certify them. An initially suspected client menu failure was cleared by the coordinator's
longer asynchronous window probe; no client action fix or negotiation-defect claim followed.
An independently observed standing-carry concern is assigned to a Sunrise-only source helper;
its eventual draft and runtime evidence are separate from these two candidates.

The carry helper read only Sunrise's own source, rules and authored design/fixture. Its final
source-only v2 patch (SHA-256 dd94a102006a187796caa06b9ce217fc00a63bf956d5870841a933a3cac7d63e)
was integrated for review. The trace distinguishes the last contact pose consumed by a client
player tick from the latest network sample, and places the existing collided carry before
the ordinary outgoing position/support-age report. Public samples remain unchanged. Its
actual-client packet regression is authored, not yet executed; its source-only provenance
and remaining graphical interpolation limits are retained separately. No personal source
or solver values were supplied, and the helper's syntax/applicability checks are not runtime
evidence.

The first combined executable built successfully with its source unchanged. Its ordinary
shutdown row then passed; the destination row passed its water child but stopped on an
incorrect fixture assumption that default bamboo had registered positioned collision, and
the client carry row stopped before carrying because its synthetic configuration omitted
the viewed level's dimension-type registry. Both failures are retained. The destination
fixture now uses an actual stock moving-piston entity in the ordinary owner and contrasts
the same moving-piston state at a position without that entity. The source whitelist still
refuses the dynamic state. Inspection of the already prepared collision tables found no
declared shape-offset rows; no new reference source or exporter acquisition was involved.

The helper's subsequent private presentation addendum (SHA-256
820609cce9a0ce7ea6790f4a9acdb6d7fbd98ef4448de0c24f1cbec73e44dc62)
was integrated after that run. It keeps client rendering on the completed local contact tick,
without changing public samples or action authority, and compares actual immutable local
collision boxes when a state witness changes. The extended actual-client candidate checks
emitted camera-relative geometry, collision-equivalent furnace state changes and ordinary
viewed chest data. These additions remain unexecuted at this entry.

The extended client/configuration and real moving-piston destination candidates passed
the subsequent type check with unchanged source. Before the next executable build, the
helper's visual-outline amendment (SHA-256
8ec6e0d1dedbcb1ba8d17e6017c5c2d6d2df36dbfb7e37ce6620b5702aba05e3)
was integrated so visible models and the pick outline share the same private presentation
pair. Picking and authoritative hit/action samples are unchanged. That additional visual
route is pending compilation and execution at this entry.

The next executable again passed the normal shutdown and all destination-context children.
The client row passed the coalesced contact, emitted models/outline, packet ordering, idle,
jump and sparse-hole checks, then failed the later furnace-state transition. Own-source
diagnosis found that ordinary viewed BlockUpdate/column storage did not advance the existing
World revision used by the observed-state/geometry/model caches. The helper independently
confirmed this trace and supplied the narrow raw-store invalidation patch (SHA-256
9805c6d3f522025e3fa093c6b5b2454ff858f936e841ed405e7e31f89ce9de27).
It advances that existing revision on real state/column changes while retaining the viewed
world's existing mesh path, and does not advance it for duplicate state, absent unload,
block-entity-only or light-only data. No relighting/simulation route or second state owner
was introduced. Its actual-client assertions remain pending the next executable.

Source-only cost review also records that the current client route constructs combined
projection placement indices more than once per tick, despite reusing actual shape/model
caches and immutable box storage. No zero-allocation or new client timing improvement is
claimed; the earlier server fixture measurements are not a benchmark of this changed route.

The final follow-up artifact 648d0a91bc005ffd2b0411e46891b44aabe4942ade3cd7f62282181f8d4a3584
built with its exact source unchanged. Actual execution passed the complete listed live-world
family (new/ignored/pure selections), ordinary moving-collider carry, ordinary no-projection
client fall/rider and negotiated view-cap controls. The client row also exercised duplicate
state, BE-only, light-only and full column insertion/replacement/removal revision behavior.
All commands, raw source snapshots, resource/environment bindings and stdout/stderr remain
under post-sdk-fixes/build-3; failed check/build/runtime iterations were retained separately.
This is not a normal SDK/window or client allocation/timing claim. Source-only helper cost
account c00acdb609f56d14e73fbc8b8cef34f4d8e7457cf65903d58d646753e0f6df0a and delivered
patch/receipt lineage are retained in the bounded evidence bundle; an unused resource-free
alternative is explicitly not applied. Final format/changed-crate Clippy/public-tree outcomes
will be appended after their actual execution.

Final format_scan passed. Strict changed-crate Clippy (`sunrise-client --lib --tests`,
`-D warnings`) passed with the complete frozen source unchanged and no Rust diagnostics.
The indexed public-tree and explicit pre-commit-hook executions are recorded at the local
commit boundary, rather than inferred from an unset global hook configuration. The earlier
full explicit-base readiness failures remain nonpasses; this follow-up did not repeat the
unmodified Linux/Python readiness universe or relabel it green. All source-only incidental
search output stayed in Sunrise's own repository; no reference/other-engine/private port
implementation was read or used.

## Upright carried-body and retired-recapture follow-up

A coordinator-supplied generic runtime observation prompted own synthetic ordinary-client
horizontal-axis rotation tests. No private caller source, coordinates, solver algorithms or
reference implementation values were read. The original own 744 artifact reproduced
uphill upright-body embedding followed by idle fall-through. Its real two-process retired
return/recapture regression separately passed; that does not classify another caller's
Revision refusal. Expanded ordinary-scheduler coverage also verifies that global/parked
block writes refuse and the actual Source callback owns state-bearing seed writes.

The bounded correction computes actual current support-face clearance within existing step
height and uses the existing one collided Carried invocation. Source-only reviews rejected
a forbidden direct collision resolver and an early zero-contact-delta return; their raw
candidates remain retained. Pure contact inspection uses the existing collision query
instead. A zero-centre-contact pivot can still need uphill footprint clearance.

Own ordinary no-projection controls established both the unchanged zero-Y/tiny-gap ground
transition and once-only landing through the measured reversal gap with downward velocity.
The signed regression's overly strict later carried-position expectation was aligned only
after those actual controls; original failures remain failures. Exact first rises/pivot,
nonembedding, horizontal stability, post-landing stability, no net rising-height loss,
ground/packet/hint behavior and ordinary jump remain checked. No Java golden or new
geometric tolerance was introduced.

Own 4294 execution then reproduced a separate existing-contract violation: one actual
FlowCancel preserved ItemEntity position/velocity/other flags but changed on_ground from
true to false. The coordinator independently repeated that failure and the ordinary
control, then retained the exact executable. A previous Mob test masked the ground setter
because that kind does not implement it. The fix propagates the existing single gate's
run/skipped outcome through generic, player and armor-stand delegates; Skip/error prevents
the carry caller's post-move contact write. Real ItemEntity and ArmorStand rows preserve
the complete actual pre-invocation tuple with exactly one callback.

Frozen source8/build6 compiled the Game and affected client-lib test targets with unchanged
source. The new focused rows and historical selected controls passed; exact commands, raw
sources, profile/resources and binary hashes live under upright-carry-candidate. One wrong
client-lib selector executed zero tests and was correctly refused by the driver; it remains
an immutable nonpass. A separate corrected exact selector actually ran the stand control,
and actual-corrected-final-6 records the explicit composition rather than rewriting history.
These are direct constructed-client/Game and library observations, not a rebuilt SDK or
physical-window claim. Final focused lint/format/index/hook outcomes are recorded separately
after execution; historical whole-readiness and legacy WASM-load failures remain nonpasses.

The canonical fixture-only follow-ups remain separate commits. The coordinator reports
normal native fixture18 compilation/packaging and its independent multi-process runtime
sequence; the legacy WASM artifact compiled but failed during ticket-controller return
decoding before opening its Source. A later coordinator read-only official audit reports
that current main removed WASM mod support; this local native-oriented continuation neither
revives that transport nor silently rebases the preserved experimental base. No public
coordination or new upstream-approval claim follows from either result.

The source8 performance row's raw JSON also retains an inaccurate hardcoded fast-profile
label. The separate timing-profile erratum binds the original log to Cargo's actual
Game/client-lib test-target profile (opt-level zero, debug information disabled,
debug assertions/overflow checks enabled). The original output is not rewritten and is not reported as a fast or client
throughput benchmark. No additional executable build was made just to change that label.

Strict focused Clippy then passed for both sunrise-client and sunrise-client-lib lib/tests
with -D warnings, with the complete frozen Rust source unchanged and no Rust diagnostics.
The coordinator independently repeated the signed/pivot, ItemEntity/ArmorStand cancellation
and targeted ordinary lower movement controls. Final formatting, ignored-row and indexed
public-tree/hook outcomes belong to the subsequent recorded local commit gate. No whole
readiness, new SDK or physical-window result is inferred from these checks.

The first ignored-row check failed because the newly authored child test modules were not
yet indexed, and because the earlier experimental player-queue aggregate test retained a
descriptive ignore without a policy row. The unchanged test/checker/parser/exception table
were hash-compared against own 74; this is an earlier local omission, not an official
upstream-baseline claim. The real aggregate refusal test then executed successfully on the
frozen lower artifact before its ignore reason was changed to daily and an exact logical
row was added. No assertion or implementation changed, and no exception hid correctness.
The first failed inventory and an unsuccessful timing-wrapper metric remain retained.

Final format and ignored-row checks passed against the indexed new modules and daily row.
Only that ignore-attribute string changed after the frozen strict Clippy/runtime proof;
all ordinary function bodies and all production carry helpers remain byte-identical. The
source9 receipt states this qualification explicitly instead of claiming another executable
build. Indexed public-tree and real per-commit hook results are bound at the Git boundary.


## Bounded common-save pressure follow-up

After the frozen e062 experiment, the coordinator reported a generic common-save byte-budget
refusal after returning projected contents and ordinary near/far travel. No caller code,
coordinates, physics or private runtime output was supplied or read. The own-source audit
found that the experimental common snapshot encoded every resident terrain column, while
the ordinary save already selected modified terrain. Compressed save-directory size does
not measure raw WAL afterimages, metadata and region witnesses.

The bounded follow-up retains the existing limits, complete resident readsets and all
entity/deletion, ordinary POI, deferred/proto, player, attachment and projection participants.
It selects ordinary dirty terrain plus the conservative live/opaque/schedule/member/transfer
union. A last stale scheduled occurrence can disappear without a block edit, so selection
also needs bookkeeping on the actual ChunkTicks owner. The two private revision fields and
four doc-hidden cross-crate helpers change Rust layout/dependency surface; no binary
compatibility is asserted. The hidden journal module adds scalar Usage, typed complete/partial
budget errors, byte_budget_error and Prepared::encoded_usage for the Game/lib boundary; its
hidden SaveDir snapshot helper accepts selected terrain. No mod-vocabulary/catalog or
persistence format changes. Exact queue revisions join the readset and only matching acknowledged selected queues become clean.

The own typed envelope counter measures borrowed frozen bytes and exact witness encoding
before allocating the envelope. Complete overflow reports exact required bytes; an earlier
frozen-payload stop explicitly reports only a lower bound. The existing trusted simulation
inspection exposes that distinction. Neither result is a durability receipt, and genuinely
oversized dirty decisions still refuse before publication without clearing dirty state.

Source-only review used Sunrise's own relevant source and the exact fetched official dc934ffa
five-path diff. The ordinary save path and its neighboring entity/streaming/schema changes
were inspected; the experimental common WAL/snapshot/checkpoint files are absent at that
exact tree. No rebase, latest-main fit, missing-official-fix or no-overlap claim follows.
The coordinator and independent reviewer supplied only generic observable requirements and
own-Sunrise scheduler/navigation findings. Their earlier unrestricted access is not a
clean-room certification. No foreign implementation or neighboring port source was read.

Own source snapshots and raw failures remain under .local/live-world-evidence/checkpoint-budget.
New test setup initially used unsupported ChunkMap.entry and invalid JSON, then omitted an
ordinary dirty-column setup and actual producer/reader SaveTables. Those failures remain
nonpasses. The actual stale-tick fixture then passed with a real raw saved occurrence and
independent decoded reader before the actual common ACK. Another new assertion incorrectly
required POI records for never-POI AIR columns; ordinary resolve_poi intentionally omits them.
The final fixture instead uses a real vanilla composter, independent strict POI reads and
actual last-removal empty Sections, preserving the far column's opaque payload.

The final source5 artifact executes the stale-last deletion and synthetic near/far common
save, completed Return, terminal common mode, oversized dirty refusal, dirty-queue retention,
opaque deletion and POI preservation/deletion paths. Existing callback/readset, multi-owner
metadata, player/guest, shutdown, handoff and fresh-process recovery controls remain selected.
The independent reviewer repeated all 41 unique selected core/filesystem/Game tests on the
pinned final binaries with all hashes unchanged. This is in-process/child-process generic
fixture evidence, not the coordinator's installed application or earlier save. Its actual
travel/save failure remains unresolved until the new exact SDK/runtime is rebuilt and tested.

The retained warnings are xcrun/Xcode SDK discovery and Mach-O __eh_frame linker classes also
observed in earlier own pre-e062 builds; this does not establish a pristine official baseline.
Focused lint, formatting, ignored inventory and indexed public-tree/actual hook outcomes are
recorded at the subsequent Git gate. Earlier whole-readiness and legacy WASM-load failures
remain nonpasses. AI authorship, human DCO eligibility/source review and maintainer claim/spec
approval remain explicit pending upstream decisions; no public action was made.


The final focused strict Clippy command passed for sunrise-client, sunrise-client-lib and
sunrise-core lib/tests with warnings denied and the complete Rust/config hash map unchanged.
No new executable build followed the final source5 proof. Source-only documentation and
input-list additions do not change that Rust scope. Formatting, ignored inventory, indexed
public-tree and the command-local .githooks/pre-commit execution are bound in the final Git
receipt; core.hooksPath being unset is not mistaken for an installed hook. Prior whole-readiness
nonpasses and the separate installed-world/SDK gates remain explicitly unresolved.

## Bounded rendering observations before an interpolation decision

A fresh-context author received only generic moving-projection rendering observations and
the questions of idle client ticks and coalesced accepted samples. The author's source-only
audit confirmed an unconditional presentation write at contact completion, but does not
establish its causal role in any observed frame hold/jump. The owner then authorized only
off-by-default bounded instrumentation; no interpolation, contact or acceptance fix is part
of this diagnostic checkpoint.

The JSONL trace and numeric frame marker are documented in `live-world-render-trace.md`,
including limits, float bits, separate accepted/coherent revisions, monotonic/UNIX anchors,
CPU-render versus display/capture qualifications, enabled-run overhead and shutdown flush.
Only own Sunrise source/rules were consulted. No neighboring port, Sable, Companion, native
caller implementation or old frame-audit evidence was opened. The implementation author
ran the preserved formatter and source-only checks, not Cargo, a compiler, Game, SDK build,
network, installation or publication. Actual executable traces remain the coordinator's
next gate. The detailed authoring input record accompanies the diagnostic documentation;
AI authorship and pending human/upstream eligibility decisions remain explicit.

## Deferred streaming retirement investigation: regression before correction

The coordinator supplied a sanitized own-engine event window and a symptom-triggered
Server thread OS stack excerpt. The window shows distinct global server ticks/publications
in bursts while client frames continue. The triggered stack places the actual delay in
ordinary frontier save service, through streaming retirement and the common checkpoint's
durable write/flush operations. Only the expressly authorized generic excerpts were read;
no native caller implementation, physical solver, private logs or neighboring port was used.

Own source inspection found that a due sweep's out-of-range columns request a barrier, but
ordinary admission discards its removal list after resetting the periodic sweep age. The
frontier's next fresh plan can therefore be empty, acknowledge a common save and retire no
column. Later sweeps request another checkpoint for the same resident column. This is a
local engine investigation, not an upstream approval or system-wide latency explanation.

The tests-only checkpoint adds a real Game/Streaming/common-WAL regression with independently
reopened terrain and unchanged later sweep epochs. Its constructed fixture holds generation
workers idle via the existing abort operation to keep unrelated arrivals out of the save
observations; streaming planning/admission/unload remains unpaused and real. Existing tests
and durability rules are unchanged. The author ran only formatting/source checks. Actual
failing-before execution is assigned to the coordinator before any production correction.
No App Nap, sleep, presentation, trajectory, physical step or carry change is included.

The ignored-inventory invocation did not pass: the default Python lacked `tomllib`; a
retry with the installed Python 3.14 reached its Rust parser subprocess, but `cargo` was
absent from PATH and was not started. The author did not add a compiler to PATH or run a
build to complete this check. The new daily row remains subject to the coordinator's
normal configured inventory gate. Formatting, diff and the actual local commit hook are
reported separately; these inventory attempts are retained nonpasses.

The coordinator subsequently compiled and actually executed the tests-only source
`b8e258f3329fc7d115a6b8ffa0e294c298add574` against unchanged production behavior. It reported
exit 101 at the expected assertion: the acknowledged common decision had left the deferred
column resident. The test completed in 4.48 seconds; the original binary and raw receipt are
retained by the coordinator. This author received that generic result, not a rewritten pass.

The bounded correction keeps the existing periodic sweep threshold due when common admission
defers either full-column or proto retirement. Frontier service therefore builds a fresh due
plan from the then-current World, results, view and forced areas. It does not reuse a stale
plan, callback result or readset. The exact fresh plan still passes the existing common
checkpoint, callback/readset validation and once-only owner-removal gate before retirement.
All WAL publication, durable writes, fsync, acknowledgment, budget and failure fences remain
unchanged. Successful admission returns to the normal periodic sweep schedule. No field,
persistence format, dependency, physics, contact, pose or rendering contract changes.

The regression and its daily policy row are byte-unchanged from the failing-before source.
The author again ran only the preserved formatter and source/commit checks. Passing-after,
focused existing controls and the rebuilt isolated application's cadence/render validation
remain separate coordinator gates; this source-only checkpoint claims none of them.
