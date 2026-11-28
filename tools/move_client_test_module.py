#!/usr/bin/env python3
"""Mechanically move one file-backed client test module to the `it` target.

Run from the Sunrise repository root, for example:

    python3 tools/move_client_test_module.py \
      --source crates/sunrise-client/src/cut_layout/game/modding/native_item_places.rs \
      --declaration-file crates/sunrise-client/src/cut_layout/game/server_side.rs \
      --module-path cut_layout::game::server_side::native_item_places \
      --integration-main crates/sunrise-client/tests/it/main.rs \
      --policy-dir tools/ci/ignored-tests

The operation validates every input before changing files. It moves the source,
removes its exact `#[path] pub(crate) mod` declaration, wires the same module
path into the integration-test crate, and changes matching daily policy rows
from `lib` to `test:it`.
"""

from __future__ import annotations

import argparse
import os
import re
import stat
import sys
import tempfile
from pathlib import Path


IDENTIFIER = r"[A-Za-z_][A-Za-z0-9_]*"
PATH_MODULE = re.compile(
    rf"(?m)^(?P<indent>[ \t]*)#\[\s*path\s*=\s*\"(?P<path>[^\"\r\n]+)\"\s*\]"
    rf"(?:[ \t]+|\r?\n(?P=indent))"
    rf"(?P<visibility>pub(?:\s*\([^)]*\))?\s+)?mod\s+(?P<name>{IDENTIFIER})\s*;"
    rf"[ \t]*(?:\r?\n|$)"
)
MODULE_ITEM = re.compile(
    rf"(?m)^[ \t]*(?P<visibility>pub(?:\s*\([^)]*\))?\s+)?mod\s+(?P<name>{IDENTIFIER})\b(?P<tail>[^\r\n]*)"
)
POLICY_FIELDS = re.compile(r"\S+")


class ExtractionError(Exception):
    """The requested extraction is not safe to apply mechanically."""


def _inside(root: Path, path: Path, label: str, *, strict: bool) -> Path:
    candidate = path if path.is_absolute() else root / path
    if candidate.is_symlink():
        raise ExtractionError(f"{label} must not be a symbolic link: {candidate}")
    try:
        resolved = candidate.resolve(strict=strict)
        resolved.relative_to(root)
    except (OSError, ValueError) as error:
        raise ExtractionError(f"{label} must be inside the repository: {candidate}") from error

    parent = candidate.parent
    while parent != root and root in parent.parents:
        if parent.is_symlink():
            raise ExtractionError(f"{label} has a symbolic-link parent: {parent}")
        parent = parent.parent
    return resolved


def _module_segments(module_path: str) -> tuple[str, ...]:
    segments = tuple(module_path.split("::"))
    if not segments or any(not re.fullmatch(IDENTIFIER, item) for item in segments):
        raise ExtractionError("--module-path must be a Rust module path using `::` separators")
    return segments


def _has_preceding_outer_attribute(text: str, position: int) -> bool:
    attributes = []
    closing_brackets = 0
    for line in reversed(text[:position].splitlines()):
        stripped = line.strip()
        if not stripped or stripped.startswith("//"):
            continue
        bracket_delta = line.count("]") - line.count("[")
        if closing_brackets:
            attributes.append(line)
            closing_brackets += bracket_delta
            continue
        if stripped.startswith("#[") or bracket_delta > 0:
            attributes.append(line)
            closing_brackets += bracket_delta
            continue
        break
    return any(line.strip().startswith("#[") for line in attributes)


def _declaration_to_remove(declaration_file: Path, source: Path, module_name: str) -> tuple[str, int, int]:
    text = declaration_file.read_text(encoding="utf-8")
    module_items = [match for match in MODULE_ITEM.finditer(text) if match.group("name") == module_name]
    matching_path_items = []
    for match in PATH_MODULE.finditer(text):
        attribute_target = (declaration_file.parent / match.group("path")).resolve(strict=False)
        if attribute_target == source:
            matching_path_items.append(match)

    if len(matching_path_items) != 1:
        raise ExtractionError(
            f"expected one #[path] declaration resolving to {source}, found {len(matching_path_items)}"
        )
    declaration = matching_path_items[0]
    if _has_preceding_outer_attribute(text, declaration.start()):
        raise ExtractionError(
            f"the file-backed module {module_name} has a preceding unsupported outer attribute"
        )
    visibility = re.sub(r"\s+", "", declaration.group("visibility") or "")
    if declaration.group("name") != module_name or visibility != "pub(crate)":
        raise ExtractionError(
            f"the declaration for {source} must be `#[path = ...] pub(crate) mod {module_name};`"
        )
    if len(module_items) != 1 or not text[declaration.start():declaration.end()].strip().endswith(
        f"mod {module_name};"
    ):
        raise ExtractionError(f"found a mismatched or duplicate declaration for module {module_name}")
    return text, declaration.start(), declaration.end()


def _add_module_declaration(path: Path, name: str, *, main: bool) -> str:
    existed = path.exists()
    if existed:
        if not path.is_file():
            raise ExtractionError(f"module wrapper is not a file: {path}")
        text = path.read_text(encoding="utf-8")
    else:
        text = ""

    items = [match for match in MODULE_ITEM.finditer(text) if match.group("name") == name]
    path_items = [match for match in PATH_MODULE.finditer(text) if match.group("name") == name]
    mentions = list(re.finditer(rf"\bmod\s+{re.escape(name)}\b", text))
    if path_items or len(mentions) != len(items):
        raise ExtractionError(f"conflicting or attributed declaration for module {name} in {path}")
    if items:
        if len(items) != 1:
            raise ExtractionError(f"duplicate declarations for module {name} in {path}")
        item = items[0]
        visibility = (item.group("visibility") or "").strip()
        if item.group("tail").strip() != ";" or visibility not in ("", "pub(crate)"):
            raise ExtractionError(f"conflicting declaration for module {name} in {path}")
        return text

    line = f"mod {name};" if main else f"pub(crate) mod {name};"
    if text and not text.endswith(("\n", "\r")):
        text += "\r\n" if "\r\n" in text else "\n"
    newline = "\r\n" if "\r\n" in text else "\n"
    return text + line + newline


def _rewrite_policy(policy_dir: Path, module_path: str) -> tuple[dict[Path, str], int]:
    prefix = module_path + "::"
    updates: dict[Path, str] = {}
    changed = 0
    for path in sorted(policy_dir.glob("*.txt")):
        if path.is_symlink() or not path.is_file():
            raise ExtractionError(f"policy file must be a regular file: {path}")
        original = path.read_text(encoding="utf-8")
        rewritten_lines = []
        file_changed = False
        for line in original.splitlines(keepends=True):
            body = line.rstrip("\r\n")
            fields = list(POLICY_FIELDS.finditer(body))
            if len(fields) < 2 or fields[0].group() != "sunrise-client":
                rewritten_lines.append(line)
                continue
            test_name = fields[1].group()
            if not test_name.startswith(prefix):
                rewritten_lines.append(line)
                continue
            if len(fields) < 5:
                raise ExtractionError(f"malformed policy row for {test_name} in {path}")
            target, tier = fields[2].group(), fields[3].group()
            if tier == "daily" and target == "lib":
                start, end = fields[2].span()
                ending = line[len(body):]
                rewritten_lines.append(body[:start] + "test:it" + body[end:] + ending)
                changed += 1
                file_changed = True
            else:
                rewritten_lines.append(line)
        if file_changed:
            updates[path] = "".join(rewritten_lines)

    if changed == 0:
        raise ExtractionError(f"no matching `lib daily` policy rows found for {module_path}")
    return updates, changed


def _commit_changes(source: Path, destination: Path, updates: dict[Path, str]) -> None:
    created_dirs: list[Path] = []
    staged: dict[Path, Path] = {}
    originals: dict[Path, tuple[bytes | None, int | None]] = {}
    replaced: list[Path] = []
    linked_destination = False
    source_removed = False

    try:
        directories = {destination.parent, *(path.parent for path in updates)}
        for directory in sorted(directories, key=lambda item: len(item.parts)):
            missing = []
            current = directory
            while not current.exists():
                missing.append(current)
                current = current.parent
            for item in reversed(missing):
                item.mkdir()
                created_dirs.append(item)

        for path, content in updates.items():
            if path.exists():
                originals[path] = (path.read_bytes(), stat.S_IMODE(path.stat().st_mode))
            else:
                originals[path] = (None, None)
            descriptor, temporary_name = tempfile.mkstemp(prefix=f".{path.name}.", suffix=".tmp", dir=path.parent)
            temporary = Path(temporary_name)
            staged[path] = temporary
            with os.fdopen(descriptor, "wb") as handle:
                handle.write(content.encode("utf-8"))
                handle.flush()
                os.fsync(handle.fileno())
            if originals[path][1] is not None:
                temporary.chmod(originals[path][1])

        if destination.exists() or destination.is_symlink():
            raise ExtractionError(f"destination already exists: {destination}")
        os.link(source, destination)
        linked_destination = True
        source.unlink()
        source_removed = True

        for path, temporary in staged.items():
            os.replace(temporary, path)
            replaced.append(path)
    except Exception:
        for path in reversed(replaced):
            previous, mode = originals[path]
            if previous is None:
                path.unlink(missing_ok=True)
            else:
                path.write_bytes(previous)
                if mode is not None:
                    path.chmod(mode)
        if linked_destination:
            if source_removed and not source.exists():
                os.link(destination, source)
            destination.unlink(missing_ok=True)
        raise
    finally:
        for temporary in staged.values():
            temporary.unlink(missing_ok=True)
        for directory in reversed(created_dirs):
            try:
                directory.rmdir()
            except OSError:
                pass


def extract(
    root: Path,
    source_arg: str,
    declaration_file_arg: str,
    module_path: str,
    integration_main_arg: str,
    policy_dir_arg: str,
) -> int:
    """Validate and perform one extraction. Return the number of policy rows moved."""
    root = root.resolve(strict=True)
    segments = _module_segments(module_path)
    source = _inside(root, Path(source_arg), "source", strict=True)
    declaration_file = _inside(root, Path(declaration_file_arg), "declaration file", strict=True)
    integration_main = _inside(root, Path(integration_main_arg), "integration main", strict=True)
    policy_dir = _inside(root, Path(policy_dir_arg), "policy directory", strict=True)

    source_relative = source.relative_to(root)
    if source_relative.parts[:3] != ("crates", "sunrise-client", "src") or not source.is_file():
        raise ExtractionError("--source must be a regular file under crates/sunrise-client/src")
    if not declaration_file.is_file() or not integration_main.is_file() or not policy_dir.is_dir():
        raise ExtractionError("declaration file, integration main, and policy directory must exist")
    if source.stem != segments[-1]:
        raise ExtractionError(f"source filename {source.name} does not match module-path leaf {segments[-1]}")

    destination = root / "crates" / "sunrise-client" / "tests" / "it" / Path(*segments).with_suffix(".rs")
    _inside(root, destination, "destination", strict=False)
    if destination.exists() or destination.is_symlink():
        raise ExtractionError(f"destination already exists: {destination}")

    declaration_text, start, end = _declaration_to_remove(declaration_file, source, segments[-1])
    updates: dict[Path, str] = {declaration_file: declaration_text[:start] + declaration_text[end:]}

    main_text = _add_module_declaration(integration_main, segments[0], main=True)
    if main_text != integration_main.read_text(encoding="utf-8"):
        updates[integration_main] = main_text

    it_root = root / "crates" / "sunrise-client" / "tests" / "it"
    for index in range(len(segments) - 1):
        wrapper = it_root / Path(*segments[: index + 1]).with_suffix(".rs")
        _inside(root, wrapper, "module wrapper", strict=False)
        updated = _add_module_declaration(wrapper, segments[index + 1], main=False)
        if updated != (wrapper.read_text(encoding="utf-8") if wrapper.exists() else ""):
            updates[wrapper] = updated

    policy_updates, changed_rows = _rewrite_policy(policy_dir, module_path)
    updates.update(policy_updates)
    _commit_changes(source, destination, updates)
    return changed_rows


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source", required=True, help="file under crates/sunrise-client/src")
    parser.add_argument("--declaration-file", required=True, help="Rust file containing the exact #[path] declaration")
    parser.add_argument("--module-path", required=True, help="full Rust module path, separated by `::`")
    parser.add_argument("--integration-main", required=True, help="sunrise-client integration-test main.rs")
    parser.add_argument("--policy-dir", required=True, help="directory containing daily ignored-test policy rows")
    args = parser.parse_args(argv)
    try:
        count = extract(Path.cwd(), args.source, args.declaration_file, args.module_path,
                        args.integration_main, args.policy_dir)
    except (ExtractionError, OSError, UnicodeError) as error:
        print(f"move_client_test_module.py: {error}", file=sys.stderr)
        return 2
    print(f"Moved {args.module_path}; updated {count} daily policy row(s).")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
