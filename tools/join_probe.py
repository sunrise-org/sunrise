#!/usr/bin/env python3
"""What a client answers when a server probes it, vanilla 26.3's client and a Sunrise build side by side.

A vanilla 26.3 dedicated server has no plugin to send probes, so the logging proxy (tools/net_capture.py) writes them
into the server's stream, once the client is in the state each belongs to, and records what the client sends back. The
probes' answers are the clients' serverbound packets after each injected one; the proxy swallows them, so the real
server never sees a packet it did not expect.

  python3 tools/join_probe.py record vanilla --out .local/join-probe/vanilla
  python3 tools/join_probe.py record sunrise --bin $PWD/.local/bin-official --out .local/join-probe/official
  python3 tools/join_probe.py diff .local/join-probe/vanilla .local/join-probe/official

Scenarios (`--scenario`), with the probes each sends (expected serverbound reply in brackets):
- `base` (default):
  - login: custom_query [custom_query_answer, no payload].
  - configuration: ping [pong], store_cookie then cookie_request [cookie_response with the stored bytes],
    minecraft:register, minecraft:unregister, unknown channel (`probe:unknown`), custom_report_details.
  - play: ping [pong], minecraft:register, and an oak sign holding a vanilla translation key, two Sunrise lang
    keys, and a vanilla keybind, opened in the sign editor and closed with Escape [sign_update].
- `sign2`:
  - play: a second oak sign holding a Sunrise keybind, an unknown translation key with fallback, a nested
    `chat.type.text` translation with keybind/Sunrise args, and plain text, opened and closed with Escape [sign_update].
- `sunrise-signals`:
  - configuration: `sunrise:payloads` Query and server brand `sunrise` (which make a Sunrise client announce
    more, and are meant to).
- `transfer`:
  - configuration: store_cookie then `transfer` back to this proxy's 127.0.0.1 port, capturing the client's
    second (transfer intent 3) connection.

Outputs (`client.log`, `capture.jsonl`, `all.json`, and `diff` / `--md` tables, including resolved vanilla sign
lines) are derived from the vanilla client: keep every output under `.local/` and never commit, attach, or paste
them publicly. Only probe names and same/different belong in a public report; `--md` is for local reading.

`diff` exits 0 when every probe matches and 1 whenever any row differs (expected for `sunrise-signals`).
"""
import argparse
import contextlib
import json
import os
import shutil
import struct
import subprocess
import sys
import tempfile
import time
import zlib
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))
from tools import lab, net_capture, test_rust_parity as base
from tools.join_announce import PLAYER, headless_game, output_folder, start_vanilla_client, stop_process
from tools.test_dedicated_server_parity import Server, free_port
from tools.vanilla_wire_e2e import PROPERTIES  # noqa: E402

SIGN = (2, -40, 2)
LINES = [
    {'translate': 'key.sneak'},
    {'translate': 'sunrise.mods.screen.sunrise.authors'},
    {'translate': 'sunrise.extras.zoom'},
    {'keybind': 'key.jump'},
    # Four lines is the sign's limit: the next probe is a second sign.
]
LINES_2 = [
    {'keybind': 'key.sunrise.zoom'},
    {'translate': 'no.such.key', 'fallback': 'fallback text'},
    {'translate': 'chat.type.text', 'with': [{'keybind': 'key.attack'}, {'translate': 'sunrise.extras.sort'}]},
    {'text': 'plain'},
]


def sign_protocol_ids(resources=None):
    """Read oak_sign (rotation=0, not waterlogged) block state id and minecraft:sign block entity
    type protocol id from prepared resources, failing if either is missing."""
    root = Path(lab.RESOURCES if resources is None else resources)
    facts = json.loads((root / 'facts.json').read_text(encoding='utf-8'))
    oak_sign_state = None
    for state_id, entry in enumerate(facts['states']):
        if (isinstance(entry, list) and len(entry) >= 5 and entry[0] == 'minecraft:oak_sign'
                and isinstance(entry[4], dict)):
            props = entry[4].get('properties', {})
            if props.get('rotation') == '0' and props.get('waterlogged') == 'false':
                oak_sign_state = state_id
                break
    if oak_sign_state is None:
        raise ValueError(f'minecraft:oak_sign[rotation=0,waterlogged=false] not found in {root / "facts.json"}')
    registries = json.loads((root / 'registries.json').read_text(encoding='utf-8'))
    sign_entry = registries.get('minecraft:block_entity_type', {}).get('entries', {}).get('minecraft:sign')
    if not isinstance(sign_entry, dict) or not isinstance(sign_entry.get('protocol_id'), int):
        raise ValueError(f'minecraft:sign protocol_id not found in {root / "registries.json"}')
    return oak_sign_state, sign_entry['protocol_id']


def varint(n):
    out = b''
    while True:
        b = n & 0x7F
        n >>= 7
        out += bytes([b | (0x80 if n else 0)])
        if not n:
            return out


def string(s):
    data = s.encode()
    return varint(len(data)) + data


def nbt_string(s):
    data = s.encode()
    return struct.pack('>H', len(data)) + data


def nbt_component(c):
    """A text component as a network NBT compound's body (strings only, which is all the probes use)."""
    out = b''
    for k, v in c.items():
        if isinstance(v, str):
            out += b'\x08' + nbt_string(k) + nbt_string(v)
        else:
            out += b'\x09' + nbt_string(k) + b'\x0a' + struct.pack('>i', len(v))
            for item in v:
                out += nbt_component(item)
    return out + b'\x00'


def sign_nbt(lines):
    def text():
        messages = b'\x09' + nbt_string('messages') + b'\x0a' + struct.pack('>i', 4)
        for line in lines:
            messages += nbt_component(line)
        return (messages + b'\x01' + nbt_string('has_glowing_text') + b'\x00'
                + b'\x08' + nbt_string('color') + nbt_string('black') + b'\x00')
    return (b'\x0a' + b'\x0a' + nbt_string('front_text') + text() + b'\x0a' + nbt_string('back_text') + text()
            + b'\x01' + nbt_string('is_waxed') + b'\x00' + b'\x00')


def position(x, y, z):
    return struct.pack('>q', (x & 0x3FFFFFF) << 38 | (z & 0x3FFFFFF) << 12 | (y & 0xFFF))


def payload(channel, data=b''):
    return string(channel) + data


def swallowed(state, name):
    return (state, name) in {('login', 'custom_query_answer'), ('configuration', 'pong'),
                             ('configuration', 'cookie_response'), ('play', 'pong')}


def plan(scenario, port, oak_sign_state=None, sign_block_entity=None):
    """(trigger (state, name), [(state, packet name, body)]) in the order they fire."""
    if scenario == 'transfer':
        # The cookie the client keeps, then a transfer to this proxy: the second connection is the client's
        # transfer join (intention intent 3), and what it sends first is what a server sees of a transfer.
        return [(('configuration', 'select_known_packs'), [
            ('configuration', 'store_cookie', string('probe:cookie') + varint(5) + b'hello'),
            ('configuration', 'transfer', string('127.0.0.1') + varint(port)),
        ])]
    if scenario == 'sunrise-signals':
        return [(('configuration', 'select_known_packs'), [
            ('configuration', 'custom_payload', payload('sunrise:payloads', b'\x00\x01')),
            ('configuration', 'custom_payload', payload('minecraft:brand', string('sunrise'))),
        ])]
    if oak_sign_state is None or sign_block_entity is None:
        oak_sign_state, sign_block_entity = sign_protocol_ids()
    if scenario == 'sign2':
        return [(('play', 'chunk_batch_finished'), [
            ('play', 'block_update', position(*SIGN) + varint(oak_sign_state)),
            ('play', 'block_entity_data', position(*SIGN) + varint(sign_block_entity) + sign_nbt(LINES_2)),
            ('play', 'open_sign_editor', position(*SIGN) + b'\x01'),
        ])]
    return [
        (('login', 'login_compression'), [('login', 'custom_query', varint(7) + string('probe:query') + b'abc')]),
        (('configuration', 'select_known_packs'), [
            ('configuration', 'ping', struct.pack('>i', 0x1234)),
            ('configuration', 'store_cookie', string('probe:cookie') + varint(5) + b'hello'),
            ('configuration', 'cookie_request', string('probe:cookie')),
            ('configuration', 'custom_payload',
             payload('minecraft:register', b'sunrise:payloads\0horizon:payloads\0minecraft:brand\0probe:x')),
            ('configuration', 'custom_payload', payload('minecraft:unregister', b'probe:x')),
            ('configuration', 'custom_payload', payload('probe:unknown', b'\x01\x02')),
            ('configuration', 'custom_report_details', varint(1) + string('probe') + string('detail')),
        ]),
        (('play', 'chunk_batch_finished'), [
            ('play', 'ping', struct.pack('>i', 0x5678)),
            ('play', 'custom_payload', payload('minecraft:register', b'sunrise:payloads\0probe:x')),
            ('play', 'block_update', position(*SIGN) + varint(oak_sign_state)),
            ('play', 'block_entity_data', position(*SIGN) + varint(sign_block_entity) + sign_nbt(LINES)),
            ('play', 'open_sign_editor', position(*SIGN) + b'\x01'),
        ]),
    ]


def frame(threshold, packet):
    if threshold < 0:
        return varint(len(packet)) + packet
    if len(packet) < threshold:
        return varint(len(packet) + 1) + b'\x00' + packet
    body = varint(len(packet)) + zlib.compress(packet)
    return varint(len(body)) + body


def make_hooks(steps, names, pause=0.05):
    """Hooks for net_capture.start_background: swallow probe replies and inject each step after its trigger."""
    pending = list(steps)

    def should_swallow(_session, direction, state, name, _payload):
        return direction == 'serverbound' and swallowed(state, name)

    def after_forward(session, direction, state, name, dst):
        if direction != 'clientbound':
            return
        for step in list(pending):
            (trigger_state, trigger_name), packets = step
            if (state, name) != (trigger_state, trigger_name):
                continue
            pending.remove(step)
            for packet_state, packet_name, body in packets:
                pid = names[(packet_state, 'clientbound')].index(packet_name)
                packet = varint(pid) + body
                raw = frame(session.threshold, packet)
                with session.lock:
                    dst.sendall(raw)
                    entry = {'conn': session.id, 't': round(time.time() - session.start, 4), 'dir': 's2c',
                             'state': packet_state, 'id': pid, 'name': packet_name, 'size': len(body),
                             'data': body.hex(), 'injected': True}
                    session.out.write(json.dumps(entry) + '\n')
                    session.out.flush()
                if pause:
                    time.sleep(pause)

    return should_swallow, after_forward


def press_escape(client, display):
    if client == 'vanilla':
        subprocess.run(['xdotool', 'mousemove', '400', '300'], env=dict(os.environ, DISPLAY=display), check=False)
        time.sleep(0.3)
        subprocess.run(['xdotool', 'key', 'Escape'], env=dict(os.environ, DISPLAY=display), check=False)


def record(args):
    _, jars, libraries = base.verify_artifacts()
    names = net_capture.load_names()
    deps = os.pathsep.join(map(str, [jars['server'], *libraries]))
    vanilla = ['java', '-Xmx1g', '-cp', deps, 'net.minecraft.server.Main', 'nogui']
    with contextlib.ExitStack() as stack:
        work = Path(tempfile.mkdtemp(prefix='join-probe-'))
        stack.callback(shutil.rmtree, work, ignore_errors=True)
        for var, sub in (('SUNRISE_HOME', 'home'), ('SUNRISE_LAUNCHER_HOME', 'launcher'), ('XDG_DATA_HOME', 'data'),
                         ('XDG_CONFIG_HOME', 'config')):
            (work / 'servers' / sub).mkdir(parents=True)
            os.environ[var] = str(work / 'servers' / sub)
        os.environ.update(lab.game_env())
        directory = work / 'server'
        directory.mkdir()
        port = free_port()
        props = dict(PROPERTIES, **{'server-port': str(port), 'server-ip': '127.0.0.1'})
        (directory / 'server.properties').write_text(''.join(f'{k}={v}\n' for k, v in props.items()),
                                                     encoding='iso-8859-1')
        (directory / 'eula.txt').write_text('eula=true\n')
        out = output_folder(args.out, ('capture.jsonl', 'all.json', 'client.log'))
        capture = out / 'capture.jsonl'
        capture.touch()
        listen = free_port()
        steps = plan(args.scenario, listen)
        swallow_hook, after_forward = make_hooks(steps, names)
        server = Server(vanilla, directory, 'server')
        stack.callback(server.stop)
        server.wait_for(r'Done \(', timeout=300)
        stop = net_capture.start_background(listen, f'127.0.0.1:{port}', capture, full=True,
                                            swallow=swallow_hook, after_forward=after_forward, names=names)
        stack.callback(stop.close)
        address = f'127.0.0.1:{listen}'
        game = display = None
        if args.client == 'vanilla':
            process, xvfb, display = start_vanilla_client(address, work, out / 'client.log')
            stack.callback(stop_process, xvfb)
            stack.callback(stop_process, process)
        else:
            game = headless_game(Path(args.bin), lab.RESOURCES, work / 'game')
            stack.callback(game.quit)
            game('connect', address, PLAYER)
            stack.callback(game, 'disconnect', ok=False)
        last = {'sunrise-signals': 'custom_payload', 'transfer': 'transfer'}.get(args.scenario, 'open_sign_editor')
        expected_injected = sum(len(packets) for _, packets in steps)
        deadline = time.time() + 240
        finished = False
        while time.time() < deadline:
            injected = [e for e in net_capture.load(capture) if e.get('injected')]
            if (len(injected) >= expected_injected
                    and any(e['name'] == last and e['state'] in ('play', 'configuration') for e in injected)):
                finished = True
                break
            time.sleep(0.5)
        if not finished:
            sys.exit(f'timed out after 240s waiting for injected probe {last}')
        time.sleep(4)
        if args.scenario in ('base', 'sign2'):
            if game is not None:
                game('key', 'esc', 'tap', ok=False)
            else:
                press_escape('vanilla', display)
            time.sleep(3)
    entries = [e for e in net_capture.load(capture) if 'name' in e and (args.scenario == 'transfer' or e.get('conn') == 1)]
    (out / 'all.json').write_text(json.dumps(entries, indent=1))
    print(f'{len(entries)} packets in {out / "all.json"}')


def describe(e):
    data = bytes.fromhex(e.get('data', ''))
    text = f"{e['state']}/{e['name']}"
    if e['name'] == 'custom_payload' and data:
        n, p = net_capture.read_varint(data, 0)
        return f"{text} channel={data[p:p + n].decode(errors='replace')} body={data[p + n:].hex()}"
    if e['name'] == 'intention':
        version, p = net_capture.read_varint(data, 0)
        n, q = net_capture.read_varint(data, p)
        host = data[q:q + n].decode()
        port = int.from_bytes(data[q + n:q + n + 2], 'big')
        return f"{text} protocol={version} host={host} port={'(proxy port)' if port else 0} intent={data[q + n + 2]}"
    if e['name'] == 'transfer' and data:
        n, q = net_capture.read_varint(data, 0)
        host = data[q:q + n].decode()
        port, _ = net_capture.read_varint(data, q + n)
        return f"{text} host={host} port={'(proxy port)' if port else 0}"
    if e['name'] == 'sign_update':
        # position, the four lines, then the side's flag
        lines, p = [], 8
        for _ in range(4):
            n, q = net_capture.read_varint(data, p)
            lines.append(data[q:q + n].decode(errors='replace'))
            p = q + n
        return f'{text} lines={lines} front={data[p]}'
    return f'{text} {data.hex()[:80]}'


def probe_key(e):
    """Stable key pairing an injected probe across two runs: (state, name, body), with transfer's
    ephemeral proxy port normalized away so two runs of the transfer scenario pair."""
    body = e.get('data', '')
    if e['name'] == 'transfer' and body:
        data = bytes.fromhex(body)
        n, q = net_capture.read_varint(data, 0)
        _, end = net_capture.read_varint(data, q + n)
        return (e['state'], e['name'], data[:q + n].hex(), data[end:].hex())
    return (e['state'], e['name'], body)


def answers(path):
    """For each injected packet: (key, label, replies) where replies are what the client sent
    between it and the next injection."""
    entries = json.loads((Path(path) / 'all.json').read_text())
    out = []
    seen = {}
    for e in entries:
        if e.get('injected'):
            base_key = probe_key(e)
            occ = seen.get(base_key, 0)
            seen[base_key] = occ + 1
            out.append(((base_key, occ), describe(e), []))
        elif e['dir'] == 'c2s' and out and e['name'] not in ('client_tick_end', 'chunk_batch_received',
                                                              'move_player_pos', 'move_player_status_only',
                                                              'move_player_pos_rot', 'move_player_rot',
                                                              'keep_alive'):
            out[-1][2].append(f"<- {describe(e)}")
    return out


def diff(args):
    a, b = answers(args.a), answers(args.b)
    by_a = {key: (label, replies) for key, label, replies in a}
    by_b = {key: (label, replies) for key, label, replies in b}
    order = list(dict.fromkeys([key for key, _, _ in a] + [key for key, _, _ in b]))
    rows = ['| probe | vanilla | sunrise |', '|---|---|---|']
    differences = 0
    for key in order:
        left = by_a.get(key)
        right = by_b.get(key)
        label = left[0] if left is not None else right[0]
        if left is not None and right is not None and left[1] == right[1]:
            rows.append(f"| {label} | {'<br>'.join(left[1]) or 'nothing'} | (same) |")
        else:
            differences += 1
            ltext = '(missing probe)' if left is None else ('<br>'.join(left[1]) or 'nothing')
            rtext = '(missing probe)' if right is None else ('<br>'.join(right[1]) or 'nothing')
            rows.append(f"| {label} | {ltext} | {rtext} |")
    if len(a) != len(b):
        rows.append(f'| probe count | {len(a)} | {len(b)} |')
        differences += 1
    text = '\n'.join(rows)
    print(text)
    if args.md:
        Path(args.md).write_text(text + '\n')
    sys.exit(1 if differences else 0)


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = parser.add_subparsers(dest='command', required=True)
    r = sub.add_parser('record')
    r.add_argument('client', choices=['vanilla', 'sunrise'])
    r.add_argument('--bin', default=str(lab.cargo_target() / 'fast'))
    r.add_argument('--out', required=True)
    r.add_argument('--scenario', choices=['base', 'sign2', 'sunrise-signals', 'transfer'], default='base')
    d = sub.add_parser('diff')
    d.add_argument('a')
    d.add_argument('b')
    d.add_argument('--md')
    args = parser.parse_args()
    (record if args.command == 'record' else diff)(args)


if __name__ == '__main__':
    main()
