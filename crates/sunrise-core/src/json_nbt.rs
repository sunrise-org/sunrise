//! JSON/NBT conversion, following 26.3 JsonOps and NbtOps.
//!
//! Call at boundaries, not on the tick. NbtAccounter's 512-container limit
//! applies before cloning payloads; scalar and typed-array tags do not add
//! nesting. Conversion is intentionally lossy: JSON has no numeric tag widths,
//! typed arrays become lists, and byte booleans become JSON numbers.
//! When the raw JSON is available, [`parse_json_tag`] preserves the source
//! member order used by NbtOps to build its hash map. An already parsed `Value`
//! only exposes its map's iteration order, not the original collision history.

use crate::nbt::{Compound, CompoundIter, JavaString, Tag, discard_tag};
use indexmap::IndexMap;
use serde_json::{Map, Number, Value, value::RawValue};

const MAX_DEPTH: usize = 512;

/// A value that cannot cross the shared JSON/NBT representation boundary.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JsonNbtError {
    /// JsonOps exposes nested JsonNull as Java null and refuses conversion.
    NestedNull,
    /// More than vanilla NBT's 512 nested list/compound containers.
    TooDeep,
    /// Tag→JSON only: JSON Value cannot hold NaN or infinity. Gson's strict
    /// JSON writer also refuses these numbers. Never substitute null or text.
    NonFinite,
    /// Tag→JSON only: a string or compound key contains an unpaired UTF-16
    /// surrogate. JSON Value uses UTF-8; never replace invalid units lossily.
    InvalidString,
}
impl std::fmt::Display for JsonNbtError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::NestedNull => "JSON-to-NBT does not accept nested null",
            Self::TooDeep => "NBT nesting exceeds 512 containers",
            Self::NonFinite => "JSON cannot represent a non-finite NBT number",
            Self::InvalidString => "JSON cannot represent an unpaired UTF-16 surrogate",
        })
    }
}
impl std::error::Error for JsonNbtError {}

/// A refusal when decoding JSON text into typed NBT.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum JsonTextError {
    /// Invalid JSON or trailing data, with a zero-based byte offset into the input.
    Malformed { cursor: usize },
    /// A number exceeds Gson's decimal digit or scale bounds.
    NumberOutOfRange { cursor: usize },
    /// A well-formed JSON value violates a shared conversion rule.
    Conversion(JsonNbtError),
}
impl std::fmt::Display for JsonTextError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Malformed { cursor } => write!(f, "malformed JSON at byte {cursor}"),
            Self::NumberOutOfRange { cursor } => {
                write!(f, "JSON number out of range at byte {cursor}")
            }
            Self::Conversion(error) => error.fmt(f),
        }
    }
}
impl std::error::Error for JsonTextError {}

/// Parse one complete UTF-8 JSON value, inserting object members in source order
/// at every level. Duplicate keys replace the value in its original position.
/// Only surviving values can cause conversion errors; malformed JSON is always
/// refused. Unicode escapes retain their UTF-16 units. Numbers narrow from the
/// original decimal, booleans become bytes, arrays become lists, and a root null
/// becomes End. Returns [`JsonTextError`] for malformed input, unsupported decimal
/// size, surviving nested nulls, or more than 512 nested list/compound containers.
pub fn parse_json_tag(source: &str) -> Result<Tag, JsonTextError> {
    let mut parser = JsonParser {
        source,
        tail: source,
    };
    let result = parser.value()?;
    if !parser.remaining().is_empty() {
        return Err(parser.malformed());
    }
    result.map(DecodedTag::into_tag)
}

// Dropping a replaced member or a partial parse must not recurse through its NBT.
struct DecodedTag(Tag);
impl DecodedTag {
    fn into_tag(mut self) -> Tag {
        std::mem::replace(&mut self.0, Tag::End)
    }
}
impl Drop for DecodedTag {
    fn drop(&mut self) {
        discard_tag(std::mem::replace(&mut self.0, Tag::End));
    }
}

// Keep a conversion error attached to its value so replacing a duplicate
// object member also replaces its error.
type Converted = Result<DecodedTag, JsonTextError>;

enum JsonFrame {
    List(Vec<Converted>),
    Object(IndexMap<JavaString, Converted>, JavaString),
}
impl JsonFrame {
    fn finish(self) -> Converted {
        match self {
            Self::List(values) => {
                let mut output = DecodedTag(Tag::List(Vec::with_capacity(values.len())));
                let Tag::List(list) = &mut output.0 else {
                    unreachable!()
                };
                for value in values {
                    list.push(value?.into_tag());
                }
                Ok(output)
            }
            Self::Object(entries, _) => {
                let mut output = DecodedTag(Tag::Compound(Compound::new()));
                let Tag::Compound(compound) = &mut output.0 else {
                    unreachable!()
                };
                for (key, value) in entries {
                    compound.insert(key, value?.into_tag());
                }
                Ok(output)
            }
        }
    }
}

struct JsonParser<'a> {
    source: &'a str,
    tail: &'a str,
}
impl JsonParser<'_> {
    fn cursor(&self) -> usize {
        self.source.len() - self.tail.len()
    }
    fn malformed(&self) -> JsonTextError {
        JsonTextError::Malformed {
            cursor: self.cursor(),
        }
    }
    fn remaining(&mut self) -> &str {
        self.tail = self.tail.trim_start_matches([' ', '\t', '\r', '\n']);
        self.tail
    }
    fn eat(&mut self, token: &str) -> bool {
        self.remaining();
        if let Some(tail) = self.tail.strip_prefix(token) {
            self.tail = tail;
            true
        } else {
            false
        }
    }
    fn string(&mut self) -> Result<JavaString, JsonTextError> {
        self.remaining();
        if !self.tail.starts_with('"') {
            return Err(self.malformed());
        }
        // RawValue validates escapes without forcing escaped UTF-16 to become UTF-8.
        let mut tokens = serde_json::Deserializer::from_str(self.tail).into_iter::<&RawValue>();
        let token = tokens
            .next()
            .expect("a token starts here")
            .map_err(|_| self.malformed())?
            .get();
        self.tail = &self.tail[tokens.byte_offset()..];
        let mut contents = &token[1..token.len() - 1];
        let mut units = Vec::new();
        while let Some((literal, escape)) = contents.split_once('\\') {
            units.extend(literal.encode_utf16());
            let (unit, width) = match escape.as_bytes()[0] {
                b'u' => (
                    u16::from_str_radix(&escape[1..5], 16).expect("validated hex escape"),
                    5,
                ),
                b'b' => (8, 1),
                b't' => (9, 1),
                b'n' => (10, 1),
                b'f' => (12, 1),
                b'r' => (13, 1),
                byte => (u16::from(byte), 1),
            };
            units.push(unit);
            contents = &escape[width..];
        }
        units.extend(contents.encode_utf16());
        Ok(JavaString(units))
    }
    fn key(&mut self) -> Result<JavaString, JsonTextError> {
        let key = self.string()?;
        if !self.eat(":") {
            return Err(self.malformed());
        }
        Ok(key)
    }
    fn value(&mut self) -> Result<Converted, JsonTextError> {
        const TOO_DEEP: Converted = Err(JsonTextError::Conversion(JsonNbtError::TooDeep));
        let mut frames = Vec::new();
        'value: loop {
            let mut value = match self.remaining().as_bytes().first().copied() {
                Some(b'[') => {
                    self.tail = &self.tail[1..];
                    if self.eat("]") {
                        if frames.len() >= MAX_DEPTH {
                            TOO_DEEP
                        } else {
                            Ok(DecodedTag(Tag::List(Vec::new())))
                        }
                    } else {
                        frames.push(JsonFrame::List(Vec::new()));
                        continue;
                    }
                }
                Some(b'{') => {
                    self.tail = &self.tail[1..];
                    if self.eat("}") {
                        if frames.len() >= MAX_DEPTH {
                            TOO_DEEP
                        } else {
                            Ok(DecodedTag(Tag::Compound(Compound::new())))
                        }
                    } else {
                        frames.push(JsonFrame::Object(IndexMap::new(), self.key()?));
                        continue;
                    }
                }
                Some(b'"') => Ok(DecodedTag(Tag::String(self.string()?))),
                Some(b't') if self.eat("true") => Ok(DecodedTag(Tag::Byte(1))),
                Some(b'f') if self.eat("false") => Ok(DecodedTag(Tag::Byte(0))),
                Some(b'n') if self.eat("null") => Ok(DecodedTag(Tag::End)),
                Some(b'-' | b'0'..=b'9') => {
                    let len = self
                        .tail
                        .bytes()
                        .take_while(|byte| {
                            matches!(byte, b'0'..=b'9' | b'.' | b'-' | b'+' | b'e' | b'E')
                        })
                        .count();
                    let (decimal, tail) = self.tail.split_at(len);
                    // RawValue checks JSON number syntax without narrowing to f64.
                    serde_json::from_str::<&RawValue>(decimal).map_err(|_| self.malformed())?;
                    // JsonPrimitive.getAsBigDecimal checks Gson's bounds before
                    // JsonOps narrows the number; this is not a JSON syntax error.
                    let result = decimal_parts(decimal)
                        .filter(|(_, scale)| {
                            decimal.len() <= 10_000 && scale.unsigned_abs() < 10_000
                        })
                        .map(|(mantissa, scale)| match exact_long(mantissa, scale) {
                            Some(value) => integer_tag(value),
                            None => {
                                floating_tag(decimal.parse::<f64>().expect("validated decimal"))
                            }
                        })
                        .ok_or_else(|| JsonTextError::NumberOutOfRange {
                            cursor: self.cursor(),
                        });
                    self.tail = tail;
                    result.map(DecodedTag)
                }
                _ => return Err(self.malformed()),
            };
            while let Some(frame) = frames.last_mut() {
                if matches!(value, Ok(DecodedTag(Tag::End))) {
                    value = Err(JsonTextError::Conversion(JsonNbtError::NestedNull));
                }
                let end = match frame {
                    JsonFrame::List(values) => {
                        values.push(value);
                        "]"
                    }
                    JsonFrame::Object(entries, key) => {
                        entries.insert(std::mem::take(key), value);
                        "}"
                    }
                };
                if !self.eat(end) {
                    if !self.eat(",") {
                        return Err(self.malformed());
                    }
                    if let JsonFrame::Object(_, key) = frame {
                        *key = self.key()?;
                    }
                    continue 'value;
                }
                let complete = frames.pop().expect("completed frame");
                value = if frames.len() >= MAX_DEPTH {
                    TOO_DEEP
                } else {
                    complete.finish()
                };
            }
            return Ok(value);
        }
    }
}

/// Convert JSON metadata using JsonOps→NbtOps numeric and nested-null rules.
/// Top-level null becomes End. Numeric choice uses the JSON number's decimal
/// spelling, already rounded by serde_json. UTF-8 input preserves valid UTF-16
/// pairs; lone surrogates cannot reach this function through JSON Value.
pub fn json_to_tag(value: &Value) -> Result<Tag, JsonNbtError> {
    build_tag(JsonInput::Value(value), 0)
}

/// Convert a borrowed object without cloning the JSON tree. The object itself
/// counts toward the same 512-container limit as an object in json_to_tag.
pub fn json_to_compound(values: &Map<String, Value>) -> Result<Compound, JsonNbtError> {
    let Tag::Compound(values) = build_tag(JsonInput::Object(values), 0)? else {
        unreachable!("an object builds a compound")
    };
    Ok(values)
}

/// Convert a projection of borrowed object fields without cloning JSON payloads.
/// `parent_depth` counts surrounding NBT containers in the caller's output
/// schema. It includes synthetic metadata wrappers not present in the input.
pub fn json_fields_to_compound<'a>(
    fields: impl IntoIterator<Item = (&'a str, &'a Value)>,
    parent_depth: usize,
) -> Result<Compound, JsonNbtError> {
    let fields: Vec<_> = fields.into_iter().collect();
    let Tag::Compound(compound) = build_tag(JsonInput::Fields(&fields), parent_depth)? else {
        unreachable!("fields build a compound")
    };
    Ok(compound)
}

#[derive(Clone, Copy)]
enum JsonInput<'a> {
    Value(&'a Value),
    Object(&'a Map<String, Value>),
    Fields(&'a [(&'a str, &'a Value)]),
}
fn validate_json(root: JsonInput<'_>, parent_depth: usize) -> Result<(), JsonNbtError> {
    let mut pending = vec![(root, parent_depth, false)];
    while let Some((input, depth, nested)) = pending.pop() {
        match input {
            JsonInput::Value(Value::Null) if nested => return Err(JsonNbtError::NestedNull),
            JsonInput::Value(Value::Array(values)) => {
                if depth >= MAX_DEPTH {
                    return Err(JsonNbtError::TooDeep);
                }
                pending.extend(
                    values
                        .iter()
                        .rev()
                        .map(|v| (JsonInput::Value(v), depth + 1, true)),
                );
            }
            JsonInput::Value(Value::Object(values)) | JsonInput::Object(values) => {
                if depth >= MAX_DEPTH {
                    return Err(JsonNbtError::TooDeep);
                }
                pending.extend(
                    values
                        .values()
                        .rev()
                        .map(|v| (JsonInput::Value(v), depth + 1, true)),
                );
            }
            JsonInput::Fields(fields) => {
                if depth >= MAX_DEPTH {
                    return Err(JsonNbtError::TooDeep);
                }
                pending.extend(
                    fields
                        .iter()
                        .rev()
                        .map(|(_, v)| (JsonInput::Value(v), depth + 1, true)),
                );
            }
            _ => {}
        }
    }
    Ok(())
}
fn build_tag(root: JsonInput<'_>, parent_depth: usize) -> Result<Tag, JsonNbtError> {
    validate_json(root, parent_depth)?;
    enum Work<'a> {
        Enter(JsonInput<'a>),
        List(usize),
        Object(Vec<JavaString>),
    }
    let mut work = vec![Work::Enter(root)];
    let mut done = Vec::new();
    while let Some(step) = work.pop() {
        match step {
            Work::Enter(input) => match input {
                JsonInput::Value(Value::Null) => done.push(Tag::End),
                JsonInput::Value(Value::Bool(v)) => done.push(Tag::Byte(i8::from(*v))),
                JsonInput::Value(Value::String(v)) => done.push(Tag::String(v.as_str().into())),
                JsonInput::Value(Value::Number(v)) => done.push(number(v)),
                JsonInput::Value(Value::Array(values)) => {
                    work.push(Work::List(values.len()));
                    work.extend(
                        values
                            .iter()
                            .rev()
                            .map(|v| Work::Enter(JsonInput::Value(v))),
                    );
                }
                JsonInput::Value(Value::Object(values)) | JsonInput::Object(values) => {
                    work.push(Work::Object(
                        values.keys().map(|k| k.as_str().into()).collect(),
                    ));
                    work.extend(
                        values
                            .values()
                            .rev()
                            .map(|v| Work::Enter(JsonInput::Value(v))),
                    );
                }
                JsonInput::Fields(fields) => {
                    work.push(Work::Object(
                        fields.iter().map(|(key, _)| (*key).into()).collect(),
                    ));
                    work.extend(
                        fields
                            .iter()
                            .rev()
                            .map(|(_, value)| Work::Enter(JsonInput::Value(value))),
                    );
                }
            },
            Work::List(len) => {
                let values = done.split_off(done.len() - len);
                done.push(Tag::List(values));
            }
            Work::Object(keys) => {
                let values = done.split_off(done.len() - keys.len());
                done.push(Tag::Compound(keys.into_iter().zip(values).collect()));
            }
        }
    }
    Ok(done.pop().expect("one root was built"))
}

fn validate_tag(root: &Tag) -> Result<(), JsonNbtError> {
    let valid_string = |s: &JavaString| {
        if char::decode_utf16(s.0.iter().copied()).any(|c| c.is_err()) {
            Err(JsonNbtError::InvalidString)
        } else {
            Ok(())
        }
    };
    let mut pending = vec![(root, 0)];
    while let Some((tag, depth)) = pending.pop() {
        match tag {
            Tag::Float(bits) if !f32::from_bits(*bits).is_finite() => {
                return Err(JsonNbtError::NonFinite);
            }
            Tag::Double(bits) if !f64::from_bits(*bits).is_finite() => {
                return Err(JsonNbtError::NonFinite);
            }
            Tag::String(s) => valid_string(s)?,
            Tag::List(values) => {
                if depth >= MAX_DEPTH {
                    return Err(JsonNbtError::TooDeep);
                }
                pending.extend(values.iter().rev().map(|v| (v, depth + 1)));
            }
            Tag::Compound(values) => {
                if depth >= MAX_DEPTH {
                    return Err(JsonNbtError::TooDeep);
                }
                for key in values.keys() {
                    valid_string(key)?;
                }
                let start = pending.len();
                pending.extend(values.values().map(|v| (v, depth + 1)));
                pending[start..].reverse();
            }
            _ => {}
        }
    }
    Ok(())
}

/// Convert using NbtOps→JsonOps, preserving its intended loss of numeric widths,
/// boolean identity and typed-array identity. Float numbers use Java's decimal
/// spelling, not the expanded f64 value of a promoted f32.
///
/// NonFinite and InvalidString are explicit representation refusals: serde_json
/// cannot hold those numbers or UTF-16 strings, and Gson's strict writer also
/// refuses non-finite numbers. No replacement with null, strings or U+FFFD.
pub fn tag_to_json(root: &Tag) -> Result<Value, JsonNbtError> {
    JsonTag::new(root).map(JsonTag::to_json)
}

/// Borrowed proof that a tag fits the JSON representation and nesting bound.
/// Construct all proofs before building a larger output to avoid partial JSON
/// temporaries when a later payload is refused.
#[derive(Clone, Copy)]
pub struct JsonTag<'a> {
    root: &'a Tag,
}
impl<'a> JsonTag<'a> {
    /// Check without cloning payloads or constructing JSON.
    pub fn new(root: &'a Tag) -> Result<Self, JsonNbtError> {
        validate_tag(root)?;
        Ok(Self { root })
    }
    /// Construct JSON from the validated, immutably borrowed tag.
    pub fn to_json(self) -> Value {
        enum Work<'a> {
            Enter(&'a Tag),
            List(usize),
            Object(Vec<String>),
        }
        let mut work = vec![Work::Enter(self.root)];
        let mut done = Vec::new();
        while let Some(step) = work.pop() {
            match step {
                Work::Enter(tag) => match tag {
                    Tag::List(values) => {
                        work.push(Work::List(values.len()));
                        work.extend(values.iter().rev().map(Work::Enter));
                    }
                    Tag::Compound(values) => {
                        work.push(Work::Object(
                            values
                                .keys()
                                .map(|k| k.to_string().expect("validated UTF-16"))
                                .collect(),
                        ));
                        work.extend(values.values().map(Work::Enter));
                    }
                    leaf => done.push(json_atom(leaf)),
                },
                Work::List(len) => {
                    let values = done.split_off(done.len() - len);
                    done.push(Value::Array(values));
                }
                Work::Object(keys) => {
                    let values = done.split_off(done.len() - keys.len());
                    done.push(Value::Object(
                        keys.into_iter().zip(values.into_iter().rev()).collect(),
                    ));
                }
            }
        }
        done.pop().expect("one root was built")
    }
}

/// Convert an NBT tag into compact JSON text without a recursive serializer.
/// Refuses the same values as [`tag_to_json`]. Object members follow NBT
/// traversal order, as in `NbtOps.convertTo(JsonOps)`.
pub fn tag_to_json_text(root: &Tag) -> Result<String, JsonNbtError> {
    validate_tag(root)?;
    enum Frame<'a> {
        List(std::slice::Iter<'a, Tag>),
        Object(CompoundIter<'a>),
    }
    let mut output = String::new();
    let mut frames = Vec::new();
    let mut next = Some(root);
    loop {
        if let Some(tag) = next.take() {
            match tag {
                Tag::List(values) => {
                    output.push('[');
                    let mut values = values.iter();
                    if let Some(first) = values.next() {
                        frames.push(Frame::List(values));
                        next = Some(first);
                        continue;
                    }
                    output.push(']');
                }
                Tag::Compound(entries) => {
                    output.push('{');
                    let mut entries = entries.iter();
                    if let Some((key, value)) = entries.next() {
                        write_json_key(&mut output, key);
                        frames.push(Frame::Object(entries));
                        next = Some(value);
                        continue;
                    }
                    output.push('}');
                }
                leaf => {
                    use std::fmt::Write;
                    write!(&mut output, "{}", json_atom(leaf)).expect("writing to a String");
                }
            }
        }
        match frames.last_mut() {
            Some(Frame::List(values)) => {
                if let Some(value) = values.next() {
                    output.push(',');
                    next = Some(value);
                    continue;
                }
                output.push(']');
            }
            Some(Frame::Object(entries)) => {
                if let Some((key, value)) = entries.next() {
                    output.push(',');
                    write_json_key(&mut output, key);
                    next = Some(value);
                    continue;
                }
                output.push('}');
            }
            None => return Ok(output),
        }
        frames.pop();
    }
}

fn write_json_key(output: &mut String, key: &JavaString) {
    use std::fmt::Write;
    let key = Value::String(key.to_string().expect("validated UTF-16"));
    write!(output, "{key}:").expect("writing to a String");
}

fn json_atom(tag: &Tag) -> Value {
    match tag {
        Tag::End => Value::Null,
        Tag::Byte(v) => Value::from(*v),
        Tag::Short(v) => Value::from(*v),
        Tag::Int(v) => Value::from(*v),
        Tag::Long(v) => Value::from(*v),
        Tag::Float(bits) => Value::Number(
            Number::from_f64(
                crate::java_number::float_to_string(f32::from_bits(*bits))
                    .parse::<f64>()
                    .expect("validated finite float"),
            )
            .expect("validated finite float"),
        ),
        // Do not reparse through serde_json: its optional parser
        // features must not affect an already typed double's bits.
        Tag::Double(bits) => {
            Value::Number(Number::from_f64(f64::from_bits(*bits)).expect("validated finite double"))
        }
        Tag::String(s) => Value::String(s.to_string().expect("validated UTF-16")),
        Tag::ByteArray(values) => {
            Value::Array(values.iter().map(|v| Value::from(*v as i8)).collect())
        }
        Tag::IntArray(values) => Value::Array(values.iter().copied().map(Value::from).collect()),
        Tag::LongArray(values) => Value::Array(values.iter().copied().map(Value::from).collect()),
        Tag::List(_) | Tag::Compound(_) => unreachable!("containers are visited by the walker"),
    }
}

fn number(number: &Number) -> Tag {
    // BigDecimal.longValueExact in JsonOps: do not round a fraction or a u64
    // through f64 into an apparently integral signed long.
    if let Some((mantissa, scale)) = decimal_parts(&number.to_string())
        && let Some(value) = exact_long(mantissa, scale)
    {
        return integer_tag(value);
    }
    floating_tag(
        number
            .as_f64()
            .expect("a JSON number is representable as f64"),
    )
}

fn integer_tag(value: i64) -> Tag {
    if let Ok(value) = i8::try_from(value) {
        Tag::Byte(value)
    } else if let Ok(value) = i16::try_from(value) {
        Tag::Short(value)
    } else if let Ok(value) = i32::try_from(value) {
        Tag::Int(value)
    } else {
        Tag::Long(value)
    }
}

fn floating_tag(value: f64) -> Tag {
    let float = value as f32;
    if f64::from(float) == value {
        Tag::Float(if float == 0.0 { 0 } else { float.to_bits() })
    } else {
        Tag::Double(if value == 0.0 { 0 } else { value.to_bits() })
    }
}

/// A validated decimal's mantissa and Java BigDecimal scale.
fn decimal_parts(decimal: &str) -> Option<(&str, i32)> {
    let (mantissa, exponent) = match decimal.split_once(['e', 'E']) {
        Some((mantissa, exponent)) => (mantissa, exponent.parse::<i32>().ok()?),
        None => (decimal, 0),
    };
    let fractional = i32::try_from(mantissa.split_once('.').map_or(0, |(_, f)| f.len())).ok()?;
    Some((mantissa, fractional.checked_sub(exponent)?))
}

/// BigDecimal.longValueExact without intermediate floating-point rounding.
fn exact_long(mantissa: &str, scale: i32) -> Option<i64> {
    let (negative, mantissa) = mantissa
        .strip_prefix('-')
        .map_or((false, mantissa), |v| (true, v));
    let mut digits: Vec<u8> = mantissa.bytes().filter(|b| *b != b'.').collect();
    let mut scale = scale.checked_neg()?;
    while scale < 0 && digits.last() == Some(&b'0') {
        digits.pop();
        scale += 1;
    }
    if digits.iter().all(|b| *b == b'0') {
        return Some(0);
    }
    if !(0..=18).contains(&scale) {
        return None;
    }
    // Accumulate negative so i64::MIN is representable as well.
    let mut value = 0i64;
    for digit in digits {
        value = value
            .checked_mul(10)?
            .checked_sub(i64::from(digit - b'0'))?;
    }
    for _ in 0..scale {
        value = value.checked_mul(10)?;
    }
    if negative {
        Some(value)
    } else {
        value.checked_neg()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn tag(value: &Value) -> Tag {
        json_to_tag(value).unwrap()
    }

    #[test]
    fn json_source_member_history_reaches_nested_nbt_hash_maps() {
        // Gson member order reaches NbtOps.createMap unchanged.
        // These two String keys have the same Java hash.
        for source in [
            r#"{"BB":{"BB":1,"Aa":2},"Aa":3}"#,
            r#"{"BB":null,"Aa":3,"BB":{"BB":1,"Aa":2}}"#,
        ] {
            let Tag::Compound(root) = parse_json_tag(source).unwrap() else {
                panic!("compound")
            };
            assert_eq!(
                root.keys().map(|key| key.0.clone()).collect::<Vec<_>>(),
                [vec![66, 66], vec![65, 97]],
            );
            let Some(Tag::Compound(nested)) = root.get(&"BB".into()) else {
                panic!("compound")
            };
            assert_eq!(
                nested.keys().map(|key| key.0.clone()).collect::<Vec<_>>(),
                [vec![66, 66], vec![65, 97]],
            );
            assert_eq!(nested.get(&"BB".into()), Some(&Tag::Byte(1)));
            assert_eq!(nested.get(&"Aa".into()), Some(&Tag::Byte(2)));
        }
        let source = r#"{"BB":1,"Aa":2}"#;
        let shared: Value = serde_json::from_str(source).unwrap();
        assert_eq!(shared.to_string(), r#"{"Aa":2,"BB":1}"#);
        assert_eq!(parse_json_tag("null").unwrap(), Tag::End);
        assert!(parse_json_tag(r#"{"BB":[null]}"#).is_err());
    }

    #[test]
    fn refuses_nested_null_without_inventing_end_payloads() {
        assert_eq!(json_to_tag(&Value::Null), Ok(Tag::End));
        for value in [json!([null]), json!({"x": null}), json!({"x": [1, null]})] {
            assert_eq!(json_to_tag(&value), Err(JsonNbtError::NestedNull));
        }
    }

    #[test]
    fn numeric_boundaries_do_not_round_through_double() {
        for (source, expected) in [
            ("-128", Tag::Byte(-128)),
            ("127", Tag::Byte(127)),
            ("128", Tag::Short(128)),
            ("-129", Tag::Short(-129)),
            ("32767", Tag::Short(32767)),
            ("32768", Tag::Int(32768)),
            ("-32769", Tag::Int(-32769)),
            ("2147483647", Tag::Int(i32::MAX)),
            ("2147483648", Tag::Long(2147483648)),
            ("-9223372036854775808", Tag::Long(i64::MIN)),
            ("9223372036854775807", Tag::Long(i64::MAX)),
            (
                "9223372036854775808",
                Tag::Float((9223372036854775808.0f32).to_bits()),
            ),
            ("1.0", Tag::Byte(1)),
            ("1e3", Tag::Short(1000)),
            ("1.5", Tag::Float(1.5f32.to_bits())),
            ("0.1", Tag::Double(0.1f64.to_bits())),
            ("-0.0", Tag::Byte(0)),
        ] {
            assert_eq!(
                tag(&serde_json::from_str::<Value>(source).unwrap()),
                expected,
                "{source}"
            );
        }
    }

    #[test]
    fn json_arrays_are_never_nbt_arrays_or_compound_wrappers() {
        assert_eq!(tag(&json!([])), Tag::List(vec![]));
        assert_eq!(
            tag(&json!([1, 2])),
            Tag::List(vec![Tag::Byte(1), Tag::Byte(2)])
        );
        assert_eq!(
            tag(&json!([32768, 2147483648i64])),
            Tag::List(vec![Tag::Int(32768), Tag::Long(2147483648)])
        );
        assert_eq!(
            tag(&json!([true, "x", {}])),
            Tag::List(vec![
                Tag::Byte(1),
                Tag::String("x".into()),
                Tag::compound([] as [(&str, Tag); 0])
            ])
        );
    }

    #[test]
    fn metadata_preserves_boolean_strings_and_nested_list() {
        let tag =
            tag(&json!({"custom": {"enabled": false, "note": "present"}, "lore": ["a", "b"]}));
        assert_eq!(
            tag.get("custom").unwrap().get("enabled"),
            Some(&Tag::Byte(0))
        );
        assert_eq!(
            tag.get("custom").unwrap().get("note"),
            Some(&Tag::String("present".into()))
        );
        assert_eq!(
            tag.get("lore"),
            Some(&Tag::List(vec![
                Tag::String("a".into()),
                Tag::String("b".into())
            ]))
        );
    }
    #[test]
    fn reverse_follows_vanilla_types_and_java_decimal_spelling() {
        for (source, expected) in [
            (Tag::End, "null"),
            (Tag::Byte(1), "1"),
            (Tag::Short(-129), "-129"),
            (Tag::Int(i32::MAX), "2147483647"),
            (Tag::Long(i64::MAX), "9223372036854775807"),
            (Tag::Float(0.1f32.to_bits()), "0.1"),
            (Tag::Float(1), "1.4e-45"),
            (Tag::Double(0.1f64.to_bits()), "0.1"),
            (Tag::Double(1), "4.9e-324"),
            (Tag::ByteArray(vec![0, 127, 128, 255]), "[0,127,-128,-1]"),
            (Tag::IntArray(vec![i32::MIN, 4]), "[-2147483648,4]"),
            (
                Tag::LongArray(vec![i64::MIN, i64::MAX]),
                "[-9223372036854775808,9223372036854775807]",
            ),
        ] {
            assert_eq!(
                tag_to_json(&source).unwrap(),
                serde_json::from_str::<Value>(expected).unwrap()
            );
        }
        assert_eq!(
            tag_to_json(&Tag::Float((-0.0f32).to_bits()))
                .unwrap()
                .to_string(),
            "-0.0"
        );
        let source = Tag::compound([
            (
                "first",
                Tag::List(vec![Tag::Byte(3), Tag::String("s".into())]),
            ),
            ("second", Tag::Compound(Compound::new())),
        ]);
        assert_eq!(
            tag_to_json(&source).unwrap(),
            json!({"first": [3, "s"], "second": {}})
        );
        // NbtOps→JsonOps is not an NBT identity codec.
        for (source, normalized) in [
            (Tag::Long(1), Tag::Byte(1)),
            (Tag::Float(0.1f32.to_bits()), Tag::Double(0.1f64.to_bits())),
            (Tag::ByteArray(vec![255]), Tag::List(vec![Tag::Byte(-1)])),
        ] {
            assert_eq!(
                json_to_tag(&tag_to_json(&source).unwrap()).unwrap(),
                normalized
            );
        }
        assert_eq!(
            tag_to_json(&json_to_tag(&json!(true)).unwrap()).unwrap(),
            json!(1)
        );
    }

    #[test]
    fn reverse_numbers_do_not_depend_on_json_parser_features() {
        for value in [
            123456789012345.67,
            f64::MIN_POSITIVE,
            f64::MAX,
            f64::from_bits(1),
            -0.0,
        ] {
            let json = tag_to_json(&Tag::Double(value.to_bits())).unwrap();
            assert_eq!(json.as_f64().unwrap().to_bits(), value.to_bits());
        }
        for (value, expected) in [
            (0.1f32, 0.1f64),
            (f32::from_bits(1), 1.4e-45f64),
            (f32::MAX, 3.4028235e38f64),
        ] {
            let json = tag_to_json(&Tag::Float(value.to_bits())).unwrap();
            assert_eq!(json.as_f64().unwrap().to_bits(), expected.to_bits());
        }
    }

    #[test]
    fn reverse_refuses_nonfinite_and_unpaired_utf16_without_replacement() {
        for source in [
            Tag::Float(f32::NAN.to_bits()),
            Tag::Double(f64::INFINITY.to_bits()),
            Tag::Float(f32::NEG_INFINITY.to_bits()),
            Tag::Double(0x7ff0000000000001),
            Tag::List(vec![Tag::Double(f64::NEG_INFINITY.to_bits())]),
        ] {
            assert_eq!(tag_to_json(&source), Err(JsonNbtError::NonFinite));
        }
        for source in [
            Tag::String(JavaString(vec![0xd800])),
            Tag::String(JavaString(vec![0xdc00])),
            Tag::Compound([(JavaString(vec![0xdbff]), Tag::Byte(1))].into()),
        ] {
            assert_eq!(tag_to_json(&source), Err(JsonNbtError::InvalidString));
        }
        let pair = JavaString(vec![0xd83d, 0xde00]);
        assert_eq!(
            tag_to_json(&Tag::String(pair.clone())).unwrap(),
            json!("😀")
        );
        assert_eq!(
            tag_to_json(&Tag::Compound([(pair.clone(), Tag::Byte(7))].into())).unwrap(),
            json!({"😀": 7})
        );
        assert_eq!(json_to_tag(&json!("😀")).unwrap(), Tag::String(pair));
    }

    fn discard_json(root: Value) {
        let mut work = vec![root];
        while let Some(value) = work.pop() {
            match value {
                Value::Array(v) => work.extend(v),
                Value::Object(v) => work.extend(v.into_iter().map(|(_, v)| v)),
                _ => {}
            }
        }
    }
    fn discard_tag(root: Tag) {
        let mut work = vec![root];
        while let Some(value) = work.pop() {
            match value {
                Tag::List(v) => work.extend(v),
                Tag::Compound(v) => work.extend(v.into_values()),
                _ => {}
            }
        }
    }
    fn nested_json(depth: usize) -> Value {
        (0..depth).fold(json!(1), |value, n| {
            if n % 2 == 0 {
                Value::Array(vec![value])
            } else {
                Value::Object([("child".into(), value)].into_iter().collect())
            }
        })
    }
    fn nested_tag(depth: usize, leaf: Tag) -> Tag {
        (0..depth).fold(leaf, |value, n| {
            if n % 2 == 0 {
                Tag::List(vec![value])
            } else {
                Tag::compound([("child", value)])
            }
        })
    }

    #[test]
    fn conversion_accepts_512_containers_and_preflights_513_on_256_kib() {
        std::thread::Builder::new()
            .stack_size(256 * 1024)
            .spawn(|| {
                let source = nested_json(512);
                let converted = json_to_tag(&source).unwrap();
                let restored = tag_to_json(&converted).unwrap();
                let mut leaf = &restored;
                for _ in 0..512 {
                    leaf = match leaf {
                        Value::Array(v) => &v[0],
                        Value::Object(v) => &v["child"],
                        _ => panic!("missing container"),
                    };
                }
                assert_eq!(leaf, &json!(1));
                discard_json(source);
                discard_tag(converted);
                discard_json(restored);
                for depth in [513, 4096] {
                    let json = nested_json(depth);
                    assert!(matches!(json_to_tag(&json), Err(JsonNbtError::TooDeep)));
                    discard_json(json);
                    let tag = nested_tag(depth, Tag::Byte(1));
                    assert!(matches!(tag_to_json(&tag), Err(JsonNbtError::TooDeep)));
                    discard_tag(tag);
                }
                // json_to_compound must count its already borrowed root object.
                for (children, accepted) in [(511, true), (512, false)] {
                    let map = [("child".into(), nested_json(children))]
                        .into_iter()
                        .collect();
                    let result = json_to_compound(&map);
                    if accepted {
                        discard_tag(Tag::Compound(result.unwrap()));
                    } else {
                        assert!(matches!(result, Err(JsonNbtError::TooDeep)));
                    }
                    discard_json(Value::Object(map));
                }
                // Primitive arrays have no recursive NBT containers of their own.
                let tag = nested_tag(512, Tag::ByteArray(vec![255]));
                let json = tag_to_json(&tag).unwrap();
                discard_tag(tag);
                discard_json(json);
            })
            .unwrap()
            .join()
            .unwrap();
    }
    #[test]
    fn borrowed_projection_counts_wrappers_and_skips_unselected_data() {
        let map: Map<_, _> = [
            ("selected".into(), nested_json(511)),
            ("ignored".into(), nested_json(4096)),
        ]
        .into_iter()
        .collect();
        let selected = || {
            map.iter()
                .filter(|(k, _)| k.as_str() == "selected")
                .map(|(k, v)| (k.as_str(), v))
        };
        let compound = json_fields_to_compound(selected(), 0).unwrap();
        discard_tag(Tag::Compound(compound));
        assert!(matches!(
            json_fields_to_compound(selected(), 1),
            Err(JsonNbtError::TooDeep)
        ));
        discard_json(Value::Object(map));
    }
}
