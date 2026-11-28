use super::*;

#[test]
fn prepared_health_load_validates_float_schema() {
    let root = tempfile::tempdir().unwrap();
    let types = ["minecraft:fall", "minecraft:stalagmite", "minecraft:starve"].map(|id| {
        serde_json::json!({
            "id": id,
            "message_id": "test",
            "exhaustion": "00000000",
            "scaling": "never",
            "effects": "hurt",
            "death_message_type": "default",
        })
    });
    fs::write(
        root.path().join("damage-catalog.json"),
        serde_json::json!({
            "data_version": sunrise_core::version::DATA_VERSION,
            "types": types,
        })
        .to_string(),
    )
    .unwrap();
    let schema = root.path().join("player-float-ids.json");
    for (absorption, message) in [
        (7, "duplicate synchronized field ID 7"),
        (255, "reserved synchronized field ID 255"),
    ] {
        fs::write(
            &schema,
            format!(r#"{{"health":7,"absorption":{absorption}}}"#),
        )
        .unwrap();
        let error = HealthResources::load(root.path())
            .err()
            .unwrap()
            .to_string();
        assert_eq!(error, format!("{}: {message}", schema.display()));
    }
    fs::write(&schema, r#"{"health":7,"absorption":254}"#).unwrap();
    let resources = HealthResources::load(root.path()).unwrap();
    let mut player = PlayerHealth::initial(&resources, 36.0).unwrap();
    assert_eq!((player.health(), player.absorption()), (36.0, 0.0));
    let exposes = |values: Option<Vec<sunrise_world::synced_values::DataValue<f32>>>| {
        values.map(|values| values.iter().map(|v| (v.id, v.value)).collect::<Vec<_>>())
    };
    assert_eq!(exposes(player.data.non_default()), Some(vec![(7, 36.0)]));
    assert_eq!(exposes(player.data.take_dirty()), Some(vec![(7, 36.0)]));
}

#[test]
#[ignore = "daily"]
fn datapack_damage_new_world_drops_a_removed_resource_root_definition() {
    use sunrise_world::loot::PackOverlay;
    let prepared = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../.local/sunrise-client")
        .join(sunrise_core::VANILLA_VERSION);
    let root = std::env::temp_dir().join(format!("sunrise-damage-root-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(root.join("data/demo/damage_type")).unwrap();
    for file in [
        "damage-catalog.json",
        "damage-tags.json",
        "player-float-ids.json",
    ] {
        fs::copy(prepared.join(file), root.join(file)).unwrap();
    }
    let definition = root.join("data/demo/damage_type/installed.json");
    fs::write(
        &definition,
        r#"{"message_id":"installed","scaling":"never","exhaustion":0.25}"#,
    )
    .unwrap();
    let cached_resources = HealthResources::load(&root).unwrap();
    let first_world = cached_resources
        .with_packs(&root, &PackOverlay::NONE, false)
        .unwrap();
    let custom = first_world.damage.id("demo:installed").unwrap();
    let vanilla = first_world.damage.id("minecraft:fall").unwrap();
    fs::remove_file(definition).unwrap();
    let reloaded = first_world
        .with_packs(&root, &PackOverlay::NONE, true)
        .unwrap();
    assert_eq!(reloaded.damage.id("demo:installed"), Some(custom));
    let next_world = cached_resources
        .with_packs(&root, &PackOverlay::NONE, false)
        .unwrap();
    assert!(next_world.damage.id("demo:installed").is_none());
    assert_eq!(next_world.damage.id("minecraft:fall"), Some(vanilla));
    assert_eq!(first_world.damage.id("demo:installed"), Some(custom));
    fs::remove_dir_all(root).unwrap();
}

#[test]
#[ignore = "daily"]
fn datapack_damage_initial_snapshot_uses_the_prepared_worlds_features() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../.local/sunrise-client")
        .join(sunrise_core::VANILLA_VERSION);
    let world =
        std::env::temp_dir().join(format!("sunrise-damage-features-{}", std::process::id()));
    let pack = world.join("datapacks/feature_damage");
    let _ = fs::remove_dir_all(&world);
    fs::create_dir_all(pack.join("data/demo/damage_type")).unwrap();
    fs::write(
        pack.join("pack.mcmeta"),
        r#"{"pack":{"description":"test","pack_format":121},"features":{"enabled":["minecraft:trade_rebalance"]}}"#,
    )
    .unwrap();
    fs::write(
        pack.join("data/demo/damage_type/heat.json"),
        r#"{"message_id":"heat","scaling":"never","exhaustion":0.1}"#,
    )
    .unwrap();
    fs::write(
        world.join("datapacks.json"),
        r#"{"enabled":["file/feature_damage"],"disabled":[]}"#,
    )
    .unwrap();
    let packs = crate::datapack::discover_with_features(&world, &root);
    let snapshot = |features: &[String]| {
        let enabled = crate::datapack::enabled_ids(&world, &packs, features);
        let selected: Vec<_> = enabled
            .iter()
            .filter_map(|id| packs.iter().find(|p| &p.id == id))
            .collect();
        let store = crate::datapack::DataStore::load(&crate::datapack::DataPacks::new(&selected));
        let files: Vec<_> = store.files.into_iter().collect();
        sunrise_world::damage_source::Catalog::load(
            &root,
            &sunrise_world::loot::PackOverlay::new(&files),
        )
        .unwrap()
    };
    let stale = snapshot(&["minecraft:vanilla".into()]);
    assert!(stale.id("demo:heat").is_none());
    let prepared = snapshot(&[
        "minecraft:vanilla".into(),
        "minecraft:trade_rebalance".into(),
    ]);
    assert!(prepared.id("demo:heat").is_some());
    fs::remove_dir_all(world).unwrap();
}
