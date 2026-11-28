//! The title screen's Shader Options page reads the pick with the startup renderer's macros
//! (#6921): `Event::OpenShaderOptions` on the startup menu (`Loading::menu_option_events`) gives
//! the pack the same device answer, `IRIS_FEATURE_PER_BUFFER_BLENDING` among them, as the
//! load reads it with. Original fixture, written here.
use super::startup_menu::tests::loading;
use sunrise_client::options_ui::Event;
use sunrise_shaders::Macros;

/// A pack whose `sliders` screen entry only the per-buffer blending answer reaches.
fn guarded_pack(root: &std::path::Path) -> std::path::PathBuf {
    let pack = root.join("guarded");
    let shaders = pack.join("shaders");
    std::fs::create_dir_all(&shaders).unwrap();
    std::fs::write(
        shaders.join("shaders.properties"),
        "iris.features.optional = PER_BUFFER_BLENDING\n\
         #ifdef IRIS_FEATURE_PER_BUFFER_BLENDING\nsliders = LEVEL\n#endif\n",
    )
    .unwrap();
    std::fs::write(shaders.join("options.glsl"), "#define LEVEL 2 // [1 2]\n").unwrap();
    pack
}

/// The sliders the startup menu's Shader Options page shows once it is opened.
fn opened_sliders(
    loading: &mut sunrise_client_gameplay::game::client_side::loading_screen::Loading,
) -> Vec<String> {
    loading.settings.shader_options = None;
    loading.menu_option_events(vec![Event::OpenShaderOptions]);
    let options = (loading.settings.shader_options.as_ref()).expect("the pick reads");
    options.screens.sliders.iter().cloned().collect()
}

/// With the startup renderer, the page shows the guarded slider exactly when its device
/// answers the flag, as the load's read does; before a renderer exists, it reads with the
/// standard set alone and does not show it.
#[test]
#[ignore = "daily"]
fn the_title_screens_options_page_reads_the_startup_renderers_device() {
    if super::native_entity_refs::tests::run_fresh_cases(
        "cut_layout::game::client_side::startup_shader_options_tests::the_title_screens_options_page_reads_the_startup_renderers_device",
        "SUNRISE_STARTUP_SHADER_OPTIONS_CASE",
        &["device"],
    ) {
        return;
    }
    let _gpu = sunrise_client_render::gfx::lock_test_gpu();
    // The prepared data the client game boots from, checked before the boot reads it.
    let root = crate::prepared_test_root::prepared_root(&[
        sunrise_core::prepared_test_support::STREAMING_INPUTS,
        sunrise_core::prepared_test_support::SERVER_INPUTS,
        &["entity-models.json", "gameplay-facts.json"],
    ]);
    let role = sunrise_gameplay_common::game::execution_role::ExecutionRole::Client;
    super::native_entity_refs::tests::with_game_role_client(root, role, |game| {
        let root = tempfile::tempdir().unwrap();
        let pack = guarded_pack(root.path());
        game.options.offscreen = true;
        let (_event_loop, window) =
            sunrise_client_gameplay::game::native_entity_refs::tests::test_window([320, 240]);
        let renderer = game.new_renderer(window).unwrap();
        // The device's answer, read as the load reads it.
        let mut load = Macros::standard();
        renderer.define_pack_macros(&mut load);
        let (_, notes) = sunrise_shaders::read_properties(
            &sunrise_shaders::Pack::open(&pack).unwrap(),
            &mut load,
        );
        let able = load.is_defined("IRIS_FEATURE_PER_BUFFER_BLENDING");
        eprintln!("STARTUP_SHADER_OPTIONS per-buffer blending {able}, notes {notes:?}");
        let (_send, receiver) = std::sync::mpsc::channel();
        let mut loading = loading(receiver);
        loading.settings.sunrise.shader_pack = pack.to_string_lossy().into_owned();
        assert!(opened_sliders(&mut loading).is_empty(), "no renderer yet");
        loading.renderer = Some(*renderer);
        let shown: &[&str] = if able { &["LEVEL"] } else { &[] };
        assert_eq!(
            opened_sliders(&mut loading),
            shown,
            "per-buffer blending {able}"
        );
    });
}
