//! NeoForge's built-in block and block-transformer data maps (`NeoForgeDataMaps`):
//! `neoforge:flammable`, `neoforge:oxidizables`, `neoforge:waxables`,
//! `neoforge:transformables`, and `neoforge:block_transform_appenders`, merged
//! from the world's data packs and mods' files as a mod's own data maps are.
use crate::game::ServerGame;
use serde_json::{Map, Value};
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use sunrise_core::{identifier::Identifier, nbt::Tag};
use sunrise_gameplay_common::game::{
    data_map_json::to_tag,
    native_data_maps_common::{MapRegistries, WorldRegistries, load_maps},
};
use sunrise_sim::block_transform::{Transform, parse_transform};
use sunrise_world::{
    level::{BlockMapValue, BlockMaps},
    registry::BlockRegistry,
};

/// The three block-behaviour maps' ids, all on `minecraft:block`.
const MAPS: [&str; 3] = [
    "neoforge:flammable",
    "neoforge:oxidizables",
    "neoforge:waxables",
];

const TRANSFORMABLES_ID: &str = "neoforge:transformables";
const BLOCK_TRANSFORM_APPENDERS_ID: &str = "neoforge:block_transform_appenders";

#[derive(Clone)]
enum TransformMapValue {
    Transformable(BTreeMap<String, (Transform, Value)>),
    Appender(Vec<(Transform, Value)>),
}

/// Loaded `neoforge:transformables` and `neoforge:block_transform_appenders` data maps
/// alongside the extra `Transform`s appended to each `BlockTransformer` in NeoForge's
/// `DataMapHooks.appendDatamapTransformers` order (appenders first, then transformables).
pub(in crate::game::server_side) struct LoadedTransformMaps {
    pub values: Vec<(&'static str, &'static str, Vec<(String, Tag)>)>,
    pub extra_transformers: HashMap<String, Vec<Transform>>,
}

/// Load `neoforge:transformables` (`minecraft:block`, `supports_tags = false`, map merger,
/// transformer-id partial remover) and `neoforge:block_transform_appenders`
/// (`minecraft:block_transformer`, list merger) reusing `block_transform::parse_transform`.
pub(in crate::game::server_side) fn load_transform_maps(
    overlay: &sunrise_world::loot::PackOverlay<'_>,
    registries: &dyn MapRegistries,
    block_registry: &BlockRegistry,
) -> LoadedTransformMaps {
    let block_reg = Identifier::parse_namespaced("minecraft:block").unwrap();
    let transformer_reg = Identifier::parse_namespaced("minecraft:block_transformer").unwrap();
    let types = vec![
        Arc::new(mod_api::__private::RegisteredDataMap::from_raw(
            "neoforge",
            block_reg,
            Identifier::parse_namespaced(TRANSFORMABLES_ID).unwrap(),
            mod_api::RawDataMap::new("minecraft:block", TRANSFORMABLES_ID).supports_tags(false),
        )),
        Arc::new(mod_api::__private::RegisteredDataMap::from_raw(
            "neoforge",
            transformer_reg,
            Identifier::parse_namespaced(BLOCK_TRANSFORM_APPENDERS_ID).unwrap(),
            mod_api::RawDataMap::new("minecraft:block_transformer", BLOCK_TRANSFORM_APPENDERS_ID),
        )),
    ];

    let mut loaded = load_maps(
        &types,
        overlay,
        registries,
        |id, value| match id {
            TRANSFORMABLES_ID => {
                let Some(object) = value.as_object() else {
                    return Err("transformable must be an object".into());
                };
                let mut entries = BTreeMap::new();
                if let (Some(transformer), Some(data)) =
                    (object.get("transformer"), object.get("transform_data"))
                {
                    let raw_id = transformer
                        .as_str()
                        .ok_or("transformer must be a block transformer identifier")?;
                    let transformer_id = Identifier::parse(raw_id)
                        .map_err(|error| format!("transformer: {error}"))?
                        .to_string();
                    let _ = to_tag(data)
                        .map_err(|_| "transform_data cannot be represented as NBT".to_owned())?;
                    let parsed = parse_transform(data, block_registry)?;
                    entries.insert(transformer_id, (parsed, data.clone()));
                } else if let Some(transformers) = object.get("transformers") {
                    let Some(map) = transformers.as_object() else {
                        return Err("transformers must be an object".into());
                    };
                    for (raw_id, data) in map {
                        let transformer_id = Identifier::parse(raw_id)
                            .map_err(|error| format!("transformers.{raw_id}: {error}"))?
                            .to_string();
                        let _ = to_tag(data).map_err(|_| {
                            format!("transformers.{raw_id} cannot be represented as NBT")
                        })?;
                        let parsed = parse_transform(data, block_registry)?;
                        entries.insert(transformer_id, (parsed, data.clone()));
                    }
                } else {
                    return Err(
                        "transformable requires {transformer, transform_data} or {transformers}"
                            .into(),
                    );
                }
                Ok(TransformMapValue::Transformable(entries))
            }
            BLOCK_TRANSFORM_APPENDERS_ID => {
                let Some(items) = value.as_array() else {
                    return Err("block_transform_appenders value must be an array".into());
                };
                let mut entries = Vec::with_capacity(items.len());
                for item in items {
                    let _ = to_tag(item).map_err(|_| {
                        "block_transform_appenders entry cannot be represented as NBT".to_owned()
                    })?;
                    let parsed = parse_transform(item, block_registry)?;
                    entries.push((parsed, item.clone()));
                }
                Ok(TransformMapValue::Appender(entries))
            }
            _ => unreachable!("only transformables and block_transform_appenders are loaded here"),
        },
        |id, rem| match id {
            // NeoForge `Transformable.Remover.CODEC`: `ResourceKey.codec(Registries.BLOCK_TRANSFORMER)`.
            TRANSFORMABLES_ID => {
                let raw = rem.as_str().ok_or_else(|| {
                    "transformables remover must be a block transformer identifier".to_owned()
                })?;
                Ok(Identifier::parse(raw)
                    .map_err(|error| format!("transformables remover: {error}"))?
                    .to_string())
            }
            _ => Err("data map does not support partial removers".into()),
        },
        |_reg, _entry, existing, incoming| match (existing, incoming) {
            (
                TransformMapValue::Transformable(existing),
                TransformMapValue::Transformable(incoming),
            ) => {
                let mut merged = existing.clone();
                for (k, v) in incoming {
                    merged.insert(k.clone(), v.clone());
                }
                Ok(TransformMapValue::Transformable(merged))
            }
            (TransformMapValue::Appender(existing), TransformMapValue::Appender(incoming)) => {
                let mut merged = existing.clone();
                merged.extend(incoming.iter().cloned());
                Ok(TransformMapValue::Appender(merged))
            }
            _ => Err("mismatched transform data map value kinds".into()),
        },
        |_reg, _entry, existing, remover| match existing {
            TransformMapValue::Transformable(map) => {
                if !map.contains_key(remover) {
                    return Ok(Some(TransformMapValue::Transformable(map.clone())));
                }
                let mut reduced = map.clone();
                reduced.remove(remover);
                if reduced.is_empty() {
                    Ok(None)
                } else {
                    Ok(Some(TransformMapValue::Transformable(reduced)))
                }
            }
            TransformMapValue::Appender(_) => {
                Err("data map does not support partial removers".into())
            }
        },
    );

    let mut extra_transformers: HashMap<String, Vec<Transform>> = HashMap::new();
    let mut appender_tags = Vec::new();
    if let Some(mut by_map) = loaded.remove("minecraft:block_transformer")
        && let Some(entries) = by_map.remove(BLOCK_TRANSFORM_APPENDERS_ID)
    {
        for (transformer_id, value) in entries {
            if let TransformMapValue::Appender(items) = value {
                let json = Value::Array(items.iter().map(|(_, j)| j.clone()).collect());
                if let Ok(tag) = to_tag(&json) {
                    appender_tags.push((transformer_id.clone(), tag));
                }
                extra_transformers
                    .entry(transformer_id)
                    .or_default()
                    .extend(items.into_iter().map(|(t, _)| t));
            }
        }
    }

    let mut transformable_tags = Vec::new();
    if let Some(mut by_map) = loaded.remove("minecraft:block")
        && let Some(entries) = by_map.remove(TRANSFORMABLES_ID)
    {
        for (block_id, value) in entries {
            if let TransformMapValue::Transformable(map) = value {
                let json = if map.len() == 1 {
                    let (id, (_, data)) = map.iter().next().unwrap();
                    serde_json::json!({
                        "transformer": id,
                        "transform_data": data,
                    })
                } else {
                    let mut obj = Map::new();
                    for (id, (_, data)) in &map {
                        obj.insert(id.clone(), data.clone());
                    }
                    serde_json::json!({ "transformers": obj })
                };
                if let Ok(tag) = to_tag(&json) {
                    transformable_tags.push((block_id, tag));
                }
                for (transformer_id, (transform, _)) in map {
                    extra_transformers
                        .entry(transformer_id)
                        .or_default()
                        .push(transform);
                }
            }
        }
    }

    LoadedTransformMaps {
        values: vec![
            ("minecraft:block", TRANSFORMABLES_ID, transformable_tags),
            (
                "minecraft:block_transformer",
                BLOCK_TRANSFORM_APPENDERS_ID,
                appender_tags,
            ),
        ],
        extra_transformers,
    }
}

/// Load the world's block maps. Resolve targets against `block_id` before merging each layer,
/// as NeoForge's `Waxable` and `Oxidizable` codecs do. The block id also orders inverse ties.
fn load(
    overlay: &sunrise_world::loot::PackOverlay<'_>,
    registries: &dyn MapRegistries,
    block_id: impl Fn(&str) -> Option<u32>,
) -> Option<Arc<BlockMaps>> {
    let block_registry = Identifier::parse_namespaced("minecraft:block").unwrap();
    let types: Vec<_> = MAPS
        .into_iter()
        .map(|id| {
            Arc::new(mod_api::__private::RegisteredDataMap::from_raw(
                "neoforge",
                block_registry.clone(),
                Identifier::parse_namespaced(id).unwrap(),
                mod_api::RawDataMap::new("minecraft:block", id),
            ))
        })
        .collect();
    let mut maps = load_maps(
        &types,
        overlay,
        registries,
        |id, value| {
            let (field, transform): (&str, fn(String) -> BlockMapValue) = match id {
                "neoforge:flammable" => {
                    let read_odds = |field| {
                        value
                            .get(field)
                            .and_then(Value::as_i64)
                            .and_then(|odds| i32::try_from(odds).ok())
                            .ok_or_else(|| format!("expected an integer in {field}"))
                    };
                    return Ok(BlockMapValue::Flammable {
                        ignite_odds: read_odds("ignite_odds")?,
                        burn_odds: read_odds("burn_odds")?,
                    });
                }
                "neoforge:oxidizables" => ("next_oxidation_stage", BlockMapValue::Oxidizable),
                "neoforge:waxables" => ("waxed", BlockMapValue::Waxable),
                _ => unreachable!("only the built-in block map types are registered here"),
            };
            // NeoForge reads this flag for its generated block transformers, which Sunrise does
            // not install. An invalid flag still makes the value undecodable.
            if !matches!(
                value.get("generate_block_transform"),
                None | Some(Value::Bool(_))
            ) {
                return Err("expected a boolean in generate_block_transform".into());
            }
            let target = value
                .get(field)
                .and_then(Value::as_str)
                .ok_or_else(|| format!("expected a block identifier in {field}"))?;
            let name = Identifier::parse(target)
                .map_err(|error| format!("{field}: {error}"))?
                .to_string();
            if block_id(&name).is_none() {
                return Err(format!("{field}: unknown block {name}"));
            }
            Ok(transform(name))
        },
        |_, _| Ok(()),
        |_, _, _, incoming| Ok(incoming.clone()),
        |_, _, _, _| Err("data map does not support partial removers".into()),
    );
    let blocks = maps.remove("minecraft:block")?;
    let block_maps = BlockMaps::from_values(blocks.into_values().flatten(), |name| {
        block_id(name).unwrap_or(u32::MAX)
    });
    (!block_maps.is_empty()).then(|| Arc::new(block_maps))
}

impl ServerGame<'_> {
    /// Install the open world's tables' block maps, replacing the last
    /// load's: none for a world without tables.
    pub(in crate::game::server_side) fn install_block_maps(&self) {
        let maps = self.server.world_tables.0.as_ref().and_then(|tables| {
            let overlay = sunrise_world::loot::PackOverlay {
                conditions: &tables.conditions,
                files: &tables.files,
                ..sunrise_world::loot::PackOverlay::default()
            };
            let table = &self.common.res.behaviours().table;
            let block_id = |block: &str| table.block_by_name(block).map(|id| u32::from(id.0));
            // Tables and the registry generation are installed together before this call.
            // Use the same membership boundary as their staged data-map load.
            let registries = WorldRegistries::new(
                self.common
                    .ui
                    .registries
                    .by_registry
                    .iter()
                    .map(|(registry, entries)| (registry.as_str(), entries.ids.as_slice())),
                self.common.mods.datapack_registries.as_deref(),
                &tables.items,
                &self.common.options.resources,
                &overlay,
            );
            load(&overlay, &registries, block_id)
        });
        self.common.res.behaviours().set_block_maps(maps);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn pack_files_give_the_block_maps() {
        let file = |value: serde_json::Value| vec![serde_json::to_vec(&value).unwrap()];
        let files = vec![
            (
                "data_maps/neoforge:block/flammable".to_owned(),
                file(json!({"values": {"minecraft:stone": {"ignite_odds": 5, "burn_odds": 20}}})),
            ),
            (
                "data_maps/neoforge:block/waxables".to_owned(),
                file(json!({"values": {
                    "test:tin": {"waxed": "test:waxed_tin"},
                    "minecraft:stone": {
                        "waxed": "test:waxed_tin",
                        "neoforge:conditions": [{"type": "neoforge:never"}]
                    }
                }})),
            ),
        ];
        let overlay = sunrise_world::loot::PackOverlay {
            files: &files,
            ..sunrise_world::loot::PackOverlay::default()
        };
        let ids = ["minecraft:stone", "test:tin", "test:waxed_tin"].map(String::from);
        let items = sunrise_items::items::Items::empty();
        let registries = WorldRegistries::new(
            [("minecraft:block", ids.as_slice())],
            None,
            &items,
            std::path::Path::new(""),
            &overlay,
        );
        let block_id = |name: &str| ids.iter().position(|id| id == name).map(|i| i as u32);
        let block_maps = load(&overlay, &registries, block_id).unwrap();
        assert_eq!(block_maps.flammable("minecraft:stone"), Some((5, 20)));
        assert_eq!(block_maps.waxed("test:tin"), Some("test:waxed_tin"));
        assert_eq!(block_maps.waxed("minecraft:stone"), None);
        assert_eq!(block_maps.next_oxidation("test:tin"), None);
        let none = load(
            &sunrise_world::loot::PackOverlay::NONE,
            &registries,
            block_id,
        );
        assert!(none.is_none());
    }

    #[test]
    fn an_undecodable_block_value_skips_the_entire_layer_even_with_replace() {
        let layers = |values: &[serde_json::Value]| {
            values
                .iter()
                .map(|value| serde_json::to_vec(value).unwrap())
                .collect::<Vec<_>>()
        };
        let files = vec![
            (
                "data_maps/neoforge:block/flammable".to_owned(),
                layers(&[
                    json!({"values": {"minecraft:stone": {"ignite_odds": 5, "burn_odds": 20}}}),
                    json!({"values": {"minecraft:stone": null}}),
                ]),
            ),
            (
                "data_maps/neoforge:block/oxidizables".to_owned(),
                layers(&[
                    json!({"values": {"test:tin": {"next_oxidation_stage": "test:weathered_tin"}}}),
                    json!({"replace": true, "values": {"test:tin": {
                        "next_oxidation_stage": "test:waxed_tin",
                        "generate_block_transform": "true"
                    }}}),
                ]),
            ),
            (
                "data_maps/neoforge:block/waxables".to_owned(),
                layers(&[
                    json!({"values": {"test:tin": {
                        "waxed": "test:waxed_tin", "generate_block_transform": false
                    }}}),
                    json!({"values": {"test:tin": {}}}),
                    json!({"replace": true, "values": {
                        "minecraft:stone": {"waxed": "test:waxed_tin"},
                        "test:tin": {"waxed": "test:not_registered"}
                    }}),
                ]),
            ),
        ];
        let overlay = sunrise_world::loot::PackOverlay {
            files: &files,
            ..sunrise_world::loot::PackOverlay::default()
        };
        let ids = [
            "minecraft:stone",
            "test:tin",
            "test:weathered_tin",
            "test:waxed_tin",
        ]
        .map(String::from);
        let items = sunrise_items::items::Items::empty();
        let registries = WorldRegistries::new(
            [("minecraft:block", ids.as_slice())],
            None,
            &items,
            std::path::Path::new(""),
            &overlay,
        );
        let block_id = |name: &str| ids.iter().position(|id| id == name).map(|i| i as u32);
        let block_maps = load(&overlay, &registries, block_id).unwrap();
        assert_eq!(block_maps.flammable("minecraft:stone"), Some((5, 20)));
        assert_eq!(block_maps.waxed("minecraft:stone"), None);
        assert_eq!(
            block_maps.next_oxidation("test:tin"),
            Some("test:weathered_tin")
        );
        assert_eq!(block_maps.waxed("test:tin"), Some("test:waxed_tin"));
    }

    #[test]
    fn transformables_and_appenders_merge_partially_remove_enforce_no_tags_and_apply() {
        use sunrise_sim::block_transform::{Around, Transformers};
        use sunrise_world::chunk::BlockState;
        use sunrise_world::registry::BlockRegistry;

        let names = [
            "minecraft:air",
            "minecraft:andesite",
            "minecraft:polished_andesite",
            "minecraft:cobblestone",
            "minecraft:quartz_block",
            "minecraft:chiseled_quartz_block",
        ];
        let registry = BlockRegistry::from_nbt(&Tag::compound([
            ("DataVersion", Tag::Int(sunrise_core::version::DATA_VERSION)),
            (
                "states",
                Tag::List(
                    names
                        .iter()
                        .map(|name| Tag::compound([("id", Tag::String((*name).into()))]))
                        .collect(),
                ),
            ),
            (
                "defaults",
                Tag::compound(
                    names
                        .iter()
                        .enumerate()
                        .map(|(i, name)| (*name, Tag::Int(i as i32))),
                ),
            ),
        ]))
        .unwrap();

        let rule = |from: &str, to: &str, sound: &str| {
            json!({
                "block_state_provider": {
                    "type": "minecraft:rule_based",
                    "rules": [{
                        "if_true": {
                            "type": "minecraft:matching_blocks",
                            "blocks": from
                        },
                        "then": { "id": to }
                    }]
                },
                "sound": sound
            })
        };

        let layers = |values: &[serde_json::Value]| {
            values
                .iter()
                .map(|value| serde_json::to_vec(value).unwrap())
                .collect::<Vec<_>>()
        };
        let files = vec![
            (
                "data_maps/neoforge:block/transformables".to_owned(),
                layers(&[
                    // Layer 1: single-form axe transform + disallowed #tag entry (must be ignored).
                    json!({
                        "values": {
                            "minecraft:andesite": {
                                "transformer": "minecraft:axe",
                                "transform_data": rule(
                                    "minecraft:andesite",
                                    "minecraft:polished_andesite",
                                    "minecraft:item.axe.strip"
                                )
                            },
                            "#test:stones": {
                                "transformer": "minecraft:axe",
                                "transform_data": rule(
                                    "minecraft:quartz_block",
                                    "minecraft:cobblestone",
                                    "minecraft:item.axe.strip"
                                )
                            }
                        }
                    }),
                    // Layer 2: merge a shovel transform onto minecraft:andesite (keeps axe too).
                    json!({
                        "values": {
                            "minecraft:andesite": {
                                "transformers": {
                                    "minecraft:shovel": rule(
                                        "minecraft:andesite",
                                        "minecraft:cobblestone",
                                        "minecraft:item.shovel.flatten"
                                    )
                                }
                            }
                        }
                    }),
                    // Layer 3: partially remove only minecraft:shovel from minecraft:andesite.
                    json!({
                        "values": {},
                        "remove": {
                            "minecraft:andesite": "minecraft:shovel"
                        }
                    }),
                ]),
            ),
            (
                "data_maps/neoforge:block_transformer/block_transform_appenders".to_owned(),
                layers(&[
                    json!({
                        "values": {
                            "minecraft:shovel": [rule(
                                "minecraft:quartz_block",
                                "minecraft:chiseled_quartz_block",
                                "minecraft:item.armor.equip_gold"
                            )]
                        }
                    }),
                    json!({
                        "values": {
                            "minecraft:shovel": [rule(
                                "minecraft:andesite",
                                "minecraft:cobblestone",
                                "minecraft:item.shovel.flatten"
                            )]
                        }
                    }),
                ]),
            ),
        ];
        let overlay = sunrise_world::loot::PackOverlay {
            files: &files,
            ..sunrise_world::loot::PackOverlay::default()
        };
        let blocks = [
            "minecraft:andesite",
            "minecraft:polished_andesite",
            "minecraft:cobblestone",
            "minecraft:quartz_block",
            "minecraft:chiseled_quartz_block",
        ]
        .map(String::from);
        let transformers_ids =
            ["minecraft:axe", "minecraft:shovel", "minecraft:hoe"].map(String::from);
        let mut items = sunrise_items::items::Items::empty();
        items.block_tags.insert(
            "test:stones".into(),
            ["minecraft:quartz_block".to_owned()].into_iter().collect(),
        );
        let registries = WorldRegistries::new(
            [
                ("minecraft:block", blocks.as_slice()),
                ("minecraft:block_transformer", transformers_ids.as_slice()),
            ],
            None,
            &items,
            std::path::Path::new(""),
            &overlay,
        );

        let loaded = load_transform_maps(&overlay, &registries, &registry);

        struct SingleBlock(&'static str);
        impl Around for SingleBlock {
            fn state(&self, _offset: [i32; 3]) -> BlockState {
                BlockState {
                    name: self.0.to_owned(),
                    properties: Default::default(),
                }
            }
        }

        let transformers = Transformers::default().with_extra(loaded.extra_transformers);
        // Axe on andesite -> polished_andesite (kept after shovel partial removal).
        let (_, t, state) = transformers
            .apply(
                &items,
                &registry,
                "minecraft:axe",
                "up",
                &SingleBlock("minecraft:andesite"),
            )
            .expect("axe transformable applies");
        assert_eq!(state.name, "minecraft:polished_andesite");
        assert_eq!(t.sound, "minecraft:item.axe.strip");

        // Axe on quartz_block -> None (#test:stones tag entry was ignored).
        assert!(
            transformers
                .apply(
                    &items,
                    &registry,
                    "minecraft:axe",
                    "up",
                    &SingleBlock("minecraft:quartz_block"),
                )
                .is_none()
        );

        // Shovel appenders concatenated both layers: quartz_block -> chiseled_quartz_block
        // and andesite -> cobblestone.
        let (_, t, state) = transformers
            .apply(
                &items,
                &registry,
                "minecraft:shovel",
                "up",
                &SingleBlock("minecraft:quartz_block"),
            )
            .expect("first shovel appender layer applies");
        assert_eq!(state.name, "minecraft:chiseled_quartz_block");
        assert_eq!(t.sound, "minecraft:item.armor.equip_gold");

        let (_, _, state) = transformers
            .apply(
                &items,
                &registry,
                "minecraft:shovel",
                "up",
                &SingleBlock("minecraft:andesite"),
            )
            .expect("second shovel appender layer applies");
        assert_eq!(state.name, "minecraft:cobblestone");
    }
}
