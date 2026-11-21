"""What tools/server_transfer_e2e.py reads off the wire and out of a bot's capture, without a server."""

import struct
import unittest
import uuid

from tools import lab
from tools.server_transfer_e2e import PLAY_TRANSFER, captured, handshake, login_finished, transfers_in
from tools.vanilla_login import string


def record(ms, packet, body):
    return struct.pack('<IiI', ms, packet, len(body)) + body


class CaptureTests(unittest.TestCase):
    def test_finds_the_transfer_and_skips_a_record_cut_short(self):
        data = (record(40, 3, b'\x01\x02')
                + record(5000, PLAY_TRANSFER, string('127.0.0.1') + lab.varint(25566))
                + record(5001, 3, b'\x03')
                + record(5002, PLAY_TRANSFER, string('cut.example'))[:-3])
        self.assertEqual([r[:2] for r in captured(data)], [(40, 3), (5000, PLAY_TRANSFER), (5001, 3)])
        self.assertEqual(transfers_in(data), [('127.0.0.1', 25566)])
        self.assertEqual(transfers_in(b''), [])


class LoginTests(unittest.TestCase):
    def test_reads_the_profile_of_login_finished(self):
        player = lab.offline_uuid('Mover0')
        body = player.bytes + string('Mover0') + lab.varint(0)
        self.assertEqual(login_finished(body), (str(player), 'Mover0'))
        self.assertEqual(uuid.UUID(login_finished(body)[0]), player)

    def test_the_handshake_names_the_intent_last(self):
        packet = handshake(25565, 777, 3)
        self.assertEqual(packet, lab.varint(777) + string('127.0.0.1') + struct.pack('>H', 25565) + b'\x03')


if __name__ == '__main__':
    unittest.main()
