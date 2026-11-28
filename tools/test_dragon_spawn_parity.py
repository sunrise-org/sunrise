#!/usr/bin/env python3
"""The EnderDragon's `Mob.finalizeSpawn` flag and draw order, against vanilla 26.3.

`EnderDragonFight.createNewDragon` does not finalize its dragon, but
`SummonCommand` and spawn eggs do. Vanilla (`VanillaFinalizeSpawnOracle`)
creates an `EnderDragon` through `EntityType.create`, runs
`Mob.finalizeSpawn` with a seeded level random and reads `LeftHanded` back,
then prints the level random's next `nextLong`. Sunrise's own spawn path is
driven by the `dragon_spawn_parity_fixture` test: for the same seeded level
random it summons through `spawn_dragon_facing`, reads the spawned dragon's
`left_handed` and then the level random's next `nextLong`. Both sides must
agree on the flag and on that draw: the pair pins the triangle draw, the
left-handed roll and every draw in between, and catches a spawn path that
drops the assignment.
"""
import argparse
import json
import os
from pathlib import Path
import re
import subprocess
import sys

try:
    from . import lab, oracle_harness as harness, test_rust_parity as base
except ImportError:
    import lab
    import oracle_harness as harness
    import test_rust_parity as base

NAME = 'dragon-spawn-parity'
SEEDS = list(range(200)) + [123456789, -7, 2**62]
ORACLE = 'VanillaFinalizeSpawnOracle'
FIXTURE = 'game::server_side::dragon::tests::dragon_spawn_parity_fixture'


def cases():
    return [
        f'case/{seed} type=minecraft:ender_dragon reason=COMMAND difficulty=normal time=0 '
        f'inhabited=0 phase=0 seed={seed} eseed=0 pack=1 pos=8.50,100,8.50 '
        f'biome=minecraft:the_end dim=minecraft:the_end floor=minecraft:stone'
        for seed in SEEDS
    ]


def parse_vanilla(line):
    name = line.split()[0]
    flag = re.search(r'\blh=(true|false)\b', line)
    following = re.search(r'\| next (-?\d+)$', line)
    if not flag or not following:
        raise AssertionError(f'no LeftHanded flag or next draw in: {line}')
    return name, flag.group(1) == 'true', int(following.group(1))


def parse_rust(line):
    name, flag, following = line.split()
    return name, flag.removeprefix('lh=') == 'true', int(following.removeprefix('next='))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--prepare', action='store_true', help='prepare the pinned game artifacts first')
    args = parser.parse_args()
    if args.prepare:
        lab.prepare()
    lock, jars, libraries = base.verify_artifacts()
    build = lab.LOCAL / NAME
    build.mkdir(parents=True, exist_ok=True)
    report = build / 'report.json'
    report.unlink(missing_ok=True)
    inputs = build / 'inputs.txt'
    inputs.write_text(''.join(line + '\n' for line in cases()), encoding='utf-8')

    classes = build / 'classes'
    classes.mkdir(exist_ok=True)
    dependencies = os.pathsep.join(map(str, libraries))
    oracle = lab.ROOT / 'mods/sunrise/src/test/java/dev/sunriselab/parity' / f'{ORACLE}.java'
    harness.run_logged(
        ['javac', '--release', str(lock['minecraft']['java_major']), '-proc:none',
         '-sourcepath', lab.ROOT / 'mods/sunrise/src/test/java',
         '-cp', str(jars['server']) + os.pathsep + dependencies, '-d', classes, oracle],
        build, 'compile-java')
    vanilla_out = build / 'vanilla.txt'
    harness.run_logged(
        ['java', '-Xmx2g', '-cp', os.pathsep.join([str(classes), str(jars['server']), dependencies]),
         f'dev.sunriselab.parity.{ORACLE}', inputs, vanilla_out],
        build, 'vanilla')
    vanilla = [parse_vanilla(line) for line in vanilla_out.read_text(encoding='utf-8').splitlines()
               if line.startswith('case/')]
    assert len(vanilla) == len(SEEDS), (len(vanilla), len(SEEDS))
    assert {flag for _, flag, _ in vanilla} == {True, False}, 'the cases must exercise both flag values'

    rust_out = build / 'rust.txt'
    rust_out.unlink(missing_ok=True)
    harness.run_logged(
        [lab.parity_fixtures(), FIXTURE, '--exact'],
        build, 'rust',
        env=dict(os.environ,
                 SUNRISE_DRAGON_SPAWN_CASES=str(inputs),
                 SUNRISE_DRAGON_SPAWN_OUTPUT=str(rust_out),
                 SUNRISE_TEST_RESOURCES=os.environ.get('SUNRISE_TEST_RESOURCES', str(lab.RESOURCES))))
    rust = [parse_rust(line) for line in rust_out.read_text(encoding='utf-8').splitlines() if line.startswith('case/')]
    if rust != vanilla:
        differing = [(a, b) for a, b in zip(vanilla, rust) if a != b]
        shown = '\n'.join(f'  vanilla {a}\n  rust    {b}' for a, b in differing[:12])
        raise AssertionError(f'{len(differing)} of {len(vanilla)} dragon spawn cases differ:\n{shown}')
    for name, left_handed, following in vanilla[:6]:
        print(f'{name}: vanilla LeftHanded={left_handed} next={following}', flush=True)
    lab.write_json(report, {
        'schema': 1, 'minecraft': lock['minecraft']['version'], 'cases': len(vanilla),
        'left_handed': {'true': sum(flag for _, flag, _ in vanilla),
                        'false': sum(not flag for _, flag, _ in vanilla)},
        'scope': 'EnderDragon Mob.finalizeSpawn through the summon spawn path: the LeftHanded flag and the level '
                 'random draw that follows it, which pin the follow-range triangle and the left-handed roll.'})
    print(f'{len(vanilla)} dragon spawn cases match vanilla 26.3 ({report})', flush=True)


if __name__ == '__main__':
    sys.exit(main())
