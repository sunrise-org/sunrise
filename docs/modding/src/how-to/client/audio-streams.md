# Audio streams

Play audio your mod makes: voice chat, a radio, a music player, proximity audio or a text reader. Open a stream, push 16-bit PCM into it, and the game's mixer plays it. See the [API reference](../../reference/api.md) for the generated signatures.

A stream is not a sound instance. It has a pool of its own and never takes one of vanilla's streaming slots, so music and records start exactly when they did before. No sound hook fires for it, and it is not a sound event: nothing in `sounds.json` names it.

## Opening a stream

```rust
use mod_api::prelude::*;
use mod_api::{SoundSource, StreamPlacement, StreamSpec};

let spec = StreamSpec::new(SoundSource::Voice, 1, 48_000)
    .with_placement(StreamPlacement::Entity(speaker))
    .with_range(32.0)
    .with_prebuffer_ms(100);
let stream = host.open_audio_stream(&spec)?;

// Whenever a frame is ready (interleaved samples, `channels` per frame):
let accepted = stream.push(&samples);
```

`open_audio_stream` is a client call. It refuses with an `AudioRefusal` and opens nothing on a server, without a sound output, for a spec out of range, and past the limits below.

| `StreamSpec` field | Meaning |
|---|---|
| `category` | The mixer group, scaled by master times the group like every sound. Voice chat uses `SoundSource::Voice`, the "Voice/Speech" slider. |
| `channels` | 1 or 2. Only mono is positioned. |
| `rate` | 8000 to 48000 Hz. The mixer resamples to the output's rate. |
| `placement` | `Flat`, `At([x, y, z])` or `Entity(entity)`, in the client's current level. |
| `range` | Blocks at which a positioned stream has faded to silence, in a straight line (`AL_LINEAR_DISTANCE`, as a sound's range). |
| `volume` | 0 to 1, before the group's and the master's. |
| `prebuffer_ms` | 20 to 500. Playing starts once this much is buffered, and again after an underrun. |

The handle has `push`, `set_placement`, `set_volume`, `state`, `buffered_ms` and `stop`. **Dropping the handle stops the stream**, so keep it for as long as the stream should play. Muffling (`muffle_sounds`) applies to streams like every other sound.

A stopped stream's samples are freed by the game, whatever handles you keep: afterwards `push` returns 0, `state` reads `Stopped` and `buffered_ms` reads 0.

## The player's switch

Sunrise Extras has a "Mod audio streams" switch (Sound section), on by default. Off, every mod stream still opens and its audio is still taken in at its own pace, but nothing is mixed and nothing is heard, so a mod needs no special case and builds up no latency. The switch is Sunrise Extras' (`crates/sunrise-official/src/mod_audio.rs`); the engine only owns the gain it sets, and a build without Extras has no switch.

## What `state` says

- `Buffering`: silent while the ring fills, or after it ran dry.
- `Playing`: audible.
- `Unplaced`: playing, but at a place nobody hears (below).
- `Stopped`: `stop` was called, the handle dropped, or the game's sound output went away (a device that could not be opened again). It never plays again; open a new stream once the output is back (`open_audio_stream` answers `NoOutput` until then). Changing the output device does not stop streams.

## Buffering, underruns and overflow

Each stream has a fixed ring, allocated when it opens: `2 * prebuffer_ms + 100` milliseconds, at most one second. Nothing allocates per push or per output buffer.

- **Underrun waits instead of ending.** A dry ring plays silence, with no time-stretching, and the state goes back to `Buffering` until `prebuffer_ms` is queued again. Only `stop` or a drop ends a stream (vanilla's sound ends for good when its queue runs dry; a mod stream is Sunrise's own surface).
- **Overflow refuses the excess.** `push` returns the frames it took; the rest is refused, never delayed, so a sender that stalls and then bursts cannot build up latency. Drop what was refused, or push it again later.
- End to end, the delay is about one output period plus `prebuffer_ms`: a target, not a bound.

## Placement

- A **mono** stream may be `Flat` (at the listener, no panning and no fading), at a point or at an entity. It is panned as a positioned sound is and fades linearly to silence at `range`.
- A **stereo** stream is `Flat` only; a stereo spec with another placement is refused at open with `SpecFault::StereoPositioned`, and `set_placement` returns false.
- An `Entity` stream follows its entity every frame. While the entity is untracked, unloaded or removed the stream is silent and reads `Unplaced`, but its audio is still consumed, so no latency builds up. It plays again if the same entity is tracked again.
- When the listener changes dimension (or leaves the level) every positional stream goes `Unplaced` until the mod calls `set_placement` again, since a point or entity of another level is never heard.

## Threading

`AudioStreamRef` is `Send`, and `push` works from any thread. A native mod can push from a `Network`-thread payload handler, so its audio never waits for a client tick, and a native `push` never waits for the mixer: it writes straight into a lock-free ring made when the stream opened, which the output callback reads, and allocates nothing. The controls (`set_placement`, `set_volume`) take no lock the output callback takes either. Only one thread pushes at a time: a push that overlaps another on the same stream is refused (returns 0), so keep each stream to one pushing thread.

Opening a stream is a host call and runs where the host does. Every call on the handle works from any thread.

## Limits

- 32 streams open per mod (`AudioRefusal::ModStreamLimit`) and 64 across every mod (`AudioRefusal::TooManyStreams`). A stopped stream frees its place.
- Sample rate 8000 to 48000 Hz, 1 or 2 channels, `prebuffer_ms` 20 to 500.
- A call from no mod, or outside the phases a client opens a stream in, answers `AudioRefusal::Unavailable`.
- Bad input never panics: a rate or channel count out of range, a volume or range that is not a number, or a position that is not finite is refused (`AudioRefusal::BadSpec`, or `false` from `set_volume` and `set_placement`).

## Known limits

- There is no echo cancellation: a mod that plays voice while capturing the microphone must deal with its own echo.
- Voice is unmoderated: the engine neither records, filters nor reports a stream's audio.
- Replay recordings capture voice payloads like any other payload.
- This API plays audio. Getting it between players (payload delivery), encoding it and capturing a microphone are separate: a mod brings its own codec as an ordinary dependency.
