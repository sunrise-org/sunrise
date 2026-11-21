//! Client ticks of mod block entities (`ClientLevel.tickBlockEntities`): the client's own
//! ticker list, made from `native_block_entity_client_ticks`.
use super::*;
use mod_api::ModBlockEntity;
use sunrise_client::block_entities::{ClientSnapshot, Ticking};
use sunrise_client::containers::BlockEntity;

/// How many client tick snapshots were made, for the tests of their reuse.
#[cfg(test)]
pub(in crate::game) static CLIENT_SNAPSHOTS_MADE: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);

impl Game<'_> {
    /// Whether block `block`'s block entity ticks on the client
    /// (`EntityBlock.getTicker` asked with the client level): set `ticks` when
    /// it is yours, and `native_block_entity_client_tick` runs for it once per
    /// client tick. Asked once for each mod block, for all of its states,
    /// the first time a client level ticks with a part that has a client tick.
    /// Set `ticks` to true to make it tick, and never clear a value an earlier mod or mixin set.
    pub(in crate::game) fn native_block_entity_client_ticks(&self, block: &str, ticks: &mut bool) {
        self.fire_block_entity_client_ticks(block, ticks);
    }
    /// A client tick of this client's copy of the mod block entity at `pos`
    /// (`ClientLevel.tickBlockEntities`), when its `id` is yours and its
    /// block's state ticks on the client (`native_block_entity_client_ticks`):
    /// once per client tick while the level runs, in vanilla's ticker order
    /// among the others, whether or not the copy is drawn. `block_entity` is
    /// its handle, `state` its block's state and `entity` its data, read only,
    /// as the frame's handling of the server's packets merged it before the
    /// frame's ticks (`native_block_entity_update_tag`). Start, stop and fade
    /// its sounds and add its particles here. Never called on a server, and
    /// never again for a copy removed or whose chunk unloaded.
    pub(in crate::game) fn native_block_entity_client_tick(
        &mut self,
        pos: [i32; 3],
        block_entity: BlockEntityRef,
        state: BlockStateId,
        entity: &ModBlockEntity,
    ) {
        self.fire_block_entity_client_tick(pos, block_entity, state, entity);
    }
}

impl Game<'_> {
    /// `ClientLevel.tickBlockEntities` on a server's client, for the block entities with a client
    /// ticker: spawners, trial spawners, vaults and mod ones. The client's ticker list
    /// (`Level.blockEntityTickers`, kept in its world's block entities as a server keeps its own)
    /// is taken once, at the tick's start, so a ticker made during the tick waits for the next;
    /// each live one inside the world border then ticks in its order, as they all draw from the
    /// one level random. A mod one is told only while a part has a client tick and there is a
    /// client level.
    pub(in crate::game) fn tick_client_block_entities(&mut self) {
        if !self.world.block_entities.has_states() {
            let states = self.client_ticking_states();
            self.world.block_entities.set_states(states);
        }
        let level = mod_api::__private::has_part_hook(mod_api::HookId::BlockEntityClientTick)
            .then(|| self.native_client_level())
            .flatten();
        let tickers = self.world.block_entity_tickers();
        // Each mod copy's snapshot is made once per revision and lent to every tick until the
        // copy changes (an update merged into it), so an unchanged copy's tick allocates nothing.
        let mut snapshots = match level {
            Some(_) => self.world.block_entities.take_client_snapshots(),
            None => Default::default(),
        };
        for &ticker in tickers.iter() {
            let key @ (x, _, z) = ticker.0;
            // Ticks before this one can remove it (`TickingBlockEntity.isRemoved`), and one
            // outside the border waits (`LevelChunk.isTicking`).
            if !self.world.block_entities.is_live(ticker)
                || !self.border.contains(f64::from(x), f64::from(z))
            {
                continue;
            }
            match self.world.block_entities.get(&key) {
                Some(BlockEntity::Spawner(_)) => self.client_tick_spawner(key),
                Some(BlockEntity::TrialSpawner(_)) => self.client_tick_trial_spawner(key),
                // VaultBlock.getTicker on the client: VaultBlockEntity.Client.tick.
                Some(BlockEntity::Vault(_)) => self.client_tick_vault(key),
                Some(BlockEntity::Mod(_)) => {
                    if let Some(level) = level {
                        self.client_tick_mod_block_entity(level, key, &mut snapshots);
                    }
                }
                _ => {}
            }
        }
        if level.is_some() {
            self.world.block_entities.keep_client_snapshots(snapshots);
        }
        self.forget_spawner_displays();
    }
    /// The client tick of the mod copy at `pos`, with its snapshot from `snapshots`, made again
    /// when its revision changed.
    fn client_tick_mod_block_entity(
        &mut self,
        level: mod_api::LevelRef,
        pos: (i32, i32, i32),
        snapshots: &mut sunrise_client::fast_hash::FxHashMap<(i32, i32, i32), ClientSnapshot>,
    ) {
        let (x, y, z) = pos;
        let block_entities = &self.world.block_entities;
        let (Some(BlockEntity::Mod(entity)), Some(generation)) =
            (block_entities.get(&pos), block_entities.generation(&pos))
        else {
            return;
        };
        let revision = entity.revision();
        if snapshots
            .get(&pos)
            .is_none_or(|snapshot| snapshot.revision != revision)
        {
            #[cfg(test)]
            CLIENT_SNAPSHOTS_MADE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let snapshot = entity
                .try_into_api()
                .map_err(|error| {
                    log::warn!(
                        "Sunrise: the client copy of {} at {x} {y} {z} has no snapshot for its client tick: {error:?}",
                        entity.id()
                    );
                })
                .ok();
            snapshots.insert(
                pos,
                ClientSnapshot {
                    revision,
                    entity: snapshot,
                },
            );
        }
        let Some(entity) = snapshots.get(&pos).and_then(|s| s.entity.as_ref()) else {
            return;
        };
        let state = self.world.block(x, y, z);
        let handle = mod_api::__private::game::block_entity_ref(level, [x, y, z], generation);
        self.native_block_entity_client_tick([x, y, z], handle, state, entity);
    }
    /// `BlockState.getTicker` with the client level, for every block state: spawners, trial
    /// spawners and vaults always, a mod block's states when a part with a client tick says so
    /// (asked once per block), every other state never.
    fn client_ticking_states(&self) -> Vec<Ticking> {
        let registry = self.pack.assets.registry;
        let mod_ticks = mod_api::__private::has_part_hook(mod_api::HookId::BlockEntityClientTick);
        let mut asked: std::collections::HashMap<&str, bool> = std::collections::HashMap::new();
        (0..registry.len())
            .map(|i| {
                let name: &str = &self.res.facts().get(BlockStateId(i as u32)).name;
                if name.starts_with("minecraft:") {
                    return match name {
                        "minecraft:spawner" | "minecraft:trial_spawner" | "minecraft:vault" => {
                            Ticking::Yes
                        }
                        _ => Ticking::No,
                    };
                }
                if !mod_ticks {
                    return Ticking::No;
                }
                let ticks = *asked.entry(name).or_insert_with(|| {
                    let mut ticks = false;
                    self.native_block_entity_client_ticks(name, &mut ticks);
                    ticks
                });
                if ticks { Ticking::Yes } else { Ticking::No }
            })
            .collect()
    }
}
