//! Initial-load publication boundary shared by startup and its resource fixture.
use super::Game;
impl Game<'_> {
    pub(in crate::game) fn publish_initial_client_models(&mut self) -> sunrise_client::Result<()> {
        if self.options.is_client_start() {
            // The renderer table, its layers and the mods' setup read the world-rest tables (the
            // mob types), which a client start loads on a background worker: wait for what is
            // left of it here rather than race it.
            self.join_world_rest()?;
            self.resolve_entity_renderers();
            self.resolve_client_item_extensions()?;
            self.texture_atlases_stitched();
            self.native_model_baking_completed();
            self.native_client_setup();
        }
        Ok(())
    }
}
