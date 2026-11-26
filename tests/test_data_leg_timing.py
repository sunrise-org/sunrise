"""Safe phase telemetry must not expose captured listing or test payloads."""
import contextlib
import io
import os
from pathlib import Path
import shlex
import subprocess
import sys
import tempfile
import unittest
from unittest import mock

from tests.os_support import needs_runner_shell
from tools.ci import ignored_tests

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "tools/ci/vendor/pyyaml-6.0.3.zip"))
import yaml


def data_leg_steps():
    return yaml.safe_load((ROOT / ".github/workflows/ignored-tests.yml").read_text())["jobs"]["ignored-tests-shards"]["steps"]


def fake_cargo(calls, raw_only=False):
    """A `cargo test` stand-in: it lists eight tests, and runs the ones not skipped; other commands run for real."""
    names = ["safe::case_" + str(n) for n in range(8)]
    real = subprocess.run

    def run(command, **kwargs):
        if "cargo" not in command:
            return real(command, **kwargs)
        args = command[command.index("cargo") + 1:]
        calls.append(args)
        if "--list" in args:
            return subprocess.CompletedProcess(command, 0, "".join(name + ": test\n" for name in names),
                                               "PRIVATE LISTING PAYLOAD\n")
        if raw_only:
            return subprocess.CompletedProcess(command, 0, "PRIVATE TEST PAYLOAD\n", "")
        skipped = {args[i + 1] for i, arg in enumerate(args) if arg == "--skip"}
        selected = [name for name in names if name not in skipped]
        lines = ["     Running unittests src/lib.rs (target/debug/deps/example)", f"running {len(selected)} tests",
                 "PRIVATE TEST PAYLOAD", *(f"test {name} ... ok" for name in selected),
                 f"test result: ok. {len(selected)} passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.1s"]
        return subprocess.CompletedProcess(command, 0, "\n".join(lines) + "\n", "")
    return run


class DataLegTimingTests(unittest.TestCase):
    def test_workflow_records_start_end_without_bypassing_filtered_runner(self):
        steps = data_leg_steps()
        start = next(i for i, s in enumerate(steps) if s.get("name") == "Record data-leg start and compiler cache")
        end = next(i for i, s in enumerate(steps) if s.get("name") == "Record data-leg end and compiler cache")
        run = next(i for i, s in enumerate(steps) if s.get("name") == "Run the ignored tests that need prepared data")
        self.assertLess(start, run)
        self.assertLess(run, end)
        self.assertEqual(steps[end]["if"], "always()")
        self.assertEqual(steps[run]["env"]["SUNRISE_DATA_LEG_TIMING"], "1")
        # The workflow may invoke only the existing filtered runner, not raw cargo
        # or a tee/cat pipeline forwarding its captured output.
        lines = steps[run]["run"].strip().splitlines()
        self.assertEqual(len(lines), 2)
        self.assertTrue(lines[0].startswith("export SUNRISE_TEST_RESOURCES="))
        self.assertEqual(shlex.split(lines[1]), ["python3", "tools/ci/with_xvfb.py", "python3", "tools/ci/ignored_tests.py", "--report", "$RUNNER_TEMP/daily-report.json", "$SHARD"])

    @needs_runner_shell
    def test_the_start_and_end_steps_print_the_compiler_cache_statistics(self):
        steps = data_leg_steps()
        start = next(i for i, s in enumerate(steps) if s.get("name") == "Record data-leg start and compiler cache")
        end = next(i for i, s in enumerate(steps) if s.get("name") == "Record data-leg end and compiler cache")
        with tempfile.TemporaryDirectory() as directory:
            fake = Path(directory) / "sccache"
            fake.write_text("#!/bin/sh\n[ \"$1\" = --show-stats ] || exit 1\necho 'Compile requests 0'\necho 'Cache hits 0'\n")
            fake.chmod(0o755)
            for i, label in ((start, "start"), (end, "end")):
                script = steps[i]["run"]
                self.assertIn("date -u", script)
                self.assertIn("sccache --show-stats", script)
                result = subprocess.run(["bash", "-c", script], env={**os.environ, "PATH": directory + os.pathsep + os.environ["PATH"]}, text=True, capture_output=True, check=True)
                self.assertRegex(result.stdout, rf"data-leg {label} utc=\d{{4}}-\d{{2}}-\d{{2}}T")
                self.assertIn("Compile requests 0", result.stdout)

    def test_real_cargo_boundary_markers_keep_payloads_private(self):
        calls = []
        with mock.patch.dict(os.environ, {"SUNRISE_DATA_LEG_TIMING": "1"}), \
                mock.patch.object(subprocess, "run", fake_cargo(calls)), \
                contextlib.redirect_stdout(io.StringIO()) as out, contextlib.redirect_stderr(io.StringIO()) as err:
            self.assertEqual(ignored_tests.main(["example#1of2"]), 0)
        self.assertNotIn("PRIVATE LISTING PAYLOAD", out.getvalue() + err.getvalue())
        self.assertNotIn("PRIVATE TEST PAYLOAD", out.getvalue() + err.getvalue())
        markers = [line for line in out.getvalue().splitlines() if line.startswith("data-leg phase=")]
        self.assertEqual(len(markers), 3)
        for line, phase in zip(markers, ("compile-list-start", "test-command-start", "test-command-end")):
            self.assertRegex(line, rf"^data-leg phase=example#1of2 {phase} utc=\d{{4}}-\d{{2}}-\d{{2}}T")
        self.assertRegex(markers[1], r"compile_list_seconds=\d+\.\d{3}$")
        self.assertRegex(markers[2], r"execution_seconds=\d+\.\d{3}$")
        self.assertEqual(len(calls), 2, "telemetry must not add cargo invocations")
        self.assertIn("--list", calls[0])
        self.assertNotIn("--list", calls[1])
        self.assertIn("--include-ignored", calls[1])
        self.assertIn("--exact", calls[1])
        self.assertIn("--test-threads=4", calls[1])
        with mock.patch.dict(os.environ, {"SUNRISE_DATA_LEG_TIMING": "1"}), \
                mock.patch.object(subprocess, "run", fake_cargo([], raw_only=True)), \
                contextlib.redirect_stdout(io.StringIO()) as out, contextlib.redirect_stderr(io.StringIO()) as err:
            self.assertNotEqual(ignored_tests.main(["example"]), 0)
        self.assertNotIn("PRIVATE TEST PAYLOAD", out.getvalue() + err.getvalue())
        self.assertIn("raw output withheld", out.getvalue())


if __name__ == "__main__":
    unittest.main()
