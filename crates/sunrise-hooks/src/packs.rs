//! Where the resource pack screen's online browser gets its packs (docs/layers.md, "Platform
//! registries"): a crate registers a [`Source`], and the engine draws the browse screen over it.
//! The engine knows no pack site: with no enabled source the pack screen has no Add button.
use crate::TooLate;
use crate::registry::Registry;
use std::path::Path;

/// One search hit, as much of it as the list draws.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hit {
    pub id: String,
    pub title: String,
    pub author: String,
    pub description: String,
    pub downloads: u64,
    /// The site's icon for the pack, empty when it has none.
    pub icon_url: String,
}

/// What a [`Source`]'s workers send back, read with [`Source::poll`].
#[derive(Debug)]
#[non_exhaustive]
pub enum Message {
    /// A search finished; these replace the list.
    Results(Vec<Hit>),
    /// A hit's icon (by [`Hit::id`]), as PNG bytes.
    Icon(String, Vec<u8>),
    /// A pack landed in the packs folder under this file name.
    Downloaded(String),
    /// A technical reason in English, shown inside `sunrise.pack.addFailed`.
    Failed(String),
}

/// The source's own text on the browse screen, as translation keys.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Labels {
    /// The Add button's tooltip.
    pub add: &'static str,
    /// The search box's hint.
    pub search_hint: &'static str,
    /// Shown while a search runs.
    pub working: &'static str,
}

/// A place to search and download resource packs from. Its work runs on its own threads and
/// reports through [`Source::poll`]; every method returns quickly. Per-player state is indexed by
/// [`crate::seat`].
pub trait Source: Sync {
    /// Whether the Add button shows. The first enabled registered source wins.
    fn enabled(&self) -> bool;
    /// The translation keys of the filters on offer; `search` gets indices into this list.
    fn filters(&self) -> &'static [&'static str];
    /// Its text on the browse screen.
    fn labels(&self) -> Labels;
    /// The player opened the browse screen: start this seat's session afresh, dropping any left over.
    fn open(&self);
    /// Starts a search for `query` with the `chosen` filters on.
    fn search(&self, query: &str, chosen: &[usize]);
    /// Starts fetching the icons of the last results.
    fn fetch_icons(&self);
    /// Starts downloading result `hit` of the last results into `dir`, for `game_version`.
    fn download(&self, hit: usize, dir: &Path, game_version: &str);
    /// The next finished piece of work, if any.
    fn poll(&self) -> Option<Message>;
    /// The browse screen opened on `seat` is gone: drop what that seat holds. It may come while
    /// another seat's turn runs, when that player's game is dropped.
    fn close(&self, seat: usize);
}

static SOURCES: Registry<dyn Source> = Registry::new();

/// Adds `source` to those the pack screen may browse. Before the game starts, as
/// [`crate::register`].
pub fn register(source: &'static dyn Source) -> Result<(), TooLate> {
    SOURCES.register(source)
}

/// The engine's side. Only the engine calls it.
pub mod engine {
    use super::*;

    /// Every registered source, sealing the list on the first call.
    pub fn all() -> &'static [&'static dyn Source] {
        SOURCES.all()
    }

    /// The first enabled registered source; `None` when there is none (the pack screen then has
    /// no Add button).
    pub fn source() -> Option<&'static dyn Source> {
        all().iter().copied().find(|s| s.enabled())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Off;
    struct On;

    macro_rules! source {
        ($t:ty, $on:expr) => {
            impl Source for $t {
                fn enabled(&self) -> bool {
                    $on
                }
                fn filters(&self) -> &'static [&'static str] {
                    &[]
                }
                fn labels(&self) -> Labels {
                    Labels {
                        add: "a",
                        search_hint: "s",
                        working: "w",
                    }
                }
                fn open(&self) {}
                fn search(&self, _: &str, _: &[usize]) {}
                fn fetch_icons(&self) {}
                fn download(&self, _: usize, _: &Path, _: &str) {}
                fn poll(&self) -> Option<Message> {
                    None
                }
                fn close(&self, _: usize) {}
            }
        };
    }
    source!(Off, false);
    source!(On, true);

    #[test]
    fn the_first_enabled_registered_source_wins_and_late_ones_are_refused() {
        static OFF: Off = Off;
        static ON: On = On;
        register(&OFF).unwrap();
        register(&ON).unwrap();
        let won = engine::source().expect("an enabled source");
        assert!(std::ptr::addr_eq(
            won as *const dyn Source,
            &ON as *const On
        ));
        assert!(register(&ON).is_err());
    }
}
