//! The game opens client mods' pull and push audio streams on the mixer's separate pool and
//! places the entity-following ones each frame.
use super::Game;
use mod_api::{AudioRefusal, AudioStream, AudioStreamRef, StreamSpec};

/// The host marks the calling native mod while its hook runs. A call outside a mod's scope
/// opens nothing.
fn calling_mod() -> Result<&'static str, AudioRefusal> {
    mod_api::content::crash::current_native_mod().ok_or(AudioRefusal::Unavailable)
}

impl Game<'_> {
    /// Opens a stream the mixer pulls from on its worker.
    pub(in crate::game) fn open_audio_stream(
        &self,
        spec: &StreamSpec,
        source: Box<dyn AudioStream>,
    ) -> Result<AudioStreamRef, AudioRefusal> {
        self.open_mod_audio_stream(spec, Some(source))
    }

    /// Opens a stream an async producer pushes into.
    pub(in crate::game) fn open_push_audio_stream(
        &self,
        spec: &StreamSpec,
    ) -> Result<AudioStreamRef, AudioRefusal> {
        self.open_mod_audio_stream(spec, None)
    }

    fn open_mod_audio_stream(
        &self,
        spec: &StreamSpec,
        source: Option<Box<dyn AudioStream>>,
    ) -> Result<AudioStreamRef, AudioRefusal> {
        if !self.native_window() {
            return Err(AudioRefusal::OnServer);
        }
        self.audio
            .open_stream(calling_mod()?, spec, source)
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
