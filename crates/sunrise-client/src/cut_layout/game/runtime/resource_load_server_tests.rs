use std::{path::Path, sync::Arc, time::Instant};
use sunrise_assets::resource_source::ResourceRoot;
use sunrise_client_lib::settings::Settings;
use sunrise_game_data::Result;
use sunrise_gameplay::game::native_entity_refs::tests::{
    load_server_resources_cached, load_server_resources_uncached,
};
use sunrise_gameplay_common::game::{
    PackResources, Resources, mods::LoadRole, resource_load::Progress,
};
use sunrise_level::datapack::DataPack;

pub(crate) struct Loaded {
    pub resources: Arc<Resources>,
    pub pack: PackResources,
}

pub(crate) fn load_resources(
    root: ResourceRoot,
    additional_data: Vec<Arc<DataPack>>,
    launch_world: Option<&str>,
    vanilla_root: &Path,
    _settings: &Settings,
    started: Instant,
    menu: bool,
    role: impl Into<LoadRole>,
    progress: &Progress,
    _publish: &mut dyn FnMut((), &Path),
) -> Result<Loaded> {
    let role = role.into();
    assert!(launch_world.is_none() && !menu && !role.execution.is_client());
    mod_platform::__private::test_platform::install();
    let (resources, pack) = load_server_resources_cached(
        root,
        additional_data,
        vanilla_root,
        role,
        1,
        started,
        false,
        progress,
        true,
    )?;
    Ok(Loaded { resources, pack })
}

pub(crate) fn load_resources_uncached(
    root: ResourceRoot,
    additional_data: Vec<Arc<DataPack>>,
    launch_world: Option<&str>,
    vanilla_root: &Path,
    _settings: &Settings,
    started: Instant,
    menu: bool,
    role: impl Into<LoadRole>,
    progress: &Progress,
    _publish: &mut dyn FnMut((), &Path),
) -> Result<Loaded> {
    let role = role.into();
    assert!(launch_world.is_none() && !menu && !role.execution.is_client());
    mod_platform::__private::test_platform::install();
    let (resources, pack) = load_server_resources_uncached(
        root,
        additional_data,
        vanilla_root,
        role,
        1,
        started,
        false,
        progress,
        true,
    )?;
    Ok(Loaded { resources, pack })
}

pub(crate) fn load_resources_skip_lang(
    root: ResourceRoot,
    additional_data: Vec<Arc<DataPack>>,
    launch_world: Option<&str>,
    vanilla_root: &Path,
    _settings: &Settings,
    started: Instant,
    role: impl Into<LoadRole>,
    progress: &Progress,
    _publish: &mut dyn FnMut((), &Path),
) -> Result<Loaded> {
    let role = role.into();
    assert!(launch_world.is_none() && !role.execution.is_client());
    mod_platform::__private::test_platform::install();
    let (resources, pack) = load_server_resources_cached(
        root,
        additional_data,
        vanilla_root,
        role,
        1,
        started,
        true,
        progress,
        true,
    )?;
    Ok(Loaded { resources, pack })
}
