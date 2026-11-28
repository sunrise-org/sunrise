"""The launcher mirror hash follows the public surface, and a change needs its announcement."""
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest

from tests.git_fixture import git_in, isolated_git_env, merge_pull_request
from tools.ci import check_launcher_mirrors as mirrors

ROOT = Path(__file__).resolve().parents[1]
WRITE = "python3 tools/ci/check_launcher_mirrors.py --write"


class MirrorCheckout:
    """A throwaway git checkout holding the check, one mirrored crate and its recorded surface, committed as the base."""

    def __init__(self, test):
        directory = tempfile.TemporaryDirectory()
        test.addCleanup(directory.cleanup)
        self.root = Path(directory.name)
        self._git = git_in(self.root)
        for script in ("tools/ci/check_launcher_mirrors.py", "tools/rust_tokens.py", "tools/merge_base.py"):
            (self.root / script).parent.mkdir(parents=True, exist_ok=True)
            shutil.copy(ROOT / script, self.root / script)
        self.write("crates/mirror/src/lib.rs", "pub fn a() {}\n")
        self.write("crates/mirror2/src/lib.rs", "pub fn a() {}\n")
        self.write("tools/launcher_mirrors.toml",
                   'version = 1\n\n[[mirror]]\nlauncher = "mirror"\nsources = ["crates/mirror/src"]\n\n'
                   '[[mirror]]\nlauncher = "mirror2"\nsources = ["crates/mirror2/src"]\n')
        self.check("--write")
        self.git("init", "-q", "--template=")
        self.git("add", ".")
        self.git("commit", "-qm", "base")
        self.base = self.git("rev-parse", "HEAD").strip()

    def write(self, path, text):
        (self.root / path).parent.mkdir(parents=True, exist_ok=True)
        (self.root / path).write_text(text, encoding="utf-8", newline="\n")

    def git(self, *arguments):
        return self._git("-c", "user.name=Test", "-c", "user.email=test@example.invalid", *arguments)

    def run(self, *command):
        """A `python3 ...` command line, run in this checkout."""
        return subprocess.run([sys.executable, *command[1:]], cwd=self.root, env=isolated_git_env(),
                              capture_output=True, text=True, encoding="utf-8")

    def check(self, *arguments):
        return self.run("python3", "tools/ci/check_launcher_mirrors.py", *arguments)

    def change_surface(self, record):
        """A new public function in the mirrored crate; `record` also writes the manifest, as `--write` does."""
        self.write("crates/mirror/src/lib.rs", "pub fn a() {}\npub fn b() {}\n")
        if record:
            self.check("--write")

    def change_second_surface(self, record):
        """The same, in the second mirrored crate."""
        self.write("crates/mirror2/src/lib.rs", "pub fn a() {}\npub fn b() {}\n")
        if record:
            self.check("--write")

    def merge_pull_request(self, main_change=lambda: None, pr_change=lambda: None):
        """HEAD becomes CI's pull-request merge (two parents) over `self.base`: the pull request makes `pr_change`,
        main moved on with `main_change` (which may record a new surface). `self.base` stays the event's stale base."""
        return merge_pull_request(self.root, self.base, ("crates", "tools"), main_change, pr_change)

    def change_privately(self):
        self.write("crates/mirror/src/lib.rs", "pub fn a() { let _ = 1; }\nfn hidden() {}\n")


MANIFEST = {
    "version": 1,
    "mirror": [
        {"launcher": "smod", "sources": ["crates/sunrise-mod-package/src"], "surface": "sha256:a"},
        {"launcher": "proxy", "sources": [], "note": "only in the launcher"},
    ],
}


class LauncherMirrorTests(unittest.TestCase):
    def test_a_private_change_keeps_the_surface(self):
        before = "\n".join(mirrors.public_surface("pub struct S;\npub fn f() -> u8 { 1 }\n"))
        after = "\n".join(mirrors.public_surface("pub struct S;\npub fn f() -> u8 { 2 }\nfn hidden() {}\n"))
        self.assertEqual(before, after)

    def test_a_public_change_moves_the_surface(self):
        before = "\n".join(mirrors.public_surface("pub fn f() -> u8 { 1 }\n"))
        after = "\n".join(mirrors.public_surface("pub fn f() -> u16 { 1 }\n"))
        self.assertNotEqual(before, after)

    def test_a_new_public_item_moves_the_surface(self):
        before = "\n".join(mirrors.public_surface("pub fn f() {}\n"))
        after = "\n".join(mirrors.public_surface("pub fn f() {}\npub fn g() {}\n"))
        self.assertNotEqual(before, after)

    def test_a_reexported_name_moves_the_surface(self):
        before = "\n".join(mirrors.public_surface("pub use crate::{a, b};\n"))
        after = "\n".join(mirrors.public_surface("pub use crate::{a, b, c};\n"))
        self.assertNotEqual(before, after)

    def test_a_mismatched_hash_fails(self):
        problems = mirrors.check(MANIFEST, {"smod": "sha256:b"})
        self.assertIn(f"Run `{WRITE}`", problems[0])

    def test_a_matching_hash_passes(self):
        self.assertEqual(mirrors.check(MANIFEST, {"smod": "sha256:a"}), [])

    def test_a_changed_surface_needs_the_marker_in_the_pull_request_body(self):
        base = dict(MANIFEST, mirror=[dict(MANIFEST["mirror"][0], surface="sha256:old")])
        config = dict(MANIFEST, mirror=[dict(MANIFEST["mirror"][0], surface="sha256:a")])
        self.assertTrue(any("launcher-mirror:" in problem
                            for problem in mirrors.check(config, {"smod": "sha256:a"}, base, "")))
        announced = "launcher-mirror: the launcher must take the new field\n"
        self.assertEqual(mirrors.check(config, {"smod": "sha256:a"}, base, announced), [])

    def test_a_stale_manifest_fails_with_the_write_command_even_when_reminding(self):
        checkout = MirrorCheckout(self)
        checkout.change_surface(record=False)
        for arguments in (("--base", checkout.base, "--remind-body"), ("--base", checkout.base, "--pr-body-file", "x")):
            checkout.write("x", "launcher-mirror: the copy follows\n")
            result = checkout.check(*arguments)
            self.assertEqual(result.returncode, 1, result.stdout)
            self.assertIn(f"Run `{WRITE}`", result.stdout)
            self.assertIn("mirror: public surface is", result.stdout)

    def test_a_changed_surface_passes_with_a_reminder_when_the_run_has_no_body(self):
        checkout = MirrorCheckout(self)
        checkout.change_surface(record=True)
        result = checkout.check("--base", checkout.base, "--remind-body")
        self.assertEqual(result.returncode, 0, result.stdout)
        self.assertIn("Reminder: launcher mirrors changed (mirror)", result.stdout)
        self.assertIn("launcher-mirror:", result.stdout)

    def test_an_unchanged_surface_passes_without_a_reminder(self):
        checkout = MirrorCheckout(self)
        checkout.change_privately()
        result = checkout.check("--base", checkout.base, "--remind-body")
        self.assertEqual(result.returncode, 0, result.stdout)
        self.assertNotIn("Reminder", result.stdout)

    def test_a_body_is_still_required_where_one_is_given(self):
        checkout = MirrorCheckout(self)
        checkout.change_surface(record=True)
        checkout.write("empty.md", "")
        checkout.write("announced.md", "Text\nlauncher-mirror: the copy follows\n")
        failed = checkout.check("--base", checkout.base, "--pr-body-file", "empty.md")
        self.assertEqual(failed.returncode, 1, failed.stdout)
        self.assertIn("without a line starting launcher-mirror:", failed.stdout)
        passed = checkout.check("--base", checkout.base, "--pr-body-file", "announced.md")
        self.assertEqual(passed.returncode, 0, passed.stdout)
        self.assertNotIn("Reminder", passed.stdout)

    def test_reminding_needs_a_base_and_takes_no_body(self):
        checkout = MirrorCheckout(self)
        checkout.write("empty.md", "")
        for arguments in (("--remind-body",), ("--base", checkout.base, "--remind-body", "--pr-body-file", "empty.md"),
                          ("--base", checkout.base, "--remind-body", "--event", "empty.md")):
            with self.subTest(arguments=arguments):
                self.assertEqual(checkout.check(*arguments).returncode, 2)

    def merged_after_main_changed_a_mirror(self, pr_change):
        """#6245: main changed `mirror` (manifest recorded) after the event's base, and the pull request is merged."""
        checkout = MirrorCheckout(self)
        checkout.write("empty.md", "")
        checkout.write("announced.md", "launcher-mirror: the copy follows\n")
        checkout.merge_pull_request(main_change=lambda: checkout.change_surface(record=True),
                                       pr_change=lambda: pr_change(checkout))
        return checkout

    def test_a_pull_request_is_judged_against_main_as_it_is_in_the_merge_not_the_stale_event_base(self):
        checkout = self.merged_after_main_changed_a_mirror(
            lambda checkout: checkout.write("crates/unrelated/note.txt", "x\n"))
        passed = checkout.check("--base", checkout.base, "--pull-request-merge", "--pr-body-file", "empty.md")
        self.assertEqual(passed.returncode, 0, passed.stdout + passed.stderr)
        # The same run without the flag blames the pull request for main's change.
        failed = checkout.check("--base", checkout.base, "--pr-body-file", "empty.md")
        self.assertEqual(failed.returncode, 1, failed.stdout)
        self.assertIn("launcher mirrors changed (mirror)", failed.stdout)

    def test_a_pull_request_that_changes_a_second_mirror_is_named_alone_until_it_announces_it(self):
        checkout = self.merged_after_main_changed_a_mirror(
            lambda checkout: checkout.change_second_surface(record=True))
        failed = checkout.check("--base", checkout.base, "--pull-request-merge", "--pr-body-file", "empty.md")
        self.assertEqual(failed.returncode, 1, failed.stdout)
        self.assertIn("launcher mirrors changed (mirror2)", failed.stdout)
        passed = checkout.check("--base", checkout.base, "--pull-request-merge", "--pr-body-file", "announced.md")
        self.assertEqual(passed.returncode, 0, passed.stdout + passed.stderr)

    def test_the_pull_request_merge_needs_a_merge_of_a_descendant_of_the_base(self):
        checkout = MirrorCheckout(self)
        checkout.write("empty.md", "")
        single = checkout.check("--base", checkout.base, "--pull-request-merge", "--pr-body-file", "empty.md")
        self.assertEqual(single.returncode, 1, single.stdout)
        self.assertIn("--pull-request-merge needs HEAD to merge the pull request into a descendant of --base",
                      single.stdout)
        pull_request, _ = checkout.merge_pull_request(pr_change=lambda: checkout.write("crates/unrelated/a.txt", "x"))
        # The pull request's own commit is no ancestor of the merge's first parent.
        wrong = checkout.check("--base", pull_request, "--pull-request-merge", "--pr-body-file", "empty.md")
        self.assertEqual(wrong.returncode, 1, wrong.stdout)
        self.assertIn("descendant of --base", wrong.stdout)
        # A single-parent commit on top of a merge is not a pull request merge either.
        checkout.write("crates/unrelated/b.txt", "x")
        checkout.git("add", "crates")
        checkout.git("commit", "-qm", "single parent")
        single = checkout.check("--base", checkout.base, "--pull-request-merge", "--pr-body-file", "empty.md")
        self.assertEqual(single.returncode, 1, single.stdout)

    def test_the_pull_request_merge_takes_a_base(self):
        checkout = MirrorCheckout(self)
        result = checkout.check("--pull-request-merge")
        self.assertEqual(result.returncode, 2)
        self.assertIn("--pull-request-merge takes --base", result.stderr)

    def test_a_baseline_that_cannot_be_read_fails_closed(self):
        checkout = MirrorCheckout(self)
        checkout.write("empty.md", "")
        missing = checkout.check("--base", "0" * 40, "--pr-body-file", "empty.md")
        self.assertEqual(missing.returncode, 1, missing.stdout)
        self.assertNotIn("Launcher mirror check passed", missing.stdout)
        self.assertIn("cannot read", missing.stderr)
        # A gitlink at the manifest's path: the tree names it, git cannot read it.
        checkout.git("update-index", "--add", "--cacheinfo",
                     "160000,1111111111111111111111111111111111111111,tools/launcher_mirrors.toml")
        checkout.git("commit", "-qm", "gitlink")
        corrupt = checkout.git("rev-parse", "HEAD").strip()
        unreadable = checkout.check("--base", corrupt, "--pr-body-file", "empty.md")
        self.assertEqual(unreadable.returncode, 1, unreadable.stdout)
        self.assertIn("tools/launcher_mirrors.toml", unreadable.stderr)
        self.assertNotIn("Launcher mirror check passed", unreadable.stdout)

    def test_importing_the_check_leaves_sys_path_alone(self):
        script = ("import sys; before = list(sys.path); import tools.ci.check_launcher_mirrors; "
                  "assert sys.path == before, sys.path")
        result = subprocess.run([sys.executable, "-c", script], cwd=ROOT, capture_output=True, text=True)
        self.assertEqual(result.returncode, 0, result.stderr)

    def test_every_shipped_manifest_source_exists(self):
        manifest = mirrors.load_manifest()
        for mirror in manifest["mirror"]:
            for source in mirror.get("sources", []):
                with self.subTest(launcher=mirror["launcher"], source=source):
                    self.assertTrue(
                        (mirrors.ROOT / source).exists(),
                        f"{mirror['launcher']}: {source} is not in the repository",
                    )

    def test_a_mirror_with_no_source_is_not_hashed(self):
        hashes, notes = mirrors.measured(MANIFEST)
        self.assertNotIn("proxy", hashes)
        self.assertIn("proxy: no source", notes[0])


if __name__ == "__main__":
    unittest.main()
