//! A server's levels, one per dimension (`MinecraftServer.levels`).
//!
//! The game's tick code reads one level: the one in place, held in `Game`'s
//! own fields (the world, its streaming and save, block ticks, entities).
//! A server (the dedicated server and single player's server thread) keeps
//! the other dimensions' levels here and ticks each one with players or forced
//! chunks by swapping it into place (`enter_level`), the way `as_guest` swaps a player
//! in, so the tick code runs unchanged, one level at a time.
//!
//! Every level shares the Overworld's clocks, weather parameters, rules, scoreboard, maps and portal arrivals.

use super::entity_residency::{define_resident_access, resident_owners};
use super::{
    Dim, ExperienceOrb, FallingBlock, Mob,
    boats_common::Boat,
    creaking::CreakingLevelState,
    dragon_common,
    dragon_parts_table::DragonSlot,
    entity_memory::{EntityMemory, OffLevel},
    fireworks,
    lightning_common::Bolt,
    minecarts,
    nether_portal::PortalLevelState,
    ominous_item_spawner_common::OminousItemSpawner,
    raids::RaidLevelState,
    sculk_server::{SculkLevelState, SculkState},
    tnt_common::PrimedTnt,
};
use mod_api::DimensionKey;
use std::{
    borrow::Cow,
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};
use sunrise_game_data::Result;
use sunrise_gameplay_common::game::GameGameExt3;
use sunrise_gameplay_common::game::commands_common::{OtherEntity, Target};
use sunrise_gameplay_common::game::end_state::EndLevelState;
use sunrise_gameplay_common::game::entity::EntityLists;
use sunrise_gameplay_common::game::entity_store::EntityStore;
use sunrise_gameplay_common::game::residency_tick::ResidencyTickGameExt as _;
use sunrise_gameplay_common::game::transactions::entities::EntityRemovalGameExt as _;
use sunrise_gameplay_common::{hoppers::HopperState, leash_knot::Knot};
use sunrise_level::spans::time as span;
use sunrise_level::world::World;
use sunrise_level::{block_entity_ticks::BlockEntityLevelTicks, chunk_streamer::ChunkStreamer};
use sunrise_sim::join_seq::{
    Joined,
    residency::{
        Journal, Kind, Owner,
        table::{EntityLookup, Location, SourceEntity},
    },
};
use sunrise_sim::thrown_trident::ThrownTrident;
use sunrise_sim::{
    random_tick_sections::RandomTickSections,
    save::SaveDir,
    tick_chunk_membership::TickChunkMembership,
    ticket_levels::{ChunkTickets, PortalTicket},
    vanillagen::VanillaTerrain,
};
use sunrise_world::level::{LevelEvent, Ticks};

#[cfg(feature = "test-support")]
pub trait LevelScopeTestExt {
    fn with_level_in_place<R>(&mut self, dim: Dim, f: impl FnOnce(&mut Self) -> R) -> Option<R>;
}

#[cfg(feature = "test-support")]
impl LevelScopeTestExt for crate::game::ServerGame<'_> {
    fn with_level_in_place<R>(&mut self, dim: Dim, f: impl FnOnce(&mut Self) -> R) -> Option<R> {
        crate::game::ServerGame::with_level_in_place(self, dim, f)
    }
}

/// A borrowed counting view: no resident-table scans and no allocation for gauges.
pub(in crate::game) struct EntityCountView<'a> {
    pub(in crate::game) fixed: [(&'static str, usize); 19],
    pub(in crate::game) mobs: &'a [Mob],
    pub(in crate::game) projectiles: &'a [sunrise_sim::projectiles::Projectile],
    pub(in crate::game) general: &'a super::general::GeneralRows,
    pub(in crate::game) others: &'a [OtherEntity],
}
impl EntityCountView<'_> {
    pub(in crate::game::server_side) fn total(&self) -> usize {
        self.fixed.iter().map(|(_, n)| n).sum::<usize>()
            + self.projectiles.len()
            + self.mobs.len()
            + self.general.len()
            + self.others.len()
    }
    pub(in crate::game::server_side) fn registry_counts(&self) -> Vec<(String, u32)> {
        let mut counts = std::collections::BTreeMap::<String, u32>::new();
        for &(id, n) in &self.fixed {
            if n > 0 {
                *counts.entry(id.to_owned()).or_default() += n as u32;
            }
        }
        for projectile in self.projectiles {
            *counts.entry(projectile.kind.id().to_owned()).or_default() += 1;
        }
        for mob in self.mobs.iter().filter(|mob| !mob.removed) {
            *counts.entry(mob.kind.id.clone()).or_default() += 1;
        }
        for row in self.general.iter() {
            *counts
                .entry(
                    super::general::general_type_catalog()
                        .key(row.type_slot)
                        .to_owned(),
                )
                .or_default() += 1;
        }
        for row in self.others {
            *counts.entry(row.kind.clone()).or_default() += 1;
        }
        counts.into_iter().collect()
    }
}

// Both Game and Parked own these typed families. Keep their instrumentation in one list.
macro_rules! entity_count_view {
    ($level:expr, $projectiles:expr, $general:expr, $knots:expr, $others:expr) => {
        crate::game::levels::EntityCountView {
            fixed: [
                ("minecraft:item", $level.entities.dropped.len()),
                ("minecraft:experience_orb", $level.orbs.len()),
                ("minecraft:minecart", $level.carts.len()),
                ("minecraft:boat", $level.boats.len()),
                ("minecraft:falling_block", $level.falling.len()),
                ("minecraft:painting", $level.paintings.len()),
                ("minecraft:item_frame", $level.item_frames.len()),
                ("minecraft:armor_stand", $level.armor_stands.len()),
                ("minecraft:trident", $level.tridents.len()),
                ("minecraft:firework_rocket", $level.rockets.len()),
                ("minecraft:tnt", $level.tnts.len()),
                ("minecraft:end_crystal", $level.crystals.len()),
                ("minecraft:leash_knot", $knots.len()),
                (
                    "minecraft:ominous_item_spawner",
                    $level.ominous_spawners.len(),
                ),
                ("minecraft:cushion", $level.cushions.len()),
                ("minecraft:area_effect_cloud", $level.breath_clouds.len()),
                ("minecraft:lightning_bolt", $level.bolts.len()),
                ("minecraft:evoker_fangs", $level.fangs.perf_count()),
                // Multipart hitboxes are not independently admitted world entities.
                (
                    "minecraft:ender_dragon",
                    usize::from($level.dragon.as_ref().is_some()),
                ),
            ],
            mobs: &$level.mobs,
            projectiles: $projectiles,
            general: $general,
            others: $others,
        }
    };
}
pub(in crate::game) use entity_count_view;

fn should_run_idle_maintenance(paused: bool) -> bool {
    !paused
}

/// Per-level chunk ticket levels and random-tick section cache.
#[derive(Default)]
pub struct ChunkTickingLevelState {
    pub tickets: ChunkTickets,
    pub(in crate::game::server_side) random_tick_sections: RandomTickSections,
}

/// Per-level fence knot ids and broken fence positions.
#[derive(Default)]
pub struct KnotLevelState {
    pub knot_ids: HashMap<[i32; 3], u32>,
    pub broken_knots: Vec<[i32; 3]>,
}

/// Per-level queued LAN block, entity, and critical-hit events.
#[derive(Default)]
pub struct LanEventLevelState {
    pub block_events: Vec<([i32; 3], i32, i32, u32)>,
    pub entity_events: Vec<(i32, i8)>,
    pub(in crate::game::server_side) critical_hits: Vec<(Option<u32>, i32, bool)>,
}

/// A level that is not in place: every per-level field of `Game`.
pub struct Parked {
    pub world: World,
    pub streaming: ChunkStreamer,
    end: EndLevelState,
    pub save: Arc<SaveDir>,
    block_ticks: Ticks,
    tick_chunks: TickChunkMembership,
    chunk_ticking: ChunkTickingLevelState,
    level_events: Vec<LevelEvent>,
    /// Entity portal processes and cooldowns belong to this level's entities.
    pub portal_states: PortalLevelState,
    falling: sunrise_sim::join_seq::JoinList<FallingBlock, false>,
    paintings: sunrise_sim::join_seq::JoinList<sunrise_sim::entities::Painting>,
    item_frames: sunrise_sim::join_seq::JoinList<sunrise_sim::entities::ItemFrame>,
    armor_stands: sunrise_sim::join_seq::JoinList<sunrise_sim::entities::ArmorStand>,
    cushions: sunrise_sim::join_seq::JoinList<sunrise_sim::entities::Cushion>,
    others: sunrise_sim::join_seq::ResidentList<OtherEntity>,
    fangs: sunrise_gameplay_common::game::fangs_common::FangState,
    pub boats: sunrise_sim::join_seq::JoinList<Boat, false>,
    pub carts: sunrise_sim::join_seq::JoinList<minecarts::Minecart, false>,
    bolts: sunrise_sim::join_seq::ResidentList<Bolt>,
    crystals: sunrise_sim::join_seq::JoinList<dragon_common::Crystal>,
    dragon: DragonSlot,
    breath_clouds: sunrise_sim::join_seq::ResidentList<dragon_common::Breath>,
    tridents: sunrise_sim::join_seq::JoinList<ThrownTrident>,
    rockets: sunrise_sim::join_seq::JoinList<fireworks::Rocket>,
    ominous_spawners: sunrise_sim::join_seq::ResidentList<OminousItemSpawner>,
    tnts: sunrise_sim::join_seq::JoinList<PrimedTnt>,
    general: super::general::GeneralStore,
    hoppers: HashMap<(i32, i32, i32), HopperState>,
    pub mobs: sunrise_sim::join_seq::JoinList<Mob>,
    knot_state: KnotLevelState,
    knots: sunrise_sim::join_seq::JoinList<Knot>,
    projectiles: sunrise_sim::join_seq::JoinList<sunrise_sim::projectiles::Projectile, false>,
    // Keep item retirement between projectiles and orbs when a parked level drops.
    entities: EntityStore,
    pub(in crate::game::server_side) orbs: sunrise_sim::join_seq::JoinList<ExperienceOrb>,
    persist: EntityMemory,
    lectern_pulses: Vec<((i32, i32, i32), i32)>,
    lid_openers: HashMap<(i32, i32, i32), i32>,
    lan_events: Vec<super::QueuedLevelEvent>,
    lan_queues: LanEventLevelState,
    /// The hub's tracked entities of this level (`LanHost::states`), as its last tick described
    /// them: what its players' moves are checked against before this tick describes them again.
    lan_states: sunrise_sim::lan::entity::States,
    /// Only the per-level parts of these are used (`swap_level`).
    anims: BlockEntityLevelTicks,
    pub raiding: RaidLevelState,
    chunk_spawn_enemies: bool,
    sculk: SculkLevelState,
    creaking: CreakingLevelState,
    /// Natural spawning's candidate cache: each level keeps its own players' squares.
    spawn_scratch: super::natural_spawning::SpawnScratch,
    /// The level's own world border (`ServerLevel.getWorldBorder`).
    pub(in crate::game) border: sunrise_level::worldborder::WorldBorder,
}

impl Parked {
    #[cfg(test)]
    pub(in crate::game::server_side) fn replace_save_for_test(&mut self, save: Arc<SaveDir>) {
        self.save = save;
    }

    /// The exact body restored by a cross-level transfer during natural-spawn admission.
    pub(in crate::game) fn natural_spawn_mob(
        &self,
        id: u32,
        join: sunrise_sim::join_seq::JoinSeq,
    ) -> Option<(&Mob, Option<mod_api::EntityTags<'_>>)> {
        let mob = if join.is_joined() {
            &self.mobs[self.mobs.locate(join)?]
        } else {
            self.persist.off_level.get(&id)?.as_mob()?
        };
        let tags = match self.persist.raw.get(&id).and_then(|tag| tag.get("Tags")) {
            Some(sunrise_core::nbt::Tag::List(tags)) => tags.as_slice(),
            _ => &[],
        };
        Some((
            mob,
            Some(mod_api::__private::game::borrow_entity_tags(tags)),
        ))
    }

    /// Whether `obstructs` returns `true` for any live `blocksBuilding` entity in this parked
    /// level (`EntityGetter.isUnobstructed`), without allocating.
    pub(super) fn any_building_blocker(
        &self,
        mut obstructs: impl FnMut(sunrise_core::geometry::Aabb) -> bool,
    ) -> bool {
        if self
            .mobs
            .iter()
            .any(|m| !m.removed && obstructs(m.bounds()))
            || self
                .dragon
                .as_ref()
                .is_some_and(|dragon| obstructs(dragon.body_bounds()))
        {
            return true;
        }
        let entities = EntityLists {
            mobs: &self.mobs,
            dropped: &self.entities.dropped,
            boats: &self.boats,
            carts: &self.carts,
            armor_stands: &self.armor_stands,
            tnts: &self.tnts,
            falling: &self.falling,
            orbs: &self.orbs,
            projectiles: &self.projectiles,
            rockets: &self.rockets,
            tridents: &self.tridents,
            general: &self.general.rows,
        };
        entities
            .non_mob_building_blocker_boxes(
                self.carts.iter().map(minecarts::Minecart::bounds),
                &self.crystals,
            )
            .any(obstructs)
    }

    resident_owners!(
        define_resident_access,
        self,
        projectiles,
        knots,
        general.rows,
        others
    );
}

impl crate::game::ServerGame<'_> {
    /// Visit exactly the actual physical lifetime slot. Resolution releases the
    /// table and its journal before lending storage; no lift, swap or replacement
    /// occurs, including when the visitor unwinds. Logical visibility is a caller
    /// policy, just as for the underlying owner resolver. The visitor must not
    /// call entity lookup or recursively enter this seam.
    pub fn with_resident_slot<R>(
        &mut self,
        dimension: &mod_api::DimensionKey,
        id: std::num::NonZeroU32,
        visit: impl FnOnce(&mut sunrise_mod_runtime::attachments::AttachmentLifetime) -> R,
    ) -> super::entity_residency::SlotAccess<R> {
        use super::entity_residency::SlotAccess;
        let resolved = {
            self.common
                .resident_entities
                .table
                .borrow_mut()
                .entity_lookup(dimension, id.get(), |at| {
                    self.locate_server_resident_owner(at)
                })
        };
        let location = match resolved {
            EntityLookup::Found { location, .. } => location,
            EntityLookup::Taken => return SlotAccess::Taken,
            EntityLookup::Missing => return SlotAccess::Missing,
        };
        debug_assert!(self.resident_entities.table.try_borrow_mut().is_ok());
        if matches!(
            location.owner.kind,
            sunrise_sim::join_seq::residency::Kind::Player
                | sunrise_sim::join_seq::residency::Kind::Hook
        ) {
            if *dimension != self.common.dim.key() {
                let dim = match *dimension {
                    DimensionKey::OVERWORLD => Dim::Overworld,
                    DimensionKey::NETHER => Dim::Nether,
                    DimensionKey::END => Dim::End,
                    key => Dim::Extra(key),
                };
                if !self.server.levels.0.parked.contains_key(&dim) {
                    return SlotAccess::Missing;
                }
            }
            return self.with_specialized_resident_slot(location, visit);
        }
        if *dimension == self.common.dim.key() {
            self.with_resident_owner_slot(location, visit)
        } else {
            let dim = match *dimension {
                mod_api::DimensionKey::OVERWORLD => Dim::Overworld,
                mod_api::DimensionKey::NETHER => Dim::Nether,
                mod_api::DimensionKey::END => Dim::End,
                key => Dim::Extra(key),
            };
            self.server
                .levels
                .0
                .parked
                .get_mut(&dim)
                .map_or(SlotAccess::Missing, |parked| {
                    parked.with_resident_owner_slot(location, visit)
                })
        }
    }

    /// Whether the living entity `id` resident in a parked level is alive (`Entity.isAlive`):
    /// a remembered target stays remembered while its holder is in another level.
    pub(in crate::game::server_side) fn parked_living_alive(&self, id: u32) -> bool {
        let EntityLookup::Found { location, value } = self.server_resident_entity(id) else {
            return false;
        };
        let Some(parked) = (self.server.levels.0.parked.iter())
            .find(|(dim, _)| dim.key() == location.owner.dimension)
            .map(|(_, parked)| parked)
        else {
            return false;
        };
        match value {
            Target::Mob(_) => {
                (parked.mobs.locate(location.join)).is_some_and(|i| parked.mobs[i].alive())
            }
            Target::Stand(_) => (parked.armor_stands.locate(location.join))
                .is_some_and(|i| parked.armor_stands[i].health > 0.0),
            Target::Dragon(id) => {
                (parked.dragon.as_ref()).is_some_and(|d| d.id == id && d.health > 0.0)
            }
            _ => false,
        }
    }

    /// `ServerLevel.getEntity(UUID)`: search this level's admitted entities, not unloaded
    /// saves or tracking entries. A departure can overlap a replacement with the same UUID.
    pub(in crate::game::server_side) fn level_target_by_uuid(
        &self,
        dim: Dim,
        uuid: [i32; 4],
    ) -> Option<Target> {
        let dimension = dim.key();
        let mut table = self.common.resident_entities.table.borrow_mut();
        let mut after = None;
        loop {
            let id = table.level_uuid_candidate(dimension, uuid, after)?;
            if let EntityLookup::Found {
                value: (target, true),
                ..
            } = table.entity_lookup(&dimension, id.get(), |at| {
                self.locate_server_resident_owner(at)
            }) && self.resident_source_target_visible(dimension, target)
            {
                return Some(target);
            }
            after = Some(id);
        }
    }

    /// `PersistentEntitySectionManager.isLoaded(UUID)` for the destination level in place,
    /// including the in-place player and any guests in this level, independently of command visibility.
    pub(in crate::game) fn is_loaded_uuid_in_level(&self, uuid: [i32; 4]) -> bool {
        if self.level_target_by_uuid(self.common.dim, uuid).is_some() {
            return true;
        }
        if self
            .common
            .resident_entities
            .local_player
            .location()
            .is_none()
            && self.server.lan_acting.is_none()
            && self.common.player.is_some()
            && self.common.level.player_uuid == Some(uuid)
        {
            return true;
        }
        self.server
            .lan
            .iter()
            .flat_map(|h| h.hub.guests.iter())
            .any(|g| {
                g.placed
                    && !g.released
                    && Dim::from_id(&g.dimension) == self.common.dim
                    && sunrise_sim::lan::playerdata::uuid_ints(g.uuid) == uuid
            })
    }

    /// Find the canonical level without swapping or probing physical families.
    pub(in crate::game) fn resident_target_dimension(&self, target: Target) -> Option<Dim> {
        let id = self.resident_target_id(target)?;
        let found = match self.server_resident_entity(id) {
            EntityLookup::Found { location, value }
                if value == target
                    || matches!(
                        (value, target),
                        (Target::Player, Target::Guest(_)) | (Target::Guest(_), Target::Player)
                    ) =>
            {
                location
            }
            _ => return None,
        };
        Some(match found.owner.dimension {
            DimensionKey::OVERWORLD => Dim::Overworld,
            DimensionKey::NETHER => Dim::Nether,
            DimensionKey::END => Dim::End,
            key => Dim::Extra(key),
        })
    }

    /// A new epoch invalidates every old producer before rebinding retained levels.
    pub fn reset_entity_residency(&mut self) {
        let table = self.common.resident_entities.table.get_mut();
        table.reset();
        let journal = table.producer();
        self.bind_resident_owners(
            &journal,
            self.common.dim.key(),
            self.common.is_logical_server(),
        );
        self.bind_specialized_resident_owners(&journal);
        for (dim, parked) in &mut self.server.levels.0.parked {
            parked.bind_resident_owners(&journal, dim.key(), true);
        }
    }
}

impl crate::game::ServerGame<'_> {
    /// Resolve through the exact owner, never by trying dimension swaps.
    pub(in crate::game) fn locate_server_resident_owner(
        &self,
        at: Location,
    ) -> Option<SourceEntity<(Target, bool)>> {
        if matches!(
            at.owner.kind,
            sunrise_sim::join_seq::residency::Kind::Player
                | sunrise_sim::join_seq::residency::Kind::Hook
        ) {
            return self
                .locate_specialized_resident_owner(at)
                .map(|mut source| {
                    source.value.1 &=
                        self.resident_source_target_visible(at.owner.dimension, source.value.0);
                    source
                });
        }
        if at.owner.dimension == self.common.dim.key() {
            // An open transaction's removal reads as gone already.
            return self.locate_resident_owner(at).map(|mut source| {
                source.value.1 &= !self.removal_pending(&source.value.0);
                source
            });
        }
        let dim = match at.owner.dimension {
            DimensionKey::OVERWORLD => Dim::Overworld,
            DimensionKey::NETHER => Dim::Nether,
            DimensionKey::END => Dim::End,
            key => Dim::Extra(key),
        };
        // The transactions stay with the game when a level is parked: its pending removals too.
        self.server
            .levels
            .0
            .parked
            .get(&dim)?
            .locate_resident_owner(at)
            .map(|mut source| {
                source.value.1 &= !self.removal_pending_in(dim, &source.value.0);
                source
            })
    }

    pub fn server_resident_entity(&self, id: u32) -> EntityLookup<Target> {
        let found = self
            .common
            .resident_entities
            .table
            .borrow_mut()
            .server_entity_lookup(id, |at| self.locate_server_resident_owner(at));
        match found {
            EntityLookup::Found {
                location,
                value: (value, true),
            } => EntityLookup::Found { location, value },
            EntityLookup::Found {
                value: (_, false), ..
            }
            | EntityLookup::Missing => EntityLookup::Missing,
            EntityLookup::Taken => EntityLookup::Taken,
        }
    }
}

/// Which passes of the server tick the level in place runs: the first
/// reads the network (`Hub::poll`), the last ends the tick for every player
/// (`Hub::end_tick`). A tick with one level is both. `running` is the
/// tick rate manager's answer for this server tick, asked once before the
/// levels (`MinecraftServer.tickServer`); `None` outside `tick_server`.
#[derive(Clone, Copy)]
pub struct Pass {
    pub first: bool,
    pub last: bool,
    pub running: Option<bool>,
    /// Set once this level's player packets were handled before block ticks.
    pub packets_drained: bool,
}

impl Default for Pass {
    fn default() -> Self {
        Self {
            first: true,
            last: true,
            running: None,
            packets_drained: false,
        }
    }
}

pub(in crate::game) enum ServerTickPhase {
    Level(usize, Dim),
    IdleLevel(Dim),
    Connections,
}

/// `MinecraftServer.tickChildren`: finish the level phase before ticking connections.
pub(in crate::game) fn server_tick_phases<'a>(
    dimensions: &'a [Dim],
    active: &'a [Dim],
) -> impl Iterator<Item = ServerTickPhase> + 'a {
    let mut active = active.iter().copied().enumerate().peekable();
    dimensions
        .iter()
        .copied()
        .map(move |dim| match active.next_if(|(_, next)| *next == dim) {
            Some((index, _)) => ServerTickPhase::Level(index, dim),
            None => ServerTickPhase::IdleLevel(dim),
        })
        .chain(std::iter::once(ServerTickPhase::Connections))
}

/// How long a server tick took and what it spent the time in.
pub struct TickTiming {
    pub wall_ms: f32,
    pub cpu_ms: Option<f32>,
    /// Spans recorded inside the tick only, in observation order.
    pub raw: Vec<sunrise_level::spans::Span>,
}

/// The levels not in place, by dimension.
#[derive(Default)]
pub struct Levels {
    pub parked: HashMap<Dim, Box<Parked>>,
    order: Vec<Dim>,
    tick_order: Vec<Dim>,
    incarnations: Vec<LevelIncarnation>,
    reset_generation: u64,
    pub(in crate::game) active_closed: bool,
    /// The levels a mod asked to have saved (`level_request_save`), each once, saved at the start
    /// of the next server tick.
    pub requested_saves: Vec<Dim>,
    /// The levels mods opened while the server runs, and the requests waiting to be applied.
    pub runtime: super::runtime_levels::RuntimeTable,
    /// The entities that travelled to another level lately: the id each had, to the id it
    /// arrived with and when. A kill credit recorded under the old id follows it there.
    travels: HashMap<u32, (u32, i64)>,
}

struct LevelIncarnation {
    dim: Dim,
    generation: u64,
    open: bool,
}

impl Levels {
    #[inline(always)]
    pub(in crate::game) fn incarnation(&self, dim: Dim) -> Option<(u64, u64)> {
        let generation = self
            .incarnations
            .iter()
            .find(|level| level.dim == dim)
            .map_or(Some(0), |level| level.open.then_some(level.generation))?;
        Some((self.reset_generation, generation))
    }

    #[inline(always)]
    pub(in crate::game) fn open_incarnation(&mut self, dim: Dim) {
        if let Some(level) = self.incarnations.iter_mut().find(|level| level.dim == dim) {
            level.generation += 1;
            level.open = true;
        } else {
            self.incarnations.push(LevelIncarnation {
                dim,
                generation: 1,
                open: true,
            });
        }
    }

    #[inline(always)]
    fn close_incarnation(&mut self, dim: Dim) {
        if let Some(level) = self.incarnations.iter_mut().find(|level| level.dim == dim) {
            level.open = false;
        } else {
            self.incarnations.push(LevelIncarnation {
                dim,
                generation: 0,
                open: false,
            });
        }
    }

    pub(in crate::game) fn parked_target_visible(&self, dim: Dim, target: &Target) -> Option<bool> {
        let parked = self.parked.get(&dim)?;
        Some(
            !parked.persist.removals.contains(target)
                && !parked.persist.destroying_vehicles.contains(target),
        )
    }

    /// Note that the entity `from` arrived in another level as `to` at `now`. A kill credit
    /// lasts a hundred ticks, so a travel older than that names nothing still credited.
    pub(in crate::game::server_side) fn note_travel(&mut self, from: u32, to: u32, now: i64) {
        self.travels.retain(|_, &mut (_, at)| now - at <= 100);
        self.travels.insert(from, (to, now));
    }

    /// The id the entity `from` arrived with in another level, if it travelled lately.
    pub(in crate::game::server_side) fn travelled(&self, from: u32) -> Option<u32> {
        self.travels.get(&from).map(|&(to, _)| to)
    }

    /// Queue `dim` to be saved at the start of the next server tick; a level already queued is
    /// not queued twice.
    pub fn request_save(&mut self, dim: Dim) {
        if !self.requested_saves.contains(&dim) {
            self.requested_saves.push(dim);
        }
    }

    /// Whether a save of `dim` is still waiting for a tick that admits
    /// saves (the #2488 gate holds it back).
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn save_pending(&self, dim: Dim) -> bool {
        self.requested_saves.contains(&dim)
    }

    /// Drop a save of `dim` still waiting for a tick: its level is gone.
    pub(in crate::game::server_side) fn cancel_requested_save(&mut self, dim: Dim) {
        self.requested_saves.retain(|saved| *saved != dim);
    }

    /// Whether mods have already been notified of `dim` opening.
    pub fn has_loaded_levels(&self) -> bool {
        !self.order.is_empty()
    }

    /// Clear every parked level and dimensions recorded as heard open.
    pub fn clear(&mut self) {
        self.parked.clear();
        self.order.clear();
        self.reset_generation += 1;
        self.incarnations.clear();
        self.tick_order.clear();
        self.active_closed = false;
        self.requested_saves.clear();
        self.runtime.clear();
    }

    #[cfg(any(test, feature = "test-support"))]
    #[inline(always)]
    pub fn reset_active_closed_for_test(&mut self, active: Dim) {
        self.active_closed = false;
        if self.incarnation(active).is_none() {
            self.open_incarnation(active);
        }
    }

    pub(in crate::game::server_side) fn abort_parked_generation(&mut self) {
        for level in self.parked.values_mut() {
            level.streaming.abort_generation();
        }
    }

    pub(in crate::game::server_side) fn parked_save_dir(&self, dim: Dim) -> Option<&Arc<SaveDir>> {
        self.parked.get(&dim).map(|level| &level.save)
    }

    /// A parked level, without opening or loading it.
    pub fn parked_world(&self, dim: Dim) -> Option<&World> {
        self.parked.get(&dim).map(|level| &level.world)
    }

    /// The world of the level not in place whose dimension is `dimension`
    /// (`minecraft:the_nether`), found without collecting the parked levels.
    pub fn parked_world_of(&self, dimension: &str) -> Option<&World> {
        self.parked
            .iter()
            .find(|(dim, _)| dim.id() == dimension)
            .map(|(_, level)| &level.world)
    }

    /// Block ticks belonging to an open level that is not currently in place.
    pub(in crate::game) fn parked_block_ticks_of(&self, dimension: &str) -> Option<&Ticks> {
        self.parked
            .iter()
            .find(|(dim, _)| dim.id() == dimension)
            .map(|(_, level)| &level.block_ticks)
    }

    pub(in crate::game) fn parked_lid_openers_of(
        &self,
        dimension: &str,
    ) -> Option<&HashMap<(i32, i32, i32), i32>> {
        self.parked_of(dimension).map(|level| &level.lid_openers)
    }

    /// The world of the level not in place whose dimension is `dimension`, to
    /// change.
    pub fn parked_world_mut_of(&mut self, dimension: &str) -> Option<&mut World> {
        self.parked
            .iter_mut()
            .find(|(dim, _)| dim.id() == dimension)
            .map(|(_, level)| &mut level.world)
    }

    /// The streaming of the level not in place whose dimension is `dimension`.
    pub fn parked_streaming_of(&self, dimension: &str) -> Option<&ChunkStreamer> {
        self.parked
            .iter()
            .find(|(dim, _)| dim.id() == dimension)
            .map(|(_, level)| &level.streaming)
    }

    /// A parked level's existing generator query worker, without switching levels.
    pub(in crate::game::server_side) fn parked_locator_of(
        &self,
        dimension: &str,
    ) -> Option<Arc<VanillaTerrain>> {
        self.parked_of(dimension)?.end.locator.clone()
    }

    /// The streaming of the level not in place whose dimension is `dimension`, to change.
    pub(in crate::game::server_side) fn parked_streaming_of_mut(
        &mut self,
        dimension: &str,
    ) -> Option<&mut ChunkStreamer> {
        self.parked
            .iter_mut()
            .find(|(dim, _)| dim.id() == dimension)
            .map(|(_, level)| &mut level.streaming)
    }

    /// `ServerChunkCache.tick` of the parked levels that `tick` names: their `UNKNOWN` tickets
    /// purged (`TicketStorage.purgeStaleTickets`) when `purge`, then their recent FULL lookups
    /// forgotten (`clearCache`).
    pub(in crate::game::server_side) fn tick_parked_chunk_caches(
        &mut self,
        purge: bool,
        tick: impl Fn(Dim) -> bool,
    ) {
        for (&dim, level) in &mut self.parked {
            if tick(dim) {
                if purge {
                    level.streaming.purge_unknown_tickets();
                }
                level.streaming.clear_full_lookups();
            }
        }
    }

    /// `ServerLevel.setChunkForced`: force a column in an already open level without changing the acting
    /// player's level; the same queue and load barrier serve the active level.
    pub(in crate::game::server_side) fn force_parked_chunk(
        &mut self,
        dimension: &str,
        x: i32,
        z: i32,
        forced: bool,
    ) -> Option<bool> {
        let level = self.parked_mut_of(dimension)?;
        Some(
            level
                .streaming
                .set_forced_loaded(&mut level.world, x, z, forced),
        )
    }

    pub(in crate::game) fn parked_anims(&self, dim: Dim) -> Option<&BlockEntityLevelTicks> {
        self.parked.get(&dim).map(|level| &level.anims)
    }

    /// The worlds of the levels not in place.
    /// A parked level's world border.
    pub fn parked_border(&self, dim: Dim) -> Option<&sunrise_level::worldborder::WorldBorder> {
        self.parked.get(&dim).map(|level| &level.border)
    }

    pub(in crate::game::server_side) fn install_profession_save_snapshots(
        &self,
        table: Arc<sunrise_sim::mobs::villager::ResolvedProfessions>,
    ) {
        for level in self.parked.values() {
            level.save.install_profession_snapshot(table.clone());
        }
    }

    /// The generators of the parked levels, for the world snapshots their workers read.
    pub(in crate::game::server_side) fn parked_terrains(
        &self,
    ) -> impl Iterator<Item = &Arc<VanillaTerrain>> {
        self.parked
            .values()
            .filter_map(|level| level.end.locator.as_ref())
    }

    pub(in crate::game::server_side) fn parked_worlds(&self) -> impl Iterator<Item = &World> {
        self.parked.values().map(|level| &level.world)
    }

    /// The streamers of the levels not in place.
    pub(in crate::game::server_side) fn parked_streamings(
        &self,
    ) -> impl Iterator<Item = &ChunkStreamer> {
        self.parked.values().map(|level| &level.streaming)
    }

    /// The level not in place, in `dim`.
    pub fn parked_mut(&mut self, dim: Dim) -> Option<&mut Parked> {
        self.parked.get_mut(&dim).map(Box::as_mut)
    }

    /// A parked level by its exact dimension identity, without swapping it in.
    pub fn parked_of(&self, dimension: &str) -> Option<&Parked> {
        self.parked
            .iter()
            .find(|(dim, _)| dim.id() == dimension)
            .map(|(_, level)| level.as_ref())
    }
    pub(in crate::game::server_side) fn parked_mut_of(
        &mut self,
        dimension: &str,
    ) -> Option<&mut Parked> {
        self.parked
            .iter_mut()
            .find(|(dim, _)| dim.id() == dimension)
            .map(|(_, level)| level.as_mut())
    }

    /// Whether a parked level's general rows hold any of these UUIDs (#792's
    /// dimension-transfer check: a destination may not already hold a row
    /// of the travelling tree's UUIDs).
    pub(in crate::game::server_side) fn parked_general_holds_any_uuid(
        &self,
        dim: Dim,
        uuids: &std::collections::HashSet<[i32; 4]>,
    ) -> bool {
        self.parked.get(&dim).is_some_and(|parked| {
            parked
                .general
                .rows
                .iter()
                .any(|row| uuids.contains(&row.uuid))
        })
    }

    /// The entities of a parked level, counted as `Game::entity_count` counts the one in place.
    pub(in crate::game::server_side) fn parked_entity_count(&self, dim: Dim) -> Option<usize> {
        let l = self.parked.get(&dim)?;
        Some(
            l.mobs.iter().filter(|m| !m.removed).count()
                + l.entities.dropped.len()
                + l.orbs.len()
                + l.projectiles.len()
                + l.carts.len()
                + l.boats.len()
                + l.falling.len()
                + l.paintings.len()
                + l.item_frames.len()
                + l.armor_stands.len()
                + l.cushions.len()
                + l.tridents.len()
                + l.rockets.len()
                + l.tnts.len(),
        )
    }

    /// The mobs of a parked level.
    pub(in crate::game::server_side) fn parked_mobs(&self, dim: Dim) -> Option<&[Mob]> {
        self.parked.get(&dim).map(|level| &*level.mobs)
    }

    pub(in crate::game::server_side) fn parked_perf_views(
        &self,
    ) -> impl Iterator<Item = (Dim, &World, EntityCountView<'_>)> {
        self.parked.iter().map(|(&dim, level)| {
            (
                dim,
                &level.world,
                entity_count_view!(
                    level,
                    &level.projectiles,
                    &level.general.rows,
                    &level.knots,
                    &level.others
                ),
            )
        })
    }

    /// A parked level's loose entities, for `sunrise-report`: items, experience orbs,
    /// projectiles, minecarts and boats.
    pub(in crate::game::server_side) fn parked_entity_counts(
        &self,
        dim: Dim,
    ) -> Option<[usize; 5]> {
        self.parked.get(&dim).map(|l| {
            [
                l.entities.dropped.len(),
                l.orbs.len(),
                l.projectiles.len(),
                l.carts.len(),
                l.boats.len(),
            ]
        })
    }

    /// The levels that are not in place, by dimension.
    pub(in crate::game::server_side) fn parked_dims(&self) -> Vec<Dim> {
        self.parked.keys().copied().collect()
    }
}

/// Build a parked level out of line so `open_level` keeps its stack small while
/// `make_streaming` decodes worldgen density graphs.
#[inline(never)]
fn box_parked(
    world: World,
    streaming: ChunkStreamer,
    locator: Option<Arc<VanillaTerrain>>,
    save: Arc<SaveDir>,
    seed: i64,
    border: sunrise_level::worldborder::WorldBorder,
) -> Box<Parked> {
    Box::new(Parked {
        world,
        streaming,
        end: EndLevelState::new(locator),
        save,
        block_ticks: Ticks::new(seed as i32),
        tick_chunks: Default::default(),
        chunk_ticking: Default::default(),
        level_events: Vec::new(),
        portal_states: Default::default(),
        falling: Default::default(),
        paintings: Default::default(),
        item_frames: Default::default(),
        armor_stands: Default::default(),
        cushions: Default::default(),
        others: Default::default(),
        fangs: Default::default(),
        boats: Default::default(),
        carts: Default::default(),
        bolts: Default::default(),
        crystals: Default::default(),
        dragon: Default::default(),
        breath_clouds: Default::default(),
        tridents: Default::default(),
        rockets: Default::default(),
        ominous_spawners: Default::default(),
        tnts: Default::default(),
        general: Default::default(),
        hoppers: HashMap::new(),
        chunk_spawn_enemies: true,
        mobs: Default::default(),
        knot_state: Default::default(),
        knots: Default::default(),
        projectiles: Default::default(),
        entities: {
            let dropped = Default::default();
            EntityStore { dropped }
        },
        orbs: Default::default(),
        persist: Default::default(),
        lectern_pulses: Vec::new(),
        lid_openers: HashMap::new(),
        lan_events: Vec::new(),
        lan_queues: Default::default(),
        lan_states: Default::default(),
        anims: Default::default(),
        raiding: Default::default(),
        // Run SculkState's HashSet defaults in order so thread-local RandomState stays aligned.
        sculk: SculkState::default().level,
        creaking: Default::default(),
        spawn_scratch: Default::default(),
        border,
    })
}

impl crate::game::ServerGame<'_> {
    /// A new level for `dim`, opened from the save (`MinecraftServer.createLevels`).
    pub fn open_level(&self, dim: Dim) -> Result<Box<Parked>> {
        // A level a mod opened has the seed and generator it asked for.
        let (generate, seed) = match self.runtime_terrain(dim) {
            Some((generate, seed)) => (Some(generate), seed),
            None => (None, self.common.level.seed),
        };
        let biomes = self
            .common
            .world_biomes
            .as_ref()
            .unwrap_or_else(|| self.common.res.biomes());
        let overrides = match &self.server.world_tables.0 {
            Some(tables) => tables.biomes.overrides.clone(),
            None => {
                let base_conditions = self.common.worldgen_base_conditions()?;
                let mut enabled: Vec<_> = self.server.fns.0.enabled_packs_highest_first().collect();
                enabled.reverse();
                let conditions =
                    sunrise_gameplay_common::game::load_conditions::worldgen_load_conditions(
                        &base_conditions,
                        &self.common.options.resources,
                        &enabled,
                    );
                super::world_loading::worldgen_overrides_of(&self.server.fns.0.data, &conditions)?.0
            }
        };
        let (streaming, save, locator, generator_type) = super::world_loading::make_streaming(
            &self.common.res,
            &self.common.pack,
            &self.common.options,
            &self.server.settings,
            &self.server.world_root,
            dim,
            self.server.level_shapes.get(dim.id()),
            seed,
            &self.common.level,
            biomes.clone(),
            &overrides,
            generate,
            self.server.save.0.chunk_reports().clone(),
            self.server.save.0.save_failure_reporter().clone(),
        )?;
        self.bind_chunk_disk_worker(&save, dim);
        save.install_profession_snapshot(self.stations().professions.clone());
        if let Some(terrain) = &locator {
            terrain.set_mob_maps(self.mob_map_snapshot());
            terrain.set_biome_keys(&self.biome_key_snapshot());
        }
        let mut world =
            sunrise_gameplay_common::game::authoritative_world_for_mods(&self.common.res, seed);
        world.attachments = save.load_attachments();
        world.shape = self.server.level_shapes.get(dim.id());
        world
            .weather
            .copy_parameters_from(self.common.world.weather);
        if world.shape.can_have_weather(dim == Dim::End) && world.weather.raining {
            world.weather.rain_level.1 = 1.0;
            if world.weather.thundering {
                world.weather.thunder_level.1 = 1.0;
            }
        }
        world.generator_type = Some(generator_type);
        world.fast_lava = self.common.dimension_type_of(dim).fast_lava;
        world.infiniburn = Some(self.resolve_level_infiniburn(self.common.dimension_type_of(dim)));
        world.golem_heads = Some(self.resolve_golem_heads());
        let border = self.load_border(dim);
        Ok(box_parked(world, streaming, locator, save, seed, border))
    }

    /// Every loaded server level in `MinecraftServer.levels` creation order.
    pub(in crate::game) fn loaded_level_order(&self) -> Vec<Dim> {
        self.level_creation_order().into_owned()
    }

    fn level_creation_order(&self) -> Cow<'_, [Dim]> {
        #[cfg(any(test, feature = "test-support"))]
        if self.server.levels.0.order.len()
            != self.server.levels.0.parked.len() + usize::from(!self.server.levels.0.active_closed)
        {
            // Isolated Game fixtures can populate level storage without running createLevels.
            let mut dims = self.server.levels.0.order.clone();
            let mut unannounced: Vec<_> = (!self.server.levels.0.active_closed)
                .then_some(self.common.dim)
                .into_iter()
                .chain(self.server.levels.0.parked.keys().copied())
                .filter(|dim| !dims.contains(dim))
                .collect();
            unannounced.sort_by_key(|dim| dim_fallback_key(*dim));
            dims.extend(unannounced);
            return Cow::Owned(dims);
        }
        Cow::Borrowed(&self.server.levels.0.order)
    }

    /// Open configured levels in registry order before restoring their saved tickets.
    pub fn open_configured_levels(&mut self, configured: &[Dim]) -> Result<()> {
        if self.common.options.execution_role.is_client() {
            return Ok(());
        }
        let starting_dim = self.common.dim;
        // World preparation can build the owner's starting level before the
        // Overworld. Keep it out of both the current and parked slots until
        // its registry turn, so earlier `level_load` listeners cannot see it.
        let mut starting_level = if starting_dim != Dim::Overworld {
            let mut overworld = self.open_level(Dim::Overworld)?;
            self.replace_level(Dim::Overworld, &mut overworld);
            self.server.streaming.radius =
                sunrise_gameplay_common::game::streaming_radius::streaming_radius_for(
                    self.server.settings.render_distance,
                    None,
                );
            self.read_raids();
            Some(overworld)
        } else {
            None
        };
        self.native_level_opened(Dim::Overworld);
        self.initialize_overworld_spawn()?;
        for dim in configured
            .iter()
            .copied()
            .filter(|&dim| dim != Dim::Overworld)
        {
            if dim == starting_dim
                && let Some(level) = starting_level.take()
            {
                self.server.levels.0.parked.insert(dim, level);
                self.enter_level(dim)?;
            } else {
                let mut level = self.open_level(dim)?;
                let left = self.replace_level(dim, &mut level);
                self.server.levels.0.parked.insert(left, level);
                self.server.streaming.radius =
                    sunrise_gameplay_common::game::streaming_radius::streaming_radius_for(
                        self.server.settings.render_distance,
                        None,
                    );
                self.read_raids();
            }
            self.native_level_opened(dim);
        }
        self.enter_level(starting_dim)?;
        Ok(())
    }

    pub(in crate::game) fn update_mob_spawning_flags(&mut self) {
        use sunrise_game_data::gamerules::vanilla::{SPAWN_MOBS, SPAWN_MONSTERS};

        let rules = &self.common.ui.rules;
        let spawn_enemies = rules.bool_at(SPAWN_MOBS) && rules.bool_at(SPAWN_MONSTERS);
        self.server.chunk_spawn_enemies = spawn_enemies;
        for level in self.server.levels.0.parked.values_mut() {
            level.chunk_spawn_enemies = spawn_enemies;
        }
    }

    /// Restore saved tickets for all published levels and wait for their chunks before the first tick.
    pub fn prepare_levels(&mut self) -> Result<()> {
        if self.common.options.execution_role.is_client() {
            return Ok(());
        }
        let back = self.common.dim;
        let dims = self.loaded_level_order();
        for &dim in &dims {
            self.enter_level(dim)?;
            Self::load_saved_level_tickets(
                &self.server.save.0,
                &mut self.server.streaming,
                &mut self.server.chunk_ticking.tickets,
            );
            self.validate_mod_chunk_tickets(dim);
        }
        self.server.startup_ticket_restore_pending = false;
        let deadline = Instant::now() + Duration::from_secs(600);
        for dim in dims {
            self.enter_level(dim)?;
            self.wait_for_ticket_chunks(deadline);
        }
        self.enter_level(back)?;
        self.update_mob_spawning_flags();
        Ok(())
    }

    /// `TicketStorage.activateAllDeactivatedTickets`: restore portal and
    /// command tickets, and stage mod tickets for their controllers to validate.
    fn load_saved_level_tickets(
        save: &SaveDir,
        streaming: &mut ChunkStreamer,
        tickets: &mut ChunkTickets,
    ) {
        tickets.restore_portal(save.load_portal_tickets());
        streaming.load_forced(
            save.load_forced_chunks(),
            save.load_mod_tickets(mod_api::__private::game::ticket_controller_slot_by_id),
        );
    }

    /// A level that opens after startup restores its saved command and portal
    /// tickets at once. Mod tickets still wait for validation after `level_load`.
    pub fn open_runtime_level(&self, dim: Dim) -> Result<Box<Parked>> {
        let mut level = self.open_level(dim)?;
        Self::load_saved_level_tickets(
            &level.save,
            &mut level.streaming,
            &mut level.chunk_ticking.tickets,
        );
        Ok(level)
    }

    /// Stream the level in place until each chunk its tickets make block
    /// ticking is a ticking chunk (lit, with its neighbours loaded).
    fn wait_for_ticket_chunks(&mut self, deadline: Instant) {
        // Forced chunks are checked two chunks out (their ticket block ticks one).
        // A `PORTAL` ticket's chunks are streamed as an area of their own (the
        // network tick does it, which hasn't run yet), whose chunks out to its
        // radius can become ticking chunks: those are waited for.
        let portal = self.portal_ticket_areas();
        for area in &portal {
            if !self.server.streaming.extra.contains(area) {
                self.server.streaming.extra.push(*area);
            }
        }
        loop {
            self.update_chunk_tickets();
            let levels = self.server.chunk_ticking.tickets.levels();
            let streaming = &self.server.streaming;
            let ready = streaming
                .forced
                .iter()
                .map(|&c| (c, 2))
                .chain(portal.iter().copied())
                .all(|((cx, cz), reach)| {
                    (cz - reach..=cz + reach).all(|z| {
                        (cx - reach..=cx + reach).all(|x| {
                            !levels.loaded_ticking(x, z)
                                || super::ticking_chunk(&self.common.world, x, z)
                        })
                    })
                });
            if ready {
                return;
            }
            if Instant::now() >= deadline {
                log::warn!("Sunrise: the chunks of the saved tickets did not load");
                return;
            }
            self.server_update_world();
            self.server.streaming.wait(Duration::from_millis(5));
        }
    }

    /// Exchange the level in place with `p`.
    fn swap_level(&mut self, p: &mut Parked) {
        use std::mem::swap;
        swap(&mut self.common.world, &mut p.world);
        // The carrier first-use gate belongs to the world, not the
        // level: a level swap keeps it.
        self.common.world.carrier_data_existed_at_open = p.world.carrier_data_existed_at_open;
        swap(&mut self.server.streaming, &mut p.streaming);
        self.common.end.swap_level(&mut p.end);
        self.share_spawn_conditions();
        swap(&mut self.server.save.0, &mut p.save);
        swap(&mut self.common.block_ticks, &mut p.block_ticks);
        swap(&mut self.server.tick_chunks, &mut p.tick_chunks);
        swap(&mut self.server.chunk_ticking, &mut p.chunk_ticking);
        swap(&mut self.common.level_events, &mut p.level_events);
        self.server.portals.swap_level(&mut p.portal_states);
        swap(&mut self.common.entities, &mut p.entities);
        swap(&mut self.common.falling, &mut p.falling);
        swap(&mut self.common.paintings, &mut p.paintings);
        swap(&mut self.common.item_frames, &mut p.item_frames);
        swap(&mut self.common.armor_stands, &mut p.armor_stands);
        swap(&mut self.common.cushions, &mut p.cushions);
        swap(&mut self.server.others.0, &mut p.others);
        swap(&mut self.common.fangs, &mut p.fangs);
        swap(&mut self.common.boats, &mut p.boats);
        swap(&mut self.common.carts, &mut p.carts);
        swap(&mut self.common.bolts, &mut p.bolts);
        swap(&mut self.common.crystals, &mut p.crystals);
        swap(&mut self.common.dragon, &mut p.dragon);
        swap(&mut self.common.breath_clouds, &mut p.breath_clouds);
        swap(&mut self.common.tridents, &mut p.tridents);
        swap(&mut self.common.rockets, &mut p.rockets);
        swap(&mut self.common.ominous_spawners, &mut p.ominous_spawners);
        swap(&mut self.common.tnts, &mut p.tnts);
        swap(&mut self.server.hoppers, &mut p.hoppers);
        swap(&mut self.common.mobs, &mut p.mobs);
        swap(&mut self.server.knot_state, &mut p.knot_state);
        swap(&mut self.common.resident_entities.knots, &mut p.knots);
        swap(&mut self.common.resident_entities.general, &mut p.general);
        swap(&mut self.common.creatures.projectiles, &mut p.projectiles);
        swap(&mut self.common.orbs, &mut p.orbs);
        swap(&mut self.common.persist, &mut p.persist);
        swap(&mut self.server.lectern_pulses, &mut p.lectern_pulses);
        swap(&mut self.server.lid_openers, &mut p.lid_openers);
        swap(&mut self.server.lan_events, &mut p.lan_events);
        swap(&mut self.server.lan_queues, &mut p.lan_queues);
        self.server.be_anims.swap_level(&mut p.anims);
        self.server.raiding.0.swap_level(&mut p.raiding);
        self.server.sculk.swap_level(&mut p.sculk);
        self.server.creaking.swap_level(&mut p.creaking);
        swap(
            &mut self.server.chunk_spawn_enemies,
            &mut p.chunk_spawn_enemies,
        );
        swap(&mut self.server.spawn_scratch, &mut p.spawn_scratch);
        swap(&mut self.common.border, &mut p.border);
    }

    /// Swap the level in place without publishing the one just left.
    fn replace_level(&mut self, target: Dim, p: &mut Parked) -> Dim {
        p.world.game_time = self.common.world.game_time;
        // The level's day time is its default clock's; a level without one follows the
        // Overworld's.
        let clock = self.common.level_clock_name(target);
        p.world.day_time = self.common.switch_clock(&clock);
        p.world.end_clock = self.common.world.end_clock;
        p.world
            .weather
            .copy_parameters_from(self.common.world.weather);
        // The game rules and the difficulty are the server's, not a
        // level's: natural regeneration holds in every dimension.
        p.world.health_rules = self.common.world.health_rules;
        // The sculk tables' watched blocks are built once per game.
        p.world.watched = std::mem::take(&mut self.common.world.watched);
        self.swap_level(p);
        // The tracked entities go with their level: a level's moves, read before its tick
        // describes its entities again, would otherwise meet the last level's (a move
        // missed the boat under a player while anyone was in another dimension).
        if let Some(h) = self.server.lan.as_mut() {
            std::mem::swap(&mut h.states, &mut p.lan_states);
        }
        std::mem::replace(&mut self.common.dim, target)
    }

    /// Activate the target level and park the previous one while keeping shared clocks and weather parameters.
    pub fn enter_level(&mut self, target: Dim) -> Result<()> {
        // A client holds no levels.
        if self.common.options.execution_role.is_client() {
            return Ok(());
        }
        if target != self.common.dim {
            let (mut p, opened) = match self.server.levels.0.parked.remove(&target) {
                Some(p) => (p, false),
                None => (self.open_runtime_level(target)?, true),
            };
            let left = self.replace_level(target, &mut p);
            self.server.levels.0.parked.insert(left, p);
            if opened {
                self.server.streaming.radius =
                    sunrise_gameplay_common::game::streaming_radius::streaming_radius_for(
                        self.server.settings.render_distance,
                        None,
                    );
                // ServerLevel's constructor reads the level's raids.
                self.read_raids();
                self.native_level_opened(target);
                self.validate_mod_chunk_tickets(target);
            }
        }
        if let Some(h) = self.server.lan.as_mut() {
            h.hub.set_dimension(target.id());
        }
        self.sync_hub_clocks();
        Ok(())
    }

    /// Follow the direct mount of a travelling player through complete source objects.
    /// A later player in the same travelling stack finds no source root: the first
    /// arrival already moved it, keeping every id that the guests' mounts name.
    pub(in crate::game::server_side) fn parked_vehicle_root(
        &self,
        from: Dim,
        mut id: u32,
    ) -> Option<u32> {
        let p = self.server.levels.0.parked.get(&from)?;
        for _ in 0..=p.mobs.len() + p.persist.off_level.len() + 1 {
            let mob = p
                .mobs
                .iter()
                .find(|m| m.id == id)
                .or_else(|| p.persist.off_level.get(&id).and_then(OffLevel::as_mob))
                .filter(|m| !m.removed);
            if let Some(parent) = mob.and_then(|m| m.extra.mount) {
                id = parent;
                continue;
            }
            let present = mob.is_some()
                || p.boats.iter().any(|b| b.id == id)
                || p.carts.iter().any(|c| c.id == id);
            let removed = p
                .persist
                .removals
                .iter()
                .chain(p.persist.destroying_vehicles.iter())
                .any(|target| {
                    matches!(
                        target, Target::Mob(n) | Target::Boat(n) | Target::Cart(n) if *n == id
                    )
                });
            return (present && !removed).then_some(id);
        }
        None
    }

    /// Move the source tree's object memory and take its Mob descendants in
    /// canonical postorder. The caller owns the extracted root and admits it last.
    /// Transfer removal preserves lifetimes; residency is decided in the destination.
    pub(in crate::game::server_side) fn move_entity_memory(
        &mut self,
        from: Dim,
        root: u32,
    ) -> Vec<Mob> {
        let Some(p) = self.server.levels.0.parked.get_mut(&from) else {
            return Vec::new();
        };
        let mut edges: Vec<_> = p
            .mobs
            .iter()
            .chain(p.persist.off_level.values().filter_map(OffLevel::as_mob))
            .filter(|mob| !mob.removed)
            .map(|mob| (mob.extra.mount, mob.extra.vehicle_order, mob.id))
            .collect();
        edges.sort_by_key(|&(_, order, id)| (order, id));
        let mut children: HashMap<u32, Vec<u32>> = HashMap::new();
        for (parent, _, id) in edges {
            if let Some(parent) = parent {
                children.entry(parent).or_default().push(id);
            }
        }
        let mut pending = vec![(root, false)];
        let mut seen = std::collections::HashSet::new();
        let mut ordered = Vec::new();
        while let Some((id, done)) = pending.pop() {
            if done {
                ordered.push(id);
            } else if seen.insert(id) {
                pending.push((id, true));
                if let Some(riders) = children.get(&id) {
                    pending.extend(riders.iter().rev().map(|&rider| (rider, false)));
                }
            }
        }
        let mut moved = Vec::new();
        for id in ordered {
            p.persist.move_object_to(id, &mut self.common.persist);
            if id == root {
                continue;
            }
            if let Some(state) = p.portal_states.entities.remove(&id) {
                self.server.portals.level.entities.insert(id, state);
            }
            let mob = if let Some(index) = p.mobs.iter().position(|m| m.id == id) {
                Some(p.mobs.remove_for_transfer(index))
            } else {
                // Boats and carts are only roots, which the caller takes.
                match p.persist.off_level.remove(&id) {
                    Some(OffLevel::Mob(mob)) => Some(mob),
                    _ => None,
                }
            };
            if let Some(mut mob) = mob {
                mob.join = sunrise_sim::join_seq::JoinSeq::UNJOINED;
                moved.push(mob);
            }
        }
        moved
    }

    /// An already-owned body changes level, never construction identity.
    /// `PersistentEntitySectionManager.addEntityUuid` decides its residency:
    /// a body whose UUID an earlier arrival took is retained off-level.
    pub(in crate::game::server_side) fn admit_transferred_object(&mut self, mut object: OffLevel) {
        let id = match &mut object {
            OffLevel::Mob(mob) => {
                self.rebind_transferred_entity(mob.id, mob);
                mob.id
            }
            OffLevel::Boat(boat) => {
                self.rebind_transferred_entity(boat.id, boat);
                boat.id
            }
            OffLevel::Cart(cart) => {
                self.rebind_transferred_entity(cart.id, cart);
                cart.id
            }
        };
        self.common.journal_arrival(id);
        let level = self.common.dim;
        self.common.entity_attachments.move_collider(id, level);
        if self.uuid_admission_refused(self.common.persist.uuids.get(&id).copied()) {
            self.common.persist.off_level.insert(id, object);
            return;
        }
        match object {
            OffLevel::Mob(mob) => self.common.mobs.push(mob),
            OffLevel::Boat(boat) => self.common.boats.push(boat),
            OffLevel::Cart(cart) => self.common.carts.push(cart),
        }
    }

    /// Put `dim`'s level in place for a command whose source is there
    /// (`execute in`, `execute at` a player in another level), opening it if
    /// nobody is in it; returns the level to put back afterwards. Only a
    /// server holds other levels.
    pub fn enter_command_level(&mut self, dim: Dim) -> Option<Dim> {
        if self.common.options.execution_role.is_client() || dim == self.common.dim {
            return None;
        }
        let back = self.common.dim;
        match self.enter_level(dim) {
            Ok(()) => Some(back),
            Err(e) => {
                log::error!("could not open {}: {e}", dim.id());
                None
            }
        }
    }

    /// Run an action with `dim` in place, then restore the caller's level.
    /// Transactions and entity admission consult the level in place during the action.
    pub(super) fn with_level_in_place<R>(
        &mut self,
        dim: Dim,
        f: impl FnOnce(&mut Self) -> R,
    ) -> Option<R> {
        if dim == self.dim {
            return Some(f(self));
        }
        let back = self.enter_command_level(dim)?;
        let result = f(self);
        self.enter_command_level(back);
        Some(result)
    }

    /// Tick shared functions before levels, then player connections, between the server tick hooks.
    pub fn tick_server(&mut self, paused: bool) -> Result<()> {
        let rng_tick = crate::rng_count::begin_tick();
        let result = self.with_residency_tick(|game| game.tick_server_body(paused));
        if result.is_ok() {
            rng_tick.publish();
        }
        result
    }

    fn tick_server_body(&mut self, paused: bool) -> Result<()> {
        #[cfg(feature = "net-trace")]
        if let Some(host) = self.server.lan.as_ref() {
            host.hub
                .lan
                .shared
                .set_capture_tick(self.common.world.game_time);
        }
        // A panic something caught left a game that carried on: its record ends with the tick.
        self.server.interrupted_turns.clear();
        // `tickRateManager.tick()` once per server tick, before any level
        // (each `ServerLevel.tick` only reads `runsNormally`), and the native
        // `ServerTickStart` before that level work (NeoForge's
        // `ServerTickEvent.Pre`). A paused pass and a held server tick nothing.
        let running = if !paused && !self.server.hold_server {
            self.common.clock.check_sprint_completion();
            if let Some(report) = self.common.clock.take_sprint_report() {
                // ServerTickRateManager.finishTickSprint: the server's sendSuccess.
                self.server.console_broadcast = Some(true);
                self.sprint_report(&report);
                self.server.console_broadcast = None;
                self.send_admin_feedback();
                self.server.admin_feedback.clear();
                // Then onTickRateChanged, its mods outside the server's broadcast.
                self.on_tick_rate_changed();
            }
            Some(self.common.clock.tick_world_runs())
        } else {
            None
        };
        let server = self.server.serving.is_some() || self.common.options.dedicated.is_some();
        if server {
            self.run_requested_saves();
        }
        // A config edited while the server is paused (empty) still reloads.
        // The parts outside every level get spans of their own, so that a slow
        // tick's line and `sunrise-report` say when a mod's tick hook took it.
        if server {
            let _span = sunrise_level::spans::guard("mod_configs_poll");
            self.native_configs_poll();
        }
        if let Some(running) = running
            && server
        {
            let _span = sunrise_level::spans::guard("mods_server_tick_start");
            self.native_lifecycle_server_tick_start(running);
        }
        if running == Some(true) {
            span("tick_functions", || self.tick_functions());
        }
        let observe_level = running.is_some()
            && (self.server.serving.is_some() || self.common.options.dedicated.is_some());
        let dimensions = self.level_creation_order().to_vec();
        let mut order = std::mem::take(&mut self.server.levels.0.tick_order);
        self.select_tick_levels(&mut order);
        let n = order.len();
        let mut result = Ok(());
        for phase in server_tick_phases(&dimensions, &order) {
            if let ServerTickPhase::IdleLevel(dim) = phase {
                if let Some(runs) = running {
                    self.count_fight_empty_time(dim, runs);
                }
                if running == Some(true) {
                    self.tick_level_environment(dim);
                    // ServerLevel.tick: the fight still ticks while `emptyTime < 300`.
                    self.tick_idle_fight(dim);
                }
                continue;
            }
            let ServerTickPhase::Level(k, dim) = phase else {
                if running.is_some() && self.server.lan.is_some() {
                    let holding = sunrise_sim::lan::Holding::start();
                    result = sunrise_level::spans::time("lan_guest_connections", || {
                        crate::rng_count::in_consumer(
                            crate::rng_count::Consumer::LanGuestConnections,
                            || self.tick_guest_connections(true),
                        )
                    });
                    if result.is_ok()
                        && let Some(host) = self.server.lan.as_mut()
                    {
                        sunrise_level::spans::time("lan_end_tick", || host.hub.end_tick());
                    }
                    drop(holding);
                    if let Some(host) = self.server.lan.as_ref() {
                        sunrise_level::spans::time("lan_handoff", || {
                            sunrise_sim::lan::flush_all(host.hub.guests.iter().map(|g| &g.sender));
                        });
                    }
                }
                break;
            };
            // Each level's share of the tick (the first one also reads the network and runs the
            // tick's shared parts): `sunrise-report`'s `level_*` spans.
            sunrise_threads::work::set_dim(match dim {
                Dim::Overworld => 1,
                Dim::Nether => 2,
                Dim::End => 3,
                Dim::Extra(_) => 0,
            });
            let _level_span = sunrise_level::spans::guard(match dim {
                Dim::Overworld => "level_overworld",
                Dim::Nether => "level_the_nether",
                Dim::End => "level_the_end",
                Dim::Extra(_) => "level_extra",
            });
            if let Err(e) = self.enter_level(dim) {
                result = Err(e);
                break;
            }
            if let Some(runs) = running {
                self.count_fight_empty_time(dim, runs);
            }
            self.server.level_pass = Pass {
                first: k == 0,
                last: k + 1 == n,
                running,
                packets_drained: false,
            };
            if observe_level && let Some(level) = self.level(dim.id()) {
                self.native_level_tick_pre(level);
            }
            // ServerChunkCache.tick's purge, before the update that may unload the columns.
            if running == Some(true) {
                self.server.streaming.purge_unknown_tickets();
            }
            sunrise_level::spans::time("dedicated_streaming", || {
                self.update_dedicated_streaming();
            });
            self.tick_spawn_tickets(running == Some(true), true);
            sunrise_level::spans::time("update_world", || self.server_update_world());
            // The end of ServerChunkCache.tick, after its unloads: `clearCache`. A paused or held
            // server ticks no level.
            if running.is_some() {
                self.server.streaming.clear_full_lookups();
            }
            let tick_span = sunrise_level::spans::guard("level_tick");
            // This level's moving colliders, solid to its movers for this tick only.
            let colliders = self.placed_moving_colliders();
            let parts = self.common.collision_parts();
            let history = self
                .collider_history(&colliders)
                .map(sunrise_world::moving_colliders::HistoryScope::enter);
            result = {
                let _scope = sunrise_world::moving_colliders::Scope::enter_level(
                    self.collider_level(),
                    colliders.clone(),
                );
                sunrise_sim::block_contact::in_level(
                    self.common.tick,
                    (self.server.serving.is_some() || self.common.options.dedicated.is_some())
                        .then_some(dim.id()),
                    || {
                        if paused {
                            self.tick_lan(true);
                            Ok(())
                        } else if k == 0 {
                            self.tick_server_first(running.unwrap_or(false))
                        } else {
                            let running = running.unwrap_or_else(|| self.common.clock.world_runs());
                            if running {
                                self.tick_level_environment(dim);
                            }
                            // ClientLevel.tick: the sky flash fades in a frozen level too.
                            self.tick_weather_effects();
                            self.tick_world(running)
                        }
                    },
                )
            };
            drop(history);
            // Then the bodies standing on a collider that moved are carried with it.
            if result.is_ok() && !paused {
                sunrise_sim::block_contact::in_level(
                    self.common.tick,
                    (self.server.serving.is_some() || self.common.options.dedicated.is_some())
                        .then_some(dim.id()),
                    || self.carry_pass(&colliders),
                );
            }
            drop(parts);
            drop(tick_span);
            self.drain_entity_inside();
            if result.is_err() {
                break;
            }
            if observe_level && let Some(level) = self.level(dim.id()) {
                self.native_level_tick_post(level);
            }
            #[cfg(feature = "state-digest")]
            self.state_digest_tick(dim, running);
            sunrise_level::spans::time("server_debug", || self.publish_server_debug(dim));
            // An entity that crossed into a level that already ticked this
            // server tick is in it when the tick ends (`addDuringTeleport`).
            if running == Some(true) {
                let _span = sunrise_level::spans::guard("late_portal_arrivals");
                let mut moved = false;
                for &earlier in &order[..k] {
                    if self.has_portal_arrivals(earlier) && self.enter_level(earlier).is_ok() {
                        self.land_portal_arrivals();
                        moved = true;
                    }
                }
                if moved && k + 1 < n {
                    result = self.enter_level(dim);
                    if result.is_err() {
                        break;
                    }
                }
            }
        }
        // Vanilla ticks the chunk source of every level, ticked here or not: the levels left
        // parked purge their tickets and forget their recent lookups too.
        if running.is_some() {
            self.server
                .levels
                .0
                .tick_parked_chunk_caches(running == Some(true), |dim| !order.contains(&dim));
        }
        // Levels held only by loading tickets: stream them without running the world tick.
        if result.is_ok() {
            result = self.run_view_passes(&order, running == Some(true));
        }
        if result.is_ok() {
            self.update_parked_light(&order);
        }
        if let Some(&first) = order.first()
            && self.common.dim != first
        {
            let _ = self.enter_level(first);
        }
        self.server.levels.0.tick_order = order;
        self.server.level_pass = Pass::default();
        if result.is_err() {
            self.common.native_lifecycle.cancel_server_tick();
        }
        result?;
        // The levels mods asked to open or close: now that no level is in the middle of a tick.
        self.apply_runtime_levels();
        let evicted = if should_run_idle_maintenance(paused) {
            let _span = sunrise_level::spans::guard("evict_idle_levels");
            self.evict_idle_levels()
        } else {
            Ok(())
        };
        if let Err(error) = evicted {
            self.common.native_lifecycle.cancel_server_tick();
            return Err(error);
        }
        // `ServerTickEnd`: an errored tick or an idle eviction failure (at
        // the `?` above) never reaches this site.
        {
            let _span = sunrise_level::spans::guard("mods_server_tick_end");
            self.native_lifecycle_server_tick_end(paused);
        }
        Ok(())
    }

    /// `tick_server` as a server thread runs it, timed whenever spans are on.
    pub(in crate::game::server_side) fn tick_server_timed(&mut self, paused: bool) -> Result<()> {
        self.tick_server_spans(paused, |_| {}).map(|_| ())
    }

    /// Return the time in `tick_server` itself, and hand its detailed timing to `record`
    /// while spans are on. The recording and thread-work bookkeeping are outside that time.
    pub fn tick_server_spans(
        &mut self,
        paused: bool,
        record: impl FnOnce(TickTiming),
    ) -> Result<Duration> {
        let started = Instant::now();
        sunrise_threads::work::tick_start();
        let result = self.tick_server_recorded(paused, record);
        sunrise_threads::work::tick_end(started.elapsed().as_nanos() as u64);
        result
    }

    fn tick_server_recorded(
        &mut self,
        paused: bool,
        record: impl FnOnce(TickTiming),
    ) -> Result<Duration> {
        if self.common.options.spikes.is_none() && !sunrise_level::spans::recording_here() {
            let started = Instant::now();
            self.tick_server(paused)?;
            return Ok(started.elapsed());
        }
        // Discard between-tick work (such as the chunk pump) before timing this tick.
        sunrise_level::spans::take_raw();
        let cpu = sunrise_level::spans::thread_cpu();
        let started = Instant::now();
        let result = self.tick_server(paused);
        let method = started.elapsed();
        let wall = method.as_secs_f32() * 1000.0;
        let cpu = sunrise_level::spans::thread_cpu()
            .zip(cpu)
            .map(|(end, start)| (end - start).as_secs_f32() * 1000.0);
        // Taken every tick: nothing else takes this thread's spans.
        let raw = sunrise_level::spans::take_raw();
        // The spans go to the slow ring; whose packets or events took the tick stays in the owner's log.
        let notes = sunrise_level::spans::take_notes();
        if let Some(threshold) = self.common.options.spikes
            && wall > threshold
            && !notes.is_empty()
        {
            log::info!(
                "Sunrise: {wall:.1} ms tick was busy with: {}",
                slowest_steps(&notes)
            );
        }
        record(TickTiming {
            wall_ms: wall,
            cpu_ms: cpu,
            raw,
        });
        result.map(|()| method)
    }

    /// The end of `MinecraftServer.runServer`'s loop for a server thread:
    /// the next tick starts one tick length after this one, or straight away
    /// while the sprint runs (`checkShouldSprintThisTick`). A server more
    /// than two seconds behind skips the ticks it missed: that returns how far
    /// behind it was, for the overload warning, and how long the thread blocked on
    /// workers or slept (`MinecraftServer.waitForTasks`'s idle time).
    pub(in crate::game::server_side) fn wait_for_next_server_tick(
        &mut self,
        next: &mut Instant,
        paused: bool,
    ) -> (Option<Duration>, Duration) {
        let now = Instant::now();
        let mut idle = Duration::ZERO;
        if !paused && self.common.clock.sprinting() {
            *next = now;
            return (None, idle);
        }
        *next += self.common.clock.tick_duration();
        if *next > now {
            // The chunks loading (the players', force-loaded ones, a
            // `/pregen` task) keep the workers fed until the next tick.
            idle = self.pump_chunks(*next, paused);
            // Integrated servers also service window `OwnerRequest`s while idle
            // between ticks (`MinecraftServer.waitUntilNextTick`), so synchronous
            // queries (`StatusExtras`, publish/unpublish, control ops) do not
            // stall the client frame for up to a full 50 ms server tick.
            loop {
                let now = Instant::now();
                if now >= *next {
                    break;
                }
                let remaining = *next - now;
                let received = match &self.server.serving {
                    Some(serving) => {
                        if serving
                            .flags
                            .stop
                            .load(std::sync::atomic::Ordering::Relaxed)
                        {
                            break;
                        }
                        serving.tasks.recv_timeout(remaining)
                    }
                    None => {
                        std::thread::sleep(remaining);
                        idle += now.elapsed();
                        break;
                    }
                };
                match received {
                    Ok(request) => {
                        idle += now.elapsed();
                        self.serve_owner_request(request);
                        self.apply_publish();
                    }
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                        idle += now.elapsed();
                        break;
                    }
                    Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                        let now2 = Instant::now();
                        if *next > now2 {
                            std::thread::sleep(*next - now2);
                        }
                        idle += now.elapsed();
                        break;
                    }
                }
            }
        } else if now - *next > Duration::from_secs(2) {
            let behind = now - *next;
            *next = now;
            return (Some(behind), idle);
        }
        (None, idle)
    }

    /// The levels this server tick runs, in `MinecraftServer.levels` order
    /// (the Overworld, the Nether, the End): every level with a player in
    /// it, forced chunks or a `/pregen` task, or the Overworld while idle.
    fn select_tick_levels(&self, dims: &mut Vec<Dim>) {
        dims.clear();
        let order = self.level_creation_order();
        dims.extend(
            order
                .iter()
                .copied()
                .filter(|&dim| self.should_tick_level(dim)),
        );
        if dims.is_empty() {
            dims.push(Dim::Overworld);
        }
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn levels_to_tick(&self) -> Vec<Dim> {
        let mut dims = Vec::new();
        self.select_tick_levels(&mut dims);
        dims
    }

    fn should_tick_level(&self, dim: Dim) -> bool {
        // `/forceload`ed chunks, or a native mod's tickets.
        let streaming = if dim == self.common.dim {
            Some(&self.server.streaming)
        } else {
            self.server.levels.0.parked.get(&dim).map(|p| &p.streaming)
        };
        streaming.is_some_and(|s| !s.forced.is_empty())
            || self.server.lan.as_ref().is_some_and(|h| {
                h.hub.guests.iter().any(|g| {
                    g.dimension == dim.id() && g.participates_in_player_simulation()
                })
            })
            // A viewer count not yet told to 0 (`tick_mod_container_viewers`).
            || self.server.mod_container_viewers.keys().any(|(k, _)| *k == dim.id())
            || self.player_bed_levels().any(|b| b == dim)
            || self.server.game_tests.0.has_tests_in(dim)
            || self.pregen_level() == Some(dim)
            // An arrival or its portal ticket keeps the destination level active.
            || self.has_portal_arrivals(dim)
            || self.server.server_trips.iter().any(|trip| trip.spawn_destination_areas(dim).next().is_some())
            || !self.portal_tickets_in(dim).is_empty()
            // A viewed `Ticking` area (#3716), for its chunks only.
            || self.viewed_level(dim) == Some(true)
    }

    /// A loading-only ticket needs chunk work, not an active simulation.
    pub(in crate::game::server_side) fn has_spawn_tickets(&self, dim: Dim) -> bool {
        if self.server.lan.as_ref().is_some_and(|host| {
            host.hub
                .lan
                .configuration_tasks
                .loading_spawns(dim.id())
                .next()
                .is_some()
        }) {
            return true;
        }
        if dim == self.common.dim {
            self.server
                .chunk_ticking
                .tickets
                .spawn_areas()
                .next()
                .is_some()
        } else {
            self.server
                .levels
                .0
                .parked
                .get(&dim)
                .is_some_and(|p| p.chunk_ticking.tickets.spawn_areas().next().is_some())
        }
    }

    /// The portal tickets of `dim`'s level (`placePortalTicket`, or saved).
    pub(in crate::game::server_side) fn portal_tickets_in(&self, dim: Dim) -> &[PortalTicket] {
        if dim == self.common.dim {
            self.server.chunk_ticking.tickets.portal_tickets()
        } else {
            self.server
                .levels
                .0
                .parked
                .get(&dim)
                .map_or(&[], |p| p.chunk_ticking.tickets.portal_tickets())
        }
    }

    /// The areas the level in place's portal tickets keep streamed: around each
    /// exit its block-ticking chunks and their neighbours, the ring vanilla
    /// keeps `FULL` (level 33), so the outer block-ticking ring is a ticking
    /// chunk too; and at least the chunks its expiry waits for
    /// (`portal_ticket_can_expire`).
    pub(in crate::game::server_side) fn portal_ticket_areas(&self) -> Vec<((i32, i32), i32)> {
        let block_ticking = i32::from(sunrise_sim::ticket_levels::BLOCK_TICKING);
        self.server
            .chunk_ticking
            .tickets
            .portal_tickets()
            .iter()
            .map(|t| (t.chunk, (block_ticking - t.level).max(1)))
            .collect()
    }

    /// Advance light jobs queued on parked levels that neither ticked nor ran a view pass
    /// this server tick, without switching `self.dim` or running chunk generation/unloads.
    pub fn update_parked_light(&mut self, ticked: &[Dim]) {
        let viewed_active = self
            .server
            .lan
            .as_ref()
            .is_some_and(|h| h.hub.viewed.active());
        let pending: Vec<Dim> = self
            .server
            .levels
            .0
            .parked
            .iter()
            .filter_map(|(&dim, parked)| {
                (!ticked.contains(&dim)
                    && !(viewed_active && self.needs_view_pass(dim))
                    && !parked.streaming.paused
                    && parked.streaming.has_pending_light(&parked.world))
                .then_some(dim)
            })
            .collect();
        for dim in pending {
            self.ask_edited_light_emission(dim);
            if let Some(parked) = self.server.levels.0.parked.get_mut(&dim) {
                parked.streaming.update_light(&mut parked.world);
            }
        }
    }

    /// Whether saving is currently disabled (`/save-off`, `ServerLevel.noSave`).
    fn saving_disabled(&self) -> bool {
        self.server
            .server_lists
            .0
            .as_ref()
            .is_some_and(|lists| !lists.lock().unwrap().autosave)
    }

    pub fn evict_idle_levels(&mut self) -> Result<()> {
        // Runtime level opens/closes are applied outside this pass. Read each parked level
        // in creation order, after previous unload listeners have had a chance to activate it.
        for index in 0..self.server.levels.0.order.len() {
            if self.saving_disabled() {
                break;
            }
            let dim = self.server.levels.0.order[index];
            if dim == Dim::Overworld
                || !self.server.levels.0.parked.get(&dim).is_some_and(|p| {
                    (!p.world.chunks.is_empty() || !p.persist.uuids.is_empty())
                        && !p.streaming.has_unknown_tickets()
                })
                || self.should_tick_level(dim)
                || self.needs_view_pass(dim)
                || self.has_spawn_tickets(dim)
                || self.has_async_chunk_requests(dim)
                || self.vehicle_waits_in(dim)
            {
                continue;
            }
            self.evict_idle_level_chunks(dim)?;
        }
        Ok(())
    }

    pub fn evict_idle_level_chunks(&mut self, dim: Dim) -> Result<()> {
        if self.saving_disabled() {
            return Ok(());
        }
        let back = self.common.dim;
        self.enter_level(dim)?;
        let prev_radius = self.server.streaming.radius;
        let prev_paused = self.server.streaming.paused;
        let prev_extra = std::mem::take(&mut self.server.streaming.extra);
        self.server.streaming.radius = -1;
        self.server.streaming.paused = false;
        self.server.streaming.force_unload_sweep();
        let evicted = self.server_update_world_with_eviction(true);
        self.rebuild_chunk_tickets(false);
        self.server.streaming.radius = prev_radius;
        self.server.streaming.paused = prev_paused;
        self.server.streaming.extra = prev_extra;
        self.enter_level(back)?;
        evicted
    }

    /// Save and retire an explicitly unloaded runtime level, publishing its chunk and level unloads.
    pub fn close_parked_level(&mut self, dim: Dim) -> Result<()> {
        debug_assert!(
            !dim.is_builtin(),
            "built-in levels stay open until server shutdown"
        );
        let back = self.common.dim;
        self.enter_level(dim)?;
        // Vanilla saves the level's chunks before their unload hooks.
        self.native_chunks_loaded();
        self.abort_abandoned_transactions(0);
        for key in self
            .server
            .streaming
            .columns_with_outstanding_light(&self.common.world)
        {
            if let Some(chunk) = self.common.world.chunks.get_mut(&key) {
                Arc::make_mut(chunk).lit = false;
                self.common.world.dirty_light.insert(key);
            }
        }
        if let Err(why) = self.save_level(true) {
            // Not saved: the level stays parked, and the level in place is the one before.
            self.enter_level(back)?;
            return Err(why);
        }
        let chunks: Vec<_> = self.common.world.chunks.values().cloned().collect();
        self.native_chunks_unloaded(&chunks);
        // Save and unload listeners can leave frames open; undo them before dropping the level.
        self.abort_abandoned_transactions(0);
        // Its entities' attachments are saved with them; they leave memory too.
        for id in self.common.persist.uuids.keys() {
            self.common.entity_attachments.remove(id);
            self.common.entity_attachments.remove_collider(*id);
        }
        self.enter_level(back)?;
        self.native_level_closed(dim);
        self.server.levels.0.parked.remove(&dim);
        Ok(())
    }

    /// The mods hear `dim`'s level open (`LevelEvent.Load`), on a server.
    pub fn native_level_opened(&mut self, dim: Dim) {
        if self.common.is_logical_server() {
            self.open_level_fight(dim);
        }
        if dim == self.common.dim {
            self.server.levels.0.active_closed = false;
        }
        if !self.common.is_logical_server() || self.server.levels.0.order.contains(&dim) {
            return;
        }
        let journal = self.common.resident_entities.table.borrow().producer();
        if dim == self.common.dim {
            self.bind_resident_owners(&journal, dim.key(), true);
        } else if let Some(parked) = self.server.levels.0.parked.get_mut(&dim) {
            parked.bind_resident_owners(&journal, dim.key(), true);
        }
        self.bind_dimension_attachments(dim);
        let levels = &mut self.server.levels.0;
        levels.order.push(dim);
        levels.open_incarnation(dim);
        self.init_custom_spawners(dim);
        self.native_level_load(dim.id());
    }

    /// The mods hear `dim`'s level close (`LevelEvent.Unload`), once for
    /// each open they heard.
    pub fn native_level_closed(&mut self, dim: Dim) {
        self.remove_level_fake_players(dim.id());
        if let Some(index) = self
            .server
            .levels
            .0
            .order
            .iter()
            .position(|&level| level == dim)
        {
            self.native_level_unload(dim.id());
            self.server.levels.0.order.remove(index);
        }
        self.server.levels.0.close_incarnation(dim);
        if dim == self.common.dim {
            self.common.world.water_tickets.clear();
        } else if let Some(world) = self.server.levels.0.parked_world_mut_of(dim.id()) {
            world.water_tickets.clear();
        }
        self.common.creatures.custom_spawners.remove(&dim);
        self.common.attachment_indexes.remove(&dim);
        self.common.passages.retire_level(dim.key());
        self.clear_async_chunk_requests_for_dim(dim);
        self.close_saved_chunk_readers_for_dim(dim);
        self.close_detached_generators_for_dim(dim);
    }

    /// Every level still open closes with the server, however it ends
    /// (`MinecraftServer.stopServer`'s `level.close()` loop), in vanilla's
    /// order of its levels: the Overworld, the Nether, the End.
    pub fn close_loaded_levels_for_stop(&mut self) {
        let dims = self.loaded_level_order();
        for dim in dims {
            if dim == self.common.dim {
                if self.server.levels.0.active_closed {
                    continue;
                }
                match self.server.save.0.flush() {
                    Ok(()) => {
                        self.server.streaming.abort_generation();
                        self.native_level_closed(dim);
                        self.server.levels.0.active_closed = true;
                    }
                    Err(error) => {
                        log::error!("Exception closing the level {}: {error}", dim.id());
                    }
                }
            } else if let Some(parked) = self.server.levels.0.parked.get_mut(&dim) {
                match parked.save.flush() {
                    Ok(()) => {
                        parked.streaming.abort_generation();
                        self.native_level_closed(dim);
                        self.server.levels.0.parked.remove(&dim);
                    }
                    Err(error) => {
                        log::error!("Exception closing the level {}: {error}", dim.id());
                    }
                }
            }
        }
    }

    /// Run the requested level save passes. `level_save` fires after a pass even if an
    /// individual record failed and was reported. A failed awaited flush ends the pass with
    /// an error and no hook. Reopen levels as needed, then restore the active one.
    pub fn run_requested_saves(&mut self) {
        if sunrise_gameplay_common::game::CommonGame::admit_entity_save().is_err() {
            return;
        }
        let requested = std::mem::take(&mut self.server.levels.0.requested_saves);
        if requested.is_empty() || !self.common.is_logical_server() {
            return;
        }
        let back = self.common.dim;
        for dim in requested {
            if let Err(e) = self.enter_level(dim).and_then(|()| self.save_level(true)) {
                log::error!("saving {} on a mod's request failed: {e}", dim.id());
            }
        }
        if let Err(e) = self.enter_level(back) {
            log::error!(
                "returning to {} after a requested save failed: {e}",
                back.id()
            );
        }
    }

    pub fn save_all_levels(&mut self, wait: bool) -> Result<()> {
        let back = self.common.dim;
        let dims = self.loaded_level_order();
        let mut saved = Ok(());
        for dim in dims {
            if let Err(error) = self.enter_level(dim).and_then(|()| self.save_level(wait)) {
                saved = Err(error);
                break;
            }
        }
        let restored = if self.common.dim != back {
            self.enter_level(back)
        } else {
            Ok(())
        };
        saved.and(restored)
    }
}

#[cfg(any(test, feature = "test-support"))]
fn dim_fallback_key(dim: Dim) -> (u8, &'static str) {
    match dim {
        Dim::Overworld => (0, ""),
        Dim::Nether => (1, ""),
        Dim::End => (2, ""),
        Dim::Extra(_) => (3, dim.id()),
    }
}

fn slowest_steps(notes: &[(f32, String)]) -> String {
    let list: Vec<String> = notes
        .iter()
        .map(|(ms, what)| format!("{what} {ms:.1}"))
        .collect();
    list.join(", ")
}

impl crate::game::ServerGame<'_> {
    /// `Level.getChunk(x, z)` (`ServerChunkCache.getChunk(x, z, FULL, true)`) on the server
    /// thread: column `chunk` of the level in place (`parked: None`) or of the parked level
    /// `parked`, without switching levels, loaded or generated to FULL (resident and lit) under
    /// an `UNKNOWN` ticket (`ChunkStreamer::begin_full_column`). It blocks until then, with no
    /// timeout, as vanilla's `managedBlock`. Each column the wait brings in is announced while
    /// it is resident, before this returns: its `dynamic_light` cells asked (the light job
    /// waits for them) and `chunk_load` fired, at the same point as vanilla's FULL task fires
    /// NeoForge's `ChunkEvent.Load` inside `getChunk`. `Ok` once FULL; `Failed` when a
    /// generation the column needs panicked, before or during the wait
    /// (`ChunkStreamer::full_column_failed`); `Unloaded` when `begin_full_column` refuses, when
    /// the level is gone, and when generation is aborted during the wait.
    pub(in crate::game::server_side) fn load_full_chunk(
        &mut self,
        parked: Option<mod_api::LevelRef>,
        chunk: (i32, i32),
    ) -> std::result::Result<(), mod_api::ChunkLoadError> {
        use mod_api::ChunkLoadError;
        use sunrise_level::chunk_streamer::{FullColumn, full_column_ready};
        let id = parked.map(|level| level.dimension_id());
        let Some((streaming, world)) = self.full_load_level(id) else {
            return Err(ChunkLoadError::Unloaded);
        };
        match streaming.begin_full_column(world, chunk) {
            FullColumn::Ready => return Ok(()),
            FullColumn::Refused => return Err(ChunkLoadError::Unloaded),
            FullColumn::Failed => return Err(ChunkLoadError::Failed),
            FullColumn::Pending => {}
        }
        loop {
            let Some((streaming, world)) = self.full_load_level(id) else {
                return Err(ChunkLoadError::Unloaded);
            };
            let alive = streaming.pump_column(world);
            self.announce_synchronous_loads(parked);
            let Some((streaming, world)) = self.full_load_level(id) else {
                return Err(ChunkLoadError::Unloaded);
            };
            let failed = streaming.full_column_failed(chunk);
            if !alive || failed || full_column_ready(world, chunk) {
                return if streaming.end_full_column(world, chunk) {
                    Ok(())
                } else if failed {
                    Err(ChunkLoadError::Failed)
                } else {
                    Err(ChunkLoadError::Unloaded)
                };
            }
            streaming.wait(Duration::from_millis(5));
        }
    }

    /// The streaming and world of the level in place, or of the parked level `parked`.
    fn full_load_level(
        &mut self,
        parked: Option<&str>,
    ) -> Option<(&mut ChunkStreamer, &mut World)> {
        match parked {
            None => Some((&mut self.server.streaming, &mut self.common.world)),
            Some(id) => {
                let level = self.server.levels.0.parked_mut_of(id)?;
                Some((&mut level.streaming, &mut level.world))
            }
        }
    }

    /// The columns a synchronous load brought into the level, told to the mods as the level's
    /// own update tells them (`native_chunks_loaded`), their `dynamic_light` cells asked first.
    fn announce_synchronous_loads(&mut self, parked: Option<mod_api::LevelRef>) {
        let Some(level) = parked else {
            self.ask_loaded_light_emission(0);
            self.native_chunks_loaded();
            return;
        };
        let id = level.dimension_id();
        let Some(world) = self.server.levels.0.parked_world_mut_of(id) else {
            return;
        };
        let loaded = std::mem::take(&mut world.loaded_chunks);
        if self.common.pack.chunk_context.light.any_dynamic() {
            let dimension =
                sunrise_gameplay_common::game::light_emission::light_emission_dimension(level);
            for &(x, z, _) in &loaded {
                self.ask_level_chunk_light_emission(Some(id), dimension, (x, z));
            }
        }
        for (x, z, new) in loaded {
            self.native_chunk_load(id, [x, z], new);
        }
    }
}

impl crate::game::ServerGame<'_> {
    /// Rebind only an already-owning transfer, before installing it in this level.
    /// Memory still belongs to the parked source until the caller moves it.
    pub fn rebind_transferred_entity(
        &mut self,
        id: u32,
        entity: &mut impl sunrise_mod_runtime::attachments::HasAttachmentLifetime,
    ) {
        let slot = entity.attachment_lifetime();
        if !slot.is_bound() {
            return;
        }
        let Some(level) = self.level(self.common.dim.id()) else {
            return;
        };
        let Some(map) = self.common.entity_attachments.get_mut(&id) else {
            return;
        };
        slot.bind(
            map,
            self.common
                .attachment_indexes
                .entry(self.common.dim)
                .or_default(),
            mod_api::AttachmentHolder::Entity(mod_api::EntityRef::new(level.epoch(), id)),
        );
    }
}

impl crate::game::ServerGame<'_> {
    /// `Game::tick` as a server runs it for its first level: no window lead, which only the window's `Game` has.
    fn tick_server_first(&mut self, running: bool) -> Result<()> {
        if !self.common.is_logical_server() {
            return Err("client-role game cannot tick a server level".into());
        }
        if self.server.hold_server {
            return Ok(());
        }
        self.abort_open_transactions();
        self.common.update_interaction_ranges();
        self.tick_first_level(running)
    }

    /// `ServerLevel.tick`: advance the world border, then the natural weather cycle.
    fn tick_level_environment(&mut self, dim: Dim) {
        self.tick_worldborder(dim);
        self.tick_level_weather(dim);
    }

    /// A server's tick of its first active level, with the shared clock and autosave.
    pub(in crate::game) fn tick_first_level(&mut self, running: bool) -> Result<()> {
        if !self.common.is_logical_server() {
            return Err("client-role game cannot tick a server level".into());
        }
        // Only a server ticks its own world from here: its players are guests, swapped in for
        // their turns (`as_guest`), and the window's player is a client of it.
        self.common.ticks += 1;
        span("mods_inventory_ticks", || self.native_inventory_ticks());
        if running {
            self.tick_level_environment(self.common.dim);
            self.common.world.game_time += 1;
            #[cfg(feature = "net-trace")]
            if let Some(host) = self.server.lan.as_ref() {
                host.hub
                    .lan
                    .shared
                    .set_capture_tick(self.common.world.game_time);
            }
        } else if self.common.clock.still() {
            // Under `SUNRISE_STILL` a frozen world's rain and thunder sit at their levels, as a
            // shader pack's smoothed values do, so `/weather rain` shows full rain without
            // stepping the world (and its mobs) through the fade.
            self.common.world.weather.settle_levels();
        }
        // ClientLevel.tick: the sky flash fades in a frozen level too.
        self.tick_weather_effects();
        let _ = span("player_autosaves", || self.finish_player_autosaves(false));
        // MinecraftServer.tickServer's countdown, then autoSave: three
        // hundred seconds of ticks at the rate the server runs at now.
        self.server.ticks_until_autosave -= 1;
        self.common.autosaving = self.server.ticks_until_autosave <= 0;
        if let Some(serving) = &self.server.serving {
            serving.set_saving(self.common.autosaving);
        }
        if self.common.autosaving {
            span("autosave", || self.save_game_with(false))?;
            self.server.ticks_until_autosave = self.next_autosave_interval();
            if let Some(serving) = &self.server.serving {
                serving.set_saving(false);
            }
        }
        if running {
            self.advance_clock();
            // `ServerLevel.tickTime`, once the clock has moved on.
            span("scheduled_functions", || self.run_scheduled_functions());
        }
        span("tick_command_extras", || self.tick_command_extras());
        self.tick_world(running)
    }
}

use sunrise_gameplay_common::game::native_hooks::base::GameHooks as _;

use sunrise_gameplay_common::game::tables::GameItems as _;

use sunrise_gameplay_common::game::tables::WorldTablesExt as _;

use sunrise_gameplay_common::game::transactions::TransactionsGameExt as _;

use sunrise_gameplay_common::game::GameGameExt1 as _;

use sunrise_gameplay_common::game::clocks::ClocksGameExt1 as _;

use sunrise_gameplay_common::game::native_configs::NativeConfigsGameExt1 as _;

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;

use sunrise_gameplay_common::game::native_hooks::SharedGameExt1 as _;

use sunrise_gameplay_common::game::piston_push::carrying::CarryingGameExt1 as _;

#[cfg(test)]
mod tests {
    use super::*;

    fn dirty_test_chunk(game: &mut crate::game::ServerGame<'_>) {
        let chunk = sunrise_level::chunk::Chunk::with_layout(0, 0, game.world.layout());
        game.world.chunks.insert((0, 0), Arc::new(chunk));
        game.world.modified.insert((0, 0));
    }

    fn queue_unencodable_entity_record(save: &SaveDir) {
        use sunrise_core::nbt::{DISK_WRITE_NESTING_GUARD, Tag};
        let invalid =
            (0..=DISK_WRITE_NESTING_GUARD).fold(Tag::Int(0), |nested, _| Tag::List(vec![nested]));
        save.save_entities_later(vec![((2, 0), vec![invalid])])
            .unwrap();
    }

    #[test]
    #[ignore = "daily"]
    fn save_all_succeeds_after_reported_failures_in_multiple_levels() {
        use crate::game::native_entity_refs::tests::with_game;
        use sunrise_sim::save::{Level, LevelShape};
        mod_platform::testing::with_mods(Vec::new().into(), || {
            with_game(|game| {
                let root = tempfile::tempdir().unwrap();
                game.server.world_root = root.path().to_path_buf();
                game.server.save.0 = Arc::new(SaveDir::new(
                    game.server.world_root.clone(),
                    LevelShape::OVERWORLD,
                    Some(Default::default()),
                ));
                game.level.name = "saved despite two refused records".into();
                let mut folders = Vec::new();
                for dim in [Dim::Nether, Dim::End, Dim::Overworld] {
                    game.enter_level(dim).unwrap();
                    dirty_test_chunk(game);
                    folders.push((
                        dim,
                        game.server.save.0.root.clone(),
                        game.server.save.0.shape,
                    ));
                    if dim != Dim::End {
                        queue_unencodable_entity_record(&game.server.save.0);
                    }
                }
                assert_eq!(
                    game.save_all_reply().translation_key(),
                    Some("commands.save.success")
                );
                assert_eq!(game.server.save.0.record_save_failures(), 2);
                assert_eq!(game.dim, Dim::Overworld);
                assert_eq!(
                    Level::read(root.path()).unwrap().name,
                    "saved despite two refused records"
                );
                for (_, folder, shape) in folders {
                    assert!(
                        SaveDir::new(folder.clone(), shape, None)
                            .raw_chunk(0, 0)
                            .is_some(),
                        "{} was not written",
                        folder.display()
                    );
                }
            });
        });
    }

    #[test]
    #[ignore = "daily"]
    fn an_idle_level_closes_after_reporting_a_record_failure() {
        use crate::game::native_entity_refs::tests::with_game;
        use mod_api::{RuntimeGenerator, RuntimeLevel};
        use sunrise_sim::save::LevelShape;
        sunrise_gameplay_common::game::in_own_process(
            "game::server_side::levels::tests::an_idle_level_closes_after_reporting_a_record_failure",
            || {
                mod_platform::testing::with_mods(Vec::new().into(), || {
                    with_game(|game| {
                        let root = tempfile::tempdir().unwrap();
                        game.server.world_root = root.path().to_path_buf();
                        game.server.save.0 = Arc::new(SaveDir::new(
                            game.server.world_root.clone(),
                            LevelShape::OVERWORLD,
                            Some(Default::default()),
                        ));
                        let id = "test_idle:record_failure";
                        let flat =
                            RuntimeLevel::new("minecraft:overworld", RuntimeGenerator::Flat, 1);
                        game.runtime_level_open(id, flat).unwrap();
                        game.apply_runtime_levels();
                        assert!(game.runtime_level_is_open(id));
                        let custom = Dim::extra(id).unwrap();
                        game.enter_level(custom).unwrap();
                        dirty_test_chunk(game);
                        queue_unencodable_entity_record(&game.server.save.0);
                        let disk = SaveDir::new(
                            game.server.save.0.root.clone(),
                            game.server.save.0.shape,
                            None,
                        );
                        game.enter_level(Dim::Overworld).unwrap();
                        game.close_parked_level(custom).unwrap();
                        assert_eq!(game.dim, Dim::Overworld);
                        assert!(!game.server.levels.0.parked.contains_key(&custom));
                        assert_eq!(game.server.save.0.record_save_failures(), 1);
                        assert!(disk.raw_chunk(0, 0).is_some());
                    });
                });
            },
        );
    }

    #[test]
    #[ignore = "daily"]
    fn idle_eviction_keeps_the_refused_column_and_retires_its_neighbor() {
        use crate::game::native_entity_refs::tests::with_game;
        use mod_api::{ChunkUnloadEvent, EntitiesUnloadEvent, Hooks, ModEntry, ServerHost};
        use std::sync::Mutex;
        use sunrise_core::nbt::{DISK_WRITE_NESTING_GUARD, Tag};
        use sunrise_gameplay_common::game::test_fixtures::run_fresh_cases;
        use sunrise_items::items::ItemStack;
        use sunrise_level::chunk::Chunk;

        if run_fresh_cases(
            "game::server_side::levels::tests::idle_eviction_keeps_the_refused_column_and_retires_its_neighbor",
            "SUNRISE_IDLE_EVICTION_WRITE_CASE",
            &["refused-entities"],
        ) {
            return;
        }

        #[derive(Debug, PartialEq)]
        enum Notice {
            Chunk([i32; 2]),
            Entities([i32; 2], usize),
        }
        struct Log(Arc<Mutex<Vec<Notice>>>);
        #[mod_api::__subscriptions]
        impl Hooks for Log {
            fn chunk_unload(&self, _: &mut dyn ServerHost, event: &mut ChunkUnloadEvent<'_>) {
                if event.dimension == "minecraft:the_nether" {
                    self.0.lock().unwrap().push(Notice::Chunk(event.chunk));
                }
            }
            fn entities_unload(&self, _: &mut dyn ServerHost, event: &mut EntitiesUnloadEvent<'_>) {
                if event.dimension == "minecraft:the_nether" {
                    self.0
                        .lock()
                        .unwrap()
                        .push(Notice::Entities(event.chunk, event.count));
                }
            }
        }
        let notices = Arc::new(Mutex::new(Vec::new()));
        mod_platform::testing::with_mods(
            vec![ModEntry::new(
                "idle_eviction",
                Box::new(Log(notices.clone())),
            )]
            .into(),
            || {
                with_game(|game| {
                    let root = tempfile::tempdir().unwrap();
                    game.server.world_root = root.path().to_owned();
                    game.enter_level(Dim::Nether).unwrap();
                    for (x, item) in [(0, "minecraft:diamond"), (1, "minecraft:gold_ingot")] {
                        let chunk = Arc::new(Chunk::with_layout(x, 0, game.world.layout()));
                        game.world.chunks.insert((x, 0), chunk);
                        game.world.modified.insert((x, 0));
                        game.spawn_item(
                            ItemStack {
                                item: item.into(),
                                count: 1,
                                damage: 0,
                                extra: None,
                            },
                            [f64::from(x * 16) + 0.5, 64.0, 0.5],
                            [0.0; 3],
                            0,
                        );
                    }
                    let previous = Tag::compound([("marker", Tag::Int(47))]);
                    game.server
                        .save
                        .0
                        .save_entities(vec![((0, 0), vec![previous.clone()])])
                        .unwrap();
                    let invalid = (0..=DISK_WRITE_NESTING_GUARD)
                        .fold(Tag::Int(0), |nested, _| Tag::List(vec![nested]));
                    game.spawn_loaded_entity((0, 0), invalid, None);
                    let disk = SaveDir::new(
                        game.server.save.0.root.clone(),
                        game.server.save.0.shape,
                        None,
                    );
                    game.enter_level(Dim::Overworld).unwrap();
                    game.evict_idle_level_chunks(Dim::Nether).unwrap();
                    game.enter_level(Dim::Nether).unwrap();
                    assert!(game.world.chunks.contains_key(&(0, 0)));
                    assert!(game.world.modified.contains(&(0, 0)));
                    assert!(!game.world.chunks.contains_key(&(1, 0)));
                    assert!(!game.world.modified.contains(&(1, 0)));
                    assert_eq!(game.entities.dropped.len(), 1);
                    assert_eq!(
                        game.entities.dropped[0].stack.item.as_ref(),
                        "minecraft:diamond"
                    );
                    assert_eq!(disk.load_entities(0, 0).unwrap(), [previous]);
                    let neighbor = disk.load_entities(1, 0).unwrap();
                    assert_eq!(neighbor.len(), 1);
                    assert_eq!(
                        neighbor[0].get("Item").and_then(|item| item.get("id")),
                        Some(&Tag::String("minecraft:gold_ingot".into()))
                    );
                    assert!(disk.raw_chunk(1, 0).is_some());
                })
            },
        );
        assert_eq!(
            *notices.lock().unwrap(),
            [Notice::Chunk([1, 0]), Notice::Entities([1, 0], 1)]
        );
    }

    #[test]
    fn levels_clear_retires_the_level_order() {
        let mut levels = Levels::default();
        levels.order.push(Dim::Nether);
        assert!(!levels.order.is_empty());
        levels.clear();
        assert!(levels.order.is_empty());
        assert!(levels.parked.is_empty());
    }

    #[test]
    fn idle_maintenance_respects_pause() {
        // Server pause skips parked-level maintenance; unpaused ticks (including
        // `/tick freeze`, where `paused == false`) still run it.
        assert!(!should_run_idle_maintenance(true));
        assert!(should_run_idle_maintenance(false));
    }

    fn make_flat_terrain(game: &crate::game::ServerGame<'_>, seed: i64) -> Arc<VanillaTerrain> {
        let directory = game.common.options.resources.join("worldgen");
        Arc::new(
            VanillaTerrain::for_flat(
                sunrise_sim::vanillagen::TerrainResources {
                    data: Some(&game.common.pack.data_changes),
                    dir: &directory,
                    registry: game.common.pack.assets.registry,
                    biomes: sunrise_gameplay_common::world_biomes!(game),
                    item_data: &game.common.res.core.item_data,
                },
                &Default::default(),
                seed,
                &sunrise_world::worldgen::generator::FlatSettings {
                    layers: vec![
                        ("minecraft:bedrock".into(), 1),
                        ("minecraft:dirt".into(), 2),
                        ("minecraft:grass_block".into(), 1),
                    ],
                    biome: "minecraft:plains".into(),
                    structure_overrides: Some(Vec::new()),
                    features: false,
                    lakes: false,
                },
                false,
            )
            .expect("flat terrain"),
        )
    }

    #[test]
    #[ignore = "daily"]
    fn end_state_swap_level_exchanges_real_locators_and_keeps_siblings() {
        use crate::game::native_entity_refs::tests::{run_fresh_cases, with_game};
        use sunrise_core::{geometry::Vec3, position::BlockPos};
        use sunrise_gameplay_common::game::end_state::{
            Arriving, EndFlash, EndLevelState, EndState, EyeOfEnder, GatewayArrival, GatewayJob,
        };

        if run_fresh_cases(
            "game::server_side::levels::tests::end_state_swap_level_exchanges_real_locators_and_keeps_siblings",
            "SUNRISE_END_STATE_SWAP_CHILD",
            &["swap"],
        ) {
            return;
        }
        with_game(|game| {
            let terrain_a = make_flat_terrain(game, 12345);
            let terrain_b = make_flat_terrain(game, 67890);

            let mut traveller = sunrise_sim::end_gateway::Traveller::default();
            traveller.enter_with_delay(BlockPos::new(2, 64, 3), 23);
            let mut flash = EndFlash::default();
            flash.tick(150);
            flash.x_angle = 10.0;
            flash.y_angle = 20.0;
            let expected_intensity = flash.intensity(0.5);
            let expected_pose = flash.pose();

            let mut state = EndState {
                gateway_cooldown: 17,
                gateways: [(9, traveller)].into_iter().collect(),
                arrivals: vec![GatewayArrival {
                    traveller: Arriving::Live(77),
                    job: GatewayJob::Known {
                        exit: [1, 2, 3],
                        exact: true,
                    },
                    target: Vec3::new(4.0, 5.0, 6.0),
                }],
                areas: vec![(3, 4)],
                tickets: vec![((5, 6), 99)],
                locator: std::sync::Mutex::new(Some(terrain_a.clone())),
                eyes: vec![EyeOfEnder {
                    id: 55,
                    position: glam::DVec3::new(1.0, 2.0, 3.0),
                    previous: glam::DVec3::new(0.5, 1.5, 2.5),
                    velocity: glam::DVec3::new(0.1, 0.2, 0.3),
                    target: glam::DVec3::new(10.0, 20.0, 30.0),
                    life: 12,
                    survive: true,
                }],
                flash,
            };
            let mut parked = EndLevelState::new(Some(terrain_b.clone()));

            let check_siblings = |s: &EndState| {
                assert_eq!(s.gateway_cooldown, 17);
                assert_eq!(s.gateways.len(), 1);
                let mut copy = s.gateways[&9];
                assert_eq!(copy.cooldown(), 0);
                assert!(!copy.is_done());
                assert_eq!(
                    copy.travel_with_delay(true, 23),
                    Some(BlockPos::new(2, 64, 3))
                );
                assert_eq!(copy.cooldown(), 23);
                assert_eq!(s.arrivals.len(), 1);
                assert!(matches!(s.arrivals[0].traveller, Arriving::Live(77)));
                assert_eq!(
                    s.arrivals[0].job,
                    GatewayJob::Known {
                        exit: [1, 2, 3],
                        exact: true,
                    }
                );
                assert_eq!(s.arrivals[0].target, Vec3::new(4.0, 5.0, 6.0));
                assert_eq!(s.areas, vec![(3, 4)]);
                assert_eq!(s.tickets, vec![((5, 6), 99)]);
                assert_eq!(s.eyes.len(), 1);
                let eye = &s.eyes[0];
                assert_eq!(eye.id, 55);
                assert_eq!(eye.position, glam::DVec3::new(1.0, 2.0, 3.0));
                assert_eq!(eye.previous, glam::DVec3::new(0.5, 1.5, 2.5));
                assert_eq!(eye.velocity, glam::DVec3::new(0.1, 0.2, 0.3));
                assert_eq!(eye.target, glam::DVec3::new(10.0, 20.0, 30.0));
                assert_eq!(eye.life, 12);
                assert!(eye.survive);
                assert_eq!(s.flash.x_angle, 10.0);
                assert_eq!(s.flash.y_angle, 20.0);
                assert_eq!(s.flash.intensity(0.5), expected_intensity);
                assert_eq!(s.flash.pose(), expected_pose);
            };

            state.swap_level(&mut parked);
            check_siblings(&state);
            assert!(Arc::ptr_eq(&state.locator().unwrap(), &terrain_b));
            assert!(Arc::ptr_eq(parked.locator.as_ref().unwrap(), &terrain_a));

            state.swap_level(&mut parked);
            check_siblings(&state);
            assert!(Arc::ptr_eq(&state.locator().unwrap(), &terrain_a));
            assert!(Arc::ptr_eq(parked.locator.as_ref().unwrap(), &terrain_b));

            let mut empty = EndLevelState::new(None);
            state.swap_level(&mut empty);
            check_siblings(&state);
            assert!(state.locator().is_none());
            assert!(Arc::ptr_eq(empty.locator.as_ref().unwrap(), &terrain_a));

            state.swap_level(&mut empty);
            check_siblings(&state);
            assert!(Arc::ptr_eq(&state.locator().unwrap(), &terrain_a));
            assert!(empty.locator.is_none());
        });
    }

    #[test]
    #[ignore = "daily"]
    fn production_level_swap_exchanges_all_six_subsystems_and_keeps_siblings() {
        use crate::game::native_entity_refs::tests::{run_fresh_cases, with_game};
        use std::rc::Rc;
        use sunrise_core::{geometry::Vec3, position::BlockPos};
        use sunrise_gameplay_common::game::end_state::{
            Arriving, EndFlash, EndLevelState, EyeOfEnder, GatewayArrival, GatewayJob,
        };
        use sunrise_level::{
            block_entity_ticks::{Bell, ConduitServer, EndGateway, Ticker},
            chunk::Chunk,
            sculk::WardenSpawnTracker,
            shulker_box::{Shulker, ShulkerStatus},
        };
        use sunrise_sim::{
            mobs::Who,
            nether_portal::{PortalState, Process},
            patrol::PatrolSpawner,
            raid::Raids,
            save::LevelShape,
            ticket_levels::SpawnTicketKind,
        };
        use sunrise_world::registry::BlockStateId;

        if run_fresh_cases(
            "game::server_side::levels::tests::production_level_swap_exchanges_all_six_subsystems_and_keeps_siblings",
            "SUNRISE_LEVEL_SWAP_CHILD",
            &["swap"],
        ) {
            return;
        }
        mod_platform::testing::with_mods(Vec::new().into(), || {
            with_game(|game| {
                let root = tempfile::tempdir().unwrap();
                game.server.world_root = root.path().to_path_buf();
                game.server.save.0 = Arc::new(SaveDir::new(
                    game.server.world_root.clone(),
                    LevelShape::OVERWORLD,
                    Some(Default::default()),
                ));

                // Populate Overworld level bundles before lazy-opening Nether so we also
                // verify a lazily opened level starts with the default empty bundle state.
                let grass = game
                    .common
                    .pack
                    .assets
                    .registry
                    .default_state("minecraft:grass_block")
                    .unwrap()
                    .0;
                let mut chunk_ow = Chunk::with_layout(-7, -8, game.common.world.layout());
                chunk_ow.set(0, 64, 0, grass);
                let chunk_ow = Arc::new(chunk_ow);
                let facts = game.common.res.facts();
                let behaviours = game.common.res.behaviours();
                game.server.chunk_ticking.tickets.add_portal((-2, -3));
                game.server
                    .chunk_ticking
                    .tickets
                    .add_spawn((-4, -5), SpawnTicketKind::Player);
                let (_, mask_ow) = game
                    .server
                    .chunk_ticking
                    .random_tick_sections
                    .next_masked(
                        (-7, -8),
                        &chunk_ow,
                        facts,
                        |id| behaviours.random_tick_candidate(BlockStateId(id)),
                        0,
                    )
                    .unwrap();
                game.server.knot_state.knot_ids = [([-1, 70, -2], 402)].into_iter().collect();
                game.server.knot_state.broken_knots = vec![[-3, 71, -4]];
                let ow_broken_ptr = game.server.knot_state.broken_knots.as_ptr();
                game.server.lan_queues.block_events = vec![([-5, 72, -6], 4, 5, 6)];
                let ow_block_events_ptr = game.server.lan_queues.block_events.as_ptr();
                game.server.lan_queues.entity_events = vec![(602, 7)];
                let ow_entity_events_ptr = game.server.lan_queues.entity_events.as_ptr();
                game.server.lan_queues.critical_hits = vec![(None, 602, false)];
                let ow_crit_ptr = game.server.lan_queues.critical_hits.as_ptr();

                // Lazy-open Nether from Overworld and verify locator parking, raid load,
                // and default bundle state on the newly opened level.
                let terrain_ow = make_flat_terrain(game, 11111);
                let terrain_nether = make_flat_terrain(game, 22222);
                game.common.end.set_locator(Some(terrain_ow.clone()));
                game.enter_level(Dim::Nether).unwrap();
                assert_eq!(game.dim, Dim::Nether);
                assert_eq!(game.server.raiding.0.level.loaded, Some(Dim::Nether));
                assert!(game.server.levels.0.order.contains(&Dim::Nether));
                assert!(
                    game.server
                        .chunk_ticking
                        .tickets
                        .portal_tickets()
                        .is_empty()
                );
                assert!(
                    game.server
                        .chunk_ticking
                        .tickets
                        .spawn_areas()
                        .next()
                        .is_none()
                );
                assert!(
                    game.server
                        .chunk_ticking
                        .tickets
                        .levels()
                        .random_tick_bounds()
                        .is_none()
                );
                let (_, fresh_nether_mask) = game
                    .server
                    .chunk_ticking
                    .random_tick_sections
                    .next_masked(
                        (-7, -8),
                        &chunk_ow,
                        game.common.res.facts(),
                        |id| {
                            game.common
                                .res
                                .behaviours()
                                .random_tick_candidate(BlockStateId(id))
                        },
                        0,
                    )
                    .unwrap();
                assert!(!Rc::ptr_eq(&fresh_nether_mask, &mask_ow));
                game.server
                    .chunk_ticking
                    .random_tick_sections
                    .remove(&(-7, -8));
                assert!(game.server.knot_state.knot_ids.is_empty());
                assert!(game.server.knot_state.broken_knots.is_empty());
                assert!(game.server.lan_queues.block_events.is_empty());
                assert!(game.server.lan_queues.entity_events.is_empty());
                assert!(game.server.lan_queues.critical_hits.is_empty());
                assert!(Arc::ptr_eq(
                    &game
                        .server
                        .levels
                        .0
                        .parked_locator_of(Dim::Overworld.id())
                        .unwrap(),
                    &terrain_ow
                ));

                // Populate active (Nether) level bundles with distinct payloads.
                let mut chunk_n = Chunk::with_layout(3, 4, game.common.world.layout());
                chunk_n.set(0, 64, 0, grass);
                let chunk_n = Arc::new(chunk_n);
                game.server.chunk_ticking.tickets.add_portal((6, 7));
                game.server
                    .chunk_ticking
                    .tickets
                    .add_spawn((8, 9), SpawnTicketKind::Search);
                let (_, mask_n) = game
                    .server
                    .chunk_ticking
                    .random_tick_sections
                    .next_masked(
                        (3, 4),
                        &chunk_n,
                        game.common.res.facts(),
                        |id| {
                            game.common
                                .res
                                .behaviours()
                                .random_tick_candidate(BlockStateId(id))
                        },
                        0,
                    )
                    .unwrap();
                game.server.knot_state.knot_ids = [([1, 64, 2], 301)].into_iter().collect();
                game.server.knot_state.broken_knots = vec![[3, 65, 4]];
                let n_broken_ptr = game.server.knot_state.broken_knots.as_ptr();
                game.server.lan_queues.block_events = vec![([5, 66, 6], 1, 2, 3)];
                let n_block_events_ptr = game.server.lan_queues.block_events.as_ptr();
                game.server.lan_queues.entity_events = vec![(501, 2)];
                let n_entity_events_ptr = game.server.lan_queues.entity_events.as_ptr();
                game.server.lan_queues.critical_hits = vec![(Some(9), 501, true)];
                let n_crit_ptr = game.server.lan_queues.critical_hits.as_ptr();

                // Populate active (Nether) per-level fields and unswapped siblings.
                game.server.creaking.level.scanned = [(10, 11)].into_iter().collect();
                game.server.creaking.level.unscanned = [(12, 13)].into_iter().collect();
                game.server.creaking.level.revision = Some(101);
                game.server.creaking.hearts = vec![false, true, true];

                game.common.end.set_locator(Some(terrain_nether.clone()));
                let mut traveller = sunrise_sim::end_gateway::Traveller::default();
                traveller.enter_with_delay(BlockPos::new(2, 64, 3), 23);
                game.common.end.gateway_cooldown = 19;
                game.common.end.gateways = [(9, traveller)].into_iter().collect();
                game.common.end.areas = vec![(7, 8)];
                game.common.end.tickets = vec![((9, 10), 40)];
                game.common.end.arrivals = vec![GatewayArrival {
                    traveller: Arriving::Live(77),
                    job: GatewayJob::Known {
                        exit: [1, 2, 3],
                        exact: true,
                    },
                    target: Vec3::new(4.0, 5.0, 6.0),
                }];
                game.common.end.eyes = vec![EyeOfEnder {
                    id: 55,
                    position: glam::DVec3::new(1.0, 2.0, 3.0),
                    previous: glam::DVec3::new(0.5, 1.5, 2.5),
                    velocity: glam::DVec3::new(0.1, 0.2, 0.3),
                    target: glam::DVec3::new(10.0, 20.0, 30.0),
                    life: 12,
                    survive: true,
                }];
                let mut flash = EndFlash::default();
                flash.tick(150);
                flash.x_angle = 11.0;
                flash.y_angle = 22.0;
                let expected_flash_intensity = flash.intensity(0.5);
                game.common.end.flash = flash;

                game.server.portals.level.entities = [(
                    101,
                    PortalState {
                        cooldown: 250,
                        process: Some(Process {
                            entry: BlockPos::new(1, 64, 2),
                            time: 4,
                            inside: true,
                        }),
                    },
                )]
                .into_iter()
                .collect();
                game.server.portals.before = [(11, Vec3::new(1.0, 2.0, 3.0))].into_iter().collect();
                game.server.portals.carrying = vec![(Dim::Nether, 12, BlockPos::new(3, 65, 4))];
                game.server.portals.carried_motion =
                    [(12, Vec3::new(0.1, 0.2, 0.3))].into_iter().collect();
                game.server.portals.carried_cooldown = [(12, 7)].into_iter().collect();
                game.server.portals.carried_at =
                    [(12, Vec3::new(3.5, 65.0, 4.5))].into_iter().collect();
                game.server.portals.loaded = true;
                game.server.portals.dirty = true;

                let shulker_n = Shulker {
                    open_count: 3,
                    status: ShulkerStatus::Opening,
                    progress: 0.5,
                    progress_old: 0.25,
                };
                let bell_n = Bell {
                    ticks: 12,
                    shaking: true,
                    click_direction: Some(2),
                    last_ring_timestamp: 90,
                    nearby: Some(vec![7]),
                    resonating: true,
                    resonation_ticks: 3,
                };
                let gateway_n = EndGateway {
                    age: 300,
                    teleport_cooldown: 20,
                };
                let mut conduit_n = ConduitServer::default();
                conduit_n.target = Some(77);
                game.server.be_anims.level.shulkers =
                    [([1, 2, 3], shulker_n.clone())].into_iter().collect();
                game.server.be_anims.level.bells =
                    [([4, 5, 6], bell_n.clone())].into_iter().collect();
                game.server.be_anims.level.gateway_servers =
                    [([7, 8, 9], gateway_n)].into_iter().collect();
                game.server.be_anims.level.conduit_servers =
                    [([10, 11, 12], conduit_n)].into_iter().collect();
                let expected_kinds = vec![None, Some(Ticker::Bell), Some(Ticker::Conduit)];
                game.server.be_anims.kinds = expected_kinds.clone();

                let mut raids_n = Raids::default();
                let (id_n, _) = raids_n.create_or_extend([10, 64, 20], 2, Some(1), 11);
                raids_n.tick = 111;
                let nbt_n = raids_n.to_nbt();
                game.server.raiding.0.level.raids = raids_n;
                game.server.raiding.0.level.loaded = Some(Dim::Nether);
                game.server.raiding.0.level.unseeded = Some(vec![(id_n, 1)]);
                game.server.raiding.0.level.bar_hurts = [(id_n, 44)].into_iter().collect();
                game.server.raiding.0.level.homes = [(id_n, (111, Arc::new(vec![[1, 64, 2]])))]
                    .into_iter()
                    .collect();
                game.server.raiding.0.patrol = PatrolSpawner { next_tick: 777 };
                game.server.raiding.0.siege.tonight = true;
                game.server.raiding.0.siege.zombies_to_spawn = 9;
                game.server.raiding.0.omen_position = Some([5, 65, 6]);
                game.server.raiding.0.omen_loaded = true;

                game.server.sculk.level.index = Default::default();
                game.server.sculk.level.index.insert((2, 64, 3));
                game.server.sculk.level.known = [(2, 3)].into_iter().collect();
                game.server.sculk.tracker = WardenSpawnTracker {
                    ticks_since_last_warning: 120,
                    warning_level: 2,
                    cooldown_ticks: 45,
                };
                game.server.sculk.tables.built = true;
                game.server.sculk.known_revision = Some(99);
                game.server.sculk.falling_items = [101].into_iter().collect();
                game.server.sculk.damaged = [(14, 500)].into_iter().collect();
                game.server.sculk.last_health = Some(18.5);
                game.server.sculk.actor = Some(Who::Player);
                game.server.sculk.ticks = 77;
                game.server.sculk.tracker_loaded = true;
                game.server.sculk.listeners = vec![3, 5];

                // Populate parked (Overworld) per-level bundles.
                let shulker_ow = Shulker {
                    open_count: 1,
                    status: ShulkerStatus::Opened,
                    progress: 1.0,
                    progress_old: 1.0,
                };
                let bell_ow = Bell {
                    ticks: 5,
                    shaking: false,
                    click_direction: None,
                    last_ring_timestamp: 200,
                    nearby: None,
                    resonating: false,
                    resonation_ticks: 0,
                };
                let gateway_ow = EndGateway {
                    age: 40,
                    teleport_cooldown: 8,
                };
                let mut conduit_ow = ConduitServer::default();
                conduit_ow.target = Some(88);
                let mut raids_ow = Raids::default();
                raids_ow.next_id = 5;
                let (id_ow, _) = raids_ow.create_or_extend([-30, 70, 40], 3, Some(2), 22);
                raids_ow.tick = 222;
                let nbt_ow = raids_ow.to_nbt();
                {
                    let parked = game.server.levels.0.parked_mut(Dim::Overworld).unwrap();
                    parked.creaking.scanned = [(-1, -2)].into_iter().collect();
                    parked.creaking.unscanned = [(-3, -4)].into_iter().collect();
                    parked.creaking.revision = Some(202);
                    parked.end = EndLevelState::new(Some(terrain_ow.clone()));
                    parked.portal_states.entities = [(
                        202,
                        PortalState {
                            cooldown: 33,
                            process: None,
                        },
                    )]
                    .into_iter()
                    .collect();
                    parked.anims.shulkers =
                        [([-1, -2, -3], shulker_ow.clone())].into_iter().collect();
                    parked.anims.bells = [([-4, -5, -6], bell_ow.clone())].into_iter().collect();
                    parked.anims.gateway_servers =
                        [([-7, -8, -9], gateway_ow)].into_iter().collect();
                    parked.anims.conduit_servers =
                        [([-10, -11, -12], conduit_ow)].into_iter().collect();
                    parked.raiding.raids = raids_ow;
                    parked.raiding.loaded = Some(Dim::Overworld);
                    parked.raiding.unseeded = Some(vec![(id_ow, 2)]);
                    parked.raiding.bar_hurts = [(id_ow, 66)].into_iter().collect();
                    parked.raiding.homes = [(id_ow, (222, Arc::new(vec![[-5, 64, -6]])))]
                        .into_iter()
                        .collect();
                    parked.sculk.index = Default::default();
                    parked.sculk.index.insert((-4, 70, -5));
                    parked.sculk.known = [(-4, -5)].into_iter().collect();
                }

                let check_siblings = |g: &crate::game::ServerGame<'_>| {
                    assert_eq!(g.server.creaking.hearts, vec![false, true, true]);
                    assert_eq!(g.common.end.gateway_cooldown, 19);
                    assert_eq!(g.common.end.gateways.len(), 1);
                    let mut gw = g.common.end.gateways[&9];
                    assert_eq!(
                        gw.travel_with_delay(true, 23),
                        Some(BlockPos::new(2, 64, 3))
                    );
                    assert_eq!(g.common.end.areas, vec![(7, 8)]);
                    assert_eq!(g.common.end.tickets, vec![((9, 10), 40)]);
                    assert_eq!(g.common.end.arrivals.len(), 1);
                    assert_eq!(g.common.end.eyes.len(), 1);
                    assert_eq!(g.common.end.eyes[0].id, 55);
                    assert_eq!(g.common.end.flash.x_angle, 11.0);
                    assert_eq!(g.common.end.flash.y_angle, 22.0);
                    assert_eq!(g.common.end.flash.intensity(0.5), expected_flash_intensity);
                    assert_eq!(g.server.portals.before.len(), 1);
                    assert_eq!(
                        g.server.portals.before.get(&11),
                        Some(&Vec3::new(1.0, 2.0, 3.0))
                    );
                    assert_eq!(
                        g.server.portals.carrying,
                        vec![(Dim::Nether, 12, BlockPos::new(3, 65, 4))]
                    );
                    assert_eq!(
                        g.server.portals.carried_motion.get(&12),
                        Some(&Vec3::new(0.1, 0.2, 0.3))
                    );
                    assert_eq!(g.server.portals.carried_cooldown.get(&12), Some(&7));
                    assert_eq!(
                        g.server.portals.carried_at.get(&12),
                        Some(&Vec3::new(3.5, 65.0, 4.5))
                    );
                    assert!(g.server.portals.loaded && g.server.portals.dirty);
                    assert_eq!(g.server.be_anims.kinds, expected_kinds);
                    assert_eq!(g.server.raiding.0.patrol.next_tick, 777);
                    assert!(g.server.raiding.0.siege.tonight);
                    assert_eq!(g.server.raiding.0.siege.zombies_to_spawn, 9);
                    assert_eq!(g.server.raiding.0.omen_position, Some([5, 65, 6]));
                    assert!(g.server.raiding.0.omen_loaded);
                    assert_eq!(g.server.sculk.tracker.warning_level, 2);
                    assert!(g.server.sculk.tables.built);
                    assert_eq!(g.server.sculk.known_revision, Some(99));
                    assert!(g.server.sculk.falling_items.contains(&101));
                    assert_eq!(g.server.sculk.damaged.get(&14), Some(&500));
                    assert_eq!(g.server.sculk.last_health, Some(18.5));
                    assert_eq!(g.server.sculk.actor, Some(Who::Player));
                    assert_eq!(g.server.sculk.ticks, 77);
                    assert!(g.server.sculk.tracker_loaded);
                    assert_eq!(g.server.sculk.listeners, vec![3, 5]);
                };

                // First transition: Nether -> Overworld through production enter_level / swap_level.
                game.enter_level(Dim::Overworld).unwrap();
                assert_eq!(game.dim, Dim::Overworld);
                check_siblings(game);

                assert_eq!(game.server.creaking.level.scanned.len(), 1);
                assert!(game.server.creaking.level.scanned.contains(&(-1, -2)));
                assert_eq!(game.server.creaking.level.unscanned.len(), 1);
                assert!(game.server.creaking.level.unscanned.contains(&(-3, -4)));
                assert_eq!(game.server.creaking.level.revision, Some(202));
                assert!(Arc::ptr_eq(
                    &game.common.end.locator().unwrap(),
                    &terrain_ow
                ));
                assert_eq!(game.server.portals.level.entities.len(), 1);
                assert_eq!(game.server.portals.level.entities[&202].cooldown, 33);
                assert!(game.server.portals.level.entities[&202].process.is_none());
                assert_eq!(game.server.be_anims.level.shulkers.len(), 1);
                assert_eq!(
                    game.server.be_anims.level.shulkers[&[-1, -2, -3]],
                    shulker_ow
                );
                assert_eq!(game.server.be_anims.level.bells.len(), 1);
                assert_eq!(game.server.be_anims.level.bells[&[-4, -5, -6]], bell_ow);
                assert_eq!(game.server.be_anims.level.gateway_servers.len(), 1);
                assert_eq!(
                    game.server.be_anims.level.gateway_servers[&[-7, -8, -9]],
                    gateway_ow
                );
                assert_eq!(game.server.be_anims.level.conduit_servers.len(), 1);
                assert_eq!(
                    game.server.be_anims.level.conduit_servers[&[-10, -11, -12]].target,
                    Some(88)
                );
                assert_eq!(game.server.raiding.0.level.raids.to_nbt(), nbt_ow);
                assert_eq!(game.server.raiding.0.level.loaded, Some(Dim::Overworld));
                assert_eq!(
                    game.server.raiding.0.level.unseeded.as_deref(),
                    Some(&[(id_ow, 2)][..])
                );
                assert_eq!(game.server.raiding.0.level.bar_hurts.len(), 1);
                assert_eq!(game.server.raiding.0.level.bar_hurts.get(&id_ow), Some(&66));
                assert_eq!(game.server.raiding.0.level.homes.len(), 1);
                assert_eq!(game.server.raiding.0.level.homes[&id_ow].0, 222);
                assert_eq!(
                    game.server.raiding.0.level.homes[&id_ow].1.as_slice(),
                    &[[-5, 64, -6]]
                );
                assert_eq!(game.server.sculk.level.index.order, vec![(-4, 70, -5)]);
                assert_eq!(game.server.sculk.level.known.len(), 1);
                assert!(game.server.sculk.level.known.contains(&(-4, -5)));
                assert_eq!(
                    game.server
                        .chunk_ticking
                        .tickets
                        .portal_tickets()
                        .iter()
                        .map(|t| t.chunk)
                        .collect::<Vec<_>>(),
                    vec![(-2, -3)]
                );
                assert_eq!(
                    game.server
                        .chunk_ticking
                        .tickets
                        .spawn_areas()
                        .map(|(c, _)| c)
                        .collect::<Vec<_>>(),
                    vec![(-4, -5)]
                );
                let (_, ow_after_first) = game
                    .server
                    .chunk_ticking
                    .random_tick_sections
                    .next_masked(
                        (-7, -8),
                        &chunk_ow,
                        game.common.res.facts(),
                        |id| {
                            game.common
                                .res
                                .behaviours()
                                .random_tick_candidate(BlockStateId(id))
                        },
                        0,
                    )
                    .unwrap();
                assert!(Rc::ptr_eq(&ow_after_first, &mask_ow));
                assert_eq!(
                    game.server.knot_state.knot_ids.get(&[-1, 70, -2]),
                    Some(&402)
                );
                assert_eq!(game.server.knot_state.knot_ids.len(), 1);
                assert_eq!(game.server.knot_state.broken_knots, vec![[-3, 71, -4]]);
                assert_eq!(game.server.knot_state.broken_knots.as_ptr(), ow_broken_ptr);
                assert_eq!(
                    game.server.lan_queues.block_events,
                    vec![([-5, 72, -6], 4, 5, 6)]
                );
                assert_eq!(
                    game.server.lan_queues.block_events.as_ptr(),
                    ow_block_events_ptr
                );
                assert_eq!(game.server.lan_queues.entity_events, vec![(602, 7)]);
                assert_eq!(
                    game.server.lan_queues.entity_events.as_ptr(),
                    ow_entity_events_ptr
                );
                assert_eq!(
                    game.server.lan_queues.critical_hits,
                    vec![(None, 602, false)]
                );
                assert_eq!(game.server.lan_queues.critical_hits.as_ptr(), ow_crit_ptr);

                {
                    let parked_n = game.server.levels.0.parked.get(&Dim::Nether).unwrap();
                    assert_eq!(parked_n.creaking.scanned.len(), 1);
                    assert!(parked_n.creaking.scanned.contains(&(10, 11)));
                    assert_eq!(parked_n.creaking.unscanned.len(), 1);
                    assert!(parked_n.creaking.unscanned.contains(&(12, 13)));
                    assert_eq!(parked_n.creaking.revision, Some(101));
                    assert!(Arc::ptr_eq(
                        parked_n.end.locator.as_ref().unwrap(),
                        &terrain_nether
                    ));
                    assert!(
                        game.server
                            .levels
                            .0
                            .parked_terrains()
                            .any(|t| Arc::ptr_eq(t, &terrain_nether))
                    );
                    assert_eq!(parked_n.portal_states.entities.len(), 1);
                    let ps_n = &parked_n.portal_states.entities[&101];
                    assert_eq!(ps_n.cooldown, 250);
                    assert_eq!(
                        ps_n.process.as_ref().map(|p| p.entry),
                        Some(BlockPos::new(1, 64, 2))
                    );
                    assert_eq!(parked_n.anims.shulkers.len(), 1);
                    assert_eq!(parked_n.anims.shulkers[&[1, 2, 3]], shulker_n);
                    assert_eq!(parked_n.anims.bells.len(), 1);
                    assert_eq!(parked_n.anims.bells[&[4, 5, 6]], bell_n);
                    assert_eq!(parked_n.anims.gateway_servers.len(), 1);
                    assert_eq!(parked_n.anims.gateway_servers[&[7, 8, 9]], gateway_n);
                    assert_eq!(parked_n.anims.conduit_servers.len(), 1);
                    assert_eq!(
                        parked_n.anims.conduit_servers[&[10, 11, 12]].target,
                        Some(77)
                    );
                    assert_eq!(parked_n.raiding.raids.to_nbt(), nbt_n);
                    assert_eq!(parked_n.raiding.loaded, Some(Dim::Nether));
                    assert_eq!(parked_n.raiding.unseeded.as_deref(), Some(&[(id_n, 1)][..]));
                    assert_eq!(parked_n.raiding.bar_hurts.len(), 1);
                    assert_eq!(parked_n.raiding.bar_hurts.get(&id_n), Some(&44));
                    assert_eq!(parked_n.raiding.homes.len(), 1);
                    assert_eq!(parked_n.raiding.homes[&id_n].0, 111);
                    assert_eq!(parked_n.raiding.homes[&id_n].1.as_slice(), &[[1, 64, 2]]);
                    assert_eq!(parked_n.sculk.index.order, vec![(2, 64, 3)]);
                    assert_eq!(parked_n.sculk.known.len(), 1);
                    assert!(parked_n.sculk.known.contains(&(2, 3)));
                    assert_eq!(
                        parked_n
                            .chunk_ticking
                            .tickets
                            .portal_tickets()
                            .iter()
                            .map(|t| t.chunk)
                            .collect::<Vec<_>>(),
                        vec![(6, 7)]
                    );
                    assert_eq!(
                        parked_n
                            .chunk_ticking
                            .tickets
                            .spawn_areas()
                            .map(|(c, _)| c)
                            .collect::<Vec<_>>(),
                        vec![(8, 9)]
                    );
                    assert_eq!(parked_n.knot_state.knot_ids.get(&[1, 64, 2]), Some(&301));
                    assert_eq!(parked_n.knot_state.knot_ids.len(), 1);
                    assert_eq!(parked_n.knot_state.broken_knots, vec![[3, 65, 4]]);
                    assert_eq!(parked_n.knot_state.broken_knots.as_ptr(), n_broken_ptr);
                    assert_eq!(
                        parked_n.lan_queues.block_events,
                        vec![([5, 66, 6], 1, 2, 3)]
                    );
                    assert_eq!(
                        parked_n.lan_queues.block_events.as_ptr(),
                        n_block_events_ptr
                    );
                    assert_eq!(parked_n.lan_queues.entity_events, vec![(501, 2)]);
                    assert_eq!(
                        parked_n.lan_queues.entity_events.as_ptr(),
                        n_entity_events_ptr
                    );
                    assert_eq!(
                        parked_n.lan_queues.critical_hits,
                        vec![(Some(9), 501, true)]
                    );
                    assert_eq!(parked_n.lan_queues.critical_hits.as_ptr(), n_crit_ptr);
                }

                // Return transition: Overworld -> Nether restores both sides completely.
                game.enter_level(Dim::Nether).unwrap();
                assert_eq!(game.dim, Dim::Nether);
                check_siblings(game);

                assert_eq!(game.server.creaking.level.scanned.len(), 1);
                assert!(game.server.creaking.level.scanned.contains(&(10, 11)));
                assert_eq!(game.server.creaking.level.unscanned.len(), 1);
                assert!(game.server.creaking.level.unscanned.contains(&(12, 13)));
                assert_eq!(game.server.creaking.level.revision, Some(101));
                assert!(Arc::ptr_eq(
                    &game.common.end.locator().unwrap(),
                    &terrain_nether
                ));
                assert_eq!(game.server.portals.level.entities.len(), 1);
                let ps_n = &game.server.portals.level.entities[&101];
                assert_eq!(ps_n.cooldown, 250);
                assert_eq!(
                    ps_n.process.as_ref().map(|p| p.entry),
                    Some(BlockPos::new(1, 64, 2))
                );
                assert_eq!(game.server.be_anims.level.shulkers.len(), 1);
                assert_eq!(game.server.be_anims.level.shulkers[&[1, 2, 3]], shulker_n);
                assert_eq!(game.server.be_anims.level.bells.len(), 1);
                assert_eq!(game.server.be_anims.level.bells[&[4, 5, 6]], bell_n);
                assert_eq!(game.server.be_anims.level.gateway_servers.len(), 1);
                assert_eq!(
                    game.server.be_anims.level.gateway_servers[&[7, 8, 9]],
                    gateway_n
                );
                assert_eq!(game.server.be_anims.level.conduit_servers.len(), 1);
                assert_eq!(
                    game.server.be_anims.level.conduit_servers[&[10, 11, 12]].target,
                    Some(77)
                );
                assert_eq!(game.server.raiding.0.level.raids.to_nbt(), nbt_n);
                assert_eq!(game.server.raiding.0.level.loaded, Some(Dim::Nether));
                assert_eq!(
                    game.server.raiding.0.level.unseeded.as_deref(),
                    Some(&[(id_n, 1)][..])
                );
                assert_eq!(game.server.raiding.0.level.bar_hurts.len(), 1);
                assert_eq!(game.server.raiding.0.level.bar_hurts.get(&id_n), Some(&44));
                assert_eq!(game.server.raiding.0.level.homes.len(), 1);
                assert_eq!(game.server.raiding.0.level.homes[&id_n].0, 111);
                assert_eq!(
                    game.server.raiding.0.level.homes[&id_n].1.as_slice(),
                    &[[1, 64, 2]]
                );
                assert_eq!(game.server.sculk.level.index.order, vec![(2, 64, 3)]);
                assert_eq!(game.server.sculk.level.known.len(), 1);
                assert!(game.server.sculk.level.known.contains(&(2, 3)));
                assert_eq!(
                    game.server
                        .chunk_ticking
                        .tickets
                        .portal_tickets()
                        .iter()
                        .map(|t| t.chunk)
                        .collect::<Vec<_>>(),
                    vec![(6, 7)]
                );
                assert_eq!(
                    game.server
                        .chunk_ticking
                        .tickets
                        .spawn_areas()
                        .map(|(c, _)| c)
                        .collect::<Vec<_>>(),
                    vec![(8, 9)]
                );
                let (_, n_after_return) = game
                    .server
                    .chunk_ticking
                    .random_tick_sections
                    .next_masked(
                        (3, 4),
                        &chunk_n,
                        game.common.res.facts(),
                        |id| {
                            game.common
                                .res
                                .behaviours()
                                .random_tick_candidate(BlockStateId(id))
                        },
                        0,
                    )
                    .unwrap();
                assert!(Rc::ptr_eq(&n_after_return, &mask_n));
                assert_eq!(game.server.knot_state.knot_ids.get(&[1, 64, 2]), Some(&301));
                assert_eq!(game.server.knot_state.knot_ids.len(), 1);
                assert_eq!(game.server.knot_state.broken_knots, vec![[3, 65, 4]]);
                assert_eq!(game.server.knot_state.broken_knots.as_ptr(), n_broken_ptr);
                assert_eq!(
                    game.server.lan_queues.block_events,
                    vec![([5, 66, 6], 1, 2, 3)]
                );
                assert_eq!(
                    game.server.lan_queues.block_events.as_ptr(),
                    n_block_events_ptr
                );
                assert_eq!(game.server.lan_queues.entity_events, vec![(501, 2)]);
                assert_eq!(
                    game.server.lan_queues.entity_events.as_ptr(),
                    n_entity_events_ptr
                );
                assert_eq!(
                    game.server.lan_queues.critical_hits,
                    vec![(Some(9), 501, true)]
                );
                assert_eq!(game.server.lan_queues.critical_hits.as_ptr(), n_crit_ptr);

                let parked_ow = game.server.levels.0.parked.get(&Dim::Overworld).unwrap();
                assert_eq!(parked_ow.creaking.scanned.len(), 1);
                assert!(parked_ow.creaking.scanned.contains(&(-1, -2)));
                assert_eq!(parked_ow.creaking.unscanned.len(), 1);
                assert!(parked_ow.creaking.unscanned.contains(&(-3, -4)));
                assert_eq!(parked_ow.creaking.revision, Some(202));
                assert!(Arc::ptr_eq(
                    parked_ow.end.locator.as_ref().unwrap(),
                    &terrain_ow
                ));
                assert_eq!(parked_ow.portal_states.entities.len(), 1);
                assert_eq!(parked_ow.portal_states.entities[&202].cooldown, 33);
                assert!(parked_ow.portal_states.entities[&202].process.is_none());
                assert_eq!(parked_ow.anims.shulkers.len(), 1);
                assert_eq!(parked_ow.anims.shulkers[&[-1, -2, -3]], shulker_ow);
                assert_eq!(parked_ow.anims.bells.len(), 1);
                assert_eq!(parked_ow.anims.bells[&[-4, -5, -6]], bell_ow);
                assert_eq!(parked_ow.anims.gateway_servers.len(), 1);
                assert_eq!(parked_ow.anims.gateway_servers[&[-7, -8, -9]], gateway_ow);
                assert_eq!(parked_ow.anims.conduit_servers.len(), 1);
                assert_eq!(
                    parked_ow.anims.conduit_servers[&[-10, -11, -12]].target,
                    Some(88)
                );
                assert_eq!(parked_ow.raiding.raids.to_nbt(), nbt_ow);
                assert_eq!(parked_ow.raiding.loaded, Some(Dim::Overworld));
                assert_eq!(
                    parked_ow.raiding.unseeded.as_deref(),
                    Some(&[(id_ow, 2)][..])
                );
                assert_eq!(parked_ow.raiding.bar_hurts.len(), 1);
                assert_eq!(parked_ow.raiding.bar_hurts.get(&id_ow), Some(&66));
                assert_eq!(parked_ow.raiding.homes.len(), 1);
                assert_eq!(parked_ow.raiding.homes[&id_ow].0, 222);
                assert_eq!(
                    parked_ow.raiding.homes[&id_ow].1.as_slice(),
                    &[[-5, 64, -6]]
                );
                assert_eq!(parked_ow.sculk.index.order, vec![(-4, 70, -5)]);
                assert_eq!(parked_ow.sculk.known.len(), 1);
                assert!(parked_ow.sculk.known.contains(&(-4, -5)));
                assert_eq!(
                    parked_ow
                        .chunk_ticking
                        .tickets
                        .portal_tickets()
                        .iter()
                        .map(|t| t.chunk)
                        .collect::<Vec<_>>(),
                    vec![(-2, -3)]
                );
                assert_eq!(
                    parked_ow
                        .chunk_ticking
                        .tickets
                        .spawn_areas()
                        .map(|(c, _)| c)
                        .collect::<Vec<_>>(),
                    vec![(-4, -5)]
                );
                assert_eq!(parked_ow.knot_state.knot_ids.get(&[-1, 70, -2]), Some(&402));
                assert_eq!(parked_ow.knot_state.knot_ids.len(), 1);
                assert_eq!(parked_ow.knot_state.broken_knots, vec![[-3, 71, -4]]);
                assert_eq!(parked_ow.knot_state.broken_knots.as_ptr(), ow_broken_ptr);
                assert_eq!(
                    parked_ow.lan_queues.block_events,
                    vec![([-5, 72, -6], 4, 5, 6)]
                );
                assert_eq!(
                    parked_ow.lan_queues.block_events.as_ptr(),
                    ow_block_events_ptr
                );
                assert_eq!(parked_ow.lan_queues.entity_events, vec![(602, 7)]);
                assert_eq!(
                    parked_ow.lan_queues.entity_events.as_ptr(),
                    ow_entity_events_ptr
                );
                assert_eq!(parked_ow.lan_queues.critical_hits, vec![(None, 602, false)]);
                assert_eq!(parked_ow.lan_queues.critical_hits.as_ptr(), ow_crit_ptr);
            });
        });
    }
}
