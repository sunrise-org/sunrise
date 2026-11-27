#!/usr/bin/env python3
"""Compare Sunrise's environment attributes with the vanilla 26.3 attribute system.

The Java oracle builds vanilla's `EnvironmentAttributeSystem` from the data pack
(dimension attributes, one biome's attributes, the dimension timelines and the
weather layers) and prints every visual/gameplay attribute for a grid of
dimensions, world times, rain/thunder levels and biomes.
`examples/probe/environment_fixture` samples the same cases through Sunrise's
`Environment::sample` + `apply_weather`; every attribute Sunrise computes must
agree with vanilla, bit for bit: both sides print binary32 values (Float.toString,
Rust's `{:?}` of f32), compared on their bits.

  test_environment_render_parity.py
"""
import argparse
import json
import os
from pathlib import Path
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))
from tools import lab, test_rust_parity as base, test_model_baking_parity as baking


def cases(resources):
    biomes = sorted(p.stem for p in (resources / 'data/minecraft/worldgen/biome').glob('*.json'))
    out = []
    for dim in ['overworld', 'the_nether', 'the_end']:
        for ticks in list(range(0, 48000, 250)) + [23999, 24001, 1234567]:
            for rain, thunder in [(0, 0), (0.5, 0), (1, 0), (1, 1), (0.7, 0.3)] if dim == 'overworld' else [(0, 0)]:
                out.append(f'{dim} {ticks} {rain} {thunder} plains')
    own = {'the_nether': {'nether_wastes', 'soul_sand_valley', 'crimson_forest', 'warped_forest', 'basalt_deltas'},
           'the_end': {'the_end', 'small_end_islands', 'end_midlands', 'end_highlands', 'end_barrens'}}
    for dim in ['overworld', 'the_nether', 'the_end']:
        for biome in biomes:
            if dim in own and biome not in own[dim]:
                continue
            if dim == 'overworld' and any(biome in b for b in own.values()):
                continue
            for ticks in (0, 6000, 12500, 18000):
                for rain, thunder in [(0, 0), (1, 0.4)] if dim == 'overworld' else [(0, 0)]:
                    out.append(f'{dim} {ticks} {rain} {thunder} {biome}')
    return out


# The attributes `environment_fixture` prints, every one compared on every row; a vanilla attribute
# outside this set is reported as not modeled, one Sunrise prints outside it fails.
MODELED = frozenset('minecraft:' + name for name in [
    'visual/sky_color', 'visual/fog_color', 'visual/cloud_color', 'visual/sunrise_sunset_color',
    'visual/sky_light_factor', 'visual/sky_light_color', 'visual/block_light_tint',
    'visual/ambient_light_color', 'visual/sun_angle', 'visual/moon_angle', 'visual/star_angle',
    'visual/star_brightness', 'gameplay/sky_light_level', 'visual/fog_start_distance',
    'visual/fog_end_distance', 'visual/cloud_fog_end_distance', 'visual/cloud_height'])


def compare_rows(vanilla, rust):
    """({attribute: [(case, vanilla, Sunrise, distances or reason)]}, vanilla-only attributes)."""
    bad, unmodeled = {}, set()
    for v, r in zip(vanilla, rust):
        case = v.get('case')
        for key in sorted((set(v) | set(r)) - {'case'}):
            if key not in MODELED:
                if key in r:
                    bad.setdefault(key, []).append((case, v.get(key), r[key], 'not in MODELED'))
                else:
                    unmodeled.add(key)
                continue
        for key in sorted(MODELED):
            if key not in v or key not in r:
                side = 'vanilla' if key not in v else 'Sunrise'
                bad.setdefault(key, []).append((case, v.get(key), r.get(key), f'missing on the {side} side'))
                continue
            expected, got = v[key], r[key]
            if isinstance(expected, list) != isinstance(got, list) or isinstance(expected, list) and len(expected) != len(got):
                bad.setdefault(key, []).append((case, expected, got, 'shape'))
                continue
            pairs = list(zip(expected, got) if isinstance(expected, list) else [(expected, got)])
            if any(isinstance(x, (bool, str)) or x is None for pair in pairs for x in pair):
                distances = [None if a == b and type(a) is type(b) else 'differs' for a, b in pairs]
            else:
                distances = [lab.float_difference(a, b) for a, b in pairs]
            if any(distances):
                bad.setdefault(key, []).append((case, expected, got, distances))
    return bad, unmodeled


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.parse_args()
    lock, jars, libraries = base.verify_artifacts()
    build = lab.LOCAL / 'rust-environment-parity'; build.mkdir(parents=True, exist_ok=True)
    resources = lab.RESOURCES
    inputs = build / 'inputs.txt'; inputs.write_text('\n'.join(cases(resources)) + '\n')
    classes = build / 'classes'; classes.mkdir(exist_ok=True)
    oracle = lab.ROOT / 'mods/sunrise/src/test/java/dev/sunriselab/parity/VanillaEnvironmentOracle.java'
    deps = os.pathsep.join(map(str, libraries))

    def run(command, name):
        with (build / (name + '.log')).open('w') as log:
            result = subprocess.run(list(map(str, command)), cwd=lab.ROOT, timeout=3600, stdout=log, stderr=subprocess.STDOUT)
        if result.returncode:
            print((build / (name + '.log')).read_text()[-4000:]); sys.exit(f'{name} failed')
    run(['javac', '--release', lock['minecraft']['java_major'], '-proc:none', '-sourcepath', lab.ROOT / 'mods/sunrise/src/test/java',
         '-cp', str(jars['server']) + os.pathsep + deps, '-d', classes, oracle], 'compile-java')
    run(['java', '-Xmx1G', '-cp', os.pathsep.join([str(classes), str(jars['server']), deps]),
         'dev.sunriselab.parity.VanillaEnvironmentOracle', inputs, build / 'vanilla.jsonl'], 'vanilla')
    run(['cargo', 'run', '--quiet', '--profile', 'fast', '-p', 'sunrise-client-lib', '--example', 'client_probe', '--',
         'environment_fixture', resources, inputs, build / 'rust.jsonl'], 'rust')
    vanilla = [lab.exact_json(l) for l in (build / 'vanilla.jsonl').read_text().splitlines()]
    rust = [lab.exact_json(l) for l in (build / 'rust.jsonl').read_text().splitlines()]
    assert len(vanilla) == len(rust), (len(vanilla), len(rust))
    bad, uncovered = compare_rows(vanilla, rust)
    print(f'{len(vanilla)} cases compared')
    for key, items in sorted(bad.items()):
        print(f'MISMATCH {key}: {len(items)} cases (case, vanilla, Sunrise, distance per component), e.g.')
        for item in items[:3]:
            print(f'  {item}')
    if uncovered:
        print('vanilla attributes Sunrise does not model:', ', '.join(sorted(uncovered)))
    if bad:
        sys.exit(f'{len(bad)} attributes differ from vanilla')
    print('environment parity ok')


if __name__ == '__main__':
    main()
