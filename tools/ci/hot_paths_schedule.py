"""Dispatch the scheduled hot-path benchmark only if the repository's heavy workflows are idle."""

import json
import os
import re
import subprocess
import sys

from dyna_ci_pool_watch import pages
from pr_gate import integration

REPO = "sunrise-org/sunrise"
REF = "refs/heads/main"
ACTIONS = f"repos/{REPO}/actions"
WORKFLOWS = ("daily.yml", "release.yml", "nightly-build.yml", "dyna-ci.yml")
UNFINISHED = ("queued", "in_progress", "waiting", "pending", "requested")


def listing(path, key):
    output = subprocess.run(["gh", "api", "--hostname", "github.com", "--paginate", path],
                            check=True, capture_output=True, text=True, timeout=30).stdout
    batches = pages(output)
    rows = []
    for page in batches:
        if not isinstance(page[key], list):
            raise ValueError("not a listing")
        rows.extend(page[key])
    # A filtered list of runs can stop at GitHub's 1,000-result search cap even with pagination.
    if (not batches or any(type(page["total_count"]) is not int or page["total_count"] != len(rows) for page in batches)
            or (key == "workflow_runs" and len(rows) >= 1000)):
        raise ValueError("incomplete listing")
    return rows


def gate_blocker(run, run_id):
    """None only for a run we can rule out as a heavy dyna-ci entry."""
    event = run["event"]
    if event == "schedule":
        return None  # The hosted wrapper only dispatches; that dispatch is a separate entry.
    if event == "workflow_dispatch":
        return "heavy gate"
    branch = run.get("head_branch")
    if not isinstance(branch, str) or not branch:
        return "gate routing is unknown"
    if event == "push":
        return "heavy gate" if branch in ("main", "release/26.3") else None
    if event != "pull_request":
        return "gate routing is unknown"
    head = run.get("head_repository")
    repo = head.get("full_name") if isinstance(head, dict) else None
    if not isinstance(repo, str) or not re.fullmatch(r"[\w-]+/[\w.-]+", repo, re.ASCII):
        return "PR head repository is unknown"
    if repo.lower() != REPO:
        return None  # Fork PRs are always light, even with integration names or labels.
    if integration((), branch):
        return "integration gate"

    # A same-repo PR with an ordinary branch can still be heavy by its event's labels. Live labels can
    # change after the event, so use what this attempt actually selected, not a fresh list of labels.
    attempt = run.get("run_attempt")
    if type(attempt) is not int or attempt < 1:
        return "PR attempt is unknown"
    jobs = listing(f"{ACTIONS}/runs/{run_id}/attempts/{attempt}/jobs?per_page=100", "jobs")
    if any(job["name"] in ("lint", "verify") and job["conclusion"] != "skipped" for job in jobs):
        return "integration gate"
    for job in jobs:
        if job["name"] == "changes" and job["conclusion"] == "success" and any(
                step["name"] == "Select checks once for this pull request" and step["conclusion"] == "success"
                for step in job["steps"]):
            return None
        # A partial re-run may have only format, which is itself conditional on heavy=false.
        if job["name"] == "format" and (job["status"] == "in_progress" or job["conclusion"] in ("success", "failure")):
            return None
    return "PR attempt has not confirmed light routing"


def blocker():
    for workflow in WORKFLOWS:
        for status in UNFINISHED:
            runs = listing(f"{ACTIONS}/workflows/{workflow}/runs?status={status}&per_page=100", "workflow_runs")
            for run in runs:
                run_id = run["id"]
                if (type(run_id) is not int or run_id < 1 or run["path"] != f".github/workflows/{workflow}"
                        or run["status"] != status):
                    raise ValueError("unexpected run metadata")
                reason = gate_blocker(run, run_id) if workflow == "dyna-ci.yml" else "busy workflow"
                if reason:
                    return f"`{workflow}` run {run_id} ({status}): {reason}"
    return None


def main():
    def report(message):
        print(message)
        with open(os.environ["GITHUB_STEP_SUMMARY"], "a", encoding="utf-8") as summary:
            summary.write(message + "\n")

    if ((os.getenv("GITHUB_REPOSITORY"), os.getenv("GITHUB_EVENT_NAME"), os.getenv("GITHUB_REF"))
            != (REPO, "schedule", REF) or not os.getenv("GH_TOKEN")):
        report("Hot paths not dispatched: expected the canonical main schedule with an Actions token.")
        return 1
    try:
        busy = blocker()
    except (KeyError, TypeError, ValueError, OSError, subprocess.SubprocessError):
        report("Hot paths not dispatched: could not read complete GitHub Actions run or job metadata.")
        return 1
    if busy:
        report(f"Skipped this hot-path schedule tick — {busy}.")
        return 0
    try:
        subprocess.run(["gh", "api", "--hostname", "github.com", "--method", "POST",
                        f"{ACTIONS}/workflows/hot-paths.yml/dispatches", "--input", "-"],
                       input=json.dumps({"ref": "main", "inputs": {}}),
                       check=True, capture_output=True, text=True, timeout=30)
    except (OSError, subprocess.SubprocessError):
        report("Hot-path dispatch request failed or was not confirmed; no retry.")
        return 1
    report("Dispatched hot paths on `main`: no blocking workflow was observed.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
