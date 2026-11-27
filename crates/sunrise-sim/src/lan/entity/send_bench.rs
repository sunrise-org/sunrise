//! Opt-in measurements of the tracker's send strategies with identical inputs and packet checks.
//! See docs/performance/tracker-send.md. Normal tests and `--ignored` alone do not measure.

use super::*;
use std::time::{Duration, Instant};

#[cfg(feature = "tracker-send-bench")]
#[global_allocator]
static ALLOCATOR: sunrise_threads::GameAllocator = sunrise_threads::GAME_ALLOCATOR;

struct Scene {
    level: Entities,
    entities: Vec<EntityState>,
    trackers: Vec<Tracker>,
    out: Vec<Vec<super::super::Body>>,
    chunks: crate::fast_hash::FxHashSet<(i32, i32)>,
    origins: Vec<[f64; 3]>,
}

impl Scene {
    fn new(viewers: usize, entities: usize, spread: bool, parallel: bool) -> Self {
        let level = Entities {
            send_apart_always: Some(parallel),
            ..Entities::default()
        };
        let origins: Vec<_> = (0..entities)
            .map(|i| {
                if spread {
                    let group = i % viewers;
                    let row = i / viewers;
                    let offset = if row % 2 == 1 { 128.0 } else { 0.0 };
                    [
                        (group % 10) as f64 * 512.0 + (row % 8) as f64 + offset,
                        64.0,
                        (group / 10) as f64 * 512.0 + ((row / 8) % 8) as f64,
                    ]
                } else {
                    [(i % 8) as f64, 64.0, ((i / 8) % 8) as f64]
                }
            })
            .collect();
        let chunks = if spread {
            origins[..viewers]
                .iter()
                .flat_map(|p| {
                    let (cx, cz) = (p[0] as i32 >> 4, p[2] as i32 >> 4);
                    (-10..=10).flat_map(move |x| (-10..=10).map(move |z| (cx + x, cz + z)))
                })
                .collect()
        } else {
            (-2..=2)
                .flat_map(|x| (-2..=2).map(move |z| (x, z)))
                .collect()
        };
        Self {
            level,
            entities: origins
                .iter()
                .enumerate()
                .map(|(i, &position)| EntityState {
                    id: i as i32 + 1,
                    uuid: i as u128 + 1,
                    kind: if i < viewers {
                        "minecraft:player"
                    } else {
                        "minecraft:zombie"
                    }
                    .into(),
                    position,
                    on_ground: true,
                    ..EntityState::default()
                })
                .collect(),
            trackers: (0..viewers).map(|_| Tracker::default()).collect(),
            out: vec![Vec::new(); viewers],
            chunks,
            origins,
        }
    }

    fn step(&mut self, tick: usize, send_only: bool) -> Duration {
        for (e, &origin) in self.entities.iter_mut().zip(&self.origins) {
            e.position = origin;
            e.position[0] += (tick % 64) as f64 / 32.0;
            e.yaw = (tick % 120) as f32 * 3.0;
            e.head_yaw = e.yaw;
        }
        // Full-tick samples include a receiver pairing again, as on a respawn.
        if !send_only && tick.is_multiple_of(8) {
            let index = tick % self.trackers.len();
            self.trackers[index] = Tracker::default();
        }
        for out in &mut self.out {
            out.clear();
        }
        let refs: Vec<_> = self.entities.iter().collect();
        let mut viewers: Vec<_> = self
            .trackers
            .iter_mut()
            .zip(&mut self.out)
            .enumerate()
            .map(|(i, (tracker, out))| Viewer {
                entity_id: i as i32 + 1,
                sunrise: true,
                general_entity: true,
                attachment_sender: None,
                sources: None,
                far: &[],
                spectator: false,
                permission: 0,
                moved: true,
                position: self.entities[i].position,
                view_distance: 10,
                chunks: &self.chunks,
                tracker,
                out,
            })
            .collect();
        let riding = Riding::default();
        if !send_only {
            let started = Instant::now();
            self.level
                .tick(&refs, &riding, &mut viewers, None, |_| Some(5));
            return started.elapsed();
        }
        let mut flags = std::mem::take(&mut self.level.flags);
        let mut starts = std::mem::take(&mut self.level.starts);
        let active = std::mem::take(&mut self.level.active);
        let mut players = viewer_work(&mut viewers, &mut flags, &mut starts);
        let started = Instant::now();
        if self.level.send_apart_always == Some(true) {
            self.level
                .send_on_threads(&refs, &riding, &mut players, &active, None);
        } else {
            self.level
                .send_in_order(&refs, &riding, &mut players, &active, None);
        }
        let elapsed = started.elapsed();
        drop(players);
        self.level.active = active;
        self.level.flags = flags;
        self.level.starts = starts;
        elapsed
    }

    fn events(&mut self) -> Vec<Vec<TrackEvent>> {
        self.trackers.iter_mut().map(Tracker::take_events).collect()
    }
}

#[test]
fn pooled_tracker_send_matches_serial_on_small_spread() {
    let viewers = 10;
    let unseen = viewers as i32 + 1;
    let [mut serial, mut pooled] =
        [false, true].map(|parallel| Scene::new(viewers, 130, true, parallel));
    let synced = 2 * viewers;
    let synced_id = synced as i32 + 1;
    for scene in [&mut serial, &mut pooled] {
        scene.level.synced.push((synced_id - 1, synced_id));
        scene.entities[synced].sync_velocity = true;
    }
    let mut saw_start = false;
    let mut saw_stop = false;
    for tick in 1..=18 {
        if tick == 8 || tick == 14 {
            let offset = if tick == 8 { -128.0 } else { 128.0 };
            for scene in [&mut serial, &mut pooled] {
                scene.origins[viewers][0] += offset;
            }
        }
        let send_only = (4..8).contains(&tick) || (10..14).contains(&tick);
        serial.step(tick, send_only);
        pooled.step(tick, send_only);
        assert_eq!(serial.out, pooled.out, "tick {tick}");
        let events = pooled.events();
        assert_eq!(serial.events(), events, "tick {tick}");
        for event in events.iter().flatten().filter(|e| e.entity == unseen) {
            saw_start |= event.start;
            saw_stop |= !event.start;
        }

        let serial_state = serial.level.states.get(&unseen).unwrap();
        let pooled_state = pooled.level.states.get(&unseen).unwrap();
        assert_eq!(serial_state.tick, pooled_state.tick);
        assert_eq!(serial_state.base, pooled_state.base);
        if tick == 7 {
            assert!(pooled_state.tick > 0);
            assert!(
                !pooled
                    .trackers
                    .iter()
                    .any(|tracker| tracker.is_tracking(unseen))
            );
        }
    }
    assert!(saw_start && saw_stop);
}

#[test]
#[ignore = "performance measurement; requires SUNRISE_SEND_BENCH=1 on an idle machine"]
fn tracker_send_benchmark() {
    if std::env::var("SUNRISE_SEND_BENCH").as_deref() != Ok("1") {
        eprintln!("Not measuring: explicitly set SUNRISE_SEND_BENCH=1 on an idle machine.");
        return;
    }
    assert!(
        !std::hint::black_box(cfg!(debug_assertions)),
        "build with --profile fast"
    );
    assert!(
        std::hint::black_box(cfg!(feature = "tracker-send-bench")),
        "build with --features tracker-send-bench for the game's allocator"
    );
    assert!(
        !std::env::var("SUNRISE_SERIAL_TICK").is_ok_and(|v| v.eq_ignore_ascii_case("all")),
        "SUNRISE_SERIAL_TICK=all disables the parallel path being measured"
    );
    let workers = sunrise_threads::worker_count();
    assert!(workers > 1, "set SUNRISE_WORKER_THREADS to at least 2");
    let crowds = [
        (15, 63),
        (15, 64),
        (16, 63),
        (16, 64),
        (17, 65),
        (32, 128),
        (50, 50),
        (120, 120),
        (400, 400),
        (600, 600),
    ];
    let spreads = [(50, 1_000), (50, 2_500), (50, 5_000)];
    let cases = crowds
        .into_iter()
        .map(|(viewers, entities)| (viewers, entities, false))
        .chain(
            spreads
                .into_iter()
                .map(|(viewers, entities)| (viewers, entities, true)),
        );
    println!(
        "SEND_BENCH {}",
        serde_json::json!({
            "workers": workers,
            "pool_lanes": workers.clamp(1, 8),
            "allocator": "sunrise_threads::GameAllocator",
            "threshold_viewers": SEND_APART_VIEWERS,
            "threshold_entities": SEND_APART_ENTITIES,
        })
    );
    for (viewers, entities, spread) in cases {
        let layout = if spread { "spread" } else { "crowd" };
        for send_only in [true, false] {
            for round in 0..7 {
                let mut scenes = [
                    Scene::new(viewers, entities, spread, false),
                    Scene::new(viewers, entities, spread, true),
                ];
                // Establish TRACKS flags and warm both paths before timing steady-state sends.
                for tick in 1..=10 {
                    for scene in &mut scenes {
                        scene.step(tick, false);
                    }
                    assert_eq!(scenes[0].out, scenes[1].out);
                    let [serial, parallel] = &mut scenes;
                    assert_eq!(serial.events(), parallel.events());
                }
                let mut samples: [Vec<u128>; 2] = std::array::from_fn(|_| Vec::new());
                let mut packets = 0;
                for tick in 11..=30 {
                    // Alternate which strategy runs first, including between rounds.
                    let first = (tick + round) % 2;
                    for side in [first, 1 - first] {
                        samples[side].push(scenes[side].step(tick, send_only).as_nanos());
                    }
                    assert_eq!(scenes[0].out, scenes[1].out);
                    packets += scenes[0].out.iter().map(Vec::len).sum::<usize>();
                    let [serial, parallel] = &mut scenes;
                    assert_eq!(serial.events(), parallel.events());
                }
                assert!(packets > 0, "the scene must send packets");
                println!(
                    "SEND_BENCH {}",
                    serde_json::json!({
                        "viewers": viewers, "entities": entities, "layout": layout, "round": round,
                        "phase": if send_only { "send" } else { "tick_with_pairing" },
                        "serial_ns": samples[0], "parallel_ns": samples[1], "packets": packets,
                    })
                );
            }
        }
    }
}
