"""Behavior tests for join_announce, join_probe, net_capture hooks, and Game launch parameters."""
import argparse
import contextlib
import io
import json
import os
import socket
import struct
import subprocess
import sys
import tempfile
import threading
import unittest
from pathlib import Path
from unittest import mock

from tools import join_announce, join_probe, net_capture, server_pack_e2e


NAMES = {
    ('handshake', 'serverbound'): ['intention'],
    ('login', 'clientbound'): ['login_disconnect', 'hello', 'login_finished', 'login_compression', 'custom_query'],
    ('login', 'serverbound'): ['hello', 'key', 'custom_query_answer', 'login_acknowledged'],
    ('configuration', 'clientbound'): ['custom_payload', 'disconnect', 'finish_configuration', 'keep_alive',
                                       'ping', 'store_cookie', 'cookie_request', 'select_known_packs', 'transfer'],
    ('configuration', 'serverbound'): ['client_information', 'cookie_response', 'custom_payload',
                                       'finish_configuration', 'keep_alive', 'pong', 'select_known_packs'],
}


class FrameReader:
    def __init__(self, sock):
        self.sock = sock
        self.buf = bytearray()

    def recv(self, threshold):
        while True:
            try:
                length, pos = net_capture.read_varint(self.buf, 0)
            except IndexError:
                length = pos = None
            if length is not None and len(self.buf) >= pos + length:
                body = bytes(self.buf[pos:pos + length])
                del self.buf[:pos + length]
                if threshold >= 0:
                    import zlib
                    size, p = net_capture.read_varint(body, 0)
                    body = zlib.decompress(body[p:]) if size else body[p:]
                pid, p = net_capture.read_varint(body, 0)
                return pid, body[p:]
            chunk = self.sock.recv(4096)
            if not chunk:
                raise EOFError('connection closed before full frame')
            self.buf.extend(chunk)


def free_local_port():
    with socket.socket() as sock:
        sock.bind(('127.0.0.1', 0))
        return sock.getsockname()[1]


class NetCaptureProbeHookTests(unittest.TestCase):
    def test_hooks_swallow_replies_inject_after_compression_and_record_errors(self):
        with tempfile.TemporaryDirectory() as directory:
            capture = Path(directory) / 'capture.jsonl'
            capture.touch()
            upstream_received = []
            upstream_ready = threading.Event()
            upstream_port = free_local_port()

            def run_upstream():
                with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as srv:
                    srv.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
                    srv.bind(('127.0.0.1', upstream_port))
                    srv.listen(2)
                    srv.settimeout(5)
                    upstream_ready.set()
                    # Connection 1: normal handshake -> login_compression -> client answers injected custom_query
                    # (swallowed) -> client sends login_acknowledged (forwarded).
                    conn, _ = srv.accept()
                    with conn:
                        conn.settimeout(5)
                        reader = FrameReader(conn)
                        upstream_received.append(('c1_handshake', reader.recv(-1)))
                        # Send login_compression (id 3, threshold=16).
                        conn.sendall(join_probe.frame(-1, join_probe.varint(3) + join_probe.varint(16)))
                        # Expect ONLY login_acknowledged (id 3) compressed; custom_query_answer (id 2) must be swallowed.
                        upstream_received.append(('c1_after', reader.recv(16)))
                    # Connection 2: server sends corrupt zlib after compression to verify error event logging.
                    conn2, _ = srv.accept()
                    with conn2:
                        conn2.settimeout(5)
                        reader2 = FrameReader(conn2)
                        reader2.recv(-1)
                        conn2.sendall(join_probe.frame(-1, join_probe.varint(3) + join_probe.varint(1)))
                        bad_body = join_probe.varint(10) + b'not-zlib-bytes'
                        conn2.sendall(join_probe.varint(len(bad_body)) + bad_body)
                        conn2.recv(1)

            t = threading.Thread(target=run_upstream, daemon=True)
            t.start()
            upstream_ready.wait(5)

            listen_port = free_local_port()
            steps = [(('login', 'login_compression'),
                      [('login', 'custom_query', join_probe.varint(7) + join_probe.string('probe:q') + b'hi')])]
            swallow_hook, after_forward = join_probe.make_hooks(steps, NAMES, pause=0)
            stop = net_capture.start_background(listen_port, f'127.0.0.1:{upstream_port}', capture, full=True,
                                                swallow=swallow_hook, after_forward=after_forward, names=NAMES)
            try:
                # Connection 1: handshake (intent=2 login) -> recv login_compression -> recv injected custom_query
                # framed under the new threshold 16 -> send custom_query_answer (swallowed) -> send login_acknowledged.
                with socket.create_connection(('127.0.0.1', listen_port), timeout=5) as cli:
                    cli.settimeout(5)
                    cli_reader = FrameReader(cli)
                    intention = (join_probe.varint(777) + join_probe.string('127.0.0.1')
                                 + struct.pack('>H', listen_port) + join_probe.varint(2))
                    cli.sendall(join_probe.frame(-1, join_probe.varint(0) + intention))
                    comp_id, comp_body = cli_reader.recv(-1)
                    self.assertEqual((comp_id, comp_body), (3, join_probe.varint(16)))
                    # Injected packet must arrive framed with threshold=16.
                    q_id, q_body = cli_reader.recv(16)
                    self.assertEqual(q_id, 4)
                    self.assertEqual(q_body, join_probe.varint(7) + join_probe.string('probe:q') + b'hi')
                    # Send custom_query_answer (id 2, swallowed) then login_acknowledged (id 3, forwarded).
                    cli.sendall(join_probe.frame(16, join_probe.varint(2) + join_probe.varint(7) + b'\x00'))
                    cli.sendall(join_probe.frame(16, join_probe.varint(3)))
                # Connection 2: trigger a zlib error on the clientbound pipe and wait for proxy EOF.
                with socket.create_connection(('127.0.0.1', listen_port), timeout=5) as cli2:
                    cli2.settimeout(5)
                    cli2_reader = FrameReader(cli2)
                    intention = (join_probe.varint(777) + join_probe.string('127.0.0.1')
                                 + struct.pack('>H', listen_port) + join_probe.varint(2))
                    cli2.sendall(join_probe.frame(-1, join_probe.varint(0) + intention))
                    cli2_reader.recv(-1)
                    self.assertEqual(cli2.recv(1), b'')
                t.join(5)
            finally:
                stop.close()

            self.assertEqual(upstream_received[1], ('c1_after', (3, b'')))
            entries = net_capture.load(capture)
            c1 = [e for e in entries if e.get('conn') == 1]
            injected_idx = next(i for i, e in enumerate(c1) if e.get('injected') and e.get('name') == 'custom_query')
            answer_idx = next(i for i, e in enumerate(c1)
                              if e.get('dir') == 'c2s' and e.get('name') == 'custom_query_answer')
            self.assertLess(injected_idx, answer_idx)
            (Path(directory) / 'all.json').write_text(
                json.dumps([e for e in c1 if 'name' in e]), encoding='utf-8',
            )
            attributed = join_probe.answers(directory)
            self.assertEqual(len(attributed), 1)
            self.assertEqual(attributed[0][0][0][:2], ('login', 'custom_query'))
            self.assertTrue(any('login/custom_query_answer' in reply for reply in attributed[0][2]), attributed)
            c2_errors = [e for e in entries if e.get('conn') == 2 and e.get('event') == 'error']
            self.assertTrue(c2_errors, f'expected proxy error event in {entries}')

    def test_stop_close_ends_active_connected_peers_and_closes_capture(self):
        with tempfile.TemporaryDirectory() as directory:
            capture = Path(directory) / 'capture.jsonl'
            capture.touch()
            upstream_ready = threading.Event()
            upstream_received = threading.Event()
            upstream_eof = threading.Event()
            upstream_port = free_local_port()

            def run_upstream():
                with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as srv:
                    srv.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
                    srv.bind(('127.0.0.1', upstream_port))
                    srv.listen(1)
                    srv.settimeout(5)
                    upstream_ready.set()
                    conn, _ = srv.accept()
                    with conn:
                        conn.settimeout(5)
                        FrameReader(conn).recv(-1)
                        upstream_received.set()
                        if conn.recv(1) == b'':
                            upstream_eof.set()

            upstream_thread = threading.Thread(target=run_upstream, daemon=True)
            upstream_thread.start()
            self.assertTrue(upstream_ready.wait(5))

            listen_port = free_local_port()
            stop = net_capture.start_background(listen_port, f'127.0.0.1:{upstream_port}', capture,
                                                full=True, names=NAMES)
            with socket.create_connection(('127.0.0.1', listen_port), timeout=5) as cli:
                cli.settimeout(5)
                intention = (join_probe.varint(777) + join_probe.string('127.0.0.1')
                             + struct.pack('>H', listen_port) + join_probe.varint(2))
                cli.sendall(join_probe.frame(-1, join_probe.varint(0) + intention))
                self.assertTrue(upstream_received.wait(5))
                # Stop the proxy while both client and upstream remain connected and blocked in recv.
                stop.close()
                self.assertFalse(stop._thread.is_alive())
                self.assertEqual(cli.recv(1), b'')
            self.assertTrue(upstream_eof.wait(5))
            upstream_thread.join(5)

            entries = net_capture.load(capture)
            self.assertEqual([e.get('name') for e in entries if 'name' in e], ['intention'])
            self.assertEqual([e for e in entries if e.get('event') == 'error'], [])

    def test_after_forward_skipped_when_packet_swallowed(self):
        forwarded = []
        out = io.StringIO()
        cli_end, proxy_cli = socket.socketpair()
        proxy_up, up_end = socket.socketpair()
        try:
            def swallow(_session, direction, state, name, _payload):
                return direction == 'serverbound' and (state, name) == ('login', 'custom_query_answer')

            def after_forward(_session, direction, state, name, _dst):
                forwarded.append((direction, state, name))

            session = net_capture.Session(1, out, NAMES, threading.Lock(), True,
                                          swallow=swallow, after_forward=after_forward,
                                          sockets=(proxy_cli, proxy_up))
            pipes = (
                net_capture.Pipe(session, 'serverbound', proxy_cli, proxy_up),
                net_capture.Pipe(session, 'clientbound', proxy_up, proxy_cli),
            )
            for pipe in pipes:
                pipe.start()
            intention = (join_probe.varint(777) + join_probe.string('127.0.0.1')
                         + struct.pack('>H', 25565) + join_probe.varint(2))
            cli_end.sendall(join_probe.frame(-1, join_probe.varint(0) + intention))
            cli_end.sendall(join_probe.frame(-1, join_probe.varint(2) + join_probe.varint(7) + b'\x00'))
            cli_end.sendall(join_probe.frame(-1, join_probe.varint(3)))
            cli_end.shutdown(socket.SHUT_WR)
            reader = FrameReader(up_end)
            self.assertEqual(reader.recv(-1)[0], 0)
            self.assertEqual(reader.recv(-1), (3, b''))
            self.assertEqual(up_end.recv(1), b'')
            for pipe in pipes:
                pipe.join()
        finally:
            cli_end.close()
            up_end.close()

        self.assertEqual(forwarded, [
            ('serverbound', 'handshake', 'intention'),
            ('serverbound', 'login', 'login_acknowledged'),
        ])

    def test_injected_row_precedes_immediate_reply_and_failed_send_writes_nothing(self):
        steps = [(('login', 'login_compression'),
                  [('login', 'custom_query', join_probe.varint(7) + join_probe.string('probe:q') + b'hi')])]
        _, after_forward = join_probe.make_hooks(steps, NAMES, pause=0)
        out = io.StringIO()
        session = net_capture.Session(1, out, NAMES, threading.Lock(), True)
        session.state['clientbound'] = session.state['serverbound'] = 'login'
        reply_started = threading.Event()
        reply_done = threading.Event()
        reply_thread = None

        class ImmediateReplyDst:
            def sendall(self, _raw):
                nonlocal reply_thread
                # session.lock must be held during sendall so a concurrent reply cannot log first.
                if session.lock.acquire(blocking=False):
                    session.lock.release()
                    raise AssertionError('session.lock was not held during injected sendall')

                def record_reply():
                    reply_started.set()
                    session.record('serverbound', 2, join_probe.varint(7) + b'\x00')
                    reply_done.set()

                reply_thread = threading.Thread(target=record_reply, daemon=True)
                reply_thread.start()
                reply_started.wait(5)

        after_forward(session, 'clientbound', 'login', 'login_compression', ImmediateReplyDst())
        self.assertIsNotNone(reply_thread)
        reply_thread.join(5)
        self.assertTrue(reply_done.is_set())

        entries = [json.loads(line) for line in out.getvalue().splitlines()]
        self.assertEqual(len(entries), 2)
        self.assertTrue(entries[0].get('injected'))
        self.assertEqual(entries[0]['name'], 'custom_query')
        self.assertEqual((entries[1]['dir'], entries[1]['name']), ('c2s', 'custom_query_answer'))
        with tempfile.TemporaryDirectory() as directory:
            (Path(directory) / 'all.json').write_text(json.dumps(entries), encoding='utf-8')
            attributed = join_probe.answers(directory)
            self.assertEqual(len(attributed), 1)
            self.assertEqual(len(attributed[0][2]), 1)
            self.assertIn('login/custom_query_answer', attributed[0][2][0])

        # A failed send must not append an injected row that could satisfy the deadline.
        _, failing_after_forward = join_probe.make_hooks(steps, NAMES, pause=0)
        fail_out = io.StringIO()
        fail_session = net_capture.Session(1, fail_out, NAMES, threading.Lock(), True)
        fail_session.state['clientbound'] = fail_session.state['serverbound'] = 'login'

        class FailingDst:
            def sendall(self, _raw):
                raise BrokenPipeError('peer closed')

        with self.assertRaises(BrokenPipeError):
            failing_after_forward(fail_session, 'clientbound', 'login', 'login_compression', FailingDst())
        self.assertEqual(fail_out.getvalue(), '')


class JoinProbeTests(unittest.TestCase):
    def test_sign_protocol_ids_reads_prepared_reports_and_rejects_missing(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / 'facts.json').write_text(json.dumps({
                'states': [
                    ['minecraft:air', False, False, 'Block{minecraft:air}', {'id': 'minecraft:air', 'properties': {}}],
                    ['minecraft:oak_sign', False, False, 'rot1', {'id': 'minecraft:oak_sign', 'properties': {'rotation': '1', 'waterlogged': 'false'}}],
                    ['minecraft:oak_sign', False, False, 'rot0w', {'id': 'minecraft:oak_sign', 'properties': {'rotation': '0', 'waterlogged': 'true'}}],
                    ['minecraft:oak_sign', False, False, 'rot0', {'id': 'minecraft:oak_sign', 'properties': {'rotation': '0', 'waterlogged': 'false'}}],
                ],
            }), encoding='utf-8')
            (root / 'registries.json').write_text(json.dumps({
                'minecraft:block_entity_type': {'entries': {'minecraft:chest': {'protocol_id': 1},
                                                            'minecraft:sign': {'protocol_id': 9}}},
            }), encoding='utf-8')
            self.assertEqual(join_probe.sign_protocol_ids(root), (3, 9))

            (root / 'registries.json').write_text(json.dumps({'minecraft:block_entity_type': {'entries': {}}}),
                                                  encoding='utf-8')
            with self.assertRaises(ValueError):
                join_probe.sign_protocol_ids(root)

    def test_diff_pairs_by_probe_without_shifting_and_normalizes_transfer_port(self):
        with tempfile.TemporaryDirectory() as directory:
            a, b = Path(directory) / 'a', Path(directory) / 'b'
            a.mkdir()
            b.mkdir()
            ping1 = struct.pack('>i', 0x1234).hex()
            ping2 = struct.pack('>i', 0x5678).hex()
            t_a = (join_probe.string('127.0.0.1') + join_probe.varint(40001)).hex()
            t_b = (join_probe.string('127.0.0.1') + join_probe.varint(49999)).hex()
            # Recording b is missing the first ping probe; the second ping and transfer must still align cleanly.
            (a / 'all.json').write_text(json.dumps([
                {'dir': 's2c', 'state': 'configuration', 'name': 'ping', 'data': ping1, 'injected': True},
                {'dir': 'c2s', 'state': 'configuration', 'name': 'pong', 'data': ping1},
                {'dir': 's2c', 'state': 'play', 'name': 'ping', 'data': ping2, 'injected': True},
                {'dir': 'c2s', 'state': 'play', 'name': 'pong', 'data': ping2},
                {'dir': 's2c', 'state': 'configuration', 'name': 'transfer', 'data': t_a, 'injected': True},
            ]), encoding='utf-8')
            (b / 'all.json').write_text(json.dumps([
                {'dir': 's2c', 'state': 'play', 'name': 'ping', 'data': ping2, 'injected': True},
                {'dir': 'c2s', 'state': 'play', 'name': 'pong', 'data': ping2},
                {'dir': 's2c', 'state': 'configuration', 'name': 'transfer', 'data': t_b, 'injected': True},
            ]), encoding='utf-8')
            out = io.StringIO()
            with contextlib.redirect_stdout(out), self.assertRaises(SystemExit) as cm:
                join_probe.diff(argparse.Namespace(a=str(a), b=str(b), md=None))
            self.assertEqual(cm.exception.code, 1)
            table = out.getvalue()
            self.assertIn(f'| configuration/ping {ping1} | <- configuration/pong {ping1} | (missing probe) |', table)
            self.assertIn(f'| play/ping {ping2} | <- play/pong {ping2} | (same) |', table)
            self.assertIn('| configuration/transfer host=127.0.0.1 port=(proxy port) | nothing | (same) |', table)
            self.assertIn('| probe count | 3 | 2 |', table)

            # Matching transfer runs with different ports exit 0.
            (a / 'all.json').write_text(json.dumps([
                {'dir': 's2c', 'state': 'configuration', 'name': 'transfer', 'data': t_a, 'injected': True},
            ]), encoding='utf-8')
            (b / 'all.json').write_text(json.dumps([
                {'dir': 's2c', 'state': 'configuration', 'name': 'transfer', 'data': t_b, 'injected': True},
            ]), encoding='utf-8')
            out = io.StringIO()
            with contextlib.redirect_stdout(out), self.assertRaises(SystemExit) as cm:
                join_probe.diff(argparse.Namespace(a=str(a), b=str(b), md=None))
            self.assertEqual(cm.exception.code, 0)
            self.assertIn('(same)', out.getvalue())

    def test_describe_decodes_multibyte_varint_channel(self):
        long_channel = 'probe:' + 'a' * 140
        body = join_probe.string(long_channel) + b'\xde\xad'
        entry = {'state': 'configuration', 'name': 'custom_payload', 'data': body.hex()}
        self.assertEqual(join_probe.describe(entry), f'configuration/custom_payload channel={long_channel} body=dead')

    def test_transfer_key_uses_raw_host_bytes_and_describe_requires_utf8_host(self):
        host_a = {'state': 'configuration', 'name': 'transfer',
                  'data': (b'\x01\x80' + join_probe.varint(40001)).hex()}
        host_b = {'state': 'configuration', 'name': 'transfer',
                  'data': (b'\x01\x81' + join_probe.varint(40001)).hex()}
        host_a_other_port = {'state': 'configuration', 'name': 'transfer',
                             'data': (b'\x01\x80' + join_probe.varint(49999)).hex()}
        host_a_trailing = {'state': 'configuration', 'name': 'transfer',
                           'data': (b'\x01\x80' + join_probe.varint(40001) + b'\xff').hex()}
        self.assertNotEqual(join_probe.probe_key(host_a), join_probe.probe_key(host_b))
        self.assertEqual(join_probe.probe_key(host_a), join_probe.probe_key(host_a_other_port))
        self.assertNotEqual(join_probe.probe_key(host_a), join_probe.probe_key(host_a_trailing))

        with self.assertRaises(UnicodeDecodeError):
            join_probe.describe(host_a)
        bad_intention = {
            'state': 'handshake', 'name': 'intention',
            'data': (join_probe.varint(777) + b'\x01\x80' + struct.pack('>H', 25565) + join_probe.varint(2)).hex(),
        }
        with self.assertRaises(UnicodeDecodeError):
            join_probe.describe(bad_intention)


class JoinAnnounceTests(unittest.TestCase):
    def test_describe_uses_varint_for_long_intention_address_and_channel(self):
        long_host = 'h' * 140 + '.example'
        intention = join_probe.varint(777) + join_probe.string(long_host) + struct.pack('>H', 25565) + join_probe.varint(2)
        self.assertEqual(
            join_announce.describe({'state': 'handshake', 'id': 0, 'name': 'intention', 'data': intention.hex()}),
            f'handshake/intention (id 0) protocol=777 address={long_host}',
        )
        long_channel = 'mod:' + 'c' * 140
        payload = join_probe.string(long_channel) + b'\x01\x02'
        self.assertEqual(
            join_announce.describe({'state': 'configuration', 'id': 2, 'name': 'custom_payload', 'data': payload.hex()}),
            f'configuration/custom_payload (id 2) channel={long_channel} body=0102',
        )

    @unittest.skipIf(os.name == 'nt', 'Xvfb -displayfd pipe uses POSIX select')
    def test_read_display_reads_number_and_refuses_dead_or_empty_xvfb(self):
        read_fd, write_fd = os.pipe()
        os.write(write_fd, b'42\n')
        os.close(write_fd)
        proc = subprocess.Popen([sys.executable, '-c', 'pass'])
        try:
            self.assertEqual(join_announce._read_display(read_fd, proc), ':42')
        finally:
            os.close(read_fd)
            proc.wait()

        # Immediate exit with no output must not hang or return ':'.
        read_fd, write_fd = os.pipe()
        proc = subprocess.Popen([sys.executable, '-c', 'import sys; sys.exit(1)'], pass_fds=(write_fd,))
        os.close(write_fd)
        try:
            with self.assertRaises(RuntimeError):
                join_announce._read_display(read_fd, proc)
        finally:
            os.close(read_fd)
            proc.wait()

        # stop_process terminates and reaps a running process.
        proc = subprocess.Popen([sys.executable, '-c', 'import time; time.sleep(60)'])
        join_announce.stop_process(proc)
        self.assertIsNotNone(proc.poll())

    def test_output_folder_refuses_strays_and_cleans_owned_files(self):
        with tempfile.TemporaryDirectory() as directory:
            out = Path(directory) / 'out'
            out.mkdir()
            (out / 'capture.jsonl').write_text('old\n', encoding='utf-8')
            cleaned = join_announce.output_folder(out, ('capture.jsonl', 'serverbound.json'))
            self.assertEqual(cleaned, out)
            self.assertFalse((out / 'capture.jsonl').exists())

            (out / 'foreign.txt').write_text('keep\n', encoding='utf-8')
            with self.assertRaises(SystemExit):
                join_announce.output_folder(out, ('capture.jsonl',))
            self.assertTrue((out / 'foreign.txt').exists())

    def test_diff_dedups_play_packets_and_reports_exit_code(self):
        with tempfile.TemporaryDirectory() as directory:
            a, b = Path(directory) / 'a', Path(directory) / 'b'
            a.mkdir()
            b.mkdir()
            packets_a = [
                {'state': 'play', 'id': 1, 'name': 'client_tick_end', 'data': ''},
                {'state': 'play', 'id': 1, 'name': 'client_tick_end', 'data': ''},
            ]
            packets_b = [
                {'state': 'play', 'id': 1, 'name': 'client_tick_end', 'data': ''},
            ]
            (a / 'serverbound.json').write_text(json.dumps(packets_a), encoding='utf-8')
            (b / 'serverbound.json').write_text(json.dumps(packets_b), encoding='utf-8')
            out = io.StringIO()
            with contextlib.redirect_stdout(out), self.assertRaises(SystemExit) as cm:
                join_announce.diff(argparse.Namespace(a=str(a), b=str(b), md=None))
            self.assertEqual(cm.exception.code, 0)

    @unittest.skipIf(os.name == 'nt', 'stub sunrise-ctl shell script uses POSIX sh')
    def test_game_accepts_headless_mode_and_extra_env(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            bins = root / 'bins'
            bins.mkdir()
            recorded = root / 'recorded.json'
            ctl = bins / 'sunrise-ctl'
            ctl.write_text(
                f'#!/usr/bin/env python3\n'
                f'import json, os, sys\n'
                f'with open({str(recorded)!r}, "w", encoding="utf-8") as fh:\n'
                f'    fh.write(json.dumps({{"argv": sys.argv[1:], '
                f'"telemetry": os.environ.get("SUNRISE_TELEMETRY_URL"), '
                f'"home": os.environ.get("SUNRISE_HOME")}}))\n'
                f'print(json.dumps({{"pid": 0}}))\n',
                encoding='utf-8',
            )
            ctl.chmod(0o755)
            out = io.StringIO()
            with contextlib.redirect_stdout(out):
                game = join_announce.headless_game(bins, root / 'res', root / 'work')
            info = json.loads(recorded.read_text(encoding='utf-8'))
            self.assertIn('--headless', info['argv'])
            self.assertNotIn('--offscreen', info['argv'])
            self.assertEqual(info['telemetry'], 'off')
            self.assertEqual(info['home'], str(root / 'work' / 'home'))
            self.assertEqual(game.log, root / 'work' / 'home' / 'logs' / 'latest.log')


class RecordTeardownTests(unittest.TestCase):
    def _run_record(self, mod, *, fail_disconnect):
        events = []
        work_dirs = []
        proxy_closed = False
        snapshot_saw_closed = []

        class FakeServer:
            def __init__(self, *_args, **_kwargs):
                pass

            def wait_for(self, *_args, **_kwargs):
                pass

            def stop(self):
                events.append('server_stop')

        class FakeStop:
            def close(self):
                nonlocal proxy_closed
                proxy_closed = True
                events.append('proxy_close')

        class FakeGame:
            def __call__(self, *words, ok=True):
                if words == ('disconnect',):
                    events.append('disconnect')
                    if fail_disconnect:
                        raise RuntimeError('disconnect failed')
                return {}

            def quit(self):
                events.append('quit')

        def fake_headless(_bins, _resources, game_work):
            work_dirs.append(game_work.parent)
            return FakeGame()

        real_load = net_capture.load

        def tracking_load(path):
            snapshot_saw_closed.append(proxy_closed)
            return real_load(path)

        with tempfile.TemporaryDirectory() as directory:
            out_dir = Path(directory) / 'out'
            if mod is join_announce:
                args = argparse.Namespace(client='sunrise', bin=directory, out=str(out_dir))
                ready_rows = [
                    {'conn': 1, 'dir': 's2c', 'state': 'play', 'id': 0, 'name': 'login', 'data': ''},
                    {'conn': 1, 'dir': 'c2s', 'state': 'play', 'id': 1, 'name': 'client_tick_end', 'data': ''},
                ]
            else:
                args = argparse.Namespace(client='sunrise', bin=directory, out=str(out_dir),
                                          scenario='sunrise-signals')
                ready_rows = [
                    {'conn': 1, 'dir': 's2c', 'state': 'configuration', 'id': 0, 'name': 'custom_payload',
                     'data': '', 'injected': True},
                    {'conn': 1, 'dir': 's2c', 'state': 'configuration', 'id': 0, 'name': 'custom_payload',
                     'data': '', 'injected': True},
                ]

            def fake_start(_listen, _target, capture, **_kwargs):
                Path(capture).write_text(''.join(json.dumps(r) + '\n' for r in ready_rows), encoding='utf-8')
                return FakeStop()

            with contextlib.ExitStack() as patches:
                patches.enter_context(mock.patch.dict(os.environ, {}, clear=False))
                patches.enter_context(mock.patch.object(mod.base, 'verify_artifacts',
                                                        return_value=(None, {'server': 's.jar'}, [])))
                patches.enter_context(mock.patch.object(mod, 'Server', FakeServer))
                patches.enter_context(mock.patch.object(mod, 'headless_game', fake_headless))
                patches.enter_context(mock.patch.object(mod, 'free_port', side_effect=[40001, 40002]))
                patches.enter_context(mock.patch.object(mod.net_capture, 'start_background', fake_start))
                patches.enter_context(mock.patch.object(mod.net_capture, 'load', tracking_load))
                patches.enter_context(mock.patch.object(mod.time, 'sleep', lambda _s: None))
                if mod is join_probe:
                    patches.enter_context(mock.patch.object(mod.net_capture, 'load_names', return_value=NAMES))
                out = io.StringIO()
                with contextlib.redirect_stdout(out):
                    if fail_disconnect:
                        with self.assertRaisesRegex(RuntimeError, 'disconnect failed'):
                            mod.record(args)
                    else:
                        mod.record(args)

            self.assertEqual(events, ['disconnect', 'quit', 'proxy_close', 'server_stop'])
            self.assertEqual(len(work_dirs), 1)
            self.assertFalse(work_dirs[0].exists())
            if not fail_disconnect:
                self.assertTrue(snapshot_saw_closed[-1])

    def test_record_runs_all_cleanups_on_disconnect_error_and_closes_proxy_before_snapshot(self):
        for mod in (join_announce, join_probe):
            for fail_disconnect in (True, False):
                with self.subTest(mod=mod.__name__, fail_disconnect=fail_disconnect):
                    self._run_record(mod, fail_disconnect=fail_disconnect)


if __name__ == '__main__':
    unittest.main()
