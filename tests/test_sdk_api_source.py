import os
import tempfile
import tomllib
import unittest
from pathlib import Path
from unittest.mock import patch

from tools import sdk_api_source as api_source
from tools.sdk_api_source import Crate, Refusal


def package(name, directory=None, kinds=("lib",), dependencies=(), source=None):
    return {
        "id": name, "name": name, "version": "0.1.0", "source": source,
        "manifest_path": f"/w/{directory or 'crates/' + name}/Cargo.toml",
        "targets": [{"kind": list(kinds), "name": name}],
        "features": {},
        "dependencies": [{"name": dependency, "kind": kind, "path": path} for dependency, kind, path in dependencies],
    }


def node(name, *edges):
    return {"id": name, "deps": [{"pkg": dependency, "dep_kinds": [{"kind": kind}]} for dependency, kind in edges]}


def metadata():
    """An API crate whose build script runs the generator, a vocabulary crate, a proc macro with a helper crate,
    a registry crate and a dev-only workspace crate."""
    return {
        "workspace_root": "/w",
        "packages": [
            package("sunrise-mod-api", kinds=("lib", "custom-build"),
                    dependencies=[("sunrise-mod-api-gen", "build", "/w/crates/sunrise-mod-api-gen"),
                                  ("helper", "dev", "/w/crates/helper")]),
            package("vocab", "crates/nested/vocab"),
            package("macros", kinds=("proc-macro",)),
            package("shape"),
            package("sunrise-mod-api-gen"),
            package("discovery"),
            package("helper"),
            package("serde", source="registry+https://github.com/rust-lang/crates.io-index"),
        ],
        "resolve": {"nodes": [
            node("sunrise-mod-api", ("vocab", None), ("macros", None), ("serde", None),
                 ("sunrise-mod-api-gen", "build"), ("helper", "dev")),
            node("vocab", ("serde", None)),
            node("macros", ("shape", None)),
            node("shape"),
            node("sunrise-mod-api-gen", ("discovery", None)),
            node("discovery"),
            node("helper"),
            node("serde"),
        ]},
    }


class ClosureTests(unittest.TestCase):
    def test_normal_and_proc_macro_dependencies_only(self):
        self.assertEqual(api_source.closure(metadata()), [
            Crate("macros", "crates/macros", False),
            Crate("shape", "crates/shape", False),
            Crate("sunrise-mod-api", "crates/sunrise-mod-api", True),
            Crate("vocab", "crates/nested/vocab", False),
        ])

    def test_a_generated_crate_not_two_folders_below_the_root_is_refused(self):
        data = metadata()
        data["packages"][0]["manifest_path"] = "/w/crates/mod/sunrise-mod-api/Cargo.toml"
        with self.assertRaises(Refusal) as refused:
            api_source.closure(data)
        self.assertEqual(refused.exception.code, "E_LAYOUT")

    def test_a_build_script_that_is_not_the_generator_is_refused(self):
        data = metadata()
        data["packages"][3]["targets"][0]["kind"].append("custom-build")
        with self.assertRaises(Refusal) as refused:
            api_source.closure(data)
        self.assertEqual(refused.exception.code, "E_BUILD_SCRIPT")


class ManifestTests(unittest.TestCase):
    WORKSPACE = {"workspace": {
        "package": {"edition": "2024", "license": "MIT", "rust-version": "1.95"},
        "dependencies": {"glam": {"version": "0.30", "features": ["serde"]}},
        "lints": {"clippy": {"type_complexity": "allow"}},
    }}
    API = {
        "package": {"name": "sunrise-mod-api", "version": "0.1.0", "edition": {"workspace": True},
                    "license": {"workspace": True}, "publish": False},
        "lib": {"name": "mod_api"},
        "dependencies": {"vocab": {"path": "../nested/vocab"}, "log": "0.4",
                         "glam": {"workspace": True, "features": ["bytemuck"]},
                         "libm": {"version": "0.2", "default-features": False}},
        "target": {"cfg(unix)": {"dependencies": {"libc": "0.2"}}},
        "build-dependencies": {"sunrise-mod-api-gen": {"path": "../sunrise-mod-api-gen"}},
        "dev-dependencies": {"helper": {"path": "../helper"}},
        # Declared before what it names, so dropping it needs a second pass.
        "features": {"chained": ["fixture"], "testing": ["vocab/testing"],
                     "fixture": ["sunrise-mod-api-gen/fixture", "vocab/fixture"]},
        "lints": {"workspace": True},
        "bin": [{"name": "tool", "path": "src/bin/tool.rs"}],
    }
    VOCAB = {
        "package": {"name": "vocab", "version": "0.1.0"},
        "dependencies": {},
        "features": {"testing": [], "fixture": ["sunrise-mod-api-gen/fixture"]},
    }

    def test_features_naming_a_dropped_dependency_go_everywhere_they_reach(self):
        kept = api_source.kept_features({"sunrise-mod-api": self.API, "vocab": self.VOCAB})
        self.assertEqual(kept, {"sunrise-mod-api": {"testing": ["vocab/testing"]}, "vocab": {"testing": []}})

    def test_a_feature_naming_an_optional_dependency_stays(self):
        manifest = {"package": {"name": "core", "version": "0.1.0"},
                    "dependencies": {"libm": {"version": "0.2", "optional": True}},
                    "features": {"fast-math": ["libm"]}}
        self.assertEqual(api_source.kept_features({"core": manifest}), {"core": {"fast-math": ["libm"]}})

    def test_manifest_keeps_the_library_and_its_normal_dependencies(self):
        crate = Crate("sunrise-mod-api", "crates/sunrise-mod-api", True)
        crates = {"sunrise-mod-api": crate, "vocab": Crate("vocab", "crates/nested/vocab", False)}
        # The game resolves glam with its defaults and three features, log with its defaults and `std`, and libm
        # with `arch` and no defaults.
        resolved = {(None, "glam"): ({"default", "std", "bytemuck", "serde"}, True),
                    (None, "log"): ({"default", "std"}, True), (None, "libm"): ({"arch"}, False)}
        written = api_source.normalise_manifest(self.API, self.WORKSPACE, crate, crates,
                                                {"testing": ["vocab/testing"]}, resolved)
        self.assertEqual(tomllib.loads(written), {
            "package": {"name": "sunrise-mod-api", "version": "0.1.0", "edition": "2024", "license": "MIT",
                        "publish": False, "autobins": False, "autoexamples": False, "autotests": False,
                        "autobenches": False, "build": "build.rs"},
            "lib": {"name": "mod_api"},
            "dependencies": {"vocab": {"path": "../nested/vocab"},
                             "log": {"version": "0.4", "features": ["std"]},
                             "glam": {"version": "0.30", "features": ["bytemuck", "serde", "std"]},
                             "libm": {"version": "0.2", "default-features": False, "features": ["arch"]}},
            "target": {"cfg(unix)": {"dependencies": {"libc": "0.2"}}},
            "features": {"testing": ["vocab/testing"]},
            "lints": {"clippy": {"type_complexity": "allow"}},
        })

    def test_a_path_dependency_outside_the_api_source_is_refused(self):
        crate = Crate("vocab", "crates/nested/vocab", False)
        manifest = {"package": {"name": "vocab", "version": "0.1.0"},
                    "dependencies": {"sunrise-items": {"path": "../../sunrise-items"}}}
        with self.assertRaises(Refusal) as refused:
            api_source.normalise_manifest(manifest, self.WORKSPACE, crate, {"vocab": crate}, {}, {})
        self.assertEqual(refused.exception.code, "E_PATH_DEPENDENCY")

    def test_asking_for_a_feature_the_game_does_not_resolve_is_refused(self):
        crate = Crate("vocab", "crates/nested/vocab", False)
        for spec, resolved in (({"version": "1", "features": ["derive"]}, ({"default", "std"}, True)),
                               ("1", ({"std"}, False))):
            manifest = {"package": {"name": "vocab", "version": "0.1.0"}, "dependencies": {"serde": spec}}
            with self.subTest(spec), self.assertRaises(Refusal) as refused:
                api_source.normalise_manifest(manifest, self.WORKSPACE, crate, {"vocab": crate}, {},
                                              {(None, "serde"): resolved})
            self.assertEqual(refused.exception.code, "E_FEATURES")


class RequestedFeatureTests(unittest.TestCase):
    REGISTRY = "registry+https://github.com/rust-lang/crates.io-index"
    CRATE = Crate("core", "crates/core", False)
    ARM = 'cfg(target_arch = "aarch64")'
    X86 = 'cfg(target_arch = "x86_64")'

    def metadata(self, *edges, optional=False):
        """Cargo's target-filtered graph. Inactive registry packages can still be in `packages`."""
        packages = [package("core")]
        dependencies = []
        for alias, name, platform in edges:
            if name not in {item["name"] for item in packages}:
                dependency = package(name, source=self.REGISTRY)
                dependency["features"] = {"default": [], "std": []}
                packages.append(dependency)
            dependencies.append({"name": alias.replace("-", "_"), "pkg": name,
                                 "dep_kinds": [{"kind": None, "target": platform}]})
            packages[0]["dependencies"].append({"name": name, "rename": alias if alias != name else None,
                                                "target": platform, "kind": None, "optional": optional})
        return {"packages": packages, "resolve": {"nodes": [{"id": "core", "deps": dependencies}]}}

    def written(self, manifest, metadata, game):
        manifest = {"package": {"name": "core", "version": "0.1.0"}, **manifest}
        resolved = api_source.requested_features(metadata, self.CRATE, manifest, game)
        return tomllib.loads(api_source.normalise_manifest(manifest, {}, self.CRATE, {"core": self.CRATE},
                                                           manifest.get("features", {}), resolved))

    def test_inactive_architecture_is_not_required_or_rewritten(self):
        manifest = {"target": {
            self.ARM: {"dependencies": {"math-kernel": {"package": "portable-math", "version": "0.1"}}},
            self.X86: {"dependencies": {"math-kernel": {"package": "x86-math", "version": "0.1",
                                                        "features": ["sse2"]}, "safe_arch": "=1.2.0"}},
        }}
        # The same renamed alias can resolve a different package on another architecture. Cargo formats cfg
        # whitespace; it has already decided that the x86 table is inactive for this ARM build.
        data = self.metadata(("math-kernel", "portable-math", 'cfg(target_arch="aarch64")'))
        data["packages"].extend([package("x86-math", source=self.REGISTRY),
                                 package("safe_arch", source=self.REGISTRY)])
        data["packages"][0]["dependencies"].extend([
            {"name": name, "rename": alias, "target": self.X86, "kind": None, "optional": False}
            for alias, name in (("math-kernel", "x86-math"), (None, "safe_arch"))])
        written = self.written(manifest, data, {"core 0.1.0": set(), "portable-math 0.1.0": {"default", "std"}})
        self.assertEqual(written["target"][self.ARM]["dependencies"]["math-kernel"],
                         {"package": "portable-math", "version": "0.1", "features": ["std"]})
        self.assertEqual(written["target"][self.X86], manifest["target"][self.X86])

    def test_unconditional_renamed_dependency_uses_its_package_features(self):
        manifest = {"dependencies": {"json-codec": {"package": "serde_json", "version": "0.1",
                                                     "default-features": False}}}
        written = self.written(manifest, self.metadata(("json-codec", "serde_json", None)),
                               {"core 0.1.0": set(), "serde_json 0.1.0": {"std", "raw_value"}})
        self.assertEqual(written["dependencies"]["json-codec"],
                         {"package": "serde_json", "version": "0.1", "default-features": False,
                          "features": ["raw_value", "std"]})

    def test_package_and_exported_library_names_can_differ(self):
        manifest = {"dependencies": {"md-5": {"version": "0.1", "default-features": False}}}
        data = self.metadata(("md-5", "md-5", None))
        data["resolve"]["nodes"][0]["deps"][0]["name"] = "md5"
        written = self.written(manifest, data, {"core 0.1.0": set(), "md-5 0.1.0": {"std"}})
        self.assertEqual(written["dependencies"]["md-5"],
                         {"version": "0.1", "default-features": False, "features": ["std"]})

    def test_only_activated_optional_edges_receive_game_features(self):
        spec = {"package": "optional-codec", "version": "0.1", "optional": True, "default-features": False}
        cases = [({}, set(), False), ({"weak": ["codec?/std"]}, {"weak"}, False),
                 ({"codec": []}, {"codec"}, False), ({}, {"codec"}, True),
                 ({"enabled": ["dep:codec"]}, {"enabled"}, True),
                 ({"strong": ["codec/std"]}, {"strong"}, True),
                 ({"default": ["enabled"], "enabled": ["dep:codec"]}, {"default"}, True)]
        for definitions, enabled, active in cases:
            with self.subTest(definitions=definitions, enabled=enabled):
                manifest = {"dependencies": {"codec": spec}, "features": definitions}
                # Workspace metadata can include an edge enabled by another package. Presence there, or even
                # somewhere else in the game, does not enable this crate's optional edge.
                data = self.metadata(("codec", "optional-codec", None), optional=True)
                written = self.written(manifest, data,
                                       {"core 0.1.0": enabled, "optional-codec 0.1.0": {"std"}})
                self.assertEqual(written["dependencies"]["codec"],
                                 {**spec, "features": ["std"]} if active else spec)

    def test_missing_active_registry_dependencies_still_refuse(self):
        for platform, optional in ((None, False), (self.ARM, False), (None, True)):
            with self.subTest(platform=platform, optional=optional):
                dependencies = {"codec": {"package": "real-codec", "version": "0.1", "optional": optional}}
                manifest = ({"dependencies": dependencies} if platform is None
                            else {"target": {platform: {"dependencies": dependencies}}})
                manifest["features"] = {"enabled": ["dep:codec"]} if optional else {}
                with self.assertRaises(Refusal) as refused:
                    self.written(manifest, self.metadata(("codec", "real-codec", platform), optional=optional),
                                 {"core 0.1.0": {"enabled"} if optional else set()})
                self.assertEqual(refused.exception.code, "E_FEATURES")
                self.assertIn("real-codec 0.1.0", str(refused.exception))

    def test_missing_unconditional_metadata_edge_is_not_treated_as_inactive(self):
        manifest = {"dependencies": {"codec": "0.1"}}
        data = self.metadata(("codec", "codec", None))
        data["resolve"]["nodes"][0]["deps"].clear()
        with self.assertRaises(Refusal) as refused:
            self.written(manifest, data, {"core 0.1.0": set(), "codec 0.1.0": {"std"}})
        self.assertEqual(refused.exception.code, "E_FEATURES")

    def test_host_proc_macro_features_are_not_requested_from_the_target_tree(self):
        manifest = {"dependencies": {"derive-codec": "0.1"}}
        data = self.metadata(("derive-codec", "derive-codec", None))
        data["packages"][1]["targets"][0]["kind"] = ["proc-macro"]
        written = self.written(manifest, data, {"core 0.1.0": set()})
        self.assertEqual(written["dependencies"], manifest["dependencies"])

    def test_cargo_filters_target_predicates_for_metadata(self):
        with (patch.object(api_source.prebuilt_game, "rustc_host", return_value="aarch64-apple-darwin"),
              patch.object(api_source, "run", return_value='{"packages": []}') as run):
            self.assertEqual(api_source.target_metadata(Path("/api"), {"TEST": "yes"}, offline=True),
                             {"packages": []})
        run.assert_called_once_with(["cargo", "metadata", "--locked", "--offline", "--format-version", "1",
                                     "--filter-platform", "aarch64-apple-darwin"],
                                    cwd=Path("/api"), env={"TEST": "yes"})


class RootTests(unittest.TestCase):
    """`check_root`: each API root is judged by a consumer that depends on it."""
    REGISTRY = "registry+https://github.com/rust-lang/crates.io-index"

    def metadata(self):
        return {"packages": [package("sunrise-mod-api"), package("sunrise-wasmtime"),
                             package("serde", source=self.REGISTRY), package("wasmtime", source=self.REGISTRY)],
                "resolve": {"nodes": [node("sunrise-mod-api", ("serde", None)),
                                      node("sunrise-wasmtime", ("wasmtime", None)), node("serde"), node("wasmtime")]}}

    GAME = "\n".join(["0sunrise-mod-host v0.1.0|", "1serde v0.1.0|derive", "1wasmtime v0.1.0|cranelift,std"])
    STARTER = "\n".join(["0mod-starter v0.1.0|", "1serde v0.1.0|derive"])
    NATIVE = "\n".join(["0mod-starter-wasmtime v0.1.0|", "1serde v0.1.0|derive", "1wasmtime v0.1.0|cranelift,std"])

    def check(self, root, consumer):
        return api_source.check_root(self.metadata(), self.metadata(), root, self.GAME, consumer, "consumer")

    def test_an_api_only_starter_is_judged_on_the_api_closure_alone(self):
        self.assertEqual(self.check("sunrise-mod-api", self.STARTER), 2)

    def test_the_wasmtime_root_needs_a_consumer_that_depends_on_it(self):
        self.assertEqual(self.check("sunrise-wasmtime", self.NATIVE), 2)
        with self.assertRaises(Refusal) as refused:
            self.check("sunrise-wasmtime", self.STARTER)
        self.assertEqual(refused.exception.code, "E_FEATURES")
        self.assertIn("wasmtime 0.1.0: game ['cranelift', 'std'], ours None", str(refused.exception))
        with self.assertRaises(Refusal) as refused:
            self.check("sunrise-wasmtime", self.NATIVE.replace("cranelift,std", "std"))
        self.assertEqual(refused.exception.code, "E_FEATURES")


    def test_a_root_the_games_other_crates_widen_may_resolve_fewer_packages_but_never_more(self):
        wider = self.metadata()
        wider["packages"].append(package("subtle", source=self.REGISTRY))
        wider["resolve"]["nodes"].append(node("subtle"))
        next(n for n in wider["resolve"]["nodes"] if n["id"] == "wasmtime")["deps"].append(
            {"pkg": "subtle", "dep_kinds": [{"kind": None}]})
        game = "\n".join([self.GAME, "1subtle v0.1.0|"])
        args = ("sunrise-wasmtime", game, self.NATIVE, "consumer")
        self.assertEqual(api_source.check_root(wider, self.metadata(), *args, exact=False), 2)
        # Wasmtime itself is exact: a feature the game lacks fails, and so does one the game has that it misses.
        for consumer in (self.NATIVE.replace("cranelift,std", "cranelift,std,wat"),
                         self.NATIVE.replace("cranelift,std", "std")):
            with self.subTest(consumer), self.assertRaises(Refusal) as refused:
                api_source.check_root(wider, self.metadata(), "sunrise-wasmtime", game, consumer, "consumer",
                                      exact=False)
            self.assertEqual(refused.exception.code, "E_FEATURES")
        # A transitive crate may have fewer features than the game's, never more.
        loose = self.NATIVE + "\n1subtle v0.1.0|"
        self.assertEqual(api_source.check_root(wider, self.metadata(), "sunrise-wasmtime", game, loose, "consumer",
                                               exact=False), 2)
        with self.assertRaises(Refusal) as refused:
            api_source.check_root(wider, self.metadata(), "sunrise-wasmtime", game,
                                  self.NATIVE + "\n1subtle v0.1.0|extra", "consumer", exact=False)
        self.assertEqual(refused.exception.code, "E_FEATURES")
        with self.assertRaises(Refusal) as refused:
            api_source.check_root(wider, self.metadata(), *args)
        self.assertEqual(refused.exception.code, "E_CLOSURE")
        with self.assertRaises(Refusal) as refused:
            api_source.check_root(self.metadata(), wider, *args, exact=False)
        self.assertEqual(refused.exception.code, "E_CLOSURE")


class FeatureTests(unittest.TestCase):
    TREE = "\n".join([
        "0sunrise-mod-host v0.1.0 (/w/crates/sunrise-mod-host)|",
        "1bytemuck v1.25.2|bytemuck_derive,derive,extern_crate_alloc",
        "2bytemuck_derive v1.12.1 (proc-macro)|",
        "3proc-macro2 v1.0.107|default,proc-macro",
        "4unicode-ident v1.0.26|",
        "3syn v2.0.119|full",
        "1serde_json v1.0.151|default,raw_value,std",
        "2memchr v2.8.3|alloc,std",
        "1sunrise-mod-api-macros v0.1.0 (proc-macro) (/w/crates/sunrise-mod-api-macros)|",
        "2serde_json v1.0.151|default,std",
        "1memchr v2.8.3|alloc,default,std (*)",
        "1log v0.4.34|",
    ])

    def test_target_features_leave_proc_macros_and_their_trees_out(self):
        self.assertEqual(api_source.target_features(self.TREE), {
            "sunrise-mod-host 0.1.0": set(),
            "bytemuck 1.25.2": {"bytemuck_derive", "derive", "extern_crate_alloc"},
            "serde_json 1.0.151": {"default", "raw_value", "std"},
            "memchr 2.8.3": {"alloc", "default", "std"},
            "log 0.4.34": set(),
        })

    def test_a_package_with_other_features_is_named(self):
        game = {"bytemuck 1.25.2": {"derive", "extern_crate_alloc"}, "log 0.4.34": set(), "zmij 1.0.23": set(),
                "memchr 2.8.3": {"alloc", "default", "std"}, "itoa 1.0.18": set()}
        ours = {"bytemuck 1.25.2": {"derive"}, "log 0.4.34": set(), "memchr 2.8.3": {"alloc", "std"}}
        self.assertEqual(api_source.feature_disagreements(
            game, ours, {"bytemuck 1.25.2", "log 0.4.34", "memchr 2.8.3", "itoa 1.0.18"}),
            ["bytemuck 1.25.2: game ['derive', 'extern_crate_alloc'], ours ['derive']",
             "itoa 1.0.18: game [], ours None"])


class CollectTests(unittest.TestCase):
    """`collect` over fake cargo messages for the three builds, on files in a temporary workspace."""

    def setUp(self):
        self.folder = tempfile.TemporaryDirectory()
        self.root = Path(self.folder.name)
        (self.root / "crates/api/src").mkdir(parents=True)
        (self.root / "crates/api/src/lib.rs").write_text("include!(concat!(env!(\"OUT_DIR\"), \"/gen.rs\"));\n")
        (self.root / "target").mkdir()
        self.crates = [Crate("api", "crates/api", True)]

    def tearDown(self):
        self.folder.cleanup()

    def messages(self, build, generated, reads=("crates/api/src/lib.rs",), cfgs=()):
        out = self.root / f"target/out-{build}"
        out.mkdir()
        (out / "gen.rs").write_text(generated)
        info = self.root / f"target/api-{build}0.d"
        info.write_text(f"/t/libapi.rmeta: {' '.join(reads)} {out}/gen.rs\n")
        package_id = "path+file:///w/crates/api#0.1.0"
        return [
            {"reason": "build-script-executed", "package_id": package_id, "out_dir": str(out), "cfgs": list(cfgs),
             "env": [], "linked_libs": [], "linked_paths": []},
            {"reason": "compiler-artifact", "package_id": package_id,
             "manifest_path": str(self.root / "crates/api/Cargo.toml"), "target": {"kind": ["lib"], "name": "api"},
             "filenames": [str(self.root / f"target/libapi-{build}0.rmeta")]},
        ]

    def collect(self, *builds):
        with (patch.object(api_source, "build_messages", side_effect=list(builds)),
              patch.dict(os.environ, {"CARGO_TARGET_DIR": str(self.root / "target")})):
            return api_source.collect(self.crates, {"api": {}}, self.root)

    def refusal(self, *builds):
        with self.assertRaises(Refusal) as refused:
            self.collect(*builds)
        return refused.exception.code

    def test_generated_files_the_same_for_every_build_come_with_the_crate(self):
        files = self.collect(*(self.messages(build, "pub struct Hook;\n") for build in "abc"))
        self.assertEqual(files, {"crates/api/generated/gen.rs": b"pub struct Hook;\n",
                                 "crates/api/src/lib.rs": (self.root / "crates/api/src/lib.rs").read_bytes()})

    def test_generated_output_that_differs_between_builds_is_refused(self):
        self.assertEqual(self.refusal(self.messages("a", "pub struct Hook;\n"),
                                      self.messages("b", "pub struct Hook(u32);\n"),
                                      self.messages("c", "pub struct Hook;\n")), "E_TARGET_DEPENDENT")

    def test_a_file_read_outside_the_crate_is_refused(self):
        outside = self.messages("a", "", reads=("crates/api/src/lib.rs", "crates/sunrise-client/src/game.rs"))
        self.assertEqual(self.refusal(outside), "E_SOURCE_OUTSIDE_CRATE")

    def test_a_folder_or_other_file_in_the_build_scripts_output_is_refused(self):
        for entry, folder in (("nested", True), ("hooks.json", False)):
            with self.subTest(entry):
                messages = self.messages(entry.split(".")[0], "")
                out = Path(messages[0]["out_dir"])
                if folder:
                    (out / entry).mkdir()
                else:
                    (out / entry).write_text("{}")
                self.assertEqual(self.refusal(messages), "E_GENERATED_ENTRY")

    def test_a_build_script_that_sets_cfgs_is_refused(self):
        self.assertEqual(self.refusal(self.messages("a", "", cfgs=("generated_hooks",))), "E_BUILD_SCRIPT")


class PackingTests(unittest.TestCase):
    CRATES = [Crate("sunrise-mod-api", "crates/sunrise-mod-api", True), Crate("core", "crates/core", False)]
    TRACKED = {"crates/sunrise-mod-api/src/lib.rs", "crates/core/src/lib.rs"}

    def files(self):
        return {
            "Cargo.toml": b"[workspace]\n",
            "Cargo.lock": b"version = 4\n",
            "crates/sunrise-mod-api/Cargo.toml": b"",
            "crates/sunrise-mod-api/build.rs": b"fn main() {}\n",
            "crates/sunrise-mod-api/generated/hooks.rs": b'#[path = "{sunrise-api-source-root}/x.rs"] mod x;\n',
            "crates/sunrise-mod-api/src/lib.rs": b"",
            "crates/core/Cargo.toml": b"",
            "crates/core/src/lib.rs": b"",
        }

    def check(self, files):
        return api_source.check_files(files, self.CRATES, self.TRACKED, ["/home/dev/sunrise"])

    def refusal(self, files):
        with self.assertRaises(Refusal) as refused:
            self.check(files)
        return refused.exception.code

    def test_the_list_and_index_do_not_depend_on_the_order_files_were_found(self):
        files = self.files()
        reversed_files = dict(reversed(list(files.items())))
        self.assertEqual(self.check(files), self.check(reversed_files))
        self.assertEqual(self.check(files)[:3], ["Cargo.lock", "Cargo.toml", "crates/core/Cargo.toml"])
        self.assertEqual(api_source.packing_index(files), api_source.packing_index(reversed_files))

    def test_a_file_naming_a_local_folder_is_refused(self):
        files = self.files()
        files["crates/sunrise-mod-api/generated/hooks.rs"] = b'#[path = "/home/dev/sunrise/crates/x.rs"] mod x;\n'
        self.assertEqual(self.refusal(files), "E_ABSOLUTE_PATH")

    def test_a_folder_that_only_starts_like_a_local_one_is_no_path_to_it(self):
        files = self.files()
        files["crates/core/src/lib.rs"] = b'// see /home/dev/sunrise-site and /home/dev/sunrises\n'
        self.assertEqual(len(self.check(files)), len(files))
        for named in (b'"/home/dev/sunrise"', b"/home/dev/sunrise/crates/x.rs", b"/home/dev/sunrise\n"):
            with self.subTest(named):
                self.assertEqual(self.refusal({**files, "crates/core/src/lib.rs": named}), "E_ABSOLUTE_PATH")

    def test_stray_files_are_refused(self):
        for stray in (".local/minecraft/client.jar", "crates/core/src/untracked.rs", "crates/core/build.rs",
                      "crates/core/generated/hooks.rs", "crates/sunrise-mod-api/generated/sub/hooks.rs",
                      "crates/other/src/lib.rs", "/etc/passwd", "crates/core/../../secret"):
            with self.subTest(stray):
                self.assertIn(self.refusal({**self.files(), stray: b""}), ("E_STRAY_FILE", "E_PATH"))


class ScratchTests(unittest.TestCase):
    def test_a_folder_check_did_not_make_is_never_emptied(self):
        with tempfile.TemporaryDirectory() as folder:
            (Path(folder) / "notes.txt").write_text("keep")
            with self.assertRaises(Refusal) as refused:
                api_source.check(Path(folder) / "api", Path(folder))
            self.assertEqual(refused.exception.code, "E_SCRATCH")
            self.assertEqual((Path(folder) / "notes.txt").read_text(), "keep")


class RelocationTests(unittest.TestCase):
    def test_both_ways_the_build_script_names_the_checkout_become_the_token(self):
        crate = Crate("vocab", "crates/vocab", True)
        forms = api_source.root_forms([Path("/w")], crate)
        code = '#[path = "/w/crates/vocab/../../crates/vocab/src/a.rs"] mod a; include!("/w/crates/vocab/src/b.rs");'
        self.assertEqual(api_source.relocate(code, forms),
                         '#[path = "{sunrise-api-source-root}/crates/vocab/src/a.rs"] mod a; '
                         'include!("{sunrise-api-source-root}/crates/vocab/src/b.rs");')

    def test_dep_info_keeps_escaped_spaces(self):
        with tempfile.TemporaryDirectory() as folder:
            info = Path(folder) / "x.d"
            info.write_text("/t/libx.rmeta: src/lib.rs src/my\\ file.rs /o/gen.rs\n\nsrc/lib.rs:\n")
            self.assertEqual(api_source.dep_info(info), ["src/lib.rs", "src/my file.rs", "/o/gen.rs"])


class LockTests(unittest.TestCase):
    def test_a_version_the_game_does_not_pin_is_named(self):
        registry = "registry+https://github.com/rust-lang/crates.io-index"
        game = {"package": [{"name": "serde", "version": "1.0.228", "source": registry, "checksum": "a"},
                            {"name": "sunrise-core", "version": "0.1.0"}]}
        ours = {"package": [{"name": "serde", "version": "1.0.229", "source": registry, "checksum": "b"},
                            {"name": "sunrise-core", "version": "0.1.0"}]}
        self.assertEqual(api_source.lock_disagreements(ours, game), ["serde 1.0.229"])
        self.assertEqual(api_source.lock_disagreements(game, game), [])


if __name__ == "__main__":
    unittest.main()
