//! Shared compact SNBT formatting for world callers.
pub use sunrise_core::nbt::snbt_text::{
    quote_and_escape, string_of, to_snbt, to_snbt_utf16, write_snbt_utf16,
};

#[cfg(test)]
mod tests {
    use super::*;
    use sunrise_core::{
        java_number::{double_to_string, float_to_string},
        nbt::{JavaString, Tag},
    };

    #[test]
    fn scientific_text_selects_the_closest_two_digit_decimal() {
        for (bits, expected) in [
            (1, "1.4E-45"),
            (2, "2.8E-45"),
            (7, "9.8E-45"),
            (71, "9.9E-44"),
            (100, "1.4E-43"),
        ] {
            for (sign, prefix) in [(0, ""), (1 << 31, "-")] {
                let bits = bits | sign;
                assert_eq!(
                    float_to_string(f32::from_bits(bits)),
                    format!("{prefix}{expected}")
                );
                assert_eq!(to_snbt(&Tag::Float(bits)), format!("{prefix}{expected}f"));
            }
        }
        for (bits, expected) in [
            (1, "4.9E-324"),
            (2, "9.9E-324"),
            (12, "5.9E-323"),
            (100, "4.94E-322"),
        ] {
            for (sign, prefix) in [(0, ""), (1 << 63, "-")] {
                let bits = bits | sign;
                assert_eq!(
                    double_to_string(f64::from_bits(bits)),
                    format!("{prefix}{expected}")
                );
                assert_eq!(to_snbt(&Tag::Double(bits)), format!("{prefix}{expected}d"));
            }
        }
    }

    #[test]
    fn utf16_strings_and_utf8_projection_keep_unpaired_units_lossless() {
        let tag = Tag::String(JavaString(vec![
            0, 8, 9, 10, 11, 12, 13, 31, 34, 39, 92, 0xd800, 0xd83d, 0xde00, 0xdc00,
        ]));
        let mut expected: Vec<u16> = "'\\x00\\b\\t\\n\\x0B\\f\\r\\x1F\"\\'\\\\"
            .encode_utf16()
            .collect();
        expected.extend([0xd800, 0xd83d, 0xde00, 0xdc00, 39]);
        assert_eq!(to_snbt_utf16(&tag).0, expected);
        let mut streamed = Vec::new();
        write_snbt_utf16(&tag, |unit| streamed.push(unit));
        assert_eq!(streamed, expected);
        assert_eq!(
            to_snbt(&tag),
            "'\\x00\\b\\t\\n\\x0B\\f\\r\\x1F\"\\'\\\\\\uD800\u{1f600}\\uDC00'"
        );
    }

    #[test]
    fn utf16_key_sorting_and_integer_extremes_match_the_compact_visitor() {
        let tag = Tag::compound([
            (JavaString(vec![0xe000]), Tag::Long(i64::MIN)),
            (JavaString(vec![0xd800, 0xdc00]), Tag::Int(i32::MIN)),
            (JavaString(vec![0xd800]), Tag::Byte(-128)),
        ]);
        let mut expected = vec![123, 34, 0xd800, 34];
        expected.extend(":-128b,".encode_utf16());
        expected.extend([34, 0xd800, 0xdc00, 34]);
        expected.extend(":-2147483648,".encode_utf16());
        expected.extend([34, 0xe000, 34]);
        expected.extend(":-9223372036854775808L}".encode_utf16());
        assert_eq!(to_snbt_utf16(&tag).0, expected);
        assert_eq!(to_snbt(&Tag::Float(0x80000000)), "-0.0f");
        assert_eq!(to_snbt(&Tag::Double(0x8000000000000000)), "-0.0d");
    }
}
