//! Installed mod code loading above the engine (docs/layers.md, #2534).
//! The platform prepares the engine, loads mods here, then makes one engine start call.
#![deny(unsafe_code)]

#[allow(unsafe_code)]
pub mod native_libraries;
pub use native_libraries::{GAME_BUILD_ID, RUSTC_VERSION, main};

mod loading;
pub use loading::main_with;

mod mods_button;

#[cfg(test)]
#[allow(dead_code)]
#[path = "../build.rs"]
mod build_script;
