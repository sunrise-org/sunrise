"""Unit tests for tools/datafix_worlds.py: the Java runtime choice, the bot's configuration phase, the server stop."""
import socket
import struct
import subprocess
import sys
import tempfile
import threading
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest import mock

from tools import datafix_worlds as worlds


class JavaRuntimeTests(unittest.TestCase):
    def java_for(self, component, platform_name, home_name):
        with tempfile.TemporaryDirectory() as temp:
            home = Path(temp, 'java', home_name)
            (home / 'bin').mkdir(parents=True)
            (home / 'bin' / 'java').write_text('')
            pin = worlds.LOCK[platform_name]['java-runtime-gamma']
            (home / '.complete').write_text(pin['manifest_sha1'])
            with mock.patch.object(worlds, 'LOCAL', Path(temp)), mock.patch.object(worlds, 'platform', return_value=platform_name), \
                    mock.patch.object(worlds, 'version_meta', return_value={'javaVersion': {'component': component}}), \
                    mock.patch.object(worlds, 'install_runtime', side_effect=AssertionError('no download')):
                return worlds.java('x'), home

    def test_a_component_no_platform_pins_is_an_error(self):
        with self.assertRaisesRegex(ValueError, 'java-runtime-zeta'):
            self.java_for('java-runtime-zeta', 'linux', 'java-runtime-gamma-linux')

    def test_a_component_this_platform_lacks_uses_gamma_as_the_lock_says(self):
        exe, home = self.java_for('jre-legacy', 'mac-os-arm64', 'java-runtime-gamma-mac-os-arm64')
        self.assertEqual(exe, str(home / 'bin' / 'java'))


class UnpinnedPlatformTests(unittest.TestCase):
    def test_a_platform_the_lock_does_not_pin_is_refused_by_name_before_any_download(self):
        for plat in ('windows-x64', 'mac-os'):
            with self.subTest(plat), mock.patch.object(worlds, 'platform', return_value=plat), \
                    mock.patch.object(worlds, 'version_meta', return_value={'javaVersion': {'component': 'java-runtime-gamma'}}), \
                    mock.patch.object(worlds, 'install_runtime', side_effect=AssertionError('no download')), \
                    self.assertRaisesRegex(ValueError, f'platform {plat};'):
                worlds.java('x')

    def test_an_unpinned_platform_is_refused_before_version_metadata_is_read(self):
        with mock.patch.object(worlds, 'platform', return_value='windows-x64'), \
                mock.patch.object(worlds, 'version_meta', side_effect=AssertionError('no metadata download')), \
                self.assertRaisesRegex(ValueError, 'platform windows-x64;'):
            worlds.java('x')

    def test_platform_names_what_mojang_calls_the_machine(self):
        for system, machine, expected in (('win32', 'AMD64', 'windows-x64'), ('win32', 'ARM64', 'windows-arm64'),
                                          ('darwin', 'x86_64', 'mac-os'), ('darwin', 'arm64', 'mac-os-arm64'), ('linux', 'x86_64', 'linux'), ('linux', 'i686', 'linux-i386')):
            with self.subTest(system, machine=machine), mock.patch.object(worlds, 'sys', SimpleNamespace(platform=system)), \
                    mock.patch.object(worlds, 'host', SimpleNamespace(machine=lambda: machine)):
                self.assertEqual(worlds.platform(), expected)


class UnsupportedMachineTests(unittest.TestCase):
    def test_a_machine_mojang_has_no_runtime_for_is_refused_by_name(self):
        for system, machine in (('linux', 'aarch64'), ('win32', 'ia64')):
            with self.subTest(system, machine=machine), mock.patch.object(worlds, 'sys', SimpleNamespace(platform=system)), \
                    mock.patch.object(worlds, 'host', SimpleNamespace(machine=lambda: machine)), \
                    self.assertRaisesRegex(ValueError, f'machine {machine}'):
                worlds.platform()


class ServerStopTests(unittest.TestCase):
    def test_a_server_that_ignores_stop_is_killed_and_the_timeout_reaches_the_caller(self):
        server = worlds.Server.__new__(worlds.Server)
        server.process = subprocess.Popen([sys.executable, '-c', 'import time; time.sleep(60)'], stdin=subprocess.PIPE,
                                           stdout=subprocess.PIPE, text=True)
        server._pump_thread = None
        self.addCleanup(server.process.kill)
        real_wait = server.process.wait
        calls = []

        def wait(timeout=None):
            calls.append(timeout)
            if timeout == 120:
                raise subprocess.TimeoutExpired('server', 120)
            return real_wait()
        server.process.wait = wait
        with self.assertRaises(subprocess.TimeoutExpired):
            server.stop()
        self.assertIsNotNone(server.process.poll())
        self.assertEqual(calls, [120, None])
        self.assertTrue(server.process.stdout.closed)

    def test_a_server_already_killed_with_its_stdin_closed_stops_without_an_error(self):
        # A server may exit on its own before cleanup.
        server = worlds.Server.__new__(worlds.Server)
        server.process = subprocess.Popen([sys.executable, '-c', 'import time; time.sleep(60)'], stdin=subprocess.PIPE,
                                           stdout=subprocess.PIPE, text=True)
        server._pump_thread = None
        self.addCleanup(server.process.kill)
        server.process.kill()
        server.process.wait()
        server.process.stdin.close()
        server.stop()
        self.assertIsNotNone(server.process.poll())
        self.assertTrue(server.process.stdout.closed)


class FakeOutput(list):
    """A process's stdout lines, with the close of a real pipe."""
    closed = False

    def close(self):
        self.closed = True


class BlockingOutput:
    """A live process's stdout: stays open until `release` is set, then ends at EOF."""

    def __init__(self):
        self.release = threading.Event()
        self.closed = False

    def __iter__(self):
        self.release.wait(5)
        return iter(())

    def close(self):
        self.closed = True


class ServerStartupTests(unittest.TestCase):
    def test_a_startup_that_fails_stops_java_and_raises_the_original_error(self):
        process = mock.Mock(stdout=FakeOutput())
        process.poll.side_effect = lambda: process.returncode
        process.returncode = None
        process.kill.side_effect = lambda: setattr(process, 'returncode', 1)
        popen = mock.Mock(return_value=process)
        with tempfile.TemporaryDirectory() as temp, \
                mock.patch.object(worlds, 'subprocess', SimpleNamespace(Popen=popen, PIPE=subprocess.PIPE, STDOUT=subprocess.STDOUT)), \
                mock.patch.object(worlds.Server, 'wait', side_effect=TimeoutError('no Done')) as wait, \
                self.assertRaisesRegex(TimeoutError, 'no Done'):
            worlds.Server(Path(temp), Path('server.jar'), 12345)
        wait.assert_called_once_with('Done (', 600)
        popen.assert_called_once()
        process.stdin.write.assert_not_called()   # no graceful stop, which would save
        process.kill.assert_called_once_with()
        process.wait.assert_called_once_with()
        process.stdin.close.assert_called_once_with()
        self.assertTrue(process.stdout.closed)

    def test_a_failing_cleanup_does_not_replace_the_startup_error(self):
        stdout = BlockingOutput()
        self.addCleanup(stdout.release.set)
        process = mock.Mock(stdout=stdout)
        process.poll.return_value = None
        process.kill.side_effect = OSError('kill failed')
        started = []
        real_start = threading.Thread.start

        def record_start(thread):
            started.append(thread)
            return real_start(thread)

        with tempfile.TemporaryDirectory() as temp, \
                mock.patch.object(worlds, 'subprocess', SimpleNamespace(Popen=lambda *a, **k: process, PIPE=subprocess.PIPE, STDOUT=subprocess.STDOUT)), \
                mock.patch.object(threading.Thread, 'start', autospec=True, side_effect=record_start), \
                mock.patch.object(worlds.Server, 'wait', side_effect=TimeoutError('no Done')), \
                self.assertRaisesRegex(TimeoutError, 'no Done'):
            worlds.Server(Path(temp), Path('server.jar'), 12345)
        self.assertFalse(stdout.closed)   # Java may still run: its pipe is not closed under the reader
        stdout.release.set()
        for thread in started:
            thread.join(5)
        self.assertTrue(stdout.closed)


class RealChildServerTests(unittest.TestCase):
    """Server with a real child process behind real pipes, which Java is replaced by."""

    def start(self, code, *, wait=None, thread_start=None):
        """Run Server(...) on a child running `code`; returns (the Server or None, the error or None, the child)."""
        children = []

        def popen(command, **kwargs):
            child = subprocess.Popen([sys.executable, '-c', code], **kwargs)
            self.addCleanup(child.wait)
            self.addCleanup(child.kill)
            children.append(child)
            return child
        fake_subprocess = SimpleNamespace(Popen=popen, PIPE=subprocess.PIPE, STDOUT=subprocess.STDOUT)
        server = error = None
        with tempfile.TemporaryDirectory() as temp, mock.patch.object(worlds, 'subprocess', fake_subprocess), \
                mock.patch.object(worlds.Server, 'wait', **(wait or {'return_value': []})), \
                mock.patch.object(threading.Thread, 'start', **(thread_start or {'autospec': True, 'side_effect': threading.Thread.start})):
            try:
                server = worlds.Server(Path(temp), Path('server.jar'), 12345)
            except Exception as caught:
                error = caught
        return server, error, children[0]

    def assert_reaped_and_closed(self, child):
        self.assertIsNotNone(child.poll())
        self.assertTrue(child.stdin.closed)
        self.assertTrue(child.stdout.closed)

    def test_a_startup_timeout_reaps_the_child_and_closes_both_pipes(self):
        server, error, child = self.start('import time; time.sleep(30)', wait={'side_effect': TimeoutError('no Done')})
        self.assertIsInstance(error, TimeoutError)
        self.assertEqual(str(error), 'no Done')
        self.assertNotEqual(child.returncode, 0)
        self.assert_reaped_and_closed(child)

    def test_a_pump_that_cannot_start_reaps_the_child_and_closes_both_pipes(self):
        server, error, child = self.start('import time; time.sleep(30)',
                                          thread_start={'side_effect': RuntimeError("can't start new thread")})
        self.assertIsInstance(error, RuntimeError)
        self.assertEqual(str(error), "can't start new thread")
        self.assertNotEqual(child.returncode, 0)
        self.assert_reaped_and_closed(child)

    def test_stop_with_save_waits_for_the_child_and_closes_both_pipes_after_its_output(self):
        server, error, child = self.start('import sys; sys.stdin.readline(); print("saved", flush=True)')
        self.assertIsNone(error)
        server.stop(save=True)
        self.assertEqual(child.returncode, 0)
        self.assertEqual(server.lines, ['saved'])
        self.assert_reaped_and_closed(child)

    def test_stop_without_save_kills_the_child_and_closes_both_pipes(self):
        server, error, child = self.start('import time; time.sleep(30)')
        self.assertIsNone(error)
        server.stop(save=False)
        self.assertNotEqual(child.returncode, 0)
        self.assert_reaped_and_closed(child)

    def test_a_pump_that_finishes_after_the_join_deadline_still_closes_stdout(self):
        server, error, child = self.start('import sys; sys.stdin.readline(); print("late", flush=True)')
        self.assertIsNone(error)
        entered = threading.Event()
        release = threading.Event()
        self.addCleanup(release.set)
        real_put = server.queue.put

        def slow_put(line):
            entered.set()
            release.wait(5)
            return real_put(line)

        server.queue.put = slow_put
        pump = server._pump_thread
        real_join = pump.join

        def expired_join(timeout=None):
            self.assertTrue(entered.wait(5), 'pump did not reach queue.put before stop joined it')
            # Simulate the five-second join expiring while the pump is still draining.
            return None

        pump.join = expired_join
        try:
            server.stop(save=True)
            self.assertEqual(child.returncode, 0)
            self.assertTrue(child.stdin.closed)
            self.assertTrue(pump.is_alive())
            self.assertFalse(child.stdout.closed)
        finally:
            release.set()
            real_join(5)
        self.assertFalse(pump.is_alive())
        self.assertEqual(server.lines, ['late'])
        self.assert_reaped_and_closed(child)


class PlayerIdleTests(unittest.TestCase):
    def player(self, receive, name='Bot', problems=None):
        Player = worlds.scene12.make_player(worlds.Bot, worlds.varint, worlds.parse_varint, worlds.mcstring)
        player = Player.__new__(Player)
        player.name = name
        player.problems = problems
        player.sock = mock.Mock()
        player.threshold = -1
        player.stop = threading.Event()
        player.protocol = 340
        player.receive = receive
        return player

    def test_a_packet_that_fails_to_parse_surfaces(self):
        # A position packet cut short: struct.unpack of its coordinates fails.
        packets = iter([(0x2F, b'short')])
        problems = []
        player = self.player(lambda: next(packets), name='Upgrader', problems=problems)
        with self.assertRaises(struct.error) as caught:
            player.idle()
        self.assertEqual(problems, [('bot Upgrader packet 0x2f', [f'error: {caught.exception}'])])

    def test_a_failure_without_a_problems_list_still_surfaces(self):
        packets = iter([(0x2F, b'short')])
        with self.assertRaises(struct.error):
            self.player(lambda: next(packets)).idle()

    def run_on_thread(self, receive, problems):
        """player.idle() on a real thread, as the keep-alive runs; returns what threading.excepthook saw."""
        seen = []
        player = self.player(receive, name='Upgrader', problems=problems)
        with mock.patch.object(threading, 'excepthook', seen.append):
            thread = threading.Thread(target=player.idle)
            thread.start()
            thread.join(5)
        self.assertFalse(thread.is_alive())
        return seen

    def test_a_handler_failure_on_the_keep_alive_thread_is_recorded_in_problems(self):
        packets = iter([(0x2F, b'short')])
        problems = []
        seen = self.run_on_thread(lambda: next(packets), problems)
        self.assertEqual([hook.exc_type for hook in seen], [struct.error])
        self.assertEqual(problems, [('bot Upgrader packet 0x2f', [f'error: {seen[0].exc_value}'])])

    def test_a_disconnect_on_the_keep_alive_thread_records_nothing(self):
        for error in (EOFError('closed'), ConnectionResetError('reset')):
            with self.subTest(type(error).__name__):
                def receive():
                    raise error
                problems = []
                self.assertEqual(self.run_on_thread(receive, problems), [])
                self.assertEqual(problems, [])

    def test_a_send_timeout_while_handling_a_packet_is_recorded_and_surfaces(self):
        teleport = struct.pack('>dddffB', 1.0, 64.0, 2.0, 0.0, 0.0, 0) + worlds.varint(7)
        for label, packet, pid_text in (
                ('keep-alive', (0x1F, b'12345678'), '0x1f'),
                ('teleport', (0x2F, teleport), '0x2f')):
            with self.subTest(label):
                packets = iter([packet])
                problems = []
                player = self.player(lambda: next(packets), name='Upgrader', problems=problems)
                player.sock.sendall.side_effect = socket.timeout('timed out')
                seen = []
                with mock.patch.object(threading, 'excepthook', seen.append):
                    thread = threading.Thread(target=player.idle)
                    thread.start()
                    thread.join(5)
                self.assertFalse(thread.is_alive())
                self.assertEqual([hook.exc_type for hook in seen], [socket.timeout])
                self.assertEqual(problems, [(f'bot Upgrader packet {pid_text}', [f'{socket.timeout.__name__}: timed out'])])

    def test_an_unexpected_send_disconnect_while_handling_a_packet_is_recorded_and_surfaces(self):
        packets = iter([(0x1F, b'12345678')])
        problems = []
        player = self.player(lambda: next(packets), name='Upgrader', problems=problems)
        player.sock.sendall.side_effect = ConnectionResetError('reset')
        with self.assertRaises(ConnectionResetError):
            player.idle()
        self.assertEqual(problems, [('bot Upgrader packet 0x1f', ['ConnectionResetError: reset'])])

    def test_a_send_disconnect_during_shutdown_ends_the_loop_without_a_problem(self):
        packets = iter([(0x1F, b'12345678')])
        problems = []
        player = self.player(lambda: next(packets), name='Upgrader', problems=problems)

        def send_while_leaving(*args):
            player.stop.set()
            raise ConnectionResetError('reset')

        player.send = mock.Mock(side_effect=send_while_leaving)
        player.idle()
        self.assertEqual(problems, [])

    def test_a_closed_connection_ends_the_loop(self):
        for error in (EOFError('closed'), ConnectionResetError('reset')):
            with self.subTest(type(error).__name__):
                def receive():
                    raise error
                self.player(receive).idle()

    def test_a_receive_timeout_keeps_the_loop_running(self):
        player = self.player(None)
        calls = []

        def receive():
            calls.append(1)
            if len(calls) == 1:
                raise socket.timeout()
            raise EOFError('closed')
        player.receive = receive
        player.idle()
        self.assertEqual(len(calls), 2)


class KnownPacksTests(unittest.TestCase):
    def configure(self, protocol, received):
        """What the bot sends while it configures against a server that sends `received` (id, payload), then finish."""
        bot = worlds.Bot.__new__(worlds.Bot)
        bot.protocol = protocol
        sent = []
        packets = iter([*received, (3, b'')])
        bot.receive = lambda: next(packets)
        bot.send = lambda packet_id, payload=b'': sent.append((packet_id, payload))
        bot.configure()
        return sent

    def test_the_bot_answers_only_the_select_known_packs_packet_of_its_protocol(self):
        # What select_known_packs carries: one pack, namespace minecraft, id core, version.
        offer = b'\x01\x09minecraft\x04core\x041.21'
        answer = (7, b'\x00')
        finish = (3, b'')
        # 26.2: select_known_packs is 14 and update_tags 15. A bare minecraft-shaped payload on 15 is left alone.
        self.assertEqual(self.configure(776, [(15, offer), (14, offer)]), [answer, finish])
        # 26.3: post_effects comes in before it, so update_tags is 14 and select_known_packs 15.
        self.assertEqual(self.configure(777, [(14, offer), (15, offer)]), [answer, finish])

    def test_a_protocol_with_no_known_id_is_refused(self):
        with self.assertRaisesRegex(ValueError, 'protocol 999'):
            self.configure(999, [])


class BotLeaveTests(unittest.TestCase):
    def test_second_leave_skips_the_wait_and_socket_close(self):
        bot = worlds.Bot.__new__(worlds.Bot)
        bot.stop = threading.Event()
        bot.sock = mock.Mock()
        sleep = mock.Mock()
        with mock.patch.object(worlds, 'time', SimpleNamespace(sleep=sleep)):
            bot.leave()
            bot.leave()
        self.assertTrue(bot.stop.is_set())
        sleep.assert_called_once_with(1.5)
        bot.sock.close.assert_called_once_with()


class BotFailedJoinTests(unittest.TestCase):
    def join_against(self, packets, protocol):
        """Run Bot against a local server that sends `packets` (id, payload) once connected; returns the error and the bot's socket."""
        listener = socket.socket()
        self.addCleanup(listener.close)
        listener.bind(('127.0.0.1', 0))
        listener.listen(1)

        def serve():
            peer, _ = listener.accept()
            with peer:
                for packet_id, payload in packets:
                    body = bytes([packet_id]) + payload
                    peer.sendall(bytes([len(body)]) + body)
                peer.settimeout(5)
                try:
                    while peer.recv(4096):
                        pass
                except OSError:
                    pass
        threading.Thread(target=serve, daemon=True).start()
        made = []

        class RecordingSocket:   # worlds' own `socket`: the global module is left alone
            def __getattr__(self, name):
                return getattr(socket, name)

            def create_connection(self, *args, **kwargs):
                made.append(socket.create_connection(*args, **kwargs))
                return made[-1]
        with mock.patch.object(worlds, 'socket', RecordingSocket()):
            with self.assertRaises((RuntimeError, ValueError)) as caught:
                worlds.Bot(listener.getsockname()[1], protocol, 'x')
        self.addCleanup(made[0].close)
        return caught.exception, made[0]

    def test_a_refused_login_closes_the_socket(self):
        error, sock = self.join_against([(0, b'no')], 770)
        self.assertIsInstance(error, RuntimeError)
        self.assertEqual(sock.fileno(), -1)

    def test_a_failed_configuration_closes_the_socket(self):
        # Login succeeds, then the bot has no select_known_packs id for protocol 999 and refuses.
        error, sock = self.join_against([(2, b'')], 999)
        self.assertIsInstance(error, ValueError)
        self.assertEqual(sock.fileno(), -1)


class BuildCleanupTests(unittest.TestCase):
    """build() with the server, the bot and the scene replaced: what is closed when something fails."""

    def build(self, scene_error, stop_error):
        events = []

        class FakeBot:
            def __init__(self, port, protocol, name):
                events.append('joined')

            def leave(self):
                events.append('left')

        class FakeServer:
            def __init__(self, *args):
                pass

            def stop(self, *, save):
                events.append('stopped')
                if stop_error:
                    raise stop_error

        def scene(server, run, Bot, *args):
            Bot(1, 2, 'x')
            if scene_error:
                raise scene_error

        with tempfile.TemporaryDirectory() as temp, mock.patch.object(worlds, 'LOCAL', Path(temp)), \
                mock.patch.object(worlds, 'server_jar', return_value=(Path('server.jar'), 3700, 770)), \
                mock.patch.object(worlds, 'java', return_value='java'), mock.patch.object(worlds, 'Server', FakeServer), \
                mock.patch.object(worlds, 'free_port', return_value=12345), \
                mock.patch.object(worlds, 'Bot', FakeBot), mock.patch.object(worlds, 'build_scene', scene), \
                mock.patch.object(worlds, 'time', SimpleNamespace(sleep=lambda seconds: None)):
            try:
                worlds.build('1.21.5')
            except Exception as error:
                return events, error
        return events, None

    def test_a_scene_that_fails_still_closes_the_bot_and_stops_the_server(self):
        events, error = self.build(ValueError('scene failed'), None)
        self.assertEqual((events, str(error)), (['joined', 'left', 'stopped'], 'scene failed'))

    def test_a_stop_that_raises_reaches_the_caller_and_the_bot_is_closed(self):
        events, error = self.build(None, RuntimeError('stop failed'))
        self.assertEqual((events, str(error)), (['joined', 'left', 'stopped'], 'stop failed'))


class BuildDebugWorldTests(unittest.TestCase):
    """build() of the debug world with the servers replaced: the first server is stopped whatever its spawn command does."""

    def build(self, spawn_command):
        events = []

        class FakeServer:
            def __init__(self, directory, *args):
                events.append(f'started (region present: {(directory / "world" / "region").exists()})')
                (directory / 'world' / 'region').mkdir(parents=True, exist_ok=True)

            def run(self, command):
                return spawn_command(command)

            def stop(self, *, save):
                events.append(f'stopped (save: {save})')

        with tempfile.TemporaryDirectory() as temp, mock.patch.object(worlds, 'LOCAL', Path(temp)), \
                mock.patch.object(worlds, 'server_jar', return_value=(Path('server.jar'), 1631, 404)), \
                mock.patch.object(worlds, 'java', return_value='java'), mock.patch.object(worlds, 'Server', FakeServer), \
                mock.patch.object(worlds, 'free_port', return_value=12345), mock.patch.object(worlds, 'build_scene'), \
                mock.patch.object(worlds, 'time', SimpleNamespace(sleep=lambda seconds: None)):
            try:
                worlds.build('1.13.2-debug')
            except Exception as error:
                return events, error
        return events, None

    def test_a_spawn_command_that_raises_stops_the_first_server_and_keeps_its_error(self):
        def broken(command):
            raise BrokenPipeError('console closed')
        events, error = self.build(broken)
        self.assertEqual(events, ['started (region present: False)', 'stopped (save: False)'])
        self.assertIsInstance(error, BrokenPipeError)

    def test_a_refused_spawn_command_stops_the_first_server_and_is_reported(self):
        events, error = self.build(lambda command: ['Nothing changed'])
        self.assertEqual(events, ['started (region present: False)', 'stopped (save: False)'])
        self.assertIsInstance(error, RuntimeError)
        self.assertIn('setworldspawn was refused', str(error))

    def test_a_set_spawn_saves_on_stop_before_the_regions_are_removed_and_the_server_restarts(self):
        events, error = self.build(lambda command: ['Set the world spawn point to -4000, 70, -4000'])
        self.assertIsNone(error)
        self.assertEqual(events, ['started (region present: False)', 'stopped (save: True)',
                                  'started (region present: False)', 'stopped (save: True)'])


class BuildFlatCleanupTests(unittest.TestCase):
    """build_flat() with the server, the bots and the scene replaced: what is closed when something fails."""

    def build_flat(self, fail_in):
        events = []

        class FakeBot(worlds.Bot):
            def __init__(self, port, protocol, name):
                events.append('joined')
                self.stop = threading.Event()
                self.sock = SimpleNamespace(close=lambda: events.append('left'))

        class FakeServer:
            def __init__(self, *args, **kwargs):
                pass

            def stop(self, *, save):
                events.append('stopped')

        def populate(server, Bot, *args, **kwargs):
            Bot(1, 340, 'x')
            raise ValueError('populate failed')

        def batch(server, commands, problems, **kwargs):
            if fail_in == 'visit' and commands[0].startswith('tp '):
                raise ValueError('visit failed')

        clock = SimpleNamespace(sleep=lambda seconds: None)
        with tempfile.TemporaryDirectory() as temp, mock.patch.object(worlds, 'LOCAL', Path(temp)), \
                mock.patch.object(worlds, 'server_jar', return_value=(Path('server.jar'), 1343, 340)), \
                mock.patch.object(worlds, 'java', return_value='java'), mock.patch.object(worlds, 'Server', FakeServer), \
                mock.patch.object(worlds, 'Bot', FakeBot), mock.patch.object(worlds, 'time', clock), \
                mock.patch.multiple(worlds.scene12, populate=populate, batch=batch, time=clock, free_port=lambda: 12345):
            with self.assertRaises(ValueError) as caught:
                worlds.scene12.build_flat(worlds)
        return events, str(caught.exception)

    def test_a_visit_that_fails_closes_the_visitor_and_stops_the_server(self):
        self.assertEqual(self.build_flat('visit'), (['joined', 'left', 'stopped'], 'visit failed'))

    def test_a_scene_that_fails_closes_every_bot_and_stops_the_server(self):
        events, error = self.build_flat('populate')
        self.assertEqual((events, error), (['joined', 'left', 'joined', 'left', 'stopped'], 'populate failed'))


class LegacyBuildTests(unittest.TestCase):
    def setUp(self):
        work = Path(self.enterContext(tempfile.TemporaryDirectory()))

        def start(*args, cwd, **kwargs):
            (cwd / 'world').mkdir()
            process.returncode = None
            return process

        self.process = process = mock.Mock(stdout=FakeOutput(['Done (\n']), returncode=None)
        process.poll.side_effect = lambda: process.returncode
        process.kill.side_effect = lambda: setattr(process, 'returncode', 1)
        self.stop = stop = mock.Mock()

        class RecordingServer(worlds.Server):
            def stop(self, *args, **kwargs):
                stop(*args, **kwargs)
                super().stop(*args, **kwargs)

        self.enterContext(mock.patch.multiple(
            worlds, LOCAL=work, server_jar=mock.Mock(return_value=(Path('server.jar'), 1343, 340)),
            java=mock.Mock(return_value='java'), free_port=lambda: 12345,
            subprocess=SimpleNamespace(Popen=start, PIPE=subprocess.PIPE, STDOUT=subprocess.STDOUT),
            Server=RecordingServer, time=SimpleNamespace(sleep=lambda seconds: None, time=worlds.time.time)))
        self.lab_actions = mock.Mock()
        self.batch = mock.Mock(return_value=[['Time is 12'], ['Time is 12']])
        self.enterContext(mock.patch.multiple(
            worlds.scene12, time=worlds.time, lab_actions=self.lab_actions, batch=self.batch,
            free_port=lambda: 12345, make_player=lambda *args: lambda *args: mock.Mock()))

    def assert_saved_shutdown(self):
        self.stop.assert_called_once_with(save=False)
        self.process.kill.assert_called_once_with()
        self.process.wait.assert_called_once_with()
        self.process.stdin.close.assert_called_once_with()
        self.assertTrue(self.process.stdout.closed)
        self.process.stdin.write.assert_not_called()  # a graceful stop would save over the last tick

    def test_legacy_world_owner_ends_the_server_once_without_resaving(self):
        worlds.build('1.12.2')
        self.assert_saved_shutdown()

    def test_flat_world_owner_ends_the_server_once_without_resaving(self):
        worlds.scene12.build_flat(worlds)
        self.assert_saved_shutdown()

    def test_older_world_does_not_change_the_next_worlds_origin_or_items(self):
        worlds.server_jar.side_effect = [(Path('server.jar'), 922, 316), (Path('server.jar'), 1343, 340)]
        with mock.patch.multiple(worlds.scene12, spawn_origin=lambda *args: (-32, 96), probe_items=lambda *args: {'stick'}):
            worlds.build('1.11.2')
            worlds.build('1.12.2')
        commands = [command for call in self.batch.call_args_list for command in call.args[1]]
        self.assertEqual([c for c in commands if c.startswith('spawnpoint ')],
                         ['spawnpoint Upgrader -32 100 96', 'spawnpoint Upgrader 144 100 144'])
        self.assertEqual([c for c in commands if c.startswith('replaceitem entity Upgrader slot.hotbar.0 ')],
                         ['replaceitem entity Upgrader slot.hotbar.0 minecraft:stick 1 0',
                          'replaceitem entity Upgrader slot.hotbar.0 minecraft:map 1 0'])

    def test_populate_and_build_flat_give_every_player_the_runs_problems(self):
        made = []
        with mock.patch.object(worlds.scene12, 'make_player', lambda *args: lambda *a: made.append(a) or mock.Mock()):
            worlds.build('1.12.2')
            worlds.scene12.build_flat(worlds)
        problems = [call.args[2] for call in self.batch.call_args_list]
        self.assertEqual(len(made), 3)    # the legacy bot, then the flat world's visitor and scene bot
        for args in made:
            self.assertEqual(len(args), 4)
            self.assertIsInstance(args[3], list)
            self.assertTrue(any(p is args[3] for p in problems))    # the list the run's commands report into
        self.assertEqual([a[3] is made[1][3] for a in made[1:]], [True, True])
        self.assertIsNot(made[0][3], made[1][3])

    def test_lab_actions_oserror_reaches_the_caller(self):
        self.lab_actions.side_effect = ConnectionResetError('lab bot disconnected')
        with self.assertRaisesRegex(ConnectionResetError, 'lab bot disconnected'):
            worlds.build('1.12.2')


if __name__ == '__main__':
    unittest.main()
