use super::*;

impl GameLevel<'_> {
    /// A crop-like block at `pos` is about to grow on a random tick
    /// (NeoForge's `CropGrowEvent.Pre`): crops, stems, nether wart, cocoa,
    /// sweet berries and bamboo at their own position; cactus and chorus
    /// flowers at the block above; sugar cane at its own position; kelp,
    /// weeping and twisting vines and cave vines at the block they grow
    /// into. `state` is the growing block. Read-only `vanilla` is vanilla's own
    /// decision for this tick: its chance roll, or true for cactus, sugar cane
    /// and chorus flowers, which have none. `Default` yields `vanilla`; set
    /// `True` to grow now or `False` to stop it (`DO_NOT_GROW`). NeoForge does
    /// not expose this default decision on its event. Rolls stay before the
    /// hook in vanilla's order, including sweet berries and bamboo, not
    /// NeoForge's moved draw order. Server-side.
    #[inline(always)]
    pub(in crate::game) fn native_crop_grow_pre(
        &mut self,
        dimension: LevelDimension,
        pos: BlockPos,
        state: BlockStateId,
        vanilla: bool,
        result: &mut TriState,
    ) {
        self.fire_crop_grow_pre(dimension, pos, state, vanilla, result);
    }

    /// A crop-like block grew (NeoForge's `CropGrowEvent.Post`).
    /// Diverges from NeoForge: original_state is always the pre-growth state.
    /// `state` is what is at `pos` now. Sugar cane reports the new cane above
    /// it, and kelp and vines the new head, while `original_state` names the
    /// growing plant before growth. A full-age stem fires even without fruit;
    /// sugar cane fires only when adding a cane, not when only aging. Cactus
    /// and chorus report the plant's position. Server-side.
    #[inline(always)]
    pub(in crate::game) fn native_crop_grow_post(
        &mut self,
        dimension: LevelDimension,
        pos: BlockPos,
        original_state: BlockStateId,
        state: BlockStateId,
    ) {
        self.fire_crop_grow_post(dimension, pos, original_state, state);
    }
}
