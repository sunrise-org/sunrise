//! What the block an entity stands on or in gives it, as the block's parts answer: its friction
//! (`IBlockExtension.getFriction`), its jump and speed factors, and whether it is a ladder.
#[cfg(any(test, feature = "test-dry-side"))]
pub use crate::block_contact::dry_side;
pub use crate::block_contact::{FrictionLevel, SideLevel};
use crate::{blocks::BlockFacts, world::World};
use sunrise_core::position::BlockPos;
use sunrise_world::{
    movement_properties::BlockFactorEdits,
    native_load_context::LoopContext,
    registry::{BlockRegistry, BlockStateId},
};

/// `IBlockExtension.getFriction` of a state at a position. A cyclic or over-deep mod
/// query gets no answer. A non-finite answer keeps the state's own coefficient; any finite one is used as given.
#[inline(never)]
pub fn read_friction(
    level: FrictionLevel<'_>,
    entity: Option<(u32, &str)>,
    pos: BlockPos,
    state: BlockStateId,
    own: f32,
) -> Option<f32> {
    let mut friction = own;
    if !crate::native_client_hooks::friction(level, entity, pos, state, &mut friction) {
        return None;
    }
    Some(if friction.is_finite() { friction } else { own })
}

/// The friction of the block at `pos` for the entity on it, starting at its `StateFacts` value.
pub fn block_friction(
    context: &LoopContext,
    side: &dyn SideLevel,
    entity: (u32, &str),
    world: &World,
    registry: &BlockRegistry,
    facts: &BlockFacts,
    pos: BlockPos,
) -> f32 {
    let state = world.block(pos.x, pos.y, pos.z);
    let own = facts.get(state).friction;
    let level = FrictionLevel {
        world,
        registry,
        context,
        side,
        coefficient: &|id| facts.states.get(id.0 as usize).map(|facts| facts.friction),
    };
    read_friction(level, Some(entity), pos, state, own).unwrap_or(own)
}

/// A block state's jump factor (`Block.getJumpFactor`) is being read (beyond NeoForge: no event).
/// Its parts may change it. Fires on replayed ticks too, so a listener must be a pure function of
/// its inputs.
#[inline(never)]
pub fn native_block_jump_factor(context: &LoopContext, state: BlockStateId, factor: &mut f32) {
    crate::native_client_hooks::block_jump_factor(context, state, factor);
}

/// A block state's speed factor (`Block.getSpeedFactor`) is being read (beyond NeoForge: no
/// event). Its parts may change it. Fires on replayed ticks too, so a listener must be a pure
/// function of its inputs.
#[inline(never)]
pub fn native_block_speed_factor(context: &LoopContext, state: BlockStateId, factor: &mut f32) {
    crate::native_client_hooks::block_speed_factor(context, state, factor);
}

/// Whether a block is a ladder to a living entity (`IBlockExtension.isLadder`, which
/// `LivingEntity.onClimbable` asks of the block the entity stands in) is being decided. `ladder`
/// starts with the `#minecraft:climbable` answer and the block's parts may change it. Fires on
/// replayed ticks too, so a listener must be a pure function of its inputs.
#[inline(never)]
pub fn native_is_ladder(
    context: &LoopContext,
    entity: u32,
    pos: BlockPos,
    state: BlockStateId,
    ladder: &mut bool,
) {
    crate::native_client_hooks::is_ladder(context, entity, pos, state, ladder);
}

/// Whether a block stops a sneaking player's slide down a climbable as scaffolding does
/// (`IBlockExtension.isScaffolding`, which `LivingEntity.handleOnClimbable` asks of the block the
/// player stands in) is being decided. `scaffolding` starts with the `minecraft:scaffolding`
/// answer and the block's parts may change it. Fires on replayed ticks too, so a listener must be
/// a pure function of its inputs.
#[inline(never)]
pub fn native_is_scaffolding(
    context: &LoopContext,
    entity: u32,
    pos: BlockPos,
    state: BlockStateId,
    scaffolding: &mut bool,
) {
    let hook = mod_api::HookId::IsScaffolding;
    let scoped = mod_api::__private::scoped_state_listens(hook, state.0);
    if !scoped.unwrap_or_else(|| mod_api::__private::has_scoped_hook(hook)) {
        return;
    }
    let Some(state) = crate::block_contact::state_to_api(state) else {
        return;
    };
    if scoped.is_none() && !mod_api::__private::block_key_listens(hook, state.block()) {
        return;
    }
    let reader = sunrise_level::api_context::LoopReader(context);
    mod_api::dispatch_is_scaffolding(&reader.context(), entity, pos, state, scaffolding);
}

/// The factors the `block_jump_factor` and `block_speed_factor` parts give the blocks an entity
/// reads, for the loop of the side that ticks it.
pub(crate) struct ModBlockFactors<'a>(pub &'a LoopContext);
impl BlockFactorEdits for ModBlockFactors<'_> {
    fn jump(&self, state: BlockStateId, mut factor: f32) -> f32 {
        native_block_jump_factor(self.0, state, &mut factor);
        factor
    }
    fn speed(&self, state: BlockStateId, mut factor: f32) -> f32 {
        native_block_speed_factor(self.0, state, &mut factor);
        factor
    }
}
