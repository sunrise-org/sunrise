// `lan.rs`'s tests (included: that file may not grow).

/// `SulfurCube`'s entity data after `AbstractCubeMob.ID_SIZE` (18), in the order
/// the class defines it: `MAX_FUSE` (19, an int, -1 until primed), then
/// `FROM_BUCKET` (20, a boolean). A 26.3 client reads each index with its own
/// serializer, and a boolean at 19 disconnected one (#13721).
#[test]
fn a_sulfur_cube_sends_its_fuse_at_19_and_from_bucket_at_20() {
    use sunrise_client::mobs::Mob;
    use sunrise_core::nbt::Tag;
    use sunrise_protocol::entity_data::{Value, serializer};
    let root = std::env::temp_dir().join(format!("sunrise-sulfur-meta-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(
        root.join("entity-types.json"),
        r#"{"minecraft:sulfur_cube":{"width":0.52,"height":0.52,"eye_height":0.325,"category":"monster","fire_immune":false,"attributes":{"minecraft:max_health":4.0}}}"#,
    )
    .unwrap();
    // No attribute catalog: the rows' values stay as written.
    let attributes = &sunrise_world::attributes::entity::Catalog::from_json(&serde_json::json!({
        "data_version": sunrise_world::DATA_VERSION,
        "attributes": [],
        "entities": {},
    }))
    .unwrap();
    let types = sunrise_client::mobs::MobTypes::load_for_server(&root, attributes).unwrap();
    let _ = std::fs::remove_dir_all(&root);
    let kind = std::sync::Arc::new(types.types["minecraft:sulfur_cube"].clone());
    let at = sunrise_core::geometry::Vec3::new(0.5, 64.0, 0.5);
    let mut cube = Mob::new(1, kind, at, 0.0);
    let hub = super::tests::meta_hub("SulfurCubeMetaTest");
    let meta_of = |m: &Mob| {
        let (mut meta, mut defaults) = (Vec::new(), Vec::new());
        super::mob_meta(m, &hub, None, 0, &mut meta, &mut defaults);
        (meta, defaults)
    };

    // A fresh cube: both entries are at their defaults.
    let (meta, defaults) = meta_of(&cube);
    assert!(
        defaults.contains(&(19, serializer::INT, Value::Int(-1))),
        "{defaults:?}"
    );
    assert!(
        defaults.contains(&(20, serializer::BOOLEAN, Value::Bool(false))),
        "{defaults:?}"
    );
    assert!(meta.iter().all(|e| e.0 != 19 && e.0 != 20), "{meta:?}");

    // A saved cube, out of a bucket and primed with 40 ticks to go.
    let saved = Tag::compound([("from_bucket", Tag::Byte(1)), ("fuse", Tag::Int(40))]);
    sunrise_client::mobs::sulfur_cube::from_nbt(&mut cube, &saved);
    let (meta, _) = meta_of(&cube);
    assert!(
        meta.contains(&(19, serializer::INT, Value::Int(40))),
        "{meta:?}"
    );
    assert!(
        meta.contains(&(20, serializer::BOOLEAN, Value::Bool(true))),
        "{meta:?}"
    );
}

/// The pinned-vanilla side is `VanillaSulfurCubeFuseOracle` (`tools/test_sulfur_cube_fuse_parity.py`):
/// a sulfur cube loaded with each `fuse` (absent, signed boundaries, every numeric tag type, a
/// nonnumeric one) and each `Invulnerable`, through the production load, save, `tickFuse` and
/// `mob_meta`, with what `send_dirty` would send to trackers already following it.
#[test]
fn a_loaded_sulfur_cube_fuse_matches_the_pinned_oracle() {
    use sunrise_client::mobs::{Mob, sulfur_cube};
    use sunrise_core::nbt::{Tag, parse_snbt};
    use sunrise_protocol::entity_data::Value;
    let root = std::env::temp_dir().join(format!("sunrise-sulfur-fuse-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(
        root.join("entity-types.json"),
        r#"{"minecraft:sulfur_cube":{"width":0.52,"height":0.52,"eye_height":0.325,"category":"monster","fire_immune":false,"attributes":{"minecraft:max_health":4.0}}}"#,
    )
    .unwrap();
    let attributes = &sunrise_world::attributes::entity::Catalog::from_json(&serde_json::json!({
        "data_version": sunrise_world::DATA_VERSION,
        "attributes": [],
        "entities": {},
    }))
    .unwrap();
    let types = sunrise_client::mobs::MobTypes::load_for_server(&root, attributes).unwrap();
    let _ = std::fs::remove_dir_all(&root);
    let types: std::collections::HashMap<String, std::sync::Arc<sunrise_client::mobs::MobType>> =
        types
            .types
            .iter()
            .map(|(id, kind)| (id.clone(), std::sync::Arc::new(kind.clone())))
            .collect();
    let hub = super::tests::meta_hub("SulfurCubeFuseTest");
    let meta_of = |m: &Mob| {
        let (mut meta, mut defaults) = (Vec::new(), Vec::new());
        super::mob_meta(m, &hub, None, 0, &mut meta, &mut defaults);
        (meta, defaults)
    };
    let max_fuse = |meta: &[(u8, i32, Value)]| match meta.iter().find(|e| e.0 == 19) {
        Some((_, _, Value::Int(v))) => v.to_string(),
        _ => "-".into(),
    };
    // `send_dirty`: a changed or new entry, or the default of one that dropped out.
    let sent =
        |before: &[(u8, i32, Value)], after: &[(u8, i32, Value)], defaults: &[(u8, i32, Value)]| {
            if let Some(entry) = after.iter().find(|e| e.0 == 19) {
                return if before.contains(entry) {
                    "-".to_owned()
                } else {
                    max_fuse(after)
                };
            }
            if before.iter().any(|e| e.0 == 19) {
                return max_fuse(defaults);
            }
            "-".into()
        };
    let load = |fuse: Option<&str>, invulnerable: Option<&str>| {
        let mut snbt = String::from(r#"{id:"minecraft:sulfur_cube",Pos:[0.5d,64d,0.5d],NoAI:1b"#);
        if let Some(fuse) = fuse {
            snbt += &format!(",fuse:{fuse}");
        }
        if let Some(invulnerable) = invulnerable {
            snbt += &format!(",Invulnerable:{invulnerable}");
        }
        let tag = parse_snbt(&(snbt + "}")).unwrap().0;
        sunrise_client::entity_nbt::mob_from_nbt(&tag, &types, 1, None).unwrap()
    };
    let mut rows = Vec::new();
    for fuse in [
        None,
        Some("-2147483648"),
        Some("-5"),
        Some("-1"),
        Some("0"),
        Some("1"),
        Some("40"),
        Some("40s"),
        Some("40b"),
        Some("4294967336L"),
        Some("-1.5f"),
        Some("1.5d"),
        Some("2147483647"),
        Some("\"lit\""),
    ] {
        for invulnerable in [None, Some("0b"), Some("1b")] {
            let mut cube = load(fuse, invulnerable);
            let loaded = (
                sulfur_cube::fuse(&cube),
                sulfur_cube::max_fuse(&cube),
                cube.invulnerable_flag,
                sulfur_cube::primed(&cube),
            );
            let tag = sunrise_client::entity_nbt::mob_to_nbt(
                &cube,
                [1, 2, 3, 4],
                None,
                None,
                |_| None,
                &Default::default(),
            );
            let saved_fuse = match tag.get("fuse") {
                Some(Tag::Int(v)) => v.to_string(),
                _ => "-".into(),
            };
            let saved_invulnerable = match tag.get("Invulnerable") {
                Some(Tag::Byte(v)) => v.to_string(),
                _ => "-".into(),
            };
            let (first, _) = meta_of(&cube);
            for _ in 0..3 {
                sulfur_cube::tick_fuse(&mut cube, &mut Vec::new());
            }
            let (ticked, defaults) = meta_of(&cube);
            let fuse_after = sulfur_cube::fuse(&cube);
            let back = load(Some("-1"), None);
            let (reset, _) = meta_of(&back);
            rows.push(format!(
                "{} {} loaded {} {} {} {} saved {saved_fuse} {saved_invulnerable} late {} ticked {fuse_after} {} back {}",
                fuse.unwrap_or("absent"),
                invulnerable.unwrap_or("absent"),
                loaded.0,
                loaded.1,
                loaded.2,
                loaded.3,
                max_fuse(&first),
                sent(&first, &ticked, &defaults),
                sent(&ticked, &reset, &defaults),
            ));
        }
    }
    if let Some(path) = std::env::var_os("SUNRISE_SULFUR_CUBE_FUSE_OUTPUT") {
        std::fs::write(path, rows.join("\n") + "\n").unwrap();
    }
    // Pinned rows of the 26.3 oracle.
    for row in [
        "0 absent loaded 0 0 false true saved 0 0 late 0 ticked 0 - back -1",
        "40s 1b loaded 40 40 true true saved 40 1 late 40 ticked 37 - back -1",
        "-5 absent loaded -5 -5 false false saved -5 0 late -5 ticked -5 - back -1",
        "4294967336L 0b loaded 40 40 false true saved 40 0 late 40 ticked 37 - back -1",
        "-1.5f 1b loaded -2 -2 true false saved -2 1 late -2 ticked -2 - back -1",
        "2147483647 absent loaded 2147483647 2147483647 false true saved 2147483647 0 late 2147483647 ticked 2147483644 - back -1",
        "\"lit\" absent loaded -1 -1 false false saved -1 0 late - ticked -1 - back -",
    ] {
        assert!(
            rows.iter().any(|r| r == row),
            "missing pinned row {row}\n{rows:#?}"
        );
    }
    assert_eq!(rows.len(), 14 * 3);
}
