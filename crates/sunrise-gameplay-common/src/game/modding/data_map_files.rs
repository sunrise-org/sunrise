//! Decode a whole data-map file before applying any replace, values or removals.
use serde::de::DeserializeOwned;
use serde_json::{Value, value::RawValue};
use std::collections::{BTreeMap, BTreeSet};
use sunrise_core::identifier::Identifier;

type RawObject = BTreeMap<String, Box<RawValue>>;

enum Key {
    Entry(String),
    Tag(String),
}

/// A resource location decoded at the file/value boundary.
pub(super) fn resource_id(raw: &str) -> Result<String, String> {
    Identifier::parse(raw)
        .map(|id| id.to_string())
        .map_err(|error| format!("invalid resource location {raw:?}: {error}"))
}

/// The folder a registry's data maps or tags are under: its path for a
/// `minecraft` registry, `<namespace>/<path>` for another.
pub(super) fn folder(registry: &str) -> String {
    match registry.split_once(':') {
        Some(("minecraft", path)) => path.to_owned(),
        Some((namespace, path)) => format!("{namespace}/{path}"),
        None => registry.to_owned(),
    }
}

impl Key {
    fn decode(raw: &str) -> Result<Self, String> {
        match raw.strip_prefix('#') {
            Some(tag) => resource_id(tag).map(Self::Tag),
            None => resource_id(raw).map(Self::Entry),
        }
    }
    fn visit<I: IntoIterator<Item: AsRef<str>>>(
        &self,
        tags: &impl Fn(&str) -> I,
        mut visit: impl FnMut(&str),
    ) {
        match self {
            Self::Entry(id) => visit(id),
            Self::Tag(tag) => {
                for id in tags(tag) {
                    visit(id.as_ref());
                }
            }
        }
    }
}

/// A decoded removal entry: full removal or partial removal with a validated remover payload.
enum Removal<R> {
    Full(Key),
    Partial(Key, R),
}

/// A validated file: callers cannot apply a half-decoded file.
struct File<T, R = ()> {
    replace: bool,
    values: Vec<(Key, bool, T)>,
    remove: Vec<Removal<R>>,
}

impl<T: Clone, R> File<T, R> {
    fn filter_disallowed_tags(&mut self, id: &Identifier, registry: &str, context: &str) {
        let disallowed: Vec<String> = self
            .values
            .iter()
            .filter_map(|(key, _, _)| match key {
                Key::Tag(tag) => Some(format!("#{tag}")),
                Key::Entry(_) => None,
            })
            .collect();
        if !disallowed.is_empty() {
            log::error!(
                "Sunrise: data map {id} on {registry}: {context}: data map does not support attaching values via tags; ignoring tag entries: {}",
                disallowed.join(", ")
            );
            self.values
                .retain(|(key, _, _)| matches!(key, Key::Entry(_)));
        }
    }

    fn apply_with_hooks<I: IntoIterator<Item: AsRef<str>>>(
        self,
        map: &mut BTreeMap<String, T>,
        tags: &impl Fn(&str) -> I,
        known: impl Fn(&str) -> bool,
        merge: &impl Fn(&str, &T, &T) -> Result<T, String>,
        remove_partial: &impl Fn(&str, &T, &R) -> Result<Option<T>, String>,
        context: &str,
    ) {
        if self.replace {
            map.clear();
        }
        let mut unknown = BTreeSet::new();
        for (key, entry_replace, value) in self.values {
            key.visit(tags, |id| {
                if known(id) {
                    if entry_replace || !map.contains_key(id) {
                        map.insert(id.to_owned(), value.clone());
                    } else if let Some(existing) = map.get(id) {
                        match merge(id, existing, &value) {
                            Ok(merged) => {
                                map.insert(id.to_owned(), merged);
                            }
                            Err(error) => log::warn!(
                                "Sunrise: {context}: merge failed for {id}: {error}; keeping existing value"
                            ),
                        }
                    }
                } else {
                    unknown.insert(id.to_owned());
                }
            });
        }
        for removal in self.remove {
            match removal {
                Removal::Full(key) => {
                    key.visit(tags, |id| {
                        map.remove(id);
                    });
                }
                Removal::Partial(key, remover) => {
                    key.visit(tags, |id| {
                        if let Some(existing) = map.get(id) {
                            match remove_partial(id, existing, &remover) {
                                Ok(None) => {
                                    map.remove(id);
                                }
                                Ok(Some(reduced)) => {
                                    map.insert(id.to_owned(), reduced);
                                }
                                Err(error) => log::error!(
                                    "Sunrise: {context}: partial removal failed for {id}: {error}; keeping existing value"
                                ),
                            }
                        }
                    });
                }
            }
        }
        if !unknown.is_empty() {
            log::warn!(
                "Sunrise: {context}: unknown targets skipped: {}",
                unknown.into_iter().collect::<Vec<_>>().join(", ")
            );
        }
    }

    fn apply<I: IntoIterator<Item: AsRef<str>>>(
        self,
        map: &mut BTreeMap<String, T>,
        tags: &impl Fn(&str) -> I,
        known: impl Fn(&str) -> bool,
        context: &str,
    ) {
        self.apply_with_hooks(
            map,
            tags,
            known,
            &|_, _, incoming| Ok(incoming.clone()),
            &|_, _, _| Err("data map does not support partial removers".into()),
            context,
        );
    }
}

pub fn entry_value(raw: &Value) -> Result<&Value, String> {
    if let Some(object) = raw.as_object()
        && let Some(value) = object.get("value")
    {
        if let Some(replace) = object.get("replace")
            && !replace.is_boolean()
        {
            return Err("entry replace must be a boolean".into());
        }
        return Ok(value);
    }
    Ok(raw)
}

fn read_field<T: DeserializeOwned>(raw: &mut RawObject, field: &str) -> Result<Option<T>, String> {
    raw.remove(field)
        .map(|value| serde_json::from_str(value.get()).map_err(|error| format!("{field}: {error}")))
        .transpose()
}

fn decode_removals<R>(
    raw: Option<Value>,
    decode_remover: impl Fn(&Value) -> Result<R, String>,
) -> Result<Vec<Removal<R>>, String> {
    let Some(raw) = raw else {
        return Ok(Vec::new());
    };
    match raw {
        Value::Array(items) => items
            .iter()
            .map(|removal| {
                if let Some(key) = removal.as_str() {
                    Ok(Removal::Full(Key::decode(key)?))
                } else if let Some(object) = removal.as_object() {
                    let key = object
                        .get("key")
                        .and_then(Value::as_str)
                        .ok_or("data map removal must name a key")?;
                    let key = Key::decode(key)?;
                    if let Some(remover) = object.get("remover") {
                        Ok(Removal::Partial(key, decode_remover(remover)?))
                    } else {
                        Ok(Removal::Full(key))
                    }
                } else {
                    Err("data map removal must name a key".into())
                }
            })
            .collect(),
        Value::Object(entries) => entries
            .iter()
            .map(|(key, remover)| {
                Ok(Removal::Partial(
                    Key::decode(key)?,
                    decode_remover(remover)?,
                ))
            })
            .collect(),
        _ => Err("remove: expected array or object".into()),
    }
}

fn decode<T, V: DeserializeOwned>(
    bytes: &[u8],
    value: impl Fn(V) -> Result<Option<T>, String>,
) -> Result<File<T>, String> {
    decode_with_removers(bytes, |raw| Ok(value(raw)?.map(|v| (false, v))), |_| Ok(()))
}

fn decode_with_removers<T, R, V: DeserializeOwned>(
    bytes: &[u8],
    value: impl Fn(V) -> Result<Option<(bool, T)>, String>,
    decode_remover: impl Fn(&Value) -> Result<R, String>,
) -> Result<File<T, R>, String> {
    let mut raw: RawObject = serde_json::from_slice(bytes).map_err(|error| error.to_string())?;
    let replace = read_field::<bool>(&mut raw, "replace")?.unwrap_or_default();
    let entries =
        read_field::<BTreeMap<String, V>>(&mut raw, "values")?.ok_or("missing data map values")?;
    let remove = decode_removals(read_field::<Value>(&mut raw, "remove")?, decode_remover)?;
    let mut values = Vec::new();
    for (key, raw) in entries {
        let key = Key::decode(&key)?;
        if let Some((entry_replace, value)) = value(raw)? {
            values.push((key, entry_replace, value));
        }
    }
    Ok(File {
        replace,
        values,
        remove,
    })
}

/// Pack-order loading shared by raw mod maps and typed builtin codecs. Each
/// value decodes before target membership, so even an unknown target cannot
/// conceal a bad value and allow the rest of that file to apply.
fn merge_versions<T: Clone, I: IntoIterator<Item: AsRef<str>>>(
    id: &Identifier,
    registry: &str,
    mut map: BTreeMap<String, T>,
    overlay: &sunrise_world::loot::PackOverlay<'_>,
    tags: impl Fn(&str) -> I,
    prepare: impl Fn(&[u8], &str) -> Result<File<T>, String>,
    known: impl Fn(&str) -> bool,
) -> (BTreeMap<String, T>, bool) {
    let (namespace, path) = (id.namespace(), id.path());
    let directory = folder(registry);
    let wanted = format!("data_maps/{namespace}:{directory}/{path}");
    let mut applied = false;
    if let Some((_, versions)) = overlay.files.iter().find(|(key, _)| *key == wanted) {
        for (layer, bytes) in versions.iter().enumerate() {
            let source = format!(
                "data/{namespace}/data_maps/{directory}/{path}.json (layer {})",
                layer + 1
            );
            match prepare(bytes, &source) {
                Ok(file) => {
                    file.apply(&mut map, &tags, &known, &source);
                    applied = true;
                }
                Err(error) => log::error!(
                    "Sunrise: data map {id} on {registry}: {source}: file skipped: {error}"
                ),
            }
        }
    }
    (map, applied)
}

pub fn load<T: Clone, I: IntoIterator<Item: AsRef<str>>>(
    id: &Identifier,
    registry: &str,
    map: BTreeMap<String, T>,
    overlay: &sunrise_world::loot::PackOverlay<'_>,
    tags: impl Fn(&str) -> I,
    value: impl Fn(&Value) -> Result<T, String>,
    known: impl Fn(&str) -> bool,
) -> (BTreeMap<String, T>, bool) {
    merge_versions(
        id,
        registry,
        map,
        overlay,
        tags,
        |bytes, context| {
            decode(bytes, |mut raw: Value| {
                // A value's own conditions are NeoForge's element conditions alone.
                const KEY: &str = sunrise_world::conditions::NEOFORGE_KEY;
                if let Some(conditions) = raw.get(KEY)
                    && !overlay
                        .conditions
                        .allows_file_isolated(&serde_json::json!({ KEY: conditions }), context)?
                {
                    return Ok(None);
                }
                if let Some(object) = raw.as_object_mut()
                    && object.remove("neoforge:conditions").is_some()
                    && let Some(payload) = object.remove("neoforge:value")
                {
                    raw = payload;
                }
                value(&raw).map(Some)
            })
        },
        known,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn load_registered<T: Clone, R, I: IntoIterator<Item: AsRef<str>>>(
    id: &Identifier,
    registry: &str,
    supports_tags: bool,
    overlay: &sunrise_world::loot::PackOverlay<'_>,
    tags: impl Fn(&str) -> I,
    decode_value: impl Fn(&Value) -> Result<(bool, T), String>,
    decode_remover: impl Fn(&Value) -> Result<R, String>,
    known: impl Fn(&str) -> bool,
    merge: impl Fn(&str, &T, &T) -> Result<T, String>,
    remove_partial: impl Fn(&str, &T, &R) -> Result<Option<T>, String>,
) -> BTreeMap<String, T> {
    let (namespace, path) = (id.namespace(), id.path());
    let directory = folder(registry);
    let wanted = format!("data_maps/{namespace}:{directory}/{path}");
    let mut map = BTreeMap::new();
    if let Some((_, versions)) = overlay.files.iter().find(|(key, _)| *key == wanted) {
        for (layer, bytes) in versions.iter().enumerate() {
            let source = format!(
                "data/{namespace}/data_maps/{directory}/{path}.json (layer {})",
                layer + 1
            );
            let prepared = decode_with_removers(
                bytes,
                |mut raw: Value| {
                    // A value's own conditions are NeoForge's element conditions alone.
                    const KEY: &str = sunrise_world::conditions::NEOFORGE_KEY;
                    if let Some(conditions) = raw.get(KEY)
                        && !overlay.conditions.allows_file_isolated(
                            &serde_json::json!({ KEY: conditions }),
                            &source,
                        )?
                    {
                        return Ok(None);
                    }
                    if let Some(object) = raw.as_object_mut()
                        && object.remove("neoforge:conditions").is_some()
                        && let Some(payload) = object.remove("neoforge:value")
                    {
                        raw = payload;
                    }
                    decode_value(&raw).map(Some)
                },
                &decode_remover,
            );
            match prepared {
                Ok(mut file) => {
                    if !supports_tags {
                        file.filter_disallowed_tags(id, registry, &source);
                    }
                    file.apply_with_hooks(
                        &mut map,
                        &tags,
                        &known,
                        &merge,
                        &remove_partial,
                        &source,
                    );
                }
                Err(error) => log::error!(
                    "Sunrise: data map {id} on {registry}: {source}: file skipped: {error}"
                ),
            }
        }
    }
    map
}

pub struct NumberCodec<F> {
    pub field: &'static str,
    pub decode: F,
}
fn allows_raw(
    object: &mut RawObject,
    conditions: &sunrise_world::conditions::LoadConditions,
    context: &str,
) -> Result<bool, String> {
    let Some(raw) = object.remove("neoforge:conditions") else {
        return Ok(true);
    };
    let value: Value = serde_json::from_str(raw.get()).map_err(|error| error.to_string())?;
    conditions.allows_file_isolated(&serde_json::json!({"neoforge:conditions": value}), context)
}
fn scalar(raw: Box<RawValue>, field: &str) -> Result<Box<RawValue>, String> {
    if !raw.get().trim_start().starts_with('{') {
        return Ok(raw);
    }
    let mut object: RawObject =
        serde_json::from_str(raw.get()).map_err(|error| error.to_string())?;
    object
        .remove(field)
        .ok_or_else(|| format!("missing {field}"))
}
/// Keep numeric entries as RawValue until their codec reads the original decimal,
/// with no f64 round trip through the generic value decoder.
pub fn load_numeric<T: Clone, I: IntoIterator<Item: AsRef<str>>>(
    id: &Identifier,
    registry: &str,
    map: BTreeMap<String, T>,
    overlay: &sunrise_world::loot::PackOverlay<'_>,
    tags: impl Fn(&str) -> I,
    codec: NumberCodec<impl Fn(&RawValue) -> Result<T, String>>,
    known: impl Fn(&str) -> bool,
) -> (BTreeMap<String, T>, bool) {
    merge_versions(
        id,
        registry,
        map,
        overlay,
        tags,
        |bytes, context| {
            decode(bytes, |raw: Box<RawValue>| {
                let payload = if raw.get().trim_start().starts_with('{') {
                    let mut entry: RawObject =
                        serde_json::from_str(raw.get()).map_err(|error| error.to_string())?;
                    if !allows_raw(&mut entry, overlay.conditions, context)? {
                        return Ok(None);
                    }
                    if let Some(value) = entry.remove("value") {
                        let _ = read_field::<bool>(&mut entry, "replace")?;
                        scalar(value, codec.field)?
                    } else {
                        entry
                            .remove(codec.field)
                            .ok_or_else(|| format!("missing {}", codec.field))?
                    }
                } else {
                    raw
                };
                (codec.decode)(&payload).map(Some)
            })
        },
        known,
    )
}

#[cfg(test)]
mod condition_tests {
    use super::*;

    #[test]
    fn a_bad_value_condition_refuses_the_typed_layer_before_any_mutation() {
        let files = vec![("data_maps/neoforge:item/fuel".to_owned(), vec![
            br#"{"values":{"minecraft:stick":2,"minecraft:coal":3}}"#.to_vec(),
            br#"{"replace":true,"values":{"minecraft:blaze_rod":4,"minecraft:planks":{"value":5,"neoforge:conditions":[{"type":"neoforge:or","values":[{"type":"neoforge:always"},{"type":"absent:fuel"}]}]}},"remove":["minecraft:coal"]}"#.to_vec(),
            br#"{"values":{"minecraft:lava_bucket":6}}"#.to_vec(),
        ])];
        let conditions = sunrise_world::conditions::LoadConditions::new([]);
        let overlay = sunrise_world::loot::PackOverlay {
            files: &files,
            conditions: &conditions,
            ..sunrise_world::loot::PackOverlay::default()
        };
        let expected = BTreeMap::from([
            ("minecraft:stick".to_owned(), 2),
            ("minecraft:coal".to_owned(), 3),
            ("minecraft:lava_bucket".to_owned(), 6),
        ]);
        let id = Identifier::parse("neoforge:fuel").unwrap();
        let (values, _) = load(
            &id,
            "minecraft:item",
            BTreeMap::new(),
            &overlay,
            |_| std::iter::empty::<String>(),
            |raw| {
                entry_value(raw)?
                    .as_i64()
                    .ok_or_else(|| "expected integer".into())
            },
            |_| true,
        );
        assert_eq!(values, expected);
        let (values, _) = load_numeric(
            &id,
            "minecraft:item",
            BTreeMap::new(),
            &overlay,
            |_| std::iter::empty::<String>(),
            NumberCodec {
                field: "burn_time",
                decode: |raw: &RawValue| {
                    serde_json::from_str::<i64>(raw.get()).map_err(|error| error.to_string())
                },
            },
            |_| true,
        );
        assert_eq!(values, expected);
        assert_eq!(conditions.check_errors(), Ok(()));
    }

    #[test]
    fn typed_maps_ignore_file_conditions_and_keep_value_conditions() {
        let files = vec![(
            "data_maps/test:item/fuel".to_owned(),
            vec![br#"{"neoforge:conditions":[{"type":"neoforge:never"}],"values":{"minecraft:stick":2,"minecraft:coal":{"neoforge:conditions":[{"type":"neoforge:never"}],"value":3}}}"#.to_vec()],
        )];
        let overlay = sunrise_world::loot::PackOverlay {
            files: &files,
            ..sunrise_world::loot::PackOverlay::default()
        };
        let id = Identifier::parse("test:fuel").unwrap();
        let (values, applied) = load(
            &id,
            "minecraft:item",
            BTreeMap::new(),
            &overlay,
            |_| std::iter::empty::<String>(),
            |raw| Ok(entry_value(raw)?.as_i64().unwrap()),
            |_| true,
        );
        assert!(applied);
        assert_eq!(values, BTreeMap::from([("minecraft:stick".to_owned(), 2)]));
        let (values, applied) = load_numeric(
            &id,
            "minecraft:item",
            BTreeMap::new(),
            &overlay,
            |_| std::iter::empty::<String>(),
            NumberCodec {
                field: "burn_time",
                decode: |raw: &RawValue| {
                    serde_json::from_str::<i64>(raw.get()).map_err(|error| error.to_string())
                },
            },
            |_| true,
        );
        assert!(applied);
        assert_eq!(values, BTreeMap::from([("minecraft:stick".to_owned(), 2)]));
    }
}
