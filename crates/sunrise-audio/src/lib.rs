//! Sound playback: vanilla's `SoundEngine` bookkeeping (`sound_engine`) over a
//! small mixer standing in for OpenAL, Ogg/Vorbis decoding, microphone capture
//! and the sound sources and their volumes. No GPU: `sunrise-client-render`
//! builds on it and re-exports each module under its old path
//! (`sunrise_client_render::audio`, ...).
#![forbid(unsafe_code)]
pub mod audio;
pub mod settings;
pub mod sound_engine;
pub mod sound_events;
pub mod sound_preview;
pub use sunrise_client_core::Result;
pub use sunrise_client_core::api_context;
pub use sunrise_client_core::random_source;
