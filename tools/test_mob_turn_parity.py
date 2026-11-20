#!/usr/bin/env python3
"""Compare the rotations four turning paths write with pinned vanilla, as raw float bits.

Java (VanillaTurnOracle) runs the real Fox$FoxPounceGoal.tick and DolphinJumpGoal.tick from a set
of motions, pitches and (for the fox) its jumping flag; DragonSittingScanningPhase.doServerTick
with its scan's player at a set of places, from a set of yaws, yRotA values and head positions;
and Entity.lookAt(FEET) to a set of offsets, including straight up and down and the eye itself.
Sunrise runs the production fox Pounce and dolphin Jump goal ticks, the dragon's scanning turn and
the client's look-at rotation on the same inputs. Every case is a fresh observation on both sides,
and each of the four outputs must be complete.
"""
import argparse
import os
from pathlib import Path
import tempfile
try:
    from . import lab, oracle_harness as harness, test_rust_parity as base, integration_env
except ImportError:
    import lab, oracle_harness as harness, test_rust_parity as base, integration_env

OUTPUTS = {'fox.txt': 13 * 3 * 2, 'dolphin.txt': 13 * 3, 'dragon.txt': 7 * 5 * 3 * 2, 'look_at.txt': 36}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--resources', type=Path,
                        default=Path(os.environ.get('SUNRISE_TEST_RESOURCES', lab.RESOURCES)))
    args = parser.parse_args()
    resources = args.resources.resolve()
    if not (resources / 'entity-types.json').is_file():
        raise SystemExit('prepared resources required; pass --resources PATH')
    lock, jars, libraries = base.verify_artifacts()
    build = lab.LOCAL / 'rust-mob-turn-parity'
    build.mkdir(parents=True, exist_ok=True)
    report = build / 'report.json'
    report.unlink(missing_ok=True)
    server, rust = build / 'server', build / 'rust'
    for folder in [server, rust]:
        folder.mkdir(exist_ok=True)
        for name in OUTPUTS:
            (folder / name).unlink(missing_ok=True)
    java_root = lab.ROOT / 'mods/sunrise/src/test/java'
    oracle = java_root / 'dev/sunriselab/parity/VanillaTurnOracle.java'
    sources = [Path(__file__), oracle, lab.ROOT / 'tools/lab.py',
               lab.ROOT / 'tools/oracle_harness.py', lab.ROOT / 'tools/test_rust_parity.py',
               lab.ROOT / 'tools/integration_env.py', lab.ROOT / 'Cargo.toml', lab.ROOT / 'Cargo.lock',
               lab.ROOT / 'crates/sunrise-client/src/mobs/fox.rs',
               lab.ROOT / 'crates/sunrise-client/src/mobs/dolphin.rs',
               lab.ROOT / 'crates/sunrise-client/src/game/mobs/dragon.rs',
               lab.ROOT / 'crates/sunrise-client/src/game/network/remote/mod.rs',
               lab.ROOT / 'crates/sunrise-client/src/game/network/remote/client_read_tests.rs',
               java_root / 'dev/sunriselab/parity/ParityData.java',
               java_root / 'dev/sunriselab/parity/VanillaBlockUpdateOracle.java']
    hashes = base.source_hashes(sources)
    classes = build / 'classes'
    classes.mkdir(exist_ok=True)
    dependencies = os.pathsep.join(map(str, [jars['server'], *libraries]))
    harness.run_logged(['javac', '--release', lock['minecraft']['java_major'], '-proc:none',
                        '-sourcepath', java_root, '-cp', dependencies, '-d', classes, oracle],
                       build, 'compile-java')
    harness.run_logged(['java', '-Xmx1G', '-Djava.awt.headless=true', '-cp',
                        str(classes) + os.pathsep + dependencies,
                        'dev.sunriselab.parity.VanillaTurnOracle', server], build, 'server')
    with tempfile.TemporaryDirectory(prefix='sunrise-mob-turn-') as scratch:
        env = integration_env.game_environment(os.environ, scratch)
        env.update(SUNRISE_TEST_RESOURCES=str(resources), SUNRISE_TURN_OUTPUT_DIR=str(rust))
        env.setdefault('CARGO_BUILD_JOBS', '1')
        env.setdefault('RUST_TEST_THREADS', '2')
        harness.run_logged(['cargo', 'test', '--locked', '-p', 'sunrise-client-lib', '--lib',
                            'turn_matches_the_pinned_oracle', '--', '--ignored'], build, 'rust-lib', env=env)
        harness.run_logged(['cargo', 'test', '--locked', '-p', 'sunrise-client', '--lib',
                            'matches_the_pinned_oracle'], build, 'rust-game', env=env)
    matched = {name: harness.compare(server / name, rust / name, lines) for name, lines in OUTPUTS.items()}
    base.ensure_sources_unchanged(sources, hashes)
    lab.write_json(report, dict(schema=1, minecraft=lock['minecraft']['version'],
        sources_sha256=hashes, server_jar_sha256=lab.digest(jars['server'], 'sha256'),
        matched_output_lines=matched, scope=__doc__))
    print(f'{sum(matched.values())} turn cases match the pinned server: {report}', flush=True)


if __name__ == '__main__':
    main()
