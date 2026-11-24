//! UNEXECUTED tests; intended child of client-render/src/block_entities/carry.rs.
use super::super::Ticking;
use super::*;
use sunrise_mod_runtime::attachments::{AttachmentType, Holder};
static VALUE: AttachmentType<i32> = AttachmentType {
    namespace: "transfer_test",
    name: "opaque",
    default: || 0,
    codec: None,
    copy_on_death: false,
    sync: false,
};

#[test]
fn actual_chest_allocation_and_attachment_binding_move_to_one_new_holder() {
    let (from, to) = ((1, 64, 1), (17, 64, 1));
    let mut source = BlockEntities::default();
    let mut destination = BlockEntities::default();
    source.set_states(vec![Ticking::Yes]);
    destination.set_states(vec![Ticking::Yes]);
    let mut slots = vec![None; 27];
    slots[0] = Some(crate::items::ItemStack::new("minecraft:stone", 7));
    let allocation = slots.as_ptr();
    source.insert(from, BlockEntity::Chest(slots));
    VALUE
        .set(source.attachments_mut(from).unwrap(), Holder::Level, 31)
        .unwrap();
    let entry = source
        .attachments(from)
        .unwrap()
        .entry("transfer_test", "opaque")
        .unwrap()
        .unwrap()
        .clone();
    let old_identity = source.identity(from).unwrap();
    let payload = source.extract_owned(from).unwrap();
    assert!(source.identity(from).is_none());
    assert!(source.attachments(from).is_none());
    assert!(source.tickers(|_| 0).is_empty());
    destination
        .install_owned(to, payload)
        .unwrap_or_else(|_| panic!("empty destination refused owned payload"));
    let Some(BlockEntity::Chest(slots)) = destination.get(&to) else {
        panic!("actual chest missing");
    };
    assert_eq!(slots.as_ptr(), allocation); // Actual Vec moved; no live inventory clone.
    assert_eq!(slots[0].as_ref().unwrap().count, 7);
    assert_eq!(destination.tickers(|_| 0).len(), 1);
    assert_ne!(destination.identity(to).unwrap(), old_identity);
    assert!(std::sync::Arc::ptr_eq(
        &entry,
        destination
            .attachments(to)
            .unwrap()
            .entry("transfer_test", "opaque")
            .unwrap()
            .unwrap()
    ));
    assert!(!source.contains_key(&from));
}
