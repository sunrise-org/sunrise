//! Client presentation for the shared prepared-resource fixture cache.
use super::{
    packs,
    resource_load_client::{UiPublishArg, load_resources_for_mods},
    resource_loader::Loaded,
    resource_tables_client::{ClientPackResources, ClientResources, UiResources},
    shader_packs,
};
use std::{
    any::Any,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock},
    time::Instant,
};
use sunrise_assets::resource_source::ResourceRoot;
use sunrise_client::settings::Settings;
use sunrise_game_data::{Result, lang};
use sunrise_gameplay_common::game::{
    api_values::TryIntoApi,
    mods,
    resource_load::{Progress, with_skip_facts_memo},
    test_fixtures::acquire_in_process_load_permit,
    test_resource_cache::{
        CacheKey, ClientCacheKey, get_or_build, hydrate_common_hit,
        is_plain_directory_resource_root,
    },
};
use sunrise_level::datapack::DataPack;

struct ClientCachedExtra {
    entity_models: Option<Arc<sunrise_client::mobs::EntityModelDocument>>,
    client_res: Arc<OnceLock<ClientResources>>,
    client_pack: ClientPackResources,
    gui: Mutex<UiResources>,
}

pub fn plain_directory_root(roots: &packs::PackRoots) -> Option<PathBuf> {
    let selected = roots.selected.as_ref().ok()?;
    if roots.cleared.is_some() || roots.base.prepared() != selected.prepared() {
        return None;
    }
    is_plain_directory_resource_root(selected)
}

fn client_inputs_are_vanilla() -> bool {
    !(mod_api::__private::listened(mod_api::HookId::AddClientReloadListeners)
        || mod_api::__private::listened(mod_api::HookId::RegisterTextureAtlases)
        || sunrise_client_render::mesher::has_custom_block_tint_sources()
        || sunrise_client_render::mesher::has_section_geometry()
        || sunrise_client_render::models::has_standalone_models()
        || sunrise_client_render::mod_sprite_sources::has_registered()
        || sunrise_client_render::mod_pipelines::has_registered()
        || !sunrise_client_render::fluid_models::resolved().is_empty()
        || sunrise_client_render::fluid_models::any_flat_look()
        || sunrise_client::item_render::ItemRenderer::has_registered_providers())
}

#[allow(clippy::too_many_arguments)]
pub fn load(
    roots: packs::PackRoots,
    additional_data: Vec<Arc<DataPack>>,
    launch_world: Option<&str>,
    vanilla_root: &Path,
    settings: &Settings,
    started: Instant,
    menu: bool,
    skip_lang: bool,
    role: mods::LoadRole,
    progress: &Progress,
    publish: &mut dyn FnMut(UiPublishArg, &Path),
) -> Result<Loaded> {
    #[cfg(test)]
    mod_platform::__private::test_platform::install();

    if !menu
        && additional_data.is_empty()
        && launch_world.is_none()
        && (!role.execution.is_client() || client_inputs_are_vanilla())
        && let Some(root) = plain_directory_root(&roots)
        && let Some(key) = CacheKey::for_vanilla_root(
            &root,
            vanilla_root,
            role,
            settings.sunrise.worker_threads(),
            role.execution.is_client().then(|| ClientCacheKey {
                texture_budget_mib: settings.sunrise.texture_budget_mib,
                labpbr_maps: shader_packs::picks_material_pack(settings),
                skip_modded_splashes: settings.extras.skip_modded_splashes_if_vanilla_modified,
            }),
        )
    {
        let build_settings = settings.clone();
        let cached = get_or_build(key.clone(), move |key| {
            let mut loaded = load_private(
                ResourceRoot::directory(key.root.clone()).into(),
                Vec::new(),
                None,
                &key.vanilla_root,
                &build_settings,
                Instant::now(),
                false,
                role,
                &Progress::new(),
                &mut |_, _| unreachable!("cached build runs with menu=false"),
            )
            .map_err(|e| e.to_string())?;
            let extra: Option<Arc<dyn Any + Send + Sync>> = if key.client.is_some() {
                Some(Arc::new(ClientCachedExtra {
                    entity_models: loaded.entity_models.take(),
                    client_res: loaded
                        .client_res
                        .take()
                        .ok_or("cached build produced no client resource slot")?,
                    client_pack: loaded
                        .client_pack
                        .take()
                        .ok_or("cached build produced no client pack")?,
                    gui: Mutex::new(
                        loaded
                            .gui
                            .take()
                            .ok_or("cached build produced no GUI bundle")?,
                    ),
                }))
            } else {
                None
            };
            Ok((loaded.resources, loaded.pack, extra))
        })?;
        if let Some(loaded) = hydrate_hit(&cached, &key, settings, skip_lang)? {
            progress.set(9);
            return Ok(loaded);
        }
    }
    load_uncached(
        roots,
        additional_data,
        launch_world,
        vanilla_root,
        settings,
        started,
        menu,
        skip_lang,
        role,
        progress,
        publish,
    )
}

fn hydrate_hit(
    cached: &sunrise_gameplay_common::game::test_resource_cache::CachedEntry,
    key: &CacheKey,
    settings: &Settings,
    skip_lang: bool,
) -> Result<Option<Loaded>> {
    let extra = if let Some(client) = &key.client {
        let data = cached
            .extra
            .as_deref()
            .and_then(|extra| extra.downcast_ref::<ClientCachedExtra>())
            .ok_or("missing cached client presentation")?;
        let mut client_pack = data.client_pack.clone();
        let Some(item_renderer) = client_pack.item_renderer.clone_for_tests() else {
            return Ok(None);
        };
        client_pack.item_renderer = Arc::new(item_renderer);
        sunrise_client::sprites::set_texture_budget(client.texture_budget_mib);
        sunrise_client::sprites::set_labpbr_maps(client.labpbr_maps);
        Some((data, client_pack))
    } else {
        None
    };
    let Some((resources, pack, registry)) = hydrate_common_hit(
        cached,
        key,
        &settings.language,
        key.role.is_dedicated_server(),
        skip_lang,
    )?
    else {
        return Ok(None);
    };
    let (entity_models, client_pack, client_res, gui) = if let Some((data, mut client_pack)) = extra
    {
        sunrise_client::pack_palette::set(&client_pack.polytone_colors);
        sunrise_level::brewing::set_pack_colors(client_pack.polytone_colors.effects.clone());
        if !std::ptr::eq(registry, cached.pack.assets.registry) {
            client_pack.context = Arc::new(sunrise_client_render::streaming::WorkerContext {
                chunk: (*pack.chunk_context).clone(),
                mesh: client_pack.context.mesh.clone(),
            });
            client_pack.water_overlay_state = registry
                .default_state("minecraft:water")
                .ok_or("block registry is missing the default minecraft:water state")?
                .try_into_api()
                .map_err(|e| format!("cannot bind the default minecraft:water state: {e:?}"))?;
        }
        let gui = data
            .gui
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone_for_tests();
        (
            data.entity_models.clone(),
            Some(client_pack),
            Some(data.client_res.clone()),
            Some(gui),
        )
    } else {
        (None, None, None, None)
    };
    Ok(Some(Loaded {
        game_cpu: None,
        entity_models,
        renderer_cpu: None,
        root: cached.root.clone(),
        pack_error: None,
        resources,
        client_res,
        rest_loader: None,
        client_applies: Vec::new(),
        pack,
        client_pack,
        gui,
    }))
}

#[allow(clippy::too_many_arguments)]
pub fn load_uncached(
    roots: packs::PackRoots,
    additional_data: Vec<Arc<DataPack>>,
    launch_world: Option<&str>,
    vanilla_root: &Path,
    settings: &Settings,
    started: Instant,
    menu: bool,
    skip_lang: bool,
    role: mods::LoadRole,
    progress: &Progress,
    publish: &mut dyn FnMut(UiPublishArg, &Path),
) -> Result<Loaded> {
    #[cfg(test)]
    mod_platform::__private::test_platform::install();

    let _permit = acquire_in_process_load_permit();
    let _language =
        skip_lang.then(|| lang::isolate_publication(lang::Language::of(lang::DEFAULT, [])));
    load_private(
        roots,
        additional_data,
        launch_world,
        vanilla_root,
        settings,
        started,
        menu,
        role,
        progress,
        publish,
    )
}

// The caller owns the heavy-load permit, either in get_or_build or in load_uncached.
#[allow(clippy::too_many_arguments)]
fn load_private(
    roots: packs::PackRoots,
    additional_data: Vec<Arc<DataPack>>,
    launch_world: Option<&str>,
    vanilla_root: &Path,
    settings: &Settings,
    started: Instant,
    menu: bool,
    role: mods::LoadRole,
    progress: &Progress,
    publish: &mut dyn FnMut(UiPublishArg, &Path),
) -> Result<Loaded> {
    let mut loaded = with_skip_facts_memo(|| {
        load_resources_for_mods(
            roots,
            additional_data,
            launch_world,
            vanilla_root,
            settings,
            started,
            menu,
            role,
            progress,
            publish,
            &mut || unreachable!("fixture without pack recovery"),
            &mods::ModsState::default(),
        )
    })?;
    if !menu && role.execution.is_client() {
        if let Some(gui) = loaded.gui.as_mut() {
            gui.atlas.pack_pixels_sync();
        }
        if let Some(client_pack) = &loaded.client_pack {
            client_pack.render.pixels.pack_sync();
        }
    }
    Ok(loaded)
}
