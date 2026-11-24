//! Read-only owner-control diagnostics over actual source Worlds, never replica inventories.
use super::*;
use serde_json::{Value, json};
use sunrise_client::containers::BlockEntity;

fn pose_json(pose: LiveWorldPose) -> Value {
    json!({"translation": pose.translation, "pivot": pose.pivot, "rotation": pose.rotation,
        "translation_bits": pose.translation.map(f64::to_bits),
        "pivot_bits": pose.pivot.map(f64::to_bits), "rotation_bits": pose.rotation.map(f64::to_bits)})
}

impl Game<'_> {
    /// Runs on the existing authoritative owner link between ordinary server turns.
    /// A bounded refusal returns no partial snapshot and never loads a missing source.
    pub(in crate::game) fn control_live_worlds(&self, source: Option<&str>) -> Value {
        let snapshot = (|| -> Result<Value, Error> {
            if !sunrise_client::join_seq::visibility::passes_settled() {
                return Err(Error::Busy);
            }
            let table = self.live_worlds.borrow();
            let selected: Vec<_> = table
                .entries
                .values()
                .filter(|entry| source.is_none_or(|id| entry.handle.level().dimension_id() == id))
                .collect();
            let count = selected.iter().try_fold(0usize, |count, entry| {
                count.checked_add(entry.cells.len()).ok_or(Error::Budget)
            })?;
            // Diagnostic bounds, separate from configurable live ownership reservations.
            if count > 65536 {
                return Err(Error::Budget);
            }
            let mut box_count = 0usize;
            let mut inventory_slots = 0usize;
            let mut projections = Vec::with_capacity(selected.len());
            for entry in selected {
                if !self.live_world_lifecycle_current(entry.handle.level())
                    || !self.live_world_lifecycle_current(entry.destination)
                {
                    return Err(Error::Stale);
                }
                let world = self
                    .native_level_world(entry.handle.level())
                    .ok_or(Error::Stale)?;
                let mut cells = Vec::with_capacity(entry.cells.len());
                for &pos in &entry.cells {
                    let state = world.block(pos[0], pos[1], pos[2]);
                    let shape = shape(self, world, pos)?;
                    box_count = box_count
                        .checked_add(shape.boxes().len())
                        .ok_or(Error::Budget)?;
                    if box_count > 262144 {
                        return Err(Error::Budget);
                    }
                    let collision: Vec<_> = shape.boxes().iter().map(|bounds| {
                        json!({"min": bounds.min_position().components(), "max": bounds.max_position().components()})
                    }).collect();
                    let entity = world.block_entities.get(&(pos[0], pos[1], pos[2]));
                    let contents = if let Some(entity) = entity {
                        inventory_slots = inventory_slots
                            .checked_add(entity.slots().len())
                            .ok_or(Error::Budget)?;
                        if inventory_slots > 65536 {
                            return Err(Error::Budget);
                        }
                        let slots: Vec<_> = entity
                            .slots()
                            .iter()
                            .map(|slot| {
                                slot.as_ref().map(|stack| {
                            json!({"item": stack.item.as_ref(), "count": stack.count})
                        })
                            })
                            .collect();
                        let progress = match entity {
                            BlockEntity::Furnace(furnace) => Some(json!({
                                "cooking_timer": furnace.cooking_timer, "cooking_total": furnace.cooking_total,
                                "lit_remaining": furnace.lit_remaining, "lit_total": furnace.lit_total})),
                            _ => None,
                        };
                        Some(json!({"type": entity.id(), "slots": slots, "progress": progress}))
                    } else {
                        None
                    };
                    let canonical = self
                        .pack
                        .assets
                        .registry
                        .state(state)
                        .ok_or(Error::Unsupported)?;
                    cells.push(json!({"pos": pos, "state": state.0, "name": canonical.name,
                        "properties": canonical.properties, "collision": collision, "block_entity": contents}));
                }
                projections.push(json!({"owner": entry.owner, "generation": entry.handle.generation(),
                    "world_epoch": entry.handle.level().epoch().get(), "source": entry.handle.level().dimension_id(),
                    "source_generation": entry.handle.level().generation(), "destination": entry.destination.dimension_id(),
                    "destination_generation": entry.destination.generation(), "membership_revision": entry.membership,
                    "source_revision": world.revision, "content_revision": world.block_entities.content_revision(),
                    "source_game_time": world.game_time, "sample": {"revision": entry.sample.revision,
                        "previous": pose_json(entry.sample.previous), "current": pose_json(entry.sample.current)},
                    "cells": cells}));
            }
            let capture = self.live_world_durability.last_capture.as_ref().map(|capture| {
                if let Some(usage) = capture.usage {
                    json!({"epoch": capture.epoch, "fits_byte_budget": capture.fits_byte_budget,
                        "exact": true, "limit_bytes": capture.limit, "required_bytes": usage.serialized_bytes,
                        "payload_bytes": usage.payload_bytes, "witness_bytes": usage.witness_bytes,
                        "records": usage.records, "levels": usage.levels,
                        "terrain_records": usage.terrain_records, "entity_records": usage.entity_records,
                        "poi_records": usage.poi_records, "metadata_records": usage.metadata_records})
                } else {
                    let (bytes, records) = capture.payload_bound.unwrap_or_default();
                    json!({"epoch": capture.epoch, "fits_byte_budget": false, "exact": false,
                        "limit_bytes": capture.limit, "required_bytes": null,
                        "required_bytes_at_least": bytes, "records_encoded": records,
                        "stage": "frozen payload"})
                }
            });
            Ok(
                json!({"schema": 1, "projection_count": projections.len(), "cell_count": count,
                "projections": projections, "last_checkpoint_capture": capture}),
            )
        })();
        match snapshot {
            Ok(value) => value,
            Err(error) => json!({"error": format!("{error:?}")}),
        }
    }
}
