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

#[cfg(test)]
mod tests {
    use mod_api::__private::{ModPhase, PhaseOwner};
    use mod_api::BlockExtraModelsError;
    use mod_api::prelude::ClientRenderClientHost;

    /// Through the host trait a native mod calls: outside the World phase (a resource reload's
    /// `client_resources_reloaded`, setup, config loading) the setter answers `Unavailable`
    /// instead of panicking; in the World phase it reaches the client's columns.
    #[test]
    #[ignore = "daily"]
    fn outside_the_world_phase_the_setter_is_unavailable() {
        super::super::native_entity_refs::tests::with_client_game(|game| {
            for phase in [ModPhase::Reloading, ModPhase::Setup, ModPhase::Config] {
                let _phase = PhaseOwner::scope(phase);
                assert_eq!(
                    ClientRenderClientHost::set_block_extra_models(game, [0, 64, 0], &[]),
                    Err(BlockExtraModelsError::Unavailable),
                    "{phase:?}"
                );
            }
            let _phase = PhaseOwner::scope(ModPhase::World);
            assert_eq!(
                ClientRenderClientHost::set_block_extra_models(game, [0, 9999, 0], &[]),
                Err(BlockExtraModelsError::NotLoaded),
                "the World phase reaches the columns"
            );
        });
    }
}
