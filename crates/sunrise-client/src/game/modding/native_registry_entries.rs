//! `RegistryAccess.lookup` and `HolderLookup.listElements` on the current side's actual tables.
use super::Game;
use mod_api::{RegistryEntry, RegistryReadError};
use std::{collections::BTreeMap, ops::ControlFlow, sync::Arc};
use sunrise_client::{command::args::normalized, lan::data::SyncData};
use sunrise_core::identifier::Identifier;
use sunrise_protocol::{
    packets::configuration::UpdateTags,
    registry::{Builtin, Dynamic},
};

enum LocalRegistry<'a> {
    Enchantments(&'a sunrise_client::enchant::Enchantments),
    Advancements(&'a sunrise_client::advancements::Tree),
    Rules(&'a sunrise_client::gamerules::RuleRegistry),
    Json(&'a BTreeMap<String, serde_json::Value>),
    Loot(&'a sunrise_client::items::Items),
    Recipes(&'a sunrise_client::crafting::Recipes),
    Professions(&'a sunrise_client::mobs::villager::ResolvedProfessions),
}
impl LocalRegistry<'_> {
    fn contains(&self, id: &str) -> bool {
        match self {
            Self::Enchantments(table) => table.get(id).is_some(),
            Self::Rules(table) => table.position(id).is_some(),
            Self::Advancements(table) => table.by_id.contains_key(id),
            Self::Json(table) => table.contains_key(id),
            Self::Loot(table) => table.has_loot_table(id),
            Self::Professions(table) => table.registry_contains(id),
            Self::Recipes(table) => table.registry_contains(id),
        }
    }
    fn visit_ids(&self, visit: &mut dyn FnMut(&str) -> ControlFlow<()>) -> ControlFlow<()> {
        match self {
            Self::Enchantments(table) => table.registry_ids().try_for_each(visit),
            Self::Advancements(table) => table.by_id.keys().map(String::as_str).try_for_each(visit),
            Self::Rules(table) => table.registry_ids().try_for_each(visit),
            Self::Json(table) => table.keys().map(String::as_str).try_for_each(visit),
            Self::Loot(table) => table.loot_table_ids().try_for_each(visit),
            Self::Professions(table) => table.registry_ids().try_for_each(visit),
            Self::Recipes(table) => table.registry_ids().try_for_each(visit),
        }
    }
}

impl Game<'_> {
    /// The same published provider definitions that `/compute` evaluates.
    pub(in crate::game) fn context_registry(
        &self,
        registry: &str,
    ) -> Option<&BTreeMap<String, serde_json::Value>> {
        self.active_providers().registry(registry)
    }
    fn local_registry(&self, registry: &str) -> Option<LocalRegistry<'_>> {
        Some(match registry {
            "minecraft:enchantment" => {
                LocalRegistry::Enchantments(self.active_enchantment_definitions())
            }
            "minecraft:advancement" => LocalRegistry::Advancements(&self.adv.tree),
            "minecraft:game_rule" => LocalRegistry::Rules(&self.level.rule_registry),
            "minecraft:predicate" => LocalRegistry::Json(&self.fns.store.predicates),
            "minecraft:item_modifier" => LocalRegistry::Json(&self.fns.store.item_modifiers),
            "minecraft:slot_source" => LocalRegistry::Json(&self.fns.store.slot_sources),
            "minecraft:loot_table" => LocalRegistry::Loot(self.items()),
            "minecraft:recipe" => LocalRegistry::Recipes(self.recipes()),
            "minecraft:trade_set" => LocalRegistry::Json(&self.stations().trades.sets),
            "minecraft:villager_trade" => LocalRegistry::Json(&self.stations().trades.trades),
            "minecraft:villager_profession" => {
                LocalRegistry::Professions(&self.stations().professions)
            }
            _ => LocalRegistry::Json(self.context_registry(registry)?),
        })
    }

    fn registry_datapacks(&self, registry: &str) -> Option<Arc<mod_api::DatapackRegistrySet>> {
        if self.is_logical_server() {
            self.mods.datapack_registries.clone()
        } else {
            // This set has already been filtered by the offer, sync policy and accepted schema.
            self.remote
                .as_ref()?
                .datapacks
                .received()
                .filter(|set| set.registry_available(registry))
                .cloned()
        }
    }

    pub(super) fn registry_sync(&self) -> Option<Arc<SyncData>> {
        self.native_lifecycle.epoch()?;
        if !self.is_logical_server() {
            return None;
        }
        self.lan
            .as_ref()
            .map(|host| host.hub.lan.shared.sync())
            .or_else(|| {
                // The same lazy, frozen/current table the server's next connection receives. Never
                // reconstruct a dynamic registry from the prepared files at a mod call boundary.
                super::lan::sync::sync_data(
                    &self.options.resources,
                    &self.res.core.item_data,
                    self.items(),
                    &self.pack_files(),
                )
                .ok()
            })
    }

    pub(super) fn registry_generator_data(
        &self,
    ) -> Option<Arc<sunrise_world::worldgen::generator::GeneratorData>> {
        self.end
            .locator()
            .map(|terrain| Arc::clone(terrain.generator_data()))
            .or_else(|| {
                self.res
                    .behaviours()
                    .live_features()
                    .map(|features| Arc::clone(features.data()))
            })
    }

    fn registry_has_entry(&self, registry: &str, id: &str) -> bool {
        if let Some(remote) = &self.remote {
            if registry == "minecraft:game_rule" {
                return self.ui.rules.registry().position(id).is_some();
            }
            return network_id(&remote.builtin, &remote.registries, registry, id).is_some()
                || self
                    .registry_datapacks(registry)
                    .is_some_and(|set| set.contains_entry(registry, id));
        }
        if !self.is_logical_server() || self.native_lifecycle.epoch().is_none() {
            return false;
        }
        if self
            .registry_datapacks(registry)
            .is_some_and(|set| set.contains_entry(registry, id))
        {
            return true;
        }
        if let Some(table) = self.local_registry(registry) {
            return table.contains(id);
        }
        if registry == "minecraft:dimension" {
            return self.level(id).is_some();
        }
        if registry == "minecraft:trial_spawner" {
            return sunrise_client::mobs::trial_spawner::registry_contains(id).unwrap_or(false);
        }
        if let Some(found) = self
            .res
            .behaviours()
            .live_features()
            .and_then(|features| features.registry_contains(registry, id))
        {
            return found;
        }
        if let Some(found) = self
            .registry_generator_data()
            .and_then(|data| data.registry_contains(registry, id))
        {
            return found;
        }
        self.registry_sync()
            .is_some_and(|sync| network_id(&sync.builtin, &sync.dynamic, registry, id).is_some())
    }

    /// `RegistryAccess.lookup` followed by `HolderGetter.get`: unknown registry/entry is absent.
    pub(super) fn registry_entry(&self, registry: &str, id: &str) -> Option<RegistryEntry> {
        let registry = normalized(registry);
        let id = normalized(id);
        self.registry_has_entry(&registry, &id).then_some(())?;
        Some(RegistryEntry::new(
            Identifier::parse(&registry).ok()?,
            Identifier::parse(&id).ok()?,
        ))
    }

    /// `HolderLookup.listElements`, from the same owners used by `registry_entry`.
    pub(super) fn visit_registry_entries(
        &self,
        registry: &str,
        visit: &mut dyn FnMut(&str) -> ControlFlow<()>,
    ) -> Result<ControlFlow<()>, RegistryReadError> {
        if self.remote.is_none()
            && (!self.is_logical_server() || self.native_lifecycle.epoch().is_none())
        {
            return Err(RegistryReadError::Unavailable);
        }
        let registry = normalized(registry);
        if let Some(set) = self.registry_datapacks(&registry)
            && let Ok(mut entries) = set.entries(&registry)
        {
            return Ok(entries.try_for_each(visit));
        }
        if let Some(remote) = &self.remote {
            if registry == "minecraft:game_rule" {
                return Ok(self.ui.rules.registry().registry_ids().try_for_each(visit));
            }
            return visit_network_entries(&remote.builtin, &remote.registries, &registry, visit);
        }
        if let Some(table) = self.local_registry(&registry) {
            return Ok(table.visit_ids(visit));
        }
        if registry == "minecraft:dimension" {
            return Ok(std::iter::once(self.dim.id())
                .chain(self.levels.parked.keys().map(|dimension| dimension.id()))
                .try_for_each(visit));
        }
        if registry == "minecraft:trial_spawner" {
            return Ok(sunrise_client::mobs::trial_spawner::registry_ids()
                .ok_or(RegistryReadError::UnknownRegistry)?
                .iter()
                .map(String::as_str)
                .try_for_each(visit));
        }
        if let Some(features) = self.res.behaviours().live_features()
            && let Some(entries) = features.registry_ids(&registry)
        {
            return Ok(entries.iter().map(String::as_str).try_for_each(visit));
        }
        if let Some(data) = self.registry_generator_data()
            && let Some(outcome) = data.visit_registry_ids(&registry, visit)
        {
            return Ok(outcome);
        }
        let sync = self.registry_sync().ok_or(RegistryReadError::Unavailable)?;
        visit_network_entries(&sync.builtin, &sync.dynamic, &registry, visit)
    }

    /// `Holder.is(TagKey)`: plain values are resolved again against the current table.
    /// An absent tag is false; an absent registry or entry is None.
    pub(super) fn registry_entry_in_tag(&self, entry: &RegistryEntry, tag: &str) -> Option<bool> {
        let registry = entry.registry.to_string();
        let id = entry.id.to_string();
        if registry == "minecraft:damage_type" && self.is_logical_server() {
            self.native_lifecycle.epoch()?;
            let damage = &self.health_resources.damage;
            return damage.id(&id).map(|id| damage.in_tag(id, &normalized(tag)));
        }
        if !self.registry_has_entry(&registry, &id) {
            return None;
        }
        let tag = normalized(tag);
        if let Some(set) = self.registry_datapacks(&registry)
            && let Some(found) = set.tag_contains(&registry, &id, &tag)
        {
            return Some(found);
        }
        if let Some(remote) = &self.remote {
            let Some(index) = network_id(&remote.builtin, &remote.registries, &registry, &id)
            else {
                return Some(false);
            };
            return Some(
                remote
                    .registries
                    .tag(&registry, &tag)
                    .is_some_and(|members| members.contains(&index)),
            );
        }
        if matches!(
            registry.as_str(),
            "minecraft:enchantment" | "minecraft:entity_type"
        ) {
            return Some(self.sync_tag_contains(&registry, &id, &tag));
        }
        if registry == "minecraft:slot_source" {
            return Some(
                self.fns
                    .store
                    .slot_source_tag(&tag)
                    .is_ok_and(|members| members.iter().any(|member| member == &id)),
            );
        }
        if registry == "minecraft:villager_trade" {
            return Some(
                self.stations()
                    .trades
                    .tags
                    .get(tag.as_ref())
                    .is_some_and(|members| members.iter().any(|member| member == &id)),
            );
        }
        let item_tags = match registry.as_str() {
            "minecraft:block" => Some(sunrise_client::items::TagRegistry::Block),
            "minecraft:item" => Some(sunrise_client::items::TagRegistry::Item),
            "minecraft:fluid" => Some(sunrise_client::items::TagRegistry::Fluid),
            _ => None,
        };
        if let Some(kind) = item_tags {
            return Some(
                self.items()
                    .tag_members(kind, &tag)
                    .is_some_and(|members| members.iter().any(|member| member == &id)),
            );
        }
        if let Some(found) = self
            .res
            .behaviours()
            .live_features()
            .and_then(|features| features.registry_tag_contains(&registry, &id, &tag))
        {
            return Some(found);
        }
        if let Some(found) = self
            .registry_generator_data()
            .and_then(|data| data.registry_tag_contains(&registry, &id, &tag))
        {
            return Some(found);
        }
        if let Some(tables) = &self.world_tables
            && let Some(kind) = registry.strip_prefix("minecraft:")
            && let Some(tags) = tables.pack_tags.get(kind)
        {
            return Some(
                tags.get(tag.as_ref())
                    .is_some_and(|members| members.contains(&id)),
            );
        }
        Some(self.sync_tag_contains(&registry, &id, &tag))
    }

    /// `HolderLookup.RegistryLookup.get(TagKey)`: the members of a tag in `HolderSet.Named` order,
    /// from the same owners as `registry_entry_in_tag`. An absent tag is empty (`getTagOrEmpty`);
    /// an unknown or unavailable registry is None.
    pub(super) fn registry_tag_members(
        &self,
        registry: &str,
        tag: &str,
    ) -> Option<Vec<Identifier>> {
        let registry = normalized(registry);
        let _ = self
            .visit_registry_entries(&registry, &mut |_| ControlFlow::Break(()))
            .ok()?;
        let tag = normalized(tag);
        if let Some(set) = self.registry_datapacks(&registry)
            && let Some(members) = set.tag_members(&registry, &tag)
        {
            return identifiers(members.iter().map(String::as_str));
        }
        if let Some(remote) = &self.remote {
            return network_tag_members(
                &remote.builtin,
                &remote.registries,
                &registry,
                remote.registries.tag(&registry, &tag),
            );
        }
        if registry == "minecraft:slot_source" {
            let members = self.fns.store.slot_source_tag(&tag).unwrap_or_default();
            return identifiers(members.iter().map(String::as_str));
        }
        if registry == "minecraft:villager_trade" {
            let members = self.stations().trades.tags.get(tag.as_ref());
            return identifiers(members.into_iter().flatten().map(String::as_str));
        }
        let item_tags = match registry.as_ref() {
            "minecraft:block" => Some(sunrise_client::items::TagRegistry::Block),
            "minecraft:item" => Some(sunrise_client::items::TagRegistry::Item),
            "minecraft:fluid" => Some(sunrise_client::items::TagRegistry::Fluid),
            _ => None,
        };
        if let Some(kind) = item_tags {
            let members = self.items().tag_members(kind, &tag).unwrap_or_default();
            return identifiers(members.iter().map(String::as_str));
        }
        let sync = self.registry_sync()?;
        network_tag_members(
            &sync.builtin,
            &sync.dynamic,
            &registry,
            sync_tag_ids(&sync.tags, &registry, &tag),
        )
    }

    fn sync_tag_contains(&self, registry: &str, id: &str, tag: &str) -> bool {
        let Some(sync) = self.registry_sync() else {
            return false;
        };
        let Some(index) = network_id(&sync.builtin, &sync.dynamic, registry, id) else {
            return false;
        };
        sync.tags
            .registries
            .iter()
            .find(|tags| tags.registry == registry)
            .and_then(|tags| tags.tags.iter().find(|candidate| candidate.name == tag))
            .is_some_and(|tag| tag.ids.contains(&index))
    }

    pub(super) fn registry_entry_key_slot(&self, entry: &RegistryEntry) -> Option<u32> {
        if !self.registry_has_entry(&entry.registry.to_string(), &entry.id.to_string()) {
            return None;
        }
        mod_api::__private::registry_entry_key_slot(entry)
    }
}

fn network_id(builtin: &Builtin, dynamic: &Dynamic, registry: &str, id: &str) -> Option<i32> {
    builtin
        .get(registry)
        .and_then(|entries| entries.id(id))
        .or_else(|| dynamic.id(registry, id))
        .or_else(|| {
            (dynamic.custom_components && registry == "minecraft:data_component_type")
                .then(|| sunrise_protocol::item::custom::id(id))
                .flatten()
        })
}
fn visit_network_entries(
    builtin: &Builtin,
    dynamic: &Dynamic,
    registry: &str,
    visit: &mut dyn FnMut(&str) -> ControlFlow<()>,
) -> Result<ControlFlow<()>, RegistryReadError> {
    if let Some(ids) = builtin.get(registry) {
        let flow = ids
            .names
            .iter()
            .map(String::as_str)
            .filter(|name| !name.is_empty())
            .try_for_each(&mut *visit);
        if flow.is_break()
            || !dynamic.custom_components
            || registry != "minecraft:data_component_type"
        {
            return Ok(flow);
        }
        return Ok(sunrise_protocol::item::custom::entries()
            .filter(|(_, name)| ids.id(name).is_none())
            .map(|(_, name)| name)
            .try_for_each(visit));
    }
    Ok(dynamic
        .registries
        .get(registry)
        .ok_or(RegistryReadError::UnknownRegistry)?
        .iter()
        .map(|(name, _)| name.as_str())
        .try_for_each(visit))
}

fn identifiers<'a>(entries: impl IntoIterator<Item = &'a str>) -> Option<Vec<Identifier>> {
    Some(
        entries
            .into_iter()
            .filter_map(|name| Identifier::parse(name).ok())
            .collect(),
    )
}

/// A tag's members from a network tag table, in the order the table lists them (the order
/// `TagNetworkSerialization` sent). An absent tag is empty; a registry neither table holds is None.
fn network_tag_members(
    builtin: &Builtin,
    dynamic: &Dynamic,
    registry: &str,
    ids: Option<&[i32]>,
) -> Option<Vec<Identifier>> {
    if !builtin.registries.contains_key(registry) && !dynamic.registries.contains_key(registry) {
        return None;
    }
    let name = |id: i32| {
        builtin
            .get(registry)
            .and_then(|entries| entries.name(id))
            .or_else(|| dynamic.name(registry, id))
    };
    identifiers(ids.into_iter().flatten().filter_map(|id| name(*id)))
}

/// One tag's ids in a server's `UpdateTags`, in the order it lists them.
fn sync_tag_ids<'a>(tags: &'a UpdateTags, registry: &str, tag: &str) -> Option<&'a [i32]> {
    tags.registries
        .iter()
        .find(|tags| tags.registry == registry)?
        .tags
        .iter()
        .find(|candidate| candidate.name == tag)
        .map(|tag| tag.ids.as_slice())
}

/// Bind a mod registry's tags against the entries it just admitted. A missing
/// required member prevents that tag from binding, not the other tags.
pub(super) fn load_registry_tags(
    root: &std::path::Path,
    overlay: &sunrise_world::loot::PackOverlay<'_>,
    folder: &str,
    known: impl Fn(&str) -> bool,
) -> Result<BTreeMap<String, Vec<String>>, String> {
    use sunrise_world::{damage_source::resource_files, worldgen::tags::TagDefinitions};
    let tag_folder = format!("tags/{folder}");
    let mut files: BTreeMap<String, Vec<Vec<u8>>> = resource_files(root, &tag_folder)
        .map_err(|error| error.to_string())?
        .into_iter()
        .map(|(id, bytes)| (id, vec![bytes]))
        .collect();
    for (id, versions) in overlay.tag_kind(folder) {
        files
            .entry(id)
            .or_default()
            .extend(versions.iter().cloned());
    }
    let mut files: Vec<_> = files.into_iter().collect();
    files.sort_by_cached_key(|(id, _)| {
        let (namespace, path) = id.split_once(':').unwrap_or(("minecraft", id));
        (format!("{path}.json"), namespace.to_owned())
    });
    let mut definitions = TagDefinitions::default();
    for (id, versions) in files {
        for bytes in versions {
            if let Ok(json) = std::str::from_utf8(&bytes) {
                // TagLoader retains lower-priority members when a tag codec fails.
                let _ = definitions.apply_json(&id, "registry tags", json);
            }
        }
    }
    Ok(definitions
        .bind(|id, _| known(id).then(|| id.to_owned()))
        .tags)
}

#[cfg(test)]
mod condition_tests {
    #[test]
    fn registry_tags_ignore_top_level_conditions() {
        let root = tempfile::tempdir().unwrap();
        let files = vec![(
            "tags/test:spell/hot".to_owned(),
            vec![
                br#"{"neoforge:conditions":[{"type":"neoforge:never"}],"values":["test:fire"]}"#
                    .to_vec(),
            ],
        )];
        let tags = super::load_registry_tags(
            root.path(),
            &sunrise_world::loot::PackOverlay {
                files: &files,
                ..sunrise_world::loot::PackOverlay::default()
            },
            "spell",
            |id| id == "test:fire",
        )
        .unwrap();
        assert_eq!(tags["test:hot"], ["test:fire"]);
    }
}

#[cfg(test)]
mod tag_member_tests {
    use super::{network_tag_members, sync_tag_ids};
    use serde_json::json;
    use sunrise_protocol::{
        packets::configuration::{RegistryTags, TagEntry, UpdateTags},
        registry::{Builtin, Dynamic},
    };
    use sunrise_world::tag_layers::TagLayers;

    const ENCHANTMENT: &str = "minecraft:enchantment";

    fn enchantments() -> Dynamic {
        let names = ["a", "b", "c", "d", "e"];
        Dynamic {
            registries: [(
                ENCHANTMENT.to_owned(),
                names
                    .map(|name| (format!("minecraft:{name}"), None))
                    .to_vec(),
            )]
            .into(),
            ..Dynamic::default()
        }
    }

    fn names(members: Option<Vec<sunrise_core::identifier::Identifier>>) -> Option<Vec<String>> {
        members.map(|members| members.iter().map(ToString::to_string).collect())
    }

    #[test]
    fn tooltip_order_keeps_tag_order_with_a_nested_tag_expanded_in_place() {
        let dynamic = enchantments();
        let raw = [
            (
                "minecraft:tooltip_order",
                json!({"values": ["minecraft:e", "#minecraft:nested", "minecraft:a"]}),
            ),
            (
                "minecraft:nested",
                json!({"values": ["minecraft:d", "minecraft:b"]}),
            ),
        ]
        .into_iter()
        .map(|(name, file)| (name.to_owned(), TagLayers::from_file(file).unwrap()))
        .collect();
        let bound = TagLayers::bind("enchantment", &raw, |id| {
            dynamic.id(ENCHANTMENT, id).is_some()
        });
        let tags = UpdateTags {
            registries: vec![RegistryTags {
                registry: ENCHANTMENT.to_owned(),
                tags: bound
                    .iter()
                    .map(|(name, members)| TagEntry {
                        name: name.clone(),
                        ids: members
                            .iter()
                            .map(|id| dynamic.id(ENCHANTMENT, id).unwrap())
                            .collect(),
                    })
                    .collect(),
            }],
        };
        let ids = sync_tag_ids(&tags, ENCHANTMENT, "minecraft:tooltip_order");
        assert_eq!(
            names(network_tag_members(
                &Builtin::default(),
                &dynamic,
                ENCHANTMENT,
                ids
            )),
            Some(
                ["minecraft:e", "minecraft:d", "minecraft:b", "minecraft:a"]
                    .map(String::from)
                    .to_vec()
            )
        );
    }

    #[test]
    fn an_absent_tag_is_empty_and_an_unknown_registry_is_none() {
        let dynamic = enchantments();
        let tags = UpdateTags { registries: vec![] };
        let absent = sync_tag_ids(&tags, ENCHANTMENT, "minecraft:missing");
        assert_eq!(
            names(network_tag_members(
                &Builtin::default(),
                &dynamic,
                ENCHANTMENT,
                absent
            )),
            Some(vec![])
        );
        assert_eq!(
            network_tag_members(&Builtin::default(), &dynamic, "test:nothing", None),
            None
        );
    }
}

#[cfg(test)]
mod network_entry_tests {
    use super::*;

    #[test]
    fn builtin_entries_visit_only_canonical_ids_in_numeric_order() {
        let builtin = Builtin::parse(
            r#"{"minecraft:item":{"entries":{
                "minecraft:Bad":{"protocol_id":2},
                "minecraft:apple":{"protocol_id":5},
                "stick":{"protocol_id":1}
            }}}"#,
        )
        .unwrap();
        let mut seen = Vec::new();
        assert_eq!(
            visit_network_entries(&builtin, &Dynamic::default(), "minecraft:item", &mut |id| {
                seen.push(id.to_owned());
                ControlFlow::Continue(())
            }),
            Ok(ControlFlow::Continue(()))
        );
        assert_eq!(seen, ["minecraft:stick", "minecraft:apple"]);
        let items = builtin.get("minecraft:item").unwrap();
        assert_eq!(items.id("minecraft:stick"), Some(1));
        assert_eq!(items.id("minecraft:Bad"), None);
        assert_eq!(items.id("minecraft:apple"), Some(5));
    }
}
