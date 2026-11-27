#!/usr/bin/env python3
"""26.3's own chunk sending on a scripted readiness schedule (the JVM kit), checked against the real server first.

`mods/sunrise/src/test/java/dev/sunriselab/parity/VanillaChunkSendOracle.java` drives the pinned jar's own
PlayerChunkSender and ChunkMap (onChunkReadyToSend, applyChunkTrackingView, markChunkPendingToSend, dropChunk,
getChunkToSend, sendNextChunks, onChunkBatchReceivedByClient) for one player, from a script that says which chunks
become ready when, where the player moves and when the client acknowledges a batch. One method is rewritten
(`ChunkSendAgent`: PlayerChunkSender.sendChunk records the chunk instead of building its packet); the oracle's
javadoc lists what else is stubbed or built with allocateInstance.

The kit has to match the real server before its scripts mean anything. The sanity step runs the jar's dedicated
server on the world and steps `tools/test_chunk_send_parity.py` uses, with the same recording client, and replays
them in the kit: every chunk of the forced square ready from the start, the join and the held step's
acknowledgements held, then one acknowledgement a tick at 64 chunks a tick (the client's `--ack-mode 64`). For the
join, the held step and every order step (their views inside the square, where readiness is the square's), the
kit's centres, chunks, forgets and batch sizes must be the jar's, packet for packet. Edge steps are left out: past the
square, when a chunk is ready is the server's own loading, which the kit is told rather than finds.

Then the cases (`crates/sunrise-client/src/lan/hub/chunk_send_cases.json`): scripts with a controlled readiness
schedule, each with the packets 26.3 sends for it (`vanilla`), which the hub's own test
(`lan::hub::tests::the_kit_cases_send_what_26_3_sends`) holds Sunrise to. The kit must give each case's `vanilla`
lines exactly. `--write-cases` fills them in from the kit instead (a new or changed script), into the work folder
and as one `CASES_JSON` line, for the file.

    python3 tools/test_chunk_send_kit_parity.py
    python3 tools/test_chunk_send_kit_parity.py --keep     # the work folder stays in .local/chunk-send-kit-parity
    python3 tools/test_chunk_send_kit_parity.py --write-cases   # the cases' vanilla lines from the kit
"""
import argparse
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import zipfile

sys.path.insert(0, str(Path(__file__).resolve().parent))
import lab  # noqa: E402
import oracle_harness as oh  # noqa: E402
import test_chunk_send_parity as jar_harness  # noqa: E402
import test_rust_parity as base  # noqa: E402

NAME = 'chunk-send-kit-parity'
ORACLE = 'VanillaChunkSendOracle'
AGENT_SOURCE = lab.ROOT / 'mods/sunrise/src/test/java/dev/sunriselab/parity/ChunkSendAgent.java'
AGENT_MANIFEST = 'Manifest-Version: 1.0\r\nPremain-Class: dev.sunriselab.parity.ChunkSendAgent\r\n\r\n'
STAMP = (1980, 1, 1, 0, 0, 0)
# Ticks a step is given to settle in the kit: a view of radius 4 sends at most a few batches of 64.
SETTLE_TICKS = 8


def agent_option(build, release):
    """The agent jar, built from ChunkSendAgent; its -javaagent option."""
    classes = build / 'agent-classes'
    shutil.rmtree(classes, ignore_errors=True)
    oh.run_logged(['javac', '--release', str(release), '-proc:none', '-d', classes, AGENT_SOURCE], build,
                  'compile-java-agent')
    jar = build / 'chunk-send-agent.jar'
    with zipfile.ZipFile(jar, 'w') as archive:
        archive.writestr(zipfile.ZipInfo('META-INF/MANIFEST.MF', STAMP), AGENT_MANIFEST)
        for path in sorted(classes.rglob('*.class')):
            archive.writestr(zipfile.ZipInfo(path.relative_to(classes).as_posix(), STAMP), path.read_bytes())
    return f'-javaagent:{jar}'


CASES = lab.ROOT / 'crates/sunrise-client/src/lan/hub/chunk_send_cases.json'


def kit_lines(build, classes, dependencies, agent, script, name):
    """The kit's output lines for `script`, as the oracle prints them."""
    path = build / f'{name}.json'
    path.write_text(json.dumps(script))
    result = subprocess.run(['java', '-Xmx2g', '-Djava.awt.headless=true', agent, '-cp',
                             str(classes) + os.pathsep + dependencies, 'dev.sunriselab.parity.' + ORACLE, str(path)],
                            capture_output=True, text=True, timeout=600)
    (build / f'{name}.log').write_text(result.stdout + result.stderr)
    if result.returncode != 0:
        raise AssertionError(f'kit {name} failed ({result.returncode}): {result.stderr[-2000:]}')
    return result.stdout.splitlines()


def cases(build, classes, dependencies, agent, write):
    """Problems: a case whose kit lines aren't its `vanilla` lines (or, writing, none: the lines are written)."""
    data = json.loads(CASES.read_text())
    problems = []
    for case in data:
        lines = kit_lines(build, classes, dependencies, agent, case['script'], 'case-' + case['name'])
        print(f'case {case["name"]}: {sum(l.startswith("chunk ") for l in lines)} chunks, '
              f'{sum(l.startswith("forget ") for l in lines)} forgets, {sum(l.startswith("batch ") for l in lines)} batches')
        if write:
            case['vanilla'] = lines
        elif case.get('vanilla') != lines:
            got = case.get('vanilla') or []
            at = next((i for i, (a, b) in enumerate(zip(got, lines)) if a != b), min(len(got), len(lines)))
            problems.append(f'case {case["name"]}: the file and the kit part at line {at}: file {got[at:at + 4]} '
                            f'kit {lines[at:at + 4]}')
    if write:
        (build / CASES.name).write_text('[\n' + ',\n'.join(json.dumps(c) for c in data) + '\n]\n')
        print('CASES_JSON', json.dumps(data))
    return problems


def run_kit(build, classes, dependencies, agent, script, name):
    """The kit's packets for `script`: [(tick, [(kind, value)])], kinds as chunk_packets gives them plus 'start'."""
    ticks = []
    for line in kit_lines(build, classes, dependencies, agent, script, name):
        kind, *rest = line.split()
        if kind == 'tick':
            ticks.append((int(rest[0]), []))
        elif kind in ('center', 'chunk', 'forget'):
            ticks[-1][1].append((kind, (int(rest[0]), int(rest[1]))))
        elif kind == 'batch':
            ticks[-1][1].append(('batch', int(rest[0])))
        elif kind == 'start':
            ticks[-1][1].append(('start', None))
        else:
            raise AssertionError(f'kit {name}: unexpected line {line!r}')
    return ticks


def sanity_script(join):
    """The jar harness's run as a kit script: the forced square ready, the join and the held step held, then one
    acknowledgement a tick at 64 chunks a tick, each step given SETTLE_TICKS ticks."""
    f = jar_harness.FORCED
    ready = [[x, z] for x in range(-f, f + 1) for z in range(-f, f + 1)]
    ticks = [{}]
    released = False
    for kind, x, z in jar_harness.STEPS:
        if kind == 'edge':
            break
        ticks.append({'packets': ([{'ack': 64.0}] if released else []) + [{'move': [x, z]}]})
        if kind == 'held':
            # Held: the move shows its forgets and silent drops with nothing more sent; then the hold lifts.
            ticks.append({})
            released = True
        ticks += [{'packets': [{'ack': 64.0}]} for _ in range(SETTLE_TICKS)]
    return {'memory': False, 'view_distance': jar_harness.VIEW, 'ready': ready, 'join': list(join), 'ticks': ticks}


def by_centre(ticks):
    """The kit's packets as the jar harness groups the client's: after each centre, without batch starts."""
    flat = [(k, v) for _, packets in ticks for k, v in packets if k != 'start']
    return jar_harness.steps_of(flat)


def sanity(build, classes, dependencies, agent):
    """Problems: where the kit's sends part from the jar's on the jar harness's own run."""
    _, jars, libraries = base.verify_artifacts()
    oh.run_logged(['cargo', 'build', '--profile', 'fast', '-p', 'sunrise-bots', '--bin', 'sunrise-bots'], build,
                  'compile-rust')
    exe = '.exe' if os.name == 'nt' else ''
    bots = lab.cargo_target() / 'fast' / f'sunrise-bots{exe}'
    server = ['java', '-Xmx2g', '-Duser.timezone=UTC', '-cp', os.pathsep.join(map(str, [jars['server'], *libraries])),
              'net.minecraft.server.Main', 'nogui']
    world = jar_harness.generate(build, server)
    real = jar_harness.run_side('vanilla', server, build, world, bots)
    if not real:
        return ['the jar sent no centre']
    kit = by_centre(run_kit(build, classes, dependencies, agent, sanity_script(real[0][0]), 'sanity'))
    checked = 1 + sum(1 for kind, _, _ in jar_harness.STEPS if kind != 'edge')
    problems = []
    for i, ((rc, r), (kc, k)) in enumerate(zip(real[:checked], kit[:checked])):
        if rc != kc or r != k:
            at = next((j for j, (a, b) in enumerate(zip(r, k)) if a != b), min(len(r), len(k)))
            problems.append(f'step {i} (jar centre {rc}, kit centre {kc}): {len(r)} jar packets, {len(k)} kit; '
                            f'they part at {at}: jar {r[at:at + 6]} kit {k[at:at + 6]}')
    if len(kit) < checked:
        problems.append(f'the kit has {len(kit)} centres, {checked} checked')
    for (rc, r) in real[:checked]:
        print(f'sanity centre {rc}: jar sent {sum(k == "chunk" for k, _ in r)} chunks, '
              f'forgot {sum(k == "forget" for k, _ in r)}, {sum(k == "batch" for k, _ in r)} batches')
    return problems


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--keep', action='store_true', help='keep the work folder')
    parser.add_argument('--write-cases', action='store_true', help="fill in the cases' vanilla lines from the kit")
    args = parser.parse_args()
    lock, jars, libraries = base.verify_artifacts()
    build = lab.LOCAL / NAME
    shutil.rmtree(build, ignore_errors=True)
    build.mkdir(parents=True)
    release = lock['minecraft']['java_major']
    classes, dependencies, _ = oh.compile_oracle(build, ORACLE, jars, libraries, release)
    agent = agent_option(build, release)
    problems = sanity(build, classes, dependencies, agent)
    problems += cases(build, classes, dependencies, agent, args.write_cases)
    for p in problems:
        print('DIFFERS', p)
    if not args.keep:
        shutil.rmtree(build, ignore_errors=True)
    print('chunk-send kit parity:', 'pass' if not problems else f'{len(problems)} differences')
    return 1 if problems else 0


if __name__ == '__main__':
    sys.exit(main())
