//! Game fixtures exposed only to external tests through the `test-support` feature.

#[cfg(test)]
use mod_platform;
use tempfile;

use sunrise_gameplay::game::native_entity_refs::tests::prepare_entity_ticking_neighborhood;
use sunrise_gameplay_common::game::test_fixtures::{
    fixture_facts_mut, fixture_root, resolve_parts,
};

/// Resource-loading fixtures publish process-wide palette and save namespace
/// state. Keep the fixture serialized and put both globals back when it ends.
struct ScopedFixtureGlobals {
    previous_namespaces: Option<Vec<String>>,
    palette: Option<sunrise_level::chunk::test_support::BlockStatePaletteScope>,
    lock: Option<std::sync::MutexGuard<'static, ()>>,
}

static FIXTURE_GLOBALS: std::sync::Mutex<()> = std::sync::Mutex::new(());

impl ScopedFixtureGlobals {
    fn new() -> Self {
        let lock = FIXTURE_GLOBALS.lock().unwrap_or_else(|e| e.into_inner());
        let palette = sunrise_level::chunk::test_support::BlockStatePaletteScope::new();
        let previous_namespaces = sunrise_sim::save::level_mods::block_namespaces();
        Self {
            previous_namespaces,
            palette: Some(palette),
            lock: Some(lock),
        }
    }
}

impl Drop for ScopedFixtureGlobals {
    fn drop(&mut self) {
        sunrise_sim::save::level_mods::set_block_namespaces(self.previous_namespaces.take());
        self.palette.take();
        self.lock.take();
    }
}

/// Runs a resource-loading fixture while its process-wide globals are scoped.
#[doc(hidden)]
pub fn with_scoped_fixture_globals<T>(test: impl FnOnce() -> T) -> T {
    let _globals = ScopedFixtureGlobals::new();
    test()
}

#[cfg(test)]
fn prepare_game_boot_with_test_platform(root: &std::path::Path) {
    sunrise_gameplay_common::game::test_fixtures::prepare_game_boot(
        root,
        mod_platform::__private::part_tests::register_game_keys,
    );
}

#[cfg(feature = "client")]
pub fn with_game_with_registration(
    prepare_game_boot: fn(&std::path::Path),
    test: impl FnOnce(&mut sunrise_gameplay::game::ServerGame<'_>),
) {
    with_game_role_and_facts_with(
        fixture_root(),
        sunrise_gameplay_common::game::execution_role::ExecutionRole::IntegratedServer,
        None,
        true,
        prepare_game_boot,
        test,
    );
}

/// Boots a server with the origin's entity-ticking neighbourhood loaded.
#[cfg(feature = "client")]
pub fn with_resident_game_with_registration(
    prepare_game_boot: fn(&std::path::Path),
    test: impl FnOnce(&mut sunrise_gameplay::game::ServerGame<'_>),
) {
    with_game_with_registration(prepare_game_boot, |game| {
        prepare_entity_ticking_neighborhood(game, sunrise_core::position::ChunkPos::new(0, 0));
        test(game);
    });
}

#[cfg(all(test, feature = "client"))]
pub(crate) fn with_game(test: impl FnOnce(&mut sunrise_gameplay::game::ServerGame<'_>)) {
    with_game_with_registration(prepare_game_boot_with_test_platform, test);
}

#[cfg(all(test, feature = "client"))]
pub(crate) fn with_resident_game(test: impl FnOnce(&mut sunrise_gameplay::game::ServerGame<'_>)) {
    with_resident_game_with_registration(prepare_game_boot_with_test_platform, test);
}

#[cfg(all(test, feature = "client"))]
pub(crate) fn with_game_at(
    root: std::path::PathBuf,
    test: impl FnOnce(&mut sunrise_gameplay::game::ServerGame<'_>),
) {
    with_game_role(
        root,
        sunrise_gameplay_common::game::execution_role::ExecutionRole::IntegratedServer,
        test,
    );
}

#[cfg(all(test, feature = "client"))]
pub(crate) fn with_client_game(
    test: impl FnOnce(&mut sunrise_client_gameplay::game::ClientGame<'_>),
) {
    with_game_role_client(
        fixture_root(),
        sunrise_gameplay_common::game::execution_role::ExecutionRole::Client,
        test,
    );
}

#[cfg(all(test, feature = "client"))]
pub(crate) fn with_uncached_client_game(
    test: impl FnOnce(&mut sunrise_client_gameplay::game::ClientGame<'_>),
) {
    with_game_role_and_facts_with_client(
        fixture_root(),
        sunrise_gameplay_common::game::execution_role::ExecutionRole::Client,
        None,
        false,
        test,
    );
}

#[cfg(all(test, feature = "client"))]
pub(crate) fn with_game_role(
    root: std::path::PathBuf,
    role: sunrise_gameplay_common::game::execution_role::ExecutionRole,
    test: impl FnOnce(&mut sunrise_gameplay::game::ServerGame<'_>),
) {
    with_game_role_and_facts(root, role, None, test);
}

#[cfg(all(test, feature = "client"))]
pub(crate) fn with_game_role_client(
    root: std::path::PathBuf,
    role: sunrise_gameplay_common::game::execution_role::ExecutionRole,
    test: impl FnOnce(&mut sunrise_client_gameplay::game::ClientGame<'_>),
) {
    with_game_role_and_facts_client(root, role, None, test);
}

/// Configure the fixture's owned gameplay facts before constructing the game.
#[cfg(all(test, feature = "client"))]
pub(crate) fn with_game_role_and_facts(
    root: std::path::PathBuf,
    role: sunrise_gameplay_common::game::execution_role::ExecutionRole,
    configure: Option<
        fn(&sunrise_world::registry::BlockRegistry, &mut sunrise_items::blocks::BlockFacts),
    >,
    test: impl FnOnce(&mut sunrise_gameplay::game::ServerGame<'_>),
) {
    with_game_role_and_facts_with(
        root,
        role,
        configure,
        configure.is_none(),
        prepare_game_boot_with_test_platform,
        test,
    );
}

#[cfg(all(test, feature = "client"))]
pub(crate) fn with_game_role_and_facts_client(
    root: std::path::PathBuf,
    role: sunrise_gameplay_common::game::execution_role::ExecutionRole,
    configure: Option<
        fn(&sunrise_world::registry::BlockRegistry, &mut sunrise_items::blocks::BlockFacts),
    >,
    test: impl FnOnce(&mut sunrise_client_gameplay::game::ClientGame<'_>),
) {
    with_game_role_and_facts_with_client(root, role, configure, configure.is_none(), test);
}

#[cfg(feature = "client")]
fn with_game_role_and_facts_with(
    root: std::path::PathBuf,
    role: sunrise_gameplay_common::game::execution_role::ExecutionRole,
    configure: Option<
        fn(&sunrise_world::registry::BlockRegistry, &mut sunrise_items::blocks::BlockFacts),
    >,
    cached: bool,
    prepare_game_boot: fn(&std::path::Path),
    test: impl FnOnce(&mut sunrise_gameplay::game::ServerGame<'_>),
) {
    use crate::settings::Settings;
    use mod_api::Authority;
    use sunrise_gameplay::game::native_entity_refs::tests::{
        load_fixture_server_resources, load_fixture_server_resources_uncached,
    };
    use sunrise_gameplay_common::game::Options;

    let _globals = ScopedFixtureGlobals::new();
    prepare_game_boot(&root);
    let saves = tempfile::tempdir().unwrap();
    let settings = Settings::default();
    let load = if cached {
        load_fixture_server_resources
    } else {
        load_fixture_server_resources_uncached
    };
    let (mut resources, pack) = load(&root, role);
    if let Some(configure) = configure {
        configure(pack.assets.registry, fixture_facts_mut(&mut resources));
    }
    let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
    options.resources = root.clone();
    options.execution_role = role;
    options.game_dir = Some(saves.path().to_path_buf());
    let mut game = sunrise_gameplay::game::ServerGame::new(
        &resources,
        pack,
        options,
        settings.sim_settings(),
        None,
        crate::game::profile_resolver,
    )
    .unwrap();
    game.reset_entity_residency();
    game.native_lifecycle
        .begin_open(Some(Authority::IntegratedServer));

    // A game that opened its world resolves its parts; the harness stops
    // short of the open, so do it here for the by-state part table.
    resolve_parts(&game).unwrap();
    game.install_root_block_transformers();
    // The title's save folder is under the working directory: give each test its own, so a
    // test that saves leaves the checkout clean and parallel tests don't share files.
    game.server.save.0 = std::sync::Arc::new(sunrise_sim::save::SaveDir::new(
        saves.path().to_path_buf(),
        sunrise_sim::save::LevelShape::OVERWORLD,
        Some(Default::default()),
    ));
    test(&mut game);
}

#[cfg(all(test, feature = "client"))]
fn with_game_role_and_facts_with_client(
    root: std::path::PathBuf,
    role: sunrise_gameplay_common::game::execution_role::ExecutionRole,
    configure: Option<
        fn(&sunrise_world::registry::BlockRegistry, &mut sunrise_items::blocks::BlockFacts),
    >,
    cached: bool,
    test: impl FnOnce(&mut sunrise_client_gameplay::game::ClientGame<'_>),
) {
    use crate::audio::Audio;
    use crate::game::{load_resources, load_resources_uncached};
    use crate::settings::Settings;
    use mod_api::Authority;
    use sunrise_gameplay_common::game::execution_role::ExecutionRole;
    use sunrise_gameplay_common::game::{Options, Progress};

    let _globals = ScopedFixtureGlobals::new();
    assert_eq!(role, ExecutionRole::Client);
    sunrise_gameplay_common::game::test_fixtures::prepare_game_boot(
        &root,
        mod_platform::__private::part_tests::register_game_keys,
    );
    let saves = tempfile::tempdir().unwrap();
    let settings = Settings::default();
    let mut loaded = if cached {
        load_resources(
            sunrise_assets::resource_source::ResourceRoot::directory(root.clone()),
            Vec::new(),
            None,
            &root,
            &settings,
            std::time::Instant::now(),
            false,
            role,
            &Progress::new(),
            &mut |_, _| unreachable!(),
        )
        .unwrap()
    } else {
        load_resources_uncached(
            sunrise_assets::resource_source::ResourceRoot::directory(root.clone()),
            Vec::new(),
            None,
            &root,
            &settings,
            std::time::Instant::now(),
            false,
            role,
            &Progress::new(),
            &mut |_, _| unreachable!(),
        )
        .unwrap()
    };
    if let Some(configure) = configure {
        configure(
            loaded.pack.assets.registry,
            fixture_facts_mut(&mut loaded.resources),
        );
    }
    let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
    options.resources = root.clone();
    options.execution_role = role;
    options.game_dir = Some(saves.path().to_path_buf());
    let mut game = sunrise_client_gameplay::game::ClientGame::new(
        &loaded.resources,
        loaded.client_res.unwrap(),
        loaded.pack,
        loaded.client_pack.unwrap(),
        loaded.gui.unwrap(),
        options,
        settings,
        Audio::silent(&sunrise_assets::resource_source::ResourceSource::directory(
            root.to_path_buf(),
        )),
        None,
        sunrise_gameplay::game::server_side::integrated::serve,
        sunrise_gameplay::game::server_side::integrated::run_component_harness_on_server,
        crate::game::session_verifier(),
        crate::game::profile_resolver,
    )
    .unwrap();

    game.native_lifecycle
        .begin_open(Some(Authority::IntegratedServer));

    // A game that opened its world resolves its parts; the harness stops
    // short of the open, so do it here for the by-state part table.
    resolve_parts(&game).unwrap();
    // The title's save folder is under the working directory: give each test its own, so a
    // test that saves leaves the checkout clean and parallel tests don't share files.

    test(&mut game);
}

#[cfg(all(test, feature = "client"))]
#[test]
fn fixture_globals_restore_the_previous_palette_and_namespaces() {
    let palette = (
        sunrise_level::assets::block_palette_bits(),
        sunrise_level::assets::block_states_count(),
        sunrise_level::assets::block_states_generation(),
    );
    let namespaces = sunrise_sim::save::level_mods::block_namespaces();

    {
        let _globals = ScopedFixtureGlobals::new();
        sunrise_level::chunk::set_block_states(131_072);
        sunrise_sim::save::level_mods::set_block_namespaces(Some(vec!["fixture_test".into()]));
        assert_eq!(sunrise_level::assets::block_states_count(), 131_072);
        assert_eq!(
            sunrise_sim::save::level_mods::block_namespaces(),
            Some(vec!["fixture_test".into()])
        );
    }

    assert_eq!(
        (
            sunrise_level::assets::block_palette_bits(),
            sunrise_level::assets::block_states_count(),
            sunrise_level::assets::block_states_generation(),
        ),
        palette
    );
    assert_eq!(
        sunrise_sim::save::level_mods::block_namespaces(),
        namespaces
    );
}
