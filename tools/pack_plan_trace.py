#!/usr/bin/env python3
"""What the frame planner (#6552) would save on real shader packs: a measure, not a proof.

    python3 tools/pack_plan_trace.py --packs DIR [--fixtures] [--bin-dir DIR] [--out DIR] [--resources DIR]
                                     [--prefix 'gamescope --backend headless --'] [--frames N]
                                     [--allow-missing CASE ...]

Builds the game with the `plan-trace` feature (or takes `--bin-dir`'s), and for each pack in DIR (a `.zip` or a
folder; `--fixtures` adds tools/fixtures/shaderpacks/*, frame_plan_cases among them) opens a creative world
offscreen with the pack, in spectator mode with every other entity gone and the world frozen, and records its frames
(SUNRISE_PLAN_TRACE, crates/sunrise-render-shaderpack/src/pack_post/plan_trace.rs):
- the pack's first frames (a `first` frame, then steady ones);
- a render-size change (the Resolution Scale slider) and back: `resize` frames;
- the Shadow Cache switched on and off again;
- a history reset (`screenshot-sequence`): `reset` frames.
Then `frame-plan-measure` projects every frame, checks each block's touches against its declared set and each
state's plan edits, and plans each render size apart; the table says per pack and size what R1 (dead draws), R4b
(folded clears) and R2 (dead sides) would remove, with the callbacks as the spec declares them (unresolved) and
narrowed to nothing (a ceiling, not sound). Bytes are per frame: the most one frame of that size holds.

The run must reach every case of CASES in some recorded frame (the guards each frame records): a Shadow Cache hit,
mod draws present and absent, a world callback that drew and none that did, and a size-skipped first `deferred`
followed by one that ran. A case no frame reached fails the run unless `--allow-missing CASE` names it.

Exits 1 when a pack doesn't load, records no frame, a check fails or a case is missing; the output names which.
"""

import argparse
import json
import os
import shlex
import shutil
import subprocess
import sys
import tempfile
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))
from tools import lab  # noqa: E402

EXE = '.exe' if os.name == 'nt' else ''
FIXTURES = ROOT / 'tools/fixtures/shaderpacks'
SIZE = (854, 480)
SEED = '6552'
# The Sunrise graphics a screenshot-sequence needs (it resets the pack's history): no Shadow Cache, TAA or frame
# generation.
GRAPHICS = 'shadowCache:false\ntaa:false\nfxaa:false\nframeGeneration:off\n'
# No chat or toasts, no pause offscreen, and no chunk fade (a screenshot-sequence needs it off).
OPTIONS = 'chatVisibility:2\ntutorialStep:none\npauseOnLostFocus:false\nchunkSectionFadeInTime:0.0\n'
BUILD = ['cargo', 'build', '--locked', '--profile', 'fast', '-p', 'sunrise-game', '-p', 'sunrise-client-lib', '--bins',
         '--features', 'sunrise-game/plan-trace']
MEASURE_BUILD = ['cargo', 'build', '--locked', '--profile', 'fast', '-p', 'sunrise-frame-plan', '--bins']


# Each case the run must reach: the guard a recorded frame names, and its value there.
CASES = {
    'shadow-cache-hit': ('shadow cache hit', True),
    'mod-draws-present': ('mod draws', True),
    'mod-draws-absent': ('mod draws', False),
    'callback-drew': ('a world callback drew', True),
    'callback-drew-nothing': ('a world callback drew', False),
    'deferred-skipped-then-ran': ('first deferred size-skipped, a later one ran', True),
}


def packs(folder, fixtures):
    """The packs to measure: every zip or folder in `folder`, then the fixtures, by name."""
    found = []
    if folder:
        found += sorted(p for p in Path(folder).iterdir() if p.suffix == '.zip' or p.is_dir())
    if fixtures:
        found += sorted(p for p in FIXTURES.iterdir() if p.is_dir())
    return found


class Game:
    """One game with one pack, started offscreen in its own folders and driven through sunrise-ctl."""

    def __init__(self, bins, folder, env, resources, prefix):
        self.ctl_path = bins / f'sunrise-ctl{EXE}'
        self.client = bins / f'sunrise-client{EXE}'
        self.folder = folder
        self.env = env
        self.resources = resources
        self.prefix = prefix
        # Sockets need short paths.
        self.socket = Path(tempfile.mkdtemp(prefix='hzpt')) / 'control.sock'
        self.process = None

    def ctl(self, *words, timeout=300, check=True):
        result = subprocess.run([str(self.ctl_path), '--socket', str(self.socket), '--timeout', str(timeout),
                                 *map(str, words)], cwd=self.folder, env=self.env, capture_output=True, text=True,
                                encoding='utf-8', errors='replace', timeout=timeout + 30)
        if check and result.returncode:
            raise RuntimeError(f'sunrise-ctl {" ".join(map(str, words))}: {result.stdout}{result.stderr}'[:2000])
        text = result.stdout.strip()
        return json.loads(text) if text.startswith(('{', '[')) else text

    def launch(self):
        command = [*self.prefix, str(self.client), '--resources', str(self.resources), '--game-dir', str(self.folder),
                   '--home', str(self.folder / 'home'), '--world', 'plan', '--seed', SEED, '--creative', '--offscreen',
                   '--width', str(SIZE[0]), '--height', str(SIZE[1]), '--control-socket', str(self.socket)]
        log = open(self.folder / 'game-stdout.log', 'w')
        self.process = subprocess.Popen(command, cwd=self.folder, env=self.env, stdout=log, stderr=subprocess.STDOUT)
        for _ in range(240):
            if self.process.poll() is not None:
                raise RuntimeError(f'the game ended at start: {self.process.returncode}')
            try:
                self.ctl('hello', timeout=5)
                break
            except (RuntimeError, subprocess.SubprocessError):
                time.sleep(1)
        else:
            raise RuntimeError('the game never answered')
        self.ctl('wait-ready', 300, timeout=330)

    def quit(self):
        if self.process is None:
            return
        try:
            self.ctl('quit', timeout=60, check=False)
            self.process.wait(timeout=60)
        except (OSError, subprocess.SubprocessError):
            self.process.kill()
            self.process.wait()
        shutil.rmtree(self.socket.parent, ignore_errors=True)


def pack_loaded(folder, name):
    """The `shader pack NAME: N programs` line of the game's log, or None."""
    for log in (folder / 'home/logs/latest.log', folder / 'game-stdout.log'):
        if log.exists():
            for line in log.read_text(errors='replace').splitlines():
                if f'shader pack {name}: ' in line and ' programs' in line:
                    return line
    return None


def widget(game, tab, prefix):
    """The id of the widget on options tab `tab` whose label starts with `prefix`, scrolling its list down to it."""
    game.ctl('click', tab)
    for _ in range(60):
        time.sleep(0.5)
        ui = game.ctl('ui')
        widgets = ui.get('widgets', [])
        # Near the list's foot the screen's footer takes the click: only a widget in its top two thirds is used.
        height = ui.get('gui_size', [0, 240])[1]
        for w in widgets:
            if str(w.get('label', '')).startswith(prefix) and sum(w['bounds'][1::2]) <= height * 2 / 3:
                return w['id']
        # The list is right of the tabs: turn the wheel over its middle.
        listed = [w['bounds'] for w in widgets if w.get('kind') != 'tab' and w.get('bounds')]
        x, y = (listed[0][0] + listed[0][2] / 2, listed[0][1] + 40) if listed else (270, 120)
        game.ctl('scroll-at', x, y, -1)
    raise RuntimeError(f'no widget labelled {prefix!r} on the {tab} tab')


def drive(game, name, wait):
    """Records the pack's frames: its start, a resize there and back, the Shadow Cache on and off, a reset."""
    for _ in range(wait):
        if pack_loaded(game.folder, name):
            break
        time.sleep(1)
    else:
        raise RuntimeError(f'shader pack {name} did not load')
    # Nothing moves: no other entity, the player a spectator (not drawn), the world frozen; so a shadow map can be
    # kept while the Shadow Cache is on.
    for command in ('gamerule advance_time false', 'gamerule advance_weather false', 'gamerule spawn_mobs false',
                    'time set noon', 'weather clear 100000', 'gamemode spectator @a',
                    'kill @e[type=!minecraft:player]', 'tick freeze'):
        game.ctl('command', command)
    game.ctl('wait-ready', 300, timeout=330)
    # 50%, then back to 100% (the slider's 0..1 is 50% to 200%), each applied as the screen closes on it.
    for value in (0.0, 1 / 3):
        game.ctl('options', 'video')
        game.ctl('set', widget(game, 'quality', 'Resolution Scale'), value)
        time.sleep(2)
        game.ctl('close', check=False)
        time.sleep(2)
    # The Shadow Cache on, then off again.
    for _ in range(2):
        game.ctl('options', 'video')
        game.ctl('click', widget(game, 'shaders', 'Shadow Cache'))
        time.sleep(2)
        game.ctl('close', check=False)
        time.sleep(2)
    game.ctl('screenshot-sequence', 4, 50, game.folder / 'sequence.png', timeout=600)
    time.sleep(2)


def measure(tool, traces):
    result = subprocess.run([str(tool), str(traces)], capture_output=True, text=True)
    try:
        return json.loads(result.stdout)[0], result.returncode
    except (json.JSONDecodeError, IndexError):
        return {'error': f'frame-plan-measure failed: {result.stderr.strip()[:500]}'}, result.returncode or 1


def sizes(report, key):
    """The per-size reports of one variant, or None when it could not be measured."""
    variant = report.get(key, {})
    return None if 'error' in variant else variant.get('sizes', [])


def failures(report):
    """What failed in one run's report, in words."""
    if 'error' in report:
        return [report['error']]
    out = [f'refused {r}' for r in report.get('refused', [])]
    for key in ('unresolved', 'narrowed'):
        measured = sizes(report, key)
        if measured is None:
            out.append(f'{key}: {report[key]["error"]}')
            continue
        for sized in measured:
            at = 'x'.join(map(str, sized['size']))
            for frame in sized['frames']:
                if frame['blocks'] != 'ok':
                    out.append(f'{key}: frame {frame["seq"]}: blocks {frame["blocks"]}')
                for error in frame['errors']:
                    out.append(f'{key}: frame {frame["seq"]}: {error}')
            for index, state in enumerate(sized['states']):
                if state['edits'] != 'ok':
                    out.append(f'{key}: {at} state {index}: edits {state["edits"]}')
    return out


def reached(reports):
    """The cases of CASES some frame of `reports` reached."""
    seen = set()
    for report in reports:
        for sized in sizes(report, 'unresolved') or []:
            for frame in sized['frames']:
                seen.update((name, bool(value)) for name, value in frame.get('guards', []))
    return {case for case, guard in CASES.items() if guard in seen}


def row(name, report):
    """One table row per variant and render size: the frames and states, the work, what each rule would remove,
    and the bytes one frame holds."""
    rows = []
    for key in ('unresolved', 'narrowed'):
        measured = sizes(report, key)
        if not measured:
            rows.append(f'| {name} | {key} | – | – | – | – | – | – | – | – | – | – |')
            continue
        for sized in measured:
            states = sized['states']
            s = sized['savings']
            checks = 'ok' if not failures({'refused': [], key: {'sizes': [sized]}}) else 'FAILED'
            rows.append(f'| {name} | {key} | {sized["size"][0]}×{sized["size"][1]} | '
                        f'{len(sized["frames"])}/{len(states)} | {checks} | '
                        f'{sum(x["passes"] for x in states)} | {sum(x["clears"] for x in states)} | '
                        f'{s["events"]} | {s["clears_folded"]} | {s["clears_dropped"]} | '
                        f'{sized["dead_bytes"]:,} of {sized["plannable"]:,} | {sized["allocated"]:,} |')
    return rows


HEADER = ['| pack | callbacks | size | frames/states | checks | passes | clears | R1 draws | R4b folds | '
          'R2 clears | R2 bytes a frame (of plannable) | allocated a frame |',
          '| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |']


def resolved(args):
    """`args` with every path made absolute from the caller's folder: the game runs in a folder of its own."""
    for name in ('packs', 'bin_dir', 'out', 'resources'):
        value = getattr(args, name)
        if value is not None:
            setattr(args, name, Path(value).resolve())
    return args


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__.split('\n\n')[0])
    parser.add_argument('--packs', help='a folder of shader packs (zips or folders)')
    parser.add_argument('--fixtures', action='store_true', help='also the packs in tools/fixtures/shaderpacks')
    parser.add_argument('--bin-dir', type=Path, help='sunrise-client, sunrise-ctl and frame-plan-measure built with '
                        'the feature (else built into the cargo target)')
    parser.add_argument('--out', type=Path, default=lab.LOCAL / 'pack-plan-trace', help='games, traces and reports')
    parser.add_argument('--resources', type=Path, default=ROOT / '.local/sunrise-client/26.3')
    parser.add_argument('--prefix', default='', help='a command the game runs under (a headless display, a cap)')
    parser.add_argument('--frames', type=int, default=4, help='frames written after each trigger')
    parser.add_argument('--wait', type=int, default=240, help='seconds to wait for a pack to load')
    parser.add_argument('--allow-missing', action='append', default=[], choices=sorted(CASES),
                        help='a case the run may not reach (repeatable)')
    args = resolved(parser.parse_args(argv))
    chosen = packs(args.packs, args.fixtures)
    if not chosen:
        parser.error('no pack: give --packs DIR or --fixtures')
    bins = args.bin_dir
    if bins is None:
        for command in (BUILD, MEASURE_BUILD):
            subprocess.run(command, cwd=ROOT, check=True)
        bins = lab.cargo_target() / 'fast'
    tool = bins / f'frame-plan-measure{EXE}'
    args.out.mkdir(parents=True, exist_ok=True)
    table, failed, reports = list(HEADER), [], []
    for pack in chosen:
        name = pack.name
        folder = args.out / 'games' / name
        traces = args.out / 'traces' / name
        for path in (folder, traces):
            shutil.rmtree(path, ignore_errors=True)
            path.mkdir(parents=True)
        (folder / 'shaderpacks').mkdir()
        (shutil.copy2 if pack.is_file() else shutil.copytree)(pack, folder / 'shaderpacks' / name)
        (folder / 'options.txt').write_text(OPTIONS)
        (folder / 'sunrise-graphics.txt').write_text(GRAPHICS)
        homes = {k: str(folder / k.lower()) for k in ('XDG_DATA_HOME', 'XDG_CONFIG_HOME', 'SUNRISE_LAUNCHER_HOME')}
        env = lab.game_env(None, SUNRISE_HOME=str(folder / 'home'), SUNRISE_SHADER_PACK=name, SUNRISE_STILL='1',
                           SUNRISE_PLAN_TRACE=str(traces), SUNRISE_PLAN_TRACE_FRAMES=str(args.frames), **homes)
        game = Game(bins, folder, env, args.resources, shlex.split(args.prefix))
        try:
            game.launch()
            drive(game, name, args.wait)
        except (RuntimeError, OSError, subprocess.SubprocessError) as error:
            failed.append(f'{name}: {error}')
        finally:
            game.quit()
        report, _ = measure(tool, traces)
        (args.out / f'{name}.json').write_text(json.dumps(report, indent=1))
        failed += [f'{name}: {f}' for f in failures(report)]
        reports.append(report)
        table += row(name, report)
    print('\n'.join(table))
    seen = reached(reports)
    for case in sorted(CASES):
        state = 'reached' if case in seen else 'MISSING (allowed)' if case in args.allow_missing else 'MISSING'
        print(f'case {case}: {state}')
        if state == 'MISSING':
            failed.append(f'case {case}: no recorded frame reached it')
    for line in failed:
        print(f'FAILED {line}')
    return 1 if failed else 0


if __name__ == '__main__':
    sys.exit(main())
