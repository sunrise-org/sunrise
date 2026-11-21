//! Saved-data loads on the owning server thread, distinct from attachment and client sync.
use super::{BlockEntity, Game, GameLevel, Pos};
use crate::game::native_level_signals::ReadLevel;
use mod_api::{BlockEntityRef, BlockEntityTypeKey, HookId, LevelRef};
use sunrise_client::world::World;
use sunrise_core::nbt::Compound;

fn loaded_payload(
    world: &World,
    level: LevelRef,
    pos: Pos,
) -> Option<(BlockEntityTypeKey, BlockEntityRef, &Compound)> {
    let BlockEntity::Mod(data) = world.block_entities.get(&pos)? else {
        return None;
    };
    let id = mod_api::__private::block_entity_type_key(data.id())?;
    let generation = world.block_entities.generation(&pos)?;
    let entity =
        mod_api::__private::game::block_entity_ref(level, [pos.0, pos.1, pos.2], generation);
    Some((id, entity, &data.nbt))
}

impl Game<'_> {
    /// An explicit engine load into an attached instance.
    pub(in crate::game) fn notify_block_entity_load_additional(&self, pos: Pos) {
        if self.is_logical_server()
            && mod_api::__private::has_part_hook(HookId::BlockEntityLoadAdditional)
            && let Some(level) = self.level(self.dim.id())
            && let Some((id, entity, tag)) = loaded_payload(&self.world, level, pos)
        {
            let host = ReadLevel { game: self, level };
            let pos = [pos.0, pos.1, pos.2];
            mod_api::dispatch_block_entity_load_additional(&host, id, entity, pos, level, tag);
        }
    }

    /// Chunk workers decode data; only the level thread tells the Parts, after
    /// the instances and their attachments are in the world.
    pub(in crate::game) fn fire_pending_block_entity_data_loads(&mut self) {
        if !self.world.block_entities.has_pending_data_load() {
            return;
        }
        while let Some((pos, generation)) = self.world.block_entities.next_data_load() {
            if self.world.block_entities.generation(&pos) == Some(generation) {
                self.notify_block_entity_load_additional(pos);
            }
        }
    }
}

impl GameLevel<'_> {
    pub(in crate::game) fn notify_block_entity_load_additional(&self, pos: Pos) {
        if self.native_authoritative
            && mod_api::__private::has_part_hook(HookId::BlockEntityLoadAdditional)
            && let Some(level) = self.api_level
            && let Some((id, entity, tag)) = loaded_payload(self.world, level, pos)
        {
            let pos = [pos.0, pos.1, pos.2];
            self.native_block_entity_load_additional(id, entity, pos, level, tag);
        }
    }

    /// The level's `BlockEntity.loadAdditional` notice once the payload is installed.
    pub(in crate::game) fn native_block_entity_load_additional(
        &self,
        id: mod_api::BlockEntityTypeKey,
        entity: BlockEntityRef,
        pos: [i32; 3],
        level: LevelRef,
        tag: &Compound,
    ) {
        self.fire_block_entity_load_additional(id, entity, pos, level, tag);
    }
}
