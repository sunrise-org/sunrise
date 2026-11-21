//! Host method declarations; access defaults and game forwarders are generated.
use crate::*;
/// Audio streams for [`crate::ClientHost`]. Both openers use a pool separate from vanilla's
/// sound instances, fire no sound hooks and require a mod call on a live client from setup
/// onward. The mixer resamples to the output's rate and applies the stream's volume, category
/// and master gain, `muffle_sounds`, and the position and linear range fade for spatial mono.
/// Muted and unplaced streams still advance. Keep the returned handle for as long as playback
/// should continue; dropping it stops the stream.
#[sunrise_mod_api_macros::host_area]
pub trait AudioClientHost {
    /// Side: client.
    /// Gives the mixer a native source of interleaved signed 16-bit PCM, like NeoForge's
    /// `SoundInstance.getStream`. The mixer takes ownership and calls [`AudioStream::read`]
    /// serially on the output's shared worker as it needs frames, with no `Host` or world borrow.
    /// The device callback requests PCM but never calls or waits for the source. A short or dry
    /// read is retried on later demand. EOF lets ready frames drain before playback stops.
    /// [`AudioStreamRef::push`] always returns zero on this handle.
    ///
    /// Refuses with [`AudioRefusal`] when playback is unavailable, the spec is invalid, a stream
    /// limit is reached, or the pull worker cannot take the source.
    ///
    /// ```
    /// use mod_api::{AudioRead, AudioStream, SoundSource, StreamSpec};
    /// # use mod_api::{AudioRefusal, AudioStreamRef, ClientHost};
    ///
    /// struct Clip(std::vec::IntoIter<i16>);
    /// impl AudioStream for Clip {
    ///     fn read(&mut self, pcm: &mut [i16]) -> AudioRead {
    ///         let mut frames = 0;
    ///         for (out, sample) in pcm.iter_mut().zip(self.0.by_ref()) {
    ///             *out = sample;
    ///             frames += 1; // One sample per frame in this mono stream.
    ///         }
    ///         if frames == 0 { AudioRead::Eof } else { AudioRead::Frames(frames) }
    ///     }
    /// }
    ///
    /// # fn open(host: &dyn ClientHost, samples: Vec<i16>) -> Result<AudioStreamRef, AudioRefusal> {
    /// let spec = StreamSpec::new(SoundSource::Music, 1, 48_000);
    /// host.open_audio_stream(&spec, Box::new(Clip(samples.into_iter())))
    /// # }
    /// ```
    #[access(ClientSetup)]
    fn open_audio_stream(
        &self,
        _spec: &StreamSpec,
        _source: Box<dyn AudioStream>,
    ) -> Result<AudioStreamRef, AudioRefusal>;

    /// Side: client.
    /// Opens a stream for an async producer, such as a network payload handler. Use
    /// [`AudioStreamRef::push`] from one producer thread per stream. It returns the whole
    /// frames the bounded ring accepted, without waiting for the mixer or a tick. Playback
    /// starts with the first available frame and plays silence when dry. Stop or drop the
    /// handle to end the stream.
    ///
    /// Refuses with [`AudioRefusal`] when playback is unavailable, the spec is invalid or a
    /// stream limit is reached.
    ///
    /// ```
    /// use mod_api::{AudioRefusal, AudioStreamRef, ClientHost, SoundSource, StreamSpec};
    ///
    /// fn open(host: &dyn ClientHost) -> Result<AudioStreamRef, AudioRefusal> {
    ///     host.open_push_audio_stream(&StreamSpec::new(SoundSource::Voice, 2, 48_000))
    /// }
    ///
    /// // On the producer thread: drop the suffix or retry it later.
    /// fn push<'a>(stream: &AudioStreamRef, stereo_pcm: &'a [i16]) -> &'a [i16] {
    ///     let frames = stream.push(stereo_pcm) as usize;
    ///     &stereo_pcm[frames * 2..]
    /// }
    /// ```
    #[access(ClientSetup)]
    fn open_push_audio_stream(&self, _spec: &StreamSpec) -> Result<AudioStreamRef, AudioRefusal>;
}
