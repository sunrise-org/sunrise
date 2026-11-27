use super::tests::{CHANGED, CLICK, FB, NONE, SIZE, press, row, ui};
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
