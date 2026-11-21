//! Load-time dimension-key resolution and selection of client loading presentations.
use mod_api::{
    Answered, DimensionKey, DimensionTransition, DimensionTransitionPresentation,
    DimensionTransitionScreen, Identifier,
};
use std::{
    collections::{HashMap, hash_map::Entry},
    fmt,
    sync::OnceLock,
};

/// A mod's presentation instance for the lifetime of one loading screen.
pub struct ActiveTransitionScreen {
    pub owner: &'static str,
    pub presentation: Box<dyn DimensionTransitionPresentation>,
}

impl fmt::Debug for ActiveTransitionScreen {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ActiveTransitionScreen")
            .field("owner", &self.owner)
            .finish_non_exhaustive()
    }
}

#[derive(PartialEq, Eq, Hash)]
enum TransitionKey {
    Incoming(DimensionKey),
    Outgoing(DimensionKey),
    Conditional {
        to: DimensionKey,
        from: DimensionKey,
    },
}

type Registration = (&'static str, DimensionTransitionScreen);

/// Registrations resolved once at load; lookups never build dimension names.
#[derive(Default)]
struct TransitionScreens(HashMap<TransitionKey, Registration>);

static SCREENS: OnceLock<TransitionScreens> = OnceLock::new();

impl TransitionScreens {
    fn new(registrations: impl IntoIterator<Item = Registration>) -> Self {
        let mut screens = HashMap::new();
        for (owner, registration) in registrations {
            let id = registration.screen();
            if !Identifier::parse_namespaced(id).is_some_and(|id| id.namespace() != "minecraft") {
                log::warn!("[{owner}] Invalid dimension transition screen id: {id}");
                continue;
            }
            let key = match registration.transition() {
                DimensionTransition::Incoming(to) => resolve(to).map(TransitionKey::Incoming),
                DimensionTransition::Outgoing(from) => resolve(from).map(TransitionKey::Outgoing),
                DimensionTransition::Conditional { to, from } => resolve(to)
                    .zip(resolve(from))
                    .map(|(to, from)| TransitionKey::Conditional { to, from }),
            };
            match key.map(|key| screens.entry(key)) {
                Some(Entry::Vacant(entry)) => {
                    entry.insert((owner, registration));
                }
                _ => log::warn!("[{owner}] Invalid or duplicate dimension transition: {id}"),
            }
        }
        Self(screens)
    }

    /// Conditional, then incoming, then outgoing, as NeoForge's event javadoc specifies.
    fn select(
        &self,
        to: DimensionKey,
        from: DimensionKey,
    ) -> Option<(&'static str, &DimensionTransitionScreen)> {
        self.0
            .get(&TransitionKey::Conditional { to, from })
            .or_else(|| self.0.get(&TransitionKey::Incoming(to)))
            .or_else(|| self.0.get(&TransitionKey::Outgoing(from)))
            .map(|(owner, factory)| (*owner, factory))
    }
}

fn resolve(id: &str) -> Option<DimensionKey> {
    Identifier::parse_namespaced(id).map(|_| mod_api::__private::dimension_key(id))
}

/// Publish the client registrations once at startup.
pub fn register(registrations: Vec<Answered<DimensionTransitionScreen>>) {
    let registrations = registrations.into_iter().map(|r| (r.mod_id, r.answer));
    if SCREENS.set(TransitionScreens::new(registrations)).is_err() {
        log::warn!("Dimension transition screens already registered; keeping the first table");
    }
}

/// Choose a factory and its owner once, when a new loading screen opens for travel.
/// Without dimension keys (login or death), the caller does not try to select a factory.
pub fn select(
    to: DimensionKey,
    from: DimensionKey,
) -> Option<(&'static str, &'static DimensionTransitionScreen)> {
    SCREENS.get().and_then(|screens| screens.select(to, from))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::level_loading::{LevelLoadTracker, PlayerCheck, Reason, ServerLoadView, Stage};
    use mod_api::{ClientHost, DrawContext, Host};
    use std::sync::Arc;

    struct Client;
    impl Host for Client {}
    mod_api::impl_host_areas!(Client);
    impl ClientHost for Client {}
    mod_api::impl_client_host_areas!(Client);

    #[derive(Default)]
    struct Canvas(Option<u32>);
    impl DrawContext for Canvas {
        fn fill(&mut self, _: [f32; 4], argb: u32) {
            self.0 = Some(argb);
        }
    }

    fn key(id: &str) -> DimensionKey {
        mod_api::__private::dimension_key(id)
    }
    fn entry(screen: &str, transition: DimensionTransition, argb: u32) -> Registration {
        (
            "registrant",
            DimensionTransitionScreen::new(screen.into(), transition, move |_, _| {
                Box::new(
                    move |_: &dyn ClientHost, _, _, draw: &mut dyn DrawContext| {
                        draw.fill([0.0, 0.0, 1.0, 1.0], argb);
                    },
                )
            }),
        )
    }
    fn chosen(screens: &TransitionScreens, to: &str, from: &str) -> Option<u32> {
        let (owner, factory) = screens.select(key(to), key(from))?;
        assert_eq!(owner, "registrant");
        let mut canvas = Canvas::default();
        factory.create(&Client, Reason::Other.into()).draw(
            &Client,
            Reason::Other.into(),
            None,
            &mut canvas,
        );
        canvas.0
    }

    #[test]
    fn conditional_wins_over_incoming_then_outgoing_and_first_registration_wins() {
        let screens = TransitionScreens::new(vec![
            entry(
                "test:leave",
                DimensionTransition::Outgoing("test:a".into()),
                1,
            ),
            entry(
                "test:arrive",
                DimensionTransition::Incoming("test:b".into()),
                2,
            ),
            entry(
                "test:pair",
                DimensionTransition::Conditional {
                    to: "test:b".into(),
                    from: "test:a".into(),
                },
                3,
            ),
            entry(
                "test:duplicate",
                DimensionTransition::Conditional {
                    to: "test:b".into(),
                    from: "test:a".into(),
                },
                4,
            ),
            entry(
                "test:duplicate",
                DimensionTransition::Incoming("test:b".into()),
                5,
            ),
            entry(
                "test:duplicate",
                DimensionTransition::Outgoing("test:a".into()),
                6,
            ),
        ]);
        assert_eq!(chosen(&screens, "test:b", "test:a"), Some(3));
        assert_eq!(chosen(&screens, "test:b", "test:c"), Some(2));
        assert_eq!(chosen(&screens, "test:c", "test:a"), Some(1));
        assert_eq!(chosen(&screens, "test:c", "test:b"), None);
    }

    #[test]
    fn invalid_ids_do_not_reserve_a_transition_or_replace_vanilla() {
        let screens = TransitionScreens::new(vec![
            entry(
                "minecraft:reserved",
                DimensionTransition::Incoming("test:b".into()),
                1,
            ),
            entry(
                "missing_namespace",
                DimensionTransition::Incoming("test:b".into()),
                2,
            ),
            entry(
                "test:invalid",
                DimensionTransition::Incoming("INVALID:b".into()),
                3,
            ),
            entry(
                "test:invalid",
                DimensionTransition::Outgoing("missing_namespace".into()),
                4,
            ),
            entry(
                "test:invalid",
                DimensionTransition::Conditional {
                    to: "test:b".into(),
                    from: "INVALID:a".into(),
                },
                5,
            ),
            entry(
                "test:valid",
                DimensionTransition::Incoming("test:b".into()),
                6,
            ),
        ]);
        assert_eq!(chosen(&screens, "test:b", "test:a"), Some(6));
        assert_eq!(chosen(&screens, "INVALID:b", "test:a"), None);
        assert_eq!(chosen(&screens, "test:c", "missing_namespace"), None);
        assert_eq!(chosen(&screens, "test:b", "INVALID:a"), Some(6));
    }

    #[test]
    fn selected_screen_follows_existing_progress_readiness_and_cleanup_then_falls_back() {
        let screens = TransitionScreens::new([entry(
            "test:arrival",
            DimensionTransition::Conditional {
                to: "test:b".into(),
                from: "test:a".into(),
            },
            0xFF102030,
        )]);
        let player = PlayerCheck {
            outside_build_height: false,
            spectator: false,
            alive: true,
        };
        let mut tracker = LevelLoadTracker::new(500);
        tracker.start_client_load(0, Reason::Other);
        let (owner, factory) = screens.select(key("test:b"), key("test:a")).unwrap();
        tracker.set_transition_screen(ActiveTransitionScreen {
            owner,
            presentation: factory.create(&Client, tracker.reason.into()),
        });
        let progress = ServerLoadView::default();
        progress.start(Stage::LoadInitialChunks, 0);
        progress.finish(Stage::LoadInitialChunks);
        progress.start(Stage::LoadPlayerChunks, 49);
        progress.update(Stage::LoadPlayerChunks, 20, 49);
        assert_eq!(progress.server_progress(), 30.0 / 59.0);
        let mut canvas = Canvas::default();
        tracker.transition_screen().unwrap().presentation.draw(
            &Client,
            tracker.reason.into(),
            Some(progress.server_progress()),
            &mut canvas,
        );
        assert_eq!(canvas.0, Some(0xFF102030));
        tracker.tick_client_load(100, player);
        assert!(!tracker.is_level_ready(100));
        tracker.loading_packets_received();
        tracker.tick_client_load(150, player);
        assert!(!tracker.is_level_ready(150));
        tracker.player_section_ready();
        tracker.tick_client_load(200, player);
        assert!(!tracker.is_level_ready(699));
        assert!(tracker.is_level_ready(700));
        assert!(tracker.transition_screen().is_some());
        tracker.finish();
        assert!(tracker.transition_screen().is_none());
        assert!(!tracker.is_active());
        tracker.start_client_load(1000, Reason::Other);
        assert!(screens.select(key("test:c"), key("test:b")).is_none());
        assert!(tracker.transition_screen().is_none());
        tracker.loading_packets_received();
        tracker.player_section_ready();
        tracker.tick_client_load(1100, player);
        assert!(!tracker.is_level_ready(1599));
        assert!(tracker.is_level_ready(1600));
    }

    #[test]
    fn restart_keeps_the_presentation_until_abort_or_timeout_finish() {
        for time_out in [false, true] {
            let mut tracker = LevelLoadTracker::new(0);
            tracker.start_client_load(0, Reason::NetherPortal);
            let token = Arc::new(());
            let lifetime = Arc::downgrade(&token);
            tracker.set_transition_screen(ActiveTransitionScreen {
                owner: "registrant",
                presentation: Box::new(move |_: &dyn ClientHost, _, _, _: &mut dyn DrawContext| {
                    assert_eq!(Arc::strong_count(&token), 1);
                }),
            });
            tracker.start_client_load(100, Reason::EndPortal);
            assert_eq!(tracker.reason, Reason::EndPortal);
            assert!(lifetime.upgrade().is_some());
            if time_out {
                let player = PlayerCheck {
                    outside_build_height: false,
                    spectator: false,
                    alive: true,
                };
                tracker.loading_packets_received();
                tracker.tick_client_load(30_100, player);
                assert!(!tracker.is_level_ready(30_100));
                tracker.tick_client_load(30_101, player);
                assert!(tracker.is_level_ready(30_101));
                assert!(lifetime.upgrade().is_some());
            }
            tracker.finish();
            assert!(tracker.transition_screen().is_none());
            assert!(lifetime.upgrade().is_none());
        }
    }
}
