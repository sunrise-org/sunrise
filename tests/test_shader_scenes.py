"""tools/shader_scenes.py and shader_scenes_report.py: the scene file, the commands each view sends, the camera
check, the PNG reader, the grid comparison, the pairing of both sides' runs and the report (no game is started)."""

import contextlib
import copy
import io
import json
from pathlib import Path
import struct
import tempfile
import unittest
from unittest import mock
import zlib

from tools import perf_bench
from tools import shader_scenes as ss
from tools import shader_scenes_java as sj
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


class SceneFileTests(unittest.TestCase):
    def test_the_committed_scene_file_parses(self):
        scenes = ss.load_scenes()
        names = [v["name"] for v in scenes["views"]]
        self.assertEqual(len(names), len(set(names)))
        self.assertEqual(scenes["size"], [1280, 720])

    def test_defaults_are_filled_and_setup_loses_its_slash(self):
        scenes = ss.parse_scenes(scene())
        self.assertEqual(scenes["settle_seconds"], 5.0)
        self.assertEqual(scenes["load_timeout_seconds"], 90.0)
        b = scenes["views"][1]
        self.assertEqual((b["yaw"], b["pitch"], b["time"], b["weather"], b["setup"]), (0.0, 0.0, None, None, []))
        self.assertEqual(scenes["views"][0]["setup"], ["setblock 1 64 1 minecraft:torch"])

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
        self.refused(scene(extra=1), "'extra'")
        self.refused(scene(format=2), "format")
        self.refused(scene(seed=True), "seed")
        self.refused(scene(seed=2**63), "seed")
        self.refused(scene(size=[1280]), "size")
        self.refused(scene(views=[]), "views")
        self.refused(scene(views=[views[0], views[0]]), "used twice")
        self.refused(view(name="../up"), "name")
        self.refused(view(name="A"), "name")
        self.refused(view(dimension="minecraft:overworld"), "dimension")
        self.refused(view(pos=[0, float("nan"), 0]), "pos")
        self.refused(view(pos=[0, 64]), "pos")
        self.refused(view(pitch=90.5), "pitch")
        self.refused(view(yaw="90"), "yaw")
        self.refused(view(weather="snow"), "weather")
        self.refused(view(time=-1), "time")
        self.refused(view(setup=["say a\nsay b"]), "setup")
        self.refused(view(look=1), "'look'")
        self.refused(scene(options={"fov": 70}), "options")
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


class CommandTests(unittest.TestCase):
    def test_view_commands_teleport_with_a_decimal_point(self):
        a, b = ss.parse_scenes(scene(views=[
            {"name": "a", "dimension": "overworld", "pos": [1.5, 64, -2], "yaw": 90, "pitch": 10, "time": 6000,
             "weather": "rain", "setup": ["setblock 1 64 1 minecraft:torch"]},
            {"name": "b", "dimension": "the_nether", "pos": [0.0, 70.0, 3.0]}]))["views"]
        # tp centres a bare integer x or z on its block: 64 and -2 must go out as 64.0 and -2.0.
        self.assertEqual(ss.view_commands(a), [
            "execute in minecraft:overworld run tp @a 1.5 64.0 -2.0 90.0 10.0", "time set 6000", "weather rain"])
        self.assertEqual(ss.view_commands(b), ["execute in minecraft:the_nether run tp @a 0.0 70.0 3.0 0.0 0.0"])
        self.assertEqual(ss.setup_commands(a), ["execute in minecraft:overworld run setblock 1 64 1 minecraft:torch"])

    def test_start_commands_and_options(self):
        scenes = ss.parse_scenes(scene(options={"gamma": "0.5", "fov": "0.0"},
                                       gamerules={"advance_time": "false", "random_tick_speed": "0"}))
        self.assertEqual(ss.start_commands(scenes), [
            "gamerule advance_time false", "gamerule random_tick_speed 0",
            "gamerule advance_time", "gamerule random_tick_speed", "gamemode spectator @a", "tick freeze"])
        self.assertEqual(ss.options_text(scenes, 5023, {"narrator": "0"}),
                         "version:5023\ngamma:0.5\nfov:0.0\nnarrator:0\n")


class SettingsTests(unittest.TestCase):
    def test_the_pinned_drawing_settings_and_their_read_back(self):
        scenes = ss.parse_scenes(scene(options={"renderDistance": "8", "simulationDistance": "8", "fov": "0.0",
                                                "gamma": "0.5"}))
        expected = ss.settings_expected(scenes)
        self.assertEqual(expected, {"render_distance": 8, "simulation_distance": 8, "fov": 70})
        self.assertEqual(ss.settings_expected(ss.parse_scenes(scene(options={"fov": "1.0"}))), {"fov": 110})
        self.assertEqual(ss.settings_problems(expected, {"render_distance": 8, "simulation_distance": 8, "fov": 70}),
                         [])
        # What a first-start preset did to Java: distances 16 and 12 against the scene's 8 and 8.
        self.assertEqual(ss.settings_problems(expected, {"render_distance": 16, "simulation_distance": 12, "fov": 70}),
                         ["render_distance 16, the scene pins 8", "simulation_distance 12, the scene pins 8"])
        self.assertEqual(ss.settings_problems(expected, None), ["no drawing settings were read back"])
        self.assertEqual(ss.settings_problems({}, None), [])

    def test_a_view_with_other_settings_has_a_problem(self):
        scenes = ss.parse_scenes(scene(options={"renderDistance": "8"}))
        view = scenes["views"][1]
        camera = {"dimension": "minecraft:the_nether", "pos": [0.0, 70.0, 3.0], "yaw": 0.0, "pitch": 0.0,
                  "screen": "playing", "spectator": True, "frozen": True, "settings": {"render_distance": 16}}
        views = {}
        ss.record_view(views, view, camera, [], True, None, ss.settings_expected(scenes))
        self.assertEqual(views["b"]["problems"], ["render_distance 16, the scene pins 8"])

    def test_sunrise_settings_read_from_status_and_options(self):
        options = {"options": {"fov": {"type": "int", "value": 70}, "simulationDistance": {"value": 8}}}
        self.assertEqual(ss.sunrise_settings({"render_distance": 8}, options),
                         {"render_distance": 8, "simulation_distance": 8, "fov": 70})


class SettleTests(unittest.TestCase):
    def test_a_weather_change_settles_longer_and_a_view_may_set_its_own(self):
        views = [{"name": "a", "dimension": "overworld", "pos": [0, 64, 0], "weather": "clear"},
                 {"name": "b", "dimension": "overworld", "pos": [0, 64, 0], "weather": "rain"},
                 {"name": "c", "dimension": "overworld", "pos": [0, 64, 0], "weather": "rain"},
                 {"name": "d", "dimension": "the_nether", "pos": [0, 64, 0]},
                 {"name": "e", "dimension": "overworld", "pos": [0, 64, 0], "weather": "clear"},
                 {"name": "f", "dimension": "overworld", "pos": [9, 64, 0], "settle_seconds": 4}]
        scenes = ss.parse_scenes(scene(views=views, settle_seconds=1, weather_settle_seconds=7,
                                       move_settle_seconds=5))
        # (thaw, settle): a: first view, moved; b: rain starts here: thawed for it; c: still rain; d: another
        # dimension, moved; e: moved back and the rain stops; f: its own settle.
        self.assertEqual(ss.settle_plan(scenes),
                         [(0.0, 5.0), (7.0, 1.0), (0.0, 1.0), (0.0, 5.0), (7.0, 5.0), (0.0, 4.0)])
        both = ss.parse_scenes(scene(settle_seconds=3))
        self.assertEqual((both["weather_settle_seconds"], both["move_settle_seconds"]), (3.0, 3.0))

    def test_same_place(self):
        a, b = ss.parse_scenes(scene())["views"]
        self.assertTrue(ss.same_place(a, dict(a, time=18000, weather="clear")))
        self.assertFalse(ss.same_place(a, b))
        self.assertFalse(ss.same_place(a, dict(a, pos=[1.5, 65.0, -2.5])))
        self.assertFalse(ss.same_place(a, None))


class StartTests(unittest.TestCase):
    def test_start_problems_compare_the_queried_rules(self):
        scenes = ss.parse_scenes(scene(gamerules={"advance_time": "false", "random_tick_speed": "3",
                                                  "custom": "on"}))
        good = {"gamerule advance_time": 0, "gamerule random_tick_speed": 3, "gamerule custom": 7}
        self.assertEqual(ss.start_problems(scenes, good), [])
        self.assertEqual(ss.start_problems(scenes, {**good, "gamerule advance_time": 1, "gamerule custom": None}), [
            "gamerule advance_time is 1, the scene asks for false", "gamerule custom is None, the scene asks for on"])
        self.assertEqual(ss.start_problems(scenes, {**good, "gamerule random_tick_speed": 0}),
                         ["gamerule random_tick_speed is 0, the scene asks for 3"])


class FreezeTests(unittest.TestCase):
    def test_tick_query_replies(self):
        frozen = {"ok": True, "success": True, "chat": ["Set SceneBot's game mode", "The game is frozen"]}
        running = {"ok": True, "success": True, "chat": ["Target tick rate: 20.0 per second.",
                                                          "The game is running normally"]}
        self.assertTrue(ss.sunrise_frozen(frozen))
        self.assertFalse(ss.sunrise_frozen(running))
        self.assertIsNone(ss.sunrise_frozen({"ok": True, "success": False, "chat": ["Unknown command"]}))
        self.assertIsNone(ss.sunrise_frozen({"error": "no world"}))
        self.assertIsNone(ss.sunrise_frozen({"ok": True, "success": True, "chat": ["something else"]}))

    def test_a_failed_freeze_is_a_start_problem(self):
        scenes = ss.parse_scenes(scene())
        self.assertEqual(ss.start_problems(scenes, {"tick freeze": None}), ["tick freeze failed"])
        self.assertEqual(ss.start_problems(scenes, {"tick freeze": 1}), [])


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
        # A game may report the yaw wrapped into another turn.
        self.assertEqual(ss.camera_problems(self.view, self.camera(yaw=-270.0)), [])
        self.assertEqual(ss.camera_problems(self.view, self.camera(yaw=450.0)), [])

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
        self.assertEqual(ss.camera_problems(self.view, self.camera(boss_bars=1)),
                         ["1 boss bar(s) shown outside the End"])
        self.assertEqual(ss.camera_problems(self.view, self.camera(boss_bars=0)), [])
        end = dict(self.view, dimension="the_end")
        self.assertEqual(ss.camera_problems(end, self.camera(dimension="minecraft:the_end", boss_bars=1)), [])
        self.assertEqual(ss.camera_problems(self.view, self.camera(frozen=False)),
                         ["the game is not frozen (tick freeze): frozen read back as False"])
        self.assertEqual(ss.camera_problems(self.view, self.camera(frozen=None)),
                         ["the game is not frozen (tick freeze): frozen read back as None"])
        # The clock counts days on: 30000 is day two's 6000.
        self.assertEqual(ss.camera_problems(self.view, self.camera(day_time=30000)), [])
        self.assertEqual(ss.camera_problems(self.view, self.camera(day_time=6001)), ["time 6001, view asks for 6000"])
        self.assertEqual(ss.camera_problems(self.view, self.camera(day_time=None)), ["time None, view asks for 6000"])
        self.assertEqual(ss.camera_problems(self.view, self.camera(raining=False)),
                         ["weather clear, view asks for rain"])
        self.assertEqual(ss.camera_problems(self.view, self.camera(thundering=True)),
                         ["weather thunder, view asks for rain"])

    def test_pack_keys(self):
        self.assertEqual(ss.pack_key("none"), "none")
        self.assertEqual(ss.pack_key("/p/BSL_v10.1.8.zip"), "bsl_v10_1_8")
        self.assertEqual(ss.pack_key("/p/Solas Shader V3.9.zip"), "solas_shader_v3_9")
        for bad in ("/p/none.zip", "/p/___.zip"):
            with self.assertRaises(ss.SceneError):
                ss.pack_key(bad)


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


class PngTests(unittest.TestCase):
    def read(self, data, size):
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "p.png"
            path.write_bytes(data)
            return rp.read_png(path, size)

    def test_rgba_with_every_filter_type(self):
        # Row 0 (None) is 10,20,30,255 | 40,50,60,255; each later row's raw bytes, filtered, give these pixels:
        # row 1 Sub: 11,22,33 | 51,72,93; row 2 Up: 12,24,36 | 53,76,99; row 3 Average; row 4 Paeth.
        rows = [bytes([10, 20, 30, 255, 40, 50, 60, 255]),
                bytes([11, 22, 33, 255, 40, 50, 60, 0]),
                bytes([1, 2, 3, 0, 2, 4, 6, 0]),
                bytes([6, 12, 18, 0, 22, 32, 42, 0]),
                bytes([1, 1, 1, 0, 1, 1, 1, 0])]
        width, height, rgb = self.read(png(2, 5, rows, colour=6, filters=[0, 1, 2, 3, 4]), [2, 5])
        self.assertEqual((width, height), (2, 5))
        pixels = [tuple(rgb[i:i + 3]) for i in range(0, len(rgb), 3)]
        self.assertEqual(pixels, [
            (10, 20, 30), (40, 50, 60),
            (11, 22, 33), (51, 72, 93),
            (12, 24, 36), (53, 76, 99),
            # Average: (left + up) // 2 + raw; the first pixel's left is 0.
            (12, 24, 36), (54, 82, 109),
            # Paeth: the first pixel predicts from up (12,24,36); the second from left/up/up-left.
            (13, 25, 37), (55, 83, 110)])

    def test_rgb_round_trip(self):
        rgb = bytes(range(24))
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "p.png"
            rp.write_png(path, 4, 2, rgb)
            self.assertEqual(rp.read_png(path, [4, 2]), (4, 2, rgb))

    def test_refusals(self):
        with self.assertRaisesRegex(rp.PictureError, "not a PNG"):
            self.read(b"GIF89a", [1, 1])
        with self.assertRaisesRegex(rp.PictureError, "2x1, the scene asks for 4x2"):
            self.read(png(2, 1, [bytes(6)]), [4, 2])
        with self.assertRaisesRegex(rp.PictureError, "colour type 0"):
            self.read(png(2, 1, [bytes(2)], colour=0), [2, 1])
        with self.assertRaisesRegex(rp.PictureError, "filter type 7"):
            self.read(png(1, 1, [bytes(3)], filters=[7]), [1, 1])


class GridTests(unittest.TestCase):
    def test_downscale_averages_each_cell(self):
        # 4x2 to 2x1: the left cell holds pixels x 0-1, the right one x 2-3, of both rows.
        rgb = bytes([0, 0, 0, 10, 20, 30, 100, 0, 0, 100, 0, 0,
                     20, 40, 60, 30, 60, 90, 0, 200, 0, 0, 0, 200])
        self.assertEqual(rp.downscale(4, 2, rgb, (2, 1)), [(15.0, 30.0, 45.0), (50.0, 50.0, 50.0)])

    def test_uneven_sizes_put_every_pixel_in_one_cell(self):
        # 3 columns into 2 cells (x * cols // width): x 0 and 1 go left, x 2 right.
        rgb = bytes([30, 0, 0, 60, 0, 0, 90, 0, 0])
        self.assertEqual(rp.downscale(3, 1, rgb, (2, 1)), [(45.0, 0.0, 0.0), (90.0, 0.0, 0.0)])

    def test_the_grid_never_has_more_cells_than_pixels(self):
        self.assertEqual(rp.grid_for([1280, 720]), (64, 36))
        self.assertEqual(rp.grid_for([4, 2]), (4, 2))
        self.assertEqual(rp.grid_for([100, 20]), (64, 20))

    def test_compare_cells(self):
        numbers, per_cell = rp.compare_cells([(10, 20, 30), (0, 0, 0)], [(13, 26, 39), (255, 0, 0)])
        self.assertEqual(per_cell, [6.0, 85.0])
        self.assertEqual(numbers, {"diff": 45.5, "worst_cell": 85.0})

    def test_frame_flags(self):
        self.assertEqual(rp.frame_flags([(0, 0, 0), (10, 10, 10)])["flags"], ["black"])
        self.assertEqual(rp.frame_flags([(250, 250, 250)] * 2)["flags"], ["washed_out", "flat"])
        self.assertEqual(rp.frame_flags([(100, 100, 100), (101, 101, 101)])["flags"], ["flat"])
        flags = rp.frame_flags([(0, 0, 0), (200, 200, 200)])
        self.assertEqual(flags, {"luma_mean": 100.0, "luma_sd": 100.0, "flags": []})


class ReportTests(unittest.TestCase):
    def make_run(self, out, side, key, views, pictures, **record):
        folder = Path(out) / side / key
        folder.mkdir(parents=True)
        (folder / rp.RUN_MARK).write_text("")
        identity = {"world_id": "w1", "scene_hash": rp.scene_hash(ss.parse_scenes(scene())), "pack_sha256": "p1"}
        (folder / "run.json").write_text(json.dumps({"pack": key, "views": views, **identity, **record}))
        for name, rgb in pictures.items():
            rp.write_png(folder / f"{name}.png", 4, 2, rgb)

    def test_compare_pairs_both_sides_and_reports_gaps(self):
        scenes = ss.parse_scenes(scene())
        grey = bytes([100] * 24)
        lighter = bytes([110] * 24)
        with tempfile.TemporaryDirectory() as out:
            self.make_run(out, "sunrise", "pack", {"a": {"problems": []}, "b": {"problems": ["command failed: x"]}},
                          {"a": grey, "b": grey})
            self.make_run(out, "java", "pack", {"a": {"problems": []}}, {"a": lighter},
                          notes=["stopped: the game exited"])
            # A folder without the run mark is not a run: never paired.
            (Path(out) / "java" / "stray").mkdir()
            report = rp.compare(out, scenes, ["pack/a"])
            self.assertEqual(list(report["packs"]), ["pack"])
            pack = report["packs"]["pack"]
            self.assertEqual((pack["mean_diff"], pack["compared"], pack["views"]), (10.0, 1, 2))
            self.assertEqual(pack["notes"], ["java: stopped: the game exited"])
            a, b = pack["rows"]
            self.assertEqual((a["diff"], a["worst_cell"]), (10.0, 10.0))
            self.assertTrue((Path(out) / "report" / a["heat"]).exists())
            self.assertTrue((Path(out) / "report" / a["pair"]).exists())
            self.assertNotIn("diff", b)
            self.assertTrue(b["java"]["missing"])
            self.assertEqual(b["sunrise"]["problems"], ["command failed: x"])
            self.assertEqual(rp._notes(b), ["sunrise: flat", "sunrise: command failed: x", "java: no picture"])
            self.assertIn("| pack | 10.0 | 1 / 2 |", (Path(out) / "report" / "report.md").read_text())
            self.assertEqual(rp.summary(report), 0)

    def test_a_side_that_never_ran_is_named_and_nothing_compared_fails(self):
        scenes = ss.parse_scenes(scene())
        with tempfile.TemporaryDirectory() as out:
            self.make_run(out, "sunrise", "pack", {}, {"a": bytes(24)})
            report = rp.compare(out, scenes)
            row = report["packs"]["pack"]["rows"][0]
            self.assertEqual(row["java"]["problems"], ["side not run"])
            self.assertEqual(rp.summary(report), 1)

    def test_cli_compare(self):
        with tempfile.TemporaryDirectory() as out:
            scene_file = Path(out) / "scenes.json"
            scene_file.write_text(json.dumps(scene()))
            for side, value in (("sunrise", 0), ("java", 30)):
                self.make_run(out, side, "none", {}, {"a": bytes([value] * 24), "b": bytes([value] * 24)})
            self.assertEqual(ss.main(["compare", "--scenes", str(scene_file), "--out", out]), 0)
            report = json.loads((Path(out) / "report" / "report.json").read_text())
            self.assertEqual(report["noise_floor"], 30.0)

    def test_cli_check_prints_each_views_commands_and_refuses_a_bad_file(self):
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
            with self.assertRaises(SystemExit) as caught:
                ss.main(["check", "--scenes", str(scene_file)])
            self.assertIn("format", str(caught.exception))

    def test_run_folders_are_only_replaced_when_this_tool_made_them(self):
        with tempfile.TemporaryDirectory() as out:
            folder = ss.run_folder(out, "sunrise", "none", False)
            (folder / "a.png").write_bytes(b"x")
            with self.assertRaises(SystemExit):
                ss.run_folder(out, "sunrise", "none", False)
            again = ss.run_folder(out, "sunrise", "none", True)
            self.assertFalse((again / "a.png").exists())
            (again / rp.RUN_MARK).unlink()
            with self.assertRaises(SystemExit):
                ss.run_folder(out, "sunrise", "none", True)



class FakeClock:
    """time.time and time.sleep for code that polls: sleeping only moves the clock."""

    def __init__(self):
        self.t = 1000.0

    def time(self):
        return self.t

    def sleep(self, seconds):
        self.t += seconds


class PathTests(unittest.TestCase):
    def test_the_documented_relative_paths_become_absolute(self):
        with tempfile.TemporaryDirectory() as tmp:
            args = ss.parser().parse_args(["run", "--out", "OUT", "--pack", "PACKS/BSL_v10.1.8.zip", "--pack", "none",
                                           "--mods-from", "MODS", "--java-home", "jdk", "--resources", "res"])
            ss.absolute_paths(args, tmp)
            root = Path(tmp).resolve()
            self.assertEqual(Path(args.out), root / "OUT")
            self.assertEqual(args.pack, [str(root / "PACKS/BSL_v10.1.8.zip"), "none"])
            self.assertEqual((Path(args.mods_from), Path(args.java_home), Path(args.resources)),
                             (root / "MODS", root / "jdk", root / "res"))
            self.assertTrue(Path(args.scenes).is_absolute())

    def test_iris_runs_only_the_checked_in_scene_file(self):
        args = ss.parser().parse_args(["java", "--out", "o", "--scenes", "other.json"])
        ss.absolute_paths(args)
        ss.iris_scope(args, ["none"])
        with self.assertRaises(SystemExit) as caught:
            ss.iris_scope(args, ["none", "/p/BSL.zip"])
        self.assertIn("D-11", str(caught.exception))
        args.scenes = str(ss.SCENES)
        ss.iris_scope(args, ["/p/BSL.zip"])


class ReadinessTests(unittest.TestCase):
    view = ss.parse_scenes(scene())["views"][0]

    def run_statuses(self, statuses, limit):
        """sunrise_wait_loaded over a scripted game: each status call returns the status for the clock's time."""
        clock = FakeClock()

        def game(*words, timeout=600):
            elapsed = clock.time() - 1000.0
            return next(s for until, s in statuses if elapsed < until)
        with mock.patch.object(perf_bench, "time", clock):
            return ss.sunrise_wait_loaded(game, self.view, limit), clock.time() - 1000.0

    @staticmethod
    def status(chunks, pending=0, dimension="minecraft:overworld", pos=(1.5, 64.0, -2.5), working=0):
        return {"dimension": dimension, "player": {"position": list(pos)}, "pending_chunks": pending,
                "screen": "playing", "chunks": chunks, "generated_chunks": chunks, "render_distance": 2,
                "meshed_sections": 0, "streaming": {"meshing": working}}

    def test_an_empty_queue_right_after_the_teleport_is_not_ready(self):
        # view_chunks(2) = 10. Nothing queued and steady counts for 3 s, but only 4 chunks: not loaded yet.
        statuses = [(3.0, self.status(4)), (4.0, self.status(7, pending=3)), (float("inf"), self.status(12))]
        ready, at = self.run_statuses(statuses, 30)
        self.assertTrue(ready)
        self.assertGreaterEqual(at, 5.0)  # one steady second after the counts reached full at 4 s

    def test_at_the_same_place_only_steady_chunks_are_waited_for(self):
        # The camera didn't move: no teleport guard, so 4 steady chunks are enough after one second.
        clock = FakeClock()
        game = lambda *words, timeout=600: self.status(4)
        with mock.patch.object(perf_bench, "time", clock):
            self.assertTrue(ss.sunrise_wait_loaded(game, self.view, 30, moved=False))
            self.assertLess(clock.time() - 1000.0, 2.0)
            self.assertFalse(ss.sunrise_wait_loaded(game, self.view, 30))  # moved: 4 of 10 never loads

    def test_another_dimension_or_position_never_counts(self):
        statuses = [(float("inf"), self.status(12, dimension="minecraft:the_nether"))]
        self.assertEqual(self.run_statuses(statuses, 10)[0], False)
        statuses = [(float("inf"), self.status(12, pos=(1.5, 64.0, 0.0)))]
        self.assertEqual(self.run_statuses(statuses, 10)[0], False)

    def test_meshing_work_holds_it(self):
        statuses = [(8.0, self.status(12, working=5)), (float("inf"), self.status(12))]
        ready, at = self.run_statuses(statuses, 30)
        self.assertTrue(ready)
        self.assertGreaterEqual(at, 8.5)


class PlaceTests(unittest.TestCase):
    view = ss.parse_scenes(scene())["views"][0]

    def test_a_view_that_never_loaded_is_not_a_loaded_place(self):
        # Only 4 of the 10 chunks the view circle needs, forever: the first view times out, and the next view at the
        # same place must not take the light wait (which 4 steady chunks would pass).
        clock = FakeClock()
        game = lambda *words, timeout=600: ReadinessTests.status(4)
        with mock.patch.object(perf_bench, "time", clock):
            ready, previous = ss.place_view(game, self.view, None, 10)
            self.assertEqual((ready, previous), (False, None))
            ready, previous = ss.place_view(game, dict(self.view, time=18000), previous, 10)
            self.assertEqual((ready, previous), (False, None))
            full = lambda *words, timeout=600: ReadinessTests.status(12)
            ready, previous = ss.place_view(full, self.view, None, 30)
            self.assertEqual((ready, previous), (True, self.view))


class ThawTests(unittest.TestCase):
    def game(self, unfreeze_ok=True, frozen_until=0.0, raining_from=0.0):
        clock = FakeClock()
        calls = []

        def game(*words, check=True, timeout=600):
            t = clock.time() - 1000.0
            calls.append((t, words))
            if words[:2] == ("command", "tick unfreeze"):
                return {"ok": True, "success": unfreeze_ok, "chat": []}
            if words[:2] == ("command", "tick query"):
                line = "The game is frozen" if t < frozen_until else "The game is running normally"
                return {"ok": True, "success": True, "chat": [line]}
            return {"weather": {"raining": t >= raining_from, "thundering": False}}
        return game, clock, calls

    def test_a_failed_thaw(self):
        game, clock, _ = self.game(unfreeze_ok=False)
        self.assertEqual(ss.sunrise_thaw(game, "rain", 6, 30, clock.time, clock.sleep), ["tick unfreeze failed"])
        game, clock, _ = self.game(frozen_until=float("inf"))
        self.assertEqual(ss.sunrise_thaw(game, "rain", 6, 30, clock.time, clock.sleep),
                         ["the world never thawed after tick unfreeze"])

    def test_a_delayed_thaw_starts_the_time_when_it_takes_effect(self):
        game, clock, calls = self.game(frozen_until=3.0)
        self.assertEqual(ss.sunrise_thaw(game, "rain", 6, 30, clock.time, clock.sleep), [])
        # The weather is read after the thaw took effect (3 s) plus the 6 s, not 6 s after the command.
        first_state = next(t for t, words in calls if words == ("state",))
        self.assertGreaterEqual(first_state, 9.0)

    def test_weather_that_never_comes(self):
        game, clock, _ = self.game(raining_from=float("inf"))
        self.assertEqual(ss.sunrise_thaw(game, "rain", 6, 30, clock.time, clock.sleep),
                         ["the weather never reached rain while thawed: {'raining': False, 'thundering': False}"])

    def test_java_thaw_events(self):
        self.assertEqual(sj.thaw_problems({"thaw": 6}, None), ["the thaw never finished"])
        self.assertEqual(sj.thaw_problems({"thaw": 6}, {"thawed": False, "rain_level": 1.0, "thunder_level": 0.0}),
                         ["the world never thawed after tick unfreeze"])
        self.assertEqual(sj.thaw_problems({"thaw": 6}, {"thawed": True, "reached": False, "rain_level": 0.4,
                                                        "thunder_level": 0.0}),
                         ["the weather never reached its levels while thawed: rain 0.4, thunder 0.0"])
        self.assertEqual(sj.thaw_problems({"thaw": 6}, {"thawed": True, "reached": True}), [])
        self.assertEqual(sj.thaw_problems({"thaw": 0}, None), [])


class CaptureTests(unittest.TestCase):
    def test_capture_problems(self):
        self.assertEqual(ss.capture_problems({"ok": True, "width": 1280, "height": 720}, [1280, 720]), [])
        self.assertEqual(ss.capture_problems({"ok": True, "width": 936, "height": 1018}, [1280, 720]),
                         ["the picture is 936x1018, the scene asks for 1280x720"])
        self.assertEqual(ss.capture_problems({"error": "no renderer"}, [1280, 720]), ["screenshot: no renderer"])

    def test_packs_sharing_a_folder_name_are_refused(self):
        with self.assertRaises(SystemExit) as caught:
            ss.main(["sunrise", "--out", "o", "--pack", "none", "--pack", "none"])
        self.assertIn("none", str(caught.exception))
        with self.assertRaises(SystemExit):
            ss.main(["sunrise", "--out", "o", "--pack", "/a/BSL.zip", "--pack", "/b/bsl.zip"])

    def test_pack_digest(self):
        with tempfile.TemporaryDirectory() as tmp:
            a, b = Path(tmp) / "a", Path(tmp) / "b.zip"
            (a / "shaders").mkdir(parents=True)
            (a / "shaders" / "x.fsh").write_bytes(b"1")
            b.write_bytes(b"zip")
            self.assertEqual(ss.pack_digest("none"), "none")
            # sha256 of b"zip"
            self.assertEqual(ss.pack_digest(b), "4a70fe9aa6436e02c2dea340fbd1e352e4ef2d8ce6ca52ad25d4b95471fc8bf2")
            before = ss.pack_digest(a)
            (a / "shaders" / "x.fsh").write_bytes(b"2")
            self.assertNotEqual(ss.pack_digest(a), before)

    def test_run_fails_when_a_requested_pack_has_no_scored_view(self):
        report = {"packs": {"none": {"compared": 16}, "bsl": {"compared": 0}}}
        self.assertEqual(ss.unscored_packs(report, ["none", "bsl", "photon"]), ["bsl", "photon"])
        self.assertEqual(ss.unscored_packs(report, ["none"]), [])

    def test_world_returns_after_publishing(self):
        with tempfile.TemporaryDirectory() as out:
            with mock.patch.object(sj, "run_world") as world:
                self.assertEqual(ss.main(["world", "--out", out]), 0)
            world.assert_called_once()


class PackLoadTests(unittest.TestCase):
    def loads(self, timeline):
        """PackLoads over a log whose events appear at the given clock times."""
        clock = FakeClock()
        read = lambda: [e for at, e in timeline if clock.time() - 1000.0 >= at]
        return ss.PackLoads(read, now=clock.time, sleep=clock.sleep), clock

    def test_each_transition_gets_its_own_load_and_a_failed_revisit_is_named(self):
        loads, _ = self.loads([(1, ("programs", 9)), (3, ("programs", 8)), (5, ("off", "no programs"))])
        self.assertEqual(loads.transition(10), ("programs", 9))
        self.assertEqual(loads.transition(10), ("programs", 8))
        # Back in the first dimension: this visit's load, not the first visit's.
        self.assertEqual(loads.transition(10), ("off", "no programs"))
        self.assertEqual(ss.pack_problem(("off", "no programs")), "the pack was turned off: no programs")
        self.assertIsNone(ss.pack_problem(("programs", 9)))

    def test_a_late_load_makes_every_later_state_unknown(self):
        loads, _ = self.loads([(1, ("programs", 9)), (50, ("programs", 8)), (52, ("programs", 9))])
        self.assertEqual(loads.transition(10), ("programs", 9))
        self.assertEqual(loads.transition(10)[0], "missing")
        # The late load arrives during the next transition: it can't be told from this one's.
        self.assertEqual(loads.transition(100)[0], "unknown")

    def test_an_extra_load_makes_later_states_unknown(self):
        loads, _ = self.loads([(0, ("programs", 9)), (1, ("programs", 8)), (1, ("programs", 7))])
        self.assertEqual(loads.transition(10), ("programs", 9))
        # Two loads for one transition: neither is taken as this transition's.
        self.assertEqual(loads.transition(10)[0], "unknown")
        self.assertEqual(loads.transition(10)[0], "unknown")


class JavaTests(unittest.TestCase):
    def test_iris_must_log_the_requested_pack(self):
        log = "[00:50:00] [Render thread/INFO]: Using shaderpack: BSL_v10.1.8.zip\n"
        self.assertTrue(sj.iris_selected(log, "BSL_v10.1.8.zip"))
        self.assertFalse(sj.iris_selected(log, "BSL_v10.1.8"))
        self.assertFalse(sj.iris_selected("[00:50:00] [Render thread/INFO]: Using shaderpack: (off)\n", "BSL.zip"))
        self.assertFalse(sj.iris_selected("", "BSL.zip"))

    def test_world_failures(self):
        scenes = ss.parse_scenes(scene())
        good = {"notes": [], "start_problems": [], "views": {"a": {"problems": []}, "b": {"problems": []}},
                "finished": []}
        self.assertEqual(sj.world_failures(good, scenes), [])
        self.assertEqual(sj.world_failures(dict(good, notes=["the game exited with status 1"]), scenes),
                         ["the game exited with status 1"])
        self.assertEqual(sj.world_failures(dict(good, views={"a": {"problems": ["the view never finished loading"]}}),
                                           scenes), ["a: the view never finished loading"])
        self.assertEqual(sj.world_failures(dict(good, start_problems=["gamerule x is 1, the scene asks for 0"]),
                                           scenes), ["gamerule x is 1, the scene asks for 0"])
        self.assertEqual(sj.world_failures(dict(good, finished=["dimension minecraft:the_end, view asks for "
                                                                "minecraft:overworld"]), scenes),
                         ["return to a: dimension minecraft:the_end, view asks for minecraft:overworld"])
        missing = dict(good)
        del missing["finished"]
        self.assertEqual(sj.world_failures(missing, scenes),
                         ["the camera was never read back after the return to the first view"])


class WorldIdentityTests(unittest.TestCase):
    def test_a_world_made_for_another_scene_is_refused(self):
        scenes = ss.parse_scenes(scene())
        with tempfile.TemporaryDirectory() as out:
            (Path(out) / "world" / ss.WORLD).mkdir(parents=True)
            (Path(out) / "world" / ss.WORLD / "level.dat").write_bytes(b"x")
            (Path(out) / "world" / "generation.json").write_text(
                json.dumps({"world_id": "g1", "scene_hash": rp.scene_hash(scenes)}))
            self.assertEqual(ss.copy_world(out, Path(out) / "a", scenes),
                             {"world_id": "g1", "scene_hash": rp.scene_hash(scenes)})
            other = ss.parse_scenes(scene(seed=1))
            with self.assertRaises(SystemExit):
                ss.copy_world(out, Path(out) / "b", other)

    def test_scene_hash_ignores_key_order_and_sees_values(self):
        a = ss.parse_scenes(scene())
        b = ss.parse_scenes(json.loads(json.dumps(scene(), sort_keys=True)))
        self.assertEqual(rp.scene_hash(a), rp.scene_hash(b))
        self.assertNotEqual(rp.scene_hash(a), rp.scene_hash(ss.parse_scenes(scene(seed=1))))

    def test_coordinates_never_use_an_exponent(self):
        self.assertEqual(ss._coordinate(0.00001), "0.00001")
        self.assertEqual(ss._coordinate(-1e-07), "-0.0000001")
        self.assertEqual(ss._coordinate(3e7), "30000000.0")


class ScoringTests(ReportTests):
    def test_pairs_from_different_world_generations_are_not_scored(self):
        scenes = ss.parse_scenes(scene())
        with tempfile.TemporaryDirectory() as out:
            self.make_run(out, "sunrise", "none", {}, {"a": bytes([0] * 24), "b": bytes([0] * 24)}, world_id="w1")
            self.make_run(out, "java", "none", {}, {"a": bytes([30] * 24), "b": bytes([30] * 24)}, world_id="w2")
            report = rp.compare(out, scenes)
            pack = report["packs"]["none"]
            self.assertEqual((pack["compared"], pack["mean_diff"], report["noise_floor"]), (0, None, None))
            self.assertEqual(pack["rows"][0]["diff"], 30.0)  # still shown
            self.assertIn("not scored", rp._notes(pack["rows"][0]))
            self.assertTrue(any("different worlds" in n for n in rp._notes(pack["rows"][0])))
            self.assertEqual(rp.summary(report), 1)

    def test_a_failed_start_check_keeps_the_side_out_of_every_score(self):
        scenes = ss.parse_scenes(scene())
        with tempfile.TemporaryDirectory() as out:
            self.make_run(out, "sunrise", "none", {}, {"a": bytes([0] * 24), "b": bytes([0] * 24)})
            self.make_run(out, "java", "none", {}, {"a": bytes([30] * 24), "b": bytes([30] * 24)},
                          start_problems=["gamerule advance_time is 1, the scene asks for false"])
            report = rp.compare(out, scenes)
            pack = report["packs"]["none"]
            self.assertEqual((pack["compared"], pack["mean_diff"], report["noise_floor"]), (0, None, None))
            a = pack["rows"][0]
            self.assertEqual((a["diff"], a["scored"]), (30.0, False))
            self.assertEqual(a["java"]["problems"], ["start: gamerule advance_time is 1, the scene asks for false"])
            self.assertTrue((Path(out) / "report" / a["java"]["picture"]).exists())
            self.assertEqual(rp.summary(report), 1)
        self.assertEqual(ss.valid_views({"start_problems": ["x"], "views": {"a": {"problems": []}}}), 0)

    def test_pairs_of_different_pack_files_are_not_scored(self):
        scenes = ss.parse_scenes(scene())
        with tempfile.TemporaryDirectory() as out:
            self.make_run(out, "sunrise", "pack", {}, {"a": bytes([0] * 24), "b": bytes([0] * 24)})
            self.make_run(out, "java", "pack", {}, {"a": bytes([30] * 24), "b": bytes([30] * 24)}, pack_sha256="p2")
            pack = rp.compare(out, scenes)["packs"]["pack"]
            self.assertEqual(pack["compared"], 0)
            self.assertTrue(any("different pack files" in n for n in rp._notes(pack["rows"][0])))

    def test_a_failed_capture_is_shown_but_not_scored(self):
        scenes = ss.parse_scenes(scene())
        with tempfile.TemporaryDirectory() as out:
            self.make_run(out, "sunrise", "none", {"a": {"problems": ["yaw 91.0, view asks for 90.0"]}},
                          {"a": bytes([0] * 24), "b": bytes([0] * 24)})
            self.make_run(out, "java", "none", {}, {"a": bytes([90] * 24), "b": bytes([30] * 24)})
            report = rp.compare(out, scenes)
            pack = report["packs"]["none"]
            a, b = pack["rows"]
            self.assertEqual((a["diff"], a["scored"], b["scored"]), (90.0, False, True))
            self.assertEqual((pack["compared"], pack["mean_diff"], report["noise_floor"]), (1, 30.0, 30.0))
            self.assertTrue((Path(out) / "report" / a["sunrise"]["picture"]).exists())

    def test_a_corrupt_picture_leaves_the_other_pairs_intact(self):
        scenes = ss.parse_scenes(scene())
        raw = b"".join(b"\x00" + bytes([5] * 12) for _ in range(2))
        corrupt = {
            "cut_stream": png(4, 2, [], idat=zlib.compress(raw)[:-4]),
            "bad_zlib": png(4, 2, [], idat=b"not zlib"),
            "bad_crc": png(4, 2, [bytes(12)] * 2)[:-5] + b"\x00" + png(4, 2, [bytes(12)] * 2)[-4:],
            "short_ihdr": b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", b"\x00" * 5) + chunk(b"IEND", b""),
            "chunk_past_end": b"\x89PNG\r\n\x1a\n" + struct.pack(">I4s", 999, b"IHDR") + b"\x00" * 10,
            "no_iend": png(4, 2, [bytes(12)] * 2)[:-12],
            "compression_method": png(4, 2, [bytes(12)] * 2).replace(
                chunk(b"IHDR", struct.pack(">IIBBBBB", 4, 2, 8, 2, 0, 0, 0)),
                chunk(b"IHDR", struct.pack(">IIBBBBB", 4, 2, 8, 2, 1, 0, 0))),
            "filter_method": png(4, 2, [bytes(12)] * 2).replace(
                chunk(b"IHDR", struct.pack(">IIBBBBB", 4, 2, 8, 2, 0, 0, 0)),
                chunk(b"IHDR", struct.pack(">IIBBBBB", 4, 2, 8, 2, 0, 1, 0))),
            "iend_with_data": png(4, 2, [bytes(12)] * 2)[:-12] + chunk(b"IEND", b"x"),
        }
        for name, data in corrupt.items():
            with self.subTest(name), tempfile.TemporaryDirectory() as out:
                self.make_run(out, "sunrise", "none", {}, {"a": bytes([0] * 24), "b": bytes([0] * 24)})
                self.make_run(out, "java", "none", {}, {"b": bytes([30] * 24)})
                (Path(out) / "java" / "none" / "a.png").write_bytes(data)
                report = rp.compare(out, scenes)
                a, b = report["packs"]["none"]["rows"]
                self.assertNotIn("diff", a)
                self.assertEqual(len(a["java"]["problems"]), 1)
                self.assertEqual((b["diff"], b["scored"]), (30.0, True))

    def test_pairs_of_one_pixel_wide_or_high_pictures(self):
        for size in ([1, 5], [5, 1]):
            with self.subTest(size), tempfile.TemporaryDirectory() as out:
                scenes = ss.parse_scenes(scene(size=size))
                for side, value in (("sunrise", 10), ("java", 20)):
                    folder = Path(out) / side / "none"
                    folder.mkdir(parents=True)
                    (folder / rp.RUN_MARK).write_text("")
                    (folder / "run.json").write_text(json.dumps(
                        {"pack": "none", "views": {}, "world_id": "w", "scene_hash": rp.scene_hash(scenes),
                         "pack_sha256": "none"}))
                    for view in ("a", "b"):
                        rp.write_png(folder / f"{view}.png", size[0], size[1], bytes([value] * 3 * size[0] * size[1]))
                report = rp.compare(out, scenes, ["all"])
                pair = Path(out) / "report" / report["packs"]["none"]["rows"][0]["pair"]
                width, height, rgb = rp.read_png(pair, [size[0] * 2, size[1]])
                self.assertEqual((width, height), (size[0] * 2, size[1]))
                self.assertEqual(rgb, bytes(([10] * 3 * size[0] + [20] * 3 * size[0]) * size[1]))

    def test_phase_shots_pair_by_best_match_and_report_their_own_variation(self):
        # Sunrise froze its animation at frame A, Java at frame C; stepping shows B then C on Sunrise's side, and D
        # then E on Java's. The best pair is Sunrise's shot 2 (C) with Java's shot 0 (C): difference 0.
        A, B, C, D, E = (bytes([v] * 24) for v in (10, 20, 40, 70, 100))
        scenes = ss.parse_scenes(scene())
        with tempfile.TemporaryDirectory() as out:
            self.make_run(out, "sunrise", "none", {}, {"a": A, "a.p1": B, "a.p2": C, "b": A})
            self.make_run(out, "java", "none", {}, {"a": C, "a.p1": D, "a.p2": E, "b": B})
            report = rp.compare(out, scenes, ["none/a"])
            a, b = report["packs"]["none"]["rows"]
            self.assertEqual((a["diff"], a["phase"]), (0.0, {"sunrise": 2, "java": 0}))
            self.assertTrue(a["sunrise"]["picture"].endswith("a.p2.png"))
            self.assertTrue(a["java"]["picture"].endswith("a.png"))
            # Own variation: the mean difference between a side's own shots. Sunrise 10, 20, 40: (10 + 30 + 20) / 3
            # = 20; Java 40, 70, 100: (30 + 60 + 30) / 3 = 40. The view's noise is the larger.
            self.assertEqual((a["sunrise"]["own_spread"], a["java"]["own_spread"], a["noise"]), (20.0, 40.0, 40.0))
            self.assertTrue(a["within_own_variation"])
            self.assertIn("within the pack's own variation", rp._notes(a))
            # One shot a side: no variation known, so never "within" it.
            self.assertEqual((b["diff"], b["noise"], b["within_own_variation"]), (10.0, None, False))
            pair = Path(out) / "report" / a["pair"]
            self.assertEqual(rp.read_png(pair, [4, 1])[2], bytes([40] * 12))  # both halves show frame C

    def test_phase_shot_names_and_steps(self):
        self.assertEqual([rp.shot_index(n) for n in ("v.png", "v.p1.png", "v.p12.png", "a_b.png")], [0, 1, 12, 0])
        scenes = ss.parse_scenes(scene(phase_shots=9))
        self.assertEqual((ss.phase_step(scenes, "sunrise"), ss.phase_step(scenes, "java")), (1, 9))
        self.assertEqual(ss.parse_scenes(scene())["phase_shots"], 1)
        for bad in (0, 17, True, 2.0):
            with self.assertRaises(ss.SceneError):
                ss.parse_scenes(scene(phase_shots=bad))
        # The offsets i * 1 - j * K for i, j in 0..K-1 cover every residue 0..K*K-1 exactly once.
        k = 9
        self.assertEqual(sorted((i - j * k) % (k * k) for i in range(k) for j in range(k)), list(range(k * k)))

    def test_valid_views(self):
        self.assertEqual(ss.valid_views({"views": {"a": {"problems": []}, "b": {"problems": ["x"]}}}), 1)
        self.assertEqual(ss.valid_views({"views": {}}), 0)


if __name__ == "__main__":
    unittest.main()
