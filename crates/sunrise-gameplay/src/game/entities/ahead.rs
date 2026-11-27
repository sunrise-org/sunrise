//! The dropped items' own moves, worked out ahead of the item loop on the tick's helper threads
//! while the parallel tick is on (`sunrise_client::parallel`).
//!
//! An item's move (`ItemEntity.tick` up to its merge) reads only itself, the level's blocks, the
//! obstacle boxes and the border, and the item loop changes none of them: so an item's move worked
//! out on a copy before the loop is the move its turn would make. Its escape from inside a block
//! draws the item's own random, which the copy carries. A merge before its turn can change its
//! count, age and pickup delay: such an item moves at its turn instead. With a native mod loaded
//! (its hooks see every item) or a mod listening to the round, every item moves at its turn.
use super::*;
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
    /// `ItemEntity.tick` up to its move's vertical contact: the fluids, the water splash, the move
    /// (pushed out at `escape_speed` when inside a block) and its fall.
    pub(in crate::game::server_side) fn move_item(
        &self,
        item: &mut ItemEntity,
        escape_speed: Option<f32>,
        entity_boxes: &[Aabb],
        splash: &mut impl FnMut(ItemSplash),
    ) -> Option<ItemStep> {
        // Entity.baseTick splashes before moving, except on the first tick, at
        // the motion before the currents push it.
        let before = item.velocity;
        let heights = item.push_by_fluid(self.world, self.fluids, self.context);
        item.water_entry(heights.0 > 0.0, item.ticks == 0, before, splash);
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
        splash: &mut impl FnMut(ItemSplash),
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
        item.water_entry(water > 0.0, false, before, splash);
        alive
    }
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
    result: Moved,
    /// What a merge before its turn can change: its count, age and pickup delay.
    started: (u32, i32, i32),
}

fn started(item: &ItemEntity) -> (u32, i32, i32) {
    (item.stack.count, item.age, item.pickup_delay)
}

/// `item`'s whole move on a server with no native mod, from the tables alone.
fn move_alone<S: sunrise_sim::motion::SideLevel + Sync>(
    tables: &ItemTables<'_, S>,
    obstacles: &ObstacleGrid,
    item: &mut ItemEntity,
    found: &mut Vec<u32>,
    near: &mut Vec<Aabb>,
) -> Option<Moved> {
    if item.fire_ticks != 0 {
        return None;
    }
    obstacles.near_shared(reach(item), found, near);
    let no_physics = item.inside_block(tables.world, tables.collision, near, Some(tables.border));
    // `moveTowardsClosestSpace` draws the speed from the item's own random.
    let escape_speed = no_physics.then(|| item.random.next_float() * 0.2 + 0.1);
    let mut splashes = Vec::new();
    let step = tables.move_item(item, escape_speed, near, &mut |s| splashes.push(s))?;
    if step.vertical_contact.is_some() && item.stack.count == 0 {
        return Some((no_physics, false, true, splashes));
    }
    // Contact effects need the owning level. The movement envelope is enough to
    // rule them out; the real sweep, when needed, tests the individual shapes.
    if !no_physics {
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
                sunrise_world::fluid::FluidKind::Lava
                    | sunrise_world::fluid::FluidKind::FlowingLava
            )
        }) {
            return None;
        }
    }
    let alive = tables.settle_item(item, no_physics, step, &mut |s| splashes.push(s));
    Some((no_physics, alive, false, splashes))
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
        let tables = game.item_tables();
        Self::work_out_with(
            &tables,
            game.common.is_logical_server(),
            game.common.world.revision,
            items,
            obstacles,
            round,
            ticks,
        )
    }

    fn work_out_with<S: sunrise_sim::motion::SideLevel + Sync>(
        tables: &ItemTables<'_, S>,
        is_logical_server: bool,
        revision: u64,
        items: &[ItemEntity],
        obstacles: &ObstacleGrid,
        round: &native_ticks::TickRound,
        ticks: impl Fn(&ItemEntity) -> bool,
    ) -> Self {
        if !ahead() {
            return Self {
                planned: Vec::new(),
                revision,
            };
        }
        assert!(
            sunrise_level::collisions::piston_noclip().is_none(),
            "ahead::Moves::work_out cannot run with a piston noclip scope active"
        );
        if !is_logical_server
            || items.len() < MIN_ITEMS
            || round.listening()
            || !mod_api::__private::mods().is_empty()
            || sunrise_world::moving_colliders::has_active_override()
        {
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
        let placed = sunrise_world::moving_colliders::Scope::placed();
        let done = sunrise_sim::parallel::par_map(&todo, MIN_ITEMS / 4, |part, out| {
            sunrise_world::moving_colliders::in_level(placed.clone(), || {
                let (mut found, mut near) = (Vec::new(), Vec::new());
                for &i in part {
                    let mut moved = items[i].clone();
                    let Some(result) =
                        move_alone(tables, obstacles, &mut moved, &mut found, &mut near)
                    else {
                        continue;
                    };
                    let started = started(&items[i]);
                    out.push((
                        i,
                        Ahead {
                            moved,
                            result,
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
        if sunrise_sim::parallel::parallel_tick() == 2 {
            let mut serial = item.clone();
            let expected = again(&mut serial);
            if !check(item.id, &serial, &expected, &a.moved, &a.result) {
                *item = serial;
                return Some(expected);
            }
        }
        *item = a.moved;
        Some(a.result)
    }
}

/// `SUNRISE_PARALLEL_TICK=verify`: an item's move worked out ahead against the one at its turn.
fn check(id: u32, serial: &ItemEntity, expected: &Moved, ahead: &ItemEntity, got: &Moved) -> bool {
    use std::sync::atomic::{AtomicU64, Ordering};
    static CHECKED: AtomicU64 = AtomicU64::new(0);
    static DIFFERED: AtomicU64 = AtomicU64::new(0);
    #[cfg(any(test, feature = "test-support"))]
    VERIFIED.with(|v| v.set(v.get() + 1));
    let matched = format!("{serial:?} {expected:?}") == format!("{ahead:?} {got:?}");
    if !matched {
        DIFFERED.fetch_add(1, Ordering::Relaxed);
        let message = format!(
            "parallel tick: item {id}'s move worked out ahead differs from its turn's: serial={serial:?} expected={expected:?}; ahead={ahead:?} got={got:?}"
        );
        log::error!("{message}");
        eprintln!("{message}");
        #[cfg(any(test, feature = "test-support"))]
        panic!("{message}");
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
        random::LegacyRandom,
    };
    use sunrise_items::{
        blocks::{Fluid, RenderShape, StateFacts},
        items::ItemStack,
    };
    use sunrise_level::{chunk::Chunk, collisions::with_piston_noclip};
    use sunrise_sim::{
        motion::{FluidResources, SideLevel},
        parallel::{ParMapStats, take_par_map_stats, with_test_mode},
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
            let cached = CachedCollisionRegistry::from_nbt(
                &Tag::compound([
                    ("DataVersion", Tag::Int(DATA_VERSION)),
                    ("states", Tag::List(self.states.clone())),
                    ("shapes", Tag::List(vec![cube, air])),
                    // air=1, stone=0, water=1, lava=1, fire=1, cactus=1, slime_block=0
                    ("cached_shapes", Tag::IntArray(vec![1, 0, 1, 1, 1, 1, 0])),
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
            (*self).is_raining_at(world, pos)
        }
        fn enabled_features(&self) -> &Arc<[String]> {
            (*self).enabled_features()
        }
        fn tag_items(&self) -> Option<&sunrise_items::items::Items> {
            (*self).tag_items()
        }
        fn damage_types(&self) -> &Catalog {
            (*self).damage_types()
        }
        fn block_entity(&self, world: &World, pos: BlockPos) -> Option<mod_api::BlockEntityRef> {
            (*self).block_entity(world, pos)
        }
        fn block_entity_data(
            &self,
            world: &World,
            registry: &BlockRegistry,
            entity: mod_api::BlockEntityRef,
        ) -> Option<mod_api::Compound> {
            (*self).block_entity_data(world, registry, entity)
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
        let step = tables
            .move_item(item, escape_speed, &near, &mut |s| splashes.push(s))
            .expect("serial_step only runs on non-bouncing items");
        if step.vertical_contact.is_some() && item.stack.count == 0 {
            return (no_physics, false, true, splashes);
        }
        let alive = tables.settle_item(item, no_physics, step, &mut |s| splashes.push(s));
        (no_physics, alive, false, splashes)
    }

    fn build_crowd_fixture() -> (Fixture, Vec<ItemEntity>, ObstacleGrid, Prepared, [usize; 6]) {
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
        for x in 0..16 {
            for z in 0..16 {
                let pos = Vec3::new(f64::from(x) + 0.5, 67.5, f64::from(z) + 0.5);
                let vel = Vec3::new(0.05, -0.1, -0.05);
                items.push(make_item(id, pos, vel));
                id += 1;
            }
        }
        // Item escaping a solid block (uses its own random).
        items.push(make_item(id, Vec3::new(1.5, 65.3, 1.5), Vec3::ZERO));
        id += 1;
        // Item entering water (records a splash).
        items.push(make_item(
            id,
            Vec3::new(2.5, 65.5, 2.5),
            Vec3::new(0.0, -0.4, 0.0),
        ));
        id += 1;
        // Item falling onto a moving collider at y = 70..71 (must be forwarded to helpers).
        let collider_idx = items.len();
        items.push(make_item(
            id,
            Vec3::new(10.5, 71.2, 10.5),
            Vec3::new(0.0, -0.5, 0.0),
        ));
        id += 1;
        // Fallback items: on fire, in lava, in fire block, touching cactus, bouncing on slime.
        let fire_ticks_idx = items.len();
        let mut burning = make_item(id, Vec3::new(8.5, 67.0, 8.5), Vec3::ZERO);
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
        let cactus_idx = items.len();
        items.push(make_item(
            id,
            Vec3::new(5.5, 65.5, 5.5),
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
        let collider_boxes = ColliderBoxes::new(vec![Aabb::new(
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(2.0, 1.0, 2.0),
        )]);
        let placed = Prepared::new(vec![Placed {
            id: 99_000,
            collider: OrientedCollider::new(
                Frame::translation(Vec3::new(9.5, 70.0, 9.5)),
                Arc::new(collider_boxes),
            ),
            exempt: Vec::new(),
        }]);
        (
            fx,
            items,
            obstacles,
            placed,
            [
                collider_idx,
                fire_ticks_idx,
                lava_idx,
                fire_block_idx,
                cactus_idx,
                bounce_idx,
            ],
        )
    }

    #[test]
    fn item_moves_exercise_serial_parallel_and_verify_paths_with_moving_colliders() {
        let (fx, base_items, obstacles, placed, indices) = build_crowd_fixture();
        let [
            collider_idx,
            fire_ticks_idx,
            lava_idx,
            fire_block_idx,
            cactus_idx,
            bounce_idx,
        ] = indices;
        let fallback_indices = [
            fire_ticks_idx,
            lava_idx,
            fire_block_idx,
            cactus_idx,
            bounce_idx,
        ];
        let collision = fx.collision();
        let tables = fx.tables(&collision);
        let round = native_ticks::TickRound::default();

        let run_mode = |mode: u8| {
            let mut items = base_items.clone();
            let mut results = Vec::with_capacity(items.len());
            let mut stats = ParMapStats::default();
            let mut taken_counts = (0, 0);
            let mut verified = 0;
            let mut fallbacks_unplanned = true;
            moving_colliders::in_level(placed.clone(), || {
                with_test_mode(mode, || {
                    AHEAD.with(|a| a.set(None));
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
                        fallbacks_unplanned = fallback_indices
                            .iter()
                            .all(|&idx| moves.planned[idx].is_none());
                    }
                    for (i, item) in items.iter_mut().enumerate() {
                        if fallback_indices.contains(&i) {
                            assert!(moves.planned.get(i).and_then(Option::as_ref).is_none());
                            continue;
                        }
                        let moved = match moves.take(i, item, fx.world.revision, |it| {
                            serial_step(&tables, &obstacles, it)
                        }) {
                            Some(moved) => moved,
                            None => serial_step(&tables, &obstacles, item),
                        };
                        results.push(format!("{item:?} {moved:?}"));
                    }
                    taken_counts = TAKEN.with(std::cell::Cell::get);
                    verified = VERIFIED.with(std::cell::Cell::get);
                });
            });
            (
                items,
                results,
                stats,
                taken_counts,
                verified,
                fallbacks_unplanned,
            )
        };

        let (serial_items, serial_results, serial_stats, serial_taken, serial_verified, _) =
            run_mode(0);
        assert_eq!(
            serial_stats,
            ParMapStats {
                serial: 0,
                parallel: 0,
            }
        );
        assert_eq!(serial_taken, (0, 0));
        assert_eq!(serial_verified, 0);
        assert_eq!(serial_items[collider_idx].position.y, 71.0);
        assert!(serial_items[collider_idx].ground.on_ground);

        let (
            parallel_items,
            parallel_results,
            parallel_stats,
            parallel_taken,
            parallel_verified,
            parallel_fallbacks,
        ) = run_mode(1);
        assert_eq!(
            parallel_stats,
            ParMapStats {
                serial: 0,
                parallel: 1,
            },
            "parallel path must run on the pool"
        );
        assert!(parallel_fallbacks, "every fallback item must stay serial");
        assert_eq!(parallel_taken.0, base_items.len() - fallback_indices.len());
        assert_eq!(parallel_taken.1, 0);
        assert_eq!(parallel_verified, 0);
        assert_eq!(parallel_items[collider_idx].position.y, 71.0);
        assert!(parallel_items[collider_idx].ground.on_ground);
        assert_eq!(parallel_results, serial_results);

        let (
            verify_items,
            verify_results,
            verify_stats,
            verify_taken,
            verify_verified,
            verify_fallbacks,
        ) = run_mode(2);
        assert_eq!(
            verify_stats,
            ParMapStats {
                serial: 0,
                parallel: 1,
            },
            "verify path must run on the pool"
        );
        assert!(verify_fallbacks);
        assert_eq!(verify_taken.0, base_items.len() - fallback_indices.len());
        assert_eq!(verify_taken.1, 0);
        assert_eq!(verify_verified, verify_taken.0);
        assert_eq!(verify_items[collider_idx].position.y, 71.0);
        assert_eq!(verify_results, serial_results);
    }

    #[test]
    fn item_moves_fallbacks_preserve_revision_merge_listener_mod_and_shove() {
        let (fx, base_items, obstacles, placed, _) = build_crowd_fixture();
        let collision = fx.collision();
        let tables = fx.tables(&collision);
        let round = native_ticks::TickRound::default();

        moving_colliders::in_level(placed, || {
            with_test_mode(1, || {
                AHEAD.with(|a| a.set(None));
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
    }

    #[test]
    fn item_moves_refuse_piston_noclip_scope() {
        let (fx, base_items, obstacles, _, _) = build_crowd_fixture();
        let collision = fx.collision();
        let tables = fx.tables(&collision);
        let round = native_ticks::TickRound::default();

        // Serial mode does not run work-ahead, so it returns empty without panicking.
        with_piston_noclip(Direction::East, || {
            with_test_mode(0, || {
                AHEAD.with(|a| a.set(None));
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
            });
        });

        // Parallel mode refuses to run while a piston noclip scope is active.
        let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            with_piston_noclip(Direction::East, || {
                with_test_mode(1, || {
                    AHEAD.with(|a| a.set(None));
                    let _ = Moves::work_out_with(
                        &tables,
                        true,
                        fx.world.revision,
                        &base_items,
                        &obstacles,
                        &round,
                        |_| true,
                    );
                });
            });
        }));
        assert!(
            caught.is_err(),
            "parallel item moves must refuse an active piston noclip scope"
        );
    }

    #[test]
    fn item_moves_verify_panics_on_mismatch() {
        let (fx, base_items, obstacles, _, _) = build_crowd_fixture();
        let collision = fx.collision();
        let tables = fx.tables(&collision);
        let round = native_ticks::TickRound::default();

        let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            with_test_mode(2, || {
                AHEAD.with(|a| a.set(None));
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
}
