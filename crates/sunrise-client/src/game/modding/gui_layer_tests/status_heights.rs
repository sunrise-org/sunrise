//! A layer reads the status columns' heights as they stand when it starts (NeoForge's
//! `Hud.leftHeight`, `rightHeight`).

use crate::game::{
    execution_role::ExecutionRole,
    mods,
    native_entity_refs::tests::{run_fresh_cases, with_client_game},
};
use mod_api::{
    ClientHost, GuiLayerPlacement, Hooks, ModEntry, RegisterGuiLayersEvent, RegistrationContext,
};
use std::sync::Mutex;
use sunrise_client::motion::PlayerMotion;
use sunrise_core::geometry::Vec3;

static SEEN: Mutex<Vec<(&str, Option<[i32; 2]>)>> = Mutex::new(Vec::new());

struct Probe {
    replace_armor: bool,
}

#[mod_api::__subscriptions]
impl Hooks for Probe {
    fn register_gui_layers(&self, _: &RegistrationContext, event: &mut RegisterGuiLayersEvent<'_>) {
        use GuiLayerPlacement::*;
        for (id, placement) in [
            (
                "heights:before_health",
                Below("minecraft:player_health".into()),
            ),
            (
                "heights:after_health",
                Above("minecraft:player_health".into()),
            ),
            ("heights:after_armor", Above("minecraft:armor_level".into())),
            ("heights:after_food", Above("minecraft:food_level".into())),
            ("heights:after_air", Above("minecraft:air_level".into())),
        ] {
            assert!(event.layers.register(
                id.into(),
                placement,
                Box::new(move |host: &dyn ClientHost, _| {
                    SEEN.lock().unwrap().push((id, host.gui_status_heights()));
                }),
            ));
        }
        if self.replace_armor {
            assert!(
                event
                    .layers
                    .replace_layer("minecraft:armor_level", Box::new(|_, _| {}))
            );
        }
    }
}

#[test]
#[ignore = "daily"]
fn a_layer_reads_the_status_heights_the_vanilla_rows_before_it_left() {
    const CASE: &str = "SUNRISE_GUI_STATUS_HEIGHTS_FIXTURE";
    if run_fresh_cases(
        "game::gui_layer_tests::status_heights::a_layer_reads_the_status_heights_the_vanilla_rows_before_it_left",
        CASE,
        &["vanilla", "replace_armor"],
    ) {
        return;
    }
    let replace_armor = std::env::var(CASE).unwrap() == "replace_armor";
    with_client_game(|game| {
        mod_platform::__private::install(
            vec![ModEntry::new("heights", Box::new(Probe { replace_armor }))].into(),
        );
        mods::register_gui_layers(ExecutionRole::Client);
        game.title = None;
        game.screen = crate::game::Screen::Playing;
        game.ui.hide_gui = false;
        game.set_mode_id(0);
        // One row of hearts, one point of armour, full food and depleted air above water: the
        // health, armour, food and air rows each take 10 pixels.
        let mut player = PlayerMotion::new(game.motion, Vec3::new(2.0, 64.0, 2.0)).unwrap();
        player.set_health(12.0, false).unwrap();
        player
            .set_modifier("minecraft:armor", "heights:armor", 2.0, "add_value", true)
            .unwrap();
        player.air = 181;
        game.player = Some(player);
        assert_eq!(game.gui_status_heights(), None);
        game.hud_layers([640, 480]);
        let after_armor = if replace_armor { [49, 39] } else { [59, 39] };
        let [left, _] = after_armor;
        assert_eq!(
            *SEEN.lock().unwrap(),
            [
                ("heights:before_health", Some([39, 39])),
                ("heights:after_health", Some([49, 39])),
                ("heights:after_armor", Some(after_armor)),
                ("heights:after_food", Some([left, 49])),
                ("heights:after_air", Some([left, 59])),
            ]
        );
        assert_eq!(game.gui_status_heights(), None);
    });
}
