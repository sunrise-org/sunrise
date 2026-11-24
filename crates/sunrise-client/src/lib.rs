//! Gameplay and startup, above the reusable client library.
// No `#![recursion_limit]` or other inner attribute horizon-mixins doesn't know here: the
// launcher's horizon-mixins then rejects every native mod targeting this crate (#8169).
#![forbid(unsafe_code)]
extern crate self as sunrise_client;
pub use sunrise_client_lib::carrier_store;
pub use sunrise_client_lib::*;
// The parity fixtures' build compiles the lib test's helpers without the tests that use most of them.
#[cfg_attr(
    sunrise_parity_fixtures,
    allow(dead_code, unused_imports, unused_macros)
)]
#[doc(hidden)]
pub mod entry;

// The game's allocator lives in the library, not the executable: the prebuilt game (#589) is this crate as a
// dylib, and the allocator must be the one its code and the mods' executable share. The Windows and macOS archives
// build with `system-allocator`: their loaders bind each library to std's own allocator, which everything then
// shares (#2394, #3877).
#[cfg(not(feature = "system-allocator"))]
#[global_allocator]
static ALLOCATOR: sunrise_threads::GameAllocator = sunrise_threads::GAME_ALLOCATOR;
#[cfg_attr(
    sunrise_parity_fixtures,
    allow(dead_code, unused_imports, unused_macros)
)]
pub mod game;
pub mod orb_limits;
#[cfg(test)]
mod test_game_dir;

/// The parity harnesses' fixtures program: crates/sunrise-parity-fixtures compiles this file as its root.
#[cfg(sunrise_parity_fixtures)]
fn main() {
    game::parity_fixtures::main();
}
