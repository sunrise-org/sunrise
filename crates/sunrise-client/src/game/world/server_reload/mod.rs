//! Prepare server listeners against the new packs before publishing any data.
use mod_api::__private::game::*;

#[cfg(test)]
#[path = "server_reload_tests.rs"]
mod tests;
use super::packs::reload_files;
use super::{Game, functions::PackStack};
use mod_api::{
    __private::reload::{ReloadStageError, apply_listeners, prepare_listeners},
    FileToIdConverter, Identifier, ServerReloadContext, ServerReloadListener,
    ServerReloadResources, VanillaServerListeners,
};
use sunrise_client::datapack::DataStore;

pub(super) type PreparedReload = PreparedStep<Box<DataStore>>;

use mod_api::__private::reload::PreparedStep;

/// Everything one successful reload publishes: its apply steps, then the
/// retained listeners' store.
pub(super) struct PreparedServerReload {
    pub(super) steps: Vec<PreparedReload>,
    pub(super) resources: mod_api::RetainedListeners,
    pub(super) loot: std::sync::Arc<mod_api::GlobalLootModifierManager>,
}
impl PreparedServerReload {
    pub(super) fn bind_loot(&self, world: &super::PreparedWorldReload) {
        if let Some(tables) = &world.tables {
            self.loot.bind_from(&tables.loot_modifier_manager);
        }
    }
}

/// Loads the vanilla function library from `stack`, logging per-file failures.
fn functions_store(stack: &PackStack) -> DataStore {
    let packs: Vec<_> = stack.builtin.iter().chain(stack.enabled_packs()).collect();
    let store = DataStore::load(&packs);
    for (id, e) in &store.errors {
        log::warn!("Sunrise: failed to load function {id}: {e}");
    }
    store
}

impl PackStack {
    fn reload_context(
        &self,
        listener: &Identifier,
        converter: &FileToIdConverter,
    ) -> ServerReloadContext {
        let packs: Vec<_> = self.builtin.iter().chain(self.enabled_packs()).collect();
        ServerReloadContext::new(reload_files("data", &packs, listener, converter))
    }
}

impl Game<'_> {
    pub(super) fn prepare_server_reload(
        &self,
        stack: &PackStack,
    ) -> Result<PreparedServerReload, ReloadStageError> {
        let _reload = self
            .mod_phase
            .reloading()
            .expect("server reload on its owning thread");
        sunrise_client::mod_content::check_jars(
            &self.options.resources,
            &sunrise_client::logging::game_home().join("jars"),
        )
        .map_err(|error| {
            ReloadStageError::OriginalJars(
                crate::game::api_values::TryIntoApi::try_into_api(&error).unwrap(),
            )
        })?;
        let mut listeners = <mod_api::ServerListenerRegistry as mod_api::__private::game::ServerReloadListenersOrder>::new();
        let loot = super::global_loot_modifiers::register_anchor(&mut listeners)
            .map_err(ReloadStageError::Order)?;
        self.native_add_server_reload_listeners(&mut listeners);
        let mut resources = mod_api::RetainedListeners::default();
        let ordered = listeners.finish().map_err(ReloadStageError::Order)?;
        resources.install(ordered.retained);
        let steps = prepare_listeners(
            ordered.listeners,
            "server_reload_prepare",
            |listener| {
                (listener.id == VanillaServerListeners::functions())
                    .then(|| Box::new(functions_store(stack)))
            },
            |id, converter| stack.reload_context(id, converter),
            ServerReloadListener::prepare,
        )?;
        Ok(PreparedServerReload {
            steps,
            resources,
            loot,
        })
    }
    pub(super) fn apply_server_reload_callbacks(
        &mut self,
        prepared: PreparedServerReload,
    ) -> PreparedServerReload {
        let phase = self.mod_phase.clone();
        let _reload = phase
            .reloading()
            .expect("server apply on its owning thread");
        let mut vanilla = Vec::new();
        apply_listeners(prepared.steps, "server_reload_apply", |store| {
            vanilla.push(PreparedStep::Vanilla(store))
        });
        PreparedServerReload {
            steps: vanilla,
            resources: prepared.resources,
            loot: prepared.loot,
        }
    }
    pub(super) fn apply_server_reload(
        &mut self,
        prepared: PreparedServerReload,
        stack: &PackStack,
    ) {
        let phase = self.mod_phase.clone();
        let _reload = phase
            .reloading()
            .expect("server apply on its owning thread");
        apply_listeners(prepared.steps, "server_reload_apply", |store| {
            self.admit_store(*store, stack);
        });
        if let Some(tables) = &self.world_tables {
            prepared.loot.bind_from(&tables.loot_modifier_manager);
        }
        self.reload_resources = prepared.resources;
        self.structure_templates.clear();
    }
    /// The retained server reload listeners of the last successful reload.
    pub(super) fn reload_resources(&self) -> &dyn ServerReloadResources {
        &self.reload_resources
    }
}
