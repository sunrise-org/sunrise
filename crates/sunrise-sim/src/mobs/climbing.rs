//! `LivingEntity.onClimbable` for mobs: the block a mob stands in, and the mods' say on whether it
//! is a ladder (`native_is_ladder`) and on whether the block under an open trapdoor makes the
//! trapdoor one (`native_makes_open_trapdoor_above_climbable`).
use crate::blocks::BlockFacts;
use crate::items::BlockTagFlags;
use crate::mobs::{Mob, MobWorld};
use crate::world::World;
use sunrise_core::position::BlockPos;
use sunrise_world::registry::BlockRegistry;

impl Mob {
    /// `LivingEntity.onClimbable` (`Spider.onClimbable`: the climbing flag):
    /// the block it stands in is `#minecraft:climbable` (vines, ladders,
    /// scaffolding...) or its mod parts' `is_ladder` says it is, or it is an open trapdoor over
    /// a ladder facing the same way, as the parts of the block below may decide
    /// (`trapdoorUsableAsLadder`). A mob gliding (`isFallFlying`) through a
    /// `#minecraft:can_glide_through` block (vines) is not on it: that test
    /// comes first.
    pub fn on_climbable(&self, w: &MobWorld) -> bool {
        self.on_climbable_in(
            w.world,
            w.registry,
            w.facts,
            w.block_tag_flags,
            w.loop_context,
        )
    }

    /// [`Self::on_climbable`] reading only the world and block tables so callers
    /// can refresh `on_climbable` in place without building a full [`MobWorld`].
    pub fn on_climbable_in(
        &self,
        world: &World,
        registry: &BlockRegistry,
        facts_table: &BlockFacts,
        block_tag_flags: &[u16],
        loop_context: &sunrise_world::native_load_context::LoopContext,
    ) -> bool {
        if self.kind.flags.climber {
            return self.climbing;
        }
        // `AbstractHorse.onClimbable`, `Ghast.onClimbable`, `Phantom.onClimbable`
        // and `HappyGhast.onClimbable` are a plain false: no horse, donkey,
        // mule, undead horse, llama, trader llama, camel or camel husk (nor
        // those flying monsters) is ever on a climbable block, whatever it
        // stands in.
        if crate::mobs::equine::is_equine(self.species())
            || matches!(
                self.species(),
                crate::mobs::Species::Ghast
                    | crate::mobs::Species::Phantom
                    | crate::mobs::Species::HappyGhast
            )
        {
            return false;
        }
        let (x, y, z) = (
            self.position.x.floor() as i32,
            self.position.y.floor() as i32,
            self.position.z.floor() as i32,
        );
        let state = world.block(x, y, z);
        let tag_bits = block_tag_flags.get(state.0 as usize).copied().unwrap_or(0);
        if self.extra.fall_flying && tag_bits & BlockTagFlags::CAN_GLIDE_THROUGH != 0 {
            return false;
        }
        // `IBlockExtension.isLadder`: the block's parts answer after the tag.
        let mut ladder = tag_bits & BlockTagFlags::CLIMBABLE != 0;
        crate::motion::native_is_ladder(
            loop_context,
            self.id,
            BlockPos::new(x, y, z),
            state,
            &mut ladder,
        );
        if ladder {
            return true;
        }
        if facts_table.is_air(state) {
            return false;
        }
        let facts = facts_table.get(state);
        let prop = |id, key| {
            registry
                .state(id)
                .and_then(|s| s.properties.get(key))
                .map(String::as_str)
        };
        if !facts.has_class("TrapDoorBlock") || prop(state, "open") != Some("true") {
            return false;
        }
        // `IBlockExtension.makesOpenTrapdoorAboveClimbable`: the parts of the block below answer
        // after vanilla's ladder test.
        let below = world.block(x, y - 1, z);
        let mut climbable = &*facts_table.get(below).name == "minecraft:ladder"
            && prop(below, "facing") == prop(state, "facing");
        crate::motion::ladder::native_makes_open_trapdoor_above_climbable(
            loop_context,
            below,
            BlockPos::new(x, y - 1, z),
            state,
            &mut climbable,
        );
        climbable
    }
}
