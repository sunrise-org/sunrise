#!/usr/bin/env python3
"""Compare a spectated entity's blindness, nightVision and darknessFactor with vanilla 26.3, frame by frame (#4126).

Compiles and runs VanillaCameraEffectsOracle against the pinned 26.3 client jar on this corpus: per case a fresh
zombie or armor stand the view is on, effect packets (decoded from their wire bytes and handled by vanilla's own
ClientPacketListener) adding, updating and removing blindness, night vision and darkness (darkness unblended and
blended, each counted down through zero), ticks run by vanilla's own ClientLevel.tickEntities in a running and a
frozen level, and this client's player mounting and leaving the entity. After each
step the oracle reads the values at each partial tick. It writes the witness and meta.json into
.local/witness/<version>/camera_effects/, then runs the sunrise-client fixture
game::remote::camera_view_tests::the_camera_entitys_effects_match_vanilla_frame_by_frame against it
(SUNRISE_CAMERA_EFFECTS_WITNESS), which replays every step through Sunrise's packet handlers and entity tick and
compares each frame's float bits exactly.

  test_camera_effects_parity.py [--resources DIR] [--record-only]
"""
import argparse
import json
import os
from pathlib import Path
import random
import struct
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))
from tools import lab, oracle_harness as harness, test_rust_parity as base, test_model_baking_parity as baking

ORACLE = 'VanillaCameraEffectsOracle'
TEST = 'game::remote::camera_view_tests::the_camera_entitys_effects_match_vanilla_frame_by_frame'
WITNESS_ENV = 'SUNRISE_CAMERA_EFFECTS_WITNESS'

BLINDNESS, NIGHT_VISION, DARKNESS = 'minecraft:blindness', 'minecraft:night_vision', 'minecraft:darkness'
# ClientboundUpdateMobEffectPacket's flags: ambient 1, visible 2, show icon 4, blend 8.
BLEND = 8


def f32_bits(value):
    return struct.unpack('<I', struct.pack('<f', value))[0]


# The frames read after each step: the partial ticks a frame can fall on.
PARTIALS = [f32_bits(p) for p in (0.0, 0.25, 0.5, 0.75, 1.0)]


def update(effect, duration, flags=0, amplifier=0):
    return {'op': 'update', 'effect': effect, 'duration': duration, 'amplifier': amplifier, 'flags': flags}


def remove(effect):
    return {'op': 'remove', 'effect': effect}


def tick(frozen=False, count=1):
    return [{'op': 'tick', 'frozen': frozen}] * count


def carry(on):
    return {'op': 'carry', 'on': on}


def fixed_cases():
    yield 'darkness blended in and out, updated, removed and added again', 'zombie', 0.5, [
        update(DARKNESS, 30, BLEND), *tick(count=10),
        # forceAddEffect's copyBlendState keeps the fade; the longer duration turns it back up.
        update(DARKNESS, 100, BLEND), *tick(count=2),
        remove(DARKNESS), update(DARKNESS, 600), remove(DARKNESS),
        update(DARKNESS, 600, BLEND), *tick(count=3), remove(DARKNESS), *tick(),
    ]
    yield ('darkness unblended from 40 ticks through zero and two ticks past it, then blended from 23 through '
           'zero, on an armor stand, a scale that rounds'), 'armor_stand', 0.37, [
        update(DARKNESS, 40, 2), *tick(count=42), remove(DARKNESS), *tick(),
        update(DARKNESS, 23, BLEND | 7), *tick(count=25), remove(DARKNESS),
    ]
    yield 'night vision over 200 ticks, flickering, infinite, negative, ending', 'zombie', 1.0, [
        update(NIGHT_VISION, 203, 6), *tick(count=5),
        update(NIGHT_VISION, -1), *tick(),
        update(NIGHT_VISION, -2), *tick(),
        update(NIGHT_VISION, 0), *tick(),
        update(NIGHT_VISION, 2, BLEND), *tick(count=3),
        remove(NIGHT_VISION), *tick(),
    ]
    yield 'blindness added and removed, and removed when absent', 'zombie', 1.0, [
        update(BLINDNESS, 40, 2), *tick(), remove(BLINDNESS), remove(BLINDNESS), *tick(),
        update(BLINDNESS, 1, 3), *tick(count=2), remove(BLINDNESS),
    ]
    for entity in ('zombie', 'armor_stand'):
        yield f'a frozen level ticks the {entity} only while it carries the player', entity, 1.0, [
            update(DARKNESS, 50, BLEND), update(NIGHT_VISION, 205), update(BLINDNESS, 30),
            *tick(count=2), *tick(frozen=True, count=3),
            carry(True), *tick(frozen=True, count=2), *tick(count=1),
            carry(False), *tick(frozen=True, count=2), *tick(count=2),
            remove(DARKNESS), remove(NIGHT_VISION), *tick(frozen=True), remove(BLINDNESS),
        ]
    yield 'all three at once, other amplifiers and flags', 'zombie', 0.75, [
        update(BLINDNESS, 60, 15, 3), update(NIGHT_VISION, 210, 15, 255), update(DARKNESS, 60, 15, 1),
        *tick(count=12), update(DARKNESS, 60, 1, 2), *tick(count=3),
        remove(NIGHT_VISION), *tick(count=2), remove(BLINDNESS), remove(DARKNESS), *tick(),
    ]


DURATIONS = [-2, -1, 0, 1, 2, 21, 22, 23, 24, 30, 199, 200, 201, 202, 400]


def random_cases(rng, count, steps):
    for n in range(count):
        entity = rng.choice(['zombie', 'armor_stand'])
        scale = rng.choice([0.0, 0.25, 0.37, 0.5, 1.0])
        carrying = False
        ops = []
        for _ in range(steps):
            roll = rng.random()
            if roll < 0.45:
                ops += tick(frozen=rng.random() < 0.3)
            elif roll < 0.75:
                ops.append(update(rng.choice([BLINDNESS, NIGHT_VISION, DARKNESS]), rng.choice(DURATIONS),
                                  rng.randrange(16), rng.choice([0, 1, 255])))
            elif roll < 0.92:
                ops.append(remove(rng.choice([BLINDNESS, NIGHT_VISION, DARKNESS])))
            else:
                carrying = not carrying
                ops.append(carry(carrying))
        yield f'random {n}', entity, scale, ops


def corpus():
    """The cases, each on an entity of its own: ids from 100, clear of the fixture's own (7, 31, 40)."""
    rng = random.Random(0x4126)
    cases = [*fixed_cases(), *random_cases(rng, 16, 48)]
    return {'cases': [
        {'name': name, 'entity': entity, 'id': 100 + i, 'darkness_scale': scale, 'partials': PARTIALS, 'steps': steps}
        for i, (name, entity, scale, steps) in enumerate(cases)
    ]}


def record(lock, jars, libraries, build):
    version = lock['minecraft']['version']
    witness_dir = lab.LOCAL / 'witness' / version / 'camera_effects'
    witness_dir.mkdir(parents=True, exist_ok=True)
    inputs = build / 'inputs.json'
    inputs.write_text(json.dumps(corpus(), separators=(',', ':')), encoding='utf-8')
    witness = witness_dir / 'witness.json'
    classes, dependencies, oracle_path = harness.compile_oracle(
        build, ORACLE, jars, libraries, lock['minecraft']['java_major'])
    harness.java_run(build, classes, dependencies, ORACLE, [inputs, witness], 'oracle')
    java = subprocess.check_output(['java', '--version'], text=True).splitlines()[0].strip()
    lab.write_json(witness_dir / 'meta.json', dict(
        minecraft=version,
        client_jar_sha256=lab.digest(jars['client'], 'sha256'),
        java_version=java,
        command='python3 tools/test_camera_effects_parity.py',
        oracle=f'mods/sunrise/src/test/java/dev/sunriselab/parity/{ORACLE}.java',
        inputs_sha256={'oracle': lab.digest(oracle_path, 'sha256'), 'corpus': lab.digest(inputs, 'sha256')},
        outputs={witness.name: lab.digest(witness, 'sha256')},
    ))
    return witness


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--resources', type=Path,
                        default=Path(os.environ.get('SUNRISE_TEST_RESOURCES', lab.RESOURCES)))
    parser.add_argument('--record-only', action='store_true', help='record the vanilla witness and skip the Rust side')
    args = parser.parse_args()
    lock, jars, _ = base.verify_artifacts()
    build = lab.LOCAL / 'camera-effects-parity'
    build.mkdir(parents=True, exist_ok=True)
    witness = record(lock, jars, baking.client_libraries(), build)
    cases = len(json.loads(witness.read_text(encoding='utf-8'))['cases'])
    if args.record_only:
        print(f'wrote {witness} ({cases} cases)')
        return
    with tempfile.TemporaryDirectory(prefix='sunrise-camera-effects-') as tmp:
        scratch = Path(tmp)
        homes = {k: scratch / k for k in ('SUNRISE_HOME', 'SUNRISE_LAUNCHER_HOME', 'XDG_DATA_HOME', 'XDG_CONFIG_HOME')}
        env = lab.game_env(os.environ, **homes, RUST_MIN_STACK='16777216')
        env['SUNRISE_TEST_RESOURCES'] = str(args.resources.resolve())
        env[WITNESS_ENV] = str(witness.resolve())
        log = build / 'rust-fixture.log'
        with log.open('w', encoding='utf-8') as out:
            ran = subprocess.run([str(lab.parity_fixtures()), TEST, '--exact', '--ignored', '--nocapture'],
                                 cwd=scratch, env=env, timeout=3600, stdout=out, stderr=subprocess.STDOUT)
        text = log.read_text(encoding='utf-8', errors='replace')
        if ran.returncode or '1 passed; 0 failed' not in text:
            sys.exit(f'{TEST} failed against {witness}; see {log}\n{text[-3000:]}')
    print(f'Camera effects parity passed ({cases} cases against {witness})')


if __name__ == '__main__':
    main()
