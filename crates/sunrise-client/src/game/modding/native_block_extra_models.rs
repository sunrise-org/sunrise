//! `ClientRenderClientHost::set_block_extra_models`: the standalone models a client mod draws as
//! part of a block (NeoForge's `ModelData` use), kept in the client's columns for the chunk mesher
//! (`World::set_extra_models`), which reads them on its workers and calls no mod code.
use super::Game;
use mod_api::{BlockExtraModelsError, ModelRef};

impl Game<'_> {
    /// `models` (resolved standalone models) replace those drawn as part of the block at `pos`,
    /// its section meshed again when they change. A stale or unknown handle refuses the list.
    pub(in crate::game) fn set_block_extra_models(
        &mut self,
        pos: [i32; 3],
        models: &[ModelRef],
    ) -> Result<(), BlockExtraModelsError> {
        for &model in models {
            self.pack
                .render
                .resolve_model(model)
                .map_err(BlockExtraModelsError::Model)?;
        }
        let [x, y, z] = pos;
        self.world.set_extra_models(x, y, z, models)
    }
}
