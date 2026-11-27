"""Replay Actions API responses through the scheduled hot-path dispatch entrypoint."""

import contextlib
import io
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "tools/ci"))
import hot_paths_schedule  # noqa: E402

ACTIONS = "repos/sunrise-org/sunrise/actions"
DISPATCH = f"{ACTIONS}/workflows/hot-paths.yml/dispatches"
JOBS = f"{ACTIONS}/runs/7/attempts/2/jobs?per_page=100"


def workflow_runs(workflow, status):
    return f"{ACTIONS}/workflows/{workflow}/runs?status={status}&per_page=100"


def run(workflow="dyna-ci.yml", status="in_progress", event="pull_request", branch="topic",
        repo="sunrise-org/sunrise", run_id=7):
    return {"id": run_id, "path": f".github/workflows/{workflow}", "status": status, "event": event,
            "head_branch": branch, "head_repository": {"full_name": repo}, "run_attempt": 2}


def page(key, rows, total=None):
    return json.dumps({key: rows, "total_count": len(rows) if total is None else total})


class HotPathsScheduleTests(unittest.TestCase):
    def tick(self, responses=None, **context):
        responses = dict(responses or {})
        gets, posts = [], []

        def cli(command, *, check, capture_output, text, timeout, input=None):
            self.assertEqual(command[:4], ["gh", "api", "--hostname", "github.com"])
            self.assertTrue(check and capture_output and text and timeout > 0)
            if command[4] == "--paginate":
                _, endpoint = command[4:]
                gets.append(endpoint)
                if endpoint in responses:
                    reply = responses.pop(endpoint)
                    if isinstance(reply, Exception):
                        raise reply
                else:
                    self.assertRegex(endpoint, rf"^{ACTIONS}/workflows/(daily|release|nightly-build|dyna-ci)\.yml/"
                                               r"runs\?status=(queued|in_progress|waiting|pending|requested)&per_page=100$")
                    reply = page("workflow_runs", [])
                return subprocess.CompletedProcess(command, 0, stdout=reply)
            self.assertEqual(command[4:], ["--method", "POST", DISPATCH, "--input", "-"])
            posts.append(json.loads(input))
            return subprocess.CompletedProcess(command, 0)

        with tempfile.TemporaryDirectory() as directory:
            summary = Path(directory) / "summary"
            env = {"GITHUB_REPOSITORY": "sunrise-org/sunrise", "GITHUB_EVENT_NAME": "schedule",
                   "GITHUB_REF": "refs/heads/main", "GH_TOKEN": "mock-token", "GITHUB_STEP_SUMMARY": str(summary)}
            with (mock.patch.dict(os.environ, env | context, clear=True),
                  mock.patch.object(hot_paths_schedule.subprocess, "run", side_effect=cli),
                  contextlib.redirect_stdout(io.StringIO()) as out):
                code = hot_paths_schedule.main()
            message = summary.read_text(encoding="utf-8")
            self.assertEqual(out.getvalue(), message)
            self.assertNotIn("mock-token", message)
        self.assertEqual(responses, {}, "not all replayed endpoints were read")
        return code, posts, gets, message

    def assert_skips(self, responses, reason):
        code, posts, _, message = self.tick(responses)
        self.assertEqual((code, posts), (0, []))
        self.assertIn("Skipped this hot-path schedule tick", message)
        self.assertIn(reason, message)

    def test_idle_dispatches_main_and_checks_all_unfinished_statuses(self):
        code, posts, gets, message = self.tick()
        self.assertEqual((code, posts), (0, [{"ref": "main", "inputs": {}}]))
        self.assertEqual(set(gets), {workflow_runs(workflow, status)
                                    for workflow in ("daily.yml", "release.yml", "nightly-build.yml", "dyna-ci.yml")
                                    for status in ("queued", "in_progress", "waiting", "pending", "requested")})
        self.assertEqual(len(gets), 20)
        self.assertIn("Dispatched", message)

    def test_heavy_gate_entries_block_at_any_unfinished_status(self):
        for status, event, branch in (("queued", "push", "main"), ("in_progress", "push", "release/26.3"),
                                      ("waiting", "workflow_dispatch", "main"),
                                      ("pending", "pull_request", "sunrise-review-rollup-240"),
                                      ("requested", "pull_request", "sync/upstream-next")):
            with self.subTest(status=status, event=event):
                self.assert_skips({workflow_runs("dyna-ci.yml", status): page(
                    "workflow_runs", [run(status=status, event=event, branch=branch)])}, f"run 7 ({status})")

    def test_daily_and_both_release_entrypoints_block(self):
        for workflow, status, branch in (("daily.yml", "queued", "main"), ("release.yml", "in_progress", "main"),
                                         ("nightly-build.yml", "waiting", "release/26.3")):
            with self.subTest(workflow=workflow):
                self.assert_skips({workflow_runs(workflow, status): page(
                    "workflow_runs", [run(workflow, status, "workflow_dispatch", branch)])}, f"`{workflow}` run 7")

    def test_fork_pr_and_same_repo_pr_with_confirmed_light_selection_do_not_block(self):
        changes = {"name": "changes", "status": "completed", "conclusion": "success", "steps": [
            {"name": "Select checks once for this pull request", "conclusion": "success"}]}
        jobs = [changes, {"name": "lint", "conclusion": "skipped"}]
        code, posts, _, _ = self.tick({
            workflow_runs("dyna-ci.yml", "in_progress"): page("workflow_runs", [run(), run(event="schedule", run_id=9)]),
            workflow_runs("dyna-ci.yml", "queued"): page("workflow_runs", [
                run(status="queued", branch="rollup-fork", repo="contributor/sunrise", run_id=8)]),
            JOBS: page("jobs", jobs),
        })
        self.assertEqual((code, posts), (0, [{"ref": "main", "inputs": {}}]))

    def test_current_attempt_evidence_catches_a_label_only_gate_without_live_labels(self):
        self.assert_skips({
            workflow_runs("dyna-ci.yml", "in_progress"): page("workflow_runs", [run()]),
            JOBS: page("jobs", [{"name": "verify", "status": "queued", "conclusion": None}]),
        }, "integration gate")

    def test_a_light_partial_rerun_can_be_proven_by_its_running_format_job(self):
        code, posts, _, _ = self.tick({
            workflow_runs("dyna-ci.yml", "in_progress"): page("workflow_runs", [run()]),
            JOBS: page("jobs", [{"name": "format", "status": "in_progress", "conclusion": None}]),
        })
        self.assertEqual((code, posts), (0, [{"ref": "main", "inputs": {}}]))

    def test_undecided_same_repo_pr_and_unknown_repository_block(self):
        self.assert_skips({
            workflow_runs("dyna-ci.yml", "queued"): page("workflow_runs", [run(status="queued")]),
            JOBS: page("jobs", []),
        }, "has not confirmed light routing")
        self.assert_skips({workflow_runs("dyna-ci.yml", "in_progress"): page("workflow_runs", [
            {**run(), "head_repository": None}])}, "PR head repository is unknown")

    def test_reads_a_heavy_run_on_a_later_page(self):
        status = "queued"
        first = [run(status=status, repo="fork/sunrise", run_id=n) for n in range(100, 200)]
        second = [run(status=status, branch="rollup-240")]
        self.assert_skips({workflow_runs("dyna-ci.yml", status): page("workflow_runs", first, 101)
                           + "\n" + page("workflow_runs", second, 101)}, "integration gate")

    def test_api_error_or_incomplete_metadata_fails_without_dispatch(self):
        for reply in (subprocess.CalledProcessError(1, "gh", stderr="mock-token"),
                      page("workflow_runs", [], 1), '{"not_workflow_runs": []}',
                      page("workflow_runs", [run("different.yml", "queued")])):
            with self.subTest(reply=reply):
                code, posts, _, message = self.tick({workflow_runs("daily.yml", "queued"): reply})
                self.assertEqual((code, posts), (1, []))
                self.assertIn("could not read complete GitHub Actions run or job metadata", message)

    def test_untrusted_schedule_context_never_calls_gh(self):
        for context in ({"GITHUB_EVENT_NAME": "workflow_dispatch"}, {"GITHUB_REPOSITORY": "fork/sunrise"},
                        {"GITHUB_REF": "refs/heads/topic"}, {"GH_TOKEN": ""}):
            with self.subTest(context=context):
                code, posts, gets, message = self.tick(**context)
                self.assertEqual((code, posts, gets), (1, [], []))
                self.assertIn("expected the canonical main schedule", message)


if __name__ == "__main__":
    unittest.main()
