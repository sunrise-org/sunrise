//! Text codecs for [`Tag`] and [`Compound`]. They need no Host or world and
//! work on both logical sides. Convert at load, save or command boundaries.

pub use crate::{Compound, JavaString, Tag};
pub use sunrise_core::json_nbt::{JsonNbtError, JsonTextError, parse_json_tag as from_json};
pub use sunrise_core::nbt::{SnbtError, parse_compound_fully, parse_fully};

/// Format compact SNBT using vanilla's `StringTagVisitor`. Compound keys sort
/// by UTF-16 units. Keys and strings retain unpaired UTF-16 units.
///
/// This is not a binary round trip: for example, `END` is read as a string,
/// and SNBT does not preserve the payload bits of a NaN.
pub fn to_snbt(tag: &Tag) -> JavaString {
    sunrise_core::nbt::snbt_text::to_snbt_utf16(tag)
}

/// Convert a tag to compact JSON using `NbtOps.convertTo(JsonOps)` value rules.
/// JSON has no tag widths or typed-array identity. Byte booleans become numbers
/// and `End` becomes null. This does not promise a round trip through [`from_json`].
///
/// Returns [`JsonNbtError`] for non-finite numbers, unpaired UTF-16 in strings
/// or keys, or more than 512 nested list/compound containers. Use [`to_snbt`]
/// to retain unpaired units.
pub fn to_json(tag: &Tag) -> Result<String, JsonNbtError> {
    sunrise_core::json_nbt::tag_to_json_text(tag)
}
