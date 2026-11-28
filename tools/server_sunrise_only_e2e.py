#!/usr/bin/env python3
"""`sunrise-clients-only`, Sunrise's own server.properties key, end to end on a dedicated server.

    python3 tools/server_sunrise_only_e2e.py --bin-dir target/fast

- off (the default): a client that sends no brand joins, as on vanilla.
- on (`sunrise-clients-only=true`): a client with no brand, or with vanilla's "vanilla", is disconnected during
  configuration with the server's reason, and the console says why; a client whose `minecraft:brand` is
  "sunrise" joins. (Any client can send that brand: the gate keeps out players who didn't install Sunrise, not
  cheaters.)

Both executables are checked, because the reason depends on the parts a build registers:
- the plain `sunrise-game` build (`sunrise-client`, the engine a packaged headless server runs) registers no
  Sunrise parts, so it sends the engine's neutral "This server only accepts its own game client.";
- the official `sunrise-official` executable registers Sunrise's website hook, so it sends the branded
  "This server is for the Sunrise client: playsunrise.net" as a translatable text.

sunrise-bots stand in for the clients (`--brand`). Each server runs in a fresh folder with fresh SUNRISE_HOME
and XDG folders (lab.game_env), on a free port. Build first:
    cargo build --profile fast -p sunrise-game --bin sunrise-client -p sunrise-official --bin sunrise-official
    cargo build --profile fast -p sunrise-bots
"""
import argparse
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))
from tools import lab
from tools.server_bench import DONE, Server  # noqa: E402

EXE = '.exe' if os.name == 'nt' else ''
PLAIN = f'sunrise-client{EXE}'
OFFICIAL = f'sunrise-official{EXE}'
NEUTRAL = 'This server only accepts its own game client.'
BRANDED = 'This server is for the Sunrise client: playsunrise.net'
PROPERTIES = {
    'online-mode': 'false',
    'white-list': 'false',
    'level-type': 'minecraft\\:flat',
    'generate-structures': 'false',
    'gamemode': 'creative',
    'force-gamemode': 'true',
    'view-distance': '3',
    'simulation-distance': '3',
}


class Failed(Exception):
    pass


def bot(bins, port, out, name, *extra):
    """One bot for a few seconds: its report's entry."""
    report = out / f'{name}.json'
    subprocess.run([bins / f'sunrise-bots{EXE}', '--server', f'127.0.0.1:{port}', '--bots', '1',
                    '--behaviour', 'idle', '--seconds', '3', '--name', name, '--report', report, *extra],
                   check=False, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=180)
    return json.loads(report.read_text())['per_bot'][0]


def server(bins, folder, out, name, extra, exe=None):
    folder.mkdir(parents=True)
    port = lab.free_port()
    props = dict(PROPERTIES, **extra, **{'server-port': str(port)})
    (folder / 'server.properties').write_text(''.join(f'{k}={v}\n' for k, v in props.items()))
    (folder / 'eula.txt').write_text('eula=true\n')
    command = [exe or bins / PLAIN, '--resources', lab.RESOURCES, '--dedicated', '--nogui']
    s = Server(command, folder, out / f'{name}.log')
    s.wait_for(DONE, 300)
    return s, port


def joined(entry):
    return entry['joined_after_ms'] > 0 and not entry['error']


def check_off(bins, exe, tag, scratch, out):
    """With the key off both builds let a client without a brand in, as vanilla does."""
    s, port = server(bins, scratch / f'{tag}-off', out, f'{tag}-off', {}, exe=exe)
    try:
        plain = bot(bins, port, out, f'{tag}Plain')
    finally:
        s.stop()
    if not joined(plain):
        raise Failed(f'{tag} off: a client without a brand was refused: {plain["error"]}')
    print(f'{tag} off: a client without a brand joins')


def check_on(bins, exe, tag, reason, scratch, out):
    """With the key on both builds turn away other brands with their own reason, wire and console."""
    s, port = server(bins, scratch / f'{tag}-on', out, f'{tag}-on',
                     {'sunrise-clients-only': 'true'}, exe=exe)
    try:
        results = {name: bot(bins, port, out, f'{tag}{name}', *extra) for name, extra in
                   [('None', ()), ('Vanilla', ('--brand', 'vanilla')), ('Sunrise', ('--brand', 'sunrise'))]}
        lines = s.since(0)
    finally:
        s.stop()
    for name in ('None', 'Vanilla'):
        entry = results[name]
        # The wire reason, resolved by a client that has no Sunrise translation key: the
        # component's English fallback.
        if joined(entry) or reason not in (entry['error'] or ''):
            raise Failed(f'{tag} on: {name} was not turned away with {reason!r}: {entry["error"]}')
        # The console line carries the reason the client was given (lan/conn.rs).
        if not any(f'Disconnecting {tag}{name}0' in l and reason in l for l in lines):
            raise Failed(f'{tag} on: the console did not say why {tag}{name}0 was turned away')
    if not joined(results['Sunrise']):
        raise Failed(f'{tag} on: the Sunrise client was refused: {results["Sunrise"]["error"]}')
    print(f'{tag} on: no brand and "vanilla" are turned away with {reason!r}; "sunrise" joins')


def check(bins, plain, official, scratch, out):
    check_off(bins, plain, 'plain', scratch, out)
    check_on(bins, plain, 'plain', NEUTRAL, scratch, out)
    check_off(bins, official, 'official', scratch, out)
    check_on(bins, official, 'official', BRANDED, scratch, out)


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--bin-dir', default=str(lab.ROOT / 'target/fast'))
    parser.add_argument('--plain-bin', default=PLAIN,
                        help=f'name in --bin-dir or a path (default: {PLAIN})')
    parser.add_argument('--official-bin', default=OFFICIAL,
                        help=f'name in --bin-dir or a path (default: {OFFICIAL})')
    parser.add_argument('--out', default=str(lab.LOCAL / 'server-sunrise-only-e2e'))
    args = parser.parse_args()
    bins, out = Path(args.bin_dir).resolve(), Path(args.out)

    def binary(value):
        path = Path(value)
        return (path if path.parent != Path('.') else bins / path).resolve()

    plain, official = binary(args.plain_bin), binary(args.official_bin)
    for path in (plain, official):
        if not path.exists():
            sys.exit(f'{path} is missing: build both with cargo build --profile fast '
                     f'-p sunrise-game --bin sunrise-client -p sunrise-official --bin sunrise-official')
    if not lab.RESOURCES.exists():
        sys.exit(f'prepared game resources are missing at {lab.RESOURCES}: prepare them with '
                 f'python3 tools/lab.py prepare from your own Minecraft client and server jars')
    shutil.rmtree(out, ignore_errors=True)
    out.mkdir(parents=True)
    os.environ.update(lab.game_env())
    with tempfile.TemporaryDirectory(prefix='server-sunrise-only-e2e-') as scratch:
        try:
            check(bins, plain, official, Path(scratch), out)
        except (Failed, RuntimeError) as e:
            sys.exit(f'server_sunrise_only_e2e failed (logs in {out}): {e}')


if __name__ == '__main__':
    main()
