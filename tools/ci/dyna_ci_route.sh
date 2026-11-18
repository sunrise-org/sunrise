#!/usr/bin/env bash
# Prints self_hosted=true|false for this run's heavy jobs (tools/ci/dyna_ci_runner.sh decides), from the pool's
# switch and lease read live when the token may read Variables, else from the run's `vars` snapshot. A snapshot is
# fixed when the run is created, so it is judged against the run's creation time, never against now: a run that
# waited in a concurrency group or GitHub's queue would otherwise see a lease that lapsed while it waited.
#
# Env: VARS_ENABLED and VARS_LEASE (the run's snapshot of DYNA_CI_SELF_HOSTED and DYNA_CI_POOL_UNTIL), EVENT,
# POOL and VARS_ARC (sunrise-pool and the snapshot of SUNRISE_POOL_ARC, for callers that route there),
# SAME_REPO, GH_TOKEN (actions: read, for the run's creation time), GITHUB_REPOSITORY and GITHUB_RUN_ID. A
# reusable workflow's GITHUB_RUN_ID is its caller's run, whose snapshot it shares.
set -uo pipefail
ENABLED=
LEASE=
if [ "${SAME_REPO:-}" = true ]; then
  ENABLED=$(gh api "repos/$GITHUB_REPOSITORY/actions/variables/DYNA_CI_SELF_HOSTED" -q .value 2>/dev/null || true)
  LEASE=$(gh api "repos/$GITHUB_REPOSITORY/actions/variables/DYNA_CI_POOL_UNTIL" -q .value 2>/dev/null || true)
fi
# A refused read prints GitHub's error JSON: keep only a well-formed answer, else use the snapshot.
case "$LEASE" in '' | *[!0-9]*) LEASE= ;; esac
case "$ENABLED" in true | false) ;; *) ENABLED= ;; esac
echo "pool lease: live=${LEASE:-unreadable} snapshot=${VARS_LEASE:-none}" >&2
SOURCE=live
CREATED=
if [ -z "$LEASE" ]; then
  SOURCE=snapshot
  CREATED_AT=$(gh api "repos/$GITHUB_REPOSITORY/actions/runs/$GITHUB_RUN_ID" -q .created_at 2>/dev/null || true)
  [ -z "$CREATED_AT" ] || CREATED=$(date -d "$CREATED_AT" +%s 2>/dev/null || true)
fi
# ARC counts only for callers whose jobs ask for sunrise-pool (POOL); every other caller still asks for Dyna's labels.
ARC=
if [ "${POOL:-}" = sunrise-pool ]; then
  if [ "${SAME_REPO:-}" = true ]; then
    ARC=$(gh api "repos/$GITHUB_REPOSITORY/actions/variables/SUNRISE_POOL_ARC" -q .value 2>/dev/null || true)
  fi
  case "$ARC" in true | false) ;; *) ARC=${VARS_ARC:-} ;; esac
  echo "pool ARC: ${ARC:-unset}" >&2
fi
ENABLED=${ENABLED:-${VARS_ENABLED:-}} LEASE=${LEASE:-${VARS_LEASE:-}} LEASE_SOURCE=$SOURCE CREATED=$CREATED ARC=$ARC \
  exec bash "$(dirname "${BASH_SOURCE[0]}")/dyna_ci_runner.sh"
