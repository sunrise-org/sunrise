import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

SCRIPT = Path(__file__).resolve().parents[1] / "tools/ci/check_unwinding_methods.py"
LIST = "crates/sunrise-mod-api-gen/unwinding-methods.txt"


def git(cwd, *args):
    return subprocess.run(["git", "-c", "commit.gpgsign=false", "-c", "user.name=t", "-c", "user.email=t@example.com", *args],
                          cwd=cwd, check=True, capture_output=True, text=True).stdout.strip()


class UnwindingListRatchet(unittest.TestCase):
    def setUp(self):
        self.repo = Path(tempfile.mkdtemp())
        git(self.repo, "init", "-q")
        (self.repo / LIST).parent.mkdir(parents=True)

    def commit(self, text):
        if text is not None:
            (self.repo / LIST).write_text(text)
        git(self.repo, "add", "-A")
        git(self.repo, "commit", "-q", "--allow-empty", "-m", "c")
        return git(self.repo, "rev-parse", "HEAD")

    def check(self, base):
        return subprocess.run([sys.executable, str(SCRIPT), base], cwd=self.repo, capture_output=True, text=True)

    def test_a_grown_list_fails_and_a_shrunk_one_passes(self):
        base = self.commit("a::one\nb::two\n")
        self.commit("a::one\nb::two\nc::three\n")
        self.assertIn("c::three", self.check(base).stderr)
        shrunk = self.commit("a::one\n")
        self.assertEqual(self.check(base).returncode, 0)
        self.assertEqual(self.check(shrunk).returncode, 0)

    def test_a_base_without_the_list_is_a_new_list(self):
        base = self.commit(None)
        self.commit("a::one\n")
        self.assertEqual(self.check(base).returncode, 0)

    def test_an_unreadable_base_fails_instead_of_passing(self):
        self.commit("a::one\n")
        result = self.check("0" * 40)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("cannot be read", result.stderr)


if __name__ == "__main__":
    unittest.main()
