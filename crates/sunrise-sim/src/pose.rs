//! Resident player pose, native dimension arithmetic and dry pose-fit selection.
//! Full synchronized entity schemas, sleeping and fluid/flight lifecycles remain
//! separate from these stored movement fields.
use sunrise_core::geometry::{Aabb, Vec3};
use sunrise_world::fluid::FluidKind;

/// A pending `Entity.updateSwimming` entry. Its answer belongs only to this actor and sample.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SwimEntry {
    pub(crate) entity: i32,
    pub(crate) position: Vec3,
    pub(crate) passenger: bool,
    pub(crate) under_water: bool,
    pub(crate) feet_fluid: FluidKind,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Pose {
    #[default]
    Standing,
    Crouching,
    Swimming,
    FallFlying,
    SpinAttack,
    Sleeping,
}
impl Pose {
    /// `Pose` by its network id (`Pose.id`); the poses a player never takes
    /// are standing. One id table: `mod_api::Pose`'s, narrowed to the avatar's
    /// six stances.
    pub fn from_id(id: i32) -> Self {
        mod_api::Pose::from_id(id).into()
    }
    /// Network ID for this pose (`Pose.id`).
    pub fn id(self) -> i32 {
        match self {
            Self::Standing => 0,
            Self::FallFlying => 1,
            Self::Sleeping => 2,
            Self::Swimming => 3,
            Self::SpinAttack => 4,
            Self::Crouching => 5,
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Self::Standing => "STANDING",
            Self::Crouching => "CROUCHING",
            Self::Swimming => "SWIMMING",
            Self::FallFlying => "FALL_FLYING",
            Self::SpinAttack => "SPIN_ATTACK",
            Self::Sleeping => "SLEEPING",
        }
    }
    /// `Avatar.POSES` (and `LivingEntity.SLEEPING_DIMENSIONS`): the box of a
    /// player or mannequin in this pose; sleeping ignores the scale.
    pub fn dimensions(self, scale: f32) -> Dimensions {
        let (width, height, eye_height) = match self {
            Self::Standing => (0.6 * scale, 1.8 * scale, 1.62 * scale),
            Self::Crouching => (0.6 * scale, 1.5 * scale, 1.27 * scale),
            Self::Swimming | Self::FallFlying | Self::SpinAttack => {
                (0.6 * scale, 0.6 * scale, 0.4 * scale)
            }
            Self::Sleeping => (0.2, 0.2, 0.2),
        };
        Dimensions {
            width,
            height,
            eye_height,
        }
    }
}
impl From<Pose> for mod_api::Pose {
    /// The living entity's pose (`Entity.getPose`) the avatar's stance names.
    fn from(pose: Pose) -> Self {
        match pose {
            Pose::Standing => Self::Standing,
            Pose::FallFlying => Self::FallFlying,
            Pose::Sleeping => Self::Sleeping,
            Pose::Swimming => Self::Swimming,
            Pose::SpinAttack => Self::SpinAttack,
            Pose::Crouching => Self::Crouching,
        }
    }
}
impl From<mod_api::Pose> for Pose {
    /// Narrow a living entity's pose to a movement stance. Dimension queries
    /// use `Dimensions::avatar` to preserve Avatar.POSES's fixed dying box.
    fn from(pose: mod_api::Pose) -> Self {
        match pose {
            mod_api::Pose::FallFlying => Self::FallFlying,
            mod_api::Pose::Sleeping => Self::Sleeping,
            mod_api::Pose::Swimming => Self::Swimming,
            mod_api::Pose::SpinAttack => Self::SpinAttack,
            mod_api::Pose::Crouching => Self::Crouching,
            _ => Self::Standing,
        }
    }
}
/// What `Player.getDesiredPose` reads of a player.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PoseFlags {
    /// `LivingEntity.isSleeping`.
    pub sleeping: bool,
    /// `Entity.isSwimming`.
    pub swimming: bool,
    /// `LivingEntity.isFallFlying`.
    pub fall_flying: bool,
    /// `LivingEntity.isAutoSpinAttack`: Riptide's spin.
    pub auto_spin_attack: bool,
    /// `Entity.isShiftKeyDown`.
    pub shift_key_down: bool,
    /// `Abilities.flying`.
    pub flying: bool,
}
/// `Player.getDesiredPose` of the player `player` (entity id): sleeping, swimming, fall flying,
/// Riptide's spin, crouching (shift held, not flying), else standing. `updatePlayerPose` then
/// keeps it only where it fits ([`crate::motion::MotionWorld::update_player_pose`]: the client's prediction
/// and the server's copy of every player), and the server shows the others the pose that
/// update took. NeoForge and Fabric have no event that decides the
/// pose, so a Java mod mixes into `Player.getDesiredPose`; a native mod answers here through
/// `[[native.mixin]]` (#4854).
#[sunrise_mod_api_macros::replaceable]
pub fn desired_pose(player: i32, flags: PoseFlags) -> Pose {
    let pose = if flags.sleeping {
        Pose::Sleeping
    } else if flags.swimming {
        Pose::Swimming
    } else if flags.fall_flying {
        Pose::FallFlying
    } else if flags.auto_spin_attack {
        Pose::SpinAttack
    } else if flags.shift_key_down && !flags.flying {
        Pose::Crouching
    } else {
        Pose::Standing
    };
    #[cfg(test)]
    let pose = crate::rule_replacements::desired_pose(player, flags).unwrap_or(pose);
    let _ = player;
    pose
}
#[derive(Clone, Copy, Debug)]
pub struct Dimensions {
    pub width: f32,
    pub height: f32,
    pub eye_height: f32,
}
/// What `entity_size` (`EntityEvent.Size`) answered for an entity whose size vanilla
/// recalculated (`Entity.refreshDimensions`) to `vanilla` (width, height, eye height) in
/// `pose`: `size` replaces it while both still hold. Any change to either goes back to
/// vanilla's until the hook is asked again; `size` is `None` when the listeners left it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SizeAnswer {
    pub pose: mod_api::Pose,
    pub vanilla: [f32; 3],
    pub size: Option<[f32; 3]>,
}
impl SizeAnswer {
    /// Whether this answer is for `pose` and vanilla's `vanilla`.
    pub fn asked(&self, pose: mod_api::Pose, vanilla: [f32; 3]) -> bool {
        self.pose == pose && self.vanilla == vanilla
    }
    /// The size to use instead of `vanilla` in `pose`, if this answer still holds.
    pub fn holds(&self, pose: mod_api::Pose, vanilla: [f32; 3]) -> Option<[f32; 3]> {
        self.size.filter(|_| self.asked(pose, vanilla))
    }
    /// The size the entity had with this answer: the listeners', else vanilla's.
    pub fn effective(&self) -> [f32; 3] {
        self.size.unwrap_or(self.vanilla)
    }
    /// The box of an entity whose pose doesn't size it, at `position`: `vanilla_box`, the one
    /// its kind makes for its vanilla size `vanilla`, unless `answer` resized it and still
    /// holds for that size. Then `EntityDimensions.makeBoundingBox` of the answer, on the same
    /// bottom (a wind charge's sits below its position).
    pub fn sized_box(
        answer: Option<Self>,
        vanilla: [f32; 3],
        position: Vec3,
        vanilla_box: Aabb,
    ) -> Aabb {
        let Some([width, height, _]) =
            answer.and_then(|answer| answer.size.filter(|_| answer.vanilla == vanilla))
        else {
            return vanilla_box;
        };
        let half = f64::from(width / 2.0);
        let bottom = vanilla_box.min_position().y;
        Aabb::new(
            Vec3::new(position.x - half, bottom, position.z - half),
            Vec3::new(
                position.x + half,
                bottom + f64::from(height),
                position.z + half,
            ),
        )
    }
}
impl Dimensions {
    pub fn to_array(self) -> [f32; 3] {
        [self.width, self.height, self.eye_height]
    }
    pub fn from_array([width, height, eye_height]: [f32; 3]) -> Self {
        Self {
            width,
            height,
            eye_height,
        }
    }
    /// `Avatar.getDefaultDimensions` with the living scale applied, including
    /// the fixed DYING entry that is not a movement stance.
    pub fn avatar(pose: mod_api::Pose, scale: f32) -> Self {
        if pose == mod_api::Pose::Dying {
            Self {
                width: 0.2,
                height: 0.2,
                eye_height: 1.62,
            }
        } else {
            Pose::from(pose).dimensions(scale)
        }
    }

    pub fn bounds(self, position: Vec3) -> Aabb {
        let half = f64::from(self.width / 2.0);
        Aabb::new(
            Vec3::new(position.x - half, position.y, position.z - half),
            Vec3::new(
                position.x + half,
                position.y + f64::from(self.height),
                position.z + half,
            ),
        )
    }
}
