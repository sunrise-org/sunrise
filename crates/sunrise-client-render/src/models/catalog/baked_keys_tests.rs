//! `StateModels::state_slots` (#861 part 1): every baked model a state can select, as the keys
//! `RenderAssets::block_state_models` hands a mod, without consuming the render random source.
use super::bounds_tests::catalog;
use super::*;

#[test]
fn a_state_lists_every_weighted_alternative_and_multipart_part_once_in_bake_order() {
    for (definition, selected, expected) in [
        (
            r#"{"variants":{"":{"model":"sunrise:b"}}}"#,
            SelectedModels::Variant(0),
            vec![1],
        ),
        (
            r#"{"variants":{"": [{"model":"sunrise:a","weight":1},{"model":"sunrise:b","weight":3},{"model":"sunrise:a","weight":2}]}}"#,
            SelectedModels::Variant(0),
            vec![0, 1],
        ),
        (
            r#"{"multipart":[{"apply":{"model":"sunrise:b"}},{"apply":{"model":"sunrise:a"}}]}"#,
            SelectedModels::Multipart(vec![0, 1]),
            vec![1, 0],
        ),
        (
            r#"{"multipart":[{"apply":{"model":"sunrise:b"}},{"apply":{"model":"sunrise:a"}}]}"#,
            SelectedModels::Multipart(vec![1]),
            vec![0],
        ),
        (
            r#"{"multipart":[{"apply":{"model":"sunrise:a"}}]}"#,
            SelectedModels::Multipart(vec![]),
            vec![],
        ),
    ] {
        let mut models = catalog(definition);
        models.states[0] = Some((0, selected));
        assert_eq!(
            models.state_slots(BlockStateId(0)),
            Some(expected),
            "{definition}"
        );
    }
}

#[test]
fn an_unbound_or_corrupt_selection_lists_nothing() {
    let mut models = catalog(r#"{"variants":{"":{"model":"sunrise:a"}}}"#);
    assert_eq!(models.state_slots(BlockStateId(1)), None, "past the table");
    models.states[0] = None;
    assert_eq!(
        models.state_slots(BlockStateId(0)),
        None,
        "no definition binds it"
    );
    models.states[0] = Some((0, SelectedModels::Variant(5)));
    assert_eq!(
        models.state_slots(BlockStateId(0)),
        None,
        "a selection past its choices"
    );
    assert_eq!(StateModels::default().state_slots(BlockStateId(0)), None);
}

#[test]
fn listing_alternatives_leaves_the_seeded_selection_unchanged() {
    let models = catalog(
        r#"{"variants":{"": [{"model":"sunrise:a","weight":1},{"model":"sunrise:b","weight":3}]}}"#,
    );
    let seed = position_seed(BlockPos::new(13, 64, -7));
    let before: Vec<_> = models
        .collect_seeded(BlockStateId(0), seed)
        .unwrap()
        .into_iter()
        .map(|(variant, _)| variant.model.clone())
        .collect();
    assert_eq!(models.state_slots(BlockStateId(0)), Some(vec![0, 1]));
    let after: Vec<_> = models
        .collect_seeded(BlockStateId(0), seed)
        .unwrap()
        .into_iter()
        .map(|(variant, _)| variant.model.clone())
        .collect();
    assert_eq!(before, after);
}
