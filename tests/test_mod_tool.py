"""The rolling mod and server tools: dependency scope, release metadata, and signing boundaries."""
import json
import re
import tempfile
import unittest
from pathlib import Path

from tests.test_ci_concurrency import ROOT, yaml
from tools.ci import mod_tool, server_tool


def package(name, root="/w"):
    return {"id": name, "name": name, "manifest_path": f"{root}/crates/{name}/Cargo.toml"}


def node(name, deps):
    return {"id": name, "deps": [{"pkg": dep, "dep_kinds": [{"kind": kind}]} for dep, kind in deps]}


class ToolFoldersTests(unittest.TestCase):
    def test_the_folders_are_the_workspace_crates_the_tool_builds_with_and_no_others(self):
        metadata = {
            "workspace_root": "/w",
            "workspace_members": ["cargo-sunrise-mod", "sunrise-server", "lib", "build-helper", "only-tests", "unrelated"],
            "packages": [package(n) for n in ("cargo-sunrise-mod", "sunrise-server", "lib", "build-helper",
                                              "only-tests", "unrelated", "registry-crate")],
            "resolve": {"nodes": [
                node("cargo-sunrise-mod", [("lib", None), ("build-helper", "build"), ("only-tests", "dev"),
                                           ("registry-crate", None)]),
                node("sunrise-server", [("lib", None), ("unrelated", "dev")]),
                node("lib", [("registry-crate", None)]),
                node("build-helper", []), node("only-tests", []), node("unrelated", []), node("registry-crate", []),
            ]},
        }
        self.assertEqual(mod_tool.crates("cargo-sunrise-mod", metadata),
                         ["crates/build-helper", "crates/cargo-sunrise-mod", "crates/lib"])
        self.assertEqual(mod_tool.crates("sunrise-server", metadata), ["crates/lib", "crates/sunrise-server"])

    def test_the_real_workspace_names_the_tool_and_its_package_crates(self):
        folders = mod_tool.crates("cargo-sunrise-mod")
        self.assertIn("crates/cargo-sunrise-mod", folders)
        self.assertIn("crates/sunrise-native-build", folders)
        self.assertIn("crates/sunrise-mod-package", folders)
        self.assertNotIn("crates/sunrise-client", folders)

    def test_the_contract_number_in_tool_json_is_the_sources(self):
        with tempfile.TemporaryDirectory() as folder:
            out = Path(folder) / "tool.json"
            mod_tool.tool_json("HEAD", out)
            document = json.loads(out.read_text())
        self.assertEqual(document["tag"], "mod-tool")
        self.assertIsInstance(document["native_contract"], int)
        self.assertIsInstance(document["release_contract"], int)


class ServerToolTests(unittest.TestCase):
    def test_manager_selects_legs_and_uses_the_game_workflows_build_settings(self):
        with tempfile.TemporaryDirectory() as folder:
            game_build = Path(folder) / "game-build.yml"
            game_build.write_text("""
jobs:
  build:
    strategy:
      matrix:
        include:
          - {os: ubuntu-22.04-arm, target: aarch64-unknown-linux-gnu, asset: sunrise-linux-aarch64.tar.gz}
          - {os: macos-15, target: x86_64-apple-darwin, asset: sunrise-macos-x64.zip}
          - {os: windows-latest, asset: sunrise-windows-x64.zip, rustflags: static}
  macos-x64-intel:
    runs-on: macos-15-intel
""")
            platforms = Path(folder) / "server-tool-platforms.txt"
            platforms.write_text("macos x86_64 server-tool-macos-x64.tar.gz\nwindows x86_64 server-tool-windows-x64.zip\n")
            legs = server_tool.matrix(game_build, platforms)["include"]
        self.assertEqual([(leg["asset"], leg["os"], leg["target"], leg["rustflags"], leg["exe"]) for leg in legs], [
            ("server-tool-macos-x64.tar.gz", "macos-15-intel", "x86_64-apple-darwin", "", ""),
            ("server-tool-windows-x64.zip", "windows-latest", "", "static", ".exe"),
        ])

    def test_matrix_refuses_a_manager_platform_without_a_game_leg(self):
        with tempfile.TemporaryDirectory() as folder:
            game_build = Path(folder) / "game-build.yml"
            game_build.write_text("""
jobs:
  build:
    strategy:
      matrix:
        include:
          - {os: ubuntu-22.04, asset: sunrise-linux-x86_64.tar.gz}
""")
            platforms = Path(folder) / "server-tool-platforms.txt"
            platforms.write_text("linux aarch64 server-tool-linux-aarch64.tar.gz\n")
            with self.assertRaises(SystemExit):
                server_tool.matrix(game_build, platforms)

    def test_metadata_binds_the_binary_to_the_build_and_keeps_its_range(self):
        commit = "1" * 40
        document = {"commit": commit, "date": "2000-01-01T00:00:00Z", "supports": {"min": 2, "max": 12}}
        self.assertEqual(server_tool.version_info(document, commit, 946684800), document)
        for changes in ({"commit": "2" * 40}, {"date": "2000-01-01T00:00:01Z"}, {"supports": {"min": 2, "max": 1}}):
            with self.subTest(changes=changes), self.assertRaises(ValueError):
                server_tool.version_info(document | changes, commit, 946684800)

    def test_signed_assets_must_include_the_manifest_and_both_sums_files(self):
        legs = {"include": [{"asset": "server-tool-windows-x64.zip"}]}
        payload = ["server-tool-windows-x64.zip", "server-tool.json"]
        with tempfile.TemporaryDirectory() as directory:
            folder = Path(directory)
            for name in [*payload, "SHA256SUMS"]:
                (folder / name).touch()
            with self.assertRaises(SystemExit):
                server_tool.check_assets(folder, legs, signed=True)
            (folder / "SHA256SUMS.sig").touch()
            self.assertEqual(server_tool.check_assets(folder, legs, signed=True), payload)


class WorkflowTests(unittest.TestCase):
    def load(self, name):
        return yaml.load((ROOT / ".github/workflows" / name).read_text(), Loader=yaml.BaseLoader)

    def test_only_the_sign_job_holds_the_key_and_only_publish_writes(self):
        for name in ("mod-tool.yml", "server-tool.yml"):
            with self.subTest(workflow=name):
                workflow = self.load(name)
                jobs = workflow["jobs"]
                text = (ROOT / ".github/workflows" / name).read_text()
                self.assertEqual(sorted(re.findall(r"secrets\.\w+", text)), ["secrets.RELEASE_SIGNING_KEY",
                                                                            "secrets.RELEASE_SIGNING_KEY_NEXT"])
                self.assertIn("RELEASE_SIGNING_KEY", json.dumps(jobs["sign"]))
                self.assertIn("'release'", jobs["sign"]["environment"])
                for job_name, job in jobs.items():
                    self.assertEqual(job.get("permissions", {}).get("contents") == "write", job_name == "publish")
                    self.assertEqual("environment" in job, job_name == "sign")
                self.assertNotIn("pull_request", workflow["on"])
                # A run started by the push itself would look before the gate had finished.
                self.assertNotIn("push", workflow["on"])
                self.assertEqual(workflow["on"]["workflow_run"]["workflows"], ["dyna-checks"])

    def test_signing_mirrors_the_sdks(self):
        mine = self.load("mod-tool.yml")["jobs"]["sign"]
        sdk = self.load("sdk-publish.yml")["jobs"]["sign"]
        self.assertEqual(mine["runs-on"], sdk["runs-on"])
        checkout = lambda job: next(step for step in job["steps"] if step.get("uses", "").startswith("actions/checkout"))
        self.assertEqual(checkout(mine)["with"], checkout(sdk)["with"])
        signing = lambda job: next(step for step in job["steps"] if step.get("name", "").startswith("Sign SHA256SUMS"))
        self.assertEqual(signing(mine)["run"].replace('"$TAG"', "TAG"), signing(sdk)["run"].replace('"$TAG"', "TAG"))


if __name__ == "__main__":
    unittest.main()
