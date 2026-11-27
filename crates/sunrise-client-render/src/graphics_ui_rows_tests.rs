use super::tests::{CHANGED, CLICK, FB, NONE, press, row, ui};
use super::*;

struct TestDescriptorsGuard(Option<&'static [&'static RowDescriptor]>);

impl Drop for TestDescriptorsGuard {
    fn drop(&mut self) {
        TEST_DESCRIPTORS.set(self.0);
    }
}

fn with_test_descriptors(rows: &'static [&'static RowDescriptor]) -> TestDescriptorsGuard {
    let prev = TEST_DESCRIPTORS.replace(Some(rows));
    TestDescriptorsGuard(prev)
}

fn always_shown(_: &Settings) -> bool {
    true
}

fn never_shown(_: &Settings) -> bool {
    false
}

fn get_fog(s: &Settings) -> bool {
    s.sunrise.fog
}

fn set_fog(s: &mut Settings, v: bool) {
    s.sunrise.fog = v;
}

fn undo_fog(s: &mut Settings, b: &Settings) {
    s.sunrise.fog = b.sunrise.fog;
}

fn get_sky(s: &Settings) -> bool {
    s.sunrise.sky
}

fn set_sky(s: &mut Settings, v: bool) {
    s.sunrise.sky = v;
}

fn undo_sky(s: &mut Settings, b: &Settings) {
    s.sunrise.sky = b.sunrise.sky;
}

fn get_weather(s: &Settings) -> bool {
    s.sunrise.weather
}

fn set_weather(s: &mut Settings, v: bool) {
    s.sunrise.weather = v;
}

fn undo_weather(s: &mut Settings, b: &Settings) {
    s.sunrise.weather = b.sunrise.weather;
}

const fn dummy_switch(id: RowId, placement: Placement) -> RowDescriptor {
    RowDescriptor {
        id,
        placement,
        caption_key: id.0,
        caption_fallback: None,
        description_key: "test.desc",
        chosen_description: None,
        cost: Cost::None,
        control: RowControl::Switch {
            get: get_fog,
            set: set_fog,
        },
        shown: always_shown,
        blocked: None,
        affects_preset: false,
        undo: undo_fog,
    }
}

fn group_row_ids(pages: &[(Page, Vec<ResolvedGroup>)], page: Page, group: GroupId) -> Vec<RowId> {
    pages
        .iter()
        .find(|(p, _)| *p == page)
        .unwrap()
        .1
        .iter()
        .find(|g| g.id == group)
        .unwrap()
        .rows
        .iter()
        .map(|r| r.id)
        .collect()
}

#[test]
fn same_anchor_rows_keep_declared_order() {
    static FIRST: RowDescriptor = dummy_switch(
        RowId("test:first"),
        Placement {
            page: Page::Quality,
            group: GroupId::SKY_AND_WEATHER,
            after: vanilla_row_id(Opt::Clouds),
        },
    );
    static SECOND: RowDescriptor = dummy_switch(
        RowId("test:second"),
        Placement {
            page: Page::Quality,
            group: GroupId::SKY_AND_WEATHER,
            after: vanilla_row_id(Opt::Clouds),
        },
    );
    let (pages, issues) = resolve_rows_with_issues(false, &[&FIRST, &SECOND]);
    assert!(issues.is_empty(), "{issues:?}");
    let ids = group_row_ids(&pages, Page::Quality, GroupId::SKY_AND_WEATHER);
    assert_eq!(
        &ids[..4],
        [
            RowId("vanilla:clouds"),
            RowId("test:first"),
            RowId("test:second"),
            RowId("vanilla:cloud_range"),
        ]
    );
}

#[test]
fn chains_of_described_rows_resolve_regardless_of_declaration_order() {
    static CHAIN_C: RowDescriptor = dummy_switch(
        RowId("test:chain_c"),
        Placement {
            page: Page::Quality,
            group: GroupId::LIGHTING_AND_ENTITIES,
            after: Some(RowId("test:chain_b")),
        },
    );
    static CHAIN_B: RowDescriptor = dummy_switch(
        RowId("test:chain_b"),
        Placement {
            page: Page::Quality,
            group: GroupId::LIGHTING_AND_ENTITIES,
            after: Some(RowId("test:chain_a")),
        },
    );
    static CHAIN_A: RowDescriptor = dummy_switch(
        RowId("test:chain_a"),
        Placement {
            page: Page::Quality,
            group: GroupId::LIGHTING_AND_ENTITIES,
            after: vanilla_row_id(Opt::SmoothLighting),
        },
    );
    let (pages, issues) = resolve_rows_with_issues(false, &[&CHAIN_C, &CHAIN_B, &CHAIN_A]);
    assert!(issues.is_empty(), "{issues:?}");
    let ids = group_row_ids(&pages, Page::Quality, GroupId::LIGHTING_AND_ENTITIES);
    assert_eq!(
        &ids[..5],
        [
            RowId("vanilla:smooth_lighting"),
            RowId("test:chain_a"),
            RowId("test:chain_b"),
            RowId("test:chain_c"),
            RowId("vanilla:biome_blend"),
        ]
    );
}

#[test]
fn platform_filter_runs_before_anchor_resolution() {
    static AFTER_GPU_CULLING: RowDescriptor = dummy_switch(
        RowId("test:after_gpu_culling"),
        Placement {
            page: Page::Performance,
            group: GroupId::CULLING,
            after: Some(H::GpuCulling.row_id()),
        },
    );

    let (non_macos, non_macos_issues) = resolve_rows_with_issues(false, &[&AFTER_GPU_CULLING]);
    assert!(non_macos_issues.is_empty(), "{non_macos_issues:?}");
    let non_macos_ids = group_row_ids(&non_macos, Page::Performance, GroupId::CULLING);
    let gpu_pos = non_macos_ids
        .iter()
        .position(|&id| id == H::GpuCulling.row_id())
        .unwrap();
    assert_eq!(non_macos_ids[gpu_pos + 1], RowId("test:after_gpu_culling"));

    let (macos, macos_issues) = resolve_rows_with_issues(true, &[&AFTER_GPU_CULLING]);
    assert_eq!(
        macos_issues,
        [OrderIssue::MissingAnchor(
            RowId("test:after_gpu_culling"),
            H::GpuCulling.row_id()
        )]
    );
    let macos_ids = group_row_ids(&macos, Page::Performance, GroupId::CULLING);
    assert!(!macos_ids.contains(&H::GpuCulling.row_id()));
    assert_eq!(macos_ids.last(), Some(&RowId("test:after_gpu_culling")));
}

#[test]
fn duplicate_id_keeps_first_and_reports_issue() {
    static FIRST: RowDescriptor = dummy_switch(
        RowId("test:dup"),
        Placement {
            page: Page::Performance,
            group: GroupId::POWER,
            after: Some(H::BatteryMode.row_id()),
        },
    );
    static SECOND: RowDescriptor = dummy_switch(
        RowId("test:dup"),
        Placement {
            page: Page::General,
            group: GroupId::INTERFACE,
            after: None,
        },
    );
    static ENGINE_COLLISION: RowDescriptor = dummy_switch(
        RowId("vanilla:particles"),
        Placement {
            page: Page::Performance,
            group: GroupId::POWER,
            after: None,
        },
    );

    let (pages, issues) = resolve_rows_with_issues(false, &[&FIRST, &SECOND, &ENGINE_COLLISION]);
    assert_eq!(
        issues,
        [
            OrderIssue::DuplicateId(RowId("test:dup")),
            OrderIssue::DuplicateId(RowId("vanilla:particles")),
        ]
    );
    let power = group_row_ids(&pages, Page::Performance, GroupId::POWER);
    assert_eq!(power, [H::BatteryMode.row_id(), RowId("test:dup")]);
    let interface = group_row_ids(&pages, Page::General, GroupId::INTERFACE);
    assert!(!interface.contains(&RowId("test:dup")));
}

#[test]
fn anchor_cycle_places_rows_at_end_of_group_and_reports_issue() {
    static CYCLE_A: RowDescriptor = dummy_switch(
        RowId("test:cycle_a"),
        Placement {
            page: Page::Performance,
            group: GroupId::POWER,
            after: Some(RowId("test:cycle_b")),
        },
    );
    static CYCLE_B: RowDescriptor = dummy_switch(
        RowId("test:cycle_b"),
        Placement {
            page: Page::Performance,
            group: GroupId::POWER,
            after: Some(RowId("test:cycle_a")),
        },
    );
    static TAIL: RowDescriptor = dummy_switch(
        RowId("test:cycle_tail"),
        Placement {
            page: Page::Performance,
            group: GroupId::POWER,
            after: Some(RowId("test:cycle_a")),
        },
    );
    static SELF_CYCLE: RowDescriptor = dummy_switch(
        RowId("test:self_cycle"),
        Placement {
            page: Page::Performance,
            group: GroupId::POWER,
            after: Some(RowId("test:self_cycle")),
        },
    );

    let (pages, issues) =
        resolve_rows_with_issues(false, &[&TAIL, &CYCLE_A, &CYCLE_B, &SELF_CYCLE]);
    assert_eq!(
        issues,
        [
            OrderIssue::AnchorCycle(RowId("test:cycle_a")),
            OrderIssue::AnchorCycle(RowId("test:cycle_b")),
            OrderIssue::AnchorCycle(RowId("test:self_cycle")),
        ]
    );
    let power = group_row_ids(&pages, Page::Performance, GroupId::POWER);
    assert_eq!(
        power,
        [
            H::BatteryMode.row_id(),
            RowId("test:cycle_a"),
            RowId("test:cycle_tail"),
            RowId("test:cycle_b"),
            RowId("test:self_cycle"),
        ]
    );
}

#[test]
fn cross_group_and_missing_anchors_place_at_end_of_group_and_unknown_group_skips() {
    static CROSS_GROUP: RowDescriptor = dummy_switch(
        RowId("test:cross_group"),
        Placement {
            page: Page::Performance,
            group: GroupId::POWER,
            after: vanilla_row_id(Opt::Particles),
        },
    );
    static MISSING: RowDescriptor = dummy_switch(
        RowId("test:missing_anchor"),
        Placement {
            page: Page::Performance,
            group: GroupId::POWER,
            after: Some(RowId("test:nonexistent")),
        },
    );
    static UNKNOWN_GROUP: RowDescriptor = dummy_switch(
        RowId("test:unknown_group"),
        Placement {
            page: Page::Performance,
            group: GroupId("performance.unknown"),
            after: None,
        },
    );

    let (pages, issues) =
        resolve_rows_with_issues(false, &[&CROSS_GROUP, &MISSING, &UNKNOWN_GROUP]);
    assert_eq!(
        issues,
        [
            OrderIssue::UnknownGroup(
                RowId("test:unknown_group"),
                Page::Performance,
                GroupId("performance.unknown"),
            ),
            OrderIssue::CrossGroupAnchor(RowId("test:cross_group"), RowId("vanilla:particles")),
            OrderIssue::MissingAnchor(RowId("test:missing_anchor"), RowId("test:nonexistent")),
        ]
    );
    let power = group_row_ids(&pages, Page::Performance, GroupId::POWER);
    assert_eq!(
        power,
        [
            H::BatteryMode.row_id(),
            RowId("test:cross_group"),
            RowId("test:missing_anchor"),
        ]
    );
}

#[test]
fn order_issues_are_logged_only_once() {
    let issue = OrderIssue::MissingAnchor(
        RowId("test:log_once_row"),
        RowId("test:log_once_missing_target"),
    );
    assert!(record_order_issue_once(issue));
    assert!(!record_order_issue_once(issue));
}

#[test]
fn runtime_visibility_is_applied_after_anchor_order() {
    static HIDDEN: RowDescriptor = RowDescriptor {
        shown: never_shown,
        ..dummy_switch(
            RowId("test:hidden_anchor"),
            Placement {
                page: Page::Quality,
                group: GroupId::SKY_AND_WEATHER,
                after: vanilla_row_id(Opt::Clouds),
            },
        )
    };
    static AFTER_HIDDEN: RowDescriptor = dummy_switch(
        RowId("test:after_hidden"),
        Placement {
            page: Page::Quality,
            group: GroupId::SKY_AND_WEATHER,
            after: Some(RowId("test:hidden_anchor")),
        },
    );
    static ROWS: [&RowDescriptor; 2] = [&HIDDEN, &AFTER_HIDDEN];
    let _guard = with_test_descriptors(&ROWS);

    let s = Settings::default();
    let sheets = GraphicsScreen::default().sheets(&s);
    let quality = sheets.iter().find(|sh| sh.id == Page::Quality).unwrap();
    let sky = quality
        .groups
        .iter()
        .find(|g| g.name == "Sky and weather")
        .unwrap();
    let visible: Vec<Item> = sky.rows.iter().map(|r| r.id).collect();
    assert_eq!(
        &visible[..3],
        [
            Item::Vanilla(Opt::Clouds),
            Item::Described(RowId("test:after_hidden")),
            Item::Vanilla(Opt::CloudRange),
        ]
    );
    assert!(!visible.contains(&Item::Described(RowId("test:hidden_anchor"))));
}

#[test]
fn described_row_affects_preset_controls_whether_an_edit_marks_custom() {
    static AFFECTS: RowDescriptor = RowDescriptor {
        id: RowId("test:affects_preset"),
        placement: Placement {
            page: Page::Performance,
            group: GroupId::POWER,
            after: Some(H::BatteryMode.row_id()),
        },
        caption_key: "test.affects",
        caption_fallback: Some("Affects Preset"),
        description_key: "test.affects.desc",
        chosen_description: None,
        cost: Cost::None,
        control: RowControl::Switch {
            get: get_sky,
            set: set_sky,
        },
        shown: always_shown,
        blocked: None,
        affects_preset: true,
        undo: undo_sky,
    };
    static KEEPS: RowDescriptor = RowDescriptor {
        id: RowId("test:keeps_preset"),
        placement: Placement {
            page: Page::Performance,
            group: GroupId::POWER,
            after: Some(RowId("test:affects_preset")),
        },
        caption_key: "test.keeps",
        caption_fallback: Some("Keeps Preset"),
        description_key: "test.keeps.desc",
        chosen_description: None,
        cost: Cost::None,
        control: RowControl::Switch {
            get: get_weather,
            set: set_weather,
        },
        shown: always_shown,
        blocked: None,
        affects_preset: false,
        undo: undo_weather,
    };
    static ROWS: [&RowDescriptor; 2] = [&AFFECTS, &KEEPS];
    let _guard = with_test_descriptors(&ROWS);

    let mut s = Settings::default();
    let mut ui = ui();
    s.sunrise.preset = Preset::Balanced;
    let keeps = row(&mut ui, &s, Item::Described(RowId("test:keeps_preset")));
    assert_eq!(
        press(&mut ui, &mut s, (keeps[0] + 4.0, keeps[1] + 4.0)),
        [CLICK, CHANGED]
    );
    assert!(!s.sunrise.weather);
    assert_eq!(s.sunrise.preset, Preset::Balanced);

    let affects = row(&mut ui, &s, Item::Described(RowId("test:affects_preset")));
    assert_eq!(
        press(&mut ui, &mut s, (affects[0] + 4.0, affects[1] + 4.0)),
        [CLICK, CHANGED]
    );
    assert!(!s.sunrise.sky);
    assert_eq!(s.sunrise.preset, Preset::Custom);

    // An ordinary edit on a preset-controlled engine row also marks Custom.
    s.sunrise.preset = Preset::Quality;
    Item::Sunrise(H::Fog).cycle(&mut s, false, FB, &NONE);
    assert_eq!(s.sunrise.preset, Preset::Custom);
}

#[cfg(feature = "sunrise-extras")]
const EXPECTED_ROWS_WITH_EXTRAS: &[(Page, GroupId, RowId)] = &[
    (Page::General, GroupId::PRESET, RowId("graphics:preset")),
    (
        Page::General,
        GroupId::DISTANCE,
        RowId("vanilla:render_distance"),
    ),
    (
        Page::General,
        GroupId::DISTANCE,
        RowId("graphics:lod_distance"),
    ),
    (
        Page::General,
        GroupId::DISTANCE,
        RowId("graphics:far_detail"),
    ),
    (
        Page::General,
        GroupId::DISTANCE,
        RowId("graphics:blue_map_radius"),
    ),
    (
        Page::General,
        GroupId::DISTANCE,
        RowId("vanilla:simulation_distance"),
    ),
    (Page::General, GroupId::DISPLAY, RowId("vanilla:gui_scale")),
    (Page::General, GroupId::DISPLAY, RowId("vanilla:fullscreen")),
    (
        Page::General,
        GroupId::DISPLAY,
        RowId("vanilla:exclusive_fullscreen"),
    ),
    (
        Page::General,
        GroupId::DISPLAY,
        RowId("vanilla:exclusive_mode"),
    ),
    (Page::General, GroupId::DISPLAY, RowId("vanilla:vsync")),
    (Page::General, GroupId::DISPLAY, RowId("vanilla:max_fps")),
    (
        Page::General,
        GroupId::DISPLAY,
        RowId("vanilla:inactivity_fps"),
    ),
    (Page::General, GroupId::DISPLAY, RowId("vanilla:gamma")),
    (
        Page::General,
        GroupId::INTERFACE,
        RowId("vanilla:menu_blur"),
    ),
    (
        Page::General,
        GroupId::INTERFACE,
        RowId("vanilla:attack_indicator"),
    ),
    (Page::General, GroupId::INTERFACE, RowId("vanilla:autosave")),
    (
        Page::Quality,
        GroupId::SKY_AND_WEATHER,
        RowId("vanilla:clouds"),
    ),
    (
        Page::Quality,
        GroupId::SKY_AND_WEATHER,
        RowId("vanilla:cloud_range"),
    ),
    (
        Page::Quality,
        GroupId::SKY_AND_WEATHER,
        RowId("graphics:sky"),
    ),
    (
        Page::Quality,
        GroupId::SKY_AND_WEATHER,
        RowId("graphics:fog"),
    ),
    (
        Page::Quality,
        GroupId::SKY_AND_WEATHER,
        RowId("graphics:weather"),
    ),
    (
        Page::Quality,
        GroupId::SKY_AND_WEATHER,
        RowId("vanilla:weather_radius"),
    ),
    (
        Page::Quality,
        GroupId::SKY_AND_WEATHER,
        RowId("vanilla:particles"),
    ),
    (
        Page::Quality,
        GroupId::SKY_AND_WEATHER,
        RowId("sunrise:hide_block_particles"),
    ),
    (
        Page::Quality,
        GroupId::LIGHTING_AND_ENTITIES,
        RowId("vanilla:smooth_lighting"),
    ),
    (
        Page::Quality,
        GroupId::LIGHTING_AND_ENTITIES,
        RowId("vanilla:biome_blend"),
    ),
    (
        Page::Quality,
        GroupId::LIGHTING_AND_ENTITIES,
        RowId("vanilla:entity_distance"),
    ),
    (
        Page::Quality,
        GroupId::LIGHTING_AND_ENTITIES,
        RowId("vanilla:entity_shadows"),
    ),
    (
        Page::Quality,
        GroupId::LIGHTING_AND_ENTITIES,
        RowId("vanilla:vignette"),
    ),
    (
        Page::Quality,
        GroupId::LIGHTING_AND_ENTITIES,
        RowId("vanilla:chunk_fade"),
    ),
    (
        Page::Quality,
        GroupId::LIGHTING_AND_ENTITIES,
        RowId("sunrise:dynamic_lights"),
    ),
    (
        Page::Quality,
        GroupId::LIGHTING_AND_ENTITIES,
        RowId("sunrise:fullbright"),
    ),
    (Page::Quality, GroupId::TEXTURES, RowId("vanilla:mipmaps")),
    (
        Page::Quality,
        GroupId::TEXTURES,
        RowId("vanilla:texture_filtering"),
    ),
    (
        Page::Quality,
        GroupId::TEXTURES,
        RowId("vanilla:max_anisotropy"),
    ),
    (
        Page::Quality,
        GroupId::TEXTURES,
        RowId("vanilla:cutout_leaves"),
    ),
    (
        Page::Quality,
        GroupId::TEXTURES,
        RowId("vanilla:improved_transparency"),
    ),
    (
        Page::Quality,
        GroupId::TEXTURES,
        RowId("graphics:animated_textures"),
    ),
    (
        Page::Quality,
        GroupId::TEXTURES,
        RowId("graphics:dither_cutout_mips"),
    ),
    (
        Page::Quality,
        GroupId::IMAGE,
        RowId("graphics:resolution_scale"),
    ),
    (
        Page::Quality,
        GroupId::IMAGE,
        RowId("graphics:dynamic_resolution"),
    ),
    (Page::Quality, GroupId::IMAGE, RowId("graphics:target_fps")),
    (
        Page::Quality,
        GroupId::IMAGE,
        RowId("graphics:antialiasing"),
    ),
    (Page::Quality, GroupId::IMAGE, RowId("graphics:upscaling")),
    (
        Page::Quality,
        GroupId::IMAGE,
        RowId("graphics:upscaling_sharpness"),
    ),
    (
        Page::Quality,
        GroupId::IMAGE,
        RowId("graphics:screenshot_quality"),
    ),
    (
        Page::Quality,
        GroupId::IMAGE,
        RowId("graphics:frame_generation"),
    ),
    (
        Page::Performance,
        GroupId::CHUNK_LOADING,
        RowId("vanilla:chunk_builder"),
    ),
    (
        Page::Performance,
        GroupId::CHUNK_LOADING,
        RowId("graphics:chunk_updates"),
    ),
    (
        Page::Performance,
        GroupId::CHUNK_LOADING,
        RowId("graphics:upload_budget"),
    ),
    (
        Page::Performance,
        GroupId::CHUNK_LOADING,
        RowId("graphics:workers"),
    ),
    (
        Page::Performance,
        GroupId::CULLING,
        RowId("graphics:cave_culling"),
    ),
    #[cfg(not(target_os = "macos"))]
    (
        Page::Performance,
        GroupId::CULLING,
        RowId("graphics:gpu_culling"),
    ),
    (
        Page::Performance,
        GroupId::CULLING,
        RowId("graphics:entity_culling"),
    ),
    (
        Page::Performance,
        GroupId::CULLING,
        RowId("graphics:particle_culling"),
    ),
    (
        Page::Performance,
        GroupId::CULLING,
        RowId("graphics:entity_culling_precheck"),
    ),
    (
        Page::Performance,
        GroupId::CULLING,
        RowId("graphics:leaf_culling"),
    ),
    (
        Page::Performance,
        GroupId::CULLING,
        RowId("graphics:opaque_ice"),
    ),
    (
        Page::Performance,
        GroupId::BLOCK_ENTITIES,
        RowId("graphics:cache_shulker_boxes"),
    ),
    (
        Page::Performance,
        GroupId::BLOCK_ENTITIES,
        RowId("graphics:cache_signs"),
    ),
    (
        Page::Performance,
        GroupId::BLOCK_ENTITIES,
        RowId("graphics:cache_decorated_pots"),
    ),
    (
        Page::Performance,
        GroupId::BLOCK_ENTITIES,
        RowId("graphics:cache_bells"),
    ),
    (
        Page::Performance,
        GroupId::BLOCK_ENTITIES,
        RowId("graphics:cache_chests"),
    ),
    (
        Page::Performance,
        GroupId::BLOCK_ENTITIES,
        RowId("graphics:cache_banners"),
    ),
    (
        Page::Performance,
        GroupId::BLOCK_ENTITIES,
        RowId("graphics:cache_copper_golem_statues"),
    ),
    (
        Page::Performance,
        GroupId::POWER,
        RowId("graphics:battery_mode"),
    ),
    (
        Page::Shaders,
        GroupId::BUILT_IN_SHADERS,
        RowId("graphics:shadow_cache"),
    ),
    (
        Page::Advanced,
        GroupId::FRAMES,
        RowId("graphics:present_mode"),
    ),
    (Page::Advanced, GroupId::FRAMES, RowId("graphics:pacing")),
    #[cfg(not(target_os = "macos"))]
    (
        Page::Advanced,
        GroupId::FRAMES,
        RowId("graphics:graphics_api"),
    ),
    #[cfg(not(target_os = "macos"))]
    (
        Page::Advanced,
        GroupId::FRAMES,
        RowId("graphics:translucent_batching"),
    ),
    (Page::Hdr, GroupId::HDR_OUTPUT, RowId("graphics:hdr")),
    (
        Page::Hdr,
        GroupId::HDR_OUTPUT,
        RowId("graphics:hdr_smooth_gradients"),
    ),
    (
        Page::Hdr,
        GroupId::HDR_OUTPUT,
        RowId("graphics:hdr_calibrate"),
    ),
    (
        Page::Hdr,
        GroupId::HDR_BRIGHTNESS,
        RowId("graphics:hdr_black_level"),
    ),
    (
        Page::Hdr,
        GroupId::HDR_BRIGHTNESS,
        RowId("graphics:hdr_peak"),
    ),
    (
        Page::Hdr,
        GroupId::HDR_BRIGHTNESS,
        RowId("graphics:hdr_paper_white"),
    ),
    (
        Page::Hdr,
        GroupId::HDR_BRIGHTNESS,
        RowId("graphics:hdr_ui_brightness"),
    ),
    (
        Page::Hdr,
        GroupId::HDR_BRIGHTNESS,
        RowId("graphics:hdr_lighting"),
    ),
];

fn flatten_resolved(pages: Vec<(Page, Vec<ResolvedGroup>)>) -> Vec<(Page, GroupId, RowId)> {
    pages
        .into_iter()
        .flat_map(|(page, groups)| {
            groups.into_iter().flat_map(move |g| {
                let group_id = g.id;
                g.rows.into_iter().map(move |r| (page, group_id, r.id))
            })
        })
        .collect()
}

fn assert_combined_sunrise_graphics_order(expected: &[(Page, GroupId, RowId)]) {
    use crate::{
        extras::Item as ExtraItem,
        extras_ui::ExtraRow,
        settings_shell::Screen as _,
        shader_ui,
        sunrise_ui::{SunriseRow, SunriseScreen, Tab},
    };
    let mut s = Settings::default();
    s.sunrise.taa = true;
    let sheets = SunriseScreen::default().sheets(&s);
    let mut graphics_rows = Vec::new();
    for sheet in &sheets {
        let Tab::Graphics(page) = sheet.id else {
            continue;
        };
        if page == Page::Shaders {
            assert_eq!(
                sheet.groups.iter().map(|g| g.name).collect::<Vec<_>>(),
                [shader_ui::INSTALLED, shader_ui::BUILT_IN]
            );
            assert_eq!(
                sheet.groups[0]
                    .rows
                    .iter()
                    .map(|r| r.id)
                    .collect::<Vec<_>>(),
                [
                    SunriseRow::Pack(shader_ui::Item::Folder),
                    SunriseRow::ShaderOptions,
                ]
            );
            assert_eq!(
                sheet.groups[1]
                    .rows
                    .iter()
                    .map(|r| r.id)
                    .collect::<Vec<_>>(),
                [
                    SunriseRow::Graphics(Item::Sunrise(H::ShadowCache)),
                    SunriseRow::Extra(ExtraRow::Item(ExtraItem::ShaderPackKey)),
                ]
            );
        }
        for group in &sheet.groups {
            for r in &group.rows {
                if let SunriseRow::Graphics(item) = r.id {
                    graphics_rows.push((page, item.row_id().unwrap()));
                }
            }
        }
    }
    let expected_combined: Vec<(Page, RowId)> =
        expected.iter().map(|&(page, _, id)| (page, id)).collect();
    assert_eq!(graphics_rows, expected_combined);
}

const EXPECTED_ROWS_WITHOUT_EXTRAS: &[(Page, GroupId, RowId)] = &[
    (Page::General, GroupId::PRESET, RowId("graphics:preset")),
    (
        Page::General,
        GroupId::DISTANCE,
        RowId("vanilla:render_distance"),
    ),
    (
        Page::General,
        GroupId::DISTANCE,
        RowId("graphics:lod_distance"),
    ),
    (
        Page::General,
        GroupId::DISTANCE,
        RowId("graphics:far_detail"),
    ),
    (
        Page::General,
        GroupId::DISTANCE,
        RowId("graphics:blue_map_radius"),
    ),
    (
        Page::General,
        GroupId::DISTANCE,
        RowId("vanilla:simulation_distance"),
    ),
    (Page::General, GroupId::DISPLAY, RowId("vanilla:gui_scale")),
    (Page::General, GroupId::DISPLAY, RowId("vanilla:fullscreen")),
    (
        Page::General,
        GroupId::DISPLAY,
        RowId("vanilla:exclusive_fullscreen"),
    ),
    (
        Page::General,
        GroupId::DISPLAY,
        RowId("vanilla:exclusive_mode"),
    ),
    (Page::General, GroupId::DISPLAY, RowId("vanilla:vsync")),
    (Page::General, GroupId::DISPLAY, RowId("vanilla:max_fps")),
    (
        Page::General,
        GroupId::DISPLAY,
        RowId("vanilla:inactivity_fps"),
    ),
    (Page::General, GroupId::DISPLAY, RowId("vanilla:gamma")),
    (
        Page::General,
        GroupId::INTERFACE,
        RowId("vanilla:menu_blur"),
    ),
    (
        Page::General,
        GroupId::INTERFACE,
        RowId("vanilla:attack_indicator"),
    ),
    (Page::General, GroupId::INTERFACE, RowId("vanilla:autosave")),
    (
        Page::Quality,
        GroupId::SKY_AND_WEATHER,
        RowId("vanilla:clouds"),
    ),
    (
        Page::Quality,
        GroupId::SKY_AND_WEATHER,
        RowId("vanilla:cloud_range"),
    ),
    (
        Page::Quality,
        GroupId::SKY_AND_WEATHER,
        RowId("graphics:sky"),
    ),
    (
        Page::Quality,
        GroupId::SKY_AND_WEATHER,
        RowId("graphics:fog"),
    ),
    (
        Page::Quality,
        GroupId::SKY_AND_WEATHER,
        RowId("graphics:weather"),
    ),
    (
        Page::Quality,
        GroupId::SKY_AND_WEATHER,
        RowId("vanilla:weather_radius"),
    ),
    (
        Page::Quality,
        GroupId::SKY_AND_WEATHER,
        RowId("vanilla:particles"),
    ),
    (
        Page::Quality,
        GroupId::LIGHTING_AND_ENTITIES,
        RowId("vanilla:smooth_lighting"),
    ),
    (
        Page::Quality,
        GroupId::LIGHTING_AND_ENTITIES,
        RowId("vanilla:biome_blend"),
    ),
    (
        Page::Quality,
        GroupId::LIGHTING_AND_ENTITIES,
        RowId("vanilla:entity_distance"),
    ),
    (
        Page::Quality,
        GroupId::LIGHTING_AND_ENTITIES,
        RowId("vanilla:entity_shadows"),
    ),
    (
        Page::Quality,
        GroupId::LIGHTING_AND_ENTITIES,
        RowId("vanilla:vignette"),
    ),
    (
        Page::Quality,
        GroupId::LIGHTING_AND_ENTITIES,
        RowId("vanilla:chunk_fade"),
    ),
    (Page::Quality, GroupId::TEXTURES, RowId("vanilla:mipmaps")),
    (
        Page::Quality,
        GroupId::TEXTURES,
        RowId("vanilla:texture_filtering"),
    ),
    (
        Page::Quality,
        GroupId::TEXTURES,
        RowId("vanilla:max_anisotropy"),
    ),
    (
        Page::Quality,
        GroupId::TEXTURES,
        RowId("vanilla:cutout_leaves"),
    ),
    (
        Page::Quality,
        GroupId::TEXTURES,
        RowId("vanilla:improved_transparency"),
    ),
    (
        Page::Quality,
        GroupId::TEXTURES,
        RowId("graphics:animated_textures"),
    ),
    (
        Page::Quality,
        GroupId::TEXTURES,
        RowId("graphics:dither_cutout_mips"),
    ),
    (
        Page::Quality,
        GroupId::IMAGE,
        RowId("graphics:resolution_scale"),
    ),
    (
        Page::Quality,
        GroupId::IMAGE,
        RowId("graphics:dynamic_resolution"),
    ),
    (Page::Quality, GroupId::IMAGE, RowId("graphics:target_fps")),
    (
        Page::Quality,
        GroupId::IMAGE,
        RowId("graphics:antialiasing"),
    ),
    (Page::Quality, GroupId::IMAGE, RowId("graphics:upscaling")),
    (
        Page::Quality,
        GroupId::IMAGE,
        RowId("graphics:upscaling_sharpness"),
    ),
    (
        Page::Quality,
        GroupId::IMAGE,
        RowId("graphics:screenshot_quality"),
    ),
    (
        Page::Quality,
        GroupId::IMAGE,
        RowId("graphics:frame_generation"),
    ),
    (
        Page::Performance,
        GroupId::CHUNK_LOADING,
        RowId("vanilla:chunk_builder"),
    ),
    (
        Page::Performance,
        GroupId::CHUNK_LOADING,
        RowId("graphics:chunk_updates"),
    ),
    (
        Page::Performance,
        GroupId::CHUNK_LOADING,
        RowId("graphics:upload_budget"),
    ),
    (
        Page::Performance,
        GroupId::CHUNK_LOADING,
        RowId("graphics:workers"),
    ),
    (
        Page::Performance,
        GroupId::CULLING,
        RowId("graphics:cave_culling"),
    ),
    #[cfg(not(target_os = "macos"))]
    (
        Page::Performance,
        GroupId::CULLING,
        RowId("graphics:gpu_culling"),
    ),
    (
        Page::Performance,
        GroupId::CULLING,
        RowId("graphics:entity_culling"),
    ),
    (
        Page::Performance,
        GroupId::CULLING,
        RowId("graphics:particle_culling"),
    ),
    (
        Page::Performance,
        GroupId::CULLING,
        RowId("graphics:entity_culling_precheck"),
    ),
    (
        Page::Performance,
        GroupId::CULLING,
        RowId("graphics:leaf_culling"),
    ),
    (
        Page::Performance,
        GroupId::CULLING,
        RowId("graphics:opaque_ice"),
    ),
    (
        Page::Performance,
        GroupId::BLOCK_ENTITIES,
        RowId("graphics:cache_shulker_boxes"),
    ),
    (
        Page::Performance,
        GroupId::BLOCK_ENTITIES,
        RowId("graphics:cache_signs"),
    ),
    (
        Page::Performance,
        GroupId::BLOCK_ENTITIES,
        RowId("graphics:cache_decorated_pots"),
    ),
    (
        Page::Performance,
        GroupId::BLOCK_ENTITIES,
        RowId("graphics:cache_bells"),
    ),
    (
        Page::Performance,
        GroupId::BLOCK_ENTITIES,
        RowId("graphics:cache_chests"),
    ),
    (
        Page::Performance,
        GroupId::BLOCK_ENTITIES,
        RowId("graphics:cache_banners"),
    ),
    (
        Page::Performance,
        GroupId::BLOCK_ENTITIES,
        RowId("graphics:cache_copper_golem_statues"),
    ),
    (
        Page::Performance,
        GroupId::POWER,
        RowId("graphics:battery_mode"),
    ),
    (
        Page::Shaders,
        GroupId::BUILT_IN_SHADERS,
        RowId("graphics:shadow_cache"),
    ),
    (
        Page::Advanced,
        GroupId::FRAMES,
        RowId("graphics:present_mode"),
    ),
    (Page::Advanced, GroupId::FRAMES, RowId("graphics:pacing")),
    #[cfg(not(target_os = "macos"))]
    (
        Page::Advanced,
        GroupId::FRAMES,
        RowId("graphics:graphics_api"),
    ),
    #[cfg(not(target_os = "macos"))]
    (
        Page::Advanced,
        GroupId::FRAMES,
        RowId("graphics:translucent_batching"),
    ),
    (Page::Hdr, GroupId::HDR_OUTPUT, RowId("graphics:hdr")),
    (
        Page::Hdr,
        GroupId::HDR_OUTPUT,
        RowId("graphics:hdr_smooth_gradients"),
    ),
    (
        Page::Hdr,
        GroupId::HDR_OUTPUT,
        RowId("graphics:hdr_calibrate"),
    ),
    (
        Page::Hdr,
        GroupId::HDR_BRIGHTNESS,
        RowId("graphics:hdr_black_level"),
    ),
    (
        Page::Hdr,
        GroupId::HDR_BRIGHTNESS,
        RowId("graphics:hdr_peak"),
    ),
    (
        Page::Hdr,
        GroupId::HDR_BRIGHTNESS,
        RowId("graphics:hdr_paper_white"),
    ),
    (
        Page::Hdr,
        GroupId::HDR_BRIGHTNESS,
        RowId("graphics:hdr_ui_brightness"),
    ),
    (
        Page::Hdr,
        GroupId::HDR_BRIGHTNESS,
        RowId("graphics:hdr_lighting"),
    ),
];

#[cfg(feature = "sunrise-extras")]
#[test]
fn ordered_rows_snapshot_with_extras_matches_baseline() {
    assert_eq!(flatten_resolved(resolved_rows()), EXPECTED_ROWS_WITH_EXTRAS);
    assert_combined_sunrise_graphics_order(EXPECTED_ROWS_WITH_EXTRAS);
}

#[cfg(not(feature = "sunrise-extras"))]
#[test]
fn ordered_rows_snapshot_without_extras_matches_baseline() {
    assert_eq!(
        flatten_resolved(resolved_rows()),
        EXPECTED_ROWS_WITHOUT_EXTRAS
    );
    assert_combined_sunrise_graphics_order(EXPECTED_ROWS_WITHOUT_EXTRAS);
}

#[test]
fn ordered_rows_snapshot_without_descriptors_matches_vanilla() {
    let (pages, issues) = resolve_rows_with_issues(cfg!(target_os = "macos"), &[]);
    assert!(issues.is_empty(), "{issues:?}");
    assert_eq!(flatten_resolved(pages), EXPECTED_ROWS_WITHOUT_EXTRAS);
}

#[cfg(feature = "sunrise-extras")]
#[test]
fn undo_restores_all_three_extras_including_hidden_fullbright() {
    use super::tests::SIZE;
    use crate::settings_shell::Columns;

    let mut s = Settings::default();
    let mut ui = ui();
    let at = |[x, y, ..]: [f32; 4]| (x + 2.0, y + 2.0);
    let hide_particles = Item::Described(RowId("sunrise:hide_block_particles"));
    let dynamic_lights = Item::Described(RowId("sunrise:dynamic_lights"));
    let fullbright = Item::Described(RowId("sunrise:fullbright"));

    for item in [hide_particles, dynamic_lights, fullbright] {
        let rect = row(&mut ui, &s, item);
        assert_eq!(press(&mut ui, &mut s, at(rect)), [CLICK, CHANGED]);
    }
    assert!(s.extras.hide_block_particles);
    assert!(s.extras.dynamic_lights);
    assert!(s.extras.fullbright);
    assert!(ui.changed());

    s.fullbright_held = true;
    ui.show(Page::Quality);
    assert!(
        !ui.listed(&s, SIZE)
            .iter()
            .any(|(_, item)| *item == fullbright)
    );

    let undo = Columns::of(SIZE.0, SIZE.1).button(1);
    assert_eq!(press(&mut ui, &mut s, at(undo)), [CLICK, CHANGED]);
    assert!(!s.extras.hide_block_particles);
    assert!(!s.extras.dynamic_lights);
    assert!(!s.extras.fullbright);
    assert!(!ui.changed());
}

#[cfg(feature = "sunrise-extras")]
#[test]
fn presets_leave_the_three_extras_alone() {
    let extras = [
        Item::Described(RowId("sunrise:hide_block_particles")),
        Item::Described(RowId("sunrise:dynamic_lights")),
        Item::Described(RowId("sunrise:fullbright")),
    ];
    for on in [false, true] {
        let mut s = Settings::default();
        s.extras.hide_block_particles = on;
        s.extras.dynamic_lights = on;
        s.extras.fullbright = on;
        for preset in Preset::ALL {
            crate::graphics::apply_preset(preset, &mut s);
            assert_eq!(
                (
                    s.extras.hide_block_particles,
                    s.extras.dynamic_lights,
                    s.extras.fullbright,
                ),
                (on, on, on),
                "{preset:?}"
            );
        }
    }
    let mut s = Settings::default();
    s.sunrise.preset = Preset::Balanced;
    for item in extras {
        item.cycle(&mut s, false, FB, &NONE);
        assert_eq!(s.sunrise.preset, Preset::Balanced, "{item:?}");
    }
}

#[test]
fn rejected_descriptors_never_participate_in_lookup_stamps_or_undo() {
    use std::sync::atomic::{AtomicBool, Ordering};

    static REJECTED_SHOWN: AtomicBool = AtomicBool::new(false);
    static REJECTED_GET: AtomicBool = AtomicBool::new(false);
    static REJECTED_SET: AtomicBool = AtomicBool::new(false);
    static REJECTED_UNDO: AtomicBool = AtomicBool::new(false);
    static WINNER_UNDO: AtomicBool = AtomicBool::new(false);
    static DUP_WINNER_UNDO: AtomicBool = AtomicBool::new(false);

    fn bad_shown(_: &Settings) -> bool {
        REJECTED_SHOWN.store(true, Ordering::SeqCst);
        true
    }
    fn bad_get(_: &Settings) -> bool {
        REJECTED_GET.store(true, Ordering::SeqCst);
        true
    }
    fn bad_set(_: &mut Settings, _: bool) {
        REJECTED_SET.store(true, Ordering::SeqCst);
    }
    fn bad_undo(_: &mut Settings, _: &Settings) {
        REJECTED_UNDO.store(true, Ordering::SeqCst);
    }
    fn track_winner_undo(s: &mut Settings, b: &Settings) {
        WINNER_UNDO.store(true, Ordering::SeqCst);
        undo_sky(s, b);
    }
    fn track_dup_winner_undo(s: &mut Settings, b: &Settings) {
        DUP_WINNER_UNDO.store(true, Ordering::SeqCst);
        undo_weather(s, b);
    }

    static INVALID_GROUP_FIRST: RowDescriptor = RowDescriptor {
        id: RowId("test:winner"),
        placement: Placement {
            page: Page::Performance,
            group: GroupId("performance.unknown"),
            after: None,
        },
        caption_key: "test.rejected_invalid",
        caption_fallback: None,
        description_key: "test.desc",
        chosen_description: None,
        cost: Cost::None,
        control: RowControl::Switch {
            get: bad_get,
            set: bad_set,
        },
        shown: bad_shown,
        blocked: None,
        affects_preset: false,
        undo: bad_undo,
    };
    static VALID_SECOND: RowDescriptor = RowDescriptor {
        id: RowId("test:winner"),
        placement: Placement {
            page: Page::Performance,
            group: GroupId::POWER,
            after: Some(H::BatteryMode.row_id()),
        },
        caption_key: "test.accepted_after_invalid",
        caption_fallback: None,
        description_key: "test.desc",
        chosen_description: None,
        cost: Cost::None,
        control: RowControl::Switch {
            get: get_sky,
            set: set_sky,
        },
        shown: always_shown,
        blocked: None,
        affects_preset: false,
        undo: track_winner_undo,
    };
    static VALID_FIRST: RowDescriptor = RowDescriptor {
        id: RowId("test:dup_winner"),
        placement: Placement {
            page: Page::Performance,
            group: GroupId::POWER,
            after: Some(RowId("test:winner")),
        },
        caption_key: "test.accepted_first",
        caption_fallback: None,
        description_key: "test.desc",
        chosen_description: None,
        cost: Cost::None,
        control: RowControl::Switch {
            get: get_weather,
            set: set_weather,
        },
        shown: always_shown,
        blocked: None,
        affects_preset: false,
        undo: track_dup_winner_undo,
    };
    static DUPLICATE_SECOND: RowDescriptor = RowDescriptor {
        id: RowId("test:dup_winner"),
        placement: Placement {
            page: Page::General,
            group: GroupId::INTERFACE,
            after: None,
        },
        caption_key: "test.rejected_dup",
        caption_fallback: None,
        description_key: "test.desc",
        chosen_description: None,
        cost: Cost::None,
        control: RowControl::Switch {
            get: bad_get,
            set: bad_set,
        },
        shown: bad_shown,
        blocked: None,
        affects_preset: false,
        undo: bad_undo,
    };
    static ENGINE_COLLISION: RowDescriptor = RowDescriptor {
        id: RowId("vanilla:particles"),
        placement: Placement {
            page: Page::Performance,
            group: GroupId::POWER,
            after: None,
        },
        caption_key: "test.rejected_engine",
        caption_fallback: None,
        description_key: "test.desc",
        chosen_description: None,
        cost: Cost::None,
        control: RowControl::Switch {
            get: bad_get,
            set: bad_set,
        },
        shown: bad_shown,
        blocked: None,
        affects_preset: false,
        undo: bad_undo,
    };
    static ROWS: [&RowDescriptor; 5] = [
        &INVALID_GROUP_FIRST,
        &VALID_SECOND,
        &VALID_FIRST,
        &DUPLICATE_SECOND,
        &ENGINE_COLLISION,
    ];

    REJECTED_SHOWN.store(false, Ordering::SeqCst);
    REJECTED_GET.store(false, Ordering::SeqCst);
    REJECTED_SET.store(false, Ordering::SeqCst);
    REJECTED_UNDO.store(false, Ordering::SeqCst);
    WINNER_UNDO.store(false, Ordering::SeqCst);
    DUP_WINNER_UNDO.store(false, Ordering::SeqCst);

    let _guard = with_test_descriptors(&ROWS);
    let mut screen = GraphicsScreen::default();
    let baseline = Settings::default();
    let mut s = baseline.clone();

    let winner = Item::Described(RowId("test:winner"));
    let dup_winner = Item::Described(RowId("test:dup_winner"));
    let collided = Item::Described(RowId("vanilla:particles"));

    assert_eq!(
        descriptor(RowId("test:winner")).map(|d| d.caption_key),
        Some("test.accepted_after_invalid")
    );
    assert_eq!(
        descriptor(RowId("test:dup_winner")).map(|d| d.caption_key),
        Some("test.accepted_first")
    );
    assert!(descriptor(RowId("vanilla:particles")).is_none());

    let stamp_before = screen.stamp(&s);
    let _ = screen.sheets(&s);
    assert_eq!(winner.switch(&s, &NONE), Some(true));
    assert_eq!(dup_winner.switch(&s, &NONE), Some(true));
    assert_eq!(collided.switch(&s, &NONE), None);

    winner.cycle(&mut s, false, FB, &NONE);
    dup_winner.cycle(&mut s, false, FB, &NONE);
    collided.cycle(&mut s, false, FB, &NONE);
    assert!(!s.sunrise.sky);
    assert!(!s.sunrise.weather);
    assert_ne!(screen.stamp(&s), stamp_before);

    screen.undo(&mut s, &baseline);
    assert!(s.sunrise.sky);
    assert!(s.sunrise.weather);
    assert!(WINNER_UNDO.load(Ordering::SeqCst));
    assert!(DUP_WINNER_UNDO.load(Ordering::SeqCst));
    assert!(!REJECTED_SHOWN.load(Ordering::SeqCst));
    assert!(!REJECTED_GET.load(Ordering::SeqCst));
    assert!(!REJECTED_SET.load(Ordering::SeqCst));
    assert!(!REJECTED_UNDO.load(Ordering::SeqCst));
}

#[test]
fn described_row_uses_language_separator_under_value_first_template() {
    struct ValueFirst;
    impl lang::ResourceSource for ValueFirst {
        fn read(&self, path: &str) -> Option<Vec<u8>> {
            (path == "assets/minecraft/lang/en_ud.json")
                .then(|| br#"{"options.generic_value":"%2$s :%1$s"}"#.to_vec())
        }
    }
    static SWITCH: RowDescriptor = RowDescriptor {
        id: RowId("test:value_first_switch"),
        placement: Placement {
            page: Page::Performance,
            group: GroupId::POWER,
            after: Some(H::BatteryMode.row_id()),
        },
        caption_key: "test.value_first",
        caption_fallback: Some("Sample Switch"),
        description_key: "test.value_first.desc",
        chosen_description: None,
        cost: Cost::None,
        control: RowControl::Switch {
            get: get_fog,
            set: set_fog,
        },
        shown: always_shown,
        blocked: None,
        affects_preset: false,
        undo: undo_fog,
    };
    static ROWS: [&RowDescriptor; 1] = [&SWITCH];
    let _guard = with_test_descriptors(&ROWS);

    let s = Settings::default();
    let setting = setting_of(&SWITCH);
    let (built, _) = lang::scoped(lang::Language::load(&[&ValueFirst], "en_ud"), || {
        super::row(setting, &s, &NONE)
    });
    assert_eq!(built.name, "On :Sample Switch");
    assert_eq!(built.value, Value::Switch(true));
}

#[cfg(feature = "sunrise-extras")]
#[test]
fn stamp_changes_on_extras_and_fullbright_held_when_extras_enabled() {
    let screen = GraphicsScreen::default();
    let mut s = Settings::default();
    let base = screen.stamp(&s);

    s.extras.own_name_tag = true;
    let after_unrelated = screen.stamp(&s);
    assert_ne!(after_unrelated, base);

    s.extras.wheel_cycles_settings = true;
    let after_wheel = screen.stamp(&s);
    assert_ne!(after_wheel, after_unrelated);

    s.fullbright_held = true;
    assert_ne!(screen.stamp(&s), after_wheel);
}

#[cfg(not(feature = "sunrise-extras"))]
#[test]
fn stamp_ignores_extras_and_fullbright_held_when_extras_disabled() {
    let screen = GraphicsScreen::default();
    let mut s = Settings::default();
    let base = screen.stamp(&s);

    s.extras.own_name_tag = true;
    s.extras.wheel_cycles_settings = true;
    s.extras.fullbright = true;
    s.fullbright_held = true;
    assert_eq!(screen.stamp(&s), base);
}
