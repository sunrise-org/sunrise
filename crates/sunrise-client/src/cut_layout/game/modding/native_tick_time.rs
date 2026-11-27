#![cfg(test)]

#[cfg(test)]
mod tests {
    use mod_api::{
        Hooks, HostSide, LevelTickPreEvent, ModEntry, ServerHost, ServerTickEvent, ServerTickPhase,
    };
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};
    use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1;
    use sunrise_gameplay_common::game::native_hooks::base::GameHooks;
    use sunrise_gameplay_common::game::native_server::NativeServerGameExt4;
    use {
        crate::game::server_side::native_entity_refs::tests::with_client_game,
        sunrise_gameplay::game::native_entity_refs::tests::with_game,
    };

    #[test]
    #[ignore = "daily"]
    fn a_server_has_time_until_its_tick_deadline_passes() {
        with_game(|game| {
            game.server.server_tick_deadline = None;
            assert!(game.server_has_time());
            game.server.server_tick_deadline = Some(Instant::now() + Duration::from_secs(60));
            assert!(game.server_has_time());
            game.server.server_tick_deadline = Some(Instant::now() - Duration::from_millis(1));
            assert!(!game.server_has_time());
        });
    }

    #[test]
    #[ignore = "daily"]
    fn a_client_level_always_has_time() {
        with_client_game(|game| {
            assert!(game.server_has_time());
        });
    }

    /// What `host.server_has_time()` answered inside each listener the docs name.
    struct Spare(Arc<Mutex<Vec<(&'static str, bool)>>>);
    #[mod_api::__subscriptions]
    impl Hooks for Spare {
        fn server_tick(&self, host: &mut dyn ServerHost, _: &mut ServerTickEvent<'_>) {
            self.0
                .lock()
                .unwrap()
                .push(("server_tick", host.server_has_time()));
        }
        fn level_tick_pre(&self, host: HostSide, _: &mut LevelTickPreEvent) {
            if let HostSide::Server(host) = host {
                self.0
                    .lock()
                    .unwrap()
                    .push(("level_tick_pre", host.server_has_time()));
            }
        }
    }

    #[test]
    #[ignore = "daily"]
    fn the_tick_listeners_see_the_deadline_through_the_host() {
        let heard = Arc::new(Mutex::new(Vec::new()));
        let mods = vec![ModEntry::new("spare", Box::new(Spare(heard.clone())))].into();
        mod_platform::testing::with_mods(mods, || {
            with_game(|game| {
                let context =
                    mod_api::LifecycleContext::from(mod_api::__private::built::LifecycleContext {
                        authority: mod_api::Authority::DedicatedServer,
                        epoch: mod_api::WorldEpoch::new(5),
                        tick: mod_api::TickId::new(1),
                    });
                let level = game.level("minecraft:overworld").unwrap();
                for (deadline, spare) in [
                    (Instant::now() + Duration::from_secs(60), true),
                    (Instant::now() - Duration::from_millis(1), false),
                ] {
                    game.server.server_tick_deadline = Some(deadline);
                    game.native_server_tick(
                        &context,
                        ServerTickPhase::ServerTickStart {
                            world_running: true,
                        },
                    );
                    game.native_level_tick_pre(level);
                    assert_eq!(
                        std::mem::take(&mut *heard.lock().unwrap()),
                        [("server_tick", spare), ("level_tick_pre", spare)]
                    );
                }
            });
        });
    }
}
