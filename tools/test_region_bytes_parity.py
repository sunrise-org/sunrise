#!/usr/bin/env python3
"""Compare Sunrise's anvil region load and rewrite with vanilla 26.3's, byte for byte.

Each case is a small `r.0.0.mca` (and maybe a `c.0.0.mcc`) this script writes: valid records in the four
compressions, an external payload, junk after the root, truncated and corrupt gzip, zlib and LZ4 streams,
unknown codecs, invalid header entries, empty sectors, and the region and compression repros of #308 (items 3,
6, 7 and 8). VanillaRegionBytesOracle runs a copy through the real jar's RegionFileStorage, and the
region_bytes_fixture probe runs another through sunrise_world::region: every listed slot read (absent, loaded
with its NBT, or an error of a class), every loaded chunk written back in place, the file closed. Then the
outputs and every file left in the case's folder must be identical.

Wall-clock time is the one input not under control: vanilla stamps a write with the current second, so the
probe is handed the stamps vanilla wrote. Compounds hold one key each, so that NBT key order (#308 item 2,
decided apart) cannot show up in the bytes.
"""
import argparse
import gzip
import json
import os
from pathlib import Path
import platform
import random
import shutil
import struct
import subprocess
import zlib
try:
    from . import lab, test_rust_parity as base
except ImportError:
    import lab
    import test_rust_parity as base

# The version every case's expected outcome was recorded from (the oracle and the probe both print theirs).
GAME_VERSION = '26.3'
BUILD = lab.LOCAL / 'region-bytes-parity'
SECTOR = 4096
GZIP, ZLIB, NONE, LZ4 = 1, 2, 3, 4


def nbt_string(text):
    data = text.encode()
    return struct.pack('>H', len(data)) + data


def root(key, tag_type, payload, name=''):
    """A root compound holding one entry."""
    return b'\x0a' + nbt_string(name) + bytes([tag_type]) + nbt_string(key) + payload + b'\x00'


def int_root(value, name=''):
    return root('v', 3, struct.pack('>i', value), name)


def bytes_root(data):
    return root('d', 7, struct.pack('>i', len(data)) + data)


def xxh32(data, seed):
    p1, p2, p3, p4, p5 = 2654435761, 2246822519, 3266489917, 668265263, 374761393
    mask = 0xffffffff

    def rotl(x, r):
        return ((x << r) | (x >> (32 - r))) & mask

    def lane(acc, word):
        return rotl((acc + word * p2) & mask, 13) * p1 & mask
    i = 0
    if len(data) >= 16:
        v = [(seed + p1 + p2) & mask, (seed + p2) & mask, seed & mask, (seed - p1) & mask]
        while i + 16 <= len(data):
            v = [lane(v[j], struct.unpack_from('<I', data, i + 4 * j)[0]) for j in range(4)]
            i += 16
        h = (rotl(v[0], 1) + rotl(v[1], 7) + rotl(v[2], 12) + rotl(v[3], 18)) & mask
    else:
        h = (seed + p5) & mask
    h = (h + len(data)) & mask
    while i + 4 <= len(data):
        h = rotl((h + struct.unpack_from('<I', data, i)[0] * p3) & mask, 17) * p4 & mask
        i += 4
    for b in data[i:]:
        h = rotl((h + b * p5) & mask, 11) * p1 & mask
    h = (h ^ (h >> 15)) * p2 & mask
    h = (h ^ (h >> 13)) * p3 & mask
    return h ^ (h >> 16)


def lz4_block(data):
    """lz4-java's LZ4BlockOutputStream framing with one stored (uncompressed) block, then the end block."""
    assert len(data) <= 1 << 16
    checksum = xxh32(data, 0x9747b28c) & 0x0fffffff
    stored = b'LZ4Block' + bytes([0x10 | 6]) + struct.pack('<III', len(data), len(data), checksum) + data
    return stored + b'LZ4Block' + bytes([0x10 | 6]) + struct.pack('<III', 0, 0, 0)


def record(codec, data):
    return struct.pack('>I', len(data) + 1) + bytes([codec]) + data


def region(entries, sectors, length=None, timestamps=None):
    """A region file: `entries` maps slot to (start sector, sector count), `sectors` start sector to bytes."""
    data = bytearray(2 * SECTOR)
    for slot, (start, count) in entries.items():
        struct.pack_into('>I', data, slot * 4, start << 8 | count)
    for slot, stamp in (timestamps or {}).items():
        struct.pack_into('>I', data, SECTOR + slot * 4, stamp)
    for start, payload in sectors.items():
        end = start * SECTOR + len(payload)
        data.extend(b'\x00' * max(0, end - len(data)))
        data[start * SECTOR:end] = payload
    data.extend(b'\x00' * (-len(data) % SECTOR))
    if length is not None:
        data.extend(b'\x00' * max(0, length - len(data)))
    return bytes(data if length is None else data[:length])


def one(codec, data, compression='deflate'):
    """A region holding one record at slot 0, sector 2."""
    payload = record(codec, data)
    return {'r.0.0.mca': region({0: (2, -(-len(payload) // SECTOR))}, {2: payload}, timestamps={0: 1000})}, compression, [0]


def corpus():
    """(name, files, region-file-compression, slots read) per case."""
    rng = random.Random(0x52454749)
    chunk = int_root(-123456)
    yield ('valid-gzip', *one(GZIP, gzip.compress(chunk, mtime=0), 'gzip'))
    yield ('valid-zlib', *one(ZLIB, zlib.compress(chunk), 'deflate'))
    yield ('valid-none', *one(NONE, chunk, 'none'))
    yield ('valid-lz4', *one(LZ4, lz4_block(chunk), 'lz4'))
    yield ('root-name-dropped', *one(ZLIB, zlib.compress(int_root(5, name='chunk'))))
    # Incompressible, so that the rewrite needs 256 sectors or more and goes external again.
    big = zlib.compress(bytes_root(rng.randbytes(1_100_000)), 0)
    stub = struct.pack('>I', 1) + bytes([ZLIB | 0x80])
    inline = record(ZLIB, zlib.compress(int_root(9)))
    yield ('external-mcc', {'r.0.0.mca': region({0: (2, 1), 1: (3, 1)}, {2: stub, 3: inline}), 'c.0.0.mcc': big}, 'deflate', [0, 1])
    yield ('external-missing', {'r.0.0.mca': region({0: (2, 1)}, {2: stub})}, 'deflate', [0])
    # #308 item 6: a byte after the root.
    yield ('junk-after-root', {'r.0.0.mca': region({0: (2, 1)}, {2: bytes.fromhex('00000006030a000000ff')})}, 'deflate', [0])
    # #308 item 3: a root name that is not modified UTF-8, and an unknown root type.
    yield ('root-name-bytes', {'r.0.0.mca': region({0: (2, 1), 1: (3, 1)}, {2: record(NONE, bytes.fromhex('0a0002ffff00')),
                                                                            3: record(NONE, bytes.fromhex('4a1f01'))})}, 'deflate', [0, 1])
    stream = zlib.compress(int_root(77) * 40)
    yield ('truncated-zlib', *one(ZLIB, stream[:len(stream) // 2]))
    yield ('corrupt-zlib-tail', *one(ZLIB, stream[:-4] + bytes(b ^ 0xff for b in stream[-4:])))
    member = gzip.compress(int_root(78), mtime=0)
    yield ('corrupt-gzip-tail', *one(GZIP, member[:-8] + bytes(b ^ 0xff for b in member[-8:-4]) + member[-4:]))
    # #308 item 7: an LZ4Block stream lz4-java refuses.
    yield ('corrupt-lz4', *one(LZ4, bytes.fromhex(
        '4c5a34426c6f636b2620000000270000002817d90bb112cd89673ffd3d11ad97420600708664345bbd2cb70b00248fc80b0020a52b'
        '4c5a34426c6f636b16000000000000000000000000')))
    yield ('unknown-codecs', {'r.0.0.mca': region({0: (2, 1), 1: (3, 1)}, {2: record(9, chunk),
                                                                          3: record(127, nbt_string('sunrise:x') + chunk)})}, 'deflate', [0, 1])
    # #308 item 8, and the other entries vanilla drops when it opens a file: header sectors, no sectors, past
    # the end. Their timestamps are kept; the rewrite of slot 0 rewrites the whole header.
    yield ('invalid-entries', {'r.0.0.mca': region({0: (2, 1), 5: (1, 1), 6: (2, 0), 7: (10, 1)}, {2: record(NONE, chunk)},
                                                   timestamps={0: 1, 5: 5, 6: 6, 7: 7})}, 'deflate', [0, 5, 6, 7])
    yield ('empty-sector', {'r.0.0.mca': region({0: (2, 1)}, {}, length=3 * SECTOR)}, 'deflate', [0])
    yield ('length-past-sectors', {'r.0.0.mca': region({0: (2, 1)}, {2: struct.pack('>I', 5000) + bytes([NONE]) + chunk})}, 'deflate', [0])
    yield ('file-truncated', {'r.0.0.mca': region({0: (2, 2)}, {2: record(NONE, chunk + bytes(SECTOR))}, length=2 * SECTOR + 100)}, 'deflate', [0])
    # A chunk that shrinks frees its sectors for the next one written: the rewrite's allocation order.
    padded = root('d', 7, struct.pack('>i', 5000) + bytes(5000))
    yield ('sector-reuse', {'r.0.0.mca': region({0: (2, 2), 1: (4, 1)}, {2: record(NONE, padded), 4: record(NONE, chunk)},
                                                timestamps={0: 2, 1: 3})}, 'deflate', [0, 1])


def timestamps(path):
    data = path.read_bytes()
    return lambda slot: struct.unpack_from('>I', data, SECTOR + slot * 4)[0] if len(data) >= 2 * SECTOR else 0


def sections(path):
    out, name = {}, None
    for line in path.read_text().splitlines():
        if line.startswith('== '):
            name = line[3:]
            out[name] = []
        elif name is not None:
            out[name].append(line)
    return out


def file_differences(left, right):
    names = sorted({p.name for p in left.iterdir()} | {p.name for p in right.iterdir()})
    differences = []
    for name in names:
        a, b = left / name, right / name
        if not (a.is_file() and b.is_file()):
            differences.append(f'{name}: only in {"vanilla" if a.is_file() else "sunrise"}')
            continue
        x, y = a.read_bytes(), b.read_bytes()
        if x != y:
            at = next((i for i in range(min(len(x), len(y))) if x[i] != y[i]), min(len(x), len(y)))
            differences.append(f'{name}: first difference at byte {at} (sizes {len(x)}, {len(y)}): '
                               f'vanilla {x[at:at + 16].hex()} sunrise {y[at:at + 16].hex()}')
    return differences


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.parse_args()
    lock, jars, libraries = base.verify_artifacts()
    shutil.rmtree(BUILD, ignore_errors=True)
    BUILD.mkdir(parents=True)
    oracle = lab.ROOT / 'mods/sunrise/src/test/java/dev/sunriselab/parity/VanillaRegionBytesOracle.java'
    sources = [Path(__file__), oracle, lab.ROOT / 'tools/lab.py', lab.ROOT / 'tools/test_rust_parity.py', lab.ROOT / 'Cargo.toml',
               lab.ROOT / 'Cargo.lock', lab.ROOT / 'crates/sunrise-world/Cargo.toml', lab.ROOT / 'crates/sunrise-core/src/nbt.rs',
               lab.ROOT / 'crates/sunrise-world/src/region.rs', lab.ROOT / 'crates/sunrise-world/src/compression.rs',
               lab.ROOT / 'crates/sunrise-world/examples/probe/region_bytes_fixture.rs']
    tested = base.source_hashes(sources)
    cases = list(corpus())
    for side in ('vanilla', 'rust'):
        for name, files, _, _ in cases:
            (BUILD / side / name).mkdir(parents=True)
            for file, data in files.items():
                (BUILD / side / name / file).write_bytes(data)

    def run(command, name):
        with (BUILD / (name + '.log')).open('w') as log:
            base.run(list(map(str, command)), stdout=log, stderr=subprocess.STDOUT)
    classes = BUILD / 'classes'
    dependencies = os.pathsep.join(map(str, [jars['server'], *libraries]))
    run(['javac', '--release', lock['minecraft']['java_major'], '-proc:none', '-cp', dependencies, '-d', classes, oracle], 'compile-java')
    (BUILD / 'vanilla.in').write_text(''.join(f'{name} {compression} {" ".join(map(str, slots))}\n' for name, _, compression, slots in cases))
    run(['java', '-Xmx1G', '-cp', str(classes) + os.pathsep + dependencies, 'dev.sunriselab.parity.VanillaRegionBytesOracle',
         BUILD / 'vanilla', BUILD / 'vanilla.in', BUILD / 'vanilla.txt'], 'vanilla')
    lines = []
    for name, _, compression, slots in cases:
        stamp = timestamps(BUILD / 'vanilla' / name / 'r.0.0.mca')
        lines.append(f'{name} {compression} {" ".join(f"{slot}:{stamp(slot)}" for slot in slots)}\n')
    (BUILD / 'rust.in').write_text(''.join(lines))
    run(['cargo', 'build', '--locked', '--profile', 'fast', '-j', os.environ.get('CARGO_BUILD_JOBS', '3'), '-p', 'sunrise-world',
         '--example', 'world_probe'], 'compile-rust')
    binary = lab.cargo_target() / 'fast/examples' / ('world_probe.exe' if os.name == 'nt' else 'world_probe')
    run([binary, 'region_bytes_fixture', BUILD / 'rust', BUILD / 'rust.in', BUILD / 'rust.txt'], 'rust')
    base.ensure_sources_unchanged(sources, tested)

    vanilla, rust = sections(BUILD / 'vanilla.txt'), sections(BUILD / 'rust.txt')
    versions = [(BUILD / side).read_text().splitlines()[0] for side in ('vanilla.txt', 'rust.txt')]
    assert versions == [f'version {GAME_VERSION}'] * 2, versions
    failed = []
    for name, *_ in cases:
        problems = [f'vanilla {a!r}\n      sunrise {b!r}' for a, b in zip(vanilla[name], rust.get(name, [])) if a != b]
        if len(vanilla[name]) != len(rust.get(name, [])):
            problems.append(f'{len(vanilla[name])} vanilla lines, {len(rust.get(name, []))} sunrise lines')
        problems += file_differences(BUILD / 'vanilla' / name, BUILD / 'rust' / name)
        print(f'{"PASS" if not problems else "FAIL"} {name}: ' + '; '.join(line.split(' loaded ')[0] + (' loaded' if ' loaded ' in line else '')
                                                                       for line in vanilla[name]), flush=True)
        for problem in problems:
            print('    ' + problem[:700])
        if problems:
            failed.append(name)
    lab.write_json(BUILD / 'report.json', {
        'schema': 1, 'minecraft': lock['minecraft']['version'], 'game_version': GAME_VERSION, 'platform': platform.platform(),
        'server_jar_sha256': lab.digest(jars['server'], 'sha256'), 'cases': [c[0] for c in cases], 'failed': failed,
        'sources_sha256': tested})
    if failed:
        raise SystemExit(f'{len(failed)} of {len(cases)} region cases differ from vanilla {GAME_VERSION}: {", ".join(failed)}')
    print(f'Region bytes parity: {len(cases)} cases match vanilla {GAME_VERSION} byte for byte')


if __name__ == '__main__':
    main()
