import struct
import unittest
from tools import test_region_bytes_parity as region


class RegionBytesCorpusTests(unittest.TestCase):
    def test_xxh32_known_values(self):
        # Reference values of the xxHash specification's XXH32.
        self.assertEqual(region.xxh32(b'', 0), 0x02CC5D05)
        self.assertEqual(region.xxh32(b'a', 0), 0x550D7456)
        self.assertEqual(region.xxh32(b'abc', 0), 0x32D153FF)
        self.assertEqual(region.xxh32(b'Nobody inspects the spammish repetition', 0), 0xE2293B2F)

    def test_region_layout(self):
        data = region.region({0: (2, 1), 5: (1, 1)}, {2: b'\x01\x02'}, timestamps={0: 7})
        self.assertEqual(len(data), 3 * region.SECTOR)
        self.assertEqual(data[:4], bytes.fromhex('00000201'))
        self.assertEqual(data[20:24], bytes.fromhex('00000101'))
        self.assertEqual(struct.unpack_from('>I', data, region.SECTOR)[0], 7)
        self.assertEqual(data[2 * region.SECTOR:2 * region.SECTOR + 3], b'\x01\x02\x00')
        self.assertEqual(len(region.region({0: (2, 1)}, {}, length=3 * region.SECTOR)), 3 * region.SECTOR)
        self.assertEqual(len(region.region({0: (2, 2)}, {2: bytes(9000)}, length=2 * region.SECTOR + 100)), 2 * region.SECTOR + 100)

    def test_corpus_is_stable_and_covers_every_codec(self):
        cases = list(region.corpus())
        self.assertEqual([c[0] for c in cases], [c[0] for c in region.corpus()])
        self.assertEqual(len({c[0] for c in cases}), len(cases))
        self.assertEqual({c[2] for c in cases}, {'gzip', 'deflate', 'none', 'lz4'})
        for name, files, _, slots in cases:
            self.assertIn('r.0.0.mca', files, name)
            self.assertTrue(slots, name)
        # A stored LZ4 block holds the data as it is, after its 21-byte header.
        chunk = region.int_root(1)
        self.assertEqual(region.lz4_block(chunk)[21:21 + len(chunk)], chunk)


if __name__ == '__main__':
    unittest.main()
