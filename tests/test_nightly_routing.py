"""Exercise workflow routing expressions and smoke commands with actual matrix entries."""
import itertools
import json
import os
from pathlib import Path
import re
import shlex
import shutil
import subprocess
import tempfile
import textwrap
import unittest

from tests.os_support import BASH, needs_runner_shell
from tools.ci import vendored_yaml

yaml = vendored_yaml.load()

ROOT = Path(__file__).resolve().parents[1]
GAME = ROOT / ".github/workflows/game-build.yml"
EXPORTERS = ROOT / ".github/workflows/exporters.yml"
NIGHTLY = ROOT / ".github/workflows/nightly-build.yml"
SMOKE = ROOT / "tools/ci/game_smoke.sh"


# Each build leg's runner: (Blacksmith's, GitHub's with BLACKSMITH=off).
RUNNERS = {
    "linux": ("blacksmith-16vcpu-ubuntu-2204", "ubuntu-22.04"),
    "windows": ("blacksmith-16vcpu-windows-2025", "windows-latest"),
    "macos": ("blacksmith-12vcpu-macos-15", "macos-15"),
}


class NightlyRoutingTests(unittest.TestCase):
    @unittest.skipUnless(shutil.which("node"), "Node evaluates GitHub's JavaScript-style expressions")
    def test_real_matrix_routes_both_trust_levels(self):
        game = yaml.safe_load(GAME.read_text())["jobs"]
        expressions = {name: game[name]["runs-on"][3:-2] for name in ("build", "linux-smoke")}
        expressions["exporters"] = yaml.safe_load(EXPORTERS.read_text())["jobs"]["exporters"]["runs-on"][3:-2]
        smoke = game["linux-smoke"]
        smoke_if = smoke["if"][3:-2]
        rows = game["build"]["strategy"]["matrix"]["include"]
        self.assertEqual({row["name"] for row in rows}, RUNNERS.keys())
        linux = next(row for row in rows if row["name"] == "linux")
        download = next(step for step in smoke["steps"] if step.get("uses", "").startswith("actions/download-artifact@"))
        self.assertEqual(download["with"]["name"], linux["asset"], "smoke must download the hosted producer's archive")
        self.assertIn(f"/hosted-linux/{linux['dir']}", str(smoke["steps"]))
        self.assertEqual(smoke["needs"], "legs")
        script = """const fs = require('fs');
const state = JSON.parse(fs.readFileSync(0, 'utf8'));
// Actions returns an empty string for absent context properties, not undefined.
const context = values => new Proxy(values, {get: (values, key) => values[key] ?? ''});
const matrix = context(state.matrix), vars = context(state.vars), inputs = state.inputs, github = context(state.github);
const expression = state.expression, fromJSON = JSON.parse, toJSON = JSON.stringify;
const cancelled = () => false, needs = {legs: {result: 'success'}};
console.log(JSON.stringify(eval(expression)));
"""
        def evaluate(expression, matrix, trusted, repository="sunrise-org/sunrise"):
            result = subprocess.run(["node", "-e", script], input=json.dumps({"expression": expression, "matrix": matrix, "inputs": {"trusted": trusted}, "vars": variables, "github": {"repository": repository}}), text=True, capture_output=True, check=True)
            return json.loads(result.stdout)
        bookkeeping = next(job["runs-on"][3:-2] for job in yaml.safe_load(NIGHTLY.read_text())["jobs"].values()
                           if "NIGHTLY_BOOKKEEPING_RUNNER" in job.get("runs-on", ""))
        for configured, disabled, trusted, dyna, blacksmith in itertools.product(
            [False, True], [False, True], [False, True], [None, "false", "true"], [None, "off"]
        ):
            variables = {"SELF_HOSTED_CI": "off" if disabled else "all"}
            if dyna is not None:
                variables["DYNA_CI_SELF_HOSTED"] = dyna
            if blacksmith:
                variables["BLACKSMITH"] = blacksmith
            override = configured and dyna == "true"
            if configured:
                variables.update({"NIGHTLY_LINUX_RUNNER": '"dyna-large"', "NIGHTLY_EXPORTERS_RUNNER": '"dyna-large"', "NIGHTLY_BOOKKEEPING_RUNNER": '"dyna-small"'})
            with self.subTest(configured=configured, disabled=disabled, trusted=trusted, dyna=dyna, blacksmith=blacksmith):
                # Exporters: Dyna's runner for an untrusted build when it is configured, else GitHub's.
                self.assertEqual(evaluate(expressions["exporters"], {}, trusted), "dyna-large" if override and not trusted else "ubuntu-latest")
                expected_bookkeeping = "dyna-small" if override else ("ubuntu-latest" if disabled else ["self-hosted", "linux", "sunrise-light"])
                self.assertEqual(evaluate(bookkeeping, {}, trusted), expected_bookkeeping)
                self.assertEqual(evaluate(smoke_if, {}, trusted), not trusted and override)
                if not trusted and override:
                    self.assertEqual(evaluate(expressions["linux-smoke"], {}, trusted), "dyna-large")
                for row in rows:
                    if row["name"] == "linux":
                        self.assertEqual(row["os"], "ubuntu-22.04", "shipped Linux binaries keep the documented libc baseline")
                    # Every leg on its Blacksmith machine, or with BLACKSMITH=off on GitHub's.
                    expected = RUNNERS[row["name"]][blacksmith == "off"]
                    with self.subTest(entry=row["name"]):
                        self.assertEqual(evaluate(expressions["build"], row, trusted), expected)
        for repository, trusted, row in itertools.product(["sunrise-org/sunrise", "horizon-rust/horizon"], [False, True], rows):
            with self.subTest(repository=repository, trusted=trusted, entry=row["name"]):
                if repository == "sunrise-org/sunrise":
                    expected = 100 if row.get("mac") else 60
                else:
                    expected = 60 if row.get("mac") else 40
                timeout = game["build"]["timeout-minutes"][3:-2]
                self.assertEqual(evaluate(timeout, row, trusted, repository), expected)

    @unittest.skipUnless(shutil.which("node"), "Node evaluates GitHub's JavaScript-style expressions")
    def test_only_a_dev_releases_legs_follow_the_owners_blacksmith_and_dyna_switches(self):
        script = """const state = JSON.parse(require('fs').readFileSync(0, 'utf8'));
const context = values => new Proxy(values, {get: (values, key) => values[key] ?? ''});
const matrix = context(state.matrix), vars = context(state.vars), inputs = context(state.inputs);
console.log(JSON.stringify(eval(state.expression)));
"""
        prebuilt = yaml.safe_load((ROOT / ".github/workflows/prebuilt-game-build.yml").read_text())
        legs = json.loads(prebuilt["jobs"]["fresh"]["steps"][1]["env"]["ALL"])
        game = yaml.safe_load(GAME.read_text())["jobs"]["build"]
        builds = {"game": (game, game["strategy"]["matrix"]["include"]), "sdk": (prebuilt["jobs"]["build"], legs)}
        switches = [None, "off", "on", "true"]
        platforms = ["DEV_ON_DYNA_LINUX", "DEV_ON_DYNA_WINDOWS", "DEV_ON_DYNA_MACOS"]

        def runs_on(job, row, variables, dev):
            state = {"expression": job["runs-on"][3:-2], "matrix": row, "vars": variables, "inputs": {"dev": dev}}
            result = subprocess.run(["node", "-e", script], input=json.dumps(state), text=True,
                                    capture_output=True, check=True)
            return json.loads(result.stdout)

        for (what, (job, rows)), switch, every, dev in itertools.product(builds.items(), switches, switches, [False, True]):
            variables = {"BLACKSMITH": "off"}
            if switch is not None:
                variables["BLACKSMITH_DEV_MACOS"] = switch
            if every is not None:
                variables["BLACKSMITH_DEV"] = every
            for row in rows:
                with self.subTest(build=what, switch=switch, every=every, dev=dev, leg=row["name"]):
                    blacksmith = dev and (every == "on" or (switch == "on" and row["os"] == "macos-15"))
                    self.assertEqual(runs_on(job, row, variables, dev), row["blacksmith"] if blacksmith else row["os"])

        # Each DEV_ON_DYNA_<platform>=on moves that platform's legs, game and SDK, to Dyna's runner in a Dev release
        # whatever the Blacksmith switches say; no other leg moves, and nothing moves for a value but `on` or without dev.
        for (what, (job, rows)), platform, value, blacksmiths, dev in itertools.product(
                builds.items(), platforms, switches, [{}, {"BLACKSMITH_DEV": "on"}], [False, True]):
            for row in rows:
                with self.subTest(build=what, platform=platform, value=value, blacksmiths=blacksmiths, dev=dev, leg=row["name"]):
                    variables = {"BLACKSMITH": "off", **blacksmiths, **({} if value is None else {platform: value})}
                    expected = row["os"] if not blacksmiths else row["blacksmith"]
                    if dev and value == "on" and row["dyna"] == platform:
                        expected = "dyna-large"
                    elif not dev:
                        expected = row["os"]
                    self.assertEqual(runs_on(job, row, variables, dev), expected)

    @needs_runner_shell
    def test_any_missing_leg_holds_the_build(self):
        decide = next(step["run"] for step in yaml.safe_load(GAME.read_text())["jobs"]["legs"]["steps"] if "run" in step)
        legs = ("linux", "windows", "macos")
        for absent in [None, *legs]:
            with self.subTest(absent=absent), tempfile.TemporaryDirectory() as folder:
                for leg in legs:
                    if leg != absent:
                        os.makedirs(f"{folder}/legs/leg-{leg}")
                output = Path(folder, "output")
                ran = subprocess.run([BASH, "-c", decide], cwd=folder, capture_output=True, text=True,
                                     env={**os.environ, "GITHUB_OUTPUT": str(output)})
                self.assertEqual(ran.returncode == 0, absent is None, ran.stdout)
                self.assertEqual(output.exists() and output.read_text() == "complete=true\n", absent is None)

    def test_build_timings_are_a_report_and_never_a_signed_or_released_asset(self):
        game = yaml.safe_load(GAME.read_text())
        steps = game["jobs"]["build"]["steps"]
        builds = [shlex.split(step["run"].replace("\\\n", " ")) for step in steps if "cargo build" in step.get("run", "")]
        self.assertIn("--timings", [argument for words in builds for argument in words])
        collect = next(i for i, step in enumerate(steps) if step.get("name") == "The build's timings, by leg")
        upload = collect + 1
        self.assertTrue(steps[upload]["uses"].startswith("actions/upload-artifact@"))
        # Both timing steps are best-effort, and run after the marker and the keychain lock, so a stalled
        # upload cannot keep a finished leg from counting as built.
        for index in (collect, upload):
            self.assertIs(steps[index]["continue-on-error"], True)
        for name in ("This leg built", "The signing keychain, locked again"):
            self.assertLess(next(i for i, step in enumerate(steps) if step.get("name") == name), collect)
        self.assertLess(next(i for i, step in enumerate(steps)
                             if step.get("with", {}).get("name") == "leg-${{ matrix.name }}"), collect)
        name = steps[upload]["with"]["name"]
        self.assertTrue(name.startswith("cargo-timings-"))
        # The sign job hashes every `sunrise-*` artifact, and publish's download leaves the reports out.
        nightly = yaml.safe_load((ROOT / ".github/workflows/nightly-build.yml").read_text())
        self.assertEqual(next(step["with"]["pattern"] for step in nightly["jobs"]["sign"]["steps"]
                              if step.get("uses", "").startswith("actions/download-artifact@") and "pattern" in step["with"]),
                         "sunrise-*")
        download = [step for step in nightly["jobs"]["publish"]["steps"]
                    if step.get("uses", "").startswith("actions/download-artifact@")]
        self.assertEqual([step["with"]["pattern"] for step in download],
                         ["@(sunrise-*|signature|ephemeral.pub|nightly-notes|smoke-test.json)"])

    def test_every_shipped_glibc_program_is_checked_by_workflow_owned_tools(self):
        text = GAME.read_text()
        steps = [step for job in yaml.safe_load(text)["jobs"].values() for step in job.get("steps", [])]
        commands = re.findall(r"check_glibc.py ([^\n]+)", "\n".join(step.get("run", "") for step in steps).replace("\\\n", " "))
        checked = [{Path(argument).name for argument in shlex.split(command)} for command in commands]
        # Every program each release folder takes (the SDK's input list), and the game program itself.
        routes = json.loads((ROOT / "crates/sunrise-mod-package/src/sdk/inputs.json").read_text())["routes"]
        programs = {route: {name for name in routes[route]["program"] if "." not in name} for route in ["package", "package-server"]}
        self.assertEqual(checked, [programs["package"] | {"sunrise-client"},
                                   programs["package-server"] | {"sunrise-engine"}])
        tools = [step for step in steps if step.get("with", {}).get("path") == ".workflow-tools"]
        self.assertTrue(tools)
        for checkout in tools:
            self.assertEqual(checkout["with"]["ref"], "${{ github.workflow_sha }}",
                             "older build candidates may lack the new workflow's validator/smoke helpers")

    def test_build_stages_the_workflows_rust_pin(self):
        game = yaml.safe_load(GAME.read_text())["jobs"]
        tools, = [step for step in game["build"]["steps"] if step.get("name") == "Workflow-owned validation tools"]
        self.assertTrue(tools["uses"].startswith("actions/checkout@"))
        self.assertNotIn("if", tools)
        self.assertEqual(tools["with"]["ref"], "${{ github.workflow_sha }}")
        self.assertEqual(tools["with"]["path"], ".workflow-tools")
        self.assertIn("tools/ci/rust-version", tools["with"]["sparse-checkout"].splitlines())

    def test_hosted_runners_never_use_machine_specific_tool_setup(self):
        # Only the exporters can run on a self-hosted (Dyna) runner; the build legs are fresh machines and take none.
        self.assertNotIn("runner-env.sh", GAME.read_text())
        steps = [step for step in yaml.safe_load(EXPORTERS.read_text())["jobs"]["exporters"]["steps"]
                 if "bash tools/ci/runner-env.sh" in step.get("run", "")]
        self.assertEqual(len(steps), 1)
        condition = steps[0]["if"]
        for environment in ["github-hosted", "self-hosted"]:
            expression = condition.replace("runner.environment", repr(environment))
            self.assertEqual(eval(expression, {"__builtins__": {}}), environment == "self-hosted")

    @unittest.skipUnless(os.name == "posix", "The pick step runs in Bash on Linux")
    def test_pick_failure_preserves_its_error_without_missing_notes_noise(self):
        shell, = [step["run"] for job in yaml.safe_load(NIGHTLY.read_text())["jobs"].values()
                  for step in job.get("steps", []) if step.get("id") == "pick"]
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            python = root / "python3"
            python.write_text("#!/bin/sh\necho 'no green dyna-checks main base'\nexit 23\n")
            python.chmod(0o755)
            summary = root / "summary"
            result = subprocess.run([BASH, "-c", shell], cwd=root, env={**os.environ, "FORCE": "", "KIND": "nightly", "GITHUB_STEP_SUMMARY": str(summary), "PATH": str(root) + os.pathsep + os.environ["PATH"]}, text=True, capture_output=True)
            self.assertEqual(result.returncode, 23)
            self.assertIn("no green dyna-checks main base", summary.read_text())
            self.assertNotIn("nightly-notes.md", result.stderr)

    @unittest.skipUnless(os.name == "posix", "The smoke command runs on Linux and macOS")
    def test_smoke_runs_from_relative_or_absolute_binary_directory(self):
        command = next(line.strip() for line in SMOKE.read_text().splitlines() if line.strip().startswith("client="))
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            binary_dir = root / "target tree/release"
            binary_dir.mkdir(parents=True)
            binary = binary_dir / "sunrise-client"
            binary.write_text("#!/bin/sh\nprintf 'smoke ran'\n")
            binary.chmod(0o755)
            for directory in [str(binary_dir), str(binary_dir.relative_to(root))]:
                with self.subTest(directory=directory):
                    result = subprocess.run([BASH, "-c", command + '\n"$client"'], cwd=root, env={**os.environ, "BIN": directory}, text=True, capture_output=True)
                    self.assertEqual(result.returncode, 0, result.stderr)
                    self.assertEqual(result.stdout, "smoke ran")

    @unittest.skipUnless(shutil.which("Xvfb"), "Xvfb is required for the Linux smoke display")
    def test_linux_smoke_gets_a_ready_display(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            binary = root / "sunrise-client"
            # A controlled game fixture isolates the real smoke orchestration and Xvfb.
            binary.write_text(textwrap.dedent('''\
                #!/usr/bin/env python3
                import json, os, pathlib, sys
                args = sys.argv[1:]
                report = {"frames": int(args[args.index("--world-frames") + 1]),
                          "chunks": 1, "meshed_sections": 1, "player": [0, 64, 0],
                          "display": os.environ.get("DISPLAY"), "wayland": os.environ.get("WAYLAND_DISPLAY"),
                          "stack_override": os.environ.get("RUST_MIN_STACK")}
                pathlib.Path(args[args.index("--smoke-report") + 1]).write_text(json.dumps(report))
            '''))
            binary.chmod(0o755)
            env = {**os.environ, "BIN": folder, "RUNNER_TEMP": folder, "RUNNER_OS": "Linux",
                   "DISPLAY": ":invalid", "WAYLAND_DISPLAY": "invalid", "RUST_MIN_STACK": "16777216"}
            result = subprocess.run([BASH, str(SMOKE)], cwd=ROOT, env=env, text=True, capture_output=True)
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            report = json.loads((root / "smoke-test.json").read_text())
            self.assertEqual(report["frames"], 1500)
            self.assertRegex(report["display"], r"^:[0-9]+$")
            self.assertIsNone(report["wayland"])
            self.assertIsNone(report["stack_override"], "the integrated server must start at its own stack floor")

    @unittest.skipUnless(os.name == "posix", "The signing gate runs in Bash on Linux")
    def test_fork_dry_run_cannot_pass_without_real_smoke_report(self):
        shell, = [step["run"] for job in yaml.safe_load(NIGHTLY.read_text())["jobs"].values()
                  for step in job.get("steps", []) if step.get("name", "").startswith("Ready to publish")]
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            (root / "tools").symlink_to(ROOT / "tools", target_is_directory=True)
            # No gh request is needed to prove a missing smoke report fails even in dry-run mode.
            stub = root / "gh"
            stub.write_text("#!/bin/sh\nprintf '[]'\n")
            stub.chmod(0o755)
            result = subprocess.run([BASH, "-c", shell], cwd=root, env={**os.environ, "DRY_RUN": "true", "GITHUB_REPOSITORY": "sunrise-org/sunrise", "PATH": str(root) + os.pathsep + os.environ["PATH"]}, text=True, capture_output=True)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("smoke", result.stdout.lower())


if __name__ == "__main__":
    unittest.main()
