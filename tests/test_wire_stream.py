"""tools/wire_stream.py cuts a capture's serverbound play stream into client ticks: the comparison of a Sunrise
client's packets with vanilla's (tools/test_wire_stream_parity.py) is only as good as this cut."""
from pathlib import Path
import importlib.util
import struct
import unittest

# Loaded by path: tools/ on sys.path would shadow tests/ modules that share a name with a tools/ harness.
_spec = importlib.util.spec_from_file_location(
    'wire_stream', Path(__file__).resolve().parents[1] / 'tools' / 'wire_stream.py')
wire_stream = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(wire_stream)


def varint(v):
    v &= 0xFFFFFFFF
    out = b''
    while True:
        b = v & 0x7F
        v >>= 7
        if v:
            out += bytes([b | 0x80])
        else:
            return out + bytes([b])


def c2s(name, data=b''):
    return {'conn': 1, 'dir': 'c2s', 'state': 'play', 'name': name, 'data': data.hex()}


def mark():
    text = b'wire-mark'
    return c2s('chat_command', varint(len(text)) + text)


LOGIN = {'conn': 1, 'dir': 's2c', 'state': 'play', 'name': 'login', 'data': struct.pack('>i', 42).hex() + '00'}
TICK_END = c2s('client_tick_end')


class WireStreamTest(unittest.TestCase):
    def test_ticks_end_at_tick_end_and_start_at_the_mark(self):
        entries = [LOGIN, c2s('move_player_rot', struct.pack('>ffB', 1, 2, 1)), TICK_END, mark(),
                   c2s('player_input', b'\x41'), c2s('move_player_pos', struct.pack('>dddB', 0.5, -60, 1.5, 1)),
                   TICK_END, c2s('pong', b'\0\0\0\7'), c2s('attack', varint(7)), c2s('punch'), TICK_END]
        [run] = wire_stream.scenarios(entries)
        self.assertEqual(run[0]['packets'], [
            ('player_input', {'keys': ['forward', 'sprint']}),
            ('move_player_pos', {'x': 0.5, 'y': -60.0, 'z': 1.5, 'on_ground': True, 'horizontal_collision': False}),
            ('client_tick_end', {})])
        # The pong is the frame's, before the tick's packets: never a tick's own, never misplaced.
        self.assertEqual(run[1]['frame'], [('pong', {'raw': '00000007'})])
        self.assertEqual(run[1]['misplaced'], [])
        # Other entities are named by first appearance, the player as `self`.
        self.assertEqual(run[1]['packets'][0], ('attack', {'entity': 'e1'}))

    def test_an_answer_between_two_packets_of_a_tick_is_misplaced(self):
        # The chunk batch acknowledgement is the network thread's (handleChunkBatchFinished): it may land there.
        entries = [LOGIN, mark(), c2s('player_command', varint(42) + varint(1) + varint(0)),
                   c2s('chunk_batch_received', struct.pack('>f', 3.5)), c2s('pong', b'\0\0\0\1'),
                   c2s('move_player_status_only', b'\x01'), TICK_END]
        [run] = wire_stream.scenarios(entries)
        self.assertEqual(run[0]['packets'][0], ('player_command', {'entity': 'self', 'action': 'start_sprinting',
                                                                   'data': 0}))
        self.assertEqual(run[0]['misplaced'], ['pong'])

    def test_a_block_position_decodes_signed_as_vanilla_packs_it(self):
        # BlockPos.asLong: (x & 2^26-1) << 38 | (z & 2^26-1) << 12 | (y & 2^12-1).
        def as_long(x, y, z):
            v = (x & 0x3FFFFFF) << 38 | (z & 0x3FFFFFF) << 12 | (y & 0xFFF)
            return struct.pack('>q', v - (1 << 64) if v >= 1 << 63 else v)
        data = varint(0) + as_long(-2, -61, 3) + varint(1) + struct.pack('>fff', 0.5, 1.0, 0.25) + b'\0\0' + varint(7)
        fields = wire_stream.decode('use_item_on', data)
        self.assertEqual((fields['pos'], fields['face'], fields['sequence']), ([-2, -61, 3], 1, 7))
        self.assertEqual(wire_stream.decode('player_action', varint(0) + as_long(29999999, 2047, -30000000)
                                            + b'\1' + varint(3))['pos'], [29999999, 2047, -30000000])

    def test_a_click_s_state_id_compares_as_how_far_it_lags_the_server(self):
        def set_slot(state):
            return {'conn': 1, 'dir': 's2c', 'state': 'play', 'name': 'container_set_slot',
                    'data': (varint(0) + varint(state) + b'\0\x24\0').hex()}

        def click(state):
            return c2s('container_click', varint(0) + varint(state) + b'\0\x24\0' + varint(0) + b'\0\0')
        # Two runs whose servers counted differently: both clicks are up to date, the third is one behind.
        a = wire_stream.scenarios([LOGIN, set_slot(53), mark(), click(53), TICK_END])
        b = wire_stream.scenarios([LOGIN, set_slot(55), mark(), click(55), TICK_END, set_slot(56), click(55),
                                   TICK_END])
        self.assertEqual(a[0][0]['packets'], b[0][0]['packets'])
        self.assertEqual(a[0][0]['packets'][0][1]['stale'], 0)
        self.assertEqual(b[0][1]['packets'][0][1]['stale'], 1)

    def test_a_late_answer_compares_with_vanilla_s_answer_and_the_script_keeps_its_clock(self):
        tick = lambda *names: {'packets': [(n, {}) for n in names], 'frame': [], 'misplaced': []}  # noqa
        # The server's answer (an abort) reaches vanilla in tick 1 and Sunrise in tick 2; the script acts in tick 3.
        vanilla = [tick('punch'), tick('abort', 'punch'), tick('punch'), tick('pos'), tick()]
        sunrise = [tick('punch'), tick('punch'), tick('abort', 'punch'), tick('pos'), tick()]
        self.assertEqual(wire_stream.compare_answered(vanilla, sunrise, 4, 1, 2), [])
        # What the answer brings is compared: a new dig instead of the abort.
        restart = [tick('punch'), tick('punch'), tick('start', 'punch'), tick('pos'), tick()]
        self.assertEqual([d[0] for d in wire_stream.compare_answered(vanilla, restart, 4, 1, 2)], [1])
        # So is the script's clock: a packet a tick late anywhere else.
        late = [tick('punch'), tick('punch'), tick('abort', 'punch'), tick(), tick('pos')]
        self.assertEqual([d[0] for d in wire_stream.compare_answered(vanilla, late, 4, 1, 2)], [3, 4])

    def test_a_late_answer_on_the_position_reminder_s_tick_keeps_the_reminder_on_the_script_s_clock(self):
        tick = lambda *names: {'packets': [(n, {}) for n in names], 'frame': [], 'misplaced': []}  # noqa
        # #14807: the reminder (sendPosition every 20 ticks) falls in tick 2 for both clients; the answer reaches
        # vanilla in tick 1 and Sunrise in tick 2, the reminder's tick.
        vanilla = [tick('punch'), tick('abort', 'punch'), tick('move_player_pos'), tick()]
        sunrise = [tick('punch'), tick('punch'), tick('abort', 'punch', 'move_player_pos'), tick()]
        self.assertEqual(wire_stream.compare_answered(vanilla, sunrise, 3, 1, 2), [])
        # A reminder a tick early is still a difference, in either answer tick.
        early = [tick('punch'), tick('punch', 'move_player_pos'), tick('abort', 'punch'), tick()]
        self.assertEqual([d[0] for d in wire_stream.compare_answered(vanilla, early, 3, 1, 2)], [1, 2])
        # So is a move the answer brings that vanilla's doesn't, outside the reminder's tick.
        moved = [tick('punch'), tick('punch'), tick('abort', 'punch', 'move_player_pos'), tick('move_player_pos')]
        self.assertEqual([d[0] for d in wire_stream.compare_answered(vanilla, moved, 3, 1, 2)], [3])

    def test_a_replay_names_entities_through_the_server_stream_so_a_different_target_differs(self):
        def add(entity):
            return {'conn': 1, 'dir': 's2c', 'state': 'play', 'name': 'add_entity', 'data': (varint(entity) + b'\0').hex()}
        # Both clients got the same two husks; vanilla hits the first, Sunrise the second. Named per client by first
        # appearance, both would be e1.
        vanilla = [LOGIN, add(7), add(9), mark(), c2s('attack', varint(7)), TICK_END]
        sunrise = [LOGIN, add(7), add(9), mark(), c2s('attack', varint(9)), TICK_END]
        [v], [h] = wire_stream.scenarios(vanilla, exact=True), wire_stream.scenarios(sunrise, exact=True)
        self.assertEqual((v[0]['packets'][0], h[0]['packets'][0]), (('attack', {'entity': 'e1'}),
                                                                     ('attack', {'entity': 'e2'})))
        self.assertEqual([d[0] for d in wire_stream.compare(v, h, 0, key='wire')], [0])

    def test_a_replay_keeps_answers_in_place_and_sequences_as_sent(self):
        use = lambda seq: c2s('use_item', varint(0) + varint(seq) + struct.pack('>ff', 0, 0))  # noqa: E731
        entries = [LOGIN, use(4), TICK_END, mark(), c2s('pong', b'\0\0\0\7'), use(5), TICK_END]
        [run] = wire_stream.scenarios(entries, exact=True)
        self.assertEqual([n for n, _ in run[0]['packets']], ['pong', 'use_item', 'client_tick_end'])
        self.assertEqual(run[0]['packets'][1][1]['sequence'], 5)
        # A pong after the tick's first packet is a different stream, not the same one with a misplaced answer.
        late = [LOGIN, use(4), TICK_END, mark(), use(5), c2s('pong', b'\0\0\0\7'), TICK_END]
        [other] = wire_stream.scenarios(late, exact=True)
        self.assertEqual([d[0] for d in wire_stream.compare(run, other, 0, key='wire')], [0])

    def test_a_replay_compares_floats_by_their_bits(self):
        rot = lambda yaw: [LOGIN, mark(), c2s('move_player_rot', struct.pack('>ffB', yaw, 0, 1)), TICK_END]  # noqa: E731
        [a], [b] = wire_stream.scenarios(rot(0.0), exact=True), wire_stream.scenarios(rot(-0.0), exact=True)
        self.assertEqual(a[0]['packets'], b[0]['packets'])
        [(tick, v, h)] = wire_stream.compare(a, b, 0, key='wire')
        self.assertEqual(wire_stream.field_diff(v, h, wire=(a[0]['wire'], b[0]['wire'])),
                         ['    #0 move_player_rot bytes: vanilla 000000000000000001, Sunrise 800000000000000001'])

    def test_the_mark_is_the_command_not_its_bytes_in_another_packet(self):
        command = lambda text: varint(len(text)) + text  # noqa: E731
        self.assertTrue(wire_stream.mark_packet('chat_command', command(b'wire-mark')))
        self.assertTrue(wire_stream.mark_packet('chat_command', command(b'wire-mark 07')))
        self.assertFalse(wire_stream.mark_packet('chat_command', command(b'wire-mark x')))
        self.assertFalse(wire_stream.mark_packet('chat_command', command(b'wire-markers')))
        self.assertFalse(wire_stream.mark_packet('chat', command(b'wire-mark') + bytes(17)))

    def test_a_replay_s_tick_is_one_ordered_stream_with_the_network_thread_s_answers_in_it(self):
        keep_alive = c2s('keep_alive', bytes(7) + b'\x09')
        batch = lambda rate: c2s('chunk_batch_received', struct.pack('>f', rate))  # noqa: E731
        swing, end = c2s('swing', varint(0)), c2s('client_tick_end')
        [between] = wire_stream.scenarios([LOGIN, mark(), swing, keep_alive, swing, end], exact=True)
        [after] = wire_stream.scenarios([LOGIN, mark(), swing, swing, keep_alive, end], exact=True)
        self.assertEqual([n for n, _ in between[0]['wire']], ['swing', 'keep_alive', 'swing', 'client_tick_end'])
        # The same answer elsewhere among the tick's packets differs, and only in where it sits.
        self.assertEqual([d[0] for d in wire_stream.compare(between, after, 0, key='wire')], [0])
        self.assertTrue(wire_stream.interleaving_only(between[0]['wire'], after[0]['wire']))
        [other] = wire_stream.scenarios([LOGIN, mark(), swing, c2s('swing', varint(1)), keep_alive, end], exact=True)
        self.assertFalse(wire_stream.interleaving_only(between[0]['wire'], other[0]['wire']))
        # A chunk batch's measured rate is left out of the stream, and kept apart.
        [a], [b] = (wire_stream.scenarios([LOGIN, mark(), batch(rate), end], exact=True) for rate in (3.5, 9.0))
        self.assertEqual(wire_stream.compare(a, b, 0, key='wire'), [])
        self.assertNotEqual(a[0]['frame_wire'], b[0]['frame_wire'])

    def test_a_short_capture_never_matches(self):
        tick = lambda: {'packets': [('client_tick_end', {})], 'frame': [], 'misplaced': []}  # noqa: E731
        whole, short = [tick(), tick(), tick()], [tick()]
        self.assertEqual([d[0] for d in wire_stream.compare(whole, short, 1)], [1])
        self.assertEqual([d[0] for d in wire_stream.compare(short, short, 1)], [1])
        self.assertTrue(wire_stream.complete(whole, 1))
        self.assertFalse(wire_stream.complete(whole[:2], 1), 'tick 1 is not closed yet')

    def test_a_timing_record_is_read_whole_for_its_scenarios(self):
        names = ['walk', 'bow']
        record = {'scenarios': names, 'uncertain': {'bow': ['explode reached vanilla 10.0-12 ms before tick 3']}}
        self.assertEqual(wire_stream.timing_doubts(record, names), record['uncertain'])
        for bad in ({'scenarios': ['walk'], 'uncertain': {}}, {'scenarios': names}, [],
                    {'scenarios': names, 'uncertain': {'swim': []}}, {'scenarios': names, 'uncertain': {'bow': 'x'}}):
            with self.subTest(record=bad), self.assertRaises(ValueError):
                wire_stream.timing_doubts(bad, names)

    def test_compare_reports_the_first_field_that_differs(self):
        tick = lambda y: {'packets': [('move_player_pos', {'x': 0.5, 'y': y})], 'frame': [], 'misplaced': []}  # noqa
        diffs = wire_stream.compare([tick(-60.0)], [tick(-60.0000001)], 0)
        self.assertEqual([d[0] for d in diffs], [0])
        self.assertEqual(wire_stream.field_diff(diffs[0][1], diffs[0][2]),
                         ['    #0 move_player_pos.y: vanilla -60.0, Sunrise -60.0000001'])
        self.assertEqual(wire_stream.compare([tick(-60.0)], [tick(-60.0)], 0), [])


if __name__ == '__main__':
    unittest.main()
