//! Fabric API's `fabric:load_conditions` (fabric-resource-conditions-api-v1 at 26.3): the
//! expected answers follow `ResourceConditionsImpl` and `DefaultResourceConditionTypes`.
use super::*;
use serde_json::json;

fn fabric(condition: Value) -> Value {
    json!({ FABRIC_KEY: condition })
}

#[test]
fn the_builtin_fabric_conditions_evaluate() {
    let load = LoadConditions::new(["create"]);
    for (condition, expected) in [
        (json!({"condition": "fabric:true"}), true),
        (json!({"condition": "fabric:false"}), false),
        (
            json!({"condition": "fabric:not", "value": {"condition": "fabric:false"}}),
            true,
        ),
        (
            json!({"condition": "fabric:and", "values": [
            {"condition": "fabric:true"}, {"condition": "fabric:false"}]}),
            false,
        ),
        (
            json!({"condition": "fabric:or", "values": [
            {"condition": "fabric:false"}, {"condition": "fabric:true"}]}),
            true,
        ),
        (
            json!({"condition": "fabric:all_mods_loaded", "values": ["create", "minecraft"]}),
            true,
        ),
        (
            json!({"condition": "fabric:all_mods_loaded", "values": ["create", "ic2"]}),
            false,
        ),
        (
            json!({"condition": "fabric:any_mods_loaded", "values": ["ic2", "create"]}),
            true,
        ),
        // `modsLoaded` over no ids answers its `and`.
        (
            json!({"condition": "fabric:all_mods_loaded", "values": []}),
            true,
        ),
        (
            json!({"condition": "fabric:any_mods_loaded", "values": []}),
            false,
        ),
        // A list is `ResourceConditions.and` of its conditions.
        (
            json!([{"condition": "fabric:true"}, {"condition": "fabric:false"}]),
            false,
        ),
        (json!([]), true),
    ] {
        assert_eq!(
            load.allows(&fabric(condition.clone())),
            expected,
            "{condition}"
        );
    }
}

#[test]
fn registry_tag_and_feature_conditions_read_the_load() {
    let load = LoadConditions::new([])
        .with_registries([("minecraft:item", ["minecraft:stone"])])
        .with_tags([(
            "minecraft:item",
            [("c:ingots/tin", true), ("c:ingots/silver", false)],
        )])
        .with_features([]);
    for (condition, expected) in [
        // `registry` defaults to items.
        (
            json!({"condition": "fabric:registry_contains", "values": ["stone"]}),
            true,
        ),
        (
            json!({"condition": "fabric:registry_contains", "values": ["stone", "test:x"]}),
            false,
        ),
        (
            json!({"condition": "fabric:registry_contains",
            "registry": "minecraft:worldgen/biome", "values": ["plains"]}),
            false,
        ),
        (
            json!({"condition": "fabric:registry_contains",
            "registry": "minecraft:worldgen/biome", "values": []}),
            true,
        ),
        (
            json!({"condition": "fabric:tags_populated", "values": ["c:ingots/tin"]}),
            true,
        ),
        (
            json!({"condition": "fabric:tags_populated", "values": ["c:ingots/tin", "c:gems"]}),
            false,
        ),
        // `tagsPopulated` asks whether the tag is bound (`HolderGetter.get` is present), not
        // whether it holds anything: a tag whose entries are all optional and absent counts.
        (
            json!({"condition": "fabric:tags_populated", "values": ["c:ingots/silver"]}),
            true,
        ),
        // `registry` is `fieldOf("registry").orElse(ITEM)`: an invalid id reads items.
        (
            json!({"condition": "fabric:tags_populated", "registry": "Bad Id",
            "values": ["c:ingots/tin"]}),
            true,
        ),
        (
            json!({"condition": "fabric:registry_contains", "registry": "Bad Id",
            "values": ["stone"]}),
            true,
        ),
        // A false answer only the fallback gives: a registry that failed to decode would drop the
        // whole key and load the file.
        (
            json!({"condition": "fabric:tags_populated", "registry": "Bad Id",
            "values": ["c:gems"]}),
            false,
        ),
        (
            json!({"condition": "fabric:registry_contains", "registry": "Bad Id",
            "values": ["test:x"]}),
            false,
        ),
        (
            json!({"condition": "fabric:features_enabled", "features": ["minecraft:vanilla"]}),
            true,
        ),
        (
            json!({"condition": "fabric:features_enabled", "features": ["trade_rebalance"]}),
            false,
        ),
        // An unknown flag fails the test (`featuresEnabled`), it is no decode error.
        (
            json!({"condition": "fabric:features_enabled", "features": ["test:typo"]}),
            false,
        ),
    ] {
        assert_eq!(
            load.allows_file_isolated(&fabric(condition.clone()), "f.json"),
            Ok(expected),
            "{condition}"
        );
    }
}

/// Each loader reads only its own key, so a file carrying both loads only when both hold.
#[test]
fn a_file_with_both_keys_needs_both() {
    let load = LoadConditions::new([]);
    let both = |neoforge: &str, fabric: &str| json!({NEOFORGE_KEY: [{"type": neoforge}], FABRIC_KEY: {"condition": fabric}});
    assert!(load.allows(&both("neoforge:always", "fabric:true")));
    assert!(!load.allows(&both("neoforge:always", "fabric:false")));
    assert!(!load.allows(&both("neoforge:never", "fabric:true")));
}

/// `applyResourceConditions` logs a Fabric key that does not parse and loads the file: no
/// omission and no registry diagnostic, unlike NeoForge's key.
#[test]
fn a_fabric_key_that_does_not_decode_loads_the_file() {
    let load = LoadConditions::new([]).with_lookup([("test:flag", |_: &Value| Ok(false))]);
    for broken in [
        json!({"condition": "fabric:not_a_condition"}),
        json!({"values": []}),
        json!({"condition": "fabric:and", "values": [{"condition": "fabric:false"}, 3]}),
    ] {
        assert_eq!(
            load.allows_file_isolated(&fabric(broken.clone()), "f.json"),
            Ok(true),
            "{broken}"
        );
        assert!(
            load.allows_file(&fabric(broken.clone()), "f.json"),
            "{broken}"
        );
    }
    assert_eq!(load.check_errors(), Ok(()));
}

/// A registered handler answers a Fabric condition named by its id, and reads the arguments
/// without the `condition` key, as it reads a NeoForge one's without `type`.
#[test]
fn a_registered_handler_answers_a_fabric_condition() {
    let load = LoadConditions::new([]).with_lookup([("testmod:flag", |args: &Value| {
        assert!(args.get("condition").is_none() && args.get("type").is_none());
        args["value"]
            .as_bool()
            .ok_or_else(|| "value must be a boolean".to_owned())
    })]);
    let flag = |value| fabric(json!({"condition": "testmod:flag", "value": value}));
    assert!(load.allows(&flag(json!(true))));
    assert!(!load.allows(&flag(json!(false))));
    let neoforge = json!({NEOFORGE_KEY: [{"type": "testmod:flag", "value": true}]});
    assert!(load.allows(&neoforge));
}

/// Fabric reads its key on whole files only: a loot pool's own Fabric key is ordinary data.
#[test]
fn a_loot_pools_fabric_key_is_not_evaluated() {
    let pool = json!({"rolls": 1, FABRIC_KEY: {"condition": "fabric:false"}});
    let mut table = json!({"pools": [pool.clone()]});
    assert!(LoadConditions::new([]).admit_loot_table(&mut table, "a:b"));
    assert_eq!(table["pools"], json!([pool]));
}

#[test]
fn the_payload_leaves_out_the_fabric_key() {
    let file = json!({FABRIC_KEY: {"condition": "fabric:true"}, "feature": "a:x"});
    assert_eq!(into_payload(file), json!({"feature": "a:x"}));
}

/// One bound tag that holds nothing reads differently to the two loaders: empty to
/// `neoforge:tag_empty` (`IContext.getTag` is an empty collection), present to
/// `fabric:tags_populated` (`HolderGetter.get` finds the bound tag).
#[test]
fn a_bound_empty_tag_is_empty_to_neoforge_and_populated_to_fabric() {
    let load = LoadConditions::new([]).with_tags([("minecraft:item", [("c:ingots/tin", false)])]);
    let neoforge = json!({NEOFORGE_KEY: [{"type": "neoforge:tag_empty", "tag": "c:ingots/tin"}]});
    let fabric = fabric(json!({"condition": "fabric:tags_populated", "values": ["c:ingots/tin"]}));
    assert!(load.allows(&neoforge));
    assert!(load.allows(&fabric));
    let unbound = LoadConditions::new([]).with_tags([("minecraft:item", Vec::new())]);
    assert!(
        !unbound.allows(&fabric),
        "a tag no file defines is not populated"
    );
}

/// The readers that only ask whether a file is conditional (biome admission, the LAN biome
/// table, the unchanged-definition shortcut) see either loader's key.
#[test]
fn either_key_makes_a_file_conditional() {
    assert!(has_conditions(&json!({NEOFORGE_KEY: [], "a": 1})));
    assert!(has_conditions(&fabric(json!({"condition": "fabric:true"}))));
    assert!(!has_conditions(&json!({"a": 1, "neoforge:value": {}})));
}
