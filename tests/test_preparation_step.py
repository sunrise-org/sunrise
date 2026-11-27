"""Observable preparation diagnostics without executing Cargo, Java or the preparer."""
import contextlib
import hashlib
import io
import os
import json
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock

from tests.os_support import needs_runner_shell
from tools.ci import preparation_step

SCRIPT = Path(__file__).resolve().parents[1] / "tools/ci/preparation_step.py"


class PreparationStepTests(unittest.TestCase):
    def invoke(self, root, code, step="resource-export", **environment):
        return subprocess.run([sys.executable, str(SCRIPT), step, str(root / "private.log"),
                               sys.executable, "-c", code], capture_output=True, text=True,
                              env=dict(os.environ, **environment))

    def diagnostic(self, step, status, reason, last="redacted-unclassified"):
        return (f"::error::preparation step={step} exit={status} reason_hint={reason} "
                "expected_key=unavailable-or-invalid found_key=not-recorded cache_state=unavailable "
                f"last_line_classification={last}\n"
                "unclassified: see private log (not retained on per-job runners)\n")

    def test_failure_is_private_and_status_survives(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            for marker, reason in [("No space left on device", "disk-full"),
                                   ("Permission denied", "permission-denied"),
                                   ("Read-only file system", "read-only-filesystem"),
                                   ("OutOfMemoryError", "java-out-of-memory"),
                                   ("Could not resolve host", "network-dns"),
                                   ("Connection timed out", "network-timeout"),
                                   ("No such file or directory", "missing-file"),
                                   ("is not a 26.3 version file Sunrise knows (sha1 0)", "pinned-version-differs"),
                                   ("does not describe the pinned 26.3 release", "pinned-version-differs"),
                                   ("is not the pinned file: expected sha1 0, got 1", "pinned-file-differs"),
                                   ("Install it, or set JAVA_HOME to it, and try again.", "missing-java"),
                                   ("is Java 21; Java 25 or newer is needed", "missing-java"),
                                   ("unknown failure", "unclassified")]:
                with self.subTest(reason=reason):
                    payload = "token=SECRET .local/minecraft/26.3/file {NBT: excerpt} ::error::injected " + marker
                    result = self.invoke(root, f"import sys; print({payload!r}); sys.exit(23)")
                    self.assertEqual(result.returncode, 23)
                    self.assertEqual(result.stdout, self.diagnostic("resource-export", 23, reason, reason if reason != "unclassified" else "redacted-unclassified"))
                    sidecar = (root / "private.diagnostic.json").read_text()
                    self.assertNotIn("SECRET", sidecar)
                    self.assertNotIn(".local", sidecar)
                    self.assertNotIn("excerpt", sidecar)
                    self.assertEqual(json.loads(sidecar)["schema_version"], 1)
                    self.assertEqual(result.stderr, "")
                    self.assertIn(payload, (root / "private.log").read_text())

    def test_a_refused_pin_wins_over_markers_earlier_in_the_log(self):
        with tempfile.TemporaryDirectory() as folder:
            code = ("import sys; print('No such file or directory'); "
                    "print('x.json is not a 26.3 version file Sunrise knows (sha1 0)'); sys.exit(1)")
            result = self.invoke(Path(folder), code)
            self.assertEqual(result.stdout, self.diagnostic("resource-export", 1, "pinned-version-differs",
                                                            "pinned-version-differs"))

    def test_a_missing_or_too_old_jdk_is_one_fixed_reason_without_its_paths(self):
        messages = ["Java 25 is needed (javac, e.g. Eclipse Temurin 25). Install it, or set JAVA_HOME to it, and try again."
                    " Found: JAVA_HOME (/home/SECRET/jdk/bin) has no compatible Java tools.",
                    "/home/SECRET/.local/jdk/bin/java is Java 21; Java 25 or newer is needed"]
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            for message in messages:
                with self.subTest(message=message):
                    # The refusal wins over a marker earlier in the log, as a missing file the search passed over.
                    code = f"import sys; print('No such file or directory'); print({message!r}); sys.exit(1)"
                    result = self.invoke(root, code)
                    self.assertEqual(result.stdout, self.diagnostic("resource-export", 1, "missing-java", "missing-java"))
                    sidecar = (root / "private.diagnostic.json").read_text()
                    self.assertEqual(json.loads(sidecar)["reason_hint"], "missing-java")
                    for private in ("SECRET", ".local", "/home"):
                        self.assertNotIn(private, result.stdout + result.stderr + sidecar)

    def test_safe_last_line_and_key_validation(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            for key, expected in [("0123456789abcdef", "0123456789abcdef"),
                                  ("0123456789abcdefSECRET", "unavailable-or-invalid"),
                                  (".local/minecraft/secret", "unavailable-or-invalid"),
                                  ("", "unavailable-or-invalid")]:
                result = self.invoke(root, "import sys; print('Permission denied'); sys.exit(2)",
                                     SUNRISE_DIAGNOSTIC_EXPECTED_KEY=key,
                                     SUNRISE_DIAGNOSTIC_CACHE_STATE="missing-entry-list")
                self.assertEqual(result.returncode, 2)
                self.assertTrue(result.stdout.endswith("\nPermission denied\n"))
                data = json.loads((root / "private.diagnostic.json").read_text())
                self.assertEqual(data["expected_key"], expected)
                self.assertEqual(data["found_key"], "not-recorded")
                self.assertEqual(data["cache_state"], "missing-entry-list")
                self.assertNotIn("SECRET", result.stdout + result.stderr + json.dumps(data))
                self.assertNotIn(".local", result.stdout + result.stderr + json.dumps(data))

    def test_success_never_prints_private_output(self):
        with tempfile.TemporaryDirectory() as folder:
            result = self.invoke(Path(folder), "print('SECRET No space left on device')")
            self.assertEqual((result.returncode, result.stdout, result.stderr), (0, "", ""))

    def test_missing_log_parent_has_safe_failure(self):
        with tempfile.TemporaryDirectory() as folder:
            result = self.invoke(Path(folder) / "absent", "raise AssertionError('must not run')")
            self.assertEqual(result.returncode, 1)
            self.assertEqual(result.stdout, self.diagnostic("resource-export", 1, "missing-file") + "preparation diagnostic sidecar unavailable\n")
            self.assertEqual(result.stderr, "")

    def test_signal_and_step_identity(self):
        # A command ended by SIGTERM reports -15 (subprocess's way to say a signal ended it); only POSIX has that.
        ended_by_sigterm = subprocess.CompletedProcess([], -15)
        with tempfile.TemporaryDirectory() as folder, \
                mock.patch.object(preparation_step.subprocess, "run", return_value=ended_by_sigterm), \
                contextlib.redirect_stdout(io.StringIO()) as out:
            status = preparation_step.run("preparer-build", Path(folder) / "private.log", ["command"])
        self.assertEqual(status, 143)
        self.assertEqual(out.getvalue(), self.diagnostic("preparer-build", -15, "unclassified"))

    @needs_runner_shell
    def test_shell_cleanup_preserves_failure(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            (root / "tools/ci").mkdir(parents=True)
            for name in ("vanilla-data.sh", "preparation_step.py"):
                shutil.copy(SCRIPT.parent / name, root / "tools/ci" / name)
            (root / "research-lock.json").write_text('{"version": "26.3"}\n')
            (root / "source/minecraft/26.3").mkdir(parents=True)
            (root / "source/minecraft/26.3/prepared.json").write_text('{}')
            (root / "bin").mkdir()
            cargo = root / "bin/cargo"
            cargo.write_text('#!/bin/sh\nexit 0\n')
            cargo.chmod(0o755)
            (root / "target/release").mkdir(parents=True)
            prepare = root / "target/release/sunrise-prepare"
            prepare.write_text('#!/bin/sh\nmkdir .local/scratch\necho "SECRET No space left on device"\nexit 23\n')
            prepare.chmod(0o755)
            env = dict(os.environ, SUNRISE_DYNA_CACHE="true", SUNRISE_PREPARED_CACHE=str(root / "cache"),
                       SUNRISE_SHARED_SOURCE=str(root / "source"), SUNRISE_PREPARE_KEY="0123456789abcdef",
                       RUNNER_TEMP=str(root), CARGO_TARGET_DIR=str(root / "target"),
                       PATH=str(root / "bin") + os.pathsep + os.environ["PATH"])
            result = subprocess.run(["bash", "tools/ci/vanilla-data.sh"], cwd=root, env=env,
                                    capture_output=True, text=True)
            self.assertEqual(result.returncode, 23, result.stderr)
            self.assertIn('step=resource-export exit=23 reason_hint=disk-full', result.stdout)
            self.assertNotIn('SECRET', result.stdout + result.stderr)
            self.assertFalse((root / '.local/scratch').exists())
            self.assertFalse((root / 'cache/prepared/0123456789abcdef').exists())
            self.assertIn('expected_key=0123456789abcdef found_key=not-recorded cache_state=missing-entry-list', result.stdout)
            stale = root / 'cache/prepared/0123456789abcdef'
            stale.mkdir()
            (stale / '.entries').write_text('other-output\n')
            again = subprocess.run(["bash", "tools/ci/vanilla-data.sh"], cwd=root, env=env,
                                   capture_output=True, text=True)
            self.assertEqual(again.returncode, 23, again.stderr)
            self.assertIn('cache_state=missing-sunrise-client', again.stdout)
            self.assertNotIn('SECRET', again.stdout + again.stderr)
            self.assertTrue((root / 'source/minecraft/26.3/prepared.json').exists())

    def checkout_with_preparer(self, root):
        """The environment of a Dyna job in `root`: a script copy, a shared source, and a preparer that writes one file."""
        (root / "tools/ci").mkdir(parents=True)
        for name in ("vanilla-data.sh", "preparation_step.py", "seed_prepared_extras.py"):
            shutil.copy(SCRIPT.parent / name, root / "tools/ci" / name)
        shutil.copy(SCRIPT.parents[1] / "prepare_output.py", root / "tools/prepare_output.py")
        (root / "research-lock.json").write_text('{"version": "26.3"}\n')
        (root / "source/minecraft/26.3").mkdir(parents=True)
        (root / "source/minecraft/26.3/prepared.json").write_text('{}')
        (root / "source/sunrise-client/26.3").mkdir(parents=True)
        (root / "bin").mkdir()
        cargo = root / "bin/cargo"
        cargo.write_text('#!/bin/sh\nexit 0\n')
        cargo.chmod(0o755)
        (root / "target/release").mkdir(parents=True)
        prepare = root / "target/release/sunrise-prepare"
        prepare.write_text('#!/bin/sh\nmkdir -p .local/sunrise-client/26.3\necho one > .local/sunrise-client/26.3/a\n'
                           'echo "prepared" >> "$PREPARED_RUNS"\n')
        prepare.chmod(0o755)
        return dict(os.environ, SUNRISE_DYNA_CACHE="true", SUNRISE_PREPARED_CACHE=str(root / "cache"),
                    SUNRISE_SHARED_SOURCE=str(root / "source"), SUNRISE_PREPARE_KEY="0123456789abcdef",
                    RUNNER_TEMP=str(root), CARGO_TARGET_DIR=str(root / "target"),
                    PREPARED_RUNS=str(root / "runs"), PATH=str(root / "bin") + os.pathsep + os.environ["PATH"])

    @needs_runner_shell
    def test_the_private_copy_on_darwin_asks_for_a_clone_not_a_reflink(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            env = self.checkout_with_preparer(root)
            real_cp = shutil.which("cp")
            self.assertIsNotNone(real_cp)
            real_sort = shutil.which("sort")
            self.assertIsNotNone(real_sort)
            uname = root / "bin/uname"
            uname.write_text("#!/bin/sh\necho Darwin\n")
            # BSD's cp has no --reflink; GNU's has no -c, so the stand-in copies the last two arguments with `cp -a`.
            cp = root / "bin/cp"
            cp.write_text('#!/bin/sh\necho "$*" >> "$CP_CALLS"\n'
                          'for arg in "$@"; do\n'
                          '  case "$arg" in --reflink*) echo "cp: unrecognized option $arg" >&2; exit 64 ;; esac\n'
                          'done\n'
                          'shift $(( $# - 2 ))\n'
                          'exec "$REAL_CP" -a "$1" "$2"\n')
            # Stock macOS has no GNU sha256sum (shasum is there) and its sort has no -z.
            sha256sum = root / "bin/sha256sum"
            sha256sum.write_text('#!/bin/sh\necho "sha256sum: command not found on macOS" >&2\nexit 127\n')
            shasum = root / "bin/shasum"
            shasum.write_text('#!/bin/sh\nexec "$PYTHON" -c \'import hashlib, sys; '
                              'print(hashlib.sha256(sys.stdin.buffer.read()).hexdigest() + "  -")\'\n')
            sort = root / "bin/sort"
            sort.write_text('#!/bin/sh\n'
                            'for arg in "$@"; do\n'
                            '  case "$arg" in -z|--zero-terminated) echo "sort: unknown option -- z" >&2; exit 2 ;; esac\n'
                            'done\n'
                            'exec "$REAL_SORT" "$@"\n')
            for stub in (uname, cp, sha256sum, shasum, sort):
                stub.chmod(0o755)
            env["REAL_CP"] = real_cp
            env["REAL_SORT"] = real_sort
            env["PYTHON"] = sys.executable
            env["CP_CALLS"] = str(root / "cp-calls")
            for _ in range(2):
                result = subprocess.run(["bash", "tools/ci/vanilla-data.sh"], cwd=root, env=env,
                                        capture_output=True, text=True)
                self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            # The second run reuses the kept tree: its private copy holds the preparation.
            self.assertIn("fresh", result.stdout)
            self.assertEqual((root / ".local/sunrise-client/26.3/a").read_text(), "one\n")
            self.assertEqual((root / ".local/sunrise-client").resolve(), (root / "cache/job/sunrise-client").resolve())
            calls = (root / "cp-calls").read_text().splitlines()
            self.assertEqual(len(calls), 2, calls)
            self.assertTrue(all(call.startswith("-cRpP ") for call in calls), calls)

    @needs_runner_shell
    def test_a_kept_preparation_is_reused_only_unchanged_and_each_job_gets_its_own_copy(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            env = self.checkout_with_preparer(root)

            def run():
                result = subprocess.run(["bash", "tools/ci/vanilla-data.sh"], cwd=root, env=env,
                                        capture_output=True, text=True)
                self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
                return result.stdout

            kept = root / "cache/prepared/0123456789abcdef"
            run()
            self.assertEqual((root / "runs").read_text().count("prepared"), 1)
            # The job reads its own copy, never the kept tree.
            self.assertEqual((root / ".local/sunrise-client").resolve(), (root / "cache/job/sunrise-client").resolve())
            (root / ".local/sunrise-client/26.3/written-by-a-test").write_text("x")
            self.assertFalse((kept / "sunrise-client/26.3/written-by-a-test").exists())
            self.assertIn("fresh", run())
            self.assertEqual((root / "runs").read_text().count("prepared"), 1)
            self.assertFalse((root / ".local/sunrise-client/26.3/written-by-a-test").exists())
            # A kept tree changed behind the script's back (or kept from before manifests) is prepared again.
            (kept / "sunrise-client/26.3/extra").write_text("x")
            again = run()
            self.assertIn("does not match its manifest", again)
            self.assertIn("files added 1, removed 0, changed 0", again)
            self.assertEqual((root / "runs").read_text().count("prepared"), 2)
            self.assertFalse((kept / "sunrise-client/26.3/extra").exists())
            (kept / ".manifest").unlink()
            (kept / ".manifest-files").unlink()
            self.assertIn("expected none", run())
            self.assertEqual((root / "runs").read_text().count("prepared"), 3)

    def run_file_digests(self, tree):
        """vanilla-data.sh's own file_digests, run as the script runs it (set -euo pipefail)."""
        text = SCRIPT.parent.joinpath("vanilla-data.sh").read_text()
        start = text.index("file_digests() {")
        function = text[start:text.index("\n}\n", start) + 3]
        return subprocess.run(["bash", "-c", "set -euo pipefail\n" + function + 'file_digests "$1" | cat', "_", str(tree)],
                              capture_output=True)

    @needs_runner_shell
    def test_file_digests_escapes_names_like_gnu_sha256sum(self):
        with tempfile.TemporaryDirectory() as folder:
            tree = Path(folder)
            # (name on disk, name as printed, escaped): NTFS forbids backslash, newline and CR in names, so those
            # are tried only off Windows; the rest runs everywhere, Git Bash included.
            cases = [("plain", b"./plain", False), ("sub/nested", b"./sub/nested", False),
                     ("with space", b"./with space", False)]
            if os.name != "nt":
                cases += [("back\\slash", b"./back\\\\slash", True), ("new\nline", b"./new\\nline", True),
                          ("cr\rname", b"./cr\\rname", True), ("literal\\n", b"./literal\\\\n", True)]
            expected = []
            for name, printed, escaped in cases:
                path = tree / name
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_bytes(name.encode())
                digest = hashlib.sha256(name.encode()).hexdigest().encode()
                expected.append((b"\\" if escaped else b"") + digest + b"  " + printed)
            (tree / ".manifest-files").write_bytes(b"left out")
            result = self.run_file_digests(tree)
            self.assertEqual(result.returncode, 0, result.stderr)
            lines = result.stdout.split(b"\n")
            self.assertEqual(lines.pop(), b"")
            self.assertEqual(sorted(lines), sorted(expected))

    @needs_runner_shell
    def test_file_digests_fails_closed_on_an_unreadable_directory(self):
        with tempfile.TemporaryDirectory() as folder:
            missing = self.run_file_digests(Path(folder) / "absent")
            self.assertNotEqual(missing.returncode, 0, missing.stdout)
            # Windows' chmod cannot make a directory unreadable, and root reads a mode 000 one.
            if os.name == "posix" and os.geteuid() != 0:
                tree = Path(folder) / "tree"
                tree.mkdir()
                (tree / "readable").write_text("x")
                locked = tree / "locked"
                locked.mkdir()
                (locked / "hidden").write_text("y")
                locked.chmod(0o000)
                try:
                    result = self.run_file_digests(tree)
                finally:
                    locked.chmod(0o755)
                self.assertNotEqual(result.returncode, 0, result.stdout)
                self.assertIn(b"PermissionError", result.stderr)

    def test_bounded_log_does_not_hide_exit(self):
        with tempfile.TemporaryDirectory() as folder:
            result = self.invoke(Path(folder), "import sys; print('x' * 1100000); print('No space left on device SECRET'); sys.exit(7)", "seed-extras")
            self.assertEqual(result.returncode, 7)
            self.assertEqual(result.stdout, self.diagnostic("seed-extras", 7, "unclassified", "disk-full"))


if __name__ == "__main__":
    unittest.main()
