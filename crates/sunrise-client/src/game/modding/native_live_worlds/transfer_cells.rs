//! Checked actual live payload/read witnesses; callbacks and durable decisions belong to Game.
use super::{Error, Game, state_shape};
use mod_api::{LevelRef, LiveWorldTransferCell, LiveWorldTransferLimits};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};
use sunrise_client::{
    chunk::Chunk,
    containers::{BlockEntity, FurnaceKind},
    save::block_entity_encode::{self, DataFields},
    world::World,
};
use sunrise_core::{
    nbt::{Compound, Tag},
    position::BlockPos,
    ticks::{TransferOccurrence, TransferTickError, TransferTicks},
};
use sunrise_world::{level::Ticks, registry::BlockStateId};

#[path = "transfer_cell_snapshots.rs"]
mod snapshots;
type Pos = [i32; 3];
type Key = (i32, i32, i32);
fn key(p: Pos) -> Key {
    (p[0], p[1], p[2])
}
fn pos(p: BlockPos) -> Pos {
    [p.x, p.y, p.z]
}
fn point(p: Pos) -> BlockPos {
    BlockPos::new(p[0], p[1], p[2])
}
fn column(p: Pos) -> (i32, i32) {
    (p[0] >> 4, p[2] >> 4)
}
fn transfer_attachment_error(
    error: sunrise_mod_runtime::attachments::AttachmentEncodeError,
) -> Error {
    log::warn!(
        "Sunrise: live-world transfer could not serialize a block entity attachment: {error}"
    );
    Error::Storage
}

fn tick_error(e: TransferTickError) -> Error {
    #[cfg(test)]
    eprintln!("LIVE_WORLD_TRANSFER actual schedule conversion: {e:?}");
    match e {
        TransferTickError::Busy => Error::Busy,
        TransferTickError::MissingChunk => Error::NotResident,
        TransferTickError::Changed => Error::Revision,
        TransferTickError::Occupied => Error::Obstructed,
        TransferTickError::Budget => Error::Budget,
    }
}

#[derive(PartialEq, Eq)]
struct CellWitness {
    state: BlockStateId,
    block_name: String,
    generation: Option<u64>,
    modeled: Option<Tag>,
    attachments: Option<Compound>,
    name: Option<Tag>,
    original: Option<Tag>,
}
struct Move {
    mapping: LiveWorldTransferCell,
    source: CellWitness,
}
struct LevelWitness {
    revision: u64,
    content: u64,
    time: i64,
    order_cursor: i64,
    air: BlockStateId,
    columns: Vec<((i32, i32), usize)>,
    from: BTreeSet<Pos>,
    to: BTreeSet<Pos>,
    blocks: TransferTicks,
    fluids: TransferTicks,
    incoming_orders: std::ops::Range<i64>,
}
struct Imported {
    source: LevelRef,
    source_fluid: bool,
    source_index: usize,
    destination: LevelRef,
    after: TransferOccurrence,
}
pub(in crate::game) struct PreparedCells {
    moves: Vec<Move>,
    levels: BTreeMap<LevelRef, LevelWitness>,
    columns: BTreeMap<LevelRef, Vec<(i32, i32)>>,
    imported: Vec<Imported>,
    maximum_bytes: u64,
}
impl PreparedCells {
    pub(in crate::game) fn affected_columns(&self) -> &BTreeMap<LevelRef, Vec<(i32, i32)>> {
        &self.columns
    }
    /// Clamp common actual serialized afterimage/WAL admission to this caller budget.
    pub(in crate::game) fn maximum_bytes(&self) -> u64 {
        self.maximum_bytes
    }
}

fn metadata_fields(tag: &Tag, expected_id: &str, furnace: bool) -> Result<(), Error> {
    let Tag::Compound(data) = tag else {
        return Err(Error::Unsupported);
    };
    for (name, value) in data {
        let name = name.to_string().map_err(|_| Error::Unsupported)?;
        match name.as_str() {
            "id" => {
                if !matches!(value, Tag::String(id) if id.to_string().ok().as_deref() == Some(expected_id))
                {
                    return Err(Error::Unsupported);
                }
            }
            "x" | "y" | "z" if matches!(value, Tag::Int(_)) => {}
            "keepPacked"
            | "components"
            | "CustomName"
            | "lock"
            | "Lock"
            | "Items"
            | "sunrise:attachments"
            | "sunrise:stack_limit" => {}
            "lit_time_remaining" | "lit_total_time" | "cooking_time_spent"
            | "cooking_total_time" | "speed_multiplier" | "RecipesUsed"
                if furnace => {}
            // LootTable and unknown position-sensitive fields are never silently transported.
            _ => return Err(Error::Unsupported),
        }
    }
    Ok(())
}
fn raw_at_or_unknown(tag: &Tag, p: Pos) -> bool {
    match [tag.get("x"), tag.get("y"), tag.get("z")] {
        [Some(Tag::Int(x)), Some(Tag::Int(y)), Some(Tag::Int(z))] => [*x, *y, *z] == p,
        // Unknown position metadata in an affected chunk cannot prove vacancy/safe transport.
        _ => true,
    }
}
fn latent_snapshot_rows(chunk: &Chunk, p: Pos, allow_modeled: bool) -> bool {
    if chunk.block_entities.iter().any(|(at, _)| *at == key(p)) {
        return true;
    }
    let Some(saved) = &chunk.saved else {
        return false;
    };
    if saved
        .block_entities
        .iter()
        .any(|tag| raw_at_or_unknown(tag, p))
        || saved.pending_loot.contains_key(&key(p))
        || saved
            .block_entity_save_edits
            .as_ref()
            .is_some_and(|m| m.contains_key(&key(p)))
    {
        return true;
    }
    !allow_modeled
        && (saved
            .block_entity_sources
            .iter()
            .any(|(at, _)| *at == key(p))
            || saved
                .block_entity_attachments
                .iter()
                .any(|(at, _)| *at == key(p))
            || saved.block_entity_order.contains(&key(p)))
}
fn resident(world: &World, ticks: &Ticks, p: Pos) -> Result<(), Error> {
    let chunk = world.chunks.get(&column(p)).ok_or(Error::NotResident)?;
    if chunk.section_index(p[1]).is_none() || world.entities_pending.contains(&column(p)) {
        return Err(Error::NotResident);
    }
    if ticks.blocks.chunk(point(p).chunk()).is_none()
        || ticks.fluids.chunk(point(p).chunk()).is_none()
    {
        return Err(Error::NotResident);
    }
    Ok(())
}

impl Game<'_> {
    fn source_cell_witness(&self, world: &World, p: Pos) -> Result<CellWitness, Error> {
        let state = world.block(p[0], p[1], p[2]);
        let canonical = self
            .pack
            .assets
            .registry
            .state(state)
            .ok_or(Error::Unsupported)?;
        let facts = self.res.facts().get(state);
        let at = key(p);
        if world.block_entities.is_taken(&at) || world.block_entities.carried(&at).is_some() {
            return Err(Error::Busy);
        }
        if world.block_entities.mod_container(state.0).is_some()
            || world.block_entities.mod_fluids(state.0).is_some()
            || world.block_entities.mod_energy(state.0).is_some()
            || world.block_entities.stateful_entity(state.0)
        {
            return Err(Error::Unsupported);
        }
        let entity = world.block_entities.get(&at);
        let kind = entity.map(BlockEntity::id);
        match mod_api::__private::game::transfer_parts_present(&canonical.name, kind) {
            Some(false) => {}
            _result => {
                #[cfg(test)]
                eprintln!(
                    "LIVE_WORLD_TRANSFER unsupported actual Parts lookup {_result:?} for {}",
                    canonical.name
                );
                return Err(Error::Unsupported);
            }
        }
        if facts.waterlogged || !matches!(facts.fluid, sunrise_client::blocks::Fluid::None) {
            return Err(Error::Unsupported);
        }
        let shape = state_shape(self, state)?;
        let furnace = match (canonical.name.as_str(), entity) {
            ("minecraft:chest" | "minecraft:trapped_chest", Some(BlockEntity::Chest(slots)))
                if slots.len() == 27 =>
            {
                if canonical.properties.get("type").map(String::as_str) != Some("single") {
                    return Err(Error::Unsupported);
                }
                if world
                    .chest_openers
                    .get(&at)
                    .is_some_and(|(count, _)| *count != 0)
                {
                    return Err(Error::Busy);
                }
                false
            }
            ("minecraft:furnace", Some(BlockEntity::Furnace(f)))
                if f.kind == FurnaceKind::Furnace =>
            {
                true
            }
            ("minecraft:blast_furnace", Some(BlockEntity::Furnace(f)))
                if f.kind == FurnaceKind::BlastFurnace =>
            {
                true
            }
            ("minecraft:smoker", Some(BlockEntity::Furnace(f)))
                if f.kind == FurnaceKind::Smoker =>
            {
                true
            }
            (_, None)
                if !facts.block_entity
                    && (shape.is_block()
                        || facts.classes.iter().any(|class| &**class == "SlabBlock"))
                    && !facts.dynamic_light
                    && !facts.random_ticks
                    && facts.classes.iter().all(|class| {
                        matches!(
                            &**class,
                            "Block" | "BlockBehaviour" | "RotatedPillarBlock" | "SlabBlock"
                        )
                    }) =>
            {
                false
            }
            // Ender chests own player context; unknown/contextual states stay outside this slice.
            _ => {
                #[cfg(test)]
                eprintln!(
                    "LIVE_WORLD_TRANSFER unsupported stock class {} {:?}",
                    canonical.name, facts.classes
                );
                return Err(Error::Unsupported);
            }
        };
        let extra = world.block_entity_extras(at);
        if extra.pending_loot.is_some() {
            return Err(Error::Unsupported);
        }
        let chunk = world.chunks.get(&column(p)).ok_or(Error::NotResident)?;
        if latent_snapshot_rows(chunk, p, entity.is_some()) {
            return Err(Error::Unsupported);
        }
        if let Some(entity) = entity {
            let expected = sunrise_client::save::block_entity_id(entity, &canonical.name);
            if sunrise_client::block_entity_types::factory_valid(&canonical.name) != Some(true)
                || sunrise_client::block_entity_types::valid(&canonical.name, expected)
                    != Some(true)
                || sunrise_client::lan::chunks::block_entity_type(&canonical.name) != Some(expected)
            {
                #[cfg(test)]
                eprintln!(
                    "LIVE_WORLD_TRANSFER actual factory admission {} valid={:?} kind={:?} expected={expected}",
                    canonical.name,
                    sunrise_client::block_entity_types::factory_valid(&canonical.name),
                    sunrise_client::lan::chunks::block_entity_type(&canonical.name)
                );
                return Err(Error::Unsupported);
            }
            if let Some(original) = &extra.source {
                metadata_fields(
                    original,
                    sunrise_client::save::block_entity_id(entity, &canonical.name),
                    furnace,
                )?;
            }
        } else if extra.source.is_some()
            || world.block_names.contains_key(&at)
            || world.block_entities.attachments(at).is_some()
            || world.block_entities.generation(&at).is_some()
        {
            return Err(Error::Unsupported);
        }
        Ok(CellWitness {
            state,
            block_name: canonical.name.clone(),
            generation: world.block_entities.generation(&at),
            modeled: entity.map(|entity| {
                block_entity_encode::data(
                    at,
                    entity,
                    &canonical.name,
                    DataFields {
                        lock: world.block_lock(at),
                        ..Default::default()
                    },
                    self.res.items(),
                )
            }),
            attachments: world
                .block_entities
                .attachments(at)
                .map(|map| map.try_to_nbt())
                .transpose()
                .map_err(transfer_attachment_error)?,
            name: world.block_names.get(&at).cloned(),
            original: extra.source,
        })
    }
    fn transfer_destination_empty(&self, world: &World, p: Pos) -> Result<(), Error> {
        let at = key(p);
        let chunk = world.chunks.get(&column(p)).ok_or(Error::NotResident)?;
        if world.block(p[0], p[1], p[2]) != world.air
            || world.block_entities.generation(&at).is_some()
            || world.block_entities.get(&at).is_some()
            || world.block_entities.is_taken(&at)
            || world.block_entities.carried(&at).is_some()
            || world.block_entities.attachments(at).is_some()
            || world.block_names.contains_key(&at)
            || world.chest_openers.contains_key(&at)
            || latent_snapshot_rows(chunk, p, false)
            || chunk.saved.as_ref().is_some_and(|saved| {
                saved
                    .block_ticks
                    .iter()
                    .chain(&saved.fluid_ticks)
                    .any(|tick| pos(tick.pos) == p)
            })
        {
            return Err(Error::Obstructed);
        }
        Ok(())
    }
    pub(super) fn prepare_transfer_cells(
        &self,
        cells: &[LiveWorldTransferCell],
        limits: LiveWorldTransferLimits,
    ) -> Result<PreparedCells, Error> {
        if cells.is_empty() || cells.len() > limits.cells as usize || limits.bytes == 0 {
            return Err(Error::Budget);
        }
        let mut from = BTreeSet::new();
        let mut to = BTreeSet::new();
        let mut columns = BTreeMap::<LevelRef, BTreeSet<(i32, i32)>>::new();
        for cell in cells {
            // Ordinary World dirtying touches immediate X/Z neighbours.
            for p in [cell.from, cell.to] {
                for n in [p[0], p[2]] {
                    n.checked_sub(1).ok_or(Error::Invalid)?;
                    n.checked_add(1).ok_or(Error::Invalid)?;
                }
            }
            if !from.insert((cell.source, cell.from)) || !to.insert((cell.destination, cell.to)) {
                return Err(Error::Invalid);
            }
            columns
                .entry(cell.source)
                .or_default()
                .insert(column(cell.from));
            columns
                .entry(cell.destination)
                .or_default()
                .insert(column(cell.to));
        }
        if from.iter().any(|p| to.contains(p)) {
            return Err(Error::Invalid);
        }
        if columns.len() > limits.levels as usize
            || columns.values().map(BTreeSet::len).sum::<usize>() > limits.chunks as usize
        {
            return Err(Error::Budget);
        }
        let mut moves = Vec::with_capacity(cells.len());
        let mut boxes = 0_usize;
        let mut witness_bytes = 0u64;
        for cell in cells {
            let (source, source_ticks) =
                self.transfer_level_parts(cell.source).ok_or(Error::Stale)?;
            let (destination, destination_ticks) = self
                .transfer_level_parts(cell.destination)
                .ok_or(Error::Stale)?;
            resident(source, source_ticks, cell.from)?;
            resident(destination, destination_ticks, cell.to)?;
            if !self.live_world_tick_ready(cell.source, cell.from)
                || !self.live_world_tick_ready(cell.destination, cell.to)
            {
                return Err(Error::NotResident);
            }
            self.transfer_destination_empty(destination, cell.to)?;
            let witness = self
                .source_cell_witness(source, cell.from)
                .inspect_err(|_error| {
                    #[cfg(test)]
                    eprintln!(
                        "LIVE_WORLD_TRANSFER actual source witness at {:?}: {_error:?}",
                        cell.from
                    );
                })?;
            for tag in witness
                .modeled
                .iter()
                .chain(&witness.original)
                .chain(&witness.name)
                .cloned()
                .chain(witness.attachments.iter().cloned().map(Tag::Compound))
            {
                let bytes = sunrise_core::nbt::write_named(
                    &sunrise_core::nbt::NamedTag {
                        name: "".into(),
                        tag,
                    },
                    sunrise_core::nbt::Limits {
                        max_bytes: limits.bytes.min(16 << 20) as usize,
                        ..sunrise_core::nbt::Limits::saved()
                    },
                )
                .map_err(|_| Error::Budget)?;
                witness_bytes = witness_bytes
                    .checked_add(bytes.len() as u64)
                    .ok_or(Error::Budget)?;
                if witness_bytes > limits.bytes {
                    return Err(Error::Budget);
                }
            }
            boxes = boxes
                .checked_add(state_shape(self, witness.state)?.boxes().len())
                .ok_or(Error::Budget)?;
            if boxes > limits.boxes as usize {
                return Err(Error::Budget);
            }
            moves.push(Move {
                mapping: *cell,
                source: witness,
            });
        }
        let mut levels = BTreeMap::new();
        let mut count = 0_usize;
        for (&level, cols) in &columns {
            let (world, ticks) = self.transfer_level_parts(level).ok_or(Error::Stale)?;
            let from: BTreeSet<_> = cells
                .iter()
                .filter(|c| c.source == level)
                .map(|c| c.from)
                .collect();
            let to: BTreeSet<_> = cells
                .iter()
                .filter(|c| c.destination == level)
                .map(|c| c.to)
                .collect();
            let selected = from.union(&to).copied().collect();
            if !ticks.transfer_settled(&selected) {
                #[cfg(test)]
                eprintln!(
                    "LIVE_WORLD_TRANSFER pending actual scheduler {} {:?}",
                    level.dimension_id(),
                    ticks.transfer_busy_bits(&selected)
                );
                return Err(Error::Busy);
            }
            let blocks = ticks
                .blocks
                .capture_transfer_cells(&from, limits.scheduled_ticks as usize)
                .map_err(|error| {
                    #[cfg(test)]
                    eprintln!(
                        "LIVE_WORLD_TRANSFER source capture {}: {error:?}",
                        level.dimension_id()
                    );
                    tick_error(error)
                })?;
            let fluids = ticks
                .fluids
                .capture_transfer_cells(&from, limits.scheduled_ticks as usize)
                .map_err(|error| {
                    #[cfg(test)]
                    eprintln!(
                        "LIVE_WORLD_TRANSFER source capture {}: {error:?}",
                        level.dimension_id()
                    );
                    tick_error(error)
                })?;
            for schedule in [&ticks.blocks, &ticks.fluids] {
                match schedule.capture_transfer_cells(&to, 0) {
                    Ok(_) => {}
                    Err(TransferTickError::Budget) => return Err(Error::Obstructed),
                    Err(error) => {
                        #[cfg(test)]
                        eprintln!(
                            "LIVE_WORLD_TRANSFER destination capture {}: {error:?}",
                            level.dimension_id()
                        );
                        return Err(tick_error(error));
                    }
                }
            }
            count = count
                .checked_add(blocks.occurrences().len())
                .and_then(|n| n.checked_add(fluids.occurrences().len()))
                .ok_or(Error::Budget)?;
            if count > limits.scheduled_ticks as usize {
                return Err(Error::Budget);
            }
            levels.insert(
                level,
                LevelWitness {
                    revision: world.revision,
                    content: world.block_entities.content_revision(),
                    time: world.game_time,
                    order_cursor: ticks.transfer_order_cursor(),
                    air: world.air,
                    columns: cols
                        .iter()
                        .map(|c| (*c, Arc::as_ptr(world.chunks.get(c).unwrap()) as usize))
                        .collect(),
                    from,
                    to,
                    blocks,
                    fluids,
                    incoming_orders: 0..0,
                },
            );
        }
        let mut imported = Vec::with_capacity(count);
        let by_source: BTreeMap<_, _> = cells
            .iter()
            .map(|cell| ((cell.source, cell.from), cell))
            .collect();
        for (&level, witness) in &levels {
            let mut source: Vec<_> = witness
                .blocks
                .occurrences()
                .iter()
                .enumerate()
                .map(|(i, t)| (false, i, *t))
                .chain(
                    witness
                        .fluids
                        .occurrences()
                        .iter()
                        .enumerate()
                        .map(|(i, t)| (true, i, *t)),
                )
                .collect();
            source.sort_by_key(|(fluid, i, t)| (t.tick.sub_tick_order, *fluid, *i));
            for (fluid, index, mut occurrence) in source {
                let mapping = by_source
                    .get(&(level, pos(occurrence.tick.pos)))
                    .ok_or(Error::Invalid)?;
                let destination_time = levels[&mapping.destination].time;
                let delay = occurrence
                    .tick
                    .trigger_tick
                    .checked_sub(witness.time)
                    .ok_or(Error::Invalid)?;
                i32::try_from(delay).map_err(|_| Error::Invalid)?;
                occurrence.tick.trigger_tick =
                    destination_time.checked_add(delay).ok_or(Error::Invalid)?;
                occurrence.tick.pos = point(mapping.to);
                imported.push(Imported {
                    source: level,
                    source_fluid: fluid,
                    source_index: index,
                    destination: mapping.destination,
                    after: occurrence,
                });
            }
        }
        for (&level, witness) in &mut levels {
            let (_, ticks) = self.transfer_level_parts(level).ok_or(Error::Stale)?;
            let count = imported.iter().filter(|t| t.destination == level).count();
            let range = ticks.transfer_order_range(count).map_err(tick_error)?;
            for (order, tick) in range
                .clone()
                .zip(imported.iter_mut().filter(|t| t.destination == level))
            {
                tick.after.tick.sub_tick_order = order;
            }
            witness.incoming_orders = range;
        }
        // Actual serialized full afterimage/WAL bytes are bounded by the lead's common freeze path.
        Ok(PreparedCells {
            moves,
            levels,
            columns: columns
                .into_iter()
                .map(|(l, c)| (l, c.into_iter().collect()))
                .collect(),
            imported,
            maximum_bytes: limits.bytes,
        })
    }
    pub(super) fn validate_transfer_cells(&self, plan: &PreparedCells) -> Result<(), Error> {
        for (&level, witness) in &plan.levels {
            let (world, ticks) = self.transfer_level_parts(level).ok_or(Error::Stale)?;
            let selected = witness.from.union(&witness.to).copied().collect();
            if !ticks.transfer_settled(&selected) {
                #[cfg(test)]
                eprintln!(
                    "LIVE_WORLD_TRANSFER pending actual scheduler {} {:?}",
                    level.dimension_id(),
                    ticks.transfer_busy_bits(&selected)
                );
                return Err(Error::Busy);
            }
            if world.revision != witness.revision
                || world.block_entities.content_revision() != witness.content
                || world.game_time != witness.time
                || ticks.transfer_order_cursor() != witness.order_cursor
                || witness.columns.iter().any(|(c, id)| {
                    world
                        .chunks
                        .get(c)
                        .is_none_or(|chunk| Arc::as_ptr(chunk) as usize != *id)
                })
            {
                return Err(Error::Revision);
            }
            let imported_count = usize::try_from(
                witness
                    .incoming_orders
                    .end
                    .checked_sub(witness.incoming_orders.start)
                    .ok_or(Error::Revision)?,
            )
            .map_err(|_| Error::Revision)?;
            if ticks
                .transfer_order_range(imported_count)
                .map_err(tick_error)?
                != witness.incoming_orders
            {
                return Err(Error::Revision);
            }
            ticks
                .blocks
                .validate_transfer_cells(&witness.blocks)
                .map_err(tick_error)?;
            ticks
                .fluids
                .validate_transfer_cells(&witness.fluids)
                .map_err(tick_error)?;
            for schedule in [&ticks.blocks, &ticks.fluids] {
                match schedule.capture_transfer_cells(&witness.to, 0) {
                    Ok(_) => {}
                    Err(TransferTickError::Budget) => return Err(Error::Revision),
                    Err(error) => {
                        #[cfg(test)]
                        eprintln!(
                            "LIVE_WORLD_TRANSFER destination capture {}: {error:?}",
                            level.dimension_id()
                        );
                        return Err(tick_error(error));
                    }
                }
            }
        }
        for cell in &plan.moves {
            let (world, _) = self
                .transfer_level_parts(cell.mapping.source)
                .ok_or(Error::Stale)?;
            if self
                .source_cell_witness(world, cell.mapping.from)
                .map_err(|_| Error::Revision)?
                != cell.source
            {
                return Err(Error::Revision);
            }
            let (world, _) = self
                .transfer_level_parts(cell.mapping.destination)
                .ok_or(Error::Stale)?;
            self.transfer_destination_empty(world, cell.mapping.to)
                .map_err(|_| Error::Revision)?;
            if !self.live_world_tick_ready(cell.mapping.source, cell.mapping.from)
                || !self.live_world_tick_ready(cell.mapping.destination, cell.mapping.to)
            {
                return Err(Error::NotResident);
            }
        }
        Ok(())
    }
    pub(super) fn transfer_cell_afterimages(
        &self,
        plan: &PreparedCells,
        snapshots: &mut BTreeMap<LevelRef, Vec<Arc<Chunk>>>,
    ) -> Result<(), Error> {
        self.validate_transfer_cells(plan)?;
        snapshots::rewrite(self, plan, snapshots)
    }
    pub(super) fn apply_prepared_transfer_cells(
        &mut self,
        plan: PreparedCells,
    ) -> Result<(), Error> {
        self.validate_transfer_cells(&plan)?; // Whole batch, before the first take.
        let mut owned_ticks = BTreeMap::new();
        for (&level, witness) in &plan.levels {
            let (_, ticks) = self.transfer_level_parts_mut(level).ok_or(Error::Stale)?;
            owned_ticks.insert(
                (level, false),
                ticks
                    .blocks
                    .extract_transfer_cells(&witness.blocks)
                    .map_err(tick_error)?
                    .into_iter()
                    .map(Some)
                    .collect::<Vec<_>>(),
            );
            owned_ticks.insert(
                (level, true),
                ticks
                    .fluids
                    .extract_transfer_cells(&witness.fluids)
                    .map_err(tick_error)?
                    .into_iter()
                    .map(Some)
                    .collect::<Vec<_>>(),
            );
        }
        let mut payloads = Vec::with_capacity(plan.moves.len());
        for cell in &plan.moves {
            let (world, _) = self
                .transfer_level_parts_mut(cell.mapping.source)
                .ok_or(Error::Stale)?;
            let at = key(cell.mapping.from);
            let mut payload = if cell.source.generation.is_some() {
                Some(
                    world
                        .block_entities
                        .extract_owned(at)
                        .ok_or(Error::Revision)?,
                )
            } else {
                None
            };
            let metadata = super::super::piston_carry_state::take_position_data(world, at);
            if let Some(payload) = &mut payload {
                payload.position_data = metadata;
            }
            snapshots::clear_live_rows(world, at);
            world
                .set(at.0, at.1, at.2, world.air)
                .map_err(|_| Error::Revision)?;
            payloads.push(payload);
        }
        for (cell, payload) in plan.moves.iter().zip(payloads) {
            let (world, _) = self
                .transfer_level_parts_mut(cell.mapping.destination)
                .ok_or(Error::Stale)?;
            let at = key(cell.mapping.to);
            world
                .set(at.0, at.1, at.2, cell.source.state)
                .map_err(|_| Error::Revision)?;
            if let Some(payload) = payload {
                let metadata = world
                    .block_entities
                    .install_owned(at, payload)
                    .map_err(|_| Error::Revision)?;
                super::super::piston_carry_state::land_position_data(world, at, &metadata);
                world.attachment_landings.insert(
                    sunrise_mod_runtime::attachments::Holder::BlockEntity([at.0, at.1, at.2]),
                );
            }
        }
        let mut incoming = BTreeMap::<(LevelRef, bool), Vec<TransferOccurrence>>::new();
        for imported in &plan.imported {
            let source = owned_ticks
                .get_mut(&(imported.source, imported.source_fluid))
                .ok_or(Error::Revision)?
                .get_mut(imported.source_index)
                .ok_or(Error::Revision)?
                .take()
                .ok_or(Error::Revision)?;
            // Mutate the just-removed actual primitive occurrence; never ordinary schedule() dedup.
            let mut moved = source;
            moved.tick.pos = imported.after.tick.pos;
            moved.tick.trigger_tick = imported.after.tick.trigger_tick;
            moved.tick.sub_tick_order = imported.after.tick.sub_tick_order;
            incoming
                .entry((imported.destination, imported.source_fluid))
                .or_default()
                .push(moved);
        }
        if owned_ticks
            .values()
            .any(|ticks| ticks.iter().any(Option::is_some))
        {
            return Err(Error::Revision);
        }
        for (&level, witness) in &plan.levels {
            let (_, ticks) = self.transfer_level_parts_mut(level).ok_or(Error::Stale)?;
            // Source extraction can remove the previous maximum: reserve the prevalidated exact
            // range, not a newly recomputed range. The helper below checks the captured cursor.
            ticks
                .reserve_prepared_transfer_orders(
                    witness.order_cursor,
                    witness.incoming_orders.clone(),
                )
                .map_err(tick_error)?;
            ticks
                .blocks
                .install_transfer_occurrences(incoming.remove(&(level, false)).unwrap_or_default())
                .map_err(tick_error)?;
            ticks
                .fluids
                .install_transfer_occurrences(incoming.remove(&(level, true)).unwrap_or_default())
                .map_err(tick_error)?;
        }
        Ok(())
    }
}
