//! `StringTagVisitor` below the text-component layer. The shared visitor emits
//! exact Java UTF-16; UTF-8 callers explicitly escape unpaired surrogate units.
use crate::{
    java_number::{double_to_string, float_to_string},
    nbt::{JavaString, Tag},
};

const HEX: &[u8; 16] = b"0123456789ABCDEF";

/// `StringTag.quoteAndEscape`, including SnbtGrammar's uppercase hex controls.
fn quoted_units(units: impl Iterator<Item = u16> + Clone, sink: &mut impl FnMut(u16)) {
    // A streaming sink cannot patch the opening delimiter afterward.
    let quote = match units.clone().find(|unit| matches!(*unit, 34 | 39)) {
        Some(34) => 39,
        _ => 34,
    };
    sink(quote);
    for unit in units {
        if unit == 92 || unit == quote {
            sink(92);
            sink(unit);
        } else if unit < 32 {
            sink(92);
            match unit {
                8 => sink(98),
                9 => sink(116),
                10 => sink(110),
                12 => sink(102),
                13 => sink(114),
                _ => {
                    sink(120);
                    sink(u16::from(HEX[usize::from(unit >> 4)]));
                    sink(u16::from(HEX[usize::from(unit & 15)]));
                }
            }
        } else {
            sink(unit);
        }
    }
    sink(quote);
}

#[derive(Default)]
struct Utf8Output {
    value: String,
    high: Option<u16>,
}
impl Utf8Output {
    fn escaped_surrogate(&mut self, unit: u16) {
        self.value.push_str("\\u");
        for shift in [12, 8, 4, 0] {
            self.value
                .push(char::from(HEX[usize::from((unit >> shift) & 15)]));
        }
    }
    fn unit(&mut self, unit: u16) {
        if let Some(high) = self.high.take() {
            if (0xdc00..=0xdfff).contains(&unit) {
                let scalar = 0x10000 + (u32::from(high - 0xd800) << 10) + u32::from(unit - 0xdc00);
                self.value
                    .push(char::from_u32(scalar).expect("surrogate pair"));
                return;
            }
            self.escaped_surrogate(high);
        }
        if (0xd800..=0xdbff).contains(&unit) {
            self.high = Some(unit);
        } else if (0xdc00..=0xdfff).contains(&unit) {
            self.escaped_surrogate(unit);
        } else {
            self.value
                .push(char::from_u32(u32::from(unit)).expect("non-surrogate UTF-16"));
        }
    }
    fn finish(mut self) -> String {
        if let Some(high) = self.high.take() {
            self.escaped_surrogate(high);
        }
        self.value
    }
}

/// `StringTag.quoteAndEscape` for a representable UTF-8 input string.
pub fn quote_and_escape(input: &str) -> String {
    let mut out = Utf8Output::default();
    quoted_units(input.encode_utf16(), &mut |unit| out.unit(unit));
    out.finish()
}

fn unquoted_key(key: &[u16]) -> bool {
    let ascii_lower = |unit: u16| {
        if (65..=90).contains(&unit) {
            unit + 32
        } else {
            unit
        }
    };
    for reserved in [b"true".as_slice(), b"false".as_slice()] {
        if key.len() == reserved.len()
            && key
                .iter()
                .zip(reserved)
                .all(|(&unit, &byte)| ascii_lower(unit) == u16::from(byte))
        {
            return false;
        }
    }
    let letter = |unit| (65..=90).contains(&unit) || (97..=122).contains(&unit);
    key.first()
        .is_some_and(|&unit| letter(unit) || matches!(unit, 46 | 95))
        && key.iter().all(|&unit| {
            letter(unit) || (48..=57).contains(&unit) || matches!(unit, 46 | 95 | 43 | 45)
        })
}

/// UI-only UTF-8 projection; lossless serialization uses the UTF-16 visitor.
pub fn string_of(value: &JavaString) -> String {
    value.to_string_lossy()
}

/// UTF-8 compact SNBT. Unpaired units become explicit `\uXXXX` SNBT escapes:
/// the NBT meaning is lossless, but exact Java String identity requires UTF-16.
/// The adapter streams directly into String without an intermediate UTF-16 buffer.
pub fn to_snbt(tag: &Tag) -> String {
    let mut out = Utf8Output::default();
    write_snbt_utf16(tag, |unit| out.unit(unit));
    out.finish()
}

/// Owning exact `Tag.toString` result, including raw unpaired UTF-16 units.
pub fn to_snbt_utf16(tag: &Tag) -> JavaString {
    let mut out = JavaString::default();
    write_snbt_utf16(tag, |unit| out.0.push(unit));
    out
}

/// `StringTagVisitor`: exact Java UTF-16 units, without an owning output buffer.
/// Compounds sort borrowed UTF-16 keys explicitly; storage traversal is not sorted.
pub fn write_snbt_utf16(tag: &Tag, mut sink: impl FnMut(u16)) {
    enum Frame<'a> {
        List(std::slice::Iter<'a, Tag>),
        Compound(std::vec::IntoIter<(&'a JavaString, &'a Tag)>),
        SingleEntry,
    }
    let mut frames = smallvec::SmallVec::<[Frame<'_>; 8]>::new();
    let mut next = Some(tag);
    loop {
        if let Some(tag) = next.take() {
            match tag {
                Tag::List(values) => {
                    sink(91);
                    let mut values = values.iter();
                    if let Some(first) = values.next() {
                        frames.push(Frame::List(values));
                        next = Some(first);
                        continue;
                    }
                    sink(93);
                }
                Tag::Compound(values) => {
                    sink(123);
                    match values.len() {
                        0 => sink(125),
                        1 => {
                            let (key, value) = values.iter().next().expect("one entry");
                            compound_key(key, &mut sink);
                            frames.push(Frame::SingleEntry);
                            next = Some(value);
                            continue;
                        }
                        _ => {
                            let mut entries: Vec<_> = values.iter().collect();
                            entries.sort_unstable_by_key(|(key, _)| *key);
                            let mut entries = entries.into_iter();
                            let (key, value) = entries.next().expect("multiple entries");
                            compound_key(key, &mut sink);
                            frames.push(Frame::Compound(entries));
                            next = Some(value);
                            continue;
                        }
                    }
                }
                leaf => visit_leaf(leaf, &mut sink),
            }
        }
        match frames.last_mut() {
            Some(Frame::List(values)) => {
                if let Some(value) = values.next() {
                    sink(44);
                    next = Some(value);
                    continue;
                }
                sink(93);
            }
            Some(Frame::Compound(entries)) => {
                if let Some((key, value)) = entries.next() {
                    sink(44);
                    compound_key(key, &mut sink);
                    next = Some(value);
                    continue;
                }
                sink(125);
            }
            Some(Frame::SingleEntry) => sink(125),
            None => break,
        }
        frames.pop();
    }
}

fn ascii(value: &str, sink: &mut impl FnMut(u16)) {
    for byte in value.bytes() {
        sink(u16::from(byte));
    }
}

fn integer(value: i64, sink: &mut impl FnMut(u16)) {
    let mut digits = [0u16; 20];
    let mut start = digits.len();
    let mut magnitude = value.unsigned_abs();
    loop {
        start -= 1;
        digits[start] = 48 + (magnitude % 10) as u16;
        magnitude /= 10;
        if magnitude == 0 {
            break;
        }
    }
    if value < 0 {
        start -= 1;
        digits[start] = 45;
    }
    for &unit in &digits[start..] {
        sink(unit);
    }
}

fn compound_key(key: &JavaString, sink: &mut impl FnMut(u16)) {
    if unquoted_key(&key.0) {
        for &unit in &key.0 {
            sink(unit);
        }
    } else {
        quoted_units(key.0.iter().copied(), sink);
    }
    sink(58);
}

fn visit_leaf(tag: &Tag, sink: &mut impl FnMut(u16)) {
    match tag {
        Tag::End => ascii("END", sink),
        Tag::Byte(value) => {
            integer(i64::from(*value), sink);
            sink(98);
        }
        Tag::Short(value) => {
            integer(i64::from(*value), sink);
            sink(115);
        }
        Tag::Int(value) => integer(i64::from(*value), sink),
        Tag::Long(value) => {
            integer(*value, sink);
            sink(76);
        }
        Tag::Float(bits) => {
            ascii(&float_to_string(f32::from_bits(*bits)), sink);
            sink(102);
        }
        Tag::Double(bits) => {
            ascii(&double_to_string(f64::from_bits(*bits)), sink);
            sink(100);
        }
        Tag::String(value) => quoted_units(value.0.iter().copied(), sink),
        Tag::ByteArray(values) => {
            ascii("[B;", sink);
            for (i, value) in values.iter().enumerate() {
                if i != 0 {
                    sink(44);
                }
                integer(i64::from(*value as i8), sink);
                sink(66);
            }
            sink(93);
        }
        Tag::IntArray(values) => {
            ascii("[I;", sink);
            for (i, value) in values.iter().enumerate() {
                if i != 0 {
                    sink(44);
                }
                integer(i64::from(*value), sink);
            }
            sink(93);
        }
        Tag::LongArray(values) => {
            ascii("[L;", sink);
            for (i, value) in values.iter().enumerate() {
                if i != 0 {
                    sink(44);
                }
                integer(*value, sink);
                sink(76);
            }
            sink(93);
        }
        Tag::List(_) | Tag::Compound(_) => unreachable!("containers are visited by the walker"),
    }
}
