"""tests/os_support.py: the capability probes answer from what the system does, and decorators skip on that answer."""
import importlib
import importlib.util
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import types
import unittest
from unittest import mock

from tests import os_support


def skipped(decorator):
    """The skip reason `decorator` gives a test, or None when it leaves the test running."""
    @decorator
    def test():
        pass
    return getattr(test, "__unittest_skip_why__", None) if getattr(test, "__unittest_skip__", False) else None


class CanSymlinkTests(unittest.TestCase):
    def can_symlink_with(self, refuse_files=False, refuse_dirs=False):
        """(can_symlink(), the target_is_directory of each symlink attempted, the needs_symlinks skip reason),
        with os.symlink replaced by a mock that never touches the host's symlink privileges."""
        calls = []

        def symlink(source, link, target_is_directory=False):
            calls.append(target_is_directory)
            if (refuse_dirs and target_is_directory) or (refuse_files and not target_is_directory):
                raise OSError("a privilege is required")

        os_support.can_symlink.cache_clear()
        try:
            with mock.patch.object(os_support.os, "symlink", symlink):
                answer = os_support.can_symlink()
                reason = skipped(os_support.needs_symlinks)
            return answer, calls, reason
        finally:
            os_support.can_symlink.cache_clear()

    def test_a_refused_file_symlink_fails_the_probe_though_a_directory_one_works(self):
        answer, calls, reason = self.can_symlink_with(refuse_files=True)
        self.assertFalse(answer)
        self.assertEqual(calls, [True, False])
        self.assertEqual(reason, "this system does not allow creating symbolic links")

    def test_a_refused_directory_symlink_fails_the_probe_without_trying_a_file_one(self):
        answer, calls, reason = self.can_symlink_with(refuse_dirs=True)
        self.assertFalse(answer)
        self.assertEqual(calls, [True])
        self.assertEqual(reason, "this system does not allow creating symbolic links")

    def test_both_kinds_of_symlink_passing_succeeds_the_probe(self):
        answer, calls, reason = self.can_symlink_with()
        self.assertTrue(answer)
        self.assertEqual(calls, [True, False])
        self.assertIsNone(reason)


class NeedsCommandTests(unittest.TestCase):
    def test_an_existing_command_runs_the_test_and_a_missing_one_skips_it(self):
        python = Path(sys.executable)
        with mock.patch.dict(os.environ, PATH=str(python.parent)):
            self.assertIsNone(skipped(os_support.needs_command(python.name)))
            self.assertEqual(skipped(os_support.needs_command("no-such-command-6060")), "needs no-such-command-6060")
            self.assertEqual(skipped(os_support.needs_command("no-such-command-6060", "because")), "because")


class Ed25519OpenSSLTests(unittest.TestCase):
    OPENSSL = "/fake/bin/openssl"
    ARGV = [OPENSSL, "genpkey", "-algorithm", "ed25519", "-outform", "DER"]

    def probe(self, path="/fake/bin", which=OPENSSL, **run):
        """has_ed25519_openssl(path) with `which` finding openssl (or nothing) and subprocess.run doing `run`; returns (answer, run mock)."""
        os_support.has_ed25519_openssl.cache_clear()
        try:
            with mock.patch.object(os_support.shutil, "which", return_value=which) as found, mock.patch.object(
                    os_support.subprocess, "run", **run) as ran:
                answer = os_support.has_ed25519_openssl(path)
            found.assert_called_once_with("openssl", path=path)
            return answer, ran
        finally:
            os_support.has_ed25519_openssl.cache_clear()

    def completed(self, returncode, stdout):
        return {"return_value": subprocess.CompletedProcess([], returncode, stdout=stdout, stderr=b"")}

    def test_the_key_is_generated_with_exactly_this_command_and_only_the_given_path(self):
        answer, ran = self.probe(path="/fake/bin", **self.completed(0, b"key"))
        self.assertTrue(answer)
        ran.assert_called_once_with(self.ARGV, env={"PATH": "/fake/bin"}, stdin=subprocess.DEVNULL, capture_output=True)

    def test_the_inherited_environment_is_used_when_no_path_is_given(self):
        answer, ran = self.probe(path=None, **self.completed(0, b"key"))
        self.assertTrue(answer)
        ran.assert_called_once_with(self.ARGV, env=None, stdin=subprocess.DEVNULL, capture_output=True)

    def test_only_an_openssl_that_exits_cleanly_with_a_key_passes(self):
        self.assertFalse(self.probe(**self.completed(1, b""))[0])
        self.assertFalse(self.probe(**self.completed(1, b"key"))[0])
        self.assertFalse(self.probe(**self.completed(0, b""))[0])
        self.assertTrue(self.probe(**self.completed(0, b"key"))[0])

    def test_a_broken_or_non_executable_openssl_is_false_not_an_error(self):
        answer, ran = self.probe(side_effect=OSError("exec format error"))
        self.assertFalse(answer)
        ran.assert_called_once()

    def test_a_path_without_openssl_is_false_and_runs_nothing(self):
        answer, ran = self.probe(which=None)
        self.assertFalse(answer)
        ran.assert_not_called()

    def test_an_empty_directory_has_no_openssl(self):
        os_support.has_ed25519_openssl.cache_clear()
        try:
            with tempfile.TemporaryDirectory() as directory:
                self.assertIs(os_support.has_ed25519_openssl(directory), False)
        finally:
            os_support.has_ed25519_openssl.cache_clear()

    def test_the_decorator_runs_a_test_where_the_probe_passes(self):
        with mock.patch.object(os_support, "has_ed25519_openssl", return_value=True):
            self.assertIsNone(skipped(os_support.needs_ed25519_openssl("/fake/bin")))

    def test_the_decorator_skips_with_its_reason_where_the_probe_fails(self):
        with tempfile.TemporaryDirectory() as directory:
            self.assertEqual(skipped(os_support.needs_ed25519_openssl(directory)), "needs OpenSSL with Ed25519 support")


class GnuTimeoutTests(unittest.TestCase):
    TIMEOUT = "/fake/bin/timeout"

    def probe(self, which=TIMEOUT, **run):
        """has_gnu_timeout() with `which` finding timeout (or nothing) and subprocess.run doing `run`; returns (answer, run mock)."""
        os_support.has_gnu_timeout.cache_clear()
        try:
            with mock.patch.object(os_support.shutil, "which", return_value=which) as found, mock.patch.object(
                    os_support.subprocess, "run", **run) as ran:
                answer = os_support.has_gnu_timeout()
                found.assert_called_once_with("timeout")
                reason = skipped(os_support.needs_gnu_timeout)
            return answer, ran, reason
        finally:
            os_support.has_gnu_timeout.cache_clear()

    def test_gnu_timeout_is_run_with_kill_after_and_exit_zero_passes(self):
        answer, ran, reason = self.probe(return_value=subprocess.CompletedProcess([], 0))
        self.assertTrue(answer)
        self.assertIsNone(reason)
        self.assertEqual(ran.call_args.args, ([self.TIMEOUT, "--kill-after=1s", "5", "true"],))

    def test_a_timeout_that_rejects_kill_after_like_windows_is_false(self):
        answer, _, reason = self.probe(return_value=subprocess.CompletedProcess([], 1))
        self.assertFalse(answer)
        self.assertEqual(reason, "needs GNU timeout (--kill-after)")

    def test_a_missing_true_is_false(self):
        answer, _, _ = self.probe(return_value=subprocess.CompletedProcess([], 127))
        self.assertFalse(answer)

    def test_a_timeout_that_cannot_start_is_false_not_an_error(self):
        answer, _, reason = self.probe(side_effect=OSError("exec format error"))
        self.assertFalse(answer)
        self.assertEqual(reason, "needs GNU timeout (--kill-after)")

    def test_no_timeout_on_path_is_false_and_runs_nothing(self):
        answer, ran, reason = self.probe(which=None)
        self.assertFalse(answer)
        self.assertEqual(reason, "needs GNU timeout (--kill-after)")
        ran.assert_not_called()


class CapabilityTests(unittest.TestCase):
    def test_has_fcntl_matches_whether_the_fcntl_module_has_flock(self):
        spec = importlib.util.find_spec("fcntl")
        expected = spec is not None and hasattr(importlib.import_module("fcntl"), "flock")
        self.assertIs(os_support.has_fcntl(), expected)
        self.assertEqual(skipped(os_support.needs_fcntl), None if expected else "needs fcntl.flock")

    def test_can_pass_fds_is_a_bool_and_true_wherever_fcntl_exists(self):
        self.assertIsInstance(os_support.can_pass_fds(), bool)
        if os_support.has_fcntl():
            self.assertTrue(os_support.can_pass_fds())
        self.assertEqual(skipped(os_support.needs_pass_fds), None if os_support.can_pass_fds() else "needs subprocess pass_fds")


class FailingCapabilityTests(unittest.TestCase):
    def test_a_missing_fcntl_or_one_without_flock_is_false(self):
        os_support.has_fcntl.cache_clear()
        try:
            with mock.patch.dict(sys.modules, {"fcntl": None}):
                self.assertFalse(os_support.has_fcntl())
            os_support.has_fcntl.cache_clear()
            with mock.patch.dict(sys.modules, {"fcntl": types.ModuleType("fcntl")}):
                self.assertFalse(os_support.has_fcntl())
                self.assertEqual(skipped(os_support.needs_fcntl), "needs fcntl.flock")
        finally:
            os_support.has_fcntl.cache_clear()

    def test_pass_fds_is_false_when_the_child_cannot_start_or_fails_and_the_pipe_is_closed(self):
        for outcome in (ValueError("pass_fds is not supported"), NotImplementedError(), OSError("no fork"),
                        subprocess.CompletedProcess([], 1)):
            with self.subTest(outcome=outcome):
                pipes = []
                real_pipe = os.pipe

                def pipe():
                    pipes.extend(real_pipe())
                    return tuple(pipes)

                os_support.can_pass_fds.cache_clear()
                try:
                    with mock.patch.object(os_support.os, "pipe", pipe), mock.patch.object(
                            os_support.subprocess, "run",
                            side_effect=outcome if isinstance(outcome, Exception) else None,
                            return_value=None if isinstance(outcome, Exception) else outcome):
                        self.assertFalse(os_support.can_pass_fds())
                        self.assertEqual(skipped(os_support.needs_pass_fds), "needs subprocess pass_fds")
                finally:
                    os_support.can_pass_fds.cache_clear()
                for descriptor in pipes:
                    with self.assertRaises(OSError):
                        os.fstat(descriptor)


if __name__ == "__main__":
    unittest.main()
