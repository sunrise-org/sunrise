//! Client mods' audio streams (`open_audio_stream`): the game opens them on the mixer's own pool and
//! places the entity-following ones each frame.
use super::Game;
use mod_api::{AudioRefusal, AudioStreamRef, StreamSpec};

/// The mod whose call this is, whichever target it runs on: the host marks it for a native mod's
/// hook. A call from no mod has no per-mod cap to count against and
/// opens nothing.
fn calling_mod() -> Result<&'static str, AudioRefusal> {
    mod_api::content::crash::current_native_mod().ok_or(AudioRefusal::Unavailable)
}

impl Game<'_> {
    /// Opens a stream for the calling mod, on the mixer's pool for mods' audio. Err on a server,
    /// without a sound output, for a spec out of range and past the 32 streams a mod or the 64
    /// all mods may hold.
    pub(in crate::game) fn open_audio_stream(
        &self,
        spec: &StreamSpec,
    ) -> Result<AudioStreamRef, AudioRefusal> {
        if !self.native_window() {
            return Err(AudioRefusal::OnServer);
        }
        self.audio
            .open_stream(calling_mod()?, spec)
            .map(AudioStreamRef::new)
    }
    /// Places every entity-following stream at its entity, or leaves it unplaced where the client
    /// tracks no such entity. Once a frame, beside the listener's own update.
    pub(in crate::game) fn follow_audio_stream_entities(&self) {
        self.audio
            .follow_stream_entities(|entity| self.entity_position(entity));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mod_api::content::crash::{ModCall, enter_native_mod};

    #[test]
    fn each_calling_mod_is_counted_on_its_own_and_no_mod_opens_nothing() {
        assert_eq!(calling_mod(), Err(AudioRefusal::Unavailable));
        let call = |id| ModCall {
            id,
            hook: "client_tick",
            part: None,
        };
        let _a = enter_native_mod(call("mod_a"));
        assert_eq!(calling_mod(), Ok("mod_a"));
        let _b = enter_native_mod(call("mod_b"));
        assert_eq!(calling_mod(), Ok("mod_b"));
    }
}
