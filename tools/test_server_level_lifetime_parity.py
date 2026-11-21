#!/usr/bin/env python3
"""The built-in levels' lifetime on a dedicated server with nobody in them, against vanilla's.

Vanilla keeps the Overworld, the Nether and the End in `MinecraftServer.levels` from `createLevels` to
`stopServer`; an idle level only unloads the chunks no ticket holds (`ChunkMap.processUnloads`). Sunrise saves and
closes an idle custom level after 300 ticks, but never a built-in one.

The pinned server JAR first generates a world (its chunk (0, 0) in the Nether and in the End saved); a vanilla
dedicated server and Sunrise's (`sunrise-client --dedicated --nogui`) then serve copies of it, with nobody joining,
and the console:

- idles past Sunrise's 300-tick idle close, then asks each built-in level for itself (`execute in`), and the
  Nether and the End whether their chunk (0, 0) is loaded (`execute if loaded`);
- force-loads chunk (0, 0) in the Nether and in the End, waits until both are loaded, and asks again;
- releases the Nether's, idles past the idle close again, and asks again: the released chunk is unloaded, the
  ticketed one is still loaded, and both levels still answer;
- stops the server, which saves every level still open (`Saving chunks for level ...`).

Compared exactly: the console's `say` lines from the questions, the forceload replies, and the stop's lines.
"""
import json
import os
from pathlib import Path
import platform
import re
import subprocess
import sys
import time

try:
    from . import lab, test_rust_parity as base
    from .test_dedicated_server_parity import (CONSOLE_LINE, Server, console_lines, free_port, wait_ticks,
                                               write_server_dir)
except ImportError:
    import lab
    import test_rust_parity as base
    from test_dedicated_server_parity import (CONSOLE_LINE, Server, console_lines, free_port, wait_ticks,
                                              write_server_dir)

NAME = 'server-level-lifetime-parity'
LEVELS = (('overworld', 'minecraft:overworld'), ('nether', 'minecraft:the_nether'), ('end', 'minecraft:the_end'))
# The levels whose chunk (0, 0) is forced and asked about (the Overworld's spawn is not this harness's business).
FORCED = ('minecraft:the_nether', 'minecraft:the_end')
# Past Sunrise's idle close of a custom level (`IDLE_TICKS`, 300).
IDLE_TICKS = 400
FORCELOAD = re.compile(r'force load')


def ask(server, step):
    """Each built-in level answers for itself, the Nether and the End for their chunk (0, 0); returns once the
    console has run them."""
    for short, level in LEVELS:
        server.console(f'execute in {level} run say {step} {short} answers')
        if level in FORCED:
            server.console(f'execute in {level} if loaded 0 64 0 run say {step} {short} 0 0 loaded')
            server.console(f'execute in {level} unless loaded 0 64 0 run say {step} {short} 0 0 not loaded')
    server.console(f'say {step} asked')
    server.wait_for(rf'\[Server\] {step} asked$', timeout=60)


def wait_loaded(server, levels, timeout=300):
    """Until chunk (0, 0) of each of `levels` is loaded; the polls are left out of the comparison."""
    deadline = time.time() + timeout
    for poll in range(1_000_000):
        start = len(server.lines)
        for level in levels:
            server.console(f'execute in {level} unless loaded 0 64 0 run say poll unloaded')
        server.console(f'say poll {poll}')
        server.wait_for(rf'\[Server\] poll {poll}$', timeout=max(1, deadline - time.time()), after=start)
        if not any(line.endswith('[Server] poll unloaded') for line in server.lines[start:]):
            return
        if time.time() > deadline:
            raise AssertionError(f'{server.log_path.name}: chunk (0, 0) of {levels} not loaded in {timeout} s')
        time.sleep(1)


def generate_world(server_command, build):
    """The world both servers load, made by vanilla, with chunk (0, 0) of the Nether and of the End saved."""
    directory = build / 'generate'
    write_server_dir(directory, free_port())
    server = Server(server_command, directory, 'generate')
    try:
        server.wait_for(r'Done \(')
        for level in FORCED:
            server.console(f'execute in {level} run forceload add 0 0')
        wait_loaded(server, FORCED)
        for level in FORCED:
            server.console(f'execute in {level} run forceload remove all')
        server.console('save-all flush')
        server.wait_for(r'Saved the game', timeout=120)
    finally:
        server.stop()
    return directory / 'world'


def run_side(label, command, build, world):
    directory = build / label
    write_server_dir(directory, free_port(), world)
    server = Server(command, directory, label)
    try:
        server.wait_for(r'Done \(')
        done = len(server.lines)
        wait_ticks(server, IDLE_TICKS)
        ask(server, 'idle')
        for level in FORCED:
            server.console(f'execute in {level} run forceload add 0 0')
        wait_loaded(server, FORCED)
        ask(server, 'forced')
        server.console('execute in minecraft:the_nether run forceload remove 0 0')
        wait_ticks(server, IDLE_TICKS)
        ask(server, 'released')
        for level in FORCED:
            server.console(f'execute in {level} run forceload query')
        server.console('say queried')
        server.wait_for(r'\[Server\] queried$', timeout=60)
    finally:
        server.stop()
    lines = server.lines[done:]
    forceload = [m.group(2) for line in lines if (m := CONSOLE_LINE.match(line)) and FORCELOAD.search(m.group(2))]
    said = [line for line in console_lines(lines) if ' poll ' not in line]
    return ['forceload ' + line for line in forceload] + said


def main():
    lock, jars, libraries = base.verify_artifacts()
    build = lab.LOCAL / NAME
    build.mkdir(parents=True, exist_ok=True)
    report = build / 'report.json'
    report.unlink(missing_ok=True)
    subprocess.run(['cargo', 'build', '--profile', 'fast', '-p', 'sunrise-game', '--bin', 'sunrise-client'], cwd=lab.ROOT,
                   check=True, capture_output=True)
    metadata = json.loads(subprocess.check_output(['cargo', 'metadata', '--no-deps', '--format-version', '1'],
                                                  cwd=lab.ROOT, text=True))
    binary = Path(metadata['target_directory']) / 'fast' / ('sunrise-client.exe' if os.name == 'nt' else 'sunrise-client')
    dependencies = os.pathsep.join(map(str, [jars['server'], *libraries]))
    vanilla_server = ['java', '-Xmx2g', '-Duser.timezone=UTC', '-cp', dependencies, 'net.minecraft.server.Main', 'nogui']
    sunrise_server = [binary, '--resources', lab.RESOURCES, '--dedicated', '--nogui']

    world = generate_world(vanilla_server, build)
    results = {label: run_side(label, command, build, world)
               for label, command in (('vanilla', vanilla_server), ('sunrise', sunrise_server))}
    for label, lines in results.items():
        (build / f'{label}.txt').write_text('\n'.join(lines) + '\n', encoding='utf-8')
    vanilla, sunrise = results['vanilla'], results['sunrise']
    if not any(line.endswith('[Server] released end 0 0 loaded') for line in vanilla):
        raise AssertionError(f'vanilla did not keep the ticketed End chunk: the script did not run (see {build})')
    if vanilla != sunrise:
        import difflib
        diff = '\n'.join(difflib.unified_diff(vanilla, sunrise, 'vanilla', 'sunrise', n=1, lineterm=''))
        raise AssertionError(f'transcripts differ (see {build}/vanilla.txt and sunrise.txt):\n{diff[:6000]}')
    lab.write_json(report, dict(
        schema=1, minecraft=lock['minecraft']['version'], platform=platform.platform(),
        java=subprocess.check_output(['java', '--version'], text=True).splitlines()[0],
        lines=len(vanilla), output_sha256=lab.digest(build / 'sunrise.txt', 'sha256'),
        scope='Built-in levels of an empty dedicated server idling past 300 ticks: each still answers, a released '
              'forced chunk unloads, a forced one stays loaded, and every level is saved at stop.'))
    print(f'server level lifetime: {len(vanilla)} lines identical')


if __name__ == '__main__':
    try:
        main()
    except AssertionError as error:
        sys.exit(f'{NAME}: {error}')
