//! Load-scoped registration and per-candidate condition lookup.
use super::Game;
use mod_api::__private::game::{
    CallbackOwner, DatapackCandidateBuilder, RegistrationContextNew, datapack_entry_id,
    datapack_registry_directory, decode_datapack_entry, valid_namespaced,
};
use mod_api::__private::{NoVanillaValueDecoder, VanillaValueDecoder};
use mod_api::{
    DatapackCandidate, DatapackDecodeError, DatapackLoadReason, DatapackRegistryDeclaration,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
    sync::Arc,
};
use sunrise_client::datapack::RegistryFiles;
use sunrise_world::{
    conditions::{ConditionProgress, LoadConditions},
    loot::PackOverlay,
};

pub(super) type ConditionTable =
    Arc<BTreeMap<mod_api::Identifier, (CallbackOwner, Arc<dyn mod_api::LoadCondition>)>>;
impl Game<'_> {
    pub(super) fn begin_condition_reload(
        &mut self,
        stack: &super::functions::PackStack,
        resources: &std::path::Path,
    ) -> Result<
        super::server_reload::PreparedServerReload,
        mod_api::__private::reload::ReloadStageError,
    > {
        use mod_api::__private::reload::ReloadStageError;
        if self.mods.load_condition_candidate.is_none() {
            let inventory = sunrise_mod_package::enabled::EnabledMods::read(resources)
                .map_err(|error| ReloadStageError::DataSource(error.to_string()))?;
            self.mods.load_condition_candidate = Some(
                conditions_for_mods(&self.mods, &inventory)
                    .map_err(ReloadStageError::DataSource)?,
            );
        }
        self.prepare_server_reload(stack)
    }
    pub(super) fn prepare_pack_condition_reload(
        &mut self,
        stack: &super::functions::PackStack,
    ) -> Result<
        super::server_reload::PreparedServerReload,
        mod_api::__private::reload::ReloadStageError,
    > {
        self.mods.load_condition_candidate = None;
        self.begin_condition_reload(stack, &self.options.resources.clone())
    }

    pub(super) fn stage_condition_inventory(
        &mut self,
    ) -> sunrise_client::Result<Option<sunrise_assets::resource_source::ResourceRoot>> {
        self.protect_condition_root();
        self.mods.load_condition_candidate = None;
        let inventory = sunrise_mod_package::enabled::EnabledMods::read(&self.options.resources)?;
        self.mods.load_condition_candidate = Some(conditions_for_mods(&self.mods, &inventory)?);
        self.stage_enabled_inventory()
    }

    pub(super) fn protect_condition_root(&mut self) {
        self.mods.active_condition_root = Some(sunrise_client::mod_content::retain_root(
            &self.options.resources,
        ));
    }

    pub(super) fn condition_root_needs_reload(&self) -> bool {
        self.mods
            .load_condition_candidate
            .as_ref()
            .is_some_and(|view| view.has_registered())
            && (!self.mods.selected.is_empty()
                || !self.mods.dev.is_empty()
                || !self.mods.native.packages().is_empty())
    }

    /// Stage the enabled packs’ actual winners before readers use their holders.
    pub(super) fn stage_datapack_registries(
        &self,
        stack: &super::functions::PackStack,
        root: &Path,
        overlay: &PackOverlay<'_>,
        view: &mut RegistryView,
        inputs: &VanillaValueInputs<'_, '_>,
    ) -> Result<
        Option<(
            DatapackCandidate,
            Option<super::native_datapack_values::DatapackVanillaValues>,
        )>,
        String,
    > {
        let declarations = mod_api::__private::datapack_registry_declarations();
        if declarations.is_empty() {
            return Ok(None);
        }
        let keys: Vec<_> = declarations
            .iter()
            .map(|declaration| declaration.key.as_str())
            .collect();
        let packs: Vec<_> = std::iter::once(&stack.content)
            .chain(stack.enabled_packs())
            .collect();
        let files = sunrise_client::datapack::registry_files(&packs, &keys)
            .map_err(|error| format_datapack_error(&error))?;
        if !mod_api::__private::declarations_use_vanilla_values(declarations) {
            return admit_datapack_registries(
                root,
                overlay,
                view,
                declarations,
                &files,
                &mut NoVanillaValueDecoder,
            )
            .map(|candidate| Some((candidate, None)));
        }
        let biome_tags = sunrise_client::items::load_tags(root, "worldgen/biome", inputs.tag_files)
            .map_err(|error| format!("datapack vanilla value biome tags: {error}"))?;
        let live_features = self.res.behaviours().live_features();
        let catalog = live_features
            .as_ref()
            .and_then(|features| features.state_provider_catalog());
        let biome_names = inputs.biomes.registry_order();
        let mut decoder = super::native_datapack_values::VanillaValueTableBuilder::new(
            self.pack.assets.registry,
            self.res.behaviours(),
            &inputs.items.block_tags,
            &inputs.items.fluid_tags,
            &biome_names,
            &biome_tags,
            catalog,
        )
        .map_err(|error| format!("datapack vanilla value decoder: {error}"))?;
        let candidate =
            admit_datapack_registries(root, overlay, view, declarations, &files, &mut decoder)?;
        Ok(Some((candidate, Some(decoder.finish()))))
    }
}

/// The world's loaded tables a vanilla value decoder reads while datapack registries are staged.
pub(super) struct VanillaValueInputs<'a, 'b> {
    pub(super) items: &'a sunrise_client::items::Items,
    pub(super) tag_files: &'a PackOverlay<'b>,
    pub(super) biomes: &'a sunrise_client::biome::Biomes,
}

#[derive(Default)]
pub(super) struct RegistryView {
    pub(super) ids: BTreeMap<String, BTreeSet<String>>,
    pub(super) tags: BTreeMap<String, BTreeSet<String>>,
}

impl RegistryView {
    pub(super) fn conditions(&self, base: &LoadConditions) -> LoadConditions {
        base.clone()
            .with_registries(
                self.ids
                    .iter()
                    .map(|(key, ids)| (key.as_str(), ids.iter().map(String::as_str))),
            )
            .with_tags(
                self.tags
                    .iter()
                    .map(|(key, tags)| (key.as_str(), tags.iter().map(String::as_str))),
            )
    }
}

fn format_datapack_error(error: &mod_api::DatapackLoadError) -> String {
    format!(
        "datapack registry {} entry {} (pack {}, file {}): {:?}",
        error.registry, error.entry, error.pack, error.file, error.reason
    )
}

fn admit_datapack_registries(
    root: &Path,
    overlay: &PackOverlay<'_>,
    view: &mut RegistryView,
    declarations: &[DatapackRegistryDeclaration],
    files: &RegistryFiles,
    decoder: &mut dyn VanillaValueDecoder,
) -> Result<DatapackCandidate, String> {
    use DatapackLoadReason::{Decode, InvalidEntryId, NotUtf8};
    let base = overlay.conditions;
    let malformed = |message| Decode(DatapackDecodeError::Malformed(message));
    let winners: Vec<_> = files
        .iter()
        .map(|(file, (pack, bytes))| {
            let json = std::str::from_utf8(bytes)
                .map_err(|_| NotUtf8)
                .and_then(|text| {
                    serde_json::from_str::<serde_json::Value>(text)
                        .map(|value| (text, value))
                        .map_err(|error| malformed(error.to_string()))
                });
            (file.as_str(), pack.as_str(), json)
        })
        .collect();
    let programs: Vec<_> = winners
        .iter()
        .map(|(_, _, json)| {
            let (text, json) = json.as_ref().map_err(Clone::clone)?;
            base.decode_file_conditions(json)
                .map(|program| (*text, program))
                .map_err(malformed)
        })
        .collect();
    let directories = declarations
        .iter()
        .map(|declaration| {
            datapack_registry_directory(&declaration.key)
                .ok_or_else(|| format!("invalid datapack registry key {}", declaration.key))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let producers: BTreeMap<_, _> = declarations
        .iter()
        .enumerate()
        .map(|(index, declaration)| {
            view.ids.remove(&declaration.key);
            view.tags.remove(&declaration.key);
            (declaration.key.as_str(), index)
        })
        .collect();
    let mut pending: Vec<_> = declarations.iter().map(|_| Vec::new()).collect();
    let mut errors = Vec::new();
    for (index, (declaration, directory)) in declarations.iter().zip(&directories).enumerate() {
        for ((file, pack, _), program) in winners.iter().zip(&programs) {
            let Some(entry) = datapack_entry_id(directory, file) else {
                continue;
            };
            let program = if valid_namespaced(&entry) {
                program.as_ref()
            } else {
                Err(&InvalidEntryId)
            };
            match program {
                Ok((text, program)) => {
                    pending[index].push((entry, *file, *pack, *text, program.evaluation()));
                }
                Err(reason) => errors.push(format!(
                    "datapack registry {} entry {entry} (pack {pack}, file {file}): {reason:?}",
                    declaration.key,
                )),
            }
        }
    }

    let mut builder = DatapackCandidateBuilder::default();
    let mut unbound = vec![true; declarations.len()];
    let mut groups: Vec<_> = (0..declarations.len()).collect();
    let mut cyclic = vec![false; declarations.len()];
    let mut entries = vec![BTreeMap::new(); declarations.len()];
    while unbound.contains(&true) {
        let context = view.conditions(base);
        let mut dependencies = vec![BTreeSet::new(); declarations.len()];
        for (index, declaration) in declarations.iter().enumerate() {
            let group = groups[index];
            pending[index].retain_mut(|(entry, file, pack, text, evaluation)| {
                let decoded = match evaluation.advance(&context, |registry| {
                    producers.get(registry).is_some_and(|&producer| {
                        unbound[producer] && (groups[producer] != group || !cyclic[group])
                    })
                }) {
                    Ok(ConditionProgress::NeedsRegistry(registry)) => {
                        dependencies[group].insert(groups[producers[registry]]);
                        return true;
                    }
                    Ok(ConditionProgress::Complete(false)) => return false,
                    Ok(ConditionProgress::Complete(true)) => {
                        decode_datapack_entry(declaration, text, &mut *decoder).map_err(Decode)
                    }
                    Err(error) => Err(malformed(error)),
                };
                match decoded {
                    Ok(value) => {
                        entries[index].insert(std::mem::take(entry), (value, (*pack).to_owned()));
                    }
                    Err(reason) => errors.push(format!(
                        "datapack registry {} entry {entry} (pack {pack}, file {file}): {reason:?}",
                        declaration.key,
                    )),
                }
                false
            });
        }
        let mut bound_any = false;
        for (index, declaration) in declarations.iter().enumerate() {
            if !unbound[index] || !dependencies[groups[index]].is_empty() {
                continue;
            }
            let entries = std::mem::take(&mut entries[index]);
            let tags = super::native_registry_entries::load_registry_tags(
                root,
                overlay,
                &directories[index],
                |id| entries.contains_key(id),
            )
            .unwrap_or_else(|error| {
                errors.push(format!(
                    "datapack registry {} tags: {error}",
                    declaration.key
                ));
                BTreeMap::new()
            });
            view.ids
                .insert(declaration.key.clone(), entries.keys().cloned().collect());
            view.tags.insert(
                declaration.key.clone(),
                tags.iter()
                    .filter(|(_, members)| !members.is_empty())
                    .map(|(tag, _)| tag.clone())
                    .collect(),
            );
            builder.stage_registry(&declaration.key, entries, tags);
            unbound[index] = false;
            bound_any = true;
        }
        if !bound_any {
            // All pending groups are blocked. A sink component can test its internal
            // reads as unbound; its entries and tags then become visible together, even
            // if resuming a program reaches another dependency before it can finish.
            let component = registry_components(&dependencies)
                .into_iter()
                .rev()
                .find(|members| !dependencies[members[0]].is_empty())
                .expect("nonempty graph of blocked registries");
            let group = component[0];
            cyclic[group] = true;
            for current in &mut groups {
                if component.contains(current) {
                    *current = group;
                }
            }
        }
    }
    if !errors.is_empty() {
        return Err(errors.join("\n"));
    }
    builder
        .finish(declarations)
        .map_err(|error| format_datapack_error(&error))
}

/// Strongly connected components, consumers before their dependencies
/// (Kosaraju's two traversals). Pack input controls the depth, so use explicit stacks.
fn registry_components(dependencies: &[BTreeSet<usize>]) -> Vec<Vec<usize>> {
    let mut consumers = vec![Vec::new(); dependencies.len()];
    for (consumer, producers) in dependencies.iter().enumerate() {
        for &producer in producers {
            consumers[producer].push(consumer);
        }
    }
    let mut seen = vec![false; dependencies.len()];
    let mut finished = Vec::with_capacity(dependencies.len());
    let mut stack = Vec::new();
    for root in 0..dependencies.len() {
        stack.push((root, false));
        while let Some((index, leaving)) = stack.pop() {
            if leaving {
                finished.push(index);
            } else if !std::mem::replace(&mut seen[index], true) {
                stack.push((index, true));
                stack.extend(
                    dependencies[index]
                        .iter()
                        .map(|&producer| (producer, false)),
                );
            }
        }
    }
    seen.fill(false);
    let mut components = Vec::new();
    let mut stack = Vec::new();
    for root in finished.into_iter().rev() {
        if seen[root] {
            continue;
        }
        let mut component = Vec::new();
        stack.push(root);
        while let Some(index) = stack.pop() {
            if !std::mem::replace(&mut seen[index], true) {
                component.push(index);
                stack.extend(consumers[index].iter().copied());
            }
        }
        components.push(component);
    }
    components
}

pub(super) fn conditions_for_mods(
    sources: &super::mods::ModsState,
    inventory: &sunrise_mod_package::enabled::EnabledMods,
) -> Result<sunrise_world::conditions::LoadConditions, String> {
    if let Some(candidate) = &sources.load_condition_candidate {
        return Ok(candidate.with_mod_inventory(inventory.server_ids()));
    }
    let table = sources
        .load_conditions
        .get_or_init(|| {
            let mut registrar = mod_api::LoadConditionDeclarations::new(
                sunrise_world::conditions::is_builtin_condition,
            );
            Game::native_register_load_conditions(
                &mod_api::RegistrationContext::new(),
                &mut registrar,
            );
            let entries = registrar.finish().map_err(|error| error.message)?;
            // Every candidate shares the owner of each retained registration.
            Ok(Arc::new(
                entries
                    .into_iter()
                    .map(|(id, (owner, handler))| (id, (CallbackOwner::for_mod(owner), handler)))
                    .collect(),
            ))
        })
        .clone()?;
    let conditions = sunrise_world::conditions::LoadConditions::new(inventory.server_ids());
    Ok(
        conditions.with_lookup(table.iter().map(|(id, (owner, handler))| {
            let name = id.to_string();
            let enabled = sources.code_owner_enabled(owner.id());
            let owner = owner.clone();
            let handler = handler.clone();
            (name.clone(), move |args: &serde_json::Value| {
                if !enabled {
                    return Ok(false);
                }
                let mut arguments = args.as_object().cloned().expect("decoded condition object");
                arguments.remove("type");
                let arguments = serde_json::Value::Object(arguments);
                owner
                    .call("load condition", "test_load_condition", || {
                        handler.test(&arguments)
                    })
                    .map_err(|reason| format!("condition {name}: {reason}"))?
                    .map_err(|error| format!("condition {name}: mod {}: {error}", owner.id()))
            })
        })),
    )
}

#[cfg(test)]
#[path = "load_conditions/startup_tests.rs"]
mod startup_tests;

#[cfg(test)]
#[path = "load_conditions/tests.rs"]
mod tests;

pub(super) fn content_root_with_conditions(
    root: &std::path::Path,
    jars: &std::path::Path,
    mods: &[sunrise_client::mod_content::ContentMod],
    cache: &std::path::Path,
    inventory: &sunrise_mod_package::enabled::EnabledMods,
    sources: &super::mods::ModsState,
) -> sunrise_client::Result<std::path::PathBuf> {
    let conditions = conditions_for_mods(sources, inventory)?;
    sunrise_client::mod_content::resource_root_with_conditions(
        root,
        jars,
        mods,
        cache,
        inventory,
        &conditions,
    )
}

#[cfg(test)]
#[path = "load_conditions/reload_tests.rs"]
mod reload_tests;
