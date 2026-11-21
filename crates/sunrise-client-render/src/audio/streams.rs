//! Mods' audio streams (`ClientHost::open_audio_stream`) in the mixer: a pool of their own
//! (never vanilla's streaming slots), each a fixed ring a mod pushes PCM into from any thread and
//! the output callback reads, resampled to the device rate.
//!
//! Who touches what:
//! - A mod thread touches only its stream's own atomics: `push` writes the ring, the controls
//!   write the volume and the placement. None of it takes a lock the callback takes.
//! - The callback touches the mixer's voices: it reads each ring, the volume and the placement
//!   (a seqlock, so it never waits for a writer) and allocates nothing.
//! - The main thread owns the pool: it opens, caps, follows entities, unplaces and reaps. The
//!   ring is allocated before the mixer is locked, and freed after a stopped stream's voice has
//!   left the mixer: a handle holds only a weak reference to its ring.
use super::{Basis, Mixer, distance_gain};
use mod_api::{
    AUDIO_STREAM_PREBUFFER_MAX_MS, AUDIO_STREAM_PREBUFFER_MIN_MS, AUDIO_STREAM_RATE_MAX,
    AUDIO_STREAM_RATE_MIN, AUDIO_STREAM_RING_MAX_MS, AUDIO_STREAMS_PER_MOD, AUDIO_STREAMS_TOTAL,
    AudioRefusal, AudioStreamSource, AudioStreamState, EntityRef, SpecFault, StreamPlacement,
    StreamSpec,
};
use std::sync::{
    Arc, Mutex, MutexGuard, Weak,
    atomic::{
        AtomicBool, AtomicI16, AtomicU8, AtomicU32, AtomicU64, AtomicUsize, Ordering::*, fence,
    },
};

/// How far from the origin a position may lie, in blocks (the world border's limit).
const POSITION_LIMIT: f64 = 30_000_000.0;

/// A stream's samples: written by one pusher, read by the output callback.
pub(super) struct Ring {
    samples: Box<[AtomicI16]>,
    /// Samples written and consumed since the stream opened; the index is the count modulo the
    /// length. Only the one pusher moves `tail`, only the callback moves `head`.
    head: AtomicU64,
    tail: AtomicU64,
    channels: usize,
}

impl Ring {
    fn new(frames: usize, channels: usize) -> Self {
        Self {
            samples: (0..frames * channels).map(|_| AtomicI16::new(0)).collect(),
            head: AtomicU64::new(0),
            tail: AtomicU64::new(0),
            channels,
        }
    }
    /// The frames waiting. `head` is read first: it never passes a `tail` read after it, so the
    /// difference cannot go negative however the producer and the callback interleave with this
    /// read, and it is at most the ring (a stale `head` may make it look fuller than it is).
    fn buffered_frames(&self) -> u64 {
        let head = self.head.load(Acquire);
        let tail = self.tail.load(Acquire);
        tail.saturating_sub(head).min(self.samples.len() as u64) / self.channels as u64
    }
}

/// Where a stream was last placed, as writers see it.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Held {
    Flat,
    At,
    Entity(EntityRef),
    /// A dimension or level change took the placement away: silent until the next `set_placement`.
    Cleared,
}

/// What the callback needs of a stream's placement, one consistent snapshot.
#[derive(Clone, Copy, Default)]
struct View {
    positional: bool,
    placed: bool,
    at: [f64; 3],
}

/// A stream's state shared by its handle, the pool and its voice.
pub(super) struct Control {
    channels: usize,
    rate: u32,
    prebuffer_frames: u64,
    /// The stream's `SoundSource` index.
    category: usize,
    range: f32,
    owner: String,
    volume: AtomicU32,
    /// Prebuffered and playing, as against buffering.
    playing: AtomicBool,
    /// Times the stream ran dry and went back to buffering.
    underruns: AtomicU32,
    closed: AtomicBool,
    /// Another thread is pushing: this push is refused, which keeps the ring single-producer.
    pushing: AtomicBool,
    /// The placement as set. Writers (a mod's `set_placement`, the frame's entity follow, a
    /// level change) take it for the whole of a change, so the newest always wins; the callback
    /// never does.
    held: Mutex<Held>,
    /// The placement the callback reads, published under `held` as a seqlock: odd while a
    /// writer is in the middle.
    sequence: AtomicU64,
    flags: AtomicU8,
    position: [AtomicU64; 3],
}

const POSITIONAL: u8 = 1;
const PLACED: u8 = 2;

impl Control {
    fn publish(&self, _held: &MutexGuard<'_, Held>, view: View) {
        let start = self.sequence.load(Relaxed);
        self.sequence.store(start + 1, Relaxed);
        fence(Release);
        for (cell, v) in self.position.iter().zip(view.at) {
            cell.store(v.to_bits(), Relaxed);
        }
        let flags = (u8::from(view.positional) * POSITIONAL) | (u8::from(view.placed) * PLACED);
        self.flags.store(flags, Relaxed);
        self.sequence.store(start + 2, Release);
    }
    /// The placement as the last writer left it, or none when a writer was in the middle of a
    /// change all four tries (the callback then keeps what it last had).
    fn view(&self) -> Option<View> {
        for _ in 0..4 {
            let before = self.sequence.load(Acquire);
            if before % 2 == 1 {
                continue;
            }
            let at = [0, 1, 2].map(|i| f64::from_bits(self.position[i].load(Relaxed)));
            let flags = self.flags.load(Relaxed);
            fence(Acquire);
            if self.sequence.load(Relaxed) == before {
                return Some(View {
                    positional: flags & POSITIONAL != 0,
                    placed: flags & PLACED != 0,
                    at,
                });
            }
        }
        None
    }
    fn lock(&self) -> MutexGuard<'_, Held> {
        self.held.lock().unwrap_or_else(|e| e.into_inner())
    }
    fn place(&self, placement: &StreamPlacement) {
        let mut held = self.lock();
        let old = self.view().unwrap_or_default();
        match *placement {
            StreamPlacement::Flat => {
                *held = Held::Flat;
                self.publish(&held, View::default());
            }
            StreamPlacement::At(at) => {
                *held = Held::At;
                self.publish(
                    &held,
                    View {
                        positional: true,
                        placed: true,
                        at,
                    },
                );
            }
            StreamPlacement::Entity(entity) => {
                *held = Held::Entity(entity);
                self.publish(
                    &held,
                    View {
                        positional: true,
                        placed: false,
                        at: old.at,
                    },
                );
            }
        }
    }
    /// Places an entity-following stream at its entity's position as `resolve` finds it now,
    /// under the lock a `set_placement` takes: an older resolution can never overwrite a newer
    /// placement.
    fn follow(&self, resolve: &mut impl FnMut(EntityRef) -> Option<[f64; 3]>) {
        let held = self.lock();
        let Held::Entity(entity) = *held else {
            return;
        };
        let old = self.view().unwrap_or_default();
        match resolve(entity).filter(|at| at.iter().all(|c| c.is_finite())) {
            Some(at) => self.publish(
                &held,
                View {
                    positional: true,
                    placed: true,
                    at,
                },
            ),
            None => self.publish(
                &held,
                View {
                    positional: true,
                    placed: false,
                    at: old.at,
                },
            ),
        }
    }
    fn unplace(&self) {
        let mut held = self.lock();
        if *held != Held::Flat {
            *held = Held::Cleared;
            let old = self.view().unwrap_or_default();
            self.publish(
                &held,
                View {
                    positional: true,
                    placed: false,
                    at: old.at,
                },
            );
        }
    }
}

fn check_placement(channels: u8, placement: &StreamPlacement) -> Result<(), SpecFault> {
    match placement {
        StreamPlacement::Flat => Ok(()),
        _ if channels != 1 => Err(SpecFault::StereoPositioned),
        StreamPlacement::At(at)
            if at
                .iter()
                .any(|v| !v.is_finite() || v.abs() > POSITION_LIMIT) =>
        {
            Err(SpecFault::Position)
        }
        _ => Ok(()),
    }
}

/// A mod's end of a stream: what `AudioStreamRef` calls. Dropping it stops the stream. It holds
/// its ring weakly, so a stopped stream's samples are freed once the mixer lets go of its voice,
/// whatever handles the mod keeps.
pub(super) struct StreamHandle {
    control: Arc<Control>,
    ring: Weak<Ring>,
}

impl AudioStreamSource for StreamHandle {
    fn push(&self, samples: &[i16]) -> u32 {
        let c = &*self.control;
        if c.closed.load(Acquire) || c.pushing.swap(true, Acquire) {
            return 0;
        }
        let accepted = match self.ring.upgrade() {
            Some(ring) => {
                let channels = ring.channels;
                let len = ring.samples.len() as u64;
                let tail = ring.tail.load(Relaxed);
                let free = (len - (tail - ring.head.load(Acquire))) / channels as u64;
                let frames = (samples.len() / channels).min(free as usize);
                for (i, sample) in samples[..frames * channels].iter().enumerate() {
                    ring.samples[((tail + i as u64) % len) as usize].store(*sample, Relaxed);
                }
                ring.tail.store(tail + (frames * channels) as u64, Release);
                frames as u32
            }
            None => 0,
        };
        c.pushing.store(false, Release);
        accepted
    }
    fn set_placement(&self, placement: &StreamPlacement) -> bool {
        let c = &*self.control;
        if c.closed.load(Acquire) || check_placement(c.channels as u8, placement).is_err() {
            return false;
        }
        c.place(placement);
        true
    }
    fn set_volume(&self, volume: f32) -> bool {
        if self.control.closed.load(Acquire) || !(0.0..=1.0).contains(&volume) {
            return false;
        }
        self.control.volume.store(volume.to_bits(), Relaxed);
        true
    }
    fn state(&self) -> AudioStreamState {
        let c = &*self.control;
        if c.closed.load(Acquire) {
            return AudioStreamState::Stopped;
        }
        if !c.playing.load(Acquire) {
            return AudioStreamState::Buffering;
        }
        // One atomic holds both flags, so what this reads is a whole placement even while a writer
        // is in the middle of publishing the position: the old one or the new.
        let flags = c.flags.load(Acquire);
        if flags & POSITIONAL != 0 && flags & PLACED == 0 {
            AudioStreamState::Unplaced
        } else {
            AudioStreamState::Playing
        }
    }
    fn buffered_ms(&self) -> u32 {
        match self.ring.upgrade() {
            Some(ring) if !self.control.closed.load(Acquire) => {
                (ring.buffered_frames() * 1000 / u64::from(self.control.rate)) as u32
            }
            _ => 0,
        }
    }
    fn stop(&self) {
        self.control.closed.store(true, Release);
    }
}
impl Drop for StreamHandle {
    fn drop(&mut self) {
        self.stop();
    }
}

/// The mixer's end of a stream: its ring, and where the resampler is.
pub(super) struct StreamVoice {
    control: Arc<Control>,
    ring: Arc<Ring>,
    /// The last frame taken from the ring, and how far (0 to 1) the output is towards the next.
    current: [f32; 2],
    fraction: f64,
    /// The placement the callback last read whole.
    view: View,
}

/// The main thread's list of open streams, with the caps. Shared by every `Audio` on one output.
#[derive(Default)]
pub(super) struct StreamPool {
    controls: Mutex<Vec<Arc<Control>>>,
    /// How many controls there are, so that with no stream open the main thread's calls and the
    /// tick take no lock at all.
    open: AtomicUsize,
    /// The output went away (a failed reopen): no stream opens until it is back.
    lost: AtomicBool,
}

impl StreamPool {
    fn controls(&self) -> MutexGuard<'_, Vec<Arc<Control>>> {
        self.controls.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Whether no stream is open: the lock-free test every call that has nothing to do starts with.
    fn idle(&self) -> bool {
        self.open.load(Relaxed) == 0
    }

    /// Opens a stream for `owner`, a mod. The ring is allocated here, before the mixer is
    /// locked, once.
    pub(super) fn open(
        &self,
        mixer: &Mutex<Mixer>,
        owner: &str,
        spec: &StreamSpec,
    ) -> Result<Arc<dyn AudioStreamSource>, AudioRefusal> {
        let fault = |fault| Err(AudioRefusal::BadSpec(fault));
        if !matches!(spec.channels, 1 | 2) {
            return fault(SpecFault::Channels);
        }
        if !(AUDIO_STREAM_RATE_MIN..=AUDIO_STREAM_RATE_MAX).contains(&spec.rate) {
            return fault(SpecFault::Rate);
        }
        if !(AUDIO_STREAM_PREBUFFER_MIN_MS..=AUDIO_STREAM_PREBUFFER_MAX_MS)
            .contains(&spec.prebuffer_ms)
        {
            return fault(SpecFault::Prebuffer);
        }
        if !(0.0..=1.0).contains(&spec.volume) {
            return fault(SpecFault::Volume);
        }
        if !(spec.range.is_finite() && spec.range > 0.0) {
            return fault(SpecFault::Range);
        }
        if let Err(f) = check_placement(spec.channels, &spec.placement) {
            return fault(f);
        }
        if self.lost.load(Acquire) {
            return Err(AudioRefusal::NoOutput);
        }
        self.reap(mixer);
        let mut controls = self.controls();
        if controls.len() >= AUDIO_STREAMS_TOTAL as usize {
            return Err(AudioRefusal::TooManyStreams);
        }
        if controls.iter().filter(|c| c.owner == owner).count() >= AUDIO_STREAMS_PER_MOD as usize {
            return Err(AudioRefusal::ModStreamLimit);
        }
        let channels = spec.channels as usize;
        let ring_ms = (2 * spec.prebuffer_ms + 100).min(AUDIO_STREAM_RING_MAX_MS);
        let ring_frames = (u64::from(spec.rate) * u64::from(ring_ms) / 1000) as usize;
        let ring = Arc::new(Ring::new(ring_frames, channels));
        let control = Arc::new(Control {
            channels,
            rate: spec.rate,
            prebuffer_frames: u64::from(spec.rate) * u64::from(spec.prebuffer_ms) / 1000,
            category: spec.category.index(),
            range: spec.range,
            owner: owner.to_owned(),
            volume: AtomicU32::new(spec.volume.to_bits()),
            playing: AtomicBool::new(false),
            underruns: AtomicU32::new(0),
            closed: AtomicBool::new(false),
            pushing: AtomicBool::new(false),
            held: Mutex::new(Held::Flat),
            sequence: AtomicU64::new(0),
            flags: AtomicU8::new(0),
            position: Default::default(),
        });
        control.place(&spec.placement);
        let handle = StreamHandle {
            control: control.clone(),
            ring: Arc::downgrade(&ring),
        };
        let voice = StreamVoice {
            control: control.clone(),
            ring,
            current: [0.0; 2],
            fraction: 0.0,
            // The placement just published: a voice is never heard Flat while its real placement
            // is not in. Should the callback find a writer in the middle of a change before it
            // has read any, it keeps this, or silence.
            view: control.view().unwrap_or(View {
                positional: true,
                placed: false,
                at: [0.0; 3],
            }),
        };
        match mixer.lock() {
            Ok(mut mixer) => mixer.streams.push(voice),
            Err(_) => return Err(AudioRefusal::NoOutput),
        }
        controls.push(control);
        self.open.store(controls.len(), Relaxed);
        Ok(Arc::new(handle))
    }

    /// Lets go of the streams a mod stopped or dropped: their voices leave the mixer, and with
    /// the last of them their rings are freed.
    pub(super) fn reap(&self, mixer: &Mutex<Mixer>) {
        if self.idle() {
            return;
        }
        let mut controls = self.controls();
        let before = controls.len();
        controls.retain(|c| !c.closed.load(Acquire));
        self.open.store(controls.len(), Relaxed);
        if controls.len() != before {
            // The voices leave into a list made before the lock, with room for every voice there
            // was (a mod may stop another stream meanwhile): nothing allocates under the lock,
            // and the rings drop after it is released, never in the callback.
            let mut gone = Vec::with_capacity(before);
            if let Ok(mut mixer) = mixer.lock() {
                let mut i = 0;
                while i < mixer.streams.len() {
                    if mixer.streams[i].control.closed.load(Acquire) {
                        gone.push(mixer.streams.swap_remove(i));
                    } else {
                        i += 1;
                    }
                }
            }
            drop(gone);
        }
    }

    /// Follows each entity-placed stream's entity: where `resolve` finds it the stream is placed
    /// there, where it doesn't (untracked, unloaded, removed) the stream is unplaced. Takes no
    /// lock the output callback takes.
    pub(super) fn follow_entities(&self, mut resolve: impl FnMut(EntityRef) -> Option<[f64; 3]>) {
        if self.idle() {
            return;
        }
        for control in self.controls().iter() {
            control.follow(&mut resolve);
        }
    }

    /// The listener left its level: every positional stream is silent until its mod places it
    /// again.
    pub(super) fn unplace(&self) {
        if self.idle() {
            return;
        }
        for control in self.controls().iter() {
            control.unplace();
        }
    }

    /// Each open stream's `prebuffer_ms` and how many times it ran dry: what a test of a mod's
    /// pacing reads.
    pub(super) fn underruns(&self) -> Vec<(u32, u32)> {
        self.controls()
            .iter()
            .map(|c| {
                let prebuffer_ms = c.prebuffer_frames * 1000 / u64::from(c.rate);
                (prebuffer_ms as u32, c.underruns.load(Relaxed))
            })
            .collect()
    }

    /// The output went away (or came back): its streams stop, and none opens until it is back.
    #[cfg(any(feature = "audio", test))]
    pub(super) fn output_lost(&self, lost: bool) {
        self.lost.store(lost, Release);
        if lost {
            for control in self.controls().iter() {
                control.closed.store(true, Release);
            }
        }
    }
}

/// Mixes every stream into `out` (the muffle's bus or the output), `channels` wide at `rate`.
/// `category_gain` is master times group by `SoundSource` index. Allocates nothing: the rings,
/// the voices and the cameras were all made before.
pub(super) fn mix(
    streams: &mut [StreamVoice],
    cameras: &[([f64; 3], Basis, u32)],
    category_gain: &[f32; 11],
    out: &mut [f32],
    channels: usize,
    rate: u32,
) {
    let shared_listeners = cameras.len() > 1;
    for voice in streams.iter_mut() {
        let c = &*voice.control;
        if c.closed.load(Acquire) {
            continue;
        }
        let ring = &*voice.ring;
        let ch = ring.channels;
        let len = ring.samples.len() as u64;
        let head = ring.head.load(Relaxed);
        let tail = ring.tail.load(Acquire);
        let mut avail = (tail - head) / ch as u64;
        let mut taken = 0u64;
        // The frame `frame` places past the ones taken, from the ring.
        let frame_at = |frame: u64| -> [f32; 2] {
            let base = head + frame * ch as u64;
            let sample = |i: u64| {
                f32::from(ring.samples[((base + i) % len) as usize].load(Relaxed)) / 32768.0
            };
            let left = sample(0);
            [left, if ch > 1 { sample(1) } else { left }]
        };
        if !c.playing.load(Relaxed) {
            if avail < c.prebuffer_frames.max(2) {
                continue;
            }
            voice.current = frame_at(0);
            voice.fraction = 0.0;
            taken = 1;
            avail -= 1;
            c.playing.store(true, Release);
        }
        let volume = f32::from_bits(c.volume.load(Relaxed)) * category_gain[c.category];
        // Gain per output channel, and the one a mono output takes (no panning); NaN anywhere
        // is silence. A stream with no volume is not looked at.
        let [left, right, mono] = if volume == 0.0 {
            [0.0; 3]
        } else {
            // What the callback last read whole stands if a writer is in the middle of a change.
            if let Some(view) = c.view() {
                voice.view = view;
            }
            if !voice.view.positional {
                [volume; 3]
            } else if !voice.view.placed {
                [0.0; 3]
            } else {
                let at = voice.view.at;
                let away = |camera: &([f64; 3], Basis, u32)| {
                    [
                        at[0] - camera.0[0],
                        at[1] - camera.0[1],
                        at[2] - camera.0[2],
                    ]
                };
                let sqr = |d: [f64; 3]| d[0] * d[0] + d[1] * d[1] + d[2] * d[2];
                match cameras
                    .iter()
                    .min_by(|a, b| sqr(away(a)).total_cmp(&sqr(away(b))))
                {
                    None => [0.0; 3],
                    Some(camera) => {
                        let d = away(camera);
                        let (forward, up, right) = camera.1;
                        let dot = |a: [f64; 3], b: [f64; 3]| {
                            (a[0] * b[0] + a[1] * b[1] + a[2] * b[2]) as f32
                        };
                        let distance = sqr(d).sqrt() as f32;
                        let gain = volume * distance_gain(Some(c.range), distance);
                        let [mut l, mut r] = super::stereo_gains_with_length(
                            dot(d, right),
                            dot(d, up),
                            dot(d, forward),
                            distance,
                        );
                        if shared_listeners {
                            let middle = (l + r) * 0.5;
                            l = middle + (l - middle) * 0.5;
                            r = middle + (r - middle) * 0.5;
                        }
                        [l * gain, r * gain, gain]
                    }
                }
            }
        };
        let [left, right, mono] = [left, right, mono].map(|g| if g.is_finite() { g } else { 0.0 });
        let step = f64::from(c.rate) / f64::from(rate);
        let output_frames = out.len() / channels;
        if left == 0.0 && right == 0.0 && mono == 0.0 {
            // Inaudible (muted, no volume, unplaced, out of range): no resampling and no mixing,
            // only the ring's head moves, as far as the output would have taken it. The source
            // frame wanted at the start of each output frame but the first is
            // `floor(fraction + step * (frames - 1))`, as the loop below counts them.
            if output_frames > 0 {
                let wanted = (voice.fraction + step * (output_frames - 1) as f64) as u64;
                let advance = wanted.min(avail);
                if advance > 0 {
                    voice.current = frame_at(taken + advance - 1);
                }
                taken += advance;
                if wanted > advance {
                    c.playing.store(false, Release);
                    c.underruns.fetch_add(1, Relaxed);
                } else {
                    voice.fraction += step * output_frames as f64 - wanted as f64;
                }
            }
            ring.head.store(head + taken * ch as u64, Release);
            continue;
        }
        for frame in out.chunks_mut(channels) {
            // The source frames the last output frame moved past. A frame that is not there yet
            // is an underrun only now, when the output needs it: one that merely has not arrived
            // by the end of a buffer is looked for again in the next.
            while voice.fraction >= 1.0 {
                if avail == 0 {
                    // Silence, and buffering again until `prebuffer_ms` is back.
                    c.playing.store(false, Release);
                    c.underruns.fetch_add(1, Relaxed);
                    break;
                }
                voice.current = frame_at(taken);
                taken += 1;
                avail -= 1;
                voice.fraction -= 1.0;
            }
            if !c.playing.load(Relaxed) {
                break;
            }
            // Between two source frames the output is their mix; past the last one queued it
            // holds that one, so every accepted frame plays for its whole length.
            let now = if voice.fraction > 0.0 && avail > 0 {
                let next = frame_at(taken);
                let t = voice.fraction as f32;
                [
                    voice.current[0] + (next[0] - voice.current[0]) * t,
                    voice.current[1] + (next[1] - voice.current[1]) * t,
                ]
            } else {
                voice.current
            };
            if channels >= 2 {
                frame[0] += now[0] * left;
                frame[1] += now[1] * right;
            } else {
                frame[0] += if ch > 1 {
                    (now[0] + now[1]) * 0.5
                } else {
                    now[0]
                } * mono;
            }
            voice.fraction += step;
        }
        ring.head.store(head + taken * ch as u64, Release);
    }
}

#[cfg(test)]
mod tests {
    use super::super::Audio;
    use super::super::{Mixer, mix as mix_output};
    use super::*;
    use mod_api::SoundSource;

    /// A mixer and the pool that feeds it, every group at full volume and the listener at the
    /// origin, facing +z.
    struct Rig {
        mixer: Arc<Mutex<Mixer>>,
        pool: StreamPool,
    }
    impl Rig {
        fn new() -> Self {
            let mixer = Mixer {
                stream_gain: [1.0; 11],
                ..Mixer::default()
            };
            Self {
                mixer: Arc::new(Mutex::new(mixer)),
                pool: StreamPool::default(),
            }
        }
        fn open(
            &self,
            owner: &str,
            spec: &StreamSpec,
        ) -> Result<Arc<dyn AudioStreamSource>, AudioRefusal> {
            self.pool.open(&self.mixer, owner, spec)
        }
        /// The next `frames` of stereo output at 48 kHz.
        fn play(&self, frames: usize) -> Vec<f32> {
            let mut data = vec![0.0; frames * 2];
            mix_output(&mut self.mixer.lock().unwrap(), &mut data, 2, 48_000);
            data
        }
        fn voices(&self) -> usize {
            self.mixer.lock().unwrap().streams.len()
        }
    }
    /// A mono 48 kHz stream with the shortest prebuffer: 960 frames to start, a 6720 frame ring.
    fn spec() -> StreamSpec {
        StreamSpec::new(SoundSource::Voice, 1, 48_000).with_prebuffer_ms(20)
    }
    /// Half-scale samples: 0.5 once mixed at gain 1.
    fn half(frames: usize) -> Vec<i16> {
        vec![16_384; frames]
    }
    /// 1, 2, 3...: each frame its own value, so which ones played shows.
    fn ramp(frames: usize) -> Vec<i16> {
        (1..=frames as i16).map(|i| i * 16).collect()
    }

    #[test]
    fn a_stream_is_silent_until_its_prebuffer_is_in_and_then_plays() {
        let rig = Rig::new();
        let stream = rig.open("mod", &spec()).unwrap();
        assert_eq!(stream.push(&half(959)), 959);
        assert!(rig.play(8).iter().all(|s| *s == 0.0));
        assert_eq!(stream.state(), AudioStreamState::Buffering);
        assert_eq!(stream.push(&half(1)), 1);
        let out = rig.play(8);
        assert!(out.iter().all(|s| *s == 0.5), "{out:?}");
        assert_eq!(stream.state(), AudioStreamState::Playing);
    }

    #[test]
    fn every_accepted_frame_plays_and_then_the_stream_goes_back_to_buffering() {
        let rig = Rig::new();
        let stream = rig.open("mod", &spec()).unwrap();
        stream.push(&ramp(960));
        let out = rig.play(2_000);
        // Equal rates: frame k is output k, exactly, all 960 of them, left and right alike.
        for k in 0..960 {
            let expected = f32::from(ramp(960)[k]) / 32768.0;
            assert_eq!(out[2 * k], expected, "frame {k}");
            assert_eq!(out[2 * k + 1], expected, "frame {k}");
        }
        assert!(out[1_920..].iter().all(|s| *s == 0.0));
        assert_eq!(stream.state(), AudioStreamState::Buffering);
        // It waits for a whole prebuffer again, and then plays on: it never ended.
        stream.push(&half(500));
        assert!(rig.play(8).iter().all(|s| *s == 0.0));
        stream.push(&half(500));
        assert!(rig.play(8).iter().all(|s| *s > 0.4));
    }

    #[test]
    fn a_slower_stream_is_interpolated_and_keeps_its_last_frame_for_its_whole_length() {
        let rig = Rig::new();
        let stream = rig
            .open(
                "mod",
                &StreamSpec::new(SoundSource::Voice, 1, 24_000).with_prebuffer_ms(20),
            )
            .unwrap();
        let samples = ramp(480);
        stream.push(&samples);
        // Across two callbacks, so the resampler's place carries over the boundary.
        let mut out = rig.play(333);
        out.extend(rig.play(1_000));
        let value = |k: usize| f32::from(samples[k]) / 32768.0;
        for k in 0..479 {
            assert_eq!(out[2 * 2 * k], value(k), "source frame {k}");
            let middle = value(k) + (value(k + 1) - value(k)) * 0.5;
            assert_eq!(out[2 * (2 * k + 1)], middle, "between {k} and {}", k + 1);
        }
        // 480 source frames are 960 output frames; the last one holds for its second half.
        assert_eq!(out[2 * 958], value(479));
        assert_eq!(out[2 * 959], value(479));
        assert!(out[2 * 960..].iter().all(|s| *s == 0.0));
    }

    #[test]
    fn a_full_ring_refuses_the_excess() {
        let rig = Rig::new();
        let stream = rig.open("mod", &spec()).unwrap();
        assert_eq!(stream.push(&half(10_000)), 6_720);
        assert_eq!(stream.push(&half(1)), 0);
        assert_eq!(stream.buffered_ms(), 140);
        // Playing makes room again: the 480 frames played.
        rig.play(480);
        assert_eq!(stream.push(&half(10_000)), 480);
    }

    #[test]
    fn a_mono_stream_at_half_its_range_has_half_the_gain() {
        let rig = Rig::new();
        let range = 16.0;
        let heard = |at: [f64; 3]| {
            let stream = rig
                .open(
                    "mod",
                    &spec()
                        .with_placement(StreamPlacement::At(at))
                        .with_range(range),
                )
                .unwrap();
            stream.push(&half(2_000));
            let out = rig.play(16);
            stream.stop();
            rig.pool.reap(&rig.mixer);
            out[8]
        };
        let here = heard([0.0, 0.0, 0.0]);
        let half_way = heard([0.0, 0.0, f64::from(range) / 2.0]);
        let edge = heard([0.0, 0.0, f64::from(range)]);
        assert!(here > 0.2);
        assert!((half_way / here - 0.5).abs() < 1e-3, "{half_way} {here}");
        assert_eq!(edge, 0.0);
    }

    #[test]
    fn a_mono_output_hears_the_distance_gain_whichever_side_the_stream_is_on() {
        let rig = Rig::new();
        let mono_output = |at: [f64; 3]| {
            let stream = rig
                .open(
                    "mod",
                    &spec()
                        .with_placement(StreamPlacement::At(at))
                        .with_range(16.0),
                )
                .unwrap();
            stream.push(&half(2_000));
            let mut data = vec![0.0; 8];
            mix_output(&mut rig.mixer.lock().unwrap(), &mut data, 1, 48_000);
            stream.stop();
            rig.pool.reap(&rig.mixer);
            data[4]
        };
        // 8 blocks away to the left, to the right and ahead: half of 0.5 whichever.
        for at in [[-8.0, 0.0, 0.0], [8.0, 0.0, 0.0], [0.0, 0.0, 8.0]] {
            assert!((mono_output(at) - 0.25).abs() < 1e-6, "{at:?}");
        }
    }

    #[test]
    fn a_stereo_stream_with_a_placement_is_refused() {
        let rig = Rig::new();
        let stereo = StreamSpec::new(SoundSource::Voice, 2, 48_000);
        let at = StreamPlacement::At([1.0, 2.0, 3.0]);
        assert!(matches!(
            rig.open("mod", &stereo.clone().with_placement(at)),
            Err(AudioRefusal::BadSpec(SpecFault::StereoPositioned))
        ));
        let flat = rig.open("mod", &stereo).unwrap();
        assert!(!flat.set_placement(&at));
        assert!(flat.set_placement(&StreamPlacement::Flat));
    }

    #[test]
    fn streams_are_capped_per_mod_and_overall() {
        let rig = Rig::new();
        let mut open = Vec::new();
        for _ in 0..32 {
            open.push(rig.open("a", &spec()).unwrap());
        }
        assert!(matches!(
            rig.open("a", &spec()),
            Err(AudioRefusal::ModStreamLimit)
        ));
        for _ in 0..32 {
            open.push(rig.open("b", &spec()).unwrap());
        }
        assert!(matches!(
            rig.open("c", &spec()),
            Err(AudioRefusal::TooManyStreams)
        ));
        // A stopped stream frees its place.
        open.pop().unwrap().stop();
        assert!(rig.open("c", &spec()).is_ok());
    }

    #[test]
    fn bad_numbers_are_refused_and_never_reach_the_mixer() {
        let rig = Rig::new();
        let bad = |s: StreamSpec| rig.open("mod", &s).err();
        assert_eq!(
            bad(StreamSpec::new(SoundSource::Voice, 3, 48_000)),
            Some(AudioRefusal::BadSpec(SpecFault::Channels))
        );
        assert_eq!(
            bad(StreamSpec::new(SoundSource::Voice, 1, 7_999)),
            Some(AudioRefusal::BadSpec(SpecFault::Rate))
        );
        assert_eq!(
            bad(spec().with_volume(f32::NAN)),
            Some(AudioRefusal::BadSpec(SpecFault::Volume))
        );
        assert_eq!(
            bad(spec().with_range(f32::INFINITY)),
            Some(AudioRefusal::BadSpec(SpecFault::Range))
        );
        assert_eq!(
            bad(spec().with_placement(StreamPlacement::At([f64::NAN, 0.0, 0.0]))),
            Some(AudioRefusal::BadSpec(SpecFault::Position))
        );
        let stream = rig.open("mod", &spec()).unwrap();
        assert!(!stream.set_volume(f32::NAN));
        assert!(!stream.set_placement(&StreamPlacement::At([0.0, f64::INFINITY, 0.0])));
        assert_eq!(rig.voices(), 1);
    }

    #[test]
    fn an_unplaced_stream_is_silent_but_its_audio_is_still_consumed() {
        let rig = Rig::new();
        let entity = mod_api::EntityRef::new(mod_api::WorldEpoch::new(1), 7);
        let stream = rig
            .open(
                "mod",
                &spec().with_placement(StreamPlacement::Entity(entity)),
            )
            .unwrap();
        stream.push(&half(2_000));
        rig.pool.follow_entities(|_| None);
        let before = stream.buffered_ms();
        assert!(rig.play(480).iter().all(|s| *s == 0.0));
        assert_eq!(stream.state(), AudioStreamState::Unplaced);
        assert_eq!(before - stream.buffered_ms(), 10);
        // Tracked again, at a place a NaN spoils: still unplaced; at a real one: heard.
        rig.pool.follow_entities(|_| Some([f64::NAN, 0.0, 0.0]));
        assert_eq!(stream.state(), AudioStreamState::Unplaced);
        rig.pool
            .follow_entities(|e| (e == entity).then_some([0.0, 0.0, 1.0]));
        assert!(rig.play(16).iter().any(|s| *s > 0.0));
        assert_eq!(stream.state(), AudioStreamState::Playing);
    }

    #[test]
    fn a_newer_placement_is_never_overwritten_by_an_older_entity_resolution() {
        let rig = Rig::new();
        let entity = mod_api::EntityRef::new(mod_api::WorldEpoch::new(1), 7);
        let heard = |follow_after: bool| {
            let stream = rig
                .open(
                    "mod",
                    &spec().with_placement(StreamPlacement::Entity(entity)),
                )
                .unwrap();
            stream.push(&half(2_000));
            // The mod places the stream at a point; the frame's resolution of the entity it
            // followed before then arrives, at another place.
            assert!(stream.set_placement(&StreamPlacement::At([0.0, 0.0, 8.0])));
            if follow_after {
                rig.pool.follow_entities(|_| Some([0.0, 0.0, 1.0]));
            }
            let out = rig.play(16);
            stream.stop();
            rig.pool.reap(&rig.mixer);
            (out[8], stream.state())
        };
        let (alone, _) = heard(false);
        let (after, _) = heard(true);
        assert!(alone > 0.0);
        assert_eq!(after, alone);
    }

    #[test]
    fn a_dimension_change_unplaces_positional_streams_until_they_are_placed_again() {
        let rig = Rig::new();
        let at = StreamPlacement::At([0.0, 0.0, 1.0]);
        let positional = rig.open("mod", &spec().with_placement(at)).unwrap();
        let flat = rig.open("mod", &spec()).unwrap();
        for stream in [&positional, &flat] {
            stream.push(&half(2_000));
        }
        rig.play(8);
        rig.pool.unplace();
        assert_eq!(positional.state(), AudioStreamState::Unplaced);
        assert_eq!(flat.state(), AudioStreamState::Playing);
        assert!(positional.set_placement(&at));
        assert_eq!(positional.state(), AudioStreamState::Playing);
    }

    #[test]
    fn a_stopped_stream_frees_its_ring_even_while_the_mod_keeps_the_handle() {
        let rig = Rig::new();
        let mut kept = Vec::new();
        let mut rings = Vec::new();
        for _ in 0..4 {
            let stream = rig.open("mod", &spec()).unwrap();
            stream.push(&half(100));
            rings.push(Arc::downgrade(
                &rig.mixer.lock().unwrap().streams.last().unwrap().ring,
            ));
            stream.stop();
            kept.push(stream);
        }
        // Each open reaps the stopped streams before it: only the last one's ring is still held.
        assert_eq!(rig.voices(), 1);
        assert!(rings[..3].iter().all(|ring| ring.upgrade().is_none()));
        // The next open or the tick lets go of that one too.
        rig.pool.reap(&rig.mixer);
        assert_eq!(rig.voices(), 0);
        assert!(rings.iter().all(|ring| ring.upgrade().is_none()));
        for stream in &kept {
            assert_eq!(stream.push(&half(10)), 0);
            assert_eq!(stream.state(), AudioStreamState::Stopped);
            assert_eq!(stream.buffered_ms(), 0);
        }
    }

    #[test]
    fn dropping_the_handle_stops_the_stream_and_the_pool_lets_it_go() {
        let rig = Rig::new();
        let stream = rig.open("mod", &spec()).unwrap();
        drop(mod_api::AudioStreamRef::new(stream));
        rig.pool.reap(&rig.mixer);
        assert_eq!(rig.voices(), 0);
    }

    #[test]
    fn leaving_a_level_unplaces_positional_streams_and_leaves_flat_ones_playing() {
        let mut audio = Audio::silent(&sunrise_assets::resource_source::ResourceSource::directory(
            std::path::PathBuf::from("/nonexistent/sunrise-test-audio"),
        ));
        audio.backend_live_for_bench();
        let positional = audio
            .open_stream(
                "mod",
                &spec().with_placement(StreamPlacement::At([0.0, 0.0, 1.0])),
            )
            .unwrap();
        let flat = audio.open_stream("mod", &spec()).unwrap();
        for stream in [&positional, &flat] {
            stream.push(&half(2_000));
        }
        mix_output(
            &mut audio.backend.mixer.lock().unwrap(),
            &mut [0.0; 16],
            2,
            48_000,
        );
        assert_eq!(positional.state(), AudioStreamState::Playing);
        audio.leave_level();
        assert_eq!(positional.state(), AudioStreamState::Unplaced);
        assert_eq!(flat.state(), AudioStreamState::Playing);
    }

    #[test]
    fn entity_streams_are_followed_again_before_they_are_heard_after_mod_audio_returns() {
        let mut audio = Audio::silent(&sunrise_assets::resource_source::ResourceSource::directory(
            std::path::PathBuf::from("/nonexistent/sunrise-test-audio"),
        ));
        audio.backend_live_for_bench();
        let entity = mod_api::EntityRef::new(mod_api::WorldEpoch::new(1), 7);
        let _stream = audio
            .open_stream(
                "mod",
                &spec().with_placement(StreamPlacement::Entity(entity)),
            )
            .unwrap();
        let heard = |audio: &Audio| {
            audio.backend.mixer.lock().unwrap().stream_gain[SoundSource::Voice.index()]
        };
        let mut asked = 0;
        // Off: nothing is followed, and when it is back the mixer still hears nothing...
        audio.follow_entities_at(0.0, |_| {
            asked += 1;
            None
        });
        assert_eq!(asked, 0);
        audio.sync_stream_gain(1.0);
        assert_eq!(heard(&audio), 0.0);
        // ...until a frame has followed the entities.
        audio.follow_entities_at(1.0, |_| {
            asked += 1;
            Some([0.0, 0.0, 1.0])
        });
        assert_eq!(asked, 1);
        audio.sync_stream_gain(1.0);
        assert!(heard(&audio) > 0.0);
    }

    #[test]
    fn a_lost_output_stops_every_stream_and_refuses_new_ones_until_it_is_back() {
        let rig = Rig::new();
        let stream = rig.open("mod", &spec()).unwrap();
        stream.push(&half(2_000));
        rig.play(8);
        assert_eq!(stream.state(), AudioStreamState::Playing);
        rig.pool.output_lost(true);
        assert_eq!(stream.state(), AudioStreamState::Stopped);
        assert!(matches!(
            rig.open("mod", &spec()),
            Err(AudioRefusal::NoOutput)
        ));
        rig.pool.output_lost(false);
        assert!(rig.open("mod", &spec()).is_ok());
    }

    #[test]
    fn the_muffle_turns_streams_down_with_everything_else() {
        let rig = Rig::new();
        let stream = rig.open("mod", &spec()).unwrap();
        stream.push(&half(2_000));
        {
            let mut mixer = rig.mixer.lock().unwrap();
            mixer.muffle.target = [1.0, 0.0];
            mixer.muffle.current = [1.0, 0.0];
        }
        assert!(rig.play(64).iter().all(|s| *s == 0.0));
    }

    /// 24 kHz mono, so one source frame plays for exactly two output frames: no rounding.
    fn half_rate(category: SoundSource) -> StreamSpec {
        StreamSpec::new(category, 1, 24_000).with_prebuffer_ms(20)
    }

    #[test]
    fn a_stream_with_no_volume_takes_its_audio_in_at_the_pace_of_one_that_is_heard() {
        let rig = Rig::new();
        let heard = rig.open("mod", &half_rate(SoundSource::Voice)).unwrap();
        let unheard = rig.open("mod", &half_rate(SoundSource::Music)).unwrap();
        for stream in [&heard, &unheard] {
            stream.push(&ramp(2_000));
        }
        rig.mixer.lock().unwrap().stream_gain[SoundSource::Music.index()] = 0.0;
        // Odd buffer lengths, so the half frames carry from one buffer to the next.
        for frames in [481, 479, 1_001, 3] {
            let out = rig.play(frames);
            assert!(out.iter().any(|s| *s != 0.0));
            assert_eq!(heard.buffered_ms(), unheard.buffered_ms());
        }
        assert!(heard.buffered_ms() < 2_000 * 1000 / 24_000);
        // Heard again, it plays from where the audio has got to, as the other does.
        rig.mixer.lock().unwrap().stream_gain[SoundSource::Music.index()] = 1.0;
        let out = rig.play(8);
        for pair in out.chunks(2) {
            assert!(pair[0] != 0.0 && pair[0] == pair[1]);
        }
        assert_eq!(heard.state(), unheard.state());
    }

    #[test]
    fn an_unheard_stream_that_runs_dry_goes_back_to_buffering_like_a_heard_one() {
        let rig = Rig::new();
        let stream = rig.open("mod", &half_rate(SoundSource::Music)).unwrap();
        stream.push(&ramp(480));
        rig.mixer.lock().unwrap().stream_gain[SoundSource::Music.index()] = 0.0;
        // 480 source frames last 960 output frames.
        rig.play(960);
        assert_eq!(stream.state(), AudioStreamState::Playing);
        rig.play(2);
        assert_eq!(stream.state(), AudioStreamState::Buffering);
    }

    #[test]
    fn audio_that_has_not_arrived_by_the_end_of_a_buffer_is_not_an_underrun() {
        let rig = Rig::new();
        let stream = rig.open("mod", &spec()).unwrap();
        stream.push(&half(960));
        // Exactly the audio there is: the buffer ends where it does, nothing has run dry.
        assert!(rig.play(960).iter().all(|s| *s == 0.5));
        assert_eq!(stream.state(), AudioStreamState::Playing);
        // A chunk much smaller than the prebuffer arrives before the next buffer: it plays.
        stream.push(&half(100));
        let out = rig.play(100);
        assert!(out.iter().all(|s| *s == 0.5), "{out:?}");
        assert_eq!(stream.state(), AudioStreamState::Playing);
        // Only now that the output needs a frame and none is there does it run dry.
        rig.play(1);
        assert_eq!(stream.state(), AudioStreamState::Buffering);
    }

    #[test]
    fn a_stream_is_never_heard_flat_before_its_placement_is_in() {
        let rig = Rig::new();
        let stream = rig
            .open(
                "mod",
                &spec().with_placement(StreamPlacement::At([0.0, 0.0, 10_000.0])),
            )
            .unwrap();
        stream.push(&half(2_000));
        // A writer is in the middle of a change when the callback first looks.
        let control = rig.pool.controls()[0].clone();
        control.sequence.store(1, Relaxed);
        assert!(rig.play(64).iter().all(|s| *s == 0.0));
        let voice = &rig.mixer.lock().unwrap().streams[0];
        assert!(voice.view.positional && voice.view.placed);
    }

    #[test]
    fn with_no_stream_open_the_pool_takes_no_lock_of_the_mixers() {
        let rig = Rig::new();
        assert!(rig.pool.idle());
        // The callback holds the mixer: with nothing open, nothing here waits for it.
        let _callback = rig.mixer.lock().unwrap();
        rig.pool.reap(&rig.mixer);
        rig.pool.unplace();
        rig.pool.follow_entities(|_| unreachable!());
        drop(_callback);
        let stream = rig.open("mod", &spec()).unwrap();
        assert!(!rig.pool.idle());
        stream.stop();
        rig.pool.reap(&rig.mixer);
        assert!(rig.pool.idle());
        assert_eq!(rig.voices(), 0);
    }

    #[test]
    fn a_pushing_thread_never_waits_on_the_mixer() {
        let rig = Rig::new();
        let stream = rig.open("mod", &spec()).unwrap();
        // The mixer is locked, as the output callback holds it, for the whole of the push.
        let _callback = rig.mixer.lock().unwrap();
        let pusher = std::thread::spawn(move || {
            stream.set_volume(0.5);
            stream.set_placement(&StreamPlacement::Flat);
            stream.push(&half(100))
        });
        assert_eq!(pusher.join().unwrap(), 100);
    }

    #[test]
    fn a_second_pusher_at_the_same_moment_gets_nothing() {
        let rig = Rig::new();
        let stream = rig.open("mod", &spec()).unwrap();
        // Another thread is mid-push: its flag is up.
        let handle = rig.pool.controls()[0].clone();
        handle.pushing.store(true, Release);
        assert_eq!(stream.push(&half(10)), 0);
        handle.pushing.store(false, Release);
        assert_eq!(stream.push(&half(10)), 10);
    }

    #[test]
    fn the_buffered_time_never_wraps_while_the_callback_consumes_and_a_mod_pushes() {
        let ring = Arc::new(Ring::new(1_000, 1));
        let running = Arc::new(AtomicBool::new(true));
        let observer = {
            let (ring, running) = (ring.clone(), running.clone());
            std::thread::spawn(move || {
                let mut worst = 0;
                while running.load(Relaxed) {
                    worst = worst.max(ring.buffered_frames());
                }
                worst
            })
        };
        // A producer and a consumer racing: tail leads head by up to the ring's length.
        for step in 1..200_000u64 {
            let tail = ring.tail.load(Relaxed);
            let head = ring.head.load(Relaxed);
            if tail - head < 1_000 {
                ring.tail.store(tail + 1, Release);
            }
            if step % 3 != 0 && head < ring.tail.load(Relaxed) {
                ring.head.store(head + 1, Release);
            }
        }
        running.store(false, Relaxed);
        assert!(observer.join().unwrap() <= 1_000);
    }

    /// How a tick-paced mod supplies audio: it pushes once a tick, 50 ms of audio each, and a tick can
    /// arrive up to 50 ms late. Returns the underruns in 30 s of 48 kHz mono for `prebuffer_ms`,
    /// with a device callback every 10 ms. The ticks' lateness is a fixed pseudo-random series,
    /// so the figures are exact and repeat.
    fn tick_paced_underruns(prebuffer_ms: u32) -> u32 {
        let rig = Rig::new();
        let stream = rig
            .open(
                "tick_paced",
                &StreamSpec::new(SoundSource::Voice, 1, 48_000).with_prebuffer_ms(prebuffer_ms),
            )
            .unwrap();
        let (mut late, mut seed) = (0u32, 0x9e37_79b9u32);
        let mut next_late = || {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (seed >> 16) % 51
        };
        let mut push_at = next_late();
        let mut underruns = 0;
        let mut was_playing = false;
        for ms in (0..30_000u32).step_by(10) {
            // The ticks due by now push the 50 ms they have made.
            while push_at <= ms {
                stream.push(&half(2_400));
                late += 1;
                push_at = late * 50 + next_late();
            }
            rig.play(480);
            let playing = stream.state() == AudioStreamState::Playing;
            if was_playing && !playing {
                underruns += 1;
            }
            was_playing = playing;
        }
        underruns
    }

    #[test]
    fn a_tick_paced_stream_underruns_less_the_larger_its_prebuffer() {
        let (a, b, c) = (
            tick_paced_underruns(50),
            tick_paced_underruns(100),
            tick_paced_underruns(150),
        );
        eprintln!("tick-paced underruns in 30 s: 50 ms {a}, 100 ms {b}, 150 ms {c}");
        // The book's table: these exact counts, on the virtual clock.
        assert_eq!((a, b, c), (2, 0, 0));
    }
}
