//! The official game: the platform's game (`sunrise-game`) with Sunrise's own in-game parts
//! plugged in through the platform's hooks (docs/layers.md, "How a Sunrise crate plugs in").
//! The release builds ship its executable under the game's name, `sunrise-client`
//! (`game-build.yml`); sunrise-client keeps its own plain executable.
//!
//! Each Sunrise crate that runs in the game gives a `&'static` value implementing
//! `sunrise_platform::hooks::Hooks`, and its settings store and pages if it has any, and
//! [`register`] adds them. Nothing here depends on an engine crate.
#![forbid(unsafe_code)]

use sunrise_platform::hooks::{self, GetTheGame, Hooks};
use sunrise_platform::{packs, pages, settings};

/// Every Sunrise part in the game, in the order its hooks run.
const PARTS: [&dyn Hooks; 5] = [
    // The anonymous "I'm playing" ping.
    &sunrise_heartbeat::HOOKS,
    // Play with Friends: `friend:` addresses join through its tunnel.
    &sunrise_friends::game::HOOKS,
    // Where a game started without its prepared files sends the player.
    &LAUNCHER_PAGE,
    // "Playing Sunrise" on the player's Discord profile.
    &sunrise_discord::HOOKS,
    // Open and Share for saved performance captures.
    &sunrise_perf_share::HOOKS,
];
/// The game started by hand without its prepared files: start it from the launcher, downloaded
/// from Sunrise's website.
static LAUNCHER_PAGE: LauncherPage = LauncherPage;

struct LauncherPage;

impl Hooks for LauncherPage {
    fn get_the_game(&self) -> Option<GetTheGame> {
        Some(GetTheGame {
            address: sunrise_addresses::WEBSITE,
            title: "sunrise.get_the_game.title",
            paragraphs: &[
                "sunrise.get_the_game.about",
                "sunrise.get_the_game.minecraft",
                "sunrise.get_the_game.download",
            ],
            button: "sunrise.get_the_game.button",
        })
    }
}

/// Their settings, in the player's settings files.
const STORES: [&dyn settings::Store; 2] =
    [&sunrise_discord::SETTING, &sunrise_pack_browser::SETTING];
/// Their rows and pages in the options screens.
const PAGES: [&dyn pages::Page; 2] = [&sunrise_discord::SECTION, &sunrise_pack_browser::SECTION];
/// Where the resource pack screen's Add button browses.
const SOURCES: [&dyn packs::Source; 1] = [&sunrise_pack_browser::SOURCE];

/// Registers Sunrise's in-game parts with the game. The executable calls it before it runs the
/// game, and so does the launcher's generated executable for native mods.
pub fn register() {
    let late = PARTS.into_iter().map(hooks::register);
    let late = late.chain(STORES.into_iter().map(settings::register));
    let late = late.chain(SOURCES.into_iter().map(packs::register));
    for result in late.chain(PAGES.into_iter().map(pages::register)) {
        if let Err(e) = result {
            // The executable registered after starting the game: a bug, not a player's problem.
            log::warn!("Sunrise: an in-game part did not register: {e}");
        }
    }
}
