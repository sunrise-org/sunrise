#![cfg(test)]
use crate::settings::Settings;
use std::sync::Arc;
use std::time::Instant;
use sunrise_client_gameplay::game::client_side::resource_load_client::UiPublishArg;
use sunrise_client_gameplay::game::client_side::resource_loader::Loaded;
use sunrise_client_gameplay::game::packs;
use sunrise_game_data::Result;
use sunrise_gameplay_common::game::mods;
use sunrise_gameplay_common::game::resource_load::Progress;
#[cfg(test)]
#[path = "resource_load/test_cache.rs"]
pub(crate) mod test_cache;

/// Fixture helper for tests without a launch mod state.
#[allow(clippy::too_many_arguments)]
#[cfg(test)]
pub(crate) fn load_resources(
    roots: impl Into<packs::PackRoots>,
    additional_data: Vec<Arc<sunrise_level::datapack::DataPack>>,
    launch_world: Option<&str>,
    vanilla_root: &std::path::Path,
    settings: &Settings,
    started: Instant,
    menu: bool,
    role: impl Into<mods::LoadRole>,
    progress: &Progress,
    publish: &mut dyn FnMut(UiPublishArg, &std::path::Path),
) -> Result<Loaded> {
    mod_platform::__private::test_platform::install();
    test_cache::load(
        test_cache::LoadArgs {
            roots: roots.into(),
            additional_data,
            launch_world,
            vanilla_root,
            settings,
            started,
            menu,
            skip_lang: false,
            role: role.into(),
            progress,
        },
        publish,
    )
}

/// [`load_resources`] that leaves process-wide language untouched, for `isolated_server_game`.
#[allow(clippy::too_many_arguments)]
#[cfg(test)]
pub(crate) fn load_resources_skip_lang(
    roots: impl Into<packs::PackRoots>,
    additional_data: Vec<Arc<sunrise_level::datapack::DataPack>>,
    launch_world: Option<&str>,
    vanilla_root: &std::path::Path,
    settings: &Settings,
    started: Instant,
    role: impl Into<mods::LoadRole>,
    progress: &Progress,
    publish: &mut dyn FnMut(UiPublishArg, &std::path::Path),
) -> Result<Loaded> {
    mod_platform::__private::test_platform::install();
    test_cache::load(
        test_cache::LoadArgs {
            roots: roots.into(),
            additional_data,
            launch_world,
            vanilla_root,
            settings,
            started,
            menu: false,
            skip_lang: true,
            role: role.into(),
            progress,
        },
        publish,
    )
}

/// Private, uncached [`load_resources`] for tests that mutate `Resources` or test distinct loads.
#[allow(clippy::too_many_arguments)]
#[cfg(test)]
pub(crate) fn load_resources_uncached(
    roots: impl Into<packs::PackRoots>,
    additional_data: Vec<Arc<sunrise_level::datapack::DataPack>>,
    launch_world: Option<&str>,
    vanilla_root: &std::path::Path,
    settings: &Settings,
    started: Instant,
    menu: bool,
    role: impl Into<mods::LoadRole>,
    progress: &Progress,
    publish: &mut dyn FnMut(UiPublishArg, &std::path::Path),
) -> Result<Loaded> {
    mod_platform::__private::test_platform::install();
    test_cache::load_uncached(
        test_cache::LoadArgs {
            roots: roots.into(),
            additional_data,
            launch_world,
            vanilla_root,
            settings,
            started,
            menu,
            skip_lang: false,
            role: role.into(),
            progress,
        },
        publish,
    )
}
