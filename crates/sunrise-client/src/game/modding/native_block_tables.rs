//! The read-only tables behind `BlockStates`: the facts a block state carries, read from the
//! game's shared resources alone. `Game` and every level view hand out this one view, so a part
//! reads the same facts in a level callback as through a `Host`.
use super::{Game, PackResources, Resources, WorldRest, api_values::TryIntoEngine};
use mod_api::{
    BlockKey, BlockSoundType, BlockState, BlockStateRange, FluidDefinition, FluidKey, FluidState,
    FluidStateError, FluidTypeProperties, StateConstraint, StateMatchError, StateProperties,
    WeatheringAge,
};
use sunrise_client::items::ItemStack;
use sunrise_mod_package::content::FluidMeasures;
use sunrise_world::fluid::FluidKind;
use sunrise_world::level::{fluid_drip_particle, fluid_explosion_resistance, fluid_randomly_ticks};
use sunrise_world::registry::BlockStateId;

/// Borrowed views of the game's block registry, facts and behaviour tables.
#[derive(Clone, Copy)]
pub(in crate::game) struct BlockTables<'a> {
    res: &'a Resources,
    pack: &'a PackResources,
    /// Whether [`Game::native_tag_items`] would currently answer `Some`.
    ///
    /// `BlockTables` only holds `res` and `pack`, so the same availability
    /// condition `Game::block_in_tag` uses has to be captured when the view is
    /// built (`BlockTables::of`): on a client, the tags the server has sent;
    /// elsewhere, once the world lock is installed. The view is short-lived, so
    /// the flag stays current for this read.
    tag_tables: bool,
}

impl<'a> BlockTables<'a> {
    pub(in crate::game) fn new(
        res: &'a Resources,
        pack: &'a PackResources,
        tag_tables: bool,
    ) -> Self {
        Self {
            res,
            pack,
            tag_tables,
        }
    }

    pub(in crate::game) fn of(game: &'a Game<'_>) -> Self {
        Self::new(&game.res, &game.pack, game.native_tag_items().is_some())
    }

    /// The item table the generated forwarders convert item values with.
    pub(in crate::game) fn items(&self) -> &sunrise_client::items::Items {
        self.res.items()
    }

    /// The exported definition of a world fluid, or a tank-only identity.
    pub(in crate::game) fn fluid_state_definition(
        &self,
        fluid: FluidKey,
    ) -> Option<FluidDefinition<'a>> {
        let id = mod_api::__private::fluid_id(fluid)?;
        let Some(definition) = self.res.behaviours().fluid_definitions().get(id) else {
            return Some(FluidDefinition::TankOnly);
        };
        let default = super::native_world_read::api_fluid_state(definition.default)?;
        Some(FluidDefinition::World {
            default,
            properties: StateProperties::of(&definition.properties),
        })
    }

    /// A registered fluid's type properties: its block's light and name for a world fluid,
    /// vanilla's `FluidTags.WATER` and `FluidTags.LAVA` behaviours by its family. None for a
    /// key the registry doesn't hold.
    pub(in crate::game) fn fluid_type_properties(
        &self,
        fluid: FluidKey,
    ) -> Option<FluidTypeProperties> {
        let id = mod_api::__private::fluid_id(fluid)?;
        let behaviours = self.res.behaviours();
        let block = behaviours.fluid_definitions().get(id).map(|definition| {
            self.res
                .facts()
                .get(behaviours.legacy_block(definition.default))
        });
        let description_id = match &block {
            Some(block) => super::native_block_states::description_id("block", &block.name)?,
            None => super::native_block_states::description_id("fluid_type", id)?,
        };
        let kind = FluidKind::from_name(id);
        let water = kind.is_some_and(|kind| kind.same_family(FluidKind::Water));
        let lava = kind.is_some_and(|kind| kind.same_family(FluidKind::Lava));
        let measures = fluid_measures(id, kind);
        Some(FluidTypeProperties {
            description_id,
            light_level: block.map_or(0, |block| block.emission),
            fall_distance_modifier: match (water, lava) {
                (true, _) => 0.0,
                (_, true) => 0.5,
                _ => 1.0,
            },
            can_push_entity: water || lava,
            can_swim: water,
            can_drown: water,
            can_extinguish: water,
            can_hydrate: water,
            supports_boating: water,
            explosion_resistance: kind.map_or(0.0, fluid_explosion_resistance),
            randomly_ticks: kind.is_some_and(fluid_randomly_ticks),
            drip_particle: kind.and_then(fluid_drip_particle).map(str::to_owned),
            temperature: measures.temperature,
            density: measures.density,
            viscosity: measures.viscosity,
        })
    }

    /// `FluidState.createLegacyBlock` for `state`, through the one table the engine converts
    /// fluid states with. None for a state no fluid can be in: no world form, an amount above
    /// 8, a zero amount for anything but the empty fluid, or a source that isn't full.
    /// The host forwarder converts this engine ID through the checked block-state index.
    pub(in crate::game) fn fluid_legacy_block(&self, state: FluidState) -> Option<BlockStateId> {
        let engine = super::native_block_states::engine_fluid_state(state)?;
        Some(self.res.behaviours().legacy_block(engine))
    }

    /// Validate fluid constraints against the exported property definition.
    pub(in crate::game) fn fluid_state_validate(
        &self,
        fluid: FluidKey,
        constraints: &[StateConstraint],
    ) -> Result<(), FluidStateError> {
        let definition = mod_api::__private::fluid_id(fluid)
            .and_then(|id| self.res.behaviours().fluid_definitions().get(id))
            .ok_or(FluidStateError::NoWorldState)?;
        for (index, constraint) in constraints.iter().enumerate() {
            if constraints[..index]
                .iter()
                .any(|previous| previous.name == constraint.name)
            {
                return Err(FluidStateError::Duplicate(index as u32));
            }
            super::native_block_states::match_state_properties(
                &definition.properties,
                true,
                |_, _| true,
                std::slice::from_ref(constraint),
            )
            .map_err(|error| match error {
                StateMatchError::UnknownProperty(_) => {
                    FluidStateError::UnknownProperty(index as u32)
                }
                StateMatchError::InvalidValue(_) => FluidStateError::InvalidValue(index as u32),
                _ => unreachable!("StateMatchError: unhandled variant"),
            })?;
        }
        Ok(())
    }

    pub(in crate::game) fn block_state_piston_push_reaction(
        &self,
        state: BlockState,
    ) -> Result<Option<mod_api::PushReaction>, mod_api::LevelReadError> {
        use mod_api::{BlockStateError, LevelReadError};
        let rest = self.res.rest.get().ok_or(LevelReadError::Unavailable)?;
        let state = match state.try_into_engine(&()) {
            Ok(state) => state,
            Err(BlockStateError::UnknownBlock | BlockStateError::UnknownState) => return Ok(None),
            Err(_) => return Err(LevelReadError::Unavailable),
        };
        Ok(Some(super::api_values::api_push_reaction(
            rest.behaviours.piston_reaction(state),
        )))
    }

    /// Require published weathering tables before resolving the API state to its engine ID.
    fn weathering_state(&self, state: BlockState) -> Option<(&WorldRest, BlockStateId)> {
        Some((self.res.rest.get()?, state.try_into_engine(&()).ok()?))
    }

    pub(in crate::game) fn block_weathering_age(&self, state: BlockState) -> Option<WeatheringAge> {
        let (rest, state) = self.weathering_state(state)?;
        Some(match rest.behaviours.weathering_age(state)? {
            0 => WeatheringAge::Unaffected,
            1 => WeatheringAge::Exposed,
            2 => WeatheringAge::Weathered,
            3 => WeatheringAge::Oxidized,
            _ => return None,
        })
    }

    pub(in crate::game) fn block_next_weathering_stage(
        &self,
        state: BlockState,
    ) -> Option<BlockStateId> {
        let (rest, state) = self.weathering_state(state)?;
        rest.behaviours.weathering_next(state)
    }

    pub(in crate::game) fn block_previous_weathering_stage(
        &self,
        state: BlockState,
    ) -> Option<BlockStateId> {
        let (rest, state) = self.weathering_state(state)?;
        rest.behaviours.weathering_previous(state)
    }

    pub(in crate::game) fn block_state_waxed(&self, state: BlockState) -> Option<BlockStateId> {
        let (_, state) = self.weathering_state(state)?;
        self.honeycomb_waxed(state)
    }

    pub(in crate::game) fn block_state_unwaxed(&self, state: BlockState) -> Option<BlockStateId> {
        let (rest, state) = self.weathering_state(state)?;
        if let Some(unwaxed) = rest.behaviours.mapped_unwaxed(state) {
            return Some(unwaxed);
        }
        let registry = self.pack.assets.registry;
        registry
            .resolve(
                &rest
                    .transformers
                    .unwaxed(registry, registry.state(state)?)?,
            )
            .ok()
    }

    /// `BlockState.canBeReplaced()`: whether placing a block where this state stands
    /// replaces it (air, grass, water, snow layers). None for an unknown state.
    pub(in crate::game) fn block_state_replaceable(&self, state: BlockStateId) -> Option<bool> {
        self.pack.assets.registry.state(state)?;
        Some(self.res.facts().get(state).replaceable)
    }
    /// `BlockBehaviour.Properties.destroyTime`, as `BlockState.getDestroySpeed` reads it:
    /// stone 1.5, obsidian 50, bedrock -1 (it can't be broken). None for an unknown state.
    pub(in crate::game) fn block_state_destroy_speed(&self, state: BlockStateId) -> Option<f32> {
        self.pack.assets.registry.state(state)?;
        Some(self.res.facts().get(state).hardness)
    }
    /// `BlockState.requiresCorrectToolForDrops()`. None for an unknown state.
    pub(in crate::game) fn block_state_requires_correct_tool(
        &self,
        state: BlockStateId,
    ) -> Option<bool> {
        self.pack.assets.registry.state(state)?;
        Some(self.res.facts().get(state).requires_tool)
    }
    /// The explosion resistance the block of this state declares (`Block.getExplosionResistance`):
    /// stone 6, obsidian 1200, bedrock 3,600,000. None for an unknown state.
    pub(in crate::game) fn block_state_explosion_resistance(
        &self,
        state: BlockStateId,
    ) -> Option<f32> {
        self.pack.assets.registry.state(state)?;
        Some(self.res.facts().get(state).explosion_resistance)
    }
    /// `BlockState.getSoundType()`: its volume, pitch and the sound events for breaking,
    /// stepping on, placing, hitting and falling on it. None for an unknown state.
    pub(in crate::game) fn block_state_sound(&self, state: BlockStateId) -> Option<BlockSoundType> {
        self.pack.assets.registry.state(state)?;
        Some(sunrise_client::block_sounds::own(
            &self.res.facts().get(state).sounds,
        ))
    }
    /// `BlockState.hasBlockEntity()`. None for an unknown state.
    pub(in crate::game) fn block_state_has_block_entity(
        &self,
        state: BlockStateId,
    ) -> Option<bool> {
        self.pack.assets.registry.state(state)?;
        Some(self.res.facts().get(state).block_entity)
    }
    /// `BlockState.getMapColor`: the state's `MapColor.id`, 0 for none. None for an unknown state.
    pub(in crate::game) fn block_state_map_color(&self, state: BlockStateId) -> Option<u8> {
        self.pack.assets.registry.state(state)?;
        Some(self.res.facts().get(state).map_color)
    }
    /// Decode a packed pixel with the same palette and shade arithmetic as map rendering.
    pub(in crate::game) fn block_state_map_color_rgba(&self, packed: u8) -> Option<[u8; 4]> {
        Some(super::maps::packed_rgba(packed))
    }
    /// `BlockState.getLightEmission()`. None for an unknown state.
    pub(in crate::game) fn block_state_light_emission(&self, state: BlockStateId) -> Option<u8> {
        self.pack.assets.registry.state(state)?;
        Some(self.res.facts().get(state).emission)
    }
    /// `BlockBehaviour.getFriction()` of the state's block. None for an unknown state.
    pub(in crate::game) fn block_state_friction(&self, state: BlockStateId) -> Option<f32> {
        self.pack.assets.registry.state(state)?;
        Some(self.res.facts().get(state).friction)
    }
    /// `BlockState.isSolidRender()`. None for an unknown state.
    pub(in crate::game) fn block_state_is_solid_render(&self, state: BlockStateId) -> Option<bool> {
        self.pack.assets.registry.state(state)?;
        Some(self.res.facts().get(state).solid_render)
    }
    /// `#minecraft:fire` (`BlockTags.FIRE`). None for an unknown state, or when
    /// the tag tables are unavailable (`tag_tables`, from `Game::native_tag_items`).
    pub(in crate::game) fn block_state_is_fire(&self, state: BlockStateId) -> Option<bool> {
        let name = &self.pack.assets.registry.state(state)?.name;
        let key = mod_api::__private::block_key(name)?;
        if !self.tag_tables {
            return None;
        }
        super::native_world_read::block_in_tag(key, "minecraft:fire")
    }
    /// `BlockState.isRedstoneConductor` for a state alone, not a block in a level.
    pub(in crate::game) fn block_state_is_redstone_conductor(
        &self,
        state: BlockStateId,
    ) -> Option<bool> {
        self.res.behaviours().state_redstone_conductor(state)
    }
    /// `instanceof` for the vanilla classes of [`mod_api::BlockClass`], from the block's exported
    /// class chain: a class and its subclasses, an interface by what the block implements. None
    /// for an unknown state.
    pub(in crate::game) fn block_state_class(
        &self,
        state: BlockStateId,
        class: mod_api::BlockClass,
    ) -> Result<Option<bool>, mod_api::LevelReadError> {
        use mod_api::BlockClass;
        use sunrise_world::level::classes::{Class, Interface};
        if self.pack.assets.registry.state(state).is_none() {
            return Ok(None);
        }
        let data = self.res.behaviours().data(state);
        Ok(Some(match class {
            BlockClass::Stair => data.is(Class::Stair),
            BlockClass::Entity => data.implements(Interface::Entity),
            BlockClass::IronBars => data.is(Class::IronBars),
            BlockClass::StainedGlassPane => data.is(Class::StainedGlassPane),
            BlockClass::TrapDoor => data.is(Class::TrapDoor),
        }))
    }
    /// The immutable local ordinals of `block`, independent of current world ID order.
    pub(in crate::game) fn block_states_of(&self, block: BlockKey) -> Option<BlockStateRange> {
        mod_api::__private::block_state_range(block).ok()
    }

    /// The properties of a resolved block, borrowed from the command registry.
    pub(in crate::game) fn block_state_definition(
        &self,
        block: BlockKey,
    ) -> Option<StateProperties<'a>> {
        let default = super::api_values::default_state(block)?;
        let state = self.pack.assets.registry.state(default)?;
        let properties = self.pack.block_registries.blocks.get(&state.name)?;
        Some(StateProperties::of(properties))
    }

    /// Match against the existing command registry definitions without allocating on the call.
    pub(in crate::game) fn block_state_matches(
        &self,
        state: BlockStateId,
        block: BlockKey,
        constraints: &[StateConstraint],
    ) -> Option<Result<bool, StateMatchError>> {
        let default = super::api_values::default_state(block)?;
        let registry = self.pack.assets.registry;
        let target = registry.state(default)?;
        let state = registry.state(state)?;
        let definition = self.pack.block_registries.blocks.get(&target.name)?;
        Some(super::native_block_states::match_state_properties(
            definition,
            state.name == target.name,
            |name, value| {
                state
                    .properties
                    .get(name)
                    .is_some_and(|actual| actual == value)
            },
            constraints,
        ))
    }

    /// `ItemStack.isCorrectToolForDrops(state)`: the stack's own tool rules, with no block
    /// requirement, player, position or hook. None for an unknown state.
    pub(in crate::game) fn block_state_correct_tool_for_drops(
        &self,
        stack: &ItemStack,
        state: BlockStateId,
    ) -> Option<bool> {
        self.pack.assets.registry.state(state)?;
        Some(
            self.res
                .items()
                .correct_for_drops(Some(stack), &self.res.facts().get(state).name),
        )
    }

    pub(in crate::game) fn block_metadata_name(
        &self,
        block: mod_api::BlockKey,
    ) -> Result<&'static str, mod_api::BlockStateError> {
        mod_api::BlockMetadata::name(super::api_values::block_metadata_table()?, block)
    }
    pub(in crate::game) fn block_metadata_property(
        &self,
        state: BlockState,
        name: &str,
    ) -> Result<Option<&'static str>, mod_api::BlockStateError> {
        mod_api::BlockMetadata::property(super::api_values::block_metadata_table()?, state, name)
    }
    pub(in crate::game) fn block_metadata_with(
        &self,
        state: BlockState,
        name: &str,
        value: &str,
    ) -> Result<BlockState, mod_api::BlockStateError> {
        mod_api::BlockMetadata::with(
            super::api_values::block_metadata_table()?,
            state,
            name,
            value,
        )
    }
    pub(in crate::game) fn block_metadata_of(
        &self,
        block: mod_api::BlockKey,
        properties: &[mod_api::StateConstraint],
    ) -> Result<BlockState, mod_api::BlockStateError> {
        mod_api::BlockMetadata::of(
            super::api_values::block_metadata_table()?,
            block,
            properties,
        )
    }

    /// The waxed twin of `state`, its properties kept: the world's
    /// `neoforge:waxables` value first (`DataMapHooks.getBlockWaxed`), else
    /// vanilla's `WAXABLES`, read from the axe's wax-off transforms. None where
    /// the block doesn't wax.
    pub(in crate::game) fn honeycomb_waxed(&self, state: BlockStateId) -> Option<BlockStateId> {
        if let Some(waxed) = self.res.behaviours().mapped_waxed(state) {
            return Some(waxed);
        }
        let registry = self.pack.assets.registry;
        let waxed = self
            .res
            .transformers()
            .waxed(registry, registry.state(state)?)?;
        registry.resolve(&waxed).ok()
    }
}

/// `FluidType.getTemperature`, `getDensity` and `getViscosity` of the registered fluid `id`,
/// whose world kind is `kind`. NeoForgeMod's EMPTY_TYPE and LAVA_TYPE state their own; water
/// keeps `FluidType.Properties`' defaults, and a mod's fluid, flowing or not, what its source's
/// file states.
fn fluid_measures(id: &str, kind: Option<FluidKind>) -> FluidMeasures {
    match kind {
        Some(FluidKind::Empty) => FluidMeasures {
            temperature: 0,
            density: 0,
            viscosity: 0,
        },
        Some(kind) if kind.same_family(FluidKind::Lava) => FluidMeasures {
            temperature: 1300,
            density: 3000,
            viscosity: 6000,
        },
        Some(kind) if kind.same_family(FluidKind::Water) => FluidMeasures::default(),
        _ => {
            let source = kind
                .and_then(FluidKind::mod_fluid)
                .map_or(id, |fluid| fluid.get().id.as_str());
            sunrise_client::mod_content::fluid_measures(source).unwrap_or_default()
        }
    }
}

#[cfg(test)]
mod tests {
    use mod_api::{HostBlockStates, prelude::BlockStatesHost};

    #[test]
    #[ignore = "daily"]
    fn is_fire_reads_the_fire_tag_and_rejects_a_bad_state() {
        crate::game::native_entity_refs::tests::with_client_game(|game| {
            assert!(game.native_tag_items().is_none());
            let fire = BlockStatesHost::block_state_id(game, "minecraft:fire").unwrap();
            let soul = BlockStatesHost::block_state_id(game, "minecraft:soul_fire").unwrap();
            let stone = BlockStatesHost::block_state_id(game, "minecraft:stone").unwrap();
            let invalid = mod_api::__private::block_state_from_ordinal(fire.block(), u16::MAX);
            let facts = game.block_states();
            assert_eq!(facts.is_fire(fire), None);
            assert_eq!(facts.is_fire(soul), None);
            assert_eq!(facts.is_fire(stone), None);
            assert_eq!(facts.is_fire(invalid), None);
        });
        crate::game::native_entity_refs::tests::with_game(|game| {
            // with_game already ran resolve_parts (so BLOCK_TAGS is published)
            // but stops short of world open. Native tag reads use the session
            // lock to tell that the tables are ready (`finish_open_world`).
            let dir = std::env::temp_dir().join(format!(
                "sunrise-block-state-is-fire-{}",
                std::process::id()
            ));
            game.session_lock = Some(sunrise_client::save::SessionLock::acquire(&dir).unwrap());
            assert!(game.native_tag_items().is_some());
            let facts = game.block_states();
            let fire = BlockStatesHost::block_state_id(game, "minecraft:fire").unwrap();
            let soul = BlockStatesHost::block_state_id(game, "minecraft:soul_fire").unwrap();
            let stone = BlockStatesHost::block_state_id(game, "minecraft:stone").unwrap();
            assert_eq!(facts.is_fire(fire), Some(true));
            assert_eq!(facts.is_fire(soul), Some(true));
            assert_eq!(facts.is_fire(stone), Some(false));
            let invalid = mod_api::__private::block_state_from_ordinal(fire.block(), u16::MAX);
            assert_eq!(facts.is_fire(invalid), None);
            game.session_lock = None;
            let _ = std::fs::remove_dir_all(dir);
        });
    }
}
