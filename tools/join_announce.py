#!/usr/bin/env python3
"""What a client announces on joining: every serverbound packet from the handshake through login,
configuration and the first seconds of play, recorded at a vanilla 26.3 dedicated server's side of
tools/net_capture.py, for vanilla's own client and for a Sunrise build, and the side-by-side diff.

  python3 tools/join_announce.py record vanilla  --out .local/join-announce/vanilla
  python3 tools/join_announce.py record sunrise  --bin .local/bin/official --out .local/join-announce/official
  python3 tools/join_announce.py diff .local/join-announce/vanilla .local/join-announce/official [--md FILE]

`record` writes `capture.jsonl` (the proxy's lines) and `serverbound.json` (the client's packets in order:
state, id, name, payload hex). `--bin` is a folder holding `sunrise-client` (and `sunrise-ctl` beside it).
The server has no plugins and signals nothing Sunrise-specific. Games run in fresh temporary folders
(HOME, SUNRISE_HOME, SUNRISE_LAUNCHER_HOME, XDG_*, no audio); the vanilla client runs on a private Xvfb screen.
Needs .local (vanilla jars, prepared resources), and for `sunrise` a built sunrise-client and sunrise-ctl.

Outputs (`client.log`, `capture.jsonl`, `serverbound.json`, and `diff` / `--md` tables) are derived from the
vanilla client: keep every output under `.local/` and never commit, attach, or paste them publicly. Only packet
names and same/different belong in a public report; `--md` is for local reading.

`diff` exits 0 when every row matches and 1 whenever any row differs.
"""
import argparse
import contextlib
import json
import os
import select
import shutil
import subprocess
import sys
import tempfile
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))
from tools import lab, net_capture, test_rust_parity as base
from tools.server_pack_e2e import Game
from tools.test_dedicated_server_parity import Server, free_port
from tools.vanilla_wire_e2e import PROPERTIES  # noqa: E402

PLAYER = 'Steve'
PLAY_SECONDS = 5.0


def headless_game(bins, resources, work):
    """server_pack_e2e's Game launched headless with telemetry disabled."""
    return Game(bins, resources, work, mode='--headless', env={'SUNRISE_TELEMETRY_URL': 'off'},
                log=work / 'home' / 'logs' / 'latest.log')


def _read_display(read_fd, xvfb):
    """Read Xvfb's display number from its -displayfd pipe without hanging if Xvfb exits."""
    chunks = bytearray()
    while b'\n' not in chunks:
        ready, _, _ = select.select([read_fd], [], [], 0.2)
        if ready:
            chunk = os.read(read_fd, 64)
            if not chunk:
                break
            chunks.extend(chunk)
        elif xvfb.poll() is not None:
            while select.select([read_fd], [], [], 0)[0]:
                chunk = os.read(read_fd, 64)
                if not chunk:
                    break
                chunks.extend(chunk)
            break
    line = chunks.decode('ascii', errors='replace').splitlines()[:1]
    number = line[0].strip() if line else ''
    if not number.isdigit():
        raise RuntimeError(f'Xvfb failed to report a display (exit={xvfb.poll()}, output={number!r})')
    return f':{number}'


def stop_process(process):
    if process is None:
        return
    if process.poll() is None:
        process.terminate()
    process.wait()


def start_vanilla_client(address, work, log):
    from tools import vanilla_client as vc
    args = argparse.Namespace(name=PLAYER, join=address, world=None, game_dir=work / 'client', assets=None,
                              heap='2G', width=854, height=480, lan_port=None, backend=None)
    args.game_dir.mkdir(parents=True)
    vc.write_options(args.game_dir, {'maxFps': '30'})
    command = vc.command(args)
    env = vc.environment()
    client_home = work / 'client-home'
    client_data = work / 'client-xdg' / 'data'
    client_config = work / 'client-xdg' / 'config'
    for folder in (client_home, client_data, client_config):
        folder.mkdir(parents=True, exist_ok=True)
    env['HOME'] = str(client_home)
    env['XDG_DATA_HOME'] = str(client_data)
    env['XDG_CONFIG_HOME'] = str(client_config)
    env.pop('WAYLAND_DISPLAY', None)
    # Xvfb picks a free display and writes its number, so two runs never share a screen.
    read_fd, write_fd = os.pipe()
    try:
        xvfb = subprocess.Popen(['Xvfb', '-displayfd', str(write_fd), '-screen', '0', '1024x768x24'],
                                stderr=subprocess.DEVNULL, pass_fds=(write_fd,))
    except BaseException:
        os.close(read_fd)
        os.close(write_fd)
        raise
    os.close(write_fd)
    try:
        try:
            display = _read_display(read_fd, xvfb)
        finally:
            os.close(read_fd)
        env['DISPLAY'] = display
        env['LIBGL_ALWAYS_SOFTWARE'] = '1'
        with open(log, 'w', encoding='utf-8') as log_file:
            process = subprocess.Popen(command, cwd=args.game_dir, env=env, stdout=log_file,
                                       stderr=subprocess.STDOUT)
    except BaseException:
        stop_process(xvfb)
        raise
    return process, xvfb, display


def output_folder(path, own):
    """`--out`, emptied of this tool's own files from an earlier run. A folder holding anything
    else is refused, so a mistyped argument never deletes a checkout or a home folder."""
    out = Path(path)
    if out.exists():
        if not out.is_dir():
            sys.exit(f'--out {out} is a file')
        stray = sorted(p.name for p in out.iterdir() if p.name not in own)
        if stray:
            sys.exit(f'--out {out} holds files this tool did not write ({", ".join(stray[:5])}); '
                     'name a new or empty folder')
        for name in own:
            (out / name).unlink(missing_ok=True)
    out.mkdir(parents=True, exist_ok=True)
    return out


def record(args):
    _, jars, libraries = base.verify_artifacts()
    deps = os.pathsep.join(map(str, [jars['server'], *libraries]))
    vanilla = ['java', '-Xmx1g', '-cp', deps, 'net.minecraft.server.Main', 'nogui']
    with contextlib.ExitStack() as stack:
        work = Path(tempfile.mkdtemp(prefix='join-announce-'))
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
        out = output_folder(args.out, ('capture.jsonl', 'serverbound.json', 'client.log'))
        capture = out / 'capture.jsonl'
        capture.touch()
        server = Server(vanilla, directory, 'server')
        stack.callback(server.stop)
        server.wait_for(r'Done \(', timeout=300)
        listen = free_port()
        stop = net_capture.start_background(listen, f'127.0.0.1:{port}', capture, full=True)
        stack.callback(stop.close)
        address = f'127.0.0.1:{listen}'
        if args.client == 'vanilla':
            process, xvfb, _ = start_vanilla_client(address, work, out / 'client.log')
            stack.callback(stop_process, xvfb)
            stack.callback(stop_process, process)
        else:
            game = headless_game(Path(args.bin), lab.RESOURCES, work / 'game')
            stack.callback(game.quit)
            game('connect', address, PLAYER)
            stack.callback(game, 'disconnect', ok=False)
        deadline = time.time() + 240
        joined = False
        while time.time() < deadline:
            if any(e.get('name') == 'login' and e.get('state') == 'play' for e in net_capture.load(capture)):
                joined = True
                break
            time.sleep(0.5)
        if not joined:
            sys.exit('timed out after 240s waiting for play/login packet')
        time.sleep(PLAY_SECONDS)
    entries = [e for e in net_capture.load(capture) if e.get('conn') == 1 and 'name' in e and e['dir'] == 'c2s']
    sent = [{k: e[k] for k in ('state', 'id', 'name', 'data') if k in e} for e in entries]
    (out / 'serverbound.json').write_text(json.dumps(sent, indent=1))
    print(f'{len(sent)} serverbound packets in {out / "serverbound.json"}')


def describe(p):
    """One readable line: packet name and, for a custom payload, its channel and body."""
    data = bytes.fromhex(p.get('data', ''))
    name = f"{p['state']}/{p['name']} (id {p['id']})"
    if p['name'] == 'intention':
        # protocol version, then the address string; the port and the intent vary by run, not by client.
        version, pos = net_capture.read_varint(data, 0)
        length, pos = net_capture.read_varint(data, pos)
        return f"{name} protocol={version} address={data[pos:pos + length].decode(errors='replace')}"
    if p['state'] == 'play':
        return f"{name}"
    if p['name'] == 'custom_payload' and data:
        length, pos = net_capture.read_varint(data, 0)
        return f"{name} channel={data[pos:pos + length].decode(errors='replace')} body={data[pos + length:].hex()}"
    return f"{name} {data.hex()[:80]}"


def packet_lines(packets):
    # Play packets repeat (ticks, chunk batches) and their counts and order follow timing: the first of each name.
    out = []
    for p in packets:
        line = describe(p)
        if p['state'] != 'play' or line not in out:
            out.append(line)
    return out


def diff(args):
    import difflib
    a = json.loads((Path(args.a) / 'serverbound.json').read_text())
    b = json.loads((Path(args.b) / 'serverbound.json').read_text())
    la, lb = packet_lines(a), packet_lines(b)
    rows = ['| # | vanilla | sunrise |', '|---|---|---|']
    matcher = difflib.SequenceMatcher(None, la, lb, autojunk=False)
    n = 0
    for tag, i1, i2, j1, j2 in matcher.get_opcodes():
        for k in range(max(i2 - i1, j2 - j1)):
            left = la[i1 + k] if i1 + k < i2 else ''
            right = lb[j1 + k] if j1 + k < j2 else ''
            n += 1
            mark = '' if tag == 'equal' else ' **≠**'
            rows.append(f'| {n}{mark} | {left} | {right} |')
    text = '\n'.join(rows)
    print(text)
    if args.md:
        Path(args.md).write_text(text + '\n')
    differing = sum(1 for t in matcher.get_opcodes() if t[0] != 'equal')
    sys.exit(1 if differing else 0)


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = parser.add_subparsers(dest='command', required=True)
    r = sub.add_parser('record')
    r.add_argument('client', choices=['vanilla', 'sunrise'])
    r.add_argument('--bin', default=str(lab.cargo_target() / 'fast'))
    r.add_argument('--out', required=True)
    d = sub.add_parser('diff')
    d.add_argument('a')
    d.add_argument('b')
    d.add_argument('--md')
    args = parser.parse_args()
    (record if args.command == 'record' else diff)(args)


if __name__ == '__main__':
    main()
