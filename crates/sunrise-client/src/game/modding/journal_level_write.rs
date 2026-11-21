//! The level in place during a transaction. Reads see tentative storage;
//! updates, scheduled ticks and effects are published only at the root commit.
use super::Game;
use super::api_values::{TryIntoApi, TryIntoEngine};
use super::creatures::NativeSound;
use super::lan_particles::QueuedParticle;
use super::native_level_signals::ReadLevel;
use super::transactions::After;
use mod_api::{
    AttachmentBatch, AttachmentStore, BlockEntityRef, BlockKey, BlockPos, BlockSide, BlockState,
    BlockUpdateFlags, Compound, Direction, FeatureFlags, FluidState, ItemStack, LegacyRandomSource,
    LevelHost, LevelRandomHost, LevelRef, LevelTicksHost, LevelWriteHost, LightLayer,
    PartStateError, ParticleOptions, Tags,
};
use sunrise_core::ticks::TickPriority;
use sunrise_world::{
    fluid::FluidKind,
    level::{FluidWrite, LevelAccess as _, flags, states::BlockId},
    registry::BlockStateId,
};

pub(super) struct JournalLevelWrite<'g, 'a> {
    game: &'g mut Game<'a>,
    level: LevelRef,
}

impl<'g, 'a> JournalLevelWrite<'g, 'a> {
    pub(super) fn new(game: &'g mut Game<'a>, level: LevelRef) -> Option<Self> {
        (game.level_in_place(level) && game.transaction_current().is_some())
            .then_some(Self { game, level })
    }

    fn read(&self) -> ReadLevel<'_, 'a> {
        ReadLevel {
            game: self.game,
            level: self.level,
        }
    }

    pub(super) fn write_block(&mut self, pos: [i32; 3], state: BlockStateId, flags: i32) -> bool {
        let [x, y, z] = pos;
        if self.game.world_debug
            || !(-30_000_000..30_000_000).contains(&x)
            || !(-30_000_000..30_000_000).contains(&z)
            || !self.game.world.is_loaded_at(x, y, z)
            || self.game.world.carrier_fences.locks_cell(&pos)
            || self.game.pack.assets.registry.state(state).is_none()
        {
            return false;
        }
        let old = self.game.world.block(x, y, z);
        if old == state || !self.game.journal_set_block_flags(pos, state, flags, true) {
            return false;
        }
        let b = self.game.res.behaviours();
        if !b.keeps_block_entity(old, state)
            && self.game.world.block_entities.get(&(x, y, z)).is_some()
        {
            self.game.journal_block_replacement((x, y, z));
        }
        true
    }

    fn queue(&mut self, write: PendingWrite) -> bool {
        self.game.after_commit(After::LevelWrite(Box::new(write)));
        true
    }
}

macro_rules! read_level {
    ($(fn $method:ident(&self $(, $arg:ident: $ty:ty)*) -> $result:ty;)*) => {$(
        fn $method(&self $(, $arg: $ty)*) -> $result {
            self.read().$method($($arg),*)
        }
    )*};
}

impl LevelHost for JournalLevelWrite<'_, '_> {
    fn block_states(&self) -> mod_api::BlockStates<'_> {
        mod_api::BlockStates::of(&*self.game)
    }
    read_level! {
        fn block_entity(&self, pos: [i32; 3]) -> Option<BlockEntityRef>;
        fn container_size(&self, at: BlockSide) -> Option<usize>;
        fn container_stack(&self, at: BlockSide, slot: usize) -> Option<ItemStack>;
        fn block_entity_part_snapshot(&self, entity: BlockEntityRef, namespace: &str, name: &str) -> Result<AttachmentBatch, PartStateError>;
        fn block_state(&self, pos: [i32; 3]) -> Option<BlockState>;
        fn block_state_id(&self, name: &str) -> Option<BlockState>;
        fn block_state_default(&self, block: BlockKey) -> Option<BlockState>;
        fn state_string(&self, state: BlockState) -> Option<String>;
        fn min_y(&self) -> i32;
        fn height(&self) -> i32;
        fn block_in_tag(&self, block: BlockKey, tag: &str) -> Option<bool>;
        fn fluid_state(&self, pos: [i32; 3]) -> Option<FluidState>;
        fn state_fluid_state(&self, state: BlockState) -> Option<FluidState>;
        fn fluid_in_tag(&self, fluid: &str, tag: &str) -> Option<bool>;
        fn state_with_property(&self, state: BlockState, key: &str, value: &str) -> Option<BlockState>;
        fn brightness(&self, layer: LightLayer, pos: [i32; 3]) -> i32;
        fn raw_brightness(&self, pos: [i32; 3], sky_darken: i32) -> i32;
        fn is_loaded(&self, pos: [i32; 3]) -> bool;
        fn is_raining_at(&self, pos: [i32; 3]) -> bool;
        fn enabled_features(&self) -> FeatureFlags;
    }
    fn state_block(&self, state: BlockState) -> Option<&str> {
        self.game
            .pack
            .assets
            .registry
            .state(state.try_into_engine(&()).ok()?)
            .map(|state| &*state.name)
    }
    fn tags(&self) -> &dyn Tags {
        self.game
    }
    fn block_entity_data(&self, entity: BlockEntityRef) -> Option<Compound> {
        self.game.block_entity_data(entity)
    }
    fn signal(&self, pos: [i32; 3], direction: Direction) -> Option<i32> {
        self.game.level_signal(self.level, pos, direction)
    }
    fn direct_signal(&self, pos: [i32; 3], direction: Direction) -> Option<i32> {
        self.game.level_direct_signal(self.level, pos, direction)
    }
    fn direct_signal_to(&self, pos: [i32; 3]) -> Option<i32> {
        self.game.level_direct_signal_to(self.level, pos)
    }
    fn best_neighbor_signal(&self, pos: [i32; 3]) -> Option<i32> {
        self.game.level_best_neighbor_signal(self.level, pos)
    }
    fn has_neighbor_signal(&self, pos: [i32; 3]) -> Option<bool> {
        Some(self.game.level_has_neighbor_signal(self.level, pos))
    }
    fn collision_shape(&self, pos: [i32; 3]) -> Option<Vec<[f64; 6]>> {
        self.is_loaded(pos).then(|| {
            let [x, y, z] = pos;
            super::native_state_shapes::collision_boxes(
                self.game.collision,
                &self.game.world,
                self.game.world.block(x, y, z),
                pos,
            )
        })
    }
}

impl LevelRandomHost for JournalLevelWrite<'_, '_> {
    fn container_signal(&mut self, pos: [i32; 3]) -> Option<i32> {
        self.is_loaded(pos)
            .then(|| self.game.stored_container_signal((pos[0], pos[1], pos[2])))
    }
    fn attachments(&mut self) -> &mut dyn AttachmentStore {
        self.game
    }
    fn random(&mut self) -> Option<LegacyRandomSource> {
        Some(super::api_values::random_to_api(&self.game.world.random))
    }
    fn set_random(&mut self, random: LegacyRandomSource) -> bool {
        self.game
            .journal_set_level_random(super::api_values::random_to_engine(&random));
        true
    }
}

impl LevelTicksHost for JournalLevelWrite<'_, '_> {
    fn schedule_block_tick(&mut self, [x, y, z]: [i32; 3], block: BlockKey, delay: i32) -> bool {
        let pos = BlockPos::new(x, y, z);
        let Some(block) = super::api_values::behavior_block(block, self.game.res.behaviours())
        else {
            return false;
        };
        self.game.block_ticks.blocks.chunk(pos.chunk()).is_some()
            && self.queue(PendingWrite::BlockTick(pos, block, delay))
    }
    fn schedule_fluid_tick(&mut self, [x, y, z]: [i32; 3], fluid: &str, delay: i32) -> bool {
        let pos = BlockPos::new(x, y, z);
        let Some(fluid) = FluidKind::from_name(fluid) else {
            return false;
        };
        self.game.block_ticks.fluids.chunk(pos.chunk()).is_some()
            && self.queue(PendingWrite::FluidTick(pos, fluid, delay))
    }
}

impl LevelWriteHost for JournalLevelWrite<'_, '_> {
    fn set_block(&mut self, pos: [i32; 3], state: BlockState, flags: BlockUpdateFlags) -> bool {
        state
            .try_into_engine(&())
            .is_ok_and(|state| self.write_block(pos, state, flags.bits()))
    }
    fn remove_block(&mut self, pos: [i32; 3], moved: bool) -> bool {
        if !self.is_loaded(pos) {
            return false;
        }
        let [x, y, z] = pos;
        let at = BlockPos::new(x, y, z);
        let state = self.game.world.block(x, y, z);
        let replacement = self.game.with_level(|b, level, _| {
            level.adjust_remove_block(at, state, moved, b.legacy_block(b.fluid(state)))
        });
        let flags = flags::UPDATE_ALL
            | if moved {
                flags::UPDATE_MOVE_BY_PISTON
            } else {
                0
            };
        self.write_block(pos, replacement, flags)
    }
    fn set_block_property(&mut self, pos: [i32; 3], key: &str, value: &str, flags: i32) -> bool {
        let [x, y, z] = pos;
        super::native_hooks::with_property(
            self.game.pack.assets.registry,
            self.game.world.block(x, y, z),
            key,
            value,
        )
        .is_some_and(|state| self.write_block(pos, state, flags))
    }
    fn play_sound(
        &mut self,
        sound: &str,
        at: [f64; 3],
        source: usize,
        volume: f32,
        pitch: f32,
    ) -> bool {
        if self.game.lan.is_none()
            || !super::native_hooks::mod_sound_ok(sound, at, source, volume, pitch)
        {
            return false;
        }
        self.queue(PendingWrite::Sound(NativeSound {
            dimension: self.level.dimension_id(),
            sound: sound.to_owned(),
            at,
            source: source as i32,
            volume,
            pitch,
            seed: 0,
            except: None,
            entity: None,
        }))
    }
    fn send_particles(
        &mut self,
        options: &ParticleOptions,
        state: Option<BlockState>,
        at: [f64; 3],
        spread: [f64; 3],
        speed: f64,
        count: i32,
    ) -> bool {
        let (Ok(options), Ok(state)) = (
            options.try_into_engine(self.game.items()),
            state.try_into_engine(&()),
        ) else {
            return false;
        };
        let registry = self.game.pack.assets.registry;
        if !super::native_hooks::mod_particles_ok(
            &options,
            state,
            |id| registry.state(id).is_some(),
            at,
            spread,
            speed,
        ) {
            return false;
        }
        self.queue(PendingWrite::Particles(QueuedParticle {
            dimension: self.level.dimension_id(),
            options,
            state,
            at,
            spread,
            speed: [speed; 3],
            count,
            randomization: 0,
        }))
    }
    fn level_event(&mut self, id: i32, pos: [i32; 3], data: i32) -> bool {
        super::native_level_events::level_event_data_ok(
            id,
            data,
            self.game.res.facts().states.len(),
        ) && self.queue(PendingWrite::Event(id, pos, data))
    }
}

pub(super) enum PendingWrite {
    BlockTick(BlockPos, BlockId, i32),
    FluidTick(BlockPos, FluidKind, i32),
    Sound(NativeSound),
    Particles(QueuedParticle),
    Event(i32, [i32; 3], i32),
}

impl Game<'_> {
    pub(super) fn commit_level_write(&mut self, write: PendingWrite) {
        match write {
            PendingWrite::BlockTick(pos, block, delay) => {
                self.block_ticks.schedule_block(
                    self.world.game_time,
                    pos,
                    block,
                    delay,
                    TickPriority::Normal,
                );
            }
            PendingWrite::FluidTick(pos, fluid, delay) => {
                self.block_ticks
                    .schedule_fluid(self.world.game_time, pos, fluid, delay);
            }
            PendingWrite::Sound(mut sound) => {
                // The network's sound seed advances only for a published sound.
                if let Some(host) = &self.lan {
                    sound.seed = host.hub.next_sound_seed();
                    self.creatures.lan_sounds.push(sound);
                    self.flush_native_sounds();
                }
            }
            PendingWrite::Particles(particles) => {
                self.creatures.lan_particles.push(particles);
                self.flush_lan_particles();
            }
            PendingWrite::Event(id, pos, data) => self.level_event_at(id, pos, data),
        }
    }

    /// A part that refuses or falls through does not keep its tentative writes.
    pub(super) fn journalled_place_liquid(
        &mut self,
        level: LevelRef,
        write: &FluidWrite,
    ) -> Option<bool> {
        if !super::api_values::block_state::block_listens(mod_api::HookId::PlaceLiquid, write.from)
            || !self.native_level_write_allowed(level, [write.pos.x, write.pos.y, write.pos.z])
        {
            return None;
        }
        let state = write.from.try_into_api().ok()?;
        let fluid = super::native_world_read::api_fluid_state(write.source_fluid_state())?;
        let mut answer = None;
        mod_api::dispatch_place_liquid_with(write.pos, state, fluid, &mut answer, |part, event| {
            // For a player bucket this can be the root scope; the fluid
            // utility already has an outer transaction.
            let seal = self.transaction_open();
            let reply = JournalLevelWrite::new(self, level)
                .and_then(|mut host| part.place_liquid(&mut host, event));
            self.transaction_close(&seal, reply == Some(true));
            reply
        });
        answer
    }
}
