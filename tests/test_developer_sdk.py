"""`tools/developer_sdk.sh` and the compiler cache it may share with later builds."""
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

from tests.git_fixture import isolated_git_env
from tests.os_support import BASH, needs_runner_shell

ROOT = Path(__file__).resolve().parents[1]


class CompilerCacheTests(unittest.TestCase):
    def run_sdk(self, wrapper_name):
        """Run the script in a one-commit checkout whose Rust pin no `rustc` matches, so it stops
        after setting up its temporary root. Return its result, the TMPDIR each `--start-server`
        call saw, the caller's TMPDIR and the SDK base's parent folder."""
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            checkout = root / "checkout"
            (checkout / "tools/ci").mkdir(parents=True)
            script = (ROOT / "tools/developer_sdk.sh").read_bytes().replace(b"\r\n", b"\n")
            (checkout / "tools/developer_sdk.sh").write_bytes(script)
            (checkout / "tools/ci/rust-version").write_text("0.0.0\n")
            git = ["git", "-c", "user.name=t", "-c", "user.email=t@t", "-C", str(checkout)]
            env = isolated_git_env()
            subprocess.run([*git, "init", "-q"], env=env, check=True)
            subprocess.run([*git, "add", "."], env=env, check=True)
            subprocess.run([*git, "commit", "-q", "-m", "sdk"], env=env, check=True)

            stubs = root / "stubs"
            stubs.mkdir()
            log = root / "started"
            caller_tmp = root / "caller-tmp"
            caller_tmp.mkdir()
            for name, body in {
                wrapper_name: '[[ "$1" == --start-server ]] && printf "%s\\n" "$TMPDIR" >> "$LOG"\n',
                "rustc": 'echo "rustc 9.9.9 (stub)"\n',
            }.items():
                path = stubs / name
                path.write_text(f"#!/bin/bash\n{body}")
                path.chmod(0o755)
            out = root / "sdks" / "local"
            env = isolated_git_env(
                PATH=f"{stubs}{os.pathsep}{os.environ['PATH']}",
                RUSTC_WRAPPER=str(stubs / wrapper_name),
                TMPDIR=str(caller_tmp),
                LOG=str(log),
                CARGO_TARGET_DIR=str(root / "target"),
            )
            result = subprocess.run([BASH, "tools/developer_sdk.sh", str(out)], cwd=checkout,
                                    env=env, text=True, capture_output=True)
            started = log.read_text().splitlines() if log.exists() else []
            left = sorted(p.name for p in out.parent.iterdir()) if out.parent.exists() else []
            return result, started, str(caller_tmp), left

    @needs_runner_shell
    def test_an_sccache_server_starts_with_the_callers_tmpdir(self):
        """#6629: the server outlives the script, which deletes its own temporary root, so it is
        started before that root exists and keeps the caller's TMPDIR."""
        result, started, caller_tmp, left = self.run_sdk("sccache")
        self.assertEqual(result.returncode, 1, result.stderr)
        self.assertIn("Build the SDK with Rust 0.0.0", result.stderr)
        # Compared by name: Git Bash on Windows spells the same folder in its own form.
        self.assertEqual(len(started), 1, started)
        self.assertEqual(started[0].replace("\\", "/").rsplit("/", 1)[-1], Path(caller_tmp).name)
        self.assertNotIn(".developer-sdk.", started[0])
        self.assertEqual(left, [], "the script's temporary root is removed on exit")

    @needs_runner_shell
    def test_another_wrapper_is_not_started(self):
        result, started, _, _ = self.run_sdk("some-other-wrapper")
        self.assertEqual(result.returncode, 1, result.stderr)
        self.assertEqual(started, [])


if __name__ == "__main__":
    unittest.main()
