"""What the operating system running a test allows, for tests whose behaviour depends on it.

A test checks a capability here, never a platform name: Windows lets a process create a symbolic
link only with Developer Mode on or as an administrator, and its bare `bash` may be WSL's, which
cannot see the host's paths. macOS and Linux always allow both.
"""
import functools
import os
import shutil
import subprocess
import sys
import tempfile
import unittest


@functools.cache
def can_symlink():
    """Whether this process may create a symbolic link to a directory and to a file (Windows can refuse one but not the other)."""
    with tempfile.TemporaryDirectory() as directory:
        file = os.path.join(directory, "file")
        with open(file, "w"):
            pass
        try:
            os.symlink(directory, os.path.join(directory, "directory-link"), target_is_directory=True)
            os.symlink(file, os.path.join(directory, "file-link"), target_is_directory=False)
        except OSError:
            return False
    return True


@functools.cache
def has_fcntl():
    """Whether `fcntl.flock` exists here."""
    try:
        import fcntl
    except ImportError:
        return False
    return hasattr(fcntl, "flock")


@functools.cache
def can_pass_fds():
    """Whether `subprocess` can hand a file descriptor to a child here."""
    read, write = os.pipe()
    try:
        return subprocess.run([sys.executable, "-c", ""], pass_fds=(read,), stdin=subprocess.DEVNULL,
                              stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL).returncode == 0
    except (ValueError, NotImplementedError, OSError):
        return False
    finally:
        os.close(read)
        os.close(write)


@functools.cache
def has_ed25519_openssl(path=None):
    """Whether `openssl` on `path` (the inherited PATH when None) generates an Ed25519 key."""
    openssl = shutil.which("openssl", path=path)
    if openssl is None:
        return False
    try:
        result = subprocess.run([openssl, "genpkey", "-algorithm", "ed25519", "-outform", "DER"],
                                env=None if path is None else {"PATH": path}, stdin=subprocess.DEVNULL,
                                capture_output=True)
    except OSError:
        return False
    return result.returncode == 0 and len(result.stdout) > 0


@functools.cache
def has_gnu_timeout():
    """Whether `timeout` here is GNU's (accepts `--kill-after`), not Windows' pause utility of that name."""
    timeout = shutil.which("timeout")
    if timeout is None:
        return False
    try:
        return subprocess.run([timeout, "--kill-after=1s", "5", "true"], stdin=subprocess.DEVNULL,
                              stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL).returncode == 0
    except OSError:
        return False


# The bash on PATH. Windows' CreateProcess looks in System32 before PATH, so a bare "bash" there is WSL's (#5383),
# which cannot see the host's paths; Git Bash and every macOS or Linux bash can.
BASH = shutil.which("bash")
POSIX_BASH = BASH is not None and not (
    os.name == "nt" and os.path.dirname(os.path.normcase(BASH)).endswith(os.path.normcase(r"\system32")))

needs_runner_shell = unittest.skipUnless(POSIX_BASH, "needs a bash that sees this system's paths, not WSL's")
"""Skips a test that runs a workflow step or a `tools/ci/*.sh` script with bash where the only bash is WSL's."""


def needs_symlinks(test):
    """Skips `test` (a test method or class) where this process may not create a symbolic link."""
    return unittest.skipUnless(can_symlink(), "this system does not allow creating symbolic links")(test)


def needs_command(name, reason=None):
    """A decorator that skips a test where the command `name` is not on PATH."""
    return unittest.skipUnless(shutil.which(name) is not None, reason or f"needs {name}")


def needs_fcntl(test):
    """Skips `test` (a test method or class) where `fcntl.flock` does not exist."""
    return unittest.skipUnless(has_fcntl(), "needs fcntl.flock")(test)


def needs_pass_fds(test):
    """Skips `test` (a test method or class) where `subprocess` cannot pass a file descriptor to a child."""
    return unittest.skipUnless(can_pass_fds(), "needs subprocess pass_fds")(test)


def needs_ed25519_openssl(path=None):
    """A decorator that skips a test where `openssl` on `path` (the inherited PATH when None) lacks Ed25519."""
    return unittest.skipUnless(has_ed25519_openssl(path), "needs OpenSSL with Ed25519 support")


def needs_gnu_timeout(test):
    """Skips `test` (a test method or class) where `timeout` is missing or not GNU's (no `--kill-after`)."""
    return unittest.skipUnless(has_gnu_timeout(), "needs GNU timeout (--kill-after)")(test)
