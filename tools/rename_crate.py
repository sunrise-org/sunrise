#!/usr/bin/env python3
"""Rename the package crate only; never rename the player's sunrise-mods folder.

Declared edits: crate paths, Cargo package/dependency/feature names, whole Rust crate
identifiers (including linker and source-index strings), generated Cargo package
list entries, two crate-tool fixtures, the AGENTS crate map, lockfile ordering,
and rustfmt wrapping. The exact staging regression below is a hand-written addition.
Every other old-name hit is written to --report for review, not changed.
"""
import argparse
from collections import Counter
from pathlib import Path
import re
import subprocess

ROOT = Path(__file__).resolve().parents[1]
OLD, NEW = "sunrise-mods", "sunrise-mod-package"
OLD_PATH, NEW_PATH = "crates/" + OLD, "crates/" + NEW
OWN_FILES = {"tools/rename_crate.py", "tests/test_rename_crate.py"}
LOCKS = {"Cargo.lock"}
PACKAGE_TOOLS = {"tools/pr_ready.py", "tests/test_pr_ready.py"}


def run(*args, data=None):
    return subprocess.check_output(args, cwd=ROOT, input=data)


def destination(path):
    return re.sub(r"^" + re.escape(OLD_PATH) + r"(?=/|$)", NEW_PATH, path)


def rewrite(path, source):
    if path in OWN_FILES or path.startswith("crates/sunrise-client/assets/sunrise/lang/"):
        return source
    source = re.sub(re.escape(OLD_PATH) + r"(?=[^a-zA-Z0-9_-]|$)", NEW_PATH, source)
    source = re.sub(r"\bsunrise_mods\b", "sunrise_mod_package", source)
    if Path(path).name == "Cargo.toml":
        source = re.sub(r"(?m)^sunrise-mods(?=\s*=)", NEW, source)
        source = re.sub(r"(?m)^(name|package)(\s*=\s*[\"'])sunrise-mods([\"'])",
                        lambda m: m[1] + m[2] + NEW + m[3], source)
        source = source.replace("../sunrise-mods\"", "../sunrise-mod-package\"")
        source = source.replace("../sunrise-mods'", "../sunrise-mod-package'")
        source = source.replace("dependencies.sunrise-mods]", "dependencies.sunrise-mod-package]")
    if path == "crates/cargo-sunrise-mod/src/check.rs":
        source = source.replace("../sunrise-mods/tests/", "../sunrise-mod-package/tests/")
    if path in LOCKS:
        source = source.replace('"sunrise-mods"', '"sunrise-mod-package"')
    if path in PACKAGE_TOOLS:
        source = source.replace(OLD, NEW)
    if path == "AGENTS.md":
        source = source.replace("`sunrise-mods`", "`sunrise-mod-package`")
    return source


def normalized(path, data):
    # Git's -I boundary selects text before decoding; binary moved files stay byte-identical.
    source = rewrite(path, data.decode())
    if path.endswith(".rs"):
        return run("rustfmt", "--edition", "2024", "--config", "skip_children=true",
                   "--emit", "stdout", data=source.encode())
    return source.encode()


def text_paths(base=None):
    args = ["git", "grep", "-Ilz", "-e", OLD, "-e", "sunrise_mods"]
    output = run(*args, *([base] if base else []))
    return {name.removeprefix(base + ":") if base else name
            for name in output.decode().split("\0") if name}


def names(base=None):
    args = ["git", "ls-tree", "-rz", "--name-only", base] if base else ["git", "ls-files", "-z"]
    return {name for name in run(*args).decode().split("\0") if name}


def verify(base):
    before, after = names(base), names()
    if after - OWN_FILES != {destination(path) for path in before - OWN_FILES}:
        raise SystemExit("unexpected file inventory")
    if (ROOT / OLD_PATH).exists():
        raise SystemExit("old crate directory still exists")
    changed = {name for name in run("git", "diff", "--name-only", "-z", "--no-renames", base).decode().split("\0") if name}
    text = text_paths(base)
    files = 0
    for path in sorted(before - OWN_FILES):
        target = destination(path)
        changed_file = path in changed or target in changed
        if not changed_file and path not in text:
            continue
        original = run("git", "show", f"{base}:{path}")
        actual = (ROOT / target).read_bytes()
        expected = normalized(path, original) if path in text else original
        if not changed_file and expected == original:
            continue
        if path in LOCKS:
            if Counter(expected.splitlines()) != Counter(actual.splitlines()):
                raise SystemExit(f"lockfile changed dependencies: {path}")
        elif actual != expected:
            raise SystemExit(f"unexpected content change: {target}")
        files += 1
    print(f"Verified {files} rename-normalized files")
    print("Declared normalization: crate-only mappings; rustfmt; lockfile ordering; exact 8-line staging regression excluded. Player paths and registry URLs unchanged.")


def report(path):
    rows = []
    for name in sorted(text_paths() - OWN_FILES):
        for line, text in enumerate((ROOT / name).read_text().splitlines(), 1):
            if OLD in text or "sunrise_mods" in text:
                rows.append(f"{name}:{line}: {text}")
    path.write_text("Old-name hits retained for review (player paths, registry URLs, prose, test names, and negative checks):\n\n" + "\n".join(rows) + "\n")
    print(f"Retained {len(rows)} old-name lines for review in {path}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    mode = parser.add_mutually_exclusive_group(required=True)
    mode.add_argument("--apply", action="store_true")
    mode.add_argument("--verify", metavar="BASE")
    parser.add_argument("--report", type=Path)
    args = parser.parse_args()
    if args.apply:
        move = (ROOT / OLD_PATH).is_dir()
        if move == (ROOT / NEW_PATH).is_dir():
            raise SystemExit("expected exactly one crate directory")
        for path in sorted(text_paths() - OWN_FILES):
            original = (ROOT / path).read_bytes()
            updated = normalized(path, original)
            if updated != original:
                (ROOT / path).write_bytes(updated)
        if move:
            run("git", "mv", OLD_PATH, NEW_PATH)
        run("cargo", "metadata", "--offline", "--format-version", "1", "--manifest-path", "Cargo.toml")
    else:
        verify(args.verify)
    if args.report:
        report(args.report)


if __name__ == "__main__":
    main()
