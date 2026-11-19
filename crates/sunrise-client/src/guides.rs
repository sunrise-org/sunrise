//! Built-in mod guides (#2664): every installed mod's guide pages, read from assets and shown in one viewer.
//!
//! Written clean-room from `docs/cleanroom/guides-spec.md` (revision 2); the choices the spec leaves open are in
//! `docs/cleanroom/guides-decisions.md`. This module is the pure part: finding the guides in the resources
//! ([`library`]), front matter ([`front`]), the Markdown subset and its tags ([`markdown`]), search ([`search`]),
//! structure files ([`structure`]), scenes ([`scene`]) and page layout ([`layout`]). The viewer screen that draws
//! them is the game's (`game/interface/guide_viewer.rs`), behind the `sunrise-extras` feature.

pub mod front;
pub mod layout;
pub mod library;
pub mod markdown;
pub mod scene;
pub mod search;
pub mod structure;

/// A problem shown on a page in the error colour (spec 4.13): a translation key of `sunrise.guide.error.*` and
/// the one value it names.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Problem {
    pub key: &'static str,
    pub arg: String,
}

impl Problem {
    pub fn new(key: &'static str, arg: impl Into<String>) -> Self {
        Problem {
            key,
            arg: arg.into(),
        }
    }
    /// The English text, for logs and for a game without the key.
    pub fn english(&self) -> String {
        english(self.key).replacen("%s", &self.arg, 1)
    }
}

/// The English text of each problem key, as `assets/sunrise/lang/en_us.json` has it.
pub const MESSAGES: [(&str, &str); 17] = [
    ("sunrise.guide.error.not_closed", "<%s> is not closed"),
    ("sunrise.guide.error.unknown_tag", "Unknown tag <%s>"),
    ("sunrise.guide.error.missing_page", "Missing page %s"),
    ("sunrise.guide.error.missing_image", "Missing image %s"),
    ("sunrise.guide.error.unknown_item", "Unknown item %s"),
    ("sunrise.guide.error.no_page", "No page for %s"),
    ("sunrise.guide.error.no_recipe", "No recipe for %s"),
    ("sunrise.guide.error.unknown_key", "Unknown key mapping %s"),
    ("sunrise.guide.error.unknown_block", "Unknown block %s"),
    ("sunrise.guide.error.unknown_color", "Unknown colour %s"),
    ("sunrise.guide.error.no_attribute", "Missing attribute %s"),
    (
        "sunrise.guide.error.bad_command",
        "Commands start with /: %s",
    ),
    (
        "sunrise.guide.error.missing_structure",
        "Missing structure %s",
    ),
    (
        "sunrise.guide.error.bad_structure",
        "Unreadable structure %s",
    ),
    (
        "sunrise.guide.error.too_many_blocks",
        "Too many blocks in this scene: %s",
    ),
    (
        "sunrise.guide.error.too_many_entities",
        "Too many entities in this scene: %s",
    ),
    ("sunrise.guide.error.not_scene", "Not a scene element: <%s>"),
];

pub fn english(key: &str) -> &'static str {
    MESSAGES
        .iter()
        .find(|(k, _)| *k == key)
        .map_or("%s", |(_, v)| v)
}

#[cfg(test)]
mod tests;
