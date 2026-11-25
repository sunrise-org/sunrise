#!/usr/bin/env python3
"""Build matrix and signed metadata for .github/workflows/server-tool.yml."""
import argparse
import datetime as dt
import json
import os
from pathlib import Path
import re
import sys


def matrix(game_build: Path, platforms: Path) -> dict:
    """Build the manager's update targets using the game workflow's build settings."""
    sys.path.insert(0, str(Path(__file__).with_name("vendor") / "pyyaml-6.0.3.zip"))
    import yaml

    jobs = yaml.load(game_build.read_text(), Loader=yaml.BaseLoader)["jobs"]
    legs = {}
    for leg in jobs["build"]["strategy"]["matrix"]["include"]:
        match = re.fullmatch(r"sunrise-([a-z0-9_-]+)\.(?:tar\.gz|zip)", leg.get("asset", ""))
        if match:
            legs[match[1]] = leg

    include = []
    targets = set()
    assets = set()
    for number, line in enumerate(platforms.read_text().splitlines(), 1):
        fields = line.split()
        if not fields:
            continue
        if len(fields) != 3:
            raise SystemExit(f"{platforms}:{number}: expected OS ARCH ASSET")
        system, arch, asset = fields
        extension = r"zip" if system == "windows" else r"tar\.gz"
        match = re.fullmatch(rf"server-tool-({re.escape(system)}-[a-z0-9_]+)\.{extension}", asset)
        if not match:
            raise SystemExit(f"{platforms}:{number}: invalid {system} asset: {asset}")
        if (system, arch) in targets or asset in assets:
            raise SystemExit(f"{platforms}:{number}: duplicate platform or asset: {line}")
        targets.add((system, arch))
        assets.add(asset)
        platform = match[1]
        if platform not in legs:
            raise SystemExit(f"{asset}: no matching build leg in {game_build}")
        leg = legs[platform]
        target = leg.get("target", "")
        # The shared main-stack startup check fails under Rosetta; build and start the small tool on native Intel.
        runner = jobs["macos-x64-intel"]["runs-on"] if target == "x86_64-apple-darwin" else leg["os"]
        include.append({"name": platform, "os": runner, "target": target, "rustflags": leg.get("rustflags", ""),
                        "asset": asset, "exe": ".exe" if system == "windows" else ""})
    if not include:
        raise SystemExit(f"{platforms}: no server-tool platforms")
    return {"include": include}


def version_info(document: object, commit: str, commit_time: int) -> dict:
    """Refuse an archive from the wrong checkout or metadata an updater could not read."""
    date = dt.datetime.fromtimestamp(commit_time, dt.timezone.utc).isoformat()
    if not isinstance(document, dict) or document.keys() != {"commit", "date", "supports"}:
        raise ValueError("expected commit, date and supports")
    if document["commit"] != commit or document["date"] not in (date, date.replace("+00:00", "Z")):
        raise ValueError(f"expected the built commit {commit} at {date}")
    supports = document["supports"]
    if (not isinstance(supports, dict) or supports.keys() != {"min", "max"}
            or any(type(v) is not int for v in supports.values())
            or not 1 <= supports["min"] <= supports["max"] <= 2**32 - 1):
        raise ValueError("expected an inclusive range of dedicated launch contracts")
    return document


def check_assets(folder: Path, legs: dict, signed: bool) -> list[str]:
    """Require the complete release file set, and return the names the signature must cover."""
    payload = sorted([leg["asset"] for leg in legs["include"]] + ["server-tool.json"])
    expected = sorted(payload + (["SHA256SUMS", "SHA256SUMS.sig"] if signed else []))
    actual = sorted(path.name for path in folder.iterdir())
    if expected != actual or not all((folder / name).is_file() for name in expected):
        raise SystemExit(f"not releasing: expected {', '.join(expected)} in {folder}, found {', '.join(actual)}")
    return payload


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    build_matrix = commands.add_parser("matrix")
    build_matrix.add_argument("game_build", type=Path)
    build_matrix.add_argument("platforms", type=Path)
    info = commands.add_parser("info", help="validate --version --json on stdin, write release metadata to stdout")
    info.add_argument("commit")
    info.add_argument("commit_time", type=int)
    assets = commands.add_parser("assets", help="check assets against the MATRIX env var")
    assets.add_argument("folder", type=Path)
    assets.add_argument("--signed", action="store_true")
    args = parser.parse_args()
    if args.command == "matrix":
        print(json.dumps(matrix(args.game_build, args.platforms), separators=(",", ":")))
    elif args.command == "info":
        try:
            print(json.dumps(version_info(json.load(sys.stdin), args.commit, args.commit_time), indent=2))
        except ValueError as error:
            sys.exit(f"sunrise-server --version --json: {error}")
    else:
        print("\n".join(check_assets(args.folder, json.loads(os.environ["MATRIX"]), args.signed)))


if __name__ == "__main__":
    main()
