"""A contributor's pull request gets only the cheap checks; rollups, syncs, main and dispatches get the heavy ones."""

from collections import deque
from contextlib import redirect_stderr, redirect_stdout
from io import StringIO
import itertools
import json
import os
from pathlib import Path
import re
import subprocess
import tempfile
import time
import unittest
from unittest.mock import patch

from tests.os_support import needs_runner_shell
from tests.test_ci_concurrency import expression_value
from tools.ci import pr_gate, vendored_yaml

yaml = vendored_yaml.load()

ROOT = Path(__file__).resolve().parents[1]
REPO = "sunrise-org/sunrise"
PR = "6235"
PULL = f"pulls/{PR}"
BASE = "7b80a3cb688002e7dcde913fee93c9a3e5cb1c27"
SHA = "a0aebe46cb844c2fd367e29641b625ed05fd2c07"
MERGE = "b" * 40
OLD_BASE = "1e308aa3ac98b3e0cf04617ac7292bc7d6e52c03"
STALE_MERGE = "62e1079bdf8d0ecce34d348349ebcf0df24995d8"
SAME_REPO = "github.event_name != 'pull_request' || github.event.pull_request.head.repo.full_name == github.repository"


def pull(*, base=BASE, merge=MERGE, mergeable=True, branch="main", head_repo=REPO, head_ref="work", labels=()):
    return {"state": "open", "mergeable": mergeable, "merge_commit_sha": merge,
            "head": {"sha": SHA, "ref": head_ref, "repo": {"full_name": head_repo}},
            "base": {"sha": base, "ref": branch, "repo": {"full_name": REPO}},
            "labels": [{"name": label} for label in labels], "title": "RAW_PR_DATA"}


def commit(sha, *parents):
    return f"git/commits/{sha}", {"sha": sha, "parents": [{"sha": parent} for parent in parents]}


def tip(sha, *, branch="main", path="git/ref/heads/main"):
    return path, {"ref": f"refs/heads/{branch}", "object": {"type": "commit", "sha": sha}}


class GateTests(unittest.TestCase):
    def test_pull_request_gate(self):
        cases = [
            (True, ["rollup"], "work", "true"),
            (True, [], "fork-review-rollup-42", "true"),
            (True, [], "sunrise-review-rollup-1", "true"),
            (True, ["sync"], "work", "true"),
            (True, [], "sync-28-35", "true"),
            (True, [], "rollup-76", "true"),
            (True, [], "rollup-77", "true"),
            (True, [], "rollupx", "false"),
            (True, [], "cfix-hooks-rollup-76", "false"),
            (False, [], "rollup-76", "false"),
            (True, ["bug"], "ci-gate", "false"),
            (False, ["rollup", "sync"], "fork-review-rollup-42", "false"),
            (False, [], "sunrise-review-rollup-1", "false"),
        ]
        def never_read(*args, **kwargs):
            raise AssertionError("only a dispatch with pr reads a pull request")

        with patch.object(pr_gate, "run", never_read):
            for same, labels, branch, expected in cases:
                with self.subTest(same=same, labels=labels, branch=branch):
                    result = pr_gate.outputs("pull_request", same, "", repository=REPO, labels=labels, head_ref=branch)
                    self.assertEqual(result, {"heavy": expected, "pr_sha": "", "pr_merge": "", "trusted": ""})
            for event in ("push", "schedule", "workflow_dispatch"):
                self.assertEqual(pr_gate.outputs(event, False, "", repository=REPO),
                                 {"heavy": "true", "pr_sha": "", "pr_merge": "", "trusted": ""})


class DispatchTests(unittest.TestCase):
    def replay(self, exchange, *, refusal=None, number=PR, latency=0):
        """Drive the actual selection/summary/output boundary with ordered gh responses and a fake clock."""
        requests = deque(exchange)
        now = 0

        def read(argv, *, check, capture_output, timeout):
            nonlocal now
            self.assertTrue(requests, "unexpected API request")
            path, response = requests.popleft()
            self.assertEqual(argv, ["gh", "api", "--method", "GET", f"repos/{REPO}/{path}"])
            self.assertTrue(check and capture_output)
            self.assertEqual(timeout, min(5, 30 - now))
            self.assertGreater(timeout, 0)
            now += latency
            if isinstance(response, Exception):
                raise response
            raw = response if isinstance(response, bytes) else json.dumps(response).encode()
            return subprocess.CompletedProcess(argv, 0, stdout=raw, stderr=b"RAW_PR_DATA")

        def wait(seconds):
            nonlocal now
            self.assertGreater(seconds, 0)
            self.assertLessEqual(seconds, 3)
            now += seconds

        stdout, stderr = StringIO(), StringIO()
        with tempfile.TemporaryDirectory() as directory:
            summary = Path(directory) / "summary"
            env = {"EVENT": "workflow_dispatch", "PR": number, "GITHUB_REPOSITORY": REPO,
                   "GITHUB_SHA": OLD_BASE, "GITHUB_STEP_SUMMARY": str(summary)}
            with (patch.multiple(pr_gate, run=read, sleep=wait, monotonic=lambda: now),
                  redirect_stdout(stdout), redirect_stderr(stderr)):
                if refusal:
                    with self.assertRaisesRegex(SystemExit, refusal) as error:
                        pr_gate.main(env)
                    self.assertNotIn("RAW_PR_DATA", str(error.exception))
                else:
                    pr_gate.main(env)
            self.assertFalse(requests, "API replay was not consumed")
            self.assertNotIn("RAW_PR_DATA", stderr.getvalue())
            if refusal:
                self.assertEqual(stdout.getvalue(), "")
                self.assertFalse(summary.exists())
                return now
            return stdout.getvalue(), summary.read_text(encoding="utf-8"), now

    def assert_selected(self, result, *, base=BASE, trusted="true", elapsed=0):
        output, summary, now = result
        self.assertEqual(output, f"heavy=true\npr_sha={SHA}\npr_merge={MERGE}\ntrusted={trusted}\n")
        self.assertEqual(summary, f"### Selected PR #{PR} test merge\n\n"
                                  f"- Tested base: `{base}`\n- Tested merge: `{MERGE}`\n- PR head: `{SHA}`\n")
        self.assertEqual(now, elapsed)

    def test_immediate_fresh_merge_for_a_same_repository_pr_and_a_fork(self):
        self.assert_selected(self.replay([
            (PULL, pull(branch="release/26.3")), commit(MERGE, BASE, SHA),
            tip(BASE, branch="release/26.3", path="git/ref/heads/release%2F26.3"),
        ]))
        self.assert_selected(self.replay([
            (PULL, pull(head_repo="contributor/fork", head_ref="sync-upstream-7", labels=["rollup"])),
            commit(MERGE, BASE, SHA), tip(BASE),
        ]), trusted="false")

    def test_recorded_stale_mergeable_pr_is_refreshed_against_the_live_tip(self):
        # Run 45882409498: GitHub said mergeable=true even though the PR's .base.sha and the merge were behind.
        self.assert_selected(self.replay([
            (PULL, pull(base=OLD_BASE, merge=STALE_MERGE)), commit(STALE_MERGE, OLD_BASE, SHA), tip(BASE),
            # Even when .base.sha remains stale, the commit and the ref give the independent proof.
            (PULL, pull(base=OLD_BASE)), commit(MERGE, BASE, SHA), tip(BASE),
        ]), elapsed=3)

    def test_the_base_can_move_between_observations_and_a_stale_conflict_is_not_final(self):
        moved = "c" * 40
        self.assert_selected(self.replay([
            (PULL, pull(base=OLD_BASE, mergeable=False, merge=None)), tip(BASE),
            (PULL, pull(merge=STALE_MERGE)), commit(STALE_MERGE, BASE, SHA), tip(moved),
            (PULL, pull(base=moved)), commit(MERGE, moved, SHA), tip(moved),
        ]), base=moved, elapsed=6)

    def test_merge_computation_can_finish_but_persistent_uncertainty_cannot_publish(self):
        computing = (PULL, pull(mergeable=None, merge=None))
        self.assert_selected(self.replay([computing, (PULL, pull()), commit(MERGE, BASE, SHA), tip(BASE)]), elapsed=3)
        for observation, diagnosis, attempts, latency in (
            ([computing], "still computing", 8, 1),  # Time spent in the API counts against the deadline.
            ([(PULL, pull()), commit(MERGE, BASE, OLD_BASE), tip(BASE)], "parents do not match", 10, 0),
            ([(PULL, pull(base=OLD_BASE, mergeable=False, merge=None)), tip(BASE)], "conflict is unconfirmed", 10, 0),
        ):
            with self.subTest(diagnosis=diagnosis):
                self.assertEqual(self.replay(observation * attempts, refusal=f"selection budget: .*{diagnosis}",
                                             latency=latency), 30)

    def test_a_conflict_against_the_live_base_is_refused_immediately(self):
        self.assertEqual(self.replay([(PULL, pull(mergeable=False, merge=None)), tip(BASE)],
                                     refusal="conflicts with the live base"), 0)

    def test_invalid_or_failed_proof_does_not_publish(self):
        candidate = [(PULL, pull())]
        live = candidate + [commit(MERGE, BASE, SHA)]
        cases = (
            ([(PULL, {"state": "closed", "title": "RAW_PR_DATA"})], "closed"),
            ([(PULL, pull() | {"base": pull()["base"] | {"repo": {"full_name": "elsewhere/repo"}}})],
             "base is not in this repository"),
            ([(PULL, None)], "incomplete pull request metadata"),
            ([(PULL, pull() | {"head": pull()["head"] | {"sha": False}})], "invalid pull request head"),
            ([(PULL, pull(mergeable=0))], "invalid pull request head, base or mergeability"),
            ([(PULL, pull(merge=None))], "valid test merge SHA"),
            ([(PULL, pull(branch="../RAW_PR_DATA"))], "invalid pull request base ref"),
            (candidate + [(f"git/commits/{MERGE}", {"sha": OLD_BASE, "parents": [{"sha": BASE}, {"sha": SHA}]})],
             "invalid test merge commit proof"),
            (candidate + [commit(MERGE, BASE, SHA, OLD_BASE)], "invalid test merge commit proof"),
            (live + [tip(BASE, branch="other")], "invalid live base ref proof"),
            (live + [tip("")], "invalid live base ref proof"),
            (live + [("git/ref/heads/main", subprocess.CalledProcessError(1, "gh", stderr=b"RAW_PR_DATA"))],
             "request for the live base ref failed"),
            (candidate + [(f"git/commits/{MERGE}", subprocess.TimeoutExpired("gh", 5, output=b"RAW_PR_DATA"))],
             "request for the test merge commit timed out"),
            ([(PULL, b"RAW_PR_DATA")], "malformed JSON for the pull request"),
        )
        for exchange, refusal in cases:
            with self.subTest(refusal=refusal):
                self.assertEqual(self.replay(exchange, refusal=refusal), 0)

    def test_a_dispatch_on_our_rollup_or_sync_is_refused_since_its_pushes_run_the_gate(self):
        for ref, labels in (("sunrise-review-rollup-43", []), ("sync-upstream-7", []), ("sync/upstream-acbd", []), ("integration", ["rollup"])):
            with self.subTest(ref=ref, labels=labels):
                self.replay([(PULL, pull(head_ref=ref, labels=labels))], refusal="rollup or sync")

    def test_pr_must_be_a_canonical_number(self):
        for bad in ("0", "012", "12a", "1;id", "refs/pull/1/head", "-1", "6235\nRAW_PR_DATA"):
            with self.subTest(pr=bad):
                self.replay([], refusal="canonical pull request number", number=bad)


class WorkflowTests(unittest.TestCase):
    workflow = (ROOT / ".github/workflows/dyna-ci.yml").read_text()

    def job(self, name):
        match = re.search(rf"^  {re.escape(name)}:\n(.*?)(?=^  [a-z0-9_-]+:\n|\Z)", self.workflow, re.M | re.S)
        self.assertIsNotNone(match, name)
        return match[1]

    def test_heavy_jobs_wait_for_the_gate_and_fail_closed(self):
        for name in ("lint", "verify"):
            with self.subTest(job=name):
                self.assertIn("(needs.changes.result != 'success' || needs.changes.outputs.heavy == 'true')",
                              self.job(name))

    def test_cheap_jobs_run_without_the_heavy_gate(self):
        # verify-static runs outside the gate too: only where it runs depends on the gate.
        self.assertNotIn("outputs.heavy", self.job("verify-static").split("\n", 2)[1])
        self.assertIn("needs.changes.outputs.heavy == 'false'", self.job("format"))
        self.assertNotRegex(self.job("format"), r"cargo (clippy|build|test|check)")

    def test_a_dispatched_test_merge_is_checked_out_with_a_read_only_token_and_reported_on_the_head(self):
        workflow = yaml.safe_load(self.workflow)
        jobs = workflow["jobs"]
        changes = jobs["changes"]
        self.assertEqual(changes["permissions"], {"contents": "read", "actions": "read", "pull-requests": "read"})
        gate = next(step for step in changes["steps"] if step.get("id") == "gate")
        self.assertEqual(gate["run"], 'python3 tools/ci/pr_gate.py >> "$GITHUB_OUTPUT"')
        self.assertNotIn("continue-on-error", gate)
        self.assertEqual({name: value for name, value in changes["outputs"].items() if name.startswith("pr_")},
                         {"pr_sha": "${{ steps.gate.outputs.pr_sha }}", "pr_merge": "${{ steps.gate.outputs.pr_merge }}"})
        for name in ("lint", "verify", "verify-static"):
            with self.subTest(job=name):
                steps = jobs[name]["steps"]
                checkout = next(step for step in steps if step.get("uses") == "actions/checkout@v4")
                self.assertGreater(steps.index(checkout), 0)
                self.assertEqual(checkout["with"]["ref"], "${{ needs.changes.outputs.pr_merge }}")
                self.assertNotIn("outputs.pr_sha", self.job(name))
                self.assertNotIn("permissions", jobs[name])
        status = jobs["pr-status"]
        self.assertEqual(status["permissions"], {"statuses": "write"})
        self.assertEqual(status["if"], "always() && github.event_name == 'workflow_dispatch' && inputs.pr != '' && "
                         "needs.changes.result == 'success' && needs.changes.outputs.pr_sha != '' && "
                         "needs.changes.outputs.pr_merge != ''")
        self.assertEqual(status["steps"][0]["env"]["SHA"], "${{ needs.changes.outputs.pr_sha }}")
        self.assertEqual(status["steps"][0]["env"]["MERGE"], "${{ needs.changes.outputs.pr_merge }}")
        self.assertNotIn("actions/checkout", self.job("pr-status"))
        self.assertEqual(workflow["permissions"], {"contents": "read"})

    def test_verify_cache_saves_only_for_main_without_a_selected_pr(self):
        verify = yaml.safe_load(self.workflow)["jobs"]["verify"]
        cache = next(step for step in verify["steps"] if step.get("uses") == "Swatinem/rust-cache@v2")
        save = re.fullmatch(r"\$\{\{(.*?)\}\}", cache["with"]["save-if"])
        self.assertIsNotNone(save)
        main = "refs/heads/main"
        for event, ref, inputs, expected in (
            ("push", main, {}, True),
            ("workflow_dispatch", main, {"pr": ""}, True),
            ("workflow_dispatch", main, {"pr": PR}, False),
            ("pull_request", f"refs/pull/{PR}/merge", {}, False),
        ):
            with self.subTest(event=event, ref=ref, inputs=inputs):
                self.assertIs(expression_value(save[1], {"event_name": event, "ref": ref}, inputs), expected)

    def test_no_fork_code_reaches_our_runners(self):
        # The one trust decision: the gate's answer for a dispatched pull request, else the event's own repository.
        self.assertIn(f"trusted: ${{{{ steps.gate.outputs.trusted || (({SAME_REPO}) && 'true' || 'false') }}}}",
                      self.job("changes"))
        self.assertNotIn("github.event.pull_request.head.repo.full_name == github.repository) &&", self.job("lint"))
        for name in ("lint", "verify", "verify-static", "boat-watch"):
            body = self.job(name)
            if "head.repo.full_name" in body:
                self.fail(f"{name} decides trust itself instead of needs.changes.outputs.trusted")

    def runs_on(self, name, outputs, self_hosted_ci):
        """Evaluates a job's runs-on expression with these changes outputs, the way Actions would."""
        expression = re.search(r"^    runs-on: \$\{\{ (.*) \}\}$", self.job(name), re.M)[1]
        names = {"needs.changes.outputs." + key: value for key, value in outputs.items()}
        names |= {"vars.SELF_HOSTED_CI": self_hosted_ci, "inputs.github-fallback": False}
        python = re.sub(r"fromJSON\('([^']*)'\)", lambda m: repr(json.loads(m[1])), expression)
        python = re.sub(r"[a-z]+(?:\.[A-Za-z_-]+)+", lambda m: repr(names[m[0]]), python)
        python = python.replace("&&", " and ").replace("||", " or ").replace("!=", " != ")
        python = re.sub(r"!(?!=)", " not ", python)
        return eval(python, {})

    def test_an_untrusted_head_never_gets_our_runners(self):
        # A fork's dispatched pr: trusted is false and the runner script answers self_hosted=false.
        for name in ("lint", "verify", "verify-static"):
            for rust, forced, label, pool in itertools.product(("true", "false"), ("true", "false"),
                                                               ("", "boat"), ("all", "")):
                outputs = {"rust_required": rust, "forced_github": forced, "hosted_label": label,
                           "self_hosted": "false", "trusted": "false", "heavy": "true"}
                with self.subTest(job=name, outputs=outputs, pool=pool):
                    runner = self.runs_on(name, outputs, pool)
                    # verify-static, one of a contributor's two checks, never waits on a paid pool either.
                    self.assertIn(runner, ("ubuntu-latest",) if name == "verify-static" else ("ubuntu-latest", "boat"))
        # The control: the same expressions do reach our runners for a trusted head.
        trusted = {"rust_required": "true", "forced_github": "false", "hosted_label": "",
                   "self_hosted": "true", "trusted": "true", "heavy": "true"}
        self.assertEqual(self.runs_on("verify", trusted, "all"), "dyna-xlarge")  # dyna-large again once sunrise#5839 lands
        self.assertEqual(self.runs_on("verify-static", trusted, "all"), "dyna-medium")

    def test_a_pull_request_outside_the_gate_gets_githubs_runner_for_verify_static(self):
        # Our own branch, Rust changed, every switch pointing at our runners or boat: still GitHub's runner.
        for rust, label, pool in itertools.product(("true", "false"), ("", "boat-large"), ("all", "")):
            outputs = {"rust_required": rust, "forced_github": "false", "hosted_label": label,
                       "self_hosted": "false", "trusted": "true", "heavy": "false"}
            with self.subTest(rust=rust, label=label, pool=pool):
                self.assertEqual(self.runs_on("verify-static", outputs, pool), "ubuntu-latest")

    def test_boat_watcher_receives_the_same_pr_input_as_the_gate(self):
        jobs = yaml.safe_load(self.workflow)["jobs"]
        gate = next(step for step in jobs["changes"]["steps"] if step.get("id") == "gate")
        watcher = next(step for step in jobs["boat-watch"]["steps"] if step.get("run") == "python3 tools/ci/boat_watch.py")
        self.assertEqual(gate["env"]["PR"], "${{ inputs.pr }}")
        self.assertEqual(watcher["env"]["RECOVERY_PR"], gate["env"]["PR"])

    def test_the_runner_script_is_given_the_gates_trust(self):
        self.assertIn("SAME_REPO: ${{ steps.gate.outputs.trusted || (", self.job("changes"))

    @needs_runner_shell
    def test_the_runner_script_hears_the_gates_trust(self):
        lease = str(int(time.time()) + 3600)
        for same_repo, expected in (("false", "self_hosted=false"), ("true", "self_hosted=true")):
            env = {"PATH": os.environ["PATH"], "ENABLED": "true", "SAME_REPO": same_repo,
                   "EVENT": "workflow_dispatch", "LEASE": lease}
            result = subprocess.run(["bash", ROOT / "tools/ci/dyna_ci_runner.sh"], env=env, capture_output=True,
                                    text=True, check=True)
            self.assertEqual(result.stdout.strip(), expected, same_repo)

    def test_a_pull_requests_cheap_checks_run_on_githubs_runners(self):
        # format and verify-static are all a pull request outside the gate gets: nothing gates on them, so they leave
        # Dyna's counted runners to the gates, and a paid pool's balance must not fail them.
        self.assertIn("runs-on: ubuntu-latest\n", self.job("format"))
        self.assertIn("self_hosted: ${{ steps.gate.outputs.heavy == 'true' && ", self.job("changes"))
        # Outside the gate, the first term decides (Actions' || and && short-circuit): no switch, paid pool or boat
        # label reached through the later terms can move verify-static, and boat's watcher never starts.
        static = self.job("verify-static")
        self.assertIn("runs-on: ${{ needs.changes.outputs.heavy == 'false' && 'ubuntu-latest' || ", static)
        self.assertIn("RUNNER_KIND: ${{ needs.changes.outputs.heavy == 'false' && 'github' || ", static)
        self.assertIn("if: needs.changes.outputs.heavy == 'true' && ", self.job("boat-watch"))
        # A contributor's dispatched pull request is heavy but untrusted: GitHub's runner too.
        self.assertIn("needs.changes.outputs.trusted != 'true' && 'ubuntu-latest' ||", static)
        self.assertIn("|| needs.changes.outputs.trusted != 'true' && 'github' ||", static)

    def test_windows_jobs_run_daily_and_by_dispatch_only(self):
        workflow = yaml.safe_load((ROOT / ".github/workflows/windows-runtime-leases.yml").read_text())
        # GitHub's Windows runners: no push, pull request, rollup or sync waits on them; the daily run calls it.
        self.assertEqual(set(workflow[True]), {"workflow_call", "workflow_dispatch"})
        jobs = workflow["jobs"]
        for name in ("runtime-leases",):
            self.assertEqual(jobs[name]["if"], "inputs.commit != '' || github.event_name == 'workflow_dispatch'")

    def test_other_pull_request_workflows_skip_what_compiles_for_forks(self):
        for workflow, jobs in (("cargo-sunrise-mod-build.yml", ("build",)), ("modding-book.yml", ("book",))):
            text = (ROOT / ".github/workflows" / workflow).read_text()
            for name in jobs:
                with self.subTest(workflow=workflow, job=name):
                    body = re.search(rf"^  {name}:\n(.*?)(?=^  [a-z0-9_-]+:\n|\Z)", text, re.M | re.S)[1]
                    self.assertIn(f"if: {SAME_REPO}\n", body)

    def test_the_mod_tool_build_is_only_called(self):
        workflow = yaml.safe_load((ROOT / ".github/workflows/cargo-sunrise-mod-build.yml").read_text())
        # Every other archive is the daily run's (and a tag's): a pull request push holds no macOS or Windows runner.
        self.assertEqual(set(workflow[True]), {"workflow_call"})

    @needs_runner_shell
    def test_a_pull_request_builds_the_mod_tool_for_linux_only(self):
        workflow = yaml.safe_load((ROOT / ".github/workflows/cargo-sunrise-mod-build.yml").read_text())
        step = next(step for step in workflow["jobs"]["plan"]["steps"] if step.get("id") == "platforms")
        def targets(event):
            import tempfile
            with tempfile.NamedTemporaryFile("r") as output:
                subprocess.run(["bash", "-e", "-c", step["run"]], cwd=ROOT, check=True,
                               env={**os.environ, "EVENT_NAME": event, "GITHUB_OUTPUT": output.name})
                return [platform["target"] for platform in json.loads(output.read().split("=", 1)[1])]
        self.assertEqual(targets("pull_request"), ["x86_64-unknown-linux-gnu"])
        from tools import sunrise_mod_binstall
        self.assertEqual(targets("workflow_dispatch"), [platform["target"] for platform in sunrise_mod_binstall.PLATFORMS])


    def test_only_a_version_tag_push_publishes_the_mod_tool_archives(self):
        build = yaml.safe_load((ROOT / ".github/workflows/cargo-sunrise-mod-build.yml").read_text())
        entry = yaml.safe_load((ROOT / ".github/workflows/cargo-sunrise-mod.yml").read_text())
        daily = yaml.safe_load((ROOT / ".github/workflows/daily.yml").read_text())
        # The daily calls the build alone, read-only: nothing it runs can publish.
        self.assertEqual(daily["jobs"]["cargo-sunrise-mod"]["uses"], "./.github/workflows/cargo-sunrise-mod-build.yml")
        self.assertEqual(daily["jobs"]["cargo-sunrise-mod"]["permissions"], {"contents": "read"})
        self.assertEqual(build["permissions"], {"contents": "read"})
        self.assertTrue(all("permissions" not in job for job in build["jobs"].values()))
        # The entry point publishes the archives it built itself, and only for the push of a version tag: a dispatch
        # may name a tag too.
        self.assertEqual(entry["jobs"]["build"]["uses"], "./.github/workflows/cargo-sunrise-mod-build.yml")
        self.assertNotIn("with", entry["jobs"]["build"])
        self.assertEqual(entry["jobs"]["publish"]["needs"], "build")
        self.assertEqual(entry["jobs"]["publish"]["if"],
                         "github.event_name == 'push' && startsWith(github.ref, 'refs/tags/cargo-sunrise-mod-v')")
        self.assertEqual(set(entry["jobs"]), {"build", "publish"})


if __name__ == "__main__":
    unittest.main()
