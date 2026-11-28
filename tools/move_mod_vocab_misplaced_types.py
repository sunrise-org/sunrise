#!/usr/bin/env python3
"""Mechanical move of misplaced shared/server and client-phase types in sunrise-mod-vocab."""
from pathlib import Path
import shutil
import sys

ROOT = Path(sys.argv[1]).resolve() if len(sys.argv) > 1 else Path.cwd()
VOCAB = ROOT / "crates/sunrise-mod-vocab/src"


def extract_lines(path: Path, start: int, end: int) -> str:
    """Extract 1-based inclusive line range [start, end] and remove it from path."""
    lines = path.read_text(encoding="utf-8").splitlines(keepends=True)
    chunk = "".join(lines[start - 1 : end])
    # Trim one trailing blank line if leaving double blank lines
    before = lines[: start - 1]
    after = lines[end:]
    while before and before[-1].strip() == "" and after and after[0].strip() == "":
        after.pop(0)
    path.write_text("".join(before + after), encoding="utf-8")
    return chunk


def append_block(path: Path, block: str) -> None:
    text = path.read_text(encoding="utf-8")
    if not text.endswith("\n"):
        text += "\n"
    if not text.endswith("\n\n"):
        text += "\n"
    path.write_text(text + block.lstrip("\n"), encoding="utf-8")


def main() -> None:
    # 1. Move whole shared particle modules from client/ to world/
    for name in ("particle_keys.rs", "particle_types.rs", "particle_stacks.rs"):
        src = VOCAB / "client" / name
        dst = VOCAB / "world" / name
        shutil.move(str(src), str(dst))

    # 2. Split client/particles.rs -> world/particles.rs (PositionSource + ParticleOptions)
    #    Keep FireworkExplosion + FireworksRequest in client/particles.rs.
    particles_src = VOCAB / "client/particles.rs"
    particles_chunk = extract_lines(particles_src, 5, 202)
    world_particles = (
        "//! Owned particle requests, with provider identities resolved once when content loads.\n"
        "#[allow(unused_imports)]\n"
        "use crate::*;\n\n"
        + particles_chunk
    )
    (VOCAB / "world/particles.rs").write_text(world_particles, encoding="utf-8")

    # 3. Split client/first_person.rs -> entity/types.rs (Hand + impl Hand)
    #    Keep HandDrawContext in client/first_person.rs.
    hand_chunk = extract_lines(VOCAB / "client/first_person.rs", 4, 14)
    append_block(VOCAB / "entity/types.rs", hand_chunk)

    # 4. Split client/types.rs in reverse line order so line numbers stay exact:
    client_types = VOCAB / "client/types.rs"
    # 4a. MenuRef (1591-1613) -> item/types.rs
    menu_ref_chunk = extract_lines(client_types, 1591, 1613)
    append_block(VOCAB / "item/types.rs", menu_ref_chunk)
    # 4b. BossBarRef, BossBarColor, BossBarOverlay (1459-1563) -> world/types.rs
    boss_bar_chunk = extract_lines(client_types, 1459, 1563)
    # 4c. ParticleStatus (1421-1432) -> world/types.rs
    particle_status_chunk = extract_lines(client_types, 1421, 1432)
    # 4d. Answered<T> (986-996) -> answered.rs (private root module beside runtime.rs so surface id stays `type:Answered`)
    answered_chunk = extract_lines(client_types, 986, 996)
    (VOCAB / "answered.rs").write_text(
        "//! Replace/collect hook answer wrapper carrying the answering mod's identity.\n\n"
        + answered_chunk,
        encoding="utf-8",
    )
    # 4e. SoundSource (705-753) -> world/types.rs
    sound_source_chunk = extract_lines(client_types, 705, 753)
    append_block(
        VOCAB / "world/types.rs",
        sound_source_chunk + "\n" + particle_status_chunk + "\n" + boss_bar_chunk,
    )
    # 4f. NoteBlockChange, NoteInstrument (560-703) -> block/types.rs
    note_block_chunk = extract_lines(client_types, 560, 703)
    append_block(VOCAB / "block/types.rs", note_block_chunk)

    # 5. Split client/arm_pose.rs -> content/arm_pose.rs (ArmPose, CustomArmPose, PoseInfo, TEST_POSES, test_table)
    #    Keep ArmAngles (114-124) in client/arm_pose.rs.
    arm_pose_src = VOCAB / "client/arm_pose.rs"
    arm_table_chunk = extract_lines(arm_pose_src, 126, 149)
    arm_pose_chunk = extract_lines(arm_pose_src, 3, 112)
    content_arm_pose = (
        "//! `HumanoidModel.ArmPose`: how a humanoid model poses an arm, and the names an item declaration gives them.\n\n"
        + arm_pose_chunk
        + "\n"
        + arm_table_chunk
    )
    (VOCAB / "content/arm_pose.rs").write_text(content_arm_pose, encoding="utf-8")

    # 6. Split world/phase.rs (207-634) -> client/phase.rs
    client_phase_chunk = extract_lines(VOCAB / "world/phase.rs", 207, 634)
    client_phase = (
        "//! Phase-scoped client resource reload and client setup views.\n"
        "use crate::ReloadContext;\n\n"
        + client_phase_chunk
    )
    (VOCAB / "client/phase.rs").write_text(client_phase, encoding="utf-8")

    import re

    # 7. Mechanical path rewrites across Rust sources for the moved #[doc(hidden)] paths:
    #    - mod_api::client::first_person::Hand -> mod_api::Hand
    #    - crate::client::first_person::Hand -> crate::Hand
    #    - client::first_person::Hand (inside grouped `use mod_api::{...}`) -> Hand
    #    - mod_api::client::particles::PositionSource -> mod_api::PositionSource
    #    - crate::client::particles::PositionSource -> crate::PositionSource
    #    - client::particles::PositionSource (inside grouped `use mod_api::{...}`) -> PositionSource
    replacements = [
        (re.compile(r"\bmod_api::client::first_person::Hand\b"), "mod_api::Hand"),
        (re.compile(r"\bcrate::client::first_person::Hand\b"), "crate::Hand"),
        (re.compile(r"(?<![A-Za-z0-9_:])client::first_person::Hand\b"), "Hand"),
        (
            re.compile(r"\bmod_api::client::particles::PositionSource\b"),
            "mod_api::PositionSource",
        ),
        (
            re.compile(r"\bcrate::client::particles::PositionSource\b"),
            "crate::PositionSource",
        ),
        (
            re.compile(r"(?<![A-Za-z0-9_:])client::particles::PositionSource\b"),
            "PositionSource",
        ),
    ]
    for path in sorted((ROOT / "crates").rglob("*.rs")):
        if "target" in path.parts:
            continue
        text = path.read_text(encoding="utf-8")
        updated = text
        for pat, new in replacements:
            updated = pat.sub(new, updated)
        if updated != text:
            path.write_text(updated, encoding="utf-8")

    print("Mechanical move complete.")


if __name__ == "__main__":
    main()
