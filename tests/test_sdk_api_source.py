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
        "targets": [{"kind": list(kinds), "name": name.replace("-", "_")}],
        "features": {},
        "dependencies": [{"name": dependency, "kind": kind, "path": path, "target": None,
                          "rename": None, "optional": False} for dependency, kind, path in dependencies],
    }


def node(name, *edges):
    return {"id": name, "deps": [{"name": dependency.replace("-", "_"), "pkg": dependency,
                                  "dep_kinds": [{"kind": kind, "target": None}]} for dependency, kind in edges]}


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
        written = api_source.normalise_manifest(self.API, self.WORKSPACE, crate, crates,
                                                {"testing": ["vocab/testing"]})
        self.assertEqual(tomllib.loads(written), {
            "package": {"name": "sunrise-mod-api", "version": "0.1.0", "edition": "2024", "license": "MIT",
                        "publish": False, "autobins": False, "autoexamples": False, "autotests": False,
                        "autobenches": False, "build": "build.rs"},
            "lib": {"name": "mod_api"},
            "dependencies": {"vocab": {"path": "../nested/vocab"},
                             "log": "0.4",
                             "glam": {"version": "0.30", "features": ["bytemuck", "serde"]},
                             "libm": {"version": "0.2", "default-features": False}},
            "target": {"cfg(unix)": {"dependencies": {"libc": "0.2"}}},
            "features": {"testing": ["vocab/testing"]},
            "lints": {"clippy": {"type_complexity": "allow"}},
        })

    def test_a_path_dependency_outside_the_api_source_is_refused(self):
        crate = Crate("vocab", "crates/nested/vocab", False)
        manifest = {"package": {"name": "vocab", "version": "0.1.0"},
                    "dependencies": {"sunrise-items": {"path": "../../sunrise-items"}}}
        with self.assertRaises(Refusal) as refused:
            api_source.normalise_manifest(manifest, self.WORKSPACE, crate, {"vocab": crate}, {})
        self.assertEqual(refused.exception.code, "E_PATH_DEPENDENCY")

class NativeFeatureTests(unittest.TestCase):
    REGISTRY = "registry+https://github.com/rust-lang/crates.io-index"
    CRATE = Crate("sunrise-mod-api", "crates/sunrise-mod-api", False)

    def data(self, platform=None):
        packages = [package(api_source.API, dependencies=[("wrapper", None, None)]),
                    package(api_source.WASMTIME, dependencies=[("wasmtime", None, None)])]
        definitions = {"wrapper": {"std": [], "simd": []}, "inner": {"default": [], "extra": ["dep:leaf"]},
                       "leaf": {}, "outside-closure": {}, "wasmtime": {"std": []},
                       "linux-only": {}, "windows-only": {}, "mac-only": {}, "ghost": {}}
        for name, features in definitions.items():
            item = package(name, source=self.REGISTRY)
            item["features"] = features
            packages.append(item)
        lines = ["0sunrise-mod-host v0.1.0|", "1sunrise-mod-api v0.1.0|", "2wrapper v0.1.0|std",
                 "3inner v0.1.0|default,extra", "4leaf v0.1.0|"]
        if platform:
            lines.append(f"3{platform} v0.1.0|")
        lines += ["1outside-closure v0.1.0|", "1sunrise-wasmtime v0.1.0|", "2wasmtime v0.1.0|std"]
        # Metadata may retain inactive dep_kinds. This deliberately stale edge must not override the actual tree.
        return {"packages": packages, "resolve": {"nodes": [node(api_source.API, ("wrapper", None)),
                node(api_source.WASMTIME, ("wasmtime", None)), node("wrapper", ("ghost", None))]}}, "\n".join(lines)

    def pins(self, metadata, tree, root=api_source.API):
        features, dependencies = api_source.target_build(tree, complete=True)
        return api_source.feature_unifiers(metadata, root, features, dependencies)

    def test_transitive_unions_are_pinned_only_inside_the_actual_api_closure(self):
        metadata, tree = self.data()
        pins = self.pins(metadata, tree)
        by_package = {spec["package"]: spec for spec in pins.values()}
        self.assertEqual(set(by_package), {"wrapper", "inner", "leaf"})
        self.assertEqual(by_package["inner"], {"package": "inner", "version": "=0.1.0",
                                               "default-features": True, "features": ["extra"]})
        self.assertEqual(by_package["leaf"], {"package": "leaf", "version": "=0.1.0", "default-features": False})
        self.assertNotIn("ghost", by_package)
        self.assertNotIn("outside-closure", by_package)
        self.assertNotIn("wasmtime", by_package)
        self.assertEqual(pins["wrapper"]["features"], ["std"])
        self.assertTrue(all(name.startswith("__sunrise_sdk_features_") for name in pins if name != "wrapper"))

    def test_one_portable_manifest_keeps_wasm_specs_and_native_targets_separate(self):
        original = {"package": {"name": api_source.API, "version": "0.1.0"},
                    "dependencies": {"wrapper": {"version": "0.1", "default-features": False}},
                    "target": {'cfg(target_arch = "wasm32")': {"dependencies": {"wasm-helper": "0.1"}}}}
        targets = {}
        expected = {"x86_64-unknown-linux-gnu": "linux-only", "x86_64-pc-windows-gnu": "windows-only",
                    "aarch64-apple-darwin": "mac-only"}
        for target, platform in expected.items():
            data, tree = self.data(platform)
            targets[target] = self.pins(data, tree)
        source = api_source.with_feature_unifiers(original, targets)
        written = api_source.normalise_manifest(source, {}, self.CRATE, {api_source.API: self.CRATE}, {})
        parsed = tomllib.loads(written)
        self.assertEqual(parsed["dependencies"], original["dependencies"])
        self.assertEqual(parsed["target"]['cfg(target_arch = "wasm32")'], original["target"]['cfg(target_arch = "wasm32")'])
        for target, platform in expected.items():
            names = {spec["package"] for spec in parsed["target"][target]["dependencies"].values()}
            self.assertEqual(names, {"wrapper", "inner", "leaf", platform})
        self.assertEqual(set(original["target"]), {'cfg(target_arch = "wasm32")'})

    def test_versions_and_renames_cannot_capture_another_package_pin(self):
        root = package(api_source.API, dependencies=[("codec-bundle", None, None), ("codec-bundle", None, None)])
        root["dependencies"][1]["rename"] = "legacy"
        versions = []
        for version, feature in (("1.0.0", "old"), ("2.0.0", "modern")):
            item = package("codec-bundle", source=self.REGISTRY)
            item.update(id="registry+codec@" + version, version=version, features={feature: []})
            item["targets"][0].update(name="codec_core", kind=["rlib"])
            versions.append(item)
        original = {"package": {"name": api_source.API, "version": "0.1.0"},
                    "dependencies": {"codec-bundle": "2", "legacy": {"package": "codec-bundle", "version": "1"}}}
        first = None
        for order in (versions, list(reversed(versions))):
            edges = [{"name": "legacy" if item["version"] == "1.0.0" else "codec_core", "pkg": item["id"],
                      "dep_kinds": [{"kind": None, "target": None}]} for item in order]
            data = {"packages": [root, *order], "resolve": {"nodes": [{"id": root["id"], "deps": edges}]}}
            tree = "\n".join(["0sunrise-mod-api v0.1.0|", *[f"1codec-bundle v{item['version']}|{next(iter(item['features']))}" for item in order]])
            pins = self.pins(data, tree)
            self.assertEqual({spec["version"]: spec["features"] for spec in pins.values()},
                             {"=1.0.0": ["old"], "=2.0.0": ["modern"]})
            source = api_source.with_feature_unifiers(original, {"native-target": pins})
            self.assertEqual(source["dependencies"], original["dependencies"])
            self.assertEqual(source["target"]["native-target"]["dependencies"]["codec-bundle"]["features"], ["modern"])
            self.assertEqual(source["target"]["native-target"]["dependencies"]["legacy"]["features"], ["old"])
            self.assertEqual(pins, first if first is not None else pins)
            first = pins

    def test_missing_active_packages_unknown_features_and_ambiguous_sources_refuse(self):
        data, tree = self.data()
        data["packages"] = [item for item in data["packages"] if item["name"] != "leaf"]
        with self.assertRaises(Refusal) as refused:
            self.pins(data, tree)
        self.assertEqual(refused.exception.code, "E_FEATURES")
        data, tree = self.data()
        with self.assertRaises(Refusal) as refused:
            self.pins(data, tree.replace("default,extra", "default,extra,unknown"))
        self.assertEqual(refused.exception.code, "E_FEATURES")
        data, tree = self.data()
        duplicate = package("inner", source="git+https://example.invalid/inner#012345")
        duplicate["id"] = "other-inner"
        data["packages"].append(duplicate)
        with self.assertRaises(Refusal) as refused:
            self.pins(data, tree)
        self.assertIn("ambiguous", str(refused.exception))

    def test_missing_or_ambiguous_direct_extern_identity_refuses(self):
        data, tree = self.data()
        root_node = next(item for item in data["resolve"]["nodes"] if item["id"] == api_source.API)
        root_node["deps"].clear()
        with self.assertRaises(Refusal) as refused:
            self.pins(data, tree)
        self.assertEqual(refused.exception.code, "E_FEATURES")
        data, tree = self.data()
        root_node = next(item for item in data["resolve"]["nodes"] if item["id"] == api_source.API)
        root_node["deps"].append({"name": "another_wrapper", "pkg": "wrapper",
                                  "dep_kinds": [{"kind": None, "target": None}]})
        with self.assertRaises(Refusal) as refused:
            self.pins(data, tree)
        self.assertEqual(refused.exception.code, "E_FEATURES")

    def test_source_names_cannot_capture_generated_transitive_dependencies(self):
        original = {"dependencies": {"__sunrise_sdk_features_collision": "1"}}
        with self.assertRaises(Refusal) as refused:
            api_source.with_feature_unifiers(original, {"native-target": {"inner": {"version": "1"}}})
        self.assertEqual(refused.exception.code, "E_FEATURES")

    def test_fidelity_still_refuses_missing_transitive_packages_or_wider_closures(self):
        metadata, game = self.data()
        consumer = game.replace("3inner v0.1.0|default,extra", "3inner v0.1.0|default").replace("4leaf v0.1.0|\n", "")
        with self.assertRaises(Refusal) as refused:
            api_source.check_root(metadata, metadata, api_source.API, game, consumer, "starter")
        self.assertEqual(refused.exception.code, "E_CLOSURE")
        consumer = game.replace("1outside-closure v0.1.0|", "2outside-closure v0.1.0|\n1outside-closure v0.1.0|")
        with self.assertRaises(Refusal) as refused:
            api_source.check_root(metadata, metadata, api_source.API, game, consumer, "starter")
        self.assertEqual(refused.exception.code, "E_CLOSURE")

    def test_equal_package_closure_with_other_features_still_refuses(self):
        metadata, game = self.data()
        consumer = game.replace("3inner v0.1.0|default,extra", "3inner v0.1.0|default").replace("4leaf v0.1.0|", "2leaf v0.1.0|")
        with self.assertRaises(Refusal) as refused:
            api_source.check_root(metadata, metadata, api_source.API, game, consumer, "starter")
        self.assertEqual(refused.exception.code, "E_FEATURES")

    def test_complete_tree_excludes_host_macros_and_rejects_deduplication(self):
        tree = "\n".join(["0sunrise-mod-api v0.1.0|", "1macros v0.1.0 (proc-macro)|",
                           "2shared v0.1.0|host", "3host-only v0.1.0|", "1shared v0.1.0|target",
                           "2target-only v0.1.0|"])
        features, dependencies = api_source.target_build(tree, complete=True)
        self.assertEqual(features["shared 0.1.0"], {"target"})
        self.assertEqual(dependencies["shared 0.1.0"], {"target-only 0.1.0"})
        self.assertNotIn("host-only 0.1.0", features)
        with self.assertRaises(Refusal) as refused:
            api_source.target_build(tree + "\n1shared v0.1.0|target (*)", complete=True)
        self.assertEqual(refused.exception.code, "E_FEATURES")

    def test_native_output_is_independent_of_leg_iteration_and_metadata_order(self):
        legs = [("linux-target", False), ("linux-target", True), ("windows-target", False)]
        def resolved(folder, target, server):
            data, tree = self.data("linux-only" if target == "linux-target" else "windows-only")
            if server:
                data["packages"].reverse()
            return data, tree
        with patch.object(api_source, "game_leg", side_effect=resolved):
            first = api_source.native_feature_unifiers(Path("/w"), legs)
            second = api_source.native_feature_unifiers(Path("/w"), list(reversed(legs)))
        self.assertEqual(first, second)
        with patch.object(api_source, "game_leg", side_effect=lambda folder, target, server: self.data("linux-only" if server else "mac-only")):
            with self.assertRaises(Refusal) as refused:
                api_source.native_feature_unifiers(Path("/w"), legs[:2])
        self.assertEqual(refused.exception.code, "E_FEATURES")

    def test_release_legs_ignore_common_source_and_never_add_wasm(self):
        names = "\n".join(["api-x86_64-unknown-linux-gnu.tar.xz", "api-x86_64-unknown-linux-gnu-server.tar.xz",
                           "api-x86_64-pc-windows-gnu.tar.xz", "api-aarch64-apple-darwin.tar.xz",
                           "runtime-x86_64-unknown-linux-gnu.tar.xz", "api-source.tar.xz"])
        with patch.object(api_source, "run", return_value=names):
            self.assertEqual(api_source.sdk_legs(), [("aarch64-apple-darwin", False),
                ("x86_64-pc-windows-gnu", False), ("x86_64-unknown-linux-gnu", False), ("x86_64-unknown-linux-gnu", True)])

    def test_cargo_metadata_and_tree_use_the_exact_native_target_without_building(self):
        def run(command, **kwargs):
            if command[:2] == ["rustc", "--print"]:
                return 'target_os="windows"\n'
            return '{"packages": []}' if command[1] == "metadata" else "tree"
        with patch.object(api_source, "run", side_effect=run) as called:
            self.assertEqual(api_source.game_leg(Path("/missing"), "x86_64-pc-windows-gnu", False), ({"packages": []}, "tree"))
        commands = [call.args[0] for call in called.call_args_list]
        self.assertEqual(commands[0], ["rustc", "--print", "cfg", "--target", "x86_64-pc-windows-gnu"])
        self.assertIn("--no-dedupe", commands[2])
        self.assertEqual(commands[2][-2:], ["--features", "system-allocator"])
        self.assertTrue(all("build" not in command and "check" not in command for command in commands))


class RootTests(unittest.TestCase):
    """`check_root`: each API root is judged by a consumer that depends on it."""
    REGISTRY = "registry+https://github.com/rust-lang/crates.io-index"

    def metadata(self):
        return {"packages": [package("sunrise-mod-api", dependencies=[("serde", None, None)]),
                             package("sunrise-wasmtime", dependencies=[("wasmtime", None, None)]),
                             package("serde", source=self.REGISTRY), package("wasmtime", source=self.REGISTRY)],
                "resolve": {"nodes": [node("sunrise-mod-api", ("serde", None)),
                                      node("sunrise-wasmtime", ("wasmtime", None)), node("serde"), node("wasmtime")]}}

    GAME = "\n".join(["0sunrise-mod-host v0.1.0|", "1sunrise-mod-api v0.1.0|", "2serde v0.1.0|derive",
                       "1sunrise-wasmtime v0.1.0|", "2wasmtime v0.1.0|cranelift,std"])
    STARTER = "\n".join(["0mod-starter v0.1.0|", "1sunrise-mod-api v0.1.0|", "2serde v0.1.0|derive"])
    NATIVE = "\n".join(["0mod-starter-wasmtime v0.1.0|", "1sunrise-mod-api v0.1.0|", "2serde v0.1.0|derive",
                         "1sunrise-wasmtime v0.1.0|", "2wasmtime v0.1.0|cranelift,std"])

    def check(self, root, consumer):
        return api_source.check_root(self.metadata(), self.metadata(), root, self.GAME, consumer, "consumer")

    def test_an_api_only_starter_is_judged_on_the_api_closure_alone(self):
        self.assertEqual(self.check("sunrise-mod-api", self.STARTER), 2)

    def test_the_wasmtime_root_needs_a_consumer_that_depends_on_it(self):
        self.assertEqual(self.check("sunrise-wasmtime", self.NATIVE), 2)
        with self.assertRaises(Refusal) as refused:
            self.check("sunrise-wasmtime", self.STARTER)
        self.assertEqual(refused.exception.code, "E_FEATURES")
        self.assertIn("does not resolve sunrise-wasmtime 0.1.0", str(refused.exception))
        with self.assertRaises(Refusal) as refused:
            self.check("sunrise-wasmtime", self.NATIVE.replace("cranelift,std", "std"))
        self.assertEqual(refused.exception.code, "E_FEATURES")


    def test_a_root_the_games_other_crates_widen_may_resolve_fewer_packages_but_never_more(self):
        wider = self.metadata()
        wider["packages"].append(package("subtle", source=self.REGISTRY))
        wider["resolve"]["nodes"].append(node("subtle"))
        next(n for n in wider["resolve"]["nodes"] if n["id"] == "wasmtime")["deps"].append(
            {"name": "subtle", "pkg": "subtle", "dep_kinds": [{"kind": None, "target": None}]})
        next(p for p in wider["packages"] if p["id"] == "wasmtime")["dependencies"].append(
            {"name": "subtle", "kind": None, "target": None, "rename": None, "optional": False})
        game = "\n".join([self.GAME, "3subtle v0.1.0|"])
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
            api_source.check_root(self.metadata(), wider, "sunrise-wasmtime", self.GAME,
                                  self.NATIVE + "\n3subtle v0.1.0|", "consumer", exact=False)
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
