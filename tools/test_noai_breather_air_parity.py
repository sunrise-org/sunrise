#!/usr/bin/env python3
"""Compare a NoAI axolotl's and nautilus's air across load, tick, save and reload with pinned vanilla.

Java loads each case from NBT with a signed-short `Air`, controls the eye's water and the active
effect queries, places water, bubble columns (at the eye, at the feet, or both) or a suffocating
block at the eye and feet, forces a lethal in-wall hurt where the case asks for it, then runs the
real `LivingEntity.baseTick` until `tickEffects`, saves, reloads that save and ticks again.
Sunrise runs the production NBT load, `base_tick` and save on the same blocks and effects. Every
case is a fresh observation on both sides; a missing row fails the comparison.
"""
import argparse
import os
from pathlib import Path
import tempfile
try:
    from . import lab, oracle_harness as harness, test_rust_parity as base, integration_env
except ImportError:
    import lab, oracle_harness as harness, test_rust_parity as base, integration_env

CASES = 2 * (5 * 8 + 1) * 6


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--resources', type=Path,
                        default=Path(os.environ.get('SUNRISE_TEST_RESOURCES', lab.RESOURCES)))
    args = parser.parse_args()
    resources = args.resources.resolve()
    if not (resources / 'entity-types.json').is_file():
        raise SystemExit('prepared resources required; pass --resources PATH')
    lock, jars, libraries = base.verify_artifacts()
    build = lab.LOCAL / 'rust-noai-breather-air-parity'
    build.mkdir(parents=True, exist_ok=True)
    report = build / 'report.json'
    report.unlink(missing_ok=True)
    for name in ['server.txt', 'rust.txt']:
        (build / name).unlink(missing_ok=True)
    java_root = lab.ROOT / 'mods/sunrise/src/test/java'
    oracle = java_root / 'dev/sunriselab/parity/VanillaNoAiBreatherAirOracle.java'
    sources = [Path(__file__), oracle, lab.ROOT / 'tools/lab.py',
               lab.ROOT / 'tools/oracle_harness.py', lab.ROOT / 'tools/test_rust_parity.py',
               lab.ROOT / 'tools/integration_env.py', lab.ROOT / 'Cargo.toml', lab.ROOT / 'Cargo.lock',
               lab.ROOT / 'crates/sunrise-client/src/mobs.rs',
               lab.ROOT / 'crates/sunrise-client/src/breathing.rs',
               lab.ROOT / 'crates/sunrise-client/src/entity_nbt.rs',
               lab.ROOT / 'crates/sunrise-client/src/mobs/ai.rs',
               lab.ROOT / 'crates/sunrise-client/src/mobs/living.rs',
               lab.ROOT / 'crates/sunrise-client/src/mobs/world.rs',
               java_root / 'dev/sunriselab/parity/ParityData.java',
               java_root / 'dev/sunriselab/parity/VanillaBlockUpdateOracle.java']
    hashes = base.source_hashes(sources)
    classes = build / 'classes'
    classes.mkdir(exist_ok=True)
    dependencies = os.pathsep.join(map(str, [jars['server'], *libraries]))
    harness.run_logged(['javac', '--release', lock['minecraft']['java_major'], '-proc:none',
                        '-sourcepath', java_root, '-cp', dependencies, '-d', classes, oracle],
                       build, 'compile-java')
    harness.run_logged(['java', '-Xmx512M', '-Djava.awt.headless=true', '-cp',
                        str(classes) + os.pathsep + dependencies,
                        'dev.sunriselab.parity.VanillaNoAiBreatherAirOracle', build / 'server.txt'],
                       build, 'server')
    with tempfile.TemporaryDirectory(prefix='sunrise-noai-breather-air-') as scratch:
        env = integration_env.game_environment(os.environ, scratch)
        env.update(SUNRISE_TEST_RESOURCES=str(resources),
                   SUNRISE_NOAI_BREATHER_AIR_OUTPUT=str(build / 'rust.txt'))
        env.setdefault('CARGO_BUILD_JOBS', '1')
        env.setdefault('RUST_TEST_THREADS', '2')
        harness.run_logged(['cargo', 'test', '--locked', '-p', 'sunrise-client-lib', '--lib',
                            'mobs::ai::noai_breather_air_tests::noai_breathers_match_the_pinned_oracle',
                            '--', '--exact', '--ignored'], build, 'rust', env=env)
    matched = harness.compare(build / 'server.txt', build / 'rust.txt', CASES)
    base.ensure_sources_unchanged(sources, hashes)
    lab.write_json(report, dict(schema=1, minecraft=lock['minecraft']['version'],
        sources_sha256=hashes, server_jar_sha256=lab.digest(jars['server'], 'sha256'),
        matched_output_lines=matched, output_sha256=lab.digest(build / 'rust.txt', 'sha256'),
        scope=__doc__))
    print(f'{matched} NoAI breather air cases match the pinned server: {report}', flush=True)


if __name__ == '__main__':
    main()
