//! What a pack reads with the device a renderer has (#6921): the Shader Options page, on the
//! title screen and in game, reads the pick with the renderer's macros where a renderer exists
//! (`shader_options_macros`), so its device's `IRIS_FEATURE_PER_BUFFER_BLENDING` answer reaches
//! the pack's `screen` entries; and once a renderer on another device is installed
//! (`ClientGame::set_renderer`, `native_shaders::follow_device`), a pack refused before is tried
//! again. Original fixtures, written here.
use super::native_entity_refs::tests::{run_fresh_cases, test_window, with_game_role_client};
use super::native_shaders::{fingerprint, refused, remember};
use super::shader_packs::{read_shader_options, shader_options_macros};
use sunrise_client_render::graphics::IrisCompat;
use sunrise_gameplay_common::game::execution_role::ExecutionRole;
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

/// The sliders the Shader Options page shows for the pack at `path`, read with `macros`.
fn sliders(path: &std::path::Path, macros: &Macros) -> Vec<String> {
    let options = read_shader_options(&path.to_string_lossy(), "en_us", macros, IrisCompat::Auto)
        .expect("the pack reads");
    options.screens.sliders.iter().cloned().collect()
}

/// Without a renderer the page reads with the standard macros alone, as before: no device
/// answer, so what only `IRIS_FEATURE_PER_BUFFER_BLENDING` reaches is not shown. The page shows
/// it from a macro set whose device draws per-buffer blends.
#[test]
fn the_options_page_reads_the_device_answer_it_is_given() {
    let root = tempfile::tempdir().unwrap();
    let pack = guarded_pack(root.path());
    assert_eq!(shader_options_macros(None), Macros::standard());
    assert!(sliders(&pack, &shader_options_macros(None)).is_empty());
    let mut able = Macros::standard();
    able.set_per_buffer_blending(true);
    assert_eq!(sliders(&pack, &able), ["LEVEL"]);
}

/// Real renderers: the page opened in game shows the guarded slider exactly when the device
/// answers the flag, as the load reads it. A renderer made again on the same device
/// (`RendererDevice::Reuse`) keeps a refusal. One on a newly opened device, as a device recovery
/// opens it, tries the refusal again although it answers every pack macro the same.
#[test]
#[ignore = "daily"]
fn the_renderers_device_reaches_the_options_page_and_the_refusals() {
    if run_fresh_cases(
        "game::client_side::shader_options_device_tests::the_renderers_device_reaches_the_options_page_and_the_refusals",
        "SUNRISE_SHADER_OPTIONS_DEVICE_CASE",
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
    with_game_role_client(root, ExecutionRole::Client, |game| {
        let scratch = tempfile::tempdir().unwrap();
        let pack = guarded_pack(scratch.path());
        game.common.options.offscreen = true;
        let (_event_loop, window) = test_window([320, 240]);
        let first = game.new_renderer(window.clone()).unwrap();
        // The device's answer, read as the load reads it.
        let pack_macros = |renderer: &sunrise_client::gfx::Renderer| {
            let mut macros = Macros::standard();
            renderer.define_pack_macros(&mut macros);
            macros
        };
        let first_macros = pack_macros(&first);
        let mut load = first_macros.clone();
        let (_, notes) = sunrise_shaders::read_properties(
            &sunrise_shaders::Pack::open(&pack).unwrap(),
            &mut load,
        );
        let able = load.is_defined("IRIS_FEATURE_PER_BUFFER_BLENDING");
        eprintln!("SHADER_OPTIONS_DEVICE per-buffer blending {able}, notes {notes:?}");
        let first_device = first.device_id();
        game.set_renderer(first);
        // The page, opened with the installed renderer's macros.
        let shown: &[&str] = if able { &["LEVEL"] } else { &[] };
        let macros = shader_options_macros(game.client.renderer.0.as_deref());
        assert_eq!(sliders(&pack, &macros), shown, "per-buffer blending {able}");
        game.client.settings.sunrise.shader_pack = pack.to_string_lossy().into_owned();
        game.open_shader_options();
        let page = game.client.settings.shader_options.as_ref().unwrap();
        assert_eq!(page.screens.sliders.iter().collect::<Vec<_>>(), shown);
        // A refusal on this device.
        let stamp = fingerprint(&pack);
        remember(&pack, stamp, IrisCompat::Auto, false);
        // Made again on the same device.
        let again = (game.renderer_for(
            window.clone(),
            &game.common.pack,
            &game.client.client_pack,
            &game.client.gui,
            None,
        ))
        .unwrap();
        assert_eq!(again.device_id(), first_device);
        game.set_renderer(again);
        assert!(refused(&pack, stamp, IrisCompat::Auto));
        // A newly opened device, as `recover_device` opens it: the old one goes first.
        drop(game.client.renderer.0.take());
        game.client.mod_gpu = None;
        let opened = pollster::block_on(sunrise_client::gfx::Opened::for_window(
            &window,
            true,
            game.client.settings.sunrise.api,
            game.client.client_pack.render.texture_layers,
        ))
        .unwrap();
        let other = (game.renderer_for(
            window.clone(),
            &game.common.pack,
            &game.client.client_pack,
            &game.client.gui,
            Some(opened),
        ))
        .unwrap();
        assert_ne!(other.device_id(), first_device);
        assert_eq!(pack_macros(&other), first_macros, "the same answers");
        game.set_renderer(other);
        assert!(!refused(&pack, stamp, IrisCompat::Auto));
    });
}
