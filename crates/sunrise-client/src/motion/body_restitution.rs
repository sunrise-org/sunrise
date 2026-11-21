//! The restitution phase of `Entity.move` for bodies without a full movement provider.
use crate::{
    block_contact::{self, Contact},
    blocks::BlockFacts,
    collisions::{PowderSnowSubject, SceneCollisions},
    motion::{FrictionLevel, SideLevel, block_support_at},
    world::World,
};
use std::convert::Infallible;
use sunrise_core::{
    geometry::{Aabb, Vec3},
    position::BlockPos,
};
use sunrise_world::{
    block_collision::BlockCollisionRegistry,
    collision_context::{CollisionContext, CollisionEntity, EntityContextKind},
    entity_move::different,
    entity_obstruction::EntityIdentity,
    ground_support::{GroundState, find_supporting_block},
    movement_properties::BoundMovementProperties,
    movement_restitution::{RestitutionInput, RestitutionWorld, restitute_movement},
    movement_support::{LEGACY_SUPPORT_OFFSET, MOVEMENT_SUPPORT_OFFSET},
    native_load_context::LoopContext,
    registry::BlockStateId,
};

/// The level data used by an entity that does not keep a movement world of its own.
#[derive(Clone, Copy)]
pub struct BodyMotion<'a> {
    pub world: &'a World,
    pub collision: &'a BlockCollisionRegistry<'a>,
    pub facts: &'a BlockFacts,
    pub blocks: Option<&'a BoundMovementProperties>,
    pub context: &'a LoopContext,
    /// Standalone simulation tests do not have a mod level host.
    pub side: Option<&'a dyn SideLevel>,
}

/// The legacy effect pair captured from the post-move footprint, and its supporting block.
#[derive(Clone, Copy, Debug)]
pub struct BodyContact {
    pub position: BlockPos,
    pub state: BlockStateId,
    feet: Vec3,
    support: Option<BlockPos>,
}

/// The body's post-move footprint and the wanted/resolved displacement pair.
pub struct BodyCollision<'a> {
    pub footprint: (Vec3, Aabb),
    pub movement: (Vec3, Vec3),
    pub authoritative: bool,
    pub context: CollisionContext<'a>,
    pub subject: Option<PowderSnowSubject<'a>>,
}

/// Wanted and clipped motion, with the state selected before the fall phase.
#[derive(Clone, Copy, Debug)]
pub struct BodyMove {
    pub wanted: Vec3,
    pub moved: Vec3,
    pub effect: BodyContact,
}

/// The numerical response, ready for the owning level's effects before it writes velocity.
pub struct BodyRestitution {
    pub velocity: Vec3,
    pub bounce: Option<BodyBounce>,
}

pub struct BodyBounce {
    pub state: BlockStateId,
    pub position: BlockPos,
    pub fall_distance: f64,
}

impl BodyMotion<'_> {
    /// `CollisionContext.of(entity)` for an ordinary nonliving body.
    pub fn entity_context(id: u32, feet: Vec3) -> CollisionContext<'static> {
        CollisionContext::for_entity(
            CollisionEntity {
                identity: EntityIdentity(u64::from(id)),
                kind: EntityContextKind::Other,
                y: feet.y,
                descending: false,
                main_hand: None,
            },
            false,
        )
    }

    /// Record the collision flags and support selected at the end of `Entity.move`.
    pub fn record_move(self, collision: BodyCollision<'_>, ground: &mut GroundState) {
        let BodyCollision {
            footprint: (feet, bounds),
            movement: (wanted, moved),
            authoritative,
            context,
            subject,
        } = collision;
        ground.horizontal_collision = different(wanted.x, moved.x) || different(wanted.z, moved.z);
        if authoritative || wanted.y.abs() > 0.0 {
            ground.on_ground = wanted.y < 0.0 && wanted.y != moved.y;
            self.update_support(ground, feet, bounds, Some(moved), context, subject);
        }
    }

    /// Update the moving body's retained support using the same shapes as its move.
    pub fn update_support(
        self,
        ground: &mut GroundState,
        feet: Vec3,
        bounds: Aabb,
        movement: Option<Vec3>,
        context: CollisionContext<'_>,
        subject: Option<PowderSnowSubject<'_>>,
    ) {
        let mut scene = SceneCollisions {
            world: self.world,
            binding: self.collision,
            subject,
        };
        let _ = ground.check_support(ground.on_ground, bounds, movement, |bounds| {
            find_supporting_block(
                &mut scene,
                context.at_entity_height(feet.y),
                feet,
                bounds,
                usize::MAX,
            )
        });
    }

    /// Read the legacy effect pair from the retained support, with no new collision scan.
    pub fn legacy_contact(self, feet: Vec3, ground: &GroundState) -> BodyContact {
        let support = ground.supporting_block;
        let position = block_support_at(
            self.world,
            self.blocks,
            feet,
            support,
            LEGACY_SUPPORT_OFFSET,
        );
        BodyContact {
            position,
            state: self.world.block(position.x, position.y, position.z),
            feet,
            support,
        }
    }

    /// Resolve the collision without writing the body's velocity. The owner calls this
    /// after fall handling, only while the entity is live and can simulate movement.
    pub fn restitute(
        self,
        entity: (u32, &str),
        movement: BodyMove,
        gravity: f64,
        drag: f32,
        velocity: Vec3,
        fall_distance: f64,
    ) -> BodyRestitution {
        let BodyMove {
            wanted,
            moved,
            effect,
        } = movement;
        let collided_x = different(wanted.x, moved.x);
        let collided_z = different(wanted.z, moved.z);
        let vertical_collision = wanted.y != moved.y;
        let mut body = BodyResponse {
            motion: self,
            effect,
            entity,
            gravity,
            drag,
            result: BodyRestitution {
                velocity,
                bounce: None,
            },
        };
        if collided_x || collided_z || (wanted.y.abs() > 0.0 && vertical_collision) {
            let Ok(()) = restitute_movement(
                &mut body,
                RestitutionInput {
                    block: effect.state,
                    effect_position: effect.position,
                    collided_x,
                    collided_z,
                    vertical_collision,
                    collision_below: vertical_collision && wanted.y < 0.0,
                    living: false,
                    resolved_movement: moved,
                    fall_distance,
                },
            );
        }
        body.result
    }
}

struct BodyResponse<'a> {
    motion: BodyMotion<'a>,
    effect: BodyContact,
    entity: (u32, &'a str),
    gravity: f64,
    drag: f32,
    result: BodyRestitution,
}

impl RestitutionWorld for BodyResponse<'_> {
    type Error = Infallible;

    fn suppressing_bounce(&mut self) -> Result<bool, Infallible> {
        Ok(false)
    }
    fn entity_bounciness(&mut self) -> Result<f64, Infallible> {
        Ok(0.0)
    }
    fn velocity(&mut self) -> Result<Vec3, Infallible> {
        Ok(self.result.velocity)
    }
    fn effective_gravity(&mut self) -> Result<f64, Infallible> {
        Ok(self.gravity)
    }
    fn air_drag(&mut self) -> Result<f32, Infallible> {
        Ok(self.drag)
    }

    fn block_suppresses_bounce(&mut self, block: BlockStateId) -> Result<bool, Infallible> {
        Ok(self
            .motion
            .blocks
            .and_then(|blocks| blocks.get(block).ok())
            .is_some_and(|p| p.suppresses_bounce))
    }

    fn block_bounce_restitution(
        &mut self,
        state: BlockStateId,
        position: BlockPos,
    ) -> Result<f32, Infallible> {
        let own = self
            .motion
            .blocks
            .and_then(|blocks| blocks.get(state).ok())
            .map_or(0.0, |p| p.coefficients.bounce_restitution);
        let Some(side) = self.motion.side else {
            return Ok(own);
        };
        let facts = self.motion.facts;
        Ok(block_contact::bounce_restitution(
            FrictionLevel {
                world: self.motion.world,
                registry: self.motion.collision.registry(),
                context: self.motion.context,
                side,
                coefficient: &|id| facts.states.get(id.0 as usize).map(|p| p.friction),
            },
            Contact {
                entity: self.entity.0 as i32,
                kind: self.entity.1,
                state,
                position,
            },
            own,
        ))
    }

    fn movement_support_position(&mut self) -> Result<BlockPos, Infallible> {
        Ok(block_support_at(
            self.motion.world,
            self.motion.blocks,
            self.effect.feet,
            self.effect.support,
            MOVEMENT_SUPPORT_OFFSET,
        ))
    }
    fn bounce_on(
        &mut self,
        state: BlockStateId,
        position: BlockPos,
        fall_distance: f64,
        movement: Vec3,
    ) -> Result<Vec3, Infallible> {
        self.result.bounce = Some(BodyBounce {
            state,
            position,
            fall_distance,
        });
        Ok(movement)
    }
    fn bounce_event(&mut self) -> Result<(), Infallible> {
        Ok(())
    }
    fn mark_position_for_sync(&mut self) {}
    fn set_velocity(&mut self, velocity: Vec3) -> Result<(), Infallible> {
        self.result.velocity = velocity;
        Ok(())
    }
}
