#[test]
#[ignore = "daily"]
fn dimension_transition_screen_is_selected_at_entry_drawn_and_dropped() {
    use crate::game::execution_role::ExecutionRole;
    use crate::game::native_entity_refs::tests::with_game;
    use mod_api::__private::game::RegistrationContextNew;
    use mod_api::{DimensionTransition, DimensionTransitionScreen};
    use sunrise_client::{hud::HudBatch, level_loading::Reason};

    struct Arrival;
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for Arrival {
        fn register_dimension_transition_screen(
            &self,
            _: &mod_api::RegistrationContext,
            event: &mut mod_api::RegisterDimensionTransitionScreenEvent<'_>,
        ) {
            event.screens.push(DimensionTransitionScreen {
                screen: "transition_test:arrival".into(),
                transition: DimensionTransition::Conditional {
                    to: "transition_test:b".into(),
                    from: "transition_test:a".into(),
                },
            });
        }
        fn dimension_transition_screen_draw(
            &self,
            _: &dyn mod_api::ClientHost,
            event: &mut mod_api::DimensionTransitionScreenDrawEvent<'_>,
        ) {
            assert_eq!(event.screen, "transition_test:arrival");
            assert_eq!(event.reason, mod_api::DimensionTransitionReason::Other);
            assert_eq!(event.progress, Some(0.25));
            event.draw.fill([0.0, 0.0, 80.0, 10.0], 0xFF102030);
        }
    }
    crate::game::in_own_process(
        "game::remote::tests::dimension_transition_tests::dimension_transition_screen_is_selected_at_entry_drawn_and_dropped",
        || {
            mod_platform::testing::with_mods(
                vec![mod_api::ModEntry::new("transition_test", Box::new(Arrival))].into(),
                || {
                    let mut registrations = Vec::new();
                    super::Game::native_register_dimension_transition_screen(
                        &mod_api::RegistrationContext::new(),
                        &mut registrations,
                    );
                    sunrise_client_render::dimension_transition::register(registrations);
                    with_game(|game| {
                        game.options.execution_role = ExecutionRole::Client;
                        let (client, _server) = sunrise_protocol::connection::Pipe::pair();
                        game.connect_local(client, "TransitionTest").unwrap();
                        let mut spawn = sunrise_protocol::packets::play::SpawnInfo {
                            dimension_type: 0,
                            dimension: "transition_test:a".into(),
                            seed: 0,
                            game_type: 0,
                            previous_game_type: None,
                            debug: false,
                            flat: true,
                            last_death: None,
                            portal_cooldown: 0,
                            sea_level: 63,
                        };
                        game.enter_remote_dimension(&spawn, false, Reason::Other);
                        assert_eq!(
                            game.remote.as_ref().unwrap().level_load.transition_screen(),
                            None
                        );
                        spawn.dimension = "transition_test:b".into();
                        game.enter_remote_dimension(&spawn, true, Reason::Other);
                        let tracker = game.remote.as_ref().unwrap().level_load;
                        assert!(tracker.transition_screen().is_some());
                        let mut batch = HudBatch::scratch([320, 240]);
                        assert!(game.draw_dimension_transition_screen(
                            &mut batch,
                            &tracker,
                            Some(0.25)
                        ));
                        assert!(!batch.vertices.is_empty());
                        game.remote.as_mut().unwrap().level_load.finish();
                        let tracker = game.remote.as_ref().unwrap().level_load;
                        assert!(!game.draw_dimension_transition_screen(&mut batch, &tracker, None));
                        spawn.dimension = "transition_test:c".into();
                        game.enter_remote_dimension(&spawn, true, Reason::Other);
                        let tracker = game.remote.as_ref().unwrap().level_load;
                        let mut vanilla = HudBatch::scratch([320, 240]);
                        assert!(!game.draw_dimension_transition_screen(
                            &mut vanilla,
                            &tracker,
                            None
                        ));
                        game.draw_level_loading_background(&mut vanilla, tracker.reason);
                        assert!(!vanilla.vertices.is_empty());
                    });
                },
            );
        },
    );
}
