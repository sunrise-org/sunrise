//! Directional Audio (vanilla's `directionalAudio` option): sounds rendered through the HRTF
//! of the OpenAL Soft build vanilla bundles, as vanilla renders them on a stereo output (spec
//! #3511).
//!
//! - The data set is OpenAL Soft's `Default HRTF.mhr` (its `Built-In HRTF`), shipped unmodified
//!   in `assets/hrtf/` and parsed once, when the first mixer is created ([`data_set`]). At an
//!   output rate other than the set's it is converted once per rate ([`HrtfSet::convert`]).
//! - A mono sound's direction picks four measured responses and blends them, delays included
//!   ([`HrtfSet::filter`]). A sound at the listener plays from straight ahead.
//! - A stereo sound is not placed: its channels play from two fixed virtual speakers at ±30°.
//! - Each voice channel is convolved into one accumulator shared by the whole output, with a
//!   64-sample history; a new direction or gain crossfades over 64 samples, a new voice starts
//!   on its target, and a stopped one fades out ([`Voice`], [`Accumulator`]).
//!
//! Vanilla's request (`Library.createAttributes`) maps onto the mixer: `ALC_HRTF_SOFT` is the
//! option; `ALC_HRTF_ID_SOFT` 0 is the first enumerated set, the built-in one on a clean machine
//! (an `.mhr` a player drops into the game or data directory is user configuration, outside the
//! port); `ALC_OUTPUT_LIMITER_SOFT` is the mixer's limiter, compared with OpenAL Soft's on its own.
//! HRTF needs a stereo output: on any other layout the option-on output is the option-off one.
//! The first-order ambisonic HRTF decoder carries nothing in vanilla (no B-Format or UHJ
//! buffers, no effect slots) and is not built.

use std::sync::{Arc, Mutex, OnceLock};

/// Taps of a response in memory (`HrirLength`); those past the set's IR size are 0.
pub(super) const HRIR_LENGTH: usize = 128;
/// Samples of input each voice channel keeps for its delays (`HrtfHistoryLength`).
pub(super) const HISTORY: usize = 64;
/// The most frames one mixer update renders (`BufferLineSize`): a voice's direction, gain and
/// filters are taken once per update.
pub(super) const UPDATE_FRAMES: usize = 1024;
/// A voice channel's working input: its history, then one update's new samples.
pub(super) const WORK: usize = HISTORY + UPDATE_FRAMES;
/// The crossfade of a parameter change, and a stopping voice's fade-out, in samples.
const FADE: usize = 64;
/// A fade's side is skipped at or below this gain.
const GAIN_SILENCE: f32 = 1e-5;
/// The longest delay, in whole samples.
const MAX_DELAY: u32 = 63;

/// One response: `[left, right]` per tap.
pub(super) type Hrir = [[f32; 2]; HRIR_LENGTH];

/// The data set file's name, and its bytes.
pub(super) const DATA_SET_NAME: &str = "Default HRTF.mhr";
pub(super) static DATA_SET: &[u8] = include_bytes!("../../assets/hrtf/Default HRTF.mhr");

/// One field: its distance (metres) and how many elevations it has.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Field {
    distance: f32,
    elevations: usize,
}

/// One elevation: its azimuth count and where its first response is.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Elevation {
    azimuths: usize,
    offset: usize,
}

/// A loaded data set, at one sample rate.
#[derive(Debug)]
pub(super) struct HrtfSet {
    pub(super) rate: u32,
    /// The responses' length in taps (`IrSize`).
    pub(super) ir_size: usize,
    /// Farthest first.
    fields: Vec<Field>,
    /// Every field's elevations, field after field, each from -90° up to +90°.
    elevations: Vec<Elevation>,
    /// Per response, elevation by elevation, azimuths clockwise from straight ahead.
    coeffs: Vec<Hrir>,
    /// Per response, `[left, right]` in quarter samples.
    delays: Vec<[u8; 2]>,
}

/// One voice channel's filter: the blended response and each ear's delay in whole samples.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Filter {
    pub(super) coeffs: Hrir,
    pub(super) delay: [usize; 2],
}

impl Default for Filter {
    fn default() -> Self {
        Self {
            coeffs: [[0.0; 2]; HRIR_LENGTH],
            delay: [0; 2],
        }
    }
}

/// Reads the `MinPHR03` format, little-endian.
struct Reader<'a> {
    data: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    fn bytes(&mut self, n: usize) -> Result<&'a [u8], String> {
        let end = self.at + n;
        let bytes = self
            .data
            .get(self.at..end)
            .ok_or_else(|| format!("the data set ends early, at byte {}", self.data.len()))?;
        self.at = end;
        Ok(bytes)
    }
    fn u8(&mut self) -> Result<u8, String> {
        Ok(self.bytes(1)?[0])
    }
    fn u16(&mut self) -> Result<u16, String> {
        let b = self.bytes(2)?;
        Ok(u16::from_le_bytes([b[0], b[1]]))
    }
    fn u32(&mut self) -> Result<u32, String> {
        let b = self.bytes(4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }
    /// A signed 24-bit sample, as a float: `v / 8388608.0`.
    fn sample(&mut self) -> Result<f32, String> {
        let b = self.bytes(3)?;
        let v = i32::from_le_bytes([0, b[0], b[1], b[2]]) >> 8;
        Ok(v as f32 / 8_388_608.0)
    }
}

/// `float2uint`: truncates toward zero; a negative value (or NaN) gives 0.
fn float2uint(v: f32) -> usize {
    if v > 0.0 { v as usize } else { 0 }
}

impl HrtfSet {
    /// Parses a `MinPHR03` data set, refusing one outside the format's limits.
    pub(super) fn parse(data: &[u8]) -> Result<Self, String> {
        let mut r = Reader { data, at: 0 };
        if r.bytes(8)? != b"MinPHR03" {
            return Err("not a MinPHR03 data set".into());
        }
        let rate = r.u32()?;
        let stereo = match r.u8()? {
            0 => false,
            1 => true,
            other => return Err(format!("unknown channel type {other}")),
        };
        let ir_size = usize::from(r.u8()?);
        if !(8..=HRIR_LENGTH).contains(&ir_size) {
            return Err(format!("IR size {ir_size} is outside 8..128"));
        }
        let field_count = usize::from(r.u8()?);
        if !(1..=16).contains(&field_count) {
            return Err(format!("field count {field_count} is outside 1..16"));
        }
        let mut fields = Vec::with_capacity(field_count);
        let mut elevations = Vec::new();
        let mut offset = 0;
        let mut last_mm = None;
        for _ in 0..field_count {
            let mm = r.u16()?;
            if !(50..=2500).contains(&mm) {
                return Err(format!("field distance {mm} mm is outside 50..2500"));
            }
            if last_mm.is_some_and(|last| mm >= last) {
                return Err("field distances must decrease".into());
            }
            last_mm = Some(mm);
            let count = usize::from(r.u8()?);
            if !(5..=128).contains(&count) {
                return Err(format!("elevation count {count} is outside 5..128"));
            }
            for _ in 0..count {
                let azimuths = usize::from(r.u8()?);
                if !(1..=128).contains(&azimuths) {
                    return Err(format!("azimuth count {azimuths} is outside 1..128"));
                }
                elevations.push(Elevation { azimuths, offset });
                offset += azimuths;
            }
            fields.push(Field {
                distance: f32::from(mm) / 1000.0,
                elevations: count,
            });
        }
        let total = offset;
        let mut coeffs = vec![[[0.0f32; 2]; HRIR_LENGTH]; total];
        for hrir in &mut coeffs {
            for tap in &mut hrir[..ir_size] {
                tap[0] = r.sample()?;
                if stereo {
                    tap[1] = r.sample()?;
                }
            }
        }
        let mut delays = vec![[0u8; 2]; total];
        for delay in &mut delays {
            for ear in 0..if stereo { 2 } else { 1 } {
                let d = r.u8()?;
                if u32::from(d) > MAX_DELAY * 4 {
                    return Err(format!("delay {d} is over {}", MAX_DELAY * 4));
                }
                delay[ear] = d;
            }
        }
        if !stereo {
            // Left ear only: the right ear is the left one mirrored left to right.
            for e in &elevations {
                let n = e.azimuths;
                for j in 0..n {
                    let to = e.offset + (n - j) % n;
                    let from = e.offset + j;
                    for t in 0..HRIR_LENGTH {
                        coeffs[to][t][1] = coeffs[from][t][0];
                    }
                    delays[to][1] = delays[from][0];
                }
            }
        }
        Ok(Self {
            rate,
            ir_size,
            fields,
            elevations,
            coeffs,
            delays,
        })
    }

    /// The taps the mixer applies: the IR size rounded up to even, as the SSE mixer does.
    pub(super) fn taps(&self) -> usize {
        ((self.ir_size + 1) & !1).min(HRIR_LENGTH)
    }

    /// The set converted to `rate`: every response resampled by a polyphase Kaiser-windowed
    /// sinc in doubles, the delays scaled, the IR size rescaled.
    pub(super) fn convert(&self, rate: u32) -> Self {
        let (src, dst) = (u64::from(self.rate), u64::from(rate));
        let resampler = Resampler::new(src, dst);
        let mut coeffs = vec![[[0.0f32; 2]; HRIR_LENGTH]; self.coeffs.len()];
        let mut input = [0.0f64; HRIR_LENGTH];
        let mut output = [0.0f32; HRIR_LENGTH];
        for (to, from) in coeffs.iter_mut().zip(&self.coeffs) {
            for ear in 0..2 {
                for (i, tap) in input.iter_mut().zip(from) {
                    *i = f64::from(tap[ear]);
                }
                resampler.run(&input, &mut output);
                for (tap, v) in to.iter_mut().zip(output) {
                    tap[ear] = v;
                }
            }
        }
        // Delays: in quarter samples, rescaled, and shrunk together if the longest no longer fits.
        let (srcf, dstf) = (self.rate as f32, rate as f32);
        let scaled: Vec<[f32; 2]> = self
            .delays
            .iter()
            .map(|d| d.map(|old| (f32::from(old) * dstf / srcf).round() / 4.0))
            .collect();
        let longest = scaled.iter().flatten().fold(0.0f32, |a, &b| a.max(b));
        let scale = if longest > MAX_DELAY as f32 {
            MAX_DELAY as f32 / longest
        } else {
            1.0
        };
        let delays = scaled
            .iter()
            .map(|d| d.map(|d| (d * 4.0 * scale + 0.5) as u8))
            .collect();
        let ir_size =
            ((self.ir_size as f64 * dst as f64 / src as f64).round() as usize).min(HRIR_LENGTH);
        Self {
            rate,
            ir_size,
            fields: self.fields.clone(),
            elevations: self.elevations.clone(),
            coeffs,
            delays,
        }
    }

    /// `getCoeffs`: the filter for a sound at `elevation` and `azimuth` (radians; positive
    /// azimuth is to the right, 0 straight ahead), `distance` metres away, with `spread`.
    /// Everything is in float, in the order written.
    pub(super) fn filter(
        &self,
        elevation: f32,
        azimuth: f32,
        distance: f32,
        spread: f32,
        out: &mut Filter,
    ) {
        use std::f32::consts::{FRAC_1_PI, FRAC_1_SQRT_2};
        let dirfact = 1.0 - FRAC_1_PI / 2.0 * spread;
        // The first field (farthest first) no farther than the sound; the last one otherwise.
        let mut field = 0;
        let mut ebase = 0;
        while field + 1 < self.fields.len() && self.fields[field].distance > distance {
            ebase += self.fields[field].elevations;
            field += 1;
        }
        let count = self.fields[field].elevations;
        let e = (elevation * FRAC_1_PI + 0.5) * (count - 1) as f32;
        let ei = float2uint(e);
        let i0 = ei.min(count - 1);
        let eb = e - ei as f32;
        let i1 = (i0 + 1).min(count - 1);
        let azimuth_of = |i: usize| {
            let n = self.elevations[ebase + i].azimuths;
            let a = (azimuth * FRAC_1_PI * 0.5 + 1.0) * n as f32;
            let k = float2uint(a);
            (k % n, a - k as f32)
        };
        let (a0, ab0) = azimuth_of(i0);
        let (a1, ab1) = azimuth_of(i1);
        let e0 = self.elevations[ebase + i0];
        let e1 = self.elevations[ebase + i1];
        let h = [
            e0.offset + a0,
            e0.offset + (a0 + 1) % e0.azimuths,
            e1.offset + a1,
            e1.offset + (a1 + 1) % e1.azimuths,
        ];
        let w = [
            (1.0 - eb) * (1.0 - ab0) * dirfact,
            (1.0 - eb) * ab0 * dirfact,
            eb * (1.0 - ab1) * dirfact,
            eb * ab1 * dirfact,
        ];
        for ear in 0..2 {
            let d = |c: usize| f32::from(self.delays[h[c]][ear]) * w[c];
            let delay = d(0) + d(1) + d(2) + d(3);
            // `cvtss2si` under the mixer's MXCSR: round to nearest, ties to even.
            out.delay[ear] = float2uint((delay * 0.25).round_ties_even());
        }
        out.coeffs = [[0.0; 2]; HRIR_LENGTH];
        let first = FRAC_1_SQRT_2 * (1.0 - dirfact);
        out.coeffs[0] = [first, first];
        for (c, &weight) in w.iter().enumerate() {
            for (coef, src) in out.coeffs.iter_mut().zip(&self.coeffs[h[c]]) {
                coef[0] += src[0] * weight;
                coef[1] += src[1] * weight;
            }
        }
    }
}

/// The polyphase Kaiser-windowed sinc that converts a data set: `P/Q = dst/src` reduced,
/// 180 dB of rejection.
struct Resampler {
    p: usize,
    q: usize,
    /// The filter's half length.
    l: usize,
    filter: Vec<f64>,
}

/// The zeroth-order modified Bessel function of the first kind, summed until it stops changing.
fn bessel_i0(x: f64) -> f64 {
    let mut term = 1.0;
    let mut sum = 1.0;
    let mut k = 1.0;
    loop {
        term *= x / 2.0 / k;
        let next = sum + term * term;
        if next == sum {
            return sum;
        }
        sum = next;
        k += 1.0;
    }
}

fn sinc(x: f64) -> f64 {
    if x.abs() < 1e-9 {
        1.0
    } else {
        (std::f64::consts::PI * x).sin() / (std::f64::consts::PI * x)
    }
}

fn gcd(a: u64, b: u64) -> u64 {
    if b == 0 { a } else { gcd(b, a % b) }
}

impl Resampler {
    fn new(src: u64, dst: u64) -> Self {
        let g = gcd(src, dst).max(1);
        let (p, q) = ((dst / g) as usize, (src / g) as usize);
        let most = p.max(q) as f64;
        let cutoff = 0.47 / most;
        let transition = 0.03 / most;
        let rejection = 180.0;
        let beta = 0.1102 * (rejection - 8.7);
        let order = ((rejection - 7.95) / (2.285 * std::f64::consts::TAU * transition)).ceil();
        let l = (order as usize).div_ceil(2);
        let taps = 2 * l + 1;
        let i0_beta = bessel_i0(beta);
        let filter = (0..taps)
            .map(|i| {
                let x = i as f64 - l as f64;
                let k = x / l as f64;
                let window = if (-1.0..=1.0).contains(&k) {
                    bessel_i0(beta * (1.0 - k * k).sqrt()) / i0_beta
                } else {
                    0.0
                };
                window * 2.0 * p as f64 * cutoff * sinc(2.0 * cutoff * x)
            })
            .collect();
        Self { p, q, l, filter }
    }

    /// Resamples `input` into `output`, both [`HRIR_LENGTH`] long.
    fn run(&self, input: &[f64; HRIR_LENGTH], output: &mut [f32; HRIR_LENGTH]) {
        let (p, q, l) = (self.p, self.q, self.l);
        for (n, out) in output.iter_mut().enumerate() {
            let phase = (l + n * q) % p;
            let at = (l + n * q) / p;
            let mut r = 0.0f64;
            // `k` from 0 while the filter lasts; only inputs in range add.
            let mut k = at.saturating_sub(HRIR_LENGTH - 1);
            while k <= at && phase + k * p < self.filter.len() {
                r += self.filter[phase + k * p] * input[at - k];
                k += 1;
            }
            *out = r as f32;
        }
    }
}

/// `Default HRTF.mhr` at 48 kHz, parsed once.
static BASE: OnceLock<Result<Arc<HrtfSet>, String>> = OnceLock::new();
/// The set converted to other rates, by rate.
static CONVERTED: Mutex<Vec<Arc<HrtfSet>>> = Mutex::new(Vec::new());

/// The data set at `rate`: parsed the first time, converted once per rate and kept.
pub(super) fn data_set(rate: u32) -> Result<Arc<HrtfSet>, String> {
    let base = BASE
        .get_or_init(|| HrtfSet::parse(DATA_SET).map(Arc::new))
        .clone()
        .map_err(|e| format!("{DATA_SET_NAME}: {e}"))?;
    if base.rate == rate || rate == 0 {
        return Ok(base);
    }
    let mut converted = CONVERTED.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(set) = converted.iter().find(|s| s.rate == rate) {
        return Ok(set.clone());
    }
    let set = Arc::new(base.convert(rate));
    converted.push(set.clone());
    Ok(set)
}

/// Listener space (`x` right, `y` up, `z` back) from the listener's position, *at* and *up*
/// vectors, all as floats: `at` and `up` normalized, `up` not made orthogonal.
#[derive(Clone, Copy, Debug)]
pub(super) struct Listener {
    position: [f32; 3],
    right: [f32; 3],
    up: [f32; 3],
    at: [f32; 3],
}

/// Scales `v` to length 1 and returns its length; a vector of squared length up to
/// `FLT_EPSILON` becomes 0, and so does the length.
pub(super) fn normalize(v: &mut [f32; 3]) -> f32 {
    let length_sq = v[0] * v[0] + v[1] * v[1] + v[2] * v[2];
    if length_sq <= f32::EPSILON {
        *v = [0.0; 3];
        return 0.0;
    }
    let length = length_sq.sqrt();
    let inv = 1.0 / length;
    for c in v.iter_mut() {
        *c *= inv;
    }
    length
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// Where a voice's direct sound comes from.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Place {
    /// A mono sound, spatialised: `getCoeffs(elevation, azimuth, distance, 0)`.
    Mono {
        elevation: f32,
        azimuth: f32,
        distance: f32,
    },
    /// A stereo sound: its channels play from the fixed virtual speakers.
    Speakers,
}

impl Listener {
    pub(super) fn new(position: [f32; 3], at: [f32; 3], up: [f32; 3]) -> Self {
        let (mut n, mut v) = (at, up);
        normalize(&mut n);
        normalize(&mut v);
        let mut u = [
            n[1] * v[2] - n[2] * v[1],
            n[2] * v[0] - n[0] * v[2],
            n[0] * v[1] - n[1] * v[0],
        ];
        normalize(&mut u);
        Self {
            position,
            right: u,
            up: v,
            at: n,
        }
    }

    /// Where a mono source at `source` is heard; a relative source's position already is in
    /// listener space.
    pub(super) fn place(&self, source: [f32; 3], relative: bool) -> Place {
        let mut d = if relative {
            source
        } else {
            let p = self.position;
            let d = [source[0] - p[0], source[1] - p[1], source[2] - p[2]];
            [dot(self.right, d), dot(self.up, d), -dot(self.at, d)]
        };
        let distance = normalize(&mut d);
        if distance > f32::EPSILON {
            Place::Mono {
                elevation: d[1].clamp(-1.0, 1.0).asin(),
                azimuth: d[0].atan2(-d[2]),
                distance,
            }
        } else {
            // At the listener: straight ahead, through the farthest field.
            Place::Mono {
                elevation: 0.0f32.asin(),
                azimuth: 0.0f32.atan2(1.0),
                distance: f32::INFINITY,
            }
        }
    }
}

/// The stereo virtual speakers' filters: left at `+π/6`, right at `-π/6` (`AL_STEREO_ANGLES`),
/// each angle `a` at `(-sin a, 0, -cos a)` in listener space.
pub(super) fn speakers(set: &HrtfSet) -> [Filter; 2] {
    [std::f32::consts::FRAC_PI_6, -std::f32::consts::FRAC_PI_6].map(|a| {
        let pos = [-a.sin(), 0.0f32, -a.cos()];
        let mut f = Filter::default();
        set.filter(
            pos[1].asin(),
            pos[0].atan2(-pos[2]),
            f32::INFINITY,
            0.0,
            &mut f,
        );
        f
    })
}

/// The output's HRTF accumulator (`BufferLineSize + HrirLength` stereo pairs): every voice
/// adds into it, and each update hands its first frames to the output.
pub(super) struct Accumulator(Box<[[f32; 2]; UPDATE_FRAMES + HRIR_LENGTH]>);

impl Default for Accumulator {
    fn default() -> Self {
        Self(Box::new([[0.0; 2]; UPDATE_FRAMES + HRIR_LENGTH]))
    }
}

impl Accumulator {
    /// `HrtfPostProcess`: adds the update's `n` frames into the left and right channels of
    /// `out` (interleaved stereo), then moves the tail to the front and clears what follows.
    pub(super) fn post(&mut self, out: &mut [f32], n: usize) {
        let acc = &mut *self.0;
        for (frame, a) in out.as_chunks_mut::<2>().0.iter_mut().zip(&acc[..n]) {
            frame[0] += a[0];
            frame[1] += a[1];
        }
        acc.copy_within(n..n + HRIR_LENGTH, 0);
        acc[HRIR_LENGTH..HRIR_LENGTH + n].fill([0.0; 2]);
    }

    pub(super) fn clear(&mut self) {
        self.0.fill([0.0; 2]);
    }
}

/// Adds one input sample pair, filtered by `coeffs`' first `taps` taps, at `at`.
#[inline(always)]
fn add(acc: &mut [[f32; 2]], at: usize, coeffs: &Hrir, taps: usize, l: f32, r: f32) {
    for (a, c) in acc[at..at + taps].iter_mut().zip(&coeffs[..taps]) {
        a[0] += c[0] * l;
        a[1] += c[1] * r;
    }
}

/// Convolves frames `range` of `input` (the working input: history, then new samples) through
/// `filter` at the gain `gain(i)` gives sample `i`.
#[inline(always)]
fn convolve(
    acc: &mut [[f32; 2]],
    input: &[f32; WORK],
    filter: &Filter,
    taps: usize,
    range: std::ops::Range<usize>,
    gain: impl Fn(usize) -> f32,
) {
    let [dl, dr] = filter.delay;
    for i in range {
        let g = gain(i);
        let l = input[HISTORY - dl + i] * g;
        let r = input[HISTORY - dr + i] * g;
        add(acc, i, &filter.coeffs, taps, l, r);
    }
}

/// One voice channel's state: its input history, the filter and gain it plays at.
#[derive(Clone, Debug)]
struct VoiceChannel {
    history: [f32; HISTORY],
    filter: Filter,
    gain: f32,
}

impl Default for VoiceChannel {
    fn default() -> Self {
        Self {
            history: [0.0; HISTORY],
            filter: Filter::default(),
            gain: 0.0,
        }
    }
}

/// One voice's HRTF state: a channel for each of the sound's channels (a mono sound uses the
/// first), and whether it has played.
#[derive(Clone, Debug, Default)]
pub(super) struct Voice {
    channels: [VoiceChannel; 2],
    playing: bool,
}

impl Voice {
    /// It has played and not stopped: the next update fades from where it is.
    pub(super) fn playing(&self) -> bool {
        self.playing
    }

    /// Forgets everything: the next update starts a new voice.
    pub(super) fn reset(&mut self) {
        if self.playing {
            *self = Self::default();
        }
    }

    /// One update of `n` frames of a playing voice: channel `c`'s new samples are
    /// `inputs[c][HISTORY..HISTORY + n]`, its filter `filters[c]`, all at `gain`. A new voice
    /// starts on its target; a changed filter or gain crossfades over 64 samples.
    pub(super) fn mix(
        &mut self,
        inputs: &mut [[f32; WORK]; 2],
        filters: &[Filter; 2],
        used: usize,
        gain: f32,
        n: usize,
        taps: usize,
        acc: &mut Accumulator,
    ) {
        let fresh = !self.playing;
        for ((ch, input), target) in self.channels.iter_mut().zip(inputs).zip(filters).take(used) {
            input[..HISTORY].copy_from_slice(&ch.history);
            let mut start = 0;
            if fresh || (ch.gain == gain && ch.filter == *target) {
                ch.filter = *target;
                ch.gain = gain;
            } else {
                let counter = n.min(FADE);
                let steps = counter as f32;
                let old = ch.gain;
                if old > GAIN_SILENCE {
                    convolve(&mut *acc.0, input, &ch.filter, taps, 0..counter, |i| {
                        old / steps * (counter - i) as f32
                    });
                }
                if gain > GAIN_SILENCE {
                    convolve(&mut *acc.0, input, target, taps, 1..counter, |i| {
                        gain / steps * i as f32
                    });
                }
                ch.filter = *target;
                ch.gain = gain;
                start = counter;
            }
            if ch.gain != 0.0 {
                let g = ch.gain;
                convolve(&mut *acc.0, input, &ch.filter, taps, start..n, |_| g);
            }
            ch.history.copy_from_slice(&input[n..n + HISTORY]);
        }
        self.playing = true;
    }

    /// A stopping voice: its filters fade to silence over 64 samples of `inputs`' new
    /// samples, its history is not kept, and the voice is done.
    pub(super) fn stop(
        &mut self,
        inputs: &mut [[f32; WORK]; 2],
        used: usize,
        n: usize,
        taps: usize,
        acc: &mut Accumulator,
    ) {
        if !self.playing {
            return;
        }
        let counter = n.min(FADE);
        let steps = counter as f32;
        for (ch, input) in self.channels.iter().zip(inputs).take(used) {
            let old = ch.gain;
            if old > GAIN_SILENCE {
                input[..HISTORY].copy_from_slice(&ch.history);
                convolve(&mut *acc.0, input, &ch.filter, taps, 0..counter, |i| {
                    old / steps * (counter - i) as f32
                });
            }
        }
        *self = Self::default();
    }
}

/// The mixer's HRTF renderer: the data set at the output's rate, the stereo speakers' filters,
/// the accumulators (the output's and a mod muffle's) and scratch, all made when the mixer is,
/// so the output callback never allocates.
pub(super) struct Hrtf {
    pub(super) set: Option<Arc<HrtfSet>>,
    pub(super) speakers: [Filter; 2],
    /// The output's accumulator, and the muffled sounds'.
    pub(super) acc: [Accumulator; 2],
    /// Each voice channel's working input.
    pub(super) inputs: Box<[[f32; WORK]; 2]>,
    /// A voice's filters for one update.
    pub(super) filters: Box<[Filter; 2]>,
    /// The accumulators may hold a tail.
    pub(super) ringing: bool,
}

impl Default for Hrtf {
    fn default() -> Self {
        let mut hrtf = Self {
            set: None,
            speakers: [Filter::default(); 2],
            acc: Default::default(),
            inputs: Box::new([[0.0; WORK]; 2]),
            filters: Box::new([Filter::default(); 2]),
            ringing: false,
        };
        match data_set(48_000) {
            Ok(set) => hrtf.install(set),
            Err(e) => log::warn!("Sunrise: Directional Audio plays without HRTF: {e}"),
        }
        hrtf
    }
}

impl Hrtf {
    /// The data set at `rate`, for [`Hrtf::install`]: off the output's thread, since converting
    /// takes a while.
    pub(super) fn prepare(rate: u32) -> Option<Arc<HrtfSet>> {
        data_set(rate)
            .inspect_err(|e| log::warn!("Sunrise: Directional Audio plays without HRTF: {e}"))
            .ok()
    }

    /// Renders with `set` from now on.
    pub(super) fn install(&mut self, set: Arc<HrtfSet>) {
        if self.set.as_ref().is_some_and(|s| Arc::ptr_eq(s, &set)) {
            return;
        }
        self.speakers = speakers(&set);
        self.set = Some(set);
        self.clear();
    }

    /// Whether the set is loaded at `rate`.
    pub(super) fn ready(&self, rate: u32) -> bool {
        self.set.as_ref().is_some_and(|s| s.rate == rate)
    }

    /// Clears the accumulators' tails.
    pub(super) fn clear(&mut self) {
        for acc in &mut self.acc {
            acc.clear();
        }
        self.ringing = false;
    }
}

#[cfg(test)]
#[path = "hrtf_tests.rs"]
mod tests;
