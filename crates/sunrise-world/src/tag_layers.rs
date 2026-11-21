//! Fold resource-pack tag layers, then bind the resulting entries to a registry.
mod order;
pub(crate) use order::dependency_order;

use crate::loot::PackOverlay;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::{io, path::Path};

/// A tag's entries over its layers, in pack order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TagLayers(Vec<Entry>);

#[derive(Clone, Debug, PartialEq, Eq)]
struct Entry {
    id: String,
    required: bool,
    remove: bool,
}

/// A tag file entry's id: a string, or an object's `id` (`TagEntry.CODEC`).
pub fn entry_id(entry: &Value) -> Option<&str> {
    entry.as_str().or_else(|| entry.get("id")?.as_str())
}

impl TagLayers {
    /// The layers of one file.
    pub fn from_file(file: &Value) -> Self {
        let mut layers = Self::default();
        layers.push(file);
        layers
    }

    /// One more file, over the layers below it (conditions already
    /// checked): `replace` drops them, then its `values` and `remove` follow.
    /// Inside `values`, an entry of `{"id": ..., "remove": true}`
    /// (`TagLoader.EntryWithSource.remove`) removes like a `remove` entry.
    pub fn push(&mut self, file: &Value) {
        if file.get("replace").and_then(Value::as_bool) == Some(true) {
            self.0.clear();
        }
        for (field, remove) in [("values", false), ("remove", true)] {
            for entry in file
                .get(field)
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                let Some(id) = entry_id(entry) else {
                    continue;
                };
                let (prefix, name) = id.strip_prefix('#').map_or(("", id), |name| ("#", name));
                let (ns, path) = name.split_once(':').unwrap_or(("minecraft", name));
                let ns = if ns.is_empty() { "minecraft" } else { ns };
                let id = format!("{prefix}{ns}:{path}");
                self.0.push(Entry {
                    id,
                    required: entry
                        .get("required")
                        .and_then(Value::as_bool)
                        .unwrap_or(true),
                    remove: remove || entry.get("remove").and_then(Value::as_bool) == Some(true),
                });
            }
        }
    }

    /// The `#tags` the entries name, adding or removing.
    pub fn tags(&self) -> impl Iterator<Item = &str> {
        self.0.iter().filter_map(|entry| entry.id.strip_prefix('#'))
    }

    /// Bind one tag to registry IDs and already bound nested tags. A missing
    /// required addition fails the entire tag; missing optional entries and
    /// removals never do. Members retain their insertion order.
    pub fn build<'a>(
        &self,
        contains: impl Fn(&str) -> bool,
        mut members: impl FnMut(&str) -> Option<&'a [String]>,
    ) -> Option<Vec<String>> {
        let mut out = Vec::new();
        for entry in &self.0 {
            let ids = match entry.id.strip_prefix('#') {
                Some(tag) => members(tag),
                None => contains(&entry.id).then_some(std::slice::from_ref(&entry.id)),
            };
            let Some(ids) = ids else {
                if entry.required && !entry.remove {
                    return None;
                }
                continue;
            };
            for id in ids {
                if entry.remove {
                    out.retain(|kept| kept != id);
                } else if !out.contains(id) {
                    out.push(id.clone());
                }
            }
        }
        Some(out)
    }

    /// Bind the registry's complete tag stack before publishing it. Admit all
    /// required dependencies before optional ones, dropping edges that would
    /// close a cycle. Build each tag once, after its admitted dependencies.
    pub fn bind(
        kind: &str,
        raw: &HashMap<String, Self>,
        contains: impl Fn(&str) -> bool,
    ) -> HashMap<String, Vec<String>> {
        let mut names: Vec<_> = raw.keys().map(String::as_str).collect();
        // ResourceManager lists tag files in Identifier order (path first,
        // namespace second), including the extension in the path comparison.
        names.sort_by_cached_key(|name| {
            let (namespace, path) = name.split_once(':').unwrap_or(("minecraft", name));
            (format!("{path}.json"), namespace)
        });
        let sorted = dependency_order(names, |name| {
            raw[name].0.iter().filter_map(|entry| {
                Some((entry.id.strip_prefix('#')?, entry.required && !entry.remove))
            })
        });
        let mut bound: HashMap<String, Vec<String>> = HashMap::new();
        for name in sorted {
            let Some(entries) = raw.get(name) else {
                continue;
            };
            match entries.build(&contains, |id| bound.get(id).map(Vec::as_slice)) {
                Some(members) => {
                    bound.insert(name.to_owned(), members);
                }
                None => {
                    log::warn!("Sunrise: couldn't bind {kind} tag {name}: missing required member")
                }
            }
        }
        bound
    }
}

/// Read a registry's resource and world-pack tag stack, then bind it against
/// the registry admitted by the owning load. Conditions do not gate tag files.
pub fn load(
    root: &Path,
    kind: &str,
    overlay: &PackOverlay<'_>,
    contains: impl Fn(&str) -> bool,
) -> io::Result<HashMap<String, Vec<String>>> {
    Ok(TagLayers::bind(
        kind,
        &layers(root, kind, overlay)?,
        contains,
    ))
}

pub(crate) fn layers(
    root: &Path,
    kind: &str,
    overlay: &PackOverlay<'_>,
) -> io::Result<HashMap<String, TagLayers>> {
    let mut raw = HashMap::<_, TagLayers>::new();
    let data = root.join("data");
    for namespace in crate::json_folder::namespaces(&data) {
        let dir = data.join(&namespace).join("tags").join(kind);
        let folder = match crate::json_folder::read(&dir) {
            Ok(folder) => folder,
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error),
        };
        for file in &folder.files {
            match file
                .bytes()
                .and_then(|bytes| Ok(serde_json::from_slice(bytes)?))
            {
                Ok(value) => {
                    raw.insert(
                        format!("{namespace}:{}", file.name),
                        TagLayers::from_file(&value),
                    );
                }
                Err(error) => log::warn!(
                    "Sunrise: couldn't read tag file {}: {error}",
                    file.path.display()
                ),
            }
        }
    }
    for (id, files) in overlay.tag_kind(kind) {
        for bytes in files {
            match serde_json::from_slice(bytes) {
                Ok(value) => raw.entry(id.clone()).or_default().push(&value),
                Err(error) => log::warn!("Sunrise: couldn't read {kind} tag {id}: {error}"),
            }
        }
    }
    Ok(raw)
}

/// Registry IDs from the prepared game report, which includes admitted
/// content-mod entries. A synthetic root may not have a report.
pub fn builtin_ids(root: &Path) -> io::Result<HashMap<String, HashSet<String>>> {
    let path = root.join("registries.json");
    let value = match std::fs::read(path) {
        Ok(bytes) => serde_json::from_slice::<Value>(&bytes)?,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(HashMap::new()),
        Err(error) => return Err(error),
    };
    Ok(value
        .as_object()
        .into_iter()
        .flatten()
        .filter_map(|(kind, value)| {
            let entries = value["entries"].as_object()?;
            Some((
                kind.strip_prefix("minecraft:").unwrap_or(kind).to_owned(),
                entries.keys().cloned().collect(),
            ))
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn removals_apply_in_layer_order_and_replace_drops_them() {
        let mut things = TagLayers::from_file(&json!({"values": ["stone", "dirt", "#test:other"]}));
        things.push(&json!({"values": [], "remove": ["dirt", "#test:other"]}));
        things.push(&json!({"values": ["gravel", "sand", {"id":"stone", "remove":true}]}));
        let other = ["minecraft:sand".into()];
        let members = |tag: &str| (tag == "test:other").then_some(other.as_slice());
        assert_eq!(
            things.build(|_| true, members).unwrap(),
            ["minecraft:gravel", "minecraft:sand"]
        );
        things.push(&json!({"replace": true, "values": ["dirt"]}));
        assert_eq!(things.build(|_| true, members).unwrap(), ["minecraft:dirt"]);
    }

    #[test]
    fn unresolved_required_entries_omit_tags_and_optional_entries_do_not() {
        let raw = [
            ("test:empty", json!({"values":[]})),
            ("test:required", json!({"values":["test:pinned", {"id":"test:absent", "required":true}]})),
            ("test:dependent", json!({"values":["#test:required"]})),
            ("test:optional", json!({"values":[{"id":"test:absent", "required":false}, {"id":"#test:required", "required":false}], "remove":["#test:required", "test:absent"]})),
            ("test:admitted", json!({"values":["#test:empty", "test:pinned", "test:pinned"]})),
        ].into_iter().map(|(name, file)| (name.to_owned(), TagLayers::from_file(&file))).collect();
        assert_eq!(
            TagLayers::bind("item", &raw, |id| id == "test:pinned"),
            HashMap::from([
                ("test:empty".into(), vec![]),
                ("test:optional".into(), vec![]),
                ("test:admitted".into(), vec!["test:pinned".into()]),
            ])
        );
    }

    #[test]
    fn shared_nested_tags_are_built_once() {
        let mut raw = HashMap::new();
        for i in 0..128 {
            let child = format!("#test:{}", i + 1);
            raw.insert(
                format!("test:{i}"),
                TagLayers::from_file(&json!({"values":[child,child]})),
            );
        }
        raw.insert(
            "test:128".into(),
            TagLayers::from_file(&json!({"values":["minecraft:stick", "minecraft:potion"]})),
        );
        let calls = std::cell::Cell::new(0);
        let tags = TagLayers::bind("item", &raw, |_| {
            calls.set(calls.get() + 1);
            assert!(calls.get() <= 2);
            true
        });
        assert_eq!(tags["test:0"], ["minecraft:stick", "minecraft:potion"]);
    }

    #[test]
    fn an_optional_cycle_binds_in_java_identifier_bucket_order() {
        let raw = HashMap::from([
            (
                "test:a".into(),
                TagLayers::from_file(
                    &json!({"values": ["sand", {"id":"#test:b", "required":false}]}),
                ),
            ),
            (
                "test:b".into(),
                TagLayers::from_file(
                    &json!({"values": ["dirt", {"id":"#test:a", "required":false}]}),
                ),
            ),
        ]);
        let bound = TagLayers::bind("block", &raw, |_| true);
        // b is in bucket 2 and a in bucket 13; b's optional edge wins.
        assert_eq!(bound["test:a"], ["minecraft:sand"]);
        assert_eq!(bound["test:b"], ["minecraft:dirt", "minecraft:sand"]);
    }

    #[test]
    fn required_dependencies_take_priority_over_optional_cycles() {
        let raw = HashMap::from([
            (
                "test:a".into(),
                TagLayers::from_file(&json!({"values": ["#test:b", "sand"]})),
            ),
            (
                "test:b".into(),
                TagLayers::from_file(
                    &json!({"values": [{"id":"#test:a", "required":false}, "dirt"]}),
                ),
            ),
        ]);
        let bound = TagLayers::bind("block", &raw, |_| true);
        assert_eq!(bound["test:a"], ["minecraft:dirt", "minecraft:sand"]);
        assert_eq!(bound["test:b"], ["minecraft:dirt"]);
    }
}
