"""tools/shader_scenes.py and shader_scenes_report.py: the scene file, the commands each view sends, the camera
and pack checks, the capture of one view through a scripted game, the staged pack's identity, the PNG reader and the
exact comparison of base and head (no game is started)."""

import argparse
import contextlib
import copy
import hashlib
import io
import json
import os
from pathlib import Path
import struct
import tempfile
import unittest
from unittest import mock
import zlib

from tools import perf_bench
from tools import shader_scenes as ss
from tools import shader_scenes_report as rp


def scene(**changes):
    data = {
        "format": 1, "seed": 318, "size": [4, 2],
        "views": [{"name": "a", "dimension": "overworld", "pos": [1.5, 64.0, -2.5], "yaw": 90.0, "pitch": 10.0,
                   "time": 6000, "weather": "rain", "setup": ["/setblock 1 64 1 minecraft:torch"]},
                  {"name": "b", "dimension": "the_nether", "pos": [0.0, 70.0, 3.0]}],
    }
    data.update(changes)
    return data


def chunk(kind, body):
    return struct.pack(">I", len(body)) + kind + body + struct.pack(">I", zlib.crc32(kind + body))


def png(width, height, rows, colour=2, filters=None, idat=None):
    """A PNG with these raw rows (bytes per row: width * channels), each with its filter type byte; `idat` replaces
    the compressed stream."""
    filters = filters or [0] * height
    raw = b"".join(bytes([f]) + row for f, row in zip(filters, rows))
    return (b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, colour, 0, 0, 0))
            + chunk(b"IDAT", zlib.compress(raw) if idat is None else idat) + chunk(b"IEND", b""))


class FakeClock:
    """time.time and time.sleep for code that polls: sleeping only moves the clock."""

    def __init__(self):
        self.t = 1000.0

    def time(self):
        return self.t

    def sleep(self, seconds):
        self.t += seconds


class SceneFileTests(unittest.TestCase):
    def test_the_committed_scene_file_parses(self):
        scenes = ss.load_scenes()
        names = [v["name"] for v in scenes["views"]]
        self.assertEqual(len(names), len(set(names)))
        self.assertEqual(scenes["size"], [1280, 720])
        # screenshot-sequence refuses to run unless these are off (screenshot_sequence.rs, sequence_refusal).
        self.assertEqual({k: scenes["graphics"][k] for k in ("lodDistance", "taa", "frameGeneration", "shadowCache")},
                         {"lodDistance": "0", "taa": "false", "frameGeneration": "off", "shadowCache": "false"})
        self.assertEqual(scenes["options"]["chunkSectionFadeInTime"], "0.0")

    def test_defaults_are_filled_and_setup_loses_its_slash(self):
        scenes = ss.parse_scenes(scene())
        self.assertEqual((scenes["frames"], scenes["step_ms"]), (64, 50.0))
        self.assertEqual((scenes["settle_seconds"], scenes["move_settle_seconds"]), (2.0, 2.0))
        self.assertEqual(scenes["load_timeout_seconds"], 90.0)
        self.assertEqual(scenes["graphics"], {})
        b = scenes["views"][1]
        self.assertEqual((b["yaw"], b["pitch"], b["time"], b["weather"], b["setup"]), (0.0, 0.0, None, None, []))
        self.assertEqual(scenes["views"][0]["setup"], ["setblock 1 64 1 minecraft:torch"])
        self.assertEqual(ss.parse_scenes(scene(settle_seconds=3))["move_settle_seconds"], 3.0)

    def refused(self, data, field):
        with self.assertRaises(ss.SceneError) as caught:
            ss.parse_scenes(data)
        self.assertIn(field, str(caught.exception))

    def test_refusals_name_the_field(self):
        views = scene()["views"]

        def view(**changes):
            changed = copy.deepcopy(views)
            changed[0].update(changes)
            return scene(views=changed)
        self.refused([], "expected an object")
        self.refused(scene(extra=1), "'extra'")
        self.refused(scene(phase_shots=9), "'phase_shots'")
        self.refused({k: v for k, v in scene().items() if k != "seed"}, "missing 'seed'")
        self.refused(scene(format=2), "format")
        self.refused(scene(format=True), "format")
        self.refused(scene(seed=True), "seed")
        self.refused(scene(seed=2**63), "seed")
        self.refused(scene(size=[1280]), "size")
        self.refused(scene(size=[0, 720]), "size")
        self.refused(scene(size=[8193, 720]), "size")
        self.refused(scene(frames=0), "frames")
        self.refused(scene(frames=10001), "frames")
        self.refused(scene(frames=64.0), "frames")
        self.refused(scene(step_ms=0), "step_ms")
        self.refused(scene(step_ms=float("inf")), "step_ms")
        self.refused(scene(step_ms="50"), "step_ms")
        self.refused(scene(settle_seconds=-1), "settle_seconds")
        self.refused(scene(views=[]), "views")
        self.refused(scene(views=[views[0], views[0]]), "used twice")
        self.refused(scene(views=[1]), "views[0]")
        self.refused(view(name="../up"), "name")
        self.refused(view(name="A"), "name")
        self.refused(view(dimension="minecraft:overworld"), "dimension")
        self.refused(view(pos=[0, float("nan"), 0]), "pos")
        self.refused(view(pos=[0, 64]), "pos")
        self.refused(view(pitch=90.5), "pitch")
        self.refused(view(yaw="90"), "yaw")
        self.refused(view(weather="snow"), "weather")
        self.refused(view(time=-1), "time")
        self.refused(view(time=True), "time")
        self.refused(view(setup=["say a\nsay b"]), "setup")
        self.refused(view(setup="say a"), "setup")
        self.refused(view(look=1), "'look'")
        self.refused(scene(options={"fov": 70}), "options")
        self.refused(scene(options={"a:b": "1"}), "options")
        self.refused(scene(graphics={"taa": "false\nhdr:true"}), "graphics")
        self.refused(scene(gamerules={"a b": "1"}), "gamerules")

    def test_the_end_comes_last_and_never_first(self):
        end = {"name": "e", "dimension": "the_end", "pos": [70.5, 90.0, 0.5]}
        a, b = scene()["views"]
        self.assertEqual(len(ss.parse_scenes(scene(views=[a, b, end, dict(end, name="f")]))["views"]), 4)
        self.refused(scene(views=[end, a]), "views[0].dimension")
        self.refused(scene(views=[a, end, b]), "views[2].dimension")

    def test_a_bad_file_is_a_scene_error(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "s.json"
            path.write_text("{")
            with self.assertRaises(ss.SceneError):
                ss.load_scenes(path)
            with self.assertRaises(ss.SceneError):
                ss.load_scenes(Path(tmp) / "missing.json")

    def test_scene_hash_ignores_key_order_and_sees_values(self):
        a = ss.parse_scenes(scene())
        b = ss.parse_scenes(json.loads(json.dumps(scene()), object_pairs_hook=lambda p: dict(reversed(p))))
        self.assertEqual(rp.scene_hash(a), rp.scene_hash(b))
        self.assertNotEqual(rp.scene_hash(a), rp.scene_hash(ss.parse_scenes(scene(frames=63))))


class CommandTests(unittest.TestCase):
    def test_view_commands_teleport_with_a_decimal_point(self):
        a, b = ss.parse_scenes(scene(views=[
            {"name": "a", "dimension": "overworld", "pos": [1.5, 64, -2], "yaw": 90, "pitch": 10, "time": 6000,
             "weather": "rain", "setup": ["setblock 1 64 1 minecraft:torch"]},
            {"name": "b", "dimension": "the_nether", "pos": [0.00001, 70.0, 3.0]}]))["views"]
        # tp centres a bare integer x or z on its block: 64 and -2 must go out as 64.0 and -2.0; the command reader
        # takes no exponent.
        self.assertEqual(ss.view_commands(a), [
            "execute in minecraft:overworld run tp @a 1.5 64.0 -2.0 90.0 10.0", "time set 6000", "weather rain"])
        self.assertEqual(ss.view_commands(b), ["execute in minecraft:the_nether run tp @a 0.00001 70.0 3.0 0.0 0.0"])
        self.assertEqual(ss.setup_commands(a), ["execute in minecraft:overworld run setblock 1 64 1 minecraft:torch"])

    def test_start_commands_and_options(self):
        scenes = ss.parse_scenes(scene(options={"gamma": "0.5", "fov": "0.0"},
                                       gamerules={"advance_time": "false", "random_tick_speed": "0"}))
        self.assertEqual(ss.start_commands(scenes), [
            "gamerule advance_time false", "gamerule random_tick_speed 0",
            "gamerule advance_time", "gamerule random_tick_speed", "gamemode spectator @a", "tick freeze"])
        self.assertEqual(ss.options_text(scenes["options"], {"version": "5023"}), "version:5023\ngamma:0.5\nfov:0.0\n")

    def test_start_problems_compare_the_queried_rules(self):
        scenes = ss.parse_scenes(scene(gamerules={"advance_time": "false", "random_tick_speed": "0"}))
        self.assertEqual(ss.start_problems(scenes, {"gamerule advance_time": 0, "gamerule random_tick_speed": 0,
                                                    "tick freeze": 1}), [])
        self.assertEqual(ss.start_problems(scenes, {"gamerule advance_time": 1, "tick freeze": None}), [
            "gamerule advance_time is 1, the scene asks for false",
            "gamerule random_tick_speed is None, the scene asks for 0", "tick freeze failed"])

    def test_settle_time(self):
        scenes = ss.parse_scenes(scene(settle_seconds=1, move_settle_seconds=6))
        a, b = scenes["views"]
        self.assertEqual(ss.settle_time(scenes, a, None), 6.0)
        self.assertEqual(ss.settle_time(scenes, dict(a, time=18000), a), 1.0)
        self.assertEqual(ss.settle_time(scenes, b, a), 6.0)
        self.assertEqual(ss.settle_time(scenes, dict(b, settle_seconds=9.0), a), 9.0)


class CameraTests(unittest.TestCase):
    view = ss.parse_scenes(scene())["views"][0]

    def camera(self, **changes):
        camera = {"dimension": "minecraft:overworld", "pos": [1.5, 64.0, -2.5], "yaw": 90.0, "pitch": 10.0,
                  "screen": "playing", "spectator": True, "frozen": True, "day_time": 6000, "raining": True,
                  "thundering": False}
        camera.update(changes)
        return camera

    def test_matching_cameras_have_no_problem(self):
        self.assertEqual(ss.camera_problems(self.view, self.camera()), [])
        self.assertEqual(ss.camera_problems(self.view, self.camera(yaw=-270.0)), [])

    def test_each_difference_is_named(self):
        self.assertEqual(ss.camera_problems(self.view, None), ["no camera was read back"])
        self.assertEqual(ss.camera_problems(self.view, self.camera(dimension="minecraft:the_end")),
                         ["dimension minecraft:the_end, view asks for minecraft:overworld"])
        self.assertEqual(ss.camera_problems(self.view, self.camera(pos=[1.5, 64.0, -2.0])),
                         ["position [1.5, 64.0, -2.0], view asks for [1.5, 64.0, -2.5]"])
        self.assertEqual(ss.camera_problems(self.view, self.camera(yaw=91.0, pitch=None)),
                         ["yaw 91.0, view asks for 90.0", "pitch None, view asks for 10.0"])
        self.assertEqual(ss.camera_problems(self.view, self.camera(screen="paused")), ["screen paused is open"])
        self.assertEqual(ss.camera_problems(self.view, self.camera(spectator=False)), ["not in spectator mode"])
        self.assertEqual(ss.camera_problems(self.view, self.camera(frozen=None)),
                         ["the game is not frozen (tick freeze): frozen read back as None"])
        # A day later at the same time of day is another day tick (the moon's phase follows the day).
        self.assertEqual(ss.camera_problems(self.view, self.camera(day_time=30000)), ["time 30000, view asks for 6000"])
        self.assertEqual(ss.camera_problems(self.view, self.camera(day_time=6001)), ["time 6001, view asks for 6000"])
        self.assertEqual(ss.camera_problems(self.view, self.camera(raining=False)),
                         ["weather clear, view asks for rain"])
        self.assertEqual(ss.camera_problems(self.view, self.camera(thundering=True)),
                         ["weather thunder, view asks for rain"])

    def test_settings_and_frozen_read_back(self):
        scenes = ss.parse_scenes(scene(options={"renderDistance": "8", "simulationDistance": "6", "fov": "0.25"}))
        expected = ss.settings_expected(scenes)
        self.assertEqual(expected, {"render_distance": 8, "simulation_distance": 6, "fov": 80})
        self.assertEqual(ss.settings_problems(expected, {"render_distance": 8, "simulation_distance": 6, "fov": 80}),
                         [])
        self.assertEqual(ss.settings_problems(expected, {"render_distance": 12, "simulation_distance": 6, "fov": 80}),
                         ["render_distance 12, the scene pins 8"])
        self.assertEqual(ss.settings_problems(expected, None), ["no drawing settings were read back"])
        self.assertEqual(ss.sunrise_settings({"render_distance": 8}, {"options": {
            "simulationDistance": {"value": 6}, "fov": {"value": 80}}}),
            {"render_distance": 8, "simulation_distance": 6, "fov": 80})
        self.assertIs(ss.sunrise_frozen({"success": True, "chat": ["The game is frozen", "Target tick rate"]}), True)
        self.assertIs(ss.sunrise_frozen({"success": True, "chat": ["The game is running normally"]}), False)
        self.assertIsNone(ss.sunrise_frozen({"success": False, "chat": ["The game is frozen"]}))
        self.assertIsNone(ss.sunrise_frozen({"error": "no game"}))


class PackTests(unittest.TestCase):
    def test_pack_keys(self):
        self.assertEqual(ss.pack_key("none"), "none")
        self.assertEqual(ss.pack_key("/p/BSL_v10.1.8.zip"), "bsl_v10_1_8")
        self.assertEqual(ss.pack_key("/p/Solas Shader V3.9.zip"), "solas_shader_v3_9")
        for bad in ("/p/none.zip", "/p/___.zip"):
            with self.assertRaises(ss.SceneError):
                ss.pack_key(bad)

    def test_packs_sharing_a_folder_name_are_refused(self):
        with contextlib.redirect_stderr(io.StringIO()) as printed:
            self.assertEqual(ss.main(["capture", "--out", "o", "--side", "base", "--bin-dir", "b",
                                      "--pack", "/a/BSL.zip", "--pack", "/b/bsl.zip"]), rp.SETUP_FAILED)
        self.assertIn("share the folder name bsl", printed.getvalue())

    def test_the_recorded_identity_is_the_staged_copys(self):
        with tempfile.TemporaryDirectory() as tmp:
            tmp = Path(tmp)
            source = tmp / "Pack.zip"
            source.write_bytes(b"one")
            name, digest = ss.stage_pack(source, tmp / "g1")
            staged = tmp / "g1" / "shaderpacks" / "Pack.zip"
            self.assertEqual((name, digest, staged.read_bytes()), ("Pack.zip", rp.sha256_file(staged), b"one"))
            # The source changed after the copy: the identity is still the copy's.
            with mock.patch.object(ss.shutil, "copyfile", side_effect=lambda a, b: (Path(b).write_bytes(b"one"),
                                                                                     Path(a).write_bytes(b"two"))):
                _, again = ss.stage_pack(source, tmp / "g2")
            self.assertEqual(again, digest)
            staged.write_bytes(b"changed")
            self.assertNotEqual(ss.staged_digest(staged), digest)

    def test_a_folder_pack_is_hashed_with_what_its_links_point_at(self):
        with tempfile.TemporaryDirectory() as tmp:
            tmp = Path(tmp)
            outside = tmp / "outside"
            (outside / "lib").mkdir(parents=True)
            (outside / "lib" / "common.glsl").write_text("x")
            pack = tmp / "Pack"
            (pack / "shaders").mkdir(parents=True)
            (pack / "shaders" / "final.fsh").write_text("f")
            (pack / "shaders" / "lib").symlink_to(outside / "lib")
            _, linked = ss.stage_pack(pack, tmp / "g1")
            plain = tmp / "plain" / "Pack"
            (plain / "shaders" / "lib").mkdir(parents=True)
            (plain / "shaders" / "final.fsh").write_text("f")
            (plain / "shaders" / "lib" / "common.glsl").write_text("x")
            self.assertEqual(linked, ss.staged_digest(plain))
            (outside / "lib" / "common.glsl").write_text("y")
            _, changed = ss.stage_pack(pack, tmp / "g2")
            self.assertNotEqual(changed, linked)
            with self.assertRaises(SystemExit):
                ss.stage_pack(tmp / "missing.zip", tmp / "g3")


class PackLogTests(unittest.TestCase):
    def test_pack_events_in_order(self):
        log = "\n".join([
            "[12:00:00] INFO shader pack BSL.zip: 41 programs (gbuffers_basic ...) in 900 ms",
            "[12:00:01] WARN shader pack BSL.zip: composite1: a note",
            "[12:00:02] INFO shader pack Other.zip: 3 programs (a) in 1 ms",
            "[12:00:03] ERROR shader pack BSL.zip: turned off: no gbuffers program compiled",
            "[12:00:04] INFO shader pack BSL.zip: 39 programs (x) in 800 ms",
        ])
        self.assertEqual(ss.pack_events(log, "BSL.zip"),
                         [("programs", 41), ("off", "no gbuffers program compiled"), ("programs", 39)])
        self.assertEqual(ss.pack_events(log, "BSL"), [])

    def watch(self, *counts_and_events):
        """A PackWatch over a scripted log: each read returns the next list (the last one repeats)."""
        reads = list(counts_and_events)
        clock = FakeClock()
        return ss.PackWatch(lambda: reads.pop(0) if len(reads) > 1 else reads[0], clock.time, clock.sleep)

    def test_each_dimension_change_waits_for_its_load(self):
        one, two = [("programs", 41)], [("programs", 41), ("programs", 40)]
        watch = self.watch([], one, one, one, one, two, two)
        watch.dimension_changed()  # the world opens
        self.assertEqual(watch.before_capture(10), ([], []))  # waited for the first load
        self.assertEqual(watch.after_capture(), [])
        self.assertEqual(watch.before_capture(10), ([], []))  # same dimension: nothing new is expected
        self.assertEqual(watch.after_capture(), [])
        watch.dimension_changed()
        self.assertEqual(watch.before_capture(10), ([], []))
        self.assertEqual(watch.after_capture(), [])

    def test_a_load_that_never_comes_or_was_turned_off(self):
        watch = self.watch([])
        watch.dimension_changed()
        self.assertEqual(watch.before_capture(10)[0], ["the pack never finished loading in this dimension"])
        watch = self.watch([("off", "no gbuffers program")])
        watch.dimension_changed()
        self.assertEqual(watch.before_capture(10)[0], ["the pack was turned off: no gbuffers program"])

    def test_an_unasked_load_invalidates_both_captures_around_it(self):
        one, two = [("programs", 41)], [("programs", 41), ("programs", 41)]
        watch = self.watch(one, one, two)
        watch.dimension_changed()
        self.assertEqual(watch.before_capture(10), ([], []))
        self.assertEqual(watch.after_capture(), [])
        mine, previous = watch.before_capture(10)
        self.assertEqual(mine, ["1 pack load(s) came without a dimension change"])
        self.assertEqual(previous, ["1 pack load(s) came without a dimension change after this capture; it may have "
                                    "run during it"])

    def test_a_load_during_the_capture(self):
        watch = self.watch([("programs", 41)], [("programs", 41)] * 2)
        watch.dimension_changed()
        self.assertEqual(watch.before_capture(10), ([], []))
        self.assertEqual(watch.after_capture(), ["1 pack load(s) came during the capture"])

    def test_the_reply_must_name_the_staged_pack(self):
        reply = {"pack": {"kind": "shaders", "name": "/run/game/shaderpacks/BSL.zip"}}
        self.assertEqual(ss.pack_reply_problems(reply, "BSL.zip"), [])
        self.assertEqual(ss.pack_reply_problems(reply, "Other.zip"),
                         [f"the capture drew {reply['pack']}, not Other.zip"])
        self.assertEqual(len(ss.pack_reply_problems({"pack": {"kind": "none", "name": None}}, "BSL.zip")), 1)
        self.assertEqual(ss.pack_reply_problems({"pack": {"kind": "none", "name": None}}, None), [])
        self.assertEqual(ss.pack_reply_problems(reply, None), [f"a pack ran: {reply['pack']}"])
        self.assertEqual(ss.pack_reply_problems({}, None), ["the capture's reply names no pack"])


class ReadinessTests(unittest.TestCase):
    view = ss.parse_scenes(scene())["views"][0]

    @staticmethod
    def status(chunks, pending=0, dimension="minecraft:overworld", pos=(1.5, 64.0, -2.5), working=0):
        return {"dimension": dimension, "player": {"position": list(pos)}, "pending_chunks": pending,
                "screen": "playing", "chunks": chunks, "generated_chunks": chunks, "render_distance": 2,
                "meshed_sections": 0, "streaming": {"meshing": working}}

    def run_statuses(self, statuses, limit, moved=True):
        clock = FakeClock()

        def game(*words, timeout=600):
            elapsed = clock.time() - 1000.0
            return next(s for until, s in statuses if elapsed < until)
        with mock.patch.object(perf_bench, "time", clock):
            return ss.wait_loaded(game, self.view, limit, moved), clock.time() - 1000.0

    def test_an_empty_queue_right_after_the_teleport_is_not_ready(self):
        # view_chunks(2) = 10. Nothing queued and steady counts for 3 s, but only 4 chunks: not loaded yet.
        statuses = [(3.0, self.status(4)), (4.0, self.status(7, pending=3)), (float("inf"), self.status(12))]
        ready, at = self.run_statuses(statuses, 30)
        self.assertTrue(ready)
        self.assertGreaterEqual(at, 5.0)

    def test_at_the_same_place_only_steady_chunks_are_waited_for(self):
        self.assertTrue(self.run_statuses([(float("inf"), self.status(4))], 30, moved=False)[0])
        self.assertFalse(self.run_statuses([(float("inf"), self.status(4))], 30)[0])

    def test_another_dimension_or_position_never_counts(self):
        self.assertFalse(self.run_statuses([(float("inf"), self.status(12, dimension="minecraft:the_nether"))], 10)[0])
        self.assertFalse(self.run_statuses([(float("inf"), self.status(12, pos=(1.5, 64.0, 0.0)))], 10)[0])


class FakeGame:
    """A scripted Sunrise: answers the control commands capture_view sends; screenshot-sequence writes `picture`
    (or replies with the next of `refusals` first)."""

    def __init__(self, view, picture, refusals=(), pack=None, day_after=None, writes=True, clock=None, log=None,
                 loads_on_teleport=()):
        self.view, self.picture, self.refusals, self.writes = view, picture, list(refusals), writes
        self.clock = clock or {}
        # The pack loads the game logs: `loads_on_teleport` are added to `log` when the camera is teleported.
        self.log, self.loads_on_teleport = log, list(loads_on_teleport)
        self.pack = pack or {"kind": "none", "name": None}
        self.day, self.day_after, self.sent, self.sequences = view["time"] or 0, day_after, [], 0

    def __call__(self, *words, check=True, timeout=600):
        self.sent.append(words)
        if words[0] == "command":
            if " run tp @a " in words[1] and self.log is not None:
                self.log += self.loads_on_teleport
            if words[1] == "tick query":
                return {"success": True, "chat": ["The game is frozen"]}
            return {"success": True, "result": 1}
        if words[0] == "status":
            return ReadinessTests.status(12, pos=self.view["pos"],
                                         dimension=f"minecraft:{self.view['dimension']}")
        if words[0] == "options-get":
            return {"options": {"simulationDistance": {"value": 8}, "fov": {"value": 70}}}
        if words[0] == "state":
            state = {"dimension": f"minecraft:{self.view['dimension']}", "screen": "playing",
                     "gamemode": "spectator", "player": {"position": self.view["pos"], "yaw": self.view["yaw"],
                                                         "pitch": self.view["pitch"]},
                     "time": {"day_time": self.day}, "weather": {"raining": self.view["weather"] == "rain",
                                                                  "thundering": False}}
            return state
        if words[0] == "screenshot-sequence":
            self.sequences += 1
            if self.refusals:
                return {"error": self.refusals.pop(0)}
            if self.writes:
                Path(words[3]).write_bytes(self.picture)
            if self.day_after is not None:
                self.day = self.day_after
            # Frame 8 of a 50 ms clock as the game sends it: 0.35 and 0.05 s, each rounded to an f32.
            return {"ok": True, "size": [4, 2], "frames": words[1], "step_ms": 50.0,
                    "frame": {"counter": words[1], "elapsed": 0.3499999940395355, "delta": 0.05000000074505806},
                    "pack": self.pack, **self.clock}
        raise AssertionError(words)

    def command(self, line):
        return self("command", line, check=False).get("success", False)


class CaptureViewTests(unittest.TestCase):
    scenes = ss.parse_scenes(scene(settle_seconds=0, frames=8, options={"renderDistance": "2"}))
    view = scenes["views"][0]
    picture = png(4, 2, [bytes(12), bytes(12)])

    def capture(self, game, staged=None):
        clock = FakeClock()
        watch = ss.PackWatch(lambda: [], clock.time, clock.sleep)
        with tempfile.TemporaryDirectory() as tmp, mock.patch.object(perf_bench, "time", clock), \
                mock.patch.object(ss, "time", clock):
            record, before, ready = ss.capture_view(game, self.scenes, self.view, None, tmp, watch, staged,
                                                    ss.settings_expected(self.scenes), sleep=clock.sleep)
            files = sorted(p.name for p in Path(tmp).iterdir())
        return record, before, ready, files

    def test_a_good_capture_records_the_files_hash_and_the_clock(self):
        game = FakeGame(self.view, self.picture)
        record, before, ready, files = self.capture(game)
        self.assertEqual((record["problems"], before, ready, files), ([], [], True, ["a.png"]))
        self.assertEqual(record["picture"], "a.png")
        self.assertEqual(record["sha256"], hashlib.sha256(self.picture).hexdigest())
        sequence = next(w for w in game.sent if w[0] == "screenshot-sequence")
        self.assertEqual(sequence[1:3], (8, "50.0"))
        # The setup ran in the view's dimension, before the capture.
        setup = game.sent.index(("command", "execute in minecraft:overworld run setblock 1 64 1 minecraft:torch"))
        self.assertLess(setup, game.sent.index(sequence))

    def test_loading_refusals_are_taken_again_and_other_errors_are_not(self):
        loading = "screenshot-sequence needs a loaded world in play, with nothing left to load"
        game = FakeGame(self.view, self.picture, refusals=[loading, loading])
        record, *_ = self.capture(game)
        self.assertEqual((record["problems"], record["attempts"]), ([], 3))
        changed = "screenshot-sequence: the world, settings, shader pack or window size changed while it ran"
        game = FakeGame(self.view, self.picture, refusals=[changed] * 5)
        record, *_ = self.capture(game)
        self.assertEqual((record["attempts"], record["picture"]), (ss.CAPTURE_ATTEMPTS, None))
        self.assertEqual(record["problems"], [f"screenshot-sequence: {changed}"])
        game = FakeGame(self.view, self.picture, refusals=["screenshot-sequence needs SUNRISE_STILL=1"])
        record, *_ = self.capture(game)
        self.assertEqual((record["attempts"], record["sha256"]), (1, None))
        self.assertEqual(record["problems"], ["screenshot-sequence: screenshot-sequence needs SUNRISE_STILL=1"])

    def test_another_pack_a_moved_clock_or_no_file_is_a_problem(self):
        record, *_ = self.capture(FakeGame(self.view, self.picture, pack={"kind": "shaders", "name": "/x/Other.zip"}),
                                  staged="BSL.zip")
        self.assertEqual(record["problems"], ["the capture drew {'kind': 'shaders', 'name': '/x/Other.zip'}, not BSL.zip"])
        record, *_ = self.capture(FakeGame(self.view, self.picture, day_after=6001))
        self.assertEqual(record["problems"], ["the time moved during the capture: 6000 to 6001"])
        record, *_ = self.capture(FakeGame(self.view, self.picture, writes=False))
        self.assertEqual((record["problems"], record["picture"]), (["a.png was not written"], None))

    def test_a_capture_on_another_clock_is_a_problem(self):
        good = {"counter": 8, "elapsed": 0.3499999940395355, "delta": 0.05000000074505806}
        record, *_ = self.capture(FakeGame(self.view, self.picture, clock={"frame": dict(good, counter=7)}))
        self.assertEqual(record["problems"], ["the picture's frame counter is 7, not 8"])
        # 0.35 as an f64, not the f32 the game sends: another clock.
        record, *_ = self.capture(FakeGame(self.view, self.picture, clock={"frame": dict(good, elapsed=0.35)}))
        self.assertEqual(record["problems"], ["the picture's frame elapsed is 0.35, not 0.3499999940395355"])
        record, *_ = self.capture(FakeGame(self.view, self.picture, clock={"frame": dict(good, delta=0.0)}))
        self.assertEqual(record["problems"], ["the picture's frame delta is 0.0, not 0.05000000074505806"])
        record, *_ = self.capture(FakeGame(self.view, self.picture, clock={"frame": {"counter": 8}}))
        self.assertEqual(record["problems"], ["the picture's frame elapsed is None, not 0.3499999940395355",
                                              "the picture's frame delta is None, not 0.05000000074505806"])
        record, *_ = self.capture(FakeGame(self.view, self.picture, clock={"step_ms": 16.5}))
        self.assertEqual(record["problems"],
                         ["the capture drew 8 frames 16.5 ms apart, the scene asks for 8 frames 50.0 ms apart"])
        record, *_ = self.capture(FakeGame(self.view, self.picture, clock={"frames": 9, "frame": dict(good, counter=9)}))
        self.assertEqual(record["problems"],
                         ["the capture drew 9 frames 50.0 ms apart, the scene asks for 8 frames 50.0 ms apart",
                          "the picture's frame counter is 9, not 8"])


class PackEvidenceThroughCaptureTests(unittest.TestCase):
    """The pack-load evidence of a sequence of views, through capture_view with one PackWatch over the game's log."""
    scenes = ss.parse_scenes(scene(settle_seconds=0, frames=8, options={"renderDistance": "2"},
                                   views=[{"name": "a", "dimension": "overworld", "pos": [1.5, 64.0, -2.5]},
                                          {"name": "b", "dimension": "the_nether", "pos": [0.0, 70.0, 3.0]},
                                          {"name": "c", "dimension": "the_nether", "pos": [0.0, 70.0, 3.0],
                                           "time": 1000}]))
    picture = png(4, 2, [bytes(12), bytes(12)])
    pack = {"kind": "shaders", "name": "/g/shaderpacks/BSL.zip"}
    load = ("programs", 41)

    def views(self, log, loads):
        """Captures a, b and c in turn; `loads[name]` are the loads the game logs when that view's camera moves, and
        `loads["stray"]` adds one load after a's capture. Returns each view's problems."""
        clock = FakeClock()
        watch = ss.PackWatch(lambda: list(log), clock.time, clock.sleep)
        watch.dimension_changed()  # the world opens in a's dimension; its load is in the log already
        problems, previous, dimension = {}, None, "overworld"
        with tempfile.TemporaryDirectory() as tmp, mock.patch.object(perf_bench, "time", clock), \
                mock.patch.object(ss, "time", clock):
            for view in self.scenes["views"]:
                game = FakeGame(view, self.picture, pack=self.pack, log=log,
                                loads_on_teleport=loads.get(view["name"], []))
                record, before, _ = ss.capture_view(game, self.scenes, view, previous, tmp, watch, "BSL.zip",
                                                    ss.settings_expected(self.scenes),
                                                    new_dimension=view["dimension"] != dimension, sleep=clock.sleep)
                if previous is not None:
                    problems[previous["name"]] += before
                problems[view["name"]] = record["problems"]
                previous, dimension = view, view["dimension"]
                if view["name"] == "a" and "stray" in loads:
                    log.append(self.load)  # logged after a's reply, before b's teleport
        return problems

    def test_each_load_where_it_belongs(self):
        self.assertEqual(self.views([self.load], {"b": [self.load]}), {"a": [], "b": [], "c": []})

    def test_a_stray_load_before_a_move_never_stands_in_for_the_moves_load(self):
        problems = self.views([self.load], {"stray": True})
        unasked = "1 pack load(s) came without a dimension change"
        self.assertEqual(problems["a"], [f"{unasked} after this capture; it may have run during it"])
        self.assertEqual(problems["b"], [unasked, "the pack never finished loading in this dimension"])
        self.assertEqual(problems["c"], ["the pack never finished loading in this dimension"])

    def test_a_load_that_timed_out_is_still_awaited_in_that_dimension(self):
        problems = self.views([self.load], {})
        self.assertEqual(problems, {"a": [], "b": ["the pack never finished loading in this dimension"],
                                    "c": ["the pack never finished loading in this dimension"]})


class PngTests(unittest.TestCase):
    def read(self, data, size):
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "p.png"
            path.write_bytes(data)
            return rp.read_png(path, size)

    def test_rgba_with_every_filter_type(self):
        # Row 0 (None) is 10,20,30,255 | 40,50,60,255; each later row's raw bytes, filtered, give these pixels.
        rows = [bytes([10, 20, 30, 255, 40, 50, 60, 255]),
                bytes([11, 22, 33, 255, 40, 50, 60, 0]),
                bytes([1, 2, 3, 0, 2, 4, 6, 0]),
                bytes([6, 12, 18, 0, 22, 32, 42, 0]),
                bytes([1, 1, 1, 0, 1, 1, 1, 0])]
        width, height, channels, data = self.read(png(2, 5, rows, colour=6, filters=[0, 1, 2, 3, 4]), [2, 5])
        self.assertEqual((width, height, channels), (2, 5, 4))
        pixels = [tuple(data[i:i + 4]) for i in range(0, len(data), 4)]
        self.assertEqual(pixels, [
            (10, 20, 30, 255), (40, 50, 60, 255),
            (11, 22, 33, 255), (51, 72, 93, 255),
            (12, 24, 36, 255), (53, 76, 99, 255),
            (12, 24, 36, 127), (54, 82, 109, 191),
            (13, 25, 37, 127), (55, 83, 110, 191)])

    def test_rgb_round_trip(self):
        rgb = bytes(range(24))
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "p.png"
            rp.write_png(path, 4, 2, rgb)
            self.assertEqual(rp.read_png(path, [4, 2]), (4, 2, 3, rgb))

    def test_refusals(self):
        good = png(1, 1, [bytes(3)])
        cases = [
            (b"GIF89a", [1, 1], "not a PNG"),
            (png(2, 1, [bytes(6)]), [4, 2], "2x1, the scene asks for 4x2"),
            (png(2, 1, [bytes(2)], colour=0), [2, 1], "colour type 0"),
            (png(1, 1, [bytes(3)], filters=[7]), [1, 1], "filter type 7"),
            (good[:-12], [1, 1], "no IEND"),
            (good[:20], [1, 1], "runs past the end"),
            (good[:30] + bytes([good[30] ^ 1]) + good[31:], [1, 1], "CRC"),
            (png(1, 1, [bytes(3)], idat=b"not zlib"), [1, 1], "invalid image data"),
            (png(1, 1, [bytes(3)], idat=zlib.compress(b"\x00" + bytes(3))[:-3]), [1, 1], "cut short"),
            (png(1, 1, [bytes(6)], idat=zlib.compress(b"\x00" + bytes(6))), [1, 1], "cut short or longer"),
        ]
        for data, size, message in cases:
            with self.subTest(message=message), self.assertRaisesRegex(rp.PictureError, message):
                self.read(data, size)


class PixelTests(unittest.TestCase):
    def test_equal_pictures_and_one_changed_pixel(self):
        a = (4, 2, 3, bytes(range(24)))
        self.assertIsNone(rp.pixel_diff(a, a))
        changed = bytearray(a[3])
        changed[3 * 6 + 1] += 3  # x 2, y 1, green
        b = (4, 2, 3, bytes(changed))
        self.assertEqual(rp.pixel_diff(a, b), {"pixels": 1, "max_delta": 3, "box": [2, 1, 2, 1]})
        changed[0] += 200  # x 0, y 0, red
        self.assertEqual(rp.pixel_diff(a, (4, 2, 3, bytes(changed))), {"pixels": 2, "max_delta": 200,
                                                                         "box": [0, 0, 2, 1]})
        _, _, diff = rp.diff_rgb(a, b)
        self.assertEqual(diff[3 * 6:3 * 6 + 3], b"\xff\x00\x00")
        self.assertEqual(diff[0:3], bytes(v // 4 for v in a[3][0:3]))

    def test_rgb_against_rgba_compares_alpha_as_opaque(self):
        rgb = (1, 1, 3, bytes([10, 20, 30]))
        self.assertIsNone(rp.pixel_diff(rgb, (1, 1, 4, bytes([10, 20, 30, 255]))))
        self.assertEqual(rp.pixel_diff(rgb, (1, 1, 4, bytes([10, 20, 30, 254]))),
                         {"pixels": 1, "max_delta": 1, "box": [0, 0, 0, 0]})


class ReportTests(unittest.TestCase):
    scenes = ss.parse_scenes(scene())
    pixels = bytes(range(24))

    def make_run(self, out, side, key, pictures, **record):
        """A run folder as run_capture leaves it: each picture written and recorded with its sha256."""
        folder = Path(out) / side / key
        folder.mkdir(parents=True)
        (folder / rp.RUN_MARK).write_text("")
        views = {}
        for view, data in pictures.items():
            path = folder / f"{view}.png"
            rp.write_png(path, 4, 2, data)
            views[view] = {"picture": path.name, "sha256": rp.sha256_file(path), "problems": []}
        full = {"side": side, "pack": key, "pack_sha256": "p", "world_id": "w", "save_sha256": "s",
                "scene_hash": rp.scene_hash(self.scenes), "views": views, "notes": [], "start_problems": []}
        full.update(record)
        (folder / "run.json").write_text(json.dumps(full))
        return folder

    def both(self, out, head=None, **record):
        self.make_run(out, "base", "none", {"a": self.pixels, "b": self.pixels})
        return self.make_run(out, "head", "none", head or {"a": self.pixels, "b": self.pixels}, **record)

    def compare(self, out, **kwargs):
        return rp.compare(out, self.scenes, jobs=1, **kwargs)

    def rows(self, report, key="none"):
        return {r["view"]: r for r in report["packs"][key]["rows"]}

    def test_equal_runs_are_same(self):
        with tempfile.TemporaryDirectory() as out:
            self.both(out)
            report = self.compare(out)
            self.assertEqual(report["status"], rp.SAME)
            self.assertEqual([r["equal"] for r in self.rows(report).values()], [True, True])

    def test_one_changed_pixel_is_a_difference_with_its_picture(self):
        with tempfile.TemporaryDirectory() as out:
            changed = bytearray(self.pixels)
            changed[5] ^= 1
            self.both(out, head={"a": self.pixels, "b": bytes(changed)})
            report = self.compare(out)
            self.assertEqual(report["status"], rp.DIFFERENT)
            b = self.rows(report)["b"]
            self.assertEqual((b["equal"], b["pixels"], b["max_delta"], b["box"]), (False, 1, 1, [1, 0, 1, 0]))
            self.assertTrue((Path(out) / "report" / b["diff"]).is_file())
            self.assertTrue(self.rows(report)["a"]["equal"])

    def test_a_view_without_a_record_is_invalid_even_with_a_file(self):
        with tempfile.TemporaryDirectory() as out:
            folder = self.both(out)
            record = json.loads((folder / "run.json").read_text())
            del record["views"]["b"]
            (folder / "run.json").write_text(json.dumps(record))
            report = self.compare(out)
            self.assertEqual(report["status"], rp.INVALID)
            b = self.rows(report)["b"]
            self.assertFalse(b["valid"])
            self.assertIn("no record of this view (the capture never finished)", b["head"]["problems"])
            self.assertIn("head: picture b.png has no record; not compared", report["packs"]["none"]["notes"])
            self.assertTrue(self.rows(report)["a"]["equal"])

    def test_a_changed_missing_or_damaged_file_is_invalid(self):
        for change, message in ((lambda p: rp.write_png(p, 4, 2, bytes(24)), "its sha256 changed"),
                                (lambda p: p.unlink(), "b.png is missing")):
            with self.subTest(message=message), tempfile.TemporaryDirectory() as out:
                folder = self.both(out)
                change(folder / "b.png")
                report = self.compare(out)
                self.assertEqual(report["status"], rp.INVALID)
                self.assertTrue(any(message in p for p in self.rows(report)["b"]["head"]["problems"]))
        with tempfile.TemporaryDirectory() as out:
            folder = self.both(out)
            (folder / "b.png").write_bytes(b"\x89PNG\r\n\x1a\nbroken")
            record = json.loads((folder / "run.json").read_text())
            record["views"]["b"]["sha256"] = rp.sha256_file(folder / "b.png")
            (folder / "run.json").write_text(json.dumps(record))
            report = self.compare(out)
            self.assertEqual(report["status"], rp.INVALID)
            self.assertTrue(any("chunk header" in p for p in self.rows(report)["b"]["head"]["problems"]))

    def edit_head(self, out, change):
        folder = self.both(out)
        record = json.loads((folder / "run.json").read_text())
        change(record, folder)
        (folder / "run.json").write_text(json.dumps(record))
        return self.compare(out)

    def test_a_view_record_without_its_checks_or_its_own_picture_is_invalid(self):
        with tempfile.TemporaryDirectory() as out:
            report = self.edit_head(out, lambda record, _: record["views"]["a"].pop("problems"))
            self.assertEqual(report["status"], rp.INVALID)
            self.assertEqual(self.rows(report)["a"]["head"]["problems"], ["the record carries no checks of this view"])
            self.assertTrue(self.rows(report)["b"]["equal"])
        with tempfile.TemporaryDirectory() as out:
            # b's record names a's picture (with a's hash): the file is checked, but it is not b's picture.
            report = self.edit_head(out, lambda record, _: record["views"].update(b=dict(record["views"]["a"])))
            self.assertEqual(report["status"], rp.INVALID)
            self.assertEqual(self.rows(report)["b"]["head"]["problems"],
                             ["the record names no picture b.png with its hash (picture 'a.png')"])

    def test_a_malformed_record_is_invalid_not_a_crash(self):
        for change, problem in ((lambda r, _: r.update(views=None), "views is not an object of view records"),
                                (lambda r, _: r["views"].update(a=[]), "views is not an object of view records"),
                                (lambda r, _: r.pop("start_problems"), "start_problems is not a list of strings"),
                                (lambda r, _: r.update(notes="x"), "notes is not a list of strings")):
            with self.subTest(problem=problem), tempfile.TemporaryDirectory() as out:
                report = self.edit_head(out, change)
                self.assertEqual(report["status"], rp.INVALID)
                path = Path(out) / "head" / "none" / "run.json"
                self.assertEqual(self.rows(report)["a"]["head"]["problems"], [f"{path}: {problem}"])

    def test_a_picture_with_a_second_ihdr_is_invalid(self):
        with tempfile.TemporaryDirectory() as out:
            def second_header(record, folder):
                data = (folder / "a.png").read_bytes()
                header = data[8:8 + 25]  # the IHDR chunk: length, type, 13 bytes, CRC
                (folder / "a.png").write_bytes(data[:33] + header + data[33:])
                record["views"]["a"]["sha256"] = rp.sha256_file(folder / "a.png")
            report = self.edit_head(out, second_header)
            self.assertEqual(report["status"], rp.INVALID)
            path = Path(out) / "head" / "none" / "a.png"
            self.assertEqual(self.rows(report)["a"]["head"]["problems"], [f"{path}: a second IHDR"])

    def test_a_recorded_problem_or_start_problem_is_invalid(self):
        with tempfile.TemporaryDirectory() as out:
            folder = self.both(out)
            record = json.loads((folder / "run.json").read_text())
            record["views"]["a"]["problems"] = ["the view never finished loading"]
            (folder / "run.json").write_text(json.dumps(record))
            report = self.compare(out)
            self.assertEqual(self.rows(report)["a"]["head"]["problems"], ["the view never finished loading"])
            self.assertFalse(self.rows(report)["a"]["valid"])
            self.assertTrue(self.rows(report)["b"]["valid"])
            record["start_problems"] = ["tick freeze failed"]
            (folder / "run.json").write_text(json.dumps(record))
            report = self.compare(out)
            self.assertFalse(self.rows(report)["b"]["valid"])

    def test_pairs_of_other_worlds_scenes_or_packs_are_invalid(self):
        for change, message in (({"world_id": "w2"}, "different worlds"), ({"pack_sha256": "q"}, "different pack"),
                                ({"scene_hash": "s"}, "different scene"), ({"world_id": None}, "records no world_id"),
                                ({"save_sha256": "t"}, "different saves")):
            with self.subTest(message=message), tempfile.TemporaryDirectory() as out:
                self.both(out, **change)
                report = self.compare(out)
                self.assertEqual(report["status"], rp.INVALID)
                self.assertTrue(any(message in p for p in self.rows(report)["a"]["problems"]))
        with tempfile.TemporaryDirectory() as out:
            self.make_run(out, "base", "none", {"a": self.pixels, "b": self.pixels}, scene_hash="old")
            self.make_run(out, "head", "none", {"a": self.pixels, "b": self.pixels}, scene_hash="old")
            report = self.compare(out)
            self.assertIn("base was taken for another scene definition than this scene file",
                          self.rows(report)["a"]["problems"])

    def test_a_pack_on_one_side_or_none_at_all_is_invalid(self):
        with tempfile.TemporaryDirectory() as out:
            self.make_run(out, "base", "none", {"a": self.pixels, "b": self.pixels})
            report = self.compare(out, packs=["none", "bsl"])
            self.assertEqual(report["status"], rp.INVALID)
            self.assertEqual(self.rows(report)["a"]["head"]["problems"], ["no run of this pack"])
            self.assertEqual(self.rows(report, "bsl")["a"]["base"]["problems"], ["no run of this pack"])
        with tempfile.TemporaryDirectory() as out:
            self.assertEqual(self.compare(out)["status"], rp.INVALID)

    def test_the_report_folder_is_replaced_only_when_this_tool_made_it(self):
        with tempfile.TemporaryDirectory() as out:
            changed = bytearray(self.pixels)
            changed[0] ^= 1
            self.both(out, head={"a": bytes(changed), "b": self.pixels})
            self.compare(out)
            diff = Path(out) / "report" / "diff" / "none" / "a.png"
            self.assertTrue(diff.exists())
            rp.write_png(Path(out) / "head" / "none" / "a.png", 4, 2, self.pixels)
            folder = Path(out) / "head" / "none"
            record = json.loads((folder / "run.json").read_text())
            record["views"]["a"]["sha256"] = rp.sha256_file(folder / "a.png")
            (folder / "run.json").write_text(json.dumps(record))
            self.assertEqual(self.compare(out)["status"], rp.SAME)
            self.assertFalse(diff.exists())  # an old difference picture never outlives the comparison
            (Path(out) / "report" / rp.REPORT_MARK).unlink()
            with self.assertRaises(SystemExit):
                self.compare(out)

    def test_cli_compare_exit_status(self):
        with tempfile.TemporaryDirectory() as out:
            scene_file = Path(out) / "scenes.json"
            scene_file.write_text(json.dumps(scene()))
            self.both(out)
            with contextlib.redirect_stdout(io.StringIO()):
                self.assertEqual(ss.main(["compare", "--scenes", str(scene_file), "--out", out, "--jobs", "1"]),
                                 rp.SAME)
            os.unlink(Path(out) / "head" / "none" / "run.json")
            with contextlib.redirect_stdout(io.StringIO()) as printed:
                self.assertEqual(ss.main(["compare", "--scenes", str(scene_file), "--out", out, "--jobs", "1"]),
                                 rp.INVALID)
            self.assertIn("a: invalid: head: the run has no run.json", printed.getvalue())


class CliTests(unittest.TestCase):
    def test_check_prints_each_views_commands_and_refuses_a_bad_file(self):
        with tempfile.TemporaryDirectory() as out:
            scene_file = Path(out) / "scenes.json"
            scene_file.write_text(json.dumps(scene()))
            printed = io.StringIO()
            with contextlib.redirect_stdout(printed):
                self.assertEqual(ss.main(["check", "--scenes", str(scene_file)]), 0)
            self.assertEqual(printed.getvalue().splitlines(), [
                "a", "  execute in minecraft:overworld run tp @a 1.5 64.0 -2.5 90.0 10.0", "  time set 6000",
                "  weather rain", "  execute in minecraft:overworld run setblock 1 64 1 minecraft:torch",
                "b", "  execute in minecraft:the_nether run tp @a 0.0 70.0 3.0 0.0 0.0"])
            scene_file.write_text(json.dumps(scene(format=2)))
            with contextlib.redirect_stderr(io.StringIO()) as printed:
                self.assertEqual(ss.main(["check", "--scenes", str(scene_file)]), rp.SETUP_FAILED)
            self.assertIn("format: 2 is not 1", printed.getvalue())

    def test_the_documented_relative_paths_become_absolute(self):
        with tempfile.TemporaryDirectory() as tmp:
            args = ss.parser().parse_args(["run", "--out", "OUT", "--pack", "PACKS/BSL_v10.1.8.zip", "--pack", "none",
                                           "--base-bin-dir", "b", "--head-bin-dir", "h", "--resources", "res",
                                           "--head-env", "SUNRISE_PACK_PLAN=off"])
            ss.absolute_paths(args, tmp)
            root = Path(tmp).resolve()
            self.assertEqual(Path(args.out), root / "OUT")
            self.assertEqual(args.pack, [str(root / "PACKS/BSL_v10.1.8.zip"), "none"])
            self.assertEqual((Path(args.base_bin_dir), Path(args.head_bin_dir), Path(args.resources)),
                             (root / "b", root / "h", root / "res"))
            self.assertEqual(ss.env_pairs(args.head_env), {"SUNRISE_PACK_PLAN": "off"})
            self.assertTrue(Path(args.scenes).is_absolute())

    def test_env_values(self):
        self.assertEqual(ss.env_pairs(["A=1", "B_2=x=y", "C="]), {"A": "1", "B_2": "x=y", "C": ""})
        for bad in ("A", "=1", "1A=2", "A-B=1", "XDG_DATA_HOME=/home/me/.local/share", "SUNRISE_HOME=/x",
                    "SUNRISE_STILL=0", "PERF_BENCH_TRACE=/tmp/t.jsonl", "SUNRISE_SHADER_PACK=x.zip",
                    "WAYLAND_DISPLAY=wayland-1", "SUNRISE_LAUNCHER_HOME=/x"):
            with self.subTest(bad=bad), self.assertRaises(SystemExit):
                ss.env_pairs([bad])

    def test_the_game_environment(self):
        inherited = {"SUNRISE_SHADER_PACK": "x.zip", "WAYLAND_DISPLAY": "wayland-1", "PERF_BENCH_TRACE": "/t.jsonl",
                     "XDG_DATA_HOME": "/home/me/.local/share", "SUNRISE_HOME": "/home/me/.sunrise"}
        with tempfile.TemporaryDirectory() as tmp, mock.patch.dict(os.environ, inherited):
            # Even an extra value past env_pairs' refusal can't replace a scratch folder or the frozen scene.
            env = ss.game_environment(Path(tmp), {"SUNRISE_PACK_PLAN": "off", "XDG_STATE_HOME": "/home/me/state",
                                                  "SUNRISE_STILL": "0"})
            self.assertEqual((env["SUNRISE_STILL"], env["SUNRISE_PACK_PLAN"]), ("1", "off"))
            for key in ("SUNRISE_SHADER_PACK", "WAYLAND_DISPLAY", "PERF_BENCH_TRACE"):
                self.assertNotIn(key, env)
            for key in ("SUNRISE_HOME", "SUNRISE_LAUNCHER_HOME", "XDG_CONFIG_HOME", "XDG_DATA_HOME",
                        "XDG_CACHE_HOME", "XDG_STATE_HOME"):
                self.assertTrue(Path(env[key]).is_relative_to(Path(tmp)), key)

    def test_readiness_writes_no_inherited_trace(self):
        view = ss.parse_scenes(scene())["views"][0]
        clock = FakeClock()
        with tempfile.TemporaryDirectory() as tmp, mock.patch.object(perf_bench, "time", clock), \
                mock.patch.object(perf_bench, "TRACE", str(Path(tmp) / "trace.jsonl")):
            ready = ss.wait_loaded(lambda *words, timeout=600: ReadinessTests.status(12), view, 30)
            self.assertTrue(ready)
            self.assertEqual(list(Path(tmp).iterdir()), [])

    def test_run_folders_are_only_replaced_when_this_tool_made_them(self):
        with tempfile.TemporaryDirectory() as out:
            folder = ss.fresh_folder(Path(out) / "base" / "none", False)
            (folder / "a.png").write_bytes(b"x")
            with self.assertRaises(SystemExit):
                ss.fresh_folder(folder, False)
            again = ss.fresh_folder(folder, True)
            self.assertFalse((again / "a.png").exists())
            (again / rp.RUN_MARK).unlink()
            with self.assertRaises(SystemExit):
                ss.fresh_folder(folder, True)

    @staticmethod
    def build(tmp):
        """A build folder with stand-ins for both programs (only their files are hashed here)."""
        bins = Path(tmp) / "bin"
        bins.mkdir()
        for name in ("sunrise-client", "sunrise-ctl"):
            (bins / name).write_bytes(name.encode())
        return bins

    @staticmethod
    def save(folder, level=b"level"):
        (folder / "region").mkdir(parents=True)
        (folder / "level.dat").write_bytes(level)
        (folder / "region" / "r.0.0.mca").write_bytes(b"chunks")
        (folder / "session.lock").write_bytes(b"lock")  # never published or hashed
        return folder

    def test_a_published_world_is_staged_only_while_it_is_the_save_it_recorded(self):
        scenes = ss.parse_scenes(scene())
        with tempfile.TemporaryDirectory() as out, contextlib.redirect_stdout(io.StringIO()):
            ss.publish_world(out, self.save(Path(out) / "made"), scenes, self.build(out))
            generation = ss.world_generation(out)
            identity = ss.copy_world(out, Path(out) / "g1", scenes)
            self.assertEqual(identity, {"world_id": generation["world_id"], "save_sha256": generation["save_sha256"],
                                        "scene_hash": rp.scene_hash(scenes)})
            self.assertFalse((Path(out) / "g1" / "saves" / ss.WORLD / "session.lock").exists())
            (Path(out) / "world" / ss.WORLD / "region" / "r.0.0.mca").write_bytes(b"edited")
            with self.assertRaises(SystemExit) as caught:
                ss.copy_world(out, Path(out) / "g2", scenes)
            self.assertIn("is not the save", str(caught.exception))
            other = ss.parse_scenes(scene(seed=1))
            with self.assertRaises(SystemExit) as caught:
                ss.copy_world(out, Path(out) / "g3", other)
            self.assertIn("another scene definition", str(caught.exception))

    def test_a_failed_generation_keeps_the_published_world(self):
        scenes = ss.parse_scenes(scene())

        class NoGame:
            def __init__(self, *args):
                self.proc = None

            def __enter__(self):
                raise RuntimeError("Sunrise did not open its control socket")

            def __exit__(self, *_):
                return False
        with tempfile.TemporaryDirectory() as out, contextlib.redirect_stdout(io.StringIO()):
            bins = self.build(out)
            ss.publish_world(out, self.save(Path(out) / "made"), scenes, bins)
            before = (Path(out) / "world" / "generation.json").read_text()
            resources = Path(out) / "res"
            resources.mkdir()
            (resources / "sunrise-resources.json").write_text(json.dumps({"data_version": 5023}))
            args = argparse.Namespace(out=out, replace=True, resources=str(resources), display="current")
            with mock.patch.object(ss, "Game", NoGame), self.assertRaises(SystemExit) as caught:
                ss.run_world(args, scenes, bins)
            self.assertIn("world not published", str(caught.exception))
            self.assertEqual((Path(out) / "world" / "generation.json").read_text(), before)
            self.assertEqual(ss.copy_world(out, Path(out) / "g", scenes)["save_sha256"],
                             json.loads(before)["save_sha256"])

    def test_the_options_version_comes_from_the_prepared_resources_alone(self):
        scenes = ss.parse_scenes(scene(options={"renderDistance": "8"}))
        with tempfile.TemporaryDirectory() as tmp, \
                mock.patch("tools.lab.version_info", side_effect=AssertionError("the client jar was opened")):
            resources = Path(tmp) / "res"
            resources.mkdir()
            (resources / "sunrise-resources.json").write_text(json.dumps({"data_version": 5023, "minecraft": "26.3"}))
            ss.prepare_game_dir(scenes, Path(tmp) / "game", resources)
            self.assertEqual((Path(tmp) / "game" / "options.txt").read_text(), "version:5023\nrenderDistance:8\n")
            (resources / "sunrise-resources.json").write_text(json.dumps({"minecraft": "26.3"}))
            with self.assertRaises(SystemExit):
                ss.prepare_game_dir(scenes, Path(tmp) / "game2", resources)


class ExitStatusTests(unittest.TestCase):
    def run_main(self, argv):
        with contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(io.StringIO()) as printed:
            return ss.main(argv), printed.getvalue()

    def test_setup_and_usage_failures_are_not_differences(self):
        with tempfile.TemporaryDirectory() as out:
            scene_file = Path(out) / "scenes.json"
            scene_file.write_text(json.dumps(scene()))
            status, printed = self.run_main(["compare", "--scenes", str(scene_file)])  # no --out
            self.assertEqual(status, rp.SETUP_FAILED)
            self.assertIn("--out", printed)
            status, printed = self.run_main(["run", "--scenes", str(scene_file), "--out", out, "--base-bin-dir", "b",
                                             "--head-bin-dir", "h", "--pack", str(Path(out) / "Missing.zip")])
            self.assertEqual(status, rp.SETUP_FAILED)
            self.assertIn("Missing.zip not found", printed)
            status, printed = self.run_main(["capture", "--scenes", str(scene_file), "--out", out, "--side", "base",
                                             "--bin-dir", "b", "--resources", out])
            self.assertEqual((status, printed.strip()), (rp.SETUP_FAILED, "no --pack given"))
            (Path(out) / "report").mkdir()  # a folder this tool didn't make
            status, printed = self.run_main(["compare", "--scenes", str(scene_file), "--out", out])
            self.assertEqual(status, rp.SETUP_FAILED)
            self.assertIn("was not made by this tool", printed)

    def test_a_valid_difference_is_exit_one(self):
        with tempfile.TemporaryDirectory() as out:
            scene_file = Path(out) / "scenes.json"
            scene_file.write_text(json.dumps(scene()))
            maker = ReportTests()
            changed = bytearray(maker.pixels)
            changed[0] ^= 1
            maker.both(out, head={"a": bytes(changed), "b": maker.pixels})
            self.assertEqual(self.run_main(["compare", "--scenes", str(scene_file), "--out", out, "--jobs", "1"])[0],
                             rp.DIFFERENT)


class ProcessTests(unittest.TestCase):
    def test_each_process_is_named_with_what_it_was_doing(self):
        self.assertEqual(ss.describe_processes({"gamescope": {"pid": 10, "status": None},
                                                "sunrise-client": {"pid": 11, "state": "gone"}}),
                         "gamescope (pid 10): running; sunrise-client (pid 11): gone")
        self.assertEqual(ss.describe_processes({"gamescope": {"pid": 10, "status": 0},
                                                "sunrise-client": {"pid": None}}),
                         "gamescope (pid 10): exited, status 0; sunrise-client: pid unknown")
        self.assertEqual(ss.describe_processes({"sunrise-client": {"pid": 12, "status": -9}}),
                         "sunrise-client (pid 12): exited, status -9")

    def test_the_game_is_found_under_its_wrapper_and_its_state_read(self):
        with tempfile.TemporaryDirectory() as tmp:
            proc = Path(tmp) / "proc"
            client = Path(tmp) / "bin" / "sunrise-client"
            client.parent.mkdir()
            client.write_bytes(b"")
            other = Path(tmp) / "bin" / "gamescope"
            other.write_bytes(b"")
            for pid, exe, children, state in ((10, other, "11 12", "S"), (11, other, "", "S"),
                                              (12, client, "", "S"), (13, client, "", "Z")):
                (proc / str(pid) / "task" / str(pid)).mkdir(parents=True)
                (proc / str(pid) / "task" / str(pid) / "children").write_text(children)
                (proc / str(pid) / "exe").symlink_to(exe)
                (proc / str(pid) / "stat").write_text(f"{pid} (name with ) in it) {state} 1 2 3")
            self.assertEqual(ss.find_descendant(10, client, proc), 12)
            self.assertIsNone(ss.find_descendant(11, client, proc))
            self.assertEqual([ss.pid_state(pid, proc) for pid in (12, 13, 99)], ["running", "gone", "gone"])

if __name__ == "__main__":
    unittest.main()
