"""Exercise the verify-static guard with synthetic Cargo unit graphs, without running Cargo."""

import io
import json
import unittest
from contextlib import redirect_stderr, redirect_stdout
from unittest.mock import patch

from tools.ci import check_extras_features

ROOT = check_extras_features.ROOT
RELEASE = {"name": "release", "opt_level": "3"}
HOST = {"name": "release", "opt_level": "0"}


def unit(package, features=(), dependencies=(), *, profile=RELEASE, source="registry+https://example/index#"):
    """One unit of `cargo build --unit-graph`; dependencies are indices into the same list."""
    name = package.split("@")[0]
    return {"pkg_id": f"{source}{package}", "target": {"name": name, "kind": ["lib"]}, "mode": "build",
            "features": list(features), "profile": profile, "platform": None,
            "dependencies": [{"index": index, "extern_crate_name": f"dep{index}"} for index in dependencies]}


def graph(*units):
    return json.dumps({"version": 1, "units": list(units), "roots": [len(units) - 1]})


def ws_unit(name, features=(), dependencies=(), *, short_id=False):
    pkg = "0.1.0" if short_id else f"{name}@0.1.0"
    u = unit(pkg, features, dependencies, source=f"path+file://{ROOT}/crates/{name}#")
    u["target"]["name"] = name
    return u


def extras_units(on):
    features = ["sunrise-extras"] if on else []
    return [
        ws_unit("sunrise-client-render", features, short_id=True),
        ws_unit("sunrise-client-gameplay", features),
        ws_unit("sunrise-client", features),
    ]


GAME = ws_unit("sunrise-game")
SHARED_DEFAULT = graph(unit("shared@1.0.0", ["std"]), *extras_units(True), GAME)
SHARED_VANILLA = graph(unit("shared@1.0.0", ["std"]), *extras_units(False), GAME)


class ExtrasFeaturesTest(unittest.TestCase):
    def check(self, default, extras_off, *, root="sunrise-game"):
        def cargo(command, **kwargs):
            self.assertEqual(kwargs["env"]["RUSTC_BOOTSTRAP"], "1")
            self.assertIn("--locked", command)
            self.assertIn("--offline", command)
            is_vanilla = "--no-default-features" in command
            if command[command.index("-p") + 1] != root:
                return SHARED_VANILLA if is_vanilla else SHARED_DEFAULT
            return extras_off if is_vanilla else default

        with patch.object(check_extras_features.subprocess, "check_output", side_effect=cargo) as calls, \
                redirect_stdout(io.StringIO()), redirect_stderr(io.StringIO()) as errors:
            status = check_extras_features.main()
        self.assertEqual(calls.call_count, 4)
        return status, errors.getvalue()

    def test_the_same_dependency_units_pass(self):
        self.assertEqual(self.check(SHARED_DEFAULT, SHARED_VANILLA), (0, ""))

    def test_a_dependency_with_other_features_fails_in_either_build_and_names_it(self):
        for root in ("sunrise-game", "sunrise-mod-host"):
            with self.subTest(root=root):
                status, diagnostic = self.check(
                    SHARED_DEFAULT,
                    graph(unit("shared@1.0.0"), *extras_units(False), GAME),
                    root=root,
                )
                self.assertEqual(status, 1)
                self.assertIn(f"{root}: shared 1.0.0 shared (build; features []) differs", diagnostic)
                self.assertIn(f"{root}: 1 dependency units would be compiled again", diagnostic)

    def test_units_only_one_build_has_pass(self):
        default = graph(unit("shared@1.0.0", ["std"]), unit("friends-only@0.1.0"), *extras_units(True), GAME)
        self.assertEqual(self.check(default, SHARED_VANILLA), (0, ""))

    def test_workspace_crates_own_the_extras_features_and_may_differ(self):
        default = graph(
            unit("shared@1.0.0", ["std"]),
            ws_unit("sunrise-client-lib", ["audio", "controllers"], [0]),
            *extras_units(True),
            GAME,
        )
        extras_off = graph(
            unit("shared@1.0.0", ["std"]),
            ws_unit("sunrise-client-lib", ["audio"], [0]),
            *extras_units(False),
            GAME,
        )
        self.assertEqual(self.check(default, extras_off), (0, ""))

    def test_a_path_package_outside_the_workspace_is_still_compared(self):
        for folder in ("/elsewhere", f"{ROOT}-sibling"):
            for package, tail in (("0.1.0", "outside"), ("outside@0.1.0", "other-dir")):
                with self.subTest(folder=folder, package=package):
                    source = f"path+file://{folder}/{tail}#"
                    default_unit = unit(package, ["std"], source=source)
                    default_unit["target"]["name"] = "outside"
                    vanilla_unit = unit(package, source=source)
                    vanilla_unit["target"]["name"] = "outside"
                    status, diagnostic = self.check(
                        graph(default_unit, *extras_units(True), GAME),
                        graph(vanilla_unit, *extras_units(False), GAME),
                    )
                    self.assertEqual(status, 1)
                    self.assertIn("outside 0.1.0 outside (build; features []) differs", diagnostic)

    def test_a_host_unit_does_not_stand_in_for_a_target_unit(self):
        # The vanilla target libc has the features only the default build's host libc had.
        default = graph(
            unit("libc@0.2.0", ["extra_traits", "std"]),
            unit("libc@0.2.0", ["std"], profile=HOST),
            *extras_units(True),
            GAME,
        )
        extras_off = graph(
            unit("libc@0.2.0", ["std"]),
            unit("libc@0.2.0", ["std"], profile=HOST),
            *extras_units(False),
            GAME,
        )
        status, diagnostic = self.check(default, extras_off)
        self.assertEqual(status, 1)
        self.assertIn("libc 0.2.0 libc (build; features [std]) differs", diagnostic)

    def test_a_unit_above_a_split_dependency_is_compiled_again_but_not_named(self):
        default = graph(
            unit("libc@0.2.0", ["extra_traits", "std"]),
            unit("above@1.0.0", ["std"], [0]),
            *extras_units(True),
            GAME,
        )
        extras_off = graph(
            unit("libc@0.2.0", ["std"]),
            unit("above@1.0.0", ["std"], [0]),
            *extras_units(False),
            GAME,
        )
        status, diagnostic = self.check(default, extras_off)
        self.assertEqual(status, 1)
        self.assertNotIn("above", diagnostic)
        self.assertIn("2 dependency units would be compiled again", diagnostic)

    def test_renderer_missing_sunrise_extras_in_default_build_fails(self):
        default = graph(
            unit("shared@1.0.0", ["std"]),
            ws_unit("sunrise-client-render", [], short_id=True),
            ws_unit("sunrise-client-gameplay", ["sunrise-extras"]),
            ws_unit("sunrise-client", ["sunrise-extras"]),
            GAME,
        )
        status, diagnostic = self.check(default, SHARED_VANILLA)
        self.assertEqual(status, 1)
        self.assertIn("sunrise-game: sunrise-client-render is missing sunrise-extras in the default build", diagnostic)

    def test_renderer_unifying_sunrise_extras_into_vanilla_build_fails(self):
        vanilla = graph(
            unit("shared@1.0.0", ["std"]),
            ws_unit("sunrise-client-render", ["sunrise-extras"], short_id=True),
            ws_unit("sunrise-client-gameplay", []),
            ws_unit("sunrise-client", []),
            GAME,
        )
        status, diagnostic = self.check(SHARED_DEFAULT, vanilla)
        self.assertEqual(status, 1)
        self.assertIn("sunrise-game: sunrise-client-render enables sunrise-extras in the vanilla build", diagnostic)


if __name__ == "__main__":
    unittest.main()
