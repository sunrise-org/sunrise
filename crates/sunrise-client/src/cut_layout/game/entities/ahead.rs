#![cfg(test)]

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::sync::Arc;
    use sunrise_core::{geometry::Vec3, random::RandomSource};
    use sunrise_gameplay::game::game_tests::tests::server_game;
    use sunrise_gameplay::game::server_side::ahead::{AHEAD, TAKEN, VERIFIED};
    use sunrise_gameplay_common::game::GameGameExt1;
    use sunrise_gameplay_common::game::collision_source::CollisionSourceGameExt1;
    use sunrise_items::items::ItemStack;
    use sunrise_level::chunk::Chunk;
    use sunrise_sim::parallel::{ParMapStats, take_par_map_stats, with_test_mode};

    fn with_ahead<R>(choice: Option<bool>, f: impl FnOnce() -> R) -> R {
        struct Reset(Option<bool>);
        impl Drop for Reset {
            fn drop(&mut self) {
                AHEAD.with(|a| a.set(self.0));
            }
        }
        let previous = AHEAD.with(|a| a.replace(choice));
        let _reset = Reset(previous);
        f()
    }

    /// One item tick with the moves worked out ahead leaves every item, and the level's random,
    /// as the tick that moves each item at its turn: items escaping a block (their own random),
    /// floating in water, falling, and merging into a later item before its turn (whose move
    /// is then made at its turn).
    #[test]
    #[ignore = "daily"]
    fn moves_worked_out_ahead_match_those_made_at_each_turn() {
        let mut resources = None;
        let mut game = server_game(&mut resources);
        let feet = sunrise_core::geometry::Vec3::new(8.5, 70.0, 8.5);
        game.player = Some(sunrise_sim::motion::PlayerMotion::new(game.motion, feet).unwrap());
        // `ChunkMap.prepareEntityTickingChunk`: the chunks within 2 are loaded too.
        for x in -2..=2 {
            for z in -2..=2 {
                game.world
                    .chunks
                    .insert((x, z), Arc::new(Chunk::empty(x, z)));
            }
        }
        game.update_chunk_tickets();
        assert!(game.entity_ticking_area()(8, 8));
        game.border.set_center([8.0, 8.0]).unwrap();
        game.border.set_size(16.0).unwrap();
        let registry = game.pack.assets.registry;
        let stone = registry.default_state("minecraft:stone").unwrap();
        let water = registry.default_state("minecraft:water").unwrap();
        let mut escaping = Vec::new();
        for x in 0..16 {
            for z in 0..16 {
                let (fx, fz) = (f64::from(x) + 0.5, f64::from(z) + 0.5);
                let stack = |count| ItemStack::new("minecraft:stone", count);
                match (x + z) % 5 {
                    0 => {
                        game.world.set(x, 65, z, stone).unwrap();
                        game.spawn_item(stack(1), [fx, 65.3, fz], [0.0; 3], 10);
                        escaping.push(game.dropped.last().unwrap().id);
                    }
                    1 => {
                        game.world.set(x, 65, z, water).unwrap();
                        game.spawn_item(stack(1), [fx, 65.5, fz], [0.0; 3], 10);
                    }
                    2 => {
                        game.spawn_item(stack(1), [fx, 66.0, fz], [0.0; 3], 0);
                        game.spawn_item(stack(5), [fx, 66.0, fz], [0.0; 3], 0);
                    }
                    _ => game.spawn_item(stack(1), [fx, 67.0, fz], [0.1, 0.2, -0.1], 10),
                }
            }
        }
        // Real solid-entity boxes, not a stubbed obstacle query. One item is
        // caught inside the boat; another falls onto its top this tick.
        game.server.next_entity += 1;
        let boat = sunrise_gameplay_common::game::boats_common::Boat::new(
            game.server.next_entity,
            Arc::from("oak"),
            Vec3::new(8.5, 80.0, 8.5),
            0.0,
        );
        let boat_top = boat.bounds().max_position().y;
        game.boats.push(boat);
        game.spawn_item(
            ItemStack::new("minecraft:iron_ingot", 1),
            [8.5, 80.2, 8.5],
            [0.0; 3],
            10,
        );
        let caught = game.dropped.last().unwrap().id;
        escaping.push(caught);
        game.spawn_item(
            ItemStack::new("minecraft:gold_ingot", 1),
            [8.5, 81.0, 8.5],
            [0.0, -1.0, 0.0],
            10,
        );
        let landing = game.dropped.last().unwrap().id;
        // The border cuts the box even though the centre is still inside it.
        game.spawn_item(
            ItemStack::new("minecraft:diamond", 1),
            [15.99, 80.0, 8.5],
            [0.0; 3],
            10,
        );
        let border = game.dropped.last().unwrap().id;
        escaping.push(border);
        let boxes =
            game.entity_collisions(sunrise_gameplay_common::game::collision_source::Mover::Passive);
        for id in [caught, border] {
            let item = game.dropped.iter().find(|i| i.id == id).unwrap();
            assert!(item.inside_block(
                &game.world,
                game.collision,
                &boxes,
                Some(game.border.geometry())
            ));
            assert!(!item.inside_block(&game.world, game.collision, &[], None));
        }
        for item in game.dropped.iter_mut() {
            item.ticks = 39;
        }
        game.server.ticked_entities_through = u32::MAX;
        let (items, random) = (game.dropped.clone(), game.world.random.clone());
        // Each merge-written field independently invalidates a prepared move.
        // Rejecting it must not overwrite the merge or advance the item's RNG.
        let planned_idx = items
            .iter()
            .position(|item| !escaping.contains(&item.id))
            .unwrap();
        let obstacles = sunrise_sim::entities::ObstacleGrid::new(boxes);
        with_test_mode(1, || {
            with_ahead(None, || {
                let mut moves = sunrise_gameplay::game::server_side::ahead::Moves::work_out(
                    &game,
                    &items,
                    &obstacles,
                    &sunrise_gameplay_common::game::native_ticks::TickRound::default(),
                    |_| true,
                );
                let mut unchanged = items[planned_idx].clone();
                assert!(
                    moves
                        .take(planned_idx, &mut unchanged, game.world.revision, |_| {
                            panic!("mode 1 must not run verify oracle")
                        })
                        .is_some(),
                    "target item must have a planned move before invalidation checks"
                );
                for changed in 0..3 {
                    let mut moves = sunrise_gameplay::game::server_side::ahead::Moves::work_out(
                        &game,
                        &items,
                        &obstacles,
                        &sunrise_gameplay_common::game::native_ticks::TickRound::default(),
                        |_| true,
                    );
                    let mut item = items[planned_idx].clone();
                    match changed {
                        0 => item.stack.count += 1,
                        1 => item.age += 1,
                        _ => item.pickup_delay += 1,
                    }
                    let before = format!("{item:?}");
                    TAKEN.with(|t| t.set((0, 0)));
                    assert!(
                        moves
                            .take(planned_idx, &mut item, game.world.revision, |_| {
                                panic!("invalidated move was verified")
                            })
                            .is_none()
                    );
                    assert_eq!(TAKEN.with(Cell::get), (0, 1));
                    assert_eq!(format!("{item:?}"), before);
                }
            });
        });

        struct TickRun {
            left: Vec<String>,
            world_random: String,
            taken: (usize, usize),
            verified: usize,
            stats: ParMapStats,
        }

        let mut run_tick = |mode: u8, ahead: Option<bool>| -> TickRun {
            with_test_mode(mode, || {
                with_ahead(ahead, || {
                    game.dropped = items.clone();
                    game.world.random = random.clone();
                    TAKEN.with(|t| t.set((0, 0)));
                    VERIFIED.with(|v| v.set(0));
                    let _ = take_par_map_stats();
                    game.tick_items();
                    let stats = take_par_map_stats();
                    let taken = TAKEN.with(Cell::get);
                    let verified = VERIFIED.with(Cell::get);
                    // Independently advance just the escaping items' starting randoms.
                    // Escaping items stay at their turn and consume one float on the
                    // tick thread; ordinary moves must consume none of it.
                    for item in game.dropped.iter() {
                        let before = items.iter().find(|i| i.id == item.id).unwrap();
                        let mut expected = before.random.clone();
                        if escaping.contains(&item.id) {
                            expected.next_float();
                        }
                        assert_eq!(
                            item.random.clone().next_int(),
                            expected.next_int(),
                            "mode {mode} item {}",
                            item.id
                        );
                    }
                    let landed = game.dropped.iter().find(|i| i.id == landing).unwrap();
                    assert_eq!(landed.position.y, boat_top);
                    assert!(landed.ground.on_ground);
                    let world_random = format!("{:?}", game.world.random);
                    assert_eq!(world_random, format!("{random:?}"));
                    let left: Vec<String> = game.dropped.iter().map(|i| format!("{i:?}")).collect();
                    TickRun {
                        left,
                        world_random,
                        taken,
                        verified,
                        stats,
                    }
                })
            })
        };

        // Drive serial (0), parallel (1), and verify (2) modes, plus explicit
        // switch-off above the threshold.
        let serial = run_tick(0, None);
        let parallel = run_tick(1, None);
        let verify = run_tick(2, None);
        let switched_off = run_tick(1, Some(false));

        assert!(serial.left.len() < items.len(), "some items merged");
        assert_eq!(serial.stats, ParMapStats::default());
        assert_eq!(serial.taken, (0, 0));
        assert_eq!(serial.verified, 0);

        assert_eq!(parallel.left, serial.left);
        assert_eq!(parallel.world_random, serial.world_random);
        assert_eq!(parallel.stats.serial, 0);
        assert_eq!(parallel.stats.parallel, 1);
        assert!(
            parallel.stats.worker_parts > 0,
            "parallel mode must execute parts on helper threads"
        );
        // The path under test ran: most moves were put in place, and a merge refused some.
        let (taken, refused) = parallel.taken;
        assert!(
            taken >= 200 && refused > 0,
            "{taken} put in place, {refused} refused"
        );
        assert_eq!(parallel.verified, 0);

        assert_eq!(verify.left, serial.left);
        assert_eq!(verify.world_random, serial.world_random);
        assert_eq!(verify.stats.serial, 0);
        assert_eq!(verify.stats.parallel, 1);
        assert!(
            verify.stats.worker_parts > 0,
            "verify mode must execute parts on helper threads"
        );
        assert_eq!(verify.taken, parallel.taken);
        assert_eq!(verify.verified, taken);

        assert_eq!(switched_off.left, serial.left);
        assert_eq!(switched_off.world_random, serial.world_random);
        assert_eq!(switched_off.stats, ParMapStats::default());
        assert_eq!(switched_off.taken, (0, 0));
        assert_eq!(switched_off.verified, 0);

        eprintln!(
            "item work-ahead proof: {} inputs, {} escaping; {taken} taken, {refused} invalidated, {} verified",
            items.len(),
            escaping.len(),
            verify.verified
        );
    }
}
