import contextlib
import io
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

from tools import state_digest_compare, state_digest_players, state_digest_scene


def line(tick, run=1, dim='minecraft:overworld', **changes):
    fields = dict.fromkeys(state_digest_compare.GROUPS, 'same')
    del fields['walk_error']
    fields.update(tick=tick, dim=dim, run=run)
    fields.update(changes)
    return ' '.join(f'{key}={value}' for key, value in fields.items()) + '\n'


class DigestCompareTests(unittest.TestCase):
    def test_reports_earliest_group_across_steps_and_each_groups_tick(self):
        a = [line(42), line(43)]
        b = [line(42, players='changed'), line(43, seats='changed')]
        report, same = state_digest_compare.compare(a, b)
        self.assertFalse(same)
        self.assertIn('first differing group: players at step 1 (tick 42 vs 42)', report)
        self.assertTrue(any(item.startswith('seats ') and '(tick 43 vs 43)' in item for item in report))

    def test_ignores_only_groups_requested(self):
        a, b = [line(42)], [line(42, players='changed')]
        report, same = state_digest_compare.compare(a, b, ['players'])
        self.assertTrue(same)
        self.assertEqual(report[-1], 'MATCH (ignored differences)')
        self.assertIn('first differing group: players (ignored) at step 1 (tick 42 vs 42)', report)
        self.assertFalse(state_digest_compare.compare(a, [line(43)], ['tick'])[1])

    def test_ignored_difference_does_not_hide_a_later_failure(self):
        a = [line(42), line(43)]
        b = [line(42, players='changed'), line(43, seats='changed')]
        report, same = state_digest_compare.compare(a, b, ['players'])
        self.assertFalse(same)
        self.assertIn('first differing group: seats at step 2 (tick 43 vs 43)', report)

    def test_missing_ignored_group_reports_the_missing_step(self):
        missing = line(43).replace(' players=same', '')
        report, same = state_digest_compare.compare(
            [line(42), line(43)], [line(42, players='changed'), missing], ['players'])
        self.assertFalse(same)
        self.assertIn('first differing group: players at step 2 (tick 43 vs 43)', report)

    def test_failed_walk_does_not_report_fallback_entity_hash_as_a_difference(self):
        a = [line(42), line(43)]
        b = [line(42, entities='fallback', walk_error='Unjoined(Mob(0))'), line(43, entities='changed')]
        report, same = state_digest_compare.compare(a, b)
        self.assertFalse(same)
        self.assertIn('first differing group: walk_error at step 1 (tick 42 vs 42)', report)
        self.assertTrue(any(item.startswith('entities ') and '(tick 43 vs 43)' in item for item in report))

    def test_rejects_missing_groups_and_unequal_tick_counts(self):
        missing = line(42).replace(' seats=same', '')
        report, same = state_digest_compare.compare([missing], [missing])
        self.assertFalse(same)
        self.assertTrue(any(item.startswith('seats ') and '<missing>' in item for item in report))
        self.assertFalse(state_digest_compare.compare([missing], [missing], ['seats'])[1])
        self.assertFalse(state_digest_compare.compare([line(42)], [line(42), line(43)])[1])

    def test_walk_error_with_spaces_fails_even_when_both_runs_agree(self):
        bad = line(42, walk_error='MissingJoin { index: 17 }')
        report, same = state_digest_compare.compare([bad], [bad])
        self.assertFalse(same)
        self.assertTrue(any('MissingJoin { index: 17 }' in item for item in report))

    def test_frozen_ticks_are_diagnostic_only(self):
        a = [line(42, run=0), line(42, run=0, level_random='changed'), line(43)]
        b = [line(43)]
        report, same = state_digest_compare.compare(a, b)
        self.assertTrue(same)
        self.assertIn('frozen ticks that moved the level random: 1 vs 0', report)


class PlayerCompareTests(unittest.TestCase):
    def test_player_digest_and_final_summary_have_independent_opt_outs(self):
        with tempfile.TemporaryDirectory() as temp, contextlib.redirect_stdout(io.StringIO()) as output:
            a, b = (Path(temp) / side for side in ('left', 'right'))
            for out in (a, b):
                out.mkdir()
                (out / 'players.txt').write_text(out.name)
            nether = line(12, dim='minecraft:the_nether')
            (a / 'digest.txt').write_text(line(42) + nether)
            (b / 'digest.txt').write_text(nether + line(42, players='changed'))
            self.assertEqual(state_digest_players.compare(a, b), 1)
            self.assertEqual(state_digest_players.compare(a, b, ['players']), 1)
            self.assertEqual(state_digest_players.compare(a, b, ['player_summary']), 1)
            self.assertEqual(state_digest_players.compare(a, b, ['players', 'player_summary']), 0)
            self.assertIn('== minecraft:the_nether', output.getvalue())
            self.assertIn('== player_summary\ndifferent (ignored)', output.getvalue())

    def test_interleaved_dimensions_keep_their_player_details(self):
        with tempfile.TemporaryDirectory() as temp, contextlib.redirect_stdout(io.StringIO()) as output:
            a, b = (Path(temp) / side for side in ('left', 'right'))
            nether = line(42, dim='minecraft:the_nether')
            for out in (a, b):
                out.mkdir()
                (out / 'players.txt').write_text('same')
            (a / 'digest.txt').write_text(line(42) + '  p\tid=2\tname=Alice\tmotion=1111\n' + nether
                                         + '  p\tid=2\tname=Alice\tmotion=2222\n')
            (b / 'digest.txt').write_text(nether + '  p\tid=2\tname=Alice\tmotion=2222\n'
                                         + line(42, players='changed') + '  p\tid=2\tname=Alice\tmotion=3333\n')
            self.assertEqual(state_digest_players.compare(a, b), 1)
            self.assertIn('  player id=2 name=Alice motion: 1111 vs 3333', output.getvalue())

    def test_frozen_only_runs_cannot_pass(self):
        with tempfile.TemporaryDirectory() as temp, contextlib.redirect_stdout(io.StringIO()):
            out = Path(temp)
            (out / 'digest.txt').write_text(line(42, run=0))
            (out / 'players.txt').write_text('same')
            self.assertEqual(state_digest_players.compare(out, out), 1)


class LaunchTests(unittest.TestCase):
    def test_dedicated_scene_requires_sync_acknowledgement_when_recording(self):
        server = object.__new__(state_digest_scene.Server)
        commands = ['digest-sync 1 2 3']
        with patch.object(server, 'run', return_value=['[12:00:00] [Server thread/INFO]: digest-sync done']) as run:
            server.run_seeded(commands, require_sync=True)
            run.assert_called_once_with(commands)
        with patch.object(server, 'run', return_value=['[12:00:00] [Server thread/WARN]: digest-sync: refused']):
            with self.assertRaisesRegex(SystemExit, 'digest-sync: refused'):
                server.run_seeded(commands, require_sync=True)
            server.run_seeded(commands, require_sync=False)

    def test_compare_option_can_follow_ignore_option(self):
        argv = ['state_digest_players.py', '--ignore-group', 'players', '--compare', 'a', 'b']
        with patch.object(sys, 'argv', argv), patch.object(state_digest_players, 'compare', return_value=0) as compare:
            with self.assertRaises(SystemExit) as exit_code:
                state_digest_players.main()
        self.assertEqual(exit_code.exception.code, 0)
        compare.assert_called_once_with(Path('a'), Path('b'), ['players'])

    def test_player_harness_passes_ctl_path_and_ticks_to_integrated_run(self):
        with tempfile.TemporaryDirectory() as temp, contextlib.redirect_stdout(io.StringIO()):
            out = Path(temp) / 'out'
            argv = ['state_digest_players.py', '--integrated', '--ctl', 'custom/ctl', '--ticks=17',
                    'game', 'resources', 'template', str(out), 'bots']
            with patch.object(sys, 'argv', argv), patch.object(state_digest_players.base, 'hold_gate'):
                with patch.object(state_digest_players, 'run_integrated') as run:
                    state_digest_players.main()
                    run.assert_called_once()
            args = run.call_args.args[0]
            self.assertEqual(args.ctl, Path('custom/ctl'))
            self.assertEqual(args.ticks, 17)
            self.assertEqual(args.out, out)


class SceneTests(unittest.TestCase):
    def test_cluster_is_in_the_default_scene_after_seed_sync(self):
        cluster = state_digest_scene.entity_cluster_commands()
        self.assertEqual(state_digest_scene.scene_commands(False)[-len(cluster):], cluster)
        summons = {command.split()[1]: command for command in cluster if command.startswith('summon ')}
        self.assertIn('Passengers:[{id:"minecraft:pig"', summons['minecraft:oak_boat'])
        self.assertIn('Passengers:[{id:"minecraft:sheep"', summons['minecraft:minecart'])
        self.assertEqual(summons['minecraft:cow'].count('Passengers:['), 2)
        self.assertEqual({'minecraft:item_frame', 'minecraft:painting', 'minecraft:armor_stand'} - summons.keys(), set())
        self.assertIn('leash:[I;30,', summons['minecraft:pig'])
        self.assertIn('minecraft:oak_fence', '\n'.join(cluster))


if __name__ == '__main__':
    unittest.main()
