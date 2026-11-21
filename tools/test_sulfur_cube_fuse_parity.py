#!/usr/bin/env python3
"""Compare a loaded sulfur cube's fuse, MAX_FUSE and Invulnerable with pinned vanilla.

Java loads each case on a detached ServerLevel through Entity.load (`fuse` absent, at the signed
boundaries, as every numeric tag type and as a string; `Invulnerable` absent, false and true), saves
it, reads the MAX_FUSE a late tracker's snapshot carries, runs tickFuse three times and reads what
packDirty sends to trackers already following it, then what a later load of `fuse:-1` sends them.
Sunrise runs the production load, save, `tick_fuse` and `mob_meta`, with `send_dirty`'s rule for
what goes to existing trackers. Every case is a fresh observation; a missing row fails.
"""
import argparse
import os
from pathlib import Path
import tempfile
try:
    from . import lab, oracle_harness as harness, test_rust_parity as base, integration_env
except ImportError:
    import lab, oracle_harness as harness, test_rust_parity as base, integration_env

CASES = 14 * 3


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--resources', type=Path,
                        default=Path(os.environ.get('SUNRISE_TEST_RESOURCES', lab.RESOURCES)))
    args = parser.parse_args()
    resources = args.resources.resolve()
    if not (resources / 'entity-types.json').is_file():
        raise SystemExit('prepared resources required; pass --resources PATH')
    lock, jars, libraries = base.verify_artifacts()
    build = lab.LOCAL / 'rust-sulfur-cube-fuse-parity'
    build.mkdir(parents=True, exist_ok=True)
    report = build / 'report.json'
    report.unlink(missing_ok=True)
    for name in ['server.txt', 'rust.txt']:
        (build / name).unlink(missing_ok=True)
    java_root = lab.ROOT / 'mods/sunrise/src/test/java'
    oracle = java_root / 'dev/sunriselab/parity/VanillaSulfurCubeFuseOracle.java'
    sources = [Path(__file__), oracle, lab.ROOT / 'tools/lab.py',
               lab.ROOT / 'tools/oracle_harness.py', lab.ROOT / 'tools/test_rust_parity.py',
               lab.ROOT / 'tools/integration_env.py', lab.ROOT / 'Cargo.toml', lab.ROOT / 'Cargo.lock',
               lab.ROOT / 'crates/sunrise-client/src/mobs.rs',
               lab.ROOT / 'crates/sunrise-client/src/entity_nbt.rs',
               lab.ROOT / 'crates/sunrise-client/src/mobs/sulfur_cube.rs',
               lab.ROOT / 'crates/sunrise-client/src/game/network/lan/mod.rs',
               lab.ROOT / 'crates/sunrise-client/src/game/network/lan/sulfur_cube_meta_tests.rs',
               lab.ROOT / 'crates/sunrise-world/src/nbt_path.rs']
    hashes = base.source_hashes(sources)
    classes = build / 'classes'
    classes.mkdir(exist_ok=True)
    dependencies = os.pathsep.join(map(str, [jars['server'], *libraries]))
    harness.run_logged(['javac', '--release', lock['minecraft']['java_major'], '-proc:none',
                        '-sourcepath', java_root, '-cp', dependencies, '-d', classes, oracle],
                       build, 'compile-java')
    harness.run_logged(['java', '-Xmx1G', '-Djava.awt.headless=true', '-cp',
                        str(classes) + os.pathsep + dependencies,
                        'dev.sunriselab.parity.VanillaSulfurCubeFuseOracle', build / 'server.txt'],
                       build, 'server')
    with tempfile.TemporaryDirectory(prefix='sunrise-sulfur-cube-fuse-') as scratch:
        env = integration_env.game_environment(os.environ, scratch)
        env.update(SUNRISE_TEST_RESOURCES=str(resources),
                   SUNRISE_SULFUR_CUBE_FUSE_OUTPUT=str(build / 'rust.txt'))
        env.setdefault('CARGO_BUILD_JOBS', '1')
        env.setdefault('RUST_TEST_THREADS', '2')
        harness.run_logged(['cargo', 'test', '--locked', '-p', 'sunrise-client', '--lib',
                            'a_loaded_sulfur_cube_fuse_matches_the_pinned_oracle'], build, 'rust', env=env)
    matched = harness.compare(build / 'server.txt', build / 'rust.txt', CASES)
    base.ensure_sources_unchanged(sources, hashes)
    lab.write_json(report, dict(schema=1, minecraft=lock['minecraft']['version'],
        sources_sha256=hashes, server_jar_sha256=lab.digest(jars['server'], 'sha256'),
        matched_output_lines=matched, output_sha256=lab.digest(build / 'rust.txt', 'sha256'),
        scope=__doc__))
    print(f'{matched} sulfur cube fuse cases match the pinned server: {report}', flush=True)


if __name__ == '__main__':
    main()
