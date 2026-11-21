//! A client mod's PCM streams: the mixer's worker pulls from a native [`AudioStream`], or an
//! async producer pushes through [`AudioStreamRef`]. Both use their own pool, never vanilla's
//! streaming slots.
use super::types::SoundSource;
use crate::EntityRef;
use std::sync::Arc;

/// The most streams one mod holds open at once ([`AudioRefusal::ModStreamLimit`]).
pub const AUDIO_STREAMS_PER_MOD: u32 = 32;
/// The most streams open at once across every mod ([`AudioRefusal::TooManyStreams`]).
pub const AUDIO_STREAMS_TOTAL: u32 = 64;
/// The lowest sample rate a stream takes, in Hz.
pub const AUDIO_STREAM_RATE_MIN: u32 = 8_000;
/// The highest sample rate a stream takes, in Hz.
pub const AUDIO_STREAM_RATE_MAX: u32 = 48_000;
/// The capacity of an async push stream's queue, in milliseconds of source audio.
pub const AUDIO_STREAM_RING_MAX_MS: u32 = 1_000;

/// A native source of PCM owned by the mixer. Its worker calls [`read`](Self::read) serially
/// on demand, never in the device callback or while holding mixer or pool locks.
///
/// The source runs in-process, with no `Host` or world borrow and no reentry into the game.
/// Reads must do bounded, nonblocking work: do not wait on I/O, locks, another thread or a tick.
/// Muting or unplacing the stream does not pause demand. The worker drops an admitted source
/// at EOF or after stop, without waiting for another device callback.
// no wire: the mixer owns and calls this native source in-process.
pub trait AudioStream: Send + 'static {
    /// Fill at most `pcm.len() / channels` whole frames of interleaved signed 16-bit PCM in the
    /// stream's [`StreamSpec`] format. The buffer is cleared before each call. Report only
    /// frames supplied; a count larger than the offered buffer stops the stream.
    fn read(&mut self, pcm: &mut [i16]) -> AudioRead;
}

/// What a pull source supplied in one [`AudioStream::read`].
// no wire: a native source returns this directly to the mixer worker.
// exhaustive: a read either supplies frames or ends the source.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AudioRead {
    /// The number of whole interleaved frames written, not the number of samples. A short read
    /// plays what is ready; zero is temporary silence. The worker retries on later demand.
    Frames(usize),
    /// Permanently out of PCM. Return this on a separate read after reporting any final frames;
    /// the mixer plays them for their full duration before stopping the stream.
    Eof,
}

/// Where a stream is heard. Positions are in the client's current level.
/// This is local playback state. Send the emitter's network entity ID in your own payload,
/// and resolve it to a local handle with `host.entity(id)` before constructing `Entity`.
///
/// ```compile_fail,E0277
/// fn requires_wire<T: mod_api::wire::Wire>() {}
/// requires_wire::<mod_api::StreamPlacement>();
/// ```
// exhaustive: mods build every form and the mixer matches them without a wildcard
// no wire: placement can contain a client-local entity handle
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum StreamPlacement {
    /// At the listener, wherever they move, with no panning and no fading with distance. The only
    /// placement a stereo stream takes.
    Flat,
    /// At a point, in blocks: panned and faded by `range` as a positioned sound is.
    At([f64; 3]),
    /// At an entity, followed each frame. While the entity is untracked, unloaded or removed the
    /// stream reads [`AudioStreamState::Unplaced`] and is silent, its audio still consumed; it
    /// is heard again if the same entity is tracked again.
    Entity(EntityRef),
}

/// What an audio stream reads (`AudioStreamRef::state`). The state describes this client's mixer,
/// not the sender of the audio, and is not a payload type.
///
/// ```compile_fail,E0277
/// fn requires_wire<T: mod_api::wire::Wire>() {}
/// requires_wire::<mod_api::AudioStreamState>();
/// ```
// no wire: playback state is local to the mixer
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum AudioStreamState {
    /// Open and placed (or flat), even if silent because it is muted or has no PCM ready.
    Playing,
    /// Open, but at a placement nobody hears: an `Entity` the client doesn't track, or any
    /// positional stream after the listener changed dimension, until the next `set_placement`.
    /// The audio is still consumed, so no latency builds up.
    Unplaced,
    /// Over: the pull source ended and its PCM drained, it reported an invalid frame count,
    /// `stop` was called, the handle was dropped, or the game's sound output went away.
    /// It never plays again.
    #[default]
    Stopped,
}

/// The format, placement and volume used to open an audio stream. Construct this on the playing
/// client rather than sending it as a payload.
///
/// ```compile_fail,E0277
/// fn requires_wire<T: mod_api::wire::Wire>() {}
/// requires_wire::<mod_api::StreamSpec>();
/// ```
// no wire: stream setup includes local playback placement
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct StreamSpec {
    /// The mixer group, scaled by master times the group, as every sound is. Voice chat plays on
    /// [`SoundSource::Voice`], the "Voice/Speech" slider.
    pub category: SoundSource,
    /// 1 or 2. Only mono is positioned, as OpenAL does: a stereo stream takes only
    /// [`StreamPlacement::Flat`].
    pub channels: u8,
    /// The rate of the source PCM, in Hz, from [`AUDIO_STREAM_RATE_MIN`] to
    /// [`AUDIO_STREAM_RATE_MAX`]. The mixer resamples to the output's.
    pub rate: u32,
    /// Where it is heard.
    pub placement: StreamPlacement,
    /// For a positioned stream, the distance in blocks at which it has faded to silence, in a
    /// straight line (`AL_LINEAR_DISTANCE`, as a sound's range). Positive and finite.
    pub range: f32,
    /// The stream's own volume, 0 to 1, before the group's and the master's.
    pub volume: f32,
}

impl StreamSpec {
    /// A flat stream on `category`: `channels` (1 or 2) at `rate` Hz, range 16 blocks (a sound's
    /// default) and volume 1.
    pub fn new(category: SoundSource, channels: u8, rate: u32) -> Self {
        Self {
            category,
            channels,
            rate,
            placement: StreamPlacement::Flat,
            range: 16.0,
            volume: 1.0,
        }
    }
    /// The same stream at `placement`.
    pub fn with_placement(self, placement: StreamPlacement) -> Self {
        Self { placement, ..self }
    }
    /// The same stream fading out at `range` blocks.
    pub fn with_range(self, range: f32) -> Self {
        Self { range, ..self }
    }
    /// The same stream at `volume`.
    pub fn with_volume(self, volume: f32) -> Self {
        Self { volume, ..self }
    }
}

/// What the playing client found wrong with a [`StreamSpec`]. This is a local validation result.
///
/// ```compile_fail,E0277
/// fn requires_wire<T: mod_api::wire::Wire>() {}
/// requires_wire::<mod_api::SpecFault>();
/// ```
// no wire: spec faults report local validation
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum SpecFault {
    /// `channels` is not 1 or 2.
    Channels,
    /// `rate` is outside [`AUDIO_STREAM_RATE_MIN`] to [`AUDIO_STREAM_RATE_MAX`].
    Rate,
    /// `volume` is not a number from 0 to 1.
    Volume,
    /// `range` is not a positive, finite number of blocks.
    Range,
    /// A stereo stream with a placement other than [`StreamPlacement::Flat`]: only mono is
    /// positioned.
    StereoPositioned,
    /// A point that is not finite or lies past 30 million blocks.
    Position,
}

/// Why a request to open an audio stream opened nothing. This is a local result, not a network error.
///
/// ```compile_fail,E0277
/// fn requires_wire<T: mod_api::wire::Wire>() {}
/// requires_wire::<mod_api::AudioRefusal>();
/// ```
// no wire: stream refusals report local mixer or host conditions
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum AudioRefusal {
    /// This is a server: it has no output to play on.
    OnServer,
    /// The call came outside the phase that allows it (a client's setup or later) or from no mod.
    Unavailable,
    /// The spec has a value out of range.
    BadSpec(SpecFault),
    /// The game has no sound output (no device, or sound off).
    NoOutput,
    /// The calling mod already holds [`AUDIO_STREAMS_PER_MOD`] streams: stop one first.
    ModStreamLimit,
    /// [`AUDIO_STREAMS_TOTAL`] streams are open across every mod: stop one first.
    TooManyStreams,
    /// The pull worker could not start or admit this source.
    WorkerUnavailable,
}
impl std::fmt::Display for AudioRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::OnServer => f.write_str("a server has no sound output"),
            Self::Unavailable => f.write_str("audio streams cannot be opened in this phase"),
            Self::BadSpec(fault) => write!(f, "the stream spec is refused: {fault}"),
            Self::NoOutput => f.write_str("the game has no sound output"),
            Self::ModStreamLimit => write!(
                f,
                "this mod already holds {AUDIO_STREAMS_PER_MOD} audio streams"
            ),
            Self::TooManyStreams => {
                write!(f, "{AUDIO_STREAMS_TOTAL} audio streams are already open")
            }
            Self::WorkerUnavailable => f.write_str("the audio pull worker cannot accept a source"),
        }
    }
}
impl std::error::Error for AudioRefusal {}
impl crate::AccessRefusal for AudioRefusal {
    fn unavailable() -> Self {
        Self::Unavailable
    }
}
impl std::fmt::Display for SpecFault {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Channels => "channels is not 1 or 2",
            Self::Rate => "rate is outside 8000 to 48000 Hz",
            Self::Volume => "volume is not a number from 0 to 1",
            Self::Range => "range is not a positive, finite number of blocks",
            Self::StereoPositioned => "only a mono stream is positioned",
            Self::Position => "the position is not finite or lies past 30 million blocks",
        })
    }
}

/// The game's side of one open audio stream. A mod reaches it through [`AudioStreamRef`], not
/// directly. Every method works from any thread; `push` and the controls never wait on the mixer.
pub trait AudioStreamSource: Send + Sync + 'static {
    /// Queues interleaved `samples`: whole frames only (a trailing odd sample of a stereo stream
    /// is not taken). Returns the frames accepted, which falls short of the frames offered when
    /// the ring is full, or when another thread is pushing to the same stream at that moment
    /// (the ring has one producer: that push returns 0 and drops nothing already queued): the
    /// rest is refused, never delayed. A pull stream accepts no pushes.
    fn push(&self, samples: &[i16]) -> u32;
    /// Moves the stream; false, changing nothing, for a stereo stream and a placement other than
    /// `Flat`, for a position that is not finite or lies past 30 million blocks, and once the
    /// stream is stopped.
    fn set_placement(&self, placement: &StreamPlacement) -> bool;
    /// Sets the stream's own volume; false, changing nothing, for a value that is not a number
    /// from 0 to 1, and once the stream is stopped.
    fn set_volume(&self, volume: f32) -> bool;
    /// Whether the stream is open and placed, unplaced or stopped, regardless of queued PCM.
    fn state(&self) -> AudioStreamState;
    /// Ends the stream for good: it is silent from the next output buffer and its audio is
    /// dropped. Dropping the last handle does the same.
    fn stop(&self);
}

/// The controls for an open mod audio stream, with push access only for async push streams.
///
/// Keep the handle while the stream should play. Dropping it stops the stream, as [`stop`]
/// does. The handle can be used from any thread, including a `Network`-thread payload handler;
/// neither push nor pull waits for a client tick.
///
/// [`stop`]: AudioStreamRef::stop
// no wire: the handle controls a stream on this client's mixer.
// exhaustive: its field is private, so mods cannot build or destructure it
pub struct AudioStreamRef {
    source: Arc<dyn AudioStreamSource>,
}

impl AudioStreamRef {
    /// A handle on the game's `source`.
    // hidden: a game-side constructor, not part of the mod surface
    #[doc(hidden)]
    pub fn new(source: Arc<dyn AudioStreamSource>) -> Self {
        Self { source }
    }
    /// Queues interleaved signed 16-bit PCM (`channels` samples per frame) and returns the
    /// number of whole frames accepted. A trailing incomplete frame is not queued. The ring
    /// holds [`AUDIO_STREAM_RING_MAX_MS`] milliseconds of source audio; drop what does not fit
    /// or push it again later. The mixer starts with the first available frame and plays
    /// silence when the stream runs dry.
    ///
    /// A push writes straight into the ring made when the stream opened: it never blocks,
    /// allocates or waits for a tick. Only one thread can push at a time: an overlapping push
    /// accepts nothing and returns 0. A pull stream always returns 0.
    pub fn push(&self, samples: &[i16]) -> u32 {
        self.source.push(samples)
    }
    /// Moves the stream. False, changing nothing, for a stereo stream and a placement other than
    /// [`StreamPlacement::Flat`], for a position that is not finite or lies past 30 million
    /// blocks, and once the stream has stopped. A stream a dimension change made
    /// [`AudioStreamState::Unplaced`] is heard again with this call.
    pub fn set_placement(&self, placement: &StreamPlacement) -> bool {
        self.source.set_placement(placement)
    }
    /// Sets the stream's own volume (0 to 1), from the next output buffer. False, changing
    /// nothing, for a value that is not a number in that range, and once the stream has stopped.
    pub fn set_volume(&self, volume: f32) -> bool {
        self.source.set_volume(volume)
    }
    /// Whether the stream is open and placed, unplaced or stopped. A dry or muted stream is
    /// still [`AudioStreamState::Playing`] if it is open and placed.
    pub fn state(&self) -> AudioStreamState {
        self.source.state()
    }
    /// Ends the stream for good. It happens on drop too.
    pub fn stop(&self) {
        self.source.stop()
    }
}
