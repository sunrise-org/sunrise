use super::{DATA_COMPONENT_TYPES, Result};
use crate::{
    json::error as err,
    math::{Mat4, normalize_quaternion, quaternion_from_axis_angle},
};
use serde_json::{Map, Value};

// ---------------------------------------------------------------------------
// Decoding helpers (DataFixerUpper `JsonOps` semantics)
// ---------------------------------------------------------------------------

pub(super) fn object(v: &Value) -> Result<&Map<String, Value>> {
    v.as_object().ok_or_else(|| err("expected object"))
}
/// A present, non-null field (`JsonOps` maps `null` to an absent key).
pub(super) fn get<'a>(m: &'a Map<String, Value>, key: &str) -> Option<&'a Value> {
    m.get(key).filter(|v| !v.is_null())
}
pub(super) fn required<'a>(m: &'a Map<String, Value>, key: &str) -> Result<&'a Value> {
    get(m, key).ok_or_else(|| err(&format!("missing {key}")))
}
/// `Codec.FLOAT`: numbers, and booleans as 1 and 0.
pub(super) fn float(v: &Value) -> Result<f32> {
    match v {
        Value::Number(n) => n
            .to_string()
            .parse()
            .or_else(|_| n.as_f64().map(|f| f as f32).ok_or(()))
            .map_err(|()| err("invalid float")),
        _ => Err(err("expected float")),
    }
}
/// `Codec.INT`: `Number.intValue`, truncating toward zero and saturating
/// like a Java narrowing of a double (longs keep their low 32 bits).
pub(super) fn int(v: &Value) -> Result<i32> {
    match v {
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Ok(i as i32)
            } else if let Some(u) = n.as_u64() {
                Ok(u as i32)
            } else {
                Ok(n.as_f64().ok_or_else(|| err("invalid int"))? as i32)
            }
        }
        _ => Err(err("expected int")),
    }
}
pub(super) fn boolean(v: &Value) -> Result<bool> {
    v.as_bool().ok_or_else(|| err("expected boolean"))
}
pub(super) fn string(v: &Value) -> Result<String> {
    v.as_str()
        .map(str::to_owned)
        .ok_or_else(|| err("expected string"))
}
pub(super) fn list(v: &Value) -> Result<&Vec<Value>> {
    v.as_array().ok_or_else(|| err("expected list"))
}
/// `Identifier.CODEC`: a lowercase `namespace:path`, defaulting the namespace.
pub fn identifier(name: &str) -> Result<String> {
    let (namespace, path) = match name.split_once(':') {
        Some((n, p)) => (if n.is_empty() { "minecraft" } else { n }, p),
        None => ("minecraft", name),
    };
    let valid = |c: u8| c.is_ascii_lowercase() || c.is_ascii_digit() || b"_.-".contains(&c);
    if path.contains(':')
        || !namespace.bytes().all(valid)
        || !path.bytes().all(|c| valid(c) || c == b'/')
    {
        return Err(err("invalid resource identifier"));
    }
    Ok(format!("{namespace}:{path}"))
}
pub(super) fn ident(v: &Value) -> Result<String> {
    identifier(&string(v)?)
}
pub(super) fn component_type(v: &Value) -> Result<String> {
    let id = ident(v)?;
    if DATA_COMPONENT_TYPES.contains(&id.as_str()) {
        Ok(id)
    } else {
        Err(err("unknown data component type"))
    }
}
fn vector<const N: usize>(v: &Value) -> Result<[f32; N]> {
    let values = list(v)?;
    if values.len() != N {
        return Err(err("wrong vector length"));
    }
    let mut out = [0.0; N];
    for (o, v) in out.iter_mut().zip(values) {
        *o = float(v)?;
    }
    Ok(out)
}
pub(super) fn non_negative(v: &Value) -> Result<i32> {
    let i = int(v)?;
    if i < 0 {
        return Err(err("value must be non-negative"));
    }
    Ok(i)
}
/// `ExtraCodecs.RGB_COLOR_CODEC`: an int, or three floats in 0..1.
pub(super) fn rgb(v: &Value) -> Result<i32> {
    if v.is_array() {
        let [r, g, b] = vector::<3>(v)?;
        return Ok(argb(255, channel(r), channel(g), channel(b)));
    }
    int(v)
}
pub(super) fn argb(a: i32, r: i32, g: i32, b: i32) -> i32 {
    (a & 255) << 24 | (r & 255) << 16 | (g & 255) << 8 | (b & 255)
}
pub(super) fn opaque(color: i32) -> i32 {
    color | (0xFF00_0000u32 as i32)
}

/// `Transformation.EXTENDED_CODEC`.
pub(super) fn transformation(v: &Value) -> Result<Mat4> {
    if v.is_array() {
        return Ok(Mat4::from_row_major(vector::<16>(v)?));
    }
    let m = object(v)?;
    let quaternion = |v: &Value| -> Result<[f32; 4]> {
        if v.is_array() {
            return Ok(normalize_quaternion(vector::<4>(v)?));
        }
        let o = object(v)?;
        Ok(quaternion_from_axis_angle(
            float(required(o, "angle")?)?,
            vector::<3>(required(o, "axis")?)?,
        ))
    };
    let translation = vector::<3>(required(m, "translation")?)?;
    let left = quaternion(required(m, "left_rotation")?)?;
    let scale = vector::<3>(required(m, "scale")?)?;
    let right = quaternion(required(m, "right_rotation")?)?;
    Ok(Mat4::translation(translation)
        .rotate(left)
        .scale(scale)
        .rotate(right))
}
pub(super) fn optional_transformation(m: &Map<String, Value>) -> Result<Option<Mat4>> {
    get(m, "transformation").map(transformation).transpose()
}
/// `Mth.floor(float)` as `ARGB.as8BitChannel` uses it.
/// `ARGB.as8BitChannel`: `Mth.floor(value * 255.0F)`.
fn channel(v: f32) -> i32 {
    let scaled = v * 255.0;
    let i = scaled as i32;
    if (i as f32) > scaled { i - 1 } else { i }
}
