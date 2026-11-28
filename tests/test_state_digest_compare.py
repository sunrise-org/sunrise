import tempfile
import unittest
from pathlib import Path

from tools import state_digest_compare


def line(tick, dim="minecraft:overworld", run="1", entities="3:aa"):
    fields = {group: "same" for group in state_digest_compare.GROUPS if group != "walk_error"}
    fields.update(tick=tick, dim=dim, run=run, entities=entities)
    return " ".join(f"{key}={value}" for key, value in fields.items()) + "\n"


BASE = [line(100, run="0"), line(101), line(102), line(103)]


class CompareTests(unittest.TestCase):
    def test_the_same_run_is_identical(self):
        report, same = state_digest_compare.compare(BASE, list(BASE))
        self.assertTrue(same)
        self.assertEqual(report[-1], "IDENTICAL")

    def test_a_shifted_game_time_is_different(self):
        shifted = [line(100, run="0"), line(101), line(103), line(104)]
        report, same = state_digest_compare.compare(BASE, shifted)
        self.assertFalse(same)
        self.assertIn("tick                  DIFFERS on 2 steps, first step 2 (tick 102 vs 103): 102 vs 103", report)

    def test_a_step_in_another_level_is_different(self):
        moved = [line(100, run="0"), line(101), line(102, dim="minecraft:the_nether"), line(103)]
        report, same = state_digest_compare.compare(BASE, moved)
        self.assertFalse(same)
        self.assertIn("dim                   DIFFERS on 1 steps, first step 2 (tick 102 vs 102): minecraft:overworld vs "
                      "minecraft:the_nether", report)

    def test_player_detail_only_explains_the_first_divergent_running_tick(self):
        def details(time, slot):
            return [
                "  e 0 minecraft:pig id=99 join=1\n",
                "  be (0, 0, 0) slot=0 minecraft:furnace abcd\n",
                f"  p\tid=1\tname=local\tselected_and_mode=({slot}, 0)\n",
                f"  p\tid=2\tname=Alice\tstat:minecraft:custom/minecraft:play_time={time}\n",
                "  cost_total_ns=100 lines=1\n",
            ]

        a = [line(100, run="0"), *details(7, 1), line(101), *details(9, 2), line(102), *details(10, 3)]
        b = [line(100, run="0"), *details(8, 1),
             line(101).replace("players=same", "players=changed"), *details(11, 4),
             line(102).replace("players=same", "players=changed"), *details(12, 5)]
        report, same = state_digest_compare.compare(a, b)
        self.assertFalse(same)
        self.assertEqual([row for row in report if row.startswith("  player ")], [
            "  player id=1 name=local selected_and_mode: (2, 0) vs (4, 0)",
            "  player id=2 name=Alice stat:minecraft:custom/minecraft:play_time: 9 vs 11",
        ])
        self.assertTrue(state_digest_compare.compare(a, [line(100, run="0"), line(101), line(102)])[1])

    def test_plain_run_can_still_be_compared_with_a_detailed_one(self):
        left = [line(101), "  p\tid=1\tname=local\tselected_and_mode=(0, 0)\n"]
        right = [line(101).replace("players=same", "players=changed")]
        report, same = state_digest_compare.compare(left, right)
        self.assertFalse(same)
        self.assertFalse(any(row.startswith("  player ") for row in report))

    def test_a_missing_step_is_different(self):
        _, same = state_digest_compare.compare(BASE, BASE[:-1])
        self.assertFalse(same)

    def test_a_run_without_stepped_ticks_is_different(self):
        _, same = state_digest_compare.compare(BASE, [line(100, run="0")])
        self.assertFalse(same)


class CoverageTests(unittest.TestCase):
    def runs(self, a, b):
        with tempfile.TemporaryDirectory() as tmp:
            dirs = [Path(tmp) / "a", Path(tmp) / "b"]
            for d, text in zip(dirs, (a, b)):
                d.mkdir()
                if text is not None:
                    (d / "coverage.txt").write_text(text)
            return state_digest_compare.compare_coverage(*dirs)

    def test_runs_without_coverage_have_nothing_to_compare(self):
        self.assertIsNone(self.runs(None, None))

    def test_a_different_saved_trader_delay_is_different(self):
        _, same = self.runs("trader spawn_delay=22800\n", "trader spawn_delay=24000\n")
        self.assertFalse(same)

    def test_coverage_in_one_run_only_is_different(self):
        _, same = self.runs("fox_spit reached\n", None)
        self.assertFalse(same)

    def test_the_same_coverage_is_identical(self):
        _, same = self.runs("fox_spit reached\n", "fox_spit reached\n")
        self.assertTrue(same)


if __name__ == "__main__":
    unittest.main()
