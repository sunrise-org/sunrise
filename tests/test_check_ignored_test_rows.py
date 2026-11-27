"""A `windows` row can leave Linux coverage only with proof from its Cargo target root."""
import os
import shutil
import subprocess
import sys
import tempfile
import threading
import unittest
from pathlib import Path
from unittest.mock import patch

from tools import merge_base
from tools.ci import check_ignored_test_rows as check, ignored_inventory, ignored_shards, ignored_tests


class WindowsTargetCoverage(unittest.TestCase):
    def setUp(self):
        scratch = self.enterContext(tempfile.TemporaryDirectory())
        self.root = Path(scratch)
        self.write("Cargo.toml", '[workspace]\nmembers = ["crates/game"]\n')
        self.write("crates/game/Cargo.toml", '[package]\nname = "game"\n')
        self.enterContext(patch.object(ignored_shards, "WORKSPACES", (".",)))
        self.enterContext(patch.object(ignored_shards, "workspace_members", return_value={
            "game": ("crates/game", "."), "launcher": ("launcher", "launcher"),
        }))

    def write(self, relative, text):
        path = self.root / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text, encoding="utf-8")

    def problems(self, package="game", target="test:exporters"):
        row = ignored_tests.Row(package, "check_exporters", target, "daily", "daily.txt", "windows")
        return check.windows_target_problems([row], self.root)

    def test_a_windows_only_test_root_is_accepted(self):
        self.write("crates/game/tests/exporters.rs", '//! Check the exporters.\n#![cfg(windows)]\n'
                   '#[test]\n#[ignore = "daily"]\nfn check_exporters() {}\n')
        self.assertEqual(self.problems(), [])

    def test_a_linux_capable_test_cannot_leave_linux_coverage(self):
        self.write("crates/game/tests/exporters.rs", '// #![cfg(windows)] for a different target\n'
                   '#[test]\n#[ignore = "daily"]\nfn check_exporters() { #[cfg(unix)] unix_check(); }\n')
        self.assertIn("tests/exporters.rs to start with #![cfg(windows)]", self.problems()[0])

    def test_an_explicit_cargo_target_cannot_borrow_an_unrelated_files_gate(self):
        self.write("crates/game/Cargo.toml", '[package]\nname = "game"\n'
                   '[[test]]\nname = "exporters"\npath = "tests/alternate.rs"\n')
        self.write("crates/game/tests/exporters.rs", '#![cfg(windows)]\n')
        self.write("crates/game/tests/alternate.rs", '#[test]\nfn check_exporters() {}\n')
        self.assertIn("requires the Cargo target root to be crates/game/tests/exporters.rs", self.problems()[0])

    def test_only_integration_test_targets_are_supported(self):
        for target in ("lib", "bin:game"):
            with self.subTest(target=target):
                self.assertIn("requires a Cargo integration test target (test:NAME)", self.problems(target=target)[0])

    def test_the_launcher_workspace_is_not_supported(self):
        self.assertIn("only supported in the game workspace, not launcher", self.problems(package="launcher")[0])


LEGACY_VARIABLES = ("SUNRISE_TEST_RESOURCES", "SUNRISE_RESOURCES", "SUNRISE_MESH_TEST_RESOURCES",
                    "SUNRISE_FISHING_RESOURCES")


def test(name, reason=None, target="test:lane"):
    return ignored_inventory.Test("game", target, "crates/game/tests/lane.rs", 1, name, reason, reason is not None)


def record(name, seed="boundary", kind="reaches", is_test=True, target="test:lane", **more):
    return {"kind": kind, "seed": seed, "package": "game", "target": target, "path": "crates/game/tests/lane.rs",
            "line": 1, "name": name, "test": is_test, "chain": "helper -> prepared_root", **more}


class PreparedRouting(unittest.TestCase):
    def test_a_daily_test_that_reaches_the_boundary_is_accepted(self):
        self.assertEqual(check.prepared_problems([test("routed", "daily")], [record("routed")]), [])

    def test_a_test_that_reaches_the_boundary_without_a_daily_mark_is_refused(self):
        for reason in (None, "needs a server"):
            with self.subTest(reason=reason):
                problems = check.prepared_problems([test("routed", reason)], [record("routed")])
                self.assertEqual(len(problems), 1)
                self.assertIn("helper -> prepared_root", problems[0])
                self.assertIn("not a daily test", problems[0])

    def test_the_identity_includes_the_target(self):
        problems = check.prepared_problems([test("routed", "daily", target="test:other")], [record("routed")])
        self.assertEqual(len(problems), 1)

    def test_a_helper_that_reaches_the_boundary_is_no_test(self):
        self.assertEqual(check.prepared_problems([], [record("with_root", is_test=False)]), [])

    def test_a_synthetic_test_has_no_record_and_needs_no_mark(self):
        self.assertEqual(check.prepared_problems([test("synthetic")], []), [])

    def test_a_path_the_tool_cannot_resolve_is_always_refused(self):
        unresolved = record("helper", kind="unresolved", is_test=False, reference="with_root")
        self.assertIn("cannot follow it", check.prepared_problems([], [unresolved])[0])

    def test_legacy_records_do_not_route(self):
        self.assertEqual(check.prepared_problems([test("old")], [record("old", seed="legacy")]), [])


class LegacyBaseline(unittest.TestCase):
    def test_debt_the_base_had_stays_and_new_debt_in_the_same_file_is_refused(self):
        base = [record("lane::old", seed="legacy")]
        head = [record("lane::old", seed="legacy", chain="another chain"), record("lane::new", seed="legacy")]
        problems = check.legacy_problems(head, base, {})
        self.assertEqual(len(problems), 1)
        self.assertIn("lane::new", problems[0])

    def test_a_renamed_file_keeps_its_debt_and_a_moved_reader_does_not(self):
        base = [record("lane::old", seed="legacy")]
        moved = dict(record("lane::old", seed="legacy"), path="crates/game/tests/renamed.rs")
        self.assertEqual(check.legacy_problems([moved], base, {"crates/game/tests/lane.rs": moved["path"]}), [])
        self.assertEqual(len(check.legacy_problems([moved], base, {})), 1)

    def test_deleted_debt_and_boundary_records_are_not_counted(self):
        self.assertEqual(check.legacy_problems([record("lane::routed")], [record("lane::gone", seed="legacy")], {}), [])

    def test_a_new_unresolved_reference_is_refused(self):
        head = [record("lane::new", seed="legacy", kind="unresolved", reference="with_game")]
        self.assertIn("calls `with_game`", check.legacy_problems(head, [], {})[0])

    def test_every_legacy_resource_variable_is_new_debt_only_where_the_base_lacked_it(self):
        readers = {variable: f"lane::reads_{variable.lower()}" for variable in LEGACY_VARIABLES}
        base = [record(name, seed="legacy") for name in readers.values()]
        self.assertEqual(check.legacy_problems(base, base, {}), [])
        fresh = {variable: f"{name}_again" for variable, name in readers.items()}
        problems = check.legacy_problems(base + [record(name, seed="legacy") for name in fresh.values()], base, {})
        self.assertEqual(len(problems), len(LEGACY_VARIABLES))
        for name in fresh.values():
            self.assertEqual(sum(name in problem for problem in problems), 1)

    def test_the_identity_tells_targets_of_one_path_apart(self):
        lane = record("lane::old", seed="legacy")
        other = record("lane::old", seed="legacy", target="test:other")
        self.assertNotEqual(check.legacy_identity(lane), check.legacy_identity(other))
        self.assertNotEqual(check.legacy_identity(lane), check.legacy_identity(dict(lane, package="another")))

    def test_a_source_file_exposed_under_a_new_target_does_not_inherit_the_old_exemption(self):
        base = [record("lane::old", seed="legacy")]
        head = [record("lane::old", seed="legacy"), record("lane::old", seed="legacy", target="test:other")]
        problems = check.legacy_problems(head, base, {})
        self.assertEqual(len(problems), 1)
        self.assertIn("lane::old", problems[0])


class PullRequestMerge(unittest.TestCase):
    def test_the_flag_is_read(self):
        self.assertTrue(check.parser().parse_args(["--base", "abc", "--pull-request-merge"]).pull_request_merge)
        self.assertFalse(check.parser().parse_args(["--base", "abc"]).pull_request_merge)

    def test_the_comparison_base_is_the_one_merge_base_picks(self):
        seen = []
        with (patch.object(merge_base, "pull_request_base", side_effect=lambda *a: (seen.append(a), ("parent", None))[1]),
              patch.object(check, "changes", return_value=({}, {"a.rs"})) as changes,
              patch.object(check, "base_records", return_value=()) as records):
            self.assertEqual(check.baseline_problems([], {}, "event-base", True), [])
        self.assertEqual(seen, [("event-base", True)])
        changes.assert_called_once_with("parent")
        self.assertEqual(records.call_args.args[0], "parent")

    def test_a_merge_that_does_not_descend_from_the_base_is_reported_and_nothing_is_compared(self):
        with (patch.object(merge_base, "pull_request_base", return_value=("base", "needs a merge")),
              patch.object(check, "changes") as changes):
            self.assertEqual(check.baseline_problems([], {}, "base", True), ["needs a merge"])
        changes.assert_not_called()


class BaseScanScope(unittest.TestCase):
    LEGACY = [record("lane::old", seed="legacy")]
    TARGET = ("game", "test:lane")
    FILES = {TARGET: ("crates/game/tests/lane.rs",)}

    def scan(self, changed, files=None):
        with (patch.object(check.subprocess, "Popen", side_effect=AssertionError("git archive ran")),
              patch.object(check.subprocess, "run", side_effect=AssertionError("git ran")),
              patch.object(ignored_inventory, "references", side_effect=AssertionError("a base scan ran"))):
            return check.base_records("base", self.LEGACY + [record("lane::routed")], files or self.FILES,
                                      set(changed))

    def test_no_legacy_target_means_nothing_to_compare(self):
        self.assertEqual(check.base_records("base", [record("lane::routed")], {}, {"crates/game/tests/lane.rs"}), ())

    def test_a_change_with_no_rust_or_manifest_file_archives_and_scans_nothing(self):
        for changed in (["docs/a.md"], ["tools/ci/check_ignored_test_rows.py", ".github/workflows/dyna-ci.yml"], []):
            with self.subTest(changed=changed):
                self.assertEqual(self.scan(changed), tuple(self.LEGACY))

    def test_a_rust_file_of_another_target_archives_and_scans_nothing(self):
        self.assertEqual(self.scan(["crates/other/src/lib.rs"]), tuple(self.LEGACY))

    def test_the_targets_a_change_can_reach(self):
        legacy = {self.TARGET, ("game", "lib")}
        files = {self.TARGET: ("crates/game/tests/lane.rs",), ("game", "lib"): ("crates/game/src/lib.rs",)}
        affected = check.affected_targets
        self.assertEqual(affected(legacy, files, {"crates/game/src/lib.rs"}), {("game", "lib")})
        self.assertEqual(affected(legacy, files, {"crates/other/Cargo.toml"}), legacy)
        self.assertEqual(affected(legacy, files, {"Cargo.toml"}), legacy)
        self.assertEqual(affected(legacy, files, {"crates/sunrise-mod-vocab/src/exports/a.rs"}), legacy)
        self.assertEqual(affected(legacy, files, {"README.md"}), set())
        self.assertEqual(affected(legacy, {}, {"crates/x/y.rs"}), legacy)


WRITE_ARCHIVE = (
    "import io, sys, tarfile\n"
    "data = b'#[test] fn t() {}\\n'\n"
    "with tarfile.open(fileobj=sys.stdout.buffer, mode='w|') as tar:\n"
    "    info = tarfile.TarInfo('crates/game/src/lib.rs')\n"
    "    info.size = len(data)\n"
    "    tar.addfile(info, io.BytesIO(data))\n"
    "sys.stdout.buffer.write(bytes(128 * 1024))\n"
)


class BaseArchiveStream(unittest.TestCase):
    """base_records reads `git archive`'s pipe to its end, whatever the child writes after the tar's own blocks."""

    def setUp(self):
        self.root = Path(self.enterContext(tempfile.TemporaryDirectory())).resolve()
        self.enterContext(patch.object(check, "ROOT", self.root))
        git(self.root, "init", "-q")
        for relative, text in {"Cargo.toml": "[workspace]\n", "crates/game/Cargo.toml": "[package]\n",
                               "crates/game/src/lib.rs": "\n"}.items():
            path = self.root / relative
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(text, encoding="utf-8")
        git(self.root, "add", "-A")
        git(self.root, "commit", "-q", "-m", "base")

    def scan(self, child_script, roots=(), references=None):
        """base_records with `git archive` replaced by a real child running `child_script`; a stuck child is killed
        after 30 s, which fails the scan instead of hanging the suite. Returns what the scan extracted. `roots` are
        the target roots the extracted tree has, and `references` stands in for the discovery tool."""
        real_popen = subprocess.Popen
        extracted = []

        def popen(args, **kwargs):
            if args[:2] != ["git", "archive"]:
                return real_popen(args, **kwargs)
            child = real_popen([sys.executable, "-c", child_script], **kwargs)
            watchdog = threading.Timer(30, child.kill)
            watchdog.start()
            self.addCleanup(watchdog.cancel)
            return child

        def target_roots(directory, tested):
            path = directory / "crates/game/src/lib.rs"
            extracted.append(path.read_text(encoding="utf-8") if path.is_file() else None)
            return list(roots)

        with (patch.object(check.subprocess, "Popen", popen),
              patch.object(ignored_inventory, "target_roots", target_roots),
              patch.object(ignored_inventory, "references", references or ignored_inventory.references)):
            check.base_records("HEAD", [record("lane::old", seed="legacy")],
                               {("game", "test:lane"): ("crates/game/src/lib.rs",)}, {"crates/game/src/lib.rs"})
        return extracted

    def test_zero_padding_past_the_pipe_buffer_is_drained_and_the_archive_extracted(self):
        self.assertEqual(self.scan(WRITE_ARCHIVE), ["#[test] fn t() {}\n"])

    def test_the_base_is_scanned_for_every_legacy_resource_variable(self):
        seen = []

        def references(root, boundary, literals, fragments, targets):
            seen.append((boundary, literals, fragments, targets))
            return [record("lane::old", seed="legacy")]

        self.scan(WRITE_ARCHIVE, roots=[("game", "test:lane", "crates/game/src/lib.rs", None)], references=references)
        self.assertEqual(seen, [(check.BOUNDARY, check.LEGACY_LITERALS, check.LEGACY_FRAGMENTS,
                                 {("game", "test:lane")})])
        self.assertEqual(set(seen[0][1]), set(LEGACY_VARIABLES))

    def test_a_failed_git_archive_is_reported_not_hung_on_or_left_to_a_tar_error(self):
        for name, script in (("empty", "import sys; sys.exit(128)"),
                             ("broken", "import sys; sys.stdout.write('not a tar'); sys.stdout.flush(); sys.exit(128)")):
            with self.subTest(name):
                with self.assertRaises(SystemExit) as raised:
                    self.scan(script)
                self.assertEqual(raised.exception.code, "git archive HEAD failed")


def git(root, *args):
    subprocess.run(["git", "-c", "commit.gpgsign=false", "-c", "tag.gpgsign=false", *args],
                   cwd=root, check=True, capture_output=True,
                   env={**os.environ, "GIT_CONFIG_GLOBAL": os.devnull, "GIT_CONFIG_SYSTEM": os.devnull,
                        "GIT_CONFIG_NOSYSTEM": "1", "GIT_AUTHOR_NAME": "t", "GIT_AUTHOR_EMAIL": "t@example.com",
                        "GIT_COMMITTER_NAME": "t", "GIT_COMMITTER_EMAIL": "t@example.com"})


@unittest.skipUnless(os.environ.get("SUNRISE_GAME_DISCOVERY") or shutil.which("cargo"),
                     "needs the discovery tool or cargo")
class BaseScanInARepository(unittest.TestCase):
    """base_records over a real git history: the base is archived (partially) from a commit and scanned."""

    def setUp(self):
        self.root = Path(self.enterContext(tempfile.TemporaryDirectory())).resolve()
        self.enterContext(patch.object(ignored_shards, "WORKSPACES", (".",)))
        self.enterContext(patch.object(check, "ROOT", self.root))
        git(self.root, "init", "-q")
        for relative, text in FIXTURE_CRATE.items():
            self.write(relative, text)
        self.commit("base")
        self.base = subprocess.run(["git", "rev-parse", "HEAD"], cwd=self.root, text=True, capture_output=True,
                                   check=True).stdout.strip()

    def write(self, relative, text):
        path = self.root / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text, encoding="utf-8")

    def commit(self, message):
        git(self.root, "add", "-A")
        git(self.root, "commit", "-q", "-m", message)

    def problems(self):
        references, files = ignored_inventory.references_with_files(
            self.root, check.BOUNDARY, check.LEGACY_LITERALS, check.LEGACY_FRAGMENTS)
        renames, changed = check.changes(self.base)
        base = check.base_records(self.base, references, files, changed)
        return check.legacy_problems(references, base, renames)

    def test_an_unchanged_tree_has_no_new_debt(self):
        self.assertEqual(self.problems(), [])

    def test_a_newly_added_legacy_reader_is_found_when_a_rust_file_changes(self):
        with (self.root / "crates/sunrise-client/src/menu.rs").open("a", encoding="utf-8") as menu:
            menu.write("#[test] fn a_second_menu_test() { with_game(|| ()) }\n")
        self.commit("head")
        problems = self.problems()
        self.assertEqual(len(problems), 1)
        self.assertIn("menu::a_second_menu_test", problems[0])

    def test_new_reads_of_every_legacy_resource_variable_are_refused_and_the_base_ones_tolerated(self):
        menu = self.root / "crates/sunrise-client/src/menu.rs"
        with menu.open("a", encoding="utf-8") as source:
            for variable in LEGACY_VARIABLES:
                source.write(f'#[test] fn old_{variable.lower()}() {{ let _ = std::env::var_os("{variable}"); }}\n')
        self.commit("base with every reader")
        self.base = subprocess.run(["git", "rev-parse", "HEAD"], cwd=self.root, text=True, capture_output=True,
                                   check=True).stdout.strip()
        with menu.open("a", encoding="utf-8") as source:
            for variable in LEGACY_VARIABLES:
                source.write(f'#[test] fn new_{variable.lower()}() {{ let _ = std::env::var_os("{variable}"); }}\n')
        self.commit("head")
        problems = self.problems()
        self.assertEqual(len(problems), len(LEGACY_VARIABLES))
        for variable in LEGACY_VARIABLES:
            self.assertEqual(sum(f"menu::new_{variable.lower()}" in problem for problem in problems), 1)
        self.assertFalse(any("menu::old_" in problem for problem in problems))

    def test_a_renamed_file_keeps_its_debt(self):
        git(self.root, "mv", "crates/sunrise-client/src/menu.rs", "crates/sunrise-client/src/menu_moved.rs")
        self.write("crates/sunrise-client/src/lib.rs",
                   '#[path = "menu_moved.rs"]\nmod menu;\nmod test_resources;\nmod refs;\nmod synthetic;\n')
        self.commit("head")
        renames, _ = check.changes(self.base)
        self.assertEqual(renames, {"crates/sunrise-client/src/menu.rs": "crates/sunrise-client/src/menu_moved.rs"})
        self.assertEqual(self.problems(), [])

    def test_a_file_and_a_crate_the_base_lacks_are_new_debt_not_a_failure(self):
        self.write("crates/sunrise-client/src/lib.rs",
                   "mod test_resources;\nmod refs;\nmod menu;\nmod synthetic;\nmod added;\n")
        self.write("crates/sunrise-client/src/added.rs",
                   '#[test] fn added_test() { let _ = std::env::var_os("SUNRISE_TEST_RESOURCES"); }\n')
        self.write("Cargo.toml", '[workspace]\nmembers = ["crates/sunrise-client", "crates/fresh"]\n')
        self.write("crates/fresh/Cargo.toml", '[package]\nname = "fresh"\n')
        self.write("crates/fresh/src/lib.rs",
                   '#[test] fn fresh_test() { let _ = std::env::var_os("SUNRISE_TEST_RESOURCES"); }\n')
        self.commit("head")
        problems = self.problems()
        self.assertEqual(len(problems), 2)
        self.assertTrue(any("added::added_test" in problem for problem in problems))
        self.assertTrue(any("fresh_test" in problem for problem in problems))


FIXTURE_CRATE = {
    "Cargo.toml": '[workspace]\nmembers = ["crates/sunrise-client"]\n',
    "crates/sunrise-client/Cargo.toml": '[package]\nname = "sunrise-client"\n',
    "crates/sunrise-client/src/lib.rs": "mod test_resources;\nmod refs;\nmod menu;\nmod synthetic;\n",
    "crates/sunrise-client/src/test_resources.rs":
        '#[path = "../../sunrise-core/src/prepared_test_root.rs"]\n'
        'mod prepared_test_root;\n'
        'pub use prepared_test_root::prepared_root;\n',
    "crates/sunrise-core/src/prepared_test_root.rs":
        'pub fn prepared_root(needs: &[&str]) -> std::path::PathBuf {\n'
        '    std::env::var_os("SUNRISE_TEST_RESOURCES").map(Into::into).unwrap_or_default()\n}\n',
    "crates/sunrise-client/src/refs.rs":
        'pub mod tests {\n'
        '    pub fn with_game(run: impl FnOnce()) {\n'
        '        let _ = std::env::var_os("SUNRISE_TEST_RESOURCES");\n'
        '        run()\n'
        '    }\n}\n',
    "crates/sunrise-client/src/menu.rs":
        'use crate::refs::tests::with_game as with_base_game;\n'
        'fn with_game(run: impl FnOnce()) { with_base_game(|| run()) }\n'
        '#[test] fn a_menu_test() { with_game(|| ()) }\n',
    "crates/sunrise-client/src/synthetic.rs":
        'use crate::test_resources::prepared_root;\n'
        'fn with_game(run: impl FnOnce()) { run() }\n'
        '#[test] fn builds_its_own_data() { with_game(|| ()) }\n'
        '#[test] #[ignore = "daily"] fn reads_the_boundary() { prepared_root(&[]); }\n'
        '#[test] fn forgot_the_mark() { prepared_root(&[]); }\n',
}


@unittest.skipUnless(os.environ.get("SUNRISE_GAME_DISCOVERY") or shutil.which("cargo"),
                     "needs the discovery tool or cargo")
class ResolvedEndToEnd(unittest.TestCase):
    """The shape of tests/native_menu_places: a legacy wrapper imported under another name and wrapped by a local
    function of the first one's name, beside a synthetic wrapper of that name."""

    def setUp(self):
        self.root = Path(self.enterContext(tempfile.TemporaryDirectory())).resolve()
        for relative, text in FIXTURE_CRATE.items():
            path = self.root / relative
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(text, encoding="utf-8")
        self.enterContext(patch.object(ignored_shards, "WORKSPACES", (".",)))

    def records(self, root=None):
        return ignored_inventory.references(root or self.root, check.BOUNDARY, check.LEGACY_LITERALS,
                                            check.LEGACY_FRAGMENTS)

    def test_the_wrapper_reaches_through_the_alias_and_the_decoy_does_not(self):
        legacy = {r["name"] for r in self.records() if r["seed"] == "legacy" and r["kind"] == "reaches"}
        self.assertEqual(legacy, {"menu::a_menu_test", "menu::with_game", "refs::tests::with_game"})

    def test_only_the_unmarked_boundary_test_is_refused(self):
        problems = check.prepared_problems(ignored_inventory.tests(self.root), self.records())
        self.assertEqual(len(problems), 1)
        self.assertIn("synthetic::forgot_the_mark", problems[0])

    def test_a_new_test_through_the_old_wrapper_is_refused_against_the_base(self):
        head = Path(self.enterContext(tempfile.TemporaryDirectory())).resolve() / "head"
        shutil.copytree(self.root, head)
        with (head / "crates/sunrise-client/src/menu.rs").open("a", encoding="utf-8") as menu:
            menu.write("#[test] fn a_second_menu_test() { with_game(|| ()) }\n")
        problems = check.legacy_problems(self.records(head), self.records(), {})
        self.assertEqual(len(problems), 1)
        self.assertIn("menu::a_second_menu_test", problems[0])

    def test_a_second_crate_including_the_shared_boundary_is_held_to_it_too(self):
        self.write("Cargo.toml", '[workspace]\nmembers = ["crates/sunrise-client", "crates/sunrise-world"]\n')
        self.write("crates/sunrise-world/Cargo.toml", '[package]\nname = "sunrise-world"\n')
        self.write("crates/sunrise-world/src/lib.rs",
                   '#[path = "../../sunrise-core/src/prepared_test_root.rs"]\n'
                   'mod prepared_test_root;\n'
                   '#[test] #[ignore = "daily"] fn world_marked() { prepared_test_root::prepared_root(&[]); }\n'
                   '#[test] fn world_unmarked() { prepared_test_root::prepared_root(&[]); }\n')
        problems = [problem for problem in check.prepared_problems(ignored_inventory.tests(self.root), self.records())
                    if "sunrise-world" in problem]
        self.assertEqual(len(problems), 1)
        self.assertIn("world_unmarked", problems[0])
        self.assertNotIn("world_marked", problems[0].replace("world_unmarked", ""))

    def write(self, relative, text):
        path = self.root / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text, encoding="utf-8")
