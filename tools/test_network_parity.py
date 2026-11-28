#!/usr/bin/env python3
"""Compare play packet bundles, post_effects lifecycle, strict entity_data decoding, and a new player's
sent input and sprint with vanilla 26.3.

Compiles and runs VanillaNetworkParityOracle against the pinned 26.3 client jar on JDK 25,
writing bundle.json, post_effects.json, entity_data.json, sent_input.json, and meta.json into a private witness
directory, then runs each Rust parity test in its own process via SUNRISE_NETWORK_WITNESS_DIR."""
import argparse
import contextlib
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))
from tools import lab, oracle_harness as harness, test_rust_parity as base, test_model_baking_parity as baking
from tools.ci import ignored_tests

ORACLE = 'VanillaNetworkParityOracle'

# The daily rows that read this harness's witnesses: the standalone run takes its tests from them, so it runs the
# exact names the daily shards run.
ROW_FILES = ('daily-network-parity.txt', 'daily-sunrise-client-new-player-sent-input.txt')

WITNESS_FILES = ('bundle.json', 'post_effects.json', 'entity_data.json', 'sent_input.json')


def tests_by_package():
    """{package: [exact test name, ...]} from ROW_FILES' `lib` daily rows, in row order."""
    files = {f'{ignored_tests.POLICY_PATH}/{name}' for name in ROW_FILES}
    tests, seen = {}, set()
    for row in ignored_tests.listed_entries():
        if row.file in files:
            assert row.mark == 'daily' and row.reference == 'lib', f'{row.file}: {row.test} is not a lib daily row'
            tests.setdefault(row.package, []).append(row.test)
            seen.add(row.file)
    assert seen == files, f'no rows in {sorted(files - seen)}'
    return tests


def record(lock, jars, libraries, build, witness_dir=None):
    version = lock['minecraft']['version']
    witness_dir = witness_dir or (lab.LOCAL / 'witness' / version / 'network')
    witness_dir.mkdir(parents=True, exist_ok=True)
    classes, dependencies, oracle_path = harness.compile_oracle(
        build, ORACLE, jars, libraries, lock['minecraft']['java_major']
    )
    harness.java_run(build, classes, dependencies, ORACLE, [str(witness_dir.resolve())], 'client')
    log = (build / 'client.log').read_text(encoding='utf-8', errors='replace')
    assert 'NETWORK_PARITY_WITNESS_OK' in log, log
    java_ver = subprocess.check_output(['java', '--version'], text=True).splitlines()[0].strip()
    lab.write_json(witness_dir / 'meta.json', dict(
        minecraft=version,
        client_jar_sha256=lab.digest(jars['client'], 'sha256'),
        java_version=java_ver,
        command='python3 tools/test_network_parity.py',
        oracle=f'mods/sunrise/src/test/java/dev/sunriselab/parity/{ORACLE}.java',
        inputs_sha256={
            'oracle': lab.digest(oracle_path, 'sha256'),
        },
    ))
    return witness_dir


def prepare_witness(work):
    """Generate the private vanilla witness JSONs for daily CI shards without running Cargo."""
    work = work.resolve()
    work.mkdir(parents=True, exist_ok=True)
    build = work / 'build'
    build.mkdir(parents=True, exist_ok=True)
    for name in WITNESS_FILES:
        (work / name).unlink(missing_ok=True)
    log = work / 'prepare-witness.log'
    with log.open('w', encoding='utf-8') as output:
        with contextlib.redirect_stdout(output), contextlib.redirect_stderr(output):
            try:
                lock, jars, _ = base.verify_artifacts()
                libraries = baking.client_libraries()
                record(lock, jars, libraries, build, witness_dir=work)
            except BaseException as exc:
                print(exc, file=output, flush=True)
                raise SystemExit(f'Network witness producer failed; private log: {log}') from None
    if not all((work / name).is_file() for name in WITNESS_FILES):
        raise SystemExit('Network witness producer did not write all required witness files')
    return {'SUNRISE_NETWORK_WITNESS_DIR': str(work)}


def run_logged(command, build, name, cwd=lab.ROOT, env=None):
    path = build / f'{name}.log'
    with path.open('w', encoding='utf-8') as log:
        result = subprocess.run([str(c) for c in command], cwd=cwd, env=env, timeout=3600,
                                stdout=log, stderr=subprocess.STDOUT)
    if result.returncode:
        tail = path.read_text(encoding='utf-8', errors='replace').splitlines()[-40:]
        print('\n'.join(tail), file=sys.stderr, flush=True)
        sys.exit(f'{name} failed: see {path}')
    return path


def build_test_binary(build, package, target_name, env):
    compiled = run_logged(
        ['cargo', 'test', '--locked', '-p', package, '--lib', '--no-run', '--message-format=json'],
        build, f'compile-{package}', env=env,
    )
    binaries = []
    for line in compiled.read_text(encoding='utf-8').splitlines():
        if not line.startswith('{'):
            continue
        row = json.loads(line)
        if (row.get('reason') == 'compiler-artifact' and row.get('executable')
                and row.get('target', {}).get('name') == target_name and row.get('profile', {}).get('test')):
            binaries.append(Path(row['executable']))
    assert len(binaries) == 1, f'expected one {target_name} test binary, found {binaries}'
    return binaries[0]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--resources', type=Path, default=Path(os.environ.get('SUNRISE_TEST_RESOURCES', lab.RESOURCES)))
    parser.add_argument('--record-only', action='store_true')
    parser.add_argument('--prepare-witness', type=Path,
                        help='generate only the private vanilla witness JSONs in this directory')
    parser.add_argument('--env-file', type=Path,
                        help='append SUNRISE_NETWORK_WITNESS_DIR to an environment file')
    args = parser.parse_args()
    if args.prepare_witness:
        if not args.env_file:
            parser.error('--prepare-witness requires --env-file')
        environment = prepare_witness(args.prepare_witness)
        with args.env_file.open('a', encoding='utf-8') as output:
            for key, value in environment.items():
                output.write(f'{key}={value}\n')
        return
    if args.env_file:
        parser.error('--env-file requires --prepare-witness')

    lock, jars, _ = base.verify_artifacts()
    libraries = baking.client_libraries()
    build = lab.LOCAL / 'network-parity'
    build.mkdir(parents=True, exist_ok=True)
    witness_dir = record(lock, jars, libraries, build)
    if args.record_only:
        print(f'wrote network witnesses under {witness_dir}')
        return
    with tempfile.TemporaryDirectory(prefix='sunrise-network-parity-') as tmp:
        scratch = Path(tmp)
        homes = {k: scratch / k for k in ('SUNRISE_HOME', 'SUNRISE_LAUNCHER_HOME', 'XDG_DATA_HOME', 'XDG_CONFIG_HOME')}
        env = lab.game_env(os.environ, **homes, RUST_MIN_STACK='16777216')
        env['SUNRISE_TEST_RESOURCES'] = str(args.resources.resolve())
        env['SUNRISE_NETWORK_WITNESS_DIR'] = str(witness_dir.resolve())

        total = 0
        for package, tests in tests_by_package().items():
            binary = build_test_binary(build, package, package.replace('-', '_'), env)
            for idx, test in enumerate(tests):
                log = run_logged(
                    [binary, '--ignored', '--exact', '--test-threads=1', test],
                    build, f'{package}-{idx}', cwd=scratch, env=env,
                )
                assert '1 passed; 0 failed' in log.read_text(encoding='utf-8'), (
                    f'expected "1 passed; 0 failed" for {test} in {log}'
                )
                total += 1

    print(f'Network parity passed ({total} tests against {witness_dir})')


if __name__ == '__main__':
    main()
