#!/usr/bin/env python3
"""A decoding TCP proxy for the Minecraft 26.3 protocol (offline mode, no
encryption): clients connect to LISTEN, it forwards to TARGET and records
every packet in both directions as JSON lines (time, direction, state, id,
name, size and the payload), following the connection state through the
handshake, login, configuration and play (and back to configuration).

  net_capture.py --listen 25570 --target localhost:25565 --out capture.jsonl

The packet names come from crates/sunrise-protocol/src/ids.rs, which is
generated from vanilla's packets report. `summary` prints what a capture
contains, `diff` compares the packet sequences of two captures (for example
Sunrise hosting and a vanilla server serving the same world).
"""
import argparse
import collections
import json
import os
import queue
import random
import re
import socket
import sys
import threading
import time
import zlib
from collections import Counter
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
STATES = ["handshake", "status", "login", "configuration", "play"]


def load_names(path=ROOT / "crates/sunrise-protocol/src/ids.rs"):
    """{(state, direction): [name by id]} from the generated id tables."""
    text = path.read_text()
    names = {}
    for state in STATES:
        block = re.search(r"pub mod %s \{(.*?)\n\}" % state, text, re.S)
        if not block:
            continue
        for direction in ("clientbound", "serverbound"):
            part = re.search(r"pub mod %s \{.*?NAMES: &\[&str\] = &\[(.*?)\];" % direction,
                             block.group(1), re.S)
            if part:
                names[(state, direction)] = re.findall(r'"([^"]+)"', part.group(1))
    return names


def read_varint(data, pos):
    value = shift = 0
    while True:
        if pos >= len(data):
            raise IndexError("truncated varint")
        b = data[pos]
        pos += 1
        value |= (b & 0x7F) << shift
        shift += 7
        if not b & 0x80:
            if value & 0x80000000:
                value -= 1 << 32
            return value, pos
        if shift > 35:
            raise ValueError("varint too long")


class Session:
    """The state both directions of one connection share."""

    def __init__(self, conn_id, out, names, lock, full):
        self.id = conn_id
        self.state = {"clientbound": "handshake", "serverbound": "handshake"}
        self.threshold = -1
        self.out = out
        self.names = names
        self.lock = lock
        self.full = full
        self.start = time.time()
        self.closed = threading.Event()
        # `phase` (seconds): in play, the server's packets reach the client that long after its last tick-end,
        # mid-way between two of its ticks, so it takes each on its next tick for certain (no packet lands near a
        # tick boundary, where which tick takes it depends on the client's frame).
        self.phase = None
        self.tick_ends = collections.deque(maxlen=200)
        self.ticked = threading.Condition()

    def write(self, line):
        with self.lock:
            self.out.write(line)
            self.out.flush()

    def hold(self, arrived):
        """With a phase, in play: hold a server packet that `arrived` until `phase` after the client's last
        tick-end before it, or after its next one when that point had passed. Times come from when packets
        arrived, so a backlog goes out together, in order, as soon as its point is due."""
        if self.phase is None or self.state["clientbound"] != "play":
            return
        with self.ticked:
            before = [t for t in self.tick_ends if t <= arrived]
            if not before:
                return
            if arrived < before[-1] + self.phase:
                target = before[-1] + self.phase
            else:
                self.ticked.wait_for(lambda: self.tick_ends and self.tick_ends[-1] > arrived, timeout=1.0)
                after = [t for t in self.tick_ends if t > arrived]
                target = (after[0] if after else arrived) + self.phase
        delay = target - time.time()
        if delay > 0:
            time.sleep(delay)

    def name(self, direction, packet_id):
        table = self.names.get((self.state[direction], direction), [])
        return table[packet_id] if 0 <= packet_id < len(table) else f"unknown_{packet_id}"

    def record(self, direction, packet_id, payload, at=None, write=True):
        """`at`: when the packet reaches its peer, if not now (a clientbound one WIRE_STREAM_LATENCY holds).
        `write` false: the line is returned for the caller to write when the packet goes out. The state follows
        the packet now either way."""
        state = self.state[direction]
        name = self.name(direction, packet_id)
        entry = {
            "conn": self.id,
            "t": round((at or time.time()) - self.start, 4),
            "dir": "s2c" if direction == "clientbound" else "c2s",
            "state": state,
            "id": packet_id,
            "name": name,
            "size": len(payload),
        }
        if self.full or len(payload) <= 4096:
            entry["data"] = payload.hex()
        else:
            entry["head"] = payload[:512].hex()
        line = json.dumps(entry) + "\n"
        if write:
            self.write(line)
        self.transition(direction, name, payload)
        if direction == "serverbound" and name == "client_tick_end" and state == "play":
            with self.ticked:
                self.tick_ends.append(time.time())
                self.ticked.notify_all()
        return line

    def transition(self, direction, name, payload):
        s = self.state
        if direction == "serverbound":
            if s["serverbound"] == "handshake" and name == "intention":
                pos = read_varint(payload, 0)[1]
                length, pos = read_varint(payload, pos)
                intent = read_varint(payload, pos + length + 2)[0]
                nxt = {1: "status", 2: "login", 3: "login"}.get(intent, "login")
                s["serverbound"] = s["clientbound"] = nxt
            elif s["serverbound"] == "login" and name == "login_acknowledged":
                s["serverbound"] = s["clientbound"] = "configuration"
            elif s["serverbound"] == "configuration" and name == "finish_configuration":
                s["serverbound"] = s["clientbound"] = "play"
            elif s["serverbound"] == "play" and name == "configuration_acknowledged":
                s["serverbound"] = s["clientbound"] = "configuration"
        elif s["clientbound"] == "login" and name == "login_compression":
            self.pending_threshold = read_varint(payload, 0)[0]


def latency_from_env():
    """WIRE_STREAM_LATENCY: a round trip each frame waits half of in each direction. `MS` a constant ping, `MS~J`
    a ping spread by up to J either way (seeded: the same run waits the same), `A..B/S` a ping rising from A to B
    and falling back every S seconds. None without the variable."""
    spec = os.environ.get("WIRE_STREAM_LATENCY")
    if not spec:
        return None
    start = time.time()
    if ".." in spec:
        span, period = spec.split("/")
        low, high = (float(v) for v in span.split(".."))
        period = float(period)

        def ping(now):
            phase = ((now - start) % period) / period
            return low + (high - low) * (1 - abs(2 * phase - 1))
    else:
        base, _, jitter = spec.partition("~")
        base, jitter = float(base), float(jitter or 0)
        rng = random.Random(1)

        def ping(now):
            return max(0.0, base + rng.uniform(-jitter, jitter))
    return lambda now: ping(now) / 2000.0


class Pipe(threading.Thread):
    def __init__(self, session, direction, src, dst):
        super().__init__(daemon=True)
        self.session, self.direction, self.src, self.dst = session, direction, src, dst
        # WIRE_STREAM_LATENCY: frames go out from a queue at their due time, never before an earlier frame's. A
        # server packet's line is written there, just before it goes out: the capture stays in time order, a reply
        # never comes before its input, and a replay walking it hands the packet over on the client tick vanilla
        # had it.
        self.latency = latency_from_env()
        self.queue = queue.Queue() if self.latency else None
        self.last_due = 0.0
        self.sender = None
        if self.queue is not None:
            self.sender = threading.Thread(target=self.send_later, daemon=True)
            self.sender.start()

    def due(self):
        """When a frame read now goes out under WIRE_STREAM_LATENCY (None without it)."""
        if self.queue is None:
            return None
        now = time.time()
        self.last_due = max(self.last_due, now + self.latency(now))
        return self.last_due

    def forward(self, frame, due, line=None):
        if due is None:
            self.dst.sendall(frame)
        else:
            self.queue.put((due, frame, line))

    def send_later(self):
        while True:
            item = self.queue.get()
            if item is None:
                return
            due, frame, line = item
            wait = due - time.time()
            if wait > 0:
                time.sleep(wait)
            if line is not None:
                self.session.write(line)
            try:
                self.dst.sendall(frame)
            except OSError:
                return

    def run(self):
        buffer = b""
        s = self.session
        try:
            while True:
                chunk = self.src.recv(1 << 16)
                if not chunk:
                    break
                arrived = time.time()
                buffer += chunk
                frames = []
                while True:
                    try:
                        length, pos = read_varint(buffer, 0)
                    except IndexError:
                        break
                    if len(buffer) < pos + length:
                        break
                    frames.append(buffer[:pos + length])
                    body, buffer = buffer[pos:pos + length], buffer[pos + length:]
                    threshold = s.threshold
                    if threshold >= 0:
                        size, p = read_varint(body, 0)
                        body = zlib.decompress(body[p:]) if size else body[p:]
                    packet_id, p = read_varint(body, 0)
                    if self.direction == "clientbound":
                        s.hold(arrived)
                    # Under WIRE_STREAM_LATENCY a server packet counts from when it reaches the client, which is
                    # what a replay of the capture delivers it by; a client packet from when the client sent it.
                    due = self.due()
                    held = due is not None and self.direction == "clientbound"
                    line = s.record(self.direction, packet_id, body[p:], at=due if held else None, write=not held)
                    # The compression threshold applies after its packet, in
                    # both directions; the client answers only after reading it.
                    pending = getattr(s, "pending_threshold", None)
                    if pending is not None:
                        s.threshold = pending
                        s.pending_threshold = None
                    # State switches must be in effect before the peer can
                    # answer, so forward each frame after recording it.
                    self.forward(frames.pop(), due, line if held else None)
        except (OSError, ValueError, zlib.error) as e:
            with s.lock:
                s.out.write(json.dumps({"conn": s.id, "event": "error", "dir": self.direction,
                                        "error": str(e)}) + "\n")
                s.out.flush()
        finally:
            # The frames still queued go out (and their lines in) before the connection closes.
            if self.sender is not None:
                self.queue.put(None)
                self.sender.join()
            for sock in (self.src, self.dst):
                try:
                    sock.shutdown(socket.SHUT_RDWR)
                except OSError:
                    pass
            s.closed.set()


def serve(listen, target, out_path, full=False, ready=None, stop=None, phase=None):
    names = load_names()
    host, port = target.rsplit(":", 1)
    server = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    server.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    server.bind(("127.0.0.1", listen))
    server.listen()
    server.settimeout(0.5)
    lock = threading.Lock()
    conn_id = 0
    pipes = []
    with open(out_path, "a") as out:
        if ready:
            ready.set()
        while not (stop and stop.is_set()):
            try:
                client, _ = server.accept()
            except socket.timeout:
                continue
            conn_id += 1
            try:
                upstream = socket.create_connection((host, int(port)))
            except OSError as e:
                client.close()
                with lock:
                    out.write(json.dumps({"conn": conn_id, "event": "error",
                                          "error": f"upstream: {e}"}) + "\n")
                continue
            for sock in (client, upstream):
                sock.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)
            session = Session(conn_id, out, names, lock, full)
            session.phase = phase
            for pipe in (Pipe(session, "serverbound", client, upstream),
                         Pipe(session, "clientbound", upstream, client)):
                pipe.start()
                pipes.append(pipe)
        # Every connection's workers finish (their queued lines written) before the capture closes.
        for pipe in pipes:
            pipe.join(10)
    server.close()


def start_background(listen, target, out_path, full=False, phase=None):
    """Run the proxy on a thread; returns the stop event."""
    ready, stop = threading.Event(), threading.Event()
    threading.Thread(target=serve, args=(listen, target, out_path, full, ready, stop, phase),
                     daemon=True).start()
    ready.wait(5)
    return stop


def load(path):
    return [json.loads(line) for line in Path(path).read_text().splitlines() if line.strip()]


def play_sessions(entries):
    """Connections that reached the play state."""
    return sorted({e["conn"] for e in entries if e.get("state") == "play"})


def summary(entries, conn=None):
    lines = []
    conns = [conn] if conn else sorted({e["conn"] for e in entries})
    for c in conns:
        mine = [e for e in entries if e["conn"] == c and "name" in e]
        lines.append(f"connection {c}: {len(mine)} packets")
        counts = Counter((e["dir"], e["state"], e["name"]) for e in mine)
        for (d, s, n), k in sorted(counts.items()):
            size = sum(e["size"] for e in mine if (e["dir"], e["state"], e["name"]) == (d, s, n))
            lines.append(f"  {d} {s:13} {n:40} x{k:<6} {size} bytes")
        for e in entries:
            if e["conn"] == c and e.get("event") == "error":
                lines.append(f"  error ({e.get('dir')}): {e['error']}")
    return "\n".join(lines)


def sequence(entries, conn, direction="s2c", until="player_position"):
    """Packet names in order, up to and including the first `until`."""
    out = []
    for e in entries:
        if e["conn"] != conn or e.get("dir") != direction or "name" not in e:
            continue
        out.append(f"{e['state']}/{e['name']}")
        if e["name"] == until and e["state"] == "play":
            break
    return out


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="command", required=True)
    p = sub.add_parser("proxy")
    p.add_argument("--listen", type=int, required=True)
    p.add_argument("--target", required=True)
    p.add_argument("--out", type=Path, required=True)
    p.add_argument("--full", action="store_true", help="keep every payload in full")
    s = sub.add_parser("summary")
    s.add_argument("capture", type=Path)
    d = sub.add_parser("diff")
    d.add_argument("a", type=Path)
    d.add_argument("b", type=Path)
    d.add_argument("--until", default="player_position")
    args = parser.parse_args()
    if args.command == "proxy":
        serve(args.listen, args.target, args.out, args.full)
    elif args.command == "summary":
        print(summary(load(args.capture)))
    else:
        import difflib
        a, b = load(args.a), load(args.b)
        sa = sequence(a, play_sessions(a)[0], until=args.until)
        sb = sequence(b, play_sessions(b)[0], until=args.until)
        sys.stdout.writelines(difflib.unified_diff([x + "\n" for x in sa], [x + "\n" for x in sb],
                                                   str(args.a), str(args.b)))


if __name__ == "__main__":
    main()
