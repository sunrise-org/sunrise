#!/usr/bin/env python3
"""Which chunks a server sends a player, and in what order, against the pinned 26.3 server jar.

The jar first generates a flat world (fixed seed, respawn_radius 0, nothing spawning, no random ticks) with a square
of FORCED chunks kept loaded by vanilla's own /forceload, and saves it. The jar and Sunrise's dedicated server
(`sunrise-client --dedicated --nogui`) then serve copies of it with the same server.properties (view distance 4). One
recording client (`sunrise-bots --behaviour stand`: it stays where the server puts it, acknowledges every chunk batch
at a fixed 64 chunks a tick, and records every play packet it gets) joins each, and the console teleports it through
STEPS:

- "order" steps keep the whole view inside the forced square. Every chunk the player tracks is loaded and ready
  the moment it is tracked, so nothing about the timing of chunk loading enters: the chunks sent and forgotten for
  the step, in their order, must be the same packet for packet (PlayerChunkSender: the nearest ready chunks first,
  ties in the order its pending set gives them).
- The "held" step comes first: the client joins holding its batch acknowledgements back (`--hold-acks`), so after
  the first batch PlayerChunkSender sends nothing more and the view's other chunks stay pending, ready but unsent.
  The harness waits for that first finished batch, then the console moves the client 7 chunks, far enough that
  chunks of both kinds leave the view: each still pending must leave silently, each sent must be forgotten
  (PlayerChunkSender.dropChunk). It waits for the new centre and the held output (the forgets, nothing sent), then
  lifts the hold and waits for the sends to resume and settle.
- Order steps compare the chunk batches too: where each ends and its size (ChunkBatchFinished), which the client's
  fixed acknowledgement rate (64 a tick) makes PlayerChunkSender's batch quota decide.
- "edge" steps put the view across the square's edge: the chunks past it load (or generate) as the player's ticket
  asks for them, so when each is ready is the machine's timing. What the move itself does is not: the forgets and the
  first batch (the square's chunks, ready at once) must be the same packet for packet, up to that batch's end. After
  it, once the step has settled, the set of chunks sent must be the same (which chunks at the edge of a view a server
  sends at all). With `--vanilla-runs N` the jar serves the world N times: Sunrise's packets must follow every
  packet all N runs agree on (vanilla's own order past that differs run to run), and every distinct order is kept.

The client's view of each step is the packets after the server's SetChunkCacheCenter for it. Each step waits for its
own centre, then settles when the client has got no new centre, chunk or forget for SETTLE_S seconds. A wait that
runs out, or a client that exits, fails the run.

    python3 tools/test_chunk_send_parity.py
    python3 tools/test_chunk_send_parity.py --keep     # the captures and logs stay in .local/chunk-send-parity
    python3 tools/test_chunk_send_parity.py --vanilla-runs 10   # the edge steps against ten runs of the jar
"""
import argparse
import json
import os
from pathlib import Path
import shutil
import struct
import subprocess
import sys
import time

sys.path.insert(0, str(Path(__file__).resolve().parent))
import lab  # noqa: E402
import net_capture  # noqa: E402
import oracle_harness as oh  # noqa: E402
import test_rust_parity as base  # noqa: E402
from test_dedicated_server_parity import Server, free_port  # noqa: E402

NAME = 'chunk-send-parity'
PLAYER = 'Tester0'
VIEW = 4
PROPERTIES = {
    'server-ip': '127.0.0.1', 'online-mode': 'false', 'white-list': 'false', 'level-seed': '12345',
    'level-type': 'minecraft\\:flat', 'generate-structures': 'false', 'view-distance': str(VIEW),
    'simulation-distance': str(VIEW), 'spawn-protection': '0', 'pause-when-empty-seconds': '-1',
    'gamemode': 'creative', 'difficulty': 'peaceful', 'max-players': '5',
}
RULES = ('spawn_mobs', 'spawn_monsters', 'spawn_patrols', 'spawn_wandering_traders', 'spawn_phantoms',
         'spawn_wardens', 'advance_time', 'advance_weather')
# The forced square, in chunks: -12..12 on both axes, as /forceload's 256-chunk commands.
FORCED = 12
# (kind, chunk x, chunk z): where the console puts the client, in order. A view reaches VIEW + 1 chunks from its
# centre (ChunkTrackingView's buffer), so an order step's centre stays within FORCED - VIEW - 1 of the origin.
STEPS = [
    ('held', 7, 0),
    ('order', 3, 0), ('order', 3, 3), ('order', 0, 5), ('order', -5, -5), ('order', 6, -2), ('order', -7, 7),
    ('order', 0, 0),
    ('edge', 10, 0), ('edge', 12, 12), ('edge', -11, 4),
]
FLOOR_Y = -60
SETTLE_S = 3.0

CLIENTBOUND = net_capture.load_names()[('play', 'clientbound')]


def varint(b, i):
    v = s = 0
    while True:
        x = b[i]
        i += 1
        v |= (x & 0x7F) << s
        if not x & 0x80:
            return (v - (1 << 32) if v >= 1 << 31 else v), i
        s += 7


def chunk_packets(path, partial=False):
    """[(kind, value)] of a capture: 'center', 'chunk' and 'forget' with a chunk position, 'batch' with a
    batch's size, in the order the client got them. `partial`:
    the client is still writing it, so a last record cut short is left for the next read."""
    data, out, i = Path(path).read_bytes(), [], 0
    while i + 12 <= len(data):
        _, pid, n = struct.unpack_from('<IiI', data, i)
        if i + 12 + n > len(data):
            if partial:
                break
            raise AssertionError(f'{Path(path).name}: a record runs past the end of the capture')
        body = data[i + 12:i + 12 + n]
        i += 12 + n
        name = CLIENTBOUND[pid] if 0 <= pid < len(CLIENTBOUND) else None
        if name == 'set_chunk_cache_center':
            x, j = varint(body, 0)
            out.append(('center', (x, varint(body, j)[0])))
        elif name == 'level_chunk_with_light':
            out.append(('chunk', struct.unpack_from('>ii', body, 0)))
        elif name == 'chunk_batch_finished':
            out.append(('batch', varint(body, 0)[0]))
        elif name == 'forget_level_chunk':
            packed = struct.unpack_from('>q', body, 0)[0]
            signed = lambda v: v - (1 << 32) if v >= 1 << 31 else v
            out.append(('forget', (signed(packed & 0xFFFFFFFF), signed((packed >> 32) & 0xFFFFFFFF))))
    return out


def steps_of(packets):
    """The packets after each SetChunkCacheCenter, by centre: [(centre, [(kind, pos)])]."""
    out = []
    for kind, pos in packets:
        if kind == 'center':
            out.append((pos, []))
        elif out:
            out[-1][1].append((kind, pos))
    return out


def captured(path):
    return chunk_packets(path, partial=True) if path.exists() else []


def alive(client, path):
    if client.poll() is not None:
        raise AssertionError(f'{path.name}: the client exited ({client.returncode}) mid-step')


def wait_quiet(path, client, limit=60.0):
    """Until the client has got no new centre, chunk or forget for SETTLE_S seconds (keep-alives and the clock
    keep coming)."""
    deadline, count, since = time.time() + limit, -1, time.time()
    while time.time() < deadline:
        alive(client, path)
        now = len(captured(path))
        if now != count:
            count, since = now, time.time()
        elif time.time() - since >= SETTLE_S:
            return
        time.sleep(0.5)
    raise AssertionError(f'{path.name}: chunks still coming after {limit} s')


def wait_until(path, client, done, what, limit=60.0):
    """Until `done(packets)` holds for the client's packets so far; a timeout or an exited client fails."""
    deadline = time.time() + limit
    while time.time() < deadline:
        alive(client, path)
        packets = captured(path)
        if done(packets):
            return packets
        time.sleep(0.2)
    raise AssertionError(f'{path.name}: no {what} after {limit} s')


def write_dir(directory, port, world=None):
    shutil.rmtree(directory, ignore_errors=True)
    directory.mkdir(parents=True)
    props = dict(PROPERTIES, **{'server-port': str(port)})
    (directory / 'server.properties').write_text(''.join(f'{k}={v}\n' for k, v in props.items()), encoding='iso-8859-1')
    (directory / 'eula.txt').write_text('eula=true\n')
    if world is not None:
        shutil.copytree(world, directory / 'world')
        for lock in (directory / 'world').glob('session.lock'):
            lock.unlink()


def generate(build, vanilla_server):
    """The world both servers serve: generated and saved by the jar, its square forced."""
    directory = build / 'generate'
    write_dir(directory, free_port())
    server = Server(vanilla_server, directory, 'generate')
    try:
        server.wait_for(r'Done \(')
        for rule in RULES:
            server.console(f'gamerule {rule} false')
        server.console('gamerule respawn_radius 0')
        server.console('gamerule random_tick_speed 0')
        lo, hi = -FORCED * 16, FORCED * 16 + 15
        for x0, x1 in ((lo, -1), (0, hi)):
            for z0, z1 in ((lo, -1), (0, hi)):
                server.console(f'forceload add {x0} {z0} {x1} {z1}')
        server.console('say forced')
        server.wait_for(r'\[Server\] forced', timeout=300)
        server.console('save-all flush')
        server.wait_for(r'Saved the game', timeout=300)
    finally:
        server.stop()
    return directory / 'world'


def tracked(cx, cz, x, z):
    """ChunkTrackingView.contains for a VIEW-distance player at (cx, cz): isWithinDistance with the neighbours' buffer."""
    dx, dz = max(abs(x - cx) - 2, 0), max(abs(z - cz) - 2, 0)
    return dx * dx + dz * dz < VIEW * VIEW


def holding(packets):
    """The chunks the client holds after `packets`: each sent and not forgotten since."""
    held = set()
    for kind, value in packets:
        if kind == 'chunk':
            held.add(value)
        elif kind == 'forget':
            held.discard(value)
    return held


def run_side(label, command, build, world, bots):
    directory = build / label
    port = free_port()
    write_dir(directory, port, world)
    capture = build / f'{label}.bin'
    capture.unlink(missing_ok=True)
    # Present from the start: the client holds its batch acknowledgements back until the held step lifts it.
    hold = build / f'{label}.hold'
    hold.write_text('')
    server = Server(command, directory, label)
    client = None
    try:
        server.wait_for(r'Done \(')
        client = subprocess.Popen(
            [str(bots), '--server', f'127.0.0.1:{port}', '--bots', '1', '--behaviour', 'stand', '--ack-mode', '64',
             '--view-distance', str(VIEW), '--name', PLAYER[:-1], '--seconds', '600', '--capture', str(capture),
             '--hold-acks', str(hold)],
            stdout=(build / f'{label}-client.log').open('w'), stderr=subprocess.STDOUT)
        server.wait_for(rf'{PLAYER} joined the game', timeout=120)
        # Held from the start: the first batch finishes, then nothing more comes.
        wait_until(capture, client, lambda ps: any(k == 'batch' for k, _ in ps), 'first finished batch')
        wait_quiet(capture, client)
        for kind, cx, cz in STEPS:
            before = len(captured(capture))
            server.console(f'tp {PLAYER} {cx * 16 + 8} {FLOOR_Y} {cz * 16 + 8}')
            wait_until(capture, client, lambda ps: ('center', (cx, cz)) in ps[before:], f'centre {(cx, cz)}')
            wait_quiet(capture, client)
            if kind == 'held':
                held = captured(capture)[before:]
                if any(k == 'chunk' for k, _ in held) or not any(k == 'forget' for k, _ in held):
                    raise AssertionError(f'{label}: the held step sent chunks or forgot none while held: {held[:12]}')
                released = len(held) + before
                hold.unlink()
                # Resumed: a finished batch after the hold, and every chunk of the view (all inside the forced
                # square, so all ready) held by the client, before the next step.
                view = {(x, z) for x in range(cx - VIEW - 3, cx + VIEW + 4) for z in range(cz - VIEW - 3, cz + VIEW + 4)
                        if tracked(cx, cz, x, z)}
                wait_until(capture, client, lambda ps: any(k == 'batch' for k, _ in ps[released:])
                           and view <= holding(ps), 'a finished batch after the hold lifted and the whole view held')
                wait_quiet(capture, client)
    finally:
        if client is not None:
            client.terminate()
            client.wait(timeout=30)
        server.stop()
    # Stopping the client may cut its last record short; every step had settled before it.
    return steps_of(chunk_packets(capture, partial=True))


def compare(vanilla, sunrise):
    """Problems: the join and every step, as their kinds say."""
    problems = []
    # The join is an order step when the spawn's whole view is in the forced square.
    join = vanilla[0][0] if vanilla else (0, 0)
    inside = max(abs(join[0]), abs(join[1])) + VIEW + 1 <= FORCED
    kinds = [('order' if inside else 'edge', None, None)] + [
        ('order' if kind == 'held' else kind, x, z) for kind, x, z in STEPS]
    if len(vanilla) != len(sunrise) or len(vanilla) != len(kinds):
        problems.append(f'centres: vanilla {[c for c, _ in vanilla]} sunrise {[c for c, _ in sunrise]} '
                        f'(the join and {len(STEPS)} steps expected)')
    for (kind, _, _), (vc, v), (sc, s) in zip(kinds, vanilla, sunrise):
        if vc != sc:
            problems.append(f'centre: vanilla {vc}, sunrise {sc}')
            continue
        rel = lambda items: [(k, v if k == 'batch' else (v[0] - vc[0], v[1] - vc[1])) for k, v in items]
        if kind == 'order':
            if v != s:
                k = next((i for i, (a, b) in enumerate(zip(v, s)) if a != b), min(len(v), len(s)))
                problems.append(f'order at centre {vc}: {len(v)} vanilla packets, {len(s)} sunrise; they part at {k}: '
                                f'vanilla {rel(v[k:k + 6])} sunrise {rel(s[k:k + 6])}')
        else:
            # The move's own packets, up to the first batch's end, are exact; the rest is the machine's timing.
            first = lambda ps: next((i + 1 for i, (k, _) in enumerate(ps) if k == 'batch'), len(ps))
            vm, sm = v[:first(v)], s[:first(s)]
            if vm != sm:
                k = next((i for i, (a, b) in enumerate(zip(vm, sm)) if a != b), min(len(vm), len(sm)))
                problems.append(f'edge move at centre {vc}: {len(vm)} vanilla packets, {len(sm)} sunrise; they part '
                                f'at {k}: vanilla {rel(vm[k:k + 6])} sunrise {rel(sm[k:k + 6])}')
            for what in ('chunk', 'forget'):
                vs = {p for k, p in v if k == what}
                ss = {p for k, p in s if k == what}
                if vs != ss:
                    off = lambda ps: sorted((x - vc[0], z - vc[1]) for x, z in ps)
                    problems.append(f'edge at centre {vc}: {what}s only vanilla {off(vs - ss)}, only sunrise {off(ss - vs)}')
    return problems


def agreed(traces):
    """How many packets every trace in `traces` begins with alike."""
    n = 0
    while all(len(t) > n for t in traces) and len({json.dumps(t[n]) for t in traces}) == 1:
        n += 1
    return n


def edge_orders(build, vanilla_runs, sunrise):
    """Vanilla's own order at each edge step over repeated runs of the jar on the same world and seed: every distinct
    trace (the move's packets through the settled step, in order), how often each came, how many packets all of them
    begin with alike (`agreed`), and where Sunrise's parts from them. Written to edge-orders.json; problems: an edge
    step where Sunrise's trace parts from vanilla's inside the packets every run agrees on. Past them the order is
    the machine's timing (vanilla's own differs run to run): the set of chunks there is `compare`'s."""
    report, problems = [], []
    edges = [i for i, (kind, _, _) in enumerate([('join', None, None)] + STEPS) if kind == 'edge']
    for i in edges:
        traces = {}
        for run in vanilla_runs:
            centre, packets = run[i]
            traces.setdefault(json.dumps(packets), []).append(len(traces))
        centre = vanilla_runs[0][i][0]
        ours = sunrise[i][1]
        vanilla = [json.loads(t) for t in traces]
        prefix = agreed(vanilla)
        follows = agreed([vanilla[0][:prefix], ours])
        report.append({'centre': centre, 'runs': len(vanilla_runs), 'distinct': len(traces),
                       'counts': [len(v) for v in traces.values()], 'agreed': prefix, 'sunrise_follows': follows,
                       'sunrise_among_them': json.dumps(ours) in traces, 'vanilla': vanilla, 'sunrise': ours})
        print(f'edge centre {centre}: {len(vanilla_runs)} vanilla runs, {len(traces)} distinct orders, all alike for '
              f'{prefix} packets; sunrise follows them for {follows}')
        if follows < prefix:
            problems.append(f'edge order at centre {centre}: sunrise parts from the {prefix} packets every vanilla '
                            f'run agrees on at packet {follows}: vanilla {vanilla[0][follows:follows + 4]}, sunrise '
                            f'{ours[follows:follows + 4]}')
    (build / 'edge-orders.json').write_text(json.dumps(report, indent=1))
    return problems


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--keep', action='store_true', help='keep the work folder')
    parser.add_argument('--vanilla-runs', type=int, default=1, metavar='N',
                        help='run the jar N times on the same world and seed: at the edge steps, Sunrise must follow '
                             'the packets every run agrees on (every order is in edge-orders.json, kept)')
    args = parser.parse_args()
    _, jars, libraries = base.verify_artifacts()
    build = lab.LOCAL / NAME
    shutil.rmtree(build, ignore_errors=True)
    build.mkdir(parents=True)
    # `--bin` applies to every package named, so each package names its own.
    oh.run_logged(['cargo', 'build', '--profile', 'fast', '-p', 'sunrise-game', '--bin', 'sunrise-client',
                   '-p', 'sunrise-bots', '--bin', 'sunrise-bots'], build, 'compile-rust')
    exe = '.exe' if os.name == 'nt' else ''
    fast = lab.cargo_target() / 'fast'
    dependencies = os.pathsep.join(map(str, [jars['server'], *libraries]))
    vanilla_server = ['java', '-Xmx2g', '-Duser.timezone=UTC', '-cp', dependencies, 'net.minecraft.server.Main', 'nogui']
    sunrise_server = [fast / f'sunrise-client{exe}', '--resources', lab.RESOURCES, '--dedicated', '--nogui']
    world = generate(build, vanilla_server)
    runs = [run_side('vanilla' if n == 0 else f'vanilla-{n}', vanilla_server, build, world, fast / f'sunrise-bots{exe}')
            for n in range(args.vanilla_runs)]
    vanilla = runs[0]
    sunrise = run_side('sunrise', sunrise_server, build, world, fast / f'sunrise-bots{exe}')
    problems = compare(vanilla, sunrise)
    if args.vanilla_runs > 1:
        problems += edge_orders(build, runs, sunrise)
    for (kind, cx, cz), (centre, packets) in zip([('join', None, None)] + STEPS, vanilla):
        print(f'{kind:5} centre {centre}: vanilla sent {sum(k == "chunk" for k, _ in packets)} chunks, '
              f'forgot {sum(k == "forget" for k, _ in packets)}')
    for p in problems:
        print('DIFFERS', p)
    if not args.keep and args.vanilla_runs == 1:
        shutil.rmtree(build, ignore_errors=True)
    print('chunk-send parity:', 'pass' if not problems else f'{len(problems)} differences')
    return 1 if problems else 0


if __name__ == '__main__':
    sys.exit(main())
