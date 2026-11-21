//! Loading a mod block entity's saved payload on the server.

pub trait Hooks {
    /// The engine applied saved data to this mod block entity on the logical
    /// server (`BlockEntity.loadAdditional`). `tag` is the applied mod payload;
    /// the read-only level host sees the new data. `event.pos` and `event.level` are the
    /// instance's own `worldPosition` and level, so a listener can store the applied data under its
    /// dimension and position, also after a later `/data merge`. The notice cannot change the
    /// tag or write to the world.
    ///
    /// This runs for chunk loads, block data commands, cloning, structure and
    /// block-item placement with data, and falling blocks that carry data.
    /// For a newly attached entity it precedes `block_entity_on_load`, which
    /// fires once per instance even for fresh placement without saved data.
    /// Direct Mod API storage edits (`merge_data`, `replace_data`) and
    /// transaction restores do not emit this notice. Client network updates
    /// use `block_entity_data_received` instead.
    #[hook(
        side = "server",
        contract = "listen",
        event_per_listener = true,
        scope = "block_entity",
        key = "id",
        target = "sunrise_client::game::native_hooks::block_entity_load_additional::GameLevel<'_>::native_block_entity_load_additional",
        neoforge = "none (vanilla BlockEntity.loadAdditional; NeoForge adds no event for this moment)",
        version = 2
    )]
    fn block_entity_load_additional();
}
