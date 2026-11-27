//! The expected streams in `fixtures/` were written by Temurin 25.0.5+8 (its
//! bundled zlib 1.3.2): `.zlib` by `new DeflaterOutputStream(out)` and `.gz` by
//! `new GZIPOutputStream(out)`, each given the whole input in one `write` and
//! closed, from the inputs the functions below build.

use super::Deflater;

/// The kit's typical region chunk: an NBT compound holding 48 KiB of packed block states.
fn chunk() -> Vec<u8> {
    let len = 48 << 10;
    let mut chunk = b"\x0a\x00\x00\x07\x00\x0cblock_states".to_vec();
    chunk.extend_from_slice(&(len as u32).to_be_bytes());
    chunk.extend((0..len).map(|i| ((i / 29) ^ (i & 7) ^ (i >> 12)) as u8 & 31));
    chunk.push(0);
    chunk
}

/// 20 000 random bytes (stored blocks), then a pattern, then zeros: past the
/// 64 KiB window, so the window slides, over several blocks.
fn mixed() -> Vec<u8> {
    let mut x: i32 = 0x2545F491;
    (0..110_000)
        .map(|i: i32| {
            x ^= x << 13;
            x ^= ((x as u32) >> 17) as i32;
            x ^= x << 5;
            if i < 20_000 {
                x as u8
            } else if i < 70_000 {
                ((i / 7) ^ (i >> 9)) as u8
            } else {
                0
            }
        })
        .collect()
}

/// A lazy match at the window's full distance, still pending when the window
/// slides: the 4 bytes at 65273 match those at 32767, 32506 back, and the
/// lone 8 keeps anything earlier from taking them.
fn far_match() -> Vec<u8> {
    let mut far = vec![0; 65277];
    far[32766..32772].copy_from_slice(&[9, 1, 2, 3, 4, 5]);
    far[65272..].copy_from_slice(&[8, 1, 2, 3, 4]);
    far
}

#[test]
fn matches_the_jdk() {
    let cases: [(&str, Vec<u8>, &[u8], &[u8]); 5] = [
        (
            "empty",
            Vec::new(),
            include_bytes!("fixtures/empty.zlib"),
            include_bytes!("fixtures/empty.gz"),
        ),
        (
            "sunrise",
            b"Sunrise".to_vec(),
            include_bytes!("fixtures/sunrise.zlib"),
            include_bytes!("fixtures/sunrise.gz"),
        ),
        (
            "chunk",
            chunk(),
            include_bytes!("fixtures/chunk.zlib"),
            include_bytes!("fixtures/chunk.gz"),
        ),
        (
            "mixed",
            mixed(),
            include_bytes!("fixtures/mixed.zlib"),
            include_bytes!("fixtures/mixed.gz"),
        ),
        (
            "far_match",
            far_match(),
            include_bytes!("fixtures/far_match.zlib"),
            include_bytes!("fixtures/far_match.gz"),
        ),
    ];
    // One deflater for every call: a stream must not depend on the one before.
    let mut deflater = Deflater::new();
    for (name, input, zlib, gzip) in cases {
        let mut out = vec![0xaa];
        deflater.zlib(&input, &mut out);
        assert_eq!(out[0], 0xaa, "{name}: appends");
        assert!(
            out[1..] == *zlib,
            "{name}: zlib stream differs from the JDK's"
        );
        out.clear();
        deflater.gzip(&input, &mut out);
        assert!(out == gzip, "{name}: gzip member differs from the JDK's");
        out.clear();
        deflater.raw(&input, &mut out);
        assert!(out == zlib[2..zlib.len() - 4], "{name}: raw stream differs");
    }
}

#[test]
fn zlib_header_and_adler32_trailer() {
    let mut deflater = Deflater::new();
    for input in [b"".as_slice(), b"hello world", &vec![0x42u8; 4096]] {
        let mut out = Vec::new();
        deflater.zlib(input, &mut out);
        assert!(out.len() >= 6);
        assert_eq!(&out[..2], &[0x78, 0x9c]);
        let expected_adler = simd_adler32::adler32(&input).to_be_bytes();
        assert_eq!(&out[out.len() - 4..], &expected_adler);
    }
}

#[test]
fn gzip_header_crc32_and_isize_trailer() {
    let mut deflater = Deflater::new();
    for input in [b"".as_slice(), b"Sunrise", &vec![b'a'; 10_000]] {
        let mut out = Vec::new();
        deflater.gzip(input, &mut out);
        assert!(out.len() >= 18);
        assert_eq!(&out[..10], &super::GZIP_HEADER);
        let expected_crc = crc32fast::hash(input).to_le_bytes();
        let expected_isize = (input.len() as u32).to_le_bytes();
        assert_eq!(&out[out.len() - 8..out.len() - 4], &expected_crc);
        assert_eq!(&out[out.len() - 4..], &expected_isize);
    }
}

#[test]
fn appends_without_clobbering_prefix() {
    let mut deflater = Deflater::new();
    let input = b"prefix-safe-payload";

    let mut zlib_out = vec![0xde, 0xad];
    deflater.zlib(input, &mut zlib_out);
    assert_eq!(&zlib_out[..2], &[0xde, 0xad]);

    let mut gzip_out = vec![0xbe, 0xef];
    deflater.gzip(input, &mut gzip_out);
    assert_eq!(&gzip_out[..2], &[0xbe, 0xef]);

    let mut raw_out = vec![0xca, 0xfe];
    deflater.raw(input, &mut raw_out);
    assert_eq!(&raw_out[..2], &[0xca, 0xfe]);
}

#[test]
fn deflater_reuse_is_stateless() {
    let mut reused = Deflater::new();
    let mut scratch = Vec::new();
    reused.zlib(&mixed(), &mut scratch);

    let mut reused_out = Vec::new();
    reused.zlib(b"after-large-input", &mut reused_out);

    let mut fresh = Deflater::new();
    let mut fresh_out = Vec::new();
    fresh.zlib(b"after-large-input", &mut fresh_out);

    assert_eq!(reused_out, fresh_out);
}

#[test]
fn incompressible_data_respects_bound() {
    let mut x: u32 = 0x1234_5678;
    let data: Vec<u8> = (0..32_768)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            x as u8
        })
        .collect();
    let mut deflater = Deflater::new();
    let mut out = Vec::new();
    deflater.raw(&data, &mut out);
    assert!(!out.is_empty());
    assert!(out.len() <= super::bound(data.len()));
}
