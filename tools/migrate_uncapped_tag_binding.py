#!/usr/bin/env python3
"""Convert the simple #6206 TagDefinitions calls; run from the repo root.

Only the listed receivers and call shapes are eligible. Callers with bespoke
error handling or block closures are converted by hand separately.
"""

from collections import Counter
from pathlib import Path
import re


PROBES = {
    "biome_source_codec_fixture": "tags",
    "block_survival_fixture": "tags fluid_tags",
    "bulk_section_fixture": "definitions",
    "chunk_stage_fixture": "definitions",
    "feature_region_fixture": "definitions fluid_defs",
    "heightmap_fixture": "definitions",
    "material_condition_fixture": "tag_definitions",
    "material_stage_fixture": "biome_tags block_tags",
    "movement_registry_fixture": "tags",
    "registry_collision_scan_fixture": "tags",
    "replace_block_fixture": "definitions",
    "rule_test_fixture": "definitions",
    "scattered_ore_fixture": "definitions",
    "simple_block_fixture": "tags fluid_tags",
    "state_provider_fixture": "definitions fluid_tags definitions",
    "suffocation_fixture": "tags",
    "support/decoration_storage": "definitions",
    "terrain_fixture": "definitions",
    "tree_feature_fixture": "tags fluid_tags",
}
CALLERS = {
    "sunrise-level/src/assets.rs": "definitions",
    "sunrise-client/src/animate_tick.rs": "definitions",
    "sunrise-sim/src/mod_content_load_conditions_tests.rs": "tags",
    "sunrise-sim/src/motion.rs": "definitions",
    "sunrise-world/src/worldgen/tags.rs": "definitions " * 5,
    **{f"sunrise-world/examples/probe/{stem}.rs": names for stem, names in PROBES.items()},
}


def main():
    changes = {}
    remaining = []
    count = 0
    for relative, names in CALLERS.items():
        path = Path("crates") / relative
        source = path.read_text(encoding="utf-8")
        expected = Counter(names.split())
        receivers = "|".join(expected)
        call = re.compile(
            rf"\b(?P<receiver>{receivers})(?P<gap>\s*)\.build"
            r"(?P<args>\(\|\w+, _(?:required)?\|[^\n]+\))"
            r"(?:\?|\s*\.unwrap\(\));$",
            re.MULTILINE,
        )
        found = Counter(m["receiver"] for m in call.finditer(source))
        if not found:
            remaining.append(path)
            continue
        if found != expected:
            raise SystemExit(f"{path}: expected {expected}, found {found}; no files changed")
        changes[path] = call.sub(r"\g<receiver>\g<gap>.bind\g<args>;", source)
        count += sum(found.values())
    if not changes:
        print("No original tag-binding calls found; migration already applied.")
        return
    if remaining:
        raise SystemExit(f"Partial migration: no original calls in {remaining}; no files changed")
    for path, updated in changes.items():
        path.write_text(updated, encoding="utf-8")
    print(f"Migrated {count} tag-binding calls in {len(changes)} Rust files.")


if __name__ == "__main__":
    main()
