//! The dropped items' own moves, worked out ahead of the item loop on the tick's helper threads
//! while the parallel tick is on (`sunrise_client::parallel`).
//!
//! An item's move (`ItemEntity.tick` up to its merge) reads only itself, the level's blocks, the
//! obstacle boxes and the border, and the item loop changes none of them: so an item's move worked
//! out on a copy before the loop is the move its turn would make. Escaping from inside a block
//! draws the item's own random and stays at its turn; water splashes are staged on helpers and
//! drawn on the tick thread. A merge before its turn can change its count, age and pickup delay:
//! such an item moves at its turn instead. With a native mod loaded (its hooks see every item) or
//! a mod listening to the round, every item moves at its turn.
use super::{entity::Entity as _, native_ticks};
use sunrise_core::geometry::{Aabb, Vec3, border::BorderGeometry};
use sunrise_items::blocks::BlockFacts;
use sunrise_level::world::World;
use sunrise_sim::entities::{ItemEntity, ItemSplash, ItemStep, ObstacleGrid};
use sunrise_world::{block_collision::BlockCollisionRegistry, registry::BlockRegistry};

/// From this many items their moves go to the helper threads.
const MIN_ITEMS: usize = 256;

/// What an item's own move reads.
pub(in crate::game) struct ItemTables<
    'a,
    S = sunrise_gameplay_common::game::native_surface::GameSide<'a>,
> {
    world: &'a World,
    facts: &'a BlockFacts,
    collision: &'a BlockCollisionRegistry<'a>,
    fluids: &'a sunrise_sim::motion::FluidResources,
    blocks: &'a sunrise_world::movement_properties::BoundMovementProperties,
    fall_reset: &'a std::collections::BTreeSet<String>,
    registry: &'a BlockRegistry,
    border: &'a BorderGeometry,
    context: &'a sunrise_world::native_load_context::LoopContext,
    side: S,
}

impl<'w> crate::game::ServerGame<'w> {
    pub(in crate::game::server_side) fn item_tables(&self) -> ItemTables<'_> {
        let motion = self.common.res.motion();
        ItemTables {
            world: &self.common.world,
            facts: self.common.res.facts(),
            collision: self.common.collision,
            fluids: &motion.fluids,
            blocks: &motion.blocks,
            fall_reset: &motion.fall_reset,
            registry: self.common.pack.assets.registry,
            border: self.common.border.geometry(),
            context: &self.common.loop_context,
            side: game_side!(self),
        }
    }
}

impl<S: sunrise_sim::motion::SideLevel + Sync> ItemTables<'_, S> {
    /// `ItemEntity.tick` up to its move's vertical contact: the fluids, the water entry, the move
    /// and its fall. Splashes are staged without drawing randoms on helper threads.
    pub(in crate::game::server_side) fn move_item(
        &self,
        item: &mut ItemEntity,
        escape_speed: Option<f32>,
        entity_boxes: &[Aabb],
        splashes: &mut Vec<PendingSplash>,
    ) -> Option<ItemStep> {
        // Entity.baseTick splashes before moving, except on the first tick, at
        // the motion before the currents push it.
        let before = item.velocity;
        let heights = item.push_by_fluid(self.world, self.fluids, self.context);
        record_water_entry(item, heights.0 > 0.0, item.ticks == 0, before, splashes);
        item.lava_fall(item.ticks > 0 && heights.1 > 0.0);
        let mut step = item.tick_move_before_block_effects(
            self.world,
            self.facts,
            self.collision,
            escape_speed.is_some(),
            escape_speed,
            heights,
            entity_boxes,
            Some(self.border),
            Some(self.blocks),
            self.context,
        );
        item.check_fall(&mut step, |fall, from, moved| {
            if fall == 0.0 || moved.length_squared() < 1.0 {
                return fall;
            }
            let reset = self.fall_reset;
            let hit = sunrise_sim::entities::fall_reset_hit(
                self.world,
                self.fluids,
                |p| reset.contains(&*self.facts.get(self.world.block(p.x, p.y, p.z)).name),
                from,
                moved,
            );
            if hit { 0.0 } else { fall }
        });
        if let Some(response) = item.move_restitution(
            sunrise_sim::motion::BodyMotion {
                world: self.world,
                collision: self.collision,
                facts: self.facts,
                blocks: Some(self.blocks),
                context: self.context,
                side: None,
            },
            step,
        ) {
            // A real bounce has synchronous level effects. Leave that move to its own turn.
            if response.bounce.is_some() {
                return None;
            }
            item.velocity = response.velocity;
        }
        Some(step)
    }
    /// The rest of the move after its vertical contact: block effects, bubble columns, settling
    /// and the water entry. Whether the item lives on.
    pub(in crate::game::server_side) fn settle_item(
        &self,
        item: &mut ItemEntity,
        no_physics: bool,
        step: ItemStep,
        splashes: &mut Vec<PendingSplash>,
    ) -> bool {
        if !no_physics {
            let motion = sunrise_sim::motion::BodyMotion {
                world: self.world,
                collision: self.collision,
                facts: self.facts,
                blocks: Some(self.blocks),
                context: self.context,
                side: None,
            };
            item.apply_move_and_vanilla_step(motion, step);
        }
        item.finish_move_block_effects(self.world, self.facts, step);
        item.push_by_bubble_column(self.world, self.registry, self.facts, self.collision);
        let alive = item.tick_settle(
            self.world,
            self.registry,
            self.facts,
            step,
            self.context,
            &self.side,
        );
        // ItemEntity.tick refreshes immersion and splashes on water entry.
        let before = item.velocity;
        let (water, _) = item.push_by_fluid(self.world, self.fluids, self.context);
        record_water_entry(item, water > 0.0, false, before, splashes);
        alive
    }
}

/// Inputs for `Entity.doWaterSplashEffect`, staged on a helper so the splash RNG draws happen on
/// the tick thread when the move is taken.
pub(in crate::game::server_side) struct PendingSplash {
    position: Vec3,
    velocity: Vec3,
    ticks: u32,
}

fn record_water_entry(
    item: &mut ItemEntity,
    in_water: bool,
    first_tick: bool,
    velocity: Vec3,
    splashes: &mut Vec<PendingSplash>,
) {
    if in_water {
        item.fall_distance = 0.0;
    }
    if in_water && !item.was_touching_water && !first_tick {
        splashes.push(PendingSplash {
            position: item.position,
            velocity,
            ticks: item.ticks,
        });
    }
    item.was_touching_water = in_water;
}

fn materialize_splashes(id: u32, pending: &[PendingSplash]) -> Vec<ItemSplash> {
    let mut out = Vec::new();
    let width = ItemEntity::WIDTH as f32;
    for splash in pending {
        let mut random = sunrise_core::random::LegacyRandom::new(
            (id as i64)
                .wrapping_mul(0x5DEECE66D)
                .wrapping_add(splash.ticks as i64),
        );
        sunrise_sim::entities::water_splash_effect(
            splash.position,
            splash.velocity,
            width,
            &mut random,
            &mut |event| out.push(event),
        );
    }
    out
}

/// `getEntityCollisions(entity, bounds.expandTowards(movement))`: the box around an item that its
/// move reads others' boxes in (a block of slack for the next velocity).
pub(in crate::game::server_side) fn reach(item: &ItemEntity) -> Aabb {
    let v = item.velocity;
    item.bounds()
        .inflate(Vec3::new(v.x.abs() + 1.0, v.y.abs() + 1.0, v.z.abs() + 1.0))
}

/// An item's move as its turn makes it: without physics (inside a block), alive, discarded, and
/// its splashes.
pub(in crate::game) type Moved = (bool, bool, bool, Vec<ItemSplash>);

/// One item's move, worked out on a copy.
pub(in crate::game) struct Ahead {
    moved: ItemEntity,
    no_physics: bool,
    alive: bool,
    discarded: bool,
    pending_splashes: Vec<PendingSplash>,
    /// What a merge before its turn can change: its count, age and pickup delay.
    started: (u32, i32, i32),
}

fn started(item: &ItemEntity) -> (u32, i32, i32) {
    (item.stack.count, item.age, item.pickup_delay)
}

/// Publishes the speculative movement fields onto the authoritative row without touching
/// `attachment_lifetime` or any non-movement ownership state.
fn publish_move(live: &mut ItemEntity, moved: ItemEntity) {
    let ItemEntity {
        attachment_lifetime: _,
        rotation: _,
        join: _,
        health: _,
        fire_ticks: _,
        id: _,
        stack: _,
        thrower: _,
        target: _,
        position,
        previous,
        velocity,
        ground,
        fall_distance,
        age,
        lifespan: _,
        pickup_delay,
        bob_offset: _,
        ticks,
        stack_revision: _,
        needs_sync,
        stuck,
        was_touching_water,
        no_gravity: _,
        vertical_collision,
        random,
        size_answer: _,
    } = moved;
    live.position = position;
    live.previous = previous;
    live.velocity = velocity;
    live.ground = ground;
    live.fall_distance = fall_distance;
    live.age = age;
    live.pickup_delay = pickup_delay;
    live.ticks = ticks;
    live.needs_sync = needs_sync;
    live.stuck = stuck;
    live.was_touching_water = was_touching_water;
    live.vertical_collision = vertical_collision;
    live.random = random;
}

/// `item`'s whole move on a server with no native mod, from the tables alone.
fn move_alone<S: sunrise_sim::motion::SideLevel + Sync>(
    tables: &ItemTables<'_, S>,
    obstacles: &ObstacleGrid,
    item: &mut ItemEntity,
    found: &mut Vec<u32>,
    near: &mut Vec<Aabb>,
) -> Option<(bool, bool, bool, Vec<PendingSplash>)> {
    if item.fire_ticks != 0 {
        return None;
    }
    obstacles.near_shared(reach(item), found, near);
    let no_physics = item.inside_block(tables.world, tables.collision, near, Some(tables.border));
    // `moveTowardsClosestSpace` draws from `item.random`; leave escaping items to the tick thread.
    if no_physics {
        return None;
    }
    let mut splashes = Vec::new();
    let step = tables.move_item(item, None, near, &mut splashes)?;
    if step.vertical_contact.is_some() && item.stack.count == 0 {
        return Some((false, false, true, splashes));
    }
    // Contact effects need the owning level. The movement envelope is enough to
    // rule them out; the real sweep, when needed, tests the individual shapes.
    let area = item
        .bounds()
        .expand_towards(item.previous.subtract(item.position));
    if !sunrise_sim::block_contact::box_cells(area, |pos| {
        let state = tables.world.block(pos.x, pos.y, pos.z);
        !matches!(
            &*tables.facts.get(state).name,
            "minecraft:fire"
                | "minecraft:soul_fire"
                | "minecraft:lava_cauldron"
                | "minecraft:cactus"
        ) && !matches!(
            tables.fluids.fluid(state).kind(),
            sunrise_world::fluid::FluidKind::Lava | sunrise_world::fluid::FluidKind::FlowingLava
        )
    }) {
        return None;
    }
    let alive = tables.settle_item(item, false, step, &mut splashes);
    Some((false, alive, false, splashes))
}

#[cfg(any(test, feature = "test-support"))]
thread_local! {
    /// A test's choice of the path, over the parallel tick's switch.
    pub static AHEAD: std::cell::Cell<Option<bool>> = const { std::cell::Cell::new(None) };
    /// The moves a test's tick put in place, and those a merge made it refuse.
    pub static TAKEN: std::cell::Cell<(usize, usize)> = const { std::cell::Cell::new((0, 0)) };
    /// The moves checked by verify mode on this thread.
    pub static VERIFIED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Whether the moves are worked out ahead: while the parallel tick is on.
fn ahead() -> bool {
    #[cfg(any(test, feature = "test-support"))]
    if let Some(ahead) = AHEAD.with(std::cell::Cell::get) {
        return ahead;
    }
    sunrise_sim::parallel::lanes() > 1
}

fn is_eligible(
    is_logical_server: bool,
    item_count: usize,
    round: &native_ticks::TickRound,
) -> bool {
    ahead()
        && is_logical_server
        && item_count >= MIN_ITEMS
        && !round.listening()
        && mod_api::__private::mods().is_empty()
        && sunrise_level::collisions::piston_noclip().is_none()
        && !sunrise_world::moving_colliders::has_active_override()
}

/// The moves worked out ahead, by the item's place in the loop.
pub struct Moves {
    planned: Vec<Option<Ahead>>,
    revision: u64,
}

impl Moves {
    /// The moves of the items that will tick (`ticks`), on the helper threads; none while the
    /// parallel tick is off, for few items, or with a mod that could see an item's move.
    pub fn work_out(
        game: &crate::game::ServerGame<'_>,
        items: &[ItemEntity],
        obstacles: &ObstacleGrid,
        round: &native_ticks::TickRound,
        ticks: impl Fn(&ItemEntity) -> bool,
    ) -> Self {
        let revision = game.common.world.revision;
        if !is_eligible(game.common.is_logical_server(), items.len(), round) {
            return Self {
                planned: Vec::new(),
                revision,
            };
        }
        let todo: Vec<usize> = (0..items.len()).filter(|&i| ticks(&items[i])).collect();
        if todo.is_empty() {
            return Self {
                planned: Vec::new(),
                revision,
            };
        }
        let tables = game.item_tables();
        Self::plan_with(&tables, revision, items, &todo, obstacles)
    }

    #[cfg(test)]
    fn work_out_with<S: sunrise_sim::motion::SideLevel + Sync>(
        tables: &ItemTables<'_, S>,
        is_logical_server: bool,
        revision: u64,
        items: &[ItemEntity],
        obstacles: &ObstacleGrid,
        round: &native_ticks::TickRound,
        ticks: impl Fn(&ItemEntity) -> bool,
    ) -> Self {
        if !is_eligible(is_logical_server, items.len(), round) {
            return Self {
                planned: Vec::new(),
                revision,
            };
        }
        let todo: Vec<usize> = (0..items.len()).filter(|&i| ticks(&items[i])).collect();
        if todo.is_empty() {
            return Self {
                planned: Vec::new(),
                revision,
            };
        }
        Self::plan_with(tables, revision, items, &todo, obstacles)
    }

    fn plan_with<S: sunrise_sim::motion::SideLevel + Sync>(
        tables: &ItemTables<'_, S>,
        revision: u64,
        items: &[ItemEntity],
        todo: &[usize],
        obstacles: &ObstacleGrid,
    ) -> Self {
        let placed = sunrise_world::moving_colliders::Scope::placed();
        let done = sunrise_sim::parallel::par_map(todo, MIN_ITEMS / 4, |part, out| {
            sunrise_world::moving_colliders::in_level(placed.clone(), || {
                let (mut found, mut near) = (Vec::new(), Vec::new());
                for &i in part {
                    let mut moved = items[i].clone();
                    let Some((no_physics, alive, discarded, pending_splashes)) =
                        move_alone(tables, obstacles, &mut moved, &mut found, &mut near)
                    else {
                        continue;
                    };
                    let started = started(&items[i]);
                    out.push((
                        i,
                        Ahead {
                            moved,
                            no_physics,
                            alive,
                            discarded,
                            pending_splashes,
                            started,
                        },
                    ));
                }
            });
        });
        let mut moves: Vec<Option<Ahead>> = (0..items.len()).map(|_| None).collect();
        for (i, a) in done {
            moves[i] = Some(a);
        }
        Self {
            planned: moves,
            revision,
        }
    }
    /// Take the planned move only if the world revision and item state still match.
    pub fn take(
        &mut self,
        i: usize,
        item: &mut ItemEntity,
        revision: u64,
        again: impl FnOnce(&mut ItemEntity) -> Moved,
    ) -> Option<Moved> {
        let a = self.planned.get_mut(i)?.take()?;
        let fits = self.revision == revision && a.started == started(item);
        #[cfg(any(test, feature = "test-support"))]
        TAKEN.with(|t| {
            let (taken, refused) = t.get();
            t.set((taken + usize::from(fits), refused + usize::from(!fits)));
        });
        if !fits {
            return None;
        }
        let splashes = materialize_splashes(item.id, &a.pending_splashes);
        let got = (a.no_physics, a.alive, a.discarded, splashes);
        if sunrise_sim::parallel::parallel_tick() == 2 {
            // Run the serial oracle directly on the live row so its `attachment_lifetime` stays in place.
            let expected = again(item);
            check(item.id, item, &expected, &a.moved, &got);
            return Some(expected);
        }
        publish_move(item, a.moved);
        Some(got)
    }
}

fn vec3_bits_eq(a: Vec3, b: Vec3) -> bool {
    a.x.to_bits() == b.x.to_bits()
        && a.y.to_bits() == b.y.to_bits()
        && a.z.to_bits() == b.z.to_bits()
}

fn random_bits_eq(
    a: &sunrise_core::random::LegacyRandom,
    b: &sunrise_core::random::LegacyRandom,
) -> bool {
    let (sa, ga): (u64, Option<f64>) = a.into();
    let (sb, gb): (u64, Option<f64>) = b.into();
    sa == sb && ga.map(f64::to_bits) == gb.map(f64::to_bits)
}

fn splash_bits_eq(a: &ItemSplash, b: &ItemSplash) -> bool {
    match (a, b) {
        (
            ItemSplash::Sound {
                at: at_a,
                volume: vol_a,
                pitch: pitch_a,
            },
            ItemSplash::Sound {
                at: at_b,
                volume: vol_b,
                pitch: pitch_b,
            },
        ) => {
            at_a.map(f64::to_bits) == at_b.map(f64::to_bits)
                && vol_a.to_bits() == vol_b.to_bits()
                && pitch_a.to_bits() == pitch_b.to_bits()
        }
        (
            ItemSplash::Particle {
                bubble: b_a,
                at: at_a,
                velocity: vel_a,
            },
            ItemSplash::Particle {
                bubble: b_b,
                at: at_b,
                velocity: vel_b,
            },
        ) => {
            b_a == b_b
                && at_a.map(f64::to_bits) == at_b.map(f64::to_bits)
                && vel_a.map(f64::to_bits) == vel_b.map(f64::to_bits)
        }
        _ => false,
    }
}

fn size_answer_bits_eq(
    a: Option<sunrise_sim::pose::SizeAnswer>,
    b: Option<sunrise_sim::pose::SizeAnswer>,
) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(a), Some(b)) => {
            a.pose == b.pose
                && a.vanilla.map(f32::to_bits) == b.vanilla.map(f32::to_bits)
                && a.size.map(|s| s.map(f32::to_bits)) == b.size.map(|s| s.map(f32::to_bits))
        }
        _ => false,
    }
}

fn items_match(a: &ItemEntity, b: &ItemEntity) -> bool {
    let ItemEntity {
        attachment_lifetime: _,
        rotation: rot_a,
        join: join_a,
        health: health_a,
        fire_ticks: fire_a,
        id: id_a,
        stack: stack_a,
        thrower: thrower_a,
        target: target_a,
        position: pos_a,
        previous: prev_a,
        velocity: vel_a,
        ground: ground_a,
        fall_distance: fall_a,
        age: age_a,
        lifespan: lifespan_a,
        pickup_delay: delay_a,
        bob_offset: bob_a,
        ticks: ticks_a,
        stack_revision: rev_a,
        needs_sync: sync_a,
        stuck: stuck_a,
        was_touching_water: water_a,
        no_gravity: nograv_a,
        vertical_collision: vcol_a,
        random: rand_a,
        size_answer: size_a,
    } = a;
    let ItemEntity {
        attachment_lifetime: _,
        rotation: rot_b,
        join: join_b,
        health: health_b,
        fire_ticks: fire_b,
        id: id_b,
        stack: stack_b,
        thrower: thrower_b,
        target: target_b,
        position: pos_b,
        previous: prev_b,
        velocity: vel_b,
        ground: ground_b,
        fall_distance: fall_b,
        age: age_b,
        lifespan: lifespan_b,
        pickup_delay: delay_b,
        bob_offset: bob_b,
        ticks: ticks_b,
        stack_revision: rev_b,
        needs_sync: sync_b,
        stuck: stuck_b,
        was_touching_water: water_b,
        no_gravity: nograv_b,
        vertical_collision: vcol_b,
        random: rand_b,
        size_answer: size_b,
    } = b;
    id_a == id_b
        && join_a == join_b
        && rot_a.map(f32::to_bits) == rot_b.map(f32::to_bits)
        && health_a == health_b
        && fire_a == fire_b
        && stack_a == stack_b
        && thrower_a == thrower_b
        && target_a == target_b
        && vec3_bits_eq(*pos_a, *pos_b)
        && vec3_bits_eq(*prev_a, *prev_b)
        && vec3_bits_eq(*vel_a, *vel_b)
        && ground_a == ground_b
        && fall_a.to_bits() == fall_b.to_bits()
        && age_a == age_b
        && lifespan_a == lifespan_b
        && delay_a == delay_b
        && bob_a.to_bits() == bob_b.to_bits()
        && ticks_a == ticks_b
        && rev_a == rev_b
        && sync_a == sync_b
        && vec3_bits_eq(*stuck_a, *stuck_b)
        && water_a == water_b
        && nograv_a == nograv_b
        && vcol_a == vcol_b
        && random_bits_eq(rand_a, rand_b)
        && size_answer_bits_eq(*size_a, *size_b)
}

fn moved_match(a: &Moved, b: &Moved) -> bool {
    a.0 == b.0
        && a.1 == b.1
        && a.2 == b.2
        && a.3.len() == b.3.len()
        && a.3.iter().zip(&b.3).all(|(sa, sb)| splash_bits_eq(sa, sb))
}

/// `SUNRISE_PARALLEL_TICK=verify`: an item's move worked out ahead against the one at its turn.
fn check(id: u32, serial: &ItemEntity, expected: &Moved, ahead: &ItemEntity, got: &Moved) -> bool {
    use std::sync::atomic::{AtomicU64, Ordering};
    static CHECKED: AtomicU64 = AtomicU64::new(0);
    static DIFFERED: AtomicU64 = AtomicU64::new(0);
    #[cfg(any(test, feature = "test-support"))]
    VERIFIED.with(|v| v.set(v.get() + 1));
    let matched = items_match(serial, ahead) && moved_match(expected, got);
    if !matched {
        DIFFERED.fetch_add(1, Ordering::Relaxed);
        let message = format!(
            "parallel tick: item {id}'s move worked out ahead differs from its turn's: serial={serial:?} expected={expected:?}; ahead={ahead:?} got={got:?}"
        );
        log::error!("{message}");
        eprintln!("{message}");
        #[cfg(any(test, feature = "test-support"))]
        if sunrise_sim::parallel::verify_panics() {
            panic!("{message}");
        }
    }
    let n = CHECKED.fetch_add(1, Ordering::Relaxed) + 1;
    if n == 1 || n.is_multiple_of(10_000) {
        log::info!(
            "parallel tick: {n} item moves worked out ahead checked, {} differed",
            DIFFERED.load(Ordering::Relaxed)
        );
    }
    matched
}

#[cfg(test)]
mod tests {
    use super::*;
    use mod_api::{BlockSoundType, BlockSoundTypeIdentity, SoundEventKey};
    use std::collections::{BTreeMap, BTreeSet};
    use std::path::PathBuf;
    use std::sync::Arc;
    use sunrise_core::{
        geometry::{
            Axis, Direction, VoxelShape,
            oriented::{ColliderBoxes, Frame, OrientedCollider},
        },
        nbt::Tag,
        position::BlockPos,
        random::{LegacyRandom, RandomSource},
    };
    use sunrise_items::{
        blocks::{Fluid, RenderShape, StateFacts},
        items::ItemStack,
    };
    use sunrise_level::{chunk::Chunk, collisions::with_piston_noclip};
    use sunrise_sim::{
        motion::{FluidResources, SideLevel},
        parallel::{ParMapStats, take_par_map_stats, with_test_mode, with_verify_panic},
    };
    use sunrise_world::{
        DATA_VERSION,
        collision::CachedCollisionRegistry,
        damage_source::Catalog,
        movement_properties::{MovementProperties, MovementTags},
        moving_colliders::{self, Placed, Prepared, Shove},
        native_load_context::{DataLoadOrigin, LoadContext, LoadInputs, LoopContext, LoopSide},
        registry::BlockStateId,
    };

    fn with_ahead<R>(override_val: Option<bool>, f: impl FnOnce() -> R) -> R {
        struct Reset(Option<bool>);
        impl Drop for Reset {
            fn drop(&mut self) {
                AHEAD.with(|a| a.set(self.0));
            }
        }
        let previous = AHEAD.with(|a| a.replace(override_val));
        let _reset = Reset(previous);
        f()
    }

    fn warm_pool() {
        use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
        use std::time::{Duration, Instant};
        let active = AtomicUsize::new(0);
        let overlapped = AtomicBool::new(false);
        let deadline = Instant::now() + Duration::from_secs(5);
        let probe = [0u8; 16];
        let _ = sunrise_sim::parallel::par_map_on(&probe, 2, 2, |part, out| {
            if active.fetch_add(1, Ordering::SeqCst) >= 1 {
                overlapped.store(true, Ordering::SeqCst);
            }
            while !overlapped.load(Ordering::SeqCst) && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(1));
            }
            out.extend_from_slice(part);
            active.fetch_sub(1, Ordering::SeqCst);
        });
        let _ = take_par_map_stats();
    }

    struct TestSide {
        features: Arc<[String]>,
        damage_types: Catalog,
    }

    impl TestSide {
        fn new() -> Self {
            Self {
                features: Arc::from([]),
                damage_types: Catalog::default(),
            }
        }
    }

    impl SideLevel for TestSide {
        fn is_raining_at(&self, _: &World, _: BlockPos) -> bool {
            false
        }
        fn enabled_features(&self) -> &Arc<[String]> {
            &self.features
        }
        fn tag_items(&self) -> Option<&sunrise_items::items::Items> {
            None
        }
        fn damage_types(&self) -> &Catalog {
            &self.damage_types
        }
        fn block_entity(&self, _: &World, _: BlockPos) -> Option<mod_api::BlockEntityRef> {
            None
        }
        fn block_entity_data(
            &self,
            _: &World,
            _: &BlockRegistry,
            _: mod_api::BlockEntityRef,
        ) -> Option<mod_api::Compound> {
            None
        }
    }

    struct Fixture {
        world: World,
        registry: BlockRegistry,
        facts: BlockFacts,
        fluids: FluidResources,
        blocks: sunrise_world::movement_properties::BoundMovementProperties,
        fall_reset: BTreeSet<String>,
        border: BorderGeometry,
        context: LoopContext,
        side: TestSide,
        states: Vec<Tag>,
    }

    const AIR: BlockStateId = BlockStateId(0);
    const STONE: BlockStateId = BlockStateId(1);
    const WATER: BlockStateId = BlockStateId(2);
    const LAVA: BlockStateId = BlockStateId(3);
    const FIRE: BlockStateId = BlockStateId(4);
    const CACTUS: BlockStateId = BlockStateId(5);
    const SLIME: BlockStateId = BlockStateId(6);

    impl Fixture {
        fn new() -> Self {
            let states = vec![
                Tag::String("minecraft:air".into()),
                Tag::String("minecraft:stone".into()),
                Tag::compound([
                    ("id", Tag::String("minecraft:water".into())),
                    (
                        "properties",
                        Tag::compound([("level", Tag::String("0".into()))]),
                    ),
                ]),
                Tag::compound([
                    ("id", Tag::String("minecraft:lava".into())),
                    (
                        "properties",
                        Tag::compound([("level", Tag::String("0".into()))]),
                    ),
                ]),
                Tag::String("minecraft:fire".into()),
                Tag::String("minecraft:cactus".into()),
                Tag::String("minecraft:slime_block".into()),
            ];
            let registry = BlockRegistry::from_nbt(&Tag::compound([
                ("DataVersion", Tag::Int(DATA_VERSION)),
                ("states", Tag::List(states.clone())),
                (
                    "defaults",
                    Tag::compound([
                        ("minecraft:air", Tag::Int(0)),
                        ("minecraft:stone", Tag::Int(1)),
                        ("minecraft:water", Tag::Int(2)),
                        ("minecraft:lava", Tag::Int(3)),
                        ("minecraft:fire", Tag::Int(4)),
                        ("minecraft:cactus", Tag::Int(5)),
                        ("minecraft:slime_block", Tag::Int(6)),
                    ]),
                ),
            ]))
            .unwrap();

            let sound = SoundEventKey::parse("").unwrap();
            let state_fact = |name: &str, solid: bool, fluid: Fluid, fluid_level: u8| StateFacts {
                name: Arc::from(name),
                emission: 0,
                dynamic_light: false,
                dampening: 0,
                skylight_down: !solid,
                shape_occlusion: solid,
                can_occlude: solid,
                solid_render: solid,
                hardness: 1.0,
                requires_tool: false,
                replaceable: !solid,
                random_ticks: false,
                block_entity: false,
                render_shape: RenderShape::Model,
                destroy_overlay: solid,
                spawn_terrain_particles: solid,
                full_collision: solid,
                friction: 0.6,
                explosion_resistance: 1.0,
                item: Arc::from(name),
                pick_item: Arc::from(name),
                sounds: BlockSoundType {
                    identity: BlockSoundTypeIdentity::Other,
                    volume: 1.0,
                    pitch: 1.0,
                    break_sound: sound.clone(),
                    step: sound.clone(),
                    place: sound.clone(),
                    hit: sound.clone(),
                    fall: sound.clone(),
                },
                fluid,
                fluid_level,
                fluid_falling: false,
                waterlogged: false,
                random_offset: false,
                offset_limits: None,
                outline_offset: false,
                legacy_solid: solid,
                sturdy_full: if solid { 0x3f } else { 0 },
                classes: Arc::from([]),
                outline: Arc::from([]),
                outline_edges: Arc::from([]),
                map_color: 0,
            };
            let facts = BlockFacts {
                states: vec![
                    state_fact("minecraft:air", false, Fluid::None, 0),
                    state_fact("minecraft:stone", true, Fluid::None, 0),
                    state_fact("minecraft:water", false, Fluid::Water, 8),
                    state_fact("minecraft:lava", false, Fluid::Lava, 8),
                    state_fact("minecraft:fire", false, Fluid::None, 0),
                    state_fact("minecraft:cactus", true, Fluid::None, 0),
                    state_fact("minecraft:slime_block", true, Fluid::None, 0),
                ],
                air: AIR,
                woods: Default::default(),
                sign_texts: Default::default(),
            };

            let coeff = |bounce: f32| {
                Tag::compound([
                    ("friction", Tag::Float(0.6f32.to_bits())),
                    ("speedFactor", Tag::Float(1.0f32.to_bits())),
                    ("jumpFactor", Tag::Float(1.0f32.to_bits())),
                    ("bounceRestitution", Tag::Float(bounce.to_bits())),
                    ("fallDistanceReduction", Tag::Float(0.0f32.to_bits())),
                    ("fenceGate", Tag::Byte(0)),
                ])
            };
            let movement_tag = Tag::compound([
                ("DataVersion", Tag::Int(DATA_VERSION)),
                ("states", Tag::List(states.clone())),
                (
                    "blocks",
                    Tag::compound([
                        ("minecraft:air", coeff(0.0)),
                        ("minecraft:stone", coeff(0.0)),
                        ("minecraft:water", coeff(0.0)),
                        ("minecraft:lava", coeff(0.0)),
                        ("minecraft:fire", coeff(0.0)),
                        ("minecraft:cactus", coeff(0.0)),
                        ("minecraft:slime_block", coeff(1.0)),
                    ]),
                ),
            ]);
            let blocks = MovementProperties::from_nbt(&movement_tag, &registry)
                .unwrap()
                .bind(&MovementTags::default())
                .unwrap();
            let fluids =
                FluidResources::from_registry(&registry, &facts, &BTreeSet::new()).unwrap();

            let mut world = World::new(AIR, 42);
            for cx in -1..=1 {
                for cz in -1..=1 {
                    world
                        .chunks
                        .insert((cx, cz), Arc::new(Chunk::empty(cx, cz)));
                }
            }
            let context = LoadContext::new(
                DataLoadOrigin::ServerDataLoad,
                Arc::new(LoadInputs::new(BTreeMap::new(), PathBuf::new())),
                [],
            )
            .loop_context(LoopSide::Server, []);

            Self {
                world,
                registry,
                facts,
                fluids,
                blocks,
                fall_reset: BTreeSet::new(),
                border: BorderGeometry::new([8.0, 8.0], 16.0, 29_999_984).unwrap(),
                context,
                side: TestSide::new(),
                states,
            }
        }

        fn collision(&self) -> BlockCollisionRegistry<'_> {
            let empty = VoxelShape::empty();
            let (min, max) = empty.discrete_bounds();
            let axes = empty.cube_coordinate_axes();
            let bits = |axis| {
                Tag::LongArray(
                    empty
                        .coordinates(axis)
                        .iter()
                        .map(|n| n.to_bits() as i64)
                        .collect(),
                )
            };
            let air = Tag::compound([
                ("canonical", Tag::Byte(0)),
                ("cube", Tag::Byte(i8::from(empty.is_cube()))),
                (
                    "cube_axes",
                    Tag::Byte((axes[0] as i8) | ((axes[1] as i8) << 1) | ((axes[2] as i8) << 2)),
                ),
                ("x", bits(Axis::X)),
                ("y", bits(Axis::Y)),
                ("z", bits(Axis::Z)),
                ("occupied", Tag::ByteArray(vec![])),
                ("discrete_min", Tag::IntArray(min.to_vec())),
                ("discrete_max", Tag::IntArray(max.to_vec())),
            ]);
            let cube = Tag::compound([
                ("canonical", Tag::Byte(0)),
                ("cube", Tag::Byte(1)),
                ("cube_axes", Tag::Byte(7)),
                ("x", Tag::LongArray(vec![0, 1.0f64.to_bits() as i64])),
                ("y", Tag::LongArray(vec![0, 1.0f64.to_bits() as i64])),
                ("z", Tag::LongArray(vec![0, 1.0f64.to_bits() as i64])),
                ("occupied", Tag::ByteArray(vec![1])),
                ("discrete_min", Tag::IntArray(vec![0; 3])),
                ("discrete_max", Tag::IntArray(vec![1; 3])),
            ]);
            let cactus_shape = Tag::compound([
                ("canonical", Tag::Byte(0)),
                ("cube", Tag::Byte(0)),
                ("cube_axes", Tag::Byte(0)),
                (
                    "x",
                    Tag::LongArray(vec![
                        (1.0 / 16.0_f64).to_bits() as i64,
                        (15.0 / 16.0_f64).to_bits() as i64,
                    ]),
                ),
                (
                    "y",
                    Tag::LongArray(vec![
                        0.0_f64.to_bits() as i64,
                        (15.0 / 16.0_f64).to_bits() as i64,
                    ]),
                ),
                (
                    "z",
                    Tag::LongArray(vec![
                        (1.0 / 16.0_f64).to_bits() as i64,
                        (15.0 / 16.0_f64).to_bits() as i64,
                    ]),
                ),
                ("occupied", Tag::ByteArray(vec![1])),
                ("discrete_min", Tag::IntArray(vec![0; 3])),
                ("discrete_max", Tag::IntArray(vec![1; 3])),
            ]);
            let cached = CachedCollisionRegistry::from_nbt(
                &Tag::compound([
                    ("DataVersion", Tag::Int(DATA_VERSION)),
                    ("states", Tag::List(self.states.clone())),
                    ("shapes", Tag::List(vec![cube, air, cactus_shape])),
                    // air=1, stone=0, water=1, lava=1, fire=1, cactus=2, slime_block=0
                    ("cached_shapes", Tag::IntArray(vec![1, 0, 1, 1, 1, 2, 0])),
                ]),
                &self.registry,
            )
            .unwrap();
            BlockCollisionRegistry::new(Arc::new(cached)).unwrap()
        }

        fn tables<'a>(
            &'a self,
            collision: &'a BlockCollisionRegistry<'a>,
        ) -> ItemTables<'a, &'a TestSide> {
            ItemTables {
                world: &self.world,
                facts: &self.facts,
                collision,
                fluids: &self.fluids,
                blocks: &self.blocks,
                fall_reset: &self.fall_reset,
                registry: &self.registry,
                border: &self.border,
                context: &self.context,
                side: &self.side,
            }
        }
    }

    impl SideLevel for &TestSide {
        fn is_raining_at(&self, world: &World, pos: BlockPos) -> bool {
            TestSide::is_raining_at(self, world, pos)
        }
        fn enabled_features(&self) -> &Arc<[String]> {
            TestSide::enabled_features(self)
        }
        fn tag_items(&self) -> Option<&sunrise_items::items::Items> {
            TestSide::tag_items(self)
        }
        fn damage_types(&self) -> &Catalog {
            TestSide::damage_types(self)
        }
        fn block_entity(&self, world: &World, pos: BlockPos) -> Option<mod_api::BlockEntityRef> {
            TestSide::block_entity(self, world, pos)
        }
        fn block_entity_data(
            &self,
            world: &World,
            registry: &BlockRegistry,
            entity: mod_api::BlockEntityRef,
        ) -> Option<mod_api::Compound> {
            TestSide::block_entity_data(self, world, registry, entity)
        }
    }

    fn make_item(id: u32, pos: Vec3, vel: Vec3) -> ItemEntity {
        let mut item = ItemEntity::new(id, ItemStack::new("minecraft:stone", 1), pos, vel, 0.0);
        item.ticks = 39;
        item.random = LegacyRandom::new(i64::from(id) + 1000);
        item
    }

    fn serial_step<S: SideLevel + Sync>(
        tables: &ItemTables<'_, S>,
        obstacles: &ObstacleGrid,
        item: &mut ItemEntity,
    ) -> Moved {
        let (mut found, mut near) = (Vec::new(), Vec::new());
        obstacles.near_shared(reach(item), &mut found, &mut near);
        let no_physics =
            item.inside_block(tables.world, tables.collision, &near, Some(tables.border));
        let escape_speed = no_physics.then(|| item.random.next_float() * 0.2 + 0.1);
        let mut splashes = Vec::new();
        let before = item.velocity;
        let heights = item.push_by_fluid(tables.world, tables.fluids, tables.context);
        item.water_entry(heights.0 > 0.0, item.ticks == 0, before, &mut |s| {
            splashes.push(s)
        });
        item.lava_fall(item.ticks > 0 && heights.1 > 0.0);
        let mut step = item.tick_move_before_block_effects(
            tables.world,
            tables.facts,
            tables.collision,
            no_physics,
            escape_speed,
            heights,
            &near,
            Some(tables.border),
            Some(tables.blocks),
            tables.context,
        );
        item.check_fall(&mut step, |fall, from, moved| {
            if fall == 0.0 || moved.length_squared() < 1.0 {
                return fall;
            }
            let hit = sunrise_sim::entities::fall_reset_hit(
                tables.world,
                tables.fluids,
                |p| {
                    tables
                        .fall_reset
                        .contains(&*tables.facts.get(tables.world.block(p.x, p.y, p.z)).name)
                },
                from,
                moved,
            );
            if hit { 0.0 } else { fall }
        });
        if let Some(response) = item.move_restitution(
            sunrise_sim::motion::BodyMotion {
                world: tables.world,
                collision: tables.collision,
                facts: tables.facts,
                blocks: Some(tables.blocks),
                context: tables.context,
                side: None,
            },
            step,
        ) {
            item.velocity = response.velocity;
        }
        if step.vertical_contact.is_some() && item.stack.count == 0 {
            return (no_physics, false, true, splashes);
        }
        if !no_physics {
            let motion = sunrise_sim::motion::BodyMotion {
                world: tables.world,
                collision: tables.collision,
                facts: tables.facts,
                blocks: Some(tables.blocks),
                context: tables.context,
                side: None,
            };
            item.apply_move_and_vanilla_step(motion, step);
        }
        item.finish_move_block_effects(tables.world, tables.facts, step);
        item.push_by_bubble_column(
            tables.world,
            tables.registry,
            tables.facts,
            tables.collision,
        );
        let alive = item.tick_settle(
            tables.world,
            tables.registry,
            tables.facts,
            step,
            tables.context,
            &tables.side,
        );
        let before = item.velocity;
        let (water, _) = item.push_by_fluid(tables.world, tables.fluids, tables.context);
        item.water_entry(water > 0.0, false, before, &mut |s| splashes.push(s));
        (no_physics, alive, false, splashes)
    }

    struct CrowdIndices {
        collider_band_end: usize,
        escape_idx: usize,
        water_idx: usize,
        collider_idx: usize,
        fire_ticks_idx: usize,
        lava_idx: usize,
        fire_block_idx: usize,
        cactus_idx: usize,
        bounce_idx: usize,
    }

    fn build_crowd_fixture() -> (
        Fixture,
        Vec<ItemEntity>,
        ObstacleGrid,
        Prepared,
        CrowdIndices,
    ) {
        let mut fx = Fixture::new();
        for x in 0..16 {
            for z in 0..16 {
                fx.world.set(x, 63, z, STONE).unwrap();
            }
        }
        // Special blocks for escaping, water, and the serial fallback envelopes.
        fx.world.set(1, 65, 1, STONE).unwrap();
        fx.world.set(2, 65, 2, WATER).unwrap();
        fx.world.set(3, 65, 3, LAVA).unwrap();
        fx.world.set(4, 65, 4, FIRE).unwrap();
        fx.world.set(5, 65, 5, CACTUS).unwrap();
        fx.world.set(6, 65, 6, SLIME).unwrap();

        let mut items = Vec::new();
        let mut id = 1u32;
        // First 128 items (z in 0..8) fall onto a wide moving-collider slab at y = 74..75 so every
        // worker part encounters the forwarded moving-collider scope without overlapping y = 65..66.
        for x in 0..16 {
            for z in 0..8 {
                let pos = Vec3::new(f64::from(x) + 0.5, 75.25, f64::from(z) + 0.5);
                let vel = Vec3::new(0.0, -0.5, 0.0);
                items.push(make_item(id, pos, vel));
                id += 1;
            }
        }
        let collider_band_end = items.len();
        for x in 0..16 {
            for z in 8..16 {
                let pos = Vec3::new(f64::from(x) + 0.5, 67.5, f64::from(z) + 0.5);
                let vel = Vec3::new(0.05, -0.1, -0.05);
                items.push(make_item(id, pos, vel));
                id += 1;
            }
        }
        // Item escaping a solid block: draws item.random, so it must defer to the tick thread.
        let escape_idx = items.len();
        items.push(make_item(id, Vec3::new(1.5, 65.3, 1.5), Vec3::ZERO));
        id += 1;
        // Item entering water: stages its splash on the helper and draws splash RNG on the tick thread.
        let water_idx = items.len();
        items.push(make_item(
            id,
            Vec3::new(2.5, 65.5, 2.5),
            Vec3::new(0.0, -0.4, 0.0),
        ));
        id += 1;
        // Item falling onto a moving collider at y = 70..71.
        let collider_idx = items.len();
        items.push(make_item(
            id,
            Vec3::new(10.5, 71.25, 10.5),
            Vec3::new(0.0, -0.5, 0.0),
        ));
        id += 1;
        // Fallback items: on fire, in lava, in fire block, touching cactus, bouncing on slime.
        let fire_ticks_idx = items.len();
        let mut burning = make_item(id, Vec3::new(8.5, 67.5, 8.5), Vec3::ZERO);
        burning.fire_ticks = 20;
        items.push(burning);
        id += 1;
        let lava_idx = items.len();
        items.push(make_item(
            id,
            Vec3::new(3.5, 65.5, 3.5),
            Vec3::new(0.0, -0.1, 0.0),
        ));
        id += 1;
        let fire_block_idx = items.len();
        items.push(make_item(
            id,
            Vec3::new(4.5, 65.5, 4.5),
            Vec3::new(0.0, -0.1, 0.0),
        ));
        id += 1;
        // Starts above the inset cactus top (65.9375) so initial `inside_block` is false and the
        // physics movement envelope enters the cactus cell.
        let cactus_idx = items.len();
        items.push(make_item(
            id,
            Vec3::new(5.5, 65.98, 5.5),
            Vec3::new(0.0, -0.1, 0.0),
        ));
        id += 1;
        let bounce_idx = items.len();
        items.push(make_item(
            id,
            Vec3::new(6.5, 66.1, 6.5),
            Vec3::new(0.0, -0.8, 0.0),
        ));

        let obstacles = ObstacleGrid::new(vec![Aabb::new(
            Vec3::new(12.0, 68.0, 12.0),
            Vec3::new(13.0, 69.0, 13.0),
        )]);
        let band_boxes = ColliderBoxes::new(vec![Aabb::new(
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(16.0, 1.0, 8.0),
        )]);
        let platform_boxes = ColliderBoxes::new(vec![Aabb::new(
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(2.0, 1.0, 2.0),
        )]);
        let placed = Prepared::new(vec![
            Placed {
                id: 98_999,
                collider: OrientedCollider::new(
                    Frame::translation(Vec3::new(0.0, 74.0, 0.0)),
                    Arc::new(band_boxes),
                ),
                exempt: Vec::new(),
            },
            Placed {
                id: 99_000,
                collider: OrientedCollider::new(
                    Frame::translation(Vec3::new(9.5, 70.0, 9.5)),
                    Arc::new(platform_boxes),
                ),
                exempt: Vec::new(),
            },
        ]);
        (
            fx,
            items,
            obstacles,
            placed,
            CrowdIndices {
                collider_band_end,
                escape_idx,
                water_idx,
                collider_idx,
                fire_ticks_idx,
                lava_idx,
                fire_block_idx,
                cactus_idx,
                bounce_idx,
            },
        )
    }

    #[test]
    fn item_moves_exercise_serial_parallel_and_verify_paths_with_moving_colliders() {
        warm_pool();
        let (fx, base_items, obstacles, placed, idx) = build_crowd_fixture();
        let collision = fx.collision();
        let tables = fx.tables(&collision);
        let round = native_ticks::TickRound::default();
        let world_rng_before = format!("{:?}", fx.world.random);

        // Verify the cactus item starts outside both the inset cactus shape and moving colliders.
        moving_colliders::in_level(placed.clone(), || {
            assert!(
                !base_items[idx.cactus_idx].inside_block(
                    &fx.world,
                    &collision,
                    &[],
                    Some(&fx.border)
                ),
                "cactus fallback item must not start inside_block"
            );
        });
        // Verify the escaping item starts inside stone.
        assert!(base_items[idx.escape_idx].inside_block(
            &fx.world,
            &collision,
            &[],
            Some(&fx.border)
        ));

        let fallback_indices = [
            idx.escape_idx,
            idx.fire_ticks_idx,
            idx.lava_idx,
            idx.fire_block_idx,
            idx.cactus_idx,
            idx.bounce_idx,
        ];

        let run_mode = |mode: u8| {
            let mut items = base_items.clone();
            let mut results = Vec::with_capacity(items.len());
            let mut stats = ParMapStats::default();
            let mut taken_counts = (0, 0);
            let mut verified = 0;
            let mut fallbacks_unplanned = true;
            let mut water_planned = false;
            moving_colliders::in_level(placed.clone(), || {
                with_ahead(None, || {
                    with_test_mode(mode, || {
                        TAKEN.with(|t| t.set((0, 0)));
                        VERIFIED.with(|v| v.set(0));
                        let _ = take_par_map_stats();
                        let mut moves = Moves::work_out_with(
                            &tables,
                            true,
                            fx.world.revision,
                            &items,
                            &obstacles,
                            &round,
                            |_| true,
                        );
                        stats = take_par_map_stats();
                        if mode > 0 {
                            fallbacks_unplanned =
                                fallback_indices.iter().all(|&i| moves.planned[i].is_none());
                            water_planned = moves.planned[idx.water_idx].is_some();
                        }
                        for (i, item) in items.iter_mut().enumerate() {
                            let moved = match moves.take(i, item, fx.world.revision, |it| {
                                serial_step(&tables, &obstacles, it)
                            }) {
                                Some(moved) => moved,
                                None => serial_step(&tables, &obstacles, item),
                            };
                            results.push(moved);
                        }
                        taken_counts = TAKEN.with(std::cell::Cell::get);
                        verified = VERIFIED.with(std::cell::Cell::get);
                    });
                });
            });
            (
                items,
                results,
                stats,
                taken_counts,
                verified,
                fallbacks_unplanned,
                water_planned,
            )
        };

        let (serial_items, serial_results, serial_stats, serial_taken, serial_verified, _, _) =
            run_mode(0);
        assert_eq!(serial_stats, ParMapStats::default());
        assert_eq!(serial_taken, (0, 0));
        assert_eq!(serial_verified, 0);
        assert!(!serial_results[idx.water_idx].3.is_empty());
        assert_eq!(serial_items[idx.collider_idx].position.y, 71.0);
        assert!(serial_items[idx.collider_idx].ground.on_ground);
        for item in &serial_items[..idx.collider_band_end] {
            assert_eq!(item.position.y, 75.0);
            assert!(item.ground.on_ground);
        }

        for mode in [1u8, 2u8] {
            let (items, results, stats, taken, verified, fallbacks_unplanned, water_planned) =
                run_mode(mode);
            assert_eq!(stats.serial, 0, "mode {mode}");
            assert_eq!(stats.parallel, 1, "mode {mode}");
            assert!(
                stats.worker_parts > 0,
                "mode {mode}: helper threads must execute parts"
            );
            assert!(
                fallbacks_unplanned,
                "mode {mode}: fallbacks must stay serial"
            );
            assert!(water_planned, "mode {mode}: water entry stages splash");
            assert_eq!(taken.0, base_items.len() - fallback_indices.len());
            assert_eq!(taken.1, 0);
            assert_eq!(verified, if mode == 2 { taken.0 } else { 0 });
            for item in &items[..idx.collider_band_end] {
                assert_eq!(item.position.y, 75.0);
                assert!(item.ground.on_ground);
            }
            assert_eq!(items[idx.collider_idx].position.y, 71.0);
            assert!(items[idx.collider_idx].ground.on_ground);
            // Bit-exact item state, per-item RNG, splashes, and level RNG parity.
            for i in 0..items.len() {
                assert!(
                    items_match(&serial_items[i], &items[i]),
                    "item {i} mode {mode}"
                );
                assert!(
                    moved_match(&serial_results[i], &results[i]),
                    "moved {i} mode {mode}"
                );
                let mut expected_rng = base_items[i].random.clone();
                if i == idx.escape_idx {
                    expected_rng.next_float();
                }
                assert!(
                    random_bits_eq(&items[i].random, &expected_rng),
                    "item {i} RNG mode {mode}"
                );
            }
            assert_eq!(format!("{:?}", fx.world.random), world_rng_before);
        }
    }

    #[test]
    fn item_moves_preserve_bound_attachment_lifetime_across_all_paths() {
        use std::sync::{
            Mutex,
            atomic::{AtomicUsize, Ordering},
        };
        use sunrise_mod_runtime::attachments::{
            AttachmentType, Attachments, Holder, index::AttachmentIndex,
        };

        #[derive(Clone)]
        struct DropProbe(Arc<AtomicUsize>);
        impl Drop for DropProbe {
            fn drop(&mut self) {
                self.0.fetch_add(1, Ordering::SeqCst);
            }
        }
        static PROBE: AttachmentType<DropProbe> = AttachmentType {
            namespace: "test",
            name: "ahead_item_lifetime",
            default: || panic!("no default"),
            codec: None,
            copy_on_death: false,
            sync_codec: None,
        };

        warm_pool();
        let (fx, mut items, obstacles, placed, _) = build_crowd_fixture();
        let collision = fx.collision();
        let tables = fx.tables(&collision);
        let round = native_ticks::TickRound::default();

        let index = Arc::new(Mutex::new(AttachmentIndex::<u32>::default()));
        let drops = Arc::new(AtomicUsize::new(0));
        let key = ("test".into(), "ahead_item_lifetime".into());
        let item_id = items[0].id;
        {
            let mut map = Attachments::default();
            PROBE
                .set(&mut map, Holder::Entity(item_id), DropProbe(drops.clone()))
                .unwrap();
            items[0].attachment_lifetime.bind(&mut map, &index, item_id);
        }
        assert!(items[0].attachment_lifetime.is_bound());
        assert_eq!(index.lock().unwrap().snapshot(&key).len(), 1);
        assert_eq!(drops.load(Ordering::SeqCst), 0);

        let check_live = |item: &ItemEntity, phase: &str| {
            assert!(
                item.attachment_lifetime.is_bound(),
                "{phase}: lifetime unbound"
            );
            let snap = index.lock().unwrap().snapshot(&key);
            assert_eq!(snap.len(), 1, "{phase}: registration retired");
            assert_eq!(snap[0].holder, item_id, "{phase}: wrong holder");
            assert_eq!(drops.load(Ordering::SeqCst), 0, "{phase}: probe dropped");
        };

        let reset_drop = |item: &mut ItemEntity| {
            item.position = Vec3::new(0.5, 75.25, 0.5);
            item.previous = item.position;
            item.velocity = Vec3::new(0.0, -0.5, 0.0);
            item.ground = Default::default();
        };

        // 1. Serial path.
        reset_drop(&mut items[0]);
        moving_colliders::in_level(placed.clone(), || {
            with_ahead(None, || {
                with_test_mode(0, || {
                    let mut moves = Moves::work_out_with(
                        &tables,
                        true,
                        fx.world.revision,
                        &items,
                        &obstacles,
                        &round,
                        |_| true,
                    );
                    assert!(
                        moves
                            .take(0, &mut items[0], fx.world.revision, |it| {
                                serial_step(&tables, &obstacles, it)
                            })
                            .is_none()
                    );
                    let _ = serial_step(&tables, &obstacles, &mut items[0]);
                });
            });
        });
        assert_eq!(items[0].position.y, 75.0);
        check_live(&items[0], "serial");

        // 2. Parallel publication path.
        reset_drop(&mut items[0]);
        moving_colliders::in_level(placed.clone(), || {
            with_ahead(None, || {
                with_test_mode(1, || {
                    let _ = take_par_map_stats();
                    let mut moves = Moves::work_out_with(
                        &tables,
                        true,
                        fx.world.revision,
                        &items,
                        &obstacles,
                        &round,
                        |_| true,
                    );
                    let stats = take_par_map_stats();
                    assert_eq!(stats.parallel, 1);
                    assert!(stats.worker_parts > 0);
                    assert!(
                        moves
                            .take(0, &mut items[0], fx.world.revision, |it| {
                                serial_step(&tables, &obstacles, it)
                            })
                            .is_some()
                    );
                });
            });
        });
        assert_eq!(items[0].position.y, 75.0);
        check_live(&items[0], "parallel");

        // 3. Verify-match path.
        reset_drop(&mut items[0]);
        moving_colliders::in_level(placed.clone(), || {
            with_ahead(None, || {
                with_test_mode(2, || {
                    VERIFIED.with(|v| v.set(0));
                    let mut moves = Moves::work_out_with(
                        &tables,
                        true,
                        fx.world.revision,
                        &items,
                        &obstacles,
                        &round,
                        |_| true,
                    );
                    assert!(
                        moves
                            .take(0, &mut items[0], fx.world.revision, |it| {
                                serial_step(&tables, &obstacles, it)
                            })
                            .is_some()
                    );
                    assert_eq!(VERIFIED.with(std::cell::Cell::get), 1);
                });
            });
        });
        assert_eq!(items[0].position.y, 75.0);
        check_live(&items[0], "verify match");

        // 4. Verify-mismatch recoverable fallback path.
        reset_drop(&mut items[0]);
        let mut expected_item = items[0].clone();
        moving_colliders::in_level(placed.clone(), || {
            let _ = serial_step(&tables, &obstacles, &mut expected_item);
            with_ahead(None, || {
                with_test_mode(2, || {
                    with_verify_panic(false, || {
                        VERIFIED.with(|v| v.set(0));
                        let mut moves = Moves::work_out_with(
                            &tables,
                            true,
                            fx.world.revision,
                            &items,
                            &obstacles,
                            &round,
                            |_| true,
                        );
                        moves.planned[0].as_mut().unwrap().moved.position.y += 5.0;
                        assert!(
                            moves
                                .take(0, &mut items[0], fx.world.revision, |it| {
                                    serial_step(&tables, &obstacles, it)
                                })
                                .is_some()
                        );
                        assert_eq!(VERIFIED.with(std::cell::Cell::get), 1);
                    });
                });
            });
        });
        assert!(items_match(&items[0], &expected_item));
        check_live(&items[0], "verify mismatch fallback");

        // Dropping the real item retires its registration exactly once.
        drop(items);
        assert!(index.lock().unwrap().snapshot(&key).is_empty());
        assert_eq!(drops.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn item_moves_fallbacks_preserve_revision_merge_listener_mod_and_shove() {
        let (fx, base_items, obstacles, placed, _) = build_crowd_fixture();
        let collision = fx.collision();
        let tables = fx.tables(&collision);
        let round = native_ticks::TickRound::default();

        moving_colliders::in_level(placed, || {
            with_ahead(None, || {
                with_test_mode(1, || {
                    // Revision change invalidates planned moves.
                    let mut moves = Moves::work_out_with(
                        &tables,
                        true,
                        fx.world.revision,
                        &base_items,
                        &obstacles,
                        &round,
                        |_| true,
                    );
                    let mut item = base_items[0].clone();
                    assert!(
                        moves
                            .take(0, &mut item, fx.world.revision + 1, |_| {
                                panic!("invalidated move must not verify")
                            })
                            .is_none()
                    );

                    // Merge-written fields invalidate planned moves.
                    for field in 0..3 {
                        let mut moves = Moves::work_out_with(
                            &tables,
                            true,
                            fx.world.revision,
                            &base_items,
                            &obstacles,
                            &round,
                            |_| true,
                        );
                        let mut item = base_items[0].clone();
                        match field {
                            0 => item.stack.count += 1,
                            1 => item.age += 1,
                            _ => item.pickup_delay += 1,
                        }
                        assert!(
                            moves
                                .take(0, &mut item, fx.world.revision, |_| {
                                    panic!("merged item must not verify")
                                })
                                .is_none()
                        );
                    }

                    // Active listener falls back to serial.
                    let listening = native_ticks::TickRound {
                        told: native_ticks::Told {
                            pre: true,
                            post: false,
                        },
                        ..Default::default()
                    };
                    let _ = take_par_map_stats();
                    let moves = Moves::work_out_with(
                        &tables,
                        true,
                        fx.world.revision,
                        &base_items,
                        &obstacles,
                        &listening,
                        |_| true,
                    );
                    assert!(moves.planned.is_empty());
                    assert_eq!(take_par_map_stats(), ParMapStats::default());

                    // Active moving-collider shove override falls back to serial.
                    {
                        let _shove = Shove::enter(1, 99_000);
                        let _ = take_par_map_stats();
                        let moves = Moves::work_out_with(
                            &tables,
                            true,
                            fx.world.revision,
                            &base_items,
                            &obstacles,
                            &round,
                            |_| true,
                        );
                        assert!(moves.planned.is_empty());
                        assert_eq!(take_par_map_stats(), ParMapStats::default());
                    }

                    // Any loaded mod falls back to serial.
                    struct DummyMod;
                    #[mod_api::__subscriptions]
                    impl mod_api::Hooks for DummyMod {}
                    let mods: mod_api::Mods =
                        vec![mod_api::ModEntry::new("ahead_fallback", Box::new(DummyMod))].into();
                    mod_platform::testing::with_mods(mods, || {
                        let _ = take_par_map_stats();
                        let moves = Moves::work_out_with(
                            &tables,
                            true,
                            fx.world.revision,
                            &base_items,
                            &obstacles,
                            &round,
                            |_| true,
                        );
                        assert!(moves.planned.is_empty());
                        assert_eq!(take_par_map_stats(), ParMapStats::default());
                    });
                });
            });
        });
    }

    #[test]
    fn item_moves_fall_back_to_serial_under_piston_noclip_scope() {
        let (fx, base_items, obstacles, _, _) = build_crowd_fixture();
        let collision = fx.collision();
        let tables = fx.tables(&collision);
        let round = native_ticks::TickRound::default();

        for mode in [0u8, 1, 2] {
            with_piston_noclip(Direction::East, || {
                with_ahead(None, || {
                    with_test_mode(mode, || {
                        let _ = take_par_map_stats();
                        let mut moves = Moves::work_out_with(
                            &tables,
                            true,
                            fx.world.revision,
                            &base_items,
                            &obstacles,
                            &round,
                            |_| true,
                        );
                        assert!(moves.planned.is_empty(), "mode {mode}");
                        assert_eq!(take_par_map_stats(), ParMapStats::default(), "mode {mode}");
                        let mut item = base_items[0].clone();
                        assert!(
                            moves
                                .take(0, &mut item, fx.world.revision, |_| {
                                    unreachable!("empty plan")
                                })
                                .is_none()
                        );
                        let _ = serial_step(&tables, &obstacles, &mut item);
                        assert_eq!(item.ticks, base_items[0].ticks + 1);
                    });
                });
            });
        }
    }

    #[test]
    fn item_moves_verify_panics_on_mismatch() {
        let (fx, base_items, obstacles, _, _) = build_crowd_fixture();
        let collision = fx.collision();
        let tables = fx.tables(&collision);
        let round = native_ticks::TickRound::default();

        let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            with_ahead(None, || {
                with_test_mode(2, || {
                    let mut moves = Moves::work_out_with(
                        &tables,
                        true,
                        fx.world.revision,
                        &base_items,
                        &obstacles,
                        &round,
                        |_| true,
                    );
                    let planned = moves.planned[0].as_mut().expect("item 0 planned");
                    planned.moved.position.y += 1.0;
                    let mut item = base_items[0].clone();
                    let _ = moves.take(0, &mut item, fx.world.revision, |it| {
                        serial_step(&tables, &obstacles, it)
                    });
                });
            });
        }));
        let payload = caught.expect_err("verify mode must fail hard on an item move mismatch");
        let message = payload
            .downcast_ref::<String>()
            .map(String::as_str)
            .or_else(|| payload.downcast_ref::<&str>().copied())
            .unwrap_or("");
        assert!(
            message.contains("item 1") && message.contains("serial=") && message.contains("ahead="),
            "unexpected panic message: {message}"
        );
    }

    #[test]
    fn item_moves_verify_recovers_when_non_panicking() {
        let (fx, base_items, obstacles, _, _) = build_crowd_fixture();
        let collision = fx.collision();
        let tables = fx.tables(&collision);
        let round = native_ticks::TickRound::default();

        let mut expected_item = base_items[0].clone();
        let expected_moved = serial_step(&tables, &obstacles, &mut expected_item);

        with_ahead(None, || {
            with_test_mode(2, || {
                with_verify_panic(false, || {
                    VERIFIED.with(|v| v.set(0));
                    let mut moves = Moves::work_out_with(
                        &tables,
                        true,
                        fx.world.revision,
                        &base_items,
                        &obstacles,
                        &round,
                        |_| true,
                    );
                    let planned = moves.planned[0].as_mut().expect("item 0 planned");
                    planned.moved.position.y += 1.0;
                    let mut item = base_items[0].clone();
                    let got = moves
                        .take(0, &mut item, fx.world.revision, |it| {
                            serial_step(&tables, &obstacles, it)
                        })
                        .expect("verify recovery returns serial result");
                    assert_eq!(VERIFIED.with(std::cell::Cell::get), 1);
                    assert!(items_match(&item, &expected_item));
                    assert!(moved_match(&got, &expected_moved));
                });
            });
        });
    }
}
