//! Host method declarations; access defaults and game forwarders are generated.
use crate::*;
/// The client host's methods of the `audio` area, implemented by the game and reached through [`crate::ClientHost`].
#[sunrise_mod_api_macros::host_area]
pub trait AudioClientHost {
    /// Side: client.
    /// Opens a stream the game's mixer plays: the mod pushes 16-bit PCM and hears it, in place
    /// of the Ogg decoder behind a sound (NeoForge's `SoundInstance.getStream`), for voice chat,
    /// a radio, a music player, proximity audio or a reader. A stream has a pool of its own and
    /// never takes one of vanilla's streaming slots, so music and records start as they always
    /// do. It plays at the output's rate behind the `spec`'s category volume (master times the
    /// group, as every sound's) and the muffle (`muffle_sounds`), at its placement, fading
    /// linearly to silence at its range. The stream is not a sound instance: no sound hook
    /// fires for it. The handle's `push` works from any thread, so audio never waits for a tick.
    /// Dropping the handle stops the stream. Returns `Err` without opening a stream on a server,
    /// without a sound output, for a spec out of range (a stereo stream
    /// with a placement other than `Flat` included), for a mod already holding 32 streams, when
    /// 64 are open across every mod, and for a call from no mod or outside the phases a client
    /// opens one in (`Unavailable`).
    #[access(ClientSetup)]
    fn open_audio_stream(&self, _spec: &StreamSpec) -> Result<AudioStreamRef, AudioRefusal>;
}
