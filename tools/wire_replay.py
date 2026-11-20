#!/usr/bin/env python3
"""Sunrise against vanilla with the same server input on the same ticks: the server packets a real vanilla 26.3
client received in a tools/test_wire_stream_parity.py run are played back to Sunrise, each one right after the
Sunrise tick that vanilla's arrived after, counted from the scenario's /wire-mark. Both clients then got the same
answers (velocity, explosions, teleports, entity ids, keep-alive ids) at the same point of the same scenario tick,
so any difference in what Sunrise sends is the client's own: the server's timing (two independent 50 ms clocks)
and its randomness are out of the comparison. A live server can't give that: the tick its answer lands on, and
values it computes from the client's moves, change from run to run.

  python3 tools/wire_replay.py --vanilla OUT/vanilla.jsonl --scenarios mace_fall_20,... --out DIR

The scenarios are the ones of that vanilla run, in its order. Sunrise runs --headless unless
WIRE_STREAM_WINDOWED=1. DIR gets
sunrise.jsonl (its capture) and a copy of vanilla.jsonl, compared as test_wire_stream_parity.py --compare-only
does. A server packet that reached vanilla just before its next tick-end was taken a tick later (REPLAY_LATE), and
the replay does the same; one a little earlier may have been taken on either tick (REPLAY_EDGE). Those ticks are
inferred from the proxy's timestamps: neither client reports the tick it handled a packet on, so a replay without
differences is a `match`, a diagnostic, never an exact pass. A scenario with such an edge packet, a packet sent
after Sunrise had passed its tick, or (phased) one sent late, is inconclusive. DIR/timing.json keeps that record
with the capture, so test_wire_stream_parity.py --compare-only DIR reaches the same verdict.
"""
import argparse
import json
import os
import shutil
import socket
import sys
import tempfile
import threading
import time
import zlib
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import lab  # noqa: E402
import net_capture  # noqa: E402
import test_wire_stream_parity as ws  # noqa: E402
import wire_stream  # noqa: E402

# A server packet that reached vanilla less than REPLAY_LATE before its next tick-end came after that tick's frame
# had started (measured: 0.2, 1.9, 2.1, 5.3 and 8.6 ms before it, taken a tick later; 10.5 ms before it, taken on
# that tick), so the replay gives it to Sunrise after that tick too. Up to REPLAY_EDGE the tick is uncertain: the
# scenario is inconclusive.
REPLAY_LATE = 0.0095
REPLAY_EDGE = 0.012
# A phased capture's packets reach the client `phase` after a tick-end, give or take this; a later one came
# after the hold point, when the server sent it.
PHASE_SLACK = 0.005
# The server packets that change how the player moves: only their uncertain ticks are worth listing.
MOVING = {'set_entity_motion', 'explode', 'player_position', 'player_rotation', 'update_mob_effect',
          'remove_mob_effect', 'update_attributes', 'player_abilities', 'set_health'}


def varint(value):
    out = bytearray()
    value &= 0xFFFFFFFF
    while True:
        byte = value & 0x7F
        value >>= 7
        out.append(byte | (0x80 if value else 0))
        if not value:
            return bytes(out)


def play_conn(rows):
    """The connection of the capture that reached the play state."""
    return next(r['conn'] for r in rows if r.get('state') == 'play' and 'conn' in r)


class Plan:
    """Vanilla's server packets with when to send them: before the first play packet in order, waiting for the
    client's own packet where vanilla's came between; in play, after the client's (mark, ticks since it)."""

    def __init__(self, rows, phase=None):
        conn = play_conn(rows)
        rows = [r for r in rows if r.get('conn') == conn and 'name' in r]
        self.steps = []  # ('send', row, gate, edge) | ('await', name)
        mark, ticks, sent, last_end = -1, 0, 0, None
        # When each of vanilla's play ticks ended, in order: a server packet's next tick-end is the next of these.
        ends = iter([r['t'] for r in rows
                     if r['dir'] == 'c2s' and r['state'] == 'play' and r['name'] == 'client_tick_end'])
        next_end = next(ends, None)
        for r in rows:
            if r['dir'] == 'c2s':
                if r['state'] != 'play':
                    self.steps.append(('await', r['name']))
                elif wire_stream.mark_packet(r['name'], bytes.fromhex(r.get('data', ''))):
                    mark, ticks, sent = mark + 1, 0, 0
                elif r['name'] == 'client_tick_end':
                    ticks, sent, last_end = ticks + 1, 0, r['t']
                    next_end = next(ends, None)
                elif r['name'] not in wire_stream.FRAME:
                    sent += 1
                continue
            before_end = next_end - r['t'] if next_end is not None and r['state'] == 'play' else None
            late = before_end is not None and before_end < REPLAY_LATE
            edge = (r['state'] == 'play' and r['name'] in MOVING and before_end is not None
                    and REPLAY_LATE <= before_end < REPLAY_EDGE)
            # An answer to the client's own packet of this tick (a command's teleport) waits for that packet:
            # the gate counts the client's packets within the tick too, and its tick-end releases it anyway.
            held = (phase is not None and last_end is not None
                    and r['t'] - last_end < phase + PHASE_SLACK)
            if held:
                # A phased capture (net_capture's phase): a server packet the hold released reached vanilla
                # `phase` after a tick-end, so it is given to Sunrise as long after the same tick-end; nothing is
                # uncertain. One the server sent after the hold point arrived when it came, as unphased.
                late = edge = False
                gate = (mark, ticks, 0) if r['state'] == 'play' else None
            elif phase is not None:
                gate = (mark, ticks + 1, 0) if late else (mark, ticks, 0)
                gate = gate if r['state'] == 'play' else None
            else:
                gate = ((mark, ticks + 1, 0) if late else (mark, ticks, sent)) if r['state'] == 'play' else None
            self.steps.append(('send', r, gate, edge))


class Replay(threading.Thread):
    def __init__(self, plan, capture, phase=None):
        super().__init__(daemon=True)
        self.plan = plan
        self.phase = phase
        # When Sunrise ended each tick, by (mark, ticks since it); (mark, 0) is its last tick-end before the mark.
        self.end_times = {}
        self.last_end = None
        self.late = 0
        self.capture = capture
        self.listener = socket.socket()
        self.listener.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        self.listener.bind(('127.0.0.1', 0))
        self.listener.listen(1)
        self.port = self.listener.getsockname()[1]
        self.cond = threading.Condition()
        self.mark, self.ticks, self.sent = -1, 0, 0
        self.received = []
        self.done = threading.Event()
        self.error = None
        self.reader = None
        # {mark: [why]}: what makes the tick Sunrise took a packet on unknown in that scenario (an edge packet,
        # one sent after the client had passed the tick vanilla took it on, one sent late after its phase point).
        self.uncertain = {}

    def run(self):
        out = open(self.capture, 'w')
        try:
            sock, _ = self.listener.accept()
            sock.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)
            self.session = net_capture.Session(1, out, net_capture.load_names(), threading.Lock(), True)
            self.sock = sock
            self.reader = threading.Thread(target=self.read, daemon=True)
            self.reader.start()
            for step in self.plan.steps:
                if step[0] == 'await':
                    self.wait(lambda name=step[1]: name in self.received, f'the client never sent {step[1]}')
                    self.received.remove(step[1])
                    continue
                _, row, gate, edge = step
                if gate is not None:
                    self.wait(lambda gate=gate: (self.mark, self.ticks, self.sent) >= gate,
                              f'the client never reached mark {gate[0]} tick {gate[1]}', timeout=600)
                    with self.cond:
                        if (self.mark, self.ticks) > gate[:2]:
                            self.doubt(gate[0], f"{row['name']} went out after Sunrise had passed tick {gate[1]}, "
                                       'the one vanilla took it on')
                    if self.phase is not None:
                        with self.cond:
                            ended = self.end_times.get(gate[:2])
                        if ended is not None:
                            delay = ended + self.phase - time.time()
                            if delay > 0:
                                time.sleep(delay)
                            elif delay < -0.02:
                                self.late += 1
                                self.doubt(gate[0], f"{row['name']} went out {-delay * 1000:.0f} ms after its phase "
                                           f'point on tick {gate[1]}')
                if edge:
                    self.doubt(gate[0], f"{row['name']} reached vanilla {REPLAY_LATE * 1000:.1f}-"
                               f'{REPLAY_EDGE * 1000:.0f} ms before the end of tick {gate[1]}: it may have taken it '
                               'on either tick')
                self.send(row['id'], bytes.fromhex(row['data']))
        except Exception as e:  # noqa: BLE001 - reported to the runner
            with self.cond:
                self.error = self.error or e
                self.cond.notify_all()
        finally:
            self.done.set()

    def doubt(self, mark, why):
        """Before the first mark a packet's tick only shifts the settle; from a mark on, that scenario's verdict."""
        if mark >= 0:
            self.uncertain.setdefault(mark, []).append(why)

    def wait(self, ready, message, timeout=120):
        with self.cond:
            if not self.cond.wait_for(lambda: self.error is not None or ready(), timeout):
                raise TimeoutError(message)
            if self.error is not None:
                raise self.error

    def send(self, packet_id, payload):
        s = self.session
        body = varint(packet_id) + payload
        if s.threshold >= 0:
            body = (varint(len(body)) + zlib.compress(body)) if len(body) >= s.threshold else varint(0) + body
        # Recorded before the client can have it: its answer can't come first in the capture.
        s.record('clientbound', packet_id, payload)
        self.sock.sendall(varint(len(body)) + body)
        pending = getattr(s, 'pending_threshold', None)
        if pending is not None:
            s.threshold, s.pending_threshold = pending, None

    def read(self):
        s, buffer = self.session, b''
        try:
            while True:
                chunk = self.sock.recv(1 << 16)
                if not chunk:
                    return
                buffer += chunk
                while True:
                    try:
                        length, pos = net_capture.read_varint(buffer, 0)
                    except IndexError:
                        break
                    if len(buffer) < pos + length:
                        break
                    body, buffer = buffer[pos:pos + length], buffer[pos + length:]
                    if s.threshold >= 0:
                        size, p = net_capture.read_varint(body, 0)
                        body = zlib.decompress(body[p:]) if size else body[p:]
                    packet_id, p = net_capture.read_varint(body, 0)
                    state = s.state['serverbound']
                    name = s.name('serverbound', packet_id)
                    s.record('serverbound', packet_id, body[p:])
                    with self.cond:
                        if state != 'play':
                            self.received.append(name)
                        elif wire_stream.mark_packet(name, body[p:]):
                            self.mark, self.ticks, self.sent = self.mark + 1, 0, 0
                            self.end_times[(self.mark, 0)] = self.last_end
                        elif name == 'client_tick_end':
                            self.ticks, self.sent = self.ticks + 1, 0
                            self.last_end = time.time()
                            self.end_times[(self.mark, self.ticks)] = self.last_end
                        elif name not in wire_stream.FRAME:
                            self.sent += 1
                        self.cond.notify_all()
        except OSError:
            return
        except Exception as e:  # noqa: BLE001 - reported to the runner, as the sender's are
            with self.cond:
                self.error = self.error or e
                self.cond.notify_all()


def replay(vanilla, scenarios, bin_dir, out, phase=None):
    """Plays `vanilla` (a run's vanilla.jsonl, of `scenarios` in that order) back to Sunrise from `bin_dir`;
    `out` gets sunrise.jsonl, a copy of vanilla.jsonl and the delivery-timing record (ws.REPLAY_TIMING). Each
    scenario's problems, as `compare` gives them."""
    out = Path(out)
    if out.exists() and (not out.is_dir() or any(out.iterdir())):
        raise ws.Failed(f'{out} is not an empty folder: the replay writes its captures only into a new one')
    out.mkdir(parents=True, exist_ok=True)
    rows = [json.loads(line) for line in open(vanilla)]
    shutil.copy(vanilla, out / 'vanilla.jsonl')
    plan = Plan(rows, phase)
    vanilla_accepts = []  # accepted teleports before each mark, in vanilla's run
    # Vanilla's ticks between the previous mark (the first: the login) and each mark. The settle before a mark is
    # timed in wall-clock polls, so it lasts a few ticks more or less from run to run; packets are gated by the
    # ticks since the last mark, so a Sunrise that marked sooner would get vanilla's last packets before the mark
    # (a thaw's last attribute updates) only on the scenario's second tick. Sunrise waits for as many ticks.
    window_ticks, accepted, ticks = [], 0, 0
    for r in rows:
        if r.get('dir') == 'c2s' and r.get('name') == 'accept_teleportation':
            accepted += 1
        elif r.get('dir') == 'c2s' and r.get('state') == 'play' and r.get('name') == 'client_tick_end':
            ticks += 1
        elif r.get('dir') == 'c2s' and wire_stream.mark_packet(r.get('name'), bytes.fromhex(r.get('data', ''))):
            vanilla_accepts.append(accepted)
            window_ticks.append(ticks)
            ticks = 0
    if len(vanilla_accepts) != len(scenarios):
        raise ws.Failed(f'the capture has {len(vanilla_accepts)} scenarios, {len(scenarios)} named')
    replay = Replay(plan, out / 'sunrise.jsonl', phase)
    replay.start()
    work = Path(tempfile.mkdtemp(prefix='hz-wire-replay-'))
    saved = {var: os.environ.get(var) for var in ('SUNRISE_HOME', 'SUNRISE_LAUNCHER_HOME', 'XDG_DATA_HOME',
                                                  'XDG_CONFIG_HOME')}
    for var, sub in (('SUNRISE_HOME', 'home'), ('SUNRISE_LAUNCHER_HOME', 'launcher'), ('XDG_DATA_HOME', 'data'),
                     ('XDG_CONFIG_HOME', 'config')):
        (work / 'env' / sub).mkdir(parents=True)
        os.environ[var] = str(work / 'env' / sub)
    os.environ.update(lab.game_env(SUNRISE_TELEMETRY_URL='off'))
    scripts = work / 'scripts'
    scripts.mkdir()
    client = None
    try:
        # WIRE_STREAM_EXTRAS: the same sunrise-extras.txt as a live Sunrise client's.
        extras = os.environ.get('WIRE_STREAM_EXTRAS', '').replace(',', '\n') or None
        client = ws.SunriseClient(Path(bin_dir).resolve(), lab.LOCAL / 'sunrise-client' / lab.VERSION,
                                  work / 'sunrise', f'127.0.0.1:{replay.port}', scripts, extras)
        counts = ws.ClientCounts(out / 'sunrise.jsonl')
        for index, (name, need) in enumerate(zip(scenarios, vanilla_accepts)):
            ws.settle(counts, need)
            replay.wait(lambda index=index: (replay.mark, replay.ticks) >= (index - 1, window_ticks[index]),
                        f'Sunrise never reached tick {window_ticks[index]} before mark {index}')
            if replay.error:
                raise replay.error
            path = scripts / f'{name}-sunrise.json'
            path.write_text(json.dumps(ws.script(name, ws.SCENARIOS[name], True)))
            client.play(path, name, timeout=max(60, ws.SCENARIOS[name]['ticks'] / 20 + 30))
        if not replay.done.wait(60):
            raise ws.Failed("the replay never finished sending vanilla's server stream")
    finally:
        if client:
            client.close()
        if replay.reader is not None:
            replay.reader.join(30)
        # Sunrise's own log, kept beside the capture.
        for log in (work / 'sunrise').glob('*.log'):
            shutil.copy(log, out / f'sunrise-{log.name}')
        shutil.rmtree(work, ignore_errors=True)
        for var, value in saved.items():
            if value is None:
                os.environ.pop(var, None)
            else:
                os.environ[var] = value
    if replay.error:
        raise ws.Failed(f'replay: {replay.error}')
    if replay.reader is not None and replay.reader.is_alive():
        raise ws.Failed("replay: the receiver never saw Sunrise's connection close")
    if replay.late:
        print(f'replay: {replay.late} packets went out more than 20 ms after their phase point (Sunrise was ahead)')
    uncertain = {scenarios[mark]: why for mark, why in sorted(replay.uncertain.items()) if mark < len(scenarios)}
    (out / ws.REPLAY_TIMING).write_text(json.dumps({'scenarios': list(scenarios), 'uncertain': uncertain}, indent=1))
    return ws.compare({'vanilla': out / 'vanilla.jsonl', 'sunrise': out / 'sunrise.jsonl'}, scenarios, True,
                      uncertain)


def main():
    parser = argparse.ArgumentParser(description=__doc__.split('\n\n')[0])
    parser.add_argument('--vanilla', required=True, help="a test_wire_stream_parity.py run's vanilla.jsonl")
    parser.add_argument('--scenarios', required=True, type=lambda s: s.split(','), help='that run\'s, in order')
    parser.add_argument('--bin', default=str(lab.cargo_target() / 'fast'), help='sunrise-client and sunrise-ctl')
    parser.add_argument('--out', required=True)
    parser.add_argument('--phase', type=float, help="the run's WIRE_STREAM_PHASE: its capture is phased")
    args = parser.parse_args()
    try:
        results = replay(args.vanilla, args.scenarios, args.bin, args.out, args.phase)
    except ws.Failed as e:
        sys.exit(f'FAIL: {e}')
    print(f'\nreplayed vanilla server stream, vanilla vs Sunrise; captures in {args.out}')
    for name, problems in results:
        print(f'  {ws.verdict(problems, "replay")}  {name}')
        for p in problems:
            print(p)
    sys.exit(1 if any(p for _, p in results) else 0)


if __name__ == '__main__':
    main()
