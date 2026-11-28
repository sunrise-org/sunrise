#!/usr/bin/env python3
"""Strip direct Result adapters after PlayerMotion::new becomes infallible.

Run from the repository root on the validation commit, before the hand edits.
Without --apply, report the inventory without changing anything.
"""

import argparse
from collections import Counter
from pathlib import Path
import re
import subprocess

CALL = re.compile(r"\bPlayerMotion::new\s*\(")
STRING = r'"(?:\\.|[^"\\])*"'
# Find the constructor's closing paren without consuming parentheses in
# nested calls, strings, or comments. This is a one-time migration, not a
# general Rust parser; unexpected forms are left alone for inspection.
TOKEN = re.compile(
    rf'r(?P<hashes>\#*)".*?"(?P=hashes)|{STRING}|\'[^\'\n]\'|//[^\n]*|/\*.*?\*/|[()]',
    re.DOTALL,
)
ADAPTER = re.compile(
    rf'\s*(?:(?P<unwrap>\.unwrap\(\))|(?P<expect>\.expect\(\s*{STRING}\s*\))|(?P<try>\?))',
    re.DOTALL,
)
EXPECTED = Counter({"unwrap": 705, "expect": 9, "try": 13, "leftover": 7})


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--apply", action="store_true")
    args = parser.parse_args()
    files = subprocess.check_output(
        ["git", "grep", "-lz", "-F", "PlayerMotion::new", "--", "*.rs"]
    ).decode().split("\0")[:-1]
    counts = Counter()
    changes = {}
    for path in sorted(map(Path, files)):
        source = path.read_text()
        removals = []
        for call in CALL.finditer(source):
            depth = 1
            for token in TOKEN.finditer(source, call.end()):
                depth += (token[0] == "(") - (token[0] == ")")
                if depth == 0:
                    break
            else:
                raise SystemExit(f"{path}: unbalanced constructor call")
            adapter = ADAPTER.match(source, token.end())
            if adapter is None:
                counts["leftover"] += 1
                line = source.count("\n", 0, call.start()) + 1
                suffix = source[token.end():].lstrip().splitlines()[0]
                print(f"{path}:{line}: {suffix}")
            else:
                counts[adapter.lastgroup] += 1
                removals.append(adapter.span())
        if removals:
            for start, end in reversed(removals):
                source = source[:start] + source[end:]
            changes[path] = source
    print(f"{dict(counts)}; {len(changes)} Rust files to rewrite")
    if len(files) != 442 or counts != EXPECTED:
        raise SystemExit("unexpected call inventory; no files changed")
    if args.apply:
        for path, source in changes.items():
            path.write_text(source)


if __name__ == "__main__":
    main()
