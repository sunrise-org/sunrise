//! Chunk copies are I/O afterimages for the common writer, never live owners.
use super::*;
use sunrise_client::chunk::ChunkExtras;
use sunrise_mod_runtime::attachments::Attachments;

struct SavedPayload {
    entity: Option<BlockEntity>,
    name: Option<Tag>,
    source: Option<Tag>,
    attachments: Option<Attachments>,
    edits: Option<Compound>,
    ordered: bool,
    limit: Option<u32>,
}
fn take_at<T>(items: &mut Vec<(Key, T)>, at: Key) -> Option<T> {
    let index = items.iter().position(|(p, _)| *p == at)?;
    Some(items.remove(index).1)
}
fn retarget(tag: &mut Tag, at: Key) -> Result<(), Error> {
    let Tag::Compound(data) = tag else {
        return Err(Error::Unsupported);
    };
    for (name, value) in [("x", at.0), ("y", at.1), ("z", at.2)] {
        data.insert(name.into(), Tag::Int(value));
    }
    Ok(())
}
fn extras(chunk: &mut Chunk) -> &mut ChunkExtras {
    Arc::make_mut(
        chunk
            .saved
            .get_or_insert_with(|| Arc::new(ChunkExtras::default())),
    )
}
fn chunk(
    all: &BTreeMap<LevelRef, Vec<Arc<Chunk>>>,
    level: LevelRef,
    p: Pos,
) -> Result<&Chunk, Error> {
    all.get(&level)
        .and_then(|chunks| chunks.iter().find(|c| (c.x, c.z) == column(p)))
        .map(Arc::as_ref)
        .ok_or(Error::NotResident)
}
fn chunk_mut(
    all: &mut BTreeMap<LevelRef, Vec<Arc<Chunk>>>,
    level: LevelRef,
    p: Pos,
) -> Result<&mut Chunk, Error> {
    all.get_mut(&level)
        .and_then(|chunks| chunks.iter_mut().find(|c| (c.x, c.z) == column(p)))
        .map(Arc::make_mut)
        .ok_or(Error::NotResident)
}
fn saved_expected(
    witness: &TransferTicks,
    column: (i32, i32),
    time: i64,
) -> Vec<sunrise_core::ticks::SavedTick> {
    witness
        .occurrences()
        .iter()
        .filter(|t| super::column(pos(t.tick.pos)) == column)
        .map(|t| t.tick.saved(time))
        .collect()
}
fn validate(
    game: &Game<'_>,
    plan: &PreparedCells,
    all: &BTreeMap<LevelRef, Vec<Arc<Chunk>>>,
) -> Result<(), Error> {
    for (&level, expected) in &plan.columns {
        let chunks = all.get(&level).ok_or(Error::NotResident)?;
        let mut seen = BTreeSet::new();
        if chunks.iter().any(|c| !seen.insert((c.x, c.z))) {
            return Err(Error::Invalid);
        }
        let witness = &plan.levels[&level];
        for &(x, z) in expected {
            let c = chunk(all, level, [x * 16, 0, z * 16])?;
            let saved = c.saved.as_ref().ok_or(Error::Revision)?;
            if saved.last_update != witness.time {
                return Err(Error::Revision);
            }
            for (actual, schedule) in [
                (&saved.block_ticks, &witness.blocks),
                (&saved.fluid_ticks, &witness.fluids),
            ] {
                let source: Vec<_> = actual
                    .iter()
                    .filter(|t| witness.from.contains(&pos(t.pos)))
                    .copied()
                    .collect();
                if source != saved_expected(schedule, (x, z), witness.time)
                    || actual.iter().any(|t| witness.to.contains(&pos(t.pos)))
                {
                    return Err(Error::Revision);
                }
            }
        }
    }
    for cell in &plan.moves {
        let source = chunk(all, cell.mapping.source, cell.mapping.from)?;
        let at = key(cell.mapping.from);
        if source.block(at.0 & 15, at.1, at.2 & 15) != cell.source.state {
            return Err(Error::Revision);
        }
        let count = source
            .block_entities
            .iter()
            .filter(|(p, _)| *p == at)
            .count();
        if count != usize::from(cell.source.generation.is_some()) {
            return Err(Error::Revision);
        }
        let modeled = source
            .block_entities
            .iter()
            .find(|(p, _)| *p == at)
            .map(|(_, entity)| {
                block_entity_encode::data(
                    at,
                    entity,
                    &cell.source.block_name,
                    DataFields {
                        lock: cell
                            .source
                            .original
                            .as_ref()
                            .and_then(|tag| tag.get("lock")),
                        ..Default::default()
                    },
                    game.res.items(),
                )
            });
        if modeled != cell.source.modeled
            || source
                .block_names
                .iter()
                .find(|(p, _)| *p == at)
                .map(|(_, tag)| tag)
                != cell.source.name.as_ref()
        {
            return Err(Error::Revision);
        }
        let destination = chunk(all, cell.mapping.destination, cell.mapping.to)?;
        let dest = key(cell.mapping.to);
        if destination.block(dest.0 & 15, dest.1, dest.2 & 15)
            != plan.levels[&cell.mapping.destination].air
            || destination.block_entities.iter().any(|(p, _)| *p == dest)
            || destination.block_names.iter().any(|(p, _)| *p == dest)
            || latent_snapshot_rows(destination, cell.mapping.to, false)
        {
            return Err(Error::Obstructed);
        }
        let saved = source.saved.as_ref().ok_or(Error::Revision)?;
        let attachment = saved
            .block_entity_attachments
            .iter()
            .find(|(p, _)| *p == at)
            .map(|(_, map)| map.try_to_nbt())
            .transpose()
            .map_err(transfer_attachment_error)?;
        if attachment != cell.source.attachments {
            return Err(Error::Revision);
        }
        let original = saved
            .block_entity_sources
            .iter()
            .find(|(p, _)| *p == at)
            .map(|(_, t)| t);
        if original != cell.source.original.as_ref() {
            return Err(Error::Revision);
        }
        if let Some(edits) = saved
            .block_entity_save_edits
            .as_ref()
            .and_then(|m| m.get(&at))
        {
            let modeled = cell.source.modeled.as_ref().ok_or(Error::Unsupported)?;
            let Some(Tag::String(id)) = modeled.get("id") else {
                return Err(Error::Unsupported);
            };
            let id = id.to_string().map_err(|_| Error::Unsupported)?;
            metadata_fields(
                &Tag::Compound(edits.clone()),
                &id,
                matches!(
                    source
                        .block_entities
                        .iter()
                        .find(|(p, _)| *p == at)
                        .map(|(_, e)| e),
                    Some(BlockEntity::Furnace(_))
                ),
            )?;
        }
    }
    Ok(())
}
fn take_saved(chunk: &mut Chunk, at: Key, air: BlockStateId) -> SavedPayload {
    let entity = take_at(&mut chunk.block_entities, at);
    let name = take_at(&mut chunk.block_names, at);
    let limit = chunk.container_stack_limits.remove(&at);
    let saved = extras(chunk);
    let source = take_at(&mut saved.block_entity_sources, at);
    let attachments = take_at(&mut saved.block_entity_attachments, at);
    let edits = saved
        .block_entity_save_edits
        .as_mut()
        .and_then(|e| Arc::make_mut(e).remove(&at));
    let ordered = saved.block_entity_order.contains(&at);
    saved.block_entity_order.retain(|p| *p != at);
    saved.pending_loot.remove(&at);
    chunk.set(at.0 & 15, at.1, at.2 & 15, air.0);
    chunk.lit = false;
    chunk.light_loaded = false;
    SavedPayload {
        entity,
        name,
        source,
        attachments,
        edits,
        ordered,
        limit,
    }
}
fn put_saved(
    chunk: &mut Chunk,
    at: Key,
    state: BlockStateId,
    mut payload: SavedPayload,
) -> Result<(), Error> {
    chunk.set(at.0 & 15, at.1, at.2 & 15, state.0);
    chunk.lit = false;
    chunk.light_loaded = false;
    if let Some(entity) = payload.entity {
        chunk.block_entities.push((at, entity));
    }
    if let Some(name) = payload.name {
        chunk.block_names.push((at, name));
    }
    if let Some(limit) = payload.limit {
        chunk.container_stack_limits.insert(at, limit);
    }
    let saved = extras(chunk);
    if let Some(source) = &mut payload.source {
        retarget(source, at)?;
    }
    if let Some(source) = payload.source {
        saved.block_entity_sources.push((at, source));
    }
    if let Some(attachments) = payload.attachments {
        saved.block_entity_attachments.push((at, attachments));
    }
    if let Some(mut edits) = payload.edits {
        for (name, value) in [("x", at.0), ("y", at.1), ("z", at.2)] {
            if edits.contains_key(&name.into()) {
                edits.insert(name.into(), Tag::Int(value));
            }
        }
        Arc::make_mut(
            saved
                .block_entity_save_edits
                .get_or_insert_with(Default::default),
        )
        .insert(at, edits);
    }
    if payload.ordered {
        saved.block_entity_order.push(at);
    }
    Ok(())
}
pub(super) fn rewrite(
    game: &Game<'_>,
    plan: &PreparedCells,
    all: &mut BTreeMap<LevelRef, Vec<Arc<Chunk>>>,
) -> Result<(), Error> {
    validate(game, plan, all)?; // Entire I/O batch before editing its first copy.
    let mut payloads = Vec::with_capacity(plan.moves.len());
    for cell in &plan.moves {
        let source = chunk_mut(all, cell.mapping.source, cell.mapping.from)?;
        payloads.push(take_saved(
            source,
            key(cell.mapping.from),
            plan.levels[&cell.mapping.source].air,
        ));
    }
    for (cell, payload) in plan.moves.iter().zip(payloads) {
        put_saved(
            chunk_mut(all, cell.mapping.destination, cell.mapping.to)?,
            key(cell.mapping.to),
            cell.source.state,
            payload,
        )?;
    }
    for (&level, witness) in &plan.levels {
        for c in all.get_mut(&level).ok_or(Error::NotResident)? {
            let saved = extras(Arc::make_mut(c));
            saved
                .block_ticks
                .retain(|t| !witness.from.contains(&pos(t.pos)));
            saved
                .fluid_ticks
                .retain(|t| !witness.from.contains(&pos(t.pos)));
        }
    }
    for imported in &plan.imported {
        let time = plan.levels[&imported.destination].time;
        let c = chunk_mut(all, imported.destination, pos(imported.after.tick.pos))?;
        let saved = extras(c);
        let ticks = if imported.source_fluid {
            &mut saved.fluid_ticks
        } else {
            &mut saved.block_ticks
        };
        ticks.push(imported.after.tick.saved(time));
    }
    Ok(())
}
pub(super) fn clear_live_rows(world: &mut World, at: Key) {
    world.with_chunk_extras(at, |saved| {
        saved.block_entity_sources.retain(|(p, _)| *p != at);
        saved.block_entity_attachments.retain(|(p, _)| *p != at);
        saved.block_entity_order.retain(|p| *p != at);
        saved
            .block_entities
            .retain(|tag| !raw_at_or_unknown(tag, [at.0, at.1, at.2]));
        saved.pending_loot.remove(&at);
    });
    world.block_names.remove(&at);
    let holder = sunrise_mod_runtime::attachments::Holder::BlockEntity([at.0, at.1, at.2]);
    world.attachment_syncs.remove(&holder);
    world.attachment_landings.remove(&holder);
}

#[cfg(test)]
#[path = "transfer_cell_snapshot_tests.rs"]
mod tests;
