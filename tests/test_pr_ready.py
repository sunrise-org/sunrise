import argparse
import contextlib
import datetime
import io
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import threading
import time
import unittest
from unittest import mock

from tools import pr_ready
from tests.git_fixture import isolated_git_env
from tests.os_support import POSIX_BASH
from tests.test_launcher_mirrors_check import WRITE, MirrorCheckout

PACKAGES = {"sunrise-core": "crates/sunrise-core", "sunrise-client": "crates/sunrise-client",
            "sunrise-theme": "crates/sunrise-theme", "xtask": "xtask"}
LIBRARIES = set(PACKAGES) - {"xtask"}
DEPENDENCIES = {"sunrise-client": {"sunrise-core"}}
# The crates sunrise-stage depends on, as cargo metadata gives them (sunrise-mod-package reaches the vocabulary and
# the core, the vocabulary its API crates); sunrise-theme and sunrise-client are outside that closure.
SURFACE_PACKAGES = {**PACKAGES, **{name: f"crates/{name}" for name in (
    "sunrise-stage", "sunrise-mod-api-gen", "sunrise-game-discovery", "sunrise-mod-package", "sunrise-mod-source",
    "sunrise-mod-surface", "sunrise-mod-vocab", "sunrise-mod-api", "sunrise-mod-api-macros",
    "sunrise-gameplay-common", "sunrise-gameplay", "sunrise-client-gameplay")}}
SURFACE_DEPENDENCIES = {
    **DEPENDENCIES,
    "sunrise-stage": {"sunrise-mod-api-gen", "sunrise-mod-package", "sunrise-mod-source", "sunrise-mod-surface"},
    "sunrise-mod-api-gen": {"sunrise-game-discovery", "sunrise-mod-source", "sunrise-mod-surface"},
    "sunrise-game-discovery": {"sunrise-mod-source"},
    "sunrise-mod-package": {"sunrise-mod-vocab", "sunrise-core"},
    "sunrise-mod-vocab": {"sunrise-mod-api", "sunrise-mod-api-macros"},
}
MODS = ""
CLIPPY = f'status=0\n{pr_ready.CI_CLIPPY} || status=$?\n{pr_ready.CI_CLIPPY_SERVER} || status=$?\nexit "$status"'


def args(host="local", quick=False, harnesses=False, full=False):
    return argparse.Namespace(host=host, quick=quick, harnesses=harnesses, full=full)


class GitChangedPathsTests(unittest.TestCase):
    def setUp(self):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.root = Path(directory.name)
        self.enterContext(mock.patch.dict(os.environ, isolated_git_env(), clear=True))
        self.git("init")
        for key, value in (("core.quotePath", "true"), ("core.autocrlf", "false"),
                           ("core.hooksPath", str(self.root / "no-hooks"))):
            self.git("config", key, value)
        for module in (pr_ready, pr_ready.parity):
            patcher = mock.patch.object(module, "ROOT", self.root)
            patcher.start()
            self.addCleanup(patcher.stop)

    def git(self, *arguments):
        return subprocess.run(["git", *arguments], cwd=self.root, capture_output=True,
                              text=True, encoding="utf-8", check=True).stdout

    def write(self, path, text):
        destination = self.root / path
        destination.parent.mkdir(parents=True, exist_ok=True)
        destination.write_text(text, encoding="utf-8", newline="\n")

    def commit(self):
        self.git("add", ".")
        self.git("-c", "user.name=Test", "-c", "user.email=test@example.invalid", "commit", "-m", "base")

    @unittest.skipUnless(os.name == "posix", "requires POSIX symlinks and FIFO")
    def test_revision_snapshot_never_reads_symlink_targets_or_nonregular_files(self):
        self.write("tracked.txt", "base\n")
        self.commit()
        (self.root / "dangling").symlink_to("absent")
        (self.root / "directory-link").symlink_to(self.root, target_is_directory=True)
        os.mkfifo(self.root / "pipe")
        actual_git = pr_ready.git
        def git(*arguments):
            if arguments[0] == "ls-files":
                # Git may omit nonregular files; exercise the refusal even if listed.
                return "dangling\0directory-link\0pipe\0"
            return actual_git(*arguments)
        with mock.patch.object(pr_ready, "git", side_effect=git), \
                mock.patch.object(Path, "open", side_effect=AssertionError("must not read untracked objects")):
            before = pr_ready.revision_state()
            self.assertTrue(before[1])
            self.assertIn("absent", before[2])
            self.assertIn("pipe", before[2])
            (self.root / "dangling").unlink()
            (self.root / "dangling").symlink_to("another-absent")
            after = pr_ready.revision_state()
        self.assertNotEqual(before, after)

    def test_self_check_header_identifies_commit_and_dirty_state(self):
        self.write("tracked.txt", "base\n")
        self.write(".gitignore", "ignored-output/\n")
        self.commit()
        head = self.git("rev-parse", "HEAD").strip()
        def header():
            return pr_ready.block(args(), "HEAD", head, [], {}, [], "skip-release", 0,
                                  pr_ready.revision_state()).splitlines()[0]
        self.assertIn(f"commit `{head}`", header())
        self.assertIn("host `", header())
        self.assertNotIn(" dirty", header())
        self.write("ignored-output/log.txt", "ignored\n")
        self.assertNotIn(" dirty", header())
        self.write("untracked.txt", "new\n")
        self.assertIn(f"commit `{head}` dirty", header())
        (self.root / "untracked.txt").unlink()
        self.write("tracked.txt", "edited\n")
        self.assertIn(" dirty", header())
        self.git("add", "tracked.txt")
        self.assertIn(" dirty", header())

    def test_unicode_paths_select_checks_and_keep_line_counts(self):
        tracked = "crates/sunrise-core/src/caf\u00e9 edit.rs"
        untracked = "crates/sunrise-core/src/\u65b0 file.rs"
        self.write(tracked, "first\n")
        self.commit()
        self.write(tracked, "first\nsecond\n")
        self.write(untracked, "new\nfile\n")
        changed = pr_ready.parity.changed_files("HEAD")
        self.assertEqual(changed, {tracked, untracked})
        self.assertEqual(pr_ready.line_counts("HEAD", changed), {tracked: (1, 0), untracked: (2, 0)})
        self.assertTrue(all(pr_ready.workspace_rust(path) for path in changed))
        self.assertEqual(pr_ready.touched_crates(changed, PACKAGES), ["sunrise-core"])

    def test_renamed_rust_source_keeps_both_paths_and_counts(self):
        before, after = "crates/sunrise-core/src/moved.rs", "docs/moved.md"
        self.write(before, "fn moved() {\n}\n")
        self.commit()
        (self.root / "docs").mkdir()
        self.git("mv", before, after)
        changed = pr_ready.parity.changed_files("HEAD")
        self.assertEqual(changed, {before, after})
        self.assertEqual(pr_ready.line_counts("HEAD", changed), {before: (0, 2), after: (2, 0)})
        self.assertEqual(pr_ready.touched_crates(changed, PACKAGES), ["sunrise-core"])

    @unittest.skipIf(os.name == "nt", "Windows filenames cannot contain tabs or newlines")
    def test_tabs_and_newlines_remain_in_paths(self):
        tracked = "crates/sunrise-core/src/tab\tfile.rs"
        untracked = "crates/sunrise-core/src/line\r\nfile.rs"
        self.write(tracked, "first\n")
        self.commit()
        self.write(tracked, "first\nsecond\n")
        self.write(untracked, "new\n")
        changed = pr_ready.parity.changed_files("HEAD")
        self.assertEqual(changed, {tracked, untracked})
        self.assertEqual(pr_ready.line_counts("HEAD", changed), {tracked: (1, 0), untracked: (1, 0)})


class PrReadyTests(unittest.TestCase):
    @unittest.skipUnless(POSIX_BASH and os.name != "nt", "executes the POSIX self-check")
    def test_cold_clippy_hint_precedes_the_step_and_respects_sccache(self):
        for wrapper in (None, "", "other-wrapper", "sccache", "/opt/bin/sccache", "sccache.exe"):
            with self.subTest(wrapper=wrapper):
                env = dict(os.environ)
                env.pop("RUSTC_WRAPPER", None)
                if wrapper is not None:
                    env["RUSTC_WRAPPER"] = wrapper
                steps = [pr_ready.Step(name, "printf 'ran\\n'")
                         for name in ("clippy", "dispatch tests clippy")]
                with mock.patch.dict(os.environ, env, clear=True), contextlib.redirect_stderr(io.StringIO()) as log:
                    pr_ready.run_script(steps, "local", verbose=False)
                self.assertEqual([(step.status, step.output.strip()) for step in steps], [("pass", "ran")] * 2)
                expected = wrapper in (None, "", "other-wrapper")
                self.assertEqual(log.getvalue().count(pr_ready.CLIPPY_CACHE_HINT), int(expected))
                if expected:
                    self.assertLess(log.getvalue().index(pr_ready.CLIPPY_CACHE_HINT), log.getvalue().index("  clippy..."))
        step = pr_ready.Step("tests", "printf 'ran\\n'")
        with mock.patch.dict(os.environ, {"RUSTC_WRAPPER": ""}), contextlib.redirect_stderr(io.StringIO()) as log:
            pr_ready.run_script([step], "local", verbose=False)
        self.assertNotIn(pr_ready.CLIPPY_CACHE_HINT, log.getvalue())

    def test_data_backed_summary_with_and_without_prepared_data(self):
        with tempfile.TemporaryDirectory() as directory, \
                mock.patch.object(pr_ready, "ROOT", Path(directory) / "checkout/repo"), \
                mock.patch.object(pr_ready, "git", side_effect=lambda *args: "a" * 40 if args[0] == "rev-parse" else ""), \
                mock.patch.dict(os.environ):
            os.environ.pop("SUNRISE_TEST_RESOURCES", None)
            step = pr_ready.Step("content root tests", summary=pr_ready.tests_passed)
            step.output = "test result: ok. 3 passed; 0 failed; 0 ignored"
            step.status = "pass"
            step.detail = pr_ready.tests_passed(step.output)
            def render():
                return pr_ready.block(args(), "origin/main", "abcdef123", [], {}, [step], "skip-release", 0,
                                      ("a" * 40, "", ""))
            self.assertIn("data-backed tests not run: no prepared data here", render())
            prepared = pr_ready.ROOT / ".local/sunrise-client/26.3"
            prepared.mkdir(parents=True)
            self.assertIn("data-backed tests not run: no prepared data here", render())
            (prepared / "gameplay-facts.json").write_text("{}")
            self.assertIn("data-backed tests: sunrise-client-lib mod_content::tests:: (--ignored): 3 passed", render())
            override = Path(directory) / "override"
            override.mkdir()
            os.environ["SUNRISE_TEST_RESOURCES"] = str(override)
            self.assertIn("data-backed tests not run: no prepared data here", render())
            (override / "gameplay-facts.json").write_text("{}")
            (prepared / "gameplay-facts.json").unlink()
            self.assertIn("data-backed tests: sunrise-client-lib mod_content::tests:: (--ignored): 3 passed", render())
            step.status, step.detail = "skip", "content root untouched"
            self.assertIn("data-backed tests not run: content root untouched", render())

    @contextlib.contextmanager
    def assert_no_reaper_errors(self):
        errors = []
        with mock.patch.object(threading, "excepthook", errors.append):
            yield
        self.assertEqual(errors, [], "reaper raised")

    def test_windows_runner_gives_a_leading_variable_to_the_environment(self):
        step = pr_ready.Step(
            "dispatch tests tests",
            "CARGO_BUILD_JOBS=${CARGO_BUILD_JOBS:-4} cargo test --manifest-path a/Cargo.toml --locked",
        )
        with mock.patch.object(pr_ready.subprocess, "Popen") as popen, \
                mock.patch.object(pr_ready, "finish"), mock.patch.dict(os.environ), \
                self.assert_no_reaper_errors():
            os.environ.pop("CARGO_BUILD_JOBS", None)
            popen.return_value.__enter__.return_value.stdout = []
            popen.return_value.__enter__.return_value.wait.return_value = 0
            popen.return_value.__enter__.return_value.returncode = 0
            # No Windows job object: on Windows the reaper hands `job` to ctypes, which would follow a mock's
            # `_as_parameter_` without end.
            popen.return_value.__enter__.return_value.job = None
            pr_ready.run_windows([step], False)
        self.assertEqual(popen.call_args.args[0],
                         ["cargo", "test", "--manifest-path", "a/Cargo.toml", "--locked"])
        self.assertEqual(popen.call_args.kwargs["env"]["CARGO_BUILD_JOBS"], "4")
        self.assertEqual(step.status, "pass")

    def test_raising_reaper_fails_the_thread_exception_check(self):
        with mock.patch.object(pr_ready.process_tree, "ended", side_effect=RuntimeError("reaper failed")):
            with self.assertRaisesRegex(AssertionError, "reaper raised"):
                self.test_windows_runner_gives_a_leading_variable_to_the_environment()

    def test_paths_map_to_the_crates_that_own_them(self):
        self.assertEqual(pr_ready.touched_crates({"crates/sunrise-core/src/jmath.rs", "crates/sunrise-client/build.rs",
                                                  "xtask/src/main.rs", "tools/parity.py"}, PACKAGES),
                         ["sunrise-client", "sunrise-core", "xtask"])
        self.assertEqual(pr_ready.touched_crates({"crates/sunrise-core/README.md"}, PACKAGES), [])
        # The generated stylesheet is checked by sunrise-theme's tests.
        self.assertEqual(pr_ready.touched_crates({"crates/sunrise-theme/tokens.css"}, PACKAGES), ["sunrise-theme"])
        # A crate whose name prefixes another's directory doesn't own it.
        self.assertEqual(pr_ready.touched_crates({"crates/sunrise-core-extra/x.rs"}, PACKAGES), [])
        # The workspace manifest and lock touch every crate.
        self.assertIsNone(pr_ready.touched_crates({"Cargo.lock", "crates/sunrise-core/src/lib.rs"}, PACKAGES))

    def test_shared_client_sources_select_the_library_without_gameplay(self):
        packages = {**PACKAGES, "sunrise-client-lib": "crates/sunrise-client-lib"}
        for path in ["src/client_lib.rs", "src/health.rs", "src/lan/hub.rs", "src/bin/sunrise-ctl/main.rs",
                     "examples/probe/main.rs", "tests/lan_bot.rs"]:
            self.assertEqual(pr_ready.touched_crates({f"crates/sunrise-client/{path}"}, packages),
                             ["sunrise-client-lib"])
        for path in ["src/lib.rs", "src/main.rs", "src/entry.rs", "src/game_facade.rs", "src/game.rs",
                     "src/game/items/combat.rs", "src/cut_layout/game/items/combat.rs",
                     "tests/it/mod_api_hooks.rs", "Cargo.toml", "build.rs"]:
            self.assertEqual(pr_ready.touched_crates({f"crates/sunrise-client/{path}"}, packages),
                             ["sunrise-client"])

    def test_which_workspace_builds_a_path(self):
        self.assertTrue(pr_ready.workspace_rust("crates/sunrise-theme/fonts/x.ttf"))
        self.assertTrue(pr_ready.workspace_rust("Cargo.toml"))
        self.assertFalse(pr_ready.workspace_rust("crates/sunrise-client/README.md"))

    def test_crate_checks_do_not_link_unrelated_examples(self):
        with mock.patch.object(pr_ready, "affected_here", return_value=[]), \
                mock.patch.object(pr_ready, "holds_loop", return_value=False):
            steps = pr_ready.plan(args(quick=True), "origin/main", {"crates/sunrise-client/src/game.rs"}, PACKAGES, LIBRARIES, DEPENDENCIES)
        by_name = {step.name: step for step in steps}
        self.assertEqual(by_name["clippy"].command, CLIPPY)
        self.assertEqual(by_name["tests: sunrise-client"].command,
                         "cargo test --locked -p sunrise-client --tests")
        self.assertEqual(by_name["doc tests"].command, "cargo test --locked -p sunrise-client --doc")
        self.assertIsNone(by_name["doc tests"].status)

    def test_port_map_checks_the_merge_base_not_the_moving_tip(self):
        steps = pr_ready.plan(args(quick=True), "origin/main", {"docs/port-ledger.md"},
                              PACKAGES, LIBRARIES, DEPENDENCIES, merge_base="abc123")
        by_name = {step.name: step for step in steps}
        self.assertIn("--base abc123", by_name["Port map"].command)
        self.assertIn("--base origin/main", by_name["Layers"].command)

    def launcher_mirror_step(self, changed, merge_base="abc123"):
        manifest = pr_ready.launcher_mirrors.load_manifest()
        mirror_crates = {Path(source).parts[1] for mirror in manifest["mirror"] for source in mirror["sources"]}
        packages = {**PACKAGES, **{crate: f"crates/{crate}" for crate in mirror_crates}}
        with mock.patch.object(pr_ready, "affected_here", return_value=[]):
            steps = pr_ready.plan(args(quick=True), "origin/main", set(changed), packages, LIBRARIES, DEPENDENCIES,
                                  merge_base=merge_base)
        names = [step.name for step in steps]
        self.assertLess(names.index("Addresses"), names.index("Launcher mirrors"))
        self.assertLess(names.index("Launcher mirrors"), names.index("Port map"))
        return steps[names.index("Launcher mirrors")]

    def test_launcher_mirrors_run_when_a_mirrored_source_or_the_manifest_changes(self):
        # The paths come from the manifest the check reads, never from a list here.
        manifest = pr_ready.launcher_mirrors.load_manifest()
        sources = [source for mirror in manifest["mirror"] for source in mirror.get("sources", [])]
        self.assertTrue(sources)
        for source in sources:
            changed = source if source.endswith(".rs") else f"{source.rstrip('/')}/lib.rs"
            with self.subTest(source=source):
                step = self.launcher_mirror_step({changed})
                self.assertIsNone(step.skip)
                self.assertEqual(step.where, "here")
                self.assertEqual(step.command, "python3 tools/ci/check_launcher_mirrors.py --base abc123 --remind-body")
        self.assertIsNone(self.launcher_mirror_step({"tools/launcher_mirrors.toml"}).skip)
        # A sibling that only shares a mirror source's name prefix is not part of it.
        self.assertEqual(self.launcher_mirror_step({f"{sources[0].rstrip('/')}-other/lib.rs"}).skip,
                         "launcher mirrors untouched")
        for path in ("README.md", "tools/pr_ready.py", "crates/sunrise-client/src/game.rs"):
            with self.subTest(path=path):
                self.assertEqual(self.launcher_mirror_step({path}).skip, "launcher mirrors untouched")

    def run_launcher_mirror_step(self, checkout):
        """The planned step's own command for a change to a mirrored source, run in `checkout` and finished as the
        self-check does."""
        source = pr_ready.launcher_mirrors.load_manifest()["mirror"][0]["sources"][0]
        step = self.launcher_mirror_step({f"{source.rstrip('/')}/lib.rs"}, merge_base=checkout.base)
        self.assertIsNone(step.skip)
        result = checkout.run(*step.command.split())
        step.output, step.status = result.stdout, "pass" if result.returncode == 0 else "fail"
        with contextlib.redirect_stderr(io.StringIO()):
            pr_ready.finish(step, verbose=True)
        return step

    def test_a_mirrored_source_change_with_a_stale_manifest_fails_with_the_write_command(self):
        checkout = MirrorCheckout(self)
        checkout.change_surface(record=False)
        step = self.run_launcher_mirror_step(checkout)
        self.assertEqual(step.status, "fail")
        self.assertIn(f"Run `{WRITE}`", step.detail)
        self.assertIn("mirror: public surface is", step.detail)

    def test_a_changed_surface_with_a_current_manifest_passes_and_reminds_about_the_body_line(self):
        checkout = MirrorCheckout(self)
        checkout.change_surface(record=True)
        step = self.run_launcher_mirror_step(checkout)
        self.assertEqual(step.status, "pass", step.output)
        self.assertIn("Reminder: launcher mirrors changed (mirror)", step.detail)
        self.assertIn("launcher-mirror:", step.detail)

    def test_a_mirrored_source_touched_without_a_surface_change_passes_without_a_reminder(self):
        checkout = MirrorCheckout(self)
        checkout.change_privately()
        step = self.run_launcher_mirror_step(checkout)
        self.assertEqual((step.status, step.detail), ("pass", ""))

    def test_content_root_tests_run_only_when_what_they_load_changes(self):
        packages = {**PACKAGES, "sunrise-mod-package": "crates/sunrise-mod-package",
                    "sunrise-assets": "crates/sunrise-assets", "sunrise-sim": "crates/sunrise-sim"}

        def content_step(path):
            with mock.patch.object(pr_ready, "affected_here", return_value=[]), \
                    mock.patch.object(pr_ready, "holds_loop", return_value=False):
                steps = pr_ready.plan(args(quick=True), "origin/main", {path}, packages, LIBRARIES, DEPENDENCIES)
            return {step.name: step for step in steps}["content root tests"]
        self.assertEqual(content_step("crates/sunrise-client/src/game.rs").skip, "content root untouched")
        for path in ["crates/sunrise-sim/src/mod_content.rs", "crates/sunrise-sim/src/mod_content_tests.rs",
                     "crates/sunrise-mod-package/src/content.rs", "crates/sunrise-assets/src/lib.rs", "Cargo.lock"]:
            with self.subTest(path=path):
                step = content_step(path)
                self.assertIsNone(step.skip)
                self.assertEqual(step.command, pr_ready.CONTENT_ROOT_TESTS)

    def test_dispatch_suite_runs_when_its_inputs_change(self):
        def suite_steps(path):
            with mock.patch.object(pr_ready, "affected_here", return_value=[]):
                steps = pr_ready.plan(args(quick=True), "origin/main", {path}, SURFACE_PACKAGES, LIBRARIES,
                                      SURFACE_DEPENDENCIES)
            return {step.name: step for step in steps}
        for path in ["crates/sunrise-mod-api-gen/src/lib.rs",
                     "crates/sunrise-mod-api-gen/dispatch-tests/src/lib.rs",
                     "crates/sunrise-mod-api-macros/src/lib.rs",
                     "crates/sunrise-mod-vocab/src/lib.rs",
                     "crates/sunrise-mod-vocab/src/parts/mod.rs",
                     "crates/sunrise-mod-vocab/src/hooks/block.rs"]:
            with self.subTest(path=path):
                by_name = suite_steps(path)
                for name in ("dispatch tests clippy", "dispatch tests tests"):
                    self.assertIsNone(by_name[name].skip)
                self.assertIn("crates/sunrise-mod-api-gen/dispatch-tests/Cargo.toml",
                              by_name["dispatch tests clippy"].command)
                self.assertIn("--test-threads=4", by_name["dispatch tests tests"].command)
        by_name = suite_steps("crates/sunrise-core/src/lib.rs")
        self.assertEqual(by_name["dispatch tests clippy"].skip, "dispatch surface untouched")
        self.assertEqual(by_name["dispatch tests tests"].skip, "dispatch surface untouched")

    def surface_step(self, changed, name="posix", packages=SURFACE_PACKAGES, dependencies=SURFACE_DEPENDENCIES):
        with mock.patch.object(pr_ready, "affected_here", return_value=[]), \
                mock.patch.object(pr_ready.os, "name", name):
            steps = pr_ready.plan(args(quick=True), "origin/main", set(changed), packages, LIBRARIES, dependencies,
                                  merge_base="abc123")
        names = [step.name for step in steps]
        self.assertGreater(names.index("Surface report"), names.index("doc tests"))
        return steps[names.index("Surface report")]

    def test_surface_report_runs_when_the_mod_api_sources_change(self):
        # No git here: none of these is a game file.
        for path in ["crates/sunrise-mod-vocab/changes/hook-note.toml", "crates/sunrise-mod-vocab/src/hooks/block.rs",
                     "crates/sunrise-mod-api/src/lib.rs", "crates/sunrise-mod-api-gen/src/lib.rs",
                     "crates/sunrise-mod-api-macros/src/lib.rs", "crates/sunrise-mod-surface/src/diff.rs",
                     "crates/sunrise-stage/src/main.rs", "crates/sunrise-game-discovery/src/lib.rs",
                     "crates/sunrise-mod-package/src/manifest.rs", "crates/sunrise-mod-source/src/lib.rs",
                     "crates/sunrise-core/src/lib.rs", "Cargo.toml", "tools/ci/surface_report.py",
                     "tools/ci/surface.toml", "tools/ci/surface-schema-3-reset.json"]:
            with self.subTest(path=path):
                step = self.surface_step({path})
                self.assertIsNone(step.skip)
                self.assertEqual(step.where, "host")
                self.assertEqual(step.command, "python3 tools/ci/surface_report.py --base abc123")
        for path in ["crates/sunrise-theme/src/lib.rs", "crates/sunrise-client/src/lib.rs", "tools/ci/check_layers.py",
                     "README.md"]:
            with self.subTest(path=path):
                self.assertEqual(self.surface_step({path}).skip, "mod API surface untouched")
        self.assertIn("CI's verify reports it",
                      self.surface_step({"crates/sunrise-mod-vocab/changes/hook-note.toml"}, name="nt").skip)
        self.assertEqual(self.surface_step({"README.md"}, name="nt").skip, "mod API surface untouched")

    def test_the_surface_crates_are_what_sunrise_stage_depends_on(self):
        self.assertEqual(pr_ready.surface_crates(SURFACE_DEPENDENCIES),
                         ["sunrise-core", "sunrise-game-discovery", "sunrise-mod-api", "sunrise-mod-api-gen",
                          "sunrise-mod-api-macros", "sunrise-mod-package", "sunrise-mod-source",
                          "sunrise-mod-surface", "sunrise-mod-vocab", "sunrise-stage"])
        # A branch with no Rust has no dependency map: the tool alone, and no folder to match.
        self.assertEqual(pr_ready.surface_crates({}), ["sunrise-stage"])

    def test_a_branch_without_rust_runs_the_surface_report_for_its_own_files_and_notes_only(self):
        # A change note is a workspace path, so plan() refuses one under crates/ when packages is empty: the
        # surface rule itself still counts it, with no packages or dependencies.
        self.assertTrue(pr_ready.surface_touched({"crates/sunrise-mod-vocab/changes/a.toml"}, {}, {}, "abc123"))
        for path in ("tools/ci/surface_report.py", "tools/ci/surface.toml"):
            with self.subTest(path=path):
                self.assertIsNone(self.surface_step({path}, packages={}, dependencies={}).skip)
        self.assertEqual(self.surface_step({"README.md"}, packages={}, dependencies={}).skip,
                         "mod API surface untouched")

    def test_game_files_run_the_surface_report_only_when_they_hold_a_loop(self):
        loop = 'impl ServerGame<\'_> {\n    #[sunrise_mod_api_macros::sunrise_loop(name = "a.tick", side = "server")]\n}\n'
        ticks = "crates/sunrise-gameplay/src/game/blocks/block_entity_ticks.rs"
        plain = "crates/sunrise-client-gameplay/src/game/items/combat.rs"
        gone = "crates/sunrise-gameplay/src/game/blocks/gone.rs"
        with tempfile.TemporaryDirectory() as directory, mock.patch.object(pr_ready, "ROOT", Path(directory)), \
                mock.patch.object(pr_ready.subprocess, "run") as run:
            for path, text in ((ticks, loop), (plain, "impl ClientGame<'_> { fn attack(&mut self) {} }\n")):
                (Path(directory) / path).parent.mkdir(parents=True, exist_ok=True)
                (Path(directory) / path).write_text(text)
            # The file holds a loop now: no need to ask git.
            self.assertIsNone(self.surface_step({ticks, plain}).skip)
            run.assert_not_called()
            # No loop now, none at the merge base.
            run.return_value = subprocess.CompletedProcess([], 1, b"", b"")
            self.assertEqual(self.surface_step({plain}).skip, "mod API surface untouched")
            self.assertEqual(run.call_args.args[0], ["git", "grep", "-l", "-F", "sunrise_loop", "abc123", "--",
                                                     f":(literal){plain}"])
            # A file with none now that held one at the merge base: deleted, or the attribute removed.
            run.return_value = subprocess.CompletedProcess([], 0, f"abc123:{gone}\n".encode(), b"")
            self.assertIsNone(self.surface_step({gone}).skip)
            self.assertIsNone(self.surface_step({plain}).skip)
            # A base git cannot read is not an answer: the report runs.
            run.return_value = subprocess.CompletedProcess([], 128, b"", b"fatal")
            self.assertIsNone(self.surface_step({plain}).skip)
            # Only Rust files of the game modules are read, and the root files are included.
            run.reset_mock()
            run.return_value = subprocess.CompletedProcess([], 1, b"", b"")
            self.assertEqual(self.surface_step({"crates/sunrise-gameplay/src/game/data.json"}).skip,
                             "mod API surface untouched")
            run.assert_not_called()
            self.assertEqual(self.surface_step({"crates/sunrise-gameplay-common/src/game.rs"}).skip,
                             "mod API surface untouched")
            self.assertEqual(run.call_count, 1)
            self.assertEqual(self.surface_step({"crates/sunrise-client/src/cut_layout/game/ticks.rs"}).skip,
                             "mod API surface untouched")
            self.assertEqual(run.call_count, 2)

    def test_surface_report_summary_names_the_verdict_and_what_failed(self):
        passing = ("CI does not compare listed internals; use surface_report.py --with-internals for a local audit.\n"
                   "Surface: additive (0 breaking, 0 conditional, 2 additive; 0 breaking without a note)\n"
                   "Old aaaa, new bbbb\n\nAdditive (2)\n- fn:a (added)\n"
                   "Base 1111; fetch 0.1s, total 2.0s\n")
        self.assertEqual(pr_ready.surface_summary(passing),
                         "Surface: additive (0 breaking, 0 conditional, 2 additive; 0 breaking without a note)")
        failing = ("Surface: breaking (1 breaking, 0 conditional, 0 additive; 1 breaking without a note)\n"
                   "Old aaaa, new bbbb\n\nBreaking (1)\n- type:HookId (changed)\n"
                   "change note old.toml names fn:gone, which did not change\n"
                   "change note typo.toml names fn:hokid, which did not change; did you mean type:HookId?\n"
                   "breaking change without a change note: type:HookId\n")
        self.assertEqual(pr_ready.surface_summary(failing),
                         "Surface: breaking (1 breaking, 0 conditional, 0 additive; 1 breaking without a note); "
                         "change note old.toml names fn:gone, which did not change; "
                         "change note typo.toml names fn:hokid, which did not change; did you mean type:HookId?; "
                         "breaking change without a change note: type:HookId")
        self.assertEqual(pr_ready.surface_summary("sunrise-stage: change note a.toml has no `why`\n"),
                         "sunrise-stage: change note a.toml has no `why`")
        self.assertEqual(pr_ready.surface_summary("No surface report: no base for a local run.\n"),
                         "No surface report: no base for a local run.")
        step = pr_ready.Step("Surface report", "x", summary=pr_ready.surface_summary)
        step.status, step.output = "fail", failing
        with contextlib.redirect_stderr(io.StringIO()):
            pr_ready.finish(step, verbose=False)
        self.assertIn("breaking change without a change note: type:HookId", step.detail)

    def test_shared_rust_configuration_keeps_workspace_coverage(self):
        for path in sorted(pr_ready.ROOT_RUST | {".cargo/config.toml"}):
            with self.subTest(path=path), mock.patch.object(pr_ready, "affected_here", return_value=[]):
                steps = pr_ready.plan(args(quick=True), "origin/main", {path}, PACKAGES, LIBRARIES, DEPENDENCIES)
                by_name = {step.name: step for step in steps}
                self.assertEqual(by_name["clippy"].command, CLIPPY)
                self.assertEqual(by_name["tests: workspace"].command,
                                 "cargo test --locked --workspace" + MODS + " --tests")
                self.assertEqual(by_name["doc tests"].command, "cargo test --locked --workspace --doc")

    def test_holder_actor_is_built_before_applicable_tests(self):
        packages = {**PACKAGES, "sunrise-mod-package": "crates/sunrise-mod-package"}
        for path in ("Cargo.lock", ".cargo/config.toml", "crates/sunrise-mod-package/src/holder.rs"):
            with self.subTest(path=path), mock.patch.object(pr_ready, "affected_here", return_value=[]):
                steps = pr_ready.plan(args(quick=True), "origin/main", {path}, packages,
                                      LIBRARIES, DEPENDENCIES)
                by_name = {step.name: step for step in steps}
                actor = by_name["holder actor"]
                self.assertIsNone(actor.skip)
                self.assertEqual(actor.command, "cargo build --locked -p sunrise-mod-package "
                                 "--features runtime-process --example holder_actor")
                # No cd, target-dir or profile override: actor() finds the sibling
                # examples directory in the root test target, including CARGO_TARGET_DIR.
                self.assertLess(steps.index(actor), next(i for i, step in enumerate(steps)
                                if step.name.startswith("tests")))
        for path in ("README.md", "crates/sunrise-core/src/lib.rs"):
            with self.subTest(path=path), mock.patch.object(pr_ready, "affected_here", return_value=[]):
                steps = pr_ready.plan(args(quick=True), "origin/main", {path}, packages,
                                      LIBRARIES, DEPENDENCIES)
                self.assertEqual(next(step for step in steps if step.name == "holder actor").skip,
                                 "holder tests untouched")

    def test_tests_get_the_features_ci_s_workspace_run_gives_them(self):
        self.assertEqual(pr_ready.mod_features(["sunrise-world"]), "")
        self.assertEqual(pr_ready.mod_features(["sunrise-client"]), "")
        self.assertEqual(pr_ready.mod_features(["sunrise-mod-package"]),
                         " --features sunrise-mod-package/index")
        self.assertEqual(pr_ready.mod_features(None), MODS)

    def test_mod_api_data_inputs_build_and_test_the_consuming_crates(self):
        packages = {**PACKAGES, "sunrise-client-render": "crates/sunrise-client-render",
                    "sunrise-mod-api-gen": "crates/sunrise-mod-api-gen",
                    "sunrise-gameplay-common": "crates/sunrise-gameplay-common",
                    "sunrise-gameplay": "crates/sunrise-gameplay",
                    "sunrise-client-gameplay": "crates/sunrise-client-gameplay"}
        for path in ("crates/sunrise-mod-vocab/src/hooks/player.rs",):
            with self.subTest(path=path), mock.patch.object(pr_ready, "affected_here", return_value=[]):
                steps = pr_ready.plan(args(quick=True), "origin/main", {path}, packages,
                                      LIBRARIES | {"sunrise-client-render", "sunrise-mod-api-gen",
                                                   "sunrise-gameplay-common", "sunrise-gameplay",
                                                   "sunrise-client-gameplay"}, DEPENDENCIES)
                by_name = {step.name: step for step in steps}
                self.assertIsNone(by_name["clippy"].skip)
                self.assertIsNone(by_name["rustfmt"].skip)
                self.assertEqual(by_name["rustfmt"].command, "python3 tools/ci/format_scan.py")
                tests = next(step for step in steps if step.name.startswith("tests:"))
                self.assertIsNone(tests.skip)
                self.assertEqual(tests.command, "cargo test --locked -p sunrise-client -p sunrise-client-gameplay "
                                               "-p sunrise-client-render -p sunrise-gameplay -p sunrise-gameplay-common "
                                               "-p sunrise-mod-api-gen --tests")

    @unittest.skipUnless(os.name == "nt", "the Windows runner's own jobs")
    def test_a_windows_check_ends_what_it_left_running(self):
        """A check that exits leaving a child that holds its output: the run goes on, and the child is ended."""
        leaving = ("import subprocess, sys; subprocess.Popen([sys.executable, '-c', 'import time; time.sleep(120)']);"
                   " print('left one')")
        step = pr_ready.Step("leaving", f"python3 -c \"{leaving}\"")
        started = time.monotonic()
        pr_ready.run_windows([step], verbose=False)
        self.assertLess(time.monotonic() - started, 60)
        self.assertEqual((step.status, step.output.strip()), ("pass", "left one"))

    def test_python_tests_always_run(self):
        with mock.patch.object(pr_ready, "affected_here", return_value=[]):
            steps = pr_ready.plan(args(quick=True), "origin/main", {"crates/sunrise-core/src/lib.rs"},
                                  PACKAGES, LIBRARIES, DEPENDENCIES)
        self.assertIsNone({step.name: step for step in steps}["Python tests"].status)

    def test_non_rust_changes_skip_workspace_checks(self):
        for path in ("README.md", "tools/pr_ready.py"):
            with self.subTest(path=path), mock.patch.object(pr_ready, "affected_here", return_value=[]):
                steps = pr_ready.plan(args(quick=True), "origin/main", {path}, PACKAGES, LIBRARIES, DEPENDENCIES)
                by_name = {step.name: step for step in steps}
                for name in ("clippy", "tests", "doc tests"):
                    self.assertEqual(by_name[name].status, "skip")

    def test_multiple_touched_crates_share_the_same_check_scope(self):
        with mock.patch.object(pr_ready, "affected_here", return_value=[]):
            steps = pr_ready.plan(args(), "origin/main",
                                  {"crates/sunrise-core/src/lib.rs", "crates/sunrise-client/examples/probe.rs"}, PACKAGES, LIBRARIES, DEPENDENCIES)
        for step in steps:
            if step.name in ("tests: sunrise-client, sunrise-core", "doc tests"):
                self.assertIn("-p sunrise-client -p sunrise-core", step.command)
                self.assertIsNone(step.status)

    def test_binary_only_crates_skip_doc_tests(self):
        with mock.patch.object(pr_ready, "affected_here", return_value=[]):
            steps = pr_ready.plan(args(), "origin/main", {"xtask/src/main.rs"}, PACKAGES, LIBRARIES, DEPENDENCIES)
        by_name = {step.name: step for step in steps}
        self.assertEqual(by_name["tests: xtask"].command, "cargo test --locked -p xtask --tests")
        self.assertIsNone(by_name["tests: xtask"].status)
        self.assertEqual(by_name["doc tests"].status, "skip")
        with mock.patch.object(pr_ready, "affected_here", return_value=[]):
            steps = pr_ready.plan(args(), "origin/main",
                                  {"xtask/src/main.rs", "crates/sunrise-core/src/lib.rs"}, PACKAGES, LIBRARIES, DEPENDENCIES)
        by_name = {step.name: step for step in steps}
        # sunrise-client depends on sunrise-core, so its doc tests run too.
        self.assertEqual(by_name["doc tests"].command, "cargo test --locked -p sunrise-client -p sunrise-core --doc")

    def test_clippy_is_ci_s_and_the_server_build_when_the_game_is_affected(self):
        packages = {name: f"crates/{name}" for name in ("core", "world", "sunrise-client", "unrelated")}
        dependencies = {"world": {"core"}, "sunrise-client": {"world"}, "unrelated": set()}
        with mock.patch.object(pr_ready, "affected_here", return_value=[]):
            steps = pr_ready.plan(args(), "origin/main", {"crates/core/src/lib.rs"},
                                  packages, set(packages), dependencies)
            unrelated = pr_ready.plan(args(), "origin/main", {"crates/unrelated/src/lib.rs"},
                                      packages, set(packages), dependencies)
        self.assertEqual({step.name: step for step in unrelated}["clippy"].command, pr_ready.CI_CLIPPY)
        by_name = {step.name: step for step in steps}
        self.assertEqual(by_name["clippy"].command, CLIPPY)
        # The dependents of core (world, then sunrise-client through it) are tested with it; unrelated is not.
        self.assertEqual(by_name["tests: core, sunrise-client, world"].command,
                         "cargo test --locked -p core -p sunrise-client -p world --tests")
        self.assertEqual(by_name["doc tests"].command,
                         "cargo test --locked -p core -p sunrise-client -p world --doc")

    def test_dependent_scope_handles_cycles_and_multiple_changed_crates(self):
        dependencies = {"a": {"b"}, "b": {"a"}, "c": {"b"}, "d": set()}
        self.assertEqual(pr_ready.dependent_crates(["a"], dependencies), ["a", "b", "c"])
        self.assertEqual(pr_ready.dependent_crates(["a", "d"], dependencies), ["a", "b", "c", "d"])
        self.assertEqual(pr_ready.dependent_crates([], dependencies), [])
        self.assertIsNone(pr_ready.dependent_crates(None, dependencies))

    def selected_steps(self, changed, packages=SURFACE_PACKAGES, dependencies=SURFACE_DEPENDENCIES, **flags):
        with mock.patch.object(pr_ready, "affected_here", return_value=[]):
            steps = pr_ready.plan(args(quick=True, **flags), "origin/main", set(changed), packages,
                                  set(packages) - {"xtask"}, dependencies)
        return {step.name: step for step in steps}

    def test_tests_doc_tests_and_holder_actor_cover_transitive_dependents(self):
        # sunrise-core is used by sunrise-client and sunrise-mod-package, and sunrise-stage uses the package.
        by_name = self.selected_steps({"crates/sunrise-core/src/lib.rs"})
        crates = "-p sunrise-client -p sunrise-core -p sunrise-mod-package -p sunrise-stage"
        tests = by_name["tests: sunrise-client, sunrise-core, sunrise-mod-package, sunrise-stage"]
        self.assertEqual(tests.command, f"cargo test --locked {crates} --features sunrise-mod-package/index --tests")
        self.assertEqual(by_name["doc tests"].command, f"cargo test --locked {crates} --doc")
        self.assertIsNone(by_name["holder actor"].skip)
        # Touching the crate directly keeps working, and a leaf crate pulls in nothing.
        self.assertIsNone(self.selected_steps({"crates/sunrise-mod-package/src/a.rs"})["holder actor"].skip)
        leaf = self.selected_steps({"crates/sunrise-client/src/game/a.rs"})
        self.assertEqual(leaf["holder actor"].skip, "holder tests untouched")
        self.assertEqual(leaf["tests: sunrise-client"].command, "cargo test --locked -p sunrise-client --tests")

    def test_a_large_dependent_closure_keeps_the_tests_title_short(self):
        names = [f"c{number:02}" for number in range(15)]
        packages = {name: f"crates/{name}" for name in names}
        dependencies = {names[i + 1]: {names[i]} for i in range(14)}
        by_name = self.selected_steps({"crates/c00/src/lib.rs"}, packages, dependencies)
        title = f"tests: {', '.join(names[:12])} and 3 more"
        self.assertIn(title, by_name)
        self.assertEqual(by_name[title].command,
                         "cargo test --locked " + " ".join(f"-p {name}" for name in names) + " --tests")

    def test_touched_crates_stays_the_directly_touched_set(self):
        self.assertEqual(pr_ready.touched_crates({"crates/sunrise-core/src/lib.rs"}, SURFACE_PACKAGES),
                         ["sunrise-core"])

    def test_an_unmapped_rust_path_is_refused_not_run_as_the_workspace(self):
        unknown = ["crates/unknown/src/lib.rs", "crates/another/Cargo.toml"]
        for changed in (unknown[:1], unknown, [*unknown, "crates/sunrise-core/src/lib.rs"],
                        [*unknown, "Cargo.toml"], [*unknown, ".cargo/config.toml"]):
            with self.subTest(changed=changed), mock.patch.object(pr_ready, "affected_here", return_value=[]):
                with self.assertRaises(ValueError) as caught:
                    pr_ready.plan(args(quick=True), "origin/main", set(changed), PACKAGES, LIBRARIES, DEPENDENCIES)
                for path in set(changed) & set(unknown):
                    self.assertIn(path, str(caught.exception))
                self.assertIn("--full", str(caught.exception))
        # Documentation next to a crate's sources, and the paths no workspace builds, are not Rust to refuse.
        for path in ("crates/unknown/README.md", "tools/pr_ready.py", "docs/guide.md"):
            with self.subTest(path=path), mock.patch.object(pr_ready, "affected_here", return_value=[]):
                steps = pr_ready.plan(args(quick=True), "origin/main", {path}, PACKAGES, LIBRARIES, DEPENDENCIES)
                self.assertEqual({step.name: step for step in steps}["clippy"].status, "skip")

    def test_the_refusal_names_every_unmapped_path_sorted(self):
        with self.assertRaises(ValueError) as caught:
            pr_ready.select_crates(args(), {"crates/zeta/a.rs", "crates/alpha/b.rs", "crates/sunrise-core/a.rs"},
                                   PACKAGES)
        self.assertIn("no workspace crate owns crates/alpha/b.rs, crates/zeta/a.rs:", str(caught.exception))

    def test_separate_workspaces_are_not_built_by_the_main_workspace(self):
        for path in ("crates/sunrise-telemetry-server/src/main.rs",):
            with self.subTest(path=path):
                self.assertFalse(pr_ready.workspace_rust(path))
                by_name = self.selected_steps({path}, PACKAGES, DEPENDENCIES)
                for name in ("rustfmt", "clippy"):
                    self.assertEqual(by_name[name].skip, "no Rust change")
                self.assertEqual(by_name["tests"].skip, "no crate touched")
                self.assertEqual(by_name["doc tests"].skip, "no crate touched")
        # Beside a mapped crate they neither widen the selection nor get refused.
        by_name = self.selected_steps({"crates/sunrise-telemetry-server/src/main.rs", "xtask/src/main.rs"}, PACKAGES,
                                      DEPENDENCIES)
        self.assertEqual(by_name["tests: xtask"].command, "cargo test --locked -p xtask --tests")

    def test_full_runs_the_workspace_whatever_changed(self):
        for changed in ({"crates/sunrise-client/src/game/a.rs"}, {"README.md"}, {"crates/unknown/src/lib.rs"},
                        {"crates/sunrise-telemetry-server/src/main.rs"}):
            with self.subTest(changed=changed):
                by_name = self.selected_steps(changed, full=True)
                self.assertEqual(by_name["clippy"].command, CLIPPY)
                self.assertEqual(by_name["tests: workspace"].command, "cargo test --locked --workspace --tests")
                self.assertEqual(by_name["doc tests"].command, "cargo test --locked --workspace --doc")
                self.assertIsNone(by_name["holder actor"].skip)
                self.assertIsNone(by_name["tests: workspace"].skip)
        self.assertEqual(pr_ready.select_crates(args(full=True), {"README.md"}, {}), (None, "--full"))
        step = pr_ready.Step("tests: workspace")
        step.status = "pass"
        text = pr_ready.block(args(full=True), "origin/main", "abcdef123", [], {}, [step], "skip-release", 0,
                              ("a" * 40, "", ""), test_scope="workspace (--full)")
        self.assertIn("`tools/pr_ready.py --full`", text.splitlines()[0])
        self.assertIn("| Test scope | workspace (--full) |", text)

    def test_root_configuration_selects_the_workspace_and_is_checked_for_unmapped_paths(self):
        self.assertEqual(pr_ready.select_crates(args(), {"Cargo.toml", ".cargo/config.toml", "crates/sunrise-core/a.rs"},
                                                PACKAGES),
                         (None, "root config: .cargo/config.toml, Cargo.toml"))
        self.assertEqual(pr_ready.format_test_scope(None, {}, "root config: Cargo.toml"),
                         "workspace (root config: Cargo.toml)")
        with self.assertRaises(ValueError):
            pr_ready.select_crates(args(), {"Cargo.toml", "crates/unknown/src/lib.rs"}, PACKAGES)
        self.assertEqual(pr_ready.select_crates(args(full=True), {"Cargo.toml", "crates/unknown/src/lib.rs"}, PACKAGES),
                         (None, "--full"))

    def test_selection_reasons_name_the_dependency_that_pulled_each_crate_in(self):
        dependencies = {"world": {"core"}, "client": {"world"}, "render": {"core", "assets"}, "app": {"client", "render"},
                        "assets": set(), "lone": set()}
        reasons = pr_ready.crate_selection_reasons(["core"], dependencies)
        self.assertEqual(list(reasons.items()), [
            ("core", "touched"), ("render", "depends on core"), ("world", "depends on core"),
            ("app", "depends on render"), ("client", "depends on world")])
        # Two touched crates reaching one dependent in the same wave are both named; a touched crate stays touched.
        reasons = pr_ready.crate_selection_reasons({"core", "assets"}, dependencies)
        self.assertEqual(reasons["render"], "depends on assets, core")
        self.assertEqual(reasons["assets"], "touched")
        reasons = pr_ready.crate_selection_reasons(["core", "world"], dependencies)
        self.assertEqual((reasons["world"], reasons["client"]), ("touched", "depends on world"))
        # The shortest wave wins: app uses core directly and through client.
        self.assertEqual(pr_ready.crate_selection_reasons(["core"], {**dependencies, "app": {"core", "client"}})["app"],
                         "depends on core")
        # A cycle terminates, and holds exactly the crates dependent_crates selects.
        cycle = {"a": {"b"}, "b": {"a"}, "c": {"b"}}
        self.assertEqual(list(pr_ready.crate_selection_reasons(["a"], cycle).items()),
                         [("a", "touched"), ("b", "depends on a"), ("c", "depends on b")])
        self.assertEqual(sorted(pr_ready.crate_selection_reasons(["a"], cycle)), pr_ready.dependent_crates(["a"], cycle))

    def test_the_test_scope_reads_as_one_line(self):
        dependencies = {"world": {"core"}, "sunrise-client": {"world"}}
        self.assertEqual(pr_ready.format_test_scope(["core"], dependencies),
                         "core (touched), world (depends on core), sunrise-client (depends on world)")
        self.assertEqual(pr_ready.format_test_scope(["core"], {}), "core (touched)")
        self.assertEqual(pr_ready.format_test_scope(None, {}, "--full"), "workspace (--full)")
        self.assertIsNone(pr_ready.format_test_scope([], dependencies))

    def test_block_shows_the_test_scope_after_the_scope_row_only_when_there_is_one(self):
        def rows(**extra):
            return pr_ready.block(args(), "origin/main", "abcdef123", [], {}, [], "skip-release", 0,
                                  ("a" * 40, "", ""), **extra).splitlines()
        without = rows()
        self.assertFalse(any(row.startswith("| Test scope") for row in without))
        shown = rows(test_scope="core (touched), world (depends on core)")
        self.assertEqual(shown[without.index(next(row for row in without if row.startswith("| Scope"))) + 1],
                         "| Test scope | core (touched), world (depends on core) |")

    def test_block_accepts_a_namespace_without_a_full_attribute(self):
        legacy = argparse.Namespace(host="local", quick=True, harnesses=False)
        text = pr_ready.block(legacy, "origin/main", "abcdef123", [], {}, [], "skip-release", 0, ("a" * 40, "", ""))
        self.assertIn("`tools/pr_ready.py --quick`", text.splitlines()[0])

    def run_main(self, changed, *argv):
        packages = (PACKAGES, LIBRARIES, DEPENDENCIES)
        with mock.patch.object(sys, "argv", ["pr_ready.py", "--quick", *argv]), \
                mock.patch.object(pr_ready.parity, "changed_files", return_value=set(changed)), \
                mock.patch.object(pr_ready, "git", return_value="abc123\n"), \
                mock.patch.object(pr_ready, "line_counts", return_value={}), \
                mock.patch.object(pr_ready, "packages", return_value=packages), \
                mock.patch.object(pr_ready, "affected_here", return_value=[]), \
                mock.patch.object(pr_ready, "rust_version", return_value=None), \
                mock.patch.object(pr_ready, "revision_state", return_value=("a" * 40, "", "")), \
                mock.patch.object(pr_ready.release_train, "exporters", return_value=set()), \
                mock.patch.object(pr_ready, "run_checks", side_effect=lambda steps, host, verbose: [
                    setattr(step, "status", step.status or "pass") for step in steps]) as run_checks, \
                contextlib.redirect_stderr(io.StringIO()) as log, contextlib.redirect_stdout(io.StringIO()) as out:
            try:
                code = pr_ready.main()
            except SystemExit as error:
                code = error.code
        return code, log.getvalue(), out.getvalue(), run_checks

    def test_main_prints_the_test_scope_and_refuses_unmapped_paths_cleanly(self):
        code, log, out, run_checks = self.run_main({"crates/sunrise-core/src/lib.rs"})
        lines = log.splitlines()
        self.assertEqual(code, 0)
        header = next(index for index, line in enumerate(lines) if line.startswith("pr_ready: 1 files changed"))
        self.assertEqual(lines[header + 1],
                         "pr_ready: test scope: sunrise-core (touched), sunrise-client (depends on sunrise-core)")
        self.assertIn("| Test scope | sunrise-core (touched), sunrise-client (depends on sunrise-core) |", out)
        code, log, out, run_checks = self.run_main({"README.md"})
        self.assertNotIn("test scope", log)
        self.assertNotIn("Test scope", out)
        code, log, out, run_checks = self.run_main({"Cargo.toml"})
        self.assertIn("pr_ready: test scope: workspace (root config: Cargo.toml)", log)
        code, log, out, run_checks = self.run_main({"crates/unknown/src/lib.rs"}, "--full")
        self.assertIn("pr_ready: test scope: workspace (--full)", log)
        self.assertIn("`tools/pr_ready.py --quick --full", out)
        code, log, out, run_checks = self.run_main({"crates/unknown/src/lib.rs"})
        self.assertEqual(out, "")
        self.assertIsInstance(code, str)
        self.assertTrue(code.startswith("pr_ready: no workspace crate owns crates/unknown/src/lib.rs"))
        self.assertIn("--full", code)
        run_checks.assert_not_called()

    def test_metadata_includes_renamed_optional_platform_build_and_dev_consumers(self):
        core = str(pr_ready.ROOT / "crates/core")
        metadata = {"workspace_members": ["core", "optional", "build", "dev"], "packages": [
            {"id": "core", "name": "core", "manifest_path": core + "/Cargo.toml",
             "targets": [{"kind": ["lib"]}], "dependencies": []},
        ]}
        for name, kind in (("optional", None), ("build", "build"), ("dev", "dev")):
            metadata["packages"].append({
                "id": name, "name": name, "manifest_path": str(pr_ready.ROOT / f"crates/{name}/Cargo.toml"),
                "targets": [{"kind": ["bin"]}], "dependencies": [
                    {"name": "core", "rename": "alias", "path": core, "kind": kind,
                     "optional": name == "optional", "target": "cfg(windows)"},
                    {"name": "external", "path": None},
                ],
            })
        # A path dependency outside this workspace must not become a selectable package.
        metadata["packages"].append({"id": "outside", "name": "outside", "manifest_path": "/outside/Cargo.toml"})
        with mock.patch.object(pr_ready.subprocess, "run", return_value=mock.Mock(stdout=json.dumps(metadata))):
            directories, libraries, dependencies = pr_ready.packages()
        self.assertEqual(set(directories), {"core", "optional", "build", "dev"})
        self.assertEqual(libraries, {"core"})
        self.assertEqual(pr_ready.dependent_crates(["core"], dependencies), ["build", "core", "dev", "optional"])

    def test_scope_warns_on_mixed_areas(self):
        self.assertEqual(pr_ready.scope_warnings({"crates/sunrise-theme/src/lib.rs", "crates/sunrise-gameplay-common/src/game.rs"}), [])
        self.assertEqual(pr_ready.scope_warnings({"crates/sunrise-gameplay-common/src/game.rs", ".github/workflows/ci.yml"}),
                         ["mixes CI, game"])

    def test_cargo_lock_changes_only_with_a_manifest(self):
        self.assertIsNone(pr_ready.lock_problem({"crates/sunrise-core/src/lib.rs"}))
        self.assertIsNone(pr_ready.lock_problem({"Cargo.lock", "crates/sunrise-client/Cargo.toml"}))
        self.assertIn("Cargo.lock", pr_ready.lock_problem({"Cargo.lock", "crates/sunrise-core/src/lib.rs"}))

    def test_label_follows_what_players_get(self):
        self.assertEqual(pr_ready.suggested_label({"tools/pr_ready.py", "CONTRIBUTING.md"}), "`skip-release`")
        self.assertEqual(pr_ready.suggested_label({"crates/sunrise-world/examples/probe.rs"}), "`skip-release`")
        self.assertIn("feature", pr_ready.suggested_label({"crates/sunrise-gameplay-common/src/game.rs"}))

    def test_affected_harnesses_come_from_the_parity_runner(self):
        found = {"food": "tools/test_food_parity.py", "datafix#1": "tools/test_datafix_parity.py",
                 "datafix#2": "tools/test_datafix_parity.py"}
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            # Exercise the real selector/SourceGraph on the records' tiny workspace, not
            # every crate in this checkout. All three selection cases share this input.
            (root / "Cargo.toml").write_text('[workspace]\nmembers = ["crates/sunrise-world"]\n')
            crate = root / "crates/sunrise-world"
            (crate / "src").mkdir(parents=True)
            (crate / "Cargo.toml").write_text(
                '[package]\nname = "sunrise-world"\nversion = "0.0.0"\n')
            (crate / "src/lib.rs").write_text('pub mod food;\npub mod datafix;\n')
            for module in ("food", "datafix"):
                (crate / f"src/{module}.rs").write_text('pub fn run() {}\n')
            found = {name: str(root / script) for name, script in found.items()}
            with mock.patch.object(pr_ready.parity, "harnesses", return_value=found), \
                    mock.patch.object(pr_ready.parity, "load_state", return_value={"harnesses": {
                        "food": {"dependencies": ["crates/sunrise-world/src/food.rs"]},
                        "datafix#1": {"dependencies": ["crates/sunrise-world/src/datafix.rs"]},
                        "datafix#2": {"dependencies": ["crates/sunrise-world/src/datafix.rs"]}}}), \
                    mock.patch.object(pr_ready.parity, "ROOT", root), \
                    mock.patch.object(pr_ready.parity, "SHARED", root / "cache"), \
                    mock.patch.object(pr_ready.parity, "STATE_DIR", root / "cache"):
                self.assertEqual(pr_ready.affected_here({"crates/sunrise-world/src/datafix.rs"}), ["datafix"])
                self.assertEqual(pr_ready.affected_here({"README.md"}), [])
                self.assertEqual(pr_ready.affected_here({"Cargo.lock"}), ["food", "datafix"])

    def test_actual_sources_place_food_probe_and_select_reached_source(self):
        # Parse the checkout's actual manifest, probe and reached implementation,
        # not another synthetic Rust program. A narrow root avoids rebuilding the
        # whole-workspace graph whose cost this test-only change removes.
        parity = pr_ready.parity
        probe = "crates/sunrise-world/examples/probe/food_fixture.rs"
        source = "crates/sunrise-world/src/food.rs"
        dependencies = {
            "crates/sunrise-world/examples/probe/main.rs", probe,
            "crates/sunrise-world/src/lib.rs", source,
        }
        artifacts = [("example", "probe", None, ["-p", "sunrise-world"])]
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            for path in dependencies | {"crates/sunrise-world/Cargo.toml"}:
                destination = root / path
                destination.parent.mkdir(parents=True, exist_ok=True)
                shutil.copyfile(pr_ready.ROOT / path, destination)
            graph = parity.parity_reach.SourceGraph(root, parity.rust_code)
            entries = graph.entries(artifacts, dependencies)
            self.assertIn(probe, entries or (), "the actual probe must be placed, not fallback-selected")
            self.assertIn(source, graph.reachable(entries, dependencies),
                          "the actual food probe must reach its food implementation")

    def test_harnesses_are_listed_when_they_cannot_run(self):
        changed = {"crates/sunrise-world/src/food.rs"}
        with mock.patch.object(pr_ready, "affected_here", return_value=["food", "light"]):
            steps = pr_ready.plan(args(quick=True), "origin/main", changed, {"sunrise-world": "crates/sunrise-world"}, {"sunrise-world"}, {})
        harness = steps[-1]
        self.assertEqual(harness.status, "list")
        self.assertIn("2 affected, to run in CI (--quick): food, light", harness.detail)
        by_name = {step.name: step for step in steps}
        self.assertIn("tests: sunrise-world", by_name)
        self.assertIsNone(by_name["clippy"].status)
        with mock.patch.object(pr_ready, "affected_here", return_value=["food"]):
            steps = pr_ready.plan(args(host="macmini", harnesses=True), "origin/main", changed, {"sunrise-world": "crates/sunrise-world"}, {"sunrise-world"}, {})
        self.assertIn("--changed-files", steps[-1].command)
        self.assertIn("crates/sunrise-world/src/food.rs", steps[-1].command)

    def test_the_local_self_check_runs_harnesses_when_their_tools_change(self):
        packages = {"sunrise-world": "crates/sunrise-world"}
        code = {"crates/sunrise-world/src/food.rs"}
        self.assertFalse(pr_ready.runs_parity(code))
        for path in ("tools/test_food_parity.py", "tools/parity.py", "tools/lab.py", "tools/parity_hosts.json",
                     "mods/sunrise/src/test/java/dev/sunriselab/parity/VanillaFoodOracle.java",
                     "crates/sunrise-world/examples/food_probe.rs", "crates/sunrise-client/src/game/sculk_fixture.rs",
                     "crates/sunrise-gameplay/src/game/blocks/sculk_fixture.rs",
                     "crates/sunrise-gameplay-common/src/game/parity_fixture.rs",
                     "crates/sunrise-client-gameplay/src/game/remote_harness.rs",
                     ".github/workflows/parity.yml"):
            with self.subTest(path=path):
                self.assertTrue(pr_ready.runs_parity(code | {path}))
        with mock.patch.object(pr_ready, "affected_here", return_value=["food", "light"]):
            steps = pr_ready.plan(args(host="macmini"), "origin/main", code, packages, {"sunrise-world"}, {})
            self.assertEqual(steps[-1].status, "list")
            self.assertIsNone(steps[-1].command)
            self.assertIn("2 affected, left to the daily on main", steps[-1].detail)
            self.assertIn("--harnesses runs them here", steps[-1].detail)
            steps = pr_ready.plan(args(host="macmini"), "origin/main", code | {"tools/test_food_parity.py"}, packages,
                                  {"sunrise-world"}, {})
            self.assertIn("--changed-files", steps[-1].command)

    def test_ci_s_rust_follows_the_six_week_train(self):
        self.assertEqual(pr_ready.ci_rust(datetime.date(2025, 2, 20)), 85)
        self.assertEqual(pr_ready.ci_rust(datetime.date(2025, 6, 26)), 88)
        self.assertEqual(pr_ready.ci_rust(datetime.date(2026, 10, 31)), 99)
        self.assertEqual(pr_ready.ci_rust(datetime.date(2026, 11, 12)), 100)

    def test_an_older_rust_warns_without_failing(self):
        today = datetime.date(2026, 10, 31)
        self.assertIsNone(pr_ready.old_rust((1, 99, 1), today))
        self.assertIsNone(pr_ready.old_rust(None, today))
        warning = pr_ready.old_rust((1, 97, 0), today)
        self.assertIn("1.97.0 is older than CI's stable (1.99)", warning)
        self.assertIn("`rustup update`", warning)
        steps = pr_ready.plan(args(quick=True), "origin/main", {"crates/sunrise-core/src/jmath.rs"}, PACKAGES, LIBRARIES,
                              DEPENDENCIES)
        by_name = {step.name: step for step in steps}
        by_name["clippy"].status, by_name["clippy"].detail = "fail", "failed"
        pr_ready.mark_old_rust(steps, warning)
        names = [step.name for step in steps]
        self.assertEqual(names.index("Rust") + 1, names.index("clippy"))
        self.assertEqual(steps[names.index("Rust")].status, "warn")
        self.assertEqual(by_name["clippy"].detail, "failed; likely the old Rust above")
        steps = pr_ready.plan(args(quick=True), "origin/main", {"README.md"}, PACKAGES, LIBRARIES, DEPENDENCIES)
        pr_ready.mark_old_rust(steps, warning)
        self.assertNotIn("Rust", [step.name for step in steps])

    def test_summaries_of_check_output(self):
        output = "test result: ok. 3 passed; 0 failed;\ntest result: FAILED. 4 passed; 1 failed;\n"
        self.assertEqual(pr_ready.tests_passed(output), "7 passed, 1 failed")
        self.assertEqual(pr_ready.unittest_passed("....\nRan 12 tests in 1.0s\n\nOK\n"), "12 passed")
        self.assertEqual(pr_ready.unittest_passed("Ran 12 tests in 1.0s\n\nFAILED (failures=2)\n"),
                         "12 run, FAILED (failures=2)")
        parity_output = ("3 files changed: 2 harnesses depend on them, 2 without a pass for their current inputs.\n"
                         "\n1 passed, 1 failed in 2.0 min.\n  light: /x/.local/parity-runner/logs/light.log\n")
        self.assertEqual(pr_ready.parity_summary(parity_output), "2 affected, 1 passed, failed: light")
        self.assertEqual(pr_ready.parity_summary("2 files changed: 0 harnesses depend on them, 0 without a pass for "
                                                 "their current inputs.\nNothing to run: every harness passed."),
                         "no harness affected")
        self.assertEqual(pr_ready.duration(125), "2 min 5 s")

    def test_unittest_summary_keeps_skips_and_expected_failures(self):
        class Outcomes(unittest.TestCase):
            def test_pass(self):
                self.assertTrue(True)

            @unittest.skip("fixture platform is unavailable")
            def test_skip(self):
                self.fail("must not run")

            @unittest.expectedFailure
            def test_expected_failure(self):
                self.fail("known fixture failure")

        stream = io.StringIO()
        suite = unittest.defaultTestLoader.loadTestsFromTestCase(Outcomes)
        result = unittest.TextTestRunner(stream=stream).run(suite)
        self.assertTrue(result.wasSuccessful())
        self.assertEqual(pr_ready.unittest_passed(stream.getvalue()),
                         "3 run, OK (skipped=1, expected failures=1)")

    def prepared_root(self, directory, **changes):
        """A root stamped as the checkout's preparer writes it, with `changes` applied (None removes a key)."""
        expected = pr_ready.expected_prepared_stamp()
        manifest = {"schema": expected["schema"], "minecraft": expected["minecraft"]}
        stamp = {"output_version": expected["output_version"]}
        root = Path(directory)
        for key, value in changes.items():
            target = stamp if key == "output_version" else manifest
            if value is None:
                target.pop(key, None)
            else:
                target[key] = value
        (root / "sunrise-resources.json").write_text(json.dumps(manifest))
        if "stamp" not in changes:
            (root / "sunrise-prepare.json").write_text(json.dumps(stamp))
        return root

    def test_expected_prepared_stamp_reads_the_checkout(self):
        expected = pr_ready.expected_prepared_stamp()
        self.assertEqual(set(expected), {"schema", "output_version", "minecraft"})
        self.assertGreater(expected["schema"], 0)
        self.assertGreater(expected["output_version"], 0)
        self.assertRegex(expected["minecraft"], r"^\d+\.\d+")

    def test_prepared_stamp_problems_accept_a_current_root_and_name_each_stale_part(self):
        expected = pr_ready.expected_prepared_stamp()
        with tempfile.TemporaryDirectory() as directory:
            self.assertEqual(pr_ready.prepared_stamp_problems(Path(directory)), ["sunrise-resources.json is missing"])
            root = self.prepared_root(directory)
            self.assertEqual(pr_ready.prepared_stamp_problems(root), [])
            stale = expected["output_version"] - 1
            self.prepared_root(directory, output_version=stale)
            self.assertEqual(pr_ready.prepared_stamp_problems(root),
                             [f"made by output version {stale}, this build needs {expected['output_version']}"])
            self.prepared_root(directory, schema=expected["schema"] - 1)
            self.assertEqual(pr_ready.prepared_stamp_problems(root),
                             [f"an older layout (schema {expected['schema'] - 1}, "
                              f"this preparer writes {expected['schema']})"])
            self.prepared_root(directory, minecraft="1.0")
            self.assertEqual(pr_ready.prepared_stamp_problems(root),
                             [f"prepared for Minecraft 1.0, this build needs {expected['minecraft']}"])
            self.prepared_root(directory)
            (root / "sunrise-prepare.json").unlink()
            self.assertEqual(pr_ready.prepared_stamp_problems(root),
                             [f"made by output version unknown, this build needs {expected['output_version']}"])
            self.prepared_root(directory, profile="server")
            self.assertEqual(pr_ready.prepared_stamp_problems(root),
                             ["prepared for the server profile, tests need full client resources"])
            (root / "sunrise-resources.json").write_text("{")
            self.assertEqual(pr_ready.prepared_stamp_problems(root),
                             ["sunrise-resources.json is not a readable JSON object"])

    def refused(self, host="local", skip=None, **changes):
        step = pr_ready.Step("content root tests", pr_ready.CONTENT_ROOT_TESTS, skip=skip)
        with tempfile.TemporaryDirectory() as directory, \
                mock.patch.dict(os.environ, {"SUNRISE_TEST_RESOURCES": str(self.prepared_root(directory, **changes))}), \
                mock.patch.object(pr_ready.subprocess, "Popen", side_effect=AssertionError("cargo started")), \
                contextlib.redirect_stderr(io.StringIO()):
            pr_ready.refuse_stale_prepared(args(host=host), [step])
            if step.status == "fail":  # the others would run cargo: only a refusal must not
                pr_ready.run_checks([step], host, verbose=False)
        return step

    def test_prepared_stamp_types_are_read_as_serde_reads_them(self):
        expected = pr_ready.expected_prepared_stamp()
        manifest_cases = [{"schema": 7.0}, {"schema": True}, {"schema": "7"}, {"schema": 1 << 63},
                          {"minecraft": 26}, {"profile": 1}, {"server_sha1": 1}, {"facts_sha256": []},
                          {"block_tags_sha256": {}}, {"server_files": [1]}, {"server_files": "a"}]
        stamp_cases = [9.0, True, -1, "9", 1 << 32, [9]]
        with tempfile.TemporaryDirectory() as directory:
            root = self.prepared_root(directory)
            for changes in manifest_cases:
                with self.subTest(changes=changes):
                    self.prepared_root(directory, **changes)
                    self.assertEqual(pr_ready.prepared_stamp_problems(root),
                                     ["sunrise-resources.json is not a valid manifest"])
                    step = self.refused(**changes)
                    self.assertEqual(step.status, "fail")
                    self.assertIn("sunrise-resources.json is not a valid manifest", step.detail)
            for value in stamp_cases:
                with self.subTest(output_version=value):
                    self.prepared_root(directory, output_version=value)
                    self.assertEqual(pr_ready.prepared_stamp_problems(root),
                                     [f"made by output version unknown, this build needs {expected['output_version']}"])
                    step = self.refused(output_version=value)
                    self.assertEqual(step.status, "fail")
                    self.assertIn("made by output version unknown", step.detail)
            # What serde reads: null and absent optional fields, and a well-typed inventory.
            self.prepared_root(directory, server_files=["a"], profile=None, server_sha1="x")
            self.assertEqual(pr_ready.prepared_stamp_problems(root), [])

    def test_a_stale_prepared_root_fails_the_content_root_tests_before_cargo_starts(self):
        expected = pr_ready.expected_prepared_stamp()
        refused = self.refused

        step = refused(output_version=expected["output_version"] - 1)
        self.assertEqual(step.status, "fail")
        self.assertIn(f"made by output version {expected['output_version'] - 1}, "
                      f"this build needs {expected['output_version']}", step.detail)
        self.assertIn("run sunrise-prepare", step.detail)
        self.assertEqual(step.output, step.detail + "\n")
        self.assertIsNone(refused().status)
        self.assertEqual(refused(skip="content root untouched", schema=0).status, "skip")
        self.assertIsNone(refused(host="macmini", schema=0).status)
        # A root that is not there is the test boundary's to name, not the preflight's.
        step = pr_ready.Step("content root tests", pr_ready.CONTENT_ROOT_TESTS)
        with tempfile.TemporaryDirectory() as directory, \
                mock.patch.dict(os.environ, {"SUNRISE_TEST_RESOURCES": str(Path(directory) / "none")}):
            pr_ready.refuse_stale_prepared(args(), [step])
        self.assertIsNone(step.status)

    def test_a_relative_override_is_read_from_the_cargo_test_directory(self):
        """cargo test -p sunrise-client-lib starts in crates/sunrise-client-lib, where Rust reads a relative root."""
        expected = pr_ready.expected_prepared_stamp()
        stale = expected["output_version"] - 1

        def make(root, path, **changes):
            (root / path).mkdir(parents=True, exist_ok=True)
            self.prepared_root(root / path, **changes)
            return (root / path).resolve()

        def run(root, override):
            step = pr_ready.Step("content root tests", pr_ready.CONTENT_ROOT_TESTS)
            with mock.patch.object(pr_ready, "ROOT", root), \
                    mock.patch.object(pr_ready, "expected_prepared_stamp", return_value=expected), \
                    mock.patch.dict(os.environ), \
                    mock.patch.object(pr_ready.subprocess, "Popen", side_effect=AssertionError("cargo started")), \
                    contextlib.redirect_stderr(io.StringIO()):
                if override is None:
                    os.environ.pop("SUNRISE_TEST_RESOURCES", None)
                else:
                    os.environ["SUNRISE_TEST_RESOURCES"] = override
                pr_ready.refuse_stale_prepared(args(), [step])
                if step.status == "fail":
                    pr_ready.run_checks([step], "local", verbose=False)
            return step

        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            crate = "crates/sunrise-client-lib/rel"
            # The checkout's `rel` is current, the crate's is stale: the crate's is the one cargo's tests read.
            make(root, "rel")
            inside = make(root, crate, output_version=stale)
            step = run(root, "rel")
            self.assertEqual(step.status, "fail")
            self.assertIn(f'--output "{inside}"', step.detail)
            self.assertNotIn(f'--output "{root / "rel"}"', step.detail)
            # The other way round, nothing is refused.
            make(root, "rel", output_version=stale)
            make(root, crate)
            self.assertIsNone(run(root, "rel").status)
            # `..` segments come back to the checkout and print without them.
            lock = ".local/sunrise-client/26.3"
            relative = f"../../{lock}"
            outside = make(root, lock, output_version=stale)
            step = run(root, relative)
            self.assertEqual(step.status, "fail")
            self.assertIn(f'--output "{outside}"', step.detail)
            self.assertNotIn("..", step.detail)
            make(root, lock)
            self.assertIsNone(run(root, relative).status)
            # An empty override is Rust's non-directory, not Python's ".": the crate's own directory (here
            # stale) is never named, and the boundary reports the missing inputs.
            make(root, "crates/sunrise-client-lib", output_version=stale)
            step = run(root, "")
            self.assertIsNone(step.status)
            self.assertNotIn("sunrise-client-lib", step.detail)
            # A missing segment before `..` fails the OS lookup, though lexically the path is the stale checkout
            # root: nothing is refused.
            make(root, lock, output_version=stale)
            self.assertFalse((root / "crates/sunrise-client-lib/missing").exists())
            self.assertIsNone(run(root, f"missing/../../../{lock}").status)
            # No override: the checkout's own prepared root.
            make(root, f".local/sunrise-client/{expected['minecraft']}", output_version=stale)
            self.assertEqual(run(root, None).status, "fail")

    def test_python_checks_run_beside_cargo_and_harnesses_after_it(self):
        steps = pr_ready.plan(args(), "origin/main", {"crates/sunrise-core/src/lib.rs"}, PACKAGES, LIBRARIES,
                              DEPENDENCIES)
        steps[0].status = "pass"
        steps[-1].where, steps[-1].command, steps[-1].status = "here", "parity", None
        cargo_started = threading.Event()
        runs = []

        def run_script(group, host, verbose):
            names = [step.name for step in group]
            if "Python tests" in names:
                # Waits for cargo: running them first, alone, would time out here.
                runs.append(("python", cargo_started.wait(2)))
            elif "rustfmt" in names:
                cargo_started.set()
                runs.append(("cargo", host))
            elif names:
                runs.append(("after", names))

        with mock.patch.object(pr_ready, "run_script", run_script):
            pr_ready.run_checks(steps, "local", verbose=False)
        self.assertCountEqual(runs[:2], [("python", True), ("cargo", "local")])
        self.assertEqual(runs[2], ("after", ["Parity"]))

        runs.clear()
        cargo_started.clear()
        with mock.patch.object(pr_ready, "run_script", run_script):
            pr_ready.run_checks(steps, "local", verbose=True)
        self.assertEqual(runs[0], ("python", False))


if __name__ == "__main__":
    unittest.main()
