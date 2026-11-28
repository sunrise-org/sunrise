use sunrise_sim::{entities::ItemEntity, join_seq::JoinList};

/// Common entity bodies exchanged between the active and parked level.
pub struct EntityStore {
    pub dropped: JoinList<ItemEntity>,
}
