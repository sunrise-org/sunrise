#!/usr/bin/env python3
"""Track a range-zero marker and marked hurt on dedicated servers with vanilla's client codecs.

Run Minecraft 26.3 and Sunrise with copies of the same vanilla-generated world. A marker exists
at an observer's exact x/z but must never be paired. A bat and a player, neither of which tracks
velocity deltas, must each send marked velocity after a non-knockback hit. Compare ordered packets
per client and the velocity bytes verbatim, omitting only the run-specific entity ID.
"""
import argparse
from pathlib import Path
import sys

try:
    from . import test_server_knockback_parity as net
except ImportError:
    import test_server_knockback_parity as net

NAME = 'server-entity-tracking-parity'
A, V = net.A, net.V
CASES = {
    'zero-range': [
        # Same x/z as Alice, with a positive-range control. Query the marker on the
        # server to distinguish a successful summon with no pairing from a failed summon.
        f'cmd Alice summon minecraft:armor_stand {A[0]} {A[1]} {A[2]} {{NoGravity:1b}}',
        f'cmd Alice summon minecraft:marker {A[0]} {A[1]} {A[2]}',
        'present marker', 'ticks 4',
    ],
    'marked-hurt': [
        f'cmd Alice summon minecraft:bat {V[0]} {V[1] + 3} {V[2]} {{NoAI:1b,NoGravity:1b}}',
        'ticks 4', 'cmd Alice damage @e[type=minecraft:bat,limit=1] 1 minecraft:generic',
        'ticks 4', 'cmd Alice damage Bob 1 minecraft:generic', 'ticks 4',
    ],
}
RECIPIENTS = {('Alice', 'bat'), ('Bob', 'bat'), ('Alice', 'Bob'), ('Bob', 'self')}


def capture(lines):
    relevant = {case: [] for case in CASES}
    for line in lines:
        who, step, event = line.split(' ', 2)
        if event.startswith(('error', 'disconnected')):
            raise AssertionError(f'{who} failed while recording {step}')
        if step == 'step=zero-range' and event in ('spawn armor_stand', 'spawn marker'):
            relevant['zero-range'].append((who, event))
        if step == 'step=marked-hurt' and event.startswith('motion-bytes '):
            _, target, velocity = event.split()
            if (who, target) in RECIPIENTS:
                relevant['marked-hurt'].append((who, target, velocity))
    return relevant


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--sunrise-binary', type=Path, help='use an existing dedicated game build')
    parser.add_argument('--recording-dir', type=Path, default=net.lab.LOCAL / NAME)
    args = parser.parse_args()
    lock, jars, libraries = net.base.verify_artifacts()
    build = args.recording_dir.resolve()
    build.mkdir(parents=True, exist_ok=True)
    report = build / 'report.json'
    report.unlink(missing_ok=True)
    vanilla_server, sunrise_server, client = net.harness_commands(lock, jars, libraries, build, args.sunrise_binary)
    world = net.generate_world(vanilla_server, build)
    vanilla = capture(net.run_side('vanilla', vanilla_server, client, build, world, CASES, tracking=True))
    # A red or empty fixture is not a parity result. Prove the reference exercised every recipient.
    if vanilla['zero-range'] != [(who, 'spawn armor_stand') for who in net.PLAYERS]:
        raise AssertionError('vanilla did not record the positive-range control without the marker')
    if sorted((who, target) for who, target, _ in vanilla['marked-hurt']) != sorted(RECIPIENTS):
        raise AssertionError('vanilla did not record a marked motion for each expected recipient')
    sunrise = capture(net.run_side('sunrise', sunrise_server, client, build, world, CASES, tracking=True))
    results = {case: 'pass' if sunrise[case] == vanilla[case] else 'fail' for case in CASES}
    version = lock['minecraft']['version']
    net.lab.write_json(report, dict(schema=1, minecraft=version, recording=NAME, cases=results,
                                    server_jar_sha256=net.lab.digest(jars['server'], 'sha256'),
                                    vanilla_sha256=net.lab.digest(build / 'vanilla.txt', 'sha256'),
                                    sunrise_sha256=net.lab.digest(build / 'sunrise.txt', 'sha256')))
    for case, result in results.items():
        print(f'{NAME} ({version}): {case}: {result}')
    if 'fail' in results.values():
        raise AssertionError('tracking packets differ (see the private recordings)')


if __name__ == '__main__':
    try:
        main()
    except AssertionError as error:
        sys.exit(f'{NAME}: {error}')
