#!/usr/bin/env python3
"""A dedicated server saves connected players on save-all/stop/SIGTERM and keeps safe data after a panic.

`MinecraftServer.runServer` catches what the tick throws and still stops the server. Sunrise's
`stop_after_crash` does the same, but a panic can interrupt a guest's turn (the game holds that
player's state swapped in) or a tick that has the hub out. `SUNRISE_CRASH_FIXTURE` makes the tick
panic at one of those points. Each run below joins a real Sunrise client, panics the server, and
checks, from the files the server leaves:

- Connected player: save-all writes five diamonds while the player stays joined;
  stop writes the newer inventory of twelve diamonds.
- `guest`: the panic comes in the joined player's turn. The log names the player; their file is byte for
  byte what the previous clean stop wrote, and a block placed before the panic is still there on the next start.
- `hub`: the panic comes while a console line has the hub out of the game, with a player in
  (the fixture takes it out first, as a panic in the network tick leaves it). Same checks.
- Bystander: a second player, given diamonds and riding a boat, is in when the panic interrupts the
  first one's turn. A crash saves the players it did not interrupt as a stop does, each leaving with
  the vehicle they alone ride: the bystander's file holds the diamonds and the boat, and the world
  saved after them no longer has the boat (it would be there twice). The interrupted
  player keeps the files the previous clean stop wrote, and a lost hub saves only the world.
- Vanilla: the pinned 26.3 server runs the same bystander and crashes while they ride
  (`VanillaCrashServer`); what it leaves of them (their diamonds, their RootVehicle, the boat in the
  world) must be Sunrise's. A second pair of runs refuses every player write during the crash. Each
  server must show the bystander on the tagged boat, log their refused write and leave no data file
  of theirs (a runner whose read-only folder still takes a file, as root's does, fails the case
  instead of passing it). `PlayerDataStorage.save` only logs the failure and `PlayerList.remove`
  still takes the boat out, so vanilla loses it, and Sunrise must too. Vanilla has no interrupted
  player or lost hub (a throw leaves no player's state half applied); those two are Sunrise's own
  answers, unverified against vanilla and not compared.
"""
import hashlib
import math
import os
import re
import shutil
import signal
import subprocess
import sys
import tempfile
import threading
import uuid
import time
from pathlib import Path

try:
    from . import lab
    from .integration_env import run_game
    from .test_dedicated_server_parity import Server, write_server_dir, free_port
except ImportError:
    import lab
    from integration_env import run_game
    from test_dedicated_server_parity import Server, write_server_dir, free_port

PLAYER = 'Tester'


def binary():
    metadata = subprocess.check_output(['cargo', 'metadata', '--no-deps', '--format-version', '1'], cwd=lab.ROOT, text=True)
    import json
    return Path(json.loads(metadata)['target_directory']) / 'fast' / 'sunrise-client'


def start(directory, fixture=None, guest=None):
    """The server, with `fixture` set to panic at that point (in `guest`'s turn only, when named)."""
    if fixture:
        os.environ['SUNRISE_CRASH_FIXTURE'] = fixture
    if guest:
        os.environ['SUNRISE_CRASH_FIXTURE_GUEST'] = guest
    try:
        server = Server([binary(), '--resources', lab.RESOURCES, '--dedicated', '--nogui'], directory,
                        f'server-{fixture or "clean"}{"-" + guest if guest else ""}')
    finally:
        os.environ.pop('SUNRISE_CRASH_FIXTURE', None)
        os.environ.pop('SUNRISE_CRASH_FIXTURE_GUEST', None)
    server.wait_for(r'Done \(', timeout=600)
    return server


def join(port, out):
    """A real client in a thread: it joins, chats and leaves, or loses its server."""
    report = out / 'report.json'

    def run():
        try:
            run_game([str(c) for c in [binary(), '--resources', lab.RESOURCES, '--render-distance', 2, '--server',
                                       f'127.0.0.1:{port}', '--name', PLAYER]],
                     cwd=out, env=lab.game_env(SUNRISE_NET_REPORT=report), stdout=subprocess.DEVNULL,
                     stderr=subprocess.STDOUT, timeout=120)
        except subprocess.TimeoutExpired:
            pass
    thread = threading.Thread(target=run, daemon=True)
    thread.start()
    return thread


def offline_uuid(name):
    """`UUIDUtil.createOfflinePlayerUUID`."""
    digest = bytearray(hashlib.md5(f'OfflinePlayer:{name}'.encode()).digest())
    digest[6] = digest[6] & 0x0F | 0x30
    digest[8] = digest[8] & 0x3F | 0x80
    return str(uuid.UUID(bytes=bytes(digest)))


def player_files(directory, name=PLAYER):
    """A joined player's files (data, advancements, stats), by content. The server's own profile is
    not theirs: its files are written by every save."""
    folder = directory / 'world' / 'players'
    me = offline_uuid(name)
    return {str(p.relative_to(folder)): hashlib.sha256(p.read_bytes()).hexdigest()
            for p in sorted(folder.rglob(f'*{me}*')) if p.is_file()}


def crash(directory, fixture, port, out, trigger):
    server = start(directory, fixture)
    try:
        server.console('forceload add 0 0')
        # The chunk loads a moment after the forceload.
        for _ in range(100):
            server.console(f'setblock 0 {200 + trigger["y"]} 0 minecraft:gold_block')
            time.sleep(0.5)
            if any('Changed the block' in l for l in server.lines):
                break
        else:
            raise AssertionError('the block was never placed:\n' + '\n'.join(server.lines[-20:]))
        client = join(port, out)
        server.wait_for(rf'{PLAYER} joined the game', timeout=300)
        if trigger.get('console'):
            server.console('say crash now')
        deadline = time.time() + 120
        while server.process.poll() is None and time.time() < deadline:
            time.sleep(0.2)
        assert server.process.poll() not in (None, 0), 'the server did not exit with an error after the panic'
        client.join(timeout=10)
    finally:
        server.stop()
    return '\n'.join(server.lines)


def connected_saves(build):
    """Save-all and stop persist a player who is still connected, before logout can save them."""
    try:
        from . import anvil_dump, lan_e2e
    except ImportError:
        import anvil_dump
        import lan_e2e
    directory = build / 'connected-server'
    port = free_port()
    write_server_dir(directory, port)
    server = start(directory)
    client = lan_e2e.Game('autosave', binary().with_name('sunrise-ctl'), lab.RESOURCES)
    file = directory / 'world' / 'players' / 'data' / f'{offline_uuid(PLAYER)}.dat'

    def diamonds():
        player = anvil_dump.plain(anvil_dump.read_dat(file), full=True)
        return sum(item['count'] for item in player['Inventory'] if item['id'] == 'minecraft:diamond')

    try:
        client.raw('launch', '--headless', '--game', binary())
        client.raw('connect', f'127.0.0.1:{port}', PLAYER)
        client.raw('wait-for', 'loaded', '--timeout', 300)
        server.wait_for(rf'{PLAYER} joined the game')
        assert not file.exists(), 'the player was already saved before save-all'
        server.console(f'give {PLAYER} minecraft:diamond 5')
        server.console('save-all flush')
        # Console commands execute in order: the marker follows the save barrier.
        server.console('say SAVED-CONNECTED-PLAYER')
        server.wait_for(r'\[Server\] SAVED-CONNECTED-PLAYER')
        assert not any(f'{PLAYER} left the game' in line for line in server.lines)
        assert diamonds() == 5, 'save-all did not persist the connected player inventory'
        server.console(f'give {PLAYER} minecraft:diamond 7')
        server.console('say STOP-WITH-CONNECTED-PLAYER')
        server.wait_for(r'\[Server\] STOP-WITH-CONNECTED-PLAYER')
        assert not any(f'{PLAYER} left the game' in line for line in server.lines)
        server.stop()
        assert diamonds() == 12, 'stop did not persist the newer connected player inventory'
    finally:
        client.close()
        server.stop()
    print('connected player save-all and stop: ok', flush=True)


def signal_saves(build):
    """SIGTERM stops the server as `stop` does (vanilla's shutdown hook): a connected player's
    inventory and ender chest changed since the last save are in their file afterwards."""
    try:
        from . import anvil_dump, lan_e2e
    except ImportError:
        import anvil_dump
        import lan_e2e
    directory = build / 'signal-server'
    port = free_port()
    write_server_dir(directory, port)
    server = start(directory)
    client = lan_e2e.Game('sigterm', binary().with_name('sunrise-ctl'), lab.RESOURCES)
    file = directory / 'world' / 'players' / 'data' / f'{offline_uuid(PLAYER)}.dat'
    try:
        client.raw('launch', '--headless', '--game', binary())
        client.raw('connect', f'127.0.0.1:{port}', PLAYER)
        client.raw('wait-for', 'loaded', '--timeout', 300)
        server.wait_for(rf'{PLAYER} joined the game')
        server.console(f'give {PLAYER} minecraft:diamond 9')
        server.console(f'item replace entity {PLAYER} enderchest.0 with minecraft:emerald 5')
        server.console('say GIVEN-BEFORE-SIGTERM')
        server.wait_for(r'\[Server\] GIVEN-BEFORE-SIGTERM')
        assert not file.exists(), 'the player was saved before the signal'
        server.process.send_signal(signal.SIGTERM)
        try:
            server.process.wait(timeout=120)
        except subprocess.TimeoutExpired:
            raise AssertionError('the server did not stop within 120 s of SIGTERM')
        server.reader.join(timeout=10)
        assert any('Saving players' in line for line in server.lines), \
            'SIGTERM ended the server without saving:\n' + '\n'.join(server.lines[-10:])
        assert file.exists(), 'SIGTERM ended the server without writing the player file'
        player = anvil_dump.plain(anvil_dump.read_dat(file), full=True)
        diamonds = sum(i['count'] for i in player['Inventory'] if i['id'] == 'minecraft:diamond')
        emeralds = sum(i['count'] for i in player.get('EnderItems', []) if i['id'] == 'minecraft:emerald')
        assert (diamonds, emeralds) == (9, 5), f'saved {diamonds} diamonds and {emeralds} ender chest emeralds, not 9 and 5'
    finally:
        client.close()
        server.stop()
    print('connected player saved on SIGTERM: ok', flush=True)


BYSTANDER = 'Bystander'


def refusing(folder):
    """Makes `folder` read-only, and proves it refuses a new file: a privileged runner (root) writes through `0555`,
    and a case meant to refuse every player write would then pass on two successful writes."""
    folder.mkdir(parents=True, exist_ok=True)
    folder.chmod(0o555)
    probe = folder / 'refused-write-probe'
    try:
        probe.write_bytes(b'')
    except PermissionError:
        return
    probe.unlink()
    folder.chmod(0o755)
    raise AssertionError(f'{folder} is read-only yet still takes a new file (running as root?): this runner cannot '
                         'refuse a player write, so the refused-write case cannot be checked here')


def ride_then_crash(server, directory, port, crash, refused=None):
    """The bystander joins `server`, is given four diamonds and rides a tagged boat beside a tagged armor stand that
    stays in the world; `crash()` then makes the server crash and returns once it has exited. With `refused`, a
    pattern for the server's log line on a failed player write, the player data folder is read-only during the
    crash: the log must show the bystander's write was attempted and refused, and no file of theirs may be left.
    Returns the bystander's block x and z (a vanilla world's spawn need not be near 0, 0)."""
    try:
        from . import lan_e2e
    except ImportError:
        import lan_e2e
    file = directory / 'world' / 'players' / 'data' / f'{offline_uuid(BYSTANDER)}.dat'
    other = lan_e2e.Game('bystander', binary().with_name('sunrise-ctl'), lab.RESOURCES)
    try:
        other.raw('launch', '--headless', '--game', binary())
        other.raw('connect', f'127.0.0.1:{port}', BYSTANDER)
        other.raw('wait-for', 'loaded', '--timeout', 300)
        server.wait_for(rf'{BYSTANDER} joined the game')
        server.console(f'give {BYSTANDER} minecraft:diamond 4')
        server.console(f'execute at {BYSTANDER} run summon minecraft:oak_boat ~ ~ ~ {{Tags:["bystander_boat"]}}')
        server.console(f'ride {BYSTANDER} mount @e[tag=bystander_boat,limit=1]')
        server.console(f'execute as {BYSTANDER} on vehicle if entity @s[tag=bystander_boat] run say BYSTANDER-MOUNTED')
        # Stays in the world: once the restart shows it, the chunk's entities have loaded.
        server.console(f'execute at {BYSTANDER} run summon minecraft:armor_stand ~ ~ ~ {{Tags:["bystander_anchor"],NoGravity:1b}}')
        server.console('say GIVEN-TO-BYSTANDER')
        server.wait_for(r'\[Server\] GIVEN-TO-BYSTANDER')
        assert any('BYSTANDER-MOUNTED' in l for l in server.lines), 'the bystander is not riding the tagged boat'
        assert not file.exists(), 'the bystander was saved before the crash'
        mark = len(server.lines)
        server.console(f'data get entity {BYSTANDER} Pos')
        server.wait_for(r'has the following entity data: \[')
        pos = next(re.search(r'\[(-?[\d.E-]+)d, (-?[\d.E-]+)d, (-?[\d.E-]+)d\]', line)
                   for line in server.lines[mark:] if 'has the following entity data' in line)
        if refused:
            refusing(file.parent)
        try:
            crash()
            if refused:
                server.reader.join(timeout=10)
                assert any(re.search(refused, l) for l in server.lines), \
                    f'no refused write of the bystander in the log ({refused!r}):\n' + '\n'.join(server.lines[-30:])
                left = sorted(p.name for p in file.parent.iterdir() if offline_uuid(BYSTANDER) in p.name)
                assert not left, f'a refused write still left the bystander\'s files: {left}'
        finally:
            if refused:
                file.parent.chmod(0o755)
    finally:
        other.close()
        server.stop()
    return math.floor(float(pos.group(1))), math.floor(float(pos.group(3)))


def bystander_outcome(directory, restart, at):
    """What a crash left of the bystander: the diamonds and the root vehicle in their file (None for both when it was
    never written), and whether the boat is still in the world once `restart()`'s server, with the bystander away,
    has loaded the armor stand beside it at `at`."""
    try:
        from . import anvil_dump
    except ImportError:
        import anvil_dump
    file = directory / 'world' / 'players' / 'data' / f'{offline_uuid(BYSTANDER)}.dat'
    outcome = {'diamonds': None, 'root_vehicle': None}
    if file.exists():
        player = anvil_dump.plain(anvil_dump.read_dat(file), full=True)
        outcome = {'diamonds': sum(i['count'] for i in player['Inventory'] if i['id'] == 'minecraft:diamond'),
                   'root_vehicle': player.get('RootVehicle', {}).get('Entity', {}).get('id')}
    x, z = at
    server = restart()
    try:
        server.console(f'forceload add {x - 16} {z - 16} {x + 16} {z + 16}')
        for _ in range(60):
            server.console('execute if entity @e[tag=bystander_anchor] run say ANCHOR-LOADED')
            time.sleep(1)
            if any('ANCHOR-LOADED' in l for l in server.lines):
                break
        else:
            raise AssertionError('the anchor beside the boat never loaded after the restart')
        server.console('execute if entity @e[tag=bystander_boat] run say BOAT-STILL-IN-WORLD')
        server.console('say BOAT-CHECKED')
        server.wait_for(r'\[Server\] BOAT-CHECKED', timeout=60)
        outcome['boat_in_world'] = any('BOAT-STILL-IN-WORLD' in l for l in server.lines)
    finally:
        server.stop()
    return outcome


def crash_saves_bystander(build, refuse=False):
    """A crash in one player's turn still saves the other player (MinecraftServer.stopServer), and
    leaves the interrupted player's files as the last clean stop wrote them. With `refuse` every
    player write fails: only vanilla's outcome says what then becomes of the bystander's boat."""
    name = 'refused' if refuse else 'bystander'
    directory = build / f'{name}-server'
    port = free_port()
    write_server_dir(directory, port)
    out = build / f'{name}-client'
    out.mkdir()
    server = start(directory)
    try:
        join(port, out).join(timeout=300)
        server.wait_for(rf'{PLAYER} left the game', timeout=60)
    finally:
        server.stop()
    saved = player_files(directory)
    assert saved, 'the clean run wrote no player file'
    server = start(directory, 'guest', guest=PLAYER)

    def crash():
        client = join(port, out)
        deadline = time.time() + 300
        while server.process.poll() is None and time.time() < deadline:
            time.sleep(0.2)
        assert server.process.poll() not in (None, 0), 'the server did not exit with an error after the panic'
        client.join(timeout=10)
    # `playerdata::save`'s warning when the file cannot be written.
    refused = rf'{offline_uuid(BYSTANDER)}\.dat is not saved' if refuse else None
    at = ride_then_crash(server, directory, port, crash, refused)
    log = '\n'.join(server.lines)
    assert f"The panic interrupted a player's turn ({PLAYER})" in log, 'the panic did not interrupt the player'
    assert 'Saving players' in log, 'the crash stopped the server without saving players'
    assert player_files(directory) == saved, f'the interrupted player\'s files changed: {saved} -> {player_files(directory)}'
    # Only the shutdown's leave save (PlayerList.remove) puts the boat they alone ride in their file, and the boat
    # leaves the world before it is saved (a world saved first keeps a second one in the chunk).
    outcome = bystander_outcome(directory, lambda: start(directory), at)
    if refuse:
        return outcome
    expected = {'diamonds': 4, 'root_vehicle': 'minecraft:oak_boat', 'boat_in_world': False}
    assert outcome == expected, f'the crash left the bystander as {outcome}, not {expected}'
    print('crash saves the bystander, keeps the interrupted player: ok', flush=True)
    return outcome


def vanilla_crash_saves_bystander(build, refuse=False):
    """The same bystander on the pinned vanilla server, crashed by `VanillaCrashServer` while the bystander rides: an
    error leaves a task on the server thread, as a throw in a player's packet would, and `runServer` stops the
    server. Vanilla has no interrupted player: the throw leaves no player's state half applied, so `stopServer`
    saves everyone. Sunrise keeping the interrupted player's last clean files, and a lost hub saving only the
    world, are its own answers to state vanilla never has; only the bystander is compared."""
    try:
        from . import test_rust_parity as base
    except ImportError:
        import test_rust_parity as base
    lock, jars, libraries = base.verify_artifacts()
    classes = build / 'vanilla-classes'
    classes.mkdir(exist_ok=True)
    dependencies = os.pathsep.join(map(str, [jars['server'], *libraries]))
    java_root = lab.ROOT / 'mods/sunrise/src/test/java'
    subprocess.run(['javac', '--release', str(lock['minecraft']['java_major']), '-proc:none', '-nowarn', '-sourcepath',
                    str(java_root), '-cp', dependencies, '-d', str(classes),
                    str(java_root / 'dev/sunriselab/parity/VanillaCrashServer.java')], check=True)
    java = ['java', '-Xmx2g', '-Duser.timezone=UTC']
    crashing = [*java, '--add-opens', 'java.base/java.lang=ALL-UNNAMED', '-cp', str(classes) + os.pathsep + dependencies,
                'dev.sunriselab.parity.VanillaCrashServer', 'nogui']
    plain = [*java, '-cp', dependencies, 'net.minecraft.server.Main', 'nogui']
    name = 'refused' if refuse else 'bystander'
    directory = build / f'vanilla-{name}-server'
    port = free_port()
    write_server_dir(directory, port)

    def started(command, label):
        server = Server(command, directory, label)
        server.wait_for(r'Done \(', timeout=600)
        return server
    server = started(crashing, f'vanilla-{name}-crash')

    def crash():
        (directory / 'crash-now').touch()
        deadline = time.time() + 300
        while server.process.poll() is None and time.time() < deadline:
            time.sleep(0.2)
        assert server.process.poll() is not None, 'the vanilla server did not stop after the crash'
    # `PlayerDataStorage.save`'s warning when the file cannot be written.
    refused = rf'Failed to save player data for {BYSTANDER}\b' if refuse else None
    at = ride_then_crash(server, directory, port, crash, refused)
    log = '\n'.join(server.lines)
    assert 'crash fixture' in log, 'the vanilla server did not crash on the fixture'
    assert 'Saving players' in log, 'the vanilla crash stopped the server without saving players'
    (directory / 'crash-now').unlink()
    return bystander_outcome(directory, lambda: started(plain, f'vanilla-{name}-restart'), at)


def main():
    subprocess.run(['cargo', 'build', '--profile', 'fast', '-p', 'sunrise-game', '-p', 'sunrise-client-lib', '--bin', 'sunrise-client', '--bin', 'sunrise-ctl'], cwd=lab.ROOT, check=True)
    build = lab.LOCAL / 'dedicated-crash-save'
    shutil.rmtree(build, ignore_errors=True)
    build.mkdir(parents=True)
    port = free_port()
    directory = build / 'server'
    write_server_dir(directory, port)
    out = build / 'client'
    out.mkdir()
    problems = []
    connected_saves(build)
    signal_saves(build)
    for refuse in (False, True):
        sunrise = crash_saves_bystander(build, refuse)
        vanilla = vanilla_crash_saves_bystander(build, refuse)
        case = 'with every player write refused' if refuse else 'saved'
        if sunrise != vanilla:
            problems.append(f'{case}: the crash left the bystander as {sunrise}; vanilla 26.3 left them as {vanilla}')
        else:
            print(f'crash {case}: the bystander as vanilla 26.3 leaves them ({vanilla}): ok', flush=True)

    # A clean stop writes the player's file.
    server = start(directory)
    try:
        join(port, out).join(timeout=300)
        server.wait_for(rf'{PLAYER} left the game', timeout=60)
    finally:
        server.stop()
    saved = player_files(directory)
    if not saved:
        problems.append('the clean run wrote no player file')

    for fixture, y, trigger, expect in (
        ('guest', 0, {'y': 0}, f"The panic interrupted a player's turn ({PLAYER})"),
        ('hub', 1, {'y': 1, 'console': True}, 'The panic interrupted the network tick'),
    ):
        log = crash(directory, fixture, port, out, trigger)
        if f'The server thread panicked: crash fixture at {fixture}' not in log:
            problems.append(f'{fixture}: no panic in the log')
        if expect not in log:
            problems.append(f'{fixture}: the log does not say {expect!r}')
        if 'Saving worlds' not in log:
            problems.append(f'{fixture}: the world was not saved')
        if player_files(directory) != saved:
            problems.append(f'{fixture}: a player file changed: {saved} -> {player_files(directory)}')
        # The next start finds the block placed before the panic.
        server = start(directory)
        try:
            server.console('forceload add 0 0')
            time.sleep(3)
            server.console(f'execute if block 0 {200 + y} 0 minecraft:gold_block run say FOUND-{fixture}')
            server.wait_for(rf'\[Server\] FOUND-{fixture}', timeout=60)
        except AssertionError:
            problems.append(f'{fixture}: the block placed before the panic is gone')
        finally:
            server.stop()
    for p in problems:
        print('FAIL', p)
    print('dedicated crash save:', 'ok' if not problems else 'FAILED')
    return 1 if problems else 0


if __name__ == '__main__':
    sys.exit(main())
