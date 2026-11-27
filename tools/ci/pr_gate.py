"""Whether this run gets the heavy checks (everything that compiles), as `heavy=true|false` for $GITHUB_OUTPUT.

Ordinary pull requests get only the cheap checks (verify-static and format). The heavy checks run on main's
pushes, dispatches, and same-repository rollup or sync pull requests, identified by labels or branch names.
Fork pull requests remain cheap even when they use those labels or names.

A dispatch with `pr` verifies GitHub's test merge against the PR head and the live tip of its target branch.
The heavy jobs check out only that merge SHA with a read-only token; a separate job posts their results on the
PR head. A rollup or sync of this repository is refused: each push to it already runs the heavy gate.
"""

from collections.abc import Mapping
from dataclasses import dataclass
import json
import os
import re
from subprocess import CalledProcessError, TimeoutExpired, run
import sys
from time import monotonic, sleep
from typing import TypeGuard
from urllib.parse import quote

# Keep selection short: this is the old ten-read, three-second window, including request time now.
REFRESH_SECONDS = 30
POLL_SECONDS = 3
REQUEST_SECONDS = 5


@dataclass(frozen=True)
class TestMerge:
    base: str
    head: str
    merge: str
    trusted: bool


def integration(labels, head_ref: str) -> bool:
    """A rollup or sync pull request: its own pushes run the heavy gate.

    A rollup builder session pushes only branches named after itself (`rollup-76`, `sunrise-review-rollup-3`), and a
    label added after the pull request opens misses its first event, so the branch name alone counts too.
    """
    return bool({"rollup", "sync"}.intersection(labels)) or head_ref.startswith(
        ("fork-review-rollup-", "sunrise-review-rollup-", "rollup-", "sync-", "sync/upstream-"))


def is_sha(value: object) -> TypeGuard[str]:
    return isinstance(value, str) and value != "0" * 40 and re.fullmatch(r"[0-9a-f]{40}", value) is not None


def select_merge(number: str, repository: str) -> TestMerge:
    deadline = monotonic() + REFRESH_SECONDS
    waiting = "GitHub has not returned a fresh test merge"

    def error(reason: str) -> SystemExit:
        return SystemExit(f"::error::pull request #{number}: {reason}")

    def get(path: str, what: str) -> object:
        left = deadline - monotonic()
        if left <= 0:
            raise error(f"no fresh test merge within the selection budget: {waiting}")
        try:
            response = run(["gh", "api", "--method", "GET", f"repos/{repository}/{path}"],
                           check=True, capture_output=True, timeout=min(REQUEST_SECONDS, left))
            return json.loads(response.stdout)
        except TimeoutExpired:
            raise error(f"GitHub API request for {what} timed out") from None
        except (CalledProcessError, OSError):
            raise error(f"GitHub API request for {what} failed") from None
        except (ValueError, RecursionError):
            raise error(f"GitHub API returned malformed JSON for {what}") from None

    # GET asks GitHub to compute the test merge in the background. See docs/ci-runners.md for the REST contract.
    while True:
        match get(f"pulls/{number}", "the pull request"):
            case {"state": "closed"}:
                raise error("the pull request is closed")
            case {"state": "open", "mergeable": mergeable, "merge_commit_sha": merge, "labels": list(raw_labels),
                  "head": {"sha": head, "ref": str(head_ref), "repo": {"full_name": str(head_repo)}},
                  "base": {"sha": reported_base, "ref": str(base_ref), "repo": {"full_name": str(base_repo)}}}:
                pass
            case _:
                raise error("invalid or incomplete pull request metadata")
        if base_repo != repository:
            raise error("the base is not in this repository")
        if (not is_sha(head) or not is_sha(reported_base) or not head_ref
                or not re.fullmatch(r"[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+", head_repo)
                or (mergeable is not None and type(mergeable) is not bool)):
            raise error("invalid pull request head, base or mergeability")
        if "\0" in base_ref or {"", ".", ".."}.intersection(base_ref.split("/")):
            raise error("invalid pull request base ref")
        try:
            ref_path = quote(base_ref, safe="")
        except UnicodeError:
            raise error("invalid pull request base ref") from None
        labels = []
        for label in raw_labels:
            match label:
                case {"name": str(name)}:
                    labels.append(name)
                case _:
                    raise error("invalid pull request labels")
        trusted = head_repo == repository
        if trusted and integration(labels, head_ref):
            raise error("a rollup or sync already runs the heavy gate on every push; read that run's checks")
        if mergeable is None:
            waiting = "GitHub is still computing the test merge"
        else:
            candidate = None
            if mergeable is True:
                if not is_sha(merge):
                    raise error("GitHub did not name a valid test merge SHA")
                match get(f"git/commits/{merge}", "the test merge commit"):
                    case {"sha": returned, "parents": [{"sha": first}, {"sha": second}]} if (
                        returned == merge and is_sha(first) and is_sha(second)
                    ):
                        candidate = merge, first, second
                    case _:
                        raise error("invalid test merge commit proof: expected the named SHA and exactly two parents")
            # Read the exact target ref *after* inspecting the immutable candidate, not the dispatch revision
            # or the PR response's base SHA (which can stay stale even when mergeable is already true).
            match get(f"git/ref/heads/{ref_path}", "the live base ref"):
                case {"ref": ref, "object": {"type": "commit", "sha": live}} if (
                    ref == f"refs/heads/{base_ref}" and is_sha(live)
                ):
                    pass
                case _:
                    raise error("invalid live base ref proof: expected the exact branch and a commit SHA")
            if candidate is None:
                if reported_base == live:
                    raise error("the pull request conflicts with the live base; there is no merge to test")
                waiting = "GitHub reports a conflict against an older base; the current conflict is unconfirmed"
            else:
                merge, first, second = candidate
                if (first, second) == (live, head):
                    return TestMerge(live, head, merge, trusted)
                waiting = "the test merge parents do not match the live base and observed PR head"
        if (left := deadline - monotonic()) > 0:
            sleep(min(POLL_SECONDS, left))


def outputs(event: str, same_repository: bool, pr: str, *, repository: str, summary_path: str = "",
            labels: tuple[str, ...] | list[str] = (), head_ref: str = "") -> dict[str, str]:
    if event == "workflow_dispatch" and pr:
        if not re.fullmatch(r"[1-9][0-9]{0,6}", pr):
            raise SystemExit("::error::pr must be a canonical pull request number (1–7 digits, no leading zero)")
        tested = select_merge(pr, repository)
        if summary_path:
            with open(summary_path, "a", encoding="utf-8") as summary:
                summary.write(f"### Selected PR #{pr} test merge\n\n"
                              f"- Tested base: `{tested.base}`\n"
                              f"- Tested merge: `{tested.merge}`\n"
                              f"- PR head: `{tested.head}`\n")
        return {"heavy": "true", "pr_sha": tested.head, "pr_merge": tested.merge,
                "trusted": "true" if tested.trusted else "false"}
    # ci.yml's full checks run only for a fork's pull request (and scheduled runs on main): this repository's other
    # pull requests get dyna-checks' cheap checks, and these integration pull requests the heavy gate.
    heavy = event != "pull_request" or (same_repository and integration(labels, head_ref))
    return {"heavy": "true" if heavy else "false", "pr_sha": "", "pr_merge": "", "trusted": ""}


def main(env: Mapping[str, str] = os.environ) -> None:
    repository = env.get("GITHUB_REPOSITORY", "")
    values = outputs(env.get("EVENT", ""), env.get("HEAD_REPO") == repository, env.get("PR", ""),
                     repository=repository, summary_path=env.get("GITHUB_STEP_SUMMARY", ""),
                     labels=json.loads(env.get("PR_LABELS", "[]")) or [], head_ref=env.get("HEAD_REF", ""))
    for key, value in values.items():
        print(f"{key}={value}")
    print(f"heavy checks: {values['heavy']}", file=sys.stderr)


if __name__ == "__main__":
    main()
