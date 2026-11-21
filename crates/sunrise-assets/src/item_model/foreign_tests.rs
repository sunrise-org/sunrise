use super::*;

fn node(json: &str) -> Result<ItemModelNode> {
    ItemModelNode::decode(&serde_json::from_str(json).unwrap())
}
fn model_id(n: &ItemModelNode) -> &str {
    match n {
        ItemModelNode::Model(leaf) => &leaf.model,
        other => panic!("not a model: {other:?}"),
    }
}

#[test]
fn unknown_vanilla_names_still_fail() {
    assert!(node(r#"{"type":"minecraft:nope"}"#).is_err());
    assert!(node(r#"{"type":"nope","model":"a:b"}"#).is_err());
    let leaf = r#"{"type":"minecraft:model","model":"a:b"}"#;
    for json in [
        format!(
            r#"{{"type":"minecraft:condition","property":"minecraft:nope","on_true":{leaf},"on_false":{leaf}}}"#
        ),
        format!(
            r#"{{"type":"minecraft:select","property":"minecraft:nope","cases":[],"fallback":{leaf}}}"#
        ),
        format!(
            r#"{{"type":"minecraft:range_dispatch","property":"minecraft:nope","entries":[],"fallback":{leaf}}}"#
        ),
    ] {
        assert!(node(&json).is_err(), "{json}");
    }
    assert!(
        node(r#"{"type":"minecraft:special","base":"a:b","model":{"type":"minecraft:nope"}}"#)
            .is_err()
    );
    assert!(ClientItem::from_json(r#"{"model":{"type":"minecraft:nope"}}"#).is_err());
}

#[test]
fn condition_property_defaults_to_false_at_draw_time() {
    let definition = ClientItem::from_json(
        r#"{"model":{"type":"minecraft:condition","property":"mod:flag",
            "on_true":{"type":"minecraft:model","model":"a:yes"},
            "on_false":{"type":"minecraft:model","model":"a:no"}}}"#,
    )
    .unwrap();
    let mut models = ItemModels::default();
    models.definitions.insert("mod:tool".into(), definition);
    Arc::make_mut(&mut models.prototypes).insert(
        "mod:tool".into(),
        BTreeMap::from([(
            "minecraft:item_model".into(),
            Value::String("mod:tool".into()),
        )]),
    );
    let context = Context {
        display: DisplayContext::Gui,
        owner: None,
        level: None,
        seed: 0,
    };
    let resolved = models.resolve(&StackView::new("mod:tool", 1), &context, &NoEnv);
    assert_eq!(resolved.layers.len(), 1);
    assert_eq!(
        resolved.layers[0].content,
        LayerContent::Model {
            model: "a:no".into()
        }
    );
}

#[test]
fn select_property_takes_the_fallback_or_nothing() {
    let n = node(
        r#"{"type":"minecraft:select","property":"mod:level",
            "cases":[{"when":"low","model":{"type":"minecraft:model","model":"a:low"}}],
            "fallback":{"type":"minecraft:model","model":"a:other"}}"#,
    )
    .unwrap();
    assert_eq!(model_id(&n), "a:other");
    let n = node(r#"{"type":"minecraft:select","property":"mod:level","cases":[]}"#).unwrap();
    assert!(matches!(n, ItemModelNode::Empty));
}

#[test]
fn range_property_reads_zero() {
    let entry = |t: f32, m: &str| {
        format!(r#"{{"threshold":{t},"model":{{"type":"minecraft:model","model":"{m}"}}}}"#)
    };
    let fallback = r#","fallback":{"type":"minecraft:model","model":"a:fb"}"#;
    let make = |entries: &[String], fallback: &str| {
        node(&format!(
            r#"{{"type":"minecraft:range_dispatch","property":"mod:fill","entries":[{}]{fallback}}}"#,
            entries.join(",")
        ))
        .unwrap()
    };
    let n = make(&[entry(0.2, "a:one"), entry(0.4, "a:two")], fallback);
    assert_eq!(model_id(&n), "a:fb");
    let n = make(
        &[
            entry(0.0, "a:zero"),
            entry(-1.0, "a:neg"),
            entry(0.5, "a:half"),
        ],
        fallback,
    );
    assert_eq!(model_id(&n), "a:zero");
    let n = make(&[entry(0.2, "a:one")], "");
    assert!(matches!(n, ItemModelNode::Empty));
}

/// A special model type no mod registered is an error naming it, so the item definition holding
/// it isn't loaded and the item draws the missing model; its `base` no longer stands in. A type a
/// mod registered, vanilla's included, decodes with its model object for the mod's renderer, and
/// vanilla's types no mod registered keep theirs.
#[test]
fn a_special_type_draws_by_its_renderer_or_fails_by_name() {
    let error = node(
        r#"{"type":"minecraft:special","base":"mod:item/cube","model":{"type":"mod:energy_cube"}}"#,
    )
    .unwrap_err()
    .to_string();
    assert!(
        error.contains("unknown special model type mod:energy_cube"),
        "{error}"
    );
    // A mod may replace a vanilla type: its models then go to the mod's renderer.
    let skillet = RegisteredSpecialModel(Arc::new(2));
    let bell = RegisteredSpecialModel(Arc::new(()));
    let load = |kind: &str, base: &str, json: &Map<String, Value>| {
        Ok(match kind {
            "foreigntest:skillet" => {
                assert_eq!(base, "minecraft:item/pan");
                assert_eq!(json["handle"], serde_json::json!(2));
                Some(skillet.clone())
            }
            "minecraft:bell" => Some(bell.clone()),
            _ => None,
        })
    };
    let decode = |json| {
        ClientItem::from_json_with_specials(json, &load)
            .unwrap()
            .model
    };
    let pan = decode(
        r#"{"model":{"type":"special","base":"item/pan","model":{"type":"foreigntest:skillet","handle":2}}}"#,
    );
    let mut dependencies = Vec::new();
    pan.dependencies(&mut dependencies);
    assert_eq!(dependencies, ["minecraft:item/pan"]);
    assert!(matches!(pan, ItemModelNode::RegisteredSpecial { model, .. } if model == skillet));
    let shield = decode(r#"{"model":{"type":"special","base":"a:b","model":{"type":"shield"}}}"#);
    assert!(matches!(shield, ItemModelNode::Special { .. }));
    let overridden = decode(r#"{"model":{"type":"special","base":"a:b","model":{"type":"bell"}}}"#);
    assert!(matches!(overridden, ItemModelNode::RegisteredSpecial { model, .. } if model == bell));
}

#[test]
fn a_registered_vanilla_override_error_rejects_only_its_enclosing_item() {
    let load = |kind: &str, _: &str, _: &Map<String, Value>| match kind {
        "minecraft:bell" => Err("the bell override rejected its model".into()),
        _ => Ok(None),
    };
    let composite = r#"{"model":{"type":"composite","models":[
        {"type":"model","model":"a:plain"},
        {"type":"select","property":"display_context","cases":[{"when":"gui","model":
            {"type":"special","base":"a:bell","model":{"type":"bell"}}
        }]}
    ]}}"#;
    // The same tree decodes with no registrations, so a builtin fallback would hide the error.
    assert!(ClientItem::from_json(composite).is_ok());
    assert_eq!(
        ClientItem::from_json_with_specials(composite, &load)
            .unwrap_err()
            .to_string(),
        "the bell override rejected its model"
    );
    let other = ClientItem::from_json_with_specials(
        r#"{"model":{"type":"special","base":"a:shield","model":{"type":"shield"}}}"#,
        &load,
    )
    .unwrap();
    assert!(matches!(
        other.model,
        ItemModelNode::Special {
            model: SpecialModel {
                kind: SpecialKind::Shield
            },
            ..
        }
    ));
}

/// A node type no mod registered is an error naming it, so the item definition holding it isn't
/// loaded and the item draws the missing model, whatever keys the node has. NeoForge's own
/// types are no exception, except `neoforge:fluid_container`, which the engine decodes (#2606).
#[test]
fn an_unregistered_node_type_fails_by_name() {
    for json in [
        r#"{"type":"mod:thing","model":"mod:item/thing"}"#,
        r#"{"type":"neoforge:trimmed_armor","base_model":{"type":"minecraft:model","model":"mod:item/boots"}}"#,
        r#"{"type":"mod:facade"}"#,
    ] {
        let kind = serde_json::from_str::<Value>(json).unwrap()["type"]
            .as_str()
            .unwrap()
            .to_owned();
        let error = node(json).unwrap_err().to_string();
        assert!(
            error.contains(&format!("unknown item model type {kind}")),
            "{error}"
        );
    }
    assert!(ClientItem::from_json(r#"{"model":{"type":"ae2:facade"}}"#).is_err());
}

/// Ownership comes from the decoder, not from the minecraft namespace or registrations.
#[test]
fn special_model_builtin_ownership_uses_the_decoder_identity() {
    assert!(SpecialModel::is_builtin_type("minecraft:bell"));
    assert!(SpecialModel::is_builtin_type("minecraft:chest"));
    assert!(!SpecialModel::is_builtin_type("minecraft:foo"));
    assert!(!SpecialModel::is_builtin_type("example:bell"));
    assert!(SpecialModel::decode(&serde_json::json!({"type": "minecraft:bell"})).is_ok());
    assert!(SpecialModel::decode(&serde_json::json!({"type": "minecraft:foo"})).is_err());
}
