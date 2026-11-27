//! Mob metadata and live attributes, built on the owning server thread.
use super::*;
use sunrise_gameplay_common::game::lan_mob_metadata::mob_freeze_meta;
use sunrise_gameplay_common::game::tables::GameItems as _;

use sunrise_items::equipment::Equipment;
use sunrise_level::fast_hash::FxHashMap;
use sunrise_sim::join_seq::{JoinSeq, Joined};
use sunrise_world::attributes::entity::Catalog;

use super::super::lan_attributes::{MobAttributeCache, SpeedByType};

impl crate::game::ServerGame<'_> {
    pub(in crate::game::server_side) fn fill_mob_state(
        &self,
        m: &Mob,
        hub: &Hub,
        player_entity: Option<i32>,
        speeds: &mut super::super::lan_attributes::SpeedByType,
        e: &mut sunrise_sim::lan::entity::EntityState,
    ) {
        self.mob_metadata_state(m, hub, player_entity, None, e);
        self.mob_attributes(m, speeds, &mut e.attributes);
    }

    #[inline(always)]
    fn mob_metadata_state(
        &self,
        m: &Mob,
        hub: &Hub,
        player_entity: Option<i32>,
        cached: Option<&mut Metadata>,
        e: &mut EntityState,
    ) {
        let inputs = cached.as_ref().and_then(|_| MetadataInputs::of(m));
        if let (Some(old), Some(inputs)) = (cached.as_ref(), inputs)
            && old.matches(m, inputs)
        {
            e.meta.extend_from_slice(&old.entries);
            e.defaults.extend_from_slice(&old.defaults);
            return;
        }
        super::super::villager_client::server_meta(
            m,
            hub,
            player_entity,
            self.common.world.game_time,
            &mut e.meta,
            &mut e.defaults,
            &self.stations().professions,
        );
        // HappyGhast.aiStep: requiresPrecisePosition while on a still timeout.
        e.precise = m.species() == sunrise_sim::mobs::Species::HappyGhast
            && sunrise_sim::mobs::happy_ghast::on_still_timeout(m);
        mob_freeze_meta(m, &mut e.meta, &mut e.defaults);
        if let (Some(old), Some(inputs)) = (cached, inputs) {
            *old = Metadata::capture(m, inputs, e);
        }
    }
}

#[derive(Default)]
pub(super) struct MobStates {
    catalog: Option<Arc<Catalog>>,
    sync: Option<Arc<lan::data::SyncData>>,
    levels: FxHashMap<super::super::Dim, MobLevel>,
}

impl MobStates {
    pub(super) fn level(
        &mut self,
        dim: super::super::Dim,
        catalog: &Arc<Catalog>,
        sync: &Arc<lan::data::SyncData>,
    ) -> &mut MobLevel {
        if !self
            .catalog
            .as_ref()
            .is_some_and(|old| Arc::ptr_eq(old, catalog))
            || !self.sync.as_ref().is_some_and(|old| Arc::ptr_eq(old, sync))
        {
            self.catalog = Some(catalog.clone());
            self.sync = Some(sync.clone());
            self.levels.clear();
        }
        self.levels.entry(dim).or_default()
    }
}

#[derive(Default)]
pub(super) struct MobLevel {
    rows: FxHashMap<u32, MobData>,
}

impl MobLevel {
    pub(super) fn trim(&mut self, count: usize) {
        if self.rows.len() > count.saturating_mul(2) {
            self.rows.clear();
        }
    }

    pub(super) fn row(&mut self, m: &Mob) -> &mut MobData {
        let row = self.rows.entry(m.id).or_insert_with(|| MobData::new(m));
        if row.join != m.join_seq() || !Arc::ptr_eq(&row.kind, &m.kind) {
            *row = MobData::new(m);
        }
        row
    }
}

pub(super) struct MobData {
    join: JoinSeq,
    kind: Arc<sunrise_sim::mobs::MobType>,
    attributes: MobAttributeCache,
    metadata: Option<Box<Metadata>>,
    equipment: Option<Box<EquipmentState>>,
}

impl MobData {
    fn new(m: &Mob) -> Self {
        Self {
            join: m.join_seq(),
            kind: m.kind.clone(),
            attributes: Default::default(),
            metadata: None,
            equipment: None,
        }
    }

    pub(super) fn fill(
        &mut self,
        game: &crate::game::ServerGame<'_>,
        m: &Mob,
        hub: &Hub,
        player: Option<i32>,
        speeds: &mut SpeedByType,
        e: &mut EntityState,
    ) {
        game.mob_metadata_state(m, hub, player, self.metadata.as_deref_mut(), e);
        if self.metadata.is_none()
            && let Some(inputs) = MetadataInputs::of(m)
        {
            self.metadata = Some(Box::new(Metadata::capture(m, inputs, e)));
        }
        game.mob_attribute_state(m, speeds, &mut e.attributes, &mut self.attributes);
    }

    pub(super) fn equipment(&mut self, m: &Mob, hub: &mut Hub, e: &mut EntityState) {
        let gear = &m.extra.equipment;
        if gear.is_empty() {
            self.equipment = None;
            return;
        }
        let plain = gear
            .iter()
            .flatten()
            .all(|stack| stack.extra.is_none() && stack.damage == 0 && stack.count <= 1);
        if plain
            && let Some(old) = &self.equipment
            && old.input == *gear
        {
            e.equipment.extend_from_slice(&old.output);
            return;
        }
        mob_equipment(m, hub, &mut e.equipment);
        self.equipment = (plain
            && e.equipment
                .iter()
                .all(|(_, stack)| stack.as_ref().is_some_and(|stack| stack.item > 0)))
        .then(|| {
            Box::new(EquipmentState {
                input: gear.clone(),
                output: e.equipment.clone(),
            })
        });
    }
}

struct EquipmentState {
    input: Equipment,
    output: Vec<(u8, Option<sunrise_protocol::item::Stack>)>,
}

type MetaEntries = Vec<(u8, i32, Value)>;

struct Metadata {
    inputs: MetadataInputs,
    name: Option<String>,
    effects: Vec<ShownEffect>,
    entries: MetaEntries,
    defaults: MetaEntries,
}

impl Metadata {
    fn capture(m: &Mob, inputs: MetadataInputs, e: &EntityState) -> Self {
        Self {
            inputs,
            name: m.custom_name.as_deref().map(str::to_owned),
            effects: mob_effects(m).collect(),
            entries: e.meta.clone(),
            defaults: e.defaults.clone(),
        }
    }

    fn matches(&self, m: &Mob, inputs: MetadataInputs) -> bool {
        self.inputs == inputs
            && self.name.as_deref() == m.custom_name.as_deref()
            && self.effects.iter().copied().eq(mob_effects(m))
    }
}

#[derive(Clone, Copy, PartialEq)]
struct MetadataInputs {
    species_data: [Option<i32>; 3],
    health: u32,
    air: i32,
    frozen: i32,
    arrows: i32,
    stingers: i32,
    pose: u8,
    flags: u16,
}

impl MetadataInputs {
    fn of(m: &Mob) -> Option<Self> {
        use sunrise_sim::mobs::{spear_use::is_using_item, sync::pose_id};
        if m.extra.modified_effect_particles.is_some()
            || m.custom_name_text.is_some()
            || m.kind.class_id() != m.kind.id.as_str()
            || sunrise_sim::mobs::raider::is_raider(m.species())
            || sunrise_sim::mobs::camel::is_camel(m.species())
            || m.species() == Species::HappyGhast
        {
            return None;
        }
        let e = &m.extra;
        let species_data = match m.kind.class_id() {
            "minecraft:cow" => [
                e.cow_variant.map(|r| r.network_id() as i32),
                e.cow_sound_variant.map(|r| r.network_id() as i32),
                None,
            ],
            "minecraft:pig" => [
                e.pig_variant.map(|r| r.network_id() as i32),
                e.pig_sound_variant.map(|r| r.network_id() as i32),
                Some(e.counter2.max(0)),
            ],
            "minecraft:chicken" => [
                e.chicken_variant.map(|r| r.network_id() as i32),
                e.chicken_sound_variant.map(|r| r.network_id() as i32),
                None,
            ],
            "minecraft:zombie" | "minecraft:husk" | "minecraft:skeleton" => {
                [Some(i32::from(e.counter2 > 0)), None, None]
            }
            "minecraft:creeper" => [
                Some(e.counter),
                Some(i32::from(e.charged)),
                Some(i32::from(e.flag)),
            ],
            "minecraft:sheep" => [
                Some(i32::from(e.color & 15)),
                Some(i32::from(e.sheared)),
                None,
            ],
            "minecraft:slime" | "minecraft:magma_cube" => [Some(e.size.max(1)), None, None],
            "minecraft:ocelot" => [Some(i32::from(e.trusting)), None, None],
            "minecraft:goat" => [
                Some(i32::from(e.lost_horns[0])),
                Some(i32::from(e.lost_horns[1])),
                None,
            ],
            "minecraft:mooshroom" | "minecraft:rabbit" | "minecraft:salmon" => {
                [Some(i32::from(e.variant)), None, None]
            }
            "minecraft:drowned" | "minecraft:bat" | "minecraft:blaze" | "minecraft:ghast" => {
                [Some(i32::from(e.flag)), None, None]
            }
            "minecraft:cod"
            | "minecraft:squid"
            | "minecraft:spider"
            | "minecraft:cave_spider"
            | "minecraft:stray"
            | "minecraft:wither_skeleton"
            | "minecraft:iron_golem"
            | "minecraft:dolphin"
            | "minecraft:zombified_piglin" => [None; 3],
            _ => return None,
        };
        let flags = [
            m.is_on_fire(),
            e.sprinting,
            m.species() == Species::Drowned && e.flag,
            m.is_invisible(),
            m.is_currently_glowing(),
            m.name_visible,
            m.no_gravity,
            is_using_item(m),
            e.using_off_hand,
            m.no_ai,
            e.left_handed,
            m.ai.aggressive,
            m.is_baby(),
            e.age_locked,
        ]
        .into_iter()
        .enumerate()
        .fold(0, |bits, (index, on)| bits | (u16::from(on) << index));
        Some(Self {
            species_data,
            health: m.health.max(0.0).to_bits(),
            air: m.air,
            frozen: m.ticks_frozen,
            arrows: m.extra.stuck.arrows,
            stingers: m.extra.stuck.stingers,
            pose: pose_id(m),
            flags,
        })
    }
}
