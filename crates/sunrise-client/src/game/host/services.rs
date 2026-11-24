//! The game's implementations of the mod API's service traits.
use crate::game::{Game, api_values::TryIntoEngine, native_console::Output};
use core::ops::ControlFlow;
use mod_api::{__private::game::CallbackOwner, *};
use std::sync::{Arc, mpsc::Sender};

impl ClientSession for Game<'_> {
    fn client_server(&self) -> Option<ClientServer<'_>> {
        CallbackOwner::call_host(|| self.native_client_server())
    }
    fn client_language(&self) -> Option<Arc<str>> {
        CallbackOwner::call_host(|| self.native_client_language())
    }
    fn client_latency(&self) -> Option<u32> {
        CallbackOwner::call_host(|| self.native_client_latency())
    }
    fn client_tab_list(&self) -> Result<Option<&dyn ClientTabList>, ClientSessionReadError> {
        Ok(CallbackOwner::call_host(|| {
            self.native_session().map(|_| self as &dyn ClientTabList)
        }))
    }
    fn client_frame_stats(&self) -> Option<ClientFrameStats> {
        CallbackOwner::call_host(|| self.native_client_frame_stats())
    }
}
impl Commands for Game<'_> {
    fn refresh_commands(&mut self) {
        CallbackOwner::call_host(|| self.native_refresh_commands())
    }
    fn feedback(
        &mut self,
        line: Text,
        broadcast_to_admins: bool,
    ) -> Result<(), mod_api::text::TextFormError> {
        let line = sunrise_client_lib::api_text::to_serialized_engine(&line)?;
        CallbackOwner::call_host(|| self.native_feedback(line, broadcast_to_admins));
        Ok(())
    }
    fn failure(&mut self, line: Text) -> Result<(), mod_api::text::TextFormError> {
        self.native_failure(sunrise_client_lib::api_text::to_serialized_engine(&line)?);
        Ok(())
    }
    fn run_command(&mut self, line: &str) -> bool {
        CallbackOwner::call_host(|| self.native_run_command(line))
    }
    fn run_command_with_output(&mut self, line: &str) -> mod_api::CommandResult {
        CallbackOwner::call_host(|| self.native_run_command_with_output(line))
    }
    fn run_command_with_feedback(
        &mut self,
        line: &str,
        output: Sender<Result<CommandFeedback, TextFormError>>,
    ) -> bool {
        CallbackOwner::call_host(|| {
            self.run_console_command(line, Some(Output::Stream(output)))
                .success
        })
    }
}
impl Levels for Game<'_> {
    fn visit_levels(
        &self,
        visit: &mut dyn FnMut(LevelRef) -> ControlFlow<()>,
    ) -> Result<ControlFlow<()>, ServerReadError> {
        // The caller's visitor runs inside its own callback scope.
        Ok(self.native_visit_levels(visit))
    }
}
impl Messages for Game<'_> {
    fn broadcast(&self, line: Text) -> Result<(), mod_api::text::TextFormError> {
        let line = sunrise_client_lib::api_text::to_serialized_engine(&line)?;
        CallbackOwner::call_host(|| self.native_broadcast(line));
        Ok(())
    }
}
impl Players for Game<'_> {
    fn server_players(&self) -> Vec<PlayerRef> {
        CallbackOwner::call_host(|| self.native_server_players())
    }
    fn player_by_name(&self, name: &str) -> Option<PlayerRef> {
        CallbackOwner::call_host(|| self.native_player_by_name(name))
    }
    fn acting_player(&self) -> Option<PlayerRef> {
        CallbackOwner::call_host(|| self.native_acting_player())
    }
    fn singleplayer_profile(&self) -> Option<GameProfile<'_>> {
        self.native_singleplayer_profile()
    }
}
impl Profiles for Game<'_> {
    fn resolve_name(&self, name: &str) -> ProfileResolution {
        self.native_resolve_profile(sunrise_client::skins::NameOrId::Name(name.to_owned()))
    }
    fn resolve_id(&self, uuid: u128) -> ProfileResolution {
        self.native_resolve_profile(sunrise_client::skins::NameOrId::Id(uuid))
    }
}
impl RuntimeLevels for Game<'_> {
    fn open(&mut self, id: &str, level: RuntimeLevel) -> Result<LevelRef, RuntimeLevelError> {
        CallbackOwner::call_host(|| self.runtime_level_open(id, level))
    }
    fn unload(&mut self, id: &str) -> Result<(), RuntimeLevelError> {
        CallbackOwner::call_host(|| self.runtime_level_unload(id))
    }
    fn delete(&mut self, id: &str) -> Result<(), RuntimeLevelError> {
        CallbackOwner::call_host(|| self.runtime_level_delete(id))
    }
    fn is_open(&mut self, id: &str) -> bool {
        CallbackOwner::call_host(|| self.runtime_level_is_open(id))
    }
}
impl RenderAssets for Game<'_> {
    fn model(&self, id: &Identifier) -> Result<ModelRef, ModelError> {
        CallbackOwner::call_host(|| self.native_model(id))
    }
    fn block_state_models(&self, state: BlockState) -> Result<BakedModelKeys<'_>, ModelError> {
        let state = state
            .try_into_engine(&())
            .map_err(|_| ModelError::UnknownModel)?;
        Ok(CallbackOwner::call_host(|| {
            self.pack.render.block_state_models(state)
        }))
    }
    fn block_state_model(&self, state: BlockState) -> Result<ModelRef, ModelError> {
        let state = state
            .try_into_engine(&())
            .map_err(|_| ModelError::UnknownModel)?;
        CallbackOwner::call_host(|| self.native_block_state_model(state))
    }
    fn baked_model(&self, key: BakedModelKey) -> Result<BakedModelView<'_>, ModelError> {
        CallbackOwner::call_host(|| self.pack.render.baked_model(key))
    }
    fn render_pipeline(&self, id: &Identifier) -> Result<RenderPipelineRef, RenderPipelineError> {
        CallbackOwner::call_host(|| self.pack.render.render_pipeline(id))
    }
    fn animation(&self, id: &Identifier) -> Option<AnimationHolder> {
        CallbackOwner::call_host(|| self.pack.render.animation(id))
    }
}
