//! A selected pack whose cloud texture vanilla 26.3 reads past (`CloudRenderer.prepare` throws
//! `(5, 2) outside of image bounds (4, 6)`): the whole load fails as vanilla's does
//! (`Minecraft.rollbackResourcePacks`, `clearResourcePacksOnError`), on a reload and at startup,
//! and the pack's other overrides go with it. Each case runs in a process of its own, in a
//! folder whose `resourcepacks` holds the packs.
use crate::game::model_baking_completed_tests::reload;
use crate::game::native_entity_refs::tests::{run_fresh_cases, with_client_game};

const NAME: &str =
    "game::packs::clouds_reload_tests::a_pack_whose_clouds_vanilla_reads_past_fails_the_load";
const CASE: &str = "SUNRISE_CLOUDS_RELOAD_CASE";

/// A `w` by `h` RGBA image with the cells at `filled` opaque.
fn png(w: u32, h: u32, filled: &[[u32; 2]]) -> Vec<u8> {
    let mut rgba = vec![0u8; (w * h * 4) as usize];
    for [x, y] in filled {
        rgba[((y * w + x) * 4 + 3) as usize] = 255;
    }
    let mut out = Vec::new();
    let mut encoder = png::Encoder::new(&mut out, w, h);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().unwrap();
    writer.write_image_data(&rgba).unwrap();
    writer.finish().unwrap();
    out
}

/// `resourcepacks/<name>`: a pack with its own `core/sky` (a visible override, which the atlas
/// loads) and a 4 by 6 cloud texture with `cells` filled.
fn write_pack(name: &str, cells: &[[u32; 2]]) {
    let dir = std::env::current_dir()
        .unwrap()
        .join("resourcepacks")
        .join(name);
    let file = |path: &str, bytes: &[u8]| {
        let path = dir.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, bytes).unwrap();
    };
    file(
        "pack.mcmeta",
        br#"{"pack": {"description": "clouds", "pack_format": 97, "min_format": [97, 0], "max_format": [97, 1]}}"#,
    );
    file(
        "assets/minecraft/shaders/core/sky.vsh",
        b"#version 330\nlayout(location = 0) in vec3 Position;\n\
          void main() {\n    gl_Position = vec4(Position, 1.0);\n}\n",
    );
    file(
        "assets/minecraft/shaders/core/sky.fsh",
        b"#version 330\nlayout(location = 0) out vec4 fragColor;\n\
          void main() {\n    fragColor = vec4(1.0, 0.0, 0.0, 1.0);\n}\n",
    );
    file(
        "assets/minecraft/textures/environment/clouds.png",
        &png(4, 6, cells),
    );
}

/// A tall texture vanilla reads inside (`tall_inner` of the Java comparison) and one with a cell
/// in its first column, whose west neighbour vanilla reads at (5, 2).
const INNER: &[[u32; 2]] = &[[1, 0], [2, 1], [1, 3], [2, 5]];
const PAST: &[[u32; 2]] = &[[0, 2], [2, 1]];

#[test]
#[ignore = "daily"]
fn a_pack_whose_clouds_vanilla_reads_past_fails_the_load() {
    if run_fresh_cases(NAME, CASE, &["reload", "startup"]) {
        return;
    }
    write_pack("inner", INNER);
    write_pack("past", PAST);
    match std::env::var(CASE).unwrap().as_str() {
        "reload" => a_reload_fails_and_drops_the_selection(),
        _ => a_startup_load_fails_back_to_the_games_resources(),
    }
}

/// The accepted pack loads, its sky shader and clouds in use. Selecting the refused one fails the
/// reload: the selection is cleared and saved, the game reloads its own resources (no pack sky,
/// vanilla's clouds), and the load-failure toast shows, as vanilla's reload does.
fn a_reload_fails_and_drops_the_selection() {
    with_client_game(|game| {
        let vanilla_clouds = game.pack.clouds.clone();
        game.settings.resource_packs = vec!["file/inner".into()];
        reload(game);
        assert_eq!(game.settings.resource_packs, ["file/inner"]);
        assert!(game.gui.atlas.sky_shader.is_some());
        assert_eq!((game.pack.clouds.0, game.pack.clouds.1), (4, 6));
        game.settings.resource_packs = vec!["file/inner".into(), "file/past".into()];
        reload(game);
        assert!(game.settings.resource_packs.is_empty());
        assert!(
            game.gui.atlas.sky_shader.is_none(),
            "the pack's sky went too"
        );
        assert_eq!(game.pack.clouds, vanilla_clouds);
        assert!(
            game.server_packs.load_fail.is_some(),
            "resourcePack.load_fail"
        );
    });
}

/// The refused pack selected when the game starts: the load fails over to the game's own
/// resources, with the error the pack screen shows (`clearResourcePacksOnError` then clears the
/// saved selection, `loading_window`).
fn a_startup_load_fails_back_to_the_games_resources() {
    use crate::game::*;
    let root = std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
    let selected = ["file/past".to_owned()];
    let resolved = super::startup_root(&root, &selected, &[], &Default::default()).unwrap();
    assert!(sunrise_client::clouds::load(resolved.source()).is_err());
    let loaded = load_resources(
        resolved,
        Vec::new(),
        None,
        &root,
        &Settings::default(),
        std::time::Instant::now(),
        false,
        execution_role::ExecutionRole::Client,
        &Progress::new(),
        &mut |_, _| unreachable!(),
    )
    .unwrap();
    assert_eq!(
        loaded.pack_error.as_deref(),
        Some(
            "assets/minecraft/textures/environment/clouds.png: (5, 2) outside of image bounds (4, 6)"
        )
    );
    assert!(loaded.gui.unwrap().atlas.sky_shader.is_none());
    let vanilla = sunrise_assets::resource_source::ResourceSource::directory(root);
    assert_eq!(
        *loaded.pack.clouds,
        sunrise_client::clouds::load(&vanilla).unwrap()
    );
}
