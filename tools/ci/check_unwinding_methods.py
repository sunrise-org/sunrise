#!/usr/bin/env python3
"""The list of gated Host methods that can only refuse by unwinding only shrinks (#3123).

crates/sunrise-mod-api-gen/unwinding-methods.txt names each `#[access]` method whose return type has no
refusal-shaped `Result`. The generator refuses a method missing from it and a line that no longer applies; this
refuses a line added since the base, so the list cannot be grown to admit a new method. A base that cannot be
read is an error, never a new list.

    python3 tools/ci/check_unwinding_methods.py BASE_SHA
"""
import subprocess
import sys

LIST = "crates/sunrise-mod-api-gen/unwinding-methods.txt"


def lines(text):
    return {line for line in text.splitlines() if line and not line.startswith("#")}


def git(*args):
    done = subprocess.run(["git", *args], capture_output=True, text=True)
    if done.returncode != 0:
        sys.exit(f"git {' '.join(args)} failed, so the base list cannot be read:\n{done.stderr}")
    return done.stdout


def main():
    base = sys.argv[1]
    git("cat-file", "-e", f"{base}^{{tree}}")
    # An existing base tree without the list means the list is new in this change.
    if not git("ls-tree", base, "--", LIST):
        return
    with open(LIST) as current:
        added = sorted(lines(current.read()) - lines(git("show", f"{base}:{LIST}")))
    if added:
        sys.exit(f"{LIST} only shrinks; these lines were added since the base:\n" + "\n".join(added))


if __name__ == "__main__":
    main()
