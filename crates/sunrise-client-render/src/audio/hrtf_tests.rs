use super::*;
use sha2::{Digest, Sha256};

fn base() -> Arc<HrtfSet> {
    data_set(48_000).expect("the bundled data set loads")
}

/// The bundled file is OpenAL Soft 1.25.2's `hrtf/Default HRTF.mhr`, unmodified (#3511).
#[test]
fn the_bundled_data_set_is_openal_softs_file_unmodified() {
    assert_eq!(DATA_SET.len(), 159_841);
    let hash: String = Sha256::digest(DATA_SET)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    assert_eq!(
        hash,
        "0b2f09f4d9167dec4e977e7e1a8c78df17e87da06fb0871b9896fadaa473a440"
    );
}

#[test]
fn the_data_set_has_the_layout_the_spec_lists() {
    let set = base();
    assert_eq!(set.rate, 48_000);
    assert_eq!(set.ir_size, 64);
    assert_eq!(set.taps(), 64);
    assert_eq!(
        set.fields,
        [Field {
            distance: 1.4,
            elevations: 19
        }]
    );
    let counts: Vec<usize> = set.elevations.iter().map(|e| e.azimuths).collect();
    assert_eq!(
        counts,
        [
            1, 12, 24, 36, 45, 56, 60, 72, 72, 72, 72, 72, 60, 56, 45, 36, 24, 12, 1
        ]
    );
    assert_eq!(set.coeffs.len(), 828);
    let delays = set.delays.iter().flatten();
    assert_eq!(delays.clone().copied().min(), Some(0));
    assert_eq!(delays.copied().max(), Some(196));
    // Taps past the IR size are 0.
    assert!(
        set.coeffs
            .iter()
            .all(|h| h[64..].iter().all(|t| *t == [0.0; 2]))
    );
}

#[test]
fn the_right_ear_is_the_left_one_mirrored() {
    let set = base();
    for e in &set.elevations {
        let n = e.azimuths;
        for j in 0..n {
            let (from, to) = (e.offset + j, e.offset + (n - j) % n);
            for t in 0..HRIR_LENGTH {
                assert_eq!(
                    set.coeffs[to][t][1].to_bits(),
                    set.coeffs[from][t][0].to_bits()
                );
            }
            assert_eq!(set.delays[to][1], set.delays[from][0]);
        }
    }
}

/// A minimal valid `MinPHR03` file: one field, five elevations of one azimuth, 8 taps.
fn tiny(ir: u8, mm: u16, delay: u8) -> Vec<u8> {
    let mut f = b"MinPHR03".to_vec();
    f.extend(48_000u32.to_le_bytes());
    f.extend([0, ir, 1]);
    f.extend(mm.to_le_bytes());
    f.extend([5, 1, 1, 1, 1, 1]);
    f.extend(std::iter::repeat_n(0u8, 5 * usize::from(ir) * 3));
    f.extend([delay; 5]);
    f
}

#[test]
fn a_data_set_outside_the_formats_limits_is_refused() {
    assert!(HrtfSet::parse(&tiny(8, 1400, 0)).is_ok());
    assert!(HrtfSet::parse(&tiny(7, 1400, 0)).is_err(), "IR size 8..128");
    assert!(
        HrtfSet::parse(&tiny(129, 1400, 0)).is_err(),
        "IR size 8..128"
    );
    assert!(
        HrtfSet::parse(&tiny(8, 49, 0)).is_err(),
        "distance 50..2500"
    );
    assert!(
        HrtfSet::parse(&tiny(8, 2501, 0)).is_err(),
        "distance 50..2500"
    );
    assert!(HrtfSet::parse(&tiny(8, 1400, 252)).is_ok());
    assert!(
        HrtfSet::parse(&tiny(8, 1400, 253)).is_err(),
        "delay up to 63 x 4"
    );
    let mut bad = tiny(8, 1400, 0);
    bad[0] = b'X';
    assert!(HrtfSet::parse(&bad).is_err(), "magic");
    let short = tiny(8, 1400, 0);
    assert!(
        HrtfSet::parse(&short[..short.len() - 1]).is_err(),
        "truncated"
    );
    let mut four = tiny(8, 1400, 0);
    four[18] = 4;
    assert!(HrtfSet::parse(&four).is_err(), "elevation count 5..128");
    // Two fields must be stored farthest first.
    let two = |near: u16, far: u16| {
        let mut f = b"MinPHR03".to_vec();
        f.extend(48_000u32.to_le_bytes());
        f.extend([0, 8, 2]);
        for mm in [far, near] {
            f.extend(mm.to_le_bytes());
            f.extend([5, 1, 1, 1, 1, 1]);
        }
        f.extend(std::iter::repeat_n(0u8, 10 * 8 * 3));
        f.extend([0u8; 10]);
        f
    };
    assert!(HrtfSet::parse(&two(1000, 2000)).is_ok());
    assert!(HrtfSet::parse(&two(2000, 1000)).is_err());
    assert!(HrtfSet::parse(&two(1000, 1000)).is_err());
}

#[test]
fn a_direction_on_the_grid_uses_its_one_response() {
    let set = base();
    let mut f = Filter::default();
    // Straight to the right: elevation 9 (0°), azimuth 18 of 72 (90°).
    set.filter(0.0, std::f32::consts::FRAC_PI_2, 1.0, 0.0, &mut f);
    let h = set.elevations[9].offset + 18;
    assert_eq!(f.coeffs, set.coeffs[h]);
    assert_eq!(
        f.delay,
        set.delays[h].map(|d| (f32::from(d) * 0.25).round_ties_even() as usize)
    );
}

#[test]
fn between_responses_the_four_neighbours_blend() {
    let set = base();
    let mut f = Filter::default();
    // Halfway between azimuths 0 and 1 (5° apart) at elevation 0°, and halfway up to 10°.
    let (az, ev) = (2.5f32.to_radians(), 5f32.to_radians());
    set.filter(ev, az, 1.4, 0.0, &mut f);
    let e = (ev * std::f32::consts::FRAC_1_PI + 0.5) * 18.0;
    let eb = e - 9.0;
    let a = (az * std::f32::consts::FRAC_1_PI * 0.5 + 1.0) * 72.0;
    let ab = a - 72.0;
    let (o0, o1) = (set.elevations[9].offset, set.elevations[10].offset);
    let h = [o0, o0 + 1, o1, o1 + 1];
    let w = [
        (1.0 - eb) * (1.0 - ab),
        (1.0 - eb) * ab,
        eb * (1.0 - ab),
        eb * ab,
    ];
    for t in 0..HRIR_LENGTH {
        for ear in 0..2 {
            let mut c = 0.0f32;
            for k in 0..4 {
                c += set.coeffs[h[k]][t][ear] * w[k];
            }
            assert_eq!(f.coeffs[t][ear].to_bits(), c.to_bits());
        }
    }
}

#[test]
fn a_sound_at_the_listener_plays_from_straight_ahead() {
    let l = Listener::new([5.0, 64.0, -3.0], [0.0, 0.0, -1.0], [0.0, 1.0, 0.0]);
    assert_eq!(
        l.place([5.0, 64.0, -3.0], false),
        Place::Mono {
            elevation: 0.0,
            azimuth: 0.0,
            distance: f32::INFINITY
        }
    );
    assert_eq!(l.place([0.0; 3], true), l.place([5.0, 64.0, -3.0], false));
    // Ahead of a listener facing -z, and to its right (+x).
    let Place::Mono { azimuth, .. } = l.place([5.0, 64.0, -4.0], false) else {
        unreachable!()
    };
    assert_eq!(azimuth, 0.0);
    let Place::Mono { azimuth, .. } = l.place([6.0, 64.0, -3.0], false) else {
        unreachable!()
    };
    assert_eq!(azimuth, std::f32::consts::FRAC_PI_2);
}

/// At 44.1 kHz the set is converted once: IR size `round(64 * 44100 / 48000)` = 59, applied as
/// 60 taps, and the delays scaled.
#[test]
fn another_rate_converts_the_set_once() {
    let set = data_set(44_100).unwrap();
    assert_eq!(set.rate, 44_100);
    assert_eq!(set.ir_size, 59);
    assert_eq!(set.taps(), 60);
    assert!(Arc::ptr_eq(&set, &data_set(44_100).unwrap()), "cached");
    let b = base();
    for (new, old) in set.delays.iter().zip(&b.delays) {
        for ear in 0..2 {
            let d = (f32::from(old[ear]) * 44_100.0 / 48_000.0).round() / 4.0;
            assert_eq!(new[ear], (d * 4.0 + 0.5) as u8);
        }
    }
}

#[test]
fn the_resampler_keeps_a_constant_at_its_level() {
    // A polyphase filter of gain P per phase: a constant input comes out at its level, but
    // for the sinc's tails the 128 samples cut off.
    let r = Resampler::new(48_000, 44_100);
    let input = [1.0f64; HRIR_LENGTH];
    let mut output = [0.0f32; HRIR_LENGTH];
    r.run(&input, &mut output);
    for v in &output[40..80] {
        assert!((v - 1.0).abs() < 1e-2, "{v}");
    }
}

/// A one-tap filter with delay `delay`, for checking the mixing arithmetic.
fn unit(delay: usize, level: f32) -> Filter {
    let mut f = Filter::default();
    f.coeffs[0] = [level, level];
    f.delay = [delay, delay];
    f
}

fn run(
    voice: &mut Voice,
    input: &[f32],
    filter: &Filter,
    gain: f32,
    acc: &mut Accumulator,
) -> Vec<[f32; 2]> {
    let n = input.len();
    let mut inputs = Box::new([[0.0f32; WORK]; 2]);
    inputs[0][HISTORY..HISTORY + n].copy_from_slice(input);
    voice.mix(&mut inputs, &[*filter, *filter], 1, gain, n, 2, acc);
    let mut out = vec![0.0f32; n * 2];
    acc.post(&mut out, n);
    out.chunks(2).map(|f| [f[0], f[1]]).collect()
}

#[test]
fn a_new_voice_starts_on_its_target_and_a_change_crossfades_over_64_samples() {
    let mut voice = Voice::default();
    let mut acc = Accumulator::default();
    let ones = vec![1.0f32; 256];
    // First update: no fade, the target at once.
    let out = run(&mut voice, &ones, &unit(0, 1.0), 0.5, &mut acc);
    assert!(out.iter().all(|f| *f == [0.5, 0.5]));
    // A new gain: the old filter fades out and the new one in over 64 samples.
    let out = run(&mut voice, &ones, &unit(0, 1.0), 1.0, &mut acc);
    for (i, f) in out.iter().enumerate() {
        let want = if i < 64 {
            let old = 0.5 / 64.0 * (64 - i) as f32;
            if i == 0 {
                old
            } else {
                old + 1.0 / 64.0 * i as f32
            }
        } else {
            1.0
        };
        assert_eq!(f[0].to_bits(), want.to_bits(), "sample {i}");
    }
    // Nothing changed: a plain mix.
    let out = run(&mut voice, &ones, &unit(0, 1.0), 1.0, &mut acc);
    assert!(out.iter().all(|f| *f == [1.0, 1.0]));
}

#[test]
fn a_delay_reads_the_history_of_the_last_update() {
    let mut voice = Voice::default();
    let mut acc = Accumulator::default();
    let ramp: Vec<f32> = (1..=100).map(|v| v as f32).collect();
    run(&mut voice, &ramp, &unit(10, 1.0), 1.0, &mut acc);
    let out = run(&mut voice, &ramp, &unit(10, 1.0), 1.0, &mut acc);
    // Ten samples late: the first ten come from the last update's end.
    assert_eq!(out[0][0], 91.0);
    assert_eq!(out[9][0], 100.0);
    assert_eq!(out[10][0], 1.0);
}

#[test]
fn a_stopped_voice_fades_out_over_64_samples() {
    let mut voice = Voice::default();
    let mut acc = Accumulator::default();
    let ones = vec![1.0f32; 256];
    run(&mut voice, &ones, &unit(0, 1.0), 1.0, &mut acc);
    let mut inputs = Box::new([[0.0f32; WORK]; 2]);
    inputs[0][HISTORY..HISTORY + 256].fill(1.0);
    voice.stop(&mut inputs, 1, 256, 2, &mut acc);
    assert!(!voice.playing());
    let mut out = vec![0.0f32; 512];
    acc.post(&mut out, 256);
    for (i, f) in out.chunks(2).enumerate() {
        let want = if i < 64 {
            1.0 / 64.0 * (64 - i) as f32
        } else {
            0.0
        };
        assert_eq!(f[0], want, "sample {i}");
    }
}

#[test]
fn the_accumulator_carries_the_tail_into_the_next_update() {
    let mut voice = Voice::default();
    let mut acc = Accumulator::default();
    let mut f = Filter::default();
    f.coeffs[100] = [1.0, 0.5];
    let mut impulse = vec![0.0f32; 64];
    impulse[0] = 1.0;
    let mut inputs = Box::new([[0.0f32; WORK]; 2]);
    inputs[0][HISTORY..HISTORY + 64].copy_from_slice(&impulse);
    voice.mix(&mut inputs, &[f, f], 1, 1.0, 64, 128, &mut acc);
    let mut out = vec![0.0f32; 128];
    acc.post(&mut out, 64);
    assert!(out.iter().all(|v| *v == 0.0));
    let mut out = vec![0.0f32; 128];
    acc.post(&mut out, 64);
    assert_eq!(&out[72..74], &[1.0, 0.5]);
}
