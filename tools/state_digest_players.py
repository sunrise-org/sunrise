#!/usr/bin/env python3
"""State-digest scene with players: BINARY RESOURCES TEMPLATE OUT BOTS [--integrated] [--ctl CTL] [--nether] [--detail] [--ticks N] [--no-digest] [--coverage], or --compare OUT_A OUT_B [--ignore-group GROUP]...; template from `state_digest_scene.py --generate`, gate in SUNRISE_SCENE_GATE. --coverage adds the base scene's coverage commands, stands Alice in its village and Bob south of it, makes both immune to damage, gives Alice a returning Loyalty trident, defaults --ticks to 2500 and fails when a spot of coverage.txt is missed."""
import argparse
import json
import os
import re
import shutil
import socket
import subprocess
import sys
import tempfile
import threading
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))
from tools import lab, state_digest_scene as base
from tools.state_digest_compare import IGNORABLE_GROUPS, compare as compare_digest, compare_coverage

STATE_PATHS = ('Pos', 'Health', 'foodLevel', 'XpLevel', 'XpTotal', 'Inventory', 'active_effects[0].id', 'active_effects[1].id', 'Dimension')
PLAYERS = ['Alice0', 'Bob0']  # dedicated: two scripted bots; integrated: Alice0 joins the host's LAN
SETTLE_ATTEMPTS = 400  # frozen server ticks to wait for a join or a settle, before giving up


def free_port():
    with socket.socket() as s:
        s.bind(('127.0.0.1', 0))
        return s.getsockname()[1]


NETHER_SPOT = (8.5, 70, 8.5)  # Bob's spot with --nether


def spots(coverage):
    """Where Alice and Bob stand (x, z): among the mobs, or (--coverage) Alice by the village bell and Bob south of the village."""
    vx, vz = base.VILLAGE
    return ((vx - 0.5, vz + 1.5), (vx + 1.5, vz + 21.5)) if coverage else ((16.5, 9.5), (66.5, 61.5))


def stand(coverage, nether):
    """Each player's (x, y, z, dimension) once the scene put them there."""
    (ax, az), (bx, bz) = spots(coverage)
    bob = (*NETHER_SPOT, 'minecraft:the_nether') if nether else (bx, base.Y, bz, 'minecraft:overworld')
    return (ax, base.Y, az, 'minecraft:overworld'), bob


def player_commands(names, nether, coverage=False):
    """The scene after the bots joined: each one among the mobs, hurt, fed, given items and a pickup, one in the Nether."""
    a, b = names[0], names[-1]
    y = base.Y
    (ax, az), (bx, bz) = spots(coverage)
    commands = [
        f'gamemode survival {a}', f'gamemode survival {b}',
        f'tp {a} {ax} {y} {az}', f'tp {b} {bx} {y} {bz}',
        f'give {a} minecraft:iron_sword', f'give {a} minecraft:bread 16', f'give {a} minecraft:cooked_beef 8',
        f'give {b} minecraft:diamond_pickaxe', f'give {b} minecraft:cobblestone 64', f'give {b} minecraft:oak_planks 32',
        f'item replace entity {a} armor.chest with minecraft:iron_chestplate',
        f'item replace entity {b} weapon.offhand with minecraft:shield',
        f'effect give {b} minecraft:regeneration 600 1', f'effect give {a} minecraft:speed 600 0',
        f'xp add {a} 5 levels', f'xp add {b} 120 points',
        f'effect give {a} minecraft:resistance 600 3', f'damage {a} 3 minecraft:generic', f'damage {b} 4 minecraft:fall',
        f'setblock 66 {y} 58 minecraft:chest{{Items:[{{Slot:0b,id:"minecraft:apple",count:5}}]}}',
        f'setblock 64 {y} 58 minecraft:furnace',
        f'setblock 70 {y} 65 minecraft:crafting_table',
        f'time set {base.DAY_TIME if coverage else 1000}',
    ]
    if nether:
        commands.append(f'execute in minecraft:the_nether run tp {b} {NETHER_SPOT[0]} {NETHER_SPOT[1]} {NETHER_SPOT[2]}')
    if coverage:  # the run is long enough for the hostiles and the TNT to kill a standing bot
        commands += [f'effect give {n} minecraft:resistance infinite 4 true' for n in names]
    return commands


def near_player_commands(at, owner=None):
    """After the settle (frozen ticks in which players act): randoms pinned, then what reaches each player at `at` (`stand`), and Alice's Loyalty trident when `owner` (her UUID ints) is given."""
    def put(spot, what, dx, dy, dz, data=''):
        x, y, z, dim = spot
        command = f'summon minecraft:{what} {x + dx:.1f} {y + dy} {z + dz:.1f}' + (f' {data}' if data else '')
        return command if dim == 'minecraft:overworld' else f'execute in {dim} run {command}'
    alice, bob = at[0], at[-1]
    commands = [
        f'digest-sync {base.SYNC[0]} {base.SYNC[1] + 1000} {base.SYNC[2]}',
        put(alice, 'item', 1, 1, 0, '{Item:{id:"minecraft:emerald",count:3}}'),
        put(alice, 'experience_orb', -1, 1, 1, '{Value:7s}'),
        put(alice, 'zombie', 4, 0, 0), put(alice, 'skeleton', -4, 0, 3),
        put(bob, 'zombie', 4, 0, 0), put(bob, 'spider', -4, 0, -3),
        put(bob, 'tnt', 0, 0, 3, '{fuse:60s}'),
    ]
    if owner:
        loyal = '{"minecraft:enchantments":{"minecraft:loyalty":3}}'
        commands.append(put(alice, 'trident', 2, 4, -1, f'{{Tags:["cov_loyal"],pickup:1b,Owner:[I;{",".join(map(str, owner))}],'
                                                       f'item:{{id:"minecraft:trident",count:1,components:{loyal}}},Motion:[0.0d,-1.0d,0.0d]}}'))
    return commands


def uuid_ints(lines):
    """The four ints of a `data get entity <player> UUID` reply."""
    match = re.search(r'\[I;\s*(-?\d+),\s*(-?\d+),\s*(-?\d+),\s*(-?\d+)\]', '\n'.join(lines))
    if not match:
        raise SystemExit('no UUID in:\n' + '\n'.join(lines))
    return [int(v) for v in match.groups()]


class Bots:
    """Scripted `sunrise-bots` that stand where the server puts them: their per-tick moves repeat one position."""

    def __init__(self, binary, port, directory):
        self.binary, self.port, self.directory = binary, port, directory
        self.processes = []

    def join(self, server, prefix):
        at = len(server.lines)
        log = open(self.directory / f'bot-{prefix}.log', 'w')
        self.processes.append(subprocess.Popen(
            [self.binary, '--server', f'127.0.0.1:{self.port}', '--bots', '1', '--behaviour', 'stand', '--name', prefix,
             '--seconds', '3600', '--view-distance', '4', '--seed', '12345'], stdout=log, stderr=subprocess.STDOUT))
        server.wait_for(f'{prefix}0[/127.0.0.1:', at, 120)

    def stop(self):
        for process in self.processes:
            process.terminate()
        for process in self.processes:
            try:
                process.wait(20)
            except subprocess.TimeoutExpired:
                process.kill()


def settle(server, names, targets):
    """Frozen ticks run until each player stands where the scene put it, so the stepped ticks see steady moves."""
    for _ in range(SETTLE_ATTEMPTS):
        lines = server.run([f'data get entity {n} Pos' for n in names])
        text = '\n'.join(lines)
        if all(any(f'{t:.1f}' in text for t in want) for want in targets):
            return
        time.sleep(0.05)
    raise SystemExit(f'players did not settle:\n{text}')


def final_state(server):
    """The players’ final command-visible state."""
    lines = []
    for name in PLAYERS:
        for path in STATE_PATHS:
            lines += [l.split(']: ', 1)[-1] for l in server.run([f'data get entity {name} {path}'])
                      if 'following entity data' in l or 'No entity' in l]
    return '\n'.join(lines) + '\n'


def wait_loaded(server, dim, at):
    """Frozen ticks until the chunk under the player is loaded, then a few more for its neighbours."""
    for _ in range(SETTLE_ATTEMPTS):
        if any('Test passed' in l for l in server.run([f'execute in {dim} if loaded {at[0]} {at[1]} {at[2]}'])):
            break
        time.sleep(0.05)
    else:
        raise SystemExit(f'{dim} did not load')
    for _ in range(40):
        server.run(['say settle'])
        time.sleep(0.1)


def by_dimension(digest):
    """Group level headers and their player detail rows by dimension."""
    parts, current = {}, None
    with digest.open() as lines:
        for line in lines:
            if line.startswith('tick='):
                current = line.split()[1].removeprefix('dim=')
                parts.setdefault(current, []).append(line)
            elif current is not None and line.startswith('  p\t'):
                parts[current].append(line)
    return parts


def compare(a, b, ignore_groups=()):
    """Compare stepped digests per dimension, coverage, and final player data."""
    status = 0
    left, right = by_dimension(a / 'digest.txt'), by_dimension(b / 'digest.txt')
    compared = False
    for dim in sorted(left.keys() | right.keys()):
        print(f'== {dim}', flush=True)
        if dim not in left or dim not in right:
            print('only in one run')
            status = 1
            continue
        if not any(
            line.startswith('tick=') and 'run=1' in line.split()
            for lines in (left[dim], right[dim]) for line in lines
        ):
            print('no stepped tick digested in this level (frozen ticks only)')
            continue
        compared = True
        report, same = compare_digest(left[dim], right[dim], ignore_groups)
        print('\n'.join(report))
        status |= int(not same)
    if not compared:
        print('no stepped ticks compared')
        status = 1
    coverage = compare_coverage(a, b)
    if coverage:
        print('\n'.join(coverage[0]))
        status |= int(not coverage[1])
    same = (a / 'players.txt').read_text() == (b / 'players.txt').read_text()
    ignored = 'player_summary' in ignore_groups
    print('== player_summary')
    print('identical' if same else 'different (ignored)' if ignored else 'different')
    return status | int(not same and not ignored)


def server_properties(port):
    return dict(base.PROPERTIES, **{'server-port': str(port), 'white-list': 'false', 'enforce-whitelist': 'false'})


def run_dedicated(args, home, timings):
    out = args.out
    run_dir = out / 'server'
    if run_dir.exists():
        shutil.rmtree(run_dir)
    run_dir.mkdir(parents=True)
    port = free_port()
    (run_dir / 'server.properties').write_text(''.join(f'{k}={v}\n' for k, v in server_properties(port).items()))
    (run_dir / 'eula.txt').write_text('eula=true\n')
    shutil.copytree(args.template / 'world', run_dir / 'world')
    (run_dir / 'world' / 'session.lock').unlink(missing_ok=True)
    coverage = args.coverage
    if coverage:
        base.seed_coverage_world(run_dir / 'world')
    digest = None if args.no_digest else out / 'digest.txt'
    server = base.Server(args.binary, args.resources, run_dir, base.scratch_env(home, digest, args.detail), freeze=True)
    players = Bots(args.bots, port, out)
    ticks = args.ticks
    nether = args.nether
    missed = []
    try:
        server.wait_for('Done (')
        server.run(['forceload query'])
        time.sleep(3)
        server.run_seeded(base.scene_commands(False, coverage), require_sync=digest is not None)
        players.join(server, 'Alice')
        players.join(server, 'Bob')
        server.run(player_commands(PLAYERS, nether, coverage))
        at = stand(coverage, nether)
        settle(server, PLAYERS, [(x, z) for x, _, z, _ in at])
        if nether:
            wait_loaded(server, 'minecraft:the_nether', tuple(int(v) for v in NETHER_SPOT))
        owner = uuid_ints(server.run([f'data get entity {PLAYERS[0]} UUID'])) if coverage else None
        server.run_seeded(near_player_commands(at, owner), require_sync=digest is not None)
        before = server.game_time()
        start = time.time()
        server.run(['tick rate 10000', f'tick step {ticks}'])
        while server.game_time() < before + ticks:
            time.sleep(0.05)
        timings['step_seconds'] = time.time() - start
        (out / 'players.txt').write_text(final_state(server))
        if coverage:
            lines, missed = base.coverage_checks(server.run, run_dir / 'world', PLAYERS[0])
    finally:
        players.stop()
        server.stop()
    if coverage:
        more, also = base.pickup_stat_check(run_dir / 'world', PLAYERS[0], base.uuid_text(owner))
        (out / 'coverage.txt').write_text('\n'.join(lines + more) + '\n')
        missed += also
    if missed:
        raise SystemExit(f'--coverage missed: {", ".join(missed)} (see {out / "coverage.txt"})')


def ctl_run(ctl, socket_path, env, cwd, *words, timeout=600):
    result = subprocess.run([str(ctl), '--socket', str(socket_path), *map(str, words)], cwd=cwd, env=env,
                            capture_output=True, text=True, timeout=timeout)
    lines = result.stdout.strip().splitlines()
    reply = json.loads(lines[-1]) if lines else {'error': result.stderr.strip()}
    if result.returncode or 'error' in reply:
        raise SystemExit(f'ctl {words}: {reply}')
    return reply


def run_integrated(args, home, timings):
    """The host's window runs a server thread, opened to LAN; one bot joins as a guest beside the host's own player."""
    out = args.out
    ctl = Path(args.ctl or os.environ.get('SUNRISE_SCENE_CTL') or args.binary.parent / 'sunrise-ctl').resolve()
    work = out / 'host'
    if work.exists():
        shutil.rmtree(work)
    (work / 'game' / 'saves').mkdir(parents=True)
    world = work / 'game' / 'saves' / 'T'
    shutil.copytree(args.template / 'world', world)
    (world / 'session.lock').unlink(missing_ok=True)
    coverage = args.coverage
    if coverage:
        base.seed_coverage_world(world)
    digest = None if args.no_digest else out / 'digest.txt'
    env = lab.game_env(base.scratch_env(home, digest, args.detail), SUNRISE_HOME=work / 'home')
    sock = work / 'c.sock'

    def run(*words, timeout=600):
        return ctl_run(ctl, sock, env, work, *words, timeout=timeout)

    def command(line):
        return run('command', line)

    run('launch', '--world', 'T', '--headless', '--wait', '--game', args.binary,
        '--game-dir', work / 'game', '--resources', args.resources)
    bot_log = open(out / 'bot-Alice.log', 'w')
    players = Bots(args.bots, None, out)
    ticks = args.ticks
    missed = []
    try:
        run('wait-ready', 400)
        command('tick freeze')
        command('gamerule spawn_mobs false')
        command('gamerule random_tick_speed 0')
        port = run('lan', 'open', 0, 'commands')['port']
        try:
            reply = json.dumps(command('digest-sync ' + ' '.join(map(str, base.SYNC))))
            if 'Unknown' in reply or 'Incorrect' in reply or 'rror' in reply:
                raise SystemExit(reply)
            timings['digest_sync'] = 'ok'
        except SystemExit as e:
            timings['digest_sync'] = f'unavailable on the integrated server, so its digest is not comparable run to run: {e}'
        for line in base.scene_commands(False, coverage)[1:]:
            command(line)
        players.processes.append(subprocess.Popen(
            [args.bots, '--server', f'127.0.0.1:{port}', '--bots', '1', '--behaviour', 'stand', '--name', 'Alice', '--seconds', '3600',
             '--view-distance', '4', '--seed', '12345'], stdout=bot_log, stderr=subprocess.STDOUT))
        deadline = time.time() + 120
        while not any(g['name'] == 'Alice0' for g in run('lan', 'status')['guests']):
            if time.time() > deadline:
                raise SystemExit('Alice0 did not join the LAN')
            time.sleep(0.5)
        for line in player_commands(['Alice0'], False, coverage):
            command(line)
        owner = uuid_ints([json.dumps(command('data get entity Alice0 UUID'))]) if coverage else None
        for line in near_player_commands(stand(coverage, False), owner)[1:]:
            command(line)
        host = run('lan', 'status').get('host', 'Player')
        command(f'tp {host} 40.5 {base.Y} 40.5')
        command(f'give {host} minecraft:bread 4')
        time.sleep(3)
        before = game_time(command)
        start = time.time()
        command('tick rate 10000')
        command(f'tick step {ticks}')
        while game_time(command) < before + ticks:
            time.sleep(0.05)
        timings['step_seconds'] = time.time() - start
        (out / 'players.txt').write_text(integrated_state(command, host).replace(host, 'HOST'))
        if coverage:
            lines, missed = base.coverage_checks(lambda cs: [json.dumps(command(c)) for c in cs], world, 'Alice0')
    finally:
        players.stop()
        try:
            run('quit')
        except SystemExit:
            pass
    if coverage:
        more, also = base.pickup_stat_check(world, 'Alice0', base.uuid_text(owner))
        (out / 'coverage.txt').write_text('\n'.join(lines + more) + '\n')
        missed += also
    if missed:
        raise SystemExit(f'--coverage missed: {", ".join(missed)} (see {out / "coverage.txt"})')


def game_time(command):
    reply = json.dumps(command('time query gametime'))
    match = re.search(r'time is (\d+)', reply)
    if not match:
        raise SystemExit(f'no game time in {reply}')
    return int(match.group(1))


def integrated_state(command, host):
    lines = []
    for name in ('Alice0', host):
        for path in STATE_PATHS:
            lines.append(f'{name} {path} {json.dumps(command(f"data get entity {name} {path}"))}')
    return '\n'.join(lines) + '\n'


def main():
    if '--compare' in sys.argv[1:]:
        parser = argparse.ArgumentParser(description=__doc__)
        parser.add_argument('--compare', nargs=2, required=True, type=Path, metavar=('LEFT', 'RIGHT'))
        parser.add_argument('--ignore-group', action='append', choices=(*IGNORABLE_GROUPS, 'player_summary'), default=[])
        args = parser.parse_args()
        raise SystemExit(compare(*args.compare, args.ignore_group))
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('binary', 'resources', 'template', 'out', 'bots'):
        parser.add_argument(name, type=lambda path: Path(path).resolve())
    for flag in ('integrated', 'nether', 'detail', 'no-digest', 'coverage'):
        parser.add_argument(f'--{flag}', action='store_true')
    parser.add_argument('--ticks', type=int)
    parser.add_argument('--ctl', type=Path)
    args = parser.parse_args()
    if args.ticks is None:
        args.ticks = 2500 if args.coverage else 200
    out = args.out
    out.mkdir(parents=True, exist_ok=True)
    home = Path(tempfile.mkdtemp(prefix='state-digest-home-'))
    base.hold_gate()
    timings = {}
    try:
        (run_integrated if args.integrated else run_dedicated)(args, home, timings)
    finally:
        shutil.rmtree(home, ignore_errors=True)
    (out / 'timings.txt').write_text(f'{timings}\n')
    print(out, timings)


if __name__ == '__main__':
    main()
