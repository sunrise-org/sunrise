"""Safe phase telemetry must not expose captured listing or test payloads."""
import contextlib
import io
import json
import os
from pathlib import Path
import shlex
import signal
import subprocess
import sys
import tempfile
import textwrap
import unittest
from unittest import mock

from tests.os_support import needs_runner_shell
from tools.ci import cgroup_telemetry, ignored_tests, vendored_yaml

ROOT = Path(__file__).resolve().parents[1]
yaml = vendored_yaml.load()
NO_JOB_CPU = ("data-leg cgroup-cpu scope=whole_job_cgroup_cumulative usage_usec=unavailable user_usec=unavailable "
              "system_usec=unavailable nr_periods=unavailable nr_throttled=unavailable throttled_usec=unavailable")


def data_leg_steps():
    return yaml.safe_load((ROOT / ".github/workflows/ignored-tests.yml").read_text())["jobs"]["ignored-tests-shards"]["steps"]


def cache_step(label, cgroup):
    step = next(s for s in data_leg_steps() if s.get("name") == f"Record data-leg {label} and compiler cache")
    script = step["run"].replace("/sys/fs/cgroup/", cgroup.as_posix() + "/")
    script = script.replace("python3 ", shlex.quote(Path(sys.executable).as_posix()) + " ")
    fake = cgroup / "sccache"
    fake.write_text("#!/bin/sh\n[ \"$1\" = --show-stats ] || exit 1\necho 'Compile requests 0'\necho 'Cache hits 0'\n")
    fake.chmod(0o755)
    return ["bash", "-e", "-c", script], {**os.environ, "PATH": str(cgroup) + os.pathsep + os.environ["PATH"]}


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
        self.assertEqual(steps[end]["if"], "always() && steps.seed.outcome == 'success'")
        self.assertEqual(steps[run]["env"]["SUNRISE_DATA_LEG_TIMING"], "1")
        # The workflow may invoke only the existing filtered runner, not raw cargo
        # or a tee/cat pipeline forwarding its captured output.
        lines = steps[run]["run"].strip().splitlines()
        self.assertEqual(len(lines), 2)
        self.assertTrue(lines[0].startswith("export SUNRISE_TEST_RESOURCES="))
        self.assertEqual(shlex.split(lines[1]), ["python3", "tools/ci/with_xvfb.py", "python3", "tools/ci/ignored_tests.py", "--report", "$RUNNER_TEMP/daily-report.json", "$SHARD"])

    @needs_runner_shell
    def test_the_start_and_end_steps_print_cache_statistics_and_job_cpu_at_end(self):
        with tempfile.TemporaryDirectory() as directory:
            cgroup = Path(directory)
            cpu = cgroup / "cpu.stat"
            cpu.write_bytes(b"usage_usec 100\nuser_usec 70\nsystem_usec PRIVATE\nnr_throttled 2\nthrottled_usec 25001\nother 999\n")
            readable = ("data-leg cgroup-cpu scope=whole_job_cgroup_cumulative usage_usec=100 user_usec=70 "
                        "system_usec=unavailable nr_periods=unavailable nr_throttled=2 throttled_usec=25001")
            for label, expected in (("start", []), ("end", [readable]), ("end", [NO_JOB_CPU])):
                with self.subTest(label=label, counter_present=cpu.exists()):
                    command, env = cache_step(label, cgroup)
                    result = subprocess.run(command, cwd=ROOT, env=env, text=True, capture_output=True,
                                            check=True, timeout=5)
                    self.assertRegex(result.stdout, rf"data-leg {label} utc=\d{{4}}-\d{{2}}-\d{{2}}T")
                    self.assertIn("Compile requests 0", result.stdout)
                    self.assertEqual([line for line in result.stdout.splitlines()
                                      if line.startswith("data-leg cgroup-cpu ")], expected)
                    self.assertNotIn(directory, result.stdout + result.stderr)
                    self.assertNotIn("PRIVATE", result.stdout + result.stderr)
                if expected == [readable]:
                    cpu.unlink()

    @needs_runner_shell
    @unittest.skipUnless(hasattr(os, "mkfifo") and hasattr(os, "killpg"), "needs a POSIX FIFO and process group")
    def test_the_end_step_reports_cache_and_exits_with_a_stalled_cpu_read(self):
        with tempfile.TemporaryDirectory() as directory:
            cgroup = Path(directory)
            cpu = cgroup / "cpu.stat"
            os.mkfifo(cpu)
            # Keep a writer open without sending data: the real reader blocks in read, not on EOF.
            writer = os.open(cpu, os.O_RDWR | os.O_NONBLOCK)
            try:
                command, env = cache_step("end", cgroup)
                with subprocess.Popen(command, cwd=ROOT, env=env, text=True, stdout=subprocess.PIPE,
                                      stderr=subprocess.PIPE, start_new_session=True) as step:
                    try:
                        stdout, stderr = step.communicate(timeout=5)
                    except subprocess.TimeoutExpired:
                        os.killpg(step.pid, signal.SIGKILL)
                        raise
                self.assertEqual(step.returncode, 0, stderr)
            finally:
                os.close(writer)
        self.assertEqual(stdout.splitlines()[-3:], ["Compile requests 0", "Cache hits 0", NO_JOB_CPU])
        self.assertEqual(stderr, "")

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
        self.assertNotIn("data-leg cgroup-phase ", out.getvalue())
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

    def test_client_half_samples_each_existing_batch_command_without_more_invocations(self):
        calls = []
        with tempfile.TemporaryDirectory() as directory:
            cgroup = Path(directory)
            report_path = cgroup / "report.json"
            with mock.patch.dict(os.environ, {"SUNRISE_DATA_LEG_TIMING": "1", "CI_CPUS": "16", "CI_JOBS": "10"}), \
                    mock.patch.object(ignored_tests, "CGROUP", cgroup), \
                    mock.patch.object(ignored_tests, "listed_entries", return_value=[]), \
                    mock.patch.object(subprocess, "run", side_effect=fake_cargo(calls)), \
                    contextlib.redirect_stdout(io.StringIO()) as out, contextlib.redirect_stderr(io.StringIO()) as err:
                self.assertEqual(ignored_tests.main(["sunrise-client#1of2"], report_path), 0)
            report = json.loads(report_path.read_text())
        self.assertEqual(set(report), {"shards", "problems", "passed", "incomplete"})
        self.assertEqual(report["problems"], [])
        self.assertEqual(report["incomplete"], [])
        self.assertEqual(len(report["passed"]), 4)
        log = out.getvalue() + err.getvalue()
        self.assertNotIn("PRIVATE", log)
        self.assertNotIn(directory, log)
        self.assertEqual(len(calls), 8)
        for listing, testing in zip(calls[::2], calls[1::2]):
            self.assertEqual(listing, ["test", "-p", "sunrise-client", "--lib", "--bins", "--tests",
                                       "--no-fail-fast", "--jobs=10", "--", "--list", "--include-ignored"])
            self.assertEqual(testing[:12], listing[:9] + ["--include-ignored", "--exact", "--test-threads=8"])
        metrics = [dict(field.split("=") for field in line.split()[2:])
                   for line in log.splitlines() if line.startswith("data-leg cgroup-phase ")]
        self.assertEqual([(row["batch"], row["phase"], row["cpu_usage_usec_delta"]) for row in metrics], [
            (f"sunrise-client#{number}of8", phase, "unavailable")
            for number in (1, 3, 5, 7) for phase in ("compile-list", "test-command")])
        for row in metrics:
            self.assertEqual(row["scope"], "job_cgroup_during_command")
            for field in ("current", "anon", "file"):
                self.assertEqual(row[f"memory_{field}_observed_max_bytes"], "unavailable")

    def test_complete_snapshots_keep_independent_maxima_cpu_deltas_and_coverage(self):
        with tempfile.TemporaryDirectory() as directory:
            cgroup = Path(directory)
            cpu = cgroup / "cpu.stat"
            current = cgroup / "memory.current"
            stat = cgroup / "memory.stat"
            cpu.write_text("usage_usec 45\nuser_usec 12\nsystem_usec 1\nnr_periods 2\nnr_throttled 0\nthrottled_usec 10\n")
            current.write_text("100\n")
            stat.write_text("anon 60\n")

            def tick(_seconds):
                if pause.call_count == 1:
                    cpu.write_text("usage_usec 53\nuser_usec 15\nsystem_usec 2\n")
                    current.write_text("200\n")
                    stat.write_text("anon 50\nfile 120\nanon_thp 999\n")
                elif pause.call_count == 2:
                    cpu.write_bytes(b"usage_usec 70\nuser_usec 11\nnr_periods 3\nnr_throttled 0\nthrottled_usec \xff\n")
                    current.write_text("150\n")
                    stat.write_text("anon 90\nfile 30\n")
                else:
                    raise StopIteration

            def publish(_mode, group, snapshot):
                with self.assertRaises(StopIteration):
                    cgroup_telemetry._sample_phase(Path(group), Path(snapshot))
                return mock.Mock()

            with mock.patch.object(cgroup_telemetry, "sleep", side_effect=tick) as pause, \
                    mock.patch.object(cgroup_telemetry, "_start_observer", side_effect=publish), \
                    mock.patch.object(cgroup_telemetry, "monotonic_ns", side_effect=[0, 1_200_000_000, 2_500_000_000]), \
                    contextlib.redirect_stdout(io.StringIO()) as out:
                with cgroup_telemetry.sample_command(cgroup, "sunrise-client#3of8", "test-command"):
                    pass
        self.assertEqual(out.getvalue().splitlines(), [
            "data-leg cgroup-phase batch=sunrise-client#3of8 phase=test-command scope=job_cgroup_during_command "
            "memory_sample_attempts=3 sample_interval_seconds=1 cpu_sample_span_ms=2500 memory_current_observed_max_bytes=200 "
            "memory_anon_observed_max_bytes=90 memory_file_observed_max_bytes=120 cpu_usage_usec_delta=25 "
            "cpu_user_usec_delta=unavailable cpu_system_usec_delta=unavailable cpu_nr_periods_delta=1 "
            "cpu_nr_throttled_delta=0 cpu_throttled_usec_delta=unavailable"])

    def test_snapshot_rejects_missing_unreadable_partial_and_malformed_publications(self):
        with tempfile.TemporaryDirectory() as directory:
            snapshot = Path(directory) / "latest"
            self.assertEqual(cgroup_telemetry._read_snapshot(snapshot).attempts, 0)
            snapshot.mkdir()  # open error on the read path
            self.assertEqual(cgroup_telemetry._read_snapshot(snapshot).attempts, 0)
            snapshot.rmdir()
            for raw in (b"1 123", b"1 123\n", b"1 PRIVATE" + b" -" * 15 + b"\n", b"1" * 400 + b"\n"):
                with self.subTest(raw=raw):
                    snapshot.write_bytes(raw)
                    self.assertEqual(cgroup_telemetry._read_snapshot(snapshot).attempts, 0)

    def test_observer_start_failure_preserves_the_command_exception(self):
        error = OSError("original command error")
        with mock.patch.object(cgroup_telemetry, "_start_observer", side_effect=OSError("PRIVATE spawn error")), \
                contextlib.redirect_stdout(io.StringIO()) as out, self.assertRaises(OSError) as raised:
            with cgroup_telemetry.sample_command(Path("."), "sunrise-client#3of8", "test-command"):
                raise error
        self.assertIs(raised.exception, error)
        self.assertIn("memory_sample_attempts=0", out.getvalue())
        self.assertNotIn("PRIVATE", out.getvalue())

    @unittest.skipUnless(hasattr(os, "mkfifo") and hasattr(os, "killpg"), "needs a POSIX FIFO and process group")
    def test_phase_wrapper_delivers_status_with_a_stalled_or_failed_observer(self):
        writer_source = textwrap.dedent("""\
            import os
            from pathlib import Path
            import runpy
            import sys
            import time

            case, script, mode, group, snapshot = sys.argv[1:]
            group, snapshot = Path(group), Path(snapshot)
            print("PRIVATE observer stdout", flush=True)
            print("PRIVATE observer stderr", file=sys.stderr, flush=True)

            def block(_seconds):
                with (group / "started").open("w") as signal:
                    print(os.getpid(), file=signal)
                # No writer ever opens this FIFO. Only the outer test kills us.
                with (group / "hold").open("rb"):
                    pass

            time.sleep = block
            if case == "write-error":
                snapshot = snapshot / "missing-parent"  # force a real publication error
            sys.argv = [script, mode, str(group), str(snapshot)]
            runpy.run_path(script, run_name="__main__")
            """)
        wrapper_source = textwrap.dedent("""\
            from pathlib import Path
            import subprocess
            import sys
            from tools.ci import cgroup_telemetry

            case, root, writer_source = sys.argv[1:]
            root = Path(root)
            real_popen, observer = subprocess.Popen, None

            def popen(argv, **kwargs):
                global observer
                if argv[2:3] != ["--sample-phase"]:
                    return real_popen(argv, **kwargs)
                # Instrument only the observer entrypoint. Keep the actual launch descriptors.
                observer = real_popen([sys.executable, "-c", writer_source, case, *argv[1:]], **kwargs)
                if case == "blocked":
                    # Model a kill request that does not make the observer reapable.
                    observer.kill = lambda: (root / "stop-requested").touch()
                return observer

            subprocess.Popen = popen
            with cgroup_telemetry.sample_command(root, "sunrise-client#3of8", "test-command"):
                command = subprocess.run([sys.executable, "-c",
                    "import sys; print('PRIVATE command output'); sys.exit(23)"], capture_output=True)
                if case == "blocked":
                    (root / "reader-pid").write_text((root / "started").read_text())
                elif observer.wait(timeout=5) == 0:
                    raise RuntimeError("fixture observer did not fail")
            if case == "blocked" and observer.poll() is not None:
                raise RuntimeError("fixture observer was not outstanding")
            print(f"command status {command.returncode}", flush=True)
            sys.exit(command.returncode)
            """)
        for case in ("blocked", "write-error"):
            with self.subTest(case=case), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                scratch = root / "scratch"
                scratch.mkdir()
                (root / "memory.stat").write_text("anon 123\nfile PRIVATE\nother 98765\n")
                if case == "blocked":
                    os.mkfifo(root / "started")
                    os.mkfifo(root / "hold")
                with subprocess.Popen([sys.executable, "-c", wrapper_source, case, directory, writer_source],
                                      cwd=ROOT, env={**os.environ, "TMPDIR": str(scratch)}, text=True,
                                      stdout=subprocess.PIPE, stderr=subprocess.PIPE, start_new_session=True) as wrapper:
                    try:
                        stdout, stderr = wrapper.communicate(timeout=5)
                        self.assertEqual(wrapper.returncode, 23, stderr)
                        if case == "blocked":
                            self.assertTrue((root / "stop-requested").is_file())
                            # Still outstanding after the real wrapper exit and capture EOF.
                            os.kill(int((root / "reader-pid").read_text()), 0)
                        self.assertEqual(list(scratch.iterdir()), [])
                    finally:
                        with contextlib.suppress(ProcessLookupError):
                            os.killpg(wrapper.pid, signal.SIGKILL)
                self.assertEqual(stderr, "")
                phase, status = stdout.splitlines()
                self.assertEqual(status, "command status 23")
                self.assertTrue(phase.startswith("data-leg cgroup-phase batch=sunrise-client#3of8 "
                                                 "phase=test-command scope=job_cgroup_during_command "))
                self.assertIn(f"memory_sample_attempts={int(case == 'blocked')}", phase.split())
                expected_anon = "123" if case == "blocked" else "unavailable"
                self.assertIn(f"memory_anon_observed_max_bytes={expected_anon}", phase.split())
                self.assertIn("memory_file_observed_max_bytes=unavailable", phase)
                self.assertIn("cpu_usage_usec_delta=unavailable", phase)
                self.assertNotIn("PRIVATE", stdout)
                self.assertNotIn("98765", stdout)
                self.assertNotIn(directory, stdout)


if __name__ == "__main__":
    unittest.main()
