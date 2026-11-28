use std::{collections::HashMap, sync::Arc};
use sunrise_core::{
    geometry::Vec3,
    random::{LegacyRandom, RandomSource},
};
use sunrise_level::containers::BlockEntity;
use sunrise_sim::{projectiles::Projectile, vanillagen::VanillaTerrain};

/// ServerLevel.END_SPAWN_POINT.
pub const END_SPAWN: [i32; 3] = [100, 50, 0];

/// A gateway teleport waiting for its destination chunks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GatewayJob {
    /// TheEndGatewayBlockEntity.findOrCreateValidTeleportPos from this gateway.
    /// `exact` is the gateway's own `ExactTeleport`, read while its chunk is loaded.
    Find { gateway: [i32; 3], exact: bool },
    /// A known exit (exact, or findExitPosition around it).
    Known { exit: [i32; 3], exact: bool },
}

/// An entity travelling through an End gateway: the entity and the job that
/// finds where it lands (`TheEndGatewayBlockEntity.getPortalPosition`).
#[derive(Clone, Debug)]
pub struct GatewayArrival {
    /// The entity, as it waits for the exit's chunks.
    pub traveller: Arriving,
    /// The gateway's exit, or the search for one.
    pub job: GatewayJob,
    /// The spot to load before the job is resolved.
    pub target: Vec3,
}

/// What waits at a gateway's exit.
#[derive(Clone, Debug)]
pub enum Arriving {
    /// A thrown ender pearl (projectiles are not saved).
    Pearl(Box<Projectile>),
    /// An ordinary entity remains live in this level while its destination loads.
    Live(u32),
}

impl GatewayJob {
    /// The source of a new exit must stay loaded until its block entity is updated.
    pub fn source_chunk(self) -> Option<(i32, i32)> {
        match self {
            Self::Find { gateway, .. } => Some((gateway[0] >> 4, gateway[2] >> 4)),
            Self::Known { .. } => None,
        }
    }

    /// TheEndGatewayBlockEntity.getPortalPosition, using the gateway block entity.
    pub fn for_gateway(gateway: [i32; 3], entity: Option<&BlockEntity>) -> Self {
        let configured = match entity {
            Some(BlockEntity::EndGateway(g)) => g.exit_portal.map(|exit| (exit, g.exact_teleport)),
            _ => None,
        };
        match configured {
            Some((exit, exact)) => Self::Known { exit, exact },
            // Worldgen return gateways (end_gateway_return) lead exactly to the End spawn.
            None if sunrise_core::geometry::length(
                f64::from(gateway[0]),
                f64::from(gateway[2]),
            ) > 512.0 =>
            {
                Self::Known {
                    exit: END_SPAWN,
                    exact: true,
                }
            }
            None => Self::Find {
                gateway,
                exact: exact_teleport(entity),
            },
        }
    }
}

/// `TheEndGatewayBlockEntity.exactTeleport` of the block entity at a gateway
/// (false without one).
pub fn exact_teleport(entity: Option<&BlockEntity>) -> bool {
    matches!(entity, Some(BlockEntity::EndGateway(g)) if g.exact_teleport)
}

/// Per-level End state exchanged on level swap.
#[derive(Default)]
pub struct EndLevelState {
    pub locator: Option<Arc<VanillaTerrain>>,
}

impl EndLevelState {
    pub fn new(locator: Option<Arc<VanillaTerrain>>) -> Self {
        Self { locator }
    }
}

#[derive(Default)]
pub struct EndState {
    /// Ticks until the player can use a gateway again (TheEndGatewayBlockEntity cooldown).
    pub gateway_cooldown: u32,
    pub gateways: HashMap<u32, sunrise_sim::end_gateway::Traveller>,
    /// The entities that went through a gateway, waiting for its exit.
    pub arrivals: Vec<GatewayArrival>,
    /// The chunks kept loaded for those arrivals.
    pub areas: Vec<(i32, i32)>,
    /// `Entity.placePortalTicket` of the pearls that came out of a gateway:
    /// the chunk each landed in, kept loaded for the ticks left of its
    /// `TicketType.PORTAL`, so that it can come down and take its owner there.
    pub tickets: Vec<((i32, i32), u32)>,
    /// The Overworld generator, for stronghold positions (ServerLevel.findNearestMapStructure).
    pub locator: std::sync::Mutex<Option<Arc<VanillaTerrain>>>,
    /// Thrown eyes of ender in flight.
    pub eyes: Vec<EyeOfEnder>,
    pub flash: EndFlash,
}

/// EndFlashState: a purple flash in the End sky every 600 ticks that also
/// lights the world (the lightmap's sky factor).
#[derive(Default)]
pub struct EndFlash {
    pub(in crate::game) seed: Option<i64>,
    pub(in crate::game) offset: i32,
    pub(in crate::game) duration: i32,
    pub(in crate::game) intensity: f32,
    pub(in crate::game) old_intensity: f32,
    pub x_angle: f32,
    pub y_angle: f32,
}
impl EndFlash {
    pub fn tick(&mut self, clock: i64) {
        let seed = clock.div_euclid(600);
        if self.seed != Some(seed) {
            let mut random = LegacyRandom::new(seed);
            random.next_float();
            let between = |r: &mut LegacyRandom, lo: i32, hi: i32| {
                r.next_int_bound(hi - lo + 1).unwrap_or(0) + lo
            };
            self.offset = between(&mut random, 0, 200);
            self.duration = between(&mut random, 100, 380.min(600 - self.offset));
            self.x_angle = random.next_float() * 70.0 - 60.0;
            self.y_angle = random.next_float() * 360.0 - 180.0;
            self.seed = Some(seed);
        }
        self.old_intensity = self.intensity;
        let t = clock.rem_euclid(600) as i32;
        self.intensity = if t >= self.offset && t <= self.offset + self.duration {
            ((t - self.offset) as f32 * std::f32::consts::PI / self.duration as f32).sin()
        } else {
            0.0
        };
    }
    /// `EndFlashState.flashStartedThisTick`.
    pub fn started_this_tick(&self) -> bool {
        self.intensity > 0.0 && self.old_intensity <= 0.0
    }
    pub fn intensity(&self, partial: f32) -> f32 {
        self.old_intensity + (self.intensity - self.old_intensity) * partial
    }
    /// SkyRenderer.renderEndFlash's pose: the sprite at height 100 turned to the flash.
    pub fn pose(&self) -> glam::Mat4 {
        flash_pose(self.x_angle, self.y_angle)
    }
}

pub fn flash_pose(x_angle: f32, y_angle: f32) -> glam::Mat4 {
    glam::Mat4::from_rotation_y((180.0 - y_angle).to_radians())
        * glam::Mat4::from_rotation_x((-90.0 - x_angle).to_radians())
}

impl EndState {
    pub fn locator(&self) -> Option<Arc<VanillaTerrain>> {
        self.locator.lock().unwrap().clone()
    }
    pub fn set_locator(&self, terrain: Option<Arc<VanillaTerrain>>) {
        *self.locator.lock().unwrap() = terrain;
    }
    pub fn swap_level(&mut self, other: &mut EndLevelState) {
        let mut locator = self.locator();
        std::mem::swap(&mut locator, &mut other.locator);
        self.set_locator(locator);
    }
}

impl EndState {
    /// The eyes' entity ids, for a server to number them.
    pub fn eye_ids(&mut self) -> impl Iterator<Item = &mut u32> {
        self.eyes.iter_mut().map(|e| &mut e.id)
    }
    /// The numbered eyes as a server's clients track them: (id, position, motion).
    pub fn eyes_tracked(&self) -> impl Iterator<Item = (u32, [f64; 3], [f64; 3])> {
        self.eyes
            .iter()
            .filter(|e| e.id != 0)
            .map(|e| (e.id, e.position.to_array(), e.velocity.to_array()))
    }
    /// `handleAddEntity` for an eye of ender on a server's client: it flies
    /// where the server says.
    pub fn add_remote_eye(&mut self, id: u32, at: [f64; 3], velocity: [f64; 3]) {
        self.eyes.retain(|e| e.id != id);
        let at = glam::DVec3::from_array(at);
        self.eyes.push(EyeOfEnder {
            id,
            position: at,
            previous: at,
            velocity: glam::DVec3::from_array(velocity),
            target: at,
            life: 0,
            survive: false,
        });
    }
    /// Where each eye in flight was last tick and is now (the hitbox view).
    pub fn eye_motions(&self) -> impl Iterator<Item = (glam::DVec3, glam::DVec3)> + '_ {
        self.eyes.iter().map(|e| (e.previous, e.position))
    }
    /// `EyeOfEnder.tick` on a server's client: it moves by its last known
    /// motion; only the server steers it.
    pub fn step_remote_eyes(&mut self) {
        for e in &mut self.eyes {
            e.previous = e.position;
            e.life += 1;
            e.position += e.velocity;
        }
    }
    /// A server's move: an eye has no interpolation handler, so
    /// `moveOrInterpolateTo` puts it there at once. False when `id` isn't one.
    pub fn remote_eye_pose(&self, id: u32) -> Option<([f64; 3], [f64; 3])> {
        let e = self.eyes.iter().find(|e| e.id == id)?;
        Some((e.position.to_array(), e.previous.to_array()))
    }
    pub fn move_remote_eye(&mut self, id: u32, at: [f64; 3]) -> bool {
        let Some(e) = self.eyes.iter_mut().find(|e| e.id == id) else {
            return false;
        };
        e.position = glam::DVec3::from_array(at);
        true
    }
    /// `handleSetEntityMotion` for an eye. False when `id` isn't one.
    pub fn remote_eye_motion(&mut self, id: u32, velocity: [f64; 3]) -> bool {
        let Some(e) = self.eyes.iter_mut().find(|e| e.id == id) else {
            return false;
        };
        e.velocity = glam::DVec3::from_array(velocity);
        true
    }
    pub fn remove_remote_eye(&mut self, id: u32) {
        self.eyes.retain(|e| e.id != id);
    }
    pub fn clear_remote_eyes(&mut self) {
        self.eyes.clear();
    }
    /// Where each eye is, for `status`.
    pub fn eye_positions(&self) -> impl Iterator<Item = (u32, [f64; 3])> {
        self.eyes.iter().map(|e| (e.id, e.position.to_array()))
    }
}

/// An eye of ender from `/summon` or a saved chunk. `signalTo` never ran for
/// it, so it has no target and `surviveAfterDeath` stays false.
#[derive(Clone, Copy, Debug)]
pub struct SummonedEye {
    /// `getDeltaMovement`, which nothing changes without a target.
    pub motion: Vec3,
    /// `EyeOfEnder.life`: not saved, so a loaded eye starts again at 0.
    pub life: u32,
    /// `Entity.remainingFireTicks`, counted down by `baseTick`.
    pub fire_ticks: i32,
    /// `Entity.DATA_SHARED_FLAGS_ID` on-fire bit, set after `baseTick` fire processing.
    pub synced_fire: bool,
    /// `Entity.hasVisualFire` (`HasVisualFire` in NBT).
    pub has_visual_fire: bool,
}
impl SummonedEye {
    /// `Entity.load`'s motion and `EyeOfEnder.readAdditionalSaveData`'s
    /// `setItem`, which keeps one of the saved item, or an eye of ender.
    pub fn load(tag: &mut sunrise_core::nbt::Tag) -> Self {
        use sunrise_core::nbt::Tag;
        let motion = sunrise_sim::entity_nbt::loaded_motion(tag);
        let fire_ticks = sunrise_sim::entity_nbt::fire_ticks_from_nbt(tag);
        let has_visual_fire = sunrise_sim::entity_nbt::flag(tag, "HasVisualFire");
        if let Tag::Compound(c) = tag {
            let mut item = match c.remove(&"Item".into()) {
                Some(Tag::Compound(item)) => item,
                _ => Default::default(),
            };
            let id = match item.get(&"id".into()) {
                Some(Tag::String(s)) => {
                    sunrise_game_data::identifier::normalize_id(&s.to_string_lossy())
                }
                _ => None,
            }
            .filter(|id| id != "minecraft:air");
            if id.is_none() {
                item.clear();
            }
            let id = id.unwrap_or_else(|| "minecraft:ender_eye".into());
            item.insert("id".into(), Tag::String(id.as_str().into()));
            item.insert("count".into(), Tag::Int(1));
            c.insert("Item".into(), Tag::Compound(item));
        }
        Self {
            motion,
            life: 0,
            fire_ticks,
            synced_fire: false,
            has_visual_fire,
        }
    }
}

/// The EyeOfEnder entity.
pub struct EyeOfEnder {
    /// Its entity id once a server's clients track it (0 until then).
    pub id: u32,
    pub position: glam::DVec3,
    pub previous: glam::DVec3,
    pub velocity: glam::DVec3,
    pub target: glam::DVec3,
    pub life: u32,
    pub survive: bool,
}
impl EyeOfEnder {
    /// EyeOfEnder.updateDeltaMovement.
    pub fn update_delta(
        old: glam::DVec3,
        position: glam::DVec3,
        target: glam::DVec3,
    ) -> glam::DVec3 {
        let horizontal = glam::DVec3::new(target.x - position.x, 0.0, target.z - position.z);
        let length = horizontal.length();
        let old_horizontal = sunrise_core::geometry::length(old.x, old.z);
        let mut wanted_speed = old_horizontal + 0.0025 * (length - old_horizontal);
        let mut y = old.y;
        if length < 1.0 {
            wanted_speed *= 0.8;
            y *= 0.8;
        }
        let wanted_y = if position.y - old.y < target.y {
            1.0
        } else {
            -1.0
        };
        horizontal * (wanted_speed / length)
            + glam::DVec3::new(0.0, y + (wanted_y - y) * 0.015, 0.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sunrise_core::position::BlockPos;

    #[test]
    fn swap_level_keeps_non_level_fields_untouched() {
        let mut traveller = sunrise_sim::end_gateway::Traveller::default();
        traveller.enter_with_delay(BlockPos::new(2, 64, 3), 23);
        let mut state = EndState {
            gateway_cooldown: 17,
            gateways: [(9, traveller)].into_iter().collect(),
            arrivals: vec![GatewayArrival {
                traveller: Arriving::Live(77),
                job: GatewayJob::Known {
                    exit: [1, 2, 3],
                    exact: true,
                },
                target: Vec3::new(4.0, 5.0, 6.0),
            }],
            areas: vec![(3, 4)],
            tickets: vec![((5, 6), 99)],
            locator: std::sync::Mutex::new(None),
            eyes: vec![EyeOfEnder {
                id: 55,
                position: glam::DVec3::new(1.0, 2.0, 3.0),
                previous: glam::DVec3::new(0.5, 1.5, 2.5),
                velocity: glam::DVec3::new(0.1, 0.2, 0.3),
                target: glam::DVec3::new(10.0, 20.0, 30.0),
                life: 12,
                survive: true,
            }],
            flash: EndFlash {
                seed: Some(7),
                offset: 12,
                duration: 150,
                intensity: 0.5,
                old_intensity: 0.25,
                x_angle: 10.0,
                y_angle: 20.0,
            },
        };
        let mut parked = EndLevelState::new(None);

        let check_non_level = |s: &EndState| {
            assert_eq!(s.gateway_cooldown, 17);
            assert_eq!(s.gateways.len(), 1);
            let mut copy = s.gateways[&9];
            assert_eq!(copy.cooldown(), 0);
            assert!(!copy.is_done());
            assert_eq!(
                copy.travel_with_delay(true, 23),
                Some(BlockPos::new(2, 64, 3))
            );
            assert_eq!(copy.cooldown(), 23);
            assert_eq!(s.arrivals.len(), 1);
            assert!(matches!(s.arrivals[0].traveller, Arriving::Live(77)));
            assert_eq!(
                s.arrivals[0].job,
                GatewayJob::Known {
                    exit: [1, 2, 3],
                    exact: true,
                }
            );
            assert_eq!(s.arrivals[0].target, Vec3::new(4.0, 5.0, 6.0));
            assert_eq!(s.areas, vec![(3, 4)]);
            assert_eq!(s.tickets, vec![((5, 6), 99)]);
            assert_eq!(s.eyes.len(), 1);
            let eye = &s.eyes[0];
            assert_eq!(eye.id, 55);
            assert_eq!(eye.position, glam::DVec3::new(1.0, 2.0, 3.0));
            assert_eq!(eye.previous, glam::DVec3::new(0.5, 1.5, 2.5));
            assert_eq!(eye.velocity, glam::DVec3::new(0.1, 0.2, 0.3));
            assert_eq!(eye.target, glam::DVec3::new(10.0, 20.0, 30.0));
            assert_eq!(eye.life, 12);
            assert!(eye.survive);
            assert_eq!(s.flash.seed, Some(7));
            assert_eq!(s.flash.offset, 12);
            assert_eq!(s.flash.duration, 150);
            assert_eq!(s.flash.intensity, 0.5);
            assert_eq!(s.flash.old_intensity, 0.25);
            assert_eq!(s.flash.x_angle, 10.0);
            assert_eq!(s.flash.y_angle, 20.0);
        };

        state.swap_level(&mut parked);
        check_non_level(&state);
        assert!(state.locator().is_none());
        assert!(parked.locator.is_none());

        state.swap_level(&mut parked);
        check_non_level(&state);
        assert!(state.locator().is_none());
        assert!(parked.locator.is_none());
    }
}
