#!/usr/bin/env python3
"""Shader-pack scene suite: the same fixed views photographed in Sunrise and in Java 26.3, then compared.

The scene file (tools/shader-scenes/scenes.json, docs in tools/shader-scenes/README.md) names a seed, pinned
options and gamerules, and views: a dimension, a camera, a time, a weather and vanilla setup commands. Every run
writes under one output folder:

    world/scenes/                    the shared world, generated once by vanilla Java (`world`)
    sunrise/<pack>/<view>.png        Sunrise's pictures (`sunrise`), with run.json and the game's log
    java/<pack>/<view>.png           Java's pictures (`java`): vanilla for the pack `none`, else Fabric + Sodium + Iris
    report/                          the comparison (`compare`): index.html, report.md, report.json, pairs/, heat/

    python3 tools/shader_scenes.py check
    python3 tools/shader_scenes.py world --out OUT
    python3 tools/shader_scenes.py sunrise --out OUT --pack none --pack PACKS/BSL_v10.1.8.zip
    python3 tools/shader_scenes.py java --out OUT --pack none --pack PACKS/BSL_v10.1.8.zip --mods-from MODS
    python3 tools/shader_scenes.py compare --out OUT
    python3 tools/shader_scenes.py run --out OUT --pack PACKS/BSL_v10.1.8.zip --mods-from MODS   # all of it

Both games load copies of the one world `world` saved, so trees and caves are the same in both pictures. Every
picture's camera is read back from the game before it is taken and checked against the view; a mismatch, a failed
setup command or a view that never finished loading is written into run.json and shown in the report.

Packs are only loaded (copied into each game's shaderpacks/), never read. Java runs Iris as a black box: released
jars pinned in tools/shader-scenes/mods.lock.json, taken from --mods-from or the Modrinth CDN, checked by sha512.
Nothing here is committed: no pictures, worlds, jars or packs. The Java side is tools/shader_scenes_java.py, the
comparison tools/shader_scenes_report.py.
"""
import argparse
import hashlib
import concurrent.futures
import itertools
import json
import math
import os
import re
import shutil
import subprocess
import sys
import tempfile
import time
from decimal import Decimal
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))
from tools.shader_scenes_report import RUN_MARK, compare, scene_hash, summary  # noqa: E402

SCENES = ROOT / "tools/shader-scenes/scenes.json"
FREEZE = "tick freeze"
# Vanilla's commands.tick.status.* lines that `tick query` answers with (assets/minecraft/lang/en_us.json).
TICK_FROZEN = "The game is frozen"
TICK_NOT_FROZEN = ("The game is running normally", "The game is running, but can't keep up with the target tick rate",
                   "The game is sprinting")
WORLD = "scenes"
# A view's or a pack's folder name: what a scene file may name, never a path.
NAME = re.compile(r"[a-z0-9_]+\Z")
DIMENSIONS = ("overworld", "the_nether", "the_end")
WEATHERS = ("clear", "rain", "thunder")
VIEW_KEYS = {"name", "dimension", "pos", "yaw", "pitch", "time", "weather", "setup", "settle_seconds"}
FILE_KEYS = {"format", "seed", "size", "settle_seconds", "weather_settle_seconds", "move_settle_seconds",
             "phase_shots", "load_timeout_seconds", "options", "gamerules", "views"}
# Camera read back against the view: tp writes doubles and floats exactly, so these only absorb float32 rounding
# of the angles and the client's position interpolation (a teleport lands exactly).
POSITION_EPSILON = 1e-3
ANGLE_EPSILON = 1e-2
# The largest picture side: wgpu's default max_texture_dimension_2d, the size every adapter gives Sunrise's
# offscreen target. It also bounds what read_png decompresses, since it reads only pictures of the scene's size.
MAX_SIZE = 8192
# phase_shots covers animation cycles up to its square in ticks (phase_step): 16 covers 256 ticks, past every
# animated block texture of vanilla 26.3 but the slow ones (prismarine's 6600-tick cycle); more only multiplies
# the pictures.
MAX_PHASE_SHOTS = 16


class SceneError(ValueError):
    """The scene file is not one this tool can run; the message names the field."""


# Scene file ----------------------------------------------------------------------------------------------------

def _number(value, where, low=None, high=None):
    if isinstance(value, bool) or not isinstance(value, (int, float)) or not math.isfinite(value):
        raise SceneError(f"{where}: expected a finite number, got {value!r}")
    if (low is not None and value < low) or (high is not None and value > high):
        raise SceneError(f"{where}: {value} is outside {low}..{high}")
    return float(value)


def _phase_shots(value):
    if isinstance(value, bool) or not isinstance(value, int) or not 1 <= value <= MAX_PHASE_SHOTS:
        raise SceneError(f"phase_shots: expected 1..{MAX_PHASE_SHOTS}, got {value!r}")
    return value


def phase_step(scenes, side):
    """Ticks stepped between a side's phase shots: 1 in Sunrise, phase_shots in Java. The frozen world holds each
    game's texture animations on whatever frame they reached, unrelated between the games; with K shots stepped 1
    tick apart on one side and K ticks apart on the other, the K x K pairs cover every offset from 0 to K*K - 1 ticks,
    so a pair with the two sides in the same phase is among them for animations up to K*K ticks long."""
    return 1 if side == "sunrise" else scenes["phase_shots"]


def _strings(value, where):
    if not isinstance(value, dict) or not all(isinstance(k, str) and isinstance(v, str) for k, v in value.items()):
        raise SceneError(f"{where}: expected an object of strings")
    for key, item in value.items():
        if not re.fullmatch(r"[A-Za-z0-9_.]+", key) or "\n" in item or ":" in key:
            raise SceneError(f"{where}.{key}: not a plain key and one-line value")
    return dict(value)


def _unknown(data, allowed, where):
    extra = sorted(set(data) - allowed)
    if extra:
        raise SceneError(f"{where}: unknown key {extra[0]!r}")


def parse_scenes(data):
    """Checks a scene file's JSON and returns it with every default filled in; raises SceneError naming the field."""
    if not isinstance(data, dict):
        raise SceneError("scene file: expected an object")
    _unknown(data, FILE_KEYS, "scene file")
    for key in ("format", "seed", "size", "views"):
        if key not in data:
            raise SceneError(f"scene file: missing {key!r}")
    if data["format"] != 1:
        raise SceneError(f"format: {data['format']!r} is not 1")
    seed = data["seed"]
    if isinstance(seed, bool) or not isinstance(seed, int) or not -2**63 <= seed < 2**63:
        raise SceneError(f"seed: expected a 64-bit integer, got {seed!r}")
    size = data["size"]
    if (not isinstance(size, list) or len(size) != 2
            or not all(isinstance(v, int) and not isinstance(v, bool) and 1 <= v <= MAX_SIZE for v in size)):
        raise SceneError(f"size: expected [width, height] in 1..{MAX_SIZE}, got {size!r}")
    out = {
        "seed": seed,
        "size": list(size),
        "settle_seconds": _number(data.get("settle_seconds", 5), "settle_seconds", 0, 600),
        "weather_settle_seconds": _number(data.get("weather_settle_seconds", data.get("settle_seconds", 5)),
                                          "weather_settle_seconds", 0, 600),
        "move_settle_seconds": _number(data.get("move_settle_seconds", data.get("settle_seconds", 5)),
                                       "move_settle_seconds", 0, 600),
        "phase_shots": _phase_shots(data.get("phase_shots", 1)),
        "load_timeout_seconds": _number(data.get("load_timeout_seconds", 90), "load_timeout_seconds", 1, 3600),
        "options": _strings(data.get("options", {}), "options"),
        "gamerules": _strings(data.get("gamerules", {}), "gamerules"),
        "views": [],
    }
    views = data["views"]
    if not isinstance(views, list) or not views:
        raise SceneError("views: expected a non-empty list")
    _end_order(views)
    seen = set()
    for i, view in enumerate(views):
        where = f"views[{i}]"
        if not isinstance(view, dict):
            raise SceneError(f"{where}: expected an object")
        _unknown(view, VIEW_KEYS, where)
        name = view.get("name")
        if not isinstance(name, str) or not NAME.match(name):
            raise SceneError(f"{where}.name: expected [a-z0-9_]+, got {name!r}")
        where = f"view {name}"
        if name in seen:
            raise SceneError(f"{where}: the name is used twice")
        seen.add(name)
        dimension = view.get("dimension")
        if dimension not in DIMENSIONS:
            raise SceneError(f"{where}.dimension: expected one of {', '.join(DIMENSIONS)}, got {dimension!r}")
        pos = view.get("pos")
        if not isinstance(pos, list) or len(pos) != 3:
            raise SceneError(f"{where}.pos: expected [x, y, z]")
        pos = [_number(v, f"{where}.pos", -3.0e7, 3.0e7) for v in pos]
        weather = view.get("weather")
        if weather is not None and weather not in WEATHERS:
            raise SceneError(f"{where}.weather: expected one of {', '.join(WEATHERS)}, got {weather!r}")
        day_time = view.get("time")
        if day_time is not None and (isinstance(day_time, bool) or not isinstance(day_time, int) or day_time < 0):
            raise SceneError(f"{where}.time: expected a tick count of 0 or more, got {day_time!r}")
        setup = view.get("setup", [])
        if not isinstance(setup, list) or not all(isinstance(c, str) and c.strip() and "\n" not in c for c in setup):
            raise SceneError(f"{where}.setup: expected a list of one-line commands")
        out["views"].append({
            "name": name, "dimension": dimension, "pos": pos,
            "yaw": _number(view.get("yaw", 0.0), f"{where}.yaw", -360, 360),
            "pitch": _number(view.get("pitch", 0.0), f"{where}.pitch", -90, 90),
            "time": day_time, "weather": weather,
            "setup": [c.strip().removeprefix("/") for c in setup],
            "settle_seconds": (None if view.get("settle_seconds") is None
                               else _number(view["settle_seconds"], f"{where}.settle_seconds", 0, 600)),
        })
    return out


def settle_plan(scenes):
    """(thaw, settle) for each view. The world is frozen (`tick freeze`), and rain fades in and out only while the
    level ticks: a view whose weather differs from what the previous view left (the saved world starts clear) thaws
    it for weather_settle_seconds, then freezes it again. Then it settles before its picture: its own
    settle_seconds, else move_settle_seconds when the camera moved from the previous view (the first view counts as
    moved), else settle_seconds, whichever is longer."""
    plan, weather, previous = [], "clear", None
    for view in scenes["views"]:
        changed = view["weather"] is not None and view["weather"] != weather
        weather = view["weather"] or weather
        thaw = scenes["weather_settle_seconds"] if changed else 0.0
        if view["settle_seconds"] is not None:
            settle = view["settle_seconds"]
        else:
            settle = max(scenes["settle_seconds"],
                         scenes["move_settle_seconds"] if not same_place(view, previous) else 0.0)
        plan.append((thaw, settle))
        previous = view
    return plan


def same_place(a, b):
    """Two views share the camera's place: same dimension and position (time, weather or setup may differ)."""
    return b is not None and a["dimension"] == b["dimension"] and a["pos"] == b["pos"]


def _end_order(views):
    """The world is frozen: leaving the End then keeps the dragon fight's boss bar, and its world fog, in other
    dimensions (vanilla drops it only at the fight's player scan, which needs the End to tick; #6545). So the first
    view (where the world is saved and opened) is not in the End, and once a view is in the End every later one is."""
    dims = [v.get("dimension") if isinstance(v, dict) else None for v in views]
    if dims[0] == "the_end":
        raise SceneError("views[0].dimension: the first view can't be in the End (the world is saved and opened "
                         "there; the dragon's boss bar would follow into other dimensions)")
    if "the_end" in dims:
        first = dims.index("the_end")
        for i in range(first, len(dims)):
            if dims[i] != "the_end":
                raise SceneError(f"views[{i}].dimension: no view can follow an End view in another dimension while "
                                 "the world is frozen (the dragon's boss bar would follow); put the End views last")


def load_scenes(path=SCENES):
    try:
        data = json.loads(Path(path).read_text(encoding="utf-8"))
    except json.JSONDecodeError as error:
        raise SceneError(f"{path}: not JSON ({error})") from None
    return parse_scenes(data)


def dimension_id(dimension):
    return f"minecraft:{dimension}"


def _coordinate(value):
    """A float as tp reads it: plain decimal digits (the command reader takes no exponent, so 1e-05 is written
    0.00001) and always a decimal point, since tp centres a bare integer x or z on its block."""
    text = format(Decimal(repr(float(value))), "f")
    return text if "." in text else text + ".0"



def start_commands(scenes):
    """Commands run once after the world opens, the same lines in both games (console source, all players): set each
    gamerule, then query it (start_problems checks the queried values), then spectator mode (camera_problems
    checks it), then `tick freeze`: entities, fluids and particles stop for every view (camera_problems checks the
    frozen state; the player and commands still run). Setting a rule to the value it already has fails in vanilla,
    so the sets' outcomes are not judged."""
    sets = [f"gamerule {k} {v}" for k, v in scenes["gamerules"].items()]
    return sets + [f"gamerule {k}" for k in scenes["gamerules"]] + ["gamemode spectator @a", FREEZE]


def _rule_value(text):
    """A gamerule's value as its query command returns it: 1 or 0 for a boolean, the number for an integer."""
    if text in ("true", "false"):
        return int(text == "true")
    return int(text) if re.fullmatch(r"-?\d+", text) else None


def start_problems(scenes, results):
    """What differs between the scene's gamerules and the values the game's queries returned (`results` maps each
    query command to its result, or None when it failed)."""
    problems = []
    for key, value in scenes["gamerules"].items():
        got = results.get(f"gamerule {key}")
        want = _rule_value(value)
        if got is None or (want is not None and got != want):
            problems.append(f"gamerule {key} is {got}, the scene asks for {value}")
    if FREEZE in results and results[FREEZE] is None:
        problems.append(f"{FREEZE} failed")
    return problems


def view_commands(view):
    """Commands that place the camera of a view: the teleport into its dimension, then time and weather.
    Their outcome is not what is checked (vanilla's `time set` fails when the clock already shows that time):
    the camera, time and weather read back before the picture are (camera_problems)."""
    x, y, z = map(_coordinate, view["pos"])
    lines = [f"execute in {dimension_id(view['dimension'])} run tp @a {x} {y} {z} "
             f"{_coordinate(view['yaw'])} {_coordinate(view['pitch'])}"]
    if view["time"] is not None:
        lines.append(f"time set {view['time']}")
    if view["weather"] is not None:
        lines.append(f"weather {view['weather']}")
    return lines


def setup_commands(view):
    """A view's setup commands, run in its dimension once its chunks are there."""
    return [f"execute in {dimension_id(view['dimension'])} run {c}" for c in view["setup"]]


def options_text(scenes, data_version, extra=None):
    """options.txt for either game: the data version, the scene's pinned options, then the game's own extras."""
    options = {"version": str(data_version), **scenes["options"], **(extra or {})}
    return "".join(f"{k}:{v}\n" for k, v in options.items())


def _angle_gap(a, b):
    return abs((a - b + 180.0) % 360.0 - 180.0)


def settings_expected(scenes):
    """The drawing settings the scene pins, as the games report them: render and simulation distance in chunks, and
    the field of view in degrees (options.txt stores it as (degrees - 70) / 40)."""
    options, out = scenes["options"], {}
    for key, name in (("renderDistance", "render_distance"), ("simulationDistance", "simulation_distance")):
        if key in options:
            out[name] = int(options[key])
    if "fov" in options:
        out["fov"] = round(70 + 40 * float(options["fov"]))
    return out


def settings_problems(expected, settings):
    """What differs between the pinned drawing settings and those a game read back (a first-start graphics preset,
    for one, replaces options.txt's distances unless graphicsPreset is "custom")."""
    if not expected:
        return []
    if not isinstance(settings, dict):
        return ["no drawing settings were read back"]
    return [f"{key} {settings.get(key)}, the scene pins {value}" for key, value in expected.items()
            if settings.get(key) != value]


def camera_problems(view, camera):
    """What differs between a view and the camera a game read back before its picture (empty: none)."""
    if not camera:
        return ["no camera was read back"]
    problems = []
    if camera.get("dimension") != dimension_id(view["dimension"]):
        problems.append(f"dimension {camera.get('dimension')}, view asks for {dimension_id(view['dimension'])}")
    pos = camera.get("pos")
    if not isinstance(pos, list) or len(pos) != 3:
        problems.append("no position")
    elif max(abs(a - b) for a, b in zip(pos, view["pos"])) > POSITION_EPSILON:
        problems.append(f"position {pos}, view asks for {view['pos']}")
    for key in ("yaw", "pitch"):
        value = camera.get(key)
        if not isinstance(value, (int, float)) or _angle_gap(value, view[key]) > ANGLE_EPSILON:
            problems.append(f"{key} {value}, view asks for {view[key]}")
    if camera.get("screen") != "playing":
        problems.append(f"screen {camera.get('screen')} is open")
    if camera.get("spectator") is not True:
        problems.append("not in spectator mode")
    bars = camera.get("boss_bars")
    if bars is not None and view["dimension"] != "the_end" and bars != 0:
        # A boss bar with world fog (the dragon's) outside the End is a stale fight, not the view (#6545).
        problems.append(f"{bars} boss bar(s) shown outside the End")
    if camera.get("frozen") is not True:
        problems.append(f"the game is not frozen ({FREEZE}): frozen read back as {camera.get('frozen')}")
    day = camera.get("day_time")
    if view["time"] is not None and (not isinstance(day, int) or day % 24000 != view["time"] % 24000):
        problems.append(f"time {day}, view asks for {view['time']}")
    if view["weather"] is not None:
        weather = ("thunder" if camera.get("thundering") else "rain") if camera.get("raining") else "clear"
        if weather != view["weather"]:
            problems.append(f"weather {weather}, view asks for {view['weather']}")
    return problems


def pack_key(pack):
    """The folder a pack's pictures go in: `none`, or the pack file's name made lower-case [a-z0-9_]."""
    if pack == "none":
        return "none"
    stem = Path(pack).name
    stem = stem[:-4] if stem.lower().endswith(".zip") else stem
    key = re.sub(r"[^a-z0-9]+", "_", stem.lower()).strip("_")
    if not key or key == "none":
        raise SceneError(f"pack {pack}: its name gives no folder name")
    return key


def pack_digest(pack):
    """The identity of the pack input a run staged: `none`, or the sha256 of the zip, or of a folder's relative file
    names and their sha256 in sorted order. Both sides record it; compare pairs only runs of the same input."""
    if pack == "none":
        return "none"
    source = Path(pack)
    if source.is_file():
        return hashlib.sha256(source.read_bytes()).hexdigest()
    digest = hashlib.sha256()
    for path in sorted(p for p in source.rglob("*") if p.is_file()):
        digest.update(path.relative_to(source).as_posix().encode() + b"\0")
        digest.update(hashlib.sha256(path.read_bytes()).digest())
    return digest.hexdigest()


# Runs ------------------------------------------------------------------------------------------------------------

def run_folder(out, side, pack, replace):
    """A fresh folder for one side's run of a pack. An existing one is only removed with --replace, and only when it
    carries this tool's mark (a folder some earlier run of this tool made)."""
    folder = Path(out) / side / pack_key(pack)
    if folder.exists():
        if not replace:
            raise SystemExit(f"{folder} exists: pass --replace to run it again")
        if not (folder / RUN_MARK).exists():
            raise SystemExit(f"{folder} was not made by this tool; not removing it")
        shutil.rmtree(folder)
    folder.mkdir(parents=True)
    (folder / RUN_MARK).write_text("")
    return folder


def world_generation(out):
    """The shared world's record, written by `world` when it publishes the save: its generation id and the scene
    hash it was made for. None when there is no published world."""
    path = Path(out) / "world" / "generation.json"
    return json.loads(path.read_text()) if path.exists() else None


def copy_world(out, game_dir, scenes):
    """Copies the published world into a game's saves/ and returns the identity a run records with its pictures
    ({"world_id", "scene_hash"}), so the report pairs only pictures of the same world and scene. A world made for
    another scene definition is refused: its chunks and saved camera are for other views."""
    source = Path(out) / "world" / WORLD
    generation = world_generation(out)
    if generation is None or not (source / "level.dat").exists():
        raise SystemExit(f"{Path(out) / 'world'} has no published world: run `world` first")
    if generation["scene_hash"] != scene_hash(scenes):
        raise SystemExit(f"{Path(out) / 'world'} was generated for another scene definition: run `world --replace`")
    shutil.copytree(source, game_dir / "saves" / WORLD, ignore=shutil.ignore_patterns("session.lock"))
    return {"world_id": generation["world_id"], "scene_hash": generation["scene_hash"]}


def install_pack(pack, game_dir):
    """Copies the pack into the game's shaderpacks/ (loaded only, never read here); returns its file name."""
    source = Path(pack)
    if not source.exists():
        raise SystemExit(f"pack {pack} not found")
    target = game_dir / "shaderpacks" / source.name
    target.parent.mkdir(parents=True, exist_ok=True)
    (shutil.copytree if source.is_dir() else shutil.copy)(source, target)
    return source.name


def record_view(views, view, camera, failed, ready, extra=None, expected=None):
    views[view["name"]] = {
        "camera": camera, "failed_commands": failed, "ready": ready,
        "problems": camera_problems(view, camera)
                    + settings_problems(expected or {}, (camera or {}).get("settings"))
                    + [f"command failed: {c}" for c in failed]
                    + ([] if ready else ["the view never finished loading"]) + (extra or []),
    }


def write_run(folder, record):
    (Path(folder) / "run.json").write_text(json.dumps(record, indent=2) + "\n")


# Sunrise

class Sunrise:
    """One Sunrise game driven through sunrise-ctl, in scratch folders of its own."""

    def __init__(self, ctl, socket, env):
        self.ctl, self.socket, self.env = ctl, socket, env

    def __call__(self, *words, check=True, timeout=600):
        done = subprocess.run([str(self.ctl), "--socket", str(self.socket), *map(str, words)], env=self.env,
                              capture_output=True, text=True, timeout=timeout)
        lines = [l for l in done.stdout.splitlines() if l.startswith("{")]
        reply = json.loads(lines[-1]) if lines else {"error": done.stderr.strip() or "no reply"}
        if check and "error" in reply:
            raise RuntimeError(f"sunrise-ctl {' '.join(map(str, words))}: {reply['error']}")
        return reply

    def command(self, line):
        reply = self("command", line, check=False)
        return "error" not in reply and reply.get("success", False)


def sunrise_camera(state):
    player = state.get("player") or {}
    weather = state.get("weather") or {}
    return {"dimension": state.get("dimension"), "pos": player.get("position"), "yaw": player.get("yaw"),
            "pitch": player.get("pitch"), "screen": state.get("screen"), "gamemode": state.get("gamemode"),
            "spectator": state.get("gamemode") == "spectator",
            "day_time": (state.get("time") or {}).get("day_time"), "raining": weather.get("raining"),
            "thundering": weather.get("thundering")}


def sunrise_frozen(reply):
    """Whether Sunrise reports the game frozen, from its `tick query` reply: True or False from vanilla's status
    line, None when the command failed or no status line came back."""
    if "error" in reply or not reply.get("success"):
        return None
    lines = [str(line) for line in reply.get("chat", [])]
    for line in reversed(lines):
        if TICK_FROZEN in line:
            return True
        if any(text in line for text in TICK_NOT_FROZEN):
            return False
    return None


def sunrise_settings(status, options):
    """The drawing settings Sunrise holds: the render distance it draws (status) and the simulation distance and
    field of view of its options (`options-get`)."""
    values = (options.get("options") or {})
    return {"render_distance": status.get("render_distance"),
            "simulation_distance": (values.get("simulationDistance") or {}).get("value"),
            "fov": (values.get("fov") or {}).get("value")}


def pack_events(log_text, name):
    """The pack loads Sunrise logged, in order: ("programs", N) for `shader pack <name>: N programs ...`, or
    ("off", why) for `shader pack <name>: turned off: why`. It loads a pack again at each dimension change."""
    prefix = f"shader pack {name}: "
    out = []
    for line in log_text.splitlines():
        at = line.find(prefix)
        if at < 0:
            continue
        rest = line[at + len(prefix):]
        loaded = re.match(r"(\d+) programs\b", rest)
        if loaded:
            out.append(("programs", int(loaded.group(1))))
        elif rest.startswith("turned off: "):
            out.append(("off", rest[len("turned off: "):]))
    return out


class PackLoads:
    """Ties each pack load Sunrise logs to the dimension change that caused it. Sunrise loads the pack once when the
    world opens and again at each change of dimension (spec 1.2), and logs one line for each load that finishes or
    fails, in order; a load cancelled by a later change logs nothing. So load n belongs to transition n only while
    every earlier load arrived in time and no other load came between: after a load that never came (it may still
    come late, or never), or an extra one, the later views' pack state is unknown, never guessed."""

    def __init__(self, read_events, now=time.time, sleep=time.sleep):
        self.read, self.now, self.sleep = read_events, now, sleep
        self.expected, self.lost = 0, None

    def transition(self, timeout):
        """Call once the game shows the new dimension; returns ("programs", N), ("off", why), ("missing", why) or
        ("unknown", why) for the load this transition started."""
        if self.lost is None and len(self.read()) > self.expected + 1:
            self.lost = "Sunrise logged a pack load no dimension change asked for"
        self.expected += 1
        if self.lost:
            return ("unknown", f"the pack's state is unknown: {self.lost}")
        deadline = self.now() + timeout
        while True:
            events = self.read()
            if len(events) > self.expected:
                # More loads than transitions: which one is this transition's can't be told.
                self.lost = "Sunrise logged a pack load no dimension change asked for"
                return ("unknown", f"the pack's state is unknown: {self.lost}")
            if len(events) == self.expected:
                return events[self.expected - 1]
            if self.now() >= deadline:
                self.lost = "an earlier pack load never finished"
                return ("missing", "the pack never finished loading in this dimension")
            self.sleep(0.5)


def pack_problem(load):
    """The problem a view has with the pack load of its dimension, or None when the pack is running."""
    kind, detail = load
    if kind == "programs":
        return None
    return f"the pack was turned off: {detail}" if kind == "off" else detail


def sunrise_wait_loaded(game, view, limit, moved=True):
    """Waits until the camera is in the view's dimension, at its position, and the chunks around it are loaded and
    drawn: tools/perf_bench.py's wait_loaded, the readiness the benchmarks use (`wait-ready` alone can answer
    before loading starts: right after a teleport nothing is queued yet). With `moved` false (the camera stays where
    the last loaded view was, or a pack load needs no new chunks) the teleport guard is skipped (the resident count
    must reach the view circle and change once or stay full for 5 s) and only steady, drawn chunks are waited for.
    True when loaded within `limit`."""
    from tools import perf_bench
    want = dimension_id(view["dimension"])

    def status(line, timeout=10):
        reply = game(line, timeout=timeout)
        if reply.get("dimension") != want:
            reply = dict(reply, player=None)  # not there yet: wait_loaded starts over
        return reply

    def near(position):
        return max(abs(a - b) for a, b in zip(position, view["pos"])) <= POSITION_EPSILON

    try:
        distance = game("status", timeout=10).get("render_distance") or 0
        perf_bench.wait_loaded(status, near, limit, perf_bench.view_chunks(distance) if moved else 0)
        return True
    except (TimeoutError, RuntimeError, OSError, subprocess.TimeoutExpired):
        return False


def display_command(args, scenes, command):
    """The game's command, inside a headless gamescope display of the scene's size unless --display current: the
    window gets exactly that size whatever the desktop's compositor does, and nothing opens on the desktop."""
    if args.display == "current":
        return command
    if not shutil.which("gamescope"):
        raise SystemExit("--display headless needs gamescope (or pass --display current)")
    width, height = map(str, scenes["size"])
    return ["gamescope", "--backend", "headless", "-W", width, "-H", height, "-w", width, "-h", height,
            "--", *command]


def start_game(args, scenes, command, env, log_path, cwd, scratch):
    """Starts a game (inside its display), with XDG folders of its own under `scratch`: neither the game nor
    gamescope reads or writes the user's."""
    env = dict(env, **{f"XDG_{kind}_HOME": str(Path(scratch) / f"xdg-{kind.lower()}")
                       for kind in ("CONFIG", "DATA", "CACHE", "STATE")})
    if args.display == "headless":
        # The game must use gamescope's X display, not the desktop's Wayland one.
        env.pop("WAYLAND_DISPLAY", None)
    with open(log_path, "w") as log:
        return subprocess.Popen(display_command(args, scenes, command), cwd=cwd, env=env, stdout=log,
                                stderr=subprocess.STDOUT, stdin=subprocess.DEVNULL)


def stop_game(proc, timeout=60):
    try:
        proc.wait(timeout=timeout)
    except subprocess.TimeoutExpired:
        proc.terminate()
        try:
            proc.wait(timeout=30)
        except subprocess.TimeoutExpired:
            proc.kill()
            proc.wait()


def wait_for_socket(game, proc, timeout):
    deadline = time.time() + timeout
    while time.time() < deadline and proc.poll() is None:
        if "error" not in game("hello", check=False, timeout=30):
            return True
        time.sleep(0.5)
    return False


def run_sunrise(args, scenes, pack):
    from tools import lab
    folder = run_folder(args.out, "sunrise", pack, args.replace)
    game_dir = folder / "game"
    game_dir.mkdir()
    generation = copy_world(args.out, game_dir, scenes)
    (game_dir / "options.txt").write_text(options_text(scenes, lab.version_info()["world_version"]))
    env = lab.game_env(SUNRISE_HOME=folder / "home", XDG_DATA_HOME=folder / "xdg-data",
                       XDG_CONFIG_HOME=folder / "xdg-config", SUNRISE_LAUNCHER_HOME=folder / "launcher-home")
    # start_game gives the game and gamescope all four XDG folders under the run folder.
    env.pop("SUNRISE_SHADER_PACK", None)
    name = None
    if pack != "none":
        name = install_pack(pack, game_dir)
        env["SUNRISE_SHADER_PACK"] = name
    # A socket path must stay under ~100 bytes: a fresh folder of /tmp, removed after the run.
    sock_dir = Path(tempfile.mkdtemp(prefix="ssc-"))
    game = Sunrise(args.sunrise_ctl, sock_dir / "ctl.sock", env)
    record = {"side": "sunrise", "pack": Path(pack).name if name else "none", "pack_sha256": pack_digest(pack),
              "views": {}, "notes": [],
              **generation}
    width, height = scenes["size"]
    limit = scenes["load_timeout_seconds"]
    log = folder / "home" / "logs" / "latest.log"
    expected = settings_expected(scenes)
    loads = PackLoads(lambda: pack_events(log.read_text(errors="replace"), name) if log.exists() else [])
    proc = None
    try:
        command = [str(args.sunrise_client), "--resources", str(args.resources), "--game-dir", str(game_dir),
                   "--home", str(folder / "home"), "--world", WORLD, "--offscreen", "--width", str(width),
                   "--height", str(height), "--control-socket", str(game.socket)]
        proc = start_game(args, scenes, command, env, folder / "game-stdout.log", game_dir, folder)
        if not wait_for_socket(game, proc, limit):
            raise RuntimeError(f"Sunrise did not open its control socket; see {folder / 'game-stdout.log'}")
        game("wait-ready", int(limit), timeout=limit + 60)
        # The world opens where the player was saved (the first view): its load is the first transition.
        dimension = (game("state").get("dimension") or "").removeprefix("minecraft:")
        load = loads.transition(limit) if name else None
        record["loads"] = [[dimension, load]] if name else []
        results = {}
        for line in start_commands(scenes):
            reply = game("command", line, check=False)
            results[line] = reply.get("result") if reply.get("success") else None
        record["start_problems"] = start_problems(scenes, results)
        game("key", "f1", "tap")
        previous = None
        for view, (thaw, settle) in zip(scenes["views"], settle_plan(scenes)):
            for line in view_commands(view):
                game.command(line)
            extra = []
            ready, previous = place_view(game, view, previous, limit)
            if name and view["dimension"] != dimension:
                dimension = view["dimension"]
                load = loads.transition(limit)
                record["loads"].append([dimension, load])
                # A pack load rebuilds what is drawn: wait for it in full once.
                ready = sunrise_wait_loaded(game, view, limit) and ready
            failed = [c for c in setup_commands(view) if not game.command(c)]
            if view["setup"]:
                # Setup changes blocks: wait until they are drawn.
                ready = sunrise_wait_loaded(game, view, limit, moved=False) and ready
            if name and pack_problem(load):
                extra.append(pack_problem(load))
            if thaw:
                extra += sunrise_thaw(game, view["weather"], thaw, limit)
                if not game.command(FREEZE):
                    extra.append(f"{FREEZE} failed after the thaw")
            time.sleep(settle)
            camera = sunrise_camera(game("state"))
            camera["frozen"] = sunrise_frozen(game("command", "tick query", check=False))
            camera["settings"] = sunrise_settings(game("status"), game("options-get", "simulationDistance", "fov"))
            shot = game("screenshot", folder / f"{view['name']}.png", check=False)
            shots = [{"index": 0, "ticks": 0}]
            for index in range(1, scenes["phase_shots"]):
                # A phase shot: step the frozen world (its texture animations with it), let the ticks pass and a few
                # frames draw them, then another picture.
                step = phase_step(scenes, "sunrise")
                if not game.command(f"tick step {step}"):
                    extra.append(f"tick step {step} failed")
                    break
                time.sleep(step * 0.05 + 0.25)
                more = game("screenshot", folder / f"{view['name']}.p{index}.png", check=False)
                extra += [f"phase shot {index}: {p}" for p in capture_problems(more, scenes["size"])]
                shots.append({"index": index, "ticks": index * step})
            record_view(record["views"], view, camera, failed, ready, extra + capture_problems(shot, scenes["size"]),
                        expected)
            record["views"][view["name"]]["shots"] = shots
            print(f"sunrise {pack_key(pack)} {view['name']}: {record['views'][view['name']]['problems'] or 'ok'}",
                  flush=True)
    except (RuntimeError, OSError, subprocess.TimeoutExpired) as error:
        # The game died or stopped answering: the views not taken stay missing, the next pack still runs.
        record["notes"].append(f"stopped: {error}")
        print(f"sunrise {pack_key(pack)}: stopped: {error}", flush=True)
    finally:
        try:
            if proc is not None:
                try:
                    game("quit", check=False, timeout=120)
                except (OSError, subprocess.TimeoutExpired) as error:
                    record["notes"].append(f"quit: {error}")
                stop_game(proc)
            if log.exists():
                shutil.copy(log, folder / "game.log")
                if name:
                    prefix = f"shader pack {name}: "
                    record["pack_notes"] = [l[l.find(prefix):] for l in log.read_text(errors="replace").splitlines()
                                            if prefix in l]
        finally:
            shutil.rmtree(sock_dir, ignore_errors=True)
            write_run(folder, record)
    return record


def place_view(game, view, previous, limit):
    """Waits for a placed view: in full (teleport guard) unless the camera stays where the previous view was loaded;
    there, steady chunks are enough. Returns (ready, the view the next one may count as loaded): a view that never
    loaded is not, so the next view at its place waits in full."""
    ready = sunrise_wait_loaded(game, view, limit, moved=not same_place(view, previous))
    return ready, (view if ready else None)


def sunrise_thaw(game, weather, seconds, limit, now=time.time, sleep=time.sleep):
    """Thaws the frozen world for a weather change: `tick unfreeze` must succeed and `tick query` must report the
    game running before the `seconds` start; then the state must show the requested weather before the caller
    freezes it again. Returns the problems (Sunrise reports the weather flags, not the rain level)."""
    reply = game("command", "tick unfreeze", check=False)
    if "error" in reply or not reply.get("success"):
        return ["tick unfreeze failed"]
    deadline = now() + limit
    while sunrise_frozen(game("command", "tick query", check=False)) is not False:
        if now() >= deadline:
            return ["the world never thawed after tick unfreeze"]
        sleep(0.25)
    sleep(seconds)
    want = {"raining": weather in ("rain", "thunder"), "thundering": weather == "thunder"}
    deadline = now() + limit
    while True:
        got = game("state").get("weather") or {}
        if all(got.get(k) == v for k, v in want.items()):
            return []
        if now() >= deadline:
            return [f"the weather never reached {weather} while thawed: {got}"]
        sleep(0.25)


def capture_problems(shot, size):
    """A screenshot reply or event that failed, or whose picture is not the scene's size."""
    if "error" in shot:
        return [f"screenshot: {shot['error']}"]
    if [shot.get("width"), shot.get("height")] != list(size):
        return [f"the picture is {shot.get('width')}x{shot.get('height')}, the scene asks for {size[0]}x{size[1]}"]
    return []


def valid_views(record):
    """How many views of a run have a picture and no problem (none when its start checks failed)."""
    if record.get("start_problems"):
        return 0
    return sum(1 for v in record.get("views", {}).values() if not v.get("problems"))


# CLI -------------------------------------------------------------------------------------------------------------

def parser():
    p = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    sub = p.add_subparsers(dest="step", required=True)
    steps = {
        "check": "check the scene file and print the commands of each view",
        "world": "generate the shared world with vanilla Java",
        "sunrise": "take Sunrise's pictures of each --pack",
        "java": "take Java's pictures of each --pack (vanilla for none, Iris for a pack)",
        "compare": "write the comparison report of everything under --out",
        "run": "world (unless it exists), then none and each --pack on both sides, then compare",
    }
    for name, text in steps.items():
        s = sub.add_parser(name, help=text, description=text)
        s.add_argument("--scenes", default=str(SCENES),
                       help="the scene file (default: tools/shader-scenes/scenes.json)")
        if name == "check":
            continue
        s.add_argument("--out", required=True, help="the output folder; every run writes only inside it")
        if name == "compare":
            s.add_argument("--pairs", action="append", default=[],
                           help="also write side-by-side pictures of PACK/VIEW (repeatable), or all")
            continue
        if name != "world":
            s.add_argument("--pack", action="append", default=[],
                           help="a shader pack zip or folder, or none (repeatable; `run` always adds none)")
        s.add_argument("--replace", action="store_true", help="run again over this tool's earlier output")
        if name in ("sunrise", "java", "run"):
            s.add_argument("--jobs", type=int, default=2,
                           help="how many games run at once (default 2: with run, a Sunrise and a Java game side "
                                "by side); each needs a few GB of memory")
        if name in ("sunrise", "run"):
            s.add_argument("--sunrise-ctl", default=None, help="sunrise-ctl (default: the cargo target's fast build)")
            s.add_argument("--sunrise-client", default=None, help="sunrise-client (default: next to sunrise-ctl)")
            s.add_argument("--resources", default=None, help="prepared resources (default: .local/sunrise-client/26.3)")
        if name != "compare":
            s.add_argument("--display", choices=("headless", "current"), default="headless",
                           help="headless: each game in a gamescope headless display of its own, of the scene's size "
                                "(default); current: the desktop's display")
        if name in ("world", "java", "run"):
            s.add_argument("--mods-from", help="a folder holding the pinned Sodium, Iris and Fabric API jars")
            s.add_argument("--java-home", default=os.environ.get("JAVA_HOME"), help="a Java 25 (default: JAVA_HOME)")
            s.add_argument("--heap", default="4G", help="the Java game's -Xmx (default 4G)")
            s.add_argument("--java-timeout", type=int, default=1800,
                           help="seconds one Java game may take before it is stopped (default 1800)")
        if name == "run":
            s.add_argument("--pairs", action="append", default=[], help="as compare --pairs")
    return p


def resolve_sunrise(args):
    from tools import lab
    target = lab.cargo_target()
    args.sunrise_ctl = Path(args.sunrise_ctl or lab.binary(target / "fast", "sunrise-ctl"))
    args.sunrise_client = Path(args.sunrise_client or lab.binary(args.sunrise_ctl.parent, "sunrise-client"))
    args.resources = Path(args.resources or lab.RESOURCES)
    for path in (args.sunrise_ctl, args.sunrise_client, args.resources):
        if not path.exists():
            raise SystemExit(f"{path} not found (cargo build --profile fast -p sunrise-game --bin sunrise-client "
                             "-p sunrise-client-lib --bin sunrise-ctl; resources: tools/prepare_native_client.py)")


def absolute_paths(args, cwd=None):
    """Makes every path argument absolute against the caller's folder: each game runs with its own game folder as
    its working directory, where a relative --out or --pack would point somewhere else."""
    base = Path(cwd or os.getcwd())

    def absolute(value):
        return value if value is None or value == "none" else str((base / value).resolve())
    for key in ("scenes", "out", "sunrise_ctl", "sunrise_client", "resources", "mods_from", "java_home"):
        if getattr(args, key, None) is not None:
            setattr(args, key, absolute(getattr(args, key)))
    if hasattr(args, "pack"):
        args.pack = [absolute(p) for p in args.pack]
    return args


def unscored_packs(report, keys):
    """The requested packs with no scored view: `run` fails on any, so a scored `none` can't hide a pack that failed."""
    return [key for key in keys if not report["packs"].get(key, {}).get("compared")]


def iris_scope(args, packs):
    """D-11 (docs/cleanroom/decisions.md): Iris runs only for the checked-in scene file's views."""
    if any(p != "none" for p in packs) and Path(args.scenes).resolve() != SCENES.resolve():
        raise SystemExit(f"Java with a pack (Iris) runs only the checked-in scene file {SCENES.relative_to(ROOT)} "
                         "(docs/cleanroom/decisions.md, D-11); --scenes names another")


def main(argv=None):
    args = absolute_paths(parser().parse_args(argv))
    try:
        scenes = load_scenes(args.scenes)
        packs = getattr(args, "pack", [])
        if args.step == "run" and "none" not in packs:
            packs = ["none", *packs]
        keys = [pack_key(pack) for pack in packs]
        twice = sorted({k for k in keys if keys.count(k) > 1})
        if twice:
            raise SceneError(f"--pack: several packs share the folder name {', '.join(twice)}")
    except SceneError as error:
        raise SystemExit(f"scene error: {error}")
    if args.step == "check":
        for view in scenes["views"]:
            print(view["name"])
            for line in view_commands(view) + setup_commands(view):
                print(f"  {line}")
        return 0
    Path(args.out).mkdir(parents=True, exist_ok=True)
    if args.step == "compare":
        return summary(compare(args.out, scenes, args.pairs))
    if not packs and args.step != "world":
        raise SystemExit("no --pack given")
    if args.step in ("java", "run"):
        iris_scope(args, packs)
    from tools import shader_scenes_java as java
    if args.step == "world":
        java.run_world(args, scenes)
        return 0
    if args.step == "run" and world_generation(args.out) is None:
        java.run_world(args, scenes)
    # One task per side and pack; `--jobs` games run at once (each has its own folders, display and socket).
    tasks = []
    if args.step in ("sunrise", "run"):
        resolve_sunrise(args)
        tasks.append([("sunrise", p, lambda p=p: run_sunrise(args, scenes, p)) for p in packs])
    if args.step in ("java", "run"):
        setup = java.java_setup(args)
        if any(p != "none" for p in packs):
            java.iris_mods(args.mods_from)  # fill the jar cache once, before games run side by side
        tasks.append([("java", p, lambda p=p: java.run_java(args, scenes, p, setup)) for p in packs])
    # Interleaved, so that with two jobs each side's games run beside the other side's.
    order = [t for group in itertools.zip_longest(*tasks) for t in group if t is not None]
    with concurrent.futures.ThreadPoolExecutor(max_workers=max(1, args.jobs)) as pool:
        records = list(pool.map(lambda t: t[2](), order))
    # A capture step fails when a pack it ran has no view without a problem.
    empty = [f"{side} {pack_key(p)}" for (side, p, _), record in zip(order, records) if not valid_views(record)]
    for run in empty:
        print(f"{run}: no view without a problem", file=sys.stderr)
    if args.step == "run":
        report = compare(args.out, scenes, args.pairs)
        unscored = unscored_packs(report, keys)
        for key in unscored:
            print(f"{key}: no view scored", file=sys.stderr)
        return max(summary(report), int(bool(empty or unscored)))
    return int(bool(empty))


if __name__ == "__main__":
    sys.exit(main())
