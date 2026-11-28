use super::{
    FallingBlock, boats_common, dragon_common, dragon_parts_table, fangs_common, fireworks,
    general, lightning_common, minecarts, ominous_item_spawner_common, tnt_common,
};
use sunrise_sim::{
    entities::{ExperienceOrb, ItemEntity},
    mobs::Mob,
};

/// Common entity bodies exchanged between the active and parked level.
pub struct EntityStore {
    pub dropped: sunrise_sim::join_seq::JoinList<ItemEntity>,
    pub mobs: sunrise_sim::join_seq::JoinList<Mob>,
    pub projectiles: sunrise_sim::join_seq::JoinList<sunrise_sim::projectiles::Projectile, false>,
    pub orbs: sunrise_sim::join_seq::JoinList<ExperienceOrb>,
    pub falling: sunrise_sim::join_seq::JoinList<FallingBlock, false>,
    pub paintings: sunrise_sim::join_seq::JoinList<sunrise_sim::entities::Painting>,
    pub item_frames: sunrise_sim::join_seq::JoinList<sunrise_sim::entities::ItemFrame>,
    pub armor_stands: sunrise_sim::join_seq::JoinList<sunrise_sim::entities::ArmorStand>,
    pub cushions: sunrise_sim::join_seq::JoinList<sunrise_sim::entities::Cushion>,
    pub boats: sunrise_sim::join_seq::JoinList<boats_common::Boat, false>,
    pub carts: sunrise_sim::join_seq::JoinList<minecarts::Minecart, false>,
    pub bolts: sunrise_sim::join_seq::ResidentList<lightning_common::Bolt>,
    pub dragon: dragon_parts_table::DragonSlot,
    pub crystals: sunrise_sim::join_seq::JoinList<dragon_common::Crystal>,
    pub breath_clouds: sunrise_sim::join_seq::ResidentList<dragon_common::Breath>,
    pub tridents: sunrise_sim::join_seq::JoinList<sunrise_sim::thrown_trident::ThrownTrident>,
    pub rockets: sunrise_sim::join_seq::JoinList<fireworks::Rocket>,
    pub ominous_spawners:
        sunrise_sim::join_seq::ResidentList<ominous_item_spawner_common::OminousItemSpawner>,
    pub tnts: sunrise_sim::join_seq::JoinList<tnt_common::PrimedTnt>,
    pub knots: sunrise_sim::join_seq::JoinList<sunrise_gameplay_common::leash_knot::Knot>,
    pub general: general::GeneralStore,
    pub fangs: fangs_common::FangState,
}

impl Default for EntityStore {
    #[inline(never)]
    fn default() -> Self {
        Self {
            dropped: Default::default(),
            mobs: Default::default(),
            projectiles: Default::default(),
            orbs: Default::default(),
            falling: Default::default(),
            paintings: Default::default(),
            item_frames: Default::default(),
            armor_stands: Default::default(),
            cushions: Default::default(),
            boats: Default::default(),
            carts: Default::default(),
            bolts: Default::default(),
            dragon: Default::default(),
            crystals: Default::default(),
            breath_clouds: Default::default(),
            tridents: Default::default(),
            rockets: Default::default(),
            ominous_spawners: Default::default(),
            tnts: Default::default(),
            knots: Default::default(),
            general: Default::default(),
            fangs: Default::default(),
        }
    }
}
