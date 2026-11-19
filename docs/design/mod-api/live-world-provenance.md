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
