//! Ordinary player turns in a transformed source context. The physical player body stays put.
use super::*;
use crate::game::{Dim, GuestUse, Hit};
use mod_api::{Hand, PlayerRef};

#[derive(Clone, Copy)]
pub(super) struct ProjectedAction {
    pub handle: LiveWorldRef,
    pub player: PlayerRef,
    pub local_eye: [f64; 3],
    hit: LiveWorldHit,
    membership: u64,
    break_writes: Option<u64>,
}

impl Game<'_> {
    pub(in crate::game) fn live_world_action_eye(&self) -> Option<[f64; 3]> {
        let table = self.live_worlds.borrow();
        let action = table.action?;
        (self.dim.key() == action.handle.level().dimension()
            && self.native_acting_player() == Some(action.player))
        .then_some(action.local_eye)
    }
    pub(in crate::game) fn live_world_action_in_level(&self) -> bool {
        self.live_world_action_eye().is_some()
    }

    /// Runs after ordinary callbacks and again immediately before the real removal.
    /// It never re-selects a target or replays an event. Vanilla actions have no projection scope.
    pub(in crate::game) fn projection_break_fresh(&self, pos: [i32; 3]) -> bool {
        let table = self.live_worlds.borrow();
        let Some(action) = table.action else {
            return true;
        };
        let Some(writes) = action.break_writes else {
            return true;
        };
        let Some(entry) = table.entries.get(&action.handle.generation()) else {
            return false;
        };
        entry.handle == action.handle
            && self.level_handle_current(action.handle.level())
            && self.level_handle_current(entry.destination)
            && pos == action.hit.cell
            && self.world.revision == action.hit.cell_revision
            && entry.sample.revision == action.hit.pose_revision
            && entry.membership == action.membership
            && entry.cells.binary_search(&pos).is_ok()
            && self.transactions.writes() == writes
            && self.native_acting_player() == Some(action.player)
    }

    /// Authoritative token + current real player eye/reach + ordinary static occlusion.
    fn check_projected_actor(
        &self,
        handle: LiveWorldRef,
        player: PlayerRef,
        hit: LiveWorldHit,
    ) -> Result<LiveWorldPose, Error> {
        self.live_world_validate_hit(handle, hit)?;
        let entry = self.projection_entry(handle)?;
        if self.player_level(player) != Some(entry.destination)
            || !self.native_player_current(player)
        {
            return Err(Error::Stale);
        }
        let eye = self.eye_position().ok_or(Error::Unavailable)?;
        let direction = vector(hit.point).subtract(vector(eye));
        let range = self.reach() + 1.0;
        if !range.is_finite() || direction.length_squared() >= range * range {
            return Err(Error::OutOfReach);
        }
        if direction.length_squared() <= 1.0e-12 {
            return Err(Error::Invalid);
        }
        let end = vector(hit.point).add_vector(direction.normalize().scale(1.0e-5));
        let geometry = self.projection_geometry(&entry)?;
        let actual = geometry
            .contact
            .clip(&checked_pose(entry.sample.current)?, vector(eye), end)
            .ok_or(Error::Revision)?;
        if actual.local_cell != position(hit.cell)
            || actual.local_direction != hit.face
            || actual.location.distance_squared(vector(hit.point)) > 1.0e-10
        {
            return Err(Error::Revision);
        }
        // The caller cannot select a farther owner through a nearer live wall. The same
        // bounded real geometry used for drawing/contact is authoritative on the server.
        let mut remaining = entry.limits.contacts as usize;
        remaining = remaining
            .checked_sub(geometry.boxes.boxes().len())
            .ok_or(Error::Budget)?;
        let table = self.live_worlds.borrow();
        for other in table
            .entries
            .values()
            .filter(|other| other.handle != handle && other.destination == entry.destination)
        {
            let geometry = self.projection_geometry(other)?;
            remaining = remaining
                .checked_sub(geometry.boxes.boxes().len())
                .ok_or(Error::Budget)?;
            if let Some(nearer) =
                geometry
                    .contact
                    .clip(&checked_pose(other.sample.current)?, vector(eye), end)
                && nearer.location.distance_squared(vector(eye)) + 1.0e-10
                    < direction.length_squared()
            {
                return Err(Error::Obstructed);
            }
        }
        drop(table);
        let obstruction = self
            .level_clip(
                entry.destination,
                eye,
                hit.point,
                mod_api::ClipBlock::Collider,
                mod_api::ClipFluid::None,
            )
            .ok_or(Error::NotResident)?;
        if !obstruction.miss
            && vector(obstruction.location).distance_squared(vector(eye)) + 1.0e-10
                < direction.length_squared()
        {
            return Err(Error::Obstructed);
        }
        let physical = hit.point.map(|v| v.floor() as i32);
        if self.lan_acting_spawn_protection.is_some_and(|radius| {
            self.spawn_protects(radius, entry.destination.dimension_id(), physical)
        }) {
            return Err(Error::Denied);
        }
        Ok(entry.sample.current)
    }

    fn with_projected_actor<T>(
        &mut self,
        handle: LiveWorldRef,
        player: PlayerRef,
        hit: LiveWorldHit,
        operation: impl FnOnce(&mut Self, Hit) -> Result<T, Error>,
    ) -> Result<T, Error> {
        if self.live_worlds.borrow().action.is_some() {
            return Err(Error::Busy);
        }
        // Resolve ownership before borrowing the player. No client or foreign owner gets this turn.
        self.projection_entry(handle)?;
        let mut answer = Err(Error::Unavailable);
        let ran = self.native_as_player_handle(player, |game| {
            answer = (|| {
                let pose = game.check_projected_actor(handle, player, hit)?;
                let actual_eye = game.eye_position().ok_or(Error::Unavailable)?;
                let local_eye = checked_pose(pose)?
                    .to_local(vector(actual_eye))
                    .components();
                let membership = game.projection_entry(handle)?.membership;
                let target = Dim::from_id(handle.level().dimension_id());
                let back = game.enter_command_level(target);
                if game.dim != target {
                    return Err(Error::Unavailable);
                }
                let view = vector(crate::game::view_vector(game.yaw, game.pitch));
                let rotation = checked_pose(pose)?.rotation();
                let local_view = rotation.inverse_rotate(view);
                let old_angles = (game.yaw, game.pitch);
                game.yaw = (-local_view.x).atan2(local_view.z).to_degrees() as f32;
                game.pitch = (-local_view.y)
                    .atan2((local_view.x * local_view.x + local_view.z * local_view.z).sqrt())
                    .to_degrees() as f32;
                game.live_worlds.get_mut().action = Some(ProjectedAction {
                    handle,
                    player,
                    local_eye,
                    hit,
                    membership,
                    break_writes: None,
                });
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    operation(
                        game,
                        Hit {
                            pos: hit.cell,
                            face: hit.face.normal(),
                            point: hit.local_point,
                            inside: false,
                        },
                    )
                }));
                game.live_worlds.get_mut().action = None;
                (game.yaw, game.pitch) = old_angles;
                if let Some(back) = back {
                    game.enter_level(back).map_err(|_| Error::Unavailable)?;
                }
                match result {
                    Ok(result) => result,
                    Err(panic) => std::panic::resume_unwind(panic),
                }
            })();
        });
        if !ran {
            return Err(Error::Stale);
        }
        answer
    }

    pub(in crate::game) fn live_world_destroy_block(
        &mut self,
        handle: LiveWorldRef,
        player: PlayerRef,
        hit: LiveWorldHit,
        into_inventory: bool,
    ) -> Result<bool, Error> {
        self.with_projected_actor(handle, player, hit, |game, hit| {
            let seal = game.transaction_open();
            let writes = game.transactions.writes();
            game.live_worlds
                .get_mut()
                .action
                .as_mut()
                .expect("scoped action")
                .break_writes = Some(writes);
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                game.player_destroy_block(player, hit.pos, into_inventory)
            }));
            game.transaction_close(&seal, result.as_ref().is_ok_and(|removed| *removed));
            match result {
                Ok(removed) => Ok(removed),
                Err(panic) => std::panic::resume_unwind(panic),
            }
        })
    }

    pub(in crate::game) fn live_world_use_block(
        &mut self,
        handle: LiveWorldRef,
        player: PlayerRef,
        hit: LiveWorldHit,
        hand: Hand,
    ) -> Result<bool, Error> {
        self.with_projected_actor(handle, player, hit, |game, hit| {
            let old_hand = game.hand;
            let old_use = game.guest_use;
            game.hand = hand;
            game.guest_use = Some(GuestUse::OnBlock(hand));
            let target = game.world.block(hit.pos[0], hit.pos[1], hit.pos[2]);
            let name = &game.res.facts().get(target).name;
            let menu = matches!(
                &**name,
                "minecraft:chest"
                    | "minecraft:trapped_chest"
                    | "minecraft:ender_chest"
                    | "minecraft:furnace"
                    | "minecraft:blast_furnace"
                    | "minecraft:smoker"
            );
            let item_supported = game.held().is_none_or(|stack| {
                game.pack
                    .assets
                    .registry
                    .default_state(&stack.item)
                    .is_some()
            });
            if !item_supported && !(menu && !game.keys.held(sunrise_client::actions::Action::Sneak))
            {
                game.hand = old_hand;
                game.guest_use = old_use;
                return Err(Error::Unsupported);
            }
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                game.use_hand(Some(&hit))
            }));
            game.hand = old_hand;
            game.guest_use = old_use;
            match result {
                Ok(result) => result.map_err(|_| Error::Unavailable).and_then(|used| {
                    if game.use_failed {
                        Err(Error::Denied)
                    } else {
                        Ok(used)
                    }
                }),
                Err(panic) => std::panic::resume_unwind(panic),
            }
        })
    }
}
