use super::*;
use sunrise_world::conditions::LoadConditions;
/// Only 26.3's canonical worldgen feature folder is admitted.
#[test]
fn worldgen_entry_reads_the_feature_folder() {
    assert_eq!(
        worldgen_entry("data/zinc/worldgen/feature/ore_zinc.json"),
        Some(("worldgen/features.json", "zinc:ore_zinc".to_owned()))
    );
    for file in [
        "data/zinc/worldgen/configured_feature/ore_zinc.json",
        "data/zinc/worldgen/nope/ore_zinc.json",
        "data/zinc/data_maps/worldgen/biome/zinc_patch.json",
        "data/zinc/custom/worldgen/feature/ore_zinc.json",
        "data/minecraft/worldgen/biome/zinc_patch.json",
    ] {
        assert_eq!(worldgen_entry(file), None, "{file}");
    }
}

#[test]
#[ignore = "daily"]
fn registered_conditions_select_content_root_tags_and_structure_modifiers() {
    let scratch = tempfile::tempdir().unwrap();
    let Some(base) = prepared() else {
        return;
    };
    let inventory =
        sunrise_mod_package::enabled::EnabledMods::new([("testmod".into(), mod_api::Side::Both)]);
    let package = folder_mod(
        &scratch.path().join("mod"),
        &[
            ("sunrise-mod.toml", &manifest("testmod", "")),
            (
                "data/testmod/tags/block/flag.json",
                r#"{"neoforge:conditions":[{"type":"testmod:flag","value":true}],"values":["minecraft:stone"]}"#,
            ),
            (
                "data/testmod/neoforge/structure_modifier/rabbits.json",
                r#"{"neoforge:conditions":[{"type":"testmod:flag","value":true}],"type":"neoforge:add_spawns","structures":"minecraft:swamp_hut","spawners":{"type":"minecraft:rabbit","weight":3,"minCount":1,"maxCount":1}}"#,
            ),
        ],
    );
    let mut previous: Option<PathBuf> = None;
    for enabled in [true, false] {
        let conditions = LoadConditions::new(["testmod"])
            .with_lookup([("testmod:flag", move |args: &serde_json::Value| {
                Ok(enabled && args["value"].as_bool().unwrap())
            })]);
        let out = super::super::resource_root_with_conditions(
            &base,
            Path::new(""),
            std::slice::from_ref(&package),
            &scratch.path().join("cache"),
            &inventory,
            &conditions,
        )
        .unwrap();
        let tag_file = out.join("data/testmod/tags/block/flag.json");
        assert!(tag_file.exists());
        let value: serde_json::Value =
            serde_json::from_slice(&fs::read(tag_file).unwrap()).unwrap();
        assert_eq!(value["values"], serde_json::json!(["minecraft:stone"]));
        assert!(value.get("neoforge:conditions").is_none());
        let spawns = crate::mobs::spawn_selection::StructureSpawns::load(&out);
        let rabbits = spawns
            .get("minecraft:swamp_hut", "creature")
            .is_some_and(|entry| {
                entry
                    .spawns
                    .iter()
                    .any(|spawn| spawn.entity == "minecraft:rabbit")
            });
        assert_eq!(rabbits, enabled);
        assert_eq!(conditions.check_errors(), Ok(()));
        if let Some(old) = &previous {
            assert_ne!(old, &out);
            assert!(
                old.join("data/testmod/tags/block/flag.json").exists(),
                "old usable root was overwritten"
            );
        }
        previous = Some(out);
    }
}

#[test]
fn conditional_base_tag_does_not_gate_later_accepted_layers() {
    let base = r#"{"neoforge:conditions":[{"type":"neoforge:mod_loaded","modid":"code_only"}],"values":["minecraft:stone"]}"#;
    let more = [J::Str("minecraft:dirt".into())];
    for enabled in [false, true] {
        let merged: serde_json::Value = serde_json::from_str(
            &merge_tag(
                Some(base),
                &[TagLayers {
                    values: more.as_slice(),
                    remove: &[],
                }],
                false,
                "conditional",
                &sunrise_world::conditions::LoadConditions::new(if enabled {
                    vec!["code_only"]
                } else {
                    vec![]
                }),
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(
            merged["values"],
            serde_json::json!(["minecraft:stone", "minecraft:dirt"])
        );
        assert!(merged.get("neoforge:conditions").is_none());
    }
}

#[test]
fn tags_merge_without_repeats() {
    let vanilla = r#"{"values": ["minecraft:stone"]}"#;
    let more = [
        J::Str("ruby:ruby_block".into()),
        J::Str("minecraft:stone".into()),
    ];
    let merged: serde_json::Value = serde_json::from_str(
        &merge_tag(
            Some(vanilla),
            &[TagLayers {
                values: more.as_slice(),
                remove: &[],
            }],
            false,
            "t",
            &sunrise_world::conditions::LoadConditions::vanilla(),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(
        merged["values"],
        serde_json::json!(["minecraft:stone", "ruby:ruby_block", "minecraft:stone"])
    );
}

#[test]
#[ignore = "daily"]
fn failed_condition_build_removes_generated_staging() {
    let Some(base) = prepared() else {
        return;
    };
    let scratch = tempfile::tempdir().unwrap();
    let package = folder_mod(
        &scratch.path().join("module"),
        &[
            ("sunrise-mod.toml", &manifest("testmod", "")),
            (
                "data/testmod/neoforge/structure_modifier/bad.json",
                r#"{"neoforge:conditions":[{"type":"testmod:flag","value":"bad"}],"values":["minecraft:stone"]}"#,
            ),
        ],
    );
    let conditions = LoadConditions::new([])
        .with_lookup([("testmod:flag", |_: &serde_json::Value| {
            Err("bad value".into())
        })]);
    let out = scratch.path().join("candidate");
    let error = super::super::build_with_conditions(
        &base,
        Path::new(""),
        &[package],
        &out,
        &sunrise_mod_package::enabled::EnabledMods::default(),
        &conditions,
    )
    .unwrap_err()
    .to_string();
    assert!(
        error.contains("data/testmod/neoforge/structure_modifier/bad.json"),
        "{error}"
    );
    assert!(!out.exists());
    assert!(!scratch.path().join("candidate-tables").exists());
}

#[test]
fn removals_follow_the_tag_file_and_a_replace_drops_them() {
    let vanilla =
        r#"{"values": ["minecraft:stone", "minecraft:dirt"], "remove": ["minecraft:sand"]}"#;
    let ids = |ids: &[&str]| -> Vec<J> { ids.iter().map(|i| J::Str((*i).into())).collect() };
    let merged: serde_json::Value = serde_json::from_str(
        &merge_tag(
            Some(vanilla),
            &[TagLayers {
                values: &ids(&["test:ruby"]),
                remove: &ids(&["minecraft:dirt"]),
            }],
            false,
            "t",
            &LoadConditions::vanilla(),
        )
        .unwrap(),
    )
    .unwrap();
    // The vanilla file's removals apply right after its values, then the
    // mods' adds and removals join `values` in load order, each removal
    // carried as `{"id", "remove": true}`.
    assert_eq!(
        merged["values"],
        serde_json::json!(["minecraft:stone", "minecraft:dirt",
            {"id": "minecraft:sand", "remove": true}, "test:ruby",
            {"id": "minecraft:dirt", "remove": true}])
    );
    assert_eq!(merged["remove"], serde_json::Value::Null);
    let file = "data/minecraft/tags/item/things.json";
    let mut merge = TagMerge::default();
    merge.add("a", file, false, true, &ids(&["a:x"]));
    if let Some((_, remove)) = merge.layers.last_mut() {
        remove.extend(ids(&["minecraft:dirt"]));
    }
    merge.add("b", file, true, true, &ids(&["b:y"]));
    assert!(merge.layers.iter().all(|(_, remove)| remove.is_empty()));
}

/// The order the mods' layers keep: a lower mod's removal, then a higher
/// mod's add of the same entry, ends with the entry in the merged file, and
/// the folder walks it back in.
#[test]
fn a_higher_mod_adds_back_what_a_lower_one_removed() {
    let ids = |ids: &[&str]| -> Vec<J> { ids.iter().map(|i| J::Str((*i).into())).collect() };
    let file = "data/minecraft/tags/item/things.json";
    let mut merge = TagMerge::default();
    merge.add("low", file, false, true, &ids(&["a:x"]));
    if let Some((_, remove)) = merge.layers.last_mut() {
        remove.extend(ids(&["minecraft:sand"]));
    }
    merge.add("high", file, false, true, &ids(&["minecraft:sand", "b:y"]));
    merge.add("last", file, false, true, &[]);
    if let Some((_, remove)) = merge.layers.last_mut() {
        remove.extend(ids(&["minecraft:stone"]));
    }
    merge.add("restore", file, false, true, &ids(&["minecraft:stone"]));
    merge.add("remove-again", file, false, true, &[]);
    if let Some((_, remove)) = merge.layers.last_mut() {
        remove.extend(ids(&["minecraft:stone"]));
    }
    let merged: serde_json::Value = serde_json::from_str(
        &merge_tag(
            Some(r#"{"values": ["minecraft:stone", "minecraft:sand"]}"#),
            &merge.layers(),
            false,
            "t",
            &LoadConditions::vanilla(),
        )
        .unwrap(),
    )
    .unwrap();
    // The fold (TagLayers::build) reads the entries in order: x, sand out,
    // sand back in, y.
    let layers = sunrise_world::tag_layers::TagLayers::from_file(merged).unwrap();
    assert_eq!(
        layers.build(|_| true, |_| None).unwrap(),
        ["a:x", "minecraft:sand", "b:y"]
    );
}

#[test]
#[ignore = "daily"]
fn content_root_tag_layers_ignore_failing_top_level_conditions() {
    let Some(base) = prepared() else { return };
    let scratch = tempfile::tempdir().unwrap();
    let file = "data/minecraft/tags/block/conditional_probe.json";
    let packages = [
        ("lower", r#"{"values":["minecraft:stone"]}"#),
        (
            "conditional",
            r#"{"neoforge:conditions":[{"type":"neoforge:never"}],"values":["minecraft:dirt"]}"#,
        ),
    ];
    let mods: Vec<_> = packages
        .into_iter()
        .map(|(id, tag)| {
            folder_mod(
                &scratch.path().join(id),
                &[("sunrise-mod.toml", &manifest(id, "")), (file, tag)],
            )
        })
        .collect();
    let root =
        resource_root_fixture(&base, Path::new(""), &mods, &scratch.path().join("cache")).unwrap();
    let merged: serde_json::Value =
        serde_json::from_slice(&fs::read(root.join(file)).unwrap()).unwrap();
    assert_eq!(
        merged["values"],
        serde_json::json!(["minecraft:stone", "minecraft:dirt"])
    );
}

#[test]
fn unknown_vanilla_tag_references_do_not_break_mod_tags() {
    use sunrise_world::worldgen::tags::TagDefinitions;

    let scratch = tempfile::tempdir().unwrap();
    let base = resources(scratch.path());
    let vanilla = r#"{"values":["minecraft:stone"]}"#;
    for name in TAG_BUNDLES {
        let bundle = BTreeMap::from([("minecraft:fixture/current", vanilla)]);
        fs::write(base.join(name), serde_json::to_vec(&bundle).unwrap()).unwrap();
    }
    for kind in ["block", "item"] {
        let path = |ns: &str, name: &str| format!("data/{ns}/tags/{kind}/{name}.json");
        let current = base.join(path("minecraft", "fixture/current"));
        fs::create_dir_all(current.parent().unwrap()).unwrap();
        fs::write(current, vanilla).unwrap();
        let combined = path("testmod", "combined");
        let m = folder_mod(
            &scratch.path().join(kind),
            &[
                ("sunrise-mod.toml", &manifest("testmod", "")),
                (
                    &combined,
                    r##"{"values":["#minecraft:fixture/removed", "#minecraft:fixture/current", "#testmod:local", "#minecraft:fixture/from_mod", {"id":"#minecraft:fixture/optional", "required":false}]}"##,
                ),
                (&path("testmod", "local"), r#"{"values":[]}"#),
                (&path("minecraft", "fixture/from_mod"), r#"{"values":[]}"#),
            ],
        );
        let out = scratch.path().join(format!("out-{kind}"));
        let warnings = build_fixture(&base, Path::new(""), &[m], &out).unwrap();
        if kind == "block" {
            for name in TAG_BUNDLES {
                let bundle: BTreeMap<String, String> =
                    serde_json::from_slice(&fs::read(out.join(name)).unwrap()).unwrap();
                let mut tags = TagDefinitions::default();
                for (id, text) in bundle {
                    tags.apply_json(&id, name, &text).unwrap();
                }
                let built = tags.build(|id, _| Some(id.to_owned())).unwrap();
                assert!(built.missing.is_empty(), "{name}: {:?}", built.missing);
                assert_eq!(built.tags["testmod:combined"], ["minecraft:stone"]);
            }
        }
        let merged: serde_json::Value =
            serde_json::from_slice(&fs::read(out.join(&combined)).unwrap()).unwrap();
        assert_eq!(
            merged["values"],
            serde_json::json!([
                "#minecraft:fixture/current",
                "#testmod:local",
                "#minecraft:fixture/from_mod"
            ])
        );
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert!(
            warnings[0].contains(&combined)
                && warnings[0].contains("testmod:combined")
                && warnings[0].contains("#minecraft:fixture/removed"),
            "{warnings:?}"
        );
    }
}

#[test]
fn no_vanilla_tag_folder_keeps_vanilla_references() {
    let scratch = tempfile::tempdir().unwrap();
    let base = resources(scratch.path());
    let mods = mod_in(
        scratch.path(),
        &tag_files(r##""#minecraft:fixture/unlisted""##),
    );
    let out = scratch.path().join("out");
    let warnings = build_fixture(&base, Path::new(""), &mods, &out).unwrap();
    let merged: serde_json::Value =
        serde_json::from_slice(&fs::read(out.join("data/testmod/tags/item/a.json")).unwrap())
            .unwrap();
    assert_eq!(
        merged["values"],
        serde_json::json!(["#minecraft:fixture/unlisted"])
    );
    assert!(warnings.is_empty(), "{warnings:?}");
}

#[test]
fn content_mod_worldgen_bundles_and_additions_evaluate_load_conditions() {
    let scratch = tempfile::tempdir().unwrap();
    let base = resources(scratch.path());
    fs::create_dir_all(base.join("worldgen")).unwrap();
    fs::write(base.join("worldgen/features.json"), "{}").unwrap();
    fs::write(
        base.join("worldgen/template_pools.json"),
        serde_json::json!({
            "minecraft:test_pool": r#"{"name":"minecraft:test_pool","fallback":"minecraft:empty","elements":[]}"#
        })
        .to_string(),
    )
    .unwrap();
    let package = folder_mod(
        &scratch.path().join("mod"),
        &[
            ("sunrise-mod.toml", &manifest("boiler", "")),
            (
                "data/boiler/worldgen/feature/gated_ore.json",
                r#"{"neoforge:conditions":[{"type":"neoforge:mod_loaded","modid":"chemlib"}],"type":"minecraft:no_op","config":{}}"#,
            ),
            (
                "data/boiler/worldgen/template_pool/pool_add.json",
                r#"{"neoforge:conditions":[{"type":"neoforge:mod_loaded","modid":"chemlib"}],"sunrise:add_to":"minecraft:test_pool","elements":[{"weight":1,"element":{"element_type":"minecraft:empty_pool_element"}}]}"#,
            ),
        ],
    );
    let inventory_on = sunrise_mod_package::enabled::EnabledMods::new([
        ("boiler".into(), mod_api::Side::Both),
        ("chemlib".into(), mod_api::Side::Both),
    ]);
    let out_on = scratch.path().join("out_on");
    super::super::build_with_conditions(
        &base,
        Path::new(""),
        std::slice::from_ref(&package),
        &out_on,
        &inventory_on,
        &LoadConditions::new(["boiler", "chemlib"]),
    )
    .unwrap();
    let features_on: serde_json::Value =
        serde_json::from_slice(&fs::read(out_on.join("worldgen/features.json")).unwrap()).unwrap();
    assert!(features_on.get("boiler:gated_ore").is_some());
    let pools_on: serde_json::Value =
        serde_json::from_slice(&fs::read(out_on.join("worldgen/template_pools.json")).unwrap())
            .unwrap();
    let pool_on: serde_json::Value =
        serde_json::from_str(pools_on["minecraft:test_pool"].as_str().unwrap()).unwrap();
    assert_eq!(pool_on["elements"].as_array().unwrap().len(), 1);

    let inventory_off =
        sunrise_mod_package::enabled::EnabledMods::new([("boiler".into(), mod_api::Side::Both)]);
    let out_off = scratch.path().join("out_off");
    super::super::build_with_conditions(
        &base,
        Path::new(""),
        std::slice::from_ref(&package),
        &out_off,
        &inventory_off,
        &LoadConditions::new(["boiler"]),
    )
    .unwrap();
    let features_off: serde_json::Value =
        serde_json::from_slice(&fs::read(out_off.join("worldgen/features.json")).unwrap()).unwrap();
    assert!(features_off.get("boiler:gated_ore").is_none());
    let pools_off: serde_json::Value =
        serde_json::from_slice(&fs::read(out_off.join("worldgen/template_pools.json")).unwrap())
            .unwrap();
    let pool_off: serde_json::Value =
        serde_json::from_str(pools_off["minecraft:test_pool"].as_str().unwrap()).unwrap();
    assert_eq!(pool_off["elements"].as_array().unwrap().len(), 0);

    // Malformed condition JSON fails atomically.
    let broken = folder_mod(
        &scratch.path().join("broken_mod"),
        &[
            ("sunrise-mod.toml", &manifest("broken", "")),
            (
                "data/broken/worldgen/feature/bad.json",
                r#"{"neoforge:conditions":[{"type":"unknown:bogus"}],"type":"minecraft:no_op","config":{}}"#,
            ),
        ],
    );
    let out_err = scratch.path().join("out_err");
    let err = super::super::build_with_conditions(
        &base,
        Path::new(""),
        std::slice::from_ref(&broken),
        &out_err,
        &sunrise_mod_package::enabled::EnabledMods::new([("broken".into(), mod_api::Side::Both)]),
        &LoadConditions::new(["broken"]),
    );
    assert!(
        err.is_err(),
        "unknown condition in content-mod worldgen must fail atomically"
    );
}

/// #6120: a mod's damage type named in vanilla's `minecraft:bypasses_armor` (Tinkers'
/// Construct's `tconstruct:bleeding`) joins the root beside the merged tag file, so the
/// root alone binds the tag with vanilla's types and the mod's, as `TagLoader` does
/// against the full registry: the server's catalog and the synchronized registry's names.
#[test]
fn a_mod_damage_type_in_a_vanilla_damage_tag_keeps_the_tag_bound() {
    use sunrise_world::{
        damage_source::{Catalog, resource_files, resource_tags},
        loot::PackOverlay,
    };
    let scratch = tempfile::tempdir().unwrap();
    let base = resources(scratch.path());
    for (file, text) in [
        (
            "data/minecraft/damage_type/starve.json",
            r#"{"message_id":"starve","scaling":"when_caused_by_living_non_player","exhaustion":0.0}"#,
        ),
        (
            "data/minecraft/damage_type/fall.json",
            r#"{"message_id":"fall","scaling":"when_caused_by_living_non_player","exhaustion":0.0,"death_message_type":"fall_variants"}"#,
        ),
        (
            "data/minecraft/tags/damage_type/bypasses_armor.json",
            r#"{"values":["minecraft:starve","minecraft:fall"]}"#,
        ),
    ] {
        let path = base.join(file);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }
    let mods = mod_in(
        scratch.path(),
        &[
            ("sunrise-mod.toml", manifest("testmod", "")),
            (
                "data/testmod/damage_type/bleeding.json",
                r#"{"message_id":"testmod.bleed","scaling":"never","exhaustion":0.1}"#.into(),
            ),
            (
                "data/minecraft/tags/damage_type/bypasses_armor.json",
                r#"{"values":["testmod:bleeding"]}"#.into(),
            ),
        ],
    );
    let out = scratch.path().join("out");
    let warnings = build_fixture(&base, Path::new(""), &mods, &out).unwrap();
    assert!(warnings.is_empty(), "{warnings:?}");
    let expected = ["minecraft:fall", "minecraft:starve", "testmod:bleeding"];

    // The server's damage registry read from the root alone (startup, structure mobs).
    let catalog = Catalog::default()
        .with_packs(&out, &PackOverlay::NONE)
        .unwrap();
    assert!(catalog.id("testmod:bleeding").is_some());
    let members: BTreeSet<_> = catalog.tag_members("minecraft:bypasses_armor").collect();
    assert_eq!(members, BTreeSet::from(expected));

    // The synchronized registry binds the tag against its own names (`lan/data.rs`).
    let names: BTreeMap<String, i32> = resource_files(&out, "damage_type")
        .unwrap()
        .into_keys()
        .zip(0..)
        .collect();
    let tags = resource_tags(&out, &PackOverlay::NONE, |id, _| names.get(id).copied()).unwrap();
    let members: BTreeSet<_> = tags["minecraft:bypasses_armor"]
        .iter()
        .map(|id| names.iter().find(|(_, n)| *n == id).unwrap().0.as_str())
        .collect();
    assert_eq!(members, BTreeSet::from(expected));
}

/// A root built with the owning load's conditions, from a base holding vanilla's
/// `starve` and `fall` and the given mods, each enabled beside the extra mod ids.
fn damage_root(scratch: &Path, mods: &[ContentMod], also_loaded: &[&str], out: &str) -> PathBuf {
    let base = scratch.join("resources");
    if !base.exists() {
        let base = resources(scratch);
        for (file, text) in [
            (
                "data/minecraft/damage_type/starve.json",
                r#"{"message_id":"starve","scaling":"when_caused_by_living_non_player","exhaustion":0.0}"#,
            ),
            (
                "data/minecraft/damage_type/fall.json",
                r#"{"message_id":"fall","scaling":"when_caused_by_living_non_player","exhaustion":0.0,"death_message_type":"fall_variants"}"#,
            ),
            (
                "data/minecraft/tags/damage_type/bypasses_armor.json",
                r#"{"values":["minecraft:starve","minecraft:fall"]}"#,
            ),
        ] {
            let path = base.join(file);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, text).unwrap();
        }
    }
    let ids: Vec<&str> = mods
        .iter()
        .map(|m| m.id.as_str())
        .chain(also_loaded.iter().copied())
        .collect();
    let inventory = sunrise_mod_package::enabled::EnabledMods::new(
        ids.iter().map(|id| ((*id).to_owned(), mod_api::Side::Both)),
    );
    let out = scratch.join(out);
    super::super::build_with_conditions(
        &base,
        Path::new(""),
        mods,
        &out,
        &inventory,
        &LoadConditions::new(ids),
    )
    .unwrap();
    out
}

/// The server's damage registry read from a root alone, as startup and the structure
/// mobs' tables do: no pack overlay, the vanilla-only inventory.
fn root_only_catalog(out: &Path) -> sunrise_world::damage_source::Catalog {
    sunrise_world::damage_source::Catalog::default()
        .with_packs(out, &sunrise_world::loot::PackOverlay::NONE)
        .unwrap()
}

/// #6120: the root's reader has the vanilla-only inventory, so a definition gated on
/// its own mod (or on another) is decided while the root is built: present when the
/// owning load's conditions hold, absent when they fail, and a negated one the other way.
#[test]
fn a_gated_mod_damage_type_is_decided_once_for_the_root_alone_readers() {
    let scratch = tempfile::tempdir().unwrap();
    for loaded in [true, false] {
        let dir = scratch.path().join(format!("loaded_{loaded}"));
        let tag = if loaded {
            "testmod:gated"
        } else {
            "testmod:negated"
        };
        let mods = vec![folder_mod(
            &dir.join("mod"),
            &[
                ("sunrise-mod.toml", &manifest("testmod", "")),
                (
                    "data/testmod/damage_type/gated.json",
                    r#"{"neoforge:conditions":[{"type":"neoforge:mod_loaded","modid":"chemlib"}],"message_id":"testmod.gated","scaling":"never","exhaustion":0.1}"#,
                ),
                (
                    "data/testmod/damage_type/negated.json",
                    r#"{"neoforge:conditions":[{"type":"neoforge:not","value":{"type":"neoforge:mod_loaded","modid":"chemlib"}}],"message_id":"testmod.negated","scaling":"never","exhaustion":0.1}"#,
                ),
                (
                    "data/minecraft/tags/damage_type/bypasses_armor.json",
                    &format!(r#"{{"values":["{tag}"]}}"#),
                ),
            ],
        )];
        let out = damage_root(&dir, &mods, if loaded { &["chemlib"] } else { &[] }, "out");
        let catalog = root_only_catalog(&out);
        assert_eq!(catalog.id("testmod:gated").is_some(), loaded, "{loaded}");
        assert_eq!(catalog.id("testmod:negated").is_some(), !loaded, "{loaded}");
        let members: BTreeSet<_> = catalog.tag_members("minecraft:bypasses_armor").collect();
        assert_eq!(
            members,
            BTreeSet::from(["minecraft:fall", "minecraft:starve", tag]),
            "{loaded}"
        );
    }
}

/// #6120: a mod damage type file that isn't JSON is left out of the root with a warning
/// naming the mod and the file, so the root's startup readers aren't blocked by it; it
/// stays in the mod's own pack, where the world's registry load reports it as vanilla
/// does. The mod's valid type still joins the root and the vanilla tag naming it holds.
#[test]
fn a_malformed_mod_damage_type_is_left_out_of_the_root_with_a_warning() {
    let scratch = tempfile::tempdir().unwrap();
    let base = resources(scratch.path());
    for (file, text) in [
        (
            "data/minecraft/damage_type/starve.json",
            r#"{"message_id":"starve","scaling":"when_caused_by_living_non_player","exhaustion":0.0}"#,
        ),
        (
            "data/minecraft/tags/damage_type/bypasses_armor.json",
            r#"{"values":["minecraft:starve"]}"#,
        ),
    ] {
        let path = base.join(file);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }
    let broken = "data/testmod/damage_type/broken.json";
    let mods = mod_in(
        scratch.path(),
        &[
            ("sunrise-mod.toml", manifest("testmod", "")),
            (broken, r#"{"message_id":"testmod.broken","#.into()),
            (
                "data/testmod/damage_type/bleeding.json",
                r#"{"message_id":"testmod.bleed","scaling":"never","exhaustion":0.1}"#.into(),
            ),
            (
                "data/minecraft/tags/damage_type/bypasses_armor.json",
                r#"{"values":["testmod:bleeding"]}"#.into(),
            ),
        ],
    );
    let out = scratch.path().join("out");
    let warnings = build_fixture(&base, Path::new(""), &mods, &out).unwrap();
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(
        warnings[0].starts_with(&format!("mod testmod: {broken}: ")),
        "{warnings:?}"
    );
    assert!(!out.join(broken).exists());
    assert!(out.join("data/testmod/damage_type/bleeding.json").exists());

    let catalog = root_only_catalog(&out);
    assert!(catalog.id("testmod:bleeding").is_some());
    assert!(catalog.id("testmod:broken").is_none());
    let members: BTreeSet<_> = catalog.tag_members("minecraft:bypasses_armor").collect();
    assert_eq!(
        members,
        BTreeSet::from(["minecraft:starve", "testmod:bleeding"])
    );
}
