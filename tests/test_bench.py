"""tools/bench.py: the parsers, the report table and the per-server setup (no server is started)."""

from pathlib import Path
import subprocess
import tempfile
import types
import unittest

from tools import bench


def args(**kw):
    base = dict(server=['sunrise'], seed=777, view_distance=8, pre_generated=None, pumpkin_config=None, exe='pumpkin',
                jar='server.jar', heap='2g', bots=5, behaviour='mixed', warmup=1, seconds=60, join_every=0.5, spread=0,
                bots_bin='sunrise-bots', bots_arg=[], save_world=None, profile=None)
    base.update(kw)
    return types.SimpleNamespace(**base)


class ParserTests(unittest.TestCase):
    def test_tick_query(self):
        lines = ['[Server] Average time per tick: 12.5 ms', 'Target: 50.0 ms', 'P50: 11.0 ms P95: 20.5 ms P99: 31.0 ms']
        self.assertEqual(bench.parse_tick_query(lines), {'average': 12.5, 'p50': 11.0, 'p95': 20.5, 'p99': 31.0})
        self.assertIsNone(bench.parse_tick_query(lines[:1]))

    def test_gametime_and_tps(self):
        # The console logs vanilla's commands.time.query.gametime: "The game time is %s tick(s)".
        self.assertEqual(bench.parse_gametime(['[12:00:01] [Server thread/INFO]: The game time is 4200 tick(s)']), 4200)
        self.assertIsNone(bench.parse_gametime(['nothing', 'The time is 4200']))
        self.assertEqual(bench.tps(1000, 2200, 60), 20.0)
        with self.assertRaises(ValueError):
            bench.tps(1000, 2200, 0)

    def test_a_server_without_a_query_reports_it_as_unsupported(self):
        self.assertFalse(bench.SERVERS['pumpkin']['tick_query'])
        self.assertTrue(all(bench.SERVERS[k].values() for k in ('sunrise', 'vanilla')))
        run = bench.summarize('pumpkin', args(), None, [], [], None, 60, {}, 1.0, 100)
        self.assertEqual((run['tps'], run['mspt']['p99'], run['mspt_worst_p99'], run['tick_queries']), ('unsupported',) * 4)
        self.assertIn('| TPS | unsupported |', bench.table([run]))
        self.assertIn('| MSPT p99 (worst query) | unsupported |', bench.table([run]))

    def test_a_supported_query_is_measured_and_none_is_omitted(self):
        ticks = [{'average': 10.0, 'p50': 9.0, 'p95': 15.0, 'p99': 20.0}]
        run = bench.summarize('sunrise', args(), ticks, [], [], (1000, 2200), 60, {}, 1.0, 100)
        self.assertEqual((run['tps'], run['mspt']['p99'], run['tick_queries']), (20.0, 20.0, 1))

    def test_ask_raises_when_the_console_gives_no_parseable_answer(self):
        class Console:
            """A server console that answers every command with the given line."""
            def __init__(self, server, reply):
                self.server, self.reply = server, reply

            def write(self, text):
                self.server.lines.append(self.reply)

            def flush(self):
                pass

        def server_answering(reply):
            server = object.__new__(bench.Server)
            server.lines, server.lock = [], bench.threading.Lock()
            server.process = types.SimpleNamespace(stdin=Console(server, reply))
            return server

        old = server_answering('[12:00:01] [Server thread/INFO]: The time is 4200')
        with self.assertRaisesRegex(RuntimeError, r'`time query gametime` got no parseable answer'):
            old.ask('time query gametime', bench.parse_gametime, timeout=0.2)
        real = server_answering('[12:00:02] [Server thread/INFO]: The game time is 4200 tick(s)')
        self.assertEqual(real.ask('time query gametime', bench.parse_gametime), 4200)

    def test_ask_raises_when_the_console_is_closed(self):
        class Stdin:
            def write(self, text): raise BrokenPipeError('closed')

        server = object.__new__(bench.Server)
        server.lines, server.lock, server.process = [], bench.threading.Lock(), types.SimpleNamespace(stdin=Stdin())
        with self.assertRaisesRegex(RuntimeError, 'could not be sent'):
            server.ask('tick query', bench.parse_tick_query)

    def test_cpu_seconds_and_percentile(self):
        self.assertEqual(bench.cpu_seconds('1-00:00:01.5'), 86401.5)
        self.assertEqual(bench.cpu_seconds('02:03'), 123.0)
        self.assertEqual(bench.percentile([5, 1, 3], 50), 3)
        self.assertIsNone(bench.percentile([], 50))

    def test_bot_output_split(self):
        samples, summary = bench.split_bot_output(['{"t": 1, "online": 2}\n', '{\n', '  "joined": 2\n', '}\n'])
        self.assertEqual(samples, [{'t': 1, 'online': 2}])
        self.assertEqual(summary, {'joined': 2})

    def test_the_first_bots_spawn_line_is_not_part_of_the_summary(self):
        lines = ['{"spawn":[96.5,145.0,-32.5]}\n', '{"t": 1, "online": 1}\n', '{\n', '  "joined": 1\n', '}\n']
        self.assertEqual(bench.split_bot_output(lines), ([{'t': 1, 'online': 1}], {'joined': 1}))

    def test_spread_spots_repeat_for_a_seed_and_stay_in_the_square(self):
        spots = bench.spread_spots(50, 10000, 12345)
        self.assertEqual(spots, bench.spread_spots(50, 10000, 12345))
        self.assertNotEqual(spots, bench.spread_spots(50, 10000, 12346))
        self.assertEqual(len(set(spots)), 50)
        self.assertTrue(all(abs(x) <= 5000 and abs(z) <= 5000 for x, z in spots))

    def test_a_spread_teleports_every_bot_up_and_then_onto_the_ground(self):
        sent, parsed = [], []
        class Console:
            def ask(self, command, parse, timeout=5):
                sent.append(command)
                name = command.split()[1 if command.startswith('tp ') else 2]
                parsed.append((parse([f'[Server thread/INFO]: Teleported {name} to 1.5, 320.0, 2.5']),
                               parse([f'Teleported {name}0 to 1.5, 320.0, 2.5'])))
        spots = bench.spread(Console(), args(bots=2, spread=1000, seed=7), settle=0)
        self.assertEqual(sent, [f'tp Bot0 {spots[0][0]} 320 {spots[0][1]}', f'tp Bot1 {spots[1][0]} 320 {spots[1][1]}',
                                'execute at Bot0 positioned over motion_blocking run tp Bot0 ~ ~ ~',
                                'execute at Bot1 positioned over motion_blocking run tp Bot1 ~ ~ ~'])
        self.assertEqual(parsed, [(True, None)] * 4)

    def test_counts_and_extra_bot_arguments(self):
        self.assertEqual(bench.parse_count(['[Server thread/INFO]: Test passed, count: 1712']), 1712)
        self.assertEqual(bench.parse_count(['Test passed. Count: 48']), 48)
        self.assertEqual(bench.parse_count(['Test failed']), 0)
        self.assertIsNone(bench.parse_count(['Teleported Bot0 to 1, 2, 3']))
        self.assertEqual(bench.bots_command(args(bots_arg=['--leash', '96']), 25565, 60)[-2:], ['--leash', '96'])

    def test_spread_bots_take_the_teleport_as_home(self):
        self.assertNotIn('--rehome', bench.bots_command(args(), 25565, 60))
        self.assertEqual(bench.bots_command(args(spread=10000), 25565, 60)[-1], '--rehome')

    def test_bandwidth_divides_by_each_bots_time_online(self):
        # Bot 0 is on for 100 s, bot 1 for 90 s (it joined 10 s later), bot 2 failed and counts for nothing.
        per_bot = [{'bytes_in': 1000, 'bytes_out': 100}, {'bytes_in': 900, 'bytes_out': 90},
                   {'bytes_in': 50, 'bytes_out': 5, 'error': 'refused'}]
        self.assertEqual(bench.bytes_per_player_second(per_bot, 100, 10), {'bytes_in': 10, 'bytes_out': 1})
        self.assertIsNone(bench.bytes_per_player_second([{'joined_after_ms': 80}], 100, 10))


class SetupTests(unittest.TestCase):
    def test_pumpkin_config_and_pre_generated_world(self):
        with tempfile.TemporaryDirectory() as tmp:
            tmp = Path(tmp)
            world = tmp / 'src'
            world.mkdir()
            (world / 'level.dat').write_text('x')
            run = tmp / 'run'
            run.mkdir()
            command = bench.prepare('pumpkin', run, 25599, args(pre_generated=str(world)))
            self.assertEqual(len(command), 1)
            config = (run / 'config/configuration.toml').read_text()
            self.assertIn('127.0.0.1:25599', config)
            self.assertIn('[networking.bedrock]\nenabled = false', config)
            self.assertIn('enabled = false', (run / 'config/features.toml').read_text())
            self.assertEqual((run / 'world/level.dat').read_text(), 'x')

    def test_vanilla_properties(self):
        with tempfile.TemporaryDirectory() as tmp:
            command = bench.prepare('vanilla', tmp, 25598, args())
            text = (Path(tmp) / 'server.properties').read_text()
            for line in ('server-ip=127.0.0.1', 'server-port=25598', 'level-seed=777', 'online-mode=false'):
                self.assertIn(line, text)
            self.assertIn('-Xmx2g', command)
            self.assertEqual((Path(tmp) / 'eula.txt').read_text(), 'eula=true\n')


class TableTests(unittest.TestCase):
    def test_table_marks_missing_numbers(self):
        run = {'server': 'sunrise', 'bots': 5, 'tps': 20.0, 'mspt': {'average': 3.0, 'p99': 9.0},
               'bytes_per_player_second': None, 'join_ms': {'p50': 80, 'worst': 120}}
        text = bench.table([run, {'server': 'pumpkin', 'error': 'no start\nmore'}])
        self.assertIn('| TPS | 20.0 |', text)
        self.assertIn('| Bytes in per player and s online | - |', text)
        self.assertIn('pumpkin failed: no start', text)
        self.assertNotIn('| Profile |', text)


class ProfileTests(unittest.TestCase):
    def test_profile_timing_stays_inside_the_measured_window(self):
        self.assertEqual(bench.profile_min_seconds(30), 40)
        self.assertEqual(bench.profile_delay(60, 30), 15.0)
        self.assertEqual(bench.profile_delay(40, 30), 5.0)
        with self.assertRaisesRegex(ValueError, 'at least 40'):
            bench.profile_delay(39, 30)

        now = [100.0]
        events = []

        class FakeServer:
            process = types.SimpleNamespace(pid=1234)

            def ask(self, command, parse, timeout=5):
                events.append((now[0], command))
                if command == 'time query gametime':
                    return int((now[0] - 100.0) * 20)
                return {'average': 10.0, 'p50': 9.0, 'p95': 15.0, 'p99': 20.0}

        def sleep(dt):
            now[0] += dt

        started = []
        fake_proc = types.SimpleNamespace(poll=lambda: 0)

        def start_profile():
            started.append(now[0])
            events.append((now[0], 'report'))
            return fake_proc

        gametime, window, ticks, rss, cpu, proc, err = bench.measure_window(
            FakeServer(), bench.SERVERS['sunrise'], 40, profile=30,
            start_profile_fn=start_profile, clock=lambda: now[0], sleep=sleep, usage_fn=lambda pid: None,
        )
        self.assertEqual(gametime, (0, 800))
        self.assertEqual(window, 40.0)
        self.assertEqual(len(ticks), 8)
        self.assertIs(proc, fake_proc)
        self.assertIsNone(err)
        self.assertEqual(len(started), 1)
        gametime_times = [t for t, cmd in events if cmd == 'time query gametime']
        self.assertGreater(started[0], gametime_times[0])
        self.assertLess(started[0] + 30, gametime_times[-1])

    def test_profile_argument_checks(self):
        with tempfile.TemporaryDirectory() as tmp:
            bots = Path(tmp) / 'sunrise-bots'
            bots.write_text('')
            bench.check(args(server=['sunrise'], exe='sunrise-server', bots_bin=str(bots), seconds=40, profile=30))
            for servers in (['vanilla'], ['pumpkin'], ['sunrise', 'vanilla']):
                with self.assertRaises(SystemExit) as cm:
                    bench.check(args(server=servers, exe='sunrise-server', jar='server.jar',
                                     bots_bin=str(bots), seconds=60, profile=30))
                self.assertIn('sunrise', str(cm.exception).lower())
            with self.assertRaises(SystemExit) as cm:
                bench.check(args(server=['sunrise'], exe='sunrise-server', bots_bin=str(bots), seconds=39, profile=30))
            self.assertIn('40', str(cm.exception))
            for bad in (0, -5, 601):
                with self.assertRaises(SystemExit):
                    bench.check(args(server=['sunrise'], exe='sunrise-server', bots_bin=str(bots),
                                     seconds=700, profile=bad))

    def test_profile_output_copy_and_report_entry(self):
        with tempfile.TemporaryDirectory() as tmp:
            tmp = Path(tmp)
            scratch = tmp / 'scratch'
            reports = scratch / 'reports'
            reports.mkdir(parents=True)
            capture = reports / 'profile-20261127-190000Z.hzperf'
            capture.write_bytes(b'hzperf-bytes')
            out = tmp / 'out'
            out.mkdir()
            stdout = '\n'.join([
                'Recording for 30 s...',
                'dedicated server, 30.0 s, 600 ticks',
                'health  TPS 20.0/20 · MSPT mean 10.0 max 15.0 ms',
                f'Saved {capture} (the file stays on your computer)',
            ])

            class FakeProc:
                returncode = 0

                def communicate(self, timeout=None):
                    return stdout, ''

            dest = out / 'sunrise-50.hzperf'
            entry = bench.finish_profile(FakeProc(), None, scratch, dest, 30)
            self.assertEqual(entry, {'path': str(dest), 'seconds': 30.0, 'ticks': 600})
            self.assertEqual(dest.read_bytes(), b'hzperf-bytes')

            # Without a summary line, the file is still copied and seconds/ticks are None.
            class MinimalProc:
                returncode = 0

                def communicate(self, timeout=None):
                    return f'Recording for 30 s...\n(no summary: bad)\nSaved {capture} (the file stays on your computer)\n', ''

            minimal = bench.finish_profile(MinimalProc(), None, scratch, out / 'sunrise-5.hzperf', 30)
            self.assertEqual(minimal, {'path': str(out / 'sunrise-5.hzperf'), 'seconds': None, 'ticks': None})

    def test_profile_failure_and_table_rows(self):
        class FailingProc:
            returncode = 1

            def communicate(self, timeout=None):
                return '', 'sunrise-server: no server is running\n'

        class TimedOutProc:
            returncode = None
            killed = False

            def communicate(self, timeout=None):
                if not self.killed:
                    raise subprocess.TimeoutExpired(['sunrise-server'], timeout)
                return '', ''

            def kill(self):
                self.killed = True

        with tempfile.TemporaryDirectory() as tmp:
            tmp = Path(tmp)
            failed = bench.finish_profile(FailingProc(), None, tmp, tmp / 'sunrise-5.hzperf', 30)
            self.assertIn('error', failed)
            self.assertIn('no server is running', failed['error'])
            timed = TimedOutProc()
            timed_entry = bench.finish_profile(timed, None, tmp, tmp / 'sunrise-5.hzperf', 30)
            self.assertTrue(timed.killed)
            self.assertIn('timed out', timed_entry['error'])
            spawn_failed = bench.finish_profile(None, 'missing binary', tmp, tmp / 'sunrise-5.hzperf', 30)
            self.assertEqual(spawn_failed, {'error': 'missing binary'})

        base = {'server': 'sunrise', 'bots': 50, 'tps': 20.0, 'mspt': {'average': 12.0, 'p99': 25.0},
                'mspt_worst_p99': 30.0, 'rss_mb': {'mean': 512.0}, 'cpu_percent': 80.0,
                'bytes_per_player_second': None, 'join_ms': {'p50': 50, 'worst': 90}, 'start_seconds': 3.5}
        ok_table = bench.table([dict(base, profile={'path': '/out/sunrise-50.hzperf', 'seconds': 30.0, 'ticks': 600})])
        self.assertIn('| Profile | /out/sunrise-50.hzperf (30.0 s, 600 ticks) |', ok_table)
        fail_table = bench.table([dict(base, profile=failed)])
        self.assertIn('| TPS | 20.0 |', fail_table)
        self.assertIn('| Profile | failed: sunrise-server report exited with 1: sunrise-server: no server is running |',
                      fail_table)
        plain_table = bench.table([base])
        self.assertNotIn('| Profile |', plain_table)


if __name__ == '__main__':
    unittest.main()
