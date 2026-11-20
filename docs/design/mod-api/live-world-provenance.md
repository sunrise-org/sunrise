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
