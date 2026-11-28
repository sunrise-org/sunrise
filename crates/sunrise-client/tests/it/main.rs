//! The integration tests of this crate, linked into one binary: a binary per file linked the crate and its
//! dependencies again for every file. A new test file goes in this folder with a `mod` line here.

mod_platform::__install_engine_test_platform!();

#[cfg(feature = "client")]
mod cut_layout;
mod datapack_biome_sync;
mod mod_api_hooks;
mod mod_biome_sync;
#[cfg(feature = "client")]
mod skull_models;
