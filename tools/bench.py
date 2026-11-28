#!/usr/bin/env python3
"""Server load bench: the same sunrise-bots load on Sunrise, vanilla 26.3 or Pumpkin.

    python3 tools/bench.py --server sunrise --exe target/release/sunrise-server --bots 50
    python3 tools/bench.py --server vanilla --jar ~/mc/server.jar --bots 50 --pre-generated ~/worlds/seed12345
    python3 tools/bench.py --server pumpkin --exe ./pumpkin --bots 50

Every run uses a fresh scratch folder (bound to 127.0.0.1, offline, creative so the bots can fly), starts the
server, lets `sunrise-bots` join one every --join-every seconds, warms up, then measures for --seconds:

- TPS: `time query gametime` at the start and the end of the window, as game ticks per wall-clock second;
- MSPT: `tick query` every 5 s (average, P50, P95, P99 over the last 100 ticks), averaged over the window;
- RSS and CPU of the server's whole process tree, once a second (needs `ps`: Linux and macOS);
- bandwidth: `bytes_in`/`bytes_out` from the bots' report, per player and second online (each bot's whole time
  on the server, its join included), when the bots report them (they do not yet: the field is null until S-6); join time: the bots' `joined_after_ms`.

It writes one JSON report (--out) and prints the Markdown table of the design (docs/design.md, section 1).

Vanilla is the owner's own server.jar (--jar): nothing of Mojang's is fetched or committed. Pumpkin is a released
binary run as a black box (--exe); its Bedrock listeners, LAN broadcast and telemetry are turned off in the
config this script writes (--pumpkin-config adds a TOML file of your own on top if a release names a key
differently). `--pre-generated DIR` copies a finished world into every run's scratch folder as `world`, so runs
repeat on the same terrain. Build the bots first: cargo build --release -p sunrise-bots

`--spread BLOCKS` puts every bot at its own spot in a square of that side around the origin (the spots follow from
--seed, so every server gets the same ones): once all bots are in, the console teleports each one high above its
spot and then down onto the ground there, and the bots (run with --rehome) take that as home; the bots stay until
the window ends, however long the teleports took. With
`--bots-arg=--leash --bots-arg=96` they stay within 96 blocks of home, so a world saved with `--save-world` and given
back with `--pre-generated` is never generated further and every run ticks the same terrain. Every run reports how
many entities, players and items were loaded when its window ended. With
`--behaviour random` each bot then keeps switching between wandering, flying off, standing, hotbar switching,
attacking mobs and placing and breaking blocks. A spread run loads view distance around every bot, so give it a
longer --warmup: the window should start after the terrain around the bots is generated. Every Sunrise run records a
performance capture inside the measured window (40 s by default, shortened automatically when `--seconds` is shorter;
`--profile SECONDS` picks its length and `--no-profile` turns it off) and copies it into `--out` as
`<server>-<bots>.hzperf`; vanilla and Pumpkin runs record none, and every run states whether a capture was recording.
"""

import argparse
import datetime
import json
import os
from pathlib import Path
import platform
import random
import re
import shutil
import socket
import statistics
import subprocess
import sys
import tempfile
import threading
import time

ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT))
from tools import lab

EXE = '.exe' if os.name == 'nt' else ''
# Which console queries each server has; a figure it lacks is reported as UNSUPPORTED, one it has but never answers fails the run.
SERVERS = {
    'sunrise': {'gametime': True, 'tick_query': True},
    'vanilla': {'gametime': True, 'tick_query': True},
    'pumpkin': {'gametime': True, 'tick_query': False},
}
UNSUPPORTED = 'unsupported'

PROPERTIES = {
    'level-seed': '12345',
    'view-distance': '10',
    'simulation-distance': '10',
    'online-mode': 'false',
    'white-list': 'false',
    'gamemode': 'creative',
    'force-gamemode': 'true',
    'spawn-protection': '0',
    'max-players': '200',
    'difficulty': 'easy',
    'server-ip': '127.0.0.1',
    'enable-query': 'false',
    'enable-rcon': 'false',
}

# Pumpkin's two config files. Bedrock, LAN broadcast and telemetry (plugins' included) are off.
PUMPKIN_CONFIGURATION = '''\
server_address = "127.0.0.1:{port}"
seed = "{seed}"
max_players = 200
view_distance = {view}
simulation_distance = {view}
online_mode = false
default_difficulty = "Easy"
allow_chat_reports = false
encryption = false
lan_broadcast = false

[networking.bedrock]
enabled = false

[networking.lan_broadcast]
enabled = false

[networking.query]
enabled = false

[networking.rcon]
enabled = false
'''
PUMPKIN_FEATURES = '''\
[telemetry]
enabled = false

[plugins]
enabled = false
'''

DONE = re.compile(r'Done|listening|Server started', re.IGNORECASE)
TICK_AVERAGE = re.compile(r'Average time per tick: ([0-9.]+) ?ms')
TICK_PERCENTILES = re.compile(r'P50: ([0-9.]+) ?ms P95: ([0-9.]+) ?ms P99: ([0-9.]+) ?ms')
GAMETIME = re.compile(r'The game time is (-?\d+) tick\(s\)')
COUNTED = re.compile(r'Test passed[.,] count: (\d+)|Test failed', re.IGNORECASE)
TELEPORTED = re.compile(r'Teleported (\w+) to')
PROFILE_SUMMARY = re.compile(
    r'^(?:dedicated|integrated) server(?:,\s*([0-9]+(?:\.[0-9]+)?)\s*s)?(?:,\s*(\d+)\s*ticks)?$'
)
PROFILE_SAVED = re.compile(r'Saved\s+(.+?\.hzperf)(?:\s|$)')
PROFILE_DEFAULT_SECONDS = 40
PROFILE_MAX_SECONDS = 600
# Margin kept before and after a capture so it sits inside the measured window.
PROFILE_MARGIN = 5
# Above every terrain height, so the drop onto the ground is always a long one.
SPREAD_HEIGHT = 320
# How long spread bots stay at most; the bench stops them when its window ends.
SPREAD_CAP = 1800


def free_port():
    with socket.socket() as s:
        s.bind(('127.0.0.1', 0))
        return s.getsockname()[1]


def parse_tick_query(lines):
    """MSPT numbers from the lines `tick query` printed, or None until both lines are there."""
    average = percentiles = None
    for line in lines:
        if (m := TICK_AVERAGE.search(line)) and average is None:
            average = float(m.group(1))
        if (m := TICK_PERCENTILES.search(line)) and percentiles is None:
            percentiles = tuple(float(g) for g in m.groups())
    if average is None or percentiles is None:
        return None
    return {'average': average, 'p50': percentiles[0], 'p95': percentiles[1], 'p99': percentiles[2]}


def parse_gametime(lines):
    """The game time `time query gametime` printed ("The game time is N tick(s)"), or None until it is there."""
    for line in lines:
        if m := GAMETIME.search(line):
            return int(m.group(1))
    return None


def parse_count(lines):
    """What `execute if entity SELECTOR` printed: the count, 0 when nothing matched, None until it answered."""
    for line in lines:
        if m := COUNTED.search(line):
            return int(m.group(1) or 0)
    return None


def profile_min_seconds(profile_seconds):
    return profile_seconds + 2 * PROFILE_MARGIN


def resolve_profile(window_seconds, requested, disabled):
    """Capture length and an optional shortening note for a Sunrise run, or (None, None) when disabled."""
    if disabled:
        if requested is not None:
            raise ValueError('--profile cannot be combined with --no-profile')
        return None, None
    if requested is not None:
        if not 1 <= requested <= PROFILE_MAX_SECONDS:
            raise ValueError(f'--profile must be between 1 and {PROFILE_MAX_SECONDS} seconds (got {requested})')
        minimum = profile_min_seconds(requested)
        if window_seconds < minimum:
            raise ValueError(f'--profile {requested} needs --seconds of at least {minimum} (got {window_seconds})')
        return requested, None
    max_fit = window_seconds - 2 * PROFILE_MARGIN
    if max_fit < 1:
        minimum = profile_min_seconds(1)
        raise ValueError(
            f'--seconds {window_seconds} is too short for a capture (needs at least {minimum}, or pass --no-profile)'
        )
    if max_fit < PROFILE_DEFAULT_SECONDS:
        note = f'capture shortened from {PROFILE_DEFAULT_SECONDS} s to {max_fit} s to fit inside the {window_seconds} s window'
        return max_fit, note
    return PROFILE_DEFAULT_SECONDS, None


def profile_delay(window_seconds, profile_seconds):
    """Seconds after the window starts to launch `sunrise-server report` so it stays inside the window."""
    minimum = profile_min_seconds(profile_seconds)
    if window_seconds < minimum:
        raise ValueError(f'--profile {profile_seconds} needs --seconds of at least {minimum} (got {window_seconds})')
    return (window_seconds - profile_seconds) / 2


def parse_profile_output(lines):
    """Path, recorded seconds and tick count from `sunrise-server report`'s output."""
    path = seconds = ticks = None
    for line in lines:
        stripped = line.strip()
        if (m := PROFILE_SUMMARY.match(stripped)) and (m.group(1) or m.group(2)):
            if m.group(1) is not None:
                seconds = float(m.group(1))
            if m.group(2) is not None:
                ticks = int(m.group(2))
        if (m := PROFILE_SAVED.search(stripped)) and path is None:
            path = m.group(1)
    return {'path': path, 'seconds': seconds, 'ticks': ticks}


def report_command(exe, scratch, seconds):
    return [str(Path(exe).resolve()), '--dir', str(scratch), 'report', str(seconds)]


def start_profile(exe, scratch, seconds):
    env = lab.game_env(SUNRISE_HOME=scratch / 'sunrise-home')
    return subprocess.Popen(report_command(exe, scratch, seconds), cwd=scratch, stdin=subprocess.DEVNULL,
                            stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True,
                            encoding='utf-8', errors='replace', env=env)


def stop_profile(proc):
    if proc is not None and proc.poll() is None:
        proc.terminate()
        try:
            proc.wait(timeout=5)
        except subprocess.TimeoutExpired:
            proc.kill()
            proc.wait()


def find_capture(scratch, reported_path):
    if reported_path:
        candidate = Path(reported_path)
        if not candidate.is_absolute():
            candidate = scratch / candidate
        if candidate.is_file():
            return candidate
    reports = Path(scratch) / 'reports'
    if reports.is_dir():
        matches = sorted(reports.glob('profile-*.hzperf'))
        if matches:
            return matches[-1]
    return None


def finish_profile(proc, start_error, scratch, destination, requested_seconds):
    """Waits for `sunrise-server report`, copies the capture into --out and returns the report entry."""
    if start_error is not None:
        return {'recording': False, 'error': start_error}
    if proc is None:
        return {'recording': False, 'error': 'capture did not start inside the measured window'}
    timeout = requested_seconds + 90
    try:
        stdout, stderr = proc.communicate(timeout=timeout)
    except subprocess.TimeoutExpired:
        proc.kill()
        proc.communicate()
        return {'recording': True, 'error': f'sunrise-server report timed out after {timeout} s'}
    except OSError as e:
        return {'recording': True, 'error': str(e)}
    if proc.returncode != 0:
        detail = (stderr or stdout or '').strip().splitlines()
        suffix = f': {detail[-1]}' if detail else ''
        return {'recording': True, 'error': f'sunrise-server report exited with {proc.returncode}{suffix}'}
    parsed = parse_profile_output(stdout.splitlines())
    source = find_capture(scratch, parsed['path'])
    if source is None:
        return {'recording': True, 'error': 'sunrise-server report finished without a capture file'}
    try:
        shutil.copy2(source, destination)
    except OSError as e:
        return {'recording': True, 'error': f'could not copy capture to {destination}: {e}'}
    return {
        'recording': True,
        'path': str(destination),
        'seconds': parsed['seconds'],
        'ticks': parsed['ticks'],
    }


def tps(start_ticks, end_ticks, seconds):
    """Game ticks per wall-clock second over the window."""
    if seconds <= 0:
        raise ValueError(f'a window of {seconds} s has no tick rate')
    return round((end_ticks - start_ticks) / seconds, 2)


def percentile(values, p):
    if not values:
        return None
    ordered = sorted(values)
    return ordered[min(len(ordered) - 1, int(p / 100 * len(ordered)))]


def cpu_seconds(text):
    """ps's cumulative CPU time: [[DD-]HH:]MM:SS[.ss]."""
    days, _, rest = text.rpartition('-')
    seconds = 0.0
    for part in rest.split(':'):
        seconds = seconds * 60 + float(part)
    return seconds + (float(days) * 86400 if days else 0)


def tree_usage(root):
    """RSS (bytes) and CPU seconds of a process and all its descendants; None without `ps`."""
    try:
        out = subprocess.run(['ps', '-A', '-o', 'pid=,ppid=,rss=,time='], capture_output=True, text=True).stdout
    except OSError:
        return None
    rows = {}
    for line in out.splitlines():
        parts = line.split()
        if len(parts) == 4:
            rows[int(parts[0])] = (int(parts[1]), int(parts[2]) * 1024, cpu_seconds(parts[3]))
    tree, frontier = {root}, [root]
    while frontier:
        parent = frontier.pop()
        for pid, (ppid, _, _) in rows.items():
            if ppid == parent and pid not in tree:
                tree.add(pid)
                frontier.append(pid)
    return sum(rows[p][1] for p in tree if p in rows), sum(rows[p][2] for p in tree if p in rows)


def prepare(kind, directory, port, args):
    """Writes the server's settings into its scratch folder and returns the command that starts it."""
    directory = Path(directory)
    if args.pre_generated:
        shutil.copytree(args.pre_generated, directory / 'world')
    props = dict(PROPERTIES, **{'server-port': str(port), 'level-seed': str(args.seed),
                                'view-distance': str(args.view_distance),
                                'simulation-distance': str(args.view_distance)})
    if kind == 'pumpkin':
        config = directory / 'config'
        config.mkdir()
        (config / 'configuration.toml').write_text(PUMPKIN_CONFIGURATION.format(
            port=port, seed=args.seed, view=args.view_distance))
        (config / 'features.toml').write_text(PUMPKIN_FEATURES)
        if args.pumpkin_config:
            with (config / 'configuration.toml').open('a') as f:
                f.write('\n' + Path(args.pumpkin_config).read_text())
        return [Path(args.exe).resolve()]
    (directory / 'server.properties').write_text(''.join(f'{k}={v}\n' for k, v in props.items()))
    (directory / 'eula.txt').write_text('eula=true\n')
    if kind == 'vanilla':
        return ['java', f'-Xms{args.heap}', f'-Xmx{args.heap}', '-jar', Path(args.jar).resolve(), 'nogui']
    server = Path(args.exe).resolve()
    subprocess.run([server, '--dir', directory, 'import', directory / 'server.properties'], check=True,
                   capture_output=True)
    toml = directory / 'server.toml'
    # No web panel in a bench.
    toml.write_text(re.sub(r'(?m)^bind = "127\.0\.0\.1:8130"', 'bind = ""', toml.read_text()))
    return [server, '--dir', directory, 'run']


class Server:
    """A server with its console on stdin and its output collected."""

    def __init__(self, command, directory, log_path):
        self.lines = []
        self.lock = threading.Lock()
        self.log = log_path.open('w', encoding='utf-8')
        env = lab.game_env(SUNRISE_HOME=directory / 'sunrise-home')
        self.process = subprocess.Popen([str(c) for c in command], cwd=directory, stdin=subprocess.PIPE,
                                        stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True,
                                        encoding='utf-8', errors='replace', env=env)
        threading.Thread(target=self._read, daemon=True).start()

    def _read(self):
        for line in self.process.stdout:
            with self.lock:
                self.lines.append(line.rstrip('\n'))
            self.log.write(line)
            self.log.flush()

    def count(self):
        with self.lock:
            return len(self.lines)

    def since(self, n):
        with self.lock:
            return self.lines[n:]

    def wait_for(self, regex, timeout):
        deadline = time.time() + timeout
        while time.time() < deadline:
            if any(regex.search(line) for line in self.since(0)):
                return
            if self.process.poll() is not None:
                raise RuntimeError('the server exited:\n' + '\n'.join(self.since(0)[-20:]))
            time.sleep(0.1)
        raise RuntimeError(f'the server was not up within {timeout} s')

    def ask(self, command, parse, timeout=5):
        """Sends a console command and returns parse(its output lines); raises when nothing parsed in time."""
        start = self.count()
        try:
            self.process.stdin.write(command + '\n')
            self.process.stdin.flush()
        except OSError as e:
            raise RuntimeError(f'`{command}` could not be sent to the server: {e}') from e
        deadline = time.time() + timeout
        while time.time() < deadline:
            if (value := parse(self.since(start))) is not None:
                return value
            time.sleep(0.05)
        raise RuntimeError(f'`{command}` got no parseable answer within {timeout} s; its output: {self.since(start)}')

    def stop(self):
        if self.process.poll() is None:
            try:
                self.process.stdin.write('stop\n')
                self.process.stdin.flush()
                self.process.wait(timeout=60)
            except (BrokenPipeError, OSError, subprocess.TimeoutExpired):
                self.process.kill()
                self.process.wait()
        self.log.close()


def bots_command(args, port, total):
    return [str(args.bots_bin), '--server', f'127.0.0.1:{port}', '--bots', str(args.bots),
            '--behaviour', args.behaviour, '--join-every', str(args.join_every), '--seconds', str(int(total)),
            '--seed', str(args.seed), '--view-distance', str(args.view_distance), '--report', '-',
            *(['--rehome'] if args.spread else []), *args.bots_arg]


def spread_spots(bots, side, seed):
    """Every bot's block column in a square of `side` blocks around the origin, the same for a seed."""
    source = random.Random(seed)
    half = side // 2
    return [(source.randint(-half, half), source.randint(-half, half)) for _ in range(bots)]


def wait_for_port(port, process, timeout):
    """Returns once the server accepts connections: its "done" line can come before the listener is up."""
    deadline = time.time() + timeout
    while time.time() < deadline:
        if process.poll() is not None:
            raise RuntimeError('the server exited before it listened')
        try:
            socket.create_connection(('127.0.0.1', port), timeout=1).close()
            return
        except OSError:
            time.sleep(0.2)
    raise RuntimeError(f'the server did not listen on {port} within {timeout} s')


def teleported(name):
    def parse(lines):
        return True if any((m := TELEPORTED.search(line)) and m.group(1) == name for line in lines) else None
    return parse


def spread(server, args, settle=10):
    """Teleports each bot above its spot, waits for the chunks there, then drops each onto the ground."""
    spots = spread_spots(args.bots, args.spread, args.seed)
    names = [f'Bot{i}' for i in range(args.bots)]
    for name, (x, z) in zip(names, spots):
        server.ask(f'tp {name} {x} {SPREAD_HEIGHT} {z}', teleported(name), timeout=30)
    time.sleep(settle)
    for name in names:
        server.ask(f'execute at {name} positioned over motion_blocking run tp {name} ~ ~ ~', teleported(name),
                   timeout=30)
    return spots


def split_bot_output(lines):
    """The per-second samples (JSON lines with a `t`) and the final summary the bots printed."""
    samples, rest = [], []
    for line in lines:
        try:
            sample = json.loads(line)
        except ValueError:
            sample = None
        if isinstance(sample, dict) and 't' in sample:
            samples.append(sample)
        elif isinstance(sample, dict) and 'spawn' in sample:
            pass  # the first bot's spawn position, for a client that comes to watch
        else:
            rest.append(line)
    text = ''.join(rest)
    return samples, (json.loads(text) if text.strip() else {})


def bytes_per_player_second(per_bot, run_seconds, join_every):
    """The bots' bytes over the seconds they were connected: bot i connects i * join_every after the first and
    stays to the end of the run. So it is each bot's average from its login on, its join included, not the
    measuring window's alone (the bots report one total at the end). None until the bots count bytes (S-6)."""
    counted = [(i, b) for i, b in enumerate(per_bot) if 'bytes_in' in b and not b.get('error')]
    online = sum(max(run_seconds - i * join_every, 0) for i, _ in counted)
    if not counted or online <= 0:
        return None
    return {k: round(sum(b.get(k, 0) for _, b in counted) / online) for k in ('bytes_in', 'bytes_out')}


def summarize(kind, args, ticks, rss, cpu, gametime, window_seconds, summary, start_seconds, run_seconds):
    mean = lambda xs: round(statistics.fmean(xs), 2) if xs else None
    joins = [b['joined_after_ms'] for b in summary.get('per_bot', []) if b.get('joined_after_ms', 0) > 0]
    bandwidth = bytes_per_player_second(summary.get('per_bot', []), run_seconds, args.join_every)
    return {
        'server': kind,
        'bots': args.bots,
        'behaviour': args.behaviour,
        'start_seconds': start_seconds,
        'tps': UNSUPPORTED if gametime is None else tps(*gametime, window_seconds),
        'mspt': {k: UNSUPPORTED if ticks is None else mean([t[k] for t in ticks])
                 for k in ('average', 'p50', 'p95', 'p99')},
        'mspt_worst_p99': UNSUPPORTED if ticks is None else max((t['p99'] for t in ticks), default=None),
        'rss_mb': {'mean': mean([r / 1e6 for r in rss]), 'max': round(max(rss) / 1e6, 1) if rss else None},
        'cpu_percent': mean(cpu),
        'bytes_per_player_second': bandwidth,
        'join_ms': {'p50': percentile(joins, 50), 'worst': max(joins, default=None)},
        'bots_joined': summary.get('joined'),
        'bots_failed': [b for b in summary.get('per_bot', []) if b.get('error')],
        'chunks_total': summary.get('chunks'),
        'tick_queries': UNSUPPORTED if ticks is None else len(ticks),
    }


def measure_window(server, supports, seconds, profile=None, start_profile_fn=None,
                   clock=time.time, sleep=time.sleep, usage_fn=tree_usage):
    """Runs the measured window queries and optionally starts a capture inside it without blocking."""
    profile_at = profile_delay(seconds, profile) if profile is not None else None
    profile_proc = None
    profile_start_error = None
    try:
        first_time = server.ask('time query gametime', parse_gametime) if supports['gametime'] else None
        began = clock()
        rss, cpu = [], []
        ticks = [] if supports['tick_query'] else None
        usage = usage_fn(server.process.pid)
        last_cpu, last_time = (usage[1] if usage else 0), clock()
        next_query = clock()
        while clock() - began < seconds:
            sleep(1)
            now = clock()
            if (profile_at is not None and profile_proc is None and profile_start_error is None
                    and now - began >= profile_at):
                try:
                    profile_proc = start_profile_fn()
                except OSError as e:
                    profile_start_error = str(e)
            if usage and (u := usage_fn(server.process.pid)):
                rss.append(u[0])
                cpu.append((u[1] - last_cpu) / (now - last_time) * 100)
                last_cpu, last_time = u[1], now
            if ticks is not None and now >= next_query:
                ticks.append(server.ask('tick query', parse_tick_query))
                next_query = now + 5
        window = clock() - began
        last = server.ask('time query gametime', parse_gametime) if supports['gametime'] else None
        return (first_time, last), window, ticks, rss, cpu, profile_proc, profile_start_error
    except BaseException:
        stop_profile(profile_proc)
        raise


def run_once(kind, args, out, profile_seconds=None):
    scratch = Path(tempfile.mkdtemp(prefix=f'sunrise-bench-{kind}-'))
    if kind != 'sunrise':
        initial_profile = {'recording': False, 'reason': UNSUPPORTED}
    elif profile_seconds is None:
        initial_profile = {'recording': False, 'reason': 'disabled'}
    else:
        initial_profile = {'recording': False}
    result = {'server': kind, 'bots': args.bots, 'behaviour': args.behaviour, 'profile': initial_profile}
    server = None
    profile_proc = None
    try:
        port = free_port()
        command = prepare(kind, scratch, port, args)
        started = time.time()
        server = Server(command, scratch, out / f'{kind}-{args.bots}.log')
        server.wait_for(DONE, args.start_timeout)
        wait_for_port(port, server.process, 60)
        start_seconds = round(time.time() - started, 2)
        total = args.bots * args.join_every + args.warmup + args.seconds
        # A spread's teleports take as long as the server needs, so its bots run until they are told to stop.
        bots_started = time.time()
        bot_process = subprocess.Popen(bots_command(args, port, total + SPREAD_CAP if args.spread else total),
                                       stdin=subprocess.PIPE if args.spread else None, stdout=subprocess.PIPE,
                                       stderr=subprocess.PIPE, text=True)
        bot_lines = []
        threading.Thread(target=lambda: bot_lines.extend(bot_process.stdout), daemon=True).start()
        time.sleep(args.bots * args.join_every)
        if args.spread:
            time.sleep(5)
            spread(server, args)
        time.sleep(args.warmup)
        supports = SERVERS[kind]
        gametime, window, ticks, rss, cpu, profile_proc, profile_start_error = measure_window(
            server, supports, args.seconds,
            profile=profile_seconds,
            start_profile_fn=(lambda: start_profile(args.exe, scratch, profile_seconds))
            if profile_seconds is not None else None,
        )
        # What the server was ticking, so two runs can be told to have done the same work.
        loaded = {name: server.ask(f'execute if entity {selector}', parse_count, timeout=30)
                  for name, selector in (('entities', '@e'), ('players', '@a'), ('items', '@e[type=minecraft:item]'))}
        if args.spread:
            bot_process.stdin.write('stop\n')
            bot_process.stdin.flush()
            total = time.time() - bots_started
        bot_process.wait(timeout=total + 120)
        time.sleep(0.5)
        _, summary = split_bot_output(list(bot_lines))
        result.update(summarize(kind, args, ticks, rss, cpu, gametime if supports['gametime'] else None, window, summary, start_seconds,
                                total))
        result['loaded'] = loaded
        if profile_seconds is not None:
            result['profile'] = finish_profile(profile_proc, profile_start_error, scratch,
                                               out / f'{kind}-{args.bots}.hzperf', profile_seconds)
    except Exception as e:  # a failed run is reported, and the next one still runs
        result['error'] = str(e)
    finally:
        stop_profile(profile_proc)
        if server:
            server.stop()
        if args.save_world and (scratch / 'world').is_dir():
            shutil.copytree(scratch / 'world', args.save_world, dirs_exist_ok=True)
        shutil.rmtree(scratch, ignore_errors=True)
    return result


def format_profile(profile):
    if not isinstance(profile, dict):
        return '-'
    if 'error' in profile:
        return f'failed: {profile["error"].splitlines()[0]}'
    if not profile.get('recording', True):
        return UNSUPPORTED if profile.get('reason') == UNSUPPORTED else 'off'
    parts = []
    if profile.get('seconds') is not None:
        parts.append(f'{profile["seconds"]} s')
    if profile.get('ticks') is not None:
        parts.append(f'{profile["ticks"]} ticks')
    suffix = f' ({", ".join(parts)})' if parts else ''
    return f'{profile.get("path", "-")}{suffix}'


def table(runs):
    """The Markdown table of the design: one column per server, one row per measure."""
    ok = [r for r in runs if 'error' not in r]
    head = '| | ' + ' | '.join(r['server'] for r in ok) + ' |\n|---|' + '---|' * len(ok) + '\n'
    def cell(r, path):
        value = r
        for key in path:
            value = value.get(key) if isinstance(value, dict) else None
        return '-' if value is None else str(value)
    rows = [
        ('Players', ('bots',)),
        ('Entities loaded', ('loaded', 'entities')),
        ('TPS', ('tps',)),
        ('MSPT average', ('mspt', 'average')),
        ('MSPT p99 (mean of 5 s queries)', ('mspt', 'p99')),
        ('MSPT p99 (worst query)', ('mspt_worst_p99',)),
        ('RSS mean MB', ('rss_mb', 'mean')),
        ('CPU %', ('cpu_percent',)),
        ('Bytes in per player and s online', ('bytes_per_player_second', 'bytes_in')),
        ('Join p50 ms', ('join_ms', 'p50')),
        ('Join worst ms', ('join_ms', 'worst')),
        ('Start s', ('start_seconds',)),
    ]
    body = ''.join(f'| {name} | ' + ' | '.join(cell(r, path) for r in ok) + ' |\n' for name, path in rows)
    body += '| Profile | ' + ' | '.join(format_profile(r.get('profile')) for r in ok) + ' |\n'
    failed = ''.join(f'\n{r["server"]} failed: {r["error"].splitlines()[0]}\n' for r in runs if 'error' in r)
    return head + body + failed


def check(args):
    for kind in args.server:
        if kind not in SERVERS:
            sys.exit(f'--server: {kind!r} is not one of {", ".join(SERVERS)}')
    if args.no_profile and args.profile is not None:
        sys.exit('--profile cannot be combined with --no-profile')
    profile_seconds = profile_note = None
    if args.profile is not None or (not args.no_profile and 'sunrise' in args.server):
        try:
            profile_seconds, profile_note = resolve_profile(args.seconds, args.profile, args.no_profile)
        except ValueError as e:
            sys.exit(str(e))
    if 'vanilla' in args.server and not args.jar:
        sys.exit('vanilla needs --jar, your own server.jar')
    if any(k != 'vanilla' for k in args.server) and not args.exe:
        sys.exit('sunrise and pumpkin need --exe, the server binary')
    if not Path(args.bots_bin).exists():
        sys.exit(f'{args.bots_bin} is missing: cargo build --release -p sunrise-bots')
    if args.pre_generated and not Path(args.pre_generated).is_dir():
        sys.exit(f'--pre-generated {args.pre_generated} is not a folder')
    return profile_seconds, profile_note


def run_failed(run):
    return 'error' in run or (isinstance(run.get('profile'), dict) and 'error' in run['profile'])


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__.split('\n\n')[0])
    parser.add_argument('--server', type=lambda s: s.split(','), default=['sunrise'],
                        help='sunrise, vanilla or pumpkin (comma-separated, one run each)')
    parser.add_argument('--exe', help='the sunrise-server or Pumpkin binary')
    parser.add_argument('--jar', help="vanilla's server.jar (your own copy)")
    parser.add_argument('--bots-bin', default=str(ROOT / 'target/release' / f'sunrise-bots{EXE}'))
    parser.add_argument('--bots', type=int, default=50)
    parser.add_argument('--behaviour', default='mixed')
    parser.add_argument('--join-every', type=float, default=0.5)
    parser.add_argument('--spread', type=int, default=0, metavar='BLOCKS',
                        help='spread the bots over a square of this side (0: they stay at the world spawn)')
    parser.add_argument('--warmup', type=int, default=30)
    parser.add_argument('--seconds', type=int, default=60)
    profile_group = parser.add_mutually_exclusive_group()
    profile_group.add_argument('--profile', type=int, metavar='SECONDS',
                               help=f'length of Sunrise\'s performance capture inside the measured window '
                                    f'(default {PROFILE_DEFAULT_SECONDS} s, shortened if --seconds is shorter)')
    profile_group.add_argument('--no-profile', action='store_true',
                               help='do not record a performance capture on Sunrise runs')
    parser.add_argument('--seed', type=int, default=12345)
    parser.add_argument('--view-distance', type=int, default=10)
    parser.add_argument('--heap', default='4g', help='the Java heap for vanilla')
    parser.add_argument('--pre-generated', help='a finished world folder, copied into every run as `world`')
    parser.add_argument('--save-world', type=Path, help='copy the run\'s world here afterwards, to use as --pre-generated')
    parser.add_argument('--bots-arg', action='append', default=[], metavar='ARG',
                        help='an extra argument for sunrise-bots, such as --bots-arg=--leash --bots-arg=96 (repeatable)')
    parser.add_argument('--pumpkin-config', help='extra TOML appended to Pumpkin\'s configuration.toml')
    parser.add_argument('--start-timeout', type=int, default=600)
    parser.add_argument('--out', type=Path, default=ROOT / '.local/bench', help='the folder for report.json and logs')
    args = parser.parse_args(argv)
    profile_seconds, profile_note = check(args)
    if profile_note:
        print(f'Note: {profile_note}', flush=True)
    args.out.mkdir(parents=True, exist_ok=True)
    runs = []
    for kind in args.server:
        print(f'{kind}, {args.bots} {args.behaviour} bots...', flush=True)
        runs.append(run_once(kind, args, args.out, profile_seconds if kind == 'sunrise' else None))
        print(json.dumps(runs[-1]), flush=True)
    settings = {k: str(v) if isinstance(v, Path) else v for k, v in vars(args).items()}
    settings['profile'] = profile_seconds if 'sunrise' in args.server else None
    report = {
        'kind': 'sunrise-bench',
        'date': datetime.datetime.now(datetime.timezone.utc).isoformat(timespec='seconds'),
        'machine': {'system': platform.system(), 'machine': platform.machine(), 'cpus': os.cpu_count()},
        'settings': settings,
        'runs': runs,
    }
    path = args.out / 'report.json'
    path.write_text(json.dumps(report, indent=2) + '\n')
    print('\n' + table(runs) + f'\nWrote {path}')
    return 1 if any(run_failed(r) for r in runs) else 0


if __name__ == '__main__':
    sys.exit(main())
