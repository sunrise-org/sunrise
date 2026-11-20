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
