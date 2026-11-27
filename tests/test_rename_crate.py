"""Crate renames must not rename player data paths or the separate registry repo."""
import importlib.util
from pathlib import Path
import shutil
import subprocess
import sys
from tempfile import TemporaryDirectory
import unittest

from tests.git_fixture import git_in, isolated_git_env

spec = importlib.util.spec_from_file_location("rename_crate", Path(__file__).resolve().parents[1] / "tools/rename_crate.py")
rename = importlib.util.module_from_spec(spec)
spec.loader.exec_module(rename)


def copy_optimized_cli(root):
    script = root / "tools/rename_crate.py"
    script.parent.mkdir()
    shutil.copyfile(rename.__file__, script)

    def run(*args):
        return subprocess.run([sys.executable, "-O", str(script), *args], cwd=root,
                              env=isolated_git_env(), capture_output=True, text=True)
    return run


class RenameCrateTests(unittest.TestCase):
    def test_manifest_and_crate_paths_change_without_touching_player_paths(self):
        source = '''[package]
name = "sunrise-mods"
[dependencies]
sunrise-mods = { path = "../sunrise-mods" }
[dev-dependencies.sunrise-mods]
path = "../crates/sunrise-mods"
# Put files in sunrise-mods/ and enable them in sunrise-mods.txt.
'''
        expected = '''[package]
name = "sunrise-mod-package"
[dependencies]
sunrise-mod-package = { path = "../sunrise-mod-package" }
[dev-dependencies.sunrise-mod-package]
path = "../crates/sunrise-mod-package"
# Put files in sunrise-mods/ and enable them in sunrise-mods.txt.
'''
        self.assertEqual(rename.rewrite("crates/sunrise-mods/Cargo.toml", source), expected)
        self.assertEqual(rename.destination("crates/sunrise-mods/tests/file.smod"),
                         "crates/sunrise-mod-package/tests/file.smod")
        self.assertEqual(rename.destination("crates/sunrise-mods-other/src/lib.rs"),
                         "crates/sunrise-mods-other/src/lib.rs")
        self.assertEqual(rename.rewrite("crates/sunrise-mod-registry/tests/submit.rs", '../sunrise-mods/tests/index'),
                         '../sunrise-mod-package/tests/index')

    def test_rust_library_identifiers_and_link_arguments_change(self):
        source = '''use sunrise_mods::{Content, Side};
mod sunrise_mods { fn fixture() {} }
let link = "sunrise_mods={}";
let module = sunrise_mods::Package;
fn sunrise_mods_test_name() {}
let folder = "sunrise-mods";
let registry = "https://github.com/horizon-rust/horizon-mods";
'''
        expected = '''use sunrise_mod_package::{Content, Side};
mod sunrise_mod_package { fn fixture() {} }
let link = "sunrise_mod_package={}";
let module = sunrise_mod_package::Package;
fn sunrise_mods_test_name() {}
let folder = "sunrise-mods";
let registry = "https://github.com/horizon-rust/horizon-mods";
'''
        self.assertEqual(rename.rewrite("crates/sunrise-client/src/native_mod_host.rs", source), expected)

    def test_verify_rejects_an_unchanged_file_that_still_needs_the_mapping(self):
        root = Path(self.enterContext(TemporaryDirectory()))
        crate = root / rename.OLD_PATH
        crate.mkdir(parents=True)
        (crate / "Cargo.toml").write_bytes(b'[package]\nname = "sunrise-mods"\n')
        consumer = root / "consumer/Cargo.toml"
        consumer.parent.mkdir()
        consumer.write_bytes(b'[dependencies]\nsunrise-mods = { path = "../crates/sunrise-mods" }\n')
        git = git_in(root)
        git("init", "-q")
        git("add", "--", rename.OLD_PATH, "consumer/Cargo.toml")
        git("-c", "user.name=Test", "-c", "user.email=test@example.invalid", "commit", "-qm", "Baseline")
        git("mv", rename.OLD_PATH, rename.NEW_PATH)
        (root / rename.NEW_PATH / "Cargo.toml").write_bytes(b'[package]\nname = "sunrise-mod-package"\n')

        run = copy_optimized_cli(root)
        refused = run("--verify", "HEAD")
        self.assertNotEqual(refused.returncode, 0, refused.stdout)
        self.assertIn("unexpected content change: consumer/Cargo.toml", refused.stderr)
        self.assertNotIn("Verified ", refused.stdout)

        consumer.write_bytes(b'[dependencies]\nsunrise-mod-package = { path = "../crates/sunrise-mod-package" }\n')
        verified = run("--verify", "HEAD")
        self.assertEqual(verified.returncode, 0, verified.stderr)
        self.assertIn("Verified 2 rename-normalized files\n", verified.stdout)

    def test_verify_rejects_an_edit_to_a_non_ascii_path_without_the_old_name(self):
        root = Path(self.enterContext(TemporaryDirectory()))
        crate = root / rename.OLD_PATH
        crate.mkdir(parents=True)
        (crate / "Cargo.toml").write_bytes(b'[package]\nname = "sunrise-mods"\n')
        notes = root / "notes-caf\u00e9.txt"
        baseline = "unrelated notes\n".encode()
        notes.write_bytes(baseline)
        git = git_in(root)
        git("init", "-q")
        git("config", "core.quotePath", "true")
        git("add", "--", rename.OLD_PATH, "notes-caf\u00e9.txt")
        git("-c", "user.name=Test", "-c", "user.email=test@example.invalid", "commit", "-qm", "Baseline")
        git("mv", rename.OLD_PATH, rename.NEW_PATH)
        (root / rename.NEW_PATH / "Cargo.toml").write_bytes(b'[package]\nname = "sunrise-mod-package"\n')

        run = copy_optimized_cli(root)
        notes.write_bytes(b"unrelated edit\n")
        refused = run("--verify", "HEAD")
        self.assertNotEqual(refused.returncode, 0, refused.stdout)
        self.assertIn("unexpected content change: notes-caf\u00e9.txt", refused.stderr)
        self.assertNotIn("Verified ", refused.stdout)

        notes.write_bytes(baseline)
        verified = run("--verify", "HEAD")
        self.assertEqual(verified.returncode, 0, verified.stderr)

    def test_apply_rejects_both_crate_directories_before_rewriting(self):
        root = Path(self.enterContext(TemporaryDirectory()))
        # Empty crate directories make a missed guard fail at git mv, before Cargo runs.
        for crate in (rename.OLD_PATH, rename.NEW_PATH):
            (root / crate).mkdir(parents=True)
        manifest = root / "Cargo.toml"
        original = b'[package]\nname = "sunrise-mods"\n'
        manifest.write_bytes(original)
        git = git_in(root)
        git("init", "-q")
        git("add", "--", "Cargo.toml")
        index = root / ".git/index"
        staged = index.read_bytes()

        run = copy_optimized_cli(root)
        report = root / "report.txt"
        refused = run("--apply", "--report", str(report))
        self.assertNotEqual(refused.returncode, 0, refused.stdout)
        self.assertEqual(refused.stderr, "expected exactly one crate directory\n")
        self.assertEqual(manifest.read_bytes(), original)
        self.assertEqual(index.read_bytes(), staged)
        self.assertFalse(report.exists())

    def test_book_filters_and_translations_obey_the_boundary(self):
        source = '{{#include ../../crates/sunrise-mods/src/lib.rs}}\n(crates/sunrise-mods)\ncrates/sunrise-mods/**\n'
        expected = '{{#include ../../crates/sunrise-mod-package/src/lib.rs}}\n(crates/sunrise-mod-package)\ncrates/sunrise-mod-package/**\n'
        self.assertEqual(rename.rewrite("docs/modding/src/reference.md", source), expected)
        translation = '{"native.mods": "In sunrise-mods; internal type sunrise_mods::Package"}'
        self.assertEqual(rename.rewrite("crates/sunrise-client/assets/sunrise/lang/en_us.json", translation), translation)
        self.assertEqual(rename.rewrite("roadmap/roadmap.toml", 'dev = "sunrise-mods/native/"'),
                         'dev = "sunrise-mods/native/"')
