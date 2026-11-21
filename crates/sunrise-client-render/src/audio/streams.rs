//! Mods' audio streams have their own pool (never vanilla's streaming slots). Each fixed PCM
//! ring is filled by an async producer or the pool's pull worker, and drained by the mixer.
//!
//! - A mod thread pushes PCM or sets the volume and placement without taking the mixer lock.
//! - The callback reads PCM and the latest placement (a seqlock) and requests more pull PCM
//!   for the next buffer. It does not allocate or call the source.
//! - The main thread opens, caps, follows entities, unplaces and reaps streams. It allocates
//!   rings before locking the mixer. Push handles hold only weak references to the rings.
//! - The worker owns, calls and drops admitted native sources without taking either lock.
mod pull;

use super::{Basis, Mixer, distance_gain};
use mod_api::{
    AUDIO_STREAM_RATE_MAX, AUDIO_STREAM_RATE_MIN, AUDIO_STREAM_RING_MAX_MS, AUDIO_STREAMS_PER_MOD,
    AUDIO_STREAMS_TOTAL, AudioRefusal, AudioStream, AudioStreamSource, AudioStreamState, EntityRef,
    SpecFault, StreamPlacement, StreamSpec,
};
use pull::{Demand, PullJob, Worker};
use std::sync::{
    Arc, Mutex, MutexGuard, Weak,
    atomic::{
        AtomicBool, AtomicI16, AtomicU8, AtomicU32, AtomicU64, AtomicUsize, Ordering::*, fence,
    },
};

/// How far from the origin a position may lie, in blocks (the world border's limit).
const POSITION_LIMIT: f64 = 30_000_000.0;

/// A stream's samples: written by one producer, read by the output callback.
pub(super) struct Ring {
    samples: Box<[AtomicI16]>,
    /// Samples written and consumed since the stream opened; the index is the count modulo the
    /// length. Only the producer moves `tail`, only the callback moves `head`.
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

    /// The producer's tail (in samples), queued frames and free frames, in that order.
    fn producer_window(&self) -> (u64, usize, usize) {
        let tail = self.tail.load(Relaxed);
        let queued = tail - self.head.load(Acquire);
        let free = self.samples.len() as u64 - queued;
        let channels = self.channels as u64;
        (
            tail,
            (queued / channels) as usize,
            (free / channels) as usize,
        )
    }

    /// Write whole frames that the producer has checked fit in its window. No other producer
    /// may write between taking the window and publishing its samples.
    fn publish_frames(&self, tail: u64, samples: &[i16]) {
        let len = self.samples.len() as u64;
        for (i, sample) in samples.iter().enumerate() {
            self.samples[((tail + i as u64) % len) as usize].store(*sample, Relaxed);
        }
        self.tail.store(tail + samples.len() as u64, Release);
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
    /// The stream's `SoundSource` index.
    category: usize,
    range: f32,
    owner: String,
    volume: AtomicU32,
    closed: AtomicBool,
    /// Another thread is pushing: this push is refused, which keeps the ring single-producer.
    pushing: AtomicBool,
    pull: Option<Arc<Demand>>,
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
    fn stop(&self) {
        if !self.closed.swap(true, AcqRel)
            && let Some(pull) = &self.pull
        {
            pull.wake.signal();
        }
    }

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
    push_ring: Option<Weak<Ring>>,
}

impl AudioStreamSource for StreamHandle {
    fn push(&self, samples: &[i16]) -> u32 {
        let Some(ring) = &self.push_ring else {
            return 0;
        };
        let c = &*self.control;
        if c.closed.load(Acquire) || c.pushing.swap(true, Acquire) {
            return 0;
        }
        let accepted = match ring.upgrade() {
            Some(ring) => {
                let channels = ring.channels;
                let (tail, _, free) = ring.producer_window();
                let frames = (samples.len() / channels).min(free);
                ring.publish_frames(tail, &samples[..frames * channels]);
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
        // One atomic holds both flags, so what this reads is a whole placement even while a writer
        // is in the middle of publishing the position: the old one or the new.
        let flags = c.flags.load(Acquire);
        if flags & POSITIONAL != 0 && flags & PLACED == 0 {
            AudioStreamState::Unplaced
        } else {
            AudioStreamState::Playing
        }
    }
    fn stop(&self) {
        self.control.stop();
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
    /// True once there is a current source frame; cleared when the next required frame is missing.
    primed: bool,
    /// The last frame taken from the ring and the output phase relative to it, in source frames.
    current: [f32; 2],
    fraction: f64,
    /// The placement the callback last read whole.
    view: View,
}

/// The main thread's list of open streams, with the caps. Shared by every `Audio` on one output.
#[derive(Default)]
pub(super) struct StreamPool {
    state: Mutex<PoolState>,
    /// How many controls there are, so that with no stream open the main thread's calls and the
    /// tick take no lock at all.
    open: AtomicUsize,
}

#[derive(Default)]
struct PoolState {
    controls: Vec<Arc<Control>>,
    worker: Option<Worker>,
    /// The output went away (a failed reopen): no stream opens until it is back.
    lost: bool,
}

impl PoolState {
    fn close_all(&self) {
        for control in &self.controls {
            control.closed.store(true, Release);
        }
    }
}

impl Drop for StreamPool {
    fn drop(&mut self) {
        let state = self.state.get_mut().unwrap_or_else(|e| e.into_inner());
        state.close_all();
        if let Some(worker) = state.worker.take() {
            worker.shutdown();
        }
    }
}

impl StreamPool {
    fn state(&self) -> MutexGuard<'_, PoolState> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
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
        source: Option<Box<dyn AudioStream>>,
    ) -> Result<Arc<dyn AudioStreamSource>, AudioRefusal> {
        let fault = |fault| Err(AudioRefusal::BadSpec(fault));
        if !matches!(spec.channels, 1 | 2) {
            return fault(SpecFault::Channels);
        }
        if !(AUDIO_STREAM_RATE_MIN..=AUDIO_STREAM_RATE_MAX).contains(&spec.rate) {
            return fault(SpecFault::Rate);
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
        self.reap(mixer);
        let mut state = self.state();
        if state.lost {
            return Err(AudioRefusal::NoOutput);
        }
        if state.controls.len() >= AUDIO_STREAMS_TOTAL as usize {
            return Err(AudioRefusal::TooManyStreams);
        }
        if state.controls.iter().filter(|c| c.owner == owner).count()
            >= AUDIO_STREAMS_PER_MOD as usize
        {
            return Err(AudioRefusal::ModStreamLimit);
        }
        let channels = spec.channels as usize;
        let ring_frames = (spec.rate * AUDIO_STREAM_RING_MAX_MS / 1000) as usize;
        let ring = Arc::new(Ring::new(ring_frames, channels));
        let mut control = Control {
            channels,
            rate: spec.rate,
            category: spec.category.index(),
            range: spec.range,
            owner: owner.to_owned(),
            volume: AtomicU32::new(spec.volume.to_bits()),
            closed: AtomicBool::new(false),
            pushing: AtomicBool::new(false),
            pull: None,
            held: Mutex::new(Held::Flat),
            sequence: AtomicU64::new(0),
            flags: AtomicU8::new(0),
            position: Default::default(),
        };
        let (control, push_ring) = if let Some(source) = source {
            let worker = match &mut state.worker {
                Some(worker) => worker,
                slot @ None => match Worker::start() {
                    Ok(worker) => slot.insert(worker),
                    Err(refusal) => {
                        drop(state);
                        return Err(refusal);
                    }
                },
            };
            let demand = Arc::new(worker.demand());
            control.pull = Some(demand.clone());
            let control = Arc::new(control);
            if let Err(job) = worker.admit(PullJob {
                control: control.clone(),
                demand,
                ring: ring.clone(),
                source,
            }) {
                drop(state);
                drop(job);
                return Err(AudioRefusal::WorkerUnavailable);
            }
            (control, None)
        } else {
            (Arc::new(control), Some(Arc::downgrade(&ring)))
        };
        control.place(&spec.placement);
        // If the mixer cannot accept the voice, this handle closes the job on its worker.
        let handle = StreamHandle {
            control: control.clone(),
            push_ring,
        };
        let voice = StreamVoice {
            control: control.clone(),
            ring,
            primed: false,
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
        state.controls.push(control);
        self.open.store(state.controls.len(), Relaxed);
        Ok(Arc::new(handle))
    }

    /// Lets go of the streams a mod stopped or dropped: their voices leave the mixer, and with
    /// the last of them their rings are freed.
    pub(super) fn reap(&self, mixer: &Mutex<Mixer>) {
        if self.idle() {
            return;
        }
        let mut state = self.state();
        let controls = &mut state.controls;
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
        for control in &self.state().controls {
            control.follow(&mut resolve);
        }
    }

    /// The listener left its level: every positional stream is silent until its mod places it
    /// again.
    pub(super) fn unplace(&self) {
        if self.idle() {
            return;
        }
        for control in &self.state().controls {
            control.unplace();
        }
    }

    /// The output went away (or came back): its streams stop, and none opens until it is back.
    #[cfg(any(feature = "audio", test))]
    pub(super) fn output_lost(&self, lost: bool) {
        let mut state = self.state();
        state.lost = lost;
        if lost {
            state.close_all();
            if let Some(worker) = &state.worker {
                worker.wake();
            }
        }
    }
}

/// After consuming a whole output buffer, ask for enough source frames for the next one,
/// including interpolation's lookahead. One wake suffices for the pool's shared worker.
pub(super) fn request_pull(streams: &[StreamVoice], output_frames: usize, rate: u32) {
    if output_frames == 0 {
        return;
    }
    let mut worker = None;
    for voice in streams {
        let control = &voice.control;
        if let Some(pull) = &control.pull
            && !control.closed.load(Acquire)
            && !pull.eof.load(Acquire)
        {
            let frames = (output_frames as u64)
                .saturating_mul(u64::from(control.rate))
                .div_ceil(u64::from(rate))
                .saturating_add(1);
            let capacity = voice.ring.samples.len() / voice.ring.channels;
            pull.frames
                .store(frames.min(capacity as u64) as usize, Release);
            worker = Some(&pull.wake);
        }
    }
    if let Some(worker) = worker {
        worker.signal();
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
    let output_frames = out.len() / channels;
    if output_frames == 0 {
        return;
    }
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
        // EOF orders every prior publication. Read it before the tail so an EOF snapshot
        // cannot miss the producer's last frames.
        let eof = c.pull.as_ref().is_some_and(|pull| pull.eof.load(Acquire));
        let tail = ring.tail.load(Acquire);
        let mut avail = (tail - head) / ch as u64;
        let mut taken = 0u64;
        // The frame at this offset from the ring head at the start of the buffer.
        let frame_at = |frame: u64| -> [f32; 2] {
            let base = head + frame * ch as u64;
            let sample = |i: u64| {
                f32::from(ring.samples[((base + i) % len) as usize].load(Relaxed)) / 32768.0
            };
            let left = sample(0);
            [left, if ch > 1 { sample(1) } else { left }]
        };
        if !voice.primed {
            if avail == 0 {
                if eof {
                    c.stop();
                }
                continue;
            }
            voice.current = frame_at(0);
            voice.fraction = 0.0;
            taken = 1;
            avail -= 1;
            voice.primed = true;
        }
        let volume = f32::from_bits(c.volume.load(Relaxed)) * category_gain[c.category];
        // Gain per output channel, and the one a mono output takes (no panning); NaN anywhere
        // is silence. Skip reading placement if the stream has no volume.
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
        if left == 0.0 && right == 0.0 && mono == 0.0 {
            // Inaudible (muted, no volume, unplaced, out of range): no interpolation or mixing.
            // Advance the resampler and ring head as far as audible output would have taken them.
            let wanted = (voice.fraction + step * (output_frames - 1) as f64) as u64;
            let advance = wanted.min(avail);
            if advance > 0 {
                voice.current = frame_at(taken + advance - 1);
            }
            taken += advance;
            if wanted > advance {
                voice.primed = false;
            } else {
                voice.fraction += step * output_frames as f64 - wanted as f64;
            }
        } else {
            'output: for frame in out.chunks_exact_mut(channels) {
                // Look for a source frame only when the output needs it. If none is queued, leave
                // the rest silent and start again with the next frame that arrives.
                while voice.fraction >= 1.0 {
                    if avail == 0 {
                        voice.primed = false;
                        break 'output;
                    }
                    voice.current = frame_at(taken);
                    taken += 1;
                    avail -= 1;
                    voice.fraction -= 1.0;
                }
                // Interpolate between source frames. With no next frame yet, hold the last one
                // for the rest of its duration.
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
        }
        let head = head + taken * ch as u64;
        ring.head.store(head, Release);
        if eof && head == tail && (!voice.primed || voice.fraction >= 1.0) {
            c.stop();
        }
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
            self.pool.open(&self.mixer, owner, spec, None)
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
    fn spec() -> StreamSpec {
        StreamSpec::new(SoundSource::Voice, 1, 48_000)
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
    fn an_async_stream_starts_and_resumes_with_one_frame() {
        let rig = Rig::new();
        let stream = rig
            .open("mod", &StreamSpec::new(SoundSource::Voice, 1, 36_000))
            .unwrap();
        assert_eq!(stream.state(), AudioStreamState::Playing);
        assert_eq!(rig.play(2), [0.0; 4]);
        assert_eq!(stream.push(&[16_384]), 1);
        // At 36 kHz a single frame is still current at output phases 0 and 0.75, in
        // separate buffers here. At 1.5 the source goes dry, without ending the stream.
        assert_eq!(rig.play(1), [0.5; 2]);
        assert_eq!(rig.play(3), [0.5, 0.5, 0.0, 0.0, 0.0, 0.0]);
        assert_eq!(stream.state(), AudioStreamState::Playing);
        assert_eq!(stream.push(&[-8192]), 1);
        // Resuming starts at phase zero, not partway through the new frame.
        assert_eq!(rig.play(3), [-0.25, -0.25, -0.25, -0.25, 0.0, 0.0]);
    }

    #[test]
    fn a_slower_stream_is_interpolated_and_keeps_its_last_frame_for_its_whole_length() {
        let rig = Rig::new();
        let stream = rig
            .open("mod", &StreamSpec::new(SoundSource::Voice, 1, 24_000))
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
    fn an_async_queue_keeps_frame_order_when_a_burst_wraps_a_full_ring() {
        let rig = Rig::new();
        let stream = rig
            .open("mod", &StreamSpec::new(SoundSource::Voice, 2, 48_000))
            .unwrap();
        // One second of stereo PCM; distinguish both ends and both channels.
        let mut queued = vec![0; 96_000];
        queued[..4].copy_from_slice(&[8192, -8192, 4096, -4096]);
        queued[95_998..].copy_from_slice(&[2048, -2048]);
        assert_eq!(stream.push(&queued), 48_000);
        let burst = [16_384, -16_384, -8192, 8192, 4096, -4096];
        assert!(rig.play(0).is_empty());
        assert_eq!(stream.push(&burst), 0);
        assert_eq!(rig.play(2), [0.25, -0.25, 0.125, -0.125]);

        // The next push can reuse the two slots just played, but not overwrite the rest.
        assert_eq!(stream.push(&burst), 2);
        let remaining = rig.play(47_998);
        let (silence, last) = remaining.split_last_chunk::<2>().unwrap();
        assert!(silence.iter().all(|&sample| sample == 0.0));
        assert_eq!(*last, [0.0625, -0.0625]);
        assert_eq!(rig.play(3), [0.5, -0.5, -0.25, 0.25, 0.0, 0.0]);
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
        assert_eq!(stream.state(), AudioStreamState::Unplaced);
        assert_eq!(stream.push(&[8192, 16_384, -16_384, -8192]), 4);
        rig.pool.follow_entities(|_| None);
        assert_eq!(rig.play(2), [0.0; 4]);
        assert_eq!(stream.state(), AudioStreamState::Unplaced);
        // Tracked again, at a place a NaN spoils: still unplaced; at the listener: heard.
        rig.pool.follow_entities(|_| Some([f64::NAN, 0.0, 0.0]));
        assert_eq!(stream.state(), AudioStreamState::Unplaced);
        rig.pool
            .follow_entities(|e| (e == entity).then_some([0.0; 3]));
        assert_eq!(stream.state(), AudioStreamState::Playing);
        let mut out = [0.0; 2];
        mix_output(&mut rig.mixer.lock().unwrap(), &mut out, 1, 48_000);
        // At zero distance, mono output has unit gain. The first two source frames were silent.
        assert_eq!(out, [-0.5, -0.25]);
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
                None,
            )
            .unwrap();
        let flat = audio.open_stream("mod", &spec(), None).unwrap();
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
                None,
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

    #[test]
    fn a_muted_stream_advances_and_resumes_after_running_dry() {
        let rig = Rig::new();
        let stream = rig
            .open("mod", &StreamSpec::new(SoundSource::Music, 1, 36_000))
            .unwrap();
        assert_eq!(stream.push(&[4096, 8192, 16_384, -16_384, -8192]), 5);
        let set_gain = |gain| {
            rig.mixer.lock().unwrap().stream_gain[SoundSource::Music.index()] = gain;
        };
        set_gain(0.0);
        assert_eq!(rig.play(1), [0.0; 2]);
        assert_eq!(rig.play(2), [0.0; 4]);
        assert_eq!(stream.state(), AudioStreamState::Playing);
        set_gain(1.0);
        // Three muted frames advance 2.25 source frames. Unmuting is a quarter of the way
        // between 0.5 and -0.5, then exactly at -0.5.
        assert_eq!(rig.play(2), [0.25, 0.25, -0.5, -0.5]);
        set_gain(0.0);
        // These output phases are 3.75, 4.5 and 5.25. The last needs PCM that is not there.
        assert_eq!(rig.play(3), [0.0; 6]);
        assert_eq!(stream.state(), AudioStreamState::Playing);
        assert_eq!(stream.push(&[16_384]), 1);
        set_gain(1.0);
        assert_eq!(rig.play(2), [0.5; 4]);
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
        rig.pool.state().controls[0].sequence.store(1, Relaxed);
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
        let handle = rig.pool.state().controls[0].clone();
        handle.pushing.store(true, Release);
        assert_eq!(stream.push(&half(10)), 0);
        handle.pushing.store(false, Release);
        assert_eq!(stream.push(&half(10)), 10);
    }
}
