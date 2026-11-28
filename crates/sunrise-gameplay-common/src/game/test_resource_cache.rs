//! Immutable prepared resources shared by the server, client and joined test fixtures.
use super::{
    PackResources, Resources, api_values, execution_role, mods,
    resource_load::publish_entity_type_defaults, test_fixtures::acquire_in_process_load_permit,
};
use crate::prepared_test_root::configured;
use std::{
    any::Any,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock},
    time::SystemTime,
};
use sunrise_assets::resource_source::ResourceRoot;
use sunrise_game_data::lang;
use sunrise_level::assets::Assets;
use sunrise_world::registry::BlockRegistry;

pub type FileStamp = Option<(u64, Option<SystemTime>)>;

const STAMPED_FILES: [&str; 8] = [
    "sunrise-resources.json",
    "facts.json",
    "block-tags.json",
    "block-behaviours.json",
    "gameplay-facts.json",
    "items.json",
    "registries.json",
    "mob-data.json",
];

#[derive(Clone, PartialEq, Eq)]
pub struct ClientCacheKey {
    pub texture_budget_mib: u32,
    pub labpbr_maps: bool,
    pub skip_modded_splashes: bool,
}

#[derive(Clone, PartialEq, Eq)]
pub struct CacheKey {
    pub root: PathBuf,
    pub vanilla_root: PathBuf,
    pub role: execution_role::ExecutionRole,
    pub owning: mods::item_data_load::OwningLoad,
    pub worker_threads: usize,
    pub client: Option<ClientCacheKey>,
    pub stamps: [FileStamp; STAMPED_FILES.len()],
}

impl CacheKey {
    pub fn for_vanilla_root(
        root: &ResourceRoot,
        vanilla_root: &Path,
        role: mods::LoadRole,
        worker_threads: usize,
        client: Option<ClientCacheKey>,
    ) -> Option<Self> {
        let prepared = canonical_prepared_root()?;
        if !is_same_prepared_root(root, vanilla_root, &prepared)
            || !load_inputs_are_vanilla_common(&prepared, role)
        {
            return None;
        }
        Some(Self {
            stamps: root_stamps(&prepared),
            root: prepared.clone(),
            vanilla_root: prepared,
            role: role.execution,
            owning: role.owning,
            worker_threads,
            client,
        })
    }
}

struct IteratedRegistries {
    entity_types: Vec<(String, Option<bool>)>,
    game_events: Option<Vec<String>>,
    menus: Option<Vec<String>>,
}

pub struct CachedEntry {
    pub root: PathBuf,
    pub vanilla_root: PathBuf,
    pub resources: Arc<Resources>,
    pub pack: PackResources,
    pub extra: Option<Arc<dyn Any + Send + Sync>>,
    iterated: IteratedRegistries,
}

type Slot = Arc<OnceLock<std::result::Result<Arc<CachedEntry>, String>>>;
const KEPT: usize = 4;
static CACHE: Mutex<Vec<(CacheKey, Slot)>> = Mutex::new(Vec::new());

fn file_stamp(path: &Path) -> FileStamp {
    std::fs::metadata(path)
        .ok()
        .map(|meta| (meta.len(), meta.modified().ok()))
}

fn root_stamps(root: &Path) -> [FileStamp; STAMPED_FILES.len()] {
    STAMPED_FILES.map(|name| file_stamp(&root.join(name)))
}

pub fn is_plain_directory_resource_root(root: &ResourceRoot) -> bool {
    let prepared = root.prepared();
    root.mod_layers().is_empty()
        && root.data_changes().is_empty()
        && root.source().as_directory() == Some(prepared)
        && root.data().as_directory() == Some(prepared)
}

fn canonical_prepared_root() -> Option<PathBuf> {
    let root = configured::prepared_root()?.canonicalize().ok()?;
    root.is_dir().then_some(root)
}

fn is_same_prepared_root(root: &ResourceRoot, vanilla_root: &Path, prepared: &Path) -> bool {
    is_plain_directory_resource_root(root)
        && root.prepared().canonicalize().as_deref().ok() == Some(prepared)
        && vanilla_root.canonicalize().as_deref().ok() == Some(prepared)
}

fn load_inputs_are_vanilla_common(root: &Path, load_role: mods::LoadRole) -> bool {
    // Generated content-mod roots are one-off temp directories and may have empty `enabled-mods.json`.
    if root.join("sunrise-generated-content-files.json").exists() {
        return false;
    }
    let Ok(inventory) = sunrise_mod_package::enabled::EnabledMods::read(root) else {
        return false;
    };
    if inventory.server_ids().next().is_some() || inventory.client_ids().next().is_some() {
        return false;
    }
    if !mod_api::__private::attribute_declarations().is_empty()
        || !mod_api::__private::attribute_bounds_widenings().is_empty()
        || !mod_api::__private::entity_attribute_declarations().is_empty()
        || !mod_api::__private::entity_attribute_modifications().is_empty()
        || !mod_api::__private::spawn_placement_declarations().is_empty()
        || !mod_api::__private::entity_part_state_declarations().is_empty()
        || !sunrise_world::fluid::mod_fluids().is_empty()
        || !super::general::general_type_catalog().is_empty()
        || mod_api::__private::listened(mod_api::HookId::ModifyDefaultComponents)
        || mod_api::__private::listened(mod_api::HookId::ExtendPoiTypes)
        || mod_api::__private::listened(mod_api::HookId::RegisterCauldronFluidContent)
        || (load_role.owning == mods::item_data_load::OwningLoad::Startup
            && mod_api::__private::RegistrationData::initial_settings_snapshot().is_some())
    {
        return false;
    }
    // Hooks whose resolution mutates interior-mutable caches inside `Resources` keep private loads.
    if mod_api::__private::has_part_hook(mod_api::HookId::CollisionShape)
        || mod_api::__private::has_part_hook(mod_api::HookId::PistonPushReaction)
        || mod_api::__private::has_part_hook(mod_api::HookId::IsPathfindable)
        || mod_api::__private::has_part_hook(mod_api::HookId::SupportsEnchantment)
        || mod_api::__private::has_part_hook(mod_api::HookId::IsPrimaryItemFor)
    {
        return false;
    }
    true
}

fn load_iterated_registries(root: &Path) -> std::result::Result<IteratedRegistries, String> {
    use sunrise_protocol::registry::Builtin;
    let builtin = Builtin::load(&root.join("registries.json")).map_err(|e| e.to_string())?;
    let ids = &builtin
        .get("minecraft:entity_type")
        .ok_or_else(|| "missing entity type registry".to_owned())?
        .names;
    let rows = super::entity_type_rows::entity_type_rows(root);
    let entity_types = ids
        .iter()
        .filter(|id| !id.is_empty())
        .map(|id| (id.clone(), rows.contains(id).then(|| rows.is_living(id))))
        .collect();
    let game_events = builtin
        .get("minecraft:game_event")
        .map(|ids| ids.names.clone());
    let menus = builtin.get("minecraft:menu").map(|ids| ids.names.clone());
    Ok(IteratedRegistries {
        entity_types,
        game_events,
        menus,
    })
}

pub fn get_or_build(
    key: CacheKey,
    build: impl FnOnce(
        &CacheKey,
    ) -> Result<
        (
            Arc<Resources>,
            PackResources,
            Option<Arc<dyn Any + Send + Sync>>,
        ),
        String,
    > + Send
    + 'static,
) -> Result<Arc<CachedEntry>, String> {
    #[cfg(test)]
    mod_platform::__private::test_platform::install();
    let slot = {
        let mut entries = CACHE.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(pos) = entries.iter().position(|(k, _)| *k == key) {
            let entry = entries.remove(pos);
            let slot = entry.1.clone();
            entries.push(entry);
            slot
        } else {
            if entries.len() >= KEPT {
                entries.remove(0);
            }
            let slot = Arc::new(OnceLock::new());
            entries.push((key.clone(), slot.clone()));
            slot
        }
    };
    slot.get_or_init(|| {
        let _permit = acquire_in_process_load_permit();
        // Start with neutral fixture scopes, not the caller's TEST_MODS/TEST_PARTS.
        std::thread::Builder::new()
            .stack_size(sunrise_threads::GAME_THREAD_STACK_BYTES)
            .name("test-res-cache".into())
            .spawn(move || {
                mod_api::testing::engine_platform::install_parts_fixture(&[])
                    .map_err(|errors| errors.join("\n"))?;
                // Only hydration publishes the requesting fixture's language (if wanted).
                let _language = lang::isolate_publication(lang::Language::of(lang::DEFAULT, []));
                let (resources, pack, extra) = build(&key)?;
                let iterated = load_iterated_registries(&key.root)?;
                Ok(Arc::new(CachedEntry {
                    root: key.root,
                    vanilla_root: key.vanilla_root,
                    resources,
                    pack,
                    extra,
                    iterated,
                }))
            })
            .map_err(|e| e.to_string())?
            .join()
            .unwrap_or_else(|panic| std::panic::resume_unwind(panic))
    })
    .clone()
}

fn cached_language(
    root: &Path,
    vanilla_root: &Path,
    code: &str,
    use_default_lang: bool,
) -> Arc<lang::Language> {
    #[derive(PartialEq, Eq)]
    struct LangKey {
        root: PathBuf,
        code: String,
        use_default_lang: bool,
        stamp: FileStamp,
    }
    type LangSlot = Arc<OnceLock<Arc<lang::Language>>>;
    static LANGS: Mutex<Vec<(LangKey, LangSlot)>> = Mutex::new(Vec::new());

    let dir = if use_default_lang { vanilla_root } else { root };
    let lang_code = if use_default_lang {
        lang::DEFAULT.to_owned()
    } else {
        code.to_owned()
    };
    let key = LangKey {
        root: dir.to_path_buf(),
        stamp: file_stamp(&dir.join(format!("assets/minecraft/lang/{lang_code}.json"))),
        code: lang_code.clone(),
        use_default_lang,
    };
    let slot = {
        let mut entries = LANGS.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(pos) = entries.iter().position(|(k, _)| *k == key) {
            let entry = entries.remove(pos);
            let slot = entry.1.clone();
            entries.push(entry);
            slot
        } else {
            if entries.len() >= 8 {
                entries.remove(0);
            }
            let slot = Arc::new(OnceLock::new());
            entries.push((key, slot.clone()));
            slot
        }
    };
    slot.get_or_init(|| {
        let language = if use_default_lang {
            lang::Language::load_default(&vanilla_root.to_path_buf())
        } else {
            let source =
                sunrise_assets::resource_source::ResourceSource::directory(root.to_path_buf());
            lang::Language::load(&[&source], code)
        };
        Arc::new(language)
    })
    .clone()
}

fn replay_iterated_registries(iterated: &IteratedRegistries) -> std::result::Result<(), String> {
    let mut living = std::collections::HashMap::with_capacity(iterated.entity_types.len());
    for (id, is_living) in &iterated.entity_types {
        living.insert(id.as_str(), *is_living);
    }
    mod_api::__private::resolve_entity_types(
        iterated.entity_types.iter().map(|(id, _)| id.as_str()),
        |id| living.get(id).copied().flatten(),
    )
    .map_err(|errors| errors.join("\n"))?;
    if let Some(events) = &iterated.game_events {
        mod_api::__private::resolve_game_events(events.iter().map(String::as_str))
            .map_err(|errors| errors.join("\n"))?;
    }
    if let Some(menus) = &iterated.menus {
        mod_api::__private::resolve_menus(menus.iter().map(String::as_str))
            .map_err(|errors| errors.join("\n"))?;
    }
    Ok(())
}

/// Rebind fixture-local registry identities before returning shared immutable resources.
pub fn hydrate_common_hit(
    cached: &CachedEntry,
    language_code: &str,
    use_default_lang: bool,
    skip_lang: bool,
) -> Result<Option<(Arc<Resources>, PackResources, &'static BlockRegistry)>, String> {
    // The prepared-root build initializes the process-global tables. Hits never replay them;
    // only the requesting fixture's parts, keys, and language need rebinding.
    if !mod_api::__private::parts_installation::parts_installed() {
        mod_api::testing::engine_platform::install_parts_fixture(&[])
            .map_err(|errors| errors.join("\n"))?;
    }
    replay_iterated_registries(&cached.iterated)?;
    // Mods that requested custom entity-type keys change `type_key` slots inside `Resources`.
    let entity_keys_match = sunrise_sim::health::LivingTypeKeys::resolve()
        == cached.resources.living_type_keys()
        && cached
            .resources
            .mob_types()
            .values()
            .all(|mob| mob.type_key == mod_api::__private::resolved_entity_type_key(&mob.id));
    if !entity_keys_match {
        return Ok(None);
    }
    publish_entity_type_defaults(cached.resources.core.motion.attributes());
    let registry = api_values::resolve_cached_block_definitions(cached.pack.assets.registry)?;
    api_values::resolve_item_keys(cached.resources.items())?;
    if !skip_lang {
        lang::set_fixture_language(cached_language(
            &cached.root,
            &cached.vanilla_root,
            language_code,
            use_default_lang,
        ));
    }

    let mut pack = cached.pack.clone();
    if !std::ptr::eq(registry, cached.pack.assets.registry) {
        let assets = Arc::new(Assets {
            registry,
            properties: cached.pack.assets.properties.clone(),
            material_states: cached.pack.assets.material_states,
            blocks_fluid_flow: cached.pack.assets.blocks_fluid_flow.clone(),
        });
        let old = &cached.pack.chunk_context;
        pack.chunk_context = Arc::new(sunrise_level::chunk_streamer::ChunkWorkerContext {
            assets: assets.clone(),
            biomes: old.biomes.clone(),
            light: old.light.clone(),
        });
        pack.assets = assets;
    }
    Ok(Some((cached.resources.clone(), pack, registry)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use sunrise_assets::resource_source::ResourceSource;

    #[test]
    fn only_the_plain_prepared_root_matches() {
        let dir = tempfile::tempdir().unwrap();
        let other = tempfile::tempdir().unwrap();
        let prepared = dir.path().canonicalize().unwrap();
        let root = ResourceRoot::directory(dir.path().to_path_buf());
        let other_root = ResourceRoot::directory(other.path().to_path_buf());
        assert!(is_same_prepared_root(&root, dir.path(), &prepared));
        assert!(!is_same_prepared_root(&other_root, dir.path(), &prepared));
        assert!(!is_same_prepared_root(&root, other.path(), &prepared));

        let overlaid = root.with_mod_layers(Arc::from([ResourceSource::default()]));
        assert!(!is_same_prepared_root(&overlaid, dir.path(), &prepared));
    }

    #[test]
    fn rewriting_a_stamped_file_changes_the_key() {
        let dir = tempfile::tempdir().unwrap();
        let facts = dir.path().join("facts.json");
        std::fs::write(&facts, b"{}").unwrap();
        let before = root_stamps(dir.path());
        std::fs::write(&facts, b"[]\n").unwrap();
        assert_ne!(before, root_stamps(dir.path()));
    }
}
