//! A client mod's audio streams (`AudioClientHost::open_audio_stream`): PCM the mod makes, played by
//! the game's mixer in place of an Ogg file (NeoForge's `SoundInstance.getStream`, vanilla's
//! `AudioStream`), on a pool of their own that never takes one of vanilla's streaming slots.
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
/// The shortest `prebuffer_ms` of a stream.
pub const AUDIO_STREAM_PREBUFFER_MIN_MS: u32 = 20;
/// The longest `prebuffer_ms` of a stream.
pub const AUDIO_STREAM_PREBUFFER_MAX_MS: u32 = 500;
/// The most audio a stream's ring holds, in milliseconds, whatever its `prebuffer_ms`.
pub const AUDIO_STREAM_RING_MAX_MS: u32 = 1_000;

/// Where a stream is heard. Positions are in the client's current level.
// exhaustive: mods build every form and the mixer matches them without a wildcard
#[derive(Clone, Copy, Debug, PartialEq, sunrise_mod_api_macros::Wire)]
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

/// What an audio stream reads (`AudioStreamRef::state`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub enum AudioStreamState {
    /// Silent while the ring fills to `prebuffer_ms`: new, or after it ran dry.
    Buffering,
    /// Audible: the mixer is playing what was pushed.
    Playing,
    /// Playing, but at a placement nobody hears: an `Entity` the client doesn't track, or any
    /// positional stream after the listener changed dimension, until the next `set_placement`.
    /// The audio is still consumed, so no latency builds up.
    Unplaced,
    /// Over: `stop` was called, the handle was dropped, or the game's sound output went away (a
    /// device that could not be opened again). It never plays again. What a stream reads when a
    /// call to the game was refused.
    #[default]
    Stopped,
}

/// What a stream is opened with (`open_audio_stream`).
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub struct StreamSpec {
    /// The mixer group, scaled by master times the group, as every sound is. Voice chat plays on
    /// [`SoundSource::Voice`], the "Voice/Speech" slider.
    pub category: SoundSource,
    /// 1 or 2. Only mono is positioned, as OpenAL does: a stereo stream takes only
    /// [`StreamPlacement::Flat`].
    pub channels: u8,
    /// The rate of the samples pushed, in Hz, from [`AUDIO_STREAM_RATE_MIN`] to
    /// [`AUDIO_STREAM_RATE_MAX`]. The mixer resamples to the output's.
    pub rate: u32,
    /// Where it is heard.
    pub placement: StreamPlacement,
    /// For a positioned stream, the distance in blocks at which it has faded to silence, in a
    /// straight line (`AL_LINEAR_DISTANCE`, as a sound's range). Positive and finite.
    pub range: f32,
    /// The stream's own volume, 0 to 1, before the group's and the master's.
    pub volume: f32,
    /// How much audio is buffered before playing starts, and again after an underrun, from
    /// [`AUDIO_STREAM_PREBUFFER_MIN_MS`] to [`AUDIO_STREAM_PREBUFFER_MAX_MS`]. The ring that holds
    /// it is `2 * prebuffer_ms + 100` milliseconds long, at most [`AUDIO_STREAM_RING_MAX_MS`],
    /// allocated when the stream opens.
    pub prebuffer_ms: u32,
}

impl StreamSpec {
    /// A flat stream on `category`: `channels` (1 or 2) at `rate` Hz, range 16 blocks (a sound's
    /// default), volume 1 and a 100 ms prebuffer.
    pub fn new(category: SoundSource, channels: u8, rate: u32) -> Self {
        Self {
            category,
            channels,
            rate,
            placement: StreamPlacement::Flat,
            range: 16.0,
            volume: 1.0,
            prebuffer_ms: 100,
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
    /// The same stream buffering `prebuffer_ms` before it plays.
    pub fn with_prebuffer_ms(self, prebuffer_ms: u32) -> Self {
        Self {
            prebuffer_ms,
            ..self
        }
    }
}

/// What was wrong with a [`StreamSpec`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub enum SpecFault {
    /// `channels` is not 1 or 2.
    Channels,
    /// `rate` is outside [`AUDIO_STREAM_RATE_MIN`] to [`AUDIO_STREAM_RATE_MAX`].
    Rate,
    /// `prebuffer_ms` is outside [`AUDIO_STREAM_PREBUFFER_MIN_MS`] to
    /// [`AUDIO_STREAM_PREBUFFER_MAX_MS`].
    Prebuffer,
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

/// Why `open_audio_stream` opened nothing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
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
            Self::Prebuffer => "prebuffer_ms is outside 20 to 500",
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
    /// rest is refused, never delayed.
    fn push(&self, samples: &[i16]) -> u32;
    /// Moves the stream; false, changing nothing, for a stereo stream and a placement other than
    /// `Flat`, for a position that is not finite or lies past 30 million blocks, and once the
    /// stream is stopped.
    fn set_placement(&self, placement: &StreamPlacement) -> bool;
    /// Sets the stream's own volume; false, changing nothing, for a value that is not a number
    /// from 0 to 1, and once the stream is stopped.
    fn set_volume(&self, volume: f32) -> bool;
    /// Where the stream is.
    fn state(&self) -> AudioStreamState;
    /// The audio waiting to play, in milliseconds.
    fn buffered_ms(&self) -> u32;
    /// Ends the stream for good: it is silent from the next output buffer and its audio is
    /// dropped. Dropping the last handle does the same.
    fn stop(&self);
}

/// A stream a mod opened with `open_audio_stream`: push PCM and the game plays it.
///
/// Keep the handle while the stream should play. Dropping it stops the stream, as [`stop`]
/// does. It is `Send`: a mod may push from any thread, a `Network`-thread payload handler
/// included, and the audio never waits for a tick.
///
/// [`stop`]: AudioStreamRef::stop
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
    /// Queues interleaved 16-bit samples (`channels` per frame) behind what is waiting and
    /// returns the frames accepted. The ring has a fixed size: what does not fit is refused,
    /// never delayed, so a stalled sender cannot build up latency, and the caller drops it or
    /// pushes it again later. A push writes straight into the ring made when the stream opened:
    /// it never blocks, allocates or waits for a tick. The ring has one
    /// producer: if a second thread pushes to the same stream at the same moment, that push
    /// accepts nothing and returns 0. A native mod that shares the handle across threads
    /// pushes from one thread at a time (a lock of its own, or one pushing thread).
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
    /// Where the stream is: buffering, playing, unplaced or stopped. An underrun plays silence
    /// and reads `Buffering` again until `prebuffer_ms` has been pushed.
    pub fn state(&self) -> AudioStreamState {
        self.source.state()
    }
    /// The audio waiting to play, in milliseconds of the stream's own rate.
    pub fn buffered_ms(&self) -> u32 {
        self.source.buffered_ms()
    }
    /// Ends the stream for good. It happens on drop too.
    pub fn stop(&self) {
        self.source.stop()
    }
}
