use super::*;

/// The hook points. Each does nothing in Sunrise.
impl Game<'_> {
    pub(in crate::game) fn frame_partial_ticks(&self) -> (f32, f32) {
        let partial_tick = self.clock.window_fraction();
        let world_partial_tick = if !self.shows_panorama() && self.player.is_some() {
            self.clock.fraction() as f32
        } else {
            partial_tick
        };
        (world_partial_tick, partial_tick)
    }

    pub(in crate::game) fn frame_world_partial_tick(&self) -> f64 {
        self.extras
            .frame_partial_ticks
            .map_or_else(|| self.clock.fraction(), |(world, _)| f64::from(world))
    }

    /// Once a frame that reaches the render call, after its extraction and just before it
    /// is rendered (`RenderFrameEvent.Pre`, around `GameRenderer.render`).
    pub(in crate::game) fn native_render_frame_pre(
        &mut self,
        elapsed: Duration,
        world_partial_tick: f32,
        partial_tick: f32,
        realtime_delta_ticks: f32,
    ) {
        self.fire_render_frame_pre(
            elapsed,
            world_partial_tick,
            partial_tick,
            realtime_delta_ticks,
        );
    }

    /// Fire `render_frame_pre` with the frame's captured time and the clock's real-time delta;
    /// the same values go to `render_frame_post`.
    pub(in crate::game) fn begin_render_frame(&mut self, elapsed: Duration) -> FrameMoments {
        let (world_partial_tick, partial_tick) = self
            .extras
            .frame_partial_ticks
            .unwrap_or_else(|| self.frame_partial_ticks());
        let moments = FrameMoments {
            world_partial_tick,
            partial_tick,
            realtime_delta_ticks: self.clock.realtime_delta_ticks(),
        };
        self.native_render_frame_pre(
            elapsed,
            moments.world_partial_tick,
            moments.partial_tick,
            moments.realtime_delta_ticks,
        );
        moments
    }
}

/// What `render_frame_pre` reported for a frame, kept for its `render_frame_post`.
#[derive(Clone, Copy)]
pub(in crate::game) struct FrameMoments {
    pub(in crate::game) world_partial_tick: f32,
    pub(in crate::game) partial_tick: f32,
    pub(in crate::game) realtime_delta_ticks: f32,
}

#[cfg(test)]
mod tests {
    use super::super::super::native_entity_refs::tests::{run_fresh_cases, with_client_game};
    use super::super::particle_count_tests::frame_scene;
    use mod_api::{
        ClientHost, FlipFrameEvent, Hooks, ModEntry, RenderFramePostEvent, RenderFramePreEvent,
        RenderLevelStageEvent,
    };
    use std::{
        sync::{Arc, Mutex},
        time::Duration,
    };
    use sunrise_client::{clock::FixedStep, title::Title};

    #[derive(Default)]
    struct Heard {
        pre: Vec<(f32, f32)>,
        post: Vec<(f32, f32)>,
        stages: Vec<f32>,
        /// The moments in the order they were heard.
        order: Vec<&'static str>,
        /// The extracted frame's camera as Pre saw it.
        cameras: Vec<Option<f32>>,
        /// `realtime_delta_ticks` of Pre and of Post, per frame.
        realtime: Vec<(f32, f32)>,
    }
    struct Frames(Arc<Mutex<Heard>>);
    #[mod_api::__subscriptions]
    impl Hooks for Frames {
        fn render_frame_pre(&self, host: &mut dyn ClientHost, event: &mut RenderFramePreEvent) {
            let mut heard = self.0.lock().unwrap();
            heard
                .pre
                .push((event.world_partial_tick, event.partial_tick));
            heard.order.push("pre");
            heard.cameras.push(host.camera().map(|c| c.partial_tick));
            heard.realtime.push((event.realtime_delta_ticks, f32::NAN));
            event.realtime_delta_ticks = -1.0;
            // Listener edits must not become the renderer's or post event's time.
            event.world_partial_tick = -1.0;
            event.partial_tick = -1.0;
        }
        fn render_frame_post(&self, _: &mut dyn ClientHost, event: &mut RenderFramePostEvent) {
            let mut heard = self.0.lock().unwrap();
            heard
                .post
                .push((event.world_partial_tick, event.partial_tick));
            heard.order.push("post");
            heard.realtime.last_mut().unwrap().1 = event.realtime_delta_ticks;
        }
        fn render_level_stage(&self, _: &dyn ClientHost, event: &mut RenderLevelStageEvent<'_>) {
            let mut heard = self.0.lock().unwrap();
            heard.stages.push(event.view.partial_tick);
            heard.order.push("draw");
        }
        fn flip_frame(&self, _: &mut dyn ClientHost, _: &mut FlipFrameEvent) {
            self.0.lock().unwrap().order.push("flip");
        }
    }

    /// Every draw of a frame that reaches the render call fires the pair once: the early
    /// loading and upgrade draws go through `bracket_frame`, the normal frame's `consume`
    /// sits between `begin_render_frame` and `finish_render_frame`.
    #[test]
    fn every_draw_in_render_with_hooks_is_bracketed() {
        let source = include_str!("../../game.rs");
        let start = source.find("    fn render_with_hooks(").unwrap();
        let rest = &source[start + 1..];
        let body = &rest[..rest.find("\n    fn ").unwrap()];
        let draws = body.matches(".render(").count();
        let bracketed = body.matches("bracket_frame(elapsed").count();
        assert!(bracketed >= 4);
        assert_eq!(draws, bracketed);
        for (at, _) in body.match_indices(".render(") {
            assert!(body[at.saturating_sub(120)..at].contains("bracket_frame(elapsed"));
        }
        let (consume, pre, post) = (
            body.find("consume(\n").unwrap(),
            body.find("begin_render_frame(").unwrap(),
            body.find("finish_render_frame(").unwrap(),
        );
        assert!(pre < consume && consume < post);
        assert_eq!(body.matches("consume(\n").count(), 1);
    }

    #[test]
    #[ignore = "daily"]
    fn frame_interpolation_matches_draw_and_holds_while_paused_or_frozen() {
        let name = "game::native_hooks::render_frame_pre::tests::frame_interpolation_matches_draw_and_holds_while_paused_or_frozen";
        if run_fresh_cases(name, "SUNRISE_FRAME_PARTIAL_CHILD", &["frames"]) {
            return;
        }
        let heard = Arc::new(Mutex::new(Heard::default()));
        let mods = vec![ModEntry::new("frames", Box::new(Frames(heard.clone())))];
        mod_platform::testing::with_mods(mods.into(), || {
            with_client_game(|game| {
                let _event_loop = frame_scene(game);
                game.clock = FixedStep::client();
                game.clock.set_frozen(false);
                game.clock.end_frame(false);
                let elapsed = Duration::from_secs_f64(1.0 / 60.0);
                let frame = |game: &mut super::Game<'_>, world: bool| {
                    game.clock.advance(elapsed, 50.0);
                    game.extras_frame(elapsed);
                    // A clock change after the frame's capture cannot change the draw/post value.
                    game.clock.advance(Duration::from_millis(1), 50.0);
                    game.render_and_flip(elapsed).unwrap();
                    let mut h = heard.lock().unwrap();
                    let pre = *h.pre.last().unwrap();
                    assert_eq!(h.post.last(), Some(&pre));
                    if world {
                        assert!(!h.stages.is_empty());
                        assert!(h.stages.iter().all(|&partial| partial == pre.0));
                        // Extraction -> pre -> draw -> post -> flip, each once around the draw.
                        let draws = h.stages.len();
                        let mut expected = vec!["pre"];
                        expected.extend(std::iter::repeat_n("draw", draws));
                        expected.extend(["post", "flip"]);
                        assert_eq!(h.order, expected);
                        // Pre already sees this frame's extracted camera, so it follows the
                        // extraction; a listener's write to its event changes neither the
                        // draw's time (checked above) nor Post's.
                        assert!(h.cameras.last().unwrap().is_some());
                    } else {
                        assert!(h.stages.is_empty());
                        assert_eq!(pre.0, pre.1);
                        assert_eq!(h.order, ["pre", "post", "flip"]);
                        assert_eq!(*h.cameras.last().unwrap(), None);
                    }
                    h.stages.clear();
                    h.order.clear();
                    pre
                };
                let running = frame(game, true);
                assert_eq!(running, (0.32, 0.32));
                let held = game.clock.fraction() as f32;
                game.clock.end_frame(true);
                let paused = frame(game, true);
                let paused_next = frame(game, true);
                assert_eq!(paused.0, held);
                assert_eq!(paused_next.0, held);
                assert_ne!(paused.1, paused_next.1);
                game.clock.set_frozen(true);
                assert!(!game.clock.world_runs());
                game.clock.end_frame(false);
                let frozen = frame(game, true);
                let frozen_next = frame(game, true);
                assert_eq!(frozen.0, 1.0);
                assert_eq!(frozen_next.0, 1.0);
                assert_ne!(frozen.1, frozen_next.1);
                // Real time in ticks of 50 ms, whatever the game ticks do: kept at 7, 0.5 above.
                let realtime = |game: &mut super::Game<'_>, millis: u64| {
                    game.clock.advance(Duration::from_millis(millis), 50.0);
                    game.extras_frame(elapsed);
                    game.render_and_flip(elapsed).unwrap();
                    let mut h = heard.lock().unwrap();
                    h.order.clear();
                    h.stages.clear();
                    *h.realtime.last().unwrap()
                };
                assert_eq!(realtime(game, 350), (7.0, 7.0));
                assert_eq!(realtime(game, 351), (0.5, 0.5));
                assert_eq!(realtime(game, 10), (0.2, 0.2));
                game.title = Some(Title::new(
                    game.save.root.clone(),
                    &game.options.resources,
                    "",
                ));
                frame(game, false);
            });
        });
    }
}
