//! The pose transitions a player's tick drives (`updatePlayerPose`, the spin countdown and
//! rider base tick), on the stored movement state.
use super::{MotionWorld, PlayerMotion};
use crate::Result;
use crate::pose::{Dimensions, Pose, PoseFlags, SizeAnswer, SwimEntry, desired_pose};
use sunrise_core::geometry::Vec3;

impl PlayerMotion {
    /// `LivingEntity.aiStep` counting down `autoSpinAttackTicks` for a
    /// server's copy of a player, which runs no client tick: the spin ends
    /// after its ticks and its damage goes with it, so the pose (and the eye
    /// a bow or a trident is shot from) comes back to standing.
    pub fn count_down_server_spin(&mut self) {
        if self.auto_spin_attack_ticks > 0 {
            self.auto_spin_attack_ticks -= 1;
            if self.auto_spin_attack_ticks <= 0 {
                self.auto_spin_attack_dmg = 0.0;
            }
        }
    }
    pub fn dimensions_for_pose(&mut self, pose: Pose) -> Result<Dimensions> {
        let vanilla = pose.dimensions(self.attribute("minecraft:scale")? as f32);
        Ok(self
            .pose_size_answer(pose, vanilla.to_array())
            .map_or(vanilla, |answer| Dimensions::from_array(answer.effective())))
    }
    fn pose_size_answer(&self, pose: Pose, vanilla: [f32; 3]) -> Option<SizeAnswer> {
        self.size_answer
            .filter(|answer| answer.asked(pose.into(), vanilla))
            .or_else(|| {
                let answer = match pose {
                    Pose::Standing => self.pose_fit_sizes[0],
                    Pose::Crouching => self.pose_fit_sizes[1],
                    Pose::Swimming => self.pose_fit_sizes[2],
                    _ => None,
                };
                answer.filter(|answer| answer.asked(pose.into(), vanilla))
            })
    }
    pub fn refresh_dimensions(&mut self) -> Result<()> {
        self.scale = self.attribute("minecraft:scale")? as f32;
        self.dimensions = self.pose.dimensions(self.scale);
        // Apply the same pre-resolved answer used by the pose fit decision.
        if let Some(answer) = self.pose_size_answer(self.pose, self.dimensions.to_array()) {
            self.size_answer = Some(answer);
            self.dimensions = Dimensions::from_array(answer.effective());
        }
        Ok(())
    }
    pub fn set_pose(&mut self, pose: Pose) -> Result<()> {
        if self.pose != pose {
            self.pose = pose;
            self.refresh_dimensions()?;
        }
        Ok(())
    }
    /// `LivingEntity.startSleeping` and `stopSleeping` set the pose at once
    /// (the 0.2 box in the bed, standing again after).
    pub fn set_sleeping(&mut self, sleeping: bool) -> Result<()> {
        if self.sleeping != sleeping {
            self.sleeping = sleeping;
            self.set_pose(if sleeping {
                Pose::Sleeping
            } else {
                Pose::Standing
            })?;
        }
        Ok(())
    }
    /// `LocalPlayer.isMovingSlowly`: crouching, or `Entity.isVisuallyCrawling`
    /// (visually swimming out of water). `LivingEntity.isVisuallySwimming`
    /// counts the swimming pose and the fall-flying pose a glide that just
    /// ended still holds, so the tick after an elytra lands moves at sneaking
    /// speed (#8295).
    pub fn moving_slowly(&self) -> bool {
        let visually_swimming =
            self.pose == Pose::Swimming || (!self.fall_flying && self.pose == Pose::FallFlying);
        self.local_crouching || (visually_swimming && !self.fluids.was_touching_water)
    }
}
impl MotionWorld<'_, '_, '_> {
    pub fn can_fit_pose(&mut self, pose: Pose) -> Result<bool> {
        let bounds = self
            .player
            .dimensions_for_pose(pose)?
            .bounds(self.player.position);
        self.no_player_collision(bounds.deflate(Vec3::new(1.0e-7, 1.0e-7, 1.0e-7)))
    }
    /// LocalPlayer.aiStep computes crouching BEFORE KeyboardInput.tick, using
    /// the old shift input. This is distinct from Entity's current pose.
    pub fn update_local_crouching(&mut self) -> Result<()> {
        self.player.local_crouching = !self.player.flying
            && !self.player.swimming
            && self.can_fit_pose(Pose::Crouching)?
            && (self.player.shift_key_down || !self.can_fit_pose(Pose::Standing)?);
        Ok(())
    }
    /// Dry Player.updatePlayerPose after movement: crawl when neither standing
    /// nor crouching fits; leave the current pose if even crawling cannot fit.
    pub fn update_player_pose(&mut self) -> Result<()> {
        if !self.can_fit_pose(Pose::Swimming)? {
            return Ok(());
        }
        let desired = desired_pose(
            self.player.entity_id,
            PoseFlags {
                sleeping: self.player.sleeping,
                swimming: self.player.swimming,
                fall_flying: self.player.fall_flying,
                auto_spin_attack: self.player.auto_spin_attack_ticks > 0,
                shift_key_down: self.player.shift_key_down,
                flying: self.player.flying,
            },
        );
        let pose = if self.player.spectator || self.can_fit_pose(desired)? {
            desired
        } else if self.can_fit_pose(Pose::Crouching)? {
            Pose::Crouching
        } else {
            Pose::Swimming
        };
        self.player.set_pose(pose)
    }
    /// The end of a server's copy of a player's tick (`ServerPlayer.doTick`
    /// runs `Player.tick`): `LivingEntity.aiStep` counts a Riptide spin down,
    /// `LivingEntity.tick` then runs `refreshDirtyAttributes` (a changed
    /// `minecraft:scale` rebuilds the box, `onAttributeUpdated`), and
    /// `Player.tick` ends with `updatePlayerPose`. The server checks the
    /// player's moves with that box (#3684).
    pub fn server_player_pose_tick(&mut self) -> Result<()> {
        self.player.count_down_server_spin();
        self.player.refresh_attributes()?;
        self.update_player_pose()
    }
    /// The pose part of `Player.tick` for a passenger (`Entity.rideTick` runs
    /// `tick`, not the travel): `Entity.updateSwimming` stops a swim, since it
    /// needs `!isPassenger()`, and `updatePlayerPose` follows. A player who
    /// boards a boat while swimming sits up instead of lying under it (#3308).
    /// The server's `LivingEntity.updateFallFlying` ends a glide the same way
    /// (`canGlide` needs `!isPassenger()`) and `LivingEntity.tick` resets
    /// `fallFlyTicks`: a player who boards while gliding sits up instead of
    /// keeping the glide pose (#3093).
    /// `Entity.baseTick` still samples the fluids first, in the box the vehicle
    /// allows (the boat the game stores in the player's fluid state), so a boat
    /// afloat keeps the eyes dry.
    pub fn passenger_pose_tick(&mut self) -> Result<()> {
        self.passenger_base_tick()?;
        self.passenger_sits_up()
    }
    /// `Entity.rideTick` for a rider this client moves (`LocalPlayer`): `Player.tick` takes it off
    /// the ground and `Entity.baseTick` samples its fluids, as for any passenger; then
    /// `LivingEntity.aiStep` travels it from where it sat (`rider_travel_tick`), and that move's
    /// ground and collisions are what its look packet sends; `updatePlayerPose` sits it up.
    pub fn rider_tick(&mut self, direction: [f32; 2]) -> Result<()> {
        self.begin_fluid_tick()?;
        self.finish_rider_base_tick(None);
        self.finish_rider_tick(direction)
    }
    /// Apply the rider's swimming decision before `LocalPlayer.aiStep`.
    pub fn finish_rider_base_tick(&mut self, answer: Option<(SwimEntry, bool)>) {
        self.apply_swim_entry(answer);
        self.clear_passenger_flags();
    }
    /// `LivingEntity.aiStep` travels the rider after its sprint decision.
    pub fn finish_rider_tick(&mut self, direction: [f32; 2]) -> Result<()> {
        if self.player.may_fly && self.player.local_spectator {
            self.player.flying = true;
        }
        self.rider_travel_tick(direction)?;
        self.passenger_sits_up()
    }
    /// A passenger's tick before it could move: the fluids in the box its vehicle allows, off the
    /// ground, and neither swimming nor gliding.
    fn passenger_base_tick(&mut self) -> Result<()> {
        self.begin_fluid_tick()?;
        self.clear_passenger_flags();
        Ok(())
    }
    fn clear_passenger_flags(&mut self) {
        // Player.tick: a passenger is never on the ground (setOnGround(false)
        // clears the supporting block too) until its own move says otherwise.
        self.player.ground.on_ground = false;
        self.player.ground.on_ground_no_blocks = false;
        self.player.ground.supporting_block = None;
        self.player.swimming = false;
        self.player.fall_flying = false;
        self.player.fall_fly_ticks = 0;
    }
    /// `Player.updatePlayerPose` for a passenger.
    fn passenger_sits_up(&mut self) -> Result<()> {
        self.update_player_pose()?;
        // A cushion's seat puts the feet in the support block, so neither
        // standing nor crawling fits and `updatePlayerPose` keeps the old
        // pose. A prior swim would stick (#4509); sit up like a boat rider.
        if self.player.pose == Pose::Swimming && !self.player.sleeping {
            self.player.set_pose(Pose::Standing)?;
        }
        Ok(())
    }
}
