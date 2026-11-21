//! The `clone_item_stack` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// `BlockBehaviour.getCloneItemStack`, asked by
    /// `ServerGamePacketListenerImpl.handlePickItemFromBlock`. `include_data`
    /// is true for a creative player picking with Ctrl. Leave `stack` unset to
    /// keep the game's item. Creative breaking, such as a beehive's
    /// `playerWillDestroy`, does not ask this query. Server-side.
    #[hook(
        receiver = "read",
        dispatch = "flat",
        side = "server",
        contract = "replace",
        out = "stack",
        scope = "block",
        key = "state",
        host = "read",
        neoforge = "IBlockExtension.getCloneItemStack"
    )]
    fn clone_item_stack(
        dimension: LevelDimension,
        pos: BlockPos,
        block: &str,
        state: BlockState,
        include_data: bool,
        stack: &mut Option<ItemStack>,
    );
}
