//! `Frog.DATA_TONGUE_TARGET_ID` (entry 19) uses
//! `EntityDataSerializers.OPTIONAL_UNSIGNED_INT_CODEC`, which encodes `id + 1`
//! with Java 32-bit wrapping arithmetic, and `0` for absent.
use super::pet_pose_meta_tests::meta_hub_with;
use super::tests::fixture_mob;
use sunrise_gameplay_common::game::lan_mob_metadata::mob_meta;
use sunrise_protocol::entity_data::{Value, serializer};
use sunrise_sim::lan::data::SyncData;

fn sent(target: Option<u32>) -> Vec<(u8, i32, Value)> {
    let hub = meta_hub_with(SyncData::empty());
    let mut frog = fixture_mob("minecraft:frog");
    frog.extra.frog.tongue_target = target;
    let (mut meta, mut defaults) = (Vec::new(), Vec::new());
    mob_meta(&frog, &hub, None, 0, &mut meta, &mut defaults);
    let default = defaults.iter().find(|(index, _, _)| *index == 19);
    assert_eq!(
        default,
        Some(&(19, serializer::OPTIONAL_UNSIGNED_INT, Value::Id(0))),
        "absent tongue target default must be 0"
    );
    meta.into_iter()
        .filter(|(index, _, _)| *index == 19)
        .collect()
}

#[test]
fn frog_tongue_target_preserves_high_bit_entity_ids() {
    assert_eq!(sent(None), vec![]);
    assert_eq!(
        sent(Some(0x7fff_ffff)),
        vec![(19, serializer::OPTIONAL_UNSIGNED_INT, Value::Id(i32::MIN))]
    );
    assert_eq!(
        sent(Some(0x8000_0002)),
        vec![(
            19,
            serializer::OPTIONAL_UNSIGNED_INT,
            Value::Id(i32::MIN + 3)
        )]
    );
}
