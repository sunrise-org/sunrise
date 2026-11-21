#!/usr/bin/env python3
"""Vanilla's transfer between two Sunrise dedicated servers, with nothing in front of them.

Two engines (`sunrise-client --dedicated`) run as an owner runs them, without a proxy: players log in to each
directly, as on vanilla's. A keeps vanilla's `accepts-transfers=false`; B has `accepts-transfers=true`.

    python3 tools/server_transfer_e2e.py --bin-dir target/fast

Steps, each checked:

1. A login to A goes straight through, as vanilla's `ServerLoginPacketListenerImpl.handleHello` lets it in offline:
   no login query before the login finishes, and the offline UUID of the name.
2. A sunrise-bots player joins A. A's console runs `transfer 127.0.0.1 <B's port> <player>` (`TransferCommand`):
   A answers as vanilla does, and the player's client is sent `ClientboundTransferPacket` with B's host and port
   (read from the bot's `--capture`).
3. The client goes where the packet says, as vanilla's does (`ClientCommonPacketListenerImpl.handleTransfer`, a
   handshake with the transfer intent): B, which accepts transfers, logs the player in with the offline UUID of
   its name.
4. The same arrival on A, which does not accept transfers, is refused as vanilla refuses it
   (`ServerHandshakePacketListenerImpl.handleIntention`: `multiplayer.disconnect.transfers_disabled`).

sunrise-bots does not follow a transfer, so this harness makes the client's second connection itself. The engines
run in a fresh folder with fresh SUNRISE_HOME and XDG folders (lab.game_env), on free ports, with no window. Every
check goes to --out/steps.json, with both servers' logs. Build first:
    cargo build --profile fast -p sunrise-game --bin sunrise-client
    cargo build --profile fast -p sunrise-bots
"""
import argparse
import json
import os
from pathlib import Path
import select
import shutil
import struct
import subprocess
import sys
import tempfile
import time

try:
    from . import lab
    from .server_bench import DONE, Server
    from .vanilla_login import Connection, string
except ImportError:
    import lab
    from server_bench import DONE, Server
    from vanilla_login import Connection, string

EXE = '.exe' if os.name == 'nt' else ''
NAME = 'Mover'
PLAYER = NAME + '0'  # sunrise-bots names its bots PREFIX0, PREFIX1...
PROPERTIES = {
    'online-mode': 'false',
    'white-list': 'false',
    'level-type': 'minecraft\\:flat',
    'generate-structures': 'false',
    'spawn-protection': '0',
    'view-distance': '3',
    'simulation-distance': '3',
}
STATUS, LOGIN, TRANSFER = 1, 2, 3  # ClientIntent's ids (sunrise-protocol handshake.rs)
PLAY_TRANSFER = 132  # ClientboundTransferPacket's id in the play state (sunrise-protocol ids.rs)
# The login state's clientbound ids: LoginDisconnect, Hello (encryption), LoginFinished, LoginCompression,
# CustomQuery.
LOGIN_DISCONNECT, LOGIN_HELLO, LOGIN_FINISHED, LOGIN_COMPRESSION, LOGIN_QUERY = 0, 1, 2, 3, 4


class Failed(Exception):
    pass


# ---- what the harness reads ----------------------------------------------------------------------

def read_string(data, pos):
    """A protocol string at data[pos]: (the position after it, the text)."""
    pos, n = lab.parse_varint(data, pos)
    return pos + n, data[pos:pos + n].decode('utf-8', 'replace')


def captured(data):
    """The records of a sunrise-bots `--capture` file: (milliseconds since it joined, packet id, body) each, the
    head little-endian; a record the bot had not finished writing is left out."""
    out, pos = [], 0
    while pos + 12 <= len(data):
        ms, packet, size = struct.unpack_from('<IiI', data, pos)
        if pos + 12 + size > len(data):
            break
        out.append((ms, packet, data[pos + 12:pos + 12 + size]))
        pos += 12 + size
    return out


def transfers_in(data):
    """The (host, port) of every ClientboundTransferPacket in a capture."""
    out = []
    for _, packet, body in captured(data):
        if packet == PLAY_TRANSFER:
            pos, host = read_string(body, 0)
            _, port = lab.parse_varint(body, pos)
            out.append((host, port))
    return out


def login_finished(body):
    """ClientboundLoginFinishedPacket's profile: (its UUID as text, its name)."""
    uuid = '%032x' % int.from_bytes(body[:16], 'big')
    _, name = read_string(body, 16)
    return f'{uuid[:8]}-{uuid[8:12]}-{uuid[12:16]}-{uuid[16:20]}-{uuid[20:]}', name


# ---- a client's connection, as far as the login --------------------------------------------------

def handshake(port, protocol, intent):
    """ClientIntentionPacket to 127.0.0.1:`port`."""
    return lab.varint(protocol) + string('127.0.0.1') + struct.pack('>H', port) + lab.varint(intent)


def protocol_of(port):
    """The protocol version the server at `port` states in its status (the version the bot and the game speak)."""
    c = Connection('127.0.0.1', port)
    try:
        c.send(0, handshake(port, 0, STATUS))
        c.send(0)
        _, data = c.receive()
        _, text = read_string(data, 0)
        return json.loads(text)['version']['protocol']
    finally:
        c.sock.close()


def log_in(port, name, intent, protocol):
    """A client's login to the server at `port` with `intent`, up to the login's end: what the server answered.
    The handshake goes first, alone: a server that refuses the intent answers it at once (and closes), so the
    login start is only sent to one that waits for it."""
    c = Connection('127.0.0.1', port)
    out = {'queries': [], 'finished': None, 'refused': None, 'online': False}
    try:
        c.send(0, handshake(port, protocol, intent))
        if not select.select([c.sock], [], [], 1.0)[0]:
            c.send(0, string(name) + lab.offline_uuid(name).bytes)
        while True:
            try:
                packet, data = c.receive()
            except (EOFError, OSError) as e:
                out['closed'] = str(e)
                return out
            if packet == LOGIN_COMPRESSION:
                c.threshold = lab.parse_varint(data, 0)[1]
            elif packet == LOGIN_FINISHED:
                out['finished'] = login_finished(data)
                return out
            elif packet == LOGIN_DISCONNECT:
                out['refused'] = data.decode('utf-8', 'replace')
                return out
            elif packet == LOGIN_HELLO:
                out['online'] = True
                return out
            elif packet == LOGIN_QUERY:
                pos, transaction = lab.parse_varint(data, 0)
                _, channel = read_string(data, pos)
                out['queries'].append(channel)
                # The answer a client gives a query it does not know: no payload.
                c.send(2, lab.varint(transaction) + b'\x00')
    finally:
        c.sock.close()


# ---- an engine and a player -----------------------------------------------------------------------

class Engine:
    def __init__(self, bins, resources, folder, out, label, **extra):
        self.label = label
        self.folder = folder
        folder.mkdir(parents=True)
        self.port = lab.free_port()
        props = dict(PROPERTIES, **extra, **{'server-port': str(self.port), 'motd': f'transfer e2e {label}'})
        (folder / 'server.properties').write_text(''.join(f'{k}={v}\n' for k, v in props.items()))
        (folder / 'eula.txt').write_text('eula=true\n')
        command = [bins / f'sunrise-client{EXE}', '--resources', resources, '--dedicated', '--nogui']
        self.server = Server(command, folder, out / f'server-{label}.log')

    def stop(self):
        self.server.stop()


class Bot:
    """One sunrise-bots client standing still, its play packets recorded."""

    def __init__(self, bins, port, out, label, seconds):
        self.report = out / f'bot-{label}.json'
        self.capture = out / f'bot-{label}.capture'
        self.log = (out / f'bot-{label}.log').open('w')
        self.process = subprocess.Popen(
            [str(bins / f'sunrise-bots{EXE}'), '--server', f'127.0.0.1:{port}', '--bots', '1',
             '--behaviour', 'stand', '--seconds', str(seconds), '--name', NAME, '--report', str(self.report),
             '--capture', str(self.capture)],
            stdout=self.log, stderr=subprocess.STDOUT, env=lab.game_env())

    def transfers(self, timeout):
        """The transfers its client was sent, once there is one or `timeout` seconds have passed (the bot
        writes its capture out about once a second)."""
        deadline = time.time() + timeout
        while True:
            found = transfers_in(self.capture.read_bytes()) if self.capture.exists() else []
            if found or time.time() >= deadline:
                return found
            time.sleep(0.5)

    def stop(self):
        if self.process.poll() is None:
            self.process.terminate()
            try:
                self.process.wait(10)
            except subprocess.TimeoutExpired:
                self.process.kill()
        self.log.close()


# ---- the run -------------------------------------------------------------------------------------

class Steps:
    def __init__(self, out):
        self.out = out
        self.steps = []
        self.failed = []

    def check(self, step, condition, **what):
        ok = bool(condition)
        self.steps.append({'step': step, 'ok': ok, **what})
        print(('ok    ' if ok else 'FAIL  ') + step + ('' if ok else f': {what}'))
        if not ok:
            self.failed.append(step)
        (self.out / 'steps.json').write_text(json.dumps(self.steps, indent=1))
        return ok


def run(bins, resources, scratch, out):
    steps = Steps(out)
    engines, bots = [], []
    try:
        a = Engine(bins, resources, scratch / 'a', out, 'a', **{'accepts-transfers': 'false'})
        engines.append(a)
        b = Engine(bins, resources, scratch / 'b', out, 'b', **{'accepts-transfers': 'true'})
        engines.append(b)
        a.server.wait_for(DONE, 300)
        b.server.wait_for(DONE, 300)

        protocol = protocol_of(a.port)
        direct = log_in(a.port, 'Direct', LOGIN, protocol)
        steps.check('1. a login to A goes straight through: no login query, the offline UUID of the name',
                    direct['finished'] == (str(lab.offline_uuid('Direct')), 'Direct') and not direct['queries'],
                    login=direct)

        bot = Bot(bins, a.port, out, 'mover', 60)
        bots.append(bot)
        a.server.wait_for(rf'{PLAYER} joined the game', 60)
        mark = a.server.count()
        a.server.console(f'transfer 127.0.0.1 {b.port} {PLAYER}')
        a.server.wait_for(rf'Transferring {PLAYER} to 127\.0\.0\.1:{b.port}', 10)
        steps.check("2. A's console transfers the player, with vanilla's answer", True, console=a.server.since(mark))
        sent = bot.transfers(20)
        steps.check("2. the player's client is sent ClientboundTransferPacket with B's host and port",
                    sent == [('127.0.0.1', b.port)], transfers=sent)

        arrival = log_in(b.port, PLAYER, TRANSFER, protocol)
        steps.check('3. the client arrives on B with the transfer intent and is logged in, with its offline UUID',
                    arrival['finished'] == (str(lab.offline_uuid(PLAYER)), PLAYER) and not arrival['queries'],
                    login=arrival)

        refused = log_in(a.port, PLAYER, TRANSFER, protocol)
        steps.check('4. A, which does not accept transfers, refuses the arrival: transfers_disabled',
                    refused['finished'] is None
                    and 'multiplayer.disconnect.transfers_disabled' in (refused['refused'] or ''), login=refused)
    finally:
        for bot in bots:
            bot.stop()
        for engine in engines:
            try:
                engine.stop()
            except Exception:  # noqa: BLE001 - stopping what is left
                pass
    if steps.failed:
        raise Failed('failed: ' + '; '.join(steps.failed))


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--bin-dir', default=str(lab.ROOT / 'target/fast'))
    parser.add_argument('--resources', default=str(lab.RESOURCES), help='the prepared game files')
    parser.add_argument('--out', default=str(lab.LOCAL / 'server-transfer-e2e'))
    args = parser.parse_args()
    bins, out = Path(args.bin_dir).resolve(), Path(args.out).resolve()
    shutil.rmtree(out, ignore_errors=True)
    out.mkdir(parents=True)
    os.environ.update(lab.game_env())
    with tempfile.TemporaryDirectory(prefix='server-transfer-e2e-') as scratch:
        try:
            run(bins, Path(args.resources).resolve(), Path(scratch), out)
        except (Failed, RuntimeError, OSError) as e:
            sys.exit(f'server_transfer_e2e failed (steps and logs in {out}):\n{e}')
    print(f"server_transfer_e2e: vanilla's transfer took the player from A to B, and A refused one (steps in {out})")


if __name__ == '__main__':
    main()
