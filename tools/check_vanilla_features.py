#!/usr/bin/env python3
"""The vanilla build (`-p sunrise-game --no-default-features --features audio`) has no Extras feature or crate (#2510 Q-L)."""

import argparse
import json
import os
import re
import shlex
import subprocess
import sys
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
VANILLA = ["-p", "sunrise-game", "--no-default-features", "--features", "audio"]
EXTRAS_FEATURES = {"extension-points", "friends", "sunrise-extras"}
EXTRAS_CRATE = "sunrise-friends"
# The one manifest that may declare `sunrise-extras`.
SWITCH_OWNER = "sunrise-official"
# Rows a plan of the vanilla build must have, or it isn't one.
EXPECTED = ("sunrise-game", "sunrise-client")

# `--format '{p}|{f}'`: `name vX.Y.Z (path)|feature,feature`; a feature node: `name feature "x"`.
PACKAGE = re.compile(r'^([A-Za-z0-9_-]+) v[^|\s]*[^|]*\|(.*?)(?: \(\*\))?$')
FEATURE = re.compile(r'^([A-Za-z0-9_-]+) feature "([^"]+)"')
# `--prefix depth`: `<depth><name> vX.Y.Z ...`.
NODE = re.compile(r'^(\d+)([A-Za-z0-9_-]+) v\S+')


def cargo(root: Path, *args: str) -> str:
    command = shlex.split(os.environ.get("CARGO_COMMAND", "cargo"))
    return subprocess.run([*command, *args], cwd=root, capture_output=True, text=True, check=True).stdout


def feature_problems(tree: str, below: set[str]) -> list[str]:
    """Extras features on engine and platform crates in `cargo tree -e features --prefix none --format '{p}|{f}'`."""
    found, seen = set(), set()
    for line in tree.splitlines():
        if m := PACKAGE.match(line):
            enabled = {f for f in m[2].split(",") if f}
        elif m := FEATURE.match(line):
            enabled = {m[2]}
        else:
            continue
        seen.add(m[1])
        if m[1] in below:
            found.update(f'{m[1]} has feature "{f}"' for f in enabled & EXTRAS_FEATURES)
    found.update(f"{name} is missing from the plan" for name in EXPECTED if name not in seen)
    return sorted(found)


def crate_problems(tree: str, sunrise: set[str], allowed: set[str]) -> list[str]:
    """Sunrise-layer crates in `cargo tree -e normal --prefix depth --no-dedupe`, but those under a layers.json exception."""
    found, path = set(), []
    for line in tree.splitlines():
        if not (m := NODE.match(line)):
            continue
        depth, name = int(m[1]), m[2]
        del path[depth:]
        excepted = any(crate in allowed for crate in path) or name in allowed
        if name == EXTRAS_CRATE or (name in sunrise and not excepted):
            found.add(f"{name} is in the graph")
        path.append(name)
    return sorted(found)


# Temporary gap, not a permanent exception: telemetry leaves the engine and this allowance goes in #2749.
def allowed_targets(layers: dict) -> set[str]:
    """Sunrise crates a lower layer may still use (layers.json `allowed`, which only shrinks); never sunrise-friends."""
    out = set()
    for edge in layers.get("allowed", []):
        source, _, target = (edge["edge"] if isinstance(edge, dict) else edge).partition(" -> ")
        if layers["crates"].get(target) == "sunrise" and layers["crates"].get(source) != "sunrise":
            out.add(target)
    return out - {EXTRAS_CRATE}


def manifest_problems(root: Path) -> list[str]:
    """Every tracked Cargo.toml but sunrise-official's that declares a `sunrise-extras` feature."""
    listed = subprocess.run(["git", "-C", str(root), "ls-files", "-z", "--", "*Cargo.toml"],
                            capture_output=True, check=True).stdout.decode().split("\0")
    found = []
    for path in filter(None, listed):
        manifest = tomllib.loads((root / path).read_text(encoding="utf-8"))
        name = manifest.get("package", {}).get("name")
        if "sunrise-extras" in manifest.get("features", {}) and name != SWITCH_OWNER:
            found.append(f"{path} declares the sunrise-extras feature")
    return found


def problems(root: Path) -> list[str]:
    layers = json.loads((root / "tools/layers.json").read_text(encoding="utf-8"))
    below = {name for name, layer in layers["crates"].items() if layer in ("engine", "platform")}
    sunrise = {name for name, layer in layers["crates"].items() if layer == "sunrise"}
    base = ["tree", "--locked", *VANILLA, "--prefix", "none"]
    features = cargo(root, *base, "-e", "features", "--format", "{p}|{f}")
    normal = cargo(root, "tree", "--locked", *VANILLA, "-e", "normal", "--prefix", "depth", "--no-dedupe")
    return (feature_problems(features, below) + crate_problems(normal, sunrise, allowed_targets(layers))
            + manifest_problems(root))


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, default=ROOT, help="the workspace to check (default: this checkout)")
    found = problems(parser.parse_args().root)
    for problem in found:
        print(f"vanilla build ({' '.join(VANILLA)}): {problem}", file=sys.stderr)
    if not found:
        print("vanilla build: no extension points, no friends, no Sunrise crate beyond layers.json's exceptions, "
              "no sunrise-extras outside sunrise-official")
    return 1 if found else 0


if __name__ == "__main__":
    sys.exit(main())
