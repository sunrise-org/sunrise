//! Gameplay and startup, above the reusable client library.
// No `#![recursion_limit]` or other inner attribute horizon-mixins doesn't know here: the
// launcher's horizon-mixins then rejects every native mod targeting this crate (#8169).
#![forbid(unsafe_code)]

#[cfg(test)]
#[used]
static KEEP_TEST_PLATFORM: fn() = mod_platform::__private::test_platform::keep;
extern crate self as sunrise_client;
#[cfg(feature = "client")]
pub use sunrise_client_lib::carrier_store;
#[cfg(feature = "client")]
pub use sunrise_client_lib::*;
// The dispatcher builds the shell's test helpers without the libtest functions that use them.
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

// The unit-test crate uses the allocator from its self dev-dependency.
#[cfg(all(not(test), not(feature = "system-allocator")))]
#[global_allocator]
static ALLOCATOR: sunrise_threads::GameAllocator = sunrise_threads::GAME_ALLOCATOR;
#[cfg_attr(
    sunrise_parity_fixtures,
    allow(dead_code, unused_imports, unused_macros)
)]
mod cut_layout;
pub mod game_facade;
pub use cut_layout::game;
#[cfg(all(test, not(feature = "client")))]
pub(crate) use sunrise_client_lib::settings;
#[cfg(all(test, not(feature = "client")))]
#[cfg_attr(sunrise_parity_fixtures, allow(unused_imports))]
pub(crate) use sunrise_game_data::scoreboard;
#[cfg(all(test, not(feature = "client")))]
#[cfg_attr(sunrise_parity_fixtures, allow(unused_imports))]
pub(crate) use sunrise_items::items;
#[cfg(all(test, not(feature = "client")))]
#[cfg_attr(sunrise_parity_fixtures, allow(unused_imports))]
pub(crate) use sunrise_level::chunk;
#[cfg(all(test, not(feature = "client")))]
#[cfg_attr(sunrise_parity_fixtures, allow(unused_imports))]
pub(crate) use sunrise_sim::{datapack_net, entities, motion, raw_payloads};
pub use sunrise_sim::{lan, orb_limits, save};
#[cfg(test)]
#[cfg_attr(sunrise_parity_fixtures, allow(dead_code))]
mod test_game_dir;

pub use sunrise_level::{crash, logging, spans};

#[cfg(sunrise_parity_fixtures)]
#[path = "../../sunrise-gameplay-common/src/game/parity_fixtures.rs"]
mod parity_fixtures_cli;

#[cfg(sunrise_parity_fixtures)]
fn main() {
    mod_platform::__private::test_platform::install();
    parity_fixtures_cli::main();
}
