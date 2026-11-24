"""Dev builds: release.yml kind dev builds a commit of main with the gate green and publishes it under its
own tag, never as the Nightly and never moving `daily-green`."""
import datetime as dt
import os
import subprocess
import tempfile
import unittest
from pathlib import Path
from unittest import mock

from tests.test_ci_concurrency import ROOT, yaml
from tools.ci import nightly

WORKFLOWS = ROOT / ".github/workflows"


def load(name):
    return yaml.load((WORKFLOWS / name).read_text(), Loader=yaml.BaseLoader)


class ReleaseDevTests(unittest.TestCase):
    def setUp(self):
        self.workflow = load("release.yml")
        self.jobs = self.workflow["jobs"]
        self.dev = {name: job for name, job in self.jobs.items() if name.startswith("dev")}

    def test_only_main_without_dry_run_publishes(self):
        mode = self.jobs["decide"]["steps"][1]
        self.assertEqual(mode["run"], 'python3 tools/release_train.py authorize --publish-kind "$KIND" --dry-run "$DRY_INPUT"')
        self.assertEqual(mode["env"], {"KIND": "${{ inputs.kind }}", "DRY_INPUT": "${{ inputs.dry_run }}"})
        self.assertEqual(self.workflow["on"]["workflow_dispatch"]["inputs"]["dry_run"]["default"], "true")
        publish = self.dev["dev-sdk-publish"]
        self.assertEqual(publish["with"]["publish"], "${{ needs.decide.outputs.publish == 'true' }}")
        self.assertEqual(publish["uses"], "./.github/workflows/sdk-publish.yml")
        # The game is built by `nightly build`, which publishes only when its own dry_run is false: it gets ours.
        run = self.dev["dev"]["steps"][0]["run"]
        self.assertIn('-f dry_run="$DRY_RUN"', run)
        self.assertEqual(self.dev["dev"]["steps"][0]["env"]["DRY_RUN"], "${{ needs.decide.outputs.dry_run }}")

    def test_a_dev_build_never_moves_daily_green_or_writes_the_nightly(self):
        text = yaml.safe_dump(self.dev)
        self.assertNotIn("daily-green", text)
        self.assertNotIn("git tag", text)
        self.assertNotIn("gh release", text)
        run = self.dev["dev"]["steps"][0]["run"]
        self.assertIn("gh workflow run nightly-build.yml --repo \"$GITHUB_REPOSITORY\" --ref main -f kind=dev", run)
        for name, job in self.dev.items():
            self.assertIn("inputs.kind == 'dev'", job["if"], name)
        self.assertIn("inputs.kind == 'nightly'", self.jobs["nightly"]["if"])

    def test_the_sdk_is_built_and_published_in_this_run_before_the_game(self):
        self.assertEqual(self.dev["dev-sdk"]["uses"], "./.github/workflows/prebuilt-game-build.yml")
        self.assertEqual(self.dev["dev-sdk"]["with"], {"commit": "${{ needs.decide.outputs.head }}"})
        self.assertEqual(self.dev["dev-sdk-publish"]["with"],
                         {"commit": "${{ needs.decide.outputs.head }}",
                          "publish": "${{ needs.decide.outputs.publish == 'true' }}"})
        game = self.dev["dev"]
        self.assertEqual(game["needs"], ["decide", "dev-sdk", "dev-sdk-publish"])
        self.assertIn("needs.dev-sdk.result == 'success'", game["if"])
        self.assertIn("(needs.dev-sdk-publish.result == 'success' || needs.dev-sdk-publish.result == 'skipped')",
                      game["if"].replace("\n", " "))

    def test_dev_builds_never_queue_with_nightlies(self):
        self.assertIn("inputs.kind == 'dev' && '-dev'", self.workflow["concurrency"]["group"])


class NightlyBuildDevTests(unittest.TestCase):
    def setUp(self):
        self.workflow = load("nightly-build.yml")
        self.steps = {step.get("name"): step for step in self.workflow["jobs"]["publish"]["steps"]}

    def test_a_dev_build_publishes_only_its_own_tag(self):
        nightly_step = self.steps["Publish as the `nightly` pre-release"]
        dev_step = self.steps["Publish as the dev-<date>-<short> pre-release"]
        self.assertIn("inputs.kind == 'nightly'", nightly_step["if"])
        self.assertIn("inputs.kind == 'dev'", dev_step["if"])
        self.assertIn("needs.pick.outputs.publish == 'true'", dev_step["if"])
        run = dev_step["run"]
        self.assertIn('gh release create "$TAG"', run)
        for never in ("rotate", "keep-source", "daily-green", "release create nightly", "prune --days", "release edit"):
            self.assertNotIn(never, run)
        self.assertIn('--title "Dev $DATE · $SHORT"', run)
        self.assertLess(run.index("require-sdk"), run.index("gh release create"))

    def test_the_channel_signed_and_checked_is_the_kind(self):
        sign = self.workflow["jobs"]["sign"]["steps"][2]
        self.assertEqual(sign["env"]["KIND"], "${{ inputs.kind }}")
        self.assertEqual(sign["env"]["ALSO_FOR"], "${{ inputs.kind == 'stable' && needs.pick.outputs.tag || "
                         "format('{0}-{1}-{2}', inputs.kind, needs.pick.outputs.date, needs.pick.outputs.short) }}")
        verify = self.steps["The signatures, checked as launchers check them"]["run"]
        self.assertIn('--tag "$SIGN_FOR" --publishing', verify)
        self.assertIn('version="${KIND}-${DATE}-${SHORT}"', verify)
        self.assertIn('--tag "$version"', verify)
        self.assertEqual(self.workflow["on"]["workflow_dispatch"]["inputs"]["kind"]["default"], "nightly")

    def test_every_platform_leg_is_required(self):
        game = load("game-build.yml")
        build = game["jobs"]["build"]
        self.assertNotIn("kind", game["on"]["workflow_call"]["inputs"])
        self.assertEqual({row["name"] for row in build["strategy"]["matrix"]["include"]}, {"linux", "windows", "macos"})
        self.assertNotIn("continue-on-error", build)
        self.assertEqual(game["jobs"]["linux-smoke"]["needs"], "legs")
        # A reusable workflow that ends failed or cancelled (an optional leg) passes back no outputs: signing and
        # publishing follow the legs job's build-complete marker, read by an ordinary job.
        self.assertNotIn("outputs", game["on"]["workflow_call"])
        marker = [step for step in game["jobs"]["legs"]["steps"] if step.get("with", {}).get("name") == "build-complete"]
        self.assertEqual([step["if"] for step in marker], ["steps.decide.outputs.complete == 'true'"])
        complete = self.workflow["jobs"]["complete"]
        self.assertEqual(complete["steps"][0]["with"]["name"], "build-complete")
        self.assertEqual(complete["outputs"]["complete"], "${{ steps.marker.outcome == 'success' }}")
        self.assertEqual(self.workflow["jobs"]["sign"]["needs"], ["pick", "build", "complete", "break-glass"])
        self.assertEqual(self.workflow["jobs"]["publish"]["needs"], ["pick", "build", "complete", "sign", "break-glass"])
        for job in ("sign", "publish"):
            self.assertIn("needs.complete.outputs.complete == 'true'", self.workflow["jobs"][job]["if"])

    @unittest.skipIf(os.name == "nt", "executes the POSIX signing step")
    def test_a_build_without_its_mac_archive_is_never_signed(self):
        sign = self.workflow["jobs"]["sign"]["steps"][2]["run"]
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            (root / "dist").mkdir()
            (root / "tools/ci").mkdir(parents=True)
            (root / "tools/ci/sign_release.sh").write_text(
                'for file in "${@:2}"; do test -f "$file" || exit 9; done\nprintf "%s\\n" "$@" > signed\n')
            for name in ("sunrise-linux-x86_64.tar.gz", "sunrise-windows-x64.zip",
                         "sunrise-server-linux-x86_64.tar.gz", "sunrise-source.tar.gz", "sunrise-release.json"):
                (root / "dist" / name).touch()
            for repository in ("sunrise-org/sunrise", "example/test"):
                result = subprocess.run(["bash", "-e", "-o", "pipefail", "-c", sign], cwd=root,
                                        env=dict(os.environ, KIND="dev", GITHUB_REPOSITORY=repository),
                                        capture_output=True, text=True)
                self.assertNotEqual(result.returncode, 0, result.stdout + result.stderr)
            (root / "dist/sunrise-macos-arm64.zip").touch()
            subprocess.run(["bash", "-e", "-o", "pipefail", "-c", sign], cwd=root, check=True,
                           env=dict(os.environ, KIND="dev", GITHUB_REPOSITORY="sunrise-org/sunrise"))
            self.assertIn("dist/sunrise-macos-arm64.zip", (root / "signed").read_text().split())

    def test_dev_builds_never_replace_a_waiting_nightly(self):
        group = self.workflow["concurrency"]["group"]
        self.assertIn("'nightly-build'", group)
        self.assertIn("format('dev-build{0}'", group)


class DylibOnlyDevTests(unittest.TestCase):
    """A Dev build with the dylib game alone (owner, 2026-11-22): sdk-<sha> and a thin, signed Dev release."""

    def setUp(self):
        self.release = load("release.yml")
        self.build = load("nightly-build.yml")

    def test_only_a_dev_build_may_leave_the_static_game_out(self):
        for workflow, job in ((self.release, "decide"), (self.build, "pick")):
            dispatch = workflow["on"]["workflow_dispatch"]["inputs"]["dylib_only"]
            self.assertEqual((dispatch["type"], dispatch["default"]), ("boolean", "false"))
            refusal = [step for step in workflow["jobs"][job]["steps"] if "dylib_only" in step.get("if", "")]
            self.assertEqual([step["if"] for step in refusal], ["inputs.kind != 'dev' && inputs.dylib_only"])
            self.assertIn("exit 1", refusal[0]["run"])
        dev = self.release["jobs"]["dev"]["steps"][0]
        self.assertIn('-f dylib_only="$DYLIB_ONLY"', dev["run"])
        self.assertEqual(dev["env"]["DYLIB_ONLY"], "${{ inputs.dylib_only && 'true' || 'false' }}")

    def test_no_game_is_built_and_no_smoke_test_is_asked_for(self):
        jobs = self.build["jobs"]
        self.assertIn("!inputs.dylib_only", jobs["exporters"]["if"])
        # build needs exporters, so it skips with it.
        self.assertIn("exporters", jobs["build"]["needs"])
        self.assertNotIn("always()", jobs["build"]["if"])
        gate = [step for step in jobs["publish"]["steps"] if step.get("name", "").startswith("Ready to publish")]
        self.assertEqual(gate[0]["if"], "${{ !inputs.dylib_only }}")
        for job in ("sign", "publish"):
            self.assertIn("inputs.dylib_only", jobs[job]["if"])

    @unittest.skipIf(os.name == "nt", "executes the POSIX signing and publishing steps")
    def test_the_thin_release_is_its_record_signed_and_published_without_archives(self):
        sign = self.build["jobs"]["sign"]["steps"][2]["run"]
        publish = [step for step in self.build["jobs"]["publish"]["steps"]
                   if step.get("name") == "Publish as the dev-<date>-<short> pre-release"][0]["run"]
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            (root / "dist").mkdir()
            (root / "tools/ci").mkdir(parents=True)
            (root / "tools/ci/sign_release.sh").write_text(
                'for file in "${@:2}"; do test -f "$file" || exit 9; done\nprintf "%s\\n" "$@" > signed\n')
            for name in ("sunrise-source.tar.gz", "sunrise-release.json"):
                (root / "dist" / name).touch()
            env = dict(os.environ, KIND="dev", SIGN_FOR="dev", DYLIB_ONLY="yes", GITHUB_REPOSITORY="sunrise-org/sunrise")
            run = lambda script, **extra: subprocess.run(["bash", "-e", "-o", "pipefail", "-c", script], cwd=root,
                                                        env={**env, **extra}, capture_output=True, text=True)
            # A real run refuses a record without the SDK's index; a dry run signs what it has.
            self.assertNotEqual(run(sign, PUBLISH="true").returncode, 0)
            self.assertEqual(run(sign, PUBLISH="false").returncode, 0)
            (root / "dist/sdk.json").touch()
            result = run(sign, PUBLISH="true")
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            self.assertEqual((root / "signed").read_text().split(),
                             ["dev", "dist/sdk.json", "dist/sunrise-source.tar.gz", "dist/sunrise-release.json"])
            # Publishing: no smoke test, no platform table; exactly SHA256SUMS' files and the signatures.
            (root / "dist/SHA256SUMS").write_text("aa  sdk.json\nbb  sunrise-source.tar.gz\ncc  sunrise-release.json\n")
            (root / "dist/nightly-notes.md").write_text("notes\n")
            bin_dir = root / "bin"
            bin_dir.mkdir()
            for tool, body in (("python3", "exit 0"), ("gh", 'printf "%s\\n" "$@" > "$GH_ARGS"')):
                (bin_dir / tool).write_text(f"#!/bin/sh\n{body}\n")
                (bin_dir / tool).chmod(0o755)
            sha = "0c02067" + "a" * 33
            result = run(publish, PATH=f"{bin_dir}:{os.environ['PATH']}", GH_ARGS=str(root / "gh-args"), SHA=sha,
                         SHORT="0c02067", DATE="2026-11-22", TAG="dev-2026-11-22-0c02067")
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            published = (root / "gh-args").read_text().split("\n")
            self.assertNotIn("dist/smoke-test.json", published)
            self.assertEqual([arg for arg in published if arg.startswith("dist/")],
                             ["dist/nightly-notes.md", "dist/sdk.json", "dist/sunrise-source.tar.gz",
                              "dist/sunrise-release.json", "dist/SHA256SUMS", "dist/SHA256SUMS.sig",
                              "dist/SHA256SUMS.v2.sig"])
            self.assertNotIn("Platform build results", (root / "dist/nightly-notes.md").read_text())

    def test_the_notes_say_the_build_is_the_dylib_game_alone(self):
        sha = "0c02067" + "a" * 33
        with tempfile.TemporaryDirectory() as folder:
            back = os.getcwd()
            os.chdir(folder)
            try:
                env = {"GITHUB_REPOSITORY": "sunrise-org/sunrise", "GITHUB_OUTPUT": str(Path(folder) / "out"),
                       "NIGHTLY_BASE": sha, "GITHUB_REF": "refs/heads/main", "NIGHTLY_DYLIB_ONLY": "true"}
                with mock.patch.dict(os.environ, env), mock.patch.object(nightly, "git"), \
                        mock.patch.object(nightly.train, "channel_head", return_value=sha), \
                        mock.patch.object(nightly, "sdk_ready", return_value=True), \
                        mock.patch.object(nightly, "gh", return_value=""):
                    nightly.pick_dev()
                notes = Path("nightly-notes.md").read_text()
            finally:
                os.chdir(back)
        self.assertIn(f"the dylib game alone, with no player archives: the prebuilt engine and runtime of `sdk-{sha}`",
                      notes)
        self.assertNotIn("The matching SDK is", notes)


class DevToolsTests(unittest.TestCase):
    def test_old_dev_builds_go_but_the_newest_and_everything_else_stay(self):
        tags = [{"tagName": "dev-2026-11-01-aaaaaaa", "isPrerelease": True},
                {"tagName": "dev-2026-11-02-bbbbbbb", "isPrerelease": True},
                {"tagName": "dev-2026-11-12-ccccccc", "isPrerelease": True},
                {"tagName": "prebuilt-" + "d" * 40, "isPrerelease": True},
                {"tagName": "sdk-" + "d" * 40, "isPrerelease": True},
                {"tagName": "nightly-2026-11-01", "isPrerelease": True},
                {"tagName": "dev-2026-11-01", "isPrerelease": True},
                {"tagName": "dev-2026-11-01-eeeeeee", "isPrerelease": False}]
        today = dt.date(2026, 11, 13)
        self.assertEqual(nightly.stale_dev_builds(tags, today, 7),
                         ["dev-2026-11-01-aaaaaaa", "dev-2026-11-02-bbbbbbb"])
        # A long quiet spell: the newest stays however old.
        self.assertEqual(nightly.stale_dev_builds(tags[:2], dt.date(2027, 1, 1), 7), ["dev-2026-11-01-aaaaaaa"])

    def test_pick_checks_the_gate_and_names_the_dev_build(self):
        sha = "0c02067" + "a" * 33
        with tempfile.TemporaryDirectory() as folder:
            out = Path(folder) / "out"
            back = os.getcwd()
            os.chdir(folder)
            try:
                env = {"GITHUB_REPOSITORY": "sunrise-org/sunrise", "GITHUB_OUTPUT": str(out), "NIGHTLY_BASE": sha,
                       "GITHUB_REF": "refs/heads/main"}
                with mock.patch.dict(os.environ, env), mock.patch.object(nightly, "git"), \
                        mock.patch.object(nightly.train, "channel_head", return_value=sha) as gate, \
                        mock.patch.object(nightly, "sdk_ready", return_value=True), \
                        mock.patch.object(nightly, "gh", return_value=""):
                    nightly.pick_dev()
                gate.assert_called_once_with("dev", sha)
                values = dict(line.split("=", 1) for line in out.read_text().splitlines())
                today = dt.datetime.now(dt.timezone.utc).date().isoformat()
                self.assertEqual(values["tag"], f"dev-{today}-0c02067")
                self.assertEqual(values["version"], f"dev-{today}")
                self.assertEqual(values["build"], "true")
                notes = Path("nightly-notes.md").read_text()
                self.assertIn(f"sunrise-launcher install {values['tag']}", notes)
                self.assertIn(f"sunrise-release-sign verify --tag {values['tag']} --signature SHA256SUMS.v2.sig", notes)
                # Published already (the same commit the same day): skipped, never replaced.
                with mock.patch.dict(os.environ, env), mock.patch.object(nightly, "git"), \
                        mock.patch.object(nightly.train, "channel_head", return_value=sha), \
                        mock.patch.object(nightly, "sdk_ready", return_value=True), \
                        mock.patch.object(nightly, "gh", return_value='{"tagName": "x"}'):
                    nightly.pick_dev()
                self.assertIn("build=false", out.read_text().splitlines()[-10:])
                self.assertIn("sdk=true", out.read_text().splitlines()[-10:])
                # No published SDK: a dry run builds without it (sdk=false: no sdk.json), a real Dev build stops.
                with mock.patch.dict(os.environ, {**env, "DRY_RUN": "true"}), mock.patch.object(nightly, "git"), \
                        mock.patch.object(nightly.train, "channel_head", return_value=sha), \
                        mock.patch.object(nightly, "sdk_ready", return_value=False):
                    nightly.pick_dev()
                self.assertIn("sdk=false", out.read_text().splitlines()[-10:])
                # No published SDK: a real Dev build stops.
                with mock.patch.dict(os.environ, env), mock.patch.object(nightly, "git"), \
                        mock.patch.object(nightly.train, "channel_head", return_value=sha), \
                        mock.patch.object(nightly, "sdk_ready", return_value=False):
                    with self.assertRaises(SystemExit):
                        nightly.pick_dev()
            finally:
                os.chdir(back)


if __name__ == "__main__":
    unittest.main()
