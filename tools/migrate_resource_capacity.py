#!/usr/bin/env python3
"""Migrate ResourceHandler capacity signatures to accept an optional resource."""

import re
import subprocess
from pathlib import Path


SIGNATURE = re.compile(
    r"(\bfn capacity(?:_int)?\(\s*&self,\s*\w+: &mut dyn (?:mod_api::)?ServerHost,"
    r"\s*\w+: usize,\s*\w+: )&([A-Za-z_]\w*)(?=[,)])"
)


def main():
    root = Path(__file__).resolve().parents[1]
    names = subprocess.check_output(
        ["git", "ls-files", "-z", "--", "crates/**/*.rs"], cwd=root
    )
    for name in filter(None, names.decode().split("\0")):
        path = root / name
        old = path.read_text()
        new = SIGNATURE.sub(r"\1Option<&\2>", old)
        if new != old:
            path.write_text(new)


if __name__ == "__main__":
    main()
