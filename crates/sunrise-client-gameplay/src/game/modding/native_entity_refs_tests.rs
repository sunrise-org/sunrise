use sunrise_gameplay_common::game::execution_role::ExecutionRole;
pub use sunrise_gameplay_common::game::test_fixtures::{
    FRESH_CASE_LIMIT, fixture_block_names, fixture_root, output_within, ran_one_test, record_key,
    resolve_parts, run_fresh_cases, run_fresh_cases_at_once, run_with_game_stack,
};
use sunrise_gameplay_common::game::test_fixtures::{fixture_facts_mut, offline_profiles, tempfile};

/// Create a hidden desktop window during winit's initial `Resumed` callback.
/// Keep the returned event loop alive while the fixture uses the window.
pub fn test_window(
    size: [u32; 2],
) -> (
    winit::event_loop::EventLoop<()>,
    std::sync::Arc<winit::window::Window>,
) {
    use winit::application::ApplicationHandler;
    use winit::dpi::PhysicalSize;
    use winit::event::WindowEvent;
    use winit::event_loop::{ActiveEventLoop, EventLoop};
    use winit::platform::pump_events::EventLoopExtPumpEvents;
    use winit::window::{Window, WindowId};

    struct HiddenWindow {
        size: [u32; 2],
        window: Option<Window>,
    }

    impl ApplicationHandler for HiddenWindow {
        fn resumed(&mut self, event_loop: &ActiveEventLoop) {
            let [width, height] = self.size;
            self.window = Some(
                event_loop
                    .create_window(
                        Window::default_attributes()
                            .with_visible(false)
                            .with_inner_size(PhysicalSize::new(width, height)),
                    )
                    .unwrap(),
            );
        }

        fn window_event(&mut self, _: &ActiveEventLoop, _: WindowId, _: WindowEvent) {}
    }

    let mut builder = EventLoop::builder();
    #[cfg(target_os = "linux")]
    {
        use winit::platform::x11::EventLoopBuilderExtX11;
        builder.with_x11().with_any_thread(true);
    }
    #[cfg(target_os = "windows")]
    {
        use winit::platform::windows::EventLoopBuilderExtWindows;
        builder.with_any_thread(true);
    }
    let mut event_loop = builder.build().unwrap();
    let mut app = HiddenWindow { size, window: None };
    event_loop.pump_app_events(Some(std::time::Duration::ZERO), &mut app);
    let window = app
        .window
        .expect("winit did not resume to create the fixture window");
    (event_loop, std::sync::Arc::new(window))
}

pub fn with_client_game(test: impl FnOnce(&mut crate::game::ClientGame<'_>)) {
    with_game_role_client(fixture_root(), ExecutionRole::Client, test);
}

pub fn with_uncached_client_game(test: impl FnOnce(&mut crate::game::ClientGame<'_>)) {
    with_fixture_game(fixture_root(), ExecutionRole::Client, false, None, test);
}

pub fn with_game_role_client(
    root: std::path::PathBuf,
    role: ExecutionRole,
    test: impl FnOnce(&mut crate::game::ClientGame<'_>),
) {
    with_game_role_and_facts_client(root, role, None, test);
}

pub fn with_game_role_and_facts_client(
    root: std::path::PathBuf,
    role: ExecutionRole,
    configure: Option<
        fn(&sunrise_world::registry::BlockRegistry, &mut sunrise_items::blocks::BlockFacts),
    >,
    test: impl FnOnce(&mut crate::game::ClientGame<'_>),
) {
    with_fixture_game(root, role, configure.is_none(), configure, test);
}

fn with_fixture_game(
    root: std::path::PathBuf,
    role: ExecutionRole,
    cached: bool,
    configure: Option<
        fn(&sunrise_world::registry::BlockRegistry, &mut sunrise_items::blocks::BlockFacts),
    >,
    test: impl FnOnce(&mut crate::game::ClientGame<'_>),
) {
    use crate::game::client_side::test_resource_cache;
    use crate::game::{Audio, Settings};
    use sunrise_gameplay_common::game::{Options, resource_load::Progress};

    #[cfg(test)]
    mod_platform::__private::test_platform::install();
    sunrise_gameplay_common::game::test_fixtures::prepare_game_boot(
        &root,
        mod_api::testing::engine_platform::register_game_keys,
    );
    let saves = tempfile::tempdir().unwrap();
    let settings = Settings::default();
    let load = if cached {
        test_resource_cache::load
    } else {
        test_resource_cache::load_uncached
    };
    let mut loaded = load(
        test_resource_cache::LoadArgs {
            roots: sunrise_assets::resource_source::ResourceRoot::directory(root.clone()).into(),
            additional_data: Vec::new(),
            launch_world: None,
            vanilla_root: &root,
            settings: &settings,
            started: std::time::Instant::now(),
            menu: false,
            skip_lang: false,
            role: ExecutionRole::Client.into(),
            progress: &Progress::new(),
        },
        &mut |_, _| unreachable!(),
    )
    .unwrap();
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
    let mut game = crate::game::ClientGame::new(
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
        |_, _, _, _, _, _, _, _, _| Err("integrated server fixture requires the shell".into()),
        |_, _, _, _, _, _| Err("component fixture requires the shell".into()),
        std::sync::Arc::new(|_, _, _| Err(sunrise_sim::lan::SessionVerifyError::Unavailable)),
        offline_profiles,
    )
    .unwrap();

    game.native_lifecycle.begin_open(None);

    // A game that opened its world resolves its parts; the harness stops
    // short of the open, so do it here for the by-state part table.
    resolve_parts(&game).unwrap();
    test(&mut game);
}
